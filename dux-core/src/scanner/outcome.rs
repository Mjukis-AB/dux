use crate::{DiskTree, ScanCoverage};

/// Definitive terminal state returned by the scanner worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanTermination {
    Completed,
    Cancelled,
    Failed,
}

/// One bounded traversal result.
///
/// Progress messages are advisory. This value is the authoritative result and
/// keeps the tree paired with the coverage facts that qualify its totals.
#[derive(Debug)]
pub struct ScanOutcome {
    tree: DiskTree,
    coverage: ScanCoverage,
    termination: ScanTermination,
}

impl ScanOutcome {
    pub(super) fn new(
        tree: DiskTree,
        coverage: ScanCoverage,
        termination: ScanTermination,
    ) -> Self {
        Self {
            tree,
            coverage,
            termination,
        }
    }

    pub fn tree(&self) -> &DiskTree {
        &self.tree
    }

    /// Consume the outcome without allowing the tree to become detached from
    /// the coverage and terminal facts that qualify its totals.
    pub fn into_parts(self) -> (DiskTree, ScanCoverage, ScanTermination) {
        (self.tree, self.coverage, self.termination)
    }

    pub fn coverage(&self) -> &ScanCoverage {
        &self.coverage
    }

    pub fn termination(&self) -> ScanTermination {
        self.termination
    }
}
