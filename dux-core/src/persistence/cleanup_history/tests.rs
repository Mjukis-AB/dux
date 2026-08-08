use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use tempfile::TempDir;

use super::*;
use crate::domain::{
    Candidate, CandidateInput, LocalizedTextKey, ProvenanceUrl, Rule, RuleDefinition, RuleGuards,
    RuleMatcher, RuleMatcherDefinition, RuleScope,
};
use crate::persistence::StoreCoordinator;
use crate::persistence::candidate_history::{CandidateReviewTransition, NewCandidateRecord};
use crate::persistence::history::{
    NewScanRecord, ScanCompletionRecord, ScanCounts, TerminalScanStatus,
};

fn rule(
    id: &str,
    category: CandidateCategory,
    safety: SafetyTier,
    action: CandidateAction,
) -> Rule {
    rule_with_schedule(id, category, safety, action, false)
}

fn rule_with_schedule(
    id: &str,
    category: CandidateCategory,
    safety: SafetyTier,
    action: CandidateAction,
    schedule_eligible: bool,
) -> Rule {
    Rule::try_new(RuleDefinition {
        reference: RuleRef::new(RuleId::new(id).unwrap(), RuleRevision::new(3).unwrap()),
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
        guards: RuleGuards::try_new(
            None,
            0,
            Vec::new(),
            action == CandidateAction::EvictLocalCopy,
        )
        .unwrap(),
        safety,
        action,
        schedule_eligible,
        explanation_key: LocalizedTextKey::new("fixture.cleanup.explanation").unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/cleanup").unwrap()],
    })
    .unwrap()
}

fn rust_target_rule() -> Rule {
    Rule::try_new(RuleDefinition {
        reference: RuleRef::new(
            RuleId::new("developer.rust.target").unwrap(),
            RuleRevision::new(crate::domain::SAFE_RUST_RULE_REVISION).unwrap(),
        ),
        title_key: LocalizedTextKey::new("fixture.cleanup.title").unwrap(),
        category: CandidateCategory::DeveloperArtifact,
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
        guards: RuleGuards::try_new(
            Some(crate::domain::SAFE_RUST_RULE_MINIMUM_AGE),
            0,
            Vec::new(),
            false,
        )
        .unwrap(),
        safety: SafetyTier::SafeRegenerable,
        action: CandidateAction::RemoveKnownRegenerableContents,
        schedule_eligible: false,
        explanation_key: LocalizedTextKey::new("fixture.cleanup.explanation").unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/cleanup").unwrap()],
    })
    .unwrap()
}

fn candidate(id: &str, scan_id: &str, rule: &Rule, path: PathBuf, bytes: u64) -> Candidate {
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

fn persist_candidate(store: &StoreCoordinator, candidate: &Candidate, second: u64) {
    store
        .record_candidate_discovered(
            &NewCandidateRecord::try_from_candidate(
                candidate,
                UNIX_EPOCH + Duration::from_secs(second),
            )
            .unwrap(),
        )
        .unwrap();
}

fn plan(id: &str, mode: CleanupMode, candidates: &[Candidate]) -> CleanupPlan {
    CleanupPlan::try_from_candidates_for_persistence_test(
        CleanupPlanId::new(id).unwrap(),
        UNIX_EPOCH + Duration::from_secs(1_750_000_010),
        mode,
        candidates,
    )
    .unwrap()
}

fn candidate_status(store: &StoreCoordinator, id: &CandidateId) -> CandidateHistoryStatus {
    let Some(StoredCandidateRecord::Complete(candidate)) = store.load_candidate(id).unwrap() else {
        panic!("candidate was not a complete record");
    };
    candidate.status
}

#[test]
fn planned_dry_run_round_trip_preserves_every_proposed_effect_after_reopen() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("root");
    let scan_id = "scan:cleanup-roundtrip";
    let rules = [
        rule(
            "fixture.cleanup.permanent",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        ),
        rule(
            "fixture.cleanup.trash",
            CandidateCategory::LargeReviewItem,
            SafetyTier::ReviewRequired,
            CandidateAction::MoveToTrash,
        ),
        rule(
            "fixture.cleanup.evict",
            CandidateCategory::CloudFile,
            SafetyTier::SafeEvictable,
            CandidateAction::EvictLocalCopy,
        ),
    ];
    let candidates = rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            candidate(
                &format!("candidate:cleanup-{index}"),
                scan_id,
                rule,
                root.join(format!("cleanup-fixture-{index}")),
                (index + 1) as u64 * 10,
            )
        })
        .collect::<Vec<_>>();
    let cleanup_plan = plan("plan:cleanup-roundtrip", CleanupMode::DryRun, &candidates);
    let session_id = CleanupSessionId::new("session:cleanup-roundtrip").unwrap();
    let started = UNIX_EPOCH + Duration::from_millis(1_750_000_011_123);
    {
        let store = StoreCoordinator::open(&database).unwrap();
        start_scan(&store, &root, scan_id);
        for (index, candidate) in candidates.iter().enumerate() {
            persist_candidate(&store, candidate, 1_750_000_002 + index as u64);
        }
        store
            .record_cleanup_session_planned(
                &NewCleanupSessionRecord::try_from_plan(
                    session_id.clone(),
                    &cleanup_plan,
                    started,
                    CleanupTrigger::Manual,
                )
                .unwrap(),
            )
            .unwrap();
    }
    let store = StoreCoordinator::open(&database).unwrap();
    let StoredCleanupSessionRecord::Planned(stored) =
        store.load_cleanup_session(&session_id).unwrap().unwrap()
    else {
        panic!("format-2 session was not complete");
    };
    assert_eq!(stored.plan_id, cleanup_plan.id().clone());
    assert_eq!(stored.started_at, started);
    assert_eq!(stored.source_scan_id, cleanup_plan.source_scan_id().clone());
    assert_eq!(stored.plan_created_at, cleanup_plan.created_at());
    assert_eq!(stored.plan_expires_at, cleanup_plan.expires_at());
    assert_eq!(stored.mode, CleanupMode::DryRun);
    assert_eq!(stored.estimated_bytes, 60);
    assert_eq!(stored.trigger, CleanupTrigger::Manual);
    assert_eq!(
        stored.candidate_status_coupling,
        CandidateStatusCoupling::PlanClaimsV1
    );
    assert_eq!(stored.items.len(), 3);
    assert!(
        stored.items.iter().all(|item| {
            item.prior_review_status == Some(CandidatePriorReviewStatus::Discovered)
        })
    );
    assert_eq!(
        stored.items[0].proposed_action,
        CandidateAction::RemoveKnownRegenerableContents
    );
    assert_eq!(
        stored.items[1].proposed_action,
        CandidateAction::MoveToTrash
    );
    assert_eq!(
        stored.items[2].proposed_action,
        CandidateAction::EvictLocalCopy
    );
    assert_eq!(stored.warnings, cleanup_plan.warnings());
}

#[test]
fn selected_review_state_preserves_plan_checks_and_dismissal_blocks_new_plans() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    let scan_id = "scan:cleanup-review-state";
    start_scan(&store, &root, scan_id);
    let policy = rule(
        "fixture.cleanup.review-state",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );

    let selected = candidate(
        "candidate:cleanup-selected",
        scan_id,
        &policy,
        root.join("cleanup-fixture-selected"),
        10,
    );
    persist_candidate(&store, &selected, 1_750_000_002);
    store
        .transition_candidate_review_status(selected.id(), CandidateReviewTransition::Select)
        .unwrap();
    let selected_plan = plan(
        "plan:cleanup-selected",
        CleanupMode::PermanentSafe,
        std::slice::from_ref(&selected),
    );
    let selected_session = CleanupSessionId::new("session:cleanup-selected").unwrap();
    store
        .record_cleanup_session_planned(
            &NewCleanupSessionRecord::try_from_plan(
                selected_session.clone(),
                &selected_plan,
                UNIX_EPOCH + Duration::from_secs(1_750_000_011),
                CleanupTrigger::Manual,
            )
            .unwrap(),
        )
        .unwrap();
    let error = store
        .transition_candidate_review_status(
            selected.id(),
            CandidateReviewTransition::DismissSelected,
        )
        .unwrap_err();
    assert_eq!(error.kind, HistoryErrorKind::InvalidTransition);
    let Some(StoredCandidateRecord::Complete(stored_candidate)) =
        store.load_candidate(selected.id()).unwrap()
    else {
        panic!("planned candidate was not complete");
    };
    assert_eq!(stored_candidate.status, CandidateHistoryStatus::Planned);
    store.with_connection(|connection| {
        let prior: String = connection
            .query_row(
                "SELECT prior_review_status FROM candidate_plan_claims
                 WHERE candidate_id = ?1",
                [selected.id().as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(prior, "selected");
    });
    assert!(matches!(
        store.load_cleanup_session(&selected_session).unwrap(),
        Some(StoredCleanupSessionRecord::Planned(_))
    ));

    let dismissed = candidate(
        "candidate:cleanup-dismissed",
        scan_id,
        &policy,
        root.join("cleanup-fixture-dismissed"),
        10,
    );
    persist_candidate(&store, &dismissed, 1_750_000_003);
    store
        .transition_candidate_review_status(
            dismissed.id(),
            CandidateReviewTransition::DismissDiscovered,
        )
        .unwrap();
    let dismissed_plan = plan(
        "plan:cleanup-dismissed",
        CleanupMode::PermanentSafe,
        std::slice::from_ref(&dismissed),
    );
    let error = store
        .record_cleanup_session_planned(
            &NewCleanupSessionRecord::try_from_plan(
                CleanupSessionId::new("session:cleanup-dismissed").unwrap(),
                &dismissed_plan,
                UNIX_EPOCH + Duration::from_secs(1_750_000_012),
                CleanupTrigger::Manual,
            )
            .unwrap(),
        )
        .unwrap_err();
    assert_eq!(error.kind, HistoryErrorKind::InvalidInput);
    assert!(
        store
            .load_cleanup_session(&CleanupSessionId::new("session:cleanup-dismissed").unwrap())
            .unwrap()
            .is_none()
    );
}

#[test]
fn ordinary_plan_claim_cannot_borrow_trusted_rust_target_coupling() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    let scan_id = "scan:cleanup-generic-blocked";
    start_scan(&store, &root, scan_id);
    let policy = rust_target_rule();
    let candidate = candidate(
        "candidate:cleanup-generic-blocked",
        scan_id,
        &policy,
        root.join("cleanup-fixture"),
        10,
    );
    persist_candidate(&store, &candidate, 1_750_000_002);
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO candidate_blockers (
                     candidate_id, blocker_ordinal, blocker_kind
                 ) VALUES (?1, 0, 'protected_path')",
                [candidate.id().as_str()],
            )
            .unwrap();
    });

    let session_id = CleanupSessionId::new("session:cleanup-generic-blocked").unwrap();
    let record = NewCleanupSessionRecord::try_from_plan(
        session_id.clone(),
        &plan(
            "plan:cleanup-generic-blocked",
            CleanupMode::PermanentSafe,
            std::slice::from_ref(&candidate),
        ),
        UNIX_EPOCH + Duration::from_secs(1_750_000_011),
        CleanupTrigger::Manual,
    )
    .unwrap();
    assert_eq!(
        store
            .record_cleanup_session_planned(&record)
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidInput
    );
    assert_eq!(
        candidate_status(&store, candidate.id()),
        CandidateHistoryStatus::Discovered
    );
    assert!(store.load_cleanup_session(&session_id).unwrap().is_none());
    store.with_connection(|connection| {
        let claims: i64 = connection
            .query_row("SELECT COUNT(*) FROM candidate_plan_claims", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(claims, 0);
    });
}

#[test]
fn ordinary_rust_target_claim_rejects_a_forged_blocker_on_reopen() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    let scan_id = "scan:cleanup-ordinary-rust-target";
    start_scan(&store, &root, scan_id);
    let candidate = candidate(
        "candidate:cleanup-ordinary-rust-target",
        scan_id,
        &rust_target_rule(),
        root.join("cleanup-fixture"),
        10,
    );
    persist_candidate(&store, &candidate, 1_750_000_002);
    let session_id = CleanupSessionId::new("session:cleanup-ordinary-rust-target").unwrap();
    store
        .record_cleanup_session_planned(
            &NewCleanupSessionRecord::try_from_plan(
                session_id.clone(),
                &plan(
                    "plan:cleanup-ordinary-rust-target",
                    CleanupMode::PermanentSafe,
                    std::slice::from_ref(&candidate),
                ),
                UNIX_EPOCH + Duration::from_secs(1_750_000_011),
                CleanupTrigger::Manual,
            )
            .unwrap(),
        )
        .unwrap();
    let Some(StoredCleanupSessionRecord::Planned(stored)) =
        store.load_cleanup_session(&session_id).unwrap()
    else {
        panic!("ordinary Rust-target-shaped plan did not reopen");
    };
    assert_eq!(
        stored.candidate_status_coupling,
        CandidateStatusCoupling::PlanClaimsV1
    );

    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO candidate_blockers (
                     candidate_id, blocker_ordinal, blocker_kind
                 ) VALUES (?1, 0, 'protected_path')",
                [candidate.id().as_str()],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_cleanup_session(&session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "DELETE FROM candidate_blockers
                 WHERE candidate_id = ?1",
                [candidate.id().as_str()],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE cleanup_sessions
                 SET status = 'running',
                     execution_owner_id = 'process:ordinary-forged-seal',
                     execution_generation = 1,
                     last_heartbeat_at_unix_ms = started_at_unix_ms
                 WHERE session_id = ?1",
                [session_id.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO trusted_rust_target_plan_claims (
                     candidate_id, session_id, item_ordinal,
                     coupling_revision
                 ) VALUES (?1, ?2, 0, 1)",
                params![candidate.id().as_str(), session_id.as_str()],
            )
            .unwrap();
        assert_eq!(
            load_frozen_cleanup_session_within_budget(connection, &session_id, false)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
    });
}

#[test]
fn historical_revision_two_trusted_rust_target_seal_remains_decodable_and_fenced() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("root");
    let scan_id = "scan:cleanup-trusted-rust-target";
    let session_id = CleanupSessionId::new("session:cleanup-trusted-rust-target").unwrap();
    let candidate_id = CandidateId::new("candidate:cleanup-trusted-rust-target").unwrap();
    {
        let store = StoreCoordinator::open(&database).unwrap();
        start_scan(&store, &root, scan_id);
        let candidate = candidate(
            candidate_id.as_str(),
            scan_id,
            &rust_target_rule(),
            root.join("cleanup-fixture"),
            10,
        );
        persist_candidate(&store, &candidate, 1_750_000_002);
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO candidate_blockers (
                         candidate_id, blocker_ordinal, blocker_kind
                     ) VALUES (?1, 0, 'protected_path')",
                    [candidate.id().as_str()],
                )
                .unwrap();
        });
        store
            .record_cleanup_session_planned(
                &NewCleanupSessionRecord::try_from_trusted_rust_target_plan(
                    session_id.clone(),
                    &plan(
                        "plan:cleanup-trusted-rust-target",
                        CleanupMode::PermanentSafe,
                        std::slice::from_ref(&candidate),
                    ),
                    UNIX_EPOCH + Duration::from_secs(1_750_000_011),
                    CleanupTrigger::Manual,
                )
                .unwrap(),
            )
            .unwrap();
        store.with_connection(|connection| {
            let seal_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM trusted_rust_target_plan_claims
                     WHERE candidate_id = ?1 AND session_id = ?2
                       AND item_ordinal = 0 AND coupling_revision = 1",
                    params![candidate.id().as_str(), session_id.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(seal_count, 1);
            // Simulate an exact schema-v12 session minted by the previous
            // rule revision. Current constructors cannot create this row;
            // the decoder must retain it only so restart recovery can
            // terminalize the already-sealed session.
            connection
                .execute(
                    "UPDATE candidates SET rule_revision = 2
                     WHERE candidate_id = ?1",
                    [candidate.id().as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE cleanup_items SET rule_revision = 2
                     WHERE session_id = ?1 AND item_ordinal = 0",
                    [session_id.as_str()],
                )
                .unwrap();
        });
    }

    let store = StoreCoordinator::open(&database).unwrap();
    let Some(StoredCleanupSessionRecord::Planned(stored)) =
        store.load_cleanup_session(&session_id).unwrap()
    else {
        panic!("trusted Rust-target plan did not reopen");
    };
    assert_eq!(
        stored.candidate_status_coupling,
        CandidateStatusCoupling::TrustedRustTargetPlanClaimsV1
    );
    assert_eq!(stored.items[0].rule.revision().get(), 2);

    store.with_connection(|connection| {
        assert!(
            connection
                .execute(
                    "INSERT INTO trusted_rust_target_plan_claims (
                         candidate_id, session_id, item_ordinal,
                         coupling_revision
                     ) VALUES (
                         'candidate:missing-trusted-claim',
                         'session:missing-trusted-claim', 0, 1
                     )",
                    [],
                )
                .is_err()
        );
        connection
            .execute(
                "UPDATE trusted_rust_target_plan_claims
                 SET session_id = 'session:moved-trusted-claim'
                 WHERE candidate_id = ?1",
                [candidate_id.as_str()],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_cleanup_session(&session_id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );

    store.with_connection(|connection| {
        connection
            .execute(
                "DELETE FROM trusted_rust_target_plan_claims
                 WHERE candidate_id = ?1",
                [candidate_id.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO trusted_rust_target_plan_claims (
                     candidate_id, session_id, item_ordinal,
                     coupling_revision
                 ) VALUES (?1, ?2, 0, 1)",
                params![candidate_id.as_str(), session_id.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO candidate_blockers (
                     candidate_id, blocker_ordinal, blocker_kind
                 ) VALUES (?1, 1, 'partial_scan_coverage')",
                [candidate_id.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE cleanup_sessions
                 SET status = 'running',
                     execution_owner_id = 'process:trusted-reopen-fixture',
                     execution_generation = 1,
                     last_heartbeat_at_unix_ms = started_at_unix_ms
                 WHERE session_id = ?1",
                [session_id.as_str()],
            )
            .unwrap();
    });
    store.with_connection(|connection| {
        assert_eq!(
            load_frozen_cleanup_session_within_budget(connection, &session_id, false)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
        connection
            .execute(
                "DELETE FROM candidate_blockers
                 WHERE candidate_id = ?1 AND blocker_ordinal = 1",
                [candidate_id.as_str()],
            )
            .unwrap();
        let recovered = load_frozen_cleanup_session_within_budget(connection, &session_id, false)
            .unwrap()
            .expect("valid trusted active claim should reopen");
        assert_eq!(
            recovered.candidate_status_coupling,
            CandidateStatusCoupling::TrustedRustTargetPlanClaimsV1
        );
        connection
            .execute(
                "DELETE FROM trusted_rust_target_plan_claims
                 WHERE candidate_id = ?1",
                [candidate_id.as_str()],
            )
            .unwrap();
        assert_eq!(
            load_frozen_cleanup_session_within_budget(connection, &session_id, false)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
    });
}

#[test]
fn missing_candidate_and_duplicate_ids_roll_back_atomically() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("root");
    let store = StoreCoordinator::open(&database).unwrap();
    start_scan(&store, &root, "scan:cleanup-atomic");
    let rule = rule(
        "fixture.cleanup.atomic",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );
    let base_candidate = candidate(
        "candidate:cleanup-atomic",
        "scan:cleanup-atomic",
        &rule,
        root.join("cleanup-fixture"),
        10,
    );
    let cleanup_plan = plan(
        "plan:cleanup-atomic",
        CleanupMode::PermanentSafe,
        std::slice::from_ref(&base_candidate),
    );
    let record = NewCleanupSessionRecord::try_from_plan(
        CleanupSessionId::new("session:cleanup-atomic").unwrap(),
        &cleanup_plan,
        UNIX_EPOCH + Duration::from_secs(1_750_000_011),
        CleanupTrigger::Manual,
    )
    .unwrap();
    assert_eq!(
        store
            .record_cleanup_session_planned(&record)
            .unwrap_err()
            .kind,
        HistoryErrorKind::NotFound
    );
    store.with_connection(|connection| {
        let count: i64 = connection
            .query_row("SELECT count(*) FROM cleanup_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    });
    persist_candidate(&store, &base_candidate, 1_750_000_002);
    store.record_cleanup_session_planned(&record).unwrap();
    assert_eq!(
        store
            .record_cleanup_session_planned(&record)
            .unwrap_err()
            .kind,
        HistoryErrorKind::AlreadyExists
    );
    let same_plan = NewCleanupSessionRecord::try_from_plan(
        CleanupSessionId::new("session:cleanup-other").unwrap(),
        &cleanup_plan,
        UNIX_EPOCH + Duration::from_secs(1_750_000_012),
        CleanupTrigger::LowDisk,
    )
    .unwrap();
    assert_eq!(
        store
            .record_cleanup_session_planned(&same_plan)
            .unwrap_err()
            .kind,
        HistoryErrorKind::AlreadyExists
    );
    store.with_connection(|connection| {
        for table in [
            "cleanup_sessions",
            "cleanup_items",
            "cleanup_item_paths",
            "cleanup_item_evidence",
            "candidate_plan_claims",
        ] {
            let count: i64 = connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 1, "unexpected rows in {table}");
        }
    });
}

#[test]
fn incompatible_later_candidate_rolls_back_the_entire_plan_claim() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    let scan_id = "scan:cleanup-claim-rollback";
    start_scan(&store, &root, scan_id);
    let policy = rule(
        "fixture.cleanup.claim-rollback",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );
    let candidates = [
        candidate(
            "candidate:cleanup-claim-rollback-0",
            scan_id,
            &policy,
            root.join("cleanup-fixture-0"),
            10,
        ),
        candidate(
            "candidate:cleanup-claim-rollback-1",
            scan_id,
            &policy,
            root.join("cleanup-fixture-1"),
            20,
        ),
    ];
    persist_candidate(&store, &candidates[0], 1_750_000_002);
    persist_candidate(&store, &candidates[1], 1_750_000_003);
    store
        .transition_candidate_review_status(
            candidates[1].id(),
            CandidateReviewTransition::DismissDiscovered,
        )
        .unwrap();
    let record = NewCleanupSessionRecord::try_from_plan(
        CleanupSessionId::new("session:cleanup-claim-rollback").unwrap(),
        &plan(
            "plan:cleanup-claim-rollback",
            CleanupMode::PermanentSafe,
            &candidates,
        ),
        UNIX_EPOCH + Duration::from_secs(1_750_000_011),
        CleanupTrigger::Manual,
    )
    .unwrap();
    assert_eq!(
        store
            .record_cleanup_session_planned(&record)
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidInput
    );
    assert_eq!(
        candidate_status(&store, candidates[0].id()),
        CandidateHistoryStatus::Discovered
    );
    assert_eq!(
        candidate_status(&store, candidates[1].id()),
        CandidateHistoryStatus::Dismissed
    );
    store.with_connection(|connection| {
        let sessions: i64 = connection
            .query_row("SELECT COUNT(*) FROM cleanup_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        let claims: i64 = connection
            .query_row("SELECT COUNT(*) FROM candidate_plan_claims", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!((sessions, claims), (0, 0));
    });
}

#[test]
fn competing_plans_create_exactly_one_candidate_claim() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    let scan_id = "scan:cleanup-claim-race";
    start_scan(&store, &root, scan_id);
    let policy = rule(
        "fixture.cleanup.claim-race",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );
    let candidate = candidate(
        "candidate:cleanup-claim-race",
        scan_id,
        &policy,
        root.join("cleanup-fixture"),
        10,
    );
    persist_candidate(&store, &candidate, 1_750_000_002);
    let records = [
        NewCleanupSessionRecord::try_from_plan(
            CleanupSessionId::new("session:cleanup-claim-race-a").unwrap(),
            &plan(
                "plan:cleanup-claim-race-a",
                CleanupMode::PermanentSafe,
                std::slice::from_ref(&candidate),
            ),
            UNIX_EPOCH + Duration::from_secs(1_750_000_011),
            CleanupTrigger::Manual,
        )
        .unwrap(),
        NewCleanupSessionRecord::try_from_plan(
            CleanupSessionId::new("session:cleanup-claim-race-b").unwrap(),
            &plan(
                "plan:cleanup-claim-race-b",
                CleanupMode::PermanentSafe,
                std::slice::from_ref(&candidate),
            ),
            UNIX_EPOCH + Duration::from_secs(1_750_000_012),
            CleanupTrigger::LowDisk,
        )
        .unwrap(),
    ];
    let barrier = Arc::new(Barrier::new(3));
    let workers = records
        .into_iter()
        .map(|record| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                store.record_cleanup_session_planned(&record)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .map(|error| error.kind)
            .collect::<Vec<_>>(),
        vec![HistoryErrorKind::InvalidInput]
    );
    assert_eq!(
        candidate_status(&store, candidate.id()),
        CandidateHistoryStatus::Planned
    );
    store.with_connection(|connection| {
        let sessions: i64 = connection
            .query_row("SELECT COUNT(*) FROM cleanup_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        let claims: i64 = connection
            .query_row("SELECT COUNT(*) FROM candidate_plan_claims", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!((sessions, claims), (1, 1));
    });
}

#[test]
fn legacy_summary_is_explicit_and_rejects_v2_child_pollution() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let store = StoreCoordinator::open(&database).unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO cleanup_sessions (
                 session_id, plan_id, started_at_unix_ms, mode, estimated_bytes,
                 trigger_source, status, record_format_version
             ) VALUES ('session:legacy-cleanup', 'plan:legacy-cleanup', 1000,
                       'trash', 42, 'cli', 'completed', 1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cleanup_items (
                 session_id, item_ordinal, rule_id, rule_revision, estimated_bytes,
                 final_status, record_format_version,
                 legacy_target_path, legacy_target_path_encoding
             ) VALUES ('session:legacy-cleanup', 0, 'fixture.cleanup.legacy', 2, 42,
                       'trashed', 1, ?1, 1)",
                [b"relative-legacy-cleanup".as_slice()],
            )
            .unwrap();
    });
    let id = CleanupSessionId::new("session:legacy-cleanup").unwrap();
    let StoredCleanupSessionRecord::LegacySummary(summary) =
        store.load_cleanup_session(&id).unwrap().unwrap()
    else {
        panic!("legacy cleanup was reconstructed as complete");
    };
    assert_eq!(summary.items.len(), 1);
    assert_eq!(
        summary.items[0].target_path,
        PathBuf::from("relative-legacy-cleanup")
    );
    assert_eq!(summary.items[0].status, LegacyItemStatus::Trashed);
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO cleanup_plan_warnings (
                 session_id, warning_ordinal, warning_kind
             ) VALUES ('session:legacy-cleanup', 0, 'estimated_bytes_unverified')",
                [],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_cleanup_session(&id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "DELETE FROM cleanup_plan_warnings
                 WHERE session_id = 'session:legacy-cleanup'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE cleanup_sessions SET status = 'recovering'
                 WHERE session_id = 'session:legacy-cleanup'",
                [],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_cleanup_session(&id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE cleanup_sessions SET status = 'completed'
                 WHERE session_id = 'session:legacy-cleanup'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE cleanup_items SET final_status = 'validating'
                 WHERE session_id = 'session:legacy-cleanup'",
                [],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_cleanup_session(&id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn malformed_children_and_newer_schema_fail_closed() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("root");
    let store = StoreCoordinator::open(&database).unwrap();
    start_scan(&store, &root, "scan:cleanup-malformed");
    let rule = rule(
        "fixture.cleanup.malformed",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );
    let candidate = candidate(
        "candidate:cleanup-malformed",
        "scan:cleanup-malformed",
        &rule,
        root.join("cleanup-fixture"),
        10,
    );
    persist_candidate(&store, &candidate, 1_750_000_002);
    let cleanup_plan = plan(
        "plan:cleanup-malformed",
        CleanupMode::PermanentSafe,
        &[candidate],
    );
    let id = CleanupSessionId::new("session:cleanup-malformed").unwrap();
    store
        .record_cleanup_session_planned(
            &NewCleanupSessionRecord::try_from_plan(
                id.clone(),
                &cleanup_plan,
                UNIX_EPOCH + Duration::from_secs(1_750_000_011),
                CleanupTrigger::Cli,
            )
            .unwrap(),
        )
        .unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE cleanup_sessions SET trigger_source = 'scheduled'
                 WHERE session_id = 'session:cleanup-malformed'",
                [],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_cleanup_session(&id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE cleanup_sessions SET trigger_source = 'cli'
                 WHERE session_id = 'session:cleanup-malformed'",
                [],
            )
            .unwrap();
        connection
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        connection
            .execute(
                "INSERT INTO cleanup_item_paths (
                     session_id, item_ordinal, path_ordinal, target_path,
                     target_path_encoding, status
                 ) VALUES ('session:cleanup-malformed', 99, 0, ?1, 1, 'planned')",
                [b"/orphan-cleanup-child".as_slice()],
            )
            .unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
    });
    assert_eq!(
        store.load_cleanup_session(&id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "DELETE FROM cleanup_item_paths
                 WHERE session_id = 'session:cleanup-malformed' AND item_ordinal = 99",
                [],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE cleanup_item_paths SET target_path = zeroblob(65537)
             WHERE session_id = 'session:cleanup-malformed'",
                [],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });
    assert_eq!(
        store.load_cleanup_session(&id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO schema_migrations (
                 version, name, checksum_sha256, applied_at_unix_ms
             ) VALUES (?1, 'future-cleanup-schema', zeroblob(32), 2)",
                [i64::from(crate::DATABASE_SCHEMA_VERSION + 1)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", crate::DATABASE_SCHEMA_VERSION + 1)
            .unwrap();
    });
    assert_eq!(
        store.load_cleanup_session(&id).unwrap_err().kind,
        HistoryErrorKind::IncompatibleSchema
    );
}

#[test]
fn maximum_legal_path_and_evidence_counts_fit_the_shared_query_budget() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("root");
    let store = StoreCoordinator::open(&database).unwrap();
    start_scan(&store, &root, "scan:cleanup-maximum");
    let rule = rule(
        "fixture.cleanup.maximum",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );
    let paths = (0..MAX_TOTAL_PATHS)
        .map(|index| root.join(format!("cleanup-fixture-{index}")))
        .collect::<Vec<_>>();
    let evidence = (0..MAX_TOTAL_EVIDENCE)
        .map(|index| Evidence::MinimumSize {
            observed_bytes: index as u64 + 1,
            minimum_bytes: index as u64,
        })
        .collect::<Vec<_>>();
    let candidate = Candidate::try_from_rule(
        &rule,
        CandidateInput::new(
            CandidateId::new("candidate:cleanup-maximum").unwrap(),
            paths,
            1,
            None,
            evidence,
            Vec::new(),
            ScanId::new("scan:cleanup-maximum").unwrap(),
        ),
    )
    .unwrap();
    persist_candidate(&store, &candidate, 1_750_000_002);
    let cleanup_plan = plan(
        "plan:cleanup-maximum",
        CleanupMode::PermanentSafe,
        &[candidate],
    );
    let id = CleanupSessionId::new("session:cleanup-maximum").unwrap();
    store
        .record_cleanup_session_planned(
            &NewCleanupSessionRecord::try_from_plan(
                id.clone(),
                &cleanup_plan,
                UNIX_EPOCH + Duration::from_secs(1_750_000_011),
                CleanupTrigger::Manual,
            )
            .unwrap(),
        )
        .unwrap();
    let StoredCleanupSessionRecord::Planned(stored) =
        store.load_cleanup_session(&id).unwrap().unwrap()
    else {
        panic!("maximum record became incomplete");
    };
    assert_eq!(stored.items[0].paths.len(), MAX_TOTAL_PATHS);
    assert_eq!(stored.items[0].evidence.len(), MAX_TOTAL_EVIDENCE);
}

#[test]
fn session_ids_and_plan_start_times_are_bounded_before_storage_locking() {
    for invalid_id in ["", "session with spaces"] {
        assert_eq!(
            CleanupSessionId::new(invalid_id).unwrap_err().kind,
            HistoryErrorKind::InvalidInput
        );
    }
    assert_eq!(
        CleanupSessionId::new("x".repeat(129)).unwrap_err().kind,
        HistoryErrorKind::InvalidInput
    );
    let rule = rule(
        "fixture.cleanup.input",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );
    let base_candidate = candidate(
        "candidate:cleanup-input",
        "scan:cleanup-input",
        &rule,
        PathBuf::from("/cleanup-fixture"),
        1,
    );
    let cleanup_plan = plan(
        "plan:cleanup-input",
        CleanupMode::PermanentSafe,
        &[base_candidate],
    );
    for invalid_start in [
        cleanup_plan
            .created_at()
            .checked_sub(Duration::from_millis(1))
            .unwrap(),
        cleanup_plan.expires_at(),
    ] {
        assert_eq!(
            NewCleanupSessionRecord::try_from_plan(
                CleanupSessionId::new("session:cleanup-input").unwrap(),
                &cleanup_plan,
                invalid_start,
                CleanupTrigger::Manual,
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );
    }

    assert_eq!(
        NewCleanupSessionRecord::try_from_plan(
            CleanupSessionId::new("session:cleanup-unscheduled").unwrap(),
            &cleanup_plan,
            cleanup_plan.created_at(),
            CleanupTrigger::Scheduled,
        )
        .unwrap_err()
        .kind,
        HistoryErrorKind::InvalidInput
    );

    let scheduled_rule = rule_with_schedule(
        "fixture.cleanup.scheduled-input",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        true,
    );
    let scheduled_candidate = candidate(
        "candidate:cleanup-scheduled-input",
        "scan:cleanup-input",
        &scheduled_rule,
        PathBuf::from("/cleanup-fixture-scheduled"),
        1,
    );
    let scheduled_plan = plan(
        "plan:cleanup-scheduled-input",
        CleanupMode::PermanentSafe,
        std::slice::from_ref(&scheduled_candidate),
    );
    NewCleanupSessionRecord::try_from_plan(
        CleanupSessionId::new("session:cleanup-scheduled-input").unwrap(),
        &scheduled_plan,
        scheduled_plan.created_at(),
        CleanupTrigger::Scheduled,
    )
    .unwrap();

    let submillisecond_created = UNIX_EPOCH + Duration::new(1_750_000_100, 500_000);
    let precise_plan = CleanupPlan::try_from_candidates_for_persistence_test(
        CleanupPlanId::new("plan:cleanup-precise-time").unwrap(),
        submillisecond_created,
        CleanupMode::PermanentSafe,
        &[scheduled_candidate],
    )
    .unwrap();
    let precise = NewCleanupSessionRecord::try_from_plan(
        CleanupSessionId::new("session:cleanup-precise-time").unwrap(),
        &precise_plan,
        precise_plan.created_at(),
        CleanupTrigger::Manual,
    )
    .unwrap();
    assert_eq!(
        precise.started_at,
        UNIX_EPOCH + Duration::new(1_750_000_100, 1_000_000)
    );
    assert_eq!(
        NewCleanupSessionRecord::try_from_plan(
            CleanupSessionId::new("session:cleanup-precise-expiry").unwrap(),
            &precise_plan,
            precise_plan.expires_at(),
            CleanupTrigger::Manual,
        )
        .unwrap_err()
        .kind,
        HistoryErrorKind::InvalidInput
    );
    assert_eq!(
        NewCleanupSessionRecord::try_from_plan(
            CleanupSessionId::new("session:cleanup-unrepresentable-expiry").unwrap(),
            &precise_plan,
            precise_plan
                .expires_at()
                .checked_sub(Duration::from_nanos(1))
                .unwrap(),
            CleanupTrigger::Manual,
        )
        .unwrap_err()
        .kind,
        HistoryErrorKind::InvalidInput
    );
}
