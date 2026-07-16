//! Bounded, path-free presentation types for durable cleanup history.
//!
//! These values are historical observations only. They deliberately omit
//! execution fences, claims, candidate identifiers, paths, and evidence
//! payloads, and cannot be consumed as a cleanup plan or validation witness.

use std::sync::Arc;
use std::time::SystemTime;

use crate::domain::{CandidateAction, CandidateCategory, RuleRef, SafetyTier, ScanId};

pub const MAX_RECENT_CLEANUP_HISTORY_LIMIT: u16 = 64;

const MAX_SESSION_ID_BYTES: usize = 128;
const MAX_PLAN_ID_BYTES: usize = 128;
const MAX_ERROR_CATEGORY_BYTES: usize = 128;
const MAX_SESSION_ITEMS: usize = 64;
const MAX_SESSION_PATHS: u16 = 256;
const MAX_SESSION_EVIDENCE: u16 = 512;
const MAX_SESSION_WARNINGS: usize = 5;

/// An opaque, presentation-only cleanup-session token copied from durable
/// history. Possessing this value grants no cleanup or recovery authority.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DurableCleanupSessionId(Arc<str>);

impl DurableCleanupSessionId {
    pub(super) fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        if !is_bounded_stable_token(&value, MAX_SESSION_ID_BYTES) {
            return None;
        }
        Some(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable keyset position for start-descending, session-ID-ascending history.
/// Its fields are observations, not a database offset or mutable capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupHistoryCursor {
    started_at: SystemTime,
    session_id: DurableCleanupSessionId,
}

impl CleanupHistoryCursor {
    pub(super) const fn new(started_at: SystemTime, session_id: DurableCleanupSessionId) -> Self {
        Self {
            started_at,
            session_id,
        }
    }

    pub const fn started_at(&self) -> SystemTime {
        self.started_at
    }

    pub const fn session_id(&self) -> &DurableCleanupSessionId {
        &self.session_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DurableCleanupRecordFormat {
    /// A migrated summary whose exact plan graph and execution journal were
    /// never stored.
    LegacyIncomplete,
    /// A schema-v2 frozen plan and journal. Summary pages validate its bounded
    /// scalar structure and lifecycle; exact item observations additionally
    /// pass the complete journal decoder.
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DurableCleanupMode {
    DryRun,
    Trash,
    PermanentSafe,
    EvictLocalCopy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DurableCleanupTrigger {
    Manual,
    LowDisk,
    Scheduled,
    Cli,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DurableCleanupSessionStatus {
    Planned,
    Running,
    Recovering,
    Completed,
    PartiallyCompleted,
    Failed,
    Cancelled,
    Interrupted,
    Rejected,
    DryRun,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DurableCleanupItemStatus {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DurableCleanupWarning {
    EstimatedBytesUnverified,
    DryRunDoesNotMutate,
    TrashDoesNotFreeSpaceImmediately,
    PermanentRemovalCannotBeUndone,
    CloudEvictionRequiresNetworkToRedownload,
}

/// Fixed status counts for either the item or path population of one session.
/// Construction counts one observation at a time with checked arithmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DurableCleanupStatusCounts {
    planned: u16,
    validating: u16,
    dry_run: u16,
    effect_started: u16,
    trashed: u16,
    removed: u16,
    evicted: u16,
    skipped: u16,
    rejected: u16,
    failed: u16,
    changed_since_plan: u16,
    interrupted: u16,
    unavailable: u16,
    outcome_unknown: u16,
    total: u16,
}

impl DurableCleanupStatusCounts {
    pub(super) const fn empty() -> Self {
        Self {
            planned: 0,
            validating: 0,
            dry_run: 0,
            effect_started: 0,
            trashed: 0,
            removed: 0,
            evicted: 0,
            skipped: 0,
            rejected: 0,
            failed: 0,
            changed_since_plan: 0,
            interrupted: 0,
            unavailable: 0,
            outcome_unknown: 0,
            total: 0,
        }
    }

    pub(super) fn from_statuses(
        statuses: impl IntoIterator<Item = DurableCleanupItemStatus>,
    ) -> Option<Self> {
        let mut counts = Self::empty();
        for status in statuses {
            counts.total = counts.total.checked_add(1)?;
            if counts.total > MAX_SESSION_PATHS {
                return None;
            }
            let count = match status {
                DurableCleanupItemStatus::Planned => &mut counts.planned,
                DurableCleanupItemStatus::Validating => &mut counts.validating,
                DurableCleanupItemStatus::DryRun => &mut counts.dry_run,
                DurableCleanupItemStatus::EffectStarted => &mut counts.effect_started,
                DurableCleanupItemStatus::Trashed => &mut counts.trashed,
                DurableCleanupItemStatus::Removed => &mut counts.removed,
                DurableCleanupItemStatus::Evicted => &mut counts.evicted,
                DurableCleanupItemStatus::Skipped => &mut counts.skipped,
                DurableCleanupItemStatus::Rejected => &mut counts.rejected,
                DurableCleanupItemStatus::Failed => &mut counts.failed,
                DurableCleanupItemStatus::ChangedSincePlan => &mut counts.changed_since_plan,
                DurableCleanupItemStatus::Interrupted => &mut counts.interrupted,
                DurableCleanupItemStatus::Unavailable => &mut counts.unavailable,
                DurableCleanupItemStatus::OutcomeUnknown => &mut counts.outcome_unknown,
            };
            *count = count.checked_add(1)?;
        }
        Some(counts)
    }

    pub const fn planned(&self) -> u16 {
        self.planned
    }

    pub const fn validating(&self) -> u16 {
        self.validating
    }

    pub const fn dry_run(&self) -> u16 {
        self.dry_run
    }

    pub const fn effect_started(&self) -> u16 {
        self.effect_started
    }

    pub const fn trashed(&self) -> u16 {
        self.trashed
    }

    pub const fn removed(&self) -> u16 {
        self.removed
    }

    pub const fn evicted(&self) -> u16 {
        self.evicted
    }

    pub const fn skipped(&self) -> u16 {
        self.skipped
    }

    pub const fn rejected(&self) -> u16 {
        self.rejected
    }

    pub const fn failed(&self) -> u16 {
        self.failed
    }

    pub const fn changed_since_plan(&self) -> u16 {
        self.changed_since_plan
    }

    pub const fn interrupted(&self) -> u16 {
        self.interrupted
    }

    pub const fn unavailable(&self) -> u16 {
        self.unavailable
    }

    pub const fn outcome_unknown(&self) -> u16 {
        self.outcome_unknown
    }

    pub const fn total(&self) -> u16 {
        self.total
    }
}

/// A bounded stable category, never a raw OS error or arbitrary diagnostic.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DurableCleanupErrorCategory(Arc<str>);

impl DurableCleanupErrorCategory {
    pub(super) fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        if !is_bounded_stable_token(&value, MAX_ERROR_CATEGORY_BYTES) {
            return None;
        }
        Some(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCleanupSessionSummary {
    id: DurableCleanupSessionId,
    plan_id: Arc<str>,
    format: DurableCleanupRecordFormat,
    source_scan_id: Option<ScanId>,
    started_at: SystemTime,
    completed_at: Option<SystemTime>,
    plan_created_at: Option<SystemTime>,
    plan_expires_at: Option<SystemTime>,
    mode: DurableCleanupMode,
    trigger: DurableCleanupTrigger,
    status: DurableCleanupSessionStatus,
    estimated_bytes: u64,
    verified_capacity_delta_bytes: Option<i64>,
    cancellation_requested: Option<bool>,
    item_total: u16,
    path_total: u16,
    evidence_total: u16,
    item_status_counts: DurableCleanupStatusCounts,
    path_status_counts: DurableCleanupStatusCounts,
}

impl DurableCleanupSessionSummary {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        id: DurableCleanupSessionId,
        plan_id: impl Into<String>,
        format: DurableCleanupRecordFormat,
        source_scan_id: Option<ScanId>,
        started_at: SystemTime,
        completed_at: Option<SystemTime>,
        plan_created_at: Option<SystemTime>,
        plan_expires_at: Option<SystemTime>,
        mode: DurableCleanupMode,
        trigger: DurableCleanupTrigger,
        status: DurableCleanupSessionStatus,
        estimated_bytes: u64,
        verified_capacity_delta_bytes: Option<i64>,
        cancellation_requested: Option<bool>,
        item_total: u16,
        path_total: u16,
        evidence_total: u16,
        item_status_counts: DurableCleanupStatusCounts,
        path_status_counts: DurableCleanupStatusCounts,
    ) -> Option<Self> {
        let plan_id = plan_id.into();
        if !is_bounded_stable_token(&plan_id, MAX_PLAN_ID_BYTES)
            || usize::from(item_total) > MAX_SESSION_ITEMS
            || path_total > MAX_SESSION_PATHS
            || evidence_total > MAX_SESSION_EVIDENCE
            || item_status_counts.total() != item_total
            || path_status_counts.total() != path_total
            || !format_fields_are_consistent(
                format,
                source_scan_id.as_ref(),
                plan_created_at,
                plan_expires_at,
            )
        {
            return None;
        }
        Some(Self {
            id,
            plan_id: plan_id.into(),
            format,
            source_scan_id,
            started_at,
            completed_at,
            plan_created_at,
            plan_expires_at,
            mode,
            trigger,
            status,
            estimated_bytes,
            verified_capacity_delta_bytes,
            cancellation_requested,
            item_total,
            path_total,
            evidence_total,
            item_status_counts,
            path_status_counts,
        })
    }

    pub const fn id(&self) -> &DurableCleanupSessionId {
        &self.id
    }

    pub fn plan_id(&self) -> &str {
        &self.plan_id
    }

    pub const fn format(&self) -> DurableCleanupRecordFormat {
        self.format
    }

    pub const fn source_scan_id(&self) -> Option<&ScanId> {
        self.source_scan_id.as_ref()
    }

    pub const fn started_at(&self) -> SystemTime {
        self.started_at
    }

    pub const fn completed_at(&self) -> Option<SystemTime> {
        self.completed_at
    }

    pub const fn plan_created_at(&self) -> Option<SystemTime> {
        self.plan_created_at
    }

    pub const fn plan_expires_at(&self) -> Option<SystemTime> {
        self.plan_expires_at
    }

    pub const fn mode(&self) -> DurableCleanupMode {
        self.mode
    }

    pub const fn trigger(&self) -> DurableCleanupTrigger {
        self.trigger
    }

    pub const fn status(&self) -> DurableCleanupSessionStatus {
        self.status
    }

    pub const fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }

    pub const fn verified_capacity_delta_bytes(&self) -> Option<i64> {
        self.verified_capacity_delta_bytes
    }

    pub const fn cancellation_requested(&self) -> Option<bool> {
        self.cancellation_requested
    }

    pub const fn item_total(&self) -> u16 {
        self.item_total
    }

    pub const fn path_total(&self) -> u16 {
        self.path_total
    }

    pub const fn evidence_total(&self) -> u16 {
        self.evidence_total
    }

    pub const fn item_status_counts(&self) -> &DurableCleanupStatusCounts {
        &self.item_status_counts
    }

    pub const fn path_status_counts(&self) -> &DurableCleanupStatusCounts {
        &self.path_status_counts
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCleanupItemSummary {
    ordinal: u16,
    rule: RuleRef,
    category: Option<CandidateCategory>,
    safety: Option<SafetyTier>,
    action: Option<CandidateAction>,
    rule_schedule_eligible: Option<bool>,
    newest_mtime: Option<SystemTime>,
    estimated_bytes: u64,
    status: DurableCleanupItemStatus,
    error_recorded: bool,
    error_category: Option<DurableCleanupErrorCategory>,
    path_count: u16,
    evidence_count: u16,
}

impl DurableCleanupItemSummary {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        ordinal: u16,
        rule: RuleRef,
        category: Option<CandidateCategory>,
        safety: Option<SafetyTier>,
        action: Option<CandidateAction>,
        rule_schedule_eligible: Option<bool>,
        newest_mtime: Option<SystemTime>,
        estimated_bytes: u64,
        status: DurableCleanupItemStatus,
        error_recorded: bool,
        error_category: Option<DurableCleanupErrorCategory>,
        path_count: u16,
        evidence_count: u16,
    ) -> Option<Self> {
        if usize::from(ordinal) >= MAX_SESSION_ITEMS
            || path_count == 0
            || path_count > MAX_SESSION_PATHS
            || evidence_count > MAX_SESSION_EVIDENCE
            || (!error_recorded && error_category.is_some())
        {
            return None;
        }
        Some(Self {
            ordinal,
            rule,
            category,
            safety,
            action,
            rule_schedule_eligible,
            newest_mtime,
            estimated_bytes,
            status,
            error_recorded,
            error_category,
            path_count,
            evidence_count,
        })
    }

    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }

    pub const fn rule(&self) -> &RuleRef {
        &self.rule
    }

    pub const fn category(&self) -> Option<CandidateCategory> {
        self.category
    }

    pub const fn safety(&self) -> Option<SafetyTier> {
        self.safety
    }

    pub const fn action(&self) -> Option<CandidateAction> {
        self.action
    }

    pub const fn rule_schedule_eligible(&self) -> Option<bool> {
        self.rule_schedule_eligible
    }

    pub const fn newest_mtime(&self) -> Option<SystemTime> {
        self.newest_mtime
    }

    pub const fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }

    pub const fn status(&self) -> DurableCleanupItemStatus {
        self.status
    }

    pub const fn error_recorded(&self) -> bool {
        self.error_recorded
    }

    pub const fn error_category(&self) -> Option<&DurableCleanupErrorCategory> {
        self.error_category.as_ref()
    }

    pub const fn path_count(&self) -> u16 {
        self.path_count
    }

    pub const fn evidence_count(&self) -> u16 {
        self.evidence_count
    }
}

/// One fully validated path-free stored session observation. Completeness is
/// explicit in `summary().format()`; legacy rows never gain fabricated facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCleanupSessionObservation {
    summary: DurableCleanupSessionSummary,
    items: Arc<[DurableCleanupItemSummary]>,
    warnings: Arc<[DurableCleanupWarning]>,
}

impl DurableCleanupSessionObservation {
    pub(super) fn new(
        summary: DurableCleanupSessionSummary,
        items: Vec<DurableCleanupItemSummary>,
        warnings: Vec<DurableCleanupWarning>,
    ) -> Option<Self> {
        if items.len() > MAX_SESSION_ITEMS
            || items.len() != usize::from(summary.item_total())
            || warnings.len() > MAX_SESSION_WARNINGS
            || has_duplicate_warnings(&warnings)
            || !items
                .iter()
                .enumerate()
                .all(|(ordinal, item)| usize::from(item.ordinal()) == ordinal)
        {
            return None;
        }

        let path_total = items
            .iter()
            .try_fold(0_u16, |total, item| total.checked_add(item.path_count()))?;
        let evidence_total = items.iter().try_fold(0_u16, |total, item| {
            total.checked_add(item.evidence_count())
        })?;
        let item_counts = DurableCleanupStatusCounts::from_statuses(
            items.iter().map(DurableCleanupItemSummary::status),
        )?;
        if path_total != summary.path_total()
            || evidence_total != summary.evidence_total()
            || item_counts != *summary.item_status_counts()
        {
            return None;
        }

        Some(Self {
            summary,
            items: items.into(),
            warnings: warnings.into(),
        })
    }

    pub const fn summary(&self) -> &DurableCleanupSessionSummary {
        &self.summary
    }

    pub fn items(&self) -> &[DurableCleanupItemSummary] {
        &self.items
    }

    pub fn warnings(&self) -> &[DurableCleanupWarning] {
        &self.warnings
    }
}

/// Summary-only recent feed. Selected records pass a bounded scalar structure
/// and lifecycle preflight without materializing stored path/evidence payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCleanupHistoryPage {
    records: Arc<[DurableCleanupSessionSummary]>,
    next_cursor: Option<CleanupHistoryCursor>,
}

impl DurableCleanupHistoryPage {
    pub(super) fn new(
        records: Vec<DurableCleanupSessionSummary>,
        next_cursor: Option<CleanupHistoryCursor>,
    ) -> Option<Self> {
        if records.len() > usize::from(MAX_RECENT_CLEANUP_HISTORY_LIMIT)
            || (records.is_empty() && next_cursor.is_some())
        {
            return None;
        }
        Some(Self {
            records: records.into(),
            next_cursor,
        })
    }

    pub fn records(&self) -> &[DurableCleanupSessionSummary] {
        &self.records
    }

    pub const fn next_cursor(&self) -> Option<&CleanupHistoryCursor> {
        self.next_cursor.as_ref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CleanupHistoryError {
    #[error("engine session is closed")]
    Closed,
    #[error("cleanup history limit must be between 1 and {maximum}")]
    InvalidLimit { maximum: u16 },
    #[error("the requested cleanup session does not exist")]
    SessionNotFound,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the cleanup history query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable cleanup history is corrupt")]
    CorruptData,
    #[error("durable cleanup history is unavailable")]
    Unavailable,
    #[error("engine cleanup-history state is unavailable")]
    InternalState,
}

fn is_bounded_stable_token(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn format_fields_are_consistent(
    format: DurableCleanupRecordFormat,
    source_scan_id: Option<&ScanId>,
    plan_created_at: Option<SystemTime>,
    plan_expires_at: Option<SystemTime>,
) -> bool {
    match format {
        DurableCleanupRecordFormat::LegacyIncomplete => {
            source_scan_id.is_none() && plan_created_at.is_none() && plan_expires_at.is_none()
        }
        DurableCleanupRecordFormat::Complete => {
            source_scan_id.is_some() && plan_created_at.is_some() && plan_expires_at.is_some()
        }
    }
}

fn has_duplicate_warnings(warnings: &[DurableCleanupWarning]) -> bool {
    warnings
        .iter()
        .enumerate()
        .any(|(index, warning)| warnings[..index].contains(warning))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_session_ids_are_bounded_stable_ascii_tokens() {
        let accepted = DurableCleanupSessionId::new("session:2026-07-17_a.b-C9").unwrap();
        assert_eq!(accepted.as_str(), "session:2026-07-17_a.b-C9");
        assert!(DurableCleanupSessionId::new("x".repeat(128)).is_some());

        for rejected in [
            String::new(),
            "x".repeat(129),
            "session/one".to_owned(),
            "session one".to_owned(),
            "session:räv".to_owned(),
        ] {
            assert!(
                DurableCleanupSessionId::new(rejected.clone()).is_none(),
                "accepted {rejected:?}"
            );
        }
    }

    #[test]
    fn status_counts_cover_every_frozen_item_and_path_status() {
        let statuses = [
            DurableCleanupItemStatus::Planned,
            DurableCleanupItemStatus::Validating,
            DurableCleanupItemStatus::DryRun,
            DurableCleanupItemStatus::EffectStarted,
            DurableCleanupItemStatus::Trashed,
            DurableCleanupItemStatus::Removed,
            DurableCleanupItemStatus::Evicted,
            DurableCleanupItemStatus::Skipped,
            DurableCleanupItemStatus::Rejected,
            DurableCleanupItemStatus::Failed,
            DurableCleanupItemStatus::ChangedSincePlan,
            DurableCleanupItemStatus::Interrupted,
            DurableCleanupItemStatus::Unavailable,
            DurableCleanupItemStatus::OutcomeUnknown,
        ];
        let counts = DurableCleanupStatusCounts::from_statuses(statuses).unwrap();

        assert_eq!(counts.total(), 14);
        assert_eq!(counts.planned(), 1);
        assert_eq!(counts.validating(), 1);
        assert_eq!(counts.dry_run(), 1);
        assert_eq!(counts.effect_started(), 1);
        assert_eq!(counts.trashed(), 1);
        assert_eq!(counts.removed(), 1);
        assert_eq!(counts.evicted(), 1);
        assert_eq!(counts.skipped(), 1);
        assert_eq!(counts.rejected(), 1);
        assert_eq!(counts.failed(), 1);
        assert_eq!(counts.changed_since_plan(), 1);
        assert_eq!(counts.interrupted(), 1);
        assert_eq!(counts.unavailable(), 1);
        assert_eq!(counts.outcome_unknown(), 1);
    }

    #[test]
    fn status_counts_reject_more_than_the_frozen_path_limit() {
        assert!(
            DurableCleanupStatusCounts::from_statuses(std::iter::repeat_n(
                DurableCleanupItemStatus::Planned,
                usize::from(MAX_SESSION_PATHS) + 1,
            ))
            .is_none()
        );
    }
}
