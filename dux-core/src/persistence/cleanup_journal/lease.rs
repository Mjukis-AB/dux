//! Cleanup-lock-coupled access to the mutable operation journal.
//!
//! The raw SQL state machine lives in the parent module and is reachable only
//! through these non-cloneable wrappers. Holding a claim proves process/store
//! exclusion and an exact database owner generation; it still grants no path,
//! plan, approval, or filesystem-effect authority.

use std::cell::Cell;
#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::collections::VecDeque;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rusqlite::{Transaction, TransactionBehavior};

use super::{
    ActivePhase, CleanupJournal, EffectOutcome, ExecutionFence, JournalLifecycle, PathStatus,
    ReconciledOutcome, RecoveryAssessment, TerminalSessionStatus, ValidationOutcome,
    assess_recovery, begin_path_validation, cancel_effect_before_call, claim_planned,
    claim_recovery, expire_planned, finish_effect, finish_path_validation, load_cleanup_journal,
    mark_effect_started, reconcile_unknown_outcome, record_heartbeat, request_cancellation,
    resume_recovery, settle_cancellation, terminalize,
};
use crate::domain::CleanupPlan;
use crate::persistence::cleanup_history::{CleanupSessionId, CleanupTrigger};
use crate::persistence::history::{
    HistoryError, HistoryErrorKind, system_time_to_unix_ms, unix_ms_to_system_time,
};
use crate::persistence::process_liveness::{
    ProcessIdentityError, ProcessInstanceId, current_process_instance,
};
use crate::persistence::storage::CleanupLockGuard;
use crate::persistence::store::StoreCoordinator;

/// A held store-wide cleanup lock before a journal owner has been claimed.
///
/// The lease is deliberately non-cloneable and dropping it performs no journal
/// write. A crashed or abandoned active owner remains recoverable only through
/// the conservative process-liveness protocol.
pub(crate) struct CleanupJournalLease {
    guard: CleanupLockGuard,
    store: Arc<StoreCoordinator>,
    owner: ProcessInstanceId,
    // Claims may move to an engine worker but must not be shared concurrently.
    // The future engine-level cleanup mutex remains a separate outer boundary.
    _not_sync: PhantomData<Cell<()>>,
}

/// One exact active owner generation bound to the held cleanup lock.
pub(crate) struct CleanupJournalClaim {
    lease: CleanupJournalLease,
    fence: ExecutionFence,
    phase: ActivePhase,
    pending_effect_start: Cell<Option<PendingEffectStart>>,
    #[cfg(test)]
    test_fault: Cell<TestJournalFault>,
}

/// A terminal outcome from validation-only dry-run orchestration.
///
/// Every refusal carries a stable, non-sensitive category. `Cancelled`
/// differs from `Interrupted` only in the terminal parent classification; both
/// persist an interrupted path without effect evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValidatedDryRunOutcome {
    DryRun,
    ChangedSincePlan(&'static str),
    Rejected(&'static str),
    Unavailable(&'static str),
    Interrupted(&'static str),
    Cancelled(&'static str),
    Failed(&'static str),
}

impl ValidatedDryRunOutcome {
    fn journal_parts(self) -> (ValidationOutcome, Option<&'static str>, bool) {
        match self {
            Self::DryRun => (ValidationOutcome::DryRun, None, false),
            Self::ChangedSincePlan(error) => {
                (ValidationOutcome::ChangedSincePlan, Some(error), false)
            }
            Self::Rejected(error) => (ValidationOutcome::Rejected, Some(error), false),
            Self::Unavailable(error) => (ValidationOutcome::Unavailable, Some(error), false),
            Self::Interrupted(error) => (ValidationOutcome::Interrupted, Some(error), false),
            Self::Cancelled(error) => (ValidationOutcome::Interrupted, Some(error), true),
            Self::Failed(error) => (ValidationOutcome::Failed, Some(error), false),
        }
    }
}

pub(in crate::persistence) enum RecoveryClaimResult {
    Claimed(Box<CleanupJournalClaim>),
    OwnerAlive,
    LivenessUnknown,
    NotRecoverable,
}

enum RecoveryDecision {
    Claimed(ExecutionFence),
    OwnerAlive,
    LivenessUnknown,
    NotRecoverable,
}

/// A failed owner-claim attempt retains the cleanup lease so an ambiguous
/// commit can be reconciled without abandoning a live owner in the database.
#[must_use = "retain the lease and reconcile or deliberately release it"]
pub(crate) struct JournalLeaseFailure {
    lease: CleanupJournalLease,
    error: HistoryError,
}

/// A failed validation-only journal write that retains the cleanup lease.
///
/// Retaining the lease lets a caller retry the exact same operation after an
/// ambiguous storage result. A retry succeeds only when the complete durable
/// graph exactly matches the requested observation.
#[must_use = "retain the lease and retry the exact observation or deliberately release it"]
pub(crate) struct DryRunJournalFailure {
    lease: CleanupJournalLease,
    error: HistoryError,
    may_have_committed: bool,
}

impl DryRunJournalFailure {
    pub(crate) fn kind(&self) -> HistoryErrorKind {
        self.error.kind
    }

    pub(crate) fn into_lease(self) -> CleanupJournalLease {
        self.lease
    }

    pub(crate) const fn may_have_committed(&self) -> bool {
        self.may_have_committed
    }
}

impl fmt::Debug for DryRunJournalFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DryRunJournalFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl JournalLeaseFailure {
    pub(crate) fn kind(&self) -> HistoryErrorKind {
        self.error.kind
    }

    pub(crate) fn into_lease(self) -> CleanupJournalLease {
        self.lease
    }
}

impl fmt::Debug for JournalLeaseFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JournalLeaseFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// Durable pre-effect journal receipt. It is evidence of ordering only and is
/// not target identity, validation, approval, or effect authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EffectStartReceipt {
    fence: ExecutionFence,
    item_ordinal: usize,
    path_ordinal: usize,
    ordered_at: SystemTime,
    started_at: SystemTime,
}

#[derive(Clone, Copy)]
struct PendingEffectStart {
    item_ordinal: usize,
    path_ordinal: usize,
    ordered_at: SystemTime,
}

struct JournalWriteFailure {
    error: HistoryError,
    may_have_committed: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestJournalFault {
    None,
    FailAfterCommitThenReconcileRead,
    FailReconcileRead,
}

#[cfg(test)]
thread_local! {
    static LEASE_TEST_FAULT: Cell<TestJournalFault> = const { Cell::new(TestJournalFault::None) };
    static LEASE_TEST_RECONCILE_READ_FAILURES: Cell<u8> = const { Cell::new(0) };
    static LEASE_TEST_AMBIGUOUS_WRITE_SCHEDULE: RefCell<VecDeque<u8>> =
        const { RefCell::new(VecDeque::new()) };
}

#[cfg(test)]
pub(crate) fn fail_next_write_after_commit_and_reconcile_read_for_test() {
    set_lease_ambiguous_write_schedule_for_test([1]);
}

#[cfg(test)]
pub(crate) fn fail_next_write_after_commit_and_two_reconcile_reads_for_test() {
    set_lease_ambiguous_write_schedule_for_test([2]);
}

#[cfg(test)]
fn set_lease_ambiguous_write_schedule_for_test(
    reconcile_read_failures: impl IntoIterator<Item = u8>,
) {
    LEASE_TEST_FAULT.with(|fault| {
        assert_eq!(fault.get(), TestJournalFault::None);
    });
    LEASE_TEST_RECONCILE_READ_FAILURES.with(|reads| {
        assert_eq!(reads.get(), 0);
    });
    LEASE_TEST_AMBIGUOUS_WRITE_SCHEDULE.with(|schedule| {
        let mut schedule = schedule.borrow_mut();
        assert!(schedule.is_empty());
        schedule.extend(reconcile_read_failures);
        assert!(schedule.iter().all(|failures| *failures > 0));
    });
}

impl StoreCoordinator {
    pub(crate) fn acquire_cleanup_journal_lease(
        self: &Arc<Self>,
        timeout: Duration,
    ) -> Result<CleanupJournalLease, HistoryError> {
        let guard = self.acquire_cleanup_lock_for_journal(timeout)?;
        self.validate_cleanup_lock_for_journal(&guard)?;
        let owner = current_process_instance().map_err(map_process_identity_error)?;
        Ok(CleanupJournalLease {
            guard,
            store: Arc::clone(self),
            owner,
            _not_sync: PhantomData,
        })
    }
}

impl CleanupJournalLease {
    /// Durably record one already validated DryRun observation without ever
    /// creating an execution owner or an effect-capable journal claim.
    ///
    /// The exact plan is inserted uncoupled and terminalized in one SQLite
    /// transaction while this value retains the store-wide cleanup lock. A
    /// deny-only user exclusion observed under the same lock converts only an
    /// otherwise successful observation to a durable
    /// `Rejected("user_excluded")` record.
    pub(crate) fn record_validated_dry_run(
        self,
        session_id: CleanupSessionId,
        plan: &CleanupPlan,
        trigger: CleanupTrigger,
        requested_outcome: ValidatedDryRunOutcome,
        started_at: SystemTime,
        completed_at: SystemTime,
    ) -> Result<TerminalSessionStatus, DryRunJournalFailure> {
        let attempt = (|| {
            if plan.mode() != crate::domain::CleanupMode::DryRun {
                return Err(definite_write_failure(HistoryError::new(
                    HistoryErrorKind::InvalidInput,
                )));
            }
            let started_at = crate::persistence::canonical_started_at(started_at)
                .map_err(definite_write_failure)?;
            let completed_at = crate::persistence::canonical_started_at(completed_at)
                .map_err(definite_write_failure)?;
            if completed_at < started_at {
                return Err(definite_write_failure(HistoryError::new(
                    HistoryErrorKind::InvalidInput,
                )));
            }
            let outcome = if matches!(requested_outcome, ValidatedDryRunOutcome::DryRun)
                && self
                    .plan_contains_user_exclusion(plan)
                    .map_err(definite_write_failure)?
            {
                ValidatedDryRunOutcome::Rejected("user_excluded")
            } else {
                requested_outcome
            };
            let (validation, error_category, cancellation_requested) = outcome.journal_parts();
            let expected_status = terminal_status_for_observed_dry_run(outcome);
            match self.write_classified(|transaction| {
                super::record_validated_dry_run(
                    transaction,
                    session_id.clone(),
                    plan,
                    trigger,
                    validation,
                    error_category,
                    cancellation_requested,
                    started_at,
                    completed_at,
                )
            }) {
                Ok(status) => Ok(status),
                Err(failure) => {
                    let reconciled = self
                        .load(&session_id)
                        .ok()
                        .flatten()
                        .is_some_and(|journal| {
                            observed_dry_run_matches(
                                &journal,
                                plan,
                                trigger,
                                outcome,
                                started_at,
                                completed_at,
                                expected_status,
                            )
                        });
                    if reconciled {
                        Ok(expected_status)
                    } else {
                        Err(failure)
                    }
                }
            }
        })();
        match attempt {
            Ok(status) => Ok(status),
            Err(failure) => Err(DryRunJournalFailure {
                lease: self,
                error: failure.error,
                may_have_committed: failure.may_have_committed,
            }),
        }
    }

    fn plan_contains_user_exclusion(&self, plan: &CleanupPlan) -> Result<bool, HistoryError> {
        self.store.validate_cleanup_lock_for_journal(&self.guard)?;
        let connection = self.store.lock_current_history_connection()?;
        self.store.validate_cleanup_lock_for_journal(&self.guard)?;
        let exclusions = crate::persistence::load_cleanup_exclusions(&connection.connection)?;
        Ok(plan
            .items()
            .iter()
            .flat_map(|item| item.paths())
            .any(|path| exclusions.contains_path(path)))
    }

    /// Compare a planned history observation before claiming it. The caller
    /// still must claim and compare again under the journal owner fence.
    pub(crate) fn validate_planned_plan(
        &self,
        session_id: &CleanupSessionId,
        expected: &CleanupPlan,
    ) -> Result<(), HistoryError> {
        let journal = self
            .load(session_id)?
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
        validate_journal_plan(&journal, expected)
    }

    #[cfg(test)]
    pub(super) fn fail_next_write_after_commit_and_reconcile_read_for_test(&self) {
        fail_next_write_after_commit_and_reconcile_read_for_test();
    }

    pub(super) fn load(
        &self,
        session_id: &CleanupSessionId,
    ) -> Result<Option<CleanupJournal>, HistoryError> {
        #[cfg(test)]
        if LEASE_TEST_RECONCILE_READ_FAILURES.with(|reads| {
            let remaining = reads.get();
            if remaining > 0 {
                reads.set(remaining - 1);
                true
            } else {
                false
            }
        }) {
            return Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable));
        }
        #[cfg(test)]
        if LEASE_TEST_FAULT.with(|fault| {
            let should_fail = fault.get() == TestJournalFault::FailReconcileRead;
            if should_fail {
                fault.set(TestJournalFault::None);
            }
            should_fail
        }) {
            return Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable));
        }
        self.store.validate_cleanup_lock_for_journal(&self.guard)?;
        let connection = self.store.lock_current_history_connection()?;
        self.store.validate_cleanup_lock_for_journal(&self.guard)?;
        load_cleanup_journal(&connection.connection, session_id)
    }

    pub(crate) fn claim_planned(
        self,
        session_id: &CleanupSessionId,
        claimed_at: SystemTime,
    ) -> Result<CleanupJournalClaim, JournalLeaseFailure> {
        let attempt = canonical_input_time(claimed_at).and_then(|expected_heartbeat| {
            let owner = self.owner.clone();
            let expected = ExecutionFence {
                session_id: session_id.clone(),
                owner: owner.clone(),
                generation: 1,
            };
            match self.write(|transaction| {
                let fence = claim_planned(transaction, session_id, owner, claimed_at)?;
                let claimed = load_cleanup_journal(transaction, session_id)?
                    .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
                ensure_active(&claimed, &fence, ActivePhase::Running)?;
                Ok(fence)
            }) {
                Ok(fence) => Ok(fence),
                Err(error) => {
                    // A commit error or post-commit storage failure is
                    // outcome-ambiguous. The lease's random owner is unique,
                    // so this exact state can only be our committed claim.
                    if self.load(session_id).ok().flatten().is_some_and(|journal| {
                        active_matches(
                            &journal,
                            &expected,
                            ActivePhase::Running,
                            Some(expected_heartbeat),
                            Some(false),
                        )
                    }) {
                        Ok(expected)
                    } else {
                        Err(error)
                    }
                }
            }
        });
        match attempt {
            Ok(fence) => Ok(CleanupJournalClaim {
                lease: self,
                fence,
                phase: ActivePhase::Running,
                pending_effect_start: Cell::new(None),
                #[cfg(test)]
                test_fault: Cell::new(TestJournalFault::None),
            }),
            Err(error) => Err(JournalLeaseFailure { lease: self, error }),
        }
    }

    /// Settle a pristine plan at its exact expiry or later. This creates only
    /// rejected history and failed candidate projections; it never grants a
    /// running claim or filesystem-effect authority.
    pub(super) fn expire_planned(
        self,
        session_id: &CleanupSessionId,
        observed_at: SystemTime,
    ) -> Result<TerminalSessionStatus, JournalLeaseFailure> {
        let attempt =
            super::canonical_expiry_settlement_time(observed_at).and_then(|completed_at| {
                let expected = ExecutionFence {
                    session_id: session_id.clone(),
                    owner: self.owner.clone(),
                    generation: 1,
                };
                match self.write(|transaction| {
                    let fence =
                        expire_planned(transaction, session_id, self.owner.clone(), observed_at)?;
                    let settled = load_cleanup_journal(transaction, session_id)?
                        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
                    if !expired_terminal_matches(&settled, &fence, completed_at) {
                        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
                    }
                    Ok(())
                }) {
                    Ok(()) => Ok(TerminalSessionStatus::Rejected),
                    Err(_)
                        if self.load(session_id).ok().flatten().is_some_and(|journal| {
                            expired_terminal_matches(&journal, &expected, completed_at)
                        }) =>
                    {
                        Ok(TerminalSessionStatus::Rejected)
                    }
                    Err(error) => Err(error),
                }
            });
        match attempt {
            Ok(status) => Ok(status),
            Err(error) => Err(JournalLeaseFailure { lease: self, error }),
        }
    }

    /// Attempt same-scope recovery while retaining the cleanup lock but never
    /// a database connection during the operating-system liveness probe.
    pub(super) fn try_recover(
        self,
        session_id: &CleanupSessionId,
        claimed_at: SystemTime,
    ) -> Result<RecoveryClaimResult, JournalLeaseFailure> {
        let decision = canonical_input_time(claimed_at).and_then(|expected_heartbeat| {
            let Some(snapshot) = self.load(session_id)? else {
                return Ok(RecoveryDecision::NotRecoverable);
            };
            let JournalLifecycle::Active {
                phase,
                fence: observed_fence,
                heartbeat_at: _,
                cancellation_requested,
            } = &snapshot.lifecycle
            else {
                return Ok(RecoveryDecision::NotRecoverable);
            };
            // Retry reconciliation after an ambiguous recovery commit. This
            // lease's owner nonce cannot have been selected by another claim.
            if observed_fence.owner == self.owner && *phase == ActivePhase::Recovering {
                return Ok(RecoveryDecision::Claimed(observed_fence.clone()));
            }
            match assess_recovery(&snapshot)? {
                RecoveryAssessment::OwnerAlive => Ok(RecoveryDecision::OwnerAlive),
                RecoveryAssessment::LivenessUnknown => Ok(RecoveryDecision::LivenessUnknown),
                RecoveryAssessment::Recoverable(permit) => {
                    let expected = ExecutionFence {
                        session_id: session_id.clone(),
                        owner: self.owner.clone(),
                        generation: observed_fence.generation.checked_add(1).ok_or_else(|| {
                            HistoryError::new(HistoryErrorKind::InvalidTransition)
                        })?,
                    };
                    let owner = self.owner.clone();
                    match self
                        .write(|transaction| claim_recovery(transaction, permit, owner, claimed_at))
                    {
                        Ok(fence) => Ok(RecoveryDecision::Claimed(fence)),
                        Err(error) => {
                            if self.load(session_id).ok().flatten().is_some_and(|journal| {
                                active_matches(
                                    &journal,
                                    &expected,
                                    ActivePhase::Recovering,
                                    Some(expected_heartbeat),
                                    Some(*cancellation_requested),
                                )
                            }) {
                                Ok(RecoveryDecision::Claimed(expected))
                            } else {
                                Err(error)
                            }
                        }
                    }
                }
            }
        });
        match decision {
            Ok(RecoveryDecision::Claimed(fence)) => Ok(RecoveryClaimResult::Claimed(Box::new(
                CleanupJournalClaim {
                    lease: self,
                    fence,
                    phase: ActivePhase::Recovering,
                    pending_effect_start: Cell::new(None),
                    #[cfg(test)]
                    test_fault: Cell::new(TestJournalFault::None),
                },
            ))),
            Ok(RecoveryDecision::OwnerAlive) => Ok(RecoveryClaimResult::OwnerAlive),
            Ok(RecoveryDecision::LivenessUnknown) => Ok(RecoveryClaimResult::LivenessUnknown),
            Ok(RecoveryDecision::NotRecoverable) => Ok(RecoveryClaimResult::NotRecoverable),
            Err(error) => Err(JournalLeaseFailure { lease: self, error }),
        }
    }

    fn write<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T, HistoryError>,
    ) -> Result<T, HistoryError> {
        self.write_classified(operation)
            .map_err(|failure| failure.error)
    }

    fn write_classified<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T, HistoryError>,
    ) -> Result<T, JournalWriteFailure> {
        self.store
            .validate_cleanup_lock_for_journal(&self.guard)
            .map_err(definite_write_failure)?;
        let mut connection = self
            .store
            .lock_current_history_connection()
            .map_err(definite_write_failure)?;
        // Connection and writer acquisition may wait. Revalidate the retained
        // cleanup control after that wait and before BEGIN IMMEDIATE.
        self.store
            .validate_cleanup_lock_for_journal(&self.guard)
            .map_err(definite_write_failure)?;
        let transaction = connection
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(super::map_write_sql_error)
            .map_err(definite_write_failure)?;
        let output = operation(&transaction).map_err(definite_write_failure)?;
        self.store
            .validate_cleanup_lock_for_journal(&self.guard)
            .map_err(definite_write_failure)?;
        transaction
            .commit()
            .map_err(super::map_write_sql_error)
            .map_err(ambiguous_write_failure)?;
        drop(connection);
        self.store
            .validate_history_storage_after_write()
            .map_err(ambiguous_write_failure)?;
        #[cfg(test)]
        if LEASE_TEST_AMBIGUOUS_WRITE_SCHEDULE.with(|schedule| {
            let Some(reconcile_reads) = schedule.borrow_mut().pop_front() else {
                return false;
            };
            LEASE_TEST_RECONCILE_READ_FAILURES.with(|reads| {
                assert_eq!(reads.get(), 0);
                reads.set(reconcile_reads);
            });
            true
        }) {
            return Err(ambiguous_write_failure(HistoryError::new(
                HistoryErrorKind::DatabaseUnavailable,
            )));
        }
        #[cfg(test)]
        if LEASE_TEST_FAULT.with(|fault| {
            let should_fail = fault.get() == TestJournalFault::FailAfterCommitThenReconcileRead;
            if should_fail {
                fault.set(TestJournalFault::FailReconcileRead);
            }
            should_fail
        }) {
            return Err(ambiguous_write_failure(HistoryError::new(
                HistoryErrorKind::DatabaseUnavailable,
            )));
        }
        Ok(output)
    }
}

/// Shared exact-plan comparison used both before and after claiming. Keeping
/// this comparison path-free and journal-owned prevents callers from
/// substituting a row with the same session ID but different content.
fn validate_journal_plan(
    journal: &CleanupJournal,
    expected: &CleanupPlan,
) -> Result<(), HistoryError> {
    validate_frozen_journal_plan(journal, expected)?;
    if journal.items.iter().any(|item| {
        item.status != PathStatus::Planned
            || item.error_category.is_some()
            || item.paths.iter().any(|path| {
                path.status != PathStatus::Planned
                    || path.attempt_generation.is_some()
                    || path.error_category.is_some()
                    || path.effect_started_at.is_some()
                    || path.completed_at.is_some()
            })
    }) {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok(())
}

fn validate_frozen_journal_plan(
    journal: &CleanupJournal,
    expected: &CleanupPlan,
) -> Result<(), HistoryError> {
    if journal.plan_id != *expected.id()
        || journal.source_scan_id != *expected.source_scan_id()
        || journal.plan_created_at != expected.created_at()
        || journal.plan_expires_at != expected.expires_at()
        || journal.mode != expected.mode()
        || journal.estimated_bytes != expected.estimated_bytes()
        || journal.warnings != expected.warnings()
        || journal.items.len() != expected.items().len()
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    for (ordinal, (journal_item, expected_item)) in
        journal.items.iter().zip(expected.items()).enumerate()
    {
        let frozen = &journal_item.frozen;
        if frozen.ordinal != ordinal
            || frozen.candidate_id != *expected_item.candidate_id()
            || frozen.rule != *expected_item.rule()
            || frozen.category != expected_item.category()
            || frozen.paths != expected_item.paths()
            || frozen.estimated_bytes != expected_item.estimated_bytes()
            || frozen.newest_mtime != expected_item.newest_mtime()
            || frozen.evidence != expected_item.evidence()
            || frozen.safety != expected_item.safety()
            || frozen.proposed_action != expected_item.action()
            || frozen.rule_schedule_eligible != expected_item.rule_marks_schedule_eligible()
            || journal_item.paths.len() != expected_item.paths().len()
        {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        for (path, expected_path) in journal_item.paths.iter().zip(expected_item.paths()) {
            if path.target != *expected_path {
                return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
            }
        }
    }
    Ok(())
}

fn terminal_status_for_observed_dry_run(outcome: ValidatedDryRunOutcome) -> TerminalSessionStatus {
    match outcome {
        ValidatedDryRunOutcome::DryRun => TerminalSessionStatus::DryRun,
        ValidatedDryRunOutcome::ChangedSincePlan(_)
        | ValidatedDryRunOutcome::Unavailable(_)
        | ValidatedDryRunOutcome::Failed(_) => TerminalSessionStatus::Failed,
        ValidatedDryRunOutcome::Rejected(_) => TerminalSessionStatus::Rejected,
        ValidatedDryRunOutcome::Interrupted(_) => TerminalSessionStatus::Interrupted,
        ValidatedDryRunOutcome::Cancelled(_) => TerminalSessionStatus::Cancelled,
    }
}

fn observed_dry_run_matches(
    journal: &CleanupJournal,
    expected_plan: &CleanupPlan,
    expected_trigger: CleanupTrigger,
    outcome: ValidatedDryRunOutcome,
    started_at: SystemTime,
    completed_at: SystemTime,
    expected_status: TerminalSessionStatus,
) -> bool {
    if validate_frozen_journal_plan(journal, expected_plan).is_err()
        || journal.trigger != expected_trigger
        || journal.started_at != started_at
        || journal.candidate_status_coupling
            != crate::persistence::cleanup_history::CandidateStatusCoupling::LegacyUncoupled
    {
        return false;
    }
    let (_, error_category, cancellation_requested) = outcome.journal_parts();
    let expected_path_status = PathStatus::from(outcome.journal_parts().0);
    matches!(
        journal.lifecycle,
        JournalLifecycle::ObservedTerminal {
            status,
            completed_at: loaded_completed_at,
            cancellation_requested: loaded_cancellation,
        } if status == expected_status
            && loaded_completed_at == completed_at
            && loaded_cancellation == cancellation_requested
    ) && journal.items.iter().all(|item| {
        item.status == expected_path_status
            && item.error_category.as_deref() == error_category
            && item.paths.iter().all(|path| {
                path.status == expected_path_status
                    && path.attempt_generation == Some(1)
                    && path.error_category.as_deref() == error_category
                    && path.effect_started_at.is_none()
                    && path.completed_at == Some(completed_at)
            })
    })
}

impl CleanupJournalClaim {
    /// Verify that the active journal is the exact frozen observation owned by
    /// the approved planner capability. This is deliberately a comparison
    /// witness only; it does not grant permission to perform an effect.
    pub(crate) fn validate_planned_plan(&self, expected: &CleanupPlan) -> Result<(), HistoryError> {
        self.require_phase(ActivePhase::Running)?;
        let journal = self.snapshot()?;
        ensure_active(&journal, &self.fence, self.phase)?;
        validate_journal_plan(&journal, expected)
    }

    #[cfg(test)]
    pub(super) fn fail_next_write_after_commit_and_reconcile_read_for_test(&self) {
        assert_eq!(self.test_fault.get(), TestJournalFault::None);
        self.test_fault
            .set(TestJournalFault::FailAfterCommitThenReconcileRead);
    }

    pub(super) fn snapshot(&self) -> Result<CleanupJournal, HistoryError> {
        #[cfg(test)]
        if self.test_fault.get() == TestJournalFault::FailReconcileRead {
            self.test_fault.set(TestJournalFault::None);
            return Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable));
        }
        self.lease
            .load(&self.fence.session_id)?
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))
    }

    /// Bind an opaque reviewed target to the exact frozen path row before any
    /// validation transition. This prevents pairing a journal fence for one
    /// planned item with filesystem evidence for another.
    pub(crate) fn validate_planned_path(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
        expected_path: &std::path::Path,
    ) -> Result<(), HistoryError> {
        self.require_phase(ActivePhase::Running)?;
        let journal = self.snapshot()?;
        ensure_active(&journal, &self.fence, self.phase)?;
        let item = journal
            .items
            .get(item_ordinal)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
        let path = item
            .paths
            .get(path_ordinal)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
        if path.status != PathStatus::Planned || path.target != expected_path {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        Ok(())
    }

    /// Confirm that the claimed journal row is a Trash-compatible effect
    /// before an admission can reach `effect_started`. This prevents a future
    /// platform driver from being called for a permanent-removal or eviction
    /// row and relying on terminal journaling to reject it afterward.
    pub(crate) fn validate_trash_effect(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
    ) -> Result<(), HistoryError> {
        let journal = self.snapshot()?;
        let item = journal
            .items
            .get(item_ordinal)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        item.paths
            .get(path_ordinal)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        if !super::success_matches(
            journal.mode,
            item.frozen.proposed_action,
            PathStatus::Trashed,
        ) {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        Ok(())
    }

    /// Confirm that the claimed journal row is the exact deterministic
    /// permanent-safe effect admitted by the planner before it can reach the
    /// private contents executor.
    pub(crate) fn validate_permanent_safe_effect(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
    ) -> Result<(), HistoryError> {
        let journal = self.snapshot()?;
        let item = journal
            .items
            .get(item_ordinal)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        item.paths
            .get(path_ordinal)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        if journal.mode != crate::domain::CleanupMode::PermanentSafe
            || item.frozen.safety != crate::domain::SafetyTier::SafeRegenerable
            || item.frozen.proposed_action
                != crate::domain::CandidateAction::RemoveKnownRegenerableContents
        {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        Ok(())
    }

    /// Re-check the journal-owned path transition after durable validation has
    /// moved one path from `planned` to `validating`. A sequential session may
    /// have a terminal prefix, but every later path must remain pristine and
    /// no sibling may be active. This prevents target switching between the
    /// validation write and live witness capture.
    pub(crate) fn validate_validating_path(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
    ) -> Result<(), HistoryError> {
        self.require_phase(ActivePhase::Running)?;
        let journal = self.snapshot()?;
        ensure_active(&journal, &self.fence, self.phase)?;
        let target_offset = journal
            .items
            .iter()
            .take(item_ordinal)
            .try_fold(0_usize, |offset, item| offset.checked_add(item.paths.len()))
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?
            .checked_add(path_ordinal)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        let mut ordinal = 0_usize;
        for (current_item_ordinal, item) in journal.items.iter().enumerate() {
            for (current_path_ordinal, path) in item.paths.iter().enumerate() {
                if (current_item_ordinal == item_ordinal
                    && current_path_ordinal == path_ordinal
                    && path.status != PathStatus::Validating)
                    || (ordinal == target_offset
                        && !(current_item_ordinal == item_ordinal
                            && current_path_ordinal == path_ordinal
                            && path.status == PathStatus::Validating))
                    || (ordinal < target_offset && !path.status.is_terminal())
                    || (ordinal > target_offset && path.status != PathStatus::Planned)
                {
                    return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
                }
                ordinal = ordinal
                    .checked_add(1)
                    .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
            }
        }
        if ordinal <= target_offset {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        Ok(())
    }

    /// Determine whether a path has already settled to a terminal status
    /// under this owner fence. This lets orchestration distinguish a durable
    /// changed/failed path from an ambiguous journal write.
    pub(crate) fn path_is_terminal(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
    ) -> Result<bool, HistoryError> {
        self.require_phase(ActivePhase::Running)?;
        let journal = self.snapshot()?;
        ensure_active(&journal, &self.fence, self.phase)?;
        Ok(journal_path(&journal, item_ordinal, path_ordinal)?
            .status
            .is_terminal())
    }

    pub(super) fn heartbeat(&self, heartbeat_at: SystemTime) -> Result<(), HistoryError> {
        let heartbeat_at = canonical_input_time(heartbeat_at)?;
        match self
            .write_active(|transaction| record_heartbeat(transaction, &self.fence, heartbeat_at))
        {
            Ok(()) => Ok(()),
            Err(_)
                if self.snapshot().ok().is_some_and(|journal| {
                    active_matches(&journal, &self.fence, self.phase, Some(heartbeat_at), None)
                }) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn request_cancellation(&self) -> Result<(), HistoryError> {
        match self.write_active(|transaction| request_cancellation(transaction, &self.fence)) {
            Ok(()) => Ok(()),
            Err(_)
                if self.snapshot().ok().is_some_and(|journal| {
                    active_matches(&journal, &self.fence, self.phase, None, Some(true))
                }) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn begin_validation(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
    ) -> Result<(), HistoryError> {
        self.require_phase(ActivePhase::Running)?;
        let write = self.write_active(|transaction| {
            begin_path_validation(transaction, &self.fence, item_ordinal, path_ordinal)
        });
        reconcile_unit_path_write(
            self,
            write,
            item_ordinal,
            path_ordinal,
            PathExpectation {
                status: PathStatus::Validating,
                effect_started_at: None,
                completed_at: None,
                error_category: None,
                exact_error: true,
            },
        )
    }

    pub(crate) fn finish_validation(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
        outcome: ValidationOutcome,
        error_category: Option<&str>,
        completed_at: SystemTime,
    ) -> Result<(), HistoryError> {
        self.require_phase(ActivePhase::Running)?;
        let completed_at = canonical_input_time(completed_at)?;
        let write = self.write_active(|transaction| {
            finish_path_validation(
                transaction,
                &self.fence,
                item_ordinal,
                path_ordinal,
                outcome,
                error_category,
                completed_at,
            )
        });
        reconcile_unit_path_write(
            self,
            write,
            item_ordinal,
            path_ordinal,
            PathExpectation {
                status: PathStatus::from(outcome),
                effect_started_at: None,
                completed_at: Some(completed_at),
                error_category,
                exact_error: true,
            },
        )
    }

    pub(crate) fn mark_effect_started(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
        started_at: SystemTime,
    ) -> Result<EffectStartReceipt, HistoryError> {
        self.require_phase(ActivePhase::Running)?;
        let journal = self.snapshot()?;
        if journal.mode == crate::domain::CleanupMode::PermanentSafe {
            self.ensure_permanent_cleanup_enabled()?;
        }
        self.ensure_path_not_excluded(&journal, item_ordinal, path_ordinal)?;
        let pending = self.pending_effect_start.get();
        if pending.is_some_and(|pending| {
            pending.item_ordinal != item_ordinal || pending.path_ordinal != path_ordinal
        }) {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let ordered_at = pending.map_or(started_at, |pending| pending.ordered_at);
        let persisted_started_at = canonical_input_time(ordered_at)?;
        let write = self.write_active_classified(|transaction| {
            mark_effect_started(
                transaction,
                &self.fence,
                item_ordinal,
                path_ordinal,
                ordered_at,
            )
        });
        let receipt = EffectStartReceipt {
            fence: self.fence.clone(),
            item_ordinal,
            path_ordinal,
            ordered_at,
            started_at: persisted_started_at,
        };
        match write {
            Ok(()) => {
                self.pending_effect_start.set(None);
                Ok(receipt)
            }
            Err(failure) if pending.is_some() || failure.may_have_committed => {
                self.pending_effect_start.set(Some(PendingEffectStart {
                    item_ordinal,
                    path_ordinal,
                    ordered_at,
                }));
                match self.reconcile_effect_start(item_ordinal, path_ordinal, ordered_at) {
                    Ok(receipt) => {
                        self.pending_effect_start.set(None);
                        Ok(receipt)
                    }
                    Err(_) => Err(failure.error),
                }
            }
            Err(failure) => Err(failure.error),
        }
    }

    /// The global permanent-cleanup switch is checked while this claim still
    /// owns the store-wide cleanup exclusion. The settings writer takes the
    /// same exclusion, so disabling cannot race this final pre-effect gate.
    fn ensure_permanent_cleanup_enabled(&self) -> Result<(), HistoryError> {
        if self.permanent_cleanup_effects_enabled()? {
            Ok(())
        } else {
            Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
        }
    }

    /// Observe the deny-by-default global gate while this claim owns the same
    /// cleanup exclusion used by setting writes. The executor can therefore
    /// durably reject a disabled path before asking for an effect receipt,
    /// without a policy write racing between observation and rejection.
    pub(crate) fn permanent_cleanup_effects_enabled(&self) -> Result<bool, HistoryError> {
        self.lease
            .store
            .validate_cleanup_lock_for_journal(&self.lease.guard)?;
        let connection = self.lease.store.lock_current_history_connection()?;
        self.lease
            .store
            .validate_cleanup_lock_for_journal(&self.lease.guard)?;
        Ok(crate::persistence::load_permanent_cleanup_setting(&connection.connection)?.enabled)
    }

    /// Re-read deny-only user exclusions while the claim owns the cleanup
    /// exclusion. An excluded target is rejected before any effect receipt is
    /// written; exclusions never grant access or weaken protected-path checks.
    fn ensure_path_not_excluded(
        &self,
        journal: &CleanupJournal,
        item_ordinal: usize,
        path_ordinal: usize,
    ) -> Result<(), HistoryError> {
        let target = journal
            .items
            .get(item_ordinal)
            .and_then(|item| item.paths.get(path_ordinal))
            .map(|path| path.target.as_path())
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        let connection = self.lease.store.lock_current_history_connection()?;
        self.lease
            .store
            .validate_cleanup_lock_for_journal(&self.lease.guard)?;
        if crate::persistence::load_cleanup_exclusions(&connection.connection)?
            .contains_path(target)
        {
            Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
        } else {
            Ok(())
        }
    }

    /// Reconcile an ambiguous pre-effect commit. A receipt is returned only if
    /// the exact current owner/generation/path row proves the intended durable
    /// `EffectStarted` state. No filesystem effect is attempted here.
    fn reconcile_effect_start(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
        started_at: SystemTime,
    ) -> Result<EffectStartReceipt, HistoryError> {
        self.require_phase(ActivePhase::Running)?;
        let ordered_at = started_at;
        let started_at = canonical_input_time(started_at)?;
        let journal = self.snapshot()?;
        ensure_active(&journal, &self.fence, self.phase)?;
        let path = journal_path(&journal, item_ordinal, path_ordinal)?;
        if path.status != PathStatus::EffectStarted
            || path.attempt_generation != Some(self.fence.generation)
            || path.effect_started_at != Some(started_at)
            || path.completed_at.is_some()
        {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        self.lease
            .store
            .validate_cleanup_lock_for_journal(&self.lease.guard)?;
        Ok(EffectStartReceipt {
            fence: self.fence.clone(),
            item_ordinal,
            path_ordinal,
            ordered_at,
            started_at,
        })
    }

    /// Revalidate the exact durable receipt and cleanup control immediately
    /// before a future centralized executor performs the operating-system call.
    pub(crate) fn revalidate_effect_receipt(
        &self,
        receipt: &EffectStartReceipt,
    ) -> Result<(), HistoryError> {
        self.validate_receipt(receipt)?;
        let journal = self.snapshot()?;
        if !active_matches(
            &journal,
            &self.fence,
            ActivePhase::Running,
            None,
            Some(false),
        ) {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let path = journal_path(&journal, receipt.item_ordinal, receipt.path_ordinal)?;
        if path.status != PathStatus::EffectStarted
            || path.attempt_generation != Some(self.fence.generation)
            || path.effect_started_at != Some(receipt.started_at)
        {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        // This is deliberately the last operation. The future executor must
        // call its reviewed effect primitive immediately after it returns.
        self.lease
            .store
            .validate_cleanup_lock_for_journal(&self.lease.guard)
    }

    /// Settle a durable effect intent after cancellation wins final
    /// revalidation and before the caller invokes the OS primitive.
    pub(crate) fn cancel_effect_before_call(
        &self,
        receipt: &EffectStartReceipt,
        completed_at: SystemTime,
    ) -> Result<(), HistoryError> {
        self.validate_receipt(receipt)?;
        self.require_phase(ActivePhase::Running)?;
        if completed_at < receipt.ordered_at {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let completed_at = canonical_input_time(completed_at)?;
        let write = self.write_active(|transaction| {
            cancel_effect_before_call(
                transaction,
                &self.fence,
                receipt.item_ordinal,
                receipt.path_ordinal,
                completed_at,
            )
        });
        reconcile_unit_path_write(
            self,
            write,
            receipt.item_ordinal,
            receipt.path_ordinal,
            PathExpectation {
                status: PathStatus::Interrupted,
                effect_started_at: None,
                completed_at: Some(completed_at),
                error_category: None,
                exact_error: true,
            },
        )
    }

    pub(crate) fn finish_effect(
        &mut self,
        receipt: &EffectStartReceipt,
        outcome: EffectOutcome,
        error_category: Option<&str>,
        completed_at: SystemTime,
    ) -> Result<(), HistoryError> {
        self.validate_receipt(receipt)?;
        self.require_phase(ActivePhase::Running)?;
        if completed_at < receipt.ordered_at {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let completed_at = canonical_input_time(completed_at)?;
        let write = self.write_active(|transaction| {
            finish_effect(
                transaction,
                &self.fence,
                receipt.item_ordinal,
                receipt.path_ordinal,
                outcome,
                error_category,
                completed_at,
            )
        });
        if let Err(error) = write {
            let reconciled = self.snapshot().ok().map(|journal| {
                let expected_phase = if outcome == EffectOutcome::OutcomeUnknown {
                    ActivePhase::Recovering
                } else {
                    ActivePhase::Running
                };
                ensure_active(&journal, &self.fence, expected_phase)
                    .ok()
                    .and_then(|()| {
                        journal_path(&journal, receipt.item_ordinal, receipt.path_ordinal).ok()
                    })
                    .is_some_and(|path| {
                        path.status == effect_outcome_status(outcome)
                            && path.attempt_generation == Some(self.fence.generation)
                            && path.effect_started_at == Some(receipt.started_at)
                            && path.completed_at == Some(completed_at)
                            && path.error_category.as_deref() == error_category
                    })
            });
            if reconciled != Some(true) {
                return Err(error);
            }
        }
        if outcome == EffectOutcome::OutcomeUnknown {
            self.phase = ActivePhase::Recovering;
        }
        Ok(())
    }

    pub(super) fn reconcile_unknown(
        &self,
        item_ordinal: usize,
        path_ordinal: usize,
        outcome: ReconciledOutcome,
        error_category: Option<&str>,
        completed_at: SystemTime,
    ) -> Result<(), HistoryError> {
        self.require_phase(ActivePhase::Recovering)?;
        let completed_at = canonical_input_time(completed_at)?;
        let effect_started_at = journal_path(&self.snapshot()?, item_ordinal, path_ordinal)?
            .effect_started_at
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
        let write = self.write_active(|transaction| {
            reconcile_unknown_outcome(
                transaction,
                &self.fence,
                item_ordinal,
                path_ordinal,
                outcome,
                error_category,
                completed_at,
            )
        });
        reconcile_unit_path_write(
            self,
            write,
            item_ordinal,
            path_ordinal,
            PathExpectation {
                status: PathStatus::from(outcome),
                effect_started_at: Some(effect_started_at),
                completed_at: Some(completed_at),
                error_category,
                exact_error: true,
            },
        )
    }

    pub(crate) fn settle_cancellation(&self, completed_at: SystemTime) -> Result<(), HistoryError> {
        let completed_at = canonical_input_time(completed_at)?;
        match self
            .write_active(|transaction| settle_cancellation(transaction, &self.fence, completed_at))
        {
            Ok(()) => Ok(()),
            Err(_)
                if self.snapshot().ok().is_some_and(|journal| {
                    active_matches(&journal, &self.fence, self.phase, None, Some(true))
                        && !journal
                            .items
                            .iter()
                            .flat_map(|item| &item.paths)
                            .any(|path| {
                                matches!(path.status, PathStatus::Planned | PathStatus::Validating)
                            })
                }) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn resume_recovery(&mut self) -> Result<(), HistoryError> {
        self.require_phase(ActivePhase::Recovering)?;
        if self.snapshot().ok().is_some_and(|journal| {
            active_matches(
                &journal,
                &self.fence,
                ActivePhase::Running,
                None,
                Some(false),
            )
        }) {
            self.phase = ActivePhase::Running;
            return Ok(());
        }
        match self.write_active(|transaction| resume_recovery(transaction, &self.fence)) {
            Ok(()) => {
                self.phase = ActivePhase::Running;
                Ok(())
            }
            Err(_)
                if self.snapshot().ok().is_some_and(|journal| {
                    active_matches(
                        &journal,
                        &self.fence,
                        ActivePhase::Running,
                        None,
                        Some(false),
                    )
                }) =>
            {
                self.phase = ActivePhase::Running;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn terminalize(
        &mut self,
        completed_at: SystemTime,
        verified_capacity_delta_bytes: Option<i64>,
    ) -> Result<TerminalSessionStatus, HistoryError> {
        let completed_at = canonical_input_time(completed_at)?;
        if let Some(status) = reconciled_terminal(
            self.snapshot().ok().as_ref(),
            &self.fence,
            completed_at,
            verified_capacity_delta_bytes,
        ) {
            return Ok(status);
        }
        match self.write_active(|transaction| {
            terminalize(
                transaction,
                &self.fence,
                completed_at,
                verified_capacity_delta_bytes,
            )
        }) {
            Ok(status) => Ok(status),
            Err(error) => reconciled_terminal(
                self.snapshot().ok().as_ref(),
                &self.fence,
                completed_at,
                verified_capacity_delta_bytes,
            )
            .ok_or(error),
        }
    }

    /// Terminalize through the cleanup capacity-verification adapter. The
    /// adapter accepts only a delta produced by the private pre/post witness;
    /// raw callers remain on the existing journal test/recovery path.
    pub(crate) fn terminalize_for_capacity_verification(
        &mut self,
        completed_at: SystemTime,
        verified_capacity_delta_bytes: Option<i64>,
    ) -> Result<(), HistoryError> {
        self.terminalize(completed_at, verified_capacity_delta_bytes)
            .map(|_| ())
    }

    /// Terminalize through the same private capacity-verification gate while
    /// retaining the bounded journal outcome for the engine orchestrator.
    pub(crate) fn terminalize_for_capacity_verification_with_status(
        &mut self,
        completed_at: SystemTime,
        verified_capacity_delta_bytes: Option<i64>,
    ) -> Result<TerminalSessionStatus, HistoryError> {
        self.terminalize(completed_at, verified_capacity_delta_bytes)
    }

    fn write_active<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T, HistoryError>,
    ) -> Result<T, HistoryError> {
        self.write_active_classified(operation)
            .map_err(|failure| failure.error)
    }

    fn write_active_classified<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T, HistoryError>,
    ) -> Result<T, JournalWriteFailure> {
        let result = self.lease.write_classified(|transaction| {
            let journal = load_cleanup_journal(transaction, &self.fence.session_id)?
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
            ensure_active(&journal, &self.fence, self.phase)?;
            let output = operation(transaction)?;
            load_cleanup_journal(transaction, &self.fence.session_id)?
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
            Ok(output)
        });
        #[cfg(test)]
        if result.is_ok()
            && self.test_fault.get() == TestJournalFault::FailAfterCommitThenReconcileRead
        {
            self.test_fault.set(TestJournalFault::FailReconcileRead);
            return Err(ambiguous_write_failure(HistoryError::new(
                HistoryErrorKind::DatabaseUnavailable,
            )));
        }
        result
    }

    fn require_phase(&self, expected: ActivePhase) -> Result<(), HistoryError> {
        if self.phase == expected {
            Ok(())
        } else {
            Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
        }
    }

    fn validate_receipt(&self, receipt: &EffectStartReceipt) -> Result<(), HistoryError> {
        if receipt.fence == self.fence {
            Ok(())
        } else {
            Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
        }
    }
}

struct PathExpectation<'error> {
    status: PathStatus,
    effect_started_at: Option<SystemTime>,
    completed_at: Option<SystemTime>,
    error_category: Option<&'error str>,
    exact_error: bool,
}

fn reconcile_unit_path_write(
    claim: &CleanupJournalClaim,
    write: Result<(), HistoryError>,
    item_ordinal: usize,
    path_ordinal: usize,
    expected: PathExpectation<'_>,
) -> Result<(), HistoryError> {
    match write {
        Ok(()) => Ok(()),
        Err(error) => {
            let reconciled = claim.snapshot().ok().is_some_and(|journal| {
                active_matches(&journal, &claim.fence, claim.phase, None, None)
                    && journal_path(&journal, item_ordinal, path_ordinal)
                        .ok()
                        .is_some_and(|path| {
                            path.status == expected.status
                                && path.attempt_generation == Some(claim.fence.generation)
                                && path.effect_started_at == expected.effect_started_at
                                && path.completed_at == expected.completed_at
                                && (!expected.exact_error
                                    || path.error_category.as_deref() == expected.error_category)
                        })
            });
            if reconciled { Ok(()) } else { Err(error) }
        }
    }
}

fn canonical_input_time(value: SystemTime) -> Result<SystemTime, HistoryError> {
    unix_ms_to_system_time(system_time_to_unix_ms(
        value,
        HistoryErrorKind::InvalidInput,
    )?)
}

fn definite_write_failure(error: HistoryError) -> JournalWriteFailure {
    JournalWriteFailure {
        error,
        may_have_committed: false,
    }
}

fn ambiguous_write_failure(error: HistoryError) -> JournalWriteFailure {
    JournalWriteFailure {
        error,
        may_have_committed: true,
    }
}

fn active_matches(
    journal: &CleanupJournal,
    fence: &ExecutionFence,
    phase: ActivePhase,
    heartbeat_at: Option<SystemTime>,
    cancellation_requested: Option<bool>,
) -> bool {
    matches!(
        journal.lifecycle,
        JournalLifecycle::Active {
            phase: loaded_phase,
            fence: ref loaded_fence,
            heartbeat_at: loaded_heartbeat,
            cancellation_requested: loaded_cancellation,
        } if loaded_phase == phase
            && loaded_fence == fence
            && heartbeat_at.is_none_or(|expected| loaded_heartbeat == expected)
            && cancellation_requested.is_none_or(|expected| loaded_cancellation == expected)
    )
}

fn expired_terminal_matches(
    journal: &CleanupJournal,
    fence: &ExecutionFence,
    completed_at: SystemTime,
) -> bool {
    matches!(
        journal.lifecycle,
        JournalLifecycle::Terminal {
            status: TerminalSessionStatus::Rejected,
            fence: ref loaded_fence,
            heartbeat_at,
            completed_at: loaded_completed_at,
            verified_capacity_delta_bytes: None,
            cancellation_requested: false,
        } if loaded_fence == fence
            && heartbeat_at == completed_at
            && loaded_completed_at == completed_at
    ) && journal.items.iter().all(|item| {
        item.status == PathStatus::Rejected
            && item.error_category.as_deref() == Some(super::PLAN_EXPIRED_ERROR)
            && item.paths.iter().all(|path| {
                path.status == PathStatus::Rejected
                    && path.attempt_generation == Some(fence.generation)
                    && path.error_category.as_deref() == Some(super::PLAN_EXPIRED_ERROR)
                    && path.effect_started_at.is_none()
                    && path.completed_at == Some(completed_at)
            })
    })
}

fn reconciled_terminal(
    journal: Option<&CleanupJournal>,
    fence: &ExecutionFence,
    completed_at: SystemTime,
    verified_capacity_delta_bytes: Option<i64>,
) -> Option<TerminalSessionStatus> {
    let JournalLifecycle::Terminal {
        status,
        fence: loaded_fence,
        completed_at: loaded_completed_at,
        verified_capacity_delta_bytes: loaded_delta,
        ..
    } = &journal?.lifecycle
    else {
        return None;
    };
    (*loaded_fence == *fence
        && *loaded_completed_at == completed_at
        && *loaded_delta == verified_capacity_delta_bytes)
        .then_some(*status)
}

fn ensure_active(
    journal: &CleanupJournal,
    fence: &ExecutionFence,
    phase: ActivePhase,
) -> Result<(), HistoryError> {
    if matches!(
        journal.lifecycle,
        JournalLifecycle::Active {
            phase: loaded_phase,
            fence: ref loaded_fence,
            ..
        } if loaded_phase == phase && loaded_fence == fence
    ) {
        Ok(())
    } else {
        Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
    }
}

fn journal_path(
    journal: &CleanupJournal,
    item_ordinal: usize,
    path_ordinal: usize,
) -> Result<&super::JournalPath, HistoryError> {
    journal
        .items
        .get(item_ordinal)
        .and_then(|item| item.paths.get(path_ordinal))
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))
}

fn map_process_identity_error(error: ProcessIdentityError) -> HistoryError {
    let kind = match error {
        ProcessIdentityError::InvalidEncoding => HistoryErrorKind::InternalState,
        ProcessIdentityError::ObservationUnavailable | ProcessIdentityError::RandomUnavailable => {
            HistoryErrorKind::DatabaseUnavailable
        }
    };
    HistoryError::new(kind)
}

fn effect_outcome_status(outcome: EffectOutcome) -> PathStatus {
    match outcome {
        EffectOutcome::Trashed => PathStatus::Trashed,
        EffectOutcome::Removed => PathStatus::Removed,
        EffectOutcome::Evicted => PathStatus::Evicted,
        EffectOutcome::Failed => PathStatus::Failed,
        EffectOutcome::OutcomeUnknown => PathStatus::OutcomeUnknown,
    }
}
