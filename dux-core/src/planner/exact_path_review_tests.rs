use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::super::rule_scope_grant::authorize_rule_target;
use super::*;
use crate::domain::{
    BlockReason, CandidateCategory, CandidateInput, Evidence, LocalizedTextKey, ProvenanceUrl,
    Rule, RuleDefinition, RuleGuards, RuleId, RuleMatcher, RuleMatcherDefinition, RuleRef,
    RuleRevision, RuleScope, ScanId,
};

fn rule(id: &str, safety: SafetyTier, action: CandidateAction) -> Rule {
    Rule::try_new(RuleDefinition {
        reference: RuleRef::new(RuleId::new(id).unwrap(), RuleRevision::new(1).unwrap()),
        title_key: LocalizedTextKey::new(format!("fixture.{id}.title")).unwrap(),
        category: CandidateCategory::DeveloperArtifact,
        scope: RuleScope::SelectedScanRoot,
        matcher: RuleMatcher::try_new(RuleMatcherDefinition {
            path_component: Some("cache".to_owned()),
            required_ancestor_markers_any: Vec::new(),
            required_markers_all: Vec::new(),
            forbidden_markers_any: Vec::new(),
            exact_bundle_identifiers: Vec::new(),
            excluded_descendants: Vec::new(),
            protected_descendants: Vec::new(),
        })
        .unwrap(),
        guards: RuleGuards::try_new(None, 0, Vec::new(), false).unwrap(),
        safety,
        action,
        schedule_eligible: false,
        explanation_key: LocalizedTextKey::new(format!("fixture.{id}.explanation")).unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/exact-review").unwrap()],
    })
    .unwrap()
}

fn candidate(
    id: &str,
    rule_id: &str,
    paths: &[PathBuf],
    estimated_bytes: u64,
    blockers: Vec<BlockReason>,
    safety: SafetyTier,
    action: CandidateAction,
) -> Candidate {
    Candidate::try_from_rule(
        &rule(rule_id, safety, action),
        CandidateInput::new(
            CandidateId::new(id).unwrap(),
            paths.to_vec(),
            estimated_bytes,
            Some(UNIX_EPOCH + Duration::from_secs(7)),
            vec![Evidence::RequiredMarker {
                path: "/fixture/Cargo.toml".into(),
            }],
            blockers,
            ScanId::new("scan:exact-review").unwrap(),
        ),
    )
    .unwrap()
}

#[cfg(target_os = "macos")]
fn trusted_candidate(path: PathBuf) -> Candidate {
    let rule = Rule::try_new(RuleDefinition {
        reference: RuleRef::new(
            RuleId::new("developer.rust.target").unwrap(),
            RuleRevision::new(crate::domain::SAFE_RUST_RULE_REVISION).unwrap(),
        ),
        title_key: LocalizedTextKey::new("developer.rust.target.title").unwrap(),
        category: CandidateCategory::DeveloperArtifact,
        scope: RuleScope::SelectedScanRoot,
        matcher: RuleMatcher::try_new(RuleMatcherDefinition {
            path_component: Some("target".to_owned()),
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
        explanation_key: LocalizedTextKey::new("developer.rust.target.explanation").unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/rust-target").unwrap()],
    })
    .unwrap();
    Candidate::try_from_rule(
        &rule,
        CandidateInput::new(
            CandidateId::new("candidate:trusted-plan").unwrap(),
            vec![path],
            7,
            Some(UNIX_EPOCH + Duration::from_secs(7)),
            vec![Evidence::RequiredMarker {
                path: "/fixture/Cargo.toml".into(),
            }],
            Vec::new(),
            ScanId::new("scan:exact-review").unwrap(),
        ),
    )
    .unwrap()
}

#[cfg(target_os = "macos")]
fn exact_rust_target_candidate(path: PathBuf, newest_mtime: SystemTime) -> Candidate {
    let rule = Rule::try_new(RuleDefinition {
        reference: RuleRef::new(
            RuleId::new("developer.rust.target").unwrap(),
            RuleRevision::new(crate::domain::SAFE_RUST_RULE_REVISION).unwrap(),
        ),
        title_key: LocalizedTextKey::new("developer.rust.target.title").unwrap(),
        category: CandidateCategory::DeveloperArtifact,
        scope: RuleScope::SelectedScanRoot,
        matcher: RuleMatcher::try_new(RuleMatcherDefinition {
            path_component: Some("target".to_owned()),
            required_ancestor_markers_any: vec!["Cargo.toml".to_owned()],
            required_markers_all: vec!["CACHEDIR.TAG".to_owned()],
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
        explanation_key: LocalizedTextKey::new("developer.rust.target.explanation").unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/rust-target").unwrap()],
    })
    .unwrap();
    let manifest = path.parent().unwrap().join("Cargo.toml");
    let cache_tag = path.join("CACHEDIR.TAG");
    Candidate::try_from_rule(
        &rule,
        CandidateInput::new(
            CandidateId::new("candidate:exact-rust-target").unwrap(),
            vec![path.clone()],
            41,
            Some(newest_mtime),
            vec![
                Evidence::MatchedPath { path },
                Evidence::RequiredMarker { path: manifest },
                Evidence::RequiredMarker { path: cache_tag },
                Evidence::MinimumAge {
                    newest_mtime,
                    minimum_age: crate::domain::SAFE_RUST_RULE_MINIMUM_AGE,
                },
            ],
            Vec::new(),
            ScanId::new("scan:exact-review").unwrap(),
        ),
    )
    .unwrap()
}

fn root() -> (tempfile::TempDir, PathBuf, CanonicalScanRoot) {
    let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let root_path = fs::canonicalize(directory.path()).unwrap();
    fs::create_dir(root_path.join("cache")).unwrap();
    fs::create_dir(root_path.join("other")).unwrap();
    fs::write(root_path.join("cache/file"), b"fixture").unwrap();
    let lexical = validate_scan_root(&root_path).unwrap();
    let canonical = crate::path_validation::capture_scan_root(lexical).unwrap();
    (directory, root_path, canonical)
}

#[cfg(target_os = "macos")]
fn trusted_rust_target_plan(
    coupled: bool,
    newest_mtime: SystemTime,
) -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
    TrustedReviewedCleanupPlan,
) {
    let (directory, root_path, scan_root) = root();
    let project = root_path.join("project");
    let target = project.join("target");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        b"[package]\nname = \"fixture\"\n",
    )
    .unwrap();
    fs::write(
        target.join("CACHEDIR.TAG"),
        b"Signature: 8a477f597d28d172789f06886806bc55",
    )
    .unwrap();
    fs::write(target.join("artifact.o"), b"unchanged artifact").unwrap();
    for path in [
        target.join("CACHEDIR.TAG"),
        target.join("artifact.o"),
        target.clone(),
    ] {
        fs::File::open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(newest_mtime))
            .unwrap();
    }
    let candidate = exact_rust_target_candidate(target.clone(), newest_mtime);
    let review = review_exact_paths(
        &scan_root,
        std::slice::from_ref(&candidate),
        CleanupMode::PermanentSafe,
    )
    .unwrap();
    let authorization = authorize_rule_target(
        &scan_root,
        review.items()[0].paths()[0].snapshot().clone(),
        review.items()[0].rule(),
    )
    .unwrap();
    let mut reviewed = review
        .into_trusted_permanent_plan(
            crate::domain::CleanupPlanId::new("plan:exact-rust-target").unwrap(),
            SystemTime::now(),
            vec![authorization],
        )
        .unwrap();
    reviewed.trusted_rust_target_coupling = coupled;
    (directory, root_path, target, reviewed)
}

#[test]
fn review_captures_exact_live_identity_and_remains_non_actionable() {
    let (_directory, root_path, scan_root) = root();
    let target_directory = root_path.join("other");
    let target_file = root_path.join("cache/file");
    let candidates = vec![candidate(
        "candidate:exact",
        "fixture.rule",
        &[target_file.clone(), target_directory.clone()],
        7,
        Vec::new(),
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    )];

    let review = review_exact_paths(&scan_root, &candidates, CleanupMode::DryRun).unwrap();

    assert_eq!(review.scan_root(), scan_root.requested_path());
    assert_eq!(review.boundary().root_identity(), scan_root.identity());
    assert_eq!(review.boundary().scan_root(), scan_root.canonical_path());
    assert_eq!(review.source_scan_id().as_str(), "scan:exact-review");
    assert_eq!(review.mode(), CleanupMode::DryRun);
    assert_eq!(review.estimated_bytes(), 7);
    assert!(!review.is_actionable());
    assert_eq!(review.candidate_groups().groups().len(), 1);
    assert!(!review.candidate_groups().has_unresolved_overlaps());
    assert_eq!(review.items().len(), 1);
    assert_eq!(
        review.items()[0].category(),
        CandidateCategory::DeveloperArtifact
    );
    assert_eq!(review.items()[0].paths().len(), 2);
    assert_eq!(review.items()[0].paths()[0].requested_path(), target_file);
    assert_eq!(
        review.items()[0].paths()[0].snapshot().target_kind(),
        crate::path_validation::FilesystemEntryKind::RegularFile
    );
    assert_eq!(
        review.items()[0].paths()[1].requested_path(),
        target_directory
    );
    assert_eq!(
        review.items()[0].paths()[1].snapshot().target_kind(),
        crate::path_validation::FilesystemEntryKind::Directory
    );
    assert!(review.items()[0].paths().iter().all(|path| matches!(
        path.protection(),
        ExactPathProtection::NoTextualMatch { .. }
    )));
    assert!(root_path.join("cache/file").exists());
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test replaces only a TempDir-owned scan root to prove boundary identity rejection"
)]
fn retained_review_boundary_rejects_scan_root_replacement() {
    let (_directory, root_path, scan_root) = root();
    let target = root_path.join("cache/file");
    let candidate = candidate(
        "candidate:boundary",
        "fixture.rule",
        std::slice::from_ref(&target),
        7,
        Vec::new(),
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );

    let review = review_exact_paths(&scan_root, &[candidate], CleanupMode::DryRun).unwrap();
    let replacement = tempfile::tempdir_in(root_path.parent().unwrap())
        .unwrap()
        .keep();
    // DUX-DESTRUCTIVE: allow=test-exact-review-replacement-remove-temp -- remove only the empty TempDir-owned replacement directory for an identity-race fixture
    fs::remove_dir(&replacement).unwrap();
    // DUX-DESTRUCTIVE: allow=test-exact-review-replacement-rename-away -- rename only the TempDir-owned scan root to prove boundary replacement is rejected
    fs::rename(&root_path, &replacement).unwrap();
    fs::create_dir(&root_path).unwrap();

    assert!(review.boundary().revalidate().is_err());

    // DUX-DESTRUCTIVE: allow=test-exact-review-replacement-remove-root -- remove only the replacement fixture root created by this test
    fs::remove_dir_all(&root_path).unwrap();
    // DUX-DESTRUCTIVE: allow=test-exact-review-replacement-rename-back -- restore only the TempDir-owned scan root after the identity-race fixture
    fs::rename(replacement, root_path).unwrap();
}

#[test]
fn exact_duplicate_is_coalesced_and_review_order_is_id_deterministic() {
    let (_directory, root_path, scan_root) = root();
    let target = root_path.join("cache/file");
    let candidates = vec![
        candidate(
            "candidate:z",
            "fixture.rule",
            std::slice::from_ref(&target),
            7,
            Vec::new(),
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        ),
        candidate(
            "candidate:a",
            "fixture.rule",
            std::slice::from_ref(&target),
            7,
            Vec::new(),
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        ),
    ];

    let review = review_exact_paths(&scan_root, &candidates, CleanupMode::DryRun).unwrap();

    assert_eq!(review.items().len(), 1);
    assert_eq!(review.items()[0].candidate_id().as_str(), "candidate:a");
    assert_eq!(review.estimated_bytes(), 7);
}

#[test]
fn invalid_scope_and_lexical_paths_fail_closed() {
    let (_directory, root_path, scan_root) = root();
    let cases = [
        root_path.to_path_buf(),
        root_path.join("missing/../file"),
        PathBuf::from(format!("{}/cache//file", root_path.display())),
        PathBuf::from(format!("{}/cache/file/", root_path.display())),
        root_path.parent().unwrap().to_path_buf(),
    ];

    for path in cases {
        let candidate = candidate(
            "candidate:invalid",
            "fixture.rule",
            &[path],
            1,
            Vec::new(),
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        );
        assert!(matches!(
            review_exact_paths(&scan_root, &[candidate], CleanupMode::DryRun),
            Err(ExactPathReviewError::PathLexical { .. }) | Err(ExactPathReviewError::Grouping(_))
        ));
    }
}

#[test]
fn blockers_non_cleanup_and_mode_mismatch_are_rejected_before_capture() {
    let (_directory, root_path, scan_root) = root();
    let target = root_path.join("cache/file");

    let blocked = candidate(
        "candidate:blocked",
        "fixture.rule",
        std::slice::from_ref(&target),
        1,
        vec![BlockReason::ProtectedPath],
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );
    assert!(matches!(
        review_exact_paths(&scan_root, &[blocked], CleanupMode::DryRun),
        Err(ExactPathReviewError::BlockedCandidate { .. })
    ));

    let informational = candidate(
        "candidate:info",
        "fixture.info",
        std::slice::from_ref(&target),
        1,
        Vec::new(),
        SafetyTier::Informational,
        CandidateAction::RevealOnly,
    );
    assert!(matches!(
        review_exact_paths(&scan_root, &[informational], CleanupMode::DryRun),
        Err(ExactPathReviewError::NonCleanupCandidate { .. })
    ));

    let mismatch = candidate(
        "candidate:mismatch",
        "fixture.rule",
        std::slice::from_ref(&target),
        1,
        Vec::new(),
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );
    assert!(matches!(
        review_exact_paths(&scan_root, &[mismatch], CleanupMode::Trash),
        Err(ExactPathReviewError::IncompatibleMode { .. })
    ));
}

#[test]
fn trusted_plan_requires_permanent_mode_and_one_authorization_per_path() {
    let (_directory, root_path, scan_root) = root();
    let target = root_path.join("cache/file");
    let candidate = candidate(
        "candidate:plan-boundary",
        "fixture.rule",
        std::slice::from_ref(&target),
        7,
        Vec::new(),
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );

    let dry_run = review_exact_paths(
        &scan_root,
        std::slice::from_ref(&candidate),
        CleanupMode::DryRun,
    )
    .unwrap();
    assert!(matches!(
        dry_run.into_trusted_permanent_plan(
            crate::domain::CleanupPlanId::new("plan:dry-run").unwrap(),
            UNIX_EPOCH,
            Vec::new(),
        ),
        Err(ExactPathPlanError::UnsupportedMode)
    ));

    let permanent = review_exact_paths(
        &scan_root,
        std::slice::from_ref(&candidate),
        CleanupMode::PermanentSafe,
    )
    .unwrap();
    assert!(matches!(
        permanent.into_trusted_permanent_plan(
            crate::domain::CleanupPlanId::new("plan:missing-grant").unwrap(),
            UNIX_EPOCH,
            Vec::new(),
        ),
        Err(ExactPathPlanError::AuthorizationCount {
            expected: 1,
            actual: 0,
        })
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn trusted_plan_consumes_matching_scope_authorization_and_revalidates() {
    let (_directory, root_path, scan_root) = root();
    let target = root_path.join("cache/file");
    let candidate = trusted_candidate(target);
    let review = review_exact_paths(
        &scan_root,
        std::slice::from_ref(&candidate),
        CleanupMode::PermanentSafe,
    )
    .unwrap();
    let authorization = authorize_rule_target(
        &scan_root,
        review.items()[0].paths()[0].snapshot().clone(),
        review.items()[0].rule(),
    )
    .unwrap();
    let plan = review
        .into_trusted_permanent_plan(
            crate::domain::CleanupPlanId::new("plan:trusted").unwrap(),
            SystemTime::now(),
            vec![authorization],
        )
        .unwrap();
    assert_eq!(plan.plan().mode(), CleanupMode::PermanentSafe);
    let approved = plan.approve(SystemTime::now()).unwrap();
    assert_eq!(approved.plan().mode(), CleanupMode::PermanentSafe);
    assert!(approved.approved_at() <= SystemTime::now());
    approved.revalidate(SystemTime::now()).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn trusted_plan_cannot_be_approved_after_expiration() {
    let (_directory, root_path, scan_root) = root();
    let candidate = trusted_candidate(root_path.join("cache/file"));
    let review = review_exact_paths(
        &scan_root,
        std::slice::from_ref(&candidate),
        CleanupMode::PermanentSafe,
    )
    .unwrap();
    let authorization = authorize_rule_target(
        &scan_root,
        review.items()[0].paths()[0].snapshot().clone(),
        review.items()[0].rule(),
    )
    .unwrap();
    let plan = review
        .into_trusted_permanent_plan(
            crate::domain::CleanupPlanId::new("plan:expired").unwrap(),
            UNIX_EPOCH,
            vec![authorization],
        )
        .unwrap();
    assert!(matches!(
        plan.approve(UNIX_EPOCH + crate::domain::CLEANUP_PLAN_VALIDITY),
        Err(ExactPathApprovalError::Expired)
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn exact_trusted_rust_target_plan_projects_to_a_fully_validated_dry_run() {
    let validation_time = SystemTime::now();
    let stale_mtime = validation_time
        .checked_sub(crate::domain::SAFE_RUST_RULE_MINIMUM_AGE + Duration::from_secs(60))
        .unwrap();
    let (_directory, root_path, target, reviewed) = trusted_rust_target_plan(true, stale_mtime);
    let expected_id = reviewed.plan().id().clone();
    let expected_items = reviewed.plan().items().to_vec();
    let expected_expiry = reviewed.effective_expires_at();
    let manifest_before = fs::read(root_path.join("project/Cargo.toml")).unwrap();
    let tag_before = fs::read(target.join("CACHEDIR.TAG")).unwrap();
    let artifact_before = fs::read(target.join("artifact.o")).unwrap();

    let dry_run = reviewed.into_rust_target_dry_run().unwrap();

    assert_eq!(dry_run.plan().id(), &expected_id);
    assert_eq!(dry_run.plan().items(), expected_items);
    assert_eq!(dry_run.plan().mode(), CleanupMode::DryRun);
    assert_eq!(dry_run.effective_expires_at(), expected_expiry);
    assert_eq!(
        dry_run.plan().warnings(),
        [
            PlanWarning::EstimatedBytesUnverified,
            PlanWarning::DryRunDoesNotMutate,
            PlanWarning::PermanentRemovalCannotBeUndone,
        ]
    );
    dry_run.validate_at(validation_time).unwrap();
    assert_eq!(
        fs::read(root_path.join("project/Cargo.toml")).unwrap(),
        manifest_before
    );
    assert_eq!(fs::read(target.join("CACHEDIR.TAG")).unwrap(), tag_before);
    assert_eq!(
        fs::read(target.join("artifact.o")).unwrap(),
        artifact_before
    );
    dry_run.release();
}

#[cfg(target_os = "macos")]
#[test]
fn generic_review_cannot_mint_the_rust_target_dry_run_authority() {
    let now = SystemTime::now();
    let stale_mtime = now
        .checked_sub(crate::domain::SAFE_RUST_RULE_MINIMUM_AGE + Duration::from_secs(60))
        .unwrap();
    let (_directory, _root_path, _target, reviewed) = trusted_rust_target_plan(false, stale_mtime);

    assert!(matches!(
        reviewed.into_rust_target_dry_run(),
        Err(ExactPathPlanError::AuthorizationMismatch)
    ));
}

#[cfg(target_os = "macos")]
#[test]
fn rust_target_dry_run_preserves_authority_expiry_and_rejects_live_marker_change() {
    let now = SystemTime::now();
    let stale_mtime = now
        .checked_sub(crate::domain::SAFE_RUST_RULE_MINIMUM_AGE + Duration::from_secs(60))
        .unwrap();
    let (_directory, _root_path, target, reviewed) = trusted_rust_target_plan(true, stale_mtime);
    let dry_run = reviewed.into_rust_target_dry_run().unwrap();
    fs::write(target.join("CACHEDIR.TAG"), b"changed marker").unwrap();
    let changed = dry_run.validate_at(now).unwrap_err();
    assert!(
        matches!(
            changed,
            RustTargetDryRunValidationError::Authorization(_)
                | RustTargetDryRunValidationError::RuleEvidence(_)
        ),
        "{changed:?}"
    );
    dry_run.release();

    let (_directory, _root_path, _target, mut reviewed) =
        trusted_rust_target_plan(true, stale_mtime);
    reviewed.authority_expires_at = now;
    let expired = reviewed.into_rust_target_dry_run().unwrap();
    assert_eq!(expired.effective_expires_at(), now);
    assert!(matches!(
        expired.validate_at(now),
        Err(RustTargetDryRunValidationError::Expired)
    ));
    expired.release();
}

#[cfg(target_os = "macos")]
#[test]
fn rust_target_dry_run_and_permanent_validation_share_recency_rejection() {
    let now = SystemTime::now();
    let (_directory, _root_path, _target, reviewed) = trusted_rust_target_plan(true, now);
    assert!(matches!(
        validate_rust_target_effect_plan(
            reviewed.plan(),
            &reviewed.authorizations[0],
            reviewed.effective_expires_at(),
            now,
        ),
        Err(RustTargetPlanObservationError::RuleEvidence(
            RustTargetLiveValidationError::RecentActivity
        ))
    ));
    let dry_run = reviewed.into_rust_target_dry_run().unwrap();

    assert!(matches!(
        dry_run.validate_at(now),
        Err(RustTargetDryRunValidationError::RuleEvidence(
            RustTargetLiveValidationError::RecentActivity
        ))
    ));
    dry_run.release();
}

#[test]
fn unresolved_overlap_missing_target_and_estimate_overflow_fail_closed() {
    let (_directory, root_path, scan_root) = root();
    let parent = root_path.join("cache");
    let child = parent.join("file");
    let overlapping = vec![
        candidate(
            "candidate:parent",
            "fixture.parent",
            std::slice::from_ref(&parent),
            10,
            Vec::new(),
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        ),
        candidate(
            "candidate:child",
            "fixture.child",
            std::slice::from_ref(&child),
            3,
            Vec::new(),
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        ),
    ];
    assert!(matches!(
        review_exact_paths(&scan_root, &overlapping, CleanupMode::DryRun),
        Err(ExactPathReviewError::UnresolvedOverlap { .. })
    ));

    let missing = candidate(
        "candidate:missing",
        "fixture.rule",
        &[root_path.join("cache/nope")],
        1,
        Vec::new(),
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );
    assert!(matches!(
        review_exact_paths(&scan_root, &[missing], CleanupMode::DryRun),
        Err(ExactPathReviewError::PathLive { .. })
    ));

    let overflow = vec![
        candidate(
            "candidate:large",
            "fixture.large",
            std::slice::from_ref(&child),
            u64::MAX,
            Vec::new(),
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        ),
        candidate(
            "candidate:small",
            "fixture.small",
            std::slice::from_ref(&parent),
            1,
            Vec::new(),
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        ),
    ];
    let overflow_result = review_exact_paths(&scan_root, &overflow, CleanupMode::DryRun);
    assert!(matches!(
        overflow_result,
        Err(ExactPathReviewError::UnresolvedOverlap { .. })
            | Err(ExactPathReviewError::EstimatedBytesOverflow)
            | Err(ExactPathReviewError::Grouping(
                CandidateGroupingError::EstimatedBytesOverflow { .. }
            ))
    ));
}

#[cfg(unix)]
#[test]
fn symlink_target_is_rejected_without_mutation() {
    let (_directory, root_path, scan_root) = root();
    std::os::unix::fs::symlink(root_path.join("cache/file"), root_path.join("cache/link")).unwrap();
    let target = root_path.join("cache/link");
    let candidate = candidate(
        "candidate:symlink",
        "fixture.rule",
        std::slice::from_ref(&target),
        1,
        Vec::new(),
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );

    assert!(matches!(
        review_exact_paths(&scan_root, &[candidate], CleanupMode::DryRun),
        Err(ExactPathReviewError::PathLive { .. })
    ));
    assert!(target.exists());
    assert!(root_path.join("cache/file").exists());
}

#[cfg(unix)]
#[test]
fn multiply_linked_permanent_file_is_rejected() {
    let (_directory, root_path, scan_root) = root();
    let first = root_path.join("cache/file");
    let second = root_path.join("cache/second");
    fs::hard_link(&first, &second).unwrap();
    let candidate = candidate(
        "candidate:hard-link",
        "fixture.rule",
        std::slice::from_ref(&first),
        7,
        Vec::new(),
        SafetyTier::SafeRegenerable,
        CandidateAction::RemoveKnownRegenerableContents,
    );

    assert!(matches!(
        review_exact_paths(&scan_root, &[candidate], CleanupMode::DryRun),
        Err(ExactPathReviewError::MultiplyLinkedRegularFile {
            hard_link_count: 2,
            ..
        })
    ));
    assert!(first.exists());
    assert!(second.exists());
}
