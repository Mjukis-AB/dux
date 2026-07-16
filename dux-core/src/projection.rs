use std::cmp::Reverse;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::{DiskTree, NodeId, NodeKind, size_percentage};

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactClassification {
    pub kind: ArtifactKind,
    /// Snapshot evidence only; callers must revalidate it before any mutation.
    pub evidence_node_ids: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuildArtifactEntry {
    pub node_id: NodeId,
    pub relative_path: String,
    pub size: u64,
    pub percentage: f64,
    pub kind: ArtifactKind,
    /// Snapshot marker paths only; callers must revalidate them before mutation.
    pub evidence_paths: Vec<PathBuf>,
    pub is_stale: bool,
    /// Most recent mtime of the artifact or any descendant node.
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
    pub fn duration(self) -> Option<Duration> {
        match self {
            Self::OneDay => Some(Duration::from_secs(86_400)),
            Self::SevenDays => Some(Duration::from_secs(7 * 86_400)),
            Self::ThirtyDays => Some(Duration::from_secs(30 * 86_400)),
            Self::NinetyDays => Some(Duration::from_secs(90 * 86_400)),
            Self::All => None,
        }
    }

    pub fn is_stale_at(self, newest_mtime: Option<SystemTime>, now: SystemTime) -> bool {
        match self.duration() {
            None => true,
            Some(duration) => newest_mtime
                .and_then(|mtime| now.duration_since(mtime).ok())
                .is_some_and(|age| age > duration),
        }
    }
}

pub fn classify_artifact(tree: &DiskTree, node_id: NodeId) -> Option<ArtifactClassification> {
    let node = tree.get(node_id)?;
    if !node.kind.is_directory() || has_symlink_component(tree, node_id) {
        return None;
    }
    let parent = node.parent;

    let (kind, evidence_node_ids) = match node.name.as_str() {
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

    Some(ArtifactClassification {
        kind,
        evidence_node_ids,
    })
}

pub fn project_large_files(tree: &DiskTree) -> Vec<LargeFileEntry> {
    let total_size = tree.total_size();
    let root_path = tree.root_path();
    let mut entries = tree
        .iter()
        .filter(|node| node.kind == NodeKind::File)
        .map(|node| LargeFileEntry {
            node_id: node.id,
            relative_path: node
                .path
                .strip_prefix(root_path)
                .unwrap_or(&node.path)
                .to_string_lossy()
                .to_string(),
            size: node.size,
            percentage: size_percentage(node.size, total_size),
        })
        .collect::<Vec<_>>();

    entries.sort_by_key(|entry| Reverse(entry.size));
    entries
}

pub fn project_build_artifacts_at(
    tree: &DiskTree,
    threshold: StaleThreshold,
    now: SystemTime,
) -> Vec<BuildArtifactEntry> {
    collect_build_artifacts_at(tree, threshold, now, None)
        .expect("an unbounded artifact projection cannot reach a match limit")
}

/// Internal bounded variant for callers that must fail closed without first
/// materializing every match in a potentially multi-million-node snapshot.
pub(crate) fn project_build_artifacts_bounded_at(
    tree: &DiskTree,
    threshold: StaleThreshold,
    now: SystemTime,
    maximum_matches: usize,
) -> Result<Vec<BuildArtifactEntry>, usize> {
    collect_build_artifacts_at(tree, threshold, now, Some(maximum_matches))
}

fn collect_build_artifacts_at(
    tree: &DiskTree,
    threshold: StaleThreshold,
    now: SystemTime,
    maximum_matches: Option<usize>,
) -> Result<Vec<BuildArtifactEntry>, usize> {
    let total_size = tree.total_size();
    let root_path = tree.root_path();
    let mut entries = Vec::new();
    for node in tree.iter() {
        let Some(classification) = classify_artifact(tree, node.id) else {
            continue;
        };
        if has_classified_ancestor(tree, node.id) {
            continue;
        }
        if maximum_matches.is_some_and(|maximum| entries.len() == maximum) {
            return Err(entries.len().saturating_add(1));
        }

        let newest_mtime = newest_descendant_mtime(tree, node.id);
        entries.push(BuildArtifactEntry {
            node_id: node.id,
            relative_path: node
                .path
                .strip_prefix(root_path)
                .unwrap_or(&node.path)
                .to_string_lossy()
                .to_string(),
            size: node.size,
            percentage: size_percentage(node.size, total_size),
            kind: classification.kind,
            evidence_paths: classification
                .evidence_node_ids
                .into_iter()
                .filter_map(|id| tree.get(id).map(|evidence| evidence.path.clone()))
                .collect(),
            is_stale: threshold.is_stale_at(newest_mtime, now),
            newest_mtime,
        });
    }

    entries.sort_by_key(|entry| Reverse(entry.size));
    Ok(entries)
}

pub fn refresh_artifact_staleness_at(
    entries: &mut [BuildArtifactEntry],
    threshold: StaleThreshold,
    now: SystemTime,
) {
    for entry in entries {
        entry.is_stale = threshold.is_stale_at(entry.newest_mtime, now);
    }
}

fn has_classified_ancestor(tree: &DiskTree, node_id: NodeId) -> bool {
    let mut parent_id = tree.get(node_id).and_then(|node| node.parent);
    while let Some(id) = parent_id {
        let Some(parent) = tree.get(id) else {
            return false;
        };
        if classify_artifact(tree, id).is_some() {
            return true;
        }
        parent_id = parent.parent;
    }
    false
}

fn newest_descendant_mtime(tree: &DiskTree, root: NodeId) -> Option<SystemTime> {
    let mut newest = None;
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(node) = tree.get(id) else {
            continue;
        };
        if let Some(mtime) = node.mtime {
            newest = Some(newest.map_or(mtime, |previous: SystemTime| previous.max(mtime)));
        }
        stack.extend(node.children.iter().copied());
    }
    newest
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
    let children = &tree.get(parent)?.children;
    names.iter().find_map(|name| {
        children.iter().copied().find(|child_id| {
            tree.get(*child_id).is_some_and(|child| {
                child.kind == NodeKind::File
                    && !child.path_is_symlink
                    && child.name.as_str() == *name
            })
        })
    })
}

fn regular_python_sibling(tree: &DiskTree, parent: NodeId) -> Option<NodeId> {
    tree.get(parent)?
        .children
        .iter()
        .copied()
        .filter(|child_id| {
            tree.get(*child_id).is_some_and(|child| {
                child.kind == NodeKind::File
                    && !child.path_is_symlink
                    && PathBuf::from(&child.name)
                        .extension()
                        .is_some_and(|extension| extension == "py")
            })
        })
        .min_by(|left, right| {
            tree.get(*left)
                .map(|node| native_path_bytes(&node.path))
                .cmp(&tree.get(*right).map(|node| native_path_bytes(&node.path)))
        })
}

#[cfg(unix)]
fn native_path_bytes(path: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn native_path_bytes(path: &std::path::Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(not(any(unix, windows)))]
fn native_path_bytes(path: &std::path::Path) -> Vec<u8> {
    path.as_os_str().to_string_lossy().as_bytes().to_vec()
}
