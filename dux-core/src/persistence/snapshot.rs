//! Immutable, private, non-authoritative scan snapshots.
//!
//! The legacy CLI cache is intentionally separate. Snapshot bytes may support
//! presentation and discovery, but never path validation or cleanup authority.

use std::cell::Cell;
use std::io::{Seek, SeekFrom};
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use rusqlite::TransactionBehavior;

use crate::domain::{ScanCoverage, ScanId};

use super::SnapshotReviewPurpose;
use super::candidate_evaluation_history::{
    CandidateEvaluationCompletion, CandidateEvaluationIdentity, NewCandidateEvaluation,
};
use super::footprint::{
    DuxOwnedStorageFootprint, OwnedSnapshotStorageFootprint, OwnedStorageUsage,
};
use super::history::{
    HistoryError, HistoryErrorKind, ScanCompletionRecord, ScanCounts, ScanStatus,
    map_write_sql_error, system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::process_liveness::{ProcessIdentityError, ProcessInstanceId, current_process_instance};
use super::snapshot_retention::{
    PreparedSnapshotRetentionTombstone, SnapshotRetentionState,
    insert_snapshot_retention_tombstone, load_snapshot_retention_state,
    reconcile_snapshot_retention_tombstone_insert,
};
use super::snapshot_retention_inventory::{
    SnapshotRetentionInventory, SnapshotRetentionLogicalState, SnapshotRetentionUsage,
    SnapshotTemporaryState, build_snapshot_physical_orphan_inventory,
    build_snapshot_retention_inventory,
};
#[cfg(test)]
use super::snapshot_review_pin::{MAX_ACTIVE_PINS, MAX_EXPIRED_PRUNE};
use super::snapshot_review_pin::{
    MAX_ACTIVE_PINS_PER_OWNER, PreparedSnapshotReviewPin, SNAPSHOT_REVIEW_PIN_ID_ATTEMPTS,
    SnapshotReviewPinId, SnapshotReviewPinState, insert_snapshot_review_pin_candidates,
    inspect_snapshot_review_pin_population, release_snapshot_review_pin, renew_snapshot_review_pin,
    snapshot_review_pin_exactly_matches, snapshot_review_pin_state, validate_snapshot_review_pin,
};
use super::snapshot_temp_lease::{
    PreparedSnapshotTempLease, SnapshotTempLeaseId, SnapshotTempLeaseState,
    delete_snapshot_temp_lease, insert_snapshot_temp_lease, inspect_snapshot_temp_leases,
    reconcile_snapshot_temp_lease_delete, reconcile_snapshot_temp_lease_insert,
    snapshot_temp_lease_state,
};
use super::snapshot_terminal_temp_inventory::{
    SnapshotTerminalTempPhysicalState, build_snapshot_terminal_temp_inventory,
};
use super::snapshot_unleased_temp_inventory::{
    SnapshotUnleasedTempPhysicalState, build_snapshot_unleased_temp_inventory,
};
use super::store::{AppDataResetStoreGuard, HistoryConnectionGuard, StoreCoordinator};

mod codec;
pub(crate) mod from_scan;
pub(crate) mod storage;

pub use codec::SNAPSHOT_FORMAT_VERSION;
pub(crate) use codec::{
    HostEncoding, HostValue, MAX_SNAPSHOT_DEPTH, MAX_SNAPSHOT_NODES, SnapshotCodecError,
    SnapshotCodecErrorKind, SnapshotDigest, SnapshotDocument, SnapshotMetadata, SnapshotNode,
    SnapshotNodeKind, SnapshotScanFlags, SnapshotTimestamp, SnapshotTotals, SnapshotUnixIdentity,
    decode_snapshot, encode_snapshot, validate_snapshot_document,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) use storage::{
    AppDataResetSnapshotPayloadDrainBatch, AppDataResetSnapshotPayloadDrainCandidate,
    AppDataResetSnapshotPayloadDrainCompletion, AppDataResetSnapshotPayloadDrainError,
    AppDataResetSnapshotStoreRetirementBatch, AppDataResetSnapshotStoreRetirementCompletion,
    AppDataResetSnapshotStoreRetirementError, AppDataResetSnapshotStoreRetirementState,
};
pub(crate) use storage::{
    RetainedSnapshot, SecureSnapshotStore, SnapshotFileName, SnapshotFileUsage,
    SnapshotFinalRemovalError, SnapshotInventoryEntryKind, SnapshotPublication,
    SnapshotPublicationLease, SnapshotStageReservation, SnapshotStorageError,
    SnapshotStorageErrorKind, SnapshotStoreAccess, SnapshotStoreInventoryLease,
    SnapshotTempKernelState, SnapshotTempMutationLease, SnapshotTempRemovalError, StagedSnapshot,
};
use storage::{
    SnapshotProvisioningStageReconciliation as StorageProvisioningStageReconciliation,
    SnapshotProvisioningStageRemoval as StorageProvisioningStageRemoval,
    SnapshotProvisioningStageRemovalError,
};
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) use storage::{
    TestAppDataResetSnapshotPayloadDrainFault, TestAppDataResetSnapshotStoreRetirementFault,
    set_test_app_data_reset_snapshot_payload_drain_fault,
    set_test_app_data_reset_snapshot_store_retirement_fault,
};
const PUBLICATION_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Stable path-free category for snapshot-store startup failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotOpenErrorKind {
    InvalidConfiguration,
    UnsafeRoot,
    UnsafeObject,
    UnrecognizedStore,
    Unavailable,
    Busy,
    InternalState,
}

/// Typed SQLite snapshot tuple. It proves storage syntax, not file validity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotReference {
    scan_id: ScanId,
    version: NonZeroU32,
    file_name: SnapshotFileName,
    digest: SnapshotDigest,
}

impl SnapshotReference {
    pub(super) fn from_stored(
        scan_id: &ScanId,
        version: u32,
        file_name: &str,
        digest: [u8; 32],
    ) -> Result<Self, SnapshotRepositoryError> {
        let version = NonZeroU32::new(version)
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch))?;
        let file_name = SnapshotFileName::parse(file_name).map_err(map_storage)?;
        if file_name != SnapshotFileName::from_scan_id(scan_id.as_str().as_bytes()) {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        Ok(Self {
            scan_id: scan_id.clone(),
            version,
            file_name,
            digest: SnapshotDigest::from_bytes(digest),
        })
    }

    pub(crate) const fn version(&self) -> u32 {
        self.version.get()
    }

    pub(crate) fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    pub(crate) fn file_name(&self) -> &SnapshotFileName {
        &self.file_name
    }

    pub(crate) const fn digest(&self) -> SnapshotDigest {
        self.digest
    }
}

/// Validated immutable publication retained across the following DB commit.
pub(crate) struct PublishedSnapshot {
    reference: SnapshotReference,
    publication: SnapshotPublicationLease,
    temp_lease: PreparedSnapshotTempLease,
}

impl PublishedSnapshot {
    pub(crate) fn reference(&self) -> &SnapshotReference {
        &self.reference
    }

    pub(crate) fn revalidate(&self) -> Result<(), SnapshotRepositoryError> {
        self.publication.revalidate().map_err(map_storage)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotRepositoryErrorKind {
    ReadOnly,
    MissingStore,
    MissingSnapshot,
    SnapshotUnavailable,
    IncompatibleVersion,
    ReferenceMismatch,
    #[allow(
        dead_code,
        reason = "review leases are wired to Explorer/FFI in a later milestone slice"
    )]
    ReviewLeaseExpired,
    Codec(SnapshotCodecErrorKind),
    Storage(SnapshotStorageErrorKind),
    History(HistoryErrorKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("snapshot repository operation failed: {kind:?}")]
pub(crate) struct SnapshotRepositoryError {
    pub(crate) kind: SnapshotRepositoryErrorKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotStorageClearErrorKind {
    ReadOnlyStore,
    ChangedSincePreview,
    Busy,
    IncompatibleSchema,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    OutcomeUnknown,
    Unavailable,
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("snapshot-storage clear operation failed: {kind:?}")]
pub(crate) struct SnapshotStorageClearError {
    pub(crate) kind: SnapshotStorageClearErrorKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SnapshotStorageClearFinalState {
    Eligible,
    Protected,
    TombstonedResidual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SnapshotStorageClearFinalWitness {
    reference: SnapshotReference,
    completed_at: SystemTime,
    usage: SnapshotFileUsage,
    state: SnapshotStorageClearFinalState,
    latest_rank: Option<u8>,
    active_pins: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SnapshotStorageClearOrphanWitness {
    file_name: SnapshotFileName,
    usage: SnapshotFileUsage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SnapshotStorageClearTemporaryWitness {
    name: String,
    usage: SnapshotFileUsage,
    state: SnapshotTemporaryState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SnapshotStorageClearResidualLeaseWitness {
    scan_id: ScanId,
    temp_name: String,
}

/// Path-free, bounded proof of one exact stable snapshot-store population.
///
/// The witness deliberately contains no scan root. It can only be consumed by
/// the repository that rebuilds and exactly compares the complete relevant
/// final and maintenance populations under database-before-snapshot locks.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedSnapshotStorageClear {
    finals: Vec<SnapshotStorageClearFinalWitness>,
    orphans: Vec<SnapshotStorageClearOrphanWitness>,
    temporaries: Vec<SnapshotStorageClearTemporaryWitness>,
    residual_temp_leases: Vec<SnapshotStorageClearResidualLeaseWitness>,
    controls: SnapshotRetentionUsage,
    clearable: OwnedStorageUsage,
    protected: OwnedStorageUsage,
    excluded_maintenance: OwnedStorageUsage,
    eligible_snapshot_count: u32,
    tombstoned_residual_count: u32,
    protected_snapshot_count: u32,
    active_review_count: u32,
    excluded_maintenance_object_count: u32,
}

impl PreparedSnapshotStorageClear {
    pub(crate) const fn eligible_snapshot_count(&self) -> u32 {
        self.eligible_snapshot_count
    }

    pub(crate) const fn tombstoned_residual_count(&self) -> u32 {
        self.tombstoned_residual_count
    }

    pub(crate) fn clearable_count(&self) -> Option<u32> {
        self.eligible_snapshot_count
            .checked_add(self.tombstoned_residual_count)
    }

    pub(crate) const fn clearable(&self) -> OwnedStorageUsage {
        self.clearable
    }

    pub(crate) const fn protected_snapshot_count(&self) -> u32 {
        self.protected_snapshot_count
    }

    pub(crate) const fn protected(&self) -> OwnedStorageUsage {
        self.protected
    }

    pub(crate) const fn active_review_count(&self) -> u32 {
        self.active_review_count
    }

    pub(crate) const fn excluded_maintenance_object_count(&self) -> u32 {
        self.excluded_maintenance_object_count
    }

    pub(crate) const fn excluded_maintenance(&self) -> OwnedStorageUsage {
        self.excluded_maintenance
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotStorageClearResult {
    pub(crate) cleared_eligible_snapshot_count: u32,
    pub(crate) cleared_tombstoned_residual_count: u32,
    pub(crate) cleared_usage: OwnedStorageUsage,
}

impl SnapshotStorageClearResult {
    pub(crate) fn cleared_count(self) -> Option<u32> {
        self.cleared_eligible_snapshot_count
            .checked_add(self.cleared_tombstoned_residual_count)
    }
}

/// One bounded production-retention decision. A batch removes at most one
/// exact final so callers can yield between potentially slow filesystem
/// durability operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotRetentionBatchResult {
    pub(crate) observed_at: SystemTime,
    pub(crate) outcome: SnapshotRetentionBatchOutcome,
    pub(crate) cap_bytes: u64,
    pub(crate) charged_bytes_before: u64,
    pub(crate) charged_bytes_after: u64,
    pub(crate) has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotRetentionBatchOutcome {
    UnderCap,
    DeferredUnstable,
    DeferredNoEligibleSnapshot,
    RemovedTombstonedResidual { scan_id: ScanId, bytes: u64 },
    TombstonedAndRemoved { scan_id: ScanId, bytes: u64 },
}

/// One bounded physical-orphan reconciliation. The repository removes at
/// most one fully decoded final per call and never changes scan history,
/// temporary-lease state, pins, or retention tombstones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotOrphanReconciliationBatchResult {
    pub(crate) observed_at: SystemTime,
    pub(crate) outcome: SnapshotOrphanReconciliationBatchOutcome,
    pub(crate) orphan_count_before: u32,
    pub(crate) orphan_count_after: u32,
    pub(crate) orphan_charged_bytes_before: u64,
    pub(crate) orphan_charged_bytes_after: u64,
    pub(crate) has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotOrphanReconciliationBatchOutcome {
    NoOrphan,
    Removed { scan_id: ScanId, bytes: u64 },
}

/// One bounded reconciliation of immutable terminal-parent snapshot-temp
/// debt. Exact scan and temporary-file identities never leave persistence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotTerminalTempReconciliationBatchResult {
    pub(crate) observed_at: SystemTime,
    pub(crate) outcome: SnapshotTerminalTempReconciliationBatchOutcome,
    pub(crate) terminal_lease_count_before: u32,
    pub(crate) terminal_lease_count_after: u32,
    pub(crate) active_terminal_lease_count_before: u32,
    pub(crate) active_terminal_lease_count_after: u32,
    pub(crate) terminal_charged_bytes_before: u64,
    pub(crate) terminal_charged_bytes_after: u64,
    pub(crate) has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotTerminalTempReconciliationBatchOutcome {
    NoTerminalResidual,
    DeferredActive,
    ReconciledRowOnly { scan_id: ScanId },
    RemovedTempAndLease { scan_id: ScanId, bytes: u64 },
}

/// One bounded reconciliation of a recognized snapshot temp that has no exact
/// row in the complete immutable lease population. The private physical name
/// never leaves persistence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotUnleasedTempReconciliationBatchResult {
    pub(crate) observed_at: SystemTime,
    pub(crate) outcome: SnapshotUnleasedTempReconciliationBatchOutcome,
    pub(crate) unleased_temp_count_before: u32,
    pub(crate) unleased_temp_count_after: u32,
    pub(crate) active_unleased_temp_count_before: u32,
    pub(crate) active_unleased_temp_count_after: u32,
    pub(crate) unleased_charged_bytes_before: u64,
    pub(crate) unleased_charged_bytes_after: u64,
    pub(crate) has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotUnleasedTempReconciliationBatchOutcome {
    NoUnleasedTemp,
    DeferredActive,
    Removed { bytes: u64 },
}

/// One bounded reconciliation of root-local snapshot provisioning debt. The
/// root-local stage identity and physical name never leave persistence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotProvisioningStageReconciliationBatchResult {
    pub(crate) observed_at: SystemTime,
    pub(crate) outcome: SnapshotProvisioningStageReconciliationBatchOutcome,
    pub(crate) total_stage_count_before: u64,
    pub(crate) total_stage_count_after: u64,
    pub(crate) marker_owned_count_before: u64,
    pub(crate) marker_owned_count_after: u64,
    pub(crate) unproven_count_before: u64,
    pub(crate) unproven_count_after: u64,
    pub(crate) control_charged_bytes_before: u64,
    pub(crate) control_charged_bytes_after: u64,
    pub(crate) has_more: bool,
}

/// Path-free result of one provisioning-stage maintenance batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotProvisioningStageReconciliationBatchOutcome {
    NoStage,
    DeferredUnproven,
    RemovedMarkerOnly { bytes: u64 },
    RemovedMarkerComplete { bytes: u64 },
}

const fn repository_error(kind: SnapshotRepositoryErrorKind) -> SnapshotRepositoryError {
    SnapshotRepositoryError { kind }
}

fn map_codec(error: SnapshotCodecError) -> SnapshotRepositoryError {
    repository_error(SnapshotRepositoryErrorKind::Codec(error.kind))
}

fn map_storage(error: SnapshotStorageError) -> SnapshotRepositoryError {
    repository_error(SnapshotRepositoryErrorKind::Storage(error.kind()))
}

fn charged_bytes_after_removal(
    charged_bytes_before: u64,
    removed_bytes: u64,
) -> Result<u64, SnapshotRepositoryError> {
    charged_bytes_before
        .checked_sub(removed_bytes)
        .ok_or_else(|| {
            repository_error(SnapshotRepositoryErrorKind::Storage(
                SnapshotStorageErrorKind::InternalState,
            ))
        })
}

impl SnapshotRepositoryError {
    pub(crate) const fn open_kind(self) -> SnapshotOpenErrorKind {
        match self.kind {
            SnapshotRepositoryErrorKind::Storage(kind) => match kind {
                SnapshotStorageErrorKind::InvalidConfiguration => {
                    SnapshotOpenErrorKind::InvalidConfiguration
                }
                SnapshotStorageErrorKind::UnsafeRoot => SnapshotOpenErrorKind::UnsafeRoot,
                SnapshotStorageErrorKind::UnsafeObject => SnapshotOpenErrorKind::UnsafeObject,
                SnapshotStorageErrorKind::UnrecognizedStore => {
                    SnapshotOpenErrorKind::UnrecognizedStore
                }
                SnapshotStorageErrorKind::Unavailable => SnapshotOpenErrorKind::Unavailable,
                SnapshotStorageErrorKind::Busy => SnapshotOpenErrorKind::Busy,
                SnapshotStorageErrorKind::InternalState => SnapshotOpenErrorKind::InternalState,
            },
            _ => SnapshotOpenErrorKind::InternalState,
        }
    }
}

fn map_history(error: HistoryError) -> SnapshotRepositoryError {
    repository_error(SnapshotRepositoryErrorKind::History(error.kind))
}

const fn history_repository_error(kind: HistoryErrorKind) -> SnapshotRepositoryError {
    repository_error(SnapshotRepositoryErrorKind::History(kind))
}

fn owned_snapshot_storage_footprint(
    inventory: &SnapshotRetentionInventory,
) -> Result<OwnedSnapshotStorageFootprint, SnapshotRepositoryError> {
    let usage = |value: SnapshotRetentionUsage| OwnedStorageUsage {
        logical_bytes: value.logical_bytes,
        allocated_bytes: value.allocated_bytes,
        charged_bytes: value.charged_bytes,
    };
    let controls = usage(inventory.totals.controls);
    let available = usage(inventory.totals.available);
    let protected = usage(inventory.totals.protected);
    let retention_eligible = usage(inventory.totals.eligible);
    let tombstoned_residual = usage(inventory.totals.tombstoned_residual);
    let orphan = usage(inventory.totals.orphan);
    let temporary_active = usage(inventory.totals.temporary_active);
    let temporary_quiescent = usage(inventory.totals.temporary_quiescent);
    let temporary_unleased = usage(inventory.totals.temporary_unleased);
    let total = usage(inventory.totals.store_total);

    let mut available_count = 0_u32;
    let mut protected_count = 0_u32;
    let mut retention_eligible_count = 0_u32;
    let mut tombstoned_residual_count = 0_u32;
    for entry in &inventory.entries {
        match entry.logical_state {
            SnapshotRetentionLogicalState::Available => {
                available_count = checked_footprint_count_increment(available_count)?;
                if entry.is_policy_protected() {
                    protected_count = checked_footprint_count_increment(protected_count)?;
                } else {
                    retention_eligible_count =
                        checked_footprint_count_increment(retention_eligible_count)?;
                }
            }
            SnapshotRetentionLogicalState::Tombstoned { .. } => {
                tombstoned_residual_count =
                    checked_footprint_count_increment(tombstoned_residual_count)?;
            }
        }
    }
    let orphan_count = checked_footprint_count(inventory.orphan_finals.len())?;
    let mut active_temporary_count = 0_u32;
    let mut quiescent_temporary_count = 0_u32;
    let mut unleased_temporary_count = 0_u32;
    for temporary in &inventory.temporary_files {
        match temporary.state {
            SnapshotTemporaryState::Active => {
                active_temporary_count = checked_footprint_count_increment(active_temporary_count)?;
            }
            SnapshotTemporaryState::QuiescentAtObservation => {
                quiescent_temporary_count =
                    checked_footprint_count_increment(quiescent_temporary_count)?;
            }
            SnapshotTemporaryState::Unleased => {
                unleased_temporary_count =
                    checked_footprint_count_increment(unleased_temporary_count)?;
            }
        }
    }
    let residual_temporary_lease_count =
        checked_footprint_count(inventory.residual_temp_leases.len())?;

    let classified_available = protected
        .checked_add(retention_eligible)
        .map_err(map_history)?;
    let classified_total = [
        controls,
        available,
        tombstoned_residual,
        orphan,
        temporary_active,
        temporary_quiescent,
        temporary_unleased,
    ]
    .into_iter()
    .try_fold(OwnedStorageUsage::default(), |sum, value| {
        sum.checked_add(value)
    })
    .map_err(map_history)?;
    let classified_available_count = protected_count
        .checked_add(retention_eligible_count)
        .ok_or_else(|| history_repository_error(HistoryErrorKind::CorruptData))?;
    let accounting_unstable = active_temporary_count > 0 || unleased_temporary_count > 0;
    let non_evictable_charged_bytes = [
        controls,
        protected,
        tombstoned_residual,
        orphan,
        temporary_active,
        temporary_quiescent,
        temporary_unleased,
    ]
    .into_iter()
    .try_fold(0_u64, |sum, value| {
        sum.checked_add(value.charged_bytes)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))
    })
    .map_err(map_history)?;
    if classified_available != available
        || classified_total != total
        || classified_available_count != available_count
        || inventory.cap_excess_bytes != total.charged_bytes.saturating_sub(inventory.cap_bytes)
        || inventory.accounting_unstable != accounting_unstable
        || inventory.non_evictable_over_cap != (non_evictable_charged_bytes > inventory.cap_bytes)
    {
        return Err(history_repository_error(HistoryErrorKind::CorruptData));
    }

    Ok(OwnedSnapshotStorageFootprint {
        cap_bytes: inventory.cap_bytes,
        cap_excess_bytes: inventory.cap_excess_bytes,
        controls,
        available,
        protected,
        retention_eligible,
        tombstoned_residual,
        orphan,
        temporary_active,
        temporary_quiescent,
        temporary_unleased,
        total,
        available_count,
        protected_count,
        retention_eligible_count,
        tombstoned_residual_count,
        orphan_count,
        active_temporary_count,
        quiescent_temporary_count,
        unleased_temporary_count,
        residual_temporary_lease_count,
        active_pin_rows: inventory.totals.active_pin_rows,
        expired_pin_rows: inventory.totals.expired_pin_rows,
        non_evictable_over_cap: inventory.non_evictable_over_cap,
        accounting_unstable: inventory.accounting_unstable,
    })
}

fn checked_footprint_count(value: usize) -> Result<u32, SnapshotRepositoryError> {
    u32::try_from(value).map_err(|_| history_repository_error(HistoryErrorKind::QueryLimitExceeded))
}

fn checked_footprint_count_increment(value: u32) -> Result<u32, SnapshotRepositoryError> {
    value
        .checked_add(1)
        .ok_or_else(|| history_repository_error(HistoryErrorKind::QueryLimitExceeded))
}

const fn snapshot_storage_clear_error(
    kind: SnapshotStorageClearErrorKind,
) -> SnapshotStorageClearError {
    SnapshotStorageClearError { kind }
}

const fn map_snapshot_storage_clear_repository_error(
    error: SnapshotRepositoryError,
) -> SnapshotStorageClearError {
    let kind = match error.kind {
        SnapshotRepositoryErrorKind::ReadOnly => SnapshotStorageClearErrorKind::ReadOnlyStore,
        SnapshotRepositoryErrorKind::MissingStore
        | SnapshotRepositoryErrorKind::MissingSnapshot => {
            SnapshotStorageClearErrorKind::Unavailable
        }
        SnapshotRepositoryErrorKind::SnapshotUnavailable
        | SnapshotRepositoryErrorKind::ReferenceMismatch
        | SnapshotRepositoryErrorKind::ReviewLeaseExpired => {
            SnapshotStorageClearErrorKind::CorruptData
        }
        SnapshotRepositoryErrorKind::IncompatibleVersion => {
            SnapshotStorageClearErrorKind::IncompatibleSchema
        }
        SnapshotRepositoryErrorKind::Codec(kind) => match kind {
            SnapshotCodecErrorKind::LimitExceeded => SnapshotStorageClearErrorKind::BudgetExceeded,
            SnapshotCodecErrorKind::Io => SnapshotStorageClearErrorKind::Unavailable,
            SnapshotCodecErrorKind::IncompatibleVersion => {
                SnapshotStorageClearErrorKind::IncompatibleSchema
            }
            SnapshotCodecErrorKind::InvalidInput
            | SnapshotCodecErrorKind::InvalidMagic
            | SnapshotCodecErrorKind::InvalidLength
            | SnapshotCodecErrorKind::ChecksumMismatch
            | SnapshotCodecErrorKind::CorruptData => SnapshotStorageClearErrorKind::CorruptData,
        },
        SnapshotRepositoryErrorKind::Storage(kind) => match kind {
            SnapshotStorageErrorKind::UnsafeRoot
            | SnapshotStorageErrorKind::UnsafeObject
            | SnapshotStorageErrorKind::UnrecognizedStore => {
                SnapshotStorageClearErrorKind::UnsafeStorage
            }
            SnapshotStorageErrorKind::Busy => SnapshotStorageClearErrorKind::Busy,
            SnapshotStorageErrorKind::Unavailable => SnapshotStorageClearErrorKind::Unavailable,
            SnapshotStorageErrorKind::InvalidConfiguration
            | SnapshotStorageErrorKind::InternalState => {
                SnapshotStorageClearErrorKind::InternalState
            }
        },
        SnapshotRepositoryErrorKind::History(kind) => match kind {
            HistoryErrorKind::IncompatibleSchema => {
                SnapshotStorageClearErrorKind::IncompatibleSchema
            }
            HistoryErrorKind::QueryLimitExceeded => SnapshotStorageClearErrorKind::BudgetExceeded,
            HistoryErrorKind::Busy => SnapshotStorageClearErrorKind::Busy,
            HistoryErrorKind::UnsafeStorage => SnapshotStorageClearErrorKind::UnsafeStorage,
            HistoryErrorKind::CorruptData => SnapshotStorageClearErrorKind::CorruptData,
            HistoryErrorKind::DatabaseUnavailable => SnapshotStorageClearErrorKind::Unavailable,
            HistoryErrorKind::OutcomeUnknown => SnapshotStorageClearErrorKind::OutcomeUnknown,
            HistoryErrorKind::InvalidInput
            | HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::NotFound
            | HistoryErrorKind::InvalidTransition
            | HistoryErrorKind::InternalState => SnapshotStorageClearErrorKind::InternalState,
        },
    };
    snapshot_storage_clear_error(kind)
}

fn snapshot_storage_clear_usage(usage: SnapshotRetentionUsage) -> OwnedStorageUsage {
    OwnedStorageUsage {
        logical_bytes: usage.logical_bytes,
        allocated_bytes: usage.allocated_bytes,
        charged_bytes: usage.charged_bytes,
    }
}

fn checked_snapshot_storage_clear_usage_add(
    left: SnapshotRetentionUsage,
    right: SnapshotRetentionUsage,
) -> Result<SnapshotRetentionUsage, SnapshotStorageClearError> {
    Ok(SnapshotRetentionUsage {
        logical_bytes: left
            .logical_bytes
            .checked_add(right.logical_bytes)
            .ok_or_else(|| {
                snapshot_storage_clear_error(SnapshotStorageClearErrorKind::CorruptData)
            })?,
        allocated_bytes: left
            .allocated_bytes
            .checked_add(right.allocated_bytes)
            .ok_or_else(|| {
                snapshot_storage_clear_error(SnapshotStorageClearErrorKind::CorruptData)
            })?,
        charged_bytes: left
            .charged_bytes
            .checked_add(right.charged_bytes)
            .ok_or_else(|| {
                snapshot_storage_clear_error(SnapshotStorageClearErrorKind::CorruptData)
            })?,
    })
}

fn checked_owned_storage_usage_add_file(
    left: OwnedStorageUsage,
    right: SnapshotFileUsage,
) -> Option<OwnedStorageUsage> {
    Some(OwnedStorageUsage {
        logical_bytes: left.logical_bytes.checked_add(right.logical_bytes())?,
        allocated_bytes: left.allocated_bytes.checked_add(right.allocated_bytes())?,
        charged_bytes: left.charged_bytes.checked_add(right.charged_bytes())?,
    })
}

fn prepared_snapshot_storage_clear(
    inventory: &SnapshotRetentionInventory,
) -> Result<PreparedSnapshotStorageClear, SnapshotStorageClearError> {
    let mut finals = Vec::new();
    finals
        .try_reserve_exact(inventory.entries.len())
        .map_err(|_| snapshot_storage_clear_error(SnapshotStorageClearErrorKind::BudgetExceeded))?;
    let mut eligible_snapshot_count = 0_u32;
    let mut tombstoned_residual_count = 0_u32;
    let mut protected_snapshot_count = 0_u32;
    for entry in &inventory.entries {
        let state = match entry.logical_state {
            SnapshotRetentionLogicalState::Available if entry.is_policy_protected() => {
                protected_snapshot_count =
                    protected_snapshot_count.checked_add(1).ok_or_else(|| {
                        snapshot_storage_clear_error(SnapshotStorageClearErrorKind::BudgetExceeded)
                    })?;
                SnapshotStorageClearFinalState::Protected
            }
            SnapshotRetentionLogicalState::Available => {
                if !entry.is_eviction_observation() {
                    return Err(snapshot_storage_clear_error(
                        SnapshotStorageClearErrorKind::InternalState,
                    ));
                }
                eligible_snapshot_count =
                    eligible_snapshot_count.checked_add(1).ok_or_else(|| {
                        snapshot_storage_clear_error(SnapshotStorageClearErrorKind::BudgetExceeded)
                    })?;
                SnapshotStorageClearFinalState::Eligible
            }
            SnapshotRetentionLogicalState::Tombstoned { .. } => {
                tombstoned_residual_count =
                    tombstoned_residual_count.checked_add(1).ok_or_else(|| {
                        snapshot_storage_clear_error(SnapshotStorageClearErrorKind::BudgetExceeded)
                    })?;
                SnapshotStorageClearFinalState::TombstonedResidual
            }
        };
        finals.push(SnapshotStorageClearFinalWitness {
            reference: entry.reference.clone(),
            completed_at: entry.completed_at,
            usage: entry.usage,
            state,
            latest_rank: entry.latest_rank,
            active_pins: entry.pins.active,
        });
    }
    finals.sort_by(|left, right| left.reference.scan_id().cmp(right.reference.scan_id()));

    let mut orphans = Vec::new();
    orphans
        .try_reserve_exact(inventory.orphan_finals.len())
        .map_err(|_| snapshot_storage_clear_error(SnapshotStorageClearErrorKind::BudgetExceeded))?;
    orphans.extend(inventory.orphan_finals.iter().map(|orphan| {
        SnapshotStorageClearOrphanWitness {
            file_name: orphan.file_name.clone(),
            usage: orphan.usage,
        }
    }));
    orphans.sort_by(|left, right| left.file_name.cmp(&right.file_name));

    let mut temporaries = Vec::new();
    temporaries
        .try_reserve_exact(inventory.temporary_files.len())
        .map_err(|_| snapshot_storage_clear_error(SnapshotStorageClearErrorKind::BudgetExceeded))?;
    temporaries.extend(inventory.temporary_files.iter().map(|temporary| {
        SnapshotStorageClearTemporaryWitness {
            name: temporary.temp_name.clone(),
            usage: temporary.usage,
            state: temporary.state,
        }
    }));
    temporaries.sort_by(|left, right| left.name.cmp(&right.name));

    let mut residual_temp_leases = Vec::new();
    residual_temp_leases
        .try_reserve_exact(inventory.residual_temp_leases.len())
        .map_err(|_| snapshot_storage_clear_error(SnapshotStorageClearErrorKind::BudgetExceeded))?;
    residual_temp_leases.extend(inventory.residual_temp_leases.iter().map(|lease| {
        SnapshotStorageClearResidualLeaseWitness {
            scan_id: lease.scan_id.clone(),
            temp_name: lease.temp_name.clone(),
        }
    }));
    residual_temp_leases.sort_by(|left, right| {
        left.scan_id
            .cmp(&right.scan_id)
            .then_with(|| left.temp_name.cmp(&right.temp_name))
    });

    let clearable = checked_snapshot_storage_clear_usage_add(
        inventory.totals.eligible,
        inventory.totals.tombstoned_residual,
    )?;
    let physical_temporary = [
        inventory.totals.temporary_active,
        inventory.totals.temporary_quiescent,
        inventory.totals.temporary_unleased,
    ]
    .into_iter()
    .try_fold(SnapshotRetentionUsage::default(), |total, usage| {
        checked_snapshot_storage_clear_usage_add(total, usage)
    })?;
    let excluded_maintenance =
        checked_snapshot_storage_clear_usage_add(inventory.totals.orphan, physical_temporary)?;
    let excluded_maintenance_object_count = u32::try_from(
        inventory
            .orphan_finals
            .len()
            .checked_add(inventory.temporary_files.len())
            .ok_or_else(|| {
                snapshot_storage_clear_error(SnapshotStorageClearErrorKind::BudgetExceeded)
            })?,
    )
    .map_err(|_| snapshot_storage_clear_error(SnapshotStorageClearErrorKind::BudgetExceeded))?;

    Ok(PreparedSnapshotStorageClear {
        finals,
        orphans,
        temporaries,
        residual_temp_leases,
        controls: inventory.totals.controls,
        clearable: snapshot_storage_clear_usage(clearable),
        protected: snapshot_storage_clear_usage(inventory.totals.protected),
        excluded_maintenance: snapshot_storage_clear_usage(excluded_maintenance),
        eligible_snapshot_count,
        tombstoned_residual_count,
        protected_snapshot_count,
        active_review_count: inventory.totals.active_pin_rows,
        excluded_maintenance_object_count,
    })
}

#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
fn map_review_history(error: HistoryError) -> SnapshotRepositoryError {
    match error.kind {
        HistoryErrorKind::InvalidTransition => {
            repository_error(SnapshotRepositoryErrorKind::ReviewLeaseExpired)
        }
        HistoryErrorKind::NotFound => {
            repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch)
        }
        _ => map_history(error),
    }
}

fn map_process_identity(error: ProcessIdentityError) -> SnapshotRepositoryError {
    let kind = match error {
        ProcessIdentityError::InvalidEncoding => HistoryErrorKind::CorruptData,
        ProcessIdentityError::ObservationUnavailable | ProcessIdentityError::RandomUnavailable => {
            HistoryErrorKind::InternalState
        }
    };
    repository_error(SnapshotRepositoryErrorKind::History(kind))
}

/// Coordinates bounded codec validation with immutable private publication.
pub(crate) struct SnapshotRepository {
    database: Arc<StoreCoordinator>,
    store: Option<SecureSnapshotStore>,
    access: SnapshotStoreAccess,
    #[allow(
        dead_code,
        reason = "review leases are wired to Explorer/FFI in a later milestone slice"
    )]
    review: Option<SnapshotReviewContext>,
}

/// Callback-scoped snapshot-writer admission for app-data reset.
///
/// The owned inventory lease remains local to the repository method that
/// invokes the higher-ranked callback. This wrapper borrows it so neither the
/// writer exclusion nor the complete physical observation can escape.
pub(crate) struct AppDataResetSnapshotAdmission<'scope> {
    inventory: &'scope SnapshotStoreInventoryLease,
    deadline: Instant,
}

impl AppDataResetSnapshotAdmission<'_> {
    pub(crate) fn revalidate(&self) -> Result<(), SnapshotRepositoryError> {
        self.inventory
            .revalidate_complete_for_app_data_reset_until(self.deadline)
            .map_err(map_storage)
    }
}

#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
struct SnapshotReviewContext {
    owner: OnceLock<ProcessInstanceId>,
    active_slots: Arc<AtomicUsize>,
    decoded_slots: Arc<AtomicUsize>,
    decoded_bytes: Arc<AtomicU64>,
}

const MAX_DECODED_REVIEW_DOCUMENTS_PER_OWNER: usize = 2;
const MAX_ESTIMATED_DECODED_REVIEW_BYTES_PER_OWNER: u64 = 1_024 * 1_024 * 1_024;
const DECODED_REVIEW_WIRE_MULTIPLIER: u64 = 3;

#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
struct SnapshotReviewSlot {
    active_slots: Arc<AtomicUsize>,
}

struct SnapshotReviewDecodedSlot {
    decoded_slots: Arc<AtomicUsize>,
    decoded_bytes: Arc<AtomicU64>,
    charged_bytes: u64,
}

pub(crate) struct SnapshotReviewDocument {
    document: SnapshotDocument,
    child_offsets: Vec<u32>,
    child_indices: Vec<u32>,
    _slot: SnapshotReviewDecodedSlot,
}

impl std::ops::Deref for SnapshotReviewDocument {
    type Target = SnapshotDocument;

    fn deref(&self) -> &Self::Target {
        &self.document
    }
}

impl SnapshotReviewDocument {
    /// Build the bounded child index needed by deterministic candidate replay
    /// without creating a user-facing review lease. Recovery owns no paths or
    /// action capability; it only replays immutable snapshot bytes.
    pub(crate) fn from_document_for_recovery(
        document: SnapshotDocument,
    ) -> Result<Self, SnapshotRepositoryError> {
        validate_snapshot_document(&document)
            .map_err(|error| repository_error(SnapshotRepositoryErrorKind::Codec(error.kind)))?;
        let decoded_slots = Arc::new(AtomicUsize::new(0));
        let decoded_bytes = Arc::new(AtomicU64::new(0));
        let slot = SnapshotReviewDecodedSlot::reserve(&decoded_slots, &decoded_bytes, 0)?;
        let (child_offsets, child_indices) = build_review_child_index(&document)?;
        Ok(Self {
            document,
            child_offsets,
            child_indices,
            _slot: slot,
        })
    }

    pub(crate) fn direct_child_indices(
        &self,
        parent_index: usize,
    ) -> Result<&[u32], SnapshotRepositoryError> {
        let start = *self.child_offsets.get(parent_index).ok_or_else(|| {
            repository_error(SnapshotRepositoryErrorKind::Codec(
                SnapshotCodecErrorKind::CorruptData,
            ))
        })?;
        let end = *self.child_offsets.get(parent_index + 1).ok_or_else(|| {
            repository_error(SnapshotRepositoryErrorKind::Codec(
                SnapshotCodecErrorKind::CorruptData,
            ))
        })?;
        self.child_indices
            .get(start as usize..end as usize)
            .ok_or_else(|| {
                repository_error(SnapshotRepositoryErrorKind::Codec(
                    SnapshotCodecErrorKind::CorruptData,
                ))
            })
    }

    #[cfg(test)]
    pub(crate) fn from_document_for_test(
        document: SnapshotDocument,
    ) -> Result<Self, SnapshotRepositoryError> {
        validate_snapshot_document(&document)
            .map_err(|error| repository_error(SnapshotRepositoryErrorKind::Codec(error.kind)))?;
        let decoded_slots = Arc::new(AtomicUsize::new(0));
        let decoded_bytes = Arc::new(AtomicU64::new(0));
        let slot = SnapshotReviewDecodedSlot::reserve(&decoded_slots, &decoded_bytes, 0)?;
        let (child_offsets, child_indices) = build_review_child_index(&document)?;
        Ok(Self {
            document,
            child_offsets,
            child_indices,
            _slot: slot,
        })
    }
}

#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
impl SnapshotReviewContext {
    fn owner(&self) -> Result<ProcessInstanceId, SnapshotRepositoryError> {
        if let Some(owner) = self.owner.get() {
            return Ok(owner.clone());
        }
        let candidate = current_process_instance().map_err(map_process_identity)?;
        let _ = self.owner.set(candidate);
        self.owner.get().cloned().ok_or_else(|| {
            repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::InternalState,
            ))
        })
    }

    fn reserve(&self) -> Result<SnapshotReviewSlot, SnapshotRepositoryError> {
        self.active_slots
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_ACTIVE_PINS_PER_OWNER).then_some(active + 1)
            })
            .map_err(|_| {
                repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::QueryLimitExceeded,
                ))
            })?;
        Ok(SnapshotReviewSlot {
            active_slots: Arc::clone(&self.active_slots),
        })
    }
}

impl Drop for SnapshotReviewSlot {
    fn drop(&mut self) {
        let previous = self.active_slots.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "snapshot review slot underflow");
    }
}

impl SnapshotReviewDecodedSlot {
    fn reserve(
        decoded_slots: &Arc<AtomicUsize>,
        decoded_bytes: &Arc<AtomicU64>,
        wire_bytes: u64,
    ) -> Result<Self, SnapshotRepositoryError> {
        let charged_bytes = wire_bytes
            .checked_mul(DECODED_REVIEW_WIRE_MULTIPLIER)
            .ok_or_else(review_budget_exceeded)?;
        if charged_bytes > MAX_ESTIMATED_DECODED_REVIEW_BYTES_PER_OWNER {
            return Err(review_budget_exceeded());
        }
        decoded_slots
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_DECODED_REVIEW_DOCUMENTS_PER_OWNER).then_some(active + 1)
            })
            .map_err(|_| review_budget_exceeded())?;
        if decoded_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                active
                    .checked_add(charged_bytes)
                    .filter(|total| *total <= MAX_ESTIMATED_DECODED_REVIEW_BYTES_PER_OWNER)
            })
            .is_err()
        {
            let previous = decoded_slots.fetch_sub(1, Ordering::AcqRel);
            debug_assert!(previous > 0, "decoded snapshot review slot underflow");
            return Err(review_budget_exceeded());
        }
        Ok(Self {
            decoded_slots: Arc::clone(decoded_slots),
            decoded_bytes: Arc::clone(decoded_bytes),
            charged_bytes,
        })
    }
}

fn review_budget_exceeded() -> SnapshotRepositoryError {
    repository_error(SnapshotRepositoryErrorKind::History(
        HistoryErrorKind::QueryLimitExceeded,
    ))
}

impl Drop for SnapshotReviewDecodedSlot {
    fn drop(&mut self) {
        let previous_bytes = self
            .decoded_bytes
            .fetch_sub(self.charged_bytes, Ordering::AcqRel);
        debug_assert!(
            previous_bytes >= self.charged_bytes,
            "decoded snapshot byte underflow"
        );
        let previous = self.decoded_slots.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "decoded snapshot review slot underflow");
    }
}

fn build_review_child_index(
    document: &SnapshotDocument,
) -> Result<(Vec<u32>, Vec<u32>), SnapshotRepositoryError> {
    let node_count = document.nodes.len();
    let mut offsets = Vec::new();
    offsets
        .try_reserve_exact(node_count.saturating_add(1))
        .map_err(|_| review_budget_exceeded())?;
    offsets.push(0_u32);
    for node in &document.nodes {
        let next = offsets
            .last()
            .copied()
            .and_then(|offset| {
                u32::try_from(node.child_count)
                    .ok()
                    .and_then(|children| offset.checked_add(children))
            })
            .ok_or_else(review_budget_exceeded)?;
        offsets.push(next);
    }
    let total_children =
        usize::try_from(*offsets.last().unwrap_or(&0)).map_err(|_| review_budget_exceeded())?;
    if total_children != node_count.saturating_sub(1) {
        return Err(repository_error(SnapshotRepositoryErrorKind::Codec(
            SnapshotCodecErrorKind::CorruptData,
        )));
    }

    let mut indices = Vec::new();
    indices
        .try_reserve_exact(total_children)
        .map_err(|_| review_budget_exceeded())?;
    indices.resize(total_children, 0_u32);
    let mut cursors = Vec::new();
    cursors
        .try_reserve_exact(node_count)
        .map_err(|_| review_budget_exceeded())?;
    cursors.extend_from_slice(&offsets[..node_count]);
    for (index, node) in document.nodes.iter().enumerate().skip(1) {
        let parent = usize::try_from(node.parent.ok_or_else(|| {
            repository_error(SnapshotRepositoryErrorKind::Codec(
                SnapshotCodecErrorKind::CorruptData,
            ))
        })?)
        .map_err(|_| review_budget_exceeded())?;
        let cursor = cursors.get_mut(parent).ok_or_else(|| {
            repository_error(SnapshotRepositoryErrorKind::Codec(
                SnapshotCodecErrorKind::CorruptData,
            ))
        })?;
        let destination = indices.get_mut(*cursor as usize).ok_or_else(|| {
            repository_error(SnapshotRepositoryErrorKind::Codec(
                SnapshotCodecErrorKind::CorruptData,
            ))
        })?;
        *destination = u32::try_from(index).map_err(|_| review_budget_exceeded())?;
        *cursor = cursor.checked_add(1).ok_or_else(review_budget_exceeded)?;
    }
    if cursors
        .iter()
        .zip(offsets.iter().skip(1))
        .any(|(cursor, end)| cursor != end)
    {
        return Err(repository_error(SnapshotRepositoryErrorKind::Codec(
            SnapshotCodecErrorKind::CorruptData,
        )));
    }
    Ok((offsets, indices))
}

/// One exact, expiring snapshot review pin plus a retained immutable handle.
///
/// The lease is intentionally non-cloneable and not `Sync`. Dropping it never
/// enters SQLite; an explicit release removes the durable row, while crashes
/// and implicit drops remain conservatively pinned until the fixed expiry.
#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
#[must_use = "keep the lease alive during review or explicitly release it"]
pub(crate) struct SnapshotReviewLease {
    database: Arc<StoreCoordinator>,
    retained: RetainedSnapshot,
    reference: SnapshotReference,
    pin: PreparedSnapshotReviewPin,
    decoded_slots: Arc<AtomicUsize>,
    decoded_bytes: Arc<AtomicU64>,
    _slot: SnapshotReviewSlot,
    _not_sync: PhantomData<Cell<()>>,
    #[cfg(test)]
    test_fault: Cell<SnapshotReviewTestFault>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SnapshotReviewTestFault {
    None,
    RenewAfterCommit,
    ReleaseAfterCommit,
    ReleaseAfterCommitConflicting,
}

#[path = "snapshot/repository.rs"]
mod repository;

#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
impl SnapshotReviewLease {
    #[cfg(test)]
    fn fail_next_renew_after_commit_for_test(&self) {
        assert_eq!(self.test_fault.get(), SnapshotReviewTestFault::None);
        self.test_fault
            .set(SnapshotReviewTestFault::RenewAfterCommit);
    }

    #[cfg(test)]
    fn fail_release_after_commit_for_test(&self) {
        assert_eq!(self.test_fault.get(), SnapshotReviewTestFault::None);
        self.test_fault
            .set(SnapshotReviewTestFault::ReleaseAfterCommit);
    }

    #[cfg(test)]
    fn replace_release_with_conflict_after_commit_for_test(&self) {
        assert_eq!(self.test_fault.get(), SnapshotReviewTestFault::None);
        self.test_fault
            .set(SnapshotReviewTestFault::ReleaseAfterCommitConflicting);
    }

    pub(crate) fn reference(&self) -> &SnapshotReference {
        &self.reference
    }

    pub(crate) fn purpose(&self) -> SnapshotReviewPurpose {
        self.pin.purpose()
    }

    pub(crate) fn expires_at(&self) -> Result<SystemTime, SnapshotRepositoryError> {
        unix_ms_to_system_time(self.pin.expires_at_unix_ms()).map_err(map_history)
    }

    /// Decode through the already-retained immutable handle after proving the
    /// durable lease is still exact and unexpired. SQLite is released before
    /// the potentially large decode.
    pub(crate) fn load(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotDocument, SnapshotRepositoryError> {
        self.validate(observed_at)?;
        decode_reference(&self.retained, &self.reference)
    }

    /// Decode once for an Explorer session while retaining a separately
    /// bounded per-engine decoded-document slot.
    pub(crate) fn load_for_review(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotReviewDocument, SnapshotRepositoryError> {
        let wire_bytes = self.retained.len().map_err(map_storage)?;
        let slot = SnapshotReviewDecodedSlot::reserve(
            &self.decoded_slots,
            &self.decoded_bytes,
            wire_bytes,
        )?;
        let document = self.load(observed_at)?;
        let (child_offsets, child_indices) = build_review_child_index(&document)?;
        Ok(SnapshotReviewDocument {
            document,
            child_offsets,
            child_indices,
            _slot: slot,
        })
    }

    /// Re-prove the exact durable pin and retained immutable object without
    /// decoding it again. Explorer uses this before every cached page read.
    pub(crate) fn validate(&self, observed_at: SystemTime) -> Result<(), SnapshotRepositoryError> {
        self.ensure_unexpired(observed_at)?;
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        validate_snapshot_review_pin(&database_guard.connection, &self.pin, observed_at)
            .map_err(map_review_history)?;
        self.retained.revalidate().map_err(map_storage)?;
        drop(database_guard);
        Ok(())
    }

    /// Load the typed coverage from the exact succeeded scan row protected by
    /// this live Explorer pin. Callers never supply coverage independently of
    /// the retained snapshot capability.
    pub(crate) fn validated_scan_coverage(
        &self,
        observed_at: SystemTime,
    ) -> Result<ScanCoverage, SnapshotRepositoryError> {
        self.ensure_unexpired(observed_at)?;
        if self.pin.purpose() != SnapshotReviewPurpose::Explorer {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        validate_snapshot_review_pin(&database_guard.connection, &self.pin, observed_at)
            .map_err(map_review_history)?;
        let scan = self
            .database
            .load_scan_with_guard(&database_guard, self.reference.scan_id())
            .map_err(map_history)?
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::SnapshotUnavailable))?;
        if scan.status() != ScanStatus::Succeeded
            || scan.snapshot() != Some(&self.reference)
            || scan.completed_at().is_none()
        {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        self.retained.revalidate().map_err(map_storage)?;
        let coverage = scan.coverage().clone();
        drop(database_guard);
        Ok(coverage)
    }

    /// Extend this exact live lease by the fixed duration. Expired leases are
    /// never resurrected; callers must reacquire through the repository.
    pub(crate) fn renew(
        &mut self,
        observed_at: SystemTime,
    ) -> Result<SystemTime, SnapshotRepositoryError> {
        self.ensure_unexpired(observed_at)?;
        let mut database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let transaction = database_guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(super::history::map_write_sql_error)
            .map_err(map_history)?;
        let renewed = renew_snapshot_review_pin(&transaction, &self.pin, observed_at)
            .map_err(map_review_history)?;
        let write = transaction
            .commit()
            .map_err(super::history::map_write_sql_error)
            .and_then(|()| self.after_renew_commit())
            .and_then(|()| {
                self.database
                    .revalidate_current_history_guard(&database_guard)
            });
        if let Err(failure) = write {
            revalidate_review_reconciliation(&self.database, &database_guard)?;
            if snapshot_review_pin_exactly_matches(&database_guard.connection, &renewed)
                .map_err(|_| outcome_unknown())?
            {
                // The exact post-renewal tuple is the only state this call can
                // have committed.
            } else if snapshot_review_pin_exactly_matches(&database_guard.connection, &self.pin)
                .map_err(|_| outcome_unknown())?
            {
                return Err(map_history(failure));
            } else {
                return Err(outcome_unknown());
            }
        }
        self.pin = renewed;
        self.expires_at()
    }

    /// Explicitly remove the exact durable pin. Consuming the lease guarantees
    /// the retained handle and local slot are closed on both success and
    /// failure. An unresolved row remains safe and expires naturally.
    pub(crate) fn release(self) -> Result<(), SnapshotRepositoryError> {
        let mut database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let transaction = database_guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(super::history::map_write_sql_error)
            .map_err(map_history)?;
        release_snapshot_review_pin(&transaction, &self.pin).map_err(map_review_history)?;
        let write = transaction
            .commit()
            .map_err(super::history::map_write_sql_error)
            .and_then(|()| self.after_release_commit(&database_guard.connection))
            .and_then(|()| {
                self.database
                    .revalidate_current_history_guard(&database_guard)
            });
        if let Err(failure) = write {
            revalidate_review_reconciliation(&self.database, &database_guard)?;
            match snapshot_review_pin_state(&database_guard.connection, &self.pin)
                .map_err(|_| outcome_unknown())?
            {
                SnapshotReviewPinState::Missing => {}
                SnapshotReviewPinState::Exact => return Err(map_history(failure)),
                SnapshotReviewPinState::Conflicting => return Err(outcome_unknown()),
            }
        }
        Ok(())
    }

    fn after_renew_commit(&self) -> Result<(), HistoryError> {
        #[cfg(test)]
        if self.test_fault.replace(SnapshotReviewTestFault::None)
            == SnapshotReviewTestFault::RenewAfterCommit
        {
            return Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable));
        }
        Ok(())
    }

    fn ensure_unexpired(&self, observed_at: SystemTime) -> Result<(), SnapshotRepositoryError> {
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)
                .map_err(map_history)?;
        if observed_at_unix_ms >= self.pin.expires_at_unix_ms() {
            Err(repository_error(
                SnapshotRepositoryErrorKind::ReviewLeaseExpired,
            ))
        } else {
            Ok(())
        }
    }

    fn after_release_commit(&self, connection: &rusqlite::Connection) -> Result<(), HistoryError> {
        #[cfg(test)]
        match self.test_fault.replace(SnapshotReviewTestFault::None) {
            SnapshotReviewTestFault::ReleaseAfterCommit => {
                return Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable));
            }
            SnapshotReviewTestFault::ReleaseAfterCommitConflicting => {
                let conflicting_purpose = match self.pin.purpose() {
                    SnapshotReviewPurpose::Explorer => "cleanup_review",
                    SnapshotReviewPurpose::CleanupReview => "explorer",
                };
                connection
                    .execute(
                        "INSERT INTO snapshot_review_pins (
                             pin_id, record_format_version, scan_id, scan_status,
                             completed_at_unix_ms, snapshot_version,
                             snapshot_relative_path, snapshot_relative_path_encoding,
                             snapshot_checksum_sha256, owner_process_instance, purpose,
                             created_at_unix_ms, renewed_at_unix_ms, expires_at_unix_ms
                         )
                         SELECT ?2, 1, scan_id, status, completed_at_unix_ms,
                                snapshot_version, snapshot_relative_path,
                                snapshot_relative_path_encoding,
                                snapshot_checksum_sha256, ?3, ?4, ?5, ?6, ?7
                         FROM scans WHERE scan_id = ?1",
                        rusqlite::params![
                            self.reference.scan_id().as_str(),
                            self.pin.id().as_str(),
                            self.pin.owner().as_str(),
                            conflicting_purpose,
                            self.pin.created_at_unix_ms(),
                            self.pin.renewed_at_unix_ms(),
                            self.pin.expires_at_unix_ms(),
                        ],
                    )
                    .map_err(super::history::map_write_sql_error)?;
                return Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable));
            }
            SnapshotReviewTestFault::None | SnapshotReviewTestFault::RenewAfterCommit => {}
        }
        #[cfg(not(test))]
        let _ = connection;
        Ok(())
    }
}

fn reconcile_review_pin(
    database: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    expected: &PreparedSnapshotReviewPin,
) -> Result<bool, SnapshotRepositoryError> {
    revalidate_review_reconciliation(database, guard)?;
    match snapshot_review_pin_state(&guard.connection, expected).map_err(|_| outcome_unknown())? {
        SnapshotReviewPinState::Missing => Ok(false),
        SnapshotReviewPinState::Exact => Ok(true),
        SnapshotReviewPinState::Conflicting => Err(outcome_unknown()),
    }
}

fn revalidate_review_reconciliation(
    database: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
) -> Result<(), SnapshotRepositoryError> {
    database
        .revalidate_current_history_guard(guard)
        .map_err(|_| outcome_unknown())
}

const fn outcome_unknown() -> SnapshotRepositoryError {
    repository_error(SnapshotRepositoryErrorKind::History(
        HistoryErrorKind::OutcomeUnknown,
    ))
}

fn validate_retained_document(
    retained: &RetainedSnapshot,
    expected: &SnapshotDocument,
    expected_digest: SnapshotDigest,
) -> Result<(), SnapshotRepositoryError> {
    let (document, digest) = decode_retained(retained)?;
    if &document != expected || digest != expected_digest {
        return Err(repository_error(
            SnapshotRepositoryErrorKind::ReferenceMismatch,
        ));
    }
    Ok(())
}

fn decode_retained(
    retained: &RetainedSnapshot,
) -> Result<(SnapshotDocument, SnapshotDigest), SnapshotRepositoryError> {
    let length = retained.len().map_err(map_storage)?;
    if length > codec::MAX_SNAPSHOT_FILE_BYTES {
        return Err(repository_error(SnapshotRepositoryErrorKind::Codec(
            SnapshotCodecErrorKind::LimitExceeded,
        )));
    }
    let mut file = retained.try_clone_file().map_err(map_storage)?;
    file.seek(SeekFrom::Start(0)).map_err(|_| {
        repository_error(SnapshotRepositoryErrorKind::Codec(
            SnapshotCodecErrorKind::Io,
        ))
    })?;
    let decoded = decode_snapshot(&mut file).map_err(map_codec)?;
    retained.revalidate().map_err(map_storage)?;
    Ok(decoded)
}

fn decode_reference(
    retained: &RetainedSnapshot,
    reference: &SnapshotReference,
) -> Result<SnapshotDocument, SnapshotRepositoryError> {
    if reference.version() != SNAPSHOT_FORMAT_VERSION {
        return Err(repository_error(
            SnapshotRepositoryErrorKind::IncompatibleVersion,
        ));
    }
    let (document, digest) = decode_retained(retained)?;
    if digest != reference.digest() || &document.metadata.scan_id != reference.scan_id() {
        return Err(repository_error(
            SnapshotRepositoryErrorKind::ReferenceMismatch,
        ));
    }
    Ok(document)
}

#[cfg(test)]
#[path = "snapshot/tests.rs"]
mod tests;
