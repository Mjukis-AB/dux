use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::config::DbConfig;
use rusqlite::limits::Limit;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use super::candidate_history::{
    CandidateEvaluationTransition, CandidateHistoryStatus, CandidateReviewTransition,
    NewCandidateRecord, PreparedCandidate, StoredCandidateRecord, insert_candidate,
    load_candidate_record, transition_candidate_evaluation, transition_candidate_review,
};
use super::capacity_history::{
    CapacityPage, CapacityPageCursor, CapacityWriteOutcome, CapacityWriteReason,
    PreparedCapacitySample, RawCapacitySample, StoredCapacitySample, exact_raw_and_volume_match,
    exact_volume_observation_match, load_latest_raw_capacity_sample, load_raw_capacity_page,
    validate_capacity_volume, write_raw_capacity_sample,
};
use super::cleanup_history::{
    CleanupSessionId, NewCleanupSessionRecord, PreparedCleanupSession, StoredCleanupSessionRecord,
    insert_cleanup_session, load_cleanup_session_record,
};
use super::history::{
    HistoryError, HistoryErrorKind, NewScanRecord, PreparedNewScan, PreparedScanCompletion,
    ScanCompletionRecord, ScanRecord, insert_scan_started, load_scan_record, map_write_sql_error,
    update_scan_finished,
};
use super::migrations::{
    SchemaState, apply_pending_migrations, inspect_schema, inspect_schema_for_status,
};
use super::status::{
    DATABASE_SCHEMA_VERSION, DatabaseAccess, DatabaseOpenError, DatabaseOpenErrorKind,
    DatabaseStatus,
};
use super::storage::{CleanupLockGuard, SecureStorePaths, StoreIdentity, WriterLockGuard};

const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

static COORDINATORS: OnceLock<Mutex<HashMap<StoreIdentity, Weak<StoreCoordinator>>>> =
    OnceLock::new();

/// One serialized SQLite owner per physical store and process.
pub(crate) struct StoreCoordinator {
    status: Mutex<DatabaseStatus>,
    paths: SecureStorePaths,
    connection: Mutex<Connection>,
}

pub(super) struct HistoryConnectionGuard<'a> {
    // Struct fields drop in declaration order: release the cross-process lease
    // before another in-process caller can acquire the connection mutex.
    _writer_lock: WriterLockGuard,
    pub(super) connection: MutexGuard<'a, Connection>,
    store_identity: StoreIdentity,
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
        })
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
    pub(super) fn validated_database_path(&self) -> Result<PathBuf, HistoryError> {
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
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        insert_scan_started(&transaction, &prepared)?;
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
        match self.load_scan(scan.id()) {
            Ok(Some(record)) if record.exactly_matches_start(scan) => Ok(()),
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
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
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
        match load_scan_record(&guard.connection, completion.id()) {
            Ok(Some(record)) if record.exactly_matches_completion(completion) => Ok(()),
            Ok(Some(record)) if record.status() == super::history::ScanStatus::Running => {
                Err(failure)
            }
            Ok(Some(_)) => Err(HistoryError::new(HistoryErrorKind::InvalidTransition)),
            Ok(None) => Err(HistoryError::new(HistoryErrorKind::NotFound)),
            Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
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
        let mut guard = self.lock_current_history_connection()?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
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

    fn revalidate_current_history_guard(
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
        let connection = self
            .connection
            .lock()
            .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?;
        let writer_lock = self
            .paths
            .acquire_writer_lock(MIGRATION_LOCK_TIMEOUT)
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
    pub(super) fn with_connection<T>(&self, inspect: impl FnOnce(&Connection) -> T) -> T {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inspect(&connection)
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
