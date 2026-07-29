//! Bounded, path-free cleanup-history observations.
//!
//! These values deliberately omit target paths, evidence payloads, candidate
//! identities, execution fences, claims, and prior-review state. They are
//! presentation observations and cannot authorize cleanup.

use std::time::SystemTime;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::domain::{
    CLEANUP_PLAN_VALIDITY, CandidateAction, CandidateCategory, CleanupMode, CleanupPlanId,
    PlanWarning, RuleRef, SafetyTier, ScanId,
};

use super::cleanup_history::{
    CleanupSessionId, CleanupTrigger, LegacyCleanupItemSummary, LegacyItemStatus,
    LegacySessionStatus, StoredCleanupSessionRecord, load_within_budget,
};
use super::cleanup_journal::{
    ActivePhase, CleanupJournal, JournalItem, JournalLifecycle, PathStatus, TerminalSessionStatus,
    load_cleanup_journal_within_budget, validate_cleanup_journal_scalar_state_within_budget,
};
use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error,
    run_bounded_cleanup_history_observation_query, run_bounded_cleanup_history_page_query,
    unix_ms_to_system_time,
};

pub(crate) const MAX_RECENT_CLEANUP_HISTORY_LIMIT: usize = 64;
const MAX_ITEMS: i64 = 64;
const MAX_PATHS: i64 = 256;
const MAX_EVIDENCE: i64 = 512;
const MAX_WARNINGS: i64 = 5;
const MAX_GRAPH_MATERIALIZATION_BYTES: u64 = 64 * 1024 * 1024;
const MATERIALIZED_PAYLOAD_MULTIPLIER: u64 = 3;
const MATERIALIZED_ITEM_CHARGE: u64 = 1_024;
const MATERIALIZED_PATH_CHARGE: u64 = 512;
const MATERIALIZED_EVIDENCE_CHARGE: u64 = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredCleanupHistoryCursor {
    pub(crate) started_at_unix_ms: i64,
    pub(crate) session_id: CleanupSessionId,
}

impl StoredCleanupHistoryCursor {
    pub(crate) fn try_new(
        started_at_unix_ms: i64,
        session_id: CleanupSessionId,
    ) -> Result<Self, HistoryError> {
        if started_at_unix_ms < 0 {
            return Err(invalid());
        }
        Ok(Self {
            started_at_unix_ms,
            session_id,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredCleanupHistoryPage {
    pub(crate) records: Vec<StoredCleanupSessionSummary>,
    pub(crate) next_cursor: Option<StoredCleanupHistoryCursor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoredCleanupRecordFormat {
    /// Migrated schema-v1 rows cannot prove complete plan or journal facts.
    LegacyIncomplete,
    CompleteV2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoredCleanupMode {
    DryRun,
    Trash,
    PermanentSafe,
    EvictLocalCopy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoredCleanupTrigger {
    Manual,
    LowDisk,
    Scheduled,
    Cli,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoredCleanupSessionStatus {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoredCleanupItemStatus {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredCleanupErrorCategory(String);

impl StoredCleanupErrorCategory {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StoredCleanupStatusCounts {
    pub(crate) planned: u16,
    pub(crate) validating: u16,
    pub(crate) dry_run: u16,
    pub(crate) effect_started: u16,
    pub(crate) trashed: u16,
    pub(crate) removed: u16,
    pub(crate) evicted: u16,
    pub(crate) skipped: u16,
    pub(crate) rejected: u16,
    pub(crate) failed: u16,
    pub(crate) changed_since_plan: u16,
    pub(crate) interrupted: u16,
    pub(crate) unavailable: u16,
    pub(crate) outcome_unknown: u16,
    pub(crate) total: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredCleanupSessionSummary {
    pub(crate) session_id: CleanupSessionId,
    pub(crate) plan_id: CleanupPlanId,
    pub(crate) format: StoredCleanupRecordFormat,
    pub(crate) source_scan_id: Option<ScanId>,
    pub(crate) started_at: SystemTime,
    pub(crate) completed_at: Option<SystemTime>,
    pub(crate) plan_created_at: Option<SystemTime>,
    pub(crate) plan_expires_at: Option<SystemTime>,
    pub(crate) mode: StoredCleanupMode,
    pub(crate) estimated_bytes: u64,
    pub(crate) verified_capacity_delta_bytes: Option<i64>,
    pub(crate) trigger: StoredCleanupTrigger,
    pub(crate) status: StoredCleanupSessionStatus,
    pub(crate) cancellation_requested: Option<bool>,
    pub(crate) item_total: u16,
    pub(crate) path_total: u16,
    pub(crate) evidence_total: u16,
    pub(crate) item_status_counts: StoredCleanupStatusCounts,
    pub(crate) path_status_counts: StoredCleanupStatusCounts,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredCleanupItemSummary {
    pub(crate) ordinal: u32,
    pub(crate) rule: RuleRef,
    pub(crate) category: Option<CandidateCategory>,
    pub(crate) safety: Option<SafetyTier>,
    pub(crate) action: Option<CandidateAction>,
    pub(crate) rule_schedule_eligible: Option<bool>,
    pub(crate) newest_mtime: Option<SystemTime>,
    pub(crate) estimated_bytes: u64,
    pub(crate) status: StoredCleanupItemStatus,
    pub(crate) error_recorded: bool,
    pub(crate) error_category: Option<StoredCleanupErrorCategory>,
    pub(crate) path_count: u16,
    pub(crate) evidence_count: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredCleanupHistoryObservation {
    pub(crate) summary: StoredCleanupSessionSummary,
    pub(crate) items: Vec<StoredCleanupItemSummary>,
    pub(crate) warnings: Vec<PlanWarning>,
}

pub(super) fn recent_cleanup_history(
    connection: &Connection,
    cursor: Option<&StoredCleanupHistoryCursor>,
    limit: usize,
) -> Result<StoredCleanupHistoryPage, HistoryError> {
    if limit == 0 || limit > MAX_RECENT_CLEANUP_HISTORY_LIMIT {
        return Err(invalid());
    }
    if cursor.is_some_and(|value| value.started_at_unix_ms < 0) {
        return Err(invalid());
    }
    run_bounded_cleanup_history_page_query(connection, || {
        recent_cleanup_history_within_budget(connection, cursor, limit)
    })
}

pub(super) fn cleanup_history_session(
    connection: &Connection,
    id: &CleanupSessionId,
) -> Result<Option<StoredCleanupHistoryObservation>, HistoryError> {
    run_bounded_cleanup_history_observation_query(connection, || {
        let Some(parent) = load_parent(connection, id)? else {
            return Ok(None);
        };
        let graph = preflight_graph(connection, &parent)?;
        Ok(Some(load_validated_observation(
            connection, &parent, &graph,
        )?))
    })
}

fn recent_cleanup_history_within_budget(
    connection: &Connection,
    cursor: Option<&StoredCleanupHistoryCursor>,
    limit: usize,
) -> Result<StoredCleanupHistoryPage, HistoryError> {
    let sql = if cursor.is_some() {
        "SELECT record_format_version, session_id, started_at_unix_ms,
                completed_at_unix_ms, mode, estimated_bytes,
                verified_capacity_delta_bytes, trigger_source, status,
                source_scan_id IS NULL, plan_created_at_unix_seconds IS NULL,
                plan_created_at_nanoseconds IS NULL,
                plan_expires_at_unix_seconds IS NULL,
                plan_expires_at_nanoseconds IS NULL,
                execution_owner_id IS NULL, execution_generation IS NULL,
                last_heartbeat_at_unix_ms IS NULL, cancellation_requested,
                candidate_status_coupling_version,
                CASE WHEN
                    typeof(record_format_version) = 'integer' AND
                    typeof(session_id) = 'text' AND length(CAST(session_id AS BLOB)) BETWEEN 1 AND 128 AND
                    typeof(plan_id) = 'text' AND length(CAST(plan_id AS BLOB)) BETWEEN 1 AND 128 AND
                    typeof(started_at_unix_ms) = 'integer' AND
                    typeof(completed_at_unix_ms) IN ('integer', 'null') AND
                    typeof(mode) = 'text' AND length(CAST(mode AS BLOB)) BETWEEN 1 AND 64 AND
                    typeof(estimated_bytes) = 'integer' AND
                    typeof(verified_capacity_delta_bytes) IN ('integer', 'null') AND
                    typeof(trigger_source) = 'text' AND length(CAST(trigger_source AS BLOB)) BETWEEN 1 AND 64 AND
                    typeof(status) = 'text' AND length(CAST(status AS BLOB)) BETWEEN 1 AND 64 AND
                    typeof(source_scan_id) IN ('text', 'null') AND
                    (source_scan_id IS NULL OR length(CAST(source_scan_id AS BLOB)) BETWEEN 1 AND 128) AND
                    typeof(plan_created_at_unix_seconds) IN ('integer', 'null') AND
                    typeof(plan_created_at_nanoseconds) IN ('integer', 'null') AND
                    typeof(plan_expires_at_unix_seconds) IN ('integer', 'null') AND
                    typeof(plan_expires_at_nanoseconds) IN ('integer', 'null') AND
                    typeof(execution_owner_id) IN ('text', 'null') AND
                    (execution_owner_id IS NULL OR length(CAST(execution_owner_id AS BLOB)) BETWEEN 1 AND 128) AND
                    typeof(execution_generation) IN ('integer', 'null') AND
                    typeof(last_heartbeat_at_unix_ms) IN ('integer', 'null') AND
                    typeof(cancellation_requested) IN ('integer', 'null') AND
                    typeof(candidate_status_coupling_version) = 'integer'
                THEN 0 ELSE 1 END,
                plan_id, source_scan_id, plan_created_at_unix_seconds,
                plan_created_at_nanoseconds, plan_expires_at_unix_seconds,
                plan_expires_at_nanoseconds
         FROM cleanup_sessions
         WHERE started_at_unix_ms < ?1
            OR (started_at_unix_ms = ?1 AND session_id > ?2)
         ORDER BY started_at_unix_ms DESC, session_id ASC LIMIT ?3"
    } else {
        "SELECT record_format_version, session_id, started_at_unix_ms,
                completed_at_unix_ms, mode, estimated_bytes,
                verified_capacity_delta_bytes, trigger_source, status,
                source_scan_id IS NULL, plan_created_at_unix_seconds IS NULL,
                plan_created_at_nanoseconds IS NULL,
                plan_expires_at_unix_seconds IS NULL,
                plan_expires_at_nanoseconds IS NULL,
                execution_owner_id IS NULL, execution_generation IS NULL,
                last_heartbeat_at_unix_ms IS NULL, cancellation_requested,
                candidate_status_coupling_version,
                CASE WHEN
                    typeof(record_format_version) = 'integer' AND
                    typeof(session_id) = 'text' AND length(CAST(session_id AS BLOB)) BETWEEN 1 AND 128 AND
                    typeof(plan_id) = 'text' AND length(CAST(plan_id AS BLOB)) BETWEEN 1 AND 128 AND
                    typeof(started_at_unix_ms) = 'integer' AND
                    typeof(completed_at_unix_ms) IN ('integer', 'null') AND
                    typeof(mode) = 'text' AND length(CAST(mode AS BLOB)) BETWEEN 1 AND 64 AND
                    typeof(estimated_bytes) = 'integer' AND
                    typeof(verified_capacity_delta_bytes) IN ('integer', 'null') AND
                    typeof(trigger_source) = 'text' AND length(CAST(trigger_source AS BLOB)) BETWEEN 1 AND 64 AND
                    typeof(status) = 'text' AND length(CAST(status AS BLOB)) BETWEEN 1 AND 64 AND
                    typeof(source_scan_id) IN ('text', 'null') AND
                    (source_scan_id IS NULL OR length(CAST(source_scan_id AS BLOB)) BETWEEN 1 AND 128) AND
                    typeof(plan_created_at_unix_seconds) IN ('integer', 'null') AND
                    typeof(plan_created_at_nanoseconds) IN ('integer', 'null') AND
                    typeof(plan_expires_at_unix_seconds) IN ('integer', 'null') AND
                    typeof(plan_expires_at_nanoseconds) IN ('integer', 'null') AND
                    typeof(execution_owner_id) IN ('text', 'null') AND
                    (execution_owner_id IS NULL OR length(CAST(execution_owner_id AS BLOB)) BETWEEN 1 AND 128) AND
                    typeof(execution_generation) IN ('integer', 'null') AND
                    typeof(last_heartbeat_at_unix_ms) IN ('integer', 'null') AND
                    typeof(cancellation_requested) IN ('integer', 'null') AND
                    typeof(candidate_status_coupling_version) = 'integer'
                THEN 0 ELSE 1 END,
                plan_id, source_scan_id, plan_created_at_unix_seconds,
                plan_created_at_nanoseconds, plan_expires_at_unix_seconds,
                plan_expires_at_nanoseconds
         FROM cleanup_sessions
         ORDER BY started_at_unix_ms DESC, session_id ASC LIMIT ?1"
    };
    let fetch = i64::try_from(limit + 1).map_err(|_| invalid())?;
    let mut statement = connection.prepare(sql).map_err(map_query_sql_error)?;
    let rows = if let Some(cursor) = cursor {
        statement
            .query_map(
                params![cursor.started_at_unix_ms, cursor.session_id.as_str(), fetch],
                raw_parent_row,
            )
            .map_err(map_query_sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_query_sql_error)?
    } else {
        statement
            .query_map([fetch], raw_parent_row)
            .map_err(map_query_sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_query_sql_error)?
    };
    let mut parents = rows;
    let has_more = parents.len() > limit;
    if has_more {
        parents.truncate(limit);
    }
    let mut records = Vec::with_capacity(parents.len());
    for parent in &parents {
        let graph = preflight_graph(connection, parent)?;
        records.push(parent.summary(&graph)?);
    }
    let next_cursor = if has_more {
        parents.last().map(|parent| StoredCleanupHistoryCursor {
            started_at_unix_ms: parent.started_at_unix_ms,
            session_id: parent.session_id.clone(),
        })
    } else {
        None
    };
    Ok(StoredCleanupHistoryPage {
        records,
        next_cursor,
    })
}

fn load_validated_observation(
    connection: &Connection,
    parent: &RawParent,
    graph: &GraphPreflight,
) -> Result<StoredCleanupHistoryObservation, HistoryError> {
    let observation = match parent.version {
        1 => {
            let Some(StoredCleanupSessionRecord::LegacySummary(record)) =
                load_within_budget(connection, &parent.session_id)?
            else {
                return Err(corrupt());
            };
            project_legacy(record)
        }
        2 => {
            let journal = load_cleanup_journal_within_budget(connection, &parent.session_id)?
                .ok_or_else(corrupt)?;
            project_journal(journal)
        }
        _ => return Err(corrupt()),
    };
    if observation.summary != parent.summary(graph)? {
        return Err(corrupt());
    }
    Ok(observation)
}

#[derive(Clone)]
struct RawParent {
    version: i64,
    session_id: CleanupSessionId,
    plan_id: CleanupPlanId,
    source_scan_id: Option<ScanId>,
    started_at_unix_ms: i64,
    completed_at_unix_ms: Option<i64>,
    mode: StoredCleanupMode,
    estimated_bytes: u64,
    capacity_delta: Option<i64>,
    trigger: StoredCleanupTrigger,
    status: StoredCleanupSessionStatus,
    source_null: bool,
    created_seconds_null: bool,
    created_nanos_null: bool,
    expires_seconds_null: bool,
    expires_nanos_null: bool,
    owner_null: bool,
    generation_null: bool,
    heartbeat_null: bool,
    cancellation_requested: Option<i64>,
    coupling: i64,
    plan_created_at: Option<SystemTime>,
    plan_expires_at: Option<SystemTime>,
}

impl RawParent {
    fn summary(&self, graph: &GraphPreflight) -> Result<StoredCleanupSessionSummary, HistoryError> {
        Ok(StoredCleanupSessionSummary {
            session_id: self.session_id.clone(),
            plan_id: self.plan_id.clone(),
            format: if self.version == 1 {
                StoredCleanupRecordFormat::LegacyIncomplete
            } else {
                StoredCleanupRecordFormat::CompleteV2
            },
            source_scan_id: self.source_scan_id.clone(),
            started_at: unix_ms_to_system_time(self.started_at_unix_ms)?,
            completed_at: self
                .completed_at_unix_ms
                .map(unix_ms_to_system_time)
                .transpose()?,
            plan_created_at: self.plan_created_at,
            plan_expires_at: self.plan_expires_at,
            mode: self.mode,
            estimated_bytes: self.estimated_bytes,
            verified_capacity_delta_bytes: self.capacity_delta,
            trigger: self.trigger,
            status: self.status,
            cancellation_requested: self.cancellation_requested.map(|value| value != 0),
            item_total: graph.item_total,
            path_total: graph.path_total,
            evidence_total: graph.evidence_total,
            item_status_counts: graph.item_status_counts,
            path_status_counts: graph.path_status_counts,
        })
    }
}

fn raw_parent_row(row: &Row<'_>) -> rusqlite::Result<RawParent> {
    if row.get::<_, i64>(19)? != 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let raw = RawParent {
        version: row.get(0)?,
        session_id: CleanupSessionId::new(row.get::<_, String>(1)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        plan_id: CleanupPlanId::new(row.get::<_, String>(20)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        source_scan_id: row
            .get::<_, Option<String>>(21)?
            .map(ScanId::new)
            .transpose()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        started_at_unix_ms: row.get(2)?,
        completed_at_unix_ms: row.get(3)?,
        mode: mode_from_stored(&row.get::<_, String>(4)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        estimated_bytes: u64::try_from(row.get::<_, i64>(5)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        capacity_delta: row.get(6)?,
        trigger: trigger_from_stored(&row.get::<_, String>(7)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        status: session_status_from_stored(&row.get::<_, String>(8)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        source_null: row.get(9)?,
        created_seconds_null: row.get(10)?,
        created_nanos_null: row.get(11)?,
        expires_seconds_null: row.get(12)?,
        expires_nanos_null: row.get(13)?,
        owner_null: row.get(14)?,
        generation_null: row.get(15)?,
        heartbeat_null: row.get(16)?,
        cancellation_requested: row.get(17)?,
        coupling: row.get(18)?,
        plan_created_at: decode_optional_seconds_nanos(row.get(22)?, row.get(23)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        plan_expires_at: decode_optional_seconds_nanos(row.get(24)?, row.get(25)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    };
    validate_parent_shape(&raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(raw)
}

fn load_parent(
    connection: &Connection,
    id: &CleanupSessionId,
) -> Result<Option<RawParent>, HistoryError> {
    connection
        .query_row(
            "SELECT record_format_version, session_id, started_at_unix_ms,
                    completed_at_unix_ms, mode, estimated_bytes,
                    verified_capacity_delta_bytes, trigger_source, status,
                    source_scan_id IS NULL, plan_created_at_unix_seconds IS NULL,
                    plan_created_at_nanoseconds IS NULL,
                    plan_expires_at_unix_seconds IS NULL,
                    plan_expires_at_nanoseconds IS NULL,
                    execution_owner_id IS NULL, execution_generation IS NULL,
                    last_heartbeat_at_unix_ms IS NULL, cancellation_requested,
                    candidate_status_coupling_version,
                    CASE WHEN
                        typeof(record_format_version) = 'integer' AND
                        typeof(session_id) = 'text' AND length(CAST(session_id AS BLOB)) BETWEEN 1 AND 128 AND
                        typeof(plan_id) = 'text' AND length(CAST(plan_id AS BLOB)) BETWEEN 1 AND 128 AND
                        typeof(started_at_unix_ms) = 'integer' AND
                        typeof(completed_at_unix_ms) IN ('integer', 'null') AND
                        typeof(mode) = 'text' AND length(CAST(mode AS BLOB)) BETWEEN 1 AND 64 AND
                        typeof(estimated_bytes) = 'integer' AND
                        typeof(verified_capacity_delta_bytes) IN ('integer', 'null') AND
                        typeof(trigger_source) = 'text' AND length(CAST(trigger_source AS BLOB)) BETWEEN 1 AND 64 AND
                        typeof(status) = 'text' AND length(CAST(status AS BLOB)) BETWEEN 1 AND 64 AND
                        typeof(source_scan_id) IN ('text', 'null') AND
                        (source_scan_id IS NULL OR length(CAST(source_scan_id AS BLOB)) BETWEEN 1 AND 128) AND
                        typeof(plan_created_at_unix_seconds) IN ('integer', 'null') AND
                        typeof(plan_created_at_nanoseconds) IN ('integer', 'null') AND
                        typeof(plan_expires_at_unix_seconds) IN ('integer', 'null') AND
                        typeof(plan_expires_at_nanoseconds) IN ('integer', 'null') AND
                        typeof(execution_owner_id) IN ('text', 'null') AND
                        (execution_owner_id IS NULL OR length(CAST(execution_owner_id AS BLOB)) BETWEEN 1 AND 128) AND
                        typeof(execution_generation) IN ('integer', 'null') AND
                        typeof(last_heartbeat_at_unix_ms) IN ('integer', 'null') AND
                        typeof(cancellation_requested) IN ('integer', 'null') AND
                        typeof(candidate_status_coupling_version) = 'integer'
                    THEN 0 ELSE 1 END,
                    plan_id, source_scan_id, plan_created_at_unix_seconds,
                    plan_created_at_nanoseconds, plan_expires_at_unix_seconds,
                    plan_expires_at_nanoseconds
             FROM cleanup_sessions WHERE session_id = ?1",
            [id.as_str()],
            raw_parent_row,
        )
        .optional()
        .map_err(map_query_sql_error)
}

fn validate_parent_shape(raw: &RawParent) -> Result<(), HistoryError> {
    if raw.started_at_unix_ms < 0
        || raw
            .completed_at_unix_ms
            .is_some_and(|value| value < raw.started_at_unix_ms)
    {
        return Err(corrupt());
    }
    let v2_nulls = raw.source_null
        || raw.created_seconds_null
        || raw.created_nanos_null
        || raw.expires_seconds_null
        || raw.expires_nanos_null;
    let all_v2_null = raw.source_null
        && raw.created_seconds_null
        && raw.created_nanos_null
        && raw.expires_seconds_null
        && raw.expires_nanos_null;
    match raw.version {
        1 if !all_v2_null
            || raw.source_scan_id.is_some()
            || raw.plan_created_at.is_some()
            || raw.plan_expires_at.is_some()
            || !raw.owner_null
            || !raw.generation_null
            || !raw.heartbeat_null
            || raw.cancellation_requested.is_some()
            || raw.coupling != 1
            || raw.status == StoredCleanupSessionStatus::Recovering =>
        {
            Err(corrupt())
        }
        1 => Ok(()),
        2 if v2_nulls
            || raw.source_scan_id.is_none()
            || raw.plan_created_at.is_none()
            || raw.plan_expires_at.is_none()
            || !matches!(raw.cancellation_requested, Some(0 | 1))
            || !matches!(raw.coupling, 1 | 2) =>
        {
            Err(corrupt())
        }
        2 => {
            let plan_created_at = raw.plan_created_at.ok_or_else(corrupt)?;
            let plan_expires_at = raw.plan_expires_at.ok_or_else(corrupt)?;
            let started_at = unix_ms_to_system_time(raw.started_at_unix_ms)?;
            if !(plan_created_at <= started_at && started_at < plan_expires_at)
                || plan_created_at.checked_add(CLEANUP_PLAN_VALIDITY) != Some(plan_expires_at)
            {
                return Err(corrupt());
            }
            let no_fence = raw.owner_null && raw.generation_null && raw.heartbeat_null;
            let full_fence = !raw.owner_null && !raw.generation_null && !raw.heartbeat_null;
            let shape = match raw.status {
                StoredCleanupSessionStatus::Planned => {
                    no_fence
                        && raw.completed_at_unix_ms.is_none()
                        && raw.capacity_delta.is_none()
                        && raw.cancellation_requested == Some(0)
                }
                StoredCleanupSessionStatus::Running | StoredCleanupSessionStatus::Recovering => {
                    full_fence && raw.completed_at_unix_ms.is_none() && raw.capacity_delta.is_none()
                }
                StoredCleanupSessionStatus::DryRun
                | StoredCleanupSessionStatus::Failed
                | StoredCleanupSessionStatus::Cancelled
                | StoredCleanupSessionStatus::Interrupted
                | StoredCleanupSessionStatus::Rejected
                    if raw.mode == StoredCleanupMode::DryRun
                        && raw.coupling == 1
                        && raw.capacity_delta.is_none() =>
                {
                    (full_fence || no_fence) && raw.completed_at_unix_ms.is_some()
                }
                _ => full_fence && raw.completed_at_unix_ms.is_some(),
            };
            if shape { Ok(()) } else { Err(corrupt()) }
        }
        _ => Err(corrupt()),
    }
}

struct GraphPreflight {
    item_total: u16,
    path_total: u16,
    evidence_total: u16,
    item_status_counts: StoredCleanupStatusCounts,
    path_status_counts: StoredCleanupStatusCounts,
}

fn preflight_graph(
    connection: &Connection,
    parent: &RawParent,
) -> Result<GraphPreflight, HistoryError> {
    let id = parent.session_id.as_str();
    let (item_count, item_bad, item_bytes, item_min, item_max): (
        i64,
        i64,
        i64,
        Option<i64>,
        Option<i64>,
    ) = connection
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN
                        typeof(item_ordinal) = 'integer' AND
                        typeof(rule_id) = 'text' AND length(CAST(rule_id AS BLOB)) BETWEEN 1 AND 128 AND
                        typeof(rule_revision) = 'integer' AND rule_revision > 0 AND
                        typeof(estimated_bytes) = 'integer' AND estimated_bytes >= 0 AND
                        typeof(final_status) = 'text' AND length(CAST(final_status AS BLOB)) BETWEEN 1 AND 64 AND
                        typeof(error_category) IN ('text', 'null') AND
                        (error_category IS NULL OR
                            (?2 = 1 AND length(error_category) BETWEEN 1 AND 128) OR
                            (?2 = 2 AND length(CAST(error_category AS BLOB)) BETWEEN 1 AND 128)) AND
                        typeof(record_format_version) = 'integer' AND record_format_version = ?2 AND
                        typeof(legacy_target_path) IN ('blob', 'null') AND
                        typeof(legacy_target_path_encoding) IN ('integer', 'null') AND
                        typeof(candidate_id) IN ('text', 'null') AND
                        (candidate_id IS NULL OR length(CAST(candidate_id AS BLOB)) BETWEEN 1 AND 128) AND
                        typeof(category) IN ('text', 'null') AND
                        typeof(safety_tier) IN ('text', 'null') AND
                        typeof(proposed_action) IN ('text', 'null') AND
                        typeof(rule_schedule_eligible) IN ('integer', 'null') AND
                        typeof(newest_mtime_unix_seconds) IN ('integer', 'null') AND
                        typeof(newest_mtime_nanoseconds) IN ('integer', 'null') AND
                        ((?2 = 1 AND typeof(legacy_target_path) = 'blob' AND
                            ((legacy_target_path_encoding = 1 AND length(legacy_target_path) BETWEEN 1 AND 32768) OR
                             (legacy_target_path_encoding = 2 AND length(legacy_target_path) BETWEEN 2 AND 65536 AND length(legacy_target_path) % 2 = 0)) AND
                            candidate_id IS NULL AND category IS NULL AND safety_tier IS NULL AND
                            proposed_action IS NULL AND rule_schedule_eligible IS NULL AND
                            newest_mtime_unix_seconds IS NULL AND newest_mtime_nanoseconds IS NULL) OR
                         (?2 = 2 AND legacy_target_path IS NULL AND legacy_target_path_encoding IS NULL AND
                            typeof(candidate_id) = 'text' AND length(CAST(candidate_id AS BLOB)) BETWEEN 1 AND 128 AND
                            category IN ('developer_artifact', 'application_cache', 'browser_cache',
                                'log_and_diagnostic', 'installer_and_download', 'device_and_simulator_data',
                                'cloud_file', 'large_review_item', 'protected_system_data', 'unknown_storage') AND
                            safety_tier IN ('safe_regenerable', 'safe_evictable', 'review_required', 'informational', 'protected') AND
                            proposed_action IN ('remove_known_regenerable_contents', 'evict_local_copy', 'move_to_trash') AND
                            rule_schedule_eligible IN (0, 1) AND
                            ((newest_mtime_unix_seconds IS NULL AND newest_mtime_nanoseconds IS NULL) OR
                             (typeof(newest_mtime_unix_seconds) = 'integer' AND newest_mtime_unix_seconds >= 0 AND
                              typeof(newest_mtime_nanoseconds) = 'integer' AND newest_mtime_nanoseconds BETWEEN 0 AND 999999999))))
                    THEN 0 ELSE 1 END), 0),
                    COALESCE(SUM(
                        length(CAST(rule_id AS BLOB)) +
                        COALESCE(length(CAST(final_status AS BLOB)), 0) +
                        COALESCE(length(CAST(error_category AS BLOB)), 0) +
                        COALESCE(length(legacy_target_path), 0) +
                        COALESCE(length(CAST(candidate_id AS BLOB)), 0) +
                        COALESCE(length(CAST(category AS BLOB)), 0) +
                        COALESCE(length(CAST(safety_tier AS BLOB)), 0) +
                        COALESCE(length(CAST(proposed_action AS BLOB)), 0)
                    ), 0),
                    MIN(item_ordinal), MAX(item_ordinal)
             FROM cleanup_items WHERE session_id = ?1",
            params![id, parent.version],
            |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            },
        )
        .map_err(map_query_sql_error)?;
    let item_estimated_bytes = load_item_estimated_bytes(connection, id)?;
    if !(0..=MAX_ITEMS).contains(&item_count)
        || (parent.version == 2 && item_count == 0)
        || item_bad != 0
        || (item_count == 0 && (item_min.is_some() || item_max.is_some()))
        || (item_count > 0 && (item_min != Some(0) || item_max != Some(item_count - 1)))
        || (parent.version == 2 && item_estimated_bytes != parent.estimated_bytes)
    {
        return Err(corrupt());
    }
    let paths = aggregate_child(
        connection,
        id,
        "cleanup_item_paths",
        "typeof(item_ordinal) = 'integer' AND typeof(path_ordinal) = 'integer' AND typeof(target_path) = 'blob' AND length(target_path) BETWEEN 1 AND 65536 AND typeof(target_path_encoding) = 'integer' AND typeof(attempt_generation) IN ('integer', 'null') AND typeof(status) = 'text' AND length(CAST(status AS BLOB)) BETWEEN 1 AND 64 AND typeof(error_category) IN ('text', 'null') AND (error_category IS NULL OR length(CAST(error_category AS BLOB)) BETWEEN 1 AND 128) AND typeof(effect_started_at_unix_ms) IN ('integer', 'null') AND typeof(completed_at_unix_ms) IN ('integer', 'null')",
        "length(target_path) + length(CAST(status AS BLOB)) + COALESCE(length(CAST(error_category AS BLOB)), 0)",
    )?;
    let evidence = aggregate_child(
        connection,
        id,
        "cleanup_item_evidence",
        "typeof(item_ordinal) = 'integer' AND typeof(evidence_ordinal) = 'integer' AND typeof(evidence_kind) = 'text' AND length(CAST(evidence_kind AS BLOB)) BETWEEN 1 AND 64 AND typeof(path_value) IN ('blob', 'null') AND (path_value IS NULL OR length(path_value) BETWEEN 1 AND 65536) AND typeof(path_value_encoding) IN ('integer', 'null') AND typeof(text_value) IN ('text', 'null') AND (text_value IS NULL OR length(CAST(text_value AS BLOB)) BETWEEN 1 AND 4096) AND typeof(observed_unix_seconds) IN ('integer', 'null') AND typeof(observed_nanoseconds) IN ('integer', 'null') AND typeof(duration_seconds) IN ('integer', 'null') AND typeof(duration_nanoseconds) IN ('integer', 'null') AND typeof(observed_bytes) IN ('integer', 'null') AND typeof(minimum_bytes) IN ('integer', 'null')",
        "length(CAST(evidence_kind AS BLOB)) + COALESCE(length(path_value), 0) + COALESCE(length(CAST(text_value AS BLOB)), 0)",
    )?;
    let warnings = aggregate_child(
        connection,
        id,
        "cleanup_plan_warnings",
        "typeof(warning_ordinal) = 'integer' AND typeof(warning_kind) = 'text' AND length(CAST(warning_kind AS BLOB)) BETWEEN 1 AND 128",
        "length(CAST(warning_kind AS BLOB))",
    )?;
    let claims = aggregate_child(
        connection,
        id,
        "candidate_plan_claims",
        "typeof(item_ordinal) = 'integer' AND typeof(candidate_id) = 'text' AND length(CAST(candidate_id AS BLOB)) BETWEEN 1 AND 128 AND typeof(prior_review_status) = 'text' AND length(CAST(prior_review_status AS BLOB)) BETWEEN 1 AND 64",
        "length(CAST(candidate_id AS BLOB)) + length(CAST(prior_review_status AS BLOB))",
    )?;
    let active_claims_required = matches!(
        parent.status,
        StoredCleanupSessionStatus::Planned
            | StoredCleanupSessionStatus::Running
            | StoredCleanupSessionStatus::Recovering
    );
    if [paths.bad, evidence.bad, warnings.bad, claims.bad]
        .into_iter()
        .any(|bad| bad != 0)
        || paths.count > MAX_PATHS
        || evidence.count > MAX_EVIDENCE
        || warnings.count > MAX_WARNINGS
        || claims.count > MAX_ITEMS
        || (parent.version == 1
            && (paths.count != 0
                || evidence.count != 0
                || warnings.count != 0
                || claims.count != 0))
        || (parent.version == 2 && (paths.count < item_count || evidence.count < item_count))
        || (parent.coupling == 1 && claims.count != 0)
        || (parent.coupling == 2
            && ((active_claims_required && claims.count != item_count)
                || (!active_claims_required && claims.count != 0)))
    {
        return Err(corrupt());
    }
    if parent.version == 2 {
        let Some(scan_id) = parent.source_scan_id.as_ref() else {
            return Err(corrupt());
        };
        let scan_exists: i64 = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM scans WHERE scan_id = ?1)",
                [scan_id.as_str()],
                |row| row.get(0),
            )
            .map_err(map_query_sql_error)?;
        if scan_exists != 1 {
            return Err(corrupt());
        }
        validate_v2_graph_relationships(connection, parent, scan_id)?;
        validate_cleanup_journal_scalar_state_within_budget(connection, &parent.session_id)?;
    }
    let payload_bytes = [
        item_bytes,
        paths.bytes,
        evidence.bytes,
        warnings.bytes,
        claims.bytes,
    ]
    .into_iter()
    .try_fold(0_u64, |total, value| {
        let value = u64::try_from(value).map_err(|_| corrupt())?;
        total.checked_add(value).ok_or_else(corrupt)
    })?;
    let materialized_bytes = payload_bytes
        .checked_mul(MATERIALIZED_PAYLOAD_MULTIPLIER)
        .and_then(|total| {
            total.checked_add(
                u64::try_from(item_count)
                    .ok()?
                    .checked_mul(MATERIALIZED_ITEM_CHARGE)?,
            )
        })
        .and_then(|total| {
            total.checked_add(
                u64::try_from(paths.count)
                    .ok()?
                    .checked_mul(MATERIALIZED_PATH_CHARGE)?,
            )
        })
        .and_then(|total| {
            total.checked_add(
                u64::try_from(evidence.count)
                    .ok()?
                    .checked_mul(MATERIALIZED_EVIDENCE_CHARGE)?,
            )
        })
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
    if materialized_bytes > MAX_GRAPH_MATERIALIZATION_BYTES {
        return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
    }
    let item_status_counts = load_status_counts(connection, "cleanup_items", id, item_count)?;
    let path_status_counts = if parent.version == 1 {
        item_status_counts
    } else {
        load_status_counts(connection, "cleanup_item_paths", id, paths.count)?
    };
    Ok(GraphPreflight {
        item_total: u16::try_from(item_count).map_err(|_| corrupt())?,
        path_total: u16::try_from(if parent.version == 1 {
            item_count
        } else {
            paths.count
        })
        .map_err(|_| corrupt())?,
        evidence_total: u16::try_from(evidence.count).map_err(|_| corrupt())?,
        item_status_counts,
        path_status_counts,
    })
}

fn load_item_estimated_bytes(connection: &Connection, id: &str) -> Result<u64, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT estimated_bytes FROM cleanup_items
             WHERE session_id = ?1 ORDER BY item_ordinal LIMIT 65",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement.query([id]).map_err(map_query_sql_error)?;
    let mut total = 0_u64;
    let mut count = 0_usize;
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if count >= usize::try_from(MAX_ITEMS).map_err(|_| corrupt())? {
            return Err(corrupt());
        }
        let value = u64::try_from(row.get::<_, i64>(0).map_err(map_query_sql_error)?)
            .map_err(|_| corrupt())?;
        total = total.checked_add(value).ok_or_else(corrupt)?;
        count += 1;
    }
    Ok(total)
}

/// Validate the relationships needed to make scalar recent-page totals honest
/// without copying path or evidence payloads out of SQLite. The journal-owned
/// scalar pass validates dynamic lifecycle; exact lookup additionally validates
/// complete frozen payload semantics.
fn validate_v2_graph_relationships(
    connection: &Connection,
    parent: &RawParent,
    scan_id: &ScanId,
) -> Result<(), HistoryError> {
    let invalid: i64 = connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM cleanup_items AS item
                 WHERE item.session_id = ?1 AND item.record_format_version = 2
                   AND (NOT EXISTS (
                           SELECT 1 FROM cleanup_item_paths AS path
                           WHERE path.session_id = item.session_id
                             AND path.item_ordinal = item.item_ordinal
                       ) OR NOT EXISTS (
                           SELECT 1 FROM cleanup_item_evidence AS evidence
                           WHERE evidence.session_id = item.session_id
                             AND evidence.item_ordinal = item.item_ordinal
                       ))
                 UNION ALL
                 SELECT 1 FROM cleanup_item_paths AS child
                 WHERE child.session_id = ?1 AND NOT EXISTS (
                     SELECT 1 FROM cleanup_items AS item
                     WHERE item.session_id = child.session_id
                       AND item.item_ordinal = child.item_ordinal
                       AND item.record_format_version = 2
                 )
                 UNION ALL
                 SELECT 1 FROM cleanup_item_evidence AS child
                 WHERE child.session_id = ?1 AND NOT EXISTS (
                     SELECT 1 FROM cleanup_items AS item
                     WHERE item.session_id = child.session_id
                       AND item.item_ordinal = child.item_ordinal
                       AND item.record_format_version = 2
                 )
                 UNION ALL
                 SELECT 1 FROM cleanup_item_paths
                 WHERE session_id = ?1 GROUP BY item_ordinal
                 HAVING MIN(path_ordinal) != 0 OR MAX(path_ordinal) != COUNT(*) - 1
                 UNION ALL
                 SELECT 1 FROM cleanup_item_evidence
                 WHERE session_id = ?1 GROUP BY item_ordinal
                 HAVING MIN(evidence_ordinal) != 0 OR MAX(evidence_ordinal) != COUNT(*) - 1
                 UNION ALL
                 SELECT 1 FROM cleanup_plan_warnings
                 WHERE session_id = ?1 GROUP BY session_id
                 HAVING MIN(warning_ordinal) != 0 OR MAX(warning_ordinal) != COUNT(*) - 1
                     OR COUNT(DISTINCT warning_kind) != COUNT(*)
                 UNION ALL
                 SELECT 1 FROM candidate_plan_claims AS claim
                 LEFT JOIN cleanup_items AS item
                   ON item.session_id = claim.session_id
                  AND item.item_ordinal = claim.item_ordinal
                 LEFT JOIN candidates AS candidate
                   ON candidate.candidate_id = claim.candidate_id
                 WHERE claim.session_id = ?1 AND (
                     item.item_id IS NULL OR item.record_format_version != 2
                     OR item.candidate_id IS NULL OR claim.candidate_id != item.candidate_id
                     OR candidate.candidate_id IS NULL OR candidate.record_format_version != 2
                     OR candidate.scan_id != ?2 OR candidate.status != 'planned'
                 )
                 LIMIT 1
             )",
            params![parent.session_id.as_str(), scan_id.as_str()],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    if invalid != 0 {
        return Err(corrupt());
    }
    Ok(())
}

struct Aggregate {
    count: i64,
    bad: i64,
    bytes: i64,
}

fn aggregate_child(
    connection: &Connection,
    id: &str,
    table: &'static str,
    valid: &'static str,
    byte_expression: &'static str,
) -> Result<Aggregate, HistoryError> {
    let sql = format!(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN {valid} THEN 0 ELSE 1 END), 0),
                COALESCE(SUM({byte_expression}), 0)
         FROM {table} WHERE session_id = ?1"
    );
    connection
        .query_row(&sql, [id], |row| {
            Ok(Aggregate {
                count: row.get(0)?,
                bad: row.get(1)?,
                bytes: row.get(2)?,
            })
        })
        .map_err(map_query_sql_error)
}

fn load_status_counts(
    connection: &Connection,
    table: &'static str,
    id: &str,
    expected: i64,
) -> Result<StoredCleanupStatusCounts, HistoryError> {
    let status_column = if table == "cleanup_items" {
        "final_status"
    } else {
        "status"
    };
    let sql = format!(
        "SELECT {status_column}, COUNT(*) FROM {table}
         WHERE session_id = ?1 GROUP BY {status_column} LIMIT 16"
    );
    let mut statement = connection.prepare(&sql).map_err(map_query_sql_error)?;
    let mut rows = statement.query([id]).map_err(map_query_sql_error)?;
    let mut counts = StoredCleanupStatusCounts::default();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        let status =
            item_status_from_stored(&row.get::<_, String>(0).map_err(map_query_sql_error)?)?;
        let count = u16::try_from(row.get::<_, i64>(1).map_err(map_query_sql_error)?)
            .map_err(|_| corrupt())?;
        counts.total = counts.total.checked_add(count).ok_or_else(corrupt)?;
        add_status_count(&mut counts, status, count)?;
    }
    if i64::from(counts.total) != expected {
        return Err(corrupt());
    }
    Ok(counts)
}

fn add_status_count(
    counts: &mut StoredCleanupStatusCounts,
    status: StoredCleanupItemStatus,
    count: u16,
) -> Result<(), HistoryError> {
    let field = match status {
        StoredCleanupItemStatus::Planned => &mut counts.planned,
        StoredCleanupItemStatus::Validating => &mut counts.validating,
        StoredCleanupItemStatus::DryRun => &mut counts.dry_run,
        StoredCleanupItemStatus::EffectStarted => &mut counts.effect_started,
        StoredCleanupItemStatus::Trashed => &mut counts.trashed,
        StoredCleanupItemStatus::Removed => &mut counts.removed,
        StoredCleanupItemStatus::Evicted => &mut counts.evicted,
        StoredCleanupItemStatus::Skipped => &mut counts.skipped,
        StoredCleanupItemStatus::Rejected => &mut counts.rejected,
        StoredCleanupItemStatus::Failed => &mut counts.failed,
        StoredCleanupItemStatus::ChangedSincePlan => &mut counts.changed_since_plan,
        StoredCleanupItemStatus::Interrupted => &mut counts.interrupted,
        StoredCleanupItemStatus::Unavailable => &mut counts.unavailable,
        StoredCleanupItemStatus::OutcomeUnknown => &mut counts.outcome_unknown,
    };
    *field = field.checked_add(count).ok_or_else(corrupt)?;
    Ok(())
}

fn project_legacy(
    record: super::cleanup_history::LegacyCleanupSessionSummary,
) -> StoredCleanupHistoryObservation {
    let items = record
        .items
        .iter()
        .map(project_legacy_item)
        .collect::<Vec<_>>();
    let counts = counts_for_items(&items);
    StoredCleanupHistoryObservation {
        summary: StoredCleanupSessionSummary {
            session_id: record.session_id,
            plan_id: record.plan_id,
            format: StoredCleanupRecordFormat::LegacyIncomplete,
            source_scan_id: None,
            started_at: record.started_at,
            completed_at: record.completed_at,
            plan_created_at: None,
            plan_expires_at: None,
            mode: mode(record.mode),
            estimated_bytes: record.estimated_bytes,
            verified_capacity_delta_bytes: record.verified_capacity_delta_bytes,
            trigger: trigger(record.trigger),
            status: legacy_session_status(record.status),
            cancellation_requested: None,
            item_total: counts.total,
            path_total: counts.total,
            evidence_total: 0,
            item_status_counts: counts,
            path_status_counts: counts,
        },
        items,
        warnings: Vec::new(),
    }
}

fn project_legacy_item(item: &LegacyCleanupItemSummary) -> StoredCleanupItemSummary {
    StoredCleanupItemSummary {
        ordinal: item.ordinal as u32,
        rule: item.rule.clone(),
        category: None,
        safety: None,
        action: None,
        rule_schedule_eligible: None,
        newest_mtime: None,
        estimated_bytes: item.estimated_bytes,
        status: legacy_item_status(item.status),
        error_recorded: item.error_category.is_some(),
        error_category: None,
        path_count: 1,
        evidence_count: 0,
    }
}

fn project_journal(journal: CleanupJournal) -> StoredCleanupHistoryObservation {
    let items = journal
        .items
        .iter()
        .map(project_journal_item)
        .collect::<Vec<_>>();
    let (status, completed_at, delta, cancellation_requested) = match journal.lifecycle {
        JournalLifecycle::Planned => (StoredCleanupSessionStatus::Planned, None, None, Some(false)),
        JournalLifecycle::Active {
            phase,
            cancellation_requested,
            ..
        } => (
            match phase {
                ActivePhase::Running => StoredCleanupSessionStatus::Running,
                ActivePhase::Recovering => StoredCleanupSessionStatus::Recovering,
            },
            None,
            None,
            Some(cancellation_requested),
        ),
        JournalLifecycle::Terminal {
            status,
            completed_at,
            verified_capacity_delta_bytes,
            cancellation_requested,
            ..
        } => (
            terminal_session_status(status),
            Some(completed_at),
            verified_capacity_delta_bytes,
            Some(cancellation_requested),
        ),
        JournalLifecycle::ObservedTerminal {
            status,
            completed_at,
            cancellation_requested,
        } => (
            terminal_session_status(status),
            Some(completed_at),
            None,
            Some(cancellation_requested),
        ),
    };
    let item_status_counts = counts_for_items(&items);
    let path_status_counts = counts_for_path_statuses(&journal.items);
    let path_total = path_status_counts.total;
    let evidence_total = journal
        .items
        .iter()
        .map(|item| item.frozen.evidence.len() as u16)
        .sum();
    StoredCleanupHistoryObservation {
        summary: StoredCleanupSessionSummary {
            session_id: journal.session_id,
            plan_id: journal.plan_id,
            format: StoredCleanupRecordFormat::CompleteV2,
            source_scan_id: Some(journal.source_scan_id),
            started_at: journal.started_at,
            completed_at,
            plan_created_at: Some(journal.plan_created_at),
            plan_expires_at: Some(journal.plan_expires_at),
            mode: mode(journal.mode),
            estimated_bytes: journal.estimated_bytes,
            verified_capacity_delta_bytes: delta,
            trigger: trigger(journal.trigger),
            status,
            cancellation_requested,
            item_total: item_status_counts.total,
            path_total,
            evidence_total,
            item_status_counts,
            path_status_counts,
        },
        items,
        warnings: journal.warnings,
    }
}

fn project_journal_item(item: &JournalItem) -> StoredCleanupItemSummary {
    StoredCleanupItemSummary {
        ordinal: item.frozen.ordinal as u32,
        rule: item.frozen.rule.clone(),
        category: Some(item.frozen.category),
        safety: Some(item.frozen.safety),
        action: Some(item.frozen.proposed_action),
        rule_schedule_eligible: Some(item.frozen.rule_schedule_eligible),
        newest_mtime: item.frozen.newest_mtime,
        estimated_bytes: item.frozen.estimated_bytes,
        status: path_status(item.status),
        error_recorded: item.error_category.is_some(),
        error_category: item.error_category.as_deref().map(error_category),
        path_count: item.paths.len() as u16,
        evidence_count: item.frozen.evidence.len() as u16,
    }
}

fn counts_for_items(items: &[StoredCleanupItemSummary]) -> StoredCleanupStatusCounts {
    let mut counts = StoredCleanupStatusCounts {
        total: items.len() as u16,
        ..StoredCleanupStatusCounts::default()
    };
    for item in items {
        add_status_count(&mut counts, item.status, 1).expect("bounded item count cannot overflow");
    }
    counts
}

fn counts_for_path_statuses(items: &[JournalItem]) -> StoredCleanupStatusCounts {
    let mut counts = StoredCleanupStatusCounts::default();
    for path in items.iter().flat_map(|item| &item.paths) {
        counts.total += 1;
        add_status_count(&mut counts, path_status(path.status), 1)
            .expect("bounded path count cannot overflow");
    }
    counts
}

fn mode(value: CleanupMode) -> StoredCleanupMode {
    match value {
        CleanupMode::DryRun => StoredCleanupMode::DryRun,
        CleanupMode::Trash => StoredCleanupMode::Trash,
        CleanupMode::PermanentSafe => StoredCleanupMode::PermanentSafe,
        CleanupMode::EvictLocalCopy => StoredCleanupMode::EvictLocalCopy,
    }
}

fn mode_from_stored(value: &str) -> Result<StoredCleanupMode, HistoryError> {
    match value {
        "dry_run" => Ok(StoredCleanupMode::DryRun),
        "trash" => Ok(StoredCleanupMode::Trash),
        "permanent_safe" => Ok(StoredCleanupMode::PermanentSafe),
        "evict_local_copy" => Ok(StoredCleanupMode::EvictLocalCopy),
        _ => Err(corrupt()),
    }
}

fn trigger(value: CleanupTrigger) -> StoredCleanupTrigger {
    match value {
        CleanupTrigger::Manual => StoredCleanupTrigger::Manual,
        CleanupTrigger::LowDisk => StoredCleanupTrigger::LowDisk,
        CleanupTrigger::Scheduled => StoredCleanupTrigger::Scheduled,
        CleanupTrigger::Cli => StoredCleanupTrigger::Cli,
    }
}

fn trigger_from_stored(value: &str) -> Result<StoredCleanupTrigger, HistoryError> {
    match value {
        "manual" => Ok(StoredCleanupTrigger::Manual),
        "low_disk" => Ok(StoredCleanupTrigger::LowDisk),
        "scheduled" => Ok(StoredCleanupTrigger::Scheduled),
        "cli" => Ok(StoredCleanupTrigger::Cli),
        _ => Err(corrupt()),
    }
}

fn session_status_from_stored(value: &str) -> Result<StoredCleanupSessionStatus, HistoryError> {
    match value {
        "planned" => Ok(StoredCleanupSessionStatus::Planned),
        "running" => Ok(StoredCleanupSessionStatus::Running),
        "recovering" => Ok(StoredCleanupSessionStatus::Recovering),
        "completed" => Ok(StoredCleanupSessionStatus::Completed),
        "partially_completed" => Ok(StoredCleanupSessionStatus::PartiallyCompleted),
        "failed" => Ok(StoredCleanupSessionStatus::Failed),
        "cancelled" => Ok(StoredCleanupSessionStatus::Cancelled),
        "interrupted" => Ok(StoredCleanupSessionStatus::Interrupted),
        "rejected" => Ok(StoredCleanupSessionStatus::Rejected),
        "dry_run" => Ok(StoredCleanupSessionStatus::DryRun),
        _ => Err(corrupt()),
    }
}

fn item_status_from_stored(value: &str) -> Result<StoredCleanupItemStatus, HistoryError> {
    match value {
        "planned" => Ok(StoredCleanupItemStatus::Planned),
        "validating" => Ok(StoredCleanupItemStatus::Validating),
        "dry_run" => Ok(StoredCleanupItemStatus::DryRun),
        "effect_started" => Ok(StoredCleanupItemStatus::EffectStarted),
        "trashed" => Ok(StoredCleanupItemStatus::Trashed),
        "removed" => Ok(StoredCleanupItemStatus::Removed),
        "evicted" => Ok(StoredCleanupItemStatus::Evicted),
        "skipped" => Ok(StoredCleanupItemStatus::Skipped),
        "rejected" => Ok(StoredCleanupItemStatus::Rejected),
        "failed" => Ok(StoredCleanupItemStatus::Failed),
        "changed_since_plan" => Ok(StoredCleanupItemStatus::ChangedSincePlan),
        "interrupted" => Ok(StoredCleanupItemStatus::Interrupted),
        "unavailable" => Ok(StoredCleanupItemStatus::Unavailable),
        "outcome_unknown" => Ok(StoredCleanupItemStatus::OutcomeUnknown),
        _ => Err(corrupt()),
    }
}

fn legacy_session_status(value: LegacySessionStatus) -> StoredCleanupSessionStatus {
    match value {
        LegacySessionStatus::Planned => StoredCleanupSessionStatus::Planned,
        LegacySessionStatus::Running => StoredCleanupSessionStatus::Running,
        LegacySessionStatus::Completed => StoredCleanupSessionStatus::Completed,
        LegacySessionStatus::PartiallyCompleted => StoredCleanupSessionStatus::PartiallyCompleted,
        LegacySessionStatus::Failed => StoredCleanupSessionStatus::Failed,
        LegacySessionStatus::Cancelled => StoredCleanupSessionStatus::Cancelled,
        LegacySessionStatus::Interrupted => StoredCleanupSessionStatus::Interrupted,
        LegacySessionStatus::Rejected => StoredCleanupSessionStatus::Rejected,
        LegacySessionStatus::DryRun => StoredCleanupSessionStatus::DryRun,
    }
}

fn terminal_session_status(value: TerminalSessionStatus) -> StoredCleanupSessionStatus {
    match value {
        TerminalSessionStatus::Completed => StoredCleanupSessionStatus::Completed,
        TerminalSessionStatus::PartiallyCompleted => StoredCleanupSessionStatus::PartiallyCompleted,
        TerminalSessionStatus::Failed => StoredCleanupSessionStatus::Failed,
        TerminalSessionStatus::Cancelled => StoredCleanupSessionStatus::Cancelled,
        TerminalSessionStatus::Interrupted => StoredCleanupSessionStatus::Interrupted,
        TerminalSessionStatus::Rejected => StoredCleanupSessionStatus::Rejected,
        TerminalSessionStatus::DryRun => StoredCleanupSessionStatus::DryRun,
    }
}

fn legacy_item_status(value: LegacyItemStatus) -> StoredCleanupItemStatus {
    match value {
        LegacyItemStatus::Planned => StoredCleanupItemStatus::Planned,
        LegacyItemStatus::DryRun => StoredCleanupItemStatus::DryRun,
        LegacyItemStatus::Trashed => StoredCleanupItemStatus::Trashed,
        LegacyItemStatus::Removed => StoredCleanupItemStatus::Removed,
        LegacyItemStatus::Evicted => StoredCleanupItemStatus::Evicted,
        LegacyItemStatus::Skipped => StoredCleanupItemStatus::Skipped,
        LegacyItemStatus::Rejected => StoredCleanupItemStatus::Rejected,
        LegacyItemStatus::Failed => StoredCleanupItemStatus::Failed,
        LegacyItemStatus::ChangedSincePlan => StoredCleanupItemStatus::ChangedSincePlan,
        LegacyItemStatus::Interrupted => StoredCleanupItemStatus::Interrupted,
        LegacyItemStatus::Unavailable => StoredCleanupItemStatus::Unavailable,
    }
}

fn path_status(value: PathStatus) -> StoredCleanupItemStatus {
    match value {
        PathStatus::Planned => StoredCleanupItemStatus::Planned,
        PathStatus::Validating => StoredCleanupItemStatus::Validating,
        PathStatus::DryRun => StoredCleanupItemStatus::DryRun,
        PathStatus::EffectStarted => StoredCleanupItemStatus::EffectStarted,
        PathStatus::Trashed => StoredCleanupItemStatus::Trashed,
        PathStatus::Removed => StoredCleanupItemStatus::Removed,
        PathStatus::Evicted => StoredCleanupItemStatus::Evicted,
        PathStatus::Skipped => StoredCleanupItemStatus::Skipped,
        PathStatus::Rejected => StoredCleanupItemStatus::Rejected,
        PathStatus::Failed => StoredCleanupItemStatus::Failed,
        PathStatus::ChangedSincePlan => StoredCleanupItemStatus::ChangedSincePlan,
        PathStatus::Interrupted => StoredCleanupItemStatus::Interrupted,
        PathStatus::Unavailable => StoredCleanupItemStatus::Unavailable,
        PathStatus::OutcomeUnknown => StoredCleanupItemStatus::OutcomeUnknown,
    }
}

fn error_category(value: &str) -> StoredCleanupErrorCategory {
    StoredCleanupErrorCategory(value.to_owned())
}

fn decode_optional_seconds_nanos(
    seconds: Option<i64>,
    nanoseconds: Option<i64>,
) -> Result<Option<SystemTime>, HistoryError> {
    use std::time::{Duration, UNIX_EPOCH};
    match (seconds, nanoseconds) {
        (None, None) => Ok(None),
        (Some(seconds), Some(nanoseconds)) => {
            let seconds = u64::try_from(seconds).map_err(|_| corrupt())?;
            let nanoseconds = u32::try_from(nanoseconds)
                .ok()
                .filter(|value| *value < 1_000_000_000)
                .ok_or_else(corrupt)?;
            UNIX_EPOCH
                .checked_add(Duration::new(seconds, nanoseconds))
                .map(Some)
                .ok_or_else(corrupt)
        }
        _ => Err(corrupt()),
    }
}

fn invalid() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidInput)
}

fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}
