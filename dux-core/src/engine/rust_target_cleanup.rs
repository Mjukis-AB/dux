//! Path-free product result and failure types for one explicitly approved
//! deterministic Rust-target cleanup.
//!
//! These values are observations only. The authority remains the consumed
//! [`super::RustTargetPlanReview`] capability and never comes from a session
//! identifier, displayed path, candidate identifier, or result record.

use std::fmt;

use thiserror::Error;

use super::cleanup_history::{DurableCleanupSessionId, DurableCleanupSessionStatus};
use super::rust_target_plan_review::RustTargetPlanReview;
use super::task::PermanentSafeCleanupFailureKind;
use crate::cleanup::permanent_safe::PermanentSafeSessionSummary;
use crate::persistence::{CleanupSessionId, HistoryErrorKind, TerminalSessionStatus};
use crate::planner::{ExactPathApprovalError, ExactPathHandoffError};

const SESSION_ID_PREFIX: &str = "cleanup:rust-target:";

/// Path-free observation returned after one consumed plan review. Failed tasks
/// that retain ambiguous authority expose only the exact session identifier
/// and `Recovering`; durable history remains authoritative for final counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustTargetCleanupResult {
    session_id: DurableCleanupSessionId,
    status: DurableCleanupSessionStatus,
    removed_entries: u64,
    removed_logical_bytes: u64,
    verified_capacity_delta_bytes: Option<i64>,
}

impl RustTargetCleanupResult {
    pub const fn session_id(&self) -> &DurableCleanupSessionId {
        &self.session_id
    }

    pub const fn status(&self) -> DurableCleanupSessionStatus {
        self.status
    }

    pub const fn removed_entries(&self) -> u64 {
        self.removed_entries
    }

    pub const fn removed_logical_bytes(&self) -> u64 {
        self.removed_logical_bytes
    }

    pub const fn verified_capacity_delta_bytes(&self) -> Option<i64> {
        self.verified_capacity_delta_bytes
    }
}

/// Stable, path-free failure taxonomy for explicit Rust-target cleanup.
///
/// `OutcomeUnknown` is intentionally distinct: callers must observe durable
/// history and must never retry the effect automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum RustTargetCleanupError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the supplied plan review belongs to another engine")]
    WrongEngine,
    #[error("the exact parent snapshot review is released or expired")]
    ParentReviewUnavailable,
    #[error("the exact Rust-target plan review expired")]
    ReviewExpired,
    #[error("the Rust-target plan evidence changed before execution")]
    ChangedDuringReview,
    #[error("cleanup was cancelled before durable execution began")]
    CancelledBeforeStart,
    #[error("the cleanup operation exceeded its bounded resource budget")]
    BudgetExceeded,
    #[error("the engine task queue is full")]
    QueueFull,
    #[error("the cleanup store is temporarily busy")]
    Busy,
    #[error("the cleanup store is unsafe")]
    UnsafeStorage,
    #[error("the cleanup schema is incompatible")]
    IncompatibleSchema,
    #[error("the cleanup journal is corrupt")]
    CorruptData,
    #[error("the cleanup operation outcome is unknown")]
    OutcomeUnknown,
    #[error("the cleanup operation is unavailable")]
    Unavailable,
    #[error("the cleanup state is internally unavailable")]
    InternalState,
}

/// A rejected task start retains the exact opaque review so ownership,
/// queue-pressure, or cleanup-contention checks never consume user authority.
#[must_use = "inspect the failure or recover the unconsumed plan review"]
pub struct RustTargetCleanupStartFailure {
    error: RustTargetCleanupError,
    review: Box<RustTargetPlanReview>,
}

impl RustTargetCleanupStartFailure {
    pub(super) fn new(error: RustTargetCleanupError, review: RustTargetPlanReview) -> Self {
        Self {
            error,
            review: Box::new(review),
        }
    }

    pub const fn error(&self) -> RustTargetCleanupError {
        self.error
    }

    pub fn into_review(self) -> RustTargetPlanReview {
        *self.review
    }
}

impl fmt::Debug for RustTargetCleanupStartFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RustTargetCleanupStartFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for RustTargetCleanupStartFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for RustTargetCleanupStartFailure {}

pub(super) fn generate_session_id() -> Result<CleanupSessionId, RustTargetCleanupError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| RustTargetCleanupError::InternalState)?;
    let mut value = String::with_capacity(SESSION_ID_PREFIX.len() + random.len() * 2);
    value.push_str(SESSION_ID_PREFIX);
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").map_err(|_| RustTargetCleanupError::InternalState)?;
    }
    CleanupSessionId::new(value).map_err(|_| RustTargetCleanupError::InternalState)
}

pub(super) fn result(
    session_id: &CleanupSessionId,
    summary: PermanentSafeSessionSummary,
) -> Result<RustTargetCleanupResult, RustTargetCleanupError> {
    let session_id = DurableCleanupSessionId::new(session_id.as_str().to_owned())
        .ok_or(RustTargetCleanupError::InternalState)?;
    Ok(RustTargetCleanupResult {
        session_id,
        status: terminal_status(summary.terminal_status),
        removed_entries: summary.removed_entries,
        removed_logical_bytes: summary.removed_logical_bytes,
        verified_capacity_delta_bytes: summary.verified_capacity_delta_bytes,
    })
}

/// Preserve exact durable correlation when this process must retain ambiguous
/// claim/effect authority. Counts remain zero because the journal is the only
/// trustworthy source while recovery is pending.
pub(super) fn recovering_result(
    session_id: &CleanupSessionId,
) -> Result<RustTargetCleanupResult, RustTargetCleanupError> {
    let session_id = DurableCleanupSessionId::new(session_id.as_str().to_owned())
        .ok_or(RustTargetCleanupError::InternalState)?;
    Ok(RustTargetCleanupResult {
        session_id,
        status: DurableCleanupSessionStatus::Recovering,
        removed_entries: 0,
        removed_logical_bytes: 0,
        verified_capacity_delta_bytes: None,
    })
}

pub(super) fn map_handoff_error(error: ExactPathHandoffError) -> RustTargetCleanupError {
    match error {
        ExactPathHandoffError::Approval(error) => map_approval_error(error),
        ExactPathHandoffError::Journal(error) => map_history_error(error.kind),
        ExactPathHandoffError::RuleEvidence(_) => RustTargetCleanupError::ChangedDuringReview,
    }
}

pub(super) fn map_approval_error(error: ExactPathApprovalError) -> RustTargetCleanupError {
    match error {
        ExactPathApprovalError::Expired => RustTargetCleanupError::ReviewExpired,
        ExactPathApprovalError::Persistence(error) => map_history_error(error.kind),
        ExactPathApprovalError::AuthorizationMismatch
        | ExactPathApprovalError::Authorization(_)
        | ExactPathApprovalError::Plan(_) => RustTargetCleanupError::ChangedDuringReview,
    }
}

const fn map_history_error(kind: HistoryErrorKind) -> RustTargetCleanupError {
    match kind {
        HistoryErrorKind::QueryLimitExceeded => RustTargetCleanupError::BudgetExceeded,
        HistoryErrorKind::Busy => RustTargetCleanupError::Busy,
        HistoryErrorKind::UnsafeStorage => RustTargetCleanupError::UnsafeStorage,
        HistoryErrorKind::IncompatibleSchema => RustTargetCleanupError::IncompatibleSchema,
        HistoryErrorKind::CorruptData => RustTargetCleanupError::CorruptData,
        HistoryErrorKind::OutcomeUnknown => RustTargetCleanupError::OutcomeUnknown,
        HistoryErrorKind::DatabaseUnavailable => RustTargetCleanupError::Unavailable,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::InternalState => RustTargetCleanupError::InternalState,
    }
}

pub(super) const fn failure_kind(error: RustTargetCleanupError) -> PermanentSafeCleanupFailureKind {
    match error {
        RustTargetCleanupError::ParentReviewUnavailable => {
            PermanentSafeCleanupFailureKind::ParentReviewUnavailable
        }
        RustTargetCleanupError::ReviewExpired => PermanentSafeCleanupFailureKind::ReviewExpired,
        RustTargetCleanupError::ChangedDuringReview => {
            PermanentSafeCleanupFailureKind::ChangedDuringReview
        }
        RustTargetCleanupError::BudgetExceeded | RustTargetCleanupError::QueueFull => {
            PermanentSafeCleanupFailureKind::BudgetExceeded
        }
        RustTargetCleanupError::Busy => PermanentSafeCleanupFailureKind::Busy,
        RustTargetCleanupError::UnsafeStorage => PermanentSafeCleanupFailureKind::UnsafeStorage,
        RustTargetCleanupError::IncompatibleSchema => {
            PermanentSafeCleanupFailureKind::IncompatibleSchema
        }
        RustTargetCleanupError::CorruptData => PermanentSafeCleanupFailureKind::CorruptData,
        RustTargetCleanupError::OutcomeUnknown => PermanentSafeCleanupFailureKind::OutcomeUnknown,
        RustTargetCleanupError::Unavailable => PermanentSafeCleanupFailureKind::Unavailable,
        RustTargetCleanupError::Closed
        | RustTargetCleanupError::WrongEngine
        | RustTargetCleanupError::CancelledBeforeStart
        | RustTargetCleanupError::InternalState => PermanentSafeCleanupFailureKind::InternalState,
    }
}

const fn terminal_status(status: TerminalSessionStatus) -> DurableCleanupSessionStatus {
    match status {
        TerminalSessionStatus::Completed => DurableCleanupSessionStatus::Completed,
        TerminalSessionStatus::PartiallyCompleted => {
            DurableCleanupSessionStatus::PartiallyCompleted
        }
        TerminalSessionStatus::Failed => DurableCleanupSessionStatus::Failed,
        TerminalSessionStatus::Cancelled => DurableCleanupSessionStatus::Cancelled,
        TerminalSessionStatus::Interrupted => DurableCleanupSessionStatus::Interrupted,
        TerminalSessionStatus::Rejected => DurableCleanupSessionStatus::Rejected,
        TerminalSessionStatus::DryRun => DurableCleanupSessionStatus::DryRun,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_session_ids_are_bounded_path_free_and_distinct() {
        let first = generate_session_id().unwrap();
        let second = generate_session_id().unwrap();
        assert_ne!(first, second);
        for id in [first, second] {
            assert!(id.as_str().starts_with(SESSION_ID_PREFIX));
            assert_eq!(id.as_str().len(), SESSION_ID_PREFIX.len() + 32);
            assert!(
                id.as_str()[SESSION_ID_PREFIX.len()..]
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            );
        }
    }
}
