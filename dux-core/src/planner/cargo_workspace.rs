//! Exact workspace-manifest closure for one bounded Cargo metadata result.
//!
//! Cargo reports workspace package manifests only after it has parsed them.
//! Production therefore treats one metadata result as discovery, captures the
//! complete reported manifest set, and accepts only an identical second result
//! while this guard remains armed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::path_validation::{
    CanonicalFileDigestSnapshot, CanonicalScanRoot, capture_regular_file_sha256, capture_scan_root,
    validate_cleanup_path, validate_scan_root,
};

const MANIFEST_POLICY_REVISION: u32 = 1;
pub(super) const MAX_WORKSPACE_MEMBERS: usize = 256;
const MAX_WORKSPACE_MANIFESTS: usize = MAX_WORKSPACE_MEMBERS + 1;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_MANIFEST_CLOSURE_BYTES: usize = 64 * 1024 * 1024;
const MAX_NATIVE_PATH_BYTES: usize = 256 * 1024;
const MAX_WATCHED_DIRECTORIES: usize = 512;
const MAX_DEPENDENCY_DECLARATIONS: usize = 4 * 1024;
const MAX_DEPENDENCY_PATH_BYTES: usize = 256 * 1024;
const DEPENDENCY_MANIFEST_POLICY_REVISION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkspaceManifestObservation {
    manifests: Vec<ObservedWorkspaceManifest>,
    closure_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedWorkspaceManifest {
    declaration: CargoWorkspaceManifestDeclaration,
    file: CanonicalFileDigestSnapshot,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CargoWorkspaceManifestDeclaration {
    pub(super) member_id: Option<String>,
    pub(super) path: PathBuf,
    pub(super) is_workspace_root: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct CargoReportedPathDependency {
    pub(super) owner_manifest: PathBuf,
    pub(super) dependency_manifest: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CargoDependencyManifestEvidence {
    pub(super) policy_revision: u32,
    pub(super) local_dependency_count: u32,
    pub(super) unique_local_manifest_count: u32,
    pub(super) closure_sha256: [u8; 32],
}

/// Non-cloneable exact manifest bytes and mutation fence for one workspace.
pub(super) struct CargoWorkspaceManifestGuard {
    project_root: CanonicalScanRoot,
    observation: WorkspaceManifestObservation,
    fence: platform::WorkspaceMutationFence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CargoWorkspaceManifestEvidence {
    pub(super) policy_revision: u32,
    pub(super) workspace_member_count: u32,
    pub(super) manifest_count: u32,
    pub(super) closure_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CargoWorkspaceManifestError {
    Invalid,
    Changed,
    Unavailable,
}

impl CargoWorkspaceManifestGuard {
    pub(super) fn capture(
        project_root: &CanonicalScanRoot,
        manifest_paths: &[CargoWorkspaceManifestDeclaration],
    ) -> Result<Self, CargoWorkspaceManifestError> {
        let observation = capture_observation(project_root, manifest_paths)?;
        let fence = platform::WorkspaceMutationFence::new(project_root, &observation.manifests)?;
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
        manifest_paths: &[CargoWorkspaceManifestDeclaration],
    ) -> Result<Self, CargoWorkspaceManifestError> {
        let observation = capture_observation(project_root, manifest_paths)?;
        Ok(Self {
            project_root: project_root.clone(),
            observation,
            fence: platform::WorkspaceMutationFence::disabled_for_test(),
        })
    }

    pub(super) fn poll(&self) -> Result<(), CargoWorkspaceManifestError> {
        self.fence.poll()
    }

    pub(super) fn revalidate(&self) -> Result<(), CargoWorkspaceManifestError> {
        self.poll()?;
        let paths: Vec<CargoWorkspaceManifestDeclaration> = self
            .observation
            .manifests
            .iter()
            .map(|manifest| manifest.declaration.clone())
            .collect();
        let current = capture_observation(&self.project_root, &paths)
            .map_err(|_| CargoWorkspaceManifestError::Changed)?;
        if current != self.observation {
            return Err(CargoWorkspaceManifestError::Changed);
        }
        self.poll()
    }

    pub(super) fn evidence(
        &self,
    ) -> Result<CargoWorkspaceManifestEvidence, CargoWorkspaceManifestError> {
        self.revalidate()?;
        let manifest_count = u32::try_from(self.observation.manifests.len())
            .map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
        let workspace_member_count = u32::try_from(
            self.observation
                .manifests
                .iter()
                .filter(|manifest| manifest.declaration.member_id.is_some())
                .count(),
        )
        .map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
        Ok(CargoWorkspaceManifestEvidence {
            policy_revision: MANIFEST_POLICY_REVISION,
            workspace_member_count,
            manifest_count,
            closure_sha256: self.observation.closure_sha256,
        })
    }

    pub(super) fn dependency_evidence(
        &self,
        reported: &[CargoReportedPathDependency],
    ) -> Result<CargoDependencyManifestEvidence, CargoWorkspaceManifestError> {
        self.revalidate()?;
        let evidence = independently_validate_path_dependencies(&self.observation, reported)?;
        self.revalidate()?;
        Ok(evidence)
    }
}

fn capture_observation(
    project_root: &CanonicalScanRoot,
    manifest_paths: &[CargoWorkspaceManifestDeclaration],
) -> Result<WorkspaceManifestObservation, CargoWorkspaceManifestError> {
    if manifest_paths.is_empty() || manifest_paths.len() > MAX_WORKSPACE_MANIFESTS {
        return Err(CargoWorkspaceManifestError::Invalid);
    }
    let current_root = capture_exact_directory(project_root.canonical_path())?;
    if &current_root != project_root {
        return Err(CargoWorkspaceManifestError::Changed);
    }

    let lexical_root = validate_scan_root(project_root.canonical_path())
        .map_err(|_| CargoWorkspaceManifestError::Invalid)?;
    let workspace_root_count = manifest_paths
        .iter()
        .filter(|manifest| manifest.is_workspace_root)
        .count();
    let workspace_member_count = manifest_paths
        .iter()
        .filter(|manifest| manifest.member_id.is_some())
        .count();
    if workspace_root_count != 1
        || workspace_member_count == 0
        || workspace_member_count > MAX_WORKSPACE_MEMBERS
    {
        return Err(CargoWorkspaceManifestError::Invalid);
    }
    let mut member_ids = BTreeSet::new();
    if manifest_paths.iter().any(|manifest| {
        manifest
            .member_id
            .as_ref()
            .is_some_and(|member_id| !member_ids.insert(member_id.as_bytes().to_vec()))
    }) {
        return Err(CargoWorkspaceManifestError::Invalid);
    }

    let mut paths = manifest_paths.to_vec();
    paths.sort_by(|left, right| {
        left.path
            .as_os_str()
            .as_bytes()
            .cmp(right.path.as_os_str().as_bytes())
    });
    if paths
        .windows(2)
        .any(|pair| pair[0].path.as_os_str().as_bytes() == pair[1].path.as_os_str().as_bytes())
    {
        return Err(CargoWorkspaceManifestError::Invalid);
    }

    let mut manifests = Vec::with_capacity(paths.len());
    let mut identities = BTreeSet::new();
    let mut total_bytes = 0_usize;
    let mut native_path_bytes = 0_usize;
    for declaration in paths {
        let path = &declaration.path;
        if !path.is_absolute()
            || path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml")
        {
            return Err(CargoWorkspaceManifestError::Invalid);
        }
        charge_path(&mut native_path_bytes, path)?;
        let lexical = validate_cleanup_path(&lexical_root, path)
            .map_err(|_| CargoWorkspaceManifestError::Invalid)?;
        let manifest = capture_regular_file_sha256(project_root, lexical, MAX_MANIFEST_BYTES)
            .map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
        if manifest.path().hard_link_count() != 1 {
            return Err(CargoWorkspaceManifestError::Invalid);
        }
        let identity = manifest.path().target_identity();
        if !identities.insert((identity.volume(), identity.object())) {
            return Err(CargoWorkspaceManifestError::Invalid);
        }
        let byte_length = usize::try_from(manifest.byte_length())
            .map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
        total_bytes = total_bytes
            .checked_add(byte_length)
            .ok_or(CargoWorkspaceManifestError::Unavailable)?;
        if total_bytes > MAX_MANIFEST_CLOSURE_BYTES {
            return Err(CargoWorkspaceManifestError::Unavailable);
        }
        let bytes = fs::read(path).map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
        let bytes_sha256: [u8; 32] = Sha256::digest(&bytes).into();
        if bytes.len() != byte_length || bytes_sha256 != manifest.sha256() {
            return Err(CargoWorkspaceManifestError::Changed);
        }
        manifests.push(ObservedWorkspaceManifest {
            declaration,
            file: manifest,
            bytes,
        });
    }

    let closure_sha256 = digest_observation(&manifests);
    Ok(WorkspaceManifestObservation {
        manifests,
        closure_sha256,
    })
}

fn independently_validate_path_dependencies(
    observation: &WorkspaceManifestObservation,
    reported: &[CargoReportedPathDependency],
) -> Result<CargoDependencyManifestEvidence, CargoWorkspaceManifestError> {
    if reported.len() > MAX_DEPENDENCY_DECLARATIONS {
        return Err(CargoWorkspaceManifestError::Invalid);
    }
    let manifests = observation
        .manifests
        .iter()
        .map(|manifest| {
            (
                manifest.file.path().canonical_path().to_path_buf(),
                manifest,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let root = observation
        .manifests
        .iter()
        .find(|manifest| manifest.declaration.is_workspace_root)
        .ok_or(CargoWorkspaceManifestError::Invalid)?;
    let root_document = parse_manifest(&root.bytes)?;
    let workspace_dependencies_value = root_document
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|workspace| workspace.get("dependencies"));
    let workspace_dependencies = workspace_dependencies_value
        .map(|value| value.as_table().ok_or(CargoWorkspaceManifestError::Invalid))
        .transpose()?;

    let mut independent = Vec::new();
    let mut path_bytes = 0_usize;
    for manifest in &observation.manifests {
        let document = parse_manifest(&manifest.bytes)?;
        collect_manifest_dependencies(
            &document,
            manifest.file.path().canonical_path(),
            root.file.path().canonical_path(),
            workspace_dependencies,
            &manifests,
            &mut independent,
            &mut path_bytes,
        )?;
    }
    independent.sort();
    let mut expected = reported.to_vec();
    expected.sort();
    if independent != expected {
        return Err(CargoWorkspaceManifestError::Invalid);
    }

    let unique = independent
        .iter()
        .map(|edge| edge.dependency_manifest.as_os_str().as_bytes().to_vec())
        .collect::<BTreeSet<_>>();
    let mut digest = Sha256::new();
    digest.update(b"dux-cargo-independent-path-dependencies-v1\0");
    digest.update((independent.len() as u64).to_le_bytes());
    digest.update((unique.len() as u64).to_le_bytes());
    for (ordinal, edge) in independent.iter().enumerate() {
        digest.update((ordinal as u64).to_le_bytes());
        digest_path(&mut digest, &edge.owner_manifest);
        digest_path(&mut digest, &edge.dependency_manifest);
    }
    Ok(CargoDependencyManifestEvidence {
        policy_revision: DEPENDENCY_MANIFEST_POLICY_REVISION,
        local_dependency_count: independent.len() as u32,
        unique_local_manifest_count: unique.len() as u32,
        closure_sha256: digest.finalize().into(),
    })
}

fn parse_manifest(bytes: &[u8]) -> Result<toml::Value, CargoWorkspaceManifestError> {
    let text = std::str::from_utf8(bytes).map_err(|_| CargoWorkspaceManifestError::Invalid)?;
    let value =
        toml::from_str::<toml::Value>(text).map_err(|_| CargoWorkspaceManifestError::Invalid)?;
    if !value.is_table() {
        return Err(CargoWorkspaceManifestError::Invalid);
    }
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn collect_manifest_dependencies(
    document: &toml::Value,
    owner_manifest: &Path,
    root_manifest: &Path,
    workspace_dependencies: Option<&toml::map::Map<String, toml::Value>>,
    manifests: &BTreeMap<PathBuf, &ObservedWorkspaceManifest>,
    edges: &mut Vec<CargoReportedPathDependency>,
    path_bytes: &mut usize,
) -> Result<(), CargoWorkspaceManifestError> {
    let table = document
        .as_table()
        .ok_or(CargoWorkspaceManifestError::Invalid)?;
    for name in dependency_table_names() {
        if let Some(dependencies) = table.get(name) {
            collect_dependency_table(
                dependencies,
                owner_manifest,
                root_manifest,
                workspace_dependencies,
                manifests,
                edges,
                path_bytes,
            )?;
        }
    }
    if let Some(targets) = table.get("target") {
        let targets = targets
            .as_table()
            .ok_or(CargoWorkspaceManifestError::Invalid)?;
        for target in targets.values() {
            let target = target
                .as_table()
                .ok_or(CargoWorkspaceManifestError::Invalid)?;
            for name in dependency_table_names() {
                if let Some(dependencies) = target.get(name) {
                    collect_dependency_table(
                        dependencies,
                        owner_manifest,
                        root_manifest,
                        workspace_dependencies,
                        manifests,
                        edges,
                        path_bytes,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn dependency_table_names() -> [&'static str; 5] {
    [
        "dependencies",
        "dev-dependencies",
        "dev_dependencies",
        "build-dependencies",
        "build_dependencies",
    ]
}

#[allow(clippy::too_many_arguments)]
fn collect_dependency_table(
    dependencies: &toml::Value,
    owner_manifest: &Path,
    root_manifest: &Path,
    workspace_dependencies: Option<&toml::map::Map<String, toml::Value>>,
    manifests: &BTreeMap<PathBuf, &ObservedWorkspaceManifest>,
    edges: &mut Vec<CargoReportedPathDependency>,
    path_bytes: &mut usize,
) -> Result<(), CargoWorkspaceManifestError> {
    let dependencies = dependencies
        .as_table()
        .ok_or(CargoWorkspaceManifestError::Invalid)?;
    for (name, dependency) in dependencies {
        let Some((path, relative_to)) = dependency_path(
            name,
            dependency,
            workspace_dependencies,
            owner_manifest,
            root_manifest,
        )?
        else {
            continue;
        };
        *path_bytes = path_bytes
            .checked_add(path.len())
            .ok_or(CargoWorkspaceManifestError::Unavailable)?;
        if *path_bytes > MAX_DEPENDENCY_PATH_BYTES || edges.len() >= MAX_DEPENDENCY_DECLARATIONS {
            return Err(CargoWorkspaceManifestError::Unavailable);
        }
        let parent = relative_to
            .parent()
            .ok_or(CargoWorkspaceManifestError::Invalid)?;
        let dependency_root = normalize_path(parent, Path::new(path))?;
        let dependency_manifest = dependency_root.join("Cargo.toml");
        if !manifests.contains_key(&dependency_manifest) {
            return Err(CargoWorkspaceManifestError::Invalid);
        }
        edges.push(CargoReportedPathDependency {
            owner_manifest: owner_manifest.to_path_buf(),
            dependency_manifest,
        });
    }
    Ok(())
}

fn dependency_path<'a>(
    name: &str,
    dependency: &'a toml::Value,
    workspace_dependencies: Option<&'a toml::map::Map<String, toml::Value>>,
    owner_manifest: &'a Path,
    root_manifest: &'a Path,
) -> Result<Option<(&'a str, &'a Path)>, CargoWorkspaceManifestError> {
    if dependency.is_str() {
        return Ok(None);
    }
    let table = dependency
        .as_table()
        .ok_or(CargoWorkspaceManifestError::Invalid)?;
    let workspace = match table.get("workspace") {
        Some(value) => Some(
            value
                .as_bool()
                .ok_or(CargoWorkspaceManifestError::Invalid)?,
        ),
        None => None,
    };
    if workspace == Some(true) {
        if table.contains_key("path") {
            return Err(CargoWorkspaceManifestError::Invalid);
        }
        let inherited = workspace_dependencies
            .and_then(|dependencies| dependencies.get(name))
            .ok_or(CargoWorkspaceManifestError::Invalid)?;
        return direct_dependency_path(inherited, root_manifest);
    }
    direct_dependency_path(dependency, owner_manifest)
}

fn direct_dependency_path<'a>(
    dependency: &'a toml::Value,
    relative_to: &'a Path,
) -> Result<Option<(&'a str, &'a Path)>, CargoWorkspaceManifestError> {
    if dependency.is_str() {
        return Ok(None);
    }
    let table = dependency
        .as_table()
        .ok_or(CargoWorkspaceManifestError::Invalid)?;
    match table.get("path") {
        Some(value) => {
            let path = value
                .as_str()
                .filter(|path| !path.is_empty() && !path.chars().any(char::is_control))
                .ok_or(CargoWorkspaceManifestError::Invalid)?;
            Ok(Some((path, relative_to)))
        }
        None => Ok(None),
    }
}

fn normalize_path(base: &Path, path: &Path) -> Result<PathBuf, CargoWorkspaceManifestError> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::RootDir => normalized.push(Component::RootDir.as_os_str()),
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(CargoWorkspaceManifestError::Invalid);
                }
            }
            Component::Prefix(_) => return Err(CargoWorkspaceManifestError::Invalid),
        }
    }
    if !normalized.is_absolute() {
        return Err(CargoWorkspaceManifestError::Invalid);
    }
    Ok(normalized)
}

fn digest_path(digest: &mut Sha256, path: &Path) {
    let bytes = path.as_os_str().as_bytes();
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}

fn capture_exact_directory(path: &Path) -> Result<CanonicalScanRoot, CargoWorkspaceManifestError> {
    let lexical = validate_scan_root(path).map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
    capture_scan_root(lexical).map_err(|_| CargoWorkspaceManifestError::Unavailable)
}

fn charge_path(total: &mut usize, path: &Path) -> Result<(), CargoWorkspaceManifestError> {
    *total = total
        .checked_add(path.as_os_str().as_bytes().len())
        .ok_or(CargoWorkspaceManifestError::Unavailable)?;
    if *total > MAX_NATIVE_PATH_BYTES {
        return Err(CargoWorkspaceManifestError::Unavailable);
    }
    Ok(())
}

fn digest_observation(manifests: &[ObservedWorkspaceManifest]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"dux-cargo-workspace-manifest-closure-v1\0");
    digest.update((manifests.len() as u64).to_le_bytes());
    for manifest in manifests {
        digest.update([u8::from(manifest.declaration.is_workspace_root)]);
        match manifest.declaration.member_id.as_ref() {
            Some(member_id) => {
                digest.update([1]);
                digest.update((member_id.len() as u64).to_le_bytes());
                digest.update(member_id.as_bytes());
            }
            None => digest.update([0]),
        }
        let path = manifest.file.path().canonical_path().as_os_str().as_bytes();
        digest.update((path.len() as u64).to_le_bytes());
        digest.update(path);
        let identity = manifest.file.path().target_identity();
        digest.update(identity.volume().to_le_bytes());
        digest.update(identity.object().to_le_bytes());
        digest.update(manifest.file.byte_length().to_le_bytes());
        digest.update(manifest.file.sha256());
    }
    digest.finalize().into()
}

#[cfg(target_os = "macos")]
mod platform {
    use std::collections::BTreeMap;
    use std::fs;
    use std::mem::MaybeUninit;
    use std::os::fd::{AsFd, AsRawFd, OwnedFd};

    use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl, open};
    use nix::libc;
    use nix::mount::MntFlags;
    use nix::sys::event::{EvFlags, EventFilter, FilterFlag, KEvent, Kqueue};
    use nix::sys::stat::{Mode, fstat};
    use nix::sys::statfs::fstatfs;

    use super::{
        CanonicalScanRoot, CargoWorkspaceManifestError, MAX_WATCHED_DIRECTORIES,
        ObservedWorkspaceManifest, capture_exact_directory,
    };
    use crate::path_validation::FilesystemIdentity;

    pub(super) struct WorkspaceMutationFence {
        queue: Kqueue,
        _files: Vec<OwnedFd>,
        _directories: Vec<OwnedFd>,
    }

    impl WorkspaceMutationFence {
        #[cfg(test)]
        pub(super) fn disabled_for_test() -> Self {
            let queue = Kqueue::new().expect("test kqueue must be available");
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .expect("test kqueue must support close-on-exec");
            Self {
                queue,
                _files: Vec::new(),
                _directories: Vec::new(),
            }
        }

        pub(super) fn new(
            project_root: &CanonicalScanRoot,
            manifests: &[ObservedWorkspaceManifest],
        ) -> Result<Self, CargoWorkspaceManifestError> {
            let queue = Kqueue::new().map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .map_err(|_| CargoWorkspaceManifestError::Unavailable)?;

            let expected_directories = watched_directories(project_root, manifests)?;
            require_descriptor_budget(
                manifests
                    .len()
                    .checked_add(expected_directories.len())
                    .and_then(|count| count.checked_add(1))
                    .ok_or(CargoWorkspaceManifestError::Unavailable)?,
            )?;
            let files: Vec<OwnedFd> = manifests
                .iter()
                .map(open_watch_file)
                .collect::<Result<_, _>>()?;
            let directories: Vec<OwnedFd> = expected_directories
                .iter()
                .map(open_watch_directory)
                .collect::<Result<_, _>>()?;

            let file_flags = FilterFlag::NOTE_DELETE
                | FilterFlag::NOTE_WRITE
                | FilterFlag::NOTE_EXTEND
                | FilterFlag::NOTE_ATTRIB
                | FilterFlag::NOTE_LINK
                | FilterFlag::NOTE_RENAME
                | FilterFlag::NOTE_REVOKE;
            let directory_flags =
                FilterFlag::NOTE_DELETE | FilterFlag::NOTE_RENAME | FilterFlag::NOTE_REVOKE;
            let changes: Vec<KEvent> = files
                .iter()
                .map(|file| vnode_event(file.as_raw_fd(), file_flags))
                .chain(
                    directories
                        .iter()
                        .map(|directory| vnode_event(directory.as_raw_fd(), directory_flags)),
                )
                .collect();
            let mut events = vec![empty_event(); changes.len().max(1)];
            let count = queue
                .kevent(&changes, &mut events, Some(zero_timeout()))
                .map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
            if events[..count]
                .iter()
                .any(|event| event.flags().contains(EvFlags::EV_ERROR))
            {
                return Err(CargoWorkspaceManifestError::Unavailable);
            }
            if count != 0 {
                return Err(CargoWorkspaceManifestError::Changed);
            }
            Ok(Self {
                queue,
                _files: files,
                _directories: directories,
            })
        }

        pub(super) fn poll(&self) -> Result<(), CargoWorkspaceManifestError> {
            let mut event = [empty_event()];
            let count = self
                .queue
                .kevent(&[], &mut event, Some(zero_timeout()))
                .map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
            if count == 0 {
                Ok(())
            } else {
                Err(CargoWorkspaceManifestError::Changed)
            }
        }
    }

    fn watched_directories(
        project_root: &CanonicalScanRoot,
        manifests: &[ObservedWorkspaceManifest],
    ) -> Result<Vec<CanonicalScanRoot>, CargoWorkspaceManifestError> {
        let mut by_identity = BTreeMap::<(u64, u128), CanonicalScanRoot>::new();
        for ancestor in project_root.canonical_path().ancestors() {
            insert_directory(&mut by_identity, capture_exact_directory(ancestor)?)?;
        }
        for manifest in manifests {
            let parent = manifest
                .file
                .path()
                .canonical_path()
                .parent()
                .ok_or(CargoWorkspaceManifestError::Invalid)?;
            let relative_parent = parent
                .strip_prefix(project_root.canonical_path())
                .map_err(|_| CargoWorkspaceManifestError::Invalid)?;
            let mut current = project_root.canonical_path().to_path_buf();
            for component in relative_parent.components() {
                current.push(component.as_os_str());
                insert_directory(&mut by_identity, capture_exact_directory(&current)?)?;
            }
        }
        Ok(by_identity.into_values().collect())
    }

    fn insert_directory(
        directories: &mut BTreeMap<(u64, u128), CanonicalScanRoot>,
        directory: CanonicalScanRoot,
    ) -> Result<(), CargoWorkspaceManifestError> {
        let identity = directory.identity();
        directories
            .entry((identity.volume(), identity.object()))
            .or_insert(directory);
        if directories.len() > MAX_WATCHED_DIRECTORIES {
            return Err(CargoWorkspaceManifestError::Unavailable);
        }
        Ok(())
    }

    fn open_watch_file(
        expected: &ObservedWorkspaceManifest,
    ) -> Result<OwnedFd, CargoWorkspaceManifestError> {
        let file = open(
            expected.file.path().canonical_path(),
            OFlag::from_bits_retain(libc::O_EVTONLY) | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
        require_reviewed_filesystem(&file)?;
        let status = fstat(&file).map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
        if !identity_matches(&status, expected.file.path().target_identity())
            || status.st_nlink != 1
        {
            return Err(CargoWorkspaceManifestError::Changed);
        }
        Ok(file)
    }

    fn open_watch_directory(
        expected: &CanonicalScanRoot,
    ) -> Result<OwnedFd, CargoWorkspaceManifestError> {
        let directory = open(
            expected.canonical_path(),
            OFlag::from_bits_retain(libc::O_EVTONLY) | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
        require_reviewed_filesystem(&directory)?;
        let status = fstat(&directory).map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
        if !identity_matches(&status, expected.identity()) {
            return Err(CargoWorkspaceManifestError::Changed);
        }
        Ok(directory)
    }

    fn require_reviewed_filesystem(
        descriptor: &impl AsFd,
    ) -> Result<(), CargoWorkspaceManifestError> {
        let status = fstatfs(descriptor).map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
        if status.filesystem_type_name().eq_ignore_ascii_case("apfs")
            && status.flags().contains(MntFlags::MNT_LOCAL)
        {
            Ok(())
        } else {
            Err(CargoWorkspaceManifestError::Unavailable)
        }
    }

    fn require_descriptor_budget(retained: usize) -> Result<(), CargoWorkspaceManifestError> {
        const RESERVE_FOR_PROCESS_AND_LAUNCH: u64 = 128;

        let mut limit = MaybeUninit::<libc::rlimit>::zeroed();
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) } == -1 {
            return Err(CargoWorkspaceManifestError::Unavailable);
        }
        let soft_limit = unsafe { limit.assume_init() }.rlim_cur;
        if soft_limit == libc::RLIM_INFINITY {
            return Ok(());
        }
        let open_descriptors = fs::read_dir("/dev/fd")
            .map_err(|_| CargoWorkspaceManifestError::Unavailable)?
            .try_fold(0_u64, |count, entry| {
                entry
                    .map(|_| count.saturating_add(1))
                    .map_err(|_| CargoWorkspaceManifestError::Unavailable)
            })?;
        let retained =
            u64::try_from(retained).map_err(|_| CargoWorkspaceManifestError::Unavailable)?;
        if descriptor_budget_fits(
            soft_limit,
            open_descriptors,
            retained,
            RESERVE_FOR_PROCESS_AND_LAUNCH,
        ) {
            Ok(())
        } else {
            Err(CargoWorkspaceManifestError::Unavailable)
        }
    }

    pub(super) fn descriptor_budget_fits(
        soft_limit: u64,
        open_descriptors: u64,
        retained: u64,
        reserve: u64,
    ) -> bool {
        open_descriptors
            .checked_add(retained)
            .and_then(|count| count.checked_add(reserve))
            .is_some_and(|required| required <= soft_limit)
    }

    fn identity_matches(status: &libc::stat, expected: FilesystemIdentity) -> bool {
        status.st_dev as u64 == expected.volume() && u128::from(status.st_ino) == expected.object()
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

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{CanonicalScanRoot, CargoWorkspaceManifestError, ObservedWorkspaceManifest};

    pub(super) struct WorkspaceMutationFence;

    impl WorkspaceMutationFence {
        #[cfg(test)]
        pub(super) fn disabled_for_test() -> Self {
            Self
        }

        pub(super) fn new(
            _project_root: &CanonicalScanRoot,
            _manifests: &[ObservedWorkspaceManifest],
        ) -> Result<Self, CargoWorkspaceManifestError> {
            Ok(Self)
        }

        pub(super) fn poll(&self) -> Result<(), CargoWorkspaceManifestError> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    fn fixture() -> (
        TempDir,
        CanonicalScanRoot,
        Vec<CargoWorkspaceManifestDeclaration>,
    ) {
        let temp = TempDir::new().unwrap();
        let root_path = fs::canonicalize(temp.path()).unwrap().join("workspace");
        let member = root_path.join("member");
        fs::create_dir_all(&member).unwrap();
        let root_manifest = root_path.join("Cargo.toml");
        let member_manifest = member.join("Cargo.toml");
        fs::write(&root_manifest, "[workspace]\nmembers = [\"member\"]\n").unwrap();
        fs::write(
            &member_manifest,
            "[package]\nname = \"member\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let root = capture_exact_directory(&root_path).unwrap();
        (
            temp,
            root,
            vec![
                CargoWorkspaceManifestDeclaration {
                    member_id: None,
                    path: root_manifest,
                    is_workspace_root: true,
                },
                CargoWorkspaceManifestDeclaration {
                    member_id: Some("member 0.1.0".to_owned()),
                    path: member_manifest,
                    is_workspace_root: false,
                },
            ],
        )
    }

    #[test]
    fn bounded_manifest_closure_is_digest_bound_and_revalidated() {
        let (_temp, root, manifests) = fixture();
        let guard =
            CargoWorkspaceManifestGuard::capture_unfenced_for_test(&root, &manifests).unwrap();
        let evidence = guard.evidence().unwrap();
        assert_eq!(evidence.policy_revision, MANIFEST_POLICY_REVISION);
        assert_eq!(evidence.workspace_member_count, 1);
        assert_eq!(evidence.manifest_count, 2);
        assert_ne!(evidence.closure_sha256, [0; 32]);

        fs::write(
            &manifests[1].path,
            "[package]\nname = \"changed\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert_eq!(
            guard.revalidate(),
            Err(CargoWorkspaceManifestError::Changed)
        );
    }

    #[test]
    fn duplicate_outside_and_non_manifest_paths_are_rejected() {
        let (temp, root, manifests) = fixture();
        assert!(matches!(
            CargoWorkspaceManifestGuard::capture_unfenced_for_test(
                &root,
                &[manifests[0].clone(), manifests[0].clone()],
            ),
            Err(CargoWorkspaceManifestError::Invalid)
        ));

        let outside = fs::canonicalize(temp.path()).unwrap().join("Cargo.toml");
        fs::write(&outside, "[workspace]\n").unwrap();
        assert!(matches!(
            CargoWorkspaceManifestGuard::capture_unfenced_for_test(
                &root,
                &[CargoWorkspaceManifestDeclaration {
                    member_id: Some("outside".to_owned()),
                    path: outside,
                    is_workspace_root: true,
                }],
            ),
            Err(CargoWorkspaceManifestError::Invalid)
        ));

        let wrong_name = root.canonical_path().join("member/not-a-manifest.toml");
        fs::write(&wrong_name, "[package]\n").unwrap();
        assert!(matches!(
            CargoWorkspaceManifestGuard::capture_unfenced_for_test(
                &root,
                &[CargoWorkspaceManifestDeclaration {
                    member_id: Some("wrong".to_owned()),
                    path: wrong_name,
                    is_workspace_root: true,
                }],
            ),
            Err(CargoWorkspaceManifestError::Invalid)
        ));
    }

    #[test]
    fn symlink_and_hard_link_manifest_aliases_are_rejected() {
        let (_temp, root, manifests) = fixture();
        let alias_directory = root.canonical_path().join("alias");
        fs::create_dir(&alias_directory).unwrap();

        let symlink_manifest = alias_directory.join("Cargo.toml");
        symlink(&manifests[1].path, &symlink_manifest).unwrap();
        let mut symlink_declarations = manifests.clone();
        symlink_declarations[1].path = symlink_manifest;
        assert!(matches!(
            CargoWorkspaceManifestGuard::capture_unfenced_for_test(&root, &symlink_declarations),
            Err(CargoWorkspaceManifestError::Unavailable)
                | Err(CargoWorkspaceManifestError::Invalid)
        ));

        let hard_alias_directory = root.canonical_path().join("hard-alias");
        fs::create_dir(&hard_alias_directory).unwrap();
        let hard_link_manifest = hard_alias_directory.join("Cargo.toml");
        fs::hard_link(&manifests[1].path, &hard_link_manifest).unwrap();
        let mut hard_link_declarations = manifests;
        hard_link_declarations[1].path = hard_link_manifest;
        assert!(matches!(
            CargoWorkspaceManifestGuard::capture_unfenced_for_test(&root, &hard_link_declarations),
            Err(CargoWorkspaceManifestError::Invalid)
        ));
    }

    #[test]
    fn workspace_member_bound_accepts_n_and_rejects_n_plus_one() {
        let temp = TempDir::new().unwrap();
        let root_path = fs::canonicalize(temp.path()).unwrap().join("workspace");
        fs::create_dir(&root_path).unwrap();
        let root_manifest = root_path.join("Cargo.toml");
        fs::write(&root_manifest, "[workspace]\n").unwrap();
        let root = capture_exact_directory(&root_path).unwrap();
        let mut declarations = vec![CargoWorkspaceManifestDeclaration {
            member_id: None,
            path: root_manifest,
            is_workspace_root: true,
        }];
        for index in 0..MAX_WORKSPACE_MEMBERS {
            let directory = root_path.join(format!("member-{index}"));
            fs::create_dir(&directory).unwrap();
            let manifest = directory.join("Cargo.toml");
            fs::write(&manifest, "[package]\nname = \"member\"\n").unwrap();
            declarations.push(CargoWorkspaceManifestDeclaration {
                member_id: Some(format!("member-{index}")),
                path: manifest,
                is_workspace_root: false,
            });
        }
        let guard =
            CargoWorkspaceManifestGuard::capture_unfenced_for_test(&root, &declarations).unwrap();
        assert_eq!(
            guard.evidence().unwrap().workspace_member_count,
            MAX_WORKSPACE_MEMBERS as u32
        );
        #[cfg(target_os = "macos")]
        match CargoWorkspaceManifestGuard::capture(&root, &declarations) {
            Ok(armed) => assert_eq!(
                armed.evidence().unwrap().workspace_member_count,
                MAX_WORKSPACE_MEMBERS as u32
            ),
            Err(CargoWorkspaceManifestError::Unavailable) => {
                // The declared limit is a parser bound, not a promise that the
                // process has enough descriptors to arm every retained vnode.
            }
            Err(error) => panic!("unexpected armed capture result: {error:?}"),
        }

        let extra = root_path.join("extra/Cargo.toml");
        declarations.push(CargoWorkspaceManifestDeclaration {
            member_id: Some("extra".to_owned()),
            path: extra,
            is_workspace_root: false,
        });
        assert!(matches!(
            CargoWorkspaceManifestGuard::capture_unfenced_for_test(&root, &declarations),
            Err(CargoWorkspaceManifestError::Invalid)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn descriptor_budget_reserves_launch_capacity_and_rejects_overflow() {
        assert!(super::platform::descriptor_budget_fits(256, 8, 120, 128));
        assert!(!super::platform::descriptor_budget_fits(256, 9, 120, 128));
        assert!(!super::platform::descriptor_budget_fits(
            u64::MAX,
            u64::MAX,
            1,
            0
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn same_inode_write_and_restore_is_a_terminal_vnode_event() {
        let (_temp, root, manifests) = fixture();
        let original = fs::read(&manifests[1].path).unwrap();
        let guard = CargoWorkspaceManifestGuard::capture(&root, &manifests).unwrap();
        fs::write(&manifests[1].path, b"changed").unwrap();
        fs::write(&manifests[1].path, original).unwrap();
        assert_eq!(guard.poll(), Err(CargoWorkspaceManifestError::Changed));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test renames only a TempDir-owned outer ancestor to prove complete manifest-path ancestry fencing"
    )]
    fn higher_workspace_ancestor_rename_is_terminal() {
        let temp = TempDir::new().unwrap();
        let canonical_temp = fs::canonicalize(temp.path()).unwrap();
        let outer = canonical_temp.join("outer");
        let workspace = outer.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let manifest = workspace.join("Cargo.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"root\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let root = capture_exact_directory(&workspace).unwrap();
        let declarations = [CargoWorkspaceManifestDeclaration {
            member_id: Some("root".to_owned()),
            path: manifest,
            is_workspace_root: true,
        }];
        let guard = CargoWorkspaceManifestGuard::capture(&root, &declarations).unwrap();

        let displaced = canonical_temp.join("displaced");
        // DUX-DESTRUCTIVE: allow=test-cargo-workspace-ancestor-rename -- move only this TempDir-owned outer ancestor to prove an absolute member path cannot be redirected without a terminal event
        fs::rename(&outer, &displaced).unwrap();
        assert_eq!(guard.poll(), Err(CargoWorkspaceManifestError::Changed));
    }
}
