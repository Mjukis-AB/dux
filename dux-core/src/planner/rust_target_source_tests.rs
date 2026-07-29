#![cfg(unix)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use super::rust_target::{RustTargetLiveValidationError, validate_live_rust_target};
use super::rust_target_source::{
    RustTargetSourceError, acquire_rust_target_durable_source,
    acquire_rust_target_durable_source_after_snapshot_hook_for_test,
    acquire_rust_target_durable_source_with_clock_for_test,
};
use super::rust_target_tests::{CARGO_CACHE_TAG_SIGNATURE, candidate, exact_evidence, rust_rule};
use crate::domain::{
    CANDIDATE_CATALOG_SCHEMA_VERSION, CANDIDATE_CATALOG_SHA256, CANDIDATE_CONTEXT_FORMAT_VERSION,
    CANDIDATE_EVALUATOR_REVISION, CandidateId, ScanId, evaluate_completed_scan_candidates,
};
use crate::persistence::snapshot::from_scan::prepare_completed_scan;
use crate::persistence::snapshot::{
    SnapshotRepository, SnapshotRepositoryErrorKind, SnapshotStoreAccess,
};
use crate::persistence::{
    CandidateEvaluationCompletion, CandidateEvaluationIdentity, CandidateReviewAction,
    NewCandidateRecord, NewScanRecord, SnapshotReviewPurpose, StoreCoordinator,
};
use crate::scanner::{ScanConfig, Scanner};

struct PersistedFixture {
    _temp: TempDir,
    root: std::path::PathBuf,
    target: std::path::PathBuf,
    manifest: std::path::PathBuf,
    cache_tag: std::path::PathBuf,
    database: std::path::PathBuf,
    store: Arc<StoreCoordinator>,
    snapshots: SnapshotRepository,
    scan_id: ScanId,
    candidate_id: CandidateId,
    observed_at: SystemTime,
}

impl PersistedFixture {
    fn new() -> Self {
        Self::with_evaluator_revision_and_forged_id(CANDIDATE_EVALUATOR_REVISION, false)
    }

    fn with_additional_candidate() -> Self {
        Self::build(CANDIDATE_EVALUATOR_REVISION, false, true, false)
    }

    fn with_every_catalog_pattern() -> Self {
        Self::build(CANDIDATE_EVALUATOR_REVISION, false, false, true)
    }

    fn with_evaluator_revision_and_forged_id(
        evaluator_revision: u32,
        forge_candidate_id: bool,
    ) -> Self {
        Self::build(evaluator_revision, forge_candidate_id, false, false)
    }

    fn build(
        evaluator_revision: u32,
        forge_candidate_id: bool,
        add_node_candidate: bool,
        add_every_catalog_pattern: bool,
    ) -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("scan-root");
        let target = root.join("project/target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(
            root.join("project/Cargo.toml"),
            b"[package]\nname = \"durable-fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(target.join("CACHEDIR.TAG"), CARGO_CACHE_TAG_SIGNATURE).unwrap();
        if add_node_candidate {
            std::fs::create_dir_all(root.join("other/node_modules")).unwrap();
            std::fs::write(root.join("other/package.json"), b"{}\n").unwrap();
        }
        if add_every_catalog_pattern {
            add_catalog_pattern_fixtures(&root);
        }
        super::rust_target_tests::set_subtree_modified_at(
            &target,
            SystemTime::now() - Duration::from_secs(8 * 86_400),
        );
        let root = root.canonicalize().unwrap();
        let target = root.join("project/target");
        let manifest = root.join("project/Cargo.toml");
        let cache_tag = target.join("CACHEDIR.TAG");
        let database = temp.path().join("store/dux.sqlite3");
        let store = StoreCoordinator::open(&database).unwrap();
        let snapshots =
            SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadWrite).unwrap();
        let scan_id = ScanId::new("scan:rust-durable-source").unwrap();
        let observed_at = {
            let milliseconds = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis();
            SystemTime::UNIX_EPOCH + Duration::from_millis(u64::try_from(milliseconds).unwrap())
        };
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    scan_id.clone(),
                    root.clone(),
                    observed_at - Duration::from_secs(2),
                )
                .unwrap(),
            )
            .unwrap();

        let (progress, worker) = Scanner::new(ScanConfig {
            num_threads: 1,
            ..ScanConfig::default()
        })
        .scan(root.clone());
        for _ in progress {}
        let artifact = worker
            .join()
            .unwrap()
            .into_completed_artifact()
            .expect("fixture scan completes");
        let batch = evaluate_completed_scan_candidates(
            &scan_id,
            &artifact,
            observed_at,
            crate::domain::CandidateEvaluationScope::SelectedScanRoot,
        )
        .unwrap();
        let discovered_candidate_id = batch
            .candidates()
            .iter()
            .find(|candidate| candidate.paths() == [target.as_path()])
            .expect("Rust target candidate is discovered")
            .id()
            .clone();
        let identity = CandidateEvaluationIdentity::try_new(
            evaluator_revision,
            CANDIDATE_CATALOG_SCHEMA_VERSION,
            CANDIDATE_CATALOG_SHA256,
            CANDIDATE_CONTEXT_FORMAT_VERSION,
            batch.context_digest_sha256(),
        )
        .unwrap();
        let evaluated_at = observed_at + Duration::from_millis(1);
        let mut evaluated_candidates = batch.into_candidates();
        if forge_candidate_id {
            let rust_candidate = evaluated_candidates
                .iter_mut()
                .find(|candidate| candidate.id() == &discovered_candidate_id)
                .expect("Rust candidate remains in the batch");
            *rust_candidate = candidate(
                &scan_id,
                &target,
                rust_rule(false),
                exact_evidence(&target),
                vec![crate::domain::BlockReason::ProtectedPath],
            );
        }
        let candidate_id = evaluated_candidates
            .iter()
            .find(|candidate| candidate.paths() == [target.as_path()])
            .unwrap()
            .id()
            .clone();
        let candidates = evaluated_candidates
            .iter()
            .map(|candidate| NewCandidateRecord::try_from_candidate(candidate, evaluated_at))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let evaluation =
            CandidateEvaluationCompletion::succeeded(evaluated_at, candidates).unwrap();
        let prepared = prepare_completed_scan(scan_id.clone(), observed_at, &artifact).unwrap();
        let (document, counts, coverage) = prepared.into_parts();
        snapshots
            .complete_scan_with_candidate_evaluation(
                observed_at,
                counts,
                &coverage,
                &document,
                &identity,
                &evaluation,
            )
            .unwrap();

        Self {
            _temp: temp,
            root,
            target,
            manifest,
            cache_tag,
            database,
            store,
            snapshots,
            scan_id,
            candidate_id,
            observed_at,
        }
    }

    fn acquire(&self) -> super::rust_target_source::RustTargetDurableSource {
        acquire_rust_target_durable_source(
            Arc::clone(&self.store),
            &self.snapshots,
            &self.scan_id,
            &self.candidate_id,
        )
        .unwrap()
    }
}

fn add_catalog_pattern_fixtures(root: &std::path::Path) {
    for directory in [
        "node/node_modules",
        "node/node_modules/nested/target",
        "gradle/build",
        "gradle/.gradle",
        "python/__pycache__",
        "tox/.tox",
        "venv/venv",
        "venv-hidden/.venv",
        "pods/Pods",
        "next/.next",
        "nuxt/.nuxt",
    ] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    for (path, bytes) in [
        ("node/package.json", b"{}\n".as_slice()),
        (
            "node/node_modules/nested/Cargo.toml",
            b"[package]\nname = \"suppressed\"\nversion = \"0.1.0\"\n".as_slice(),
        ),
        (
            "node/node_modules/nested/target/CACHEDIR.TAG",
            CARGO_CACHE_TAG_SIGNATURE,
        ),
        ("gradle/build.gradle", b"plugins {}\n".as_slice()),
        ("python/module.py", b"pass\n".as_slice()),
        ("tox/tox.ini", b"[tox]\n".as_slice()),
        ("venv/venv/pyvenv.cfg", b"home = /tmp\n".as_slice()),
        ("venv-hidden/.venv/pyvenv.cfg", b"home = /tmp\n".as_slice()),
        ("pods/Podfile", b"platform :osx\n".as_slice()),
        ("pods/Pods/Manifest.lock", b"PODS:\n".as_slice()),
        ("next/package.json", b"{}\n".as_slice()),
        ("next/next.config.js", b"module.exports = {}\n".as_slice()),
        ("nuxt/package.json", b"{}\n".as_slice()),
        ("nuxt/nuxt.config.ts", b"export default {}\n".as_slice()),
    ] {
        std::fs::write(root.join(path), bytes).unwrap();
    }
}

#[test]
fn durable_source_can_be_reacquired_after_repository_reopen() {
    let fixture = PersistedFixture::new();
    let PersistedFixture {
        _temp,
        root,
        target,
        manifest: _,
        cache_tag: _,
        database,
        store,
        snapshots,
        scan_id,
        candidate_id,
        observed_at: _,
    } = fixture;
    drop(snapshots);
    drop(store);

    let reopened_store = StoreCoordinator::open(&database).unwrap();
    let reopened_snapshots =
        SnapshotRepository::open(Arc::clone(&reopened_store), SnapshotStoreAccess::ReadWrite)
            .unwrap();
    let source = acquire_rust_target_durable_source(
        reopened_store,
        &reopened_snapshots,
        &scan_id,
        &candidate_id,
    )
    .unwrap();
    let candidate = source.candidate_for_promotion().unwrap();
    assert_eq!(candidate.id(), &candidate_id);
    let witness = validate_live_rust_target(source).unwrap();
    assert_eq!(witness.scan_root().canonical_path(), root);
    assert_eq!(witness.target().canonical_path(), target);
}

#[test]
fn snapshot_replay_matches_fresh_evaluation_for_every_catalog_pattern() {
    PersistedFixture::with_every_catalog_pattern()
        .acquire()
        .release()
        .unwrap();
}

#[test]
fn stale_evaluator_identity_and_forged_candidate_id_fail_closed() {
    let stale = PersistedFixture::with_evaluator_revision_and_forged_id(
        CANDIDATE_EVALUATOR_REVISION + 1,
        false,
    );
    assert!(matches!(
        acquire_rust_target_durable_source(
            Arc::clone(&stale.store),
            &stale.snapshots,
            &stale.scan_id,
            &stale.candidate_id,
        ),
        Err(RustTargetSourceError::History {
            kind: crate::persistence::HistoryErrorKind::InvalidTransition
        })
    ));

    let forged =
        PersistedFixture::with_evaluator_revision_and_forged_id(CANDIDATE_EVALUATOR_REVISION, true);
    assert!(matches!(
        acquire_rust_target_durable_source(
            Arc::clone(&forged.store),
            &forged.snapshots,
            &forged.scan_id,
            &forged.candidate_id,
        ),
        Err(RustTargetSourceError::CandidateMismatch)
    ));
}

#[test]
fn snapshot_replay_rejects_forged_size_and_newest_time() {
    let forged_size = PersistedFixture::new();
    rusqlite::Connection::open(&forged_size.database)
        .unwrap()
        .execute(
            "UPDATE candidates
             SET estimated_bytes = estimated_bytes + 1
             WHERE candidate_id = ?1",
            [forged_size.candidate_id.as_str()],
        )
        .unwrap();
    assert!(matches!(
        acquire_rust_target_durable_source(
            Arc::clone(&forged_size.store),
            &forged_size.snapshots,
            &forged_size.scan_id,
            &forged_size.candidate_id,
        ),
        Err(RustTargetSourceError::EvaluatorReplayMismatch)
    ));

    let forged_time = PersistedFixture::new();
    rusqlite::Connection::open(&forged_time.database)
        .unwrap()
        .execute(
            "UPDATE candidates
             SET newest_mtime_unix_seconds = NULL,
                 newest_mtime_nanoseconds = NULL
             WHERE candidate_id = ?1",
            [forged_time.candidate_id.as_str()],
        )
        .unwrap();
    assert!(matches!(
        acquire_rust_target_durable_source(
            Arc::clone(&forged_time.store),
            &forged_time.snapshots,
            &forged_time.scan_id,
            &forged_time.candidate_id,
        ),
        Err(RustTargetSourceError::CandidateMismatch)
    ));
}

#[test]
fn replay_failures_release_cleanup_review_pins() {
    let fixture = PersistedFixture::new();
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection
        .execute(
            "UPDATE candidates
             SET estimated_bytes = estimated_bytes + 1
             WHERE candidate_id = ?1",
            [fixture.candidate_id.as_str()],
        )
        .unwrap();
    for _ in 0..65 {
        assert!(matches!(
            acquire_rust_target_durable_source(
                Arc::clone(&fixture.store),
                &fixture.snapshots,
                &fixture.scan_id,
                &fixture.candidate_id,
            ),
            Err(RustTargetSourceError::EvaluatorReplayMismatch)
        ));
    }
    connection
        .execute(
            "UPDATE candidates
             SET estimated_bytes = estimated_bytes - 1
             WHERE candidate_id = ?1",
            [fixture.candidate_id.as_str()],
        )
        .unwrap();
    fixture.acquire().release().unwrap();
}

#[test]
fn snapshot_replay_rejects_reordered_evidence() {
    let fixture = PersistedFixture::new();
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection
        .execute(
            "UPDATE candidate_evidence
             SET evidence_ordinal = 99
             WHERE candidate_id = ?1 AND evidence_ordinal = 1",
            [fixture.candidate_id.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE candidate_evidence
             SET evidence_ordinal = 1
             WHERE candidate_id = ?1 AND evidence_ordinal = 2",
            [fixture.candidate_id.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE candidate_evidence
             SET evidence_ordinal = 2
             WHERE candidate_id = ?1 AND evidence_ordinal = 99",
            [fixture.candidate_id.as_str()],
        )
        .unwrap();

    assert!(matches!(
        acquire_rust_target_durable_source(
            Arc::clone(&fixture.store),
            &fixture.snapshots,
            &fixture.scan_id,
            &fixture.candidate_id,
        ),
        Err(RustTargetSourceError::EvaluatorReplayMismatch)
    ));
}

#[test]
fn snapshot_replay_rejects_an_omitted_sibling_candidate() {
    let fixture = PersistedFixture::with_additional_candidate();
    let connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    assert_eq!(
        connection
            .execute(
                "DELETE FROM candidates
                 WHERE scan_id = ?1 AND candidate_id <> ?2",
                rusqlite::params![fixture.scan_id.as_str(), fixture.candidate_id.as_str()],
            )
            .unwrap(),
        1
    );
    connection
        .execute(
            "UPDATE candidate_evaluations
             SET candidate_count = 1
             WHERE scan_id = ?1",
            [fixture.scan_id.as_str()],
        )
        .unwrap();

    assert!(matches!(
        acquire_rust_target_durable_source(
            Arc::clone(&fixture.store),
            &fixture.snapshots,
            &fixture.scan_id,
            &fixture.candidate_id,
        ),
        Err(RustTargetSourceError::EvaluatorReplayMismatch)
    ));
}

#[test]
fn snapshot_replay_rejects_an_injected_candidate() {
    let fixture = PersistedFixture::new();
    let mut connection = rusqlite::Connection::open(&fixture.database).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    let transaction = connection.transaction().unwrap();
    let injected = "candidate:injected-snapshot-replay-row";
    transaction
        .execute(
            "INSERT INTO candidates (
                 candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                 estimated_bytes, created_at_unix_ms, status, record_format_version,
                 category, proposed_action, rule_schedule_eligible,
                 newest_mtime_unix_seconds, newest_mtime_nanoseconds
             )
             SELECT ?1, scan_id, rule_id, rule_revision, safety_tier,
                    estimated_bytes, created_at_unix_ms, status, record_format_version,
                    category, proposed_action, rule_schedule_eligible,
                    newest_mtime_unix_seconds, newest_mtime_nanoseconds
             FROM candidates WHERE candidate_id = ?2",
            rusqlite::params![injected, fixture.candidate_id.as_str()],
        )
        .unwrap();
    for (table, columns) in [
        (
            "candidate_paths",
            "path_ordinal, observed_path, observed_path_encoding",
        ),
        (
            "candidate_evidence",
            "evidence_ordinal, evidence_kind, path_value, path_value_encoding,
             text_value, observed_unix_seconds, observed_nanoseconds,
             duration_seconds, duration_nanoseconds, observed_bytes, minimum_bytes",
        ),
        ("candidate_blockers", "blocker_ordinal, blocker_kind"),
    ] {
        transaction
            .execute(
                &format!(
                    "INSERT INTO {table} (candidate_id, {columns})
                     SELECT ?1, {columns} FROM {table} WHERE candidate_id = ?2"
                ),
                rusqlite::params![injected, fixture.candidate_id.as_str()],
            )
            .unwrap();
    }
    transaction
        .execute(
            "UPDATE candidate_evaluations SET candidate_count = 2 WHERE scan_id = ?1",
            [fixture.scan_id.as_str()],
        )
        .unwrap();
    transaction.commit().unwrap();

    assert!(matches!(
        acquire_rust_target_durable_source(
            Arc::clone(&fixture.store),
            &fixture.snapshots,
            &fixture.scan_id,
            &fixture.candidate_id,
        ),
        Err(RustTargetSourceError::EvaluatorReplayMismatch)
    ));
}

#[test]
fn acquisition_revalidates_the_lease_with_fresh_time_before_returning() {
    let fixture = PersistedFixture::new();
    let initial = SystemTime::now();
    let expired = initial + Duration::from_secs(24 * 60 * 60);
    let mut times = [initial, initial, initial, expired].into_iter();
    let result = acquire_rust_target_durable_source_with_clock_for_test(
        Arc::clone(&fixture.store),
        &fixture.snapshots,
        &fixture.scan_id,
        &fixture.candidate_id,
        || times.next().expect("acquisition uses bounded clock reads"),
    );
    assert!(matches!(
        result,
        Err(RustTargetSourceError::Snapshot {
            kind: SnapshotRepositoryErrorKind::ReviewLeaseExpired
        })
    ));
}

#[test]
fn source_decode_uses_review_budget_and_failed_acquisitions_release_their_pins() {
    let fixture = PersistedFixture::new();
    let reference = fixture
        .store
        .load_scan(&fixture.scan_id)
        .unwrap()
        .unwrap()
        .snapshot()
        .unwrap()
        .clone();
    let first_lease = fixture
        .snapshots
        .acquire_review_lease(
            &reference,
            SnapshotReviewPurpose::Explorer,
            SystemTime::now(),
        )
        .unwrap();
    let first_document = first_lease.load_for_review(SystemTime::now()).unwrap();
    let second_lease = fixture
        .snapshots
        .acquire_review_lease(
            &reference,
            SnapshotReviewPurpose::Explorer,
            SystemTime::now(),
        )
        .unwrap();
    let second_document = second_lease.load_for_review(SystemTime::now()).unwrap();

    for _ in 0..65 {
        assert!(matches!(
            acquire_rust_target_durable_source(
                Arc::clone(&fixture.store),
                &fixture.snapshots,
                &fixture.scan_id,
                &fixture.candidate_id,
            ),
            Err(RustTargetSourceError::Snapshot {
                kind: SnapshotRepositoryErrorKind::History(
                    crate::persistence::HistoryErrorKind::QueryLimitExceeded
                )
            })
        ));
    }

    drop(second_document);
    drop(first_document);
    second_lease.release().unwrap();
    first_lease.release().unwrap();
    fixture.acquire().release().unwrap();
}

#[test]
fn successful_witness_drop_and_explicit_release_do_not_exhaust_review_pins() {
    let fixture = PersistedFixture::new();
    for _ in 0..65 {
        drop(validate_live_rust_target(fixture.acquire()).unwrap());
    }
    for _ in 0..65 {
        validate_live_rust_target(fixture.acquire())
            .unwrap()
            .release()
            .unwrap();
    }
    fixture.acquire().release().unwrap();
}

#[test]
fn exact_persisted_scan_candidate_and_snapshot_mint_only_a_blocked_live_witness() {
    let fixture = PersistedFixture::new();
    let source = fixture.acquire();
    assert_eq!(source.purpose(), SnapshotReviewPurpose::CleanupReview);

    let witness = validate_live_rust_target(source).unwrap();
    assert_eq!(witness.source_scan_id(), &fixture.scan_id);
    assert_eq!(witness.candidate_id(), &fixture.candidate_id);
    assert!(witness.protected_path_is_still_unresolved());
    assert_eq!(witness.target().canonical_path(), fixture.target);
    assert_eq!(witness.manifest().canonical_path(), fixture.manifest);
    assert_eq!(
        witness.cache_tag().path().canonical_path(),
        fixture.cache_tag
    );
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test replaces only a TempDir-owned target to prove scan-time identity rejection"
)]
fn recreated_target_tree_is_rejected_even_when_the_paths_and_markers_match() {
    let fixture = PersistedFixture::new();
    let displaced = fixture.root.join("project/target-before-replacement");
    // DUX-DESTRUCTIVE: allow=test-durable-rust-target-replace-rename -- rename only this TempDir-owned target to prove a recreated path cannot satisfy scan-time identity binding
    std::fs::rename(&fixture.target, displaced).unwrap();
    std::fs::create_dir(&fixture.target).unwrap();
    std::fs::write(&fixture.cache_tag, CARGO_CACHE_TAG_SIGNATURE).unwrap();

    assert!(matches!(
        validate_live_rust_target(fixture.acquire()),
        Err(RustTargetLiveValidationError::ChangedSinceScan)
    ));
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test moves only TempDir-owned objects to prove scan-time ancestor rejection"
)]
fn replaced_parent_is_rejected_even_when_original_terminal_objects_are_moved_back() {
    let fixture = PersistedFixture::new();
    let original_project = fixture.root.join("project-before-replacement");
    // DUX-DESTRUCTIVE: allow=test-durable-rust-project-replace-rename -- rename only the TempDir-owned project ancestor before recreating it
    std::fs::rename(fixture.root.join("project"), &original_project).unwrap();
    std::fs::create_dir(fixture.root.join("project")).unwrap();
    // DUX-DESTRUCTIVE: allow=test-durable-rust-target-move-back -- move only the original TempDir-owned target into the recreated project ancestor
    std::fs::rename(original_project.join("target"), &fixture.target).unwrap();
    // DUX-DESTRUCTIVE: allow=test-durable-rust-manifest-move-back -- move only the original TempDir-owned manifest into the recreated project ancestor
    std::fs::rename(original_project.join("Cargo.toml"), &fixture.manifest).unwrap();

    assert!(matches!(
        validate_live_rust_target(fixture.acquire()),
        Err(RustTargetLiveValidationError::ChangedSinceScan)
    ));
}

#[test]
fn acquisition_sandwich_rejects_candidate_status_changed_after_snapshot_load() {
    let fixture = PersistedFixture::new();
    let store_for_hook = Arc::clone(&fixture.store);
    let scan_id_for_hook = fixture.scan_id.clone();
    let candidate_id_for_hook = fixture.candidate_id.clone();

    let error = match acquire_rust_target_durable_source_after_snapshot_hook_for_test(
        Arc::clone(&fixture.store),
        &fixture.snapshots,
        &fixture.scan_id,
        &fixture.candidate_id,
        fixture.observed_at,
        move || {
            store_for_hook
                .review_candidate(
                    &scan_id_for_hook,
                    &candidate_id_for_hook,
                    CandidateReviewAction::Dismiss,
                )
                .unwrap();
        },
    ) {
        Ok(_) => panic!("status change must invalidate durable acquisition"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        RustTargetSourceError::History {
            kind: crate::persistence::HistoryErrorKind::InvalidTransition
        }
    ));
}

#[test]
fn a_live_witness_fails_closed_after_its_durable_candidate_is_dismissed() {
    let fixture = PersistedFixture::new();
    let witness = validate_live_rust_target(fixture.acquire()).unwrap();
    fixture
        .store
        .review_candidate(
            &fixture.scan_id,
            &fixture.candidate_id,
            CandidateReviewAction::Dismiss,
        )
        .unwrap();

    assert!(matches!(
        witness.revalidate_current(),
        Err(RustTargetLiveValidationError::DurableSourceChanged)
    ));
}

#[test]
fn wrong_candidate_and_unrelated_repository_cannot_mint_a_source() {
    let fixture = PersistedFixture::new();
    let wrong_candidate = CandidateId::new("candidate:missing-rust-source").unwrap();
    assert!(matches!(
        acquire_rust_target_durable_source(
            Arc::clone(&fixture.store),
            &fixture.snapshots,
            &fixture.scan_id,
            &wrong_candidate,
        ),
        Err(RustTargetSourceError::History {
            kind: crate::persistence::HistoryErrorKind::NotFound
        })
    ));

    let other_temp = TempDir::new().unwrap();
    let other_store = StoreCoordinator::open(&other_temp.path().join("store/dux.sqlite3")).unwrap();
    assert!(matches!(
        acquire_rust_target_durable_source(
            other_store,
            &fixture.snapshots,
            &fixture.scan_id,
            &fixture.candidate_id,
        ),
        Err(RustTargetSourceError::StoreMismatch)
    ));
}

#[test]
fn source_error_categories_do_not_disclose_candidate_paths() {
    let error = RustTargetSourceError::Snapshot {
        kind: SnapshotRepositoryErrorKind::MissingSnapshot,
    };
    let rendered = error.to_string();
    assert!(!rendered.contains("scan-root"));
    assert!(!rendered.contains("Cargo.toml"));
}
