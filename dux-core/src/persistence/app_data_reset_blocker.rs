//! Path-free durable blockers for application-data reset admission.
//!
//! These scalar observations carry no row identifier, path, cleanup target, or
//! effect authority. Reset admission must retain the store locks separately.

use rusqlite::Connection;

use super::history::{HistoryError, HistoryErrorKind, map_query_sql_error, run_bounded_query};

const BLOCKER_QUERY: &str = "
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

/// Bounded, path-free reasons why a store cannot enter app-data reset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AppDataResetStoreBlockers {
    cleanup_lock_busy: bool,
    active_cleanup: bool,
    uncertain_cleanup_effect: bool,
    active_scan_evidence: bool,
    scan_scope_lease: bool,
}

impl AppDataResetStoreBlockers {
    pub(super) const fn cleanup_lock_busy() -> Self {
        Self {
            cleanup_lock_busy: true,
            active_cleanup: false,
            uncertain_cleanup_effect: false,
            active_scan_evidence: false,
            scan_scope_lease: false,
        }
    }

    pub(crate) const fn is_empty(self) -> bool {
        !self.cleanup_lock_busy
            && !self.active_cleanup
            && !self.uncertain_cleanup_effect
            && !self.active_scan_evidence
            && !self.scan_scope_lease
    }

    pub(crate) const fn cleanup_lock_is_busy(self) -> bool {
        self.cleanup_lock_busy
    }

    pub(crate) const fn has_active_cleanup(self) -> bool {
        self.active_cleanup
    }

    pub(crate) const fn has_uncertain_cleanup_effect(self) -> bool {
        self.uncertain_cleanup_effect
    }

    pub(crate) const fn has_active_scan_evidence(self) -> bool {
        self.active_scan_evidence
    }

    pub(crate) const fn has_scan_scope_lease(self) -> bool {
        self.scan_scope_lease
    }
}

pub(super) fn inspect_app_data_reset_store_blockers(
    connection: &Connection,
) -> Result<AppDataResetStoreBlockers, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(BLOCKER_QUERY, [], |row| {
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
        let active_cleanup = decode_flag(raw.0)?;
        let uncertain_cleanup_item = decode_flag(raw.1)?;
        let uncertain_cleanup_path = decode_flag(raw.2)?;
        let running_scan = decode_flag(raw.3)?;
        let process_claim = decode_flag(raw.4)?;
        Ok(AppDataResetStoreBlockers {
            cleanup_lock_busy: false,
            active_cleanup,
            uncertain_cleanup_effect: uncertain_cleanup_item || uncertain_cleanup_path,
            active_scan_evidence: running_scan || process_claim,
            scan_scope_lease: decode_flag(raw.5)?,
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
    use rusqlite::{Connection, params};

    use super::*;
    use crate::persistence::migrations::test_migrations;

    fn current_schema() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        for migration in test_migrations() {
            connection.execute_batch(migration.sql).unwrap();
        }
        connection
    }

    fn insert_cleanup_session(connection: &Connection, status: &str) {
        connection
            .execute(
                "INSERT INTO cleanup_sessions (
                     session_id, plan_id, started_at_unix_ms, completed_at_unix_ms,
                     mode, estimated_bytes, verified_capacity_delta_bytes,
                     trigger_source, status, record_format_version
                 ) VALUES (
                     'session', 'plan', 1, NULL, 'dry_run', 0, NULL,
                     'manual', ?1, 1
                 )",
                [status],
            )
            .unwrap();
    }

    fn insert_cleanup_item(connection: &Connection, status: &str) {
        insert_cleanup_session(connection, "planned");
        connection
            .execute(
                "INSERT INTO cleanup_items (
                     item_id, session_id, item_ordinal, rule_id, rule_revision,
                     estimated_bytes, final_status, error_category,
                     record_format_version, legacy_target_path,
                     legacy_target_path_encoding
                 ) VALUES (
                     1, 'session', 0, 'rule', 1, 0, ?1, NULL, 1, x'2f746d70', 1
                 )",
                [status],
            )
            .unwrap();
    }

    #[test]
    fn empty_store_has_no_durable_blocker() {
        let connection = current_schema();
        assert!(
            inspect_app_data_reset_store_blockers(&connection)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn active_cleanup_and_uncertain_item_are_distinct_path_free_bits() {
        let running = current_schema();
        insert_cleanup_session(&running, "running");
        let blockers = inspect_app_data_reset_store_blockers(&running).unwrap();
        assert!(blockers.has_active_cleanup());
        assert!(!blockers.has_uncertain_cleanup_effect());

        let uncertain = current_schema();
        insert_cleanup_item(&uncertain, "outcome_unknown");
        let blockers = inspect_app_data_reset_store_blockers(&uncertain).unwrap();
        assert!(!blockers.has_active_cleanup());
        assert!(blockers.has_uncertain_cleanup_effect());
    }

    #[test]
    fn uncertain_path_blocks_even_when_parent_item_is_not_uncertain() {
        let connection = current_schema();
        insert_cleanup_item(&connection, "planned");
        connection
            .execute(
                "INSERT INTO cleanup_item_paths (
                     session_id, item_ordinal, path_ordinal, target_path,
                     target_path_encoding, attempt_generation, status,
                     error_category, effect_started_at_unix_ms,
                     completed_at_unix_ms
                 ) VALUES (
                     'session', 0, 0, x'2f746d702f6368696c64', 1, 1,
                     'effect_started', NULL, 2, NULL
                 )",
                [],
            )
            .unwrap();
        let blockers = inspect_app_data_reset_store_blockers(&connection).unwrap();
        assert!(blockers.has_uncertain_cleanup_effect());
    }

    #[test]
    fn running_scan_and_scope_lease_are_distinct_bits() {
        let running = current_schema();
        running
            .execute(
                "INSERT INTO scans (
                     scan_id, root_path, root_path_encoding, started_at_unix_ms, status
                 ) VALUES ('scan', x'2f746d70', 1, 1, 'running')",
                [],
            )
            .unwrap();
        let blockers = inspect_app_data_reset_store_blockers(&running).unwrap();
        assert!(blockers.has_active_scan_evidence());
        assert!(!blockers.has_scan_scope_lease());

        let leased = current_schema();
        leased
            .execute(
                "INSERT INTO scan_scope_leases (
                     lease_id, record_format_version, root_path, root_path_encoding,
                     owner_process_instance, recovery_scope, acquired_at_unix_ms,
                     execution_host_identity_v1_sha256,
                     execution_boot_scope_v1_sha256, execution_recovery_policy
                 ) VALUES (
                     ?1, 1, x'2f746d70', 1, 'owner', NULL, 1, NULL, NULL, NULL
                 )",
                params![&[7_u8; 16]],
            )
            .unwrap();
        let blockers = inspect_app_data_reset_store_blockers(&leased).unwrap();
        assert!(!blockers.has_active_scan_evidence());
        assert!(blockers.has_scan_scope_lease());
    }

    #[test]
    fn orphan_process_claim_fails_closed_as_active_scan_evidence() {
        let connection = current_schema();
        connection
            .execute_batch(
                "DROP TRIGGER scan_process_claims_insert_guard;
                 PRAGMA foreign_keys = OFF;",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO scan_process_claims (
                     scan_id, record_format_version, owner_process_instance,
                     recovery_scope, claimed_at_unix_ms,
                     execution_host_identity_v1_sha256,
                     execution_boot_scope_v1_sha256, execution_recovery_policy
                 ) VALUES (
                     'orphan', 1, 'owner', NULL, 1, NULL, NULL, NULL
                 )",
                [],
            )
            .unwrap();
        let blockers = inspect_app_data_reset_store_blockers(&connection).unwrap();
        assert!(blockers.has_active_scan_evidence());
    }
}
