use super::*;
use crate::domain::{
    CandidateInput, LocalizedTextKey, ProvenanceUrl, Rule, RuleDefinition, RuleGuards, RuleMatcher,
    RuleMatcherDefinition, RuleScope,
};
use crate::persistence::StoreCoordinator;
use crate::persistence::history::{
    NewScanRecord, ScanCompletionRecord, ScanCounts, TerminalScanStatus,
};
use tempfile::TempDir;

const ALL_BLOCKERS: [BlockReason; 14] = [
    BlockReason::MissingOrIncompleteEvidence,
    BlockReason::MissingModificationTime,
    BlockReason::PartialScanCoverage,
    BlockReason::RecentActivity,
    BlockReason::BelowMinimumBytes,
    BlockReason::ActiveUse,
    BlockReason::AccessDenied,
    BlockReason::ProtectedPath,
    BlockReason::ProtectedDescendant,
    BlockReason::SymlinkBoundary,
    BlockReason::VolumeBoundary,
    BlockReason::ChangedSinceScan,
    BlockReason::UnsupportedPlatform,
    BlockReason::CloudUploadUnconfirmed,
];

fn matcher() -> RuleMatcher {
    RuleMatcher::try_new(RuleMatcherDefinition {
        path_component: Some("candidate-fixture".to_owned()),
        required_ancestor_markers_any: Vec::new(),
        required_markers_all: Vec::new(),
        forbidden_markers_any: Vec::new(),
        exact_bundle_identifiers: Vec::new(),
        excluded_descendants: Vec::new(),
        protected_descendants: Vec::new(),
    })
    .unwrap()
}

fn rule(
    id: &str,
    category: CandidateCategory,
    safety: SafetyTier,
    action: CandidateAction,
    schedule_eligible: bool,
) -> Rule {
    Rule::try_new(RuleDefinition {
        reference: RuleRef::new(RuleId::new(id).unwrap(), RuleRevision::new(7).unwrap()),
        title_key: LocalizedTextKey::new("fixture.candidate.title").unwrap(),
        category,
        scope: RuleScope::ConfiguredProjectRoots,
        matcher: matcher(),
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
        explanation_key: LocalizedTextKey::new("fixture.candidate.explanation").unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/candidate").unwrap()],
    })
    .unwrap()
}

#[expect(
    clippy::too_many_arguments,
    reason = "the persistence fixture keeps every frozen candidate fact explicit at call sites"
)]
fn candidate(
    id: &str,
    scan_id: &str,
    rule: &Rule,
    paths: Vec<PathBuf>,
    evidence: Vec<Evidence>,
    blockers: Vec<BlockReason>,
    newest_mtime: Option<SystemTime>,
    estimated_bytes: u64,
) -> Candidate {
    Candidate::try_from_rule(
        rule,
        CandidateInput::new(
            CandidateId::new(id).unwrap(),
            paths,
            estimated_bytes,
            newest_mtime,
            evidence,
            blockers,
            ScanId::new(scan_id).unwrap(),
        ),
    )
    .unwrap()
}

fn start_and_finish_scan(store: &StoreCoordinator, root: &Path, id: &str) {
    let scan_id = ScanId::new(id).unwrap();
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                scan_id.clone(),
                root.to_path_buf(),
                UNIX_EPOCH + Duration::from_secs(1_750_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    store
        .record_scan_finished(
            &ScanCompletionRecord::try_new(
                scan_id,
                UNIX_EPOCH + Duration::from_secs(1_750_000_001),
                TerminalScanStatus::Succeeded,
                ScanCounts::default(),
            )
            .unwrap(),
        )
        .unwrap();
}

fn load_complete(store: &StoreCoordinator, id: &CandidateId) -> CompleteCandidateRecord {
    let StoredCandidateRecord::Complete(record) =
        store.load_candidate(id).unwrap().expect("candidate exists")
    else {
        panic!("format-2 candidate became a legacy summary");
    };
    record
}

fn persist_review_candidate(
    store: &StoreCoordinator,
    root: &Path,
    id: &str,
    scan_id: &str,
    policy: &Rule,
    blockers: Vec<BlockReason>,
) -> CandidateId {
    let path = root.join(format!("candidate-fixture-{id}"));
    let candidate = candidate(
        id,
        scan_id,
        policy,
        vec![path.clone()],
        vec![Evidence::MatchedPath { path }],
        blockers,
        None,
        4_096,
    );
    store
        .record_candidate_discovered(
            &NewCandidateRecord::try_from_candidate(
                &candidate,
                UNIX_EPOCH + Duration::from_secs(1_750_000_002),
            )
            .unwrap(),
        )
        .unwrap();
    candidate.id().clone()
}

#[test]
fn review_status_is_typed_idempotent_and_preserves_frozen_facts_after_reopen() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("root");
    let store = StoreCoordinator::open(&database).unwrap();
    start_and_finish_scan(&store, &root, "scan:review-status");
    let policy = rule(
        "fixture.candidate.review-status",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let id = persist_review_candidate(
        &store,
        &root,
        "candidate:review-status",
        "scan:review-status",
        &policy,
        Vec::new(),
    );
    let discovered = load_complete(&store, &id);

    assert_eq!(
        store
            .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
            .unwrap(),
        CandidateHistoryStatus::Selected
    );
    assert_eq!(
        store
            .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
            .unwrap(),
        CandidateHistoryStatus::Selected
    );
    let mut expected = discovered.clone();
    expected.status = CandidateHistoryStatus::Selected;
    assert_eq!(load_complete(&store, &id), expected);

    assert_eq!(
        store
            .transition_candidate_review_status(&id, CandidateReviewTransition::ClearSelection,)
            .unwrap(),
        CandidateHistoryStatus::Discovered
    );
    assert_eq!(load_complete(&store, &id), discovered);
    assert_eq!(
        store
            .transition_candidate_review_status(&id, CandidateReviewTransition::DismissDiscovered,)
            .unwrap(),
        CandidateHistoryStatus::Dismissed
    );
    assert_eq!(
        store
            .transition_candidate_review_status(&id, CandidateReviewTransition::DismissDiscovered,)
            .unwrap(),
        CandidateHistoryStatus::Dismissed
    );
    assert_eq!(
        store
            .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        store
            .transition_candidate_review_status(&id, CandidateReviewTransition::Restore)
            .unwrap(),
        CandidateHistoryStatus::Discovered
    );
    assert_eq!(
        store
            .transition_candidate_review_status(&id, CandidateReviewTransition::Restore)
            .unwrap(),
        CandidateHistoryStatus::Discovered
    );
    store
        .transition_candidate_review_status(&id, CandidateReviewTransition::DismissDiscovered)
        .unwrap();

    let selected_dismissal = persist_review_candidate(
        &store,
        &root,
        "candidate:review-selected-dismissal",
        "scan:review-status",
        &policy,
        Vec::new(),
    );
    store
        .transition_candidate_review_status(&selected_dismissal, CandidateReviewTransition::Select)
        .unwrap();
    assert_eq!(
        store
            .transition_candidate_review_status(
                &selected_dismissal,
                CandidateReviewTransition::DismissSelected,
            )
            .unwrap(),
        CandidateHistoryStatus::Dismissed
    );

    drop(store);
    let reopened = StoreCoordinator::open(&database).unwrap();
    expected.status = CandidateHistoryStatus::Dismissed;
    assert_eq!(load_complete(&reopened, &id), expected);
}

#[test]
fn evaluator_status_is_source_typed_terminal_and_unavailable_refines_to_stale() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("root");
    let store = StoreCoordinator::open(&database).unwrap();
    let scan_id = "scan:evaluator-status";
    start_and_finish_scan(&store, &root, scan_id);
    let policy = rule(
        "fixture.candidate.evaluator-status",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let cases = [
        (
            "discovered-stale",
            None,
            CandidateEvaluationTransition::DiscoveredToStale,
            CandidateHistoryStatus::Stale,
        ),
        (
            "selected-stale",
            Some(CandidateReviewTransition::Select),
            CandidateEvaluationTransition::SelectedToStale,
            CandidateHistoryStatus::Stale,
        ),
        (
            "dismissed-stale",
            Some(CandidateReviewTransition::DismissDiscovered),
            CandidateEvaluationTransition::DismissedToStale,
            CandidateHistoryStatus::Stale,
        ),
        (
            "discovered-unavailable",
            None,
            CandidateEvaluationTransition::DiscoveredToUnavailable,
            CandidateHistoryStatus::Unavailable,
        ),
        (
            "selected-unavailable",
            Some(CandidateReviewTransition::Select),
            CandidateEvaluationTransition::SelectedToUnavailable,
            CandidateHistoryStatus::Unavailable,
        ),
        (
            "dismissed-unavailable",
            Some(CandidateReviewTransition::DismissDiscovered),
            CandidateEvaluationTransition::DismissedToUnavailable,
            CandidateHistoryStatus::Unavailable,
        ),
    ];

    for (label, review, transition, target) in cases {
        let id = persist_review_candidate(
            &store,
            &root,
            &format!("candidate:evaluator-{label}"),
            scan_id,
            &policy,
            Vec::new(),
        );
        if let Some(review) = review {
            store
                .transition_candidate_review_status(&id, review)
                .unwrap();
        }
        let before = load_complete(&store, &id);
        assert_eq!(
            store
                .transition_candidate_evaluation_status(&id, transition)
                .unwrap(),
            target
        );
        assert_eq!(
            store
                .transition_candidate_evaluation_status(&id, transition)
                .unwrap(),
            target
        );
        let mut expected = before;
        expected.status = target;
        assert_eq!(load_complete(&store, &id), expected);
        assert_eq!(
            store
                .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
    }

    let refined = persist_review_candidate(
        &store,
        &root,
        "candidate:evaluator-refined",
        scan_id,
        &policy,
        Vec::new(),
    );
    store
        .transition_candidate_evaluation_status(
            &refined,
            CandidateEvaluationTransition::DiscoveredToUnavailable,
        )
        .unwrap();
    assert_eq!(
        store
            .transition_candidate_evaluation_status(
                &refined,
                CandidateEvaluationTransition::UnavailableToStale,
            )
            .unwrap(),
        CandidateHistoryStatus::Stale
    );
    assert_eq!(
        store
            .transition_candidate_evaluation_status(
                &refined,
                CandidateEvaluationTransition::DiscoveredToUnavailable,
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );

    let wrong_source = persist_review_candidate(
        &store,
        &root,
        "candidate:evaluator-wrong-source",
        scan_id,
        &policy,
        Vec::new(),
    );
    assert_eq!(
        store
            .transition_candidate_evaluation_status(
                &wrong_source,
                CandidateEvaluationTransition::SelectedToStale,
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        load_complete(&store, &wrong_source).status,
        CandidateHistoryStatus::Discovered
    );

    let reconciled = persist_review_candidate(
        &store,
        &root,
        "candidate:evaluator-reconciled",
        scan_id,
        &policy,
        Vec::new(),
    );
    assert_eq!(
        store
            .transition_candidate_evaluation_status_after_commit_failure_for_test(
                &reconciled,
                CandidateEvaluationTransition::DiscoveredToStale,
            )
            .unwrap(),
        CandidateHistoryStatus::Stale
    );

    drop(store);
    let reopened = StoreCoordinator::open(&database).unwrap();
    assert_eq!(
        load_complete(&reopened, &reconciled).status,
        CandidateHistoryStatus::Stale
    );
}

#[test]
fn review_selection_rejects_blocked_and_non_cleanup_candidates_without_erasing_facts() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    start_and_finish_scan(&store, &root, "scan:review-policy");
    let cleanup = rule(
        "fixture.candidate.review-cleanup",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let informational = rule(
        "fixture.candidate.review-info",
        CandidateCategory::UnknownStorage,
        SafetyTier::Informational,
        CandidateAction::RevealOnly,
        false,
    );
    let blocked = persist_review_candidate(
        &store,
        &root,
        "candidate:review-blocked",
        "scan:review-policy",
        &cleanup,
        vec![BlockReason::PartialScanCoverage],
    );
    let info = persist_review_candidate(
        &store,
        &root,
        "candidate:review-info",
        "scan:review-policy",
        &informational,
        Vec::new(),
    );

    for id in [&blocked, &info] {
        assert_eq!(
            store
                .transition_candidate_review_status(id, CandidateReviewTransition::Select)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        assert_eq!(
            load_complete(&store, id).status,
            CandidateHistoryStatus::Discovered
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE candidates SET status = 'selected' WHERE candidate_id = ?1",
                    [id.as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            store
                .transition_candidate_review_status(id, CandidateReviewTransition::Select)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        assert_eq!(
            store
                .transition_candidate_review_status(id, CandidateReviewTransition::DismissSelected,)
                .unwrap(),
            CandidateHistoryStatus::Dismissed
        );
        assert_eq!(
            store
                .transition_candidate_evaluation_status(
                    id,
                    CandidateEvaluationTransition::DismissedToStale,
                )
                .unwrap(),
            CandidateHistoryStatus::Stale
        );
    }
    assert_eq!(
        load_complete(&store, &blocked).blockers,
        vec![BlockReason::PartialScanCoverage]
    );
}

#[test]
fn review_selection_policy_matrix_is_exhaustive_and_all_blockers_fail_closed() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    let scan_id = "scan:review-policy-matrix";
    start_and_finish_scan(&store, &root, scan_id);

    let cleanup_policies = [
        (
            "regenerable",
            rule(
                "fixture.candidate.matrix.regenerable",
                CandidateCategory::ApplicationCache,
                SafetyTier::SafeRegenerable,
                CandidateAction::RemoveKnownRegenerableContents,
                false,
            ),
        ),
        (
            "evictable",
            rule(
                "fixture.candidate.matrix.evictable",
                CandidateCategory::CloudFile,
                SafetyTier::SafeEvictable,
                CandidateAction::EvictLocalCopy,
                false,
            ),
        ),
        (
            "trash",
            rule(
                "fixture.candidate.matrix.trash",
                CandidateCategory::LargeReviewItem,
                SafetyTier::ReviewRequired,
                CandidateAction::MoveToTrash,
                false,
            ),
        ),
    ];
    for (label, policy) in cleanup_policies {
        let path = root.join(format!("candidate-fixture-matrix-{label}"));
        let evidence = if policy.action() == CandidateAction::EvictLocalCopy {
            vec![Evidence::CloudUploadComplete { path: path.clone() }]
        } else {
            vec![Evidence::MatchedPath { path: path.clone() }]
        };
        let candidate = candidate(
            &format!("candidate:matrix-{label}"),
            scan_id,
            &policy,
            vec![path],
            evidence,
            Vec::new(),
            None,
            1,
        );
        store
            .record_candidate_discovered(
                &NewCandidateRecord::try_from_candidate(
                    &candidate,
                    UNIX_EPOCH + Duration::from_secs(1_750_000_002),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            store
                .transition_candidate_review_status(
                    candidate.id(),
                    CandidateReviewTransition::Select,
                )
                .unwrap(),
            CandidateHistoryStatus::Selected
        );
    }

    for (label, safety, action) in [
        (
            "informational",
            SafetyTier::Informational,
            CandidateAction::RevealOnly,
        ),
        (
            "protected",
            SafetyTier::Protected,
            CandidateAction::NoAction,
        ),
    ] {
        let policy = rule(
            &format!("fixture.candidate.matrix.{label}"),
            CandidateCategory::UnknownStorage,
            safety,
            action,
            false,
        );
        let id = persist_review_candidate(
            &store,
            &root,
            &format!("candidate:matrix-{label}"),
            scan_id,
            &policy,
            Vec::new(),
        );
        assert_eq!(
            store
                .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
    }

    let cleanup = rule(
        "fixture.candidate.matrix.blocked",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    for (index, blocker) in ALL_BLOCKERS.iter().enumerate() {
        let id = persist_review_candidate(
            &store,
            &root,
            &format!("candidate:matrix-blocker-{index}"),
            scan_id,
            &cleanup,
            vec![blocker.clone()],
        );
        assert_eq!(
            store
                .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition,
            "selection accepted blocker {blocker:?}"
        );
    }
}

#[test]
fn review_status_reconciles_post_commit_failure_and_exact_cas_races() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    start_and_finish_scan(&store, &root, "scan:review-race");
    let policy = rule(
        "fixture.candidate.review-race",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let reconciled = persist_review_candidate(
        &store,
        &root,
        "candidate:review-reconciled",
        "scan:review-race",
        &policy,
        Vec::new(),
    );
    assert_eq!(
        store
            .transition_candidate_review_status_after_commit_failure_for_test(
                &reconciled,
                CandidateReviewTransition::Select,
            )
            .unwrap(),
        CandidateHistoryStatus::Selected
    );

    let raced = persist_review_candidate(
        &store,
        &root,
        "candidate:review-raced",
        "scan:review-race",
        &policy,
        Vec::new(),
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let first_store = std::sync::Arc::clone(&store);
    let first_id = raced.clone();
    let first_barrier = std::sync::Arc::clone(&barrier);
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        first_store.transition_candidate_review_status(&first_id, CandidateReviewTransition::Select)
    });
    let second_store = std::sync::Arc::clone(&store);
    let second_id = raced.clone();
    let second_barrier = std::sync::Arc::clone(&barrier);
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        second_store.transition_candidate_review_status(
            &second_id,
            CandidateReviewTransition::DismissDiscovered,
        )
    });
    barrier.wait();
    let results = [first.join().unwrap(), second.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                result
                    .as_ref()
                    .is_err_and(|error| error.kind == HistoryErrorKind::InvalidTransition)
            })
            .count(),
        1
    );
    assert!(matches!(
        load_complete(&store, &raced).status,
        CandidateHistoryStatus::Selected | CandidateHistoryStatus::Dismissed
    ));

    let evaluator_race = persist_review_candidate(
        &store,
        &root,
        "candidate:review-evaluator-race",
        "scan:review-race",
        &policy,
        Vec::new(),
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let review_store = std::sync::Arc::clone(&store);
    let review_id = evaluator_race.clone();
    let review_barrier = std::sync::Arc::clone(&barrier);
    let review = std::thread::spawn(move || {
        review_barrier.wait();
        review_store
            .transition_candidate_review_status(&review_id, CandidateReviewTransition::Select)
    });
    let evaluator_store = std::sync::Arc::clone(&store);
    let evaluator_id = evaluator_race.clone();
    let evaluator_barrier = std::sync::Arc::clone(&barrier);
    let evaluator = std::thread::spawn(move || {
        evaluator_barrier.wait();
        evaluator_store.transition_candidate_evaluation_status(
            &evaluator_id,
            CandidateEvaluationTransition::DiscoveredToStale,
        )
    });
    barrier.wait();
    let results = [review.join().unwrap(), evaluator.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                result
                    .as_ref()
                    .is_err_and(|error| error.kind == HistoryErrorKind::InvalidTransition)
            })
            .count(),
        1
    );
    assert!(matches!(
        load_complete(&store, &evaluator_race).status,
        CandidateHistoryStatus::Selected | CandidateHistoryStatus::Stale
    ));

    let evaluator_refinement = persist_review_candidate(
        &store,
        &root,
        "candidate:evaluator-refinement-race",
        "scan:review-race",
        &policy,
        Vec::new(),
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let unavailable_store = std::sync::Arc::clone(&store);
    let unavailable_id = evaluator_refinement.clone();
    let unavailable_barrier = std::sync::Arc::clone(&barrier);
    let unavailable = std::thread::spawn(move || {
        unavailable_barrier.wait();
        unavailable_store.transition_candidate_evaluation_status(
            &unavailable_id,
            CandidateEvaluationTransition::DiscoveredToUnavailable,
        )
    });
    let stale_store = std::sync::Arc::clone(&store);
    let stale_id = evaluator_refinement.clone();
    let stale_barrier = std::sync::Arc::clone(&barrier);
    let stale = std::thread::spawn(move || {
        stale_barrier.wait();
        stale_store.transition_candidate_evaluation_status(
            &stale_id,
            CandidateEvaluationTransition::DiscoveredToStale,
        )
    });
    barrier.wait();
    let results = [unavailable.join().unwrap(), stale.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                result
                    .as_ref()
                    .is_err_and(|error| error.kind == HistoryErrorKind::InvalidTransition)
            })
            .count(),
        1
    );
    if load_complete(&store, &evaluator_refinement).status == CandidateHistoryStatus::Unavailable {
        store
            .transition_candidate_evaluation_status(
                &evaluator_refinement,
                CandidateEvaluationTransition::UnavailableToStale,
            )
            .unwrap();
    }
    assert_eq!(
        load_complete(&store, &evaluator_refinement).status,
        CandidateHistoryStatus::Stale
    );
}

#[cfg(unix)]
#[test]
fn review_status_exact_match_cannot_mask_unsafe_post_commit_storage() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let marker = database.with_extension("sqlite3.writer.lock");
    let store = StoreCoordinator::open(&database).unwrap();
    let root = temp.path().join("root");
    start_and_finish_scan(&store, &root, "scan:review-unsafe");
    let policy = rule(
        "fixture.candidate.review-unsafe",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let id = persist_review_candidate(
        &store,
        &root,
        "candidate:review-unsafe",
        "scan:review-unsafe",
        &policy,
        Vec::new(),
    );

    let error = store
        .transition_candidate_review_status_with_after_commit_hook_for_test(
            &id,
            CandidateReviewTransition::Select,
            || {
                std::fs::write(&marker, b"NOT-A-DUX-MARKER")
                    .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))
            },
        )
        .unwrap_err();
    assert_eq!(error.kind, HistoryErrorKind::OutcomeUnknown);
    store.with_connection(|connection| {
        let durable: String = connection
            .query_row(
                "SELECT status FROM candidates WHERE candidate_id = ?1",
                [id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(durable, "selected");
    });
}

#[cfg(unix)]
#[test]
fn evaluator_status_exact_match_cannot_mask_unsafe_post_commit_storage() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let marker = database.with_extension("sqlite3.writer.lock");
    let store = StoreCoordinator::open(&database).unwrap();
    let root = temp.path().join("root");
    start_and_finish_scan(&store, &root, "scan:evaluator-unsafe");
    let policy = rule(
        "fixture.candidate.evaluator-unsafe",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let id = persist_review_candidate(
        &store,
        &root,
        "candidate:evaluator-unsafe",
        "scan:evaluator-unsafe",
        &policy,
        Vec::new(),
    );

    let error = store
        .transition_candidate_evaluation_status_with_after_commit_hook_for_test(
            &id,
            CandidateEvaluationTransition::DiscoveredToStale,
            || {
                std::fs::write(&marker, b"NOT-A-DUX-MARKER")
                    .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))
            },
        )
        .unwrap_err();
    assert_eq!(error.kind, HistoryErrorKind::OutcomeUnknown);
    store.with_connection(|connection| {
        let durable: String = connection
            .query_row(
                "SELECT status FROM candidates WHERE candidate_id = ?1",
                [id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(durable, "stale");
    });
}

#[test]
fn candidate_status_boundaries_refuse_missing_legacy_corrupt_and_foreign_owned_rows() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    start_and_finish_scan(&store, &root, "scan:review-refusal");
    let missing = CandidateId::new("candidate:review-missing").unwrap();
    assert_eq!(
        store
            .transition_candidate_review_status(&missing, CandidateReviewTransition::Select,)
            .unwrap_err()
            .kind,
        HistoryErrorKind::NotFound
    );
    assert_eq!(
        store
            .transition_candidate_evaluation_status(
                &missing,
                CandidateEvaluationTransition::DiscoveredToStale,
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::NotFound
    );

    let legacy = CandidateId::new("candidate:review-legacy").unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO candidates (
                     candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                     estimated_bytes, created_at_unix_ms, status, record_format_version
                 ) VALUES (?1, 'scan:review-refusal', 'fixture.legacy', 1,
                     'review_required', 1, 1, 'discovered', 1)",
                [legacy.as_str()],
            )
            .unwrap();
    });
    assert_eq!(
        store
            .transition_candidate_review_status(&legacy, CandidateReviewTransition::Select)
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        store
            .transition_candidate_evaluation_status(
                &legacy,
                CandidateEvaluationTransition::DiscoveredToStale,
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        store.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT status FROM candidates WHERE candidate_id = ?1",
                    [legacy.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        }),
        "discovered"
    );

    let policy = rule(
        "fixture.candidate.review-refusal",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let lifecycle = persist_review_candidate(
        &store,
        &root,
        "candidate:review-lifecycle",
        "scan:review-refusal",
        &policy,
        Vec::new(),
    );
    for status in ["stale", "planned", "completed", "failed", "unavailable"] {
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE candidates SET status = ?1 WHERE candidate_id = ?2",
                    params![status, lifecycle.as_str()],
                )
                .unwrap();
        });
        let expected = if status == "planned" {
            HistoryErrorKind::CorruptData
        } else {
            HistoryErrorKind::InvalidTransition
        };
        assert_eq!(
            store
                .transition_candidate_review_status(&lifecycle, CandidateReviewTransition::Select,)
                .unwrap_err()
                .kind,
            expected,
            "review API accepted lifecycle-owned {status} status"
        );
        if matches!(status, "planned" | "completed" | "failed") {
            assert_eq!(
                store
                    .transition_candidate_evaluation_status(
                        &lifecycle,
                        CandidateEvaluationTransition::DiscoveredToStale,
                    )
                    .unwrap_err()
                    .kind,
                expected,
                "evaluator API accepted planner/journal-owned {status} status"
            );
        }
    }

    let corrupt = persist_review_candidate(
        &store,
        &root,
        "candidate:review-corrupt",
        "scan:review-refusal",
        &policy,
        Vec::new(),
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "DELETE FROM candidate_evidence WHERE candidate_id = ?1",
                [corrupt.as_str()],
            )
            .unwrap();
    });
    assert_eq!(
        store
            .transition_candidate_review_status(&corrupt, CandidateReviewTransition::Select,)
            .unwrap_err()
            .kind,
        HistoryErrorKind::CorruptData
    );
    assert_eq!(
        store
            .transition_candidate_evaluation_status(
                &corrupt,
                CandidateEvaluationTransition::DiscoveredToStale,
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::CorruptData
    );
    assert_eq!(
        store.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT status FROM candidates WHERE candidate_id = ?1",
                    [corrupt.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        }),
        "discovered"
    );
}

#[test]
fn complete_candidates_require_a_durably_succeeded_source_scan() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    let policy = rule(
        "fixture.candidate.source-scan",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );

    let cases = [
        ("failed", Some(TerminalScanStatus::Failed)),
        ("cancelled", Some(TerminalScanStatus::Cancelled)),
        ("interrupted", Some(TerminalScanStatus::Interrupted)),
        ("running", None),
    ];
    for (label, terminal) in cases {
        let scan_id = ScanId::new(format!("scan:candidate-source-{label}")).unwrap();
        store
            .record_scan_started(
                &NewScanRecord::try_new_without_root_identity(
                    scan_id.clone(),
                    root.clone(),
                    UNIX_EPOCH + Duration::from_secs(1_750_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        if let Some(terminal) = terminal {
            store
                .record_scan_finished(
                    &ScanCompletionRecord::try_new(
                        scan_id.clone(),
                        UNIX_EPOCH + Duration::from_secs(1_750_000_001),
                        terminal,
                        ScanCounts::default(),
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        let path = root.join(format!("candidate-fixture-{label}"));
        let candidate = candidate(
            &format!("candidate:source-{label}"),
            scan_id.as_str(),
            &policy,
            vec![path.clone()],
            vec![Evidence::MatchedPath { path }],
            Vec::new(),
            None,
            1,
        );
        assert_eq!(
            store
                .record_candidate_discovered(
                    &NewCandidateRecord::try_from_candidate(
                        &candidate,
                        UNIX_EPOCH + Duration::from_secs(1_750_000_002),
                    )
                    .unwrap(),
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition,
            "candidate accepted {label} source scan"
        );
    }

    let queued_scan = ScanId::new("scan:candidate-source-queued").unwrap();
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                queued_scan.clone(),
                root.clone(),
                UNIX_EPOCH + Duration::from_secs(1_750_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                [queued_scan.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE scans SET status = 'queued' WHERE scan_id = ?1",
                [queued_scan.as_str()],
            )
            .unwrap();
    });
    let queued_path = root.join("candidate-fixture-queued");
    let queued_candidate = candidate(
        "candidate:source-queued",
        queued_scan.as_str(),
        &policy,
        vec![queued_path.clone()],
        vec![Evidence::MatchedPath { path: queued_path }],
        Vec::new(),
        None,
        1,
    );
    assert_eq!(
        store
            .record_candidate_discovered(
                &NewCandidateRecord::try_from_candidate(
                    &queued_candidate,
                    UNIX_EPOCH + Duration::from_secs(1_750_000_002),
                )
                .unwrap(),
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );

    start_and_finish_scan(&store, &root, "scan:candidate-source-succeeded");
    let accepted = persist_review_candidate(
        &store,
        &root,
        "candidate:source-succeeded",
        "scan:candidate-source-succeeded",
        &policy,
        Vec::new(),
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE scans SET status = 'failed'
                 WHERE scan_id = 'scan:candidate-source-succeeded'",
                [],
            )
            .unwrap();
    });
    assert_eq!(
        store
            .load_scan(&ScanId::new("scan:candidate-source-succeeded").unwrap())
            .unwrap()
            .unwrap()
            .status(),
        ScanStatus::Failed
    );
    assert_eq!(
        store.load_candidate(&accepted).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn candidate_query_budget_survives_the_nested_source_scan_read() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    start_and_finish_scan(&store, &root, "scan:candidate-budget");
    let policy = rule(
        "fixture.candidate.budget",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let id = persist_review_candidate(
        &store,
        &root,
        "candidate:budget",
        "scan:candidate-budget",
        &policy,
        Vec::new(),
    );

    store.with_connection(|connection| {
        let interrupt = connection.get_interrupt_handle();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let failsafe = std::thread::spawn(move || {
            if done_rx.recv_timeout(Duration::from_secs(2)).is_err() {
                interrupt.interrupt();
            }
        });
        let started = std::time::Instant::now();
        let error = run_bounded_query(connection, || {
            load_candidate_record_within_budget_and_hook(connection, &id, |connection| {
                connection
                    .query_row(
                        "WITH RECURSIVE counter(value) AS (
                             VALUES(0)
                             UNION ALL
                             SELECT value + 1 FROM counter WHERE value < 100000000
                         )
                         SELECT sum(value) FROM counter",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .map(|_| ())
                    .map_err(map_query_sql_error)
            })
            .map(|_| ())
        })
        .unwrap_err();
        let elapsed = started.elapsed();
        let _ = done_tx.send(());
        failsafe.join().unwrap();

        assert_eq!(error.kind, HistoryErrorKind::QueryLimitExceeded);
        assert!(
            elapsed < Duration::from_secs(1),
            "candidate query budget was removed before child loading: {elapsed:?}"
        );
    });
}

#[test]
fn complete_candidate_round_trip_preserves_ordered_facts_after_reopen() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("scan-root-å");
    let first = root.join("candidate-fixture-a");
    let second = root.join("candidate-fixture-b");
    let marker = root.join("Cargo.toml");
    let newest = UNIX_EPOCH + Duration::new(1_750_000_010, 123_456_789);
    let evidence = vec![
        Evidence::MatchedPath {
            path: first.clone(),
        },
        Evidence::RequiredMarker {
            path: marker.clone(),
        },
        Evidence::ForbiddenMarkerAbsent {
            path: root.join("keep"),
        },
        Evidence::BundleIdentifier {
            path: second.clone(),
            identifier: "com.example.fixture".to_owned(),
        },
        Evidence::MinimumAge {
            newest_mtime: newest,
            minimum_age: Duration::new(86_400, 987_654_321),
        },
        Evidence::MinimumSize {
            observed_bytes: 123_456,
            minimum_bytes: 65_536,
        },
        Evidence::InactiveProcess {
            identifier: "com.example.fixture".to_owned(),
        },
        Evidence::CloudUploadComplete {
            path: second.clone(),
        },
    ];
    let policy = rule(
        "fixture.candidate.complete",
        CandidateCategory::DeveloperArtifact,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        true,
    );
    let candidate = candidate(
        "candidate:complete",
        "scan:complete",
        &policy,
        vec![first.clone(), second.clone()],
        evidence.clone(),
        ALL_BLOCKERS.to_vec(),
        Some(newest),
        123_456,
    );
    let created_at = UNIX_EPOCH + Duration::from_millis(1_750_000_020_123);
    let new = NewCandidateRecord::try_from_candidate(&candidate, created_at).unwrap();
    assert_eq!(new.candidate(), &candidate);
    assert_eq!(new.created_at(), created_at);

    {
        let store = StoreCoordinator::open(&database).unwrap();
        start_and_finish_scan(&store, &root, "scan:complete");
        store.record_candidate_discovered(&new).unwrap();
    }

    let store = StoreCoordinator::open(&database).unwrap();
    let stored = store.load_candidate(candidate.id()).unwrap().unwrap();
    let StoredCandidateRecord::Complete(stored) = stored else {
        panic!("format-2 candidate was not returned as complete");
    };
    assert_eq!(stored.id, candidate.id().clone());
    assert_eq!(stored.source_scan_id, candidate.source_scan_id().clone());
    assert_eq!(stored.rule, candidate.rule().clone());
    assert_eq!(stored.category, candidate.category());
    assert_eq!(stored.paths, [first, second]);
    assert_eq!(stored.estimated_bytes, candidate.estimated_bytes());
    assert_eq!(stored.newest_mtime, Some(newest));
    assert_eq!(stored.evidence, evidence);
    assert_eq!(stored.safety, candidate.safety());
    assert_eq!(stored.action, candidate.action());
    assert!(stored.rule_schedule_eligible);
    assert_eq!(stored.blockers, ALL_BLOCKERS);
    assert_eq!(stored.created_at, created_at);
    assert_eq!(stored.status, CandidateHistoryStatus::Discovered);
}

#[test]
fn legal_policy_and_category_mappings_are_exhaustive() {
    let categories = [
        CandidateCategory::DeveloperArtifact,
        CandidateCategory::ApplicationCache,
        CandidateCategory::BrowserCache,
        CandidateCategory::LogAndDiagnostic,
        CandidateCategory::InstallerAndDownload,
        CandidateCategory::DeviceAndSimulatorData,
        CandidateCategory::CloudFile,
        CandidateCategory::LargeReviewItem,
        CandidateCategory::ProtectedSystemData,
        CandidateCategory::UnknownStorage,
    ];
    for category in categories {
        assert_eq!(
            category_from_stored(category_as_stored(category)).unwrap(),
            category
        );
    }
    let pairs = [
        (
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        ),
        (SafetyTier::SafeEvictable, CandidateAction::EvictLocalCopy),
        (SafetyTier::ReviewRequired, CandidateAction::MoveToTrash),
        (SafetyTier::Informational, CandidateAction::RevealOnly),
        (SafetyTier::Informational, CandidateAction::NoAction),
        (SafetyTier::Protected, CandidateAction::RevealOnly),
        (SafetyTier::Protected, CandidateAction::NoAction),
    ];
    for (safety, action) in pairs {
        assert_eq!(
            safety_from_stored(safety_as_stored(safety)).unwrap(),
            safety
        );
        assert_eq!(
            action_from_stored(action_as_stored(action)).unwrap(),
            action
        );
        validate_policy(safety, action, false, corrupt).unwrap();
    }
    for blocker in ALL_BLOCKERS {
        assert_eq!(
            blocker_from_stored(blocker_as_stored(&blocker)).unwrap(),
            blocker
        );
    }
    let statuses = [
        ("discovered", CandidateHistoryStatus::Discovered),
        ("selected", CandidateHistoryStatus::Selected),
        ("dismissed", CandidateHistoryStatus::Dismissed),
        ("stale", CandidateHistoryStatus::Stale),
        ("planned", CandidateHistoryStatus::Planned),
        ("completed", CandidateHistoryStatus::Completed),
        ("failed", CandidateHistoryStatus::Failed),
        ("unavailable", CandidateHistoryStatus::Unavailable),
    ];
    for (stored, status) in statuses {
        assert_eq!(CandidateHistoryStatus::from_stored(stored).unwrap(), status);
    }
}

#[test]
fn duplicate_and_missing_scan_writes_leave_no_partial_children() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("root");
    let path = root.join("candidate-fixture");
    let policy = rule(
        "fixture.candidate.duplicate",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let missing = candidate(
        "candidate:missing-scan",
        "scan:missing",
        &policy,
        vec![path.clone()],
        vec![Evidence::MatchedPath { path: path.clone() }],
        Vec::new(),
        None,
        10,
    );
    let store = StoreCoordinator::open(&database).unwrap();
    let missing =
        NewCandidateRecord::try_from_candidate(&missing, UNIX_EPOCH + Duration::from_secs(1))
            .unwrap();
    assert_eq!(
        store
            .record_candidate_discovered(&missing)
            .unwrap_err()
            .kind,
        HistoryErrorKind::NotFound
    );
    start_and_finish_scan(&store, &root, "scan:duplicate");
    let duplicate = candidate(
        "candidate:duplicate",
        "scan:duplicate",
        &policy,
        vec![path.clone()],
        vec![Evidence::MatchedPath { path }],
        Vec::new(),
        None,
        10,
    );
    let duplicate =
        NewCandidateRecord::try_from_candidate(&duplicate, UNIX_EPOCH + Duration::from_secs(2))
            .unwrap();
    store.record_candidate_discovered(&duplicate).unwrap();
    assert_eq!(
        store
            .record_candidate_discovered(&duplicate)
            .unwrap_err()
            .kind,
        HistoryErrorKind::AlreadyExists
    );
    store.with_connection(|connection| {
        for table in ["candidates", "candidate_paths", "candidate_evidence"] {
            let sql = format!("SELECT count(*) FROM {table}");
            let count: i64 = connection.query_row(&sql, [], |row| row.get(0)).unwrap();
            assert_eq!(count, 1, "unexpected row count in {table}");
        }
    });
}

#[test]
fn input_bounds_and_non_authoritative_paths_fail_before_writing() {
    let policy = rule(
        "fixture.candidate.invalid",
        CandidateCategory::UnknownStorage,
        SafetyTier::Informational,
        CandidateAction::RevealOnly,
        false,
    );
    let absolute = PathBuf::from("/candidate-fixture");
    let cases = [
        candidate(
            "candidate:relative",
            "scan:invalid",
            &policy,
            vec![PathBuf::from("relative")],
            vec![Evidence::MatchedPath {
                path: absolute.clone(),
            }],
            Vec::new(),
            None,
            1,
        ),
        candidate(
            "candidate:pre-epoch",
            "scan:invalid",
            &policy,
            vec![absolute.clone()],
            vec![Evidence::MatchedPath {
                path: absolute.clone(),
            }],
            Vec::new(),
            UNIX_EPOCH.checked_sub(Duration::from_nanos(1)),
            1,
        ),
        candidate(
            "candidate:large-bytes",
            "scan:invalid",
            &policy,
            vec![absolute.clone()],
            vec![Evidence::MatchedPath {
                path: absolute.clone(),
            }],
            Vec::new(),
            None,
            u64::MAX,
        ),
        candidate(
            "candidate:control-text",
            "scan:invalid",
            &policy,
            vec![absolute.clone()],
            vec![Evidence::InactiveProcess {
                identifier: "bad\nidentifier".to_owned(),
            }],
            Vec::new(),
            None,
            1,
        ),
    ];
    for invalid in cases {
        assert_eq!(
            NewCandidateRecord::try_from_candidate(&invalid, UNIX_EPOCH + Duration::from_secs(1))
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidInput
        );
    }

    let too_many_paths = (0..=MAX_PATHS)
        .map(|index| PathBuf::from(format!("/candidate-fixture-{index}")))
        .collect::<Vec<_>>();
    let too_many = candidate(
        "candidate:too-many",
        "scan:invalid",
        &policy,
        too_many_paths,
        vec![Evidence::MatchedPath {
            path: absolute.clone(),
        }],
        Vec::new(),
        None,
        1,
    );
    assert_eq!(
        NewCandidateRecord::try_from_candidate(&too_many, UNIX_EPOCH + Duration::from_secs(1))
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidInput
    );
    let valid = candidate(
        "candidate:valid",
        "scan:invalid",
        &policy,
        vec![absolute.clone()],
        vec![Evidence::MatchedPath { path: absolute }],
        Vec::new(),
        None,
        1,
    );
    assert_eq!(
        NewCandidateRecord::try_from_candidate(
            &valid,
            UNIX_EPOCH.checked_sub(Duration::from_nanos(1)).unwrap()
        )
        .unwrap_err()
        .kind,
        HistoryErrorKind::InvalidInput
    );
    let normalized =
        NewCandidateRecord::try_from_candidate(&valid, UNIX_EPOCH + Duration::new(1, 123_456_789))
            .unwrap();
    assert_eq!(
        normalized.created_at(),
        UNIX_EPOCH + Duration::from_millis(1_123)
    );
}

#[test]
fn legacy_summaries_are_explicit_and_reject_v2_child_pollution() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("root");
    let store = StoreCoordinator::open(&database).unwrap();
    start_and_finish_scan(&store, &root, "scan:legacy-candidate");
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO candidates (
                     candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                     estimated_bytes, created_at_unix_ms, status, record_format_version
                 ) VALUES (
                     'candidate:legacy-summary', 'scan:legacy-candidate',
                     'fixture.candidate.legacy', 3, 'review_required',
                     42, 1234, 'dismissed', 1
                 )",
                [],
            )
            .unwrap();
    });
    let id = CandidateId::new("candidate:legacy-summary").unwrap();
    let stored = store.load_candidate(&id).unwrap().unwrap();
    let StoredCandidateRecord::LegacySummary(summary) = stored else {
        panic!("legacy candidate was presented as complete");
    };
    assert_eq!(summary.id, id);
    assert_eq!(summary.source_scan_id.as_str(), "scan:legacy-candidate");
    assert_eq!(summary.rule.id().as_str(), "fixture.candidate.legacy");
    assert_eq!(summary.rule.revision().get(), 3);
    assert_eq!(summary.safety, SafetyTier::ReviewRequired);
    assert_eq!(summary.estimated_bytes, 42);
    assert_eq!(summary.created_at, UNIX_EPOCH + Duration::from_millis(1234));
    assert_eq!(summary.status, CandidateHistoryStatus::Dismissed);

    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO candidate_paths (
                     candidate_id, path_ordinal, observed_path, observed_path_encoding
                 ) VALUES ('candidate:legacy-summary', 0, ?1, 1)",
                [b"/polluted".as_slice()],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_candidate(&id).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn malformed_ordinals_and_eviction_evidence_fail_closed_on_load() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("root");
    let first = root.join("candidate-fixture-a");
    let second = root.join("candidate-fixture-b");
    let store = StoreCoordinator::open(&database).unwrap();
    start_and_finish_scan(&store, &root, "scan:gapped");
    let regular_policy = rule(
        "fixture.candidate.gapped",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let regular = candidate(
        "candidate:gapped",
        "scan:gapped",
        &regular_policy,
        vec![first.clone(), second.clone()],
        vec![Evidence::MatchedPath {
            path: first.clone(),
        }],
        Vec::new(),
        None,
        2,
    );
    store
        .record_candidate_discovered(
            &NewCandidateRecord::try_from_candidate(&regular, UNIX_EPOCH + Duration::from_secs(1))
                .unwrap(),
        )
        .unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "DELETE FROM candidate_paths
                 WHERE candidate_id = 'candidate:gapped' AND path_ordinal = 0",
                [],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_candidate(regular.id()).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );

    start_and_finish_scan(&store, &root, "scan:eviction");
    let eviction_policy = rule(
        "fixture.candidate.eviction",
        CandidateCategory::CloudFile,
        SafetyTier::SafeEvictable,
        CandidateAction::EvictLocalCopy,
        false,
    );
    let eviction = candidate(
        "candidate:eviction",
        "scan:eviction",
        &eviction_policy,
        vec![first.clone()],
        vec![Evidence::CloudUploadComplete {
            path: first.clone(),
        }],
        Vec::new(),
        None,
        1,
    );
    store
        .record_candidate_discovered(
            &NewCandidateRecord::try_from_candidate(&eviction, UNIX_EPOCH + Duration::from_secs(1))
                .unwrap(),
        )
        .unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE candidate_evidence
                 SET path_value = ?2
                 WHERE candidate_id = ?1 AND evidence_ordinal = 0",
                params![eviction.id().as_str(), b"/unrelated-cloud-path".as_slice()],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_candidate(eviction.id()).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn oversized_child_blob_is_rejected_before_path_materialization() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("root");
    let path = root.join("candidate-fixture");
    let store = StoreCoordinator::open(&database).unwrap();
    start_and_finish_scan(&store, &root, "scan:oversized-candidate");
    let policy = rule(
        "fixture.candidate.oversized",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let candidate = candidate(
        "candidate:oversized",
        "scan:oversized-candidate",
        &policy,
        vec![path.clone()],
        vec![Evidence::MatchedPath { path }],
        Vec::new(),
        None,
        1,
    );
    store
        .record_candidate_discovered(
            &NewCandidateRecord::try_from_candidate(
                &candidate,
                UNIX_EPOCH + Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    store.with_connection(|connection| {
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE candidate_paths SET observed_path = zeroblob(65537)
                 WHERE candidate_id = 'candidate:oversized'",
                [],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });
    assert_eq!(
        store.load_candidate(candidate.id()).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    let encoded_path = encode_host_path(&candidate.paths()[0]).unwrap();
    store.with_connection(|connection| {
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE candidate_paths
                 SET observed_path = ?2, observed_path_encoding = ?3
                 WHERE candidate_id = ?1",
                params![
                    candidate.id().as_str(),
                    encoded_path.bytes,
                    encoded_path.encoding as i64
                ],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE candidate_evidence
                 SET text_value = printf('%.*c', 4097, 'x')
                 WHERE candidate_id = ?1 AND evidence_ordinal = 0",
                [candidate.id().as_str()],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });
    assert_eq!(
        store.load_candidate(candidate.id()).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn maximum_legal_child_counts_fit_the_bounded_reader() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("root");
    let paths = (0..MAX_PATHS)
        .map(|index| root.join(format!("candidate-fixture-{index}")))
        .collect::<Vec<_>>();
    let evidence = (0..MAX_EVIDENCE)
        .map(|index| Evidence::MinimumSize {
            observed_bytes: index as u64 + 1,
            minimum_bytes: index as u64,
        })
        .collect::<Vec<_>>();
    let blockers = (0..MAX_BLOCKERS)
        .map(|index| ALL_BLOCKERS[index % ALL_BLOCKERS.len()].clone())
        .collect::<Vec<_>>();
    let store = StoreCoordinator::open(&database).unwrap();
    start_and_finish_scan(&store, &root, "scan:max-candidate");
    let policy = rule(
        "fixture.candidate.maximum",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let candidate = candidate(
        "candidate:maximum",
        "scan:max-candidate",
        &policy,
        paths,
        evidence,
        blockers,
        None,
        1,
    );
    store
        .record_candidate_discovered(
            &NewCandidateRecord::try_from_candidate(
                &candidate,
                UNIX_EPOCH + Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    let StoredCandidateRecord::Complete(stored) =
        store.load_candidate(candidate.id()).unwrap().unwrap()
    else {
        panic!("maximum format-2 record became incomplete");
    };
    assert_eq!(stored.paths.len(), MAX_PATHS);
    assert_eq!(stored.evidence.len(), MAX_EVIDENCE);
    assert_eq!(stored.blockers.len(), MAX_BLOCKERS);

    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO candidate_evidence (
                     candidate_id, evidence_ordinal, evidence_kind,
                     observed_bytes, minimum_bytes
                 ) VALUES ('candidate:maximum', 512, 'minimum_size', 1, 1)",
                [],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_candidate(candidate.id()).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn external_schema_upgrade_blocks_candidate_reads_and_writes() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let root = temp.path().join("root");
    let path = root.join("candidate-fixture");
    let store = StoreCoordinator::open(&database).unwrap();
    start_and_finish_scan(&store, &root, "scan:blocked-candidate");
    let policy = rule(
        "fixture.candidate.blocked",
        CandidateCategory::ApplicationCache,
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        false,
    );
    let candidate = candidate(
        "candidate:blocked",
        "scan:blocked-candidate",
        &policy,
        vec![path.clone()],
        vec![Evidence::MatchedPath { path }],
        Vec::new(),
        None,
        1,
    );
    let candidate =
        NewCandidateRecord::try_from_candidate(&candidate, UNIX_EPOCH + Duration::from_secs(1))
            .unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO schema_migrations (
                     version, name, checksum_sha256, applied_at_unix_ms
                 ) VALUES (?1, 'future-candidate-schema', zeroblob(32), 2)",
                [i64::from(crate::DATABASE_SCHEMA_VERSION + 1)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", crate::DATABASE_SCHEMA_VERSION + 1)
            .unwrap();
    });
    assert_eq!(
        store
            .record_candidate_discovered(&candidate)
            .unwrap_err()
            .kind,
        HistoryErrorKind::IncompatibleSchema
    );
    assert_eq!(
        store
            .transition_candidate_evaluation_status(
                &CandidateId::new("candidate:future-evaluator").unwrap(),
                CandidateEvaluationTransition::DiscoveredToStale,
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::IncompatibleSchema
    );
    assert_eq!(
        store
            .load_candidate(candidate.candidate().id())
            .unwrap_err()
            .kind,
        HistoryErrorKind::IncompatibleSchema
    );
    assert_eq!(
        store
            .transition_candidate_review_status(
                &CandidateId::new("candidate:future-review").unwrap(),
                CandidateReviewTransition::Select,
            )
            .unwrap_err()
            .kind,
        HistoryErrorKind::IncompatibleSchema
    );
    assert!(
        !format!(
            "{:?}",
            store.record_candidate_discovered(&candidate).unwrap_err()
        )
        .contains(database.to_str().unwrap())
    );
}
