use std::time::SystemTime;

use crate::domain::DiskPressureConfig;

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
