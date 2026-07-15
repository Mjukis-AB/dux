use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, params};
use tempfile::TempDir;

use super::codec::{
    CodecError, EncodedBytes, StoredEncoding, decode_host_path, decode_logical_key,
    encode_host_path, encode_logical_key,
};
use super::migrations::{
    DUX_APPLICATION_ID, Migration, SchemaState, apply_pending_migrations, apply_test_chain,
    apply_test_upgrade_chain, inspect_schema, inspect_schema_with_test_budget,
    panic_with_test_budget, schema_fingerprint, test_migrations, test_v1_schema_fingerprint,
    validate_compiled_migrations,
};
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
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    file.write_all(b"ready").unwrap();
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
             VALUES (2, 'subprocess-future-schema', zeroblob(32), 2)",
            [],
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
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
            schema_version: 2,
            access: DatabaseAccess::ReadOnlyNewer {
                found: 2,
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
        _ => panic!("unknown persistence helper mode"),
    }
}

fn fresh_v1_schema() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(test_migrations()[0].sql).unwrap();
    connection
}

fn fresh_current_schema() -> Connection {
    let connection = fresh_v1_schema();
    let migration = &test_migrations()[0];
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
    connection
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
             VALUES (2, 'future-without-sentinel', zeroblob(32), 2)",
            [],
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    drop(connection);
    drop(paths);

    let store = StoreCoordinator::open(&path).unwrap();
    assert_eq!(
        store.status().unwrap().access,
        DatabaseAccess::ReadOnlyNewer {
            found: 2,
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
             VALUES (2, 'future-schema', zeroblob(32), 2)",
            [],
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    drop(connection);

    assert_eq!(
        first.status().unwrap(),
        DatabaseStatus {
            schema_version: 2,
            access: DatabaseAccess::ReadOnlyNewer {
                found: 2,
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
        DatabaseAccess::ReadOnlyNewer { found: 2, .. }
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
             VALUES (2, 'future-schema', zeroblob(32), 2)",
            [],
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    drop(connection);

    let store = StoreCoordinator::open(&path).unwrap();
    assert_eq!(
        store.status().unwrap(),
        DatabaseStatus {
            schema_version: 2,
            access: DatabaseAccess::ReadOnlyNewer {
                found: 2,
                supported: 1,
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
                 VALUES (2, 'racing-future-schema', zeroblob(32), 2)",
                [],
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 2).unwrap();
        Ok(())
    })
    .unwrap();

    assert_eq!(
        store.status().unwrap(),
        DatabaseStatus {
            schema_version: 2,
            access: DatabaseAccess::ReadOnlyNewer {
                found: 2,
                supported: 1,
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
