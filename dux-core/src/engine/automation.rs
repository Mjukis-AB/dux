//! Public, path-free view of inert automation schedule drafts.

use crate::domain::{
    AUTOMATION_ELIGIBILITY_POLICY_REVISION, AutomationDraftPolicyReason, AutomationScheduleDraft,
    AutomationScheduleId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationScheduleDraftEligibilityStatus {
    BlockedByStaticPolicy,
    AwaitingRuntimeEvidence,
}

/// Path-free policy preflight bound to one exact disabled draft revision.
/// `AwaitingRuntimeEvidence` is not an eligible or runnable state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationScheduleDraftEligibilityAssessment {
    schedule_id: AutomationScheduleId,
    draft_revision: u64,
    policy_revision: u32,
    status: AutomationScheduleDraftEligibilityStatus,
    included_statically_eligible_rule_count: u16,
    reasons: Vec<AutomationDraftPolicyReason>,
}

impl AutomationScheduleDraftEligibilityAssessment {
    pub(super) fn new(
        schedule_id: AutomationScheduleId,
        draft_revision: u64,
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
            draft_revision,
            policy_revision: AUTOMATION_ELIGIBILITY_POLICY_REVISION,
            status,
            included_statically_eligible_rule_count,
            reasons,
        }
    }

    pub fn schedule_id(&self) -> &AutomationScheduleId {
        &self.schedule_id
    }

    pub const fn draft_revision(&self) -> u64 {
        self.draft_revision
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

/// Read-only automation state. This foundation deliberately reports both
/// gates as false; no API in this slice can change them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationOverview {
    pub global_enabled: bool,
    pub execution_available: bool,
    pub eligible_rule_count: u16,
    pub drafts: Vec<AutomationScheduleDraft>,
    pub draft_eligibility: Vec<AutomationScheduleDraftEligibilityAssessment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationScheduleDraftUpdate {
    pub draft: AutomationScheduleDraft,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationScheduleDraftDeleteOutcome {
    pub deleted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AutomationScheduleDraftError {
    #[error("engine session is closed")]
    Closed,
    #[error("the schedule draft input is invalid")]
    InvalidInput,
    #[error("the schedule draft registry reached its fixed limit")]
    DraftLimitExceeded,
    #[error("the schedule draft was not found")]
    NotFound,
    #[error("the schedule draft changed since it was read")]
    RevisionConflict,
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
