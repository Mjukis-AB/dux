use std::ffi::OsStr;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use rusqlite::params;
use tempfile::TempDir;

use super::*;
use crate::domain::{
    BlockReason, Candidate, CandidateAction, CandidateCategory, CandidateId, CandidateInput,
    Evidence, LocalizedTextKey, ProvenanceUrl, Rule, RuleDefinition, RuleGuards, RuleId,
    RuleMatcher, RuleMatcherDefinition, RuleRef, RuleRevision, RuleScope, SafetyTier,
};
use crate::persistence::candidate_history::{NewCandidateRecord, StoredCandidateRecord};
use crate::persistence::history::{NewScanRecord, TerminalScanStatus};
use crate::{CoveragePermille, ScanIssue, ScanIssueKind};

fn document(scan_id: &str, root: &Path) -> SnapshotDocument {
    SnapshotDocument {
        metadata: SnapshotMetadata {
            scan_id: ScanId::new(scan_id).unwrap(),
            root: HostValue::from_root(root).unwrap(),
            captured_at: SnapshotTimestamp::new(1_750_000_000, 123).unwrap(),
            totals: SnapshotTotals {
                directory_count: 1,
                file_count: 1,
                logical_bytes: 10,
                allocated_bytes: Some(16),
            },
        },
        nodes: vec![
            SnapshotNode {
                id: 0,
                parent: None,
                depth: 0,
                kind: SnapshotNodeKind::Directory,
                name: None,
                logical_bytes: 10,
                allocated_bytes: Some(16),
                file_count: 1,
                child_count: 1,
                modified_at: None,
                accessed_at: None,
                scan_flags: SnapshotScanFlags::NONE,
                unix_identity: if cfg!(unix) {
                    Some(SnapshotUnixIdentity::new(7, 10))
                } else {
                    None
                },
            },
            SnapshotNode {
                id: 1,
                parent: Some(0),
                depth: 1,
                kind: SnapshotNodeKind::File,
                name: Some(HostValue::from_component(OsStr::new("artifact.o")).unwrap()),
                logical_bytes: 10,
                allocated_bytes: Some(16),
                file_count: 1,
                child_count: 0,
                modified_at: None,
                accessed_at: None,
                scan_flags: SnapshotScanFlags::NONE,
                unix_identity: if cfg!(unix) {
                    Some(SnapshotUnixIdentity::new(7, 11))
                } else {
                    None
                },
            },
        ],
    }
}

fn counts() -> ScanCounts {
    ScanCounts {
        directory_count: 1,
        file_count: 1,
        logical_bytes: 10,
        allocated_bytes: Some(16),
    }
}

fn create_unleased_temp(
    repository: &SnapshotRepository,
    seed: &[u8],
    contents: &[u8],
) -> (String, StagedSnapshot) {
    let store = repository.store.as_ref().unwrap();
    let mut reservation = store
        .reserve_stage(
            SnapshotFileName::from_scan_id(seed),
            PUBLICATION_LOCK_TIMEOUT,
        )
        .unwrap();
    let name = reservation.temp_name().to_owned();
    let mut staged = reservation.create().unwrap();
    std::io::Write::write_all(&mut staged, contents).unwrap();
    staged.sync_all().unwrap();
    drop(reservation);
    (name, staged)
}

fn evaluation_identity(seed: u8) -> CandidateEvaluationIdentity {
    CandidateEvaluationIdentity::try_new(1, 1, [seed; 32], 1, [seed.wrapping_add(1); 32]).unwrap()
}

fn candidate(scan_id: &ScanId, id: &str, path: &Path) -> NewCandidateRecord {
    let rule = Rule::try_new(RuleDefinition {
        reference: RuleRef::new(
            RuleId::new("fixture.snapshot-evaluation").unwrap(),
            RuleRevision::new(1).unwrap(),
        ),
        title_key: LocalizedTextKey::new("fixture.snapshot-evaluation.title").unwrap(),
        category: CandidateCategory::DeveloperArtifact,
        scope: RuleScope::SelectedScanRoot,
        matcher: RuleMatcher::try_new(RuleMatcherDefinition {
            path_component: Some("artifact.o".to_owned()),
            required_ancestor_markers_any: Vec::new(),
            required_markers_all: Vec::new(),
            forbidden_markers_any: Vec::new(),
            exact_bundle_identifiers: Vec::new(),
            excluded_descendants: Vec::new(),
            protected_descendants: Vec::new(),
        })
        .unwrap(),
        guards: RuleGuards::try_new(None, 0, Vec::new(), false).unwrap(),
        safety: SafetyTier::Informational,
        action: CandidateAction::RevealOnly,
        schedule_eligible: false,
        explanation_key: LocalizedTextKey::new("fixture.snapshot-evaluation.explanation").unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/dux-fixture").unwrap()],
    })
    .unwrap();
    let candidate = Candidate::try_from_rule(
        &rule,
        CandidateInput::new(
            CandidateId::new(id).unwrap(),
            vec![path.to_path_buf()],
            10,
            None,
            vec![Evidence::MatchedPath {
                path: path.to_path_buf(),
            }],
            vec![BlockReason::ProtectedPath],
            scan_id.clone(),
        ),
    )
    .unwrap();
    NewCandidateRecord::try_from_candidate(
        &candidate,
        UNIX_EPOCH + Duration::from_millis(1_750_000_002_500),
    )
    .unwrap()
}

fn complete_coverage() -> ScanCoverage {
    ScanCoverage::try_from_terminal(None, Vec::new()).unwrap()
}

fn partial_coverage(root: &Path) -> ScanCoverage {
    ScanCoverage::try_from_terminal(
        Some(CoveragePermille::new(700).unwrap()),
        vec![
            ScanIssue::try_new(
                ScanIssueKind::MetadataError,
                Some(root.join("artifact.o")),
                2,
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

fn counts_for(document: &SnapshotDocument) -> ScanCounts {
    ScanCounts {
        directory_count: document.metadata.totals.directory_count,
        file_count: document.metadata.totals.file_count,
        logical_bytes: document.metadata.totals.logical_bytes,
        allocated_bytes: document.metadata.totals.allocated_bytes,
    }
}

fn open_repository(database: &Path) -> (Arc<StoreCoordinator>, SnapshotRepository) {
    let store = StoreCoordinator::open(database).unwrap();
    let repository =
        SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadWrite).unwrap();
    (store, repository)
}

fn complete_snapshot(
    store: &StoreCoordinator,
    repository: &SnapshotRepository,
    document: &SnapshotDocument,
    root: &Path,
    completed_at: SystemTime,
) -> SnapshotReference {
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root.to_path_buf(),
                completed_at - Duration::from_secs(2),
            )
            .unwrap(),
        )
        .unwrap();
    repository
        .complete_scan(
            completed_at,
            counts_for(document),
            &complete_coverage(),
            document,
        )
        .unwrap()
}

fn review_pin_count(store: &StoreCoordinator) -> i64 {
    store.with_connection(|connection| {
        connection
            .query_row("SELECT count(*) FROM snapshot_review_pins", [], |row| {
                row.get(0)
            })
            .unwrap()
    })
}

fn temp_lease_count(store: &StoreCoordinator) -> i64 {
    store.with_connection(|connection| {
        connection
            .query_row("SELECT count(*) FROM snapshot_temp_leases", [], |row| {
                row.get(0)
            })
            .unwrap()
    })
}

fn record_running_scan(
    store: &StoreCoordinator,
    document: &SnapshotDocument,
    root: &Path,
    started_at: SystemTime,
) {
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root.to_path_buf(),
                started_at,
            )
            .unwrap(),
        )
        .unwrap();
}

fn terminalize_scan(
    store: &StoreCoordinator,
    document: &SnapshotDocument,
    completed_at: SystemTime,
    status: TerminalScanStatus,
) {
    store
        .record_scan_finished(
            &ScanCompletionRecord::try_new(
                document.metadata.scan_id.clone(),
                completed_at,
                status,
                counts(),
            )
            .unwrap(),
        )
        .unwrap();
}

fn retention_tombstone_count(store: &StoreCoordinator) -> i64 {
    store.with_connection(|connection| {
        connection
            .query_row(
                "SELECT count(*) FROM snapshot_retention_tombstones",
                [],
                |row| row.get(0),
            )
            .unwrap()
    })
}

fn scan_row_count(store: &StoreCoordinator) -> i64 {
    store.with_connection(|connection| {
        connection
            .query_row("SELECT count(*) FROM scans", [], |row| row.get(0))
            .unwrap()
    })
}

fn publish_running_orphan(
    store: &StoreCoordinator,
    repository: &SnapshotRepository,
    document: &SnapshotDocument,
    root: &Path,
    started_at: SystemTime,
) -> SnapshotFileName {
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root.to_path_buf(),
                started_at,
            )
            .unwrap(),
        )
        .unwrap();
    drop(repository.publish_orphan_for_test(document).unwrap());
    SnapshotFileName::from_scan_id(document.metadata.scan_id.as_str().as_bytes())
}

fn encoded_document(document: &SnapshotDocument) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    encode_snapshot(document, &mut output).unwrap();
    output.into_inner()
}

#[test]
fn retention_removal_accounting_fails_closed_on_underflow() {
    assert_eq!(charged_bytes_after_removal(10, 4).unwrap(), 6);
    assert_eq!(
        charged_bytes_after_removal(4, 10).unwrap_err().kind,
        SnapshotRepositoryErrorKind::Storage(SnapshotStorageErrorKind::InternalState)
    );
}

#[test]
fn physical_orphan_reconciliation_is_sorted_bounded_and_ignores_active_temps() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_010_123);
    let (store, repository) = open_repository(&database);
    let first = document("scan:orphan-bounded-first", &root);
    let second = document("scan:orphan-bounded-second", &root);
    let first_name = publish_running_orphan(
        &store,
        &repository,
        &first,
        &root,
        observed_at - Duration::from_secs(3),
    );
    let second_name = publish_running_orphan(
        &store,
        &repository,
        &second,
        &root,
        observed_at - Duration::from_secs(2),
    );

    let temp_document = document("scan:orphan-bounded-active-temp", &root);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                temp_document.metadata.scan_id.clone(),
                root.clone(),
                observed_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    let (staged, _, _) = repository.stage_document(&temp_document).unwrap();
    let before_rows = (
        scan_row_count(&store),
        temp_lease_count(&store),
        retention_tombstone_count(&store),
    );

    let mut expected = [
        (first_name.clone(), first.metadata.scan_id.clone()),
        (second_name.clone(), second.metadata.scan_id.clone()),
    ];
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    let result = repository
        .reconcile_physical_orphan(observed_at + Duration::from_nanos(999_999))
        .unwrap();
    assert_eq!(result.observed_at, observed_at);
    assert_eq!(result.orphan_count_before, 2);
    assert_eq!(result.orphan_count_after, 1);
    assert!(result.has_more);
    let SnapshotOrphanReconciliationBatchOutcome::Removed { scan_id, bytes } = result.outcome
    else {
        panic!("expected one orphan removal")
    };
    assert_eq!(scan_id, expected[0].1);
    assert!(bytes > 0);
    assert_eq!(
        result.orphan_charged_bytes_before - bytes,
        result.orphan_charged_bytes_after
    );
    assert_eq!(
        before_rows,
        (
            scan_row_count(&store),
            temp_lease_count(&store),
            retention_tombstone_count(&store),
        )
    );

    let second_result = repository
        .reconcile_physical_orphan(observed_at + Duration::from_millis(1))
        .unwrap();
    assert_eq!(second_result.orphan_count_before, 1);
    assert_eq!(second_result.orphan_count_after, 0);
    assert!(!second_result.has_more);
    assert!(matches!(
        second_result.outcome,
        SnapshotOrphanReconciliationBatchOutcome::Removed { .. }
    ));
    let settled = repository
        .reconcile_physical_orphan(observed_at + Duration::from_millis(2))
        .unwrap();
    assert_eq!(
        settled.outcome,
        SnapshotOrphanReconciliationBatchOutcome::NoOrphan
    );
    assert_eq!(settled.orphan_count_before, 0);
    assert_eq!(settled.orphan_charged_bytes_before, 0);
    staged.abandon();
}

#[test]
fn physical_orphan_reconciliation_accepts_terminal_non_success_without_mutating_history() {
    for (label, status) in [
        ("failed", TerminalScanStatus::Failed),
        ("cancelled", TerminalScanStatus::Cancelled),
        ("interrupted", TerminalScanStatus::Interrupted),
    ] {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_020_000);
        let (store, repository) = open_repository(&database);
        let document = document(&format!("scan:orphan-terminal-{label}"), &root);
        publish_running_orphan(
            &store,
            &repository,
            &document,
            &root,
            observed_at - Duration::from_secs(2),
        );
        store
            .record_scan_finished(
                &ScanCompletionRecord::try_new(
                    document.metadata.scan_id.clone(),
                    observed_at - Duration::from_secs(1),
                    status,
                    counts(),
                )
                .unwrap(),
            )
            .unwrap();
        let before = store
            .load_scan(&document.metadata.scan_id)
            .unwrap()
            .unwrap();

        let result = repository.reconcile_physical_orphan(observed_at).unwrap();
        assert!(matches!(
            result.outcome,
            SnapshotOrphanReconciliationBatchOutcome::Removed { ref scan_id, .. }
                if scan_id == &document.metadata.scan_id
        ));
        assert_eq!(
            store
                .load_scan(&document.metadata.scan_id)
                .unwrap()
                .unwrap(),
            before
        );
        assert_eq!(temp_lease_count(&store), 1);
        assert_eq!(retention_tombstone_count(&store), 0);
    }
}

#[test]
fn physical_orphan_reconciliation_rejects_missing_queued_succeeded_and_wrong_root_parents() {
    for case in ["missing", "queued", "succeeded", "wrong-root"] {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let stored_root = if case == "wrong-root" {
            temp.path().join("different-root")
        } else {
            root.clone()
        };
        let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_030_000);
        let (store, repository) = open_repository(&database);
        let document = document(&format!("scan:orphan-invalid-{case}"), &root);
        let file_name = publish_running_orphan(
            &store,
            &repository,
            &document,
            &stored_root,
            observed_at - Duration::from_secs(2),
        );
        store.with_connection(|connection| match case {
            "missing" => {
                connection
                    .execute(
                        "DELETE FROM snapshot_temp_leases WHERE scan_id = ?1",
                        [document.metadata.scan_id.as_str()],
                    )
                    .unwrap();
                connection
                    .execute(
                        "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                        [document.metadata.scan_id.as_str()],
                    )
                    .unwrap();
                connection
                    .execute(
                        "DELETE FROM scans WHERE scan_id = ?1",
                        [document.metadata.scan_id.as_str()],
                    )
                    .unwrap();
            }
            "queued" => {
                connection
                    .execute(
                        "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                        [document.metadata.scan_id.as_str()],
                    )
                    .unwrap();
                connection
                    .execute(
                        "UPDATE scans SET status = 'queued' WHERE scan_id = ?1",
                        [document.metadata.scan_id.as_str()],
                    )
                    .unwrap();
            }
            "succeeded" => {
                connection
                    .execute(
                        "DELETE FROM snapshot_temp_leases WHERE scan_id = ?1",
                        [document.metadata.scan_id.as_str()],
                    )
                    .unwrap();
                connection
                    .execute(
                        "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                        [document.metadata.scan_id.as_str()],
                    )
                    .unwrap();
                connection
                    .execute(
                        "UPDATE scans SET status = 'succeeded', completed_at_unix_ms = ?2
                          WHERE scan_id = ?1",
                        rusqlite::params![
                            document.metadata.scan_id.as_str(),
                            1_750_000_029_000_i64
                        ],
                    )
                    .unwrap();
            }
            "wrong-root" => {}
            _ => unreachable!(),
        });

        assert_eq!(
            repository
                .reconcile_physical_orphan(observed_at)
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData),
            "case {case}"
        );
        assert!(
            database
                .parent()
                .unwrap()
                .join("snapshots")
                .join(file_name.as_str())
                .is_file()
        );
    }
}

#[test]
fn physical_orphan_reconciliation_rejects_corrupt_and_wrong_name_bodies() {
    for case in ["corrupt", "wrong-name"] {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_040_000);
        let (store, repository) = open_repository(&database);
        let orphan_document = document(&format!("scan:orphan-invalid-body-{case}"), &root);
        let file_name = publish_running_orphan(
            &store,
            &repository,
            &orphan_document,
            &root,
            observed_at - Duration::from_secs(1),
        );
        let mut bytes = if case == "corrupt" {
            encoded_document(&orphan_document)
        } else {
            encoded_document(&document("scan:orphan-different-body-name", &root))
        };
        if case == "corrupt" {
            *bytes.last_mut().unwrap() ^= 0x01;
        }
        repository
            .store
            .as_ref()
            .unwrap()
            .replace_final_for_test(&file_name, &bytes)
            .unwrap();

        let error = repository
            .reconcile_physical_orphan(observed_at)
            .unwrap_err();
        if case == "corrupt" {
            assert!(matches!(
                error.kind,
                SnapshotRepositoryErrorKind::Codec(SnapshotCodecErrorKind::ChecksumMismatch)
            ));
        } else {
            assert_eq!(error.kind, SnapshotRepositoryErrorKind::ReferenceMismatch);
        }
        assert!(
            database
                .parent()
                .unwrap()
                .join("snapshots")
                .join(file_name.as_str())
                .is_file(),
            "case {case} must leave the final untouched"
        );
    }
}

#[test]
fn physical_orphan_reconciliation_never_adopts_a_referenced_final() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_045_000);
    let (store, repository) = open_repository(&database);
    let document = document("scan:orphan-reference-control", &root);
    let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);

    let result = repository
        .reconcile_physical_orphan(completed_at + Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        result.outcome,
        SnapshotOrphanReconciliationBatchOutcome::NoOrphan
    );
    assert_eq!(result.orphan_count_before, 0);
    assert!(
        database
            .parent()
            .unwrap()
            .join("snapshots")
            .join(reference.file_name().as_str())
            .is_file()
    );
}

#[test]
fn physical_orphan_reconciliation_rejects_an_invalid_clock() {
    let invalid_clock = UNIX_EPOCH - Duration::from_millis(1);
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let (_store, repository) = open_repository(&database);
    assert_eq!(
        repository
            .reconcile_physical_orphan(invalid_clock)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
    );
}

#[test]
fn physical_orphan_reconciliation_maps_post_unlink_uncertainty_to_history() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_050_000);
    let (store, repository) = open_repository(&database);
    let document = document("scan:orphan-outcome-unknown", &root);
    publish_running_orphan(
        &store,
        &repository,
        &document,
        &root,
        observed_at - Duration::from_secs(1),
    );

    assert_eq!(
        repository
            .reconcile_physical_orphan_with_remover(observed_at, |_, _| {
                Err(SnapshotFinalRemovalError::OutcomeUnknown)
            })
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::OutcomeUnknown)
    );
}

fn install_future_schema(database: &Path) {
    let future = super::super::status::DATABASE_SCHEMA_VERSION + 1;
    let external = rusqlite::Connection::open(database).unwrap();
    external
        .execute(
            "INSERT INTO schema_migrations (
                 version, name, checksum_sha256, applied_at_unix_ms
             ) VALUES (?1, 'test-future-temp-lease-schema', zeroblob(32), 1)",
            [future],
        )
        .unwrap();
    external
        .pragma_update(None, "user_version", future)
        .unwrap();
}

fn restore_current_schema(database: &Path) {
    let current = super::super::status::DATABASE_SCHEMA_VERSION;
    let future = current + 1;
    let external = rusqlite::Connection::open(database).unwrap();
    external
        .execute("DELETE FROM schema_migrations WHERE version = ?1", [future])
        .unwrap();
    external
        .pragma_update(None, "user_version", current)
        .unwrap();
}

#[cfg(unix)]
fn create_provisioning_stage(database: &Path, suffix: &str, marker_complete: bool) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let stage = database
        .parent()
        .unwrap()
        .join(format!(".dux-snapshot-stage-{suffix}"));
    fs::create_dir(&stage).unwrap();
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).unwrap();
    let marker = stage.join(".dux-snapshot-store");
    fs::write(&marker, b"DUXSNAPSTOREV1\0\0").unwrap();
    fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();
    if marker_complete {
        let writer = stage.join(".dux-snapshot.writer.lock");
        fs::write(&writer, b"DUXSNAPWRITER1\0\0").unwrap();
        fs::set_permissions(&writer, fs::Permissions::from_mode(0o600)).unwrap();
    }
    stage
}

#[test]
fn provisioning_stage_reconciliation_canonicalizes_time_and_is_sql_read_only() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let (_store, repository) = open_repository(&database);
    let observer = rusqlite::Connection::open(&database).unwrap();
    let data_version_before = observer
        .pragma_query_value(None, "data_version", |row| row.get::<_, i64>(0))
        .unwrap();
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_060_123);

    let result = repository
        .reconcile_snapshot_provisioning_stage(observed_at + Duration::from_nanos(999_999))
        .unwrap();

    assert_eq!(result.observed_at, observed_at);
    assert_eq!(
        result.outcome,
        SnapshotProvisioningStageReconciliationBatchOutcome::NoStage
    );
    assert_eq!(result.total_stage_count_before, 0);
    assert_eq!(result.total_stage_count_after, 0);
    assert_eq!(result.marker_owned_count_before, 0);
    assert_eq!(result.marker_owned_count_after, 0);
    assert_eq!(result.unproven_count_before, 0);
    assert_eq!(result.unproven_count_after, 0);
    assert_eq!(result.control_charged_bytes_before, 0);
    assert_eq!(result.control_charged_bytes_after, 0);
    assert!(!result.has_more);
    assert_eq!(
        observer
            .pragma_query_value(None, "data_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        data_version_before
    );
}

#[cfg(unix)]
#[test]
fn provisioning_stage_reconciliation_removes_one_proven_stage_per_batch() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let (_store, repository) = open_repository(&database);
    let marker_only =
        create_provisioning_stage(&database, "00000000000000000000000000000000", false);
    let marker_complete =
        create_provisioning_stage(&database, "11111111111111111111111111111111", true);
    let unproven = database
        .parent()
        .unwrap()
        .join(".dux-snapshot-stage-ffffffffffffffffffffffffffffffff");
    fs::create_dir(&unproven).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&unproven, fs::Permissions::from_mode(0o700)).unwrap();
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_060_200);

    let first = repository
        .reconcile_snapshot_provisioning_stage(observed_at)
        .unwrap();
    assert!(matches!(
        first.outcome,
        SnapshotProvisioningStageReconciliationBatchOutcome::RemovedMarkerOnly { bytes }
            if bytes > 0
    ));
    assert_eq!(first.total_stage_count_before, 3);
    assert_eq!(first.total_stage_count_after, 2);
    assert_eq!(first.marker_owned_count_before, 2);
    assert_eq!(first.marker_owned_count_after, 1);
    assert_eq!(first.unproven_count_before, 1);
    assert_eq!(first.unproven_count_after, 1);
    assert!(first.control_charged_bytes_before > first.control_charged_bytes_after);
    assert!(first.has_more);
    assert!(!marker_only.exists());
    assert!(marker_complete.is_dir());

    let second = repository
        .reconcile_snapshot_provisioning_stage(observed_at + Duration::from_millis(1))
        .unwrap();
    assert!(matches!(
        second.outcome,
        SnapshotProvisioningStageReconciliationBatchOutcome::RemovedMarkerComplete { bytes }
            if bytes > 0
    ));
    assert_eq!(second.total_stage_count_before, 2);
    assert_eq!(second.total_stage_count_after, 1);
    assert_eq!(second.marker_owned_count_before, 1);
    assert_eq!(second.marker_owned_count_after, 0);
    assert_eq!(
        second.control_charged_bytes_before,
        first.control_charged_bytes_after
    );
    assert_eq!(second.control_charged_bytes_after, 0);
    assert!(!second.has_more);
    assert!(!marker_complete.exists());
    assert!(unproven.is_dir());

    let deferred = repository
        .reconcile_snapshot_provisioning_stage(observed_at + Duration::from_millis(2))
        .unwrap();
    assert_eq!(
        deferred.outcome,
        SnapshotProvisioningStageReconciliationBatchOutcome::DeferredUnproven
    );
    assert_eq!(deferred.total_stage_count_before, 1);
    assert_eq!(deferred.total_stage_count_after, 1);
    assert_eq!(deferred.marker_owned_count_before, 0);
    assert_eq!(deferred.marker_owned_count_after, 0);
    assert_eq!(deferred.unproven_count_before, 1);
    assert_eq!(deferred.unproven_count_after, 1);
    assert!(!deferred.has_more);
    assert!(unproven.is_dir());
}

#[test]
fn provisioning_stage_reconciliation_rejects_invalid_clock_and_read_only_access() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let (store, repository) = open_repository(&database);
    assert_eq!(
        repository
            .reconcile_snapshot_provisioning_stage(UNIX_EPOCH - Duration::from_millis(1))
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
    );
    drop(repository);
    let read_only =
        SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadOnly).unwrap();
    assert_eq!(
        read_only
            .reconcile_snapshot_provisioning_stage(UNIX_EPOCH)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::ReadOnly
    );
}

#[test]
fn provisioning_stage_reconciliation_revalidates_schema_and_maps_uncertain_effects() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let (_store, repository) = open_repository(&database);
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_060_300);

    let skew = repository
        .reconcile_snapshot_provisioning_stage_with_reconciler(observed_at, |store| {
            install_future_schema(&database);
            store.reconcile_provisioning_stage()
        })
        .unwrap_err();
    assert_eq!(
        skew.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::IncompatibleSchema)
    );
    restore_current_schema(&database);

    let uncertain = repository
        .reconcile_snapshot_provisioning_stage_with_reconciler(observed_at, |_| {
            Err(SnapshotProvisioningStageRemovalError::OutcomeUnknown)
        })
        .unwrap_err();
    assert_eq!(
        uncertain.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::OutcomeUnknown)
    );
}

#[test]
fn terminal_temp_row_only_reconciles_every_allowed_status_without_changing_parent() {
    for (label, status) in [
        ("failed", TerminalScanStatus::Failed),
        ("cancelled", TerminalScanStatus::Cancelled),
        ("interrupted", TerminalScanStatus::Interrupted),
    ] {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let base = UNIX_EPOCH + Duration::from_millis(1_750_000_030_000);
        let document = document(&format!("scan:terminal-temp-row-only:{label}"), &root);
        let (store, repository) = open_repository(&database);
        record_running_scan(&store, &document, &root, base);
        repository
            .leave_snapshot_temp_residual_for_test(&document, false)
            .unwrap();
        terminalize_scan(&store, &document, base + Duration::from_secs(1), status);
        let parent_before = store
            .load_scan(&document.metadata.scan_id)
            .unwrap()
            .unwrap();

        let result = repository
            .reconcile_terminal_snapshot_temp_residual(
                base + Duration::from_secs(2) + Duration::from_micros(999),
            )
            .unwrap();
        assert_eq!(
            result.observed_at,
            base + Duration::from_secs(2),
            "batch time must be millisecond canonical"
        );
        assert!(matches!(
            result.outcome,
            SnapshotTerminalTempReconciliationBatchOutcome::ReconciledRowOnly {
                ref scan_id
            } if scan_id == &document.metadata.scan_id
        ));
        assert_eq!(result.terminal_lease_count_before, 1);
        assert_eq!(result.terminal_lease_count_after, 0);
        assert_eq!(result.active_terminal_lease_count_before, 0);
        assert_eq!(result.active_terminal_lease_count_after, 0);
        assert_eq!(result.terminal_charged_bytes_before, 0);
        assert_eq!(result.terminal_charged_bytes_after, 0);
        assert!(!result.has_more);
        assert_eq!(temp_lease_count(&store), 0);
        assert_eq!(
            store
                .load_scan(&document.metadata.scan_id)
                .unwrap()
                .unwrap(),
            parent_before
        );
        assert_eq!(retention_tombstone_count(&store), 0);
    }
}

#[test]
fn prior_boot_scan_recovery_preserves_exact_temp_lease_for_terminal_reconciliation() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_031_000);
    let document = document("scan:recovered-temp-debt", &root);
    let (store, repository) = open_repository(&database);
    let owner = crate::persistence::process_liveness::ProcessInstanceId::from_stored(&format!(
        "1:l:2a:1234:{}:{}",
        "11".repeat(32),
        "22".repeat(16)
    ))
    .unwrap();
    let provenance = crate::persistence::process_liveness::ExecutionProvenance::from_stored(
        &owner,
        &[0x33; 32],
        &[0x11; 32],
    )
    .unwrap();
    store.set_scan_process_identity_for_test(
        crate::persistence::process_liveness::ProcessExecutionIdentity {
            owner,
            provenance: Some(provenance),
        },
    );
    record_running_scan(&store, &document, &root, base);
    repository
        .leave_snapshot_temp_residual_for_test(&document, false)
        .unwrap();
    assert_eq!(temp_lease_count(&store), 1);
    let lease_before: (String, i64, String, String, String, String, String, i64) = store
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT lease_id, record_format_version, scan_id, scan_status,
                            final_relative_name, temp_relative_name,
                            owner_process_instance, created_at_unix_ms
                     FROM snapshot_temp_leases WHERE scan_id = ?1",
                    [document.metadata.scan_id.as_str()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                        ))
                    },
                )
                .unwrap()
        });
    store.with_connection(|connection| {
        let (owner, claimed_at, host, mut boot): (String, i64, Vec<u8>, Vec<u8>) = connection
            .query_row(
                "SELECT owner_process_instance, claimed_at_unix_ms,
                        execution_host_identity_v1_sha256,
                        execution_boot_scope_v1_sha256
                 FROM scan_process_claims WHERE scan_id = ?1",
                [document.metadata.scan_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        boot[0] ^= 0xff;
        let scope = boot
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut components = owner.split(':').map(str::to_owned).collect::<Vec<_>>();
        components[4] = scope.clone();
        let prior_owner = components.join(":");
        crate::persistence::process_liveness::ProcessInstanceId::from_stored(&prior_owner).unwrap();
        connection
            .execute(
                "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                [document.metadata.scan_id.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO scan_process_claims (
                     scan_id, record_format_version, owner_process_instance,
                     recovery_scope, claimed_at_unix_ms,
                     execution_host_identity_v1_sha256,
                     execution_boot_scope_v1_sha256,
                     execution_recovery_policy
                 ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, 'interrupt_only')",
                params![
                    document.metadata.scan_id.as_str(),
                    prior_owner,
                    format!("l:{scope}"),
                    claimed_at,
                    host,
                    boot,
                ],
            )
            .unwrap();
    });

    let recovered = store
        .run_scan_recovery_batch_with_hooks_for_test(
            base + Duration::from_secs(1),
            |_| panic!("prior-boot interruption must not probe a PID"),
            || Ok(()),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        recovered.outcome,
        crate::persistence::ScanRecoveryBatchOutcome::Interrupted
    );
    assert_eq!(temp_lease_count(&store), 1);
    let lease_after: (String, i64, String, String, String, String, String, i64) = store
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT lease_id, record_format_version, scan_id, scan_status,
                            final_relative_name, temp_relative_name,
                            owner_process_instance, created_at_unix_ms
                     FROM snapshot_temp_leases WHERE scan_id = ?1",
                    [document.metadata.scan_id.as_str()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                        ))
                    },
                )
                .unwrap()
        });
    assert_eq!(lease_after, lease_before);
    assert_eq!(
        store
            .load_scan(&document.metadata.scan_id)
            .unwrap()
            .unwrap()
            .status(),
        ScanStatus::Interrupted
    );

    repository
        .reconcile_terminal_snapshot_temp_residual(base + Duration::from_secs(2))
        .unwrap();
    assert_eq!(temp_lease_count(&store), 0);
}

#[test]
fn terminal_temp_quiescent_removal_is_one_item_and_exactly_accounted() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_031_000);
    let document = document("scan:terminal-temp-quiescent", &root);
    let (store, repository) = open_repository(&database);
    record_running_scan(&store, &document, &root, base);
    repository
        .leave_snapshot_temp_residual_for_test(&document, true)
        .unwrap();
    terminalize_scan(
        &store,
        &document,
        base + Duration::from_secs(1),
        TerminalScanStatus::Failed,
    );
    let before = repository.inspect_retention_inventory(base).unwrap();
    let expected_bytes = before.totals.temporary_quiescent.charged_bytes;
    assert!(expected_bytes > 0);

    let result = repository
        .reconcile_terminal_snapshot_temp_residual(base + Duration::from_secs(2))
        .unwrap();
    assert!(matches!(
        result.outcome,
        SnapshotTerminalTempReconciliationBatchOutcome::RemovedTempAndLease {
            ref scan_id,
            bytes
        } if scan_id == &document.metadata.scan_id && bytes == expected_bytes
    ));
    assert_eq!(result.terminal_lease_count_before, 1);
    assert_eq!(result.terminal_lease_count_after, 0);
    assert_eq!(result.terminal_charged_bytes_before, expected_bytes);
    assert_eq!(result.terminal_charged_bytes_after, 0);
    assert_eq!(temp_lease_count(&store), 0);
    let after = repository.inspect_retention_inventory(base).unwrap();
    assert!(after.temporary_files.is_empty());
    assert!(after.residual_temp_leases.is_empty());
}

#[test]
fn active_terminal_temp_defers_but_does_not_starve_later_row_only_debt() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_032_000);
    let active_document = document("scan:terminal-temp-active-first", &root);
    let row_only_document = document("scan:terminal-temp-row-only-second", &root);
    let (store, repository) = open_repository(&database);

    record_running_scan(&store, &active_document, &root, base);
    let (active_stage, _, _) = repository.stage_document(&active_document).unwrap();
    terminalize_scan(
        &store,
        &active_document,
        base + Duration::from_secs(1),
        TerminalScanStatus::Interrupted,
    );
    record_running_scan(
        &store,
        &row_only_document,
        &root,
        base + Duration::from_secs(2),
    );
    repository
        .leave_snapshot_temp_residual_for_test(&row_only_document, false)
        .unwrap();
    terminalize_scan(
        &store,
        &row_only_document,
        base + Duration::from_secs(3),
        TerminalScanStatus::Cancelled,
    );

    let first = repository
        .reconcile_terminal_snapshot_temp_residual(base + Duration::from_secs(4))
        .unwrap();
    assert!(matches!(
        first.outcome,
        SnapshotTerminalTempReconciliationBatchOutcome::ReconciledRowOnly {
            ref scan_id
        } if scan_id == &row_only_document.metadata.scan_id
    ));
    assert_eq!(first.terminal_lease_count_before, 2);
    assert_eq!(first.terminal_lease_count_after, 1);
    assert_eq!(first.active_terminal_lease_count_before, 1);
    assert_eq!(first.active_terminal_lease_count_after, 1);
    assert!(first.has_more);

    let deferred = repository
        .reconcile_terminal_snapshot_temp_residual(base + Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        deferred.outcome,
        SnapshotTerminalTempReconciliationBatchOutcome::DeferredActive
    );
    assert_eq!(deferred.terminal_lease_count_before, 1);
    assert_eq!(deferred.active_terminal_lease_count_before, 1);
    assert!(deferred.has_more);

    active_stage.abandon();
    let settled = repository
        .reconcile_terminal_snapshot_temp_residual(base + Duration::from_secs(6))
        .unwrap();
    assert!(matches!(
        settled.outcome,
        SnapshotTerminalTempReconciliationBatchOutcome::RemovedTempAndLease {
            ref scan_id,
            ..
        } if scan_id == &active_document.metadata.scan_id
    ));
    assert_eq!(temp_lease_count(&store), 0);
}

#[test]
fn terminal_temp_selection_is_oldest_created_then_one_item_per_batch() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_032_500);
    let older = document("scan:terminal-temp-z-older", &root);
    let newer = document("scan:terminal-temp-a-newer", &root);
    let (store, repository) = open_repository(&database);

    for (document, created_at) in [
        (&older, base + Duration::from_millis(1)),
        (&newer, base + Duration::from_millis(2)),
    ] {
        record_running_scan(&store, document, &root, base);
        let mut guard = store.lock_current_history_connection().unwrap();
        let (reservation, lease) = reserve_prepared_temp_lease(
            &repository,
            &guard,
            &document.metadata.scan_id,
            created_at,
        );
        repository
            .insert_temp_lease_with_guard(&mut guard, &lease)
            .unwrap();
        drop(reservation);
        drop(guard);
        terminalize_scan(
            &store,
            document,
            base + Duration::from_secs(1),
            TerminalScanStatus::Failed,
        );
    }

    let first = repository
        .reconcile_terminal_snapshot_temp_residual(base + Duration::from_secs(2))
        .unwrap();
    assert!(matches!(
        first.outcome,
        SnapshotTerminalTempReconciliationBatchOutcome::ReconciledRowOnly {
            ref scan_id
        } if scan_id == &older.metadata.scan_id
    ));
    assert_eq!(first.terminal_lease_count_before, 2);
    assert_eq!(first.terminal_lease_count_after, 1);
    assert!(first.has_more);
    assert_eq!(temp_lease_count(&store), 1);

    let second = repository
        .reconcile_terminal_snapshot_temp_residual(base + Duration::from_secs(3))
        .unwrap();
    assert!(matches!(
        second.outcome,
        SnapshotTerminalTempReconciliationBatchOutcome::ReconciledRowOnly {
            ref scan_id
        } if scan_id == &newer.metadata.scan_id
    ));
    assert_eq!(second.terminal_lease_count_before, 1);
    assert_eq!(second.terminal_lease_count_after, 0);
    assert!(!second.has_more);
}

#[test]
fn running_rows_and_unleased_temps_are_outside_terminal_maintenance() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_033_000);
    let running = document("scan:terminal-temp-running", &root);
    let unleased_document = document("scan:terminal-temp-unleased", &root);
    let (store, repository) = open_repository(&database);
    let stage_sibling = temp
        .path()
        .join(".dux-snapshot-stage-11111111111111111111111111111111");
    fs::create_dir(&stage_sibling).unwrap();
    fs::write(stage_sibling.join("sentinel"), b"unowned-stage-debt").unwrap();
    record_running_scan(&store, &running, &root, base);
    repository
        .leave_snapshot_temp_residual_for_test(&running, true)
        .unwrap();

    let unleased_name =
        SnapshotFileName::from_scan_id(unleased_document.metadata.scan_id.as_str().as_bytes());
    let mut reservation = repository
        .store
        .as_ref()
        .unwrap()
        .reserve_stage(unleased_name, PUBLICATION_LOCK_TIMEOUT)
        .unwrap();
    reservation.create().unwrap().abandon();
    drop(reservation);

    let result = repository
        .reconcile_terminal_snapshot_temp_residual(base + Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        result.outcome,
        SnapshotTerminalTempReconciliationBatchOutcome::NoTerminalResidual
    );
    assert_eq!(result.terminal_lease_count_before, 0);
    assert_eq!(result.terminal_charged_bytes_before, 0);
    assert!(!result.has_more);
    assert_eq!(temp_lease_count(&store), 1);
    let inventory = repository.inspect_retention_inventory(base).unwrap();
    assert_eq!(inventory.temporary_files.len(), 2);
    assert!(inventory.temporary_files.iter().any(|temporary| matches!(
        temporary.state,
        super::super::snapshot_retention_inventory::SnapshotTemporaryState::QuiescentAtObservation
    )));
    assert!(inventory.temporary_files.iter().any(|temporary| matches!(
        temporary.state,
        super::super::snapshot_retention_inventory::SnapshotTemporaryState::Unleased
    )));
    assert!(inventory.residual_temp_leases.is_empty());
    assert_eq!(
        fs::read(stage_sibling.join("sentinel")).unwrap(),
        b"unowned-stage-debt"
    );
}

#[test]
fn unleased_temp_reconciliation_is_lexical_one_at_a_time_and_exactly_accounted() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let (_store, repository) = open_repository(&database);
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_033_100);
    let (first_name, first_stage) =
        create_unleased_temp(&repository, b"unleased-first", b"first unleased bytes");
    first_stage.abandon();
    let (second_name, second_stage) = create_unleased_temp(
        &repository,
        b"unleased-second",
        b"second and longer unleased bytes",
    );
    second_stage.abandon();
    let expected_first_name = first_name.min(second_name);
    let selected_name = std::cell::RefCell::new(None);

    let first = repository
        .reconcile_unleased_snapshot_temp_with_remover(observed_at, |storage, name| {
            selected_name.replace(Some(name.to_owned()));
            storage.remove_observed_unleased_temp_reconciled(name)
        })
        .unwrap();
    assert_eq!(selected_name.into_inner().unwrap(), expected_first_name);
    assert!(matches!(
        first.outcome,
        SnapshotUnleasedTempReconciliationBatchOutcome::Removed { bytes } if bytes > 0
    ));
    assert_eq!(first.unleased_temp_count_before, 2);
    assert_eq!(first.unleased_temp_count_after, 1);
    assert_eq!(first.active_unleased_temp_count_before, 0);
    assert_eq!(first.active_unleased_temp_count_after, 0);
    assert!(first.unleased_charged_bytes_before > first.unleased_charged_bytes_after);
    assert!(first.has_more);

    let second = repository
        .reconcile_unleased_snapshot_temp(observed_at + Duration::from_millis(1))
        .unwrap();
    assert!(matches!(
        second.outcome,
        SnapshotUnleasedTempReconciliationBatchOutcome::Removed { bytes } if bytes > 0
    ));
    assert_eq!(second.unleased_temp_count_before, 1);
    assert_eq!(second.unleased_temp_count_after, 0);
    assert_eq!(
        second.unleased_charged_bytes_before,
        first.unleased_charged_bytes_after
    );
    assert_eq!(second.unleased_charged_bytes_after, 0);
    assert!(!second.has_more);

    let empty = repository
        .reconcile_unleased_snapshot_temp(observed_at + Duration::from_millis(2))
        .unwrap();
    assert_eq!(
        empty.outcome,
        SnapshotUnleasedTempReconciliationBatchOutcome::NoUnleasedTemp
    );
    assert_eq!(empty.unleased_temp_count_before, 0);
    assert!(!empty.has_more);
}

#[test]
fn active_unleased_temp_does_not_starve_quiescent_debt_and_then_defers() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let (_store, repository) = open_repository(&database);
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_033_200);
    let (_active_name, active_stage) =
        create_unleased_temp(&repository, b"unleased-active", b"active bytes");
    let (_quiescent_name, quiescent_stage) =
        create_unleased_temp(&repository, b"unleased-quiescent", b"quiescent bytes");
    quiescent_stage.abandon();

    let first = repository
        .reconcile_unleased_snapshot_temp(observed_at)
        .unwrap();
    assert!(matches!(
        first.outcome,
        SnapshotUnleasedTempReconciliationBatchOutcome::Removed { .. }
    ));
    assert_eq!(first.unleased_temp_count_before, 2);
    assert_eq!(first.unleased_temp_count_after, 1);
    assert_eq!(first.active_unleased_temp_count_before, 1);
    assert_eq!(first.active_unleased_temp_count_after, 1);
    assert!(first.has_more);

    let deferred = repository
        .reconcile_unleased_snapshot_temp(observed_at + Duration::from_millis(1))
        .unwrap();
    assert_eq!(
        deferred.outcome,
        SnapshotUnleasedTempReconciliationBatchOutcome::DeferredActive
    );
    assert_eq!(deferred.unleased_temp_count_before, 1);
    assert_eq!(deferred.active_unleased_temp_count_before, 1);
    assert!(deferred.has_more);

    active_stage.abandon();
    let settled = repository
        .reconcile_unleased_snapshot_temp(observed_at + Duration::from_millis(2))
        .unwrap();
    assert!(matches!(
        settled.outcome,
        SnapshotUnleasedTempReconciliationBatchOutcome::Removed { .. }
    ));
    assert_eq!(settled.unleased_temp_count_after, 0);
    assert!(!settled.has_more);
}

#[test]
fn unleased_temp_reconciliation_never_consumes_row_bound_or_stage_debt() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_033_300);
    let row_bound = document("scan:unleased-row-bound", &root);
    let (store, repository) = open_repository(&database);
    record_running_scan(&store, &row_bound, &root, observed_at);
    repository
        .leave_snapshot_temp_residual_for_test(&row_bound, true)
        .unwrap();
    let stage_sibling = temp
        .path()
        .join(".dux-snapshot-stage-11111111111111111111111111111111");
    fs::create_dir(&stage_sibling).unwrap();
    fs::write(stage_sibling.join("sentinel"), b"stage debt").unwrap();
    let (_name, unleased_stage) =
        create_unleased_temp(&repository, b"unleased-with-row", b"unleased bytes");
    unleased_stage.abandon();

    let result = repository
        .reconcile_unleased_snapshot_temp(observed_at + Duration::from_millis(1))
        .unwrap();
    assert!(matches!(
        result.outcome,
        SnapshotUnleasedTempReconciliationBatchOutcome::Removed { .. }
    ));
    assert_eq!(result.unleased_temp_count_before, 1);
    assert_eq!(temp_lease_count(&store), 1);
    assert_eq!(
        store
            .load_scan(&row_bound.metadata.scan_id)
            .unwrap()
            .unwrap()
            .status(),
        ScanStatus::Running
    );
    let inventory = repository.inspect_retention_inventory(observed_at).unwrap();
    assert!(matches!(
        inventory.temporary_files.as_slice(),
        [temporary]
            if temporary.state
                == super::super::snapshot_retention_inventory::SnapshotTemporaryState::QuiescentAtObservation
    ));
    assert_eq!(
        fs::read(stage_sibling.join("sentinel")).unwrap(),
        b"stage debt"
    );
}

#[test]
fn unleased_temp_effect_uncertainty_and_invalid_clock_fail_closed() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let (_store, repository) = open_repository(&database);
    assert_eq!(
        repository
            .reconcile_unleased_snapshot_temp(UNIX_EPOCH - Duration::from_millis(1))
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
    );

    let (_name, stage) = create_unleased_temp(&repository, b"unleased-unknown", b"unknown bytes");
    stage.abandon();
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_033_400);
    assert_eq!(
        repository
            .reconcile_unleased_snapshot_temp_with_remover(observed_at, |_, _| {
                Err(SnapshotTempRemovalError::OutcomeUnknown)
            })
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::OutcomeUnknown)
    );

    let retry = repository
        .reconcile_unleased_snapshot_temp(observed_at + Duration::from_millis(1))
        .unwrap();
    assert!(matches!(
        retry.outcome,
        SnapshotUnleasedTempReconciliationBatchOutcome::Removed { .. }
    ));
}

#[test]
fn terminal_temp_requires_a_fully_valid_selected_parent() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_034_000);
    let document = document("scan:terminal-temp-corrupt-parent", &root);
    let (store, repository) = open_repository(&database);
    record_running_scan(&store, &document, &root, base);
    repository
        .leave_snapshot_temp_residual_for_test(&document, false)
        .unwrap();
    terminalize_scan(
        &store,
        &document,
        base + Duration::from_secs(1),
        TerminalScanStatus::Failed,
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE scans SET root_path = x'72656c6174697665', root_path_encoding = 1
                 WHERE scan_id = ?1",
                [document.metadata.scan_id.as_str()],
            )
            .unwrap();
    });

    let error = repository
        .reconcile_terminal_snapshot_temp_residual(base + Duration::from_secs(2))
        .unwrap_err();
    assert_eq!(
        error.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );
    assert_eq!(temp_lease_count(&store), 1);
}

#[test]
fn terminal_temp_effect_and_commit_uncertainty_fail_closed() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_035_000);
    let document = document("scan:terminal-temp-uncertain-remove", &root);
    let (store, repository) = open_repository(&database);
    record_running_scan(&store, &document, &root, base);
    repository
        .leave_snapshot_temp_residual_for_test(&document, true)
        .unwrap();
    terminalize_scan(
        &store,
        &document,
        base + Duration::from_secs(1),
        TerminalScanStatus::Interrupted,
    );
    let error = repository
        .reconcile_terminal_snapshot_temp_residual_with_hooks(
            base + Duration::from_secs(2),
            |_, _| Err(SnapshotTempRemovalError::OutcomeUnknown),
            || Ok(()),
        )
        .unwrap_err();
    assert_eq!(
        error.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::OutcomeUnknown)
    );
    assert_eq!(temp_lease_count(&store), 1);

    let invalid_clock = repository
        .reconcile_terminal_snapshot_temp_residual(UNIX_EPOCH - Duration::from_millis(1))
        .unwrap_err();
    assert_eq!(
        invalid_clock.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
    );
}

#[test]
fn terminal_temp_exactly_reconciles_commit_failure_but_not_schema_uncertainty() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_036_000);
    let row_only = document("scan:terminal-temp-exact-commit", &root);
    let (store, repository) = open_repository(&database);
    record_running_scan(&store, &row_only, &root, base);
    repository
        .leave_snapshot_temp_residual_for_test(&row_only, false)
        .unwrap();
    terminalize_scan(
        &store,
        &row_only,
        base + Duration::from_secs(1),
        TerminalScanStatus::Failed,
    );
    let reconciled = repository
        .reconcile_terminal_snapshot_temp_residual_with_hooks(
            base + Duration::from_secs(2),
            |storage, name| storage.remove_observed_quiescent_temp_reconciled(name),
            || Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable)),
        )
        .unwrap();
    assert!(matches!(
        reconciled.outcome,
        SnapshotTerminalTempReconciliationBatchOutcome::ReconciledRowOnly { .. }
    ));
    assert_eq!(temp_lease_count(&store), 0);

    let physical = document("scan:terminal-temp-schema-uncertain", &root);
    record_running_scan(&store, &physical, &root, base + Duration::from_secs(3));
    repository
        .leave_snapshot_temp_residual_for_test(&physical, true)
        .unwrap();
    terminalize_scan(
        &store,
        &physical,
        base + Duration::from_secs(4),
        TerminalScanStatus::Cancelled,
    );
    let error = repository
        .reconcile_terminal_snapshot_temp_residual_with_hooks(
            base + Duration::from_secs(5),
            |storage, name| storage.remove_observed_quiescent_temp_reconciled(name),
            || {
                install_future_schema(&database);
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(
        error.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::OutcomeUnknown)
    );
    let external = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        external
            .query_row("SELECT count(*) FROM snapshot_temp_leases", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    drop(external);
    restore_current_schema(&database);
}

fn reserve_prepared_temp_lease(
    repository: &SnapshotRepository,
    database_guard: &HistoryConnectionGuard<'_>,
    scan_id: &ScanId,
    created_at: SystemTime,
) -> (SnapshotStageReservation, PreparedSnapshotTempLease) {
    repository
        .database
        .validate_history_guard(database_guard)
        .unwrap();
    let file_name = SnapshotFileName::from_scan_id(scan_id.as_str().as_bytes());
    let reservation = repository
        .store
        .as_ref()
        .unwrap()
        .reserve_stage(file_name.clone(), PUBLICATION_LOCK_TIMEOUT)
        .unwrap();
    let lease = PreparedSnapshotTempLease::prepare(
        SnapshotTempLeaseId::random().unwrap(),
        scan_id.clone(),
        file_name,
        reservation.temp_name().to_owned(),
        current_process_instance().unwrap(),
        created_at,
    )
    .unwrap();
    (reservation, lease)
}

#[test]
fn temp_lease_insert_does_not_adopt_exact_row_after_newer_schema_wins() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:temp-insert-schema-race", &root);
    let created_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                created_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    let mut guard = store.lock_current_history_connection().unwrap();
    let (reservation, lease) =
        reserve_prepared_temp_lease(&repository, &guard, &document.metadata.scan_id, created_at);

    let error = repository
        .insert_temp_lease_with_guard_and_hook(&mut guard, &lease, || {
            install_future_schema(&database);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(
        error.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::OutcomeUnknown)
    );
    drop(reservation);
    drop(guard);
    let external = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        snapshot_temp_lease_state(&external, &lease).unwrap(),
        SnapshotTempLeaseState::Exact
    );
    drop(external);
    restore_current_schema(&database);
}

#[test]
fn temp_lease_delete_does_not_adopt_absence_after_newer_schema_wins() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:temp-delete-schema-race", &root);
    let created_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                created_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    let mut guard = store.lock_current_history_connection().unwrap();
    let (reservation, lease) =
        reserve_prepared_temp_lease(&repository, &guard, &document.metadata.scan_id, created_at);
    repository
        .insert_temp_lease_with_guard(&mut guard, &lease)
        .unwrap();

    let error = repository
        .delete_temp_lease_with_guard_and_hook(&mut guard, &lease, || {
            install_future_schema(&database);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(
        error.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::OutcomeUnknown)
    );
    drop(reservation);
    drop(guard);
    let external = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        snapshot_temp_lease_state(&external, &lease).unwrap(),
        SnapshotTempLeaseState::Missing
    );
    drop(external);
    restore_current_schema(&database);
}

#[test]
fn durable_insert_failures_leave_exact_retryable_row_and_file_states() {
    for (suffix, create_file) in [("row-only", false), ("row-and-file", true)] {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document(&format!("scan:temp-create-failure:{suffix}"), &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new_without_root_identity(
                    document.metadata.scan_id.clone(),
                    root,
                    completed_at - Duration::from_secs(1),
                )
                .unwrap(),
            )
            .unwrap();

        let error = repository
            .stage_document_with_create(&document, |reservation| {
                if create_file {
                    let mut staged = reservation.create().map_err(map_storage)?;
                    std::io::Write::write_all(&mut staged, b"durable-stage-debt").map_err(
                        |_| {
                            repository_error(SnapshotRepositoryErrorKind::Storage(
                                SnapshotStorageErrorKind::Unavailable,
                            ))
                        },
                    )?;
                    staged.sync_all().map_err(map_storage)?;
                    staged.abandon();
                }
                Err(repository_error(SnapshotRepositoryErrorKind::Storage(
                    SnapshotStorageErrorKind::Unavailable,
                )))
            })
            .err()
            .unwrap();
        assert_eq!(
            error.kind,
            SnapshotRepositoryErrorKind::Storage(SnapshotStorageErrorKind::Unavailable)
        );
        assert_eq!(temp_lease_count(&store), 1);
        let debt = repository
            .inspect_retention_inventory(completed_at)
            .unwrap();
        if create_file {
            assert_eq!(debt.temporary_files.len(), 1);
            assert_eq!(
                debt.temporary_files[0].state,
                super::super::snapshot_retention_inventory::SnapshotTemporaryState::QuiescentAtObservation
            );
            assert!(debt.residual_temp_leases.is_empty());
            assert!(debt.totals.temporary_quiescent.charged_bytes > 0);
        } else {
            assert!(debt.temporary_files.is_empty());
            assert_eq!(debt.residual_temp_leases.len(), 1);
            assert_eq!(
                debt.residual_temp_leases[0].scan_id,
                document.metadata.scan_id
            );
        }

        repository
            .complete_scan(completed_at, counts(), &complete_coverage(), &document)
            .unwrap();
        assert_eq!(temp_lease_count(&store), 0);
        let clean = repository
            .inspect_retention_inventory(completed_at)
            .unwrap();
        assert!(clean.temporary_files.is_empty());
        assert!(clean.residual_temp_leases.is_empty());
    }
}

#[test]
fn missing_live_stage_row_rolls_back_only_the_retained_current_temp() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:temp-live-row-missing", &root);
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                observed_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    let (staged, _, lease) = repository.stage_document(&document).unwrap();
    let mut guard = store.lock_current_history_connection().unwrap();
    let transaction = guard
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    delete_snapshot_temp_lease(&transaction, &lease).unwrap();
    transaction.commit().unwrap();

    let error = repository
        .abort_staged_with_guard(&mut guard, staged, &lease)
        .unwrap_err();
    assert_eq!(
        error.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );
    assert_eq!(
        snapshot_temp_lease_state(&guard.connection, &lease).unwrap(),
        SnapshotTempLeaseState::Missing
    );
    drop(guard);
    let inventory = repository.inspect_retention_inventory(observed_at).unwrap();
    assert!(inventory.temporary_files.is_empty());
    assert!(inventory.residual_temp_leases.is_empty());
}

#[test]
fn conflicting_live_stage_row_is_untouched_while_current_temp_is_rolled_back() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:temp-live-row-conflict", &root);
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                observed_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    let (staged, _, lease) = repository.stage_document(&document).unwrap();
    let mut guard = store.lock_current_history_connection().unwrap();
    let (reservation, conflicting) = reserve_prepared_temp_lease(
        &repository,
        &guard,
        &document.metadata.scan_id,
        observed_at + Duration::from_millis(1),
    );
    let transaction = guard
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    delete_snapshot_temp_lease(&transaction, &lease).unwrap();
    insert_snapshot_temp_lease(&transaction, &conflicting).unwrap();
    transaction.commit().unwrap();
    drop(reservation);
    assert_eq!(
        snapshot_temp_lease_state(&guard.connection, &lease).unwrap(),
        SnapshotTempLeaseState::Conflicting
    );

    let error = repository
        .abort_staged_with_guard(&mut guard, staged, &lease)
        .unwrap_err();
    assert_eq!(
        error.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );
    assert_eq!(
        snapshot_temp_lease_state(&guard.connection, &conflicting).unwrap(),
        SnapshotTempLeaseState::Exact
    );
    drop(guard);
    let inventory = repository.inspect_retention_inventory(observed_at).unwrap();
    assert!(inventory.temporary_files.is_empty());
    assert_eq!(inventory.residual_temp_leases.len(), 1);
    assert_eq!(
        inventory.residual_temp_leases[0].scan_id,
        document.metadata.scan_id
    );
}

#[test]
fn codec_rejection_aborts_exact_temp_and_durable_lease() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let mut invalid = document("scan:temp-codec-abort", &root);
    invalid.nodes[1].id = 0;
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                invalid.metadata.scan_id.clone(),
                root,
                completed_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();

    let error = repository
        .complete_scan(completed_at, counts(), &complete_coverage(), &invalid)
        .unwrap_err();
    assert!(matches!(error.kind, SnapshotRepositoryErrorKind::Codec(_)));
    assert_eq!(temp_lease_count(&store), 0);
    assert_eq!(
        store
            .load_scan(&invalid.metadata.scan_id)
            .unwrap()
            .unwrap()
            .status(),
        ScanStatus::Running
    );
    let inventory = repository
        .inspect_retention_inventory(completed_at)
        .unwrap();
    assert!(inventory.temporary_files.is_empty());
    assert!(inventory.residual_temp_leases.is_empty());
    assert!(inventory.orphan_finals.is_empty());
}

#[test]
#[allow(clippy::disallowed_methods)]
fn cross_process_crash_preserves_row_and_transitions_temp_to_quiescent() {
    const ROLE: &str = "DUX_SNAPSHOT_DURABLE_TEMP_CHILD";
    const DATABASE: &str = "DUX_SNAPSHOT_DURABLE_TEMP_DATABASE";
    const ROOT: &str = "DUX_SNAPSHOT_DURABLE_TEMP_ROOT";
    const READY: &str = "DUX_SNAPSHOT_DURABLE_TEMP_READY";
    const SCAN_ID: &str = "scan:durable-temp-process-crash";

    if std::env::var_os(ROLE).is_some() {
        let database = PathBuf::from(std::env::var_os(DATABASE).unwrap());
        let root = PathBuf::from(std::env::var_os(ROOT).unwrap());
        let ready = PathBuf::from(std::env::var_os(READY).unwrap());
        let document = document(SCAN_ID, &root);
        let (_store, repository) = open_repository(&database);
        let (mut staged, _, _) = repository.stage_document(&document).unwrap();
        staged.sync_all().unwrap();
        fs::write(ready, b"ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("parent did not terminate durable-temp helper");
    }

    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let ready = temp.path().join("durable-temp-ready");
    let document = document(SCAN_ID, &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root.clone(),
                completed_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();

    // DUX-DESTRUCTIVE: allow=test-snapshot-durable-temp-helper-spawn -- relaunch only this exact unit test against its TempDir-owned database to prove durable row and kernel-lock behavior across abrupt process death
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(
            "persistence::snapshot::tests::cross_process_crash_preserves_row_and_transitions_temp_to_quiescent",
        )
        .arg("--nocapture")
        .env(ROLE, "1")
        .env(DATABASE, &database)
        .env(ROOT, &root)
        .env(READY, &ready)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "child did not publish readiness");
        std::thread::sleep(Duration::from_millis(5));
    }

    let active = repository
        .inspect_retention_inventory(completed_at)
        .unwrap();
    assert_eq!(temp_lease_count(&store), 1);
    assert_eq!(active.temporary_files.len(), 1);
    assert_eq!(
        active.temporary_files[0].state,
        super::super::snapshot_retention_inventory::SnapshotTemporaryState::Active
    );
    drop(active);

    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let quiescent = repository
        .inspect_retention_inventory(completed_at)
        .unwrap();
    assert_eq!(temp_lease_count(&store), 1);
    assert_eq!(quiescent.temporary_files.len(), 1);
    assert_eq!(
        quiescent.temporary_files[0].state,
        super::super::snapshot_retention_inventory::SnapshotTemporaryState::QuiescentAtObservation
    );

    repository
        .complete_scan(completed_at, counts(), &complete_coverage(), &document)
        .unwrap();
    assert_eq!(temp_lease_count(&store), 0);
    let clean = repository
        .inspect_retention_inventory(completed_at)
        .unwrap();
    assert!(clean.temporary_files.is_empty());
    assert!(clean.residual_temp_leases.is_empty());
}

#[test]
fn active_stage_blocks_retry_then_quiescent_debt_is_reconciled_exactly() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:temp-lease-retry", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                completed_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();

    let (staged, _, _) = repository.stage_document(&document).unwrap();
    assert_eq!(temp_lease_count(&store), 1);
    let active = repository
        .inspect_retention_inventory(completed_at)
        .unwrap();
    assert_eq!(active.temporary_files.len(), 1);
    assert_eq!(
        active.temporary_files[0].state,
        super::super::snapshot_retention_inventory::SnapshotTemporaryState::Active
    );
    assert!(active.accounting_unstable);
    assert_eq!(
        repository
            .complete_scan(completed_at, counts(), &complete_coverage(), &document)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::Storage(SnapshotStorageErrorKind::Busy)
    );
    staged.abandon();

    let quiescent = repository
        .inspect_retention_inventory(completed_at)
        .unwrap();
    assert_eq!(
        quiescent.temporary_files[0].state,
        super::super::snapshot_retention_inventory::SnapshotTemporaryState::QuiescentAtObservation
    );
    assert!(!quiescent.accounting_unstable);

    repository
        .complete_scan(completed_at, counts(), &complete_coverage(), &document)
        .unwrap();
    assert_eq!(temp_lease_count(&store), 0);
    let clean = repository
        .inspect_retention_inventory(completed_at)
        .unwrap();
    assert!(clean.temporary_files.is_empty());
    assert!(clean.residual_temp_leases.is_empty());
}

#[test]
fn row_before_file_crash_debt_is_visible_and_retryable() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:temp-row-only-retry", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                completed_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    let mut guard = store.lock_current_history_connection().unwrap();
    let reservation = repository
        .store
        .as_ref()
        .unwrap()
        .reserve_stage(
            SnapshotFileName::from_scan_id(document.metadata.scan_id.as_str().as_bytes()),
            PUBLICATION_LOCK_TIMEOUT,
        )
        .unwrap();
    let lease = PreparedSnapshotTempLease::prepare(
        SnapshotTempLeaseId::random().unwrap(),
        document.metadata.scan_id.clone(),
        SnapshotFileName::from_scan_id(document.metadata.scan_id.as_str().as_bytes()),
        reservation.temp_name().to_owned(),
        current_process_instance().unwrap(),
        completed_at,
    )
    .unwrap();
    repository
        .insert_temp_lease_with_guard(&mut guard, &lease)
        .unwrap();
    drop(reservation);
    drop(guard);

    let debt = repository
        .inspect_retention_inventory(completed_at)
        .unwrap();
    assert!(debt.temporary_files.is_empty());
    assert_eq!(debt.residual_temp_leases.len(), 1);
    assert_eq!(
        debt.residual_temp_leases[0].scan_id,
        document.metadata.scan_id
    );
    repository
        .complete_scan(completed_at, counts(), &complete_coverage(), &document)
        .unwrap();
    assert_eq!(temp_lease_count(&store), 0);
}

#[test]
fn unleased_legacy_temp_remains_explicit_non_authoritative_inventory_debt() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let (_store, repository) = open_repository(&database);
    let staged = repository
        .store
        .as_ref()
        .unwrap()
        .stage(
            SnapshotFileName::from_scan_id(b"legacy-unleased-temp"),
            Duration::from_millis(100),
        )
        .unwrap();
    staged.abandon();
    let inventory = repository
        .inspect_retention_inventory(UNIX_EPOCH + Duration::from_secs(1))
        .unwrap();
    assert_eq!(inventory.temporary_files.len(), 1);
    assert_eq!(
        inventory.temporary_files[0].state,
        super::super::snapshot_retention_inventory::SnapshotTemporaryState::Unleased
    );
    assert!(inventory.accounting_unstable);
    assert!(inventory.residual_temp_leases.is_empty());
    let footprint = repository
        .inspect_owned_storage_footprint(UNIX_EPOCH + Duration::from_secs(1))
        .unwrap();
    assert_eq!(footprint.snapshots.unleased_temporary_count, 1);
    assert_eq!(footprint.snapshots.active_temporary_count, 0);
    assert!(footprint.snapshots.accounting_unstable);
}

#[test]
fn retention_inventory_reconciles_policy_storage_and_pin_observations() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_000_000);
    let (store, repository) = open_repository(&database);

    let mut references = Vec::new();
    for (ordinal, scan_id) in [
        "scan:retention-1",
        "scan:retention-2",
        "scan:retention-3",
        "scan:retention-4",
        "scan:retention-retired",
    ]
    .into_iter()
    .enumerate()
    {
        let completed_at = base + Duration::from_secs((ordinal + 1) as u64 * 10);
        let document = document(scan_id, &root);
        references.push(complete_snapshot(
            &store,
            &repository,
            &document,
            &root,
            completed_at,
        ));
    }

    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO snapshot_retention_tombstones (
                     scan_id, record_format_version, scan_status,
                     completed_at_unix_ms, snapshot_version,
                     snapshot_relative_path, snapshot_relative_path_encoding,
                     snapshot_checksum_sha256, committed_at_unix_ms
                 )
                 SELECT scan_id, 1, status, completed_at_unix_ms,
                        snapshot_version, snapshot_relative_path,
                        snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, completed_at_unix_ms + 1
                 FROM scans WHERE scan_id = ?1",
                [references[4].scan_id().as_str()],
            )
            .unwrap();
    });

    let orphan_document = document("scan:retention-orphan", &root);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                orphan_document.metadata.scan_id.clone(),
                root.clone(),
                base + Duration::from_secs(51),
            )
            .unwrap(),
        )
        .unwrap();
    drop(
        repository
            .publish_orphan_for_test(&orphan_document)
            .unwrap(),
    );
    let temp_document = document("scan:retention-live-temp", &root);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                temp_document.metadata.scan_id.clone(),
                root.clone(),
                base + Duration::from_secs(52),
            )
            .unwrap(),
        )
        .unwrap();
    let (staged, _, _) = repository.stage_document(&temp_document).unwrap();
    staged.abandon();

    let observed_at = base + Duration::from_secs(60);
    let pinned = repository
        .acquire_review_lease(&references[0], SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();
    let expires_at = pinned.expires_at().unwrap();
    let before_rows = store.with_connection(|connection| {
        (
            connection
                .query_row("SELECT count(*) FROM scans", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            connection
                .query_row("SELECT count(*) FROM snapshot_review_pins", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            connection
                .query_row(
                    "SELECT count(*) FROM snapshot_retention_tombstones",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
        )
    });

    store
        .set_snapshot_retention_cap_at_for_test(0, observed_at)
        .unwrap();
    let inventory = repository.inspect_retention_inventory(observed_at).unwrap();
    assert_eq!(inventory.cap_bytes, 0);
    assert_eq!(inventory.entries.len(), 5);
    assert_eq!(inventory.orphan_finals.len(), 1);
    assert_eq!(inventory.temporary_files.len(), 1);
    assert_eq!(
        inventory.temporary_files[0].state,
        super::super::snapshot_retention_inventory::SnapshotTemporaryState::QuiescentAtObservation
    );
    assert_eq!(inventory.residual_temp_leases.len(), 1);
    assert!(!inventory.accounting_unstable);
    assert!(inventory.non_evictable_over_cap);
    assert_eq!(inventory.totals.active_pin_rows, 1);
    assert_eq!(inventory.totals.expired_pin_rows, 0);
    assert_eq!(
        inventory
            .entries
            .iter()
            .find(|entry| entry.scan_id == references[3].scan_id().clone())
            .unwrap()
            .latest_rank,
        Some(1)
    );
    assert_eq!(
        inventory
            .entries
            .iter()
            .find(|entry| entry.scan_id == references[2].scan_id().clone())
            .unwrap()
            .latest_rank,
        Some(2)
    );
    let oldest = inventory
        .entries
        .iter()
        .find(|entry| entry.scan_id == references[0].scan_id().clone())
        .unwrap();
    assert_eq!(oldest.latest_rank, None);
    assert_eq!(oldest.pins.active, 1);
    assert!(oldest.is_policy_protected());
    assert_eq!(
        inventory
            .eviction_observations
            .iter()
            .map(|candidate| candidate.scan_id.as_str())
            .collect::<Vec<_>>(),
        vec![references[1].scan_id().as_str()]
    );
    assert!(matches!(
        inventory
            .entries
            .iter()
            .find(|entry| entry.scan_id == references[4].scan_id().clone())
            .unwrap()
            .logical_state,
        super::super::snapshot_retention_inventory::SnapshotRetentionLogicalState::Tombstoned { .. }
    ));
    assert!(inventory.totals.tombstoned_residual.charged_bytes > 0);
    assert!(inventory.totals.orphan.charged_bytes > 0);
    assert!(inventory.totals.temporary_quiescent.charged_bytes > 0);
    assert_eq!(
        inventory.totals.store_total.charged_bytes,
        inventory.totals.controls.charged_bytes
            + inventory.totals.available.charged_bytes
            + inventory.totals.tombstoned_residual.charged_bytes
            + inventory.totals.orphan.charged_bytes
            + inventory.totals.temporary_active.charged_bytes
            + inventory.totals.temporary_quiescent.charged_bytes
            + inventory.totals.temporary_unleased.charged_bytes
    );
    let footprint = repository
        .inspect_owned_storage_footprint(observed_at)
        .unwrap();
    assert_eq!(footprint.observed_at, observed_at);
    assert_eq!(
        footprint.snapshots.total.charged_bytes,
        inventory.totals.store_total.charged_bytes
    );
    assert_eq!(
        footprint.snapshots.protected.charged_bytes,
        inventory.totals.protected.charged_bytes
    );
    assert_eq!(
        footprint.snapshots.retention_eligible.charged_bytes,
        inventory.totals.eligible.charged_bytes
    );
    assert_eq!(footprint.snapshots.available_count, 4);
    assert_eq!(footprint.snapshots.protected_count, 3);
    assert_eq!(footprint.snapshots.retention_eligible_count, 1);
    assert_eq!(footprint.snapshots.tombstoned_residual_count, 1);
    assert_eq!(footprint.snapshots.orphan_count, 1);
    assert_eq!(footprint.snapshots.quiescent_temporary_count, 1);
    assert_eq!(footprint.snapshots.active_temporary_count, 0);
    assert_eq!(footprint.snapshots.unleased_temporary_count, 0);
    assert_eq!(footprint.snapshots.residual_temporary_lease_count, 1);
    assert_eq!(footprint.snapshots.active_pin_rows, 1);
    assert!(!footprint.snapshots.accounting_unstable);
    assert_eq!(footprint.embedded_ai_cache.record_count, 0);
    assert_eq!(
        footprint.physical_total.charged_bytes,
        footprint
            .database
            .charged_bytes
            .checked_add(footprint.snapshots.total.charged_bytes)
            .unwrap()
    );

    let after_rows = store.with_connection(|connection| {
        (
            connection
                .query_row("SELECT count(*) FROM scans", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            connection
                .query_row("SELECT count(*) FROM snapshot_review_pins", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            connection
                .query_row(
                    "SELECT count(*) FROM snapshot_retention_tombstones",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
        )
    });
    assert_eq!(after_rows, before_rows, "inventory must not mutate history");

    store
        .set_snapshot_retention_cap_at_for_test(u64::MAX, expires_at)
        .unwrap();
    let at_expiry = repository.inspect_retention_inventory(expires_at).unwrap();
    assert_eq!(at_expiry.cap_bytes, u64::MAX);
    assert_eq!(at_expiry.totals.active_pin_rows, 0);
    assert_eq!(at_expiry.totals.expired_pin_rows, 1);
    assert_eq!(
        review_pin_count(&store),
        1,
        "read-only inventory must not prune expired pins"
    );
    assert_eq!(
        at_expiry
            .eviction_observations
            .iter()
            .map(|candidate| candidate.scan_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            references[0].scan_id().as_str(),
            references[1].scan_id().as_str()
        ]
    );
    pinned.release().unwrap();

    let read_only =
        SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadOnly).unwrap();
    assert_eq!(
        read_only
            .inspect_retention_inventory(observed_at)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::ReadOnly
    );
    drop(read_only);
    drop(repository);
    drop(store);
    let (reopened_store, reopened_repository) = open_repository(&database);
    let reopened_inventory = reopened_repository
        .inspect_retention_inventory(expires_at + Duration::from_millis(1))
        .unwrap();
    assert_eq!(reopened_inventory.cap_bytes, u64::MAX);
    assert_eq!(reopened_inventory.entries.len(), 5);
    assert_eq!(reopened_inventory.orphan_finals.len(), 1);
    assert_eq!(reopened_inventory.temporary_files.len(), 1);
    assert_eq!(reopened_inventory.residual_temp_leases.len(), 1);
    drop(reopened_repository);
    drop(reopened_store);
}

#[test]
fn snapshot_storage_clear_removes_only_eligible_and_residual_finals() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_004_750);
    let (store, repository) = open_repository(&database);
    let mut references = Vec::new();
    for ordinal in 0..5_u64 {
        references.push(complete_snapshot(
            &store,
            &repository,
            &document(&format!("scan:explicit-clear-{ordinal}"), &root),
            &root,
            base + Duration::from_secs(10 + ordinal),
        ));
    }
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO snapshot_retention_tombstones (
                     scan_id, record_format_version, scan_status,
                     completed_at_unix_ms, snapshot_version,
                     snapshot_relative_path, snapshot_relative_path_encoding,
                     snapshot_checksum_sha256, committed_at_unix_ms
                 )
                 SELECT scan_id, 1, status, completed_at_unix_ms,
                        snapshot_version, snapshot_relative_path,
                        snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, completed_at_unix_ms + 1
                 FROM scans WHERE scan_id = ?1",
                [references[0].scan_id().as_str()],
            )
            .unwrap();
    });
    let observed_at = base + Duration::from_secs(30);
    let pin = repository
        .acquire_review_lease(&references[2], SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();

    let orphan_document = document("scan:explicit-clear-orphan", &root);
    record_running_scan(
        &store,
        &orphan_document,
        &root,
        observed_at - Duration::from_secs(1),
    );
    drop(
        repository
            .publish_orphan_for_test(&orphan_document)
            .unwrap(),
    );

    let temp_document = document("scan:explicit-clear-temp", &root);
    record_running_scan(
        &store,
        &temp_document,
        &root,
        observed_at - Duration::from_millis(500),
    );
    let (staged, _, _) = repository.stage_document(&temp_document).unwrap();
    staged.abandon();

    let prepared = repository
        .prepare_snapshot_storage_clear(observed_at + Duration::from_millis(1))
        .unwrap()
        .unwrap();
    assert_eq!(prepared.eligible_snapshot_count(), 1);
    assert_eq!(prepared.tombstoned_residual_count(), 1);
    assert_eq!(prepared.clearable_count(), Some(2));
    assert_eq!(prepared.protected_snapshot_count(), 3);
    assert_eq!(prepared.active_review_count(), 1);
    assert_eq!(prepared.excluded_maintenance_object_count(), 2);
    assert!(prepared.clearable().charged_bytes > 0);
    assert!(prepared.protected().charged_bytes > 0);
    assert!(prepared.excluded_maintenance().charged_bytes > 0);

    let expected_clearable = prepared.clearable();
    let result = repository
        .clear_snapshot_storage(prepared, observed_at + Duration::from_millis(2))
        .unwrap();
    assert_eq!(result.cleared_eligible_snapshot_count, 1);
    assert_eq!(result.cleared_tombstoned_residual_count, 1);
    assert_eq!(result.cleared_count(), Some(2));
    assert_eq!(result.cleared_usage, expected_clearable);
    assert_eq!(retention_tombstone_count(&store), 2);
    for reference in &references[..2] {
        assert_eq!(
            repository.load(reference).unwrap_err().kind,
            SnapshotRepositoryErrorKind::SnapshotUnavailable
        );
    }
    for reference in &references[2..] {
        assert!(repository.load(reference).is_ok());
    }
    assert!(pin.load(observed_at + Duration::from_millis(3)).is_ok());
    let after = repository
        .inspect_retention_inventory(observed_at + Duration::from_millis(3))
        .unwrap();
    assert_eq!(after.entries.len(), 3);
    assert_eq!(after.orphan_finals.len(), 1);
    assert_eq!(after.temporary_files.len(), 1);
    assert_eq!(after.totals.active_pin_rows, 1);
    pin.release().unwrap();
}

#[test]
fn snapshot_storage_clear_is_busy_for_unstable_temp_and_rejects_preview_drift() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_004_800);
    let (store, repository) = open_repository(&database);
    let mut references = Vec::new();
    for ordinal in 0..3_u64 {
        references.push(complete_snapshot(
            &store,
            &repository,
            &document(&format!("scan:explicit-clear-busy-{ordinal}"), &root),
            &root,
            base + Duration::from_secs(10 + ordinal),
        ));
    }
    let observed_at = base + Duration::from_secs(30);
    let temp_document = document("scan:explicit-clear-active-temp", &root);
    record_running_scan(
        &store,
        &temp_document,
        &root,
        observed_at - Duration::from_secs(1),
    );
    let (staged, _, _) = repository.stage_document(&temp_document).unwrap();
    assert_eq!(
        repository
            .prepare_snapshot_storage_clear(observed_at)
            .unwrap_err()
            .kind,
        SnapshotStorageClearErrorKind::Busy
    );
    staged.abandon();

    let pinned_preview = repository
        .prepare_snapshot_storage_clear(observed_at + Duration::from_millis(1))
        .unwrap()
        .unwrap();
    let pin = repository
        .acquire_review_lease(
            &references[0],
            SnapshotReviewPurpose::CleanupReview,
            observed_at + Duration::from_millis(2),
        )
        .unwrap();
    assert_eq!(
        repository
            .clear_snapshot_storage(pinned_preview, observed_at + Duration::from_millis(3))
            .unwrap_err()
            .kind,
        SnapshotStorageClearErrorKind::ChangedSincePreview
    );
    assert_eq!(retention_tombstone_count(&store), 0);
    pin.release().unwrap();

    let prepared = repository
        .prepare_snapshot_storage_clear(observed_at + Duration::from_millis(4))
        .unwrap()
        .unwrap();
    complete_snapshot(
        &store,
        &repository,
        &document("scan:explicit-clear-drift", &root),
        &root,
        observed_at + Duration::from_millis(5),
    );
    assert_eq!(
        repository
            .clear_snapshot_storage(prepared, observed_at + Duration::from_millis(6))
            .unwrap_err()
            .kind,
        SnapshotStorageClearErrorKind::ChangedSincePreview
    );
    assert_eq!(retention_tombstone_count(&store), 0);

    let read_only =
        SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadOnly).unwrap();
    assert_eq!(
        read_only
            .prepare_snapshot_storage_clear(observed_at + Duration::from_millis(7))
            .unwrap_err()
            .kind,
        SnapshotStorageClearErrorKind::ReadOnlyStore
    );
}

#[test]
fn snapshot_storage_clear_predecodes_all_and_retains_safe_residuals_after_effect_failure() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_004_850);
    let (store, repository) = open_repository(&database);
    let mut references = Vec::new();
    for ordinal in 0..3_u64 {
        references.push(complete_snapshot(
            &store,
            &repository,
            &document(&format!("scan:explicit-clear-failure-{ordinal}"), &root),
            &root,
            base + Duration::from_secs(10 + ordinal),
        ));
    }
    let observed_at = base + Duration::from_secs(30);
    let candidate_path = database
        .parent()
        .unwrap()
        .join("snapshots")
        .join(references[0].file_name().as_str());
    let mut corrupt_bytes = fs::read(&candidate_path).unwrap();
    *corrupt_bytes.last_mut().unwrap() ^= 0xff;
    let exact_bytes = fs::read(&candidate_path).unwrap();

    let prepared = repository
        .prepare_snapshot_storage_clear(observed_at)
        .unwrap()
        .unwrap();
    fs::write(&candidate_path, &corrupt_bytes).unwrap();
    assert_eq!(
        repository
            .clear_snapshot_storage(prepared, observed_at + Duration::from_millis(1))
            .unwrap_err()
            .kind,
        SnapshotStorageClearErrorKind::CorruptData
    );
    assert_eq!(retention_tombstone_count(&store), 0);
    fs::write(&candidate_path, &exact_bytes).unwrap();

    let prepared = repository
        .prepare_snapshot_storage_clear(observed_at + Duration::from_millis(2))
        .unwrap()
        .unwrap();
    let mut replaced = false;
    let error = repository
        .clear_snapshot_storage_with_hooks(
            prepared,
            observed_at + Duration::from_millis(3),
            || Ok(()),
            |storage, retained| {
                if !replaced {
                    repository
                        .store
                        .as_ref()
                        .unwrap()
                        .replace_final_for_test(references[0].file_name(), &exact_bytes)
                        .unwrap();
                    replaced = true;
                }
                storage.remove_observed_final_reconciled(retained)
            },
        )
        .unwrap_err();
    assert_eq!(error.kind, SnapshotStorageClearErrorKind::OutcomeUnknown);
    assert_eq!(retention_tombstone_count(&store), 1);
    assert!(candidate_path.exists());
    assert_eq!(
        repository.load(&references[0]).unwrap_err().kind,
        SnapshotRepositoryErrorKind::SnapshotUnavailable
    );
    let residual = repository
        .prepare_snapshot_storage_clear(observed_at + Duration::from_millis(4))
        .unwrap()
        .unwrap();
    assert_eq!(residual.eligible_snapshot_count(), 0);
    assert_eq!(residual.tombstoned_residual_count(), 1);
    repository
        .clear_snapshot_storage(residual, observed_at + Duration::from_millis(5))
        .unwrap();
    assert!(!candidate_path.exists());
}

#[test]
fn snapshot_storage_clear_reconciles_commit_adjacent_failures_and_unknown_outcomes() {
    let committed = TempDir::new().unwrap();
    let committed_database = committed.path().join("store/dux.sqlite3");
    let committed_root = committed.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_004_900);
    let (committed_store, committed_repository) = open_repository(&committed_database);
    for ordinal in 0..3_u64 {
        complete_snapshot(
            &committed_store,
            &committed_repository,
            &document(
                &format!("scan:explicit-clear-commit-{ordinal}"),
                &committed_root,
            ),
            &committed_root,
            base + Duration::from_secs(10 + ordinal),
        );
    }
    let prepared = committed_repository
        .prepare_snapshot_storage_clear(base + Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let reconciled = committed_repository
        .clear_snapshot_storage_with_hooks(
            prepared,
            base + Duration::from_secs(31),
            || Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable)),
            |storage, retained| storage.remove_observed_final_reconciled(retained),
        )
        .unwrap();
    assert_eq!(reconciled.cleared_eligible_snapshot_count, 1);
    assert_eq!(retention_tombstone_count(&committed_store), 1);

    let unknown = TempDir::new().unwrap();
    let unknown_database = unknown.path().join("store/dux.sqlite3");
    let unknown_root = unknown.path().join("scan-root");
    let (unknown_store, unknown_repository) = open_repository(&unknown_database);
    let mut references = Vec::new();
    for ordinal in 0..3_u64 {
        references.push(complete_snapshot(
            &unknown_store,
            &unknown_repository,
            &document(
                &format!("scan:explicit-clear-unknown-{ordinal}"),
                &unknown_root,
            ),
            &unknown_root,
            base + Duration::from_secs(40 + ordinal),
        ));
    }
    let candidate_path = unknown_database
        .parent()
        .unwrap()
        .join("snapshots")
        .join(references[0].file_name().as_str());
    let prepared = unknown_repository
        .prepare_snapshot_storage_clear(base + Duration::from_secs(50))
        .unwrap()
        .unwrap();
    let error = unknown_repository
        .clear_snapshot_storage_with_hooks(
            prepared,
            base + Duration::from_secs(51),
            || {
                install_future_schema(&unknown_database);
                Ok(())
            },
            |storage, retained| storage.remove_observed_final_reconciled(retained),
        )
        .unwrap_err();
    assert_eq!(error.kind, SnapshotStorageClearErrorKind::OutcomeUnknown);
    assert!(candidate_path.exists());
    assert_eq!(retention_tombstone_count(&unknown_store), 1);

    restore_current_schema(&unknown_database);
    let residual = unknown_repository
        .prepare_snapshot_storage_clear(base + Duration::from_secs(52))
        .unwrap()
        .unwrap();
    assert_eq!(residual.eligible_snapshot_count(), 0);
    assert_eq!(residual.tombstoned_residual_count(), 1);
    unknown_repository
        .clear_snapshot_storage(residual, base + Duration::from_secs(53))
        .unwrap();
    assert!(!candidate_path.exists());
}

#[test]
fn retention_cap_evicts_oldest_one_at_a_time_and_preserves_latest_two() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_005_000);
    let (store, repository) = open_repository(&database);
    let mut references = Vec::new();
    for ordinal in 0..4_u64 {
        let completed_at = base + Duration::from_secs(10 + ordinal);
        let document = document(&format!("scan:cap-order-{ordinal}"), &root);
        references.push(complete_snapshot(
            &store,
            &repository,
            &document,
            &root,
            completed_at,
        ));
    }
    let canonical_observed_at = base + Duration::from_secs(30);
    let observed_at = canonical_observed_at + Duration::from_nanos(999_999);
    store
        .set_snapshot_retention_cap_at_for_test(0, observed_at)
        .unwrap();

    let first = repository
        .enforce_retention_cap_with_hook(observed_at, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
        .unwrap();
    assert_eq!(first.observed_at, canonical_observed_at);
    assert!(matches!(
        first.outcome,
        SnapshotRetentionBatchOutcome::TombstonedAndRemoved { ref scan_id, .. }
            if scan_id == references[0].scan_id()
    ));
    assert!(first.charged_bytes_after < first.charged_bytes_before);
    assert!(first.has_more);
    assert_eq!(retention_tombstone_count(&store), 1);
    assert_eq!(
        repository.load(&references[0]).unwrap_err().kind,
        SnapshotRepositoryErrorKind::SnapshotUnavailable
    );
    assert!(repository.load(&references[2]).is_ok());
    assert!(repository.load(&references[3]).is_ok());

    let second = repository
        .enforce_retention_cap(observed_at + Duration::from_millis(1))
        .unwrap();
    assert_eq!(
        second.observed_at,
        canonical_observed_at + Duration::from_millis(1)
    );
    assert!(matches!(
        second.outcome,
        SnapshotRetentionBatchOutcome::TombstonedAndRemoved { ref scan_id, .. }
            if scan_id == references[1].scan_id()
    ));
    assert!(!second.has_more);
    assert_eq!(retention_tombstone_count(&store), 2);

    let protected_only = repository
        .enforce_retention_cap(observed_at + Duration::from_millis(2))
        .unwrap();
    assert_eq!(
        protected_only.outcome,
        SnapshotRetentionBatchOutcome::DeferredNoEligibleSnapshot
    );
    assert_eq!(retention_tombstone_count(&store), 2);
    assert!(repository.load(&references[2]).is_ok());
    assert!(repository.load(&references[3]).is_ok());
}

#[test]
fn retention_cap_normalizes_every_observation_and_rejects_an_invalid_clock() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let (_store, repository) = open_repository(&database);
    let canonical_observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_005_500);

    let under_cap = repository
        .enforce_retention_cap(canonical_observed_at + Duration::from_nanos(999_999))
        .unwrap();
    assert_eq!(under_cap.observed_at, canonical_observed_at);
    assert_eq!(under_cap.outcome, SnapshotRetentionBatchOutcome::UnderCap);

    let error = repository
        .enforce_retention_cap(UNIX_EPOCH - Duration::from_millis(1))
        .unwrap_err();
    assert_eq!(
        error.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
    );
}

#[test]
fn retention_cap_defers_for_active_temp_and_never_evicts_an_active_pin() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_006_000);
    let (store, repository) = open_repository(&database);
    let mut references = Vec::new();
    for ordinal in 0..3_u64 {
        let completed_at = base + Duration::from_secs(10 + ordinal);
        let document = document(&format!("scan:cap-pin-{ordinal}"), &root);
        references.push(complete_snapshot(
            &store,
            &repository,
            &document,
            &root,
            completed_at,
        ));
    }
    let observed_at = base + Duration::from_secs(30);
    store
        .set_snapshot_retention_cap_at_for_test(0, observed_at)
        .unwrap();

    let live_document = document("scan:cap-live-temp", &root);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                live_document.metadata.scan_id.clone(),
                root.clone(),
                observed_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    let (staged, _, _) = repository.stage_document(&live_document).unwrap();
    let active_footprint = repository
        .inspect_owned_storage_footprint(observed_at)
        .unwrap();
    assert_eq!(active_footprint.snapshots.active_temporary_count, 1);
    assert_eq!(active_footprint.snapshots.unleased_temporary_count, 0);
    assert!(active_footprint.snapshots.accounting_unstable);
    let unstable = repository.enforce_retention_cap(observed_at).unwrap();
    assert_eq!(
        unstable.outcome,
        SnapshotRetentionBatchOutcome::DeferredUnstable
    );
    assert_eq!(retention_tombstone_count(&store), 0);
    staged.abandon();

    let pin = repository
        .acquire_review_lease(
            &references[0],
            SnapshotReviewPurpose::CleanupReview,
            observed_at + Duration::from_millis(1),
        )
        .unwrap();
    let protected = repository
        .enforce_retention_cap(observed_at + Duration::from_millis(2))
        .unwrap();
    assert_eq!(
        protected.outcome,
        SnapshotRetentionBatchOutcome::DeferredNoEligibleSnapshot
    );
    assert_eq!(retention_tombstone_count(&store), 0);
    assert!(pin.load(observed_at + Duration::from_millis(3)).is_ok());
    pin.release().unwrap();
}

#[test]
fn retention_cap_defers_for_an_unleased_temp() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_006_500);
    let (store, repository) = open_repository(&database);
    for ordinal in 0..3_u64 {
        let completed_at = base + Duration::from_secs(10 + ordinal);
        complete_snapshot(
            &store,
            &repository,
            &document(&format!("scan:cap-unleased-{ordinal}"), &root),
            &root,
            completed_at,
        );
    }
    let observed_at = base + Duration::from_secs(30);
    store
        .set_snapshot_retention_cap_at_for_test(0, observed_at)
        .unwrap();
    let unleased = repository
        .store
        .as_ref()
        .unwrap()
        .stage(
            SnapshotFileName::from_scan_id(b"cap-unleased-temp"),
            Duration::from_millis(100),
        )
        .unwrap();
    unleased.abandon();

    let result = repository.enforce_retention_cap(observed_at).unwrap();
    assert_eq!(
        result.outcome,
        SnapshotRetentionBatchOutcome::DeferredUnstable
    );
    assert_eq!(retention_tombstone_count(&store), 0);
    let inventory = repository
        .inspect_retention_inventory(observed_at + Duration::from_millis(1))
        .unwrap();
    assert!(inventory.accounting_unstable);
    assert!(matches!(
        inventory.temporary_files.as_slice(),
        [temporary]
            if temporary.state
                == super::super::snapshot_retention_inventory::SnapshotTemporaryState::Unleased
    ));
}

#[test]
fn retention_cap_can_retire_with_a_quiescent_row_bound_temp() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_006_750);
    let (store, repository) = open_repository(&database);
    let mut references = Vec::new();
    for ordinal in 0..3_u64 {
        let completed_at = base + Duration::from_secs(10 + ordinal);
        references.push(complete_snapshot(
            &store,
            &repository,
            &document(&format!("scan:cap-quiescent-{ordinal}"), &root),
            &root,
            completed_at,
        ));
    }
    let observed_at = base + Duration::from_secs(30);
    store
        .set_snapshot_retention_cap_at_for_test(0, observed_at)
        .unwrap();
    let live_document = document("scan:cap-quiescent-temp", &root);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                live_document.metadata.scan_id.clone(),
                root,
                observed_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    let (staged, _, _) = repository.stage_document(&live_document).unwrap();
    staged.abandon();

    let before = repository.inspect_retention_inventory(observed_at).unwrap();
    assert!(!before.accounting_unstable);
    assert!(matches!(
        before.temporary_files.as_slice(),
        [temporary]
            if temporary.state
                == super::super::snapshot_retention_inventory::SnapshotTemporaryState::QuiescentAtObservation
    ));
    let result = repository.enforce_retention_cap(observed_at).unwrap();
    assert!(matches!(
        result.outcome,
        SnapshotRetentionBatchOutcome::TombstonedAndRemoved { ref scan_id, .. }
            if scan_id == references[0].scan_id()
    ));
    assert_eq!(retention_tombstone_count(&store), 1);
    let after = repository
        .inspect_retention_inventory(observed_at + Duration::from_millis(1))
        .unwrap();
    assert!(!after.accounting_unstable);
    assert_eq!(after.temporary_files.len(), 1);
}

#[test]
fn retention_retries_tombstoned_residual_before_considering_the_cap() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_007_000);
    let (store, repository) = open_repository(&database);
    let reference = complete_snapshot(
        &store,
        &repository,
        &document("scan:cap-residual", &root),
        &root,
        completed_at,
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO snapshot_retention_tombstones (
                     scan_id, record_format_version, scan_status,
                     completed_at_unix_ms, snapshot_version,
                     snapshot_relative_path, snapshot_relative_path_encoding,
                     snapshot_checksum_sha256, committed_at_unix_ms
                 )
                 SELECT scan_id, 1, status, completed_at_unix_ms,
                        snapshot_version, snapshot_relative_path,
                        snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, completed_at_unix_ms + 1
                 FROM scans WHERE scan_id = ?1",
                [reference.scan_id().as_str()],
            )
            .unwrap();
    });
    store
        .set_snapshot_retention_cap_at_for_test(u64::MAX, completed_at)
        .unwrap();

    let result = repository
        .enforce_retention_cap(completed_at + Duration::from_millis(2))
        .unwrap();
    assert!(matches!(
        result.outcome,
        SnapshotRetentionBatchOutcome::RemovedTombstonedResidual { ref scan_id, .. }
            if scan_id == reference.scan_id()
    ));
    assert_eq!(retention_tombstone_count(&store), 1);
    assert_eq!(
        repository.load(&reference).unwrap_err().kind,
        SnapshotRepositoryErrorKind::SnapshotUnavailable
    );
    let settled = repository
        .enforce_retention_cap(completed_at + Duration::from_millis(3))
        .unwrap();
    assert_eq!(settled.outcome, SnapshotRetentionBatchOutcome::UnderCap);
}

#[test]
fn retention_never_removes_a_tombstoned_name_with_changed_snapshot_bytes() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_007_500);
    let (store, repository) = open_repository(&database);
    let reference = complete_snapshot(
        &store,
        &repository,
        &document("scan:cap-changed-residual", &root),
        &root,
        completed_at,
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO snapshot_retention_tombstones (
                     scan_id, record_format_version, scan_status,
                     completed_at_unix_ms, snapshot_version,
                     snapshot_relative_path, snapshot_relative_path_encoding,
                     snapshot_checksum_sha256, committed_at_unix_ms
                 )
                 SELECT scan_id, 1, status, completed_at_unix_ms,
                        snapshot_version, snapshot_relative_path,
                        snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, completed_at_unix_ms + 1
                 FROM scans WHERE scan_id = ?1",
                [reference.scan_id().as_str()],
            )
            .unwrap();
    });
    let snapshot_path = database
        .parent()
        .unwrap()
        .join("snapshots")
        .join(reference.file_name().as_str());
    let mut changed = fs::read(&snapshot_path).unwrap();
    let last = changed.last_mut().unwrap();
    *last ^= 0xff;
    fs::write(&snapshot_path, changed).unwrap();

    let error = repository
        .enforce_retention_cap(completed_at + Duration::from_millis(2))
        .unwrap_err();
    assert!(matches!(
        error.kind,
        SnapshotRepositoryErrorKind::Codec(_) | SnapshotRepositoryErrorKind::ReferenceMismatch
    ));
    assert!(snapshot_path.exists());
    assert_eq!(retention_tombstone_count(&store), 1);
}

#[test]
fn retention_revalidates_a_replacement_created_after_body_validation() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_007_750);
    let (store, repository) = open_repository(&database);
    let mut references = Vec::new();
    for ordinal in 0..3_u64 {
        let completed_at = base + Duration::from_secs(10 + ordinal);
        references.push(complete_snapshot(
            &store,
            &repository,
            &document(
                &format!("scan:cap-post-decode-replacement-{ordinal}"),
                &root,
            ),
            &root,
            completed_at,
        ));
    }
    let observed_at = base + Duration::from_secs(30);
    store
        .set_snapshot_retention_cap_at_for_test(0, observed_at)
        .unwrap();
    let snapshot_path = database
        .parent()
        .unwrap()
        .join("snapshots")
        .join(references[0].file_name().as_str());
    let exact_bytes = fs::read(&snapshot_path).unwrap();

    let error = repository
        .enforce_retention_cap_with_hook(observed_at, || {
            repository
                .store
                .as_ref()
                .unwrap()
                .replace_final_for_test(references[0].file_name(), &exact_bytes)
                .unwrap();
            Ok(())
        })
        .unwrap_err();
    assert_eq!(
        error.kind,
        SnapshotRepositoryErrorKind::Storage(SnapshotStorageErrorKind::UnsafeObject)
    );
    assert_eq!(fs::read(&snapshot_path).unwrap(), exact_bytes);
    assert_eq!(retention_tombstone_count(&store), 1);

    let retried = repository
        .enforce_retention_cap(observed_at + Duration::from_millis(1))
        .unwrap();
    assert!(matches!(
        retried.outcome,
        SnapshotRetentionBatchOutcome::RemovedTombstonedResidual { ref scan_id, .. }
            if scan_id == references[0].scan_id()
    ));
    assert!(!snapshot_path.exists());
}

#[test]
fn retention_commit_schema_race_leaves_a_safe_retryable_residual() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let base = UNIX_EPOCH + Duration::from_millis(1_750_000_008_000);
    let (store, repository) = open_repository(&database);
    let mut references = Vec::new();
    for ordinal in 0..3_u64 {
        let completed_at = base + Duration::from_secs(10 + ordinal);
        let document = document(&format!("scan:cap-race-{ordinal}"), &root);
        references.push(complete_snapshot(
            &store,
            &repository,
            &document,
            &root,
            completed_at,
        ));
    }
    let observed_at = base + Duration::from_secs(30);
    store
        .set_snapshot_retention_cap_at_for_test(0, observed_at)
        .unwrap();
    let expected_path = database
        .parent()
        .unwrap()
        .join("snapshots")
        .join(references[0].file_name().as_str());

    let error = repository
        .enforce_retention_cap_with_hook(observed_at, || {
            install_future_schema(&database);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(
        error.kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::OutcomeUnknown)
    );
    assert!(expected_path.exists());
    assert_eq!(retention_tombstone_count(&store), 1);

    restore_current_schema(&database);
    assert_eq!(
        repository.load(&references[0]).unwrap_err().kind,
        SnapshotRepositoryErrorKind::SnapshotUnavailable
    );
    let retry = repository
        .enforce_retention_cap(observed_at + Duration::from_millis(1))
        .unwrap();
    assert!(matches!(
        retry.outcome,
        SnapshotRetentionBatchOutcome::RemovedTombstonedResidual { ref scan_id, .. }
            if scan_id == references[0].scan_id()
    ));
    assert!(!expected_path.exists());
    assert_eq!(retention_tombstone_count(&store), 1);
}

#[test]
fn retention_inventory_fails_on_invalid_cap_before_snapshot_lock() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    let lock_document = document("scan:retention-setting-lock", &root);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                lock_document.metadata.scan_id.clone(),
                root.clone(),
                observed_at - Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    let held_publication = repository.publish_orphan_for_test(&lock_document).unwrap();

    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO settings (
                     setting_key, value_json, value_schema_version,
                     updated_at_unix_ms
                 ) VALUES (
                     'snapshot_retention', '{\"cap_bytes\":1,\"extra\":2}', 1, 1
                 )",
                [],
            )
            .unwrap();
    });
    assert_eq!(
        repository
            .inspect_retention_inventory(observed_at)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );

    store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE settings
                 SET value_json = '{\"cap_bytes\":1}', value_schema_version = 2
                 WHERE setting_key = 'snapshot_retention'",
                [],
            )
            .unwrap();
    });
    assert_eq!(
        repository
            .inspect_retention_inventory(observed_at)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::IncompatibleSchema)
    );
    drop(held_publication);
}

#[test]
fn retention_inventory_rejects_duplicate_history_for_one_physical_name() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    let document = document("scan:retention-duplicate-source", &root);
    let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO scans (
                     scan_id, volume_id, root_path, root_path_encoding,
                     started_at_unix_ms, completed_at_unix_ms, status,
                     snapshot_version, snapshot_relative_path,
                     snapshot_relative_path_encoding, snapshot_checksum_sha256,
                     directory_count, file_count, logical_bytes, allocated_bytes,
                     coverage_status, coverage_permille, issue_count
                 )
                 SELECT 'scan:retention-duplicate-hostile', volume_id,
                        root_path, root_path_encoding, started_at_unix_ms,
                        completed_at_unix_ms, status, snapshot_version,
                        snapshot_relative_path, snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, directory_count, file_count,
                        logical_bytes, allocated_bytes, coverage_status,
                        coverage_permille, issue_count
                 FROM scans WHERE scan_id = ?1",
                [reference.scan_id().as_str()],
            )
            .unwrap();
    });
    assert_eq!(
        repository
            .inspect_retention_inventory(completed_at + Duration::from_secs(1))
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );
    assert_eq!(
        repository
            .reconcile_physical_orphan(completed_at + Duration::from_secs(1))
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );
}

#[cfg(unix)]
#[test]
fn retention_inventory_rejects_lexically_aliased_stored_roots() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    let document = document("scan:retention-aliased-root", &root);
    let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);
    for hostile_root in [
        b"/alias/./root".as_slice(),
        b"/alias//root",
        b"/alias/root/",
    ] {
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE scans
                     SET root_path = ?1, root_path_encoding = 1
                     WHERE scan_id = ?2",
                    rusqlite::params![hostile_root, reference.scan_id().as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            repository
                .inspect_retention_inventory(completed_at + Duration::from_secs(1))
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
        );
        assert_eq!(
            repository
                .reconcile_physical_orphan(completed_at + Duration::from_secs(1))
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)]
fn retention_inventory_rejects_inconsistent_active_pin_storage() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("missing/dux.sqlite3");
    let root = temp.path().join("missing/scan-root");
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let observed_at = completed_at + Duration::from_secs(1);
    let (store, repository) = open_repository(&database);
    let missing_document = document("scan:retention-active-missing", &root);
    let reference = complete_snapshot(&store, &repository, &missing_document, &root, completed_at);
    let lease = repository
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();
    // DUX-DESTRUCTIVE: allow=test-snapshot-inventory-remove-pinned-final -- remove only this TempDir-owned published fixture to prove an active pin cannot hide a missing physical final
    std::fs::remove_file(
        database
            .parent()
            .unwrap()
            .join("snapshots")
            .join(reference.file_name().as_str()),
    )
    .unwrap();
    assert_eq!(
        repository
            .inspect_retention_inventory(observed_at)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );
    lease.release().unwrap();

    let database = temp.path().join("tombstoned/dux.sqlite3");
    let root = temp.path().join("tombstoned/scan-root");
    let (store, repository) = open_repository(&database);
    let document = document("scan:retention-active-tombstone", &root);
    let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);
    let lease = repository
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO snapshot_retention_tombstones (
                     scan_id, record_format_version, scan_status,
                     completed_at_unix_ms, snapshot_version,
                     snapshot_relative_path, snapshot_relative_path_encoding,
                     snapshot_checksum_sha256, committed_at_unix_ms
                 )
                 SELECT scan_id, 1, status, completed_at_unix_ms,
                        snapshot_version, snapshot_relative_path,
                        snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, completed_at_unix_ms + 1
                 FROM scans WHERE scan_id = ?1",
                [reference.scan_id().as_str()],
            )
            .unwrap();
    });
    assert_eq!(
        repository
            .inspect_retention_inventory(observed_at)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );
    lease.release().unwrap();
}

#[test]
fn review_lease_is_explicit_renewable_and_drop_expires_without_writing() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-review-lease", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let observed_at = completed_at + Duration::from_secs(1);
    let (store, repository) = open_repository(&database);
    let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);

    let mut lease = repository
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();
    assert_eq!(lease.reference(), &reference);
    assert_eq!(lease.purpose(), SnapshotReviewPurpose::Explorer);
    assert_eq!(
        lease.expires_at().unwrap(),
        observed_at + Duration::from_secs(10 * 60)
    );
    assert_eq!(lease.load(observed_at).unwrap(), document);
    assert_eq!(review_pin_count(&store), 1);
    assert_eq!(
        lease
            .renew(observed_at - Duration::from_millis(1))
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
    );

    let renewed_at = observed_at + Duration::from_secs(2 * 60);
    let renewed_expiry = lease.renew(renewed_at).unwrap();
    assert_eq!(renewed_expiry, renewed_at + Duration::from_secs(10 * 60));
    assert_eq!(lease.load(renewed_at).unwrap(), document);
    assert_eq!(
        lease.load(renewed_expiry).unwrap_err().kind,
        SnapshotRepositoryErrorKind::ReviewLeaseExpired
    );
    assert_eq!(
        lease.renew(renewed_expiry).unwrap_err().kind,
        SnapshotRepositoryErrorKind::ReviewLeaseExpired
    );
    lease.release().unwrap();
    assert_eq!(review_pin_count(&store), 0);

    let cross_repository = repository
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();
    drop(repository);
    let reopened =
        SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadWrite).unwrap();
    assert_eq!(review_pin_count(&store), 1);
    assert_eq!(cross_repository.load(observed_at).unwrap(), document);
    cross_repository.release().unwrap();

    let dropped = reopened
        .acquire_review_lease(
            &reference,
            SnapshotReviewPurpose::CleanupReview,
            observed_at,
        )
        .unwrap();
    let dropped_expiry = dropped.expires_at().unwrap();
    drop(dropped);
    // Drop is close-only so it remains safe during unwinding or while a
    // caller happens to own another persistence guard.
    assert_eq!(review_pin_count(&store), 1);

    let replacement = reopened
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, dropped_expiry)
        .unwrap();
    // Equality is expired, and acquisition prunes the stale coordination
    // row before inserting its replacement.
    assert_eq!(review_pin_count(&store), 1);
    replacement.release().unwrap();
    assert_eq!(review_pin_count(&store), 0);

    let mut expired_while_retained = reopened
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();
    let expired_at = expired_while_retained.expires_at().unwrap();
    let concurrent =
        SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadWrite).unwrap();
    let successor = concurrent
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, expired_at)
        .unwrap();
    assert_eq!(
        expired_while_retained.load(expired_at).unwrap_err().kind,
        SnapshotRepositoryErrorKind::ReviewLeaseExpired
    );
    assert_eq!(
        expired_while_retained.renew(expired_at).unwrap_err().kind,
        SnapshotRepositoryErrorKind::ReviewLeaseExpired
    );
    // Pruning by another repository already established the exact release
    // postcondition, so releasing the old retained object is idempotent.
    expired_while_retained.release().unwrap();
    successor.release().unwrap();
    assert_eq!(review_pin_count(&store), 0);
}

#[test]
fn review_lease_reconciles_exact_post_commit_outcomes() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-review-reconcile", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let observed_at = completed_at + Duration::from_secs(1);
    let (store, repository) = open_repository(&database);
    let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);

    let mut lease = repository
        .acquire_review_lease_after_commit_failure_for_test(
            &reference,
            SnapshotReviewPurpose::Explorer,
            observed_at,
        )
        .unwrap();
    assert_eq!(review_pin_count(&store), 1);
    assert_eq!(lease.load(observed_at).unwrap(), document);

    lease.fail_next_renew_after_commit_for_test();
    let renewed_at = observed_at + Duration::from_secs(60);
    assert_eq!(
        lease.renew(renewed_at).unwrap(),
        renewed_at + Duration::from_secs(10 * 60)
    );
    lease.fail_release_after_commit_for_test();
    lease.release().unwrap();
    assert_eq!(review_pin_count(&store), 0);

    let conflicting = repository
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();
    conflicting.replace_release_with_conflict_after_commit_for_test();
    assert_eq!(
        conflicting.release().unwrap_err().kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::OutcomeUnknown)
    );
    assert_eq!(review_pin_count(&store), 1);
    store.with_connection(|connection| {
        connection
            .execute("DELETE FROM snapshot_review_pins", [])
            .unwrap();
    });
}

#[test]
fn review_lease_rejects_tombstones_and_hostile_active_pin_rows() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-review-hostile", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let observed_at = completed_at + Duration::from_secs(1);
    let (store, repository) = open_repository(&database);
    let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);

    let read_only =
        SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadOnly).unwrap();
    assert_eq!(
        read_only
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
            .err()
            .unwrap()
            .kind,
        SnapshotRepositoryErrorKind::ReadOnly
    );

    store.with_connection(|connection| {
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "INSERT INTO snapshot_review_pins (
                     pin_id, record_format_version, scan_id, scan_status,
                     completed_at_unix_ms, snapshot_version,
                     snapshot_relative_path, snapshot_relative_path_encoding,
                     snapshot_checksum_sha256, owner_process_instance, purpose,
                     created_at_unix_ms, renewed_at_unix_ms, expires_at_unix_ms
                 )
                 SELECT 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 1, scan_id, status,
                        completed_at_unix_ms, snapshot_version,
                        snapshot_relative_path, snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, 'not-a-process-instance', 'explorer',
                        ?2, ?2, ?2 + 600000
                 FROM scans WHERE scan_id = ?1",
                rusqlite::params![
                    reference.scan_id().as_str(),
                    super::super::history::system_time_to_unix_ms(
                        observed_at,
                        HistoryErrorKind::InvalidInput,
                    )
                    .unwrap(),
                ],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });
    assert_eq!(
        repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
            .err()
            .unwrap()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );

    store.with_connection(|connection| {
        connection
            .execute(
                "DELETE FROM snapshot_review_pins
                 WHERE pin_id = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
                [],
            )
            .unwrap();
    });

    let hostile_scan_id = "scan:snapshot-review-hostile-parent";
    let owner = repository.review.as_ref().unwrap().owner().unwrap();
    let observed_ms =
        super::super::history::system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)
            .unwrap();
    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO scans (
                     scan_id, volume_id, root_path, root_path_encoding,
                     started_at_unix_ms, completed_at_unix_ms, status,
                     snapshot_version, snapshot_relative_path,
                     snapshot_relative_path_encoding, snapshot_checksum_sha256,
                     directory_count, file_count, logical_bytes, allocated_bytes,
                     coverage_status, coverage_permille, issue_count
                 )
                 SELECT ?2, volume_id, root_path, root_path_encoding,
                        started_at_unix_ms, completed_at_unix_ms, status,
                        snapshot_version, snapshot_relative_path,
                        snapshot_relative_path_encoding, snapshot_checksum_sha256,
                        directory_count, file_count, logical_bytes, allocated_bytes,
                        coverage_status, coverage_permille, issue_count
                 FROM scans WHERE scan_id = ?1",
                rusqlite::params![reference.scan_id().as_str(), hostile_scan_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO snapshot_review_pins (
                     pin_id, record_format_version, scan_id, scan_status,
                     completed_at_unix_ms, snapshot_version,
                     snapshot_relative_path, snapshot_relative_path_encoding,
                     snapshot_checksum_sha256, owner_process_instance, purpose,
                     created_at_unix_ms, renewed_at_unix_ms, expires_at_unix_ms
                 )
                 SELECT 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', 1, scan_id, status,
                        completed_at_unix_ms, snapshot_version,
                        snapshot_relative_path, snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, ?2, 'explorer', ?3, ?3, ?3 + 600000
                 FROM scans WHERE scan_id = ?1",
                rusqlite::params![hostile_scan_id, owner.as_str(), observed_ms],
            )
            .unwrap();
    });
    assert_eq!(
        repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
            .err()
            .unwrap()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );
    store.with_connection(|connection| {
        connection
            .execute(
                "DELETE FROM snapshot_review_pins WHERE scan_id = ?1",
                [hostile_scan_id],
            )
            .unwrap();
        connection
            .execute("DELETE FROM scans WHERE scan_id = ?1", [hostile_scan_id])
            .unwrap();
        connection
            .execute(
                "INSERT INTO snapshot_retention_tombstones (
                     scan_id, record_format_version, scan_status,
                     completed_at_unix_ms, snapshot_version,
                     snapshot_relative_path, snapshot_relative_path_encoding,
                     snapshot_checksum_sha256, committed_at_unix_ms
                 )
                 SELECT scan_id, 1, status, completed_at_unix_ms,
                        snapshot_version, snapshot_relative_path,
                        snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, completed_at_unix_ms + 1
                 FROM scans WHERE scan_id = ?1",
                [reference.scan_id().as_str()],
            )
            .unwrap();
    });
    assert_eq!(
        repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
            .err()
            .unwrap()
            .kind,
        SnapshotRepositoryErrorKind::SnapshotUnavailable
    );
}

#[test]
fn review_lease_local_bound_is_fail_closed_and_releases_slots() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-review-bound", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let observed_at = completed_at + Duration::from_secs(1);
    let (store, repository) = open_repository(&database);
    let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);

    let mut leases = Vec::new();
    for _ in 0..MAX_ACTIVE_PINS_PER_OWNER {
        leases.push(
            repository
                .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
                .unwrap(),
        );
    }
    assert_eq!(review_pin_count(&store), MAX_ACTIVE_PINS_PER_OWNER as i64);
    assert_eq!(
        repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
            .err()
            .unwrap()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::QueryLimitExceeded)
    );
    leases.pop().unwrap().release().unwrap();
    let replacement = repository
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();
    drop(replacement);
    drop(leases);
    assert_eq!(review_pin_count(&store), MAX_ACTIVE_PINS_PER_OWNER as i64);
}

#[test]
fn review_population_pruning_is_bounded_and_oversize_fails_corrupt() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-review-population", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let observed_at = completed_at + Duration::from_secs(20 * 60);
    let observed_ms =
        super::super::history::system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)
            .unwrap();
    let (store, repository) = open_repository(&database);
    let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);
    let owner = repository.review.as_ref().unwrap().owner().unwrap();
    let owner_prefix = owner.as_str().rsplit_once(':').unwrap().0.to_owned();

    let insert_population = |count: usize, expired: bool| {
        store.with_connection(|connection| {
            connection.execute_batch("BEGIN IMMEDIATE").unwrap();
            for ordinal in 1..=count {
                let id = format!("{ordinal:032x}");
                let row_owner = format!("{owner_prefix}:{ordinal:032x}");
                let renewed = if expired {
                    observed_ms - 600_000
                } else {
                    observed_ms
                };
                connection
                    .execute(
                        "INSERT INTO snapshot_review_pins (
                             pin_id, record_format_version, scan_id, scan_status,
                             completed_at_unix_ms, snapshot_version,
                             snapshot_relative_path, snapshot_relative_path_encoding,
                             snapshot_checksum_sha256, owner_process_instance, purpose,
                             created_at_unix_ms, renewed_at_unix_ms, expires_at_unix_ms
                         )
                         SELECT ?2, 1, scan_id, status, completed_at_unix_ms,
                                snapshot_version, snapshot_relative_path,
                                snapshot_relative_path_encoding,
                                snapshot_checksum_sha256, ?3, 'explorer', ?4, ?4, ?4 + 600000
                         FROM scans WHERE scan_id = ?1",
                        rusqlite::params![reference.scan_id().as_str(), id, row_owner, renewed,],
                    )
                    .unwrap();
            }
            connection.execute_batch("COMMIT").unwrap();
        });
    };

    insert_population(MAX_EXPIRED_PRUNE + 1, true);
    let colliding =
        SnapshotReviewPinId::from_stored(format!("{:032x}", MAX_EXPIRED_PRUNE + 1)).unwrap();
    let unique =
        SnapshotReviewPinId::from_stored("ffffffffffffffffffffffffffffffff".into()).unwrap();
    let lease = repository
        .acquire_review_lease_with_pin_ids_for_test(
            &reference,
            SnapshotReviewPurpose::Explorer,
            observed_at,
            vec![colliding, unique],
        )
        .unwrap();
    // One bounded batch removes exactly 64 stale rows. The first prepared
    // ID collides with the remaining sentinel and the second succeeds;
    // collision retry must not run pruning a second time.
    assert_eq!(review_pin_count(&store), 2);
    lease.release().unwrap();
    store.with_connection(|connection| {
        connection
            .execute("DELETE FROM snapshot_review_pins", [])
            .unwrap();
    });

    insert_population(MAX_ACTIVE_PINS + 1, false);
    assert_eq!(
        repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .err()
            .unwrap()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );
}

#[test]
fn dropped_review_pin_survives_independent_store_reopen_until_expiry() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-review-store-reopen", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let observed_at = completed_at + Duration::from_secs(1);

    let (reference, expiry) = {
        let (store, repository) = open_repository(&database);
        let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);
        let lease = repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        let expiry = lease.expires_at().unwrap();
        drop(lease);
        assert_eq!(review_pin_count(&store), 1);
        (reference, expiry)
    };

    let (reopened_store, reopened_repository) = open_repository(&database);
    assert_eq!(review_pin_count(&reopened_store), 1);
    let replacement = reopened_repository
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, expiry)
        .unwrap();
    assert_eq!(review_pin_count(&reopened_store), 1);
    replacement.release().unwrap();
    assert_eq!(review_pin_count(&reopened_store), 0);
}

#[test]
fn review_lease_operations_fail_closed_after_external_newer_schema() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let review_document = document("scan:snapshot-review-schema-fence", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let observed_at = completed_at + Duration::from_secs(1);
    let (store, repository) = open_repository(&database);
    let reference = complete_snapshot(&store, &repository, &review_document, &root, completed_at);
    let mut lease = repository
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();
    let orphan_root = temp.path().join("orphan-root");
    let orphan_document = document("scan:snapshot-review-schema-fence-orphan", &orphan_root);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                orphan_document.metadata.scan_id.clone(),
                orphan_root,
                observed_at,
            )
            .unwrap(),
        )
        .unwrap();
    let held_publication = repository
        .publish_orphan_for_test(&orphan_document)
        .unwrap();

    let current = super::super::status::DATABASE_SCHEMA_VERSION;
    let future = current + 1;
    let external = rusqlite::Connection::open(&database).unwrap();
    external
        .execute(
            "INSERT INTO schema_migrations (
                 version, name, checksum_sha256, applied_at_unix_ms
             ) VALUES (?1, 'test-future-review-schema', zeroblob(32), 1)",
            [future],
        )
        .unwrap();
    external
        .pragma_update(None, "user_version", future)
        .unwrap();
    drop(external);

    let incompatible = SnapshotRepositoryErrorKind::History(HistoryErrorKind::IncompatibleSchema);
    // The current-schema database fence is acquired before snapshot-store
    // inventory. Even though the publication holds that later lock, the
    // newer schema wins as IncompatibleSchema rather than Busy.
    assert_eq!(
        repository
            .inspect_retention_inventory(observed_at)
            .unwrap_err()
            .kind,
        incompatible
    );
    drop(held_publication);
    assert_eq!(
        repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .err()
            .unwrap()
            .kind,
        incompatible
    );
    assert_eq!(
        lease
            .renew(observed_at + Duration::from_secs(1))
            .unwrap_err()
            .kind,
        incompatible
    );
    assert_eq!(lease.release().unwrap_err().kind, incompatible);

    let external = rusqlite::Connection::open(&database).unwrap();
    external
        .execute("DELETE FROM schema_migrations WHERE version = ?1", [future])
        .unwrap();
    external
        .pragma_update(None, "user_version", current)
        .unwrap();
    drop(external);
    store.with_connection(|connection| {
        connection
            .execute("DELETE FROM snapshot_review_pins", [])
            .unwrap();
    });
}

#[test]
fn review_acquisition_obeys_database_before_snapshot_lock_order() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let review_document = document("scan:snapshot-review-lock-order", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let observed_at = completed_at + Duration::from_secs(1);
    let (store, repository) = open_repository(&database);
    let reference = complete_snapshot(&store, &repository, &review_document, &root, completed_at);

    let orphan_root = temp.path().join("orphan-root");
    let orphan_document = document("scan:snapshot-review-lock-orphan", &orphan_root);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                orphan_document.metadata.scan_id.clone(),
                orphan_root,
                observed_at,
            )
            .unwrap(),
        )
        .unwrap();
    let publication = repository
        .publish_orphan_for_test(&orphan_document)
        .unwrap();
    assert_eq!(
        repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
            .err()
            .unwrap()
            .kind,
        SnapshotRepositoryErrorKind::Storage(SnapshotStorageErrorKind::Busy)
    );
    drop(publication);
    let lease = repository
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();
    lease.release().unwrap();
}

#[test]
fn file_first_completion_and_exact_retry_survive_reopen() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-complete", &root);
    let coverage = partial_coverage(&root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);

    let reference = {
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new_without_root_identity(
                    document.metadata.scan_id.clone(),
                    root.clone(),
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        let reference = repository
            .complete_scan(completed_at, counts(), &coverage, &document)
            .unwrap();
        assert_eq!(
            repository
                .complete_scan(completed_at, counts(), &coverage, &document)
                .unwrap(),
            reference
        );
        let durable = store
            .load_scan(&document.metadata.scan_id)
            .unwrap()
            .unwrap();
        assert_eq!(durable.snapshot(), Some(&reference));
        assert_eq!(durable.coverage(), &coverage);
        reference
    };

    let (store, reopened) = open_repository(&database);
    assert_eq!(reopened.load(&reference).unwrap(), document);
    assert_eq!(
        store
            .load_scan(&ScanId::new("scan:snapshot-complete").unwrap())
            .unwrap()
            .unwrap()
            .coverage(),
        &coverage
    );
}

#[test]
fn committed_tombstone_blocks_valid_snapshot_and_survives_reopen() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-tombstoned", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    let reference = repository
        .complete_scan(completed_at, counts(), &complete_coverage(), &document)
        .unwrap();
    let snapshot_path = database
        .parent()
        .unwrap()
        .join("snapshots")
        .join(reference.file_name().as_str());
    assert!(snapshot_path.is_file());
    let guard = store.lock_current_history_connection().unwrap();
    assert_eq!(
        repository.load_with_guard(&guard, &reference).unwrap(),
        document
    );
    drop(guard);

    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO snapshot_retention_tombstones (
                     scan_id, record_format_version, scan_status,
                     completed_at_unix_ms, snapshot_version,
                     snapshot_relative_path, snapshot_relative_path_encoding,
                     snapshot_checksum_sha256, committed_at_unix_ms
                 )
                 SELECT scan_id, 1, status, completed_at_unix_ms,
                        snapshot_version, snapshot_relative_path,
                        snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, completed_at_unix_ms + 1
                 FROM scans WHERE scan_id = ?1",
                [reference.scan_id().as_str()],
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    "UPDATE snapshot_retention_tombstones
                     SET committed_at_unix_ms = committed_at_unix_ms + 1
                     WHERE scan_id = ?1",
                    [reference.scan_id().as_str()],
                )
                .is_err()
        );
        assert!(
            connection
                .execute(
                    "DELETE FROM snapshot_retention_tombstones WHERE scan_id = ?1",
                    [reference.scan_id().as_str()],
                )
                .is_err()
        );
    });

    assert!(snapshot_path.is_file());
    assert_eq!(
        repository.load(&reference).unwrap_err().kind,
        SnapshotRepositoryErrorKind::SnapshotUnavailable
    );
    let guard = store.lock_current_history_connection().unwrap();
    assert_eq!(
        repository
            .load_with_guard(&guard, &reference)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::SnapshotUnavailable
    );
    drop(guard);
    let durable = store.load_scan(reference.scan_id()).unwrap().unwrap();
    assert_eq!(durable.snapshot(), Some(&reference));
    drop(repository);
    drop(store);

    let (reopened_store, reopened) = open_repository(&database);
    assert_eq!(
        reopened.load(&reference).unwrap_err().kind,
        SnapshotRepositoryErrorKind::SnapshotUnavailable
    );
    assert_eq!(
        reopened_store
            .load_scan(reference.scan_id())
            .unwrap()
            .unwrap()
            .snapshot(),
        Some(&reference)
    );
}

#[test]
fn tombstone_mismatched_from_parent_scan_fails_corrupt_before_opening_snapshot() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-hostile-tombstone", &root);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    let reference = repository
        .complete_scan(
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
            counts(),
            &complete_coverage(),
            &document,
        )
        .unwrap();
    store.with_connection(|connection| {
        connection
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        connection
            .execute(
                "INSERT INTO snapshot_retention_tombstones (
                     scan_id, record_format_version, scan_status,
                     completed_at_unix_ms, snapshot_version,
                     snapshot_relative_path, snapshot_relative_path_encoding,
                     snapshot_checksum_sha256, committed_at_unix_ms
                 )
                 SELECT scan_id, 1, status, completed_at_unix_ms + 1,
                        snapshot_version, snapshot_relative_path,
                        snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, completed_at_unix_ms + 2
                 FROM scans WHERE scan_id = ?1",
                [reference.scan_id().as_str()],
            )
            .unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
    });

    assert_eq!(
        repository.load(&reference).unwrap_err().kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
    );
}

#[test]
fn orphan_publication_is_adopted_only_after_exact_collision_validation() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-orphan", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root.clone(),
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();

    let orphan = repository.publish_orphan_for_test(&document).unwrap();
    orphan.revalidate().unwrap();
    let orphan_reference = orphan.reference().clone();
    drop(orphan);
    assert_eq!(temp_lease_count(&store), 1);
    assert_eq!(
        store
            .load_scan(&document.metadata.scan_id)
            .unwrap()
            .unwrap()
            .status(),
        ScanStatus::Running
    );
    let adopted = repository
        .complete_scan(completed_at, counts(), &complete_coverage(), &document)
        .unwrap();
    assert_eq!(adopted, orphan_reference);
    assert_eq!(temp_lease_count(&store), 0);
}

#[test]
fn same_scan_id_different_orphan_document_is_rejected_without_finishing_scan() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let original = document("scan:snapshot-collision", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                original.metadata.scan_id.clone(),
                root,
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    let orphan = repository.publish_orphan_for_test(&original).unwrap();
    let original_reference = orphan.reference().clone();
    drop(orphan);

    let mut different = original.clone();
    different.metadata.totals.logical_bytes = 11;
    different.metadata.totals.allocated_bytes = Some(17);
    different.nodes[0].logical_bytes = 11;
    different.nodes[0].allocated_bytes = Some(17);
    different.nodes[1].logical_bytes = 11;
    different.nodes[1].allocated_bytes = Some(17);

    assert_eq!(
        repository
            .complete_scan(
                completed_at,
                counts_for(&different),
                &complete_coverage(),
                &different,
            )
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::ReferenceMismatch
    );
    assert_eq!(
        store
            .load_scan(&original.metadata.scan_id)
            .unwrap()
            .unwrap()
            .status(),
        ScanStatus::Running
    );
    assert_eq!(repository.load(&original_reference).unwrap(), original);
}

#[test]
#[allow(clippy::disallowed_methods)]
fn durable_reference_fails_closed_when_snapshot_is_missing_or_corrupt() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-reference-failure", &root);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root.clone(),
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    let reference = repository
        .complete_scan(
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
            counts(),
            &complete_coverage(),
            &document,
        )
        .unwrap();
    let path = database
        .parent()
        .unwrap()
        .join("snapshots")
        .join(reference.file_name().as_str());
    std::fs::write(&path, b"corrupt").unwrap();
    assert!(matches!(
        repository.load(&reference).unwrap_err().kind,
        SnapshotRepositoryErrorKind::Codec(_)
    ));
    let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_003_000);
    let corrupt_lease = repository
        .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
        .unwrap();
    assert!(matches!(
        corrupt_lease.load(observed_at).unwrap_err().kind,
        SnapshotRepositoryErrorKind::Codec(_)
    ));
    corrupt_lease.release().unwrap();

    // DUX-DESTRUCTIVE: allow=test-snapshot-referenced-file-remove -- remove only this test-owned published fixture to prove a durable reference fails closed when its file disappears
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        repository.load(&reference).unwrap_err().kind,
        SnapshotRepositoryErrorKind::MissingSnapshot
    );
    assert_eq!(
        repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
            .err()
            .unwrap()
            .kind,
        SnapshotRepositoryErrorKind::MissingSnapshot
    );
    assert_eq!(review_pin_count(&store), 0);

    store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO snapshot_retention_tombstones (
                     scan_id, record_format_version, scan_status,
                     completed_at_unix_ms, snapshot_version,
                     snapshot_relative_path, snapshot_relative_path_encoding,
                     snapshot_checksum_sha256, committed_at_unix_ms
                 )
                 SELECT scan_id, 1, status, completed_at_unix_ms,
                        snapshot_version, snapshot_relative_path,
                        snapshot_relative_path_encoding,
                        snapshot_checksum_sha256, completed_at_unix_ms + 1
                 FROM scans WHERE scan_id = ?1",
                [reference.scan_id().as_str()],
            )
            .unwrap();
    });
    assert_eq!(
        repository.load(&reference).unwrap_err().kind,
        SnapshotRepositoryErrorKind::SnapshotUnavailable
    );
}

#[test]
fn missing_scan_and_mismatched_summary_publish_nothing() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-rejected", &root);
    let (store, repository) = open_repository(&database);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);

    assert_eq!(
        repository
            .complete_scan(completed_at, counts(), &complete_coverage(), &document)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::NotFound)
    );
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        repository
            .complete_scan(completed_at, counts(), &ScanCoverage::unknown(), &document)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::ReferenceMismatch
    );
    let outside_coverage = ScanCoverage::try_from_terminal(
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
    assert_eq!(
        repository
            .complete_scan(completed_at, counts(), &outside_coverage, &document)
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::ReferenceMismatch
    );
    assert_eq!(
        std::fs::read_dir(database.parent().unwrap().join("snapshots"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("snapshot-"))
            .count(),
        0
    );
}

#[test]
fn out_of_sqlite_range_counts_are_rejected_before_snapshot_publication() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let mut document = document("scan:snapshot-count-overflow", &root);
    let overflow = (i64::MAX as u64) + 1;
    document.metadata.totals.logical_bytes = overflow;
    document.nodes[0].logical_bytes = overflow;
    document.nodes[1].logical_bytes = overflow;
    let counts = counts_for(&document);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();

    assert_eq!(
        repository
            .complete_scan(
                UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
                counts,
                &complete_coverage(),
                &document,
            )
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
    );
    assert_eq!(
        std::fs::read_dir(database.parent().unwrap().join("snapshots"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("snapshot-"))
            .count(),
        0
    );
    assert_eq!(
        store
            .load_scan(&document.metadata.scan_id)
            .unwrap()
            .unwrap()
            .status(),
        ScanStatus::Running
    );
}

#[test]
fn scan_and_terminal_candidate_batch_commit_atomically_and_retry_exactly() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:evaluation-success", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let evaluation_completed_at = completed_at + Duration::from_millis(500);
    let terminal = CandidateEvaluationCompletion::succeeded(
        evaluation_completed_at,
        vec![candidate(
            &document.metadata.scan_id,
            "candidate:evaluation-success",
            &root.join("artifact.o"),
        )],
    )
    .unwrap();
    let identity = evaluation_identity(7);

    {
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new_without_root_identity(
                    document.metadata.scan_id.clone(),
                    root.clone(),
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        let reference = repository
            .complete_scan_with_candidate_evaluation(
                completed_at,
                counts(),
                &complete_coverage(),
                &document,
                &identity,
                &terminal,
            )
            .unwrap();
        assert_eq!(
            repository
                .complete_scan_with_candidate_evaluation(
                    completed_at,
                    counts(),
                    &complete_coverage(),
                    &document,
                    &identity,
                    &terminal,
                )
                .unwrap(),
            reference
        );
        assert_eq!(
            store
                .load_candidate_evaluation(&document.metadata.scan_id)
                .unwrap()
                .unwrap()
                .status(),
            super::super::candidate_evaluation_history::CandidateEvaluationStatus::Succeeded {
                candidate_count: 1
            }
        );
        assert!(matches!(
            store
                .load_candidate(&CandidateId::new("candidate:evaluation-success").unwrap())
                .unwrap(),
            Some(StoredCandidateRecord::Complete(_))
        ));
    }

    let (store, repository) = open_repository(&database);
    repository
        .complete_scan_with_candidate_evaluation(
            completed_at,
            counts(),
            &complete_coverage(),
            &document,
            &identity,
            &terminal,
        )
        .unwrap();
    assert_eq!(
        store
            .load_scan(&document.metadata.scan_id)
            .unwrap()
            .unwrap()
            .status(),
        ScanStatus::Succeeded
    );
    assert_eq!(
        repository
            .complete_scan_with_candidate_evaluation(
                completed_at,
                counts(),
                &complete_coverage(),
                &document,
                &evaluation_identity(9),
                &terminal,
            )
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::ReferenceMismatch
    );
}

#[test]
fn maximum_candidate_evaluation_batch_loads_after_reopen_within_query_budget() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:evaluation-maximum-batch", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let evaluation_completed_at = completed_at + Duration::from_millis(500);
    let candidates = (0..crate::domain::MAX_EVALUATED_CANDIDATES)
        .map(|index| {
            candidate(
                &document.metadata.scan_id,
                &format!("candidate:maximum:{index:04}"),
                &root.join(format!("artifact-{index:04}.o")),
            )
        })
        .collect();
    let terminal =
        CandidateEvaluationCompletion::succeeded(evaluation_completed_at, candidates).unwrap();

    {
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new_without_root_identity(
                    document.metadata.scan_id.clone(),
                    root,
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        repository
            .complete_scan_with_candidate_evaluation(
                completed_at,
                counts(),
                &complete_coverage(),
                &document,
                &evaluation_identity(8),
                &terminal,
            )
            .unwrap();
    }

    let (store, _repository) = open_repository(&database);
    let evaluation = store
        .load_candidate_evaluation(&document.metadata.scan_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        evaluation.status(),
        super::super::candidate_evaluation_history::CandidateEvaluationStatus::Succeeded {
            candidate_count: crate::domain::MAX_EVALUATED_CANDIDATES as u32,
        }
    );
    assert_eq!(
        evaluation.candidates().len(),
        crate::domain::MAX_EVALUATED_CANDIDATES
    );
}

#[test]
fn typed_evaluation_failure_is_terminal_and_contains_no_candidates() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:evaluation-failed", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let terminal = CandidateEvaluationCompletion::failed(
        completed_at + Duration::from_millis(1),
        super::super::candidate_evaluation_history::CandidateEvaluationFailureKind::Cancelled,
    )
    .unwrap();
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root.clone(),
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    repository
        .complete_scan_with_candidate_evaluation(
            completed_at,
            counts(),
            &complete_coverage(),
            &document,
            &evaluation_identity(11),
            &terminal,
        )
        .unwrap();
    let stored_evaluation = store
        .load_candidate_evaluation(&document.metadata.scan_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        stored_evaluation.status(),
        super::super::candidate_evaluation_history::CandidateEvaluationStatus::Failed {
            kind: super::super::candidate_evaluation_history::CandidateEvaluationFailureKind::Cancelled
        }
    );
    assert!(stored_evaluation.candidates().is_empty());
    assert_eq!(
        store
            .record_candidate_discovered(&candidate(
                &document.metadata.scan_id,
                "candidate:after-failed-evaluation",
                &root.join("artifact.o"),
            ))
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
}

#[test]
fn hostile_terminal_time_and_record_format_are_rejected_by_loader() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:evaluation-corrupt-row", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let terminal = CandidateEvaluationCompletion::succeeded(
        completed_at + Duration::from_millis(500),
        Vec::new(),
    )
    .unwrap();
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    repository
        .complete_scan_with_candidate_evaluation(
            completed_at,
            counts(),
            &complete_coverage(),
            &document,
            &evaluation_identity(12),
            &terminal,
        )
        .unwrap();

    store.with_connection(|connection| {
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE candidate_evaluations
                 SET completed_at_unix_ms = scheduled_at_unix_ms - 1
                 WHERE scan_id = ?1",
                [document.metadata.scan_id.as_str()],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });
    assert_eq!(
        store
            .load_candidate_evaluation(&document.metadata.scan_id)
            .unwrap_err()
            .kind,
        HistoryErrorKind::CorruptData
    );

    store.with_connection(|connection| {
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE candidate_evaluations
                 SET completed_at_unix_ms = ?2, record_format_version = 2
                 WHERE scan_id = ?1",
                rusqlite::params![document.metadata.scan_id.as_str(), 1_750_000_002_500_i64],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });
    assert_eq!(
        store
            .load_candidate_evaluation(&document.metadata.scan_id)
            .unwrap_err()
            .kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn candidate_created_time_must_match_terminal_evaluation_time() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:evaluation-candidate-time", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let evaluation_completed_at = completed_at + Duration::from_millis(500);
    let terminal = CandidateEvaluationCompletion::succeeded(
        evaluation_completed_at,
        vec![candidate(
            &document.metadata.scan_id,
            "candidate:evaluation-candidate-time",
            &root.join("artifact.o"),
        )],
    )
    .unwrap();
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    repository
        .complete_scan_with_candidate_evaluation(
            completed_at,
            counts(),
            &complete_coverage(),
            &document,
            &evaluation_identity(14),
            &terminal,
        )
        .unwrap();

    store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE candidates
                 SET created_at_unix_ms = created_at_unix_ms + 1
                 WHERE scan_id = ?1",
                [document.metadata.scan_id.as_str()],
            )
            .unwrap();
    });
    assert_eq!(
        store
            .load_candidate_evaluation(&document.metadata.scan_id)
            .unwrap_err()
            .kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn late_candidate_collision_rolls_back_scan_evaluation_and_earlier_candidate() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let prior_root = temp.path().join("prior-root");
    let prior = document("scan:evaluation-prior", &prior_root);
    let root = temp.path().join("scan-root");
    let target = document("scan:evaluation-rollback", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);

    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                prior.metadata.scan_id.clone(),
                prior_root.clone(),
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    repository
        .complete_scan(completed_at, counts(), &complete_coverage(), &prior)
        .unwrap();
    store
        .record_candidate_discovered(&candidate(
            &prior.metadata.scan_id,
            "candidate:collision",
            &prior_root.join("artifact.o"),
        ))
        .unwrap();

    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                target.metadata.scan_id.clone(),
                root.clone(),
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    let mismatched_time_terminal = CandidateEvaluationCompletion::succeeded(
        completed_at + Duration::from_millis(250),
        vec![candidate(
            &target.metadata.scan_id,
            "candidate:mismatched-evaluation-time",
            &root.join("mismatched-time.o"),
        )],
    )
    .unwrap();
    assert_eq!(
        repository
            .complete_scan_with_candidate_evaluation(
                completed_at,
                counts(),
                &complete_coverage(),
                &target,
                &evaluation_identity(13),
                &mismatched_time_terminal,
            )
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
    );
    let cross_scan_terminal = CandidateEvaluationCompletion::succeeded(
        completed_at + Duration::from_millis(250),
        vec![candidate(
            &prior.metadata.scan_id,
            "candidate:cross-scan",
            &root.join("cross-scan.o"),
        )],
    )
    .unwrap();
    assert_eq!(
        repository
            .complete_scan_with_candidate_evaluation(
                completed_at,
                counts(),
                &complete_coverage(),
                &target,
                &evaluation_identity(13),
                &cross_scan_terminal,
            )
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
    );
    let terminal = CandidateEvaluationCompletion::succeeded(
        completed_at + Duration::from_millis(500),
        vec![
            candidate(
                &target.metadata.scan_id,
                "candidate:inserted-first",
                &root.join("first.o"),
            ),
            candidate(
                &target.metadata.scan_id,
                "candidate:collision",
                &root.join("artifact.o"),
            ),
        ],
    )
    .unwrap();
    assert_eq!(
        repository
            .complete_scan_with_candidate_evaluation(
                completed_at,
                counts(),
                &complete_coverage(),
                &target,
                &evaluation_identity(13),
                &terminal,
            )
            .unwrap_err()
            .kind,
        SnapshotRepositoryErrorKind::History(HistoryErrorKind::AlreadyExists)
    );
    assert_eq!(
        store
            .load_scan(&target.metadata.scan_id)
            .unwrap()
            .unwrap()
            .status(),
        ScanStatus::Running
    );
    store.with_connection(|connection| {
        let population = inspect_snapshot_temp_leases(connection).unwrap();
        let lease = population
            .rows()
            .iter()
            .find(|lease| lease.scan_id() == &target.metadata.scan_id)
            .unwrap();
        assert_eq!(
            lease.parent_status(),
            super::super::snapshot_temp_lease::SnapshotTempLeaseParentStatus::Running
        );
    });
    let retention = repository
        .inspect_retention_inventory(completed_at)
        .unwrap();
    assert!(retention.temporary_files.is_empty());
    assert!(
        retention
            .residual_temp_leases
            .iter()
            .any(|lease| lease.scan_id == target.metadata.scan_id)
    );
    assert!(
        store
            .load_candidate_evaluation(&target.metadata.scan_id)
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .load_candidate(&CandidateId::new("candidate:inserted-first").unwrap())
            .unwrap()
            .is_none()
    );
}

#[test]
fn exact_post_commit_failure_reconciles_non_evaluation_completion_and_lease() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:snapshot-completion-ambiguous", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    let published = repository.publish_orphan_for_test(&document).unwrap();
    let completion = ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
        document.metadata.scan_id.clone(),
        completed_at,
        counts(),
        complete_coverage(),
        published.reference().clone(),
    )
    .unwrap();

    store
        .record_scan_finished_with_temp_lease_after_commit_failure_for_test(
            &completion,
            &published.temp_lease,
        )
        .unwrap();
    assert_eq!(temp_lease_count(&store), 0);
    assert!(
        store
            .load_scan(&document.metadata.scan_id)
            .unwrap()
            .unwrap()
            .exactly_matches_completion(&completion)
    );
    published.revalidate().unwrap();
}

#[test]
fn exact_post_commit_failure_reconciles_full_terminal_output() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:evaluation-ambiguous", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root,
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    let published = repository.publish_orphan_for_test(&document).unwrap();
    let completion = ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
        document.metadata.scan_id.clone(),
        completed_at,
        counts(),
        complete_coverage(),
        published.reference().clone(),
    )
    .unwrap();
    let request = NewCandidateEvaluation::try_new(
        evaluation_identity(15),
        published.reference(),
        completed_at,
    )
    .unwrap();
    let terminal = CandidateEvaluationCompletion::succeeded(
        completed_at + Duration::from_millis(1) + Duration::from_nanos(789),
        Vec::new(),
    )
    .unwrap();
    store
        .record_scan_finished_with_evaluation_and_temp_lease_after_commit_failure_for_test(
            &completion,
            &request,
            &terminal,
            &published.temp_lease,
        )
        .unwrap();
    assert_eq!(
        store
            .load_candidate_evaluation(&document.metadata.scan_id)
            .unwrap()
            .unwrap()
            .status(),
        super::super::candidate_evaluation_history::CandidateEvaluationStatus::Succeeded {
            candidate_count: 0
        }
    );
}

#[test]
fn pending_evaluation_loads_and_standalone_terminal_commit_reconciles() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("store/dux.sqlite3");
    let root = temp.path().join("scan-root");
    let document = document("scan:evaluation-pending", &root);
    let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
    let (store, repository) = open_repository(&database);
    store
        .record_scan_started(
            &NewScanRecord::try_new_without_root_identity(
                document.metadata.scan_id.clone(),
                root.clone(),
                UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
            )
            .unwrap(),
        )
        .unwrap();
    let published = repository.publish_orphan_for_test(&document).unwrap();
    let completion = ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
        document.metadata.scan_id.clone(),
        completed_at,
        counts(),
        complete_coverage(),
        published.reference().clone(),
    )
    .unwrap();
    let request = NewCandidateEvaluation::try_new(
        evaluation_identity(17),
        published.reference(),
        completed_at,
    )
    .unwrap();
    {
        let mut guard = store.lock_current_history_connection().unwrap();
        store
            .record_scan_finished_and_schedule_evaluation_with_temp_lease_reconciled_with_guard(
                &mut guard,
                &completion,
                &request,
                &published.temp_lease,
            )
            .unwrap();
    }
    assert_eq!(
        store
            .load_candidate_evaluation(&document.metadata.scan_id)
            .unwrap()
            .unwrap()
            .status(),
        super::super::candidate_evaluation_history::CandidateEvaluationStatus::Pending
    );
    assert_eq!(
        store
            .record_candidate_discovered(&candidate(
                &document.metadata.scan_id,
                "candidate:while-pending",
                &root.join("artifact.o"),
            ))
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
    let terminal = CandidateEvaluationCompletion::succeeded(
        completed_at + Duration::from_millis(500),
        vec![candidate(
            &document.metadata.scan_id,
            "candidate:pending",
            &root.join("artifact.o"),
        )],
    )
    .unwrap();
    store
        .record_candidate_evaluation_completed_after_commit_failure_for_test(&request, &terminal)
        .unwrap();
    assert_eq!(
        store
            .load_candidate_evaluation(&document.metadata.scan_id)
            .unwrap()
            .unwrap()
            .status(),
        super::super::candidate_evaluation_history::CandidateEvaluationStatus::Succeeded {
            candidate_count: 1
        }
    );
    assert_eq!(
        store
            .record_candidate_discovered(&candidate(
                &document.metadata.scan_id,
                "candidate:after-succeeded-evaluation",
                &root.join("other.o"),
            ))
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidTransition
    );
}
