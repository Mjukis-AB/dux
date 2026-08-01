use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(test)]
use std::cell::Cell;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::collections::HashMap;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::ffi::OsStrExt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::sync::{OnceLock, Weak};

use fs4::{FileExt, TryLockError};

use super::footprint::OwnedStorageUsage;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::snapshot::storage::{
    AppDataResetSnapshotPayloadDrainCandidate, AppDataResetSnapshotPayloadDrainCompletion,
    AppDataResetSnapshotPayloadDrainError, AppDataResetSnapshotRecovery,
    AppDataResetSnapshotStoreRetirementCompletion, AppDataResetSnapshotStoreRetirementError,
    AppDataResetSnapshotStoreRetirementState, SecureSnapshotStore, SnapshotStorageError,
    SnapshotStorageErrorKind, SnapshotStoreInventoryLease,
};
use super::status::{DatabaseOpenError, DatabaseOpenErrorKind};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use super::{
    AppDataResetOldDatabasePayloadDrainAuthority, AppDataResetSnapshotPayloadDrainAuthority,
    AppDataResetSnapshotStoreRetireAuthority,
};

const WRITER_LOCK_SUFFIX: &str = ".writer.lock";
const CLEANUP_LOCK_SUFFIX: &str = ".cleanup.lock";
const CLEANUP_LOCK_READY_SUFFIX: &str = ".cleanup.lock.ready";
const INITIALIZATION_SENTINEL_SUFFIX: &str = ".initialized";
const SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];
// These are separate, code-owned stores in the normative Application Support
// layout. SQLite does not inspect or mutate them, but an older database layer
// must tolerate their presence so adding snapshots, AI cache, or logs cannot
// make the database itself unreadable.
const RESERVED_APP_SUPPORT_ENTRIES: [&str; 3] = ["snapshots", "ai", "logs"];
const SNAPSHOT_STAGE_PREFIX: &str = ".dux-snapshot-stage-";
const SNAPSHOT_STAGE_SUFFIX_LENGTH: usize = 32;
const MAX_SNAPSHOT_STAGES: usize = 64;
const ROOT_INVENTORY_MAX_NAME_BYTES: usize = 256 * 1024;
const ROOT_INVENTORY_TIMEOUT: Duration = Duration::from_millis(250);
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(5);
const CONTROL_OBJECT_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const APP_DATA_RESET_POST_EFFECT_TIMEOUT: Duration = Duration::from_millis(250);
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
const APP_DATA_RESET_OLD_DATABASE_MAX_ENTRIES: usize = 6 + SIDECAR_SUFFIXES.len();
const APP_DATA_RESET_FRESH_ORIGIN_NAME: &str = ".dux-reset-origin-v1";
const APP_DATA_RESET_FRESH_ORIGIN_MAGIC: &[u8; 16] = b"DUXRESETORIGIN1\0";
const APP_DATA_RESET_FRESH_ORIGIN_LENGTH: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetDataDetachFault {
    BeforeRename,
    AfterRename,
    AfterDirectorySync,
    DuringReadback,
    ExpireBeforeRename,
    #[allow(
        dead_code,
        reason = "constructed only by the test build's last-moment no-replace collision seam"
    )]
    RaceDestinationCollision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetFreshNamespaceFault {
    BeforeStageCreate,
    AfterStagePrepared,
    AfterFreshStageRename,
    AfterFreshStageSync,
    #[allow(
        dead_code,
        reason = "constructed only by the test build's last-moment no-replace collision seam"
    )]
    RaceCanonicalCollision,
    AfterCanonicalRename,
    AfterParentSync,
    DuringReadback,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining faults"
)]
pub(crate) enum AppDataResetOldDatabasePayloadDrainFault {
    ExpireBeforeEffect,
    BeforeEffect,
    AfterEffect,
    AfterDirectorySync,
    DuringReadback,
    ExhaustPostEffectDeadline,
}

#[cfg(test)]
pub(crate) type TestAppDataResetDataDetachFault = AppDataResetDataDetachFault;

#[cfg(test)]
std::thread_local! {
    static TEST_APP_DATA_RESET_DATA_DETACH_FAULT: Cell<Option<AppDataResetDataDetachFault>> =
        const { Cell::new(None) };
}

#[cfg(test)]
pub(crate) type TestAppDataResetFreshNamespaceFault = AppDataResetFreshNamespaceFault;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestAppDataResetSnapshotPostcheckFault {
    ExhaustBeforeOldFreshReadback,
}

#[cfg(test)]
std::thread_local! {
    static TEST_APP_DATA_RESET_FRESH_NAMESPACE_FAULT:
        Cell<Option<AppDataResetFreshNamespaceFault>> = const { Cell::new(None) };
    static TEST_APP_DATA_RESET_SNAPSHOT_POSTCHECK_FAULT:
        Cell<Option<TestAppDataResetSnapshotPostcheckFault>> = const { Cell::new(None) };
    static TEST_APP_DATA_RESET_OLD_DATABASE_PAYLOAD_DRAIN_FAULT:
        Cell<Option<AppDataResetOldDatabasePayloadDrainFault>> = const { Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_test_app_data_reset_fresh_namespace_fault(
    fault: TestAppDataResetFreshNamespaceFault,
) {
    TEST_APP_DATA_RESET_FRESH_NAMESPACE_FAULT.with(|current| current.set(Some(fault)));
}

#[cfg(test)]
pub(crate) fn set_test_app_data_reset_snapshot_postcheck_fault(
    fault: TestAppDataResetSnapshotPostcheckFault,
) {
    TEST_APP_DATA_RESET_SNAPSHOT_POSTCHECK_FAULT.with(|current| current.set(Some(fault)));
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "higher-layer reset integration adds cross-boundary fault coverage"
)]
pub(crate) fn set_test_app_data_reset_old_database_payload_drain_fault(
    fault: AppDataResetOldDatabasePayloadDrainFault,
) {
    TEST_APP_DATA_RESET_OLD_DATABASE_PAYLOAD_DRAIN_FAULT.with(|current| current.set(Some(fault)));
}

#[cfg(test)]
fn take_test_app_data_reset_snapshot_postcheck_fault(
    expected: TestAppDataResetSnapshotPostcheckFault,
) -> bool {
    TEST_APP_DATA_RESET_SNAPSHOT_POSTCHECK_FAULT.with(|current| {
        if current.get() == Some(expected) {
            current.set(None);
            true
        } else {
            false
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining faults"
)]
fn take_test_app_data_reset_old_database_payload_drain_fault(
    expected: AppDataResetOldDatabasePayloadDrainFault,
) -> bool {
    #[cfg(test)]
    {
        TEST_APP_DATA_RESET_OLD_DATABASE_PAYLOAD_DRAIN_FAULT.with(|current| {
            if current.get() == Some(expected) {
                current.set(None);
                true
            } else {
                false
            }
        })
    }
    #[cfg(not(test))]
    {
        let _ = expected;
        false
    }
}

#[cfg(test)]
fn exhaust_app_data_reset_deadline(deadline: Instant) {
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(remaining.min(Duration::from_millis(1)));
    }
}

#[cfg(test)]
fn take_test_app_data_reset_fresh_namespace_fault(
    expected: AppDataResetFreshNamespaceFault,
) -> bool {
    TEST_APP_DATA_RESET_FRESH_NAMESPACE_FAULT.with(|current| {
        if current.get() == Some(expected) {
            current.set(None);
            true
        } else {
            false
        }
    })
}

#[cfg(not(test))]
fn take_test_app_data_reset_fresh_namespace_fault(
    _expected: AppDataResetFreshNamespaceFault,
) -> bool {
    false
}

#[cfg(test)]
pub(crate) fn set_test_app_data_reset_data_detach_fault(fault: TestAppDataResetDataDetachFault) {
    TEST_APP_DATA_RESET_DATA_DETACH_FAULT.with(|current| current.set(Some(fault)));
}

#[cfg(test)]
fn take_test_app_data_reset_data_detach_fault(expected: AppDataResetDataDetachFault) -> bool {
    TEST_APP_DATA_RESET_DATA_DETACH_FAULT.with(|current| {
        if current.get() == Some(expected) {
            current.set(None);
            true
        } else {
            false
        }
    })
}

#[cfg(not(test))]
fn take_test_app_data_reset_data_detach_fault(_expected: AppDataResetDataDetachFault) -> bool {
    false
}

const SQLITE_HEADER_LENGTH: usize = 100;
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";
const SQLITE_APPLICATION_ID_OFFSET: usize = 68;
const DUX_ROOT_MARKER: &[u8; 16] = b"DUXWRITERLOCK1\0\0";
const DUX_ROOT_MARKER_LAYOUT_V2: &[u8; 16] = b"DUXSTORELAYOUT2\0";
const DUX_CLEANUP_LOCK_MARKER: &[u8; 16] = b"DUXCLEANUPLOCK1\0";
const DUX_CLEANUP_LOCK_READY_MARKER: &[u8; 16] = b"DUXCLEANREADY1\0\0";
const DUX_INITIALIZATION_SENTINEL: &[u8; 16] = b"DUXINITDONE1\0\0\0\0";

struct PreparedRoot {
    directory: File,
    identity: PlatformIdentity,
    object_path: PathBuf,
    state: PreparedRootState,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    publication_parent: File,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    publication_parent_identity: PlatformIdentity,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    _publication_fence: RootPublicationFence,
    #[cfg(windows)]
    publication_parent: Option<File>,
    #[cfg(windows)]
    publication_parent_identity: Option<PlatformIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreparedRootState {
    Existing,
    FreshStaged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootPublicationResult {
    Published,
    Collision,
}

impl PreparedRoot {
    const fn is_fresh(&self) -> bool {
        !matches!(self.state, PreparedRootState::Existing)
    }
}

/// A descriptor-backed, non-mutating inspection of a possible DUX store.
///
/// The only mutations allowed while constructing this value are atomically
/// provisioning a previously absent store, or continuing a prefix carrying
/// the immutable DUX ownership marker. An unmarked existing directory is
/// never populated, chmodded, or given a lock artifact; a SQLite application
/// ID alone does not prove ownership of its containing directory.
pub(crate) struct StoreProbe {
    root_path: PathBuf,
    database_path: PathBuf,
    lock_path: PathBuf,
    cleanup_lock_path: PathBuf,
    cleanup_lock_ready_path: PathBuf,
    root_directory: File,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    publication_parent: File,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    publication_parent_identity: PlatformIdentity,
    database_file: File,
    root_identity: PlatformIdentity,
    database_identity: PlatformIdentity,
    provenance: StoreProvenance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoreProvenance {
    FreshPrivateStore,
    ExistingDuxDatabase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootMarkerState {
    Missing,
    Valid,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootLayoutVersion {
    LegacyV1,
    CleanupLockV2,
}

impl StoreProbe {
    pub(crate) fn prepare(database_path: &Path) -> Result<Self, DatabaseOpenError> {
        Self::prepare_once(database_path, true)
    }

    fn prepare_once(
        database_path: &Path,
        retry_publication_collision: bool,
    ) -> Result<Self, DatabaseOpenError> {
        let root_path = database_path
            .parent()
            .filter(|parent| parent.parent().is_some())
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let requested_database_name = database_path
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;

        let prepared_root = platform::prepare_root_for_probe(root_path)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let publication_parent = prepared_root
            .publication_parent
            .try_clone()
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let publication_parent_identity = prepared_root.publication_parent_identity;
        let (
            root_directory,
            root_identity,
            database_name,
            database_file,
            database_identity,
            provenance,
        ) = if prepared_root.is_fresh() {
            let marker_name = lock_name(requested_database_name);
            let (marker_file, marker_identity) = platform::create_private_file_exclusive(
                &prepared_root.directory,
                &prepared_root.object_path,
                &marker_name,
            )?;
            ensure_root_marker(&marker_file)?;
            let (database_file, database_identity) = platform::create_private_file_exclusive(
                &prepared_root.directory,
                &prepared_root.object_path,
                requested_database_name,
            )?;
            database_file
                .sync_all()
                .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?;
            match platform::publish_prepared_root(
                &prepared_root,
                root_path,
                requested_database_name,
                &database_file,
                database_identity,
                &marker_name,
                &marker_file,
                marker_identity,
            )? {
                RootPublicationResult::Published => {}
                RootPublicationResult::Collision if retry_publication_collision => {
                    drop(marker_file);
                    drop(database_file);
                    drop(prepared_root);
                    return Self::prepare_once(database_path, false);
                }
                RootPublicationResult::Collision => {
                    return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
                }
            }
            drop(marker_file);
            let PreparedRoot {
                directory: publication_directory,
                identity,
                ..
            } = prepared_root;
            #[cfg(windows)]
            let directory =
                platform::reopen_published_root(publication_directory, root_path, identity)?;
            #[cfg(not(windows))]
            let directory = publication_directory;
            (
                directory,
                identity,
                requested_database_name.to_os_string(),
                database_file,
                database_identity,
                StoreProvenance::FreshPrivateStore,
            )
        } else {
            let PreparedRoot {
                directory: root_directory,
                identity: root_identity,
                ..
            } = prepared_root;
            let existing_database = platform::open_existing_file(
                &root_directory,
                root_path,
                requested_database_name,
                PermissionPolicy::InspectOnly,
            )?;
            let Some((file, identity)) = existing_database else {
                let marker_state = probe_root_marker(
                    &root_directory,
                    root_path,
                    &lock_name(requested_database_name),
                )?;
                let kind = if marker_state == RootMarkerState::Valid {
                    DatabaseOpenErrorKind::UnsafeStorageObject
                } else {
                    DatabaseOpenErrorKind::UnrecognizedDatabase
                };
                return Err(DatabaseOpenError::new(kind));
            };
            let database_name = platform::resolve_existing_file_name(
                &root_directory,
                root_path,
                requested_database_name,
                identity,
            )?;
            let marker_state =
                probe_root_marker(&root_directory, root_path, &lock_name(&database_name))?;
            let initialization_state = probe_initialization_sentinel(
                &root_directory,
                root_path,
                &initialization_name(&database_name),
            )?;
            if initialization_state == RootMarkerState::Invalid {
                return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
            }
            let database_name_ref = database_name.as_os_str();
            let length = file
                .metadata()
                .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?
                .len();
            let provenance = if length != 0 {
                match prove_dux_header(&file) {
                    Ok(()) => {
                        validate_existing_dux_store_root(
                            &root_directory,
                            root_path,
                            database_name_ref,
                            marker_state,
                        )?;
                        StoreProvenance::ExistingDuxDatabase
                    }
                    Err(_) if marker_state == RootMarkerState::Valid => {
                        if initialization_state == RootMarkerState::Valid {
                            validate_owned_store_prefix(
                                &root_directory,
                                root_path,
                                database_name_ref,
                                root_identity,
                                Some((&file, identity)),
                            )?;
                            return Err(DatabaseOpenError::new(
                                DatabaseOpenErrorKind::CorruptDatabase,
                            ));
                        }
                        validate_owned_store_prefix(
                            &root_directory,
                            root_path,
                            database_name_ref,
                            root_identity,
                            Some((&file, identity)),
                        )?;
                        StoreProvenance::FreshPrivateStore
                    }
                    Err(error) => return Err(error),
                }
            } else {
                if marker_state != RootMarkerState::Valid {
                    return Err(DatabaseOpenError::new(
                        DatabaseOpenErrorKind::UnrecognizedDatabase,
                    ));
                }
                if initialization_state == RootMarkerState::Valid {
                    validate_owned_store_prefix(
                        &root_directory,
                        root_path,
                        database_name_ref,
                        root_identity,
                        Some((&file, identity)),
                    )?;
                    return Err(DatabaseOpenError::new(
                        DatabaseOpenErrorKind::CorruptDatabase,
                    ));
                }
                validate_owned_store_prefix(
                    &root_directory,
                    root_path,
                    database_name_ref,
                    root_identity,
                    Some((&file, identity)),
                )?;
                StoreProvenance::FreshPrivateStore
            };
            (
                root_directory,
                root_identity,
                database_name,
                file,
                identity,
                provenance,
            )
        };

        let database_path = root_path.join(&database_name);
        let lock_path = root_path.join(lock_name(&database_name));
        let cleanup_lock_path = root_path.join(cleanup_lock_name(&database_name));
        let cleanup_lock_ready_path = root_path.join(cleanup_lock_ready_name(&database_name));
        let probe = Self {
            root_path: root_path.to_path_buf(),
            database_path,
            lock_path,
            cleanup_lock_path,
            cleanup_lock_ready_path,
            root_directory,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            publication_parent,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            publication_parent_identity,
            database_file,
            root_identity,
            database_identity,
            provenance,
        };
        probe.validate_probe_identity()?;
        Ok(probe)
    }

    pub(crate) fn secure(self) -> Result<SecureStorePaths, DatabaseOpenError> {
        self.secure_with_control_lock_timeout(CONTROL_OBJECT_LOCK_TIMEOUT)
    }

    fn secure_with_control_lock_timeout(
        self,
        control_lock_timeout: Duration,
    ) -> Result<SecureStorePaths, DatabaseOpenError> {
        self.validate_probe_identity()?;
        if self.provenance == StoreProvenance::ExistingDuxDatabase {
            prove_dux_header(&self.database_file)?;
        }

        platform::secure_retained_file(
            &self.root_directory,
            ObjectKind::Directory,
            self.root_identity,
        )?;
        #[cfg(windows)]
        let root_rename_guard =
            platform::open_root_rename_guard(&self.root_path, self.root_identity)?;
        platform::secure_retained_file(
            &self.database_file,
            ObjectKind::RegularFile,
            self.database_identity,
        )?;

        let database_name = self
            .database_path
            .file_name()
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        let lock_name = suffixed_name(database_name, WRITER_LOCK_SUFFIX);
        let Some((lock_file, lock_identity)) = platform::open_existing_writer_file(
            &self.root_directory,
            &self.root_path,
            &lock_name,
            PermissionPolicy::InspectOnly,
        )?
        else {
            return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
        };
        if inspect_root_marker(&lock_file)? != RootMarkerState::Valid {
            return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
        }
        platform::secure_retained_file(&lock_file, ObjectKind::RegularFile, lock_identity)?;
        platform::validate_path_identity(
            &self.lock_path,
            ObjectKind::RegularFile,
            lock_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        let provisioning_lock = acquire_advisory_lock(&lock_file, control_lock_timeout)?;
        let root_layout = inspect_root_layout(&lock_file)?
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        platform::validate_path_identity(
            &self.lock_path,
            ObjectKind::RegularFile,
            lock_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        let cleanup_lock = open_or_create_cleanup_lock(
            &self.root_directory,
            &self.root_path,
            database_name,
            &self.cleanup_lock_path,
            &self.cleanup_lock_ready_path,
            &lock_file,
            root_layout,
        )?;
        FileExt::unlock(&provisioning_lock)
            .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?;
        let initialization_sentinel = open_initialization_sentinel(
            &self.root_directory,
            &self.root_path,
            database_name,
            PermissionPolicy::RequirePrivate,
        )?;
        validate_sidecars(
            &self.root_directory,
            &self.root_path,
            database_name,
            PermissionPolicy::RepairPrivate,
        )?;

        let storage = SecureStorePaths {
            root_path: self.root_path,
            database_path: self.database_path,
            lock_path: self.lock_path,
            cleanup_lock_path: self.cleanup_lock_path,
            cleanup_lock_ready_path: self.cleanup_lock_ready_path,
            root_directory: self.root_directory,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            publication_parent: self.publication_parent,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            publication_parent_identity: self.publication_parent_identity,
            #[cfg(windows)]
            root_rename_guard,
            database_file: self.database_file,
            lock_file,
            cleanup_lock_file: cleanup_lock.file,
            cleanup_lock_ready_file: cleanup_lock.ready_file,
            initialization_sentinel: Mutex::new(initialization_sentinel),
            root_identity: self.root_identity,
            database_identity: self.database_identity,
            lock_identity,
            cleanup_lock_identity: cleanup_lock.identity,
            cleanup_lock_ready_identity: cleanup_lock.ready_identity,
            requires_initialization: self.provenance == StoreProvenance::FreshPrivateStore,
            writer_lock_in_use: Arc::new(AtomicBool::new(false)),
            cleanup_lock_in_use: Arc::new(AtomicBool::new(false)),
        };
        storage.validate_for_database_open()?;
        Ok(storage)
    }

    fn validate_probe_identity(&self) -> Result<(), DatabaseOpenError> {
        platform::validate_path_identity(
            &self.root_path,
            ObjectKind::Directory,
            self.root_identity,
            PermissionPolicy::InspectOnly,
        )?;
        platform::validate_retained_file(
            &self.database_file,
            ObjectKind::RegularFile,
            self.database_identity,
            PermissionPolicy::InspectOnly,
        )?;
        platform::validate_path_identity(
            &self.database_path,
            ObjectKind::RegularFile,
            self.database_identity,
            PermissionPolicy::InspectOnly,
        )
    }
}

/// Descriptor-backed storage objects prepared before SQLite sees a path.
///
/// The retained handles and identities let the caller reject replacement of
/// the owned directory, database, or stable writer lock between provisioning
/// and a later SQLite open. SQLite still performs its own path-based open, so
/// callers must invoke `validate_for_database_open` immediately beforehand.
pub(crate) struct SecureStorePaths {
    root_path: PathBuf,
    database_path: PathBuf,
    lock_path: PathBuf,
    cleanup_lock_path: PathBuf,
    cleanup_lock_ready_path: PathBuf,
    root_directory: File,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    publication_parent: File,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    publication_parent_identity: PlatformIdentity,
    #[cfg(windows)]
    root_rename_guard: File,
    database_file: File,
    lock_file: File,
    cleanup_lock_file: File,
    cleanup_lock_ready_file: File,
    initialization_sentinel: Mutex<Option<RetainedInitializationSentinel>>,
    root_identity: PlatformIdentity,
    database_identity: PlatformIdentity,
    lock_identity: PlatformIdentity,
    cleanup_lock_identity: PlatformIdentity,
    cleanup_lock_ready_identity: PlatformIdentity,
    requires_initialization: bool,
    writer_lock_in_use: Arc<AtomicBool>,
    cleanup_lock_in_use: Arc<AtomicBool>,
}

/// The only two namespace states accepted while resuming a journaled reset.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetRecoveryDataLocation {
    Canonical,
    Detached,
}

/// The only three fresh-root shapes accepted while `DataDetached` or
/// `FreshNamespaceReady` is durable.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetFreshNamespaceLocation {
    Absent,
    Staged,
    Canonical,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct AppDataResetFreshRoot {
    directory: File,
    identity: PlatformIdentity,
    database_file: File,
    database_identity: PlatformIdentity,
    writer_file: File,
    writer_identity: PlatformIdentity,
    cleanup_file: File,
    cleanup_identity: PlatformIdentity,
    cleanup_ready_file: File,
    cleanup_ready_identity: PlatformIdentity,
    origin_file: File,
    origin_identity: PlatformIdentity,
}

/// Exact storage-local states admitted after cache and snapshot debt are absent.
///
/// The main database remains the final payload. Once it is absent, only the
/// initialization sentinel and three retained lock controls may remain. No
/// ordinary store opener accepts the latter shape.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
pub(crate) enum AppDataResetOldDatabasePayloadState {
    DatabasePresent,
    DatabaseAbsentControlsFull,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
struct AppDataResetRetainedOldDatabaseFile {
    name: OsString,
    file: File,
    identity: PlatformIdentity,
}

/// Descriptor-retained old root backing one callback-scoped draining admission.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
struct AppDataResetDrainingOldRootState<'scope> {
    publication_parent: &'scope File,
    publication_parent_identity: PlatformIdentity,
    root_name: &'scope OsStr,
    detached_name: &'scope OsStr,
    root_path: &'scope Path,
    root_directory: File,
    root_identity: PlatformIdentity,
    database_name: &'scope OsStr,
    fresh_stage_name: &'scope OsStr,
    transaction_id: &'scope str,
    fresh: AppDataResetFreshRoot,
    database: Option<AppDataResetRetainedOldDatabaseFile>,
    initialization: AppDataResetRetainedOldDatabaseFile,
    writer: AppDataResetRetainedOldDatabaseFile,
    cleanup: AppDataResetRetainedOldDatabaseFile,
    cleanup_ready: AppDataResetRetainedOldDatabaseFile,
    snapshot_directory: Option<(File, PlatformIdentity)>,
    sidecars: Vec<AppDataResetRetainedOldDatabaseFile>,
    deadline: Instant,
}

/// Journal-sealed facts required to reopen the detached old database root.
/// Grouping them keeps the reset-only storage boundary explicit and prevents
/// individual names or identities from being reordered at call sites.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
pub(crate) struct AppDataResetOldDatabaseOpenBinding<'scope> {
    transaction_id: &'scope str,
    expected_old_identity: (u64, u64),
    expected_fresh_identity: (u64, u64),
    detached_name: &'scope OsStr,
    fresh_stage_name: &'scope OsStr,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'scope> AppDataResetOldDatabaseOpenBinding<'scope> {
    pub(crate) const fn new(
        transaction_id: &'scope str,
        expected_old_identity: (u64, u64),
        expected_fresh_identity: (u64, u64),
        detached_name: &'scope OsStr,
        fresh_stage_name: &'scope OsStr,
    ) -> Self {
        Self {
            transaction_id,
            expected_old_identity,
            expected_fresh_identity,
            detached_name,
            fresh_stage_name,
        }
    }
}

/// Complete coordinator-side authority binding for one old-database payload
/// candidate. The publication parent and canonical root are added only after
/// the opener has derived and retained them from the database path.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy)]
pub(crate) struct AppDataResetOldDatabaseAuthorityBinding<'scope> {
    open: AppDataResetOldDatabaseOpenBinding<'scope>,
    publication_parent_identity: (u64, u64),
    canonical_root_name: &'scope OsStr,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'scope> AppDataResetOldDatabaseAuthorityBinding<'scope> {
    pub(crate) const fn new(
        open: AppDataResetOldDatabaseOpenBinding<'scope>,
        publication_parent_identity: (u64, u64),
        canonical_root_name: &'scope OsStr,
    ) -> Self {
        Self {
            open,
            publication_parent_identity,
            canonical_root_name,
        }
    }
}

/// The only storage-local result of opening a detached old database payload.
/// A payload candidate is consume-once; exact absence carries validation only.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[must_use = "the old database draining admission must be consumed or revalidated"]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
pub(crate) enum AppDataResetOldDatabaseDrainingAdmission<'scope> {
    /// The bounded old root still contains its exact private snapshot
    /// directory. This no-effect state is the only authorization to resume
    /// the earlier cache/snapshot recovery pipeline.
    SnapshotStorePresent,
    PayloadsRemain(AppDataResetOldDatabasePayloadDrainCandidate<'scope>),
    Absent(AppDataResetOldDatabasePayloadAbsentWitness<'scope>),
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
enum AppDataResetOldDatabasePayloadTarget {
    Sidecar(usize),
    MainDatabase,
}

/// Consume-once selection of one exact old SQLite sidecar or the main database.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[must_use = "the old database payload candidate must enter a coordinator-bound batch"]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
pub(crate) struct AppDataResetOldDatabasePayloadDrainCandidate<'scope> {
    state: AppDataResetDrainingOldRootState<'scope>,
    target: AppDataResetOldDatabasePayloadTarget,
    _fence: &'scope RootPublicationFence,
    cleanup_guard: &'scope CleanupLockGuard,
    writer_guard: &'scope WriterLockGuard,
}

/// Exact no-effect proof that only the old root structural controls remain.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[must_use = "database absence must be joined to the old-root structural checkpoint"]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
pub(crate) struct AppDataResetOldDatabasePayloadAbsentWitness<'scope> {
    state: AppDataResetDrainingOldRootState<'scope>,
    _fence: &'scope RootPublicationFence,
    cleanup_guard: &'scope CleanupLockGuard,
    writer_guard: &'scope WriterLockGuard,
}

/// Path- and byte-free progress from one old database payload unlink.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining progress"
)]
pub(crate) struct AppDataResetOldDatabasePayloadDrainBatch {
    removed_objects: u8,
    old_database_payload_has_more: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining progress"
)]
impl AppDataResetOldDatabasePayloadDrainBatch {
    pub(crate) const fn removed_objects(self) -> u8 {
        self.removed_objects
    }

    pub(crate) const fn old_database_payload_has_more(self) -> bool {
        self.old_database_payload_has_more
    }
}

/// Exact certainty classification for one old SQLite payload removal.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining errors"
)]
pub(crate) enum AppDataResetOldDatabasePayloadDrainError {
    BeforeEffect(DatabaseOpenErrorKind),
    OutcomeUnknown,
}

/// Internal completion carrying the one fresh shared post-effect deadline.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug)]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining completion"
)]
pub(crate) struct AppDataResetOldDatabasePayloadDrainCompletion {
    progress: AppDataResetOldDatabasePayloadDrainBatch,
    post_effect_deadline: Instant,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining completion"
)]
impl AppDataResetOldDatabasePayloadDrainCompletion {
    pub(crate) const fn post_effect_deadline(&self) -> Instant {
        self.post_effect_deadline
    }

    pub(crate) fn into_progress(self) -> AppDataResetOldDatabasePayloadDrainBatch {
        self.progress
    }
}

struct RetainedInitializationSentinel {
    file: File,
    identity: PlatformIdentity,
}

struct RetainedCleanupLock {
    file: File,
    identity: PlatformIdentity,
    ready_file: File,
    ready_identity: PlatformIdentity,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn open_app_data_reset_fresh_root(
    parent: &File,
    parent_identity: PlatformIdentity,
    opened: (File, PlatformIdentity),
    namespace_path: &Path,
    namespace_name: &OsStr,
    database_name: &OsStr,
    transaction_id: &str,
    old_identity: (u64, u64),
    deadline: Instant,
) -> Result<AppDataResetFreshRoot, DatabaseOpenError> {
    if Instant::now() >= deadline {
        return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
    }
    let (directory, identity) = opened;
    if (identity.device, identity.inode) == old_identity
        || identity.device != parent_identity.device
        || platform::validate_publication_parent_identity(parent)? != parent_identity
    {
        return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
    }
    platform::validate_named_object(
        parent,
        namespace_name,
        &directory,
        identity,
        ObjectKind::Directory,
    )?;

    let open = |name: &OsStr| {
        platform::open_existing_file(
            &directory,
            namespace_path,
            name,
            PermissionPolicy::RequirePrivate,
        )?
        .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
    };
    let (database_file, database_identity) = open(database_name)?;
    if database_file
        .metadata()
        .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?
        .len()
        != 0
    {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    let writer_name = lock_name(database_name);
    let (writer_file, writer_identity) = open(&writer_name)?;
    prove_current_root_marker(&writer_file)?;
    let cleanup_name = cleanup_lock_name(database_name);
    let (cleanup_file, cleanup_identity) = open(&cleanup_name)?;
    prove_cleanup_lock_marker(&cleanup_file)?;
    let cleanup_ready_name = cleanup_lock_ready_name(database_name);
    let (cleanup_ready_file, cleanup_ready_identity) = open(&cleanup_ready_name)?;
    prove_cleanup_lock_ready_marker(&cleanup_ready_file)?;
    let origin_name = OsStr::new(APP_DATA_RESET_FRESH_ORIGIN_NAME);
    let (origin_file, origin_identity) = open(origin_name)?;
    prove_app_data_reset_fresh_origin(
        &origin_file,
        transaction_id,
        old_identity,
        (identity.device, identity.inode),
    )?;

    let allowed = [
        database_name,
        writer_name.as_os_str(),
        cleanup_name.as_os_str(),
        cleanup_ready_name.as_os_str(),
        origin_name,
    ];
    if !platform::root_contains_only_exact_until(&directory, &allowed, deadline)? {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    for (name, file, retained_identity) in [
        (database_name, &database_file, database_identity),
        (writer_name.as_os_str(), &writer_file, writer_identity),
        (cleanup_name.as_os_str(), &cleanup_file, cleanup_identity),
        (
            cleanup_ready_name.as_os_str(),
            &cleanup_ready_file,
            cleanup_ready_identity,
        ),
        (origin_name, &origin_file, origin_identity),
    ] {
        platform::validate_retained_file(
            file,
            ObjectKind::RegularFile,
            retained_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        platform::validate_named_object(
            &directory,
            name,
            file,
            retained_identity,
            ObjectKind::RegularFile,
        )?;
    }
    if Instant::now() >= deadline {
        return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
    }
    Ok(AppDataResetFreshRoot {
        directory,
        identity,
        database_file,
        database_identity,
        writer_file,
        writer_identity,
        cleanup_file,
        cleanup_identity,
        cleanup_ready_file,
        cleanup_ready_identity,
        origin_file,
        origin_identity,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::too_many_arguments)]
fn revalidate_app_data_reset_fresh_root(
    fresh: &AppDataResetFreshRoot,
    parent: &File,
    parent_identity: PlatformIdentity,
    namespace_name: &OsStr,
    database_name: &OsStr,
    transaction_id: &str,
    old_identity: (u64, u64),
    deadline: Instant,
) -> Result<(), DatabaseOpenError> {
    if Instant::now() >= deadline
        || (fresh.identity.device, fresh.identity.inode) == old_identity
        || fresh.identity.device != parent_identity.device
        || platform::validate_publication_parent_identity(parent)? != parent_identity
    {
        return Err(storage_root_error(if Instant::now() >= deadline {
            DatabaseOpenErrorKind::Busy
        } else {
            DatabaseOpenErrorKind::UnsafeStorageRoot
        }));
    }
    platform::validate_named_object(
        parent,
        namespace_name,
        &fresh.directory,
        fresh.identity,
        ObjectKind::Directory,
    )?;
    prove_current_root_marker(&fresh.writer_file)?;
    prove_cleanup_lock_marker(&fresh.cleanup_file)?;
    prove_cleanup_lock_ready_marker(&fresh.cleanup_ready_file)?;
    prove_app_data_reset_fresh_origin(
        &fresh.origin_file,
        transaction_id,
        old_identity,
        (fresh.identity.device, fresh.identity.inode),
    )?;
    if fresh
        .database_file
        .metadata()
        .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?
        .len()
        != 0
    {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    let writer_name = lock_name(database_name);
    let cleanup_name = cleanup_lock_name(database_name);
    let cleanup_ready_name = cleanup_lock_ready_name(database_name);
    let origin_name = OsStr::new(APP_DATA_RESET_FRESH_ORIGIN_NAME);
    let allowed = [
        database_name,
        writer_name.as_os_str(),
        cleanup_name.as_os_str(),
        cleanup_ready_name.as_os_str(),
        origin_name,
    ];
    if !platform::root_contains_only_exact_until(&fresh.directory, &allowed, deadline)? {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    for (name, file, identity) in [
        (database_name, &fresh.database_file, fresh.database_identity),
        (
            writer_name.as_os_str(),
            &fresh.writer_file,
            fresh.writer_identity,
        ),
        (
            cleanup_name.as_os_str(),
            &fresh.cleanup_file,
            fresh.cleanup_identity,
        ),
        (
            cleanup_ready_name.as_os_str(),
            &fresh.cleanup_ready_file,
            fresh.cleanup_ready_identity,
        ),
        (origin_name, &fresh.origin_file, fresh.origin_identity),
    ] {
        platform::validate_named_object(
            &fresh.directory,
            name,
            file,
            identity,
            ObjectKind::RegularFile,
        )?;
    }
    if Instant::now() >= deadline {
        Err(storage_root_error(DatabaseOpenErrorKind::Busy))
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
#[allow(clippy::too_many_arguments)]
fn open_app_data_reset_draining_old_root<'scope>(
    publication_parent: &'scope File,
    publication_parent_identity: PlatformIdentity,
    root_name: &'scope OsStr,
    detached_name: &'scope OsStr,
    root_path: &'scope Path,
    root_directory: File,
    root_identity: PlatformIdentity,
    database_name: &'scope OsStr,
    fresh_stage_name: &'scope OsStr,
    transaction_id: &'scope str,
    fresh: AppDataResetFreshRoot,
    deadline: Instant,
) -> Result<AppDataResetDrainingOldRootState<'scope>, DatabaseOpenError> {
    if Instant::now() >= deadline {
        return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
    }
    let root_object_path = Path::new(".");
    let open = |name: &OsStr| {
        platform::open_existing_file(
            &root_directory,
            root_object_path,
            name,
            PermissionPolicy::RequirePrivate,
        )?
        .map(|(file, identity)| AppDataResetRetainedOldDatabaseFile {
            name: name.to_os_string(),
            file,
            identity,
        })
        .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
    };
    let open_writer = |name: &OsStr| {
        platform::open_existing_writer_file(
            &root_directory,
            root_object_path,
            name,
            PermissionPolicy::RequirePrivate,
        )?
        .map(|(file, identity)| AppDataResetRetainedOldDatabaseFile {
            name: name.to_os_string(),
            file,
            identity,
        })
        .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
    };

    let initialization = open(&initialization_name(database_name))?;
    prove_initialization_sentinel(&initialization.file)?;
    let writer = open_writer(&lock_name(database_name))?;
    prove_current_root_marker(&writer.file)?;
    let cleanup = open(&cleanup_lock_name(database_name))?;
    prove_cleanup_lock_marker(&cleanup.file)?;
    let cleanup_ready = open(&cleanup_lock_ready_name(database_name))?;
    prove_cleanup_lock_ready_marker(&cleanup_ready.file)?;

    let snapshot_directory =
        platform::open_existing_private_directory(&root_directory, OsStr::new("snapshots"))?;
    if snapshot_directory
        .as_ref()
        .is_some_and(|(_, identity)| identity.device != root_identity.device)
    {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }

    let database = platform::open_existing_file(
        &root_directory,
        root_object_path,
        database_name,
        PermissionPolicy::RequirePrivate,
    )?
    .map(|(file, identity)| AppDataResetRetainedOldDatabaseFile {
        name: database_name.to_os_string(),
        file,
        identity,
    });
    if let Some(database) = database.as_ref() {
        prove_dux_header(&database.file)?;
    }

    let mut sidecars = Vec::with_capacity(SIDECAR_SUFFIXES.len());
    for suffix in SIDECAR_SUFFIXES {
        let name = suffixed_name(database_name, suffix);
        if let Some((file, identity)) = platform::open_existing_file(
            &root_directory,
            root_object_path,
            &name,
            PermissionPolicy::RequirePrivate,
        )? {
            sidecars.push(AppDataResetRetainedOldDatabaseFile {
                name,
                file,
                identity,
            });
        }
    }
    sidecars.sort_unstable_by(|left, right| {
        left.name
            .as_os_str()
            .as_bytes()
            .cmp(right.name.as_os_str().as_bytes())
    });
    if database.is_none() && !sidecars.is_empty() {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }

    let state = AppDataResetDrainingOldRootState {
        publication_parent,
        publication_parent_identity,
        root_name,
        detached_name,
        root_path,
        root_directory,
        root_identity,
        database_name,
        fresh_stage_name,
        transaction_id,
        fresh,
        database,
        initialization,
        writer,
        cleanup,
        cleanup_ready,
        snapshot_directory,
        sidecars,
        deadline,
    };
    state.revalidate_contents_until(None, deadline)?;
    Ok(state)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
impl AppDataResetDrainingOldRootState<'_> {
    fn payload_state(&self) -> AppDataResetOldDatabasePayloadState {
        if self.database.is_some() {
            AppDataResetOldDatabasePayloadState::DatabasePresent
        } else {
            AppDataResetOldDatabasePayloadState::DatabaseAbsentControlsFull
        }
    }

    fn is_bound_to(&self, binding: AppDataResetOldDatabaseAuthorityBinding<'_>) -> bool {
        binding.open.transaction_id == self.transaction_id
            && binding.open.expected_old_identity
                == (self.root_identity.device, self.root_identity.inode)
            && binding.open.expected_fresh_identity
                == (self.fresh.identity.device, self.fresh.identity.inode)
            && binding.publication_parent_identity
                == (
                    self.publication_parent_identity.device,
                    self.publication_parent_identity.inode,
                )
            && binding.canonical_root_name == self.root_name
            && binding.open.detached_name == self.detached_name
            && binding.open.fresh_stage_name == self.fresh_stage_name
    }

    fn validate_retained_named_file(
        &self,
        retained: &AppDataResetRetainedOldDatabaseFile,
    ) -> Result<(), DatabaseOpenError> {
        if retained.identity.device != self.root_identity.device {
            return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
        }
        platform::validate_retained_file(
            &retained.file,
            ObjectKind::RegularFile,
            retained.identity,
            PermissionPolicy::RequirePrivate,
        )?;
        platform::validate_named_object(
            &self.root_directory,
            &retained.name,
            &retained.file,
            retained.identity,
            ObjectKind::RegularFile,
        )
    }

    fn expected_names_without(&self, removed: Option<&OsStr>) -> Vec<OsString> {
        let mut expected = Vec::with_capacity(APP_DATA_RESET_OLD_DATABASE_MAX_ENTRIES);
        for retained in [
            self.database.as_ref(),
            Some(&self.initialization),
            Some(&self.writer),
            Some(&self.cleanup),
            Some(&self.cleanup_ready),
        ]
        .into_iter()
        .flatten()
        .chain(self.sidecars.iter())
        {
            if removed != Some(retained.name.as_os_str()) {
                expected.push(retained.name.clone());
            }
        }
        if self.snapshot_directory.is_some() {
            expected.push(OsString::from("snapshots"));
        }
        expected.sort_unstable_by(|left, right| {
            left.as_os_str()
                .as_bytes()
                .cmp(right.as_os_str().as_bytes())
        });
        expected
    }

    fn revalidate_contents_until(
        &self,
        removed: Option<&OsStr>,
        deadline: Instant,
    ) -> Result<(), DatabaseOpenError> {
        if Instant::now() >= deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        platform::validate_detached_old_data_root_until(
            self.publication_parent,
            self.publication_parent_identity,
            &self.root_directory,
            self.root_identity,
            self.detached_name,
            deadline,
        )?;
        for retained in [
            Some(&self.initialization),
            Some(&self.writer),
            Some(&self.cleanup),
            Some(&self.cleanup_ready),
        ]
        .into_iter()
        .flatten()
        {
            self.validate_retained_named_file(retained)?;
        }
        prove_initialization_sentinel(&self.initialization.file)?;
        prove_current_root_marker(&self.writer.file)?;
        prove_cleanup_lock_marker(&self.cleanup.file)?;
        prove_cleanup_lock_ready_marker(&self.cleanup_ready.file)?;

        match self.snapshot_directory.as_ref() {
            Some((directory, identity)) => {
                platform::validate_retained_file(
                    directory,
                    ObjectKind::Directory,
                    *identity,
                    PermissionPolicy::RequirePrivate,
                )?;
                platform::validate_named_object(
                    &self.root_directory,
                    OsStr::new("snapshots"),
                    directory,
                    *identity,
                    ObjectKind::Directory,
                )?;
            }
            None => {
                platform::validate_named_absence(&self.root_directory, OsStr::new("snapshots"))?
            }
        }

        if let Some(database) = self.database.as_ref() {
            if removed == Some(database.name.as_os_str()) {
                platform::validate_named_absence(&self.root_directory, &database.name)?;
            } else {
                self.validate_retained_named_file(database)?;
                prove_dux_header(&database.file)?;
            }
        } else {
            platform::validate_named_absence(&self.root_directory, self.database_name)?;
        }
        for sidecar in &self.sidecars {
            if removed == Some(sidecar.name.as_os_str()) {
                platform::validate_named_absence(&self.root_directory, &sidecar.name)?;
            } else {
                self.validate_retained_named_file(sidecar)?;
            }
        }

        let expected = self.expected_names_without(removed);
        let actual = platform::directory_entry_names_until(
            &self.root_directory,
            APP_DATA_RESET_OLD_DATABASE_MAX_ENTRIES,
            ROOT_INVENTORY_MAX_NAME_BYTES,
            deadline,
        )?;
        if actual != expected || Instant::now() >= deadline {
            return Err(object_error(if Instant::now() >= deadline {
                DatabaseOpenErrorKind::Busy
            } else {
                DatabaseOpenErrorKind::UnsafeStorageObject
            }));
        }
        Ok(())
    }

    fn revalidate_until(
        &self,
        cleanup_guard: &CleanupLockGuard,
        writer_guard: &WriterLockGuard,
        deadline: Instant,
    ) -> Result<(), DatabaseOpenError> {
        self.revalidate_contents_until(None, deadline)?;
        platform::validate_retained_file(
            &cleanup_guard.file,
            ObjectKind::RegularFile,
            self.cleanup.identity,
            PermissionPolicy::RequirePrivate,
        )?;
        platform::validate_retained_file(
            &writer_guard.file,
            ObjectKind::RegularFile,
            self.writer.identity,
            PermissionPolicy::RequirePrivate,
        )?;
        self.revalidate_fresh_namespace_until(deadline)?;
        if Instant::now() >= deadline {
            Err(storage_root_error(DatabaseOpenErrorKind::Busy))
        } else {
            Ok(())
        }
    }

    fn revalidate_fresh_namespace_until(&self, deadline: Instant) -> Result<(), DatabaseOpenError> {
        if Instant::now() >= deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        platform::validate_named_absence(self.publication_parent, self.fresh_stage_name)?;
        platform::validate_detached_data_with_fresh_namespace(
            self.publication_parent,
            self.publication_parent_identity,
            &self.root_directory,
            self.root_identity,
            &self.fresh.directory,
            self.fresh.identity,
            self.root_path,
            self.root_name,
            self.detached_name,
            deadline,
        )?;
        revalidate_app_data_reset_fresh_root(
            &self.fresh,
            self.publication_parent,
            self.publication_parent_identity,
            self.root_name,
            self.database_name,
            self.transaction_id,
            (self.root_identity.device, self.root_identity.inode),
            deadline,
        )
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
impl AppDataResetOldDatabasePayloadDrainCandidate<'_> {
    pub(crate) const fn deadline(&self) -> Instant {
        self.state.deadline
    }

    pub(crate) fn state(&self) -> AppDataResetOldDatabasePayloadState {
        self.state.payload_state()
    }

    pub(crate) fn is_bound_to(&self, binding: AppDataResetOldDatabaseAuthorityBinding<'_>) -> bool {
        self.state.is_bound_to(binding)
    }

    pub(crate) fn revalidate_until(&self, deadline: Instant) -> Result<(), DatabaseOpenError> {
        self.state
            .revalidate_until(self.cleanup_guard, self.writer_guard, deadline)
    }

    fn selected(&self) -> Result<&AppDataResetRetainedOldDatabaseFile, DatabaseOpenError> {
        match self.target {
            AppDataResetOldDatabasePayloadTarget::Sidecar(index) => self
                .state
                .sidecars
                .get(index)
                .ok_or_else(|| object_error(DatabaseOpenErrorKind::InternalState)),
            AppDataResetOldDatabasePayloadTarget::MainDatabase => self
                .state
                .database
                .as_ref()
                .ok_or_else(|| object_error(DatabaseOpenErrorKind::InternalState)),
        }
    }

    /// Remove exactly the selected sidecar or main database, synchronize the
    /// old root, and read back only the protocol-produced next shape.
    pub(crate) fn drain_one(
        self,
        authority: AppDataResetOldDatabasePayloadDrainAuthority,
    ) -> std::result::Result<
        AppDataResetOldDatabasePayloadDrainCompletion,
        AppDataResetOldDatabasePayloadDrainError,
    > {
        let pre_effect_deadline = authority.pre_effect_deadline();
        let before_effect = |error: DatabaseOpenError| {
            AppDataResetOldDatabasePayloadDrainError::BeforeEffect(error.kind)
        };
        if pre_effect_deadline != self.deadline() {
            return Err(AppDataResetOldDatabasePayloadDrainError::BeforeEffect(
                DatabaseOpenErrorKind::InternalState,
            ));
        }
        #[cfg(test)]
        if take_test_app_data_reset_old_database_payload_drain_fault(
            AppDataResetOldDatabasePayloadDrainFault::ExpireBeforeEffect,
        ) {
            exhaust_app_data_reset_deadline(pre_effect_deadline);
        }
        self.revalidate_until(pre_effect_deadline)
            .map_err(before_effect)?;
        if take_test_app_data_reset_old_database_payload_drain_fault(
            AppDataResetOldDatabasePayloadDrainFault::BeforeEffect,
        ) {
            return Err(AppDataResetOldDatabasePayloadDrainError::BeforeEffect(
                DatabaseOpenErrorKind::DatabaseUnavailable,
            ));
        }
        if Instant::now() >= pre_effect_deadline {
            return Err(AppDataResetOldDatabasePayloadDrainError::BeforeEffect(
                DatabaseOpenErrorKind::Busy,
            ));
        }

        let selected = self.selected().map_err(before_effect)?;
        let selected_name = selected.name.clone();
        platform::unlink_app_data_reset_old_database_payload_with_before_unlink(
            &self.state.root_directory,
            &selected_name,
            &selected.file,
            selected.identity,
            || {
                self.revalidate_until(pre_effect_deadline)?;
                if Instant::now() >= pre_effect_deadline {
                    Err(storage_root_error(DatabaseOpenErrorKind::Busy))
                } else {
                    Ok(())
                }
            },
        )
        .map_err(before_effect)?;
        let post_effect_deadline = Instant::now()
            .checked_add(APP_DATA_RESET_POST_EFFECT_TIMEOUT)
            .ok_or(AppDataResetOldDatabasePayloadDrainError::OutcomeUnknown)?;
        if take_test_app_data_reset_old_database_payload_drain_fault(
            AppDataResetOldDatabasePayloadDrainFault::AfterEffect,
        ) {
            return Err(AppDataResetOldDatabasePayloadDrainError::OutcomeUnknown);
        }
        platform::sync_directory(&self.state.root_directory)
            .map_err(|_| AppDataResetOldDatabasePayloadDrainError::OutcomeUnknown)?;
        if Instant::now() >= post_effect_deadline
            || take_test_app_data_reset_old_database_payload_drain_fault(
                AppDataResetOldDatabasePayloadDrainFault::AfterDirectorySync,
            )
        {
            return Err(AppDataResetOldDatabasePayloadDrainError::OutcomeUnknown);
        }
        if take_test_app_data_reset_old_database_payload_drain_fault(
            AppDataResetOldDatabasePayloadDrainFault::DuringReadback,
        ) {
            return Err(AppDataResetOldDatabasePayloadDrainError::OutcomeUnknown);
        }
        #[cfg(test)]
        if take_test_app_data_reset_old_database_payload_drain_fault(
            AppDataResetOldDatabasePayloadDrainFault::ExhaustPostEffectDeadline,
        ) {
            exhaust_app_data_reset_deadline(post_effect_deadline);
        }
        self.state
            .revalidate_contents_until(Some(&selected_name), post_effect_deadline)
            .map_err(|_| AppDataResetOldDatabasePayloadDrainError::OutcomeUnknown)?;
        self.state
            .revalidate_fresh_namespace_until(post_effect_deadline)
            .map_err(|_| AppDataResetOldDatabasePayloadDrainError::OutcomeUnknown)?;
        Ok(AppDataResetOldDatabasePayloadDrainCompletion {
            progress: AppDataResetOldDatabasePayloadDrainBatch {
                removed_objects: 1,
                old_database_payload_has_more: matches!(
                    self.target,
                    AppDataResetOldDatabasePayloadTarget::Sidecar(_)
                ),
            },
            post_effect_deadline,
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(
    dead_code,
    reason = "the persistence composition slice consumes old database draining"
)]
impl AppDataResetOldDatabasePayloadAbsentWitness<'_> {
    pub(crate) const fn deadline(&self) -> Instant {
        self.state.deadline
    }

    pub(crate) fn state(&self) -> AppDataResetOldDatabasePayloadState {
        self.state.payload_state()
    }

    pub(crate) fn is_bound_to(&self, binding: AppDataResetOldDatabaseAuthorityBinding<'_>) -> bool {
        self.state.is_bound_to(binding)
    }

    pub(crate) fn revalidate_until(&self, deadline: Instant) -> Result<(), DatabaseOpenError> {
        if self.state.payload_state()
            != AppDataResetOldDatabasePayloadState::DatabaseAbsentControlsFull
        {
            return Err(object_error(DatabaseOpenErrorKind::InternalState));
        }
        self.state
            .revalidate_until(self.cleanup_guard, self.writer_guard, deadline)
    }
}

impl SecureStorePaths {
    pub(crate) fn prepare(database_path: &Path) -> Result<Self, DatabaseOpenError> {
        StoreProbe::prepare(database_path)?.secure()
    }

    /// Retain the data-root parent publication fence for one bounded reset
    /// validation callback.
    ///
    /// The fence is acquired before callers take cleanup/database exclusion.
    /// Its higher-ranked witness cannot escape the callback even if forgotten.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn with_app_data_reset_namespace_fence_until<T>(
        &self,
        detached_name: &OsStr,
        deadline: Instant,
        operation: impl for<'scope> FnOnce(AppDataResetDataNamespaceAdmission<'scope>) -> T,
    ) -> Result<T, DatabaseOpenError> {
        let root_name = self
            .root_path
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let fence = acquire_root_publication_fence_until(
            &self.publication_parent,
            self.publication_parent_identity,
            deadline,
        )?;
        let admission = AppDataResetDataNamespaceAdmission {
            paths: self,
            root_name,
            detached_name,
            _fence: &fence,
            deadline,
            detached: false,
        };
        admission.revalidate()?;
        Ok(operation(admission))
    }

    /// Reopen an already-journaled reset's data store without provisioning,
    /// repair, migration, or any SQLite open.
    ///
    /// The publication fence is acquired before the retained cleanup and
    /// writer leases. The higher-ranked witness keeps all three authorities
    /// callback-scoped and therefore impossible to leak or forget.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn with_app_data_reset_recovery_namespace_until<T>(
        database_path: &Path,
        expected_identity: (u64, u64),
        detached_name: &OsStr,
        deadline: Instant,
        operation: impl for<'scope> FnOnce(AppDataResetRecoveryDataNamespace<'scope>) -> T,
    ) -> Result<T, DatabaseOpenError> {
        let root_path = database_path
            .parent()
            .filter(|parent| parent.parent().is_some())
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let root_name = root_path
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let parent_path = root_path
            .parent()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let (publication_parent, publication_parent_identity) =
            platform::open_existing_publication_parent(parent_path)?;
        let fence = acquire_root_publication_fence_until(
            &publication_parent,
            publication_parent_identity,
            deadline,
        )?;

        let canonical = platform::open_existing_private_directory(&publication_parent, root_name)?;
        let detached =
            platform::open_existing_private_directory(&publication_parent, detached_name)?;
        let (root_directory, root_identity, location) = match (canonical, detached) {
            (Some((directory, identity)), None) => (
                directory,
                identity,
                AppDataResetRecoveryDataLocation::Canonical,
            ),
            (None, Some((directory, identity))) => (
                directory,
                identity,
                AppDataResetRecoveryDataLocation::Detached,
            ),
            (Some(_), Some(_)) | (None, None) => {
                return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
            }
        };
        if expected_identity != (root_identity.device, root_identity.inode) {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }

        match location {
            AppDataResetRecoveryDataLocation::Canonical => {
                platform::validate_data_reset_namespace(
                    &publication_parent,
                    publication_parent_identity,
                    &root_directory,
                    root_identity,
                    root_path,
                    root_name,
                    detached_name,
                    deadline,
                )?;
            }
            AppDataResetRecoveryDataLocation::Detached => {
                platform::validate_detached_data_reset_namespace(
                    &publication_parent,
                    publication_parent_identity,
                    &root_directory,
                    root_identity,
                    root_path,
                    root_name,
                    detached_name,
                    deadline,
                )?;
            }
        }

        let paths = Self::open_existing_app_data_reset_store(
            database_path,
            root_directory,
            root_identity,
            publication_parent,
            publication_parent_identity,
        )?;
        let cleanup = paths.acquire_app_data_reset_recovery_cleanup_lock_until(deadline)?;
        let writer = paths.acquire_app_data_reset_recovery_writer_lock_until(deadline)?;
        let snapshot_store = SecureSnapshotStore::open_existing_for_app_data_reset_root(
            &paths.root_path,
            paths
                .root_directory
                .try_clone()
                .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?,
        )
        .map_err(map_snapshot_recovery_error)?;
        let snapshot_inventory = snapshot_store
            .inventory_with_writer_lease_until(deadline)
            .map_err(map_snapshot_recovery_error)?;
        snapshot_inventory
            .revalidate_complete_for_app_data_reset_until(deadline)
            .map_err(map_snapshot_recovery_error)?;
        let admission = AppDataResetRecoveryDataNamespace {
            paths: &paths,
            root_name,
            detached_name,
            _fence: &fence,
            writer: &writer,
            cleanup: &cleanup,
            snapshot: &snapshot_inventory,
            deadline,
            location,
        };
        admission.revalidate()?;
        Ok(operation(admission))
    }

    /// Reopen only the exact detached old SQLite payload while `Draining` is
    /// already durable. This path never provisions, repairs, or opens SQLite
    /// and deliberately accepts the protocol-produced controls-only tail that
    /// ordinary and earlier reset recovery openers reject.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[allow(
        dead_code,
        reason = "the persistence composition slice consumes old database draining"
    )]
    pub(super) fn with_app_data_reset_draining_old_database_until<T>(
        database_path: &Path,
        binding: AppDataResetOldDatabaseOpenBinding<'_>,
        deadline: Instant,
        operation: impl for<'scope> FnOnce(AppDataResetOldDatabaseDrainingAdmission<'scope>) -> T,
    ) -> Result<T, DatabaseOpenError> {
        let root_path = database_path
            .parent()
            .filter(|parent| parent.parent().is_some())
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let root_name = root_path
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let database_name = database_path
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        let parent_path = root_path
            .parent()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let (publication_parent, publication_parent_identity) =
            platform::open_existing_publication_parent(parent_path)?;
        let fence = acquire_root_publication_fence_until(
            &publication_parent,
            publication_parent_identity,
            deadline,
        )?;
        let (root_directory, root_identity) =
            platform::open_existing_private_directory(&publication_parent, binding.detached_name)?
                .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        if binding.expected_old_identity != (root_identity.device, root_identity.inode)
            || root_identity.device != publication_parent_identity.device
        {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        platform::validate_detached_old_data_root_until(
            &publication_parent,
            publication_parent_identity,
            &root_directory,
            root_identity,
            binding.detached_name,
            deadline,
        )?;

        let (fresh_directory, fresh_identity) =
            platform::open_existing_private_directory(&publication_parent, root_name)?
                .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        if binding.expected_fresh_identity != (fresh_identity.device, fresh_identity.inode) {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        platform::validate_named_absence(&publication_parent, binding.fresh_stage_name)?;
        let fresh = open_app_data_reset_fresh_root(
            &publication_parent,
            publication_parent_identity,
            (fresh_directory, fresh_identity),
            root_path,
            root_name,
            database_name,
            binding.transaction_id,
            binding.expected_old_identity,
            deadline,
        )?;
        platform::validate_detached_data_with_fresh_namespace(
            &publication_parent,
            publication_parent_identity,
            &root_directory,
            root_identity,
            &fresh.directory,
            fresh.identity,
            root_path,
            root_name,
            binding.detached_name,
            deadline,
        )?;

        let state = open_app_data_reset_draining_old_root(
            &publication_parent,
            publication_parent_identity,
            root_name,
            binding.detached_name,
            root_path,
            root_directory,
            root_identity,
            database_name,
            binding.fresh_stage_name,
            binding.transaction_id,
            fresh,
            deadline,
        )?;
        let cleanup_in_use = Arc::new(AtomicBool::new(true));
        let cleanup_guard =
            acquire_advisory_lock_until(&state.cleanup.file, deadline, false).map(|file| {
                CleanupLockGuard {
                    file,
                    in_use: Arc::clone(&cleanup_in_use),
                }
            })?;
        let writer_in_use = Arc::new(AtomicBool::new(true));
        let writer_guard =
            acquire_advisory_lock_until(&state.writer.file, deadline, false).map(|file| {
                WriterLockGuard {
                    file,
                    in_use: Arc::clone(&writer_in_use),
                }
            })?;
        state.revalidate_until(&cleanup_guard, &writer_guard, deadline)?;

        let admission = if state.snapshot_directory.is_some() {
            AppDataResetOldDatabaseDrainingAdmission::SnapshotStorePresent
        } else if !state.sidecars.is_empty() {
            AppDataResetOldDatabaseDrainingAdmission::PayloadsRemain(
                AppDataResetOldDatabasePayloadDrainCandidate {
                    state,
                    target: AppDataResetOldDatabasePayloadTarget::Sidecar(0),
                    _fence: &fence,
                    cleanup_guard: &cleanup_guard,
                    writer_guard: &writer_guard,
                },
            )
        } else if state.database.is_some() {
            AppDataResetOldDatabaseDrainingAdmission::PayloadsRemain(
                AppDataResetOldDatabasePayloadDrainCandidate {
                    state,
                    target: AppDataResetOldDatabasePayloadTarget::MainDatabase,
                    _fence: &fence,
                    cleanup_guard: &cleanup_guard,
                    writer_guard: &writer_guard,
                },
            )
        } else {
            AppDataResetOldDatabaseDrainingAdmission::Absent(
                AppDataResetOldDatabasePayloadAbsentWitness {
                    state,
                    _fence: &fence,
                    cleanup_guard: &cleanup_guard,
                    writer_guard: &writer_guard,
                },
            )
        };
        Ok(operation(admission))
    }

    /// Reopen the exact detached old store and the transaction-bound fresh
    /// bootstrap (if already staged or published) without opening SQLite.
    ///
    /// The data-parent publication fence remains outside the old store's
    /// cleanup/database/snapshot locks. The callback-scoped witness is the only
    /// value that can publish a missing fresh root.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn with_app_data_reset_fresh_namespace_until<T>(
        database_path: &Path,
        expected_old_identity: (u64, u64),
        detached_name: &OsStr,
        fresh_stage_name: &OsStr,
        transaction_id: &str,
        allow_snapshot_structural_tail: bool,
        deadline: Instant,
        operation: impl for<'scope> FnOnce(AppDataResetFreshNamespace<'scope>) -> T,
    ) -> Result<T, DatabaseOpenError> {
        let root_path = database_path
            .parent()
            .filter(|parent| parent.parent().is_some())
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let root_name = root_path
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let database_name = database_path
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        let parent_path = root_path
            .parent()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let (publication_parent, publication_parent_identity) =
            platform::open_existing_publication_parent(parent_path)?;
        let fence = acquire_root_publication_fence_until(
            &publication_parent,
            publication_parent_identity,
            deadline,
        )?;

        let (old_root, old_identity) =
            platform::open_existing_private_directory(&publication_parent, detached_name)?
                .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        if expected_old_identity != (old_identity.device, old_identity.inode) {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        let canonical = platform::open_existing_private_directory(&publication_parent, root_name)?;
        let staged =
            platform::open_existing_private_directory(&publication_parent, fresh_stage_name)?;
        let (fresh, location) = match (canonical, staged) {
            (None, None) => (None, AppDataResetFreshNamespaceLocation::Absent),
            (None, Some(opened)) => (
                Some(open_app_data_reset_fresh_root(
                    &publication_parent,
                    publication_parent_identity,
                    opened,
                    &root_path.with_file_name(fresh_stage_name),
                    fresh_stage_name,
                    database_name,
                    transaction_id,
                    expected_old_identity,
                    deadline,
                )?),
                AppDataResetFreshNamespaceLocation::Staged,
            ),
            (Some(opened), None) => (
                Some(open_app_data_reset_fresh_root(
                    &publication_parent,
                    publication_parent_identity,
                    opened,
                    root_path,
                    root_name,
                    database_name,
                    transaction_id,
                    expected_old_identity,
                    deadline,
                )?),
                AppDataResetFreshNamespaceLocation::Canonical,
            ),
            (Some(_), Some(_)) => {
                return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
            }
        };
        if let Some(fresh) = fresh
            .as_ref()
            .filter(|_| location == AppDataResetFreshNamespaceLocation::Canonical)
        {
            platform::validate_detached_data_with_fresh_namespace(
                &publication_parent,
                publication_parent_identity,
                &old_root,
                old_identity,
                &fresh.directory,
                fresh.identity,
                root_path,
                root_name,
                detached_name,
                deadline,
            )?;
        } else {
            platform::validate_detached_data_reset_namespace(
                &publication_parent,
                publication_parent_identity,
                &old_root,
                old_identity,
                root_path,
                root_name,
                detached_name,
                deadline,
            )?;
        }

        let paths = Self::open_existing_app_data_reset_store(
            database_path,
            old_root,
            old_identity,
            publication_parent
                .try_clone()
                .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?,
            publication_parent_identity,
        )?;
        let cleanup = paths.acquire_app_data_reset_recovery_cleanup_lock_until(deadline)?;
        let writer = paths.acquire_app_data_reset_recovery_writer_lock_until(deadline)?;
        let snapshot = AppDataResetSnapshotRecovery::open_until(
            &paths.root_path,
            paths
                .root_directory
                .try_clone()
                .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?,
            allow_snapshot_structural_tail,
            deadline,
        )
        .map_err(map_snapshot_recovery_error)?;

        let admission = AppDataResetFreshNamespace {
            old_paths: &paths,
            root_name,
            detached_name,
            fresh_stage_name,
            database_name,
            transaction_id,
            expected_old_identity,
            publication_parent: &publication_parent,
            publication_parent_identity,
            _fence: &fence,
            writer: &writer,
            cleanup: &cleanup,
            snapshot,
            deadline,
            fresh,
            location,
        };
        admission.revalidate()?;
        Ok(operation(admission))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn open_existing_app_data_reset_store(
        database_path: &Path,
        root_directory: File,
        root_identity: PlatformIdentity,
        publication_parent: File,
        publication_parent_identity: PlatformIdentity,
    ) -> Result<Self, DatabaseOpenError> {
        let root_path = database_path
            .parent()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let database_name = database_path
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        let (database_file, database_identity) = platform::open_existing_file(
            &root_directory,
            root_path,
            database_name,
            PermissionPolicy::RequirePrivate,
        )?
        .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;

        let writer_name = lock_name(database_name);
        let (lock_file, lock_identity) = platform::open_existing_writer_file(
            &root_directory,
            root_path,
            &writer_name,
            PermissionPolicy::RequirePrivate,
        )?
        .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        prove_current_root_marker(&lock_file)?;

        let cleanup_name = cleanup_lock_name(database_name);
        let (cleanup_lock_file, cleanup_lock_identity) = platform::open_existing_file(
            &root_directory,
            root_path,
            &cleanup_name,
            PermissionPolicy::RequirePrivate,
        )?
        .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        prove_cleanup_lock_marker(&cleanup_lock_file)?;

        let ready_name = cleanup_lock_ready_name(database_name);
        let (cleanup_lock_ready_file, cleanup_lock_ready_identity) = platform::open_existing_file(
            &root_directory,
            root_path,
            &ready_name,
            PermissionPolicy::RequirePrivate,
        )?
        .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        prove_cleanup_lock_ready_marker(&cleanup_lock_ready_file)?;

        let initialization_sentinel = open_initialization_sentinel(
            &root_directory,
            root_path,
            database_name,
            PermissionPolicy::RequirePrivate,
        )?
        .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        let paths = Self {
            root_path: root_path.to_path_buf(),
            database_path: database_path.to_path_buf(),
            lock_path: root_path.join(&writer_name),
            cleanup_lock_path: root_path.join(&cleanup_name),
            cleanup_lock_ready_path: root_path.join(&ready_name),
            root_directory,
            publication_parent,
            publication_parent_identity,
            database_file,
            lock_file,
            cleanup_lock_file,
            cleanup_lock_ready_file,
            initialization_sentinel: Mutex::new(Some(initialization_sentinel)),
            root_identity,
            database_identity,
            lock_identity,
            cleanup_lock_identity,
            cleanup_lock_ready_identity,
            requires_initialization: false,
            writer_lock_in_use: Arc::new(AtomicBool::new(false)),
            cleanup_lock_in_use: Arc::new(AtomicBool::new(false)),
        };
        paths.validate_app_data_reset_retained_store()?;
        Ok(paths)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn acquire_app_data_reset_recovery_cleanup_lock_until(
        &self,
        deadline: Instant,
    ) -> Result<CleanupLockGuard, DatabaseOpenError> {
        self.validate_app_data_reset_retained_store()?;
        if Instant::now() >= deadline {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }
        if self
            .cleanup_lock_in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }
        let result =
            acquire_advisory_lock_until(&self.cleanup_lock_file, deadline, false).map(|file| {
                CleanupLockGuard {
                    file,
                    in_use: Arc::clone(&self.cleanup_lock_in_use),
                }
            });
        if result.is_err() {
            self.cleanup_lock_in_use.store(false, Ordering::Release);
        }
        result
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn acquire_app_data_reset_recovery_writer_lock_until(
        &self,
        deadline: Instant,
    ) -> Result<WriterLockGuard, DatabaseOpenError> {
        self.validate_app_data_reset_retained_store()?;
        if Instant::now() >= deadline {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }
        if self
            .writer_lock_in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }
        let result = acquire_advisory_lock_until(&self.lock_file, deadline, false).map(|file| {
            WriterLockGuard {
                file,
                in_use: Arc::clone(&self.writer_lock_in_use),
            }
        });
        if result.is_err() {
            self.writer_lock_in_use.store(false, Ordering::Release);
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub(crate) const fn identity(&self) -> StoreIdentity {
        StoreIdentity(self.database_identity)
    }

    pub(crate) const fn requires_initialization(&self) -> bool {
        self.requires_initialization
    }

    pub(crate) fn sqlite_path(&self) -> Result<PathBuf, DatabaseOpenError> {
        let canonical = self
            .database_path
            .canonicalize()
            .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?;
        platform::validate_path_identity(
            &canonical,
            ObjectKind::RegularFile,
            self.database_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        Ok(canonical)
    }

    /// Observe only the exact marker-owned SQLite and stable control files.
    ///
    /// Directory metadata, snapshots, reserved future cache/log directories,
    /// and unproven provisioning stages are intentionally excluded. Callers
    /// hold the cross-process writer lease so compliant SQLite writers cannot
    /// change the measured objects; two observations reject external drift.
    pub(crate) fn observe_physical_usage(
        &self,
        writer_guard: &WriterLockGuard,
    ) -> Result<OwnedStorageUsage, DatabaseOpenError> {
        self.validate_writer_lock_guard(writer_guard)?;
        self.validate_control_objects()?;
        validate_sidecars(
            &self.root_directory,
            &self.root_path,
            self.database_name()?,
            PermissionPolicy::RequirePrivate,
        )?;

        let fixed = self.observe_fixed_physical_usage()?;
        let sidecars = self.observe_sidecar_physical_usage()?;

        self.validate_control_objects()?;
        let fixed_revalidated = self.observe_fixed_physical_usage()?;
        let sidecars_revalidated = self.observe_sidecar_physical_usage()?;
        self.validate_writer_lock_guard(writer_guard)?;
        if fixed != fixed_revalidated || sidecars != sidecars_revalidated {
            return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
        }
        checked_usage_add(
            fixed,
            sidecars
                .iter()
                .try_fold(OwnedStorageUsage::default(), |total, observation| {
                    checked_usage_add(total, observation.2)
                })?,
        )
    }

    /// Revalidate every path-based object that SQLite may open or create.
    pub(crate) fn validate_for_database_open(&self) -> Result<(), DatabaseOpenError> {
        self.validate_control_objects()?;
        let database_name = self
            .database_path
            .file_name()
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        validate_sidecars(
            &self.root_directory,
            &self.root_path,
            database_name,
            PermissionPolicy::RequirePrivate,
        )
    }

    /// Repair SQLite-created sidecars only after the retained immutable marker
    /// and every control object still prove this is the same DUX-owned store.
    pub(crate) fn repair_sqlite_sidecars(&self) -> Result<(), DatabaseOpenError> {
        self.validate_control_objects()?;
        prove_current_root_marker(&self.lock_file)?;
        let database_name = self
            .database_path
            .file_name()
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        validate_sidecars(
            &self.root_directory,
            &self.root_path,
            database_name,
            PermissionPolicy::RepairPrivate,
        )?;
        validate_sidecars(
            &self.root_directory,
            &self.root_path,
            database_name,
            PermissionPolicy::RequirePrivate,
        )
    }

    /// Report a retained rollback journal or WAL requiring a recovery-capable
    /// open. Callers hold the writer lease and repair marker-authorized SQLite
    /// sidecars before this check.
    pub(crate) fn recovery_artifact_exists(&self) -> Result<bool, DatabaseOpenError> {
        self.validate_control_objects()?;
        prove_current_root_marker(&self.lock_file)?;
        let database_name = self
            .database_path
            .file_name()
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        for suffix in ["-journal", "-wal"] {
            let artifact_name = suffixed_name(database_name, suffix);
            let Some((artifact, identity)) = platform::open_existing_file(
                &self.root_directory,
                &self.root_path,
                &artifact_name,
                PermissionPolicy::RequirePrivate,
            )?
            else {
                continue;
            };
            platform::validate_retained_file(
                &artifact,
                ObjectKind::RegularFile,
                identity,
                PermissionPolicy::RequirePrivate,
            )?;
            platform::validate_path_identity(
                &self.root_path.join(artifact_name),
                ObjectKind::RegularFile,
                identity,
                PermissionPolicy::RequirePrivate,
            )?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Durably record that this marker-owned database completed migration and
    /// WAL setup. Creation is descriptor-relative and exclusive; callers hold
    /// the stable writer lease for the whole operation.
    pub(crate) fn mark_initialized(&self) -> Result<(), DatabaseOpenError> {
        self.validate_control_objects()?;
        let database_name = self
            .database_path
            .file_name()
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        let mut retained = self
            .initialization_sentinel
            .lock()
            .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?;
        if let Some(sentinel) = retained.as_ref() {
            prove_initialization_sentinel(&sentinel.file)?;
            return Ok(());
        }

        if let Some(existing) = open_initialization_sentinel(
            &self.root_directory,
            &self.root_path,
            database_name,
            PermissionPolicy::RequirePrivate,
        )? {
            *retained = Some(existing);
            return Ok(());
        }

        let name = initialization_name(database_name);
        let (file, identity) =
            platform::create_private_file_exclusive(&self.root_directory, &self.root_path, &name)?;
        ensure_initialization_sentinel(&file)?;
        platform::validate_path_identity(
            &self.root_path.join(&name),
            ObjectKind::RegularFile,
            identity,
            PermissionPolicy::RequirePrivate,
        )?;
        platform::sync_directory(&self.root_directory)?;
        *retained = Some(RetainedInitializationSentinel { file, identity });
        Ok(())
    }

    fn validate_control_objects(&self) -> Result<(), DatabaseOpenError> {
        let database_name = self
            .database_path
            .file_name()
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        #[cfg(windows)]
        platform::validate_retained_file(
            &self.root_rename_guard,
            ObjectKind::Directory,
            self.root_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        platform::validate_path_identity(
            &self.root_path,
            ObjectKind::Directory,
            self.root_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        platform::validate_retained_file(
            &self.database_file,
            ObjectKind::RegularFile,
            self.database_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        platform::validate_path_identity(
            &self.database_path,
            ObjectKind::RegularFile,
            self.database_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        prove_current_root_marker(&self.lock_file)?;
        platform::validate_retained_file(
            &self.lock_file,
            ObjectKind::RegularFile,
            self.lock_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        platform::validate_path_identity(
            &self.lock_path,
            ObjectKind::RegularFile,
            self.lock_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        prove_cleanup_lock_marker(&self.cleanup_lock_file)?;
        platform::validate_retained_file(
            &self.cleanup_lock_file,
            ObjectKind::RegularFile,
            self.cleanup_lock_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        platform::validate_path_identity(
            &self.cleanup_lock_path,
            ObjectKind::RegularFile,
            self.cleanup_lock_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        prove_cleanup_lock_ready_marker(&self.cleanup_lock_ready_file)?;
        platform::validate_retained_file(
            &self.cleanup_lock_ready_file,
            ObjectKind::RegularFile,
            self.cleanup_lock_ready_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        platform::validate_path_identity(
            &self.cleanup_lock_ready_path,
            ObjectKind::RegularFile,
            self.cleanup_lock_ready_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        if let Some(sentinel) = self
            .initialization_sentinel
            .lock()
            .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?
            .as_ref()
        {
            prove_initialization_sentinel(&sentinel.file)?;
            platform::validate_retained_file(
                &sentinel.file,
                ObjectKind::RegularFile,
                sentinel.identity,
                PermissionPolicy::RequirePrivate,
            )?;
            platform::validate_path_identity(
                &self.root_path.join(initialization_name(database_name)),
                ObjectKind::RegularFile,
                sentinel.identity,
                PermissionPolicy::RequirePrivate,
            )?;
        }
        validate_store_inventory(&self.root_directory, &self.root_path, database_name)
    }

    /// Validate the complete reset-owned store only through retained
    /// descriptors and names beneath the retained root. Unlike ordinary open
    /// validation, this never consults the former canonical root path and
    /// never repairs SQLite sidecars, so it remains valid after an exact root
    /// detach without causing a new filesystem effect.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn validate_app_data_reset_retained_store(&self) -> Result<(), DatabaseOpenError> {
        let database_name = self.database_name()?;
        platform::validate_retained_file(
            &self.root_directory,
            ObjectKind::Directory,
            self.root_identity,
            PermissionPolicy::RequirePrivate,
        )?;

        for (name, file, identity) in [
            (database_name, &self.database_file, self.database_identity),
            (
                self.lock_path
                    .file_name()
                    .ok_or_else(|| object_error(DatabaseOpenErrorKind::InternalState))?,
                &self.lock_file,
                self.lock_identity,
            ),
            (
                self.cleanup_lock_path
                    .file_name()
                    .ok_or_else(|| object_error(DatabaseOpenErrorKind::InternalState))?,
                &self.cleanup_lock_file,
                self.cleanup_lock_identity,
            ),
            (
                self.cleanup_lock_ready_path
                    .file_name()
                    .ok_or_else(|| object_error(DatabaseOpenErrorKind::InternalState))?,
                &self.cleanup_lock_ready_file,
                self.cleanup_lock_ready_identity,
            ),
        ] {
            platform::validate_retained_file(
                file,
                ObjectKind::RegularFile,
                identity,
                PermissionPolicy::RequirePrivate,
            )?;
            platform::validate_named_object(
                &self.root_directory,
                name,
                file,
                identity,
                ObjectKind::RegularFile,
            )?;
        }
        prove_dux_header(&self.database_file)?;
        prove_current_root_marker(&self.lock_file)?;
        prove_cleanup_lock_marker(&self.cleanup_lock_file)?;
        prove_cleanup_lock_ready_marker(&self.cleanup_lock_ready_file)?;

        if let Some(sentinel) = self
            .initialization_sentinel
            .lock()
            .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?
            .as_ref()
        {
            let name = initialization_name(database_name);
            platform::validate_retained_file(
                &sentinel.file,
                ObjectKind::RegularFile,
                sentinel.identity,
                PermissionPolicy::RequirePrivate,
            )?;
            platform::validate_named_object(
                &self.root_directory,
                &name,
                &sentinel.file,
                sentinel.identity,
                ObjectKind::RegularFile,
            )?;
            prove_initialization_sentinel(&sentinel.file)?;
        }
        validate_sidecars(
            &self.root_directory,
            &self.root_path,
            database_name,
            PermissionPolicy::RequirePrivate,
        )?;

        let allowed = allowed_store_entry_names(database_name, lock_name(database_name));
        let allowed_refs: Vec<&OsStr> = allowed.iter().map(OsString::as_os_str).collect();
        if platform::root_contains_only_exact(&self.root_directory, &allowed_refs)? {
            Ok(())
        } else {
            Err(DatabaseOpenError::new(
                DatabaseOpenErrorKind::UnrecognizedDatabase,
            ))
        }
    }

    pub(crate) fn validate_all_existing(&self) -> Result<(), DatabaseOpenError> {
        self.validate_for_database_open()
    }

    /// Acquire the cross-process writer/migration lease within a fixed bound.
    pub(crate) fn acquire_writer_lock(
        &self,
        timeout: Duration,
    ) -> Result<WriterLockGuard, DatabaseOpenError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?;
        self.acquire_writer_lock_until_mode(deadline, true)
    }

    pub(crate) fn acquire_writer_lock_until(
        &self,
        deadline: Instant,
    ) -> Result<WriterLockGuard, DatabaseOpenError> {
        self.acquire_writer_lock_until_mode(deadline, false)
    }

    fn acquire_writer_lock_until_mode(
        &self,
        deadline: Instant,
        allow_expired_initial_try: bool,
    ) -> Result<WriterLockGuard, DatabaseOpenError> {
        if !allow_expired_initial_try && Instant::now() >= deadline {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }
        if self
            .writer_lock_in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }

        let result =
            acquire_advisory_lock_until(&self.lock_file, deadline, allow_expired_initial_try).map(
                |file| WriterLockGuard {
                    file,
                    in_use: Arc::clone(&self.writer_lock_in_use),
                },
            );
        if result.is_err() {
            self.writer_lock_in_use.store(false, Ordering::Release);
        }
        result
    }

    fn validate_writer_lock_guard(&self, guard: &WriterLockGuard) -> Result<(), DatabaseOpenError> {
        if !Arc::ptr_eq(&self.writer_lock_in_use, &guard.in_use) {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState));
        }
        platform::validate_retained_file(
            &guard.file,
            ObjectKind::RegularFile,
            self.lock_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        self.validate_control_objects()
    }

    /// Acquire store-wide cross-process exclusion for cleanup effects.
    ///
    /// This permanent OS lock has no expiry and cannot be stolen. The guard is
    /// deliberately only an exclusion primitive: it carries no plan, journal
    /// owner, recovery generation, target identity, or effect authority.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "cleanup execution integrates this storage-only lock in a later slice"
        )
    )]
    pub(crate) fn acquire_cleanup_lock(
        &self,
        timeout: Duration,
    ) -> Result<CleanupLockGuard, DatabaseOpenError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?;
        self.acquire_cleanup_lock_until_mode(deadline, true)
    }

    pub(crate) fn acquire_cleanup_lock_until(
        &self,
        deadline: Instant,
    ) -> Result<CleanupLockGuard, DatabaseOpenError> {
        self.acquire_cleanup_lock_until_mode(deadline, false)
    }

    fn acquire_cleanup_lock_until_mode(
        &self,
        deadline: Instant,
        allow_expired_initial_try: bool,
    ) -> Result<CleanupLockGuard, DatabaseOpenError> {
        if !allow_expired_initial_try && Instant::now() >= deadline {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }
        self.validate_control_objects()?;
        if self
            .cleanup_lock_in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }

        let result = acquire_advisory_lock_until(
            &self.cleanup_lock_file,
            deadline,
            allow_expired_initial_try,
        )
        .map(|file| CleanupLockGuard {
            file,
            in_use: Arc::clone(&self.cleanup_lock_in_use),
        });
        if result.is_err() {
            self.cleanup_lock_in_use.store(false, Ordering::Release);
        }
        let guard = result?;
        if let Err(error) = self.validate_cleanup_lock_guard(&guard) {
            drop(guard);
            return Err(error);
        }
        if !allow_expired_initial_try && Instant::now() >= deadline {
            drop(guard);
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }
        Ok(guard)
    }

    /// Revalidate that this guard still refers to the retained cleanup lock.
    /// Future executors must call this immediately before every OS effect;
    /// success proves exclusion only and never target or cleanup authority.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "cleanup execution integrates held-lock revalidation in a later slice"
        )
    )]
    pub(crate) fn validate_cleanup_lock_guard(
        &self,
        guard: &CleanupLockGuard,
    ) -> Result<(), DatabaseOpenError> {
        if !Arc::ptr_eq(&self.cleanup_lock_in_use, &guard.in_use) {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState));
        }
        platform::validate_retained_file(
            &guard.file,
            ObjectKind::RegularFile,
            self.cleanup_lock_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        self.validate_control_objects()
    }

    /// Revalidate the exact retained reset locks and store after the canonical
    /// root name has been detached. This is intentionally descriptor-relative
    /// and performs no path repair or SQLite operation.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn validate_app_data_reset_detached_guards(
        &self,
        writer: &WriterLockGuard,
        cleanup: &CleanupLockGuard,
    ) -> Result<(), DatabaseOpenError> {
        if !Arc::ptr_eq(&self.writer_lock_in_use, &writer.in_use)
            || !Arc::ptr_eq(&self.cleanup_lock_in_use, &cleanup.in_use)
        {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState));
        }
        platform::validate_retained_file(
            &writer.file,
            ObjectKind::RegularFile,
            self.lock_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        platform::validate_retained_file(
            &cleanup.file,
            ObjectKind::RegularFile,
            self.cleanup_lock_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        self.validate_app_data_reset_retained_store()
    }

    fn database_name(&self) -> Result<&OsStr, DatabaseOpenError> {
        self.database_path
            .file_name()
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
    }

    fn observe_fixed_physical_usage(&self) -> Result<OwnedStorageUsage, DatabaseOpenError> {
        let mut total = OwnedStorageUsage::default();
        for file in [
            &self.database_file,
            &self.lock_file,
            &self.cleanup_lock_file,
            &self.cleanup_lock_ready_file,
        ] {
            total = checked_usage_add(total, observe_file_usage(file)?)?;
        }
        if let Some(sentinel) = self
            .initialization_sentinel
            .lock()
            .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?
            .as_ref()
        {
            total = checked_usage_add(total, observe_file_usage(&sentinel.file)?)?;
        }
        Ok(total)
    }

    fn observe_sidecar_physical_usage(
        &self,
    ) -> Result<Vec<(OsString, PlatformIdentity, OwnedStorageUsage)>, DatabaseOpenError> {
        let mut observations = Vec::with_capacity(SIDECAR_SUFFIXES.len());
        let database_name = self.database_name()?;
        for suffix in SIDECAR_SUFFIXES {
            let name = suffixed_name(database_name, suffix);
            let Some((file, identity)) = platform::open_existing_file(
                &self.root_directory,
                &self.root_path,
                &name,
                PermissionPolicy::RequirePrivate,
            )?
            else {
                continue;
            };
            platform::validate_retained_file(
                &file,
                ObjectKind::RegularFile,
                identity,
                PermissionPolicy::RequirePrivate,
            )?;
            let usage = observe_file_usage(&file)?;
            platform::validate_path_identity(
                &self.root_path.join(&name),
                ObjectKind::RegularFile,
                identity,
                PermissionPolicy::RequirePrivate,
            )?;
            observations.push((name, identity, usage));
        }
        Ok(observations)
    }

    #[cfg(test)]
    fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    #[cfg(test)]
    fn cleanup_lock_path(&self) -> &Path {
        &self.cleanup_lock_path
    }
}

fn observe_file_usage(file: &File) -> Result<OwnedStorageUsage, DatabaseOpenError> {
    let (logical_bytes, allocated_bytes) = platform::file_usage(file)?;
    Ok(OwnedStorageUsage::from_sizes(
        logical_bytes,
        allocated_bytes,
    ))
}

fn checked_usage_add(
    left: OwnedStorageUsage,
    right: OwnedStorageUsage,
) -> Result<OwnedStorageUsage, DatabaseOpenError> {
    left.checked_add(right)
        .map_err(|_| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
}

/// RAII guard for the stable advisory writer lock.
pub(crate) struct WriterLockGuard {
    file: File,
    in_use: Arc<AtomicBool>,
}

impl Drop for WriterLockGuard {
    fn drop(&mut self) {
        // A failed unlock leaves this storage instance permanently busy. The
        // file close remains the final release mechanism, which is safer than
        // allowing another same-process acquisition on an uncertain lock.
        if FileExt::unlock(&self.file).is_ok() {
            self.in_use.store(false, Ordering::Release);
        }
    }
}

/// RAII guard for the permanent store-wide cleanup-effect lock.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "cleanup execution integrates this storage-only guard in a later slice"
    )
)]
pub(crate) struct CleanupLockGuard {
    file: File,
    in_use: Arc<AtomicBool>,
}

impl Drop for CleanupLockGuard {
    fn drop(&mut self) {
        // Never advertise same-process availability after an uncertain unlock.
        // Closing the descriptor remains the final non-stealable release path.
        if FileExt::unlock(&self.file).is_ok() {
            self.in_use.store(false, Ordering::Release);
        }
    }
}

fn acquire_advisory_lock(file: &File, timeout: Duration) -> Result<File, DatabaseOpenError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?;
    acquire_advisory_lock_until(file, deadline, true)
}

fn acquire_advisory_lock_until(
    file: &File,
    deadline: Instant,
    allow_expired_initial_try: bool,
) -> Result<File, DatabaseOpenError> {
    let lock_file = file
        .try_clone()
        .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::DatabaseUnavailable))?;
    let mut first_attempt = true;
    loop {
        if Instant::now() >= deadline && !(allow_expired_initial_try && first_attempt) {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }
        first_attempt = false;
        match FileExt::try_lock(&lock_file) {
            Ok(()) if !allow_expired_initial_try && Instant::now() >= deadline => {
                let _ = FileExt::unlock(&lock_file);
                return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
            }
            Ok(()) => return Ok(lock_file),
            Err(TryLockError::WouldBlock) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
                }
                std::thread::sleep(LOCK_RETRY_INTERVAL.min(deadline.duration_since(now)));
            }
            Err(TryLockError::Error(_)) => {
                return Err(DatabaseOpenError::new(
                    DatabaseOpenErrorKind::DatabaseUnavailable,
                ));
            }
        }
    }
}

fn open_or_create_cleanup_lock(
    root_directory: &File,
    root_path: &Path,
    database_name: &OsStr,
    cleanup_path: &Path,
    ready_path: &Path,
    root_marker: &File,
    root_layout: RootLayoutVersion,
) -> Result<RetainedCleanupLock, DatabaseOpenError> {
    let cleanup_name = cleanup_lock_name(database_name);
    let ready_name = cleanup_lock_ready_name(database_name);
    if cleanup_path != root_path.join(&cleanup_name) || ready_path != root_path.join(&ready_name) {
        return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState));
    }
    validate_store_inventory(root_directory, root_path, database_name)?;
    if inspect_root_layout(root_marker)? != Some(root_layout) {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }

    let ready = open_valid_control_marker(
        root_directory,
        root_path,
        &ready_name,
        ready_path,
        DUX_CLEANUP_LOCK_READY_MARKER,
    )?;
    if let Some((ready_file, ready_identity)) = ready {
        let Some((file, identity)) = open_valid_control_marker(
            root_directory,
            root_path,
            &cleanup_name,
            cleanup_path,
            DUX_CLEANUP_LOCK_MARKER,
        )?
        else {
            // Once the durable layout checkpoint exists, recreating a missing
            // lock could produce a second inode while an old process still
            // holds the original. Missing means unsafe, never "repair".
            return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
        };
        validate_store_inventory(root_directory, root_path, database_name)?;
        upgrade_root_layout_marker(root_marker, root_layout)?;
        return Ok(RetainedCleanupLock {
            file,
            identity,
            ready_file,
            ready_identity,
        });
    }

    if root_layout == RootLayoutVersion::CleanupLockV2 {
        // The versioned root-ownership marker is the non-recreatable layout
        // anchor. A current layout missing its ready control is tampered or
        // corrupt even if the cleanup leaf is also missing.
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }

    let (file, identity) = match open_valid_control_marker(
        root_directory,
        root_path,
        &cleanup_name,
        cleanup_path,
        DUX_CLEANUP_LOCK_MARKER,
    )? {
        Some(existing) => existing,
        None => create_durable_control_marker(
            root_directory,
            root_path,
            &cleanup_name,
            cleanup_path,
            DUX_CLEANUP_LOCK_MARKER,
        )?,
    };

    // Directory durability for the lock is established before publishing the
    // ready marker. That ordering makes every crash state either resumable
    // (ready absent) or fail-closed (ready present but lock absent).
    let (ready_file, ready_identity) = create_durable_control_marker(
        root_directory,
        root_path,
        &ready_name,
        ready_path,
        DUX_CLEANUP_LOCK_READY_MARKER,
    )?;
    validate_store_inventory(root_directory, root_path, database_name)?;
    upgrade_root_layout_marker(root_marker, root_layout)?;
    Ok(RetainedCleanupLock {
        file,
        identity,
        ready_file,
        ready_identity,
    })
}

fn validate_store_inventory(
    root_directory: &File,
    root_path: &Path,
    database_name: &OsStr,
) -> Result<(), DatabaseOpenError> {
    let allowed = allowed_store_entry_names(database_name, lock_name(database_name));
    let allowed_refs: Vec<&OsStr> = allowed.iter().map(OsString::as_os_str).collect();
    if platform::root_contains_only(root_directory, root_path, &allowed_refs)? {
        Ok(())
    } else {
        Err(DatabaseOpenError::new(
            DatabaseOpenErrorKind::UnrecognizedDatabase,
        ))
    }
}

fn open_valid_control_marker(
    root_directory: &File,
    root_path: &Path,
    name: &OsStr,
    path: &Path,
    marker: &[u8; 16],
) -> Result<Option<(File, PlatformIdentity)>, DatabaseOpenError> {
    let Some((file, identity)) = platform::open_existing_control_file(
        root_directory,
        root_path,
        name,
        PermissionPolicy::RequirePrivate,
    )?
    else {
        return Ok(None);
    };
    prove_exact_marker(&file, marker)?;
    platform::validate_path_identity(
        path,
        ObjectKind::RegularFile,
        identity,
        PermissionPolicy::RequirePrivate,
    )?;
    Ok(Some((file, identity)))
}

fn create_durable_control_marker(
    root_directory: &File,
    root_path: &Path,
    name: &OsStr,
    path: &Path,
    marker: &[u8; 16],
) -> Result<(File, PlatformIdentity), DatabaseOpenError> {
    let (created, created_identity) =
        platform::create_private_file_exclusive(root_directory, root_path, name)?;
    ensure_exact_marker(&created, marker)?;
    platform::validate_path_identity(
        path,
        ObjectKind::RegularFile,
        created_identity,
        PermissionPolicy::RequirePrivate,
    )?;
    platform::sync_directory(root_directory)?;

    let Some((reopened, reopened_identity)) = platform::open_existing_control_file(
        root_directory,
        root_path,
        name,
        PermissionPolicy::RequirePrivate,
    )?
    else {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    };
    if reopened_identity != created_identity {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    prove_exact_marker(&reopened, marker)?;
    drop(created);
    Ok((reopened, reopened_identity))
}

#[derive(Clone, Copy)]
enum ObjectKind {
    Directory,
    RegularFile,
}

#[derive(Clone, Copy)]
enum PermissionPolicy {
    InspectOnly,
    RepairPrivate,
    RequirePrivate,
}

fn validate_sidecars(
    root_directory: &File,
    root_path: &Path,
    database_name: &OsStr,
    permissions: PermissionPolicy,
) -> Result<(), DatabaseOpenError> {
    for suffix in SIDECAR_SUFFIXES {
        let name = suffixed_name(database_name, suffix);
        if let Some((file, identity)) =
            platform::open_existing_file(root_directory, root_path, &name, permissions)?
        {
            platform::validate_retained_file(
                &file,
                ObjectKind::RegularFile,
                identity,
                permissions,
            )?;
        }
    }
    Ok(())
}

fn validate_owned_store_prefix(
    root_directory: &File,
    root_path: &Path,
    database_name: &OsStr,
    root_identity: PlatformIdentity,
    database: Option<(&File, PlatformIdentity)>,
) -> Result<(), DatabaseOpenError> {
    platform::validate_retained_file(
        root_directory,
        ObjectKind::Directory,
        root_identity,
        PermissionPolicy::RequirePrivate,
    )?;
    if let Some((file, identity)) = database {
        platform::validate_retained_file(
            file,
            ObjectKind::RegularFile,
            identity,
            PermissionPolicy::RequirePrivate,
        )?;
    }

    let marker_name = lock_name(database_name);
    let Some((marker, marker_identity)) = platform::open_existing_file(
        root_directory,
        root_path,
        &marker_name,
        PermissionPolicy::RequirePrivate,
    )?
    else {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    };
    platform::validate_retained_file(
        &marker,
        ObjectKind::RegularFile,
        marker_identity,
        PermissionPolicy::RequirePrivate,
    )?;
    if inspect_root_marker(&marker)? != RootMarkerState::Valid {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    open_initialization_sentinel(
        root_directory,
        root_path,
        database_name,
        PermissionPolicy::RequirePrivate,
    )?;
    validate_sidecars(
        root_directory,
        root_path,
        database_name,
        PermissionPolicy::RequirePrivate,
    )?;

    let allowed = allowed_store_entry_names(database_name, marker_name);
    let allowed_refs: Vec<&OsStr> = allowed.iter().map(OsString::as_os_str).collect();
    if !platform::root_contains_only(root_directory, root_path, &allowed_refs)? {
        return Err(DatabaseOpenError::new(
            DatabaseOpenErrorKind::UnrecognizedDatabase,
        ));
    }
    Ok(())
}

fn validate_existing_dux_store_root(
    root_directory: &File,
    root_path: &Path,
    database_name: &OsStr,
    marker_state: RootMarkerState,
) -> Result<(), DatabaseOpenError> {
    if marker_state != RootMarkerState::Valid {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    validate_sidecars(
        root_directory,
        root_path,
        database_name,
        PermissionPolicy::InspectOnly,
    )?;

    let marker_name = lock_name(database_name);
    let allowed = allowed_store_entry_names(database_name, marker_name);
    let allowed_refs: Vec<&OsStr> = allowed.iter().map(OsString::as_os_str).collect();
    if !platform::root_contains_only(root_directory, root_path, &allowed_refs)? {
        return Err(DatabaseOpenError::new(
            DatabaseOpenErrorKind::UnrecognizedDatabase,
        ));
    }
    Ok(())
}

fn allowed_store_entry_names(database_name: &OsStr, marker_name: OsString) -> Vec<OsString> {
    let mut allowed = vec![
        database_name.to_os_string(),
        marker_name,
        cleanup_lock_name(database_name),
        cleanup_lock_ready_name(database_name),
        initialization_name(database_name),
    ];
    allowed.extend(
        SIDECAR_SUFFIXES
            .into_iter()
            .map(|suffix| suffixed_name(database_name, suffix)),
    );
    allowed.extend(RESERVED_APP_SUPPORT_ENTRIES.into_iter().map(OsString::from));
    allowed
}

fn is_canonical_snapshot_stage_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(suffix) = name.strip_prefix(SNAPSHOT_STAGE_PREFIX) else {
        return false;
    };
    suffix.len() == SNAPSHOT_STAGE_SUFFIX_LENGTH
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn prove_dux_header(file: &File) -> Result<(), DatabaseOpenError> {
    let mut header = [0_u8; SQLITE_HEADER_LENGTH];
    platform::read_exact_at(file, &mut header, 0)
        .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::UnrecognizedDatabase))?;
    let page_size = u16::from_be_bytes([header[16], header[17]]);
    let valid_page_size =
        page_size == 1 || ((512..=32_768).contains(&page_size) && page_size.is_power_of_two());
    let application_id = u32::from_be_bytes(
        header[SQLITE_APPLICATION_ID_OFFSET..SQLITE_APPLICATION_ID_OFFSET + 4]
            .try_into()
            .expect("fixed SQLite header range"),
    );
    if &header[..SQLITE_MAGIC.len()] != SQLITE_MAGIC
        || !valid_page_size
        || !matches!(header[18], 1 | 2)
        || !matches!(header[19], 1 | 2)
        || application_id != super::migrations::DUX_APPLICATION_ID
    {
        return Err(DatabaseOpenError::new(
            DatabaseOpenErrorKind::UnrecognizedDatabase,
        ));
    }
    Ok(())
}

fn suffixed_name(name: &OsStr, suffix: &str) -> OsString {
    let mut result = name.to_os_string();
    result.push(suffix);
    result
}

fn lock_name(database_name: &OsStr) -> OsString {
    suffixed_name(database_name, WRITER_LOCK_SUFFIX)
}

fn cleanup_lock_name(database_name: &OsStr) -> OsString {
    suffixed_name(database_name, CLEANUP_LOCK_SUFFIX)
}

fn cleanup_lock_ready_name(database_name: &OsStr) -> OsString {
    suffixed_name(database_name, CLEANUP_LOCK_READY_SUFFIX)
}

fn initialization_name(database_name: &OsStr) -> OsString {
    suffixed_name(database_name, INITIALIZATION_SENTINEL_SUFFIX)
}

fn probe_root_marker(
    root_directory: &File,
    root_path: &Path,
    name: &OsStr,
) -> Result<RootMarkerState, DatabaseOpenError> {
    let Some((file, _)) = platform::open_existing_file(
        root_directory,
        root_path,
        name,
        PermissionPolicy::InspectOnly,
    )?
    else {
        return Ok(RootMarkerState::Missing);
    };
    inspect_root_marker(&file)
}

fn probe_initialization_sentinel(
    root_directory: &File,
    root_path: &Path,
    name: &OsStr,
) -> Result<RootMarkerState, DatabaseOpenError> {
    let Some((file, _)) = platform::open_existing_file(
        root_directory,
        root_path,
        name,
        PermissionPolicy::InspectOnly,
    )?
    else {
        return Ok(RootMarkerState::Missing);
    };
    inspect_exact_marker(&file, DUX_INITIALIZATION_SENTINEL)
}

fn open_initialization_sentinel(
    root_directory: &File,
    root_path: &Path,
    database_name: &OsStr,
    permissions: PermissionPolicy,
) -> Result<Option<RetainedInitializationSentinel>, DatabaseOpenError> {
    let name = initialization_name(database_name);
    let Some((file, identity)) =
        platform::open_existing_file(root_directory, root_path, &name, permissions)?
    else {
        return Ok(None);
    };
    prove_initialization_sentinel(&file)?;
    platform::validate_retained_file(&file, ObjectKind::RegularFile, identity, permissions)?;
    Ok(Some(RetainedInitializationSentinel { file, identity }))
}

fn ensure_root_marker(file: &File) -> Result<(), DatabaseOpenError> {
    ensure_exact_marker(file, DUX_ROOT_MARKER)?;
    prove_root_marker(file)
}

fn ensure_initialization_sentinel(file: &File) -> Result<(), DatabaseOpenError> {
    ensure_exact_marker(file, DUX_INITIALIZATION_SENTINEL)?;
    prove_initialization_sentinel(file)
}

fn app_data_reset_fresh_origin_bytes(
    transaction_id: &str,
    old_identity: (u64, u64),
    fresh_identity: (u64, u64),
) -> Result<[u8; APP_DATA_RESET_FRESH_ORIGIN_LENGTH], DatabaseOpenError> {
    if transaction_id.len() != 32
        || !transaction_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || old_identity.0 == 0
        || old_identity.1 == 0
        || fresh_identity.0 == 0
        || fresh_identity.1 == 0
        || fresh_identity == old_identity
    {
        return Err(storage_root_error(DatabaseOpenErrorKind::InternalState));
    }
    let mut bytes = [0_u8; APP_DATA_RESET_FRESH_ORIGIN_LENGTH];
    bytes[..16].copy_from_slice(APP_DATA_RESET_FRESH_ORIGIN_MAGIC);
    bytes[16..48].copy_from_slice(transaction_id.as_bytes());
    bytes[48..56].copy_from_slice(&old_identity.0.to_le_bytes());
    bytes[56..64].copy_from_slice(&old_identity.1.to_le_bytes());
    bytes[64..72].copy_from_slice(&fresh_identity.0.to_le_bytes());
    bytes[72..80].copy_from_slice(&fresh_identity.1.to_le_bytes());
    Ok(bytes)
}

fn ensure_app_data_reset_fresh_origin(
    file: &File,
    transaction_id: &str,
    old_identity: (u64, u64),
    fresh_identity: (u64, u64),
) -> Result<(), DatabaseOpenError> {
    let bytes = app_data_reset_fresh_origin_bytes(transaction_id, old_identity, fresh_identity)?;
    platform::write_all_at(file, &bytes, 0)
        .and_then(|()| file.sync_all())
        .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?;
    prove_app_data_reset_fresh_origin(file, transaction_id, old_identity, fresh_identity)
}

fn prove_app_data_reset_fresh_origin(
    file: &File,
    transaction_id: &str,
    old_identity: (u64, u64),
    fresh_identity: (u64, u64),
) -> Result<(), DatabaseOpenError> {
    let expected = app_data_reset_fresh_origin_bytes(transaction_id, old_identity, fresh_identity)?;
    if file
        .metadata()
        .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?
        .len()
        != expected.len() as u64
    {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    let mut observed = [0_u8; APP_DATA_RESET_FRESH_ORIGIN_LENGTH];
    platform::read_exact_at(file, &mut observed, 0)
        .map_err(|_| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
    if observed == expected {
        Ok(())
    } else {
        Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
    }
}

fn ensure_exact_marker(file: &File, marker: &[u8; 16]) -> Result<(), DatabaseOpenError> {
    platform::write_all_at(file, marker, 0)
        .and_then(|()| file.sync_all())
        .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))
}

fn inspect_root_marker(file: &File) -> Result<RootMarkerState, DatabaseOpenError> {
    Ok(if inspect_root_layout(file)?.is_some() {
        RootMarkerState::Valid
    } else {
        RootMarkerState::Invalid
    })
}

fn inspect_root_layout(file: &File) -> Result<Option<RootLayoutVersion>, DatabaseOpenError> {
    let length = file
        .metadata()
        .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?
        .len();
    if length != DUX_ROOT_MARKER.len() as u64 {
        return Ok(None);
    }
    let mut marker = [0_u8; 16];
    if platform::read_exact_at(file, &mut marker, 0).is_err() {
        return Ok(None);
    }
    match &marker {
        value if value == DUX_ROOT_MARKER => Ok(Some(RootLayoutVersion::LegacyV1)),
        value if value == DUX_ROOT_MARKER_LAYOUT_V2 => Ok(Some(RootLayoutVersion::CleanupLockV2)),
        _ => Ok(None),
    }
}

fn inspect_exact_marker(
    file: &File,
    expected: &[u8; 16],
) -> Result<RootMarkerState, DatabaseOpenError> {
    let length = file
        .metadata()
        .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?
        .len();
    if length != expected.len() as u64 {
        return Ok(RootMarkerState::Invalid);
    }
    let mut marker = [0_u8; 16];
    if platform::read_exact_at(file, &mut marker, 0).is_err() {
        return Ok(RootMarkerState::Invalid);
    }
    if &marker == expected {
        Ok(RootMarkerState::Valid)
    } else {
        Ok(RootMarkerState::Invalid)
    }
}

fn prove_root_marker(file: &File) -> Result<(), DatabaseOpenError> {
    if inspect_root_layout(file)?.is_some() {
        Ok(())
    } else {
        Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
    }
}

fn prove_current_root_marker(file: &File) -> Result<(), DatabaseOpenError> {
    prove_exact_marker(file, DUX_ROOT_MARKER_LAYOUT_V2)
}

fn upgrade_root_layout_marker(
    file: &File,
    observed: RootLayoutVersion,
) -> Result<(), DatabaseOpenError> {
    match observed {
        RootLayoutVersion::LegacyV1 => {
            if inspect_root_layout(file)? != Some(RootLayoutVersion::LegacyV1) {
                return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
            }
            ensure_exact_marker(file, DUX_ROOT_MARKER_LAYOUT_V2)?;
            prove_current_root_marker(file)
        }
        RootLayoutVersion::CleanupLockV2 => prove_current_root_marker(file),
    }
}

fn prove_initialization_sentinel(file: &File) -> Result<(), DatabaseOpenError> {
    prove_exact_marker(file, DUX_INITIALIZATION_SENTINEL)
}

fn prove_cleanup_lock_marker(file: &File) -> Result<(), DatabaseOpenError> {
    prove_exact_marker(file, DUX_CLEANUP_LOCK_MARKER)
}

fn prove_cleanup_lock_ready_marker(file: &File) -> Result<(), DatabaseOpenError> {
    prove_exact_marker(file, DUX_CLEANUP_LOCK_READY_MARKER)
}

fn prove_exact_marker(file: &File, expected: &[u8; 16]) -> Result<(), DatabaseOpenError> {
    if file
        .metadata()
        .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?
        .len()
        != expected.len() as u64
    {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    let mut marker = [0_u8; 16];
    platform::read_exact_at(file, &mut marker, 0)
        .map_err(|_| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
    if &marker != expected {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    Ok(())
}

fn storage_root_error(kind: DatabaseOpenErrorKind) -> DatabaseOpenError {
    DatabaseOpenError::new(kind)
}

fn object_error(kind: DatabaseOpenErrorKind) -> DatabaseOpenError {
    DatabaseOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PlatformIdentity {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PlatformIdentity {
    volume_serial: u64,
    file_id: [u8; 16],
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PlatformIdentity;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct StoreIdentity(PlatformIdentity);

#[cfg(any(target_os = "linux", target_os = "macos"))]
static ROOT_PUBLICATION_FENCES: OnceLock<Mutex<HashMap<PlatformIdentity, Weak<AtomicBool>>>> =
    OnceLock::new();

/// Exclusive publication ownership for one retained data-root parent.
///
/// The process-local bit closes advisory-lock self-conflict gaps while the
/// descriptor lock excludes other processes. Unlock happens before the local
/// bit is advertised as available.
#[cfg(any(target_os = "linux", target_os = "macos"))]
struct RootPublicationFence {
    file: File,
    in_use: Arc<AtomicBool>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for RootPublicationFence {
    fn drop(&mut self) {
        if FileExt::unlock(&self.file).is_ok() {
            self.in_use.store(false, Ordering::Release);
        } else {
            // Keep the registry state live and claimed after an uncertain
            // unlock. Closing the descriptor still releases the OS lock, but
            // no same-process caller may infer that publication is safe.
            std::mem::forget(Arc::clone(&self.in_use));
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn acquire_root_publication_fence_until(
    parent: &File,
    identity: PlatformIdentity,
    deadline: Instant,
) -> Result<RootPublicationFence, DatabaseOpenError> {
    let registry = ROOT_PUBLICATION_FENCES.get_or_init(|| Mutex::new(HashMap::new()));
    let in_use = {
        let mut registry = registry
            .lock()
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
        let state = registry.get(&identity).and_then(Weak::upgrade);
        if let Some(state) = state {
            state
        } else {
            let state = Arc::new(AtomicBool::new(false));
            registry.insert(identity, Arc::downgrade(&state));
            registry.retain(|_, state| state.strong_count() > 0);
            state
        }
    };

    loop {
        if Instant::now() >= deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        if in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            let now = Instant::now();
            if now >= deadline {
                return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
            }
            std::thread::sleep(LOCK_RETRY_INTERVAL.min(deadline.duration_since(now)));
            continue;
        }

        let lock_file = match parent.try_clone() {
            Ok(file) => file,
            Err(_) => {
                in_use.store(false, Ordering::Release);
                return Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                ));
            }
        };
        loop {
            if Instant::now() >= deadline {
                in_use.store(false, Ordering::Release);
                return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
            }
            match FileExt::try_lock(&lock_file) {
                Ok(()) if Instant::now() >= deadline => {
                    if FileExt::unlock(&lock_file).is_ok() {
                        in_use.store(false, Ordering::Release);
                    } else {
                        std::mem::forget(Arc::clone(&in_use));
                    }
                    return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
                }
                Ok(()) => {
                    return Ok(RootPublicationFence {
                        file: lock_file,
                        in_use,
                    });
                }
                Err(TryLockError::WouldBlock) => {
                    let now = Instant::now();
                    if now >= deadline {
                        in_use.store(false, Ordering::Release);
                        return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
                    }
                    std::thread::sleep(LOCK_RETRY_INTERVAL.min(deadline.duration_since(now)));
                }
                Err(TryLockError::Error(_)) => {
                    in_use.store(false, Ordering::Release);
                    return Err(storage_root_error(
                        DatabaseOpenErrorKind::StorageRootUnavailable,
                    ));
                }
            }
        }
    }
}

/// Callback-scoped proof that the exact data root is still published and its
/// generated detached destination is still absent.
///
/// This type grants validation only. It intentionally exposes no descriptor,
/// path, name, identity, rename, journal, or effect operation.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) struct AppDataResetDataNamespaceAdmission<'scope> {
    paths: &'scope SecureStorePaths,
    root_name: &'scope OsStr,
    detached_name: &'scope OsStr,
    _fence: &'scope RootPublicationFence,
    deadline: Instant,
    detached: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetDataNamespaceAdmission<'_> {
    pub(super) const fn journal_identity_parts(&self) -> (u64, u64) {
        (
            self.paths.root_identity.device,
            self.paths.root_identity.inode,
        )
    }

    pub(super) const fn canonical_root_name(&self) -> &OsStr {
        self.root_name
    }

    pub(super) const fn is_detached(&self) -> bool {
        self.detached
    }

    pub(super) fn revalidate(&self) -> Result<(), DatabaseOpenError> {
        self.revalidate_until(self.deadline)
    }

    fn revalidate_until(&self, deadline: Instant) -> Result<(), DatabaseOpenError> {
        if Instant::now() >= deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        if self.detached {
            platform::validate_detached_data_reset_namespace(
                &self.paths.publication_parent,
                self.paths.publication_parent_identity,
                &self.paths.root_directory,
                self.paths.root_identity,
                &self.paths.root_path,
                self.root_name,
                self.detached_name,
                deadline,
            )?;
        } else {
            platform::validate_data_reset_namespace(
                &self.paths.publication_parent,
                self.paths.publication_parent_identity,
                &self.paths.root_directory,
                self.paths.root_identity,
                &self.paths.root_path,
                self.root_name,
                self.detached_name,
                deadline,
            )?;
        }
        self.paths.validate_app_data_reset_retained_store()?;
        if Instant::now() >= deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        Ok(())
    }

    /// Detach the exact retained canonical root to the sealed transaction
    /// stage. This consumes the witness so no rename-attempt uncertainty can
    /// be retried through the same admission.
    pub(super) fn detach(
        mut self,
        expected_identity: (u64, u64),
        expected_detached_name: &OsStr,
    ) -> Result<Self, DatabaseOpenError> {
        if self.detached
            || expected_identity
                != (
                    self.paths.root_identity.device,
                    self.paths.root_identity.inode,
                )
            || expected_detached_name != self.detached_name
        {
            return Err(storage_root_error(DatabaseOpenErrorKind::InternalState));
        }
        self.revalidate()?;
        if take_test_app_data_reset_data_detach_fault(AppDataResetDataDetachFault::BeforeRename) {
            return Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ));
        }
        if take_test_app_data_reset_data_detach_fault(
            AppDataResetDataDetachFault::ExpireBeforeRename,
        ) {
            std::thread::sleep(
                self.deadline
                    .saturating_duration_since(Instant::now())
                    .saturating_add(Duration::from_millis(1)),
            );
        }
        if Instant::now() >= self.deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        #[cfg(test)]
        if take_test_app_data_reset_data_detach_fault(
            AppDataResetDataDetachFault::RaceDestinationCollision,
        ) {
            platform::create_test_data_reset_collision(
                &self.paths.publication_parent,
                self.detached_name,
            )?;
        }
        platform::detach_data_root_no_replace(
            &self.paths.publication_parent,
            self.root_name,
            &self.paths.root_directory,
            self.paths.root_identity,
            self.detached_name,
        )?;
        if take_test_app_data_reset_data_detach_fault(AppDataResetDataDetachFault::AfterRename) {
            return Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ));
        }
        platform::sync_directory(&self.paths.publication_parent)?;
        if take_test_app_data_reset_data_detach_fault(
            AppDataResetDataDetachFault::AfterDirectorySync,
        ) {
            return Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ));
        }
        self.detached = true;
        let post_effect_deadline = Instant::now()
            .checked_add(APP_DATA_RESET_POST_EFFECT_TIMEOUT)
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
        self.revalidate_until(post_effect_deadline)?;
        if take_test_app_data_reset_data_detach_fault(AppDataResetDataDetachFault::DuringReadback) {
            return Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ));
        }
        Ok(self)
    }
}

/// Callback-scoped recovery authority for an already-journaled data reset.
///
/// Construction opens only existing descriptor-relative objects. The
/// publication, cleanup, and writer guards are retained by the surrounding
/// callback and are revalidated with every observation or namespace effect.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) struct AppDataResetRecoveryDataNamespace<'scope> {
    paths: &'scope SecureStorePaths,
    root_name: &'scope OsStr,
    detached_name: &'scope OsStr,
    _fence: &'scope RootPublicationFence,
    writer: &'scope WriterLockGuard,
    cleanup: &'scope CleanupLockGuard,
    snapshot: &'scope SnapshotStoreInventoryLease,
    deadline: Instant,
    location: AppDataResetRecoveryDataLocation,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetRecoveryDataNamespace<'_> {
    pub(super) const fn location(&self) -> AppDataResetRecoveryDataLocation {
        self.location
    }

    pub(super) const fn journal_identity_parts(&self) -> (u64, u64) {
        (
            self.paths.root_identity.device,
            self.paths.root_identity.inode,
        )
    }

    pub(super) const fn canonical_root_name(&self) -> &OsStr {
        self.root_name
    }

    pub(super) fn revalidate(&self) -> Result<(), DatabaseOpenError> {
        self.revalidate_until(self.deadline)
    }

    fn revalidate_until(&self, deadline: Instant) -> Result<(), DatabaseOpenError> {
        if Instant::now() >= deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        match self.location {
            AppDataResetRecoveryDataLocation::Canonical => {
                platform::validate_data_reset_namespace(
                    &self.paths.publication_parent,
                    self.paths.publication_parent_identity,
                    &self.paths.root_directory,
                    self.paths.root_identity,
                    &self.paths.root_path,
                    self.root_name,
                    self.detached_name,
                    deadline,
                )?;
            }
            AppDataResetRecoveryDataLocation::Detached => {
                platform::validate_detached_data_reset_namespace(
                    &self.paths.publication_parent,
                    self.paths.publication_parent_identity,
                    &self.paths.root_directory,
                    self.paths.root_identity,
                    &self.paths.root_path,
                    self.root_name,
                    self.detached_name,
                    deadline,
                )?;
            }
        }
        self.paths
            .validate_app_data_reset_detached_guards(self.writer, self.cleanup)?;
        self.snapshot
            .revalidate_complete_for_app_data_reset_until(deadline)
            .map_err(map_snapshot_recovery_error)?;
        if Instant::now() >= deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        Ok(())
    }

    /// Reconcile the journaled detach exactly once through this witness.
    /// Already-detached recovery is a validated no-op; canonical recovery uses
    /// the same no-replace rename, parent sync, and detached readback as the
    /// live reset path.
    pub(super) fn detach_if_canonical(
        mut self,
        expected_identity: (u64, u64),
        expected_detached_name: &OsStr,
    ) -> Result<Self, DatabaseOpenError> {
        if expected_identity
            != (
                self.paths.root_identity.device,
                self.paths.root_identity.inode,
            )
            || expected_detached_name != self.detached_name
        {
            return Err(storage_root_error(DatabaseOpenErrorKind::InternalState));
        }
        self.revalidate()?;
        if self.location == AppDataResetRecoveryDataLocation::Detached {
            return Ok(self);
        }
        if take_test_app_data_reset_data_detach_fault(AppDataResetDataDetachFault::BeforeRename) {
            return Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ));
        }
        if Instant::now() >= self.deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        platform::detach_data_root_no_replace(
            &self.paths.publication_parent,
            self.root_name,
            &self.paths.root_directory,
            self.paths.root_identity,
            self.detached_name,
        )?;
        if take_test_app_data_reset_data_detach_fault(AppDataResetDataDetachFault::AfterRename) {
            return Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ));
        }
        platform::sync_directory(&self.paths.publication_parent)?;
        if take_test_app_data_reset_data_detach_fault(
            AppDataResetDataDetachFault::AfterDirectorySync,
        ) {
            return Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ));
        }
        self.location = AppDataResetRecoveryDataLocation::Detached;
        let post_effect_deadline = Instant::now()
            .checked_add(APP_DATA_RESET_POST_EFFECT_TIMEOUT)
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
        self.revalidate_until(post_effect_deadline)?;
        if take_test_app_data_reset_data_detach_fault(AppDataResetDataDetachFault::DuringReadback) {
            return Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ));
        }
        Ok(self)
    }
}

/// Callback-scoped authority for exactly one transaction-bound fresh root.
///
/// The old detached store and all of its exclusion guards remain retained
/// while a complete private bootstrap is staged or atomically published. The
/// bootstrap contains no initialized SQLite database, snapshots, cache, AI,
/// logs, or sidecars.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) struct AppDataResetFreshNamespace<'scope> {
    old_paths: &'scope SecureStorePaths,
    root_name: &'scope OsStr,
    detached_name: &'scope OsStr,
    fresh_stage_name: &'scope OsStr,
    database_name: &'scope OsStr,
    transaction_id: &'scope str,
    expected_old_identity: (u64, u64),
    publication_parent: &'scope File,
    publication_parent_identity: PlatformIdentity,
    _fence: &'scope RootPublicationFence,
    writer: &'scope WriterLockGuard,
    cleanup: &'scope CleanupLockGuard,
    snapshot: AppDataResetSnapshotRecovery,
    deadline: Instant,
    fresh: Option<AppDataResetFreshRoot>,
    location: AppDataResetFreshNamespaceLocation,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl<'scope> AppDataResetFreshNamespace<'scope> {
    pub(super) const fn deadline(&self) -> Instant {
        self.deadline
    }

    pub(super) const fn location(&self) -> AppDataResetFreshNamespaceLocation {
        self.location
    }

    pub(super) fn fresh_identity_parts(&self) -> Option<(u64, u64)> {
        self.fresh
            .as_ref()
            .map(|fresh| (fresh.identity.device, fresh.identity.inode))
    }

    pub(super) fn is_ready_to_drain_bound_to(
        &self,
        transaction_id: &str,
        old_identity: (u64, u64),
        fresh_identity: (u64, u64),
        fresh_stage_name: &OsStr,
        publication_parent_identity: (u64, u64),
        canonical_root_name: &OsStr,
    ) -> bool {
        self.location == AppDataResetFreshNamespaceLocation::Canonical
            && self.fresh_identity_parts() == Some(fresh_identity)
            && self.transaction_id == transaction_id
            && self.expected_old_identity == old_identity
            && self.fresh_stage_name == fresh_stage_name
            && (
                self.publication_parent_identity.device,
                self.publication_parent_identity.inode,
            ) == publication_parent_identity
            && self.root_name == canonical_root_name
    }

    pub(super) fn snapshot_payload_drain_candidate(
        &self,
    ) -> Result<Option<AppDataResetSnapshotPayloadDrainCandidate>, DatabaseOpenError> {
        self.revalidate()?;
        self.snapshot
            .payload_drain_candidate_until(self.deadline)
            .map_err(map_snapshot_recovery_error)
    }

    pub(super) fn snapshot_store_retirement_state(
        &self,
    ) -> Result<Option<AppDataResetSnapshotStoreRetirementState>, DatabaseOpenError> {
        self.revalidate()?;
        Ok(self.snapshot.retirement_state())
    }

    pub(super) fn revalidate_snapshot_payload_drain_candidate_until(
        &self,
        candidate: &AppDataResetSnapshotPayloadDrainCandidate,
        deadline: Instant,
    ) -> Result<(), DatabaseOpenError> {
        let deadline = deadline.min(self.deadline);
        self.revalidate_until(deadline)?;
        self.snapshot
            .revalidate_payload_candidate_until(candidate, deadline)
            .map_err(map_snapshot_recovery_error)
    }

    pub(super) fn drain_one_snapshot_payload(
        mut self,
        candidate: AppDataResetSnapshotPayloadDrainCandidate,
        authority: AppDataResetSnapshotPayloadDrainAuthority,
    ) -> std::result::Result<
        AppDataResetSnapshotPayloadDrainCompletion,
        AppDataResetSnapshotPayloadDrainError,
    > {
        let deadline = authority.pre_effect_deadline();
        if deadline != self.deadline {
            return Err(AppDataResetSnapshotPayloadDrainError::BeforeEffect(
                SnapshotStorageErrorKind::InternalState,
            ));
        }
        self.revalidate_until(deadline).map_err(|error| {
            AppDataResetSnapshotPayloadDrainError::BeforeEffect(map_database_open_to_snapshot_kind(
                error.kind,
            ))
        })?;
        self.snapshot
            .revalidate_payload_candidate_until(&candidate, deadline)
            .map_err(|error| AppDataResetSnapshotPayloadDrainError::BeforeEffect(error.kind()))?;
        let progress = self.snapshot.drain_one_payload(candidate, authority)?;
        let post_effect_deadline = progress.post_effect_deadline();
        #[cfg(test)]
        if take_test_app_data_reset_snapshot_postcheck_fault(
            TestAppDataResetSnapshotPostcheckFault::ExhaustBeforeOldFreshReadback,
        ) {
            exhaust_app_data_reset_deadline(post_effect_deadline);
        }
        self.snapshot
            .revalidate_after_payload_effect_until(post_effect_deadline)
            .map_err(|_| AppDataResetSnapshotPayloadDrainError::OutcomeUnknown)?;
        self.revalidate_data_namespaces_until(post_effect_deadline)
            .map_err(|_| AppDataResetSnapshotPayloadDrainError::OutcomeUnknown)?;
        Ok(progress)
    }

    pub(super) fn retire_one_snapshot_store_structure(
        mut self,
        authority: AppDataResetSnapshotStoreRetireAuthority,
    ) -> std::result::Result<
        AppDataResetSnapshotStoreRetirementCompletion,
        AppDataResetSnapshotStoreRetirementError,
    > {
        let deadline = authority.pre_effect_deadline();
        if deadline != self.deadline {
            return Err(AppDataResetSnapshotStoreRetirementError::BeforeEffect(
                SnapshotStorageErrorKind::InternalState,
            ));
        }
        self.revalidate_until(deadline).map_err(|error| {
            AppDataResetSnapshotStoreRetirementError::BeforeEffect(
                map_database_open_to_snapshot_kind(error.kind),
            )
        })?;
        let progress = self.snapshot.retire_one_structure(authority)?;
        let post_effect_deadline = progress.post_effect_deadline();
        #[cfg(test)]
        if take_test_app_data_reset_snapshot_postcheck_fault(
            TestAppDataResetSnapshotPostcheckFault::ExhaustBeforeOldFreshReadback,
        ) {
            exhaust_app_data_reset_deadline(post_effect_deadline);
        }
        self.revalidate_data_namespaces_until(post_effect_deadline)
            .map_err(|_| AppDataResetSnapshotStoreRetirementError::OutcomeUnknown)?;
        Ok(progress)
    }

    pub(super) fn revalidate(&self) -> Result<(), DatabaseOpenError> {
        self.revalidate_until(self.deadline)
    }

    pub(super) fn revalidate_until(&self, deadline: Instant) -> Result<(), DatabaseOpenError> {
        if Instant::now() >= deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        self.old_paths
            .validate_app_data_reset_detached_guards(self.writer, self.cleanup)?;
        self.snapshot
            .revalidate_until(deadline)
            .map_err(map_snapshot_recovery_error)?;
        self.revalidate_data_namespaces_until(deadline)
    }

    fn revalidate_data_namespaces_until(&self, deadline: Instant) -> Result<(), DatabaseOpenError> {
        if Instant::now() >= deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        self.old_paths
            .validate_app_data_reset_detached_guards(self.writer, self.cleanup)?;
        match (self.location, self.fresh.as_ref()) {
            (AppDataResetFreshNamespaceLocation::Absent, None) => {
                platform::validate_detached_data_reset_namespace(
                    self.publication_parent,
                    self.publication_parent_identity,
                    &self.old_paths.root_directory,
                    self.old_paths.root_identity,
                    &self.old_paths.root_path,
                    self.root_name,
                    self.detached_name,
                    deadline,
                )?;
                platform::validate_named_absence(self.publication_parent, self.root_name)?;
                platform::validate_named_absence(self.publication_parent, self.fresh_stage_name)?;
            }
            (AppDataResetFreshNamespaceLocation::Staged, Some(fresh)) => {
                platform::validate_detached_data_reset_namespace(
                    self.publication_parent,
                    self.publication_parent_identity,
                    &self.old_paths.root_directory,
                    self.old_paths.root_identity,
                    &self.old_paths.root_path,
                    self.root_name,
                    self.detached_name,
                    deadline,
                )?;
                platform::validate_named_absence(self.publication_parent, self.root_name)?;
                revalidate_app_data_reset_fresh_root(
                    fresh,
                    self.publication_parent,
                    self.publication_parent_identity,
                    self.fresh_stage_name,
                    self.database_name,
                    self.transaction_id,
                    self.expected_old_identity,
                    deadline,
                )?;
            }
            (AppDataResetFreshNamespaceLocation::Canonical, Some(fresh)) => {
                platform::validate_named_absence(self.publication_parent, self.fresh_stage_name)?;
                platform::validate_detached_data_with_fresh_namespace(
                    self.publication_parent,
                    self.publication_parent_identity,
                    &self.old_paths.root_directory,
                    self.old_paths.root_identity,
                    &fresh.directory,
                    fresh.identity,
                    &self.old_paths.root_path,
                    self.root_name,
                    self.detached_name,
                    deadline,
                )?;
                revalidate_app_data_reset_fresh_root(
                    fresh,
                    self.publication_parent,
                    self.publication_parent_identity,
                    self.root_name,
                    self.database_name,
                    self.transaction_id,
                    self.expected_old_identity,
                    deadline,
                )?;
            }
            _ => return Err(storage_root_error(DatabaseOpenErrorKind::InternalState)),
        }
        Ok(())
    }

    /// Consume this witness and converge an absent or staged bootstrap to the
    /// canonical name. Every rename is no-replace and every post-effect error
    /// consumes the witness so the caller can only enter recovery again.
    pub(super) fn publish_if_needed(
        mut self,
    ) -> Result<AppDataResetPublishedFreshNamespace<'scope>, DatabaseOpenError> {
        self.revalidate()?;
        if self.location == AppDataResetFreshNamespaceLocation::Absent {
            if take_test_app_data_reset_fresh_namespace_fault(
                AppDataResetFreshNamespaceFault::BeforeStageCreate,
            ) {
                return Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                ));
            }
            let opened = platform::prepare_app_data_reset_fresh_stage(
                self.publication_parent,
                self.publication_parent_identity,
                self.fresh_stage_name,
                self.database_name,
                self.transaction_id,
                self.expected_old_identity,
                self.deadline,
            )?;
            let post_effect_deadline = Instant::now()
                .checked_add(APP_DATA_RESET_POST_EFFECT_TIMEOUT)
                .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
            self.fresh = Some(open_app_data_reset_fresh_root(
                self.publication_parent,
                self.publication_parent_identity,
                opened,
                &self
                    .old_paths
                    .root_path
                    .with_file_name(self.fresh_stage_name),
                self.fresh_stage_name,
                self.database_name,
                self.transaction_id,
                self.expected_old_identity,
                post_effect_deadline,
            )?);
            self.location = AppDataResetFreshNamespaceLocation::Staged;
            if take_test_app_data_reset_fresh_namespace_fault(
                AppDataResetFreshNamespaceFault::AfterStagePrepared,
            ) {
                return Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                ));
            }
            self.revalidate_until(post_effect_deadline)?;
        }

        if self.location == AppDataResetFreshNamespaceLocation::Staged {
            let fresh = self
                .fresh
                .as_ref()
                .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
            #[cfg(test)]
            if take_test_app_data_reset_fresh_namespace_fault(
                AppDataResetFreshNamespaceFault::RaceCanonicalCollision,
            ) {
                platform::create_test_data_reset_collision(
                    self.publication_parent,
                    self.root_name,
                )?;
            }
            platform::publish_app_data_reset_fresh_stage(
                self.publication_parent,
                self.publication_parent_identity,
                self.fresh_stage_name,
                &fresh.directory,
                fresh.identity,
                self.root_name,
                self.deadline,
            )?;
            self.location = AppDataResetFreshNamespaceLocation::Canonical;
            let post_effect_deadline = Instant::now()
                .checked_add(APP_DATA_RESET_POST_EFFECT_TIMEOUT)
                .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
            if take_test_app_data_reset_fresh_namespace_fault(
                AppDataResetFreshNamespaceFault::AfterCanonicalRename,
            ) {
                return Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                ));
            }
            platform::sync_directory(self.publication_parent)?;
            if take_test_app_data_reset_fresh_namespace_fault(
                AppDataResetFreshNamespaceFault::AfterParentSync,
            ) {
                return Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                ));
            }
            self.revalidate_until(post_effect_deadline)?;
            if take_test_app_data_reset_fresh_namespace_fault(
                AppDataResetFreshNamespaceFault::DuringReadback,
            ) {
                return Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                ));
            }
        }
        if self.location != AppDataResetFreshNamespaceLocation::Canonical {
            return Err(storage_root_error(DatabaseOpenErrorKind::InternalState));
        }
        Ok(AppDataResetPublishedFreshNamespace { inner: self })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) struct AppDataResetPublishedFreshNamespace<'scope> {
    inner: AppDataResetFreshNamespace<'scope>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetPublishedFreshNamespace<'_> {
    pub(super) fn revalidate(&self) -> Result<(), DatabaseOpenError> {
        if self.inner.location != AppDataResetFreshNamespaceLocation::Canonical {
            return Err(storage_root_error(DatabaseOpenErrorKind::InternalState));
        }
        self.inner.revalidate()
    }

    pub(super) fn fresh_identity_parts(&self) -> Result<(u64, u64), DatabaseOpenError> {
        self.inner
            .fresh_identity_parts()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn is_bound_to(
        &self,
        transaction_id: &str,
        old_identity: (u64, u64),
        fresh_stage_name: &OsStr,
        publication_parent_identity: (u64, u64),
        canonical_root_name: &OsStr,
    ) -> bool {
        self.inner.transaction_id == transaction_id
            && self.inner.expected_old_identity == old_identity
            && self.inner.fresh_stage_name == fresh_stage_name
            && (
                self.inner.publication_parent_identity.device,
                self.inner.publication_parent_identity.inode,
            ) == publication_parent_identity
            && self.inner.root_name == canonical_root_name
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_snapshot_recovery_error(error: SnapshotStorageError) -> DatabaseOpenError {
    let kind = match error.kind() {
        SnapshotStorageErrorKind::Busy => DatabaseOpenErrorKind::Busy,
        SnapshotStorageErrorKind::Unavailable => DatabaseOpenErrorKind::DatabaseUnavailable,
        SnapshotStorageErrorKind::UnsafeRoot => DatabaseOpenErrorKind::UnsafeStorageRoot,
        SnapshotStorageErrorKind::UnsafeObject | SnapshotStorageErrorKind::UnrecognizedStore => {
            DatabaseOpenErrorKind::UnsafeStorageObject
        }
        SnapshotStorageErrorKind::InvalidConfiguration
        | SnapshotStorageErrorKind::InternalState => DatabaseOpenErrorKind::InternalState,
    };
    DatabaseOpenError::new(kind)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
const fn map_database_open_to_snapshot_kind(
    kind: DatabaseOpenErrorKind,
) -> SnapshotStorageErrorKind {
    match kind {
        DatabaseOpenErrorKind::Busy => SnapshotStorageErrorKind::Busy,
        DatabaseOpenErrorKind::UnsafeStorageRoot | DatabaseOpenErrorKind::UnsafePermissions => {
            SnapshotStorageErrorKind::UnsafeRoot
        }
        DatabaseOpenErrorKind::UnsafeStorageObject
        | DatabaseOpenErrorKind::OwnershipMismatch
        | DatabaseOpenErrorKind::UnrecognizedDatabase
        | DatabaseOpenErrorKind::CorruptDatabase => SnapshotStorageErrorKind::UnsafeObject,
        DatabaseOpenErrorKind::StorageRootUnavailable
        | DatabaseOpenErrorKind::InspectionLimitExceeded
        | DatabaseOpenErrorKind::DatabaseUnavailable
        | DatabaseOpenErrorKind::MigrationFailed => SnapshotStorageErrorKind::Unavailable,
        DatabaseOpenErrorKind::InternalState => SnapshotStorageErrorKind::InternalState,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod platform {
    use std::ffi::{CString, OsStr, OsString};
    use std::fs::File;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::fs::FileExt;
    use std::path::Path;
    use std::time::Instant;

    use nix::dir::Dir;
    use nix::errno::Errno;
    use nix::fcntl::{AtFlags, OFlag, open, openat};
    use nix::sys::stat::{Mode, SFlag, fchmod, fstat, fstatat, mkdirat};
    use nix::unistd::{UnlinkatFlags, geteuid, unlinkat};

    use super::{
        APP_DATA_RESET_FRESH_ORIGIN_NAME, APP_DATA_RESET_POST_EFFECT_TIMEOUT,
        AppDataResetFreshNamespaceFault, CONTROL_OBJECT_LOCK_TIMEOUT, DUX_CLEANUP_LOCK_MARKER,
        DUX_CLEANUP_LOCK_READY_MARKER, DUX_ROOT_MARKER_LAYOUT_V2, DatabaseOpenError,
        DatabaseOpenErrorKind, MAX_SNAPSHOT_STAGES, ObjectKind, PermissionPolicy, PlatformIdentity,
        PreparedRoot, PreparedRootState, ROOT_INVENTORY_MAX_NAME_BYTES, ROOT_INVENTORY_TIMEOUT,
        RootPublicationResult, acquire_root_publication_fence_until, cleanup_lock_name,
        cleanup_lock_ready_name, ensure_app_data_reset_fresh_origin, ensure_exact_marker,
        is_canonical_snapshot_stage_name, lock_name, object_error,
        prove_app_data_reset_fresh_origin, prove_cleanup_lock_marker,
        prove_cleanup_lock_ready_marker, prove_current_root_marker, storage_root_error,
        take_test_app_data_reset_fresh_namespace_fault,
    };

    const DIRECTORY_MODE: Mode = Mode::S_IRWXU;
    const FILE_MODE: Mode = Mode::S_IRUSR.union(Mode::S_IWUSR);
    const STAGING_ATTEMPTS: usize = 8;

    pub(super) fn sync_directory(directory: &File) -> Result<(), DatabaseOpenError> {
        directory
            .sync_all()
            .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))
    }

    pub(super) fn file_usage(file: &File) -> Result<(u64, u64), DatabaseOpenError> {
        let status =
            fstat(file).map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?;
        let logical_bytes = u64::try_from(status.st_size)
            .map_err(|_| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        let blocks = u64::try_from(status.st_blocks)
            .map_err(|_| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        let allocated_bytes = blocks
            .checked_mul(512)
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        Ok((logical_bytes, allocated_bytes))
    }

    /// Open only an already-existing publication parent. Recovery must never
    /// invoke the normal root preparation path because it may provision a
    /// missing canonical root.
    pub(super) fn open_existing_publication_parent(
        parent_path: &Path,
    ) -> Result<(File, PlatformIdentity), DatabaseOpenError> {
        let parent = open(
            parent_path,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(map_root_open_error)?;
        let identity = validate_publication_parent(&parent)?;
        Ok((parent, identity))
    }

    pub(super) fn validate_publication_parent_identity(
        parent: &File,
    ) -> Result<PlatformIdentity, DatabaseOpenError> {
        validate_publication_parent(parent)
    }

    pub(super) fn validate_named_absence(
        parent: &File,
        name: &OsStr,
    ) -> Result<(), DatabaseOpenError> {
        match fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
            Err(Errno::ENOENT) => Ok(()),
            Ok(_) | Err(Errno::ELOOP | Errno::ENOTDIR) => {
                Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))
            }
            Err(_) => Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            )),
        }
    }

    #[allow(
        dead_code,
        reason = "the persistence composition slice consumes old database draining"
    )]
    pub(super) fn directory_entry_names_until(
        directory: &File,
        maximum_entries: usize,
        maximum_name_bytes: usize,
        deadline: Instant,
    ) -> Result<Vec<OsString>, DatabaseOpenError> {
        let clone = directory
            .try_clone()
            .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?;
        let owned: OwnedFd = clone.into();
        let mut entries = Dir::from_fd(owned)
            .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?;
        let mut names = Vec::new();
        let mut name_bytes = 0_usize;
        for entry in entries.iter() {
            if Instant::now() >= deadline {
                return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
            }
            let entry =
                entry.map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?;
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            name_bytes = name_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
            if names.len() >= maximum_entries || name_bytes > maximum_name_bytes {
                return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
            }
            names.push(OsString::from_vec(bytes.to_vec()));
        }
        names.sort_unstable_by(|left, right| {
            left.as_os_str()
                .as_bytes()
                .cmp(right.as_os_str().as_bytes())
        });
        if Instant::now() >= deadline {
            Err(storage_root_error(DatabaseOpenErrorKind::Busy))
        } else {
            Ok(names)
        }
    }

    #[allow(
        dead_code,
        reason = "the persistence composition slice consumes old database draining"
    )]
    pub(super) fn validate_detached_old_data_root_until(
        parent: &File,
        parent_identity: PlatformIdentity,
        root: &File,
        root_identity: PlatformIdentity,
        detached_name: &OsStr,
        deadline: Instant,
    ) -> Result<(), DatabaseOpenError> {
        let bytes = detached_name.as_bytes();
        if Instant::now() >= deadline
            || bytes.is_empty()
            || matches!(bytes, b"." | b"..")
            || bytes.contains(&b'/')
            || bytes.contains(&0)
            || root_identity.device != parent_identity.device
            || validate_publication_parent(parent)? != parent_identity
        {
            return Err(storage_root_error(if Instant::now() >= deadline {
                DatabaseOpenErrorKind::Busy
            } else {
                DatabaseOpenErrorKind::UnsafeStorageRoot
            }));
        }
        validate_retained_file(
            root,
            ObjectKind::Directory,
            root_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        validate_named_object(
            parent,
            detached_name,
            root,
            root_identity,
            ObjectKind::Directory,
        )?;

        let clone = parent
            .try_clone()
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let owned: OwnedFd = clone.into();
        let mut directory = Dir::from_fd(owned)
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let mut exact_name_seen = false;
        for entry in directory.iter() {
            if Instant::now() >= deadline {
                return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
            }
            let entry = entry
                .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
            if entry.file_name().to_bytes() == bytes {
                exact_name_seen = true;
                break;
            }
        }
        if !exact_name_seen
            || validate_publication_parent(parent)? != parent_identity
            || Instant::now() >= deadline
        {
            return Err(storage_root_error(if Instant::now() >= deadline {
                DatabaseOpenErrorKind::Busy
            } else {
                DatabaseOpenErrorKind::UnsafeStorageRoot
            }));
        }
        validate_named_object(
            parent,
            detached_name,
            root,
            root_identity,
            ObjectKind::Directory,
        )
    }

    #[allow(
        dead_code,
        reason = "the persistence composition slice consumes old database draining"
    )]
    pub(super) fn unlink_app_data_reset_old_database_payload_with_before_unlink(
        directory: &File,
        name: &OsStr,
        retained: &File,
        retained_identity: PlatformIdentity,
        before_unlink: impl FnOnce() -> Result<(), DatabaseOpenError>,
    ) -> Result<(), DatabaseOpenError> {
        validate_named_object(
            directory,
            name,
            retained,
            retained_identity,
            ObjectKind::RegularFile,
        )?;
        before_unlink()?;
        // DUX-DESTRUCTIVE: allow=app-data-reset-old-database-payload-unlink -- unlink only one retained private single-link SQLite payload after exact detached-root inventory and final identity revalidation
        unlinkat(directory, name, UnlinkatFlags::NoRemoveDir)
            .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_app_data_reset_fresh_stage(
        parent: &File,
        parent_identity: PlatformIdentity,
        fresh_stage_name: &OsStr,
        database_name: &OsStr,
        transaction_id: &str,
        old_identity: (u64, u64),
        deadline: Instant,
    ) -> Result<(File, PlatformIdentity), DatabaseOpenError> {
        fn valid_component(name: &OsStr) -> bool {
            let bytes = name.as_bytes();
            !bytes.is_empty()
                && bytes != b"."
                && bytes != b".."
                && !bytes.contains(&b'/')
                && !bytes.contains(&0)
        }

        if Instant::now() >= deadline
            || !valid_component(fresh_stage_name)
            || !valid_component(database_name)
            || validate_publication_parent(parent)? != parent_identity
        {
            return Err(storage_root_error(if Instant::now() >= deadline {
                DatabaseOpenErrorKind::Busy
            } else {
                DatabaseOpenErrorKind::UnsafeStorageRoot
            }));
        }
        validate_named_absence(parent, fresh_stage_name)?;

        let (work_name, work_root, work_identity) = loop {
            if Instant::now() >= deadline {
                return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
            }
            let work_name = random_stage_name()?;
            match mkdirat(parent, work_name.as_os_str(), DIRECTORY_MODE) {
                Ok(()) => {
                    let root = openat(
                        parent,
                        work_name.as_os_str(),
                        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
                        Mode::empty(),
                    )
                    .map(File::from)
                    .map_err(map_root_open_error)?;
                    let identity = validate_file(
                        &root,
                        ObjectKind::Directory,
                        None,
                        PermissionPolicy::RequirePrivate,
                    )?;
                    if identity.device != parent_identity.device
                        || (identity.device, identity.inode) == old_identity
                    {
                        return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
                    }
                    break (work_name, root, identity);
                }
                Err(Errno::EEXIST) => continue,
                Err(error) => return Err(map_root_create_error(error)),
            }
        };

        let work_path = Path::new(".");
        let (database, database_identity) =
            create_private_file_exclusive(&work_root, work_path, database_name)?;
        database
            .sync_all()
            .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?;
        let writer_name = lock_name(database_name);
        let (writer, writer_identity) =
            create_private_file_exclusive(&work_root, work_path, &writer_name)?;
        ensure_exact_marker(&writer, DUX_ROOT_MARKER_LAYOUT_V2)?;
        let cleanup_name = cleanup_lock_name(database_name);
        let (cleanup, cleanup_identity) =
            create_private_file_exclusive(&work_root, work_path, &cleanup_name)?;
        ensure_exact_marker(&cleanup, DUX_CLEANUP_LOCK_MARKER)?;
        let ready_name = cleanup_lock_ready_name(database_name);
        let (ready, ready_identity) =
            create_private_file_exclusive(&work_root, work_path, &ready_name)?;
        ensure_exact_marker(&ready, DUX_CLEANUP_LOCK_READY_MARKER)?;
        let origin_name = OsStr::new(APP_DATA_RESET_FRESH_ORIGIN_NAME);
        let (origin, origin_identity) =
            create_private_file_exclusive(&work_root, work_path, origin_name)?;
        ensure_app_data_reset_fresh_origin(
            &origin,
            transaction_id,
            old_identity,
            (work_identity.device, work_identity.inode),
        )?;
        let allowed = [
            database_name,
            writer_name.as_os_str(),
            cleanup_name.as_os_str(),
            ready_name.as_os_str(),
            origin_name,
        ];
        if !root_contains_only_exact(&work_root, &allowed)? {
            return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
        }
        if database
            .metadata()
            .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))?
            .len()
            != 0
        {
            return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
        }
        prove_current_root_marker(&writer)?;
        prove_cleanup_lock_marker(&cleanup)?;
        prove_cleanup_lock_ready_marker(&ready)?;
        prove_app_data_reset_fresh_origin(
            &origin,
            transaction_id,
            old_identity,
            (work_identity.device, work_identity.inode),
        )?;
        for (name, file, identity) in [
            (database_name, &database, database_identity),
            (writer_name.as_os_str(), &writer, writer_identity),
            (cleanup_name.as_os_str(), &cleanup, cleanup_identity),
            (ready_name.as_os_str(), &ready, ready_identity),
            (origin_name, &origin, origin_identity),
        ] {
            validate_retained_file(
                file,
                ObjectKind::RegularFile,
                identity,
                PermissionPolicy::RequirePrivate,
            )?;
            validate_named_object(&work_root, name, file, identity, ObjectKind::RegularFile)?;
        }
        validate_named_object(
            parent,
            &work_name,
            &work_root,
            work_identity,
            ObjectKind::Directory,
        )?;
        work_root
            .sync_all()
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        if Instant::now() >= deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        if validate_publication_parent(parent)? != parent_identity {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        rename_no_replace(parent, &work_name, fresh_stage_name)
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let post_effect_deadline = Instant::now()
            .checked_add(APP_DATA_RESET_POST_EFFECT_TIMEOUT)
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
        if take_test_app_data_reset_fresh_namespace_fault(
            AppDataResetFreshNamespaceFault::AfterFreshStageRename,
        ) {
            return Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ));
        }
        sync_directory(parent)?;
        if take_test_app_data_reset_fresh_namespace_fault(
            AppDataResetFreshNamespaceFault::AfterFreshStageSync,
        ) {
            return Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ));
        }
        validate_named_object(
            parent,
            fresh_stage_name,
            &work_root,
            work_identity,
            ObjectKind::Directory,
        )?;
        if Instant::now() >= post_effect_deadline {
            return Err(storage_root_error(DatabaseOpenErrorKind::Busy));
        }
        Ok((work_root, work_identity))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn publish_app_data_reset_fresh_stage(
        parent: &File,
        parent_identity: PlatformIdentity,
        stage_name: &OsStr,
        stage: &File,
        stage_identity: PlatformIdentity,
        canonical_name: &OsStr,
        deadline: Instant,
    ) -> Result<(), DatabaseOpenError> {
        if Instant::now() >= deadline || validate_publication_parent(parent)? != parent_identity {
            return Err(storage_root_error(if Instant::now() >= deadline {
                DatabaseOpenErrorKind::Busy
            } else {
                DatabaseOpenErrorKind::UnsafeStorageRoot
            }));
        }
        validate_named_object(
            parent,
            stage_name,
            stage,
            stage_identity,
            ObjectKind::Directory,
        )?;
        validate_named_absence(parent, canonical_name)?;
        rename_no_replace(parent, stage_name, canonical_name)
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        Ok(())
    }

    /// Descriptor-relative, no-follow open of one possible recovery root.
    /// Absence is data for the recovery state matrix; every other wrong type,
    /// ownership, mode, ACL, or I/O result fails closed.
    pub(super) fn open_existing_private_directory(
        parent: &File,
        name: &OsStr,
    ) -> Result<Option<(File, PlatformIdentity)>, DatabaseOpenError> {
        match openat(
            parent,
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(descriptor) => {
                let directory = File::from(descriptor);
                let identity = validate_file(
                    &directory,
                    ObjectKind::Directory,
                    None,
                    PermissionPolicy::RequirePrivate,
                )?;
                Ok(Some((directory, identity)))
            }
            Err(Errno::ENOENT) => Ok(None),
            Err(Errno::ELOOP | Errno::ENOTDIR) => {
                Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))
            }
            Err(_) => Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            )),
        }
    }

    pub(super) fn prepare_root_for_probe(
        root_path: &Path,
    ) -> Result<PreparedRoot, DatabaseOpenError> {
        let parent_path = root_path
            .parent()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let root_name = root_path
            .file_name()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let parent = open(
            parent_path,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(map_root_open_error)?;
        let publication_parent_identity = validate_publication_parent(&parent)?;
        let deadline = Instant::now()
            .checked_add(CONTROL_OBJECT_LOCK_TIMEOUT)
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
        let publication_fence =
            acquire_root_publication_fence_until(&parent, publication_parent_identity, deadline)?;

        match openat(
            &parent,
            root_name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(descriptor) => {
                let root = File::from(descriptor);
                let identity = validate_file(
                    &root,
                    ObjectKind::Directory,
                    None,
                    PermissionPolicy::InspectOnly,
                )?;
                return Ok(PreparedRoot {
                    directory: root,
                    identity,
                    object_path: root_path.to_path_buf(),
                    state: PreparedRootState::Existing,
                    publication_parent: parent,
                    publication_parent_identity,
                    _publication_fence: publication_fence,
                });
            }
            Err(Errno::ENOENT) => {}
            Err(error) => return Err(map_root_open_error(error)),
        }

        for _ in 0..STAGING_ATTEMPTS {
            let stage_name = random_stage_name()?;
            match mkdirat(&parent, stage_name.as_os_str(), DIRECTORY_MODE) {
                Ok(()) => {
                    let root = openat(
                        &parent,
                        stage_name.as_os_str(),
                        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
                        Mode::empty(),
                    )
                    .map(File::from)
                    .map_err(map_root_open_error)?;
                    let identity = validate_file(
                        &root,
                        ObjectKind::Directory,
                        None,
                        PermissionPolicy::RequirePrivate,
                    )?;
                    return Ok(PreparedRoot {
                        directory: root,
                        identity,
                        object_path: parent_path.join(stage_name),
                        state: PreparedRootState::FreshStaged,
                        publication_parent: parent,
                        publication_parent_identity,
                        _publication_fence: publication_fence,
                    });
                }
                Err(Errno::EEXIST) => {}
                Err(error) => return Err(map_root_create_error(error)),
            }
        }
        Err(storage_root_error(
            DatabaseOpenErrorKind::StorageRootUnavailable,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn publish_prepared_root(
        prepared: &PreparedRoot,
        final_root_path: &Path,
        database_name: &OsStr,
        database_file: &File,
        database_identity: PlatformIdentity,
        marker_name: &OsStr,
        marker_file: &File,
        marker_identity: PlatformIdentity,
    ) -> Result<RootPublicationResult, DatabaseOpenError> {
        if prepared.state != PreparedRootState::FreshStaged
            || prepared.object_path.parent() != final_root_path.parent()
        {
            return Err(storage_root_error(DatabaseOpenErrorKind::InternalState));
        }
        prepared
            .directory
            .sync_all()
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        validate_path_identity(
            &prepared.object_path,
            ObjectKind::Directory,
            prepared.identity,
            PermissionPolicy::RequirePrivate,
        )?;
        for (name, file, identity) in [
            (database_name, database_file, database_identity),
            (marker_name, marker_file, marker_identity),
        ] {
            validate_retained_file(
                file,
                ObjectKind::RegularFile,
                identity,
                PermissionPolicy::RequirePrivate,
            )?;
            validate_path_identity(
                &prepared.object_path.join(name),
                ObjectKind::RegularFile,
                identity,
                PermissionPolicy::RequirePrivate,
            )?;
        }

        let stage_name = prepared
            .object_path
            .file_name()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
        let final_name = final_root_path
            .file_name()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        if validate_publication_parent(&prepared.publication_parent)?
            != prepared.publication_parent_identity
        {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        match rename_no_replace(&prepared.publication_parent, stage_name, final_name) {
            Ok(()) => {}
            Err(Errno::EEXIST | Errno::ENOTEMPTY) => {
                return Ok(RootPublicationResult::Collision);
            }
            Err(_) => {
                return Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                ));
            }
        }
        prepared
            .publication_parent
            .sync_all()
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        validate_path_identity(
            final_root_path,
            ObjectKind::Directory,
            prepared.identity,
            PermissionPolicy::RequirePrivate,
        )?;
        for (name, file, identity) in [
            (database_name, database_file, database_identity),
            (marker_name, marker_file, marker_identity),
        ] {
            validate_retained_file(
                file,
                ObjectKind::RegularFile,
                identity,
                PermissionPolicy::RequirePrivate,
            )?;
            validate_path_identity(
                &final_root_path.join(name),
                ObjectKind::RegularFile,
                identity,
                PermissionPolicy::RequirePrivate,
            )?;
        }
        Ok(RootPublicationResult::Published)
    }

    fn random_stage_name() -> Result<OsString, DatabaseOpenError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let mut name = String::with_capacity(".dux-stage-".len() + random.len() * 2);
        name.push_str(".dux-stage-");
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in random {
            name.push(char::from(HEX[usize::from(byte >> 4)]));
            name.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        Ok(OsString::from(name))
    }

    fn validate_publication_parent(parent: &File) -> Result<PlatformIdentity, DatabaseOpenError> {
        let status = fstat(parent)
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != SFlag::S_IFDIR
            || status.st_uid != geteuid().as_raw()
            || status.st_mode & 0o022 != 0
        {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        reject_granting_acl(parent)?;
        Ok(PlatformIdentity {
            device: status.st_dev as u64,
            inode: status.st_ino as u64,
        })
    }

    pub(super) fn validate_named_object(
        parent: &File,
        name: &OsStr,
        retained: &File,
        retained_identity: PlatformIdentity,
        kind: ObjectKind,
    ) -> Result<(), DatabaseOpenError> {
        let status = fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW)
            .map_err(|_| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        let expected_type = match kind {
            ObjectKind::Directory => SFlag::S_IFDIR,
            ObjectKind::RegularFile => SFlag::S_IFREG,
        };
        let named_identity = PlatformIdentity {
            device: status.st_dev as u64,
            inode: status.st_ino as u64,
        };
        if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != expected_type
            || named_identity != retained_identity
        {
            return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
        }
        validate_retained_file(
            retained,
            kind,
            retained_identity,
            PermissionPolicy::RequirePrivate,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate_data_reset_namespace(
        parent: &File,
        parent_identity: PlatformIdentity,
        root: &File,
        root_identity: PlatformIdentity,
        root_path: &Path,
        root_name: &OsStr,
        detached_name: &OsStr,
        deadline: Instant,
    ) -> Result<(), DatabaseOpenError> {
        fn ensure_before_deadline(deadline: Instant) -> Result<(), DatabaseOpenError> {
            if Instant::now() >= deadline {
                Err(storage_root_error(DatabaseOpenErrorKind::Busy))
            } else {
                Ok(())
            }
        }

        fn valid_component(name: &OsStr) -> bool {
            let bytes = name.as_bytes();
            !bytes.is_empty()
                && bytes != b"."
                && bytes != b".."
                && !bytes.contains(&b'/')
                && !bytes.contains(&0)
        }

        fn validate_named_root(
            parent: &File,
            root_name: &OsStr,
            root_identity: PlatformIdentity,
        ) -> Result<(), DatabaseOpenError> {
            let status = fstatat(parent, root_name, AtFlags::AT_SYMLINK_NOFOLLOW)
                .map_err(|_| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
            let identity = PlatformIdentity {
                device: status.st_dev as u64,
                inode: status.st_ino as u64,
            };
            if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != SFlag::S_IFDIR
                || identity != root_identity
            {
                return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
            }
            Ok(())
        }

        fn validate_detached_absence(
            parent: &File,
            detached_name: &OsStr,
        ) -> Result<(), DatabaseOpenError> {
            match fstatat(parent, detached_name, AtFlags::AT_SYMLINK_NOFOLLOW) {
                Err(Errno::ENOENT) => Ok(()),
                Ok(_) | Err(Errno::ELOOP | Errno::ENOTDIR) => {
                    Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))
                }
                Err(_) => Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                )),
            }
        }

        fn validate_unowned_reserved_children_absent(root: &File) -> Result<(), DatabaseOpenError> {
            for name in [OsStr::new("ai"), OsStr::new("logs")] {
                match fstatat(root, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
                    Err(Errno::ENOENT) => {}
                    Ok(_) | Err(Errno::ELOOP | Errno::ENOTDIR) => {
                        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
                    }
                    Err(_) => {
                        return Err(object_error(DatabaseOpenErrorKind::DatabaseUnavailable));
                    }
                }
            }
            Ok(())
        }

        ensure_before_deadline(deadline)?;
        if !valid_component(root_name)
            || !valid_component(detached_name)
            || root_name == detached_name
            || root_path.file_name() != Some(root_name)
        {
            return Err(storage_root_error(DatabaseOpenErrorKind::InternalState));
        }
        let parent_path = root_path
            .parent()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        if root_identity.device != parent_identity.device {
            // Same-parent detach must not cross into a mount-bound namespace.
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        if validate_publication_parent(parent)? != parent_identity {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        validate_path_identity(
            parent_path,
            ObjectKind::Directory,
            parent_identity,
            PermissionPolicy::InspectOnly,
        )?;
        validate_retained_file(
            root,
            ObjectKind::Directory,
            root_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        validate_named_root(parent, root_name, root_identity)?;
        validate_detached_absence(parent, detached_name)?;
        validate_unowned_reserved_children_absent(root)?;
        ensure_before_deadline(deadline)?;

        // Case-folding filesystems can satisfy lookup through an alias. Bind
        // the witness to the exact spelling that was retained at store open.
        let clone = parent
            .try_clone()
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let owned: OwnedFd = clone.into();
        let mut directory = Dir::from_fd(owned)
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let mut exact_name_seen = false;
        for entry in directory.iter() {
            ensure_before_deadline(deadline)?;
            let entry = entry
                .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
            if entry.file_name().to_bytes() == root_name.as_bytes() {
                exact_name_seen = true;
                break;
            }
        }
        if !exact_name_seen {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }

        validate_named_root(parent, root_name, root_identity)?;
        validate_detached_absence(parent, detached_name)?;
        validate_unowned_reserved_children_absent(root)?;
        if validate_publication_parent(parent)? != parent_identity {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        ensure_before_deadline(deadline)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate_detached_data_reset_namespace(
        parent: &File,
        parent_identity: PlatformIdentity,
        root: &File,
        root_identity: PlatformIdentity,
        root_path: &Path,
        root_name: &OsStr,
        detached_name: &OsStr,
        deadline: Instant,
    ) -> Result<(), DatabaseOpenError> {
        fn ensure_before_deadline(deadline: Instant) -> Result<(), DatabaseOpenError> {
            if Instant::now() >= deadline {
                Err(storage_root_error(DatabaseOpenErrorKind::Busy))
            } else {
                Ok(())
            }
        }

        fn valid_component(name: &OsStr) -> bool {
            let bytes = name.as_bytes();
            !bytes.is_empty()
                && bytes != b"."
                && bytes != b".."
                && !bytes.contains(&b'/')
                && !bytes.contains(&0)
        }

        fn validate_absent(parent: &File, name: &OsStr) -> Result<(), DatabaseOpenError> {
            match fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
                Err(Errno::ENOENT) => Ok(()),
                Ok(_) | Err(Errno::ELOOP | Errno::ENOTDIR) => {
                    Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))
                }
                Err(_) => Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                )),
            }
        }

        fn validate_unowned_reserved_children_absent(root: &File) -> Result<(), DatabaseOpenError> {
            for name in [OsStr::new("ai"), OsStr::new("logs")] {
                match fstatat(root, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
                    Err(Errno::ENOENT) => {}
                    Ok(_) | Err(Errno::ELOOP | Errno::ENOTDIR) => {
                        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
                    }
                    Err(_) => {
                        return Err(object_error(DatabaseOpenErrorKind::DatabaseUnavailable));
                    }
                }
            }
            Ok(())
        }

        ensure_before_deadline(deadline)?;
        if !valid_component(root_name)
            || !valid_component(detached_name)
            || root_name == detached_name
            || root_path.file_name() != Some(root_name)
            || root_identity.device != parent_identity.device
        {
            return Err(storage_root_error(DatabaseOpenErrorKind::InternalState));
        }
        let parent_path = root_path
            .parent()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        if validate_publication_parent(parent)? != parent_identity {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        validate_path_identity(
            parent_path,
            ObjectKind::Directory,
            parent_identity,
            PermissionPolicy::InspectOnly,
        )?;
        validate_retained_file(
            root,
            ObjectKind::Directory,
            root_identity,
            PermissionPolicy::RequirePrivate,
        )?;
        validate_absent(parent, root_name)?;
        validate_named_object(
            parent,
            detached_name,
            root,
            root_identity,
            ObjectKind::Directory,
        )?;
        validate_unowned_reserved_children_absent(root)?;

        let clone = parent
            .try_clone()
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let owned: OwnedFd = clone.into();
        let mut directory = Dir::from_fd(owned)
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let mut exact_stage_seen = false;
        for entry in directory.iter() {
            ensure_before_deadline(deadline)?;
            let entry = entry
                .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
            if entry.file_name().to_bytes() == detached_name.as_bytes() {
                exact_stage_seen = true;
                break;
            }
        }
        if !exact_stage_seen {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }

        validate_absent(parent, root_name)?;
        validate_named_object(
            parent,
            detached_name,
            root,
            root_identity,
            ObjectKind::Directory,
        )?;
        validate_unowned_reserved_children_absent(root)?;
        if validate_publication_parent(parent)? != parent_identity {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        ensure_before_deadline(deadline)
    }

    /// Validate the only post-publication two-generation shape: the old exact
    /// root remains at its transaction stage while a distinct fresh root is
    /// present at the canonical name.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate_detached_data_with_fresh_namespace(
        parent: &File,
        parent_identity: PlatformIdentity,
        old_root: &File,
        old_identity: PlatformIdentity,
        fresh_root: &File,
        fresh_identity: PlatformIdentity,
        root_path: &Path,
        root_name: &OsStr,
        detached_name: &OsStr,
        deadline: Instant,
    ) -> Result<(), DatabaseOpenError> {
        if Instant::now() >= deadline
            || root_name == detached_name
            || root_path.file_name() != Some(root_name)
            || old_identity == fresh_identity
            || old_identity.device != parent_identity.device
            || fresh_identity.device != parent_identity.device
            || validate_publication_parent(parent)? != parent_identity
        {
            return Err(storage_root_error(if Instant::now() >= deadline {
                DatabaseOpenErrorKind::Busy
            } else {
                DatabaseOpenErrorKind::UnsafeStorageRoot
            }));
        }
        let parent_path = root_path
            .parent()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        validate_path_identity(
            parent_path,
            ObjectKind::Directory,
            parent_identity,
            PermissionPolicy::InspectOnly,
        )?;
        validate_named_object(
            parent,
            detached_name,
            old_root,
            old_identity,
            ObjectKind::Directory,
        )?;
        validate_named_object(
            parent,
            root_name,
            fresh_root,
            fresh_identity,
            ObjectKind::Directory,
        )?;
        for name in [OsStr::new("ai"), OsStr::new("logs")] {
            match fstatat(old_root, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
                Err(Errno::ENOENT) => {}
                Ok(_) | Err(Errno::ELOOP | Errno::ENOTDIR) => {
                    return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
                }
                Err(_) => {
                    return Err(object_error(DatabaseOpenErrorKind::DatabaseUnavailable));
                }
            }
        }
        if validate_publication_parent(parent)? != parent_identity || Instant::now() >= deadline {
            return Err(storage_root_error(if Instant::now() >= deadline {
                DatabaseOpenErrorKind::Busy
            } else {
                DatabaseOpenErrorKind::UnsafeStorageRoot
            }));
        }
        Ok(())
    }

    pub(super) fn detach_data_root_no_replace(
        parent: &File,
        source: &OsStr,
        source_directory: &File,
        source_identity: PlatformIdentity,
        destination: &OsStr,
    ) -> Result<(), DatabaseOpenError> {
        validate_named_object(
            parent,
            source,
            source_directory,
            source_identity,
            ObjectKind::Directory,
        )?;
        rename_no_replace(parent, source, destination)
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))
    }

    #[cfg(test)]
    pub(super) fn create_test_data_reset_collision(
        parent: &File,
        destination: &OsStr,
    ) -> Result<(), DatabaseOpenError> {
        let collision = openat(
            parent,
            destination,
            OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::S_IRUSR | Mode::S_IWUSR,
        )
        .map(File::from)
        .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        collision
            .write_at(b"foreign race collision", 0)
            .and_then(|_| collision.sync_all())
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))
    }

    #[cfg(target_os = "linux")]
    pub(super) fn rename_no_replace(
        parent: &File,
        source: &OsStr,
        destination: &OsStr,
    ) -> Result<(), Errno> {
        let source = CString::new(source.as_bytes()).map_err(|_| Errno::EINVAL)?;
        let destination = CString::new(destination.as_bytes()).map_err(|_| Errno::EINVAL)?;
        // SAFETY: both names are NUL-terminated single components and the live
        // parent descriptor is used for source and destination. renameat2 with
        // RENAME_NOREPLACE is atomic and cannot overwrite an existing root.
        let result = unsafe {
            nix::libc::syscall(
                // DUX-DESTRUCTIVE: allow=storage-root-linux-publish -- atomically publish only the private retained DUX stage without replacing an existing root
                nix::libc::SYS_renameat2,
                parent.as_raw_fd(),
                source.as_ptr(),
                parent.as_raw_fd(),
                destination.as_ptr(),
                nix::libc::RENAME_NOREPLACE,
            )
        };
        Errno::result(result).map(drop)
    }

    #[cfg(target_os = "macos")]
    pub(super) fn rename_no_replace(
        parent: &File,
        source: &OsStr,
        destination: &OsStr,
    ) -> Result<(), Errno> {
        use std::ffi::{c_char, c_int, c_uint};

        const RENAME_EXCL: c_uint = 0x0000_0004;
        const RENAME_NOFOLLOW_ANY: c_uint = 0x0000_0010;
        unsafe extern "C" {
            fn renameatx_np(
                from_fd: c_int,
                from: *const c_char,
                to_fd: c_int,
                to: *const c_char,
                flags: c_uint,
            ) -> c_int;
        }

        let source = CString::new(source.as_bytes()).map_err(|_| Errno::EINVAL)?;
        let destination = CString::new(destination.as_bytes()).map_err(|_| Errno::EINVAL)?;
        // SAFETY: both names are NUL-terminated single components and the live
        // parent descriptor scopes both sides. EXCL forbids replacement and
        // NOFOLLOW_ANY rejects symlink traversal anywhere in the operation.
        let result = unsafe {
            // DUX-DESTRUCTIVE: allow=storage-root-macos-publish -- atomically publish only the private retained DUX stage without replacing an existing root
            renameatx_np(
                parent.as_raw_fd(),
                source.as_ptr(),
                parent.as_raw_fd(),
                destination.as_ptr(),
                RENAME_EXCL | RENAME_NOFOLLOW_ANY,
            )
        };
        Errno::result(result).map(drop)
    }

    pub(super) fn create_private_file_exclusive(
        root_directory: &File,
        _root_path: &Path,
        name: &std::ffi::OsStr,
    ) -> Result<(File, PlatformIdentity), DatabaseOpenError> {
        match openat(
            root_directory,
            name,
            OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            FILE_MODE,
        ) {
            Ok(descriptor) => {
                let file = File::from(descriptor);
                let identity = validate_file(
                    &file,
                    ObjectKind::RegularFile,
                    None,
                    PermissionPolicy::RepairPrivate,
                )?;
                Ok((file, identity))
            }
            Err(Errno::ELOOP | Errno::EMLINK | Errno::EISDIR | Errno::ENOTDIR) => {
                Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
            }
            Err(_) => Err(object_error(DatabaseOpenErrorKind::DatabaseUnavailable)),
        }
    }

    pub(super) fn open_existing_file(
        root_directory: &File,
        _root_path: &Path,
        name: &std::ffi::OsStr,
        permissions: PermissionPolicy,
    ) -> Result<Option<(File, PlatformIdentity)>, DatabaseOpenError> {
        match openat(
            root_directory,
            name,
            OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(descriptor) => {
                let file = File::from(descriptor);
                let identity = validate_file(&file, ObjectKind::RegularFile, None, permissions)?;
                Ok(Some((file, identity)))
            }
            Err(Errno::ENOENT) => Ok(None),
            Err(Errno::ELOOP | Errno::EMLINK | Errno::EISDIR | Errno::ENOTDIR) => {
                Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
            }
            Err(_) => Err(object_error(DatabaseOpenErrorKind::DatabaseUnavailable)),
        }
    }

    pub(super) fn open_existing_control_file(
        root_directory: &File,
        root_path: &Path,
        name: &std::ffi::OsStr,
        permissions: PermissionPolicy,
    ) -> Result<Option<(File, PlatformIdentity)>, DatabaseOpenError> {
        open_existing_file(root_directory, root_path, name, permissions)
    }

    pub(super) fn open_existing_writer_file(
        root_directory: &File,
        _root_path: &Path,
        name: &std::ffi::OsStr,
        permissions: PermissionPolicy,
    ) -> Result<Option<(File, PlatformIdentity)>, DatabaseOpenError> {
        match openat(
            root_directory,
            name,
            OFlag::O_RDWR | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(descriptor) => {
                let file = File::from(descriptor);
                let identity = validate_file(&file, ObjectKind::RegularFile, None, permissions)?;
                Ok(Some((file, identity)))
            }
            Err(Errno::ENOENT) => Ok(None),
            Err(Errno::ELOOP | Errno::EMLINK | Errno::EISDIR | Errno::ENOTDIR) => {
                Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
            }
            Err(_) => Err(object_error(DatabaseOpenErrorKind::DatabaseUnavailable)),
        }
    }

    pub(super) fn resolve_existing_file_name(
        root_directory: &File,
        _root_path: &Path,
        _requested_name: &OsStr,
        expected: PlatformIdentity,
    ) -> Result<std::ffi::OsString, DatabaseOpenError> {
        let clone = root_directory
            .try_clone()
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let owned: OwnedFd = clone.into();
        let mut directory = Dir::from_fd(owned)
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        for entry in directory.iter() {
            let entry = entry
                .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            let name = OsStr::from_bytes(bytes);
            if let Ok(Some((file, identity))) = open_existing_file(
                root_directory,
                Path::new("."),
                name,
                PermissionPolicy::InspectOnly,
            ) {
                drop(file);
                if identity == expected {
                    return Ok(name.to_os_string());
                }
            }
        }
        Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
    }

    pub(super) fn secure_retained_file(
        file: &File,
        kind: ObjectKind,
        expected: PlatformIdentity,
    ) -> Result<(), DatabaseOpenError> {
        validate_file(file, kind, Some(expected), PermissionPolicy::RepairPrivate).map(drop)
    }

    pub(super) fn validate_retained_file(
        file: &File,
        kind: ObjectKind,
        expected: PlatformIdentity,
        permissions: PermissionPolicy,
    ) -> Result<(), DatabaseOpenError> {
        validate_file(file, kind, Some(expected), permissions).map(drop)
    }

    pub(super) fn validate_path_identity(
        path: &Path,
        kind: ObjectKind,
        expected: PlatformIdentity,
        permissions: PermissionPolicy,
    ) -> Result<(), DatabaseOpenError> {
        let flags = match kind {
            ObjectKind::Directory => {
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
            }
            ObjectKind::RegularFile => {
                OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
            }
        };
        let file =
            open(path, flags, Mode::empty())
                .map(File::from)
                .map_err(|error| match kind {
                    ObjectKind::Directory => map_root_open_error(error),
                    ObjectKind::RegularFile => {
                        object_error(DatabaseOpenErrorKind::UnsafeStorageObject)
                    }
                })?;
        validate_file(&file, kind, Some(expected), permissions).map(drop)
    }

    pub(super) fn read_exact_at(
        file: &File,
        mut buffer: &mut [u8],
        mut offset: u64,
    ) -> std::io::Result<()> {
        while !buffer.is_empty() {
            let read = file.read_at(buffer, offset)?;
            if read == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
            }
            offset = offset
                .checked_add(read as u64)
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
            buffer = &mut buffer[read..];
        }
        Ok(())
    }

    pub(super) fn write_all_at(
        file: &File,
        mut buffer: &[u8],
        mut offset: u64,
    ) -> std::io::Result<()> {
        while !buffer.is_empty() {
            let written = file.write_at(buffer, offset)?;
            if written == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
            }
            offset = offset
                .checked_add(written as u64)
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
            buffer = &buffer[written..];
        }
        Ok(())
    }

    pub(super) fn root_contains_only(
        root_directory: &File,
        _root_path: &Path,
        allowed: &[&OsStr],
    ) -> Result<bool, DatabaseOpenError> {
        root_contains_only_mode(root_directory, allowed, true)
    }

    pub(super) fn root_contains_only_exact(
        root_directory: &File,
        allowed: &[&OsStr],
    ) -> Result<bool, DatabaseOpenError> {
        root_contains_only_mode(root_directory, allowed, false)
    }

    /// Exact reset-root inventory under the caller's one monotonic deadline.
    /// This deliberately does not mint the ordinary helper's private 250 ms
    /// budget, so a post-effect readback cannot run beyond the shared reset
    /// certainty window.
    pub(super) fn root_contains_only_exact_until(
        root_directory: &File,
        allowed: &[&OsStr],
        deadline: Instant,
    ) -> Result<bool, DatabaseOpenError> {
        let actual = directory_entry_names_until(
            root_directory,
            allowed.len(),
            ROOT_INVENTORY_MAX_NAME_BYTES,
            deadline,
        )?;
        let mut expected = allowed
            .iter()
            .map(|name| (*name).to_os_string())
            .collect::<Vec<_>>();
        expected.sort_unstable_by(|left, right| {
            left.as_os_str()
                .as_bytes()
                .cmp(right.as_os_str().as_bytes())
        });
        Ok(actual == expected)
    }

    fn root_contains_only_mode(
        root_directory: &File,
        allowed: &[&OsStr],
        admit_snapshot_stages_and_case_aliases: bool,
    ) -> Result<bool, DatabaseOpenError> {
        let deadline = Instant::now()
            .checked_add(ROOT_INVENTORY_TIMEOUT)
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let maximum_entries = allowed
            .len()
            .checked_add(MAX_SNAPSHOT_STAGES)
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let clone = root_directory
            .try_clone()
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let owned: OwnedFd = clone.into();
        let mut directory = Dir::from_fd(owned)
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        let mut entry_count = 0_usize;
        let mut stage_count = 0_usize;
        let mut name_bytes = 0_usize;
        for entry in directory.iter() {
            if Instant::now() > deadline {
                return Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                ));
            }
            let entry = entry
                .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
            if Instant::now() > deadline {
                return Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                ));
            }
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            entry_count = entry_count
                .checked_add(1)
                .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
            name_bytes = name_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
            if name_bytes > ROOT_INVENTORY_MAX_NAME_BYTES {
                return Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                ));
            }
            if entry_count > maximum_entries {
                return Ok(false);
            }
            let actual_name = OsStr::from_bytes(bytes);
            if allowed.contains(&actual_name) {
                continue;
            }
            if admit_snapshot_stages_and_case_aliases
                && is_canonical_snapshot_stage_name(actual_name)
            {
                stage_count = stage_count.checked_add(1).ok_or_else(|| {
                    storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable)
                })?;
                if stage_count > MAX_SNAPSHOT_STAGES {
                    return Ok(false);
                }
                validate_snapshot_stage_directory(root_directory, actual_name)?;
                continue;
            }
            #[cfg(target_os = "macos")]
            if admit_snapshot_stages_and_case_aliases
                && names_resolve_to_same_entry(root_directory, actual_name, allowed)?
            {
                continue;
            }
            return Ok(false);
        }
        if Instant::now() > deadline {
            return Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ));
        }
        Ok(true)
    }

    fn validate_snapshot_stage_directory(
        root_directory: &File,
        name: &OsStr,
    ) -> Result<(), DatabaseOpenError> {
        let status = fstatat(root_directory, name, AtFlags::AT_SYMLINK_NOFOLLOW)
            .map_err(|_| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        let kind = SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT;
        let mode = status.st_mode & 0o7777;
        if kind != SFlag::S_IFDIR || status.st_uid != geteuid().as_raw() || mode & !0o700 != 0 {
            return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
        }

        // `mkdirat(..., 0700)` is filtered by the process umask before the
        // following `fchmod`. A crash in that tiny interval can therefore
        // leave an owner-owned canonical stage with a stricter subset of 0700,
        // including 000. It is safe to tolerate that opaque, non-traversable
        // namespace entry so the owning store can reopen; this does not prove
        // marker ownership and never grants cleanup authority. Exact 0700
        // stages remain fully opened and ACL-validated below.
        if mode != 0o700 {
            return Ok(());
        }
        let directory = openat(
            root_directory,
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
        validate_file(
            &directory,
            ObjectKind::Directory,
            None,
            PermissionPolicy::RequirePrivate,
        )?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn names_resolve_to_same_entry(
        root_directory: &File,
        actual_name: &OsStr,
        allowed: &[&OsStr],
    ) -> Result<bool, DatabaseOpenError> {
        let Some(actual_identity) = open_named_entry_identity(root_directory, actual_name)? else {
            return Ok(false);
        };
        for allowed_name in allowed {
            if open_named_entry_identity(root_directory, allowed_name)? == Some(actual_identity) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[cfg(target_os = "macos")]
    fn open_named_entry_identity(
        root_directory: &File,
        name: &OsStr,
    ) -> Result<Option<PlatformIdentity>, DatabaseOpenError> {
        match open_existing_file(
            root_directory,
            Path::new("."),
            name,
            PermissionPolicy::InspectOnly,
        ) {
            Ok(Some((_file, identity))) => return Ok(Some(identity)),
            Ok(None) => return Ok(None),
            Err(error) if error.kind != DatabaseOpenErrorKind::UnsafeStorageObject => {
                return Err(error);
            }
            Err(_) => {}
        }

        match openat(
            root_directory,
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(descriptor) => validate_file(
                &File::from(descriptor),
                ObjectKind::Directory,
                None,
                PermissionPolicy::InspectOnly,
            )
            .map(Some),
            Err(Errno::ENOENT) => Ok(None),
            Err(Errno::ELOOP | Errno::ENOTDIR) => {
                Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
            }
            Err(_) => Err(object_error(DatabaseOpenErrorKind::DatabaseUnavailable)),
        }
    }

    fn validate_file(
        file: &File,
        kind: ObjectKind,
        expected: Option<PlatformIdentity>,
        permissions: PermissionPolicy,
    ) -> Result<PlatformIdentity, DatabaseOpenError> {
        let mut status = fstat(file).map_err(|_| unavailable_for(kind))?;
        let expected_type = match kind {
            ObjectKind::Directory => SFlag::S_IFDIR,
            ObjectKind::RegularFile => SFlag::S_IFREG,
        };
        if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != expected_type {
            return Err(unsafe_for(kind));
        }
        if status.st_uid != geteuid().as_raw() {
            return Err(object_error(DatabaseOpenErrorKind::OwnershipMismatch));
        }
        if matches!(kind, ObjectKind::RegularFile) && status.st_nlink != 1 {
            return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
        }

        reject_extended_acl(file)?;

        let required_mode: Mode = match kind {
            ObjectKind::Directory => DIRECTORY_MODE,
            ObjectKind::RegularFile => FILE_MODE,
        };
        let mode_is_private = status.st_mode & 0o7777 == required_mode.bits();
        match permissions {
            PermissionPolicy::InspectOnly => {}
            PermissionPolicy::RepairPrivate if !mode_is_private => {
                fchmod(file, required_mode)
                    .map_err(|_| object_error(DatabaseOpenErrorKind::UnsafePermissions))?;
                status = fstat(file).map_err(|_| unavailable_for(kind))?;
                reject_extended_acl(file)?;
                if status.st_mode & 0o7777 != required_mode.bits()
                    || status.st_uid != geteuid().as_raw()
                    || (matches!(kind, ObjectKind::RegularFile) && status.st_nlink != 1)
                {
                    return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
                }
            }
            PermissionPolicy::RequirePrivate if !mode_is_private => {
                return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
            }
            PermissionPolicy::RepairPrivate | PermissionPolicy::RequirePrivate => {}
        }

        let identity = PlatformIdentity {
            device: status.st_dev as u64,
            inode: status.st_ino as u64,
        };
        if expected.is_some_and(|expected| expected != identity) {
            return Err(unsafe_for(kind));
        }
        Ok(identity)
    }

    #[cfg(target_os = "macos")]
    fn reject_extended_acl(file: &File) -> Result<(), DatabaseOpenError> {
        use std::ffi::{c_int, c_void};

        const ACL_TYPE_EXTENDED: c_int = 0x100;
        const ACL_FIRST_ENTRY: c_int = 0;

        unsafe extern "C" {
            fn acl_get_fd_np(fd: c_int, acl_type: c_int) -> *mut c_void;
            fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
            fn acl_free(object: *mut c_void) -> c_int;
        }

        // SAFETY: the file owns a live descriptor and Darwin returns an
        // independently allocated ACL object for this descriptor.
        let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
        if acl.is_null() {
            if Errno::last() == Errno::ENOENT {
                return Ok(());
            }
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        let mut entry = std::ptr::null_mut();
        // SAFETY: `acl` is live and `entry` points to writable pointer storage.
        let entry_result = unsafe { acl_get_entry(acl, ACL_FIRST_ENTRY, &raw mut entry) };
        // SAFETY: `acl` came from acl_get_fd_np and is freed exactly once.
        let free_result = unsafe { acl_free(acl) };
        if free_result != 0 || entry_result < 0 {
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        // Darwin returns zero both when iteration succeeds and when an empty
        // ACL has no entry; the output pointer distinguishes those cases.
        if !entry.is_null() {
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn reject_extended_acl(_file: &File) -> Result<(), DatabaseOpenError> {
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn reject_granting_acl(file: &File) -> Result<(), DatabaseOpenError> {
        use std::ffi::{c_int, c_void};

        const ACL_TYPE_EXTENDED: c_int = 0x100;
        const ACL_FIRST_ENTRY: c_int = 0;
        const ACL_NEXT_ENTRY: c_int = -1;
        const ACL_EXTENDED_DENY: c_int = 2;
        const ACL_MAX_ENTRIES: usize = 128;

        unsafe extern "C" {
            fn acl_get_fd_np(fd: c_int, acl_type: c_int) -> *mut c_void;
            fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
            fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_int) -> c_int;
            fn acl_free(object: *mut c_void) -> c_int;
        }

        // SAFETY: the file owns a live descriptor and Darwin returns an
        // independently allocated ACL object for that descriptor.
        let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
        if acl.is_null() {
            if Errno::last() == Errno::ENOENT {
                return Ok(());
            }
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }

        let inspection = (|| {
            let mut selector = ACL_FIRST_ENTRY;
            for _ in 0..ACL_MAX_ENTRIES {
                let mut entry = std::ptr::null_mut();
                // SAFETY: `acl` is live and `entry` points to writable storage.
                if unsafe { acl_get_entry(acl, selector, &raw mut entry) } < 0 {
                    if selector == ACL_NEXT_ENTRY && Errno::last() == Errno::EINVAL {
                        return Ok(());
                    }
                    return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
                }
                if entry.is_null() {
                    return Ok(());
                }
                let mut tag = 0;
                // SAFETY: `entry` belongs to the live ACL and `tag` is writable.
                if unsafe { acl_get_tag_type(entry, &raw mut tag) } != 0 || tag != ACL_EXTENDED_DENY
                {
                    return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
                }
                selector = ACL_NEXT_ENTRY;
            }
            Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))
        })();
        // SAFETY: `acl` came from acl_get_fd_np and is freed exactly once.
        if unsafe { acl_free(acl) } != 0 {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        inspection
    }

    #[cfg(target_os = "linux")]
    fn reject_granting_acl(_file: &File) -> Result<(), DatabaseOpenError> {
        Ok(())
    }

    fn map_root_open_error(error: Errno) -> DatabaseOpenError {
        match error {
            Errno::ELOOP | Errno::ENOTDIR => {
                storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot)
            }
            _ => storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable),
        }
    }

    fn map_root_create_error(error: Errno) -> DatabaseOpenError {
        match error {
            Errno::ELOOP | Errno::ENOTDIR => {
                storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot)
            }
            _ => storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable),
        }
    }

    fn unsafe_for(kind: ObjectKind) -> DatabaseOpenError {
        match kind {
            ObjectKind::Directory => storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot),
            ObjectKind::RegularFile => object_error(DatabaseOpenErrorKind::UnsafeStorageObject),
        }
    }

    fn unavailable_for(kind: ObjectKind) -> DatabaseOpenError {
        match kind {
            ObjectKind::Directory => {
                storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable)
            }
            ObjectKind::RegularFile => object_error(DatabaseOpenErrorKind::DatabaseUnavailable),
        }
    }
}

#[cfg(windows)]
#[path = "storage/windows.rs"]
mod platform;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
    use std::ffi::OsStr;
    use std::fs::File;
    use std::path::Path;

    use super::{
        DatabaseOpenError, DatabaseOpenErrorKind, ObjectKind, PermissionPolicy, PlatformIdentity,
        PreparedRoot, RootPublicationResult,
    };

    pub(super) fn sync_directory(_directory: &File) -> Result<(), DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn file_usage(_file: &File) -> Result<(u64, u64), DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn prepare_root_for_probe(
        _root_path: &Path,
    ) -> Result<PreparedRoot, DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn publish_prepared_root(
        _prepared: &PreparedRoot,
        _final_root_path: &Path,
        _database_name: &OsStr,
        _database_file: &File,
        _database_identity: PlatformIdentity,
        _marker_name: &OsStr,
        _marker_file: &File,
        _marker_identity: PlatformIdentity,
    ) -> Result<RootPublicationResult, DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn create_private_file_exclusive(
        _root_directory: &File,
        _root_path: &Path,
        _name: &std::ffi::OsStr,
    ) -> Result<(File, PlatformIdentity), DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn open_existing_file(
        _root_directory: &File,
        _root_path: &Path,
        _name: &std::ffi::OsStr,
        _permissions: PermissionPolicy,
    ) -> Result<Option<(File, PlatformIdentity)>, DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn open_existing_control_file(
        _root_directory: &File,
        _root_path: &Path,
        _name: &std::ffi::OsStr,
        _permissions: PermissionPolicy,
    ) -> Result<Option<(File, PlatformIdentity)>, DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn open_existing_writer_file(
        _root_directory: &File,
        _root_path: &Path,
        _name: &std::ffi::OsStr,
        _permissions: PermissionPolicy,
    ) -> Result<Option<(File, PlatformIdentity)>, DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn resolve_existing_file_name(
        _root_directory: &File,
        _root_path: &Path,
        _requested_name: &OsStr,
        _expected: PlatformIdentity,
    ) -> Result<std::ffi::OsString, DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn secure_retained_file(
        _file: &File,
        _kind: ObjectKind,
        _expected: PlatformIdentity,
    ) -> Result<(), DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn validate_retained_file(
        _file: &File,
        _kind: ObjectKind,
        _expected: PlatformIdentity,
        _permissions: PermissionPolicy,
    ) -> Result<(), DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn validate_path_identity(
        _path: &Path,
        _kind: ObjectKind,
        _expected: PlatformIdentity,
        _permissions: PermissionPolicy,
    ) -> Result<(), DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }

    pub(super) fn read_exact_at(
        _file: &File,
        _buffer: &mut [u8],
        _offset: u64,
    ) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }

    pub(super) fn write_all_at(_file: &File, _buffer: &[u8], _offset: u64) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }

    pub(super) fn root_contains_only(
        _root_directory: &File,
        _root_path: &Path,
        _allowed: &[&OsStr],
    ) -> Result<bool, DatabaseOpenError> {
        Err(DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;

    fn database_path(temp: &TempDir) -> PathBuf {
        temp.path().join("owned").join("dux.sqlite3")
    }

    fn initialize_dux_header(database: &Path) {
        let connection = rusqlite::Connection::open(database).unwrap();
        connection
            .pragma_update(
                None,
                "application_id",
                super::super::migrations::DUX_APPLICATION_ID,
            )
            .unwrap();
    }

    #[test]
    fn physical_usage_requires_writer_guard_and_counts_known_sidecars() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        let writer = storage
            .acquire_writer_lock(Duration::from_millis(20))
            .unwrap();
        platform::write_all_at(&storage.database_file, &[0_u8; 123], 0).unwrap();
        storage.database_file.sync_all().unwrap();
        storage.mark_initialized().unwrap();

        let before = storage.observe_physical_usage(&writer).unwrap();
        assert!(before.logical_bytes >= 123);
        assert!(before.charged_bytes >= before.logical_bytes);
        assert!(before.charged_bytes >= before.allocated_bytes);
        let foreign_temp = TempDir::new().unwrap();
        let foreign_storage = SecureStorePaths::prepare(&database_path(&foreign_temp)).unwrap();
        let foreign_writer = foreign_storage
            .acquire_writer_lock(Duration::from_millis(20))
            .unwrap();
        assert_eq!(
            storage
                .observe_physical_usage(&foreign_writer)
                .unwrap_err()
                .kind,
            DatabaseOpenErrorKind::InternalState
        );

        let wal_name = suffixed_name(database.file_name().unwrap(), "-wal");
        let (wal, _) = platform::create_private_file_exclusive(
            &storage.root_directory,
            &storage.root_path,
            &wal_name,
        )
        .unwrap();
        platform::write_all_at(&wal, &[0_u8; 321], 0).unwrap();
        wal.sync_all().unwrap();
        platform::sync_directory(&storage.root_directory).unwrap();

        let after = storage.observe_physical_usage(&writer).unwrap();
        assert_eq!(
            after.logical_bytes,
            before.logical_bytes.checked_add(321).unwrap()
        );
        assert!(after.allocated_bytes >= before.allocated_bytes);
        assert!(after.charged_bytes >= after.logical_bytes);
        assert!(after.charged_bytes >= after.allocated_bytes);
    }

    #[cfg(unix)]
    fn create_private_snapshot_stage(root: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let stage = root.join(name);
        fs::create_dir(&stage).unwrap();
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).unwrap();
        stage
    }

    #[cfg(unix)]
    fn open_and_close_store(database: &Path) {
        use super::super::store::StoreCoordinator;

        drop(StoreCoordinator::open(database).unwrap());
    }

    #[cfg(unix)]
    fn snapshot_stage_name(sequence: usize) -> String {
        format!("{SNAPSHOT_STAGE_PREFIX}{sequence:032x}")
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn prepare_old_database_draining_fixture(
        temp: &TempDir,
        sidecar_suffixes: &[&str],
    ) -> (PathBuf, (u64, u64), &'static OsStr, (u64, u64)) {
        let database = database_path(temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        storage.mark_initialized().unwrap();
        let database_name = database.file_name().unwrap();
        for suffix in sidecar_suffixes {
            let name = suffixed_name(database_name, suffix);
            let (file, _) = platform::create_private_file_exclusive(
                &storage.root_directory,
                &storage.root_path,
                &name,
            )
            .unwrap();
            platform::write_all_at(&file, suffix.as_bytes(), 0).unwrap();
            file.sync_all().unwrap();
        }
        platform::sync_directory(&storage.root_directory).unwrap();
        let identity = (storage.root_identity.device, storage.root_identity.inode);
        let detached = OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff");
        platform::detach_data_root_no_replace(
            &storage.publication_parent,
            storage.root_path.file_name().unwrap(),
            &storage.root_directory,
            storage.root_identity,
            detached,
        )
        .unwrap();
        platform::sync_directory(&storage.publication_parent).unwrap();
        let fresh_stage = OsStr::new(".dux-reset-fresh-00112233445566778899aabbccddeeff");
        let transaction = "00112233445566778899aabbccddeeff";
        let (fresh, fresh_identity) = platform::prepare_app_data_reset_fresh_stage(
            &storage.publication_parent,
            storage.publication_parent_identity,
            fresh_stage,
            database_name,
            transaction,
            identity,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        platform::publish_app_data_reset_fresh_stage(
            &storage.publication_parent,
            storage.publication_parent_identity,
            fresh_stage,
            &fresh,
            fresh_identity,
            storage.root_path.file_name().unwrap(),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        platform::sync_directory(&storage.publication_parent).unwrap();
        drop(storage);
        (
            database,
            identity,
            detached,
            (fresh_identity.device, fresh_identity.inode),
        )
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn old_database_open_binding(
        old_identity: (u64, u64),
        detached_name: &OsStr,
        fresh_identity: (u64, u64),
    ) -> AppDataResetOldDatabaseOpenBinding<'_> {
        AppDataResetOldDatabaseOpenBinding::new(
            "00112233445566778899aabbccddeeff",
            old_identity,
            fresh_identity,
            detached_name,
            OsStr::new(".dux-reset-fresh-00112233445566778899aabbccddeeff"),
        )
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn old_database_draining_removes_lexical_sidecars_then_main_one_per_open() {
        use std::os::unix::fs::MetadataExt;

        let temp = TempDir::new().unwrap();
        let (database, identity, detached, fresh_identity) =
            prepare_old_database_draining_fixture(&temp, &["-wal", "-journal", "-shm"]);
        let detached_root = temp.path().join(detached);
        let expected_order = [
            suffixed_name(database.file_name().unwrap(), "-journal"),
            suffixed_name(database.file_name().unwrap(), "-shm"),
            suffixed_name(database.file_name().unwrap(), "-wal"),
            database.file_name().unwrap().to_os_string(),
        ];

        for (index, expected_removed) in expected_order.iter().enumerate() {
            let before = std::fs::read_dir(&detached_root)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>();
            let parent = std::fs::metadata(temp.path()).unwrap();
            SecureStorePaths::with_app_data_reset_draining_old_database_until(
                &database,
                old_database_open_binding(identity, detached, fresh_identity),
                Instant::now() + Duration::from_secs(1),
                |admission| match admission {
                    AppDataResetOldDatabaseDrainingAdmission::SnapshotStorePresent => {
                        panic!("snapshot store unexpectedly present in payload fixture")
                    }
                    AppDataResetOldDatabaseDrainingAdmission::PayloadsRemain(candidate) => {
                        assert_eq!(
                            candidate.state(),
                            AppDataResetOldDatabasePayloadState::DatabasePresent
                        );
                        assert!(candidate.is_bound_to(
                            AppDataResetOldDatabaseAuthorityBinding::new(
                                old_database_open_binding(identity, detached, fresh_identity),
                                (parent.dev(), parent.ino()),
                                OsStr::new("owned"),
                            )
                        ));
                        let deadline = candidate.deadline();
                        let completion = candidate
                            .drain_one(AppDataResetOldDatabasePayloadDrainAuthority::for_test(
                                deadline,
                            ))
                            .unwrap();
                        assert!(completion.post_effect_deadline() > Instant::now());
                        let progress = completion.into_progress();
                        assert_eq!(progress.removed_objects(), 1);
                        assert_eq!(
                            progress.old_database_payload_has_more(),
                            index + 1 < expected_order.len()
                        );
                    }
                    AppDataResetOldDatabaseDrainingAdmission::Absent(_) => {
                        panic!("payload absence arrived before every payload was removed")
                    }
                },
            )
            .unwrap();
            assert!(!detached_root.join(expected_removed).exists());
            for retained in before.into_iter().filter(|name| name != expected_removed) {
                assert!(detached_root.join(retained).exists());
            }
        }

        SecureStorePaths::with_app_data_reset_draining_old_database_until(
            &database,
            old_database_open_binding(identity, detached, fresh_identity),
            Instant::now() + Duration::from_secs(1),
            |admission| match admission {
                AppDataResetOldDatabaseDrainingAdmission::SnapshotStorePresent => {
                    panic!("snapshot store unexpectedly present in controls-only fixture")
                }
                AppDataResetOldDatabaseDrainingAdmission::PayloadsRemain(_) => {
                    panic!("controls-only tail exposed another payload")
                }
                AppDataResetOldDatabaseDrainingAdmission::Absent(absent) => {
                    assert_eq!(
                        absent.state(),
                        AppDataResetOldDatabasePayloadState::DatabaseAbsentControlsFull
                    );
                    absent
                        .revalidate_until(Instant::now() + Duration::from_secs(1))
                        .unwrap();
                }
            },
        )
        .unwrap();
        let mut remaining = std::fs::read_dir(detached_root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        remaining.sort();
        assert_eq!(
            remaining,
            [
                cleanup_lock_name(database.file_name().unwrap()),
                cleanup_lock_ready_name(database.file_name().unwrap()),
                initialization_name(database.file_name().unwrap()),
                lock_name(database.file_name().unwrap()),
            ]
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn old_database_draining_refuses_unknown_or_sidecar_without_main() {
        let unknown = TempDir::new().unwrap();
        let (database, identity, detached, fresh_identity) =
            prepare_old_database_draining_fixture(&unknown, &[]);
        let detached_root = unknown.path().join(detached);
        std::fs::write(detached_root.join("foreign"), b"preserve").unwrap();
        assert!(
            SecureStorePaths::with_app_data_reset_draining_old_database_until(
                &database,
                old_database_open_binding(identity, detached, fresh_identity),
                Instant::now() + Duration::from_secs(1),
                |_| (),
            )
            .is_err()
        );
        assert!(detached_root.join("foreign").exists());

        let partial = TempDir::new().unwrap();
        let (database, identity, detached, fresh_identity) =
            prepare_old_database_draining_fixture(&partial, &[]);
        SecureStorePaths::with_app_data_reset_draining_old_database_until(
            &database,
            old_database_open_binding(identity, detached, fresh_identity),
            Instant::now() + Duration::from_secs(1),
            |admission| match admission {
                AppDataResetOldDatabaseDrainingAdmission::SnapshotStorePresent => {
                    panic!("snapshot store unexpectedly present in partial fixture")
                }
                AppDataResetOldDatabaseDrainingAdmission::PayloadsRemain(candidate) => {
                    let deadline = candidate.deadline();
                    candidate
                        .drain_one(AppDataResetOldDatabasePayloadDrainAuthority::for_test(
                            deadline,
                        ))
                        .unwrap();
                }
                AppDataResetOldDatabaseDrainingAdmission::Absent(_) => panic!("database missing"),
            },
        )
        .unwrap();
        let detached_root = partial.path().join(detached);
        std::fs::write(
            detached_root.join(suffixed_name(database.file_name().unwrap(), "-wal")),
            b"late sidecar",
        )
        .unwrap();
        assert!(
            SecureStorePaths::with_app_data_reset_draining_old_database_until(
                &database,
                old_database_open_binding(identity, detached, fresh_identity),
                Instant::now() + Duration::from_secs(1),
                |_| (),
            )
            .is_err()
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn old_database_draining_distinguishes_before_and_after_effect_uncertainty() {
        let before = TempDir::new().unwrap();
        let (database, identity, detached, fresh_identity) =
            prepare_old_database_draining_fixture(&before, &[]);
        set_test_app_data_reset_old_database_payload_drain_fault(
            AppDataResetOldDatabasePayloadDrainFault::BeforeEffect,
        );
        let result = SecureStorePaths::with_app_data_reset_draining_old_database_until(
            &database,
            old_database_open_binding(identity, detached, fresh_identity),
            Instant::now() + Duration::from_secs(1),
            |admission| match admission {
                AppDataResetOldDatabaseDrainingAdmission::SnapshotStorePresent => {
                    panic!("snapshot store unexpectedly present before fault")
                }
                AppDataResetOldDatabaseDrainingAdmission::PayloadsRemain(candidate) => {
                    let deadline = candidate.deadline();
                    candidate.drain_one(AppDataResetOldDatabasePayloadDrainAuthority::for_test(
                        deadline,
                    ))
                }
                AppDataResetOldDatabaseDrainingAdmission::Absent(_) => panic!("database missing"),
            },
        )
        .unwrap();
        assert_eq!(
            result.unwrap_err(),
            AppDataResetOldDatabasePayloadDrainError::BeforeEffect(
                DatabaseOpenErrorKind::DatabaseUnavailable
            )
        );
        assert!(before.path().join(detached).join("dux.sqlite3").is_file());

        let after = TempDir::new().unwrap();
        let (database, identity, detached, fresh_identity) =
            prepare_old_database_draining_fixture(&after, &[]);
        set_test_app_data_reset_old_database_payload_drain_fault(
            AppDataResetOldDatabasePayloadDrainFault::AfterEffect,
        );
        let result = SecureStorePaths::with_app_data_reset_draining_old_database_until(
            &database,
            old_database_open_binding(identity, detached, fresh_identity),
            Instant::now() + Duration::from_secs(1),
            |admission| match admission {
                AppDataResetOldDatabaseDrainingAdmission::SnapshotStorePresent => {
                    panic!("snapshot store unexpectedly present after fault")
                }
                AppDataResetOldDatabaseDrainingAdmission::PayloadsRemain(candidate) => {
                    let deadline = candidate.deadline();
                    candidate.drain_one(AppDataResetOldDatabasePayloadDrainAuthority::for_test(
                        deadline,
                    ))
                }
                AppDataResetOldDatabaseDrainingAdmission::Absent(_) => panic!("database missing"),
            },
        )
        .unwrap();
        assert_eq!(
            result.unwrap_err(),
            AppDataResetOldDatabasePayloadDrainError::OutcomeUnknown
        );
        assert!(!after.path().join(detached).join("dux.sqlite3").exists());
        SecureStorePaths::with_app_data_reset_draining_old_database_until(
            &database,
            old_database_open_binding(identity, detached, fresh_identity),
            Instant::now() + Duration::from_secs(1),
            |admission| {
                assert!(matches!(
                    admission,
                    AppDataResetOldDatabaseDrainingAdmission::Absent(_)
                ))
            },
        )
        .unwrap();
    }

    #[cfg(target_os = "macos")]
    fn add_everyone_acl(path: &Path, tag_type: i32, granted_permissions: &[i32]) {
        use std::ffi::{CString, c_char, c_int, c_void};
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;

        const ACL_TYPE_EXTENDED: c_int = 0x100;
        const EVERYONE_GROUP_UUID: [u8; 16] = [
            0xab, 0xcd, 0xef, 0xab, 0xcd, 0xef, 0xab, 0xcd, 0xef, 0xab, 0xcd, 0xef, 0, 0, 0, 0x0c,
        ];

        unsafe extern "C" {
            fn acl_init(entry_count: c_int) -> *mut c_void;
            fn acl_create_entry(acl: *mut *mut c_void, entry: *mut *mut c_void) -> c_int;
            fn acl_set_tag_type(entry: *mut c_void, tag_type: c_int) -> c_int;
            fn acl_set_qualifier(entry: *mut c_void, qualifier: *const c_void) -> c_int;
            fn acl_get_permset(entry: *mut c_void, permissions: *mut *mut c_void) -> c_int;
            fn acl_add_perm(permissions: *mut c_void, permission: c_int) -> c_int;
            fn acl_set_permset(entry: *mut c_void, permissions: *mut c_void) -> c_int;
            fn acl_get_flagset_np(entry: *mut c_void, flags: *mut *mut c_void) -> c_int;
            fn acl_clear_flags_np(flags: *mut c_void) -> c_int;
            fn acl_set_flagset_np(entry: *mut c_void, flags: *mut c_void) -> c_int;
            fn acl_valid(acl: *mut c_void) -> c_int;
            fn acl_set_file(path: *const c_char, acl_type: c_int, acl: *mut c_void) -> c_int;
            fn acl_get_fd_np(fd: c_int, acl_type: c_int) -> *mut c_void;
            fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
            fn acl_free(object: *mut c_void) -> c_int;
        }

        // SAFETY: Darwin allocates a one-entry ACL owned by this test.
        let mut acl = unsafe { acl_init(1) };
        assert!(!acl.is_null());
        let mut entry = std::ptr::null_mut();
        // SAFETY: all pointers refer to live ACL-owned storage, and the UUID
        // remains live for the duration of the qualifier copy.
        let configured = unsafe {
            acl_create_entry(&raw mut acl, &raw mut entry) == 0
                && acl_set_tag_type(entry, tag_type) == 0
                && acl_set_qualifier(entry, EVERYONE_GROUP_UUID.as_ptr().cast()) == 0
        };
        let mut permissions = std::ptr::null_mut();
        let mut flags = std::ptr::null_mut();
        // SAFETY: `entry` was initialized above and the permission set belongs
        // to that entry for the ACL's lifetime.
        let mut permissions_configured =
            configured && unsafe { acl_get_permset(entry, &raw mut permissions) } == 0;
        for permission in granted_permissions {
            // SAFETY: `permissions` belongs to the live entry permission set.
            permissions_configured &= unsafe { acl_add_perm(permissions, *permission) } == 0;
        }
        // SAFETY: every pointer belongs to the live ACL entry.
        permissions_configured &= unsafe {
            acl_set_permset(entry, permissions) == 0
                && acl_get_flagset_np(entry, &raw mut flags) == 0
                && acl_clear_flags_np(flags) == 0
                && acl_set_flagset_np(entry, flags) == 0
        };
        assert!(permissions_configured);
        // SAFETY: `acl` is fully configured and live.
        assert_eq!(unsafe { acl_valid(acl) }, 0);
        let file = File::open(path).unwrap();
        let path_text = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: the path is NUL-terminated and `acl` is a live Darwin ACL.
        let set_result = unsafe { acl_set_file(path_text.as_ptr(), ACL_TYPE_EXTENDED, acl) };
        // SAFETY: `acl` came from acl_from_text and is freed exactly once.
        let free_result = unsafe { acl_free(acl) };
        assert_eq!(set_result, 0);
        assert_eq!(free_result, 0);
        // SAFETY: the live descriptor returns an independently owned ACL.
        let applied = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
        assert!(!applied.is_null());
        let mut applied_entry = std::ptr::null_mut();
        // SAFETY: `applied` is live and the output pointer is writable.
        let entry_result = unsafe { acl_get_entry(applied, 0, &raw mut applied_entry) };
        // SAFETY: `applied` is freed exactly once after inspection.
        assert_eq!(unsafe { acl_free(applied) }, 0);
        assert_eq!(entry_result, 0);
        assert!(!applied_entry.is_null());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn provisions_beneath_a_restrictive_system_style_parent_acl() {
        const ACL_EXTENDED_DENY: i32 = 2;
        const ACL_DELETE: i32 = 1 << 4;

        let temp = TempDir::new().unwrap();
        add_everyone_acl(temp.path(), ACL_EXTENDED_DENY, &[ACL_DELETE]);
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();

        storage.validate_for_database_open().unwrap();
        assert!(database.is_file());
    }

    #[test]
    fn provisions_and_revalidates_owned_storage() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();

        assert_eq!(storage.database_path(), database);
        assert!(storage.database_path().is_file());
        assert!(storage.lock_path().is_file());
        assert_eq!(
            fs::read(storage.lock_path()).unwrap(),
            DUX_ROOT_MARKER_LAYOUT_V2
        );
        assert!(storage.cleanup_lock_path().is_file());
        assert_eq!(
            fs::read(storage.cleanup_lock_path()).unwrap(),
            DUX_CLEANUP_LOCK_MARKER
        );
        assert_eq!(
            fs::read(
                database.with_file_name(cleanup_lock_ready_name(database.file_name().unwrap()))
            )
            .unwrap(),
            DUX_CLEANUP_LOCK_READY_MARKER
        );
        storage.validate_for_database_open().unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(database.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                0o700
            );
            assert_eq!(
                fs::metadata(storage.database_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                0o600
            );
            assert_eq!(
                fs::metadata(storage.lock_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                0o600
            );
            assert_eq!(
                fs::metadata(storage.cleanup_lock_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o7777,
                0o600
            );
        }
    }

    #[test]
    fn provisions_cleanup_lock_for_an_existing_owned_layout() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let legacy_probe = StoreProbe::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let cleanup = database.with_file_name(cleanup_lock_name(database.file_name().unwrap()));
        let ready = database.with_file_name(cleanup_lock_ready_name(database.file_name().unwrap()));
        assert!(!cleanup.exists());
        assert!(!ready.exists());
        drop(legacy_probe);

        let storage = SecureStorePaths::prepare(&database).unwrap();
        assert_eq!(fs::read(&cleanup).unwrap(), DUX_CLEANUP_LOCK_MARKER);
        assert_eq!(fs::read(&ready).unwrap(), DUX_CLEANUP_LOCK_READY_MARKER);
        storage.validate_for_database_open().unwrap();
    }

    #[test]
    fn legacy_cleanup_layout_upgrade_obeys_the_writer_lock() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let probe = StoreProbe::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let writer_name = lock_name(database.file_name().unwrap());
        let (writer, _) = platform::open_existing_file(
            &probe.root_directory,
            &probe.root_path,
            &writer_name,
            PermissionPolicy::RequirePrivate,
        )
        .unwrap()
        .unwrap();
        let writer_guard = acquire_advisory_lock(&writer, Duration::from_millis(20)).unwrap();
        let cleanup = probe
            .root_path
            .join(cleanup_lock_name(database.file_name().unwrap()));
        let ready = probe
            .root_path
            .join(cleanup_lock_ready_name(database.file_name().unwrap()));

        let error = probe
            .secure_with_control_lock_timeout(Duration::from_millis(10))
            .err()
            .unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::Busy);
        assert!(!cleanup.exists());
        assert!(!ready.exists());

        FileExt::unlock(&writer_guard).unwrap();
        drop(writer_guard);
        drop(writer);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        assert_eq!(fs::read(cleanup).unwrap(), DUX_CLEANUP_LOCK_MARKER);
        assert_eq!(fs::read(ready).unwrap(), DUX_CLEANUP_LOCK_READY_MARKER);
        drop(storage);
    }

    #[test]
    fn ready_layout_never_recreates_a_missing_cleanup_lock() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let probe = StoreProbe::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let ready_name = cleanup_lock_ready_name(database.file_name().unwrap());
        let (ready, ready_identity) = platform::create_private_file_exclusive(
            &probe.root_directory,
            &probe.root_path,
            &ready_name,
        )
        .unwrap();
        ensure_exact_marker(&ready, DUX_CLEANUP_LOCK_READY_MARKER).unwrap();
        platform::validate_path_identity(
            &probe.root_path.join(&ready_name),
            ObjectKind::RegularFile,
            ready_identity,
            PermissionPolicy::RequirePrivate,
        )
        .unwrap();
        platform::sync_directory(&probe.root_directory).unwrap();
        drop(ready);
        drop(probe);

        let cleanup = database.with_file_name(cleanup_lock_name(database.file_name().unwrap()));
        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
        assert!(!cleanup.exists());
    }

    #[test]
    fn current_layout_never_recreates_both_missing_cleanup_controls() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let probe = StoreProbe::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let writer_name = lock_name(database.file_name().unwrap());
        let (writer, _) = platform::open_existing_writer_file(
            &probe.root_directory,
            &probe.root_path,
            &writer_name,
            PermissionPolicy::RequirePrivate,
        )
        .unwrap()
        .unwrap();
        ensure_exact_marker(&writer, DUX_ROOT_MARKER_LAYOUT_V2).unwrap();
        drop(writer);
        drop(probe);

        let cleanup = database.with_file_name(cleanup_lock_name(database.file_name().unwrap()));
        let ready = database.with_file_name(cleanup_lock_ready_name(database.file_name().unwrap()));
        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
        assert!(!cleanup.exists());
        assert!(!ready.exists());
        assert_eq!(
            fs::read(database.with_file_name(writer_name)).unwrap(),
            DUX_ROOT_MARKER_LAYOUT_V2
        );
    }

    #[test]
    fn database_reopen_tolerates_only_reserved_application_support_siblings() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let initial = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        drop(initial);

        for name in RESERVED_APP_SUPPORT_ENTRIES {
            fs::create_dir(database.parent().unwrap().join(name)).unwrap();
        }
        let reopened = SecureStorePaths::prepare(&database).unwrap();
        reopened.validate_for_database_open().unwrap();
        drop(reopened);

        let foreign = database.parent().unwrap().join("foreign-data");
        fs::write(&foreign, b"untouched").unwrap();
        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnrecognizedDatabase);
        assert_eq!(fs::read(foreign).unwrap(), b"untouched");
    }

    #[cfg(unix)]
    #[test]
    fn store_coordinator_reopens_with_exact_root_local_snapshot_stages() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        open_and_close_store(&database);
        let root = database.parent().unwrap();

        create_private_snapshot_stage(root, &snapshot_stage_name(1));
        let marker_complete = create_private_snapshot_stage(root, &snapshot_stage_name(2));
        let interrupted_before_mode_repair =
            create_private_snapshot_stage(root, &snapshot_stage_name(3));
        fs::set_permissions(
            &interrupted_before_mode_repair,
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        for (name, bytes) in [
            (".dux-snapshot-store", b"DUXSNAPSTOREV1\0\0".as_slice()),
            (
                ".dux-snapshot.writer.lock",
                b"DUXSNAPWRITER1\0\0".as_slice(),
            ),
        ] {
            let control = marker_complete.join(name);
            fs::write(&control, bytes).unwrap();
            fs::set_permissions(control, fs::Permissions::from_mode(0o600)).unwrap();
        }

        open_and_close_store(&database);
        assert!(root.join(snapshot_stage_name(1)).is_dir());
        assert_eq!(
            fs::read(marker_complete.join(".dux-snapshot-store")).unwrap(),
            b"DUXSNAPSTOREV1\0\0"
        );
        assert_eq!(
            fs::metadata(interrupted_before_mode_repair)
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn root_local_snapshot_stage_inventory_rejects_malformed_and_unsafe_shapes() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        enum UnsafeShape {
            MalformedName,
            RegularFile,
            SymbolicLink,
            BroadDirectory,
        }

        for shape in [
            UnsafeShape::MalformedName,
            UnsafeShape::RegularFile,
            UnsafeShape::SymbolicLink,
            UnsafeShape::BroadDirectory,
        ] {
            let temp = TempDir::new().unwrap();
            let database = database_path(&temp);
            open_and_close_store(&database);
            let root = database.parent().unwrap();
            let canonical = snapshot_stage_name(1);
            let entry = match shape {
                UnsafeShape::MalformedName => {
                    let malformed =
                        format!("{SNAPSHOT_STAGE_PREFIX}AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
                    create_private_snapshot_stage(root, &malformed)
                }
                UnsafeShape::RegularFile => {
                    let entry = root.join(&canonical);
                    fs::write(&entry, b"not a stage directory").unwrap();
                    entry
                }
                UnsafeShape::SymbolicLink => {
                    let target = temp.path().join("link-target");
                    fs::create_dir(&target).unwrap();
                    let entry = root.join(&canonical);
                    symlink(target, &entry).unwrap();
                    entry
                }
                UnsafeShape::BroadDirectory => {
                    let entry = create_private_snapshot_stage(root, &canonical);
                    fs::set_permissions(&entry, fs::Permissions::from_mode(0o755)).unwrap();
                    entry
                }
            };

            assert!(super::super::store::StoreCoordinator::open(&database).is_err());
            assert!(entry.exists() || entry.symlink_metadata().is_ok());
        }
    }

    #[cfg(unix)]
    #[test]
    fn root_local_snapshot_stage_inventory_caps_at_sixty_four_complete_entries() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        open_and_close_store(&database);
        let root = database.parent().unwrap();
        for sequence in 0..MAX_SNAPSHOT_STAGES {
            create_private_snapshot_stage(root, &snapshot_stage_name(sequence));
        }

        open_and_close_store(&database);
        let sixty_fifth =
            create_private_snapshot_stage(root, &snapshot_stage_name(MAX_SNAPSHOT_STAGES));
        let error = super::super::store::StoreCoordinator::open(&database)
            .err()
            .unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnrecognizedDatabase);
        assert!(sixty_fifth.is_dir());
    }

    #[test]
    fn malformed_ownership_marker_is_rejected_without_claiming_the_store() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let initial = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let marker = initial.lock_path().to_path_buf();
        drop(initial);

        let malformed = b"NOT-A-DUX-MARKER";
        assert_eq!(malformed.len(), DUX_ROOT_MARKER.len());
        fs::write(&marker, malformed).unwrap();

        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
        assert_eq!(fs::read(marker).unwrap(), malformed);
    }

    #[test]
    fn malformed_cleanup_control_markers_are_rejected_without_replacement() {
        const MALFORMED: &[u8; 16] = b"NOT-CLEANUP-LOCK";

        for malformed_ready in [false, true] {
            let temp = TempDir::new().unwrap();
            let database = database_path(&temp);
            let probe = StoreProbe::prepare(&database).unwrap();
            initialize_dux_header(&database);
            let cleanup_name = cleanup_lock_name(database.file_name().unwrap());
            let cleanup_path = probe.root_path.join(&cleanup_name);
            let (cleanup, cleanup_identity) = platform::create_private_file_exclusive(
                &probe.root_directory,
                &probe.root_path,
                &cleanup_name,
            )
            .unwrap();
            ensure_exact_marker(
                &cleanup,
                if malformed_ready {
                    DUX_CLEANUP_LOCK_MARKER
                } else {
                    MALFORMED
                },
            )
            .unwrap();
            platform::validate_path_identity(
                &cleanup_path,
                ObjectKind::RegularFile,
                cleanup_identity,
                PermissionPolicy::RequirePrivate,
            )
            .unwrap();

            let malformed_path = if malformed_ready {
                let ready_name = cleanup_lock_ready_name(database.file_name().unwrap());
                let ready_path = probe.root_path.join(&ready_name);
                let (ready, ready_identity) = platform::create_private_file_exclusive(
                    &probe.root_directory,
                    &probe.root_path,
                    &ready_name,
                )
                .unwrap();
                ensure_exact_marker(&ready, MALFORMED).unwrap();
                platform::validate_path_identity(
                    &ready_path,
                    ObjectKind::RegularFile,
                    ready_identity,
                    PermissionPolicy::RequirePrivate,
                )
                .unwrap();
                ready_path
            } else {
                cleanup_path
            };
            platform::sync_directory(&probe.root_directory).unwrap();
            drop(cleanup);
            drop(probe);

            let error = SecureStorePaths::prepare(&database).err().unwrap();
            assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
            assert_eq!(fs::read(malformed_path).unwrap(), MALFORMED);
        }
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_lock_hard_link_is_rejected() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let probe = StoreProbe::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let outside = temp.path().join("outside-cleanup-lock");
        fs::write(&outside, DUX_CLEANUP_LOCK_MARKER).unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).unwrap();
        let cleanup = probe
            .root_path
            .join(cleanup_lock_name(database.file_name().unwrap()));
        fs::hard_link(&outside, &cleanup).unwrap();
        drop(probe);

        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
        assert_eq!(fs::read(outside).unwrap(), DUX_CLEANUP_LOCK_MARKER);
    }

    #[cfg(unix)]
    #[test]
    fn preexisting_cleanup_control_permissions_are_not_repaired() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let probe = StoreProbe::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let cleanup = probe
            .root_path
            .join(cleanup_lock_name(database.file_name().unwrap()));
        fs::write(&cleanup, DUX_CLEANUP_LOCK_MARKER).unwrap();
        fs::set_permissions(&cleanup, fs::Permissions::from_mode(0o644)).unwrap();
        drop(probe);

        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafePermissions);
        assert_eq!(
            fs::metadata(cleanup).unwrap().permissions().mode() & 0o7777,
            0o644
        );
    }

    #[cfg(unix)]
    #[test]
    fn initialization_sentinel_symlink_is_rejected_without_following_it() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let initial = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        drop(initial);
        let outside = temp.path().join("outside-initialization-target");
        fs::write(&outside, DUX_INITIALIZATION_SENTINEL).unwrap();
        let sentinel = database.with_file_name(initialization_name(database.file_name().unwrap()));
        symlink(&outside, &sentinel).unwrap();

        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
        assert_eq!(fs::read(outside).unwrap(), DUX_INITIALIZATION_SENTINEL);
    }

    #[cfg(unix)]
    #[test]
    fn initialization_sentinel_requires_exact_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let initial = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        drop(initial);
        let sentinel = database.with_file_name(initialization_name(database.file_name().unwrap()));
        fs::write(&sentinel, DUX_INITIALIZATION_SENTINEL).unwrap();
        fs::set_permissions(&sentinel, fs::Permissions::from_mode(0o644)).unwrap();

        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafePermissions);
        assert_eq!(
            fs::metadata(&sentinel).unwrap().permissions().mode() & 0o7777,
            0o644
        );
    }

    #[cfg(unix)]
    #[test]
    fn staged_publication_never_overwrites_a_racing_root() {
        let temp = TempDir::new().unwrap();
        let final_root = temp.path().join("owned");
        let database_name = OsStr::new("dux.sqlite3");
        let marker_name = lock_name(database_name);
        let prepared = platform::prepare_root_for_probe(&final_root).unwrap();
        assert_eq!(prepared.state, PreparedRootState::FreshStaged);
        assert!(!final_root.exists());

        let (marker, marker_identity) = platform::create_private_file_exclusive(
            &prepared.directory,
            &prepared.object_path,
            &marker_name,
        )
        .unwrap();
        ensure_root_marker(&marker).unwrap();
        let (database, database_identity) = platform::create_private_file_exclusive(
            &prepared.directory,
            &prepared.object_path,
            database_name,
        )
        .unwrap();
        database.sync_all().unwrap();

        fs::create_dir(&final_root).unwrap();
        let sentinel = final_root.join("foreign-sentinel");
        fs::write(&sentinel, b"untouched").unwrap();
        assert_eq!(
            platform::publish_prepared_root(
                &prepared,
                &final_root,
                database_name,
                &database,
                database_identity,
                &marker_name,
                &marker,
                marker_identity,
            )
            .unwrap(),
            RootPublicationResult::Collision
        );
        assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");
        assert!(prepared.object_path.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn special_file_probe_is_nonblocking_and_rejects_fifo() {
        use std::sync::mpsc;

        use nix::fcntl::{OFlag, open};
        use nix::sys::stat::Mode;
        use nix::unistd::mkfifo;

        for name in [
            "dux.sqlite3",
            "dux.sqlite3.writer.lock",
            "dux.sqlite3.cleanup.lock",
            "dux.sqlite3.cleanup.lock.ready",
            "dux.sqlite3-journal",
        ] {
            let temp = TempDir::new().unwrap();
            let root_path = temp.path().join("root");
            fs::create_dir(&root_path).unwrap();
            let fifo = root_path.join(name);
            mkfifo(&fifo, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
            let (sent, received) = mpsc::channel();
            let worker_root = root_path.clone();
            let worker_name = OsString::from(name);
            let worker = std::thread::spawn(move || {
                let root = File::open(&worker_root).unwrap();
                let result = platform::open_existing_file(
                    &root,
                    &worker_root,
                    &worker_name,
                    PermissionPolicy::InspectOnly,
                );
                sent.send(result.map(drop)).unwrap();
            });

            let timeout_error = match received.recv_timeout(Duration::from_millis(250)) {
                Ok(result) => {
                    assert_eq!(
                        result.unwrap_err().kind,
                        DatabaseOpenErrorKind::UnsafeStorageObject
                    );
                    None
                }
                Err(error) => {
                    let _writer =
                        open(&fifo, OFlag::O_WRONLY | OFlag::O_NONBLOCK, Mode::empty()).unwrap();
                    let _ = received.recv_timeout(Duration::from_secs(1));
                    Some(error)
                }
            };
            worker.join().unwrap();
            if let Some(error) = timeout_error {
                panic!("special-file probe blocked for {name}: {error}");
            }
        }
    }

    #[test]
    fn writer_lock_is_bounded_and_released() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let first = SecureStorePaths::prepare(&database).unwrap();
        let second = SecureStorePaths::prepare(&database).unwrap();
        let guard = first
            .acquire_writer_lock(Duration::from_millis(20))
            .unwrap();

        let error = second
            .acquire_writer_lock(Duration::from_millis(10))
            .err()
            .unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::Busy);
        assert_eq!(
            first
                .acquire_writer_lock(Duration::from_millis(1))
                .err()
                .unwrap()
                .kind,
            DatabaseOpenErrorKind::Busy
        );

        drop(guard);
        second
            .acquire_writer_lock(Duration::from_millis(20))
            .unwrap();
    }

    #[test]
    fn cleanup_lock_is_bounded_revalidated_and_independent() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let first = SecureStorePaths::prepare(&database).unwrap();
        let second = SecureStorePaths::prepare(&database).unwrap();
        let guard = first
            .acquire_cleanup_lock(Duration::from_millis(20))
            .unwrap();
        first.validate_cleanup_lock_guard(&guard).unwrap();

        assert_eq!(
            second
                .acquire_cleanup_lock(Duration::from_millis(10))
                .err()
                .unwrap()
                .kind,
            DatabaseOpenErrorKind::Busy
        );
        assert_eq!(
            first
                .acquire_cleanup_lock(Duration::from_millis(1))
                .err()
                .unwrap()
                .kind,
            DatabaseOpenErrorKind::Busy
        );

        // Cleanup exclusion is intentionally distinct from SQLite writer
        // serialization, so future journal writes can occur while it is held.
        first
            .acquire_writer_lock(Duration::from_millis(20))
            .unwrap();
        drop(guard);
        second
            .acquire_cleanup_lock(Duration::from_millis(20))
            .unwrap();
    }

    #[test]
    fn cleanup_lock_rejects_root_inventory_changes_before_and_while_held() {
        for change_while_held in [false, true] {
            let temp = TempDir::new().unwrap();
            let database = database_path(&temp);
            let storage = SecureStorePaths::prepare(&database).unwrap();
            let guard = change_while_held.then(|| {
                storage
                    .acquire_cleanup_lock(Duration::from_millis(20))
                    .unwrap()
            });
            fs::write(
                database.parent().unwrap().join("foreign-after-prepare"),
                b"untouched",
            )
            .unwrap();

            let error = if let Some(guard) = guard.as_ref() {
                storage.validate_cleanup_lock_guard(guard).err().unwrap()
            } else {
                storage
                    .acquire_cleanup_lock(Duration::from_millis(20))
                    .err()
                    .unwrap()
            };
            assert_eq!(error.kind, DatabaseOpenErrorKind::UnrecognizedDatabase);
        }
    }

    #[test]
    fn cleanup_lock_rejects_writer_marker_content_changes() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        let malformed = b"NOT-A-DUX-MARKER";
        assert_eq!(malformed.len(), DUX_ROOT_MARKER.len());
        fs::write(storage.lock_path(), malformed).unwrap();

        let error = storage
            .acquire_cleanup_lock(Duration::from_millis(20))
            .err()
            .unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
        assert_eq!(fs::read(storage.lock_path()).unwrap(), malformed);
    }

    #[cfg(unix)]
    #[test]
    fn repairs_owned_permissions_and_rejects_hard_links() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let initial = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        drop(initial);
        fs::set_permissions(
            database.parent().unwrap(),
            fs::Permissions::from_mode(0o777),
        )
        .unwrap();
        fs::set_permissions(&database, fs::Permissions::from_mode(0o666)).unwrap();

        let storage = SecureStorePaths::prepare(&database).unwrap();
        assert_eq!(
            fs::metadata(database.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o700
        );
        assert_eq!(
            fs::metadata(&database).unwrap().permissions().mode() & 0o7777,
            0o600
        );
        drop(storage);

        fs::hard_link(&database, temp.path().join("database-alias")).unwrap();
        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
    }

    #[cfg(unix)]
    #[test]
    fn existing_database_and_marker_must_both_be_present_and_untouched() {
        use std::os::unix::fs::PermissionsExt;

        let missing_marker = TempDir::new().unwrap();
        let database = database_path(&missing_marker);
        fs::create_dir(database.parent().unwrap()).unwrap();
        initialize_dux_header(&database);
        fs::set_permissions(
            database.parent().unwrap(),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        fs::set_permissions(&database, fs::Permissions::from_mode(0o644)).unwrap();

        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
        assert_eq!(
            fs::metadata(database.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );
        assert_eq!(
            fs::metadata(&database).unwrap().permissions().mode() & 0o7777,
            0o644
        );
        assert!(
            !database
                .parent()
                .unwrap()
                .join(lock_name(database.file_name().unwrap()))
                .exists()
        );

        let missing_database = TempDir::new().unwrap();
        let database = database_path(&missing_database);
        fs::create_dir(database.parent().unwrap()).unwrap();
        fs::set_permissions(
            database.parent().unwrap(),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let marker = database
            .parent()
            .unwrap()
            .join(lock_name(database.file_name().unwrap()));
        fs::write(&marker, DUX_ROOT_MARKER).unwrap();
        fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();

        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
        assert!(!database.exists());
        assert_eq!(fs::read(marker).unwrap(), DUX_ROOT_MARKER);
    }

    #[cfg(windows)]
    #[test]
    fn repairs_owned_dacls_and_rejects_hard_links() {
        for shape in [
            platform::TestAclShape::Broad,
            platform::TestAclShape::Inherited,
            platform::TestAclShape::Null,
        ] {
            let temp = TempDir::new().unwrap();
            let database = database_path(&temp);
            let initial = SecureStorePaths::prepare(&database).unwrap();
            initialize_dux_header(&database);
            drop(initial);

            let storage = SecureStorePaths::prepare(&database).unwrap();
            platform::replace_acl_for_test(&database, ObjectKind::RegularFile, shape).unwrap();
            let error = storage.validate_for_database_open().err().unwrap();
            assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafePermissions);
            drop(storage);

            let repaired = SecureStorePaths::prepare(&database).unwrap();
            platform::assert_private_for_test(&database, ObjectKind::RegularFile).unwrap();
            repaired.validate_for_database_open().unwrap();
        }

        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let initial = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        drop(initial);
        fs::hard_link(&database, temp.path().join("database-alias")).unwrap();

        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
    }

    #[cfg(windows)]
    #[test]
    #[allow(clippy::disallowed_methods)]
    fn retained_cleanup_lock_blocks_path_replacement() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        let ready = database.with_file_name(cleanup_lock_ready_name(database.file_name().unwrap()));
        for (index, control) in [storage.lock_path(), storage.cleanup_lock_path(), &ready]
            .into_iter()
            .enumerate()
        {
            let displaced = temp.path().join(format!("displaced-control-{index}"));
            // DUX-DESTRUCTIVE: allow=test-storage-cleanup-lock-rename-guard -- attempt only to rename each TempDir-owned retained control and prove the Windows handles deny replacement
            assert!(fs::rename(control, displaced).is_err());
        }
        storage.validate_for_database_open().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn preexisting_cleanup_control_dacl_is_not_repaired() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let probe = StoreProbe::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let cleanup_name = cleanup_lock_name(database.file_name().unwrap());
        let cleanup_path = probe.root_path.join(&cleanup_name);
        let (cleanup, _) = platform::create_private_file_exclusive(
            &probe.root_directory,
            &probe.root_path,
            &cleanup_name,
        )
        .unwrap();
        ensure_exact_marker(&cleanup, DUX_CLEANUP_LOCK_MARKER).unwrap();
        platform::sync_directory(&probe.root_directory).unwrap();
        drop(cleanup);
        drop(probe);
        platform::replace_acl_for_test(
            &cleanup_path,
            ObjectKind::RegularFile,
            platform::TestAclShape::Broad,
        )
        .unwrap();

        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafePermissions);
        assert_eq!(
            platform::assert_private_for_test(&cleanup_path, ObjectKind::RegularFile)
                .err()
                .unwrap()
                .kind,
            DatabaseOpenErrorKind::UnsafePermissions
        );
    }

    #[cfg(windows)]
    #[test]
    fn inherited_sqlite_sidecar_is_repaired_only_by_marker_authorized_hook() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        let journal =
            database.with_file_name(suffixed_name(database.file_name().unwrap(), "-journal"));
        fs::write(&journal, b"sqlite-owned test sidecar").unwrap();

        assert_eq!(
            platform::assert_private_for_test(&journal, ObjectKind::RegularFile)
                .err()
                .unwrap()
                .kind,
            DatabaseOpenErrorKind::UnsafePermissions
        );
        assert_eq!(
            storage.validate_for_database_open().err().unwrap().kind,
            DatabaseOpenErrorKind::UnsafePermissions
        );

        storage.repair_sqlite_sidecars().unwrap();
        platform::assert_private_for_test(&journal, ObjectKind::RegularFile).unwrap();
        storage.validate_for_database_open().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn sqlite_created_sidecars_have_exact_private_dacls() {
        use super::super::store::StoreCoordinator;

        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let _store = StoreCoordinator::open(&database).unwrap();
        let sentinel = database.with_file_name(initialization_name(database.file_name().unwrap()));
        platform::assert_private_for_test(&sentinel, ObjectKind::RegularFile).unwrap();
        let mut existing = 0;
        for suffix in SIDECAR_SUFFIXES {
            let sidecar =
                database.with_file_name(suffixed_name(database.file_name().unwrap(), suffix));
            if sidecar.exists() {
                existing += 1;
                platform::assert_private_for_test(&sidecar, ObjectKind::RegularFile).unwrap();
            }
        }
        assert!(existing > 0, "WAL configuration must create a sidecar");
    }

    #[cfg(unix)]
    #[test]
    fn empty_database_in_an_unproven_nonempty_root_is_not_claimed() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        fs::create_dir(database.parent().unwrap()).unwrap();
        fs::set_permissions(
            database.parent().unwrap(),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::write(&database, []).unwrap();
        fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(
            database.parent().unwrap().join("foreign-data"),
            b"untouched",
        )
        .unwrap();

        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnrecognizedDatabase);
        assert!(
            !database
                .parent()
                .unwrap()
                .join(lock_name(database.file_name().unwrap()))
                .exists()
        );
        assert_eq!(
            fs::read(database.parent().unwrap().join("foreign-data")).unwrap(),
            b"untouched"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rejects_extended_acl_without_repairing_it() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let initial = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        drop(initial);
        const ACL_EXTENDED_ALLOW: i32 = 1;
        const ACL_READ_DATA: i32 = 1 << 1;
        const ACL_EXECUTE: i32 = 1 << 3;
        add_everyone_acl(
            database.parent().unwrap(),
            ACL_EXTENDED_ALLOW,
            &[ACL_READ_DATA, ACL_EXECUTE],
        );

        let error = SecureStorePaths::prepare(&database).err().unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafePermissions);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_root_database_lock_and_sidecar_symlinks() {
        use std::os::unix::fs::symlink;

        let root_case = TempDir::new().unwrap();
        let external_root = root_case.path().join("external-root");
        fs::create_dir(&external_root).unwrap();
        symlink(&external_root, root_case.path().join("owned")).unwrap();
        let error = SecureStorePaths::prepare(&database_path(&root_case))
            .err()
            .unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageRoot);

        let database_case = TempDir::new().unwrap();
        fs::create_dir(database_case.path().join("owned")).unwrap();
        let external_file = database_case.path().join("external-file");
        fs::write(&external_file, b"untouched").unwrap();
        symlink(&external_file, database_path(&database_case)).unwrap();
        let error = SecureStorePaths::prepare(&database_path(&database_case))
            .err()
            .unwrap();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
        assert_eq!(fs::read(&external_file).unwrap(), b"untouched");

        for suffix in [
            WRITER_LOCK_SUFFIX,
            CLEANUP_LOCK_SUFFIX,
            CLEANUP_LOCK_READY_SUFFIX,
            "-wal",
            "-shm",
            "-journal",
        ] {
            let temp = TempDir::new().unwrap();
            let database = database_path(&temp);
            if suffix == WRITER_LOCK_SUFFIX {
                fs::create_dir(database.parent().unwrap()).unwrap();
                initialize_dux_header(&database);
            } else if matches!(suffix, CLEANUP_LOCK_SUFFIX | CLEANUP_LOCK_READY_SUFFIX) {
                let legacy_probe = StoreProbe::prepare(&database).unwrap();
                initialize_dux_header(&database);
                drop(legacy_probe);
            } else {
                let initial = SecureStorePaths::prepare(&database).unwrap();
                initialize_dux_header(&database);
                drop(initial);
            }
            let name = suffixed_name(database.file_name().unwrap(), suffix);
            let external = temp.path().join(format!("external-{suffix:?}"));
            fs::write(&external, b"untouched").unwrap();
            symlink(&external, database.parent().unwrap().join(name)).unwrap();

            let error = SecureStorePaths::prepare(&database).err().unwrap();
            assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
            assert_eq!(fs::read(external).unwrap(), b"untouched");
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn data_namespace_fence_excludes_same_process_root_publication() {
        use std::sync::mpsc::{self, TryRecvError};

        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let sibling_database = temp.path().join("other-owned").join("dux.sqlite3");
        let (sent, received) = mpsc::channel();

        std::thread::scope(|scope| {
            storage
                .with_app_data_reset_namespace_fence_until(
                    OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff"),
                    Instant::now() + Duration::from_secs(2),
                    |admission| {
                        admission.revalidate().unwrap();
                        scope.spawn(|| {
                            let result = StoreProbe::prepare(&sibling_database).map(drop);
                            sent.send(result).unwrap();
                        });
                        std::thread::sleep(Duration::from_millis(40));
                        assert!(matches!(received.try_recv(), Err(TryRecvError::Empty)));
                    },
                )
                .unwrap();
        });

        received
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test relaunches only its exact unit-test helper against a TempDir-owned sibling root"
    )]
    fn data_namespace_fence_excludes_subprocess_root_publication() {
        const ROLE: &str = "DUX_DATA_NAMESPACE_PUBLICATION_CHILD";
        const DATABASE: &str = "DUX_DATA_NAMESPACE_PUBLICATION_DATABASE";
        const READY: &str = "DUX_DATA_NAMESPACE_PUBLICATION_READY";

        if std::env::var_os(ROLE).is_some() {
            let database = PathBuf::from(std::env::var_os(DATABASE).unwrap());
            let ready = PathBuf::from(std::env::var_os(READY).unwrap());
            fs::write(ready, b"ready").unwrap();
            StoreProbe::prepare(&database).unwrap();
            return;
        }

        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let sibling_database = temp.path().join("child-owned").join("dux.sqlite3");
        let ready = temp.path().join("child-ready");
        let mut child = None;

        storage
            .with_app_data_reset_namespace_fence_until(
                OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff"),
                Instant::now() + Duration::from_secs(3),
                |admission| {
                    admission.revalidate().unwrap();
                    child = Some(
                        // DUX-DESTRUCTIVE: allow=test-data-namespace-publication-helper-spawn -- relaunch only this exact unit test against its TempDir-owned sibling root to prove the retained parent fence across a real process boundary
                        std::process::Command::new(std::env::current_exe().unwrap())
                            .arg("--exact")
                            .arg(
                                "persistence::storage::tests::data_namespace_fence_excludes_subprocess_root_publication",
                            )
                            .arg("--nocapture")
                            .env(ROLE, "1")
                            .env(DATABASE, &sibling_database)
                            .env(READY, &ready)
                            .spawn()
                            .unwrap(),
                    );
                    let ready_deadline = Instant::now() + Duration::from_secs(2);
                    while !ready.exists() {
                        assert!(
                            Instant::now() < ready_deadline,
                            "subprocess never reached its publication attempt"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    assert!(child.as_mut().unwrap().try_wait().unwrap().is_none());
                },
            )
            .unwrap();

        assert!(child.unwrap().wait().unwrap().success());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn data_namespace_fence_rejects_collision_and_strict_expiry() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let detached_name = OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff");
        fs::create_dir(temp.path().join(detached_name)).unwrap();

        let collision = storage
            .with_app_data_reset_namespace_fence_until(
                detached_name,
                Instant::now() + Duration::from_secs(1),
                |_| (),
            )
            .unwrap_err();
        assert_eq!(collision.kind, DatabaseOpenErrorKind::UnsafeStorageRoot);

        let expired = storage
            .with_app_data_reset_namespace_fence_until(
                OsStr::new(".dux-reset-data-ffeeddccbbaa99887766554433221100"),
                Instant::now(),
                |_| panic!("an expired admission must not invoke its callback"),
            )
            .unwrap_err();
        assert_eq!(expired.kind, DatabaseOpenErrorKind::Busy);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn data_namespace_fence_rejects_unowned_reserved_children() {
        for child_name in ["ai", "logs"] {
            let temp = TempDir::new().unwrap();
            let database = database_path(&temp);
            let storage = SecureStorePaths::prepare(&database).unwrap();
            fs::create_dir(database.parent().unwrap().join(child_name)).unwrap();

            let error = storage
                .with_app_data_reset_namespace_fence_until(
                    OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff"),
                    Instant::now() + Duration::from_secs(1),
                    |_| (),
                )
                .unwrap_err();
            assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn data_namespace_validation_rejects_mount_bound_root_identity() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        let mount_bound_identity = PlatformIdentity {
            device: storage.root_identity.device.wrapping_add(1),
            inode: storage.root_identity.inode,
        };

        let error = platform::validate_data_reset_namespace(
            &storage.publication_parent,
            storage.publication_parent_identity,
            &storage.root_directory,
            mount_bound_identity,
            &storage.root_path,
            storage.root_path.file_name().unwrap(),
            OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff"),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap_err();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageRoot);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn forgotten_or_panicking_data_witness_cannot_retain_parent_fence() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let detached_name = OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff");

        storage
            .with_app_data_reset_namespace_fence_until(
                detached_name,
                Instant::now() + Duration::from_secs(1),
                |admission| {
                    let _forgotten = std::mem::ManuallyDrop::new(admission);
                },
            )
            .unwrap();
        storage
            .with_app_data_reset_namespace_fence_until(
                detached_name,
                Instant::now() + Duration::from_secs(1),
                |admission| admission.revalidate().unwrap(),
            )
            .unwrap();

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = storage.with_app_data_reset_namespace_fence_until(
                detached_name,
                Instant::now() + Duration::from_secs(1),
                |_| panic!("exercise fence unwinding"),
            );
        }));
        assert!(panic.is_err());
        storage
            .with_app_data_reset_namespace_fence_until(
                detached_name,
                Instant::now() + Duration::from_secs(1),
                |admission| admission.revalidate().unwrap(),
            )
            .unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn data_namespace_revalidation_rejects_published_root_replacement() {
        let temp = TempDir::new().unwrap();
        let database = database_path(&temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        let root = database.parent().unwrap();
        let displaced = temp.path().join("displaced-owned");

        storage
            .with_app_data_reset_namespace_fence_until(
                OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff"),
                Instant::now() + Duration::from_secs(1),
                |admission| {
                    platform::rename_no_replace(
                        &storage.publication_parent,
                        root.file_name().unwrap(),
                        displaced.file_name().unwrap(),
                    )
                    .unwrap();
                    fs::create_dir(root).unwrap();
                    let error = admission.revalidate().unwrap_err();
                    assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageRoot);
                },
            )
            .unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn prepare_recovery_fixture(
        temp: &TempDir,
    ) -> (
        PathBuf,
        (u64, u64),
        super::super::snapshot::storage::SecureSnapshotStore,
    ) {
        let database = database_path(temp);
        let storage = SecureStorePaths::prepare(&database).unwrap();
        initialize_dux_header(&database);
        storage.mark_initialized().unwrap();
        let identity = (storage.root_identity.device, storage.root_identity.inode);
        let snapshots = super::super::snapshot::storage::SecureSnapshotStore::open_for_database(
            &database,
            super::super::snapshot::storage::SnapshotStoreAccess::ReadWrite,
        )
        .unwrap()
        .unwrap();
        drop(storage);
        (database, identity, snapshots)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn recovery_classifies_canonical_detaches_once_and_accepts_detached_resume() {
        let temp = TempDir::new().unwrap();
        let (database, identity, _snapshots) = prepare_recovery_fixture(&temp);
        let stage = OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff");

        SecureStorePaths::with_app_data_reset_recovery_namespace_until(
            &database,
            identity,
            stage,
            Instant::now() + Duration::from_secs(1),
            |admission| {
                assert_eq!(
                    admission.location(),
                    AppDataResetRecoveryDataLocation::Canonical
                );
                let admission = admission.detach_if_canonical(identity, stage).unwrap();
                assert_eq!(
                    admission.location(),
                    AppDataResetRecoveryDataLocation::Detached
                );
                admission.revalidate().unwrap();
            },
        )
        .unwrap();
        assert!(!database.parent().unwrap().exists());
        assert!(temp.path().join(stage).is_dir());

        SecureStorePaths::with_app_data_reset_recovery_namespace_until(
            &database,
            identity,
            stage,
            Instant::now() + Duration::from_secs(1),
            |admission| {
                assert_eq!(
                    admission.location(),
                    AppDataResetRecoveryDataLocation::Detached
                );
                let admission = admission.detach_if_canonical(identity, stage).unwrap();
                assert_eq!(
                    admission.location(),
                    AppDataResetRecoveryDataLocation::Detached
                );
            },
        )
        .unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn prepare_fresh_reset_fixture(
        temp: &TempDir,
    ) -> (
        PathBuf,
        (u64, u64),
        super::super::snapshot::storage::SecureSnapshotStore,
    ) {
        let (database, identity, snapshots) = prepare_recovery_fixture(temp);
        SecureStorePaths::with_app_data_reset_recovery_namespace_until(
            &database,
            identity,
            OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff"),
            Instant::now() + Duration::from_secs(1),
            |admission| {
                admission
                    .detach_if_canonical(
                        identity,
                        OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff"),
                    )
                    .unwrap()
                    .revalidate()
                    .unwrap();
            },
        )
        .unwrap();
        (database, identity, snapshots)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn fresh_reset_bootstrap_publishes_exact_pre_sqlite_layout_and_reopens() {
        use std::os::unix::fs::MetadataExt;

        let temp = TempDir::new().unwrap();
        let (database, old_identity, _snapshots) = prepare_fresh_reset_fixture(&temp);
        let data_stage = OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff");
        let fresh_stage = OsStr::new(".dux-reset-fresh-00112233445566778899aabbccddeeff");
        let transaction = "00112233445566778899aabbccddeeff";
        let fresh_identity = SecureStorePaths::with_app_data_reset_fresh_namespace_until(
            &database,
            old_identity,
            data_stage,
            fresh_stage,
            transaction,
            false,
            Instant::now() + Duration::from_secs(1),
            |fresh| {
                assert_eq!(fresh.location(), AppDataResetFreshNamespaceLocation::Absent);
                let published = fresh.publish_if_needed().unwrap();
                published.revalidate().unwrap();
                let parent = std::fs::metadata(temp.path()).unwrap();
                let parent_identity = (parent.dev(), parent.ino());
                assert!(published.is_bound_to(
                    transaction,
                    old_identity,
                    fresh_stage,
                    parent_identity,
                    OsStr::new("owned"),
                ));
                assert!(!published.is_bound_to(
                    "ffeeddccbbaa99887766554433221100",
                    old_identity,
                    fresh_stage,
                    parent_identity,
                    OsStr::new("owned"),
                ));
                assert!(!published.is_bound_to(
                    transaction,
                    (old_identity.0, old_identity.1.checked_add(1).unwrap()),
                    fresh_stage,
                    parent_identity,
                    OsStr::new("owned"),
                ));
                assert!(!published.is_bound_to(
                    transaction,
                    old_identity,
                    OsStr::new(".dux-reset-fresh-ffeeddccbbaa99887766554433221100"),
                    parent_identity,
                    OsStr::new("owned"),
                ));
                assert!(!published.is_bound_to(
                    transaction,
                    old_identity,
                    fresh_stage,
                    (parent_identity.0, parent_identity.1 + 1),
                    OsStr::new("owned"),
                ));
                assert!(!published.is_bound_to(
                    transaction,
                    old_identity,
                    fresh_stage,
                    parent_identity,
                    OsStr::new("other"),
                ));
                published.fresh_identity_parts().unwrap()
            },
        )
        .unwrap();

        assert_ne!(fresh_identity, old_identity);
        assert_eq!(
            std::fs::metadata(database.parent().unwrap()).unwrap().dev(),
            fresh_identity.0
        );
        assert_eq!(
            std::fs::metadata(database.parent().unwrap()).unwrap().ino(),
            fresh_identity.1
        );
        assert_eq!(std::fs::metadata(&database).unwrap().len(), 0);
        assert!(temp.path().join(data_stage).is_dir());
        assert!(!temp.path().join(fresh_stage).exists());
        let mut entries = std::fs::read_dir(database.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        entries.sort();
        assert_eq!(
            entries,
            [
                OsString::from(APP_DATA_RESET_FRESH_ORIGIN_NAME),
                OsString::from("dux.sqlite3"),
                cleanup_lock_name(OsStr::new("dux.sqlite3")),
                cleanup_lock_ready_name(OsStr::new("dux.sqlite3")),
                lock_name(OsStr::new("dux.sqlite3")),
            ]
        );

        SecureStorePaths::with_app_data_reset_fresh_namespace_until(
            &database,
            old_identity,
            data_stage,
            fresh_stage,
            transaction,
            false,
            Instant::now() + Duration::from_secs(1),
            |fresh| {
                assert_eq!(
                    fresh.location(),
                    AppDataResetFreshNamespaceLocation::Canonical
                );
                assert_eq!(fresh.fresh_identity_parts(), Some(fresh_identity));
                fresh.revalidate().unwrap();
            },
        )
        .unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn fresh_reset_every_typed_stage_and_canonical_effect_gap_converges_once() {
        for fault in [
            TestAppDataResetFreshNamespaceFault::AfterFreshStageRename,
            TestAppDataResetFreshNamespaceFault::AfterFreshStageSync,
            TestAppDataResetFreshNamespaceFault::AfterStagePrepared,
            TestAppDataResetFreshNamespaceFault::AfterCanonicalRename,
            TestAppDataResetFreshNamespaceFault::AfterParentSync,
            TestAppDataResetFreshNamespaceFault::DuringReadback,
        ] {
            let temp = TempDir::new().unwrap();
            let (database, old_identity, _snapshots) = prepare_fresh_reset_fixture(&temp);
            let data_stage = OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff");
            let fresh_stage = OsStr::new(".dux-reset-fresh-00112233445566778899aabbccddeeff");
            let transaction = "00112233445566778899aabbccddeeff";

            set_test_app_data_reset_fresh_namespace_fault(fault);
            let interrupted = SecureStorePaths::with_app_data_reset_fresh_namespace_until(
                &database,
                old_identity,
                data_stage,
                fresh_stage,
                transaction,
                false,
                Instant::now() + Duration::from_secs(1),
                |fresh| fresh.publish_if_needed().map(drop),
            )
            .unwrap();
            assert!(interrupted.is_err(), "fault {fault:?} did not interrupt");

            let interrupted_before_canonical = matches!(
                fault,
                TestAppDataResetFreshNamespaceFault::AfterFreshStageRename
                    | TestAppDataResetFreshNamespaceFault::AfterFreshStageSync
                    | TestAppDataResetFreshNamespaceFault::AfterStagePrepared
            );
            assert_eq!(
                database.parent().unwrap().exists(),
                !interrupted_before_canonical
            );
            assert_eq!(
                temp.path().join(fresh_stage).exists(),
                interrupted_before_canonical
            );

            SecureStorePaths::with_app_data_reset_fresh_namespace_until(
                &database,
                old_identity,
                data_stage,
                fresh_stage,
                transaction,
                false,
                Instant::now() + Duration::from_secs(1),
                |fresh| fresh.publish_if_needed().unwrap().revalidate().unwrap(),
            )
            .unwrap();
            assert!(database.parent().unwrap().is_dir());
            assert!(!temp.path().join(fresh_stage).exists());
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn fresh_reset_preserves_last_moment_canonical_collision() {
        let temp = TempDir::new().unwrap();
        let (database, old_identity, _snapshots) = prepare_fresh_reset_fixture(&temp);
        let data_stage = OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff");
        let fresh_stage = OsStr::new(".dux-reset-fresh-00112233445566778899aabbccddeeff");
        let transaction = "00112233445566778899aabbccddeeff";

        set_test_app_data_reset_fresh_namespace_fault(
            TestAppDataResetFreshNamespaceFault::RaceCanonicalCollision,
        );
        let result = SecureStorePaths::with_app_data_reset_fresh_namespace_until(
            &database,
            old_identity,
            data_stage,
            fresh_stage,
            transaction,
            false,
            Instant::now() + Duration::from_secs(1),
            |fresh| fresh.publish_if_needed().map(drop),
        )
        .unwrap();
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(database.parent().unwrap()).unwrap(),
            b"foreign race collision"
        );
        assert!(temp.path().join(fresh_stage).is_dir());

        assert_eq!(
            std::fs::read(database.parent().unwrap()).unwrap(),
            b"foreign race collision"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn fresh_reset_rejects_wrong_transaction_origin_without_mutating_stage() {
        use std::os::unix::fs::MetadataExt;

        let temp = TempDir::new().unwrap();
        let (database, old_identity, _snapshots) = prepare_fresh_reset_fixture(&temp);
        let data_stage = OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff");
        let fresh_stage = OsStr::new(".dux-reset-fresh-00112233445566778899aabbccddeeff");
        let transaction = "00112233445566778899aabbccddeeff";

        set_test_app_data_reset_fresh_namespace_fault(
            TestAppDataResetFreshNamespaceFault::AfterStagePrepared,
        );
        let first = SecureStorePaths::with_app_data_reset_fresh_namespace_until(
            &database,
            old_identity,
            data_stage,
            fresh_stage,
            transaction,
            false,
            Instant::now() + Duration::from_secs(1),
            |fresh| fresh.publish_if_needed().map(drop),
        )
        .unwrap();
        assert!(first.is_err());

        let stage_path = temp.path().join(fresh_stage);
        let origin_path = stage_path.join(APP_DATA_RESET_FRESH_ORIGIN_NAME);
        let before_metadata = std::fs::metadata(&stage_path).unwrap();
        let before_origin = std::fs::read(&origin_path).unwrap();
        assert!(
            SecureStorePaths::with_app_data_reset_fresh_namespace_until(
                &database,
                old_identity,
                data_stage,
                fresh_stage,
                "ffeeddccbbaa99887766554433221100",
                false,
                Instant::now() + Duration::from_secs(1),
                |_| (),
            )
            .is_err()
        );
        let after_metadata = std::fs::metadata(&stage_path).unwrap();
        assert_eq!(before_metadata.dev(), after_metadata.dev());
        assert_eq!(before_metadata.ino(), after_metadata.ino());
        assert_eq!(before_origin, std::fs::read(origin_path).unwrap());
        assert!(!database.parent().unwrap().exists());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn recovery_rejects_neither_without_provisioning_and_rejects_both() {
        use std::os::unix::fs::PermissionsExt;

        let empty = TempDir::new().unwrap();
        let database = database_path(&empty);
        let stage = OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff");
        let error = SecureStorePaths::with_app_data_reset_recovery_namespace_until(
            &database,
            (1, 1),
            stage,
            Instant::now() + Duration::from_secs(1),
            |_| (),
        )
        .unwrap_err();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageRoot);
        assert!(!database.parent().unwrap().exists());

        let both = TempDir::new().unwrap();
        let (database, identity, _snapshots) = prepare_recovery_fixture(&both);
        let stage_path = both.path().join(stage);
        fs::create_dir(&stage_path).unwrap();
        fs::set_permissions(&stage_path, fs::Permissions::from_mode(0o700)).unwrap();
        let error = SecureStorePaths::with_app_data_reset_recovery_namespace_until(
            &database,
            identity,
            stage,
            Instant::now() + Duration::from_secs(1),
            |_| (),
        )
        .unwrap_err();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageRoot);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "TempDir-only renames construct unsafe recovery namespace fixtures"
    )]
    fn recovery_rejects_wrong_identity_layout_alias_and_stage_type() {
        let stage = OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff");

        let incomplete = TempDir::new().unwrap();
        let incomplete_database = database_path(&incomplete);
        let incomplete_storage = SecureStorePaths::prepare(&incomplete_database).unwrap();
        initialize_dux_header(&incomplete_database);
        let incomplete_identity = (
            incomplete_storage.root_identity.device,
            incomplete_storage.root_identity.inode,
        );
        let _snapshots = super::super::snapshot::storage::SecureSnapshotStore::open_for_database(
            &incomplete_database,
            super::super::snapshot::storage::SnapshotStoreAccess::ReadWrite,
        )
        .unwrap()
        .unwrap();
        drop(incomplete_storage);
        assert!(
            SecureStorePaths::with_app_data_reset_recovery_namespace_until(
                &incomplete_database,
                incomplete_identity,
                stage,
                Instant::now() + Duration::from_secs(1),
                |_| (),
            )
            .is_err()
        );

        let wrong_identity = TempDir::new().unwrap();
        let (database, identity, _snapshots) = prepare_recovery_fixture(&wrong_identity);
        let error = SecureStorePaths::with_app_data_reset_recovery_namespace_until(
            &database,
            (identity.0, identity.1.checked_add(1).unwrap()),
            stage,
            Instant::now() + Duration::from_secs(1),
            |_| (),
        )
        .unwrap_err();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageRoot);

        let wrong_layout = TempDir::new().unwrap();
        let (database, identity, _snapshots) = prepare_recovery_fixture(&wrong_layout);
        fs::write(database.parent().unwrap().join("foreign"), b"foreign").unwrap();
        assert!(
            SecureStorePaths::with_app_data_reset_recovery_namespace_until(
                &database,
                identity,
                stage,
                Instant::now() + Duration::from_secs(1),
                |_| (),
            )
            .is_err()
        );

        let alias = TempDir::new().unwrap();
        let (database, identity, _snapshots) = prepare_recovery_fixture(&alias);
        // DUX-DESTRUCTIVE: allow=test-storage-recovery-alias-rename -- move only the TempDir-owned data root to a differently spelled sibling to prove exact-name recovery refusal
        fs::rename(database.parent().unwrap(), alias.path().join("OWNED")).unwrap();
        assert!(
            SecureStorePaths::with_app_data_reset_recovery_namespace_until(
                &database,
                identity,
                stage,
                Instant::now() + Duration::from_secs(1),
                |_| (),
            )
            .is_err()
        );

        let wrong_type = TempDir::new().unwrap();
        let (database, identity, _snapshots) = prepare_recovery_fixture(&wrong_type);
        // DUX-DESTRUCTIVE: allow=test-storage-recovery-wrong-type-rename -- move only the TempDir-owned data root aside before placing a wrong-type detached-stage fixture
        fs::rename(
            database.parent().unwrap(),
            wrong_type.path().join("displaced"),
        )
        .unwrap();
        fs::write(wrong_type.path().join(stage), b"not a directory").unwrap();
        assert!(
            SecureStorePaths::with_app_data_reset_recovery_namespace_until(
                &database,
                identity,
                stage,
                Instant::now() + Duration::from_secs(1),
                |_| (),
            )
            .is_err()
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn recovery_requires_quiescent_complete_snapshot_inventory() {
        let stage = OsStr::new(".dux-reset-data-00112233445566778899aabbccddeeff");

        let contended = TempDir::new().unwrap();
        let (database, identity, snapshots) = prepare_recovery_fixture(&contended);
        let inventory = snapshots
            .inventory_with_writer_lease(Duration::from_millis(100))
            .unwrap();
        let error = SecureStorePaths::with_app_data_reset_recovery_namespace_until(
            &database,
            identity,
            stage,
            Instant::now() + Duration::from_millis(30),
            |_| (),
        )
        .unwrap_err();
        assert_eq!(error.kind, DatabaseOpenErrorKind::Busy);
        drop(inventory);

        let foreign = TempDir::new().unwrap();
        let (database, identity, _snapshots) = prepare_recovery_fixture(&foreign);
        fs::write(
            database.parent().unwrap().join("snapshots").join("foreign"),
            b"unrecognized",
        )
        .unwrap();
        let error = SecureStorePaths::with_app_data_reset_recovery_namespace_until(
            &database,
            identity,
            stage,
            Instant::now() + Duration::from_secs(1),
            |_| (),
        )
        .unwrap_err();
        assert_eq!(error.kind, DatabaseOpenErrorKind::UnsafeStorageObject);
    }
}
