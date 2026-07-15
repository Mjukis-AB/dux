use std::cmp::Reverse;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use dux_core::{DiskTree, NodeId, NodeKind, size_percentage};

#[derive(Debug, Clone)]
pub struct LargeFileEntry {
    pub node_id: NodeId,
    pub relative_path: String,
    pub size: u64,
    pub percentage: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Rust,
    Node,
    Gradle,
    Python,
    CocoaPods,
    NextNuxt,
}

impl ArtifactKind {
    pub fn label(&self) -> &'static str {
        match self {
            ArtifactKind::Rust => "Rust",
            ArtifactKind::Node => "Node",
            ArtifactKind::Gradle => "Gradle",
            ArtifactKind::Python => "Python",
            ArtifactKind::CocoaPods => "CocoaPods",
            ArtifactKind::NextNuxt => "Next/Nuxt",
        }
    }
}

#[derive(Debug)]
struct ArtifactMatch {
    kind: ArtifactKind,
    evidence: Vec<NodeId>,
}

fn classify_artifact(tree: &DiskTree, node_id: NodeId) -> Option<ArtifactMatch> {
    let node = tree.get(node_id)?;
    if !node.kind.is_directory() || has_symlink_component(tree, node_id) {
        return None;
    }
    let parent = node.parent;

    let (kind, evidence) = match node.name.as_str() {
        "target" => (
            ArtifactKind::Rust,
            vec![regular_sibling_named(tree, parent?, &["Cargo.toml"])?],
        ),
        "node_modules" => (
            ArtifactKind::Node,
            vec![regular_sibling_named(tree, parent?, &["package.json"])?],
        ),
        "build" | ".gradle" => (
            ArtifactKind::Gradle,
            vec![regular_sibling_named(
                tree,
                parent?,
                &[
                    "build.gradle",
                    "build.gradle.kts",
                    "settings.gradle",
                    "settings.gradle.kts",
                ],
            )?],
        ),
        "__pycache__" => (
            ArtifactKind::Python,
            vec![regular_python_sibling(tree, parent?)?],
        ),
        ".tox" => (
            ArtifactKind::Python,
            vec![regular_sibling_named(tree, parent?, &["tox.ini"])?],
        ),
        ".venv" | "venv" => (
            ArtifactKind::Python,
            vec![regular_child_named(tree, node_id, &["pyvenv.cfg"])?],
        ),
        "Pods" => (
            ArtifactKind::CocoaPods,
            vec![
                regular_sibling_named(tree, parent?, &["Podfile"])?,
                regular_child_named(tree, node_id, &["Manifest.lock"])?,
            ],
        ),
        ".next" => (
            ArtifactKind::NextNuxt,
            vec![
                regular_sibling_named(tree, parent?, &["package.json"])?,
                regular_sibling_named(
                    tree,
                    parent?,
                    &["next.config.js", "next.config.mjs", "next.config.ts"],
                )?,
            ],
        ),
        ".nuxt" => (
            ArtifactKind::NextNuxt,
            vec![
                regular_sibling_named(tree, parent?, &["package.json"])?,
                regular_sibling_named(
                    tree,
                    parent?,
                    &["nuxt.config.js", "nuxt.config.mjs", "nuxt.config.ts"],
                )?,
            ],
        ),
        // These names are too ambiguous to authorize a permanent-delete-backed
        // artifact entry without stronger, tool-specific evidence.
        "DerivedData" | "Build" | "dist" | "vendor" | ".cache" => return None,
        _ => return None,
    };

    Some(ArtifactMatch { kind, evidence })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ComputedViews::rebuild_build_artifacts(tree, StaleThreshold::All)
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
        assert_entry(&entries, rust_target, ArtifactKind::Rust, &["Cargo.toml"]);
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
        add_dir(&mut tree, wrong_kind, "target");

        let wrong_case = add_dir(&mut tree, NodeId::ROOT, "wrong-case");
        add_file(&mut tree, wrong_case, "cargo.toml");
        add_dir(&mut tree, wrong_case, "target");

        let wrong_location = add_dir(&mut tree, NodeId::ROOT, "wrong-location");
        let target = add_dir(&mut tree, wrong_location, "target");
        add_file(&mut tree, target, "Cargo.toml");

        let symlink_marker = add_dir(&mut tree, NodeId::ROOT, "symlink-marker");
        let marker = add_file(&mut tree, symlink_marker, "Cargo.toml");
        tree.get_mut(marker).unwrap().path_is_symlink = true;
        add_dir(&mut tree, symlink_marker, "target");

        assert!(artifact_entries(&tree).is_empty());
    }

    #[test]
    fn symlinked_artifact_or_ancestor_is_rejected() {
        let mut tree = DiskTree::new(PathBuf::from("/scan"));

        let direct = add_dir(&mut tree, NodeId::ROOT, "direct");
        add_file(&mut tree, direct, "Cargo.toml");
        let direct_target = add_dir(&mut tree, direct, "target");
        tree.get_mut(direct_target).unwrap().path_is_symlink = true;

        let linked_parent = add_dir(&mut tree, NodeId::ROOT, "linked-parent");
        tree.get_mut(linked_parent).unwrap().path_is_symlink = true;
        let nested = add_dir(&mut tree, linked_parent, "nested");
        add_file(&mut tree, nested, "Cargo.toml");
        add_dir(&mut tree, nested, "target");

        assert!(artifact_entries(&tree).is_empty());
    }

    #[test]
    fn unverified_artifact_name_does_not_suppress_verified_descendant() {
        let mut tree = DiskTree::new(PathBuf::from("/scan"));
        let unverified_build = add_dir(&mut tree, NodeId::ROOT, "build");
        let project = add_dir(&mut tree, unverified_build, "project");
        add_file(&mut tree, project, "Cargo.toml");
        let target = add_dir(&mut tree, project, "target");

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
        add_dir(&mut tree, package, "target");

        let entries = artifact_entries(&tree);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].node_id, modules);
    }

    #[test]
    fn removing_marker_removes_candidate_on_rebuild() {
        let mut tree = DiskTree::new(PathBuf::from("/scan"));
        let marker = add_file(&mut tree, NodeId::ROOT, "Cargo.toml");
        add_dir(&mut tree, NodeId::ROOT, "target");
        assert_eq!(artifact_entries(&tree).len(), 1);

        tree.remove_node(marker);

        assert!(artifact_entries(&tree).is_empty());
    }
}

fn has_symlink_component(tree: &DiskTree, mut node_id: NodeId) -> bool {
    loop {
        let Some(node) = tree.get(node_id) else {
            return true;
        };
        if node.path_is_symlink {
            return true;
        }
        let Some(parent) = node.parent else {
            return false;
        };
        node_id = parent;
    }
}

fn regular_sibling_named(tree: &DiskTree, parent: NodeId, names: &[&str]) -> Option<NodeId> {
    regular_child_named(tree, parent, names)
}

fn regular_child_named(tree: &DiskTree, parent: NodeId, names: &[&str]) -> Option<NodeId> {
    tree.get(parent)?.children.iter().copied().find(|child_id| {
        tree.get(*child_id).is_some_and(|child| {
            child.kind == NodeKind::File
                && !child.path_is_symlink
                && names.contains(&child.name.as_str())
        })
    })
}

fn regular_python_sibling(tree: &DiskTree, parent: NodeId) -> Option<NodeId> {
    tree.get(parent)?.children.iter().copied().find(|child_id| {
        tree.get(*child_id).is_some_and(|child| {
            child.kind == NodeKind::File
                && !child.path_is_symlink
                && PathBuf::from(&child.name)
                    .extension()
                    .is_some_and(|ext| ext == "py")
        })
    })
}

#[derive(Debug, Clone)]
pub struct BuildArtifactEntry {
    pub node_id: NodeId,
    pub relative_path: String,
    pub size: u64,
    pub percentage: f64,
    pub kind: ArtifactKind,
    /// Non-symlink marker paths that justified this snapshot classification
    pub evidence_paths: Vec<PathBuf>,
    pub is_stale: bool,
    /// Most recent mtime of any descendant directory
    pub newest_mtime: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleThreshold {
    OneDay,
    SevenDays,
    ThirtyDays,
    NinetyDays,
    All,
}

impl StaleThreshold {
    pub fn label(&self) -> &'static str {
        match self {
            StaleThreshold::OneDay => "1d",
            StaleThreshold::SevenDays => "7d",
            StaleThreshold::ThirtyDays => "30d",
            StaleThreshold::NinetyDays => "90d",
            StaleThreshold::All => "All",
        }
    }

    pub fn duration(&self) -> Option<Duration> {
        match self {
            StaleThreshold::OneDay => Some(Duration::from_secs(86400)),
            StaleThreshold::SevenDays => Some(Duration::from_secs(7 * 86400)),
            StaleThreshold::ThirtyDays => Some(Duration::from_secs(30 * 86400)),
            StaleThreshold::NinetyDays => Some(Duration::from_secs(90 * 86400)),
            StaleThreshold::All => None,
        }
    }

    pub fn next(&self) -> Self {
        match self {
            StaleThreshold::OneDay => StaleThreshold::SevenDays,
            StaleThreshold::SevenDays => StaleThreshold::ThirtyDays,
            StaleThreshold::ThirtyDays => StaleThreshold::NinetyDays,
            StaleThreshold::NinetyDays => StaleThreshold::All,
            StaleThreshold::All => StaleThreshold::OneDay,
        }
    }
}

pub struct ComputedViews {
    pub large_files: Vec<LargeFileEntry>,
    pub build_artifacts: Vec<BuildArtifactEntry>,
    pub dirty: bool,
    pub stale_threshold: StaleThreshold,
}

impl ComputedViews {
    pub fn new() -> Self {
        Self {
            large_files: Vec::new(),
            build_artifacts: Vec::new(),
            dirty: true,
            stale_threshold: StaleThreshold::SevenDays,
        }
    }

    pub fn rebuild(&mut self, tree: &DiskTree) {
        self.large_files = Self::rebuild_large_files(tree);
        self.build_artifacts = Self::rebuild_build_artifacts(tree, self.stale_threshold);
        self.dirty = false;
    }

    pub fn cycle_stale_threshold(&mut self) {
        self.stale_threshold = self.stale_threshold.next();
        // Only update is_stale flags — no need to re-collect from tree
        let now = SystemTime::now();
        let threshold = self.stale_threshold;
        for entry in &mut self.build_artifacts {
            entry.is_stale = match threshold.duration() {
                None => true,
                Some(dur) => entry
                    .newest_mtime
                    .and_then(|mt| now.duration_since(mt).ok())
                    .map(|age| age > dur)
                    .unwrap_or(false),
            };
        }
    }

    fn rebuild_large_files(tree: &DiskTree) -> Vec<LargeFileEntry> {
        let total_size = tree.total_size();
        let root_path = tree.root_path();

        let mut entries: Vec<LargeFileEntry> = tree
            .iter()
            .filter(|node| node.kind == NodeKind::File)
            .map(|node| {
                let relative_path = node
                    .path
                    .strip_prefix(root_path)
                    .unwrap_or(&node.path)
                    .to_string_lossy()
                    .to_string();
                LargeFileEntry {
                    node_id: node.id,
                    relative_path,
                    size: node.size,
                    percentage: size_percentage(node.size, total_size),
                }
            })
            .collect();

        entries.sort_by_key(|entry| Reverse(entry.size));
        entries
    }

    fn rebuild_build_artifacts(
        tree: &DiskTree,
        threshold: StaleThreshold,
    ) -> Vec<BuildArtifactEntry> {
        let total_size = tree.total_size();
        let root_path = tree.root_path();
        let now = SystemTime::now();

        let mut entries: Vec<BuildArtifactEntry> = tree
            .iter()
            .filter_map(|node| {
                if !node.kind.is_directory() {
                    return None;
                }
                let artifact_match = classify_artifact(tree, node.id)?;
                // Skip if any ancestor is also a build artifact (e.g. target/debug/build)
                let mut parent_id = node.parent;
                while let Some(pid) = parent_id {
                    if let Some(parent) = tree.get(pid) {
                        if classify_artifact(tree, parent.id).is_some() {
                            return None;
                        }
                        parent_id = parent.parent;
                    } else {
                        break;
                    }
                }
                let relative_path = node
                    .path
                    .strip_prefix(root_path)
                    .unwrap_or(&node.path)
                    .to_string_lossy()
                    .to_string();
                // Find the newest mtime among all descendant directories
                let newest_mtime = Self::newest_descendant_mtime(tree, node.id);
                let is_stale = match threshold.duration() {
                    None => true,
                    Some(dur) => newest_mtime
                        .and_then(|mt| now.duration_since(mt).ok())
                        .map(|age| age > dur)
                        .unwrap_or(false),
                };
                Some(BuildArtifactEntry {
                    node_id: node.id,
                    relative_path,
                    size: node.size,
                    percentage: size_percentage(node.size, total_size),
                    kind: artifact_match.kind,
                    evidence_paths: artifact_match
                        .evidence
                        .into_iter()
                        .filter_map(|id| tree.get(id).map(|evidence| evidence.path.clone()))
                        .collect(),
                    is_stale,
                    newest_mtime,
                })
            })
            .collect();

        entries.sort_by_key(|entry| Reverse(entry.size));
        entries
    }

    /// Walk all descendant directories and return the most recent mtime
    fn newest_descendant_mtime(tree: &DiskTree, root: NodeId) -> Option<SystemTime> {
        let mut newest: Option<SystemTime> = None;
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if let Some(node) = tree.get(id) {
                if let Some(mt) = node.mtime {
                    newest = Some(match newest {
                        Some(prev) => prev.max(mt),
                        None => mt,
                    });
                }
                for &child in &node.children {
                    stack.push(child);
                }
            }
        }
        newest
    }
}
