use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::*;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(super) struct CargoMetadataDocument {
    packages: Vec<CargoMetadataPackageDocument>,
    workspace_members: Vec<String>,
    workspace_default_members: Vec<String>,
    version: u32,
    workspace_root: String,
    target_directory: String,
    resolve: serde_json::Value,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct CargoMetadataPackageDocument {
    id: String,
    manifest_path: String,
    source: serde_json::Value,
    readme: serde_json::Value,
    license_file: serde_json::Value,
    dependencies: Vec<CargoMetadataDependencyDocument>,
    targets: Vec<CargoMetadataTargetDocument>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct CargoMetadataDependencyDocument {
    source: serde_json::Value,
    path: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct CargoMetadataTargetDocument {
    name: String,
    kind: Vec<String>,
    src_path: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CargoPathDependencyEvidence {
    pub(super) policy_revision: u32,
    pub(super) dependency_declaration_count: u32,
    pub(super) local_path_dependency_count: u32,
    pub(super) unique_local_manifest_count: u32,
    pub(super) closure_sha256: [u8; 32],
    pub(super) reported_edges: Vec<CargoReportedPathDependency>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CargoWorkspaceMembershipConsistencyEvidence {
    pub(super) policy_revision: u32,
    pub(super) seed_package_count: u32,
    pub(super) excluded_member_count: u32,
    pub(super) reachable_package_count: u32,
    pub(super) default_member_count: u32,
    pub(super) closure_sha256: [u8; 32],
}

pub(super) struct ParsedCargoMetadata {
    pub(super) document: CargoMetadataDocument,
    pub(super) workspace_manifests: Vec<CargoWorkspaceManifestDeclaration>,
    pub(super) path_dependencies: CargoPathDependencyEvidence,
    pub(super) package_metadata: Vec<CargoPackageMetadataDeclaration>,
    pub(super) target_namespace: Vec<CargoTargetNamespaceDeclaration>,
}

pub(super) fn parse_metadata_document(
    bytes: &[u8],
    live: &RustTargetLiveWitness,
) -> Result<ParsedCargoMetadata, CargoMetadataValidationError> {
    let metadata: CargoMetadataDocument =
        serde_json::from_slice(bytes).map_err(|_| CargoMetadataValidationError::InvalidMetadata)?;
    if metadata.version != 1 || !metadata.resolve.is_null() {
        return Err(CargoMetadataValidationError::InvalidMetadata);
    }
    let metadata_root = validate_scan_root(Path::new(&metadata.workspace_root))?;
    let _metadata_target =
        validate_cleanup_path(&metadata_root, Path::new(&metadata.target_directory))?;
    if Path::new(&metadata.workspace_root) != live.project_root() {
        return Err(CargoMetadataValidationError::WorkspaceMismatch);
    }
    if Path::new(&metadata.target_directory) != live.target_path() {
        return Err(CargoMetadataValidationError::TargetDirectoryMismatch);
    }

    if metadata.workspace_members.is_empty()
        || metadata.workspace_members.len() > MAX_WORKSPACE_MEMBERS
        || metadata.packages.len() != metadata.workspace_members.len()
        || metadata.workspace_default_members.len() > MAX_WORKSPACE_DEFAULT_MEMBER_ROWS
    {
        return Err(CargoMetadataValidationError::InvalidWorkspaceMembers);
    }
    let mut packages = BTreeMap::new();
    let mut manifest_paths = BTreeSet::new();
    for package in &metadata.packages {
        if package.id.is_empty()
            || package.id.len() > MAX_PACKAGE_ID_BYTES
            || package.manifest_path.is_empty()
            || !package.source.is_null()
            || packages.insert(package.id.as_str(), package).is_some()
            || !manifest_paths.insert(package.manifest_path.as_bytes().to_vec())
        {
            return Err(CargoMetadataValidationError::InvalidWorkspaceMembers);
        }
    }
    let mut members = BTreeSet::new();
    for member in &metadata.workspace_members {
        if member.is_empty()
            || member.len() > MAX_PACKAGE_ID_BYTES
            || !members.insert(member.as_str())
            || !packages.contains_key(member.as_str())
        {
            return Err(CargoMetadataValidationError::InvalidWorkspaceMembers);
        }
    }
    if members.len() != packages.len() {
        return Err(CargoMetadataValidationError::InvalidWorkspaceMembers);
    }
    for member in &metadata.workspace_default_members {
        if member.is_empty()
            || member.len() > MAX_PACKAGE_ID_BYTES
            || !members.contains(member.as_str())
        {
            return Err(CargoMetadataValidationError::InvalidWorkspaceMembers);
        }
    }

    let path_dependencies = validate_path_dependencies(&metadata, &manifest_paths)?;
    let mut declarations = Vec::with_capacity(metadata.workspace_members.len() + 1);
    let mut root_is_member = false;
    for member in &metadata.workspace_members {
        let package = packages
            .get(member.as_str())
            .ok_or(CargoMetadataValidationError::InvalidWorkspaceMembers)?;
        let path = PathBuf::from(&package.manifest_path);
        let is_workspace_root = path == live.manifest_path();
        root_is_member |= is_workspace_root;
        declarations.push(CargoWorkspaceManifestDeclaration {
            member_id: Some(member.clone()),
            path,
            is_workspace_root,
        });
    }
    if !root_is_member {
        declarations.push(CargoWorkspaceManifestDeclaration {
            member_id: None,
            path: live.manifest_path().to_path_buf(),
            is_workspace_root: true,
        });
    }
    declarations.sort_by(|left, right| {
        left.path
            .as_os_str()
            .as_bytes()
            .cmp(right.path.as_os_str().as_bytes())
    });
    let target_namespace = metadata
        .packages
        .iter()
        .map(|package| CargoTargetNamespaceDeclaration {
            package_id: package.id.clone(),
            manifest_path: PathBuf::from(&package.manifest_path),
            targets: package
                .targets
                .iter()
                .map(|target| CargoTargetDeclaration {
                    name: target.name.clone(),
                    kinds: target.kind.clone(),
                    src_path: PathBuf::from(&target.src_path),
                })
                .collect(),
        })
        .collect();
    let package_metadata = metadata
        .packages
        .iter()
        .map(|package| -> Result<_, CargoMetadataValidationError> {
            Ok(CargoPackageMetadataDeclaration {
                package_id: package.id.clone(),
                manifest_path: PathBuf::from(&package.manifest_path),
                readme: required_nullable_string(&package.readme)?,
                license_file: required_nullable_string(&package.license_file)?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ParsedCargoMetadata {
        document: metadata,
        workspace_manifests: declarations,
        path_dependencies,
        package_metadata,
        target_namespace,
    })
}

fn required_nullable_string(
    value: &serde_json::Value,
) -> Result<Option<String>, CargoMetadataValidationError> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(value) => Ok(Some(value.clone())),
        _ => Err(CargoMetadataValidationError::InvalidMetadata),
    }
}

fn validate_path_dependencies(
    metadata: &CargoMetadataDocument,
    reported_manifest_paths: &BTreeSet<Vec<u8>>,
) -> Result<CargoPathDependencyEvidence, CargoMetadataValidationError> {
    let mut declaration_count = 0_usize;
    let mut local_path_count = 0_usize;
    let mut local_path_bytes = 0_usize;
    let mut unique_manifests = BTreeSet::new();
    let mut edges = Vec::new();
    let mut reported_edges = Vec::new();

    for package in &metadata.packages {
        declaration_count = declaration_count
            .checked_add(package.dependencies.len())
            .ok_or(CargoMetadataValidationError::InvalidPathDependencies)?;
        if declaration_count > MAX_PACKAGE_DEPENDENCY_DECLARATIONS {
            return Err(CargoMetadataValidationError::InvalidPathDependencies);
        }
        for dependency in &package.dependencies {
            let Some(path) = dependency.path.as_deref() else {
                let Some(source) = dependency.source.as_str() else {
                    return Err(CargoMetadataValidationError::InvalidPathDependencies);
                };
                if source.is_empty() || source.chars().any(char::is_control) {
                    return Err(CargoMetadataValidationError::InvalidPathDependencies);
                }
                continue;
            };
            if !dependency.source.is_null() {
                return Err(CargoMetadataValidationError::InvalidPathDependencies);
            }
            local_path_count = local_path_count
                .checked_add(1)
                .ok_or(CargoMetadataValidationError::InvalidPathDependencies)?;
            local_path_bytes = local_path_bytes
                .checked_add(path.len())
                .ok_or(CargoMetadataValidationError::InvalidPathDependencies)?;
            if local_path_bytes > MAX_LOCAL_DEPENDENCY_PATH_BYTES {
                return Err(CargoMetadataValidationError::InvalidPathDependencies);
            }
            let dependency_root = Path::new(path);
            let normalized_root: PathBuf = dependency_root.components().collect();
            if path.is_empty()
                || path.chars().any(char::is_control)
                || !dependency_root.is_absolute()
                || normalized_root.as_os_str().as_bytes() != path.as_bytes()
                || dependency_root.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::CurDir | std::path::Component::ParentDir
                    )
                })
            {
                return Err(CargoMetadataValidationError::InvalidPathDependencies);
            }
            let manifest = dependency_root.join("Cargo.toml");
            let manifest_bytes = manifest.as_os_str().as_bytes().to_vec();
            if !reported_manifest_paths.contains(&manifest_bytes) {
                return Err(CargoMetadataValidationError::CargoPathDependenciesUnsupported);
            }
            unique_manifests.insert(manifest_bytes.clone());
            edges.push((package.id.as_bytes().to_vec(), manifest_bytes));
            reported_edges.push(CargoReportedPathDependency {
                owner_manifest: PathBuf::from(&package.manifest_path),
                dependency_manifest: manifest,
            });
        }
    }

    edges.sort();
    let mut digest = Sha256::new();
    digest.update(b"dux-cargo-reported-path-dependencies-v1\0");
    digest.update((declaration_count as u64).to_le_bytes());
    digest.update((local_path_count as u64).to_le_bytes());
    digest.update((unique_manifests.len() as u64).to_le_bytes());
    for (ordinal, (package_id, manifest)) in edges.iter().enumerate() {
        digest.update((ordinal as u64).to_le_bytes());
        digest.update((package_id.len() as u64).to_le_bytes());
        digest.update(package_id);
        digest.update((manifest.len() as u64).to_le_bytes());
        digest.update(manifest);
    }

    Ok(CargoPathDependencyEvidence {
        policy_revision: CARGO_PATH_DEPENDENCY_POLICY_REVISION,
        dependency_declaration_count: declaration_count as u32,
        local_path_dependency_count: local_path_count as u32,
        unique_local_manifest_count: unique_manifests.len() as u32,
        closure_sha256: digest.finalize().into(),
        reported_edges,
    })
}

// This proves that the final document is internally reachable from the
// independently expanded workspace seeds. This function consumes Cargo's
// reported rows; the separate dependency-manifest policy requires those local
// rows to match the retained manifest bytes before a witness can escape.
pub(super) fn validate_workspace_membership_consistency(
    expansion: &CargoWorkspaceGlobExpansion,
    metadata: &CargoMetadataDocument,
) -> Result<CargoWorkspaceMembershipConsistencyEvidence, CargoMetadataValidationError> {
    let workspace_root = Path::new(&metadata.workspace_root);
    let root_manifest = workspace_root.join("Cargo.toml");
    if expansion.root_manifest != root_manifest {
        return Err(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported);
    }

    let mut packages_by_manifest = BTreeMap::<Vec<u8>, &CargoMetadataPackageDocument>::new();
    let mut packages_by_id = BTreeMap::<&str, &CargoMetadataPackageDocument>::new();
    for package in &metadata.packages {
        let manifest = Path::new(&package.manifest_path);
        if packages_by_manifest
            .insert(manifest.as_os_str().as_bytes().to_vec(), package)
            .is_some()
            || packages_by_id
                .insert(package.id.as_str(), package)
                .is_some()
        {
            return Err(CargoMetadataValidationError::InvalidWorkspaceMembers);
        }
    }

    let root_key = root_manifest.as_os_str().as_bytes().to_vec();
    let root_package = packages_by_manifest.get(&root_key).copied();
    if root_package.is_some() != expansion.root_package_present {
        return Err(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported);
    }
    if !expansion.workspace_present
        && (!expansion.root_package_present || packages_by_manifest.len() != 1)
    {
        return Err(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported);
    }

    let mut declared_member_paths = BTreeSet::<Vec<u8>>::new();
    let mut seeds = BTreeSet::<Vec<u8>>::new();
    let mut excluded_members = BTreeSet::<Vec<u8>>::new();
    for pattern in &expansion.members {
        if pattern.used_literal_fallback {
            let fallback = workspace_declaration_path(&pattern.pattern);
            validate_declared_member_path(
                workspace_root,
                &fallback,
                expansion,
                &packages_by_manifest,
                &mut declared_member_paths,
                &mut seeds,
                &mut excluded_members,
            )?;
        } else {
            for relative in &pattern.directory_matches {
                validate_declared_member_path(
                    workspace_root,
                    relative,
                    expansion,
                    &packages_by_manifest,
                    &mut declared_member_paths,
                    &mut seeds,
                    &mut excluded_members,
                )?;
            }
        }
    }

    if expansion.root_package_present {
        let relative_root_manifest = Path::new("Cargo.toml");
        if workspace_path_is_excluded(relative_root_manifest, expansion) {
            return Err(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported);
        }
        seeds.insert(root_key.clone());
    }
    if seeds.is_empty() {
        return Err(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported);
    }

    let mut reachable = BTreeSet::<Vec<u8>>::new();
    let mut pending = seeds.iter().cloned().collect::<Vec<_>>();
    while let Some(manifest_key) = pending.pop() {
        if !reachable.insert(manifest_key.clone()) {
            continue;
        }
        let package = packages_by_manifest
            .get(&manifest_key)
            .copied()
            .ok_or(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported)?;
        for dependency in &package.dependencies {
            let Some(path) = dependency.path.as_deref() else {
                continue;
            };
            let dependency_manifest = Path::new(path).join("Cargo.toml");
            let relative = dependency_manifest
                .strip_prefix(workspace_root)
                .map_err(|_| CargoMetadataValidationError::CargoWorkspaceGlobUnsupported)?;
            if workspace_path_is_excluded(relative, expansion) {
                return Err(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported);
            }
            let dependency_key = dependency_manifest.as_os_str().as_bytes().to_vec();
            if !packages_by_manifest.contains_key(&dependency_key) {
                return Err(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported);
            }
            if !reachable.contains(&dependency_key) {
                pending.push(dependency_key);
            }
        }
    }
    if reachable.len() != packages_by_manifest.len() {
        return Err(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported);
    }

    let expected_defaults = if expansion.default_members_declared {
        let mut defaults = Vec::new();
        for pattern in &expansion.default_members {
            if pattern.used_literal_fallback {
                validate_default_member_path(
                    workspace_root,
                    &workspace_declaration_path(&pattern.pattern),
                    expansion,
                    &declared_member_paths,
                    &packages_by_manifest,
                    &mut defaults,
                )?;
            } else {
                for relative in &pattern.directory_matches {
                    validate_default_member_path(
                        workspace_root,
                        relative,
                        expansion,
                        &declared_member_paths,
                        &packages_by_manifest,
                        &mut defaults,
                    )?;
                }
            }
        }
        defaults
    } else if expansion.root_package_present {
        vec![
            root_package
                .ok_or(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported)?
                .id
                .clone(),
        ]
    } else {
        metadata.workspace_members.clone()
    };
    if expected_defaults != metadata.workspace_default_members {
        return Err(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported);
    }

    let mut digest = Sha256::new();
    digest.update(b"dux-cargo-workspace-membership-consistency-v1\0");
    digest.update([u8::from(expansion.workspace_present)]);
    digest.update([u8::from(expansion.root_package_present)]);
    digest_workspace_rows(&mut digest, &seeds);
    digest_workspace_rows(&mut digest, &excluded_members);
    digest_workspace_rows(&mut digest, &reachable);
    digest.update((expected_defaults.len() as u64).to_le_bytes());
    for member in &expected_defaults {
        digest.update((member.len() as u64).to_le_bytes());
        digest.update(member.as_bytes());
    }
    for member in &metadata.workspace_members {
        if !packages_by_id.contains_key(member.as_str()) {
            return Err(CargoMetadataValidationError::InvalidWorkspaceMembers);
        }
    }
    Ok(CargoWorkspaceMembershipConsistencyEvidence {
        policy_revision: CARGO_WORKSPACE_MEMBERSHIP_CONSISTENCY_POLICY_REVISION,
        seed_package_count: u32::try_from(seeds.len())
            .map_err(|_| CargoMetadataValidationError::CargoWorkspaceGlobUnavailable)?,
        excluded_member_count: u32::try_from(excluded_members.len())
            .map_err(|_| CargoMetadataValidationError::CargoWorkspaceGlobUnavailable)?,
        reachable_package_count: u32::try_from(reachable.len())
            .map_err(|_| CargoMetadataValidationError::CargoWorkspaceGlobUnavailable)?,
        default_member_count: u32::try_from(expected_defaults.len())
            .map_err(|_| CargoMetadataValidationError::CargoWorkspaceGlobUnavailable)?,
        closure_sha256: digest.finalize().into(),
    })
}

fn validate_declared_member_path(
    workspace_root: &Path,
    relative: &Path,
    expansion: &CargoWorkspaceGlobExpansion,
    packages_by_manifest: &BTreeMap<Vec<u8>, &CargoMetadataPackageDocument>,
    declared_member_paths: &mut BTreeSet<Vec<u8>>,
    seeds: &mut BTreeSet<Vec<u8>>,
    excluded_members: &mut BTreeSet<Vec<u8>>,
) -> Result<(), CargoMetadataValidationError> {
    let manifest_relative = relative.join("Cargo.toml");
    declared_member_paths.insert(relative.as_os_str().as_bytes().to_vec());
    let manifest = workspace_root.join(&manifest_relative);
    let key = manifest.as_os_str().as_bytes().to_vec();
    if workspace_path_is_excluded(&manifest_relative, expansion) {
        excluded_members.insert(key);
        return Ok(());
    }
    if !packages_by_manifest.contains_key(&key) {
        return Err(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported);
    }
    seeds.insert(key);
    Ok(())
}

fn validate_default_member_path(
    workspace_root: &Path,
    relative: &Path,
    expansion: &CargoWorkspaceGlobExpansion,
    declared_member_paths: &BTreeSet<Vec<u8>>,
    packages_by_manifest: &BTreeMap<Vec<u8>, &CargoMetadataPackageDocument>,
    defaults: &mut Vec<String>,
) -> Result<(), CargoMetadataValidationError> {
    let manifest = workspace_root.join(relative).join("Cargo.toml");
    let key = manifest.as_os_str().as_bytes().to_vec();
    if let Some(package) = packages_by_manifest.get(&key) {
        defaults.push(package.id.clone());
        return Ok(());
    }
    let relative_key = relative.as_os_str().as_bytes().to_vec();
    if declared_member_paths.contains(&relative_key)
        && workspace_path_is_excluded(&relative.join("Cargo.toml"), expansion)
    {
        return Ok(());
    }
    Err(CargoMetadataValidationError::CargoWorkspaceGlobUnsupported)
}

fn workspace_path_is_excluded(relative: &Path, expansion: &CargoWorkspaceGlobExpansion) -> bool {
    let excluded = expansion
        .excludes
        .iter()
        .map(|declaration| workspace_declaration_path(declaration))
        .any(|declaration| relative.starts_with(declaration));
    let explicitly_listed = expansion
        .members
        .iter()
        .map(|declaration| workspace_declaration_path(&declaration.pattern))
        .any(|declaration| relative.starts_with(declaration));
    excluded && !explicitly_listed
}

fn workspace_declaration_path(declaration: &str) -> PathBuf {
    if declaration == "." {
        PathBuf::new()
    } else {
        PathBuf::from(declaration)
    }
}

fn digest_workspace_rows(digest: &mut Sha256, rows: &BTreeSet<Vec<u8>>) {
    digest.update((rows.len() as u64).to_le_bytes());
    for row in rows {
        digest.update((row.len() as u64).to_le_bytes());
        digest.update(row);
    }
}
