//! Bounded, read-only diagnostics for active cleanup recovery journals.
//!
//! The census observes only scalar session state and provenance relationships.
//! It never selects cleanup paths and returns no session, plan, owner, or
//! provenance identity.

use rusqlite::Connection;

use super::cleanup_history::{CleanupSessionId, mode_from_stored};
use super::cleanup_history_query::validate_cleanup_recovery_scalar_graph_within_budget;
use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error,
    run_bounded_cleanup_recovery_diagnostic_query, unix_ms_to_system_time,
};
use super::process_liveness::{
    ExecutionProvenance, ProcessInstanceId, ProvenanceRelationship, compare_execution_provenance,
};

pub(super) const MAX_CLEANUP_RECOVERY_DIAGNOSTIC_ROWS: usize = 64;
const CLEANUP_RECOVERY_POLICY: &str = "resumable";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CleanupRecoveryDiagnosticCensus {
    pub(crate) active_total: u16,
    pub(crate) running_count: u16,
    pub(crate) recovering_count: u16,
    pub(crate) same_host_current_boot_count: u16,
    pub(crate) same_host_prior_boot_count: u16,
    pub(crate) foreign_host_count: u16,
    pub(crate) stored_unproven_count: u16,
    pub(crate) current_context_unavailable_count: u16,
    pub(crate) has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActivePhase {
    Running,
    Recovering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProvenanceClass {
    SameHostCurrentBoot,
    SameHostPriorBoot,
    ForeignHost,
    StoredUnproven,
    CurrentContextUnavailable,
}

struct ActiveCleanupSession {
    session_id: CleanupSessionId,
    version: i64,
    phase: ActivePhase,
    provenance: Option<ExecutionProvenance>,
}

fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

pub(super) const fn cleanup_recovery_diagnostic_query() -> &'static str {
    // Match the recovery index order exactly. The 65th row is a strict
    // lookahead sentinel: callers validate it before truncating the page.
    "SELECT
         typeof(session.session_id), length(CAST(session.session_id AS BLOB)),
         session.session_id, session.record_format_version,
         session.started_at_unix_ms, session.completed_at_unix_ms,
         session.mode, session.verified_capacity_delta_bytes,
         session.status, session.execution_owner_id,
         session.execution_generation, session.last_heartbeat_at_unix_ms,
         session.cancellation_requested,
         session.execution_host_identity_v1_sha256,
         session.execution_boot_scope_v1_sha256,
         session.execution_recovery_policy,
         CASE WHEN
             typeof(session.session_id) = 'text' AND
             length(CAST(session.session_id AS BLOB)) BETWEEN 1 AND 128 AND
             typeof(session.record_format_version) = 'integer' AND
             session.record_format_version IN (1, 2) AND
             typeof(session.started_at_unix_ms) = 'integer' AND
             typeof(session.completed_at_unix_ms) = 'null' AND
             typeof(session.mode) = 'text' AND
             length(CAST(session.mode AS BLOB)) BETWEEN 1 AND 64 AND
             typeof(session.verified_capacity_delta_bytes) = 'null' AND
             typeof(session.status) = 'text' AND
             session.status IN ('running', 'recovering') AND
             typeof(session.execution_owner_id) IN ('text', 'null') AND
             (session.execution_owner_id IS NULL OR
                 length(CAST(session.execution_owner_id AS BLOB)) BETWEEN 1 AND 128) AND
             typeof(session.execution_generation) IN ('integer', 'null') AND
             typeof(session.last_heartbeat_at_unix_ms) IN ('integer', 'null') AND
             typeof(session.cancellation_requested) IN ('integer', 'null') AND
             typeof(session.execution_host_identity_v1_sha256) IN ('blob', 'null') AND
             (session.execution_host_identity_v1_sha256 IS NULL OR
                 length(session.execution_host_identity_v1_sha256) = 32) AND
             typeof(session.execution_boot_scope_v1_sha256) IN ('blob', 'null') AND
             (session.execution_boot_scope_v1_sha256 IS NULL OR
                 length(session.execution_boot_scope_v1_sha256) = 32) AND
             typeof(session.execution_recovery_policy) IN ('text', 'null') AND
             (session.execution_recovery_policy IS NULL OR
                 length(CAST(session.execution_recovery_policy AS BLOB)) BETWEEN 1 AND 32)
         THEN 0 ELSE 1 END
     FROM cleanup_sessions AS session INDEXED BY cleanup_sessions_by_recovery
     WHERE session.status IN ('running', 'recovering')
     ORDER BY session.status, session.last_heartbeat_at_unix_ms, session.session_id
     LIMIT 65"
}

fn load_active_cleanup_session(
    row: &rusqlite::Row<'_>,
) -> Result<ActiveCleanupSession, HistoryError> {
    let id_storage: String = row.get(0).map_err(map_query_sql_error)?;
    let id_length: i64 = row.get(1).map_err(map_query_sql_error)?;
    let session_id: String = row.get(2).map_err(map_query_sql_error)?;
    let version: i64 = row.get(3).map_err(map_query_sql_error)?;
    let started_ms: i64 = row.get(4).map_err(map_query_sql_error)?;
    let completed_ms: Option<i64> = row.get(5).map_err(map_query_sql_error)?;
    let mode: String = row.get(6).map_err(map_query_sql_error)?;
    let capacity_delta: Option<i64> = row.get(7).map_err(map_query_sql_error)?;
    let status: String = row.get(8).map_err(map_query_sql_error)?;
    let owner: Option<String> = row.get(9).map_err(map_query_sql_error)?;
    let generation: Option<i64> = row.get(10).map_err(map_query_sql_error)?;
    let heartbeat_ms: Option<i64> = row.get(11).map_err(map_query_sql_error)?;
    let cancellation_requested: Option<i64> = row.get(12).map_err(map_query_sql_error)?;
    let host_identity: Option<Vec<u8>> = row.get(13).map_err(map_query_sql_error)?;
    let boot_scope: Option<Vec<u8>> = row.get(14).map_err(map_query_sql_error)?;
    let recovery_policy: Option<String> = row.get(15).map_err(map_query_sql_error)?;
    let invalid_storage: i64 = row.get(16).map_err(map_query_sql_error)?;

    if invalid_storage != 0
        || id_storage != "text"
        || !(1..=128).contains(&id_length)
        || session_id.len() != id_length as usize
        || !matches!(version, 1 | 2)
        || completed_ms.is_some()
        || capacity_delta.is_some()
    {
        return Err(corrupt());
    }
    let session_id = CleanupSessionId::new(session_id).map_err(|_| corrupt())?;
    let started_at = unix_ms_to_system_time(started_ms)?;
    mode_from_stored(&mode)?;
    let phase = match status.as_str() {
        "running" => ActivePhase::Running,
        "recovering" => ActivePhase::Recovering,
        _ => return Err(corrupt()),
    };
    let provenance = if version == 1 {
        if phase != ActivePhase::Running
            || owner.is_some()
            || generation.is_some()
            || heartbeat_ms.is_some()
            || cancellation_requested.is_some()
            || host_identity.is_some()
            || boot_scope.is_some()
            || recovery_policy.is_some()
        {
            return Err(corrupt());
        }
        None
    } else {
        let owner = owner
            .as_deref()
            .ok_or_else(corrupt)
            .and_then(|owner| ProcessInstanceId::from_stored(owner).map_err(|_| corrupt()))?;
        let generation = generation.ok_or_else(corrupt)?;
        let heartbeat_at = unix_ms_to_system_time(heartbeat_ms.ok_or_else(corrupt)?)?;
        if generation <= 0
            || !matches!(cancellation_requested, Some(0 | 1))
            || heartbeat_at < started_at
        {
            return Err(corrupt());
        }
        match (
            host_identity.as_deref(),
            boot_scope.as_deref(),
            recovery_policy.as_deref(),
        ) {
            (None, None, None) => None,
            (Some(host), Some(boot), Some(CLEANUP_RECOVERY_POLICY)) => {
                Some(ExecutionProvenance::from_stored(&owner, host, boot).map_err(|_| corrupt())?)
            }
            _ => return Err(corrupt()),
        }
    };
    Ok(ActiveCleanupSession {
        session_id,
        version,
        phase,
        provenance,
    })
}

fn classify_provenance(
    stored: Option<&ExecutionProvenance>,
    current: Option<&ExecutionProvenance>,
) -> Result<ProvenanceClass, HistoryError> {
    let Some(stored) = stored else {
        return Ok(ProvenanceClass::StoredUnproven);
    };
    let Some(current) = current else {
        return Ok(ProvenanceClass::CurrentContextUnavailable);
    };
    match compare_execution_provenance(Some(stored), Some(current)) {
        ProvenanceRelationship::SameBoot => Ok(ProvenanceClass::SameHostCurrentBoot),
        ProvenanceRelationship::PriorBoot => Ok(ProvenanceClass::SameHostPriorBoot),
        ProvenanceRelationship::ForeignHost => Ok(ProvenanceClass::ForeignHost),
        ProvenanceRelationship::Unproven => Err(corrupt()),
    }
}

pub(super) fn load_cleanup_recovery_diagnostic_census(
    connection: &Connection,
    current: Option<&ExecutionProvenance>,
) -> Result<CleanupRecoveryDiagnosticCensus, HistoryError> {
    run_bounded_cleanup_recovery_diagnostic_query(connection, || {
        load_cleanup_recovery_diagnostic_census_within_budget(connection, current)
    })
}

fn load_cleanup_recovery_diagnostic_census_within_budget(
    connection: &Connection,
    current: Option<&ExecutionProvenance>,
) -> Result<CleanupRecoveryDiagnosticCensus, HistoryError> {
    let mut statement = connection
        .prepare(cleanup_recovery_diagnostic_query())
        .map_err(map_query_sql_error)?;
    let mut rows = statement.query([]).map_err(map_query_sql_error)?;
    let mut sessions = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        let session = load_active_cleanup_session(row)?;
        validate_cleanup_recovery_scalar_graph_within_budget(
            connection,
            &session.session_id,
            session.version,
        )?;
        sessions.push(session);
    }
    let has_more = sessions.len() > MAX_CLEANUP_RECOVERY_DIAGNOSTIC_ROWS;
    sessions.truncate(MAX_CLEANUP_RECOVERY_DIAGNOSTIC_ROWS);

    let mut census = CleanupRecoveryDiagnosticCensus {
        has_more,
        ..CleanupRecoveryDiagnosticCensus::default()
    };
    for session in &sessions {
        let phase_count = match session.phase {
            ActivePhase::Running => &mut census.running_count,
            ActivePhase::Recovering => &mut census.recovering_count,
        };
        *phase_count = phase_count.checked_add(1).ok_or_else(corrupt)?;
        let provenance_count = match classify_provenance(session.provenance.as_ref(), current)? {
            ProvenanceClass::SameHostCurrentBoot => &mut census.same_host_current_boot_count,
            ProvenanceClass::SameHostPriorBoot => &mut census.same_host_prior_boot_count,
            ProvenanceClass::ForeignHost => &mut census.foreign_host_count,
            ProvenanceClass::StoredUnproven => &mut census.stored_unproven_count,
            ProvenanceClass::CurrentContextUnavailable => {
                &mut census.current_context_unavailable_count
            }
        };
        *provenance_count = provenance_count.checked_add(1).ok_or_else(corrupt)?;
    }
    census.active_total = u16::try_from(sessions.len()).map_err(|_| corrupt())?;

    let phases = census.running_count.checked_add(census.recovering_count);
    let comparable = census
        .same_host_current_boot_count
        .checked_add(census.same_host_prior_boot_count)
        .and_then(|count| count.checked_add(census.foreign_host_count));
    let classified = comparable
        .and_then(|count| count.checked_add(census.stored_unproven_count))
        .and_then(|count| count.checked_add(census.current_context_unavailable_count));
    if phases != Some(census.active_total)
        || classified != Some(census.active_total)
        || (census.current_context_unavailable_count > 0 && comparable != Some(0))
        || usize::from(census.active_total) > MAX_CLEANUP_RECOVERY_DIAGNOSTIC_ROWS
        || (census.has_more
            && usize::from(census.active_total) != MAX_CLEANUP_RECOVERY_DIAGNOSTIC_ROWS)
    {
        return Err(corrupt());
    }
    Ok(census)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rusqlite::params;
    use tempfile::TempDir;

    use super::*;
    use crate::persistence::StoreCoordinator;
    use crate::persistence::history::run_bounded_cleanup_recovery_diagnostic_query_for_test;

    fn fixture() -> (TempDir, std::sync::Arc<StoreCoordinator>) {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("data/dux.sqlite3")).unwrap();
        (temp, store)
    }

    fn owner(boot: u8, nonce: u128) -> String {
        let value = format!(
            "1:l:2a:1234:{}:{nonce:032x}",
            format!("{boot:02x}").repeat(32)
        );
        ProcessInstanceId::from_stored(&value).unwrap();
        value
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_active(
        connection: &Connection,
        ordinal: usize,
        phase: &str,
        owner: &str,
        heartbeat_ms: i64,
        host: Option<&[u8]>,
        boot: Option<&[u8]>,
        policy: Option<&str>,
    ) {
        let scan_id = format!("scan:cleanup-census:{ordinal:03}");
        let session_id = format!("cleanup:census:{ordinal:03}");
        let plan_id = format!("plan:cleanup-census:{ordinal:03}");
        connection
            .execute(
                "INSERT INTO scans (
                     scan_id, root_path, root_path_encoding,
                     started_at_unix_ms, status, coverage_status
                 ) VALUES (?1, X'2F', 1, 0, 'failed', 'unknown')",
                [&scan_id],
            )
            .unwrap();
        let candidate_id = format!("candidate:cleanup-census:{ordinal:03}");
        connection
            .execute(
                "INSERT INTO candidates (
                     candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                     estimated_bytes, created_at_unix_ms, status,
                     record_format_version, category, proposed_action,
                     rule_schedule_eligible
                 ) VALUES (
                     ?1, ?2, 'developer.rust.target', 1, 'safe_regenerable',
                     0, 0, 'planned', 2, 'developer_artifact',
                     'remove_known_regenerable_contents', 0
                 )",
                params![candidate_id, scan_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO candidate_paths (
                     candidate_id, path_ordinal, observed_path, observed_path_encoding
                 ) VALUES (?1, 0, X'2F746D70', 1)",
                [&candidate_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO candidate_evidence (
                     candidate_id, evidence_ordinal, evidence_kind,
                     observed_bytes, minimum_bytes
                 ) VALUES (?1, 0, 'minimum_size', 0, 0)",
                [&candidate_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cleanup_sessions (
                     session_id, plan_id, started_at_unix_ms, mode,
                     estimated_bytes, trigger_source, status,
                     record_format_version, source_scan_id,
                     plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                     plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                     execution_owner_id, execution_generation,
                     last_heartbeat_at_unix_ms, cancellation_requested,
                     execution_host_identity_v1_sha256,
                     execution_boot_scope_v1_sha256, execution_recovery_policy,
                     candidate_status_coupling_version
                 ) VALUES (
                     ?1, ?2, 0, 'dry_run', 0, 'manual', ?3, 2, ?4,
                     0, 0, 900, 0, ?5, 1, ?6, 0, ?7, ?8, ?9, 2
                 )",
                params![
                    session_id,
                    plan_id,
                    phase,
                    scan_id,
                    owner,
                    heartbeat_ms,
                    host,
                    boot,
                    policy,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cleanup_items (
                     session_id, item_ordinal, rule_id, rule_revision,
                     estimated_bytes, final_status, record_format_version,
                     candidate_id, category, safety_tier, proposed_action,
                     rule_schedule_eligible
                 ) VALUES (
                     ?1, 0, 'developer.rust.target', 1, 0, 'planned', 2,
                     ?2, 'developer_artifact', 'safe_regenerable',
                     'remove_known_regenerable_contents', 0
                 )",
                params![session_id, candidate_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cleanup_item_paths (
                     session_id, item_ordinal, path_ordinal,
                     target_path, target_path_encoding, status
                 ) VALUES (?1, 0, 0, X'2F746D70', 1, 'planned')",
                [&session_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cleanup_item_evidence (
                     session_id, item_ordinal, evidence_ordinal, evidence_kind,
                     observed_bytes, minimum_bytes
                 ) VALUES (?1, 0, 0, 'minimum_size', 0, 0)",
                [&session_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO candidate_plan_claims (
                     candidate_id, session_id, item_ordinal, prior_review_status
                 ) VALUES (?1, ?2, 0, 'discovered')",
                params![candidate_id, session_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO trusted_rust_target_plan_claims (
                     candidate_id, session_id, item_ordinal, coupling_revision
                 ) VALUES (?1, ?2, 0, 1)",
                params![candidate_id, session_id],
            )
            .unwrap();
    }

    fn insert_legacy_active(connection: &Connection, ordinal: usize) {
        let session_id = format!("cleanup:legacy-census:{ordinal:03}");
        let plan_id = format!("plan:legacy-census:{ordinal:03}");
        connection
            .execute(
                "INSERT INTO cleanup_sessions (
                     session_id, plan_id, started_at_unix_ms, mode,
                     estimated_bytes, trigger_source, status, record_format_version
                 ) VALUES (?1, ?2, 0, 'dry_run', 0, 'manual', 'running', 1)",
                params![session_id, plan_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cleanup_items (
                     session_id, item_ordinal, rule_id, rule_revision,
                     estimated_bytes, final_status, record_format_version,
                     legacy_target_path, legacy_target_path_encoding
                 ) VALUES (
                     ?1, 0, 'legacy.rule', 1, 0, 'planned', 1, X'2F746D70', 1
                 )",
                [&session_id],
            )
            .unwrap();
    }

    fn expand_active_to_maximum_scalar_graph(connection: &Connection, ordinal: usize) {
        let scan_id = format!("scan:cleanup-census:{ordinal:03}");
        let session_id = format!("cleanup:census:{ordinal:03}");
        let candidate_prefix = format!("candidate:cleanup-census:{ordinal:03}");
        connection
            .execute(
                "WITH RECURSIVE seq(value) AS (
                     SELECT 1 UNION ALL SELECT value + 1 FROM seq WHERE value < 63
                 )
                 INSERT INTO candidates (
                     candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                     estimated_bytes, created_at_unix_ms, status,
                     record_format_version, category, proposed_action,
                     rule_schedule_eligible
                 )
                 SELECT ?1 || printf(':%03d', value), ?2,
                        'developer.rust.target', 1, 'safe_regenerable',
                        0, 0, 'planned', 2, 'developer_artifact',
                        'remove_known_regenerable_contents', 0
                 FROM seq",
                params![candidate_prefix, scan_id],
            )
            .unwrap();
        connection
            .execute(
                "WITH RECURSIVE seq(value) AS (
                     SELECT 1 UNION ALL SELECT value + 1 FROM seq WHERE value < 63
                 )
                 INSERT INTO candidate_paths (
                     candidate_id, path_ordinal, observed_path, observed_path_encoding
                 )
                 SELECT ?1 || printf(':%03d', value), 0, X'2F746D70', 1
                 FROM seq",
                [&candidate_prefix],
            )
            .unwrap();
        connection
            .execute(
                "WITH RECURSIVE seq(value) AS (
                     SELECT 1 UNION ALL SELECT value + 1 FROM seq WHERE value < 63
                 )
                 INSERT INTO candidate_evidence (
                     candidate_id, evidence_ordinal, evidence_kind,
                     observed_bytes, minimum_bytes
                 )
                 SELECT ?1 || printf(':%03d', value), 0, 'minimum_size', 0, 0
                 FROM seq",
                [&candidate_prefix],
            )
            .unwrap();
        connection
            .execute(
                "WITH RECURSIVE seq(value) AS (
                     SELECT 1 UNION ALL SELECT value + 1 FROM seq WHERE value < 63
                 )
                 INSERT INTO cleanup_items (
                     session_id, item_ordinal, rule_id, rule_revision,
                     estimated_bytes, final_status, record_format_version,
                     candidate_id, category, safety_tier, proposed_action,
                     rule_schedule_eligible
                 )
                 SELECT ?1, value, 'developer.rust.target', 1, 0, 'planned', 2,
                        ?2 || printf(':%03d', value), 'developer_artifact',
                        'safe_regenerable', 'remove_known_regenerable_contents', 0
                 FROM seq",
                params![session_id, candidate_prefix],
            )
            .unwrap();
        connection
            .execute(
                "WITH RECURSIVE
                     item(value) AS (
                         SELECT 0 UNION ALL SELECT value + 1 FROM item WHERE value < 63
                     ),
                     path(value) AS (
                         SELECT 0 UNION ALL SELECT value + 1 FROM path WHERE value < 3
                     )
                 INSERT INTO cleanup_item_paths (
                     session_id, item_ordinal, path_ordinal,
                     target_path, target_path_encoding, status
                 )
                 SELECT ?1, item.value, path.value,
                        CAST(printf('/tmp/%03d/%03d', item.value, path.value) AS BLOB),
                        1, 'planned'
                 FROM item CROSS JOIN path
                 WHERE item.value != 0 OR path.value != 0",
                [&session_id],
            )
            .unwrap();
        connection
            .execute(
                "WITH RECURSIVE
                     item(value) AS (
                         SELECT 0 UNION ALL SELECT value + 1 FROM item WHERE value < 63
                     ),
                     evidence(value) AS (
                         SELECT 0 UNION ALL SELECT value + 1 FROM evidence WHERE value < 7
                     )
                 INSERT INTO cleanup_item_evidence (
                     session_id, item_ordinal, evidence_ordinal, evidence_kind,
                     observed_bytes, minimum_bytes
                 )
                 SELECT ?1, item.value, evidence.value, 'minimum_size', 0, 0
                 FROM item CROSS JOIN evidence
                 WHERE item.value != 0 OR evidence.value != 0",
                [&session_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cleanup_plan_warnings (
                     session_id, warning_ordinal, warning_kind
                 ) VALUES
                     (?1, 0, 'estimated_bytes_unverified'),
                     (?1, 1, 'dry_run_does_not_mutate'),
                     (?1, 2, 'trash_does_not_free_space_immediately'),
                     (?1, 3, 'permanent_removal_cannot_be_undone'),
                     (?1, 4, 'cloud_eviction_requires_network_to_redownload')",
                [&session_id],
            )
            .unwrap();
        connection
            .execute(
                "WITH RECURSIVE seq(value) AS (
                     SELECT 1 UNION ALL SELECT value + 1 FROM seq WHERE value < 63
                 )
                 INSERT INTO candidate_plan_claims (
                     candidate_id, session_id, item_ordinal, prior_review_status
                 )
                 SELECT ?2 || printf(':%03d', value), ?1, value, 'discovered'
                 FROM seq",
                params![session_id, candidate_prefix],
            )
            .unwrap();
        connection
            .execute(
                "WITH RECURSIVE seq(value) AS (
                     SELECT 1 UNION ALL SELECT value + 1 FROM seq WHERE value < 63
                 )
                 INSERT INTO trusted_rust_target_plan_claims (
                     candidate_id, session_id, item_ordinal, coupling_revision
                 )
                 SELECT ?2 || printf(':%03d', value), ?1, value, 1
                 FROM seq",
                params![session_id, candidate_prefix],
            )
            .unwrap();
    }

    fn current_provenance() -> ExecutionProvenance {
        let owner = ProcessInstanceId::from_stored(&owner(0x11, 1)).unwrap();
        ExecutionProvenance::from_stored(&owner, &[0x33; 32], &[0x11; 32]).unwrap()
    }

    fn mutable_cleanup_graph_snapshot(connection: &Connection) -> Vec<String> {
        let mut snapshot = Vec::new();
        for table in [
            "cleanup_sessions",
            "cleanup_items",
            "cleanup_item_paths",
            "cleanup_item_evidence",
            "cleanup_plan_warnings",
            "candidates",
            "candidate_paths",
            "candidate_evidence",
            "candidate_plan_claims",
            "trusted_rust_target_plan_claims",
        ] {
            let columns = connection
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let projection = columns
                .iter()
                .map(|column| format!("quote(\"{column}\")"))
                .collect::<Vec<_>>()
                .join(", ");
            let mut statement = connection
                .prepare(&format!("SELECT {projection} FROM {table} ORDER BY rowid"))
                .unwrap();
            let mut rows = statement.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                snapshot.push(table.to_owned());
                for index in 0..columns.len() {
                    snapshot.push(row.get::<_, String>(index).unwrap());
                }
            }
        }
        snapshot
    }

    #[test]
    fn census_is_read_only_and_partitions_phase_and_provenance() {
        let (_temp, store) = fixture();
        store.with_connection(|connection| {
            insert_active(
                connection,
                0,
                "running",
                &owner(0x11, 1),
                1,
                Some(&[0x33; 32]),
                Some(&[0x11; 32]),
                Some("resumable"),
            );
            insert_active(
                connection,
                1,
                "recovering",
                &owner(0x44, 2),
                2,
                Some(&[0x33; 32]),
                Some(&[0x44; 32]),
                Some("resumable"),
            );
            insert_active(
                connection,
                2,
                "running",
                &owner(0x11, 3),
                3,
                Some(&[0x55; 32]),
                Some(&[0x11; 32]),
                Some("resumable"),
            );
            insert_active(
                connection,
                3,
                "recovering",
                &owner(0x11, 4),
                4,
                None,
                None,
                None,
            );
        });
        let current = current_provenance();
        let changes_before = store.with_connection(Connection::total_changes);
        let graph_before = store.with_connection(mutable_cleanup_graph_snapshot);
        let census = store.with_connection(|connection| {
            load_cleanup_recovery_diagnostic_census(connection, Some(&current)).unwrap()
        });
        assert_eq!(
            census,
            CleanupRecoveryDiagnosticCensus {
                active_total: 4,
                running_count: 2,
                recovering_count: 2,
                same_host_current_boot_count: 1,
                same_host_prior_boot_count: 1,
                foreign_host_count: 1,
                stored_unproven_count: 1,
                current_context_unavailable_count: 0,
                has_more: false,
            }
        );
        assert_eq!(
            store.with_connection(Connection::total_changes),
            changes_before
        );
        assert_eq!(
            store.with_connection(mutable_cleanup_graph_snapshot),
            graph_before
        );

        let unavailable = store.with_connection(|connection| {
            load_cleanup_recovery_diagnostic_census(connection, None).unwrap()
        });
        assert_eq!(unavailable.stored_unproven_count, 1);
        assert_eq!(unavailable.current_context_unavailable_count, 3);
        assert_eq!(unavailable.same_host_current_boot_count, 0);
        assert_eq!(unavailable.same_host_prior_boot_count, 0);
        assert_eq!(unavailable.foreign_host_count, 0);
        assert_eq!(
            store.with_connection(Connection::total_changes),
            changes_before
        );
        assert_eq!(
            store.with_connection(mutable_cleanup_graph_snapshot),
            graph_before
        );
    }

    #[test]
    fn census_classifies_active_legacy_v1_as_stored_unproven() {
        let (_temp, store) = fixture();
        store.with_connection(|connection| insert_legacy_active(connection, 0));
        let changes_before = store.with_connection(Connection::total_changes);
        let census = store.with_connection(|connection| {
            load_cleanup_recovery_diagnostic_census(connection, Some(&current_provenance()))
                .unwrap()
        });
        assert_eq!(census.active_total, 1);
        assert_eq!(census.running_count, 1);
        assert_eq!(census.recovering_count, 0);
        assert_eq!(census.stored_unproven_count, 1);
        assert_eq!(census.current_context_unavailable_count, 0);
        assert_eq!(
            store.with_connection(Connection::total_changes),
            changes_before
        );
    }

    #[test]
    fn census_uses_recovery_index_and_strictly_validates_lookahead() {
        let (_temp, store) = fixture();
        store.with_connection(|connection| {
            for ordinal in 0..=MAX_CLEANUP_RECOVERY_DIAGNOSTIC_ROWS {
                insert_active(
                    connection,
                    ordinal,
                    "running",
                    &owner(0x11, ordinal as u128 + 1),
                    ordinal as i64,
                    None,
                    None,
                    None,
                );
            }
            let plan = connection
                .prepare(&format!(
                    "EXPLAIN QUERY PLAN {}",
                    cleanup_recovery_diagnostic_query()
                ))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .join("\n");
            assert!(plan.contains("cleanup_sessions_by_recovery"), "{plan}");
        });

        let census = store.with_connection(|connection| {
            load_cleanup_recovery_diagnostic_census(connection, Some(&current_provenance()))
                .unwrap()
        });
        assert_eq!(census.active_total, 64);
        assert_eq!(census.running_count, 64);
        assert_eq!(census.stored_unproven_count, 64);
        assert!(census.has_more);

        store.with_connection(|connection| {
            connection
                .execute_batch("PRAGMA ignore_check_constraints = ON")
                .unwrap();
            connection
                .execute(
                    "UPDATE cleanup_item_paths SET status = 'effect_started'
                     WHERE session_id = 'cleanup:census:064'",
                    [],
                )
                .unwrap();
        });
        let changes_before = store.with_connection(Connection::total_changes);
        let error = store.with_connection(|connection| {
            load_cleanup_recovery_diagnostic_census(connection, None).unwrap_err()
        });
        assert_eq!(error.kind, HistoryErrorKind::CorruptData);
        assert_eq!(
            store.with_connection(Connection::total_changes),
            changes_before
        );
    }

    #[test]
    fn census_accepts_a_legal_maximum_page_of_maximum_scalar_graphs() {
        let (_temp, store) = fixture();
        store.with_connection(|connection| {
            connection.execute_batch("BEGIN IMMEDIATE").unwrap();
            for ordinal in 0..=MAX_CLEANUP_RECOVERY_DIAGNOSTIC_ROWS {
                insert_active(
                    connection,
                    ordinal,
                    "running",
                    &owner(0x11, ordinal as u128 + 1),
                    ordinal as i64,
                    None,
                    None,
                    None,
                );
                expand_active_to_maximum_scalar_graph(connection, ordinal);
            }
            connection.execute_batch("COMMIT").unwrap();
        });

        let changes_before = store.with_connection(Connection::total_changes);
        let census = store.with_connection(|connection| {
            load_cleanup_recovery_diagnostic_census(connection, Some(&current_provenance()))
                .unwrap()
        });
        assert_eq!(census.active_total, 64);
        assert_eq!(census.running_count, 64);
        assert_eq!(census.stored_unproven_count, 64);
        assert!(census.has_more);
        assert_eq!(
            store.with_connection(Connection::total_changes),
            changes_before
        );
    }

    #[test]
    fn census_query_budget_exhaustion_is_typed_and_removes_its_handler() {
        let (_temp, store) = fixture();
        store.with_connection(|connection| {
            insert_active(
                connection,
                0,
                "running",
                &owner(0x11, 1),
                0,
                None,
                None,
                None,
            );
        });
        let error = store.with_connection(|connection| {
            run_bounded_cleanup_recovery_diagnostic_query_for_test(
                connection,
                1,
                Duration::from_secs(1),
                || load_cleanup_recovery_diagnostic_census_within_budget(connection, None),
            )
            .unwrap_err()
        });
        assert_eq!(error.kind, HistoryErrorKind::QueryLimitExceeded);

        let census = store.with_connection(|connection| {
            load_cleanup_recovery_diagnostic_census(connection, None).unwrap()
        });
        assert_eq!(census.active_total, 1);
        assert_eq!(census.stored_unproven_count, 1);
    }

    #[test]
    fn census_rejects_partial_unknown_and_owner_inconsistent_provenance() {
        for corruption in ["partial", "unknown-policy", "owner-mismatch"] {
            let (_temp, store) = fixture();
            store.with_connection(|connection| {
                insert_active(
                    connection,
                    0,
                    "running",
                    &owner(0x11, 1),
                    1,
                    Some(&[0x33; 32]),
                    Some(&[0x11; 32]),
                    Some("resumable"),
                );
                connection
                    .execute_batch("PRAGMA ignore_check_constraints = ON")
                    .unwrap();
                match corruption {
                    "partial" => connection
                        .execute(
                            "UPDATE cleanup_sessions
                             SET execution_boot_scope_v1_sha256 = NULL,
                                 execution_recovery_policy = NULL",
                            [],
                        )
                        .unwrap(),
                    "unknown-policy" => connection
                        .execute(
                            "UPDATE cleanup_sessions
                             SET execution_recovery_policy = 'future-policy'",
                            [],
                        )
                        .unwrap(),
                    "owner-mismatch" => connection
                        .execute(
                            "UPDATE cleanup_sessions
                             SET execution_boot_scope_v1_sha256 = ?1",
                            [&[0x44_u8; 32] as &[u8]],
                        )
                        .unwrap(),
                    _ => unreachable!(),
                };
            });
            let changes_before = store.with_connection(Connection::total_changes);
            let error = store.with_connection(|connection| {
                load_cleanup_recovery_diagnostic_census(connection, None).unwrap_err()
            });
            assert_eq!(error.kind, HistoryErrorKind::CorruptData, "{corruption}");
            assert_eq!(
                store.with_connection(Connection::total_changes),
                changes_before
            );
        }
    }
}
