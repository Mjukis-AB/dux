use crate::{DiskTree, ScanCoverage};

use super::FreshScanFacts;

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
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "consumed by the staged durable engine scan task in the next M2 slice"
        )
    )]
    fresh_facts: Option<FreshScanFacts>,
}

impl ScanOutcome {
    pub(super) fn cancelled(tree: DiskTree, coverage: ScanCoverage) -> Self {
        Self {
            tree,
            coverage,
            termination: ScanTermination::Cancelled,
            fresh_facts: None,
        }
    }

    pub(super) fn failed(tree: DiskTree, coverage: ScanCoverage) -> Self {
        Self {
            tree,
            coverage,
            termination: ScanTermination::Failed,
            fresh_facts: None,
        }
    }

    pub(super) fn completed(
        tree: DiskTree,
        coverage: ScanCoverage,
        fresh_facts: FreshScanFacts,
    ) -> Self {
        Self {
            tree,
            coverage,
            termination: ScanTermination::Completed,
            fresh_facts: Some(fresh_facts),
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

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "consumed by the staged durable engine scan task in the next M2 slice"
        )
    )]
    pub(crate) fn into_completed_artifact(self) -> Option<CompletedScanArtifact> {
        if self.termination != ScanTermination::Completed {
            return None;
        }
        Some(CompletedScanArtifact {
            tree: self.tree,
            coverage: self.coverage,
            facts: self.fresh_facts?,
        })
    }
}

/// Type-state witness that only a successful fresh traversal can construct.
/// Persistence consumes this rather than accepting an arbitrary `DiskTree`.
pub(crate) struct CompletedScanArtifact {
    tree: DiskTree,
    coverage: ScanCoverage,
    facts: FreshScanFacts,
}

impl CompletedScanArtifact {
    pub(crate) fn parts(&self) -> (&DiskTree, &FreshScanFacts, &ScanCoverage) {
        (&self.tree, &self.facts, &self.coverage)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn non_completed_outcomes_cannot_mint_a_fresh_artifact() {
        let root = || {
            DiskTree::new(PathBuf::from(if cfg!(windows) {
                r"C:\fixture"
            } else {
                "/fixture"
            }))
        };
        for outcome in [
            ScanOutcome::cancelled(root(), ScanCoverage::unknown()),
            ScanOutcome::failed(root(), ScanCoverage::unknown()),
        ] {
            assert!(outcome.into_completed_artifact().is_none());
        }
    }
}
