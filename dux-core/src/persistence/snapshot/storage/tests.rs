use std::fs;
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
// DUX-DESTRUCTIVE: allow=test-snapshot-lock-command-import -- import only the fixed Rust test-harness relaunch primitive used by the bounded cross-process writer-lock regression
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::*;

fn database_path(temp: &TempDir) -> PathBuf {
    temp.path().join("Dux").join("dux.sqlite3")
}

fn private_database_root(temp: &TempDir) -> PathBuf {
    let root = temp.path().join("Dux");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn open_rw(database: &Path) -> SecureSnapshotStore {
    SecureSnapshotStore::open_for_database(database, SnapshotStoreAccess::ReadWrite)
        .unwrap()
        .unwrap()
}

fn publish_test_snapshot(
    store: &SecureSnapshotStore,
    scan_id: &[u8],
    bytes: &[u8],
) -> SnapshotFileName {
    let name = SnapshotFileName::from_scan_id(scan_id);
    let mut staged = store
        .stage(name.clone(), Duration::from_millis(100))
        .unwrap();
    staged.write_all(bytes).unwrap();
    drop(staged.publish_no_replace().unwrap());
    name
}

fn abandon_test_snapshot_temp(store: &SecureSnapshotStore, scan_id: &[u8]) -> String {
    let mut staged = store
        .stage(
            SnapshotFileName::from_scan_id(scan_id),
            Duration::from_millis(100),
        )
        .unwrap();
    staged.write_all(b"quiescent reset temp").unwrap();
    staged.sync_all().unwrap();
    let name = staged.temp_name.clone();
    staged.abandon();
    name
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn reset_payload_test_deadline() -> Instant {
    Instant::now().checked_add(Duration::from_secs(1)).unwrap()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn reset_snapshot_recovery_at_state(
    database: &Path,
    expected: AppDataResetSnapshotStoreRetirementState,
) -> (AppDataResetSnapshotRecovery, Instant) {
    loop {
        let deadline = reset_payload_test_deadline();
        let mut recovery = AppDataResetSnapshotRecovery::open_until(
            database.parent().unwrap(),
            File::open(database.parent().unwrap()).unwrap(),
            true,
            deadline,
        )
        .unwrap();
        let state = recovery.retirement_state().unwrap();
        if state == expected {
            return (recovery, deadline);
        }
        assert_ne!(state, AppDataResetSnapshotStoreRetirementState::Absent);
        recovery
            .retire_one_structure(AppDataResetSnapshotStoreRetireAuthority::for_test(deadline))
            .unwrap();
    }
}

fn provisioning_stage(root: &Path, suffix: &str, marker: bool, writer: bool) -> PathBuf {
    assert_eq!(suffix.len(), PROVISIONING_STAGE_HEX_LENGTH);
    let stage = root.join(format!("{PROVISIONING_STAGE_PREFIX}{suffix}"));
    fs::create_dir(&stage).unwrap();
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).unwrap();
    if marker {
        fs::write(stage.join(MARKER_NAME), STORE_MARKER).unwrap();
        fs::set_permissions(stage.join(MARKER_NAME), fs::Permissions::from_mode(0o600)).unwrap();
    }
    if writer {
        fs::write(stage.join(WRITER_LOCK_NAME), WRITER_MARKER).unwrap();
        fs::set_permissions(
            stage.join(WRITER_LOCK_NAME),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    stage
}

#[test]
fn typed_name_is_exact_lowercase_single_component() {
    let name = SnapshotFileName::from_scan_id(b"scan-1");
    assert_eq!(name.as_str().len(), 85);
    assert_eq!(SnapshotFileName::parse(name.as_str()).unwrap(), name);
    assert!(SnapshotFileName::parse("../snapshot-deadbeef.duxsnapshot").is_err());
    assert!(
        SnapshotFileName::parse(
            "snapshot-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.duxsnapshot"
        )
        .is_err()
    );
}

#[test]
fn read_only_missing_store_does_not_provision() {
    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let database = root.join("dux.sqlite3");
    assert!(
        SecureSnapshotStore::open_for_database(&database, SnapshotStoreAccess::ReadOnly)
            .unwrap()
            .is_none()
    );
    assert!(!root.join(DIRECTORY_NAME).exists());
}

#[test]
fn provisioning_stage_reconciliation_reports_no_stage_exactly() {
    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let store = open_rw(&root.join("dux.sqlite3"));
    #[cfg(target_os = "linux")]
    fs::write(
        root.join(std::ffi::OsString::from_vec(vec![0xff, b'x'])),
        b"unrelated",
    )
    .unwrap();

    let result = store.reconcile_provisioning_stage().unwrap();
    assert_eq!(result.outcome(), SnapshotProvisioningStageRemoval::NoStage);
    assert_eq!(result.total_stage_count_before(), 0);
    assert_eq!(result.total_stage_count_after(), 0);
    assert_eq!(result.removed_control_usage(), None);
    assert!(!result.has_more());
}

#[test]
fn empty_and_stricter_provisioning_stages_are_deferred() {
    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let store = open_rw(&root.join("dux.sqlite3"));
    let empty = provisioning_stage(&root, "00000000000000000000000000000000", false, false);
    let stricter = provisioning_stage(&root, "11111111111111111111111111111111", false, false);
    fs::set_permissions(&stricter, fs::Permissions::from_mode(0o500)).unwrap();
    let opaque = provisioning_stage(&root, "22222222222222222222222222222222", false, false);
    fs::set_permissions(&opaque, fs::Permissions::from_mode(0o000)).unwrap();

    let result = store.reconcile_provisioning_stage().unwrap();
    assert_eq!(
        result.outcome(),
        SnapshotProvisioningStageRemoval::DeferredUnproven
    );
    assert_eq!(result.total_stage_count_before(), 3);
    assert_eq!(result.unproven_count_before(), 3);
    assert_eq!(result.marker_owned_count_before(), 0);
    assert!(empty.exists());
    assert!(stricter.exists());
    assert!(opaque.exists());
}

#[test]
fn empty_stage_does_not_starve_later_marker_owned_stage() {
    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let store = open_rw(&root.join("dux.sqlite3"));
    let empty = provisioning_stage(&root, "00000000000000000000000000000000", false, false);
    let removable = provisioning_stage(&root, "11111111111111111111111111111111", true, false);

    let result = store.reconcile_provisioning_stage().unwrap();
    assert_eq!(
        result.outcome(),
        SnapshotProvisioningStageRemoval::RemovedMarkerOnly
    );
    assert_eq!(result.total_stage_count_before(), 2);
    assert_eq!(result.total_stage_count_after(), 1);
    assert_eq!(result.marker_owned_count_before(), 1);
    assert_eq!(result.marker_owned_count_after(), 0);
    assert_eq!(result.unproven_count_before(), 1);
    assert_eq!(result.unproven_count_after(), 1);
    assert_eq!(result.removed_control_usage().unwrap().logical_bytes(), 16);
    assert_eq!(result.control_usage_after(), SnapshotFileUsage::default());
    assert!(!result.has_more());
    assert!(empty.exists());
    assert!(!removable.exists());
}

#[test]
fn marker_owned_stages_are_removed_lexically_one_at_a_time_with_exact_accounting() {
    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let store = open_rw(&root.join("dux.sqlite3"));
    let first = provisioning_stage(&root, "00000000000000000000000000000000", true, true);
    let second = provisioning_stage(&root, "11111111111111111111111111111111", true, false);

    let first_result = store.reconcile_provisioning_stage().unwrap();
    assert_eq!(
        first_result.outcome(),
        SnapshotProvisioningStageRemoval::RemovedMarkerComplete
    );
    assert_eq!(first_result.total_stage_count_before(), 2);
    assert_eq!(first_result.total_stage_count_after(), 1);
    assert_eq!(first_result.marker_owned_count_before(), 2);
    assert_eq!(first_result.marker_owned_count_after(), 1);
    assert_eq!(
        first_result
            .removed_control_usage()
            .unwrap()
            .logical_bytes(),
        32
    );
    assert_eq!(first_result.control_usage_after().logical_bytes(), 16);
    assert!(first_result.has_more());
    assert!(!first.exists());
    assert!(second.exists());

    let second_result = store.reconcile_provisioning_stage().unwrap();
    assert_eq!(
        second_result.outcome(),
        SnapshotProvisioningStageRemoval::RemovedMarkerOnly
    );
    assert_eq!(second_result.total_stage_count_after(), 0);
    assert_eq!(second_result.marker_owned_count_after(), 0);
    assert_eq!(
        second_result.control_usage_after(),
        SnapshotFileUsage::default()
    );
    assert!(!second_result.has_more());
    assert!(!second.exists());
}

#[test]
fn malformed_provisioning_stage_fails_before_effect_and_preserves_valid_debt() {
    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let store = open_rw(&root.join("dux.sqlite3"));
    let valid = provisioning_stage(&root, "00000000000000000000000000000000", true, false);
    let malformed = provisioning_stage(&root, "11111111111111111111111111111111", false, true);

    assert!(matches!(
        store.reconcile_provisioning_stage(),
        Err(SnapshotProvisioningStageRemovalError::BeforeEffect(_))
    ));
    assert!(valid.exists());
    assert!(valid.join(MARKER_NAME).exists());
    assert!(malformed.exists());
}

#[test]
#[allow(clippy::disallowed_methods)]
fn wrong_marker_and_broad_stage_mode_fail_without_effect() {
    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let store = open_rw(&root.join("dux.sqlite3"));
    let wrong = provisioning_stage(&root, "00000000000000000000000000000000", true, false);
    fs::write(wrong.join(MARKER_NAME), b"NOTDUXSNAPSTORE!").unwrap();
    fs::set_permissions(wrong.join(MARKER_NAME), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        store.reconcile_provisioning_stage(),
        Err(SnapshotProvisioningStageRemovalError::BeforeEffect(_))
    ));
    assert!(wrong.join(MARKER_NAME).exists());

    // Remove only the test fixture's invalid stage, then independently
    // prove that broader directory authority is rejected.
    // DUX-DESTRUCTIVE: allow=test-snapshot-provisioning-stage-fixture-control-reset -- remove only this TempDir-owned malformed control between two independent no-effect assertions
    fs::remove_file(wrong.join(MARKER_NAME)).unwrap();
    // DUX-DESTRUCTIVE: allow=test-snapshot-provisioning-stage-fixture-directory-reset -- remove only the now-empty TempDir-owned malformed stage between two independent no-effect assertions
    fs::remove_dir(&wrong).unwrap();
    let broad = provisioning_stage(&root, "11111111111111111111111111111111", true, false);
    fs::set_permissions(&broad, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(matches!(
        store.reconcile_provisioning_stage(),
        Err(SnapshotProvisioningStageRemovalError::BeforeEffect(_))
    ));
    assert!(broad.join(MARKER_NAME).exists());
}

#[test]
#[allow(clippy::disallowed_methods)]
fn canonical_stage_file_and_symlink_fail_without_touching_targets() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let store = open_rw(&root.join("dux.sqlite3"));
    let file_name = format!("{PROVISIONING_STAGE_PREFIX}00000000000000000000000000000000");
    let file = root.join(&file_name);
    fs::write(&file, b"not a directory").unwrap();
    assert!(matches!(
        store.reconcile_provisioning_stage(),
        Err(SnapshotProvisioningStageRemovalError::BeforeEffect(_))
    ));
    assert_eq!(fs::read(&file).unwrap(), b"not a directory");

    // DUX-DESTRUCTIVE: allow=test-snapshot-provisioning-stage-file-reset -- remove only the TempDir-owned hostile file fixture before the symlink case
    fs::remove_file(&file).unwrap();
    let target = root.join("legacy-external");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("user-data"), b"untouched").unwrap();
    symlink(&target, &file).unwrap();
    assert!(matches!(
        store.reconcile_provisioning_stage(),
        Err(SnapshotProvisioningStageRemovalError::BeforeEffect(_))
    ));
    assert_eq!(fs::read(target.join("user-data")).unwrap(), b"untouched");
    assert!(file.symlink_metadata().unwrap().file_type().is_symlink());
}

#[test]
fn provisioning_stage_candidate_cap_accepts_64_and_rejects_65_without_effect() {
    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let store = open_rw(&root.join("dux.sqlite3"));
    for value in 0_u64..64 {
        provisioning_stage(&root, &format!("{value:032x}"), false, false);
    }
    let accepted = store.reconcile_provisioning_stage().unwrap();
    assert_eq!(
        accepted.outcome(),
        SnapshotProvisioningStageRemoval::DeferredUnproven
    );
    assert_eq!(accepted.total_stage_count_before(), 64);
    assert_eq!(accepted.unproven_count_before(), 64);

    provisioning_stage(&root, &format!("{:032x}", 64_u64), false, false);
    assert!(matches!(
        store.reconcile_provisioning_stage(),
        Err(SnapshotProvisioningStageRemovalError::BeforeEffect(_))
    ));
    for value in 0_u64..65 {
        assert!(
            root.join(format!("{PROVISIONING_STAGE_PREFIX}{value:032x}"))
                .exists()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)]
fn extra_or_linked_provisioning_stage_children_fail_without_effect() {
    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let store = open_rw(&root.join("dux.sqlite3"));
    let stage = provisioning_stage(&root, "00000000000000000000000000000000", true, false);
    fs::write(stage.join("extra"), b"unknown").unwrap();
    fs::set_permissions(stage.join("extra"), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        store.reconcile_provisioning_stage(),
        Err(SnapshotProvisioningStageRemovalError::BeforeEffect(_))
    ));
    assert!(stage.join(MARKER_NAME).exists());

    // DUX-DESTRUCTIVE: allow=test-snapshot-provisioning-stage-extra-reset -- remove only this TempDir-owned hostile extra fixture before the independent hard-link assertion
    fs::remove_file(stage.join("extra")).unwrap();
    fs::hard_link(stage.join(MARKER_NAME), root.join("linked-control")).unwrap();
    assert!(matches!(
        store.reconcile_provisioning_stage(),
        Err(SnapshotProvisioningStageRemovalError::BeforeEffect(_))
    ));
    assert!(stage.join(MARKER_NAME).exists());
}

#[test]
fn provisioning_stage_effect_boundary_is_exact_under_injected_failures() {
    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let store = open_rw(&root.join("dux.sqlite3"));
    let stage = provisioning_stage(&root, "00000000000000000000000000000000", true, false);
    let retained = store
        .inventory_provisioning_stages()
        .unwrap()
        .first_marker_owned
        .unwrap();
    let before = store.remove_provisioning_stage_with_hooks(
        retained,
        || {
            Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            ))
        },
        platform::sync_directory,
        platform::sync_directory,
    );
    assert!(matches!(
        before,
        Err(SnapshotProvisioningStageRemovalError::BeforeEffect(_))
    ));
    assert!(stage.join(MARKER_NAME).exists());

    let retained = store
        .inventory_provisioning_stages()
        .unwrap()
        .first_marker_owned
        .unwrap();
    let after = store.remove_provisioning_stage_with_hooks(
        retained,
        || Ok(()),
        platform::sync_directory,
        |_| {
            Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            ))
        },
    );
    assert!(matches!(
        after,
        Err(SnapshotProvisioningStageRemovalError::OutcomeUnknown)
    ));
    assert!(!stage.exists());
}

#[test]
fn provisions_marker_complete_private_store_and_files() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let root = database.parent().unwrap().join(DIRECTORY_NAME);
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    for control in [MARKER_NAME, WRITER_LOCK_NAME] {
        assert_eq!(
            fs::metadata(root.join(control))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o600
        );
    }

    let name = SnapshotFileName::from_scan_id(b"private");
    let mut stage = store
        .stage(name.clone(), Duration::from_millis(100))
        .unwrap();
    stage.write_all(b"snapshot bytes").unwrap();
    let SnapshotPublication::Published(published) = stage.publish_no_replace().unwrap() else {
        panic!("first publication must win");
    };
    assert_eq!(published.name(), &name);
    assert_eq!(published.len().unwrap(), 14);
    let mut published_handle = published.try_clone_file().unwrap();
    assert!(
        published_handle
            .write_all(b"must remain immutable")
            .is_err(),
        "published snapshot handles must be read-only"
    );
    assert_eq!(
        fs::metadata(root.join(name.as_str()))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );
}

#[test]
fn provisioning_stages_and_collision_losers_stay_inside_their_database_root() {
    let temp = TempDir::new().unwrap();
    let legacy_stage_name = ".dux-snapshot-stage-00000000000000000000000000000000";
    let legacy_stage = temp.path().join(legacy_stage_name);
    fs::create_dir(&legacy_stage).unwrap();
    fs::write(legacy_stage.join("legacy"), b"leave untouched").unwrap();

    let roots: Vec<PathBuf> = ["First", "Second"]
        .into_iter()
        .map(|name| {
            let root = temp.path().join(name);
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            root
        })
        .collect();
    let barrier = Arc::new(Barrier::new(roots.len() * 2));
    let mut provisioners = Vec::new();
    for root in &roots {
        for _ in 0..2 {
            let root = root.clone();
            let barrier = Arc::clone(&barrier);
            provisioners.push(thread::spawn(move || {
                let directory = platform::open_private_directory(&root).unwrap();
                let identity =
                    Identity(platform::identity(&directory, platform::Kind::Directory).unwrap());
                drop(
                    SecureSnapshotStore::provision_with_before_publish(
                        &root,
                        &directory,
                        identity,
                        root.join(DIRECTORY_NAME),
                        || {
                            barrier.wait();
                        },
                    )
                    .unwrap(),
                );
            }));
        }
    }
    for provisioner in provisioners {
        provisioner.join().unwrap();
    }

    let sibling_stages: Vec<_> = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(".dux-snapshot-stage-"))
        .collect();
    assert_eq!(sibling_stages, [legacy_stage_name]);
    assert_eq!(
        fs::read(legacy_stage.join("legacy")).unwrap(),
        b"leave untouched"
    );

    for root in roots {
        assert!(root.join(DIRECTORY_NAME).is_dir());
        let collision_losers: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".dux-snapshot-stage-")
            })
            .collect();
        assert_eq!(collision_losers.len(), 1);
        let collision_loser = collision_losers[0].path();
        assert_eq!(
            fs::read(collision_loser.join(MARKER_NAME)).unwrap(),
            STORE_MARKER
        );
        assert_eq!(
            fs::read(collision_loser.join(WRITER_LOCK_NAME)).unwrap(),
            WRITER_MARKER
        );
    }
}

#[test]
fn inventory_accounts_finals_temps_and_controls_under_one_lease() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let final_name = SnapshotFileName::from_scan_id(b"inventory-final");
    let final_bytes = b"published inventory bytes";
    let mut published = store
        .stage(final_name.clone(), Duration::from_millis(100))
        .unwrap();
    published.write_all(final_bytes).unwrap();
    drop(published.publish_no_replace().unwrap());

    let mut staged = store
        .stage(
            SnapshotFileName::from_scan_id(b"inventory-temp"),
            Duration::from_millis(100),
        )
        .unwrap();
    staged.write_all(b"unknown-liveness-temp").unwrap();
    staged.sync_all().unwrap();
    let staged_name = staged.temp_name.clone();

    let inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    assert_eq!(inventory.entries().len(), 2);
    let final_entry = inventory
        .entries()
        .iter()
        .find(|entry| entry.name() == final_name.as_str())
        .unwrap();
    assert_eq!(
        final_entry.kind(),
        &SnapshotInventoryEntryKind::Final(final_name.clone())
    );
    assert_eq!(
        final_entry.usage().logical_bytes(),
        final_bytes.len() as u64
    );
    let final_metadata = fs::metadata(
        database
            .parent()
            .unwrap()
            .join(DIRECTORY_NAME)
            .join(final_name.as_str()),
    )
    .unwrap();
    assert_eq!(
        final_entry.usage().allocated_bytes(),
        final_metadata.blocks() * 512
    );
    assert_eq!(
        final_entry.usage().charged_bytes(),
        final_entry
            .usage()
            .logical_bytes()
            .max(final_entry.usage().allocated_bytes())
    );
    let temp_entry = inventory
        .entries()
        .iter()
        .find(|entry| entry.name() == staged_name)
        .unwrap();
    assert_eq!(
        temp_entry.kind(),
        &SnapshotInventoryEntryKind::RecognizedTemp
    );
    assert_eq!(
        temp_entry.temp_kernel_state(),
        Some(SnapshotTempKernelState::Active)
    );

    let controls = inventory.control_usage();
    assert_eq!(controls.store_marker().logical_bytes(), 16);
    assert_eq!(controls.writer_lock().logical_bytes(), 16);
    assert_eq!(controls.total().logical_bytes(), 32);
    assert_eq!(
        inventory.total_usage().logical_bytes(),
        inventory.entries_usage().logical_bytes() + 32
    );
    inventory.revalidate().unwrap();
    assert_eq!(
        store
            .stage(
                SnapshotFileName::from_scan_id(b"inventory-lock-proof"),
                Duration::from_millis(1),
            )
            .err()
            .unwrap()
            .kind(),
        SnapshotStorageErrorKind::Busy
    );
    drop(inventory);
    staged.abort().unwrap();
}

#[test]
fn reset_revalidation_rejects_a_new_child_despite_retained_writer_exclusion() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();

    fs::write(
        database
            .parent()
            .unwrap()
            .join(DIRECTORY_NAME)
            .join("unknown"),
        b"actor ignored the advisory writer lock",
    )
    .unwrap();

    assert_eq!(
        inventory.revalidate_complete().unwrap_err().kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test renames only a TempDir-owned snapshot store to prove canonical-name revalidation"
)]
fn reset_revalidation_rejects_detached_canonical_snapshot_directory() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let canonical = database.parent().unwrap().join(DIRECTORY_NAME);
    let detached = database.parent().unwrap().join("detached-snapshots");

    // DUX-DESTRUCTIVE: allow=test-snapshot-reset-canonical-binding-rename -- rename only this TempDir-owned marker-validated snapshot directory to prove a retained descriptor cannot stand in for the canonical name
    fs::rename(&canonical, &detached).unwrap();

    assert!(matches!(
        inventory.revalidate_complete().unwrap_err().kind(),
        SnapshotStorageErrorKind::UnsafeRoot | SnapshotStorageErrorKind::UnsafeObject
    ));
    assert!(!canonical.exists());
    assert!(detached.exists());
}

#[test]
fn inventory_lease_retains_exact_final_and_rejects_same_name_replacement() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let store = open_rw(&database_path(&temp));
    let exact_name = SnapshotFileName::from_scan_id(b"retention-retain-exact");
    let replaced_name = SnapshotFileName::from_scan_id(b"retention-retain-replaced");
    for (name, bytes) in [
        (exact_name.clone(), b"exact retained bytes".as_slice()),
        (replaced_name.clone(), b"same-size-old".as_slice()),
    ] {
        let mut staged = store.stage(name, Duration::from_millis(100)).unwrap();
        staged.write_all(bytes).unwrap();
        drop(staged.publish_no_replace().unwrap());
    }

    let inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let retained = inventory.retain_observed_final(&exact_name).unwrap();
    assert_eq!(retained.name(), &exact_name);
    assert_eq!(retained.len().unwrap(), 20);
    let mut exact_bytes = Vec::new();
    retained
        .try_clone_file()
        .unwrap()
        .read_to_end(&mut exact_bytes)
        .unwrap();
    assert_eq!(exact_bytes, b"exact retained bytes");
    retained.revalidate().unwrap();
    drop(retained);

    let (original, original_identity) = platform::open_named_final_for_removal(
        &store.inner.directory,
        &store.inner.path,
        replaced_name.as_str(),
    )
    .unwrap()
    .unwrap();
    platform::remove_retained_final(
        &store.inner.directory,
        replaced_name.as_str(),
        original,
        original_identity,
    )
    .unwrap();
    platform::sync_directory(&store.inner.directory).unwrap();
    let (mut replacement, _) = platform::create_private_file_exclusive(
        &store.inner.directory,
        &store.inner.path,
        replaced_name.as_str(),
    )
    .unwrap()
    .unwrap();
    replacement.write_all(b"same-size-new").unwrap();
    replacement.sync_all().unwrap();
    drop(replacement);

    assert_eq!(
        inventory
            .retain_observed_final(&replaced_name)
            .err()
            .unwrap()
            .kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );
}

#[test]
fn inventory_lease_removes_only_the_exact_observed_final_and_stays_valid() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let removed_name = SnapshotFileName::from_scan_id(b"retention-remove-final");
    let retained_name = SnapshotFileName::from_scan_id(b"retention-keep-final");
    for (name, bytes) in [
        (removed_name.clone(), b"removed snapshot".as_slice()),
        (retained_name.clone(), b"retained snapshot".as_slice()),
    ] {
        let mut staged = store.stage(name, Duration::from_millis(100)).unwrap();
        staged.write_all(bytes).unwrap();
        drop(staged.publish_no_replace().unwrap());
    }

    let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);
    let mut inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let expected_usage = inventory
        .entries()
        .iter()
        .find(|entry| entry.name() == removed_name.as_str())
        .unwrap()
        .usage();
    let before_total = inventory.total_usage();

    let retained = inventory.retain_observed_final(&removed_name).unwrap();
    assert_eq!(
        inventory
            .remove_observed_final_reconciled(&retained)
            .unwrap(),
        expected_usage
    );
    drop(retained);
    assert!(!snapshot_root.join(removed_name.as_str()).exists());
    assert!(snapshot_root.join(retained_name.as_str()).exists());
    assert!(
        inventory
            .entries()
            .iter()
            .all(|entry| entry.name() != removed_name.as_str())
    );
    assert_eq!(
        inventory.total_usage().logical_bytes(),
        before_total.logical_bytes() - expected_usage.logical_bytes()
    );
    inventory.revalidate().unwrap();

    let never_observed = SnapshotFileName::from_scan_id(b"retention-never-observed");
    assert_eq!(
        inventory
            .retain_observed_final(&never_observed)
            .err()
            .unwrap()
            .kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );
    assert!(snapshot_root.join(retained_name.as_str()).exists());
}

#[test]
fn reconciled_final_removal_reports_post_unlink_sync_failure_as_outcome_unknown() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let removed_name = SnapshotFileName::from_scan_id(b"reconciled-sync-unknown");
    let mut staged = store
        .stage(removed_name.clone(), Duration::from_millis(100))
        .unwrap();
    staged.write_all(b"snapshot bytes").unwrap();
    drop(staged.publish_no_replace().unwrap());

    let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);
    let mut inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let before_entries_usage = inventory.entries_usage();
    let before_total_usage = inventory.total_usage();
    let retained = inventory.retain_observed_final(&removed_name).unwrap();

    let error = inventory
        .remove_observed_final_with_sync(&retained, |_| {
            Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            ))
        })
        .unwrap_err();
    assert!(matches!(error, SnapshotFinalRemovalError::OutcomeUnknown));
    assert_eq!(inventory.entries_usage(), before_entries_usage);
    assert_eq!(inventory.total_usage(), before_total_usage);
    assert!(
        inventory
            .entries()
            .iter()
            .any(|entry| entry.name() == removed_name.as_str())
    );

    drop(retained);
    assert!(!snapshot_root.join(removed_name.as_str()).exists());
    drop(inventory);

    let fresh = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    assert!(
        fresh
            .entries()
            .iter()
            .all(|entry| entry.name() != removed_name.as_str())
    );
}

#[test]
fn reconciled_temp_removal_updates_exact_inventory_accounting() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let mut staged = store
        .stage(
            SnapshotFileName::from_scan_id(b"reconciled-temp-accounting"),
            Duration::from_millis(100),
        )
        .unwrap();
    staged.write_all(b"temporary snapshot bytes").unwrap();
    staged.sync_all().unwrap();
    let temp_name = staged.temp_name.clone();
    staged.abandon();

    let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);
    let mut inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let expected_usage = inventory
        .entries()
        .iter()
        .find(|entry| entry.name() == temp_name)
        .unwrap()
        .usage();
    let before_entries_usage = inventory.entries_usage();
    let before_total_usage = inventory.total_usage();

    assert_eq!(
        inventory
            .remove_observed_quiescent_temp_reconciled(&temp_name)
            .unwrap(),
        expected_usage
    );
    assert!(!snapshot_root.join(&temp_name).exists());
    assert!(
        inventory
            .entries()
            .iter()
            .all(|entry| entry.name() != temp_name)
    );
    assert_eq!(
        inventory.entries_usage(),
        before_entries_usage.checked_sub(expected_usage).unwrap()
    );
    assert_eq!(
        inventory.total_usage(),
        before_total_usage.checked_sub(expected_usage).unwrap()
    );
    inventory.revalidate().unwrap();
}

#[test]
fn unleased_temp_capability_removes_only_the_exact_quiescent_observation() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let final_name = SnapshotFileName::from_scan_id(b"unleased-capability-final");
    let mut final_stage = store
        .stage(final_name.clone(), Duration::from_millis(100))
        .unwrap();
    final_stage.write_all(b"immutable final bytes").unwrap();
    drop(final_stage.publish_no_replace().unwrap());

    let mut temp_stage = store
        .stage(
            SnapshotFileName::from_scan_id(b"unleased-capability-temp"),
            Duration::from_millis(100),
        )
        .unwrap();
    temp_stage.write_all(b"unleased temporary bytes").unwrap();
    temp_stage.sync_all().unwrap();
    let temp_name = temp_stage.temp_name.clone();
    temp_stage.abandon();

    let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);
    let mut inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let expected_usage = inventory
        .entries()
        .iter()
        .find(|entry| entry.name() == temp_name)
        .unwrap()
        .usage();
    let before_entries_usage = inventory.entries_usage();
    let before_total_usage = inventory.total_usage();

    match inventory
        .remove_observed_unleased_temp_reconciled(final_name.as_str())
        .unwrap_err()
    {
        SnapshotTempRemovalError::BeforeEffect(error) => {
            assert_eq!(error.kind(), SnapshotStorageErrorKind::UnsafeObject);
        }
        SnapshotTempRemovalError::OutcomeUnknown => {
            panic!("a final-name rejection must precede any physical effect")
        }
    }
    assert!(snapshot_root.join(final_name.as_str()).exists());
    assert!(snapshot_root.join(&temp_name).exists());

    assert_eq!(
        inventory
            .remove_observed_unleased_temp_reconciled(&temp_name)
            .unwrap(),
        expected_usage
    );
    assert!(!snapshot_root.join(&temp_name).exists());
    assert!(snapshot_root.join(final_name.as_str()).exists());
    assert_eq!(
        inventory.entries_usage(),
        before_entries_usage.checked_sub(expected_usage).unwrap()
    );
    assert_eq!(
        inventory.total_usage(),
        before_total_usage.checked_sub(expected_usage).unwrap()
    );
    inventory.revalidate().unwrap();
}

#[test]
fn noncanonical_unleased_temp_pid_names_are_never_recognized_or_removed() {
    for pid in ["0", "01", "4294967296"] {
        let temp = TempDir::new().unwrap();
        private_database_root(&temp);
        let database = database_path(&temp);
        let store = open_rw(&database);
        // Keep the exact generated layout: digest.pid.random.tmp.
        let name = format!(
            ".snapshot-{}.{}.{}.tmp",
            "a".repeat(FINAL_HEX_LENGTH),
            pid,
            "b".repeat(32),
        );
        assert!(!is_recognized_temp_name(&name));
        let (mut file, _) = platform::create_private_file_exclusive(
            &store.inner.directory,
            &store.inner.path,
            &name,
        )
        .unwrap()
        .unwrap();
        file.write_all(b"not generated by DUX").unwrap();
        file.sync_all().unwrap();
        drop(file);

        let error = match store.inventory_with_writer_lease(Duration::from_millis(100)) {
            Ok(_) => panic!("noncanonical temp name must make inventory unavailable"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), SnapshotStorageErrorKind::UnsafeObject);
        assert!(
            database
                .parent()
                .unwrap()
                .join(DIRECTORY_NAME)
                .join(&name)
                .exists()
        );
    }
}

#[test]
fn legacy_temp_removal_wrapper_preserves_durable_success_behavior() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let staged = store
        .stage(
            SnapshotFileName::from_scan_id(b"legacy-temp-removal-wrapper"),
            Duration::from_millis(100),
        )
        .unwrap();
    let temp_name = staged.temp_name.clone();
    staged.abandon();

    let mut inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    inventory.remove_quiescent_temp(&temp_name).unwrap();
    assert!(
        inventory
            .entries()
            .iter()
            .all(|entry| entry.name() != temp_name)
    );
    inventory.revalidate().unwrap();
}

#[test]
fn reconciled_temp_removal_rejects_active_observation_before_effect() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let staged = store
        .stage(
            SnapshotFileName::from_scan_id(b"reconciled-active-temp"),
            Duration::from_millis(100),
        )
        .unwrap();
    let temp_name = staged.temp_name.clone();
    let snapshot_path = database
        .parent()
        .unwrap()
        .join(DIRECTORY_NAME)
        .join(&temp_name);
    let mut inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();

    match inventory
        .remove_observed_quiescent_temp_reconciled(&temp_name)
        .unwrap_err()
    {
        SnapshotTempRemovalError::BeforeEffect(error) => {
            assert_eq!(error.kind(), SnapshotStorageErrorKind::UnsafeObject);
        }
        SnapshotTempRemovalError::OutcomeUnknown => {
            panic!("active observation must fail before physical removal")
        }
    }
    assert!(snapshot_path.exists());
    drop(inventory);
    staged.abandon();
}

#[test]
fn reconciled_temp_removal_rejects_usage_change_before_effect() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let mut staged = store
        .stage(
            SnapshotFileName::from_scan_id(b"reconciled-temp-usage-change"),
            Duration::from_millis(100),
        )
        .unwrap();
    staged.write_all(b"initial bytes").unwrap();
    staged.sync_all().unwrap();
    let temp_name = staged.temp_name.clone();
    staged.abandon();

    let snapshot_path = database
        .parent()
        .unwrap()
        .join(DIRECTORY_NAME)
        .join(&temp_name);
    let mut inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let mut changed = fs::OpenOptions::new()
        .append(true)
        .open(&snapshot_path)
        .unwrap();
    changed.write_all(b" changed").unwrap();
    changed.sync_all().unwrap();
    drop(changed);

    match inventory
        .remove_observed_quiescent_temp_reconciled(&temp_name)
        .unwrap_err()
    {
        SnapshotTempRemovalError::BeforeEffect(error) => {
            assert_eq!(error.kind(), SnapshotStorageErrorKind::UnsafeObject);
        }
        SnapshotTempRemovalError::OutcomeUnknown => {
            panic!("usage change must fail before physical removal")
        }
    }
    assert!(snapshot_path.exists());
}

#[test]
fn reconciled_temp_removal_freezes_accounting_before_effect() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let mut staged = store
        .stage(
            SnapshotFileName::from_scan_id(b"reconciled-temp-accounting-underflow"),
            Duration::from_millis(100),
        )
        .unwrap();
    staged.write_all(b"nonempty temporary bytes").unwrap();
    staged.sync_all().unwrap();
    let temp_name = staged.temp_name.clone();
    staged.abandon();

    let snapshot_path = database
        .parent()
        .unwrap()
        .join(DIRECTORY_NAME)
        .join(&temp_name);
    let mut inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    inventory.entries_usage = SnapshotFileUsage::default();

    match inventory
        .remove_observed_quiescent_temp_reconciled(&temp_name)
        .unwrap_err()
    {
        SnapshotTempRemovalError::BeforeEffect(error) => {
            assert_eq!(error.kind(), SnapshotStorageErrorKind::UnsafeObject);
        }
        SnapshotTempRemovalError::OutcomeUnknown => {
            panic!("accounting underflow must fail before physical removal")
        }
    }
    assert!(snapshot_path.exists());
}

#[test]
fn reconciled_temp_removal_reports_post_unlink_sync_failure_as_outcome_unknown() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let mut staged = store
        .stage(
            SnapshotFileName::from_scan_id(b"reconciled-temp-sync-unknown"),
            Duration::from_millis(100),
        )
        .unwrap();
    staged.write_all(b"temporary snapshot bytes").unwrap();
    staged.sync_all().unwrap();
    let temp_name = staged.temp_name.clone();
    staged.abandon();

    let snapshot_path = database
        .parent()
        .unwrap()
        .join(DIRECTORY_NAME)
        .join(&temp_name);
    let mut inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let before_entries_usage = inventory.entries_usage();
    let before_total_usage = inventory.total_usage();

    let error = inventory
        .remove_quiescent_temp_with_sync(&temp_name, |_| {
            Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            ))
        })
        .unwrap_err();
    assert!(matches!(error, SnapshotTempRemovalError::OutcomeUnknown));
    assert_eq!(inventory.entries_usage(), before_entries_usage);
    assert_eq!(inventory.total_usage(), before_total_usage);
    assert!(
        inventory
            .entries()
            .iter()
            .any(|entry| entry.name() == temp_name)
    );
    assert!(!snapshot_path.exists());
    drop(inventory);

    let fresh = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    assert!(
        fresh
            .entries()
            .iter()
            .all(|entry| entry.name() != temp_name)
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_last_payload_advances_the_retained_structural_state() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    publish_test_snapshot(&store, b"last-reset-payload", b"snapshot bytes");
    drop(store);

    let deadline = reset_payload_test_deadline();
    let mut recovery = AppDataResetSnapshotRecovery::open_until(
        database.parent().unwrap(),
        File::open(database.parent().unwrap()).unwrap(),
        true,
        deadline,
    )
    .unwrap();
    assert_eq!(recovery.retirement_state(), None);
    let candidate = recovery
        .payload_drain_candidate_until(deadline)
        .unwrap()
        .expect("the sole payload must be selected");
    let completion = recovery
        .drain_one_payload(
            candidate,
            AppDataResetSnapshotPayloadDrainAuthority::for_test(deadline),
        )
        .expect("the last payload must have a certain success result");
    let post_effect_deadline = completion.post_effect_deadline();
    assert!(!completion.into_progress().snapshot_payload_has_more());
    assert_eq!(
        recovery.retirement_state(),
        Some(AppDataResetSnapshotStoreRetirementState::FullControlsEmpty)
    );
    recovery
        .revalidate_after_payload_effect_until(post_effect_deadline)
        .unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_snapshot_store_structural_tail_is_monotonic_and_one_effect_per_open() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);
    drop(store);

    for (state, has_more) in [
        (
            AppDataResetSnapshotStoreRetirementState::FullControlsEmpty,
            true,
        ),
        (AppDataResetSnapshotStoreRetirementState::WriterOnly, true),
        (
            AppDataResetSnapshotStoreRetirementState::EmptyDirectory,
            false,
        ),
    ] {
        let deadline = reset_payload_test_deadline();
        let mut recovery = AppDataResetSnapshotRecovery::open_until(
            database.parent().unwrap(),
            File::open(database.parent().unwrap()).unwrap(),
            true,
            deadline,
        )
        .unwrap();
        assert_eq!(recovery.retirement_state(), Some(state));
        recovery.revalidate_until(deadline).unwrap();
        let progress = recovery
            .retire_one_structure(AppDataResetSnapshotStoreRetireAuthority::for_test(deadline))
            .unwrap()
            .into_progress();
        assert_eq!(progress.removed_structural_objects(), 1);
        assert_eq!(progress.snapshot_store_has_more(), has_more);
    }

    assert!(!snapshot_root.exists());
    let deadline = reset_payload_test_deadline();
    let recovery = AppDataResetSnapshotRecovery::open_until(
        database.parent().unwrap(),
        File::open(database.parent().unwrap()).unwrap(),
        true,
        deadline,
    )
    .unwrap();
    assert_eq!(
        recovery.retirement_state(),
        Some(AppDataResetSnapshotStoreRetirementState::Absent)
    );
    recovery.revalidate_until(deadline).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
#[allow(clippy::disallowed_methods)]
fn app_data_reset_snapshot_store_rejects_case_alias_at_the_final_gate() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    drop(open_rw(&database));
    let root = database.parent().unwrap();
    let canonical = root.join(DIRECTORY_NAME);
    let alias = root.join("Snapshots");
    let marker = alias.join(MARKER_NAME);

    let deadline = reset_payload_test_deadline();
    let mut recovery =
        AppDataResetSnapshotRecovery::open_until(root, File::open(root).unwrap(), true, deadline)
            .unwrap();
    set_test_app_data_reset_snapshot_store_retirement_fault(
        TestAppDataResetSnapshotStoreRetirementFault::CaseAliasAtFinalGate,
    );
    assert_eq!(
        recovery
            .retire_one_structure(AppDataResetSnapshotStoreRetireAuthority::for_test(deadline,))
            .unwrap_err(),
        AppDataResetSnapshotStoreRetirementError::BeforeEffect(
            SnapshotStorageErrorKind::UnsafeObject
        )
    );
    assert!(marker.exists(), "case-only alias lost its ownership marker");

    // DUX-DESTRUCTIVE: allow=test-reset-snapshot-case-alias-restore -- restore only the test fixture's retained case-only snapshot directory spelling before proving a fresh exact-gate retry
    std::fs::rename(&alias, &canonical).unwrap();
    recovery.revalidate_until(deadline).unwrap();
    let progress = recovery
        .retire_one_structure(AppDataResetSnapshotStoreRetireAuthority::for_test(deadline))
        .unwrap()
        .into_progress();
    assert_eq!(progress.removed_structural_objects(), 1);
    assert!(progress.snapshot_store_has_more());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_snapshot_store_retirement_uncertainty_is_restart_safe_at_every_state() {
    for state in [
        AppDataResetSnapshotStoreRetirementState::FullControlsEmpty,
        AppDataResetSnapshotStoreRetirementState::WriterOnly,
        AppDataResetSnapshotStoreRetirementState::EmptyDirectory,
    ] {
        let successor = match state {
            AppDataResetSnapshotStoreRetirementState::FullControlsEmpty => {
                AppDataResetSnapshotStoreRetirementState::WriterOnly
            }
            AppDataResetSnapshotStoreRetirementState::WriterOnly => {
                AppDataResetSnapshotStoreRetirementState::EmptyDirectory
            }
            AppDataResetSnapshotStoreRetirementState::EmptyDirectory => {
                AppDataResetSnapshotStoreRetirementState::Absent
            }
            AppDataResetSnapshotStoreRetirementState::Absent => unreachable!(),
        };
        for (fault, before_effect_kind) in [
            (
                TestAppDataResetSnapshotStoreRetirementFault::BeforeEffect,
                Some(SnapshotStorageErrorKind::Unavailable),
            ),
            (
                TestAppDataResetSnapshotStoreRetirementFault::ExhaustPreEffectDeadline,
                Some(SnapshotStorageErrorKind::Busy),
            ),
            (
                TestAppDataResetSnapshotStoreRetirementFault::ForeignFilesystemAtFinalGate,
                Some(SnapshotStorageErrorKind::UnsafeObject),
            ),
            (
                TestAppDataResetSnapshotStoreRetirementFault::ExhaustDeadlineAtFinalGate,
                Some(SnapshotStorageErrorKind::Busy),
            ),
            (
                TestAppDataResetSnapshotStoreRetirementFault::AfterEffect,
                None,
            ),
            (
                TestAppDataResetSnapshotStoreRetirementFault::AfterDirectorySync,
                None,
            ),
            (
                TestAppDataResetSnapshotStoreRetirementFault::DuringReadback,
                None,
            ),
            (
                TestAppDataResetSnapshotStoreRetirementFault::ExhaustPostEffectDeadline,
                None,
            ),
        ] {
            let temp = TempDir::new().unwrap();
            private_database_root(&temp);
            let database = database_path(&temp);
            drop(open_rw(&database));
            let (mut recovery, deadline) = reset_snapshot_recovery_at_state(&database, state);

            set_test_app_data_reset_snapshot_store_retirement_fault(fault);
            let error = recovery
                .retire_one_structure(AppDataResetSnapshotStoreRetireAuthority::for_test(deadline))
                .unwrap_err();
            if let Some(kind) = before_effect_kind {
                assert_eq!(
                    error,
                    AppDataResetSnapshotStoreRetirementError::BeforeEffect(kind),
                    "{state:?} {fault:?}"
                );
            } else {
                assert_eq!(
                    error,
                    AppDataResetSnapshotStoreRetirementError::OutcomeUnknown,
                    "{state:?} {fault:?}"
                );
            }
            drop(recovery);

            let (mut restarted, restart_deadline) = reset_snapshot_recovery_at_state(
                &database,
                before_effect_kind.map_or(successor, |_| state),
            );
            if before_effect_kind.is_some() {
                restarted
                    .retire_one_structure(AppDataResetSnapshotStoreRetireAuthority::for_test(
                        restart_deadline,
                    ))
                    .unwrap();
                drop(restarted);
                let (reobserved, _) = reset_snapshot_recovery_at_state(&database, successor);
                drop(reobserved);
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_snapshot_store_writer_contention_is_bounded_and_retryable() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    drop(open_rw(&database));
    let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);

    let (mut full, deadline) = reset_snapshot_recovery_at_state(
        &database,
        AppDataResetSnapshotStoreRetirementState::FullControlsEmpty,
    );
    full.retire_one_structure(AppDataResetSnapshotStoreRetireAuthority::for_test(deadline))
        .unwrap();
    assert!(!snapshot_root.join(MARKER_NAME).exists());
    assert!(snapshot_root.join(WRITER_LOCK_NAME).exists());

    let contended_deadline = Instant::now()
        .checked_add(Duration::from_millis(20))
        .unwrap();
    let error = match AppDataResetSnapshotRecovery::open_until(
        database.parent().unwrap(),
        File::open(database.parent().unwrap()).unwrap(),
        true,
        contended_deadline,
    ) {
        Ok(_) => panic!("contended snapshot writer was admitted"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), SnapshotStorageErrorKind::Busy);

    drop(full);
    let (writer_only, _) = reset_snapshot_recovery_at_state(
        &database,
        AppDataResetSnapshotStoreRetirementState::WriterOnly,
    );
    writer_only
        .revalidate_until(reset_payload_test_deadline())
        .unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
#[allow(clippy::disallowed_methods)]
fn app_data_reset_snapshot_store_rejects_non_monotonic_partial_shapes() {
    for shape in ["marker-only", "writer-with-unknown-child"] {
        let temp = TempDir::new().unwrap();
        private_database_root(&temp);
        let database = database_path(&temp);
        drop(open_rw(&database));
        let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);

        match shape {
            "marker-only" => {
                // DUX-DESTRUCTIVE: allow=test-reset-snapshot-marker-only-fixture -- remove only the writer control inside this TempDir-owned snapshot store to fabricate the otherwise unreachable unsafe marker-only recovery shape
                std::fs::remove_file(snapshot_root.join(WRITER_LOCK_NAME)).unwrap()
            }
            "writer-with-unknown-child" => {
                let (mut full, deadline) = reset_snapshot_recovery_at_state(
                    &database,
                    AppDataResetSnapshotStoreRetirementState::FullControlsEmpty,
                );
                full.retire_one_structure(AppDataResetSnapshotStoreRetireAuthority::for_test(
                    deadline,
                ))
                .unwrap();
                drop(full);
                std::fs::write(snapshot_root.join("unknown-reset-object"), b"foreign").unwrap();
                std::fs::set_permissions(
                    snapshot_root.join("unknown-reset-object"),
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }

        let error = match AppDataResetSnapshotRecovery::open_until(
            database.parent().unwrap(),
            File::open(database.parent().unwrap()).unwrap(),
            true,
            reset_payload_test_deadline(),
        ) {
            Ok(_) => panic!("non-monotonic snapshot shape was admitted: {shape}"),
            Err(error) => error,
        };
        assert_eq!(
            error.kind(),
            SnapshotStorageErrorKind::UnsafeObject,
            "{shape}"
        );
        assert!(snapshot_root.exists(), "{shape}");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_payload_drain_is_lexical_bounded_and_handles_temp_final_and_empty() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let first_final = publish_test_snapshot(&store, b"reset-final-z", b"final z");
    let second_final = publish_test_snapshot(&store, b"reset-final-a", b"final a");
    let temp_name = abandon_test_snapshot_temp(&store, b"reset-temp");
    let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);
    let mut expected_finals = [first_final.as_str(), second_final.as_str()];
    expected_finals.sort_unstable();

    let mut lease = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let deadline = reset_payload_test_deadline();
    let candidate = lease
        .app_data_reset_payload_drain_candidate_until(deadline)
        .unwrap()
        .unwrap();
    assert_eq!(candidate.selected.name, temp_name);
    assert!(candidate.is_bound_to(&lease));
    candidate
        .revalidate_against_until(&lease, deadline)
        .unwrap();
    let first = lease
        .drain_one_app_data_reset_payload(
            candidate,
            AppDataResetSnapshotPayloadDrainAuthority::for_test(deadline),
        )
        .unwrap()
        .into_progress();
    assert_eq!(first.removed_objects(), 1);
    assert!(first.snapshot_payload_has_more());
    assert!(!snapshot_root.join(&temp_name).exists());

    for (index, expected_name) in expected_finals.into_iter().enumerate() {
        let candidate = lease
            .app_data_reset_payload_drain_candidate_until(deadline)
            .unwrap()
            .unwrap();
        assert_eq!(candidate.selected.name, expected_name);
        let progress = lease
            .drain_one_app_data_reset_payload(
                candidate,
                AppDataResetSnapshotPayloadDrainAuthority::for_test(deadline),
            )
            .unwrap()
            .into_progress();
        assert_eq!(progress.removed_objects(), 1);
        assert_eq!(progress.snapshot_payload_has_more(), index == 0);
        assert!(!snapshot_root.join(expected_name).exists());
    }
    assert!(
        lease
            .app_data_reset_payload_drain_candidate_until(deadline)
            .unwrap()
            .is_none()
    );
    lease.revalidate_complete().unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_payload_drain_compares_real_filesystem_identities() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let store = open_rw(&database_path(&temp));
    let identity = store.inner.directory_identity.0;
    assert!(platform::same_filesystem(identity, identity));
    assert!(!platform::same_filesystem(
        identity,
        platform::different_filesystem_identity_for_test(identity)
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_payload_drain_rejects_foreign_store_filesystem_when_empty() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let lease = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    assert!(lease.entries().is_empty());

    set_test_app_data_reset_snapshot_payload_drain_fault(
        TestAppDataResetSnapshotPayloadDrainFault::ForeignStoreFilesystem,
    );
    let error =
        match lease.app_data_reset_payload_drain_candidate_until(reset_payload_test_deadline()) {
            Err(error) => error,
            Ok(_) => panic!("a foreign-filesystem empty snapshot store was admitted"),
        };
    assert_eq!(error.kind(), SnapshotStorageErrorKind::UnsafeObject);
    let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);
    assert!(snapshot_root.join(MARKER_NAME).exists());
    assert!(snapshot_root.join(WRITER_LOCK_NAME).exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_payload_drain_rejects_foreign_nonselected_payload_filesystem() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let names = [
        publish_test_snapshot(&store, b"reset-foreign-payload-a", b"a"),
        publish_test_snapshot(&store, b"reset-foreign-payload-z", b"z"),
    ];
    let lease = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();

    set_test_app_data_reset_snapshot_payload_drain_fault(
        TestAppDataResetSnapshotPayloadDrainFault::ForeignPayloadFilesystem,
    );
    let error =
        match lease.app_data_reset_payload_drain_candidate_until(reset_payload_test_deadline()) {
            Err(error) => error,
            Ok(_) => panic!("an inventory containing a foreign-filesystem payload was admitted"),
        };
    assert_eq!(error.kind(), SnapshotStorageErrorKind::UnsafeObject);
    let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);
    for name in names {
        assert!(snapshot_root.join(name.as_str()).exists());
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_final_payload_rechecks_filesystem_at_exact_effect_gate() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let name = publish_test_snapshot(&store, b"reset-final-gate-final", b"final");
    let path = database
        .parent()
        .unwrap()
        .join(DIRECTORY_NAME)
        .join(name.as_str());
    let mut lease = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let deadline = reset_payload_test_deadline();
    let candidate = lease
        .app_data_reset_payload_drain_candidate_until(deadline)
        .unwrap()
        .unwrap();

    set_test_app_data_reset_snapshot_payload_drain_fault(
        TestAppDataResetSnapshotPayloadDrainFault::ForeignFilesystemAtFinalGate,
    );
    assert_eq!(
        lease
            .drain_one_app_data_reset_payload(
                candidate,
                AppDataResetSnapshotPayloadDrainAuthority::for_test(deadline),
            )
            .unwrap_err(),
        AppDataResetSnapshotPayloadDrainError::BeforeEffect(SnapshotStorageErrorKind::UnsafeObject)
    );
    assert!(path.exists());

    let retry_deadline = reset_payload_test_deadline();
    let retry = lease
        .app_data_reset_payload_drain_candidate_until(retry_deadline)
        .unwrap()
        .unwrap();
    lease
        .drain_one_app_data_reset_payload(
            retry,
            AppDataResetSnapshotPayloadDrainAuthority::for_test(retry_deadline),
        )
        .unwrap();
    assert!(!path.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_quiescent_temp_rechecks_filesystem_at_exact_effect_gate() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let name = abandon_test_snapshot_temp(&store, b"reset-final-gate-temp");
    let path = database.parent().unwrap().join(DIRECTORY_NAME).join(&name);
    let mut lease = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let deadline = reset_payload_test_deadline();
    let candidate = lease
        .app_data_reset_payload_drain_candidate_until(deadline)
        .unwrap()
        .unwrap();

    set_test_app_data_reset_snapshot_payload_drain_fault(
        TestAppDataResetSnapshotPayloadDrainFault::ForeignFilesystemAtFinalGate,
    );
    assert_eq!(
        lease
            .drain_one_app_data_reset_payload(
                candidate,
                AppDataResetSnapshotPayloadDrainAuthority::for_test(deadline),
            )
            .unwrap_err(),
        AppDataResetSnapshotPayloadDrainError::BeforeEffect(SnapshotStorageErrorKind::UnsafeObject)
    );
    assert!(path.exists());

    let retry_deadline = reset_payload_test_deadline();
    let retry = lease
        .app_data_reset_payload_drain_candidate_until(retry_deadline)
        .unwrap()
        .unwrap();
    lease
        .drain_one_app_data_reset_payload(
            retry,
            AppDataResetSnapshotPayloadDrainAuthority::for_test(retry_deadline),
        )
        .unwrap();
    assert!(!path.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_locked_temp_override_requires_exact_observed_facts() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let name = abandon_test_snapshot_temp(&store, b"reset-temp-override-facts");
    let path = database.parent().unwrap().join(DIRECTORY_NAME).join(&name);
    let lease = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let selected = lease.entries().first().unwrap().clone();
    assert_eq!(selected.name, name);

    let mut wrong_name = selected.clone();
    wrong_name.name.push('x');
    let mut wrong_identity = selected.clone();
    wrong_identity.identity = Identity(platform::different_filesystem_identity_for_test(
        wrong_identity.identity.0,
    ));
    let mut wrong_usage = selected.clone();
    wrong_usage.usage = wrong_usage
        .usage
        .checked_add(SnapshotFileUsage::from_sizes(1, 1))
        .unwrap();
    let mut wrong_state = selected;
    wrong_state.temp_kernel_state = Some(SnapshotTempKernelState::Active);

    for altered in [wrong_name, wrong_identity, wrong_usage, wrong_state] {
        assert_eq!(
            lease
                .revalidate_complete_for_app_data_reset_before_effect_until(
                    &altered,
                    reset_payload_test_deadline(),
                )
                .unwrap_err()
                .kind(),
            SnapshotStorageErrorKind::UnsafeObject
        );
        assert!(path.exists());
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_locked_temp_override_still_probes_every_other_temp() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let first = abandon_test_snapshot_temp(&store, b"reset-temp-override-first");
    let second = abandon_test_snapshot_temp(&store, b"reset-temp-override-second");
    let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);
    let lease = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let selected = lease
        .entries()
        .iter()
        .min_by(|left, right| left.name.cmp(&right.name))
        .unwrap()
        .clone();
    let other_name = if selected.name == first {
        &second
    } else {
        &first
    };
    let other = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(snapshot_root.join(other_name))
        .unwrap();
    FileExt::try_lock(&other).unwrap();

    assert_eq!(
        lease
            .revalidate_complete_for_app_data_reset_before_effect_until(
                &selected,
                reset_payload_test_deadline(),
            )
            .unwrap_err()
            .kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );
    assert!(snapshot_root.join(&selected.name).exists());
    assert!(snapshot_root.join(other_name).exists());
    FileExt::unlock(&other).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_payload_candidate_rejects_cross_store_and_object_drift() {
    let first_temp = TempDir::new().unwrap();
    private_database_root(&first_temp);
    let first_database = database_path(&first_temp);
    let first_store = open_rw(&first_database);
    let changed_name = publish_test_snapshot(&first_store, b"reset-candidate-changed", b"before");
    let mut first_lease = first_store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let first_deadline = reset_payload_test_deadline();
    let changed_candidate = first_lease
        .app_data_reset_payload_drain_candidate_until(first_deadline)
        .unwrap()
        .unwrap();

    let second_temp = TempDir::new().unwrap();
    private_database_root(&second_temp);
    let second_store = open_rw(&database_path(&second_temp));
    publish_test_snapshot(&second_store, b"reset-candidate-other", b"other");
    let second_lease = second_store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    assert!(!changed_candidate.is_bound_to(&second_lease));
    assert_eq!(
        changed_candidate
            .revalidate_against_until(&second_lease, reset_payload_test_deadline())
            .unwrap_err()
            .kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );
    drop(second_lease);

    let changed_path = first_database
        .parent()
        .unwrap()
        .join(DIRECTORY_NAME)
        .join(changed_name.as_str());
    let mut changed = fs::OpenOptions::new()
        .append(true)
        .open(&changed_path)
        .unwrap();
    changed.write_all(b" changed").unwrap();
    changed.sync_all().unwrap();
    drop(changed);
    assert_eq!(
        first_lease
            .drain_one_app_data_reset_payload(
                changed_candidate,
                AppDataResetSnapshotPayloadDrainAuthority::for_test(first_deadline),
            )
            .unwrap_err(),
        AppDataResetSnapshotPayloadDrainError::BeforeEffect(SnapshotStorageErrorKind::UnsafeObject)
    );
    assert!(changed_path.exists());
    drop(first_lease);

    let replacement_temp = TempDir::new().unwrap();
    private_database_root(&replacement_temp);
    let replacement_database = database_path(&replacement_temp);
    let replacement_store = open_rw(&replacement_database);
    let replacement_name = publish_test_snapshot(
        &replacement_store,
        b"reset-candidate-replaced",
        b"same bytes",
    );
    let mut replacement_lease = replacement_store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let replacement_deadline = reset_payload_test_deadline();
    let replacement_candidate = replacement_lease
        .app_data_reset_payload_drain_candidate_until(replacement_deadline)
        .unwrap()
        .unwrap();
    replacement_store
        .replace_final_for_test(&replacement_name, b"same bytes")
        .unwrap();
    assert_eq!(
        replacement_lease
            .drain_one_app_data_reset_payload(
                replacement_candidate,
                AppDataResetSnapshotPayloadDrainAuthority::for_test(replacement_deadline),
            )
            .unwrap_err(),
        AppDataResetSnapshotPayloadDrainError::BeforeEffect(SnapshotStorageErrorKind::UnsafeObject)
    );
    assert!(
        replacement_database
            .parent()
            .unwrap()
            .join(DIRECTORY_NAME)
            .join(replacement_name.as_str())
            .exists()
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_payload_candidate_refuses_any_active_temp_before_effect() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let final_name = publish_test_snapshot(&store, b"reset-active-final", b"final");
    let active = store
        .stage(
            SnapshotFileName::from_scan_id(b"reset-active-temp"),
            Duration::from_millis(100),
        )
        .unwrap();
    let lease = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let error =
        match lease.app_data_reset_payload_drain_candidate_until(reset_payload_test_deadline()) {
            Err(error) => error,
            Ok(_) => panic!("an active temporary was admitted for reset draining"),
        };
    assert_eq!(error.kind(), SnapshotStorageErrorKind::Busy);
    assert!(
        database
            .parent()
            .unwrap()
            .join(DIRECTORY_NAME)
            .join(final_name.as_str())
            .exists()
    );
    drop(lease);
    active.abandon();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn app_data_reset_payload_drain_uncertainty_seams_are_restart_safe() {
    for (fault, before_effect_kind) in [
        (
            TestAppDataResetSnapshotPayloadDrainFault::BeforeEffect,
            Some(SnapshotStorageErrorKind::Unavailable),
        ),
        (
            TestAppDataResetSnapshotPayloadDrainFault::ExhaustPreEffectDeadline,
            Some(SnapshotStorageErrorKind::Busy),
        ),
        (
            TestAppDataResetSnapshotPayloadDrainFault::ExhaustDeadlineAtFinalGate,
            Some(SnapshotStorageErrorKind::Busy),
        ),
        (TestAppDataResetSnapshotPayloadDrainFault::AfterEffect, None),
        (
            TestAppDataResetSnapshotPayloadDrainFault::AfterDirectorySync,
            None,
        ),
        (
            TestAppDataResetSnapshotPayloadDrainFault::DuringReadback,
            None,
        ),
        (
            TestAppDataResetSnapshotPayloadDrainFault::ExhaustPostEffectDeadline,
            None,
        ),
    ] {
        let temp = TempDir::new().unwrap();
        private_database_root(&temp);
        let database = database_path(&temp);
        let store = open_rw(&database);
        let name = publish_test_snapshot(&store, b"reset-fault-final", b"payload");
        let path = database
            .parent()
            .unwrap()
            .join(DIRECTORY_NAME)
            .join(name.as_str());
        let mut lease = store
            .inventory_with_writer_lease(Duration::from_millis(100))
            .unwrap();
        let deadline = if matches!(
            fault,
            TestAppDataResetSnapshotPayloadDrainFault::ExhaustPreEffectDeadline
                | TestAppDataResetSnapshotPayloadDrainFault::ExhaustDeadlineAtFinalGate
        ) {
            Instant::now()
                .checked_add(Duration::from_millis(10))
                .unwrap()
        } else {
            reset_payload_test_deadline()
        };
        let candidate = lease
            .app_data_reset_payload_drain_candidate_until(deadline)
            .unwrap()
            .unwrap();
        set_test_app_data_reset_snapshot_payload_drain_fault(fault);
        let error = lease
            .drain_one_app_data_reset_payload(
                candidate,
                AppDataResetSnapshotPayloadDrainAuthority::for_test(deadline),
            )
            .unwrap_err();
        if let Some(kind) = before_effect_kind {
            assert_eq!(
                error,
                AppDataResetSnapshotPayloadDrainError::BeforeEffect(kind)
            );
            assert!(path.exists());
        } else {
            assert_eq!(error, AppDataResetSnapshotPayloadDrainError::OutcomeUnknown);
            assert!(!path.exists());
        }
        drop(lease);

        let mut restarted = store
            .inventory_with_writer_lease(Duration::from_millis(100))
            .unwrap();
        let restart_deadline = reset_payload_test_deadline();
        if before_effect_kind.is_some() {
            let candidate = restarted
                .app_data_reset_payload_drain_candidate_until(restart_deadline)
                .unwrap()
                .unwrap();
            let progress = restarted
                .drain_one_app_data_reset_payload(
                    candidate,
                    AppDataResetSnapshotPayloadDrainAuthority::for_test(restart_deadline),
                )
                .unwrap()
                .into_progress();
            assert_eq!(progress.removed_objects(), 1);
            assert!(!progress.snapshot_payload_has_more());
        } else {
            assert!(
                restarted
                    .app_data_reset_payload_drain_candidate_until(restart_deadline)
                    .unwrap()
                    .is_none()
            );
        }
        restarted.revalidate_complete().unwrap();
        assert!(!path.exists());
    }
}

#[test]
fn inventory_lease_rejects_changed_usage_and_replaced_final_identity() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let changed_name = SnapshotFileName::from_scan_id(b"retention-changed-final");
    let replacement_name = SnapshotFileName::from_scan_id(b"retention-replaced-final");
    for name in [&changed_name, &replacement_name] {
        let mut staged = store
            .stage((*name).clone(), Duration::from_millis(100))
            .unwrap();
        staged.write_all(b"fixed snapshot bytes").unwrap();
        drop(staged.publish_no_replace().unwrap());
    }

    let snapshot_root = database.parent().unwrap().join(DIRECTORY_NAME);
    let mut inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let changed_retained = inventory.retain_observed_final(&changed_name).unwrap();
    let replacement_retained = inventory.retain_observed_final(&replacement_name).unwrap();
    let mut changed = fs::OpenOptions::new()
        .append(true)
        .open(snapshot_root.join(changed_name.as_str()))
        .unwrap();
    changed.write_all(b"changed").unwrap();
    changed.sync_all().unwrap();
    drop(changed);
    match inventory
        .remove_observed_final_reconciled(&changed_retained)
        .unwrap_err()
    {
        SnapshotFinalRemovalError::BeforeEffect(error) => {
            assert_eq!(error.kind(), SnapshotStorageErrorKind::UnsafeObject);
        }
        SnapshotFinalRemovalError::OutcomeUnknown => {
            panic!("usage change must fail before physical removal")
        }
    }
    assert!(snapshot_root.join(changed_name.as_str()).exists());

    let (original, original_identity) = platform::open_named_final_for_removal(
        &store.inner.directory,
        &store.inner.path,
        replacement_name.as_str(),
    )
    .unwrap()
    .unwrap();
    platform::remove_retained_final(
        &store.inner.directory,
        replacement_name.as_str(),
        original,
        original_identity,
    )
    .unwrap();
    platform::sync_directory(&store.inner.directory).unwrap();
    let (mut replacement, _) = platform::create_private_file_exclusive(
        &store.inner.directory,
        &store.inner.path,
        replacement_name.as_str(),
    )
    .unwrap()
    .unwrap();
    replacement.write_all(b"fixed snapshot bytes").unwrap();
    replacement.sync_all().unwrap();
    drop(replacement);

    assert_eq!(
        inventory
            .remove_observed_final(&replacement_retained)
            .unwrap_err()
            .kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );
    assert_eq!(
        fs::read(snapshot_root.join(replacement_name.as_str())).unwrap(),
        b"fixed snapshot bytes"
    );
}

#[test]
fn staged_temp_lock_transitions_from_active_to_quiescent_on_close_only_drop() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let store = open_rw(&database_path(&temp));
    let staged = store
        .stage(
            SnapshotFileName::from_scan_id(b"temp-kernel-transition"),
            Duration::from_millis(100),
        )
        .unwrap();
    let temp_name = staged.temp_name.clone();
    let active = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    assert_eq!(
        active
            .entries()
            .iter()
            .find(|entry| entry.name() == temp_name)
            .unwrap()
            .temp_kernel_state(),
        Some(SnapshotTempKernelState::Active)
    );
    drop(active);
    staged.abandon();
    let quiescent = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    assert_eq!(
        quiescent
            .entries()
            .iter()
            .find(|entry| entry.name() == temp_name)
            .unwrap()
            .temp_kernel_state(),
        Some(SnapshotTempKernelState::Quiescent)
    );
}

#[test]
#[allow(clippy::disallowed_methods)]
fn inventory_closes_entry_handles_and_rejects_final_usage_change() {
    const ROLE: &str = "DUX_SNAPSHOT_INVENTORY_FD_CHILD";
    const DATABASE: &str = "DUX_SNAPSHOT_INVENTORY_FD_DATABASE";
    if std::env::var_os(ROLE).is_some() {
        let database = PathBuf::from(std::env::var_os(DATABASE).unwrap());
        let limit = nix::libc::rlimit {
            rlim_cur: 128,
            rlim_max: 128,
        };
        // SAFETY: this exact-test child lowers only its own descriptor
        // limit and exits immediately after the bounded inventory probe.
        assert_eq!(
            unsafe { nix::libc::setrlimit(nix::libc::RLIMIT_NOFILE, &limit) },
            0
        );
        let store = open_rw(&database);
        let inventory = store
            .inventory_with_writer_lease(Duration::from_millis(250))
            .unwrap();
        assert_eq!(inventory.entries().len(), 300);
        inventory.revalidate().unwrap();
        return;
    }

    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    for ordinal in 0..300_u32 {
        let name = SnapshotFileName::from_scan_id(format!("fd-bound-{ordinal}").as_bytes());
        let (file, _) = platform::create_private_file_exclusive(
            &store.inner.directory,
            &store.inner.path,
            name.as_str(),
        )
        .unwrap()
        .unwrap();
        drop(file);
    }
    drop(store);
    let executable = std::env::current_exe().unwrap();
    // DUX-DESTRUCTIVE: allow=test-snapshot-inventory-fd-helper-spawn -- relaunch only this exact test against its TempDir-owned store so the child can lower its own descriptor limit without racing the parent harness
    let status = Command::new(executable)
        .arg("--exact")
        .arg(
            "persistence::snapshot::storage::tests::inventory_closes_entry_handles_and_rejects_final_usage_change",
        )
        .arg("--nocapture")
        .env(ROLE, "1")
        .env(DATABASE, &database)
        .status()
        .unwrap();
    assert!(status.success());

    let store = open_rw(&database);
    let inventory = store
        .inventory_with_writer_lease(Duration::from_millis(250))
        .unwrap();
    assert_eq!(inventory.entries().len(), 300);
    let first = inventory
        .entries()
        .iter()
        .find_map(|entry| match entry.kind() {
            SnapshotInventoryEntryKind::Final(name) => Some(name.clone()),
            SnapshotInventoryEntryKind::RecognizedTemp => None,
        })
        .unwrap();
    let mut external_writer = fs::OpenOptions::new()
        .append(true)
        .open(
            database
                .parent()
                .unwrap()
                .join(DIRECTORY_NAME)
                .join(first.as_str()),
        )
        .unwrap();
    external_writer.write_all(b"changed").unwrap();
    external_writer.sync_all().unwrap();
    drop(external_writer);
    assert_eq!(
        inventory.revalidate().unwrap_err().kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );
}

#[test]
fn stage_rejects_the_sixty_fifth_temp_without_creating_it() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let store = open_rw(&database_path(&temp));
    for ordinal in 0..MAX_RECOGNIZED_TEMPS {
        let final_name =
            SnapshotFileName::from_scan_id(format!("temp-population-{ordinal}").as_bytes());
        let temp_name = random_temp_name(&final_name).unwrap();
        let (file, _) = platform::create_private_file_exclusive(
            &store.inner.directory,
            &store.inner.path,
            &temp_name,
        )
        .unwrap()
        .unwrap();
        drop(file);
    }
    let before = store
        .inventory_with_writer_lease(Duration::from_millis(250))
        .unwrap();
    assert_eq!(
        before
            .entries()
            .iter()
            .filter(|entry| matches!(entry.kind(), SnapshotInventoryEntryKind::RecognizedTemp))
            .count(),
        MAX_RECOGNIZED_TEMPS
    );
    drop(before);
    assert_eq!(
        store
            .stage(
                SnapshotFileName::from_scan_id(b"sixty-fifth-temp"),
                Duration::from_millis(250),
            )
            .err()
            .unwrap()
            .kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );
    let after = store
        .inventory_with_writer_lease(Duration::from_millis(250))
        .unwrap();
    assert_eq!(after.entries().len(), MAX_RECOGNIZED_TEMPS);
}

#[test]
#[allow(clippy::disallowed_methods)]
fn inventory_rejects_final_larger_than_codec_limit_but_not_temp_by_policy() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let store = open_rw(&database_path(&temp));
    let temp_name = random_temp_name(&SnapshotFileName::from_scan_id(b"oversized-temp")).unwrap();
    let (temp_file, _) = platform::create_private_file_exclusive(
        &store.inner.directory,
        &store.inner.path,
        &temp_name,
    )
    .unwrap()
    .unwrap();
    // DUX-DESTRUCTIVE: allow=test-snapshot-inventory-oversized-temp -- create a sparse oversized file only inside this TempDir-owned private snapshot fixture to prove temps are observed but not policy-classified
    temp_file.set_len(MAX_SNAPSHOT_FILE_BYTES + 1).unwrap();
    temp_file.sync_all().unwrap();
    drop(temp_file);
    let inventory = store
        .inventory_with_writer_lease(Duration::from_millis(100))
        .unwrap();
    let observed_temp = inventory
        .entries()
        .iter()
        .find(|entry| entry.name() == temp_name)
        .unwrap();
    assert_eq!(
        observed_temp.kind(),
        &SnapshotInventoryEntryKind::RecognizedTemp
    );
    assert_eq!(
        observed_temp.usage().logical_bytes(),
        MAX_SNAPSHOT_FILE_BYTES + 1
    );
    drop(inventory);

    let final_name = SnapshotFileName::from_scan_id(b"oversized-final");
    let (file, _) = platform::create_private_file_exclusive(
        &store.inner.directory,
        &store.inner.path,
        final_name.as_str(),
    )
    .unwrap()
    .unwrap();
    // DUX-DESTRUCTIVE: allow=test-snapshot-inventory-oversized-final -- create a sparse oversized file only inside this TempDir-owned private snapshot fixture to prove final inventory fails closed
    file.set_len(MAX_SNAPSHOT_FILE_BYTES + 1).unwrap();
    file.sync_all().unwrap();
    drop(file);

    assert_eq!(
        store
            .inventory_with_writer_lease(Duration::from_millis(100))
            .err()
            .unwrap()
            .kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );
}

#[test]
fn inventory_usage_totals_fail_closed_on_overflow() {
    let maximum = SnapshotFileUsage::from_sizes(u64::MAX, u64::MAX);
    assert_eq!(
        maximum
            .checked_add(SnapshotFileUsage::from_sizes(1, 0))
            .err()
            .unwrap()
            .kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );
}

#[test]
fn refuses_unmarked_existing_snapshot_directory_without_populating_it() {
    let temp = TempDir::new().unwrap();
    let root = private_database_root(&temp);
    let snapshots = root.join(DIRECTORY_NAME);
    fs::create_dir(&snapshots).unwrap();
    fs::set_permissions(&snapshots, fs::Permissions::from_mode(0o700)).unwrap();

    let error = SecureSnapshotStore::open_for_database(
        &root.join("dux.sqlite3"),
        SnapshotStoreAccess::ReadWrite,
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), SnapshotStorageErrorKind::UnrecognizedStore);
    assert!(fs::read_dir(snapshots).unwrap().next().is_none());
}

#[test]
fn exact_id_collision_never_replaces_and_returns_existing_bytes() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let store = open_rw(&database_path(&temp));
    let name = SnapshotFileName::from_scan_id(b"same scan");

    let mut first = store
        .stage(name.clone(), Duration::from_millis(100))
        .unwrap();
    first.write_all(b"first").unwrap();
    assert!(matches!(
        first.publish_no_replace().unwrap(),
        SnapshotPublication::Published(_)
    ));

    let mut second = store
        .stage(name.clone(), Duration::from_millis(100))
        .unwrap();
    second.write_all(b"second must not replace").unwrap();
    let SnapshotPublication::Existing(existing) = second.publish_no_replace().unwrap() else {
        panic!("exact ID collision must return the immutable winner");
    };
    let mut bytes = Vec::new();
    existing
        .try_clone_file()
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"first");
    drop(existing);
    assert!(store.open(&name).unwrap().is_some());
}

#[test]
fn writers_stage_unique_temps_without_holding_the_publication_lock() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let store = open_rw(&database_path(&temp));
    let name = SnapshotFileName::from_scan_id(b"lock");
    let first_name = random_temp_name(&name).unwrap();
    let second_name = random_temp_name(&name).unwrap();
    assert_ne!(first_name, second_name);

    let first = store
        .stage(name.clone(), Duration::from_millis(100))
        .unwrap();
    let second = store.stage(name, Duration::from_millis(100)).unwrap();
    assert_ne!(first.temp_name, second.temp_name);
    first.abort().unwrap();
    second.abort().unwrap();

    let lock = store
        .acquire_writer_lock(Duration::from_millis(100))
        .unwrap();
    let error = store
        .stage(
            SnapshotFileName::from_scan_id(b"busy"),
            Duration::from_millis(1),
        )
        .err()
        .unwrap();
    assert_eq!(error.kind(), SnapshotStorageErrorKind::Busy);
    drop(lock);
}

#[test]
fn uncertain_writer_unlock_never_advertises_same_process_availability() {
    let in_use = AtomicBool::new(true);
    record_writer_unlock(&in_use, false);
    assert!(in_use.load(Ordering::Acquire));
    record_writer_unlock(&in_use, true);
    assert!(!in_use.load(Ordering::Acquire));
}

#[test]
#[allow(clippy::disallowed_methods)]
fn restrictive_umask_still_provisions_exact_private_modes() {
    const ROLE: &str = "DUX_SNAPSHOT_UMASK_CHILD";
    const DATABASE: &str = "DUX_SNAPSHOT_UMASK_DATABASE";
    if std::env::var_os(ROLE).is_some() {
        let _previous = nix::sys::stat::umask(nix::sys::stat::Mode::from_bits_truncate(0o777));
        let database = PathBuf::from(std::env::var_os(DATABASE).unwrap());
        let store = open_rw(&database);
        let name = SnapshotFileName::from_scan_id(b"restrictive-umask");
        let mut stage = store
            .stage(name.clone(), Duration::from_millis(100))
            .unwrap();
        stage.write_all(b"private").unwrap();
        drop(stage.publish_no_replace().unwrap());
        let snapshots = database.parent().unwrap().join(DIRECTORY_NAME);
        assert_eq!(
            fs::metadata(&snapshots).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        for path in [
            snapshots.join(MARKER_NAME),
            snapshots.join(WRITER_LOCK_NAME),
            snapshots.join(name.as_str()),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o7777,
                0o600
            );
        }
        return;
    }

    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    // DUX-DESTRUCTIVE: allow=test-snapshot-umask-helper-spawn -- relaunch only this exact unit test so a process-global restrictive umask cannot race unrelated tests
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("persistence::snapshot::storage::tests::restrictive_umask_still_provisions_exact_private_modes")
        .arg("--nocapture")
        .env(ROLE, "1")
        .env(DATABASE, database)
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(target_os = "macos")]
#[test]
#[allow(clippy::disallowed_methods)]
fn macos_accepts_deny_only_parent_acl_and_rejects_final_object_acl() {
    let temp = TempDir::new().unwrap();
    // DUX-DESTRUCTIVE: allow=test-snapshot-macos-parent-acl-command -- invoke fixed system chmod only on this test-owned publication parent to prove deny-only ACL admission
    let parent_acl = Command::new("/bin/chmod")
        .args(["+a", "everyone deny delete"])
        .arg(temp.path())
        .status()
        .unwrap();
    assert!(parent_acl.success());
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    drop(store);

    let snapshots = database.parent().unwrap().join(DIRECTORY_NAME);
    // DUX-DESTRUCTIVE: allow=test-snapshot-macos-final-acl-command -- invoke fixed system chmod only on this test-owned snapshot root to prove final-object granting ACL rejection
    let final_acl = Command::new("/bin/chmod")
        .args(["+a", "everyone allow read"])
        .arg(&snapshots)
        .status()
        .unwrap();
    assert!(final_acl.success());
    assert_eq!(
        SecureSnapshotStore::open_for_database(&database, SnapshotStoreAccess::ReadWrite)
            .err()
            .unwrap()
            .kind(),
        SnapshotStorageErrorKind::UnsafeRoot
    );
}

#[test]
fn recognized_crash_temp_is_validated_but_not_scavenged() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let name = SnapshotFileName::from_scan_id(b"crash");
    let temp_name = random_temp_name(&name).unwrap();
    let (file, _) = platform::create_private_file_exclusive(
        &store.inner.directory,
        &store.inner.path,
        &temp_name,
    )
    .unwrap()
    .unwrap();
    file.sync_all().unwrap();
    drop(file);
    drop(store);

    let reopened = open_rw(&database);
    reopened.validate_inventory(None).unwrap();
    assert!(
        database
            .parent()
            .unwrap()
            .join(DIRECTORY_NAME)
            .join(temp_name)
            .exists()
    );
}

#[test]
fn dropped_unfenced_stage_is_close_only_recognized_debt() {
    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let stage = store
        .stage(
            SnapshotFileName::from_scan_id(b"unwind-debt"),
            Duration::from_millis(100),
        )
        .unwrap();
    let temp_name = stage.temp_name.clone();
    drop(stage);

    let path = database
        .parent()
        .unwrap()
        .join(DIRECTORY_NAME)
        .join(temp_name);
    assert!(path.exists());
    store.validate_inventory(None).unwrap();
}

#[test]
fn symlink_hard_link_fifo_and_broad_permissions_fail_closed() {
    use std::os::unix::fs::symlink;

    let symlinked = TempDir::new().unwrap();
    let root = private_database_root(&symlinked);
    let outside = symlinked.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&outside, root.join(DIRECTORY_NAME)).unwrap();
    assert!(matches!(
        SecureSnapshotStore::open_for_database(
            &root.join("dux.sqlite3"),
            SnapshotStoreAccess::ReadWrite
        )
        .err()
        .unwrap()
        .kind(),
        SnapshotStorageErrorKind::UnsafeRoot | SnapshotStorageErrorKind::Unavailable
    ));

    let linked = TempDir::new().unwrap();
    private_database_root(&linked);
    let database = database_path(&linked);
    drop(open_rw(&database));
    let snapshots = database.parent().unwrap().join(DIRECTORY_NAME);
    fs::hard_link(
        snapshots.join(MARKER_NAME),
        linked.path().join("marker-alias"),
    )
    .unwrap();
    assert_eq!(
        SecureSnapshotStore::open_for_database(&database, SnapshotStoreAccess::ReadWrite)
            .err()
            .unwrap()
            .kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );

    let special = TempDir::new().unwrap();
    private_database_root(&special);
    let database = database_path(&special);
    let store = open_rw(&database);
    let name = SnapshotFileName::from_scan_id(b"special");
    nix::unistd::mkfifo(
        &database
            .parent()
            .unwrap()
            .join(DIRECTORY_NAME)
            .join(name.as_str()),
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    assert_eq!(
        store.open(&name).err().unwrap().kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );

    let broad = TempDir::new().unwrap();
    private_database_root(&broad);
    let database = database_path(&broad);
    let store = open_rw(&database);
    let name = SnapshotFileName::from_scan_id(b"broad");
    let mut stage = store
        .stage(name.clone(), Duration::from_millis(100))
        .unwrap();
    stage.write_all(b"private").unwrap();
    drop(stage.publish_no_replace().unwrap());
    fs::set_permissions(
        database
            .parent()
            .unwrap()
            .join(DIRECTORY_NAME)
            .join(name.as_str()),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(
        store.open(&name).err().unwrap().kind(),
        SnapshotStorageErrorKind::UnsafeObject
    );
}

#[test]
#[allow(clippy::disallowed_methods)]
fn cross_process_temp_lock_proves_active_then_quiescent() {
    const ROLE: &str = "DUX_SNAPSHOT_TEMP_LOCK_CHILD";
    const DATABASE: &str = "DUX_SNAPSHOT_TEMP_LOCK_DATABASE";
    const READY: &str = "DUX_SNAPSHOT_TEMP_LOCK_READY";
    const RELEASE: &str = "DUX_SNAPSHOT_TEMP_LOCK_RELEASE";

    if std::env::var_os(ROLE).is_some() {
        let database = PathBuf::from(std::env::var_os(DATABASE).unwrap());
        let ready = PathBuf::from(std::env::var_os(READY).unwrap());
        let release = PathBuf::from(std::env::var_os(RELEASE).unwrap());
        let store = open_rw(&database);
        let mut staged = store
            .stage(
                SnapshotFileName::from_scan_id(b"cross-process-temp-lock"),
                Duration::from_secs(1),
            )
            .unwrap();
        staged.write_all(b"locked").unwrap();
        staged.sync_all().unwrap();
        fs::write(ready, b"ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !release.exists() {
            assert!(Instant::now() < deadline, "parent did not release child");
            std::thread::sleep(Duration::from_millis(5));
        }
        staged.abandon();
        return;
    }

    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let ready = temp.path().join("temp-lock-ready");
    let release = temp.path().join("temp-lock-release");
    // DUX-DESTRUCTIVE: allow=test-snapshot-temp-lock-helper-spawn -- relaunch only this exact unit test against its TempDir-owned private store to prove the kernel lease across a real process boundary
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(
            "persistence::snapshot::storage::tests::cross_process_temp_lock_proves_active_then_quiescent",
        )
        .arg("--nocapture")
        .env(ROLE, "1")
        .env(DATABASE, &database)
        .env(READY, &ready)
        .env(RELEASE, &release)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "child did not lock temp");
        std::thread::sleep(Duration::from_millis(5));
    }
    let active = store
        .inventory_with_writer_lease(Duration::from_millis(250))
        .unwrap();
    let temp_name = active
        .entries()
        .iter()
        .find(|entry| matches!(entry.kind(), SnapshotInventoryEntryKind::RecognizedTemp))
        .unwrap()
        .name()
        .to_owned();
    assert_eq!(
        active
            .entries()
            .iter()
            .find(|entry| entry.name() == temp_name)
            .unwrap()
            .temp_kernel_state(),
        Some(SnapshotTempKernelState::Active)
    );
    drop(active);
    fs::write(&release, b"release").unwrap();
    assert!(child.wait().unwrap().success());
    let quiescent = store
        .inventory_with_writer_lease(Duration::from_millis(250))
        .unwrap();
    assert_eq!(
        quiescent
            .entries()
            .iter()
            .find(|entry| entry.name() == temp_name)
            .unwrap()
            .temp_kernel_state(),
        Some(SnapshotTempKernelState::Quiescent)
    );
}

#[test]
#[allow(clippy::disallowed_methods)]
fn cross_process_writer_contention_is_bounded() {
    const ROLE: &str = "DUX_SNAPSHOT_LOCK_CHILD";
    const DATABASE: &str = "DUX_SNAPSHOT_LOCK_DATABASE";
    const READY: &str = "DUX_SNAPSHOT_LOCK_READY";
    const RELEASE: &str = "DUX_SNAPSHOT_LOCK_RELEASE";

    if std::env::var_os(ROLE).is_some() {
        let database = PathBuf::from(std::env::var_os(DATABASE).unwrap());
        let ready = PathBuf::from(std::env::var_os(READY).unwrap());
        let release = PathBuf::from(std::env::var_os(RELEASE).unwrap());
        let store = open_rw(&database);
        let lock = store.acquire_writer_lock(Duration::from_secs(1)).unwrap();
        fs::write(ready, b"ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !release.exists() {
            assert!(Instant::now() < deadline, "parent did not release child");
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(lock);
        return;
    }

    let temp = TempDir::new().unwrap();
    private_database_root(&temp);
    let database = database_path(&temp);
    let store = open_rw(&database);
    let ready = temp.path().join("ready");
    let release = temp.path().join("release");
    // DUX-DESTRUCTIVE: allow=test-snapshot-lock-helper-spawn -- relaunch only this exact unit-test executable with a fixed exact-test filter to prove cross-process lock exclusion
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("persistence::snapshot::storage::tests::cross_process_writer_contention_is_bounded")
        .arg("--nocapture")
        .env(ROLE, "1")
        .env(DATABASE, &database)
        .env(READY, &ready)
        .env(RELEASE, &release)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "child did not acquire writer lock"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(
        store
            .stage(
                SnapshotFileName::from_scan_id(b"cross-process"),
                Duration::from_millis(20),
            )
            .err()
            .unwrap()
            .kind(),
        SnapshotStorageErrorKind::Busy
    );
    fs::write(&release, b"release").unwrap();
    assert!(child.wait().unwrap().success());
    store
        .stage(
            SnapshotFileName::from_scan_id(b"after-release"),
            Duration::from_millis(100),
        )
        .unwrap()
        .abort()
        .unwrap();
}
