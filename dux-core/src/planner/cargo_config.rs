//! Bounded negative Cargo-configuration observation for one retained tree.
//!
//! Cargo 1.96 does not report the configuration files it opened. Until DUX has
//! a directly attested positive-config transport, production accepts only the
//! state observed absent at every file-based lookup before and after metadata.
//! Project and existing non-Cargo-home lookup mutations are fenced; broader
//! transient create-remove remains an explicit non-authoritative limitation.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::path_validation::{
    CanonicalScanRoot, FilesystemIdentity, capture_scan_root, validate_scan_root,
};

const CONFIG_POLICY_REVISION: u32 = 2;
const MAX_CWD_ANCESTORS: usize = 64;
const MAX_WATCHED_DIRECTORIES: usize = 132;
const MAX_NATIVE_PATH_BYTES: usize = 64 * 1024;
const CONFIG_NAMES: [&str; 2] = ["config", "config.toml"];

#[derive(Clone, Debug, PartialEq, Eq)]
struct LookupDirectory {
    parent: CanonicalScanRoot,
    cargo_directory: Option<CanonicalScanRoot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConfigAbsenceObservation {
    lookups: Vec<LookupDirectory>,
    separate_cargo_home: Option<CanonicalScanRoot>,
    closure_sha256: [u8; 32],
    watched_directories: Vec<WatchedDirectory>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WatchedDirectory {
    directory: CanonicalScanRoot,
    entry_changes_terminal: bool,
}

/// Sealed evidence for one bounded negative config observation. It is
/// deliberately non-cloneable and cannot prove Cargo's exact read set.
pub(super) struct CargoConfigurationGuard {
    project_root: CanonicalScanRoot,
    cargo_home: CanonicalScanRoot,
    observation: ConfigAbsenceObservation,
    fence: platform::DirectoryMutationFence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CargoConfigurationEvidence {
    pub(super) policy_revision: u32,
    pub(super) lookup_count: u32,
    pub(super) closure_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CargoConfigurationError {
    Present,
    Changed,
    Unavailable,
}

impl CargoConfigurationGuard {
    pub(super) fn capture(
        project_root: &CanonicalScanRoot,
        cargo_home: &CanonicalScanRoot,
    ) -> Result<Self, CargoConfigurationError> {
        let observation = capture_absence(project_root, cargo_home)?;
        let fence = platform::DirectoryMutationFence::new(&observation.watched_directories)?;
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
        let observation = capture_absence(project_root, cargo_home)?;
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
        let current = match capture_absence(&self.project_root, &self.cargo_home) {
            Ok(current) => current,
            Err(CargoConfigurationError::Present | CargoConfigurationError::Changed) => {
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
            closure_sha256: self.observation.closure_sha256,
        })
    }
}

fn capture_absence(
    project_root: &CanonicalScanRoot,
    cargo_home: &CanonicalScanRoot,
) -> Result<ConfigAbsenceObservation, CargoConfigurationError> {
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
        if let Some(directory) = cargo_directory.as_ref() {
            platform::review_directory(directory)?;
            require_config_absent(directory.canonical_path(), &mut native_path_bytes)?;
        }
        lookups.push(LookupDirectory {
            parent,
            cargo_directory,
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
        require_config_absent(cargo_home.canonical_path(), &mut native_path_bytes)?;
        Some(cargo_home.clone())
    };

    let watched_directories = watched_directories(&lookups, &current_cargo_home)?;
    let closure_sha256 =
        digest_observation(&lookups, separate_cargo_home.as_ref(), &watched_directories);
    Ok(ConfigAbsenceObservation {
        lookups,
        separate_cargo_home,
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

fn require_config_absent(
    directory: &Path,
    native_path_bytes: &mut usize,
) -> Result<(), CargoConfigurationError> {
    for name in CONFIG_NAMES {
        let config = directory.join(name);
        charge_path(native_path_bytes, &config)?;
        match fs::symlink_metadata(config) {
            Ok(_) => return Err(CargoConfigurationError::Present),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(CargoConfigurationError::Unavailable),
        }
    }
    Ok(())
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
) -> Result<Vec<WatchedDirectory>, CargoConfigurationError> {
    let mut by_identity = BTreeMap::<(u64, u128), WatchedDirectory>::new();
    let project_root = lookups
        .first()
        .ok_or(CargoConfigurationError::Unavailable)?
        .parent
        .clone();
    let project_identity = project_root.identity();
    by_identity.insert(
        (project_identity.volume(), project_identity.object()),
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
        by_identity
            .entry((identity.volume(), identity.object()))
            .or_insert_with(|| WatchedDirectory {
                directory: directory.clone(),
                entry_changes_terminal: identity != cargo_home.identity(),
            });
    }
    let cargo_home_identity = cargo_home.identity();
    by_identity.insert(
        (cargo_home_identity.volume(), cargo_home_identity.object()),
        WatchedDirectory {
            directory: cargo_home.clone(),
            // Cargo legitimately updates locks and caches in its home during
            // metadata. Persistent config appearance is still caught by the
            // full before/after absence observation.
            entry_changes_terminal: false,
        },
    );
    if by_identity.len() > MAX_WATCHED_DIRECTORIES {
        return Err(CargoConfigurationError::Unavailable);
    }
    Ok(by_identity.into_values().collect())
}

fn digest_observation(
    lookups: &[LookupDirectory],
    cargo_home: Option<&CanonicalScanRoot>,
    watched_directories: &[WatchedDirectory],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"dux-cargo-config-absence-v2\0");
    for lookup in lookups {
        digest_directory(&mut digest, 1, &lookup.parent);
        match lookup.cargo_directory.as_ref() {
            Some(directory) => digest_directory(&mut digest, 2, directory),
            None => digest.update([0]),
        }
    }
    match cargo_home {
        Some(directory) => digest_directory(&mut digest, 3, directory),
        None => digest.update([0xff]),
    }
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
    use std::os::fd::{AsFd, AsRawFd, OwnedFd};

    use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl, open};
    use nix::libc;
    use nix::mount::MntFlags;
    use nix::sys::event::{EvFlags, EventFilter, FilterFlag, KEvent, Kqueue};
    use nix::sys::stat::{Mode, fstat};
    use nix::sys::statfs::fstatfs;

    use super::{CanonicalScanRoot, CargoConfigurationError, WatchedDirectory};

    pub(super) struct DirectoryMutationFence {
        queue: Kqueue,
        _directories: Vec<OwnedFd>,
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
            }
        }

        pub(super) fn new(
            expected_directories: &[WatchedDirectory],
        ) -> Result<Self, CargoConfigurationError> {
            let queue = Kqueue::new().map_err(|_| CargoConfigurationError::Unavailable)?;
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .map_err(|_| CargoConfigurationError::Unavailable)?;
            let mut directories = Vec::with_capacity(expected_directories.len());
            for expected in expected_directories {
                directories.push(open_watch_directory(&expected.directory)?);
            }
            let changes: Vec<KEvent> = directories
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
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{CanonicalScanRoot, CargoConfigurationError, WatchedDirectory};

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
    fn empty_closure_is_stable_and_config_presence_is_typed() {
        let temp = TempDir::new().unwrap();
        let (project, cargo_home) = roots(&temp);
        let guard =
            CargoConfigurationGuard::capture_unfenced_for_test(&project, &cargo_home).unwrap();
        let evidence = guard.evidence().unwrap();
        assert_eq!(evidence.policy_revision, CONFIG_POLICY_REVISION);
        assert!(evidence.lookup_count >= 2);
        assert_ne!(evidence.closure_sha256, [0; 32]);

        fs::write(cargo_home.canonical_path().join("config.toml"), "").unwrap();
        assert_eq!(guard.revalidate(), Err(CargoConfigurationError::Changed));
        assert_eq!(
            CargoConfigurationGuard::capture(&project, &cargo_home).err(),
            Some(CargoConfigurationError::Present)
        );
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test removes only a TempDir-owned config fixture before checking the ancestor lookup"
    )]
    fn extensionless_project_and_outer_configs_are_rejected() {
        let temp = TempDir::new().unwrap();
        let (project, cargo_home) = roots(&temp);
        let cargo = project.canonical_path().join(".cargo");
        fs::create_dir(&cargo).unwrap();
        fs::write(cargo.join("config"), "[build]").unwrap();
        assert_eq!(
            CargoConfigurationGuard::capture(&project, &cargo_home).err(),
            Some(CargoConfigurationError::Present)
        );

        // DUX-DESTRUCTIVE: allow=test-cargo-config-reset -- remove only the TempDir-owned project config so the same fixture can exercise the outer-ancestor lookup
        fs::remove_file(cargo.join("config")).unwrap();
        let outer = temp.path().join(".cargo");
        fs::create_dir(&outer).unwrap();
        fs::write(outer.join("config.toml"), "").unwrap();
        assert_eq!(
            CargoConfigurationGuard::capture(&project, &cargo_home).err(),
            Some(CargoConfigurationError::Present)
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
        let fence = platform::DirectoryMutationFence::new(std::slice::from_ref(&watched)).unwrap();
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
