use std::time::{Duration, UNIX_EPOCH};

use super::super::candidate::CandidateInput;
use super::super::{
    BlockReason, Candidate, CandidateAction, CandidateCategory, CandidateId, Evidence,
    LocalizedTextKey, ProvenanceUrl, Rule, RuleDefinition, RuleGuards, RuleId, RuleMatcher,
    RuleMatcherDefinition, RuleRef, RuleRevision, RuleScope, SafetyTier, ScanId,
};
use super::*;

fn rule(id: &str) -> Rule {
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
        safety: SafetyTier::SafeRegenerable,
        action: CandidateAction::RemoveKnownRegenerableContents,
        schedule_eligible: false,
        explanation_key: LocalizedTextKey::new(format!("fixture.{id}.explanation")).unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/candidate-groups").unwrap()],
    })
    .unwrap()
}

fn candidate(
    id: &str,
    rule_id: &str,
    paths: &[&str],
    estimated_bytes: u64,
    blockers: Vec<BlockReason>,
) -> Candidate {
    Candidate::try_from_rule(
        &rule(rule_id),
        CandidateInput::new(
            CandidateId::new(id).unwrap(),
            paths.iter().map(|path| (*path).into()).collect(),
            estimated_bytes,
            Some(UNIX_EPOCH + Duration::from_secs(7)),
            vec![Evidence::RequiredMarker {
                path: "/fixture/Cargo.toml".into(),
            }],
            blockers,
            ScanId::new("scan:groups").unwrap(),
        ),
    )
    .unwrap()
}

fn selected_ids<'a>(set: &'a CandidateGroupSet, candidates: &'a [Candidate]) -> Vec<&'a str> {
    set.groups()
        .iter()
        .flat_map(|group| group.selected_indices())
        .map(|index| candidates[*index].id().as_str())
        .collect()
}

#[test]
fn exact_duplicate_same_rule_is_coalesced_by_candidate_id() {
    let candidates = vec![
        candidate(
            "candidate:z",
            "fixture.rule",
            &["/fixture/cache"],
            10,
            Vec::new(),
        ),
        candidate(
            "candidate:a",
            "fixture.rule",
            &["/fixture/cache"],
            10,
            Vec::new(),
        ),
    ];

    let set = group_candidates(&candidates).unwrap();

    assert_eq!(set.source_scan_id().as_str(), "scan:groups");
    assert_eq!(
        set.groups()[0].key().category,
        CandidateCategory::DeveloperArtifact
    );
    assert_eq!(set.groups()[0].member_indices(), &[1, 0]);
    assert_eq!(selected_ids(&set, &candidates), vec!["candidate:a"]);
    assert_eq!(set.groups()[0].estimated_bytes(), 10);
    assert_eq!(set.groups()[0].actionable_bytes(), 10);
    assert!(matches!(
        set.overlap_decisions()[0].resolution(),
        CandidateOverlapResolution::Coalesced {
            retained_index: 1,
            suppressed_index: 0
        }
    ));
    assert_eq!(set.overlap_decisions()[0].first_index(), 0);
    assert_eq!(set.overlap_decisions()[0].second_index(), 1);
}

#[test]
fn same_rule_parent_owns_child_in_either_input_order() {
    let parent = candidate(
        "candidate:parent",
        "fixture.rule",
        &["/fixture/cache"],
        100,
        Vec::new(),
    );
    let child = candidate(
        "candidate:child",
        "fixture.rule",
        &["/fixture/cache/child"],
        20,
        Vec::new(),
    );

    for candidates in [vec![parent.clone(), child.clone()], vec![child, parent]] {
        let set = group_candidates(&candidates).unwrap();
        assert_eq!(selected_ids(&set, &candidates), vec!["candidate:parent"]);
        assert!(!set.has_unresolved_overlaps());
    }
}

#[test]
fn mixed_rule_parent_child_overlap_remains_unresolved() {
    let candidates = vec![
        candidate(
            "candidate:parent",
            "fixture.parent",
            &["/fixture/cache"],
            100,
            Vec::new(),
        ),
        candidate(
            "candidate:child",
            "fixture.child",
            &["/fixture/cache/child"],
            20,
            Vec::new(),
        ),
    ];

    let set = group_candidates(&candidates).unwrap();

    assert!(set.has_unresolved_overlaps());
    assert!(matches!(
        set.overlap_decisions()[0].resolution(),
        CandidateOverlapResolution::Unresolved(CandidateOverlapReason::DifferentRule)
    ));
    assert_eq!(set.groups()[0].actionable_bytes(), 0);
}

#[test]
fn blocked_overlap_is_unresolved_and_not_actionable() {
    let candidates = vec![
        candidate(
            "candidate:parent",
            "fixture.rule",
            &["/fixture/cache"],
            100,
            Vec::new(),
        ),
        candidate(
            "candidate:blocked-child",
            "fixture.rule",
            &["/fixture/cache/child"],
            20,
            vec![BlockReason::ProtectedPath],
        ),
    ];

    let set = group_candidates(&candidates).unwrap();

    assert!(set.has_unresolved_overlaps());
    assert_eq!(set.groups()[0].blocked_member_count(), 1);
    assert_eq!(set.groups()[0].actionable_bytes(), 0);
}

#[test]
fn internal_candidate_overlap_is_unresolved() {
    let candidates = vec![candidate(
        "candidate:internal",
        "fixture.rule",
        &["/fixture/cache", "/fixture/cache/child"],
        100,
        Vec::new(),
    )];

    let set = group_candidates(&candidates).unwrap();

    assert!(set.has_unresolved_overlaps());
    assert!(set.overlap_decisions().iter().any(|decision| matches!(
        decision.resolution(),
        CandidateOverlapResolution::Unresolved(CandidateOverlapReason::InternalCandidateOverlap)
    )));
}

#[test]
fn component_prefix_siblings_are_disjoint_and_group_order_is_stable() {
    let candidates = vec![
        candidate(
            "candidate:z",
            "fixture.rule",
            &["/fixture/cache-archive"],
            30,
            Vec::new(),
        ),
        candidate(
            "candidate:a",
            "fixture.rule",
            &["/fixture/cache"],
            20,
            Vec::new(),
        ),
    ];

    let set = group_candidates(&candidates).unwrap();

    assert!(!set.has_unresolved_overlaps());
    assert!(set.overlap_decisions().is_empty());
    assert_eq!(
        selected_ids(&set, &candidates),
        vec!["candidate:a", "candidate:z"]
    );
}

#[test]
fn duplicate_ids_mixed_scans_and_relative_paths_fail_closed() {
    let duplicate = vec![
        candidate(
            "candidate:same",
            "fixture.rule",
            &["/fixture/one"],
            1,
            Vec::new(),
        ),
        candidate(
            "candidate:same",
            "fixture.rule",
            &["/fixture/two"],
            1,
            Vec::new(),
        ),
    ];
    assert!(matches!(
        group_candidates(&duplicate),
        Err(CandidateGroupingError::DuplicateCandidateId { .. })
    ));

    let mut relative = candidate(
        "candidate:relative",
        "fixture.rule",
        &["relative"],
        1,
        Vec::new(),
    );
    assert!(matches!(
        group_candidates(&[relative.clone()]),
        Err(CandidateGroupingError::InvalidPath { .. })
    ));
    relative = candidate("candidate:root", "fixture.rule", &["/"], 1, Vec::new());
    assert!(matches!(
        group_candidates(&[relative]),
        Err(CandidateGroupingError::InvalidPath { .. })
    ));
    relative = candidate(
        "candidate:relative",
        "fixture.rule",
        &["/fixture/one/../two"],
        1,
        Vec::new(),
    );
    assert!(matches!(
        group_candidates(&[relative]),
        Err(CandidateGroupingError::InvalidPath { .. })
    ));
}

#[test]
fn checked_group_totals_fail_closed_on_overflow() {
    let candidates = vec![
        candidate(
            "candidate:large",
            "fixture.rule",
            &["/fixture/large"],
            u64::MAX,
            Vec::new(),
        ),
        candidate(
            "candidate:small",
            "fixture.rule",
            &["/fixture/small"],
            1,
            Vec::new(),
        ),
    ];

    assert!(matches!(
        group_candidates(&candidates),
        Err(CandidateGroupingError::EstimatedBytesOverflow { .. })
    ));
}
