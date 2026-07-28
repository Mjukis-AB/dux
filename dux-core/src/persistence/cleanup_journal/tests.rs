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
    TrashExecutionAdmission, TrashExecutionError, TrashPlatformEffect, TrashPlatformError,
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
use crate::persistence::StoreCoordinator;
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

const PLAN_CREATED_SECONDS: u64 = 1_750_000_010;
const SESSION_STARTED_MILLIS: u64 = 1_750_000_011_123;
const LOCK_TIMEOUT: Duration = Duration::from_secs(2);

struct Fixture {
    _temp: TempDir,
    store: Arc<StoreCoordinator>,
    session_id: CleanupSessionId,
    started_at: SystemTime,
    expires_at: SystemTime,
    plan: CleanupPlan,
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
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let store = StoreCoordinator::open(&database).unwrap();
        let scan_id = "scan:cleanup-journal";
        start_scan(&store, &root, scan_id);

        let rule = fixture_rule(action);
        let candidates = (0..item_count)
            .map(|index| {
                fixture_candidate(
                    &format!("candidate:journal-{index}"),
                    scan_id,
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
        Self {
            _temp: temp,
            store,
            session_id,
            started_at,
            expires_at,
            plan,
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
            &NewScanRecord::try_new(
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
    const QUERIES: [&str; 3] = [
        "SELECT status, completed_at_unix_ms, verified_capacity_delta_bytes, execution_owner_id, execution_generation, last_heartbeat_at_unix_ms, cancellation_requested FROM cleanup_sessions WHERE session_id = ?1",
        "SELECT item_ordinal, final_status, error_category FROM cleanup_items WHERE session_id = ?1 ORDER BY item_ordinal",
        "SELECT item_ordinal, path_ordinal, attempt_generation, status, error_category, effect_started_at_unix_ms, completed_at_unix_ms FROM cleanup_item_paths WHERE session_id = ?1 ORDER BY item_ordinal, path_ordinal",
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

    assert_eq!(
        admission
            .execute_with_at(&mut platform, fixture.started_at + Duration::from_secs(3))
            .unwrap_err(),
        TrashExecutionError::Platform(TrashPlatformError::OutcomeUnknown)
    );
    assert_eq!(platform.calls, 1);
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

    assert_eq!(
        error,
        crate::cleanup::executor::TrashAdmissionError::UnsupportedEffectMode
    );
    let journal = fixture.lease().load(&fixture.session_id).unwrap().unwrap();
    assert_eq!(journal.items[0].paths[0].status, PathStatus::Rejected);
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
fn global_permanent_cleanup_disable_rejects_before_effect_started() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    fixture.store.set_permanent_cleanup_enabled(false).unwrap();
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
    drop(fixture.claim());
    let before = mutable_journal_bytes(&fixture.store, &fixture.session_id);
    let result = fixture
        .lease()
        .try_recover(
            &fixture.session_id,
            fixture.started_at + Duration::from_secs(10),
        )
        .unwrap();
    assert!(matches!(result, RecoveryClaimResult::OwnerAlive));
    let after = mutable_journal_bytes(&fixture.store, &fixture.session_id);
    assert_eq!(after, before);
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
fn foreign_scope_recovery_refusal_is_a_byte_for_byte_state_no_op() {
    let fixture = Fixture::new(
        CleanupMode::PermanentSafe,
        CandidateAction::RemoveKnownRegenerableContents,
        1,
    );
    let claim = fixture.claim();
    let active = claim.snapshot().unwrap();
    let JournalLifecycle::Active { fence, .. } = active.lifecycle else {
        panic!("claim did not produce active journal state");
    };
    let foreign_owner = changed_scope_owner(&fence.owner);
    drop(claim);
    fixture.execute(
        "UPDATE cleanup_sessions SET execution_owner_id = ?2 WHERE session_id = ?1",
        params![fixture.session_id.as_str(), foreign_owner],
    );
    let before = mutable_journal_bytes(&fixture.store, &fixture.session_id);
    let result = fixture
        .lease()
        .try_recover(
            &fixture.session_id,
            fixture.started_at + Duration::from_secs(10),
        )
        .unwrap();
    assert!(matches!(result, RecoveryClaimResult::LivenessUnknown));
    assert_eq!(
        mutable_journal_bytes(&fixture.store, &fixture.session_id),
        before
    );
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
