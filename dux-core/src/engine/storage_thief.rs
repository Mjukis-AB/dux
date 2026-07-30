//! Path-free presentation boundary for recurring deterministic cleanup rules.
//!
//! These values summarize a bounded history window. They are observations,
//! never current filesystem evidence, cleanup authority, or schedule approval.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::domain::RuleRef;

pub const MAX_STORAGE_THIEF_RANKING_GROUPS: u16 = 12;
pub const MAX_STORAGE_THIEF_RANKING_SOURCE_SESSIONS: u16 = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableStorageThiefRanking {
    permanent_safe_session_count: u16,
    manual_cleanup_session_count: u16,
    ranked_rule_count: u16,
    has_older_permanent_safe_sessions: bool,
    groups: Arc<[DurableStorageThiefGroup]>,
}

impl DurableStorageThiefRanking {
    pub(super) fn new(
        permanent_safe_session_count: u16,
        manual_cleanup_session_count: u16,
        ranked_rule_count: u16,
        has_older_permanent_safe_sessions: bool,
        groups: Vec<DurableStorageThiefGroup>,
    ) -> Self {
        Self {
            permanent_safe_session_count,
            manual_cleanup_session_count,
            ranked_rule_count,
            has_older_permanent_safe_sessions,
            groups: groups.into(),
        }
    }

    pub const fn permanent_safe_session_count(&self) -> u16 {
        self.permanent_safe_session_count
    }

    pub const fn manual_cleanup_session_count(&self) -> u16 {
        self.manual_cleanup_session_count
    }

    pub const fn ranked_rule_count(&self) -> u16 {
        self.ranked_rule_count
    }

    pub const fn has_older_permanent_safe_sessions(&self) -> bool {
        self.has_older_permanent_safe_sessions
    }

    pub fn groups(&self) -> &[DurableStorageThiefGroup] {
        &self.groups
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableStorageThiefGroup {
    latest_rule: RuleRef,
    observed_revision_count: u16,
    successful_cleanup_count: u16,
    successful_manual_cleanup_count: u16,
    observed_regrowth_cycle_count: u16,
    manual_regrowth_cycle_count: u16,
    total_observed_regrown_bytes: u64,
    total_regrowth_duration: Duration,
    bytes_regrown_per_day: u64,
    rate_capped: bool,
    latest_cleanup_at: SystemTime,
    latest_regrowth_at: SystemTime,
    automation_history_threshold_met: bool,
}

impl DurableStorageThiefGroup {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        latest_rule: RuleRef,
        observed_revision_count: u16,
        successful_cleanup_count: u16,
        successful_manual_cleanup_count: u16,
        observed_regrowth_cycle_count: u16,
        manual_regrowth_cycle_count: u16,
        total_observed_regrown_bytes: u64,
        total_regrowth_duration: Duration,
        bytes_regrown_per_day: u64,
        rate_capped: bool,
        latest_cleanup_at: SystemTime,
        latest_regrowth_at: SystemTime,
        automation_history_threshold_met: bool,
    ) -> Self {
        Self {
            latest_rule,
            observed_revision_count,
            successful_cleanup_count,
            successful_manual_cleanup_count,
            observed_regrowth_cycle_count,
            manual_regrowth_cycle_count,
            total_observed_regrown_bytes,
            total_regrowth_duration,
            bytes_regrown_per_day,
            rate_capped,
            latest_cleanup_at,
            latest_regrowth_at,
            automation_history_threshold_met,
        }
    }

    pub const fn latest_rule(&self) -> &RuleRef {
        &self.latest_rule
    }

    pub const fn observed_revision_count(&self) -> u16 {
        self.observed_revision_count
    }

    pub const fn successful_cleanup_count(&self) -> u16 {
        self.successful_cleanup_count
    }

    pub const fn successful_manual_cleanup_count(&self) -> u16 {
        self.successful_manual_cleanup_count
    }

    pub const fn observed_regrowth_cycle_count(&self) -> u16 {
        self.observed_regrowth_cycle_count
    }

    pub const fn manual_regrowth_cycle_count(&self) -> u16 {
        self.manual_regrowth_cycle_count
    }

    pub const fn total_observed_regrown_bytes(&self) -> u64 {
        self.total_observed_regrown_bytes
    }

    pub const fn total_regrowth_duration(&self) -> Duration {
        self.total_regrowth_duration
    }

    pub const fn bytes_regrown_per_day(&self) -> u64 {
        self.bytes_regrown_per_day
    }

    pub const fn rate_capped(&self) -> bool {
        self.rate_capped
    }

    pub const fn latest_cleanup_at(&self) -> SystemTime {
        self.latest_cleanup_at
    }

    pub const fn latest_regrowth_at(&self) -> SystemTime {
        self.latest_regrowth_at
    }

    pub const fn automation_history_threshold_met(&self) -> bool {
        self.automation_history_threshold_met
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StorageThiefError {
    #[error("engine session is closed")]
    Closed,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the storage-thief query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable storage-thief evidence is corrupt")]
    CorruptData,
    #[error("durable storage-thief evidence is unavailable")]
    Unavailable,
    #[error("engine storage-thief state is unavailable")]
    InternalState,
}
