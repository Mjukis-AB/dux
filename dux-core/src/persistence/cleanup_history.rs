//! Typed frozen cleanup-plan history stored in SQLite schema v2.
//!
//! These records are review and recovery observations only. Decoded paths are
//! not current validation witnesses, and neither a complete record nor a
//! legacy summary can authorize a filesystem effect.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::domain::{
    CLEANUP_PLAN_VALIDITY, CandidateAction, CandidateCategory, CandidateId, CleanupMode,
    CleanupPlan, CleanupPlanId, Evidence, PlanWarning, RuleId, RuleRef, RuleRevision, SafetyTier,
    ScanId,
};

use super::candidate_history::{
    CandidateHistoryStatus, CandidatePriorReviewStatus, CompleteCandidateRecord, PreparedEvidence,
    RawEvidence, StoredCandidateRecord, TimeParts, action_as_stored, action_from_stored,
    category_as_stored, category_from_stored, decode_absolute_path, decode_evidence,
    decode_optional_time, from_i64, load_candidate_record_within_budget, mark_candidate_planned,
    mark_trusted_rust_target_candidate_planned, safety_as_stored, safety_from_stored, stored_bool,
    time_parts, to_i64, validate_complete_children, validate_null_value, validate_optional_value,
    validate_policy, validate_required_value,
};
use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    system_time_to_unix_ms, unix_ms_to_system_time,
};

const MAX_ITEMS: usize = 64;
const MAX_TOTAL_PATHS: usize = 256;
const MAX_TOTAL_EVIDENCE: usize = 512;
const MAX_WARNINGS: usize = 5;
const MAX_ID_BYTES: i64 = 128;
const MAX_LEGACY_ERROR_CHARACTERS: i64 = 128;
const MAX_LEGACY_ERROR_BYTES: i64 = MAX_LEGACY_ERROR_CHARACTERS * 4;
const MAX_POLICY_BYTES: i64 = 64;
const MAX_PATH_BYTES: i64 = 65_536;
const MAX_TEXT_BYTES: i64 = 4_096;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct CleanupSessionId(String);

impl CleanupSessionId {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, HistoryError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_ID_BYTES as usize
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':')
            })
        {
            return Err(invalid());
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CleanupTrigger {
    Manual,
    LowDisk,
    Scheduled,
    Cli,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NewCleanupSessionRecord {
    session_id: CleanupSessionId,
    plan: CleanupPlan,
    started_at: SystemTime,
    trigger: CleanupTrigger,
    candidate_status_coupling: CandidateStatusCoupling,
}

impl NewCleanupSessionRecord {
    pub(crate) fn try_from_plan(
        session_id: CleanupSessionId,
        plan: &CleanupPlan,
        started_at: SystemTime,
        trigger: CleanupTrigger,
    ) -> Result<Self, HistoryError> {
        if !(plan.created_at() <= started_at && started_at < plan.expires_at()) {
            return Err(invalid());
        }
        let started_at = canonical_started_at(started_at)?;
        validate_plan(plan, started_at, trigger, HistoryErrorKind::InvalidInput)?;
        Ok(Self {
            session_id,
            plan: plan.clone(),
            started_at,
            trigger,
            candidate_status_coupling: CandidateStatusCoupling::PlanClaimsV1,
        })
    }

    pub(crate) fn try_from_uncoupled_plan(
        session_id: CleanupSessionId,
        plan: &CleanupPlan,
        started_at: SystemTime,
        trigger: CleanupTrigger,
    ) -> Result<Self, HistoryError> {
        let mut record = Self::try_from_plan(session_id, plan, started_at, trigger)?;
        record.candidate_status_coupling = CandidateStatusCoupling::LegacyUncoupled;
        Ok(record)
    }

    pub(crate) fn try_from_trusted_rust_target_plan(
        session_id: CleanupSessionId,
        plan: &CleanupPlan,
        started_at: SystemTime,
        trigger: CleanupTrigger,
    ) -> Result<Self, HistoryError> {
        let mut record = Self::try_from_plan(session_id, plan, started_at, trigger)?;
        let [item] = plan.items() else {
            return Err(invalid());
        };
        if plan.mode() != CleanupMode::PermanentSafe
            || item.rule().id().as_str() != "developer.rust.target"
            || item.rule().revision().get() != 2
            || item.safety() != SafetyTier::SafeRegenerable
            || item.action() != CandidateAction::RemoveKnownRegenerableContents
            || item.rule_marks_schedule_eligible()
        {
            return Err(invalid());
        }
        record.candidate_status_coupling = CandidateStatusCoupling::TrustedRustTargetPlanClaimsV1;
        Ok(record)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StoredCleanupSessionRecord {
    LegacySummary(LegacyCleanupSessionSummary),
    Planned(PlannedCleanupSessionRecord),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LegacyCleanupSessionSummary {
    pub(crate) session_id: CleanupSessionId,
    pub(crate) plan_id: CleanupPlanId,
    pub(crate) started_at: SystemTime,
    pub(crate) completed_at: Option<SystemTime>,
    pub(crate) mode: CleanupMode,
    pub(crate) estimated_bytes: u64,
    pub(crate) verified_capacity_delta_bytes: Option<i64>,
    pub(crate) trigger: CleanupTrigger,
    pub(crate) status: LegacySessionStatus,
    pub(crate) items: Vec<LegacyCleanupItemSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LegacyCleanupItemSummary {
    pub(crate) ordinal: usize,
    pub(crate) rule: RuleRef,
    pub(crate) target_path: PathBuf,
    pub(crate) estimated_bytes: u64,
    pub(crate) status: LegacyItemStatus,
    pub(crate) error_category: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegacySessionStatus {
    Planned,
    Running,
    Completed,
    PartiallyCompleted,
    Failed,
    Cancelled,
    Interrupted,
    Rejected,
    DryRun,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LegacyItemStatus {
    Planned,
    DryRun,
    Trashed,
    Removed,
    Evicted,
    Skipped,
    Rejected,
    Failed,
    ChangedSincePlan,
    Interrupted,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlannedCleanupSessionRecord {
    pub(crate) session_id: CleanupSessionId,
    pub(crate) plan_id: CleanupPlanId,
    pub(crate) started_at: SystemTime,
    pub(crate) source_scan_id: ScanId,
    pub(crate) plan_created_at: SystemTime,
    pub(crate) plan_expires_at: SystemTime,
    pub(crate) mode: CleanupMode,
    pub(crate) estimated_bytes: u64,
    pub(crate) trigger: CleanupTrigger,
    pub(super) candidate_status_coupling: CandidateStatusCoupling,
    pub(crate) items: Vec<PlannedCleanupItemRecord>,
    pub(crate) warnings: Vec<PlanWarning>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlannedCleanupItemRecord {
    pub(crate) ordinal: usize,
    pub(crate) candidate_id: CandidateId,
    pub(crate) rule: RuleRef,
    pub(crate) category: CandidateCategory,
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) estimated_bytes: u64,
    pub(crate) newest_mtime: Option<SystemTime>,
    pub(crate) evidence: Vec<Evidence>,
    pub(crate) safety: SafetyTier,
    pub(crate) proposed_action: CandidateAction,
    pub(crate) rule_schedule_eligible: bool,
    pub(super) prior_review_status: Option<CandidatePriorReviewStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CandidateStatusCoupling {
    LegacyUncoupled,
    PlanClaimsV1,
    /// An insertion-only capability. It persists as `PlanClaimsV1`; only the
    /// private trusted Rust-target constructor can mint this in-memory variant.
    TrustedRustTargetPlanClaimsV1,
}

impl CandidateStatusCoupling {
    pub(super) fn as_i64(self) -> i64 {
        match self {
            Self::LegacyUncoupled => 1,
            Self::PlanClaimsV1 | Self::TrustedRustTargetPlanClaimsV1 => 2,
        }
    }

    pub(super) fn claims_candidates(self) -> bool {
        matches!(
            self,
            Self::PlanClaimsV1 | Self::TrustedRustTargetPlanClaimsV1
        )
    }

    fn allows_trusted_rust_target_blocker(self) -> bool {
        matches!(self, Self::TrustedRustTargetPlanClaimsV1)
    }
}

pub(super) struct PreparedCleanupSession {
    session_id: String,
    plan_id: String,
    started_at_unix_ms: i64,
    source_scan_id: String,
    plan_created_at: TimeParts,
    plan_expires_at: TimeParts,
    mode: &'static str,
    estimated_bytes: i64,
    trigger: &'static str,
    items: Vec<PreparedCleanupItem>,
    warnings: Vec<&'static str>,
    candidate_status_coupling: CandidateStatusCoupling,
}

struct PreparedCleanupItem {
    candidate_id: String,
    rule_id: String,
    rule_revision: i64,
    category: &'static str,
    paths: Vec<EncodedBytes>,
    estimated_bytes: i64,
    newest_mtime: Option<TimeParts>,
    evidence: Vec<PreparedEvidence>,
    safety: &'static str,
    proposed_action: &'static str,
    rule_schedule_eligible: bool,
}

impl PreparedCleanupSession {
    pub(super) fn prepare(record: &NewCleanupSessionRecord) -> Result<Self, HistoryError> {
        validate_plan(
            &record.plan,
            record.started_at,
            record.trigger,
            HistoryErrorKind::InvalidInput,
        )?;
        let items = record
            .plan
            .items()
            .iter()
            .map(|item| {
                Ok(PreparedCleanupItem {
                    candidate_id: item.candidate_id().as_str().to_owned(),
                    rule_id: item.rule().id().as_str().to_owned(),
                    rule_revision: i64::from(item.rule().revision().get()),
                    category: category_as_stored(item.category()),
                    paths: item
                        .paths()
                        .iter()
                        .map(|path| encode_host_path(path).map_err(|_| invalid()))
                        .collect::<Result<Vec<_>, _>>()?,
                    estimated_bytes: to_i64(
                        item.estimated_bytes(),
                        HistoryErrorKind::InvalidInput,
                    )?,
                    newest_mtime: item
                        .newest_mtime()
                        .map(|value| time_parts(value, HistoryErrorKind::InvalidInput))
                        .transpose()?,
                    evidence: item
                        .evidence()
                        .iter()
                        .map(PreparedEvidence::prepare)
                        .collect::<Result<Vec<_>, _>>()?,
                    safety: safety_as_stored(item.safety()),
                    proposed_action: action_as_stored(item.action()),
                    rule_schedule_eligible: item.rule_marks_schedule_eligible(),
                })
            })
            .collect::<Result<Vec<_>, HistoryError>>()?;
        Ok(Self {
            session_id: record.session_id.as_str().to_owned(),
            plan_id: record.plan.id().as_str().to_owned(),
            started_at_unix_ms: system_time_to_unix_ms(
                record.started_at,
                HistoryErrorKind::InvalidInput,
            )?,
            source_scan_id: record.plan.source_scan_id().as_str().to_owned(),
            plan_created_at: time_parts(record.plan.created_at(), HistoryErrorKind::InvalidInput)?,
            plan_expires_at: time_parts(record.plan.expires_at(), HistoryErrorKind::InvalidInput)?,
            mode: mode_as_stored(record.plan.mode()),
            estimated_bytes: to_i64(
                record.plan.estimated_bytes(),
                HistoryErrorKind::InvalidInput,
            )?,
            trigger: trigger_as_stored(record.trigger),
            items,
            warnings: record
                .plan
                .warnings()
                .iter()
                .map(warning_as_stored)
                .collect(),
            candidate_status_coupling: record.candidate_status_coupling,
        })
    }
}

pub(super) fn insert_cleanup_session(
    transaction: &Transaction<'_>,
    session: &PreparedCleanupSession,
) -> Result<(), HistoryError> {
    let conflicts: i64 = transaction
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM cleanup_sessions
                 WHERE session_id = ?1 OR plan_id = ?2
             )",
            params![session.session_id, session.plan_id],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    if conflicts != 0 {
        return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
    }
    let candidates = if session.candidate_status_coupling.claims_candidates() {
        ensure_dependencies_match(transaction, session)?
    } else {
        Vec::new()
    };
    let changed = transaction
        .execute(
            "INSERT INTO cleanup_sessions (
                 session_id, plan_id, started_at_unix_ms, mode, estimated_bytes,
                 trigger_source, status, record_format_version, source_scan_id,
                 plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                 plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                 cancellation_requested, candidate_status_coupling_version
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, 'planned', 2, ?7,
                 ?8, ?9, ?10, ?11, 0, ?12
             ) ON CONFLICT DO NOTHING",
            params![
                session.session_id,
                session.plan_id,
                session.started_at_unix_ms,
                session.mode,
                session.estimated_bytes,
                session.trigger,
                session.source_scan_id,
                session.plan_created_at.seconds,
                session.plan_created_at.nanoseconds,
                session.plan_expires_at.seconds,
                session.plan_expires_at.nanoseconds,
                session.candidate_status_coupling.as_i64(),
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
    }

    for (item_ordinal, item) in session.items.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO cleanup_items (
                     session_id, item_ordinal, rule_id, rule_revision, estimated_bytes,
                     final_status, record_format_version, candidate_id, category,
                     safety_tier, proposed_action, rule_schedule_eligible,
                     newest_mtime_unix_seconds, newest_mtime_nanoseconds
                 ) VALUES (
                     ?1, ?2, ?3, ?4, ?5, 'planned', 2, ?6, ?7, ?8, ?9, ?10, ?11, ?12
                 )",
                params![
                    session.session_id,
                    item_ordinal as i64,
                    item.rule_id,
                    item.rule_revision,
                    item.estimated_bytes,
                    item.candidate_id,
                    item.category,
                    item.safety,
                    item.proposed_action,
                    item.rule_schedule_eligible,
                    item.newest_mtime.map(|value| value.seconds),
                    item.newest_mtime.map(|value| value.nanoseconds),
                ],
            )
            .map_err(map_write_sql_error)?;
        for (path_ordinal, path) in item.paths.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO cleanup_item_paths (
                         session_id, item_ordinal, path_ordinal, target_path,
                         target_path_encoding, status
                     ) VALUES (?1, ?2, ?3, ?4, ?5, 'planned')",
                    params![
                        session.session_id,
                        item_ordinal as i64,
                        path_ordinal as i64,
                        path.bytes,
                        path.encoding as i64,
                    ],
                )
                .map_err(map_write_sql_error)?;
        }
        for (evidence_ordinal, evidence) in item.evidence.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO cleanup_item_evidence (
                         session_id, item_ordinal, evidence_ordinal, evidence_kind,
                         path_value, path_value_encoding, text_value,
                         observed_unix_seconds, observed_nanoseconds,
                         duration_seconds, duration_nanoseconds, observed_bytes, minimum_bytes
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        session.session_id,
                        item_ordinal as i64,
                        evidence_ordinal as i64,
                        evidence.kind,
                        evidence.path.as_ref().map(|value| value.bytes.as_slice()),
                        evidence.path.as_ref().map(|value| value.encoding as i64),
                        evidence.text,
                        evidence.observed_time.map(|value| value.seconds),
                        evidence.observed_time.map(|value| value.nanoseconds),
                        evidence.duration.map(|value| value.seconds),
                        evidence.duration.map(|value| value.nanoseconds),
                        evidence.observed_bytes,
                        evidence.minimum_bytes,
                    ],
                )
                .map_err(map_write_sql_error)?;
        }
    }
    for (warning_ordinal, warning) in session.warnings.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO cleanup_plan_warnings (
                     session_id, warning_ordinal, warning_kind
                 ) VALUES (?1, ?2, ?3)",
                params![session.session_id, warning_ordinal as i64, warning],
            )
            .map_err(map_write_sql_error)?;
    }
    for (item_ordinal, candidate) in candidates.iter().enumerate() {
        let prior = if session
            .candidate_status_coupling
            .allows_trusted_rust_target_blocker()
            && is_trusted_rust_target_candidate(candidate)
        {
            mark_trusted_rust_target_candidate_planned(transaction, candidate)?
        } else {
            mark_candidate_planned(transaction, candidate)?
        };
        let changed = transaction
            .execute(
                "INSERT INTO candidate_plan_claims (
                     candidate_id, session_id, item_ordinal, prior_review_status
                 ) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT DO NOTHING",
                params![
                    candidate.id.as_str(),
                    session.session_id,
                    item_ordinal as i64,
                    prior.as_stored(),
                ],
            )
            .map_err(map_write_sql_error)?;
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
    }
    Ok(())
}

fn ensure_dependencies_match(
    connection: &Connection,
    session: &PreparedCleanupSession,
) -> Result<Vec<CompleteCandidateRecord>, HistoryError> {
    let scan_exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM scans WHERE scan_id = ?1",
            [&session.source_scan_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    if scan_exists.is_none() {
        return Err(HistoryError::new(HistoryErrorKind::NotFound));
    }
    let mut candidates = Vec::with_capacity(session.items.len());
    for item in &session.items {
        let id = CandidateId::new(item.candidate_id.clone()).map_err(|_| invalid())?;
        let Some(StoredCandidateRecord::Complete(candidate)) =
            load_candidate_record_within_budget(connection, &id)?
        else {
            return Err(HistoryError::new(HistoryErrorKind::NotFound));
        };
        if !prepared_item_matches_candidate(
            item,
            &candidate,
            &session.source_scan_id,
            session.candidate_status_coupling,
        )? {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        candidates.push(candidate);
    }
    Ok(candidates)
}

fn is_trusted_rust_target_candidate(candidate: &CompleteCandidateRecord) -> bool {
    candidate.rule.id().as_str() == "developer.rust.target"
        && candidate.rule.revision().get() == 2
        && candidate.category == CandidateCategory::DeveloperArtifact
        && candidate.safety == SafetyTier::SafeRegenerable
        && candidate.action == CandidateAction::RemoveKnownRegenerableContents
        && !candidate.rule_schedule_eligible
        && candidate.blockers == [crate::domain::BlockReason::ProtectedPath]
}

fn prepared_item_matches_candidate(
    item: &PreparedCleanupItem,
    candidate: &CompleteCandidateRecord,
    source_scan_id: &str,
    coupling: CandidateStatusCoupling,
) -> Result<bool, HistoryError> {
    let paths = candidate
        .paths
        .iter()
        .map(|path| encode_host_path(path).map_err(|_| corrupt()))
        .collect::<Result<Vec<_>, _>>()?;
    let evidence = candidate
        .evidence
        .iter()
        .map(PreparedEvidence::prepare)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(candidate.source_scan_id.as_str() == source_scan_id
        && candidate.id.as_str() == item.candidate_id
        && candidate.rule.id().as_str() == item.rule_id
        && i64::from(candidate.rule.revision().get()) == item.rule_revision
        && category_as_stored(candidate.category) == item.category
        && paths == item.paths
        && to_i64(candidate.estimated_bytes, HistoryErrorKind::CorruptData)?
            == item.estimated_bytes
        && candidate
            .newest_mtime
            .map(|value| time_parts(value, HistoryErrorKind::CorruptData))
            .transpose()?
            == item.newest_mtime
        && evidence == item.evidence
        && safety_as_stored(candidate.safety) == item.safety
        && action_as_stored(candidate.action) == item.proposed_action
        && candidate.rule_schedule_eligible == item.rule_schedule_eligible
        && (candidate.blockers.is_empty()
            || (coupling.allows_trusted_rust_target_blocker()
                && is_trusted_rust_target_candidate(candidate)))
        && matches!(
            candidate.status,
            CandidateHistoryStatus::Discovered | CandidateHistoryStatus::Selected
        ))
}

pub(super) fn load_cleanup_session_record(
    connection: &Connection,
    id: &CleanupSessionId,
) -> Result<Option<StoredCleanupSessionRecord>, HistoryError> {
    run_bounded_query(connection, || load_within_budget(connection, id))
}

pub(super) fn load_within_budget(
    connection: &Connection,
    id: &CleanupSessionId,
) -> Result<Option<StoredCleanupSessionRecord>, HistoryError> {
    let raw = connection
        .query_row(
            "SELECT record_format_version,
                    typeof(session_id), length(CAST(session_id AS BLOB)), session_id,
                    typeof(plan_id), length(CAST(plan_id AS BLOB)), plan_id,
                    started_at_unix_ms, completed_at_unix_ms,
                    typeof(mode), length(CAST(mode AS BLOB)), mode,
                    estimated_bytes, verified_capacity_delta_bytes,
                    typeof(trigger_source), length(CAST(trigger_source AS BLOB)), trigger_source,
                    typeof(status), length(CAST(status AS BLOB)), status,
                    typeof(source_scan_id), length(CAST(source_scan_id AS BLOB)), source_scan_id,
                    plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                    plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                    typeof(execution_owner_id), length(CAST(execution_owner_id AS BLOB)),
                    execution_owner_id, execution_generation, last_heartbeat_at_unix_ms,
                    cancellation_requested, candidate_status_coupling_version
             FROM cleanup_sessions WHERE session_id = ?1",
            [id.as_str()],
            raw_session_row,
        )
        .optional()
        .map_err(map_query_sql_error)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let common = decode_session_common(&raw)?;
    match raw.record_format_version {
        1 => Ok(Some(StoredCleanupSessionRecord::LegacySummary(
            decode_legacy_session(connection, common, &raw)?,
        ))),
        2 => Ok(Some(StoredCleanupSessionRecord::Planned(
            decode_planned_session(connection, common, &raw)?,
        ))),
        _ => Err(corrupt()),
    }
}

struct RawSessionRow {
    record_format_version: i64,
    session_id: String,
    plan_id: String,
    started_at_unix_ms: i64,
    completed_at_unix_ms: Option<i64>,
    mode: String,
    estimated_bytes: i64,
    verified_capacity_delta_bytes: Option<i64>,
    trigger: String,
    status: String,
    source_scan_id: Option<String>,
    plan_created_seconds: Option<i64>,
    plan_created_nanoseconds: Option<i64>,
    plan_expires_seconds: Option<i64>,
    plan_expires_nanoseconds: Option<i64>,
    execution_owner_id: Option<String>,
    execution_generation: Option<i64>,
    last_heartbeat_unix_ms: Option<i64>,
    cancellation_requested: Option<i64>,
    candidate_status_coupling_version: i64,
}

fn raw_session_row(row: &Row<'_>) -> rusqlite::Result<RawSessionRow> {
    validate_required_value(row, 1, 2, "text", MAX_ID_BYTES)?;
    validate_required_value(row, 4, 5, "text", MAX_ID_BYTES)?;
    validate_required_value(row, 9, 10, "text", MAX_POLICY_BYTES)?;
    validate_required_value(row, 14, 15, "text", MAX_POLICY_BYTES)?;
    validate_required_value(row, 17, 18, "text", MAX_POLICY_BYTES)?;
    let version: i64 = row.get(0)?;
    match version {
        1 => {
            validate_optional_value(row, 20, 21, "text", MAX_ID_BYTES)?;
            validate_optional_value(row, 27, 28, "text", MAX_ID_BYTES)?;
        }
        2 => {
            validate_required_value(row, 20, 21, "text", MAX_ID_BYTES)?;
            validate_optional_value(row, 27, 28, "text", MAX_ID_BYTES)?;
        }
        _ => return Err(rusqlite::Error::InvalidQuery),
    }
    Ok(RawSessionRow {
        record_format_version: version,
        session_id: row.get(3)?,
        plan_id: row.get(6)?,
        started_at_unix_ms: row.get(7)?,
        completed_at_unix_ms: row.get(8)?,
        mode: row.get(11)?,
        estimated_bytes: row.get(12)?,
        verified_capacity_delta_bytes: row.get(13)?,
        trigger: row.get(16)?,
        status: row.get(19)?,
        source_scan_id: row.get(22)?,
        plan_created_seconds: row.get(23)?,
        plan_created_nanoseconds: row.get(24)?,
        plan_expires_seconds: row.get(25)?,
        plan_expires_nanoseconds: row.get(26)?,
        execution_owner_id: row.get(29)?,
        execution_generation: row.get(30)?,
        last_heartbeat_unix_ms: row.get(31)?,
        cancellation_requested: row.get(32)?,
        candidate_status_coupling_version: row.get(33)?,
    })
}

struct DecodedSessionCommon {
    session_id: CleanupSessionId,
    plan_id: CleanupPlanId,
    started_at: SystemTime,
    mode: CleanupMode,
    estimated_bytes: u64,
    trigger: CleanupTrigger,
}

fn decode_session_common(raw: &RawSessionRow) -> Result<DecodedSessionCommon, HistoryError> {
    Ok(DecodedSessionCommon {
        session_id: CleanupSessionId::new(raw.session_id.clone()).map_err(|_| corrupt())?,
        plan_id: CleanupPlanId::new(raw.plan_id.clone()).map_err(|_| corrupt())?,
        started_at: unix_ms_to_system_time(raw.started_at_unix_ms)?,
        mode: mode_from_stored(&raw.mode)?,
        estimated_bytes: from_i64(raw.estimated_bytes)?,
        trigger: trigger_from_stored(&raw.trigger)?,
    })
}

fn decode_legacy_session(
    connection: &Connection,
    common: DecodedSessionCommon,
    raw: &RawSessionRow,
) -> Result<LegacyCleanupSessionSummary, HistoryError> {
    if raw.source_scan_id.is_some()
        || raw.plan_created_seconds.is_some()
        || raw.plan_created_nanoseconds.is_some()
        || raw.plan_expires_seconds.is_some()
        || raw.plan_expires_nanoseconds.is_some()
        || raw.execution_owner_id.is_some()
        || raw.execution_generation.is_some()
        || raw.last_heartbeat_unix_ms.is_some()
        || raw.cancellation_requested.is_some()
        || raw.candidate_status_coupling_version != 1
    {
        return Err(corrupt());
    }
    ensure_no_v2_cleanup_children(connection, &common.session_id)?;
    let completed_at = raw
        .completed_at_unix_ms
        .map(unix_ms_to_system_time)
        .transpose()?;
    if completed_at.is_some_and(|value| value < common.started_at) {
        return Err(corrupt());
    }
    let items = load_legacy_items(connection, &common.session_id)?;
    Ok(LegacyCleanupSessionSummary {
        session_id: common.session_id,
        plan_id: common.plan_id,
        started_at: common.started_at,
        completed_at,
        mode: common.mode,
        estimated_bytes: common.estimated_bytes,
        verified_capacity_delta_bytes: raw.verified_capacity_delta_bytes,
        trigger: common.trigger,
        status: session_status_from_stored(&raw.status)?,
        items,
    })
}

fn decode_planned_session(
    connection: &Connection,
    common: DecodedSessionCommon,
    raw: &RawSessionRow,
) -> Result<PlannedCleanupSessionRecord, HistoryError> {
    if raw.status != "planned"
        || raw.completed_at_unix_ms.is_some()
        || raw.verified_capacity_delta_bytes.is_some()
        || raw.execution_owner_id.is_some()
        || raw.execution_generation.is_some()
        || raw.last_heartbeat_unix_ms.is_some()
        || raw.cancellation_requested != Some(0)
    {
        return Err(corrupt());
    }
    decode_frozen_session(connection, common, raw, true, true)
}

/// Decode the immutable format-2 plan facts without interpreting mutable
/// journal columns. The journal decoder calls this inside its own bounded
/// query, then validates every session/item/path lifecycle column separately.
/// Candidate retention must not make crash recovery impossible, so only the
/// pristine planned reader requires the current candidate rows to match.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "execution-state journal integration is crate-private until the executor slice"
    )
)]
pub(super) fn load_frozen_cleanup_session_within_budget(
    connection: &Connection,
    id: &CleanupSessionId,
    require_candidate_match: bool,
) -> Result<Option<PlannedCleanupSessionRecord>, HistoryError> {
    let raw = connection
        .query_row(
            "SELECT record_format_version,
                    typeof(session_id), length(CAST(session_id AS BLOB)), session_id,
                    typeof(plan_id), length(CAST(plan_id AS BLOB)), plan_id,
                    started_at_unix_ms, completed_at_unix_ms,
                    typeof(mode), length(CAST(mode AS BLOB)), mode,
                    estimated_bytes, verified_capacity_delta_bytes,
                    typeof(trigger_source), length(CAST(trigger_source AS BLOB)), trigger_source,
                    typeof(status), length(CAST(status AS BLOB)), status,
                    typeof(source_scan_id), length(CAST(source_scan_id AS BLOB)), source_scan_id,
                    plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                    plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                    typeof(execution_owner_id), length(CAST(execution_owner_id AS BLOB)),
                    execution_owner_id, execution_generation, last_heartbeat_at_unix_ms,
                    cancellation_requested, candidate_status_coupling_version
             FROM cleanup_sessions WHERE session_id = ?1",
            [id.as_str()],
            raw_session_row,
        )
        .optional()
        .map_err(map_query_sql_error)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.record_format_version != 2 {
        return Err(corrupt());
    }
    let common = decode_session_common(&raw)?;
    decode_frozen_session(connection, common, &raw, false, require_candidate_match).map(Some)
}

fn decode_frozen_session(
    connection: &Connection,
    common: DecodedSessionCommon,
    raw: &RawSessionRow,
    require_pristine_children: bool,
    require_candidate_match: bool,
) -> Result<PlannedCleanupSessionRecord, HistoryError> {
    let candidate_status_coupling = match raw.candidate_status_coupling_version {
        1 => CandidateStatusCoupling::LegacyUncoupled,
        2 => CandidateStatusCoupling::PlanClaimsV1,
        _ => return Err(corrupt()),
    };
    let source_scan_id =
        ScanId::new(raw.source_scan_id.clone().ok_or_else(corrupt)?).map_err(|_| corrupt())?;
    let plan_created_at =
        decode_optional_time(raw.plan_created_seconds, raw.plan_created_nanoseconds)?
            .ok_or_else(corrupt)?;
    let plan_expires_at =
        decode_optional_time(raw.plan_expires_seconds, raw.plan_expires_nanoseconds)?
            .ok_or_else(corrupt)?;
    if !(plan_created_at <= common.started_at && common.started_at < plan_expires_at) {
        return Err(corrupt());
    }
    if plan_created_at.checked_add(CLEANUP_PLAN_VALIDITY) != Some(plan_expires_at) {
        return Err(corrupt());
    }
    let scan_exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM scans WHERE scan_id = ?1",
            [source_scan_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    if scan_exists.is_none() {
        return Err(corrupt());
    }
    let mut items = load_v2_items(connection, &common.session_id, require_pristine_children)?;
    load_candidate_plan_claims(
        connection,
        &common.session_id,
        &raw.status,
        candidate_status_coupling,
        &mut items,
    )?;
    if require_candidate_match && candidate_status_coupling.claims_candidates() {
        for item in &items {
            ensure_loaded_item_matches_candidate(connection, item, &source_scan_id)?;
        }
    }
    if items
        .iter()
        .any(|item| !mode_accepts(common.mode, item.safety, item.proposed_action))
        || (common.trigger == CleanupTrigger::Scheduled
            && (common.mode != CleanupMode::PermanentSafe
                || items.iter().any(|item| !item.rule_schedule_eligible)))
        || cleanup_paths_overlap(&items)
    {
        return Err(corrupt());
    }
    let estimated_bytes = items.iter().try_fold(0_u64, |sum, item| {
        sum.checked_add(item.estimated_bytes).ok_or_else(corrupt)
    })?;
    if estimated_bytes != common.estimated_bytes {
        return Err(corrupt());
    }
    let warnings = load_warnings(connection, &common.session_id)?;
    if warnings != derive_warnings(common.mode, &items) {
        return Err(corrupt());
    }
    Ok(PlannedCleanupSessionRecord {
        session_id: common.session_id,
        plan_id: common.plan_id,
        started_at: common.started_at,
        source_scan_id,
        plan_created_at,
        plan_expires_at,
        mode: common.mode,
        estimated_bytes: common.estimated_bytes,
        trigger: common.trigger,
        candidate_status_coupling,
        items,
        warnings,
    })
}

fn load_legacy_items(
    connection: &Connection,
    session_id: &CleanupSessionId,
) -> Result<Vec<LegacyCleanupItemSummary>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT item_ordinal,
                    typeof(rule_id), length(CAST(rule_id AS BLOB)), rule_id, rule_revision,
                    estimated_bytes,
                    typeof(final_status), length(CAST(final_status AS BLOB)), final_status,
                    typeof(error_category), length(CAST(error_category AS BLOB)),
                    length(error_category), error_category,
                    record_format_version,
                    typeof(legacy_target_path), length(legacy_target_path), legacy_target_path,
                    legacy_target_path_encoding,
                    candidate_id, category, safety_tier, proposed_action,
                    rule_schedule_eligible, newest_mtime_unix_seconds,
                    newest_mtime_nanoseconds
             FROM cleanup_items WHERE session_id = ?1
             ORDER BY item_ordinal LIMIT 65",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([session_id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut items = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if items.len() >= MAX_ITEMS {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != items.len() as i64 {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "text", MAX_ID_BYTES).map_err(map_query_sql_error)?;
        validate_required_value(row, 6, 7, "text", MAX_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 9, 10, "text", MAX_LEGACY_ERROR_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 14, 15, "blob", MAX_PATH_BYTES)
            .map_err(map_query_sql_error)?;
        let error_characters: Option<i64> = row.get(11).map_err(map_query_sql_error)?;
        let error_category: Option<String> = row.get(12).map_err(map_query_sql_error)?;
        if error_category.is_none() != error_characters.is_none()
            || error_characters
                .is_some_and(|value| !(1..=MAX_LEGACY_ERROR_CHARACTERS).contains(&value))
        {
            return Err(corrupt());
        }
        let version: i64 = row.get(13).map_err(map_query_sql_error)?;
        let candidate_id: Option<String> = row.get(18).map_err(map_query_sql_error)?;
        let category: Option<String> = row.get(19).map_err(map_query_sql_error)?;
        let safety: Option<String> = row.get(20).map_err(map_query_sql_error)?;
        let action: Option<String> = row.get(21).map_err(map_query_sql_error)?;
        let schedule: Option<i64> = row.get(22).map_err(map_query_sql_error)?;
        let newest_seconds: Option<i64> = row.get(23).map_err(map_query_sql_error)?;
        let newest_nanos: Option<i64> = row.get(24).map_err(map_query_sql_error)?;
        if version != 1
            || candidate_id.is_some()
            || category.is_some()
            || safety.is_some()
            || action.is_some()
            || schedule.is_some()
            || newest_seconds.is_some()
            || newest_nanos.is_some()
        {
            return Err(corrupt());
        }
        let rule_id: String = row.get(3).map_err(map_query_sql_error)?;
        let revision: i64 = row.get(4).map_err(map_query_sql_error)?;
        let path = decode_legacy_path(
            row.get(16).map_err(map_query_sql_error)?,
            row.get(17).map_err(map_query_sql_error)?,
        )?;
        items.push(LegacyCleanupItemSummary {
            ordinal: items.len(),
            rule: decode_rule(rule_id, revision)?,
            target_path: path,
            estimated_bytes: from_i64(row.get(5).map_err(map_query_sql_error)?)?,
            status: item_status_from_stored(
                &row.get::<_, String>(8).map_err(map_query_sql_error)?,
            )?,
            error_category,
        });
    }
    Ok(items)
}

fn ensure_no_v2_cleanup_children(
    connection: &Connection,
    session_id: &CleanupSessionId,
) -> Result<(), HistoryError> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM cleanup_item_paths WHERE session_id = ?1
             UNION ALL SELECT 1 FROM cleanup_item_evidence WHERE session_id = ?1
             UNION ALL SELECT 1 FROM cleanup_plan_warnings WHERE session_id = ?1
             UNION ALL SELECT 1 FROM candidate_plan_claims WHERE session_id = ?1
             LIMIT 1",
            [session_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    if exists.is_some() {
        return Err(corrupt());
    }
    Ok(())
}

fn load_v2_items(
    connection: &Connection,
    session_id: &CleanupSessionId,
    require_pristine: bool,
) -> Result<Vec<PlannedCleanupItemRecord>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT item_ordinal,
                    typeof(rule_id), length(CAST(rule_id AS BLOB)), rule_id, rule_revision,
                    estimated_bytes,
                    typeof(final_status), length(CAST(final_status AS BLOB)), final_status,
                    typeof(error_category), length(CAST(error_category AS BLOB)), error_category,
                    record_format_version,
                    typeof(legacy_target_path), length(legacy_target_path), legacy_target_path,
                    legacy_target_path_encoding,
                    typeof(candidate_id), length(CAST(candidate_id AS BLOB)), candidate_id,
                    typeof(category), length(CAST(category AS BLOB)), category,
                    typeof(safety_tier), length(CAST(safety_tier AS BLOB)), safety_tier,
                    typeof(proposed_action), length(CAST(proposed_action AS BLOB)), proposed_action,
                    rule_schedule_eligible, newest_mtime_unix_seconds,
                    newest_mtime_nanoseconds
             FROM cleanup_items WHERE session_id = ?1
             ORDER BY item_ordinal LIMIT 65",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([session_id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut items = Vec::new();
    let mut total_paths = 0_usize;
    let mut total_evidence = 0_usize;
    let mut candidate_ids = HashSet::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if items.len() >= MAX_ITEMS {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != items.len() as i64 {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "text", MAX_ID_BYTES).map_err(map_query_sql_error)?;
        validate_required_value(row, 6, 7, "text", MAX_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 9, 10, "text", MAX_ID_BYTES).map_err(map_query_sql_error)?;
        validate_null_value(row, 13, 14).map_err(map_query_sql_error)?;
        validate_required_value(row, 17, 18, "text", MAX_ID_BYTES).map_err(map_query_sql_error)?;
        validate_required_value(row, 20, 21, "text", MAX_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 23, 24, "text", MAX_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 26, 27, "text", MAX_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        let version: i64 = row.get(12).map_err(map_query_sql_error)?;
        let status: String = row.get(8).map_err(map_query_sql_error)?;
        let error: Option<String> = row.get(11).map_err(map_query_sql_error)?;
        let legacy_path: Option<Vec<u8>> = row.get(15).map_err(map_query_sql_error)?;
        let legacy_encoding: Option<i64> = row.get(16).map_err(map_query_sql_error)?;
        if version != 2
            || (require_pristine && (status != "planned" || error.is_some()))
            || legacy_path.is_some()
            || legacy_encoding.is_some()
        {
            return Err(corrupt());
        }
        let candidate_id = CandidateId::new(row.get::<_, String>(19).map_err(map_query_sql_error)?)
            .map_err(|_| corrupt())?;
        if !candidate_ids.insert(candidate_id.clone()) {
            return Err(corrupt());
        }
        let rule = decode_rule(
            row.get(3).map_err(map_query_sql_error)?,
            row.get(4).map_err(map_query_sql_error)?,
        )?;
        let category =
            category_from_stored(&row.get::<_, String>(22).map_err(map_query_sql_error)?)?;
        let safety = safety_from_stored(&row.get::<_, String>(25).map_err(map_query_sql_error)?)?;
        let proposed_action =
            action_from_stored(&row.get::<_, String>(28).map_err(map_query_sql_error)?)?;
        let schedule = stored_bool(row.get(29).map_err(map_query_sql_error)?)?;
        validate_policy(safety, proposed_action, schedule, corrupt)?;
        let paths = load_v2_paths(
            connection,
            session_id,
            items.len(),
            &mut total_paths,
            require_pristine,
        )?;
        let evidence =
            load_item_evidence(connection, session_id, items.len(), &mut total_evidence)?;
        validate_complete_children(safety, proposed_action, &paths, &evidence)?;
        let item = PlannedCleanupItemRecord {
            ordinal: items.len(),
            candidate_id,
            rule,
            category,
            paths,
            estimated_bytes: from_i64(row.get(5).map_err(map_query_sql_error)?)?,
            newest_mtime: decode_optional_time(
                row.get(30).map_err(map_query_sql_error)?,
                row.get(31).map_err(map_query_sql_error)?,
            )?,
            evidence,
            safety,
            proposed_action,
            rule_schedule_eligible: schedule,
            prior_review_status: None,
        };
        items.push(item);
    }
    if items.is_empty() {
        return Err(corrupt());
    }
    ensure_no_orphan_v2_children(connection, session_id)?;
    Ok(items)
}

fn load_candidate_plan_claims(
    connection: &Connection,
    session_id: &CleanupSessionId,
    session_status: &str,
    coupling: CandidateStatusCoupling,
    items: &mut [PlannedCleanupItemRecord],
) -> Result<(), HistoryError> {
    let claims_required = coupling.claims_candidates()
        && matches!(session_status, "planned" | "running" | "recovering");
    let mut statement = connection
        .prepare(
            "SELECT claim.item_ordinal,
                    typeof(claim.candidate_id),
                    length(CAST(claim.candidate_id AS BLOB)), claim.candidate_id,
                    typeof(claim.prior_review_status),
                    length(CAST(claim.prior_review_status AS BLOB)),
                    claim.prior_review_status,
                    typeof(candidate.status), length(CAST(candidate.status AS BLOB)),
                    candidate.status, candidate.record_format_version
             FROM candidate_plan_claims AS claim
             LEFT JOIN candidates AS candidate ON candidate.candidate_id = claim.candidate_id
             WHERE claim.session_id = ?1
             ORDER BY claim.item_ordinal
             LIMIT 65",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([session_id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut count = 0_usize;
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if !claims_required || count >= items.len() || count >= MAX_ITEMS {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != count as i64 || items[count].ordinal != count {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "text", MAX_ID_BYTES).map_err(map_query_sql_error)?;
        validate_required_value(row, 4, 5, "text", MAX_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 7, 8, "text", MAX_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        let candidate_id: String = row.get(3).map_err(map_query_sql_error)?;
        let prior: String = row.get(6).map_err(map_query_sql_error)?;
        let candidate_status: String = row.get(9).map_err(map_query_sql_error)?;
        let candidate_version: Option<i64> = row.get(10).map_err(map_query_sql_error)?;
        if candidate_id != items[count].candidate_id.as_str()
            || candidate_status != "planned"
            || candidate_version != Some(2)
        {
            return Err(corrupt());
        }
        items[count].prior_review_status = Some(CandidatePriorReviewStatus::from_stored(&prior)?);
        count += 1;
    }
    if claims_required && count != items.len() {
        return Err(corrupt());
    }
    Ok(())
}

fn ensure_no_orphan_v2_children(
    connection: &Connection,
    session_id: &CleanupSessionId,
) -> Result<(), HistoryError> {
    let orphan: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM cleanup_item_paths AS child
             WHERE child.session_id = ?1 AND NOT EXISTS (
                 SELECT 1 FROM cleanup_items AS parent
                 WHERE parent.session_id = child.session_id
                   AND parent.item_ordinal = child.item_ordinal
                   AND parent.record_format_version = 2
             )
             UNION ALL
             SELECT 1 FROM cleanup_item_evidence AS child
             WHERE child.session_id = ?1 AND NOT EXISTS (
                 SELECT 1 FROM cleanup_items AS parent
                 WHERE parent.session_id = child.session_id
                   AND parent.item_ordinal = child.item_ordinal
                   AND parent.record_format_version = 2
             )
             LIMIT 1",
            [session_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    if orphan.is_some() {
        return Err(corrupt());
    }
    Ok(())
}

fn load_v2_paths(
    connection: &Connection,
    session_id: &CleanupSessionId,
    item_ordinal: usize,
    total: &mut usize,
    require_pristine: bool,
) -> Result<Vec<PathBuf>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT path_ordinal, typeof(target_path), length(target_path), target_path,
                    target_path_encoding, attempt_generation,
                    typeof(status), length(CAST(status AS BLOB)), status,
                    typeof(error_category), length(CAST(error_category AS BLOB)), error_category,
                    effect_started_at_unix_ms, completed_at_unix_ms
             FROM cleanup_item_paths
             WHERE session_id = ?1 AND item_ordinal = ?2
             ORDER BY path_ordinal LIMIT 257",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![session_id.as_str(), item_ordinal as i64])
        .map_err(map_query_sql_error)?;
    let mut paths = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if paths.len() >= 256 || *total >= MAX_TOTAL_PATHS {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != paths.len() as i64 {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "blob", MAX_PATH_BYTES).map_err(map_query_sql_error)?;
        validate_required_value(row, 6, 7, "text", MAX_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 9, 10, "text", MAX_ID_BYTES).map_err(map_query_sql_error)?;
        let attempt: Option<i64> = row.get(5).map_err(map_query_sql_error)?;
        let status: String = row.get(8).map_err(map_query_sql_error)?;
        let error: Option<String> = row.get(11).map_err(map_query_sql_error)?;
        let effect: Option<i64> = row.get(12).map_err(map_query_sql_error)?;
        let completed: Option<i64> = row.get(13).map_err(map_query_sql_error)?;
        if require_pristine
            && (attempt.is_some()
                || status != "planned"
                || error.is_some()
                || effect.is_some()
                || completed.is_some())
        {
            return Err(corrupt());
        }
        let path = decode_absolute_path(
            row.get(3).map_err(map_query_sql_error)?,
            row.get(4).map_err(map_query_sql_error)?,
        )?;
        if paths.contains(&path) {
            return Err(corrupt());
        }
        paths.push(path);
        *total += 1;
    }
    if paths.is_empty() {
        return Err(corrupt());
    }
    Ok(paths)
}

fn load_item_evidence(
    connection: &Connection,
    session_id: &CleanupSessionId,
    item_ordinal: usize,
    total: &mut usize,
) -> Result<Vec<Evidence>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT evidence_ordinal,
                    typeof(evidence_kind), length(CAST(evidence_kind AS BLOB)), evidence_kind,
                    typeof(path_value), length(path_value), path_value, path_value_encoding,
                    typeof(text_value), length(CAST(text_value AS BLOB)), text_value,
                    observed_unix_seconds, observed_nanoseconds,
                    duration_seconds, duration_nanoseconds, observed_bytes, minimum_bytes
             FROM cleanup_item_evidence
             WHERE session_id = ?1 AND item_ordinal = ?2
             ORDER BY evidence_ordinal LIMIT 513",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![session_id.as_str(), item_ordinal as i64])
        .map_err(map_query_sql_error)?;
    let mut evidence = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if evidence.len() >= 512 || *total >= MAX_TOTAL_EVIDENCE {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != evidence.len() as i64 {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "text", MAX_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 4, 5, "blob", MAX_PATH_BYTES).map_err(map_query_sql_error)?;
        validate_optional_value(row, 8, 9, "text", MAX_TEXT_BYTES).map_err(map_query_sql_error)?;
        evidence.push(decode_evidence(RawEvidence {
            kind: row.get(3).map_err(map_query_sql_error)?,
            path: row.get(6).map_err(map_query_sql_error)?,
            path_encoding: row.get(7).map_err(map_query_sql_error)?,
            text: row.get(10).map_err(map_query_sql_error)?,
            observed_seconds: row.get(11).map_err(map_query_sql_error)?,
            observed_nanoseconds: row.get(12).map_err(map_query_sql_error)?,
            duration_seconds: row.get(13).map_err(map_query_sql_error)?,
            duration_nanoseconds: row.get(14).map_err(map_query_sql_error)?,
            observed_bytes: row.get(15).map_err(map_query_sql_error)?,
            minimum_bytes: row.get(16).map_err(map_query_sql_error)?,
        })?);
        *total += 1;
    }
    if evidence.is_empty() {
        return Err(corrupt());
    }
    Ok(evidence)
}

fn ensure_loaded_item_matches_candidate(
    connection: &Connection,
    item: &PlannedCleanupItemRecord,
    source_scan_id: &ScanId,
) -> Result<(), HistoryError> {
    let Some(StoredCandidateRecord::Complete(candidate)) =
        load_candidate_record_within_budget(connection, &item.candidate_id)?
    else {
        return Err(corrupt());
    };
    if candidate.source_scan_id != *source_scan_id
        || candidate.id != item.candidate_id
        || candidate.rule != item.rule
        || candidate.category != item.category
        || candidate.paths != item.paths
        || candidate.estimated_bytes != item.estimated_bytes
        || candidate.newest_mtime != item.newest_mtime
        || candidate.evidence != item.evidence
        || candidate.safety != item.safety
        || candidate.action != item.proposed_action
        || candidate.rule_schedule_eligible != item.rule_schedule_eligible
        || (!candidate.blockers.is_empty() && !is_trusted_rust_target_candidate(&candidate))
    {
        return Err(corrupt());
    }
    Ok(())
}

fn load_warnings(
    connection: &Connection,
    session_id: &CleanupSessionId,
) -> Result<Vec<PlanWarning>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT warning_ordinal, typeof(warning_kind),
                    length(CAST(warning_kind AS BLOB)), warning_kind
             FROM cleanup_plan_warnings WHERE session_id = ?1
             ORDER BY warning_ordinal LIMIT 6",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([session_id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut warnings = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if warnings.len() >= MAX_WARNINGS {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != warnings.len() as i64 {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "text", MAX_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        let warning = warning_from_stored(&row.get::<_, String>(3).map_err(map_query_sql_error)?)?;
        if warnings.contains(&warning) {
            return Err(corrupt());
        }
        warnings.push(warning);
    }
    Ok(warnings)
}

fn validate_plan(
    plan: &CleanupPlan,
    started_at: SystemTime,
    trigger: CleanupTrigger,
    kind: HistoryErrorKind,
) -> Result<(), HistoryError> {
    if plan.items().is_empty() || plan.items().len() > MAX_ITEMS {
        return Err(HistoryError::new(kind));
    }
    if !(plan.created_at() <= started_at && started_at < plan.expires_at()) {
        return Err(HistoryError::new(kind));
    }
    if trigger == CleanupTrigger::Scheduled && plan.mode() != CleanupMode::PermanentSafe {
        return Err(HistoryError::new(kind));
    }
    time_parts(plan.created_at(), kind)?;
    time_parts(plan.expires_at(), kind)?;
    system_time_to_unix_ms(started_at, kind)?;
    let mut total_paths = 0_usize;
    let mut total_evidence = 0_usize;
    let mut estimated_bytes = 0_u64;
    let mut candidate_ids = HashSet::new();
    for item in plan.items() {
        if !candidate_ids.insert(item.candidate_id())
            || item.paths().is_empty()
            || item.evidence().is_empty()
            || !mode_accepts(plan.mode(), item.safety(), item.action())
        {
            return Err(HistoryError::new(kind));
        }
        if trigger == CleanupTrigger::Scheduled && !item.rule_marks_schedule_eligible() {
            return Err(HistoryError::new(kind));
        }
        total_paths = total_paths
            .checked_add(item.paths().len())
            .ok_or_else(|| HistoryError::new(kind))?;
        total_evidence = total_evidence
            .checked_add(item.evidence().len())
            .ok_or_else(|| HistoryError::new(kind))?;
        if total_paths > MAX_TOTAL_PATHS || total_evidence > MAX_TOTAL_EVIDENCE {
            return Err(HistoryError::new(kind));
        }
        validate_policy(
            item.safety(),
            item.action(),
            item.rule_marks_schedule_eligible(),
            || HistoryError::new(kind),
        )?;
        validate_complete_children(item.safety(), item.action(), item.paths(), item.evidence())
            .map_err(|_| HistoryError::new(kind))?;
        estimated_bytes = estimated_bytes
            .checked_add(item.estimated_bytes())
            .ok_or_else(|| HistoryError::new(kind))?;
        for path in item.paths() {
            if !path.is_absolute() || encode_host_path(path).is_err() {
                return Err(HistoryError::new(kind));
            }
        }
        for evidence in item.evidence() {
            PreparedEvidence::prepare(evidence)?;
        }
    }
    if estimated_bytes != plan.estimated_bytes()
        || plan.warnings().len() > MAX_WARNINGS
        || plan.warnings() != derive_warnings_from_plan(plan).as_slice()
    {
        return Err(HistoryError::new(kind));
    }
    to_i64(estimated_bytes, kind)?;
    Ok(())
}

fn derive_warnings_from_plan(plan: &CleanupPlan) -> Vec<PlanWarning> {
    let items = plan
        .items()
        .iter()
        .enumerate()
        .map(|(ordinal, item)| PlannedCleanupItemRecord {
            ordinal,
            candidate_id: item.candidate_id().clone(),
            rule: item.rule().clone(),
            category: item.category(),
            paths: item.paths().to_vec(),
            estimated_bytes: item.estimated_bytes(),
            newest_mtime: item.newest_mtime(),
            evidence: item.evidence().to_vec(),
            safety: item.safety(),
            proposed_action: item.action(),
            rule_schedule_eligible: item.rule_marks_schedule_eligible(),
            prior_review_status: None,
        })
        .collect::<Vec<_>>();
    derive_warnings(plan.mode(), &items)
}

fn derive_warnings(mode: CleanupMode, items: &[PlannedCleanupItemRecord]) -> Vec<PlanWarning> {
    let dry_run = mode == CleanupMode::DryRun;
    let has_action = |action| items.iter().any(|item| item.proposed_action == action);
    let mut warnings = vec![PlanWarning::EstimatedBytesUnverified];
    if dry_run {
        warnings.push(PlanWarning::DryRunDoesNotMutate);
    }
    if mode == CleanupMode::Trash || (dry_run && has_action(CandidateAction::MoveToTrash)) {
        warnings.push(PlanWarning::TrashDoesNotFreeSpaceImmediately);
    }
    if mode == CleanupMode::PermanentSafe
        || (dry_run && has_action(CandidateAction::RemoveKnownRegenerableContents))
    {
        warnings.push(PlanWarning::PermanentRemovalCannotBeUndone);
    }
    if mode == CleanupMode::EvictLocalCopy
        || (dry_run && has_action(CandidateAction::EvictLocalCopy))
    {
        warnings.push(PlanWarning::CloudEvictionRequiresNetworkToRedownload);
    }
    warnings
}

fn mode_accepts(mode: CleanupMode, safety: SafetyTier, action: CandidateAction) -> bool {
    match mode {
        CleanupMode::DryRun => action.is_cleanup_operation(),
        CleanupMode::Trash => {
            safety == SafetyTier::ReviewRequired && action == CandidateAction::MoveToTrash
        }
        CleanupMode::PermanentSafe => {
            safety == SafetyTier::SafeRegenerable
                && action == CandidateAction::RemoveKnownRegenerableContents
        }
        CleanupMode::EvictLocalCopy => {
            safety == SafetyTier::SafeEvictable && action == CandidateAction::EvictLocalCopy
        }
    }
}

fn cleanup_paths_overlap(items: &[PlannedCleanupItemRecord]) -> bool {
    let mut paths = items
        .iter()
        .flat_map(|item| item.paths.iter().map(PathBuf::as_path))
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths.windows(2).any(|pair| {
        pair[0] == pair[1] || pair[0].starts_with(pair[1]) || pair[1].starts_with(pair[0])
    })
}

fn decode_legacy_path(bytes: Vec<u8>, encoding: i64) -> Result<PathBuf, HistoryError> {
    let encoding = match encoding {
        1 => StoredEncoding::Utf8HostPath,
        2 => StoredEncoding::Utf16LeHostPath,
        _ => return Err(corrupt()),
    };
    decode_host_path(&EncodedBytes { bytes, encoding }).map_err(|_| corrupt())
}

fn decode_rule(id: String, revision: i64) -> Result<RuleRef, HistoryError> {
    let id = RuleId::new(id).map_err(|_| corrupt())?;
    let revision = u32::try_from(revision).map_err(|_| corrupt())?;
    Ok(RuleRef::new(
        id,
        RuleRevision::new(revision).map_err(|_| corrupt())?,
    ))
}

fn mode_as_stored(value: CleanupMode) -> &'static str {
    match value {
        CleanupMode::DryRun => "dry_run",
        CleanupMode::Trash => "trash",
        CleanupMode::PermanentSafe => "permanent_safe",
        CleanupMode::EvictLocalCopy => "evict_local_copy",
    }
}

fn mode_from_stored(value: &str) -> Result<CleanupMode, HistoryError> {
    match value {
        "dry_run" => Ok(CleanupMode::DryRun),
        "trash" => Ok(CleanupMode::Trash),
        "permanent_safe" => Ok(CleanupMode::PermanentSafe),
        "evict_local_copy" => Ok(CleanupMode::EvictLocalCopy),
        _ => Err(corrupt()),
    }
}

fn trigger_as_stored(value: CleanupTrigger) -> &'static str {
    match value {
        CleanupTrigger::Manual => "manual",
        CleanupTrigger::LowDisk => "low_disk",
        CleanupTrigger::Scheduled => "scheduled",
        CleanupTrigger::Cli => "cli",
    }
}

fn trigger_from_stored(value: &str) -> Result<CleanupTrigger, HistoryError> {
    match value {
        "manual" => Ok(CleanupTrigger::Manual),
        "low_disk" => Ok(CleanupTrigger::LowDisk),
        "scheduled" => Ok(CleanupTrigger::Scheduled),
        "cli" => Ok(CleanupTrigger::Cli),
        _ => Err(corrupt()),
    }
}

fn warning_as_stored(value: &PlanWarning) -> &'static str {
    match value {
        PlanWarning::EstimatedBytesUnverified => "estimated_bytes_unverified",
        PlanWarning::DryRunDoesNotMutate => "dry_run_does_not_mutate",
        PlanWarning::TrashDoesNotFreeSpaceImmediately => "trash_does_not_free_space_immediately",
        PlanWarning::PermanentRemovalCannotBeUndone => "permanent_removal_cannot_be_undone",
        PlanWarning::CloudEvictionRequiresNetworkToRedownload => {
            "cloud_eviction_requires_network_to_redownload"
        }
    }
}

fn warning_from_stored(value: &str) -> Result<PlanWarning, HistoryError> {
    match value {
        "estimated_bytes_unverified" => Ok(PlanWarning::EstimatedBytesUnverified),
        "dry_run_does_not_mutate" => Ok(PlanWarning::DryRunDoesNotMutate),
        "trash_does_not_free_space_immediately" => {
            Ok(PlanWarning::TrashDoesNotFreeSpaceImmediately)
        }
        "permanent_removal_cannot_be_undone" => Ok(PlanWarning::PermanentRemovalCannotBeUndone),
        "cloud_eviction_requires_network_to_redownload" => {
            Ok(PlanWarning::CloudEvictionRequiresNetworkToRedownload)
        }
        _ => Err(corrupt()),
    }
}

fn session_status_from_stored(value: &str) -> Result<LegacySessionStatus, HistoryError> {
    match value {
        "planned" => Ok(LegacySessionStatus::Planned),
        "running" => Ok(LegacySessionStatus::Running),
        "completed" => Ok(LegacySessionStatus::Completed),
        "partially_completed" => Ok(LegacySessionStatus::PartiallyCompleted),
        "failed" => Ok(LegacySessionStatus::Failed),
        "cancelled" => Ok(LegacySessionStatus::Cancelled),
        "interrupted" => Ok(LegacySessionStatus::Interrupted),
        "rejected" => Ok(LegacySessionStatus::Rejected),
        "dry_run" => Ok(LegacySessionStatus::DryRun),
        _ => Err(corrupt()),
    }
}

fn item_status_from_stored(value: &str) -> Result<LegacyItemStatus, HistoryError> {
    match value {
        "planned" => Ok(LegacyItemStatus::Planned),
        "dry_run" => Ok(LegacyItemStatus::DryRun),
        "trashed" => Ok(LegacyItemStatus::Trashed),
        "removed" => Ok(LegacyItemStatus::Removed),
        "evicted" => Ok(LegacyItemStatus::Evicted),
        "skipped" => Ok(LegacyItemStatus::Skipped),
        "rejected" => Ok(LegacyItemStatus::Rejected),
        "failed" => Ok(LegacyItemStatus::Failed),
        "changed_since_plan" => Ok(LegacyItemStatus::ChangedSincePlan),
        "interrupted" => Ok(LegacyItemStatus::Interrupted),
        "unavailable" => Ok(LegacyItemStatus::Unavailable),
        _ => Err(corrupt()),
    }
}

pub(crate) fn canonical_started_at(value: SystemTime) -> Result<SystemTime, HistoryError> {
    let elapsed = value.duration_since(UNIX_EPOCH).map_err(|_| invalid())?;
    let floor_milliseconds = elapsed.as_millis();
    let has_sub_millisecond = elapsed.subsec_nanos() % 1_000_000 != 0;
    let milliseconds = floor_milliseconds
        .checked_add(u128::from(has_sub_millisecond))
        .ok_or_else(invalid)?;
    let milliseconds = u64::try_from(milliseconds).map_err(|_| invalid())?;
    UNIX_EPOCH
        .checked_add(Duration::from_millis(milliseconds))
        .ok_or_else(invalid)
}

fn invalid() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidInput)
}

fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::{
        Candidate, CandidateInput, LocalizedTextKey, ProvenanceUrl, Rule, RuleDefinition,
        RuleGuards, RuleMatcher, RuleMatcherDefinition, RuleScope,
    };
    use crate::persistence::StoreCoordinator;
    use crate::persistence::candidate_history::{CandidateReviewTransition, NewCandidateRecord};
    use crate::persistence::history::{
        NewScanRecord, ScanCompletionRecord, ScanCounts, TerminalScanStatus,
    };

    fn rule(
        id: &str,
        category: CandidateCategory,
        safety: SafetyTier,
        action: CandidateAction,
    ) -> Rule {
        rule_with_schedule(id, category, safety, action, false)
    }

    fn rule_with_schedule(
        id: &str,
        category: CandidateCategory,
        safety: SafetyTier,
        action: CandidateAction,
        schedule_eligible: bool,
    ) -> Rule {
        Rule::try_new(RuleDefinition {
            reference: RuleRef::new(RuleId::new(id).unwrap(), RuleRevision::new(3).unwrap()),
            title_key: LocalizedTextKey::new("fixture.cleanup.title").unwrap(),
            category,
            scope: RuleScope::ConfiguredProjectRoots,
            matcher: RuleMatcher::try_new(RuleMatcherDefinition {
                path_component: Some("cleanup-fixture".to_owned()),
                required_ancestor_markers_any: Vec::new(),
                required_markers_all: Vec::new(),
                forbidden_markers_any: Vec::new(),
                exact_bundle_identifiers: Vec::new(),
                excluded_descendants: Vec::new(),
                protected_descendants: Vec::new(),
            })
            .unwrap(),
            guards: RuleGuards::try_new(
                None,
                0,
                Vec::new(),
                action == CandidateAction::EvictLocalCopy,
            )
            .unwrap(),
            safety,
            action,
            schedule_eligible,
            explanation_key: LocalizedTextKey::new("fixture.cleanup.explanation").unwrap(),
            provenance: vec![ProvenanceUrl::new("https://example.com/cleanup").unwrap()],
        })
        .unwrap()
    }

    fn candidate(id: &str, scan_id: &str, rule: &Rule, path: PathBuf, bytes: u64) -> Candidate {
        let evidence = if rule.action() == CandidateAction::EvictLocalCopy {
            vec![Evidence::CloudUploadComplete { path: path.clone() }]
        } else {
            vec![Evidence::MatchedPath { path: path.clone() }]
        };
        Candidate::try_from_rule(
            rule,
            CandidateInput::new(
                CandidateId::new(id).unwrap(),
                vec![path],
                bytes,
                None,
                evidence,
                Vec::new(),
                ScanId::new(scan_id).unwrap(),
            ),
        )
        .unwrap()
    }

    fn start_scan(store: &StoreCoordinator, root: &Path, id: &str) {
        let id = ScanId::new(id).unwrap();
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    id.clone(),
                    root.to_path_buf(),
                    UNIX_EPOCH + Duration::from_secs(1_750_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        store
            .record_scan_finished(
                &ScanCompletionRecord::try_new(
                    id,
                    UNIX_EPOCH + Duration::from_secs(1_750_000_001),
                    TerminalScanStatus::Succeeded,
                    ScanCounts::default(),
                )
                .unwrap(),
            )
            .unwrap();
    }

    fn persist_candidate(store: &StoreCoordinator, candidate: &Candidate, second: u64) {
        store
            .record_candidate_discovered(
                &NewCandidateRecord::try_from_candidate(
                    candidate,
                    UNIX_EPOCH + Duration::from_secs(second),
                )
                .unwrap(),
            )
            .unwrap();
    }

    fn plan(id: &str, mode: CleanupMode, candidates: &[Candidate]) -> CleanupPlan {
        CleanupPlan::try_from_candidates_for_persistence_test(
            CleanupPlanId::new(id).unwrap(),
            UNIX_EPOCH + Duration::from_secs(1_750_000_010),
            mode,
            candidates,
        )
        .unwrap()
    }

    fn candidate_status(store: &StoreCoordinator, id: &CandidateId) -> CandidateHistoryStatus {
        let Some(StoredCandidateRecord::Complete(candidate)) = store.load_candidate(id).unwrap()
        else {
            panic!("candidate was not a complete record");
        };
        candidate.status
    }

    #[test]
    fn planned_dry_run_round_trip_preserves_every_proposed_effect_after_reopen() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let scan_id = "scan:cleanup-roundtrip";
        let rules = [
            rule(
                "fixture.cleanup.permanent",
                CandidateCategory::ApplicationCache,
                SafetyTier::SafeRegenerable,
                CandidateAction::RemoveKnownRegenerableContents,
            ),
            rule(
                "fixture.cleanup.trash",
                CandidateCategory::LargeReviewItem,
                SafetyTier::ReviewRequired,
                CandidateAction::MoveToTrash,
            ),
            rule(
                "fixture.cleanup.evict",
                CandidateCategory::CloudFile,
                SafetyTier::SafeEvictable,
                CandidateAction::EvictLocalCopy,
            ),
        ];
        let candidates = rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                candidate(
                    &format!("candidate:cleanup-{index}"),
                    scan_id,
                    rule,
                    root.join(format!("cleanup-fixture-{index}")),
                    (index + 1) as u64 * 10,
                )
            })
            .collect::<Vec<_>>();
        let cleanup_plan = plan("plan:cleanup-roundtrip", CleanupMode::DryRun, &candidates);
        let session_id = CleanupSessionId::new("session:cleanup-roundtrip").unwrap();
        let started = UNIX_EPOCH + Duration::from_millis(1_750_000_011_123);
        {
            let store = StoreCoordinator::open(&database).unwrap();
            start_scan(&store, &root, scan_id);
            for (index, candidate) in candidates.iter().enumerate() {
                persist_candidate(&store, candidate, 1_750_000_002 + index as u64);
            }
            store
                .record_cleanup_session_planned(
                    &NewCleanupSessionRecord::try_from_plan(
                        session_id.clone(),
                        &cleanup_plan,
                        started,
                        CleanupTrigger::Manual,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        let store = StoreCoordinator::open(&database).unwrap();
        let StoredCleanupSessionRecord::Planned(stored) =
            store.load_cleanup_session(&session_id).unwrap().unwrap()
        else {
            panic!("format-2 session was not complete");
        };
        assert_eq!(stored.plan_id, cleanup_plan.id().clone());
        assert_eq!(stored.started_at, started);
        assert_eq!(stored.source_scan_id, cleanup_plan.source_scan_id().clone());
        assert_eq!(stored.plan_created_at, cleanup_plan.created_at());
        assert_eq!(stored.plan_expires_at, cleanup_plan.expires_at());
        assert_eq!(stored.mode, CleanupMode::DryRun);
        assert_eq!(stored.estimated_bytes, 60);
        assert_eq!(stored.trigger, CleanupTrigger::Manual);
        assert_eq!(
            stored.candidate_status_coupling,
            CandidateStatusCoupling::PlanClaimsV1
        );
        assert_eq!(stored.items.len(), 3);
        assert!(stored.items.iter().all(|item| {
            item.prior_review_status == Some(CandidatePriorReviewStatus::Discovered)
        }));
        assert_eq!(
            stored.items[0].proposed_action,
            CandidateAction::RemoveKnownRegenerableContents
        );
        assert_eq!(
            stored.items[1].proposed_action,
            CandidateAction::MoveToTrash
        );
        assert_eq!(
            stored.items[2].proposed_action,
            CandidateAction::EvictLocalCopy
        );
        assert_eq!(stored.warnings, cleanup_plan.warnings());
    }

    #[test]
    fn selected_review_state_preserves_plan_checks_and_dismissal_blocks_new_plans() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        let scan_id = "scan:cleanup-review-state";
        start_scan(&store, &root, scan_id);
        let policy = rule(
            "fixture.cleanup.review-state",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        );

        let selected = candidate(
            "candidate:cleanup-selected",
            scan_id,
            &policy,
            root.join("cleanup-fixture-selected"),
            10,
        );
        persist_candidate(&store, &selected, 1_750_000_002);
        store
            .transition_candidate_review_status(selected.id(), CandidateReviewTransition::Select)
            .unwrap();
        let selected_plan = plan(
            "plan:cleanup-selected",
            CleanupMode::PermanentSafe,
            std::slice::from_ref(&selected),
        );
        let selected_session = CleanupSessionId::new("session:cleanup-selected").unwrap();
        store
            .record_cleanup_session_planned(
                &NewCleanupSessionRecord::try_from_plan(
                    selected_session.clone(),
                    &selected_plan,
                    UNIX_EPOCH + Duration::from_secs(1_750_000_011),
                    CleanupTrigger::Manual,
                )
                .unwrap(),
            )
            .unwrap();
        let error = store
            .transition_candidate_review_status(
                selected.id(),
                CandidateReviewTransition::DismissSelected,
            )
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::InvalidTransition);
        let Some(StoredCandidateRecord::Complete(stored_candidate)) =
            store.load_candidate(selected.id()).unwrap()
        else {
            panic!("planned candidate was not complete");
        };
        assert_eq!(stored_candidate.status, CandidateHistoryStatus::Planned);
        store.with_connection(|connection| {
            let prior: String = connection
                .query_row(
                    "SELECT prior_review_status FROM candidate_plan_claims
                     WHERE candidate_id = ?1",
                    [selected.id().as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(prior, "selected");
        });
        assert!(matches!(
            store.load_cleanup_session(&selected_session).unwrap(),
            Some(StoredCleanupSessionRecord::Planned(_))
        ));

        let dismissed = candidate(
            "candidate:cleanup-dismissed",
            scan_id,
            &policy,
            root.join("cleanup-fixture-dismissed"),
            10,
        );
        persist_candidate(&store, &dismissed, 1_750_000_003);
        store
            .transition_candidate_review_status(
                dismissed.id(),
                CandidateReviewTransition::DismissDiscovered,
            )
            .unwrap();
        let dismissed_plan = plan(
            "plan:cleanup-dismissed",
            CleanupMode::PermanentSafe,
            std::slice::from_ref(&dismissed),
        );
        let error = store
            .record_cleanup_session_planned(
                &NewCleanupSessionRecord::try_from_plan(
                    CleanupSessionId::new("session:cleanup-dismissed").unwrap(),
                    &dismissed_plan,
                    UNIX_EPOCH + Duration::from_secs(1_750_000_012),
                    CleanupTrigger::Manual,
                )
                .unwrap(),
            )
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::InvalidInput);
        assert!(
            store
                .load_cleanup_session(&CleanupSessionId::new("session:cleanup-dismissed").unwrap())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn ordinary_plan_claim_cannot_borrow_trusted_rust_target_coupling() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        let scan_id = "scan:cleanup-generic-blocked";
        start_scan(&store, &root, scan_id);
        let policy = Rule::try_new(RuleDefinition {
            reference: RuleRef::new(
                RuleId::new("developer.rust.target").unwrap(),
                RuleRevision::new(2).unwrap(),
            ),
            title_key: LocalizedTextKey::new("fixture.cleanup.title").unwrap(),
            category: CandidateCategory::DeveloperArtifact,
            scope: RuleScope::ConfiguredProjectRoots,
            matcher: RuleMatcher::try_new(RuleMatcherDefinition {
                path_component: Some("cleanup-fixture".to_owned()),
                required_ancestor_markers_any: Vec::new(),
                required_markers_all: Vec::new(),
                forbidden_markers_any: Vec::new(),
                exact_bundle_identifiers: Vec::new(),
                excluded_descendants: Vec::new(),
                protected_descendants: Vec::new(),
            })
            .unwrap(),
            guards: RuleGuards::try_new(None, 0, Vec::new(), false).unwrap(),
            safety: SafetyTier::SafeRegenerable,
            action: CandidateAction::RemoveKnownRegenerableContents,
            schedule_eligible: false,
            explanation_key: LocalizedTextKey::new("fixture.cleanup.explanation").unwrap(),
            provenance: vec![ProvenanceUrl::new("https://example.com/cleanup").unwrap()],
        })
        .unwrap();
        let candidate = candidate(
            "candidate:cleanup-generic-blocked",
            scan_id,
            &policy,
            root.join("cleanup-fixture"),
            10,
        );
        persist_candidate(&store, &candidate, 1_750_000_002);
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO candidate_blockers (
                         candidate_id, blocker_ordinal, blocker_kind
                     ) VALUES (?1, 0, 'protected_path')",
                    [candidate.id().as_str()],
                )
                .unwrap();
        });

        let session_id = CleanupSessionId::new("session:cleanup-generic-blocked").unwrap();
        let record = NewCleanupSessionRecord::try_from_plan(
            session_id.clone(),
            &plan(
                "plan:cleanup-generic-blocked",
                CleanupMode::PermanentSafe,
                std::slice::from_ref(&candidate),
            ),
            UNIX_EPOCH + Duration::from_secs(1_750_000_011),
            CleanupTrigger::Manual,
        )
        .unwrap();
        assert_eq!(
            store
                .record_cleanup_session_planned(&record)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidInput
        );
        assert_eq!(
            candidate_status(&store, candidate.id()),
            CandidateHistoryStatus::Discovered
        );
        assert!(store.load_cleanup_session(&session_id).unwrap().is_none());
        store.with_connection(|connection| {
            let claims: i64 = connection
                .query_row("SELECT COUNT(*) FROM candidate_plan_claims", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(claims, 0);
        });
    }

    #[test]
    fn missing_candidate_and_duplicate_ids_roll_back_atomically() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let store = StoreCoordinator::open(&database).unwrap();
        start_scan(&store, &root, "scan:cleanup-atomic");
        let rule = rule(
            "fixture.cleanup.atomic",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        );
        let base_candidate = candidate(
            "candidate:cleanup-atomic",
            "scan:cleanup-atomic",
            &rule,
            root.join("cleanup-fixture"),
            10,
        );
        let cleanup_plan = plan(
            "plan:cleanup-atomic",
            CleanupMode::PermanentSafe,
            std::slice::from_ref(&base_candidate),
        );
        let record = NewCleanupSessionRecord::try_from_plan(
            CleanupSessionId::new("session:cleanup-atomic").unwrap(),
            &cleanup_plan,
            UNIX_EPOCH + Duration::from_secs(1_750_000_011),
            CleanupTrigger::Manual,
        )
        .unwrap();
        assert_eq!(
            store
                .record_cleanup_session_planned(&record)
                .unwrap_err()
                .kind,
            HistoryErrorKind::NotFound
        );
        store.with_connection(|connection| {
            let count: i64 = connection
                .query_row("SELECT count(*) FROM cleanup_sessions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
        });
        persist_candidate(&store, &base_candidate, 1_750_000_002);
        store.record_cleanup_session_planned(&record).unwrap();
        assert_eq!(
            store
                .record_cleanup_session_planned(&record)
                .unwrap_err()
                .kind,
            HistoryErrorKind::AlreadyExists
        );
        let same_plan = NewCleanupSessionRecord::try_from_plan(
            CleanupSessionId::new("session:cleanup-other").unwrap(),
            &cleanup_plan,
            UNIX_EPOCH + Duration::from_secs(1_750_000_012),
            CleanupTrigger::LowDisk,
        )
        .unwrap();
        assert_eq!(
            store
                .record_cleanup_session_planned(&same_plan)
                .unwrap_err()
                .kind,
            HistoryErrorKind::AlreadyExists
        );
        store.with_connection(|connection| {
            for table in [
                "cleanup_sessions",
                "cleanup_items",
                "cleanup_item_paths",
                "cleanup_item_evidence",
                "candidate_plan_claims",
            ] {
                let count: i64 = connection
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(count, 1, "unexpected rows in {table}");
            }
        });
    }

    #[test]
    fn incompatible_later_candidate_rolls_back_the_entire_plan_claim() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        let scan_id = "scan:cleanup-claim-rollback";
        start_scan(&store, &root, scan_id);
        let policy = rule(
            "fixture.cleanup.claim-rollback",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        );
        let candidates = [
            candidate(
                "candidate:cleanup-claim-rollback-0",
                scan_id,
                &policy,
                root.join("cleanup-fixture-0"),
                10,
            ),
            candidate(
                "candidate:cleanup-claim-rollback-1",
                scan_id,
                &policy,
                root.join("cleanup-fixture-1"),
                20,
            ),
        ];
        persist_candidate(&store, &candidates[0], 1_750_000_002);
        persist_candidate(&store, &candidates[1], 1_750_000_003);
        store
            .transition_candidate_review_status(
                candidates[1].id(),
                CandidateReviewTransition::DismissDiscovered,
            )
            .unwrap();
        let record = NewCleanupSessionRecord::try_from_plan(
            CleanupSessionId::new("session:cleanup-claim-rollback").unwrap(),
            &plan(
                "plan:cleanup-claim-rollback",
                CleanupMode::PermanentSafe,
                &candidates,
            ),
            UNIX_EPOCH + Duration::from_secs(1_750_000_011),
            CleanupTrigger::Manual,
        )
        .unwrap();
        assert_eq!(
            store
                .record_cleanup_session_planned(&record)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidInput
        );
        assert_eq!(
            candidate_status(&store, candidates[0].id()),
            CandidateHistoryStatus::Discovered
        );
        assert_eq!(
            candidate_status(&store, candidates[1].id()),
            CandidateHistoryStatus::Dismissed
        );
        store.with_connection(|connection| {
            let sessions: i64 = connection
                .query_row("SELECT COUNT(*) FROM cleanup_sessions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let claims: i64 = connection
                .query_row("SELECT COUNT(*) FROM candidate_plan_claims", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!((sessions, claims), (0, 0));
        });
    }

    #[test]
    fn competing_plans_create_exactly_one_candidate_claim() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        let scan_id = "scan:cleanup-claim-race";
        start_scan(&store, &root, scan_id);
        let policy = rule(
            "fixture.cleanup.claim-race",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        );
        let candidate = candidate(
            "candidate:cleanup-claim-race",
            scan_id,
            &policy,
            root.join("cleanup-fixture"),
            10,
        );
        persist_candidate(&store, &candidate, 1_750_000_002);
        let records = [
            NewCleanupSessionRecord::try_from_plan(
                CleanupSessionId::new("session:cleanup-claim-race-a").unwrap(),
                &plan(
                    "plan:cleanup-claim-race-a",
                    CleanupMode::PermanentSafe,
                    std::slice::from_ref(&candidate),
                ),
                UNIX_EPOCH + Duration::from_secs(1_750_000_011),
                CleanupTrigger::Manual,
            )
            .unwrap(),
            NewCleanupSessionRecord::try_from_plan(
                CleanupSessionId::new("session:cleanup-claim-race-b").unwrap(),
                &plan(
                    "plan:cleanup-claim-race-b",
                    CleanupMode::PermanentSafe,
                    std::slice::from_ref(&candidate),
                ),
                UNIX_EPOCH + Duration::from_secs(1_750_000_012),
                CleanupTrigger::LowDisk,
            )
            .unwrap(),
        ];
        let barrier = Arc::new(Barrier::new(3));
        let workers = records
            .into_iter()
            .map(|record| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    store.record_cleanup_session_planned(&record)
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .map(|error| error.kind)
                .collect::<Vec<_>>(),
            vec![HistoryErrorKind::InvalidInput]
        );
        assert_eq!(
            candidate_status(&store, candidate.id()),
            CandidateHistoryStatus::Planned
        );
        store.with_connection(|connection| {
            let sessions: i64 = connection
                .query_row("SELECT COUNT(*) FROM cleanup_sessions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let claims: i64 = connection
                .query_row("SELECT COUNT(*) FROM candidate_plan_claims", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!((sessions, claims), (1, 1));
        });
    }

    #[test]
    fn legacy_summary_is_explicit_and_rejects_v2_child_pollution() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let store = StoreCoordinator::open(&database).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO cleanup_sessions (
                     session_id, plan_id, started_at_unix_ms, mode, estimated_bytes,
                     trigger_source, status, record_format_version
                 ) VALUES ('session:legacy-cleanup', 'plan:legacy-cleanup', 1000,
                           'trash', 42, 'cli', 'completed', 1)",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO cleanup_items (
                     session_id, item_ordinal, rule_id, rule_revision, estimated_bytes,
                     final_status, record_format_version,
                     legacy_target_path, legacy_target_path_encoding
                 ) VALUES ('session:legacy-cleanup', 0, 'fixture.cleanup.legacy', 2, 42,
                           'trashed', 1, ?1, 1)",
                    [b"relative-legacy-cleanup".as_slice()],
                )
                .unwrap();
        });
        let id = CleanupSessionId::new("session:legacy-cleanup").unwrap();
        let StoredCleanupSessionRecord::LegacySummary(summary) =
            store.load_cleanup_session(&id).unwrap().unwrap()
        else {
            panic!("legacy cleanup was reconstructed as complete");
        };
        assert_eq!(summary.items.len(), 1);
        assert_eq!(
            summary.items[0].target_path,
            PathBuf::from("relative-legacy-cleanup")
        );
        assert_eq!(summary.items[0].status, LegacyItemStatus::Trashed);
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO cleanup_plan_warnings (
                     session_id, warning_ordinal, warning_kind
                 ) VALUES ('session:legacy-cleanup', 0, 'estimated_bytes_unverified')",
                    [],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_cleanup_session(&id).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM cleanup_plan_warnings
                     WHERE session_id = 'session:legacy-cleanup'",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE cleanup_sessions SET status = 'recovering'
                     WHERE session_id = 'session:legacy-cleanup'",
                    [],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_cleanup_session(&id).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE cleanup_sessions SET status = 'completed'
                     WHERE session_id = 'session:legacy-cleanup'",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE cleanup_items SET final_status = 'validating'
                     WHERE session_id = 'session:legacy-cleanup'",
                    [],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_cleanup_session(&id).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn malformed_children_and_newer_schema_fail_closed() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let store = StoreCoordinator::open(&database).unwrap();
        start_scan(&store, &root, "scan:cleanup-malformed");
        let rule = rule(
            "fixture.cleanup.malformed",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        );
        let candidate = candidate(
            "candidate:cleanup-malformed",
            "scan:cleanup-malformed",
            &rule,
            root.join("cleanup-fixture"),
            10,
        );
        persist_candidate(&store, &candidate, 1_750_000_002);
        let cleanup_plan = plan(
            "plan:cleanup-malformed",
            CleanupMode::PermanentSafe,
            &[candidate],
        );
        let id = CleanupSessionId::new("session:cleanup-malformed").unwrap();
        store
            .record_cleanup_session_planned(
                &NewCleanupSessionRecord::try_from_plan(
                    id.clone(),
                    &cleanup_plan,
                    UNIX_EPOCH + Duration::from_secs(1_750_000_011),
                    CleanupTrigger::Cli,
                )
                .unwrap(),
            )
            .unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE cleanup_sessions SET trigger_source = 'scheduled'
                     WHERE session_id = 'session:cleanup-malformed'",
                    [],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_cleanup_session(&id).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE cleanup_sessions SET trigger_source = 'cli'
                     WHERE session_id = 'session:cleanup-malformed'",
                    [],
                )
                .unwrap();
            connection
                .pragma_update(None, "foreign_keys", false)
                .unwrap();
            connection
                .execute(
                    "INSERT INTO cleanup_item_paths (
                         session_id, item_ordinal, path_ordinal, target_path,
                         target_path_encoding, status
                     ) VALUES ('session:cleanup-malformed', 99, 0, ?1, 1, 'planned')",
                    [b"/orphan-cleanup-child".as_slice()],
                )
                .unwrap();
            connection
                .pragma_update(None, "foreign_keys", true)
                .unwrap();
        });
        assert_eq!(
            store.load_cleanup_session(&id).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM cleanup_item_paths
                     WHERE session_id = 'session:cleanup-malformed' AND item_ordinal = 99",
                    [],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "UPDATE cleanup_item_paths SET target_path = zeroblob(65537)
                 WHERE session_id = 'session:cleanup-malformed'",
                    [],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            store.load_cleanup_session(&id).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO schema_migrations (
                     version, name, checksum_sha256, applied_at_unix_ms
                 ) VALUES (?1, 'future-cleanup-schema', zeroblob(32), 2)",
                    [i64::from(crate::DATABASE_SCHEMA_VERSION + 1)],
                )
                .unwrap();
            connection
                .pragma_update(None, "user_version", crate::DATABASE_SCHEMA_VERSION + 1)
                .unwrap();
        });
        assert_eq!(
            store.load_cleanup_session(&id).unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
    }

    #[test]
    fn maximum_legal_path_and_evidence_counts_fit_the_shared_query_budget() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let store = StoreCoordinator::open(&database).unwrap();
        start_scan(&store, &root, "scan:cleanup-maximum");
        let rule = rule(
            "fixture.cleanup.maximum",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        );
        let paths = (0..MAX_TOTAL_PATHS)
            .map(|index| root.join(format!("cleanup-fixture-{index}")))
            .collect::<Vec<_>>();
        let evidence = (0..MAX_TOTAL_EVIDENCE)
            .map(|index| Evidence::MinimumSize {
                observed_bytes: index as u64 + 1,
                minimum_bytes: index as u64,
            })
            .collect::<Vec<_>>();
        let candidate = Candidate::try_from_rule(
            &rule,
            CandidateInput::new(
                CandidateId::new("candidate:cleanup-maximum").unwrap(),
                paths,
                1,
                None,
                evidence,
                Vec::new(),
                ScanId::new("scan:cleanup-maximum").unwrap(),
            ),
        )
        .unwrap();
        persist_candidate(&store, &candidate, 1_750_000_002);
        let cleanup_plan = plan(
            "plan:cleanup-maximum",
            CleanupMode::PermanentSafe,
            &[candidate],
        );
        let id = CleanupSessionId::new("session:cleanup-maximum").unwrap();
        store
            .record_cleanup_session_planned(
                &NewCleanupSessionRecord::try_from_plan(
                    id.clone(),
                    &cleanup_plan,
                    UNIX_EPOCH + Duration::from_secs(1_750_000_011),
                    CleanupTrigger::Manual,
                )
                .unwrap(),
            )
            .unwrap();
        let StoredCleanupSessionRecord::Planned(stored) =
            store.load_cleanup_session(&id).unwrap().unwrap()
        else {
            panic!("maximum record became incomplete");
        };
        assert_eq!(stored.items[0].paths.len(), MAX_TOTAL_PATHS);
        assert_eq!(stored.items[0].evidence.len(), MAX_TOTAL_EVIDENCE);
    }

    #[test]
    fn session_ids_and_plan_start_times_are_bounded_before_storage_locking() {
        for invalid_id in ["", "session with spaces"] {
            assert_eq!(
                CleanupSessionId::new(invalid_id).unwrap_err().kind,
                HistoryErrorKind::InvalidInput
            );
        }
        assert_eq!(
            CleanupSessionId::new("x".repeat(129)).unwrap_err().kind,
            HistoryErrorKind::InvalidInput
        );
        let rule = rule(
            "fixture.cleanup.input",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        );
        let base_candidate = candidate(
            "candidate:cleanup-input",
            "scan:cleanup-input",
            &rule,
            PathBuf::from("/cleanup-fixture"),
            1,
        );
        let cleanup_plan = plan(
            "plan:cleanup-input",
            CleanupMode::PermanentSafe,
            &[base_candidate],
        );
        for invalid_start in [
            cleanup_plan
                .created_at()
                .checked_sub(Duration::from_millis(1))
                .unwrap(),
            cleanup_plan.expires_at(),
        ] {
            assert_eq!(
                NewCleanupSessionRecord::try_from_plan(
                    CleanupSessionId::new("session:cleanup-input").unwrap(),
                    &cleanup_plan,
                    invalid_start,
                    CleanupTrigger::Manual,
                )
                .unwrap_err()
                .kind,
                HistoryErrorKind::InvalidInput
            );
        }

        assert_eq!(
            NewCleanupSessionRecord::try_from_plan(
                CleanupSessionId::new("session:cleanup-unscheduled").unwrap(),
                &cleanup_plan,
                cleanup_plan.created_at(),
                CleanupTrigger::Scheduled,
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );

        let scheduled_rule = rule_with_schedule(
            "fixture.cleanup.scheduled-input",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            true,
        );
        let scheduled_candidate = candidate(
            "candidate:cleanup-scheduled-input",
            "scan:cleanup-input",
            &scheduled_rule,
            PathBuf::from("/cleanup-fixture-scheduled"),
            1,
        );
        let scheduled_plan = plan(
            "plan:cleanup-scheduled-input",
            CleanupMode::PermanentSafe,
            std::slice::from_ref(&scheduled_candidate),
        );
        NewCleanupSessionRecord::try_from_plan(
            CleanupSessionId::new("session:cleanup-scheduled-input").unwrap(),
            &scheduled_plan,
            scheduled_plan.created_at(),
            CleanupTrigger::Scheduled,
        )
        .unwrap();

        let submillisecond_created = UNIX_EPOCH + Duration::new(1_750_000_100, 500_000);
        let precise_plan = CleanupPlan::try_from_candidates_for_persistence_test(
            CleanupPlanId::new("plan:cleanup-precise-time").unwrap(),
            submillisecond_created,
            CleanupMode::PermanentSafe,
            &[scheduled_candidate],
        )
        .unwrap();
        let precise = NewCleanupSessionRecord::try_from_plan(
            CleanupSessionId::new("session:cleanup-precise-time").unwrap(),
            &precise_plan,
            precise_plan.created_at(),
            CleanupTrigger::Manual,
        )
        .unwrap();
        assert_eq!(
            precise.started_at,
            UNIX_EPOCH + Duration::new(1_750_000_100, 1_000_000)
        );
        assert_eq!(
            NewCleanupSessionRecord::try_from_plan(
                CleanupSessionId::new("session:cleanup-precise-expiry").unwrap(),
                &precise_plan,
                precise_plan.expires_at(),
                CleanupTrigger::Manual,
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );
        assert_eq!(
            NewCleanupSessionRecord::try_from_plan(
                CleanupSessionId::new("session:cleanup-unrepresentable-expiry").unwrap(),
                &precise_plan,
                precise_plan
                    .expires_at()
                    .checked_sub(Duration::from_nanos(1))
                    .unwrap(),
                CleanupTrigger::Manual,
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );
    }
}
