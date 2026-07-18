use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::domain::DiskPressureConfig;

/// Kind of macOS code signature bound to an explicitly inspected Cargo file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectCargoSignatureClass {
    /// A linker-generated signature without a publisher identity. Explicit
    /// enrollment trusts these exact bytes; it does not authenticate an author.
    AdHoc,
    /// A CMS signature. The exact signing evidence is still only one part of
    /// the enrollment identity and is never cleanup authority.
    Cms,
}

/// Exact bounded macOS signing evidence captured for direct Cargo enrollment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectCargoCodeSignature {
    pub class: DirectCargoSignatureClass,
    pub flags: u32,
    pub code_directory_hashes: Vec<Vec<u8>>,
    pub signing_identifier: String,
    pub team_identifier: Option<String>,
    pub designated_requirement_sha256: Option<[u8; 32]>,
}

/// One inspected Cargo executable that may be consumed exactly once by the
/// originating engine. Inspection does not modify settings.
#[must_use = "inspection must be explicitly committed to enroll Cargo"]
pub struct DirectCargoEnrollmentPreview {
    pub(crate) path: PathBuf,
    pub(crate) executable_sha256: [u8; 32],
    pub(crate) code_signature: DirectCargoCodeSignature,
    #[cfg(target_os = "macos")]
    pub(crate) inner: crate::planner::DirectCargoEnrollmentPreview,
    #[cfg(target_os = "macos")]
    pub(crate) owner: std::sync::Arc<()>,
}

impl std::fmt::Debug for DirectCargoEnrollmentPreview {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectCargoEnrollmentPreview")
            .field("path", &self.path)
            .field("executable_sha256", &self.executable_sha256)
            .field("code_signature", &self.code_signature)
            .finish_non_exhaustive()
    }
}

impl DirectCargoEnrollmentPreview {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn executable_sha256(&self) -> [u8; 32] {
        self.executable_sha256
    }

    pub fn code_signature(&self) -> &DirectCargoCodeSignature {
        &self.code_signature
    }
}

/// Durable explicit direct-Cargo trust state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectCargoEnrollmentState {
    /// No Cargo enrollment has ever been stored.
    NotEnrolled,
    /// The exact executable, version output, and static-code evidence enrolled
    /// by the user. This state grants discovery permission only.
    Enrolled {
        path: PathBuf,
        executable_sha256: [u8; 32],
        version_sha256: [u8; 32],
        cargo_release: [u32; 3],
        code_signature: Box<DirectCargoCodeSignature>,
    },
    /// A retained tombstone proving that a prior enrollment was revoked.
    Revoked,
}

/// Revisioned enrollment state from the shared DUX settings store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectCargoEnrollmentStatus {
    pub revision: u64,
    pub state: DirectCargoEnrollmentState,
    pub updated_at: Option<SystemTime>,
}

/// Result of explicitly enrolling or revoking direct Cargo discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectCargoEnrollmentUpdate {
    pub status: DirectCargoEnrollmentStatus,
    /// False when the exact enrollment or revocation was already effective.
    pub changed: bool,
}

/// Stable failure taxonomy for direct-Cargo inspection and enrollment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DirectCargoEnrollmentError {
    #[error("engine session is closed")]
    Closed,
    #[error("direct Cargo enrollment is supported only on macOS")]
    UnsupportedPlatform,
    #[error("the Cargo locator is not one exact canonical absolute file named cargo")]
    InvalidExecutableLocator,
    #[error("the Cargo executable is not a regular file")]
    ExecutableNotRegular,
    #[error("the Cargo executable or its environment changed during inspection")]
    ChangedDuringInspection,
    #[error("Cargo inspection could not run to completion")]
    InspectionUnavailable,
    #[error("Cargo inspection exceeded its fixed time or output budget")]
    InspectionLimitExceeded,
    #[error("Cargo resolution directories are not canonical directories")]
    InvalidResolutionEnvironment,
    #[error("Cargo verbose version is invalid or unsupported")]
    InvalidCargoVersion,
    #[error("Cargo does not have valid bounded macOS code-signing evidence")]
    InvalidCodeSignature,
    #[error("the inspection preview belongs to a different engine session")]
    WrongEngine,
    #[error("the enrollment revision cannot advance")]
    RevisionExhausted,
    #[error("the system clock cannot be represented by the settings store")]
    InvalidClock,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the settings query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("direct Cargo enrollment is corrupt")]
    CorruptData,
    #[error("direct Cargo enrollment is unavailable")]
    Unavailable,
    #[error("the enrollment write outcome could not be proven")]
    OutcomeUnknown,
    #[error("direct Cargo enrollment state is unavailable")]
    InternalState,
}

/// Origin of the effective disk-pressure policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskPressurePolicySource {
    /// The current versioned core default is effective. Revision zero means no
    /// row has ever been stored; later revisions are explicit reset epochs.
    Default,
    /// The user explicitly stored this configuration, including when its
    /// values happen to equal the current defaults.
    Stored,
}

/// Path-free deterministic disk-pressure configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskPressurePolicy {
    pub config: DiskPressureConfig,
    pub source: DiskPressurePolicySource,
    pub revision: u64,
    pub updated_at: Option<SystemTime>,
}

/// Result of an explicit pressure-policy change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskPressurePolicyUpdate {
    pub settings: DiskPressurePolicy,
    /// False when the exact source and values were already effective.
    pub changed: bool,
}

/// Stable, path-free failure taxonomy for pressure-policy settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DiskPressurePolicyError {
    #[error("engine session is closed")]
    Closed,
    #[error("the pressure policy revision cannot advance")]
    RevisionExhausted,
    #[error("the system clock cannot be represented by the settings store")]
    InvalidClock,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the settings query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("disk-pressure settings are corrupt")]
    CorruptData,
    #[error("disk-pressure settings are unavailable")]
    Unavailable,
    #[error("the settings write outcome could not be proven")]
    OutcomeUnknown,
    #[error("engine settings state is unavailable")]
    InternalState,
}

/// Origin of the effective snapshot-store size cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotRetentionCapSource {
    /// No stored override exists; the versioned core default is effective.
    Default,
    /// An explicit override is stored in the shared DUX database.
    Stored,
}

/// Path-free snapshot-retention configuration exposed by the shared engine.
///
/// This is policy input only. It cannot select a snapshot, create a retention
/// tombstone, or authorize a filesystem mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotRetentionCap {
    pub cap_bytes: u64,
    pub source: SnapshotRetentionCapSource,
    pub updated_at: Option<SystemTime>,
}

/// Result of an explicit settings change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotRetentionCapUpdate {
    pub settings: SnapshotRetentionCap,
    /// False when set/reset already had the exact requested effect.
    pub changed: bool,
}

/// Stable, path-free failure taxonomy for snapshot-retention settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SnapshotRetentionCapError {
    #[error("engine session is closed")]
    Closed,
    #[error("the system clock cannot be represented by the settings store")]
    InvalidClock,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the settings query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("snapshot-retention settings are corrupt")]
    CorruptData,
    #[error("snapshot-retention settings are unavailable")]
    Unavailable,
    #[error("the settings write outcome could not be proven")]
    OutcomeUnknown,
    #[error("engine settings state is unavailable")]
    InternalState,
}
