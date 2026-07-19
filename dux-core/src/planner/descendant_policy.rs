//! Bounded evidence for exact protected/excluded descendant selectors.
//!
//! This is deliberately an observation-only seam. It does not enumerate an
//! arbitrary directory tree, authorize recursive deletion, or cross an FFI
//! boundary. A future permanent-safe executor must still enumerate explicit
//! child effects with retained descriptor-relative handles.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::path_validation::{
    CanonicalPathError, CanonicalPathSnapshot, CanonicalScanRoot, LexicalPathError,
    capture_path_snapshot, capture_scan_root, validate_cleanup_path, validate_scan_root,
};

pub(crate) const DESCENDANT_POLICY_PROOF_REVISION: u32 = 1;
const MAX_POLICY_ENTRIES: usize = 32;
const MAX_POLICY_PATH_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PolicyKind {
    Protected,
    Excluded,
}

struct PolicyEntry {
    relative_path: PathBuf,
    kind: PolicyKind,
    snapshot: CanonicalPathSnapshot,
}

/// Exact selector observations retained for a future rule-boundary join.
///
/// The token is intentionally not `Clone`, serializable, path-readable, or
/// convertible to a plan/effect. Missing selectors, symlinks, multiply-linked
/// entries, duplicate selectors, and any identity change fail closed.
#[must_use = "descendant policy evidence must be revalidated before use"]
pub(crate) struct DescendantPolicyWitness {
    revision: u32,
    root: CanonicalScanRoot,
    entries: Vec<PolicyEntry>,
}

#[derive(Debug, Error)]
pub(crate) enum DescendantPolicyError {
    #[error("descendant policy proof revision is unsupported")]
    UnsupportedRevision,
    #[error("descendant policy contains too many exact selectors")]
    TooManyEntries,
    #[error("descendant policy selector is empty or exceeds the bounded path size")]
    InvalidSelector,
    #[error("descendant policy selectors overlap or are duplicated")]
    OverlappingSelectors,
    #[error("a required descendant policy selector is missing")]
    MissingSelector,
    #[error("a descendant policy entry is multiply linked")]
    MultiplyLinked,
    #[error("descendant policy evidence changed since capture")]
    ChangedSinceCapture,
    #[error("the current deterministic rule does not declare descendant selectors")]
    UnexpectedSelectors,
    #[error(transparent)]
    Lexical(#[from] LexicalPathError),
    #[error(transparent)]
    Filesystem(#[from] CanonicalPathError),
}

impl DescendantPolicyWitness {
    /// Capture exact, rule-relative selectors from a code-owned scan root.
    ///
    /// Selector overlap is rejected because a future executor must not have
    /// to infer precedence between protected and excluded subtrees.
    pub(crate) fn capture(
        root: &CanonicalScanRoot,
        protected: &[String],
        excluded: &[String],
    ) -> Result<Self, DescendantPolicyError> {
        let total = protected.len().saturating_add(excluded.len());
        if total > MAX_POLICY_ENTRIES {
            return Err(DescendantPolicyError::TooManyEntries);
        }

        let lexical_root = validate_scan_root(root.requested_path())?;
        let mut selectors = BTreeSet::new();
        let mut entries = Vec::with_capacity(total);
        for (kind, values) in [
            (PolicyKind::Protected, protected),
            (PolicyKind::Excluded, excluded),
        ] {
            for value in values {
                if value.is_empty() || value.len() > MAX_POLICY_PATH_BYTES {
                    return Err(DescendantPolicyError::InvalidSelector);
                }
                let relative = Path::new(value);
                if relative.is_absolute()
                    || relative
                        .components()
                        .any(|component| !matches!(component, std::path::Component::Normal(_)))
                {
                    return Err(DescendantPolicyError::InvalidSelector);
                }
                let absolute = root.requested_path().join(relative);
                let lexical = validate_cleanup_path(&lexical_root, &absolute)?;
                let normalized = lexical.relative_to_scan_root().to_path_buf();
                if !selectors.insert(normalized.clone())
                    || selectors.iter().any(|other| {
                        other != &normalized
                            && (other.starts_with(&normalized) || normalized.starts_with(other))
                    })
                {
                    return Err(DescendantPolicyError::OverlappingSelectors);
                }
                let snapshot =
                    capture_path_snapshot(root, lexical).map_err(|error| match error {
                        CanonicalPathError::Missing { .. } => {
                            DescendantPolicyError::MissingSelector
                        }
                        other => DescendantPolicyError::Filesystem(other),
                    })?;
                if matches!(
                    snapshot.target_kind(),
                    crate::path_validation::FilesystemEntryKind::RegularFile
                ) && snapshot.hard_link_count() > 1
                {
                    return Err(DescendantPolicyError::MultiplyLinked);
                }
                entries.push(PolicyEntry {
                    relative_path: normalized,
                    kind,
                    snapshot,
                });
            }
        }
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

        Ok(Self {
            revision: DESCENDANT_POLICY_PROOF_REVISION,
            root: root.clone(),
            entries,
        })
    }

    pub(crate) fn revalidate(&self) -> Result<(), DescendantPolicyError> {
        if self.revision != DESCENDANT_POLICY_PROOF_REVISION {
            return Err(DescendantPolicyError::UnsupportedRevision);
        }
        let lexical_root = validate_scan_root(self.root.requested_path())?;
        let current_root = capture_scan_root(lexical_root.clone())?;
        if current_root != self.root {
            return Err(DescendantPolicyError::ChangedSinceCapture);
        }
        for entry in &self.entries {
            let absolute = self.root.requested_path().join(&entry.relative_path);
            let lexical = validate_cleanup_path(&lexical_root, &absolute)?;
            let current =
                capture_path_snapshot(&self.root, lexical).map_err(|error| match error {
                    CanonicalPathError::Missing { .. }
                    | CanonicalPathError::ChangedDuringValidation { .. }
                    | CanonicalPathError::SymlinkOrReparsePoint { .. }
                    | CanonicalPathError::CanonicalPathMismatch { .. } => {
                        DescendantPolicyError::ChangedSinceCapture
                    }
                    other => DescendantPolicyError::Filesystem(other),
                })?;
            if current != entry.snapshot
                || (matches!(
                    current.target_kind(),
                    crate::path_validation::FilesystemEntryKind::RegularFile
                ) && current.hard_link_count() > 1)
            {
                return Err(DescendantPolicyError::ChangedSinceCapture);
            }
            let _ = entry.kind;
        }
        Ok(())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn release(self) {}
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::fs::{create_dir_all, hard_link, rename, write};

    use tempfile::tempdir;

    use super::*;

    fn root() -> (tempfile::TempDir, CanonicalScanRoot) {
        let directory = tempdir().unwrap();
        let path = std::fs::canonicalize(directory.path()).unwrap();
        let canonical = capture_scan_root(validate_scan_root(&path).unwrap()).unwrap();
        (directory, canonical)
    }

    #[test]
    fn captures_and_revalidates_exact_component_paths() {
        let (_directory, root) = root();
        create_dir_all(root.requested_path().join(".git")).unwrap();
        write(root.requested_path().join(".git/config"), b"x").unwrap();
        write(root.requested_path().join(".github"), b"x").unwrap();
        let witness =
            DescendantPolicyWitness::capture(&root, &[".git/config".into()], &[".github".into()])
                .unwrap();
        witness.revalidate().unwrap();
    }

    #[test]
    fn replacement_and_removal_fail_closed() {
        let (directory, root) = root();
        let path = directory.path().join("cache");
        write(&path, b"old").unwrap();
        let witness = DescendantPolicyWitness::capture(&root, &["cache".into()], &[]).unwrap();
        rename(&path, directory.path().join("cache-old")).unwrap();
        write(&path, b"new").unwrap();
        assert!(matches!(
            witness.revalidate(),
            Err(DescendantPolicyError::ChangedSinceCapture)
        ));
    }

    #[test]
    fn rejects_overlap_missing_and_multiply_linked_entries() {
        let (directory, root) = root();
        create_dir_all(directory.path().join("parent/child")).unwrap();
        write(directory.path().join("parent/child/file"), b"x").unwrap();
        let overlap =
            DescendantPolicyWitness::capture(&root, &["parent".into()], &["parent/child".into()]);
        assert!(matches!(
            &overlap,
            Err(DescendantPolicyError::OverlappingSelectors)
        ));
        assert!(matches!(
            DescendantPolicyWitness::capture(&root, &["missing".into()], &[]),
            Err(DescendantPolicyError::MissingSelector)
        ));
        let linked = directory.path().join("linked");
        write(&linked, b"x").unwrap();
        hard_link(&linked, directory.path().join("linked-2")).unwrap();
        assert!(matches!(
            DescendantPolicyWitness::capture(&root, &["linked".into()], &[]),
            Err(DescendantPolicyError::MultiplyLinked)
        ));
    }
}
