use std::time::UNIX_EPOCH;

use super::*;
use crate::domain::candidate::CandidateInput;
use crate::domain::{
    BlockReason, LocalizedTextKey, ProvenanceUrl, Rule, RuleDefinition, RuleGuards, RuleId,
    RuleMatcher, RuleMatcherDefinition, RuleRevision, RuleScope,
};

fn matcher() -> RuleMatcher {
    RuleMatcher::try_new(RuleMatcherDefinition {
        path_component: Some("fixture-target".to_owned()),
        required_ancestor_markers_any: Vec::new(),
        required_markers_all: Vec::new(),
        forbidden_markers_any: Vec::new(),
        exact_bundle_identifiers: Vec::new(),
        excluded_descendants: Vec::new(),
        protected_descendants: Vec::new(),
    })
    .unwrap()
}

fn rule(safety: SafetyTier, action: CandidateAction, schedule_eligible: bool) -> Rule {
    let suffix = match (safety, action) {
        (SafetyTier::SafeRegenerable, CandidateAction::RemoveKnownRegenerableContents) => {
            "regenerable"
        }
        (SafetyTier::SafeEvictable, CandidateAction::EvictLocalCopy) => "evictable",
        (SafetyTier::ReviewRequired, CandidateAction::MoveToTrash) => "review",
        (SafetyTier::Informational, CandidateAction::RevealOnly) => "informational-reveal",
        (SafetyTier::Informational, CandidateAction::NoAction) => "informational-none",
        (SafetyTier::Protected, CandidateAction::RevealOnly) => "protected-reveal",
        (SafetyTier::Protected, CandidateAction::NoAction) => "protected-none",
        _ => panic!("invalid test policy pair"),
    };
    Rule::try_new(RuleDefinition {
        reference: RuleRef::new(
            RuleId::new(format!("fixture.plan.{suffix}")).unwrap(),
            RuleRevision::new(7).unwrap(),
        ),
        title_key: LocalizedTextKey::new(format!("fixture.plan.{suffix}.title")).unwrap(),
        category: if safety == SafetyTier::SafeEvictable {
            CandidateCategory::CloudFile
        } else {
            CandidateCategory::DeveloperArtifact
        },
        scope: RuleScope::SelectedScanRoot,
        matcher: matcher(),
        guards: RuleGuards::try_new(None, 0, Vec::new(), safety == SafetyTier::SafeEvictable)
            .unwrap(),
        safety,
        action,
        schedule_eligible,
        explanation_key: LocalizedTextKey::new(format!("fixture.plan.{suffix}.explanation"))
            .unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/fixture-plan").unwrap()],
    })
    .unwrap()
}

struct CandidateOptions<'a> {
    id: &'a str,
    scan_id: &'a str,
    paths: Vec<&'a str>,
    estimated_bytes: u64,
    blockers: Vec<BlockReason>,
    schedule_eligible: bool,
}

fn candidate(
    safety: SafetyTier,
    action: CandidateAction,
    options: CandidateOptions<'_>,
) -> Candidate {
    let paths = options
        .paths
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let evidence = if action == CandidateAction::EvictLocalCopy {
        paths
            .iter()
            .cloned()
            .map(|path| Evidence::CloudUploadComplete { path })
            .collect()
    } else {
        vec![Evidence::RequiredMarker {
            path: PathBuf::from("/fixture/Cargo.toml"),
        }]
    };
    Candidate::try_from_rule(
        &rule(safety, action, options.schedule_eligible),
        CandidateInput::new(
            CandidateId::new(options.id).unwrap(),
            paths,
            options.estimated_bytes,
            Some(UNIX_EPOCH + Duration::from_secs(42)),
            evidence,
            options.blockers,
            ScanId::new(options.scan_id).unwrap(),
        ),
    )
    .unwrap()
}

fn regenerable(id: &str, path: &str, estimated_bytes: u64) -> Candidate {
    candidate(
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        CandidateOptions {
            id,
            scan_id: "scan:one",
            paths: vec![path],
            estimated_bytes,
            blockers: Vec::new(),
            schedule_eligible: false,
        },
    )
}

fn plan(mode: CleanupMode, candidates: &[Candidate]) -> CleanupPlan {
    CleanupPlan::try_from_candidates(
        CleanupPlanId::new("plan:one").unwrap(),
        UNIX_EPOCH + Duration::from_secs(100),
        mode,
        candidates,
    )
    .unwrap()
}

fn trusted_facts(candidate: &Candidate) -> CleanupPlanCandidateFacts {
    CleanupPlanCandidateFacts {
        candidate_id: candidate.id().clone(),
        source_scan_id: candidate.source_scan_id().clone(),
        rule: candidate.rule().clone(),
        category: candidate.category(),
        paths: candidate.paths().to_vec(),
        estimated_bytes: candidate.estimated_bytes(),
        newest_mtime: candidate.newest_mtime(),
        evidence: candidate.evidence().to_vec(),
        safety: candidate.safety(),
        action: candidate.action(),
        rule_schedule_eligible: candidate.rule_marks_schedule_eligible(),
    }
}

#[test]
fn explorer_trash_selection_plan_is_fixed_review_only_and_path_bound() {
    let plan = CleanupPlan::try_from_trash_selection(
        CleanupPlanId::new("plan:explorer-trash-test").unwrap(),
        UNIX_EPOCH + Duration::from_secs(100),
        ScanId::new("scan:explorer").unwrap(),
        CandidateId::new("candidate:explorer-trash-test").unwrap(),
        PathBuf::from("/Users/example/Library/Caches/item"),
    )
    .unwrap();

    assert_eq!(plan.mode(), CleanupMode::Trash);
    assert_eq!(plan.estimated_bytes(), 0);
    assert_eq!(plan.items().len(), 1);
    let item = &plan.items()[0];
    assert_eq!(item.action(), CandidateAction::MoveToTrash);
    assert_eq!(item.safety(), SafetyTier::ReviewRequired);
    assert!(!item.rule_marks_schedule_eligible());
    assert_eq!(
        item.paths(),
        &[PathBuf::from("/Users/example/Library/Caches/item")]
    );
    assert!(
        plan.warnings()
            .contains(&PlanWarning::EstimatedBytesUnverified)
    );
    assert!(
        plan.warnings()
            .contains(&PlanWarning::TrashDoesNotFreeSpaceImmediately)
    );
}

#[test]
fn explorer_trash_selection_plan_rejects_roots_and_traversal() {
    let make = |path| {
        CleanupPlan::try_from_trash_selection(
            CleanupPlanId::new("plan:explorer-trash-invalid").unwrap(),
            UNIX_EPOCH,
            ScanId::new("scan:explorer").unwrap(),
            CandidateId::new("candidate:explorer-trash-invalid").unwrap(),
            PathBuf::from(path),
        )
    };

    assert_eq!(
        make("/").unwrap_err(),
        CleanupPlanValidationError::InvalidSelectionPath
    );
    assert_eq!(
        make("/Users/example/../other").unwrap_err(),
        CleanupPlanValidationError::InvalidSelectionPath
    );
    assert_eq!(
        make("relative/item").unwrap_err(),
        CleanupPlanValidationError::InvalidSelectionPath
    );
}

#[test]
fn plan_freezes_candidate_facts_totals_and_expiration() {
    let first = regenerable("candidate:first", "/fixture/first", 400);
    let second = regenerable("candidate:second", "/fixture/second", 600);
    let plan = plan(CleanupMode::PermanentSafe, &[first.clone(), second]);

    assert_eq!(plan.id().as_str(), "plan:one");
    assert_eq!(plan.source_scan_id().as_str(), "scan:one");
    assert_eq!(plan.mode(), CleanupMode::PermanentSafe);
    assert_eq!(plan.estimated_bytes(), 1_000);
    assert_eq!(plan.items().len(), 2);
    assert_eq!(plan.created_at(), UNIX_EPOCH + Duration::from_secs(100));
    assert_eq!(plan.expires_at(), plan.created_at() + CLEANUP_PLAN_VALIDITY);
    assert!(!plan.has_expired_at(plan.expires_at() - Duration::from_nanos(1)));
    assert!(plan.has_expired_at(plan.expires_at()));

    let item = &plan.items()[0];
    assert_eq!(item.candidate_id(), first.id());
    assert_eq!(item.rule(), first.rule());
    assert_eq!(item.rule().revision().get(), 7);
    assert_eq!(item.category(), first.category());
    assert_eq!(item.paths(), first.paths());
    assert_eq!(item.estimated_bytes(), first.estimated_bytes());
    assert_eq!(item.newest_mtime(), first.newest_mtime());
    assert_eq!(item.evidence(), first.evidence());
    assert_eq!(item.safety(), first.safety());
    assert_eq!(item.action(), first.action());
    assert_eq!(
        item.rule_marks_schedule_eligible(),
        first.rule_marks_schedule_eligible()
    );
}

#[test]
fn trusted_candidate_facts_construct_the_same_bounded_plan_shape() {
    let candidate = regenerable("candidate:trusted-facts", "/fixture/trusted", 321);
    let plan = CleanupPlan::try_from_trusted_candidate_facts(
        CleanupPlanId::new("plan:trusted-facts").unwrap(),
        UNIX_EPOCH + Duration::from_secs(100),
        CleanupMode::PermanentSafe,
        &[trusted_facts(&candidate)],
    )
    .unwrap();

    assert_eq!(plan.source_scan_id(), candidate.source_scan_id());
    assert_eq!(plan.estimated_bytes(), candidate.estimated_bytes());
    assert_eq!(plan.items().len(), 1);
    assert_eq!(plan.items()[0].candidate_id(), candidate.id());
    assert_eq!(plan.items()[0].paths(), candidate.paths());
    assert_eq!(plan.items()[0].evidence(), candidate.evidence());
    assert!(
        plan.warnings()
            .contains(&PlanWarning::PermanentRemovalCannotBeUndone)
    );
}

#[test]
fn trusted_candidate_facts_repeat_mode_and_overlap_validation() {
    let first = regenerable("candidate:trusted-first", "/fixture/parent", 1);
    let second = regenerable("candidate:trusted-second", "/fixture/parent/child", 1);
    assert_eq!(
        CleanupPlan::try_from_trusted_candidate_facts(
            CleanupPlanId::new("plan:trusted-overlap").unwrap(),
            UNIX_EPOCH,
            CleanupMode::Trash,
            &[trusted_facts(&first)],
        )
        .unwrap_err(),
        CleanupPlanValidationError::IncompatibleMode {
            candidate_index: 0,
            mode: CleanupMode::Trash,
            action: CandidateAction::RemoveKnownRegenerableContents,
        }
    );
    assert_eq!(
        CleanupPlan::try_from_trusted_candidate_facts(
            CleanupPlanId::new("plan:trusted-overlap").unwrap(),
            UNIX_EPOCH,
            CleanupMode::PermanentSafe,
            &[trusted_facts(&first), trusted_facts(&second)],
        )
        .unwrap_err(),
        CleanupPlanValidationError::OverlappingPaths {
            first_candidate_index: 0,
            first_path_index: 0,
            second_candidate_index: 1,
            second_path_index: 0,
        }
    );
}

#[test]
fn plans_reject_missing_duplicate_and_mixed_scan_candidates() {
    let created_at = UNIX_EPOCH;
    let id = CleanupPlanId::new("plan:validation").unwrap();
    assert_eq!(
        CleanupPlan::try_from_candidates(id.clone(), created_at, CleanupMode::DryRun, &[])
            .unwrap_err(),
        CleanupPlanValidationError::MissingCandidates
    );

    let first = regenerable("candidate:duplicate", "/fixture/first", 1);
    let from_other_scan = candidate(
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        CandidateOptions {
            id: "candidate:other",
            scan_id: "scan:other",
            paths: vec!["/fixture/second"],
            estimated_bytes: 1,
            blockers: Vec::new(),
            schedule_eligible: false,
        },
    );

    assert_eq!(
        CleanupPlan::try_from_candidates(
            id.clone(),
            created_at,
            CleanupMode::DryRun,
            &[first.clone(), first.clone()],
        )
        .unwrap_err(),
        CleanupPlanValidationError::DuplicateCandidate {
            first_index: 0,
            duplicate_index: 1,
        }
    );
    assert_eq!(
        CleanupPlan::try_from_candidates(
            id,
            created_at,
            CleanupMode::DryRun,
            &[first, from_other_scan],
        )
        .unwrap_err(),
        CleanupPlanValidationError::MixedSourceScans {
            first_index: 0,
            conflicting_index: 1,
        }
    );
}

#[test]
fn plans_reject_blocked_and_non_cleanup_candidates() {
    let blocked = candidate(
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        CandidateOptions {
            id: "candidate:blocked",
            scan_id: "scan:one",
            paths: vec!["/fixture/blocked"],
            estimated_bytes: 1,
            blockers: vec![BlockReason::PartialScanCoverage, BlockReason::ActiveUse],
            schedule_eligible: false,
        },
    );
    assert_eq!(
        CleanupPlan::try_from_candidates(
            CleanupPlanId::new("plan:blocked").unwrap(),
            UNIX_EPOCH,
            CleanupMode::DryRun,
            &[blocked],
        )
        .unwrap_err(),
        CleanupPlanValidationError::BlockedCandidate { candidate_index: 0 }
    );

    let informational = candidate(
        SafetyTier::Informational,
        CandidateAction::RevealOnly,
        CandidateOptions {
            id: "candidate:information",
            scan_id: "scan:one",
            paths: vec!["/fixture/information"],
            estimated_bytes: 1,
            blockers: Vec::new(),
            schedule_eligible: false,
        },
    );
    assert_eq!(
        CleanupPlan::try_from_candidates(
            CleanupPlanId::new("plan:information").unwrap(),
            UNIX_EPOCH,
            CleanupMode::DryRun,
            &[informational],
        )
        .unwrap_err(),
        CleanupPlanValidationError::NonCleanupCandidate { candidate_index: 0 }
    );
}

#[test]
fn mode_policy_compatibility_is_exhaustive_and_eviction_is_explicit() {
    let policies = [
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
    let modes = [
        CleanupMode::DryRun,
        CleanupMode::Trash,
        CleanupMode::PermanentSafe,
        CleanupMode::EvictLocalCopy,
    ];

    for (policy_index, (safety, action)) in policies.into_iter().enumerate() {
        for mode in modes {
            let candidate = candidate(
                safety,
                action,
                CandidateOptions {
                    id: &format!("candidate:{policy_index}"),
                    scan_id: "scan:one",
                    paths: vec!["/fixture/policy"],
                    estimated_bytes: 1,
                    blockers: Vec::new(),
                    schedule_eligible: false,
                },
            );
            let result = CleanupPlan::try_from_candidates(
                CleanupPlanId::new(format!("plan:{policy_index}")).unwrap(),
                UNIX_EPOCH,
                mode,
                &[candidate],
            );
            let cleanup = action.is_cleanup_operation();
            let compatible = match mode {
                CleanupMode::DryRun => cleanup,
                CleanupMode::Trash => action == CandidateAction::MoveToTrash,
                CleanupMode::PermanentSafe => {
                    action == CandidateAction::RemoveKnownRegenerableContents
                }
                CleanupMode::EvictLocalCopy => action == CandidateAction::EvictLocalCopy,
            };
            assert_eq!(result.is_ok(), compatible, "{safety:?}/{action:?}/{mode:?}");
        }
    }
}

#[test]
fn unresolved_duplicate_and_parent_child_paths_fail_closed() {
    let first = regenerable("candidate:first", "/fixture/cache", 1);
    for second_path in ["/fixture/cache", "/fixture/cache/child"] {
        let second = regenerable("candidate:second", second_path, 1);
        assert!(matches!(
            CleanupPlan::try_from_candidates(
                CleanupPlanId::new("plan:overlap").unwrap(),
                UNIX_EPOCH,
                CleanupMode::PermanentSafe,
                &[first.clone(), second],
            )
            .unwrap_err(),
            CleanupPlanValidationError::OverlappingPaths { .. }
        ));
    }

    let child = regenerable("candidate:child", "/fixture/cache/child", 1);
    assert!(matches!(
        CleanupPlan::try_from_candidates(
            CleanupPlanId::new("plan:reverse-overlap").unwrap(),
            UNIX_EPOCH,
            CleanupMode::PermanentSafe,
            &[child, first],
        )
        .unwrap_err(),
        CleanupPlanValidationError::OverlappingPaths { .. }
    ));

    let parent = regenerable("candidate:parent", "/fixture/cache", 1);
    let interposed = regenerable("candidate:interposed", "/fixture/cache-archive", 1);
    let child = regenerable("candidate:child-after-sibling", "/fixture/cache/child", 1);
    assert!(matches!(
        CleanupPlan::try_from_candidates(
            CleanupPlanId::new("plan:sorted-overlap").unwrap(),
            UNIX_EPOCH,
            CleanupMode::PermanentSafe,
            &[parent, interposed, child],
        )
        .unwrap_err(),
        CleanupPlanValidationError::OverlappingPaths { .. }
    ));

    let within_one_candidate = candidate(
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        CandidateOptions {
            id: "candidate:internal-overlap",
            scan_id: "scan:one",
            paths: vec!["/fixture/one", "/fixture/one/child"],
            estimated_bytes: 1,
            blockers: Vec::new(),
            schedule_eligible: false,
        },
    );
    assert!(matches!(
        CleanupPlan::try_from_candidates(
            CleanupPlanId::new("plan:internal-overlap").unwrap(),
            UNIX_EPOCH,
            CleanupMode::PermanentSafe,
            &[within_one_candidate],
        )
        .unwrap_err(),
        CleanupPlanValidationError::OverlappingPaths { .. }
    ));
}

#[test]
fn byte_and_expiration_arithmetic_fail_closed() {
    let first = regenerable("candidate:first", "/fixture/first", u64::MAX);
    let second = regenerable("candidate:second", "/fixture/second", 1);
    assert_eq!(
        CleanupPlan::try_from_candidates(
            CleanupPlanId::new("plan:bytes").unwrap(),
            UNIX_EPOCH,
            CleanupMode::PermanentSafe,
            &[first.clone(), second],
        )
        .unwrap_err(),
        CleanupPlanValidationError::EstimatedBytesOverflow
    );

    let latest = latest_representable_time();
    assert_eq!(
        CleanupPlan::try_from_candidates(
            CleanupPlanId::new("plan:time").unwrap(),
            latest,
            CleanupMode::PermanentSafe,
            &[first],
        )
        .unwrap_err(),
        CleanupPlanValidationError::ExpirationOverflow
    );
}

fn latest_representable_time() -> SystemTime {
    let mut lower = 0_u64;
    let mut upper = u64::MAX;
    while lower < upper {
        let midpoint = lower + (upper - lower) / 2 + 1;
        if UNIX_EPOCH
            .checked_add(Duration::from_secs(midpoint))
            .is_some()
        {
            lower = midpoint;
        } else {
            upper = midpoint - 1;
        }
    }
    UNIX_EPOCH.checked_add(Duration::from_secs(lower)).unwrap()
}

#[test]
fn warnings_are_mandatory_deduplicated_and_operation_specific() {
    let permanent = regenerable("candidate:permanent", "/fixture/permanent", 1);
    let second_permanent =
        regenerable("candidate:second-permanent", "/fixture/second-permanent", 1);
    let trash = candidate(
        SafetyTier::ReviewRequired,
        CandidateAction::MoveToTrash,
        CandidateOptions {
            id: "candidate:trash",
            scan_id: "scan:one",
            paths: vec!["/fixture/trash"],
            estimated_bytes: 1,
            blockers: Vec::new(),
            schedule_eligible: false,
        },
    );
    let eviction = candidate(
        SafetyTier::SafeEvictable,
        CandidateAction::EvictLocalCopy,
        CandidateOptions {
            id: "candidate:eviction",
            scan_id: "scan:one",
            paths: vec!["/fixture/eviction"],
            estimated_bytes: 1,
            blockers: Vec::new(),
            schedule_eligible: false,
        },
    );

    let dry_run = plan(
        CleanupMode::DryRun,
        &[
            permanent.clone(),
            second_permanent,
            trash.clone(),
            eviction.clone(),
        ],
    );
    assert_eq!(
        dry_run.warnings(),
        [
            PlanWarning::EstimatedBytesUnverified,
            PlanWarning::DryRunDoesNotMutate,
            PlanWarning::TrashDoesNotFreeSpaceImmediately,
            PlanWarning::PermanentRemovalCannotBeUndone,
            PlanWarning::CloudEvictionRequiresNetworkToRedownload,
        ]
    );
    assert_eq!(
        plan(CleanupMode::Trash, &[trash]).warnings(),
        [
            PlanWarning::EstimatedBytesUnverified,
            PlanWarning::TrashDoesNotFreeSpaceImmediately,
        ]
    );
    assert_eq!(
        plan(CleanupMode::PermanentSafe, &[permanent]).warnings(),
        [
            PlanWarning::EstimatedBytesUnverified,
            PlanWarning::PermanentRemovalCannotBeUndone,
        ]
    );
    assert_eq!(
        plan(CleanupMode::EvictLocalCopy, &[eviction]).warnings(),
        [
            PlanWarning::EstimatedBytesUnverified,
            PlanWarning::CloudEvictionRequiresNetworkToRedownload,
        ]
    );
}

#[test]
fn permanent_plan_projects_to_dry_run_without_changing_frozen_facts_or_expiry() {
    let original = plan(
        CleanupMode::PermanentSafe,
        &[regenerable(
            "candidate:projection",
            "/fixture/projection",
            41,
        )],
    );
    let expected_id = original.id().clone();
    let expected_created_at = original.created_at();
    let expected_source_scan_id = original.source_scan_id().clone();
    let expected_items = original.items().to_vec();
    let expected_bytes = original.estimated_bytes();
    let expected_expiry = original.expires_at();

    let dry_run = original.into_dry_run().unwrap();

    assert_eq!(dry_run.id(), &expected_id);
    assert_eq!(dry_run.created_at(), expected_created_at);
    assert_eq!(dry_run.source_scan_id(), &expected_source_scan_id);
    assert_eq!(dry_run.items(), expected_items);
    assert_eq!(dry_run.estimated_bytes(), expected_bytes);
    assert_eq!(dry_run.expires_at(), expected_expiry);
    assert_eq!(dry_run.mode(), CleanupMode::DryRun);
    assert_eq!(
        dry_run.warnings(),
        [
            PlanWarning::EstimatedBytesUnverified,
            PlanWarning::DryRunDoesNotMutate,
            PlanWarning::PermanentRemovalCannotBeUndone,
        ]
    );
}

#[test]
fn only_permanent_safe_plan_can_use_the_trusted_dry_run_projection() {
    let dry_run = plan(
        CleanupMode::DryRun,
        &[regenerable(
            "candidate:already-dry",
            "/fixture/already-dry",
            1,
        )],
    );
    assert_eq!(
        dry_run.into_dry_run(),
        Err(CleanupPlanValidationError::InvalidDryRunSourceMode)
    );
}

#[test]
fn manual_permanent_plan_preserves_but_does_not_require_schedule_eligibility() {
    let manual = regenerable("candidate:manual", "/fixture/manual", 1);
    let scheduled = candidate(
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
        CandidateOptions {
            id: "candidate:scheduled",
            scan_id: "scan:one",
            paths: vec!["/fixture/scheduled"],
            estimated_bytes: 1,
            blockers: Vec::new(),
            schedule_eligible: true,
        },
    );
    let plan = plan(CleanupMode::PermanentSafe, &[manual, scheduled]);
    assert!(!plan.items()[0].rule_marks_schedule_eligible());
    assert!(plan.items()[1].rule_marks_schedule_eligible());
}

#[test]
fn operation_status_taxonomy_includes_non_destructive_eviction() {
    let statuses = [
        OperationStatus::Planned,
        OperationStatus::DryRun,
        OperationStatus::Trashed,
        OperationStatus::Removed,
        OperationStatus::Evicted,
        OperationStatus::Skipped,
        OperationStatus::Rejected,
        OperationStatus::Failed,
        OperationStatus::ChangedSincePlan,
    ];
    assert_eq!(statuses.len(), 9);
}
