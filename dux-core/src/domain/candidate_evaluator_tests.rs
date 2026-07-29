use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::*;
use crate::domain::{
    CleanupMode, CleanupPlan, CleanupPlanId, CleanupPlanValidationError, ScanIssue, ScanIssueKind,
};
use crate::tree::{NodeId, NodeKind};

fn complete_coverage() -> ScanCoverage {
    ScanCoverage::from_validated_terminal_issues(Vec::new())
}

fn evaluated_at() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(30 * 86_400)
}

fn add_file(tree: &mut DiskTree, parent: NodeId, name: &str, path: PathBuf) -> NodeId {
    tree.add_node(name.to_owned(), NodeKind::File, path, parent)
}

fn add_rust_project(tree: &mut DiskTree, project_name: &str, bytes: u64) -> PathBuf {
    let project_path = tree.root_path().join(project_name);
    let project = tree.add_node(
        project_name.to_owned(),
        NodeKind::Directory,
        project_path.clone(),
        NodeId::ROOT,
    );
    add_file(tree, project, "Cargo.toml", project_path.join("Cargo.toml"));
    let target_path = project_path.join("target");
    let target = tree.add_node(
        "target".to_owned(),
        NodeKind::Directory,
        target_path.clone(),
        project,
    );
    let cache_tag = add_file(
        tree,
        target,
        "CACHEDIR.TAG",
        target_path.join("CACHEDIR.TAG"),
    );
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(86_400);
    tree.get_mut(target).unwrap().mtime = Some(old);
    tree.get_mut(cache_tag).unwrap().mtime = Some(old);
    tree.set_size(target, bytes);
    target_path
}

fn add_python_pycache_project(tree: &mut DiskTree, project_name: &str, bytes: u64) -> PathBuf {
    let project_path = tree.root_path().join(project_name);
    let project = tree.add_node(
        project_name.to_owned(),
        NodeKind::Directory,
        project_path.clone(),
        NodeId::ROOT,
    );
    add_file(tree, project, "module.py", project_path.join("module.py"));
    let cache_path = project_path.join("__pycache__");
    let cache = tree.add_node(
        "__pycache__".to_owned(),
        NodeKind::Directory,
        cache_path.clone(),
        project,
    );
    add_file(
        tree,
        cache,
        "module.cpython-314.pyc",
        cache_path.join("module.cpython-314.pyc"),
    );
    tree.set_size(cache, bytes);
    cache_path
}

fn evaluate(tree: &DiskTree) -> CandidateBatch {
    evaluate_artifact_candidates(
        &ScanId::new("scan:fixture").unwrap(),
        tree,
        &complete_coverage(),
        evaluated_at(),
    )
    .unwrap()
}

#[test]
fn marker_verified_artifact_preserves_paths_size_evidence_and_scan_binding() {
    let mut tree = DiskTree::new(PathBuf::from("/fixture"));
    let target_path = add_rust_project(&mut tree, "project", 4096);

    let batch = evaluate(&tree);
    assert_eq!(batch.observed_match_count(), 1);
    let candidate = &batch.candidates()[0];
    assert_eq!(candidate.paths(), std::slice::from_ref(&target_path));
    assert_eq!(candidate.estimated_bytes(), 4096);
    assert_eq!(candidate.source_scan_id().as_str(), "scan:fixture");
    assert_eq!(candidate.rule().id().as_str(), "developer.rust.target");
    assert_eq!(candidate.rule().revision().get(), 3);
    assert_eq!(candidate.safety(), SafetyTier::SafeRegenerable);
    assert_eq!(
        candidate.action(),
        CandidateAction::RemoveKnownRegenerableContents
    );
    assert!(!candidate.rule_marks_schedule_eligible());
    assert_eq!(
        candidate.evidence(),
        &[
            Evidence::MatchedPath {
                path: target_path.clone(),
            },
            Evidence::RequiredMarker {
                path: PathBuf::from("/fixture/project/Cargo.toml"),
            },
            Evidence::RequiredMarker {
                path: PathBuf::from("/fixture/project/target/CACHEDIR.TAG"),
            },
            Evidence::MinimumAge {
                newest_mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(86_400),
                minimum_age: SAFE_RUST_RULE_MINIMUM_AGE,
            },
        ]
    );
    assert_eq!(candidate.blockers(), &[BlockReason::ProtectedPath]);
}

#[test]
fn ambiguous_name_without_required_marker_produces_an_empty_batch() {
    let mut tree = DiskTree::new(PathBuf::from("/fixture"));
    let target = tree.add_node(
        "target".to_owned(),
        NodeKind::Directory,
        PathBuf::from("/fixture/target"),
        NodeId::ROOT,
    );
    tree.set_size(target, 100);

    let batch = evaluate(&tree);
    assert_eq!(batch.observed_match_count(), 0);
    assert!(batch.candidates().is_empty());
}

#[test]
fn rust_target_requires_a_regular_non_symlink_cargo_cache_tag() {
    let mut tree = DiskTree::new(PathBuf::from("/fixture"));
    let project_path = tree.root_path().join("project");
    let project = tree.add_node(
        "project".to_owned(),
        NodeKind::Directory,
        project_path.clone(),
        NodeId::ROOT,
    );
    add_file(
        &mut tree,
        project,
        "Cargo.toml",
        project_path.join("Cargo.toml"),
    );
    let target_path = project_path.join("target");
    let target = tree.add_node(
        "target".to_owned(),
        NodeKind::Directory,
        target_path.clone(),
        project,
    );
    let tag = add_file(
        &mut tree,
        target,
        "CACHEDIR.TAG",
        target_path.join("CACHEDIR.TAG"),
    );
    tree.get_mut(tag).unwrap().path_is_symlink = true;

    assert!(evaluate(&tree).candidates().is_empty());
}

#[test]
fn nested_classified_artifacts_emit_only_the_outer_candidate() {
    let mut tree = DiskTree::new(PathBuf::from("/fixture"));
    let project = tree.add_node(
        "project".to_owned(),
        NodeKind::Directory,
        PathBuf::from("/fixture/project"),
        NodeId::ROOT,
    );
    add_file(
        &mut tree,
        project,
        "Cargo.toml",
        PathBuf::from("/fixture/project/Cargo.toml"),
    );
    let outer = tree.add_node(
        "target".to_owned(),
        NodeKind::Directory,
        PathBuf::from("/fixture/project/target"),
        project,
    );
    add_file(
        &mut tree,
        outer,
        "CACHEDIR.TAG",
        PathBuf::from("/fixture/project/target/CACHEDIR.TAG"),
    );
    let nested_project = tree.add_node(
        "nested".to_owned(),
        NodeKind::Directory,
        PathBuf::from("/fixture/project/target/nested"),
        outer,
    );
    add_file(
        &mut tree,
        nested_project,
        "Cargo.toml",
        PathBuf::from("/fixture/project/target/nested/Cargo.toml"),
    );
    let nested_target = tree.add_node(
        "target".to_owned(),
        NodeKind::Directory,
        PathBuf::from("/fixture/project/target/nested/target"),
        nested_project,
    );
    add_file(
        &mut tree,
        nested_target,
        "CACHEDIR.TAG",
        PathBuf::from("/fixture/project/target/nested/target/CACHEDIR.TAG"),
    );

    let batch = evaluate(&tree);
    assert_eq!(batch.candidates().len(), 1);
    assert_eq!(
        batch.candidates()[0].paths(),
        &[PathBuf::from("/fixture/project/target")]
    );
}

#[test]
fn ids_and_order_are_independent_of_arena_insertion_order_and_bound_to_scan() {
    let make = |names: &[&str]| {
        let mut tree = DiskTree::new(PathBuf::from("/fixture"));
        for name in names {
            add_rust_project(&mut tree, name, 10);
        }
        tree
    };
    let forward = make(&["a", "b"]);
    let reverse = make(&["b", "a"]);
    let facts = |batch: CandidateBatch| {
        batch
            .into_candidates()
            .into_iter()
            .map(|candidate| {
                (
                    candidate.paths()[0].clone(),
                    candidate.id().as_str().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(facts(evaluate(&forward)), facts(evaluate(&reverse)));

    let other_scan = evaluate_artifact_candidates(
        &ScanId::new("scan:other").unwrap(),
        &forward,
        &complete_coverage(),
        evaluated_at(),
    )
    .unwrap();
    assert_ne!(
        evaluate(&forward).candidates()[0].id(),
        other_scan.candidates()[0].id()
    );
}

#[test]
fn evidence_selection_is_deterministic_when_multiple_markers_match() {
    let make = |reverse_markers: bool| {
        let mut tree = DiskTree::new(PathBuf::from("/fixture"));
        let fixtures = [
            (
                "gradle",
                "build",
                &["build.gradle", "settings.gradle.kts"][..],
            ),
            (
                "next",
                ".next",
                &["package.json", "next.config.js", "next.config.ts"][..],
            ),
            (
                "nuxt",
                ".nuxt",
                &["package.json", "nuxt.config.mjs", "nuxt.config.ts"][..],
            ),
            ("python", "__pycache__", &["a.py", "z.py"][..]),
        ];
        for (project_name, artifact_name, markers) in fixtures {
            let project_path = tree.root_path().join(project_name);
            let project = tree.add_node(
                project_name.to_owned(),
                NodeKind::Directory,
                project_path.clone(),
                NodeId::ROOT,
            );
            let marker_order: Box<dyn Iterator<Item = &&str>> = if reverse_markers {
                Box::new(markers.iter().rev())
            } else {
                Box::new(markers.iter())
            };
            for marker in marker_order {
                add_file(&mut tree, project, marker, project_path.join(marker));
            }
            tree.add_node(
                artifact_name.to_owned(),
                NodeKind::Directory,
                project_path.join(artifact_name),
                project,
            );
        }
        tree
    };

    let forward = evaluate(&make(false));
    let reverse = evaluate(&make(true));
    assert_eq!(forward.candidates(), reverse.candidates());
    let marker_paths = forward
        .candidates()
        .iter()
        .flat_map(|candidate| candidate.evidence())
        .filter_map(|evidence| match evidence {
            Evidence::RequiredMarker { path } => Some(path.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(marker_paths.contains(&PathBuf::from("/fixture/gradle/build.gradle")));
    assert!(marker_paths.contains(&PathBuf::from("/fixture/next/next.config.js")));
    assert!(marker_paths.contains(&PathBuf::from("/fixture/nuxt/nuxt.config.mjs")));
    assert!(marker_paths.contains(&PathBuf::from("/fixture/python/a.py")));
    assert!(forward.candidates().iter().all(|candidate| {
        (if candidate.rule().id().as_str() == SAFE_PYTHON_PYCACHE_RULE_ID {
            candidate.safety() == SafetyTier::SafeRegenerable
                && candidate.action() == CandidateAction::RemoveKnownRegenerableContents
        } else {
            candidate.safety() == SafetyTier::Informational
                && candidate.action() == CandidateAction::RevealOnly
        }) && !candidate.rule_marks_schedule_eligible()
    }));
}

#[test]
fn python_pycache_policy_is_safe_but_remains_blocked_and_unschedulable() {
    let mut tree = DiskTree::new(PathBuf::from("/fixture"));
    let cache_path = add_python_pycache_project(&mut tree, "project", 4096);

    let batch = evaluate(&tree);
    assert_eq!(batch.candidates().len(), 1);
    let candidate = &batch.candidates()[0];
    assert_eq!(candidate.rule().id().as_str(), SAFE_PYTHON_PYCACHE_RULE_ID);
    assert_eq!(candidate.rule().revision().get(), 2);
    assert_eq!(candidate.paths(), &[cache_path]);
    assert_eq!(candidate.estimated_bytes(), 4096);
    assert_eq!(candidate.safety(), SafetyTier::SafeRegenerable);
    assert_eq!(
        candidate.action(),
        CandidateAction::RemoveKnownRegenerableContents
    );
    assert!(!candidate.rule_marks_schedule_eligible());
    assert_eq!(candidate.blockers(), &[BlockReason::ProtectedPath]);
    assert_eq!(
        CleanupPlan::try_from_candidates_for_persistence_test(
            CleanupPlanId::new("plan:blocked-python-pycache").unwrap(),
            SystemTime::UNIX_EPOCH,
            CleanupMode::PermanentSafe,
            std::slice::from_ref(candidate),
        ),
        Err(CleanupPlanValidationError::BlockedCandidate { candidate_index: 0 })
    );
}

#[test]
fn python_pycache_requires_a_case_sensitive_regular_python_source_marker() {
    let mut wrong_case = DiskTree::new(PathBuf::from("/fixture"));
    let project = wrong_case.add_node(
        "project".to_owned(),
        NodeKind::Directory,
        PathBuf::from("/fixture/project"),
        NodeId::ROOT,
    );
    add_file(
        &mut wrong_case,
        project,
        "module.PY",
        PathBuf::from("/fixture/project/module.PY"),
    );
    wrong_case.add_node(
        "__pycache__".to_owned(),
        NodeKind::Directory,
        PathBuf::from("/fixture/project/__pycache__"),
        project,
    );
    assert!(evaluate(&wrong_case).candidates().is_empty());

    let mut symlink_marker = DiskTree::new(PathBuf::from("/fixture"));
    let project = symlink_marker.add_node(
        "project".to_owned(),
        NodeKind::Directory,
        PathBuf::from("/fixture/project"),
        NodeId::ROOT,
    );
    let marker = add_file(
        &mut symlink_marker,
        project,
        "module.py",
        PathBuf::from("/fixture/project/module.py"),
    );
    symlink_marker.get_mut(marker).unwrap().path_is_symlink = true;
    symlink_marker.add_node(
        "__pycache__".to_owned(),
        NodeKind::Directory,
        PathBuf::from("/fixture/project/__pycache__"),
        project,
    );
    assert!(evaluate(&symlink_marker).candidates().is_empty());
}

#[cfg(unix)]
#[test]
fn python_evidence_order_uses_lossless_native_paths() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let make = |reverse: bool| {
        let mut tree = DiskTree::new(PathBuf::from("/fixture"));
        let project_path = tree.root_path().join("python");
        let project = tree.add_node(
            "python".to_owned(),
            NodeKind::Directory,
            project_path.clone(),
            NodeId::ROOT,
        );
        let native_names = if reverse {
            [b"\xff.py".as_slice(), b"\xfe.py".as_slice()]
        } else {
            [b"\xfe.py".as_slice(), b"\xff.py".as_slice()]
        };
        for native_name in native_names {
            add_file(
                &mut tree,
                project,
                "�.py",
                project_path.join(OsStr::from_bytes(native_name)),
            );
        }
        tree.add_node(
            "__pycache__".to_owned(),
            NodeKind::Directory,
            project_path.join("__pycache__"),
            project,
        );
        tree
    };

    let forward = evaluate(&make(false));
    let reverse = evaluate(&make(true));
    assert_eq!(forward.candidates(), reverse.candidates());
    assert!(forward.candidates()[0].evidence().iter().any(|evidence| {
        matches!(
            evidence,
            Evidence::RequiredMarker { path }
                if path.file_name() == Some(OsStr::from_bytes(b"\xfe.py"))
        )
    }));
}

#[test]
fn exact_catalog_digest_is_stable() {
    let mut actual = String::new();
    push_lower_hex(&mut actual, &bundled_candidate_catalog_digest_sha256());
    assert_eq!(
        actual,
        "c0c4544d6c2c3d96ebc356425ee99b7698d631411282125421267e766759f09e"
    );
    let catalog = load_and_validate_catalog().unwrap();
    assert!(
        catalog
            .iter()
            .all(|rule| rule.scope() == RuleScope::SelectedScanRoot)
    );
    let safe_rules = catalog
        .iter()
        .filter(|rule| rule.safety() == SafetyTier::SafeRegenerable)
        .collect::<Vec<_>>();
    assert_eq!(safe_rules.len(), 2);
    assert!(safe_rules.iter().any(|rule| {
        rule.reference().id().as_str() == SAFE_RUST_RULE_ID
            && rule.reference().revision().get() == SAFE_RUST_RULE_REVISION
            && rule.guards().minimum_age() == Some(SAFE_RUST_RULE_MINIMUM_AGE)
    }));
    assert!(safe_rules.iter().any(|rule| {
        rule.reference().id().as_str() == SAFE_PYTHON_PYCACHE_RULE_ID
            && rule.reference().revision().get() == 2
            && rule.guards().minimum_age().is_none()
    }));
    assert!(safe_rules.iter().all(|rule| !rule.schedule_eligible()));
    assert!(
        safe_rules
            .iter()
            .any(|rule| { rule.reference().id().as_str() == SAFE_RUST_RULE_ID })
    );
    assert!(
        safe_rules
            .iter()
            .any(|rule| { rule.reference().id().as_str() == SAFE_PYTHON_PYCACHE_RULE_ID })
    );
    assert_eq!(
        evaluate(&DiskTree::new(PathBuf::from("/fixture"))).catalog_digest_sha256(),
        bundled_candidate_catalog_digest_sha256()
    );
    let empty = evaluate(&DiskTree::new(PathBuf::from("/fixture")));
    assert_eq!(empty.evaluator_revision(), CANDIDATE_EVALUATOR_REVISION);
    assert_eq!(
        empty.catalog_schema_version(),
        CANDIDATE_CATALOG_SCHEMA_VERSION
    );
    assert_eq!(
        empty.context_format_version(),
        CANDIDATE_CONTEXT_FORMAT_VERSION
    );
}

#[test]
fn incomplete_coverage_adds_a_distinct_blocker() {
    let mut tree = DiskTree::new(PathBuf::from("/fixture"));
    add_rust_project(&mut tree, "project", 10);
    let issue = ScanIssue::try_new(
        ScanIssueKind::MetadataError,
        Some(PathBuf::from("/fixture/project")),
        1,
    )
    .unwrap();
    let coverage = ScanCoverage::try_new(ScanCoverageStatus::Partial, None, vec![issue]).unwrap();

    let batch = evaluate_artifact_candidates(
        &ScanId::new("scan:partial").unwrap(),
        &tree,
        &coverage,
        evaluated_at(),
    )
    .unwrap();
    assert_eq!(
        batch.candidates()[0].blockers(),
        &[BlockReason::PartialScanCoverage, BlockReason::ProtectedPath,]
    );
}

#[test]
fn evaluation_context_is_stable_and_bound_to_scan_root_and_coverage() {
    let empty = DiskTree::new(PathBuf::from("/fixture"));
    let first = evaluate_artifact_candidates(
        &ScanId::new("scan:context").unwrap(),
        &empty,
        &complete_coverage(),
        evaluated_at(),
    )
    .unwrap();
    let repeated = evaluate_artifact_candidates(
        &ScanId::new("scan:context").unwrap(),
        &empty,
        &complete_coverage(),
        evaluated_at(),
    )
    .unwrap();
    assert_eq!(
        first.context_digest_sha256(),
        repeated.context_digest_sha256()
    );

    let different_root = evaluate_artifact_candidates(
        &ScanId::new("scan:context").unwrap(),
        &DiskTree::new(PathBuf::from("/other")),
        &complete_coverage(),
        evaluated_at(),
    )
    .unwrap();
    let unknown_coverage = evaluate_artifact_candidates(
        &ScanId::new("scan:context").unwrap(),
        &empty,
        &ScanCoverage::unknown(),
        evaluated_at(),
    )
    .unwrap();
    assert_ne!(
        first.context_digest_sha256(),
        different_root.context_digest_sha256()
    );
    assert_ne!(
        first.context_digest_sha256(),
        unknown_coverage.context_digest_sha256()
    );
    let different_evaluation_time = evaluate_artifact_candidates(
        &ScanId::new("scan:context").unwrap(),
        &empty,
        &complete_coverage(),
        evaluated_at() + Duration::from_nanos(1),
    )
    .unwrap();
    assert_ne!(
        first.context_digest_sha256(),
        different_evaluation_time.context_digest_sha256()
    );
}

#[test]
fn rust_target_minimum_age_is_inclusive_and_fails_closed() {
    let now = evaluated_at();
    let cases = [
        (
            "older",
            Some(now - SAFE_RUST_RULE_MINIMUM_AGE - Duration::from_nanos(1)),
            None,
            None,
        ),
        ("exact", Some(now - SAFE_RUST_RULE_MINIMUM_AGE), None, None),
        (
            "young",
            Some(now - SAFE_RUST_RULE_MINIMUM_AGE + Duration::from_nanos(1)),
            None,
            Some(BlockReason::RecentActivity),
        ),
        (
            "future",
            Some(now + Duration::from_nanos(1)),
            None,
            Some(BlockReason::RecentActivity),
        ),
        (
            "missing-directory",
            Some(now - SAFE_RUST_RULE_MINIMUM_AGE),
            Some("directory"),
            Some(BlockReason::MissingModificationTime),
        ),
        (
            "missing-file",
            Some(now - SAFE_RUST_RULE_MINIMUM_AGE),
            Some("file"),
            Some(BlockReason::MissingModificationTime),
        ),
    ];

    for (name, newest_mtime, missing_mtime, recency_blocker) in cases {
        let mut tree = DiskTree::new(PathBuf::from("/fixture"));
        let target_path = add_rust_project(&mut tree, name, 4096);
        let target = tree
            .iter()
            .find(|node| node.path == target_path)
            .map(|node| node.id)
            .unwrap();
        let payload = add_file(
            &mut tree,
            target,
            "artifact.rlib",
            target_path.join("artifact.rlib"),
        );
        tree.get_mut(target).unwrap().mtime = newest_mtime;
        for child in tree.get(target).unwrap().children.clone() {
            tree.get_mut(child).unwrap().mtime = newest_mtime;
        }
        tree.get_mut(payload).unwrap().mtime = newest_mtime;
        match missing_mtime {
            Some("directory") => tree.get_mut(target).unwrap().mtime = None,
            Some("file") => tree.get_mut(payload).unwrap().mtime = None,
            None => {}
            Some(_) => unreachable!(),
        }

        let batch = evaluate_artifact_candidates(
            &ScanId::new(format!("scan:{name}")).unwrap(),
            &tree,
            &complete_coverage(),
            now,
        )
        .unwrap();
        let candidate = &batch.candidates()[0];
        let recency_satisfied = recency_blocker.is_none();
        let expected_blockers = match recency_blocker {
            Some(blocker) => vec![blocker, BlockReason::ProtectedPath],
            None => vec![BlockReason::ProtectedPath],
        };
        assert_eq!(candidate.blockers(), expected_blockers, "{name}");
        let minimum_age_evidence = candidate
            .evidence()
            .iter()
            .filter(|evidence| matches!(evidence, Evidence::MinimumAge { .. }))
            .count();
        assert_eq!(
            minimum_age_evidence,
            usize::from(recency_satisfied),
            "{name}"
        );
        if recency_satisfied {
            assert_eq!(candidate.newest_mtime(), newest_mtime, "{name}");
            assert!(
                candidate.evidence().iter().any(|evidence| {
                    matches!(
                        evidence,
                        Evidence::MinimumAge {
                            newest_mtime: observed,
                            minimum_age,
                        } if Some(*observed) == newest_mtime
                            && *minimum_age == SAFE_RUST_RULE_MINIMUM_AGE
                    )
                }),
                "{name}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn native_non_utf8_parent_bytes_are_preserved_in_path_and_candidate_id() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let root = PathBuf::from("/fixture");
    let project_path = root.join(OsStr::from_bytes(b"project-\xff"));
    let mut tree = DiskTree::new(root);
    let project = tree.add_node(
        "project-lossy".to_owned(),
        NodeKind::Directory,
        project_path.clone(),
        NodeId::ROOT,
    );
    add_file(
        &mut tree,
        project,
        "Cargo.toml",
        project_path.join("Cargo.toml"),
    );
    let target_path = project_path.join("target");
    let target = tree.add_node(
        "target".to_owned(),
        NodeKind::Directory,
        target_path.clone(),
        project,
    );
    add_file(
        &mut tree,
        target,
        "CACHEDIR.TAG",
        target_path.join("CACHEDIR.TAG"),
    );

    let first = evaluate(&tree);
    let second = evaluate(&tree);
    assert_eq!(first.candidates()[0].paths(), &[target_path]);
    assert_eq!(first.candidates()[0].id(), second.candidates()[0].id());
}

#[test]
fn safe_rust_policy_remains_blocked_unschedulable_and_unplannable() {
    let mut tree = DiskTree::new(PathBuf::from("/fixture"));
    add_rust_project(&mut tree, "project", 10);

    let batch = evaluate(&tree);
    let candidate = &batch.candidates()[0];
    assert_eq!(candidate.safety(), SafetyTier::SafeRegenerable);
    assert_eq!(
        candidate.action(),
        CandidateAction::RemoveKnownRegenerableContents
    );
    assert!(candidate.has_cleanup_operation());
    assert!(!candidate.rule_marks_schedule_eligible());
    assert_eq!(candidate.blockers(), &[BlockReason::ProtectedPath]);
    assert_eq!(
        CleanupPlan::try_from_candidates_for_persistence_test(
            CleanupPlanId::new("plan:blocked-rust-target").unwrap(),
            SystemTime::UNIX_EPOCH,
            CleanupMode::PermanentSafe,
            std::slice::from_ref(candidate),
        ),
        Err(CleanupPlanValidationError::BlockedCandidate { candidate_index: 0 })
    );
}

#[test]
fn output_limit_fails_as_a_typed_error_instead_of_silently_truncating() {
    let mut tree = DiskTree::new(PathBuf::from("/fixture"));
    for index in 0..=MAX_EVALUATED_CANDIDATES {
        add_rust_project(&mut tree, &format!("project-{index:04}"), 1);
    }

    assert_eq!(
        evaluate_artifact_candidates(
            &ScanId::new("scan:limit").unwrap(),
            &tree,
            &complete_coverage(),
            evaluated_at(),
        ),
        Err(CandidateEvaluationError::CandidateLimitExceeded {
            observed_at_least: MAX_EVALUATED_CANDIDATES + 1,
            maximum: MAX_EVALUATED_CANDIDATES,
        })
    );
}

#[test]
fn native_path_byte_helper_is_lossless_for_fixture_paths() {
    let path = Path::new("/fixture/project/target");
    assert!(!native_path_bytes(path).is_empty());
}
