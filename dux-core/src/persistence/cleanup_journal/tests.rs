use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, types::Value};
use tempfile::TempDir;

use crate::cleanup::executor::{
    TrashAdmissionError, TrashAdmissionStartError, TrashExecutionAdmission, TrashExecutionError,
    TrashPlatformEffect, TrashPlatformError,
};
use crate::engine::SnapshotReviewTrashTarget;
use crate::path_validation::{
    capture_scan_root, capture_trash_path_snapshot, validate_cleanup_path, validate_scan_root,
};

use super::lease::*;
use super::*;
use crate::domain::{
    Candidate, CandidateCategory, CandidateInput, CleanupPlan, LocalizedTextKey, ProvenanceUrl,
    Rule, RuleDefinition, RuleGuards, RuleMatcher, RuleMatcherDefinition, RuleScope,
};
use crate::persistence::candidate_history::{
    CandidateEvaluationTransition, CandidateHistoryStatus, CandidateReviewTransition,
    NewCandidateRecord, StoredCandidateRecord,
};
use crate::persistence::cleanup_history::{
    CleanupSessionId, CleanupTrigger, NewCleanupSessionRecord,
};
use crate::persistence::history::{
    HistoryErrorKind, NewScanRecord, ScanCompletionRecord, ScanCounts, TerminalScanStatus,
};
use crate::persistence::{
    MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS, MAX_STORAGE_THIEF_SOURCE_SESSIONS, StoreCoordinator,
};

const PLAN_CREATED_SECONDS: u64 = 1_750_000_010;
const SESSION_STARTED_MILLIS: u64 = 1_750_000_011_123;
const LOCK_TIMEOUT: Duration = Duration::from_secs(2);

struct Fixture {
    _temp: TempDir,
    database: PathBuf,
    store: Arc<StoreCoordinator>,
    session_id: CleanupSessionId,
    started_at: SystemTime,
    expires_at: SystemTime,
    plan: CleanupPlan,
    rule: Rule,
    source_scan_id: ScanId,
}

impl Fixture {
    fn new(mode: CleanupMode, action: CandidateAction, item_count: usize) -> Self {
        Self::new_with_selected(mode, action, item_count, &[])
    }

    fn new_with_selected(
        mode: CleanupMode,
        action: CandidateAction,
        item_count: usize,
        selected: &[usize],
    ) -> Self {
        let temp = TempDir::new().unwrap();
        Self::new_with_selected_in_temp(temp, mode, action, item_count, selected)
    }

    fn new_with_selected_in_current_dir(
        mode: CleanupMode,
        action: CandidateAction,
        item_count: usize,
        selected: &[usize],
    ) -> Self {
        let temp = TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        Self::new_with_selected_in_temp(temp, mode, action, item_count, selected)
    }

    fn new_with_selected_in_temp(
        temp: TempDir,
        mode: CleanupMode,
        action: CandidateAction,
        item_count: usize,
        selected: &[usize],
    ) -> Self {
        Self::new_with_selected_in_temp_and_record(temp, mode, action, item_count, selected, true)
    }

    fn new_unrecorded(mode: CleanupMode, action: CandidateAction, item_count: usize) -> Self {
        Self::new_with_selected_in_temp_and_record(
            TempDir::new().unwrap(),
            mode,
            action,
            item_count,
            &[],
            false,
        )
    }

    fn new_with_selected_in_temp_and_record(
        temp: TempDir,
        mode: CleanupMode,
        action: CandidateAction,
        item_count: usize,
        selected: &[usize],
        record_plan: bool,
    ) -> Self {
        Self::new_with_rule_and_scan_id(
            temp,
            mode,
            item_count,
            selected,
            record_plan,
            fixture_rule(action),
            ScanId::new("scan:cleanup-journal").unwrap(),
        )
    }

    fn new_automation_suggestion() -> Self {
        Self::new_with_rule_and_scan_id(
            TempDir::new().unwrap(),
            CleanupMode::PermanentSafe,
            1,
            &[],
            true,
            automation_suggestion_fixture_rule(),
            ScanId::new(format!(
                "{}fixture",
                crate::domain::KNOWN_USER_CACHE_SCAN_ID_PREFIX
            ))
            .unwrap(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_rule_and_scan_id(
        temp: TempDir,
        mode: CleanupMode,
        item_count: usize,
        selected: &[usize],
        record_plan: bool,
        rule: Rule,
        source_scan_id: ScanId,
    ) -> Self {
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let store = StoreCoordinator::open(&database).unwrap();
        if mode == CleanupMode::PermanentSafe {
            store
                .set_permanent_cleanup_enabled(true)
                .expect("effect-path fixture must opt in explicitly");
        }
        start_scan(&store, &root, source_scan_id.as_str());

        let candidates = (0..item_count)
            .map(|index| {
                fixture_candidate(
                    &format!("candidate:journal-{index}"),
                    source_scan_id.as_str(),
                    &rule,
                    root.join(format!("cleanup-fixture-{index}")),
                    100 + index as u64,
                )
            })
            .collect::<Vec<_>>();
        for (index, candidate) in candidates.iter().enumerate() {
            store
                .record_candidate_discovered(
                    &NewCandidateRecord::try_from_candidate(
                        candidate,
                        UNIX_EPOCH + Duration::from_secs(1_750_000_002 + index as u64),
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        for index in selected {
            store
                .transition_candidate_review_status(
                    candidates[*index].id(),
                    CandidateReviewTransition::Select,
                )
                .unwrap();
        }
        let plan = CleanupPlan::try_from_candidates_for_persistence_test(
            CleanupPlanId::new("plan:cleanup-journal").unwrap(),
            UNIX_EPOCH + Duration::from_secs(PLAN_CREATED_SECONDS) + Duration::from_nanos(456_789),
            mode,
            &candidates,
        )
        .unwrap();
        let expires_at = plan.expires_at();
        let started_at = UNIX_EPOCH + Duration::from_millis(SESSION_STARTED_MILLIS);
        let session_id = CleanupSessionId::new("session:cleanup-journal").unwrap();
        if record_plan {
            store
                .record_cleanup_session_planned(
                    &NewCleanupSessionRecord::try_from_plan(
                        session_id.clone(),
                        &plan,
                        started_at,
                        CleanupTrigger::Manual,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        Self {
            _temp: temp,
            database,
            store,
            session_id,
            started_at,
            expires_at,
            plan,
            rule,
            source_scan_id,
        }
    }

    fn lease(&self) -> CleanupJournalLease {
        self.store
            .acquire_cleanup_journal_lease(LOCK_TIMEOUT)
            .unwrap()
    }

    fn claim(&self) -> CleanupJournalClaim {
        self.lease()
            .claim_planned(&self.session_id, self.started_at + Duration::from_secs(1))
            .unwrap()
    }

    fn execute(&self, sql: &str, values: impl rusqlite::Params) {
        let connection = self.store.lock_current_history_connection().unwrap();
        connection.connection.execute(sql, values).unwrap();
    }

    fn candidate_status(&self, index: usize) -> CandidateHistoryStatus {
        let id = CandidateId::new(format!("candidate:journal-{index}")).unwrap();
        let Some(StoredCandidateRecord::Complete(candidate)) =
            self.store.load_candidate(&id).unwrap()
        else {
            panic!("candidate was not a complete record");
        };
        candidate.status
    }

    fn claim_count(&self) -> i64 {
        self.store.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT COUNT(*) FROM candidate_plan_claims WHERE session_id = ?1",
                    [self.session_id.as_str()],
                    |row| row.get(0),
                )
                .unwrap()
        })
    }

    fn validate_scalar_state(&self) -> Result<(), HistoryError> {
        self.store.with_connection(|connection| {
            validate_cleanup_journal_scalar_state_within_budget(connection, &self.session_id)
        })
    }

    fn make_legacy_uncoupled(&self, candidate_status: &str) {
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction().unwrap();
            transaction
                .execute(
                    "UPDATE candidates SET status = ?2
                     WHERE candidate_id IN (
                         SELECT candidate_id FROM cleanup_items WHERE session_id = ?1
                     )",
                    params![self.session_id.as_str(), candidate_status],
                )
                .unwrap();
            transaction
                .execute(
                    "DELETE FROM candidate_plan_claims WHERE session_id = ?1",
                    [self.session_id.as_str()],
                )
                .unwrap();
            transaction
                .execute(
                    "UPDATE cleanup_sessions
                     SET candidate_status_coupling_version = 1
                     WHERE session_id = ?1",
                    [self.session_id.as_str()],
                )
                .unwrap();
            transaction.commit().unwrap();
        });
    }

    fn enable_outcome_source(&self) {
        use crate::persistence::snapshot::SnapshotFileName;

        let scan_id = self.source_scan_id.clone();
        let root = self._temp.path().join("root");
        let scheduled_at = UNIX_EPOCH + Duration::from_millis(1_750_000_001_500);
        let context_digest = crate::domain::candidate_evaluation_context_digest_for_observation(
            &scan_id,
            &root,
            &crate::domain::ScanCoverage::from_validated_terminal_issues(Vec::new()),
            scheduled_at,
        );
        let snapshot_name = SnapshotFileName::from_scan_id(scan_id.as_str().as_bytes());
        let encoded_snapshot =
            crate::persistence::codec::encode_host_path(Path::new(snapshot_name.as_str())).unwrap();
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction().unwrap();
            transaction
                .execute(
                    "UPDATE scans
                     SET snapshot_version = 1, snapshot_relative_path = ?2,
                         snapshot_relative_path_encoding = ?3,
                         snapshot_checksum_sha256 = ?4,
                         coverage_status = 'complete', coverage_permille = 1000,
                         root_identity_v1_sha256 = ?5
                     WHERE scan_id = ?1",
                    params![
                        scan_id.as_str(),
                        encoded_snapshot.bytes,
                        encoded_snapshot.encoding as i64,
                        [3_u8; 32].as_slice(),
                        [7_u8; 32].as_slice(),
                    ],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO candidate_evaluations (
                         scan_id, record_format_version, evaluator_revision,
                         rule_catalog_schema_version, rule_catalog_sha256,
                         context_format_version, context_sha256, snapshot_version,
                         snapshot_sha256, scheduled_at_unix_ms, completed_at_unix_ms,
                         status, candidate_count, failure_kind
                     ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, 1, ?7,
                               1750000001500, 1750000002000, 'succeeded', 1, NULL)",
                    params![
                        scan_id.as_str(),
                        crate::domain::CANDIDATE_EVALUATOR_REVISION,
                        crate::domain::CANDIDATE_CATALOG_SCHEMA_VERSION,
                        crate::domain::CANDIDATE_CATALOG_SHA256.as_slice(),
                        crate::domain::CANDIDATE_CONTEXT_FORMAT_VERSION,
                        context_digest.as_slice(),
                        [3_u8; 32].as_slice(),
                    ],
                )
                .unwrap();
            transaction.commit().unwrap();
        });
    }

    fn complete_removed(&self) -> SystemTime {
        let mut claim = self.claim();
        claim.begin_validation(0, 0).unwrap();
        let effect_started = self.started_at + Duration::from_secs(2);
        let receipt = claim.mark_effect_started(0, 0, effect_started).unwrap();
        let completed = self.started_at + Duration::from_secs(3);
        claim
            .finish_effect(&receipt, EffectOutcome::Removed, None, completed)
            .unwrap();
        assert_eq!(
            claim
                .terminalize(self.started_at + Duration::from_secs(4), None)
                .unwrap(),
            TerminalSessionStatus::Completed
        );
        completed
    }

    fn insert_outcome_followup(
        &self,
        suffix: &str,
        started_at: SystemTime,
        completed_at: SystemTime,
        observed_bytes: Option<u64>,
    ) {
        use crate::persistence::snapshot::SnapshotFileName;

        let scan_id = if self
            .source_scan_id
            .as_str()
            .starts_with(crate::domain::KNOWN_USER_CACHE_SCAN_ID_PREFIX)
        {
            ScanId::new(format!(
                "{}outcome:{suffix}",
                crate::domain::KNOWN_USER_CACHE_SCAN_ID_PREFIX
            ))
            .unwrap()
        } else {
            ScanId::new(format!("scan:outcome:{suffix}")).unwrap()
        };
        let root = self._temp.path().join("root");
        let encoded_root = crate::persistence::codec::encode_host_path(&root).unwrap();
        let snapshot_name = SnapshotFileName::from_scan_id(scan_id.as_str().as_bytes());
        let encoded_snapshot =
            crate::persistence::codec::encode_host_path(Path::new(snapshot_name.as_str())).unwrap();
        let started_ms = started_at.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let completed_ms = completed_at.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let snapshot_digest = [suffix.as_bytes()[0]; 32];
        let context_digest = crate::domain::candidate_evaluation_context_digest_for_observation(
            &scan_id,
            &root,
            &crate::domain::ScanCoverage::from_validated_terminal_issues(Vec::new()),
            completed_at,
        );
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction().unwrap();
            transaction
                .execute(
                    "INSERT INTO scans (
                         scan_id, root_path, root_path_encoding, started_at_unix_ms,
                         completed_at_unix_ms, status, snapshot_version,
                         snapshot_relative_path, snapshot_relative_path_encoding,
                         snapshot_checksum_sha256, coverage_status, coverage_permille,
                         root_identity_v1_sha256
                     ) VALUES (?1, ?2, ?3, ?4, ?5, 'succeeded', 1, ?6, ?7, ?8,
                               'complete', 1000, ?9)",
                    params![
                        scan_id.as_str(),
                        encoded_root.bytes,
                        encoded_root.encoding as i64,
                        started_ms,
                        completed_ms,
                        encoded_snapshot.bytes,
                        encoded_snapshot.encoding as i64,
                        snapshot_digest.as_slice(),
                        [7_u8; 32].as_slice(),
                    ],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO candidate_evaluations (
                         scan_id, record_format_version, evaluator_revision,
                         rule_catalog_schema_version, rule_catalog_sha256,
                         context_format_version, context_sha256, snapshot_version,
                         snapshot_sha256, scheduled_at_unix_ms, completed_at_unix_ms,
                         status, candidate_count, failure_kind
                     ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9,
                               'succeeded', ?10, NULL)",
                    params![
                        scan_id.as_str(),
                        crate::domain::CANDIDATE_EVALUATOR_REVISION,
                        crate::domain::CANDIDATE_CATALOG_SCHEMA_VERSION,
                        crate::domain::CANDIDATE_CATALOG_SHA256.as_slice(),
                        crate::domain::CANDIDATE_CONTEXT_FORMAT_VERSION,
                        context_digest.as_slice(),
                        snapshot_digest.as_slice(),
                        completed_ms,
                        completed_ms,
                        i64::from(observed_bytes.is_some()),
                    ],
                )
                .unwrap();
            if let Some(observed_bytes) = observed_bytes {
                let candidate_id = format!("candidate:outcome:{suffix}");
                let target = root.join("cleanup-fixture-0");
                let encoded_target = crate::persistence::codec::encode_host_path(&target).unwrap();
                transaction
                    .execute(
                        "INSERT INTO candidates (
                             candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                             estimated_bytes, created_at_unix_ms, status,
                             record_format_version, category, proposed_action,
                             rule_schedule_eligible
                         ) VALUES (?1, ?2, 'fixture.cleanup.journal', 1,
                                   'safe_regenerable', ?3, ?4, 'discovered', 2,
                                   'application_cache',
                                   'remove_known_regenerable_contents', 0)",
                        params![
                            candidate_id,
                            scan_id.as_str(),
                            i64::try_from(observed_bytes).unwrap(),
                            completed_ms,
                        ],
                    )
                    .unwrap();
                transaction
                    .execute(
                        "INSERT INTO candidate_paths (
                             candidate_id, path_ordinal, observed_path,
                             observed_path_encoding
                         ) VALUES (?1, 0, ?2, ?3)",
                        params![
                            candidate_id,
                            encoded_target.bytes,
                            encoded_target.encoding as i64,
                        ],
                    )
                    .unwrap();
                transaction
                    .execute(
                        "INSERT INTO candidate_evidence (
                             candidate_id, evidence_ordinal, evidence_kind,
                             path_value, path_value_encoding
                         ) VALUES (?1, 0, 'matched_path', ?2, ?3)",
                        params![
                            candidate_id,
                            encoded_target.bytes,
                            encoded_target.encoding as i64,
                        ],
                    )
                    .unwrap();
            }
            transaction.commit().unwrap();
        });
    }

    fn insert_active_superseding_journal(&self, session: &str, target: &Path, anchor: SystemTime) {
        let encoded_target = crate::persistence::codec::encode_host_path(target).unwrap();
        let started_ms = anchor.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64 + 10;
        let effect_ms = started_ms + 10;
        let completed_ms = effect_ms + 10;
        let heartbeat_ms = completed_ms + 10;
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction().unwrap();
            transaction
                .execute(
                    "INSERT INTO cleanup_sessions (
                         session_id, plan_id, started_at_unix_ms, completed_at_unix_ms,
                         mode, estimated_bytes, verified_capacity_delta_bytes,
                         trigger_source, status, record_format_version, source_scan_id,
                         plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                         plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                         execution_owner_id, execution_generation,
                         last_heartbeat_at_unix_ms, cancellation_requested,
                         candidate_status_coupling_version
                     )
                     SELECT ?2, ?3, ?4, NULL, mode, estimated_bytes, NULL,
                            trigger_source, 'running', 2, source_scan_id,
                            plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                            plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                            execution_owner_id, execution_generation, ?5, 0, 1
                     FROM cleanup_sessions WHERE session_id = ?1",
                    params![
                        self.session_id.as_str(),
                        session,
                        format!("plan:{session}"),
                        started_ms,
                        heartbeat_ms,
                    ],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO cleanup_items (
                         session_id, item_ordinal, rule_id, rule_revision,
                         estimated_bytes, final_status, error_category,
                         record_format_version, candidate_id, category, safety_tier,
                         proposed_action, rule_schedule_eligible,
                         newest_mtime_unix_seconds, newest_mtime_nanoseconds
                     )
                     SELECT ?2, item_ordinal, rule_id, rule_revision, estimated_bytes,
                            'removed', NULL, 2, candidate_id, category, safety_tier,
                            proposed_action, rule_schedule_eligible,
                            newest_mtime_unix_seconds, newest_mtime_nanoseconds
                     FROM cleanup_items WHERE session_id = ?1",
                    params![self.session_id.as_str(), session],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO cleanup_item_paths (
                         session_id, item_ordinal, path_ordinal, target_path,
                         target_path_encoding, attempt_generation, status,
                         error_category, effect_started_at_unix_ms,
                         completed_at_unix_ms
                     ) VALUES (?1, 0, 0, ?2, ?3, 1, 'removed', NULL, ?4, ?5)",
                    params![
                        session,
                        encoded_target.bytes,
                        encoded_target.encoding as i64,
                        effect_ms,
                        completed_ms,
                    ],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO cleanup_item_evidence
                     SELECT ?2, item_ordinal, evidence_ordinal, evidence_kind,
                            ?3, ?4, text_value,
                            observed_unix_seconds, observed_nanoseconds,
                            duration_seconds, duration_nanoseconds,
                            observed_bytes, minimum_bytes
                     FROM cleanup_item_evidence WHERE session_id = ?1",
                    params![
                        self.session_id.as_str(),
                        session,
                        encoded_target.bytes,
                        encoded_target.encoding as i64,
                    ],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO cleanup_plan_warnings
                     SELECT ?2, warning_ordinal, warning_kind
                     FROM cleanup_plan_warnings WHERE session_id = ?1",
                    params![self.session_id.as_str(), session],
                )
                .unwrap();
            transaction.commit().unwrap();
        });
    }

    fn insert_completed_clone(
        &self,
        session: &str,
        started_at: SystemTime,
        trigger: CleanupTrigger,
    ) -> SystemTime {
        let started_ms = started_at.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let effect_ms = started_ms + 1_000;
        let path_completed_ms = started_ms + 2_000;
        let session_completed_ms = started_ms + 3_000;
        let trigger = match trigger {
            CleanupTrigger::Manual => "manual",
            CleanupTrigger::LowDisk => "low_disk",
            CleanupTrigger::Scheduled => "scheduled",
            CleanupTrigger::Cli => "cli",
        };
        self.store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction().unwrap();
            transaction
                .execute(
                    "INSERT INTO cleanup_sessions (
                         session_id, plan_id, started_at_unix_ms, completed_at_unix_ms,
                         mode, estimated_bytes, verified_capacity_delta_bytes,
                         trigger_source, status, record_format_version, source_scan_id,
                         plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                         plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                         execution_owner_id, execution_generation,
                         last_heartbeat_at_unix_ms, cancellation_requested,
                         candidate_status_coupling_version
                     )
                     SELECT ?2, ?3, ?4, ?5, mode, estimated_bytes, NULL,
                            ?6, 'completed', 2, source_scan_id,
                            plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                            plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                            execution_owner_id, execution_generation, ?5, 0,
                            candidate_status_coupling_version
                     FROM cleanup_sessions WHERE session_id = ?1",
                    params![
                        self.session_id.as_str(),
                        session,
                        format!("plan:{session}"),
                        started_ms,
                        session_completed_ms,
                        trigger,
                    ],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO cleanup_items (
                         session_id, item_ordinal, rule_id, rule_revision,
                         estimated_bytes, final_status, error_category,
                         record_format_version, candidate_id, category, safety_tier,
                         proposed_action, rule_schedule_eligible,
                         newest_mtime_unix_seconds, newest_mtime_nanoseconds
                     )
                     SELECT ?2, item_ordinal, rule_id, rule_revision, estimated_bytes,
                            final_status, error_category, record_format_version, candidate_id,
                            category, safety_tier, proposed_action, rule_schedule_eligible,
                            newest_mtime_unix_seconds, newest_mtime_nanoseconds
                     FROM cleanup_items WHERE session_id = ?1",
                    params![self.session_id.as_str(), session],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO cleanup_item_paths (
                         session_id, item_ordinal, path_ordinal, target_path,
                         target_path_encoding, attempt_generation, status,
                         error_category, effect_started_at_unix_ms,
                         completed_at_unix_ms
                     )
                     SELECT ?2, item_ordinal, path_ordinal, target_path,
                            target_path_encoding, attempt_generation, status, error_category,
                            ?3, ?4
                     FROM cleanup_item_paths WHERE session_id = ?1",
                    params![
                        self.session_id.as_str(),
                        session,
                        effect_ms,
                        path_completed_ms,
                    ],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO cleanup_item_evidence
                     SELECT ?2, item_ordinal, evidence_ordinal, evidence_kind,
                            path_value, path_value_encoding, text_value,
                            observed_unix_seconds, observed_nanoseconds,
                            duration_seconds, duration_nanoseconds,
                            observed_bytes, minimum_bytes
                     FROM cleanup_item_evidence WHERE session_id = ?1",
                    params![self.session_id.as_str(), session],
                )
                .unwrap();
            transaction
                .execute(
                    "INSERT INTO cleanup_plan_warnings
                     SELECT ?2, warning_ordinal, warning_kind
                     FROM cleanup_plan_warnings WHERE session_id = ?1",
                    params![self.session_id.as_str(), session],
                )
                .unwrap();
            transaction.commit().unwrap();
        });
        UNIX_EPOCH + Duration::from_millis(path_completed_ms as u64)
    }
}

#[test]
fn rule_outcome_engine_query_derives_regrowth_ignores_legacy_rows_and_writes_no_database_bytes() {
    use crate::engine::{
        DurableCleanupSessionId, DurableRuleOutcomeState, EngineConfig, EngineHandle,
    };

    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.enable_outcome_source();
    let anchor = fixture.complete_removed();
    fixture.execute(
        "INSERT INTO rule_outcomes (
             rule_id, rule_revision, cleaned_at_unix_ms, cleaned_bytes,
             next_observed_bytes, regrowth_duration_ms
         ) VALUES ('fixture.cleanup.journal', 1, 1, 999999, 888888, 1)",
        [],
    );
    fixture.insert_outcome_followup(
        "absence",
        anchor + Duration::from_secs(1),
        anchor + Duration::from_secs(2),
        None,
    );
    fixture.insert_outcome_followup(
        "zero",
        anchor + Duration::from_secs(3),
        anchor + Duration::from_secs(4),
        Some(0),
    );
    fixture.insert_outcome_followup(
        "regrown",
        anchor + Duration::from_secs(5),
        anchor + Duration::from_secs(6),
        Some(321),
    );

    fixture.store.with_connection(|connection| {
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
    });
    let before = fs::read(&fixture.database).unwrap();
    let config = EngineConfig::new(
        fixture.database.clone(),
        fixture.database.parent().unwrap().join("snapshots"),
        fixture._temp.path().join("cache/Dux"),
    )
    .unwrap();
    let engine = EngineHandle::open(config).unwrap();
    let session =
        DurableCleanupSessionId::from_stable_str(fixture.session_id.as_str().to_owned()).unwrap();
    let batch = engine.rule_outcomes_for_cleanup_session(&session).unwrap();
    assert_eq!(batch.session_id(), &session);
    assert_eq!(batch.outcomes().len(), 1);
    assert!(matches!(
        batch.outcomes()[0].state(),
        DurableRuleOutcomeState::Regrown {
            cleaned_at,
            zero_observed_at,
            observed_at,
            observed_bytes: 321,
            regrowth_duration,
        } if *cleaned_at == anchor
            && *zero_observed_at == anchor + Duration::from_secs(4)
            && *observed_at == anchor + Duration::from_secs(6)
            && *regrowth_duration == Duration::from_secs(2)
    ));
    let after = fs::read(&fixture.database).unwrap();
    assert_eq!(after, before);
}

#[test]
fn recurring_storage_thieves_rank_confirmed_growth_and_count_sessions_once() {
    use crate::engine::{EngineConfig, EngineHandle};

    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.enable_outcome_source();
    let anchor = fixture.complete_removed();
    fixture.insert_outcome_followup(
        "ranking-zero",
        anchor + Duration::from_secs(1),
        anchor + Duration::from_secs(2),
        Some(0),
    );
    fixture.insert_outcome_followup(
        "ranking-regrown",
        anchor + Duration::from_secs(3),
        anchor + Duration::from_secs(4),
        Some(321),
    );
    fixture.insert_completed_clone(
        "session:ranking-manual-two",
        anchor + Duration::from_secs(10),
        CleanupTrigger::Manual,
    );
    fixture.insert_completed_clone(
        "session:ranking-cli",
        anchor + Duration::from_secs(20),
        CleanupTrigger::Cli,
    );
    fixture.execute(
        "INSERT INTO rule_outcomes (
             rule_id, rule_revision, cleaned_at_unix_ms, cleaned_bytes,
             next_observed_bytes, regrowth_duration_ms
         ) VALUES ('poison.legacy.rule', 99, 1, 999999, 888888, 1)",
        [],
    );

    fixture.store.with_connection(|connection| {
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
    });
    let before = fs::read(&fixture.database).unwrap();
    let engine = EngineHandle::open(
        EngineConfig::new(
            fixture.database.clone(),
            fixture.database.parent().unwrap().join("snapshots"),
            fixture._temp.path().join("cache/Dux"),
        )
        .unwrap(),
    )
    .unwrap();
    let ranking = engine.recurring_storage_thieves().unwrap();
    assert_eq!(ranking.permanent_safe_session_count(), 3);
    assert_eq!(ranking.manual_cleanup_session_count(), 2);
    assert_eq!(ranking.ranked_rule_count(), 1);
    assert!(!ranking.has_older_permanent_safe_sessions());
    let [group] = ranking.groups() else {
        panic!("expected one recurring rule");
    };
    assert_eq!(group.latest_rule().id().as_str(), "fixture.cleanup.journal");
    assert_eq!(group.observed_revision_count(), 1);
    assert_eq!(group.successful_cleanup_count(), 3);
    assert_eq!(group.successful_manual_cleanup_count(), 2);
    assert_eq!(group.observed_regrowth_cycle_count(), 1);
    assert_eq!(group.manual_regrowth_cycle_count(), 1);
    assert_eq!(group.total_observed_regrown_bytes(), 321);
    assert_eq!(group.total_regrowth_duration(), Duration::from_secs(2));
    assert_eq!(group.bytes_regrown_per_day(), 13_867_200);
    assert!(!group.rate_capped());
    assert!(group.automation_history_threshold_met());
    let after = fs::read(&fixture.database).unwrap();
    assert_eq!(after, before);
}

#[test]
fn automation_history_suggests_exact_current_rule_after_two_newest_manual_successes_and_regrowth() {
    let fixture = Fixture::new_automation_suggestion();
    fixture.enable_outcome_source();
    let anchor = fixture.complete_removed();
    fixture.insert_outcome_followup(
        "suggestion-zero",
        anchor + Duration::from_secs(1),
        anchor + Duration::from_secs(2),
        Some(0),
    );
    let regrowth_at = anchor + Duration::from_secs(4);
    fixture.insert_outcome_followup(
        "suggestion-regrown",
        anchor + Duration::from_secs(3),
        regrowth_at,
        Some(321),
    );
    let latest_attempt_at = anchor + Duration::from_secs(10);
    fixture.insert_completed_clone(
        "session:suggestion-manual-two",
        latest_attempt_at,
        CleanupTrigger::Manual,
    );

    let feed = fixture
        .store
        .automation_schedule_suggestions(std::slice::from_ref(&fixture.rule))
        .unwrap();
    assert_eq!(feed.source_session_count, 2);
    assert_eq!(feed.qualifying_rule_count, 1);
    assert!(!feed.has_older_source_sessions);
    let [suggestion] = feed.suggestions.as_slice() else {
        panic!("expected one exact-current-rule suggestion");
    };
    assert_eq!(suggestion.rule, *fixture.rule.reference());
    assert_eq!(suggestion.successful_manual_run_count, 2);
    assert_eq!(suggestion.manual_regrowth_cycle_count, 1);
    assert_eq!(suggestion.latest_manual_attempt_at, latest_attempt_at);
    assert_eq!(suggestion.latest_regrowth_at, regrowth_at);
}

#[test]
fn automation_history_engine_feed_is_empty_for_current_catalog_and_read_only() {
    use crate::engine::{EngineConfig, EngineHandle};

    let fixture = Fixture::new_automation_suggestion();
    fixture.store.with_connection(|connection| {
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
    });
    let before = fs::read(&fixture.database).unwrap();
    let engine = EngineHandle::open(
        EngineConfig::new(
            fixture.database.clone(),
            fixture.database.parent().unwrap().join("snapshots"),
            fixture._temp.path().join("cache/Dux"),
        )
        .unwrap(),
    )
    .unwrap();
    let feed = engine.automation_schedule_suggestions().unwrap();
    assert_eq!(feed.source_session_count(), 1);
    assert_eq!(feed.qualifying_rule_count(), 0);
    assert!(!feed.has_older_source_sessions());
    assert!(feed.suggestions().is_empty());
    let after = fs::read(&fixture.database).unwrap();
    assert_eq!(after, before);
}

#[test]
fn newest_active_manual_attempt_suppresses_automation_history_suggestion() {
    let fixture = Fixture::new_automation_suggestion();
    fixture.enable_outcome_source();
    let anchor = fixture.complete_removed();
    fixture.insert_outcome_followup(
        "active-suppression-zero",
        anchor + Duration::from_secs(1),
        anchor + Duration::from_secs(2),
        Some(0),
    );
    fixture.insert_outcome_followup(
        "active-suppression-regrown",
        anchor + Duration::from_secs(3),
        anchor + Duration::from_secs(4),
        Some(1),
    );
    fixture.insert_completed_clone(
        "session:active-suppression-two",
        anchor + Duration::from_secs(10),
        CleanupTrigger::Manual,
    );
    let target = fixture._temp.path().join("root/cleanup-fixture-0");
    fixture.insert_active_superseding_journal(
        "session:active-suppression-three",
        &target,
        anchor + Duration::from_secs(20),
    );
    fixture.execute(
        "UPDATE cleanup_items SET final_status = 'validating'
         WHERE session_id = 'session:active-suppression-three'",
        [],
    );
    fixture.execute(
        "UPDATE cleanup_item_paths
         SET status = 'validating', effect_started_at_unix_ms = NULL,
             completed_at_unix_ms = NULL
         WHERE session_id = 'session:active-suppression-three'",
        [],
    );

    let feed = fixture
        .store
        .automation_schedule_suggestions(std::slice::from_ref(&fixture.rule))
        .unwrap();
    assert_eq!(feed.source_session_count, 3);
    assert_eq!(feed.qualifying_rule_count, 0);
    assert!(feed.suggestions.is_empty());
}

#[test]
fn prior_rule_revision_cannot_satisfy_current_automation_history() {
    let fixture = Fixture::new_automation_suggestion();
    fixture.enable_outcome_source();
    let anchor = fixture.complete_removed();
    fixture.insert_outcome_followup(
        "revision-suggestion-zero",
        anchor + Duration::from_secs(1),
        anchor + Duration::from_secs(2),
        Some(0),
    );
    fixture.insert_outcome_followup(
        "revision-suggestion-regrown",
        anchor + Duration::from_secs(3),
        anchor + Duration::from_secs(4),
        Some(1),
    );
    fixture.insert_completed_clone(
        "session:revision-suggestion-two",
        anchor + Duration::from_secs(10),
        CleanupTrigger::Manual,
    );

    let current_revision = automation_suggestion_fixture_rule_at_revision(2);
    let feed = fixture
        .store
        .automation_schedule_suggestions(&[current_revision])
        .unwrap();
    assert_eq!(feed.source_session_count, 2);
    assert_eq!(feed.qualifying_rule_count, 0);
    assert!(feed.suggestions.is_empty());
}

#[test]
fn automation_history_source_window_has_one_row_lookahead() {
    let fixture = Fixture::new_automation_suggestion();
    let anchor = fixture.complete_removed();
    let base = anchor + Duration::from_secs(10);
    for index in 0..MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS {
        fixture.insert_completed_clone(
            &format!("session:suggestion-window:{index:02}"),
            base + Duration::from_secs(index as u64 * 10),
            CleanupTrigger::Manual,
        );
    }

    let feed = fixture.store.automation_schedule_suggestions(&[]).unwrap();
    assert_eq!(
        feed.source_session_count,
        MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS
    );
    assert!(feed.has_older_source_sessions);
    assert_eq!(feed.qualifying_rule_count, 0);
    assert!(feed.suggestions.is_empty());
}

#[test]
fn current_protected_descendant_selector_is_never_suggestible() {
    let fixture = Fixture::new_automation_suggestion();
    let protected = automation_suggestion_fixture_rule_with_protected_descendant();
    assert_eq!(
        fixture
            .store
            .automation_schedule_suggestions(&[protected])
            .unwrap_err()
            .kind,
        HistoryErrorKind::InternalState
    );
}

#[test]
fn nonmanual_cleanup_cannot_bootstrap_automation_history_threshold() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.enable_outcome_source();
    let anchor = fixture.complete_removed();
    fixture.insert_outcome_followup(
        "threshold-zero",
        anchor + Duration::from_secs(1),
        anchor + Duration::from_secs(2),
        Some(0),
    );
    fixture.insert_outcome_followup(
        "threshold-regrown",
        anchor + Duration::from_secs(3),
        anchor + Duration::from_secs(4),
        Some(1),
    );
    fixture.insert_completed_clone(
        "session:threshold-cli",
        anchor + Duration::from_secs(10),
        CleanupTrigger::Cli,
    );

    let ranking = fixture.store.recurring_storage_thieves().unwrap();
    let [group] = ranking.groups.as_slice() else {
        panic!("expected one recurring rule");
    };
    assert_eq!(group.successful_cleanup_count, 2);
    assert_eq!(group.successful_manual_cleanup_count, 1);
    assert_eq!(group.manual_regrowth_cycle_count, 1);
    assert!(!group.automation_history_threshold_met);
}

#[test]
fn recurring_storage_thief_window_is_bounded_to_newest_qualifying_sessions() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.enable_outcome_source();
    let anchor = fixture.complete_removed();
    fixture.insert_outcome_followup(
        "bounded-zero",
        anchor + Duration::from_secs(1),
        anchor + Duration::from_secs(2),
        Some(0),
    );
    fixture.insert_outcome_followup(
        "bounded-regrown",
        anchor + Duration::from_secs(3),
        anchor + Duration::from_secs(4),
        Some(1),
    );
    for index in 0..MAX_STORAGE_THIEF_SOURCE_SESSIONS {
        fixture.insert_completed_clone(
            &format!("session:bounded:{index:02}"),
            anchor + Duration::from_secs(10 + index as u64 * 10),
            CleanupTrigger::Manual,
        );
    }
    let ranking = fixture.store.recurring_storage_thieves().unwrap();
    assert_eq!(
        ranking.permanent_safe_session_count,
        MAX_STORAGE_THIEF_SOURCE_SESSIONS
    );
    assert_eq!(
        ranking.manual_cleanup_session_count,
        MAX_STORAGE_THIEF_SOURCE_SESSIONS
    );
    assert!(ranking.has_older_permanent_safe_sessions);
    assert_eq!(ranking.ranked_rule_count, 0);
    assert!(ranking.groups.is_empty());
}

#[test]
fn latest_rule_revision_needs_its_own_manual_recurrence_evidence() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.enable_outcome_source();
    let anchor = fixture.complete_removed();
    fixture.insert_outcome_followup(
        "revision-zero",
        anchor + Duration::from_secs(1),
        anchor + Duration::from_secs(2),
        Some(0),
    );
    fixture.insert_outcome_followup(
        "revision-regrown",
        anchor + Duration::from_secs(3),
        anchor + Duration::from_secs(4),
        Some(1),
    );
    fixture.insert_completed_clone(
        "session:revision-one-two",
        anchor + Duration::from_secs(10),
        CleanupTrigger::Manual,
    );
    fixture.insert_completed_clone(
        "session:revision-two-one",
        anchor + Duration::from_secs(20),
        CleanupTrigger::Manual,
    );
    fixture.execute(
        "UPDATE cleanup_items SET rule_revision = 2
         WHERE session_id = 'session:revision-two-one'",
        [],
    );

    let ranking = fixture.store.recurring_storage_thieves().unwrap();
    let [group] = ranking.groups.as_slice() else {
        panic!("expected one recurring rule");
    };
    assert_eq!(group.latest_rule.revision().get(), 2);
    assert_eq!(group.observed_revision_count, 2);
    assert_eq!(group.successful_manual_cleanup_count, 3);
    assert_eq!(group.manual_regrowth_cycle_count, 1);
    assert!(!group.automation_history_threshold_met);
}

#[test]
fn active_source_is_ineligible_but_active_overlapping_later_cleanup_supersedes() {
    use crate::persistence::StoredRuleOutcomeState;

    let active_source = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    active_source.enable_outcome_source();
    let mut claim = active_source.claim();
    claim.begin_validation(0, 0).unwrap();
    let receipt = claim
        .mark_effect_started(0, 0, active_source.started_at + Duration::from_secs(2))
        .unwrap();
    claim
        .finish_effect(
            &receipt,
            EffectOutcome::Removed,
            None,
            active_source.started_at + Duration::from_secs(3),
        )
        .unwrap();
    let active = active_source
        .store
        .rule_outcomes_for_cleanup_session(&active_source.session_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        active.outcomes[0].state,
        StoredRuleOutcomeState::NotEligible {
            reason: crate::persistence::StoredRuleOutcomeNotEligibleReason::SourceCleanupIncomplete
        }
    );
    drop(claim);

    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.enable_outcome_source();
    let anchor = fixture.complete_removed();
    let root = fixture._temp.path().join("root");
    fixture.insert_active_superseding_journal(
        "session:text-prefix-only",
        &root.join("cleanup-fixture-0-suffix"),
        anchor,
    );
    let text_prefix = fixture
        .store
        .rule_outcomes_for_cleanup_session(&fixture.session_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        text_prefix.outcomes[0].state,
        StoredRuleOutcomeState::AwaitingComparableScan { cleaned_at } if cleaned_at == anchor
    ));

    fixture.insert_active_superseding_journal(
        "session:actual-descendant",
        &root.join("cleanup-fixture-0").join("child"),
        anchor,
    );
    let overlapping = fixture
        .store
        .rule_outcomes_for_cleanup_session(&fixture.session_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        overlapping.outcomes[0].state,
        StoredRuleOutcomeState::Superseded { cleaned_at, .. } if cleaned_at == anchor
    ));
}

#[test]
fn terminal_size_survives_later_cleanup_but_equal_scan_effect_boundary_fails_closed() {
    use crate::persistence::StoredRuleOutcomeState;

    let terminal = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    terminal.enable_outcome_source();
    let anchor = terminal.complete_removed();
    terminal.insert_outcome_followup(
        "terminal",
        anchor + Duration::from_millis(1),
        anchor + Duration::from_millis(19),
        Some(41),
    );
    let target = terminal._temp.path().join("root/cleanup-fixture-0");
    terminal.insert_active_superseding_journal("session:after-terminal", &target, anchor);
    let retained = terminal
        .store
        .rule_outcomes_for_cleanup_session(&terminal.session_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        retained.outcomes[0].state,
        StoredRuleOutcomeState::LaterSizeObserved {
            cleaned_at,
            observed_bytes: 41,
            ..
        } if cleaned_at == anchor
    ));

    let equal = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    equal.enable_outcome_source();
    let anchor = equal.complete_removed();
    equal.insert_outcome_followup(
        "equal",
        anchor + Duration::from_millis(1),
        anchor + Duration::from_millis(20),
        Some(0),
    );
    let target = equal._temp.path().join("root/cleanup-fixture-0");
    equal.insert_active_superseding_journal("session:equal-boundary", &target, anchor);
    let superseded = equal
        .store
        .rule_outcomes_for_cleanup_session(&equal.session_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        superseded.outcomes[0].state,
        StoredRuleOutcomeState::Superseded { cleaned_at, .. } if cleaned_at == anchor
    ));
}

#[test]
fn reversed_completion_and_equal_boundary_overlaps_are_ignored() {
    use crate::persistence::StoredRuleOutcomeState;

    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.enable_outcome_source();
    let anchor = fixture.complete_removed();
    fixture.insert_outcome_followup(
        "zero-started-first",
        anchor + Duration::from_secs(1),
        anchor + Duration::from_secs(10),
        Some(0),
    );
    fixture.insert_outcome_followup(
        "nonzero-completed-first",
        anchor + Duration::from_secs(2),
        anchor + Duration::from_secs(5),
        Some(73),
    );

    let outcomes = fixture
        .store
        .rule_outcomes_for_cleanup_session(&fixture.session_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        outcomes.outcomes[0].state,
        StoredRuleOutcomeState::AwaitingComparableScan { cleaned_at } if cleaned_at == anchor
    ));

    let equal = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    equal.enable_outcome_source();
    let anchor = equal.complete_removed();
    equal.insert_outcome_followup(
        "equal-first",
        anchor + Duration::from_secs(1),
        anchor + Duration::from_secs(5),
        Some(0),
    );
    equal.insert_outcome_followup(
        "equal-second",
        anchor + Duration::from_secs(5),
        anchor + Duration::from_secs(7),
        Some(88),
    );
    let outcomes = equal
        .store
        .rule_outcomes_for_cleanup_session(&equal.session_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        outcomes.outcomes[0].state,
        StoredRuleOutcomeState::AwaitingComparableScan { cleaned_at } if cleaned_at == anchor
    ));
}

#[test]
fn post_plan_source_evaluation_and_source_mismatch_short_circuit_ineligibility() {
    use crate::persistence::{
        StoredRuleOutcomeNotEligibleReason as Reason, StoredRuleOutcomeState,
    };

    let post_plan = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    post_plan.enable_outcome_source();
    post_plan.complete_removed();
    post_plan.execute(
        "UPDATE candidate_evaluations
         SET completed_at_unix_ms = 1750000010001
         WHERE scan_id = 'scan:cleanup-journal'",
        [],
    );
    post_plan.execute(
        "UPDATE candidates
         SET created_at_unix_ms = 1750000010001
         WHERE scan_id = 'scan:cleanup-journal'",
        [],
    );
    let outcomes = post_plan
        .store
        .rule_outcomes_for_cleanup_session(&post_plan.session_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        outcomes.outcomes[0].state,
        StoredRuleOutcomeState::NotEligible {
            reason: Reason::SourceEvaluationAfterPlan
        }
    );

    let mismatch = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    mismatch.enable_outcome_source();
    let anchor = mismatch.complete_removed();
    let root =
        crate::persistence::codec::encode_host_path(&mismatch._temp.path().join("root")).unwrap();
    let anchor_ms = anchor.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    mismatch.store.with_connection(|connection| {
        let transaction = connection.unchecked_transaction().unwrap();
        transaction
            .execute(
                "UPDATE cleanup_items SET candidate_id = 'candidate:source-mismatch'
                 WHERE session_id = ?1",
                [mismatch.session_id.as_str()],
            )
            .unwrap();
        transaction
            .execute(
                "WITH RECURSIVE n(value) AS (
                     SELECT 0 UNION ALL SELECT value + 1 FROM n WHERE value < 256
                 )
                 INSERT INTO scans (
                     scan_id, root_path, root_path_encoding, started_at_unix_ms,
                     status, coverage_status
                 )
                 SELECT 'scan:overflow:' || printf('%03d', value), ?1, ?2,
                        ?3 + value + 1, 'running', 'unknown'
                 FROM n",
                params![root.bytes, root.encoding as i64, anchor_ms],
            )
            .unwrap();
        transaction.commit().unwrap();
    });
    let outcomes = mismatch
        .store
        .rule_outcomes_for_cleanup_session(&mismatch.session_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        outcomes.outcomes[0].state,
        StoredRuleOutcomeState::NotEligible {
            reason: Reason::SourceCandidateMismatch
        }
    );
}

#[test]
fn claimed_journal_requires_exact_frozen_plan_witness() {
    let fixture = Fixture::new(CleanupMode::Trash, CandidateAction::MoveToTrash, 1);
    let lease = fixture.lease();
    lease
        .validate_planned_plan(&fixture.session_id, &fixture.plan)
        .unwrap();
    let claim = lease
        .claim_planned(
            &fixture.session_id,
            fixture.started_at + Duration::from_secs(1),
        )
        .unwrap();
    claim.validate_planned_plan(&fixture.plan).unwrap();
}

fn fixture_rule(action: CandidateAction) -> Rule {
    let (category, safety, cloud_guard) = match action {
        CandidateAction::MoveToTrash => (
            CandidateCategory::LargeReviewItem,
            SafetyTier::ReviewRequired,
            false,
        ),
        CandidateAction::RemoveKnownRegenerableContents => (
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            false,
        ),
        CandidateAction::EvictLocalCopy => (
            CandidateCategory::CloudFile,
            SafetyTier::SafeEvictable,
            true,
        ),
        CandidateAction::RevealOnly | CandidateAction::NoAction => {
            panic!("fixture requires a cleanup action")
        }
    };
    Rule::try_new(RuleDefinition {
        reference: RuleRef::new(
            RuleId::new("fixture.cleanup.journal").unwrap(),
            RuleRevision::new(1).unwrap(),
        ),
        title_key: LocalizedTextKey::new("fixture.cleanup.title").unwrap(),
        category,
        scope: RuleScope::ConfiguredProjectRoots,
        matcher: RuleMatcher::try_new(RuleMatcherDefinition {
            path_component: Some("cleanup-fixture".to_owned()),
            required_ancestor_markers_any: Vec::new(),
            required_markers_all: Vec::new(),
            forbidden_markers_any: Vec::new(),
            exact_bundle_identifiers: Vec::new(),
            excluded_descendants: Vec::new(),
            protected_descendants: Vec::new(),
        })
        .unwrap(),
        guards: RuleGuards::try_new(None, 0, Vec::new(), cloud_guard).unwrap(),
        safety,
        action,
        schedule_eligible: false,
        explanation_key: LocalizedTextKey::new("fixture.cleanup.explanation").unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/cleanup").unwrap()],
    })
    .unwrap()
}

fn automation_suggestion_fixture_rule() -> Rule {
    automation_suggestion_fixture_rule_at_revision(1)
}

fn automation_suggestion_fixture_rule_at_revision(revision: u32) -> Rule {
    automation_suggestion_fixture_rule_with_policy(revision, Vec::new())
}

fn automation_suggestion_fixture_rule_with_protected_descendant() -> Rule {
    automation_suggestion_fixture_rule_with_policy(1, vec!["sensitive".to_owned()])
}

fn automation_suggestion_fixture_rule_with_policy(
    revision: u32,
    protected_descendants: Vec<String>,
) -> Rule {
    Rule::try_new(RuleDefinition {
        reference: RuleRef::new(
            RuleId::new("fixture.cleanup.journal").unwrap(),
            RuleRevision::new(revision).unwrap(),
        ),
        title_key: LocalizedTextKey::new("fixture.cleanup.title").unwrap(),
        category: CandidateCategory::ApplicationCache,
        scope: RuleScope::UserCacheDirectory,
        matcher: RuleMatcher::try_new(RuleMatcherDefinition {
            path_component: Some("cleanup-fixture".to_owned()),
            required_ancestor_markers_any: Vec::new(),
            required_markers_all: Vec::new(),
            forbidden_markers_any: Vec::new(),
            exact_bundle_identifiers: Vec::new(),
            excluded_descendants: Vec::new(),
            protected_descendants,
        })
        .unwrap(),
        guards: RuleGuards::try_new(None, 0, Vec::new(), false).unwrap(),
        safety: SafetyTier::SafeRegenerable,
        action: CandidateAction::RemoveKnownRegenerableContents,
        schedule_eligible: true,
        explanation_key: LocalizedTextKey::new("fixture.cleanup.explanation").unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/cleanup").unwrap()],
    })
    .unwrap()
}

fn fixture_candidate(id: &str, scan_id: &str, rule: &Rule, path: PathBuf, bytes: u64) -> Candidate {
    let evidence = if rule.action() == CandidateAction::EvictLocalCopy {
        vec![Evidence::CloudUploadComplete { path: path.clone() }]
    } else {
        vec![Evidence::MatchedPath { path: path.clone() }]
    };
    Candidate::try_from_rule(
        rule,
        CandidateInput::new(
            CandidateId::new(id).unwrap(),
            vec![path],
            bytes,
            None,
            evidence,
            Vec::new(),
            ScanId::new(scan_id).unwrap(),
        ),
    )
    .unwrap()
}

fn start_scan(store: &StoreCoordinator, root: &Path, id: &str) {
    let id = ScanId::new(id).unwrap();
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                id.clone(),
                root.to_path_buf(),
                UNIX_EPOCH + Duration::from_secs(1_750_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    store
        .record_scan_finished(
            &ScanCompletionRecord::try_new(
                id,
                UNIX_EPOCH + Duration::from_secs(1_750_000_001),
                TerminalScanStatus::Succeeded,
                ScanCounts::default(),
            )
            .unwrap(),
        )
        .unwrap();
}

fn mutable_journal_bytes(store: &StoreCoordinator, session_id: &CleanupSessionId) -> Vec<u8> {
    const QUERIES: [&str; 5] = [
        "SELECT status, completed_at_unix_ms, verified_capacity_delta_bytes, execution_owner_id, execution_generation, last_heartbeat_at_unix_ms, cancellation_requested, execution_host_identity_v1_sha256, execution_boot_scope_v1_sha256, execution_recovery_policy FROM cleanup_sessions WHERE session_id = ?1",
        "SELECT item_ordinal, final_status, error_category FROM cleanup_items WHERE session_id = ?1 ORDER BY item_ordinal",
        "SELECT item_ordinal, path_ordinal, attempt_generation, status, error_category, effect_started_at_unix_ms, completed_at_unix_ms FROM cleanup_item_paths WHERE session_id = ?1 ORDER BY item_ordinal, path_ordinal",
        "SELECT candidate_id, item_ordinal, prior_review_status FROM candidate_plan_claims WHERE session_id = ?1 ORDER BY item_ordinal",
        "SELECT candidate_id, item_ordinal, coupling_revision FROM trusted_rust_target_plan_claims WHERE session_id = ?1 ORDER BY item_ordinal",
    ];
    store.with_connection(|connection| {
        let mut bytes = Vec::new();
        for query in QUERIES {
            let mut statement = connection.prepare(query).unwrap();
            let column_count = statement.column_count();
            let mut rows = statement.query([session_id.as_str()]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                bytes.push(0xff);
                for column in 0..column_count {
                    append_value(&mut bytes, row.get::<_, Value>(column).unwrap());
                }
            }
        }
        bytes
    })
}

fn append_value(output: &mut Vec<u8>, value: Value) {
    match value {
        Value::Null => output.push(0),
        Value::Integer(value) => {
            output.push(1);
            output.extend_from_slice(&value.to_le_bytes());
        }
        Value::Real(value) => {
            output.push(2);
            output.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        Value::Text(value) => {
            output.push(3);
            output.extend_from_slice(&(value.len() as u64).to_le_bytes());
            output.extend_from_slice(value.as_bytes());
        }
        Value::Blob(value) => {
            output.push(4);
            output.extend_from_slice(&(value.len() as u64).to_le_bytes());
            output.extend_from_slice(&value);
        }
    }
}

#[test]
fn scalar_state_validator_accepts_planned_active_and_terminal_journals() {
    let fixture = Fixture::new(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.validate_scalar_state().unwrap();

    let mut claim = fixture.claim();
    fixture.validate_scalar_state().unwrap();
    claim.begin_validation(0, 0).unwrap();
    fixture.validate_scalar_state().unwrap();
    claim
        .finish_validation(
            0,
            0,
            ValidationOutcome::DryRun,
            None,
            fixture.started_at + Duration::from_secs(2),
        )
        .unwrap();
    fixture.validate_scalar_state().unwrap();
    claim
        .terminalize(fixture.started_at + Duration::from_secs(3), None)
        .unwrap();
    fixture.validate_scalar_state().unwrap();
}

#[test]
fn validated_dry_run_is_atomic_uncoupled_terminal_history_without_effect_fields() {
    let fixture = Fixture::new_unrecorded(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        2,
    );
    let completed_at = fixture.started_at + Duration::from_secs(2);
    assert_eq!(
        fixture
            .lease()
            .record_validated_dry_run(
                fixture.session_id.clone(),
                &fixture.plan,
                CleanupTrigger::Manual,
                ValidatedDryRunOutcome::DryRun,
                fixture.started_at,
                completed_at,
            )
            .unwrap(),
        TerminalSessionStatus::DryRun
    );

    let snapshot = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert!(matches!(
        snapshot.lifecycle,
        JournalLifecycle::ObservedTerminal {
            status: TerminalSessionStatus::DryRun,
            cancellation_requested: false,
            ..
        }
    ));
    assert!(snapshot.items.iter().all(|item| {
        item.status == PathStatus::DryRun
            && item.error_category.is_none()
            && item.paths.iter().all(|path| {
                path.status == PathStatus::DryRun
                    && path.attempt_generation == Some(1)
                    && path.error_category.is_none()
                    && path.effect_started_at.is_none()
                    && path.completed_at.is_some()
            })
    }));
    assert_eq!(
        fixture.candidate_status(0),
        CandidateHistoryStatus::Discovered
    );
    assert_eq!(
        fixture.candidate_status(1),
        CandidateHistoryStatus::Discovered
    );
    assert_eq!(fixture.claim_count(), 0);

    let parent = fixture.store.with_connection(|connection| {
        connection
            .query_row(
                "SELECT status, verified_capacity_delta_bytes,
                        execution_owner_id, execution_generation,
                        last_heartbeat_at_unix_ms, cancellation_requested,
                        candidate_status_coupling_version
                 FROM cleanup_sessions WHERE session_id = ?1",
                [fixture.session_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .unwrap()
    });
    assert_eq!(parent, ("dry_run".to_owned(), None, None, None, None, 0, 1));
    let history = fixture
        .store
        .cleanup_history_session(&fixture.session_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        history.summary.status,
        crate::persistence::StoredCleanupSessionStatus::DryRun
    );
    assert_eq!(history.summary.verified_capacity_delta_bytes, None);
    assert_eq!(history.summary.path_status_counts.dry_run, 2);
    fixture.validate_scalar_state().unwrap();
}

#[test]
fn validated_dry_run_refusals_are_ownerless_terminal_observations() {
    let cases = [
        (
            ValidatedDryRunOutcome::ChangedSincePlan("target_changed"),
            TerminalSessionStatus::Failed,
            PathStatus::ChangedSincePlan,
            false,
        ),
        (
            ValidatedDryRunOutcome::Rejected("protected_path"),
            TerminalSessionStatus::Rejected,
            PathStatus::Rejected,
            false,
        ),
        (
            ValidatedDryRunOutcome::Unavailable("volume_unavailable"),
            TerminalSessionStatus::Failed,
            PathStatus::Unavailable,
            false,
        ),
        (
            ValidatedDryRunOutcome::Interrupted("validation_interrupted"),
            TerminalSessionStatus::Interrupted,
            PathStatus::Interrupted,
            false,
        ),
        (
            ValidatedDryRunOutcome::Cancelled("cancelled"),
            TerminalSessionStatus::Cancelled,
            PathStatus::Interrupted,
            true,
        ),
    ];
    for (outcome, expected_session, expected_path, expected_cancellation) in cases {
        let fixture = Fixture::new_unrecorded(
            CleanupMode::DryRun,
            CandidateAction::RemoveKnownRegenerableContents,
            1,
        );
        assert_eq!(
            fixture
                .lease()
                .record_validated_dry_run(
                    fixture.session_id.clone(),
                    &fixture.plan,
                    CleanupTrigger::Manual,
                    outcome,
                    fixture.started_at,
                    fixture.started_at + Duration::from_secs(2),
                )
                .unwrap(),
            expected_session
        );
        let snapshot = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
        assert!(matches!(
            snapshot.lifecycle,
            JournalLifecycle::ObservedTerminal {
                status,
                cancellation_requested,
                ..
            } if status == expected_session
                && cancellation_requested == expected_cancellation
        ));
        let path = &snapshot.items[0].paths[0];
        assert_eq!(path.status, expected_path);
        assert_eq!(path.attempt_generation, Some(1));
        assert!(path.effect_started_at.is_none());
        assert_eq!(
            fixture.candidate_status(0),
            CandidateHistoryStatus::Discovered
        );
        assert_eq!(fixture.claim_count(), 0);
    }
}

#[test]
fn validated_dry_run_exclusion_is_a_durable_rejection_under_the_lease() {
    let fixture = Fixture::new_unrecorded(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture
        .store
        .set_cleanup_exclusions(vec![fixture._temp.path().join("root")])
        .unwrap();
    assert_eq!(
        fixture
            .lease()
            .record_validated_dry_run(
                fixture.session_id.clone(),
                &fixture.plan,
                CleanupTrigger::Manual,
                ValidatedDryRunOutcome::DryRun,
                fixture.started_at,
                fixture.started_at + Duration::from_secs(2),
            )
            .unwrap(),
        TerminalSessionStatus::Rejected
    );
    let snapshot = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    let path = &snapshot.items[0].paths[0];
    assert_eq!(path.status, PathStatus::Rejected);
    assert_eq!(path.error_category.as_deref(), Some("user_excluded"));
    assert_eq!(path.attempt_generation, Some(1));
    assert!(path.effect_started_at.is_none());
    assert_eq!(
        fixture.candidate_status(0),
        CandidateHistoryStatus::Discovered
    );
    assert_eq!(fixture.claim_count(), 0);
}

#[test]
fn validated_dry_run_exclusion_does_not_hide_a_more_specific_refusal() {
    let fixture = Fixture::new_unrecorded(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture
        .store
        .set_cleanup_exclusions(vec![fixture._temp.path().join("root")])
        .unwrap();
    assert_eq!(
        fixture
            .lease()
            .record_validated_dry_run(
                fixture.session_id.clone(),
                &fixture.plan,
                CleanupTrigger::Manual,
                ValidatedDryRunOutcome::ChangedSincePlan("target_changed"),
                fixture.started_at,
                fixture.started_at + Duration::from_secs(2),
            )
            .unwrap(),
        TerminalSessionStatus::Failed
    );
    let snapshot = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    let path = &snapshot.items[0].paths[0];
    assert_eq!(path.status, PathStatus::ChangedSincePlan);
    assert_eq!(path.error_category.as_deref(), Some("target_changed"));
}

#[test]
fn validated_dry_run_records_post_expiry_refusal_from_pre_expiry_start() {
    let fixture = Fixture::new_unrecorded(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let completed_at = fixture.expires_at + Duration::from_secs(1);
    assert!(fixture.started_at < fixture.expires_at);
    assert_eq!(
        fixture
            .lease()
            .record_validated_dry_run(
                fixture.session_id.clone(),
                &fixture.plan,
                CleanupTrigger::Manual,
                ValidatedDryRunOutcome::ChangedSincePlan("review_expired"),
                fixture.started_at,
                completed_at,
            )
            .unwrap(),
        TerminalSessionStatus::Failed
    );
    let snapshot = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert!(matches!(
        snapshot.lifecycle,
        JournalLifecycle::ObservedTerminal {
            status: TerminalSessionStatus::Failed,
            ..
        }
    ));
    assert_eq!(
        snapshot.items[0].paths[0].error_category.as_deref(),
        Some("review_expired")
    );
}

#[test]
fn validated_dry_run_rejects_completion_before_start_as_definite_failure() {
    let fixture = Fixture::new_unrecorded(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let failure = fixture
        .lease()
        .record_validated_dry_run(
            fixture.session_id.clone(),
            &fixture.plan,
            CleanupTrigger::Manual,
            ValidatedDryRunOutcome::DryRun,
            fixture.started_at,
            fixture.started_at - Duration::from_millis(1),
        )
        .unwrap_err();
    assert_eq!(failure.kind(), HistoryErrorKind::InvalidInput);
    assert!(!failure.may_have_committed());
    drop(failure.into_lease());
    assert!(fixture.lease().load(&fixture.session_id).unwrap().is_none());
}

#[test]
fn ambiguous_validated_dry_run_write_reconciles_only_the_exact_graph_on_retry() {
    let fixture = Fixture::new_unrecorded(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let completed_at = fixture.started_at + Duration::from_secs(2);
    let lease = fixture.lease();
    lease.fail_next_write_after_commit_and_reconcile_read_for_test();
    let failure = lease
        .record_validated_dry_run(
            fixture.session_id.clone(),
            &fixture.plan,
            CleanupTrigger::Manual,
            ValidatedDryRunOutcome::DryRun,
            fixture.started_at,
            completed_at,
        )
        .unwrap_err();
    assert_eq!(failure.kind(), HistoryErrorKind::DatabaseUnavailable);
    assert!(failure.may_have_committed());
    let mismatch = failure
        .into_lease()
        .record_validated_dry_run(
            fixture.session_id.clone(),
            &fixture.plan,
            CleanupTrigger::Manual,
            ValidatedDryRunOutcome::Rejected("different_observation"),
            fixture.started_at,
            completed_at,
        )
        .unwrap_err();
    assert_eq!(mismatch.kind(), HistoryErrorKind::AlreadyExists);
    assert!(!mismatch.may_have_committed());
    assert_eq!(
        mismatch
            .into_lease()
            .record_validated_dry_run(
                fixture.session_id.clone(),
                &fixture.plan,
                CleanupTrigger::Manual,
                ValidatedDryRunOutcome::DryRun,
                fixture.started_at,
                completed_at,
            )
            .unwrap(),
        TerminalSessionStatus::DryRun
    );
    let count = fixture.store.with_connection(|connection| {
        connection
            .query_row(
                "SELECT COUNT(*) FROM cleanup_sessions WHERE session_id = ?1",
                [fixture.session_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    });
    assert_eq!(count, 1);
    assert_eq!(fixture.claim_count(), 0);
}

#[test]
fn unresolved_dry_run_metadata_releases_lease_without_cleanup_authority() {
    let fixture = Fixture::new_unrecorded(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let completed_at = fixture.started_at + Duration::from_secs(2);
    crate::persistence::fail_next_write_after_commit_and_two_reconcile_reads_for_test();
    let first = fixture
        .lease()
        .record_validated_dry_run(
            fixture.session_id.clone(),
            &fixture.plan,
            CleanupTrigger::Manual,
            ValidatedDryRunOutcome::DryRun,
            fixture.started_at,
            completed_at,
        )
        .unwrap_err();
    assert!(first.may_have_committed());
    let second = first
        .into_lease()
        .record_validated_dry_run(
            fixture.session_id.clone(),
            &fixture.plan,
            CleanupTrigger::Manual,
            ValidatedDryRunOutcome::DryRun,
            fixture.started_at,
            completed_at,
        )
        .unwrap_err();
    assert!(!second.may_have_committed());

    drop(second.into_lease());
    let lease = fixture.lease();
    assert_eq!(
        lease
            .record_validated_dry_run(
                fixture.session_id.clone(),
                &fixture.plan,
                CleanupTrigger::Manual,
                ValidatedDryRunOutcome::DryRun,
                fixture.started_at,
                completed_at,
            )
            .unwrap(),
        TerminalSessionStatus::DryRun
    );
    assert_eq!(fixture.claim_count(), 0);
}

#[test]
fn scalar_state_validator_rejects_ordinal_and_derived_item_corruption() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.execute(
        "UPDATE cleanup_item_paths SET path_ordinal = 1 WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    assert_eq!(
        fixture.validate_scalar_state().unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );

    fixture.execute(
        "UPDATE cleanup_item_paths SET path_ordinal = 0 WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    fixture.execute(
        "UPDATE cleanup_items SET final_status = 'failed' WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    assert_eq!(
        fixture.validate_scalar_state().unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn scalar_state_validator_rejects_malformed_execution_owner() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    drop(fixture.claim());
    fixture.execute(
        "UPDATE cleanup_sessions SET execution_owner_id = 'not-an-owner' WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    assert_eq!(
        fixture.validate_scalar_state().unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn pristine_claim_uses_generation_one_and_rejects_the_exact_expiry() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let failure = fixture
        .lease()
        .claim_planned(&fixture.session_id, fixture.expires_at)
        .err()
        .unwrap();
    assert_eq!(failure.kind(), HistoryErrorKind::InvalidTransition);

    let claim = failure
        .into_lease()
        .claim_planned(
            &fixture.session_id,
            fixture.expires_at - Duration::from_nanos(1),
        )
        .unwrap();
    let snapshot = claim.snapshot().unwrap();
    assert!(matches!(
        snapshot.lifecycle,
        JournalLifecycle::Active {
            phase: ActivePhase::Running,
            ref fence,
            cancellation_requested: false,
            ..
        } if fence.generation == 1
    ));
    assert_eq!(snapshot.items[0].paths[0].status, PathStatus::Planned);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    assert!(snapshot.execution_provenance.is_some());
    let stored: (Option<Vec<u8>>, Option<Vec<u8>>, Option<String>) =
        fixture.store.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT execution_host_identity_v1_sha256,
                            execution_boot_scope_v1_sha256,
                            execution_recovery_policy
                     FROM cleanup_sessions WHERE session_id = ?1",
                    [fixture.session_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap()
        });
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    assert!(matches!(
        stored,
        (Some(ref host), Some(ref boot), Some(ref policy))
            if host.len() == 32 && boot.len() == 32 && policy == "resumable"
    ));
}

#[test]
fn decoder_rejects_partial_or_owner_mismatched_execution_provenance() {
    let partial = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    drop(partial.claim());
    partial.execute(
        "UPDATE cleanup_sessions
         SET execution_host_identity_v1_sha256 = NULL
         WHERE session_id = ?1",
        [partial.session_id.as_str()],
    );
    assert_eq!(
        partial.lease().load(&partial.session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    assert_eq!(
        partial.validate_scalar_state().unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let mismatch = Fixture::new(
            CleanupMode::PermanentSafe,
            CandidateAction::RemoveKnownRegenerableContents,
            1,
        );
        let claim = mismatch.claim();
        let JournalLifecycle::Active { fence, .. } = claim.snapshot().unwrap().lifecycle else {
            panic!("claim did not produce active journal state");
        };
        let changed_owner = changed_scope_owner(&fence.owner);
        drop(claim);
        mismatch.execute(
            "UPDATE cleanup_sessions SET execution_owner_id = ?2 WHERE session_id = ?1",
            params![mismatch.session_id.as_str(), changed_owner],
        );
        assert_eq!(
            mismatch
                .lease()
                .load(&mismatch.session_id)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
    }
}

#[test]
fn exact_expiry_rejects_pristine_plan_and_fails_candidates_without_effect_authority() {
    let fixture = Fixture::new_with_selected(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        2,
        &[1],
    );
    let failure = fixture
        .lease()
        .expire_planned(
            &fixture.session_id,
            fixture.expires_at - Duration::from_nanos(1),
        )
        .unwrap_err();
    assert_eq!(failure.kind(), HistoryErrorKind::InvalidTransition);
    assert_eq!(fixture.candidate_status(0), CandidateHistoryStatus::Planned);
    assert_eq!(fixture.candidate_status(1), CandidateHistoryStatus::Planned);
    assert_eq!(fixture.claim_count(), 2);

    assert_eq!(
        failure
            .into_lease()
            .expire_planned(&fixture.session_id, fixture.expires_at)
            .unwrap(),
        TerminalSessionStatus::Rejected
    );
    let snapshot = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert!(matches!(
        snapshot.lifecycle,
        JournalLifecycle::Terminal {
            status: TerminalSessionStatus::Rejected,
            cancellation_requested: false,
            ..
        }
    ));
    assert!(snapshot.items.iter().all(|item| {
        item.status == PathStatus::Rejected
            && item.error_category.as_deref() == Some(PLAN_EXPIRED_ERROR)
            && item.paths.iter().all(|path| {
                path.status == PathStatus::Rejected
                    && path.attempt_generation == Some(1)
                    && path.error_category.as_deref() == Some(PLAN_EXPIRED_ERROR)
                    && path.effect_started_at.is_none()
                    && path.completed_at.is_some()
            })
    }));
    assert_eq!(fixture.candidate_status(0), CandidateHistoryStatus::Failed);
    assert_eq!(fixture.candidate_status(1), CandidateHistoryStatus::Failed);
    assert_eq!(fixture.claim_count(), 0);
}

#[test]
fn ambiguous_expiry_commit_retries_only_the_exact_terminal_projection() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let lease = fixture.lease();
    lease.fail_next_write_after_commit_and_reconcile_read_for_test();
    let failure = lease
        .expire_planned(&fixture.session_id, fixture.expires_at)
        .unwrap_err();
    assert_eq!(failure.kind(), HistoryErrorKind::DatabaseUnavailable);
    assert_eq!(fixture.candidate_status(0), CandidateHistoryStatus::Failed);
    assert_eq!(fixture.claim_count(), 0);

    assert_eq!(
        failure
            .into_lease()
            .expire_planned(&fixture.session_id, fixture.expires_at)
            .unwrap(),
        TerminalSessionStatus::Rejected
    );
    let snapshot = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert!(matches!(
        snapshot.lifecycle,
        JournalLifecycle::Terminal {
            status: TerminalSessionStatus::Rejected,
            ..
        }
    ));
}

#[test]
fn expiry_failure_on_a_later_candidate_rolls_back_every_projection_and_path() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        2,
    );
    fixture.execute(
        "CREATE TEMP TRIGGER fail_second_expiry_candidate
         BEFORE UPDATE OF status ON candidates
         WHEN OLD.candidate_id = 'candidate:journal-1' AND NEW.status = 'failed'
         BEGIN SELECT RAISE(ABORT, 'injected expiry failure'); END",
        [],
    );
    let failure = fixture
        .lease()
        .expire_planned(&fixture.session_id, fixture.expires_at)
        .unwrap_err();
    assert_eq!(failure.kind(), HistoryErrorKind::DatabaseUnavailable);
    let snapshot = failure
        .into_lease()
        .load(&fixture.session_id)
        .unwrap()
        .unwrap();
    assert!(matches!(snapshot.lifecycle, JournalLifecycle::Planned));
    assert!(snapshot.items.iter().all(|item| {
        item.status == PathStatus::Planned
            && item
                .paths
                .iter()
                .all(|path| path.status == PathStatus::Planned)
    }));
    assert_eq!(fixture.candidate_status(0), CandidateHistoryStatus::Planned);
    assert_eq!(fixture.candidate_status(1), CandidateHistoryStatus::Planned);
    assert_eq!(fixture.claim_count(), 2);
}

#[test]
fn failed_claim_retains_a_readable_lease_until_deliberately_released() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let failure = fixture
        .lease()
        .claim_planned(&fixture.session_id, fixture.expires_at)
        .err()
        .unwrap();
    assert_eq!(failure.kind(), HistoryErrorKind::InvalidTransition);

    let retained = failure.into_lease();
    let snapshot = retained.load(&fixture.session_id).unwrap().unwrap();
    assert!(matches!(snapshot.lifecycle, JournalLifecycle::Planned));
    assert_eq!(
        fixture
            .store
            .acquire_cleanup_journal_lease(Duration::ZERO)
            .err()
            .unwrap()
            .kind,
        HistoryErrorKind::Busy
    );

    drop(retained);
    let claim = fixture
        .lease()
        .claim_planned(
            &fixture.session_id,
            fixture.expires_at - Duration::from_nanos(1),
        )
        .unwrap();
    assert!(matches!(
        claim.snapshot().unwrap().lifecycle,
        JournalLifecycle::Active {
            phase: ActivePhase::Running,
            ref fence,
            ..
        } if fence.generation == 1
    ));
}

#[test]
fn migrated_uncoupled_plans_cannot_be_newly_claimed_but_active_work_can_finish() {
    let pristine = Fixture::new(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    pristine.make_legacy_uncoupled("discovered");
    let failure = pristine
        .lease()
        .claim_planned(
            &pristine.session_id,
            pristine.started_at + Duration::from_secs(1),
        )
        .err()
        .unwrap();
    assert_eq!(failure.kind(), HistoryErrorKind::InvalidTransition);
    let retained = failure.into_lease();
    let snapshot = retained.load(&pristine.session_id).unwrap().unwrap();
    assert_eq!(
        snapshot.candidate_status_coupling,
        CandidateStatusCoupling::LegacyUncoupled
    );
    assert!(matches!(snapshot.lifecycle, JournalLifecycle::Planned));
    drop(retained);
    assert_eq!(
        pristine.candidate_status(0),
        CandidateHistoryStatus::Discovered
    );
    assert_eq!(pristine.claim_count(), 0);

    let active = Fixture::new_with_selected(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
        &[0],
    );
    let mut claim = active.claim();
    active.make_legacy_uncoupled("selected");
    claim.begin_validation(0, 0).unwrap();
    claim
        .finish_validation(
            0,
            0,
            ValidationOutcome::DryRun,
            None,
            active.started_at + Duration::from_secs(2),
        )
        .unwrap();
    assert_eq!(
        claim
            .terminalize(active.started_at + Duration::from_secs(3), None)
            .unwrap(),
        TerminalSessionStatus::DryRun
    );
    drop(claim);
    assert_eq!(active.candidate_status(0), CandidateHistoryStatus::Selected);
    assert_eq!(active.claim_count(), 0);
}

#[test]
fn maximum_legal_active_journal_fits_the_shared_query_budget() {
    const ITEM_COUNT: usize = 64;
    const PATHS_PER_ITEM: usize = 4;
    const EVIDENCE_PER_ITEM: usize = 8;

    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("root");
    let store = StoreCoordinator::open(&database).unwrap();
    let scan_id = "scan:cleanup-journal-maximum";
    start_scan(&store, &root, scan_id);
    let rule = fixture_rule(CandidateAction::RemoveKnownRegenerableContents);
    let candidates = (0..ITEM_COUNT)
        .map(|item| {
            let paths = (0..PATHS_PER_ITEM)
                .map(|path| root.join(format!("cleanup-fixture-{item}-{path}")))
                .collect::<Vec<_>>();
            let evidence = (0..EVIDENCE_PER_ITEM)
                .map(|fact| Evidence::MinimumSize {
                    observed_bytes: (item * EVIDENCE_PER_ITEM + fact + 1) as u64,
                    minimum_bytes: (item * EVIDENCE_PER_ITEM + fact) as u64,
                })
                .collect::<Vec<_>>();
            Candidate::try_from_rule(
                &rule,
                CandidateInput::new(
                    CandidateId::new(format!("candidate:journal-maximum-{item}")).unwrap(),
                    paths,
                    item as u64 + 1,
                    None,
                    evidence,
                    Vec::new(),
                    ScanId::new(scan_id).unwrap(),
                ),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    for candidate in &candidates {
        store
            .record_candidate_discovered(
                &NewCandidateRecord::try_from_candidate(
                    candidate,
                    UNIX_EPOCH + Duration::from_secs(1_750_000_002),
                )
                .unwrap(),
            )
            .unwrap();
    }
    let plan = CleanupPlan::try_from_candidates_for_persistence_test(
        CleanupPlanId::new("plan:cleanup-journal-maximum").unwrap(),
        UNIX_EPOCH + Duration::from_secs(PLAN_CREATED_SECONDS),
        CleanupMode::PermanentSafe,
        &candidates,
    )
    .unwrap();
    let session_id = CleanupSessionId::new("session:cleanup-journal-maximum").unwrap();
    let started_at = UNIX_EPOCH + Duration::from_millis(SESSION_STARTED_MILLIS);
    store
        .record_cleanup_session_planned(
            &NewCleanupSessionRecord::try_from_plan(
                session_id.clone(),
                &plan,
                started_at,
                CleanupTrigger::Manual,
            )
            .unwrap(),
        )
        .unwrap();

    let claim = store
        .acquire_cleanup_journal_lease(LOCK_TIMEOUT)
        .unwrap()
        .claim_planned(&session_id, started_at + Duration::from_secs(1))
        .unwrap();
    let snapshot = claim.snapshot().unwrap();
    assert!(matches!(
        snapshot.lifecycle,
        JournalLifecycle::Active {
            phase: ActivePhase::Running,
            ref fence,
            cancellation_requested: false,
            ..
        } if fence.generation == 1
    ));
    assert_eq!(snapshot.items.len(), ITEM_COUNT);
    assert_eq!(
        snapshot
            .items
            .iter()
            .map(|item| item.paths.len())
            .sum::<usize>(),
        ITEM_COUNT * PATHS_PER_ITEM
    );
    assert_eq!(
        snapshot
            .items
            .iter()
            .map(|item| item.frozen.evidence.len())
            .sum::<usize>(),
        ITEM_COUNT * EVIDENCE_PER_ITEM
    );
}

#[test]
fn dry_run_validates_every_path_and_terminalizes_without_an_effect() {
    let fixture = Fixture::new_with_selected(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        2,
        &[1],
    );
    assert_eq!(fixture.candidate_status(0), CandidateHistoryStatus::Planned);
    assert_eq!(fixture.candidate_status(1), CandidateHistoryStatus::Planned);
    assert_eq!(fixture.claim_count(), 2);
    let mut claim = fixture.claim();
    for item in 0..2 {
        claim.begin_validation(item, 0).unwrap();
        claim
            .finish_validation(
                item,
                0,
                ValidationOutcome::DryRun,
                None,
                fixture.started_at + Duration::from_secs(2 + item as u64),
            )
            .unwrap();
    }
    assert_eq!(
        claim
            .terminalize(fixture.started_at + Duration::from_secs(5), None)
            .unwrap(),
        TerminalSessionStatus::DryRun
    );
    drop(claim);

    let snapshot = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert!(matches!(
        snapshot.lifecycle,
        JournalLifecycle::Terminal {
            status: TerminalSessionStatus::DryRun,
            ..
        }
    ));
    assert!(snapshot.items.iter().all(|item| {
        item.status == PathStatus::DryRun
            && item.paths.iter().all(|path| {
                path.status == PathStatus::DryRun
                    && path.effect_started_at.is_none()
                    && path.completed_at.is_some()
            })
    }));
    assert_eq!(
        fixture.candidate_status(0),
        CandidateHistoryStatus::Discovered
    );
    assert_eq!(
        fixture.candidate_status(1),
        CandidateHistoryStatus::Selected
    );
    assert_eq!(fixture.claim_count(), 0);
    fixture
        .store
        .transition_candidate_evaluation_status(
            &CandidateId::new("candidate:journal-1").unwrap(),
            CandidateEvaluationTransition::SelectedToUnavailable,
        )
        .unwrap();
    assert_eq!(
        fixture.candidate_status(1),
        CandidateHistoryStatus::Unavailable
    );
    assert!(matches!(
        fixture
            .lease()
            .load(&fixture.session_id)
            .unwrap()
            .unwrap()
            .lifecycle,
        JournalLifecycle::Terminal {
            status: TerminalSessionStatus::DryRun,
            ..
        }
    ));
}

#[test]
fn trash_admission_binds_the_review_to_the_frozen_journal_path() {
    let fixture = Fixture::new_with_selected_in_current_dir(
        CleanupMode::Trash,
        CandidateAction::MoveToTrash,
        1,
        &[0],
    );
    let expected = fixture._temp.path().join("root/cleanup-fixture-0");
    let claim = fixture.claim();

    claim.validate_planned_path(0, 0, &expected).unwrap();
    assert_eq!(
        claim
            .validate_planned_path(0, 0, &expected.with_file_name("other"))
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    drop(claim);

    // Keep the executor symbol exercised in this journal-focused test module;
    // the platform adapter is intentionally not called by this slice.
    let _ = std::mem::size_of::<TrashExecutionAdmission>();
}

struct RecordingTrashPlatform {
    calls: usize,
    result: Result<(), TrashPlatformError>,
    requested_path: Option<PathBuf>,
}

impl TrashPlatformEffect for RecordingTrashPlatform {
    fn trash(
        &mut self,
        target: &crate::path_validation::TrashPathSnapshot,
    ) -> Result<(), TrashPlatformError> {
        self.calls += 1;
        self.requested_path = Some(target.requested_path().to_path_buf());
        self.result
    }
}

struct OrderedTrashPlatform {
    effect_called: Arc<AtomicBool>,
}

impl TrashPlatformEffect for OrderedTrashPlatform {
    fn trash(
        &mut self,
        _: &crate::path_validation::TrashPathSnapshot,
    ) -> Result<(), TrashPlatformError> {
        self.effect_called.store(true, Ordering::Release);
        Ok(())
    }
}

fn fixture_trash_target(fixture: &Fixture) -> SnapshotReviewTrashTarget {
    let root = fixture._temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    let item = root.join("cleanup-fixture-0");
    fs::write(&item, b"review me").unwrap();
    let lexical_root = validate_scan_root(&root).unwrap();
    let live_root = capture_scan_root(lexical_root.clone()).unwrap();
    let lexical_item = validate_cleanup_path(&lexical_root, &item).unwrap();
    let snapshot = capture_trash_path_snapshot(&live_root, lexical_item).unwrap();
    SnapshotReviewTrashTarget {
        node_id: 11,
        snapshot,
    }
}

#[test]
fn trash_execution_samples_completion_after_the_platform_effect_returns() {
    let fixture = Fixture::new_with_selected_in_current_dir(
        CleanupMode::Trash,
        CandidateAction::MoveToTrash,
        1,
        &[0],
    );
    let admission = TrashExecutionAdmission::from_claim(
        fixture.claim(),
        fixture_trash_target(&fixture),
        0,
        0,
        fixture.started_at + Duration::from_secs(1),
    )
    .unwrap();
    let effect_called = Arc::new(AtomicBool::new(false));
    let mut platform = OrderedTrashPlatform {
        effect_called: Arc::clone(&effect_called),
    };
    let completed_at = fixture.started_at + Duration::from_secs(3);

    admission
        .execute_with_clock_for_test(&mut platform, || {
            assert!(
                effect_called.load(Ordering::Acquire),
                "completion time must be sampled after the platform effect returns"
            );
            completed_at
        })
        .unwrap();

    let journal = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert_eq!(journal.items[0].paths[0].completed_at, Some(completed_at));
}

#[test]
fn trash_execution_driver_records_success_and_calls_once() {
    let fixture = Fixture::new_with_selected_in_current_dir(
        CleanupMode::Trash,
        CandidateAction::MoveToTrash,
        1,
        &[0],
    );
    let expected_path = fixture._temp.path().join("root/cleanup-fixture-0");
    let admission = TrashExecutionAdmission::from_claim(
        fixture.claim(),
        fixture_trash_target(&fixture),
        0,
        0,
        fixture.started_at + Duration::from_secs(1),
    )
    .unwrap();
    let mut platform = RecordingTrashPlatform {
        calls: 0,
        result: Ok(()),
        requested_path: None,
    };

    admission
        .execute_with_at(&mut platform, fixture.started_at + Duration::from_secs(3))
        .unwrap();

    assert_eq!(platform.calls, 1);
    assert_eq!(
        platform.requested_path.as_deref(),
        Some(expected_path.as_path())
    );
    assert!(
        expected_path.exists(),
        "recording adapter must not mutate files"
    );
    let journal = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert_eq!(journal.items[0].paths[0].status, PathStatus::Trashed);
}

#[test]
fn trash_execution_driver_records_unknown_outcome_without_retrying() {
    let fixture = Fixture::new_with_selected_in_current_dir(
        CleanupMode::Trash,
        CandidateAction::MoveToTrash,
        1,
        &[0],
    );
    let admission = TrashExecutionAdmission::from_claim(
        fixture.claim(),
        fixture_trash_target(&fixture),
        0,
        0,
        fixture.started_at + Duration::from_secs(1),
    )
    .unwrap();
    let mut platform = RecordingTrashPlatform {
        calls: 0,
        result: Err(TrashPlatformError::OutcomeUnknown),
        requested_path: None,
    };

    let unresolved = match admission
        .execute_with_at(&mut platform, fixture.started_at + Duration::from_secs(3))
    {
        Err(TrashExecutionError::UnresolvedEffect(unresolved)) => unresolved,
        other => panic!("unknown Trash outcome must retain the recovering claim: {other:?}"),
    };
    assert_eq!(platform.calls, 1);
    assert_eq!(
        unresolved.observed_platform_error(),
        Some(TrashPlatformError::OutcomeUnknown)
    );
    assert!(unresolved.journal_outcome_is_unknown());
    drop(unresolved);
    let journal = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert_eq!(journal.items[0].paths[0].status, PathStatus::OutcomeUnknown);
}

#[test]
fn trash_admission_rejects_a_non_trash_journal_row_before_driver_call() {
    let fixture = Fixture::new_with_selected_in_current_dir(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
        &[],
    );
    let error = match TrashExecutionAdmission::from_claim(
        fixture.claim(),
        fixture_trash_target(&fixture),
        0,
        0,
        fixture.started_at + Duration::from_secs(1),
    ) {
        Ok(_) => panic!("permanent-safe rows must not enter Trash admission"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        TrashAdmissionStartError::Admission(TrashAdmissionError::UnsupportedEffectMode)
    ));
    let journal = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert_eq!(journal.items[0].paths[0].status, PathStatus::Rejected);
    assert!(matches!(
        journal.lifecycle,
        JournalLifecycle::Terminal {
            status: TerminalSessionStatus::Rejected,
            ..
        }
    ));
}

#[test]
fn trash_admission_retains_the_claim_when_a_post_claim_write_is_ambiguous() {
    let fixture = Fixture::new_with_selected_in_current_dir(
        CleanupMode::Trash,
        CandidateAction::MoveToTrash,
        1,
        &[0],
    );
    let claim = fixture.claim();
    fail_next_write_after_commit_and_reconcile_read_for_test();

    let unresolved = match TrashExecutionAdmission::from_claim(
        claim,
        fixture_trash_target(&fixture),
        0,
        0,
        fixture.started_at + Duration::from_secs(1),
    ) {
        Err(TrashAdmissionStartError::ClaimedAdmissionUnresolved(unresolved)) => unresolved,
        Err(error) => panic!("post-claim ambiguity must retain its owner: {error:?}"),
        Ok(_) => panic!("an ambiguous validation write must not admit Trash"),
    };

    let second_lease = fixture.store.acquire_cleanup_journal_lease(LOCK_TIMEOUT);
    assert!(
        matches!(
            second_lease,
            Err(ref error) if error.kind == HistoryErrorKind::Busy
        ),
        "the unresolved capability must keep the store-wide cleanup lease"
    );
    let _quarantined = unresolved;
}

#[test]
#[allow(clippy::disallowed_methods)]
fn trash_admission_terminalizes_known_target_drift_before_releasing_the_claim() {
    let fixture = Fixture::new_with_selected_in_current_dir(
        CleanupMode::Trash,
        CandidateAction::MoveToTrash,
        1,
        &[0],
    );
    let target = fixture_trash_target(&fixture);
    let object_path = target.snapshot.object_path().to_path_buf();
    // DUX-DESTRUCTIVE: allow=test-trash-admission-target-drift-remove -- remove only a TempDir-owned reviewed file before installing an identity-changing replacement
    fs::remove_file(&object_path).unwrap();
    fs::write(&object_path, b"replacement").unwrap();

    let error = match TrashExecutionAdmission::from_claim(
        fixture.claim(),
        target,
        0,
        0,
        fixture.started_at + Duration::from_secs(1),
    ) {
        Err(error) => error,
        Ok(_) => panic!("replaced identity must not enter Trash admission"),
    };
    assert!(matches!(
        error,
        TrashAdmissionStartError::Admission(TrashAdmissionError::TargetChanged)
    ));

    let journal = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert_eq!(
        journal.items[0].paths[0].status,
        PathStatus::ChangedSincePlan
    );
    assert!(matches!(
        journal.lifecycle,
        JournalLifecycle::Terminal {
            status: TerminalSessionStatus::Failed,
            ..
        }
    ));
}

#[test]
fn effect_receipt_revalidates_and_only_the_compatible_outcome_completes() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    let effect_started = fixture.started_at + Duration::from_secs(2);
    let receipt = claim.mark_effect_started(0, 0, effect_started).unwrap();
    claim.revalidate_effect_receipt(&receipt).unwrap();
    assert_eq!(
        claim
            .finish_effect(
                &receipt,
                EffectOutcome::Trashed,
                None,
                fixture.started_at + Duration::from_secs(3),
            )
            .err()
            .unwrap()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    claim
        .finish_effect(
            &receipt,
            EffectOutcome::Removed,
            None,
            fixture.started_at + Duration::from_secs(3),
        )
        .unwrap();
    assert_eq!(
        claim
            .terminalize(fixture.started_at + Duration::from_secs(4), Some(91))
            .unwrap(),
        TerminalSessionStatus::Completed
    );
    drop(claim);
    assert_eq!(
        fixture.candidate_status(0),
        CandidateHistoryStatus::Completed
    );
    assert_eq!(fixture.claim_count(), 0);
}

#[test]
fn rowless_permanent_cleanup_policy_rejects_before_effect_started() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.store.reset_permanent_cleanup().unwrap();
    let claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    assert_eq!(
        claim
            .mark_effect_started(0, 0, fixture.started_at + Duration::from_secs(2))
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        claim.snapshot().unwrap().items[0].paths[0].status,
        PathStatus::Validating
    );
    drop(claim);

    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.store.set_permanent_cleanup_enabled(true).unwrap();
    let claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    let receipt = claim
        .mark_effect_started(0, 0, fixture.started_at + Duration::from_secs(3))
        .unwrap();
    claim.revalidate_effect_receipt(&receipt).unwrap();
    assert_eq!(
        claim.snapshot().unwrap().items[0].paths[0].status,
        PathStatus::EffectStarted
    );
}

#[test]
fn user_cleanup_exclusion_rejects_matching_target_before_effect_started() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture
        .store
        .set_cleanup_exclusions(vec![fixture._temp.path().join("root")])
        .unwrap();
    let claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    assert_eq!(
        claim
            .mark_effect_started(0, 0, fixture.started_at + Duration::from_secs(2))
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        claim.snapshot().unwrap().items[0].paths[0].status,
        PathStatus::Validating
    );
}

#[test]
fn unknown_effect_atomically_enters_recovery_and_blocks_new_work() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        2,
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    let receipt = claim
        .mark_effect_started(0, 0, fixture.started_at + Duration::from_secs(2))
        .unwrap();
    claim
        .finish_effect(
            &receipt,
            EffectOutcome::OutcomeUnknown,
            Some("effect_result_unavailable"),
            fixture.started_at + Duration::from_secs(3),
        )
        .unwrap();

    let snapshot = claim.snapshot().unwrap();
    assert!(matches!(
        snapshot.lifecycle,
        JournalLifecycle::Active {
            phase: ActivePhase::Recovering,
            ..
        }
    ));
    assert_eq!(
        snapshot.items[0].paths[0].status,
        PathStatus::OutcomeUnknown
    );
    assert_eq!(snapshot.items[1].paths[0].status, PathStatus::Planned);
    assert_eq!(
        claim.begin_validation(1, 0).unwrap_err().kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        claim
            .terminalize(fixture.started_at + Duration::from_secs(4), None)
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(fixture.candidate_status(0), CandidateHistoryStatus::Planned);
    assert_eq!(fixture.candidate_status(1), CandidateHistoryStatus::Planned);
    assert_eq!(fixture.claim_count(), 2);
}

#[test]
fn sequential_validation_accepts_terminal_prefix_and_rejects_target_switching() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        2,
    );
    let claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    claim
        .finish_validation(
            0,
            0,
            ValidationOutcome::ChangedSincePlan,
            Some("target_changed"),
            fixture.started_at + Duration::from_secs(2),
        )
        .unwrap();
    claim.begin_validation(1, 0).unwrap();
    assert!(claim.validate_validating_path(1, 0).is_ok());
    assert_eq!(
        claim.validate_validating_path(0, 0).unwrap_err().kind,
        HistoryErrorKind::InvalidTransition
    );
}

#[test]
fn cancellation_settlement_interrupts_remaining_work_and_terminalizes_cancelled() {
    let fixture = Fixture::new_with_selected(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        2,
        &[1],
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    claim.request_cancellation().unwrap();
    assert_eq!(
        claim.begin_validation(1, 0).unwrap_err().kind,
        HistoryErrorKind::InvalidTransition
    );
    claim
        .settle_cancellation(fixture.started_at + Duration::from_secs(3))
        .unwrap();
    assert_eq!(
        claim
            .terminalize(fixture.started_at + Duration::from_secs(4), None)
            .unwrap(),
        TerminalSessionStatus::Cancelled
    );
    drop(claim);

    let snapshot = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert!(matches!(
        snapshot.lifecycle,
        JournalLifecycle::Terminal {
            status: TerminalSessionStatus::Cancelled,
            cancellation_requested: true,
            ..
        }
    ));
    assert!(
        snapshot
            .items
            .iter()
            .all(|item| item.paths.iter().all(|path| {
                path.status == PathStatus::Interrupted && path.attempt_generation == Some(1)
            }))
    );
    assert_eq!(
        fixture.candidate_status(0),
        CandidateHistoryStatus::Discovered
    );
    assert_eq!(
        fixture.candidate_status(1),
        CandidateHistoryStatus::Selected
    );
    assert_eq!(fixture.claim_count(), 0);
    fixture
        .store
        .transition_candidate_review_status(
            &CandidateId::new("candidate:journal-1").unwrap(),
            CandidateReviewTransition::DismissSelected,
        )
        .unwrap();
    assert!(matches!(
        fixture
            .lease()
            .load(&fixture.session_id)
            .unwrap()
            .unwrap()
            .lifecycle,
        JournalLifecycle::Terminal {
            status: TerminalSessionStatus::Cancelled,
            ..
        }
    ));
}

#[test]
fn loader_rejects_stale_or_newer_attempts_for_active_work() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    drop(claim);
    fixture.execute(
        "UPDATE cleanup_item_paths SET attempt_generation = 2 WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    assert_eq!(
        fixture.lease().load(&fixture.session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );

    fixture.execute(
        "UPDATE cleanup_sessions SET execution_generation = 2 WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    fixture.execute(
        "UPDATE cleanup_item_paths SET attempt_generation = 1 WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    assert_eq!(
        fixture.lease().load(&fixture.session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn planned_candidate_and_journal_both_reject_a_missing_claim() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.execute(
        "DELETE FROM candidate_plan_claims WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    let id = CandidateId::new("candidate:journal-0").unwrap();
    assert_eq!(
        fixture.store.load_candidate(&id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    assert_eq!(
        fixture.lease().load(&fixture.session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn claim_loader_rejects_wrong_candidate_version_and_oversized_prior_state() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.execute("PRAGMA ignore_check_constraints = ON", []);
    fixture.execute(
        "UPDATE candidates SET record_format_version = 1
         WHERE candidate_id = 'candidate:journal-0'",
        [],
    );
    assert_eq!(
        fixture.lease().load(&fixture.session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    fixture.execute(
        "UPDATE candidates SET record_format_version = 2
         WHERE candidate_id = 'candidate:journal-0'",
        [],
    );
    fixture.execute(
        "UPDATE candidate_plan_claims SET prior_review_status = ?1
         WHERE session_id = ?2",
        params!["x".repeat(129), fixture.session_id.as_str()],
    );
    fixture.execute("PRAGMA ignore_check_constraints = OFF", []);
    assert_eq!(
        fixture.lease().load(&fixture.session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn loader_rejects_malformed_terminal_generation_and_timestamps() {
    let fixture = Fixture::new(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    claim
        .finish_validation(
            0,
            0,
            ValidationOutcome::DryRun,
            None,
            fixture.started_at + Duration::from_secs(2),
        )
        .unwrap();
    claim
        .terminalize(fixture.started_at + Duration::from_secs(3), None)
        .unwrap();
    drop(claim);

    fixture.execute("PRAGMA ignore_check_constraints = ON", []);
    fixture.execute(
        "UPDATE cleanup_sessions SET execution_generation = 0 WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    assert_eq!(
        fixture.lease().load(&fixture.session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    fixture.execute(
        "UPDATE cleanup_sessions SET execution_generation = 1, completed_at_unix_ms = last_heartbeat_at_unix_ms - 1 WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    fixture.execute("PRAGMA ignore_check_constraints = OFF", []);
    assert_eq!(
        fixture.lease().load(&fixture.session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn live_owner_recovery_refusal_is_a_byte_for_byte_state_no_op() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let claim = fixture.claim();
    let has_provenance = claim.snapshot().unwrap().execution_provenance.is_some();
    drop(claim);
    let before = mutable_journal_bytes(&fixture.store, &fixture.session_id);
    let result = fixture
        .lease()
        .try_recover(
            &fixture.session_id,
            fixture.started_at + Duration::from_secs(10),
        )
        .unwrap();
    if has_provenance {
        assert!(matches!(result, RecoveryClaimResult::OwnerAlive));
    } else {
        assert!(matches!(result, RecoveryClaimResult::Unproven));
    }
    let after = mutable_journal_bytes(&fixture.store, &fixture.session_id);
    assert_eq!(after, before);
}

#[test]
fn live_owner_without_provenance_is_unproven_and_a_byte_for_byte_no_op() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    drop(fixture.claim());
    fixture.execute(
        "UPDATE cleanup_sessions
         SET execution_host_identity_v1_sha256 = NULL,
             execution_boot_scope_v1_sha256 = NULL,
             execution_recovery_policy = NULL
         WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    let before = mutable_journal_bytes(&fixture.store, &fixture.session_id);
    let result = fixture
        .lease()
        .try_recover(
            &fixture.session_id,
            fixture.started_at + Duration::from_secs(10),
        )
        .unwrap();
    assert!(matches!(result, RecoveryClaimResult::Unproven));
    assert_eq!(
        mutable_journal_bytes(&fixture.store, &fixture.session_id),
        before
    );
}

#[test]
fn migrated_v13_active_journal_is_unproven_and_a_byte_for_byte_no_op() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let paths = crate::persistence::storage::SecureStorePaths::prepare(&database).unwrap();
    let sqlite_path = paths.sqlite_path().unwrap();
    let connection = rusqlite::Connection::open(sqlite_path).unwrap();
    for migration in &crate::persistence::migrations::test_migrations()[..13] {
        connection.execute_batch(migration.sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations (
                     version, name, checksum_sha256, applied_at_unix_ms
                 ) VALUES (?1, ?2, ?3, 1)",
                params![
                    i64::from(migration.version),
                    migration.name,
                    migration.checksum_sha256.as_slice(),
                ],
            )
            .unwrap();
        connection
            .pragma_update(
                None,
                "application_id",
                crate::persistence::migrations::DUX_APPLICATION_ID,
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", migration.version)
            .unwrap();
    }
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();

    let owner = crate::persistence::process_liveness::current_process_instance().unwrap();
    connection
        .execute(
            "INSERT INTO scans (
                 scan_id, root_path, root_path_encoding, started_at_unix_ms,
                 completed_at_unix_ms, status
             ) VALUES ('scan:v13-active', ?1, 1, 998000, 999000, 'succeeded')",
            [b"/v13".as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_sessions (
                 session_id, plan_id, started_at_unix_ms, mode, estimated_bytes,
                 trigger_source, status, record_format_version, source_scan_id,
                 plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                 plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                 execution_owner_id, execution_generation, last_heartbeat_at_unix_ms,
                 cancellation_requested
             ) VALUES (
                 'session:v13-active', 'plan:v13-active', 1000500,
                 'permanent_safe', 8, 'manual', 'running', 2,
                 'scan:v13-active', 1000, 0, 1900, 0, ?1, 1, 1000600, 0
             )",
            [owner.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_items (
                 session_id, item_ordinal, rule_id, rule_revision, estimated_bytes,
                 final_status, record_format_version, candidate_id, category,
                 safety_tier, proposed_action, rule_schedule_eligible
             ) VALUES (
                 'session:v13-active', 0, 'fixture.v13-active', 1, 8,
                 'planned', 2, 'candidate:v13-active', 'application_cache',
                 'safe_regenerable', 'remove_known_regenerable_contents', 0
             )",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_item_paths (
                 session_id, item_ordinal, path_ordinal, target_path,
                 target_path_encoding, status
             ) VALUES ('session:v13-active', 0, 0, ?1, 1, 'planned')",
            [b"/v13/cache".as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_item_evidence (
                 session_id, item_ordinal, evidence_ordinal, evidence_kind,
                 path_value, path_value_encoding
             ) VALUES (
                 'session:v13-active', 0, 0, 'matched_path', ?1, 1
             )",
            [b"/v13/cache".as_slice()],
        )
        .unwrap();
    for (ordinal, warning) in [
        "estimated_bytes_unverified",
        "permanent_removal_cannot_be_undone",
    ]
    .into_iter()
    .enumerate()
    {
        connection
            .execute(
                "INSERT INTO cleanup_plan_warnings (
                     session_id, warning_ordinal, warning_kind
                 ) VALUES ('session:v13-active', ?1, ?2)",
                params![ordinal as i64, warning],
            )
            .unwrap();
    }
    drop(connection);
    drop(paths);

    let store = StoreCoordinator::open(&database).unwrap();
    let session_id = CleanupSessionId::new("session:v13-active").unwrap();
    let lease = store.acquire_cleanup_journal_lease(LOCK_TIMEOUT).unwrap();
    let migrated = lease
        .load(&session_id)
        .unwrap()
        .expect("the migrated v13 active graph must decode");
    assert_eq!(migrated.execution_provenance, None);
    assert!(matches!(
        migrated.lifecycle,
        JournalLifecycle::Active {
            phase: ActivePhase::Running,
            ..
        }
    ));
    assert_eq!(migrated.items.len(), 1);
    drop(lease);

    let before = mutable_journal_bytes(&store, &session_id);
    let result = store
        .acquire_cleanup_journal_lease(LOCK_TIMEOUT)
        .unwrap()
        .try_recover(&session_id, UNIX_EPOCH + Duration::from_millis(1_000_700))
        .unwrap();
    assert!(matches!(result, RecoveryClaimResult::Unproven));
    assert_eq!(mutable_journal_bytes(&store, &session_id), before);
}

#[test]
fn cleanup_journal_lease_is_exclusive_within_the_store() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let held = fixture.lease();
    assert_eq!(
        fixture
            .store
            .acquire_cleanup_journal_lease(Duration::ZERO)
            .err()
            .unwrap()
            .kind,
        HistoryErrorKind::Busy
    );
    drop(held);
    fixture
        .store
        .acquire_cleanup_journal_lease(LOCK_TIMEOUT)
        .unwrap();
}

#[test]
fn every_non_effect_validation_outcome_is_durable_and_terminal() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        6,
    );
    let mut claim = fixture.claim();
    let outcomes = [
        ValidationOutcome::Skipped,
        ValidationOutcome::Rejected,
        ValidationOutcome::Failed,
        ValidationOutcome::ChangedSincePlan,
        ValidationOutcome::Interrupted,
        ValidationOutcome::Unavailable,
    ];
    let expected = [
        PathStatus::Skipped,
        PathStatus::Rejected,
        PathStatus::Failed,
        PathStatus::ChangedSincePlan,
        PathStatus::Interrupted,
        PathStatus::Unavailable,
    ];
    for (item, outcome) in outcomes.into_iter().enumerate() {
        claim.begin_validation(item, 0).unwrap();
        claim
            .finish_validation(
                item,
                0,
                outcome,
                Some("validation_refused"),
                fixture.started_at + Duration::from_secs(2 + item as u64),
            )
            .unwrap();
    }
    let snapshot = claim.snapshot().unwrap();
    assert_eq!(
        snapshot
            .items
            .iter()
            .map(|item| item.paths[0].status)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        claim
            .terminalize(fixture.started_at + Duration::from_secs(9), None)
            .unwrap(),
        TerminalSessionStatus::Failed
    );
    drop(claim);
    for item in 0..6 {
        assert_eq!(
            fixture.candidate_status(item),
            CandidateHistoryStatus::Failed
        );
    }
    assert_eq!(fixture.claim_count(), 0);
}

#[test]
fn effect_outcomes_cover_eviction_and_explicit_failure() {
    let evict = Fixture::new(
        CleanupMode::EvictLocalCopy,
        CandidateAction::EvictLocalCopy,
        1,
    );
    let mut evict_claim = evict.claim();
    evict_claim.begin_validation(0, 0).unwrap();
    let evict_receipt = evict_claim
        .mark_effect_started(0, 0, evict.started_at + Duration::from_secs(2))
        .unwrap();
    evict_claim
        .finish_effect(
            &evict_receipt,
            EffectOutcome::Evicted,
            None,
            evict.started_at + Duration::from_secs(3),
        )
        .unwrap();
    assert_eq!(
        evict_claim
            .terminalize(evict.started_at + Duration::from_secs(4), Some(90))
            .unwrap(),
        TerminalSessionStatus::Completed
    );
    drop(evict_claim);
    assert_eq!(evict.candidate_status(0), CandidateHistoryStatus::Completed);
    assert_eq!(evict.claim_count(), 0);

    let failed = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut failed_claim = failed.claim();
    failed_claim.begin_validation(0, 0).unwrap();
    let failed_receipt = failed_claim
        .mark_effect_started(0, 0, failed.started_at + Duration::from_secs(2))
        .unwrap();
    failed_claim
        .finish_effect(
            &failed_receipt,
            EffectOutcome::Failed,
            Some("effect_failed"),
            failed.started_at + Duration::from_secs(3),
        )
        .unwrap();
    assert_eq!(
        failed_claim
            .terminalize(failed.started_at + Duration::from_secs(4), None)
            .unwrap(),
        TerminalSessionStatus::Failed
    );
    drop(failed_claim);
    assert_eq!(failed.candidate_status(0), CandidateHistoryStatus::Failed);
    assert_eq!(failed.claim_count(), 0);
}

#[test]
fn partial_completion_projects_each_candidate_from_its_own_item_result() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        2,
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    let receipt = claim
        .mark_effect_started(0, 0, fixture.started_at + Duration::from_secs(2))
        .unwrap();
    claim
        .finish_effect(
            &receipt,
            EffectOutcome::Removed,
            None,
            fixture.started_at + Duration::from_secs(3),
        )
        .unwrap();
    claim.begin_validation(1, 0).unwrap();
    claim
        .finish_validation(
            1,
            0,
            ValidationOutcome::Failed,
            Some("validation_failed"),
            fixture.started_at + Duration::from_secs(4),
        )
        .unwrap();
    assert_eq!(
        claim
            .terminalize(fixture.started_at + Duration::from_secs(5), Some(90))
            .unwrap(),
        TerminalSessionStatus::PartiallyCompleted
    );
    drop(claim);
    assert_eq!(
        fixture.candidate_status(0),
        CandidateHistoryStatus::Completed
    );
    assert_eq!(fixture.candidate_status(1), CandidateHistoryStatus::Failed);
    assert_eq!(fixture.claim_count(), 0);
}

#[test]
fn unknown_reconciliation_enforces_mode_and_accepts_explicit_failure() {
    let removed = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut removed_claim = removed.claim();
    removed_claim.begin_validation(0, 0).unwrap();
    let removed_receipt = removed_claim
        .mark_effect_started(0, 0, removed.started_at + Duration::from_secs(2))
        .unwrap();
    removed_claim
        .finish_effect(
            &removed_receipt,
            EffectOutcome::OutcomeUnknown,
            Some("effect_result_unavailable"),
            removed.started_at + Duration::from_secs(3),
        )
        .unwrap();
    assert_eq!(
        removed_claim
            .reconcile_unknown(
                0,
                0,
                ReconciledOutcome::Trashed,
                None,
                removed.started_at + Duration::from_secs(4),
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        removed_claim
            .reconcile_unknown(
                0,
                0,
                ReconciledOutcome::Evicted,
                None,
                removed.started_at + Duration::from_secs(4),
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    removed_claim
        .reconcile_unknown(
            0,
            0,
            ReconciledOutcome::Removed,
            None,
            removed.started_at + Duration::from_secs(4),
        )
        .unwrap();
    assert_eq!(
        removed_claim
            .terminalize(removed.started_at + Duration::from_secs(5), None)
            .unwrap(),
        TerminalSessionStatus::Completed
    );

    let failed = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut failed_claim = failed.claim();
    failed_claim.begin_validation(0, 0).unwrap();
    let failed_receipt = failed_claim
        .mark_effect_started(0, 0, failed.started_at + Duration::from_secs(2))
        .unwrap();
    failed_claim
        .finish_effect(
            &failed_receipt,
            EffectOutcome::OutcomeUnknown,
            Some("effect_result_unavailable"),
            failed.started_at + Duration::from_secs(3),
        )
        .unwrap();
    failed_claim
        .reconcile_unknown(
            0,
            0,
            ReconciledOutcome::Failed,
            Some("reconciled_failure"),
            failed.started_at + Duration::from_secs(4),
        )
        .unwrap();
    assert_eq!(
        failed_claim
            .terminalize(failed.started_at + Duration::from_secs(5), None)
            .unwrap(),
        TerminalSessionStatus::Failed
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn definitely_gone_owner_recovery_claims_a_new_generation_and_resumes_safely() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        3,
    );
    let claim = fixture.claim();
    claim
        .heartbeat(fixture.started_at + Duration::from_secs(2))
        .unwrap();
    claim.begin_validation(0, 0).unwrap();
    claim.begin_validation(1, 0).unwrap();
    claim
        .mark_effect_started(1, 0, fixture.started_at + Duration::from_secs(3))
        .unwrap();
    let active = claim.snapshot().unwrap();
    let JournalLifecycle::Active { fence, .. } = active.lifecycle else {
        panic!("claim did not produce active journal state");
    };
    let gone_owner = same_scope_changed_start_owner(&fence.owner);
    drop(claim);
    fixture.execute(
        "UPDATE cleanup_sessions SET execution_owner_id = ?2 WHERE session_id = ?1",
        params![fixture.session_id.as_str(), gone_owner],
    );

    let result = fixture
        .lease()
        .try_recover(
            &fixture.session_id,
            fixture.started_at + Duration::from_secs(5),
        )
        .unwrap();
    let RecoveryClaimResult::Claimed(mut recovered) = result else {
        panic!("definitely-gone same-scope owner was not recovered");
    };
    let snapshot = recovered.snapshot().unwrap();
    let JournalLifecycle::Active {
        phase,
        fence,
        cancellation_requested,
        ..
    } = snapshot.lifecycle
    else {
        panic!("recovery did not retain an active journal");
    };
    assert_eq!(phase, ActivePhase::Recovering);
    assert_eq!(fence.generation, 2);
    assert!(!cancellation_requested);
    assert_eq!(snapshot.items[0].paths[0].status, PathStatus::Planned);
    assert_eq!(snapshot.items[0].paths[0].attempt_generation, None);
    assert_eq!(
        snapshot.items[1].paths[0].status,
        PathStatus::OutcomeUnknown
    );
    assert_eq!(snapshot.items[1].paths[0].attempt_generation, Some(1));
    assert_eq!(snapshot.items[2].paths[0].status, PathStatus::Planned);

    recovered
        .reconcile_unknown(
            1,
            0,
            ReconciledOutcome::Removed,
            None,
            fixture.started_at + Duration::from_secs(6),
        )
        .unwrap();
    recovered.resume_recovery().unwrap();
    let resumed = recovered.snapshot().unwrap();
    assert!(matches!(
        resumed.lifecycle,
        JournalLifecycle::Active {
            phase: ActivePhase::Running,
            ref fence,
            ..
        } if fence.generation == 2
    ));
    recovered.begin_validation(0, 0).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn same_scope_changed_start_owner(owner: &ProcessInstanceId) -> String {
    let mut components = owner
        .as_str()
        .split(':')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(components.len(), 6);
    let current = u64::from_str_radix(&components[3], 16).unwrap();
    let changed = current.checked_add(1).unwrap_or(current - 1);
    components[3] = format!("{changed:x}");
    let encoded = components.join(":");
    ProcessInstanceId::from_stored(&encoded).unwrap();
    encoded
}

#[test]
fn sub_millisecond_effect_start_is_canonical_and_receipt_remains_revalidatable() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    let requested = fixture.started_at + Duration::from_secs(2) + Duration::from_nanos(456_789);
    let receipt = claim.mark_effect_started(0, 0, requested).unwrap();
    claim.revalidate_effect_receipt(&receipt).unwrap();

    let snapshot = claim.snapshot().unwrap();
    assert_eq!(
        snapshot.items[0].paths[0].effect_started_at,
        Some(fixture.started_at + Duration::from_secs(2))
    );
    claim
        .finish_effect(
            &receipt,
            EffectOutcome::Removed,
            None,
            fixture.started_at + Duration::from_secs(3),
        )
        .unwrap();
}

#[test]
fn ambiguous_effect_start_retry_retains_the_original_fine_grained_instant() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();

    let millisecond = fixture.started_at + Duration::from_secs(2);
    let original = millisecond + Duration::from_nanos(900_000);
    let replacement = millisecond + Duration::from_nanos(100_000);
    claim.fail_next_write_after_commit_and_reconcile_read_for_test();

    assert_eq!(
        claim.mark_effect_started(0, 0, original).unwrap_err().kind,
        HistoryErrorKind::DatabaseUnavailable
    );

    // The retry argument is deliberately earlier within the same persisted
    // millisecond. The retained pending capability must ignore it and keep the
    // original fine-grained ordering instant.
    let receipt = claim.mark_effect_started(0, 0, replacement).unwrap();
    assert_eq!(
        claim
            .finish_effect(&receipt, EffectOutcome::Removed, None, replacement)
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    claim
        .finish_effect(&receipt, EffectOutcome::Removed, None, original)
        .unwrap();
}

#[test]
fn ambiguous_terminal_commit_reconciles_candidate_projection_and_claim_removal() {
    let fixture = Fixture::new_with_selected(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
        &[0],
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    claim
        .finish_validation(
            0,
            0,
            ValidationOutcome::DryRun,
            None,
            fixture.started_at + Duration::from_secs(2),
        )
        .unwrap();
    claim.fail_next_write_after_commit_and_reconcile_read_for_test();
    assert_eq!(
        claim
            .terminalize(fixture.started_at + Duration::from_secs(3), None)
            .unwrap_err()
            .kind,
        HistoryErrorKind::DatabaseUnavailable
    );
    assert_eq!(
        fixture.candidate_status(0),
        CandidateHistoryStatus::Selected
    );
    assert_eq!(fixture.claim_count(), 0);
    assert_eq!(
        claim
            .terminalize(fixture.started_at + Duration::from_secs(3), None)
            .unwrap(),
        TerminalSessionStatus::DryRun
    );
}

#[test]
fn effect_completion_before_its_receipt_is_rejected_without_losing_the_claim() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    let effect_started =
        fixture.started_at + Duration::from_secs(2) + Duration::from_nanos(900_000);
    let receipt = claim.mark_effect_started(0, 0, effect_started).unwrap();
    assert_eq!(
        claim
            .finish_effect(
                &receipt,
                EffectOutcome::Removed,
                None,
                fixture.started_at + Duration::from_secs(2) + Duration::from_nanos(100_000),
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        claim.snapshot().unwrap().items[0].paths[0].status,
        PathStatus::EffectStarted
    );
    claim
        .finish_effect(
            &receipt,
            EffectOutcome::Failed,
            Some("effect_failed"),
            fixture.started_at + Duration::from_secs(3),
        )
        .unwrap();
}

#[test]
fn cancellation_after_effect_intent_blocks_the_call_and_records_known_no_effect() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    let receipt = claim
        .mark_effect_started(0, 0, fixture.started_at + Duration::from_secs(2))
        .unwrap();
    claim.request_cancellation().unwrap();
    assert_eq!(
        claim.revalidate_effect_receipt(&receipt).unwrap_err().kind,
        HistoryErrorKind::InvalidTransition
    );
    claim
        .cancel_effect_before_call(&receipt, fixture.started_at + Duration::from_secs(3))
        .unwrap();
    let snapshot = claim.snapshot().unwrap();
    let path = &snapshot.items[0].paths[0];
    assert_eq!(path.status, PathStatus::Interrupted);
    assert_eq!(path.attempt_generation, Some(1));
    assert_eq!(path.effect_started_at, None);
    assert_eq!(
        path.completed_at,
        Some(fixture.started_at + Duration::from_secs(3))
    );
    assert_eq!(
        claim
            .terminalize(fixture.started_at + Duration::from_secs(4), None)
            .unwrap(),
        TerminalSessionStatus::Cancelled
    );
    drop(claim);
    assert_eq!(
        fixture.candidate_status(0),
        CandidateHistoryStatus::Discovered
    );
    assert_eq!(fixture.claim_count(), 0);
}

#[test]
fn cancellation_requested_after_every_path_was_skipped_preserves_rejected_result() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    claim
        .finish_validation(
            0,
            0,
            ValidationOutcome::Skipped,
            None,
            fixture.started_at + Duration::from_secs(2),
        )
        .unwrap();
    claim.request_cancellation().unwrap();
    assert_eq!(
        claim
            .terminalize(fixture.started_at + Duration::from_secs(3), None)
            .unwrap(),
        TerminalSessionStatus::Rejected
    );
    drop(claim);
    assert_eq!(fixture.candidate_status(0), CandidateHistoryStatus::Failed);
    assert_eq!(fixture.claim_count(), 0);
}

#[test]
fn loader_rejects_effect_timestamp_on_validation_only_outcome() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    claim
        .finish_validation(
            0,
            0,
            ValidationOutcome::Skipped,
            None,
            fixture.started_at + Duration::from_secs(2),
        )
        .unwrap();
    drop(claim);
    fixture.execute(
        "UPDATE cleanup_item_paths SET effect_started_at_unix_ms = ?2 WHERE session_id = ?1",
        params![
            fixture.session_id.as_str(),
            SESSION_STARTED_MILLIS as i64 + 1_500
        ],
    );
    assert_eq!(
        fixture.lease().load(&fixture.session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn loader_rejects_effect_start_later_than_the_fenced_heartbeat() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    let receipt = claim
        .mark_effect_started(0, 0, fixture.started_at + Duration::from_secs(2))
        .unwrap();
    claim
        .finish_effect(
            &receipt,
            EffectOutcome::Removed,
            None,
            fixture.started_at + Duration::from_secs(3),
        )
        .unwrap();
    drop(claim);
    fixture.execute(
        "UPDATE cleanup_item_paths
         SET effect_started_at_unix_ms = (
             SELECT last_heartbeat_at_unix_ms + 1 FROM cleanup_sessions
             WHERE cleanup_sessions.session_id = cleanup_item_paths.session_id
         ) WHERE session_id = ?1",
        [fixture.session_id.as_str()],
    );
    assert_eq!(
        fixture.lease().load(&fixture.session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn premature_terminalize_error_keeps_the_claim_usable() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let mut claim = fixture.claim();
    assert_eq!(
        claim
            .terminalize(fixture.started_at + Duration::from_secs(2), None)
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    claim.begin_validation(0, 0).unwrap();
    claim
        .finish_validation(
            0,
            0,
            ValidationOutcome::Skipped,
            None,
            fixture.started_at + Duration::from_secs(3),
        )
        .unwrap();
}

#[test]
fn heartbeat_is_monotonic_and_a_rejected_update_keeps_the_claim_usable() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let claim = fixture.claim();
    assert_eq!(
        claim
            .heartbeat(fixture.started_at + Duration::from_millis(500))
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    claim
        .heartbeat(fixture.started_at + Duration::from_secs(2))
        .unwrap();
    assert!(matches!(
        claim.snapshot().unwrap().lifecycle,
        JournalLifecycle::Active { heartbeat_at, .. }
            if heartbeat_at == fixture.started_at + Duration::from_secs(2)
    ));
}

#[test]
fn invalid_error_category_is_rejected_without_advancing_the_path() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    assert_eq!(
        claim
            .finish_validation(
                0,
                0,
                ValidationOutcome::Failed,
                Some("Contains spaces"),
                fixture.started_at + Duration::from_secs(2),
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidInput
    );
    assert_eq!(
        claim.snapshot().unwrap().items[0].paths[0].status,
        PathStatus::Validating
    );
    claim
        .finish_validation(
            0,
            0,
            ValidationOutcome::Failed,
            Some("validation_failed"),
            fixture.started_at + Duration::from_secs(2),
        )
        .unwrap();
}

#[test]
fn dry_run_cannot_record_effect_intent() {
    let fixture = Fixture::new(
        CleanupMode::DryRun,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    assert_eq!(
        claim
            .mark_effect_started(0, 0, fixture.started_at + Duration::from_secs(2))
            .err()
            .unwrap()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        claim.snapshot().unwrap().items[0].paths[0].status,
        PathStatus::Validating
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn prior_boot_recovery_is_a_typed_byte_for_byte_no_op() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        2,
    );
    let claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    claim.begin_validation(1, 0).unwrap();
    claim
        .mark_effect_started(1, 0, fixture.started_at + Duration::from_secs(3))
        .unwrap();
    let active = claim.snapshot().unwrap();
    let JournalLifecycle::Active { fence, .. } = active.lifecycle else {
        panic!("claim did not produce active journal state");
    };
    let prior_boot_owner = changed_scope_owner(&fence.owner);
    let prior_boot_scope = owner_scope_bytes(&prior_boot_owner);
    drop(claim);
    fixture.execute(
        "UPDATE cleanup_sessions
         SET execution_owner_id = ?2, execution_boot_scope_v1_sha256 = ?3
         WHERE session_id = ?1",
        params![
            fixture.session_id.as_str(),
            prior_boot_owner,
            prior_boot_scope
        ],
    );
    let before = mutable_journal_bytes(&fixture.store, &fixture.session_id);
    let result = fixture
        .lease()
        .try_recover(
            &fixture.session_id,
            fixture.started_at + Duration::from_secs(10),
        )
        .unwrap();
    assert!(matches!(result, RecoveryClaimResult::PriorBoot));
    assert_eq!(
        mutable_journal_bytes(&fixture.store, &fixture.session_id),
        before
    );
    assert_eq!(fixture.claim_count(), 2);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn prior_boot_debt_does_not_block_a_fresh_candidate_for_the_same_path() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let old_claim = fixture.claim();
    old_claim.begin_validation(0, 0).unwrap();
    let active = old_claim.snapshot().unwrap();
    let JournalLifecycle::Active { fence, .. } = active.lifecycle else {
        panic!("claim did not produce active journal state");
    };
    let prior_boot_owner = changed_scope_owner(&fence.owner);
    let prior_boot_scope = owner_scope_bytes(&prior_boot_owner);
    drop(old_claim);
    fixture.execute(
        "UPDATE cleanup_sessions
         SET execution_owner_id = ?2, execution_boot_scope_v1_sha256 = ?3
         WHERE session_id = ?1",
        params![
            fixture.session_id.as_str(),
            prior_boot_owner,
            prior_boot_scope
        ],
    );
    let old_before = mutable_journal_bytes(&fixture.store, &fixture.session_id);
    assert!(matches!(
        fixture
            .lease()
            .try_recover(
                &fixture.session_id,
                fixture.started_at + Duration::from_secs(10),
            )
            .unwrap(),
        RecoveryClaimResult::PriorBoot
    ));

    let root = fixture._temp.path().join("root");
    let fresh_scan_id = "scan:cleanup-journal-fresh";
    start_scan(&fixture.store, &root, fresh_scan_id);
    let fresh_candidate_id = CandidateId::new("candidate:journal-fresh").unwrap();
    let fresh_candidate = fixture_candidate(
        fresh_candidate_id.as_str(),
        fresh_scan_id,
        &fixture_rule(CandidateAction::RemoveKnownRegenerableContents),
        fixture.plan.items()[0].paths()[0].clone(),
        101,
    );
    fixture
        .store
        .record_candidate_discovered(
            &NewCandidateRecord::try_from_candidate(
                &fresh_candidate,
                UNIX_EPOCH + Duration::from_secs(1_750_000_003),
            )
            .unwrap(),
        )
        .unwrap();
    let fresh_plan = CleanupPlan::try_from_candidates_for_persistence_test(
        CleanupPlanId::new("plan:cleanup-journal-fresh").unwrap(),
        UNIX_EPOCH + Duration::from_secs(PLAN_CREATED_SECONDS + 1),
        CleanupMode::PermanentSafe,
        &[fresh_candidate],
    )
    .unwrap();
    let fresh_session_id = CleanupSessionId::new("session:cleanup-journal-fresh").unwrap();
    let fresh_started_at = fixture.started_at + Duration::from_secs(20);
    fixture
        .store
        .record_cleanup_session_planned(
            &NewCleanupSessionRecord::try_from_plan(
                fresh_session_id.clone(),
                &fresh_plan,
                fresh_started_at,
                CleanupTrigger::Manual,
            )
            .unwrap(),
        )
        .unwrap();
    let fresh_claim = fixture
        .lease()
        .claim_planned(&fresh_session_id, fresh_started_at)
        .unwrap();
    let fresh = fresh_claim.snapshot().unwrap();
    assert!(matches!(
        fresh.lifecycle,
        JournalLifecycle::Active {
            phase: ActivePhase::Running,
            ..
        }
    ));
    assert_eq!(fresh.items[0].frozen.candidate_id, fresh_candidate_id);
    assert_eq!(fixture.candidate_status(0), CandidateHistoryStatus::Planned);
    assert_eq!(fixture.claim_count(), 1);
    assert_eq!(
        mutable_journal_bytes(&fixture.store, &fixture.session_id),
        old_before
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn foreign_host_recovery_is_a_typed_byte_for_byte_no_op() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    let active = claim.snapshot().unwrap();
    let JournalLifecycle::Active { fence, .. } = active.lifecycle else {
        panic!("claim did not produce active journal state");
    };
    let gone_owner = same_scope_changed_start_owner(&fence.owner);
    drop(claim);
    let mut foreign_host: Vec<u8> = fixture.store.with_connection(|connection| {
        connection
            .query_row(
                "SELECT execution_host_identity_v1_sha256
                 FROM cleanup_sessions WHERE session_id = ?1",
                [fixture.session_id.as_str()],
                |row| row.get(0),
            )
            .unwrap()
    });
    foreign_host[0] ^= 0xff;
    fixture.execute(
        "UPDATE cleanup_sessions
         SET execution_owner_id = ?2, execution_host_identity_v1_sha256 = ?3
         WHERE session_id = ?1",
        params![fixture.session_id.as_str(), gone_owner, foreign_host],
    );
    let before = mutable_journal_bytes(&fixture.store, &fixture.session_id);
    let result = fixture
        .lease()
        .try_recover(
            &fixture.session_id,
            fixture.started_at + Duration::from_secs(10),
        )
        .unwrap();
    assert!(matches!(result, RecoveryClaimResult::ForeignHost));
    assert_eq!(
        mutable_journal_bytes(&fixture.store, &fixture.session_id),
        before
    );
    assert_eq!(fixture.claim_count(), 1);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn changed_scope_owner(owner: &ProcessInstanceId) -> String {
    let mut components = owner
        .as_str()
        .split(':')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(components.len(), 6);
    assert_eq!(components[4].len(), 64);
    let replacement = if components[4].starts_with('0') {
        '1'
    } else {
        '0'
    };
    components[4].replace_range(..1, &replacement.to_string());
    let encoded = components.join(":");
    ProcessInstanceId::from_stored(&encoded).unwrap();
    encoded
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn owner_scope_bytes(owner: &str) -> Vec<u8> {
    let scope = owner.split(':').nth(4).unwrap().as_bytes();
    assert_eq!(scope.len(), 64);
    scope
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16).unwrap() as u8;
            let low = (pair[1] as char).to_digit(16).unwrap() as u8;
            (high << 4) | low
        })
        .collect()
}

#[test]
fn windows_unproven_owner_recovery_is_a_byte_for_byte_no_op() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let claim = fixture.claim();
    claim.begin_validation(0, 0).unwrap();
    let active = claim.snapshot().unwrap();
    let JournalLifecycle::Active { fence, .. } = active.lifecycle else {
        panic!("claim did not produce active journal state");
    };
    let windows_owner = windows_unproven_changed_start_owner(&fence.owner);
    drop(claim);
    fixture.execute(
        "UPDATE cleanup_sessions
         SET execution_owner_id = ?2,
             execution_host_identity_v1_sha256 = NULL,
             execution_boot_scope_v1_sha256 = NULL,
             execution_recovery_policy = NULL
         WHERE session_id = ?1",
        params![fixture.session_id.as_str(), windows_owner],
    );

    let before = mutable_journal_bytes(&fixture.store, &fixture.session_id);
    let result = fixture
        .lease()
        .try_recover(
            &fixture.session_id,
            fixture.started_at + Duration::from_secs(10),
        )
        .unwrap();
    assert!(matches!(result, RecoveryClaimResult::Unproven));
    assert_eq!(
        mutable_journal_bytes(&fixture.store, &fixture.session_id),
        before
    );
    assert_eq!(fixture.claim_count(), 1);
}

fn windows_unproven_changed_start_owner(owner: &ProcessInstanceId) -> String {
    let components = owner.as_str().split(':').collect::<Vec<_>>();
    assert_eq!(components.len(), 6);
    let current = u64::from_str_radix(components[3], 16).unwrap();
    let changed = current.checked_add(1).unwrap_or(current - 1);
    let encoded = format!(
        "1:w:{}:{changed:x}:-:00000000000000000000000000000001",
        components[2]
    );
    ProcessInstanceId::from_stored(&encoded).unwrap();
    encoded
}

#[test]
fn stale_generation_and_owner_cannot_mutate_an_active_journal() {
    let generation_fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let generation_claim = generation_fixture.claim();
    generation_fixture.execute(
        "UPDATE cleanup_sessions SET execution_generation = 2 WHERE session_id = ?1",
        [generation_fixture.session_id.as_str()],
    );
    assert_eq!(
        generation_claim
            .heartbeat(generation_fixture.started_at + Duration::from_secs(2))
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        generation_claim.begin_validation(0, 0).unwrap_err().kind,
        HistoryErrorKind::InvalidTransition
    );

    let owner_fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let owner_claim = owner_fixture.claim();
    let replacement = crate::persistence::process_liveness::current_process_instance().unwrap();
    owner_fixture.execute(
        "UPDATE cleanup_sessions SET execution_owner_id = ?2 WHERE session_id = ?1",
        params![owner_fixture.session_id.as_str(), replacement.as_str()],
    );
    assert_eq!(
        owner_claim
            .heartbeat(owner_fixture.started_at + Duration::from_secs(2))
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        owner_claim.begin_validation(0, 0).unwrap_err().kind,
        HistoryErrorKind::InvalidTransition
    );
}
