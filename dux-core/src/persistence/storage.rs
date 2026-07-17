use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fs4::{FileExt, TryLockError};

use super::status::{DatabaseOpenError, DatabaseOpenErrorKind};

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

impl SecureStorePaths {
    pub(crate) fn prepare(database_path: &Path) -> Result<Self, DatabaseOpenError> {
        StoreProbe::prepare(database_path)?.secure()
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

    pub(crate) fn validate_all_existing(&self) -> Result<(), DatabaseOpenError> {
        self.validate_for_database_open()
    }

    /// Acquire the cross-process writer/migration lease within a fixed bound.
    pub(crate) fn acquire_writer_lock(
        &self,
        timeout: Duration,
    ) -> Result<WriterLockGuard, DatabaseOpenError> {
        if self
            .writer_lock_in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }

        let result = acquire_advisory_lock(&self.lock_file, timeout).map(|file| WriterLockGuard {
            file,
            in_use: Arc::clone(&self.writer_lock_in_use),
        });
        if result.is_err() {
            self.writer_lock_in_use.store(false, Ordering::Release);
        }
        result
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
        self.validate_control_objects()?;
        if self
            .cleanup_lock_in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(DatabaseOpenError::new(DatabaseOpenErrorKind::Busy));
        }

        let result =
            acquire_advisory_lock(&self.cleanup_lock_file, timeout).map(|file| CleanupLockGuard {
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

    #[cfg(test)]
    fn lock_path(&self) -> &Path {
        &self.lock_path
    }

    #[cfg(test)]
    fn cleanup_lock_path(&self) -> &Path {
        &self.cleanup_lock_path
    }
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
    let lock_file = file
        .try_clone()
        .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::DatabaseUnavailable))?;
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?;
    loop {
        match FileExt::try_lock(&lock_file) {
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
mod platform {
    use std::ffi::{CString, OsStr, OsString};
    use std::fs::File;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::FileExt;
    use std::path::Path;
    use std::time::Instant;

    use nix::dir::Dir;
    use nix::errno::Errno;
    use nix::fcntl::{AtFlags, OFlag, open, openat};
    use nix::sys::stat::{Mode, SFlag, fchmod, fstat, fstatat, mkdirat};
    use nix::unistd::geteuid;

    use super::{
        DatabaseOpenError, DatabaseOpenErrorKind, MAX_SNAPSHOT_STAGES, ObjectKind,
        PermissionPolicy, PlatformIdentity, PreparedRoot, PreparedRootState,
        ROOT_INVENTORY_MAX_NAME_BYTES, ROOT_INVENTORY_TIMEOUT, RootPublicationResult,
        is_canonical_snapshot_stage_name, object_error, storage_root_error,
    };

    const DIRECTORY_MODE: Mode = Mode::S_IRWXU;
    const FILE_MODE: Mode = Mode::S_IRUSR.union(Mode::S_IWUSR);
    const STAGING_ATTEMPTS: usize = 8;

    pub(super) fn sync_directory(directory: &File) -> Result<(), DatabaseOpenError> {
        directory
            .sync_all()
            .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))
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
        validate_publication_parent(&parent)?;

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

        let parent_path = final_root_path
            .parent()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let stage_name = prepared
            .object_path
            .file_name()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
        let final_name = final_root_path
            .file_name()
            .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
        let parent = open(
            parent_path,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(map_root_open_error)?;
        validate_publication_parent(&parent)?;
        match rename_no_replace(&parent, stage_name, final_name) {
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
        parent
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

    fn validate_publication_parent(parent: &File) -> Result<(), DatabaseOpenError> {
        let status = fstat(parent)
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
        if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != SFlag::S_IFDIR
            || status.st_uid != geteuid().as_raw()
            || status.st_mode & 0o022 != 0
        {
            return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
        }
        reject_granting_acl(parent)?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn rename_no_replace(parent: &File, source: &OsStr, destination: &OsStr) -> Result<(), Errno> {
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
    fn rename_no_replace(parent: &File, source: &OsStr, destination: &OsStr) -> Result<(), Errno> {
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
            if is_canonical_snapshot_stage_name(actual_name) {
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
            if names_resolve_to_same_entry(root_directory, actual_name, allowed)? {
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
}
