//! Consume-once confirmation for clearing removable DUX snapshot storage.
//!
//! Public observations are path-free. The opaque witness remains bound to one
//! repository and can authorize only the repository's exact, unchanged set of
//! retention-eligible available finals and tombstoned residual finals.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use crate::persistence::OwnedStorageUsage;
use crate::persistence::snapshot::{
    PreparedSnapshotStorageClear, SnapshotRepository, SnapshotStorageClearErrorKind,
    SnapshotStorageClearResult as StoredSnapshotStorageClearResult,
};

use super::storage_footprint::DuxOwnedStorageUsage;

pub(super) const SNAPSHOT_STORAGE_CLEAR_PREVIEW_LIFETIME: Duration = Duration::from_secs(2 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DuxSnapshotStorageClearPreviewInfo {
    eligible_snapshot_count: u32,
    tombstoned_residual_count: u32,
    clearable_count: u32,
    clearable: DuxOwnedStorageUsage,
    protected_snapshot_count: u32,
    protected: DuxOwnedStorageUsage,
    active_review_count: u32,
    excluded_maintenance_object_count: u32,
    excluded_maintenance: DuxOwnedStorageUsage,
    prepared_at: SystemTime,
    expires_at: SystemTime,
}

impl DuxSnapshotStorageClearPreviewInfo {
    pub const fn eligible_snapshot_count(&self) -> u32 {
        self.eligible_snapshot_count
    }

    pub const fn tombstoned_residual_count(&self) -> u32 {
        self.tombstoned_residual_count
    }

    pub const fn clearable_count(&self) -> u32 {
        self.clearable_count
    }

    pub const fn clearable(&self) -> DuxOwnedStorageUsage {
        self.clearable
    }

    pub const fn protected_snapshot_count(&self) -> u32 {
        self.protected_snapshot_count
    }

    pub const fn protected(&self) -> DuxOwnedStorageUsage {
        self.protected
    }

    pub const fn active_review_count(&self) -> u32 {
        self.active_review_count
    }

    pub const fn excluded_maintenance_object_count(&self) -> u32 {
        self.excluded_maintenance_object_count
    }

    pub const fn excluded_maintenance(&self) -> DuxOwnedStorageUsage {
        self.excluded_maintenance
    }

    pub const fn prepared_at(&self) -> SystemTime {
        self.prepared_at
    }

    pub const fn expires_at(&self) -> SystemTime {
        self.expires_at
    }
}

pub struct DuxSnapshotStorageClearPreview {
    owner: Weak<SnapshotRepository>,
    prepared: PreparedSnapshotStorageClear,
    info: DuxSnapshotStorageClearPreviewInfo,
    monotonic_expires_at: Instant,
}

impl std::fmt::Debug for DuxSnapshotStorageClearPreview {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DuxSnapshotStorageClearPreview")
            .field("info", &self.info)
            .field("owner_live", &self.owner.strong_count())
            .finish_non_exhaustive()
    }
}

impl DuxSnapshotStorageClearPreview {
    pub(super) fn new(
        repository: &Arc<SnapshotRepository>,
        prepared: PreparedSnapshotStorageClear,
        prepared_at: SystemTime,
        monotonic_now: Instant,
    ) -> Option<Self> {
        let clearable_count = prepared.clearable_count()?;
        if clearable_count == 0 {
            return None;
        }
        let monotonic_expires_at =
            monotonic_now.checked_add(SNAPSHOT_STORAGE_CLEAR_PREVIEW_LIFETIME)?;
        let expires_at = prepared_at.checked_add(SNAPSHOT_STORAGE_CLEAR_PREVIEW_LIFETIME)?;
        if prepared_at >= expires_at {
            return None;
        }
        let info = DuxSnapshotStorageClearPreviewInfo {
            eligible_snapshot_count: prepared.eligible_snapshot_count(),
            tombstoned_residual_count: prepared.tombstoned_residual_count(),
            clearable_count,
            clearable: public_usage(prepared.clearable()),
            protected_snapshot_count: prepared.protected_snapshot_count(),
            protected: public_usage(prepared.protected()),
            active_review_count: prepared.active_review_count(),
            excluded_maintenance_object_count: prepared.excluded_maintenance_object_count(),
            excluded_maintenance: public_usage(prepared.excluded_maintenance()),
            prepared_at,
            expires_at,
        };
        Some(Self {
            owner: Arc::downgrade(repository),
            prepared,
            info,
            monotonic_expires_at,
        })
    }

    pub fn info(&self) -> Result<DuxSnapshotStorageClearPreviewInfo, DuxSnapshotStorageClearError> {
        self.info_at(Instant::now())
    }

    pub(super) fn info_at(
        &self,
        now: Instant,
    ) -> Result<DuxSnapshotStorageClearPreviewInfo, DuxSnapshotStorageClearError> {
        if now >= self.monotonic_expires_at {
            return Err(DuxSnapshotStorageClearError::PreviewExpired);
        }
        Ok(self.info)
    }

    pub(super) fn belongs_to(&self, repository: &Arc<SnapshotRepository>) -> bool {
        self.owner
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, repository))
    }

    pub(super) fn into_prepared(
        self,
        now: Instant,
    ) -> Result<PreparedSnapshotStorageClear, DuxSnapshotStorageClearError> {
        if now >= self.monotonic_expires_at {
            return Err(DuxSnapshotStorageClearError::PreviewExpired);
        }
        Ok(self.prepared)
    }

    #[cfg(test)]
    pub(super) const fn monotonic_expires_at_for_test(&self) -> Instant {
        self.monotonic_expires_at
    }

    pub fn release(self) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DuxSnapshotStorageClearResult {
    cleared_eligible_snapshot_count: u32,
    cleared_tombstoned_residual_count: u32,
    cleared_count: u32,
    cleared_usage: DuxOwnedStorageUsage,
}

impl DuxSnapshotStorageClearResult {
    pub const fn cleared_eligible_snapshot_count(&self) -> u32 {
        self.cleared_eligible_snapshot_count
    }

    pub const fn cleared_tombstoned_residual_count(&self) -> u32 {
        self.cleared_tombstoned_residual_count
    }

    pub const fn cleared_count(&self) -> u32 {
        self.cleared_count
    }

    pub const fn cleared_usage(&self) -> DuxOwnedStorageUsage {
        self.cleared_usage
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DuxSnapshotStorageClearError {
    #[error("engine session is closed")]
    Closed,
    #[error("there are no removable snapshots to clear")]
    NothingToClear,
    #[error("the snapshot store is read-only")]
    ReadOnlyStore,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("snapshot storage changed after the clear preview")]
    ChangedSincePreview,
    #[error("the snapshot-storage clear preview expired")]
    PreviewExpired,
    #[error("the snapshot-storage clear preview belongs to another engine")]
    WrongEngine,
    #[error("snapshot storage is busy or has unstable temporary-file accounting")]
    Busy,
    #[error("snapshot storage ownership or permissions are unsafe")]
    UnsafeStorage,
    #[error("snapshot-storage validation exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("snapshot storage is corrupt")]
    CorruptData,
    #[error("the result of clearing snapshot storage is unknown")]
    OutcomeUnknown,
    #[error("snapshot storage is unavailable")]
    Unavailable,
    #[error("snapshot-storage clearing state is invalid")]
    InternalState,
}

pub(super) fn public_clear_result(
    result: StoredSnapshotStorageClearResult,
    expected: DuxSnapshotStorageClearPreviewInfo,
) -> Result<DuxSnapshotStorageClearResult, DuxSnapshotStorageClearError> {
    let cleared_count = result
        .cleared_count()
        .ok_or(DuxSnapshotStorageClearError::OutcomeUnknown)?;
    let cleared_usage = public_usage(result.cleared_usage);
    if result.cleared_eligible_snapshot_count != expected.eligible_snapshot_count
        || result.cleared_tombstoned_residual_count != expected.tombstoned_residual_count
        || cleared_count != expected.clearable_count
        || cleared_usage != expected.clearable
    {
        return Err(DuxSnapshotStorageClearError::OutcomeUnknown);
    }
    Ok(DuxSnapshotStorageClearResult {
        cleared_eligible_snapshot_count: result.cleared_eligible_snapshot_count,
        cleared_tombstoned_residual_count: result.cleared_tombstoned_residual_count,
        cleared_count,
        cleared_usage,
    })
}

pub(super) const fn map_clear_error(
    kind: SnapshotStorageClearErrorKind,
) -> DuxSnapshotStorageClearError {
    match kind {
        SnapshotStorageClearErrorKind::ReadOnlyStore => DuxSnapshotStorageClearError::ReadOnlyStore,
        SnapshotStorageClearErrorKind::ChangedSincePreview => {
            DuxSnapshotStorageClearError::ChangedSincePreview
        }
        SnapshotStorageClearErrorKind::Busy => DuxSnapshotStorageClearError::Busy,
        SnapshotStorageClearErrorKind::IncompatibleSchema => {
            DuxSnapshotStorageClearError::IncompatibleSchema
        }
        SnapshotStorageClearErrorKind::UnsafeStorage => DuxSnapshotStorageClearError::UnsafeStorage,
        SnapshotStorageClearErrorKind::BudgetExceeded => {
            DuxSnapshotStorageClearError::BudgetExceeded
        }
        SnapshotStorageClearErrorKind::CorruptData => DuxSnapshotStorageClearError::CorruptData,
        SnapshotStorageClearErrorKind::OutcomeUnknown => {
            DuxSnapshotStorageClearError::OutcomeUnknown
        }
        SnapshotStorageClearErrorKind::Unavailable => DuxSnapshotStorageClearError::Unavailable,
        SnapshotStorageClearErrorKind::InternalState => DuxSnapshotStorageClearError::InternalState,
    }
}

const fn public_usage(usage: OwnedStorageUsage) -> DuxOwnedStorageUsage {
    DuxOwnedStorageUsage {
        logical_bytes: usage.logical_bytes,
        allocated_bytes: usage.allocated_bytes,
        charged_bytes: usage.charged_bytes,
    }
}
