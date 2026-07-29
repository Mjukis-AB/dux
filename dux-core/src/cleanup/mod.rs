//! Cleanup effect boundaries.

use std::path::PathBuf;

use crate::path_validation::TrashTargetKind;

/// Stable target-kind metadata for the one-shot Explorer Trash callback.
/// The callback receives this only after the core has revalidated the target
/// and fenced the journal receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrashEffectTargetKind {
    Directory,
    File,
    Symlink,
}

/// A core-issued, ephemeral Trash request. The path is consumed by the
/// platform callback during the journal-held call and cannot be queried or
/// reused afterward.
#[must_use = "a Trash request must be consumed by the platform callback"]
pub struct TrashEffectRequest {
    target_kind: TrashEffectTargetKind,
    absolute_path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TrashEffectRequestError {
    #[error("the Trash request path cannot be represented losslessly")]
    UnsupportedPathEncoding,
}

impl TrashEffectRequest {
    pub(crate) fn from_target(
        target_kind: TrashTargetKind,
        absolute_path: &std::path::Path,
    ) -> Self {
        Self {
            target_kind: match target_kind {
                TrashTargetKind::Directory => TrashEffectTargetKind::Directory,
                TrashTargetKind::RegularFile => TrashEffectTargetKind::File,
                TrashTargetKind::Symlink => TrashEffectTargetKind::Symlink,
            },
            absolute_path: absolute_path.to_path_buf(),
        }
    }

    pub fn target_kind(&self) -> TrashEffectTargetKind {
        self.target_kind
    }

    /// Consume this request into the exact host path bytes expected by the
    /// synchronous platform adapter. No string-lossy conversion is allowed.
    pub fn into_parts(self) -> Result<(TrashEffectTargetKind, Vec<u8>), TrashEffectRequestError> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Ok((
                self.target_kind,
                self.absolute_path.as_os_str().as_bytes().to_vec(),
            ))
        }
        #[cfg(not(unix))]
        {
            let _ = self;
            Err(TrashEffectRequestError::UnsupportedPathEncoding)
        }
    }
}

/// Result returned by the synchronous platform Trash callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrashPlatformResult {
    Completed,
    Unsupported,
    Failed,
    OutcomeUnknown,
}

/// Path-free, bounded failure returned before a platform callback can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TrashSelectionError {
    #[error("the Explorer review could not supply a valid Trash target")]
    Review,
    #[error("the Explorer Trash selection request is invalid")]
    InvalidRequest,
    #[error("the reviewed Explorer Trash target changed since the plan was created")]
    ChangedSincePlan,
    #[error("the cleanup journal is temporarily busy")]
    Busy,
    #[error("the cleanup store is unavailable")]
    Unavailable,
    #[error("the cleanup store is read-only or unsafe")]
    UnsafeStorage,
    #[error("the cleanup schema is incompatible")]
    IncompatibleSchema,
    #[error("the cleanup journal is corrupt")]
    CorruptData,
    #[error("the cleanup operation outcome is unknown")]
    OutcomeUnknown,
    #[error("the cleanup engine is closed or internally unavailable")]
    InternalState,
}

pub(crate) use executor::execute_reviewed_trash_selection;

pub(crate) mod capacity;

#[allow(
    dead_code,
    reason = "journal-fenced Trash admission is consumed by the platform adapter slice"
)]
pub(crate) mod executor;

#[allow(
    dead_code,
    reason = "the permanent-safe driver is reachable only through the crate-private approved-session engine bridge"
)]
pub(crate) mod permanent_safe;
