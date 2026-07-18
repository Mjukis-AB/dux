use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{LexicalCleanupPath, LexicalScanRoot, platform};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FilesystemIdentity {
    volume: u64,
    object: u128,
}

impl FilesystemIdentity {
    pub fn volume(self) -> u64 {
        self.volume
    }

    pub fn object(self) -> u128 {
        self.object
    }

    pub(super) fn new(volume: u64, object: u128) -> Self {
        Self { volume, object }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilesystemEntryKind {
    Directory,
    RegularFile,
}

/// The final object kind captured by the separate ad-hoc Trash witness.
///
/// Unlike [`FilesystemEntryKind`], this deliberately includes a symlink. The
/// witness never follows that final link; it exists so a future macOS Trash
/// adapter can move the link object itself while preserving the existing
/// no-symlink ancestry rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TrashTargetKind {
    Directory,
    RegularFile,
    Symlink,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AncestorIdentity {
    relative_path: PathBuf,
    identity: FilesystemIdentity,
}

impl AncestorIdentity {
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    pub fn identity(&self) -> FilesystemIdentity {
        self.identity
    }

    pub(super) fn new(relative_path: PathBuf, identity: FilesystemIdentity) -> Self {
        Self {
            relative_path,
            identity,
        }
    }
}

/// A live identity snapshot of a non-symlink directory selected as a scan root.
///
/// The snapshot can become stale immediately and grants no cleanup authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalScanRoot {
    requested_path: PathBuf,
    canonical_path: PathBuf,
    identity: FilesystemIdentity,
}

impl CanonicalScanRoot {
    pub fn requested_path(&self) -> &Path {
        &self.requested_path
    }

    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub fn identity(&self) -> FilesystemIdentity {
        self.identity
    }
}

/// Time-bound filesystem evidence for a single strict descendant of a scan root.
///
/// The path and identity are intentionally bound together. This snapshot can
/// become stale immediately and grants no cleanup authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalPathSnapshot {
    scan_root: PathBuf,
    requested_path: PathBuf,
    canonical_path: PathBuf,
    relative_path: PathBuf,
    target_identity: FilesystemIdentity,
    target_kind: FilesystemEntryKind,
    hard_link_count: u64,
    ancestors: Vec<AncestorIdentity>,
}

/// No-follow live evidence for an object selected for ad-hoc Trash.
///
/// The `object_path` is the validated lexical path to the directory entry; it
/// is intentionally not canonicalized through a final symlink. This is
/// momentary, non-authoritative evidence and cannot construct a cleanup plan
/// or invoke a platform effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrashPathSnapshot {
    scan_root: PathBuf,
    requested_path: PathBuf,
    object_path: PathBuf,
    relative_path: PathBuf,
    target_identity: FilesystemIdentity,
    target_kind: TrashTargetKind,
    hard_link_count: u64,
    ancestors: Vec<AncestorIdentity>,
}

impl TrashPathSnapshot {
    pub(crate) fn scan_root(&self) -> &Path {
        &self.scan_root
    }

    pub(crate) fn requested_path(&self) -> &Path {
        &self.requested_path
    }

    pub(crate) fn object_path(&self) -> &Path {
        &self.object_path
    }

    pub(crate) fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    pub(crate) fn target_identity(&self) -> FilesystemIdentity {
        self.target_identity
    }

    pub(crate) fn target_kind(&self) -> TrashTargetKind {
        self.target_kind
    }

    pub(crate) fn hard_link_count(&self) -> u64 {
        self.hard_link_count
    }

    pub(crate) fn ancestors(&self) -> &[AncestorIdentity] {
        &self.ancestors
    }
}

impl CanonicalPathSnapshot {
    pub fn scan_root(&self) -> &Path {
        &self.scan_root
    }

    pub fn requested_path(&self) -> &Path {
        &self.requested_path
    }

    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    pub fn target_identity(&self) -> FilesystemIdentity {
        self.target_identity
    }

    pub fn target_kind(&self) -> FilesystemEntryKind {
        self.target_kind
    }

    pub fn hard_link_count(&self) -> u64 {
        self.hard_link_count
    }

    pub fn ancestors(&self) -> &[AncestorIdentity] {
        &self.ancestors
    }
}

#[derive(Debug, Error)]
pub enum CanonicalPathError {
    #[error("filesystem identity checks are unsupported on this platform")]
    UnsupportedPlatform,
    #[error("path component {component_index} does not exist")]
    Missing { component_index: usize },
    #[error("path component {component_index} cannot be inspected")]
    AccessDenied { component_index: usize },
    #[error("I/O error at path component {component_index}: {kind:?}")]
    Io {
        component_index: usize,
        kind: io::ErrorKind,
        #[source]
        source: io::Error,
    },
    #[error("scan root is not a directory")]
    ScanRootNotDirectory,
    #[error("path component {component_index} is a symlink or reparse point")]
    SymlinkOrReparsePoint {
        component_index: usize,
        target: bool,
    },
    #[error("path component {component_index} is not a directory")]
    NonDirectoryAncestor { component_index: usize },
    #[error("cleanup target has an unsupported filesystem entry kind")]
    UnsupportedTargetKind,
    #[error("filesystem identity is unavailable at path component {component_index}")]
    IdentityUnavailable { component_index: usize },
    #[error(
        "path component {component_index} crosses volumes (expected {expected}, observed {observed})"
    )]
    CrossVolume {
        component_index: usize,
        expected: u64,
        observed: u64,
    },
    #[error("validated target does not belong to the supplied scan root")]
    MismatchedScanRoot,
    #[error("filesystem canonicalization failed for {target_label}")]
    CanonicalizationFailed {
        target_label: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("canonical target escapes its scan root")]
    CanonicalEscapesScanRoot,
    #[error("canonical {target_label} does not preserve the validated path location")]
    CanonicalPathMismatch { target_label: &'static str },
    #[error("path component {component_index} changed during validation")]
    ChangedDuringValidation { component_index: usize },
}

#[derive(Debug, Error)]
pub(crate) enum CanonicalFilePrefixError {
    #[error("invalid bounded file-prefix length")]
    InvalidLength,
    #[error(transparent)]
    Path(#[from] CanonicalPathError),
    #[error("validated file prefix cannot be read: {kind:?}")]
    Read {
        kind: io::ErrorKind,
        #[source]
        source: io::Error,
    },
}

/// A bounded prefix read from the exact regular-file object captured here.
///
/// The path is validated before and after the retained-handle read. This is
/// still momentary evidence, not a retained cleanup handle or authorization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CanonicalFilePrefixSnapshot {
    path: CanonicalPathSnapshot,
    prefix: Vec<u8>,
}

#[derive(Debug, Error)]
pub(crate) enum CanonicalFileDigestError {
    #[error("invalid bounded file-digest length")]
    InvalidLength,
    #[error("validated file exceeds its digest byte limit")]
    TooLarge,
    #[error(transparent)]
    Path(#[from] CanonicalPathError),
    #[error("validated file cannot be read for digest: {kind:?}")]
    Read {
        kind: io::ErrorKind,
        #[source]
        source: io::Error,
    },
}

/// A bounded full-file digest tied to one exact regular-file observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CanonicalFileDigestSnapshot {
    path: CanonicalPathSnapshot,
    byte_length: u64,
    sha256: [u8; 32],
}

/// Bounded full bytes read from the same exact regular-file descriptor as the
/// accompanying identity and digest snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CanonicalFileContentsSnapshot {
    file: CanonicalFileDigestSnapshot,
    contents: Vec<u8>,
}

impl CanonicalFileDigestSnapshot {
    pub(crate) fn path(&self) -> &CanonicalPathSnapshot {
        &self.path
    }

    pub(crate) fn byte_length(&self) -> u64 {
        self.byte_length
    }

    pub(crate) fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

impl CanonicalFileContentsSnapshot {
    pub(crate) fn file(&self) -> &CanonicalFileDigestSnapshot {
        &self.file
    }

    pub(crate) fn contents(&self) -> &[u8] {
        &self.contents
    }
}

impl CanonicalFilePrefixSnapshot {
    pub(crate) fn path(&self) -> &CanonicalPathSnapshot {
        &self.path
    }

    pub(crate) fn prefix(&self) -> &[u8] {
        &self.prefix
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PlatformEntrySnapshot {
    pub(super) identity: FilesystemIdentity,
    pub(super) kind: FilesystemEntryKind,
    pub(super) hard_link_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlatformRootSnapshot {
    pub(super) identity: FilesystemIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlatformPathSnapshot {
    pub(super) target: PlatformEntrySnapshot,
    pub(super) ancestors: Vec<AncestorIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TrashPlatformEntrySnapshot {
    pub(super) identity: FilesystemIdentity,
    pub(super) kind: TrashTargetKind,
    pub(super) hard_link_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TrashPlatformPathSnapshot {
    pub(super) target: TrashPlatformEntrySnapshot,
    pub(super) ancestors: Vec<AncestorIdentity>,
}

pub(crate) fn capture_scan_root(
    root: LexicalScanRoot,
) -> Result<CanonicalScanRoot, CanonicalPathError> {
    let first = platform::capture_root(root.as_path())?;
    let canonical_path = std::fs::canonicalize(root.as_path()).map_err(|source| {
        CanonicalPathError::CanonicalizationFailed {
            target_label: "scan root",
            source,
        }
    })?;
    if !platform::paths_equivalent(root.as_path(), &canonical_path) {
        return Err(CanonicalPathError::CanonicalPathMismatch {
            target_label: "scan root",
        });
    }
    let canonical = platform::capture_root(&canonical_path)?;
    let second = platform::capture_root(root.as_path())?;

    if first != second || first != canonical {
        return Err(CanonicalPathError::ChangedDuringValidation { component_index: 0 });
    }

    Ok(CanonicalScanRoot {
        requested_path: root.as_path().to_path_buf(),
        canonical_path,
        identity: first.identity,
    })
}

pub(crate) fn capture_path_snapshot(
    root: &CanonicalScanRoot,
    target: LexicalCleanupPath,
) -> Result<CanonicalPathSnapshot, CanonicalPathError> {
    if target.scan_root() != root.requested_path() {
        return Err(CanonicalPathError::MismatchedScanRoot);
    }

    let current_root = platform::capture_root(root.canonical_path())?;
    if current_root.identity != root.identity {
        return Err(CanonicalPathError::ChangedDuringValidation { component_index: 0 });
    }

    let first =
        platform::capture_descendant(root.canonical_path(), target.relative_to_scan_root())?;
    if first.ancestors.first().map(AncestorIdentity::identity) != Some(root.identity) {
        return Err(CanonicalPathError::ChangedDuringValidation { component_index: 0 });
    }
    let target_path = root.canonical_path().join(target.relative_to_scan_root());
    let canonical_path = std::fs::canonicalize(&target_path).map_err(|source| {
        CanonicalPathError::CanonicalizationFailed {
            target_label: "cleanup target",
            source,
        }
    })?;
    if !platform::paths_equivalent(&target_path, &canonical_path) {
        return Err(CanonicalPathError::CanonicalPathMismatch {
            target_label: "cleanup target",
        });
    }
    let relative_path = canonical_path
        .strip_prefix(root.canonical_path())
        .map_err(|_| CanonicalPathError::CanonicalEscapesScanRoot)?
        .to_path_buf();
    if relative_path.as_os_str().is_empty() {
        return Err(CanonicalPathError::CanonicalEscapesScanRoot);
    }
    let canonical = platform::capture_descendant(root.canonical_path(), &relative_path)?;
    if canonical != first {
        return Err(CanonicalPathError::ChangedDuringValidation {
            component_index: first.ancestors.len(),
        });
    }

    let second =
        platform::capture_descendant(root.canonical_path(), target.relative_to_scan_root())?;
    if first != second {
        return Err(CanonicalPathError::ChangedDuringValidation {
            component_index: first.ancestors.len(),
        });
    }

    Ok(CanonicalPathSnapshot {
        scan_root: root.canonical_path.clone(),
        requested_path: target.as_path().to_path_buf(),
        canonical_path,
        relative_path,
        target_identity: first.target.identity,
        target_kind: first.target.kind,
        hard_link_count: first.target.hard_link_count,
        ancestors: first.ancestors,
    })
}

pub(crate) fn capture_trash_path_snapshot(
    root: &CanonicalScanRoot,
    target: LexicalCleanupPath,
) -> Result<TrashPathSnapshot, CanonicalPathError> {
    if target.scan_root() != root.requested_path() {
        return Err(CanonicalPathError::MismatchedScanRoot);
    }

    let first_root = platform::capture_root(root.canonical_path())?;
    if first_root.identity != root.identity {
        return Err(CanonicalPathError::ChangedDuringValidation { component_index: 0 });
    }

    let relative_path = target.relative_to_scan_root().to_path_buf();
    let first = platform::capture_trash_descendant(root.canonical_path(), &relative_path)?;
    if first.ancestors.first().map(AncestorIdentity::identity) != Some(root.identity) {
        return Err(CanonicalPathError::ChangedDuringValidation { component_index: 0 });
    }
    let second = platform::capture_trash_descendant(root.canonical_path(), &relative_path)?;
    let second_root = platform::capture_root(root.canonical_path())?;
    if first != second || first_root != second_root || first_root.identity != root.identity {
        return Err(CanonicalPathError::ChangedDuringValidation {
            component_index: first.ancestors.len(),
        });
    }

    Ok(TrashPathSnapshot {
        scan_root: root.canonical_path.clone(),
        requested_path: target.as_path().to_path_buf(),
        object_path: root.canonical_path.join(&relative_path),
        relative_path,
        target_identity: first.target.identity,
        target_kind: first.target.kind,
        hard_link_count: first.target.hard_link_count,
        ancestors: first.ancestors,
    })
}

const MAX_FILE_PREFIX_BYTES: usize = 4 * 1024;

pub(crate) fn capture_regular_file_prefix(
    root: &CanonicalScanRoot,
    target: LexicalCleanupPath,
    prefix_length: usize,
) -> Result<CanonicalFilePrefixSnapshot, CanonicalFilePrefixError> {
    if prefix_length == 0 || prefix_length > MAX_FILE_PREFIX_BYTES {
        return Err(CanonicalFilePrefixError::InvalidLength);
    }

    let before = capture_path_snapshot(root, target.clone())?;
    if before.target_kind != FilesystemEntryKind::RegularFile {
        return Err(CanonicalPathError::UnsupportedTargetKind.into());
    }

    let (mut file, opened) =
        platform::open_regular_descendant(root.canonical_path(), target.relative_to_scan_root())?;
    if opened.identity != before.target_identity
        || opened.kind != before.target_kind
        || opened.hard_link_count != before.hard_link_count
    {
        return Err(CanonicalPathError::ChangedDuringValidation {
            component_index: before.ancestors.len(),
        }
        .into());
    }

    let mut prefix = vec![0_u8; prefix_length];
    file.read_exact(&mut prefix)
        .map_err(|source| CanonicalFilePrefixError::Read {
            kind: source.kind(),
            source,
        })?;

    let after = capture_path_snapshot(root, target)?;
    if before != after {
        return Err(CanonicalPathError::ChangedDuringValidation {
            component_index: before.ancestors.len(),
        }
        .into());
    }

    Ok(CanonicalFilePrefixSnapshot {
        path: after,
        prefix,
    })
}

pub(crate) fn capture_regular_file_sha256(
    root: &CanonicalScanRoot,
    target: LexicalCleanupPath,
    maximum_bytes: usize,
) -> Result<CanonicalFileDigestSnapshot, CanonicalFileDigestError> {
    if maximum_bytes == 0 {
        return Err(CanonicalFileDigestError::InvalidLength);
    }

    let before = capture_path_snapshot(root, target.clone())?;
    if before.target_kind != FilesystemEntryKind::RegularFile {
        return Err(CanonicalPathError::UnsupportedTargetKind.into());
    }
    let (mut file, opened) =
        platform::open_regular_descendant(root.canonical_path(), target.relative_to_scan_root())?;
    if opened.identity != before.target_identity
        || opened.kind != before.target_kind
        || opened.hard_link_count != before.hard_link_count
    {
        return Err(CanonicalPathError::ChangedDuringValidation {
            component_index: before.ancestors.len(),
        }
        .into());
    }

    let mut digest = Sha256::new();
    let mut byte_length = 0_usize;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|source| CanonicalFileDigestError::Read {
                kind: source.kind(),
                source,
            })?;
        if count == 0 {
            break;
        }
        byte_length = byte_length
            .checked_add(count)
            .ok_or(CanonicalFileDigestError::TooLarge)?;
        if byte_length > maximum_bytes {
            return Err(CanonicalFileDigestError::TooLarge);
        }
        digest.update(&buffer[..count]);
    }

    let after = capture_path_snapshot(root, target)?;
    if before != after {
        return Err(CanonicalPathError::ChangedDuringValidation {
            component_index: before.ancestors.len(),
        }
        .into());
    }
    Ok(CanonicalFileDigestSnapshot {
        path: after,
        byte_length: byte_length as u64,
        sha256: digest.finalize().into(),
    })
}

pub(crate) fn capture_regular_file_contents(
    root: &CanonicalScanRoot,
    target: LexicalCleanupPath,
    maximum_bytes: usize,
) -> Result<CanonicalFileContentsSnapshot, CanonicalFileDigestError> {
    if maximum_bytes == 0 {
        return Err(CanonicalFileDigestError::InvalidLength);
    }

    let before = capture_path_snapshot(root, target.clone())?;
    if before.target_kind != FilesystemEntryKind::RegularFile {
        return Err(CanonicalPathError::UnsupportedTargetKind.into());
    }
    let (mut file, opened) =
        platform::open_regular_descendant(root.canonical_path(), target.relative_to_scan_root())?;
    if opened.identity != before.target_identity
        || opened.kind != before.target_kind
        || opened.hard_link_count != before.hard_link_count
    {
        return Err(CanonicalPathError::ChangedDuringValidation {
            component_index: before.ancestors.len(),
        }
        .into());
    }

    let mut contents = Vec::new();
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|source| CanonicalFileDigestError::Read {
                kind: source.kind(),
                source,
            })?;
        if count == 0 {
            break;
        }
        let byte_length = contents
            .len()
            .checked_add(count)
            .ok_or(CanonicalFileDigestError::TooLarge)?;
        if byte_length > maximum_bytes {
            return Err(CanonicalFileDigestError::TooLarge);
        }
        contents.extend_from_slice(&buffer[..count]);
        digest.update(&buffer[..count]);
    }

    let after = capture_path_snapshot(root, target)?;
    if before != after {
        return Err(CanonicalPathError::ChangedDuringValidation {
            component_index: before.ancestors.len(),
        }
        .into());
    }
    Ok(CanonicalFileContentsSnapshot {
        file: CanonicalFileDigestSnapshot {
            path: after,
            byte_length: contents.len() as u64,
            sha256: digest.finalize().into(),
        },
        contents,
    })
}

pub(super) fn map_io_error(component_index: usize, source: io::Error) -> CanonicalPathError {
    match source.kind() {
        io::ErrorKind::NotFound => CanonicalPathError::Missing { component_index },
        io::ErrorKind::PermissionDenied => CanonicalPathError::AccessDenied { component_index },
        kind => CanonicalPathError::Io {
            component_index,
            kind,
            source,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_validation::{validate_cleanup_path, validate_scan_root};
    use tempfile::TempDir;

    struct Fixture {
        _temp: TempDir,
        root_path: PathBuf,
        lexical_root: LexicalScanRoot,
        canonical_root: CanonicalScanRoot,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let root_path = std::fs::canonicalize(temp.path()).unwrap();
            let lexical_root = validate_scan_root(&root_path).unwrap();
            let canonical_root = capture_scan_root(lexical_root.clone()).unwrap();
            Self {
                _temp: temp,
                root_path,
                lexical_root,
                canonical_root,
            }
        }

        fn capture(
            &self,
            relative_path: &str,
        ) -> Result<CanonicalPathSnapshot, CanonicalPathError> {
            let path = self.root_path.join(relative_path);
            let target = validate_cleanup_path(&self.lexical_root, &path).unwrap();
            capture_path_snapshot(&self.canonical_root, target)
        }

        fn capture_trash(
            &self,
            relative_path: &str,
        ) -> Result<TrashPathSnapshot, CanonicalPathError> {
            let path = self.root_path.join(relative_path);
            let target = validate_cleanup_path(&self.lexical_root, &path).unwrap();
            capture_trash_path_snapshot(&self.canonical_root, target)
        }
    }

    #[test]
    fn captures_regular_files_directories_and_ordered_ancestor_identities() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.root_path.join("parent")).unwrap();
        std::fs::write(fixture.root_path.join("parent/file"), b"unchanged").unwrap();

        let directory = fixture.capture("parent").unwrap();
        let file = fixture.capture("parent/file").unwrap();

        assert_eq!(directory.target_kind(), FilesystemEntryKind::Directory);
        assert_eq!(file.target_kind(), FilesystemEntryKind::RegularFile);
        assert_eq!(file.relative_path(), Path::new("parent/file"));
        assert_eq!(
            file.ancestors()
                .iter()
                .map(AncestorIdentity::relative_path)
                .collect::<Vec<_>>(),
            vec![Path::new(""), Path::new("parent")]
        );
        assert_eq!(std::fs::read(file.requested_path()).unwrap(), b"unchanged");
    }

    #[test]
    fn missing_targets_fail_closed() {
        let fixture = Fixture::new();
        assert!(matches!(
            fixture.capture("missing"),
            Err(CanonicalPathError::Missing { component_index: 1 })
        ));
    }

    #[test]
    fn regular_files_cannot_be_scan_roots() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("file");
        std::fs::write(&path, b"data").unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let lexical = validate_scan_root(&path).unwrap();

        let result = capture_scan_root(lexical);
        assert!(
            matches!(result, Err(CanonicalPathError::ScanRootNotDirectory)),
            "unexpected scan-root result: {result:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn final_and_ancestor_symlinks_fail_closed() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        std::fs::create_dir(fixture.root_path.join("real")).unwrap();
        std::fs::write(fixture.root_path.join("real/file"), b"data").unwrap();
        symlink("real/file", fixture.root_path.join("file-link")).unwrap();
        symlink("real", fixture.root_path.join("directory-link")).unwrap();
        symlink("missing", fixture.root_path.join("dangling-link")).unwrap();
        symlink("loop-link", fixture.root_path.join("loop-link")).unwrap();

        assert!(matches!(
            fixture.capture("file-link"),
            Err(CanonicalPathError::SymlinkOrReparsePoint { target: true, .. })
        ));
        assert!(matches!(
            fixture.capture("directory-link/file"),
            Err(CanonicalPathError::SymlinkOrReparsePoint { target: false, .. })
        ));
        assert!(matches!(
            fixture.capture("dangling-link"),
            Err(CanonicalPathError::SymlinkOrReparsePoint { target: true, .. })
        ));
        assert!(matches!(
            fixture.capture("loop-link"),
            Err(CanonicalPathError::SymlinkOrReparsePoint { target: true, .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn trash_witness_captures_final_symlink_without_following_it() {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        std::fs::create_dir(fixture.root_path.join("real")).unwrap();
        let target = fixture.root_path.join("real/file");
        std::fs::write(&target, b"target remains").unwrap();
        symlink("real/file", fixture.root_path.join("file-link")).unwrap();
        symlink("missing", fixture.root_path.join("dangling-link")).unwrap();
        symlink("loop-link", fixture.root_path.join("loop-link")).unwrap();

        let target_identity = std::fs::symlink_metadata(&target).unwrap();
        let link = fixture.capture_trash("file-link").unwrap();
        assert_eq!(link.target_kind(), TrashTargetKind::Symlink);
        assert_eq!(link.object_path(), fixture.root_path.join("file-link"));
        assert_eq!(
            link.target_identity().object(),
            std::fs::symlink_metadata(fixture.root_path.join("file-link"))
                .unwrap()
                .ino()
                .into()
        );
        assert_ne!(
            link.target_identity().object(),
            target_identity.ino().into()
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"target remains");

        assert_eq!(
            fixture
                .capture_trash("dangling-link")
                .unwrap()
                .target_kind(),
            TrashTargetKind::Symlink
        );
        assert_eq!(
            fixture.capture_trash("loop-link").unwrap().target_kind(),
            TrashTargetKind::Symlink
        );
    }

    #[cfg(unix)]
    #[test]
    fn trash_witness_keeps_ancestor_and_special_entry_guards() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        std::fs::create_dir(fixture.root_path.join("real")).unwrap();
        std::fs::write(fixture.root_path.join("real/file"), b"data").unwrap();
        symlink("real", fixture.root_path.join("directory-link")).unwrap();

        assert!(matches!(
            fixture.capture_trash("directory-link/file"),
            Err(CanonicalPathError::SymlinkOrReparsePoint { target: false, .. })
        ));
    }

    #[test]
    fn trash_witness_accepts_non_symlink_objects_without_canonicalization() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.root_path.join("directory")).unwrap();
        std::fs::write(fixture.root_path.join("file"), b"data").unwrap();

        assert_eq!(
            fixture.capture_trash("directory").unwrap().target_kind(),
            TrashTargetKind::Directory
        );
        assert_eq!(
            fixture.capture_trash("file").unwrap().target_kind(),
            TrashTargetKind::RegularFile
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_scan_roots_fail_closed() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let real = temp.path().join("real");
        let linked = temp.path().join("linked");
        std::fs::create_dir(&real).unwrap();
        symlink(&real, &linked).unwrap();
        let lexical = validate_scan_root(&linked).unwrap();

        assert!(matches!(
            capture_scan_root(lexical),
            Err(CanonicalPathError::SymlinkOrReparsePoint { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn special_files_fail_closed() {
        use std::os::unix::net::UnixListener;

        let fixture = Fixture::new();
        let socket_path = fixture.root_path.join("socket");
        let _listener = UnixListener::bind(&socket_path).unwrap();

        assert!(matches!(
            fixture.capture("socket"),
            Err(CanonicalPathError::UnsupportedTargetKind)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn hard_links_share_identity_and_expose_link_count() {
        let fixture = Fixture::new();
        let first_path = fixture.root_path.join("first");
        let second_path = fixture.root_path.join("second");
        std::fs::write(&first_path, b"same inode").unwrap();
        std::fs::hard_link(&first_path, &second_path).unwrap();

        let first = fixture.capture("first").unwrap();
        let second = fixture.capture("second").unwrap();

        assert_eq!(first.target_identity(), second.target_identity());
        assert!(first.hard_link_count() >= 2);
        assert!(second.hard_link_count() >= 2);
    }

    #[test]
    fn target_evidence_cannot_be_paired_with_another_scan_root() {
        let first = Fixture::new();
        let second = Fixture::new();
        let target_path = first.root_path.join("file");
        std::fs::write(&target_path, b"data").unwrap();
        let target = validate_cleanup_path(&first.lexical_root, &target_path).unwrap();

        assert!(matches!(
            capture_path_snapshot(&second.canonical_root, target),
            Err(CanonicalPathError::MismatchedScanRoot)
        ));
    }

    #[test]
    fn diagnostic_errors_do_not_echo_requested_paths() {
        let fixture = Fixture::new();
        let error = fixture.capture("sensitive-name").unwrap_err().to_string();
        assert!(!error.contains("sensitive-name"));
        assert!(!error.contains(&fixture.root_path.to_string_lossy().to_string()));
    }
}
