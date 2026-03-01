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
    Xcode,
    Node,
    Generic,
    Gradle,
    Python,
    CocoaPods,
    NextNuxt,
    Vendor,
    Cache,
}

impl ArtifactKind {
    pub fn label(&self) -> &'static str {
        match self {
            ArtifactKind::Rust => "Rust",
            ArtifactKind::Xcode => "Xcode",
            ArtifactKind::Node => "Node",
            ArtifactKind::Generic => "Build",
            ArtifactKind::Gradle => "Gradle",
            ArtifactKind::Python => "Python",
            ArtifactKind::CocoaPods => "CocoaPods",
            ArtifactKind::NextNuxt => "Next/Nuxt",
            ArtifactKind::Vendor => "Vendor",
            ArtifactKind::Cache => "Cache",
        }
    }
}

pub fn classify_artifact(name: &str) -> Option<ArtifactKind> {
    match name {
        "target" => Some(ArtifactKind::Rust),
        "DerivedData" | "Build" => Some(ArtifactKind::Xcode),
        "node_modules" => Some(ArtifactKind::Node),
        "build" | "dist" => Some(ArtifactKind::Generic),
        ".gradle" => Some(ArtifactKind::Gradle),
        "__pycache__" | ".tox" | ".venv" | "venv" => Some(ArtifactKind::Python),
        "Pods" => Some(ArtifactKind::CocoaPods),
        ".next" | ".nuxt" => Some(ArtifactKind::NextNuxt),
        "vendor" => Some(ArtifactKind::Vendor),
        ".cache" => Some(ArtifactKind::Cache),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct BuildArtifactEntry {
    pub node_id: NodeId,
    pub relative_path: String,
    pub size: u64,
    pub percentage: f64,
    pub kind: ArtifactKind,
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

        entries.sort_by(|a, b| b.size.cmp(&a.size));
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
                let kind = classify_artifact(&node.name)?;
                // Skip if any ancestor is also a build artifact (e.g. target/debug/build)
                let mut parent_id = node.parent;
                while let Some(pid) = parent_id {
                    if let Some(parent) = tree.get(pid) {
                        if classify_artifact(&parent.name).is_some() {
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
                    kind,
                    is_stale,
                    newest_mtime,
                })
            })
            .collect();

        entries.sort_by(|a, b| b.size.cmp(&a.size));
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
