//! Opaque, observation-only review of one fully admitted Rust-target plan.
//!
//! The retained planner capability is deliberately inaccessible outside the
//! core. This module exposes only a freshly revalidated, bounded observation
//! and consuming release; it cannot approve, persist, claim, or execute.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use thiserror::Error;

use crate::domain::{
    CandidateAction, CandidateCategory, CandidateId, CleanupMode, PlanWarning, SafetyTier, ScanId,
};
use crate::planner::TrustedReviewedCleanupPlan;

use super::snapshot_review::SnapshotReviewOwner;

const RUST_TARGET_RULE_ID: &str = "developer.rust.target";
const RUST_TARGET_RULE_REVISION: u32 = 2;

/// Exact immutable plan facts safe to present while the opaque review remains
/// current. The path is an observation only and cannot be supplied back to a
/// planner or executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustTargetPlanReviewInfo {
    pub plan_id: String,
    pub source_scan_id: ScanId,
    pub candidate_id: CandidateId,
    pub rule_id: String,
    pub rule_revision: u32,
    pub category: CandidateCategory,
    pub mode: CleanupMode,
    pub safety: SafetyTier,
    pub action: CandidateAction,
    pub estimated_bytes: u64,
    pub warnings: Vec<PlanWarning>,
    pub created_at: SystemTime,
    pub effective_expires_at: SystemTime,
    pub schedule_eligible: bool,
    pub item_count: u16,
    pub path_count: u16,
    pub path: PathBuf,
}

/// Stable path-free failure taxonomy for app-facing plan review.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum RustTargetPlanReviewError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the supplied snapshot review belongs to another engine")]
    WrongEngine,
    #[error("the exact parent snapshot review is released or expired")]
    ParentReviewUnavailable,
    #[error("the exact Rust-target plan review expired")]
    ReviewExpired,
    #[error("the exact Rust-target candidate is unavailable for review")]
    CandidateUnavailable,
    #[error("direct Cargo enrollment is required before this plan can be reviewed")]
    CargoNotEnrolled,
    #[error("Cargo or rustc is active")]
    ActiveProcesses,
    #[error("the Rust-target plan evidence changed during review")]
    ChangedDuringReview,
    #[error("Rust-target plan review is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("the bounded plan-review operation exceeded its resource budget")]
    BudgetExceeded,
    #[error("the plan-review store is temporarily busy")]
    Busy,
    #[error("the plan-review store is unsafe")]
    UnsafeStorage,
    #[error("the plan-review store is corrupt")]
    CorruptData,
    #[error("the plan-review store is unavailable")]
    Unavailable,
    #[error("the plan-review state is internally unavailable")]
    InternalState,
}

/// Non-cloneable retained reviewed-plan capability. Public operations are
/// observation and release only.
#[must_use = "retain the plan review while presenting its exact observation"]
pub struct RustTargetPlanReview {
    reviewed: TrustedReviewedCleanupPlan,
    info: RustTargetPlanReviewInfo,
    parent_review_expires_at: SystemTime,
    parent_review_live: Arc<AtomicBool>,
}

/// Opaque, non-cloneable proof that one exact parent review admitted a
/// candidate before expensive live discovery began.
pub struct RustTargetPlanReviewAdmission {
    pub(super) owner: Arc<SnapshotReviewOwner>,
    pub(super) parent_session_identity: u64,
    pub(super) parent_review_live: Arc<AtomicBool>,
    pub(super) source_scan_id: ScanId,
    pub(super) candidate_id: CandidateId,
    pub(super) parent_review_expires_at: SystemTime,
}

/// Opaque, non-cloneable reviewed result that still requires the exact parent
/// review to pass a post-work validation before it can be presented.
pub struct PendingRustTargetPlanReview {
    pub(super) owner: Arc<SnapshotReviewOwner>,
    pub(super) parent_session_identity: u64,
    pub(super) parent_review_live: Arc<AtomicBool>,
    pub(super) source_scan_id: ScanId,
    pub(super) candidate_id: CandidateId,
    pub(super) parent_review_expires_at: SystemTime,
    pub(super) reviewed: TrustedReviewedCleanupPlan,
}

/// Opaque pending result whose exact parent was revalidated after preparation.
pub struct ValidatedPendingRustTargetPlanReview {
    pub(super) candidate_id: CandidateId,
    pub(super) parent_review_expires_at: SystemTime,
    pub(super) parent_review_live: Arc<AtomicBool>,
    pub(super) reviewed: TrustedReviewedCleanupPlan,
}

impl RustTargetPlanReview {
    pub(crate) fn new(
        reviewed: TrustedReviewedCleanupPlan,
        candidate_id: &CandidateId,
        parent_review_expires_at: SystemTime,
        parent_review_live: Arc<AtomicBool>,
        observed_at: SystemTime,
    ) -> Result<Self, RustTargetPlanReviewError> {
        if !parent_review_live.load(Ordering::Acquire) {
            return Err(RustTargetPlanReviewError::ParentReviewUnavailable);
        }
        reviewed
            .revalidate_at(observed_at)
            .map_err(|error| match error {
                crate::planner::ExactPathPlanError::Expired => {
                    RustTargetPlanReviewError::ReviewExpired
                }
                _ => RustTargetPlanReviewError::ChangedDuringReview,
            })?;
        let plan = reviewed.plan();
        let [item] = plan.items() else {
            return Err(RustTargetPlanReviewError::InternalState);
        };
        let [path] = item.paths() else {
            return Err(RustTargetPlanReviewError::InternalState);
        };
        let expected_warnings = [
            PlanWarning::EstimatedBytesUnverified,
            PlanWarning::PermanentRemovalCannotBeUndone,
        ];
        if item.candidate_id() != candidate_id
            || item.rule().id().as_str() != RUST_TARGET_RULE_ID
            || item.rule().revision().get() != RUST_TARGET_RULE_REVISION
            || item.category() != CandidateCategory::DeveloperArtifact
            || plan.mode() != CleanupMode::PermanentSafe
            || item.safety() != SafetyTier::SafeRegenerable
            || item.action() != CandidateAction::RemoveKnownRegenerableContents
            || item.rule_marks_schedule_eligible()
            || plan.estimated_bytes() != item.estimated_bytes()
            || plan.warnings() != expected_warnings
        {
            return Err(RustTargetPlanReviewError::InternalState);
        }
        if observed_at >= parent_review_expires_at {
            return Err(RustTargetPlanReviewError::ParentReviewUnavailable);
        }
        let child_expires_at = plan.expires_at().min(reviewed.effective_expires_at());
        if observed_at >= child_expires_at {
            return Err(RustTargetPlanReviewError::ReviewExpired);
        }
        let effective_expires_at = child_expires_at.min(parent_review_expires_at);
        let info = RustTargetPlanReviewInfo {
            plan_id: plan.id().as_str().to_owned(),
            source_scan_id: plan.source_scan_id().clone(),
            candidate_id: item.candidate_id().clone(),
            rule_id: item.rule().id().as_str().to_owned(),
            rule_revision: item.rule().revision().get(),
            category: item.category(),
            mode: plan.mode(),
            safety: item.safety(),
            action: item.action(),
            estimated_bytes: plan.estimated_bytes(),
            warnings: plan.warnings().to_vec(),
            created_at: plan.created_at(),
            effective_expires_at,
            schedule_eligible: item.rule_marks_schedule_eligible(),
            item_count: 1,
            path_count: 1,
            path: path.clone(),
        };
        let completed_at = SystemTime::now();
        if !parent_review_live.load(Ordering::Acquire) || completed_at >= parent_review_expires_at {
            return Err(RustTargetPlanReviewError::ParentReviewUnavailable);
        }
        if completed_at >= effective_expires_at {
            return Err(RustTargetPlanReviewError::ReviewExpired);
        }
        Ok(Self {
            reviewed,
            info,
            parent_review_expires_at,
            parent_review_live,
        })
    }

    pub fn info(&self) -> Result<RustTargetPlanReviewInfo, RustTargetPlanReviewError> {
        self.info_at(SystemTime::now())
    }

    /// Cheap terminal probe for capacity management. This checks only frozen
    /// deadlines and exact-parent liveness; it performs no filesystem or
    /// persistence revalidation and grants no authority.
    pub fn terminal_error(&self) -> Option<RustTargetPlanReviewError> {
        self.terminal_error_at(SystemTime::now())
    }

    pub(crate) fn terminal_error_at(&self, now: SystemTime) -> Option<RustTargetPlanReviewError> {
        if now >= self.parent_review_expires_at || !self.parent_review_live.load(Ordering::Acquire)
        {
            return Some(RustTargetPlanReviewError::ParentReviewUnavailable);
        }
        if now >= self.info.effective_expires_at {
            return Some(RustTargetPlanReviewError::ReviewExpired);
        }
        None
    }

    pub(crate) fn info_at(
        &self,
        now: SystemTime,
    ) -> Result<RustTargetPlanReviewInfo, RustTargetPlanReviewError> {
        if let Some(error) = self.terminal_error_at(now) {
            return Err(error);
        }
        self.reviewed
            .revalidate_at(now)
            .map_err(|error| match error {
                crate::planner::ExactPathPlanError::Expired => {
                    RustTargetPlanReviewError::ReviewExpired
                }
                _ => RustTargetPlanReviewError::ChangedDuringReview,
            })?;
        Ok(self.info.clone())
    }

    pub fn release(self) {
        self.reviewed.release();
    }
}
