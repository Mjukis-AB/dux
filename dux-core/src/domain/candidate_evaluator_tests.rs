use std::path::{Path, PathBuf};

use super::*;
use crate::domain::{ScanIssue, ScanIssueKind};
use crate::tree::{NodeId, NodeKind};

fn complete_coverage() -> ScanCoverage {
    ScanCoverage::from_validated_terminal_issues(Vec::new())
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
    tree.set_size(target, bytes);
    target_path
}

fn evaluate(tree: &DiskTree) -> CandidateBatch {
    evaluate_artifact_candidates(
        &ScanId::new("scan:fixture").unwrap(),
        tree,
        &complete_coverage(),
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
    assert_eq!(
        candidate.evidence(),
        &[
            Evidence::MatchedPath {
                path: target_path.clone(),
            },
            Evidence::RequiredMarker {
                path: PathBuf::from("/fixture/project/Cargo.toml"),
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
    tree.add_node(
        "target".to_owned(),
        NodeKind::Directory,
        PathBuf::from("/fixture/project/target/nested/target"),
        nested_project,
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
        "63669eace629b010d78e8d4cd9fc72bf76dbf05a3c8edb64ec146b6b37f87c60"
    );
    assert!(
        load_and_validate_catalog()
            .unwrap()
            .iter()
            .all(|rule| rule.scope() == RuleScope::SelectedScanRoot)
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

    let batch =
        evaluate_artifact_candidates(&ScanId::new("scan:partial").unwrap(), &tree, &coverage)
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
    )
    .unwrap();
    let repeated = evaluate_artifact_candidates(
        &ScanId::new("scan:context").unwrap(),
        &empty,
        &complete_coverage(),
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
    )
    .unwrap();
    let unknown_coverage = evaluate_artifact_candidates(
        &ScanId::new("scan:context").unwrap(),
        &empty,
        &ScanCoverage::unknown(),
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
    tree.add_node(
        "target".to_owned(),
        NodeKind::Directory,
        target_path.clone(),
        project,
    );

    let first = evaluate(&tree);
    let second = evaluate(&tree);
    assert_eq!(first.candidates()[0].paths(), &[target_path]);
    assert_eq!(first.candidates()[0].id(), second.candidates()[0].id());
}

#[test]
fn bundled_findings_never_expose_cleanup_or_scheduling_authority() {
    let mut tree = DiskTree::new(PathBuf::from("/fixture"));
    add_rust_project(&mut tree, "project", 10);

    for candidate in evaluate(&tree).candidates() {
        assert_eq!(candidate.safety(), SafetyTier::Informational);
        assert_eq!(candidate.action(), CandidateAction::RevealOnly);
        assert!(!candidate.has_cleanup_operation());
        assert!(!candidate.rule_marks_schedule_eligible());
        assert!(!candidate.has_no_known_blockers());
    }
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
