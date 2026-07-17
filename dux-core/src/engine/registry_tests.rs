use std::ffi::OsStr;
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

#[test]
fn explorer_review_facade_is_scan_bound_expiring_and_idempotently_released() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("payload"), b"snapshot review").unwrap();
    let engine = EngineHandle::open(config(&temp)).unwrap();

    assert!(matches!(
        engine.acquire_explorer_snapshot_review(&ScanId::new("scan:missing").unwrap()),
        Err(SnapshotReviewError::ScanNotFound)
    ));

    let running_id = ScanId::new("scan:running-review").unwrap();
    let running =
        NewScanRecord::try_new(running_id.clone(), root.clone(), SystemTime::now()).unwrap();
    engine
        .inner
        .store
        .record_scan_started_reconciled(&running)
        .unwrap();
    assert!(matches!(
        engine.acquire_explorer_snapshot_review(&running_id),
        Err(SnapshotReviewError::SnapshotUnavailable)
    ));

    let task = engine.start_scan(root).unwrap();
    assert_eq!(wait_terminal(&engine, task).phase, TaskPhase::Succeeded);
    let scan_id = engine.scan_result(task).unwrap().unwrap().scan_id().clone();
    let mut review = engine.acquire_explorer_snapshot_review(&scan_id).unwrap();
    assert_eq!(review.scan_id(), &scan_id);
    let expiry = review.expires_at().unwrap();
    assert_eq!(
        review.renew_at_for_test(expiry).unwrap_err(),
        SnapshotReviewError::LeaseExpired
    );
    assert_eq!(
        review.release().unwrap(),
        super::super::snapshot_review::SnapshotReviewReleaseOutcome::Released
    );
    assert!(review.is_released());
    assert_eq!(
        review.release().unwrap(),
        super::super::snapshot_review::SnapshotReviewReleaseOutcome::AlreadyReleased
    );

    engine.close();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert!(matches!(
        engine.acquire_explorer_snapshot_review(&scan_id),
        Err(SnapshotReviewError::Closed)
    ));
}

#[test]
fn latest_explorer_review_selects_exact_newest_available_snapshot_and_skips_tombstones() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("latest-review-root");
    std::fs::create_dir(&root).unwrap();
    let engine = EngineHandle::open(config(&temp)).unwrap();

    assert!(matches!(
        engine.acquire_latest_explorer_snapshot_review(),
        Err(SnapshotReviewError::SnapshotUnavailable)
    ));

    for index in 0..2 {
        std::fs::write(root.join("payload"), format!("latest review {index}")).unwrap();
        let task = engine.start_scan(root.clone()).unwrap();
        assert_eq!(wait_terminal(&engine, task).phase, TaskPhase::Succeeded);
        assert!(engine.scan_result(task).unwrap().is_some());
        std::thread::sleep(Duration::from_millis(2));
    }
    let history = engine.recent_scan_history(2).unwrap();
    let expected_newest = history.scans[0].scan_id.clone();
    let expected_older = history.scans[1].scan_id.clone();
    assert_ne!(expected_newest, expected_older);

    let mut latest = engine.acquire_latest_explorer_snapshot_review().unwrap();
    assert_eq!(latest.scan_id(), &expected_newest);
    assert_eq!(
        latest.release().unwrap(),
        super::super::snapshot_review::SnapshotReviewReleaseOutcome::Released
    );

    engine.inner.store.with_connection(|connection| {
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
                        snapshot_relative_path_encoding, snapshot_checksum_sha256,
                        completed_at_unix_ms + 1
                 FROM scans WHERE scan_id = ?1",
                [expected_newest.as_str()],
            )
            .unwrap();
    });

    let mut fallback = engine.acquire_latest_explorer_snapshot_review().unwrap();
    assert_eq!(fallback.scan_id(), &expected_older);
    assert_eq!(
        fallback.release().unwrap(),
        super::super::snapshot_review::SnapshotReviewReleaseOutcome::Released
    );

    engine.close();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert!(matches!(
        engine.acquire_latest_explorer_snapshot_review(),
        Err(SnapshotReviewError::Closed)
    ));
}

#[test]
fn explorer_review_rejects_tombstoned_snapshot_through_path_free_facade() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine = EngineHandle::open(config(&temp)).unwrap();
    let mut scan_ids = Vec::new();
    for index in 0..3 {
        std::fs::write(root.join("payload"), format!("snapshot review {index}")).unwrap();
        let task = engine.start_scan(root.clone()).unwrap();
        assert_eq!(wait_terminal(&engine, task).phase, TaskPhase::Succeeded);
        scan_ids.push(engine.scan_result(task).unwrap().unwrap().scan_id().clone());
    }
    let tombstoned = scan_ids.remove(0);
    engine.set_snapshot_retention_cap(0).unwrap();
    let retention = started_snapshot_retention(
        engine
            .start_snapshot_retention_at(SystemTime::now() + Duration::from_secs(60))
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, retention).phase,
        TaskPhase::Succeeded
    );
    assert!(matches!(
        engine
            .snapshot_retention_result(retention)
            .unwrap()
            .unwrap()
            .outcome(),
        SnapshotRetentionOutcome::TombstonedAndRemoved { bytes } if bytes > 0
    ));
    assert!(
        engine
            .inner
            .store
            .load_scan(&tombstoned)
            .unwrap()
            .unwrap()
            .snapshot()
            .is_some()
    );
    assert!(matches!(
        engine.acquire_explorer_snapshot_review(&tombstoned),
        Err(SnapshotReviewError::SnapshotUnavailable)
    ));
}

#[test]
fn close_wins_inflight_review_acquisition_without_leaking_a_pin() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("payload"), b"snapshot review").unwrap();
    let engine = EngineHandle::open(config(&temp)).unwrap();
    let task = engine.start_scan(root).unwrap();
    assert_eq!(wait_terminal(&engine, task).phase, TaskPhase::Succeeded);
    let scan_id = engine.scan_result(task).unwrap().unwrap().scan_id().clone();
    let (preflight_tx, preflight_rx) = mpsc::channel();
    let (continue_tx, continue_rx) = mpsc::channel();
    let acquiring = engine.clone();
    let acquisition = std::thread::spawn(move || {
        acquiring.acquire_explorer_snapshot_review_with_test_hook(&scan_id, move || {
            preflight_tx.send(()).unwrap();
            continue_rx.recv().unwrap();
        })
    });
    preflight_rx.recv_timeout(TEST_TIMEOUT).unwrap();

    assert_eq!(engine.close(), CloseOutcome::Initiated);
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    continue_tx.send(()).unwrap();
    assert!(matches!(
        acquisition.join().unwrap(),
        Err(SnapshotReviewError::Closed)
    ));
    let pin_count = engine.inner.store.with_connection(|connection| {
        connection
            .query_row("SELECT count(*) FROM snapshot_review_pins", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    });
    assert_eq!(pin_count, 0);
}

#[test]
fn acquired_review_remains_renewable_and_releasable_after_engine_close() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("payload"), b"snapshot review").unwrap();
    let engine = EngineHandle::open(config(&temp)).unwrap();
    let task = engine.start_scan(root).unwrap();
    assert_eq!(wait_terminal(&engine, task).phase, TaskPhase::Succeeded);
    let scan_id = engine.scan_result(task).unwrap().unwrap().scan_id().clone();
    let mut review = engine.acquire_explorer_snapshot_review(&scan_id).unwrap();
    let expiry = review.expires_at().unwrap();

    assert_eq!(engine.close(), CloseOutcome::Initiated);
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert!(
        review
            .renew_at_for_test(expiry - Duration::from_millis(1))
            .is_ok()
    );
    assert_eq!(
        review.release().unwrap(),
        super::super::snapshot_review::SnapshotReviewReleaseOutcome::Released
    );
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

fn started_scan_recovery_maintenance(outcome: ScanRecoveryMaintenanceStartOutcome) -> TaskId {
    match outcome {
        ScanRecoveryMaintenanceStartOutcome::Started(id) => id,
        other => panic!("expected started scan recovery maintenance, got {other:?}"),
    }
}

fn started_snapshot_retention(outcome: SnapshotRetentionStartOutcome) -> TaskId {
    match outcome {
        SnapshotRetentionStartOutcome::Started(id) => id,
        other => panic!("expected started snapshot retention, got {other:?}"),
    }
}

fn started_snapshot_orphan_maintenance(outcome: SnapshotOrphanMaintenanceStartOutcome) -> TaskId {
    match outcome {
        SnapshotOrphanMaintenanceStartOutcome::Started(id) => id,
        other => panic!("expected started snapshot orphan maintenance, got {other:?}"),
    }
}

fn started_snapshot_provisioning_stage_maintenance(
    outcome: SnapshotProvisioningStageMaintenanceStartOutcome,
) -> TaskId {
    match outcome {
        SnapshotProvisioningStageMaintenanceStartOutcome::Started(id) => id,
        other => panic!("expected started snapshot provisioning-stage maintenance, got {other:?}"),
    }
}

fn started_snapshot_terminal_temp_maintenance(
    outcome: SnapshotTerminalTempMaintenanceStartOutcome,
) -> TaskId {
    match outcome {
        SnapshotTerminalTempMaintenanceStartOutcome::Started(id) => id,
        other => panic!("expected started snapshot terminal-temp maintenance, got {other:?}"),
    }
}

fn started_snapshot_unleased_temp_maintenance(
    outcome: SnapshotUnleasedTempMaintenanceStartOutcome,
) -> TaskId {
    match outcome {
        SnapshotUnleasedTempMaintenanceStartOutcome::Started(id) => id,
        other => panic!("expected started snapshot unleased-temp maintenance, got {other:?}"),
    }
}

fn publish_running_orphan(
    engine: &EngineHandle,
    root: &std::path::Path,
    scan_id: &str,
    started_at: SystemTime,
) {
    use crate::persistence::snapshot::{
        SnapshotDocument, SnapshotMetadata, SnapshotNode, SnapshotNodeKind, SnapshotScanFlags,
        SnapshotTimestamp, SnapshotTotals, SnapshotUnixIdentity,
    };

    let scan_id = ScanId::new(scan_id).unwrap();
    let document = SnapshotDocument {
        metadata: SnapshotMetadata {
            scan_id: scan_id.clone(),
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
                unix_identity: cfg!(unix).then(|| SnapshotUnixIdentity::new(7, 10)),
            },
            SnapshotNode {
                id: 1,
                parent: Some(0),
                depth: 1,
                kind: SnapshotNodeKind::File,
                name: Some(HostValue::from_component(OsStr::new("orphan.bin")).unwrap()),
                logical_bytes: 10,
                allocated_bytes: Some(16),
                file_count: 1,
                child_count: 0,
                modified_at: None,
                accessed_at: None,
                scan_flags: SnapshotScanFlags::NONE,
                unix_identity: cfg!(unix).then(|| SnapshotUnixIdentity::new(7, 11)),
            },
        ],
    };
    engine
        .inner
        .store
        .record_scan_started(
            &NewScanRecord::try_new(scan_id, root.to_path_buf(), started_at).unwrap(),
        )
        .unwrap();
    drop(
        engine
            .inner
            .snapshots
            .publish_orphan_for_test(&document)
            .unwrap(),
    );
}

fn leave_terminal_temp_residual(
    engine: &EngineHandle,
    root: &std::path::Path,
    scan_id: &str,
    started_at: SystemTime,
    status: TerminalScanStatus,
    create_file: bool,
) -> ScanId {
    use crate::persistence::snapshot::{
        SnapshotDocument, SnapshotMetadata, SnapshotNode, SnapshotNodeKind, SnapshotScanFlags,
        SnapshotTimestamp, SnapshotTotals, SnapshotUnixIdentity,
    };

    let scan_id = ScanId::new(scan_id).unwrap();
    let document = SnapshotDocument {
        metadata: SnapshotMetadata {
            scan_id: scan_id.clone(),
            root: HostValue::from_root(root).unwrap(),
            captured_at: SnapshotTimestamp::new(1_750_000_000, 123).unwrap(),
            totals: SnapshotTotals {
                directory_count: 1,
                file_count: 0,
                logical_bytes: 0,
                allocated_bytes: Some(0),
            },
        },
        nodes: vec![SnapshotNode {
            id: 0,
            parent: None,
            depth: 0,
            kind: SnapshotNodeKind::Directory,
            name: None,
            logical_bytes: 0,
            allocated_bytes: Some(0),
            file_count: 0,
            child_count: 0,
            modified_at: None,
            accessed_at: None,
            scan_flags: SnapshotScanFlags::NONE,
            unix_identity: cfg!(unix).then(|| SnapshotUnixIdentity::new(7, 10)),
        }],
    };
    engine
        .inner
        .store
        .record_scan_started(
            &NewScanRecord::try_new(scan_id.clone(), root.to_path_buf(), started_at).unwrap(),
        )
        .unwrap();
    engine
        .inner
        .snapshots
        .leave_snapshot_temp_residual_for_test(&document, create_file)
        .unwrap();
    engine
        .inner
        .store
        .record_scan_finished(
            &ScanCompletionRecord::try_new(
                scan_id.clone(),
                started_at + Duration::from_millis(1),
                status,
                ScanCounts::default(),
            )
            .unwrap(),
        )
        .unwrap();
    scan_id
}

fn publish_snapshots(engine: &EngineHandle, root: &std::path::Path, count: usize) {
    for index in 0..count {
        std::fs::write(root.join("payload"), format!("snapshot-{index}")).unwrap();
        let task = engine.start_scan(root.to_path_buf()).unwrap();
        assert_eq!(wait_terminal(engine, task).phase, TaskPhase::Succeeded);
    }
}

fn over_budget_candidate_records(
    scan_id: &ScanId,
    completed_at: SystemTime,
) -> Vec<NewCandidateRecord> {
    use crate::domain::{
        Candidate, CandidateInput, LocalizedTextKey, ProvenanceUrl, Rule, RuleDefinition,
        RuleGuards, RuleMatcher, RuleMatcherDefinition, RuleScope,
    };

    let matcher = RuleMatcher::try_new(RuleMatcherDefinition {
        path_component: Some("fixture".to_owned()),
        required_ancestor_markers_any: Vec::new(),
        required_markers_all: Vec::new(),
        forbidden_markers_any: Vec::new(),
        exact_bundle_identifiers: Vec::new(),
        excluded_descendants: Vec::new(),
        protected_descendants: Vec::new(),
    })
    .unwrap();
    let rule = Rule::try_new(RuleDefinition {
        reference: crate::RuleRef::new(
            crate::RuleId::new("fixture.over-budget").unwrap(),
            crate::RuleRevision::new(1).unwrap(),
        ),
        title_key: LocalizedTextKey::new("fixture.over_budget.title").unwrap(),
        category: crate::CandidateCategory::DeveloperArtifact,
        scope: RuleScope::ConfiguredProjectRoots,
        matcher,
        guards: RuleGuards::try_new(None, 0, Vec::new(), false).unwrap(),
        safety: crate::SafetyTier::Informational,
        action: crate::CandidateAction::RevealOnly,
        schedule_eligible: false,
        explanation_key: LocalizedTextKey::new("fixture.over_budget.explanation").unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/over-budget").unwrap()],
    })
    .unwrap();
    let long_path = |fill: char, index: usize| {
        #[cfg(windows)]
        let value = format!("C:\\{}{:07}", fill.to_string().repeat(32_758), index);
        #[cfg(not(windows))]
        let value = format!("/{}{:07}", fill.to_string().repeat(32_760), index);
        PathBuf::from(value)
    };
    let paths = (0..256)
        .map(|index| long_path('p', index))
        .collect::<Vec<_>>();
    let evidence = (0..100)
        .map(|index| crate::Evidence::RequiredMarker {
            path: long_path('e', index),
        })
        .collect::<Vec<_>>();
    let candidate = Candidate::try_from_rule(
        &rule,
        CandidateInput::new(
            crate::CandidateId::new("candidate:over-budget").unwrap(),
            paths,
            1,
            None,
            evidence,
            Vec::new(),
            scan_id.clone(),
        ),
    )
    .unwrap();
    vec![NewCandidateRecord::try_from_candidate(&candidate, completed_at).unwrap()]
}

fn completed_marker_candidate() -> (
    TempDir,
    EngineConfig,
    EngineHandle,
    ScanId,
    crate::CandidateId,
    PathBuf,
) {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    let project = root.join("project");
    let target = project.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(project.join("Cargo.toml"), b"[package]\nname='fixture'\n").unwrap();
    std::fs::write(target.join("object"), vec![7_u8; 8 * 1024]).unwrap();
    let expected_target = target.canonicalize().unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 4, 8, 16))
            .unwrap();
    let task = engine.start_scan(root).unwrap();
    assert_eq!(wait_terminal(&engine, task).phase, TaskPhase::Succeeded);
    let scan_id = engine.scan_result(task).unwrap().unwrap().scan_id().clone();
    let history = engine.candidate_history_for_scan(&scan_id).unwrap();
    assert_eq!(history.candidates().len(), 1);
    let candidate_id = history.candidates()[0].id().clone();
    (temp, config, engine, scan_id, candidate_id, expected_target)
}

fn make_marker_candidate_cleanup_reviewable(engine: &EngineHandle, scan_id: &ScanId) {
    engine.inner.store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE candidates
                 SET safety_tier = 'safe_regenerable',
                     proposed_action = 'remove_known_regenerable_contents',
                     rule_schedule_eligible = 1
                 WHERE scan_id = ?1",
                [scan_id.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM candidate_blockers
                 WHERE candidate_id = (
                     SELECT candidate_id FROM candidates WHERE scan_id = ?1
                 )",
                [scan_id.as_str()],
            )
            .unwrap();
    });
}

fn record_planned_cleanup_history(
    engine: &EngineHandle,
    scan_id: &ScanId,
    candidate_id: &crate::CandidateId,
    session: &str,
) {
    use crate::domain::{
        Candidate, CandidateInput, CleanupMode, CleanupPlan, CleanupPlanId, LocalizedTextKey,
        ProvenanceUrl, Rule, RuleDefinition, RuleGuards, RuleMatcher, RuleMatcherDefinition,
        RuleScope,
    };

    make_marker_candidate_cleanup_reviewable(engine, scan_id);
    let StoredCandidateRecord::Complete(stored) = engine
        .inner
        .store
        .load_candidate(candidate_id)
        .unwrap()
        .unwrap()
    else {
        panic!("expected complete candidate");
    };
    let matcher = RuleMatcher::try_new(RuleMatcherDefinition {
        path_component: Some("target".to_owned()),
        required_ancestor_markers_any: Vec::new(),
        required_markers_all: Vec::new(),
        forbidden_markers_any: Vec::new(),
        exact_bundle_identifiers: Vec::new(),
        excluded_descendants: Vec::new(),
        protected_descendants: Vec::new(),
    })
    .unwrap();
    let rule = Rule::try_new(RuleDefinition {
        reference: stored.rule().clone(),
        title_key: LocalizedTextKey::new("fixture.cleanup_history.title").unwrap(),
        category: stored.category(),
        scope: RuleScope::ConfiguredProjectRoots,
        matcher,
        guards: RuleGuards::try_new(None, 0, Vec::new(), false).unwrap(),
        safety: stored.safety(),
        action: stored.action(),
        schedule_eligible: stored.rule_schedule_eligible(),
        explanation_key: LocalizedTextKey::new("fixture.cleanup_history.explanation").unwrap(),
        provenance: vec![ProvenanceUrl::new("https://example.com/cleanup-history").unwrap()],
    })
    .unwrap();
    let candidate = Candidate::try_from_rule(
        &rule,
        CandidateInput::new(
            stored.id().clone(),
            stored.paths().to_vec(),
            stored.estimated_bytes(),
            stored.newest_mtime(),
            stored.evidence().to_vec(),
            stored.blockers().to_vec(),
            stored.source_scan_id().clone(),
        ),
    )
    .unwrap();
    let started_at = std::time::UNIX_EPOCH + Duration::from_millis(1_800_000_001_000);
    let plan = CleanupPlan::try_from_candidates_for_persistence_test(
        CleanupPlanId::new(format!("plan:{session}")).unwrap(),
        started_at - Duration::from_secs(1),
        CleanupMode::PermanentSafe,
        &[candidate],
    )
    .unwrap();
    engine
        .inner
        .store
        .record_cleanup_session_planned(
            &NewCleanupSessionRecord::try_from_plan(
                CleanupSessionId::new(session).unwrap(),
                &plan,
                started_at,
                CleanupTrigger::Manual,
            )
            .unwrap(),
        )
        .unwrap();
}

fn insert_legacy_cleanup_history(
    engine: &EngineHandle,
    session: &str,
    started_at_unix_ms: i64,
    status: &str,
    item_status: &str,
    error_category: Option<&str>,
) {
    engine.inner.store.with_connection(|connection| {
        connection
            .execute(
                "INSERT INTO cleanup_sessions (
                     session_id, plan_id, started_at_unix_ms,
                     completed_at_unix_ms, mode, estimated_bytes,
                     verified_capacity_delta_bytes, trigger_source, status,
                     record_format_version, candidate_status_coupling_version
                 ) VALUES (?1, ?2, ?3, ?4, 'permanent_safe', 10, 7,
                           'manual', ?5, 1, 1)",
                rusqlite::params![
                    session,
                    format!("plan:{session}"),
                    started_at_unix_ms,
                    (status != "planned").then_some(started_at_unix_ms + 1),
                    status,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO cleanup_items (
                     session_id, item_ordinal, rule_id, rule_revision,
                     estimated_bytes, final_status, error_category,
                     record_format_version, legacy_target_path,
                     legacy_target_path_encoding
                 ) VALUES (?1, 0, 'fixture.legacy', 1, 10, ?2, ?3, 1, ?4, 1)",
                rusqlite::params![
                    session,
                    item_status,
                    error_category,
                    b"relative/legacy-target".as_slice(),
                ],
            )
            .unwrap();
    });
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
        engine.start_snapshot_retention(),
        Err(StartTaskError::ReadOnlyStore)
    );
    assert_eq!(
        engine.recent_scan_history(1),
        Err(ScanHistoryError::IncompatibleSchema)
    );
    assert!(matches!(
        engine.acquire_explorer_snapshot_review(&ScanId::new("scan:future-review-schema").unwrap()),
        Err(SnapshotReviewError::IncompatibleSchema)
    ));
    assert_eq!(
        engine.candidate_history_for_scan(&ScanId::new("scan:future-schema").unwrap()),
        Err(CandidateHistoryError::IncompatibleSchema)
    );
    let future_scan = ScanId::new("scan:future-schema").unwrap();
    let future_candidate = crate::CandidateId::new("candidate:future-schema").unwrap();
    assert_eq!(
        engine.candidate_path_page(&future_scan, &future_candidate, 0, 1),
        Err(CandidateDetailError::IncompatibleSchema)
    );
    assert_eq!(
        engine.review_candidate(
            &future_scan,
            &future_candidate,
            CandidateReviewCommand::Dismiss,
        ),
        Err(CandidateReviewError::IncompatibleSchema)
    );
    assert_eq!(
        engine.recent_cleanup_history(None, 1),
        Err(CleanupHistoryError::IncompatibleSchema)
    );
    assert_eq!(
        engine.cleanup_session_history(
            &DurableCleanupSessionId::new("session:future-schema").unwrap()
        ),
        Err(CleanupHistoryError::IncompatibleSchema)
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
fn candidate_history_distinguishes_missing_scan_and_closed_engine() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    let missing = ScanId::new("scan:missing-candidate-history").unwrap();

    assert_eq!(
        engine.candidate_history_for_scan(&missing),
        Err(CandidateHistoryError::ScanNotFound)
    );

    engine.close();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(
        engine.candidate_history_for_scan(&missing),
        Err(CandidateHistoryError::Closed)
    );
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
    engine.inner.store.with_connection(|connection| {
        for index in 0..=crate::persistence::MAX_RECENT_SCAN_HISTORY_LIMIT {
            let path = root.join(format!("root-{index:03}"));
            let observed = crate::persistence::observe_host_path(&path).unwrap();
            let encoding = match observed.encoding() {
                crate::persistence::HostPathObservationEncoding::Utf8 => 1_i64,
                crate::persistence::HostPathObservationEncoding::Utf16LittleEndian => 2_i64,
            };
            connection
                .execute(
                    "INSERT INTO scans (
                         scan_id, root_path, root_path_encoding, started_at_unix_ms,
                         status, coverage_status
                     ) VALUES (?1, ?2, ?3, ?4, 'running', 'unknown')",
                    rusqlite::params![
                        format!("scan:history-limit:{index:03}"),
                        observed.bytes(),
                        encoding,
                        1_750_000_000_000_i64 + index as i64,
                    ],
                )
                .unwrap();
        }
    });

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
fn scan_recovery_maintenance_runs_one_typed_path_free_noop_batch() {
    let (temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(9_000);
    let id = started_scan_recovery_maintenance(
        engine.start_scan_recovery_maintenance_at(observed).unwrap(),
    );
    let terminal = wait_terminal(&engine, id);
    assert_eq!(terminal.kind, TaskKind::ScanRecoveryMaintenance);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.result_available);

    let result = engine
        .scan_recovery_maintenance_result(id)
        .unwrap()
        .unwrap();
    assert_eq!(result.observed_at(), observed);
    assert_eq!(result.outcome(), ScanRecoveryMaintenanceOutcome::NoClaim);
    assert_eq!(result.claimed_count_before(), 0);
    assert_eq!(result.claimed_count_after(), 0);
    assert_eq!(result.alive_count(), 0);
    assert_eq!(result.unknown_count(), 0);
    assert_eq!(result.recoverable_count(), 0);
    assert!(!result.has_more());
    assert_eq!(
        engine.history_maintenance_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.scan_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );

    let events = engine.task_events(id, 0, 8).unwrap().events;
    let applying = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::ScanRecoveryMaintenanceBatchApplying
            )
        })
        .unwrap();
    let finished = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::ScanRecoveryMaintenanceBatchFinished {
                    outcome: ScanRecoveryMaintenanceOutcome::NoClaim,
                    claimed_count_before: 0,
                    claimed_count_after: 0,
                    alive_count: 0,
                    unknown_count: 0,
                    recoverable_count: 0,
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

    let debug = format!("{result:?}{events:?}");
    assert!(!debug.contains(temp.path().to_string_lossy().as_ref()));
    assert!(!debug.contains("scan:"));
    assert!(!debug.contains("owner_process_instance"));
}

#[test]
fn scan_recovery_maintenance_preserves_changed_race_counts_and_has_more() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(9_001);
    let id = started_scan_recovery_maintenance(
        engine
            .start_scan_recovery_maintenance_with_test_batch(
                observed,
                crate::persistence::ScanRecoveryBatchResult {
                    observed_at: observed,
                    outcome: crate::persistence::ScanRecoveryBatchOutcome::ChangedConcurrently,
                    claimed_count_before: 5,
                    claimed_count_after: 4,
                    alive_count: 1,
                    unknown_count: 1,
                    recoverable_count: 3,
                    has_more: true,
                },
            )
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, id).phase, TaskPhase::Succeeded);
    let result = engine
        .scan_recovery_maintenance_result(id)
        .unwrap()
        .unwrap();
    assert_eq!(
        result.outcome(),
        ScanRecoveryMaintenanceOutcome::ChangedConcurrently
    );
    assert_eq!(result.claimed_count_before(), 5);
    assert_eq!(result.claimed_count_after(), 4);
    assert_eq!(result.alive_count(), 1);
    assert_eq!(result.unknown_count(), 1);
    assert_eq!(result.recoverable_count(), 3);
    assert!(result.has_more());
}

#[test]
fn scan_recovery_maintenance_is_idle_deduplicated_and_cross_exclusive() {
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
    assert_eq!(
        engine.start_scan_recovery_maintenance().unwrap(),
        ScanRecoveryMaintenanceStartOutcome::DeferredBusy
    );
    foreground_release_tx.send(()).unwrap();
    wait_terminal(&engine, foreground);

    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first = started_scan_recovery_maintenance(
        engine
            .start_scan_recovery_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(9_002),
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_scan_recovery_maintenance().unwrap(),
        ScanRecoveryMaintenanceStartOutcome::AlreadyActive(first)
    );
    assert_eq!(
        engine.start_history_maintenance().unwrap(),
        HistoryMaintenanceStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.start_snapshot_terminal_temp_maintenance().unwrap(),
        SnapshotTerminalTempMaintenanceStartOutcome::DeferredBusy
    );
    release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, first).phase, TaskPhase::Succeeded);
    assert!(
        engine
            .inner
            .shared
            .lock_registry_recover()
            .active_scan_recovery_maintenance
            .is_none()
    );
}

#[test]
fn scan_recovery_cancellation_and_close_are_linearized_at_applying() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 8, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(9_003);

    let (before_tx, before_rx) = mpsc::channel();
    let (before_release_tx, before_release_rx) = mpsc::channel();
    let cancelled = started_scan_recovery_maintenance(
        engine
            .start_scan_recovery_maintenance_with_test_hooks(
                observed,
                move || {
                    before_tx.send(()).unwrap();
                    before_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    before_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(cancelled).unwrap(),
        CancelOutcome::Requested
    );
    before_release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&engine, cancelled).phase,
        TaskPhase::Cancelled
    );
    assert!(
        engine
            .task_events(cancelled, 0, 8)
            .unwrap()
            .events
            .iter()
            .all(|event| !matches!(
                event.kind,
                TaskEventKind::ScanRecoveryMaintenanceBatchApplying
            ))
    );

    let (applying_tx, applying_rx) = mpsc::channel();
    let (applying_release_tx, applying_release_rx) = mpsc::channel();
    let applied = started_scan_recovery_maintenance(
        engine
            .start_scan_recovery_maintenance_with_test_hooks(
                observed + Duration::from_millis(1),
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    applying_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(engine.close(), CloseOutcome::Initiated);
    applying_release_tx.send(()).unwrap();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    let registry = engine.inner.shared.lock_registry_recover();
    let record = registry.records.get(&applied).unwrap();
    assert_eq!(record.phase, TaskPhase::Succeeded);
    assert!(record.cancellation_requested);
    assert!(matches!(
        record.result,
        Some(TaskResult::ScanRecoveryMaintenance(_))
    ));
}

#[test]
fn scan_recovery_mappings_are_exhaustive_and_stable() {
    for (input, expected) in [
        (
            crate::persistence::ScanRecoveryBatchOutcome::NoClaim,
            ScanRecoveryMaintenanceOutcome::NoClaim,
        ),
        (
            crate::persistence::ScanRecoveryBatchOutcome::DeferredUnproven,
            ScanRecoveryMaintenanceOutcome::DeferredUnproven,
        ),
        (
            crate::persistence::ScanRecoveryBatchOutcome::Interrupted,
            ScanRecoveryMaintenanceOutcome::Interrupted,
        ),
        (
            crate::persistence::ScanRecoveryBatchOutcome::ChangedConcurrently,
            ScanRecoveryMaintenanceOutcome::ChangedConcurrently,
        ),
    ] {
        assert_eq!(public_scan_recovery_maintenance_outcome(&input), expected);
    }

    for (input, expected) in [
        (
            HistoryErrorKind::InvalidInput,
            ScanRecoveryMaintenanceFailureKind::InvalidClock,
        ),
        (
            HistoryErrorKind::IncompatibleSchema,
            ScanRecoveryMaintenanceFailureKind::IncompatibleSchema,
        ),
        (
            HistoryErrorKind::QueryLimitExceeded,
            ScanRecoveryMaintenanceFailureKind::BudgetExceeded,
        ),
        (
            HistoryErrorKind::Busy,
            ScanRecoveryMaintenanceFailureKind::Busy,
        ),
        (
            HistoryErrorKind::UnsafeStorage,
            ScanRecoveryMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            HistoryErrorKind::CorruptData,
            ScanRecoveryMaintenanceFailureKind::CorruptData,
        ),
        (
            HistoryErrorKind::DatabaseUnavailable,
            ScanRecoveryMaintenanceFailureKind::Unavailable,
        ),
        (
            HistoryErrorKind::OutcomeUnknown,
            ScanRecoveryMaintenanceFailureKind::OutcomeUnknown,
        ),
        (
            HistoryErrorKind::AlreadyExists,
            ScanRecoveryMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::NotFound,
            ScanRecoveryMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::InvalidTransition,
            ScanRecoveryMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::InternalState,
            ScanRecoveryMaintenanceFailureKind::InternalState,
        ),
    ] {
        assert_eq!(
            map_scan_recovery_maintenance_failure(input),
            TaskFailureKind::ScanRecoveryMaintenance(expected)
        );
    }
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
    assert_eq!(
        engine.snapshot_retention_result(id).unwrap_err(),
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
fn candidate_history_status_and_error_mappings_are_exhaustive_and_stable() {
    for (input, expected) in [
        (
            CandidateHistoryStatus::Discovered,
            DurableCandidateStatus::Discovered,
        ),
        (
            CandidateHistoryStatus::Selected,
            DurableCandidateStatus::Selected,
        ),
        (
            CandidateHistoryStatus::Dismissed,
            DurableCandidateStatus::Dismissed,
        ),
        (CandidateHistoryStatus::Stale, DurableCandidateStatus::Stale),
        (
            CandidateHistoryStatus::Planned,
            DurableCandidateStatus::Planned,
        ),
        (
            CandidateHistoryStatus::Completed,
            DurableCandidateStatus::Completed,
        ),
        (
            CandidateHistoryStatus::Failed,
            DurableCandidateStatus::Failed,
        ),
        (
            CandidateHistoryStatus::Unavailable,
            DurableCandidateStatus::Unavailable,
        ),
    ] {
        assert_eq!(public_candidate_status(input), expected);
    }

    for (input, expected) in [
        (
            HistoryErrorKind::IncompatibleSchema,
            CandidateHistoryError::IncompatibleSchema,
        ),
        (
            HistoryErrorKind::QueryLimitExceeded,
            CandidateHistoryError::QueryLimitExceeded,
        ),
        (HistoryErrorKind::Busy, CandidateHistoryError::Busy),
        (
            HistoryErrorKind::UnsafeStorage,
            CandidateHistoryError::UnsafeStorage,
        ),
        (
            HistoryErrorKind::CorruptData,
            CandidateHistoryError::CorruptData,
        ),
        (
            HistoryErrorKind::InternalState,
            CandidateHistoryError::InternalState,
        ),
        (
            HistoryErrorKind::InvalidInput,
            CandidateHistoryError::Unavailable,
        ),
        (
            HistoryErrorKind::AlreadyExists,
            CandidateHistoryError::Unavailable,
        ),
        (
            HistoryErrorKind::NotFound,
            CandidateHistoryError::Unavailable,
        ),
        (
            HistoryErrorKind::InvalidTransition,
            CandidateHistoryError::Unavailable,
        ),
        (
            HistoryErrorKind::DatabaseUnavailable,
            CandidateHistoryError::Unavailable,
        ),
        (
            HistoryErrorKind::OutcomeUnknown,
            CandidateHistoryError::Unavailable,
        ),
    ] {
        assert_eq!(map_candidate_history_error(input), expected);
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
fn snapshot_retention_runs_one_typed_path_free_batch() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(10_000);

    let id = started_snapshot_retention(engine.start_snapshot_retention_at(observed).unwrap());
    let terminal = wait_terminal(&engine, id);
    assert_eq!(terminal.kind, TaskKind::SnapshotRetention);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.result_available);
    let result = engine.snapshot_retention_result(id).unwrap().unwrap();
    assert_eq!(result.observed_at(), observed);
    assert_eq!(result.outcome(), SnapshotRetentionOutcome::UnderCap);
    assert_eq!(result.cap_bytes(), 2 * 1024 * 1024 * 1024);
    assert_eq!(result.charged_bytes_before(), result.charged_bytes_after());
    assert!(!result.has_more());
    assert_eq!(
        engine.history_maintenance_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.scan_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );

    let events = engine.task_events(id, 0, 8).unwrap().events;
    let applying = events
        .iter()
        .position(|event| matches!(event.kind, TaskEventKind::SnapshotRetentionBatchApplying))
        .unwrap();
    let finished = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::SnapshotRetentionBatchFinished {
                    outcome: SnapshotRetentionOutcome::UnderCap,
                    has_more: false,
                    ..
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
fn snapshot_retention_removes_one_final_and_requires_explicit_rescheduling() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 4, 16, 8))
            .unwrap();
    publish_snapshots(&engine, &root, 4);
    assert_eq!(final_snapshot_count(&config), 4);
    engine.set_snapshot_retention_cap(0).unwrap();
    let observed = SystemTime::now() + Duration::from_secs(60);

    let first = started_snapshot_retention(engine.start_snapshot_retention_at(observed).unwrap());
    assert_eq!(wait_terminal(&engine, first).phase, TaskPhase::Succeeded);
    let first_result = engine.snapshot_retention_result(first).unwrap().unwrap();
    assert!(matches!(
        first_result.outcome(),
        SnapshotRetentionOutcome::TombstonedAndRemoved { bytes } if bytes > 0
    ));
    assert!(first_result.charged_bytes_after() < first_result.charged_bytes_before());
    assert!(first_result.has_more());
    assert_eq!(final_snapshot_count(&config), 3);

    let second = started_snapshot_retention(
        engine
            .start_snapshot_retention_at(observed + Duration::from_millis(1))
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, second).phase, TaskPhase::Succeeded);
    let second_result = engine.snapshot_retention_result(second).unwrap().unwrap();
    assert!(matches!(
        second_result.outcome(),
        SnapshotRetentionOutcome::TombstonedAndRemoved { bytes } if bytes > 0
    ));
    assert!(!second_result.has_more());
    assert_eq!(final_snapshot_count(&config), 2);

    let protected = started_snapshot_retention(
        engine
            .start_snapshot_retention_at(observed + Duration::from_millis(2))
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, protected).phase,
        TaskPhase::Succeeded
    );
    assert_eq!(
        engine
            .snapshot_retention_result(protected)
            .unwrap()
            .unwrap()
            .outcome(),
        SnapshotRetentionOutcome::DeferredNoEligibleSnapshot
    );
    assert_eq!(final_snapshot_count(&config), 2);
}

#[test]
fn snapshot_retention_is_idle_deduplicated_and_excludes_history_maintenance() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 8, 8));
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
    let record_count = engine.inner.shared.lock_registry_recover().records.len();
    assert_eq!(
        engine.start_snapshot_retention().unwrap(),
        SnapshotRetentionStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.inner.shared.lock_registry_recover().records.len(),
        record_count
    );
    foreground_release_tx.send(()).unwrap();
    wait_terminal(&engine, foreground);

    let (history_started_tx, history_started_rx) = mpsc::channel();
    let (history_release_tx, history_release_rx) = mpsc::channel();
    let history = started_maintenance(
        engine
            .start_history_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_000),
                move || {
                    history_started_tx.send(()).unwrap();
                    history_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    history_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_retention().unwrap(),
        SnapshotRetentionStartOutcome::DeferredBusy
    );
    history_release_tx.send(()).unwrap();
    wait_terminal(&engine, history);

    let (snapshot_started_tx, snapshot_started_rx) = mpsc::channel();
    let (snapshot_release_tx, snapshot_release_rx) = mpsc::channel();
    let snapshot = started_snapshot_retention(
        engine
            .start_snapshot_retention_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_000),
                move || {
                    snapshot_started_tx.send(()).unwrap();
                    snapshot_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    snapshot_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_retention().unwrap(),
        SnapshotRetentionStartOutcome::AlreadyActive(snapshot)
    );
    assert_eq!(
        engine.start_history_maintenance().unwrap(),
        HistoryMaintenanceStartOutcome::DeferredBusy
    );
    snapshot_release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, snapshot).phase, TaskPhase::Succeeded);
}

#[test]
fn snapshot_retention_cancellation_is_linearized_at_applying() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    publish_snapshots(&engine, &root, 3);
    engine.set_snapshot_retention_cap(0).unwrap();
    let observed = SystemTime::now() + Duration::from_secs(60);

    let (before_tx, before_rx) = mpsc::channel();
    let (before_release_tx, before_release_rx) = mpsc::channel();
    let cancelled = started_snapshot_retention(
        engine
            .start_snapshot_retention_with_test_hooks(
                observed,
                move || {
                    before_tx.send(()).unwrap();
                    before_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    before_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(cancelled).unwrap(),
        CancelOutcome::Requested
    );
    before_release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&engine, cancelled).phase,
        TaskPhase::Cancelled
    );
    assert_eq!(final_snapshot_count(&config), 3);
    assert!(
        engine
            .task_events(cancelled, 0, 8)
            .unwrap()
            .events
            .iter()
            .all(|event| !matches!(event.kind, TaskEventKind::SnapshotRetentionBatchApplying))
    );

    let (applying_tx, applying_rx) = mpsc::channel();
    let (applying_release_tx, applying_release_rx) = mpsc::channel();
    let applied = started_snapshot_retention(
        engine
            .start_snapshot_retention_with_test_hooks(
                observed + Duration::from_millis(1),
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    applying_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(applied).unwrap(),
        CancelOutcome::Requested
    );
    applying_release_tx.send(()).unwrap();
    let terminal = wait_terminal(&engine, applied);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.cancellation_requested);
    assert!(matches!(
        engine
            .snapshot_retention_result(applied)
            .unwrap()
            .unwrap()
            .outcome(),
        SnapshotRetentionOutcome::TombstonedAndRemoved { .. }
    ));
    assert_eq!(final_snapshot_count(&config), 2);
}

#[test]
fn snapshot_retention_serializes_distinct_victims_across_engine_sessions() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let first_engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 16, 8))
            .unwrap();
    publish_snapshots(&first_engine, &root, 4);
    first_engine.set_snapshot_retention_cap(0).unwrap();
    let second_engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 16, 8))
            .unwrap();
    let observed = SystemTime::now() + Duration::from_secs(60);
    let (applying_tx, applying_rx) = mpsc::channel();
    let (first_release_tx, first_release_rx) = mpsc::channel();
    let (second_release_tx, second_release_rx) = mpsc::channel();
    let first_ready = applying_tx.clone();
    let first = started_snapshot_retention(
        first_engine
            .start_snapshot_retention_with_test_hooks(
                observed,
                || {},
                move || {
                    first_ready.send(()).unwrap();
                    first_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    let second = started_snapshot_retention(
        second_engine
            .start_snapshot_retention_with_test_hooks(
                observed,
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    second_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    first_release_tx.send(()).unwrap();
    second_release_tx.send(()).unwrap();

    assert_eq!(
        wait_terminal(&first_engine, first).phase,
        TaskPhase::Succeeded
    );
    assert_eq!(
        wait_terminal(&second_engine, second).phase,
        TaskPhase::Succeeded
    );
    for result in [
        first_engine
            .snapshot_retention_result(first)
            .unwrap()
            .unwrap(),
        second_engine
            .snapshot_retention_result(second)
            .unwrap()
            .unwrap(),
    ] {
        assert!(matches!(
            result.outcome(),
            SnapshotRetentionOutcome::TombstonedAndRemoved { bytes } if bytes > 0
        ));
        assert!(!format!("{result:?}").contains("scan:"));
        assert!(!format!("{result:?}").contains(root.to_string_lossy().as_ref()));
    }
    assert_eq!(final_snapshot_count(&config), 2);
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    let (tombstones, distinct_victims) = connection
        .query_row(
            "SELECT COUNT(*), COUNT(DISTINCT scan_id)
             FROM snapshot_retention_tombstones",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap();
    assert_eq!((tombstones, distinct_victims), (2, 2));
}

#[test]
fn close_before_snapshot_retention_applying_mutates_nothing_and_quiesces() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    publish_snapshots(&engine, &root, 3);
    engine.set_snapshot_retention_cap(0).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let task = started_snapshot_retention(
        engine
            .start_snapshot_retention_with_test_hooks(
                SystemTime::now() + Duration::from_secs(60),
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(engine.close(), CloseOutcome::Initiated);
    release_tx.send(()).unwrap();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));

    let registry = engine.inner.shared.lock_registry_recover();
    let record = registry.records.get(&task).unwrap();
    assert_eq!(record.phase, TaskPhase::Cancelled);
    assert!(record.cancellation_requested);
    assert!(
        record
            .events
            .iter()
            .all(|event| !matches!(event.kind, TaskEventKind::SnapshotRetentionBatchApplying))
    );
    drop(registry);
    assert_eq!(final_snapshot_count(&config), 3);
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM snapshot_retention_tombstones",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn snapshot_retention_failure_mapping_is_exhaustive_and_stable() {
    let assert_mapping = |input, expected| {
        assert_eq!(
            map_snapshot_retention_failure(input),
            TaskFailureKind::SnapshotRetention(expected)
        );
    };
    for input in [
        SnapshotRepositoryErrorKind::ReadOnly,
        SnapshotRepositoryErrorKind::MissingStore,
        SnapshotRepositoryErrorKind::ReviewLeaseExpired,
    ] {
        assert_mapping(input, SnapshotRetentionFailureKind::InternalState);
    }
    for input in [
        SnapshotRepositoryErrorKind::MissingSnapshot,
        SnapshotRepositoryErrorKind::SnapshotUnavailable,
        SnapshotRepositoryErrorKind::ReferenceMismatch,
    ] {
        assert_mapping(input, SnapshotRetentionFailureKind::CorruptData);
    }
    assert_mapping(
        SnapshotRepositoryErrorKind::IncompatibleVersion,
        SnapshotRetentionFailureKind::IncompatibleSnapshot,
    );
    for (input, expected) in [
        (
            SnapshotCodecErrorKind::InvalidInput,
            SnapshotRetentionFailureKind::InternalState,
        ),
        (
            SnapshotCodecErrorKind::Io,
            SnapshotRetentionFailureKind::Unavailable,
        ),
        (
            SnapshotCodecErrorKind::LimitExceeded,
            SnapshotRetentionFailureKind::BudgetExceeded,
        ),
        (
            SnapshotCodecErrorKind::IncompatibleVersion,
            SnapshotRetentionFailureKind::IncompatibleSnapshot,
        ),
        (
            SnapshotCodecErrorKind::InvalidMagic,
            SnapshotRetentionFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::InvalidLength,
            SnapshotRetentionFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::ChecksumMismatch,
            SnapshotRetentionFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::CorruptData,
            SnapshotRetentionFailureKind::CorruptData,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::Codec(input), expected);
    }
    for (input, expected) in [
        (
            SnapshotStorageErrorKind::InvalidConfiguration,
            SnapshotRetentionFailureKind::InternalState,
        ),
        (
            SnapshotStorageErrorKind::UnsafeRoot,
            SnapshotRetentionFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::UnsafeObject,
            SnapshotRetentionFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::UnrecognizedStore,
            SnapshotRetentionFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::Unavailable,
            SnapshotRetentionFailureKind::Unavailable,
        ),
        (
            SnapshotStorageErrorKind::Busy,
            SnapshotRetentionFailureKind::Busy,
        ),
        (
            SnapshotStorageErrorKind::InternalState,
            SnapshotRetentionFailureKind::InternalState,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::Storage(input), expected);
    }
    for (input, expected) in [
        (
            HistoryErrorKind::InvalidInput,
            SnapshotRetentionFailureKind::InvalidClock,
        ),
        (
            HistoryErrorKind::IncompatibleSchema,
            SnapshotRetentionFailureKind::IncompatibleSchema,
        ),
        (
            HistoryErrorKind::QueryLimitExceeded,
            SnapshotRetentionFailureKind::BudgetExceeded,
        ),
        (HistoryErrorKind::Busy, SnapshotRetentionFailureKind::Busy),
        (
            HistoryErrorKind::UnsafeStorage,
            SnapshotRetentionFailureKind::UnsafeStorage,
        ),
        (
            HistoryErrorKind::CorruptData,
            SnapshotRetentionFailureKind::CorruptData,
        ),
        (
            HistoryErrorKind::DatabaseUnavailable,
            SnapshotRetentionFailureKind::Unavailable,
        ),
        (
            HistoryErrorKind::OutcomeUnknown,
            SnapshotRetentionFailureKind::OutcomeUnknown,
        ),
        (
            HistoryErrorKind::AlreadyExists,
            SnapshotRetentionFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::NotFound,
            SnapshotRetentionFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::InvalidTransition,
            SnapshotRetentionFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::InternalState,
            SnapshotRetentionFailureKind::InternalState,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::History(input), expected);
    }
}

#[test]
fn snapshot_retention_outcome_mapping_redacts_repository_identity() {
    let private_id = ScanId::new("scan:must-not-cross-retention-boundary").unwrap();
    for (input, expected) in [
        (
            SnapshotRetentionBatchOutcome::UnderCap,
            SnapshotRetentionOutcome::UnderCap,
        ),
        (
            SnapshotRetentionBatchOutcome::DeferredUnstable,
            SnapshotRetentionOutcome::DeferredUnstable,
        ),
        (
            SnapshotRetentionBatchOutcome::DeferredNoEligibleSnapshot,
            SnapshotRetentionOutcome::DeferredNoEligibleSnapshot,
        ),
        (
            SnapshotRetentionBatchOutcome::RemovedTombstonedResidual {
                scan_id: private_id.clone(),
                bytes: 41,
            },
            SnapshotRetentionOutcome::RemovedTombstonedResidual { bytes: 41 },
        ),
        (
            SnapshotRetentionBatchOutcome::TombstonedAndRemoved {
                scan_id: private_id.clone(),
                bytes: 42,
            },
            SnapshotRetentionOutcome::TombstonedAndRemoved { bytes: 42 },
        ),
    ] {
        let public = public_snapshot_retention_outcome(&input);
        assert_eq!(public, expected);
        assert!(!format!("{public:?}").contains(private_id.as_str()));
    }
}

#[test]
fn schema_upgrade_while_snapshot_retention_waits_fails_without_removal() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    publish_snapshots(&engine, &root, 3);
    engine.set_snapshot_retention_cap(0).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let task = started_snapshot_retention(
        engine
            .start_snapshot_retention_with_test_hooks(
                SystemTime::now() + Duration::from_secs(60),
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
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
                 VALUES (?1, 'future-snapshot-retention-race', zeroblob(32), 2)",
                [i64::from(future)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", future)
            .unwrap();
    });
    release_tx.send(()).unwrap();

    let terminal = wait_terminal(&engine, task);
    assert_eq!(terminal.phase, TaskPhase::Failed);
    assert_eq!(
        terminal.failure,
        Some(TaskFailureKind::SnapshotRetention(
            SnapshotRetentionFailureKind::IncompatibleSchema
        ))
    );
    assert!(!terminal.result_available);
    assert_eq!(final_snapshot_count(&config), 3);
}

#[test]
fn invalid_snapshot_retention_clock_and_panic_release_exclusive_admission() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let invalid = started_snapshot_retention(
        engine
            .start_snapshot_retention_at(SystemTime::UNIX_EPOCH - Duration::from_millis(1))
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, invalid).failure,
        Some(TaskFailureKind::SnapshotRetention(
            SnapshotRetentionFailureKind::InvalidClock
        ))
    );

    let panicking = started_snapshot_retention(
        engine
            .start_snapshot_retention_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_000),
                || panic!("snapshot retention hook panic"),
                || {},
                || {},
            )
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, panicking).phase, TaskPhase::Failed);
    let replacement = started_snapshot_retention(
        engine
            .start_snapshot_retention_at(SystemTime::UNIX_EPOCH + Duration::from_millis(10_000))
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );
}

#[test]
fn snapshot_orphan_maintenance_runs_one_typed_path_free_no_orphan_batch() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(10_000);

    let id = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_at(observed)
            .unwrap(),
    );
    let terminal = wait_terminal(&engine, id);
    assert_eq!(terminal.kind, TaskKind::SnapshotOrphanMaintenance);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.result_available);
    let result = engine
        .snapshot_orphan_maintenance_result(id)
        .unwrap()
        .unwrap();
    assert_eq!(result.observed_at(), observed);
    assert_eq!(result.outcome(), SnapshotOrphanMaintenanceOutcome::NoOrphan);
    assert_eq!(result.orphan_count_before(), 0);
    assert_eq!(result.orphan_count_after(), 0);
    assert_eq!(result.orphan_charged_bytes_before(), 0);
    assert_eq!(result.orphan_charged_bytes_after(), 0);
    assert!(!result.has_more());
    assert_eq!(
        engine.snapshot_retention_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.history_maintenance_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.scan_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );

    let events = engine.task_events(id, 0, 8).unwrap().events;
    let applying = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::SnapshotOrphanMaintenanceBatchApplying
            )
        })
        .unwrap();
    let finished = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::SnapshotOrphanMaintenanceBatchFinished {
                    outcome: SnapshotOrphanMaintenanceOutcome::NoOrphan,
                    orphan_count_before: 0,
                    orphan_count_after: 0,
                    orphan_charged_bytes_before: 0,
                    orphan_charged_bytes_after: 0,
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

    assert_eq!(engine.close(), CloseOutcome::Initiated);
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(
        engine.start_snapshot_orphan_maintenance(),
        Err(StartTaskError::Closed)
    );
}

#[test]
fn snapshot_orphan_maintenance_removes_one_final_and_requires_resubmission() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 4, 16, 8))
            .unwrap();
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(20_000);
    publish_running_orphan(
        &engine,
        &root,
        "scan:engine-orphan-first-private",
        observed - Duration::from_millis(2),
    );
    publish_running_orphan(
        &engine,
        &root,
        "scan:engine-orphan-second-private",
        observed - Duration::from_millis(1),
    );
    assert_eq!(final_snapshot_count(&config), 2);

    let first = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_at(observed)
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, first).phase, TaskPhase::Succeeded);
    let first_result = engine
        .snapshot_orphan_maintenance_result(first)
        .unwrap()
        .unwrap();
    let first_bytes = match first_result.outcome() {
        SnapshotOrphanMaintenanceOutcome::Removed { bytes } => bytes,
        other => panic!("expected removal, got {other:?}"),
    };
    assert!(first_bytes > 0);
    assert_eq!(first_result.orphan_count_before(), 2);
    assert_eq!(first_result.orphan_count_after(), 1);
    assert_eq!(
        first_result.orphan_charged_bytes_before() - first_bytes,
        first_result.orphan_charged_bytes_after()
    );
    assert!(first_result.has_more());
    assert_eq!(final_snapshot_count(&config), 1);
    let debug = format!("{first_result:?}");
    assert!(!debug.contains("engine-orphan"));
    assert!(!debug.contains(root.to_string_lossy().as_ref()));

    let second = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_at(observed + Duration::from_millis(1))
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, second).phase, TaskPhase::Succeeded);
    let second_result = engine
        .snapshot_orphan_maintenance_result(second)
        .unwrap()
        .unwrap();
    assert!(matches!(
        second_result.outcome(),
        SnapshotOrphanMaintenanceOutcome::Removed { bytes } if bytes > 0
    ));
    assert_eq!(second_result.orphan_count_before(), 1);
    assert_eq!(second_result.orphan_count_after(), 0);
    assert!(!second_result.has_more());
    assert_eq!(final_snapshot_count(&config), 0);

    let empty = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_at(observed + Duration::from_millis(2))
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, empty).phase, TaskPhase::Succeeded);
    assert_eq!(
        engine
            .snapshot_orphan_maintenance_result(empty)
            .unwrap()
            .unwrap()
            .outcome(),
        SnapshotOrphanMaintenanceOutcome::NoOrphan
    );
}

#[test]
fn snapshot_orphan_maintenance_is_idle_deduplicated_and_cross_exclusive() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 3, 8, 8));
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
    let record_count = engine.inner.shared.lock_registry_recover().records.len();
    assert_eq!(
        engine.start_snapshot_orphan_maintenance().unwrap(),
        SnapshotOrphanMaintenanceStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.inner.shared.lock_registry_recover().records.len(),
        record_count
    );
    foreground_release_tx.send(()).unwrap();
    wait_terminal(&engine, foreground);

    let (history_started_tx, history_started_rx) = mpsc::channel();
    let (history_release_tx, history_release_rx) = mpsc::channel();
    let history = started_maintenance(
        engine
            .start_history_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(9_999),
                move || {
                    history_started_tx.send(()).unwrap();
                    history_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    history_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_orphan_maintenance().unwrap(),
        SnapshotOrphanMaintenanceStartOutcome::DeferredBusy
    );
    history_release_tx.send(()).unwrap();
    wait_terminal(&engine, history);

    let (orphan_started_tx, orphan_started_rx) = mpsc::channel();
    let (orphan_release_tx, orphan_release_rx) = mpsc::channel();
    let orphan = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_000),
                move || {
                    orphan_started_tx.send(()).unwrap();
                    orphan_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    orphan_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_orphan_maintenance().unwrap(),
        SnapshotOrphanMaintenanceStartOutcome::AlreadyActive(orphan)
    );
    assert_eq!(
        engine.start_history_maintenance().unwrap(),
        HistoryMaintenanceStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.start_snapshot_retention().unwrap(),
        SnapshotRetentionStartOutcome::DeferredBusy
    );
    orphan_release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, orphan).phase, TaskPhase::Succeeded);

    let (retention_started_tx, retention_started_rx) = mpsc::channel();
    let (retention_release_tx, retention_release_rx) = mpsc::channel();
    let retention = started_snapshot_retention(
        engine
            .start_snapshot_retention_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_001),
                move || {
                    retention_started_tx.send(()).unwrap();
                    retention_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    retention_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_orphan_maintenance().unwrap(),
        SnapshotOrphanMaintenanceStartOutcome::DeferredBusy
    );
    retention_release_tx.send(()).unwrap();
    wait_terminal(&engine, retention);
}

#[test]
fn snapshot_orphan_maintenance_cancellation_is_linearized_at_applying() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(20_000);
    publish_running_orphan(
        &engine,
        &root,
        "scan:cancel-orphan-private",
        observed - Duration::from_millis(1),
    );

    let (before_tx, before_rx) = mpsc::channel();
    let (before_release_tx, before_release_rx) = mpsc::channel();
    let cancelled = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_with_test_hooks(
                observed,
                move || {
                    before_tx.send(()).unwrap();
                    before_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    before_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(cancelled).unwrap(),
        CancelOutcome::Requested
    );
    before_release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&engine, cancelled).phase,
        TaskPhase::Cancelled
    );
    assert_eq!(final_snapshot_count(&config), 1);
    assert!(
        engine
            .task_events(cancelled, 0, 8)
            .unwrap()
            .events
            .iter()
            .all(|event| !matches!(
                event.kind,
                TaskEventKind::SnapshotOrphanMaintenanceBatchApplying
            ))
    );

    let (applying_tx, applying_rx) = mpsc::channel();
    let (applying_release_tx, applying_release_rx) = mpsc::channel();
    let applied = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_with_test_hooks(
                observed + Duration::from_millis(1),
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    applying_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(applied).unwrap(),
        CancelOutcome::Requested
    );
    applying_release_tx.send(()).unwrap();
    let terminal = wait_terminal(&engine, applied);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.cancellation_requested);
    assert!(matches!(
        engine
            .snapshot_orphan_maintenance_result(applied)
            .unwrap()
            .unwrap()
            .outcome(),
        SnapshotOrphanMaintenanceOutcome::Removed { .. }
    ));
    assert_eq!(final_snapshot_count(&config), 0);
}

#[test]
fn close_before_snapshot_orphan_applying_mutates_nothing_and_quiesces() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(20_000);
    publish_running_orphan(
        &engine,
        &root,
        "scan:close-orphan-private",
        observed - Duration::from_millis(1),
    );
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let task = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_with_test_hooks(
                observed,
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(engine.close(), CloseOutcome::Initiated);
    release_tx.send(()).unwrap();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    let registry = engine.inner.shared.lock_registry_recover();
    let record = registry.records.get(&task).unwrap();
    assert_eq!(record.phase, TaskPhase::Cancelled);
    assert!(record.cancellation_requested);
    assert!(record.events.iter().all(|event| !matches!(
        event.kind,
        TaskEventKind::SnapshotOrphanMaintenanceBatchApplying
    )));
    drop(registry);
    assert_eq!(final_snapshot_count(&config), 1);
}

#[test]
fn snapshot_orphan_maintenance_serializes_across_engine_sessions() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root");
    std::fs::create_dir(&root).unwrap();
    let first_engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(20_000);
    publish_running_orphan(
        &first_engine,
        &root,
        "scan:cross-session-orphan-private",
        observed - Duration::from_millis(1),
    );
    let second_engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    let (applying_tx, applying_rx) = mpsc::channel();
    let (first_release_tx, first_release_rx) = mpsc::channel();
    let (second_release_tx, second_release_rx) = mpsc::channel();
    let first_ready = applying_tx.clone();
    let first = started_snapshot_orphan_maintenance(
        first_engine
            .start_snapshot_orphan_maintenance_with_test_hooks(
                observed,
                || {},
                move || {
                    first_ready.send(()).unwrap();
                    first_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    let second = started_snapshot_orphan_maintenance(
        second_engine
            .start_snapshot_orphan_maintenance_with_test_hooks(
                observed,
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    second_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    first_release_tx.send(()).unwrap();
    second_release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&first_engine, first).phase,
        TaskPhase::Succeeded
    );
    assert_eq!(
        wait_terminal(&second_engine, second).phase,
        TaskPhase::Succeeded
    );
    let outcomes = [
        first_engine
            .snapshot_orphan_maintenance_result(first)
            .unwrap()
            .unwrap()
            .outcome(),
        second_engine
            .snapshot_orphan_maintenance_result(second)
            .unwrap()
            .unwrap()
            .outcome(),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, SnapshotOrphanMaintenanceOutcome::Removed { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, SnapshotOrphanMaintenanceOutcome::NoOrphan))
            .count(),
        1
    );
    assert_eq!(final_snapshot_count(&config), 0);
}

#[test]
fn invalid_orphan_clock_schema_race_and_panic_release_exclusive_admission() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    let invalid = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_at(SystemTime::UNIX_EPOCH - Duration::from_millis(1))
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, invalid).failure,
        Some(TaskFailureKind::SnapshotOrphanMaintenance(
            SnapshotOrphanMaintenanceFailureKind::InvalidClock
        ))
    );

    let panicking = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_000),
                || panic!("snapshot orphan hook panic"),
                || {},
                || {},
            )
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, panicking).phase, TaskPhase::Failed);
    let replacement = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_at(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_000),
            )
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );

    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let racing = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(20_000),
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
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
                 VALUES (?1, 'future-orphan-maintenance-race', zeroblob(32), 2)",
                [i64::from(future)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", future)
            .unwrap();
    });
    release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&engine, racing).failure,
        Some(TaskFailureKind::SnapshotOrphanMaintenance(
            SnapshotOrphanMaintenanceFailureKind::IncompatibleSchema
        ))
    );
}

#[test]
fn snapshot_orphan_failure_and_outcome_mapping_are_path_free_and_exhaustive() {
    let assert_mapping = |input, expected| {
        assert_eq!(
            map_snapshot_orphan_maintenance_failure(input),
            TaskFailureKind::SnapshotOrphanMaintenance(expected)
        );
    };
    for input in [
        SnapshotRepositoryErrorKind::ReadOnly,
        SnapshotRepositoryErrorKind::MissingStore,
        SnapshotRepositoryErrorKind::ReviewLeaseExpired,
    ] {
        assert_mapping(input, SnapshotOrphanMaintenanceFailureKind::InternalState);
    }
    for input in [
        SnapshotRepositoryErrorKind::MissingSnapshot,
        SnapshotRepositoryErrorKind::SnapshotUnavailable,
        SnapshotRepositoryErrorKind::ReferenceMismatch,
    ] {
        assert_mapping(input, SnapshotOrphanMaintenanceFailureKind::CorruptData);
    }
    assert_mapping(
        SnapshotRepositoryErrorKind::IncompatibleVersion,
        SnapshotOrphanMaintenanceFailureKind::IncompatibleSnapshot,
    );
    for (input, expected) in [
        (
            SnapshotCodecErrorKind::InvalidInput,
            SnapshotOrphanMaintenanceFailureKind::InternalState,
        ),
        (
            SnapshotCodecErrorKind::Io,
            SnapshotOrphanMaintenanceFailureKind::Unavailable,
        ),
        (
            SnapshotCodecErrorKind::LimitExceeded,
            SnapshotOrphanMaintenanceFailureKind::BudgetExceeded,
        ),
        (
            SnapshotCodecErrorKind::IncompatibleVersion,
            SnapshotOrphanMaintenanceFailureKind::IncompatibleSnapshot,
        ),
        (
            SnapshotCodecErrorKind::InvalidMagic,
            SnapshotOrphanMaintenanceFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::InvalidLength,
            SnapshotOrphanMaintenanceFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::ChecksumMismatch,
            SnapshotOrphanMaintenanceFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::CorruptData,
            SnapshotOrphanMaintenanceFailureKind::CorruptData,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::Codec(input), expected);
    }
    for (input, expected) in [
        (
            SnapshotStorageErrorKind::InvalidConfiguration,
            SnapshotOrphanMaintenanceFailureKind::InternalState,
        ),
        (
            SnapshotStorageErrorKind::UnsafeRoot,
            SnapshotOrphanMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::UnsafeObject,
            SnapshotOrphanMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::UnrecognizedStore,
            SnapshotOrphanMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::Unavailable,
            SnapshotOrphanMaintenanceFailureKind::Unavailable,
        ),
        (
            SnapshotStorageErrorKind::Busy,
            SnapshotOrphanMaintenanceFailureKind::Busy,
        ),
        (
            SnapshotStorageErrorKind::InternalState,
            SnapshotOrphanMaintenanceFailureKind::InternalState,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::Storage(input), expected);
    }
    for (input, expected) in [
        (
            HistoryErrorKind::InvalidInput,
            SnapshotOrphanMaintenanceFailureKind::InvalidClock,
        ),
        (
            HistoryErrorKind::IncompatibleSchema,
            SnapshotOrphanMaintenanceFailureKind::IncompatibleSchema,
        ),
        (
            HistoryErrorKind::QueryLimitExceeded,
            SnapshotOrphanMaintenanceFailureKind::BudgetExceeded,
        ),
        (
            HistoryErrorKind::Busy,
            SnapshotOrphanMaintenanceFailureKind::Busy,
        ),
        (
            HistoryErrorKind::UnsafeStorage,
            SnapshotOrphanMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            HistoryErrorKind::CorruptData,
            SnapshotOrphanMaintenanceFailureKind::CorruptData,
        ),
        (
            HistoryErrorKind::DatabaseUnavailable,
            SnapshotOrphanMaintenanceFailureKind::Unavailable,
        ),
        (
            HistoryErrorKind::OutcomeUnknown,
            SnapshotOrphanMaintenanceFailureKind::OutcomeUnknown,
        ),
        (
            HistoryErrorKind::AlreadyExists,
            SnapshotOrphanMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::NotFound,
            SnapshotOrphanMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::InvalidTransition,
            SnapshotOrphanMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::InternalState,
            SnapshotOrphanMaintenanceFailureKind::InternalState,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::History(input), expected);
    }

    let private_id = ScanId::new("scan:must-not-cross-orphan-boundary").unwrap();
    let removed = public_snapshot_orphan_maintenance_outcome(
        &SnapshotOrphanReconciliationBatchOutcome::Removed {
            scan_id: private_id.clone(),
            bytes: 42,
        },
    );
    assert_eq!(
        removed,
        SnapshotOrphanMaintenanceOutcome::Removed { bytes: 42 }
    );
    assert!(!format!("{removed:?}").contains(private_id.as_str()));
    assert_eq!(
        public_snapshot_orphan_maintenance_outcome(
            &SnapshotOrphanReconciliationBatchOutcome::NoOrphan
        ),
        SnapshotOrphanMaintenanceOutcome::NoOrphan
    );
}

#[test]
fn snapshot_terminal_temp_maintenance_runs_one_typed_path_free_noop_batch() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(10_000);
    let id = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_at(observed)
            .unwrap(),
    );
    let terminal = wait_terminal(&engine, id);
    assert_eq!(terminal.kind, TaskKind::SnapshotTerminalTempMaintenance);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.result_available);
    let result = engine
        .snapshot_terminal_temp_maintenance_result(id)
        .unwrap()
        .unwrap();
    assert_eq!(result.observed_at(), observed);
    assert_eq!(
        result.outcome(),
        SnapshotTerminalTempMaintenanceOutcome::NoTerminalResidual
    );
    assert_eq!(result.terminal_lease_count_before(), 0);
    assert_eq!(result.terminal_lease_count_after(), 0);
    assert_eq!(result.active_terminal_lease_count_before(), 0);
    assert_eq!(result.active_terminal_lease_count_after(), 0);
    assert_eq!(result.terminal_charged_bytes_before(), 0);
    assert_eq!(result.terminal_charged_bytes_after(), 0);
    assert!(!result.has_more());
    assert_eq!(
        engine.snapshot_orphan_maintenance_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.snapshot_retention_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.history_maintenance_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.scan_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );

    let events = engine.task_events(id, 0, 8).unwrap().events;
    let applying = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::SnapshotTerminalTempMaintenanceBatchApplying
            )
        })
        .unwrap();
    let finished = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::SnapshotTerminalTempMaintenanceBatchFinished {
                    outcome: SnapshotTerminalTempMaintenanceOutcome::NoTerminalResidual,
                    terminal_lease_count_before: 0,
                    terminal_lease_count_after: 0,
                    active_terminal_lease_count_before: 0,
                    active_terminal_lease_count_after: 0,
                    terminal_charged_bytes_before: 0,
                    terminal_charged_bytes_after: 0,
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

    assert_eq!(engine.close(), CloseOutcome::Initiated);
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(
        engine.start_snapshot_terminal_temp_maintenance(),
        Err(StartTaskError::Closed)
    );
}

#[test]
fn snapshot_terminal_temp_maintenance_reconciles_exactly_one_and_requires_resubmission() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("scan-root-private");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 4, 16, 8))
            .unwrap();
    let base = SystemTime::UNIX_EPOCH + Duration::from_millis(20_000);
    let private_row = "scan:terminal-a-row-private";
    let private_temp = "scan:terminal-b-temp-private";
    leave_terminal_temp_residual(
        &engine,
        &root,
        private_row,
        base,
        TerminalScanStatus::Failed,
        false,
    );
    leave_terminal_temp_residual(
        &engine,
        &root,
        private_temp,
        base + Duration::from_millis(2),
        TerminalScanStatus::Interrupted,
        true,
    );

    let first = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_at(base + Duration::from_millis(10))
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, first).phase, TaskPhase::Succeeded);
    let first_result = engine
        .snapshot_terminal_temp_maintenance_result(first)
        .unwrap()
        .unwrap();
    assert_eq!(
        first_result.outcome(),
        SnapshotTerminalTempMaintenanceOutcome::ReconciledRowOnly
    );
    assert_eq!(first_result.terminal_lease_count_before(), 2);
    assert_eq!(first_result.terminal_lease_count_after(), 1);
    assert_eq!(first_result.active_terminal_lease_count_before(), 0);
    assert_eq!(first_result.active_terminal_lease_count_after(), 0);
    assert_eq!(
        first_result.terminal_charged_bytes_before(),
        first_result.terminal_charged_bytes_after()
    );
    assert!(first_result.has_more());
    {
        let registry = engine.inner.shared.lock_registry_recover();
        assert_eq!(registry.records.len(), 1);
        assert!(registry.queue.is_empty());
        assert!(registry.active_snapshot_terminal_temp_maintenance.is_none());
    }

    let second = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_at(base + Duration::from_millis(11))
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, second).phase, TaskPhase::Succeeded);
    let second_result = engine
        .snapshot_terminal_temp_maintenance_result(second)
        .unwrap()
        .unwrap();
    let bytes = match second_result.outcome() {
        SnapshotTerminalTempMaintenanceOutcome::RemovedTemp { bytes } => bytes,
        other => panic!("expected terminal temp removal, got {other:?}"),
    };
    assert!(bytes > 0);
    assert_eq!(second_result.terminal_lease_count_before(), 1);
    assert_eq!(second_result.terminal_lease_count_after(), 0);
    assert_eq!(second_result.active_terminal_lease_count_before(), 0);
    assert_eq!(second_result.active_terminal_lease_count_after(), 0);
    assert_eq!(second_result.terminal_charged_bytes_before(), bytes);
    assert_eq!(second_result.terminal_charged_bytes_after(), 0);
    assert!(!second_result.has_more());

    for result in [&*first_result, &*second_result] {
        let debug = format!("{result:?}");
        assert!(!debug.contains(private_row));
        assert!(!debug.contains(private_temp));
        assert!(!debug.contains(root.to_string_lossy().as_ref()));
        assert!(!debug.contains(".snapshot-"));
    }
    let connection = rusqlite::Connection::open(config.database_path()).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM snapshot_temp_leases", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );

    let empty = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_at(base + Duration::from_millis(12))
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, empty).phase, TaskPhase::Succeeded);
    assert_eq!(
        engine
            .snapshot_terminal_temp_maintenance_result(empty)
            .unwrap()
            .unwrap()
            .outcome(),
        SnapshotTerminalTempMaintenanceOutcome::NoTerminalResidual
    );
}

#[test]
fn snapshot_terminal_temp_maintenance_is_idle_deduplicated_and_cross_exclusive() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 3, 16, 8));

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
    let record_count = engine.inner.shared.lock_registry_recover().records.len();
    assert_eq!(
        engine.start_snapshot_terminal_temp_maintenance().unwrap(),
        SnapshotTerminalTempMaintenanceStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.inner.shared.lock_registry_recover().records.len(),
        record_count
    );
    foreground_release_tx.send(()).unwrap();
    wait_terminal(&engine, foreground);

    let (history_started_tx, history_started_rx) = mpsc::channel();
    let (history_release_tx, history_release_rx) = mpsc::channel();
    let history = started_maintenance(
        engine
            .start_history_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(9_997),
                move || {
                    history_started_tx.send(()).unwrap();
                    history_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    history_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_terminal_temp_maintenance().unwrap(),
        SnapshotTerminalTempMaintenanceStartOutcome::DeferredBusy
    );
    history_release_tx.send(()).unwrap();
    wait_terminal(&engine, history);

    let (retention_started_tx, retention_started_rx) = mpsc::channel();
    let (retention_release_tx, retention_release_rx) = mpsc::channel();
    let retention = started_snapshot_retention(
        engine
            .start_snapshot_retention_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(9_998),
                move || {
                    retention_started_tx.send(()).unwrap();
                    retention_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    retention_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_terminal_temp_maintenance().unwrap(),
        SnapshotTerminalTempMaintenanceStartOutcome::DeferredBusy
    );
    retention_release_tx.send(()).unwrap();
    wait_terminal(&engine, retention);

    let (orphan_started_tx, orphan_started_rx) = mpsc::channel();
    let (orphan_release_tx, orphan_release_rx) = mpsc::channel();
    let orphan = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(9_999),
                move || {
                    orphan_started_tx.send(()).unwrap();
                    orphan_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    orphan_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_terminal_temp_maintenance().unwrap(),
        SnapshotTerminalTempMaintenanceStartOutcome::DeferredBusy
    );
    orphan_release_tx.send(()).unwrap();
    wait_terminal(&engine, orphan);

    let (temp_started_tx, temp_started_rx) = mpsc::channel();
    let (temp_release_tx, temp_release_rx) = mpsc::channel();
    let temp = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_000),
                move || {
                    temp_started_tx.send(()).unwrap();
                    temp_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    temp_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_terminal_temp_maintenance().unwrap(),
        SnapshotTerminalTempMaintenanceStartOutcome::AlreadyActive(temp)
    );
    assert_eq!(
        engine.start_history_maintenance().unwrap(),
        HistoryMaintenanceStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.start_snapshot_retention().unwrap(),
        SnapshotRetentionStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.start_snapshot_orphan_maintenance().unwrap(),
        SnapshotOrphanMaintenanceStartOutcome::DeferredBusy
    );
    temp_release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, temp).phase, TaskPhase::Succeeded);
}

#[test]
fn snapshot_terminal_temp_cancellation_is_linearized_at_applying() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("cancel-terminal-temp-private");
    std::fs::create_dir(&root).unwrap();
    let engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(30_000);
    leave_terminal_temp_residual(
        &engine,
        &root,
        "scan:cancel-terminal-temp-private",
        observed - Duration::from_millis(2),
        TerminalScanStatus::Cancelled,
        false,
    );

    let (before_tx, before_rx) = mpsc::channel();
    let (before_release_tx, before_release_rx) = mpsc::channel();
    let cancelled = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_with_test_hooks(
                observed,
                move || {
                    before_tx.send(()).unwrap();
                    before_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    before_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(cancelled).unwrap(),
        CancelOutcome::Requested
    );
    before_release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&engine, cancelled).phase,
        TaskPhase::Cancelled
    );
    assert!(
        engine
            .task_events(cancelled, 0, 8)
            .unwrap()
            .events
            .iter()
            .all(|event| !matches!(
                event.kind,
                TaskEventKind::SnapshotTerminalTempMaintenanceBatchApplying
            ))
    );
    let row_count = || {
        rusqlite::Connection::open(config.database_path())
            .unwrap()
            .query_row("SELECT COUNT(*) FROM snapshot_temp_leases", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    };
    assert_eq!(row_count(), 1);

    let (applying_tx, applying_rx) = mpsc::channel();
    let (applying_release_tx, applying_release_rx) = mpsc::channel();
    let applied = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_with_test_hooks(
                observed + Duration::from_millis(1),
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    applying_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(applied).unwrap(),
        CancelOutcome::Requested
    );
    applying_release_tx.send(()).unwrap();
    let terminal = wait_terminal(&engine, applied);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.cancellation_requested);
    assert_eq!(
        engine
            .snapshot_terminal_temp_maintenance_result(applied)
            .unwrap()
            .unwrap()
            .outcome(),
        SnapshotTerminalTempMaintenanceOutcome::ReconciledRowOnly
    );
    assert_eq!(row_count(), 0);
}

#[test]
fn snapshot_terminal_temp_close_is_linearized_at_applying() {
    let run = |close_after_applying: bool| {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let root = temp.path().join(if close_after_applying {
            "close-after-terminal-temp-private"
        } else {
            "close-before-terminal-temp-private"
        });
        std::fs::create_dir(&root).unwrap();
        let engine =
            EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
                .unwrap();
        let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(40_000);
        leave_terminal_temp_residual(
            &engine,
            &root,
            if close_after_applying {
                "scan:close-after-terminal-temp-private"
            } else {
                "scan:close-before-terminal-temp-private"
            },
            observed - Duration::from_millis(2),
            TerminalScanStatus::Failed,
            false,
        );
        let (blocked_tx, blocked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let task = if close_after_applying {
            started_snapshot_terminal_temp_maintenance(
                engine
                    .start_snapshot_terminal_temp_maintenance_with_test_hooks(
                        observed,
                        || {},
                        move || {
                            blocked_tx.send(()).unwrap();
                            release_rx.recv().unwrap();
                        },
                        || {},
                    )
                    .unwrap(),
            )
        } else {
            started_snapshot_terminal_temp_maintenance(
                engine
                    .start_snapshot_terminal_temp_maintenance_with_test_hooks(
                        observed,
                        move || {
                            blocked_tx.send(()).unwrap();
                            release_rx.recv().unwrap();
                        },
                        || {},
                        || {},
                    )
                    .unwrap(),
            )
        };
        blocked_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        assert_eq!(engine.close(), CloseOutcome::Initiated);
        release_tx.send(()).unwrap();
        assert!(engine.wait_until_closed(TEST_TIMEOUT));

        let registry = engine.inner.shared.lock_registry_recover();
        let record = registry.records.get(&task).unwrap();
        assert!(record.cancellation_requested);
        let applying_seen = record.events.iter().any(|event| {
            matches!(
                event.kind,
                TaskEventKind::SnapshotTerminalTempMaintenanceBatchApplying
            )
        });
        if close_after_applying {
            assert_eq!(record.phase, TaskPhase::Succeeded);
            assert!(applying_seen);
            assert!(matches!(
                record.result,
                Some(TaskResult::SnapshotTerminalTempMaintenance(_))
            ));
        } else {
            assert_eq!(record.phase, TaskPhase::Cancelled);
            assert!(!applying_seen);
            assert!(record.result.is_none());
        }
        drop(registry);
        let rows = rusqlite::Connection::open(config.database_path())
            .unwrap()
            .query_row("SELECT COUNT(*) FROM snapshot_temp_leases", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(rows, if close_after_applying { 0 } else { 1 });
    };

    run(false);
    run(true);
}

#[test]
fn snapshot_terminal_temp_failure_and_outcome_mapping_are_path_free_and_exhaustive() {
    let assert_mapping = |input, expected| {
        assert_eq!(
            map_snapshot_terminal_temp_maintenance_failure(input),
            TaskFailureKind::SnapshotTerminalTempMaintenance(expected)
        );
    };
    for input in [
        SnapshotRepositoryErrorKind::ReadOnly,
        SnapshotRepositoryErrorKind::MissingStore,
        SnapshotRepositoryErrorKind::ReviewLeaseExpired,
        SnapshotRepositoryErrorKind::IncompatibleVersion,
    ] {
        assert_mapping(
            input,
            SnapshotTerminalTempMaintenanceFailureKind::InternalState,
        );
    }
    for input in [
        SnapshotRepositoryErrorKind::MissingSnapshot,
        SnapshotRepositoryErrorKind::SnapshotUnavailable,
        SnapshotRepositoryErrorKind::ReferenceMismatch,
    ] {
        assert_mapping(
            input,
            SnapshotTerminalTempMaintenanceFailureKind::CorruptData,
        );
    }
    for (input, expected) in [
        (
            SnapshotCodecErrorKind::InvalidInput,
            SnapshotTerminalTempMaintenanceFailureKind::InternalState,
        ),
        (
            SnapshotCodecErrorKind::Io,
            SnapshotTerminalTempMaintenanceFailureKind::Unavailable,
        ),
        (
            SnapshotCodecErrorKind::LimitExceeded,
            SnapshotTerminalTempMaintenanceFailureKind::BudgetExceeded,
        ),
        (
            SnapshotCodecErrorKind::IncompatibleVersion,
            SnapshotTerminalTempMaintenanceFailureKind::InternalState,
        ),
        (
            SnapshotCodecErrorKind::InvalidMagic,
            SnapshotTerminalTempMaintenanceFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::InvalidLength,
            SnapshotTerminalTempMaintenanceFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::ChecksumMismatch,
            SnapshotTerminalTempMaintenanceFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::CorruptData,
            SnapshotTerminalTempMaintenanceFailureKind::CorruptData,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::Codec(input), expected);
    }
    for (input, expected) in [
        (
            SnapshotStorageErrorKind::InvalidConfiguration,
            SnapshotTerminalTempMaintenanceFailureKind::InternalState,
        ),
        (
            SnapshotStorageErrorKind::UnsafeRoot,
            SnapshotTerminalTempMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::UnsafeObject,
            SnapshotTerminalTempMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::UnrecognizedStore,
            SnapshotTerminalTempMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::Unavailable,
            SnapshotTerminalTempMaintenanceFailureKind::Unavailable,
        ),
        (
            SnapshotStorageErrorKind::Busy,
            SnapshotTerminalTempMaintenanceFailureKind::Busy,
        ),
        (
            SnapshotStorageErrorKind::InternalState,
            SnapshotTerminalTempMaintenanceFailureKind::InternalState,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::Storage(input), expected);
    }
    for (input, expected) in [
        (
            HistoryErrorKind::InvalidInput,
            SnapshotTerminalTempMaintenanceFailureKind::InvalidClock,
        ),
        (
            HistoryErrorKind::IncompatibleSchema,
            SnapshotTerminalTempMaintenanceFailureKind::IncompatibleSchema,
        ),
        (
            HistoryErrorKind::QueryLimitExceeded,
            SnapshotTerminalTempMaintenanceFailureKind::BudgetExceeded,
        ),
        (
            HistoryErrorKind::Busy,
            SnapshotTerminalTempMaintenanceFailureKind::Busy,
        ),
        (
            HistoryErrorKind::UnsafeStorage,
            SnapshotTerminalTempMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            HistoryErrorKind::CorruptData,
            SnapshotTerminalTempMaintenanceFailureKind::CorruptData,
        ),
        (
            HistoryErrorKind::DatabaseUnavailable,
            SnapshotTerminalTempMaintenanceFailureKind::Unavailable,
        ),
        (
            HistoryErrorKind::OutcomeUnknown,
            SnapshotTerminalTempMaintenanceFailureKind::OutcomeUnknown,
        ),
        (
            HistoryErrorKind::AlreadyExists,
            SnapshotTerminalTempMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::NotFound,
            SnapshotTerminalTempMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::InvalidTransition,
            SnapshotTerminalTempMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::InternalState,
            SnapshotTerminalTempMaintenanceFailureKind::InternalState,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::History(input), expected);
    }

    let private_id = ScanId::new("scan:must-not-cross-terminal-temp-boundary").unwrap();
    for (input, expected) in [
        (
            SnapshotTerminalTempReconciliationBatchOutcome::NoTerminalResidual,
            SnapshotTerminalTempMaintenanceOutcome::NoTerminalResidual,
        ),
        (
            SnapshotTerminalTempReconciliationBatchOutcome::DeferredActive,
            SnapshotTerminalTempMaintenanceOutcome::DeferredActive,
        ),
        (
            SnapshotTerminalTempReconciliationBatchOutcome::ReconciledRowOnly {
                scan_id: private_id.clone(),
            },
            SnapshotTerminalTempMaintenanceOutcome::ReconciledRowOnly,
        ),
        (
            SnapshotTerminalTempReconciliationBatchOutcome::RemovedTempAndLease {
                scan_id: private_id.clone(),
                bytes: 42,
            },
            SnapshotTerminalTempMaintenanceOutcome::RemovedTemp { bytes: 42 },
        ),
    ] {
        let public = public_snapshot_terminal_temp_maintenance_outcome(&input);
        assert_eq!(public, expected);
        assert!(!format!("{public:?}").contains(private_id.as_str()));
    }
}

#[test]
fn invalid_terminal_temp_clock_schema_race_and_panic_release_exclusive_admission() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let engine =
        EngineHandle::open_with_limits(config, RegistryLimits::testing(1, 2, 8, 8)).unwrap();
    let invalid = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_at(
                SystemTime::UNIX_EPOCH - Duration::from_millis(1),
            )
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, invalid).failure,
        Some(TaskFailureKind::SnapshotTerminalTempMaintenance(
            SnapshotTerminalTempMaintenanceFailureKind::InvalidClock
        ))
    );

    let panicking = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_000),
                || panic!("snapshot terminal-temp hook panic"),
                || {},
                || {},
            )
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, panicking).phase, TaskPhase::Failed);
    let replacement = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_at(
                SystemTime::UNIX_EPOCH + Duration::from_millis(10_000),
            )
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );

    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let racing = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(20_000),
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
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
                 VALUES (?1, 'future-terminal-temp-maintenance-race', zeroblob(32), 2)",
                [i64::from(future)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", future)
            .unwrap();
    });
    release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&engine, racing).failure,
        Some(TaskFailureKind::SnapshotTerminalTempMaintenance(
            SnapshotTerminalTempMaintenanceFailureKind::IncompatibleSchema
        ))
    );
}

#[test]
fn snapshot_terminal_temp_maintenance_serializes_across_engine_sessions() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let root = temp.path().join("cross-session-terminal-temp-private");
    std::fs::create_dir(&root).unwrap();
    let first_engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(50_000);
    leave_terminal_temp_residual(
        &first_engine,
        &root,
        "scan:cross-session-terminal-temp-private",
        observed - Duration::from_millis(2),
        TerminalScanStatus::Interrupted,
        false,
    );
    let second_engine =
        EngineHandle::open_with_limits(config, RegistryLimits::testing(1, 2, 8, 8)).unwrap();
    let (applying_tx, applying_rx) = mpsc::channel();
    let (first_release_tx, first_release_rx) = mpsc::channel();
    let (second_release_tx, second_release_rx) = mpsc::channel();
    let first_ready = applying_tx.clone();
    let first = started_snapshot_terminal_temp_maintenance(
        first_engine
            .start_snapshot_terminal_temp_maintenance_with_test_hooks(
                observed,
                || {},
                move || {
                    first_ready.send(()).unwrap();
                    first_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    let second = started_snapshot_terminal_temp_maintenance(
        second_engine
            .start_snapshot_terminal_temp_maintenance_with_test_hooks(
                observed,
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    second_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    first_release_tx.send(()).unwrap();
    second_release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&first_engine, first).phase,
        TaskPhase::Succeeded
    );
    assert_eq!(
        wait_terminal(&second_engine, second).phase,
        TaskPhase::Succeeded
    );
    let outcomes = [
        first_engine
            .snapshot_terminal_temp_maintenance_result(first)
            .unwrap()
            .unwrap()
            .outcome(),
        second_engine
            .snapshot_terminal_temp_maintenance_result(second)
            .unwrap()
            .unwrap()
            .outcome(),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    SnapshotTerminalTempMaintenanceOutcome::ReconciledRowOnly
                )
            })
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    SnapshotTerminalTempMaintenanceOutcome::NoTerminalResidual
                )
            })
            .count(),
        1
    );
    for (engine, id) in [(&first_engine, first), (&second_engine, second)] {
        let result = engine
            .snapshot_terminal_temp_maintenance_result(id)
            .unwrap()
            .unwrap();
        let debug = format!("{result:?}");
        assert!(!debug.contains("cross-session-terminal-temp-private"));
        assert!(!debug.contains(root.to_string_lossy().as_ref()));
    }
}

#[test]
fn snapshot_unleased_temp_maintenance_runs_one_typed_path_free_noop_batch() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(60_000);
    let id = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_at(observed)
            .unwrap(),
    );
    let terminal = wait_terminal(&engine, id);
    assert_eq!(terminal.kind, TaskKind::SnapshotUnleasedTempMaintenance);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.result_available);
    let result = engine
        .snapshot_unleased_temp_maintenance_result(id)
        .unwrap()
        .unwrap();
    assert_eq!(result.observed_at(), observed);
    assert_eq!(
        result.outcome(),
        SnapshotUnleasedTempMaintenanceOutcome::NoUnleasedTemp
    );
    assert_eq!(result.unleased_temp_count_before(), 0);
    assert_eq!(result.unleased_temp_count_after(), 0);
    assert_eq!(result.active_unleased_temp_count_before(), 0);
    assert_eq!(result.active_unleased_temp_count_after(), 0);
    assert_eq!(result.unleased_charged_bytes_before(), 0);
    assert_eq!(result.unleased_charged_bytes_after(), 0);
    assert!(!result.has_more());
    assert_eq!(
        engine
            .snapshot_terminal_temp_maintenance_result(id)
            .unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.snapshot_orphan_maintenance_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.snapshot_retention_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.history_maintenance_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    assert_eq!(
        engine.scan_result(id).unwrap_err(),
        TaskAccessError::WrongTaskKind
    );

    let events = engine.task_events(id, 0, 8).unwrap().events;
    let applying = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::SnapshotUnleasedTempMaintenanceBatchApplying
            )
        })
        .unwrap();
    let finished = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::SnapshotUnleasedTempMaintenanceBatchFinished {
                    outcome: SnapshotUnleasedTempMaintenanceOutcome::NoUnleasedTemp,
                    unleased_temp_count_before: 0,
                    unleased_temp_count_after: 0,
                    active_unleased_temp_count_before: 0,
                    active_unleased_temp_count_after: 0,
                    unleased_charged_bytes_before: 0,
                    unleased_charged_bytes_after: 0,
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
fn snapshot_unleased_temp_maintenance_is_one_at_a_time_accounted_and_defers_active() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let engine =
        EngineHandle::open_with_limits(config, RegistryLimits::testing(1, 4, 16, 8)).unwrap();
    let active_seed: &[u8] = b"active-unleased-private-seed";
    let first_seed: &[u8] = b"first-unleased-private-seed";
    let second_seed: &[u8] = b"second-unleased-private-seed";
    let active = engine
        .inner
        .snapshots
        .leave_unleased_snapshot_temp_for_test(active_seed, true)
        .unwrap()
        .unwrap();
    engine
        .inner
        .snapshots
        .leave_unleased_snapshot_temp_for_test(first_seed, false)
        .unwrap();
    engine
        .inner
        .snapshots
        .leave_unleased_snapshot_temp_for_test(second_seed, false)
        .unwrap();
    let base = SystemTime::UNIX_EPOCH + Duration::from_millis(61_000);

    let first = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_at(base)
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, first).phase, TaskPhase::Succeeded);
    let first_result = engine
        .snapshot_unleased_temp_maintenance_result(first)
        .unwrap()
        .unwrap();
    let first_bytes = match first_result.outcome() {
        SnapshotUnleasedTempMaintenanceOutcome::Removed { bytes } => bytes,
        other => panic!("expected unleased temp removal, got {other:?}"),
    };
    assert!(first_bytes > 0);
    assert_eq!(first_result.unleased_temp_count_before(), 3);
    assert_eq!(first_result.unleased_temp_count_after(), 2);
    assert_eq!(first_result.active_unleased_temp_count_before(), 1);
    assert_eq!(first_result.active_unleased_temp_count_after(), 1);
    assert_eq!(
        first_result.unleased_charged_bytes_before() - first_bytes,
        first_result.unleased_charged_bytes_after()
    );
    assert!(first_result.has_more());
    assert!(
        engine
            .inner
            .shared
            .lock_registry_recover()
            .active_snapshot_unleased_temp_maintenance
            .is_none()
    );

    let second = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_at(base + Duration::from_millis(1))
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, second).phase, TaskPhase::Succeeded);
    let second_result = engine
        .snapshot_unleased_temp_maintenance_result(second)
        .unwrap()
        .unwrap();
    let second_bytes = match second_result.outcome() {
        SnapshotUnleasedTempMaintenanceOutcome::Removed { bytes } => bytes,
        other => panic!("expected unleased temp removal, got {other:?}"),
    };
    assert_eq!(second_result.unleased_temp_count_before(), 2);
    assert_eq!(second_result.unleased_temp_count_after(), 1);
    assert_eq!(second_result.active_unleased_temp_count_before(), 1);
    assert_eq!(second_result.active_unleased_temp_count_after(), 1);
    assert_eq!(
        second_result.unleased_charged_bytes_before() - second_bytes,
        second_result.unleased_charged_bytes_after()
    );
    assert!(second_result.has_more());

    let deferred = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_at(base + Duration::from_millis(2))
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, deferred).phase, TaskPhase::Succeeded);
    let deferred_result = engine
        .snapshot_unleased_temp_maintenance_result(deferred)
        .unwrap()
        .unwrap();
    assert_eq!(
        deferred_result.outcome(),
        SnapshotUnleasedTempMaintenanceOutcome::DeferredActive
    );
    assert_eq!(deferred_result.unleased_temp_count_before(), 1);
    assert_eq!(deferred_result.unleased_temp_count_after(), 1);
    assert_eq!(deferred_result.active_unleased_temp_count_before(), 1);
    assert_eq!(deferred_result.active_unleased_temp_count_after(), 1);
    assert_eq!(
        deferred_result.unleased_charged_bytes_before(),
        deferred_result.unleased_charged_bytes_after()
    );
    assert!(deferred_result.has_more());

    active.abandon();
    let final_task = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_at(base + Duration::from_millis(3))
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, final_task).phase,
        TaskPhase::Succeeded
    );
    let final_result = engine
        .snapshot_unleased_temp_maintenance_result(final_task)
        .unwrap()
        .unwrap();
    assert!(matches!(
        final_result.outcome(),
        SnapshotUnleasedTempMaintenanceOutcome::Removed { bytes } if bytes > 0
    ));
    assert_eq!(final_result.unleased_temp_count_after(), 0);
    assert_eq!(final_result.unleased_charged_bytes_after(), 0);
    assert!(!final_result.has_more());

    for result in [
        &*first_result,
        &*second_result,
        &*deferred_result,
        &*final_result,
    ] {
        let debug = format!("{result:?}");
        for seed in [active_seed, first_seed, second_seed] {
            assert!(!debug.contains(std::str::from_utf8(seed).unwrap()));
        }
        assert!(!debug.contains(".snapshot-"));
    }
}

#[test]
fn snapshot_unleased_temp_maintenance_is_idle_deduplicated_and_cross_exclusive() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 3, 16, 8));

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
    assert_eq!(
        engine.start_snapshot_unleased_temp_maintenance().unwrap(),
        SnapshotUnleasedTempMaintenanceStartOutcome::DeferredBusy
    );
    foreground_release_tx.send(()).unwrap();
    wait_terminal(&engine, foreground);

    let (history_started_tx, history_started_rx) = mpsc::channel();
    let (history_release_tx, history_release_rx) = mpsc::channel();
    let history = started_maintenance(
        engine
            .start_history_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(62_000),
                move || {
                    history_started_tx.send(()).unwrap();
                    history_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    history_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_unleased_temp_maintenance().unwrap(),
        SnapshotUnleasedTempMaintenanceStartOutcome::DeferredBusy
    );
    history_release_tx.send(()).unwrap();
    wait_terminal(&engine, history);

    let (retention_started_tx, retention_started_rx) = mpsc::channel();
    let (retention_release_tx, retention_release_rx) = mpsc::channel();
    let retention = started_snapshot_retention(
        engine
            .start_snapshot_retention_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(62_001),
                move || {
                    retention_started_tx.send(()).unwrap();
                    retention_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    retention_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_unleased_temp_maintenance().unwrap(),
        SnapshotUnleasedTempMaintenanceStartOutcome::DeferredBusy
    );
    retention_release_tx.send(()).unwrap();
    wait_terminal(&engine, retention);

    let (orphan_started_tx, orphan_started_rx) = mpsc::channel();
    let (orphan_release_tx, orphan_release_rx) = mpsc::channel();
    let orphan = started_snapshot_orphan_maintenance(
        engine
            .start_snapshot_orphan_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(62_002),
                move || {
                    orphan_started_tx.send(()).unwrap();
                    orphan_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    orphan_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_unleased_temp_maintenance().unwrap(),
        SnapshotUnleasedTempMaintenanceStartOutcome::DeferredBusy
    );
    orphan_release_tx.send(()).unwrap();
    wait_terminal(&engine, orphan);

    let (terminal_started_tx, terminal_started_rx) = mpsc::channel();
    let (terminal_release_tx, terminal_release_rx) = mpsc::channel();
    let terminal = started_snapshot_terminal_temp_maintenance(
        engine
            .start_snapshot_terminal_temp_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(62_003),
                move || {
                    terminal_started_tx.send(()).unwrap();
                    terminal_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    terminal_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_unleased_temp_maintenance().unwrap(),
        SnapshotUnleasedTempMaintenanceStartOutcome::DeferredBusy
    );
    terminal_release_tx.send(()).unwrap();
    wait_terminal(&engine, terminal);

    let (unleased_started_tx, unleased_started_rx) = mpsc::channel();
    let (unleased_release_tx, unleased_release_rx) = mpsc::channel();
    let unleased = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(62_004),
                move || {
                    unleased_started_tx.send(()).unwrap();
                    unleased_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    unleased_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.start_snapshot_unleased_temp_maintenance().unwrap(),
        SnapshotUnleasedTempMaintenanceStartOutcome::AlreadyActive(unleased)
    );
    assert_eq!(
        engine.start_history_maintenance().unwrap(),
        HistoryMaintenanceStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.start_snapshot_retention().unwrap(),
        SnapshotRetentionStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.start_snapshot_orphan_maintenance().unwrap(),
        SnapshotOrphanMaintenanceStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.start_snapshot_terminal_temp_maintenance().unwrap(),
        SnapshotTerminalTempMaintenanceStartOutcome::DeferredBusy
    );
    unleased_release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, unleased).phase, TaskPhase::Succeeded);
}

#[test]
fn snapshot_unleased_temp_cancellation_is_linearized_at_applying() {
    let temp = TempDir::new().unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 2, 8, 8)).unwrap();
    let private_seed = b"cancel-unleased-temp-private";
    engine
        .inner
        .snapshots
        .leave_unleased_snapshot_temp_for_test(private_seed, false)
        .unwrap();
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(63_000);

    let (before_tx, before_rx) = mpsc::channel();
    let (before_release_tx, before_release_rx) = mpsc::channel();
    let cancelled = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_with_test_hooks(
                observed,
                move || {
                    before_tx.send(()).unwrap();
                    before_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    before_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(cancelled).unwrap(),
        CancelOutcome::Requested
    );
    before_release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&engine, cancelled).phase,
        TaskPhase::Cancelled
    );
    assert!(
        engine
            .task_events(cancelled, 0, 8)
            .unwrap()
            .events
            .iter()
            .all(|event| !matches!(
                event.kind,
                TaskEventKind::SnapshotUnleasedTempMaintenanceBatchApplying
            ))
    );

    let (applying_tx, applying_rx) = mpsc::channel();
    let (applying_release_tx, applying_release_rx) = mpsc::channel();
    let applied = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_with_test_hooks(
                observed + Duration::from_millis(1),
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    applying_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(applied).unwrap(),
        CancelOutcome::Requested
    );
    applying_release_tx.send(()).unwrap();
    let terminal = wait_terminal(&engine, applied);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.cancellation_requested);
    let result = engine
        .snapshot_unleased_temp_maintenance_result(applied)
        .unwrap()
        .unwrap();
    assert!(matches!(
        result.outcome(),
        SnapshotUnleasedTempMaintenanceOutcome::Removed { bytes } if bytes > 0
    ));
    assert!(!format!("{result:?}").contains(std::str::from_utf8(private_seed).unwrap()));
}

#[test]
fn snapshot_unleased_temp_close_is_linearized_at_applying() {
    let run = |close_after_applying: bool| {
        let temp = TempDir::new().unwrap();
        let engine =
            EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 2, 8, 8))
                .unwrap();
        engine
            .inner
            .snapshots
            .leave_unleased_snapshot_temp_for_test(
                if close_after_applying {
                    b"close-after-unleased-private"
                } else {
                    b"close-before-unleased-private"
                },
                false,
            )
            .unwrap();
        let (blocked_tx, blocked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(64_000);
        let task = if close_after_applying {
            started_snapshot_unleased_temp_maintenance(
                engine
                    .start_snapshot_unleased_temp_maintenance_with_test_hooks(
                        observed,
                        || {},
                        move || {
                            blocked_tx.send(()).unwrap();
                            release_rx.recv().unwrap();
                        },
                        || {},
                    )
                    .unwrap(),
            )
        } else {
            started_snapshot_unleased_temp_maintenance(
                engine
                    .start_snapshot_unleased_temp_maintenance_with_test_hooks(
                        observed,
                        move || {
                            blocked_tx.send(()).unwrap();
                            release_rx.recv().unwrap();
                        },
                        || {},
                        || {},
                    )
                    .unwrap(),
            )
        };
        blocked_rx.recv_timeout(TEST_TIMEOUT).unwrap();
        assert_eq!(engine.close(), CloseOutcome::Initiated);
        release_tx.send(()).unwrap();
        assert!(engine.wait_until_closed(TEST_TIMEOUT));

        let registry = engine.inner.shared.lock_registry_recover();
        let record = registry.records.get(&task).unwrap();
        assert!(record.cancellation_requested);
        let applying_seen = record.events.iter().any(|event| {
            matches!(
                event.kind,
                TaskEventKind::SnapshotUnleasedTempMaintenanceBatchApplying
            )
        });
        if close_after_applying {
            assert_eq!(record.phase, TaskPhase::Succeeded);
            assert!(applying_seen);
            assert!(matches!(
                record.result,
                Some(TaskResult::SnapshotUnleasedTempMaintenance(_))
            ));
        } else {
            assert_eq!(record.phase, TaskPhase::Cancelled);
            assert!(!applying_seen);
            assert!(record.result.is_none());
        }
    };

    run(false);
    run(true);
}

#[test]
fn snapshot_unleased_temp_failure_and_outcome_mapping_is_exhaustive() {
    let assert_mapping = |input, expected| {
        assert_eq!(
            map_snapshot_unleased_temp_maintenance_failure(input),
            TaskFailureKind::SnapshotUnleasedTempMaintenance(expected)
        );
    };
    for input in [
        SnapshotRepositoryErrorKind::ReadOnly,
        SnapshotRepositoryErrorKind::MissingStore,
        SnapshotRepositoryErrorKind::ReviewLeaseExpired,
        SnapshotRepositoryErrorKind::IncompatibleVersion,
    ] {
        assert_mapping(
            input,
            SnapshotUnleasedTempMaintenanceFailureKind::InternalState,
        );
    }
    for input in [
        SnapshotRepositoryErrorKind::MissingSnapshot,
        SnapshotRepositoryErrorKind::SnapshotUnavailable,
        SnapshotRepositoryErrorKind::ReferenceMismatch,
    ] {
        assert_mapping(
            input,
            SnapshotUnleasedTempMaintenanceFailureKind::CorruptData,
        );
    }
    for (input, expected) in [
        (
            SnapshotCodecErrorKind::InvalidInput,
            SnapshotUnleasedTempMaintenanceFailureKind::InternalState,
        ),
        (
            SnapshotCodecErrorKind::Io,
            SnapshotUnleasedTempMaintenanceFailureKind::Unavailable,
        ),
        (
            SnapshotCodecErrorKind::LimitExceeded,
            SnapshotUnleasedTempMaintenanceFailureKind::BudgetExceeded,
        ),
        (
            SnapshotCodecErrorKind::IncompatibleVersion,
            SnapshotUnleasedTempMaintenanceFailureKind::InternalState,
        ),
        (
            SnapshotCodecErrorKind::InvalidMagic,
            SnapshotUnleasedTempMaintenanceFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::InvalidLength,
            SnapshotUnleasedTempMaintenanceFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::ChecksumMismatch,
            SnapshotUnleasedTempMaintenanceFailureKind::CorruptData,
        ),
        (
            SnapshotCodecErrorKind::CorruptData,
            SnapshotUnleasedTempMaintenanceFailureKind::CorruptData,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::Codec(input), expected);
    }
    for (input, expected) in [
        (
            SnapshotStorageErrorKind::InvalidConfiguration,
            SnapshotUnleasedTempMaintenanceFailureKind::InternalState,
        ),
        (
            SnapshotStorageErrorKind::UnsafeRoot,
            SnapshotUnleasedTempMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::UnsafeObject,
            SnapshotUnleasedTempMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::UnrecognizedStore,
            SnapshotUnleasedTempMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            SnapshotStorageErrorKind::Unavailable,
            SnapshotUnleasedTempMaintenanceFailureKind::Unavailable,
        ),
        (
            SnapshotStorageErrorKind::Busy,
            SnapshotUnleasedTempMaintenanceFailureKind::Busy,
        ),
        (
            SnapshotStorageErrorKind::InternalState,
            SnapshotUnleasedTempMaintenanceFailureKind::InternalState,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::Storage(input), expected);
    }
    for (input, expected) in [
        (
            HistoryErrorKind::InvalidInput,
            SnapshotUnleasedTempMaintenanceFailureKind::InvalidClock,
        ),
        (
            HistoryErrorKind::IncompatibleSchema,
            SnapshotUnleasedTempMaintenanceFailureKind::IncompatibleSchema,
        ),
        (
            HistoryErrorKind::QueryLimitExceeded,
            SnapshotUnleasedTempMaintenanceFailureKind::BudgetExceeded,
        ),
        (
            HistoryErrorKind::Busy,
            SnapshotUnleasedTempMaintenanceFailureKind::Busy,
        ),
        (
            HistoryErrorKind::UnsafeStorage,
            SnapshotUnleasedTempMaintenanceFailureKind::UnsafeStorage,
        ),
        (
            HistoryErrorKind::CorruptData,
            SnapshotUnleasedTempMaintenanceFailureKind::CorruptData,
        ),
        (
            HistoryErrorKind::DatabaseUnavailable,
            SnapshotUnleasedTempMaintenanceFailureKind::Unavailable,
        ),
        (
            HistoryErrorKind::OutcomeUnknown,
            SnapshotUnleasedTempMaintenanceFailureKind::OutcomeUnknown,
        ),
        (
            HistoryErrorKind::AlreadyExists,
            SnapshotUnleasedTempMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::NotFound,
            SnapshotUnleasedTempMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::InvalidTransition,
            SnapshotUnleasedTempMaintenanceFailureKind::InternalState,
        ),
        (
            HistoryErrorKind::InternalState,
            SnapshotUnleasedTempMaintenanceFailureKind::InternalState,
        ),
    ] {
        assert_mapping(SnapshotRepositoryErrorKind::History(input), expected);
    }

    for (input, expected) in [
        (
            SnapshotUnleasedTempReconciliationBatchOutcome::NoUnleasedTemp,
            SnapshotUnleasedTempMaintenanceOutcome::NoUnleasedTemp,
        ),
        (
            SnapshotUnleasedTempReconciliationBatchOutcome::DeferredActive,
            SnapshotUnleasedTempMaintenanceOutcome::DeferredActive,
        ),
        (
            SnapshotUnleasedTempReconciliationBatchOutcome::Removed { bytes: 42 },
            SnapshotUnleasedTempMaintenanceOutcome::Removed { bytes: 42 },
        ),
    ] {
        assert_eq!(
            public_snapshot_unleased_temp_maintenance_outcome(&input),
            expected
        );
    }
}

#[test]
fn invalid_unleased_temp_clock_schema_race_and_panic_release_exclusive_admission() {
    let temp = TempDir::new().unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 2, 8, 8)).unwrap();
    let invalid = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_at(
                SystemTime::UNIX_EPOCH - Duration::from_millis(1),
            )
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, invalid).failure,
        Some(TaskFailureKind::SnapshotUnleasedTempMaintenance(
            SnapshotUnleasedTempMaintenanceFailureKind::InvalidClock
        ))
    );

    let panicking = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(65_000),
                || panic!("snapshot unleased-temp hook panic"),
                || {},
                || {},
            )
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, panicking).phase, TaskPhase::Failed);
    let replacement = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_at(
                SystemTime::UNIX_EPOCH + Duration::from_millis(65_001),
            )
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );

    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let racing = started_snapshot_unleased_temp_maintenance(
        engine
            .start_snapshot_unleased_temp_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(65_002),
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
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
                 VALUES (?1, 'future-unleased-temp-maintenance-race', zeroblob(32), 2)",
                [i64::from(future)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", future)
            .unwrap();
    });
    release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&engine, racing).failure,
        Some(TaskFailureKind::SnapshotUnleasedTempMaintenance(
            SnapshotUnleasedTempMaintenanceFailureKind::IncompatibleSchema
        ))
    );
}

#[test]
fn snapshot_unleased_temp_maintenance_serializes_across_engine_sessions() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let first_engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    let private_seed = b"cross-session-unleased-temp-private";
    first_engine
        .inner
        .snapshots
        .leave_unleased_snapshot_temp_for_test(private_seed, false)
        .unwrap();
    let second_engine =
        EngineHandle::open_with_limits(config, RegistryLimits::testing(1, 2, 8, 8)).unwrap();
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(66_000);
    let (applying_tx, applying_rx) = mpsc::channel();
    let (first_release_tx, first_release_rx) = mpsc::channel();
    let (second_release_tx, second_release_rx) = mpsc::channel();
    let first_ready = applying_tx.clone();
    let first = started_snapshot_unleased_temp_maintenance(
        first_engine
            .start_snapshot_unleased_temp_maintenance_with_test_hooks(
                observed,
                || {},
                move || {
                    first_ready.send(()).unwrap();
                    first_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    let second = started_snapshot_unleased_temp_maintenance(
        second_engine
            .start_snapshot_unleased_temp_maintenance_with_test_hooks(
                observed,
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    second_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    first_release_tx.send(()).unwrap();
    second_release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&first_engine, first).phase,
        TaskPhase::Succeeded
    );
    assert_eq!(
        wait_terminal(&second_engine, second).phase,
        TaskPhase::Succeeded
    );
    let outcomes = [
        first_engine
            .snapshot_unleased_temp_maintenance_result(first)
            .unwrap()
            .unwrap()
            .outcome(),
        second_engine
            .snapshot_unleased_temp_maintenance_result(second)
            .unwrap()
            .unwrap()
            .outcome(),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                SnapshotUnleasedTempMaintenanceOutcome::Removed { .. }
            ))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                SnapshotUnleasedTempMaintenanceOutcome::NoUnleasedTemp
            ))
            .count(),
        1
    );
    for (engine, id) in [(&first_engine, first), (&second_engine, second)] {
        let result = engine
            .snapshot_unleased_temp_maintenance_result(id)
            .unwrap()
            .unwrap();
        assert!(!format!("{result:?}").contains(std::str::from_utf8(private_seed).unwrap()));
    }
}

#[cfg(unix)]
fn create_engine_provisioning_stage(
    database: &std::path::Path,
    suffix: &str,
    marker_complete: bool,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let stage = database
        .parent()
        .unwrap()
        .join(format!(".dux-snapshot-stage-{suffix}"));
    std::fs::create_dir(&stage).unwrap();
    std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o700)).unwrap();
    let marker = stage.join(".dux-snapshot-store");
    std::fs::write(&marker, b"DUXSNAPSTOREV1\0\0").unwrap();
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).unwrap();
    if marker_complete {
        let writer = stage.join(".dux-snapshot.writer.lock");
        std::fs::write(&writer, b"DUXSNAPWRITER1\0\0").unwrap();
        std::fs::set_permissions(&writer, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    stage
}

#[test]
fn snapshot_provisioning_stage_maintenance_runs_one_typed_path_free_noop_batch() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 2, 4, 8));
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(67_000);
    let id = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_at(observed)
            .unwrap(),
    );
    let terminal = wait_terminal(&engine, id);
    assert_eq!(
        terminal.kind,
        TaskKind::SnapshotProvisioningStageMaintenance
    );
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.result_available);
    let result = engine
        .snapshot_provisioning_stage_maintenance_result(id)
        .unwrap()
        .unwrap();
    assert_eq!(result.observed_at(), observed);
    assert_eq!(
        result.outcome(),
        SnapshotProvisioningStageMaintenanceOutcome::NoStage
    );
    assert_eq!(result.total_stage_count_before(), 0);
    assert_eq!(result.total_stage_count_after(), 0);
    assert_eq!(result.marker_owned_count_before(), 0);
    assert_eq!(result.marker_owned_count_after(), 0);
    assert_eq!(result.unproven_count_before(), 0);
    assert_eq!(result.unproven_count_after(), 0);
    assert_eq!(result.control_charged_bytes_before(), 0);
    assert_eq!(result.control_charged_bytes_after(), 0);
    assert!(!result.has_more());
    assert_eq!(
        engine
            .snapshot_unleased_temp_maintenance_result(id)
            .unwrap_err(),
        TaskAccessError::WrongTaskKind
    );
    let debug = format!("{result:?}");
    assert!(!debug.contains(".dux-snapshot-stage-"));
    assert!(!debug.contains("snapshots"));

    let events = engine.task_events(id, 0, 8).unwrap().events;
    let applying = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::SnapshotProvisioningStageMaintenanceBatchApplying
            )
        })
        .unwrap();
    let finished = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                TaskEventKind::SnapshotProvisioningStageMaintenanceBatchFinished {
                    outcome: SnapshotProvisioningStageMaintenanceOutcome::NoStage,
                    total_stage_count_before: 0,
                    total_stage_count_after: 0,
                    marker_owned_count_before: 0,
                    marker_owned_count_after: 0,
                    unproven_count_before: 0,
                    unproven_count_after: 0,
                    control_charged_bytes_before: 0,
                    control_charged_bytes_after: 0,
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

#[cfg(unix)]
#[test]
fn snapshot_provisioning_stage_maintenance_removes_one_per_batch_and_does_not_spin_unproven() {
    let temp = TempDir::new().unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 4, 16, 16))
            .unwrap();
    let database = engine.inner.config.database_path();
    let marker_only =
        create_engine_provisioning_stage(database, "00000000000000000000000000000000", false);
    let marker_complete =
        create_engine_provisioning_stage(database, "11111111111111111111111111111111", true);
    let base = SystemTime::UNIX_EPOCH + Duration::from_millis(68_000);

    let first = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_at(base)
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, first).phase, TaskPhase::Succeeded);
    let first_result = engine
        .snapshot_provisioning_stage_maintenance_result(first)
        .unwrap()
        .unwrap();
    let first_bytes = match first_result.outcome() {
        SnapshotProvisioningStageMaintenanceOutcome::RemovedMarkerOnly { bytes } => bytes,
        other => panic!("expected marker-only removal, got {other:?}"),
    };
    assert!(first_bytes > 0);
    assert_eq!(first_result.total_stage_count_before(), 2);
    assert_eq!(first_result.total_stage_count_after(), 1);
    assert_eq!(first_result.marker_owned_count_before(), 2);
    assert_eq!(first_result.marker_owned_count_after(), 1);
    assert_eq!(first_result.unproven_count_before(), 0);
    assert_eq!(first_result.unproven_count_after(), 0);
    assert_eq!(
        first_result.control_charged_bytes_before() - first_bytes,
        first_result.control_charged_bytes_after()
    );
    assert!(first_result.has_more());
    assert!(!marker_only.exists());
    assert!(marker_complete.exists());
    let first_debug = format!("{first_result:?}");
    assert!(!first_debug.contains("00000000000000000000000000000000"));
    assert!(!first_debug.contains(database.parent().unwrap().to_string_lossy().as_ref()));

    let second = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_at(base + Duration::from_millis(1))
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, second).phase, TaskPhase::Succeeded);
    let second_result = engine
        .snapshot_provisioning_stage_maintenance_result(second)
        .unwrap()
        .unwrap();
    assert!(matches!(
        second_result.outcome(),
        SnapshotProvisioningStageMaintenanceOutcome::RemovedMarkerComplete { bytes }
            if bytes > first_bytes
    ));
    assert_eq!(second_result.total_stage_count_before(), 1);
    assert_eq!(second_result.total_stage_count_after(), 0);
    assert_eq!(second_result.marker_owned_count_after(), 0);
    assert_eq!(second_result.control_charged_bytes_after(), 0);
    assert!(!second_result.has_more());
    assert!(!marker_complete.exists());
    let second_debug = format!("{second_result:?}");
    assert!(!second_debug.contains("11111111111111111111111111111111"));
    assert!(!second_debug.contains(database.parent().unwrap().to_string_lossy().as_ref()));

    let unproven = database
        .parent()
        .unwrap()
        .join(".dux-snapshot-stage-22222222222222222222222222222222");
    std::fs::create_dir(&unproven).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&unproven, std::fs::Permissions::from_mode(0o700)).unwrap();
    let deferred = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_at(base + Duration::from_millis(2))
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, deferred).phase, TaskPhase::Succeeded);
    let deferred_result = engine
        .snapshot_provisioning_stage_maintenance_result(deferred)
        .unwrap()
        .unwrap();
    assert_eq!(
        deferred_result.outcome(),
        SnapshotProvisioningStageMaintenanceOutcome::DeferredUnproven
    );
    assert_eq!(deferred_result.total_stage_count_before(), 1);
    assert_eq!(deferred_result.total_stage_count_after(), 1);
    assert_eq!(deferred_result.marker_owned_count_before(), 0);
    assert_eq!(deferred_result.unproven_count_before(), 1);
    assert!(!deferred_result.has_more());
    assert!(unproven.exists());
}

#[test]
fn snapshot_provisioning_stage_maintenance_is_idle_deduplicated_and_cross_exclusive() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 3, 16, 8));
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
    assert_eq!(
        engine
            .start_snapshot_provisioning_stage_maintenance()
            .unwrap(),
        SnapshotProvisioningStageMaintenanceStartOutcome::DeferredBusy
    );
    foreground_release_tx.send(()).unwrap();
    wait_terminal(&engine, foreground);

    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let stage = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(69_000),
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine
            .start_snapshot_provisioning_stage_maintenance()
            .unwrap(),
        SnapshotProvisioningStageMaintenanceStartOutcome::AlreadyActive(stage)
    );
    assert_eq!(
        engine.start_history_maintenance().unwrap(),
        HistoryMaintenanceStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.start_snapshot_retention().unwrap(),
        SnapshotRetentionStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.start_snapshot_orphan_maintenance().unwrap(),
        SnapshotOrphanMaintenanceStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.start_snapshot_terminal_temp_maintenance().unwrap(),
        SnapshotTerminalTempMaintenanceStartOutcome::DeferredBusy
    );
    assert_eq!(
        engine.start_snapshot_unleased_temp_maintenance().unwrap(),
        SnapshotUnleasedTempMaintenanceStartOutcome::DeferredBusy
    );
    release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, stage).phase, TaskPhase::Succeeded);
}

#[cfg(unix)]
#[test]
fn snapshot_provisioning_stage_cancellation_and_close_are_linearized_at_applying() {
    let temp = TempDir::new().unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 2, 8, 8)).unwrap();
    let database = engine.inner.config.database_path();
    let stage =
        create_engine_provisioning_stage(database, "33333333333333333333333333333333", false);
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(70_000);

    let (before_tx, before_rx) = mpsc::channel();
    let (before_release_tx, before_release_rx) = mpsc::channel();
    let cancelled = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_with_test_hooks(
                observed,
                move || {
                    before_tx.send(()).unwrap();
                    before_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    before_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(cancelled).unwrap(),
        CancelOutcome::Requested
    );
    before_release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&engine, cancelled).phase,
        TaskPhase::Cancelled
    );
    assert!(stage.exists());

    let (applying_tx, applying_rx) = mpsc::channel();
    let (applying_release_tx, applying_release_rx) = mpsc::channel();
    let applied = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_with_test_hooks(
                observed + Duration::from_millis(1),
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    applying_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(
        engine.cancel_task(applied).unwrap(),
        CancelOutcome::Requested
    );
    applying_release_tx.send(()).unwrap();
    let terminal = wait_terminal(&engine, applied);
    assert_eq!(terminal.phase, TaskPhase::Succeeded);
    assert!(terminal.cancellation_requested);
    assert!(!stage.exists());

    let close_stage =
        create_engine_provisioning_stage(database, "44444444444444444444444444444444", false);
    let (close_tx, close_rx) = mpsc::channel();
    let (close_release_tx, close_release_rx) = mpsc::channel();
    let closing = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_with_test_hooks(
                observed + Duration::from_millis(2),
                move || {
                    close_tx.send(()).unwrap();
                    close_release_rx.recv().unwrap();
                },
                || {},
                || {},
            )
            .unwrap(),
    );
    close_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(engine.close(), CloseOutcome::Initiated);
    close_release_tx.send(()).unwrap();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    let registry = engine.inner.shared.lock_registry_recover();
    let record = registry.records.get(&closing).unwrap();
    assert_eq!(record.phase, TaskPhase::Cancelled);
    assert!(close_stage.exists());
}

#[cfg(unix)]
#[test]
fn snapshot_provisioning_stage_close_after_applying_preserves_exact_outcome() {
    let temp = TempDir::new().unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 2, 8, 8)).unwrap();
    let stage = create_engine_provisioning_stage(
        engine.inner.config.database_path(),
        "55555555555555555555555555555555",
        false,
    );
    let (applying_tx, applying_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let task = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(70_003),
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    assert_eq!(engine.close(), CloseOutcome::Initiated);
    release_tx.send(()).unwrap();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    let registry = engine.inner.shared.lock_registry_recover();
    let record = registry.records.get(&task).unwrap();
    assert_eq!(record.phase, TaskPhase::Succeeded);
    assert!(record.cancellation_requested);
    assert!(matches!(
        record.result,
        Some(TaskResult::SnapshotProvisioningStageMaintenance(_))
    ));
    assert!(!stage.exists());
}

#[cfg(unix)]
#[test]
fn snapshot_provisioning_stage_maintenance_serializes_across_engine_sessions() {
    let temp = TempDir::new().unwrap();
    let config = config(&temp);
    let first_engine =
        EngineHandle::open_with_limits(config.clone(), RegistryLimits::testing(1, 2, 8, 8))
            .unwrap();
    let stage = create_engine_provisioning_stage(
        first_engine.inner.config.database_path(),
        "66666666666666666666666666666666",
        true,
    );
    let second_engine =
        EngineHandle::open_with_limits(config, RegistryLimits::testing(1, 2, 8, 8)).unwrap();
    let observed = SystemTime::UNIX_EPOCH + Duration::from_millis(70_004);
    let (applying_tx, applying_rx) = mpsc::channel();
    let (first_release_tx, first_release_rx) = mpsc::channel();
    let (second_release_tx, second_release_rx) = mpsc::channel();
    let first_ready = applying_tx.clone();
    let first = started_snapshot_provisioning_stage_maintenance(
        first_engine
            .start_snapshot_provisioning_stage_maintenance_with_test_hooks(
                observed,
                || {},
                move || {
                    first_ready.send(()).unwrap();
                    first_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    let second = started_snapshot_provisioning_stage_maintenance(
        second_engine
            .start_snapshot_provisioning_stage_maintenance_with_test_hooks(
                observed,
                || {},
                move || {
                    applying_tx.send(()).unwrap();
                    second_release_rx.recv().unwrap();
                },
                || {},
            )
            .unwrap(),
    );
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    applying_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    first_release_tx.send(()).unwrap();
    second_release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&first_engine, first).phase,
        TaskPhase::Succeeded
    );
    assert_eq!(
        wait_terminal(&second_engine, second).phase,
        TaskPhase::Succeeded
    );
    let outcomes = [
        first_engine
            .snapshot_provisioning_stage_maintenance_result(first)
            .unwrap()
            .unwrap()
            .outcome(),
        second_engine
            .snapshot_provisioning_stage_maintenance_result(second)
            .unwrap()
            .unwrap()
            .outcome(),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                SnapshotProvisioningStageMaintenanceOutcome::RemovedMarkerComplete { .. }
            ))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                SnapshotProvisioningStageMaintenanceOutcome::NoStage
            ))
            .count(),
        1
    );
    assert!(!stage.exists());
    for (engine, id) in [(&first_engine, first), (&second_engine, second)] {
        let result = engine
            .snapshot_provisioning_stage_maintenance_result(id)
            .unwrap()
            .unwrap();
        let debug = format!("{result:?}");
        assert!(!debug.contains("66666666666666666666666666666666"));
        assert!(!debug.contains(temp.path().to_string_lossy().as_ref()));
    }
}

#[test]
fn snapshot_provisioning_stage_mappings_invalid_clock_schema_and_panic_release() {
    for (input, expected) in [
        (
            SnapshotProvisioningStageReconciliationBatchOutcome::NoStage,
            SnapshotProvisioningStageMaintenanceOutcome::NoStage,
        ),
        (
            SnapshotProvisioningStageReconciliationBatchOutcome::DeferredUnproven,
            SnapshotProvisioningStageMaintenanceOutcome::DeferredUnproven,
        ),
        (
            SnapshotProvisioningStageReconciliationBatchOutcome::RemovedMarkerOnly { bytes: 42 },
            SnapshotProvisioningStageMaintenanceOutcome::RemovedMarkerOnly { bytes: 42 },
        ),
        (
            SnapshotProvisioningStageReconciliationBatchOutcome::RemovedMarkerComplete {
                bytes: 43,
            },
            SnapshotProvisioningStageMaintenanceOutcome::RemovedMarkerComplete { bytes: 43 },
        ),
    ] {
        assert_eq!(
            public_snapshot_provisioning_stage_maintenance_outcome(&input),
            expected
        );
    }
    assert_eq!(
        map_snapshot_provisioning_stage_maintenance_failure(SnapshotRepositoryErrorKind::History(
            HistoryErrorKind::OutcomeUnknown
        )),
        TaskFailureKind::SnapshotProvisioningStageMaintenance(
            SnapshotProvisioningStageMaintenanceFailureKind::OutcomeUnknown
        )
    );
    assert_eq!(
        map_snapshot_provisioning_stage_maintenance_failure(SnapshotRepositoryErrorKind::Storage(
            SnapshotStorageErrorKind::UnsafeObject
        )),
        TaskFailureKind::SnapshotProvisioningStageMaintenance(
            SnapshotProvisioningStageMaintenanceFailureKind::UnsafeStorage
        )
    );

    let temp = TempDir::new().unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 2, 8, 8)).unwrap();
    let invalid = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_at(
                SystemTime::UNIX_EPOCH - Duration::from_millis(1),
            )
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, invalid).failure,
        Some(TaskFailureKind::SnapshotProvisioningStageMaintenance(
            SnapshotProvisioningStageMaintenanceFailureKind::InvalidClock
        ))
    );
    let panicking = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(71_000),
                || panic!("snapshot provisioning-stage hook panic"),
                || {},
                || {},
            )
            .unwrap(),
    );
    assert_eq!(wait_terminal(&engine, panicking).phase, TaskPhase::Failed);
    let replacement = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_at(
                SystemTime::UNIX_EPOCH + Duration::from_millis(71_001),
            )
            .unwrap(),
    );
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );

    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let racing = started_snapshot_provisioning_stage_maintenance(
        engine
            .start_snapshot_provisioning_stage_maintenance_with_test_hooks(
                SystemTime::UNIX_EPOCH + Duration::from_millis(71_002),
                move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
                || {},
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
                 VALUES (?1, 'future-provisioning-stage-maintenance-race', zeroblob(32), 2)",
                [i64::from(future)],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", future)
            .unwrap();
    });
    release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&engine, racing).failure,
        Some(TaskFailureKind::SnapshotProvisioningStageMaintenance(
            SnapshotProvisioningStageMaintenanceFailureKind::IncompatibleSchema
        ))
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
    let candidate_history = engine.candidate_history_for_scan(&scan_id).unwrap();
    assert_eq!(candidate_history.scan_id(), &scan_id);
    assert_eq!(
        candidate_history.source_scan_status(),
        DurableScanStatus::Succeeded
    );
    assert!(candidate_history.scheduled_at().is_some());
    assert!(candidate_history.completed_at().is_some());
    assert_eq!(
        candidate_history.status(),
        DurableCandidateEvaluationStatus::Succeeded { candidate_count: 0 }
    );
    assert!(candidate_history.candidates().is_empty());
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
        reopened.candidate_history_for_scan(&scan_id).unwrap(),
        candidate_history
    );
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
    let candidate_history = engine.candidate_history_for_scan(&scan_id).unwrap();
    assert_eq!(
        candidate_history.status(),
        DurableCandidateEvaluationStatus::Succeeded { candidate_count: 1 }
    );
    assert_eq!(
        candidate_history.source_scan_status(),
        DurableScanStatus::Succeeded
    );
    let public_candidate = &candidate_history.candidates()[0];
    assert_eq!(
        public_candidate.rule().id().as_str(),
        "developer.rust.target"
    );
    assert_eq!(
        public_candidate.category(),
        crate::CandidateCategory::DeveloperArtifact
    );
    assert!(public_candidate.estimated_bytes() > 0);
    assert_eq!(public_candidate.path_count(), 1);
    assert_eq!(public_candidate.safety(), crate::SafetyTier::Informational);
    assert_eq!(
        public_candidate.action(),
        crate::CandidateAction::RevealOnly
    );
    assert!(!public_candidate.rule_schedule_eligible());
    assert_eq!(
        public_candidate.blockers(),
        [crate::BlockReason::ProtectedPath]
    );
    assert!(
        public_candidate
            .evidence_kinds()
            .contains(&crate::EvidenceKind::RequiredMarker)
    );
    assert_eq!(
        public_candidate.status(),
        DurableCandidateStatus::Discovered
    );
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
    assert_eq!(
        reopened.candidate_history_for_scan(&scan_id).unwrap(),
        candidate_history
    );
    let reopened_evaluation = reopened
        .inner
        .store
        .load_candidate_evaluation(&scan_id)
        .unwrap()
        .unwrap();
    assert_eq!(reopened_evaluation, evaluation);
}

#[test]
fn candidate_detail_pages_are_bounded_contiguous_and_survive_reopen() {
    let (_temp, config, engine, scan_id, candidate_id, expected_target) =
        completed_marker_candidate();

    assert_eq!(
        engine.candidate_path_page(&scan_id, &candidate_id, 0, 0),
        Err(CandidateDetailError::InvalidLimit {
            maximum: MAX_CANDIDATE_DETAIL_PAGE_LIMIT,
        })
    );
    assert_eq!(
        engine.candidate_evidence_page(
            &scan_id,
            &candidate_id,
            0,
            MAX_CANDIDATE_DETAIL_PAGE_LIMIT + 1,
        ),
        Err(CandidateDetailError::InvalidLimit {
            maximum: MAX_CANDIDATE_DETAIL_PAGE_LIMIT,
        })
    );

    let paths = engine
        .candidate_path_page(&scan_id, &candidate_id, 0, 1)
        .unwrap();
    assert_eq!(paths.scan_id(), &scan_id);
    assert_eq!(paths.candidate().id(), &candidate_id);
    assert_eq!(paths.cursor(), 0);
    assert_eq!(paths.total_paths(), 1);
    assert_eq!(paths.next_cursor(), None);
    assert_eq!(paths.paths().len(), 1);
    assert_eq!(paths.paths()[0].ordinal(), 0);
    assert_eq!(
        paths.paths()[0].path().display(),
        expected_target.display().to_string()
    );
    assert!(!paths.paths()[0].path().encoded_bytes().is_empty());
    #[cfg(windows)]
    assert_eq!(
        paths.paths()[0].path().encoding(),
        DurablePathEncoding::Utf16LittleEndian
    );
    #[cfg(not(windows))]
    assert_eq!(
        paths.paths()[0].path().encoding(),
        DurablePathEncoding::Utf8
    );

    let path_end = engine
        .candidate_path_page(&scan_id, &candidate_id, paths.total_paths(), 1)
        .unwrap();
    assert!(path_end.paths().is_empty());
    assert_eq!(path_end.next_cursor(), None);
    assert_eq!(
        engine.candidate_path_page(&scan_id, &candidate_id, paths.total_paths() + 1, 1),
        Err(CandidateDetailError::CursorOutOfRange)
    );

    let first_evidence = engine
        .candidate_evidence_page(&scan_id, &candidate_id, 0, 1)
        .unwrap();
    assert_eq!(first_evidence.scan_id(), &scan_id);
    assert_eq!(first_evidence.candidate().id(), &candidate_id);
    assert_eq!(first_evidence.cursor(), 0);
    assert_eq!(first_evidence.evidence().len(), 1);
    assert_eq!(first_evidence.evidence()[0].ordinal(), 0);
    assert_eq!(first_evidence.next_cursor(), Some(1));
    let mut observed = first_evidence.evidence().to_vec();
    let mut cursor = first_evidence.next_cursor();
    while let Some(next) = cursor {
        let page = engine
            .candidate_evidence_page(&scan_id, &candidate_id, next, 1)
            .unwrap();
        observed.extend_from_slice(page.evidence());
        cursor = page.next_cursor();
    }
    assert_eq!(observed.len(), usize::from(first_evidence.total_evidence()));
    for (ordinal, item) in observed.iter().enumerate() {
        assert_eq!(item.ordinal(), u16::try_from(ordinal).unwrap());
    }

    engine.close();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    drop(engine);
    let reopened = EngineHandle::open(config).unwrap();
    assert_eq!(
        reopened
            .candidate_path_page(&scan_id, &candidate_id, 0, 1)
            .unwrap(),
        paths
    );
    assert_eq!(
        reopened
            .candidate_evidence_page(&scan_id, &candidate_id, 0, 1)
            .unwrap(),
        first_evidence
    );
}

#[test]
fn candidate_detail_distinguishes_scan_evaluation_candidate_and_lifecycle_failures() {
    let (_temp, _config, engine, scan_id, candidate_id, _target) = completed_marker_candidate();
    let missing_scan = ScanId::new("scan:missing-detail").unwrap();
    let missing_candidate = crate::CandidateId::new("candidate:missing-detail").unwrap();
    assert_eq!(
        engine.candidate_path_page(&missing_scan, &candidate_id, 0, 1),
        Err(CandidateDetailError::ScanNotFound)
    );
    assert_eq!(
        engine.candidate_path_page(&scan_id, &missing_candidate, 0, 1),
        Err(CandidateDetailError::CandidateNotFound)
    );

    let cancelled_scan = ScanId::new("scan:detail-not-run").unwrap();
    engine
        .inner
        .store
        .record_scan_started(
            &NewScanRecord::try_new(
                cancelled_scan.clone(),
                engine.config().cache_directory().join("detail-not-run"),
                SystemTime::UNIX_EPOCH + Duration::from_secs(1),
            )
            .unwrap(),
        )
        .unwrap();
    engine
        .inner
        .store
        .record_scan_finished(
            &ScanCompletionRecord::try_new_with_coverage(
                cancelled_scan.clone(),
                SystemTime::UNIX_EPOCH + Duration::from_secs(2),
                TerminalScanStatus::Cancelled,
                ScanCounts::default(),
                ScanCoverage::unknown(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        engine.candidate_evidence_page(&cancelled_scan, &candidate_id, 0, 1),
        Err(CandidateDetailError::EvaluationNotSucceeded)
    );

    engine.close();
    assert!(engine.wait_until_closed(TEST_TIMEOUT));
    assert_eq!(
        engine.candidate_path_page(&scan_id, &candidate_id, 0, 1),
        Err(CandidateDetailError::Closed)
    );
    assert_eq!(
        engine.review_candidate(&scan_id, &candidate_id, CandidateReviewCommand::Dismiss),
        Err(CandidateReviewError::Closed)
    );
}

#[test]
fn every_candidate_evidence_variant_maps_to_a_distinct_non_authoritative_dto() {
    #[cfg(windows)]
    let path = PathBuf::from(r"C:\fixture\observed");
    #[cfg(not(windows))]
    let path = PathBuf::from("/fixture/observed");
    let newest_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    let evidence = [
        Evidence::MatchedPath { path: path.clone() },
        Evidence::RequiredMarker { path: path.clone() },
        Evidence::ForbiddenMarkerAbsent { path: path.clone() },
        Evidence::BundleIdentifier {
            path: path.clone(),
            identifier: "com.example.fixture".to_owned(),
        },
        Evidence::MinimumAge {
            newest_mtime,
            minimum_age: Duration::from_secs(20),
        },
        Evidence::MinimumSize {
            observed_bytes: 30,
            minimum_bytes: 20,
        },
        Evidence::InactiveProcess {
            identifier: "fixture-process".to_owned(),
        },
        Evidence::CloudUploadComplete { path },
    ];
    let mapped = evidence
        .iter()
        .map(public_candidate_evidence)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(matches!(
        mapped[0],
        DurableCandidateEvidence::MatchedPath { .. }
    ));
    assert!(matches!(
        mapped[1],
        DurableCandidateEvidence::RequiredMarker { .. }
    ));
    assert!(matches!(
        mapped[2],
        DurableCandidateEvidence::ForbiddenMarkerAbsent { .. }
    ));
    assert!(matches!(
        &mapped[3],
        DurableCandidateEvidence::BundleIdentifier { identifier, .. }
            if identifier.as_ref() == "com.example.fixture"
    ));
    assert!(matches!(
        mapped[4],
        DurableCandidateEvidence::MinimumAge {
            newest_mtime: value,
            minimum_age
        } if value == newest_mtime && minimum_age == Duration::from_secs(20)
    ));
    assert!(matches!(
        mapped[5],
        DurableCandidateEvidence::MinimumSize {
            observed_bytes: 30,
            minimum_bytes: 20
        }
    ));
    assert!(matches!(
        &mapped[6],
        DurableCandidateEvidence::InactiveProcess { identifier }
            if identifier.as_ref() == "fixture-process"
    ));
    assert!(matches!(
        mapped[7],
        DurableCandidateEvidence::CloudUploadComplete { .. }
    ));
}

#[test]
fn semantic_candidate_review_is_scan_bound_idempotent_and_never_plans() {
    let (_temp, _config, engine, scan_id, candidate_id, _target) = completed_marker_candidate();

    assert_eq!(
        engine.review_candidate(&scan_id, &candidate_id, CandidateReviewCommand::Select),
        Err(CandidateReviewError::NotReviewable)
    );
    let dismissed = engine
        .review_candidate(&scan_id, &candidate_id, CandidateReviewCommand::Dismiss)
        .unwrap();
    assert_eq!(dismissed.scan_id(), &scan_id);
    assert_eq!(dismissed.candidate_id(), &candidate_id);
    assert_eq!(dismissed.status(), DurableCandidateStatus::Dismissed);
    assert_eq!(
        engine
            .review_candidate(&scan_id, &candidate_id, CandidateReviewCommand::Dismiss)
            .unwrap(),
        dismissed
    );
    assert_eq!(
        engine
            .candidate_path_page(&scan_id, &candidate_id, 0, 1)
            .unwrap()
            .candidate()
            .status(),
        DurableCandidateStatus::Dismissed
    );
    assert_eq!(
        engine
            .review_candidate(&scan_id, &candidate_id, CandidateReviewCommand::Restore)
            .unwrap()
            .status(),
        DurableCandidateStatus::Discovered
    );
    assert_eq!(
        engine.review_candidate(
            &ScanId::new("scan:wrong-review-source").unwrap(),
            &candidate_id,
            CandidateReviewCommand::Dismiss,
        ),
        Err(CandidateReviewError::CandidateNotFound)
    );
    engine.inner.store.with_connection(|connection| {
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM cleanup_sessions", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM candidate_plan_claims", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
    });
    assert_eq!(
        engine
            .candidate_history_for_scan(&scan_id)
            .unwrap()
            .candidates()[0]
            .status(),
        DurableCandidateStatus::Discovered
    );
}

#[test]
fn cleanup_eligible_candidate_supports_select_clear_and_selected_dismissal() {
    let (_temp, _config, engine, scan_id, candidate_id, _target) = completed_marker_candidate();
    make_marker_candidate_cleanup_reviewable(&engine, &scan_id);

    let selected = engine
        .review_candidate(&scan_id, &candidate_id, CandidateReviewCommand::Select)
        .unwrap();
    assert_eq!(selected.status(), DurableCandidateStatus::Selected);
    assert_eq!(
        engine
            .review_candidate(&scan_id, &candidate_id, CandidateReviewCommand::Select)
            .unwrap(),
        selected
    );
    assert_eq!(
        engine
            .review_candidate(
                &scan_id,
                &candidate_id,
                CandidateReviewCommand::ClearSelection,
            )
            .unwrap()
            .status(),
        DurableCandidateStatus::Discovered
    );
    engine
        .review_candidate(&scan_id, &candidate_id, CandidateReviewCommand::Select)
        .unwrap();
    assert_eq!(
        engine
            .review_candidate(&scan_id, &candidate_id, CandidateReviewCommand::Dismiss)
            .unwrap()
            .status(),
        DurableCandidateStatus::Dismissed
    );
    assert_eq!(
        engine
            .review_candidate(&scan_id, &candidate_id, CandidateReviewCommand::Restore)
            .unwrap()
            .status(),
        DurableCandidateStatus::Discovered
    );
}

#[test]
fn cleanup_history_is_empty_bounded_and_closed_with_typed_errors() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 4, 8, 16));
    let page = engine.recent_cleanup_history(None, 1).unwrap();
    assert!(page.records().is_empty());
    assert!(page.next_cursor().is_none());
    assert_eq!(
        engine.recent_cleanup_history(None, 0),
        Err(CleanupHistoryError::InvalidLimit {
            maximum: MAX_RECENT_CLEANUP_HISTORY_LIMIT
        })
    );
    assert_eq!(
        engine.recent_cleanup_history(None, MAX_RECENT_CLEANUP_HISTORY_LIMIT + 1),
        Err(CleanupHistoryError::InvalidLimit {
            maximum: MAX_RECENT_CLEANUP_HISTORY_LIMIT
        })
    );
    let missing = DurableCleanupSessionId::new("session:missing").unwrap();
    assert_eq!(
        engine.cleanup_session_history(&missing),
        Err(CleanupHistoryError::SessionNotFound)
    );
    engine.close();
    assert_eq!(
        engine.recent_cleanup_history(None, 1),
        Err(CleanupHistoryError::Closed)
    );
    assert_eq!(
        engine.cleanup_session_history(&missing),
        Err(CleanupHistoryError::Closed)
    );
}

#[test]
fn planned_cleanup_history_is_path_free_exact_and_survives_reopen() {
    let (_temp, config, engine, scan_id, candidate_id, target) = completed_marker_candidate();
    record_planned_cleanup_history(&engine, &scan_id, &candidate_id, "session:engine-history");
    let before = engine.inner.store.with_connection(|connection| {
        (
            connection
                .query_row("SELECT count(*) FROM cleanup_sessions", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            connection
                .query_row("SELECT count(*) FROM candidate_plan_claims", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            connection
                .query_row(
                    "SELECT status FROM candidates WHERE candidate_id = ?1",
                    [candidate_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
    });

    let page = engine.recent_cleanup_history(None, 64).unwrap();
    assert_eq!(page.records().len(), 1);
    let summary = &page.records()[0];
    assert_eq!(summary.id().as_str(), "session:engine-history");
    assert_eq!(summary.format(), DurableCleanupRecordFormat::Complete);
    assert_eq!(summary.source_scan_id(), Some(&scan_id));
    assert_eq!(summary.status(), DurableCleanupSessionStatus::Planned);
    assert_eq!(summary.mode(), DurableCleanupMode::PermanentSafe);
    assert_eq!(summary.trigger(), DurableCleanupTrigger::Manual);
    assert_eq!(summary.cancellation_requested(), Some(false));
    assert_eq!(summary.item_total(), 1);
    assert_eq!(summary.path_total(), 1);
    assert!(summary.evidence_total() >= 1);
    assert_eq!(summary.item_status_counts().planned(), 1);
    assert_eq!(summary.path_status_counts().planned(), 1);
    assert!(page.next_cursor().is_none());

    let session_id = summary.id().clone();
    let exact = engine.cleanup_session_history(&session_id).unwrap();
    assert_eq!(exact.summary(), summary);
    assert_eq!(exact.items().len(), 1);
    let item = &exact.items()[0];
    assert_eq!(item.ordinal(), 0);
    assert_eq!(item.status(), DurableCleanupItemStatus::Planned);
    assert_eq!(
        item.category(),
        Some(crate::CandidateCategory::DeveloperArtifact)
    );
    assert_eq!(item.safety(), Some(crate::SafetyTier::SafeRegenerable));
    assert_eq!(
        item.action(),
        Some(crate::CandidateAction::RemoveKnownRegenerableContents)
    );
    assert_eq!(item.rule_schedule_eligible(), Some(true));
    assert_eq!(item.path_count(), 1);
    assert!(item.evidence_count() >= 1);
    assert!(!item.error_recorded());
    assert!(item.error_category().is_none());
    assert!(
        exact
            .warnings()
            .contains(&DurableCleanupWarning::PermanentRemovalCannotBeUndone)
    );
    let after = engine.inner.store.with_connection(|connection| {
        (
            connection
                .query_row("SELECT count(*) FROM cleanup_sessions", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            connection
                .query_row("SELECT count(*) FROM candidate_plan_claims", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            connection
                .query_row(
                    "SELECT status FROM candidates WHERE candidate_id = ?1",
                    [candidate_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
    });
    assert_eq!(
        before, after,
        "history reads must not mutate journal authority"
    );
    assert!(
        !format!("{exact:?}").contains(&target.to_string_lossy().to_string()),
        "path-free cleanup history must not disclose the stored target"
    );

    engine.close();
    let reopened = EngineHandle::open(config).unwrap();
    assert_eq!(
        reopened
            .cleanup_session_history(&session_id)
            .unwrap()
            .summary(),
        summary
    );
}

#[test]
fn cleanup_history_keyset_order_and_legacy_incompleteness_are_explicit() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 4, 8, 16));
    insert_legacy_cleanup_history(
        &engine,
        "session:a",
        1_700_000_002_000,
        "completed",
        "removed",
        None,
    );
    let legacy_error = "é".repeat(128);
    insert_legacy_cleanup_history(
        &engine,
        "session:b",
        1_700_000_002_000,
        "failed",
        "failed",
        Some(&legacy_error),
    );
    insert_legacy_cleanup_history(
        &engine,
        "session:c",
        1_700_000_001_000,
        "dry_run",
        "dry_run",
        None,
    );

    let first = engine.recent_cleanup_history(None, 1).unwrap();
    assert_eq!(first.records()[0].id().as_str(), "session:a");
    let second = engine
        .recent_cleanup_history(first.next_cursor(), 1)
        .unwrap();
    assert_eq!(second.records()[0].id().as_str(), "session:b");
    let third = engine
        .recent_cleanup_history(second.next_cursor(), 1)
        .unwrap();
    assert_eq!(third.records()[0].id().as_str(), "session:c");
    assert!(third.next_cursor().is_none());

    let exact = engine
        .cleanup_session_history(second.records()[0].id())
        .unwrap();
    assert_eq!(
        exact.summary().format(),
        DurableCleanupRecordFormat::LegacyIncomplete
    );
    assert!(exact.summary().source_scan_id().is_none());
    assert!(exact.summary().plan_created_at().is_none());
    assert!(exact.summary().plan_expires_at().is_none());
    assert_eq!(exact.summary().verified_capacity_delta_bytes(), Some(7));
    assert_eq!(exact.items().len(), 1);
    assert_eq!(exact.items()[0].path_count(), 1);
    assert!(exact.items()[0].category().is_none());
    assert!(exact.items()[0].safety().is_none());
    assert!(exact.items()[0].action().is_none());
    assert!(exact.items()[0].error_recorded());
    assert!(exact.items()[0].error_category().is_none());
}

#[test]
fn cleanup_history_status_and_error_mappings_are_exhaustive() {
    let session_statuses = [
        StoredCleanupSessionStatus::Planned,
        StoredCleanupSessionStatus::Running,
        StoredCleanupSessionStatus::Recovering,
        StoredCleanupSessionStatus::Completed,
        StoredCleanupSessionStatus::PartiallyCompleted,
        StoredCleanupSessionStatus::Failed,
        StoredCleanupSessionStatus::Cancelled,
        StoredCleanupSessionStatus::Interrupted,
        StoredCleanupSessionStatus::Rejected,
        StoredCleanupSessionStatus::DryRun,
    ];
    assert_eq!(
        session_statuses.map(public_cleanup_session_status),
        [
            DurableCleanupSessionStatus::Planned,
            DurableCleanupSessionStatus::Running,
            DurableCleanupSessionStatus::Recovering,
            DurableCleanupSessionStatus::Completed,
            DurableCleanupSessionStatus::PartiallyCompleted,
            DurableCleanupSessionStatus::Failed,
            DurableCleanupSessionStatus::Cancelled,
            DurableCleanupSessionStatus::Interrupted,
            DurableCleanupSessionStatus::Rejected,
            DurableCleanupSessionStatus::DryRun,
        ]
    );
    let item_statuses = [
        StoredCleanupItemStatus::Planned,
        StoredCleanupItemStatus::Validating,
        StoredCleanupItemStatus::DryRun,
        StoredCleanupItemStatus::EffectStarted,
        StoredCleanupItemStatus::Trashed,
        StoredCleanupItemStatus::Removed,
        StoredCleanupItemStatus::Evicted,
        StoredCleanupItemStatus::Skipped,
        StoredCleanupItemStatus::Rejected,
        StoredCleanupItemStatus::Failed,
        StoredCleanupItemStatus::ChangedSincePlan,
        StoredCleanupItemStatus::Interrupted,
        StoredCleanupItemStatus::Unavailable,
        StoredCleanupItemStatus::OutcomeUnknown,
    ];
    assert_eq!(
        item_statuses.map(public_cleanup_item_status),
        [
            DurableCleanupItemStatus::Planned,
            DurableCleanupItemStatus::Validating,
            DurableCleanupItemStatus::DryRun,
            DurableCleanupItemStatus::EffectStarted,
            DurableCleanupItemStatus::Trashed,
            DurableCleanupItemStatus::Removed,
            DurableCleanupItemStatus::Evicted,
            DurableCleanupItemStatus::Skipped,
            DurableCleanupItemStatus::Rejected,
            DurableCleanupItemStatus::Failed,
            DurableCleanupItemStatus::ChangedSincePlan,
            DurableCleanupItemStatus::Interrupted,
            DurableCleanupItemStatus::Unavailable,
            DurableCleanupItemStatus::OutcomeUnknown,
        ]
    );
    for (kind, expected) in [
        (
            HistoryErrorKind::IncompatibleSchema,
            CleanupHistoryError::IncompatibleSchema,
        ),
        (
            HistoryErrorKind::QueryLimitExceeded,
            CleanupHistoryError::QueryLimitExceeded,
        ),
        (HistoryErrorKind::Busy, CleanupHistoryError::Busy),
        (
            HistoryErrorKind::UnsafeStorage,
            CleanupHistoryError::UnsafeStorage,
        ),
        (
            HistoryErrorKind::CorruptData,
            CleanupHistoryError::CorruptData,
        ),
        (
            HistoryErrorKind::InternalState,
            CleanupHistoryError::InternalState,
        ),
        (
            HistoryErrorKind::InvalidInput,
            CleanupHistoryError::InternalState,
        ),
        (
            HistoryErrorKind::DatabaseUnavailable,
            CleanupHistoryError::Unavailable,
        ),
        (
            HistoryErrorKind::AlreadyExists,
            CleanupHistoryError::Unavailable,
        ),
        (HistoryErrorKind::NotFound, CleanupHistoryError::Unavailable),
        (
            HistoryErrorKind::InvalidTransition,
            CleanupHistoryError::Unavailable,
        ),
        (
            HistoryErrorKind::OutcomeUnknown,
            CleanupHistoryError::Unavailable,
        ),
    ] {
        assert_eq!(map_cleanup_history_error(kind), expected);
    }
}

#[test]
fn cleanup_history_rejects_over_budget_graph_before_payload_decode() {
    let (_temp, _config, engine, scan_id, candidate_id, _target) = completed_marker_candidate();
    record_planned_cleanup_history(
        &engine,
        &scan_id,
        &candidate_id,
        "session:over-budget-history",
    );
    engine.inner.store.with_connection(|connection| {
        let first: i64 = connection
            .query_row(
                "SELECT count(*) FROM cleanup_item_evidence
                 WHERE session_id = 'session:over-budget-history' AND item_ordinal = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        connection
            .execute(
                "WITH RECURSIVE seq(value) AS (
                     SELECT 0 UNION ALL SELECT value + 1 FROM seq WHERE value < 349
                 )
                 INSERT INTO cleanup_item_evidence (
                     session_id, item_ordinal, evidence_ordinal, evidence_kind,
                     path_value, path_value_encoding
                 )
                 SELECT 'session:over-budget-history', 0, ?1 + value,
                        'required_marker', zeroblob(65536), 2
                 FROM seq",
                [first],
            )
            .unwrap();
    });
    let id = DurableCleanupSessionId::new("session:over-budget-history").unwrap();
    assert_eq!(
        engine.cleanup_session_history(&id),
        Err(CleanupHistoryError::QueryLimitExceeded)
    );
    assert_eq!(
        engine.recent_cleanup_history(None, 1),
        Err(CleanupHistoryError::QueryLimitExceeded)
    );
}

#[test]
fn cleanup_history_legal_maximum_page_uses_one_more_sentinel() {
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 4, 8, 16));
    for index in 0..=usize::from(MAX_RECENT_CLEANUP_HISTORY_LIMIT) {
        insert_legacy_cleanup_history(
            &engine,
            &format!("session:page-{index:03}"),
            1_700_100_000_000 - i64::try_from(index).unwrap(),
            "completed",
            "removed",
            None,
        );
    }
    let first = engine
        .recent_cleanup_history(None, MAX_RECENT_CLEANUP_HISTORY_LIMIT)
        .unwrap();
    assert_eq!(
        first.records().len(),
        usize::from(MAX_RECENT_CLEANUP_HISTORY_LIMIT)
    );
    assert!(first.next_cursor().is_some());
    let second = engine
        .recent_cleanup_history(first.next_cursor(), MAX_RECENT_CLEANUP_HISTORY_LIMIT)
        .unwrap();
    assert_eq!(second.records().len(), 1);
    assert_eq!(second.records()[0].id().as_str(), "session:page-064");
    assert!(second.next_cursor().is_none());
}

#[test]
fn cleanup_history_rejects_gapped_graph_without_partial_results() {
    let (_temp, _config, engine, scan_id, candidate_id, _target) = completed_marker_candidate();
    record_planned_cleanup_history(&engine, &scan_id, &candidate_id, "session:corrupt-history");
    engine.inner.store.with_connection(|connection| {
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        connection
            .execute(
                "UPDATE cleanup_items SET item_ordinal = 1
                 WHERE session_id = 'session:corrupt-history'",
                [],
            )
            .unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
    });
    let id = DurableCleanupSessionId::new("session:corrupt-history").unwrap();
    assert_eq!(
        engine.cleanup_session_history(&id),
        Err(CleanupHistoryError::CorruptData)
    );
    assert_eq!(
        engine.recent_cleanup_history(None, 64),
        Err(CleanupHistoryError::CorruptData)
    );
}

#[test]
fn cleanup_history_rejects_orphaned_children_before_payload_decode() {
    let (_temp, _config, engine, scan_id, candidate_id, _target) = completed_marker_candidate();
    record_planned_cleanup_history(&engine, &scan_id, &candidate_id, "session:orphan-history");
    engine.inner.store.with_connection(|connection| {
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        connection
            .execute(
                "UPDATE cleanup_item_paths SET item_ordinal = 63
                 WHERE session_id = 'session:orphan-history'",
                [],
            )
            .unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
    });
    let id = DurableCleanupSessionId::new("session:orphan-history").unwrap();
    assert_eq!(
        engine.cleanup_session_history(&id),
        Err(CleanupHistoryError::CorruptData)
    );
    assert_eq!(
        engine.recent_cleanup_history(None, 64),
        Err(CleanupHistoryError::CorruptData)
    );
}

#[test]
fn cleanup_history_recent_feed_runs_complete_lifecycle_validation() {
    let (_temp, _config, engine, scan_id, candidate_id, _target) = completed_marker_candidate();
    record_planned_cleanup_history(
        &engine,
        &scan_id,
        &candidate_id,
        "session:lifecycle-history",
    );
    engine.inner.store.with_connection(|connection| {
        connection
            .execute(
                "UPDATE cleanup_items SET final_status = 'failed'
                 WHERE session_id = 'session:lifecycle-history'",
                [],
            )
            .unwrap();
    });
    let id = DurableCleanupSessionId::new("session:lifecycle-history").unwrap();
    assert_eq!(
        engine.cleanup_session_history(&id),
        Err(CleanupHistoryError::CorruptData)
    );
    assert_eq!(
        engine.recent_cleanup_history(None, 64),
        Err(CleanupHistoryError::CorruptData)
    );
}

#[test]
fn candidate_review_reconciles_exact_post_commit_state_and_rejects_terminal_owners() {
    let (_temp, _config, engine, scan_id, candidate_id, _target) = completed_marker_candidate();
    make_marker_candidate_cleanup_reviewable(&engine, &scan_id);

    assert_eq!(
        engine
            .inner
            .store
            .review_candidate_after_commit_failure_for_test(
                &scan_id,
                &candidate_id,
                CandidateReviewAction::Select,
            )
            .unwrap(),
        CandidateHistoryStatus::Selected
    );
    assert_eq!(
        engine
            .review_candidate(
                &scan_id,
                &candidate_id,
                CandidateReviewCommand::ClearSelection,
            )
            .unwrap()
            .status(),
        DurableCandidateStatus::Discovered
    );

    for status in ["stale", "unavailable", "completed", "failed"] {
        engine.inner.store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE candidates SET status = ?2 WHERE candidate_id = ?1",
                    rusqlite::params![candidate_id.as_str(), status],
                )
                .unwrap();
        });
        assert_eq!(
            engine.review_candidate(&scan_id, &candidate_id, CandidateReviewCommand::Dismiss),
            Err(CandidateReviewError::NotReviewable),
            "status {status} must remain evaluator/journal owned"
        );
    }
}

#[test]
fn concurrent_select_and_dismiss_converge_without_overwriting_dismissal() {
    let (_temp, _config, engine, scan_id, candidate_id, _target) = completed_marker_candidate();
    make_marker_candidate_cleanup_reviewable(&engine, &scan_id);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let select_engine = engine.clone();
    let select_scan = scan_id.clone();
    let select_candidate = candidate_id.clone();
    let select_barrier = Arc::clone(&barrier);
    let select = std::thread::spawn(move || {
        select_barrier.wait();
        select_engine.review_candidate(
            &select_scan,
            &select_candidate,
            CandidateReviewCommand::Select,
        )
    });
    let dismiss_engine = engine.clone();
    let dismiss_scan = scan_id.clone();
    let dismiss_candidate = candidate_id.clone();
    let dismiss_barrier = Arc::clone(&barrier);
    let dismiss = std::thread::spawn(move || {
        dismiss_barrier.wait();
        dismiss_engine.review_candidate(
            &dismiss_scan,
            &dismiss_candidate,
            CandidateReviewCommand::Dismiss,
        )
    });
    barrier.wait();

    let select = select.join().unwrap();
    let dismiss = dismiss.join().unwrap();
    assert!(select.is_ok() || select == Err(CandidateReviewError::NotReviewable));
    assert_eq!(dismiss.unwrap().status(), DurableCandidateStatus::Dismissed);
    assert_eq!(
        engine
            .candidate_history_for_scan(&scan_id)
            .unwrap()
            .candidates()[0]
            .status(),
        DurableCandidateStatus::Dismissed
    );
}

#[test]
fn candidate_detail_and_review_error_mappings_are_exhaustive() {
    let mappings = [
        (
            HistoryErrorKind::IncompatibleSchema,
            CandidateDetailError::IncompatibleSchema,
            CandidateReviewError::IncompatibleSchema,
        ),
        (
            HistoryErrorKind::QueryLimitExceeded,
            CandidateDetailError::QueryLimitExceeded,
            CandidateReviewError::QueryLimitExceeded,
        ),
        (
            HistoryErrorKind::Busy,
            CandidateDetailError::Busy,
            CandidateReviewError::Busy,
        ),
        (
            HistoryErrorKind::UnsafeStorage,
            CandidateDetailError::UnsafeStorage,
            CandidateReviewError::UnsafeStorage,
        ),
        (
            HistoryErrorKind::CorruptData,
            CandidateDetailError::CorruptData,
            CandidateReviewError::CorruptData,
        ),
        (
            HistoryErrorKind::InternalState,
            CandidateDetailError::InternalState,
            CandidateReviewError::InternalState,
        ),
        (
            HistoryErrorKind::NotFound,
            CandidateDetailError::CandidateNotFound,
            CandidateReviewError::CandidateNotFound,
        ),
    ];
    for (kind, detail, review) in mappings {
        assert_eq!(map_candidate_detail_error(kind), detail);
        assert_eq!(map_candidate_review_error(kind), review);
    }
    for kind in [
        HistoryErrorKind::InvalidInput,
        HistoryErrorKind::AlreadyExists,
        HistoryErrorKind::InvalidTransition,
    ] {
        assert_eq!(
            map_candidate_review_error(kind),
            CandidateReviewError::NotReviewable
        );
    }
    assert_eq!(
        map_candidate_review_error(HistoryErrorKind::OutcomeUnknown),
        CandidateReviewError::OutcomeUnknown
    );
    assert_eq!(
        map_candidate_review_error(HistoryErrorKind::DatabaseUnavailable),
        CandidateReviewError::Unavailable
    );
}

#[test]
fn candidate_history_rejects_corrupt_child_rows_without_partial_results() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("scan-root");
    let project = root.join("project");
    let target = project.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(project.join("Cargo.toml"), b"[package]\nname='fixture'\n").unwrap();
    std::fs::write(target.join("object"), vec![7_u8; 8 * 1024]).unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 4, 8, 16))
            .unwrap();

    let task = engine.start_scan(root).unwrap();
    assert_eq!(wait_terminal(&engine, task).phase, TaskPhase::Succeeded);
    let result = engine.scan_result(task).unwrap().unwrap();
    let scan_id = result.scan_id().clone();
    assert_eq!(
        result.candidate_evaluation(),
        CandidateEvaluationTaskStatus::Succeeded { candidate_count: 1 }
    );
    let candidate_id = engine
        .candidate_history_for_scan(&scan_id)
        .unwrap()
        .candidates()[0]
        .id()
        .clone();
    engine.inner.store.with_connection(|connection| {
        assert_eq!(
            connection
                .execute(
                    "UPDATE candidate_paths SET path_ordinal = 1
                     WHERE candidate_id = (
                         SELECT candidate_id FROM candidates WHERE scan_id = ?1
                     )",
                    [scan_id.as_str()],
                )
                .unwrap(),
            1
        );
    });

    assert_eq!(
        engine.candidate_history_for_scan(&scan_id),
        Err(CandidateHistoryError::CorruptData)
    );
    assert_eq!(
        engine.candidate_path_page(&scan_id, &candidate_id, 0, 1),
        Err(CandidateDetailError::CorruptData)
    );
}

#[test]
fn candidate_history_rejects_over_budget_graph_before_payload_decode() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("scan-root");
    let project = root.join("project");
    let target = project.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(project.join("Cargo.toml"), b"[package]\nname='fixture'\n").unwrap();
    std::fs::write(target.join("object"), vec![7_u8; 8 * 1024]).unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 4, 8, 16))
            .unwrap();

    let task = engine.start_scan(root).unwrap();
    assert_eq!(wait_terminal(&engine, task).phase, TaskPhase::Succeeded);
    let result = engine.scan_result(task).unwrap().unwrap();
    let scan_id = result.scan_id().clone();
    assert_eq!(
        result.candidate_evaluation(),
        CandidateEvaluationTaskStatus::Succeeded { candidate_count: 1 }
    );
    let candidate_id = engine
        .candidate_history_for_scan(&scan_id)
        .unwrap()
        .candidates()[0]
        .id()
        .clone();
    engine.inner.store.with_connection(|connection| {
        connection
            .execute(
                "WITH RECURSIVE ordinal(value) AS (
                     VALUES(1)
                     UNION ALL
                     SELECT value + 1 FROM ordinal WHERE value < 255
                 )
                 INSERT INTO candidate_paths (
                     candidate_id, path_ordinal, observed_path, observed_path_encoding
                 )
                 SELECT candidate.candidate_id, ordinal.value,
                        CAST('/' || printf('%.*c', 32760, 'a') ||
                             printf('%07d', ordinal.value) AS BLOB),
                        1
                 FROM candidates AS candidate, ordinal
                 WHERE candidate.scan_id = ?1",
                [scan_id.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "WITH RECURSIVE ordinal(value) AS (
                     VALUES(0)
                     UNION ALL
                     SELECT value + 1 FROM ordinal WHERE value < 99
                 ), selected(candidate_id, first_ordinal) AS (
                     SELECT candidate.candidate_id,
                            count(evidence.evidence_ordinal)
                     FROM candidates AS candidate
                     JOIN candidate_evidence AS evidence USING (candidate_id)
                     WHERE candidate.scan_id = ?1
                 )
                 INSERT INTO candidate_evidence (
                     candidate_id, evidence_ordinal, evidence_kind,
                     path_value, path_value_encoding
                 )
                 SELECT selected.candidate_id,
                        selected.first_ordinal + ordinal.value,
                        'required_marker',
                        CAST('/' || printf('%.*c', 32760, 'e') ||
                             printf('%07d', ordinal.value) AS BLOB),
                        1
                 FROM selected, ordinal",
                [scan_id.as_str()],
            )
            .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*), max(length(path.observed_path))
                     FROM candidates AS candidate
                     JOIN candidate_paths AS path USING (candidate_id)
                     WHERE candidate.scan_id = ?1",
                    [scan_id.as_str()],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap(),
            (256, 32_768)
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*)
                     FROM candidates AS candidate
                     JOIN candidate_evidence AS evidence USING (candidate_id)
                     WHERE candidate.scan_id = ?1 AND length(evidence.path_value) = 32768",
                    [scan_id.as_str()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            100
        );
    });

    assert_eq!(
        engine.candidate_history_for_scan(&scan_id),
        Err(CandidateHistoryError::QueryLimitExceeded)
    );
    assert_eq!(
        engine.candidate_path_page(&scan_id, &candidate_id, 0, 1),
        Err(CandidateDetailError::QueryLimitExceeded)
    );
    assert_eq!(
        engine
            .inner
            .store
            .load_candidate(&candidate_id)
            .unwrap_err()
            .kind,
        HistoryErrorKind::QueryLimitExceeded
    );
}

#[test]
fn candidate_history_preflight_rejects_oversized_payloads() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("scan-root");
    let project = root.join("project");
    let target = project.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(project.join("Cargo.toml"), b"[package]\nname='fixture'\n").unwrap();
    std::fs::write(target.join("object"), vec![7_u8; 8 * 1024]).unwrap();
    let engine =
        EngineHandle::open_with_limits(config(&temp), RegistryLimits::testing(1, 4, 8, 16))
            .unwrap();

    let task = engine.start_scan(root).unwrap();
    assert_eq!(wait_terminal(&engine, task).phase, TaskPhase::Succeeded);
    let scan_id = engine.scan_result(task).unwrap().unwrap().scan_id().clone();
    engine.inner.store.with_connection(|connection| {
        connection
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        connection
            .execute(
                "UPDATE candidate_paths SET observed_path = zeroblob(16777216)
                 WHERE candidate_id = (
                     SELECT candidate_id FROM candidates WHERE scan_id = ?1
                 )",
                [scan_id.as_str()],
            )
            .unwrap();
        connection
            .pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
    });
    assert_eq!(
        engine.candidate_history_for_scan(&scan_id),
        Err(CandidateHistoryError::CorruptData)
    );
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
fn materialization_limit_becomes_typed_discovery_failure_before_publication() {
    let evaluated_at = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    let scan_id = ScanId::new("scan:over-budget-evaluation").unwrap();
    let candidates = over_budget_candidate_records(&scan_id, evaluated_at);
    assert!(
        !CandidateEvaluationCompletion::batch_fits_materialization_budget(&candidates).unwrap()
    );
    let (_temp, engine) = engine_with_limits(RegistryLimits::testing(1, 1, 1, 4));
    assert_eq!(
        engine
            .inner
            .store
            .record_candidate_discovered(&candidates[0])
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidInput
    );
    let identity = CandidateEvaluationIdentity::try_new(1, 1, [1; 32], 1, [2; 32]).unwrap();

    let (_, _completion, status) =
        complete_candidate_evaluation(identity, evaluated_at, candidates).unwrap();

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
    let durable_candidates = engine.candidate_history_for_scan(result.scan_id()).unwrap();
    assert_eq!(
        durable_candidates.status(),
        DurableCandidateEvaluationStatus::Failed {
            kind: CandidateEvaluationTaskFailureKind::Cancelled
        }
    );
    assert!(durable_candidates.candidates().is_empty());
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
    let candidates = engine.candidate_history_for_scan(&scan_id).unwrap();
    assert_eq!(candidates.scan_id(), &scan_id);
    assert_eq!(
        candidates.source_scan_status(),
        DurableScanStatus::Cancelled
    );
    assert_eq!(
        candidates.status(),
        DurableCandidateEvaluationStatus::NotRun
    );
    assert_eq!(candidates.scheduled_at(), None);
    assert_eq!(candidates.completed_at(), None);
    assert!(candidates.candidates().is_empty());
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
        engine.start_scan(child.clone()),
        Err(StartTaskError::ScanScopeBusy)
    );
    assert_eq!(engine.cancel_task(first).unwrap(), CancelOutcome::Requested);
    release_tx.send(()).unwrap();
    assert_eq!(wait_terminal(&engine, first).phase, TaskPhase::Cancelled);
    let replacement = engine.start_scan(root).unwrap();
    assert_eq!(
        wait_terminal(&engine, replacement).phase,
        TaskPhase::Succeeded
    );

    let (child_started_tx, child_started_rx) = mpsc::channel();
    let (child_release_tx, child_release_rx) = mpsc::channel();
    let child_scan = engine
        .start_scan_with_before_traversal_hook(child, move |_| {
            child_started_tx.send(()).unwrap();
            child_release_rx.recv().unwrap();
        })
        .unwrap();
    child_started_rx.recv_timeout(TEST_TIMEOUT).unwrap();
    let parent = temp.path().join("scan-root");
    assert_eq!(
        engine.start_scan(parent),
        Err(StartTaskError::ScanScopeBusy)
    );
    assert_eq!(
        engine.cancel_task(child_scan).unwrap(),
        CancelOutcome::Requested
    );
    child_release_tx.send(()).unwrap();
    assert_eq!(
        wait_terminal(&engine, child_scan).phase,
        TaskPhase::Cancelled
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
