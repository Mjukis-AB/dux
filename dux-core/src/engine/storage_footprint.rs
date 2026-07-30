use std::time::SystemTime;

/// Exact logical, allocated, and conservative per-file charged usage.
///
/// `charged_bytes` is the sum of each file's `max(logical, allocated)`, so it
/// can exceed both aggregate logical and aggregate allocated bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DuxOwnedStorageUsage {
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub charged_bytes: u64,
}

/// Path-free physical and policy categories for the active snapshot store.
///
/// Retention eligibility is an observation, not cleanup authority or a
/// reclaimable-space promise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DuxSnapshotStorageFootprint {
    pub cap_bytes: u64,
    pub cap_excess_bytes: u64,
    pub controls: DuxOwnedStorageUsage,
    pub available: DuxOwnedStorageUsage,
    pub protected: DuxOwnedStorageUsage,
    pub retention_eligible: DuxOwnedStorageUsage,
    pub tombstoned_residual: DuxOwnedStorageUsage,
    pub orphan: DuxOwnedStorageUsage,
    pub temporary_active: DuxOwnedStorageUsage,
    pub temporary_quiescent: DuxOwnedStorageUsage,
    pub temporary_unleased: DuxOwnedStorageUsage,
    pub total: DuxOwnedStorageUsage,
    pub available_count: u32,
    pub protected_count: u32,
    pub retention_eligible_count: u32,
    pub tombstoned_residual_count: u32,
    pub orphan_count: u32,
    pub active_temporary_count: u32,
    pub quiescent_temporary_count: u32,
    pub unleased_temporary_count: u32,
    pub residual_temporary_lease_count: u32,
    pub active_pin_rows: u32,
    pub expired_pin_rows: u32,
    pub non_evictable_over_cap: bool,
    pub accounting_unstable: bool,
}

/// Logical variable-length content embedded in the DUX SQLite database.
///
/// This includes insight identifiers, input digests, provider/adapter/model
/// labels, and response payloads. It excludes integer fields and SQLite
/// record, page, index, and fragmentation overhead. It is already part of the
/// database usage and must never be added to physical totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DuxEmbeddedAiCacheFootprint {
    pub record_count: u32,
    pub logical_content_bytes: u64,
    pub expired_record_count: u32,
    pub expired_logical_content_bytes: u64,
}

/// Physical usage inside DUX's fixed marker-owned managed scan-cache child.
///
/// The conventional outer cache container and legacy caller-selected cache
/// files are outside this ownership boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DuxManagedScanCacheFootprint {
    pub controls: DuxOwnedStorageUsage,
    pub entries: DuxOwnedStorageUsage,
    pub temporary: DuxOwnedStorageUsage,
    pub total: DuxOwnedStorageUsage,
    pub entry_count: u32,
    pub temporary_count: u32,
}

/// A bounded point-in-time observation of DUX's active private stores.
///
/// This excludes directory metadata, the conventional outer cache container,
/// legacy caller-selected CLI cache files, and unattributable interrupted
/// provisioning stages. It is neither free space nor an estimate of bytes
/// that cleanup would reclaim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DuxOwnedStorageFootprint {
    pub observed_at: SystemTime,
    pub database: DuxOwnedStorageUsage,
    pub snapshots: DuxSnapshotStorageFootprint,
    pub managed_scan_cache: DuxManagedScanCacheFootprint,
    pub embedded_ai_cache: DuxEmbeddedAiCacheFootprint,
    pub physical_total: DuxOwnedStorageUsage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DuxOwnedStorageFootprintError {
    #[error("engine is closed")]
    Closed,
    #[error("system clock cannot be represented")]
    InvalidClock,
    #[error("database schema is incompatible")]
    IncompatibleSchema,
    #[error("storage observation is busy")]
    Busy,
    #[error("storage ownership or permissions are unsafe")]
    UnsafeStorage,
    #[error("bounded observation budget was exceeded")]
    BudgetExceeded,
    #[error("stored accounting data is corrupt")]
    CorruptData,
    #[error("owned storage is unavailable")]
    Unavailable,
    #[error("internal storage accounting state is invalid")]
    InternalState,
}
