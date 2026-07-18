//! Journal-fenced admission for a reviewed Explorer Trash request.
//!
//! This module intentionally stops immediately before the platform effect. It
//! owns the one-shot evidence and the durable journal receipt so a future
//! macOS adapter can be added without creating a second cleanup authority.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::engine::SnapshotReviewTrashTarget;
use crate::path_validation::{
    CanonicalPathError, capture_scan_root, capture_trash_path_snapshot, validate_cleanup_path,
    validate_scan_root,
};
use crate::persistence::{
    CleanupJournalClaim, CleanupSessionId, EffectStartReceipt, HistoryErrorKind, StoreCoordinator,
    ValidationOutcome,
};

/// The only failures returned by the core Trash admission boundary. Paths are
/// deliberately absent so a caller cannot treat an error string as an effect
/// instruction or accidentally display a sensitive path outside the review.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum TrashAdmissionError {
    #[error("the reviewed Trash target is no longer available")]
    TargetUnavailable,
    #[error("the reviewed Trash target changed before execution")]
    TargetChanged,
    #[error("the reviewed Trash target has an unsupported kind")]
    UnsupportedTargetKind,
    #[error("the reviewed Trash target is not bound to the planned journal path")]
    TargetNotBound,
    #[error("the cleanup journal rejected the Trash admission: {0:?}")]
    Journal(HistoryErrorKind),
}

/// A non-cloneable, one-shot capability that proves journal ordering and a
/// fresh no-follow target witness. It grants no platform effect by itself.
pub(crate) struct TrashExecutionAdmission {
    target: SnapshotReviewTrashTarget,
    claim: CleanupJournalClaim,
    receipt: EffectStartReceipt,
}

impl TrashExecutionAdmission {
    /// Claim one planned journal session and admit exactly one reviewed path.
    /// The journal enters `effect_started`, but this function never invokes an
    /// operating-system or platform Trash primitive.
    pub(crate) fn begin(
        store: &Arc<StoreCoordinator>,
        session_id: &CleanupSessionId,
        target: SnapshotReviewTrashTarget,
        item_ordinal: usize,
        path_ordinal: usize,
        observed_at: SystemTime,
        lock_timeout: Duration,
    ) -> Result<Self, TrashAdmissionError> {
        let lease = store
            .acquire_cleanup_journal_lease(lock_timeout)
            .map_err(|error| TrashAdmissionError::Journal(error.kind))?;
        let claim = lease
            .claim_planned(session_id, observed_at)
            .map_err(|error| TrashAdmissionError::Journal(error.kind()))?;
        Self::from_claim(claim, target, item_ordinal, path_ordinal, observed_at)
    }

    /// Admit against an already fenced claim. This narrow constructor keeps
    /// tests and future engine orchestration from bypassing the same ordering
    /// rules used by [`Self::begin`].
    pub(crate) fn from_claim(
        claim: CleanupJournalClaim,
        target: SnapshotReviewTrashTarget,
        item_ordinal: usize,
        path_ordinal: usize,
        observed_at: SystemTime,
    ) -> Result<Self, TrashAdmissionError> {
        if let Err(error) = claim.validate_planned_path(
            item_ordinal,
            path_ordinal,
            target.snapshot.requested_path(),
        ) {
            if error.kind == HistoryErrorKind::InvalidTransition {
                // A valid row with a mismatched path is a rejected request,
                // not an abandoned running journal. Invalid ordinals or a
                // failed write remain journal errors and are never guessed.
                claim
                    .begin_validation(item_ordinal, path_ordinal)
                    .map_err(|journal_error| TrashAdmissionError::Journal(journal_error.kind))?;
                claim
                    .finish_validation(
                        item_ordinal,
                        path_ordinal,
                        ValidationOutcome::Rejected,
                        Some("trash_target_not_bound"),
                        observed_at,
                    )
                    .map_err(|journal_error| TrashAdmissionError::Journal(journal_error.kind))?;
                return Err(TrashAdmissionError::TargetNotBound);
            }
            return Err(TrashAdmissionError::Journal(error.kind));
        }
        claim
            .begin_validation(item_ordinal, path_ordinal)
            .map_err(|error| TrashAdmissionError::Journal(error.kind))?;

        if let Err(error) = revalidate_target(&target) {
            let outcome = match error {
                TrashAdmissionError::TargetChanged => ValidationOutcome::ChangedSincePlan,
                TrashAdmissionError::UnsupportedTargetKind => ValidationOutcome::Rejected,
                TrashAdmissionError::TargetUnavailable | TrashAdmissionError::Journal(_) => {
                    ValidationOutcome::Unavailable
                }
                TrashAdmissionError::TargetNotBound => ValidationOutcome::Rejected,
            };
            claim
                .finish_validation(
                    item_ordinal,
                    path_ordinal,
                    outcome,
                    Some(validation_error_category(error)),
                    observed_at,
                )
                .map_err(|journal_error| TrashAdmissionError::Journal(journal_error.kind))?;
            return Err(error);
        }

        let receipt = claim
            .mark_effect_started(item_ordinal, path_ordinal, observed_at)
            .map_err(|error| TrashAdmissionError::Journal(error.kind))?;
        if let Err(error) = claim.revalidate_effect_receipt(&receipt) {
            // The receipt was durable, but no platform call has happened. Use
            // the journal's explicit pre-effect cancellation transition rather
            // than leaving an effect-started row that could be mistaken for a
            // call in a later recovery pass.
            claim
                .cancel_effect_before_call(&receipt, observed_at)
                .map_err(|cancel_error| TrashAdmissionError::Journal(cancel_error.kind))?;
            return Err(TrashAdmissionError::Journal(error.kind));
        }

        Ok(Self {
            target,
            claim,
            receipt,
        })
    }

    pub(crate) fn node_id(&self) -> u64 {
        self.target.node_id
    }

    pub(crate) fn target_kind(&self) -> crate::path_validation::TrashTargetKind {
        self.target.snapshot.target_kind()
    }

    /// Cancel the durable intent without invoking a platform effect. The
    /// consuming receiver enforces that this admission cannot be reused.
    pub(crate) fn cancel_before_effect(
        self,
        completed_at: SystemTime,
    ) -> Result<(), TrashAdmissionError> {
        self.claim
            .cancel_effect_before_call(&self.receipt, completed_at)
            .map_err(|error| TrashAdmissionError::Journal(error.kind))
    }
}

fn revalidate_target(target: &SnapshotReviewTrashTarget) -> Result<(), TrashAdmissionError> {
    let expected = &target.snapshot;
    let lexical_root = validate_scan_root(expected.scan_root())
        .map_err(|_| TrashAdmissionError::TargetUnavailable)?;
    let live_root = capture_scan_root(lexical_root.clone()).map_err(map_path_error)?;
    let requested_path = live_root.canonical_path().join(expected.relative_path());
    let lexical_target = validate_cleanup_path(&lexical_root, &requested_path)
        .map_err(|_| TrashAdmissionError::TargetUnavailable)?;
    let live = capture_trash_path_snapshot(&live_root, lexical_target).map_err(map_path_error)?;

    if live.scan_root() != expected.scan_root()
        || live.object_path() != expected.object_path()
        || live.relative_path() != expected.relative_path()
        || live.target_identity() != expected.target_identity()
        || live.target_kind() != expected.target_kind()
        || live.hard_link_count() != expected.hard_link_count()
        || live.ancestors() != expected.ancestors()
    {
        return Err(TrashAdmissionError::TargetChanged);
    }
    Ok(())
}

fn map_path_error(error: CanonicalPathError) -> TrashAdmissionError {
    match error {
        CanonicalPathError::UnsupportedPlatform => TrashAdmissionError::TargetUnavailable,
        CanonicalPathError::UnsupportedTargetKind => TrashAdmissionError::UnsupportedTargetKind,
        CanonicalPathError::ChangedDuringValidation { .. } => TrashAdmissionError::TargetChanged,
        CanonicalPathError::Missing { .. }
        | CanonicalPathError::AccessDenied { .. }
        | CanonicalPathError::Io { .. }
        | CanonicalPathError::ScanRootNotDirectory
        | CanonicalPathError::SymlinkOrReparsePoint { .. }
        | CanonicalPathError::NonDirectoryAncestor { .. }
        | CanonicalPathError::IdentityUnavailable { .. }
        | CanonicalPathError::CrossVolume { .. }
        | CanonicalPathError::MismatchedScanRoot
        | CanonicalPathError::CanonicalizationFailed { .. }
        | CanonicalPathError::CanonicalEscapesScanRoot
        | CanonicalPathError::CanonicalPathMismatch { .. }
        | CanonicalPathError::BoundaryTooDeep { .. } => TrashAdmissionError::TargetUnavailable,
    }
}

fn validation_error_category(error: TrashAdmissionError) -> &'static str {
    match error {
        TrashAdmissionError::TargetUnavailable => "trash_target_unavailable",
        TrashAdmissionError::TargetChanged => "trash_target_changed",
        TrashAdmissionError::UnsupportedTargetKind => "trash_target_unsupported",
        TrashAdmissionError::TargetNotBound => "trash_target_not_bound",
        TrashAdmissionError::Journal(_) => "trash_journal_rejected",
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::path_validation::{
        TrashTargetKind, capture_scan_root, capture_trash_path_snapshot, validate_cleanup_path,
        validate_scan_root,
    };

    fn reviewed_file_target(temp: &TempDir) -> SnapshotReviewTrashTarget {
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        let item = root.join("item.txt");
        fs::write(&item, b"review me").unwrap();
        let lexical_root = validate_scan_root(&root).unwrap();
        let live_root = capture_scan_root(lexical_root.clone()).unwrap();
        let lexical_item = validate_cleanup_path(&lexical_root, &item).unwrap();
        let snapshot = capture_trash_path_snapshot(&live_root, lexical_item).unwrap();
        assert_eq!(snapshot.target_kind(), TrashTargetKind::RegularFile);
        SnapshotReviewTrashTarget {
            node_id: 7,
            snapshot,
        }
    }

    #[test]
    fn revalidation_rejects_replaced_object_identity() {
        let temp = TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let target = reviewed_file_target(&temp);
        let path = target.snapshot.object_path().to_path_buf();
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"replacement").unwrap();

        assert_eq!(
            revalidate_target(&target),
            Err(TrashAdmissionError::TargetChanged)
        );
    }

    #[test]
    fn revalidation_rejects_missing_object_without_returning_a_path() {
        let temp = TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let target = reviewed_file_target(&temp);
        fs::remove_file(target.snapshot.object_path()).unwrap();

        assert_eq!(
            revalidate_target(&target),
            Err(TrashAdmissionError::TargetUnavailable)
        );
        assert_eq!(
            TrashAdmissionError::TargetUnavailable.to_string(),
            "the reviewed Trash target is no longer available"
        );
    }
}
