//! Bounded Cargo 1.96 package target/source/build discovery closure.
//!
//! Cargo serializes package targets after probing a finite conventional
//! namespace. One metadata pass is therefore discovery only. This guard binds
//! the reported target paths and that conventional namespace before an
//! identical accepted pass runs.

use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::path_validation::{
    CanonicalScanRoot, FilesystemEntryKind, FilesystemIdentity, capture_path_snapshot,
    capture_scan_root, validate_cleanup_path, validate_scan_root,
};

const TARGET_NAMESPACE_POLICY_REVISION: u32 = 1;
const MAX_PACKAGES: usize = 256;
const MAX_TARGETS: usize = 4_096;
const MAX_KINDS_PER_TARGET: usize = 16;
const MAX_NAMESPACE_RECORDS: usize = 16_384;
const MAX_NATIVE_PATH_BYTES: usize = 2 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 512 * 1024;
const MAX_WATCHED_OBJECTS: usize = 24_576;
const DISCOVERY_DIRECTORIES: [&str; 4] = ["src/bin", "examples", "tests", "benches"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CargoTargetDeclaration {
    pub(super) name: String,
    pub(super) kinds: Vec<String>,
    pub(super) src_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CargoTargetNamespaceDeclaration {
    pub(super) package_id: String,
    pub(super) manifest_path: PathBuf,
    pub(super) targets: Vec<CargoTargetDeclaration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TargetNamespaceObservation {
    declarations: Vec<CargoTargetNamespaceDeclaration>,
    records: Vec<NamespaceRecord>,
    target_count: usize,
    closure_sha256: [u8; 32],
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

/// Non-cloneable exact namespace state and mutation fence for one result.
pub(super) struct CargoTargetNamespaceGuard {
    project_root: CanonicalScanRoot,
    observation: TargetNamespaceObservation,
    fence: platform::TargetNamespaceMutationFence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CargoTargetNamespaceEvidence {
    pub(super) policy_revision: u32,
    pub(super) package_count: u32,
    pub(super) target_count: u32,
    pub(super) namespace_count: u32,
    pub(super) closure_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CargoTargetNamespaceError {
    Invalid,
    Changed,
    Unavailable,
}

impl CargoTargetNamespaceGuard {
    pub(super) fn capture(
        project_root: &CanonicalScanRoot,
        declarations: &[CargoTargetNamespaceDeclaration],
    ) -> Result<Self, CargoTargetNamespaceError> {
        let observation = capture_observation(project_root, declarations)?;
        let fence = platform::TargetNamespaceMutationFence::new(project_root, &observation)?;
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
        declarations: &[CargoTargetNamespaceDeclaration],
    ) -> Result<Self, CargoTargetNamespaceError> {
        let observation = capture_observation(project_root, declarations)?;
        Ok(Self {
            project_root: project_root.clone(),
            observation,
            fence: platform::TargetNamespaceMutationFence::disabled_for_test(),
        })
    }

    pub(super) fn poll(&self) -> Result<(), CargoTargetNamespaceError> {
        self.fence.poll()
    }

    pub(super) fn revalidate(&self) -> Result<(), CargoTargetNamespaceError> {
        self.poll()?;
        let current = capture_observation(&self.project_root, &self.observation.declarations)
            .map_err(|_| CargoTargetNamespaceError::Changed)?;
        if current != self.observation {
            return Err(CargoTargetNamespaceError::Changed);
        }
        self.poll()
    }

    pub(super) fn evidence(
        &self,
    ) -> Result<CargoTargetNamespaceEvidence, CargoTargetNamespaceError> {
        self.revalidate()?;
        Ok(CargoTargetNamespaceEvidence {
            policy_revision: TARGET_NAMESPACE_POLICY_REVISION,
            package_count: u32::try_from(self.observation.declarations.len())
                .map_err(|_| CargoTargetNamespaceError::Unavailable)?,
            target_count: u32::try_from(self.observation.target_count)
                .map_err(|_| CargoTargetNamespaceError::Unavailable)?,
            namespace_count: u32::try_from(self.observation.records.len())
                .map_err(|_| CargoTargetNamespaceError::Unavailable)?,
            closure_sha256: self.observation.closure_sha256,
        })
    }
}

fn capture_observation(
    project_root: &CanonicalScanRoot,
    declarations: &[CargoTargetNamespaceDeclaration],
) -> Result<TargetNamespaceObservation, CargoTargetNamespaceError> {
    if declarations.is_empty() || declarations.len() > MAX_PACKAGES {
        return Err(CargoTargetNamespaceError::Invalid);
    }
    let current_root = capture_exact_directory(project_root.canonical_path())?;
    if &current_root != project_root {
        return Err(CargoTargetNamespaceError::Changed);
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
    let mut manifests = BTreeSet::new();
    let mut target_count = 0_usize;
    let mut text_bytes = 0_usize;
    let mut path_bytes = 0_usize;
    let mut records = Vec::new();

    for package in &mut declarations {
        charge_text(&mut text_bytes, &package.package_id)?;
        if package.package_id.is_empty()
            || package.package_id.chars().any(char::is_control)
            || !package_ids.insert(package.package_id.as_bytes().to_vec())
            || package.targets.is_empty()
        {
            return Err(CargoTargetNamespaceError::Invalid);
        }
        let manifest = &package.manifest_path;
        if !is_normalized_absolute(manifest)
            || !manifest.starts_with(project_root.canonical_path())
            || manifest.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml")
            || !manifests.insert(manifest.as_os_str().as_bytes().to_vec())
        {
            return Err(CargoTargetNamespaceError::Invalid);
        }
        charge_path(&mut path_bytes, manifest)?;
        let manifest_snapshot = capture_present_entry(project_root, manifest)?;
        if !matches!(
            manifest_snapshot.state,
            NamespaceState::Present {
                kind: FilesystemEntryKind::RegularFile,
                hard_link_count: 1,
                ..
            }
        ) {
            return Err(CargoTargetNamespaceError::Invalid);
        }
        let package_root_path = manifest
            .parent()
            .ok_or(CargoTargetNamespaceError::Invalid)?;
        let package_root = capture_exact_directory(package_root_path)?;
        if package_root_path != project_root.canonical_path()
            && !package_root_path.starts_with(project_root.canonical_path())
        {
            return Err(CargoTargetNamespaceError::Invalid);
        }

        target_count = target_count
            .checked_add(package.targets.len())
            .ok_or(CargoTargetNamespaceError::Unavailable)?;
        if target_count > MAX_TARGETS {
            return Err(CargoTargetNamespaceError::Invalid);
        }
        for target in &package.targets {
            if target.name.is_empty()
                || target.name.chars().any(char::is_control)
                || target.name.as_bytes().contains(&b'/')
                || target.name.as_bytes().contains(&b'\\')
                || target.kinds.is_empty()
                || target.kinds.len() > MAX_KINDS_PER_TARGET
            {
                return Err(CargoTargetNamespaceError::Invalid);
            }
            charge_text(&mut text_bytes, &target.name)?;
            for kind in &target.kinds {
                if kind.is_empty() || kind.chars().any(char::is_control) {
                    return Err(CargoTargetNamespaceError::Invalid);
                }
                charge_text(&mut text_bytes, kind)?;
            }
            if !is_normalized_absolute(&target.src_path)
                || !target.src_path.starts_with(project_root.canonical_path())
            {
                return Err(CargoTargetNamespaceError::Invalid);
            }
            charge_path(&mut path_bytes, &target.src_path)?;
        }
        package.targets.sort_by(|left, right| {
            left.name
                .as_bytes()
                .cmp(right.name.as_bytes())
                .then_with(|| left.kinds.cmp(&right.kinds))
                .then_with(|| {
                    left.src_path
                        .as_os_str()
                        .as_bytes()
                        .cmp(right.src_path.as_os_str().as_bytes())
                })
        });
        for target in &package.targets {
            // The source path is retained both in the declaration and in the
            // exact present-entry record; charge the second owned occurrence.
            charge_path(&mut path_bytes, &target.src_path)?;
            let source = capture_present_entry(project_root, &target.src_path)?;
            if !matches!(
                source.state,
                NamespaceState::Present {
                    kind: FilesystemEntryKind::RegularFile,
                    hard_link_count: 1,
                    ..
                }
            ) {
                return Err(CargoTargetNamespaceError::Invalid);
            }
            push_record(&mut records, source)?;

            // Cargo 1.96 retains edition-2015 fallback probes for library and
            // binary targets at `src/<target-name>.rs`. Observing them for all
            // editions is conservative and keeps this closure independent of
            // manifest edition inheritance.
            let legacy_source = package_root_path
                .join("src")
                .join(format!("{}.rs", target.name));
            charge_path(&mut path_bytes, &legacy_source)?;
            push_record(
                &mut records,
                capture_optional_entry(project_root, &legacy_source)?,
            )?;
        }

        charge_path(&mut path_bytes, package_root.canonical_path())?;
        push_package_root_record(&mut records, &package_root)?;
        let src = package_root_path.join("src");
        let src_record = capture_optional_entry(project_root, &src)?;
        if matches!(
            src_record.state,
            NamespaceState::Present {
                kind: FilesystemEntryKind::RegularFile,
                ..
            }
        ) {
            return Err(CargoTargetNamespaceError::Invalid);
        }
        charge_path(&mut path_bytes, &src)?;
        push_record(&mut records, src_record)?;

        for relative in ["src/lib.rs", "src/main.rs", "src/bench.rs", "build.rs"] {
            let path = package_root_path.join(relative);
            charge_path(&mut path_bytes, &path)?;
            push_record(&mut records, capture_optional_entry(project_root, &path)?)?;
        }
        for relative in DISCOVERY_DIRECTORIES {
            capture_discovery_directory(
                project_root,
                &package_root_path.join(relative),
                &mut records,
                &mut path_bytes,
            )?;
        }
    }

    records.sort_by(|left, right| {
        left.path
            .as_os_str()
            .as_bytes()
            .cmp(right.path.as_os_str().as_bytes())
            .then_with(|| state_tag(left.state).cmp(&state_tag(right.state)))
    });
    if records.len() > MAX_NAMESPACE_RECORDS {
        return Err(CargoTargetNamespaceError::Unavailable);
    }
    let closure_sha256 = digest_observation(&declarations, &records, target_count);
    Ok(TargetNamespaceObservation {
        declarations,
        records,
        target_count,
        closure_sha256,
    })
}

fn capture_discovery_directory(
    project_root: &CanonicalScanRoot,
    path: &Path,
    records: &mut Vec<NamespaceRecord>,
    path_bytes: &mut usize,
) -> Result<(), CargoTargetNamespaceError> {
    charge_path(path_bytes, path)?;
    let directory_record = capture_optional_entry(project_root, path)?;
    match directory_record.state {
        NamespaceState::Missing => {
            push_record(records, directory_record)?;
            return Ok(());
        }
        NamespaceState::Present {
            kind: FilesystemEntryKind::Directory,
            ..
        } => push_record(records, directory_record)?,
        NamespaceState::Present { .. } => return Err(CargoTargetNamespaceError::Invalid),
    }

    let mut names = Vec::new();
    for entry in fs::read_dir(path).map_err(|_| CargoTargetNamespaceError::Unavailable)? {
        if records.len().saturating_add(names.len()) >= MAX_NAMESPACE_RECORDS {
            return Err(CargoTargetNamespaceError::Unavailable);
        }
        names.push(
            entry
                .map_err(|_| CargoTargetNamespaceError::Unavailable)?
                .file_name(),
        );
    }
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for name in names {
        if name.as_bytes().is_empty() || name.as_bytes().contains(&0) {
            return Err(CargoTargetNamespaceError::Invalid);
        }
        let entry_path = path.join(&name);
        charge_path(path_bytes, &entry_path)?;
        let entry = capture_present_entry(project_root, &entry_path)?;
        let child_directory = matches!(
            entry.state,
            NamespaceState::Present {
                kind: FilesystemEntryKind::Directory,
                ..
            }
        );
        push_record(records, entry)?;
        if child_directory {
            let main = entry_path.join("main.rs");
            charge_path(path_bytes, &main)?;
            push_record(records, capture_optional_entry(project_root, &main)?)?;
        }
    }
    Ok(())
}

fn capture_optional_entry(
    project_root: &CanonicalScanRoot,
    path: &Path,
) -> Result<NamespaceRecord, CargoTargetNamespaceError> {
    if !is_normalized_absolute(path) || !path.starts_with(project_root.canonical_path()) {
        return Err(CargoTargetNamespaceError::Invalid);
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(CargoTargetNamespaceError::Invalid)
        }
        Ok(_) => capture_present_entry(project_root, path),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(NamespaceRecord {
            path: path.to_path_buf(),
            state: NamespaceState::Missing,
        }),
        Err(_) => Err(CargoTargetNamespaceError::Unavailable),
    }
}

fn capture_present_entry(
    project_root: &CanonicalScanRoot,
    path: &Path,
) -> Result<NamespaceRecord, CargoTargetNamespaceError> {
    if path == project_root.canonical_path() {
        return Ok(NamespaceRecord {
            path: path.to_path_buf(),
            state: NamespaceState::Present {
                kind: FilesystemEntryKind::Directory,
                identity: project_root.identity(),
                hard_link_count: 1,
            },
        });
    }
    let lexical_root = validate_scan_root(project_root.canonical_path())
        .map_err(|_| CargoTargetNamespaceError::Invalid)?;
    let lexical = validate_cleanup_path(&lexical_root, path)
        .map_err(|_| CargoTargetNamespaceError::Invalid)?;
    let snapshot = capture_path_snapshot(project_root, lexical)
        .map_err(|_| CargoTargetNamespaceError::Unavailable)?;
    if !matches!(
        snapshot.target_kind(),
        FilesystemEntryKind::RegularFile | FilesystemEntryKind::Directory
    ) || (snapshot.target_kind() == FilesystemEntryKind::RegularFile
        && snapshot.hard_link_count() != 1)
    {
        return Err(CargoTargetNamespaceError::Invalid);
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

fn push_package_root_record(
    records: &mut Vec<NamespaceRecord>,
    package_root: &CanonicalScanRoot,
) -> Result<(), CargoTargetNamespaceError> {
    push_record(
        records,
        NamespaceRecord {
            path: package_root.canonical_path().to_path_buf(),
            state: NamespaceState::Present {
                kind: FilesystemEntryKind::Directory,
                identity: package_root.identity(),
                hard_link_count: 1,
            },
        },
    )
}

fn push_record(
    records: &mut Vec<NamespaceRecord>,
    record: NamespaceRecord,
) -> Result<(), CargoTargetNamespaceError> {
    records.push(record);
    if records.len() > MAX_NAMESPACE_RECORDS {
        Err(CargoTargetNamespaceError::Unavailable)
    } else {
        Ok(())
    }
}

fn capture_exact_directory(path: &Path) -> Result<CanonicalScanRoot, CargoTargetNamespaceError> {
    let lexical = validate_scan_root(path).map_err(|_| CargoTargetNamespaceError::Invalid)?;
    capture_scan_root(lexical).map_err(|_| CargoTargetNamespaceError::Unavailable)
}

fn is_normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .collect::<PathBuf>()
            .as_os_str()
            .as_bytes()
            == path.as_os_str().as_bytes()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
}

fn charge_path(total: &mut usize, path: &Path) -> Result<(), CargoTargetNamespaceError> {
    *total = total
        .checked_add(path.as_os_str().as_bytes().len())
        .ok_or(CargoTargetNamespaceError::Unavailable)?;
    if *total > MAX_NATIVE_PATH_BYTES {
        Err(CargoTargetNamespaceError::Unavailable)
    } else {
        Ok(())
    }
}

fn charge_text(total: &mut usize, text: &str) -> Result<(), CargoTargetNamespaceError> {
    *total = total
        .checked_add(text.len())
        .ok_or(CargoTargetNamespaceError::Unavailable)?;
    if *total > MAX_TEXT_BYTES {
        Err(CargoTargetNamespaceError::Unavailable)
    } else {
        Ok(())
    }
}

fn state_tag(state: NamespaceState) -> u8 {
    match state {
        NamespaceState::Missing => 0,
        NamespaceState::Present {
            kind: FilesystemEntryKind::RegularFile,
            ..
        } => 1,
        NamespaceState::Present {
            kind: FilesystemEntryKind::Directory,
            ..
        } => 2,
    }
}

fn digest_observation(
    declarations: &[CargoTargetNamespaceDeclaration],
    records: &[NamespaceRecord],
    target_count: usize,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"dux-cargo-target-namespace-v1\0");
    digest.update((declarations.len() as u64).to_le_bytes());
    digest.update((target_count as u64).to_le_bytes());
    for package in declarations {
        digest_bytes(&mut digest, package.package_id.as_bytes());
        digest_bytes(&mut digest, package.manifest_path.as_os_str().as_bytes());
        digest.update((package.targets.len() as u64).to_le_bytes());
        for target in &package.targets {
            digest_bytes(&mut digest, target.name.as_bytes());
            digest.update((target.kinds.len() as u64).to_le_bytes());
            for kind in &target.kinds {
                digest_bytes(&mut digest, kind.as_bytes());
            }
            digest_bytes(&mut digest, target.src_path.as_os_str().as_bytes());
        }
    }
    digest.update((records.len() as u64).to_le_bytes());
    for record in records {
        digest_bytes(&mut digest, record.path.as_os_str().as_bytes());
        digest.update([state_tag(record.state)]);
        if let NamespaceState::Present {
            identity,
            hard_link_count,
            ..
        } = record.state
        {
            digest.update(identity.volume().to_le_bytes());
            digest.update(identity.object().to_le_bytes());
            digest.update(hard_link_count.to_le_bytes());
        }
    }
    digest.finalize().into()
}

fn digest_bytes(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
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
        CanonicalScanRoot, CargoTargetNamespaceError, FilesystemEntryKind, FilesystemIdentity,
        MAX_WATCHED_OBJECTS, NamespaceState, TargetNamespaceObservation, capture_exact_directory,
    };

    pub(super) struct TargetNamespaceMutationFence {
        queue: Kqueue,
        _objects: Vec<OwnedFd>,
    }

    impl TargetNamespaceMutationFence {
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
            observation: &TargetNamespaceObservation,
        ) -> Result<Self, CargoTargetNamespaceError> {
            let expected = watched_objects(project_root, observation)?;
            require_descriptor_budget(expected.len().saturating_add(1))?;
            let queue = Kqueue::new().map_err(|_| CargoTargetNamespaceError::Unavailable)?;
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .map_err(|_| CargoTargetNamespaceError::Unavailable)?;
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
                .map_err(|_| CargoTargetNamespaceError::Unavailable)?;
            if count != 0 {
                return Err(CargoTargetNamespaceError::Changed);
            }
            Ok(Self {
                queue,
                _objects: objects,
            })
        }

        pub(super) fn poll(&self) -> Result<(), CargoTargetNamespaceError> {
            let mut event = [empty_event()];
            let count = self
                .queue
                .kevent(&[], &mut event, Some(zero_timeout()))
                .map_err(|_| CargoTargetNamespaceError::Unavailable)?;
            if count == 0 {
                Ok(())
            } else {
                Err(CargoTargetNamespaceError::Changed)
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
        observation: &TargetNamespaceObservation,
    ) -> Result<Vec<WatchedObject>, CargoTargetNamespaceError> {
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
                .ok_or(CargoTargetNamespaceError::Invalid)?;
            let relative = parent
                .strip_prefix(project_root.canonical_path())
                .map_err(|_| CargoTargetNamespaceError::Invalid)?;
            let mut current = project_root.canonical_path().to_path_buf();
            for component in relative.components() {
                current.push(component.as_os_str());
                insert_directory(&mut objects, capture_exact_directory(&current)?)?;
            }
        }
        if objects.len() > MAX_WATCHED_OBJECTS {
            return Err(CargoTargetNamespaceError::Unavailable);
        }
        Ok(objects.into_values().collect())
    }

    fn insert_directory(
        objects: &mut BTreeMap<(u64, u128), WatchedObject>,
        directory: CanonicalScanRoot,
    ) -> Result<(), CargoTargetNamespaceError> {
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
            Err(CargoTargetNamespaceError::Unavailable)
        } else {
            Ok(())
        }
    }

    fn open_watch_object(expected: &WatchedObject) -> Result<OwnedFd, CargoTargetNamespaceError> {
        let object = open(
            &expected.path,
            OFlag::from_bits_retain(libc::O_EVTONLY) | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| CargoTargetNamespaceError::Unavailable)?;
        require_reviewed_filesystem(&object)?;
        let status = fstat(&object).map_err(|_| CargoTargetNamespaceError::Unavailable)?;
        let kind_matches = if expected.kind == FilesystemEntryKind::Directory {
            (status.st_mode & libc::S_IFMT) == libc::S_IFDIR
        } else {
            (status.st_mode & libc::S_IFMT) == libc::S_IFREG
        };
        if !kind_matches
            || !identity_matches(&status, expected.identity)
            || (expected.kind == FilesystemEntryKind::RegularFile
                && status.st_nlink as u64 != expected.hard_link_count)
        {
            return Err(CargoTargetNamespaceError::Changed);
        }
        Ok(object)
    }

    fn require_reviewed_filesystem(
        descriptor: &impl AsFd,
    ) -> Result<(), CargoTargetNamespaceError> {
        let status = fstatfs(descriptor).map_err(|_| CargoTargetNamespaceError::Unavailable)?;
        if status.filesystem_type_name().eq_ignore_ascii_case("apfs")
            && status.flags().contains(MntFlags::MNT_LOCAL)
        {
            Ok(())
        } else {
            Err(CargoTargetNamespaceError::Unavailable)
        }
    }

    fn require_descriptor_budget(retained: usize) -> Result<(), CargoTargetNamespaceError> {
        const RESERVE: u64 = 128;
        let mut limit = MaybeUninit::<libc::rlimit>::zeroed();
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) } == -1 {
            return Err(CargoTargetNamespaceError::Unavailable);
        }
        let soft_limit = unsafe { limit.assume_init() }.rlim_cur;
        if soft_limit == libc::RLIM_INFINITY {
            return Ok(());
        }
        let open_descriptors = fs::read_dir("/dev/fd")
            .map_err(|_| CargoTargetNamespaceError::Unavailable)?
            .try_fold(0_u64, |count, entry| {
                entry
                    .map(|_| count.saturating_add(1))
                    .map_err(|_| CargoTargetNamespaceError::Unavailable)
            })?;
        let retained =
            u64::try_from(retained).map_err(|_| CargoTargetNamespaceError::Unavailable)?;
        if open_descriptors
            .checked_add(retained)
            .and_then(|count| count.checked_add(RESERVE))
            .is_some_and(|required| required <= soft_limit)
        {
            Ok(())
        } else {
            Err(CargoTargetNamespaceError::Unavailable)
        }
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
    use super::{CanonicalScanRoot, CargoTargetNamespaceError, TargetNamespaceObservation};

    pub(super) struct TargetNamespaceMutationFence;

    impl TargetNamespaceMutationFence {
        #[cfg(test)]
        pub(super) fn disabled_for_test() -> Self {
            Self
        }

        pub(super) fn new(
            _project_root: &CanonicalScanRoot,
            _observation: &TargetNamespaceObservation,
        ) -> Result<Self, CargoTargetNamespaceError> {
            Ok(Self)
        }

        pub(super) fn poll(&self) -> Result<(), CargoTargetNamespaceError> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn fixture() -> (
        TempDir,
        CanonicalScanRoot,
        Vec<CargoTargetNamespaceDeclaration>,
    ) {
        let temp = TempDir::new().unwrap();
        let root_path = temp.path().join("workspace");
        fs::create_dir_all(root_path.join("src/bin/tool")).unwrap();
        fs::create_dir(root_path.join("examples")).unwrap();
        let root_path = fs::canonicalize(root_path).unwrap();
        fs::write(root_path.join("Cargo.toml"), "[package]\nname='fixture'\n").unwrap();
        fs::write(root_path.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
        fs::write(root_path.join("src/bin/tool/main.rs"), "fn main() {}\n").unwrap();
        let root = capture_exact_directory(&root_path).unwrap();
        let declarations = vec![CargoTargetNamespaceDeclaration {
            package_id: "fixture 0.1.0".to_owned(),
            manifest_path: root_path.join("Cargo.toml"),
            targets: vec![
                CargoTargetDeclaration {
                    name: "fixture".to_owned(),
                    kinds: vec!["lib".to_owned()],
                    src_path: root_path.join("src/lib.rs"),
                },
                CargoTargetDeclaration {
                    name: "tool".to_owned(),
                    kinds: vec!["bin".to_owned()],
                    src_path: root_path.join("src/bin/tool/main.rs"),
                },
            ],
        }];
        (temp, root, declarations)
    }

    #[test]
    fn finite_target_namespace_is_digest_bound_and_revalidated() {
        let (_temp, root, declarations) = fixture();
        let guard =
            CargoTargetNamespaceGuard::capture_unfenced_for_test(&root, &declarations).unwrap();
        let evidence = guard.evidence().unwrap();
        assert_eq!(evidence.policy_revision, 1);
        assert_eq!(evidence.package_count, 1);
        assert_eq!(evidence.target_count, 2);
        assert!(evidence.namespace_count >= 10);
        assert_ne!(evidence.closure_sha256, [0; 32]);
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test removes only TempDir-owned entries to exercise namespace replacement"
    )]
    fn target_create_remove_and_source_replacement_change_observation() {
        let (_temp, root, declarations) = fixture();
        let guard =
            CargoTargetNamespaceGuard::capture_unfenced_for_test(&root, &declarations).unwrap();
        let ghost = root.canonical_path().join("src/bin/ghost.rs");
        fs::write(&ghost, "fn main() {}\n").unwrap();
        assert_eq!(guard.revalidate(), Err(CargoTargetNamespaceError::Changed));
        // DUX-DESTRUCTIVE: allow=test-target-namespace-ghost-remove -- TempDir-owned ghost target is removed to test namespace restoration
        fs::remove_file(ghost).unwrap();

        let guard =
            CargoTargetNamespaceGuard::capture_unfenced_for_test(&root, &declarations).unwrap();
        let legacy = root.canonical_path().join("src/fixture.rs");
        fs::write(&legacy, "pub fn legacy() {}\n").unwrap();
        assert_eq!(guard.revalidate(), Err(CargoTargetNamespaceError::Changed));
        // DUX-DESTRUCTIVE: allow=test-target-namespace-legacy-remove -- TempDir-owned legacy target is removed before the next observation
        fs::remove_file(legacy).unwrap();

        let guard =
            CargoTargetNamespaceGuard::capture_unfenced_for_test(&root, &declarations).unwrap();
        let source = root.canonical_path().join("src/lib.rs");
        // DUX-DESTRUCTIVE: allow=test-target-namespace-source-remove -- TempDir-owned source is removed to test replacement detection
        fs::remove_file(&source).unwrap();
        fs::write(source, "pub fn replacement() {}\n").unwrap();
        assert_eq!(guard.revalidate(), Err(CargoTargetNamespaceError::Changed));
    }

    #[test]
    fn external_missing_and_symlink_target_sources_fail_closed() {
        use std::os::unix::fs::symlink;

        let (temp, root, mut declarations) = fixture();
        declarations[0].targets[0].src_path = temp.path().join("external.rs");
        fs::write(
            &declarations[0].targets[0].src_path,
            "pub fn external() {}\n",
        )
        .unwrap();
        assert!(matches!(
            CargoTargetNamespaceGuard::capture_unfenced_for_test(&root, &declarations),
            Err(CargoTargetNamespaceError::Invalid)
        ));

        declarations[0].targets[0].src_path = root.canonical_path().join("src/missing.rs");
        assert!(
            CargoTargetNamespaceGuard::capture_unfenced_for_test(&root, &declarations).is_err()
        );

        let real = root.canonical_path().join("src/lib.rs");
        let alias = root.canonical_path().join("src/alias.rs");
        symlink(real, &alias).unwrap();
        declarations[0].targets[0].src_path = alias;
        assert!(
            CargoTargetNamespaceGuard::capture_unfenced_for_test(&root, &declarations).is_err()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test removes only a TempDir-owned entry to exercise vnode write/restore"
    )]
    fn directory_write_and_restore_is_terminal_for_armed_guard() {
        let (_temp, root, declarations) = fixture();
        let guard = CargoTargetNamespaceGuard::capture(&root, &declarations).unwrap();
        let ghost = root.canonical_path().join("src/bin/ghost.rs");
        fs::write(&ghost, "fn main() {}\n").unwrap();
        // DUX-DESTRUCTIVE: allow=test-target-namespace-transient-remove -- TempDir-owned transient target is removed to test vnode persistence
        fs::remove_file(ghost).unwrap();
        assert_eq!(guard.poll(), Err(CargoTargetNamespaceError::Changed));
    }
}
