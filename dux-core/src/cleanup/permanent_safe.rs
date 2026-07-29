//! Private, descriptor-relative executor boundary for deterministic
//! permanent-safe rules.
//!
//! This module is intentionally not exported through the engine or UniFFI.
//! It is a tested effect boundary only: the app cannot reach it until the
//! remaining trusted-volume, protected-root, process, descendant, approval,
//! and orchestration gates are complete.

use std::ffi::OsStr;
use std::fs::File;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use thiserror::Error;

use super::capacity::{
    CleanupCapacityIdentity, CleanupCapacityObservation, CleanupCapacitySampler,
    VerifiedCleanupCapacity, terminalize_with_capacity_with_status,
    verify_cleanup_capacity_identity,
};
use crate::path_validation::FilesystemIdentity;
use crate::persistence::{
    CleanupJournalClaim, EffectOutcome, EffectStartReceipt, HistoryErrorKind, TerminalSessionStatus,
};
use crate::planner::{ApprovedCleanupSession, RustTargetEffectWitness};

const MAX_DESCENDANT_ENTRIES: usize = 16_384;
const MAX_DESCENDANT_NAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_DESCENDANT_DEPTH: usize = 64;
const PRESERVED_MARKER: &[u8] = b"CACHEDIR.TAG";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PermanentSafeRemovalSummary {
    pub(crate) removed_entries: u32,
    pub(crate) removed_logical_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PermanentSafeSessionSummary {
    pub(crate) removed_entries: u64,
    pub(crate) removed_logical_bytes: u64,
    pub(crate) terminal_status: TerminalSessionStatus,
    pub(crate) verified_capacity_delta_bytes: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(crate) enum PermanentSafePlatformError {
    #[error("the permanent-safe executor is unsupported on this platform")]
    Unsupported,
    #[error("the reviewed permanent-safe target changed")]
    Changed,
    #[error("the permanent-safe target contains unsafe or unbounded descendants")]
    Unsafe,
    #[error("the permanent-safe operation was cancelled before mutation")]
    Cancelled,
    #[error("the permanent-safe operation failed before its outcome was known")]
    Failed,
    #[error("the permanent-safe operation outcome is unknown")]
    OutcomeUnknown,
}

#[derive(Debug, Error)]
pub(crate) enum PermanentSafeExecutionError {
    #[error("permanent-safe admission was rejected: {0:?}")]
    Admission(HistoryErrorKind),
    #[error("permanent-safe platform effect failed: {0}")]
    Platform(PermanentSafePlatformError),
    #[error("permanent-safe effect settlement could not be reconciled")]
    UnsettledEffect(Box<UnsettledPermanentSafeEffect>),
}

#[derive(Clone, Copy, Debug)]
enum PendingEffectSettlement {
    CancelBeforeCall,
    Finish {
        outcome: EffectOutcome,
        error_category: Option<&'static str>,
    },
}

/// A one-shot filesystem call has returned but its exact journal post-state
/// could not be proven. This capability retains the receipt and intended
/// settlement so callers may retry persistence only; it contains no witness
/// and cannot invoke or repeat the filesystem effect.
#[must_use = "retry journal settlement or quarantine the owning cleanup session"]
pub(crate) struct UnsettledPermanentSafeEffect {
    receipt: EffectStartReceipt,
    settlement: PendingEffectSettlement,
    completed_at: SystemTime,
    observed_result: Result<PermanentSafeRemovalSummary, PermanentSafePlatformError>,
}

impl std::fmt::Debug for UnsettledPermanentSafeEffect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UnsettledPermanentSafeEffect")
            .field("settlement", &self.settlement)
            .field("completed_at", &self.completed_at)
            .field("observed_result", &self.observed_result)
            .finish_non_exhaustive()
    }
}

impl UnsettledPermanentSafeEffect {
    /// Retry only the exact durable post-effect transition. The receipt is
    /// retained again on failure and no platform callback is reachable here.
    pub(crate) fn retry(
        self: Box<Self>,
        claim: &mut CleanupJournalClaim,
    ) -> Result<
        Result<PermanentSafeRemovalSummary, PermanentSafePlatformError>,
        Box<UnsettledPermanentSafeEffect>,
    > {
        let result = match self.settlement {
            PendingEffectSettlement::CancelBeforeCall => {
                claim.cancel_effect_before_call(&self.receipt, self.completed_at)
            }
            PendingEffectSettlement::Finish {
                outcome,
                error_category,
            } => claim.finish_effect(&self.receipt, outcome, error_category, self.completed_at),
        };
        match result {
            Ok(()) => Ok(self.observed_result),
            Err(_) => Err(self),
        }
    }
}

/// Synchronous, one-shot effect seam. Implementations must not retain the
/// witness or retry after returning; the witness is stale outside this call.
pub(crate) trait PermanentSafeContentsDriver {
    fn remove_contents(
        &mut self,
        witness: &RustTargetEffectWitness,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PermanentSafeRemovalSummary, PermanentSafePlatformError>;
}

/// Consume every ordered path in one approved permanent-safe session and
/// terminalize the journal only after all paths have a durable outcome. This
/// remains crate-private: it accepts no paths, callbacks, AI output, FFI
/// values, or CLI requests. A platform failure settles its path and allows
/// later paths to be attempted; an unknown outcome leaves the journal in
/// recovery and stops immediately.
pub(crate) fn execute_rust_target_session(
    session: &mut ApprovedCleanupSession,
    now: SystemTime,
    driver: &mut impl PermanentSafeContentsDriver,
    cancelled: &dyn Fn() -> bool,
) -> Result<PermanentSafeSessionSummary, PermanentSafeExecutionError> {
    execute_rust_target_session_with_capacity(session, now, driver, cancelled, None)
}

/// Ordered session execution with an optional private pre/post capacity
/// sampler. The sampler is deliberately not part of the public engine/FFI
/// surface; missing or mismatched observations only remove the telemetry
/// delta and never change effect authority or terminal status.
pub(crate) fn execute_rust_target_session_with_capacity(
    session: &mut ApprovedCleanupSession,
    now: SystemTime,
    driver: &mut impl PermanentSafeContentsDriver,
    cancelled: &dyn Fn() -> bool,
    capacity_sampler: Option<&mut dyn CleanupCapacitySampler>,
) -> Result<PermanentSafeSessionSummary, PermanentSafeExecutionError> {
    execute_rust_target_session_with_capacity_and_clock(
        session,
        now,
        driver,
        cancelled,
        capacity_sampler,
        &mut SystemTime::now,
    )
}

#[cfg(test)]
pub(crate) fn execute_rust_target_session_with_capacity_and_clock_for_test(
    session: &mut ApprovedCleanupSession,
    now: SystemTime,
    driver: &mut impl PermanentSafeContentsDriver,
    cancelled: &dyn Fn() -> bool,
    capacity_sampler: Option<&mut dyn CleanupCapacitySampler>,
    authority_now: &mut dyn FnMut() -> SystemTime,
) -> Result<PermanentSafeSessionSummary, PermanentSafeExecutionError> {
    execute_rust_target_session_with_capacity_and_clock(
        session,
        now,
        driver,
        cancelled,
        capacity_sampler,
        authority_now,
    )
}

fn execute_rust_target_session_with_capacity_and_clock(
    session: &mut ApprovedCleanupSession,
    now: SystemTime,
    driver: &mut impl PermanentSafeContentsDriver,
    cancelled: &dyn Fn() -> bool,
    mut capacity_sampler: Option<&mut dyn CleanupCapacitySampler>,
    authority_now: &mut dyn FnMut() -> SystemTime,
) -> Result<PermanentSafeSessionSummary, PermanentSafeExecutionError> {
    let expected_capacity_identity = capacity_sampler
        .as_ref()
        .map(|sampler| sampler.expected_identity());
    let pre_capacity = capacity_sampler
        .as_mut()
        .and_then(|sampler| sampler.sample());
    // Capacity attribution uses the real effect window, independently from
    // the canonical journal timestamp supplied for deterministic transitions.
    let effect_started_at = SystemTime::now();
    let ordered_paths = session
        .plan()
        .items()
        .iter()
        .enumerate()
        .flat_map(|(item_ordinal, item)| {
            (0..item.paths().len()).map(move |path_ordinal| (item_ordinal, path_ordinal))
        })
        .collect::<Vec<_>>();
    let mut removed_entries = 0_u64;
    let mut removed_logical_bytes = 0_u64;

    for (item_ordinal, path_ordinal) in ordered_paths {
        if cancelled() {
            let (terminal_status, verified_capacity_delta_bytes) = cancel_and_terminalize(
                session,
                now,
                expected_capacity_identity.as_ref(),
                pre_capacity.as_ref(),
                capacity_sampler.take(),
                effect_started_at,
            )?;
            return Ok(PermanentSafeSessionSummary {
                removed_entries,
                removed_logical_bytes,
                terminal_status,
                verified_capacity_delta_bytes,
            });
        }
        match execute_rust_target_contents_ordered(
            session,
            item_ordinal,
            path_ordinal,
            now,
            driver,
            cancelled,
            authority_now,
        ) {
            Ok(summary) => {
                removed_entries =
                    removed_entries.saturating_add(u64::from(summary.removed_entries));
                removed_logical_bytes =
                    removed_logical_bytes.saturating_add(summary.removed_logical_bytes);
            }
            Err(PermanentSafeExecutionError::Platform(PermanentSafePlatformError::Cancelled)) => {
                let (terminal_status, verified_capacity_delta_bytes) = cancel_and_terminalize(
                    session,
                    now,
                    expected_capacity_identity.as_ref(),
                    pre_capacity.as_ref(),
                    capacity_sampler.take(),
                    effect_started_at,
                )?;
                return Ok(PermanentSafeSessionSummary {
                    removed_entries,
                    removed_logical_bytes,
                    terminal_status,
                    verified_capacity_delta_bytes,
                });
            }
            Err(
                error @ PermanentSafeExecutionError::Platform(
                    PermanentSafePlatformError::OutcomeUnknown,
                ),
            ) => return Err(error),
            Err(PermanentSafeExecutionError::UnsettledEffect(settlement)) => {
                match settlement.retry(session.claim_mut()) {
                    Ok(Ok(summary)) => {
                        removed_entries =
                            removed_entries.saturating_add(u64::from(summary.removed_entries));
                        removed_logical_bytes =
                            removed_logical_bytes.saturating_add(summary.removed_logical_bytes);
                    }
                    Ok(Err(PermanentSafePlatformError::Cancelled)) => {
                        let (terminal_status, verified_capacity_delta_bytes) =
                            cancel_and_terminalize(
                                session,
                                now,
                                expected_capacity_identity.as_ref(),
                                pre_capacity.as_ref(),
                                capacity_sampler.take(),
                                effect_started_at,
                            )?;
                        return Ok(PermanentSafeSessionSummary {
                            removed_entries,
                            removed_logical_bytes,
                            terminal_status,
                            verified_capacity_delta_bytes,
                        });
                    }
                    Ok(Err(PermanentSafePlatformError::OutcomeUnknown)) => {
                        return Err(PermanentSafeExecutionError::Platform(
                            PermanentSafePlatformError::OutcomeUnknown,
                        ));
                    }
                    Ok(Err(_)) => {
                        // The exact path failure is now durably settled. Later
                        // independent paths remain eligible for bounded work.
                    }
                    Err(settlement) => {
                        return Err(PermanentSafeExecutionError::UnsettledEffect(settlement));
                    }
                }
            }
            Err(PermanentSafeExecutionError::Platform(_)) => {
                // The path executor durably settled failures before returning.
                // Continue in plan order so a bad target cannot strand later
                // independent paths in `planned`.
                continue;
            }
            Err(error @ PermanentSafeExecutionError::Admission(_)) => {
                let terminal = session
                    .claim()
                    .path_is_terminal(item_ordinal, path_ordinal)
                    .map_err(|journal| PermanentSafeExecutionError::Admission(journal.kind))?;
                if terminal {
                    continue;
                }
                return Err(error);
            }
        }
    }

    let verification = capture_verified_capacity(
        expected_capacity_identity.as_ref(),
        pre_capacity.as_ref(),
        capacity_sampler.take(),
        effect_started_at,
        SystemTime::now(),
    );
    let verified_capacity_delta_bytes = verification
        .as_ref()
        .map(VerifiedCleanupCapacity::delta_bytes);
    let terminal_status =
        terminalize_with_capacity_with_status(session.claim_mut(), now, verification.as_ref())
            .map_err(|error| PermanentSafeExecutionError::Admission(error.kind))?;
    Ok(PermanentSafeSessionSummary {
        removed_entries,
        removed_logical_bytes,
        terminal_status,
        verified_capacity_delta_bytes,
    })
}

fn cancel_and_terminalize(
    session: &mut ApprovedCleanupSession,
    now: SystemTime,
    expected_identity: Option<&CleanupCapacityIdentity>,
    pre_capacity: Option<&CleanupCapacityObservation>,
    capacity_sampler: Option<&mut dyn CleanupCapacitySampler>,
    effect_started_at: SystemTime,
) -> Result<(TerminalSessionStatus, Option<i64>), PermanentSafeExecutionError> {
    session
        .claim_mut()
        .request_cancellation()
        .map_err(|error| PermanentSafeExecutionError::Admission(error.kind))?;
    session
        .claim_mut()
        .settle_cancellation(now)
        .map_err(|error| PermanentSafeExecutionError::Admission(error.kind))?;
    let verification = capture_verified_capacity(
        expected_identity,
        pre_capacity,
        capacity_sampler,
        effect_started_at,
        SystemTime::now(),
    );
    let verified_capacity_delta_bytes = verification
        .as_ref()
        .map(VerifiedCleanupCapacity::delta_bytes);
    let terminal_status =
        terminalize_with_capacity_with_status(session.claim_mut(), now, verification.as_ref())
            .map_err(|error| PermanentSafeExecutionError::Admission(error.kind))?;
    Ok((terminal_status, verified_capacity_delta_bytes))
}

fn capture_verified_capacity(
    expected_identity: Option<&CleanupCapacityIdentity>,
    pre_capacity: Option<&CleanupCapacityObservation>,
    mut capacity_sampler: Option<&mut dyn CleanupCapacitySampler>,
    effect_started_at: SystemTime,
    effect_completed_at: SystemTime,
) -> Option<VerifiedCleanupCapacity> {
    let post_capacity = capacity_sampler
        .as_mut()
        .and_then(|sampler| sampler.sample());
    let expected_identity = expected_identity?;
    let pre_capacity = pre_capacity?;
    verify_cleanup_capacity_identity(
        expected_identity,
        Some(pre_capacity),
        post_capacity.as_ref(),
        effect_started_at,
        effect_completed_at,
    )
    .ok()
}

/// Consume one approved journal path through the private deterministic rule
/// boundary. This function is not called by the current app/FFI surface.
pub(crate) fn execute_rust_target_contents(
    session: &mut ApprovedCleanupSession,
    item_ordinal: usize,
    path_ordinal: usize,
    now: SystemTime,
    driver: &mut impl PermanentSafeContentsDriver,
    cancelled: &dyn Fn() -> bool,
) -> Result<PermanentSafeRemovalSummary, PermanentSafeExecutionError> {
    let mut authority_now = SystemTime::now;
    let mut clock = EffectExecutionClock {
        journal_time: now,
        authority_now: &mut authority_now,
    };
    execute_rust_target_contents_inner(
        session,
        item_ordinal,
        path_ordinal,
        &mut clock,
        driver,
        cancelled,
        false,
    )
}

fn execute_rust_target_contents_ordered(
    session: &mut ApprovedCleanupSession,
    item_ordinal: usize,
    path_ordinal: usize,
    now: SystemTime,
    driver: &mut impl PermanentSafeContentsDriver,
    cancelled: &dyn Fn() -> bool,
    authority_now: &mut dyn FnMut() -> SystemTime,
) -> Result<PermanentSafeRemovalSummary, PermanentSafeExecutionError> {
    let mut clock = EffectExecutionClock {
        journal_time: now,
        authority_now,
    };
    execute_rust_target_contents_inner(
        session,
        item_ordinal,
        path_ordinal,
        &mut clock,
        driver,
        cancelled,
        true,
    )
}

struct EffectExecutionClock<'a> {
    journal_time: SystemTime,
    authority_now: &'a mut dyn FnMut() -> SystemTime,
}

fn execute_rust_target_contents_inner(
    session: &mut ApprovedCleanupSession,
    item_ordinal: usize,
    path_ordinal: usize,
    clock: &mut EffectExecutionClock<'_>,
    driver: &mut impl PermanentSafeContentsDriver,
    cancelled: &dyn Fn() -> bool,
    ordered_session: bool,
) -> Result<PermanentSafeRemovalSummary, PermanentSafeExecutionError> {
    let now = clock.journal_time;
    let expected_path = session
        .plan()
        .items()
        .get(item_ordinal)
        .and_then(|item| item.paths().get(path_ordinal))
        .map(ToOwned::to_owned)
        .ok_or(PermanentSafeExecutionError::Admission(
            HistoryErrorKind::InvalidInput,
        ))?;
    {
        let claim = session.claim_mut();
        claim
            .validate_planned_path(item_ordinal, path_ordinal, &expected_path)
            .map_err(|error| PermanentSafeExecutionError::Admission(error.kind))?;
        claim
            .begin_validation(item_ordinal, path_ordinal)
            .map_err(|error| PermanentSafeExecutionError::Admission(error.kind))?;
        if let Err(error) = claim.validate_permanent_safe_effect(item_ordinal, path_ordinal) {
            let _ = claim.finish_validation(
                item_ordinal,
                path_ordinal,
                crate::persistence::ValidationOutcome::Rejected,
                Some("permanent_safe_effect_mode_mismatch"),
                now,
            );
            return Err(PermanentSafeExecutionError::Admission(error.kind));
        }
    }

    // Validation is now durable before the live witness is rebuilt. This is
    // important for changed/expired targets: a rejection must not leave a
    // planned row looking executable on restart.
    //
    // Read the authority clock only here, after any capacity pre-sample and
    // immediately before the live grant/witness revalidation. Journal
    // transition timestamps are deliberately separate and cannot extend an
    // expired approval lease.
    let authority_time = (clock.authority_now)();
    let witness_result = if ordered_session {
        session.revalidated_rust_target_effect_for_ordered_session(
            item_ordinal,
            path_ordinal,
            authority_time,
        )
    } else {
        session.revalidated_rust_target_effect(item_ordinal, path_ordinal, authority_time)
    };
    let witness = match witness_result {
        Ok(witness) => witness,
        Err(error) => {
            let (outcome, category) = validation_failure(&error);
            let claim = session.claim_mut();
            claim
                .finish_validation(item_ordinal, path_ordinal, outcome, Some(category), now)
                .map_err(|journal_error| {
                    PermanentSafeExecutionError::Admission(journal_error.kind)
                })?;
            return Err(map_handoff_error(error));
        }
    };
    let claim = session.claim_mut();
    if cancelled() {
        claim
            .finish_validation(
                item_ordinal,
                path_ordinal,
                crate::persistence::ValidationOutcome::Interrupted,
                Some("permanent_safe_cancelled_before_effect"),
                now,
            )
            .map_err(|error| PermanentSafeExecutionError::Admission(error.kind))?;
        return Err(PermanentSafeExecutionError::Platform(
            PermanentSafePlatformError::Cancelled,
        ));
    }
    match claim.permanent_cleanup_effects_enabled() {
        Ok(true) => {}
        Ok(false) => {
            claim
                .finish_validation(
                    item_ordinal,
                    path_ordinal,
                    crate::persistence::ValidationOutcome::Rejected,
                    Some("permanent_cleanup_disabled"),
                    now,
                )
                .map_err(|error| PermanentSafeExecutionError::Admission(error.kind))?;
            return Err(PermanentSafeExecutionError::Admission(
                HistoryErrorKind::InvalidTransition,
            ));
        }
        Err(error) => return Err(PermanentSafeExecutionError::Admission(error.kind)),
    }
    let receipt = claim
        .mark_effect_started(item_ordinal, path_ordinal, now)
        .map_err(|error| PermanentSafeExecutionError::Admission(error.kind))?;
    if let Err(error) = claim.revalidate_effect_receipt(&receipt) {
        let _ = claim.cancel_effect_before_call(&receipt, now);
        return Err(PermanentSafeExecutionError::Admission(error.kind));
    }

    let (result, effect_panicked) = match catch_unwind(AssertUnwindSafe(|| {
        driver.remove_contents(&witness, cancelled)
    })) {
        Ok(result) => (result, false),
        Err(_) => (Err(PermanentSafePlatformError::OutcomeUnknown), true),
    };
    settle_effect(claim, receipt, result, now, effect_panicked)
}

fn map_handoff_error(error: crate::planner::ExactPathHandoffError) -> PermanentSafeExecutionError {
    PermanentSafeExecutionError::Admission(match error {
        crate::planner::ExactPathHandoffError::Journal(error) => error.kind,
        crate::planner::ExactPathHandoffError::Approval(_)
        | crate::planner::ExactPathHandoffError::RuleEvidence(_) => {
            HistoryErrorKind::InvalidTransition
        }
    })
}

fn validation_failure(
    error: &crate::planner::ExactPathHandoffError,
) -> (crate::persistence::ValidationOutcome, &'static str) {
    match error {
        crate::planner::ExactPathHandoffError::RuleEvidence(_) => (
            crate::persistence::ValidationOutcome::ChangedSincePlan,
            "permanent_safe_target_changed",
        ),
        crate::planner::ExactPathHandoffError::Approval(_) => (
            crate::persistence::ValidationOutcome::Rejected,
            "permanent_safe_approval_stale",
        ),
        crate::planner::ExactPathHandoffError::Journal(_) => (
            crate::persistence::ValidationOutcome::Unavailable,
            "permanent_safe_journal_unavailable",
        ),
    }
}

fn settle_effect(
    claim: &mut CleanupJournalClaim,
    receipt: EffectStartReceipt,
    result: Result<PermanentSafeRemovalSummary, PermanentSafePlatformError>,
    completed_at: SystemTime,
    effect_panicked: bool,
) -> Result<PermanentSafeRemovalSummary, PermanentSafeExecutionError> {
    let settlement = match result {
        Ok(_) => PendingEffectSettlement::Finish {
            outcome: EffectOutcome::Removed,
            error_category: None,
        },
        Err(PermanentSafePlatformError::Cancelled) => PendingEffectSettlement::CancelBeforeCall,
        Err(PermanentSafePlatformError::OutcomeUnknown) => PendingEffectSettlement::Finish {
            outcome: EffectOutcome::OutcomeUnknown,
            error_category: Some(if effect_panicked {
                "permanent_safe_effect_panicked"
            } else {
                "permanent_safe_outcome_unknown"
            }),
        },
        Err(_) => PendingEffectSettlement::Finish {
            outcome: EffectOutcome::Failed,
            error_category: Some("permanent_safe_effect_failed"),
        },
    };
    let unsettled = Box::new(UnsettledPermanentSafeEffect {
        receipt,
        settlement,
        completed_at,
        observed_result: result,
    });
    match unsettled.retry(claim) {
        Ok(Ok(summary)) => Ok(summary),
        Ok(Err(error)) => Err(PermanentSafeExecutionError::Platform(error)),
        Err(unsettled) => Err(PermanentSafeExecutionError::UnsettledEffect(unsettled)),
    }
}

/// The only concrete driver. It inventories the entire target before the
/// first unlink, preserves Cargo's marker, rejects symlinks/special entries
/// and multiply-linked regular files, then removes descendants relative to
/// retained directory descriptors in deepest-first order.
pub(crate) struct DescriptorRelativePermanentSafeDriver;

impl PermanentSafeContentsDriver for DescriptorRelativePermanentSafeDriver {
    fn remove_contents(
        &mut self,
        witness: &RustTargetEffectWitness,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PermanentSafeRemovalSummary, PermanentSafePlatformError> {
        #[cfg(not(unix))]
        {
            let _ = (witness, cancelled);
            return Err(PermanentSafePlatformError::Unsupported);
        }
        #[cfg(unix)]
        {
            remove_contents_unix(witness, cancelled, &mut || {})
        }
    }
}

#[cfg(unix)]
fn remove_contents_unix(
    witness: &RustTargetEffectWitness,
    cancelled: &dyn Fn() -> bool,
    before_final_inventory: &mut dyn FnMut(),
) -> Result<PermanentSafeRemovalSummary, PermanentSafePlatformError> {
    witness
        .revalidate_current()
        .map_err(|_| PermanentSafePlatformError::Changed)?;
    let _ = validated_removal_inventory(
        witness.target_path(),
        witness.target_identity(),
        witness.recency_cutoff(),
        cancelled,
    )?;
    witness
        .revalidate_current()
        .map_err(|_| PermanentSafePlatformError::Changed)?;
    before_final_inventory();
    // The witness revalidation above may perform bounded Cargo/read-set work.
    // Inventory again afterward so activity during that widened window is
    // rejected before the first unlink.
    let (target, mut entries) = validated_removal_inventory(
        witness.target_path(),
        witness.target_identity(),
        witness.recency_cutoff(),
        cancelled,
    )?;
    if !same_directory_identity(&target, witness.target_identity())? {
        return Err(PermanentSafePlatformError::Changed);
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.relative.len()));
    let mut summary = PermanentSafeRemovalSummary::default();
    for entry in entries {
        if cancelled() {
            return if summary.removed_entries == 0 {
                Err(PermanentSafePlatformError::Cancelled)
            } else {
                Err(PermanentSafePlatformError::OutcomeUnknown)
            };
        }
        let parent = open_parent(&target, &entry.parent, &entry.ancestors)?;
        let live = stat_entry(&parent, &entry.name)?;
        if !same_entry_observation(&live, &entry.observation) {
            return Err(if summary.removed_entries == 0 {
                PermanentSafePlatformError::Changed
            } else {
                PermanentSafePlatformError::OutcomeUnknown
            });
        }
        let flags = if entry.observation.is_directory {
            nix::unistd::UnlinkatFlags::RemoveDir
        } else {
            nix::unistd::UnlinkatFlags::NoRemoveDir
        };
        // DUX-DESTRUCTIVE: allow=permanent-safe-rust-target-descriptor-contents -- unlink only reviewed journal-fenced entries relative to retained target descriptors
        nix::unistd::unlinkat(&parent, entry.name.as_os_str(), flags).map_err(|_| {
            if summary.removed_entries == 0 {
                PermanentSafePlatformError::Failed
            } else {
                PermanentSafePlatformError::OutcomeUnknown
            }
        })?;
        summary.removed_entries = summary.removed_entries.saturating_add(1);
        summary.removed_logical_bytes = summary
            .removed_logical_bytes
            .saturating_add(entry.observation.logical_bytes);
    }
    Ok(summary)
}

/// Read-only, bounded descriptor-relative recency validation shared by plan
/// preview and execution. It opens no mutation capability and rejects any
/// root or descendant newer than the supplied inclusive cutoff.
pub(crate) fn validate_rust_target_subtree_recency(
    target_path: &Path,
    target_identity: FilesystemIdentity,
    recency_cutoff: SystemTime,
) -> Result<(), PermanentSafePlatformError> {
    #[cfg(not(unix))]
    {
        let _ = (target_path, target_identity, recency_cutoff);
        Err(PermanentSafePlatformError::Unsupported)
    }
    #[cfg(unix)]
    {
        let _ =
            validated_removal_inventory(target_path, target_identity, recency_cutoff, &|| false)?;
        Ok(())
    }
}

#[cfg(unix)]
fn validated_removal_inventory(
    target_path: &Path,
    target_identity: FilesystemIdentity,
    recency_cutoff: SystemTime,
    cancelled: &dyn Fn() -> bool,
) -> Result<(File, Vec<RemovalEntry>), PermanentSafePlatformError> {
    let target = open_directory(target_path)?;
    if !same_directory_identity(&target, target_identity)? {
        return Err(PermanentSafePlatformError::Changed);
    }
    let target_observation = stat_open_directory(&target)?;
    if target_observation.modified_at > recency_cutoff {
        return Err(PermanentSafePlatformError::Changed);
    }
    let root_observation = DirectoryObservation {
        relative: Vec::new(),
        identity: target_observation.identity,
        modified_at: target_observation.modified_at,
    };
    let mut entries = Vec::new();
    let mut names = 0_usize;
    inventory(
        &target,
        &[],
        std::slice::from_ref(&root_observation),
        &mut entries,
        &mut names,
        recency_cutoff,
        cancelled,
    )?;
    if cancelled() {
        return Err(PermanentSafePlatformError::Cancelled);
    }
    if stat_open_directory(&target)? != target_observation {
        return Err(PermanentSafePlatformError::Changed);
    }
    let mut verification_entries = Vec::new();
    let mut verification_names = 0_usize;
    inventory(
        &target,
        &[],
        std::slice::from_ref(&root_observation),
        &mut verification_entries,
        &mut verification_names,
        recency_cutoff,
        cancelled,
    )?;
    entries.sort_by(|left, right| left.relative.cmp(&right.relative));
    verification_entries.sort_by(|left, right| left.relative.cmp(&right.relative));
    if entries != verification_entries || verification_names != names {
        return Err(PermanentSafePlatformError::Changed);
    }
    entries.retain(|entry| !entry.preserve);
    Ok((target, entries))
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryObservation {
    relative: Vec<Vec<u8>>,
    identity: FilesystemIdentity,
    modified_at: SystemTime,
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct EntryObservation {
    identity: FilesystemIdentity,
    is_directory: bool,
    hard_links: u64,
    logical_bytes: u64,
    modified_at: SystemTime,
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
struct RemovalEntry {
    relative: Vec<Vec<u8>>,
    parent: Vec<Vec<u8>>,
    name: std::ffi::OsString,
    ancestors: Vec<DirectoryObservation>,
    observation: EntryObservation,
    preserve: bool,
}

#[cfg(unix)]
fn open_directory(path: &Path) -> Result<File, PermanentSafePlatformError> {
    use nix::fcntl::{OFlag, open};
    use nix::sys::stat::Mode;
    open(
        path,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| PermanentSafePlatformError::Failed)
}

#[cfg(unix)]
fn stat_identity(file: &File) -> Result<FilesystemIdentity, PermanentSafePlatformError> {
    let stat = nix::sys::stat::fstat(file).map_err(|_| PermanentSafePlatformError::Failed)?;
    Ok(identity_from_stat(&stat))
}

#[cfg(unix)]
fn stat_open_directory(file: &File) -> Result<EntryObservation, PermanentSafePlatformError> {
    let stat = nix::sys::stat::fstat(file).map_err(|_| PermanentSafePlatformError::Failed)?;
    let flags =
        nix::sys::stat::SFlag::from_bits_truncate(stat.st_mode) & nix::sys::stat::SFlag::S_IFMT;
    if flags != nix::sys::stat::SFlag::S_IFDIR {
        return Err(PermanentSafePlatformError::Changed);
    }
    Ok(EntryObservation {
        identity: identity_from_stat(&stat),
        is_directory: true,
        hard_links: stat.st_nlink as u64,
        logical_bytes: 0,
        modified_at: modified_time_from_stat(&stat)?,
    })
}

#[cfg(unix)]
fn same_directory_identity(
    file: &File,
    expected: FilesystemIdentity,
) -> Result<bool, PermanentSafePlatformError> {
    Ok(stat_identity(file)? == expected)
}

#[cfg(unix)]
fn identity_from_stat(stat: &nix::libc::stat) -> FilesystemIdentity {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let volume = stat.st_dev as u64;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let volume = stat.st_dev as u64;
    FilesystemIdentity::new(volume, stat.st_ino as u128)
}

#[cfg(unix)]
fn modified_time_from_stat(
    stat: &nix::libc::stat,
) -> Result<SystemTime, PermanentSafePlatformError> {
    let (seconds, nanoseconds) = (stat.st_mtime, stat.st_mtime_nsec);

    let seconds = u64::try_from(seconds).map_err(|_| PermanentSafePlatformError::Unsafe)?;
    let nanoseconds = u32::try_from(nanoseconds).map_err(|_| PermanentSafePlatformError::Unsafe)?;
    if nanoseconds >= 1_000_000_000 {
        return Err(PermanentSafePlatformError::Unsafe);
    }
    UNIX_EPOCH
        .checked_add(Duration::new(seconds, nanoseconds))
        .ok_or(PermanentSafePlatformError::Unsafe)
}

#[cfg(unix)]
fn stat_entry(parent: &File, name: &OsStr) -> Result<EntryObservation, PermanentSafePlatformError> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::{SFlag, fstatat};
    let stat = fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW)
        .map_err(|_| PermanentSafePlatformError::Changed)?;
    let flags = SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT;
    let is_directory = flags == SFlag::S_IFDIR;
    let is_regular = flags == SFlag::S_IFREG;
    if !is_directory && !is_regular {
        return Err(PermanentSafePlatformError::Unsafe);
    }
    if is_regular && stat.st_nlink != 1 {
        return Err(PermanentSafePlatformError::Unsafe);
    }
    let logical_bytes = if is_regular {
        u64::try_from(stat.st_size).map_err(|_| PermanentSafePlatformError::Unsafe)?
    } else {
        0
    };
    Ok(EntryObservation {
        identity: identity_from_stat(&stat),
        is_directory,
        hard_links: stat.st_nlink as u64,
        logical_bytes,
        modified_at: modified_time_from_stat(&stat)?,
    })
}

#[cfg(unix)]
fn same_entry_observation(actual: &EntryObservation, expected: &EntryObservation) -> bool {
    actual.identity == expected.identity
        && actual.is_directory == expected.is_directory
        && (actual.is_directory
            || (actual.hard_links == expected.hard_links
                && actual.logical_bytes == expected.logical_bytes
                && actual.modified_at == expected.modified_at))
}

#[cfg(unix)]
fn inventory(
    directory: &File,
    relative: &[Vec<u8>],
    ancestors: &[DirectoryObservation],
    entries: &mut Vec<RemovalEntry>,
    name_bytes: &mut usize,
    recency_cutoff: SystemTime,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), PermanentSafePlatformError> {
    use nix::dir::Dir;
    use nix::fcntl::{OFlag, openat};
    use nix::sys::stat::Mode;
    use std::os::unix::ffi::OsStringExt;

    if relative.len() > MAX_DESCENDANT_DEPTH {
        return Err(PermanentSafePlatformError::Unsafe);
    }
    let clone = directory
        .try_clone()
        .map_err(|_| PermanentSafePlatformError::Failed)?;
    let mut dir = Dir::from_fd(clone.into()).map_err(|_| PermanentSafePlatformError::Failed)?;
    for item in dir.iter() {
        if cancelled() {
            return Err(PermanentSafePlatformError::Cancelled);
        }
        let item = item.map_err(|_| PermanentSafePlatformError::Failed)?;
        let bytes = item.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        if bytes.is_empty() || bytes.contains(&b'/') {
            return Err(PermanentSafePlatformError::Unsafe);
        }
        *name_bytes = name_bytes
            .checked_add(bytes.len())
            .ok_or(PermanentSafePlatformError::Unsafe)?;
        if *name_bytes > MAX_DESCENDANT_NAME_BYTES || entries.len() >= MAX_DESCENDANT_ENTRIES {
            return Err(PermanentSafePlatformError::Unsafe);
        }
        let name = std::ffi::OsString::from_vec(bytes.to_vec());
        let observation = stat_entry(directory, name.as_os_str())?;
        if observation.modified_at > recency_cutoff {
            return Err(PermanentSafePlatformError::Changed);
        }
        let preserve = relative.is_empty() && bytes == PRESERVED_MARKER;
        if preserve && observation.is_directory {
            return Err(PermanentSafePlatformError::Unsafe);
        }
        let mut child_relative = relative.to_vec();
        child_relative.push(bytes.to_vec());
        if observation.is_directory {
            let child = openat(
                directory,
                name.as_os_str(),
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .map(File::from)
            .map_err(|_| PermanentSafePlatformError::Changed)?;
            if stat_identity(&child)? != observation.identity {
                return Err(PermanentSafePlatformError::Changed);
            }
            let mut child_ancestors = ancestors.to_vec();
            child_ancestors.push(DirectoryObservation {
                relative: child_relative.clone(),
                identity: observation.identity,
                modified_at: observation.modified_at,
            });
            inventory(
                &child,
                &child_relative,
                &child_ancestors,
                entries,
                name_bytes,
                recency_cutoff,
                cancelled,
            )?;
        }
        entries.push(RemovalEntry {
            relative: child_relative,
            parent: relative.to_vec(),
            name,
            ancestors: ancestors.to_vec(),
            observation,
            preserve,
        });
    }
    Ok(())
}

#[cfg(unix)]
fn open_parent(
    root: &File,
    relative: &[Vec<u8>],
    ancestors: &[DirectoryObservation],
) -> Result<File, PermanentSafePlatformError> {
    use nix::fcntl::{OFlag, openat};
    use nix::sys::stat::Mode;
    use std::os::unix::ffi::OsStrExt;
    let mut current = root
        .try_clone()
        .map_err(|_| PermanentSafePlatformError::Failed)?;
    for (index, component) in relative.iter().enumerate() {
        let name = OsStr::from_bytes(component);
        let next = openat(
            &current,
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| PermanentSafePlatformError::Changed)?;
        let expected = ancestors
            .iter()
            .find(|entry| entry.relative.as_slice() == &relative[..=index])
            .ok_or(PermanentSafePlatformError::Changed)?;
        if stat_identity(&next)? != expected.identity {
            return Err(PermanentSafePlatformError::Changed);
        }
        current = next;
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use tempfile::TempDir;

    #[test]
    fn bounds_and_marker_are_explicit() {
        assert_eq!(PRESERVED_MARKER, b"CACHEDIR.TAG");
        assert_eq!(MAX_DESCENDANT_DEPTH, 64);
        assert_eq!(MAX_DESCENDANT_ENTRIES, 16_384);
    }

    #[test]
    fn witness_failures_map_to_terminal_validation_outcomes() {
        let changed = crate::planner::ExactPathHandoffError::RuleEvidence(
            crate::planner::RustTargetLiveValidationError::ChangedDuringValidation,
        );
        assert_eq!(
            validation_failure(&changed),
            (
                crate::persistence::ValidationOutcome::ChangedSincePlan,
                "permanent_safe_target_changed",
            )
        );

        let stale = crate::planner::ExactPathHandoffError::Approval(
            crate::planner::ExactPathApprovalError::Expired,
        );
        assert_eq!(
            validation_failure(&stale),
            (
                crate::persistence::ValidationOutcome::Rejected,
                "permanent_safe_approval_stale",
            )
        );

        let journal = crate::planner::ExactPathHandoffError::Journal(
            crate::persistence::HistoryError::new(HistoryErrorKind::DatabaseUnavailable),
        );
        assert_eq!(
            validation_failure(&journal),
            (
                crate::persistence::ValidationOutcome::Unavailable,
                "permanent_safe_journal_unavailable",
            )
        );
    }

    #[cfg(unix)]
    fn witness_fixture() -> (TempDir, RustTargetEffectWitness) {
        let recency_cutoff = SystemTime::now() - Duration::from_secs(7 * 86_400);
        witness_fixture_at(recency_cutoff - Duration::from_secs(86_400), recency_cutoff)
    }

    #[cfg(unix)]
    fn witness_fixture_at(
        modified_at: SystemTime,
        recency_cutoff: SystemTime,
    ) -> (TempDir, RustTargetEffectWitness) {
        use crate::path_validation::{
            capture_path_snapshot, capture_scan_root, validate_cleanup_path, validate_scan_root,
        };
        let temp = tempfile::tempdir().unwrap();
        let scan_root_path = fs::canonicalize(temp.path()).unwrap();
        let project = scan_root_path.join("project");
        let target = project.join("target");
        fs::create_dir_all(target.join("debug/deps")).unwrap();
        fs::write(
            project.join("Cargo.toml"),
            b"[package]\nname = \"fixture\"\n",
        )
        .unwrap();
        fs::write(
            target.join("CACHEDIR.TAG"),
            b"Signature: 8a477f597d28d172789f06886806bc55\n",
        )
        .unwrap();
        fs::write(target.join("debug/deps/libfixture.rlib"), b"artifact").unwrap();
        fs::write(target.join("debug/.fingerprint"), b"artifact").unwrap();
        crate::planner::rust_target_tests::set_subtree_modified_at(&target, modified_at);
        let lexical_root = validate_scan_root(&scan_root_path).unwrap();
        let scan_root = capture_scan_root(lexical_root.clone()).unwrap();
        let lexical_target = validate_cleanup_path(&lexical_root, &target).unwrap();
        let snapshot = capture_path_snapshot(&scan_root, lexical_target).unwrap();
        let witness =
            crate::planner::validate_rust_target_effect(snapshot, recency_cutoff).unwrap();
        (temp, witness)
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_driver_removes_only_descendants_and_preserves_marker() {
        let (temp, witness) = witness_fixture();
        let mut driver = DescriptorRelativePermanentSafeDriver;
        let summary = driver
            .remove_contents(&witness, &|| false)
            .expect("fixture should be safe");
        assert_eq!(summary.removed_entries, 4);
        assert!(summary.removed_logical_bytes > 0);
        assert!(temp.path().join("project/target/CACHEDIR.TAG").exists());
        assert!(!temp.path().join("project/target/debug").exists());
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_driver_rejects_symlink_before_mutation() {
        use std::os::unix::fs::symlink;
        let (temp, witness) = witness_fixture();
        symlink("/tmp", temp.path().join("project/target/debug/link")).unwrap();
        std::fs::File::open(temp.path().join("project/target/debug"))
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(witness.recency_cutoff() - Duration::from_secs(1)),
            )
            .unwrap();
        let mut driver = DescriptorRelativePermanentSafeDriver;
        assert_eq!(
            driver.remove_contents(&witness, &|| false),
            Err(PermanentSafePlatformError::Unsafe)
        );
        assert!(
            temp.path()
                .join("project/target/debug/deps/libfixture.rlib")
                .exists()
        );
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_driver_rejects_recent_descendant_before_first_unlink() {
        let (temp, witness) = witness_fixture();
        let target = temp.path().join("project/target");
        let recent = target.join("debug/deps/recent.rlib");
        fs::write(&recent, b"recent").unwrap();
        let original = target.join("debug/deps/libfixture.rlib");
        let fingerprint = target.join("debug/.fingerprint");

        let mut driver = DescriptorRelativePermanentSafeDriver;
        assert_eq!(
            driver.remove_contents(&witness, &|| false),
            Err(PermanentSafePlatformError::Changed)
        );
        assert!(target.join("CACHEDIR.TAG").exists());
        assert!(original.exists());
        assert!(fingerprint.exists());
        assert!(recent.exists());
    }

    #[cfg(unix)]
    #[test]
    fn final_inventory_rejects_leaf_activity_after_witness_revalidation() {
        let (temp, witness) = witness_fixture();
        let target = temp.path().join("project/target");
        let changed_leaf = target.join("debug/deps/libfixture.rlib");
        let untouched_leaf = target.join("debug/.fingerprint");
        let mut touch_leaf = || {
            File::open(&changed_leaf)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(SystemTime::now()))
                .unwrap();
        };

        assert_eq!(
            remove_contents_unix(&witness, &|| false, &mut touch_leaf),
            Err(PermanentSafePlatformError::Changed)
        );
        assert!(target.join("CACHEDIR.TAG").exists());
        assert!(changed_leaf.exists());
        assert!(untouched_leaf.exists());
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_recency_cutoff_is_inclusive() {
        let cutoff = SystemTime::now() - Duration::from_secs(7 * 86_400);
        let (_temp, witness) = witness_fixture_at(cutoff, cutoff);

        let mut driver = DescriptorRelativePermanentSafeDriver;
        let summary = driver.remove_contents(&witness, &|| false).unwrap();
        assert_eq!(summary.removed_entries, 4);
    }
}
