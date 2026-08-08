use rusqlite::{Connection, OptionalExtension, params};

use super::*;

pub(super) fn load_cleanup_journal(
    connection: &Connection,
    session_id: &CleanupSessionId,
) -> Result<Option<CleanupJournal>, HistoryError> {
    run_bounded_query(connection, || {
        load_cleanup_journal_within_budget(connection, session_id)
    })
}

pub(in crate::persistence) fn load_cleanup_journal_within_budget(
    connection: &Connection,
    session_id: &CleanupSessionId,
) -> Result<Option<CleanupJournal>, HistoryError> {
    // Decide the candidate-retention requirement without materializing any
    // dynamic text. This keeps a maximum-size pristine journal to one frozen
    // graph pass under the shared query budget.
    let require_candidate_match: Option<i64> = connection
        .query_row(
            "SELECT CASE WHEN status = 'planned' THEN 1 ELSE 0 END
             FROM cleanup_sessions WHERE session_id = ?1",
            [session_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    let Some(require_candidate_match) = require_candidate_match else {
        return Ok(None);
    };
    let require_candidate_match = decode_bool(require_candidate_match)?;
    // Validate every static field's SQLite storage class and byte bound before
    // the state-only queries below materialize those values again.
    let Some(frozen) =
        load_frozen_cleanup_session_within_budget(connection, session_id, require_candidate_match)?
    else {
        return Ok(None);
    };
    let raw = connection
        .query_row(
            "SELECT record_format_version, plan_id, started_at_unix_ms,
                    completed_at_unix_ms, mode, estimated_bytes,
                    verified_capacity_delta_bytes, trigger_source, status,
                    source_scan_id, plan_created_at_unix_seconds,
                    plan_created_at_nanoseconds, plan_expires_at_unix_seconds,
                    plan_expires_at_nanoseconds, execution_owner_id,
                    execution_generation, last_heartbeat_at_unix_ms,
                    cancellation_requested,
                    execution_host_identity_v1_sha256,
                    execution_boot_scope_v1_sha256,
                    execution_recovery_policy
             FROM cleanup_sessions WHERE session_id = ?1",
            [session_id.as_str()],
            |row| {
                Ok(RawSession {
                    version: row.get(0)?,
                    plan_id: row.get(1)?,
                    started_ms: row.get(2)?,
                    completed_ms: row.get(3)?,
                    mode: row.get(4)?,
                    estimated_bytes: row.get(5)?,
                    capacity_delta: row.get(6)?,
                    trigger: row.get(7)?,
                    status: row.get(8)?,
                    source_scan_id: row.get(9)?,
                    plan_created_seconds: row.get(10)?,
                    plan_created_nanos: row.get(11)?,
                    plan_expires_seconds: row.get(12)?,
                    plan_expires_nanos: row.get(13)?,
                    owner: row.get(14)?,
                    generation: row.get(15)?,
                    heartbeat_ms: row.get(16)?,
                    cancellation_requested: row.get(17)?,
                    host_identity: row.get(18)?,
                    boot_scope: row.get(19)?,
                    recovery_policy: row.get(20)?,
                })
            },
        )
        .optional()
        .map_err(map_query_sql_error)?;
    let Some(raw) = raw else { return Ok(None) };
    if raw.version != 2 {
        return Err(corrupt());
    }
    let started_at = unix_ms_to_system_time(raw.started_ms)?;
    let plan_id = CleanupPlanId::new(raw.plan_id.clone()).map_err(|_| corrupt())?;
    let source_scan_id = ScanId::new(raw.source_scan_id.clone()).map_err(|_| corrupt())?;
    let plan_created_at =
        decode_optional_time(Some(raw.plan_created_seconds), Some(raw.plan_created_nanos))?
            .ok_or_else(corrupt)?;
    let plan_expires_at =
        decode_optional_time(Some(raw.plan_expires_seconds), Some(raw.plan_expires_nanos))?
            .ok_or_else(corrupt)?;
    if plan_created_at > started_at
        || started_at >= plan_expires_at
        || plan_created_at.checked_add(CLEANUP_PLAN_VALIDITY) != Some(plan_expires_at)
    {
        return Err(corrupt());
    }
    let mode = mode_from_stored(&raw.mode)?;
    let trigger = trigger_from_stored(&raw.trigger)?;
    let estimated_bytes = from_i64(raw.estimated_bytes)?;
    let cancellation_requested = decode_bool(raw.cancellation_requested)?;
    let lifecycle = decode_lifecycle(session_id, &raw, cancellation_requested, started_at)?;
    let execution_provenance = decode_execution_provenance(
        raw.owner.as_deref(),
        raw.host_identity.as_deref(),
        raw.boot_scope.as_deref(),
        raw.recovery_policy.as_deref(),
    )?;
    if matches!(
        lifecycle,
        JournalLifecycle::Planned | JournalLifecycle::ObservedTerminal { .. }
    ) && execution_provenance.is_some()
    {
        return Err(corrupt());
    }
    if require_candidate_match != matches!(lifecycle, JournalLifecycle::Planned) {
        return Err(corrupt());
    }
    let mut totals = LoadTotals::default();
    let mut items = load_items(connection, session_id, &mut totals)?;
    let warnings = load_warnings(connection, session_id)?;

    if items.len() != frozen.items.len() {
        return Err(corrupt());
    }
    for (item, frozen_item) in items.iter_mut().zip(&frozen.items) {
        item.frozen.prior_review_status = frozen_item.prior_review_status;
    }

    let frozen_items = items
        .iter()
        .map(|item| item.frozen.clone())
        .collect::<Vec<_>>();
    if frozen.session_id != *session_id
        || frozen.plan_id != plan_id
        || frozen.started_at != started_at
        || frozen.source_scan_id != source_scan_id
        || frozen.plan_created_at != plan_created_at
        || frozen.plan_expires_at != plan_expires_at
        || frozen.mode != mode
        || frozen.estimated_bytes != estimated_bytes
        || frozen.trigger != trigger
        || frozen.items != frozen_items
        || frozen.warnings != warnings
        || items.is_empty()
        || items
            .iter()
            .map(|item| item.frozen.estimated_bytes)
            .try_fold(0_u64, |sum, bytes| sum.checked_add(bytes))
            != Some(estimated_bytes)
        || items
            .iter()
            .any(|item| !mode_accepts(mode, item.frozen.safety, item.frozen.proposed_action))
        || (trigger == CleanupTrigger::Scheduled
            && (mode != CleanupMode::PermanentSafe
                || items.iter().any(|item| !item.frozen.rule_schedule_eligible)))
        || (matches!(lifecycle, JournalLifecycle::ObservedTerminal { .. })
            && (mode != CleanupMode::DryRun
                || frozen.candidate_status_coupling != CandidateStatusCoupling::LegacyUncoupled))
        || cleanup_paths_overlap(&items)
    {
        return Err(corrupt());
    }
    validate_dynamic_graph(&lifecycle, mode, started_at, &items)?;
    Ok(Some(CleanupJournal {
        session_id: session_id.clone(),
        plan_id,
        started_at,
        source_scan_id,
        plan_created_at,
        plan_expires_at,
        mode,
        estimated_bytes,
        trigger,
        candidate_status_coupling: frozen.candidate_status_coupling,
        warnings,
        execution_provenance,
        lifecycle,
        items,
    }))
}

/// Validate only the bounded scalar state of a schema-v2 cleanup journal.
///
/// This is the path-free companion to [`load_cleanup_journal_within_budget`]
/// for recent-history projections. It deliberately does not select target
/// paths, evidence path/text fields, candidate identities, or other frozen
/// candidate payload. The caller must already own the surrounding query
/// budget; installing another progress handler here would replace that guard.
pub(in crate::persistence) fn validate_cleanup_journal_scalar_state_within_budget(
    connection: &Connection,
    session_id: &CleanupSessionId,
) -> Result<(), HistoryError> {
    let raw = connection
        .query_row(
            "SELECT record_format_version, started_at_unix_ms,
                    completed_at_unix_ms,
                    CASE WHEN typeof(mode) = 'text'
                              AND length(CAST(mode AS BLOB)) BETWEEN 1 AND 64
                         THEN mode END,
                    verified_capacity_delta_bytes,
                    CASE WHEN typeof(status) = 'text'
                              AND length(CAST(status AS BLOB)) BETWEEN 1 AND 64
                         THEN status END,
                    CASE WHEN typeof(execution_owner_id) = 'text'
                              AND length(CAST(execution_owner_id AS BLOB)) BETWEEN 1 AND 128
                         THEN execution_owner_id END,
                    execution_generation, last_heartbeat_at_unix_ms,
                    cancellation_requested,
                    execution_host_identity_v1_sha256,
                    execution_boot_scope_v1_sha256,
                    CASE WHEN typeof(execution_recovery_policy) = 'text'
                              AND length(CAST(execution_recovery_policy AS BLOB))
                                  BETWEEN 1 AND 32
                         THEN execution_recovery_policy END,
                    CASE WHEN
                        typeof(record_format_version) = 'integer' AND
                        typeof(started_at_unix_ms) = 'integer' AND
                        typeof(completed_at_unix_ms) IN ('integer', 'null') AND
                        typeof(mode) = 'text' AND length(CAST(mode AS BLOB)) BETWEEN 1 AND 64 AND
                        typeof(verified_capacity_delta_bytes) IN ('integer', 'null') AND
                        typeof(status) = 'text' AND length(CAST(status AS BLOB)) BETWEEN 1 AND 64 AND
                        typeof(execution_owner_id) IN ('text', 'null') AND
                        (execution_owner_id IS NULL OR
                            length(CAST(execution_owner_id AS BLOB)) BETWEEN 1 AND 128) AND
                        typeof(execution_generation) IN ('integer', 'null') AND
                        typeof(last_heartbeat_at_unix_ms) IN ('integer', 'null') AND
                        typeof(cancellation_requested) = 'integer' AND
                        typeof(execution_host_identity_v1_sha256) IN ('blob', 'null') AND
                        (execution_host_identity_v1_sha256 IS NULL OR
                            length(execution_host_identity_v1_sha256) = 32) AND
                        typeof(execution_boot_scope_v1_sha256) IN ('blob', 'null') AND
                        (execution_boot_scope_v1_sha256 IS NULL OR
                            length(execution_boot_scope_v1_sha256) = 32) AND
                        typeof(execution_recovery_policy) IN ('text', 'null') AND
                        (execution_recovery_policy IS NULL OR
                            length(CAST(execution_recovery_policy AS BLOB)) BETWEEN 1 AND 32)
                    THEN 0 ELSE 1 END
             FROM cleanup_sessions WHERE session_id = ?1",
            [session_id.as_str()],
            |row| {
                Ok(ScalarRawSession {
                    version: row.get(0)?,
                    started_ms: row.get(1)?,
                    completed_ms: row.get(2)?,
                    mode: row.get(3)?,
                    capacity_delta: row.get(4)?,
                    status: row.get(5)?,
                    owner: row.get(6)?,
                    generation: row.get(7)?,
                    heartbeat_ms: row.get(8)?,
                    cancellation_requested: row.get(9)?,
                    host_identity: row.get(10)?,
                    boot_scope: row.get(11)?,
                    recovery_policy: row.get(12)?,
                    invalid_storage: row.get(13)?,
                })
            },
        )
        .optional()
        .map_err(map_query_sql_error)?
        .ok_or_else(corrupt)?;
    if raw.version != 2 || raw.invalid_storage != 0 {
        return Err(corrupt());
    }
    let started_at = unix_ms_to_system_time(raw.started_ms)?;
    let mode = mode_from_stored(raw.mode.as_deref().ok_or_else(corrupt)?)?;
    let cancellation_requested = decode_bool(raw.cancellation_requested)?;
    let lifecycle = decode_lifecycle_fields(
        session_id,
        raw.lifecycle_fields()?,
        cancellation_requested,
        started_at,
    )?;
    let execution_provenance = decode_execution_provenance(
        raw.owner.as_deref(),
        raw.host_identity.as_deref(),
        raw.boot_scope.as_deref(),
        raw.recovery_policy.as_deref(),
    )?;
    if matches!(
        lifecycle,
        JournalLifecycle::Planned | JournalLifecycle::ObservedTerminal { .. }
    ) && execution_provenance.is_some()
    {
        return Err(corrupt());
    }
    let mut items = load_scalar_journal_items(connection, session_id)?;
    load_scalar_journal_paths(connection, session_id, &mut items)?;
    validate_dynamic_state(&lifecycle, mode, started_at, &items)
}

struct ScalarRawSession {
    version: i64,
    started_ms: i64,
    completed_ms: Option<i64>,
    mode: Option<String>,
    capacity_delta: Option<i64>,
    status: Option<String>,
    owner: Option<String>,
    generation: Option<i64>,
    heartbeat_ms: Option<i64>,
    cancellation_requested: i64,
    host_identity: Option<Vec<u8>>,
    boot_scope: Option<Vec<u8>>,
    recovery_policy: Option<String>,
    invalid_storage: i64,
}

impl ScalarRawSession {
    fn lifecycle_fields(&self) -> Result<RawLifecycleFields<'_>, HistoryError> {
        Ok(RawLifecycleFields {
            status: self.status.as_deref().ok_or_else(corrupt)?,
            owner: self.owner.as_deref(),
            generation: self.generation,
            heartbeat_ms: self.heartbeat_ms,
            completed_ms: self.completed_ms,
            capacity_delta: self.capacity_delta,
        })
    }
}

#[derive(Debug)]
struct ScalarJournalItem {
    ordinal: usize,
    action: CandidateAction,
    status: PathStatus,
    error_category: Option<String>,
    paths: Vec<ScalarJournalPath>,
}

#[derive(Debug)]
struct ScalarJournalPath {
    attempt_generation: Option<u64>,
    status: PathStatus,
    error_category: Option<String>,
    effect_started_at: Option<SystemTime>,
    completed_at: Option<SystemTime>,
}

fn load_scalar_journal_items(
    connection: &Connection,
    session_id: &CleanupSessionId,
) -> Result<Vec<ScalarJournalItem>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT item_ordinal,
                    CASE WHEN typeof(proposed_action) = 'text'
                              AND length(CAST(proposed_action AS BLOB)) BETWEEN 1 AND 64
                         THEN proposed_action END,
                    CASE WHEN typeof(final_status) = 'text'
                              AND length(CAST(final_status AS BLOB)) BETWEEN 1 AND 64
                         THEN final_status END,
                    CASE WHEN typeof(error_category) = 'text'
                              AND length(CAST(error_category AS BLOB)) BETWEEN 1 AND 128
                         THEN error_category END,
                    error_category IS NULL,
                    CASE WHEN
                        typeof(item_ordinal) = 'integer' AND
                        typeof(record_format_version) = 'integer' AND
                        record_format_version = 2 AND
                        typeof(proposed_action) = 'text' AND
                        length(CAST(proposed_action AS BLOB)) BETWEEN 1 AND 64 AND
                        typeof(final_status) = 'text' AND
                        length(CAST(final_status AS BLOB)) BETWEEN 1 AND 64 AND
                        typeof(error_category) IN ('text', 'null') AND
                        (error_category IS NULL OR
                            length(CAST(error_category AS BLOB)) BETWEEN 1 AND 128)
                    THEN 0 ELSE 1 END
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
        let ordinal = row.get::<_, i64>(0).map_err(map_query_sql_error)?;
        let action = row
            .get::<_, Option<String>>(1)
            .map_err(map_query_sql_error)?
            .ok_or_else(corrupt)?;
        let status = row
            .get::<_, Option<String>>(2)
            .map_err(map_query_sql_error)?
            .ok_or_else(corrupt)?;
        let error = row
            .get::<_, Option<String>>(3)
            .map_err(map_query_sql_error)?;
        let error_is_null = decode_bool(row.get(4).map_err(map_query_sql_error)?)?;
        let invalid_storage: i64 = row.get(5).map_err(map_query_sql_error)?;
        if ordinal != items.len() as i64 || invalid_storage != 0 || error_is_null != error.is_none()
        {
            return Err(corrupt());
        }
        validate_stored_error(error.as_deref())?;
        items.push(ScalarJournalItem {
            ordinal: items.len(),
            action: action_from_stored(&action)?,
            status: PathStatus::from_stored(&status)?,
            error_category: error,
            paths: Vec::new(),
        });
    }
    if items.is_empty() {
        return Err(corrupt());
    }
    Ok(items)
}

fn load_scalar_journal_paths(
    connection: &Connection,
    session_id: &CleanupSessionId,
    items: &mut [ScalarJournalItem],
) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT item_ordinal, path_ordinal, attempt_generation,
                    CASE WHEN typeof(status) = 'text'
                              AND length(CAST(status AS BLOB)) BETWEEN 1 AND 64
                         THEN status END,
                    CASE WHEN typeof(error_category) = 'text'
                              AND length(CAST(error_category AS BLOB)) BETWEEN 1 AND 128
                         THEN error_category END,
                    error_category IS NULL, effect_started_at_unix_ms,
                    completed_at_unix_ms,
                    CASE WHEN
                        typeof(item_ordinal) = 'integer' AND
                        typeof(path_ordinal) = 'integer' AND
                        typeof(attempt_generation) IN ('integer', 'null') AND
                        typeof(status) = 'text' AND
                        length(CAST(status AS BLOB)) BETWEEN 1 AND 64 AND
                        typeof(error_category) IN ('text', 'null') AND
                        (error_category IS NULL OR
                            length(CAST(error_category AS BLOB)) BETWEEN 1 AND 128) AND
                        typeof(effect_started_at_unix_ms) IN ('integer', 'null') AND
                        typeof(completed_at_unix_ms) IN ('integer', 'null')
                    THEN 0 ELSE 1 END
             FROM cleanup_item_paths WHERE session_id = ?1
             ORDER BY item_ordinal, path_ordinal LIMIT 257",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([session_id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut total = 0_usize;
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if total >= MAX_TOTAL_PATHS {
            return Err(corrupt());
        }
        let item_ordinal = row.get::<_, i64>(0).map_err(map_query_sql_error)?;
        let item_index = usize::try_from(item_ordinal).map_err(|_| corrupt())?;
        let item = items.get_mut(item_index).ok_or_else(corrupt)?;
        if item.ordinal != item_index {
            return Err(corrupt());
        }
        let path_ordinal = row.get::<_, i64>(1).map_err(map_query_sql_error)?;
        let status = row
            .get::<_, Option<String>>(3)
            .map_err(map_query_sql_error)?
            .ok_or_else(corrupt)?;
        let error = row
            .get::<_, Option<String>>(4)
            .map_err(map_query_sql_error)?;
        let error_is_null = decode_bool(row.get(5).map_err(map_query_sql_error)?)?;
        let invalid_storage: i64 = row.get(8).map_err(map_query_sql_error)?;
        if path_ordinal != item.paths.len() as i64
            || invalid_storage != 0
            || error_is_null != error.is_none()
        {
            return Err(corrupt());
        }
        validate_stored_error(error.as_deref())?;
        item.paths.push(ScalarJournalPath {
            attempt_generation: row
                .get::<_, Option<i64>>(2)
                .map_err(map_query_sql_error)?
                .map(decode_generation)
                .transpose()?,
            status: PathStatus::from_stored(&status)?,
            error_category: error,
            effect_started_at: row
                .get::<_, Option<i64>>(6)
                .map_err(map_query_sql_error)?
                .map(unix_ms_to_system_time)
                .transpose()?,
            completed_at: row
                .get::<_, Option<i64>>(7)
                .map_err(map_query_sql_error)?
                .map(unix_ms_to_system_time)
                .transpose()?,
        });
        total += 1;
    }
    if items.iter().any(|item| item.paths.is_empty()) {
        return Err(corrupt());
    }
    Ok(())
}

struct RawSession {
    version: i64,
    plan_id: String,
    started_ms: i64,
    completed_ms: Option<i64>,
    mode: String,
    estimated_bytes: i64,
    capacity_delta: Option<i64>,
    trigger: String,
    status: String,
    source_scan_id: String,
    plan_created_seconds: i64,
    plan_created_nanos: i64,
    plan_expires_seconds: i64,
    plan_expires_nanos: i64,
    owner: Option<String>,
    generation: Option<i64>,
    heartbeat_ms: Option<i64>,
    cancellation_requested: i64,
    host_identity: Option<Vec<u8>>,
    boot_scope: Option<Vec<u8>>,
    recovery_policy: Option<String>,
}

fn decode_execution_provenance(
    owner: Option<&str>,
    host_identity: Option<&[u8]>,
    boot_scope: Option<&[u8]>,
    recovery_policy: Option<&str>,
) -> Result<Option<ExecutionProvenance>, HistoryError> {
    match (host_identity, boot_scope, recovery_policy) {
        (None, None, None) => Ok(None),
        (Some(host_identity), Some(boot_scope), Some("resumable")) => {
            let owner = owner
                .ok_or_else(corrupt)
                .and_then(|value| ProcessInstanceId::from_stored(value).map_err(|_| corrupt()))?;
            ExecutionProvenance::from_stored(&owner, host_identity, boot_scope)
                .map(Some)
                .map_err(|_| corrupt())
        }
        _ => Err(corrupt()),
    }
}

fn decode_lifecycle(
    session_id: &CleanupSessionId,
    raw: &RawSession,
    cancellation_requested: bool,
    started_at: SystemTime,
) -> Result<JournalLifecycle, HistoryError> {
    decode_lifecycle_fields(
        session_id,
        RawLifecycleFields {
            status: &raw.status,
            owner: raw.owner.as_deref(),
            generation: raw.generation,
            heartbeat_ms: raw.heartbeat_ms,
            completed_ms: raw.completed_ms,
            capacity_delta: raw.capacity_delta,
        },
        cancellation_requested,
        started_at,
    )
}

#[derive(Clone, Copy)]
struct RawLifecycleFields<'a> {
    status: &'a str,
    owner: Option<&'a str>,
    generation: Option<i64>,
    heartbeat_ms: Option<i64>,
    completed_ms: Option<i64>,
    capacity_delta: Option<i64>,
}

fn decode_lifecycle_fields(
    session_id: &CleanupSessionId,
    raw: RawLifecycleFields<'_>,
    cancellation_requested: bool,
    started_at: SystemTime,
) -> Result<JournalLifecycle, HistoryError> {
    let execution = match (raw.owner, raw.generation, raw.heartbeat_ms) {
        (None, None, None) => None,
        (Some(owner), Some(generation), Some(heartbeat)) => {
            let heartbeat = unix_ms_to_system_time(heartbeat)?;
            if heartbeat < started_at {
                return Err(corrupt());
            }
            Some((
                ProcessInstanceId::from_stored(owner).map_err(|_| corrupt())?,
                decode_generation(generation)?,
                heartbeat,
            ))
        }
        _ => return Err(corrupt()),
    };
    if raw.status == "planned" {
        if execution.is_some()
            || raw.completed_ms.is_some()
            || raw.capacity_delta.is_some()
            || cancellation_requested
        {
            return Err(corrupt());
        }
        return Ok(JournalLifecycle::Planned);
    }
    if matches!(raw.status, "running" | "recovering") {
        if raw.completed_ms.is_some() || raw.capacity_delta.is_some() {
            return Err(corrupt());
        }
        let (owner, generation, heartbeat_at) = execution.ok_or_else(corrupt)?;
        return Ok(JournalLifecycle::Active {
            phase: active_phase(raw.status)?,
            fence: ExecutionFence {
                session_id: session_id.clone(),
                owner,
                generation,
            },
            heartbeat_at,
            cancellation_requested,
        });
    }
    let status = terminal_session_status_from_stored(raw.status)?;
    let completed_at = unix_ms_to_system_time(raw.completed_ms.ok_or_else(corrupt)?)?;
    if let Some((owner, generation, heartbeat_at)) = execution {
        return Ok(JournalLifecycle::Terminal {
            status,
            fence: ExecutionFence {
                session_id: session_id.clone(),
                owner,
                generation,
            },
            heartbeat_at,
            completed_at,
            verified_capacity_delta_bytes: raw.capacity_delta,
            cancellation_requested,
        });
    }
    if raw.capacity_delta.is_some() {
        return Err(corrupt());
    }
    Ok(JournalLifecycle::ObservedTerminal {
        status,
        completed_at,
        cancellation_requested,
    })
}

#[derive(Default)]
struct LoadTotals {
    paths: usize,
    evidence: usize,
}

fn load_items(
    connection: &Connection,
    session_id: &CleanupSessionId,
    totals: &mut LoadTotals,
) -> Result<Vec<JournalItem>, HistoryError> {
    let rows = {
        let mut statement = connection
            .prepare(
                "SELECT item_ordinal, rule_id, rule_revision, estimated_bytes,
                        final_status, error_category, record_format_version,
                        legacy_target_path, legacy_target_path_encoding, candidate_id,
                        category, safety_tier, proposed_action, rule_schedule_eligible,
                        newest_mtime_unix_seconds, newest_mtime_nanoseconds
                 FROM cleanup_items WHERE session_id = ?1
                 ORDER BY item_ordinal LIMIT 65",
            )
            .map_err(map_query_sql_error)?;
        statement
            .query_map([session_id.as_str()], |row| {
                Ok(RawItem {
                    ordinal: row.get(0)?,
                    rule_id: row.get(1)?,
                    rule_revision: row.get(2)?,
                    estimated_bytes: row.get(3)?,
                    status: row.get(4)?,
                    error: row.get(5)?,
                    version: row.get(6)?,
                    legacy_path: row.get(7)?,
                    legacy_encoding: row.get(8)?,
                    candidate_id: row.get(9)?,
                    category: row.get(10)?,
                    safety: row.get(11)?,
                    action: row.get(12)?,
                    schedule: row.get(13)?,
                    newest_seconds: row.get(14)?,
                    newest_nanos: row.get(15)?,
                })
            })
            .map_err(map_query_sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_query_sql_error)?
    };
    if rows.is_empty() || rows.len() > MAX_ITEMS {
        return Err(corrupt());
    }
    let mut items = Vec::with_capacity(rows.len());
    let mut candidate_ids = HashSet::new();
    for (expected, raw) in rows.into_iter().enumerate() {
        if raw.ordinal != expected as i64
            || raw.version != 2
            || raw.legacy_path.is_some()
            || raw.legacy_encoding.is_some()
        {
            return Err(corrupt());
        }
        validate_stored_error(raw.error.as_deref())?;
        let candidate_id = CandidateId::new(raw.candidate_id).map_err(|_| corrupt())?;
        if !candidate_ids.insert(candidate_id.clone()) {
            return Err(corrupt());
        }
        let rule_id = RuleId::new(raw.rule_id).map_err(|_| corrupt())?;
        let revision = u32::try_from(raw.rule_revision).map_err(|_| corrupt())?;
        let rule = RuleRef::new(rule_id, RuleRevision::new(revision).map_err(|_| corrupt())?);
        let category = category_from_stored(&raw.category)?;
        let safety = safety_from_stored(&raw.safety)?;
        let proposed_action = action_from_stored(&raw.action)?;
        let schedule = stored_bool(raw.schedule)?;
        validate_policy(safety, proposed_action, schedule, corrupt)?;
        let paths = load_paths(connection, session_id, expected, totals)?;
        let evidence = load_evidence(connection, session_id, expected, totals)?;
        let target_paths = paths
            .iter()
            .map(|path| path.target.clone())
            .collect::<Vec<_>>();
        validate_complete_children(safety, proposed_action, &target_paths, &evidence)?;
        let frozen = PlannedCleanupItemRecord {
            ordinal: expected,
            candidate_id,
            rule,
            category,
            paths: target_paths,
            estimated_bytes: from_i64(raw.estimated_bytes)?,
            newest_mtime: decode_optional_time(raw.newest_seconds, raw.newest_nanos)?,
            evidence,
            safety,
            proposed_action,
            rule_schedule_eligible: schedule,
            prior_review_status: None,
        };
        items.push(JournalItem {
            frozen,
            status: PathStatus::from_stored(&raw.status)?,
            error_category: raw.error,
            paths,
        });
    }
    ensure_no_orphans(connection, session_id)?;
    Ok(items)
}

struct RawItem {
    ordinal: i64,
    rule_id: String,
    rule_revision: i64,
    estimated_bytes: i64,
    status: String,
    error: Option<String>,
    version: i64,
    legacy_path: Option<Vec<u8>>,
    legacy_encoding: Option<i64>,
    candidate_id: String,
    category: String,
    safety: String,
    action: String,
    schedule: i64,
    newest_seconds: Option<i64>,
    newest_nanos: Option<i64>,
}

fn load_paths(
    connection: &Connection,
    session_id: &CleanupSessionId,
    item_ordinal: usize,
    totals: &mut LoadTotals,
) -> Result<Vec<JournalPath>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT path_ordinal, target_path, target_path_encoding,
                    attempt_generation, status, error_category,
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
        if paths.len() >= MAX_TOTAL_PATHS || totals.paths >= MAX_TOTAL_PATHS {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != paths.len() as i64 {
            return Err(corrupt());
        }
        let error: Option<String> = row.get(5).map_err(map_query_sql_error)?;
        validate_stored_error(error.as_deref())?;
        paths.push(JournalPath {
            ordinal: paths.len(),
            target: decode_absolute_path(
                row.get(1).map_err(map_query_sql_error)?,
                row.get(2).map_err(map_query_sql_error)?,
            )?,
            attempt_generation: row
                .get::<_, Option<i64>>(3)
                .map_err(map_query_sql_error)?
                .map(decode_generation)
                .transpose()?,
            status: PathStatus::from_stored(
                &row.get::<_, String>(4).map_err(map_query_sql_error)?,
            )?,
            error_category: error,
            effect_started_at: row
                .get::<_, Option<i64>>(6)
                .map_err(map_query_sql_error)?
                .map(unix_ms_to_system_time)
                .transpose()?,
            completed_at: row
                .get::<_, Option<i64>>(7)
                .map_err(map_query_sql_error)?
                .map(unix_ms_to_system_time)
                .transpose()?,
        });
        totals.paths += 1;
    }
    if paths.is_empty()
        || paths
            .iter()
            .map(|path| &path.target)
            .collect::<HashSet<_>>()
            .len()
            != paths.len()
    {
        return Err(corrupt());
    }
    Ok(paths)
}

pub(super) fn load_path_statuses(
    connection: &Connection,
    session_id: &CleanupSessionId,
    item_ordinal: usize,
) -> Result<Vec<JournalPath>, HistoryError> {
    load_paths(
        connection,
        session_id,
        item_ordinal,
        &mut LoadTotals::default(),
    )
}

fn load_evidence(
    connection: &Connection,
    session_id: &CleanupSessionId,
    item_ordinal: usize,
    totals: &mut LoadTotals,
) -> Result<Vec<Evidence>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT evidence_ordinal, evidence_kind, path_value,
                    path_value_encoding, text_value, observed_unix_seconds,
                    observed_nanoseconds, duration_seconds, duration_nanoseconds,
                    observed_bytes, minimum_bytes
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
        if evidence.len() >= MAX_TOTAL_EVIDENCE || totals.evidence >= MAX_TOTAL_EVIDENCE {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != evidence.len() as i64 {
            return Err(corrupt());
        }
        evidence.push(decode_evidence(RawEvidence {
            kind: row.get(1).map_err(map_query_sql_error)?,
            path: row.get(2).map_err(map_query_sql_error)?,
            path_encoding: row.get(3).map_err(map_query_sql_error)?,
            text: row.get(4).map_err(map_query_sql_error)?,
            observed_seconds: row.get(5).map_err(map_query_sql_error)?,
            observed_nanoseconds: row.get(6).map_err(map_query_sql_error)?,
            duration_seconds: row.get(7).map_err(map_query_sql_error)?,
            duration_nanoseconds: row.get(8).map_err(map_query_sql_error)?,
            observed_bytes: row.get(9).map_err(map_query_sql_error)?,
            minimum_bytes: row.get(10).map_err(map_query_sql_error)?,
        })?);
        totals.evidence += 1;
    }
    if evidence.is_empty() {
        return Err(corrupt());
    }
    Ok(evidence)
}

fn load_warnings(
    connection: &Connection,
    session_id: &CleanupSessionId,
) -> Result<Vec<PlanWarning>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT warning_ordinal, warning_kind FROM cleanup_plan_warnings
             WHERE session_id = ?1 ORDER BY warning_ordinal LIMIT 6",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([session_id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut warnings = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if warnings.len() >= MAX_WARNINGS
            || row.get::<_, i64>(0).map_err(map_query_sql_error)? != warnings.len() as i64
        {
            return Err(corrupt());
        }
        let warning = warning_from_stored(&row.get::<_, String>(1).map_err(map_query_sql_error)?)?;
        if warnings.contains(&warning) {
            return Err(corrupt());
        }
        warnings.push(warning);
    }
    Ok(warnings)
}

fn ensure_no_orphans(
    connection: &Connection,
    session_id: &CleanupSessionId,
) -> Result<(), HistoryError> {
    let orphan: i64 = connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM cleanup_item_paths AS child
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
             )",
            [session_id.as_str()],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    if orphan != 0 { Err(corrupt()) } else { Ok(()) }
}

pub(super) trait DynamicPathState {
    fn attempt_generation(&self) -> Option<u64>;
    fn status(&self) -> PathStatus;
    fn error_category(&self) -> Option<&str>;
    fn effect_started_at(&self) -> Option<SystemTime>;
    fn completed_at(&self) -> Option<SystemTime>;
}

impl DynamicPathState for JournalPath {
    fn attempt_generation(&self) -> Option<u64> {
        self.attempt_generation
    }

    fn status(&self) -> PathStatus {
        self.status
    }

    fn error_category(&self) -> Option<&str> {
        self.error_category.as_deref()
    }

    fn effect_started_at(&self) -> Option<SystemTime> {
        self.effect_started_at
    }

    fn completed_at(&self) -> Option<SystemTime> {
        self.completed_at
    }
}

impl DynamicPathState for ScalarJournalPath {
    fn attempt_generation(&self) -> Option<u64> {
        self.attempt_generation
    }

    fn status(&self) -> PathStatus {
        self.status
    }

    fn error_category(&self) -> Option<&str> {
        self.error_category.as_deref()
    }

    fn effect_started_at(&self) -> Option<SystemTime> {
        self.effect_started_at
    }

    fn completed_at(&self) -> Option<SystemTime> {
        self.completed_at
    }
}

trait DynamicItemState {
    type Path: DynamicPathState;

    fn action(&self) -> CandidateAction;
    fn status(&self) -> PathStatus;
    fn error_category(&self) -> Option<&str>;
    fn paths(&self) -> &[Self::Path];
}

impl DynamicItemState for JournalItem {
    type Path = JournalPath;

    fn action(&self) -> CandidateAction {
        self.frozen.proposed_action
    }

    fn status(&self) -> PathStatus {
        self.status
    }

    fn error_category(&self) -> Option<&str> {
        self.error_category.as_deref()
    }

    fn paths(&self) -> &[Self::Path] {
        &self.paths
    }
}

impl DynamicItemState for ScalarJournalItem {
    type Path = ScalarJournalPath;

    fn action(&self) -> CandidateAction {
        self.action
    }

    fn status(&self) -> PathStatus {
        self.status
    }

    fn error_category(&self) -> Option<&str> {
        self.error_category.as_deref()
    }

    fn paths(&self) -> &[Self::Path] {
        &self.paths
    }
}

fn validate_dynamic_graph(
    lifecycle: &JournalLifecycle,
    mode: CleanupMode,
    started_at: SystemTime,
    items: &[JournalItem],
) -> Result<(), HistoryError> {
    validate_dynamic_state(lifecycle, mode, started_at, items)
}

fn validate_dynamic_state<I: DynamicItemState>(
    lifecycle: &JournalLifecycle,
    mode: CleanupMode,
    started_at: SystemTime,
    items: &[I],
) -> Result<(), HistoryError> {
    let (current_generation, heartbeat_at) = match lifecycle {
        JournalLifecycle::Planned => (None, None),
        JournalLifecycle::ObservedTerminal { .. } => (Some(1), None),
        JournalLifecycle::Active {
            fence,
            heartbeat_at,
            ..
        }
        | JournalLifecycle::Terminal {
            fence,
            heartbeat_at,
            ..
        } => (Some(fence.generation), Some(*heartbeat_at)),
    };
    for item in items {
        if item.status() != derive_item_status(item.paths())
            || item.error_category()
                != item
                    .paths()
                    .iter()
                    .find(|path| path.status() == item.status())
                    .and_then(DynamicPathState::error_category)
        {
            return Err(corrupt());
        }
        for path in item.paths() {
            validate_path_shape(
                path,
                current_generation,
                matches!(lifecycle, JournalLifecycle::ObservedTerminal { .. }),
                started_at,
            )?;
            if path
                .effect_started_at()
                .zip(heartbeat_at)
                .is_some_and(|(effect, heartbeat)| effect > heartbeat)
            {
                return Err(corrupt());
            }
            if path.status().is_success() && !success_matches(mode, item.action(), path.status()) {
                return Err(corrupt());
            }
        }
    }
    let statuses = items
        .iter()
        .flat_map(|item| item.paths().iter().map(DynamicPathState::status))
        .collect::<Vec<_>>();
    match lifecycle {
        JournalLifecycle::Planned => {
            if statuses.iter().any(|status| *status != PathStatus::Planned) {
                return Err(corrupt());
            }
        }
        JournalLifecycle::Active { phase, fence, .. } => {
            if items.iter().flat_map(DynamicItemState::paths).any(|path| {
                matches!(
                    path.status(),
                    PathStatus::Validating | PathStatus::EffectStarted
                ) && path.attempt_generation() != Some(fence.generation)
            }) || (*phase == ActivePhase::Running
                && statuses.contains(&PathStatus::OutcomeUnknown))
                || (*phase == ActivePhase::Recovering
                    && statuses.iter().any(|status| {
                        matches!(status, PathStatus::Validating | PathStatus::EffectStarted)
                    }))
            {
                return Err(corrupt());
            }
        }
        JournalLifecycle::Terminal {
            status,
            heartbeat_at,
            completed_at,
            cancellation_requested,
            ..
        } => {
            if derive_terminal_session_status(&statuses, mode, *cancellation_requested)
                .map_err(|_| corrupt())?
                != *status
            {
                return Err(corrupt());
            }
            if items
                .iter()
                .flat_map(DynamicItemState::paths)
                .filter_map(DynamicPathState::completed_at)
                .any(|path_completed| path_completed > *completed_at)
            {
                return Err(corrupt());
            }
            if *completed_at < *heartbeat_at {
                return Err(corrupt());
            }
        }
        JournalLifecycle::ObservedTerminal {
            status,
            completed_at,
            cancellation_requested,
        } => {
            if mode != CleanupMode::DryRun
                || derive_terminal_session_status(&statuses, mode, *cancellation_requested)
                    .map_err(|_| corrupt())?
                    != *status
                || items.iter().flat_map(DynamicItemState::paths).any(|path| {
                    path.attempt_generation() != Some(1)
                        || path.effect_started_at().is_some()
                        || path.completed_at() != Some(*completed_at)
                })
            {
                return Err(corrupt());
            }
        }
    }
    Ok(())
}

fn validate_path_shape<P: DynamicPathState>(
    path: &P,
    current_generation: Option<u64>,
    observed_terminal: bool,
    started_at: SystemTime,
) -> Result<(), HistoryError> {
    if path
        .attempt_generation()
        .is_some_and(|generation| current_generation.is_none_or(|current| generation > current))
        || (path.status() == PathStatus::Planned && path.attempt_generation().is_some())
        || (path.status() != PathStatus::Planned
            && path.attempt_generation().is_none()
            && !observed_terminal)
        || (observed_terminal && path.attempt_generation() != Some(1))
        || (path.status() == PathStatus::Planned
            && (path.error_category().is_some()
                || path.effect_started_at().is_some()
                || path.completed_at().is_some()))
        || (path.status() == PathStatus::Validating
            && (path.error_category().is_some()
                || path.effect_started_at().is_some()
                || path.completed_at().is_some()))
        || (matches!(
            path.status(),
            PathStatus::EffectStarted
                | PathStatus::Trashed
                | PathStatus::Removed
                | PathStatus::Evicted
                | PathStatus::OutcomeUnknown
        ) && path.effect_started_at().is_none())
        || (path.status() == PathStatus::EffectStarted && path.completed_at().is_some())
        || (path.status() == PathStatus::EffectStarted && path.error_category().is_some())
        || (matches!(
            path.status(),
            PathStatus::DryRun
                | PathStatus::Skipped
                | PathStatus::Rejected
                | PathStatus::ChangedSincePlan
                | PathStatus::Interrupted
                | PathStatus::Unavailable
        ) && path.effect_started_at().is_some())
        || (path.status().is_terminal() && path.completed_at().is_none())
        || (path.status().is_success() && path.error_category().is_some())
        || path
            .effect_started_at()
            .zip(path.completed_at())
            .is_some_and(|(start, end)| end < start)
        || path
            .effect_started_at()
            .is_some_and(|value| value < started_at)
        || path.completed_at().is_some_and(|value| value < started_at)
    {
        return Err(corrupt());
    }
    Ok(())
}

fn cleanup_paths_overlap(items: &[JournalItem]) -> bool {
    let paths = items
        .iter()
        .flat_map(|item| item.frozen.paths.iter())
        .collect::<Vec<_>>();
    paths.iter().enumerate().any(|(index, left)| {
        paths[index + 1..]
            .iter()
            .any(|right| left == right || left.starts_with(right) || right.starts_with(left))
    })
}

fn active_phase(value: &str) -> Result<ActivePhase, HistoryError> {
    match value {
        "running" => Ok(ActivePhase::Running),
        "recovering" => Ok(ActivePhase::Recovering),
        _ => Err(corrupt()),
    }
}

fn terminal_session_status_from_stored(value: &str) -> Result<TerminalSessionStatus, HistoryError> {
    match value {
        "completed" => Ok(TerminalSessionStatus::Completed),
        "partially_completed" => Ok(TerminalSessionStatus::PartiallyCompleted),
        "failed" => Ok(TerminalSessionStatus::Failed),
        "cancelled" => Ok(TerminalSessionStatus::Cancelled),
        "interrupted" => Ok(TerminalSessionStatus::Interrupted),
        "rejected" => Ok(TerminalSessionStatus::Rejected),
        "dry_run" => Ok(TerminalSessionStatus::DryRun),
        _ => Err(corrupt()),
    }
}

fn mode_accepts(mode: CleanupMode, safety: SafetyTier, action: CandidateAction) -> bool {
    matches!(
        (mode, safety, action),
        (
            CleanupMode::DryRun,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents
        ) | (
            CleanupMode::DryRun,
            SafetyTier::SafeEvictable,
            CandidateAction::EvictLocalCopy
        ) | (
            CleanupMode::DryRun,
            SafetyTier::ReviewRequired,
            CandidateAction::MoveToTrash
        ) | (
            CleanupMode::Trash,
            SafetyTier::ReviewRequired,
            CandidateAction::MoveToTrash
        ) | (
            CleanupMode::PermanentSafe,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents
        ) | (
            CleanupMode::EvictLocalCopy,
            SafetyTier::SafeEvictable,
            CandidateAction::EvictLocalCopy
        )
    )
}

pub(super) fn mode_accepts_action(mode: CleanupMode, action: CandidateAction) -> bool {
    matches!(
        (mode, action),
        (CleanupMode::Trash, CandidateAction::MoveToTrash)
            | (
                CleanupMode::PermanentSafe,
                CandidateAction::RemoveKnownRegenerableContents
            )
            | (CleanupMode::EvictLocalCopy, CandidateAction::EvictLocalCopy)
    )
}

fn decode_bool(value: i64) -> Result<bool, HistoryError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(corrupt()),
    }
}

fn validate_stored_error(value: Option<&str>) -> Result<(), HistoryError> {
    if value.is_some_and(|value| {
        value.is_empty()
            || value.len() > MAX_ERROR_BYTES
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-' | b':')
            })
    }) {
        return Err(corrupt());
    }
    Ok(())
}
