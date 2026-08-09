//! Path-free, read-only suggestions derived from bounded manual history.
//!
//! Suggestions are observations only. They carry no draft mutation,
//! scheduling approval, filesystem witness, or cleanup authority.

use std::sync::Arc;
use std::time::SystemTime;

use crate::domain::RuleRef;

pub const MAX_AUTOMATION_HISTORY_SUGGESTION_SOURCE_SESSIONS: u16 = 32;
pub const MAX_AUTOMATION_HISTORY_SUGGESTIONS: u16 = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationScheduleSuggestionFeed {
    source_session_count: u16,
    qualifying_rule_count: u16,
    has_older_source_sessions: bool,
    suggestions: Arc<[AutomationScheduleSuggestion]>,
}

impl AutomationScheduleSuggestionFeed {
    pub(super) fn new(
        source_session_count: u16,
        qualifying_rule_count: u16,
        has_older_source_sessions: bool,
        suggestions: Vec<AutomationScheduleSuggestion>,
    ) -> Self {
        Self {
            source_session_count,
            qualifying_rule_count,
            has_older_source_sessions,
            suggestions: suggestions.into(),
        }
    }

    pub const fn source_session_count(&self) -> u16 {
        self.source_session_count
    }

    pub const fn qualifying_rule_count(&self) -> u16 {
        self.qualifying_rule_count
    }

    pub const fn has_older_source_sessions(&self) -> bool {
        self.has_older_source_sessions
    }

    pub fn suggestions(&self) -> &[AutomationScheduleSuggestion] {
        &self.suggestions
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationScheduleSuggestion {
    rule: RuleRef,
    successful_manual_run_count: u16,
    manual_regrowth_cycle_count: u16,
    latest_manual_attempt_at: SystemTime,
    latest_regrowth_at: SystemTime,
}

impl AutomationScheduleSuggestion {
    pub(super) const fn new(
        rule: RuleRef,
        successful_manual_run_count: u16,
        manual_regrowth_cycle_count: u16,
        latest_manual_attempt_at: SystemTime,
        latest_regrowth_at: SystemTime,
    ) -> Self {
        Self {
            rule,
            successful_manual_run_count,
            manual_regrowth_cycle_count,
            latest_manual_attempt_at,
            latest_regrowth_at,
        }
    }

    pub const fn rule(&self) -> &RuleRef {
        &self.rule
    }

    pub const fn successful_manual_run_count(&self) -> u16 {
        self.successful_manual_run_count
    }

    pub const fn manual_regrowth_cycle_count(&self) -> u16 {
        self.manual_regrowth_cycle_count
    }

    pub const fn latest_manual_attempt_at(&self) -> SystemTime {
        self.latest_manual_attempt_at
    }

    pub const fn latest_regrowth_at(&self) -> SystemTime {
        self.latest_regrowth_at
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AutomationScheduleSuggestionError {
    #[error("engine session is closed")]
    Closed,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the automation-history suggestion query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable automation-history evidence is corrupt")]
    CorruptData,
    #[error("durable automation-history evidence is unavailable")]
    Unavailable,
    #[error("engine automation-history state is unavailable")]
    InternalState,
}
