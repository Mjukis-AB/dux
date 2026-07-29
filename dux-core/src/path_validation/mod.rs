//! Non-authoritative path validation and live filesystem evidence.
//!
//! These types prove structural facts observed at one moment. They do not
//! authorize cleanup, bypass protected-path policy, or remain valid after the
//! filesystem changes.

mod filesystem;
mod lexical;
mod protected;

#[cfg(fuzzing)]
pub(crate) mod fuzz_support;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
mod unsupported {
    use std::fs::File;
    use std::path::Path;

    use super::filesystem::{
        CanonicalPathError, PlatformBoundarySnapshot, PlatformEntrySnapshot, PlatformPathSnapshot,
        PlatformRootSnapshot, TrashPlatformPathSnapshot,
    };

    pub(super) fn capture_root(_path: &Path) -> Result<PlatformRootSnapshot, CanonicalPathError> {
        Err(CanonicalPathError::UnsupportedPlatform)
    }

    pub(super) fn capture_boundary(
        _path: &Path,
    ) -> Result<PlatformBoundarySnapshot, CanonicalPathError> {
        Err(CanonicalPathError::UnsupportedPlatform)
    }

    pub(super) fn capture_descendant(
        _root: &Path,
        _relative_path: &Path,
    ) -> Result<PlatformPathSnapshot, CanonicalPathError> {
        Err(CanonicalPathError::UnsupportedPlatform)
    }

    pub(super) fn capture_trash_descendant(
        _root: &Path,
        _relative_path: &Path,
    ) -> Result<TrashPlatformPathSnapshot, CanonicalPathError> {
        Err(CanonicalPathError::UnsupportedPlatform)
    }

    pub(super) fn open_regular_descendant(
        _root: &Path,
        _relative_path: &Path,
    ) -> Result<(File, PlatformEntrySnapshot), CanonicalPathError> {
        Err(CanonicalPathError::UnsupportedPlatform)
    }

    pub(super) fn paths_equivalent(_requested: &Path, _canonical: &Path) -> bool {
        false
    }
}

#[allow(unused_imports)]
pub use filesystem::{
    AncestorIdentity, CanonicalPathError, CanonicalPathSnapshot, CanonicalScanRoot,
    FilesystemEntryKind, FilesystemIdentity,
};
#[allow(unused_imports)]
pub(crate) use filesystem::{TrashPathSnapshot, TrashTargetKind};
#[cfg(windows)]
#[allow(unused_imports)]
pub(crate) use lexical::WindowsComponentError;
// These crate-only entry points are intentionally staged for the protected-root
// registry. They must not become an arbitrary-path FFI or client API.
#[allow(unused_imports)]
pub(crate) use lexical::{LexicalCleanupPath, LexicalPathError, LexicalScanRoot};
#[allow(unused_imports)]
pub(crate) use protected::{
    KNOWN_USER_LIBRARY_CACHES_PROOF_REVISION, KnownUserLibraryCachesError,
    KnownUserLibraryCachesPath, PROTECTED_ROOT_POLICY_REVISION, ProtectedPathForm,
    ProtectedPathKind, ProtectedRootDisposition, ProtectedRootError, ProtectedRootRegistry,
    TRUSTED_HOME_MOUNT_PROOF_REVISION, TrustedHomeMountError, TrustedHomeMountWitness,
};

#[cfg(unix)]
use unix as platform;
#[cfg(not(any(unix, windows)))]
use unsupported as platform;
#[cfg(windows)]
use windows as platform;

#[allow(unused_imports)]
pub(crate) use filesystem::{
    CanonicalFileContentsSnapshot, CanonicalFileDigestError, CanonicalFileDigestSnapshot,
    CanonicalFilePrefixError, CanonicalFilePrefixSnapshot, FilesystemBoundarySnapshot,
    FilesystemCapacityScope, TRUSTED_VOLUME_LOCATION_PROOF_REVISION, TrustedVolumeLocationError,
    TrustedVolumeLocationWitness, capture_filesystem_boundary, capture_path_snapshot,
    capture_regular_file_contents, capture_regular_file_prefix, capture_regular_file_sha256,
    capture_scan_root, capture_trash_path_snapshot,
};
#[allow(unused_imports)]
pub(crate) use lexical::{validate_cleanup_path, validate_scan_root};
