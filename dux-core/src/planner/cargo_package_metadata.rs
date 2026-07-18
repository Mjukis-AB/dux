//! Independent Cargo 1.96 package README/license-file provenance.
//!
//! Cargo metadata reports declaration-derived relative paths and performs one
//! small implicit README lookup. This guard derives those values from the
//! exact manifests, binds the complete implicit lookup namespace, and keeps it
//! stable across the accepted metadata pass.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::path_validation::{
    CanonicalFileDigestSnapshot, CanonicalScanRoot, FilesystemEntryKind, FilesystemIdentity,
    capture_path_snapshot, capture_regular_file_sha256, capture_scan_root, validate_cleanup_path,
    validate_scan_root,
};

const PACKAGE_METADATA_POLICY_REVISION: u32 = 1;
const MAX_PACKAGES: usize = 256;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_MANIFEST_AGGREGATE_BYTES: usize = 64 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 256 * 1024;
const MAX_PATH_BYTES: usize = 2 * 1024 * 1024;
const MAX_PATH_COMPONENTS: usize = 64;
const MAX_RECORDS: usize = 2_048;
const MAX_WATCHED_OBJECTS: usize = 4_096;
const IMPLICIT_README_NAMES: [&str; 3] = ["README.md", "README.txt", "README"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CargoPackageMetadataDeclaration {
    pub(super) package_id: String,
    pub(super) manifest_path: PathBuf,
    pub(super) readme: Option<String>,
    pub(super) license_file: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PackageMetadataObservation {
    declarations: Vec<CargoPackageMetadataDeclaration>,
    manifests: Vec<ManifestObservation>,
    records: Vec<NamespaceRecord>,
    implicit_readme_probe_count: usize,
    implicit_readme_selection_count: usize,
    declared_readme_count: usize,
    license_file_count: usize,
    closure_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManifestObservation {
    file: CanonicalFileDigestSnapshot,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NamespaceRecord {
    path: PathBuf,
    state: NamespaceState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NamespaceState {
    Missing,
    Present {
        kind: FilesystemEntryKind,
        identity: FilesystemIdentity,
        hard_link_count: u64,
    },
}

pub(super) struct CargoPackageMetadataGuard {
    project_root: CanonicalScanRoot,
    observation: PackageMetadataObservation,
    fence: platform::PackageMetadataMutationFence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CargoPackageMetadataEvidence {
    pub(super) policy_revision: u32,
    pub(super) package_count: u32,
    pub(super) implicit_readme_probe_count: u32,
    pub(super) implicit_readme_selection_count: u32,
    pub(super) declared_readme_count: u32,
    pub(super) license_file_count: u32,
    pub(super) closure_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CargoPackageMetadataError {
    Invalid,
    Changed,
    Unavailable,
}

impl CargoPackageMetadataGuard {
    pub(super) fn capture(
        project_root: &CanonicalScanRoot,
        declarations: &[CargoPackageMetadataDeclaration],
    ) -> Result<Self, CargoPackageMetadataError> {
        let observation = capture_observation(project_root, declarations)?;
        let fence = platform::PackageMetadataMutationFence::new(project_root, &observation)?;
        let guard = Self {
            project_root: project_root.clone(),
            observation,
            fence,
        };
        guard.revalidate()?;
        Ok(guard)
    }

    #[cfg(test)]
    pub(super) fn capture_unfenced_for_test(
        project_root: &CanonicalScanRoot,
        declarations: &[CargoPackageMetadataDeclaration],
    ) -> Result<Self, CargoPackageMetadataError> {
        let observation = capture_observation(project_root, declarations)?;
        Ok(Self {
            project_root: project_root.clone(),
            observation,
            fence: platform::PackageMetadataMutationFence::disabled_for_test(),
        })
    }

    pub(super) fn poll(&self) -> Result<(), CargoPackageMetadataError> {
        self.fence.poll()
    }

    pub(super) fn revalidate(&self) -> Result<(), CargoPackageMetadataError> {
        self.poll()?;
        let current = capture_observation(&self.project_root, &self.observation.declarations)
            .map_err(|_| CargoPackageMetadataError::Changed)?;
        if current != self.observation {
            return Err(CargoPackageMetadataError::Changed);
        }
        self.poll()
    }

    pub(super) fn evidence(
        &self,
    ) -> Result<CargoPackageMetadataEvidence, CargoPackageMetadataError> {
        self.revalidate()?;
        Ok(CargoPackageMetadataEvidence {
            policy_revision: PACKAGE_METADATA_POLICY_REVISION,
            package_count: bounded_u32(self.observation.declarations.len())?,
            implicit_readme_probe_count: bounded_u32(self.observation.implicit_readme_probe_count)?,
            implicit_readme_selection_count: bounded_u32(
                self.observation.implicit_readme_selection_count,
            )?,
            declared_readme_count: bounded_u32(self.observation.declared_readme_count)?,
            license_file_count: bounded_u32(self.observation.license_file_count)?,
            closure_sha256: self.observation.closure_sha256,
        })
    }
}

fn capture_observation(
    project_root: &CanonicalScanRoot,
    declarations: &[CargoPackageMetadataDeclaration],
) -> Result<PackageMetadataObservation, CargoPackageMetadataError> {
    if declarations.is_empty() || declarations.len() > MAX_PACKAGES {
        return Err(CargoPackageMetadataError::Invalid);
    }
    if capture_exact_directory(project_root.canonical_path())? != *project_root {
        return Err(CargoPackageMetadataError::Changed);
    }

    let mut declarations = declarations.to_vec();
    declarations.sort_by(|left, right| {
        left.package_id
            .as_bytes()
            .cmp(right.package_id.as_bytes())
            .then_with(|| {
                left.manifest_path
                    .as_os_str()
                    .as_bytes()
                    .cmp(right.manifest_path.as_os_str().as_bytes())
            })
    });
    let mut package_ids = BTreeSet::new();
    let mut manifest_paths = BTreeSet::new();
    let mut text_bytes = 0_usize;
    let mut path_bytes = 0_usize;
    for declaration in &declarations {
        charge_text(&mut text_bytes, &declaration.package_id)?;
        charge_optional_text(&mut text_bytes, declaration.readme.as_deref())?;
        charge_optional_text(&mut text_bytes, declaration.license_file.as_deref())?;
        charge_path(&mut path_bytes, &declaration.manifest_path)?;
        if declaration.package_id.is_empty()
            || declaration.package_id.chars().any(char::is_control)
            || !package_ids.insert(declaration.package_id.as_bytes().to_vec())
            || !manifest_paths.insert(declaration.manifest_path.as_os_str().as_bytes().to_vec())
            || !is_normalized_absolute(&declaration.manifest_path)
            || !declaration
                .manifest_path
                .starts_with(project_root.canonical_path())
            || declaration
                .manifest_path
                .file_name()
                .and_then(|name| name.to_str())
                != Some("Cargo.toml")
        {
            return Err(CargoPackageMetadataError::Invalid);
        }
    }

    let workspace_manifest_path = project_root.canonical_path().join("Cargo.toml");
    charge_path(&mut path_bytes, &workspace_manifest_path)?;
    let mut manifests_by_path = BTreeMap::new();
    let mut aggregate_manifest_bytes = 0_usize;
    let workspace_manifest = capture_manifest(
        project_root,
        &workspace_manifest_path,
        &mut aggregate_manifest_bytes,
    )?;
    manifests_by_path.insert(workspace_manifest_path.clone(), workspace_manifest);
    for declaration in &declarations {
        if manifests_by_path.contains_key(&declaration.manifest_path) {
            continue;
        }
        let manifest = capture_manifest(
            project_root,
            &declaration.manifest_path,
            &mut aggregate_manifest_bytes,
        )?;
        manifests_by_path.insert(declaration.manifest_path.clone(), manifest);
    }
    let workspace_document = parse_manifest(
        &manifests_by_path
            .get(&workspace_manifest_path)
            .ok_or(CargoPackageMetadataError::Invalid)?
            .bytes,
    )?;
    let workspace_package = workspace_document
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|workspace| workspace.get("package"))
        .map(|value| value.as_table().ok_or(CargoPackageMetadataError::Invalid))
        .transpose()?;

    let mut records = BTreeMap::<Vec<u8>, NamespaceRecord>::new();
    for manifest in manifests_by_path.values() {
        insert_record(
            &mut records,
            NamespaceRecord {
                path: manifest.file.path().canonical_path().to_path_buf(),
                state: NamespaceState::Present {
                    kind: FilesystemEntryKind::RegularFile,
                    identity: manifest.file.path().target_identity(),
                    hard_link_count: manifest.file.path().hard_link_count(),
                },
            },
        )?;
    }

    let mut implicit_readme_probe_count = 0_usize;
    let mut implicit_readme_selection_count = 0_usize;
    let mut declared_readme_count = 0_usize;
    let mut license_file_count = 0_usize;
    for declaration in &declarations {
        let manifest = manifests_by_path
            .get(&declaration.manifest_path)
            .ok_or(CargoPackageMetadataError::Invalid)?;
        let document = parse_manifest(&manifest.bytes)?;
        let package = document
            .get("package")
            .and_then(toml::Value::as_table)
            .ok_or(CargoPackageMetadataError::Invalid)?;
        let package_root_path = declaration
            .manifest_path
            .parent()
            .ok_or(CargoPackageMetadataError::Invalid)?;
        let package_root = capture_exact_directory(package_root_path)?;
        insert_record(
            &mut records,
            NamespaceRecord {
                path: package_root.canonical_path().to_path_buf(),
                state: NamespaceState::Present {
                    kind: FilesystemEntryKind::Directory,
                    identity: package_root.identity(),
                    hard_link_count: 1,
                },
            },
        )?;

        let (expected_readme, implicit_probes, implicit_selected, declared) = derive_readme(
            project_root,
            package_root_path,
            package,
            workspace_package,
            &mut records,
            &mut text_bytes,
            &mut path_bytes,
        )?;
        if expected_readme != declaration.readme {
            return Err(CargoPackageMetadataError::Invalid);
        }
        implicit_readme_probe_count = implicit_readme_probe_count
            .checked_add(implicit_probes)
            .ok_or(CargoPackageMetadataError::Unavailable)?;
        implicit_readme_selection_count = implicit_readme_selection_count
            .checked_add(usize::from(implicit_selected))
            .ok_or(CargoPackageMetadataError::Unavailable)?;
        declared_readme_count = declared_readme_count
            .checked_add(usize::from(declared))
            .ok_or(CargoPackageMetadataError::Unavailable)?;

        let expected_license = derive_license_file(
            project_root,
            package_root_path,
            package,
            workspace_package,
            &mut text_bytes,
        )?;
        if expected_license != declaration.license_file {
            return Err(CargoPackageMetadataError::Invalid);
        }
        license_file_count = license_file_count
            .checked_add(usize::from(expected_license.is_some()))
            .ok_or(CargoPackageMetadataError::Unavailable)?;
    }

    let manifests = manifests_by_path.into_values().collect::<Vec<_>>();
    let records = records.into_values().collect::<Vec<_>>();
    if records.len() > MAX_RECORDS {
        return Err(CargoPackageMetadataError::Unavailable);
    }
    let closure_sha256 = digest_observation(
        &declarations,
        &manifests,
        &records,
        implicit_readme_probe_count,
        implicit_readme_selection_count,
        declared_readme_count,
        license_file_count,
    );
    Ok(PackageMetadataObservation {
        declarations,
        manifests,
        records,
        implicit_readme_probe_count,
        implicit_readme_selection_count,
        declared_readme_count,
        license_file_count,
        closure_sha256,
    })
}

fn capture_manifest(
    project_root: &CanonicalScanRoot,
    path: &Path,
    aggregate_bytes: &mut usize,
) -> Result<ManifestObservation, CargoPackageMetadataError> {
    let root = validate_scan_root(project_root.canonical_path())
        .map_err(|_| CargoPackageMetadataError::Invalid)?;
    let relative =
        validate_cleanup_path(&root, path).map_err(|_| CargoPackageMetadataError::Invalid)?;
    let file = capture_regular_file_sha256(project_root, relative, MAX_MANIFEST_BYTES)
        .map_err(|_| CargoPackageMetadataError::Unavailable)?;
    if file.path().target_kind() != FilesystemEntryKind::RegularFile
        || file.path().hard_link_count() != 1
    {
        return Err(CargoPackageMetadataError::Invalid);
    }
    let length =
        usize::try_from(file.byte_length()).map_err(|_| CargoPackageMetadataError::Unavailable)?;
    *aggregate_bytes = aggregate_bytes
        .checked_add(length)
        .ok_or(CargoPackageMetadataError::Unavailable)?;
    if *aggregate_bytes > MAX_MANIFEST_AGGREGATE_BYTES {
        return Err(CargoPackageMetadataError::Unavailable);
    }
    let bytes = fs::read(path).map_err(|_| CargoPackageMetadataError::Unavailable)?;
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    if bytes.len() != length || digest != file.sha256() {
        return Err(CargoPackageMetadataError::Changed);
    }
    Ok(ManifestObservation { file, bytes })
}

#[allow(clippy::too_many_arguments)]
fn derive_readme(
    project_root: &CanonicalScanRoot,
    package_root: &Path,
    package: &toml::map::Map<String, toml::Value>,
    workspace_package: Option<&toml::map::Map<String, toml::Value>>,
    records: &mut BTreeMap<Vec<u8>, NamespaceRecord>,
    text_bytes: &mut usize,
    path_bytes: &mut usize,
) -> Result<(Option<String>, usize, bool, bool), CargoPackageMetadataError> {
    let Some(value) = package.get("readme") else {
        let mut selected = None;
        for name in IMPLICIT_README_NAMES {
            let path = package_root.join(name);
            charge_path(path_bytes, &path)?;
            let record = capture_optional_entry(project_root, &path)?;
            if selected.is_none()
                && matches!(
                    record.state,
                    NamespaceState::Present {
                        kind: FilesystemEntryKind::RegularFile,
                        ..
                    }
                )
            {
                selected = Some(name.to_owned());
            }
            insert_record(records, record)?;
        }
        if let Some(value) = selected.as_deref() {
            charge_text(text_bytes, value)?;
        }
        let selected_present = selected.is_some();
        return Ok((
            selected,
            IMPLICIT_README_NAMES.len(),
            selected_present,
            false,
        ));
    };
    let inherited = inherited_value(value, workspace_package, "readme")?;
    let (raw, origin, declared) = match inherited {
        InheritedValue::Direct(value) => (readme_path_value(value)?, package_root, true),
        InheritedValue::Workspace(value) => (
            readme_path_value(value)?,
            project_root.canonical_path(),
            true,
        ),
    };
    let Some(raw) = raw else {
        return Ok((None, 0, false, declared));
    };
    let expected = reported_path(project_root, package_root, origin, raw)?;
    charge_text(text_bytes, &expected)?;
    Ok((Some(expected), 0, false, declared))
}

fn derive_license_file(
    project_root: &CanonicalScanRoot,
    package_root: &Path,
    package: &toml::map::Map<String, toml::Value>,
    workspace_package: Option<&toml::map::Map<String, toml::Value>>,
    text_bytes: &mut usize,
) -> Result<Option<String>, CargoPackageMetadataError> {
    let Some(value) = package.get("license-file") else {
        return Ok(None);
    };
    let inherited = inherited_value(value, workspace_package, "license-file")?;
    let (raw, origin) = match inherited {
        InheritedValue::Direct(value) => (required_path_string(value)?, package_root),
        InheritedValue::Workspace(value) => {
            (required_path_string(value)?, project_root.canonical_path())
        }
    };
    let expected = reported_path(project_root, package_root, origin, raw)?;
    charge_text(text_bytes, &expected)?;
    Ok(Some(expected))
}

enum InheritedValue<'a> {
    Direct(&'a toml::Value),
    Workspace(&'a toml::Value),
}

fn inherited_value<'a>(
    value: &'a toml::Value,
    workspace_package: Option<&'a toml::map::Map<String, toml::Value>>,
    key: &str,
) -> Result<InheritedValue<'a>, CargoPackageMetadataError> {
    let Some(table) = value.as_table() else {
        return Ok(InheritedValue::Direct(value));
    };
    if table.len() != 1 || table.get("workspace").and_then(toml::Value::as_bool) != Some(true) {
        return Err(CargoPackageMetadataError::Invalid);
    }
    let inherited = workspace_package
        .and_then(|package| package.get(key))
        .ok_or(CargoPackageMetadataError::Invalid)?;
    Ok(InheritedValue::Workspace(inherited))
}

fn readme_path_value(value: &toml::Value) -> Result<Option<&str>, CargoPackageMetadataError> {
    match value {
        toml::Value::Boolean(false) => Ok(None),
        toml::Value::Boolean(true) => Ok(Some("README.md")),
        toml::Value::String(value) => Ok(Some(required_path_text(value)?)),
        _ => Err(CargoPackageMetadataError::Invalid),
    }
}

fn required_path_string(value: &toml::Value) -> Result<&str, CargoPackageMetadataError> {
    required_path_text(value.as_str().ok_or(CargoPackageMetadataError::Invalid)?)
}

fn required_path_text(value: &str) -> Result<&str, CargoPackageMetadataError> {
    if value.is_empty() || value.chars().any(char::is_control) || value.len() > 4 * 1024 {
        Err(CargoPackageMetadataError::Invalid)
    } else {
        Ok(value)
    }
}

fn reported_path(
    project_root: &CanonicalScanRoot,
    package_root: &Path,
    origin: &Path,
    raw: &str,
) -> Result<String, CargoPackageMetadataError> {
    let raw_path = Path::new(raw);
    if raw_path.is_absolute() || raw_path.components().count() > MAX_PATH_COMPONENTS {
        return Err(CargoPackageMetadataError::Invalid);
    }
    let resolved = normalize_path(origin, raw_path)?;
    if !resolved.starts_with(project_root.canonical_path()) {
        return Err(CargoPackageMetadataError::Invalid);
    }
    let reported = if origin == package_root {
        raw_path.to_path_buf()
    } else {
        let relative_package = package_root
            .strip_prefix(project_root.canonical_path())
            .map_err(|_| CargoPackageMetadataError::Invalid)?;
        let mut rebased = PathBuf::new();
        for _ in relative_package.components() {
            rebased.push("..");
        }
        rebased.push(raw_path);
        rebased
    };
    reported
        .to_str()
        .map(str::to_owned)
        .ok_or(CargoPackageMetadataError::Invalid)
}

fn normalize_path(base: &Path, path: &Path) -> Result<PathBuf, CargoPackageMetadataError> {
    let mut normalized = PathBuf::new();
    for component in base.join(path).components() {
        match component {
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(CargoPackageMetadataError::Invalid);
                }
            }
            Component::Prefix(_) => return Err(CargoPackageMetadataError::Invalid),
        }
    }
    if normalized.is_absolute() {
        Ok(normalized)
    } else {
        Err(CargoPackageMetadataError::Invalid)
    }
}

fn capture_optional_entry(
    project_root: &CanonicalScanRoot,
    path: &Path,
) -> Result<NamespaceRecord, CargoPackageMetadataError> {
    if !is_normalized_absolute(path) || !path.starts_with(project_root.canonical_path()) {
        return Err(CargoPackageMetadataError::Invalid);
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(CargoPackageMetadataError::Invalid)
        }
        Ok(_) => {
            let root = validate_scan_root(project_root.canonical_path())
                .map_err(|_| CargoPackageMetadataError::Invalid)?;
            let relative = validate_cleanup_path(&root, path)
                .map_err(|_| CargoPackageMetadataError::Invalid)?;
            let snapshot = capture_path_snapshot(project_root, relative)
                .map_err(|_| CargoPackageMetadataError::Unavailable)?;
            if !matches!(
                snapshot.target_kind(),
                FilesystemEntryKind::RegularFile | FilesystemEntryKind::Directory
            ) || (snapshot.target_kind() == FilesystemEntryKind::RegularFile
                && snapshot.hard_link_count() != 1)
            {
                return Err(CargoPackageMetadataError::Invalid);
            }
            Ok(NamespaceRecord {
                path: path.to_path_buf(),
                state: NamespaceState::Present {
                    kind: snapshot.target_kind(),
                    identity: snapshot.target_identity(),
                    hard_link_count: snapshot.hard_link_count(),
                },
            })
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(NamespaceRecord {
            path: path.to_path_buf(),
            state: NamespaceState::Missing,
        }),
        Err(_) => Err(CargoPackageMetadataError::Unavailable),
    }
}

fn insert_record(
    records: &mut BTreeMap<Vec<u8>, NamespaceRecord>,
    record: NamespaceRecord,
) -> Result<(), CargoPackageMetadataError> {
    let key = record.path.as_os_str().as_bytes().to_vec();
    if let Some(existing) = records.insert(key, record.clone())
        && existing != record
    {
        return Err(CargoPackageMetadataError::Invalid);
    }
    if records.len() > MAX_RECORDS {
        Err(CargoPackageMetadataError::Unavailable)
    } else {
        Ok(())
    }
}

fn capture_exact_directory(path: &Path) -> Result<CanonicalScanRoot, CargoPackageMetadataError> {
    let lexical = validate_scan_root(path).map_err(|_| CargoPackageMetadataError::Invalid)?;
    capture_scan_root(lexical).map_err(|_| CargoPackageMetadataError::Unavailable)
}

fn parse_manifest(bytes: &[u8]) -> Result<toml::Value, CargoPackageMetadataError> {
    let text = std::str::from_utf8(bytes).map_err(|_| CargoPackageMetadataError::Invalid)?;
    toml::from_str(text).map_err(|_| CargoPackageMetadataError::Invalid)
}

fn is_normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .collect::<PathBuf>()
            .as_os_str()
            .as_bytes()
            == path.as_os_str().as_bytes()
        && !path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
}

fn charge_text(total: &mut usize, value: &str) -> Result<(), CargoPackageMetadataError> {
    *total = total
        .checked_add(value.len())
        .ok_or(CargoPackageMetadataError::Unavailable)?;
    if *total > MAX_TEXT_BYTES {
        Err(CargoPackageMetadataError::Unavailable)
    } else {
        Ok(())
    }
}

fn charge_optional_text(
    total: &mut usize,
    value: Option<&str>,
) -> Result<(), CargoPackageMetadataError> {
    if let Some(value) = value {
        required_path_text(value)?;
        charge_text(total, value)?;
    }
    Ok(())
}

fn charge_path(total: &mut usize, path: &Path) -> Result<(), CargoPackageMetadataError> {
    *total = total
        .checked_add(path.as_os_str().as_bytes().len())
        .ok_or(CargoPackageMetadataError::Unavailable)?;
    if *total > MAX_PATH_BYTES {
        Err(CargoPackageMetadataError::Unavailable)
    } else {
        Ok(())
    }
}

fn bounded_u32(value: usize) -> Result<u32, CargoPackageMetadataError> {
    u32::try_from(value).map_err(|_| CargoPackageMetadataError::Unavailable)
}

#[allow(clippy::too_many_arguments)]
fn digest_observation(
    declarations: &[CargoPackageMetadataDeclaration],
    manifests: &[ManifestObservation],
    records: &[NamespaceRecord],
    implicit_readme_probe_count: usize,
    implicit_readme_selection_count: usize,
    declared_readme_count: usize,
    license_file_count: usize,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"dux-cargo-package-metadata-v1\0");
    digest.update((declarations.len() as u64).to_le_bytes());
    digest.update((implicit_readme_probe_count as u64).to_le_bytes());
    digest.update((implicit_readme_selection_count as u64).to_le_bytes());
    digest.update((declared_readme_count as u64).to_le_bytes());
    digest.update((license_file_count as u64).to_le_bytes());
    for declaration in declarations {
        digest_bytes(&mut digest, declaration.package_id.as_bytes());
        digest_bytes(
            &mut digest,
            declaration.manifest_path.as_os_str().as_bytes(),
        );
        digest_optional_text(&mut digest, declaration.readme.as_deref());
        digest_optional_text(&mut digest, declaration.license_file.as_deref());
    }
    for manifest in manifests {
        digest_bytes(
            &mut digest,
            manifest.file.path().canonical_path().as_os_str().as_bytes(),
        );
        digest.update(manifest.file.byte_length().to_le_bytes());
        digest.update(manifest.file.sha256());
    }
    for record in records {
        digest_bytes(&mut digest, record.path.as_os_str().as_bytes());
        match record.state {
            NamespaceState::Missing => digest.update([0]),
            NamespaceState::Present {
                kind,
                identity,
                hard_link_count,
            } => {
                digest.update([match kind {
                    FilesystemEntryKind::RegularFile => 1,
                    FilesystemEntryKind::Directory => 2,
                }]);
                digest.update(identity.volume().to_le_bytes());
                digest.update(identity.object().to_le_bytes());
                digest.update(hard_link_count.to_le_bytes());
            }
        }
    }
    digest.finalize().into()
}

fn digest_optional_text(digest: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            digest.update([1]);
            digest_bytes(digest, value.as_bytes());
        }
        None => digest.update([0]),
    }
}

fn digest_bytes(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value);
}

#[cfg(target_os = "macos")]
mod platform {
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::fd::{AsFd, AsRawFd, OwnedFd};

    use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl, open};
    use nix::libc;
    use nix::mount::MntFlags;
    use nix::sys::event::{EvFlags, EventFilter, FilterFlag, KEvent, Kqueue};
    use nix::sys::stat::{Mode, fstat};
    use nix::sys::statfs::fstatfs;

    use super::{
        CanonicalScanRoot, CargoPackageMetadataError, FilesystemEntryKind, FilesystemIdentity,
        MAX_WATCHED_OBJECTS, NamespaceState, PackageMetadataObservation, capture_exact_directory,
    };

    pub(super) struct PackageMetadataMutationFence {
        queue: Kqueue,
        _objects: Vec<OwnedFd>,
    }

    impl PackageMetadataMutationFence {
        #[cfg(test)]
        pub(super) fn disabled_for_test() -> Self {
            let queue = Kqueue::new().expect("test kqueue must be available");
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .expect("test kqueue must support close-on-exec");
            Self {
                queue,
                _objects: Vec::new(),
            }
        }

        pub(super) fn new(
            project_root: &CanonicalScanRoot,
            observation: &PackageMetadataObservation,
        ) -> Result<Self, CargoPackageMetadataError> {
            let expected = watched_objects(project_root, observation)?;
            require_descriptor_budget(expected.len().saturating_add(1))?;
            let queue = Kqueue::new().map_err(|_| CargoPackageMetadataError::Unavailable)?;
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .map_err(|_| CargoPackageMetadataError::Unavailable)?;
            let objects = expected
                .iter()
                .map(open_watch_object)
                .collect::<Result<Vec<_>, _>>()?;
            let changes = objects
                .iter()
                .zip(expected.iter())
                .map(|(object, expected)| {
                    let flags = if expected.kind == FilesystemEntryKind::Directory {
                        FilterFlag::NOTE_DELETE
                            | FilterFlag::NOTE_WRITE
                            | FilterFlag::NOTE_ATTRIB
                            | FilterFlag::NOTE_RENAME
                            | FilterFlag::NOTE_REVOKE
                    } else {
                        FilterFlag::NOTE_DELETE
                            | FilterFlag::NOTE_ATTRIB
                            | FilterFlag::NOTE_LINK
                            | FilterFlag::NOTE_RENAME
                            | FilterFlag::NOTE_REVOKE
                    };
                    vnode_event(object.as_raw_fd(), flags)
                })
                .collect::<Vec<_>>();
            let mut events = vec![empty_event(); changes.len().max(1)];
            let count = queue
                .kevent(&changes, &mut events, Some(zero_timeout()))
                .map_err(|_| CargoPackageMetadataError::Unavailable)?;
            if count != 0 {
                return Err(CargoPackageMetadataError::Changed);
            }
            Ok(Self {
                queue,
                _objects: objects,
            })
        }

        pub(super) fn poll(&self) -> Result<(), CargoPackageMetadataError> {
            let mut event = [empty_event()];
            let count = self
                .queue
                .kevent(&[], &mut event, Some(zero_timeout()))
                .map_err(|_| CargoPackageMetadataError::Unavailable)?;
            if count == 0 {
                Ok(())
            } else {
                Err(CargoPackageMetadataError::Changed)
            }
        }
    }

    #[derive(Clone)]
    struct WatchedObject {
        path: std::path::PathBuf,
        kind: FilesystemEntryKind,
        identity: FilesystemIdentity,
        hard_link_count: u64,
    }

    fn watched_objects(
        project_root: &CanonicalScanRoot,
        observation: &PackageMetadataObservation,
    ) -> Result<Vec<WatchedObject>, CargoPackageMetadataError> {
        let mut objects = BTreeMap::<(u64, u128), WatchedObject>::new();
        insert_directory(
            &mut objects,
            capture_exact_directory(project_root.canonical_path())?,
        )?;
        for record in &observation.records {
            let NamespaceState::Present {
                kind,
                identity,
                hard_link_count,
            } = record.state
            else {
                continue;
            };
            objects
                .entry((identity.volume(), identity.object()))
                .or_insert(WatchedObject {
                    path: record.path.clone(),
                    kind,
                    identity,
                    hard_link_count,
                });
            if record.path == project_root.canonical_path() {
                continue;
            }
            let parent = record
                .path
                .parent()
                .ok_or(CargoPackageMetadataError::Invalid)?;
            let relative = parent
                .strip_prefix(project_root.canonical_path())
                .map_err(|_| CargoPackageMetadataError::Invalid)?;
            let mut current = project_root.canonical_path().to_path_buf();
            for component in relative.components() {
                current.push(component.as_os_str());
                insert_directory(&mut objects, capture_exact_directory(&current)?)?;
            }
        }
        if objects.len() > MAX_WATCHED_OBJECTS {
            return Err(CargoPackageMetadataError::Unavailable);
        }
        Ok(objects.into_values().collect())
    }

    fn insert_directory(
        objects: &mut BTreeMap<(u64, u128), WatchedObject>,
        directory: CanonicalScanRoot,
    ) -> Result<(), CargoPackageMetadataError> {
        let identity = directory.identity();
        objects
            .entry((identity.volume(), identity.object()))
            .or_insert(WatchedObject {
                path: directory.canonical_path().to_path_buf(),
                kind: FilesystemEntryKind::Directory,
                identity,
                hard_link_count: 1,
            });
        if objects.len() > MAX_WATCHED_OBJECTS {
            Err(CargoPackageMetadataError::Unavailable)
        } else {
            Ok(())
        }
    }

    fn open_watch_object(expected: &WatchedObject) -> Result<OwnedFd, CargoPackageMetadataError> {
        let object = open(
            &expected.path,
            OFlag::from_bits_retain(libc::O_EVTONLY) | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| CargoPackageMetadataError::Unavailable)?;
        require_reviewed_filesystem(&object)?;
        let status = fstat(&object).map_err(|_| CargoPackageMetadataError::Unavailable)?;
        let kind_matches = if expected.kind == FilesystemEntryKind::Directory {
            (status.st_mode & libc::S_IFMT) == libc::S_IFDIR
        } else {
            (status.st_mode & libc::S_IFMT) == libc::S_IFREG
        };
        if !kind_matches
            || status.st_dev as u64 != expected.identity.volume()
            || u128::from(status.st_ino) != expected.identity.object()
            || (expected.kind == FilesystemEntryKind::RegularFile
                && status.st_nlink as u64 != expected.hard_link_count)
        {
            return Err(CargoPackageMetadataError::Changed);
        }
        Ok(object)
    }

    fn require_reviewed_filesystem(
        descriptor: &impl AsFd,
    ) -> Result<(), CargoPackageMetadataError> {
        let status = fstatfs(descriptor).map_err(|_| CargoPackageMetadataError::Unavailable)?;
        if status.filesystem_type_name().eq_ignore_ascii_case("apfs")
            && status.flags().contains(MntFlags::MNT_LOCAL)
        {
            Ok(())
        } else {
            Err(CargoPackageMetadataError::Invalid)
        }
    }

    fn require_descriptor_budget(required: usize) -> Result<(), CargoPackageMetadataError> {
        const RESERVE: usize = 128;
        let mut limits = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `limits` points to initialized writable storage.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) } != 0 {
            return Err(CargoPackageMetadataError::Unavailable);
        }
        let current = fs::read_dir("/dev/fd")
            .map_err(|_| CargoPackageMetadataError::Unavailable)?
            .count();
        let soft_limit =
            usize::try_from(limits.rlim_cur).map_err(|_| CargoPackageMetadataError::Unavailable)?;
        if current
            .checked_add(required)
            .and_then(|count| count.checked_add(RESERVE))
            .is_some_and(|count| count <= soft_limit)
        {
            Ok(())
        } else {
            Err(CargoPackageMetadataError::Unavailable)
        }
    }

    fn vnode_event(descriptor: libc::c_int, flags: FilterFlag) -> KEvent {
        KEvent::new(
            descriptor as usize,
            EventFilter::EVFILT_VNODE,
            EvFlags::EV_ADD | EvFlags::EV_ENABLE | EvFlags::EV_CLEAR,
            flags,
            0,
            0,
        )
    }

    fn empty_event() -> KEvent {
        KEvent::new(
            0,
            EventFilter::EVFILT_VNODE,
            EvFlags::empty(),
            FilterFlag::empty(),
            0,
            0,
        )
    }

    fn zero_timeout() -> libc::timespec {
        libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn exact_manifest_semantics_cover_implicit_explicit_suppressed_and_inherited_paths() {
        let fixture = Fixture::new();
        let auto = fixture.package("auto", "[package]\nname='auto'\nversion='0.1.0'\n");
        fs::write(auto.parent().unwrap().join("README.txt"), "auto\n").unwrap();
        let suppressed = fixture.package(
            "suppressed",
            "[package]\nname='suppressed'\nversion='0.1.0'\nreadme=false\n",
        );
        fs::write(suppressed.parent().unwrap().join("README.md"), "ignored\n").unwrap();
        let explicit = fixture.package(
            "explicit",
            "[package]\nname='explicit'\nversion='0.1.0'\nreadme=true\nlicense-file='../LICENSE.direct'\n",
        );
        let inherited = fixture.package(
            "inherited",
            "[package]\nname='inherited'\nversion='0.1.0'\nreadme.workspace=true\nlicense-file.workspace=true\n",
        );

        let declarations = vec![
            declaration("auto", auto, Some("README.txt"), None),
            declaration("suppressed", suppressed, None, None),
            declaration(
                "explicit",
                explicit,
                Some("README.md"),
                Some("../LICENSE.direct"),
            ),
            declaration(
                "inherited",
                inherited,
                Some("../README.workspace.md"),
                Some("../LICENSE.workspace"),
            ),
        ];
        let guard =
            CargoPackageMetadataGuard::capture_unfenced_for_test(&fixture.root, &declarations)
                .unwrap();
        let evidence = guard.evidence().unwrap();

        assert_eq!(evidence.policy_revision, 1);
        assert_eq!(evidence.package_count, 4);
        assert_eq!(evidence.implicit_readme_probe_count, 3);
        assert_eq!(evidence.implicit_readme_selection_count, 1);
        assert_eq!(evidence.declared_readme_count, 3);
        assert_eq!(evidence.license_file_count, 2);
        assert_ne!(evidence.closure_sha256, [0; 32]);
    }

    #[test]
    fn fabricated_omitted_and_symlinked_readme_results_fail_closed() {
        let fixture = Fixture::new();
        let manifest = fixture.package(
            "member",
            "[package]\nname='member'\nversion='0.1.0'\nreadme=true\n",
        );
        let omitted = declaration("member", manifest.clone(), None, None);
        assert!(matches!(
            CargoPackageMetadataGuard::capture_unfenced_for_test(&fixture.root, &[omitted]),
            Err(CargoPackageMetadataError::Invalid)
        ));

        fs::write(&manifest, "[package]\nname='member'\nversion='0.1.0'\n").unwrap();
        let fabricated = declaration("member", manifest.clone(), Some("README.md"), None);
        assert!(matches!(
            CargoPackageMetadataGuard::capture_unfenced_for_test(&fixture.root, &[fabricated]),
            Err(CargoPackageMetadataError::Invalid)
        ));

        let real = fixture.root.canonical_path().join("real-readme");
        fs::write(&real, "readme\n").unwrap();
        symlink(&real, manifest.parent().unwrap().join("README.md")).unwrap();
        let symlinked = declaration("member", manifest, Some("README.md"), None);
        assert!(matches!(
            CargoPackageMetadataGuard::capture_unfenced_for_test(&fixture.root, &[symlinked]),
            Err(CargoPackageMetadataError::Invalid)
        ));
    }

    struct Fixture {
        _temp: TempDir,
        root: CanonicalScanRoot,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let path = temp.path().join("workspace");
            fs::create_dir(&path).unwrap();
            fs::write(
                path.join("Cargo.toml"),
                "[workspace]\nmembers=[]\n\n[workspace.package]\nreadme='README.workspace.md'\nlicense-file='LICENSE.workspace'\n",
            )
            .unwrap();
            let path = fs::canonicalize(path).unwrap();
            let root = capture_exact_directory(&path).unwrap();
            Self { _temp: temp, root }
        }

        fn package(&self, name: &str, contents: &str) -> PathBuf {
            let directory = self.root.canonical_path().join(name);
            fs::create_dir(&directory).unwrap();
            let manifest = directory.join("Cargo.toml");
            fs::write(&manifest, contents).unwrap();
            manifest
        }
    }

    fn declaration(
        package_id: &str,
        manifest_path: PathBuf,
        readme: Option<&str>,
        license_file: Option<&str>,
    ) -> CargoPackageMetadataDeclaration {
        CargoPackageMetadataDeclaration {
            package_id: package_id.to_owned(),
            manifest_path,
            readme: readme.map(str::to_owned),
            license_file: license_file.map(str::to_owned),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{CanonicalScanRoot, CargoPackageMetadataError, PackageMetadataObservation};

    pub(super) struct PackageMetadataMutationFence;

    impl PackageMetadataMutationFence {
        #[cfg(test)]
        pub(super) fn disabled_for_test() -> Self {
            Self
        }

        pub(super) fn new(
            _project_root: &CanonicalScanRoot,
            _observation: &PackageMetadataObservation,
        ) -> Result<Self, CargoPackageMetadataError> {
            Ok(Self)
        }

        pub(super) fn poll(&self) -> Result<(), CargoPackageMetadataError> {
            Ok(())
        }
    }
}
