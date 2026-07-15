use std::time::SystemTime;

use dux_core::{
    DiskTree, project_build_artifacts_at, project_large_files, refresh_artifact_staleness_at,
};

pub use dux_core::{ArtifactKind, BuildArtifactEntry, LargeFileEntry, StaleThreshold};

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
        let now = SystemTime::now();
        self.large_files = project_large_files(tree);
        self.build_artifacts = project_build_artifacts_at(tree, self.stale_threshold, now);
        self.dirty = false;
    }

    pub fn cycle_stale_threshold(&mut self) {
        self.stale_threshold = next_stale_threshold(self.stale_threshold);
        refresh_artifact_staleness_at(
            &mut self.build_artifacts,
            self.stale_threshold,
            SystemTime::now(),
        );
    }
}

pub fn artifact_kind_label(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Rust => "Rust",
        ArtifactKind::Node => "Node",
        ArtifactKind::Gradle => "Gradle",
        ArtifactKind::Python => "Python",
        ArtifactKind::CocoaPods => "CocoaPods",
        ArtifactKind::NextNuxt => "Next/Nuxt",
    }
}

pub fn stale_threshold_label(threshold: StaleThreshold) -> &'static str {
    match threshold {
        StaleThreshold::OneDay => "1d",
        StaleThreshold::SevenDays => "7d",
        StaleThreshold::ThirtyDays => "30d",
        StaleThreshold::NinetyDays => "90d",
        StaleThreshold::All => "All",
    }
}

fn next_stale_threshold(threshold: StaleThreshold) -> StaleThreshold {
    match threshold {
        StaleThreshold::OneDay => StaleThreshold::SevenDays,
        StaleThreshold::SevenDays => StaleThreshold::ThirtyDays,
        StaleThreshold::ThirtyDays => StaleThreshold::NinetyDays,
        StaleThreshold::NinetyDays => StaleThreshold::All,
        StaleThreshold::All => StaleThreshold::OneDay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_threshold_cycle_preserves_existing_order() {
        let mut threshold = StaleThreshold::OneDay;
        for expected in [
            StaleThreshold::SevenDays,
            StaleThreshold::ThirtyDays,
            StaleThreshold::NinetyDays,
            StaleThreshold::All,
            StaleThreshold::OneDay,
        ] {
            threshold = next_stale_threshold(threshold);
            assert_eq!(threshold, expected);
        }
    }
}
