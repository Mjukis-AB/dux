use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use super::*;
use crate::cache::CACHE_VERSION;

fn container(temp: &TempDir) -> PathBuf {
    temp.path().join("Dux")
}

fn open_rw(path: &Path) -> ManagedCacheStore {
    ManagedCacheStore::open(path, ManagedCacheStoreAccess::ReadWrite)
        .unwrap()
        .unwrap()
}

fn reset_transaction() -> AppDataResetTransaction {
    AppDataResetTransaction::for_test("00112233445566778899aabbccddeeff").unwrap()
}

fn store_identity(path: &Path) -> (u64, u64) {
    let metadata = fs::metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}

fn cache_fixture(root: &Path) -> (PathBuf, CachedScanConfig, CacheMetadata, DiskTree) {
    fs::create_dir(root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let config = CachedScanConfig {
        follow_symlinks: false,
        same_filesystem: true,
        max_depth: None,
    };
    let tree = DiskTree::new(root.clone());
    let timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
    let metadata = CacheMetadata {
        version: CACHE_VERSION,
        root_path: root.clone(),
        scan_time: timestamp,
        root_mtime: timestamp,
        total_size: tree.total_size(),
        node_count: tree.len(),
        config: config.clone(),
    };
    (root, config, metadata, tree)
}

fn detached_payload_names(stage: &Path) -> Vec<String> {
    let mut names = fs::read_dir(stage)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != MARKER_NAME && name != WRITER_LOCK_NAME)
        .collect::<Vec<_>>();
    names.sort_unstable();
    names
}

fn detach_cache_for_drain(
    path: &Path,
    identity: (u64, u64),
    transaction: &AppDataResetTransaction,
) {
    ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        path,
        Some(identity),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| {
            let detached = admission.detach_if_canonical().unwrap();
            assert_eq!(
                detached.location(),
                AppDataResetManagedCacheRecoveryLocation::Detached
            );
            detached.revalidate().unwrap();
        },
    )
    .unwrap();
}

fn drain_cache_once(
    path: &Path,
    identity: Option<(u64, u64)>,
    transaction: &AppDataResetTransaction,
) -> std::result::Result<AppDataResetManagedCacheDrainBatch, AppDataResetManagedCacheDrainError> {
    ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        path,
        identity,
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| {
            let candidate = admission
                .into_drain_candidate(identity, transaction.cache_stage())
                .unwrap();
            assert!(candidate.is_bound_to(identity, transaction.cache_stage()));
            candidate.revalidate().unwrap();
            candidate.drain_one_detached_payload(AppDataResetCacheDrainAuthority::for_test())
        },
    )
    .unwrap()
}

fn draining_admission_kind(
    path: &Path,
    identity: Option<(u64, u64)>,
    transaction: &AppDataResetTransaction,
) -> std::result::Result<
    (bool, Option<AppDataResetManagedCacheStageRetirementState>),
    ManagedCacheStoreErrorKind,
> {
    match ManagedCacheStore::with_app_data_reset_draining_cache_admission_until(
        path,
        identity,
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| match admission {
            AppDataResetManagedCacheDrainingAdmission::PayloadsRemain(candidate) => {
                assert!(candidate.is_bound_to(identity, transaction.cache_stage()));
                candidate.revalidate().unwrap();
                (true, None)
            }
            AppDataResetManagedCacheDrainingAdmission::Retirement(candidate) => {
                assert!(candidate.is_bound_to(identity, transaction.cache_stage()));
                candidate.revalidate().unwrap();
                (false, Some(candidate.state()))
            }
            AppDataResetManagedCacheDrainingAdmission::Absent(candidate) => {
                assert!(candidate.is_bound_to(identity, transaction.cache_stage()));
                candidate.revalidate().unwrap();
                (
                    false,
                    Some(AppDataResetManagedCacheStageRetirementState::Absent),
                )
            }
        },
    ) {
        Ok(kind) => Ok(kind),
        Err(error) => Err(error.kind()),
    }
}

fn retire_cache_structure_once(
    path: &Path,
    identity: Option<(u64, u64)>,
    transaction: &AppDataResetTransaction,
) -> std::result::Result<
    AppDataResetManagedCacheStageRetirementBatch,
    AppDataResetManagedCacheStageRetirementError,
> {
    ManagedCacheStore::with_app_data_reset_draining_cache_admission_until(
        path,
        identity,
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| match admission {
            AppDataResetManagedCacheDrainingAdmission::PayloadsRemain(_) => {
                panic!("payload admission was mistaken for structural retirement")
            }
            AppDataResetManagedCacheDrainingAdmission::Retirement(candidate) => {
                candidate.retire_one_structure(AppDataResetCacheStageRetireAuthority::for_test())
            }
            AppDataResetManagedCacheDrainingAdmission::Absent(_) => {
                panic!("exact cache absence is not a structural retirement effect")
            }
        },
    )
    .unwrap()
}

fn assert_cache_absent_witness(
    path: &Path,
    identity: Option<(u64, u64)>,
    transaction: &AppDataResetTransaction,
) {
    ManagedCacheStore::with_app_data_reset_draining_cache_admission_until(
        path,
        identity,
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| match admission {
            AppDataResetManagedCacheDrainingAdmission::Absent(candidate) => {
                assert!(candidate.is_bound_to(identity, transaction.cache_stage()));
                candidate.revalidate().unwrap();
            }
            AppDataResetManagedCacheDrainingAdmission::PayloadsRemain(_)
            | AppDataResetManagedCacheDrainingAdmission::Retirement(_) => {
                panic!("retained cache debt was mistaken for exact absence")
            }
        },
    )
    .unwrap();
}

#[test]
fn absent_witness_uses_the_fresh_post_effect_deadline() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let transaction = reset_transaction();
    let admission_deadline = Instant::now() + Duration::from_millis(50);

    ManagedCacheStore::with_app_data_reset_draining_cache_admission_until(
        &path,
        None,
        transaction.cache_stage(),
        admission_deadline,
        |admission| {
            let AppDataResetManagedCacheDrainingAdmission::Absent(candidate) = admission else {
                panic!("missing cache was not admitted as exact absence")
            };
            while Instant::now() < admission_deadline {
                std::thread::yield_now();
            }
            let post_effect_deadline = Instant::now() + Duration::from_millis(250);
            assert_eq!(
                candidate
                    .revalidate_until(post_effect_deadline)
                    .unwrap_err()
                    .kind(),
                ManagedCacheStoreErrorKind::Busy
            );
            candidate
                .revalidate_after_effect_until(post_effect_deadline)
                .expect("post-effect absence must not be clipped to admission time");
        },
    )
    .unwrap();
}

fn remove_test_stage_file(stage: &Path, name: &str) {
    let directory = File::open(stage).unwrap();
    let (file, identity) =
        platform::open_named_private_file(&directory, stage, name, name == WRITER_LOCK_NAME)
            .unwrap()
            .unwrap();
    platform::remove_retained_file(&directory, name, file, identity).unwrap();
    platform::sync_directory(&directory).unwrap();
}

#[test]
fn configured_container_is_exact_absolute_dux_component() {
    let relative = Path::new("Dux");
    let Err(relative_error) = ManagedCacheStore::open(relative, ManagedCacheStoreAccess::ReadOnly)
    else {
        panic!("relative container was accepted");
    };
    assert_eq!(
        relative_error.kind(),
        ManagedCacheStoreErrorKind::InvalidConfiguration
    );

    let temp = TempDir::new().unwrap();
    let wrong_case = temp.path().join("dux");
    let Err(case_error) = ManagedCacheStore::open(&wrong_case, ManagedCacheStoreAccess::ReadOnly)
    else {
        panic!("wrong-case container was accepted");
    };
    assert_eq!(
        case_error.kind(),
        ManagedCacheStoreErrorKind::InvalidConfiguration
    );
}

#[test]
fn read_only_missing_store_creates_nothing() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    assert!(
        ManagedCacheStore::open(&path, ManagedCacheStoreAccess::ReadOnly)
            .unwrap()
            .is_none()
    );
    assert!(!path.exists());
}

#[test]
fn provisioning_owns_only_fixed_private_child_and_preserves_legacy_siblings() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let legacy = path.join("legacy-cache.dux");
    fs::write(&legacy, vec![7_u8; 1024 * 1024]).unwrap();

    let store = open_rw(&path);
    let owned = path.join(STORE_DIRECTORY_NAME);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o755);
    assert_eq!(fs::metadata(&owned).unwrap().mode() & 0o777, 0o700);
    assert_eq!(
        fs::metadata(owned.join(MARKER_NAME)).unwrap().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(owned.join(WRITER_LOCK_NAME)).unwrap().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read(owned.join(MARKER_NAME)).unwrap(), STORE_MARKER);
    assert_eq!(
        fs::read(owned.join(WRITER_LOCK_NAME)).unwrap(),
        WRITER_MARKER
    );
    assert_eq!(fs::read(&legacy).unwrap(), vec![7_u8; 1024 * 1024]);
    assert!(store.footprint().unwrap().total.charged_bytes < 1024 * 1024);
}

#[test]
fn existing_unmarked_child_is_never_adopted() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let child = path.join(STORE_DIRECTORY_NAME);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(&child).unwrap();
    fs::set_permissions(&child, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(child.join("user-data"), b"untouched").unwrap();

    let Err(error) = ManagedCacheStore::open(&path, ManagedCacheStoreAccess::ReadWrite) else {
        panic!("unmarked child was adopted");
    };
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::UnrecognizedStore);
    assert!(!child.join(MARKER_NAME).exists());
    assert_eq!(fs::read(child.join("user-data")).unwrap(), b"untouched");
}

#[test]
fn unsafe_outer_permissions_are_rejected_without_repair() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();

    let Err(error) = ManagedCacheStore::open(&path, ManagedCacheStoreAccess::ReadWrite) else {
        panic!("group/world-writable container was accepted");
    };
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::UnsafeContainer);
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o777);
    assert!(!path.join(STORE_DIRECTORY_NAME).exists());
}

#[test]
fn present_store_on_a_different_filesystem_is_never_detach_admitted() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let mounted_child_identity =
        platform::different_filesystem_identity(store.inner.directory_identity);

    let error = validate_same_filesystem(store.inner.container_identity, mounted_child_identity)
        .unwrap_err();
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::UnsafeStore);
}

#[test]
fn save_load_replace_and_clear_preserve_controls_and_outer_siblings() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let legacy = path.join("legacy");
    fs::write(&legacy, b"outside ownership").unwrap();
    let store = open_rw(&path);
    let (root, config, metadata, tree) = cache_fixture(&temp.path().join("scan-root"));

    store.save(&root, &config, &metadata, &tree).unwrap();
    store.save(&root, &config, &metadata, &tree).unwrap();
    let loaded = store.load(&root, &config).unwrap().unwrap();
    let (loaded_metadata, loaded_tree) = loaded.into_parts();
    assert_eq!(loaded_metadata.root_path, root);
    assert_eq!(loaded_tree.root_path(), root);

    let footprint = store.footprint().unwrap();
    assert_eq!(footprint.entry_count, 1);
    assert_eq!(footprint.temporary_count, 0);
    assert!(footprint.entries.logical_bytes >= MANAGED_CACHE_HEADER_BYTES as u64);
    assert_eq!(
        footprint.total,
        footprint.controls.checked_add(footprint.entries).unwrap()
    );
    let snapshot = store.prepare_clear().unwrap().unwrap();
    assert_eq!(snapshot.footprint(), footprint);
    let result = store.clear(snapshot).unwrap();
    assert_eq!(result.cleared_entries, 1);
    assert_eq!(result.cleared_temporary, 0);
    assert_eq!(result.cleared_usage, footprint.entries);
    assert!(path.join(STORE_DIRECTORY_NAME).join(MARKER_NAME).exists());
    assert!(
        path.join(STORE_DIRECTORY_NAME)
            .join(WRITER_LOCK_NAME)
            .exists()
    );
    assert_eq!(fs::read(legacy).unwrap(), b"outside ownership");
    assert_eq!(store.footprint().unwrap().entry_count, 0);
}

#[test]
fn clear_requires_an_exact_unchanged_inventory() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let first = cache_fixture(&temp.path().join("first-root"));
    let second = cache_fixture(&temp.path().join("second-root"));
    store.save(&first.0, &first.1, &first.2, &first.3).unwrap();
    let snapshot = store.prepare_clear().unwrap().unwrap();
    store
        .save(&second.0, &second.1, &second.2, &second.3)
        .unwrap();

    let Err(ManagedCacheClearError::BeforeEffect(error)) = store.clear(snapshot) else {
        panic!("changed inventory was cleared");
    };
    assert_eq!(
        error.kind(),
        ManagedCacheStoreErrorKind::ChangedSinceSnapshot
    );
    assert_eq!(store.footprint().unwrap().entry_count, 2);
}

#[test]
fn unknown_symlink_and_hard_link_objects_fail_closed() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let owned = path.join(STORE_DIRECTORY_NAME);
    fs::write(owned.join("unknown"), b"user data").unwrap();
    let error = store.footprint().unwrap_err();
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::UnsafeObject);

    let other = TempDir::new().unwrap();
    let symlink_path = container(&other);
    let symlink_store = open_rw(&symlink_path);
    let external = other.path().join("external");
    fs::write(&external, b"untouched").unwrap();
    let fake_final = format!("{}.dux", "0".repeat(KEY_HEX_BYTES));
    symlink(
        &external,
        symlink_path.join(STORE_DIRECTORY_NAME).join(fake_final),
    )
    .unwrap();
    assert_eq!(
        symlink_store.footprint().unwrap_err().kind(),
        ManagedCacheStoreErrorKind::UnsafeObject
    );
    assert_eq!(fs::read(external).unwrap(), b"untouched");

    let linked = TempDir::new().unwrap();
    let linked_path = container(&linked);
    let linked_store = open_rw(&linked_path);
    let fixture = cache_fixture(&linked.path().join("linked-root"));
    linked_store
        .save(&fixture.0, &fixture.1, &fixture.2, &fixture.3)
        .unwrap();
    let key = managed_cache_entry_key(&fixture.0, &fixture.1).unwrap();
    let original = linked_path
        .join(STORE_DIRECTORY_NAME)
        .join(final_name(&key));
    let second_final = linked_path
        .join(STORE_DIRECTORY_NAME)
        .join(format!("{}.dux", "f".repeat(KEY_HEX_BYTES)));
    fs::hard_link(&original, &second_final).unwrap();
    assert_eq!(fs::metadata(&original).unwrap().nlink(), 2);
    assert_eq!(
        linked_store.footprint().unwrap_err().kind(),
        ManagedCacheStoreErrorKind::UnsafeObject
    );
}

#[test]
fn writer_lock_is_exclusive_and_read_only_store_cannot_mutate() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let independent = open_rw(&path);
    let held = store.acquire_writer_lock(Duration::ZERO).unwrap();
    assert_eq!(
        store.footprint().unwrap_err().kind(),
        ManagedCacheStoreErrorKind::Busy
    );
    let Err(independent_error) = independent.acquire_writer_lock(Duration::ZERO) else {
        panic!("independent store acquired an already-held writer lock");
    };
    assert_eq!(independent_error.kind(), ManagedCacheStoreErrorKind::Busy);
    drop(held);

    let read_only = ManagedCacheStore::open(&path, ManagedCacheStoreAccess::ReadOnly)
        .unwrap()
        .unwrap();
    let fixture = cache_fixture(&temp.path().join("read-only-root"));
    assert!(matches!(
        read_only.save(&fixture.0, &fixture.1, &fixture.2, &fixture.3),
        Err(ManagedCacheSaveError::BeforePublication(error))
            if error.kind() == ManagedCacheStoreErrorKind::ReadOnly
    ));
    let Err(error) = read_only.prepare_clear() else {
        panic!("read-only store prepared a clear");
    };
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::ReadOnly);
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test spawns only its exact unit-test helper against a TempDir-owned cache store"
)]
fn reset_writer_admission_excludes_an_independent_process() {
    const ROLE: &str = "DUX_CACHE_RESET_LOCK_CHILD";
    const CONTAINER: &str = "DUX_CACHE_RESET_LOCK_CONTAINER";
    const ABSENT_READY: &str = "DUX_CACHE_RESET_ABSENT_READY";
    const ABSENT_RELEASE: &str = "DUX_CACHE_RESET_ABSENT_RELEASE";
    const PROVISIONED: &str = "DUX_CACHE_RESET_PROVISIONED";
    const WRITER_READY: &str = "DUX_CACHE_RESET_WRITER_READY";
    const WRITER_RELEASE: &str = "DUX_CACHE_RESET_WRITER_RELEASE";

    if std::env::var_os(ROLE).is_some() {
        let path = PathBuf::from(std::env::var_os(CONTAINER).unwrap());
        let absent_ready = PathBuf::from(std::env::var_os(ABSENT_READY).unwrap());
        let absent_release = PathBuf::from(std::env::var_os(ABSENT_RELEASE).unwrap());
        let provisioned = PathBuf::from(std::env::var_os(PROVISIONED).unwrap());
        let writer_ready = PathBuf::from(std::env::var_os(WRITER_READY).unwrap());
        let writer_release = PathBuf::from(std::env::var_os(WRITER_RELEASE).unwrap());
        ManagedCacheStore::with_app_data_reset_admission_until(
            &path,
            &reset_transaction(),
            Instant::now() + Duration::from_secs(1),
            |admission| {
                assert!(!admission.is_present());
                admission.revalidate().unwrap();
                fs::write(absent_ready, b"ready").unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                while !absent_release.exists() {
                    assert!(Instant::now() < deadline, "parent did not release absence");
                    std::thread::sleep(Duration::from_millis(5));
                }
                admission.revalidate().unwrap();
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !provisioned.exists() {
            assert!(Instant::now() < deadline, "parent did not provision store");
            std::thread::sleep(Duration::from_millis(5));
        }
        let store = open_rw(&path);
        store
            .with_app_data_reset_writer_admission(Duration::from_secs(1), |admission| {
                admission.revalidate().unwrap();
                fs::write(writer_ready, b"ready").unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                while !writer_release.exists() {
                    assert!(Instant::now() < deadline, "parent did not release writer");
                    std::thread::sleep(Duration::from_millis(5));
                }
                admission.revalidate().unwrap();
            })
            .unwrap();
        return;
    }

    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let absent_ready = temp.path().join("cache-reset-absent-ready");
    let absent_release = temp.path().join("cache-reset-absent-release");
    let provisioned = temp.path().join("cache-reset-provisioned");
    let writer_ready = temp.path().join("cache-reset-writer-ready");
    let writer_release = temp.path().join("cache-reset-writer-release");
    // DUX-DESTRUCTIVE: allow=test-cache-reset-lock-helper-spawn -- relaunch only this exact unit test against its TempDir-owned cache namespace to prove kernel publication and writer exclusion across a real process boundary
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("cache::managed_store::tests::reset_writer_admission_excludes_an_independent_process")
        .arg("--nocapture")
        .env(ROLE, "1")
        .env(CONTAINER, &path)
        .env(ABSENT_READY, &absent_ready)
        .env(ABSENT_RELEASE, &absent_release)
        .env(PROVISIONED, &provisioned)
        .env(WRITER_READY, &writer_ready)
        .env(WRITER_RELEASE, &writer_release)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !absent_ready.exists() {
        assert!(
            Instant::now() < deadline,
            "child did not acquire absent publication fence"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let Err(error) = ManagedCacheStore::open_until(
        &path,
        ManagedCacheStoreAccess::ReadWrite,
        Instant::now() + Duration::from_millis(20),
    ) else {
        panic!("independent publisher crossed the absent fence");
    };
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::Busy);
    assert!(!path.exists());
    fs::write(&absent_release, b"release").unwrap();
    let store = open_rw(&path);
    fs::write(&provisioned, b"ready").unwrap();
    while !writer_ready.exists() {
        assert!(
            Instant::now() < deadline,
            "child did not acquire cache writer lock"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        store
            .with_app_data_reset_writer_admission(Duration::from_millis(20), |_| (),)
            .unwrap_err()
            .kind(),
        ManagedCacheStoreErrorKind::Busy
    );
    fs::write(&writer_release, b"release").unwrap();
    assert!(child.wait().unwrap().success());
    store
        .with_app_data_reset_writer_admission(Duration::from_millis(100), |admission| {
            admission.revalidate().unwrap();
        })
        .unwrap();
}

#[test]
fn reset_writer_admission_is_scoped_and_unwind_releases_the_lock() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let independent = open_rw(&path);

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store.with_app_data_reset_writer_admission(
            Duration::from_millis(100),
            |admission| -> () {
                admission.revalidate().unwrap();
                let Err(error) = independent.acquire_writer_lock(Duration::ZERO) else {
                    panic!("reset callback did not retain the cache writer lock");
                };
                assert_eq!(error.kind(), ManagedCacheStoreErrorKind::Busy);
                panic!("simulate reset callback panic");
            },
        );
    }));

    assert!(panic.is_err());
    drop(independent.acquire_writer_lock(Duration::ZERO).unwrap());
}

#[test]
fn reset_inventory_never_restarts_the_original_deadline() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let mut called = false;

    set_test_inventory_delay(Duration::from_millis(60));
    let error = store
        .with_app_data_reset_writer_admission(Duration::from_millis(30), |_| called = true)
        .unwrap_err();
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::Busy);
    assert!(!called);

    let kind = store
        .with_app_data_reset_writer_admission(Duration::from_millis(100), |admission| {
            set_test_inventory_delay(Duration::from_millis(130));
            admission.revalidate().unwrap_err().kind()
        })
        .unwrap();
    assert_eq!(kind, ManagedCacheStoreErrorKind::Busy);
}

#[test]
#[allow(
    clippy::forget_non_drop,
    reason = "the wrapper is deliberately forgotten to prove its borrowed owners still release their fences"
)]
fn absent_reset_admission_excludes_same_process_publishers_and_releases_after_forget() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);

    ManagedCacheStore::with_app_data_reset_admission_until(
        &path,
        &reset_transaction(),
        Instant::now() + Duration::from_millis(100),
        |admission| {
            assert!(!admission.is_present());
            admission.revalidate().unwrap();
            let Err(error) = ManagedCacheStore::open_until(
                &path,
                ManagedCacheStoreAccess::ReadWrite,
                Instant::now() + Duration::from_millis(20),
            ) else {
                panic!("same-process publisher crossed the absent fence");
            };
            assert_eq!(error.kind(), ManagedCacheStoreErrorKind::Busy);
            std::mem::forget(admission);
        },
    )
    .unwrap();

    assert!(!path.exists());
    drop(open_rw(&path));
    assert!(path.join(STORE_DIRECTORY_NAME).is_dir());
}

#[test]
fn absent_child_reset_admission_preserves_unknown_outer_siblings() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let sibling = path.join("legacy-cache");
    fs::write(&sibling, b"outside ownership").unwrap();

    ManagedCacheStore::with_app_data_reset_admission_until(
        &path,
        &reset_transaction(),
        Instant::now() + Duration::from_millis(100),
        |admission| {
            assert!(!admission.is_present());
            admission.revalidate().unwrap();
            let Err(error) = ManagedCacheStore::open_until(
                &path,
                ManagedCacheStoreAccess::ReadWrite,
                Instant::now() + Duration::from_millis(20),
            ) else {
                panic!("same-process publisher crossed the child-absence fence");
            };
            assert_eq!(error.kind(), ManagedCacheStoreErrorKind::Busy);
        },
    )
    .unwrap();

    assert_eq!(fs::read(sibling).unwrap(), b"outside ownership");
    assert!(!path.join(STORE_DIRECTORY_NAME).exists());
}

#[test]
fn reset_admission_requires_the_typed_cache_stage_destination_to_remain_absent() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let transaction = reset_transaction();
    let stage = path.join(transaction.cache_stage().as_str());
    fs::create_dir(&stage).unwrap();
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).unwrap();

    let mut called = false;
    let error = ManagedCacheStore::with_app_data_reset_admission_until(
        &path,
        &transaction,
        Instant::now() + Duration::from_millis(100),
        |_| called = true,
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        ManagedCacheStoreErrorKind::ChangedSinceSnapshot
    );
    assert!(!called);

    let present = TempDir::new().unwrap();
    let present_path = container(&present);
    let store = open_rw(&present_path);
    let present_stage = present_path.join(transaction.cache_stage().as_str());
    let kind = store
        .with_app_data_reset_writer_admission(Duration::from_millis(100), |admission| {
            fs::create_dir(&present_stage).unwrap();
            fs::set_permissions(&present_stage, fs::Permissions::from_mode(0o700)).unwrap();
            admission.revalidate().unwrap_err().kind()
        })
        .unwrap();
    assert_eq!(kind, ManagedCacheStoreErrorKind::ChangedSinceSnapshot);
}

#[test]
fn absent_reset_admission_rejects_parent_and_container_replacement() {
    let parent_case = TempDir::new().unwrap();
    let parent = parent_case.path().join("Caches");
    let path = parent.join("Dux");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let parent_kind = ManagedCacheStore::with_app_data_reset_admission_until(
        &path,
        &reset_transaction(),
        Instant::now() + Duration::from_millis(100),
        |admission| {
            let root = File::open(parent_case.path()).unwrap();
            let retained =
                platform::open_existing_private_directory(&root, parent_case.path(), "Caches")
                    .unwrap()
                    .unwrap();
            let identity = platform::identity(&retained, platform::Kind::PrivateDirectory).unwrap();
            assert_eq!(
                platform::publish_directory_no_replace(
                    &root,
                    "Caches",
                    &retained,
                    identity,
                    "Caches-detached",
                )
                .unwrap(),
                platform::Publication::Published
            );
            fs::create_dir(&parent).unwrap();
            admission.revalidate().unwrap_err().kind()
        },
    )
    .unwrap();
    assert_eq!(parent_kind, ManagedCacheStoreErrorKind::UnsafeContainer);

    let container_case = TempDir::new().unwrap();
    let path = container(&container_case);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let container_kind = ManagedCacheStore::with_app_data_reset_admission_until(
        &path,
        &reset_transaction(),
        Instant::now() + Duration::from_millis(100),
        |admission| {
            let (root, _) = platform::open_container_parent(&path).unwrap();
            let retained =
                platform::open_existing_private_directory(&root, container_case.path(), "Dux")
                    .unwrap()
                    .unwrap();
            let identity = platform::identity(&retained, platform::Kind::PrivateDirectory).unwrap();
            assert_eq!(
                platform::publish_directory_no_replace(
                    &root,
                    "Dux",
                    &retained,
                    identity,
                    "Dux-detached",
                )
                .unwrap(),
                platform::Publication::Published
            );
            fs::create_dir(&path).unwrap();
            admission.revalidate().unwrap_err().kind()
        },
    )
    .unwrap();
    assert!(matches!(
        container_kind,
        ManagedCacheStoreErrorKind::UnsafeContainer
            | ManagedCacheStoreErrorKind::ChangedSinceSnapshot
    ));
}

#[test]
fn reset_writer_admission_rejects_a_new_child_during_revalidation() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let owned = path.join(STORE_DIRECTORY_NAME);

    let kind = store
        .with_app_data_reset_writer_admission(Duration::from_millis(100), |admission| {
            fs::write(
                owned.join("unknown"),
                b"actor ignored the advisory writer lock",
            )
            .unwrap();
            admission.revalidate().unwrap_err().kind()
        })
        .unwrap();

    assert_eq!(kind, ManagedCacheStoreErrorKind::UnsafeObject);
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test renames only a TempDir-owned cache store to prove canonical-name revalidation"
)]
fn reset_writer_admission_rejects_detached_canonical_cache_directory() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let detached = path.join("detached-scan-cache");

    let kind = store
        .with_app_data_reset_writer_admission(Duration::from_millis(100), |admission| {
            // DUX-DESTRUCTIVE: allow=test-cache-reset-canonical-binding-rename -- rename only this TempDir-owned marker-validated cache directory to prove a retained descriptor cannot stand in for the canonical name
            fs::rename(&canonical, &detached).unwrap();
            admission.revalidate().unwrap_err().kind()
        })
        .unwrap();

    assert!(matches!(
        kind,
        ManagedCacheStoreErrorKind::UnsafeStore | ManagedCacheStoreErrorKind::UnsafeObject
    ));
    assert!(!canonical.exists());
    assert!(detached.exists());
}

#[test]
fn recovery_admission_detaches_only_the_exact_canonical_store() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    drop(open_rw(&path));
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let expected_identity = store_identity(&canonical);
    let transaction = reset_transaction();
    let stage = path.join(transaction.cache_stage().as_str());

    ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        &path,
        Some(expected_identity),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| {
            assert_eq!(
                admission.location(),
                AppDataResetManagedCacheRecoveryLocation::Canonical
            );
            admission.revalidate().unwrap();
            let detached = admission.detach_if_canonical().unwrap();
            assert_eq!(
                detached.location(),
                AppDataResetManagedCacheRecoveryLocation::Detached
            );
            detached.revalidate().unwrap();
            let writer = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(stage.join(WRITER_LOCK_NAME))
                .unwrap();
            assert!(matches!(
                FileExt::try_lock(&writer),
                Err(TryLockError::WouldBlock)
            ));
        },
    )
    .unwrap();

    assert!(!canonical.exists());
    assert_eq!(store_identity(&stage), expected_identity);
    let writer = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(stage.join(WRITER_LOCK_NAME))
        .unwrap();
    FileExt::try_lock(&writer).unwrap();
    FileExt::unlock(&writer).unwrap();
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test renames only its TempDir-owned marker-validated cache fixture to model a crash after detachment"
)]
fn recovery_admission_accepts_an_exact_already_detached_store_without_repeating_effect() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    drop(open_rw(&path));
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let expected_identity = store_identity(&canonical);
    let transaction = reset_transaction();
    let stage = path.join(transaction.cache_stage().as_str());
    // DUX-DESTRUCTIVE: allow=test-cache-recovery-already-detached-rename -- move only the exact TempDir-owned cache store to its transaction-derived recovery name
    fs::rename(&canonical, &stage).unwrap();

    ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        &path,
        Some(expected_identity),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| {
            assert_eq!(
                admission.location(),
                AppDataResetManagedCacheRecoveryLocation::Detached
            );
            let detached = admission.detach_if_canonical().unwrap();
            assert_eq!(
                detached.location(),
                AppDataResetManagedCacheRecoveryLocation::Detached
            );
            detached.revalidate().unwrap();
        },
    )
    .unwrap();

    assert!(!canonical.exists());
    assert_eq!(store_identity(&stage), expected_identity);
}

#[test]
fn recovery_admission_proves_journaled_absence_without_provisioning() {
    let missing = TempDir::new().unwrap();
    let missing_path = container(&missing);
    let transaction = reset_transaction();
    ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        &missing_path,
        None,
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| {
            assert_eq!(
                admission.location(),
                AppDataResetManagedCacheRecoveryLocation::ProvenAbsent
            );
            let absent = admission.detach_if_canonical().unwrap();
            assert_eq!(
                absent.location(),
                AppDataResetManagedCacheRecoveryLocation::ProvenAbsent
            );
        },
    )
    .unwrap();
    assert!(!missing_path.exists());

    let present_container = TempDir::new().unwrap();
    let path = container(&present_container);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let sibling = path.join("foreign-sibling");
    fs::write(&sibling, b"unowned").unwrap();
    ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        &path,
        None,
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |_| (),
    )
    .unwrap();
    assert_eq!(fs::read(sibling).unwrap(), b"unowned");
    assert!(!path.join(STORE_DIRECTORY_NAME).exists());
    assert!(!path.join(transaction.cache_stage().as_str()).exists());
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test moves only a TempDir-owned cache fixture to cover both journaled-absence namespace locations"
)]
fn recovery_admission_rejects_cache_appearance_after_journaled_absence() {
    for detached in [false, true] {
        let temp = TempDir::new().unwrap();
        let path = container(&temp);
        drop(open_rw(&path));
        let transaction = reset_transaction();
        let canonical = path.join(STORE_DIRECTORY_NAME);
        let target = if detached {
            let stage = path.join(transaction.cache_stage().as_str());
            // DUX-DESTRUCTIVE: allow=test-cache-recovery-absence-stage-rename -- move only the TempDir-owned marker-valid cache child to model unexpected detached-cache appearance
            fs::rename(&canonical, &stage).unwrap();
            stage
        } else {
            canonical
        };
        let identity = store_identity(&target);
        let error = ManagedCacheStore::with_app_data_reset_recovery_admission_until(
            &path,
            None,
            transaction.cache_stage(),
            Instant::now() + Duration::from_secs(1),
            |_| (),
        )
        .unwrap_err();
        assert_eq!(
            error.kind(),
            ManagedCacheStoreErrorKind::ChangedSinceSnapshot
        );
        assert_eq!(store_identity(&target), identity);
    }
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test moves only marker-validated TempDir-owned cache fixtures to construct exact recovery collision shapes"
)]
fn recovery_admission_rejects_both_neither_and_wrong_identity_without_mutation() {
    let neither = TempDir::new().unwrap();
    let neither_path = container(&neither);
    let transaction = reset_transaction();
    let neither_error = ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        &neither_path,
        Some((1, 2)),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |_| (),
    )
    .unwrap_err();
    assert_eq!(
        neither_error.kind(),
        ManagedCacheStoreErrorKind::ChangedSinceSnapshot
    );
    assert!(!neither_path.exists());

    let wrong = TempDir::new().unwrap();
    let wrong_path = container(&wrong);
    drop(open_rw(&wrong_path));
    let canonical = wrong_path.join(STORE_DIRECTORY_NAME);
    let canonical_identity = store_identity(&canonical);
    let wrong_error = ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        &wrong_path,
        Some((canonical_identity.0, canonical_identity.1.wrapping_add(1))),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |_| (),
    )
    .unwrap_err();
    assert_eq!(
        wrong_error.kind(),
        ManagedCacheStoreErrorKind::ChangedSinceSnapshot
    );
    assert_eq!(store_identity(&canonical), canonical_identity);

    let both = TempDir::new().unwrap();
    let both_path = container(&both);
    drop(open_rw(&both_path));
    let canonical = both_path.join(STORE_DIRECTORY_NAME);
    let canonical_identity = store_identity(&canonical);
    let source_parent = both.path().join("source");
    let source_path = source_parent.join("Dux");
    fs::create_dir(&source_parent).unwrap();
    drop(open_rw(&source_path));
    let source = source_path.join(STORE_DIRECTORY_NAME);
    let stage = both_path.join(transaction.cache_stage().as_str());
    // DUX-DESTRUCTIVE: allow=test-cache-recovery-both-names-rename -- move only a second TempDir-owned marker-valid cache store into the exact detached collision name
    fs::rename(&source, &stage).unwrap();
    let stage_identity = store_identity(&stage);
    let both_error = ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        &both_path,
        Some(canonical_identity),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |_| (),
    )
    .unwrap_err();
    assert_eq!(
        both_error.kind(),
        ManagedCacheStoreErrorKind::ChangedSinceSnapshot
    );
    assert_eq!(store_identity(&canonical), canonical_identity);
    assert_eq!(store_identity(&stage), stage_identity);
}

#[test]
fn recovery_admission_rejects_wrong_types_and_inventory_drift() {
    let wrong_type = TempDir::new().unwrap();
    let path = container(&wrong_type);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let transaction = reset_transaction();
    let stage = path.join(transaction.cache_stage().as_str());
    fs::write(&stage, b"foreign file").unwrap();
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o600)).unwrap();
    let error = ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        &path,
        Some((1, 2)),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |_| (),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::UnsafeStore);
    assert_eq!(fs::read(stage).unwrap(), b"foreign file");

    let drift = TempDir::new().unwrap();
    let path = container(&drift);
    drop(open_rw(&path));
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let identity = store_identity(&canonical);
    let kind = ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        &path,
        Some(identity),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| {
            fs::write(canonical.join("unknown"), b"ignored writer lease").unwrap();
            admission.revalidate().unwrap_err().kind()
        },
    )
    .unwrap();
    assert_eq!(kind, ManagedCacheStoreErrorKind::UnsafeObject);
}

#[test]
fn recovery_detach_faults_preserve_the_exact_pre_or_post_rename_shape() {
    for (fault, renamed) in [
        (TEST_FAULT_RESET_BEFORE_RENAME, false),
        (TEST_FAULT_RESET_AFTER_RENAME, true),
        (TEST_FAULT_RESET_AFTER_DIRECTORY_SYNC, true),
        (TEST_FAULT_RESET_DURING_READBACK, true),
    ] {
        let temp = TempDir::new().unwrap();
        let path = container(&temp);
        drop(open_rw(&path));
        let transaction = reset_transaction();
        let canonical = path.join(STORE_DIRECTORY_NAME);
        let stage = path.join(transaction.cache_stage().as_str());
        let identity = store_identity(&canonical);
        set_test_fault(fault);
        let kind = ManagedCacheStore::with_app_data_reset_recovery_admission_until(
            &path,
            Some(identity),
            transaction.cache_stage(),
            Instant::now() + Duration::from_secs(1),
            |admission| match admission.detach_if_canonical() {
                Err(error) => error.kind(),
                Ok(_) => panic!("injected recovery detach fault did not fire"),
            },
        )
        .unwrap();
        assert_eq!(kind, ManagedCacheStoreErrorKind::Unavailable);
        if renamed {
            assert!(!canonical.exists());
            assert_eq!(store_identity(&stage), identity);
        } else {
            assert_eq!(store_identity(&canonical), identity);
            assert!(!stage.exists());
        }
        let writer_path = if renamed { &stage } else { &canonical };
        let writer = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(writer_path.join(WRITER_LOCK_NAME))
            .unwrap();
        FileExt::try_lock(&writer).unwrap();
        FileExt::unlock(&writer).unwrap();
    }
}

#[test]
fn detached_cache_drain_removes_one_lexical_payload_per_recovery_admission() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let outer_sibling = path.join("outside-managed-store");
    fs::write(&outer_sibling, b"untouched outer sibling").unwrap();
    let store = open_rw(&path);
    for root in ["scan-root-a", "scan-root-b"] {
        let (root, config, metadata, tree) = cache_fixture(&temp.path().join(root));
        store.save(&root, &config, &metadata, &tree).unwrap();
    }
    drop(store);

    let transaction = reset_transaction();
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let stage = path.join(transaction.cache_stage().as_str());
    let identity = store_identity(&canonical);
    detach_cache_for_drain(&path, identity, &transaction);
    let original_names = detached_payload_names(&stage);
    assert_eq!(original_names.len(), 2);

    let first = drain_cache_once(&path, Some(identity), &transaction).unwrap();
    assert_eq!(first.removed_objects(), 1);
    assert!(first.cache_payload_has_more());
    assert_eq!(detached_payload_names(&stage), original_names[1..]);
    assert_eq!(store_identity(&stage), identity);

    let second = drain_cache_once(&path, Some(identity), &transaction).unwrap();
    assert_eq!(second.removed_objects(), 1);
    assert!(!second.cache_payload_has_more());
    assert!(detached_payload_names(&stage).is_empty());
    assert_eq!(store_identity(&stage), identity);

    let empty = drain_cache_once(&path, Some(identity), &transaction).unwrap();
    assert_eq!(empty.removed_objects(), 0);
    assert!(!empty.cache_payload_has_more());
    assert_eq!(fs::read(stage.join(MARKER_NAME)).unwrap(), STORE_MARKER);
    assert_eq!(
        fs::read(stage.join(WRITER_LOCK_NAME)).unwrap(),
        WRITER_MARKER
    );
    assert_eq!(fs::read(outer_sibling).unwrap(), b"untouched outer sibling");
}

#[test]
fn detached_cache_drain_faults_are_restart_safe_and_preserve_controls() {
    for (fault, before_effect) in [
        (AppDataResetCacheDrainFault::BeforeUnlink, true),
        (AppDataResetCacheDrainFault::AfterUnlink, false),
        (AppDataResetCacheDrainFault::AfterDirectorySync, false),
        (AppDataResetCacheDrainFault::DuringReadback, false),
    ] {
        let temp = TempDir::new().unwrap();
        let path = container(&temp);
        let store = open_rw(&path);
        let (root, config, metadata, tree) = cache_fixture(&temp.path().join("fault-scan-root"));
        store.save(&root, &config, &metadata, &tree).unwrap();
        drop(store);
        let transaction = reset_transaction();
        let canonical = path.join(STORE_DIRECTORY_NAME);
        let stage = path.join(transaction.cache_stage().as_str());
        let identity = store_identity(&canonical);
        detach_cache_for_drain(&path, identity, &transaction);
        let original_names = detached_payload_names(&stage);
        assert_eq!(original_names.len(), 1);

        set_test_app_data_reset_cache_drain_fault(fault);
        let error = drain_cache_once(&path, Some(identity), &transaction).unwrap_err();
        if before_effect {
            assert_eq!(
                error,
                AppDataResetManagedCacheDrainError::BeforeEffect(
                    ManagedCacheStoreErrorKind::Unavailable
                )
            );
            assert_eq!(detached_payload_names(&stage), original_names);
            let resumed = drain_cache_once(&path, Some(identity), &transaction).unwrap();
            assert_eq!(resumed.removed_objects(), 1);
            assert!(!resumed.cache_payload_has_more());
        } else {
            assert_eq!(error, AppDataResetManagedCacheDrainError::OutcomeUnknown);
            assert!(detached_payload_names(&stage).is_empty());
            let resumed = drain_cache_once(&path, Some(identity), &transaction).unwrap();
            assert_eq!(resumed.removed_objects(), 0);
            assert!(!resumed.cache_payload_has_more());
        }
        assert_eq!(store_identity(&stage), identity);
        assert_eq!(fs::read(stage.join(MARKER_NAME)).unwrap(), STORE_MARKER);
        assert_eq!(
            fs::read(stage.join(WRITER_LOCK_NAME)).unwrap(),
            WRITER_MARKER
        );
    }
}

#[test]
fn cache_drain_candidate_requires_exact_detached_or_absent_journal_binding() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    drop(open_rw(&path));
    let transaction = reset_transaction();
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let identity = store_identity(&canonical);

    let canonical_kind = ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        &path,
        Some(identity),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| match admission.into_drain_candidate(Some(identity), transaction.cache_stage())
        {
            Ok(_) => panic!("canonical cache minted drain authority"),
            Err(error) => error.kind(),
        },
    )
    .unwrap();
    assert_eq!(canonical_kind, ManagedCacheStoreErrorKind::InternalState);
    assert_eq!(store_identity(&canonical), identity);

    detach_cache_for_drain(&path, identity, &transaction);
    let other_transaction =
        AppDataResetTransaction::for_test("ffeeddccbbaa99887766554433221100").unwrap();
    for wrong_binding in [
        (Some((identity.0, identity.1.wrapping_add(1))), false),
        (Some(identity), true),
    ] {
        let kind = ManagedCacheStore::with_app_data_reset_recovery_admission_until(
            &path,
            Some(identity),
            transaction.cache_stage(),
            Instant::now() + Duration::from_secs(1),
            |admission| {
                let expected_stage = if wrong_binding.1 {
                    other_transaction.cache_stage()
                } else {
                    transaction.cache_stage()
                };
                match admission.into_drain_candidate(wrong_binding.0, expected_stage) {
                    Ok(_) => panic!("wrong cache binding minted drain authority"),
                    Err(error) => error.kind(),
                }
            },
        )
        .unwrap();
        assert_eq!(kind, ManagedCacheStoreErrorKind::InternalState);
    }

    let missing = TempDir::new().unwrap();
    let missing_path = container(&missing);
    let absent = drain_cache_once(&missing_path, None, &transaction).unwrap();
    assert_eq!(absent.removed_objects(), 0);
    assert!(!absent.cache_payload_has_more());
    assert!(!missing_path.exists());
}

#[test]
fn cache_drain_refuses_unknown_detached_object_before_effect() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    drop(open_rw(&path));
    let transaction = reset_transaction();
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let stage = path.join(transaction.cache_stage().as_str());
    let identity = store_identity(&canonical);
    detach_cache_for_drain(&path, identity, &transaction);
    let disputed = stage.join("unknown-object");
    fs::write(&disputed, b"not managed-cache grammar").unwrap();

    let error = ManagedCacheStore::with_app_data_reset_recovery_admission_until(
        &path,
        Some(identity),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |_| (),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::UnsafeObject);
    assert_eq!(fs::read(disputed).unwrap(), b"not managed-cache grammar");
    assert_eq!(fs::read(stage.join(MARKER_NAME)).unwrap(), STORE_MARKER);
    assert_eq!(
        fs::read(stage.join(WRITER_LOCK_NAME)).unwrap(),
        WRITER_MARKER
    );
}

#[test]
fn draining_cache_admission_routes_payloads_before_structural_retirement() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let (root, config, metadata, tree) = cache_fixture(&temp.path().join("payload-route"));
    store.save(&root, &config, &metadata, &tree).unwrap();
    drop(store);
    let transaction = reset_transaction();
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let identity = store_identity(&canonical);
    detach_cache_for_drain(&path, identity, &transaction);

    assert_eq!(
        draining_admission_kind(&path, Some(identity), &transaction).unwrap(),
        (true, None)
    );
    let drained = drain_cache_once(&path, Some(identity), &transaction).unwrap();
    assert_eq!(drained.removed_objects(), 1);
    assert!(!drained.cache_payload_has_more());
    assert_eq!(
        draining_admission_kind(&path, Some(identity), &transaction).unwrap(),
        (
            false,
            Some(AppDataResetManagedCacheStageRetirementState::FullControlsEmpty)
        )
    );
}

#[test]
fn detached_cache_structural_retirement_converges_one_exact_effect_per_admission() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let outer_sibling = path.join("unowned-sibling");
    fs::write(&outer_sibling, b"preserved").unwrap();
    drop(open_rw(&path));
    let transaction = reset_transaction();
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let stage = path.join(transaction.cache_stage().as_str());
    let identity = store_identity(&canonical);
    detach_cache_for_drain(&path, identity, &transaction);

    assert_eq!(
        draining_admission_kind(&path, Some(identity), &transaction).unwrap(),
        (
            false,
            Some(AppDataResetManagedCacheStageRetirementState::FullControlsEmpty)
        )
    );
    let marker = retire_cache_structure_once(&path, Some(identity), &transaction).unwrap();
    assert_eq!(marker.removed_structural_objects(), 1);
    assert!(marker.cache_stage_has_more());
    assert!(!stage.join(MARKER_NAME).exists());
    assert_eq!(
        fs::read(stage.join(WRITER_LOCK_NAME)).unwrap(),
        WRITER_MARKER
    );
    assert_eq!(store_identity(&stage), identity);
    assert_eq!(
        draining_admission_kind(&path, Some(identity), &transaction).unwrap(),
        (
            false,
            Some(AppDataResetManagedCacheStageRetirementState::WriterOnly)
        )
    );

    let writer = retire_cache_structure_once(&path, Some(identity), &transaction).unwrap();
    assert_eq!(writer.removed_structural_objects(), 1);
    assert!(writer.cache_stage_has_more());
    assert!(fs::read_dir(&stage).unwrap().next().is_none());
    assert_eq!(store_identity(&stage), identity);
    assert_eq!(
        draining_admission_kind(&path, Some(identity), &transaction).unwrap(),
        (
            false,
            Some(AppDataResetManagedCacheStageRetirementState::EmptyStage)
        )
    );

    let shell = retire_cache_structure_once(&path, Some(identity), &transaction).unwrap();
    assert_eq!(shell.removed_structural_objects(), 1);
    assert!(!shell.cache_stage_has_more());
    assert!(!stage.exists());
    assert!(!canonical.exists());
    assert_eq!(fs::read(&outer_sibling).unwrap(), b"preserved");
    assert_eq!(
        draining_admission_kind(&path, Some(identity), &transaction).unwrap(),
        (
            false,
            Some(AppDataResetManagedCacheStageRetirementState::Absent)
        )
    );
    assert_cache_absent_witness(&path, Some(identity), &transaction);

    let missing = TempDir::new().unwrap();
    let missing_path = container(&missing);
    assert_eq!(
        draining_admission_kind(&missing_path, None, &transaction).unwrap(),
        (
            false,
            Some(AppDataResetManagedCacheStageRetirementState::Absent)
        )
    );
    assert!(!missing_path.exists());
    assert_cache_absent_witness(&missing_path, None, &transaction);
}

#[test]
fn detached_cache_structural_retirement_faults_resume_from_exact_tail_state() {
    for initial_state in [
        AppDataResetManagedCacheStageRetirementState::FullControlsEmpty,
        AppDataResetManagedCacheStageRetirementState::WriterOnly,
        AppDataResetManagedCacheStageRetirementState::EmptyStage,
    ] {
        for (fault, before_effect) in [
            (
                TestAppDataResetCacheStageRetirementFault::BeforeEffect,
                true,
            ),
            (
                TestAppDataResetCacheStageRetirementFault::AfterEffect,
                false,
            ),
            (
                TestAppDataResetCacheStageRetirementFault::AfterDirectorySync,
                false,
            ),
            (
                TestAppDataResetCacheStageRetirementFault::DuringReadback,
                false,
            ),
        ] {
            let temp = TempDir::new().unwrap();
            let path = container(&temp);
            drop(open_rw(&path));
            let transaction = reset_transaction();
            let canonical = path.join(STORE_DIRECTORY_NAME);
            let identity = store_identity(&canonical);
            detach_cache_for_drain(&path, identity, &transaction);
            let successful_prefix = match initial_state {
                AppDataResetManagedCacheStageRetirementState::FullControlsEmpty => 0,
                AppDataResetManagedCacheStageRetirementState::WriterOnly => 1,
                AppDataResetManagedCacheStageRetirementState::EmptyStage => 2,
                AppDataResetManagedCacheStageRetirementState::Absent => unreachable!(),
            };
            for _ in 0..successful_prefix {
                retire_cache_structure_once(&path, Some(identity), &transaction).unwrap();
            }
            assert_eq!(
                draining_admission_kind(&path, Some(identity), &transaction)
                    .unwrap()
                    .1,
                Some(initial_state)
            );

            set_test_app_data_reset_cache_stage_retirement_fault(fault);
            let error =
                retire_cache_structure_once(&path, Some(identity), &transaction).unwrap_err();
            if before_effect {
                assert_eq!(
                    error,
                    AppDataResetManagedCacheStageRetirementError::BeforeEffect(
                        ManagedCacheStoreErrorKind::Unavailable
                    ),
                    "{initial_state:?} {fault:?}"
                );
                assert_eq!(
                    draining_admission_kind(&path, Some(identity), &transaction)
                        .unwrap()
                        .1,
                    Some(initial_state),
                    "{initial_state:?} {fault:?}"
                );
            } else {
                assert_eq!(
                    error,
                    AppDataResetManagedCacheStageRetirementError::OutcomeUnknown,
                    "{initial_state:?} {fault:?}"
                );
                let expected_after = match initial_state {
                    AppDataResetManagedCacheStageRetirementState::FullControlsEmpty => {
                        AppDataResetManagedCacheStageRetirementState::WriterOnly
                    }
                    AppDataResetManagedCacheStageRetirementState::WriterOnly => {
                        AppDataResetManagedCacheStageRetirementState::EmptyStage
                    }
                    AppDataResetManagedCacheStageRetirementState::EmptyStage => {
                        AppDataResetManagedCacheStageRetirementState::Absent
                    }
                    AppDataResetManagedCacheStageRetirementState::Absent => unreachable!(),
                };
                assert_eq!(
                    draining_admission_kind(&path, Some(identity), &transaction)
                        .unwrap()
                        .1,
                    Some(expected_after),
                    "{initial_state:?} {fault:?}"
                );
            }

            // A fresh admission resumes the observed state and never
            // repeats the structural object already proven absent.
            if draining_admission_kind(&path, Some(identity), &transaction)
                .unwrap()
                .1
                == Some(AppDataResetManagedCacheStageRetirementState::Absent)
            {
                assert_cache_absent_witness(&path, Some(identity), &transaction);
            } else {
                let resumed =
                    retire_cache_structure_once(&path, Some(identity), &transaction).unwrap();
                assert_eq!(resumed.removed_structural_objects(), 1);
            }
        }
    }
}

#[test]
fn structural_retirement_rejects_non_monotonic_and_unsafe_shapes() {
    #[derive(Clone, Copy)]
    enum Shape {
        MarkerOnly,
        PartialWithPayload,
        UnknownObject,
        Symlink,
    }

    for shape in [
        Shape::MarkerOnly,
        Shape::PartialWithPayload,
        Shape::UnknownObject,
        Shape::Symlink,
    ] {
        let temp = TempDir::new().unwrap();
        let path = container(&temp);
        let store = open_rw(&path);
        if matches!(shape, Shape::PartialWithPayload) {
            let fixture = cache_fixture(&temp.path().join("partial-payload"));
            store
                .save(&fixture.0, &fixture.1, &fixture.2, &fixture.3)
                .unwrap();
        }
        drop(store);
        let transaction = reset_transaction();
        let canonical = path.join(STORE_DIRECTORY_NAME);
        let stage = path.join(transaction.cache_stage().as_str());
        let identity = store_identity(&canonical);
        detach_cache_for_drain(&path, identity, &transaction);
        match shape {
            Shape::MarkerOnly => remove_test_stage_file(&stage, WRITER_LOCK_NAME),
            Shape::PartialWithPayload => remove_test_stage_file(&stage, MARKER_NAME),
            Shape::UnknownObject => {
                fs::write(stage.join("unknown"), b"not managed cache").unwrap();
            }
            Shape::Symlink => {
                symlink("outside", stage.join(format!("{}.dux", "a".repeat(64)))).unwrap();
            }
        }

        assert!(
            draining_admission_kind(&path, Some(identity), &transaction).is_err(),
            "unsafe structural shape was admitted"
        );
        assert_eq!(store_identity(&stage), identity);
    }

    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    drop(open_rw(&path));
    let transaction = reset_transaction();
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let identity = store_identity(&canonical);
    detach_cache_for_drain(&path, identity, &transaction);
    assert_eq!(
        draining_admission_kind(
            &path,
            Some((identity.0, identity.1.wrapping_add(1))),
            &transaction
        )
        .unwrap_err(),
        ManagedCacheStoreErrorKind::ChangedSinceSnapshot
    );

    fs::create_dir(&canonical).unwrap();
    fs::set_permissions(&canonical, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(draining_admission_kind(&path, Some(identity), &transaction).is_err());
    assert!(path.join(transaction.cache_stage().as_str()).exists());
}

#[test]
fn structural_retirement_candidate_binding_rejects_other_transaction() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    drop(open_rw(&path));
    let transaction = reset_transaction();
    let other = AppDataResetTransaction::for_test("ffeeddccbbaa99887766554433221100").unwrap();
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let identity = store_identity(&canonical);
    detach_cache_for_drain(&path, identity, &transaction);

    ManagedCacheStore::with_app_data_reset_draining_cache_admission_until(
        &path,
        Some(identity),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| match admission {
            AppDataResetManagedCacheDrainingAdmission::PayloadsRemain(_) => {
                panic!("empty store had payloads")
            }
            AppDataResetManagedCacheDrainingAdmission::Retirement(candidate) => {
                assert!(candidate.is_bound_to(Some(identity), transaction.cache_stage()));
                assert!(!candidate.is_bound_to(Some(identity), other.cache_stage()));
                assert!(!candidate.is_bound_to(
                    Some((identity.0, identity.1.wrapping_add(1))),
                    transaction.cache_stage()
                ));
                candidate.revalidate().unwrap();
            }
            AppDataResetManagedCacheDrainingAdmission::Absent(_) => {
                panic!("present empty stage was mistaken for absence")
            }
        },
    )
    .unwrap();
}

#[test]
fn structural_retirement_candidate_rechecks_inventory_before_effect() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    drop(open_rw(&path));
    let transaction = reset_transaction();
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let stage = path.join(transaction.cache_stage().as_str());
    let identity = store_identity(&canonical);
    detach_cache_for_drain(&path, identity, &transaction);

    let error = ManagedCacheStore::with_app_data_reset_draining_cache_admission_until(
        &path,
        Some(identity),
        transaction.cache_stage(),
        Instant::now() + Duration::from_secs(1),
        |admission| match admission {
            AppDataResetManagedCacheDrainingAdmission::PayloadsRemain(_) => {
                panic!("empty store had payloads")
            }
            AppDataResetManagedCacheDrainingAdmission::Retirement(candidate) => {
                let disputed = stage.join("late-unknown");
                fs::write(&disputed, b"actor ignored the retained writer lock").unwrap();
                let error = candidate
                    .retire_one_structure(AppDataResetCacheStageRetireAuthority::for_test())
                    .unwrap_err();
                assert_eq!(
                    fs::read(disputed).unwrap(),
                    b"actor ignored the retained writer lock"
                );
                error
            }
            AppDataResetManagedCacheDrainingAdmission::Absent(_) => {
                panic!("present empty stage was mistaken for absence")
            }
        },
    )
    .unwrap();

    assert_eq!(
        error,
        AppDataResetManagedCacheStageRetirementError::BeforeEffect(
            ManagedCacheStoreErrorKind::UnsafeObject
        )
    );
    assert_eq!(fs::read(stage.join(MARKER_NAME)).unwrap(), STORE_MARKER);
    assert_eq!(
        fs::read(stage.join(WRITER_LOCK_NAME)).unwrap(),
        WRITER_MARKER
    );
}

#[test]
fn writer_only_retirement_requires_the_retained_writer_lock() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    drop(open_rw(&path));
    let transaction = reset_transaction();
    let canonical = path.join(STORE_DIRECTORY_NAME);
    let stage = path.join(transaction.cache_stage().as_str());
    let identity = store_identity(&canonical);
    detach_cache_for_drain(&path, identity, &transaction);
    retire_cache_structure_once(&path, Some(identity), &transaction).unwrap();

    let writer = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(stage.join(WRITER_LOCK_NAME))
        .unwrap();
    FileExt::try_lock(&writer).unwrap();
    let error = ManagedCacheStore::with_app_data_reset_draining_cache_admission_until(
        &path,
        Some(identity),
        transaction.cache_stage(),
        Instant::now() + Duration::from_millis(25),
        |_| (),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::Busy);
    assert_eq!(
        fs::read(stage.join(WRITER_LOCK_NAME)).unwrap(),
        WRITER_MARKER
    );
    FileExt::unlock(&writer).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test changes only exact spelling of a TempDir-owned cache child to exercise case-folded alias refusal"
)]
fn recovery_admission_rejects_case_folded_canonical_and_stage_aliases() {
    for detached in [false, true] {
        let temp = TempDir::new().unwrap();
        let path = container(&temp);
        drop(open_rw(&path));
        let canonical = path.join(STORE_DIRECTORY_NAME);
        let identity = store_identity(&canonical);
        let transaction = reset_transaction();
        let requested = if detached {
            transaction.cache_stage().as_str()
        } else {
            STORE_DIRECTORY_NAME
        };
        let alias = requested.to_ascii_uppercase();
        // DUX-DESTRUCTIVE: allow=test-cache-recovery-case-alias-rename -- change only the spelling of the exact TempDir-owned cache child to prove recovery does not accept a case-folded lookup alias
        fs::rename(&canonical, path.join(&alias)).unwrap();
        if !path.join(requested).exists() {
            continue;
        }
        let error = ManagedCacheStore::with_app_data_reset_recovery_admission_until(
            &path,
            Some(identity),
            transaction.cache_stage(),
            Instant::now() + Duration::from_secs(1),
            |_| (),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ManagedCacheStoreErrorKind::UnsafeStore);
        let draining_error = ManagedCacheStore::with_app_data_reset_draining_cache_admission_until(
            &path,
            Some(identity),
            transaction.cache_stage(),
            Instant::now() + Duration::from_secs(1),
            |_| (),
        )
        .unwrap_err();
        assert_eq!(
            draining_error.kind(),
            ManagedCacheStoreErrorKind::UnsafeStore
        );
        assert_eq!(store_identity(&path.join(alias)), identity);
    }
}

#[test]
fn temporary_inventory_recovers_one_legacy_overflow_but_rejects_more() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let fixture = cache_fixture(&temp.path().join("temporary-root"));
    let key = managed_cache_entry_key(&fixture.0, &fixture.1).unwrap();
    for _ in 0..RECOVERY_MAX_TEMPORARY_OBJECTS {
        let pending = store.create_temp(&key).unwrap();
        drop(pending);
    }
    let footprint = store.footprint().unwrap();
    assert_eq!(
        footprint.temporary_count,
        u32::try_from(RECOVERY_MAX_TEMPORARY_OBJECTS).unwrap()
    );
    assert_eq!(footprint.entry_count, 0);
    assert!(matches!(
        store.save(&fixture.0, &fixture.1, &fixture.2, &fixture.3),
        Err(ManagedCacheSaveError::BeforePublication(error))
            if error.kind() == ManagedCacheStoreErrorKind::BudgetExceeded
    ));
    assert_eq!(
        store.footprint().unwrap().temporary_count,
        footprint.temporary_count
    );

    let snapshot = store.prepare_clear().unwrap().unwrap();
    let cleared = store.clear(snapshot).unwrap();
    assert_eq!(cleared.cleared_temporary, footprint.temporary_count);
    assert_eq!(store.footprint().unwrap().temporary_count, 0);

    for _ in 0..=RECOVERY_MAX_TEMPORARY_OBJECTS {
        let pending = store.create_temp(&key).unwrap();
        drop(pending);
    }
    assert_eq!(
        store.footprint().unwrap_err().kind(),
        ManagedCacheStoreErrorKind::BudgetExceeded
    );
    assert_eq!(
        fs::read_dir(path.join(STORE_DIRECTORY_NAME))
            .unwrap()
            .count(),
        RECOVERY_MAX_TEMPORARY_OBJECTS + 3
    );
}

#[test]
fn save_capacity_reserves_insertion_and_replacement_differently() {
    assert!(validate_save_capacity(MAX_NON_CONTROL_OBJECTS - 1, 0, false).is_ok());
    assert!(validate_save_capacity(MAX_NON_CONTROL_OBJECTS, 0, true).is_ok());
    assert_eq!(
        validate_save_capacity(MAX_NON_CONTROL_OBJECTS, 0, false)
            .unwrap_err()
            .kind(),
        ManagedCacheStoreErrorKind::BudgetExceeded
    );
    assert_eq!(
        validate_save_capacity(MAX_NON_CONTROL_OBJECTS + 1, 0, true)
            .unwrap_err()
            .kind(),
        ManagedCacheStoreErrorKind::BudgetExceeded
    );
    assert_eq!(
        validate_save_capacity(1, MAX_TEMPORARY_OBJECTS, true)
            .unwrap_err()
            .kind(),
        ManagedCacheStoreErrorKind::BudgetExceeded
    );
}

#[test]
fn one_over_limit_owned_inventory_remains_countable_and_clearable() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let replacement = cache_fixture(&temp.path().join("replacement-root"));
    let insertion = cache_fixture(&temp.path().join("insertion-root"));
    let replacement_name =
        final_name(&managed_cache_entry_key(&replacement.0, &replacement.1).unwrap());
    let insertion_name = final_name(&managed_cache_entry_key(&insertion.0, &insertion.1).unwrap());
    let create_fake = |name: &str| {
        let (mut file, _) = platform::create_private_file_exclusive(
            &store.inner.directory,
            &store.inner.path,
            name,
        )
        .unwrap()
        .unwrap();
        file.write_all(&[0_u8; MANAGED_CACHE_HEADER_BYTES]).unwrap();
    };
    create_fake(&replacement_name);
    let mut value = 0_usize;
    let mut created = 1_usize;
    while created < MAX_NON_CONTROL_OBJECTS {
        let name = format!("{value:064x}.dux");
        value += 1;
        if name == replacement_name || name == insertion_name {
            continue;
        }
        create_fake(&name);
        created += 1;
    }
    store
        .save(
            &replacement.0,
            &replacement.1,
            &replacement.2,
            &replacement.3,
        )
        .unwrap();
    assert_eq!(
        store.footprint().unwrap().entry_count,
        u32::try_from(MAX_NON_CONTROL_OBJECTS).unwrap()
    );
    assert!(matches!(
        store.save(&insertion.0, &insertion.1, &insertion.2, &insertion.3),
        Err(ManagedCacheSaveError::BeforePublication(error))
            if error.kind() == ManagedCacheStoreErrorKind::BudgetExceeded
    ));
    assert_eq!(store.footprint().unwrap().temporary_count, 0);

    loop {
        let name = format!("{value:064x}.dux");
        value += 1;
        if name != replacement_name && name != insertion_name {
            create_fake(&name);
            break;
        }
    }
    let footprint = store.footprint().unwrap();
    assert_eq!(
        footprint.entry_count,
        u32::try_from(RECOVERY_MAX_NON_CONTROL_OBJECTS).unwrap()
    );
    let snapshot = store.prepare_clear().unwrap().unwrap();
    let cleared = store.clear(snapshot).unwrap();
    assert_eq!(cleared.cleared_entries, footprint.entry_count);
    assert_eq!(store.footprint().unwrap().entry_count, 0);
}

#[test]
fn pending_temp_cleanup_removes_only_its_exact_retained_object() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let fixture = cache_fixture(&temp.path().join("temp-guard-root"));
    let key = managed_cache_entry_key(&fixture.0, &fixture.1).unwrap();

    let pending = store.create_temp(&key).unwrap();
    let pending_path = path.join(STORE_DIRECTORY_NAME).join(&pending.name);
    pending.cleanup(&store.inner.directory).unwrap();
    assert!(!pending_path.exists());

    let pending = store.create_temp(&key).unwrap();
    let pending_path = path.join(STORE_DIRECTORY_NAME).join(&pending.name);
    let retained = pending.file().unwrap().try_clone().unwrap();
    platform::remove_retained_file(
        &store.inner.directory,
        &pending.name,
        retained,
        pending.identity,
    )
    .unwrap();
    fs::write(&pending_path, b"replacement").unwrap();
    fs::set_permissions(&pending_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(pending.cleanup(&store.inner.directory).is_err());
    assert_eq!(fs::read(pending_path).unwrap(), b"replacement");
}

#[test]
fn injected_save_faults_preserve_the_publication_boundary() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let first = cache_fixture(&temp.path().join("pre-publication-root"));
    set_test_fault(TEST_FAULT_PRE_PUBLICATION);
    assert!(matches!(
        store.save(&first.0, &first.1, &first.2, &first.3),
        Err(ManagedCacheSaveError::BeforePublication(error))
            if error.kind() == ManagedCacheStoreErrorKind::Unavailable
    ));
    let after_prepublication = store.footprint().unwrap();
    assert_eq!(after_prepublication.entry_count, 0);
    assert_eq!(after_prepublication.temporary_count, 0);

    let second = cache_fixture(&temp.path().join("post-publication-root"));
    set_test_fault(TEST_FAULT_POST_PUBLICATION);
    assert!(matches!(
        store.save(&second.0, &second.1, &second.2, &second.3),
        Err(ManagedCacheSaveError::OutcomeUnknown)
    ));
    let after_publication = store.footprint().unwrap();
    assert_eq!(after_publication.entry_count, 1);
    assert_eq!(after_publication.temporary_count, 0);
    assert!(store.load(&second.0, &second.1).unwrap().is_some());
}

#[test]
fn provisioning_collision_cleans_only_its_create_new_stage() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let publication = PublicationFence::acquire(
        &path,
        ManagedCacheStoreAccess::ReadOnly,
        Instant::now() + Duration::from_millis(100),
    )
    .unwrap();
    let reopened = ManagedCacheStore::provision(
        &publication,
        path.join(STORE_DIRECTORY_NAME),
        ManagedCacheStoreAccess::ReadWrite,
    )
    .unwrap();
    assert_eq!(reopened.footprint().unwrap(), store.footprint().unwrap());
    let names: Vec<_> = fs::read_dir(&path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [std::ffi::OsString::from(STORE_DIRECTORY_NAME)]);
}

#[test]
fn provisioning_stage_cleanup_refuses_a_replaced_control() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let store = open_rw(&path);
    let stage_name = random_stage_name().unwrap();
    let directory =
        platform::create_private_directory_exclusive(&store.inner.container, &path, &stage_name)
            .unwrap()
            .unwrap();
    let identity = platform::identity(&directory, platform::Kind::PrivateDirectory).unwrap();
    let stage_path = path.join(&stage_name);
    let mut stage = ProvisioningStage {
        name: stage_name,
        directory,
        identity,
        controls: Vec::new(),
    };
    let (marker, marker_identity) = stage
        .create_control(&stage_path, MARKER_NAME, STORE_MARKER)
        .unwrap();
    platform::remove_retained_file(&stage.directory, MARKER_NAME, marker, marker_identity).unwrap();
    let replacement = stage_path.join(MARKER_NAME);
    fs::write(&replacement, b"not our marker").unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();

    assert!(stage.cleanup(&store.inner.container).is_err());
    assert_eq!(fs::read(replacement).unwrap(), b"not our marker");
    assert!(stage_path.exists());
}

#[test]
fn final_and_temporary_name_grammars_are_exact() {
    let final_name = format!("{}.dux", "a".repeat(KEY_HEX_BYTES));
    let temporary = format!(".{final_name}.1.{}.tmp", "0".repeat(RANDOM_HEX_BYTES));
    assert!(is_final_name(&final_name));
    assert!(is_temp_name(&temporary));
    assert!(!is_final_name(&format!(
        "{}.DUX",
        "a".repeat(KEY_HEX_BYTES)
    )));
    assert!(!is_final_name(&format!(
        "{}.dux",
        "A".repeat(KEY_HEX_BYTES)
    )));
    assert!(!is_temp_name(&format!(
        ".{final_name}.01.{}.tmp",
        "0".repeat(RANDOM_HEX_BYTES)
    )));
    assert!(!is_temp_name(&format!(
        ".{final_name}.1.{}.tmp",
        "A".repeat(RANDOM_HEX_BYTES)
    )));
    assert!(!is_temp_name(&format!("../{temporary}")));
}

#[test]
fn completed_reset_optional_cache_errors_are_an_exact_object_only_allowlist() {
    for kind in [
        ManagedCacheStoreErrorKind::UnsafeStore,
        ManagedCacheStoreErrorKind::UnsafeObject,
        ManagedCacheStoreErrorKind::UnrecognizedStore,
        ManagedCacheStoreErrorKind::CorruptData,
        ManagedCacheStoreErrorKind::Unavailable,
    ] {
        assert!(is_optional_completed_cache_object_error(kind), "{kind:?}");
    }
    for kind in [
        ManagedCacheStoreErrorKind::InvalidConfiguration,
        ManagedCacheStoreErrorKind::ReadOnly,
        ManagedCacheStoreErrorKind::UnsafeContainer,
        ManagedCacheStoreErrorKind::Busy,
        ManagedCacheStoreErrorKind::BudgetExceeded,
        ManagedCacheStoreErrorKind::ChangedSinceSnapshot,
        ManagedCacheStoreErrorKind::OutcomeUnknown,
        ManagedCacheStoreErrorKind::UnsupportedPlatform,
        ManagedCacheStoreErrorKind::InternalState,
    ] {
        assert!(!is_optional_completed_cache_object_error(kind), "{kind:?}");
    }
}

#[test]
fn completed_reset_initialized_cache_cannot_hide_fence_configuration_or_deadline_errors() {
    let transaction = reset_transaction();
    let invalid = ManagedCacheStore::validate_app_data_reset_completed_namespace_until(
        Path::new("relative/Dux"),
        None,
        transaction.cache_stage(),
        true,
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap_err();
    assert_eq!(
        invalid.kind(),
        ManagedCacheStoreErrorKind::InvalidConfiguration
    );

    let temp = TempDir::new().unwrap();
    let expired = ManagedCacheStore::validate_app_data_reset_completed_namespace_until(
        &container(&temp),
        None,
        transaction.cache_stage(),
        true,
        Instant::now(),
    )
    .unwrap_err();
    assert_eq!(expired.kind(), ManagedCacheStoreErrorKind::Busy);
}

#[test]
fn completed_reset_publication_registry_contention_obeys_the_original_deadline() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let transaction = reset_transaction();
    let _registry = PUBLICATION_LOCKS_IN_USE
        .lock()
        .expect("test publication registry poisoned");
    let started = Instant::now();
    let error = ManagedCacheStore::validate_app_data_reset_completed_namespace_until(
        &path,
        None,
        transaction.cache_stage(),
        false,
        Instant::now() + Duration::from_millis(25),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::Busy);
    assert!(started.elapsed() < Duration::from_millis(200));
    assert!(!path.exists());
}

#[test]
fn completed_reset_cache_fence_survives_validation_until_explicit_drop() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let transaction = reset_transaction();
    let fence = ManagedCacheStore::validate_app_data_reset_completed_namespace_until(
        &path,
        None,
        transaction.cache_stage(),
        false,
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap();

    let started = Instant::now();
    let Err(error) = ManagedCacheStore::open_until(
        &path,
        ManagedCacheStoreAccess::ReadWrite,
        Instant::now() + Duration::from_millis(25),
    ) else {
        panic!("completed-reset fence did not exclude a cache publisher");
    };
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::Busy);
    assert!(started.elapsed() < Duration::from_millis(200));
    assert!(!path.exists());

    drop(fence);
    assert!(
        ManagedCacheStore::open_until(
            &path,
            ManagedCacheStoreAccess::ReadWrite,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap()
        .is_some()
    );
}

#[test]
fn completed_reset_cache_fence_release_never_waits_for_the_registry() {
    let temp = TempDir::new().unwrap();
    let path = container(&temp);
    let publication = PublicationFence::acquire(
        &path,
        ManagedCacheStoreAccess::ReadOnly,
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap();
    let identity = publication.parent_identity;
    let registry = PUBLICATION_LOCKS_IN_USE
        .lock()
        .expect("test publication registry poisoned");

    let started = Instant::now();
    drop(publication);
    assert!(started.elapsed() < Duration::from_millis(200));
    drop(registry);

    let Err(error) = PublicationFence::acquire(
        &path,
        ManagedCacheStoreAccess::ReadOnly,
        Instant::now() + Duration::from_millis(25),
    ) else {
        panic!("a fail-closed release unexpectedly freed the publication identity");
    };
    assert_eq!(error.kind(), ManagedCacheStoreErrorKind::Busy);
    assert!(
        PUBLICATION_LOCKS_IN_USE
            .lock()
            .expect("test publication registry poisoned")
            .remove(&identity)
    );
}

#[cfg(target_os = "macos")]
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "test changes only the spelling of TempDir-owned completed-reset cache names to prove case-folded aliases cannot satisfy exact absence"
)]
fn completed_reset_rejects_case_folded_canonical_and_transaction_stage_aliases() {
    for canonical in [false, true] {
        let temp = TempDir::new().unwrap();
        let path = container(&temp);
        let transaction = reset_transaction();
        let requested = if canonical {
            STORE_DIRECTORY_NAME
        } else {
            transaction.cache_stage().as_str()
        };
        if canonical {
            drop(open_rw(&path));
        } else {
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            fs::create_dir(path.join(requested)).unwrap();
            fs::set_permissions(path.join(requested), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let alias = requested.to_ascii_uppercase();
        // DUX-DESTRUCTIVE: allow=test-cache-completed-case-alias-rename -- change only the spelling of this TempDir-owned completed-reset cache child to prove exact-name admission rejects a case-folded alias
        fs::rename(path.join(requested), path.join(&alias)).unwrap();
        if !path.join(requested).exists() {
            continue;
        }

        let error = ManagedCacheStore::validate_app_data_reset_completed_namespace_until(
            &path,
            None,
            transaction.cache_stage(),
            canonical,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ManagedCacheStoreErrorKind::UnsafeStore);
        assert!(path.join(alias).is_dir());
    }
}
