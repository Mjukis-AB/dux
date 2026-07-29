//! Path-free product result and failure types for one deterministic
//! Rust-target dry run.
//!
//! A dry run consumes the same opaque reviewed-plan capability as the proposed
//! permanent-safe operation, but the admitted task never receives an effect
//! driver or an effect-capable witness. These values are observations only.

use std::fmt;

use thiserror::Error;

use super::cleanup_history::{DurableCleanupSessionId, DurableCleanupSessionStatus};
use super::rust_target_plan_review::RustTargetPlanReview;
use super::task::RustTargetDryRunFailureKind;
use crate::persistence::{CleanupSessionId, HistoryErrorKind, TerminalSessionStatus};

const SESSION_ID_PREFIX: &str = "cleanup:rust-target-dry-run:";

/// Path-free observation returned after one consumed Rust-target dry run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustTargetDryRunResult {
    session_id: DurableCleanupSessionId,
    status: DurableCleanupSessionStatus,
}

impl RustTargetDryRunResult {
    pub const fn session_id(&self) -> &DurableCleanupSessionId {
        &self.session_id
    }

    pub const fn status(&self) -> DurableCleanupSessionStatus {
        self.status
    }
}

/// Stable, path-free failure taxonomy for an effect-free Rust-target dry run.
///
/// `HistoryUnresolved` concerns only the durable observation record. The
/// filesystem outcome is always known: a dry-run task has no mutation
/// capability and therefore cannot leave an unknown effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum RustTargetDryRunError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the supplied plan review belongs to another engine")]
    WrongEngine,
    #[error("the exact parent snapshot review is released or expired")]
    ParentReviewUnavailable,
    #[error("the exact Rust-target plan review expired")]
    ReviewExpired,
    #[error("the Rust-target plan evidence changed before or during validation")]
    ChangedDuringReview,
    #[error("the dry run was cancelled before durable validation began")]
    CancelledBeforeStart,
    #[error("the dry run exceeded its bounded resource budget")]
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
    #[error("the dry-run history outcome could not be reconciled")]
    HistoryUnresolved,
    #[error("the dry run is unavailable")]
    Unavailable,
    #[error("the dry-run state is internally unavailable")]
    InternalState,
}

/// A rejected task start retains the exact opaque review. Once task admission
/// succeeds, the review is consumed exactly once and cannot be replayed.
#[must_use = "inspect the failure or recover the unconsumed plan review"]
pub struct RustTargetDryRunStartFailure {
    error: RustTargetDryRunError,
    review: Box<RustTargetPlanReview>,
}

impl RustTargetDryRunStartFailure {
    pub(super) fn new(error: RustTargetDryRunError, review: RustTargetPlanReview) -> Self {
        Self {
            error,
            review: Box::new(review),
        }
    }

    pub const fn error(&self) -> RustTargetDryRunError {
        self.error
    }

    pub fn into_review(self) -> RustTargetPlanReview {
        *self.review
    }
}

impl fmt::Debug for RustTargetDryRunStartFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RustTargetDryRunStartFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for RustTargetDryRunStartFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for RustTargetDryRunStartFailure {}

pub(super) fn generate_session_id() -> Result<CleanupSessionId, RustTargetDryRunError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| RustTargetDryRunError::InternalState)?;
    let mut value = String::with_capacity(SESSION_ID_PREFIX.len() + random.len() * 2);
    value.push_str(SESSION_ID_PREFIX);
    for byte in random {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").map_err(|_| RustTargetDryRunError::InternalState)?;
    }
    CleanupSessionId::new(value).map_err(|_| RustTargetDryRunError::InternalState)
}

pub(super) fn result(
    session_id: &CleanupSessionId,
    status: TerminalSessionStatus,
) -> Result<RustTargetDryRunResult, RustTargetDryRunError> {
    let session_id = DurableCleanupSessionId::new(session_id.as_str().to_owned())
        .ok_or(RustTargetDryRunError::InternalState)?;
    let status = terminal_status(status);
    Ok(RustTargetDryRunResult { session_id, status })
}

pub(super) const fn map_history_error(kind: HistoryErrorKind) -> RustTargetDryRunError {
    match kind {
        HistoryErrorKind::QueryLimitExceeded => RustTargetDryRunError::BudgetExceeded,
        HistoryErrorKind::Busy => RustTargetDryRunError::Busy,
        HistoryErrorKind::UnsafeStorage => RustTargetDryRunError::UnsafeStorage,
        HistoryErrorKind::IncompatibleSchema => RustTargetDryRunError::IncompatibleSchema,
        HistoryErrorKind::CorruptData => RustTargetDryRunError::CorruptData,
        HistoryErrorKind::OutcomeUnknown => RustTargetDryRunError::HistoryUnresolved,
        HistoryErrorKind::DatabaseUnavailable => RustTargetDryRunError::Unavailable,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::InternalState => RustTargetDryRunError::InternalState,
    }
}

pub(super) const fn failure_kind(error: RustTargetDryRunError) -> RustTargetDryRunFailureKind {
    match error {
        RustTargetDryRunError::ParentReviewUnavailable => {
            RustTargetDryRunFailureKind::ParentReviewUnavailable
        }
        RustTargetDryRunError::ReviewExpired => RustTargetDryRunFailureKind::ReviewExpired,
        RustTargetDryRunError::ChangedDuringReview => {
            RustTargetDryRunFailureKind::ChangedDuringReview
        }
        RustTargetDryRunError::BudgetExceeded | RustTargetDryRunError::QueueFull => {
            RustTargetDryRunFailureKind::BudgetExceeded
        }
        RustTargetDryRunError::Busy => RustTargetDryRunFailureKind::Busy,
        RustTargetDryRunError::UnsafeStorage => RustTargetDryRunFailureKind::UnsafeStorage,
        RustTargetDryRunError::IncompatibleSchema => {
            RustTargetDryRunFailureKind::IncompatibleSchema
        }
        RustTargetDryRunError::CorruptData => RustTargetDryRunFailureKind::CorruptData,
        RustTargetDryRunError::HistoryUnresolved => RustTargetDryRunFailureKind::HistoryUnresolved,
        RustTargetDryRunError::Unavailable => RustTargetDryRunFailureKind::Unavailable,
        RustTargetDryRunError::Closed
        | RustTargetDryRunError::WrongEngine
        | RustTargetDryRunError::CancelledBeforeStart
        | RustTargetDryRunError::InternalState => RustTargetDryRunFailureKind::InternalState,
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
