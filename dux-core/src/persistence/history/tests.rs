use super::*;
use crate::domain::MAX_SCAN_ISSUES;
use crate::persistence::StoreCoordinator;
use crate::persistence::snapshot::{SnapshotFileName, SnapshotReference};
use crate::{
    CoveragePermille, DATABASE_SCHEMA_VERSION, ScanCoverageStatus, ScanIssue, ScanIssueKind,
};
use tempfile::TempDir;

fn started(id: &str, root: PathBuf, offset_ms: u64) -> NewScanRecord {
    NewScanRecord::try_new_without_root_identity(
        ScanId::new(id).unwrap(),
        root,
        UNIX_EPOCH + Duration::from_millis(1_750_000_000_000 + offset_ms),
    )
    .unwrap()
}

fn completion(id: &str, offset_ms: u64) -> ScanCompletionRecord {
    ScanCompletionRecord::try_new(
        ScanId::new(id).unwrap(),
        UNIX_EPOCH + Duration::from_millis(1_750_000_002_000 + offset_ms),
        TerminalScanStatus::Succeeded,
        ScanCounts {
            directory_count: 3,
            file_count: 7,
            logical_bytes: 11,
            allocated_bytes: Some(13),
        },
    )
    .unwrap()
}

fn snapshot_reference(id: &ScanId) -> SnapshotReference {
    let name = SnapshotFileName::from_scan_id(id.as_str().as_bytes());
    SnapshotReference::from_stored(id, 1, name.as_str(), [0x5a; 32]).unwrap()
}

fn partial_coverage(root: &Path) -> ScanCoverage {
    ScanCoverage::try_from_terminal(
        Some(CoveragePermille::new(750).unwrap()),
        vec![
            ScanIssue::try_new(
                ScanIssueKind::MetadataError,
                Some(root.join("metadata-å")),
                3,
            )
            .unwrap(),
            ScanIssue::try_new(
                ScanIssueKind::PermissionDenied,
                Some(root.join("denied")),
                2,
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

#[test]
fn start_finish_and_load_survive_process_style_reopen() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let start = started("scan:one", temp.path().join("scan-root-å"), 0);
    let finish = completion("scan:one", 0);

    {
        let store = StoreCoordinator::open(&database).unwrap();
        store.record_scan_started(&start).unwrap();
        let running = store.load_scan(start.id()).unwrap().unwrap();
        assert_eq!(running.id(), start.id());
        assert_eq!(running.status(), ScanStatus::Running);
        assert_eq!(running.root(), start.root());
        assert_eq!(running.started_at(), start.started_at());
        assert_eq!(running.completed_at(), None);
        assert_eq!(running.counts(), ScanCounts::default());
        assert_eq!(running.coverage(), &ScanCoverage::unknown());
        store.record_scan_finished(&finish).unwrap();
    }

    let reopened = StoreCoordinator::open(&database).unwrap();
    let completed = reopened.load_scan(start.id()).unwrap().unwrap();
    assert_eq!(completed.status(), ScanStatus::Succeeded);
    assert_eq!(completed.completed_at(), Some(finish.completed_at()));
    assert_eq!(completed.counts(), finish.counts());
}

#[test]
fn scan_start_persists_domain_separated_root_identity_digest() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let start = NewScanRecord::try_new_with_root_identity(
        ScanId::new("scan:root-identity").unwrap(),
        temp.path().join("root"),
        UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
        FilesystemIdentity::new(7, 11),
    )
    .unwrap();
    let expected = [
        0x7e, 0x88, 0x76, 0x62, 0xf3, 0x76, 0x7f, 0xc2, 0x2f, 0x21, 0x11, 0x0b, 0x46, 0x0c, 0x81,
        0xf7, 0x0f, 0x47, 0x05, 0x0a, 0x3a, 0xa2, 0x7e, 0x11, 0xf2, 0x89, 0xbe, 0x25, 0x8d, 0x92,
        0x52, 0x45,
    ];
    assert_eq!(start.root_identity_v1_sha256(), Some(expected));

    store.record_scan_started(&start).unwrap();

    let loaded = store.load_scan(start.id()).unwrap().unwrap();
    assert_eq!(loaded.root_identity_v1_sha256(), Some(expected));
    store.with_connection(|connection| {
        let stored: Vec<u8> = connection
            .query_row(
                "SELECT root_identity_v1_sha256 FROM scans WHERE scan_id = ?1",
                [start.id().as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, expected);
    });
}

#[test]
fn exact_root_since_query_is_bounded_newest_and_byte_exact() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let selected = temp.path().join("selected");
    let other = temp.path().join("other");
    let older = started("scan:targeted:exact-root-older", selected.clone(), 10);
    let newer = started("scan:targeted:exact-root-newer", selected.clone(), 30);
    let failed_newest = started("scan:exact-root-failed", selected.clone(), 40);
    let unrelated = started("scan:exact-root-other", other, 40);
    for scan in [&older, &newer, &failed_newest, &unrelated] {
        store.record_scan_started(scan).unwrap();
    }
    for scan in [&older, &newer] {
        store
            .record_scan_finished(
                &ScanCompletionRecord::try_succeeded_with_snapshot(
                    scan.id().clone(),
                    scan.started_at() + Duration::from_millis(1),
                    ScanCounts::default(),
                    snapshot_reference(scan.id()),
                )
                .unwrap(),
            )
            .unwrap();
    }
    store.with_connection(|connection| {
        for scan in [&older, &newer] {
            let scheduled =
                system_time_to_unix_ms(scan.started_at(), HistoryErrorKind::InvalidInput).unwrap();
            connection
                .execute(
                    "INSERT INTO candidate_evaluations (
                       scan_id, record_format_version, evaluator_revision,
                       rule_catalog_schema_version, rule_catalog_sha256,
                       context_format_version, context_sha256,
                       snapshot_version, snapshot_sha256,
                       scheduled_at_unix_ms, completed_at_unix_ms,
                       status, candidate_count, failure_kind
                     ) VALUES (
                       ?1, 1, 1, 1, zeroblob(32), 1, zeroblob(32),
                       1, zeroblob(32), ?2, ?3, 'failed', NULL, 'cancelled'
                     )",
                    params![scan.id().as_str(), scheduled, scheduled + 1],
                )
                .unwrap();
        }
    });
    store
        .record_scan_finished(
            &ScanCompletionRecord::try_new(
                failed_newest.id().clone(),
                failed_newest.started_at() + Duration::from_millis(1),
                TerminalScanStatus::Failed,
                ScanCounts::default(),
            )
            .unwrap(),
        )
        .unwrap();

    assert_eq!(
        store
            .load_latest_scan_for_exact_root_since(
                &selected,
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_020),
            )
            .unwrap()
            .unwrap()
            .id(),
        newer.id()
    );
    assert!(
        store
            .load_latest_scan_for_exact_root_since(
                &selected,
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_031),
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn committed_scan_start_reconciles_an_injected_post_commit_failure() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let start = started("scan:ambiguous-start", temp.path().join("root"), 0);

    store
        .record_scan_started_reconciled_after_commit_failure_for_test(&start)
        .unwrap();
    store.record_scan_started_reconciled(&start).unwrap();
    assert!(
        store
            .load_scan(start.id())
            .unwrap()
            .unwrap()
            .exactly_matches_start(&start)
    );
}

#[test]
fn terminal_coverage_and_issues_are_atomic_shortened_and_survive_reopen() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let start = started("scan:coverage", root.clone(), 0);
    let coverage = partial_coverage(&root);
    let finish = ScanCompletionRecord::try_new_with_coverage(
        start.id().clone(),
        UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
        TerminalScanStatus::Succeeded,
        ScanCounts {
            directory_count: 2,
            file_count: 4,
            logical_bytes: 8,
            allocated_bytes: Some(12),
        },
        coverage.clone(),
    )
    .unwrap();

    {
        let store = StoreCoordinator::open(&database).unwrap();
        store.record_scan_started(&start).unwrap();
        store.record_scan_finished_reconciled(&finish).unwrap();
        let stored = store.load_scan(start.id()).unwrap().unwrap();
        assert_eq!(stored.coverage(), &coverage);
        assert!(stored.exactly_matches_completion(&finish));
        store.with_connection(|connection| {
            let issue_count: i64 = connection
                .query_row(
                    "SELECT issue_count FROM scans WHERE scan_id = ?1",
                    [start.id().as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(issue_count, 5);
            let shortened = connection
                .prepare(
                    "SELECT shortened_path, shortened_path_encoding
                     FROM scan_issues WHERE scan_id = ?1 ORDER BY issue_id",
                )
                .unwrap()
                .query_map([start.id().as_str()], |row| {
                    Ok(EncodedBytes {
                        bytes: row.get(0)?,
                        encoding: StoredEncoding::host_path_from_stored(row.get(1)?).unwrap(),
                    })
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(
                shortened
                    .into_iter()
                    .map(|path| decode_host_path(&path).unwrap())
                    .collect::<Vec<_>>(),
                [PathBuf::from("denied"), PathBuf::from("metadata-å")]
            );
        });
    }

    let reopened = StoreCoordinator::open(&database).unwrap();
    assert_eq!(
        reopened.load_scan(start.id()).unwrap().unwrap().coverage(),
        &coverage
    );
}

#[test]
fn terminal_status_and_known_coverage_must_agree() {
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let cancelled = ScanCoverage::try_from_terminal(
        None,
        vec![ScanIssue::try_new(ScanIssueKind::Cancelled, None, 1).unwrap()],
    )
    .unwrap();
    let partial = ScanCoverage::try_from_terminal(
        None,
        vec![
            ScanIssue::try_new(
                ScanIssueKind::MetadataError,
                Some(if cfg!(windows) {
                    PathBuf::from(r"C:\scan\root\error")
                } else {
                    PathBuf::from("/scan/root/error")
                }),
                1,
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let complete = ScanCoverage::try_from_terminal(None, Vec::new()).unwrap();

    let cases = [
        (TerminalScanStatus::Succeeded, cancelled.clone()),
        (TerminalScanStatus::Cancelled, partial.clone()),
        (TerminalScanStatus::Failed, complete.clone()),
        (TerminalScanStatus::Interrupted, complete),
    ];
    for (index, (status, coverage)) in cases.into_iter().enumerate() {
        let completion = ScanCompletionRecord::try_new_with_coverage(
            ScanId::new(format!("scan:coverage-status-{index}")).unwrap(),
            completed_at,
            status,
            ScanCounts::default(),
            coverage,
        )
        .unwrap();
        assert_eq!(
            PreparedScanCompletion::prepare(&completion)
                .err()
                .unwrap()
                .kind,
            HistoryErrorKind::InvalidInput
        );
    }

    let valid_cancelled = ScanCompletionRecord::try_new_with_coverage(
        ScanId::new("scan:coverage-status-valid").unwrap(),
        completed_at,
        TerminalScanStatus::Cancelled,
        ScanCounts::default(),
        cancelled,
    )
    .unwrap();
    PreparedScanCompletion::prepare(&valid_cancelled).unwrap();
}

#[test]
fn out_of_root_issue_rolls_back_parent_and_every_child() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("scan-root");
    let start = started("scan:issue-rollback", root, 0);
    store.record_scan_started(&start).unwrap();
    let coverage = ScanCoverage::try_from_terminal(
        None,
        vec![
            ScanIssue::try_new(
                ScanIssueKind::MetadataError,
                Some(temp.path().join("outside")),
                1,
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let finish = ScanCompletionRecord::try_new_with_coverage(
        start.id().clone(),
        UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
        TerminalScanStatus::Succeeded,
        ScanCounts::default(),
        coverage,
    )
    .unwrap();

    assert_eq!(
        store.record_scan_finished(&finish).unwrap_err().kind,
        HistoryErrorKind::InvalidInput
    );
    let running = store.load_scan(start.id()).unwrap().unwrap();
    assert_eq!(running.status(), ScanStatus::Running);
    assert_eq!(running.coverage().status(), ScanCoverageStatus::Unknown);
    store.with_connection(|connection| {
        let children: i64 = connection
            .query_row(
                "SELECT count(*) FROM scan_issues WHERE scan_id = ?1",
                [start.id().as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(children, 0);
    });
}

#[test]
fn scan_ids_are_immutable_and_finish_is_compare_and_set() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store").join("dux.sqlite3")).unwrap();
    let start = started("scan:one", temp.path().join("one"), 0);
    let finish = completion("scan:one", 0);
    store.record_scan_started(&start).unwrap();
    assert_eq!(
        store.record_scan_started(&start).unwrap_err().kind,
        HistoryErrorKind::AlreadyExists
    );
    store.record_scan_finished(&finish).unwrap();
    assert_eq!(
        store.record_scan_finished(&finish).unwrap_err().kind,
        HistoryErrorKind::InvalidTransition
    );
    assert_eq!(
        store
            .record_scan_finished(&completion("scan:missing", 0))
            .unwrap_err()
            .kind,
        HistoryErrorKind::NotFound
    );
}

#[test]
fn exact_retry_adopts_only_the_same_canonical_coverage_facts() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    let start = started("scan:coverage-collision", root.clone(), 0);
    store.record_scan_started(&start).unwrap();
    let first = ScanCompletionRecord::try_new_with_coverage(
        start.id().clone(),
        UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
        TerminalScanStatus::Succeeded,
        ScanCounts::default(),
        partial_coverage(&root),
    )
    .unwrap();
    let different = ScanCompletionRecord::try_new_with_coverage(
        start.id().clone(),
        first.completed_at(),
        TerminalScanStatus::Succeeded,
        first.counts(),
        ScanCoverage::try_from_terminal(
            None,
            vec![
                ScanIssue::try_new(ScanIssueKind::TimedOut, Some(root.join("different")), 1)
                    .unwrap(),
            ],
        )
        .unwrap(),
    )
    .unwrap();

    store.record_scan_finished_reconciled(&first).unwrap();
    store.record_scan_finished_reconciled(&first).unwrap();
    assert_eq!(
        store
            .record_scan_finished_reconciled(&different)
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    assert!(
        store
            .load_scan(start.id())
            .unwrap()
            .unwrap()
            .exactly_matches_completion(&first)
    );
}

#[test]
fn snapshot_tuple_is_atomic_typed_and_exactly_reconciled() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let start = started("scan:with-snapshot", temp.path().join("root"), 0);
    let reference = snapshot_reference(start.id());
    let finish = ScanCompletionRecord::try_succeeded_with_snapshot(
        start.id().clone(),
        UNIX_EPOCH + Duration::from_nanos(1_750_000_002_000_999_999),
        ScanCounts {
            directory_count: 1,
            file_count: 2,
            logical_bytes: 3,
            allocated_bytes: Some(4),
        },
        reference.clone(),
    )
    .unwrap();

    {
        let store = StoreCoordinator::open(&database).unwrap();
        store.record_scan_started(&start).unwrap();
        store.record_scan_finished_reconciled(&finish).unwrap();
        store.record_scan_finished_reconciled(&finish).unwrap();
    }

    let reopened = StoreCoordinator::open(&database).unwrap();
    let stored = reopened.load_scan(start.id()).unwrap().unwrap();
    assert_eq!(stored.snapshot(), Some(&reference));
    assert!(stored.exactly_matches_completion(&finish));
    assert_eq!(
        finish.completed_at(),
        UNIX_EPOCH + Duration::from_millis(1_750_000_002_000)
    );
}

#[test]
fn committed_scan_completion_reconciles_an_injected_post_commit_failure() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let start = started("scan:ambiguous-commit", temp.path().join("root"), 0);
    let finish = ScanCompletionRecord::try_succeeded_with_snapshot(
        start.id().clone(),
        UNIX_EPOCH + Duration::from_nanos(1_750_000_002_000_999_999),
        ScanCounts {
            directory_count: 1,
            file_count: 1,
            logical_bytes: 2,
            allocated_bytes: Some(3),
        },
        snapshot_reference(start.id()),
    )
    .unwrap();
    store.record_scan_started(&start).unwrap();

    store
        .record_scan_finished_reconciled_after_commit_failure_for_test(&finish)
        .unwrap();

    assert!(
        store
            .load_scan(start.id())
            .unwrap()
            .unwrap()
            .exactly_matches_completion(&finish)
    );
}

#[cfg(unix)]
#[test]
fn exact_post_commit_match_cannot_mask_unsafe_retained_storage() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let marker = database.with_extension("sqlite3.writer.lock");
    let store = StoreCoordinator::open(&database).unwrap();
    let root = temp.path().join("root");
    let start = started("scan:unsafe-post-commit", root.clone(), 0);
    let finish = ScanCompletionRecord::try_new_with_coverage(
        start.id().clone(),
        UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
        TerminalScanStatus::Succeeded,
        ScanCounts::default(),
        partial_coverage(&root),
    )
    .unwrap();
    store.record_scan_started(&start).unwrap();

    let error = store
        .record_scan_finished_reconciled_with_after_commit_hook_for_test(&finish, || {
            std::fs::write(&marker, b"NOT-A-DUX-MARKER")
                .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))
        })
        .unwrap_err();
    assert_eq!(error.kind, HistoryErrorKind::OutcomeUnknown);
    store.with_connection(|connection| {
        let durable: (String, i64) = connection
            .query_row(
                "SELECT status, issue_count FROM scans WHERE scan_id = ?1",
                [start.id().as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(durable, ("succeeded".to_owned(), 5));
    });
}

#[test]
fn scan_start_time_is_canonical_before_persistence_and_ordering() {
    let temp = TempDir::new().unwrap();
    let input = UNIX_EPOCH + Duration::from_nanos(1_750_000_000_000_999_999);
    let start = NewScanRecord::try_new_without_root_identity(
        ScanId::new("scan:canonical-start").unwrap(),
        temp.path().join("root"),
        input,
    )
    .unwrap();
    assert_eq!(
        start.started_at(),
        UNIX_EPOCH + Duration::from_millis(1_750_000_000_000)
    );

    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    store.record_scan_started(&start).unwrap();
    assert_eq!(
        store.load_scan(start.id()).unwrap().unwrap().started_at(),
        start.started_at()
    );
}

#[test]
fn malformed_or_non_success_snapshot_tuples_fail_as_corrupt() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();

    let partial = started("scan:snapshot-partial", temp.path().join("partial"), 0);
    store.record_scan_started(&partial).unwrap();
    store.with_connection(|connection| {
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE scans SET snapshot_version = 1 WHERE scan_id = ?1",
                [partial.id().as_str()],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });
    assert_eq!(
        store.load_scan(partial.id()).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );

    let failed = started("scan:snapshot-failed", temp.path().join("failed"), 1);
    store.record_scan_started(&failed).unwrap();
    let reference = snapshot_reference(failed.id());
    let finish = ScanCompletionRecord::try_succeeded_with_snapshot(
        failed.id().clone(),
        UNIX_EPOCH + Duration::from_millis(1_750_000_002_001),
        ScanCounts::default(),
        reference,
    )
    .unwrap();
    store.record_scan_finished(&finish).unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE scans SET status = 'failed' WHERE scan_id = ?1",
                [failed.id().as_str()],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_scan(failed.id()).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn validation_rejects_relative_paths_pre_epoch_times_and_large_counts() {
    let temp = TempDir::new().unwrap();
    let id = ScanId::new("scan:invalid").unwrap();
    assert_eq!(
        NewScanRecord::try_new_without_root_identity(
            id.clone(),
            PathBuf::from("relative"),
            UNIX_EPOCH
        )
        .unwrap_err()
        .kind,
        HistoryErrorKind::InvalidInput
    );
    assert_eq!(
        NewScanRecord::try_new_without_root_identity(
            id.clone(),
            temp.path().to_path_buf(),
            UNIX_EPOCH - Duration::from_millis(1),
        )
        .unwrap_err()
        .kind,
        HistoryErrorKind::InvalidInput
    );
    assert_eq!(
        ScanCompletionRecord::try_new(
            id,
            UNIX_EPOCH,
            TerminalScanStatus::Failed,
            ScanCounts {
                logical_bytes: u64::MAX,
                ..ScanCounts::default()
            },
        )
        .unwrap_err()
        .kind,
        HistoryErrorKind::InvalidInput
    );
}

#[test]
fn completion_before_start_rolls_back_without_rewriting_running_record() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store").join("dux.sqlite3")).unwrap();
    let start = started("scan:one", temp.path().join("one"), 10);
    store.record_scan_started(&start).unwrap();
    let too_early = ScanCompletionRecord::try_new(
        start.id.clone(),
        UNIX_EPOCH + Duration::from_millis(1_750_000_000_009),
        TerminalScanStatus::Cancelled,
        ScanCounts::default(),
    )
    .unwrap();
    assert_eq!(
        store.record_scan_finished(&too_early).unwrap_err().kind,
        HistoryErrorKind::InvalidInput
    );
    assert_eq!(
        store.load_scan(start.id()).unwrap().unwrap().status(),
        ScanStatus::Running
    );
}

#[test]
fn every_terminal_status_is_persisted_without_rewrite_authority() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store").join("dux.sqlite3")).unwrap();
    for (index, terminal) in [
        TerminalScanStatus::Succeeded,
        TerminalScanStatus::Failed,
        TerminalScanStatus::Cancelled,
        TerminalScanStatus::Interrupted,
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("scan:terminal:{index}");
        let start = started(&id, temp.path().join(format!("root-{index}")), index as u64);
        store.record_scan_started(&start).unwrap();
        let finish = ScanCompletionRecord::try_new(
            start.id.clone(),
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_000 + index as u64),
            terminal,
            ScanCounts::default(),
        )
        .unwrap();
        store.record_scan_finished(&finish).unwrap();
        assert!(
            store
                .load_scan(start.id())
                .unwrap()
                .unwrap()
                .status()
                .is_terminal()
        );
    }
}

#[test]
fn malformed_lifecycle_row_fails_as_corrupt_observation() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store").join("dux.sqlite3")).unwrap();
    let start = started("scan:malformed", temp.path().join("root"), 0);
    store.record_scan_started(&start).unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                [start.id().as_str()],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE scans SET status = 'succeeded' WHERE scan_id = ?1",
                [start.id().as_str()],
            )
            .unwrap();
    });
    assert_eq!(
        store.load_scan(start.id()).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
    assert_eq!(
        store
            .load_scan(&ScanId::new("scan:unrelated").unwrap())
            .unwrap(),
        None,
        "the progress handler must be removed after a decode failure"
    );
}

#[test]
fn nonterminal_scan_cannot_claim_measured_coverage() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let start = started("scan:running-coverage", temp.path().join("root"), 0);
    store.record_scan_started(&start).unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE scans
                 SET coverage_status = 'complete', coverage_permille = 1000
                 WHERE scan_id = ?1",
                [start.id().as_str()],
            )
            .unwrap();
    });

    assert_eq!(
        store.load_scan(start.id()).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn hostile_issue_kind_count_and_shortened_path_fail_closed() {
    for corruption in ["kind", "count", "path", "message"] {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        let start = started(&format!("scan:hostile:{corruption}"), root.clone(), 0);
        let finish = ScanCompletionRecord::try_new_with_coverage(
            start.id().clone(),
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
            TerminalScanStatus::Succeeded,
            ScanCounts::default(),
            partial_coverage(&root),
        )
        .unwrap();
        store.record_scan_started(&start).unwrap();
        store.record_scan_finished(&finish).unwrap();
        store.with_connection(|connection| match corruption {
            "kind" => {
                connection
                    .execute(
                        "UPDATE scan_issues SET issue_kind = 'future_issue'
                         WHERE issue_id = (
                             SELECT min(issue_id) FROM scan_issues WHERE scan_id = ?1
                         )",
                        [start.id().as_str()],
                    )
                    .unwrap();
            }
            "count" => {
                connection
                    .execute(
                        "UPDATE scans SET issue_count = issue_count + 1 WHERE scan_id = ?1",
                        [start.id().as_str()],
                    )
                    .unwrap();
            }
            "path" => {
                let encoded = encode_host_path(&root).unwrap();
                connection
                    .execute(
                        "UPDATE scan_issues
                         SET shortened_path = ?2, shortened_path_encoding = ?3
                         WHERE issue_id = (
                             SELECT min(issue_id) FROM scan_issues WHERE scan_id = ?1
                         )",
                        params![start.id().as_str(), encoded.bytes, encoded.encoding as i64],
                    )
                    .unwrap();
            }
            "message" => {
                connection
                    .execute(
                        "UPDATE scan_issues SET message_key = 'scan.issue.wrong'
                         WHERE issue_id = (
                             SELECT min(issue_id) FROM scan_issues WHERE scan_id = ?1
                         )",
                        [start.id().as_str()],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        });
        assert_eq!(
            store.load_scan(start.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData,
            "accepted {corruption} corruption"
        );
        assert_eq!(
            store
                .load_scan(&ScanId::new(format!("scan:clean:{corruption}")).unwrap())
                .unwrap(),
            None,
            "query progress handler leaked after {corruption} decode"
        );
    }
}

#[test]
fn hostile_issue_rows_over_the_domain_bound_are_rejected_before_materialization() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
    let root = temp.path().join("root");
    let start = started("scan:too-many-issues", root, 0);
    store.record_scan_started(&start).unwrap();
    store
        .record_scan_finished(&completion(start.id().as_str(), 0))
        .unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE scans
                 SET coverage_status = 'partial', coverage_permille = NULL,
                     issue_count = ?2
                 WHERE scan_id = ?1",
                params![
                    start.id().as_str(),
                    i64::try_from(MAX_SCAN_ISSUES + 1).unwrap()
                ],
            )
            .unwrap();
        let mut statement = connection
            .prepare(
                "INSERT INTO scan_issues (
                    scan_id, issue_kind, occurrence_count, message_key
                 ) VALUES (?1, 'cancelled', 1, 'scan.issue.cancelled')",
            )
            .unwrap();
        for _ in 0..=MAX_SCAN_ISSUES {
            statement.execute([start.id().as_str()]).unwrap();
        }
    });
    assert_eq!(
        store.load_scan(start.id()).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn oversized_blob_is_rejected_before_path_materialization() {
    let temp = TempDir::new().unwrap();
    let store = StoreCoordinator::open(&temp.path().join("store").join("dux.sqlite3")).unwrap();
    let start = started("scan:oversized", temp.path().join("root"), 0);
    store.record_scan_started(&start).unwrap();
    store.with_connection(|connection| {
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE scans SET root_path = zeroblob(65537) WHERE scan_id = ?1",
                [start.id().as_str()],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });
    assert_eq!(
        store.load_scan(start.id()).unwrap_err().kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn external_schema_upgrade_blocks_history_writes_before_transaction() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store").join("dux.sqlite3");
    let store = StoreCoordinator::open(&database).unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO schema_migrations
                 (version, name, checksum_sha256, applied_at_unix_ms)
                 VALUES (?1, 'future-schema', zeroblob(32), 2)",
                [i64::from(DATABASE_SCHEMA_VERSION + 1)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION + 1)
            .unwrap();
    });

    let scan = started("scan:blocked", temp.path().join("root"), 0);
    let error = store.record_scan_started(&scan).unwrap_err();
    assert_eq!(error.kind, HistoryErrorKind::IncompatibleSchema);
    assert!(!error.to_string().contains(database.to_str().unwrap()));
    store.with_connection(|connection| {
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM scans WHERE scan_id = ?1",
                [scan.id().as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    });
}
