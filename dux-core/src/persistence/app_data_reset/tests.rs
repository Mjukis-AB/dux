use std::sync::{Arc, Barrier, mpsc};

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

use tempfile::TempDir;

use super::*;

const FIRST_TRANSACTION: &str = "00112233445566778899aabbccddeeff";
const SECOND_TRANSACTION: &str = "ffeeddccbbaa99887766554433221100";

fn identities() -> (AppDataResetStoreIdentity, AppDataResetStoreIdentity) {
    (
        AppDataResetStoreIdentity::new(11, 22).unwrap(),
        AppDataResetStoreIdentity::new(33, 44).unwrap(),
    )
}

fn coordinator(temp: &TempDir) -> AppDataResetCoordinator {
    let data_root = temp.path().canonicalize().unwrap().join("Dux");
    AppDataResetCoordinator::open_or_create(&data_root).unwrap()
}

#[test]
fn completed_reset_store_admission_preserves_only_ordinary_database_failures() {
    for kind in [
        DatabaseOpenErrorKind::CorruptDatabase,
        DatabaseOpenErrorKind::MigrationFailed,
        DatabaseOpenErrorKind::DatabaseUnavailable,
    ] {
        assert!(matches!(
            map_completed_store_admission_error(DatabaseOpenError::new(kind)),
            AppDataResetCompletedEngineOpenError::Database(found) if found == kind
        ));
    }

    for kind in [
        DatabaseOpenErrorKind::Busy,
        DatabaseOpenErrorKind::UnsafeStorageRoot,
        DatabaseOpenErrorKind::StorageRootUnavailable,
        DatabaseOpenErrorKind::InternalState,
    ] {
        assert!(matches!(
            map_completed_store_admission_error(DatabaseOpenError::new(kind)),
            AppDataResetCompletedEngineOpenError::Reset(_)
        ));
    }
}

#[test]
fn canonical_journal_round_trips_and_digest_detects_changes() {
    let (data, cache) = identities();
    let journal = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
        data,
        Some(cache),
    )
    .unwrap();
    let encoded = encode_journal(&journal).unwrap();

    assert_eq!(decode_journal(&encoded).unwrap(), journal);
    assert_eq!(journal.transaction_id(), FIRST_TRANSACTION);
    assert_eq!(journal.data_identity().device(), 11);
    assert_eq!(journal.data_identity().inode(), 22);
    assert_eq!(journal.cache_identity(), Some(cache));
    let reconstructed = journal.validated_transaction().unwrap();
    assert_eq!(reconstructed.transaction_id(), FIRST_TRANSACTION);
    assert_eq!(
        reconstructed.data_stage().as_str(),
        ".dux-reset-data-00112233445566778899aabbccddeeff"
    );
    assert_eq!(
        reconstructed.cache_stage().as_str(),
        ".dux-reset-cache-00112233445566778899aabbccddeeff"
    );
    assert_eq!(
        reconstructed.fresh_stage().as_str(),
        ".dux-reset-fresh-00112233445566778899aabbccddeeff"
    );
    assert_eq!(
        journal.data_stage_name(),
        ".dux-reset-data-00112233445566778899aabbccddeeff"
    );
    assert_eq!(
        journal.cache_stage_name(),
        Some(".dux-reset-cache-00112233445566778899aabbccddeeff")
    );
    assert_eq!(
        journal.fresh_stage_name(),
        ".dux-reset-fresh-00112233445566778899aabbccddeeff"
    );
    assert_eq!(journal.fresh_data_identity(), None);

    let mut wrong_role_name = journal.clone();
    wrong_role_name.data_stage_name =
        ".dux-reset-cache-00112233445566778899aabbccddeeff".to_owned();
    let error = match wrong_role_name.validated_transaction() {
        Ok(_) => panic!("role-swapped journal name reconstructed a transaction"),
        Err(error) => error,
    };
    assert_eq!(
        error.kind(),
        AppDataResetCoordinatorErrorKind::CorruptJournal
    );

    let mut changed: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    changed["payload"]["phase"] = serde_json::Value::String("cache_detached".into());
    let changed = serde_json::to_vec(&changed).unwrap();
    assert_eq!(
        decode_journal(&changed).unwrap_err().kind(),
        AppDataResetCoordinatorErrorKind::CorruptJournal
    );
}

#[test]
fn canonical_root_codec_is_lossless_and_rejects_ambiguous_components() {
    let non_utf8 = OsStr::from_bytes(b"Dux-\xff");
    let encoded = encode_canonical_root_name(non_utf8).unwrap();
    assert_eq!(encoded, "4475782dff");
    assert_eq!(
        decode_canonical_root_name(&encoded).unwrap(),
        non_utf8.as_bytes()
    );

    for invalid in ["", "0", "00", "2e", "2e2e", "2f", "4A", "zz"] {
        assert_eq!(decode_canonical_root_name(invalid), None, "{invalid}");
    }

    let (data, _) = identities();
    let transaction = AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap();
    for reserved in [
        storage::COORDINATOR_DIRECTORY_NAME,
        transaction.data_stage().as_str(),
        transaction.cache_stage().as_str(),
        transaction.fresh_stage().as_str(),
    ] {
        assert_eq!(
            AppDataResetJournal::prepared_for_test_root(
                &transaction,
                data,
                None,
                OsStr::new(reserved),
            )
            .unwrap_err()
            .kind(),
            AppDataResetCoordinatorErrorKind::CorruptJournal
        );
    }
}

#[test]
fn legacy_v1_prefix_requires_a_proven_root_upgrade_and_complete_stays_compatible() {
    let (data, cache) = identities();
    let encode_v1 = |phase| {
        let payload = JournalPayloadV1 {
            transaction_id: FIRST_TRANSACTION.to_owned(),
            phase,
            data_identity: data,
            cache_identity: Some(cache),
            data_stage_name: format!(".dux-reset-data-{FIRST_TRANSACTION}"),
            cache_stage_name: Some(format!(".dux-reset-cache-{FIRST_TRANSACTION}")),
        };
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        serde_json::to_vec(&JournalEnvelopeV1 {
            format_version: LEGACY_JOURNAL_FORMAT_VERSION,
            digest_sha256: journal_digest(&payload_bytes),
            payload,
        })
        .unwrap()
    };
    for phase in [
        AppDataResetPhase::Prepared,
        AppDataResetPhase::CacheDetached,
    ] {
        let journal = decode_journal(&encode_v1(phase)).unwrap();
        assert_eq!(journal.phase(), phase);
        assert!(!journal.has_canonical_root_name_binding());
        assert_eq!(journal.fresh_data_identity(), None);
    }
    assert_eq!(
        decode_journal(&encode_v1(AppDataResetPhase::DataDetached))
            .unwrap_err()
            .kind(),
        AppDataResetCoordinatorErrorKind::IncompatibleJournal
    );

    let complete_encoded = encode_v1(AppDataResetPhase::Complete);
    let complete = decode_journal(&complete_encoded).unwrap();
    assert_eq!(complete.phase(), AppDataResetPhase::Complete);
    assert_eq!(complete.fresh_data_identity(), None);
    assert!(complete.legacy_complete_without_fresh_identity);
    assert_eq!(
        encode_journal(&complete).unwrap_err().kind(),
        AppDataResetCoordinatorErrorKind::IncompatibleJournal
    );

    let temp = TempDir::new().unwrap();
    let data_root = temp.path().canonicalize().unwrap().join("data");
    let complete_coordinator = AppDataResetCoordinator::open_or_create(&data_root).unwrap();
    complete_coordinator
        .with_exclusive_session(|session| session.storage.write_journal(&complete_encoded))
        .unwrap();
    drop(complete_coordinator);
    assert!(matches!(
        AppDataResetCoordinator::acquire_engine_lease_until(
            &data_root,
            Instant::now() + Duration::from_secs(1)
        )
        .unwrap(),
        AppDataResetEngineLeaseOutcome::Admitted(_)
    ));

    let upgrade_temp = TempDir::new().unwrap();
    let upgrade_coordinator = coordinator(&upgrade_temp);
    let legacy_prepared = encode_v1(AppDataResetPhase::Prepared);
    upgrade_coordinator
        .with_exclusive_session(|session| session.storage.write_journal(&legacy_prepared))
        .unwrap();
    let upgraded = upgrade_coordinator
        .with_exclusive_session(|session| {
            let legacy = session.recover()?.unwrap();
            session.bind_legacy_canonical_root(
                &legacy,
                AppDataResetCanonicalRootBinding::for_test(OsStr::new("Dux"), data),
            )
        })
        .unwrap();
    assert!(upgraded.has_canonical_root_name_binding());
    assert!(upgraded.is_bound_to_canonical_root_name(OsStr::new("Dux")));
    assert_eq!(upgrade_coordinator.recover().unwrap(), Some(upgraded));

    assert_eq!(
        decode_journal(&encode_v1(AppDataResetPhase::FreshNamespaceReady))
            .unwrap_err()
            .kind(),
        AppDataResetCoordinatorErrorKind::IncompatibleJournal
    );
    assert_eq!(
        decode_journal(&encode_v1(AppDataResetPhase::Draining))
            .unwrap_err()
            .kind(),
        AppDataResetCoordinatorErrorKind::IncompatibleJournal
    );
}

#[test]
fn journal_rejects_noncanonical_unknown_and_newer_shapes() {
    let (data, _) = identities();
    let journal = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
        data,
        None,
    )
    .unwrap();
    let encoded = encode_journal(&journal).unwrap();

    let mut whitespace = encoded.clone();
    whitespace.push(b'\n');
    assert_eq!(
        decode_journal(&whitespace).unwrap_err().kind(),
        AppDataResetCoordinatorErrorKind::CorruptJournal
    );

    let mut unknown: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    unknown["unexpected"] = serde_json::Value::Bool(true);
    assert_eq!(
        decode_journal(&serde_json::to_vec(&unknown).unwrap())
            .unwrap_err()
            .kind(),
        AppDataResetCoordinatorErrorKind::CorruptJournal
    );

    let mut newer: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    newer["format_version"] = serde_json::Value::from(JOURNAL_FORMAT_VERSION + 1);
    assert_eq!(
        decode_journal(&serde_json::to_vec(&newer).unwrap())
            .unwrap_err()
            .kind(),
        AppDataResetCoordinatorErrorKind::IncompatibleJournal
    );
}

#[test]
fn singleton_moves_forward_by_exact_compare_only() {
    let temp = TempDir::new().unwrap();
    let coordinator = coordinator(&temp);
    let (data, cache) = identities();
    let prepared = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
        data,
        Some(cache),
    )
    .unwrap();

    assert_eq!(coordinator.recover().unwrap(), None);
    coordinator.begin(&prepared).unwrap();
    assert_eq!(coordinator.recover().unwrap(), Some(prepared.clone()));

    let cache_detached = coordinator
        .advance(&prepared, AppDataResetPhase::CacheDetached)
        .unwrap();
    assert_eq!(coordinator.recover().unwrap(), Some(cache_detached.clone()));
    assert_eq!(
        coordinator
            .advance(&prepared, AppDataResetPhase::CacheDetached)
            .unwrap_err()
            .kind(),
        AppDataResetCoordinatorErrorKind::ChangedSinceRead
    );
    assert_eq!(
        coordinator
            .advance(&cache_detached, AppDataResetPhase::FreshNamespaceReady)
            .unwrap_err()
            .kind(),
        AppDataResetCoordinatorErrorKind::InvalidTransition
    );

    let data_detached = coordinator
        .advance(&cache_detached, AppDataResetPhase::DataDetached)
        .unwrap();
    let fresh = coordinator
        .with_exclusive_session(|session| {
            session.advance_fresh_namespace_identity(
                &data_detached,
                AppDataResetStoreIdentity::new(55, 66).unwrap(),
            )
        })
        .unwrap();
    assert_eq!(
        fresh.fresh_data_identity(),
        AppDataResetStoreIdentity::new(55, 66)
    );
    assert_eq!(
        coordinator
            .advance(&fresh, AppDataResetPhase::Draining)
            .unwrap_err()
            .kind(),
        AppDataResetCoordinatorErrorKind::InvalidTransition
    );
    let draining = coordinator
        .advance_late_phase_for_test(&fresh, AppDataResetPhase::Draining)
        .unwrap();
    assert_eq!(
        coordinator
            .advance(&draining, AppDataResetPhase::Complete)
            .unwrap_err()
            .kind(),
        AppDataResetCoordinatorErrorKind::InvalidTransition
    );
    let complete = coordinator
        .advance_late_phase_for_test(&draining, AppDataResetPhase::Complete)
        .unwrap();
    assert_eq!(complete.phase(), AppDataResetPhase::Complete);

    let replacement = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(SECOND_TRANSACTION).unwrap(),
        data,
        None,
    )
    .unwrap();
    coordinator.begin(&replacement).unwrap();
    assert_eq!(coordinator.recover().unwrap(), Some(replacement));
}

#[test]
fn incomplete_singleton_cannot_be_replaced() {
    let temp = TempDir::new().unwrap();
    let coordinator = coordinator(&temp);
    let (data, cache) = identities();
    let first = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
        data,
        Some(cache),
    )
    .unwrap();
    let second = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(SECOND_TRANSACTION).unwrap(),
        data,
        None,
    )
    .unwrap();
    coordinator.begin(&first).unwrap();

    assert_eq!(
        coordinator.begin(&second).unwrap_err().kind(),
        AppDataResetCoordinatorErrorKind::InvalidTransition
    );
    assert_eq!(coordinator.recover().unwrap(), Some(first));
}

#[test]
fn recovery_intent_rejects_journal_change_during_shared_to_exclusive_handoff() {
    let temp = TempDir::new().unwrap();
    let coordinator = coordinator(&temp);
    let data_root = temp.path().canonicalize().unwrap().join("Dux");
    let (data, _) = identities();
    let prepared = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
        data,
        None,
    )
    .unwrap();
    coordinator.begin(&prepared).unwrap();
    let changed = self::coordinator(&temp);

    let intent = match AppDataResetCoordinator::acquire_engine_lease_until(
        &data_root,
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap()
    {
        AppDataResetEngineLeaseOutcome::RecoveryRequired(intent) => *intent,
        AppDataResetEngineLeaseOutcome::Admitted(_)
        | AppDataResetEngineLeaseOutcome::CompletedStateValidationRequired(_) => {
            panic!("incomplete journal unexpectedly admitted ordinary engine")
        }
    };
    let expected_for_hook = prepared.clone();
    let error = intent
        .with_exclusive_session_with_handoff_hook_until(
            Instant::now() + Duration::from_secs(1),
            move || {
                changed
                    .advance(&expected_for_hook, AppDataResetPhase::CacheDetached)
                    .unwrap();
            },
            |_, _| -> Result<()> { panic!("changed journal entered recovery callback") },
        )
        .unwrap_err();
    assert_eq!(
        error.kind(),
        AppDataResetCoordinatorErrorKind::ChangedSinceRead
    );
    assert_eq!(
        coordinator.recover().unwrap().unwrap().phase(),
        AppDataResetPhase::CacheDetached
    );
}

#[test]
fn retained_session_sequences_journal_transitions_without_relocking() {
    let temp = TempDir::new().unwrap();
    let coordinator = coordinator(&temp);
    let (data, cache) = identities();
    let prepared = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
        data,
        Some(cache),
    )
    .unwrap();

    coordinator
        .with_exclusive_session(|session| {
            assert_eq!(session.recover()?, None);
            session.begin(&prepared)?;
            assert_eq!(session.recover()?, Some(prepared.clone()));
            let cache_detached = session.advance(&prepared, AppDataResetPhase::CacheDetached)?;
            let data_detached =
                session.advance(&cache_detached, AppDataResetPhase::DataDetached)?;
            assert_eq!(session.recover()?, Some(data_detached));
            Ok(())
        })
        .unwrap();
}

#[test]
fn nested_same_instance_session_is_busy_without_unlocking_outer_session() {
    let temp = TempDir::new().unwrap();
    let coordinator = coordinator(&temp);
    let independent = self::coordinator(&temp);
    let (data, _) = identities();
    let prepared = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
        data,
        None,
    )
    .unwrap();

    coordinator
        .with_exclusive_session(|session| {
            assert_eq!(
                coordinator.recover().unwrap_err().kind(),
                AppDataResetCoordinatorErrorKind::Busy
            );
            assert_eq!(
                independent
                    .with_exclusive_session_with_timeout(Duration::ZERO, |other| {
                        other.recover()
                    })
                    .unwrap_err()
                    .kind(),
                AppDataResetCoordinatorErrorKind::Busy
            );
            session.begin(&prepared)?;
            assert_eq!(session.recover()?, Some(prepared.clone()));
            Ok(())
        })
        .unwrap();
    assert_eq!(coordinator.recover().unwrap(), Some(prepared));
}

#[test]
fn simultaneous_same_instance_sessions_admit_exactly_one_callback() {
    let temp = TempDir::new().unwrap();
    let coordinator = Arc::new(coordinator(&temp));
    let start = Arc::new(Barrier::new(3));
    let release = Arc::new(Barrier::new(2));
    let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
    let (result_sender, result_receiver) = mpsc::sync_channel(2);
    let mut workers = Vec::new();

    for _ in 0..2 {
        let coordinator = Arc::clone(&coordinator);
        let start = Arc::clone(&start);
        let release = Arc::clone(&release);
        let entered_sender = entered_sender.clone();
        let result_sender = result_sender.clone();
        workers.push(std::thread::spawn(move || {
            start.wait();
            let result = coordinator.with_exclusive_session(|_| {
                entered_sender.send(()).unwrap();
                release.wait();
                Ok(())
            });
            result_sender
                .send(result.map_err(|error| error.kind()))
                .unwrap();
        }));
    }
    drop(entered_sender);
    drop(result_sender);

    start.wait();
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let first_result = result_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(first_result, Err(AppDataResetCoordinatorErrorKind::Busy));
    assert!(
        entered_receiver.try_recv().is_err(),
        "more than one coordinator callback entered"
    );
    release.wait();
    assert_eq!(
        result_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap(),
        Ok(())
    );
    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
fn transition_error_does_not_release_retained_session() {
    let temp = TempDir::new().unwrap();
    let first = coordinator(&temp);
    let independent = coordinator(&temp);
    let (data, _) = identities();
    let prepared = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
        data,
        None,
    )
    .unwrap();

    first
        .with_exclusive_session(|session| {
            session.begin(&prepared)?;
            assert_eq!(
                session
                    .advance(&prepared, AppDataResetPhase::DataDetached)
                    .unwrap_err()
                    .kind(),
                AppDataResetCoordinatorErrorKind::InvalidTransition
            );
            assert_eq!(
                independent
                    .with_exclusive_session_with_timeout(Duration::ZERO, |other| {
                        other.recover()
                    })
                    .unwrap_err()
                    .kind(),
                AppDataResetCoordinatorErrorKind::Busy
            );
            let next = session.advance(&prepared, AppDataResetPhase::CacheDetached)?;
            assert_eq!(session.recover()?, Some(next));
            Ok(())
        })
        .unwrap();
}

#[test]
fn store_admission_is_nested_inside_retained_coordinator_session() {
    let temp = TempDir::new().unwrap();
    let coordinator = coordinator(&temp);
    let independent = self::coordinator(&temp);
    let store =
        StoreCoordinator::open(&temp.path().canonicalize().unwrap().join("Dux/dux.sqlite3"))
            .unwrap();

    coordinator
        .with_exclusive_session(|session| {
            session
                .with_admitted_store(&store, |session, guard| {
                    assert!(guard.revalidate().unwrap().is_empty());
                    assert_eq!(
                        independent
                            .with_exclusive_session_with_timeout(Duration::ZERO, |other| {
                                other.recover()
                            })
                            .unwrap_err()
                            .kind(),
                        AppDataResetCoordinatorErrorKind::Busy
                    );
                    assert_eq!(session.recover().unwrap(), None);
                })
                .map(|outcome| match outcome {
                    AppDataResetAdmittedStoreOutcome::Admitted(()) => {}
                    AppDataResetAdmittedStoreOutcome::Blocked(_) => {
                        panic!("empty current store unexpectedly blocked reset admission");
                    }
                })
                .unwrap();
            assert_eq!(session.recover()?, None);
            Ok(())
        })
        .unwrap();
    assert_eq!(independent.recover().unwrap(), None);
}

#[test]
fn coordinator_and_retained_session_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<AppDataResetCoordinator>();
    assert_send::<AppDataResetCoordinatorSession<'static>>();
}

#[test]
fn independent_coordinator_stays_excluded_until_retained_session_releases() {
    let temp = TempDir::new().unwrap();
    let first = coordinator(&temp);
    let second = coordinator(&temp);
    let (data, _) = identities();
    let prepared = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
        data,
        None,
    )
    .unwrap();

    first
        .with_exclusive_session(|session| {
            session.begin(&prepared)?;
            assert_eq!(
                second
                    .with_exclusive_session_with_timeout(Duration::ZERO, |other| {
                        other.recover()
                    })
                    .unwrap_err()
                    .kind(),
                AppDataResetCoordinatorErrorKind::Busy
            );
            assert_eq!(session.recover()?, Some(prepared.clone()));
            Ok(())
        })
        .unwrap();
    assert_eq!(second.recover().unwrap(), Some(prepared));
}

#[test]
fn callback_error_and_panic_release_session_without_rolling_back_journal() {
    let temp = TempDir::new().unwrap();
    let coordinator = coordinator(&temp);
    let (data, _) = identities();
    let prepared = AppDataResetJournal::prepared_for_test(
        &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
        data,
        None,
    )
    .unwrap();

    let error = coordinator
        .with_exclusive_session(|session| {
            session.begin(&prepared)?;
            Err::<(), _>(error(AppDataResetCoordinatorErrorKind::Unavailable))
        })
        .unwrap_err();
    assert_eq!(error.kind(), AppDataResetCoordinatorErrorKind::Unavailable);
    assert_eq!(coordinator.recover().unwrap(), Some(prepared.clone()));

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<()> = coordinator.with_exclusive_session(|session| {
            assert_eq!(session.recover()?, Some(prepared.clone()));
            panic!("simulate reset coordinator callback panic");
        });
    }));
    assert!(panic.is_err());
    assert_eq!(coordinator.recover().unwrap(), Some(prepared));
}

#[test]
fn fixed_sibling_is_private_and_does_not_create_data_root() {
    let temp = TempDir::new().unwrap();
    let canonical = temp.path().canonicalize().unwrap();
    let data_root = canonical.join("Dux");
    let coordinator = AppDataResetCoordinator::open_or_create(&data_root).unwrap();

    assert!(!data_root.exists());
    let root = canonical.join(storage::COORDINATOR_DIRECTORY_NAME);
    let metadata = fs::metadata(&root).unwrap();
    assert_eq!(metadata.mode() & 0o7777, 0o700);
    assert_eq!(metadata.uid(), unsafe { nix::libc::geteuid() });
    assert_eq!(coordinator.recover().unwrap(), None);
    assert_eq!(
        coordinator
            .provisioning_debt()
            .unwrap()
            .unproven_stage_count(),
        0
    );
}

#[test]
fn symlink_and_permissive_coordinator_are_rejected() {
    let symlink_case = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    symlink(
        outside.path(),
        symlink_case
            .path()
            .join(storage::COORDINATOR_DIRECTORY_NAME),
    )
    .unwrap();
    let symlink_root = symlink_case.path().canonicalize().unwrap().join("Dux");
    let error = match AppDataResetCoordinator::open_or_create(&symlink_root) {
        Ok(_) => panic!("symlink coordinator unexpectedly opened"),
        Err(error) => error,
    };
    assert_eq!(
        error.kind(),
        AppDataResetCoordinatorErrorKind::UnsafeCoordinator
    );

    let mode_case = TempDir::new().unwrap();
    let coordinator = coordinator(&mode_case);
    drop(coordinator);
    let canonical = mode_case.path().canonicalize().unwrap();
    let root = canonical.join(storage::COORDINATOR_DIRECTORY_NAME);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    let error = match AppDataResetCoordinator::open_or_create(&canonical.join("Dux")) {
        Ok(_) => panic!("permissive coordinator unexpectedly opened"),
        Err(error) => error,
    };
    assert_eq!(
        error.kind(),
        AppDataResetCoordinatorErrorKind::UnsafeCoordinator
    );
}
