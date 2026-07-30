#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

use dux_core::{EngineConfig, EngineHandle, ScanTaskStatus, TaskPhase};
use serde_json::Value;

const WAIT_TIMEOUT: Duration = Duration::from_secs(15);

#[test]
fn status_and_history_reopen_a_durable_scan_from_an_isolated_home() {
    let home = tempfile::TempDir::new().unwrap();
    let root = home.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("observed.bin"), [7_u8; 32]).unwrap();

    let config = engine_config(home.path());
    let engine = EngineHandle::open(config).unwrap();
    let task_id = engine.start_scan(root).unwrap();
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        let task = engine.task_snapshot(task_id).unwrap();
        if task.phase.is_terminal() {
            assert_eq!(task.phase, TaskPhase::Succeeded, "task: {task:?}");
            break;
        }
        assert!(Instant::now() < deadline, "engine scan did not finish");
        thread::sleep(Duration::from_millis(10));
    }
    let result = engine.scan_result(task_id).unwrap().unwrap();
    assert_eq!(result.status(), ScanTaskStatus::Succeeded);
    let scan_id = result.scan_id().as_str().to_owned();
    engine.close();
    assert!(engine.wait_until_closed(WAIT_TIMEOUT));

    let status = run_in_home(home.path(), &["status", "--json"]);
    assert_success_without_terminal_control(&status);
    let status_json: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_all_objects_are_versioned(&status_json);
    assert_eq!(status_json["command"], "status");
    assert_eq!(status_json["path_disclosure"], "none");
    assert_eq!(status_json["latest_scan"]["scan_id"], scan_id);
    assert_eq!(status_json["latest_scan"]["status"], "succeeded");
    assert_eq!(status_json["latest_scan"]["snapshot_recorded"], true);
    assert_forbidden_storage_keys_are_absent(&status_json);

    let history = run_in_home(home.path(), &["history", "--json", "--limit", "1"]);
    assert_success_without_terminal_control(&history);
    let history_json: Value = serde_json::from_slice(&history.stdout).unwrap();
    assert_all_objects_are_versioned(&history_json);
    assert_eq!(history_json["command"], "history");
    assert_eq!(history_json["requested_limit"], 1);
    assert_eq!(history_json["items"].as_array().unwrap().len(), 1);
    assert_eq!(history_json["items"][0]["scan_id"], scan_id);
    assert_forbidden_storage_keys_are_absent(&history_json);

    let detail = run_in_home(
        home.path(),
        &["scan-detail", "--scan-id", &scan_id, "--json"],
    );
    assert_success_without_terminal_control(&detail);
    let detail_json: Value = serde_json::from_slice(&detail.stdout).unwrap();
    assert_all_objects_are_versioned(&detail_json);
    assert_eq!(detail_json["command"], "scan-detail");
    assert_eq!(detail_json["scan_id"], scan_id);
    assert_eq!(detail_json["path_disclosure"], "none");
    assert_forbidden_storage_keys_are_absent(&detail_json);

    let candidates = run_in_home(
        home.path(),
        &["candidates", "--scan-id", &scan_id, "--json"],
    );
    assert_success_without_terminal_control(&candidates);
    let candidates_json: Value = serde_json::from_slice(&candidates.stdout).unwrap();
    assert_all_objects_are_versioned(&candidates_json);
    assert_eq!(candidates_json["command"], "candidates");
    assert_eq!(candidates_json["scan_id"], scan_id);
    assert_eq!(candidates_json["path_disclosure"], "none");
    assert_forbidden_storage_keys_are_absent(&candidates_json);
}

#[test]
fn invalid_history_limit_never_enters_the_tui_or_writes_stdout() {
    let home = tempfile::TempDir::new().unwrap();
    let output = run_in_home(home.path(), &["history", "--json", "--limit", "0"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
    assert!(!output.stderr.contains(&0x1b));
}

#[test]
fn status_prepares_standard_storage_in_a_completely_fresh_home() {
    let home = tempfile::TempDir::new().unwrap();
    let output = run_in_home(home.path(), &["status", "--json"]);

    assert_success_without_terminal_control(&output);
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["command"], "status");
    assert_eq!(document["latest_scan"], Value::Null);
    assert_forbidden_storage_keys_are_absent(&document);

    let (data_parent, cache_parent) = platform_parents(home.path());
    assert!(data_parent.is_dir());
    assert!(cache_parent.is_dir());
    assert!(data_parent.join("Dux/dux.sqlite3").is_file());
}

#[test]
fn advanced_commands_preserve_empty_and_typed_missing_state_over_the_process_boundary() {
    let home = tempfile::TempDir::new().unwrap();

    let cleanup = run_in_home(home.path(), &["cleanup-history", "list", "--json"]);
    assert_success_without_terminal_control(&cleanup);
    let cleanup_json: Value = serde_json::from_slice(&cleanup.stdout).unwrap();
    assert_all_objects_are_versioned(&cleanup_json);
    assert_eq!(cleanup_json["command"], "cleanup-history");
    assert_eq!(cleanup_json["operation"], "list");
    assert_eq!(cleanup_json["has_more"], false);
    assert_eq!(cleanup_json["next_cursor"], Value::Null);
    assert_eq!(cleanup_json["items"], Value::Array(Vec::new()));
    assert_forbidden_storage_keys_are_absent(&cleanup_json);

    for (arguments, expected_code) in [
        (
            vec!["scan-detail", "--scan-id", "scan:missing", "--json"],
            "scan_not_found",
        ),
        (
            vec!["candidates", "--scan-id", "scan:missing", "--json"],
            "scan_not_found",
        ),
        (
            vec![
                "review-state",
                "--scan-id",
                "scan:missing",
                "--candidate-id",
                "candidate:missing",
                "--command",
                "select",
                "--json",
            ],
            "scan_not_found",
        ),
        (
            vec![
                "cleanup-history",
                "show",
                "--session-id",
                "cleanup:missing",
                "--json",
            ],
            "cleanup_session_not_found",
        ),
    ] {
        let output = run_in_home(home.path(), &arguments);
        assert_eq!(output.status.code(), Some(3), "arguments: {arguments:?}");
        assert!(output.stdout.is_empty());
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_all_objects_are_versioned(&error);
        assert_eq!(error["error"]["code"], expected_code);
        assert_eq!(error["error"]["retryable"], false);
        assert_forbidden_storage_keys_are_absent(&error);
    }
}

#[test]
fn hidden_bundle_metadata_is_exact_path_free_and_does_not_prepare_storage() {
    let home = tempfile::TempDir::new().unwrap();
    let output = run_in_home(home.path(), &["__bundle-metadata"]);

    assert_success_without_terminal_control(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "{{\"record_version\":1,\"product\":\"dux-cli\",\"version\":\"{}\",\
             \"database_schema_version\":{},\"snapshot_format_version\":{}}}\n",
            env!("CARGO_PKG_VERSION"),
            dux_core::DATABASE_SCHEMA_VERSION,
            dux_core::SNAPSHOT_FORMAT_VERSION
        )
    );
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    let object = document.as_object().unwrap();
    assert_eq!(object.len(), 5);
    assert_eq!(object["product"], "dux-cli");
    assert_forbidden_storage_keys_are_absent(&document);

    let (data_parent, cache_parent) = platform_parents(home.path());
    assert!(!data_parent.exists());
    assert!(!cache_parent.exists());
}

fn engine_config(home: &Path) -> EngineConfig {
    let (data_parent, cache_parent) = platform_parents(home);
    std::fs::create_dir_all(&data_parent).unwrap();
    std::fs::create_dir_all(&cache_parent).unwrap();
    let data_root = data_parent.join("Dux");
    EngineConfig::new(
        data_root.join("dux.sqlite3"),
        data_root.join("snapshots"),
        cache_parent.join("Dux"),
    )
    .unwrap()
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "the CLI contract test launches only the exact Cargo-built DUX binary in an isolated HOME"
)]
fn run_in_home(home: &Path, arguments: &[&str]) -> Output {
    // DUX-DESTRUCTIVE: allow=test-cli-inspection-spawn -- launch only Cargo's exact DUX test binary with fixed test-owned arguments and an isolated HOME
    let mut command = Command::new(env!("CARGO_BIN_EXE_dux"));
    command.args(arguments).env("HOME", home);
    #[cfg(not(target_os = "macos"))]
    {
        let (data_parent, cache_parent) = platform_parents(home);
        command
            .env("XDG_DATA_HOME", data_parent)
            .env("XDG_CACHE_HOME", cache_parent);
    }
    command.output().unwrap()
}

#[cfg(target_os = "macos")]
fn platform_parents(home: &Path) -> (PathBuf, PathBuf) {
    (
        home.join("Library/Application Support"),
        home.join("Library/Caches"),
    )
}

#[cfg(not(target_os = "macos"))]
fn platform_parents(home: &Path) -> (PathBuf, PathBuf) {
    (home.join("data"), home.join("cache"))
}

fn assert_success_without_terminal_control(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains(&0x1b));
}

fn assert_all_objects_are_versioned(value: &Value) {
    match value {
        Value::Object(object) => {
            assert_eq!(
                object.get("schema_version"),
                Some(&Value::from(1)),
                "unversioned JSON object: {value}"
            );
            for child in object.values() {
                assert_all_objects_are_versioned(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                assert_all_objects_are_versioned(child);
            }
        }
        _ => {}
    }
}

fn assert_forbidden_storage_keys_are_absent(value: &Value) {
    const FORBIDDEN_KEYS: [&str; 10] = [
        "root",
        "root_path",
        "path",
        "database_path",
        "cache_path",
        "snapshot_path",
        "snapshot_name",
        "snapshot_filename",
        "snapshot_digest",
        "snapshot_checksum",
    ];
    match value {
        Value::Object(object) => {
            for key in FORBIDDEN_KEYS {
                assert!(
                    !object.contains_key(key),
                    "forbidden JSON field {key}: {value}"
                );
            }
            for child in object.values() {
                assert_forbidden_storage_keys_are_absent(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                assert_forbidden_storage_keys_are_absent(child);
            }
        }
        _ => {}
    }
}
