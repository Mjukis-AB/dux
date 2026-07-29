use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::SystemTime;

use crate::domain::{
    BlockReason, CandidateAction, CandidateCategory, CandidateId, CoveragePermille, EvidenceKind,
    RuleRef, SafetyTier, ScanCoverage, ScanCoverageStatus, ScanId,
};

/// Opaque, non-durable identifier scoped to one running DUX process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct TaskId(NonZeroU64);

impl TaskId {
    pub fn get(self) -> u64 {
        self.0.get()
    }

    pub(super) const fn from_nonzero(value: NonZeroU64) -> Self {
        Self(value)
    }
}

/// Closed set of engine-owned task kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskKind {
    FormatSizeBatch,
    Scan,
    ScanRecoveryMaintenance,
    CandidateEvaluationRecoveryMaintenance,
    HistoryMaintenance,
    SnapshotRetention,
    SnapshotOrphanMaintenance,
    SnapshotProvisioningStageMaintenance,
    SnapshotTerminalTempMaintenance,
    SnapshotUnleasedTempMaintenance,
    PermanentSafeCleanup,
}

/// Execution phase. Cancellation intent is reported separately until work is
/// actually quiescent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskPhase {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl TaskPhase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskFailureKind {
    InternalFailure,
    ScanRootChanged,
    ScanFailed,
    SnapshotRejected,
    PersistenceUnavailable,
    PersistenceOutcomeUnknown,
    ScanRecoveryMaintenance(ScanRecoveryMaintenanceFailureKind),
    CandidateEvaluationRecoveryMaintenance(CandidateEvaluationRecoveryMaintenanceFailureKind),
    HistoryMaintenance(HistoryMaintenanceFailureKind),
    SnapshotRetention(SnapshotRetentionFailureKind),
    SnapshotOrphanMaintenance(SnapshotOrphanMaintenanceFailureKind),
    SnapshotProvisioningStageMaintenance(SnapshotProvisioningStageMaintenanceFailureKind),
    SnapshotTerminalTempMaintenance(SnapshotTerminalTempMaintenanceFailureKind),
    SnapshotUnleasedTempMaintenance(SnapshotUnleasedTempMaintenanceFailureKind),
    PermanentSafeCleanup(PermanentSafeCleanupFailureKind),
}

/// Path-free terminal failure categories for one engine-owned permanent-safe
/// cleanup task. Known durable cleanup terminal states are returned as task
/// results instead; `OutcomeUnknown` always requires history/recovery and must
/// never be retried automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PermanentSafeCleanupFailureKind {
    ParentReviewUnavailable,
    ReviewExpired,
    ChangedDuringReview,
    BudgetExceeded,
    Busy,
    UnsafeStorage,
    IncompatibleSchema,
    CorruptData,
    OutcomeUnknown,
    Unavailable,
    InternalState,
}

/// Path-free failure categories for durable running-scan recovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanRecoveryMaintenanceFailureKind {
    InvalidClock,
    IncompatibleSchema,
    Busy,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    Unavailable,
    OutcomeUnknown,
    InternalState,
}

/// Path-free failure categories for durable pending candidate-evaluation
/// recovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CandidateEvaluationRecoveryMaintenanceFailureKind {
    InvalidClock,
    IncompatibleSchema,
    Busy,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    Unavailable,
    OutcomeUnknown,
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HistoryMaintenanceFailureKind {
    InvalidClock,
    IncompatibleSchema,
    Busy,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    Unavailable,
    OutcomeUnknown,
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotRetentionFailureKind {
    InvalidClock,
    IncompatibleSchema,
    Busy,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    IncompatibleSnapshot,
    Unavailable,
    OutcomeUnknown,
    InternalState,
}

/// Path-free failure categories for physical-orphan reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotOrphanMaintenanceFailureKind {
    InvalidClock,
    IncompatibleSchema,
    Busy,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    IncompatibleSnapshot,
    Unavailable,
    OutcomeUnknown,
    InternalState,
}

/// Path-free failure categories for root-local snapshot provisioning-stage
/// reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotProvisioningStageMaintenanceFailureKind {
    InvalidClock,
    IncompatibleSchema,
    Busy,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    Unavailable,
    OutcomeUnknown,
    InternalState,
}

/// Path-free failure categories for terminal snapshot-temp reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotTerminalTempMaintenanceFailureKind {
    InvalidClock,
    IncompatibleSchema,
    Busy,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    Unavailable,
    OutcomeUnknown,
    InternalState,
}

/// Path-free failure categories for unleased snapshot-temp reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotUnleasedTempMaintenanceFailureKind {
    InvalidClock,
    IncompatibleSchema,
    Busy,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    Unavailable,
    OutcomeUnknown,
    InternalState,
}

/// Authoritative current state for one retained task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskSnapshot {
    pub id: TaskId,
    pub kind: TaskKind,
    pub phase: TaskPhase,
    pub cancellation_requested: bool,
    pub revision: u64,
    pub result_available: bool,
    pub failure: Option<TaskFailureKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskEventKind {
    Queued,
    Started,
    Progress {
        completed: u64,
        total: u64,
    },
    ScanProgress {
        files: u64,
        directories: u64,
        known_allocated_bytes: u64,
        errors: u64,
    },
    ScanFinalizing,
    CandidateEvaluationStarted,
    CandidateEvaluationFinished {
        status: CandidateEvaluationTaskStatus,
    },
    ScanRecoveryMaintenanceBatchApplying,
    ScanRecoveryMaintenanceBatchFinished {
        outcome: ScanRecoveryMaintenanceOutcome,
        claimed_count_before: u32,
        claimed_count_after: u32,
        alive_count: u32,
        unknown_count: u32,
        recoverable_count: u32,
        has_more: bool,
    },
    CandidateEvaluationRecoveryMaintenanceApplying,
    CandidateEvaluationRecoveryMaintenanceFinished {
        outcome: CandidateEvaluationRecoveryMaintenanceOutcome,
        has_more: bool,
    },
    HistoryMaintenanceBatchApplying,
    HistoryMaintenanceBatchFinished {
        daily_rollups_created: u32,
        raw_samples_pruned: u32,
        daily_rollups_pruned: u32,
        ai_insights_pruned: u32,
        has_more: bool,
    },
    SnapshotRetentionBatchApplying,
    SnapshotRetentionBatchFinished {
        outcome: SnapshotRetentionOutcome,
        cap_bytes: u64,
        charged_bytes_before: u64,
        charged_bytes_after: u64,
        has_more: bool,
    },
    SnapshotOrphanMaintenanceBatchApplying,
    SnapshotOrphanMaintenanceBatchFinished {
        outcome: SnapshotOrphanMaintenanceOutcome,
        orphan_count_before: u32,
        orphan_count_after: u32,
        orphan_charged_bytes_before: u64,
        orphan_charged_bytes_after: u64,
        has_more: bool,
    },
    SnapshotProvisioningStageMaintenanceBatchApplying,
    SnapshotProvisioningStageMaintenanceBatchFinished {
        outcome: SnapshotProvisioningStageMaintenanceOutcome,
        total_stage_count_before: u64,
        total_stage_count_after: u64,
        marker_owned_count_before: u64,
        marker_owned_count_after: u64,
        unproven_count_before: u64,
        unproven_count_after: u64,
        control_charged_bytes_before: u64,
        control_charged_bytes_after: u64,
        has_more: bool,
    },
    SnapshotTerminalTempMaintenanceBatchApplying,
    SnapshotTerminalTempMaintenanceBatchFinished {
        outcome: SnapshotTerminalTempMaintenanceOutcome,
        terminal_lease_count_before: u32,
        terminal_lease_count_after: u32,
        active_terminal_lease_count_before: u32,
        active_terminal_lease_count_after: u32,
        terminal_charged_bytes_before: u64,
        terminal_charged_bytes_after: u64,
        has_more: bool,
    },
    SnapshotUnleasedTempMaintenanceBatchApplying,
    SnapshotUnleasedTempMaintenanceBatchFinished {
        outcome: SnapshotUnleasedTempMaintenanceOutcome,
        unleased_temp_count_before: u32,
        unleased_temp_count_after: u32,
        active_unleased_temp_count_before: u32,
        active_unleased_temp_count_after: u32,
        unleased_charged_bytes_before: u64,
        unleased_charged_bytes_after: u64,
        has_more: bool,
    },
    CancellationRequested,
    Terminal {
        phase: TaskPhase,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskEvent {
    pub sequence: u64,
    pub kind: TaskEventKind,
}

/// Bounded pull page of task events. `truncated` means the requested cursor
/// predates the oldest event still retained in the per-task ring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskEventBatch {
    pub events: Vec<TaskEvent>,
    pub next_sequence: u64,
    pub oldest_available_sequence: u64,
    pub truncated: bool,
    pub terminal: bool,
}

/// One deterministic read-only formatting result. The display value remains a
/// migration probe, not a localization or cleanup authority contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormattedSizeEntry {
    pub bytes: u64,
    pub display: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatSizeBatchResult {
    entries: Arc<[FormattedSizeEntry]>,
}

/// Immutable, path-free outcome from one bounded history-maintenance batch.
/// `has_more` asks an idle caller to schedule another task; one engine task
/// never extends its writer transaction into an unbounded drain loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryMaintenanceResult {
    observed_at: SystemTime,
    daily_rollups_created: u32,
    raw_samples_pruned: u32,
    daily_rollups_pruned: u32,
    ai_insights_pruned: u32,
    has_more: bool,
}

impl HistoryMaintenanceResult {
    pub(super) const fn new(
        observed_at: SystemTime,
        daily_rollups_created: u32,
        raw_samples_pruned: u32,
        daily_rollups_pruned: u32,
        ai_insights_pruned: u32,
        has_more: bool,
    ) -> Self {
        Self {
            observed_at,
            daily_rollups_created,
            raw_samples_pruned,
            daily_rollups_pruned,
            ai_insights_pruned,
            has_more,
        }
    }

    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }

    pub const fn daily_rollups_created(&self) -> u32 {
        self.daily_rollups_created
    }

    pub const fn raw_samples_pruned(&self) -> u32 {
        self.raw_samples_pruned
    }

    pub const fn daily_rollups_pruned(&self) -> u32 {
        self.daily_rollups_pruned
    }

    pub const fn ai_insights_pruned(&self) -> u32 {
        self.ai_insights_pruned
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

/// Path-free outcome of one bounded running-scan recovery decision. Exact
/// scan and process-instance identities stay private inside persistence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanRecoveryMaintenanceOutcome {
    NoClaim,
    DeferredUnproven,
    Interrupted,
    ChangedConcurrently,
}

/// Immutable path-free outcome from one inspected page of an idle-only
/// running-scan recovery batch.
/// `has_more` asks the caller to submit a later task; core never self-enqueues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanRecoveryMaintenanceResult {
    observed_at: SystemTime,
    outcome: ScanRecoveryMaintenanceOutcome,
    claimed_count_before: u32,
    claimed_count_after: u32,
    alive_count: u32,
    unknown_count: u32,
    recoverable_count: u32,
    has_more: bool,
}

impl ScanRecoveryMaintenanceResult {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        observed_at: SystemTime,
        outcome: ScanRecoveryMaintenanceOutcome,
        claimed_count_before: u32,
        claimed_count_after: u32,
        alive_count: u32,
        unknown_count: u32,
        recoverable_count: u32,
        has_more: bool,
    ) -> Self {
        Self {
            observed_at,
            outcome,
            claimed_count_before,
            claimed_count_after,
            alive_count,
            unknown_count,
            recoverable_count,
            has_more,
        }
    }

    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }

    pub const fn outcome(&self) -> ScanRecoveryMaintenanceOutcome {
        self.outcome
    }

    pub const fn claimed_count_before(&self) -> u32 {
        self.claimed_count_before
    }

    pub const fn claimed_count_after(&self) -> u32 {
        self.claimed_count_after
    }

    pub const fn alive_count(&self) -> u32 {
        self.alive_count
    }

    pub const fn unknown_count(&self) -> u32 {
        self.unknown_count
    }

    pub const fn recoverable_count(&self) -> u32 {
        self.recoverable_count
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

/// Path-free outcome of one bounded pending candidate-evaluation recovery
/// decision. Exact scan and candidate identities stay private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CandidateEvaluationRecoveryMaintenanceOutcome {
    None,
    Recovered { candidate_count: u32 },
    Incompatible,
}

/// Immutable outcome from one idle-only pending candidate-evaluation recovery
/// task. `has_more` asks the caller to submit a later task; core never
/// self-enqueues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandidateEvaluationRecoveryMaintenanceResult {
    observed_at: SystemTime,
    outcome: CandidateEvaluationRecoveryMaintenanceOutcome,
    has_more: bool,
}

impl CandidateEvaluationRecoveryMaintenanceResult {
    pub(super) const fn new(
        observed_at: SystemTime,
        outcome: CandidateEvaluationRecoveryMaintenanceOutcome,
        has_more: bool,
    ) -> Self {
        Self {
            observed_at,
            outcome,
            has_more,
        }
    }

    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }

    pub const fn outcome(&self) -> CandidateEvaluationRecoveryMaintenanceOutcome {
        self.outcome
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

/// Path-free outcome of one bounded snapshot-retention decision. Removed
/// snapshot identities remain private observations inside the repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotRetentionOutcome {
    UnderCap,
    DeferredUnstable,
    DeferredNoEligibleSnapshot,
    RemovedTombstonedResidual { bytes: u64 },
    TombstonedAndRemoved { bytes: u64 },
}

/// Immutable outcome from one idle-only snapshot-retention batch. `has_more`
/// asks the caller to retry at a later idle boundary; core never self-enqueues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotRetentionResult {
    observed_at: SystemTime,
    outcome: SnapshotRetentionOutcome,
    cap_bytes: u64,
    charged_bytes_before: u64,
    charged_bytes_after: u64,
    has_more: bool,
}

impl SnapshotRetentionResult {
    pub(super) const fn new(
        observed_at: SystemTime,
        outcome: SnapshotRetentionOutcome,
        cap_bytes: u64,
        charged_bytes_before: u64,
        charged_bytes_after: u64,
        has_more: bool,
    ) -> Self {
        Self {
            observed_at,
            outcome,
            cap_bytes,
            charged_bytes_before,
            charged_bytes_after,
            has_more,
        }
    }

    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }

    pub const fn outcome(&self) -> SnapshotRetentionOutcome {
        self.outcome
    }

    pub const fn cap_bytes(&self) -> u64 {
        self.cap_bytes
    }

    pub const fn charged_bytes_before(&self) -> u64 {
        self.charged_bytes_before
    }

    pub const fn charged_bytes_after(&self) -> u64 {
        self.charged_bytes_after
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

/// Path-free outcome of one bounded physical-orphan reconciliation. Exact
/// scan identities remain private repository observations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotOrphanMaintenanceOutcome {
    NoOrphan,
    Removed { bytes: u64 },
}

/// Immutable outcome from one idle-only physical-orphan batch. `has_more`
/// asks the caller to submit a later task; core never self-enqueues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotOrphanMaintenanceResult {
    observed_at: SystemTime,
    outcome: SnapshotOrphanMaintenanceOutcome,
    orphan_count_before: u32,
    orphan_count_after: u32,
    orphan_charged_bytes_before: u64,
    orphan_charged_bytes_after: u64,
    has_more: bool,
}

impl SnapshotOrphanMaintenanceResult {
    pub(super) const fn new(
        observed_at: SystemTime,
        outcome: SnapshotOrphanMaintenanceOutcome,
        orphan_count_before: u32,
        orphan_count_after: u32,
        orphan_charged_bytes_before: u64,
        orphan_charged_bytes_after: u64,
        has_more: bool,
    ) -> Self {
        Self {
            observed_at,
            outcome,
            orphan_count_before,
            orphan_count_after,
            orphan_charged_bytes_before,
            orphan_charged_bytes_after,
            has_more,
        }
    }

    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }

    pub const fn outcome(&self) -> SnapshotOrphanMaintenanceOutcome {
        self.outcome
    }

    pub const fn orphan_count_before(&self) -> u32 {
        self.orphan_count_before
    }

    pub const fn orphan_count_after(&self) -> u32 {
        self.orphan_count_after
    }

    pub const fn orphan_charged_bytes_before(&self) -> u64 {
        self.orphan_charged_bytes_before
    }

    pub const fn orphan_charged_bytes_after(&self) -> u64 {
        self.orphan_charged_bytes_after
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

/// Path-free outcome of one bounded root-local snapshot provisioning-stage
/// reconciliation. Exact stage names and filesystem identities stay private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotProvisioningStageMaintenanceOutcome {
    NoStage,
    DeferredUnproven,
    RemovedMarkerOnly { bytes: u64 },
    RemovedMarkerComplete { bytes: u64 },
}

/// Immutable outcome from one idle-only provisioning-stage batch. `has_more`
/// asks the caller to submit a later task; core never self-enqueues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotProvisioningStageMaintenanceResult {
    observed_at: SystemTime,
    outcome: SnapshotProvisioningStageMaintenanceOutcome,
    total_stage_count_before: u64,
    total_stage_count_after: u64,
    marker_owned_count_before: u64,
    marker_owned_count_after: u64,
    unproven_count_before: u64,
    unproven_count_after: u64,
    control_charged_bytes_before: u64,
    control_charged_bytes_after: u64,
    has_more: bool,
}

impl SnapshotProvisioningStageMaintenanceResult {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        observed_at: SystemTime,
        outcome: SnapshotProvisioningStageMaintenanceOutcome,
        total_stage_count_before: u64,
        total_stage_count_after: u64,
        marker_owned_count_before: u64,
        marker_owned_count_after: u64,
        unproven_count_before: u64,
        unproven_count_after: u64,
        control_charged_bytes_before: u64,
        control_charged_bytes_after: u64,
        has_more: bool,
    ) -> Self {
        Self {
            observed_at,
            outcome,
            total_stage_count_before,
            total_stage_count_after,
            marker_owned_count_before,
            marker_owned_count_after,
            unproven_count_before,
            unproven_count_after,
            control_charged_bytes_before,
            control_charged_bytes_after,
            has_more,
        }
    }

    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }

    pub const fn outcome(&self) -> SnapshotProvisioningStageMaintenanceOutcome {
        self.outcome
    }

    pub const fn total_stage_count_before(&self) -> u64 {
        self.total_stage_count_before
    }

    pub const fn total_stage_count_after(&self) -> u64 {
        self.total_stage_count_after
    }

    pub const fn marker_owned_count_before(&self) -> u64 {
        self.marker_owned_count_before
    }

    pub const fn marker_owned_count_after(&self) -> u64 {
        self.marker_owned_count_after
    }

    pub const fn unproven_count_before(&self) -> u64 {
        self.unproven_count_before
    }

    pub const fn unproven_count_after(&self) -> u64 {
        self.unproven_count_after
    }

    pub const fn control_charged_bytes_before(&self) -> u64 {
        self.control_charged_bytes_before
    }

    pub const fn control_charged_bytes_after(&self) -> u64 {
        self.control_charged_bytes_after
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

/// Path-free outcome of one bounded terminal snapshot-temp reconciliation.
/// Exact scan, lease, owner, and temporary-file identities stay private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotTerminalTempMaintenanceOutcome {
    NoTerminalResidual,
    DeferredActive,
    ReconciledRowOnly,
    RemovedTemp { bytes: u64 },
}

/// Immutable outcome from one idle-only terminal snapshot-temp batch.
/// `has_more` asks the caller to submit a later task; core never self-enqueues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotTerminalTempMaintenanceResult {
    observed_at: SystemTime,
    outcome: SnapshotTerminalTempMaintenanceOutcome,
    terminal_lease_count_before: u32,
    terminal_lease_count_after: u32,
    active_terminal_lease_count_before: u32,
    active_terminal_lease_count_after: u32,
    terminal_charged_bytes_before: u64,
    terminal_charged_bytes_after: u64,
    has_more: bool,
}

impl SnapshotTerminalTempMaintenanceResult {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        observed_at: SystemTime,
        outcome: SnapshotTerminalTempMaintenanceOutcome,
        terminal_lease_count_before: u32,
        terminal_lease_count_after: u32,
        active_terminal_lease_count_before: u32,
        active_terminal_lease_count_after: u32,
        terminal_charged_bytes_before: u64,
        terminal_charged_bytes_after: u64,
        has_more: bool,
    ) -> Self {
        Self {
            observed_at,
            outcome,
            terminal_lease_count_before,
            terminal_lease_count_after,
            active_terminal_lease_count_before,
            active_terminal_lease_count_after,
            terminal_charged_bytes_before,
            terminal_charged_bytes_after,
            has_more,
        }
    }

    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }

    pub const fn outcome(&self) -> SnapshotTerminalTempMaintenanceOutcome {
        self.outcome
    }

    pub const fn terminal_lease_count_before(&self) -> u32 {
        self.terminal_lease_count_before
    }

    pub const fn terminal_lease_count_after(&self) -> u32 {
        self.terminal_lease_count_after
    }

    pub const fn active_terminal_lease_count_before(&self) -> u32 {
        self.active_terminal_lease_count_before
    }

    pub const fn active_terminal_lease_count_after(&self) -> u32 {
        self.active_terminal_lease_count_after
    }

    pub const fn terminal_charged_bytes_before(&self) -> u64 {
        self.terminal_charged_bytes_before
    }

    pub const fn terminal_charged_bytes_after(&self) -> u64 {
        self.terminal_charged_bytes_after
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

/// Path-free outcome of one bounded unleased snapshot-temp reconciliation.
/// The exact temporary-file identity stays private inside persistence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotUnleasedTempMaintenanceOutcome {
    NoUnleasedTemp,
    DeferredActive,
    Removed { bytes: u64 },
}

/// Immutable outcome from one idle-only unleased snapshot-temp batch.
/// `has_more` asks the caller to submit a later task; core never self-enqueues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotUnleasedTempMaintenanceResult {
    observed_at: SystemTime,
    outcome: SnapshotUnleasedTempMaintenanceOutcome,
    unleased_temp_count_before: u32,
    unleased_temp_count_after: u32,
    active_unleased_temp_count_before: u32,
    active_unleased_temp_count_after: u32,
    unleased_charged_bytes_before: u64,
    unleased_charged_bytes_after: u64,
    has_more: bool,
}

impl SnapshotUnleasedTempMaintenanceResult {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        observed_at: SystemTime,
        outcome: SnapshotUnleasedTempMaintenanceOutcome,
        unleased_temp_count_before: u32,
        unleased_temp_count_after: u32,
        active_unleased_temp_count_before: u32,
        active_unleased_temp_count_after: u32,
        unleased_charged_bytes_before: u64,
        unleased_charged_bytes_after: u64,
        has_more: bool,
    ) -> Self {
        Self {
            observed_at,
            outcome,
            unleased_temp_count_before,
            unleased_temp_count_after,
            active_unleased_temp_count_before,
            active_unleased_temp_count_after,
            unleased_charged_bytes_before,
            unleased_charged_bytes_after,
            has_more,
        }
    }

    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }

    pub const fn outcome(&self) -> SnapshotUnleasedTempMaintenanceOutcome {
        self.outcome
    }

    pub const fn unleased_temp_count_before(&self) -> u32 {
        self.unleased_temp_count_before
    }

    pub const fn unleased_temp_count_after(&self) -> u32 {
        self.unleased_temp_count_after
    }

    pub const fn active_unleased_temp_count_before(&self) -> u32 {
        self.active_unleased_temp_count_before
    }

    pub const fn active_unleased_temp_count_after(&self) -> u32 {
        self.active_unleased_temp_count_after
    }

    pub const fn unleased_charged_bytes_before(&self) -> u64 {
        self.unleased_charged_bytes_before
    }

    pub const fn unleased_charged_bytes_after(&self) -> u64 {
        self.unleased_charged_bytes_after
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

impl FormatSizeBatchResult {
    pub(super) fn new(entries: Vec<FormattedSizeEntry>) -> Self {
        Self {
            entries: entries.into(),
        }
    }

    pub fn entries(&self) -> &[FormattedSizeEntry] {
        &self.entries
    }
}

/// Frozen counts from one durable scan summary. Non-successful scans retain
/// zero counts and unknown allocation rather than presenting an unfinalized
/// partial tree as an exact total.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanTaskCounts {
    pub directory_count: u64,
    pub file_count: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
}

/// Durable lifecycle stored for one scan. `Queued` is retained because schema
/// v1 permits historical queued rows even though the current engine persists a
/// scan only when its worker starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DurableScanStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

/// Trustworthy terminal counts from a succeeded durable scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DurableScanCounts {
    pub directory_count: u64,
    pub file_count: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: Option<u64>,
}

/// Path-free summary of a fully validated durable coverage report. Individual
/// issue paths remain behind later paged inspection APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DurableScanCoverage {
    pub status: ScanCoverageStatus,
    pub measured_permille: Option<CoveragePermille>,
    pub issue_record_count: usize,
    pub issue_occurrence_count: u64,
}

/// Path-free durable scan observation. `snapshot_recorded` means SQLite holds
/// a validated immutable snapshot reference; this API does not open that file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableScanSummary {
    pub scan_id: ScanId,
    pub started_at: SystemTime,
    pub completed_at: Option<SystemTime>,
    pub status: DurableScanStatus,
    pub counts: Option<DurableScanCounts>,
    pub coverage: DurableScanCoverage,
    pub snapshot_recorded: bool,
}

/// Newest durable scans in stable start-descending, ID-ascending order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentScanHistory {
    pub scans: Vec<DurableScanSummary>,
    pub has_more: bool,
}

/// Path-free failure taxonomy for the durable recent-history boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ScanHistoryError {
    #[error("scan history limit must be between 1 and {max}")]
    InvalidLimit { max: usize },
    #[error("engine session is closed")]
    Closed,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the scan history query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable scan history is corrupt")]
    CorruptData,
    #[error("durable scan history is unavailable")]
    Unavailable,
    #[error("engine history state is unavailable")]
    InternalState,
}

/// Durable review projection for one scan-bound candidate observation.
/// These values describe history only; none is a current validation, plan, or
/// cleanup capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DurableCandidateStatus {
    Discovered,
    Selected,
    Dismissed,
    Stale,
    Planned,
    Completed,
    Failed,
    Unavailable,
}

/// Durable evaluator state for one exact scan. `NotRun` distinguishes scans
/// that legitimately predate discovery or did not complete successfully from
/// a reserved pending evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DurableCandidateEvaluationStatus {
    NotRun,
    Pending,
    Succeeded {
        candidate_count: u32,
    },
    Failed {
        kind: CandidateEvaluationTaskFailureKind,
    },
}

/// Path-free summary of a fully validated stored candidate. Exact paths and
/// evidence payloads remain behind a future paged inspection boundary; counts
/// and typed reason kinds are sufficient for overview and filtering without
/// making history executable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCandidateSummary {
    id: CandidateId,
    rule: RuleRef,
    category: CandidateCategory,
    estimated_bytes: u64,
    newest_mtime: Option<SystemTime>,
    safety: SafetyTier,
    action: CandidateAction,
    rule_schedule_eligible: bool,
    path_count: u16,
    evidence_kinds: Arc<[EvidenceKind]>,
    blockers: Arc<[BlockReason]>,
    created_at: SystemTime,
    status: DurableCandidateStatus,
}

impl DurableCandidateSummary {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        id: CandidateId,
        rule: RuleRef,
        category: CandidateCategory,
        estimated_bytes: u64,
        newest_mtime: Option<SystemTime>,
        safety: SafetyTier,
        action: CandidateAction,
        rule_schedule_eligible: bool,
        path_count: u16,
        evidence_kinds: Vec<EvidenceKind>,
        blockers: Vec<BlockReason>,
        created_at: SystemTime,
        status: DurableCandidateStatus,
    ) -> Self {
        Self {
            id,
            rule,
            category,
            estimated_bytes,
            newest_mtime,
            safety,
            action,
            rule_schedule_eligible,
            path_count,
            evidence_kinds: evidence_kinds.into(),
            blockers: blockers.into(),
            created_at,
            status,
        }
    }

    pub fn id(&self) -> &CandidateId {
        &self.id
    }

    pub fn rule(&self) -> &RuleRef {
        &self.rule
    }

    pub fn category(&self) -> CandidateCategory {
        self.category
    }

    pub fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }

    pub fn newest_mtime(&self) -> Option<SystemTime> {
        self.newest_mtime
    }

    pub fn safety(&self) -> SafetyTier {
        self.safety
    }

    pub fn action(&self) -> CandidateAction {
        self.action
    }

    pub fn rule_schedule_eligible(&self) -> bool {
        self.rule_schedule_eligible
    }

    pub fn path_count(&self) -> u16 {
        self.path_count
    }

    pub fn evidence_kinds(&self) -> &[EvidenceKind] {
        &self.evidence_kinds
    }

    pub fn blockers(&self) -> &[BlockReason] {
        &self.blockers
    }

    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    pub fn status(&self) -> DurableCandidateStatus {
        self.status
    }
}

/// Exact-scan durable discovery observation. The candidate list is present
/// only for a fully validated successful evaluation and is stable across task
/// eviction and process restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCandidateEvaluation {
    scan_id: ScanId,
    source_scan_status: DurableScanStatus,
    scheduled_at: Option<SystemTime>,
    completed_at: Option<SystemTime>,
    status: DurableCandidateEvaluationStatus,
    candidates: Arc<[DurableCandidateSummary]>,
}

impl DurableCandidateEvaluation {
    pub(super) fn new(
        scan_id: ScanId,
        source_scan_status: DurableScanStatus,
        scheduled_at: Option<SystemTime>,
        completed_at: Option<SystemTime>,
        status: DurableCandidateEvaluationStatus,
        candidates: Vec<DurableCandidateSummary>,
    ) -> Self {
        Self {
            scan_id,
            source_scan_status,
            scheduled_at,
            completed_at,
            status,
            candidates: candidates.into(),
        }
    }

    pub fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    pub fn source_scan_status(&self) -> DurableScanStatus {
        self.source_scan_status
    }

    pub fn scheduled_at(&self) -> Option<SystemTime> {
        self.scheduled_at
    }

    pub fn completed_at(&self) -> Option<SystemTime> {
        self.completed_at
    }

    pub fn status(&self) -> DurableCandidateEvaluationStatus {
        self.status
    }

    pub fn candidates(&self) -> &[DurableCandidateSummary] {
        &self.candidates
    }
}

/// Stable, path-free failure taxonomy for exact-scan candidate history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CandidateHistoryError {
    #[error("engine session is closed")]
    Closed,
    #[error("the requested durable scan does not exist")]
    ScanNotFound,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the candidate history query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable candidate history is corrupt")]
    CorruptData,
    #[error("durable candidate history is unavailable")]
    Unavailable,
    #[error("engine candidate-history state is unavailable")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanTaskStatus {
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

/// Stable, path-free reason why deterministic discovery produced no candidate
/// batch. This never changes the succeeded scan or grants cleanup authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CandidateEvaluationTaskFailureKind {
    Cancelled,
    CatalogInvalid,
    ContextInvalid,
    EvaluationFailed,
    CandidateInvalid,
    LimitExceeded,
}

/// Terminal discovery state attached to a retained scan result. Non-successful
/// traversals never run candidate evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CandidateEvaluationTaskStatus {
    NotRun,
    Succeeded {
        candidate_count: u32,
    },
    Failed {
        kind: CandidateEvaluationTaskFailureKind,
    },
}

/// Worker-private result of one pending-evaluation replay attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CandidateEvaluationRecoveryOutcome {
    NoPending,
    Recovered {
        candidate_count: u32,
        has_more: bool,
    },
    Incompatible {
        has_more: bool,
    },
}

/// Worker-private failures from one pending-evaluation replay attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub(super) enum CandidateEvaluationRecoveryError {
    #[error("the recovery clock is invalid")]
    InvalidClock,
    #[error("the durable engine schema is incompatible")]
    IncompatibleSchema,
    #[error("the durable evaluator state is busy")]
    Busy,
    #[error("the durable evaluator state is unsafe")]
    UnsafeStorage,
    #[error("the pending evaluator state exceeded its fixed budget")]
    BudgetExceeded,
    #[error("the pending evaluator state is corrupt")]
    CorruptData,
    #[error("the pending evaluator state is unavailable")]
    Unavailable,
    #[error("the pending evaluator completion outcome is unknown")]
    OutcomeUnknown,
    #[error("the pending evaluator state is internally inconsistent")]
    InternalState,
}

/// Immutable, non-authoritative durable result for one engine scan task.
/// Paths and tree nodes remain behind the paged snapshot APIs added later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanTaskResult {
    scan_id: ScanId,
    started_at: SystemTime,
    completed_at: SystemTime,
    status: ScanTaskStatus,
    counts: ScanTaskCounts,
    coverage: ScanCoverage,
    snapshot_available: bool,
    candidate_evaluation: CandidateEvaluationTaskStatus,
}

impl ScanTaskResult {
    pub(super) fn without_snapshot(
        scan_id: ScanId,
        started_at: SystemTime,
        completed_at: SystemTime,
        status: ScanTaskStatus,
        counts: ScanTaskCounts,
        coverage: ScanCoverage,
    ) -> Self {
        Self {
            scan_id,
            started_at,
            completed_at,
            status,
            counts,
            coverage,
            snapshot_available: false,
            candidate_evaluation: CandidateEvaluationTaskStatus::NotRun,
        }
    }

    pub(super) fn succeeded(
        scan_id: ScanId,
        started_at: SystemTime,
        completed_at: SystemTime,
        counts: ScanTaskCounts,
        coverage: ScanCoverage,
        candidate_evaluation: CandidateEvaluationTaskStatus,
    ) -> Self {
        Self {
            scan_id,
            started_at,
            completed_at,
            status: ScanTaskStatus::Succeeded,
            counts,
            coverage,
            snapshot_available: true,
            candidate_evaluation,
        }
    }

    pub fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    pub fn started_at(&self) -> SystemTime {
        self.started_at
    }

    pub fn completed_at(&self) -> SystemTime {
        self.completed_at
    }

    pub fn status(&self) -> ScanTaskStatus {
        self.status
    }

    pub fn counts(&self) -> ScanTaskCounts {
        self.counts
    }

    pub fn coverage(&self) -> &ScanCoverage {
        &self.coverage
    }

    pub fn snapshot_available(&self) -> bool {
        self.snapshot_available
    }

    pub fn candidate_evaluation(&self) -> CandidateEvaluationTaskStatus {
        self.candidate_evaluation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineLifecycle {
    Open,
    Closing,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelOutcome {
    CancelledBeforeStart,
    Requested,
    AlreadyRequested,
    AlreadyTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryMaintenanceStartOutcome {
    Started(TaskId),
    AlreadyActive(TaskId),
    DeferredBusy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanRecoveryMaintenanceStartOutcome {
    Started(TaskId),
    AlreadyActive(TaskId),
    DeferredBusy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateEvaluationRecoveryMaintenanceStartOutcome {
    Started(TaskId),
    AlreadyActive(TaskId),
    DeferredBusy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotRetentionStartOutcome {
    Started(TaskId),
    AlreadyActive(TaskId),
    DeferredBusy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotOrphanMaintenanceStartOutcome {
    Started(TaskId),
    AlreadyActive(TaskId),
    DeferredBusy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotProvisioningStageMaintenanceStartOutcome {
    Started(TaskId),
    AlreadyActive(TaskId),
    DeferredBusy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotTerminalTempMaintenanceStartOutcome {
    Started(TaskId),
    AlreadyActive(TaskId),
    DeferredBusy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotUnleasedTempMaintenanceStartOutcome {
    Started(TaskId),
    AlreadyActive(TaskId),
    DeferredBusy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    Initiated,
    AlreadyClosing,
    AlreadyClosed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EngineOpenError {
    #[error("the embedded candidate catalog is invalid")]
    CandidateCatalogInvalid,
    #[error("engine database is unavailable: {0:?}")]
    Database(crate::persistence::DatabaseOpenErrorKind),
    #[error("engine snapshot storage is unavailable: {0:?}")]
    Snapshot(crate::persistence::SnapshotOpenErrorKind),
    #[error("engine worker resources are unavailable")]
    WorkerUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StartTaskError {
    #[error("engine session is closed")]
    Closed,
    #[error("engine task queue is full")]
    QueueFull,
    #[error("task input exceeds the fixed limit of {limit} items")]
    InputTooLarge { limit: u16 },
    #[error("scan root is invalid or unavailable: {reason:?}")]
    InvalidScanRoot { reason: ScanRootErrorKind },
    #[error("a scan for this filesystem object is already active as task {existing:?}")]
    ScanAlreadyActive { existing: TaskId },
    #[error("an overlapping but non-equivalent scan scope is already active")]
    ScanScopeBusy,
    #[error("the durable engine store is read-only")]
    ReadOnlyStore,
    #[error("the durable engine store is unavailable")]
    PersistenceUnavailable,
    #[error("engine task identifiers are exhausted")]
    TaskIdExhausted,
    #[error("engine task registry is unavailable")]
    InternalState,
}

/// Failure to derive and admit a scan from one exact Explorer review node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StartSubtreeScanError {
    #[error("snapshot review belongs to a different engine session")]
    ForeignReview,
    #[error("snapshot review could not produce a current directory target: {0}")]
    Review(#[from] super::snapshot_review::SnapshotReviewError),
    #[error("subtree scan could not be admitted: {0}")]
    Task(#[from] StartTaskError),
}

/// Path-free scan-root validation categories suitable for UI and FFI mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanRootErrorKind {
    InvalidPath,
    Missing,
    AccessDenied,
    NotDirectory,
    Symlink,
    ChangedDuringValidation,
    IdentityUnavailable,
    UnsupportedPlatform,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TaskAccessError {
    #[error("engine session is closed")]
    Closed,
    #[error("unknown or expired task identifier")]
    UnknownTask,
    #[error("event page limit must be between 1 and {max}")]
    InvalidEventLimit { max: u16 },
    #[error("event cursor is ahead of the task event stream")]
    InvalidEventCursor,
    #[error("task has a different result kind")]
    WrongTaskKind,
    #[error("engine task registry is unavailable")]
    InternalState,
}
