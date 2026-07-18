//! Bounded Cargo 1.96 configuration provenance for one retained tree.
//!
//! The pinned executable emits one fixed tracing record immediately before
//! every configuration-file read. DUX independently captures the same bounded
//! discovery/include closure, fences its exact bytes, and requires the ordered
//! intent records to agree. This remains path-intent inference, not kernel
//! proof of which inode Cargo opened.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::cargo_config_closure::{CargoConfigurationFileClosure, ObservedCargoConfigurationFile};
use crate::path_validation::{
    CanonicalScanRoot, FilesystemIdentity, capture_scan_root, validate_scan_root,
};

const CONFIG_POLICY_REVISION: u32 = 3;
const MAX_CWD_ANCESTORS: usize = 64;
const MAX_WATCHED_DIRECTORIES: usize = 512;
const MAX_NATIVE_PATH_BYTES: usize = 128 * 1024;
const CONFIG_NAMES: [&str; 2] = ["config", "config.toml"];

#[derive(Clone, Debug, PartialEq, Eq)]
struct LookupDirectory {
    parent: CanonicalScanRoot,
    cargo_directory: Option<CanonicalScanRoot>,
    selected_config: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConfigObservation {
    lookups: Vec<LookupDirectory>,
    separate_cargo_home: Option<CanonicalScanRoot>,
    file_closure: CargoConfigurationFileClosure,
    closure_sha256: [u8; 32],
    watched_directories: Vec<WatchedDirectory>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WatchedDirectory {
    directory: CanonicalScanRoot,
    entry_changes_terminal: bool,
}

/// Sealed evidence for bounded positive configuration path intent and exact
/// file bytes. It is deliberately non-cloneable and cannot prove Cargo's exact
/// kernel read set.
pub(super) struct CargoConfigurationGuard {
    project_root: CanonicalScanRoot,
    cargo_home: CanonicalScanRoot,
    observation: ConfigObservation,
    fence: platform::DirectoryMutationFence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CargoConfigurationEvidence {
    pub(super) policy_revision: u32,
    pub(super) lookup_count: u32,
    pub(super) root_config_count: u32,
    pub(super) config_file_count: u32,
    pub(super) include_edge_count: u32,
    pub(super) config_byte_count: u64,
    pub(super) closure_sha256: [u8; 32],
    pub(super) read_intent_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CargoConfigurationError {
    Unsupported,
    Changed,
    Unavailable,
}

impl CargoConfigurationGuard {
    pub(super) fn capture(
        project_root: &CanonicalScanRoot,
        cargo_home: &CanonicalScanRoot,
    ) -> Result<Self, CargoConfigurationError> {
        let observation = capture_observation(project_root, cargo_home)?;
        let fence = platform::DirectoryMutationFence::new(
            &observation.watched_directories,
            observation.file_closure.files(),
        )?;
        let guard = Self {
            project_root: project_root.clone(),
            cargo_home: cargo_home.clone(),
            observation,
            fence,
        };
        // Close the gap between the filesystem observation and arming the
        // vnode fence. Any persistent change is caught here; any change after
        // fence registration is terminal even if later restored.
        guard.revalidate()?;
        Ok(guard)
    }

    #[cfg(test)]
    pub(super) fn capture_unfenced_for_test(
        project_root: &CanonicalScanRoot,
        cargo_home: &CanonicalScanRoot,
    ) -> Result<Self, CargoConfigurationError> {
        let observation = capture_observation(project_root, cargo_home)?;
        Ok(Self {
            project_root: project_root.clone(),
            cargo_home: cargo_home.clone(),
            observation,
            fence: platform::DirectoryMutationFence::disabled_for_test(),
        })
    }

    pub(super) fn poll(&self) -> Result<(), CargoConfigurationError> {
        self.fence.poll()
    }

    pub(super) fn revalidate(&self) -> Result<(), CargoConfigurationError> {
        self.poll()?;
        let current = match capture_observation(&self.project_root, &self.cargo_home) {
            Ok(current) => current,
            Err(CargoConfigurationError::Unsupported | CargoConfigurationError::Changed) => {
                return Err(CargoConfigurationError::Changed);
            }
            Err(CargoConfigurationError::Unavailable) => {
                return Err(CargoConfigurationError::Unavailable);
            }
        };
        if current != self.observation {
            return Err(CargoConfigurationError::Changed);
        }
        self.poll()
    }

    pub(super) fn evidence(&self) -> Result<CargoConfigurationEvidence, CargoConfigurationError> {
        self.revalidate()?;
        let lookup_count = u32::try_from(
            self.observation.lookups.len()
                + usize::from(self.observation.separate_cargo_home.is_some()),
        )
        .map_err(|_| CargoConfigurationError::Unavailable)?;
        Ok(CargoConfigurationEvidence {
            policy_revision: CONFIG_POLICY_REVISION,
            lookup_count,
            root_config_count: u32::try_from(self.observation.file_closure.root_count())
                .map_err(|_| CargoConfigurationError::Unavailable)?,
            config_file_count: u32::try_from(self.observation.file_closure.file_count())
                .map_err(|_| CargoConfigurationError::Unavailable)?,
            include_edge_count: u32::try_from(self.observation.file_closure.include_edge_count())
                .map_err(|_| CargoConfigurationError::Unavailable)?,
            config_byte_count: u64::try_from(self.observation.file_closure.total_bytes())
                .map_err(|_| CargoConfigurationError::Unavailable)?,
            closure_sha256: self.observation.closure_sha256,
            read_intent_sha256: self.observation.file_closure.read_intent_sha256(),
        })
    }

    pub(super) fn verify_read_intent(&self, stderr: &[u8]) -> Result<(), CargoConfigurationError> {
        self.poll()?;
        self.observation.file_closure.verify_read_intent(stderr)?;
        self.poll()
    }
}

fn capture_observation(
    project_root: &CanonicalScanRoot,
    cargo_home: &CanonicalScanRoot,
) -> Result<ConfigObservation, CargoConfigurationError> {
    let current_project = capture_exact_directory(project_root.canonical_path())?;
    let current_cargo_home = capture_exact_directory(cargo_home.canonical_path())?;
    if &current_project != project_root || &current_cargo_home != cargo_home {
        return Err(CargoConfigurationError::Changed);
    }
    if current_project.identity() == current_cargo_home.identity() {
        return Err(CargoConfigurationError::Unavailable);
    }
    platform::review_directory(&current_project)?;
    platform::review_directory(&current_cargo_home)?;

    let mut lookups = Vec::new();
    let mut root_configs = Vec::new();
    let mut native_path_bytes = 0_usize;
    for ancestor_path in project_root.canonical_path().ancestors() {
        if lookups.len() == MAX_CWD_ANCESTORS {
            return Err(CargoConfigurationError::Unavailable);
        }
        charge_path(&mut native_path_bytes, ancestor_path)?;
        let parent = capture_exact_directory(ancestor_path)?;
        platform::review_directory(&parent)?;
        let cargo_path = ancestor_path.join(".cargo");
        charge_path(&mut native_path_bytes, &cargo_path)?;
        let cargo_directory = capture_optional_exact_directory(&cargo_path)?;
        let selected_config = if let Some(directory) = cargo_directory.as_ref() {
            platform::review_directory(directory)?;
            let selected = select_config_file(directory.canonical_path(), &mut native_path_bytes)?;
            if selected.is_some() && directory.identity() == cargo_home.identity() {
                return Err(CargoConfigurationError::Unsupported);
            }
            selected
        } else {
            None
        };
        if let Some(path) = selected_config.as_ref() {
            root_configs.push(path.clone());
        }
        lookups.push(LookupDirectory {
            parent,
            cargo_directory,
            selected_config,
        });
    }

    let home_already_visited = lookups.iter().any(|lookup| {
        lookup
            .cargo_directory
            .as_ref()
            .is_some_and(|directory| directory.identity() == cargo_home.identity())
    });
    let separate_cargo_home = if home_already_visited {
        None
    } else {
        charge_path(&mut native_path_bytes, cargo_home.canonical_path())?;
        if select_config_file(cargo_home.canonical_path(), &mut native_path_bytes)?.is_some() {
            return Err(CargoConfigurationError::Unsupported);
        }
        Some(cargo_home.clone())
    };

    let file_closure = if root_configs.is_empty() {
        CargoConfigurationFileClosure::empty()
    } else {
        CargoConfigurationFileClosure::capture(&root_configs)?
    };
    let watched_directories =
        watched_directories(&lookups, &current_cargo_home, file_closure.files())?;
    let closure_sha256 = digest_observation(
        &lookups,
        separate_cargo_home.as_ref(),
        &file_closure,
        &watched_directories,
    );
    Ok(ConfigObservation {
        lookups,
        separate_cargo_home,
        file_closure,
        closure_sha256,
        watched_directories,
    })
}

fn capture_optional_exact_directory(
    path: &Path,
) -> Result<Option<CanonicalScanRoot>, CargoConfigurationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(CargoConfigurationError::Unavailable);
            }
            let canonical =
                fs::canonicalize(path).map_err(|_| CargoConfigurationError::Unavailable)?;
            if canonical != path {
                return Err(CargoConfigurationError::Unavailable);
            }
            capture_exact_directory(path).map(Some)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(CargoConfigurationError::Unavailable),
    }
}

fn capture_exact_directory(path: &Path) -> Result<CanonicalScanRoot, CargoConfigurationError> {
    let lexical = validate_scan_root(path).map_err(|_| CargoConfigurationError::Unavailable)?;
    capture_scan_root(lexical).map_err(|_| CargoConfigurationError::Unavailable)
}

fn select_config_file(
    directory: &Path,
    native_path_bytes: &mut usize,
) -> Result<Option<PathBuf>, CargoConfigurationError> {
    let mut present = Vec::new();
    for name in CONFIG_NAMES {
        let config = directory.join(name);
        charge_path(native_path_bytes, &config)?;
        match fs::symlink_metadata(&config) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(CargoConfigurationError::Unsupported);
                }
                present.push(config);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(CargoConfigurationError::Unavailable),
        }
    }
    match present.as_slice() {
        [] => Ok(None),
        [path] => Ok(Some(path.clone())),
        _ => Err(CargoConfigurationError::Unsupported),
    }
}

fn charge_path(total: &mut usize, path: &Path) -> Result<(), CargoConfigurationError> {
    *total = total
        .checked_add(path.as_os_str().as_bytes().len())
        .ok_or(CargoConfigurationError::Unavailable)?;
    if *total > MAX_NATIVE_PATH_BYTES {
        return Err(CargoConfigurationError::Unavailable);
    }
    Ok(())
}

fn watched_directories(
    lookups: &[LookupDirectory],
    cargo_home: &CanonicalScanRoot,
    config_files: &[ObservedCargoConfigurationFile],
) -> Result<Vec<WatchedDirectory>, CargoConfigurationError> {
    let mut by_identity = BTreeMap::<(u64, u128), WatchedDirectory>::new();
    let project_root = lookups
        .first()
        .ok_or(CargoConfigurationError::Unavailable)?
        .parent
        .clone();
    insert_watched_directory(
        &mut by_identity,
        WatchedDirectory {
            directory: project_root,
            entry_changes_terminal: true,
        },
    );
    for directory in lookups
        .iter()
        .filter_map(|lookup| lookup.cargo_directory.as_ref())
    {
        let identity = directory.identity();
        insert_watched_directory(
            &mut by_identity,
            WatchedDirectory {
                directory: directory.clone(),
                entry_changes_terminal: identity != cargo_home.identity(),
            },
        );
    }
    for file in config_files {
        for (index, ancestor_path) in file.parent().canonical_path().ancestors().enumerate() {
            let directory = if index == 0 {
                file.parent().clone()
            } else {
                capture_exact_directory(ancestor_path)?
            };
            platform::review_directory(&directory)?;
            insert_watched_directory(
                &mut by_identity,
                WatchedDirectory {
                    directory,
                    // Only the direct parent can replace this exact file.
                    // Higher ancestors need rename/delete continuity without
                    // treating unrelated sibling writes as terminal.
                    entry_changes_terminal: index == 0,
                },
            );
        }
    }
    insert_watched_directory(
        &mut by_identity,
        WatchedDirectory {
            directory: cargo_home.clone(),
            // Cargo legitimately updates locks and caches in its home during
            // metadata. A directly included file in Cargo home upgrades this
            // directory to strict entry-change handling above.
            entry_changes_terminal: false,
        },
    );
    if by_identity.len() > MAX_WATCHED_DIRECTORIES {
        return Err(CargoConfigurationError::Unavailable);
    }
    Ok(by_identity.into_values().collect())
}

fn insert_watched_directory(
    by_identity: &mut BTreeMap<(u64, u128), WatchedDirectory>,
    watched: WatchedDirectory,
) {
    let identity = watched.directory.identity();
    by_identity
        .entry((identity.volume(), identity.object()))
        .and_modify(|existing| {
            existing.entry_changes_terminal |= watched.entry_changes_terminal;
        })
        .or_insert(watched);
}

fn digest_observation(
    lookups: &[LookupDirectory],
    cargo_home: Option<&CanonicalScanRoot>,
    file_closure: &CargoConfigurationFileClosure,
    watched_directories: &[WatchedDirectory],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"dux-cargo-config-closure-v3\0");
    for lookup in lookups {
        digest_directory(&mut digest, 1, &lookup.parent);
        match lookup.cargo_directory.as_ref() {
            Some(directory) => digest_directory(&mut digest, 2, directory),
            None => digest.update([0]),
        }
        match lookup.selected_config.as_ref() {
            Some(path) => {
                digest.update([1]);
                let bytes = path.as_os_str().as_bytes();
                digest.update((bytes.len() as u64).to_le_bytes());
                digest.update(bytes);
            }
            None => digest.update([0]),
        }
    }
    match cargo_home {
        Some(directory) => digest_directory(&mut digest, 3, directory),
        None => digest.update([0xff]),
    }
    digest.update(file_closure.digest_sha256());
    digest.update(file_closure.read_intent_sha256());
    digest.update((watched_directories.len() as u64).to_le_bytes());
    for watched in watched_directories {
        digest_directory(&mut digest, 4, &watched.directory);
        digest.update([u8::from(watched.entry_changes_terminal)]);
    }
    digest.finalize().into()
}

fn digest_directory(digest: &mut Sha256, tag: u8, directory: &CanonicalScanRoot) {
    let path = directory.canonical_path().as_os_str().as_bytes();
    digest.update([tag]);
    digest.update((path.len() as u64).to_le_bytes());
    digest.update(path);
    digest_identity(digest, directory.identity());
}

fn digest_identity(digest: &mut Sha256, identity: FilesystemIdentity) {
    digest.update(identity.volume().to_le_bytes());
    digest.update(identity.object().to_le_bytes());
}

#[cfg(target_os = "macos")]
mod platform {
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
        CanonicalScanRoot, CargoConfigurationError, ObservedCargoConfigurationFile,
        WatchedDirectory,
    };

    pub(super) struct DirectoryMutationFence {
        queue: Kqueue,
        _directories: Vec<OwnedFd>,
        _files: Vec<OwnedFd>,
    }

    pub(super) fn review_directory(
        expected: &CanonicalScanRoot,
    ) -> Result<(), CargoConfigurationError> {
        drop(open_watch_directory(expected)?);
        Ok(())
    }

    impl DirectoryMutationFence {
        #[cfg(test)]
        pub(super) fn disabled_for_test() -> Self {
            let queue = Kqueue::new().expect("test kqueue must be available");
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .expect("test kqueue must support close-on-exec");
            Self {
                queue,
                _directories: Vec::new(),
                _files: Vec::new(),
            }
        }

        pub(super) fn new(
            expected_directories: &[WatchedDirectory],
            expected_files: &[ObservedCargoConfigurationFile],
        ) -> Result<Self, CargoConfigurationError> {
            require_descriptor_budget(
                expected_directories
                    .len()
                    .checked_add(expected_files.len())
                    .and_then(|count| count.checked_add(1))
                    .ok_or(CargoConfigurationError::Unavailable)?,
            )?;
            let queue = Kqueue::new().map_err(|_| CargoConfigurationError::Unavailable)?;
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .map_err(|_| CargoConfigurationError::Unavailable)?;
            let mut directories = Vec::with_capacity(expected_directories.len());
            for expected in expected_directories {
                directories.push(open_watch_directory(&expected.directory)?);
            }
            let mut files = Vec::with_capacity(expected_files.len());
            for expected in expected_files {
                files.push(open_watch_file(expected)?);
            }
            let mut changes: Vec<KEvent> = directories
                .iter()
                .zip(expected_directories)
                .map(|(directory, expected)| {
                    let mut flags =
                        FilterFlag::NOTE_DELETE | FilterFlag::NOTE_RENAME | FilterFlag::NOTE_REVOKE;
                    if expected.entry_changes_terminal {
                        flags |= FilterFlag::NOTE_WRITE;
                    }
                    KEvent::new(
                        directory.as_raw_fd() as usize,
                        EventFilter::EVFILT_VNODE,
                        EvFlags::EV_ADD | EvFlags::EV_ENABLE | EvFlags::EV_CLEAR,
                        flags,
                        0,
                        0,
                    )
                })
                .collect();
            changes.extend(files.iter().map(|file| {
                KEvent::new(
                    file.as_raw_fd() as usize,
                    EventFilter::EVFILT_VNODE,
                    EvFlags::EV_ADD | EvFlags::EV_ENABLE | EvFlags::EV_CLEAR,
                    FilterFlag::NOTE_DELETE
                        | FilterFlag::NOTE_WRITE
                        | FilterFlag::NOTE_EXTEND
                        | FilterFlag::NOTE_ATTRIB
                        | FilterFlag::NOTE_LINK
                        | FilterFlag::NOTE_RENAME
                        | FilterFlag::NOTE_REVOKE,
                    0,
                    0,
                )
            }));
            let mut events = vec![empty_event(); changes.len().max(1)];
            let count = queue
                .kevent(&changes, &mut events, Some(zero_timeout()))
                .map_err(|_| CargoConfigurationError::Unavailable)?;
            if events[..count]
                .iter()
                .any(|event| event.flags().contains(EvFlags::EV_ERROR))
            {
                return Err(CargoConfigurationError::Unavailable);
            }
            if count != 0 {
                return Err(CargoConfigurationError::Changed);
            }
            Ok(Self {
                queue,
                _directories: directories,
                _files: files,
            })
        }

        pub(super) fn poll(&self) -> Result<(), CargoConfigurationError> {
            let mut event = [empty_event()];
            let count = self
                .queue
                .kevent(&[], &mut event, Some(zero_timeout()))
                .map_err(|_| CargoConfigurationError::Unavailable)?;
            if count == 0 {
                Ok(())
            } else {
                Err(CargoConfigurationError::Changed)
            }
        }
    }

    fn open_watch_directory(
        expected: &CanonicalScanRoot,
    ) -> Result<OwnedFd, CargoConfigurationError> {
        let directory = open(
            expected.canonical_path(),
            OFlag::from_bits_retain(libc::O_EVTONLY) | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| CargoConfigurationError::Unavailable)?;
        require_reviewed_filesystem(&directory)?;
        let status = fstat(&directory).map_err(|_| CargoConfigurationError::Unavailable)?;
        if status.st_dev as u64 != expected.identity().volume()
            || u128::from(status.st_ino) != expected.identity().object()
        {
            return Err(CargoConfigurationError::Changed);
        }
        Ok(directory)
    }

    fn open_watch_file(
        expected: &ObservedCargoConfigurationFile,
    ) -> Result<OwnedFd, CargoConfigurationError> {
        let file = open(
            expected.path(),
            OFlag::from_bits_retain(libc::O_EVTONLY) | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| CargoConfigurationError::Unavailable)?;
        require_reviewed_filesystem(&file)?;
        let status = fstat(&file).map_err(|_| CargoConfigurationError::Unavailable)?;
        let identity = expected.file().path().target_identity();
        if status.st_dev as u64 != identity.volume()
            || u128::from(status.st_ino) != identity.object()
            || status.st_nlink != 1
        {
            return Err(CargoConfigurationError::Changed);
        }
        Ok(file)
    }

    fn require_descriptor_budget(retained: usize) -> Result<(), CargoConfigurationError> {
        const RESERVE_FOR_PROCESS_AND_LAUNCH: u64 = 128;

        let mut limit = MaybeUninit::<libc::rlimit>::zeroed();
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) } == -1 {
            return Err(CargoConfigurationError::Unavailable);
        }
        let soft_limit = unsafe { limit.assume_init() }.rlim_cur;
        if soft_limit == libc::RLIM_INFINITY {
            return Ok(());
        }
        let open_descriptors = fs::read_dir("/dev/fd")
            .map_err(|_| CargoConfigurationError::Unavailable)?
            .try_fold(0_u64, |count, entry| {
                entry
                    .map(|_| count.saturating_add(1))
                    .map_err(|_| CargoConfigurationError::Unavailable)
            })?;
        let retained = u64::try_from(retained).map_err(|_| CargoConfigurationError::Unavailable)?;
        if descriptor_budget_fits(
            open_descriptors,
            retained,
            RESERVE_FOR_PROCESS_AND_LAUNCH,
            soft_limit,
        ) {
            Ok(())
        } else {
            Err(CargoConfigurationError::Unavailable)
        }
    }

    fn descriptor_budget_fits(open: u64, retained: u64, reserve: u64, soft_limit: u64) -> bool {
        open.checked_add(retained)
            .and_then(|count| count.checked_add(reserve))
            .is_some_and(|required| required <= soft_limit)
    }

    fn require_reviewed_filesystem(descriptor: &impl AsFd) -> Result<(), CargoConfigurationError> {
        let status = fstatfs(descriptor).map_err(|_| CargoConfigurationError::Unavailable)?;
        if !status.filesystem_type_name().eq_ignore_ascii_case("apfs")
            || !status.flags().contains(MntFlags::MNT_LOCAL)
        {
            return Err(CargoConfigurationError::Unavailable);
        }
        Ok(())
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

    fn zero_timeout() -> nix::libc::timespec {
        nix::libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::descriptor_budget_fits;

        #[test]
        fn descriptor_budget_rejects_shortfall_and_overflow() {
            assert!(descriptor_budget_fits(10, 20, 128, 158));
            assert!(!descriptor_budget_fits(10, 20, 128, 157));
            assert!(!descriptor_budget_fits(u64::MAX, 1, 0, u64::MAX));
            assert!(!descriptor_budget_fits(u64::MAX - 1, 1, 1, u64::MAX));
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{
        CanonicalScanRoot, CargoConfigurationError, ObservedCargoConfigurationFile,
        WatchedDirectory,
    };

    pub(super) struct DirectoryMutationFence;

    pub(super) fn review_directory(
        _expected: &CanonicalScanRoot,
    ) -> Result<(), CargoConfigurationError> {
        Ok(())
    }

    impl DirectoryMutationFence {
        #[cfg(test)]
        pub(super) fn disabled_for_test() -> Self {
            Self
        }

        pub(super) fn new(
            _expected_directories: &[WatchedDirectory],
            _expected_files: &[ObservedCargoConfigurationFile],
        ) -> Result<Self, CargoConfigurationError> {
            Ok(Self)
        }

        pub(super) fn poll(&self) -> Result<(), CargoConfigurationError> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn roots(temp: &TempDir) -> (CanonicalScanRoot, CanonicalScanRoot) {
        let canonical_temp = fs::canonicalize(temp.path()).unwrap();
        let project = canonical_temp.join("project");
        let cargo_home = canonical_temp.join("cargo-home");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&cargo_home).unwrap();
        (
            capture_exact_directory(&project).unwrap(),
            capture_exact_directory(&cargo_home).unwrap(),
        )
    }

    #[test]
    fn empty_closure_is_stable_and_cargo_home_config_is_unsupported() {
        let temp = TempDir::new().unwrap();
        let (project, cargo_home) = roots(&temp);
        let guard =
            CargoConfigurationGuard::capture_unfenced_for_test(&project, &cargo_home).unwrap();
        let evidence = guard.evidence().unwrap();
        assert_eq!(evidence.policy_revision, CONFIG_POLICY_REVISION);
        assert!(evidence.lookup_count >= 2);
        assert_eq!(evidence.root_config_count, 0);
        assert_eq!(evidence.config_file_count, 0);
        assert_ne!(evidence.closure_sha256, [0; 32]);

        fs::write(cargo_home.canonical_path().join("config.toml"), "").unwrap();
        assert_eq!(guard.revalidate(), Err(CargoConfigurationError::Changed));
        assert_eq!(
            CargoConfigurationGuard::capture(&project, &cargo_home).err(),
            Some(CargoConfigurationError::Unsupported)
        );
    }

    #[test]
    fn extensionless_project_and_outer_configs_are_captured_but_dual_names_reject() {
        let temp = TempDir::new().unwrap();
        let (project, cargo_home) = roots(&temp);
        let cargo = project.canonical_path().join(".cargo");
        fs::create_dir(&cargo).unwrap();
        fs::write(cargo.join("config"), "[build]\ntarget-dir = \"target\"\n").unwrap();
        let guard =
            CargoConfigurationGuard::capture_unfenced_for_test(&project, &cargo_home).unwrap();
        let evidence = guard.evidence().unwrap();
        assert_eq!(evidence.root_config_count, 1);
        assert_eq!(evidence.config_file_count, 1);
        assert!(evidence.config_byte_count > 0);

        let outer_temp = TempDir::new().unwrap();
        let (outer_project, outer_cargo_home) = roots(&outer_temp);
        let outer = outer_temp.path().join(".cargo");
        fs::create_dir(&outer).unwrap();
        fs::write(outer.join("config.toml"), "").unwrap();
        let outer_guard =
            CargoConfigurationGuard::capture_unfenced_for_test(&outer_project, &outer_cargo_home)
                .unwrap();
        assert_eq!(outer_guard.evidence().unwrap().root_config_count, 1);

        fs::write(cargo.join("config.toml"), "").unwrap();
        assert_eq!(
            CargoConfigurationGuard::capture(&project, &cargo_home).err(),
            Some(CargoConfigurationError::Unsupported)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test removes only a TempDir-owned config to prove create-remove remains observable"
    )]
    fn create_then_remove_is_still_a_terminal_vnode_event() {
        let temp = TempDir::new().unwrap();
        let (_project, cargo_home) = roots(&temp);
        let watched = WatchedDirectory {
            directory: cargo_home.clone(),
            entry_changes_terminal: true,
        };
        let fence =
            platform::DirectoryMutationFence::new(std::slice::from_ref(&watched), &[]).unwrap();
        let config = cargo_home.canonical_path().join("config.toml");
        fs::write(&config, "").unwrap();
        // DUX-DESTRUCTIVE: allow=test-cargo-config-transient-remove -- remove only the just-created TempDir-owned config to prove the vnode event remains terminal after restoration
        fs::remove_file(config).unwrap();
        assert_eq!(fence.poll(), Err(CargoConfigurationError::Changed));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unrelated_ancestor_and_cargo_home_runtime_writes_do_not_false_positive() {
        let temp = TempDir::new().unwrap();
        let (project, cargo_home) = roots(&temp);
        let guard = CargoConfigurationGuard::capture(&project, &cargo_home).unwrap();

        fs::write(temp.path().join("unrelated-sibling"), "unrelated").unwrap();
        fs::write(cargo_home.canonical_path().join(".package-cache"), "lock").unwrap();
        guard.revalidate().unwrap();
    }
}
