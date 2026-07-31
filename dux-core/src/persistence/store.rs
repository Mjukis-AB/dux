use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::config::DbConfig;
use rusqlite::limits::Limit;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use crate::app_data_reset_transaction::AppDataResetTransaction;

use super::app_data_reset::AppDataResetStoreIdentity;
use super::app_data_reset_blocker::{
    AppDataResetStoreBlockers, inspect_app_data_reset_store_blockers,
};
use super::candidate_evaluation_history::{
    CandidateEvaluationCompletion, CandidateEvaluationObservation, CandidateEvaluationRecord,
    NewCandidateEvaluation, PendingCandidateEvaluation, PreparedCandidateEvaluation,
    insert_candidate_evaluation_pending, load_candidate_evaluation,
    load_candidate_evaluation_for_scan, load_candidate_evaluation_within_budget,
    load_candidate_validation_source, load_candidate_validation_source_for_trusted_claim,
    load_pending_candidate_evaluation,
};
use super::candidate_history::{
    CandidateEvaluationTransition, CandidateHistoryStatus, CandidateReviewTransition,
    NewCandidateRecord, PreparedCandidate, StoredCandidateRecord,
    ensure_prepared_candidate_batch_budget, insert_candidate, load_candidate_record,
    transition_candidate_evaluation, transition_candidate_review,
};
use super::capacity_history::{
    CapacityObservationOutcome, CapacityPage, CapacityPageCursor, CapacityPressureBaseline,
    CapacityTrend, CapacityWriteOutcome, CapacityWriteReason, PreparedCapacitySample,
    RawCapacityObservation, RawCapacitySample, StoredCapacitySample, StoredPressureEpisode,
    exact_raw_and_volume_match, exact_volume_observation_match, load_capacity_trend,
    load_latest_raw_capacity_sample, load_pressure_episode_page,
    load_pressure_episode_page_at_anchor, load_raw_capacity_page, load_volume_mount_path_at_anchor,
    validate_capacity_volume, validate_ephemeral_capacity_observation, write_raw_capacity_sample,
};
use super::cleanup_history::{
    CleanupSessionId, NewCleanupSessionRecord, PreparedCleanupSession, StoredCleanupSessionRecord,
    insert_cleanup_session, load_cleanup_session_record,
};
use super::cleanup_history_clear::{
    CleanupHistoryClearReconciliation, CleanupHistoryClearResult, CleanupHistoryClearStoreError,
    PreparedCleanupHistoryClear, apply_cleanup_history_clear, prepare_cleanup_history_clear,
    reconcile_cleanup_history_clear,
};
use super::cleanup_history_query::{
    StoredCleanupHistoryCursor, StoredCleanupHistoryObservation, StoredCleanupHistoryPage,
    cleanup_history_session as query_cleanup_history_session,
    recent_cleanup_history as query_recent_cleanup_history,
};
use super::footprint::{AiCacheFootprint, OwnedStorageUsage, inspect_ai_cache_footprint};
use super::history::{
    HistoryError, HistoryErrorKind, NewScanRecord, PreparedNewScan, PreparedScanCompletion,
    RecentScanRecords, ScanCompletionRecord, ScanRecord, ScanStatus, insert_scan_started,
    load_latest_available_snapshot_scan_record, load_latest_scan_record_for_exact_root_since,
    load_previous_comparable_snapshot_scan_record, load_recent_scan_records, load_scan_record,
    map_write_sql_error, update_scan_finished,
};
use super::migrations::{
    SchemaState, apply_pending_migrations, inspect_schema, inspect_schema_for_status,
};
use super::pressure_settings::load_disk_pressure_policy;
use super::process_liveness::{
    ProcessExecutionIdentity, ProcessIdentityError, ProcessInstanceId, ProcessLiveness,
    current_process_execution_identity, current_process_execution_provenance,
    probe_process_instance,
};
use super::retention::{RetentionBatchResult, apply_retention_batch, reconcile_retention_batch};
use super::running_scan_debt::{RunningScanDebtCensus, load_running_scan_debt_census};
use super::scan_process_claim::{
    ClaimedRunningScanProvenanceCensus, ScanClaimRecoveryState, ScanRecoveryBatchOutcome,
    ScanRecoveryBatchResult, canonical_recovery_time, classify_claims,
    consume_owned_scan_process_claim, count_remaining_scan_process_claims,
    exact_recovered_scan_matches, exact_scan_process_claim_matches, insert_scan_process_claim,
    interrupt_scan_process_claim, load_claimed_running_scan_provenance_census,
    load_scan_process_claim_page, scan_process_claim_is_missing,
};
use super::scan_scope_lease::{
    ScanScopeLeaseError, ScanScopeLeaseErrorKind, ScanScopeLeaseToken,
    acquire as acquire_scope_lease, exact_token_exists,
    map_history_error as map_scope_history_error, new_lease_id, prepare_canonical_root,
    release as release_scope_lease,
};
use super::snapshot_temp_lease::{
    PreparedSnapshotTempLease, SnapshotTempLeaseState, delete_snapshot_temp_lease,
    snapshot_temp_lease_state,
};
use super::status::{
    DATABASE_SCHEMA_VERSION, DatabaseAccess, DatabaseOpenError, DatabaseOpenErrorKind,
    DatabaseStatus,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::storage::{
    AppDataResetDataNamespaceAdmission as StorageDataNamespaceAdmission,
    AppDataResetRecoveryDataLocation as StorageRecoveryDataLocation,
    AppDataResetRecoveryDataNamespace as StorageRecoveryDataNamespace,
};
use super::storage::{CleanupLockGuard, SecureStorePaths, StoreIdentity, WriterLockGuard};

const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const RESET_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(5);

static COORDINATORS: OnceLock<Mutex<HashMap<StoreIdentity, Weak<StoreCoordinator>>>> =
    OnceLock::new();

fn select_capacity_pressure_baseline(
    durable: Option<&StoredCapacitySample>,
    session: Option<CapacityPressureBaseline>,
    observed_at: SystemTime,
    observed_capacity: crate::domain::VolumeCapacity,
    policy_revision: u64,
) -> Result<crate::domain::DiskPressure, HistoryError> {
    if durable.is_some_and(|value| value.policy_revision > policy_revision)
        || session.is_some_and(|value| value.policy_revision > policy_revision)
    {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    if let Some(durable) = durable {
        validate_capacity_observation_order(
            observed_at,
            durable.sampled_at,
            stored_capacity_matches(durable, observed_capacity),
        )?;
    }
    if let Some(session) = session {
        validate_capacity_observation_order(
            observed_at,
            session.sampled_at,
            session.capacity == observed_capacity,
        )?;
    }

    if let (Some(durable), Some(session)) = (durable, session)
        && durable.sampled_at == session.sampled_at
        && !stored_capacity_matches(durable, session.capacity)
    {
        return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
    }

    let durable = durable.filter(|value| value.policy_revision == policy_revision);
    let session = session.filter(|value| value.policy_revision == policy_revision);

    match (durable, session) {
        (Some(durable), Some(session)) => match durable.sampled_at.cmp(&session.sampled_at) {
            std::cmp::Ordering::Greater => Ok(durable.pressure),
            std::cmp::Ordering::Less => Ok(session.pressure),
            std::cmp::Ordering::Equal => Ok(durable.pressure),
        },
        (Some(durable), None) => Ok(durable.pressure),
        (None, Some(session)) => Ok(session.pressure),
        (None, None) => Ok(crate::domain::DiskPressure::Unknown),
    }
}

#[cfg(test)]
mod capacity_pressure_baseline_tests {
    use super::*;
    use crate::domain::{DiskPressure, VolumeCapacity, VolumeId};

    fn capacity() -> VolumeCapacity {
        VolumeCapacity::new(100, Some(40), Some(40)).unwrap()
    }

    #[test]
    fn future_durable_policy_revision_fails_closed() {
        let sampled_at = UNIX_EPOCH + Duration::from_secs(1);
        let durable = StoredCapacitySample {
            volume_id: VolumeId::new("volume:future-durable-policy").unwrap(),
            sampled_at,
            total_bytes: 100,
            available_bytes: 40,
            important_available_bytes: Some(40),
            pressure: DiskPressure::Warning,
            policy_revision: 2,
        };

        let error = select_capacity_pressure_baseline(
            Some(&durable),
            None,
            sampled_at + Duration::from_secs(1),
            capacity(),
            1,
        )
        .unwrap_err();

        assert_eq!(error.kind, HistoryErrorKind::CorruptData);
    }

    #[test]
    fn future_session_policy_revision_fails_closed() {
        let sampled_at = UNIX_EPOCH + Duration::from_secs(1);
        let session =
            CapacityPressureBaseline::new(sampled_at, capacity(), DiskPressure::Warning, 2);

        let error = select_capacity_pressure_baseline(
            None,
            Some(session),
            sampled_at + Duration::from_secs(1),
            capacity(),
            1,
        )
        .unwrap_err();

        assert_eq!(error.kind, HistoryErrorKind::CorruptData);
    }
}

fn validate_capacity_observation_order(
    observed_at: SystemTime,
    baseline_at: SystemTime,
    exact_facts_match: bool,
) -> Result<(), HistoryError> {
    match observed_at.cmp(&baseline_at) {
        std::cmp::Ordering::Less => Err(HistoryError::new(HistoryErrorKind::InvalidTransition)),
        std::cmp::Ordering::Equal if !exact_facts_match => {
            Err(HistoryError::new(HistoryErrorKind::AlreadyExists))
        }
        std::cmp::Ordering::Equal | std::cmp::Ordering::Greater => Ok(()),
    }
}

fn stored_capacity_matches(
    stored: &StoredCapacitySample,
    capacity: crate::domain::VolumeCapacity,
) -> bool {
    stored.total_bytes == capacity.total_bytes()
        && Some(stored.available_bytes) == capacity.available_bytes()
        && stored.important_available_bytes == capacity.important_available_bytes()
}

/// Semantic user review intent. Persistence resolves the exact source status
/// while holding the same transaction that performs the compare-and-set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CandidateReviewAction {
    Select,
    ClearSelection,
    Dismiss,
    Restore,
}

/// One serialized SQLite owner per physical store and process.
pub(crate) struct StoreCoordinator {
    status: Mutex<DatabaseStatus>,
    paths: SecureStorePaths,
    connection: Mutex<Connection>,
    scan_process_owner: OnceLock<ProcessExecutionIdentity>,
    scan_recovery_cursor: Mutex<Option<(i64, String)>>,
}

pub(super) struct HistoryConnectionGuard<'a> {
    // Struct fields drop in declaration order: release the cross-process lease
    // before another in-process caller can acquire the connection mutex.
    _writer_lock: WriterLockGuard,
    pub(super) connection: MutexGuard<'a, Connection>,
    store_identity: StoreIdentity,
}

/// Result of path-free app-data-reset admission.
///
/// `Blocked` carries observation only. `Admitted` is the sole variant that
/// retains store exclusion and therefore must remain live through reset
/// handoff.
#[must_use = "reset admission must retain or inspect the returned state"]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the namespace-handoff slice consumes retained reset admission"
    )
)]
pub(crate) enum AppDataResetStoreAdmission<'a> {
    Blocked(AppDataResetStoreBlockers),
    Admitted(AppDataResetStoreGuard<'a>),
}

/// Move-only retained database/cleanup exclusion for one app-data reset.
///
/// Fields intentionally drop in declaration order: the database writer and
/// connection are released before the cleanup exclusion.
#[must_use = "dropping the guard releases reset store exclusion"]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the namespace-handoff slice consumes the retained reset store guard"
    )
)]
pub(crate) struct AppDataResetStoreGuard<'a> {
    history: HistoryConnectionGuard<'a>,
    cleanup: CleanupLockGuard,
    store: &'a StoreCoordinator,
}

/// Validation-only proof that the exact data namespace remains safe to detach.
///
/// The owned publication fence stays outside this borrowed value, so neither
/// forgetting nor panicking with the witness can retain exclusion.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[must_use = "the namespace witness must be revalidated before reset handoff"]
pub(crate) struct AppDataResetDataNamespaceAdmission<'scope> {
    inner: StorageDataNamespaceAdmission<'scope>,
    store: &'scope StoreCoordinator,
    store_identity: StoreIdentity,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetRecoveryDataLocation {
    Canonical,
    Detached,
}

/// Descriptor-only recovery authority for a data root named by an existing
/// reset journal. No live coordinator or SQLite connection is constructed.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[must_use = "the recovery witness must be revalidated or reconciled"]
pub(crate) struct AppDataResetRecoveryDataNamespace<'scope> {
    inner: StorageRecoveryDataNamespace<'scope>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetRecoveryDataNamespace<'_> {
    pub(crate) const fn location(&self) -> AppDataResetRecoveryDataLocation {
        match self.inner.location() {
            StorageRecoveryDataLocation::Canonical => AppDataResetRecoveryDataLocation::Canonical,
            StorageRecoveryDataLocation::Detached => AppDataResetRecoveryDataLocation::Detached,
        }
    }

    pub(crate) fn journal_identity(&self) -> Result<AppDataResetStoreIdentity, HistoryError> {
        let (device, inode) = self.inner.journal_identity_parts();
        AppDataResetStoreIdentity::new(device, inode)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InternalState))
    }

    pub(crate) fn revalidate(&self) -> Result<(), HistoryError> {
        self.inner.revalidate().map_err(map_history_database_error)
    }

    pub(crate) fn detach_if_canonical(
        self,
        expected_identity: AppDataResetStoreIdentity,
        expected_stage_name: &str,
    ) -> Result<Self, HistoryError> {
        let inner = self
            .inner
            .detach_if_canonical(
                (expected_identity.device(), expected_identity.inode()),
                std::ffi::OsStr::new(expected_stage_name),
            )
            .map_err(map_history_database_error)?;
        Ok(Self { inner })
    }
}

/// Fail-closed placeholder on targets without proven namespace-detach
/// semantics. It is never constructed or passed to the callback.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[must_use = "unsupported platforms cannot mint a namespace witness"]
pub(crate) struct AppDataResetDataNamespaceAdmission<'scope> {
    _scope: std::marker::PhantomData<&'scope StoreCoordinator>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetDataNamespaceAdmission<'_> {
    pub(crate) fn journal_identity(&self) -> Result<AppDataResetStoreIdentity, HistoryError> {
        let (device, inode) = self.inner.journal_identity_parts();
        AppDataResetStoreIdentity::new(device, inode)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InternalState))
    }

    pub(crate) fn revalidate(
        &self,
        store_guard: &AppDataResetStoreGuard<'_>,
    ) -> Result<(), HistoryError> {
        if store_guard.history.store_identity != self.store_identity
            || !std::ptr::eq(store_guard.store, self.store)
        {
            return Err(HistoryError::new(HistoryErrorKind::InternalState));
        }
        self.inner.revalidate().map_err(map_history_database_error)
    }

    pub(crate) const fn is_detached(&self) -> bool {
        self.inner.is_detached()
    }

    pub(crate) fn detach(
        self,
        store_guard: &AppDataResetStoreGuard<'_>,
        expected_identity: AppDataResetStoreIdentity,
        expected_stage_name: &str,
    ) -> Result<Self, HistoryError> {
        if store_guard.history.store_identity != self.store_identity
            || !std::ptr::eq(store_guard.store, self.store)
        {
            return Err(HistoryError::new(HistoryErrorKind::InternalState));
        }
        let inner = self
            .inner
            .detach(
                (expected_identity.device(), expected_identity.inode()),
                std::ffi::OsStr::new(expected_stage_name),
            )
            .map_err(map_history_database_error)?;
        Ok(Self {
            inner,
            store: self.store,
            store_identity: self.store_identity,
        })
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl AppDataResetDataNamespaceAdmission<'_> {
    pub(crate) fn journal_identity(&self) -> Result<AppDataResetStoreIdentity, HistoryError> {
        Err(HistoryError::new(HistoryErrorKind::InternalState))
    }

    pub(crate) fn revalidate(
        &self,
        _store_guard: &AppDataResetStoreGuard<'_>,
    ) -> Result<(), HistoryError> {
        Err(HistoryError::new(HistoryErrorKind::InternalState))
    }

    pub(crate) const fn is_detached(&self) -> bool {
        false
    }

    pub(crate) fn detach(
        self,
        _store_guard: &AppDataResetStoreGuard<'_>,
        _expected_identity: AppDataResetStoreIdentity,
        _expected_stage_name: &str,
    ) -> Result<Self, HistoryError> {
        Err(HistoryError::new(HistoryErrorKind::InternalState))
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the namespace-handoff slice consumes retained reset revalidation"
    )
)]
impl AppDataResetStoreGuard<'_> {
    /// Prove that this retained guard belongs to the exact coordinator used by
    /// a later persistence layer.
    pub(crate) fn coordinates_store(&self, store: &Arc<StoreCoordinator>) -> bool {
        std::ptr::eq(self.store, Arc::as_ptr(store))
    }

    /// Revalidate every retained store control and repeat the bounded blocker
    /// observation without disclosing identifiers or paths.
    pub(crate) fn revalidate(&self) -> Result<AppDataResetStoreBlockers, HistoryError> {
        self.store
            .validate_cleanup_lock_for_journal(&self.cleanup)?;
        self.store.validate_history_guard(&self.history)?;
        let blockers =
            inspect_app_data_reset_store_blockers(&self.history.connection, SystemTime::now())?;
        self.store.revalidate_current_history_guard(&self.history)?;
        self.store
            .validate_cleanup_lock_for_journal(&self.cleanup)?;
        Ok(blockers)
    }

    /// Revalidate only retained filesystem guards after the data root has
    /// moved away from its canonical name. No canonical-path repair or SQLite
    /// operation is permitted after the namespace effect.
    pub(crate) fn revalidate_after_data_detach(&self) -> Result<(), HistoryError> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            self.store.validate_history_guard(&self.history)?;
            self.store
                .paths
                .validate_app_data_reset_detached_guards(&self.history._writer_lock, &self.cleanup)
                .map_err(map_history_database_error)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err(HistoryError::new(HistoryErrorKind::InternalState))
        }
    }
}

impl StoreCoordinator {
    pub(crate) fn open(database_path: &Path) -> Result<Arc<Self>, DatabaseOpenError> {
        super::migrations::validate_compiled_migrations()?;
        let paths = SecureStorePaths::prepare(database_path)?;
        let key = paths.identity();
        let registry = COORDINATORS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut coordinators = registry
            .lock()
            .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?;
        let existing = coordinators.get(&key).and_then(Weak::upgrade);
        if let Some(existing) = existing {
            drop(coordinators);
            existing.refresh_compatibility()?;
            return Ok(existing);
        }
        coordinators.retain(|_, coordinator| coordinator.strong_count() > 0);

        let sqlite_path = paths.sqlite_path()?;
        let coordinator = Arc::new(Self::open_unregistered(paths, &sqlite_path)?);
        coordinators.insert(key, Arc::downgrade(&coordinator));
        Ok(coordinator)
    }

    /// Inspect and, if necessary, roll forward the data detach recorded by an
    /// existing reset journal. This path cannot create a root, repair a
    /// sidecar, run a migration, or open SQLite.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn with_app_data_reset_recovery_data_namespace_until<T>(
        database_path: &Path,
        transaction: &AppDataResetTransaction,
        expected_identity: AppDataResetStoreIdentity,
        deadline: Instant,
        operation: impl for<'scope> FnOnce(AppDataResetRecoveryDataNamespace<'scope>) -> T,
    ) -> Result<T, HistoryError> {
        SecureStorePaths::with_app_data_reset_recovery_namespace_until(
            database_path,
            (expected_identity.device(), expected_identity.inode()),
            std::ffi::OsStr::new(transaction.data_stage().as_str()),
            deadline,
            |inner| operation(AppDataResetRecoveryDataNamespace { inner }),
        )
        .map_err(map_history_database_error)
    }

    /// Acquire the data-parent publication fence before any reset cleanup or
    /// database exclusion. The sealed transaction supplies the only accepted
    /// data-stage destination, and the higher-ranked witness cannot escape.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn with_app_data_reset_data_namespace_admission_until<T>(
        &self,
        transaction: &AppDataResetTransaction,
        deadline: Instant,
        operation: impl for<'scope> FnOnce(AppDataResetDataNamespaceAdmission<'scope>) -> T,
    ) -> Result<T, HistoryError> {
        self.paths
            .with_app_data_reset_namespace_fence_until(
                std::ffi::OsStr::new(transaction.data_stage().as_str()),
                deadline,
                |inner| {
                    operation(AppDataResetDataNamespaceAdmission {
                        inner,
                        store: self,
                        store_identity: self.paths.identity(),
                    })
                },
            )
            .map_err(map_history_database_error)
    }

    #[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn with_test_data_namespace_publication_fence_until<T>(
        &self,
        deadline: Instant,
        operation: impl for<'scope> FnOnce(AppDataResetDataNamespaceAdmission<'scope>) -> T,
    ) -> Result<T, HistoryError> {
        let transaction = AppDataResetTransaction::for_test("00112233445566778899aabbccddeeff")
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InternalState))?;
        self.with_app_data_reset_data_namespace_admission_until(&transaction, deadline, operation)
    }

    /// Namespace detachment remains unsupported until this target has native
    /// handle, access-control, reparse, and same-filesystem evidence.
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(super) fn with_app_data_reset_data_namespace_admission_until<T>(
        &self,
        _transaction: &AppDataResetTransaction,
        _deadline: Instant,
        _operation: impl for<'scope> FnOnce(AppDataResetDataNamespaceAdmission<'scope>) -> T,
    ) -> Result<T, HistoryError> {
        Err(HistoryError::new(HistoryErrorKind::InternalState))
    }

    fn open_unregistered(
        paths: SecureStorePaths,
        sqlite_path: &Path,
    ) -> Result<Self, DatabaseOpenError> {
        Self::open_unregistered_with_hook(paths, sqlite_path, || Ok(()))
    }

    fn open_unregistered_with_hook(
        paths: SecureStorePaths,
        sqlite_path: &Path,
        between_probe_and_lock: impl FnOnce() -> Result<(), DatabaseOpenError>,
    ) -> Result<Self, DatabaseOpenError> {
        paths.validate_all_existing()?;
        between_probe_and_lock()?;
        let _writer_lock = paths.acquire_writer_lock(MIGRATION_LOCK_TIMEOUT)?;
        paths.repair_sqlite_sidecars()?;
        paths.validate_all_existing()?;

        // A marker-owned database with a rollback journal or WAL must be
        // opened RW so SQLite can recover or recreate shared-memory state
        // before compatibility inspection. Without a recovery artifact, the
        // initial classification remains a strict RO open.
        let needs_recovery = paths.requires_initialization() || paths.recovery_artifact_exists()?;
        let mut read_only = None;
        let mut read_write = None;
        let schema = if needs_recovery {
            let connection = open_connection(sqlite_path, false)?;
            configure_connection(&connection, false)?;
            let schema = inspect_schema(&connection);
            paths.repair_sqlite_sidecars()?;
            read_write = Some(connection);
            schema?
        } else {
            let connection = open_connection(sqlite_path, true)?;
            configure_connection(&connection, true)?;
            let schema = inspect_schema(&connection)?;
            read_only = Some(connection);
            schema
        };

        if let SchemaState::Newer { found } = schema {
            drop(read_write);
            let connection = if let Some(connection) = read_only {
                connection
            } else {
                let connection = open_connection(sqlite_path, true)?;
                configure_connection(&connection, true)?;
                connection
            };
            if inspect_schema(&connection)? != (SchemaState::Newer { found }) {
                return Err(DatabaseOpenError::new(
                    DatabaseOpenErrorKind::CorruptDatabase,
                ));
            }
            paths.validate_all_existing()?;
            return Ok(Self {
                status: Mutex::new(DatabaseStatus {
                    schema_version: found,
                    access: DatabaseAccess::ReadOnlyNewer {
                        found,
                        supported: DATABASE_SCHEMA_VERSION,
                    },
                }),
                paths,
                connection: Mutex::new(connection),
                scan_process_owner: OnceLock::new(),
                scan_recovery_cursor: Mutex::new(None),
            });
        }

        drop(read_only);
        let mut connection = if let Some(connection) = read_write {
            connection
        } else {
            let connection = open_connection(sqlite_path, false)?;
            configure_connection(&connection, false)?;
            connection
        };
        apply_pending_migrations(&mut connection, unix_time_ms()?)?;
        enable_verified_schema_triggers(&connection)?;
        configure_write_ahead_log(&connection)?;
        paths.repair_sqlite_sidecars()?;
        paths.validate_all_existing()?;
        paths.mark_initialized()?;
        Ok(Self {
            status: Mutex::new(DatabaseStatus {
                schema_version: DATABASE_SCHEMA_VERSION,
                access: DatabaseAccess::ReadWriteCurrent,
            }),
            paths,
            connection: Mutex::new(connection),
            scan_process_owner: OnceLock::new(),
            scan_recovery_cursor: Mutex::new(None),
        })
    }

    fn scan_process_owner(&self) -> Result<ProcessExecutionIdentity, HistoryError> {
        if let Some(owner) = self.scan_process_owner.get() {
            return Ok(owner.clone());
        }
        let candidate =
            current_process_execution_identity().map_err(map_scan_process_identity_error)?;
        let _ = self.scan_process_owner.set(candidate);
        self.scan_process_owner
            .get()
            .cloned()
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InternalState))
    }

    /// Acquire one durable, cross-process exclusion for an exact canonical
    /// scan root. Existing exact, ancestor, descendant, and transitional
    /// running scopes, including legacy rows without claims, fail closed as
    /// `Busy`.
    pub(crate) fn acquire_scan_scope_lease(
        &self,
        canonical_root: &Path,
    ) -> Result<ScanScopeLeaseToken, ScanScopeLeaseError> {
        self.acquire_scan_scope_lease_with_hook(canonical_root, || Ok(()))
    }

    fn acquire_scan_scope_lease_with_hook(
        &self,
        canonical_root: &Path,
        after_commit: impl FnOnce() -> Result<(), ScanScopeLeaseError>,
    ) -> Result<ScanScopeLeaseToken, ScanScopeLeaseError> {
        let root = prepare_canonical_root(canonical_root)?;
        let owner = self.scan_process_owner().map_err(map_scope_history_error)?;
        let lease_id = new_lease_id()?;
        let mut guard = self
            .lock_current_history_connection()
            .map_err(map_scope_history_error)?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)
            .map_err(map_scope_history_error)?;
        let token = acquire_scope_lease(
            &transaction,
            self.paths.identity(),
            root,
            owner,
            lease_id,
            SystemTime::now(),
        )?;
        if let Err(error) = transaction.commit() {
            let failure = map_scope_history_error(map_write_sql_error(error));
            if self.revalidate_current_history_guard(&guard).is_err() {
                return Err(ScanScopeLeaseError::new(
                    ScanScopeLeaseErrorKind::OutcomeUnknown,
                ));
            }
            return match exact_token_exists(&guard.connection, &token) {
                Ok(true) => Ok(token),
                Ok(false) => Err(failure),
                Err(_) => Err(ScanScopeLeaseError::new(
                    ScanScopeLeaseErrorKind::OutcomeUnknown,
                )),
            };
        }
        if let Err(failure) = after_commit() {
            if self.revalidate_current_history_guard(&guard).is_err() {
                return Err(ScanScopeLeaseError::new(
                    ScanScopeLeaseErrorKind::OutcomeUnknown,
                ));
            }
            return match exact_token_exists(&guard.connection, &token) {
                Ok(true) => Ok(token),
                Ok(false) => Err(failure),
                Err(_) => Err(ScanScopeLeaseError::new(
                    ScanScopeLeaseErrorKind::OutcomeUnknown,
                )),
            };
        }
        if self.revalidate_current_history_guard(&guard).is_err() {
            // The committed row intentionally remains as a process-lifetime
            // availability quarantine. A failed storage/schema revalidation
            // makes this connection untrusted, so even an exact compensating
            // delete would be unsafe. Conservative stale recovery may reclaim
            // it only after this process instance is proven gone.
            return Err(ScanScopeLeaseError::new(
                ScanScopeLeaseErrorKind::OutcomeUnknown,
            ));
        }
        Ok(token)
    }

    #[cfg(test)]
    pub(super) fn acquire_scan_scope_lease_after_commit_failure_for_test(
        &self,
        canonical_root: &Path,
    ) -> Result<ScanScopeLeaseToken, ScanScopeLeaseError> {
        self.acquire_scan_scope_lease_with_hook(canonical_root, || {
            Err(ScanScopeLeaseError::new(
                ScanScopeLeaseErrorKind::Unavailable,
            ))
        })
    }

    /// Reconcile release of one exact move-only scope token. A missing row is
    /// idempotent success; a row with the same random ID but different facts
    /// is corruption and is never removed.
    pub(crate) fn release_scan_scope_lease(
        &self,
        token: &ScanScopeLeaseToken,
    ) -> Result<(), ScanScopeLeaseError> {
        self.release_scan_scope_lease_with_hook(token, || Ok(()))
    }

    fn release_scan_scope_lease_with_hook(
        &self,
        token: &ScanScopeLeaseToken,
        after_commit: impl FnOnce() -> Result<(), ScanScopeLeaseError>,
    ) -> Result<(), ScanScopeLeaseError> {
        let mut guard = self
            .lock_current_history_connection()
            .map_err(map_scope_history_error)?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)
            .map_err(map_scope_history_error)?;
        release_scope_lease(&transaction, self.paths.identity(), token)?;
        if let Err(error) = transaction.commit() {
            let failure = map_scope_history_error(map_write_sql_error(error));
            if self.revalidate_current_history_guard(&guard).is_err() {
                return Err(ScanScopeLeaseError::new(
                    ScanScopeLeaseErrorKind::OutcomeUnknown,
                ));
            }
            return match exact_token_exists(&guard.connection, token) {
                Ok(false) => Ok(()),
                Ok(true) => Err(failure),
                Err(_) => Err(ScanScopeLeaseError::new(
                    ScanScopeLeaseErrorKind::OutcomeUnknown,
                )),
            };
        }
        if let Err(failure) = after_commit() {
            if self.revalidate_current_history_guard(&guard).is_err() {
                return Err(ScanScopeLeaseError::new(
                    ScanScopeLeaseErrorKind::OutcomeUnknown,
                ));
            }
            return match exact_token_exists(&guard.connection, token) {
                Ok(false) => Ok(()),
                Ok(true) => Err(failure),
                Err(_) => Err(ScanScopeLeaseError::new(
                    ScanScopeLeaseErrorKind::OutcomeUnknown,
                )),
            };
        }
        match self.revalidate_current_history_guard(&guard) {
            Ok(()) => Ok(()),
            Err(error) => match exact_token_exists(&guard.connection, token) {
                Ok(false) => Ok(()),
                Ok(true) | Err(_) => {
                    let _ = error;
                    Err(ScanScopeLeaseError::new(
                        ScanScopeLeaseErrorKind::OutcomeUnknown,
                    ))
                }
            },
        }
    }

    #[cfg(test)]
    pub(super) fn release_scan_scope_lease_after_commit_failure_for_test(
        &self,
        token: &ScanScopeLeaseToken,
    ) -> Result<(), ScanScopeLeaseError> {
        self.release_scan_scope_lease_with_hook(token, || {
            Err(ScanScopeLeaseError::new(
                ScanScopeLeaseErrorKind::Unavailable,
            ))
        })
    }

    #[cfg(test)]
    pub(super) fn set_scan_process_identity_for_test(&self, identity: ProcessExecutionIdentity) {
        assert!(self.scan_process_owner.set(identity).is_ok());
    }

    fn refresh_compatibility(&self) -> Result<(), DatabaseOpenError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?;
        let _writer_lock = self.paths.acquire_writer_lock(MIGRATION_LOCK_TIMEOUT)?;
        self.paths.repair_sqlite_sidecars()?;
        self.paths.validate_all_existing()?;
        // A live coordinator already owns a recovery-capable RW connection
        // when its schema is current. Its healthy WAL is ordinary connection
        // state, not evidence that status should churn the connection or run
        // startup integrity inspection. A newer coordinator is already bound
        // to a validated RO connection and must never be reopened RW.
        let refreshed_schema = inspect_schema_for_status(&connection);
        self.paths.repair_sqlite_sidecars()?;
        match refreshed_schema? {
            SchemaState::Current => {
                if matches!(
                    self.cached_status().access,
                    DatabaseAccess::ReadOnlyNewer { .. }
                ) {
                    return Err(DatabaseOpenError::new(
                        DatabaseOpenErrorKind::CorruptDatabase,
                    ));
                }
                *self
                    .status
                    .lock()
                    .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))? =
                    DatabaseStatus {
                        schema_version: DATABASE_SCHEMA_VERSION,
                        access: DatabaseAccess::ReadWriteCurrent,
                    };
            }
            SchemaState::Newer { found } => {
                let sqlite_path = self.paths.sqlite_path()?;
                let read_only = open_connection(&sqlite_path, true)?;
                configure_connection(&read_only, true)?;
                if inspect_schema_for_status(&read_only)? != (SchemaState::Newer { found }) {
                    return Err(DatabaseOpenError::new(
                        DatabaseOpenErrorKind::CorruptDatabase,
                    ));
                }
                self.paths.validate_all_existing()?;
                *connection = read_only;
                *self
                    .status
                    .lock()
                    .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))? =
                    DatabaseStatus {
                        schema_version: found,
                        access: DatabaseAccess::ReadOnlyNewer {
                            found,
                            supported: DATABASE_SCHEMA_VERSION,
                        },
                    };
            }
            SchemaState::Empty | SchemaState::Older { .. } => {
                return Err(DatabaseOpenError::new(
                    DatabaseOpenErrorKind::CorruptDatabase,
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn status(&self) -> Result<DatabaseStatus, DatabaseOpenError> {
        self.refresh_compatibility()?;
        Ok(self.cached_status())
    }

    /// Return the exact retained database path used to derive owned sibling
    /// stores. Callers must never accept an independent path alongside this
    /// coordinator, because that could fence one database while mutating
    /// another root.
    pub(crate) fn validated_database_path(&self) -> Result<PathBuf, HistoryError> {
        self.paths
            .validate_all_existing()
            .and_then(|()| self.paths.sqlite_path())
            .map_err(map_history_database_error)
    }

    /// Persist one raw capacity observation after applying cross-process
    /// cadence admission. Capacity facts are telemetry only and never cleanup
    /// authority.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "capacity persistence integrates with the volume monitor in a later slice"
        )
    )]
    pub(crate) fn record_raw_capacity_sample(
        &self,
        sample: &RawCapacitySample,
        reason: CapacityWriteReason,
    ) -> Result<CapacityWriteOutcome, HistoryError> {
        self.record_raw_capacity_sample_with_hook(sample, reason, || Ok(()))
    }

    /// Atomically derive and persist pressure for one capacity observation.
    ///
    /// Loading the previous pressure, applying hysteresis, selecting transition
    /// admission, and writing all happen while holding the same cross-process
    /// writer lease. This prevents two app/CLI processes from losing a
    /// transition. Important-only observations are evaluated against the
    /// latest durable pressure but deliberately cause no SQLite mutation.
    pub(crate) fn observe_capacity(
        &self,
        observation: &RawCapacityObservation,
        session_baseline: Option<CapacityPressureBaseline>,
    ) -> Result<CapacityObservationOutcome, HistoryError> {
        self.observe_capacity_with_hook(observation, session_baseline, || Ok(()))
    }

    fn observe_capacity_with_hook(
        &self,
        observation: &RawCapacityObservation,
        session_baseline: Option<CapacityPressureBaseline>,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<CapacityObservationOutcome, HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        if observation.capacity().available_bytes().is_none() {
            let transaction = guard
                .connection
                .transaction_with_behavior(TransactionBehavior::Deferred)
                .map_err(map_write_sql_error)?;
            let latest = load_latest_raw_capacity_sample(&transaction, observation.volume_id())?;
            let policy = load_disk_pressure_policy(&transaction)?;
            let has_volume = validate_ephemeral_capacity_observation(
                &transaction,
                observation.volume_id(),
                latest.as_slice(),
                observation.sampled_at(),
            )?;
            if latest.is_some() && !has_volume {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            let previous = select_capacity_pressure_baseline(
                latest.as_ref(),
                session_baseline,
                observation.sampled_at(),
                observation.capacity(),
                policy.revision,
            )?;
            let evaluation = policy.config.evaluate(observation.capacity(), previous);
            transaction.commit().map_err(map_write_sql_error)?;
            self.revalidate_current_history_guard(&guard)?;
            return Ok(CapacityObservationOutcome {
                evaluation,
                previous_durable_pressure: latest.as_ref().map(|sample| sample.pressure),
                write: None,
                effective_policy: policy,
            });
        }

        let mut attempted_outcome = None;
        let attempt = (|| {
            let transaction = guard
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(map_write_sql_error)?;
            // Re-read under the SQLite write transaction. The advisory writer
            // lease already fences cooperating processes; this second read
            // also makes the policy dependency explicit at the mutation site.
            let transactional_latest =
                load_latest_raw_capacity_sample(&transaction, observation.volume_id())?;
            let transactional_policy = load_disk_pressure_policy(&transaction)?;
            let transactional_previous = select_capacity_pressure_baseline(
                transactional_latest.as_ref(),
                session_baseline,
                observation.sampled_at(),
                observation.capacity(),
                transactional_policy.revision,
            )?;
            let transactional_evaluation = transactional_policy
                .config
                .evaluate(observation.capacity(), transactional_previous);
            let transactional_sample = observation
                .with_pressure(
                    transactional_evaluation.pressure(),
                    transactional_policy.revision,
                )?
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::InternalState))?;
            let transactional_prepared = PreparedCapacitySample::prepare(&transactional_sample)?;
            let transactional_reason = match transactional_latest.as_ref() {
                Some(previous) if previous.policy_revision != transactional_policy.revision => {
                    CapacityWriteReason::PolicyBaseline
                }
                Some(previous) if previous.pressure != transactional_evaluation.pressure() => {
                    CapacityWriteReason::PressureTransition
                }
                Some(_) | None => CapacityWriteReason::Routine,
            };
            let outcome = write_raw_capacity_sample(
                &transaction,
                &transactional_prepared,
                transactional_reason,
            )?;
            let previous_durable_pressure =
                transactional_latest.as_ref().map(|sample| sample.pressure);
            attempted_outcome = Some((
                outcome,
                transactional_evaluation,
                previous_durable_pressure,
                transactional_prepared,
                transactional_policy,
            ));
            transaction.commit().map_err(map_write_sql_error)?;
            after_commit()?;
            self.revalidate_current_history_guard(&guard)?;
            Ok(CapacityObservationOutcome {
                evaluation: transactional_evaluation,
                previous_durable_pressure,
                write: Some(outcome),
                effective_policy: transactional_policy,
            })
        })();
        let failure = match attempt {
            Ok(outcome) => return Ok(outcome),
            Err(error) => error,
        };

        let Some((
            attempted_write,
            attempted_evaluation,
            attempted_previous_pressure,
            attempted_prepared,
            attempted_policy,
        )) = attempted_outcome
        else {
            return Err(failure);
        };
        if self.revalidate_current_history_guard(&guard).is_err() {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        let reconciled = match attempted_write {
            CapacityWriteOutcome::Inserted | CapacityWriteOutcome::ExistingExact => {
                exact_raw_and_volume_match(&guard.connection, &attempted_prepared)
            }
            CapacityWriteOutcome::Suppressed => {
                exact_volume_observation_match(&guard.connection, &attempted_prepared)
            }
        };
        match reconciled {
            Ok(true) => Ok(CapacityObservationOutcome {
                evaluation: attempted_evaluation,
                previous_durable_pressure: attempted_previous_pressure,
                write: Some(attempted_write),
                effective_policy: attempted_policy,
            }),
            Ok(false) => Err(failure),
            Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    #[cfg(test)]
    pub(super) fn observe_capacity_after_commit_failure_for_test(
        &self,
        observation: &RawCapacityObservation,
    ) -> Result<CapacityObservationOutcome, HistoryError> {
        self.observe_capacity_with_hook(observation, None, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    /// Evaluate an intentionally ephemeral observation against the latest
    /// durable pressure without mutating volume metadata or sample history.
    pub(crate) fn evaluate_capacity(
        &self,
        volume_id: &crate::domain::VolumeId,
        sampled_at: SystemTime,
        capacity: crate::domain::VolumeCapacity,
        session_baseline: Option<CapacityPressureBaseline>,
    ) -> Result<CapacityObservationOutcome, HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(map_write_sql_error)?;
        let latest = load_latest_raw_capacity_sample(&transaction, volume_id)?;
        let policy = load_disk_pressure_policy(&transaction)?;
        let has_volume = validate_ephemeral_capacity_observation(
            &transaction,
            volume_id,
            latest.as_slice(),
            sampled_at,
        )?;
        if latest.is_some() && !has_volume {
            return Err(HistoryError::new(HistoryErrorKind::CorruptData));
        }
        let previous = select_capacity_pressure_baseline(
            latest.as_ref(),
            session_baseline,
            sampled_at,
            capacity,
            policy.revision,
        );
        let result = policy.config.evaluate(capacity, previous?);
        transaction.commit().map_err(map_write_sql_error)?;
        self.revalidate_current_history_guard(&guard)?;
        Ok(CapacityObservationOutcome {
            evaluation: result,
            previous_durable_pressure: latest.as_ref().map(|sample| sample.pressure),
            write: None,
            effective_policy: policy,
        })
    }

    /// Evaluate a capacity observation without stable volume identity. The
    /// current policy still comes from the guarded store, while durable volume
    /// history is deliberately unavailable and untouched.
    pub(crate) fn evaluate_unidentified_capacity(
        &self,
        sampled_at: SystemTime,
        capacity: crate::domain::VolumeCapacity,
        session_baseline: Option<CapacityPressureBaseline>,
    ) -> Result<CapacityObservationOutcome, HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(map_write_sql_error)?;
        let policy = load_disk_pressure_policy(&transaction)?;
        let previous = select_capacity_pressure_baseline(
            None,
            session_baseline,
            sampled_at,
            capacity,
            policy.revision,
        )?;
        let evaluation = policy.config.evaluate(capacity, previous);
        transaction.commit().map_err(map_write_sql_error)?;
        self.revalidate_current_history_guard(&guard)?;
        Ok(CapacityObservationOutcome {
            evaluation,
            previous_durable_pressure: None,
            write: None,
            effective_policy: policy,
        })
    }

    fn record_raw_capacity_sample_with_hook(
        &self,
        sample: &RawCapacitySample,
        reason: CapacityWriteReason,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<CapacityWriteOutcome, HistoryError> {
        let prepared = PreparedCapacitySample::prepare(sample)?;
        let mut guard = self.lock_current_history_connection()?;
        let mut attempted_outcome = None;
        let attempt = (|| {
            let transaction = guard
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(map_write_sql_error)?;
            let outcome = write_raw_capacity_sample(&transaction, &prepared, reason)?;
            attempted_outcome = Some(outcome);
            transaction.commit().map_err(map_write_sql_error)?;
            after_commit()?;
            self.revalidate_current_history_guard(&guard)?;
            Ok(outcome)
        })();
        let failure = match attempt {
            Ok(outcome) => return Ok(outcome),
            Err(error) => error,
        };

        let Some(attempted_outcome) = attempted_outcome else {
            return Err(failure);
        };
        if self.revalidate_current_history_guard(&guard).is_err() {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        let reconciled = match attempted_outcome {
            CapacityWriteOutcome::Inserted | CapacityWriteOutcome::ExistingExact => {
                exact_raw_and_volume_match(&guard.connection, &prepared)
            }
            CapacityWriteOutcome::Suppressed => {
                exact_volume_observation_match(&guard.connection, &prepared)
            }
        };
        match reconciled {
            Ok(true) => Ok(attempted_outcome),
            Ok(false) => Err(failure),
            Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    #[cfg(test)]
    pub(super) fn record_raw_capacity_sample_after_commit_failure_for_test(
        &self,
        sample: &RawCapacitySample,
        reason: CapacityWriteReason,
    ) -> Result<CapacityWriteOutcome, HistoryError> {
        self.record_raw_capacity_sample_with_hook(sample, reason, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    #[cfg(all(test, unix))]
    pub(super) fn record_raw_capacity_sample_with_after_commit_hook_for_test(
        &self,
        sample: &RawCapacitySample,
        reason: CapacityWriteReason,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<CapacityWriteOutcome, HistoryError> {
        self.record_raw_capacity_sample_with_hook(sample, reason, after_commit)
    }

    /// Load the newest raw capacity observation for one stable volume.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "capacity history integrates with app status in a later slice"
        )
    )]
    pub(crate) fn load_latest_raw_capacity_sample(
        &self,
        volume_id: &crate::domain::VolumeId,
    ) -> Result<Option<StoredCapacitySample>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        let sample = load_latest_raw_capacity_sample(&guard.connection, volume_id)?;
        let has_volume = validate_capacity_volume(&guard.connection, volume_id, sample.as_slice())?;
        if sample.is_some() && !has_volume {
            return Err(HistoryError::new(HistoryErrorKind::CorruptData));
        }
        Ok(sample)
    }

    /// Load one deterministic descending page of raw observations.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "capacity charts integrate with app history in a later slice"
        )
    )]
    pub(crate) fn load_raw_capacity_page(
        &self,
        volume_id: &crate::domain::VolumeId,
        cursor: Option<CapacityPageCursor>,
        limit: usize,
    ) -> Result<CapacityPage, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        let page = load_raw_capacity_page(&guard.connection, volume_id, cursor, limit)?;
        let has_volume = validate_capacity_volume(&guard.connection, volume_id, &page.samples)?;
        if !page.samples.is_empty() && !has_volume {
            return Err(HistoryError::new(HistoryErrorKind::CorruptData));
        }
        Ok(page)
    }

    /// Load a bounded newest-first pressure-episode page for trend and
    /// notification consumers. The returned rows are telemetry only.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "pressure episode pages integrate with later trend and notification slices"
        )
    )]
    pub(crate) fn load_pressure_episode_page(
        &self,
        volume_id: &crate::domain::VolumeId,
        limit: usize,
    ) -> Result<Vec<StoredPressureEpisode>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_pressure_episode_page(&guard.connection, volume_id, limit)
    }

    pub(crate) fn load_pressure_episode_page_at_anchor(
        &self,
        volume_id: &crate::domain::VolumeId,
        anchor_at: SystemTime,
        limit: usize,
    ) -> Result<Vec<StoredPressureEpisode>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_pressure_episode_page_at_anchor(&guard.connection, volume_id, anchor_at, limit)
    }

    pub(crate) fn load_volume_mount_path_at_anchor(
        &self,
        volume_id: &crate::domain::VolumeId,
        anchor_at: SystemTime,
    ) -> Result<PathBuf, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_volume_mount_path_at_anchor(&guard.connection, volume_id, anchor_at)
    }

    /// Build a bounded path-free trend view from durable capacity samples.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "capacity trend data integrates with the app history chart in a later slice"
        )
    )]
    pub(crate) fn load_capacity_trend(
        &self,
        volume_id: &crate::domain::VolumeId,
        anchor_at: SystemTime,
    ) -> Result<Option<CapacityTrend>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_capacity_trend(&guard.connection, volume_id, anchor_at)
    }

    /// Test primitive for exercising non-reconciled scan-start behavior.
    #[cfg(test)]
    pub(crate) fn record_scan_started(&self, scan: &NewScanRecord) -> Result<(), HistoryError> {
        self.record_scan_started_with_hook(scan, || Ok(()))
    }

    fn record_scan_started_with_hook(
        &self,
        scan: &NewScanRecord,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<(), HistoryError> {
        let prepared = PreparedNewScan::prepare(scan)?;
        let owner = self.scan_process_owner()?;
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        insert_scan_started(&transaction, &prepared)?;
        insert_scan_process_claim(&transaction, scan.id(), scan.started_at(), &owner)?;
        transaction.commit().map_err(map_write_sql_error)?;
        after_commit()?;
        self.paths
            .repair_sqlite_sidecars()
            .and_then(|()| self.paths.validate_all_existing())
            .map_err(map_history_database_error)
    }

    /// Insert one frozen scan start and reconcile a possible post-commit
    /// failure against that exact ID/root/time tuple. A collision with any
    /// different row is never adopted.
    pub(crate) fn record_scan_started_reconciled(
        &self,
        scan: &NewScanRecord,
    ) -> Result<(), HistoryError> {
        self.record_scan_started_reconciled_with_hook(scan, || Ok(()))
    }

    fn record_scan_started_reconciled_with_hook(
        &self,
        scan: &NewScanRecord,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<(), HistoryError> {
        let failure = match self.record_scan_started_with_hook(scan, after_commit) {
            Ok(()) => return Ok(()),
            Err(failure) => failure,
        };
        let owner = self.scan_process_owner()?;
        match self.load_scan(scan.id()) {
            Ok(Some(record)) if record.exactly_matches_start(scan) => {
                let guard = self
                    .lock_current_history_connection()
                    .map_err(|_| HistoryError::new(HistoryErrorKind::OutcomeUnknown))?;
                match exact_scan_process_claim_matches(
                    &guard.connection,
                    scan.id(),
                    scan.started_at(),
                    &owner,
                ) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err(HistoryError::new(HistoryErrorKind::AlreadyExists)),
                    Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
                }
            }
            Ok(Some(_)) => Err(HistoryError::new(HistoryErrorKind::AlreadyExists)),
            Ok(None) => Err(failure),
            Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    #[cfg(test)]
    pub(super) fn record_scan_started_reconciled_after_commit_failure_for_test(
        &self,
        scan: &NewScanRecord,
    ) -> Result<(), HistoryError> {
        self.record_scan_started_reconciled_with_hook(scan, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    /// Compare-and-set one running scan to a terminal durable summary.
    /// A database/storage error after commit can have an ambiguous outcome;
    /// callers reconcile by loading this exact scan ID before retrying.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the engine uses reconciled completion; this primitive remains for focused persistence tests"
        )
    )]
    pub(crate) fn record_scan_finished(
        &self,
        completion: &ScanCompletionRecord,
    ) -> Result<(), HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        self.record_scan_finished_with_guard(&mut guard, completion)
    }

    fn record_scan_finished_with_guard(
        &self,
        guard: &mut HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
    ) -> Result<(), HistoryError> {
        self.record_scan_finished_with_guard_and_hook(guard, completion, || Ok(()))
    }

    fn record_scan_finished_with_guard_and_hook(
        &self,
        guard: &mut HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<(), HistoryError> {
        self.validate_history_guard(guard)?;
        let prepared = PreparedScanCompletion::prepare(completion)?;
        let owner = self.scan_process_owner()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        consume_owned_scan_process_claim(&transaction, completion.id(), &owner)?;
        update_scan_finished(&transaction, &prepared)?;
        transaction.commit().map_err(map_write_sql_error)?;
        after_commit()?;
        self.revalidate_current_history_guard(guard)
    }

    /// Complete one frozen scan operation and reconcile every potentially
    /// ambiguous failure against that exact operation. Only an exact durable
    /// match is idempotent success; a different terminal row is never adopted.
    pub(crate) fn record_scan_finished_reconciled(
        &self,
        completion: &ScanCompletionRecord,
    ) -> Result<(), HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        self.record_scan_finished_reconciled_with_guard(&mut guard, completion)
    }

    pub(super) fn record_scan_finished_reconciled_with_guard(
        &self,
        guard: &mut HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
    ) -> Result<(), HistoryError> {
        self.record_scan_finished_reconciled_with_guard_and_hook(guard, completion, || Ok(()))
    }

    /// Atomically consume the exact snapshot staging row and commit its
    /// succeeded scan summary. The immutable snapshot file is already durable
    /// and the caller retains snapshot-writer exclusion through this commit.
    pub(super) fn record_scan_finished_with_temp_lease_reconciled_with_guard(
        &self,
        guard: &mut HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        lease: &PreparedSnapshotTempLease,
    ) -> Result<(), HistoryError> {
        self.record_scan_finished_with_temp_lease_with_hook(guard, completion, lease, || Ok(()))
    }

    fn record_scan_finished_with_temp_lease_with_hook(
        &self,
        guard: &mut HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        lease: &PreparedSnapshotTempLease,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<(), HistoryError> {
        self.validate_history_guard(guard)?;
        if completion.id() != lease.scan_id() {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let prepared = PreparedScanCompletion::prepare(completion)?;
        let owner = self.scan_process_owner()?;
        let failure = {
            let transaction = guard
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(map_write_sql_error)?;
            let attempted = delete_snapshot_temp_lease(&transaction, lease)
                .and_then(|()| {
                    consume_owned_scan_process_claim(&transaction, completion.id(), &owner)
                })
                .and_then(|()| update_scan_finished(&transaction, &prepared))
                .and_then(|()| transaction.commit().map_err(map_write_sql_error))
                .and_then(|()| after_commit())
                .and_then(|()| self.revalidate_current_history_guard(guard));
            match attempted {
                Ok(()) => return Ok(()),
                Err(failure) => failure,
            }
        };
        self.reconcile_scan_completion_and_temp_lease(guard, completion, lease, failure)
    }

    #[cfg(test)]
    pub(super) fn record_scan_finished_with_temp_lease_after_commit_failure_for_test(
        &self,
        completion: &ScanCompletionRecord,
        lease: &PreparedSnapshotTempLease,
    ) -> Result<(), HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        self.record_scan_finished_with_temp_lease_with_hook(&mut guard, completion, lease, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    fn record_scan_finished_reconciled_with_guard_and_hook(
        &self,
        guard: &mut HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<(), HistoryError> {
        self.validate_history_guard(guard)?;
        let failure =
            match self.record_scan_finished_with_guard_and_hook(guard, completion, after_commit) {
                Ok(()) => return Ok(()),
                Err(failure) => failure,
            };
        if self.revalidate_current_history_guard(guard).is_err() {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        let claim_missing = scan_process_claim_is_missing(&guard.connection, completion.id());
        match (
            load_scan_record(&guard.connection, completion.id()),
            claim_missing,
        ) {
            (Ok(Some(record)), Ok(true)) if record.exactly_matches_completion(completion) => Ok(()),
            (Ok(Some(record)), Ok(_)) if record.status() == super::history::ScanStatus::Running => {
                Err(failure)
            }
            (Ok(Some(_)), Ok(_)) => Err(HistoryError::new(HistoryErrorKind::InvalidTransition)),
            (Ok(None), Ok(_)) => Err(HistoryError::new(HistoryErrorKind::NotFound)),
            _ => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    fn reconcile_scan_completion_and_temp_lease(
        &self,
        guard: &HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        lease: &PreparedSnapshotTempLease,
        failure: HistoryError,
    ) -> Result<(), HistoryError> {
        if self.revalidate_current_history_guard(guard).is_err() {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        let scan = load_scan_record(&guard.connection, completion.id());
        let lease_state = snapshot_temp_lease_state(&guard.connection, lease);
        let claim_missing = scan_process_claim_is_missing(&guard.connection, completion.id());
        match (scan, lease_state, claim_missing) {
            (Ok(Some(scan)), Ok(SnapshotTempLeaseState::Missing), Ok(true))
                if scan.exactly_matches_completion(completion) =>
            {
                Ok(())
            }
            (Ok(Some(scan)), Ok(SnapshotTempLeaseState::Exact), Ok(_))
                if scan.status() == super::history::ScanStatus::Running =>
            {
                Err(failure)
            }
            (Ok(Some(scan)), Ok(SnapshotTempLeaseState::Conflicting), Ok(_))
                if scan.status() == super::history::ScanStatus::Running =>
            {
                Err(HistoryError::new(HistoryErrorKind::CorruptData))
            }
            (Ok(Some(_)), Ok(_), Ok(_)) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
            (Ok(None), Ok(_), Ok(_)) => Err(HistoryError::new(HistoryErrorKind::NotFound)),
            _ => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    #[cfg(test)]
    pub(super) fn record_scan_finished_reconciled_after_commit_failure_for_test(
        &self,
        completion: &ScanCompletionRecord,
    ) -> Result<(), HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        self.record_scan_finished_reconciled_with_guard_and_hook(&mut guard, completion, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    #[cfg(all(test, unix))]
    pub(super) fn record_scan_finished_reconciled_with_after_commit_hook_for_test(
        &self,
        completion: &ScanCompletionRecord,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<(), HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        self.record_scan_finished_reconciled_with_guard_and_hook(
            &mut guard,
            completion,
            after_commit,
        )
    }

    /// Atomically mark a scan succeeded and schedule its exact evaluation.
    /// This pending-only path is reserved for a future asynchronous/recovery
    /// protocol; normal engine scans use the terminal combined path below.
    #[allow(
        dead_code,
        reason = "reserved for a future asynchronous evaluation recovery protocol"
    )]
    pub(super) fn record_scan_finished_and_schedule_evaluation_reconciled_with_guard(
        &self,
        guard: &mut HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        request: &NewCandidateEvaluation,
    ) -> Result<(), HistoryError> {
        self.record_scan_finished_and_schedule_evaluation_with_hook(
            guard,
            completion,
            request,
            || Ok(()),
        )
    }

    #[cfg(test)]
    pub(super) fn record_scan_finished_and_schedule_evaluation_with_temp_lease_reconciled_with_guard(
        &self,
        guard: &mut HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        request: &NewCandidateEvaluation,
        lease: &PreparedSnapshotTempLease,
    ) -> Result<(), HistoryError> {
        self.validate_history_guard(guard)?;
        if completion.id() != request.scan_id() || completion.id() != lease.scan_id() {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let prepared_completion = PreparedScanCompletion::prepare(completion)?;
        let prepared_evaluation = PreparedCandidateEvaluation::prepare(request)?;
        let owner = self.scan_process_owner()?;
        let failure = {
            let transaction = guard
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(map_write_sql_error)?;
            let attempted = delete_snapshot_temp_lease(&transaction, lease)
                .and_then(|()| {
                    consume_owned_scan_process_claim(&transaction, completion.id(), &owner)
                })
                .and_then(|()| update_scan_finished(&transaction, &prepared_completion))
                .and_then(|()| {
                    insert_candidate_evaluation_pending(&transaction, &prepared_evaluation)
                })
                .and_then(|()| transaction.commit().map_err(map_write_sql_error))
                .and_then(|()| self.revalidate_current_history_guard(guard));
            match attempted {
                Ok(()) => return Ok(()),
                Err(failure) => failure,
            }
        };
        if self.revalidate_current_history_guard(guard).is_err() {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        let scan = load_scan_record(&guard.connection, completion.id());
        let evaluation = load_candidate_evaluation(&guard.connection, request.scan_id());
        let lease_state = snapshot_temp_lease_state(&guard.connection, lease);
        let claim_missing = scan_process_claim_is_missing(&guard.connection, completion.id());
        match (scan, evaluation, lease_state, claim_missing) {
            (
                Ok(Some(scan)),
                Ok(Some(evaluation)),
                Ok(SnapshotTempLeaseState::Missing),
                Ok(true),
            ) if scan.exactly_matches_completion(completion)
                && evaluation.exactly_matches_request(request) =>
            {
                Ok(())
            }
            (Ok(Some(scan)), Ok(None), Ok(SnapshotTempLeaseState::Exact), Ok(_))
                if scan.status() == super::history::ScanStatus::Running =>
            {
                Err(failure)
            }
            (Ok(None), Ok(None), Ok(_), Ok(_)) => {
                Err(HistoryError::new(HistoryErrorKind::NotFound))
            }
            _ => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    #[allow(
        dead_code,
        reason = "reserved for a future asynchronous evaluation recovery protocol"
    )]
    fn record_scan_finished_and_schedule_evaluation_with_hook(
        &self,
        guard: &mut HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        request: &NewCandidateEvaluation,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<(), HistoryError> {
        self.validate_history_guard(guard)?;
        if completion.id() != request.scan_id() {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let prepared_completion = PreparedScanCompletion::prepare(completion)?;
        let prepared_evaluation = PreparedCandidateEvaluation::prepare(request)?;
        let owner = self.scan_process_owner()?;
        let failure = {
            let transaction = guard
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(map_write_sql_error)?;
            let attempted = consume_owned_scan_process_claim(&transaction, completion.id(), &owner)
                .and_then(|()| update_scan_finished(&transaction, &prepared_completion))
                .and_then(|()| {
                    insert_candidate_evaluation_pending(&transaction, &prepared_evaluation)
                })
                .and_then(|()| transaction.commit().map_err(map_write_sql_error))
                .and_then(|()| after_commit())
                .and_then(|()| self.revalidate_current_history_guard(guard));
            match attempted {
                Ok(()) => return Ok(()),
                Err(failure) => failure,
            }
        };
        self.reconcile_scan_and_evaluation(guard, completion, request, None, failure)
    }

    /// Atomically consume the exact snapshot staging row, commit the succeeded
    /// scan, and finalize its complete deterministic candidate evaluation.
    pub(super) fn record_scan_finished_with_evaluation_and_temp_lease_reconciled_with_guard(
        &self,
        guard: &mut HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        request: &NewCandidateEvaluation,
        evaluation: &CandidateEvaluationCompletion,
        lease: &PreparedSnapshotTempLease,
    ) -> Result<(), HistoryError> {
        self.record_scan_finished_with_evaluation_and_temp_lease_with_hook(
            guard,
            completion,
            request,
            evaluation,
            lease,
            || Ok(()),
        )
    }

    fn record_scan_finished_with_evaluation_and_temp_lease_with_hook(
        &self,
        guard: &mut HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        request: &NewCandidateEvaluation,
        evaluation: &CandidateEvaluationCompletion,
        lease: &PreparedSnapshotTempLease,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<(), HistoryError> {
        self.validate_history_guard(guard)?;
        if completion.id() != request.scan_id() || completion.id() != lease.scan_id() {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        evaluation.validate_for_request(request)?;
        let prepared_completion = PreparedScanCompletion::prepare(completion)?;
        let prepared_evaluation = PreparedCandidateEvaluation::prepare(request)?;
        let candidates = evaluation.prepare_candidates()?;
        let owner = self.scan_process_owner()?;
        let failure = {
            let transaction = guard
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(map_write_sql_error)?;
            let attempted = delete_snapshot_temp_lease(&transaction, lease)
                .and_then(|()| {
                    consume_owned_scan_process_claim(&transaction, completion.id(), &owner)
                })
                .and_then(|()| update_scan_finished(&transaction, &prepared_completion))
                .and_then(|()| {
                    insert_candidate_evaluation_pending(&transaction, &prepared_evaluation)
                })
                .and_then(|()| evaluation.finalize(&transaction, request, &candidates))
                .and_then(|()| transaction.commit().map_err(map_write_sql_error))
                .and_then(|()| after_commit())
                .and_then(|()| self.revalidate_current_history_guard(guard));
            match attempted {
                Ok(()) => return Ok(()),
                Err(failure) => failure,
            }
        };
        self.reconcile_scan_evaluation_and_temp_lease(
            guard, completion, request, evaluation, lease, failure,
        )
    }

    #[allow(
        dead_code,
        reason = "reserved for a future asynchronous evaluation recovery protocol"
    )]
    fn reconcile_scan_and_evaluation(
        &self,
        guard: &HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        request: &NewCandidateEvaluation,
        terminal: Option<&CandidateEvaluationCompletion>,
        failure: HistoryError,
    ) -> Result<(), HistoryError> {
        if self.revalidate_current_history_guard(guard).is_err() {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        let scan = load_scan_record(&guard.connection, completion.id());
        let evaluation = load_candidate_evaluation(&guard.connection, request.scan_id());
        let claim_missing = scan_process_claim_is_missing(&guard.connection, completion.id());
        match (scan, evaluation, claim_missing) {
            (Ok(Some(scan)), Ok(Some(evaluation)), Ok(true))
                if scan.exactly_matches_completion(completion)
                    && evaluation.exactly_matches_request(request)
                    && terminal.is_none() =>
            {
                Ok(())
            }
            (Ok(Some(scan)), Ok(Some(evaluation)), Ok(true))
                if scan.exactly_matches_completion(completion)
                    && terminal.is_some_and(|terminal| {
                        terminal.exactly_matches_record(&evaluation, request)
                    }) =>
            {
                Ok(())
            }
            (Ok(Some(scan)), Ok(None), Ok(_))
                if scan.status() == super::history::ScanStatus::Running =>
            {
                Err(failure)
            }
            (Ok(Some(scan)), Ok(None), Ok(_)) if scan.exactly_matches_completion(completion) => {
                Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown))
            }
            (Ok(Some(_)), Ok(_), Ok(_)) => {
                Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
            }
            (Ok(None), Ok(None), Ok(_)) => Err(HistoryError::new(HistoryErrorKind::NotFound)),
            _ => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    fn reconcile_scan_evaluation_and_temp_lease(
        &self,
        guard: &HistoryConnectionGuard<'_>,
        completion: &ScanCompletionRecord,
        request: &NewCandidateEvaluation,
        terminal: &CandidateEvaluationCompletion,
        lease: &PreparedSnapshotTempLease,
        failure: HistoryError,
    ) -> Result<(), HistoryError> {
        if self.revalidate_current_history_guard(guard).is_err() {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        let scan = load_scan_record(&guard.connection, completion.id());
        let evaluation = load_candidate_evaluation(&guard.connection, request.scan_id());
        let lease_state = snapshot_temp_lease_state(&guard.connection, lease);
        let claim_missing = scan_process_claim_is_missing(&guard.connection, completion.id());
        match (scan, evaluation, lease_state, claim_missing) {
            (
                Ok(Some(scan)),
                Ok(Some(evaluation)),
                Ok(SnapshotTempLeaseState::Missing),
                Ok(true),
            ) if scan.exactly_matches_completion(completion)
                && terminal.exactly_matches_record(&evaluation, request) =>
            {
                Ok(())
            }
            (Ok(Some(scan)), Ok(None), Ok(SnapshotTempLeaseState::Exact), Ok(_))
                if scan.status() == super::history::ScanStatus::Running =>
            {
                Err(failure)
            }
            (Ok(Some(scan)), Ok(None), Ok(SnapshotTempLeaseState::Conflicting), Ok(_))
                if scan.status() == super::history::ScanStatus::Running =>
            {
                Err(HistoryError::new(HistoryErrorKind::CorruptData))
            }
            (Ok(None), Ok(None), Ok(_), Ok(_)) => {
                Err(HistoryError::new(HistoryErrorKind::NotFound))
            }
            (Ok(Some(_)), Ok(_), Ok(_), Ok(_)) => {
                Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown))
            }
            _ => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    /// Finish a previously scheduled evaluation in one all-or-nothing write.
    #[allow(
        dead_code,
        reason = "reserved for a future asynchronous evaluation recovery protocol"
    )]
    pub(crate) fn record_candidate_evaluation_completed_reconciled(
        &self,
        request: &NewCandidateEvaluation,
        evaluation: &CandidateEvaluationCompletion,
    ) -> Result<(), HistoryError> {
        self.record_candidate_evaluation_completed_with_hook(request, evaluation, || Ok(()))
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "reserved for a future asynchronous evaluation recovery protocol"
        )
    )]
    fn record_candidate_evaluation_completed_with_hook(
        &self,
        request: &NewCandidateEvaluation,
        evaluation: &CandidateEvaluationCompletion,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<(), HistoryError> {
        evaluation.validate_for_request(request)?;
        let candidates = evaluation.prepare_candidates()?;
        let mut guard = self.lock_current_history_connection()?;
        let failure = {
            let transaction = guard
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(map_write_sql_error)?;
            let attempted = evaluation
                .finalize(&transaction, request, &candidates)
                .and_then(|()| transaction.commit().map_err(map_write_sql_error))
                .and_then(|()| after_commit())
                .and_then(|()| self.revalidate_current_history_guard(&guard));
            match attempted {
                Ok(()) => return Ok(()),
                Err(failure) => failure,
            }
        };
        if self.revalidate_current_history_guard(&guard).is_err() {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        match load_candidate_evaluation(&guard.connection, request.scan_id()) {
            Ok(Some(record)) if evaluation.exactly_matches_record(&record, request) => Ok(()),
            Ok(Some(record))
                if record.exactly_matches_request(request)
                    && record.status()
                        == super::candidate_evaluation_history::CandidateEvaluationStatus::Pending =>
            {
                Err(failure)
            }
            Ok(Some(_)) => Err(HistoryError::new(HistoryErrorKind::InvalidTransition)),
            Ok(None) => Err(HistoryError::new(HistoryErrorKind::NotFound)),
            Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    /// Load one exact, bounded evaluation and its complete candidate batch.
    pub(crate) fn load_candidate_evaluation(
        &self,
        scan_id: &crate::domain::ScanId,
    ) -> Result<Option<CandidateEvaluationRecord>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_candidate_evaluation(&guard.connection, scan_id)
    }

    /// Select one bounded pending evaluator row together with its exact
    /// succeeded scan. Snapshot identity and scan terminal shape are checked
    /// before the row leaves persistence; malformed or incompatible state is
    /// returned as a typed failure rather than becoming replay authority.
    pub(crate) fn load_pending_candidate_evaluation(
        &self,
    ) -> Result<Option<(ScanRecord, PendingCandidateEvaluation)>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        let Some(pending) = load_pending_candidate_evaluation(&guard.connection)? else {
            return Ok(None);
        };
        let scan = load_scan_record(&guard.connection, pending.record().scan_id())?
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
        let Some(snapshot) = scan.snapshot() else {
            return Err(HistoryError::new(HistoryErrorKind::CorruptData));
        };
        if scan.status() != ScanStatus::Succeeded
            || snapshot.scan_id() != scan.id()
            || pending.record().request_for_snapshot(snapshot).is_err()
        {
            return Err(HistoryError::new(HistoryErrorKind::CorruptData));
        }
        Ok(Some((scan, pending)))
    }

    /// Load the complete candidate-evaluation observation for one exact scan.
    /// The result is immutable history and never planner or executor authority.
    pub(crate) fn load_candidate_evaluation_for_scan(
        &self,
        scan_id: &crate::domain::ScanId,
    ) -> Result<CandidateEvaluationObservation, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_candidate_evaluation_for_scan(&guard.connection, scan_id)
    }

    /// Load one exact current succeeded scan/evaluation/candidate join under a
    /// single durable history guard. The result remains observational.
    pub(crate) fn load_candidate_validation_source(
        &self,
        scan_id: &crate::domain::ScanId,
        candidate_id: &crate::domain::CandidateId,
    ) -> Result<super::CandidateValidationSourceRecord, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_candidate_validation_source(&guard.connection, scan_id, candidate_id)
    }

    /// Reopen the exact scan/evaluation/candidate source only while one active
    /// trusted Rust-target journal owns its revisioned candidate claim.
    pub(crate) fn load_candidate_validation_source_for_trusted_claim(
        &self,
        scan_id: &crate::domain::ScanId,
        candidate_id: &crate::domain::CandidateId,
        session_id: &CleanupSessionId,
        item_ordinal: usize,
    ) -> Result<super::CandidateValidationSourceRecord, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_candidate_validation_source_for_trusted_claim(
            &guard.connection,
            scan_id,
            candidate_id,
            session_id,
            item_ordinal,
        )
    }

    pub(super) fn load_candidate_evaluation_with_guard(
        &self,
        guard: &HistoryConnectionGuard<'_>,
        scan_id: &crate::domain::ScanId,
    ) -> Result<Option<CandidateEvaluationRecord>, HistoryError> {
        self.validate_history_guard(guard)?;
        load_candidate_evaluation(&guard.connection, scan_id)
    }

    #[cfg(test)]
    pub(super) fn record_scan_finished_with_evaluation_and_temp_lease_after_commit_failure_for_test(
        &self,
        completion: &ScanCompletionRecord,
        request: &NewCandidateEvaluation,
        evaluation: &CandidateEvaluationCompletion,
        lease: &PreparedSnapshotTempLease,
    ) -> Result<(), HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        self.record_scan_finished_with_evaluation_and_temp_lease_with_hook(
            &mut guard,
            completion,
            request,
            evaluation,
            lease,
            || Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable)),
        )
    }

    #[cfg(test)]
    pub(super) fn record_candidate_evaluation_completed_after_commit_failure_for_test(
        &self,
        request: &NewCandidateEvaluation,
        evaluation: &CandidateEvaluationCompletion,
    ) -> Result<(), HistoryError> {
        self.record_candidate_evaluation_completed_with_hook(request, evaluation, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    /// Load at most one typed scan observation by its stable ID.
    pub(crate) fn load_scan(
        &self,
        id: &crate::domain::ScanId,
    ) -> Result<Option<ScanRecord>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_scan_record(&guard.connection, id)
    }

    pub(super) fn load_scan_with_guard(
        &self,
        guard: &HistoryConnectionGuard<'_>,
        id: &crate::domain::ScanId,
    ) -> Result<Option<ScanRecord>, HistoryError> {
        self.validate_history_guard(guard)?;
        load_scan_record(&guard.connection, id)
    }

    pub(crate) fn load_recent_scans(
        &self,
        limit: usize,
    ) -> Result<RecentScanRecords, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_recent_scan_records(&guard.connection, limit)
    }

    pub(crate) fn load_latest_scan_for_exact_root_since(
        &self,
        root: &Path,
        started_at_or_after: SystemTime,
    ) -> Result<Option<ScanRecord>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_latest_scan_record_for_exact_root_since(&guard.connection, root, started_at_or_after)
    }

    pub(crate) fn load_latest_available_snapshot_scan(
        &self,
    ) -> Result<Option<ScanRecord>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_latest_available_snapshot_scan_record(&guard.connection)
    }

    pub(crate) fn load_previous_comparable_snapshot_scan(
        &self,
        current: &ScanRecord,
    ) -> Result<Option<ScanRecord>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_previous_comparable_snapshot_scan_record(&guard.connection, current)
    }

    /// Count only unclaimed running rows in one bounded, deterministic page.
    /// This is a read-only diagnostic and carries no row identity or authority.
    pub(crate) fn running_scan_debt_census(&self) -> Result<RunningScanDebtCensus, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_running_scan_debt_census(&guard.connection)
    }

    /// Classify one bounded deterministic page of claimed running scans using
    /// only stored and current host/boot provenance. This read never probes a
    /// claimed process and carries no recovery or mutation authority.
    pub(crate) fn claimed_running_scan_provenance_census(
        &self,
    ) -> Result<ClaimedRunningScanProvenanceCensus, HistoryError> {
        let current = current_process_execution_provenance();
        let guard = self.lock_current_history_connection()?;
        load_claimed_running_scan_provenance_census(&guard.connection, current.as_ref())
    }

    /// Recover at most one pristine claimed scan after same-boot process death
    /// or complete same-host prior-boot proof. OS probes, when needed, run
    /// with SQLite locks dropped; prior-boot interruption performs no probe.
    pub(crate) fn run_scan_recovery_batch(
        &self,
        observed_at: SystemTime,
    ) -> Result<ScanRecoveryBatchResult, HistoryError> {
        self.run_scan_recovery_batch_with_hooks(
            observed_at,
            probe_process_instance,
            || Ok(()),
            || Ok(()),
        )
    }

    fn run_scan_recovery_batch_with_hooks(
        &self,
        observed_at: SystemTime,
        probe: impl FnMut(&ProcessInstanceId) -> ProcessLiveness,
        before_write: impl FnOnce() -> Result<(), HistoryError>,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<ScanRecoveryBatchResult, HistoryError> {
        let (observed_at, completed_at_unix_ms) = canonical_recovery_time(observed_at)?;
        let identity = self.scan_process_owner()?;
        let cursor = self
            .scan_recovery_cursor
            .lock()
            .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?
            .clone();
        let page = {
            let guard = self.lock_current_history_connection()?;
            let mut page = load_scan_process_claim_page(
                &guard.connection,
                cursor.as_ref().map(|cursor| (cursor.0, cursor.1.as_str())),
            )?;
            if page.claims.is_empty() && cursor.is_some() {
                page = load_scan_process_claim_page(&guard.connection, None)?;
            }
            page
        };
        let page_has_more = page.has_more;
        let claims = page.claims;
        let claimed_count_before = u32::try_from(claims.len())
            .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
        if claims.is_empty() {
            *self
                .scan_recovery_cursor
                .lock()
                .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))? = None;
            return Ok(ScanRecoveryBatchResult {
                observed_at,
                outcome: ScanRecoveryBatchOutcome::NoClaim,
                claimed_count_before: 0,
                claimed_count_after: 0,
                alive_count: 0,
                unknown_count: 0,
                recoverable_count: 0,
                has_more: false,
            });
        }

        let (states, alive_count, unknown_count, recoverable_count) =
            classify_claims(&claims, &identity, probe);
        let Some(target_index) = states
            .iter()
            .position(|state| *state == ScanClaimRecoveryState::Recoverable)
        else {
            *self
                .scan_recovery_cursor
                .lock()
                .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))? = if page_has_more
            {
                claims
                    .last()
                    .map(|claim| (claim.cursor().0, claim.cursor().1.to_owned()))
            } else {
                None
            };
            return Ok(ScanRecoveryBatchResult {
                observed_at,
                outcome: ScanRecoveryBatchOutcome::DeferredUnproven,
                claimed_count_before,
                claimed_count_after: claimed_count_before,
                alive_count,
                unknown_count,
                recoverable_count: 0,
                has_more: page_has_more,
            });
        };
        let target = &claims[target_index];
        let target_cursor = (target.cursor().0, target.cursor().1.to_owned());
        before_write()?;

        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        if !interrupt_scan_process_claim(&transaction, target, completed_at_unix_ms)? {
            drop(transaction);
            let claimed_count_after =
                count_remaining_scan_process_claims(&guard.connection, &claims)?;
            *self
                .scan_recovery_cursor
                .lock()
                .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))? =
                Some(target_cursor);
            return Ok(ScanRecoveryBatchResult {
                observed_at,
                outcome: ScanRecoveryBatchOutcome::ChangedConcurrently,
                claimed_count_before,
                claimed_count_after,
                alive_count,
                unknown_count,
                recoverable_count,
                has_more: true,
            });
        }
        let attempt = transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard));
        if let Err(failure) = attempt {
            if self.revalidate_current_history_guard(&guard).is_err() {
                return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
            }
            match exact_recovered_scan_matches(&guard.connection, target, completed_at_unix_ms) {
                Ok(true) => {}
                Ok(false) => return Err(failure),
                Err(_) => return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
            }
        }
        let claimed_count_after =
            count_remaining_scan_process_claims(&guard.connection, &claims)
                .map_err(|_| HistoryError::new(HistoryErrorKind::OutcomeUnknown))?;
        let has_more = recoverable_count > 1 || page_has_more;
        *self
            .scan_recovery_cursor
            .lock()
            .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))? =
            has_more.then_some(target_cursor);
        Ok(ScanRecoveryBatchResult {
            observed_at,
            outcome: ScanRecoveryBatchOutcome::Interrupted,
            claimed_count_before,
            claimed_count_after,
            alive_count,
            unknown_count,
            recoverable_count,
            has_more,
        })
    }

    #[cfg(test)]
    pub(super) fn run_scan_recovery_batch_with_hooks_for_test(
        &self,
        observed_at: SystemTime,
        probe: impl FnMut(&ProcessInstanceId) -> ProcessLiveness,
        before_write: impl FnOnce() -> Result<(), HistoryError>,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<ScanRecoveryBatchResult, HistoryError> {
        self.run_scan_recovery_batch_with_hooks(observed_at, probe, before_write, after_commit)
    }

    /// Apply one bounded batch of automatic retention to DUX-owned capacity
    /// telemetry and expired AI cache rows. Cleanup, scan, candidate, outcome,
    /// schedule, and settings history are outside the SQL mutation allowlist.
    pub(crate) fn run_history_retention_batch(
        &self,
        observed_at: SystemTime,
    ) -> Result<RetentionBatchResult, HistoryError> {
        self.run_history_retention_batch_with_hook(observed_at, || Ok(()))
    }

    fn run_history_retention_batch_with_hook(
        &self,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<RetentionBatchResult, HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let applied = apply_retention_batch(&transaction, observed_at)?;
        let result = applied.result;
        let reconciliation = applied.reconciliation;
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => return Ok(result),
            Err(failure) => failure,
        };

        if self.revalidate_current_history_guard(&guard).is_err() {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        match reconcile_retention_batch(&guard.connection, &reconciliation) {
            Ok(true) => Ok(result),
            Ok(false) => Err(failure),
            Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    #[cfg(test)]
    pub(super) fn run_history_retention_batch_after_commit_failure_for_test(
        &self,
        observed_at: SystemTime,
    ) -> Result<RetentionBatchResult, HistoryError> {
        self.run_history_retention_batch_with_hook(observed_at, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    /// Insert one complete deterministic candidate observation atomically.
    /// A database/storage error after commit can have an ambiguous outcome;
    /// callers reconcile by loading this exact candidate ID before retrying.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "typed candidate persistence is integrated by the later evaluator task slice"
        )
    )]
    pub(crate) fn record_candidate_discovered(
        &self,
        candidate: &NewCandidateRecord,
    ) -> Result<(), HistoryError> {
        let prepared = PreparedCandidate::prepare(candidate)?;
        ensure_prepared_candidate_batch_budget(std::slice::from_ref(&prepared))?;
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        if load_candidate_evaluation_within_budget(
            &transaction,
            candidate.candidate().source_scan_id(),
        )?
        .is_some()
        {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        insert_candidate(&transaction, &prepared)?;
        transaction.commit().map_err(map_write_sql_error)?;
        self.paths
            .repair_sqlite_sidecars()
            .and_then(|()| self.paths.validate_all_existing())
            .map_err(map_history_database_error)
    }

    /// Load one candidate history observation without granting plan authority.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "typed candidate persistence is integrated by the later evaluator task slice"
        )
    )]
    pub(crate) fn load_candidate(
        &self,
        id: &crate::domain::CandidateId,
    ) -> Result<Option<StoredCandidateRecord>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_candidate_record(&guard.connection, id)
    }

    /// Persist one review-only candidate status transition. Selection is user
    /// intent, not plan approval or cleanup authority.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "candidate review transport integrates with the evaluator task slice"
        )
    )]
    pub(crate) fn transition_candidate_review_status(
        &self,
        id: &crate::domain::CandidateId,
        transition: CandidateReviewTransition,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        self.transition_candidate_review_status_with_hook(id, transition, || Ok(()))
    }

    /// Apply one scan-bound semantic review command. The scan binding and the
    /// discovered-versus-selected dismissal source are resolved inside the
    /// mutation transaction; no engine pre-read can authorize this update.
    pub(crate) fn review_candidate(
        &self,
        scan_id: &crate::domain::ScanId,
        id: &crate::domain::CandidateId,
        action: CandidateReviewAction,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        self.review_candidate_with_hook(scan_id, id, action, || Ok(()))
    }

    fn review_candidate_with_hook(
        &self,
        scan_id: &crate::domain::ScanId,
        id: &crate::domain::CandidateId,
        action: CandidateReviewAction,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        self.transition_candidate_status_with_hook(
            id,
            |transaction, id| {
                let Some(record) = load_candidate_record(transaction, id)? else {
                    return Err(HistoryError::new(HistoryErrorKind::NotFound));
                };
                let StoredCandidateRecord::Complete(candidate) = record else {
                    return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
                };
                if candidate.source_scan_id() != scan_id {
                    return Err(HistoryError::new(HistoryErrorKind::NotFound));
                }
                let transition = match action {
                    CandidateReviewAction::Select => CandidateReviewTransition::Select,
                    CandidateReviewAction::ClearSelection => {
                        CandidateReviewTransition::ClearSelection
                    }
                    CandidateReviewAction::Restore => CandidateReviewTransition::Restore,
                    CandidateReviewAction::Dismiss => match candidate.status() {
                        CandidateHistoryStatus::Discovered | CandidateHistoryStatus::Dismissed => {
                            CandidateReviewTransition::DismissDiscovered
                        }
                        CandidateHistoryStatus::Selected => {
                            CandidateReviewTransition::DismissSelected
                        }
                        _ => {
                            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
                        }
                    },
                };
                transition_candidate_review(transaction, id, transition)
            },
            after_commit,
        )
    }

    #[cfg(test)]
    pub(crate) fn review_candidate_after_commit_failure_for_test(
        &self,
        scan_id: &crate::domain::ScanId,
        id: &crate::domain::CandidateId,
        action: CandidateReviewAction,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        self.review_candidate_with_hook(scan_id, id, action, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    fn transition_candidate_review_status_with_hook(
        &self,
        id: &crate::domain::CandidateId,
        transition: CandidateReviewTransition,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        self.transition_candidate_status_with_hook(
            id,
            |transaction, id| transition_candidate_review(transaction, id, transition),
            after_commit,
        )
    }

    /// Persist one evaluator-owned terminal disposition for a scan-bound
    /// candidate observation. This cannot enter plan or execution states.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "candidate evaluator transport integrates with the evaluator task slice"
        )
    )]
    pub(super) fn transition_candidate_evaluation_status(
        &self,
        id: &crate::domain::CandidateId,
        transition: CandidateEvaluationTransition,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        self.transition_candidate_evaluation_status_with_hook(id, transition, || Ok(()))
    }

    fn transition_candidate_evaluation_status_with_hook(
        &self,
        id: &crate::domain::CandidateId,
        transition: CandidateEvaluationTransition,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        self.transition_candidate_status_with_hook(
            id,
            |transaction, id| transition_candidate_evaluation(transaction, id, transition),
            after_commit,
        )
    }

    fn transition_candidate_status_with_hook(
        &self,
        id: &crate::domain::CandidateId,
        transition: impl FnOnce(
            &rusqlite::Transaction<'_>,
            &crate::domain::CandidateId,
        ) -> Result<
            (CandidateHistoryStatus, CandidateHistoryStatus),
            HistoryError,
        >,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let (expected, target) = transition(&transaction, id)?;
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => return Ok(target),
            Err(failure) => failure,
        };

        if self.revalidate_current_history_guard(&guard).is_err() {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        match load_candidate_record(&guard.connection, id) {
            Ok(Some(StoredCandidateRecord::Complete(record))) if record.status == target => {
                Ok(target)
            }
            Ok(Some(StoredCandidateRecord::Complete(record))) if record.status == expected => {
                Err(failure)
            }
            Ok(Some(_)) => Err(HistoryError::new(HistoryErrorKind::InvalidTransition)),
            Ok(None) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    #[cfg(test)]
    pub(super) fn transition_candidate_evaluation_status_after_commit_failure_for_test(
        &self,
        id: &crate::domain::CandidateId,
        transition: CandidateEvaluationTransition,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        self.transition_candidate_evaluation_status_with_hook(id, transition, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    #[cfg(all(test, unix))]
    pub(super) fn transition_candidate_evaluation_status_with_after_commit_hook_for_test(
        &self,
        id: &crate::domain::CandidateId,
        transition: CandidateEvaluationTransition,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        self.transition_candidate_evaluation_status_with_hook(id, transition, after_commit)
    }

    #[cfg(test)]
    pub(super) fn transition_candidate_review_status_after_commit_failure_for_test(
        &self,
        id: &crate::domain::CandidateId,
        transition: CandidateReviewTransition,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        self.transition_candidate_review_status_with_hook(id, transition, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    #[cfg(all(test, unix))]
    pub(super) fn transition_candidate_review_status_with_after_commit_hook_for_test(
        &self,
        id: &crate::domain::CandidateId,
        transition: CandidateReviewTransition,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<CandidateHistoryStatus, HistoryError> {
        self.transition_candidate_review_status_with_hook(id, transition, after_commit)
    }

    /// Atomically freeze one review-data plan as a non-executable planned journal.
    /// Callers reconcile an ambiguous post-commit failure by loading the exact
    /// session ID before retrying.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "typed cleanup history is integrated by the later planner/executor slices"
        )
    )]
    pub(crate) fn record_cleanup_session_planned(
        &self,
        session: &NewCleanupSessionRecord,
    ) -> Result<(), HistoryError> {
        let prepared = PreparedCleanupSession::prepare(session)?;
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        insert_cleanup_session(&transaction, &prepared)?;
        transaction.commit().map_err(map_write_sql_error)?;
        self.paths
            .repair_sqlite_sidecars()
            .and_then(|()| self.paths.validate_all_existing())
            .map_err(map_history_database_error)
    }

    /// Load an explicit legacy summary or one complete planned observation.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "typed cleanup history is integrated by the later planner/executor slices"
        )
    )]
    pub(crate) fn load_cleanup_session(
        &self,
        id: &CleanupSessionId,
    ) -> Result<Option<StoredCleanupSessionRecord>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_cleanup_session_record(&guard.connection, id)
    }

    /// Return a bounded, path-free page ordered newest first. The ordinary
    /// writer/compatibility guard is held across every scalar child query; no
    /// cleanup OS lock is acquired because this is observation only.
    pub(crate) fn recent_cleanup_history(
        &self,
        cursor: Option<&StoredCleanupHistoryCursor>,
        limit: usize,
    ) -> Result<StoredCleanupHistoryPage, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        query_recent_cleanup_history(&guard.connection, cursor, limit)
    }

    /// Load one fully validated journal graph and project it to scrubbed,
    /// path-free history. This observation carries no execution authority.
    pub(crate) fn cleanup_history_session(
        &self,
        id: &CleanupSessionId,
    ) -> Result<Option<StoredCleanupHistoryObservation>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        query_cleanup_history_session(&guard.connection, id)
    }

    /// Prepare an exact, path-free witness for all terminal cleanup history.
    /// The same cleanup exclusion used by journal effects prevents lifecycle
    /// transitions while the complete graph is validated and fingerprinted.
    pub(crate) fn prepare_cleanup_history_clear(
        &self,
    ) -> Result<PreparedCleanupHistoryClear, CleanupHistoryClearStoreError> {
        let _cleanup_lock = self
            .acquire_cleanup_lock_for_journal(MIGRATION_LOCK_TIMEOUT)
            .map_err(CleanupHistoryClearStoreError::History)?;
        let guard = self
            .lock_current_history_connection()
            .map_err(CleanupHistoryClearStoreError::History)?;
        prepare_cleanup_history_clear(&guard.connection)
    }

    /// Consume one previously prepared witness and clear the exact unchanged
    /// history graph. This operation accepts no caller-selected row or path.
    pub(crate) fn clear_cleanup_history(
        &self,
        prepared: &PreparedCleanupHistoryClear,
    ) -> Result<CleanupHistoryClearResult, CleanupHistoryClearStoreError> {
        self.clear_cleanup_history_with_hook(prepared, || Ok(()))
    }

    fn clear_cleanup_history_with_hook(
        &self,
        prepared: &PreparedCleanupHistoryClear,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<CleanupHistoryClearResult, CleanupHistoryClearStoreError> {
        self.clear_cleanup_history_with_hooks(
            prepared,
            after_commit,
            reconcile_cleanup_history_clear,
        )
    }

    fn clear_cleanup_history_with_hooks(
        &self,
        prepared: &PreparedCleanupHistoryClear,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
        reconcile: impl FnOnce(
            &Connection,
            &super::cleanup_history_clear::CleanupHistoryClearWitness,
        ) -> Result<CleanupHistoryClearReconciliation, HistoryError>,
    ) -> Result<CleanupHistoryClearResult, CleanupHistoryClearStoreError> {
        // Lock order is cleanup exclusion -> history connection/writer lease,
        // matching journal admission and preventing any lifecycle change
        // between witness comparison and commit/reconciliation.
        let _cleanup_lock = self
            .acquire_cleanup_lock_for_journal(MIGRATION_LOCK_TIMEOUT)
            .map_err(CleanupHistoryClearStoreError::History)?;
        let mut guard = self
            .lock_current_history_connection()
            .map_err(CleanupHistoryClearStoreError::History)?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)
            .map_err(CleanupHistoryClearStoreError::History)?;
        let result = apply_cleanup_history_clear(&transaction, prepared.witness())?;
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => return Ok(result),
            Err(failure) => failure,
        };

        if self.revalidate_current_history_guard(&guard).is_err() {
            return Err(CleanupHistoryClearStoreError::History(HistoryError::new(
                HistoryErrorKind::OutcomeUnknown,
            )));
        }
        match reconcile(&guard.connection, prepared.witness()) {
            Ok(CleanupHistoryClearReconciliation::Applied) => Ok(result),
            Ok(CleanupHistoryClearReconciliation::NotApplied) => {
                Err(CleanupHistoryClearStoreError::History(failure))
            }
            Ok(CleanupHistoryClearReconciliation::Ambiguous) | Err(_) => {
                Err(CleanupHistoryClearStoreError::History(HistoryError::new(
                    HistoryErrorKind::OutcomeUnknown,
                )))
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn clear_cleanup_history_after_commit_failure_for_test(
        &self,
        prepared: &PreparedCleanupHistoryClear,
    ) -> Result<CleanupHistoryClearResult, CleanupHistoryClearStoreError> {
        self.clear_cleanup_history_with_hook(prepared, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    #[cfg(test)]
    pub(crate) fn clear_cleanup_history_after_reconciliation_failure_for_test(
        &self,
        prepared: &PreparedCleanupHistoryClear,
    ) -> Result<CleanupHistoryClearResult, CleanupHistoryClearStoreError> {
        self.clear_cleanup_history_with_hooks(
            prepared,
            || Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable)),
            |_, _| Err(HistoryError::new(HistoryErrorKind::CorruptData)),
        )
    }

    /// Acquire the store-wide cleanup exclusion before any journal connection
    /// or writer lease. This remains an exclusion primitive only; the journal
    /// layer must separately bind a typed owner and generation.
    pub(super) fn acquire_cleanup_lock_for_journal(
        &self,
        timeout: Duration,
    ) -> Result<CleanupLockGuard, HistoryError> {
        self.paths
            .acquire_cleanup_lock(timeout)
            .map_err(map_history_database_error)
    }

    fn acquire_cleanup_lock_for_journal_until(
        &self,
        deadline: Instant,
    ) -> Result<CleanupLockGuard, HistoryError> {
        self.paths
            .acquire_cleanup_lock_until(deadline)
            .map_err(map_history_database_error)
    }

    /// Acquire and retain every database-side exclusion needed before an
    /// application-data reset may hand off to exact namespace witnesses.
    ///
    /// Cleanup lock contention is a normal path-free blocker. Unsafe,
    /// incompatible, corrupt, or unavailable storage remains a typed failure
    /// and must never be softened into an admission result.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the namespace-handoff slice consumes retained reset admission"
        )
    )]
    pub(super) fn begin_app_data_reset_store_admission(
        &self,
    ) -> Result<AppDataResetStoreAdmission<'_>, HistoryError> {
        self.begin_app_data_reset_store_admission_with_timeout(MIGRATION_LOCK_TIMEOUT)
    }

    pub(super) fn begin_app_data_reset_store_admission_with_timeout(
        &self,
        cleanup_timeout: Duration,
    ) -> Result<AppDataResetStoreAdmission<'_>, HistoryError> {
        let deadline = Instant::now()
            .checked_add(cleanup_timeout)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        self.begin_app_data_reset_store_admission_until(deadline)
    }

    pub(super) fn begin_app_data_reset_store_admission_until(
        &self,
        deadline: Instant,
    ) -> Result<AppDataResetStoreAdmission<'_>, HistoryError> {
        let cleanup = match self.acquire_cleanup_lock_for_journal_until(deadline) {
            Ok(cleanup) => cleanup,
            Err(error) if error.kind == HistoryErrorKind::Busy => {
                return Ok(AppDataResetStoreAdmission::Blocked(
                    AppDataResetStoreBlockers::cleanup_lock_busy(),
                ));
            }
            Err(error) => return Err(error),
        };
        self.validate_cleanup_lock_for_journal(&cleanup)?;
        let history = self.lock_current_history_connection_until(deadline)?;
        self.validate_cleanup_lock_for_journal(&cleanup)?;
        let blockers =
            inspect_app_data_reset_store_blockers(&history.connection, SystemTime::now())?;
        self.revalidate_current_history_guard(&history)?;
        self.validate_cleanup_lock_for_journal(&cleanup)?;
        if !blockers.is_empty() {
            return Ok(AppDataResetStoreAdmission::Blocked(blockers));
        }
        Ok(AppDataResetStoreAdmission::Admitted(
            AppDataResetStoreGuard {
                history,
                cleanup,
                store: self,
            },
        ))
    }

    #[cfg(test)]
    pub(super) fn begin_app_data_reset_store_admission_with_timeout_for_test(
        &self,
        cleanup_timeout: Duration,
    ) -> Result<AppDataResetStoreAdmission<'_>, HistoryError> {
        self.begin_app_data_reset_store_admission_with_timeout(cleanup_timeout)
    }

    /// Revalidate a held cleanup control without granting target or effect
    /// authority. Journal callers invoke this before every short transaction.
    pub(super) fn validate_cleanup_lock_for_journal(
        &self,
        guard: &CleanupLockGuard,
    ) -> Result<(), HistoryError> {
        self.paths
            .validate_cleanup_lock_guard(guard)
            .map_err(map_history_database_error)
    }

    /// Repair and revalidate SQLite sidecars after a committed journal write.
    /// A failure here makes the commit outcome ambiguous to the caller.
    pub(super) fn validate_history_storage_after_write(&self) -> Result<(), HistoryError> {
        self.paths
            .repair_sqlite_sidecars()
            .and_then(|()| self.paths.validate_all_existing())
            .map_err(map_history_database_error)
    }

    pub(super) fn revalidate_current_history_guard(
        &self,
        guard: &HistoryConnectionGuard<'_>,
    ) -> Result<(), HistoryError> {
        self.validate_history_guard(guard)?;
        self.validate_history_storage_after_write()?;
        match inspect_schema_for_status(&guard.connection).map_err(map_history_database_error)? {
            SchemaState::Current => Ok(()),
            SchemaState::Newer { .. } => {
                Err(HistoryError::new(HistoryErrorKind::IncompatibleSchema))
            }
            SchemaState::Empty | SchemaState::Older { .. } => {
                Err(HistoryError::new(HistoryErrorKind::CorruptData))
            }
        }
    }

    pub(super) fn lock_current_history_connection(
        &self,
    ) -> Result<HistoryConnectionGuard<'_>, HistoryError> {
        let deadline = Instant::now()
            .checked_add(MIGRATION_LOCK_TIMEOUT)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        self.lock_current_history_connection_until(deadline)
    }

    fn lock_current_history_connection_until(
        &self,
        deadline: Instant,
    ) -> Result<HistoryConnectionGuard<'_>, HistoryError> {
        let connection = loop {
            if Instant::now() >= deadline {
                return Err(HistoryError::new(HistoryErrorKind::Busy));
            }
            match self.connection.try_lock() {
                Ok(_connection) if Instant::now() >= deadline => {
                    return Err(HistoryError::new(HistoryErrorKind::Busy));
                }
                Ok(connection) => break connection,
                Err(TryLockError::WouldBlock) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(HistoryError::new(HistoryErrorKind::Busy));
                    }
                    std::thread::sleep(RESET_LOCK_RETRY_INTERVAL.min(deadline.duration_since(now)));
                }
                Err(TryLockError::Poisoned(_)) => {
                    return Err(HistoryError::new(HistoryErrorKind::InternalState));
                }
            }
        };
        let writer_lock = self
            .paths
            .acquire_writer_lock_until(deadline)
            .map_err(map_history_database_error)?;
        self.paths
            .repair_sqlite_sidecars()
            .and_then(|()| self.paths.validate_all_existing())
            .map_err(map_history_database_error)?;
        let schema = inspect_schema_for_status(&connection).map_err(map_history_database_error)?;
        match schema {
            SchemaState::Current
                if matches!(
                    self.cached_status().access,
                    DatabaseAccess::ReadWriteCurrent
                ) =>
            {
                Ok(HistoryConnectionGuard {
                    _writer_lock: writer_lock,
                    connection,
                    store_identity: self.paths.identity(),
                })
            }
            SchemaState::Newer { .. } => {
                Err(HistoryError::new(HistoryErrorKind::IncompatibleSchema))
            }
            SchemaState::Current => Err(HistoryError::new(HistoryErrorKind::InternalState)),
            SchemaState::Empty | SchemaState::Older { .. } => {
                Err(HistoryError::new(HistoryErrorKind::CorruptData))
            }
        }
    }

    pub(super) fn validate_history_guard(
        &self,
        guard: &HistoryConnectionGuard<'_>,
    ) -> Result<(), HistoryError> {
        if guard.store_identity != self.paths.identity() {
            return Err(HistoryError::new(HistoryErrorKind::InternalState));
        }
        Ok(())
    }

    pub(super) fn inspect_owned_database_footprint_with_guard(
        &self,
        guard: &HistoryConnectionGuard<'_>,
        observed_at: SystemTime,
    ) -> Result<(OwnedStorageUsage, AiCacheFootprint), HistoryError> {
        self.validate_history_guard(guard)?;
        let embedded_ai_cache = inspect_ai_cache_footprint(&guard.connection, observed_at)?;
        let physical = self
            .paths
            .observe_physical_usage(&guard._writer_lock)
            .map_err(map_history_database_error)?;
        self.validate_history_guard(guard)?;
        Ok((physical, embedded_ai_cache))
    }

    fn cached_status(&self) -> DatabaseStatus {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(super) fn open_unregistered_for_test(
        paths: SecureStorePaths,
        sqlite_path: &Path,
        between_probe_and_lock: impl FnOnce() -> Result<(), DatabaseOpenError>,
    ) -> Result<Self, DatabaseOpenError> {
        Self::open_unregistered_with_hook(paths, sqlite_path, between_probe_and_lock)
    }

    #[cfg(test)]
    pub(crate) fn with_connection<T>(&self, inspect: impl FnOnce(&Connection) -> T) -> T {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inspect(&connection)
    }

    #[cfg(test)]
    pub(crate) fn with_current_history_guard_for_test<T>(
        &self,
        inspect: impl FnOnce() -> T,
    ) -> Result<T, HistoryError> {
        let _guard = self.lock_current_history_connection()?;
        Ok(inspect())
    }
}

pub(super) fn map_history_database_error(error: DatabaseOpenError) -> HistoryError {
    let kind = match error.kind {
        DatabaseOpenErrorKind::Busy => HistoryErrorKind::Busy,
        DatabaseOpenErrorKind::CorruptDatabase | DatabaseOpenErrorKind::UnrecognizedDatabase => {
            HistoryErrorKind::CorruptData
        }
        DatabaseOpenErrorKind::OwnershipMismatch
        | DatabaseOpenErrorKind::UnsafeStorageRoot
        | DatabaseOpenErrorKind::UnsafeStorageObject
        | DatabaseOpenErrorKind::UnsafePermissions => HistoryErrorKind::UnsafeStorage,
        DatabaseOpenErrorKind::InternalState => HistoryErrorKind::InternalState,
        DatabaseOpenErrorKind::InspectionLimitExceeded => HistoryErrorKind::QueryLimitExceeded,
        DatabaseOpenErrorKind::StorageRootUnavailable
        | DatabaseOpenErrorKind::DatabaseUnavailable
        | DatabaseOpenErrorKind::MigrationFailed => HistoryErrorKind::DatabaseUnavailable,
    };
    HistoryError::new(kind)
}

fn open_connection(path: &Path, read_only: bool) -> Result<Connection, DatabaseOpenError> {
    let access = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    Connection::open_with_flags(
        path,
        access
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(map_open_error)
}

fn configure_connection(connection: &Connection, read_only: bool) -> Result<(), DatabaseOpenError> {
    connection
        .busy_timeout(DATABASE_BUSY_TIMEOUT)
        .map_err(map_configuration_error)?;

    for (limit, value) in [
        (Limit::SQLITE_LIMIT_LENGTH, 32 * 1024 * 1024),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, 1024 * 1024),
        (Limit::SQLITE_LIMIT_COLUMN, 128),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 256),
        (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 16),
        (Limit::SQLITE_LIMIT_FUNCTION_ARG, 64),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
        (Limit::SQLITE_LIMIT_LIKE_PATTERN_LENGTH, 4096),
        (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 1024),
        (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 0),
        (Limit::SQLITE_LIMIT_WORKER_THREADS, 2),
    ] {
        connection
            .set_limit(limit, value)
            .map_err(map_configuration_error)?;
    }

    for (config, enabled) in [
        (DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false),
        (DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true),
        (DbConfig::SQLITE_DBCONFIG_WRITABLE_SCHEMA, false),
        (DbConfig::SQLITE_DBCONFIG_DQS_DML, false),
        (DbConfig::SQLITE_DBCONFIG_DQS_DDL, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false),
        (DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_COMMENTS, false),
    ] {
        let actual = connection
            .set_db_config(config, enabled)
            .map_err(map_configuration_error)?;
        if actual != enabled {
            return Err(DatabaseOpenError::new(
                DatabaseOpenErrorKind::DatabaseUnavailable,
            ));
        }
    }

    connection
        .pragma_update(None, "foreign_keys", true)
        .and_then(|()| connection.pragma_update(None, "query_only", read_only))
        .and_then(|()| connection.pragma_update(None, "cell_size_check", true))
        .and_then(|()| connection.pragma_update(None, "mmap_size", 0_i64))
        .and_then(|()| connection.pragma_update(None, "temp_store", "MEMORY"))
        .and_then(|()| connection.pragma_update(None, "locking_mode", "NORMAL"))
        .map_err(map_configuration_error)?;
    if !read_only {
        configure_full_synchronous(connection)?;
    }
    Ok(())
}

/// Keep trigger programs inert while an untrusted database is inspected. Only
/// the exact supported schema fingerprint may activate the small, migration-
/// owned trigger set; newer schemas remain on a separately configured
/// trigger-disabled read-only connection.
fn enable_verified_schema_triggers(connection: &Connection) -> Result<(), DatabaseOpenError> {
    connection
        .set_limit(Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 1)
        .map_err(map_configuration_error)?;
    let enabled = connection
        .set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)
        .map_err(map_configuration_error)?;
    if !enabled {
        return Err(DatabaseOpenError::new(
            DatabaseOpenErrorKind::DatabaseUnavailable,
        ));
    }
    Ok(())
}

fn configure_write_ahead_log(connection: &Connection) -> Result<(), DatabaseOpenError> {
    let mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(map_configuration_error)?;
    if !mode.eq_ignore_ascii_case("wal") {
        let changed: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(map_configuration_error)?;
        if !changed.eq_ignore_ascii_case("wal") {
            return Err(DatabaseOpenError::new(
                DatabaseOpenErrorKind::DatabaseUnavailable,
            ));
        }
    }
    configure_full_synchronous(connection)
}

fn configure_full_synchronous(connection: &Connection) -> Result<(), DatabaseOpenError> {
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(map_configuration_error)?;
    let synchronous: i64 = connection
        .pragma_query_value(None, "synchronous", |row| row.get(0))
        .map_err(map_configuration_error)?;
    if synchronous != 2 {
        return Err(DatabaseOpenError::new(
            DatabaseOpenErrorKind::DatabaseUnavailable,
        ));
    }
    Ok(())
}

fn map_scan_process_identity_error(error: ProcessIdentityError) -> HistoryError {
    let kind = match error {
        ProcessIdentityError::InvalidEncoding => HistoryErrorKind::InternalState,
        ProcessIdentityError::ObservationUnavailable | ProcessIdentityError::RandomUnavailable => {
            HistoryErrorKind::DatabaseUnavailable
        }
    };
    HistoryError::new(kind)
}

fn unix_time_ms() -> Result<i64, DatabaseOpenError> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::MigrationFailed))?
        .as_millis();
    i64::try_from(milliseconds)
        .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::MigrationFailed))
}

fn map_open_error(error: rusqlite::Error) -> DatabaseOpenError {
    use rusqlite::ErrorCode;
    let kind = match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => DatabaseOpenErrorKind::Busy,
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => {
            DatabaseOpenErrorKind::CorruptDatabase
        }
        _ => DatabaseOpenErrorKind::DatabaseUnavailable,
    };
    DatabaseOpenError::new(kind)
}

fn map_configuration_error(error: rusqlite::Error) -> DatabaseOpenError {
    use rusqlite::ErrorCode;
    let kind = match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => DatabaseOpenErrorKind::Busy,
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => {
            DatabaseOpenErrorKind::CorruptDatabase
        }
        _ => DatabaseOpenErrorKind::DatabaseUnavailable,
    };
    DatabaseOpenError::new(kind)
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod app_data_reset_data_namespace_tests {
    use tempfile::TempDir;

    use super::*;

    fn open_store(temp: &TempDir, name: &str) -> Arc<StoreCoordinator> {
        StoreCoordinator::open(&temp.path().join(name).join("dux.sqlite3")).unwrap()
    }

    fn transaction() -> AppDataResetTransaction {
        AppDataResetTransaction::for_test("00112233445566778899aabbccddeeff").unwrap()
    }

    #[test]
    fn data_namespace_witness_accepts_its_issuing_store_guard() {
        let temp = TempDir::new().unwrap();
        let store = open_store(&temp, "owned");
        let deadline = Instant::now() + Duration::from_secs(1);

        let result = store
            .with_app_data_reset_data_namespace_admission_until(
                &transaction(),
                deadline,
                |namespace| {
                    let guard = match store
                        .begin_app_data_reset_store_admission_until(deadline)
                        .unwrap()
                    {
                        AppDataResetStoreAdmission::Admitted(guard) => guard,
                        AppDataResetStoreAdmission::Blocked(_) => {
                            panic!("fresh store must admit reset validation")
                        }
                    };
                    namespace.revalidate(&guard)
                },
            )
            .unwrap();

        result.unwrap();
    }

    #[test]
    fn data_namespace_witness_rejects_a_different_store_guard() {
        let temp = TempDir::new().unwrap();
        let issuing_store = open_store(&temp, "owned-a");
        let other_store = open_store(&temp, "owned-b");
        let deadline = Instant::now() + Duration::from_secs(1);

        let error = issuing_store
            .with_app_data_reset_data_namespace_admission_until(
                &transaction(),
                deadline,
                |namespace| {
                    let guard = match other_store
                        .begin_app_data_reset_store_admission_until(deadline)
                        .unwrap()
                    {
                        AppDataResetStoreAdmission::Admitted(guard) => guard,
                        AppDataResetStoreAdmission::Blocked(_) => {
                            panic!("independent fresh store must admit reset validation")
                        }
                    };
                    namespace.revalidate(&guard).unwrap_err()
                },
            )
            .unwrap();

        assert_eq!(error.kind, HistoryErrorKind::InternalState);
    }

    #[test]
    fn recovery_wrapper_classifies_and_rolls_forward_without_opening_a_coordinator() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("owned").join("dux.sqlite3");
        let store = StoreCoordinator::open(&database).unwrap();
        let transaction = transaction();
        let deadline = Instant::now() + Duration::from_secs(1);
        let identity = store
            .with_app_data_reset_data_namespace_admission_until(
                &transaction,
                deadline,
                |namespace| namespace.journal_identity().unwrap(),
            )
            .unwrap();
        let snapshots = super::super::snapshot::storage::SecureSnapshotStore::open_for_database(
            &database,
            super::super::snapshot::storage::SnapshotStoreAccess::ReadWrite,
        )
        .unwrap()
        .unwrap();
        drop(store);

        StoreCoordinator::with_app_data_reset_recovery_data_namespace_until(
            &database,
            &transaction,
            identity,
            Instant::now() + Duration::from_secs(1),
            |namespace| {
                assert_eq!(
                    namespace.location(),
                    AppDataResetRecoveryDataLocation::Canonical
                );
                assert_eq!(namespace.journal_identity().unwrap(), identity);
                let namespace = namespace
                    .detach_if_canonical(identity, transaction.data_stage().as_str())
                    .unwrap();
                assert_eq!(
                    namespace.location(),
                    AppDataResetRecoveryDataLocation::Detached
                );
                namespace.revalidate().unwrap();
            },
        )
        .unwrap();
        drop(snapshots);
    }
}
