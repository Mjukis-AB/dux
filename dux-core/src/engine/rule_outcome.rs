//! Path-free presentation boundary for derived cleanup-item outcomes.
//!
//! These values summarize historical observations only. They contain no path,
//! candidate, evaluator, snapshot, journal-owner, approval, or effect data.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::domain::RuleRef;

use super::cleanup_history::DurableCleanupSessionId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableRuleOutcomeBatch {
    session_id: DurableCleanupSessionId,
    outcomes: Arc<[DurableRuleOutcome]>,
}

impl DurableRuleOutcomeBatch {
    pub(super) fn new(
        session_id: DurableCleanupSessionId,
        outcomes: Vec<DurableRuleOutcome>,
    ) -> Self {
        Self {
            session_id,
            outcomes: outcomes.into(),
        }
    }

    pub const fn session_id(&self) -> &DurableCleanupSessionId {
        &self.session_id
    }

    pub fn outcomes(&self) -> &[DurableRuleOutcome] {
        &self.outcomes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableRuleOutcome {
    item_ordinal: u16,
    rule: RuleRef,
    state: DurableRuleOutcomeState,
}

impl DurableRuleOutcome {
    pub(super) const fn new(
        item_ordinal: u16,
        rule: RuleRef,
        state: DurableRuleOutcomeState,
    ) -> Self {
        Self {
            item_ordinal,
            rule,
            state,
        }
    }

    pub const fn item_ordinal(&self) -> u16 {
        self.item_ordinal
    }

    pub const fn rule(&self) -> &RuleRef {
        &self.rule
    }

    pub const fn state(&self) -> &DurableRuleOutcomeState {
        &self.state
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DurableRuleOutcomeState {
    NotEligible {
        reason: RuleOutcomeNotEligibleReason,
    },
    AwaitingComparableScan {
        cleaned_at: SystemTime,
    },
    Superseded {
        cleaned_at: SystemTime,
        superseded_at: SystemTime,
    },
    LaterSizeObserved {
        cleaned_at: SystemTime,
        observed_at: SystemTime,
        observed_bytes: u64,
    },
    ZeroBaselineObserved {
        cleaned_at: SystemTime,
        observed_at: SystemTime,
    },
    Regrown {
        cleaned_at: SystemTime,
        zero_observed_at: SystemTime,
        observed_at: SystemTime,
        observed_bytes: u64,
        regrowth_duration: Duration,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuleOutcomeNotEligibleReason {
    SourceCleanupIncomplete,
    ItemNotSuccessfulPermanentRegenerable,
    SourceScanNotComparable,
    SourceEvaluationNotComparable,
    SourceEvaluationAfterPlan,
    SourceCandidateMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RuleOutcomeError {
    #[error("engine session is closed")]
    Closed,
    #[error("the requested cleanup session does not exist")]
    SessionNotFound,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the rule-outcome query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable rule-outcome evidence is corrupt")]
    CorruptData,
    #[error("durable rule-outcome evidence is unavailable")]
    Unavailable,
    #[error("engine rule-outcome state is unavailable")]
    InternalState,
}
