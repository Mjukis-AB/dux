use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use tempfile::TempDir;

use super::*;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

fn config(temp: &TempDir) -> EngineConfig {
    EngineConfig::new(
        temp.path().join("data/dux.sqlite3"),
        temp.path().join("data/snapshots"),
        temp.path().join("cache"),
    )
    .unwrap()
}

fn engine_with_limits(limits: RegistryLimits) -> (TempDir, EngineHandle) {
    let temp = TempDir::new().unwrap();
    let engine = EngineHandle::open_with_limits(config(&temp), limits).unwrap();
    (temp, engine)
}

fn wait_terminal(engine: &EngineHandle, id: TaskId) -> TaskSnapshot {
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        let snapshot = engine.task_snapshot(id).unwrap();
        if snapshot.phase.is_terminal() {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "task did not quiesce");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn final_snapshot_count(config: &EngineConfig) -> usize {
    std::fs::read_dir(config.snapshots_directory())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("snapshot-"))
        .count()
}

fn started_maintenance(outcome: HistoryMaintenanceStartOutcome) -> TaskId {
    match outcome {
        HistoryMaintenanceStartOutcome::Started(id) => id,
        other => panic!("expected started maintenance, got {other:?}"),
    }
}

fn seed_ai_insight(engine: &EngineHandle, id: &str, created_ms: i64, expires_ms: i64) {
    let mut digest = [0_u8; 32];
    for (destination, source) in digest.iter_mut().zip(id.as_bytes()) {
        *destination = *source;
    }
    engine.inner.store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO ai_insights (
                    insight_id, input_digest, provider, adapter_version,
                    model_label, output_schema_version, output_payload,
                    created_at_unix_ms, expires_at_unix_ms
                 ) VALUES (?1, ?2, 'engine-test', '1', NULL, 1, x'01', ?3, ?4)",
                rusqlite::params![id, digest, created_ms, expires_ms],
            )
            .unwrap();
    });
}

fn ai_insight_count(engine: &EngineHandle) -> i64 {
    engine.inner.store.with_connection(|connection| {
        connection
            .query_row("SELECT count(*) FROM ai_insights", [], |row| row.get(0))
            .unwrap()
    })
}

#[test]
fn handle_is_send_sync_and_config_is_explicit() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<EngineHandle>();
    assert_send_sync::<SnapshotRepository>();

    let temp = TempDir::new().unwrap();
    let expected = config(&temp);
    let engine = EngineHandle::open(expected.clone()).unwrap();
    assert_eq!(engine.config(), &expected);
    assert_eq!(
        engine.database_status().unwrap(),
        crate::persistence::DatabaseStatus {
            schema_version: crate::persistence::DATABASE_SCHEMA_VERSION,
            access: crate::persistence::DatabaseAccess::ReadWriteCurrent,
        }
    );
    engine.close();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
}

#[test]
fn snapshot_retention_cap_is_shared_versioned_and_closed_with_typed_errors() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let first = EngineHandle::open(config.clone()).unwrap();
    let second = EngineHandle::open(config).unwrap();

    assert_eq!(
        first.snapshot_retention_cap().unwrap(),
        SnapshotRetentionCap {
            cap_bytes: 2 * 1024 * 1024 * 1024,
            source: SnapshotRetentionCapSource::Default,
            updated_at: None,
        }
    );
    let explicit_default = first
        .set_snapshot_retention_cap(2 * 1024 * 1024 * 1024)
        .unwrap();
    assert!(explicit_default.changed);
    assert_eq!(
        explicit_default.settings.source,
        SnapshotRetentionCapSource::Stored
    );
    assert_eq!(
        second.snapshot_retention_cap().unwrap(),
        explicit_default.settings
    );
    let zero = first.set_snapshot_retention_cap(0).unwrap();
    assert!(zero.changed);
    assert_eq!(zero.settings.cap_bytes, 0);
    assert_eq!(zero.settings.source, SnapshotRetentionCapSource::Stored);
    assert!(zero.settings.updated_at.is_some());
    assert_eq!(second.snapshot_retention_cap().unwrap(), zero.settings);

    let exact = second.set_snapshot_retention_cap(0).unwrap();
    assert!(!exact.changed);
    assert_eq!(exact.settings, zero.settings);
    let maximum = second.set_snapshot_retention_cap(u64::MAX).unwrap();
    assert!(maximum.changed);
    assert_eq!(first.snapshot_retention_cap().unwrap(), maximum.settings);

    let reset = first.reset_snapshot_retention_cap().unwrap();
    assert!(reset.changed);
    assert_eq!(reset.settings.source, SnapshotRetentionCapSource::Default);
    assert_eq!(reset.settings.cap_bytes, 2 * 1024 * 1024 * 1024);
    assert_eq!(reset.settings.updated_at, None);
    assert_eq!(second.snapshot_retention_cap().unwrap(), reset.settings);

    first.close();
    assert!(first.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(
        first.snapshot_retention_cap(),
        Err(SnapshotRetentionCapError::Closed)
    );
    assert_eq!(
        first.set_snapshot_retention_cap(1),
        Err(SnapshotRetentionCapError::Closed)
    );
    assert_eq!(
        first.reset_snapshot_retention_cap(),
        Err(SnapshotRetentionCapError::Closed)
    );
    second.close();
    assert!(second.wait_until_closed(TEST_TIMEOUT));
}

#[test]
fn ambiguous_terminal_persistence_disarms_changed_fact_fallback() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    let start = NewScanRecord::try_new(
        ScanId::new("scan:ambiguous-terminal").unwrap(),
        engine.config().cache_directory().join("root"),
        SystemTime::now(),
    )
    .unwrap();
    let mut guard = DurableScanGuard::new(Arc::clone(&engine.inner.store), &start);

    assert_eq!(
        guard.map_settle_failure(HistoryErrorKind::OutcomeUnknown),
        TaskFailureKind::PersistenceOutcomeUnknown
    );
    assert!(guard.settled);

    guard.settled = false;
    assert_eq!(
        guard.map_settle_failure(HistoryErrorKind::DatabaseUnavailable),
        TaskFailureKind::PersistenceUnavailable
    );
    assert!(!guard.settled);
    guard.disarm();
}

#[test]
fn database_failure_prevents_engine_publication_without_echoing_paths() {
    let temp = TempDir::new().unwrap();
    let blocked_root = temp.path().join("blocked-root");
    std::fs::write(&blocked_root, b"not a directory").unwrap();
    let config = EngineConfig::new(
        blocked_root.join("dux.sqlite3"),
        blocked_root.join("snapshots"),
        temp.path().join("cache"),
    )
    .unwrap();

    let error = EngineHandle::open(config).err().unwrap();
    assert_eq!(
        error,
        EngineOpenError::Database(crate::persistence::DatabaseOpenErrorKind::UnsafeStorageRoot)
    );
    assert!(!error.to_string().contains("blocked-root"));
}

#[test]
fn snapshot_failure_prevents_engine_publication_without_echoing_paths() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let store = crate::persistence::StoreCoordinator::open(config.database_path()).unwrap();
    drop(store);
    std::fs::create_dir(config.snapshots_directory()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            config.snapshots_directory(),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }

    let error = EngineHandle::open(config).err().unwrap();
    assert!(matches!(
        error,
        EngineOpenError::Snapshot(
            crate::persistence::SnapshotOpenErrorKind::UnrecognizedStore
                | crate::persistence::SnapshotOpenErrorKind::UnsafeRoot
        )
    ));
    assert!(!error.to_string().contains("snapshots"));
}

#[test]
fn newer_database_never_provisions_missing_snapshot_storage() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    drop(crate::persistence::StoreCoordinator::open(config.database_path()).unwrap());
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    let future = crate::persistence::DATABASE_SCHEMA_VERSION + 1;
    connection
        .execute(
            "INSERT INTO schema_migrations
             (version, name, checksum_sha256, applied_at_unix_ms)
             VALUES (?1, 'future-engine-schema', zeroblob(32), 2)",
            [i64::from(future)],
        )
        .unwrap();
    connection
        .pragma_update(None, "user_version", future)
        .unwrap();
    drop(connection);
    assert!(!config.snapshots_directory().exists());

    let engine = EngineHandle::open(config.clone()).unwrap();
    assert!(matches!(
        engine.database_status().unwrap().access,
        crate::persistence::DatabaseAccess::ReadOnlyNewer { found, .. } if found == future
    ));
    assert!(!config.snapshots_directory().exists());
    let scan_root = temp.path().join("scan-root");
    std::fs::create_dir(&scan_root).unwrap();
    assert_eq!(
        engine.start_scan(scan_root),
        Err(StartTaskError::ReadOnlyStore)
    );
    assert_eq!(
        engine.start_history_maintenance(),
        Err(StartTaskError::ReadOnlyStore)
    );
    assert_eq!(
        engine.recent_scan_history(1),
        Err(ScanHistoryError::IncompatibleSchema)
    );
    assert_eq!(
        engine.snapshot_retention_cap(),
        Err(SnapshotRetentionCapError::IncompatibleSchema)
    );
    assert_eq!(
        engine.set_snapshot_retention_cap(1),
        Err(SnapshotRetentionCapError::IncompatibleSchema)
    );
    assert_eq!(
        engine.reset_snapshot_retention_cap(),
        Err(SnapshotRetentionCapError::IncompatibleSchema)
    );
    assert!(!config.snapshots_directory().exists());
    engine.close();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
}

#[test]
fn recent_scan_history_is_empty_bounded_and_closed_with_typed_errors() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    assert_eq!(
        engine.recent_scan_history(1).unwrap(),
        RecentScanHistory {
            scans: Vec::new(),
            has_more: false,
        }
    );
    for limit in [0, crate::persistence::MAX_RECENT_SCAN_HISTORY_LIMIT + 1] {
        assert_eq!(
            engine.recent_scan_history(limit),
            Err(ScanHistoryError::InvalidLimit {
                max: crate::persistence::MAX_RECENT_SCAN_HISTORY_LIMIT,
            })
        );
    }
    engine.close();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(engine.recent_scan_history(1), Err(ScanHistoryError::Closed));
}

#[test]
fn recent_scan_history_orders_ties_reports_more_and_exposes_only_succeeded_counts() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    let root = engine.config().cache_directory().join("history-root");
    let base = SystemTime::UNIX_EPOCH + Duration::from_millis(1_750_000_000_000);
    for (id, offset) in [
        ("scan:history-a", 1_u64),
        ("scan:history-b", 2),
        ("scan:history-c", 2),
    ] {
        engine
            .inner
            .store
            .record_scan_started(
                &NewScanRecord::try_new(
                    ScanId::new(id).unwrap(),
                    root.join(id),
                    base + Duration::from_millis(offset),
                )
                .unwrap(),
            )
            .unwrap();
    }
    engine
        .inner
        .store
        .record_scan_finished_reconciled(
            &ScanCompletionRecord::try_new(
                ScanId::new("scan:history-c").unwrap(),
                base + Duration::from_millis(3),
                TerminalScanStatus::Succeeded,
                ScanCounts {
                    directory_count: 7,
                    file_count: 11,
                    logical_bytes: 13,
                    allocated_bytes: Some(17),
                },
            )
            .unwrap(),
        )
        .unwrap();

    let page = engine.recent_scan_history(2).unwrap();
    assert!(page.has_more);
    assert_eq!(
        page.scans
            .iter()
            .map(|scan| scan.scan_id.as_str())
            .collect::<Vec<_>>(),
        ["scan:history-b", "scan:history-c"]
    );
    assert_eq!(page.scans[0].status, DurableScanStatus::Running);
    assert_eq!(page.scans[0].counts, None);
    assert_eq!(page.scans[1].status, DurableScanStatus::Succeeded);
    assert_eq!(
        page.scans[1].counts,
        Some(DurableScanCounts {
            directory_count: 7,
            file_count: 11,
            logical_bytes: 13,
            allocated_bytes: Some(17),
        })
    );
    assert!(!page.scans[1].snapshot_recorded);
    assert_eq!(
        page.scans[1].coverage.status,
        crate::ScanCoverageStatus::Unknown
    );
    assert_eq!(page.scans[1].coverage.issue_record_count, 0);
    assert_eq!(page.scans[1].coverage.issue_occurrence_count, 0);

    let complete = engine.recent_scan_history(3).unwrap();
    assert!(!complete.has_more);
    assert_eq!(complete.scans.len(), 3);
}

#[test]
fn recent_scan_history_exact_limit_uses_one_bounded_page_and_more_sentinel() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    let root = engine.config().cache_directory().join("history-limit-root");
    let base = SystemTime::UNIX_EPOCH + Duration::from_millis(1_750_000_000_000);
    for index in 0..=crate::persistence::MAX_RECENT_SCAN_HISTORY_LIMIT {
        engine
            .inner
            .store
            .record_scan_started(
                &NewScanRecord::try_new(
                    ScanId::new(format!("scan:history-limit:{index:03}")).unwrap(),
                    root.join(format!("root-{index:03}")),
                    base + Duration::from_millis(index as u64),
                )
                .unwrap(),
            )
            .unwrap();
    }

    let page = engine
        .recent_scan_history(crate::persistence::MAX_RECENT_SCAN_HISTORY_LIMIT)
        .unwrap();
    assert_eq!(
        page.scans.len(),
        crate::persistence::MAX_RECENT_SCAN_HISTORY_LIMIT
    );
    assert!(page.has_more);
    assert_eq!(page.scans[0].scan_id.as_str(), "scan:history-limit:200");
    assert_eq!(
        page.scans.last().unwrap().scan_id.as_str(),
        "scan:history-limit:001"
    );
}

#[test]
fn recent_scan_history_accepts_the_legal_maximum_coverage_page() {
    use crate::domain::{
        CoveragePermille, MAX_SCAN_ISSUES, ScanCoverage, ScanIssue, ScanIssueKind,
    };

    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    let parent = engine
        .config()
        .cache_directory()
        .join("history-full-coverage");
    let base = SystemTime::UNIX_EPOCH + Duration::from_millis(1_750_000_000_000);
    for index in 0..crate::persistence::MAX_RECENT_SCAN_HISTORY_LIMIT {
        let id = ScanId::new(format!("scan:history-full:{index:03}")).unwrap();
        let root = parent.join(format!("root-{index:03}"));
        engine
            .inner
            .store
            .record_scan_started(
                &NewScanRecord::try_new(
                    id.clone(),
                    root.clone(),
                    base + Duration::from_millis(index as u64),
                )
                .unwrap(),
            )
            .unwrap();
        let issues = (0..MAX_SCAN_ISSUES)
            .map(|issue_index| {
                ScanIssue::try_new(
                    ScanIssueKind::MetadataError,
                    Some(root.join(format!("issue-{issue_index:03}"))),
                    1,
                )
                .unwrap()
            })
            .collect();
        let coverage =
            ScanCoverage::try_from_terminal(Some(CoveragePermille::new(500).unwrap()), issues)
                .unwrap();
        engine
            .inner
            .store
            .record_scan_finished_reconciled(
                &ScanCompletionRecord::try_new_with_coverage(
                    id,
                    base + Duration::from_millis(1_000 + index as u64),
                    TerminalScanStatus::Succeeded,
                    ScanCounts::default(),
                    coverage,
                )
                .unwrap(),
            )
            .unwrap();
    }

    let page = engine
        .recent_scan_history(crate::persistence::MAX_RECENT_SCAN_HISTORY_LIMIT)
        .unwrap();
    assert_eq!(page.scans.len(), 200);
    assert!(!page.has_more);
    assert!(page.scans.iter().all(|scan| {
        scan.coverage.issue_record_count == MAX_SCAN_ISSUES
            && scan.coverage.issue_occurrence_count == MAX_SCAN_ISSUES as u64
    }));
}

#[test]
fn recent_scan_history_survives_process_style_reopen() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let scan_id = ScanId::new("scan:history-reopen").unwrap();
    {
        let engine = EngineHandle::open(config.clone()).unwrap();
        engine
            .inner
            .store
            .record_scan_started(
                &NewScanRecord::try_new(
                    scan_id.clone(),
                    temp.path().join("history-reopen-root"),
                    SystemTime::UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        engine.close();
        assert!(engine.wait_until_closed(TEST_TIMEOUT));
    }

    let reopened = EngineHandle::open(config).unwrap();
    let page = reopened.recent_scan_history(1).unwrap();
    assert_eq!(page.scans.len(), 1);
    assert_eq!(page.scans[0].scan_id, scan_id);
    assert_eq!(page.scans[0].status, DurableScanStatus::Running);
}

#[test]
fn recent_scan_history_rejects_corrupt_selected_coverage_row() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    let scan_id = ScanId::new("scan:history-corrupt-coverage").unwrap();
    engine
        .inner
        .store
        .record_scan_started(
            &NewScanRecord::try_new(
                scan_id.clone(),
                engine
                    .config()
                    .cache_directory()
                    .join("history-corrupt-root"),
                SystemTime::UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    let connection = rusqlite::Connection::open(engine.config().database_path()).unwrap();
    connection
        .execute(
            "UPDATE scans SET issue_count = 1 WHERE scan_id = ?1",
            [scan_id.as_str()],
        )
        .unwrap();
    drop(connection);
    assert_eq!(
        engine.recent_scan_history(1),
        Err(ScanHistoryError::CorruptData)
    );
}

#[test]
fn schema_upgrade_after_status_sample_fences_snapshot_provisioning() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let database = config.database_path().to_path_buf();
    let future = crate::persistence::DATABASE_SCHEMA_VERSION + 1;

    let error = EngineHandle::open_with_snapshot_hook(config.clone(), move || {
        let connection = rusqlite::Connection::open(database).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations
                 (version, name, checksum_sha256, applied_at_unix_ms)
                 VALUES (?1, 'future-snapshot-race', zeroblob(32), 2)",
                [i64::from(future)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", future)
            .unwrap();
    })
    .err()
    .unwrap();

    assert!(matches!(error, EngineOpenError::Snapshot(_)));
    assert!(!config.snapshots_directory().exists());
}

#[test]
fn real_format_batch_runs_through_registry_and_publishes_immutable_result() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(2, 4, 4, 8));
    let id = engine.start_format_size_batch(vec![0, 1_536]).unwrap();

    let snapshot = wait_terminal(&engine, id);
    assert_eq!(snapshot.phase, TaskPhase::Succeeded);
    assert!(snapshot.result_available);
    let result = engine.format_size_batch_result(id).unwrap().unwrap();
    assert_eq!(result.entries()[0].display, "0 B");
    assert_eq!(result.entries()[1].display, "1.5 KB");
    assert!(
        engine
            .task_events(id, 0, 8)
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(event.kind, TaskEventKind::Terminal { .. }))
    );
}

#[test]
fn history_maintenance_runs_one_typed_path_free_batch() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(10_000);
    seed_ai_insight(&engine, "ai:engine-one", 1, 9_999);

    let id = started_maintenance(engine.start_history_maintenance_at(observed).unwrap());
    let terminal = wait_terminal(&engine, id);
    assert_eq!(terminal.kind, TaskKind::HistoryMaintenance);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.result_available);
    let result = engine.history_maintenance_result(id).unwrap().unwrap();
    assert_eq!(result.observed_at(), observed);
    assert_eq!(result.daily_rollups_created(), 0);
    assert_eq!(result.raw_samples_pruned(), 0);
    assert_eq!(result.daily_rollups_pruned(), 0);
    assert_eq!(result.ai_insights_pruned(), 1);
    assert!(!result.has_more());
    assert_eq!(ai_insight_count(&engine), 0);
    assert_eq!(
        engine.format_size_batch_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.scan_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );

    let events = engine.task_events(id, 0, 8).unwrap().events;
    let applying = events
        .iter()
        .position(|event| matches!(event.kind, TaskEventKind::HistoryMaintenanceBatchApplying))
        .unwrap();
    let finished = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::HistoryMaintenanceBatchFinished {
                    daily_rollups_created: 0,
                    raw_samples_pruned: 0,
                    daily_rollups_pruned: 0,
                    ai_insights_pruned: 1,
                    has_more: false,
                }
            )
        })
        .unwrap();
    let terminal_event = events
        .iter()
        .position(|event| matches!(event.kind, TaskEventKind::Terminal { .. }))
        .unwrap();
    assert!(applying < finished && finished < terminal_event);
}

#[test]
fn history_maintenance_requires_explicit_idle_rescheduling_for_more_work() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(10_000);
    for index in 0..17 {
        seed_ai_insight(&engine, &format!("ai:engine-bounded-{index:02}"), 1, 9_999);
    }

    let first = started_maintenance(engine.start_history_maintenance_at(observed).unwrap());
    assert_eq!(wait_terminal(&engine, first).phase, TaskPhase::Succeeded);
    let first_result = engine.history_maintenance_result(first).unwrap().unwrap();
    assert_eq!(first_result.ai_insights_pruned(), 16);
    assert!(first_result.has_more());
    assert_eq!(ai_insight_count(&engine), 1);
    assert_eq!(engine.inner.shared.lock_registry_recover().records.len(), 1);

    let second = started_maintenance(engine.start_history_maintenance_at(observed).unwrap());
    assert_eq!(wait_terminal(&engine, second).phase, TaskPhase::Succeeded);
    let second_result = engine.history_maintenance_result(second).unwrap().unwrap();
    assert_eq!(second_result.ai_insights_pruned(), 1);
    assert!(!second_result.has_more());
    assert_eq!(ai_insight_count(&engine), 0);
}

#[test]
fn history_maintenance_is_safe_across_engine_sessions_sharing_one_store() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let first_engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 4, 8))
            .unwrap();
    let second_engine =
        EngineHandle::open_with_limits(config, RegistryLimits::testing(1, 2, 4, 8)).unwrap();
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(10_000);
    for index in 0..17 {
        seed_ai_insight(
            &first_engine,
            &format!("ai:engine-shared-{index:02}"),
            1,
            9_999,
        );
    }

    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first_ready_tx = ready_tx.clone();
    let first_release_rx = Arc::new(Mutex::new(release_rx));
    let second_release_rx = Arc::clone(&first_release_rx);
    let first = started_maintenance(
        first_engine
            .start_history_maintenance_with_test_hooks(
                observed,
                move || {
                    first_ready_tx.send(()).unwrap();
                    first_release_rx.lock().unwrap().recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    let second = started_maintenance(
        second_engine
            .start_history_maintenance_with_test_hooks(
                observed,
                move || {
                    ready_tx.send(()).unwrap();
                    second_release_rx.lock().unwrap().recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    ready_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    ready_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    release_tx.send(()).unwrap();
    release_tx.send(()).unwrap();

    assert_eq!(
        wait_terminal(&first_engine, first).phase,
        TaskPhase::Succeeded
    );
    assert_eq!(
        wait_terminal(&second_engine, second).phase,
        TaskPhase::Succeeded
    );
    let first_pruned = first_engine
        .history_maintenance_result(first)
        .unwrap()
        .unwrap()
        .ai_insights_pruned();
    let second_pruned = second_engine
        .history_maintenance_result(second)
        .unwrap()
        .unwrap()
        .ai_insights_pruned();
    assert_eq!(first_pruned + second_pruned, 17);
    assert_eq!(ai_insight_count(&first_engine), 0);
}

#[test]
fn corrupt_history_row_fails_typed_without_partial_mutation() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(10_000);
    seed_ai_insight(&engine, "ai:engine-corrupt", 1, 9_999);
    engine.inner.store.with_connection(|connection| {
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE ai_insights SET output_schema_version = 0
                 WHERE insight_id = 'ai:engine-corrupt'",
                [],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });

    let id = started_maintenance(engine.start_history_maintenance_at(observed).unwrap());
    let terminal = wait_terminal(&engine, id);
    assert_eq!(terminal.phase, TaskPhase::Failed);
    assert_eq!(
        terminal.failure,
        Some(TaskFailureKind::HistoryMaintenance(
            HistoryMaintenanceFailureKind::CorruptData
        ))
    );
    assert!(!terminal.result_available);
    assert_eq!(ai_insight_count(&engine), 1);
}

#[test]
fn history_maintenance_is_idle_only_and_deduplicated() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let (foreground_started_tx, foreground_started_rx) = mpsc::channel();
    let (foreground_release_tx, foreground_release_rx) = mpsc::channel();
    let foreground = engine
        .submit_test(Box::new(move |_| {
            foreground_started_tx.send(()).unwrap();
            foreground_release_rx.recv().unwrap();
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    foreground_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let foreground_record_count = engine.inner.shared.lock_registry_recover().records.len();
    assert_eq!(
        engine.start_history_maintenance().unwrap(),
        HistoryMaintenanceStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.inner.shared.lock_registry_recover().records.len(),
        foreground_record_count
    );
    foreground_release_tx.send(()).unwrap();
    wait_terminal(&engine, foreground);

    let (maintenance_started_tx, maintenance_started_rx) = mpsc::channel();
    let (maintenance_release_tx, maintenance_release_rx) = mpsc::channel();
    let first = started_maintenance(
        engine
            .start_history_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_000),
                move || {
                    maintenance_started_tx.send(()).unwrap();
                    maintenance_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    maintenance_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let (store_locked_tx, store_locked_rx) = mpsc::channel();
    let (store_release_tx, store_release_rx) = mpsc::channel();
    let store = Arc::clone(&engine.inner.store);
    let store_holder = std::thread::spawn(move || {
        store.with_connection(|_| {
            store_locked_tx.send(()).unwrap();
            store_release_rx.recv().unwrap();
        });
    });
    store_locked_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let (duplicate_tx, duplicate_rx) = mpsc::channel();
    let duplicate_engine = engine.clone();
    let duplicate_request = std::thread::spawn(move || {
        duplicate_tx
            .send(duplicate_engine.start_history_maintenance())
            .unwrap();
    });
    let duplicate = duplicate_rx.recv_timeout(Duration::from_secs(1));
    store_release_tx.send(()).unwrap();
    store_holder.join().unwrap();
    duplicate_request.join().unwrap();
    assert_eq!(
        duplicate.unwrap().unwrap(),
        HistoryMaintenanceStartOutcome::AlreadyActive(first)
    );
    assert_eq!(engine.inner.shared.lock_registry_recover().records.len(), 2);
    maintenance_release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, first).phase, TaskPhase::Succeeded);
}

#[test]
fn cancellation_before_history_batch_mutates_nothing_and_releases_admission() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(10_000);
    seed_ai_insight(&engine, "ai:engine-cancel-before", 1, 9_999);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let id = started_maintenance(
        engine
            .start_history_maintenance_with_test_hooks(
                observed,
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(engine.cancel_task(id).unwrap(), CancelOutcome::Requested);
    release_tx.send(()).unwrap();
    let terminal = wait_terminal(&engine, id);
    assert_eq!(terminal.phase, TaskPhase::Cancelled);
    assert!(!terminal.result_available);
    assert_eq!(ai_insight_count(&engine), 1);
    assert!(
        engine
            .task_events(id, 0, 8)
            .unwrap()
            .events
            .iter()
            .all(|event| !matches!(event.kind, TaskEventKind::HistoryMaintenanceBatchApplying))
    );

    let replacement = started_maintenance(engine.start_history_maintenance_at(observed).unwrap());
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );
    assert_eq!(ai_insight_count(&engine), 0);
}

#[test]
fn cancellation_after_history_commit_is_intent_not_a_terminal_rewrite() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(10_000);
    seed_ai_insight(&engine, "ai:engine-cancel-after", 1, 9_999);
    let (committed_tx, committed_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let id = started_maintenance(
        engine
            .start_history_maintenance_with_test_hooks(
                observed,
                || {},
                move || {
                    committed_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
            )
            .unwrap(),
    );
    committed_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(ai_insight_count(&engine), 0);
    assert_eq!(engine.cancel_task(id).unwrap(), CancelOutcome::Requested);
    release_tx.send(()).unwrap();
    let terminal = wait_terminal(&engine, id);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.cancellation_requested);
    assert_eq!(
        engine
            .history_maintenance_result(id)
            .unwrap()
            .unwrap()
            .ai_insights_pruned(),
        1
    );
}

#[test]
fn schema_upgrade_while_history_task_waits_fails_typed_without_mutation() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(10_000);
    seed_ai_insight(&engine, "ai:engine-version-race", 1, 9_999);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let id = started_maintenance(
        engine
            .start_history_maintenance_with_test_hooks(
                observed,
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let future = crate::persistence::DATABASE_SCHEMA_VERSION + 1;
    engine.inner.store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO schema_migrations
                 (version, name, checksum_sha256, applied_at_unix_ms)
                 VALUES (?1, 'future-maintenance-race', zeroblob(32), 2)",
                [i64::from(future)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", future)
            .unwrap();
    });
    assert_eq!(
        engine.start_history_maintenance().unwrap(),
        HistoryMaintenanceStartOutcome::AlreadyActive(id)
    );
    release_tx.send(()).unwrap();

    let terminal = wait_terminal(&engine, id);
    assert_eq!(terminal.phase, TaskPhase::Failed);
    assert_eq!(
        terminal.failure,
        Some(TaskFailureKind::HistoryMaintenance(
            HistoryMaintenanceFailureKind::IncompatibleSchema
        ))
    );
    assert!(!terminal.result_available);
    assert_eq!(ai_insight_count(&engine), 1);
}

#[test]
fn history_maintenance_failure_mapping_is_exhaustive_and_stable() {
    for (input, expected) in [
        (
            HistoryErrorKind::InvalidInput,
            HistoryMaintenanceFailureKind::InvalidClock,
        ),
        (
            HistoryErrorKind::IncompatibleSchema,
            HistoryMaintenanceFailureKind::IncompatibleSchema,
        ),
        (
            HistoryErrorKind::QueryLimitExceeded,
            HistoryMaintenanceFailureKind::BudgetExceeded,
        ),
        (HistoryErrorKind::Busy, HistoryMaintenanceFailureKind::Busy),
        (
            HistoryErrorKind::UnsafeStorage,
            HistoryMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            HistoryErrorKind::CorruptData,
            HistoryMaintenanceFailureKind::CorruptData,
        ),
        (
            HistoryErrorKind::DatabaseUnavailable,
            HistoryMaintenanceFailureKind::Unavailable,
        ),
        (
            HistoryErrorKind::OutcomeUnknown,
            HistoryMaintenanceFailureKind::OutcomeUnknown,
        ),
        (
            HistoryErrorKind::InternalState,
            HistoryMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::AlreadyExists,
            HistoryMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::NotFound,
            HistoryMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::InvalidTransition,
            HistoryMaintenanceFailureKind::InternalState,
        ),
    ] {
        assert_eq!(
            map_history_maintenance_failure(input),
            TaskFailureKind::HistoryMaintenance(expected)
        );
    }
}

#[test]
fn invalid_maintenance_clock_fails_and_panic_releases_exclusive_admission() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let invalid = started_maintenance(
        engine
            .start_history_maintenance_at(SystemTime::UNIX_EPOCH - Duration::from_millis(1))
            .unwrap(),
    );
    let invalid_terminal = wait_terminal(&engine, invalid);
    assert_eq!(
        invalid_terminal.failure,
        Some(TaskFailureKind::HistoryMaintenance(
            HistoryMaintenanceFailureKind::InvalidClock
        ))
    );

    let panicking = started_maintenance(
        engine
            .start_history_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_000),
                || panic!("maintenance hook panic"),
                || {},
            )
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, panicking).phase, TaskPhase::Failed);
    let replacement = started_maintenance(
        engine
            .start_history_maintenance_at(SystemTime::UNIX_EPOCH + Duration::from_millis(10_000))
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );
}

#[test]
fn completed_scan_publishes_durable_snapshot_and_survives_reopen() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir_all(root.join("nested")).unwrap();
    std::fs::write(root.join("nested/payload"), b"payload").unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 4, 8, 16))
            .unwrap();

    let task = engine.start_scan(root.clone()).unwrap();
    let terminal = wait_terminal(&engine, task);
    assert_eq!(terminal.kind, TaskKind::Scan);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.result_available);
    let result = engine.scan_result(task).unwrap().unwrap();
    assert_eq!(
        engine.format_size_batch_result(task).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(result.status(), ScanTaskStatus::Succeeded);
    assert!(result.snapshot_available());
    assert_eq!(
        result.candidate_evaluation(),
        CandidateEvaluationTaskStatus::Succeeded { candidate_count: 0 }
    );
    assert_eq!(result.counts().logical_bytes, 7);
    assert_eq!(result.counts().file_count, 1);
    assert_eq!(result.counts().directory_count, 2);
    assert_ne!(
        result.coverage().status(),
        crate::ScanCoverageStatus::Unknown
    );
    let scan_id = result.scan_id().clone();
    let durable = engine.inner.store.load_scan(&scan_id).unwrap().unwrap();
    assert_eq!(durable.status(), ScanStatus::Succeeded);
    assert_eq!(durable.started_at(), result.started_at());
    assert_eq!(durable.completed_at(), Some(result.completed_at()));
    assert_eq!(durable.root(), root.canonicalize().unwrap());
    assert_eq!(durable.counts().logical_bytes, 7);
    assert_eq!(durable.coverage(), result.coverage());
    assert_eq!(
        engine
            .inner
            .store
            .load_candidate_evaluation(&scan_id)
            .unwrap()
            .unwrap()
            .status(),
        crate::persistence::CandidateEvaluationStatus::Succeeded { candidate_count: 0 }
    );
    let reference = durable.snapshot().unwrap().clone();
    let document = engine.inner.snapshots.load(&reference).unwrap();
    assert_eq!(document.metadata.scan_id, scan_id);
    assert_eq!(document.metadata.totals.logical_bytes, 7);
    assert_eq!(document.nodes.len(), 3);
    assert_eq!(final_snapshot_count(&config), 1);
    let events = engine.task_events(task, 0, 16).unwrap().events;
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, TaskEventKind::ScanFinalizing))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, TaskEventKind::ScanProgress { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, TaskEventKind::CandidateEvaluationStarted))
    );
    assert!(events.iter().any(|event| matches!(
        event.kind,
        TaskEventKind::CandidateEvaluationFinished {
            status: CandidateEvaluationTaskStatus::Succeeded { candidate_count: 0 }
        }
    )));
    let sequence_for = |predicate: fn(&TaskEventKind) -> bool| {
        events
            .iter()
            .find(|event| predicate(&event.kind))
            .unwrap()
            .sequence
    };
    let finalizing = sequence_for(|kind| matches!(kind, TaskEventKind::ScanFinalizing));
    let evaluating = sequence_for(|kind| matches!(kind, TaskEventKind::CandidateEvaluationStarted));
    let evaluated =
        sequence_for(|kind| matches!(kind, TaskEventKind::CandidateEvaluationFinished { .. }));
    let terminal_event = sequence_for(|kind| matches!(kind, TaskEventKind::Terminal { .. }));
    assert!(finalizing < evaluating && evaluating < evaluated && evaluated < terminal_event);

    engine.close();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    drop(engine);
    let reopened = EngineHandle::open(config).unwrap();
    let durable = reopened.inner.store.load_scan(&scan_id).unwrap().unwrap();
    let reference = durable.snapshot().unwrap();
    assert_eq!(reopened.inner.snapshots.load(reference).unwrap(), document);
    assert_eq!(
        reopened
            .inner
            .store
            .load_candidate_evaluation(&scan_id)
            .unwrap()
            .unwrap()
            .status(),
        crate::persistence::CandidateEvaluationStatus::Succeeded { candidate_count: 0 }
    );
}

#[test]
fn completed_scan_persists_marker_verified_discovery_batch_across_reopen() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    let project = root.join("project");
    let target = project.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(project.join("Cargo.toml"), b"[package]\nname='fixture'\n").unwrap();
    std::fs::write(target.join("object"), vec![7_u8; 8 * 1024]).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 4, 8, 16))
            .unwrap();

    let task = engine.start_scan(root).unwrap();
    let terminal = wait_terminal(&engine, task);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    let result = engine.scan_result(task).unwrap().unwrap();
    assert_eq!(
        result.candidate_evaluation(),
        CandidateEvaluationTaskStatus::Succeeded { candidate_count: 1 }
    );
    let scan_id = result.scan_id().clone();
    let evaluation = engine
        .inner
        .store
        .load_candidate_evaluation(&scan_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        evaluation.status(),
        crate::persistence::CandidateEvaluationStatus::Succeeded { candidate_count: 1 }
    );
    let candidate = &evaluation.candidates()[0];
    assert_eq!(candidate.source_scan_id, scan_id);
    assert_eq!(candidate.rule.id().as_str(), "developer.rust.target");
    assert_eq!(candidate.paths, [target.canonicalize().unwrap()]);
    assert!(candidate.estimated_bytes > 0);
    assert_eq!(candidate.safety, crate::SafetyTier::Informational);
    assert_eq!(candidate.action, crate::CandidateAction::RevealOnly);
    assert!(!candidate.rule_schedule_eligible);
    assert_eq!(candidate.blockers, [crate::BlockReason::ProtectedPath]);
    assert!(candidate.evidence.iter().any(|evidence| matches!(
        evidence,
        crate::Evidence::RequiredMarker { path }
            if path.file_name().is_some_and(|name| name == "Cargo.toml")
    )));

    engine.close();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    drop(engine);
    let reopened = EngineHandle::open(config).unwrap();
    let reopened_evaluation = reopened
        .inner
        .store
        .load_candidate_evaluation(&scan_id)
        .unwrap()
        .unwrap();
    assert_eq!(reopened_evaluation, evaluation);
}

#[test]
fn candidate_limit_failure_maps_to_the_public_discovery_status() {
    let kind = map_candidate_evaluation_error(CandidateEvaluationError::CandidateLimitExceeded {
        observed_at_least: crate::domain::MAX_EVALUATED_CANDIDATES + 1,
        maximum: crate::domain::MAX_EVALUATED_CANDIDATES,
    });
    assert_eq!(kind, CandidateEvaluationFailureKind::LimitExceeded);
    let identity = CandidateEvaluationIdentity::try_new(1, 1, [1; 32], 1, [2; 32]).unwrap();
    let (_, _, status) =
        failed_candidate_evaluation(identity, SystemTime::UNIX_EPOCH, kind).unwrap();
    assert_eq!(
        status,
        CandidateEvaluationTaskStatus::Failed {
            kind: CandidateEvaluationTaskFailureKind::LimitExceeded
        }
    );
}

#[test]
fn cancellation_after_completed_traversal_cancels_discovery_not_scan() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("payload"), b"payload").unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 4, 16))
            .unwrap();
    let (reached_tx, reached_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let task = engine
        .start_scan_with_before_candidate_evaluation_hook(root, move || {
            reached_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();
    reached_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    assert_eq!(engine.cancel_task(task).unwrap(), CancelOutcome::Requested);
    release_tx.send(()).unwrap();
    let terminal = wait_terminal(&engine, task);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.cancellation_requested);
    let result = engine.scan_result(task).unwrap().unwrap();
    assert_eq!(result.status(), ScanTaskStatus::Succeeded);
    assert!(result.snapshot_available());
    assert_eq!(
        result.candidate_evaluation(),
        CandidateEvaluationTaskStatus::Failed {
            kind: CandidateEvaluationTaskFailureKind::Cancelled
        }
    );
    let durable = engine
        .inner
        .store
        .load_scan(result.scan_id())
        .unwrap()
        .unwrap();
    assert_eq!(durable.status(), ScanStatus::Succeeded);
    assert!(durable.snapshot().is_some());
    let evaluation = engine
        .inner
        .store
        .load_candidate_evaluation(result.scan_id())
        .unwrap()
        .unwrap();
    assert_eq!(
        evaluation.status(),
        crate::persistence::CandidateEvaluationStatus::Failed {
            kind: CandidateEvaluationFailureKind::Cancelled
        }
    );
    assert!(evaluation.candidates().is_empty());
    assert_eq!(final_snapshot_count(&config), 1);
}

#[test]
fn cancellation_after_discovery_checkpoint_is_intent_not_a_terminal_rewrite() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("payload"), b"payload").unwrap();
    let engine =
        EngineHandle::open_with_limits(config, RegistryLimits::testing(1, 2, 4, 16)).unwrap();
    let (reached_tx, reached_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let task = engine
        .start_scan_with_before_candidate_persistence_hook(root, move || {
            reached_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();
    reached_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    assert_eq!(engine.cancel_task(task).unwrap(), CancelOutcome::Requested);
    release_tx.send(()).unwrap();
    let terminal = wait_terminal(&engine, task);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.cancellation_requested);
    let result = engine.scan_result(task).unwrap().unwrap();
    assert_eq!(result.status(), ScanTaskStatus::Succeeded);
    assert_eq!(
        result.candidate_evaluation(),
        CandidateEvaluationTaskStatus::Succeeded { candidate_count: 0 }
    );
    assert_eq!(
        engine
            .inner
            .store
            .load_candidate_evaluation(result.scan_id())
            .unwrap()
            .unwrap()
            .status(),
        crate::persistence::CandidateEvaluationStatus::Succeeded { candidate_count: 0 }
    );
}

#[test]
fn running_scan_cancellation_is_durable_and_publishes_no_snapshot() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("payload"), b"payload").unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 4, 16))
            .unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let task = engine
        .start_scan_with_before_traversal_hook(root, move |scan_id| {
            started_tx.send(scan_id.clone()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();
    let scan_id = started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    assert_eq!(engine.cancel_task(task).unwrap(), CancelOutcome::Requested);
    assert_eq!(engine.scan_result(task).unwrap(), None);
    release_tx.send(()).unwrap();
    let terminal = wait_terminal(&engine, task);
    assert_eq!(terminal.phase, TaskPhase::Cancelled);
    assert!(terminal.cancellation_requested);
    assert!(terminal.result_available);
    let result = engine.scan_result(task).unwrap().unwrap();
    assert_eq!(result.scan_id(), &scan_id);
    assert_eq!(result.status(), ScanTaskStatus::Cancelled);
    assert!(!result.snapshot_available());
    assert_eq!(
        result.candidate_evaluation(),
        CandidateEvaluationTaskStatus::NotRun
    );
    assert_eq!(result.counts(), ScanTaskCounts::default());
    assert!(
        result
            .coverage()
            .issues()
            .iter()
            .any(|issue| { issue.kind() == crate::ScanIssueKind::Cancelled })
    );
    let durable = engine.inner.store.load_scan(&scan_id).unwrap().unwrap();
    assert_eq!(durable.status(), ScanStatus::Cancelled);
    assert!(durable.snapshot().is_none());
    assert!(
        engine
            .inner
            .store
            .load_candidate_evaluation(&scan_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(final_snapshot_count(&config), 0);
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test moves only a TempDir-owned scan root to force a deterministic scanner failure"
)]
fn scanner_failure_is_durable_and_publishes_no_snapshot() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    let moved = temp.path().join("moved-root");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 4, 16))
            .unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let task = engine
        .start_scan_with_before_traversal_hook(root.clone(), move |scan_id| {
            started_tx.send(scan_id.clone()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();
    let scan_id = started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    // DUX-DESTRUCTIVE: allow=test-engine-move-scan-root -- move only this TempDir-owned root so the real scanner observes a deterministic missing-root failure
    std::fs::rename(&root, moved).unwrap();
    release_tx.send(()).unwrap();

    let terminal = wait_terminal(&engine, task);
    assert_eq!(terminal.phase, TaskPhase::Failed);
    assert_eq!(terminal.failure, Some(TaskFailureKind::ScanFailed));
    let result = engine.scan_result(task).unwrap().unwrap();
    assert_eq!(result.status(), ScanTaskStatus::Failed);
    assert_eq!(result.scan_id(), &scan_id);
    assert!(!result.snapshot_available());
    assert_eq!(
        result.candidate_evaluation(),
        CandidateEvaluationTaskStatus::NotRun
    );
    let durable = engine.inner.store.load_scan(&scan_id).unwrap().unwrap();
    assert_eq!(durable.status(), ScanStatus::Failed);
    assert!(durable.snapshot().is_none());
    assert!(
        engine
            .inner
            .store
            .load_candidate_evaluation(&scan_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(final_snapshot_count(&config), 0);
}

#[test]
fn queued_scan_cancellation_never_starts_or_persists() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 4, 8))
            .unwrap();
    let (blocker_started_tx, blocker_started_rx) = mpsc::channel();
    let (blocker_release_tx, blocker_release_rx) = mpsc::channel();
    let blocker = engine
        .submit_test(Box::new(move |_| {
            blocker_started_tx.send(()).unwrap();
            blocker_release_rx.recv().unwrap();
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    blocker_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&hook_calls);
    let scan = engine
        .start_scan_with_before_traversal_hook(root.clone(), move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

    assert_eq!(
        engine.cancel_task(scan).unwrap(),
        CancelOutcome::CancelledBeforeStart
    );
    assert_eq!(wait_terminal(&engine, scan).phase, TaskPhase::Cancelled);
    assert_eq!(hook_calls.load(Ordering::SeqCst), 0);
    blocker_release_tx.send(()).unwrap();
    wait_terminal(&engine, blocker);
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM scans", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(final_snapshot_count(&config), 0);
    let replacement = engine.start_scan(root).unwrap();
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );
}

#[test]
fn overlapping_scan_scope_is_rejected_then_released() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("scan-root");
    let child = root.join("child");
    std::fs::create_dir_all(&child).unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(2, 4, 8, 8)).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first = engine
        .start_scan_with_before_traversal_hook(root.clone(), move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    assert_eq!(
        engine.start_scan(root.clone()),
        Err(StartTaskError::ScanAlreadyActive { existing: first })
    );
    assert_eq!(
        engine.start_scan(child),
        Err(StartTaskError::ScanAlreadyActive { existing: first })
    );
    assert_eq!(engine.cancel_task(first).unwrap(), CancelOutcome::Requested);
    release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, first).phase, TaskPhase::Cancelled);
    let replacement = engine.start_scan(root).unwrap();
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );
}

#[test]
fn scan_results_are_kind_checked_and_invalid_roots_fail_before_queueing() {
    let temp = TempDir::new().unwrap();
    let engine = EngineHandle::open(config(&temp)).unwrap();
    let format = engine.start_format_size_batch(vec![1]).unwrap();
    assert_eq!(
        engine.scan_result(format).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    let relative = PathBuf::from("relative-root");
    assert_eq!(
        engine.start_scan(relative),
        Err(StartTaskError::InvalidScanRoot {
            reason: ScanRootErrorKind::InvalidPath
        })
    );
    let file = temp.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(
        engine.start_scan(file),
        Err(StartTaskError::InvalidScanRoot {
            reason: ScanRootErrorKind::NotDirectory
        })
    );
}

#[test]
fn live_schema_upgrade_fences_scan_submission_before_queueing() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine = EngineHandle::open(config.clone()).unwrap();
    let future = crate::persistence::DATABASE_SCHEMA_VERSION + 1;
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations
             (version, name, checksum_sha256, applied_at_unix_ms)
             VALUES (?1, 'future-live-scan-schema', zeroblob(32), 2)",
            [i64::from(future)],
        )
        .unwrap();
    connection
        .pragma_update(None, "user_version", future)
        .unwrap();
    drop(connection);

    assert_eq!(engine.start_scan(root), Err(StartTaskError::ReadOnlyStore));
    assert!(matches!(
        engine.database_status().unwrap().access,
        crate::persistence::DatabaseAccess::ReadOnlyNewer { found, .. } if found == future
    ));
    let format = engine.start_format_size_batch(vec![1]).unwrap();
    assert_eq!(wait_terminal(&engine, format).phase, TaskPhase::Succeeded);
}

#[cfg(unix)]
#[test]
fn canonical_root_alias_shares_the_active_scan_scope() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let root = temp.path().join("scan-root");
    let alias = temp.path().join("scan-alias");
    std::fs::create_dir(&root).unwrap();
    symlink(&root, &alias).unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 2, 4, 8)).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first = engine
        .start_scan_with_before_traversal_hook(root, move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    assert_eq!(
        engine.start_scan(alias),
        Err(StartTaskError::ScanAlreadyActive { existing: first })
    );
    assert_eq!(engine.cancel_task(first).unwrap(), CancelOutcome::Requested);
    release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, first).phase, TaskPhase::Cancelled);
}

#[test]
fn panicking_scan_hook_is_durably_interrupted_and_releases_scope() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 2, 4, 8)).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let task = engine
        .start_scan_with_before_traversal_hook(root.clone(), move |scan_id| {
            started_tx.send(scan_id.clone()).unwrap();
            panic!("injected scan task panic");
        })
        .unwrap();
    let scan_id = started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let terminal = wait_terminal(&engine, task);
    assert_eq!(terminal.phase, TaskPhase::Failed);
    assert_eq!(terminal.failure, Some(TaskFailureKind::InternalFailure));
    assert!(!terminal.result_available);
    let durable = engine.inner.store.load_scan(&scan_id).unwrap().unwrap();
    assert_eq!(durable.status(), ScanStatus::Interrupted);
    assert!(durable.snapshot().is_none());
    let replacement = engine.start_scan(root).unwrap();
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );
    let format = engine.start_format_size_batch(vec![1]).unwrap();
    assert_eq!(wait_terminal(&engine, format).phase, TaskPhase::Succeeded);
}

#[test]
fn close_cancels_and_terminalizes_a_running_scan_before_workers_exit() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 2, 4, 8)).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    engine
        .start_scan_with_before_traversal_hook(root, move |scan_id| {
            started_tx.send(scan_id.clone()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();
    let scan_id = started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    assert_eq!(engine.close(), CloseOutcome::Initiated);
    release_tx.send(()).unwrap();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    let durable = engine.inner.store.load_scan(&scan_id).unwrap().unwrap();
    assert_eq!(durable.status(), ScanStatus::Cancelled);
    assert!(durable.snapshot().is_none());
}

#[test]
fn format_batch_input_is_bounded() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    assert_eq!(
        engine.start_format_size_batch(vec![0; FORMAT_BATCH_LIMIT + 1]),
        Err(StartTaskError::InputTooLarge { limit: 256 })
    );
}

#[test]
fn identifiers_are_nonzero_monotonic_and_allocator_never_wraps() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 4, 4, 4));
    let first = engine.start_format_size_batch(Vec::new()).unwrap();
    let second = engine.start_format_size_batch(Vec::new()).unwrap();
    let (_other_temp, other_engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    let across_engines = other_engine.start_format_size_batch(Vec::new()).unwrap();
    assert!(first.get() > 0);
    assert!(second > first);
    assert!(across_engines > second);

    let allocator = TaskIdAllocator::new(NonZeroU64::MAX);
    assert_eq!(allocator.allocate().unwrap().get(), u64::MAX);
    assert_eq!(allocator.allocate(), Err(StartTaskError::TaskIdExhausted));
}

#[test]
fn queue_capacity_is_fixed_and_queued_cancellation_never_executes() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 4, 8));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let running = engine
        .submit_test(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let executions = Arc::new(AtomicUsize::new(0));
    let executions_for_job = Arc::clone(&executions);
    let queued = engine
        .submit_test(Box::new(move |_| {
            executions_for_job.fetch_add(1, Ordering::SeqCst);
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    assert_eq!(
        engine.start_format_size_batch(Vec::new()),
        Err(StartTaskError::QueueFull)
    );
    assert_eq!(
        engine.cancel_task(queued).unwrap(),
        CancelOutcome::CancelledBeforeStart
    );
    assert_eq!(wait_terminal(&engine, queued).phase, TaskPhase::Cancelled);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    let replacement = engine.start_format_size_batch(vec![1]).unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, running).phase, TaskPhase::Succeeded);
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );
}

#[test]
fn configured_workers_are_an_exact_concurrency_bound() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(2, 6, 8, 4));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let mut ids = Vec::new();

    for _ in 0..6 {
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        let started_tx = started_tx.clone();
        let release_rx = Arc::clone(&release_rx);
        ids.push(
            engine
                .submit_test(Box::new(move |_| {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    started_tx.send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                    WorkOutcome::Succeeded(TaskResult::TestOnly)
                }))
                .unwrap(),
        );
    }

    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(active.load(Ordering::SeqCst), 2);
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    for _ in 0..ids.len() {
        release_tx.send(()).unwrap();
    }
    for id in ids {
        assert_eq!(wait_terminal(&engine, id).phase, TaskPhase::Succeeded);
    }
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

#[test]
fn one_worker_executes_queued_tasks_in_fifo_order() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 4, 4, 4));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let blocker = engine
        .submit_test(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let order = Arc::new(Mutex::new(Vec::new()));
    let mut queued = Vec::new();
    for index in 0..3 {
        let order = Arc::clone(&order);
        queued.push(
            engine
                .submit_test(Box::new(move |_| {
                    order.lock().unwrap().push(index);
                    WorkOutcome::Succeeded(TaskResult::TestOnly)
                }))
                .unwrap(),
        );
    }
    release_tx.send(()).unwrap();
    wait_terminal(&engine, blocker);
    for id in queued {
        wait_terminal(&engine, id);
    }
    assert_eq!(*order.lock().unwrap(), vec![0, 1, 2]);
}

#[test]
fn running_cancellation_is_intent_until_worker_returns() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let (started_tx, started_rx) = mpsc::channel();
    let (observed_tx, observed_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let id = engine
        .submit_test(Box::new(move |context| {
            started_tx.send(()).unwrap();
            while !context.is_cancellation_requested() {
                std::thread::yield_now();
            }
            observed_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Cancelled(None)
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    assert_eq!(engine.cancel_task(id).unwrap(), CancelOutcome::Requested);
    assert_eq!(
        engine.cancel_task(id).unwrap(),
        CancelOutcome::AlreadyRequested
    );
    observed_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let pending = engine.task_snapshot(id).unwrap();
    assert_eq!(pending.phase, TaskPhase::Running);
    assert!(pending.cancellation_requested);
    release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, id).phase, TaskPhase::Cancelled);
    assert_eq!(
        engine.cancel_task(id).unwrap(),
        CancelOutcome::AlreadyTerminal
    );
}

#[test]
fn cancellation_intent_does_not_rewrite_completed_or_failed_work() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let completed = engine
        .submit_test(Box::new(move |_| {
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    ready_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(completed).unwrap(),
        CancelOutcome::Requested
    );
    release_tx.send(()).unwrap();
    let completed_snapshot = wait_terminal(&engine, completed);
    assert_eq!(completed_snapshot.phase, TaskPhase::Succeeded);
    assert!(completed_snapshot.cancellation_requested);
    assert!(completed_snapshot.result_available);

    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let failed = engine
        .submit_test(Box::new(move |_| {
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            panic!("sanitized task failure")
        }))
        .unwrap();
    ready_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(failed).unwrap(),
        CancelOutcome::Requested
    );
    release_tx.send(()).unwrap();
    let failed_snapshot = wait_terminal(&engine, failed);
    assert_eq!(failed_snapshot.phase, TaskPhase::Failed);
    assert!(failed_snapshot.cancellation_requested);
    assert_eq!(
        failed_snapshot.failure,
        Some(TaskFailureKind::InternalFailure)
    );
}

#[test]
fn worker_panic_is_contained_and_pool_survives() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let failed = engine
        .submit_test(Box::new(|_| panic!("payload must not escape")))
        .unwrap();
    let failed_snapshot = wait_terminal(&engine, failed);
    assert_eq!(failed_snapshot.phase, TaskPhase::Failed);
    assert_eq!(
        failed_snapshot.failure,
        Some(TaskFailureKind::InternalFailure)
    );

    let next = engine.start_format_size_batch(vec![1]).unwrap();
    assert_eq!(wait_terminal(&engine, next).phase, TaskPhase::Succeeded);
}

#[test]
fn terminal_records_and_events_are_bounded() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 1, 3));
    let first = engine.start_format_size_batch(vec![1, 2, 3, 4]).unwrap();
    wait_terminal(&engine, first);
    let page = engine.task_events(first, 0, 3).unwrap();
    assert_eq!(page.events.len(), 3);
    assert!(page.truncated);
    assert!(page.terminal);

    let second = engine.start_format_size_batch(Vec::new()).unwrap();
    wait_terminal(&engine, second);
    assert_eq!(
        engine.task_snapshot(first),
        Err(TaskAccessError::UnknownTask)
    );
    assert!(engine.task_snapshot(second).is_ok());
    assert_eq!(
        engine.task_events(second, 0, 0),
        Err(TaskAccessError::InvalidEventLimit { max: 3 })
    );
    assert_eq!(
        engine.task_events(second, 0, 4),
        Err(TaskAccessError::InvalidEventLimit { max: 3 })
    );
}

#[test]
fn aggregate_registry_state_is_bounded_and_workers_are_reaped() {
    let limits = RegistryLimits::testing(1, 2, 2, 4);
    let (_temp, engine) = engine_with_limits(limits);
    let shared = Arc::clone(&engine.inner.shared);
    let first_terminal = engine.start_format_size_batch(Vec::new()).unwrap();
    wait_terminal(&engine, first_terminal);
    let second_terminal = engine.start_format_size_batch(Vec::new()).unwrap();
    wait_terminal(&engine, second_terminal);

    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let running = engine
        .submit_test(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Cancelled(None)
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    let executions = Arc::new(AtomicUsize::new(0));
    let mut queued = Vec::new();
    for _ in 0..limits.queued_tasks {
        let executions = Arc::clone(&executions);
        queued.push(
            engine
                .submit_test(Box::new(move |_| {
                    executions.fetch_add(1, Ordering::SeqCst);
                    WorkOutcome::Succeeded(TaskResult::TestOnly)
                }))
                .unwrap(),
        );
    }

    {
        let registry = shared.lock_registry_recover();
        assert_eq!(registry.live_workers, limits.workers);
        assert_eq!(registry.running_tasks, limits.workers);
        assert_eq!(registry.queue.len(), limits.queued_tasks);
        assert_eq!(
            registry.terminal_order.len(),
            limits.retained_terminal_tasks
        );
        assert_eq!(
            registry.records.len(),
            limits.workers + limits.queued_tasks + limits.retained_terminal_tasks
        );
        for id in [first_terminal, second_terminal, running]
            .into_iter()
            .chain(queued.iter().copied())
        {
            assert!(registry.records.contains_key(&id));
        }
    }

    assert_eq!(engine.close(), CloseOutcome::Initiated);
    release_tx.send(()).unwrap();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    {
        let registry = shared.lock_registry_recover();
        assert_eq!(registry.live_workers, 0);
        assert_eq!(registry.running_tasks, 0);
        assert!(registry.queue.is_empty());
        assert!(registry.records.len() <= limits.retained_terminal_tasks);
    }
    assert!(engine.inner.workers.lock().unwrap().is_none());
}

#[test]
fn event_pages_are_contiguous_and_reject_future_cursors() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let id = engine.start_format_size_batch(vec![1, 2]).unwrap();
    let snapshot = wait_terminal(&engine, id);

    let first = engine.task_events(id, 0, 2).unwrap();
    let second = engine.task_events(id, first.next_sequence, 8).unwrap();
    let sequences: Vec<_> = first
        .events
        .iter()
        .chain(second.events.iter())
        .map(|event| event.sequence)
        .collect();
    assert_eq!(sequences, vec![1, 2, 3, 4, 5]);
    assert!(matches!(
        second.events.last().map(|event| &event.kind),
        Some(TaskEventKind::Terminal { .. })
    ));
    assert_eq!(snapshot.revision, 5);
    let current = engine.task_events(id, second.next_sequence, 8).unwrap();
    assert!(current.events.is_empty());
    assert_eq!(current.next_sequence, second.next_sequence);
    assert_eq!(
        engine.task_events(id, second.next_sequence + 1, 8),
        Err(TaskAccessError::InvalidEventCursor)
    );
    assert_eq!(
        engine.task_events(id, u64::MAX, 8),
        Err(TaskAccessError::InvalidEventCursor)
    );
}

#[test]
fn close_is_nonblocking_idempotent_cancels_work_and_rejects_use() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let shared = Arc::clone(&engine.inner.shared);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let running = engine
        .submit_test(Box::new(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            WorkOutcome::Cancelled(None)
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let queued_executions = Arc::clone(&executions);
    let queued = engine
        .submit_test(Box::new(move |_| {
            queued_executions.fetch_add(1, Ordering::SeqCst);
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();

    assert_eq!(engine.close(), CloseOutcome::Initiated);
    assert_eq!(engine.lifecycle(), EngineLifecycle::Closing);
    assert!(!engine.wait_until_closed(Duration::from_millis(10)));
    assert!(matches!(
        engine.close(),
        CloseOutcome::AlreadyClosing | CloseOutcome::AlreadyClosed
    ));
    assert_eq!(
        engine.start_format_size_batch(Vec::new()),
        Err(StartTaskError::Closed)
    );
    assert_eq!(engine.task_snapshot(running), Err(TaskAccessError::Closed));
    assert_eq!(engine.task_snapshot(queued), Err(TaskAccessError::Closed));
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    release_tx.send(()).unwrap();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(engine.lifecycle(), EngineLifecycle::Closed);
    assert_eq!(engine.close(), CloseOutcome::AlreadyClosed);
    let registry = shared.lock_registry_recover();
    assert_eq!(registry.records[&running].phase, TaskPhase::Cancelled);
    assert_eq!(registry.records[&queued].phase, TaskPhase::Cancelled);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    for id in [running, queued] {
        let record = &registry.records[&id];
        assert!(matches!(
            record.events.back().map(|event| &event.kind),
            Some(TaskEventKind::Terminal {
                phase: TaskPhase::Cancelled
            })
        ));
        assert_eq!(
            record
                .events
                .iter()
                .filter(|event| matches!(event.kind, TaskEventKind::Terminal { .. }))
                .count(),
            1
        );
    }
}

#[test]
fn only_last_handle_drop_cancels_running_and_queued_work() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 2, 8));
    let shared = Arc::clone(&engine.inner.shared);
    let (started_tx, started_rx) = mpsc::channel();
    let (cancelled_tx, cancelled_rx) = mpsc::channel();
    let running = engine
        .submit_test(Box::new(move |context| {
            started_tx.send(()).unwrap();
            while !context.is_cancellation_requested() {
                std::thread::yield_now();
            }
            cancelled_tx.send(()).unwrap();
            WorkOutcome::Cancelled(None)
        }))
        .unwrap();
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let queued_executions = Arc::clone(&executions);
    let queued = engine
        .submit_test(Box::new(move |_| {
            queued_executions.fetch_add(1, Ordering::SeqCst);
            WorkOutcome::Succeeded(TaskResult::TestOnly)
        }))
        .unwrap();
    let clone = engine.clone();
    drop(engine);
    assert_eq!(clone.lifecycle(), EngineLifecycle::Open);
    drop(clone);
    cancelled_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        if shared
            .registry
            .lock()
            .is_ok_and(|registry| registry.lifecycle == EngineLifecycle::Closed)
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let registry = shared.lock_registry_recover();
    assert_eq!(registry.records[&running].phase, TaskPhase::Cancelled);
    assert_eq!(registry.records[&queued].phase, TaskPhase::Cancelled);
    assert_eq!(executions.load(Ordering::SeqCst), 0);
}

#[test]
fn poisoned_registry_never_masquerades_as_closed() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    let shared = Arc::clone(&engine.inner.shared);
    let poison_shared = Arc::clone(&shared);
    let _ = std::thread::spawn(move || {
        let _guard = poison_shared.registry.lock().unwrap();
        panic!("poison registry for recovery test");
    })
    .join();

    assert_eq!(engine.lifecycle(), EngineLifecycle::Open);
    assert_eq!(engine.close(), CloseOutcome::Initiated);
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(engine.lifecycle(), EngineLifecycle::Closed);
}

#[test]
fn config_paths_do_not_depend_on_home() {
    let temp = TempDir::new().unwrap();
    let explicit = config(&temp);
    assert!(explicit.database_path().is_absolute());
    assert_ne!(explicit.database_path(), PathBuf::from("~/.dux"));
}
