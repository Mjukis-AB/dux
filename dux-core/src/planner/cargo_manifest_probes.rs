//! Bounded ancestor-manifest namespace for Cargo 1.96 workspace discovery.
//!
//! Cargo searches for an implicit workspace by probing `Cargo.toml` in
//! ancestor directories. This guard independently captures every candidate
//! entry before Cargo runs and keeps both present files and absent-entry
//! namespaces fenced throughout resolution.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::path_validation::{
    CanonicalFileDigestSnapshot, CanonicalScanRoot, FilesystemEntryKind,
    capture_regular_file_sha256, capture_scan_root, validate_cleanup_path, validate_scan_root,
};

const PROBE_POLICY_REVISION: u32 = 2;
const MAX_ANCESTOR_PROBES: usize = 64;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_MANIFEST_CLOSURE_BYTES: usize = 64 * 1024 * 1024;
const MAX_NATIVE_PATH_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AncestorManifestProbe {
    directory: CanonicalScanRoot,
    path: PathBuf,
    manifest: Option<CanonicalFileDigestSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManifestProbeObservation {
    probes: Vec<AncestorManifestProbe>,
    manifest_count: usize,
    manifest_bytes: usize,
    closure_sha256: [u8; 32],
}

/// Exact ancestor entry state retained across both Cargo metadata passes.
pub(super) struct CargoManifestProbeGuard {
    project_root: CanonicalScanRoot,
    cargo_home: CanonicalScanRoot,
    observation: ManifestProbeObservation,
    fence: platform::ManifestProbeFence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CargoManifestProbeEvidence {
    pub(super) policy_revision: u32,
    pub(super) probe_count: u32,
    pub(super) manifest_count: u32,
    pub(super) absent_probe_count: u32,
    pub(super) manifest_bytes: u64,
    pub(super) closure_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CargoManifestProbeError {
    Unsupported,
    Changed,
    Unavailable,
}

impl CargoManifestProbeGuard {
    pub(super) fn capture(
        project_root: &CanonicalScanRoot,
        cargo_home: &CanonicalScanRoot,
    ) -> Result<Self, CargoManifestProbeError> {
        let fence_cursor =
            platform::ManifestProbeFenceCursor::capture(project_root.identity().volume())?;
        let observation = capture_observation(project_root, cargo_home)?;
        let fence = platform::ManifestProbeFence::new(&observation.probes, fence_cursor)?;
        let guard = Self {
            project_root: project_root.clone(),
            cargo_home: cargo_home.clone(),
            observation,
            fence,
        };
        guard.revalidate()?;
        Ok(guard)
    }

    #[cfg(test)]
    pub(super) fn capture_unfenced_for_test(
        project_root: &CanonicalScanRoot,
        cargo_home: &CanonicalScanRoot,
    ) -> Result<Self, CargoManifestProbeError> {
        Ok(Self {
            project_root: project_root.clone(),
            cargo_home: cargo_home.clone(),
            observation: capture_observation(project_root, cargo_home)?,
            fence: platform::ManifestProbeFence::disabled_for_test(),
        })
    }

    pub(super) fn poll(&self) -> Result<(), CargoManifestProbeError> {
        match self.fence.poll()? {
            platform::ManifestProbeFencePoll::Unchanged => Ok(()),
            platform::ManifestProbeFencePoll::ManifestChanged => {
                Err(CargoManifestProbeError::Changed)
            }
            platform::ManifestProbeFencePoll::NamespaceChanged => {
                self.fence.flush_exact_events()?;
                let current = capture_observation(&self.project_root, &self.cargo_home)?;
                if current == self.observation {
                    Ok(())
                } else {
                    Err(CargoManifestProbeError::Changed)
                }
            }
        }
    }

    pub(super) fn revalidate(&self) -> Result<(), CargoManifestProbeError> {
        self.fence.flush_exact_events()?;
        self.poll()?;
        let current = capture_observation(&self.project_root, &self.cargo_home).map_err(
            |error| match error {
                CargoManifestProbeError::Unsupported | CargoManifestProbeError::Changed => {
                    CargoManifestProbeError::Changed
                }
                CargoManifestProbeError::Unavailable => CargoManifestProbeError::Unavailable,
            },
        )?;
        if current != self.observation {
            return Err(CargoManifestProbeError::Changed);
        }
        self.fence.flush_exact_events()?;
        self.poll()
    }

    pub(super) fn evidence(&self) -> Result<CargoManifestProbeEvidence, CargoManifestProbeError> {
        self.revalidate()?;
        Ok(CargoManifestProbeEvidence {
            policy_revision: PROBE_POLICY_REVISION,
            probe_count: u32::try_from(self.observation.probes.len())
                .map_err(|_| CargoManifestProbeError::Unavailable)?,
            manifest_count: u32::try_from(self.observation.manifest_count)
                .map_err(|_| CargoManifestProbeError::Unavailable)?,
            absent_probe_count: u32::try_from(
                self.observation
                    .probes
                    .len()
                    .saturating_sub(self.observation.manifest_count),
            )
            .map_err(|_| CargoManifestProbeError::Unavailable)?,
            manifest_bytes: u64::try_from(self.observation.manifest_bytes)
                .map_err(|_| CargoManifestProbeError::Unavailable)?,
            closure_sha256: self.observation.closure_sha256,
        })
    }
}

fn capture_observation(
    project_root: &CanonicalScanRoot,
    cargo_home: &CanonicalScanRoot,
) -> Result<ManifestProbeObservation, CargoManifestProbeError> {
    let current_project = capture_exact_directory(project_root.canonical_path())?;
    let current_cargo_home = capture_exact_directory(cargo_home.canonical_path())?;
    if &current_project != project_root || &current_cargo_home != cargo_home {
        return Err(CargoManifestProbeError::Changed);
    }

    let mut probes = Vec::new();
    let mut manifest_count = 0_usize;
    let mut manifest_bytes = 0_usize;
    let mut native_path_bytes = 0_usize;
    let mut previous = None::<&Path>;
    let ancestors = project_root
        .canonical_path()
        .parent()
        .into_iter()
        .flat_map(Path::ancestors);
    for directory_path in ancestors {
        // Cargo's LookBehind boundary includes CARGO_HOME itself, then stops
        // before considering its parent.
        if previous.is_some_and(|path| path == cargo_home.canonical_path()) {
            break;
        }
        if directory_path.ends_with("target/package") {
            break;
        }
        if probes.len() == MAX_ANCESTOR_PROBES {
            return Err(CargoManifestProbeError::Unavailable);
        }
        charge_path(&mut native_path_bytes, directory_path)?;
        let directory = capture_exact_directory(directory_path)?;
        platform::review_directory(&directory)?;
        let path = directory_path.join("Cargo.toml");
        charge_path(&mut native_path_bytes, &path)?;
        let manifest = capture_optional_manifest(&directory, &path)?;
        if let Some(file) = manifest.as_ref() {
            manifest_count = manifest_count
                .checked_add(1)
                .ok_or(CargoManifestProbeError::Unavailable)?;
            let bytes = usize::try_from(file.byte_length())
                .map_err(|_| CargoManifestProbeError::Unavailable)?;
            manifest_bytes = manifest_bytes
                .checked_add(bytes)
                .ok_or(CargoManifestProbeError::Unavailable)?;
            if manifest_bytes > MAX_MANIFEST_CLOSURE_BYTES {
                return Err(CargoManifestProbeError::Unavailable);
            }
        }
        probes.push(AncestorManifestProbe {
            directory,
            path,
            manifest,
        });
        previous = Some(directory_path);
    }

    let closure_sha256 = digest_observation(&probes, manifest_count, manifest_bytes);
    Ok(ManifestProbeObservation {
        probes,
        manifest_count,
        manifest_bytes,
        closure_sha256,
    })
}

fn capture_optional_manifest(
    directory: &CanonicalScanRoot,
    path: &Path,
) -> Result<Option<CanonicalFileDigestSnapshot>, CargoManifestProbeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(CargoManifestProbeError::Unsupported);
            }
            let lexical_root = validate_scan_root(directory.canonical_path())
                .map_err(|_| CargoManifestProbeError::Unsupported)?;
            let lexical_file = validate_cleanup_path(&lexical_root, path)
                .map_err(|_| CargoManifestProbeError::Unsupported)?;
            let manifest = capture_regular_file_sha256(directory, lexical_file, MAX_MANIFEST_BYTES)
                .map_err(|_| CargoManifestProbeError::Unsupported)?;
            if manifest.path().target_kind() != FilesystemEntryKind::RegularFile
                || manifest.path().hard_link_count() != 1
            {
                return Err(CargoManifestProbeError::Unsupported);
            }
            Ok(Some(manifest))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(CargoManifestProbeError::Unavailable),
    }
}

fn capture_exact_directory(path: &Path) -> Result<CanonicalScanRoot, CargoManifestProbeError> {
    let lexical = validate_scan_root(path).map_err(|_| CargoManifestProbeError::Unavailable)?;
    capture_scan_root(lexical).map_err(|_| CargoManifestProbeError::Unavailable)
}

fn charge_path(total: &mut usize, path: &Path) -> Result<(), CargoManifestProbeError> {
    let Some(text) = path.to_str() else {
        return Err(CargoManifestProbeError::Unsupported);
    };
    if text.chars().any(char::is_control) {
        return Err(CargoManifestProbeError::Unsupported);
    }
    *total = total
        .checked_add(path.as_os_str().as_bytes().len())
        .ok_or(CargoManifestProbeError::Unavailable)?;
    if *total > MAX_NATIVE_PATH_BYTES {
        return Err(CargoManifestProbeError::Unavailable);
    }
    Ok(())
}

fn digest_observation(
    probes: &[AncestorManifestProbe],
    manifest_count: usize,
    manifest_bytes: usize,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"dux-cargo-ancestor-manifest-probes-v2\0");
    digest.update((probes.len() as u64).to_le_bytes());
    digest.update((manifest_count as u64).to_le_bytes());
    digest.update((manifest_bytes as u64).to_le_bytes());
    for (ordinal, probe) in probes.iter().enumerate() {
        digest.update((ordinal as u64).to_le_bytes());
        let path = probe.path.as_os_str().as_bytes();
        digest.update((path.len() as u64).to_le_bytes());
        digest.update(path);
        let directory_identity = probe.directory.identity();
        digest.update(directory_identity.volume().to_le_bytes());
        digest.update(directory_identity.object().to_le_bytes());
        match probe.manifest.as_ref() {
            Some(manifest) => {
                digest.update([1]);
                let identity = manifest.path().target_identity();
                digest.update(identity.volume().to_le_bytes());
                digest.update(identity.object().to_le_bytes());
                digest.update(manifest.byte_length().to_le_bytes());
                digest.update(manifest.sha256());
            }
            None => digest.update([0]),
        }
    }
    digest.finalize().into()
}

#[cfg(target_os = "macos")]
#[path = "cargo_manifest_probes/fsevents_macos.rs"]
mod fsevents_macos;

#[cfg(target_os = "macos")]
mod platform {
    use std::fs;
    use std::mem::MaybeUninit;
    use std::os::fd::{AsFd, AsRawFd, OwnedFd};
    use std::sync::atomic::{AtomicBool, Ordering};

    use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl, open};
    use nix::libc;
    use nix::mount::MntFlags;
    use nix::sys::event::{EvFlags, EventFilter, FilterFlag, KEvent, Kqueue};
    use nix::sys::stat::{Mode, fstat};
    use nix::sys::statfs::fstatfs;

    use super::{
        AncestorManifestProbe, CanonicalScanRoot, CargoManifestProbeError,
        fsevents_macos::{ExactManifestEventCursor, ExactManifestEventFence},
    };

    pub(super) struct ManifestProbeFenceCursor {
        exact: ExactManifestEventCursor,
    }

    impl ManifestProbeFenceCursor {
        pub(super) fn capture(volume: u64) -> Result<Self, CargoManifestProbeError> {
            Ok(Self {
                exact: ExactManifestEventCursor::capture(volume)?,
            })
        }
    }

    pub(super) struct ManifestProbeFence {
        queue: Kqueue,
        _directories: Vec<OwnedFd>,
        _files: Vec<OwnedFd>,
        exact_events: ExactManifestEventFence,
        pending_namespace_change: AtomicBool,
    }

    pub(super) enum ManifestProbeFencePoll {
        Unchanged,
        NamespaceChanged,
        ManifestChanged,
    }

    pub(super) fn review_directory(
        expected: &CanonicalScanRoot,
    ) -> Result<(), CargoManifestProbeError> {
        drop(open_watch_directory(expected)?);
        Ok(())
    }

    impl ManifestProbeFence {
        #[cfg(test)]
        pub(super) fn disabled_for_test() -> Self {
            let queue = Kqueue::new().expect("test kqueue must be available");
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .expect("test kqueue must support close-on-exec");
            Self {
                queue,
                _directories: Vec::new(),
                _files: Vec::new(),
                exact_events: ExactManifestEventFence::disabled(),
                pending_namespace_change: AtomicBool::new(false),
            }
        }

        pub(super) fn new(
            probes: &[AncestorManifestProbe],
            cursor: ManifestProbeFenceCursor,
        ) -> Result<Self, CargoManifestProbeError> {
            let retained = probes
                .len()
                .checked_add(
                    probes
                        .iter()
                        .filter(|probe| probe.manifest.is_some())
                        .count(),
                )
                .and_then(|count| count.checked_add(1))
                .ok_or(CargoManifestProbeError::Unavailable)?;
            require_descriptor_budget(retained)?;
            let queue = Kqueue::new().map_err(|_| CargoManifestProbeError::Unavailable)?;
            fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
                .map_err(|_| CargoManifestProbeError::Unavailable)?;
            let directories = probes
                .iter()
                .map(|probe| open_watch_directory(&probe.directory))
                .collect::<Result<Vec<_>, _>>()?;
            let files = probes
                .iter()
                .filter_map(|probe| probe.manifest.as_ref())
                .map(open_watch_file)
                .collect::<Result<Vec<_>, _>>()?;
            let directory_flags = FilterFlag::NOTE_DELETE
                | FilterFlag::NOTE_WRITE
                | FilterFlag::NOTE_RENAME
                | FilterFlag::NOTE_REVOKE;
            let file_flags = FilterFlag::NOTE_DELETE
                | FilterFlag::NOTE_WRITE
                | FilterFlag::NOTE_EXTEND
                | FilterFlag::NOTE_ATTRIB
                | FilterFlag::NOTE_LINK
                | FilterFlag::NOTE_RENAME
                | FilterFlag::NOTE_REVOKE;
            let changes: Vec<KEvent> = directories
                .iter()
                .map(|directory| vnode_event(directory.as_raw_fd(), directory_flags, 1))
                .chain(
                    files
                        .iter()
                        .map(|file| vnode_event(file.as_raw_fd(), file_flags, 2)),
                )
                .collect();
            let mut events = vec![empty_event(); changes.len().max(1)];
            let count = queue
                .kevent(&changes, &mut events, Some(zero_timeout()))
                .map_err(|_| CargoManifestProbeError::Unavailable)?;
            if events[..count]
                .iter()
                .any(|event| event.flags().contains(EvFlags::EV_ERROR))
            {
                return Err(CargoManifestProbeError::Unavailable);
            }
            let mut pending_namespace_change = false;
            for event in &events[..count] {
                match event.udata() {
                    1 if event.fflags().intersects(
                        FilterFlag::NOTE_DELETE | FilterFlag::NOTE_RENAME | FilterFlag::NOTE_REVOKE,
                    ) =>
                    {
                        return Err(CargoManifestProbeError::Changed);
                    }
                    1 if event.fflags().contains(FilterFlag::NOTE_WRITE) => {
                        pending_namespace_change = true;
                    }
                    2 => return Err(CargoManifestProbeError::Changed),
                    _ => return Err(CargoManifestProbeError::Unavailable),
                }
            }
            let exact_events = ExactManifestEventFence::new(probes, cursor.exact)?;
            Ok(Self {
                queue,
                _directories: directories,
                _files: files,
                exact_events,
                pending_namespace_change: AtomicBool::new(pending_namespace_change),
            })
        }

        pub(super) fn flush_exact_events(&self) -> Result<(), CargoManifestProbeError> {
            self.exact_events.flush()
        }

        pub(super) fn poll(&self) -> Result<ManifestProbeFencePoll, CargoManifestProbeError> {
            self.exact_events.poll()?;
            let mut outcome = if self.pending_namespace_change.swap(false, Ordering::AcqRel) {
                ManifestProbeFencePoll::NamespaceChanged
            } else {
                ManifestProbeFencePoll::Unchanged
            };
            loop {
                let mut event = [empty_event()];
                let count = self
                    .queue
                    .kevent(&[], &mut event, Some(zero_timeout()))
                    .map_err(|_| CargoManifestProbeError::Unavailable)?;
                if count == 0 {
                    return Ok(outcome);
                }
                if event[0].flags().contains(EvFlags::EV_ERROR) {
                    return Err(CargoManifestProbeError::Unavailable);
                }
                outcome = match event[0].udata() {
                    1 if event[0].fflags().intersects(
                        FilterFlag::NOTE_DELETE | FilterFlag::NOTE_RENAME | FilterFlag::NOTE_REVOKE,
                    ) =>
                    {
                        ManifestProbeFencePoll::ManifestChanged
                    }
                    1 if matches!(outcome, ManifestProbeFencePoll::Unchanged) => {
                        ManifestProbeFencePoll::NamespaceChanged
                    }
                    1 => outcome,
                    2 => ManifestProbeFencePoll::ManifestChanged,
                    _ => return Err(CargoManifestProbeError::Unavailable),
                };
            }
        }
    }

    fn open_watch_directory(
        expected: &CanonicalScanRoot,
    ) -> Result<OwnedFd, CargoManifestProbeError> {
        let directory = open(
            expected.canonical_path(),
            OFlag::from_bits_retain(libc::O_EVTONLY) | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| CargoManifestProbeError::Unavailable)?;
        require_reviewed_filesystem(&directory)?;
        let status = fstat(&directory).map_err(|_| CargoManifestProbeError::Unavailable)?;
        let identity = expected.identity();
        if status.st_dev as u64 != identity.volume()
            || u128::from(status.st_ino) != identity.object()
        {
            return Err(CargoManifestProbeError::Changed);
        }
        Ok(directory)
    }

    fn open_watch_file(
        expected: &super::CanonicalFileDigestSnapshot,
    ) -> Result<OwnedFd, CargoManifestProbeError> {
        let file = open(
            expected.path().canonical_path(),
            OFlag::from_bits_retain(libc::O_EVTONLY) | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| CargoManifestProbeError::Unavailable)?;
        require_reviewed_filesystem(&file)?;
        let status = fstat(&file).map_err(|_| CargoManifestProbeError::Unavailable)?;
        let identity = expected.path().target_identity();
        if status.st_dev as u64 != identity.volume()
            || u128::from(status.st_ino) != identity.object()
            || status.st_nlink != 1
        {
            return Err(CargoManifestProbeError::Changed);
        }
        Ok(file)
    }

    fn require_reviewed_filesystem(descriptor: &impl AsFd) -> Result<(), CargoManifestProbeError> {
        let status = fstatfs(descriptor).map_err(|_| CargoManifestProbeError::Unavailable)?;
        if status.filesystem_type_name().eq_ignore_ascii_case("apfs")
            && status.flags().contains(MntFlags::MNT_LOCAL)
        {
            Ok(())
        } else {
            Err(CargoManifestProbeError::Unavailable)
        }
    }

    fn require_descriptor_budget(retained: usize) -> Result<(), CargoManifestProbeError> {
        const RESERVE_FOR_PROCESS_AND_LAUNCH: u64 = 128;
        let mut limit = MaybeUninit::<libc::rlimit>::zeroed();
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) } == -1 {
            return Err(CargoManifestProbeError::Unavailable);
        }
        let soft_limit = unsafe { limit.assume_init() }.rlim_cur;
        if soft_limit == libc::RLIM_INFINITY {
            return Ok(());
        }
        let open_descriptors = fs::read_dir("/dev/fd")
            .map_err(|_| CargoManifestProbeError::Unavailable)?
            .try_fold(0_u64, |count, entry| {
                entry
                    .map(|_| count.saturating_add(1))
                    .map_err(|_| CargoManifestProbeError::Unavailable)
            })?;
        let retained = u64::try_from(retained).map_err(|_| CargoManifestProbeError::Unavailable)?;
        if open_descriptors
            .checked_add(retained)
            .and_then(|count| count.checked_add(RESERVE_FOR_PROCESS_AND_LAUNCH))
            .is_some_and(|required| required <= soft_limit)
        {
            Ok(())
        } else {
            Err(CargoManifestProbeError::Unavailable)
        }
    }

    fn vnode_event(descriptor: libc::c_int, flags: FilterFlag, tag: isize) -> KEvent {
        KEvent::new(
            descriptor as usize,
            EventFilter::EVFILT_VNODE,
            EvFlags::EV_ADD | EvFlags::EV_ENABLE | EvFlags::EV_CLEAR,
            flags,
            0,
            tag,
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
    use super::{AncestorManifestProbe, CanonicalScanRoot, CargoManifestProbeError};

    pub(super) struct ManifestProbeFence;
    pub(super) struct ManifestProbeFenceCursor;

    impl ManifestProbeFenceCursor {
        pub(super) fn capture(_volume: u64) -> Result<Self, CargoManifestProbeError> {
            Ok(Self)
        }
    }

    pub(super) enum ManifestProbeFencePoll {
        Unchanged,
        NamespaceChanged,
        ManifestChanged,
    }

    pub(super) fn review_directory(
        _expected: &CanonicalScanRoot,
    ) -> Result<(), CargoManifestProbeError> {
        Ok(())
    }

    impl ManifestProbeFence {
        pub(super) fn disabled_for_test() -> Self {
            Self
        }

        pub(super) fn poll(&self) -> Result<ManifestProbeFencePoll, CargoManifestProbeError> {
            Ok(ManifestProbeFencePoll::Unchanged)
        }

        pub(super) fn flush_exact_events(&self) -> Result<(), CargoManifestProbeError> {
            Ok(())
        }

        #[allow(dead_code)]
        pub(super) fn new(
            _probes: &[AncestorManifestProbe],
            _cursor: ManifestProbeFenceCursor,
        ) -> Result<Self, CargoManifestProbeError> {
            Err(CargoManifestProbeError::Unavailable)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    fn roots(temp: &TempDir) -> (CanonicalScanRoot, CanonicalScanRoot, PathBuf, PathBuf) {
        let base = fs::canonicalize(temp.path()).unwrap();
        let parent = base.join("parent");
        let project = parent.join("project");
        let cargo_home = base.join("cargo-home");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&cargo_home).unwrap();
        let project_root = capture_exact_directory(&project).unwrap();
        let cargo_home_root = capture_exact_directory(&cargo_home).unwrap();
        (project_root, cargo_home_root, parent, project)
    }

    #[test]
    fn captures_present_and_absent_ancestor_entries() {
        let temp = TempDir::new().unwrap();
        let (project, cargo_home, parent, _) = roots(&temp);
        fs::write(
            parent.join("Cargo.toml"),
            b"[package]\nname='outer'\nversion='0.1.0'\n",
        )
        .unwrap();
        let guard =
            CargoManifestProbeGuard::capture_unfenced_for_test(&project, &cargo_home).unwrap();
        let evidence = guard.evidence().unwrap();
        assert!(evidence.probe_count >= 2);
        assert_eq!(evidence.manifest_count, 1);
        assert!(evidence.manifest_bytes > 0);
        assert_ne!(evidence.closure_sha256, [0; 32]);
    }

    #[test]
    fn absent_entry_creation_and_present_file_mutation_change_observation() {
        let temp = TempDir::new().unwrap();
        let (project, cargo_home, parent, _) = roots(&temp);
        let guard =
            CargoManifestProbeGuard::capture_unfenced_for_test(&project, &cargo_home).unwrap();
        fs::write(parent.join("Cargo.toml"), b"[workspace]\n").unwrap();
        assert_eq!(guard.revalidate(), Err(CargoManifestProbeError::Changed));

        let guard =
            CargoManifestProbeGuard::capture_unfenced_for_test(&project, &cargo_home).unwrap();
        fs::write(parent.join("Cargo.toml"), b"[workspace]\nmembers=[]\n").unwrap();
        assert_eq!(guard.revalidate(), Err(CargoManifestProbeError::Changed));
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test replaces only a TempDir-owned probe entry to exercise alias rejection"
    )]
    fn aliases_and_non_regular_candidates_fail_closed() {
        let temp = TempDir::new().unwrap();
        let (project, cargo_home, parent, _) = roots(&temp);
        let source = temp.path().join("source");
        fs::write(&source, b"[workspace]\n").unwrap();
        symlink(&source, parent.join("Cargo.toml")).unwrap();
        assert!(matches!(
            CargoManifestProbeGuard::capture_unfenced_for_test(&project, &cargo_home),
            Err(CargoManifestProbeError::Unsupported)
        ));
        // DUX-DESTRUCTIVE: allow=test-manifest-probe-alias-replace -- replace only a TempDir-owned probe entry to exercise symlink and hard-link rejection
        fs::remove_file(parent.join("Cargo.toml")).unwrap();
        fs::hard_link(&source, parent.join("Cargo.toml")).unwrap();
        assert!(matches!(
            CargoManifestProbeGuard::capture_unfenced_for_test(&project, &cargo_home),
            Err(CargoManifestProbeError::Unsupported)
        ));
    }

    #[test]
    fn target_package_boundary_stops_search() {
        let temp = TempDir::new().unwrap();
        let base = fs::canonicalize(temp.path()).unwrap();
        let package = base.join("target/package");
        let project_path = package.join("crate");
        let cargo_home_path = base.join("cargo-home");
        fs::create_dir_all(&project_path).unwrap();
        fs::create_dir_all(&cargo_home_path).unwrap();
        fs::write(package.join("Cargo.toml"), b"[workspace]\n").unwrap();
        let project = capture_exact_directory(&project_path).unwrap();
        let cargo_home = capture_exact_directory(&cargo_home_path).unwrap();
        let guard =
            CargoManifestProbeGuard::capture_unfenced_for_test(&project, &cargo_home).unwrap();
        let evidence = guard.evidence().unwrap();
        assert_eq!(evidence.probe_count, 0);
        assert_eq!(evidence.manifest_count, 0);
    }

    #[test]
    fn cargo_home_is_included_then_stops_and_equal_project_root_keeps_source_quirk() {
        let temp = TempDir::new().unwrap();
        let base = fs::canonicalize(temp.path()).unwrap();
        let cargo_home_path = base.join("cargo-home");
        let project_path = cargo_home_path.join("nested/project");
        fs::create_dir_all(&project_path).unwrap();
        let project = capture_exact_directory(&project_path).unwrap();
        let cargo_home = capture_exact_directory(&cargo_home_path).unwrap();
        let guard =
            CargoManifestProbeGuard::capture_unfenced_for_test(&project, &cargo_home).unwrap();
        let paths: Vec<&Path> = guard
            .observation
            .probes
            .iter()
            .map(|probe| probe.directory.canonical_path())
            .collect();
        assert_eq!(paths, [cargo_home_path.join("nested"), cargo_home_path]);

        let equal =
            CargoManifestProbeGuard::capture_unfenced_for_test(&cargo_home, &cargo_home).unwrap();
        assert_eq!(
            equal
                .observation
                .probes
                .first()
                .unwrap()
                .directory
                .canonical_path(),
            base
        );
    }

    #[test]
    fn more_than_sixty_four_candidates_and_oversized_files_fail_closed() {
        let temp = TempDir::new().unwrap();
        let base = fs::canonicalize(temp.path()).unwrap();
        let cargo_home_path = base.join("cargo-home");
        fs::create_dir(&cargo_home_path).unwrap();
        let mut deep = base.join("deep");
        for _ in 0..65 {
            deep.push("d");
        }
        fs::create_dir_all(&deep).unwrap();
        let project = capture_exact_directory(&deep).unwrap();
        let cargo_home = capture_exact_directory(&cargo_home_path).unwrap();
        assert!(matches!(
            CargoManifestProbeGuard::capture_unfenced_for_test(&project, &cargo_home),
            Err(CargoManifestProbeError::Unavailable)
        ));

        let parent = base.join("bounded");
        let shallow = parent.join("project");
        fs::create_dir_all(&shallow).unwrap();
        fs::write(
            parent.join("Cargo.toml"),
            vec![b'x'; MAX_MANIFEST_BYTES + 1],
        )
        .unwrap();
        let project = capture_exact_directory(&shallow).unwrap();
        assert!(matches!(
            CargoManifestProbeGuard::capture_unfenced_for_test(&project, &cargo_home),
            Err(CargoManifestProbeError::Unsupported)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn persistent_namespace_change_and_manifest_write_restore_are_rejected() {
        let temp = TempDir::new().unwrap();
        let (project, cargo_home, parent, _) = roots(&temp);
        let path = parent.join("Cargo.toml");
        let absent = CargoManifestProbeGuard::capture(&project, &cargo_home).unwrap();
        fs::write(&path, b"[workspace]\n").unwrap();
        assert_eq!(absent.poll(), Err(CargoManifestProbeError::Changed));
        drop(absent);

        let original = b"[workspace]\n";
        let present = CargoManifestProbeGuard::capture(&project, &cargo_home).unwrap();
        fs::write(&path, b"[workspace]\nmembers=[]\n").unwrap();
        fs::write(&path, original).unwrap();
        assert_eq!(present.poll(), Err(CargoManifestProbeError::Changed));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test removes only TempDir-owned names to distinguish exact FSEvents evidence"
    )]
    fn absent_create_remove_is_terminal_but_unrelated_restoration_replays() {
        let temp = TempDir::new().unwrap();
        let (project, cargo_home, parent, _) = roots(&temp);
        let guard = CargoManifestProbeGuard::capture(&project, &cargo_home).unwrap();

        let unrelated = parent.join("unrelated.tmp");
        fs::write(&unrelated, b"temporary\n").unwrap();
        // DUX-DESTRUCTIVE: allow=test-manifest-probe-unrelated-remove -- remove only a TempDir-owned unrelated sibling to prove exact-name filtering
        fs::remove_file(&unrelated).unwrap();
        assert_eq!(guard.revalidate(), Ok(()));

        let manifest = parent.join("Cargo.toml");
        fs::write(&manifest, b"[workspace]\n").unwrap();
        // DUX-DESTRUCTIVE: allow=test-manifest-probe-absent-remove -- remove only a TempDir-owned transient Cargo manifest to prove event persistence
        fs::remove_file(&manifest).unwrap();
        assert_eq!(guard.revalidate(), Err(CargoManifestProbeError::Changed));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test renames and removes only TempDir-owned directories to prove ancestor replacement remains terminal"
    )]
    fn ancestor_directory_rename_and_restore_is_terminal() {
        let temp = TempDir::new().unwrap();
        let (project, cargo_home, parent, _) = roots(&temp);
        let guard = CargoManifestProbeGuard::capture(&project, &cargo_home).unwrap();
        let displaced = parent.with_file_name("displaced-parent");
        // DUX-DESTRUCTIVE: allow=test-manifest-probe-ancestor-rename -- rename only the TempDir-owned watched ancestor before restoring it to prove the vnode rename event remains terminal
        fs::rename(&parent, &displaced).unwrap();
        fs::create_dir(&parent).unwrap();
        // DUX-DESTRUCTIVE: allow=test-manifest-probe-replacement-remove -- remove only the empty TempDir-owned replacement before restoring the watched ancestor
        fs::remove_dir(&parent).unwrap();
        // DUX-DESTRUCTIVE: allow=test-manifest-probe-ancestor-restore -- restore only the displaced TempDir-owned watched ancestor after exercising replacement
        fs::rename(&displaced, &parent).unwrap();
        assert_eq!(guard.poll(), Err(CargoManifestProbeError::Changed));
    }
}
