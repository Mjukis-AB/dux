//! Public, path-free view of inert automation schedule drafts.

use std::time::SystemTime;

use crate::domain::{
    AUTOMATION_ELIGIBILITY_POLICY_REVISION, AutomationDraftPolicyReason, AutomationSchedule,
    AutomationScheduleId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationScheduleDraftEligibilityStatus {
    BlockedByStaticPolicy,
    AwaitingRuntimeEvidence,
}

/// Path-free policy preflight bound to one exact schedule revision.
/// `AwaitingRuntimeEvidence` is not an eligible or runnable state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationScheduleDraftEligibilityAssessment {
    schedule_id: AutomationScheduleId,
    schedule_revision: u64,
    policy_revision: u32,
    status: AutomationScheduleDraftEligibilityStatus,
    included_statically_eligible_rule_count: u16,
    reasons: Vec<AutomationDraftPolicyReason>,
}

impl AutomationScheduleDraftEligibilityAssessment {
    pub(super) fn new(
        schedule_id: AutomationScheduleId,
        schedule_revision: u64,
        included_statically_eligible_rule_count: u16,
        reasons: Vec<AutomationDraftPolicyReason>,
    ) -> Self {
        let status = if reasons.is_empty() && included_statically_eligible_rule_count > 0 {
            AutomationScheduleDraftEligibilityStatus::AwaitingRuntimeEvidence
        } else {
            AutomationScheduleDraftEligibilityStatus::BlockedByStaticPolicy
        };
        Self {
            schedule_id,
            schedule_revision,
            policy_revision: AUTOMATION_ELIGIBILITY_POLICY_REVISION,
            status,
            included_statically_eligible_rule_count,
            reasons,
        }
    }

    pub fn schedule_id(&self) -> &AutomationScheduleId {
        &self.schedule_id
    }

    pub const fn schedule_revision(&self) -> u64 {
        self.schedule_revision
    }

    pub const fn policy_revision(&self) -> u32 {
        self.policy_revision
    }

    pub const fn status(&self) -> AutomationScheduleDraftEligibilityStatus {
        self.status
    }

    pub const fn included_statically_eligible_rule_count(&self) -> u16 {
        self.included_statically_eligible_rule_count
    }

    pub fn reasons(&self) -> &[AutomationDraftPolicyReason] {
        &self.reasons
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationGlobalControlSource {
    Default,
    Stored,
}

/// Dedicated default-off master control. It is independent from the permanent
/// cleanup policy and cannot authorize a target or an effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationGlobalControl {
    pub enabled: bool,
    pub source: AutomationGlobalControlSource,
    pub revision: u64,
    pub updated_at: Option<SystemTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationGlobalControlUpdate {
    pub control: AutomationGlobalControl,
    pub changed: bool,
}

/// Read-only automation state. Persisted activation is consent/configuration
/// evidence only; `execution_available` remains a separate fail-closed gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationOverview {
    pub global_control: AutomationGlobalControl,
    pub execution_available: bool,
    pub eligible_rule_count: u16,
    pub schedules: Vec<AutomationSchedule>,
    pub schedule_eligibility: Vec<AutomationScheduleDraftEligibilityAssessment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationScheduleUpdate {
    pub schedule: AutomationSchedule,
    pub changed: bool,
}

/// Source compatibility for the create/replace draft methods during the v65
/// transport migration. The payload is activation-aware.
pub type AutomationScheduleDraftUpdate = AutomationScheduleUpdate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationScheduleDraftDeleteOutcome {
    pub deleted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AutomationScheduleAuthoringCatalogError {
    #[error("engine session is closed")]
    Closed,
    #[error("the automation authoring catalog is unavailable")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AutomationScheduleDraftError {
    #[error("engine session is closed")]
    Closed,
    #[error("the schedule draft input is invalid")]
    InvalidInput,
    #[error("a current automation authoring catalog binding is required")]
    AuthoringCatalogRequired,
    #[error("the automation authoring catalog binding is stale")]
    AuthoringCatalogStale,
    #[error("the automation category selection is invalid")]
    InvalidAuthoringSelection,
    #[error("the schedule draft registry reached its fixed limit")]
    DraftLimitExceeded,
    #[error("the schedule draft was not found")]
    NotFound,
    #[error("the schedule draft changed since it was read")]
    RevisionConflict,
    #[error("the schedule state does not admit the requested transition")]
    InvalidStateTransition,
    #[error("the schedule is blocked by current shipped automation policy")]
    StaticPolicyBlocked,
    #[error("the schedule requires runtime evidence that is not yet available")]
    ActivationUnavailable,
    #[error("the schedule draft revision cannot advance")]
    RevisionExhausted,
    #[error("the system clock cannot be represented by the settings store")]
    InvalidClock,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the schedule draft query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("automation schedule drafts are corrupt")]
    CorruptData,
    #[error("automation schedule drafts are unavailable")]
    Unavailable,
    #[error("the schedule draft write outcome could not be proven")]
    OutcomeUnknown,
    #[error("automation schedule draft state is unavailable")]
    InternalState,
}
