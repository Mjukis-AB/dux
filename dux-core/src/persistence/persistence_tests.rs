use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, params};
use tempfile::TempDir;

use super::cleanup_history::{
    CandidateStatusCoupling, CleanupSessionId, StoredCleanupSessionRecord,
    load_frozen_cleanup_session_within_budget,
};
use super::codec::{
    CodecError, EncodedBytes, StoredEncoding, decode_host_path, decode_logical_key,
    encode_host_path, encode_logical_key,
};
use super::migrations::{
    DUX_APPLICATION_ID, Migration, SchemaState, apply_pending_migrations, apply_test_chain,
    apply_test_upgrade_chain, inspect_schema, inspect_schema_with_test_budget,
    panic_with_test_budget, schema_fingerprint, test_migrations, test_v1_schema_fingerprint,
    test_v2_schema_fingerprint, test_v3_schema_fingerprint, test_v4_schema_fingerprint,
    test_v5_schema_fingerprint, validate_compiled_migrations,
};
use super::process_liveness::current_process_instance;
#[cfg(unix)]
use super::process_liveness::{ProcessInstanceId, ProcessLiveness, probe_process_instance};
use super::storage::SecureStorePaths;
use super::*;

fn database_path(temp: &TempDir) -> std::path::PathBuf {
    temp.path().join("data/dux.sqlite3")
}

fn initialization_path(database: &Path) -> PathBuf {
    let mut name = database.file_name().unwrap().to_os_string();
    name.push(".initialized");
    database.with_file_name(name)
}

const SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(10);
const SUBPROCESS_HELPER_TEST: &str = "persistence::tests::sqlite_subprocess_helper";
const FUTURE_SCHEMA_VERSION: u32 = DATABASE_SCHEMA_VERSION + 1;

struct TestChild {
    child: Child,
}

impl TestChild {
    fn try_status(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().unwrap()
    }

    #[cfg(unix)]
    fn terminate_without_unwinding(&mut self) {
        self.child.kill().unwrap();
        let status = self.child.wait().unwrap();
        assert!(!status.success());
    }

    fn wait_for_success(mut self) {
        let deadline = Instant::now() + SUBPROCESS_TIMEOUT;
        loop {
            if let Some(status) = self.try_status() {
                assert!(status.success(), "persistence helper failed: {status}");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "persistence helper exceeded its bounded runtime"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for TestChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "the persistence process-boundary tests relaunch only their exact ignored test helper"
)]
fn spawn_persistence_helper(mode: &str, paths: &[(&str, &Path)]) -> TestChild {
    let executable = std::env::current_exe().unwrap();
    // DUX-DESTRUCTIVE: allow=test-persistence-helper-spawn -- relaunch only this exact ignored test executable helper with fixed harness arguments
    let mut command = Command::new(executable);
    command
        .args([
            "--ignored",
            "--exact",
            SUBPROCESS_HELPER_TEST,
            "--test-threads=1",
        ])
        .env("DUX_PERSISTENCE_HELPER_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (name, path) in paths {
        command.env(name, path);
    }
    TestChild {
        child: command.spawn().unwrap(),
    }
}

fn helper_path(name: &str) -> PathBuf {
    std::env::var_os(name).map(PathBuf::from).unwrap()
}

fn publish_handshake(path: &Path) {
    publish_bytes(path, b"ready");
}

fn publish_bytes(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

fn wait_for_handshake(path: &Path) {
    let deadline = Instant::now() + SUBPROCESS_TIMEOUT;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "persistence helper handshake timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_child_handshake(child: &mut TestChild, path: &Path) {
    let deadline = Instant::now() + SUBPROCESS_TIMEOUT;
    while !path.exists() {
        assert!(
            child.try_status().is_none(),
            "persistence helper exited before its handshake"
        );
        assert!(
            Instant::now() < deadline,
            "persistence helper handshake timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn switch_to_delete_journal(path: &Path) {
    let connection = Connection::open(path).unwrap();
    let mode: String = connection
        .query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode.to_ascii_lowercase(), "delete");
}

#[cfg(unix)]
fn helper_leave_hot_rollback_journal() {
    let database = helper_path("DUX_PERSISTENCE_DATABASE");
    let ready = helper_path("DUX_PERSISTENCE_READY");
    let connection = Connection::open(&database).unwrap();
    connection
        .pragma_update(None, "synchronous", "FULL")
        .unwrap();
    connection.pragma_update(None, "cache_size", 1).unwrap();
    let mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(mode.to_ascii_lowercase(), "delete");
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    let uncommitted_json = format!("\"{}\"", "x".repeat(512 * 1024));
    connection
        .execute(
            "UPDATE settings SET value_json = ?1, updated_at_unix_ms = 2 \
             WHERE setting_key = 'crash-sentinel'",
            [&uncommitted_json],
        )
        .unwrap();
    publish_handshake(&ready);
    wait_for_handshake(&helper_path("DUX_PERSISTENCE_RELEASE"));
    unreachable!("the parent must terminate this helper without SQLite destructors");
}

#[cfg(unix)]
fn helper_leave_wal_without_shared_memory_recovery() {
    let database = helper_path("DUX_PERSISTENCE_DATABASE");
    let ready = helper_path("DUX_PERSISTENCE_READY");
    let connection = Connection::open(&database).unwrap();
    connection
        .pragma_update(None, "synchronous", "FULL")
        .unwrap();
    connection
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    connection.pragma_update(None, "cache_size", 1).unwrap();
    let mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    connection
        .execute(
            "UPDATE settings SET value_json = '\"wal-committed\"', updated_at_unix_ms = 2 \
             WHERE setting_key = 'crash-sentinel'",
            [],
        )
        .unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    let uncommitted_json = format!("\"{}\"", "x".repeat(512 * 1024));
    connection
        .execute(
            "UPDATE settings SET value_json = ?1, updated_at_unix_ms = 3 \
             WHERE setting_key = 'crash-sentinel'",
            [&uncommitted_json],
        )
        .unwrap();
    publish_handshake(&ready);
    wait_for_handshake(&helper_path("DUX_PERSISTENCE_RELEASE"));
    unreachable!("the parent must terminate this helper without SQLite destructors");
}

fn helper_upgrade_while_holding_writer_lock() {
    let database = helper_path("DUX_PERSISTENCE_DATABASE");
    let ready = helper_path("DUX_PERSISTENCE_READY");
    let paths = SecureStorePaths::prepare(&database).unwrap();
    let sqlite_path = paths.sqlite_path().unwrap();
    let _writer_lock = paths.acquire_writer_lock(SUBPROCESS_TIMEOUT).unwrap();
    let connection = Connection::open(sqlite_path).unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations \
             (version, name, checksum_sha256, applied_at_unix_ms) \
             VALUES (?1, 'subprocess-future-schema', zeroblob(32), 2)",
            [i64::from(FUTURE_SCHEMA_VERSION)],
        )
        .unwrap();
    connection
        .pragma_update(None, "user_version", FUTURE_SCHEMA_VERSION)
        .unwrap();
    drop(connection);
    publish_handshake(&ready);
    wait_for_handshake(&helper_path("DUX_PERSISTENCE_RELEASE"));
}

fn helper_open_after_writer_lock_race() {
    let database = helper_path("DUX_PERSISTENCE_DATABASE");
    let probed = helper_path("DUX_PERSISTENCE_READY");
    let continue_probe = helper_path("DUX_PERSISTENCE_RELEASE");
    let acquiring = helper_path("DUX_PERSISTENCE_ACQUIRING");
    let paths = SecureStorePaths::prepare(&database).unwrap();
    let sqlite_path = paths.sqlite_path().unwrap();
    let store = StoreCoordinator::open_unregistered_for_test(paths, &sqlite_path, || {
        publish_handshake(&probed);
        wait_for_handshake(&continue_probe);
        publish_handshake(&acquiring);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        store.status().unwrap(),
        DatabaseStatus {
            schema_version: FUTURE_SCHEMA_VERSION,
            access: DatabaseAccess::ReadOnlyNewer {
                found: FUTURE_SCHEMA_VERSION,
                supported: DATABASE_SCHEMA_VERSION,
            },
        }
    );
    store.with_connection(|connection| {
        let mode: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_ascii_lowercase(), "delete");
        assert!(
            connection
                .execute(
                    "INSERT INTO settings \
                     (setting_key, value_json, value_schema_version, updated_at_unix_ms) \
                     VALUES ('forbidden-subprocess-race', '{}', 1, 2)",
                    [],
                )
                .is_err()
        );
    });
}

fn helper_hold_cleanup_lock() {
    let database = helper_path("DUX_PERSISTENCE_DATABASE");
    let ready = helper_path("DUX_PERSISTENCE_READY");
    let release = helper_path("DUX_PERSISTENCE_RELEASE");
    let paths = SecureStorePaths::prepare(&database).unwrap();
    let guard = paths.acquire_cleanup_lock(SUBPROCESS_TIMEOUT).unwrap();
    paths.validate_cleanup_lock_guard(&guard).unwrap();
    publish_handshake(&ready);
    wait_for_handshake(&release);
    paths.validate_cleanup_lock_guard(&guard).unwrap();
}

fn helper_hold_process_instance() {
    let identity_path = helper_path("DUX_PERSISTENCE_IDENTITY");
    let ready = helper_path("DUX_PERSISTENCE_READY");
    let release = helper_path("DUX_PERSISTENCE_RELEASE");
    let identity = current_process_instance().unwrap();
    publish_bytes(&identity_path, identity.as_str().as_bytes());
    publish_handshake(&ready);
    wait_for_handshake(&release);
}

#[test]
#[ignore = "launched by the process-boundary persistence regressions"]
fn sqlite_subprocess_helper() {
    match std::env::var("DUX_PERSISTENCE_HELPER_MODE")
        .unwrap()
        .as_str()
    {
        #[cfg(unix)]
        "hot-rollback-journal" => helper_leave_hot_rollback_journal(),
        #[cfg(unix)]
        "hot-wal" => helper_leave_wal_without_shared_memory_recovery(),
        "upgrade-under-writer-lock" => helper_upgrade_while_holding_writer_lock(),
        "open-after-writer-lock-race" => helper_open_after_writer_lock_race(),
        "hold-cleanup-lock" => helper_hold_cleanup_lock(),
        "hold-process-instance" => helper_hold_process_instance(),
        _ => panic!("unknown persistence helper mode"),
    }
}

fn fresh_v1_schema() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(test_migrations()[0].sql).unwrap();
    connection
}

fn fresh_v2_schema() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for migration in &test_migrations()[..2] {
        connection.execute_batch(migration.sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations (
                     version, name, checksum_sha256, applied_at_unix_ms
                 ) VALUES (?1, ?2, ?3, 1)",
                params![
                    i64::from(migration.version),
                    migration.name,
                    migration.checksum_sha256.as_slice(),
                ],
            )
            .unwrap();
        connection
            .pragma_update(None, "application_id", DUX_APPLICATION_ID)
            .unwrap();
        connection
            .pragma_update(None, "user_version", migration.version)
            .unwrap();
    }
    connection
}

fn fresh_v3_schema() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for migration in &test_migrations()[..3] {
        connection.execute_batch(migration.sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations (
                     version, name, checksum_sha256, applied_at_unix_ms
                 ) VALUES (?1, ?2, ?3, 1)",
                params![
                    i64::from(migration.version),
                    migration.name,
                    migration.checksum_sha256.as_slice(),
                ],
            )
            .unwrap();
        connection
            .pragma_update(None, "application_id", DUX_APPLICATION_ID)
            .unwrap();
        connection
            .pragma_update(None, "user_version", migration.version)
            .unwrap();
    }
    connection
}

fn fresh_v4_schema() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for migration in &test_migrations()[..4] {
        connection.execute_batch(migration.sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations (
                     version, name, checksum_sha256, applied_at_unix_ms
                 ) VALUES (?1, ?2, ?3, 1)",
                params![
                    i64::from(migration.version),
                    migration.name,
                    migration.checksum_sha256.as_slice(),
                ],
            )
            .unwrap();
        connection
            .pragma_update(None, "application_id", DUX_APPLICATION_ID)
            .unwrap();
        connection
            .pragma_update(None, "user_version", migration.version)
            .unwrap();
    }
    connection
}

fn fresh_current_schema() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for migration in test_migrations() {
        connection.execute_batch(migration.sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations (
                     version, name, checksum_sha256, applied_at_unix_ms
                 ) VALUES (?1, ?2, ?3, 1)",
                params![
                    i64::from(migration.version),
                    migration.name,
                    migration.checksum_sha256.as_slice(),
                ],
            )
            .unwrap();
        connection
            .pragma_update(None, "application_id", DUX_APPLICATION_ID)
            .unwrap();
        connection
            .pragma_update(None, "user_version", migration.version)
            .unwrap();
    }
    connection
}

fn install_v2_schema(connection: &Connection) {
    for migration in &test_migrations()[..2] {
        connection.execute_batch(migration.sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations (
                     version, name, checksum_sha256, applied_at_unix_ms
                 ) VALUES (?1, ?2, ?3, 1)",
                params![
                    i64::from(migration.version),
                    migration.name,
                    migration.checksum_sha256.as_slice(),
                ],
            )
            .unwrap();
        connection
            .pragma_update(None, "application_id", DUX_APPLICATION_ID)
            .unwrap();
        connection
            .pragma_update(None, "user_version", migration.version)
            .unwrap();
    }
}

fn insert_v2_format2_cleanup_session(
    connection: &Connection,
    label: &str,
    status: &str,
    owner: &str,
) {
    let session_id = format!("session:v2-{label}");
    let plan_id = format!("plan:v2-{label}");
    let candidate_id = format!("candidate:v2-{label}");
    let target = format!("/v2/{label}/cache").into_bytes();
    let (completed_at, capacity_delta, execution_owner, execution_generation, heartbeat) =
        match status {
            "planned" => (None, None, None, None, None),
            "running" | "recovering" => (None, None, Some(owner), Some(1_i64), Some(1_000_600_i64)),
            "completed" => (
                Some(1_000_800_i64),
                Some(8_i64),
                Some(owner),
                Some(1_i64),
                Some(1_000_600_i64),
            ),
            _ => panic!("unsupported v2 cleanup fixture status: {status}"),
        };
    let (item_status, attempt_generation, effect_started, path_completed) = if status == "completed"
    {
        (
            "removed",
            Some(1_i64),
            Some(1_000_700_i64),
            Some(1_000_750_i64),
        )
    } else {
        ("planned", None, None, None)
    };

    connection
        .execute(
            "INSERT INTO candidates (
                 candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                 estimated_bytes, created_at_unix_ms, status, record_format_version,
                 category, proposed_action, rule_schedule_eligible
             ) VALUES (
                 ?1, 'scan:v2-lifecycle', 'fixture.v2-lifecycle', 1,
                 'safe_regenerable', 8, 999000, 'discovered', 2,
                 'application_cache', 'remove_known_regenerable_contents', 0
             )",
            [&candidate_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO candidate_paths (
                 candidate_id, path_ordinal, observed_path, observed_path_encoding
             ) VALUES (?1, 0, ?2, 1)",
            params![candidate_id, target.as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO candidate_evidence (
                 candidate_id, evidence_ordinal, evidence_kind,
                 path_value, path_value_encoding
             ) VALUES (?1, 0, 'matched_path', ?2, 1)",
            params![candidate_id, target.as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_sessions (
                 session_id, plan_id, started_at_unix_ms, completed_at_unix_ms, mode,
                 estimated_bytes, verified_capacity_delta_bytes, trigger_source, status,
                 record_format_version, source_scan_id,
                 plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                 plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                 execution_owner_id, execution_generation, last_heartbeat_at_unix_ms,
                 cancellation_requested
             ) VALUES (
                 ?1, ?2, 1000500, ?3, 'permanent_safe', 8, ?4, 'manual', ?5, 2,
                 'scan:v2-lifecycle', 1000, 0, 1900, 0, ?6, ?7, ?8, 0
             )",
            params![
                session_id,
                plan_id,
                completed_at,
                capacity_delta,
                status,
                execution_owner,
                execution_generation,
                heartbeat,
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_items (
                 session_id, item_ordinal, rule_id, rule_revision, estimated_bytes,
                 final_status, record_format_version, candidate_id, category, safety_tier,
                 proposed_action, rule_schedule_eligible
             ) VALUES (
                 ?1, 0, 'fixture.v2-lifecycle', 1, 8, ?2, 2, ?3,
                 'application_cache', 'safe_regenerable',
                 'remove_known_regenerable_contents', 0
             )",
            params![session_id, item_status, candidate_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_item_paths (
                 session_id, item_ordinal, path_ordinal, target_path,
                 target_path_encoding, attempt_generation, status,
                 effect_started_at_unix_ms, completed_at_unix_ms
             ) VALUES (?1, 0, 0, ?2, 1, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                target.as_slice(),
                attempt_generation,
                item_status,
                effect_started,
                path_completed,
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_item_evidence (
                 session_id, item_ordinal, evidence_ordinal, evidence_kind,
                 path_value, path_value_encoding
             ) VALUES (?1, 0, 0, 'matched_path', ?2, 1)",
            params![session_id, target.as_slice()],
        )
        .unwrap();
    for (ordinal, warning) in [
        "estimated_bytes_unverified",
        "permanent_removal_cannot_be_undone",
    ]
    .into_iter()
    .enumerate()
    {
        connection
            .execute(
                "INSERT INTO cleanup_plan_warnings (
                     session_id, warning_ordinal, warning_kind
                 ) VALUES (?1, ?2, ?3)",
                params![session_id, ordinal as i64, warning],
            )
            .unwrap();
    }
}

#[test]
fn compiled_migration_chain_and_embedded_checksum_are_valid() {
    validate_compiled_migrations().unwrap();
    assert_eq!(test_migrations().len(), DATABASE_SCHEMA_VERSION as usize);
}

#[test]
fn embedded_v1_schema_fingerprint_matches_migration() {
    let connection = fresh_v1_schema();
    assert_eq!(
        schema_fingerprint(&connection).unwrap(),
        test_v1_schema_fingerprint()
    );
}

#[test]
fn embedded_v2_schema_fingerprint_matches_complete_chain() {
    let connection = fresh_v2_schema();
    assert_eq!(
        schema_fingerprint(&connection).unwrap(),
        test_v2_schema_fingerprint()
    );
    assert_eq!(
        inspect_schema(&connection).unwrap(),
        SchemaState::Older { found: 2 }
    );
}

#[test]
fn embedded_v3_schema_fingerprint_matches_complete_chain() {
    let connection = fresh_v3_schema();
    assert_eq!(
        schema_fingerprint(&connection).unwrap(),
        test_v3_schema_fingerprint()
    );
    assert_eq!(
        inspect_schema(&connection).unwrap(),
        SchemaState::Older { found: 3 }
    );
}

#[test]
fn embedded_v4_schema_fingerprint_matches_complete_chain() {
    let connection = fresh_v4_schema();
    assert_eq!(
        schema_fingerprint(&connection).unwrap(),
        test_v4_schema_fingerprint()
    );
    assert_eq!(
        inspect_schema(&connection).unwrap(),
        SchemaState::Older { found: 4 }
    );
}

#[test]
fn embedded_v5_schema_fingerprint_matches_complete_chain() {
    let connection = fresh_current_schema();
    assert_eq!(
        schema_fingerprint(&connection).unwrap(),
        test_v5_schema_fingerprint()
    );
    assert_eq!(inspect_schema(&connection).unwrap(), SchemaState::Current);
}

#[test]
fn populated_v4_upgrade_preserves_snapshot_and_fabricates_no_tombstone() {
    let mut connection = fresh_v4_schema();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection
        .execute(
            "INSERT INTO scans (
                 scan_id, root_path, root_path_encoding, started_at_unix_ms,
                 completed_at_unix_ms, status, snapshot_version,
                 snapshot_relative_path, snapshot_relative_path_encoding,
                 snapshot_checksum_sha256, coverage_status, coverage_permille
             ) VALUES (
                 'scan:v4-snapshot', ?1, 1, 10, 20, 'succeeded', 1,
                 ?2, 1, ?3, 'complete', 1000
             )",
            params![
                b"/v4-snapshot".as_slice(),
                b"snapshot-v4.duxsnapshot".as_slice(),
                [0x5a_u8; 32].as_slice(),
            ],
        )
        .unwrap();

    apply_pending_migrations(&mut connection, 30).unwrap();

    assert_eq!(inspect_schema(&connection).unwrap(), SchemaState::Current);
    let snapshot: (String, i64, Vec<u8>, i64, Vec<u8>) = connection
        .query_row(
            "SELECT status, snapshot_version, snapshot_relative_path,
                    snapshot_relative_path_encoding, snapshot_checksum_sha256
             FROM scans WHERE scan_id = 'scan:v4-snapshot'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        snapshot,
        (
            "succeeded".to_owned(),
            1,
            b"snapshot-v4.duxsnapshot".to_vec(),
            1,
            vec![0x5a; 32],
        )
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM snapshot_retention_tombstones",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn v5_snapshot_tombstones_are_exact_append_only_history() {
    let connection = fresh_current_schema();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection
        .execute(
            "INSERT INTO scans (
                 scan_id, root_path, root_path_encoding, started_at_unix_ms,
                 completed_at_unix_ms, status, snapshot_version,
                 snapshot_relative_path, snapshot_relative_path_encoding,
                 snapshot_checksum_sha256, coverage_status, coverage_permille
             ) VALUES (
                 'scan:tombstone-shape', ?1, 1, 10, 20, 'succeeded', 1,
                 ?2, 1, ?3, 'complete', 1000
             )",
            params![
                b"/tombstone-shape".as_slice(),
                b"snapshot-tombstone.duxsnapshot".as_slice(),
                [0x6b_u8; 32].as_slice(),
            ],
        )
        .unwrap();
    let insert = "INSERT INTO snapshot_retention_tombstones (
            scan_id, record_format_version, scan_status, completed_at_unix_ms,
            snapshot_version, snapshot_relative_path,
            snapshot_relative_path_encoding, snapshot_checksum_sha256,
            committed_at_unix_ms
         ) VALUES (
            'scan:tombstone-shape', 1, 'succeeded', 20, 1, ?1, 1, ?2, ?3
         )";
    assert!(
        connection
            .execute(
                insert,
                params![
                    b"snapshot-tombstone.duxsnapshot".as_slice(),
                    [0x6b_u8; 32].as_slice(),
                    19_i64,
                ],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                insert,
                params![
                    b"snapshot-tombstone.duxsnapshot".as_slice(),
                    [0x7c_u8; 32].as_slice(),
                    21_i64,
                ],
            )
            .is_err()
    );
    connection
        .execute(
            insert,
            params![
                b"snapshot-tombstone.duxsnapshot".as_slice(),
                [0x6b_u8; 32].as_slice(),
                21_i64,
            ],
        )
        .unwrap();

    assert!(
        connection
            .execute(
                "UPDATE snapshot_retention_tombstones
                 SET committed_at_unix_ms = 22
                 WHERE scan_id = 'scan:tombstone-shape'",
                [],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "DELETE FROM snapshot_retention_tombstones
                 WHERE scan_id = 'scan:tombstone-shape'",
                [],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "UPDATE scans SET snapshot_checksum_sha256 = zeroblob(32)
                 WHERE scan_id = 'scan:tombstone-shape'",
                [],
            )
            .is_err()
    );
}

#[test]
fn populated_v3_upgrade_adds_no_fabricated_candidate_evaluations() {
    let mut connection = fresh_v3_schema();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection
        .execute(
            "INSERT INTO scans (
                 scan_id, root_path, root_path_encoding, started_at_unix_ms,
                 completed_at_unix_ms, status, coverage_status, coverage_permille
             ) VALUES ('scan:v3-evaluation', ?1, 1, 10, 20, 'succeeded', 'complete', 1000)",
            [b"/v3-evaluation".as_slice()],
        )
        .unwrap();

    apply_pending_migrations(&mut connection, 30).unwrap();

    assert_eq!(inspect_schema(&connection).unwrap(), SchemaState::Current);
    let count: i64 = connection
        .query_row("SELECT count(*) FROM candidate_evaluations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn v4_candidate_evaluation_constraints_are_terminal_shape_strict() {
    let connection = fresh_current_schema();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection
        .execute(
            "INSERT INTO scans (
                 scan_id, root_path, root_path_encoding, started_at_unix_ms,
                 completed_at_unix_ms, status, snapshot_version,
                 snapshot_relative_path, snapshot_relative_path_encoding,
                 snapshot_checksum_sha256, coverage_status, coverage_permille
             ) VALUES (
                 'scan:evaluation-shape', ?1, 1, 10, 20, 'succeeded', 1,
                 ?2, 1, zeroblob(32), 'complete', 1000
             )",
            params![
                b"/evaluation-shape".as_slice(),
                b"snapshot-evaluation-shape.duxsnapshot".as_slice()
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO candidate_evaluations (
                 scan_id, record_format_version, evaluator_revision,
                 rule_catalog_schema_version, rule_catalog_sha256,
                 context_format_version, context_sha256,
                 snapshot_version, snapshot_sha256, scheduled_at_unix_ms,
                 status
             ) VALUES (
                 'scan:evaluation-shape', 1, 1, 1, zeroblob(32),
                 1, zeroblob(32), 1, zeroblob(32), 20, 'pending'
             )",
            [],
        )
        .unwrap();

    assert!(
        connection
            .execute(
                "UPDATE candidate_evaluations
                 SET status = 'succeeded', completed_at_unix_ms = 21
                 WHERE scan_id = 'scan:evaluation-shape'",
                [],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "UPDATE candidate_evaluations
                 SET status = 'failed', completed_at_unix_ms = 21,
                     failure_kind = 'not_closed'
                 WHERE scan_id = 'scan:evaluation-shape'",
                [],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "DELETE FROM scans WHERE scan_id = 'scan:evaluation-shape'",
                [],
            )
            .is_err()
    );
}

#[test]
fn populated_v1_upgrade_preserves_legacy_candidate_and_cleanup_observations() {
    let mut connection = fresh_v1_schema();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    let v1 = &test_migrations()[0];
    connection
        .execute(
            "INSERT INTO schema_migrations (
                 version, name, checksum_sha256, applied_at_unix_ms
             ) VALUES (1, ?1, ?2, 1)",
            params![v1.name, v1.checksum_sha256.as_slice()],
        )
        .unwrap();
    connection
        .pragma_update(None, "application_id", DUX_APPLICATION_ID)
        .unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    connection
        .execute(
            "INSERT INTO scans (
                 scan_id, root_path, root_path_encoding, started_at_unix_ms, status
             ) VALUES ('scan:legacy', ?1, 1, 10, 'succeeded')",
            [b"/legacy-root".as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO candidates (
                 candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                 estimated_bytes, created_at_unix_ms, status
             ) VALUES (
                 'candidate:legacy', 'scan:legacy', 'fixture.legacy', 7,
                 'review_required', 4096, 11, 'dismissed'
             )",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_sessions (
                 session_id, plan_id, started_at_unix_ms, completed_at_unix_ms, mode,
                 estimated_bytes, verified_capacity_delta_bytes, trigger_source, status
             ) VALUES (
                 'session:legacy', 'plan:legacy', 12, 13, 'dry_run',
                 4096, 0, 'manual', 'dry_run'
             )",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_items (
                 item_id, session_id, item_ordinal, rule_id, rule_revision,
                 target_path, target_path_encoding, estimated_bytes, final_status,
                 error_category
             ) VALUES (
                 41, 'session:legacy', 0, 'fixture.legacy', 7,
                 ?1, 1, 4096, 'dry_run', ?2
             )",
            params![b"/legacy-root/cache".as_slice(), "é".repeat(128)],
        )
        .unwrap();

    apply_pending_migrations(&mut connection, 20).unwrap();

    assert_eq!(inspect_schema(&connection).unwrap(), SchemaState::Current);
    assert_eq!(
        schema_fingerprint(&connection).unwrap(),
        test_v5_schema_fingerprint()
    );
    let candidate: (i64, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT record_format_version, category, proposed_action
             FROM candidates WHERE candidate_id = 'candidate:legacy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(candidate, (1, None, None));
    let session: (i64, Option<String>) = connection
        .query_row(
            "SELECT record_format_version, source_scan_id
             FROM cleanup_sessions WHERE session_id = 'session:legacy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(session, (1, None));
    let item: (i64, Vec<u8>, i64, Option<String>, Option<String>) = connection
        .query_row(
            "SELECT record_format_version, legacy_target_path,
                    legacy_target_path_encoding, candidate_id, error_category
             FROM cleanup_items WHERE item_id = 41",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        item,
        (
            1,
            b"/legacy-root/cache".to_vec(),
            1,
            None,
            Some("é".repeat(128))
        )
    );
    let versions: Vec<i64> = connection
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(versions, [1, 2, 3, 4, 5]);
}

#[test]
fn populated_v2_upgrade_defaults_existing_sessions_and_creates_no_claims() {
    let mut connection = fresh_v2_schema();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_sessions (
                 session_id, plan_id, started_at_unix_ms, completed_at_unix_ms, mode,
                 estimated_bytes, verified_capacity_delta_bytes, trigger_source, status,
                 record_format_version
             ) VALUES (
                 'session:v2-existing', 'plan:v2-existing', 12, 13, 'dry_run',
                 4096, 0, 'manual', 'dry_run', 1
             )",
            [],
        )
        .unwrap();

    apply_pending_migrations(&mut connection, 20).unwrap();

    assert_eq!(inspect_schema(&connection).unwrap(), SchemaState::Current);
    let coupling_version: i64 = connection
        .query_row(
            "SELECT candidate_status_coupling_version
             FROM cleanup_sessions WHERE session_id = 'session:v2-existing'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(coupling_version, 1);
    let claim_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM candidate_plan_claims", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(claim_count, 0);
}

#[test]
fn v2_format2_cleanup_lifecycles_migrate_as_explicitly_uncoupled_history() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let paths = SecureStorePaths::prepare(&path).unwrap();
    let sqlite_path = paths.sqlite_path().unwrap();
    let connection = Connection::open(sqlite_path).unwrap();
    install_v2_schema(&connection);
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection
        .execute(
            "INSERT INTO scans (
                 scan_id, root_path, root_path_encoding, started_at_unix_ms,
                 completed_at_unix_ms, status
             ) VALUES ('scan:v2-lifecycle', ?1, 1, 998000, 999000, 'succeeded')",
            [b"/v2".as_slice()],
        )
        .unwrap();
    let owner = current_process_instance().unwrap();
    for (label, status) in [
        ("planned", "planned"),
        ("running", "running"),
        ("recovering", "recovering"),
        ("terminal", "completed"),
    ] {
        insert_v2_format2_cleanup_session(&connection, label, status, owner.as_str());
    }
    drop(connection);
    drop(paths);

    let store = StoreCoordinator::open(&path).unwrap();
    assert_eq!(
        store.status().unwrap(),
        DatabaseStatus {
            schema_version: DATABASE_SCHEMA_VERSION,
            access: DatabaseAccess::ReadWriteCurrent,
        }
    );
    store.with_connection(|connection| {
        let migrated: Vec<(String, i64)> = connection
            .prepare(
                "SELECT session_id, candidate_status_coupling_version
                 FROM cleanup_sessions
                 WHERE session_id LIKE 'session:v2-%'
                 ORDER BY session_id",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            migrated,
            [
                ("session:v2-planned".into(), 1),
                ("session:v2-recovering".into(), 1),
                ("session:v2-running".into(), 1),
                ("session:v2-terminal".into(), 1),
            ]
        );
        let claim_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM candidate_plan_claims", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(claim_count, 0);
        let projected_statuses: Vec<String> = connection
            .prepare(
                "SELECT status FROM candidates
                 WHERE candidate_id LIKE 'candidate:v2-%'
                 ORDER BY candidate_id",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(projected_statuses, ["discovered"; 4]);
    });

    let planned_id = CleanupSessionId::new("session:v2-planned").unwrap();
    let Some(StoredCleanupSessionRecord::Planned(planned)) =
        store.load_cleanup_session(&planned_id).unwrap()
    else {
        panic!("migrated pristine v2 session did not decode as a plan");
    };
    assert_eq!(
        planned.candidate_status_coupling,
        CandidateStatusCoupling::LegacyUncoupled
    );
    assert_eq!(planned.items.len(), 1);
    assert_eq!(planned.items[0].prior_review_status, None);

    store.with_connection(|connection| {
        let newly_claimable: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM cleanup_sessions
                 WHERE session_id = ?1 AND record_format_version = 2
                   AND candidate_status_coupling_version = 2
                   AND status = 'planned' AND completed_at_unix_ms IS NULL
                   AND verified_capacity_delta_bytes IS NULL
                   AND execution_owner_id IS NULL AND execution_generation IS NULL
                   AND last_heartbeat_at_unix_ms IS NULL AND cancellation_requested = 0",
                [planned_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(newly_claimable, 0);

        for session in [
            "session:v2-running",
            "session:v2-recovering",
            "session:v2-terminal",
        ] {
            let id = CleanupSessionId::new(session).unwrap();
            let frozen = load_frozen_cleanup_session_within_budget(connection, &id, false)
                .unwrap()
                .unwrap();
            assert_eq!(
                frozen.candidate_status_coupling,
                CandidateStatusCoupling::LegacyUncoupled
            );
            assert_eq!(frozen.items.len(), 1);
            assert_eq!(frozen.items[0].prior_review_status, None);
        }
        let mutable_lifecycles: Vec<(String, String, Option<i64>, Option<i64>)> = connection
            .prepare(
                "SELECT session_id, status, execution_generation, completed_at_unix_ms
                 FROM cleanup_sessions
                 WHERE session_id IN (
                     'session:v2-running', 'session:v2-recovering', 'session:v2-terminal'
                 )
                 ORDER BY session_id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            mutable_lifecycles,
            [
                (
                    "session:v2-recovering".into(),
                    "recovering".into(),
                    Some(1),
                    None
                ),
                ("session:v2-running".into(), "running".into(), Some(1), None),
                (
                    "session:v2-terminal".into(),
                    "completed".into(),
                    Some(1),
                    Some(1_000_800),
                ),
            ]
        );
        let post_read_statuses: Vec<String> = connection
            .prepare(
                "SELECT status FROM candidates
                 WHERE candidate_id LIKE 'candidate:v2-%'
                 ORDER BY candidate_id",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(post_read_statuses, ["discovered"; 4]);
        let planned_state: (String, Option<String>, Option<i64>) = connection
            .query_row(
                "SELECT status, execution_owner_id, execution_generation
                 FROM cleanup_sessions WHERE session_id = ?1",
                [planned_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(planned_state, ("planned".into(), None, None));
    });
}

#[test]
fn v3_candidate_plan_claim_constraints_preserve_both_ownership_edges() {
    let connection = fresh_current_schema();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection
        .execute(
            "INSERT INTO scans (
                 scan_id, root_path, root_path_encoding, started_at_unix_ms, status
             ) VALUES ('scan:v3-claims', ?1, 1, 10, 'succeeded')",
            [b"/v3-claims".as_slice()],
        )
        .unwrap();
    for candidate_id in ["candidate:v3-first", "candidate:v3-second"] {
        connection
            .execute(
                "INSERT INTO candidates (
                     candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                     estimated_bytes, created_at_unix_ms, status, record_format_version
                 ) VALUES (?1, 'scan:v3-claims', 'fixture.v3', 1,
                     'review_required', 8, 11, 'discovered', 1)",
                [candidate_id],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO cleanup_sessions (
                 session_id, plan_id, started_at_unix_ms, mode, estimated_bytes,
                 trigger_source, status, record_format_version
             ) VALUES (
                 'session:v3-claims', 'plan:v3-claims', 12, 'dry_run', 8,
                 'manual', 'planned', 1
             )",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO cleanup_items (
                 item_id, session_id, item_ordinal, rule_id, rule_revision,
                 estimated_bytes, final_status, record_format_version,
                 legacy_target_path, legacy_target_path_encoding
             ) VALUES (
                 1, 'session:v3-claims', 0, 'fixture.v3', 1,
                 8, 'planned', 1, ?1, 1
             )",
            [b"/v3-claims/cache".as_slice()],
        )
        .unwrap();

    let default_coupling: i64 = connection
        .query_row(
            "SELECT candidate_status_coupling_version
             FROM cleanup_sessions WHERE session_id = 'session:v3-claims'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(default_coupling, 1);
    connection
        .execute(
            "UPDATE cleanup_sessions SET candidate_status_coupling_version = 2
             WHERE session_id = 'session:v3-claims'",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "UPDATE cleanup_sessions SET candidate_status_coupling_version = 3
                 WHERE session_id = 'session:v3-claims'",
                [],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "INSERT INTO candidate_plan_claims (
                     candidate_id, session_id, item_ordinal, prior_review_status
                 ) VALUES ('candidate:v3-first', 'session:v3-claims', 0, 'dismissed')",
                [],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "INSERT INTO candidate_plan_claims (
                     candidate_id, session_id, item_ordinal, prior_review_status
                 ) VALUES ('candidate:v3-first', 'session:v3-claims', 1, 'discovered')",
                [],
            )
            .is_err()
    );
    connection
        .execute(
            "INSERT INTO candidate_plan_claims (
                 candidate_id, session_id, item_ordinal, prior_review_status
             ) VALUES ('candidate:v3-first', 'session:v3-claims', 0, 'selected')",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO candidate_plan_claims (
                     candidate_id, session_id, item_ordinal, prior_review_status
                 ) VALUES ('candidate:v3-second', 'session:v3-claims', 0, 'discovered')",
                [],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "DELETE FROM candidates WHERE candidate_id = 'candidate:v3-first'",
                [],
            )
            .is_err()
    );

    assert!(
        connection
            .execute(
                "DELETE FROM cleanup_items
                 WHERE session_id = 'session:v3-claims' AND item_ordinal = 0",
                [],
            )
            .is_err()
    );
    let claim_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM candidate_plan_claims", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(claim_count, 1);
    connection
        .execute(
            "DELETE FROM candidate_plan_claims WHERE candidate_id = 'candidate:v3-first'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "DELETE FROM cleanup_items
             WHERE session_id = 'session:v3-claims' AND item_ordinal = 0",
            [],
        )
        .unwrap();
}

#[test]
fn v2_history_constraints_reject_incomplete_or_incompatible_facts() {
    let connection = fresh_current_schema();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    connection
        .execute(
            "INSERT INTO scans (
                 scan_id, root_path, root_path_encoding, started_at_unix_ms, status
             ) VALUES ('scan:v2', ?1, 1, 10, 'succeeded')",
            [b"/v2-root".as_slice()],
        )
        .unwrap();

    assert!(
        connection
            .execute(
                "INSERT INTO candidates (
                     candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                     estimated_bytes, created_at_unix_ms, status, record_format_version
                 ) VALUES (
                     'candidate:incomplete', 'scan:v2', 'fixture.v2', 1,
                     'safe_regenerable', 8, 11, 'discovered', 2
                 )",
                [],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "INSERT INTO candidates (
                     candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                     estimated_bytes, created_at_unix_ms, status, record_format_version,
                     category, proposed_action, rule_schedule_eligible
                 ) VALUES (
                     'candidate:wrong-pair', 'scan:v2', 'fixture.v2', 1,
                     'safe_evictable', 8, 11, 'discovered', 2,
                     'cloud_file', 'remove_known_regenerable_contents', 0
                 )",
                [],
            )
            .is_err()
    );
    connection
        .execute(
            "INSERT INTO candidates (
                 candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                 estimated_bytes, created_at_unix_ms, status, record_format_version,
                 category, proposed_action, rule_schedule_eligible,
                 newest_mtime_unix_seconds, newest_mtime_nanoseconds
             ) VALUES (
                 'candidate:v2', 'scan:v2', 'fixture.v2', 1,
                 'safe_regenerable', 8, 11, 'discovered', 2,
                 'developer_artifact', 'remove_known_regenerable_contents', 1,
                 20, 123456789
             )",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO candidate_evidence (
                     candidate_id, evidence_ordinal, evidence_kind,
                     observed_unix_seconds, duration_seconds, duration_nanoseconds
                 ) VALUES ('candidate:v2', 0, 'minimum_age', 20, 30, 0)",
                [],
            )
            .is_err()
    );
    connection
        .execute(
            "INSERT INTO candidate_evidence (
                 candidate_id, evidence_ordinal, evidence_kind,
                 observed_unix_seconds, observed_nanoseconds,
                 duration_seconds, duration_nanoseconds
             ) VALUES ('candidate:v2', 0, 'minimum_age', 20, 1, 30, 2)",
            [],
        )
        .unwrap();

    assert!(
        connection
            .execute(
                "INSERT INTO cleanup_sessions (
                     session_id, plan_id, started_at_unix_ms, mode, estimated_bytes,
                     trigger_source, status, record_format_version, source_scan_id,
                     plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                     plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                     cancellation_requested
                 ) VALUES (
                     'session:expired', 'plan:expired', 40, 'dry_run', 8,
                     'manual', 'planned', 2, 'scan:v2', 50, 1, 49, 999999999, 0
                 )",
                [],
            )
            .is_err()
    );
    connection
        .execute(
            "INSERT INTO cleanup_sessions (
                 session_id, plan_id, started_at_unix_ms, mode, estimated_bytes,
                 trigger_source, status, record_format_version, source_scan_id,
                 plan_created_at_unix_seconds, plan_created_at_nanoseconds,
                 plan_expires_at_unix_seconds, plan_expires_at_nanoseconds,
                 cancellation_requested
             ) VALUES (
                 'session:v2', 'plan:v2', 40, 'dry_run', 8,
                 'manual', 'planned', 2, 'scan:v2', 50, 1, 60, 2, 0
             )",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO cleanup_items (
                     session_id, item_ordinal, rule_id, rule_revision, estimated_bytes,
                     final_status, record_format_version, candidate_id, category,
                     safety_tier, proposed_action, rule_schedule_eligible
                 ) VALUES (
                     'session:v2', 0, 'fixture.v2', 1, 8, 'planned', 2,
                     'candidate:v2', 'developer_artifact', 'safe_evictable',
                     'remove_known_regenerable_contents', 0
                 )",
                [],
            )
            .is_err()
    );
    connection
        .execute(
            "INSERT INTO cleanup_items (
                 session_id, item_ordinal, rule_id, rule_revision, estimated_bytes,
                 final_status, record_format_version, candidate_id, category,
                 safety_tier, proposed_action, rule_schedule_eligible
             ) VALUES (
                 'session:v2', 0, 'fixture.v2', 1, 8, 'planned', 2,
                 'candidate:v2', 'developer_artifact', 'safe_regenerable',
                 'remove_known_regenerable_contents', 1
             )",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO cleanup_item_paths (
                     session_id, item_ordinal, path_ordinal, target_path,
                     target_path_encoding, status
                 ) VALUES ('session:v2', 0, 0, ?1, 1, 'effect_started')",
                [b"/v2-root/cache".as_slice()],
            )
            .is_err()
    );
    connection
        .execute(
            "INSERT INTO cleanup_item_paths (
                 session_id, item_ordinal, path_ordinal, target_path,
                 target_path_encoding, status
             ) VALUES ('session:v2', 0, 0, ?1, 1, 'planned')",
            [b"/v2-root/cache".as_slice()],
        )
        .unwrap();
}

#[test]
fn oversized_schema_text_is_rejected_before_fingerprinting_it() {
    let connection = Connection::open_in_memory().unwrap();
    let oversized_default = "x".repeat(64 * 1024);
    connection
        .execute_batch(&format!(
            "CREATE TABLE oversized(value TEXT DEFAULT '{oversized_default}') STRICT;"
        ))
        .unwrap();

    let error = schema_fingerprint(&connection).unwrap_err();
    assert_eq!(error.kind, DatabaseOpenErrorKind::CorruptDatabase);
}

#[test]
fn frozen_byte_codec_is_lossless_and_uses_stable_tags() {
    assert_eq!(StoredEncoding::Utf8LogicalKey as i64, 0);
    assert_eq!(StoredEncoding::Utf8HostPath as i64, 1);
    assert_eq!(StoredEncoding::Utf16LeHostPath as i64, 2);

    let key = encode_logical_key("category.développeur").unwrap();
    assert_eq!(decode_logical_key(&key).unwrap(), "category.développeur");
    assert_eq!(encode_logical_key(""), Err(CodecError::Empty));
    assert_eq!(
        decode_logical_key(&EncodedBytes {
            encoding: StoredEncoding::Utf8LogicalKey,
            bytes: vec![0xff],
        }),
        Err(CodecError::InvalidEncoding)
    );

    let path = if cfg!(windows) {
        PathBuf::from(r"C:\Données\💾")
    } else {
        PathBuf::from("/Données/💾")
    };
    let encoded = encode_host_path(&path).unwrap();
    #[cfg(windows)]
    assert_eq!(encoded.encoding, StoredEncoding::Utf16LeHostPath);
    #[cfg(not(windows))]
    assert_eq!(encoded.encoding, StoredEncoding::Utf8HostPath);
    assert_eq!(decode_host_path(&encoded).unwrap(), path);
}

#[test]
fn schema_path_encoding_bounds_match_the_frozen_codec() {
    let connection = fresh_v1_schema();
    let insert_volume = |id: &str, bytes: &[u8], encoding: i64| {
        connection.execute(
            "INSERT INTO volumes (
                volume_id, mount_path, mount_path_encoding, display_name, filesystem,
                is_internal, is_removable, first_seen_unix_ms, last_seen_unix_ms
             ) VALUES (?1, ?2, ?3, 'Disk', 'test', 1, 0, 1, 1)",
            params![id, bytes, encoding],
        )
    };

    assert!(insert_volume("utf8-max", &vec![b'a'; 32 * 1024], 1).is_ok());
    assert!(insert_volume("utf8-too-long", &vec![b'a'; 32 * 1024 + 1], 1).is_err());
    assert!(insert_volume("utf16-max", &vec![0_u8; 64 * 1024], 2).is_ok());
    assert!(insert_volume("utf16-odd", &[0, 0, 0], 2).is_err());
    assert!(insert_volume("logical-tag", b"/", 0).is_err());
    assert!(insert_volume("unknown-tag", b"/", 3).is_err());
}

#[test]
fn valid_integrity_inspection_stops_at_the_vm_operation_budget() {
    let connection = fresh_current_schema();
    connection
        .execute_batch(
            "WITH RECURSIVE sequence(value) AS (
                     VALUES(1) UNION ALL SELECT value + 1 FROM sequence WHERE value < 20000
                 )
                 INSERT INTO settings (
                     setting_key, value_json, value_schema_version, updated_at_unix_ms
                 ) SELECT printf('budget-%d', value), '{}', 1, value FROM sequence;",
        )
        .unwrap();
    let error = inspect_schema_with_test_budget(&connection, 1, Duration::from_secs(60))
        .expect_err("the first progress callback must exhaust the VM budget");
    assert_eq!(error.kind, DatabaseOpenErrorKind::InspectionLimitExceeded);
    assert_ne!(error.kind, DatabaseOpenErrorKind::CorruptDatabase);

    // The scoped handler must be removed even when it interrupts an operation.
    let sentinel: i64 = connection
        .query_row("SELECT 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(sentinel, 1);
}

#[test]
fn valid_integrity_inspection_stops_at_the_wall_clock_budget() {
    let connection = fresh_current_schema();
    connection
        .execute_batch(
            "WITH RECURSIVE sequence(value) AS (
                 VALUES(1) UNION ALL SELECT value + 1 FROM sequence WHERE value < 20000
             )
             INSERT INTO settings (
                 setting_key, value_json, value_schema_version, updated_at_unix_ms
             ) SELECT printf('deadline-%d', value), '{}', 1, value FROM sequence;",
        )
        .unwrap();
    let error = inspect_schema_with_test_budget(&connection, u64::MAX, Duration::ZERO)
        .expect_err("the first progress callback must observe the expired deadline");
    assert_eq!(error.kind, DatabaseOpenErrorKind::InspectionLimitExceeded);
    assert!(!error.to_string().contains("sqlite"));
    let sentinel: i64 = connection
        .query_row("SELECT 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(sentinel, 1);
}

#[test]
fn inspection_budget_handler_is_removed_during_unwinding() {
    let connection = Connection::open_in_memory().unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        panic_with_test_budget(&connection);
    }));
    assert!(panic.is_err());

    connection
        .execute_batch("CREATE TABLE after_panic(value INTEGER) STRICT;")
        .unwrap();
}

#[test]
fn disk_sample_kind_separates_raw_and_daily_retention_records() {
    let connection = fresh_v1_schema();
    connection
        .execute(
            "INSERT INTO volumes (
                volume_id, mount_path, mount_path_encoding, display_name, filesystem,
                is_internal, is_removable, first_seen_unix_ms, last_seen_unix_ms
             ) VALUES ('volume', ?1, 1, 'Disk', 'apfs', 1, 0, 1, 1)",
            [b"/".as_slice()],
        )
        .unwrap();
    let insert_sample = |kind: &str| {
        connection.execute(
            "INSERT INTO disk_samples (
                volume_id, sample_kind, sampled_at_unix_ms, total_bytes,
                available_bytes, important_available_bytes, pressure
             ) VALUES ('volume', ?1, 86400000, 1000, 400, 300, 'healthy')",
            [kind],
        )
    };

    insert_sample("raw").unwrap();
    insert_sample("daily_rollup").unwrap();
    assert!(insert_sample("raw").is_err());
    assert!(insert_sample("hourly_rollup").is_err());

    let retained_kinds: Vec<String> = connection
        .prepare("SELECT sample_kind FROM disk_samples ORDER BY sample_kind")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(retained_kinds, ["daily_rollup", "raw"]);
}

#[test]
fn scan_coverage_keeps_unknown_distinct_from_measured_zero() {
    let connection = fresh_v1_schema();
    let insert_scan = |id: &str, coverage_status: &str, coverage_permille: Option<i64>| {
        connection.execute(
            "INSERT INTO scans (
                scan_id, root_path, root_path_encoding, started_at_unix_ms, status,
                coverage_status, coverage_permille
             ) VALUES (?1, ?2, 1, 1, 'succeeded', ?3, ?4)",
            params![id, b"/".as_slice(), coverage_status, coverage_permille],
        )
    };

    connection
        .execute(
            "INSERT INTO scans (
                scan_id, root_path, root_path_encoding, started_at_unix_ms, status
             ) VALUES ('unknown-default', ?1, 1, 1, 'succeeded')",
            [b"/".as_slice()],
        )
        .unwrap();
    insert_scan("complete", "complete", Some(1000)).unwrap();
    insert_scan("limited-unmeasured", "limited_access", None).unwrap();
    insert_scan("partial-zero", "partial", Some(0)).unwrap();

    assert!(insert_scan("unknown-zero", "unknown", Some(0)).is_err());
    assert!(insert_scan("complete-short", "complete", Some(999)).is_err());
    assert!(insert_scan("partial-full", "partial", Some(1000)).is_err());
    assert!(insert_scan("invalid-status", "unbounded", None).is_err());

    let coverage: Vec<(String, String, Option<i64>)> = connection
        .prepare(
            "SELECT scan_id, coverage_status, coverage_permille
             FROM scans ORDER BY scan_id",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        coverage,
        [
            ("complete".into(), "complete".into(), Some(1000)),
            ("limited-unmeasured".into(), "limited_access".into(), None),
            ("partial-zero".into(), "partial".into(), Some(0)),
            ("unknown-default".into(), "unknown".into(), None),
        ]
    );
}

#[test]
fn fresh_store_installs_exact_current_schema_and_hardened_pragmas() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&database_path(&temp)).unwrap();

    assert_eq!(
        store.status().unwrap(),
        DatabaseStatus {
            schema_version: DATABASE_SCHEMA_VERSION,
            access: DatabaseAccess::ReadWriteCurrent,
        }
    );
    store.with_connection(|connection| {
        assert_eq!(inspect_schema(connection).unwrap(), SchemaState::Current);
        let journal: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        let foreign_keys: i64 = connection
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        let synchronous: i64 = connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .unwrap();
        let query_only: i64 = connection
            .pragma_query_value(None, "query_only", |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_ascii_lowercase(), "wal");
        assert_eq!(foreign_keys, 1);
        assert_eq!(synchronous, 2);
        assert_eq!(query_only, 0);
    });
}

#[test]
fn published_empty_store_is_recovered_and_migrated() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let paths = SecureStorePaths::prepare(&path).unwrap();
    assert!(paths.requires_initialization());
    drop(paths);

    let store = StoreCoordinator::open(&path).unwrap();
    assert_eq!(
        store.status().unwrap().schema_version,
        DATABASE_SCHEMA_VERSION
    );

    let mut lock_name = path.file_name().unwrap().to_os_string();
    lock_name.push(".writer.lock");
    assert!(
        std::fs::metadata(path.with_file_name(lock_name))
            .unwrap()
            .len()
            > 0
    );
    assert_eq!(
        std::fs::read(initialization_path(&path)).unwrap(),
        b"DUXINITDONE1\0\0\0\0"
    );
}

#[test]
fn current_database_without_initialization_evidence_is_verified_then_backfilled() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let paths = SecureStorePaths::prepare(&path).unwrap();
    let sqlite_path = paths.sqlite_path().unwrap();
    let mut connection = Connection::open(sqlite_path).unwrap();
    apply_pending_migrations(&mut connection, 1).unwrap();
    let mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    drop(connection);
    drop(paths);
    assert!(!initialization_path(&path).exists());

    let store = StoreCoordinator::open(&path).unwrap();
    assert_eq!(
        store.status().unwrap().access,
        DatabaseAccess::ReadWriteCurrent
    );
    assert_eq!(
        std::fs::read(initialization_path(&path)).unwrap(),
        b"DUXINITDONE1\0\0\0\0"
    );
}

#[test]
fn newer_database_without_initialization_evidence_remains_read_only_and_untouched() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let paths = SecureStorePaths::prepare(&path).unwrap();
    let sqlite_path = paths.sqlite_path().unwrap();
    let mut connection = Connection::open(sqlite_path).unwrap();
    apply_pending_migrations(&mut connection, 1).unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations \
             (version, name, checksum_sha256, applied_at_unix_ms) \
             VALUES (?1, 'future-without-sentinel', zeroblob(32), 2)",
            [i64::from(FUTURE_SCHEMA_VERSION)],
        )
        .unwrap();
    connection
        .pragma_update(None, "user_version", FUTURE_SCHEMA_VERSION)
        .unwrap();
    drop(connection);
    drop(paths);

    let store = StoreCoordinator::open(&path).unwrap();
    assert_eq!(
        store.status().unwrap().access,
        DatabaseAccess::ReadOnlyNewer {
            found: FUTURE_SCHEMA_VERSION,
            supported: DATABASE_SCHEMA_VERSION,
        }
    );
    assert!(!initialization_path(&path).exists());
}

#[test]
fn malformed_initialization_evidence_is_rejected_untouched() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    drop(StoreCoordinator::open(&path).unwrap());
    let sentinel = initialization_path(&path);
    let malformed = b"NOT-DUX-INIT!!!!";
    assert_eq!(malformed.len(), 16);
    std::fs::write(&sentinel, malformed).unwrap();

    let error = StoreCoordinator::open(&path).err().unwrap();
    assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
    assert_eq!(std::fs::read(sentinel).unwrap(), malformed);
}

#[test]
fn coordinator_reopens_with_reserved_application_support_siblings_only() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    drop(StoreCoordinator::open(&path).unwrap());

    for name in ["snapshots", "ai", "logs"] {
        std::fs::create_dir(path.parent().unwrap().join(name)).unwrap();
    }
    let reopened = StoreCoordinator::open(&path).unwrap();
    assert_eq!(
        reopened.status().unwrap().access,
        DatabaseAccess::ReadWriteCurrent
    );
    drop(reopened);

    let foreign = path.parent().unwrap().join("foreign-data");
    std::fs::write(&foreign, b"untouched").unwrap();
    let error = StoreCoordinator::open(&path).err().unwrap();
    assert_eq!(error.kind, DatabaseOpenErrorKind::UnrecognizedDatabase);
    assert_eq!(std::fs::read(foreign).unwrap(), b"untouched");
}

#[test]
fn repeated_open_reuses_one_process_coordinator_and_preserves_data() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let first = StoreCoordinator::open(&path).unwrap();
    first.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO settings \
                 (setting_key, value_json, value_schema_version, updated_at_unix_ms) \
                 VALUES ('sentinel', '{}', 1, 1)",
                [],
            )
            .unwrap();
    });
    let second = StoreCoordinator::open(&path).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    drop(first);
    drop(second);

    let reopened = StoreCoordinator::open(&path).unwrap();
    reopened.with_connection(|connection| {
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM settings WHERE setting_key = 'sentinel'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    });
}

#[test]
fn live_coordinator_refreshes_to_read_only_after_external_upgrade() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let first = StoreCoordinator::open(&path).unwrap();

    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations \
             (version, name, checksum_sha256, applied_at_unix_ms) \
             VALUES (?1, 'future-schema', zeroblob(32), 2)",
            [i64::from(FUTURE_SCHEMA_VERSION)],
        )
        .unwrap();
    connection
        .pragma_update(None, "user_version", FUTURE_SCHEMA_VERSION)
        .unwrap();
    drop(connection);

    assert_eq!(
        first.status().unwrap(),
        DatabaseStatus {
            schema_version: FUTURE_SCHEMA_VERSION,
            access: DatabaseAccess::ReadOnlyNewer {
                found: FUTURE_SCHEMA_VERSION,
                supported: DATABASE_SCHEMA_VERSION,
            },
        }
    );
    first.with_connection(|connection| {
        let query_only: i64 = connection
            .pragma_query_value(None, "query_only", |row| row.get(0))
            .unwrap();
        assert_eq!(query_only, 1);
        assert!(
            connection
                .execute(
                    "INSERT INTO settings \
                     (setting_key, value_json, value_schema_version, updated_at_unix_ms) \
                     VALUES ('forbidden-live-upgrade', '{}', 1, 2)",
                    [],
                )
                .is_err()
        );
    });

    let second = StoreCoordinator::open(&path).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert!(matches!(
        second.status().unwrap().access,
        DatabaseAccess::ReadOnlyNewer {
            found: FUTURE_SCHEMA_VERSION,
            ..
        }
    ));
}

#[test]
fn live_status_reports_schema_drift_instead_of_cached_compatibility() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let store = StoreCoordinator::open(&path).unwrap();
    store.with_connection(|connection| {
        connection
            .execute("CREATE TABLE unexpected(value INTEGER) STRICT", [])
            .unwrap();
    });

    assert_eq!(
        store.status().unwrap_err().kind,
        DatabaseOpenErrorKind::CorruptDatabase
    );
}

#[test]
fn live_status_keeps_the_active_connection_for_an_ordinary_wal() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let store = StoreCoordinator::open(&path).unwrap();
    store.with_connection(|connection| {
        connection
            .execute_batch(
                "CREATE TEMP TABLE connection_sentinel(value TEXT) STRICT;
                 INSERT INTO connection_sentinel VALUES ('same-connection');
                 INSERT INTO settings (
                     setting_key, value_json, value_schema_version, updated_at_unix_ms
                 ) VALUES ('create-ordinary-wal', '{}', 1, 1);",
            )
            .unwrap();
    });
    assert!(path.with_file_name("dux.sqlite3-wal").exists());

    assert_eq!(
        store.status().unwrap().access,
        DatabaseAccess::ReadWriteCurrent
    );
    store.with_connection(|connection| {
        let sentinel: String = connection
            .query_row("SELECT value FROM connection_sentinel", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(sentinel, "same-connection");
    });
}

#[test]
fn live_status_rechecks_application_id_and_migration_ledger() {
    for (mutation, expected) in [
        (
            "PRAGMA application_id = 1",
            DatabaseOpenErrorKind::UnrecognizedDatabase,
        ),
        (
            "UPDATE schema_migrations SET checksum_sha256 = zeroblob(32) WHERE version = 1",
            DatabaseOpenErrorKind::CorruptDatabase,
        ),
    ] {
        let temp = TempDir::new().unwrap();
        let path = database_path(&temp);
        let store = StoreCoordinator::open(&path).unwrap();
        store.with_connection(|connection| connection.execute_batch(mutation).unwrap());

        let error = store.status().unwrap_err();
        assert_eq!(error.kind, expected);
        assert!(!error.to_string().contains(path.to_str().unwrap()));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn case_aliases_share_the_physical_store_coordinator() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let first = StoreCoordinator::open(&path).unwrap();
    let alias = path.with_file_name("DUX.SQLITE3");

    // APFS may be formatted case-sensitive. The alias invariant is relevant
    // only when both spellings resolve to the same physical file.
    if !alias.exists() {
        return;
    }

    let second = StoreCoordinator::open(&alias).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn newer_valid_dux_schema_opens_strictly_read_only() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    drop(StoreCoordinator::open(&path).unwrap());
    let connection = Connection::open_with_flags(
        path.canonicalize().unwrap(),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations \
             (version, name, checksum_sha256, applied_at_unix_ms) \
             VALUES (?1, 'future-schema', zeroblob(32), 2)",
            [i64::from(FUTURE_SCHEMA_VERSION)],
        )
        .unwrap();
    connection
        .pragma_update(None, "user_version", FUTURE_SCHEMA_VERSION)
        .unwrap();
    drop(connection);

    let store = StoreCoordinator::open(&path).unwrap();
    assert_eq!(
        store.status().unwrap(),
        DatabaseStatus {
            schema_version: FUTURE_SCHEMA_VERSION,
            access: DatabaseAccess::ReadOnlyNewer {
                found: FUTURE_SCHEMA_VERSION,
                supported: DATABASE_SCHEMA_VERSION,
            },
        }
    );
    store.with_connection(|connection| {
        assert!(
            connection
                .execute(
                    "INSERT INTO settings \
                     (setting_key, value_json, value_schema_version, updated_at_unix_ms) \
                     VALUES ('forbidden', '{}', 1, 2)",
                    [],
                )
                .is_err()
        );
    });
}

#[cfg(unix)]
#[test]
fn hot_rollback_journal_is_recovered_after_abrupt_process_death() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let store = StoreCoordinator::open(&path).unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO settings \
                 (setting_key, value_json, value_schema_version, updated_at_unix_ms) \
                 VALUES ('crash-sentinel', '\"committed\"', 1, 1)",
                [],
            )
            .unwrap();
    });
    drop(store);
    switch_to_delete_journal(&path);

    let ready = temp.path().join("hot-journal-ready");
    let never_released = temp.path().join("hot-journal-release");
    let mut child = spawn_persistence_helper(
        "hot-rollback-journal",
        &[
            ("DUX_PERSISTENCE_DATABASE", &path),
            ("DUX_PERSISTENCE_READY", &ready),
            ("DUX_PERSISTENCE_RELEASE", &never_released),
        ],
    );
    wait_for_child_handshake(&mut child, &ready);

    let journal = path.with_file_name("dux.sqlite3-journal");
    let journal_bytes = std::fs::read(&journal).unwrap();
    assert!(journal_bytes.len() > 512);
    assert!(journal_bytes[..8].iter().any(|byte| *byte != 0));
    child.terminate_without_unwinding();
    let hot_journal_bytes = std::fs::read(&journal).unwrap();
    assert!(hot_journal_bytes.len() > 512);
    assert!(hot_journal_bytes[..8].iter().any(|byte| *byte != 0));

    let recovered = StoreCoordinator::open(&path).unwrap();
    assert_eq!(
        recovered.status().unwrap().access,
        DatabaseAccess::ReadWriteCurrent
    );
    recovered.with_connection(|connection| {
        let value: String = connection
            .query_row(
                "SELECT value_json FROM settings WHERE setting_key = 'crash-sentinel'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "\"committed\"");
        assert_eq!(inspect_schema(connection).unwrap(), SchemaState::Current);
    });
}

#[cfg(unix)]
#[test]
#[allow(clippy::disallowed_methods)]
fn wal_is_recovered_after_crash_even_when_shared_memory_is_missing() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let store = StoreCoordinator::open(&path).unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO settings \
                 (setting_key, value_json, value_schema_version, updated_at_unix_ms) \
                 VALUES ('crash-sentinel', '\"before-wal\"', 1, 1)",
                [],
            )
            .unwrap();
    });
    drop(store);

    let ready = temp.path().join("hot-wal-ready");
    let never_released = temp.path().join("hot-wal-release");
    let mut child = spawn_persistence_helper(
        "hot-wal",
        &[
            ("DUX_PERSISTENCE_DATABASE", &path),
            ("DUX_PERSISTENCE_READY", &ready),
            ("DUX_PERSISTENCE_RELEASE", &never_released),
        ],
    );
    wait_for_child_handshake(&mut child, &ready);
    child.terminate_without_unwinding();

    let wal = path.with_file_name("dux.sqlite3-wal");
    assert!(std::fs::metadata(&wal).unwrap().len() > 32);
    let shared_memory = path.with_file_name("dux.sqlite3-shm");
    if shared_memory.exists() {
        let displaced = temp.path().join("displaced-test-shm");
        // DUX-DESTRUCTIVE: allow=test-persistence-displace-shm -- move only the crash fixture's TempDir-owned SQLite shm to prove WAL recovery recreates missing shared memory
        std::fs::rename(&shared_memory, &displaced).unwrap();
    }

    let recovered = StoreCoordinator::open(&path).unwrap();
    assert_eq!(
        recovered.status().unwrap().access,
        DatabaseAccess::ReadWriteCurrent
    );
    recovered.with_connection(|connection| {
        let value: String = connection
            .query_row(
                "SELECT value_json FROM settings WHERE setting_key = 'crash-sentinel'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "\"wal-committed\"");
        assert_eq!(inspect_schema(connection).unwrap(), SchemaState::Current);
    });
}

#[test]
fn cross_process_writer_waiter_rechecks_version_before_writing() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    drop(StoreCoordinator::open(&path).unwrap());
    switch_to_delete_journal(&path);

    let probe_ready = temp.path().join("waiter-probed");
    let probe_continue = temp.path().join("waiter-continue");
    let lock_attempt = temp.path().join("waiter-acquiring");
    let mut waiter = spawn_persistence_helper(
        "open-after-writer-lock-race",
        &[
            ("DUX_PERSISTENCE_DATABASE", &path),
            ("DUX_PERSISTENCE_READY", &probe_ready),
            ("DUX_PERSISTENCE_RELEASE", &probe_continue),
            ("DUX_PERSISTENCE_ACQUIRING", &lock_attempt),
        ],
    );
    wait_for_child_handshake(&mut waiter, &probe_ready);

    let upgrade_ready = temp.path().join("upgrader-ready");
    let upgrade_release = temp.path().join("upgrader-release");
    let mut upgrader = spawn_persistence_helper(
        "upgrade-under-writer-lock",
        &[
            ("DUX_PERSISTENCE_DATABASE", &path),
            ("DUX_PERSISTENCE_READY", &upgrade_ready),
            ("DUX_PERSISTENCE_RELEASE", &upgrade_release),
        ],
    );
    wait_for_child_handshake(&mut upgrader, &upgrade_ready);

    publish_handshake(&probe_continue);
    wait_for_child_handshake(&mut waiter, &lock_attempt);
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        waiter.try_status().is_none(),
        "writer waiter bypassed the held cross-process lease"
    );

    publish_handshake(&upgrade_release);
    upgrader.wait_for_success();
    waiter.wait_for_success();
}

#[test]
fn cross_process_cleanup_lock_is_exclusive_and_released() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    drop(StoreCoordinator::open(&path).unwrap());

    let ready = temp.path().join("cleanup-holder-ready");
    let release = temp.path().join("cleanup-holder-release");
    let mut holder = spawn_persistence_helper(
        "hold-cleanup-lock",
        &[
            ("DUX_PERSISTENCE_DATABASE", &path),
            ("DUX_PERSISTENCE_READY", &ready),
            ("DUX_PERSISTENCE_RELEASE", &release),
        ],
    );
    wait_for_child_handshake(&mut holder, &ready);

    let paths = SecureStorePaths::prepare(&path).unwrap();
    assert_eq!(
        paths
            .acquire_cleanup_lock(Duration::from_millis(20))
            .err()
            .unwrap()
            .kind,
        DatabaseOpenErrorKind::Busy
    );
    paths
        .acquire_writer_lock(Duration::from_millis(20))
        .unwrap();

    publish_handshake(&release);
    holder.wait_for_success();
    let guard = paths.acquire_cleanup_lock(SUBPROCESS_TIMEOUT).unwrap();
    paths.validate_cleanup_lock_guard(&guard).unwrap();
}

#[cfg(unix)]
#[test]
fn cleanup_lock_is_released_after_abrupt_process_death() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    drop(StoreCoordinator::open(&path).unwrap());

    let ready = temp.path().join("cleanup-crash-holder-ready");
    let never_released = temp.path().join("cleanup-crash-holder-release");
    let mut holder = spawn_persistence_helper(
        "hold-cleanup-lock",
        &[
            ("DUX_PERSISTENCE_DATABASE", &path),
            ("DUX_PERSISTENCE_READY", &ready),
            ("DUX_PERSISTENCE_RELEASE", &never_released),
        ],
    );
    wait_for_child_handshake(&mut holder, &ready);
    holder.terminate_without_unwinding();

    let paths = SecureStorePaths::prepare(&path).unwrap();
    let guard = paths.acquire_cleanup_lock(SUBPROCESS_TIMEOUT).unwrap();
    paths.validate_cleanup_lock_guard(&guard).unwrap();
}

#[cfg(unix)]
#[test]
fn process_instance_liveness_tracks_graceful_and_abrupt_death() {
    for abrupt in [false, true] {
        let temp = TempDir::new().unwrap();
        let identity_path = temp.path().join("process-owner-identity");
        let ready = temp.path().join("process-owner-ready");
        let release = temp.path().join("process-owner-release");
        let mut child = spawn_persistence_helper(
            "hold-process-instance",
            &[
                ("DUX_PERSISTENCE_IDENTITY", &identity_path),
                ("DUX_PERSISTENCE_READY", &ready),
                ("DUX_PERSISTENCE_RELEASE", &release),
            ],
        );
        wait_for_child_handshake(&mut child, &ready);
        let encoded = std::fs::read_to_string(&identity_path).unwrap();
        let identity = ProcessInstanceId::from_stored(&encoded).unwrap();
        assert_eq!(probe_process_instance(&identity), ProcessLiveness::Alive);

        if abrupt {
            child.terminate_without_unwinding();
        } else {
            publish_handshake(&release);
            child.wait_for_success();
        }
        assert_eq!(
            probe_process_instance(&identity),
            ProcessLiveness::DefinitelyGone
        );
    }
}

#[test]
fn upgrade_between_probe_and_writer_lock_never_opens_newer_schema_for_write() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    drop(StoreCoordinator::open(&path).unwrap());
    let sqlite_path = path.canonicalize().unwrap();
    let connection = Connection::open(&sqlite_path).unwrap();
    let changed: String = connection
        .query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(changed.to_ascii_lowercase(), "delete");
    drop(connection);

    let paths = SecureStorePaths::prepare(&path).unwrap();
    let upgrade_path = sqlite_path.clone();
    let store = StoreCoordinator::open_unregistered_for_test(paths, &sqlite_path, move || {
        let connection = Connection::open(&upgrade_path).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations \
                 (version, name, checksum_sha256, applied_at_unix_ms) \
                 VALUES (?1, 'racing-future-schema', zeroblob(32), 2)",
                [i64::from(FUTURE_SCHEMA_VERSION)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", FUTURE_SCHEMA_VERSION)
            .unwrap();
        Ok(())
    })
    .unwrap();

    assert_eq!(
        store.status().unwrap(),
        DatabaseStatus {
            schema_version: FUTURE_SCHEMA_VERSION,
            access: DatabaseAccess::ReadOnlyNewer {
                found: FUTURE_SCHEMA_VERSION,
                supported: DATABASE_SCHEMA_VERSION,
            },
        }
    );
    store.with_connection(|connection| {
        let mode: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_ascii_lowercase(), "delete");
        assert!(
            connection
                .execute(
                    "INSERT INTO settings \
                     (setting_key, value_json, value_schema_version, updated_at_unix_ms) \
                     VALUES ('forbidden-race', '{}', 1, 2)",
                    [],
                )
                .is_err()
        );
    });
}

#[test]
fn schema_or_ledger_drift_fails_closed_without_path_disclosure() {
    for mutation in [
        "CREATE TABLE unexpected(value INTEGER) STRICT",
        "CREATE TABLE sqliteXunexpected(value INTEGER) STRICT",
        "UPDATE schema_migrations SET checksum_sha256 = zeroblob(32) WHERE version = 1",
        "DELETE FROM schema_migrations WHERE version = 1",
    ] {
        let temp = TempDir::new().unwrap();
        let path = database_path(&temp);
        drop(StoreCoordinator::open(&path).unwrap());
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(mutation).unwrap();
        drop(connection);

        let error = StoreCoordinator::open(&path).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::CorruptDatabase);
        assert!(!error.to_string().contains(path.to_str().unwrap()));
    }
}

#[test]
fn foreign_database_is_rejected_without_becoming_a_dux_store() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    std::fs::create_dir(path.parent().unwrap()).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE foreign_data(value TEXT) STRICT;")
        .unwrap();
    drop(connection);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(
            path.parent().unwrap(),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    let mut lock_name = path.file_name().unwrap().to_os_string();
    lock_name.push(".writer.lock");
    let lock_path = path.with_file_name(lock_name);

    let error = StoreCoordinator::open(&path).err().unwrap();
    assert_eq!(error.kind, DatabaseOpenErrorKind::UnrecognizedDatabase);
    assert!(!lock_path.exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o644
        );
    }

    let connection = Connection::open(&path).unwrap();
    let dux_tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(dux_tables, 0);
}

#[test]
#[allow(clippy::disallowed_methods)]
fn marker_owned_database_truncated_after_its_valid_header_is_corrupt() {
    for truncated_length in [0, 100] {
        let temp = TempDir::new().unwrap();
        let path = database_path(&temp);
        drop(StoreCoordinator::open(&path).unwrap());

        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        // DUX-DESTRUCTIVE: allow=test-persistence-truncate-owned-database -- truncate only this TempDir-owned DUX database fixture after closing every SQLite connection
        file.set_len(truncated_length).unwrap();
        drop(file);

        let error = StoreCoordinator::open(&path).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::CorruptDatabase);
        assert!(!error.to_string().contains(path.to_str().unwrap()));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), truncated_length);
    }
}

#[test]
fn failed_migration_rolls_back_schema_and_ledger_together() {
    const LEDGER_AND_FIRST: &str = "
        CREATE TABLE schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            checksum_sha256 BLOB NOT NULL,
            applied_at_unix_ms INTEGER NOT NULL
        ) STRICT;
        CREATE TABLE first(value INTEGER) STRICT;
    ";
    let migrations = [
        Migration {
            version: 1,
            name: "first",
            checksum_sha256: [1; 32],
            sql: LEDGER_AND_FIRST,
        },
        Migration {
            version: 2,
            name: "broken",
            checksum_sha256: [2; 32],
            sql: "CREATE TABLE broken(",
        },
    ];
    let mut connection = Connection::open_in_memory().unwrap();
    let transaction = connection.transaction().unwrap();
    assert!(apply_test_chain(&transaction, &migrations, 7).is_err());
    drop(transaction);

    let objects: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(objects, 0);
}

#[test]
fn embedded_transaction_control_cannot_escape_the_migration_transaction() {
    const ATTEMPTED_ESCAPE: &str = "
        CREATE TABLE schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            checksum_sha256 BLOB NOT NULL,
            applied_at_unix_ms INTEGER NOT NULL
        ) STRICT;
        CREATE TABLE before_escape(value INTEGER) STRICT;
        COMMIT;
        CREATE TABLE after_escape(value INTEGER) STRICT;
    ";
    let migrations = [Migration {
        version: 1,
        name: "transaction-escape",
        checksum_sha256: [3; 32],
        sql: ATTEMPTED_ESCAPE,
    }];
    let mut connection = Connection::open_in_memory().unwrap();
    let transaction = connection.transaction().unwrap();
    let error = apply_test_chain(&transaction, &migrations, 7).unwrap_err();
    assert_eq!(error.kind, DatabaseOpenErrorKind::MigrationFailed);
    drop(transaction);

    let objects: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(objects, 0);
}

#[test]
fn successful_upgrade_preserves_existing_data_and_advances_ledger_atomically() {
    const V1: &str = "
        CREATE TABLE schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            checksum_sha256 BLOB NOT NULL,
            applied_at_unix_ms INTEGER NOT NULL
        ) STRICT;
        CREATE TABLE durable_rows (
            row_id INTEGER PRIMARY KEY,
            value TEXT NOT NULL
        ) STRICT;
    ";
    const V2: &str = "
        ALTER TABLE durable_rows
            ADD COLUMN classification TEXT NOT NULL DEFAULT 'preserved';
        CREATE INDEX durable_rows_by_classification
            ON durable_rows(classification, row_id);
    ";
    let migrations = [
        Migration {
            version: 1,
            name: "synthetic-v1",
            checksum_sha256: [1; 32],
            sql: V1,
        },
        Migration {
            version: 2,
            name: "synthetic-v2",
            checksum_sha256: [2; 32],
            sql: V2,
        },
    ];
    let mut connection = Connection::open_in_memory().unwrap();
    let transaction = connection.transaction().unwrap();
    apply_test_chain(&transaction, &migrations[..1], 10).unwrap();
    transaction.commit().unwrap();
    connection
        .execute(
            "INSERT INTO durable_rows (row_id, value) VALUES (7, 'keep-me')",
            [],
        )
        .unwrap();

    let transaction = connection.transaction().unwrap();
    apply_test_upgrade_chain(&transaction, &migrations, 1, 20).unwrap();
    transaction.commit().unwrap();

    let row: (String, String) = connection
        .query_row(
            "SELECT value, classification FROM durable_rows WHERE row_id = 7",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("keep-me".into(), "preserved".into()));
    let versions: Vec<i64> = connection
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(versions, [1, 2]);
    let user_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, 2);
}
