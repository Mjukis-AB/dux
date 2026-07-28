//! Journal-fenced admission for a reviewed Explorer Trash request.
//!
//! It owns the one-shot review evidence, durable journal receipt, synchronous
//! platform callback boundary, and any unresolved post-effect capability so
//! no second cleanup authority can be created after an ambiguous outcome.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::capacity::terminalize_with_capacity;
use super::{TrashEffectRequest, TrashPlatformResult, TrashSelectionError};
use crate::domain::{CandidateId, CleanupPlan, CleanupPlanId};
use crate::engine::{SnapshotReviewSession, SnapshotReviewTrashTarget};
use crate::path_validation::{
    CanonicalPathError, capture_scan_root, capture_trash_path_snapshot, validate_cleanup_path,
    validate_scan_root,
};
use crate::persistence::{
    CleanupJournalClaim, CleanupJournalLease, CleanupSessionId, CleanupTrigger, EffectOutcome,
    EffectStartReceipt, HistoryErrorKind, JournalLeaseFailure, NewCleanupSessionRecord,
    StoreCoordinator, ValidationOutcome,
};

use std::sync::atomic::{AtomicU64, Ordering};

static TRASH_SELECTION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Execute one explicit Explorer Trash selection. The review lease supplies
/// the only target evidence; this function creates a one-item reviewed
/// journal row, admits it through the existing receipt fence, and invokes the
/// callback synchronously while the journal claim remains held.
pub(crate) fn execute_reviewed_trash_selection<F>(
    store: &Arc<StoreCoordinator>,
    review: &mut SnapshotReviewSession,
    node_id: u64,
    driver: F,
) -> Result<TrashPlatformResult, TrashSelectionExecutionError>
where
    F: FnOnce(TrashEffectRequest) -> TrashPlatformResult,
{
    let target = review
        .trash_target(node_id)
        .map_err(|_| TrashSelectionExecutionError::Selection(TrashSelectionError::Review))?;
    let sequence = TRASH_SELECTION_SEQUENCE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| {
            TrashSelectionExecutionError::Selection(TrashSelectionError::InvalidRequest)
        })?;
    let plan_id = CleanupPlanId::new(format!("plan:explorer-trash-{sequence}")).map_err(|_| {
        TrashSelectionExecutionError::Selection(TrashSelectionError::InvalidRequest)
    })?;
    let session_id =
        CleanupSessionId::new(format!("cleanup:explorer-trash-{sequence}")).map_err(|_| {
            TrashSelectionExecutionError::Selection(TrashSelectionError::InvalidRequest)
        })?;
    let candidate_id =
        CandidateId::new(format!("candidate:explorer-trash-{sequence}")).map_err(|_| {
            TrashSelectionExecutionError::Selection(TrashSelectionError::InvalidRequest)
        })?;
    let started_at = SystemTime::now();
    let plan = CleanupPlan::try_from_trash_selection(
        plan_id,
        started_at,
        review.scan_id().clone(),
        candidate_id,
        target.snapshot.requested_path().to_path_buf(),
    )
    .map_err(|_| TrashSelectionExecutionError::Selection(TrashSelectionError::InvalidRequest))?;
    let record = NewCleanupSessionRecord::try_from_uncoupled_plan(
        session_id.clone(),
        &plan,
        started_at,
        CleanupTrigger::Manual,
    )
    .map_err(|error| {
        TrashSelectionExecutionError::Selection(map_selection_history_error(error.kind))
    })?;
    store
        .record_cleanup_session_planned(&record)
        .map_err(|error| {
            TrashSelectionExecutionError::Selection(map_selection_history_error(error.kind))
        })?;

    let admission_at = started_at.checked_add(Duration::from_millis(1)).ok_or(
        TrashSelectionExecutionError::Selection(TrashSelectionError::InvalidRequest),
    )?;

    let admission = match TrashExecutionAdmission::begin(
        store,
        &session_id,
        target,
        0,
        0,
        admission_at,
        Duration::from_secs(5),
    ) {
        Ok(admission) => admission,
        Err(TrashAdmissionStartError::JournalClaimUnresolved(unresolved)) => unresolved
            .retry()
            .map_err(map_selection_admission_start_error)?,
        Err(error) => return Err(map_selection_admission_start_error(error)),
    };
    let mut platform = CallbackTrashPlatform {
        driver: Some(driver),
    };
    match admission.execute_with(&mut platform) {
        Ok(()) => Ok(TrashPlatformResult::Completed),
        Err(TrashExecutionError::Platform(TrashPlatformError::Unsupported)) => {
            Ok(TrashPlatformResult::Unsupported)
        }
        Err(TrashExecutionError::Platform(TrashPlatformError::Failed)) => {
            Ok(TrashPlatformResult::Failed)
        }
        Err(TrashExecutionError::Platform(TrashPlatformError::OutcomeUnknown)) => {
            Ok(TrashPlatformResult::OutcomeUnknown)
        }
        Err(TrashExecutionError::Admission(error)) => Err(TrashSelectionExecutionError::Selection(
            map_selection_admission_error(error),
        )),
        Err(TrashExecutionError::UnresolvedEffect(unresolved)) => {
            Err(TrashSelectionExecutionError::UnresolvedEffect(unresolved))
        }
    }
}

/// An engine-facing Trash failure that retains every ambiguous owner
/// capability. Known, path-free selection failures remain separately
/// mappable to the stable public [`TrashSelectionError`] surface.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TrashSelectionExecutionError {
    #[error(transparent)]
    Selection(TrashSelectionError),
    #[error("the Trash journal owner claim could not be reconciled")]
    JournalClaimUnresolved(Box<UnresolvedTrashJournalClaim>),
    #[error("the claimed Trash admission could not be safely completed")]
    ClaimedAdmissionUnresolved(Box<UnsettledTrashAdmission>),
    #[error("the Trash effect outcome requires journal quarantine")]
    UnresolvedEffect(Box<UnsettledTrashEffect>),
}

struct CallbackTrashPlatform<F> {
    driver: Option<F>,
}

impl<F> TrashPlatformEffect for CallbackTrashPlatform<F>
where
    F: FnOnce(TrashEffectRequest) -> TrashPlatformResult,
{
    fn trash(
        &mut self,
        target: &crate::path_validation::TrashPathSnapshot,
    ) -> Result<(), TrashPlatformError> {
        let driver = self.driver.take().ok_or(TrashPlatformError::Failed)?;
        let request = TrashEffectRequest::from_target(target.target_kind(), target.object_path());
        match driver(request) {
            TrashPlatformResult::Completed => Ok(()),
            TrashPlatformResult::Unsupported => Err(TrashPlatformError::Unsupported),
            TrashPlatformResult::Failed => Err(TrashPlatformError::Failed),
            TrashPlatformResult::OutcomeUnknown => Err(TrashPlatformError::OutcomeUnknown),
        }
    }
}

fn map_selection_history_error(kind: HistoryErrorKind) -> TrashSelectionError {
    match kind {
        HistoryErrorKind::Busy => TrashSelectionError::Busy,
        HistoryErrorKind::UnsafeStorage => TrashSelectionError::UnsafeStorage,
        HistoryErrorKind::IncompatibleSchema => TrashSelectionError::IncompatibleSchema,
        HistoryErrorKind::CorruptData => TrashSelectionError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => TrashSelectionError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => TrashSelectionError::OutcomeUnknown,
        HistoryErrorKind::InvalidInput => TrashSelectionError::InvalidRequest,
        HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::QueryLimitExceeded
        | HistoryErrorKind::InternalState => TrashSelectionError::InternalState,
    }
}

fn map_selection_admission_error(error: TrashAdmissionError) -> TrashSelectionError {
    match error {
        TrashAdmissionError::TargetUnavailable
        | TrashAdmissionError::UnsupportedTargetKind
        | TrashAdmissionError::UnsupportedEffectMode
        | TrashAdmissionError::TargetNotBound => TrashSelectionError::Review,
        TrashAdmissionError::TargetChanged => TrashSelectionError::ChangedSincePlan,
        TrashAdmissionError::Journal(kind) => map_selection_history_error(kind),
    }
}

fn map_selection_admission_start_error(
    error: TrashAdmissionStartError,
) -> TrashSelectionExecutionError {
    match error {
        TrashAdmissionStartError::Admission(error) => {
            TrashSelectionExecutionError::Selection(map_selection_admission_error(error))
        }
        TrashAdmissionStartError::JournalClaimUnresolved(unresolved) => {
            TrashSelectionExecutionError::JournalClaimUnresolved(unresolved)
        }
        TrashAdmissionStartError::ClaimedAdmissionUnresolved(unresolved) => {
            TrashSelectionExecutionError::ClaimedAdmissionUnresolved(unresolved)
        }
    }
}

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
    #[error("the cleanup journal row is not a Trash effect")]
    UnsupportedEffectMode,
    #[error("the reviewed Trash target is not bound to the planned journal path")]
    TargetNotBound,
    #[error("the cleanup journal rejected the Trash admission: {0:?}")]
    Journal(HistoryErrorKind),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TrashAdmissionStartError {
    #[error(transparent)]
    Admission(TrashAdmissionError),
    #[error("the Trash journal owner claim could not be reconciled")]
    JournalClaimUnresolved(Box<UnresolvedTrashJournalClaim>),
    #[error("the claimed Trash admission could not be safely completed")]
    ClaimedAdmissionUnresolved(Box<UnsettledTrashAdmission>),
}

/// An exact generation-one Trash claim whose commit could not yet be proven.
/// The original lease, reviewed target, session identifier, ordinals, and
/// canonical claim time stay inseparable. Retrying can only adopt or repeat
/// that exact journal claim; it cannot invoke the platform callback.
#[must_use = "retry the exact claim or quarantine its cleanup lease"]
pub(crate) struct UnresolvedTrashJournalClaim {
    target: SnapshotReviewTrashTarget,
    session_id: CleanupSessionId,
    item_ordinal: usize,
    path_ordinal: usize,
    observed_at: SystemTime,
    failure: JournalLeaseFailure,
}

impl std::fmt::Debug for UnresolvedTrashJournalClaim {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UnresolvedTrashJournalClaim")
            .field("session_id", &self.session_id)
            .field("item_ordinal", &self.item_ordinal)
            .field("path_ordinal", &self.path_ordinal)
            .field("observed_at", &self.observed_at)
            .field("failure", &self.failure)
            .finish_non_exhaustive()
    }
}

impl UnresolvedTrashJournalClaim {
    pub(crate) fn kind(&self) -> HistoryErrorKind {
        self.failure.kind()
    }

    /// Retry only the original owner/generation-one journal claim. A matching
    /// post-commit owner is adopted by `claim_planned`; the reviewed target is
    /// admitted only after that exact claim is proven.
    pub(crate) fn retry(
        self: Box<Self>,
    ) -> Result<TrashExecutionAdmission, TrashAdmissionStartError> {
        let Self {
            target,
            session_id,
            item_ordinal,
            path_ordinal,
            observed_at,
            failure,
        } = *self;
        let claim = match failure.into_lease().claim_planned(&session_id, observed_at) {
            Ok(claim) => claim,
            Err(failure) => {
                return Err(TrashAdmissionStartError::JournalClaimUnresolved(Box::new(
                    Self {
                        target,
                        session_id,
                        item_ordinal,
                        path_ordinal,
                        observed_at,
                        failure,
                    },
                )));
            }
        };
        TrashExecutionAdmission::from_claim(claim, target, item_ordinal, path_ordinal, observed_at)
    }

    /// Retain the held cleanup lease in the engine quarantine when the exact
    /// claim cannot be reconciled during this process lifetime.
    pub(crate) fn into_lease(self: Box<Self>) -> CleanupJournalLease {
        self.failure.into_lease()
    }
}

/// A journal claim whose post-claim admission result cannot be proven safe to
/// release. A receipt is retained when `effect_started` was durably minted,
/// even though no platform call was made. Ordinary orchestration never retries
/// this capability: the engine quarantines it for the rest of the process so
/// restart recovery remains the only authority that can settle the journal.
#[must_use = "quarantine the claimed Trash admission until restart recovery"]
pub(crate) struct UnsettledTrashAdmission {
    claim: CleanupJournalClaim,
    receipt: Option<EffectStartReceipt>,
}

impl std::fmt::Debug for UnsettledTrashAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UnsettledTrashAdmission")
            .field("has_effect_receipt", &self.receipt.is_some())
            .finish_non_exhaustive()
    }
}

fn unresolved_trash_admission(
    claim: CleanupJournalClaim,
    receipt: Option<EffectStartReceipt>,
) -> TrashAdmissionStartError {
    TrashAdmissionStartError::ClaimedAdmissionUnresolved(Box::new(UnsettledTrashAdmission {
        claim,
        receipt,
    }))
}

fn terminalize_trash_admission_rejection(
    mut claim: CleanupJournalClaim,
    item_ordinal: usize,
    path_ordinal: usize,
    outcome: ValidationOutcome,
    error_category: &'static str,
    observed_at: SystemTime,
    error: TrashAdmissionError,
) -> TrashAdmissionStartError {
    if claim
        .finish_validation(
            item_ordinal,
            path_ordinal,
            outcome,
            Some(error_category),
            observed_at,
        )
        .is_err()
        || terminalize_with_capacity(&mut claim, observed_at, None).is_err()
    {
        return unresolved_trash_admission(claim, None);
    }
    TrashAdmissionStartError::Admission(error)
}

/// A non-cloneable, one-shot capability that proves journal ordering and a
/// fresh no-follow target witness. It grants no platform effect by itself.
pub(crate) struct TrashExecutionAdmission {
    target: SnapshotReviewTrashTarget,
    claim: CleanupJournalClaim,
    receipt: EffectStartReceipt,
}

/// A platform driver invoked synchronously while the journal claim and
/// cleanup lock remain held. Implementations must not retain the snapshot or
/// retry after returning; the snapshot is stale outside this one-shot call.
pub(crate) trait TrashPlatformEffect {
    fn trash(
        &mut self,
        target: &crate::path_validation::TrashPathSnapshot,
    ) -> Result<(), TrashPlatformError>;
}

/// Path-free, bounded platform outcomes. A Foundation error after call entry
/// is conservatively represented as [`Self::OutcomeUnknown`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum TrashPlatformError {
    #[error("the Trash platform adapter does not support this target")]
    Unsupported,
    #[error("the Trash platform adapter failed before completing the operation")]
    Failed,
    #[error("the Trash platform operation outcome is unknown")]
    OutcomeUnknown,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TrashExecutionError {
    #[error("Trash admission failed: {0}")]
    Admission(TrashAdmissionError),
    #[error("Trash platform effect failed: {0}")]
    Platform(TrashPlatformError),
    #[error("Trash effect outcome requires journal quarantine")]
    UnresolvedEffect(Box<UnsettledTrashEffect>),
}

impl PartialEq for TrashExecutionError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Admission(left), Self::Admission(right)) => left == right,
            (Self::Platform(left), Self::Platform(right)) => left == right,
            (Self::UnresolvedEffect(_), Self::UnresolvedEffect(_)) => false,
            _ => false,
        }
    }
}

impl Eq for TrashExecutionError {}

#[derive(Clone, Copy, Debug)]
enum PendingTrashSettlement {
    CancelBeforeCall,
    Finish {
        outcome: EffectOutcome,
        error_category: Option<&'static str>,
    },
    Terminalize,
    QuarantinedOutcomeUnknown,
}

#[derive(Clone, Copy, Debug)]
enum TrashEffectCompletion {
    Completed,
    Admission(TrashAdmissionError),
    Platform(TrashPlatformError),
}

impl TrashEffectCompletion {
    fn into_result(self) -> Result<(), TrashExecutionError> {
        match self {
            Self::Completed => Ok(()),
            Self::Admission(error) => Err(TrashExecutionError::Admission(error)),
            Self::Platform(error) => Err(TrashExecutionError::Platform(error)),
        }
    }
}

/// A Trash effect has entered the durable receipt fence but its exact
/// post-state cannot safely be released. The capability owns both the live
/// journal claim and receipt. Its retry path performs persistence only and
/// can never reach the platform callback or reconstruct a target path.
///
/// A successfully recorded `outcome_unknown` deliberately remains in this
/// capability so the engine can quarantine the recovering claim instead of
/// dropping the cleanup lease while the effect remains unresolved.
#[must_use = "retry journal settlement or quarantine the owning cleanup claim"]
pub(crate) struct UnsettledTrashEffect {
    claim: CleanupJournalClaim,
    receipt: EffectStartReceipt,
    settlement: PendingTrashSettlement,
    completed_at: SystemTime,
    observed: TrashEffectCompletion,
}

impl std::fmt::Debug for UnsettledTrashEffect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UnsettledTrashEffect")
            .field("settlement", &self.settlement)
            .field("completed_at", &self.completed_at)
            .field("observed", &self.observed)
            .finish_non_exhaustive()
    }
}

impl UnsettledTrashEffect {
    /// Retry only the exact durable post-effect transition and, for known
    /// outcomes, terminalization. A second ambiguous write returns the same
    /// owned claim/receipt capability again. A settled unknown outcome also
    /// stays owned for engine quarantine and recovery.
    pub(crate) fn retry(
        mut self: Box<Self>,
    ) -> Result<Result<(), TrashExecutionError>, Box<UnsettledTrashEffect>> {
        loop {
            match self.settlement {
                PendingTrashSettlement::CancelBeforeCall => {
                    if self
                        .claim
                        .cancel_effect_before_call(&self.receipt, self.completed_at)
                        .is_err()
                    {
                        return Err(self);
                    }
                    self.settlement = PendingTrashSettlement::Terminalize;
                }
                PendingTrashSettlement::Finish {
                    outcome,
                    error_category,
                } => {
                    if self
                        .claim
                        .finish_effect(&self.receipt, outcome, error_category, self.completed_at)
                        .is_err()
                    {
                        return Err(self);
                    }
                    self.settlement = if outcome == EffectOutcome::OutcomeUnknown {
                        PendingTrashSettlement::QuarantinedOutcomeUnknown
                    } else {
                        PendingTrashSettlement::Terminalize
                    };
                }
                PendingTrashSettlement::Terminalize => {
                    if terminalize_with_capacity(&mut self.claim, self.completed_at, None).is_err()
                    {
                        return Err(self);
                    }
                    return Ok(self.observed.into_result());
                }
                PendingTrashSettlement::QuarantinedOutcomeUnknown => return Err(self),
            }
        }
    }

    pub(crate) fn observed_platform_error(&self) -> Option<TrashPlatformError> {
        match self.observed {
            TrashEffectCompletion::Platform(error) => Some(error),
            TrashEffectCompletion::Completed | TrashEffectCompletion::Admission(_) => None,
        }
    }

    pub(crate) fn journal_outcome_is_unknown(&self) -> bool {
        matches!(
            self.settlement,
            PendingTrashSettlement::QuarantinedOutcomeUnknown
        )
    }
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
    ) -> Result<Self, TrashAdmissionStartError> {
        let lease = store
            .acquire_cleanup_journal_lease(lock_timeout)
            .map_err(|error| {
                TrashAdmissionStartError::Admission(TrashAdmissionError::Journal(error.kind))
            })?;
        let claim = match lease.claim_planned(session_id, observed_at) {
            Ok(claim) => claim,
            Err(failure) => {
                return Err(TrashAdmissionStartError::JournalClaimUnresolved(Box::new(
                    UnresolvedTrashJournalClaim {
                        target,
                        session_id: session_id.clone(),
                        item_ordinal,
                        path_ordinal,
                        observed_at,
                        failure,
                    },
                )));
            }
        };
        Self::from_claim(claim, target, item_ordinal, path_ordinal, observed_at)
    }

    /// Admit against an already fenced claim. This narrow constructor keeps
    /// tests and future engine orchestration from bypassing the same ordering
    /// rules used by [`Self::begin`].
    pub(crate) fn from_claim(
        mut claim: CleanupJournalClaim,
        target: SnapshotReviewTrashTarget,
        item_ordinal: usize,
        path_ordinal: usize,
        observed_at: SystemTime,
    ) -> Result<Self, TrashAdmissionStartError> {
        if let Err(error) = claim.validate_planned_path(
            item_ordinal,
            path_ordinal,
            target.snapshot.requested_path(),
        ) {
            if error.kind == HistoryErrorKind::InvalidTransition {
                // A valid row with a mismatched path is a rejected request,
                // not an abandoned running journal. Invalid ordinals or a
                // failed write remain journal errors and are never guessed.
                if claim.begin_validation(item_ordinal, path_ordinal).is_err() {
                    return Err(unresolved_trash_admission(claim, None));
                }
                return Err(terminalize_trash_admission_rejection(
                    claim,
                    item_ordinal,
                    path_ordinal,
                    ValidationOutcome::Rejected,
                    "trash_target_not_bound",
                    observed_at,
                    TrashAdmissionError::TargetNotBound,
                ));
            }
            return Err(unresolved_trash_admission(claim, None));
        }
        if claim.begin_validation(item_ordinal, path_ordinal).is_err() {
            return Err(unresolved_trash_admission(claim, None));
        }

        if let Err(error) = claim.validate_trash_effect(item_ordinal, path_ordinal) {
            if error.kind == HistoryErrorKind::InvalidTransition {
                return Err(terminalize_trash_admission_rejection(
                    claim,
                    item_ordinal,
                    path_ordinal,
                    ValidationOutcome::Rejected,
                    "trash_effect_mode_mismatch",
                    observed_at,
                    TrashAdmissionError::UnsupportedEffectMode,
                ));
            }
            return Err(unresolved_trash_admission(claim, None));
        }

        if let Err(error) = revalidate_target(&target) {
            let outcome = match error {
                TrashAdmissionError::TargetChanged => ValidationOutcome::ChangedSincePlan,
                TrashAdmissionError::UnsupportedTargetKind => ValidationOutcome::Rejected,
                TrashAdmissionError::UnsupportedEffectMode => ValidationOutcome::Rejected,
                TrashAdmissionError::TargetUnavailable | TrashAdmissionError::Journal(_) => {
                    ValidationOutcome::Unavailable
                }
                TrashAdmissionError::TargetNotBound => ValidationOutcome::Rejected,
            };
            return Err(terminalize_trash_admission_rejection(
                claim,
                item_ordinal,
                path_ordinal,
                outcome,
                validation_error_category(error),
                observed_at,
                error,
            ));
        }

        let receipt = match claim.mark_effect_started(item_ordinal, path_ordinal, observed_at) {
            Ok(receipt) => receipt,
            Err(_) => return Err(unresolved_trash_admission(claim, None)),
        };
        if let Err(error) = claim.revalidate_effect_receipt(&receipt) {
            // The receipt was durable, but no platform call has happened. Use
            // the journal's explicit pre-effect cancellation transition rather
            // than leaving an effect-started row that could be mistaken for a
            // call in a later recovery pass.
            if claim
                .cancel_effect_before_call(&receipt, observed_at)
                .is_err()
                || terminalize_with_capacity(&mut claim, observed_at, None).is_err()
            {
                return Err(unresolved_trash_admission(claim, Some(receipt)));
            }
            return Err(TrashAdmissionStartError::Admission(
                TrashAdmissionError::Journal(error.kind),
            ));
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

    /// Revalidate the target and durable receipt immediately before invoking
    /// the synchronous platform driver, then settle the journal outcome while
    /// the one-shot claim is still held. The caller cannot retry this
    /// admission because it is consumed by the method.
    pub(crate) fn execute_with(
        self,
        platform: &mut impl TrashPlatformEffect,
    ) -> Result<(), TrashExecutionError> {
        self.execute_with_clock(platform, SystemTime::now)
    }

    #[cfg(test)]
    pub(crate) fn execute_with_at(
        self,
        platform: &mut impl TrashPlatformEffect,
        completed_at: SystemTime,
    ) -> Result<(), TrashExecutionError> {
        self.execute_with_clock(platform, || completed_at)
    }

    #[cfg(test)]
    pub(crate) fn execute_with_clock_for_test(
        self,
        platform: &mut impl TrashPlatformEffect,
        clock: impl FnMut() -> SystemTime,
    ) -> Result<(), TrashExecutionError> {
        self.execute_with_clock(platform, clock)
    }

    fn execute_with_clock(
        self,
        platform: &mut impl TrashPlatformEffect,
        mut clock: impl FnMut() -> SystemTime,
    ) -> Result<(), TrashExecutionError> {
        if let Err(error) = revalidate_target(&self.target) {
            let completed_at = clock();
            return settle_trash_effect(
                self.claim,
                self.receipt,
                PendingTrashSettlement::CancelBeforeCall,
                completed_at,
                TrashEffectCompletion::Admission(error),
            );
        }
        if let Err(error) = self.claim.revalidate_effect_receipt(&self.receipt) {
            let completed_at = clock();
            return settle_trash_effect(
                self.claim,
                self.receipt,
                PendingTrashSettlement::CancelBeforeCall,
                completed_at,
                TrashEffectCompletion::Admission(TrashAdmissionError::Journal(error.kind)),
            );
        }

        let (platform_result, effect_panicked) =
            match catch_unwind(AssertUnwindSafe(|| platform.trash(&self.target.snapshot))) {
                Ok(result) => (result, false),
                Err(_) => (Err(TrashPlatformError::OutcomeUnknown), true),
            };
        let completed_at = clock();
        let (outcome, error_category, observed) = match platform_result {
            Ok(()) => (
                EffectOutcome::Trashed,
                None,
                TrashEffectCompletion::Completed,
            ),
            Err(error @ TrashPlatformError::Unsupported) => (
                EffectOutcome::Failed,
                Some("trash_platform_unsupported"),
                TrashEffectCompletion::Platform(error),
            ),
            Err(error @ TrashPlatformError::Failed) => (
                EffectOutcome::Failed,
                Some("trash_platform_failed"),
                TrashEffectCompletion::Platform(error),
            ),
            Err(error @ TrashPlatformError::OutcomeUnknown) => (
                EffectOutcome::OutcomeUnknown,
                Some(if effect_panicked {
                    "trash_platform_panicked"
                } else {
                    "trash_platform_outcome_unknown"
                }),
                TrashEffectCompletion::Platform(error),
            ),
        };
        settle_trash_effect(
            self.claim,
            self.receipt,
            PendingTrashSettlement::Finish {
                outcome,
                error_category,
            },
            completed_at,
            observed,
        )
    }
}

fn settle_trash_effect(
    claim: CleanupJournalClaim,
    receipt: EffectStartReceipt,
    settlement: PendingTrashSettlement,
    completed_at: SystemTime,
    observed: TrashEffectCompletion,
) -> Result<(), TrashExecutionError> {
    let unresolved = Box::new(UnsettledTrashEffect {
        claim,
        receipt,
        settlement,
        completed_at,
        observed,
    });
    match unresolved.retry() {
        Ok(result) => result,
        Err(unresolved) => Err(TrashExecutionError::UnresolvedEffect(unresolved)),
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
        TrashAdmissionError::UnsupportedEffectMode => "trash_effect_mode_mismatch",
        TrashAdmissionError::TargetNotBound => "trash_target_not_bound",
        TrashAdmissionError::Journal(_) => "trash_journal_rejected",
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::fs;
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::ScanId;
    use crate::path_validation::{
        TrashTargetKind, capture_scan_root, capture_trash_path_snapshot, validate_cleanup_path,
        validate_scan_root,
    };
    use crate::persistence::{NewScanRecord, ScanCompletionRecord, ScanCounts, TerminalScanStatus};

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

    struct TrashFixture {
        _temp: TempDir,
        store: Arc<StoreCoordinator>,
        session_id: CleanupSessionId,
        target: Option<SnapshotReviewTrashTarget>,
        observed_at: SystemTime,
    }

    impl TrashFixture {
        fn new(label: &str) -> Self {
            let temp = TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
            let target = reviewed_file_target(&temp);
            let store =
                StoreCoordinator::open(&temp.path().join("store").join("dux.sqlite3")).unwrap();
            let started_at = UNIX_EPOCH + Duration::from_secs(1_780_000_000);
            let observed_at = started_at + Duration::from_millis(1);
            let scan_id = ScanId::new(format!("scan:trash-executor-{label}")).unwrap();
            store
                .record_scan_started(
                    &NewScanRecord::try_new(
                        scan_id.clone(),
                        target.snapshot.scan_root().to_path_buf(),
                        started_at - Duration::from_secs(2),
                    )
                    .unwrap(),
                )
                .unwrap();
            store
                .record_scan_finished(
                    &ScanCompletionRecord::try_new(
                        scan_id.clone(),
                        started_at - Duration::from_secs(1),
                        TerminalScanStatus::Succeeded,
                        ScanCounts::default(),
                    )
                    .unwrap(),
                )
                .unwrap();
            let plan = CleanupPlan::try_from_trash_selection(
                CleanupPlanId::new(format!("plan:trash-executor-{label}")).unwrap(),
                started_at,
                scan_id,
                CandidateId::new(format!("candidate:trash-executor-{label}")).unwrap(),
                target.snapshot.requested_path().to_path_buf(),
            )
            .unwrap();
            let session_id =
                CleanupSessionId::new(format!("cleanup:trash-executor-{label}")).unwrap();
            let record = NewCleanupSessionRecord::try_from_uncoupled_plan(
                session_id.clone(),
                &plan,
                started_at,
                CleanupTrigger::Manual,
            )
            .unwrap();
            store.record_cleanup_session_planned(&record).unwrap();
            Self {
                _temp: temp,
                store,
                session_id,
                target: Some(target),
                observed_at,
            }
        }

        fn admission(&mut self) -> TrashExecutionAdmission {
            match TrashExecutionAdmission::begin(
                &self.store,
                &self.session_id,
                self.target.take().expect("fixture target is one-shot"),
                0,
                0,
                self.observed_at,
                Duration::from_secs(2),
            ) {
                Ok(admission) => admission,
                Err(error) => panic!("fixture admission failed: {error:?}"),
            }
        }
    }

    struct CountingTrashPlatform {
        calls: usize,
        outcome: Result<(), TrashPlatformError>,
    }

    impl TrashPlatformEffect for CountingTrashPlatform {
        fn trash(
            &mut self,
            _: &crate::path_validation::TrashPathSnapshot,
        ) -> Result<(), TrashPlatformError> {
            self.calls += 1;
            self.outcome
        }
    }

    struct PanickingTrashPlatform {
        calls: usize,
    }

    impl TrashPlatformEffect for PanickingTrashPlatform {
        fn trash(
            &mut self,
            _: &crate::path_validation::TrashPathSnapshot,
        ) -> Result<(), TrashPlatformError> {
            self.calls += 1;
            panic!("simulated platform callback panic");
        }
    }

    #[test]
    fn revalidation_rejects_replaced_object_identity() {
        let temp = TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let target = reviewed_file_target(&temp);
        let path = target.snapshot.object_path().to_path_buf();
        // DUX-DESTRUCTIVE: allow=test-reviewed-trash-replaced-file-remove -- remove only the reviewed TempDir-owned file before installing an identity-changing replacement
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"replacement").unwrap();

        assert_eq!(
            revalidate_target(&target),
            Err(TrashAdmissionError::TargetChanged)
        );
        assert_eq!(
            map_selection_admission_error(TrashAdmissionError::TargetChanged),
            TrashSelectionError::ChangedSincePlan
        );
    }

    #[test]
    fn revalidation_rejects_missing_object_without_returning_a_path() {
        let temp = TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let target = reviewed_file_target(&temp);
        // DUX-DESTRUCTIVE: allow=test-reviewed-trash-missing-file-remove -- remove only the reviewed TempDir-owned file to prove missing targets fail without path disclosure
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

    #[test]
    fn ambiguous_generation_one_claim_retains_and_adopts_the_exact_lease() {
        let mut fixture = TrashFixture::new("claim-ambiguity");
        crate::persistence::fail_next_write_after_commit_and_reconcile_read_for_test();

        let unresolved = match TrashExecutionAdmission::begin(
            &fixture.store,
            &fixture.session_id,
            fixture.target.take().unwrap(),
            0,
            0,
            fixture.observed_at,
            Duration::from_secs(2),
        ) {
            Err(TrashAdmissionStartError::JournalClaimUnresolved(unresolved)) => unresolved,
            Err(error) => panic!("expected retained ambiguous claim, got {error:?}"),
            Ok(_) => panic!("ambiguous claim must not be reported as admitted"),
        };
        assert_eq!(unresolved.kind(), HistoryErrorKind::DatabaseUnavailable);

        let admission = match unresolved.retry() {
            Ok(admission) => admission,
            Err(error) => panic!("the exact committed owner should be adopted: {error:?}"),
        };
        let mut platform = CountingTrashPlatform {
            calls: 0,
            outcome: Ok(()),
        };
        admission
            .execute_with_at(&mut platform, fixture.observed_at + Duration::from_secs(1))
            .unwrap();
        assert_eq!(platform.calls, 1);
    }

    #[test]
    fn platform_callback_panic_is_caught_and_retains_the_recovering_claim() {
        let mut fixture = TrashFixture::new("platform-panic");
        let completed_at = fixture.observed_at + Duration::from_secs(1);
        let admission = fixture.admission();
        let mut platform = PanickingTrashPlatform { calls: 0 };

        let unresolved = match admission.execute_with_at(&mut platform, completed_at) {
            Err(TrashExecutionError::UnresolvedEffect(unresolved)) => unresolved,
            other => panic!("platform panic must return a retained unknown outcome: {other:?}"),
        };
        assert_eq!(platform.calls, 1);
        assert_eq!(
            unresolved.observed_platform_error(),
            Some(TrashPlatformError::OutcomeUnknown)
        );
        assert!(unresolved.journal_outcome_is_unknown());
        let error_category = fixture.store.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT error_category
                     FROM cleanup_item_paths
                     WHERE session_id = ?1 AND item_ordinal = 0 AND path_ordinal = 0",
                    [fixture.session_id.as_str()],
                    |row| row.get::<_, Option<String>>(0),
                )
                .unwrap()
        });
        assert_eq!(error_category.as_deref(), Some("trash_platform_panicked"));

        let _quarantined = unresolved;
    }

    #[test]
    fn platform_and_finish_ambiguity_retries_only_the_exact_settlement() {
        let mut fixture = TrashFixture::new("double-ambiguity");
        let completed_at = fixture.observed_at + Duration::from_secs(1);
        let admission = fixture.admission();
        let mut platform = CountingTrashPlatform {
            calls: 0,
            outcome: Err(TrashPlatformError::OutcomeUnknown),
        };
        crate::persistence::fail_next_write_after_commit_and_reconcile_read_for_test();

        let unsettled = match admission.execute_with_at(&mut platform, completed_at) {
            Err(TrashExecutionError::UnresolvedEffect(unsettled)) => unsettled,
            other => panic!("ambiguous finish must retain its claim and receipt: {other:?}"),
        };
        assert_eq!(platform.calls, 1);
        assert!(!unsettled.journal_outcome_is_unknown());

        let quarantined = match unsettled.retry() {
            Err(quarantined) => quarantined,
            Ok(other) => panic!("an unknown platform outcome must remain quarantined: {other:?}"),
        };
        assert_eq!(
            platform.calls, 1,
            "settlement retry must not call Trash again"
        );
        assert_eq!(
            quarantined.observed_platform_error(),
            Some(TrashPlatformError::OutcomeUnknown)
        );
        assert!(quarantined.journal_outcome_is_unknown());

        let _quarantined = quarantined;
    }
}
