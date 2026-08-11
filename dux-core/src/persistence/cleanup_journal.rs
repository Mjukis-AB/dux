//! Owner/generation-fenced cleanup-journal persistence for schema v2.
//!
//! This module records and validates journal state only. It does not authorize
//! or perform filesystem effects. Callers must hold and revalidate the cleanup
//! OS lock before entering any function here; the exact owner/generation
//! predicates below are an additional fence, not a replacement for that lock.

mod lease;
mod loader;

use loader::{DynamicPathState, load_cleanup_journal, load_path_statuses, mode_accepts_action};
pub(super) use loader::{
    load_cleanup_journal_within_budget, validate_cleanup_journal_scalar_state_within_budget,
};

pub(crate) use lease::{
    CleanupAdmissionLease, CleanupJournalClaim, CleanupJournalLease, DryRunJournalFailure,
    EffectStartReceipt, JournalLeaseFailure, ValidatedDryRunOutcome,
};
#[cfg(test)]
pub(crate) use lease::{
    fail_next_write_after_commit_and_reconcile_read_for_test,
    fail_next_write_after_commit_and_two_reconcile_reads_for_test,
};

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension, Transaction, params};

use crate::domain::{
    CLEANUP_PLAN_VALIDITY, CandidateAction, CandidateId, CleanupMode, CleanupPlan, CleanupPlanId,
    Evidence, PlanWarning, RuleId, RuleRef, RuleRevision, SafetyTier, ScanId,
};

use super::candidate_history::{
    CandidatePlanSettlement, RawEvidence, action_from_stored, category_from_stored,
    decode_absolute_path, decode_evidence, decode_optional_time, safety_from_stored,
    settle_planned_candidate, validate_complete_children, validate_policy,
};
use super::cleanup_history::{
    CandidateStatusCoupling, CleanupSessionId, CleanupTrigger, NewCleanupSessionRecord,
    PlannedCleanupItemRecord, insert_uncoupled_cleanup_session,
    load_frozen_cleanup_session_within_budget, mode_from_stored, trigger_from_stored,
    warning_from_stored,
};
use super::history::{
    HistoryError, HistoryErrorKind, from_i64, map_query_sql_error, map_write_sql_error,
    run_bounded_query, stored_bool, system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::process_liveness::{
    ExecutionProvenance, ProcessInstanceId, ProcessLiveness, ProvenanceRelationship,
    compare_execution_provenance, probe_process_instance,
};

const MAX_ITEMS: usize = 64;
const MAX_TOTAL_PATHS: usize = 256;
const MAX_TOTAL_EVIDENCE: usize = 512;
const MAX_WARNINGS: usize = 5;
const MAX_ERROR_BYTES: usize = 128;
const PLAN_EXPIRED_ERROR: &str = "plan_expired";

/// The active parent-row fence copied into every journal mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ExecutionFence {
    session_id: CleanupSessionId,
    owner: ProcessInstanceId,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ActivePhase {
    Running,
    Recovering,
}

impl ActivePhase {
    fn stored(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Recovering => "recovering",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalSessionStatus {
    Completed,
    PartiallyCompleted,
    Failed,
    Cancelled,
    Interrupted,
    Rejected,
    DryRun,
}

impl TerminalSessionStatus {
    fn stored(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::PartiallyCompleted => "partially_completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::Rejected => "rejected",
            Self::DryRun => "dry_run",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum JournalLifecycle {
    Planned,
    Active {
        phase: ActivePhase,
        fence: ExecutionFence,
        heartbeat_at: SystemTime,
        cancellation_requested: bool,
    },
    Terminal {
        status: TerminalSessionStatus,
        fence: ExecutionFence,
        heartbeat_at: SystemTime,
        completed_at: SystemTime,
        verified_capacity_delta_bytes: Option<i64>,
        cancellation_requested: bool,
    },
    /// A terminal validation observation that never claimed an execution
    /// owner. This shape is valid only for an uncoupled DryRun plan and can
    /// contain only the schema-required first validation attempt, never an
    /// execution generation, effect receipt, or capacity delta.
    ObservedTerminal {
        status: TerminalSessionStatus,
        completed_at: SystemTime,
        cancellation_requested: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PathStatus {
    Planned,
    Validating,
    DryRun,
    EffectStarted,
    Trashed,
    Removed,
    Evicted,
    Skipped,
    Rejected,
    Failed,
    ChangedSincePlan,
    Interrupted,
    Unavailable,
    OutcomeUnknown,
}

impl PathStatus {
    fn stored(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Validating => "validating",
            Self::DryRun => "dry_run",
            Self::EffectStarted => "effect_started",
            Self::Trashed => "trashed",
            Self::Removed => "removed",
            Self::Evicted => "evicted",
            Self::Skipped => "skipped",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
            Self::ChangedSincePlan => "changed_since_plan",
            Self::Interrupted => "interrupted",
            Self::Unavailable => "unavailable",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }

    fn from_stored(value: &str) -> Result<Self, HistoryError> {
        match value {
            "planned" => Ok(Self::Planned),
            "validating" => Ok(Self::Validating),
            "dry_run" => Ok(Self::DryRun),
            "effect_started" => Ok(Self::EffectStarted),
            "trashed" => Ok(Self::Trashed),
            "removed" => Ok(Self::Removed),
            "evicted" => Ok(Self::Evicted),
            "skipped" => Ok(Self::Skipped),
            "rejected" => Ok(Self::Rejected),
            "failed" => Ok(Self::Failed),
            "changed_since_plan" => Ok(Self::ChangedSincePlan),
            "interrupted" => Ok(Self::Interrupted),
            "unavailable" => Ok(Self::Unavailable),
            "outcome_unknown" => Ok(Self::OutcomeUnknown),
            _ => Err(corrupt()),
        }
    }

    fn is_terminal(self) -> bool {
        !matches!(self, Self::Planned | Self::Validating | Self::EffectStarted)
    }

    fn is_success(self) -> bool {
        matches!(
            self,
            Self::DryRun | Self::Trashed | Self::Removed | Self::Evicted
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedAttempt {
    None,
    Current,
    AtMostCurrent,
}

#[derive(Clone, Copy)]
enum CancellationRequirement {
    Any,
    NotRequested,
    Requested,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct JournalPath {
    pub(super) ordinal: usize,
    pub(super) target: PathBuf,
    pub(super) attempt_generation: Option<u64>,
    pub(super) status: PathStatus,
    pub(super) error_category: Option<String>,
    pub(super) effect_started_at: Option<SystemTime>,
    pub(super) completed_at: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct JournalItem {
    pub(super) frozen: PlannedCleanupItemRecord,
    pub(super) status: PathStatus,
    pub(super) error_category: Option<String>,
    pub(super) paths: Vec<JournalPath>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CleanupJournal {
    pub(super) session_id: CleanupSessionId,
    pub(super) plan_id: CleanupPlanId,
    pub(super) started_at: SystemTime,
    pub(super) source_scan_id: ScanId,
    pub(super) plan_created_at: SystemTime,
    pub(super) plan_expires_at: SystemTime,
    pub(super) mode: CleanupMode,
    pub(super) estimated_bytes: u64,
    pub(super) trigger: CleanupTrigger,
    pub(super) candidate_status_coupling: CandidateStatusCoupling,
    pub(super) warnings: Vec<PlanWarning>,
    pub(super) execution_provenance: Option<ExecutionProvenance>,
    pub(super) lifecycle: JournalLifecycle,
    pub(super) items: Vec<JournalItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValidationOutcome {
    DryRun,
    Skipped,
    Rejected,
    Failed,
    ChangedSincePlan,
    Interrupted,
    Unavailable,
}

impl From<ValidationOutcome> for PathStatus {
    fn from(value: ValidationOutcome) -> Self {
        match value {
            ValidationOutcome::DryRun => Self::DryRun,
            ValidationOutcome::Skipped => Self::Skipped,
            ValidationOutcome::Rejected => Self::Rejected,
            ValidationOutcome::Failed => Self::Failed,
            ValidationOutcome::ChangedSincePlan => Self::ChangedSincePlan,
            ValidationOutcome::Interrupted => Self::Interrupted,
            ValidationOutcome::Unavailable => Self::Unavailable,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectOutcome {
    Trashed,
    Removed,
    Evicted,
    Failed,
    OutcomeUnknown,
}

impl From<EffectOutcome> for PathStatus {
    fn from(value: EffectOutcome) -> Self {
        match value {
            EffectOutcome::Trashed => Self::Trashed,
            EffectOutcome::Removed => Self::Removed,
            EffectOutcome::Evicted => Self::Evicted,
            EffectOutcome::Failed => Self::Failed,
            EffectOutcome::OutcomeUnknown => Self::OutcomeUnknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReconciledOutcome {
    Trashed,
    Removed,
    Evicted,
    Failed,
}

impl From<ReconciledOutcome> for PathStatus {
    fn from(value: ReconciledOutcome) -> Self {
        match value {
            ReconciledOutcome::Trashed => Self::Trashed,
            ReconciledOutcome::Removed => Self::Removed,
            ReconciledOutcome::Evicted => Self::Evicted,
            ReconciledOutcome::Failed => Self::Failed,
        }
    }
}

/// Recovery eligibility can only be created by this module's process probe.
pub(super) enum RecoveryAssessment {
    OwnerAlive,
    LivenessUnknown,
    PriorBoot,
    ForeignHost,
    Unproven,
    Recoverable(RecoveryPermit),
}

pub(super) struct RecoveryPermit {
    stale_fence: ExecutionFence,
    phase: ActivePhase,
    heartbeat_at_unix_ms: i64,
    cancellation_requested: bool,
}

fn invalid_transition() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidTransition)
}

fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

fn to_generation(value: u64) -> Result<i64, HistoryError> {
    i64::try_from(value).map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))
}

fn decode_generation(value: i64) -> Result<u64, HistoryError> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(corrupt)
}

fn validate_error(value: Option<&str>) -> Result<(), HistoryError> {
    if value.is_some_and(|value| {
        value.is_empty()
            || value.len() > MAX_ERROR_BYTES
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-' | b':')
            })
    }) {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    Ok(())
}

fn require_one(changed: usize) -> Result<(), HistoryError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(invalid_transition())
    }
}

/// Claim a pristine, unexpired schema-v2 plan as generation one.
fn claim_planned(
    transaction: &Transaction<'_>,
    session_id: &CleanupSessionId,
    owner: ProcessInstanceId,
    provenance: Option<&ExecutionProvenance>,
    claimed_at: SystemTime,
) -> Result<ExecutionFence, HistoryError> {
    let graph = load_cleanup_journal(transaction, session_id)?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    if (!graph.candidate_status_coupling.claims_candidates()
        && !is_explicit_explorer_selection(&graph))
        || graph.lifecycle != JournalLifecycle::Planned
        || claimed_at < graph.started_at
        || graph.plan_expires_at <= claimed_at
    {
        return Err(invalid_transition());
    }
    let heartbeat = system_time_to_unix_ms(claimed_at, HistoryErrorKind::InvalidInput)?;
    let host_identity = provenance.map(ExecutionProvenance::stable_host);
    let boot_scope = provenance.map(ExecutionProvenance::boot_scope);
    let recovery_policy = provenance.map(|_| "resumable");
    let changed = transaction
        .execute(
            "UPDATE cleanup_sessions
             SET status = 'running', execution_owner_id = ?2,
                 execution_generation = 1, last_heartbeat_at_unix_ms = ?3,
                 execution_host_identity_v1_sha256 = ?4,
                 execution_boot_scope_v1_sha256 = ?5,
                 execution_recovery_policy = ?6
             WHERE session_id = ?1 AND record_format_version = 2
               AND status = 'planned' AND completed_at_unix_ms IS NULL
               AND verified_capacity_delta_bytes IS NULL
               AND execution_owner_id IS NULL AND execution_generation IS NULL
               AND last_heartbeat_at_unix_ms IS NULL AND cancellation_requested = 0
               AND execution_host_identity_v1_sha256 IS NULL
               AND execution_boot_scope_v1_sha256 IS NULL
               AND execution_recovery_policy IS NULL",
            params![
                session_id.as_str(),
                owner.as_str(),
                heartbeat,
                host_identity,
                boot_scope,
                recovery_policy,
            ],
        )
        .map_err(map_write_sql_error)?;
    require_one(changed)?;
    Ok(ExecutionFence {
        session_id: session_id.clone(),
        owner,
        generation: 1,
    })
}

fn is_explicit_explorer_selection(journal: &CleanupJournal) -> bool {
    journal.candidate_status_coupling == CandidateStatusCoupling::LegacyUncoupled
        && journal.mode == CleanupMode::Trash
        && journal.items.len() == 1
        && journal.items[0].frozen.rule.id().as_str() == "explorer.selection.trash"
        && journal.items[0].frozen.rule.revision().get() == 1
}

/// Settle an exact pristine plan at or after expiry without granting effect
/// authority. The cleanup lock still supplies exclusion, and generation one is
/// retained only as terminal history provenance.
fn expire_planned(
    transaction: &Transaction<'_>,
    session_id: &CleanupSessionId,
    owner: ProcessInstanceId,
    observed_at: SystemTime,
) -> Result<ExecutionFence, HistoryError> {
    let journal = load_cleanup_journal(transaction, session_id)?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    if (!journal.candidate_status_coupling.claims_candidates()
        && !is_explicit_explorer_selection(&journal))
        || journal.lifecycle != JournalLifecycle::Planned
        || observed_at < journal.plan_expires_at
    {
        return Err(invalid_transition());
    }
    let settled_at = canonical_expiry_settlement_time(observed_at)?;
    let completed = system_time_to_unix_ms(settled_at, HistoryErrorKind::InvalidInput)?;
    let path_count = journal
        .items
        .iter()
        .map(|item| item.paths.len())
        .sum::<usize>();
    let changed_paths = transaction
        .execute(
            "UPDATE cleanup_item_paths
             SET attempt_generation = 1, status = 'rejected',
                 error_category = ?2, completed_at_unix_ms = ?3
             WHERE session_id = ?1 AND status = 'planned'
               AND attempt_generation IS NULL AND error_category IS NULL
               AND effect_started_at_unix_ms IS NULL AND completed_at_unix_ms IS NULL",
            params![session_id.as_str(), PLAN_EXPIRED_ERROR, completed],
        )
        .map_err(map_write_sql_error)?;
    if changed_paths != path_count {
        return Err(invalid_transition());
    }
    let changed_items = transaction
        .execute(
            "UPDATE cleanup_items
             SET final_status = 'rejected', error_category = ?2
             WHERE session_id = ?1 AND record_format_version = 2
               AND final_status = 'planned' AND error_category IS NULL",
            params![session_id.as_str(), PLAN_EXPIRED_ERROR],
        )
        .map_err(map_write_sql_error)?;
    if changed_items != journal.items.len() {
        return Err(invalid_transition());
    }
    if journal.candidate_status_coupling.claims_candidates() {
        for item in &journal.items {
            settle_candidate_plan_claim(
                transaction,
                &journal,
                item,
                CandidatePlanSettlement::Failed,
            )?;
        }
    }
    let changed = transaction
        .execute(
            "UPDATE cleanup_sessions
             SET status = 'rejected', completed_at_unix_ms = ?3,
                 execution_owner_id = ?2, execution_generation = 1,
                 last_heartbeat_at_unix_ms = ?3
             WHERE session_id = ?1 AND record_format_version = 2
               AND candidate_status_coupling_version = ?4
               AND status = 'planned' AND completed_at_unix_ms IS NULL
               AND verified_capacity_delta_bytes IS NULL
               AND execution_owner_id IS NULL AND execution_generation IS NULL
               AND last_heartbeat_at_unix_ms IS NULL AND cancellation_requested = 0",
            params![
                session_id.as_str(),
                owner.as_str(),
                completed,
                journal.candidate_status_coupling.as_i64(),
            ],
        )
        .map_err(map_write_sql_error)?;
    require_one(changed)?;
    Ok(ExecutionFence {
        session_id: session_id.clone(),
        owner,
        generation: 1,
    })
}

/// Atomically insert and settle one non-effect DryRun observation.
///
/// The row never enters `running` or `recovering`: there is no execution owner,
/// execution generation, heartbeat, effect receipt, or capacity measurement.
/// The surrounding cleanup lease is the sole serialization boundary.
#[allow(clippy::too_many_arguments)]
fn record_validated_dry_run(
    transaction: &Transaction<'_>,
    session_id: CleanupSessionId,
    plan: &CleanupPlan,
    trigger: CleanupTrigger,
    outcome: ValidationOutcome,
    error_category: Option<&str>,
    cancellation_requested: bool,
    started_at: SystemTime,
    completed_at: SystemTime,
) -> Result<TerminalSessionStatus, HistoryError> {
    if plan.mode() != CleanupMode::DryRun {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    validate_error(error_category)?;
    if matches!(outcome, ValidationOutcome::DryRun) != error_category.is_none()
        || (cancellation_requested && !matches!(outcome, ValidationOutcome::Interrupted))
    {
        return Err(invalid_transition());
    }
    let record = NewCleanupSessionRecord::try_from_uncoupled_plan(
        session_id.clone(),
        plan,
        started_at,
        trigger,
    )?;
    insert_uncoupled_cleanup_session(transaction, &record)?;

    let path_status = PathStatus::from(outcome);
    let completed = system_time_to_unix_ms(completed_at, HistoryErrorKind::InvalidInput)?;
    let expected_paths = plan
        .items()
        .iter()
        .try_fold(0_usize, |total, item| total.checked_add(item.paths().len()))
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    let changed_paths = transaction
        .execute(
            "UPDATE cleanup_item_paths
             SET status = ?2, attempt_generation = 1,
                 error_category = ?3, completed_at_unix_ms = ?4
             WHERE session_id = ?1 AND status = 'planned'
               AND attempt_generation IS NULL
               AND error_category IS NULL
               AND effect_started_at_unix_ms IS NULL
               AND completed_at_unix_ms IS NULL",
            params![
                session_id.as_str(),
                path_status.stored(),
                error_category,
                completed,
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed_paths != expected_paths {
        return Err(invalid_transition());
    }
    let changed_items = transaction
        .execute(
            "UPDATE cleanup_items
             SET final_status = ?2, error_category = ?3
             WHERE session_id = ?1 AND record_format_version = 2
               AND final_status = 'planned' AND error_category IS NULL",
            params![session_id.as_str(), path_status.stored(), error_category],
        )
        .map_err(map_write_sql_error)?;
    if changed_items != plan.items().len() {
        return Err(invalid_transition());
    }
    let statuses = vec![path_status; expected_paths];
    let terminal =
        derive_terminal_session_status(&statuses, CleanupMode::DryRun, cancellation_requested)?;
    let changed_session = transaction
        .execute(
            "UPDATE cleanup_sessions
             SET status = ?2, completed_at_unix_ms = ?3,
                 cancellation_requested = ?4
             WHERE session_id = ?1 AND record_format_version = 2
               AND mode = 'dry_run'
               AND candidate_status_coupling_version = 1
               AND status = 'planned' AND completed_at_unix_ms IS NULL
               AND verified_capacity_delta_bytes IS NULL
               AND execution_owner_id IS NULL
               AND execution_generation IS NULL
               AND last_heartbeat_at_unix_ms IS NULL
               AND cancellation_requested = 0",
            params![
                session_id.as_str(),
                terminal.stored(),
                completed,
                i64::from(cancellation_requested),
            ],
        )
        .map_err(map_write_sql_error)?;
    require_one(changed_session)?;
    Ok(terminal)
}

fn canonical_expiry_settlement_time(value: SystemTime) -> Result<SystemTime, HistoryError> {
    let elapsed = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    let floor = system_time_to_unix_ms(value, HistoryErrorKind::InvalidInput)?;
    let milliseconds = if elapsed.subsec_nanos() % 1_000_000 == 0 {
        floor
    } else {
        floor
            .checked_add(1)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?
    };
    unix_ms_to_system_time(milliseconds)
}

fn record_heartbeat(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    heartbeat_at: SystemTime,
) -> Result<(), HistoryError> {
    let heartbeat = system_time_to_unix_ms(heartbeat_at, HistoryErrorKind::InvalidInput)?;
    let generation = to_generation(fence.generation)?;
    require_one(
        transaction
            .execute(
                "UPDATE cleanup_sessions SET last_heartbeat_at_unix_ms = ?4
                 WHERE session_id = ?1 AND record_format_version = 2
                   AND status IN ('running', 'recovering')
                   AND execution_owner_id = ?2 AND execution_generation = ?3
                   AND last_heartbeat_at_unix_ms <= ?4",
                params![
                    fence.session_id.as_str(),
                    fence.owner.as_str(),
                    generation,
                    heartbeat
                ],
            )
            .map_err(map_write_sql_error)?,
    )
}

fn request_cancellation(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
) -> Result<(), HistoryError> {
    let generation = to_generation(fence.generation)?;
    require_one(
        transaction
            .execute(
                "UPDATE cleanup_sessions SET cancellation_requested = 1
                 WHERE session_id = ?1 AND record_format_version = 2
                   AND status IN ('running', 'recovering')
                   AND execution_owner_id = ?2 AND execution_generation = ?3",
                params![fence.session_id.as_str(), fence.owner.as_str(), generation],
            )
            .map_err(map_write_sql_error)?,
    )
}

fn begin_path_validation(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    item_ordinal: usize,
    path_ordinal: usize,
) -> Result<(), HistoryError> {
    transition_path(
        transaction,
        fence,
        item_ordinal,
        path_ordinal,
        PathStatus::Planned,
        PathStatus::Validating,
        ActivePhase::Running,
        ExpectedAttempt::None,
        None,
        None,
        false,
        None,
        CancellationRequirement::NotRequested,
    )
}

fn finish_path_validation(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    item_ordinal: usize,
    path_ordinal: usize,
    outcome: ValidationOutcome,
    error_category: Option<&str>,
    completed_at: SystemTime,
) -> Result<(), HistoryError> {
    validate_error(error_category)?;
    let status = PathStatus::from(outcome);
    if status.is_success() && error_category.is_some() {
        return Err(invalid_transition());
    }
    if matches!(outcome, ValidationOutcome::DryRun)
        && session_mode(transaction, fence)? != CleanupMode::DryRun
    {
        return Err(invalid_transition());
    }
    transition_path(
        transaction,
        fence,
        item_ordinal,
        path_ordinal,
        PathStatus::Validating,
        status,
        ActivePhase::Running,
        ExpectedAttempt::Current,
        error_category,
        None,
        false,
        Some(completed_at),
        CancellationRequirement::Any,
    )
}

/// Record intent immediately before an effect. This still performs no effect.
fn mark_effect_started(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    item_ordinal: usize,
    path_ordinal: usize,
    started_at: SystemTime,
) -> Result<(), HistoryError> {
    if !path_accepts_effect(transaction, fence, item_ordinal)? {
        return Err(invalid_transition());
    }
    // Heartbeat cadence is evidence that the current owner is making progress,
    // never an expiry. Advancing it in the same transaction prevents a new
    // effect from beginning behind a stale fenced heartbeat.
    record_heartbeat(transaction, fence, started_at)?;
    transition_path(
        transaction,
        fence,
        item_ordinal,
        path_ordinal,
        PathStatus::Validating,
        PathStatus::EffectStarted,
        ActivePhase::Running,
        ExpectedAttempt::Current,
        None,
        Some(started_at),
        false,
        None,
        CancellationRequirement::NotRequested,
    )
}

fn finish_effect(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    item_ordinal: usize,
    path_ordinal: usize,
    outcome: EffectOutcome,
    error_category: Option<&str>,
    completed_at: SystemTime,
) -> Result<(), HistoryError> {
    validate_error(error_category)?;
    let status = PathStatus::from(outcome);
    if status.is_success() && error_category.is_some() {
        return Err(invalid_transition());
    }
    if status.is_success() && !success_matches_item(transaction, fence, item_ordinal, status)? {
        return Err(invalid_transition());
    }
    transition_path(
        transaction,
        fence,
        item_ordinal,
        path_ordinal,
        PathStatus::EffectStarted,
        status,
        ActivePhase::Running,
        ExpectedAttempt::Current,
        error_category,
        None,
        false,
        Some(completed_at),
        CancellationRequirement::Any,
    )?;
    if outcome == EffectOutcome::OutcomeUnknown {
        require_one(
            transaction
                .execute(
                    "UPDATE cleanup_sessions SET status = 'recovering'
                     WHERE session_id = ?1 AND record_format_version = 2
                       AND status = 'running' AND execution_owner_id = ?2
                       AND execution_generation = ?3",
                    params![
                        fence.session_id.as_str(),
                        fence.owner.as_str(),
                        to_generation(fence.generation)?,
                    ],
                )
                .map_err(map_write_sql_error)?,
        )?;
    }
    Ok(())
}

fn assess_recovery(
    journal: &CleanupJournal,
    current_provenance: Option<&ExecutionProvenance>,
) -> Result<RecoveryAssessment, HistoryError> {
    let JournalLifecycle::Active {
        phase,
        fence: stale_fence,
        heartbeat_at,
        cancellation_requested,
    } = &journal.lifecycle
    else {
        return Err(invalid_transition());
    };
    match compare_execution_provenance(journal.execution_provenance.as_ref(), current_provenance) {
        ProvenanceRelationship::PriorBoot => Ok(RecoveryAssessment::PriorBoot),
        ProvenanceRelationship::ForeignHost => Ok(RecoveryAssessment::ForeignHost),
        ProvenanceRelationship::Unproven => Ok(RecoveryAssessment::Unproven),
        ProvenanceRelationship::SameBoot => match probe_process_instance(&stale_fence.owner) {
            ProcessLiveness::Alive => Ok(RecoveryAssessment::OwnerAlive),
            ProcessLiveness::Unknown => Ok(RecoveryAssessment::LivenessUnknown),
            ProcessLiveness::DefinitelyGone => {
                Ok(RecoveryAssessment::Recoverable(RecoveryPermit {
                    stale_fence: stale_fence.clone(),
                    phase: *phase,
                    heartbeat_at_unix_ms: system_time_to_unix_ms(
                        *heartbeat_at,
                        HistoryErrorKind::CorruptData,
                    )?,
                    cancellation_requested: *cancellation_requested,
                }))
            }
        },
    }
}

fn claim_recovery(
    transaction: &Transaction<'_>,
    permit: RecoveryPermit,
    new_owner: ProcessInstanceId,
    provenance: &ExecutionProvenance,
    claimed_at: SystemTime,
) -> Result<ExecutionFence, HistoryError> {
    let observed = load_cleanup_journal(transaction, &permit.stale_fence.session_id)?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    let JournalLifecycle::Active {
        phase,
        fence,
        heartbeat_at,
        cancellation_requested,
    } = observed.lifecycle
    else {
        return Err(invalid_transition());
    };
    if phase != permit.phase
        || fence != permit.stale_fence
        || system_time_to_unix_ms(heartbeat_at, HistoryErrorKind::CorruptData)?
            != permit.heartbeat_at_unix_ms
        || cancellation_requested != permit.cancellation_requested
    {
        return Err(invalid_transition());
    }
    let old_generation = to_generation(permit.stale_fence.generation)?;
    let new_generation_u64 = permit
        .stale_fence
        .generation
        .checked_add(1)
        .ok_or_else(invalid_transition)?;
    let new_generation = to_generation(new_generation_u64)?;
    let now = system_time_to_unix_ms(claimed_at, HistoryErrorKind::InvalidInput)?;
    if claimed_at < observed.started_at || now < permit.heartbeat_at_unix_ms {
        return Err(invalid_transition());
    }
    let changed = transaction
        .execute(
            "UPDATE cleanup_sessions
             SET status = 'recovering', execution_owner_id = ?6,
                 execution_generation = ?7, last_heartbeat_at_unix_ms = ?8,
                 execution_host_identity_v1_sha256 = ?10,
                 execution_boot_scope_v1_sha256 = ?11,
                 execution_recovery_policy = 'resumable'
             WHERE session_id = ?1 AND record_format_version = 2 AND status = ?2
               AND execution_owner_id = ?3 AND execution_generation = ?4
               AND last_heartbeat_at_unix_ms = ?5 AND cancellation_requested = ?9
               AND execution_host_identity_v1_sha256 = ?10
               AND execution_boot_scope_v1_sha256 = ?11
               AND execution_recovery_policy = 'resumable'
               AND completed_at_unix_ms IS NULL",
            params![
                permit.stale_fence.session_id.as_str(),
                permit.phase.stored(),
                permit.stale_fence.owner.as_str(),
                old_generation,
                permit.heartbeat_at_unix_ms,
                new_owner.as_str(),
                new_generation,
                now,
                permit.cancellation_requested,
                provenance.stable_host(),
                provenance.boot_scope(),
            ],
        )
        .map_err(map_write_sql_error)?;
    require_one(changed)?;
    let fence = ExecutionFence {
        session_id: permit.stale_fence.session_id,
        owner: new_owner,
        generation: new_generation_u64,
    };

    // Validation never began an effect and is safe to repeat. An interrupted
    // effect is never guessed: its outcome remains explicitly unknown.
    fenced_bulk_path_update(
        transaction,
        &fence,
        "status = 'planned', attempt_generation = NULL, error_category = NULL,
         effect_started_at_unix_ms = NULL, completed_at_unix_ms = NULL",
        "status = 'validating' AND attempt_generation = ?4",
        Some(old_generation),
        None,
    )?;
    fenced_bulk_path_update(
        transaction,
        &fence,
        "status = 'outcome_unknown', completed_at_unix_ms = ?5",
        "status = 'effect_started' AND attempt_generation = ?4",
        Some(old_generation),
        Some(now),
    )?;
    recompute_all_items(transaction, &fence, Some(now))?;
    let recovered = load_cleanup_journal(transaction, &fence.session_id)?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    if !matches!(
        recovered.lifecycle,
        JournalLifecycle::Active {
            phase: ActivePhase::Recovering,
            fence: ref loaded_fence,
            ..
        } if loaded_fence == &fence
    ) {
        return Err(corrupt());
    }
    Ok(fence)
}

fn resume_recovery(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
) -> Result<(), HistoryError> {
    let journal = load_cleanup_journal(transaction, &fence.session_id)?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    let JournalLifecycle::Active {
        phase: ActivePhase::Recovering,
        fence: ref loaded_fence,
        cancellation_requested: false,
        ..
    } = journal.lifecycle
    else {
        return Err(invalid_transition());
    };
    if loaded_fence != fence
        || !journal
            .items
            .iter()
            .flat_map(|item| &item.paths)
            .any(|path| path.status == PathStatus::Planned)
    {
        return Err(invalid_transition());
    }
    let generation = to_generation(fence.generation)?;
    let unresolved: i64 = transaction
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM cleanup_item_paths
                 WHERE session_id = ?1 AND status = 'outcome_unknown'
             )",
            [fence.session_id.as_str()],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    if unresolved != 0 {
        return Err(invalid_transition());
    }
    require_one(
        transaction
            .execute(
                "UPDATE cleanup_sessions SET status = 'running'
                 WHERE session_id = ?1 AND record_format_version = 2
                   AND status = 'recovering' AND execution_owner_id = ?2
                   AND execution_generation = ?3 AND cancellation_requested = 0",
                params![fence.session_id.as_str(), fence.owner.as_str(), generation],
            )
            .map_err(map_write_sql_error)?,
    )
}

fn settle_cancellation(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    completed_at: SystemTime,
) -> Result<(), HistoryError> {
    let journal = load_cleanup_journal(transaction, &fence.session_id)?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    let JournalLifecycle::Active {
        fence: ref loaded_fence,
        heartbeat_at,
        cancellation_requested: true,
        ..
    } = journal.lifecycle
    else {
        return Err(invalid_transition());
    };
    if loaded_fence != fence || completed_at < journal.started_at || completed_at < heartbeat_at {
        return Err(invalid_transition());
    }
    let now = system_time_to_unix_ms(completed_at, HistoryErrorKind::InvalidInput)?;
    let generation = to_generation(fence.generation)?;
    let requested: Option<i64> = transaction
        .query_row(
            "SELECT cancellation_requested FROM cleanup_sessions
             WHERE session_id = ?1 AND record_format_version = 2
               AND status IN ('running', 'recovering')
               AND execution_owner_id = ?2 AND execution_generation = ?3",
            params![fence.session_id.as_str(), fence.owner.as_str(), generation],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    if requested != Some(1) {
        return Err(invalid_transition());
    }
    fenced_bulk_path_update(
        transaction,
        fence,
        "status = 'interrupted', attempt_generation = ?3,
         error_category = NULL, completed_at_unix_ms = ?5",
        "status IN ('planned', 'validating')",
        None,
        Some(now),
    )?;
    recompute_all_items(transaction, fence, Some(now))?;
    Ok(())
}

fn reconcile_unknown_outcome(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    item_ordinal: usize,
    path_ordinal: usize,
    outcome: ReconciledOutcome,
    error_category: Option<&str>,
    completed_at: SystemTime,
) -> Result<(), HistoryError> {
    validate_error(error_category)?;
    let status = PathStatus::from(outcome);
    if status.is_success() && error_category.is_some() {
        return Err(invalid_transition());
    }
    if status.is_success() && !success_matches_item(transaction, fence, item_ordinal, status)? {
        return Err(invalid_transition());
    }
    transition_path(
        transaction,
        fence,
        item_ordinal,
        path_ordinal,
        PathStatus::OutcomeUnknown,
        status,
        ActivePhase::Recovering,
        ExpectedAttempt::AtMostCurrent,
        error_category,
        None,
        false,
        Some(completed_at),
        CancellationRequirement::Any,
    )
}

/// Settle a durable effect intent when final revalidation observed
/// cancellation and the caller therefore knows the OS effect was not called.
fn cancel_effect_before_call(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    item_ordinal: usize,
    path_ordinal: usize,
    completed_at: SystemTime,
) -> Result<(), HistoryError> {
    transition_path(
        transaction,
        fence,
        item_ordinal,
        path_ordinal,
        PathStatus::EffectStarted,
        PathStatus::Interrupted,
        ActivePhase::Running,
        ExpectedAttempt::Current,
        None,
        None,
        true,
        Some(completed_at),
        CancellationRequirement::Requested,
    )
}

/// Derive and persist the only valid terminal parent status.
fn terminalize(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    completed_at: SystemTime,
    verified_capacity_delta_bytes: Option<i64>,
) -> Result<TerminalSessionStatus, HistoryError> {
    let journal = load_cleanup_journal(transaction, &fence.session_id)?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    let JournalLifecycle::Active {
        fence: ref loaded_fence,
        heartbeat_at,
        cancellation_requested,
        ..
    } = journal.lifecycle
    else {
        return Err(invalid_transition());
    };
    if loaded_fence != fence {
        return Err(invalid_transition());
    }
    let statuses = journal
        .items
        .iter()
        .flat_map(|item| item.paths.iter().map(|path| path.status))
        .collect::<Vec<_>>();
    let status = derive_terminal_session_status(&statuses, journal.mode, cancellation_requested)?;
    if completed_at < heartbeat_at
        || completed_at < journal.started_at
        || journal
            .items
            .iter()
            .flat_map(|item| &item.paths)
            .filter_map(|path| path.completed_at)
            .any(|path_completed| path_completed > completed_at)
    {
        return Err(invalid_transition());
    }
    if journal.candidate_status_coupling.claims_candidates() {
        settle_candidate_plan_claims(transaction, &journal, status)?;
    }
    let completed = system_time_to_unix_ms(completed_at, HistoryErrorKind::InvalidInput)?;
    let generation = to_generation(fence.generation)?;
    require_one(
        transaction
            .execute(
                "UPDATE cleanup_sessions
                 SET status = ?4, completed_at_unix_ms = ?5,
                     verified_capacity_delta_bytes = ?6
                 WHERE session_id = ?1 AND record_format_version = 2
                   AND status IN ('running', 'recovering')
                   AND execution_owner_id = ?2 AND execution_generation = ?3
                   AND completed_at_unix_ms IS NULL",
                params![
                    fence.session_id.as_str(),
                    fence.owner.as_str(),
                    generation,
                    status.stored(),
                    completed,
                    verified_capacity_delta_bytes,
                ],
            )
            .map_err(map_write_sql_error)?,
    )?;
    Ok(status)
}

fn settle_candidate_plan_claims(
    transaction: &Transaction<'_>,
    journal: &CleanupJournal,
    session_status: TerminalSessionStatus,
) -> Result<(), HistoryError> {
    for item in &journal.items {
        let prior = item.frozen.prior_review_status.ok_or_else(corrupt)?;
        let settlement = match item.status {
            PathStatus::Trashed | PathStatus::Removed | PathStatus::Evicted => {
                CandidatePlanSettlement::Completed
            }
            PathStatus::DryRun => CandidatePlanSettlement::Restore(prior),
            PathStatus::Interrupted | PathStatus::Skipped
                if session_status == TerminalSessionStatus::Cancelled =>
            {
                CandidatePlanSettlement::Restore(prior)
            }
            PathStatus::Skipped
            | PathStatus::Rejected
            | PathStatus::Failed
            | PathStatus::ChangedSincePlan
            | PathStatus::Interrupted
            | PathStatus::Unavailable => CandidatePlanSettlement::Failed,
            PathStatus::Planned
            | PathStatus::Validating
            | PathStatus::EffectStarted
            | PathStatus::OutcomeUnknown => return Err(invalid_transition()),
        };
        settle_candidate_plan_claim(transaction, journal, item, settlement)?;
    }
    Ok(())
}

fn settle_candidate_plan_claim(
    transaction: &Transaction<'_>,
    journal: &CleanupJournal,
    item: &JournalItem,
    settlement: CandidatePlanSettlement,
) -> Result<(), HistoryError> {
    let prior = item.frozen.prior_review_status.ok_or_else(corrupt)?;
    settle_planned_candidate(transaction, &item.frozen.candidate_id, settlement)?;
    let deleted = transaction
        .execute(
            "DELETE FROM candidate_plan_claims
             WHERE candidate_id = ?1 AND session_id = ?2
               AND item_ordinal = ?3 AND prior_review_status = ?4",
            params![
                item.frozen.candidate_id.as_str(),
                journal.session_id.as_str(),
                item.frozen.ordinal as i64,
                prior.as_stored(),
            ],
        )
        .map_err(map_write_sql_error)?;
    require_one(deleted)
}

#[allow(clippy::too_many_arguments)]
fn transition_path(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    item_ordinal: usize,
    path_ordinal: usize,
    expected: PathStatus,
    next: PathStatus,
    required_phase: ActivePhase,
    expected_attempt: ExpectedAttempt,
    error_category: Option<&str>,
    effect_started_at: Option<SystemTime>,
    clear_effect_started: bool,
    completed_at: Option<SystemTime>,
    cancellation: CancellationRequirement,
) -> Result<(), HistoryError> {
    let generation = to_generation(fence.generation)?;
    let item = i64::try_from(item_ordinal)
        .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    let path = i64::try_from(path_ordinal)
        .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    let effect = effect_started_at
        .map(|value| system_time_to_unix_ms(value, HistoryErrorKind::InvalidInput))
        .transpose()?;
    let completed = completed_at
        .map(|value| system_time_to_unix_ms(value, HistoryErrorKind::InvalidInput))
        .transpose()?;
    let session_started: Option<i64> = transaction
        .query_row(
            "SELECT started_at_unix_ms FROM cleanup_sessions
             WHERE session_id = ?1 AND record_format_version = 2
               AND status = ?4 AND execution_owner_id = ?2
               AND execution_generation = ?3",
            params![
                fence.session_id.as_str(),
                fence.owner.as_str(),
                generation,
                required_phase.stored(),
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    let session_started = session_started.ok_or_else(invalid_transition)?;
    if effect.is_some_and(|value| value < session_started)
        || completed.is_some_and(|value| value < session_started)
    {
        return Err(invalid_transition());
    }
    let cancellation_predicate = match cancellation {
        CancellationRequirement::Any => "",
        CancellationRequirement::NotRequested => "AND parent.cancellation_requested = 0",
        CancellationRequirement::Requested => "AND parent.cancellation_requested = 1",
    };
    let attempt_predicate = match expected_attempt {
        ExpectedAttempt::None => "AND attempt_generation IS NULL",
        ExpectedAttempt::Current => "AND attempt_generation = ?3",
        ExpectedAttempt::AtMostCurrent => {
            "AND attempt_generation IS NOT NULL AND attempt_generation <= ?3"
        }
    };
    let sql = format!(
        "UPDATE cleanup_item_paths
         SET status = ?6, attempt_generation = CASE
                 WHEN ?6 = 'planned' THEN NULL ELSE COALESCE(attempt_generation, ?3) END,
             error_category = ?7,
             effect_started_at_unix_ms = CASE
                 WHEN ?12 = 1 THEN NULL ELSE COALESCE(?8, effect_started_at_unix_ms) END,
             completed_at_unix_ms = ?9
         WHERE session_id = ?1 AND item_ordinal = ?4 AND path_ordinal = ?5
           AND status = ?10
           {attempt_predicate}
           AND EXISTS (
               SELECT 1 FROM cleanup_sessions AS parent
               WHERE parent.session_id = cleanup_item_paths.session_id
                 AND parent.record_format_version = 2
                 AND parent.status = ?11
                 AND parent.execution_owner_id = ?2
                 AND parent.execution_generation = ?3
                 {cancellation_predicate}
           )"
    );
    let changed = transaction
        .execute(
            &sql,
            params![
                fence.session_id.as_str(),
                fence.owner.as_str(),
                generation,
                item,
                path,
                next.stored(),
                error_category,
                effect,
                completed,
                expected.stored(),
                required_phase.stored(),
                i64::from(clear_effect_started),
            ],
        )
        .map_err(map_write_sql_error)?;
    require_one(changed)?;
    recompute_item(transaction, fence, item_ordinal, completed)?;
    Ok(())
}

fn session_mode(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
) -> Result<CleanupMode, HistoryError> {
    let generation = to_generation(fence.generation)?;
    let mode: Option<String> = transaction
        .query_row(
            "SELECT mode FROM cleanup_sessions
             WHERE session_id = ?1 AND record_format_version = 2
               AND status = 'running'
               AND execution_owner_id = ?2 AND execution_generation = ?3",
            params![fence.session_id.as_str(), fence.owner.as_str(), generation],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    mode_from_stored(mode.as_deref().ok_or_else(invalid_transition)?)
}

fn path_accepts_effect(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    item_ordinal: usize,
) -> Result<bool, HistoryError> {
    let generation = to_generation(fence.generation)?;
    let item = i64::try_from(item_ordinal)
        .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    let pair: Option<(String, String)> = transaction
        .query_row(
            "SELECT parent.mode, child.proposed_action
             FROM cleanup_sessions AS parent
             JOIN cleanup_items AS child ON child.session_id = parent.session_id
             WHERE parent.session_id = ?1 AND child.item_ordinal = ?4
               AND parent.record_format_version = 2
               AND parent.status = 'running'
               AND parent.execution_owner_id = ?2 AND parent.execution_generation = ?3
               AND parent.cancellation_requested = 0",
            params![
                fence.session_id.as_str(),
                fence.owner.as_str(),
                generation,
                item
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    let Some((mode, action)) = pair else {
        return Ok(false);
    };
    let mode = mode_from_stored(&mode)?;
    let action = action_from_stored(&action)?;
    Ok(mode != CleanupMode::DryRun && mode_accepts_action(mode, action))
}

fn success_matches_item(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    item_ordinal: usize,
    status: PathStatus,
) -> Result<bool, HistoryError> {
    let generation = to_generation(fence.generation)?;
    let item = i64::try_from(item_ordinal)
        .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    let pair: Option<(String, String)> = transaction
        .query_row(
            "SELECT parent.mode, child.proposed_action
             FROM cleanup_sessions AS parent
             JOIN cleanup_items AS child ON child.session_id = parent.session_id
             WHERE parent.session_id = ?1 AND child.item_ordinal = ?4
               AND parent.record_format_version = 2
               AND parent.status IN ('running', 'recovering')
               AND parent.execution_owner_id = ?2 AND parent.execution_generation = ?3",
            params![
                fence.session_id.as_str(),
                fence.owner.as_str(),
                generation,
                item
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    let Some((mode, action)) = pair else {
        return Ok(false);
    };
    Ok(success_matches(
        mode_from_stored(&mode)?,
        action_from_stored(&action)?,
        status,
    ))
}

fn success_matches(mode: CleanupMode, action: CandidateAction, status: PathStatus) -> bool {
    matches!(
        (mode, action, status),
        (CleanupMode::DryRun, _, PathStatus::DryRun)
            | (
                CleanupMode::Trash,
                CandidateAction::MoveToTrash,
                PathStatus::Trashed
            )
            | (
                CleanupMode::PermanentSafe,
                CandidateAction::RemoveKnownRegenerableContents,
                PathStatus::Removed
            )
            | (
                CleanupMode::EvictLocalCopy,
                CandidateAction::EvictLocalCopy,
                PathStatus::Evicted
            )
    )
}

fn fenced_bulk_path_update(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    assignment: &'static str,
    condition: &'static str,
    prior_generation: Option<i64>,
    completed_at_unix_ms: Option<i64>,
) -> Result<(), HistoryError> {
    // Both fragments are compile-time-only call-site constants in this module.
    let sql = format!(
        "UPDATE cleanup_item_paths SET {assignment}
         WHERE session_id = ?1 AND {condition}
           AND (?4 IS NULL OR ?4 IS NOT NULL) AND (?5 IS NULL OR ?5 IS NOT NULL)
           AND EXISTS (
               SELECT 1 FROM cleanup_sessions AS parent
               WHERE parent.session_id = cleanup_item_paths.session_id
                 AND parent.record_format_version = 2
                 AND parent.status IN ('running', 'recovering')
                 AND parent.execution_owner_id = ?2
                 AND parent.execution_generation = ?3
           )"
    );
    transaction
        .execute(
            &sql,
            params![
                fence.session_id.as_str(),
                fence.owner.as_str(),
                to_generation(fence.generation)?,
                prior_generation,
                completed_at_unix_ms,
            ],
        )
        .map_err(map_write_sql_error)?;
    Ok(())
}

fn recompute_all_items(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    completed_at_unix_ms: Option<i64>,
) -> Result<(), HistoryError> {
    let ordinals = {
        let mut statement = transaction
            .prepare(
                "SELECT item_ordinal FROM cleanup_items
                 WHERE session_id = ?1 ORDER BY item_ordinal LIMIT 65",
            )
            .map_err(map_query_sql_error)?;
        let values = statement
            .query_map([fence.session_id.as_str()], |row| row.get::<_, i64>(0))
            .map_err(map_query_sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_query_sql_error)?;
        if values.len() > MAX_ITEMS {
            return Err(corrupt());
        }
        values
    };
    for (expected, ordinal) in ordinals.into_iter().enumerate() {
        if ordinal != expected as i64 {
            return Err(corrupt());
        }
        recompute_item(transaction, fence, expected, completed_at_unix_ms)?;
    }
    Ok(())
}

fn recompute_item(
    transaction: &Transaction<'_>,
    fence: &ExecutionFence,
    item_ordinal: usize,
    _completed_at_unix_ms: Option<i64>,
) -> Result<(), HistoryError> {
    let item = i64::try_from(item_ordinal).map_err(|_| corrupt())?;
    let paths = load_path_statuses(transaction, &fence.session_id, item_ordinal)?;
    if paths.is_empty() {
        return Err(corrupt());
    }
    let status = derive_item_status(&paths);
    let error = paths
        .iter()
        .find(|path| path.status == status)
        .and_then(|path| path.error_category.as_deref());
    let generation = to_generation(fence.generation)?;
    require_one(
        transaction
            .execute(
                "UPDATE cleanup_items SET final_status = ?5, error_category = ?6
                 WHERE session_id = ?1 AND item_ordinal = ?4
                   AND record_format_version = 2
                   AND EXISTS (
                       SELECT 1 FROM cleanup_sessions AS parent
                       WHERE parent.session_id = cleanup_items.session_id
                         AND parent.record_format_version = 2
                         AND parent.status IN ('running', 'recovering')
                         AND parent.execution_owner_id = ?2
                         AND parent.execution_generation = ?3
                   )",
                params![
                    fence.session_id.as_str(),
                    fence.owner.as_str(),
                    generation,
                    item,
                    status.stored(),
                    error,
                ],
            )
            .map_err(map_write_sql_error)?,
    )
}

fn derive_item_status<P: DynamicPathState>(paths: &[P]) -> PathStatus {
    const PRECEDENCE: &[PathStatus] = &[
        PathStatus::EffectStarted,
        PathStatus::Validating,
        PathStatus::Planned,
        PathStatus::OutcomeUnknown,
        PathStatus::Interrupted,
        PathStatus::Failed,
        PathStatus::ChangedSincePlan,
        PathStatus::Rejected,
        PathStatus::Unavailable,
        PathStatus::Skipped,
        PathStatus::Trashed,
        PathStatus::Removed,
        PathStatus::Evicted,
        PathStatus::DryRun,
    ];
    PRECEDENCE
        .iter()
        .copied()
        .find(|status| paths.iter().any(|path| path.status() == *status))
        .unwrap_or(PathStatus::Failed)
}

fn derive_terminal_session_status(
    statuses: &[PathStatus],
    mode: CleanupMode,
    cancellation_requested: bool,
) -> Result<TerminalSessionStatus, HistoryError> {
    if statuses.is_empty()
        || statuses
            .iter()
            .any(|status| !status.is_terminal() || *status == PathStatus::OutcomeUnknown)
        || statuses.iter().any(|status| {
            status.is_success()
                && !matches!(
                    (mode, status),
                    (CleanupMode::DryRun, PathStatus::DryRun)
                        | (CleanupMode::Trash, PathStatus::Trashed)
                        | (CleanupMode::PermanentSafe, PathStatus::Removed)
                        | (CleanupMode::EvictLocalCopy, PathStatus::Evicted)
                )
        })
    {
        return Err(invalid_transition());
    }
    let success = statuses.iter().filter(|status| status.is_success()).count();
    if success == statuses.len() {
        return Ok(if mode == CleanupMode::DryRun {
            TerminalSessionStatus::DryRun
        } else {
            TerminalSessionStatus::Completed
        });
    }
    if success > 0 {
        return Ok(TerminalSessionStatus::PartiallyCompleted);
    }
    if statuses.iter().any(|status| {
        matches!(
            status,
            PathStatus::Failed | PathStatus::ChangedSincePlan | PathStatus::Unavailable
        )
    }) {
        return Ok(TerminalSessionStatus::Failed);
    }
    if cancellation_requested
        && statuses.contains(&PathStatus::Interrupted)
        && statuses
            .iter()
            .all(|status| matches!(status, PathStatus::Interrupted | PathStatus::Skipped))
    {
        return Ok(TerminalSessionStatus::Cancelled);
    }
    if statuses.contains(&PathStatus::Interrupted) {
        return Ok(TerminalSessionStatus::Interrupted);
    }
    if statuses
        .iter()
        .all(|status| matches!(status, PathStatus::Rejected | PathStatus::Skipped))
    {
        return Ok(TerminalSessionStatus::Rejected);
    }
    Ok(TerminalSessionStatus::Failed)
}
