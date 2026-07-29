use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use dux_core::{
    ArtifactKind, BuildArtifactEntry, DiskTree, NodeId, NodeKind, StaleThreshold,
    project_build_artifacts_at, project_large_files, refresh_artifact_staleness_at,
};

fn add_node(tree: &mut DiskTree, parent: NodeId, name: &str, kind: NodeKind) -> NodeId {
    let path = tree.get(parent).unwrap().path.join(name);
    tree.add_node(name.to_string(), kind, path, parent)
}

fn add_dir(tree: &mut DiskTree, parent: NodeId, name: &str) -> NodeId {
    add_node(tree, parent, name, NodeKind::Directory)
}

fn add_file(tree: &mut DiskTree, parent: NodeId, name: &str) -> NodeId {
    add_node(tree, parent, name, NodeKind::File)
}

fn artifact_entries(tree: &DiskTree) -> Vec<BuildArtifactEntry> {
    project_build_artifacts_at(tree, StaleThreshold::All, SystemTime::UNIX_EPOCH)
}

fn assert_entry(
    entries: &[BuildArtifactEntry],
    node_id: NodeId,
    kind: ArtifactKind,
    expected_evidence: &[&str],
) {
    let entry = entries
        .iter()
        .find(|entry| entry.node_id == node_id)
        .unwrap();
    assert_eq!(entry.kind, kind);
    let mut evidence_names = entry
        .evidence_paths
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    evidence_names.sort();
    let mut expected = expected_evidence
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(evidence_names, expected);
}

#[test]
fn bare_artifact_names_are_rejected_without_evidence() {
    let mut tree = DiskTree::new(PathBuf::from("/scan"));
    for name in [
        "target",
        "node_modules",
        "build",
        ".gradle",
        "__pycache__",
        ".tox",
        ".venv",
        "venv",
        "Pods",
        ".next",
        ".nuxt",
        "vendor",
        "DerivedData",
        "Build",
        "dist",
        ".cache",
    ] {
        add_dir(&mut tree, NodeId::ROOT, name);
    }

    assert!(artifact_entries(&tree).is_empty());
}

#[test]
fn classifies_only_supported_artifacts_with_exact_markers() {
    let mut tree = DiskTree::new(PathBuf::from("/scan"));

    let rust = add_dir(&mut tree, NodeId::ROOT, "rust-project");
    add_file(&mut tree, rust, "Cargo.toml");
    let rust_target = add_dir(&mut tree, rust, "target");
    add_file(&mut tree, rust_target, "CACHEDIR.TAG");

    let node = add_dir(&mut tree, NodeId::ROOT, "node-project");
    add_file(&mut tree, node, "package.json");
    let node_modules = add_dir(&mut tree, node, "node_modules");

    let gradle = add_dir(&mut tree, NodeId::ROOT, "gradle-project");
    add_file(&mut tree, gradle, "build.gradle.kts");
    let gradle_build = add_dir(&mut tree, gradle, "build");

    let gradle_cache_project = add_dir(&mut tree, NodeId::ROOT, "gradle-cache-project");
    add_file(&mut tree, gradle_cache_project, "settings.gradle");
    let gradle_cache = add_dir(&mut tree, gradle_cache_project, ".gradle");

    let python = add_dir(&mut tree, NodeId::ROOT, "python-package");
    add_file(&mut tree, python, "module.py");
    let pycache = add_dir(&mut tree, python, "__pycache__");

    let venv_parent = add_dir(&mut tree, NodeId::ROOT, "python-project");
    let venv = add_dir(&mut tree, venv_parent, ".venv");
    add_file(&mut tree, venv, "pyvenv.cfg");

    let plain_venv_parent = add_dir(&mut tree, NodeId::ROOT, "plain-venv-project");
    let plain_venv = add_dir(&mut tree, plain_venv_parent, "venv");
    add_file(&mut tree, plain_venv, "pyvenv.cfg");

    let tox = add_dir(&mut tree, NodeId::ROOT, "tox-project");
    add_file(&mut tree, tox, "tox.ini");
    let tox_dir = add_dir(&mut tree, tox, ".tox");

    let pods = add_dir(&mut tree, NodeId::ROOT, "ios-project");
    add_file(&mut tree, pods, "Podfile");
    let pods_dir = add_dir(&mut tree, pods, "Pods");
    add_file(&mut tree, pods_dir, "Manifest.lock");

    let next = add_dir(&mut tree, NodeId::ROOT, "next-project");
    add_file(&mut tree, next, "package.json");
    add_file(&mut tree, next, "next.config.ts");
    let next_dir = add_dir(&mut tree, next, ".next");

    let nuxt = add_dir(&mut tree, NodeId::ROOT, "nuxt-project");
    add_file(&mut tree, nuxt, "package.json");
    add_file(&mut tree, nuxt, "nuxt.config.ts");
    let nuxt_dir = add_dir(&mut tree, nuxt, ".nuxt");

    let entries = artifact_entries(&tree);

    assert_eq!(entries.len(), 11);
    assert_entry(
        &entries,
        rust_target,
        ArtifactKind::Rust,
        &["CACHEDIR.TAG", "Cargo.toml"],
    );
    assert_entry(
        &entries,
        node_modules,
        ArtifactKind::Node,
        &["package.json"],
    );
    assert_entry(
        &entries,
        gradle_build,
        ArtifactKind::Gradle,
        &["build.gradle.kts"],
    );
    assert_entry(
        &entries,
        gradle_cache,
        ArtifactKind::Gradle,
        &["settings.gradle"],
    );
    assert_entry(&entries, pycache, ArtifactKind::Python, &["module.py"]);
    assert_entry(&entries, venv, ArtifactKind::Python, &["pyvenv.cfg"]);
    assert_entry(&entries, plain_venv, ArtifactKind::Python, &["pyvenv.cfg"]);
    assert_entry(&entries, tox_dir, ArtifactKind::Python, &["tox.ini"]);
    assert_entry(
        &entries,
        pods_dir,
        ArtifactKind::CocoaPods,
        &["Manifest.lock", "Podfile"],
    );
    assert_entry(
        &entries,
        next_dir,
        ArtifactKind::NextNuxt,
        &["next.config.ts", "package.json"],
    );
    assert_entry(
        &entries,
        nuxt_dir,
        ArtifactKind::NextNuxt,
        &["nuxt.config.ts", "package.json"],
    );
}

#[test]
fn partial_or_ambiguous_evidence_is_rejected() {
    let mut tree = DiskTree::new(PathBuf::from("/scan"));

    let rust_without_tag = add_dir(&mut tree, NodeId::ROOT, "rust-without-tag");
    add_file(&mut tree, rust_without_tag, "Cargo.toml");
    add_dir(&mut tree, rust_without_tag, "target");

    let rust_without_manifest = add_dir(&mut tree, NodeId::ROOT, "rust-without-manifest");
    let target = add_dir(&mut tree, rust_without_manifest, "target");
    add_file(&mut tree, target, "CACHEDIR.TAG");

    let pods = add_dir(&mut tree, NodeId::ROOT, "pods-without-lock");
    add_file(&mut tree, pods, "Podfile");
    add_dir(&mut tree, pods, "Pods");

    let next = add_dir(&mut tree, NodeId::ROOT, "next-without-config");
    add_file(&mut tree, next, "package.json");
    add_dir(&mut tree, next, ".next");

    let nuxt = add_dir(&mut tree, NodeId::ROOT, "nuxt-without-package");
    add_file(&mut tree, nuxt, "nuxt.config.ts");
    add_dir(&mut tree, nuxt, ".nuxt");

    let venv_parent = add_dir(&mut tree, NodeId::ROOT, "symlinked-venv-marker");
    let venv = add_dir(&mut tree, venv_parent, "venv");
    let pyvenv = add_file(&mut tree, venv, "pyvenv.cfg");
    tree.get_mut(pyvenv).unwrap().path_is_symlink = true;

    let generic = add_dir(&mut tree, NodeId::ROOT, "generic-project");
    add_file(&mut tree, generic, "package.json");
    add_file(&mut tree, generic, "CACHEDIR.TAG");
    add_file(&mut tree, generic, "build.gradle");
    add_dir(&mut tree, generic, "dist");
    add_dir(&mut tree, generic, ".cache");
    add_dir(&mut tree, generic, "Build");
    let derived_data = add_dir(&mut tree, generic, "DerivedData");
    add_dir(&mut tree, derived_data, "Build");

    assert!(artifact_entries(&tree).is_empty());
}

#[test]
fn wrong_kind_case_location_and_symlink_markers_are_rejected() {
    let mut tree = DiskTree::new(PathBuf::from("/scan"));

    let wrong_kind = add_dir(&mut tree, NodeId::ROOT, "wrong-kind");
    add_dir(&mut tree, wrong_kind, "Cargo.toml");
    let target = add_dir(&mut tree, wrong_kind, "target");
    add_file(&mut tree, target, "CACHEDIR.TAG");

    let wrong_case = add_dir(&mut tree, NodeId::ROOT, "wrong-case");
    add_file(&mut tree, wrong_case, "cargo.toml");
    let target = add_dir(&mut tree, wrong_case, "target");
    add_file(&mut tree, target, "CACHEDIR.TAG");

    let wrong_location = add_dir(&mut tree, NodeId::ROOT, "wrong-location");
    let target = add_dir(&mut tree, wrong_location, "target");
    add_file(&mut tree, target, "Cargo.toml");
    add_file(&mut tree, target, "CACHEDIR.TAG");

    let symlink_marker = add_dir(&mut tree, NodeId::ROOT, "symlink-marker");
    let marker = add_file(&mut tree, symlink_marker, "Cargo.toml");
    tree.get_mut(marker).unwrap().path_is_symlink = true;
    let target = add_dir(&mut tree, symlink_marker, "target");
    add_file(&mut tree, target, "CACHEDIR.TAG");

    let symlink_tag = add_dir(&mut tree, NodeId::ROOT, "symlink-tag");
    add_file(&mut tree, symlink_tag, "Cargo.toml");
    let target = add_dir(&mut tree, symlink_tag, "target");
    let tag = add_file(&mut tree, target, "CACHEDIR.TAG");
    tree.get_mut(tag).unwrap().path_is_symlink = true;

    assert!(artifact_entries(&tree).is_empty());
}

#[test]
fn symlinked_artifact_or_ancestor_is_rejected() {
    let mut tree = DiskTree::new(PathBuf::from("/scan"));

    let direct = add_dir(&mut tree, NodeId::ROOT, "direct");
    add_file(&mut tree, direct, "Cargo.toml");
    let direct_target = add_dir(&mut tree, direct, "target");
    add_file(&mut tree, direct_target, "CACHEDIR.TAG");
    tree.get_mut(direct_target).unwrap().path_is_symlink = true;

    let linked_parent = add_dir(&mut tree, NodeId::ROOT, "linked-parent");
    tree.get_mut(linked_parent).unwrap().path_is_symlink = true;
    let nested = add_dir(&mut tree, linked_parent, "nested");
    add_file(&mut tree, nested, "Cargo.toml");
    let target = add_dir(&mut tree, nested, "target");
    add_file(&mut tree, target, "CACHEDIR.TAG");

    assert!(artifact_entries(&tree).is_empty());
}

#[test]
fn unverified_artifact_name_does_not_suppress_verified_descendant() {
    let mut tree = DiskTree::new(PathBuf::from("/scan"));
    let unverified_build = add_dir(&mut tree, NodeId::ROOT, "build");
    let project = add_dir(&mut tree, unverified_build, "project");
    add_file(&mut tree, project, "Cargo.toml");
    let target = add_dir(&mut tree, project, "target");
    add_file(&mut tree, target, "CACHEDIR.TAG");

    let entries = artifact_entries(&tree);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].node_id, target);
}

#[test]
fn verified_artifact_ancestor_suppresses_verified_descendant() {
    let mut tree = DiskTree::new(PathBuf::from("/scan"));
    add_file(&mut tree, NodeId::ROOT, "package.json");
    let modules = add_dir(&mut tree, NodeId::ROOT, "node_modules");
    let package = add_dir(&mut tree, modules, "package");
    add_file(&mut tree, package, "Cargo.toml");
    let target = add_dir(&mut tree, package, "target");
    add_file(&mut tree, target, "CACHEDIR.TAG");

    let entries = artifact_entries(&tree);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].node_id, modules);
}

#[test]
fn removing_marker_removes_candidate_on_rebuild() {
    let mut tree = DiskTree::new(PathBuf::from("/scan"));
    let marker = add_file(&mut tree, NodeId::ROOT, "Cargo.toml");
    let target = add_dir(&mut tree, NodeId::ROOT, "target");
    add_file(&mut tree, target, "CACHEDIR.TAG");
    assert_eq!(artifact_entries(&tree).len(), 1);

    tree.remove_node(marker);

    assert!(artifact_entries(&tree).is_empty());
}

#[test]
fn large_file_projection_is_file_only_relative_and_stably_size_sorted() {
    let mut tree = DiskTree::new(PathBuf::from("/scan"));
    let first_tie = add_file(&mut tree, NodeId::ROOT, "first.bin");
    let directory = add_dir(&mut tree, NodeId::ROOT, "nested");
    let largest = add_file(&mut tree, directory, "largest.bin");
    let second_tie = add_file(&mut tree, directory, "second.bin");
    let empty = add_file(&mut tree, NodeId::ROOT, "empty.bin");
    tree.set_size(first_tie, 10);
    tree.set_size(largest, 20);
    tree.set_size(second_tie, 10);
    tree.set_size(empty, 0);
    tree.aggregate_sizes();

    let entries = project_large_files(&tree);

    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.node_id)
            .collect::<Vec<_>>(),
        [largest, first_tie, second_tie, empty]
    );
    assert_eq!(
        entries[0].relative_path,
        PathBuf::from("nested")
            .join("largest.bin")
            .to_string_lossy()
    );
    assert!((entries[0].percentage - 50.0).abs() < f64::EPSILON);
    assert!((entries[1].percentage - 25.0).abs() < f64::EPSILON);
    assert!(entries.iter().all(|entry| entry.node_id != directory));
}

#[test]
fn zero_total_large_file_projection_has_zero_percentages() {
    let mut tree = DiskTree::new(PathBuf::from("/scan"));
    add_file(&mut tree, NodeId::ROOT, "empty.bin");
    tree.aggregate_sizes();

    let entries = project_large_files(&tree);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].percentage, 0.0);
}

#[test]
fn large_file_projection_falls_back_to_the_full_path() {
    let mut tree = DiskTree::new(PathBuf::from("/scan"));
    let file = add_file(&mut tree, NodeId::ROOT, "moved.bin");
    tree.get_mut(file).unwrap().path = PathBuf::from("/outside/moved.bin");
    tree.set_size(file, 1);
    tree.aggregate_sizes();

    let entries = project_large_files(&tree);

    assert_eq!(entries[0].relative_path, "/outside/moved.bin");
}

#[test]
fn staleness_is_deterministic_at_boundaries_and_clock_anomalies() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100 * 86_400);
    let exact = now - Duration::from_secs(86_400);
    let old = exact - Duration::from_secs(1);
    let future = now + Duration::from_secs(1);

    assert!(!StaleThreshold::OneDay.is_stale_at(None, now));
    assert!(!StaleThreshold::OneDay.is_stale_at(Some(future), now));
    assert!(!StaleThreshold::OneDay.is_stale_at(Some(exact), now));
    assert!(StaleThreshold::OneDay.is_stale_at(Some(old), now));
    assert!(StaleThreshold::All.is_stale_at(None, now));
}

#[test]
fn artifact_projection_uses_newest_subtree_mtime_and_refreshes_in_place() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100 * 86_400);
    let mut tree = DiskTree::new(PathBuf::from("/scan"));
    add_file(&mut tree, NodeId::ROOT, "Cargo.toml");
    let target = add_dir(&mut tree, NodeId::ROOT, "target");
    let cache_tag = add_file(&mut tree, target, "CACHEDIR.TAG");
    let descendant = add_dir(&mut tree, target, "debug");
    let output = add_file(&mut tree, descendant, "output");
    tree.get_mut(target).unwrap().mtime = Some(now - Duration::from_secs(20 * 86_400));
    tree.get_mut(cache_tag).unwrap().mtime = Some(now - Duration::from_secs(30 * 86_400));
    tree.get_mut(descendant).unwrap().mtime = Some(now - Duration::from_secs(10 * 86_400));
    let newest_mtime = now - Duration::from_secs(1);
    tree.get_mut(output).unwrap().mtime = Some(newest_mtime);

    let mut entries = project_build_artifacts_at(&tree, StaleThreshold::SevenDays, now);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].newest_mtime, Some(newest_mtime));
    assert!(!entries[0].is_stale);
    let unchanged = (
        entries[0].node_id,
        entries[0].kind,
        entries[0].evidence_paths.clone(),
    );

    refresh_artifact_staleness_at(&mut entries, StaleThreshold::All, now);

    assert!(entries[0].is_stale);
    assert_eq!(
        (
            entries[0].node_id,
            entries[0].kind,
            entries[0].evidence_paths.clone(),
        ),
        unchanged
    );
}
