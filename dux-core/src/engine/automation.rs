//! Public, path-free view of inert automation schedule drafts.

use crate::domain::AutomationScheduleDraft;

/// Read-only automation state. This foundation deliberately reports both
/// gates as false; no API in this slice can change them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationOverview {
    pub global_enabled: bool,
    pub execution_available: bool,
    pub eligible_rule_count: u16,
    pub drafts: Vec<AutomationScheduleDraft>,
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
