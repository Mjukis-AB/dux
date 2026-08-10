//! Bounded, read-only aggregate work facts for core automation blockers.
//!
//! No row identity, path, selector, recovery handle, or mutation crosses this
//! adapter. Durable scan/cleanup rows are deliberately classified as
//! unresolved because this query does not claim their current-process
//! liveness.

use rusqlite::Connection;

use super::history::{HistoryError, HistoryErrorKind, map_query_sql_error, run_bounded_query};

const AUTOMATION_RUNTIME_WORK_QUERY: &str = "
    SELECT
        EXISTS(
            SELECT 1
            FROM cleanup_sessions INDEXED BY cleanup_sessions_by_recovery
            WHERE status IN ('running', 'recovering')
            LIMIT 1
        ),
        EXISTS(
            SELECT 1
            FROM cleanup_items INDEXED BY cleanup_items_by_unresolved_effect
            WHERE final_status IN ('effect_started', 'outcome_unknown')
            LIMIT 1
        ),
        EXISTS(
            SELECT 1
            FROM cleanup_item_paths INDEXED BY cleanup_item_paths_by_unresolved_effect
            WHERE status IN ('effect_started', 'outcome_unknown')
            LIMIT 1
        ),
        EXISTS(
            SELECT 1
            FROM scans INDEXED BY scans_running_by_started
            WHERE status = 'running'
            LIMIT 1
        ),
        EXISTS(SELECT 1 FROM scan_process_claims LIMIT 1),
        EXISTS(SELECT 1 FROM scan_scope_leases LIMIT 1)
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StoredAutomationRuntimeObservation {
    durable_scan_work_unresolved: bool,
    cleanup_work_active: bool,
    durable_cleanup_work_unresolved: bool,
    cross_process_admission_unresolved: bool,
}

impl StoredAutomationRuntimeObservation {
    pub(crate) const fn scan_work_unresolved(self) -> bool {
        self.cross_process_admission_unresolved || self.durable_scan_work_unresolved
    }

    pub(crate) const fn cleanup_work_active(self) -> bool {
        self.cleanup_work_active
    }

    pub(crate) const fn cleanup_work_unresolved(self) -> bool {
        self.cross_process_admission_unresolved || self.durable_cleanup_work_unresolved
    }

    pub(super) const fn cleanup_contention() -> Self {
        Self {
            durable_scan_work_unresolved: false,
            cleanup_work_active: true,
            durable_cleanup_work_unresolved: false,
            cross_process_admission_unresolved: true,
        }
    }

    #[cfg(test)]
    pub(crate) const fn clear_for_test() -> Self {
        Self {
            durable_scan_work_unresolved: false,
            cleanup_work_active: false,
            durable_cleanup_work_unresolved: false,
            cross_process_admission_unresolved: false,
        }
    }

    #[cfg(test)]
    pub(crate) const fn unresolved_for_test() -> Self {
        Self {
            durable_scan_work_unresolved: true,
            cleanup_work_active: false,
            durable_cleanup_work_unresolved: true,
            cross_process_admission_unresolved: true,
        }
    }
}

pub(super) fn inspect_automation_runtime_work(
    connection: &Connection,
) -> Result<StoredAutomationRuntimeObservation, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(AUTOMATION_RUNTIME_WORK_QUERY, [], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(map_query_sql_error)?;
        let cleanup_session = decode_flag(raw.0)?;
        let cleanup_item = decode_flag(raw.1)?;
        let cleanup_path = decode_flag(raw.2)?;
        let running_scan = decode_flag(raw.3)?;
        let scan_claim = decode_flag(raw.4)?;
        let scan_lease = decode_flag(raw.5)?;
        // Scan and cleanup admission is recorded in the admitting process
        // before every worker publishes a durable row or retained exclusion.
        // This aggregate query therefore cannot prove that another process
        // has no just-admitted work, even when every durable table is empty.
        Ok(StoredAutomationRuntimeObservation {
            durable_scan_work_unresolved: running_scan || scan_claim || scan_lease,
            cleanup_work_active: false,
            durable_cleanup_work_unresolved: cleanup_session || cleanup_item || cleanup_path,
            cross_process_admission_unresolved: true,
        })
    })
}

fn decode_flag(value: i64) -> Result<bool, HistoryError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(HistoryError::new(HistoryErrorKind::CorruptData)),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::persistence::migrations::test_migrations;

    fn current_schema() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        for migration in test_migrations() {
            connection.execute_batch(migration.sql).unwrap();
        }
        connection
    }

    #[test]
    fn empty_current_store_keeps_cross_process_admission_unproven() {
        let connection = current_schema();
        let observation = inspect_automation_runtime_work(&connection).unwrap();
        assert!(observation.scan_work_unresolved());
        assert!(observation.cleanup_work_unresolved());
        assert!(!observation.cleanup_work_active());
        assert_eq!(connection.changes(), 0);
    }

    #[test]
    fn durable_scan_rows_are_unresolved_without_disclosing_identity() {
        for statement in [
            "INSERT INTO scans (
                 scan_id, root_path, root_path_encoding, started_at_unix_ms, status
             ) VALUES ('scan', x'2f746d70', 1, 1, 'running')",
            "INSERT INTO scan_scope_leases (
                 lease_id, record_format_version, root_path, root_path_encoding,
                 owner_process_instance, recovery_scope, acquired_at_unix_ms,
                 execution_host_identity_v1_sha256,
                 execution_boot_scope_v1_sha256, execution_recovery_policy
             ) VALUES (
                 x'0102030405060708090a0b0c0d0e0f10', 1, x'2f746d70', 1,
                 'owner', NULL, 1, NULL, NULL, NULL
             )",
        ] {
            let connection = current_schema();
            connection.execute_batch(statement).unwrap();
            let observation = inspect_automation_runtime_work(&connection).unwrap();
            assert!(observation.scan_work_unresolved());
            assert!(!observation.cleanup_work_active());
            assert!(observation.cleanup_work_unresolved());
        }
    }

    #[test]
    fn durable_cleanup_rows_are_unresolved_without_becoming_active_claims() {
        let connection = current_schema();
        connection
            .execute_batch(
                "INSERT INTO cleanup_sessions (
                     session_id, plan_id, started_at_unix_ms, completed_at_unix_ms,
                     mode, estimated_bytes, verified_capacity_delta_bytes,
                     trigger_source, status, record_format_version
                 ) VALUES (
                     'session', 'plan', 1, NULL, 'dry_run', 0, NULL,
                     'manual', 'running', 1
                 )",
            )
            .unwrap();
        let observation = inspect_automation_runtime_work(&connection).unwrap();
        assert!(observation.cleanup_work_unresolved());
        assert!(!observation.cleanup_work_active());
        assert!(observation.scan_work_unresolved());
    }

    #[test]
    fn malformed_aggregate_flags_fail_closed() {
        assert_eq!(
            decode_flag(2).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }
}
