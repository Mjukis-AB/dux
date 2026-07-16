//! Private immutable snapshot-file storage.
//!
//! This layer owns only publication and file identity. Snapshot decoding,
//! checksum/version validation, retention, and database references belong to
//! higher layers. In particular, an existing exact-ID file is returned to the
//! caller for full codec validation; its name alone never proves idempotence.

use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use fs4::{FileExt, TryLockError};
use sha2::{Digest, Sha256};

use super::codec::MAX_SNAPSHOT_FILE_BYTES;

const DIRECTORY_NAME: &str = "snapshots";
const MARKER_NAME: &str = ".dux-snapshot-store";
const WRITER_LOCK_NAME: &str = ".dux-snapshot.writer.lock";
const STORE_MARKER: &[u8; 16] = b"DUXSNAPSTOREV1\0\0";
const WRITER_MARKER: &[u8; 16] = b"DUXSNAPWRITER1\0\0";
const FINAL_PREFIX: &str = "snapshot-";
const FINAL_SUFFIX: &str = ".duxsnapshot";
const FINAL_HEX_LENGTH: usize = 64;
const RANDOM_ATTEMPTS: usize = 16;
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(5);
const OPEN_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const INVENTORY_DEADLINE: Duration = Duration::from_millis(250);
const MAX_INVENTORY_ENTRIES: usize = 2_048;
const MAX_INVENTORY_NAME_BYTES: usize = 256 * 1_024;
const MAX_RECOGNIZED_TEMPS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotStorageErrorKind {
    InvalidConfiguration,
    UnsafeRoot,
    UnsafeObject,
    UnrecognizedStore,
    Unavailable,
    Busy,
    InternalState,
}

#[derive(Debug)]
pub(crate) struct SnapshotStorageError {
    kind: SnapshotStorageErrorKind,
}

impl SnapshotStorageError {
    const fn new(kind: SnapshotStorageErrorKind) -> Self {
        Self { kind }
    }

    pub(crate) const fn kind(&self) -> SnapshotStorageErrorKind {
        self.kind
    }
}

impl fmt::Display for SnapshotStorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            SnapshotStorageErrorKind::InvalidConfiguration => {
                "invalid snapshot-store configuration"
            }
            SnapshotStorageErrorKind::UnsafeRoot => "unsafe snapshot-store root",
            SnapshotStorageErrorKind::UnsafeObject => "unsafe snapshot-store object",
            SnapshotStorageErrorKind::UnrecognizedStore => "unrecognized snapshot store",
            SnapshotStorageErrorKind::Unavailable => "snapshot store unavailable",
            SnapshotStorageErrorKind::Busy => "snapshot store is busy",
            SnapshotStorageErrorKind::InternalState => "invalid snapshot-store state",
        })
    }
}

impl std::error::Error for SnapshotStorageError {}

type Result<T> = std::result::Result<T, SnapshotStorageError>;

/// A validated, single-component immutable snapshot file name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SnapshotFileName(String);

impl SnapshotFileName {
    pub(crate) fn from_scan_id(scan_id: &[u8]) -> Self {
        let digest = Sha256::digest(scan_id);
        let mut value = String::with_capacity(FINAL_PREFIX.len() + 64 + FINAL_SUFFIX.len());
        value.push_str(FINAL_PREFIX);
        push_lower_hex(&mut value, digest.as_ref());
        value.push_str(FINAL_SUFFIX);
        Self(value)
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        let expected_length = FINAL_PREFIX.len() + FINAL_HEX_LENGTH + FINAL_SUFFIX.len();
        let digest_end = FINAL_PREFIX.len() + FINAL_HEX_LENGTH;
        if value.len() != expected_length
            || !value.starts_with(FINAL_PREFIX)
            || !value.ends_with(FINAL_SUFFIX)
            || !value.as_bytes()[FINAL_PREFIX.len()..digest_end]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::UnsafeObject,
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

fn push_lower_hex(output: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity(platform::Identity);

struct StoreInner {
    path: PathBuf,
    directory: File,
    directory_identity: Identity,
    marker: File,
    marker_identity: Identity,
    writer_lock: File,
    writer_lock_identity: Identity,
    writer_in_use: AtomicBool,
}

/// Independently marker-owned snapshot directory beneath a DUX database root.
#[derive(Clone)]
pub(crate) struct SecureSnapshotStore {
    inner: Arc<StoreInner>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotStoreAccess {
    ReadOnly,
    ReadWrite,
}

/// Exact point-in-time physical usage for one retained store object.
///
/// `charged_bytes` is deliberately conservative: sparse/compressed files can
/// report either logical or allocated size as the larger value on supported
/// filesystems, so retention budgets charge the maximum of both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SnapshotFileUsage {
    logical_bytes: u64,
    allocated_bytes: u64,
    charged_bytes: u64,
}

impl SnapshotFileUsage {
    fn from_sizes(logical_bytes: u64, allocated_bytes: u64) -> Self {
        Self {
            logical_bytes,
            allocated_bytes,
            charged_bytes: logical_bytes.max(allocated_bytes),
        }
    }

    fn checked_add(self, other: Self) -> Result<Self> {
        let logical_bytes = self
            .logical_bytes
            .checked_add(other.logical_bytes)
            .ok_or_else(unsafe_inventory_object)?;
        let allocated_bytes = self
            .allocated_bytes
            .checked_add(other.allocated_bytes)
            .ok_or_else(unsafe_inventory_object)?;
        let charged_bytes = self
            .charged_bytes
            .checked_add(other.charged_bytes)
            .ok_or_else(unsafe_inventory_object)?;
        Ok(Self {
            logical_bytes,
            allocated_bytes,
            charged_bytes,
        })
    }

    pub(crate) const fn logical_bytes(self) -> u64 {
        self.logical_bytes
    }

    pub(crate) const fn allocated_bytes(self) -> u64 {
        self.allocated_bytes
    }

    pub(crate) const fn charged_bytes(self) -> u64 {
        self.charged_bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotInventoryEntryKind {
    Final(SnapshotFileName),
    /// A point-in-time observation whose process liveness is unknown.
    ///
    /// Inventory never treats this as reclaimable and exposes no mutation
    /// operation. A later scavenger needs its own durable liveness proof.
    RecognizedTemp,
}

/// Kernel-observed liveness for a recognized temporary snapshot file.
///
/// This observation is deliberately independent from the durable SQLite row:
/// the row binds a name to one staging operation, while only a contended
/// kernel lock proves that a writer still owns the file now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotTempKernelState {
    Active,
    Quiescent,
}

/// One no-follow, identity-checked object observed under the inventory lease.
///
/// The file descriptor is closed after its facts are captured. Keeping up to
/// 2,048 descriptors would exceed the common macOS launchd soft limit. The
/// store-wide writer lease preserves legitimate-name stability, and the entry
/// is reopened and identity-revalidated sequentially before handoff.
pub(crate) struct SnapshotInventoryEntry {
    name: String,
    kind: SnapshotInventoryEntryKind,
    identity: Identity,
    usage: SnapshotFileUsage,
    temp_kernel_state: Option<SnapshotTempKernelState>,
}

impl SnapshotInventoryEntry {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn kind(&self) -> &SnapshotInventoryEntryKind {
        &self.kind
    }

    pub(crate) const fn usage(&self) -> SnapshotFileUsage {
        self.usage
    }

    pub(crate) const fn temp_kernel_state(&self) -> Option<SnapshotTempKernelState> {
        self.temp_kernel_state
    }

    fn revalidate(&self, store: &StoreInner) -> Result<()> {
        platform::validate_retained(
            &store.directory,
            store.directory_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        let Some((file, identity)) =
            platform::open_named_regular(&store.directory, &store.path, &self.name, false)?
        else {
            return Err(unsafe_inventory_object());
        };
        if Identity(identity) != self.identity {
            return Err(unsafe_inventory_object());
        }
        platform::validate_retained(&file, identity, platform::Kind::RegularFile, true)?;
        platform::validate_named(
            &store.directory,
            &self.name,
            &file,
            identity,
            platform::Kind::RegularFile,
        )?;
        match &self.kind {
            SnapshotInventoryEntryKind::Final(_) => {
                if snapshot_file_usage(&file)? != self.usage {
                    return Err(unsafe_inventory_object());
                }
            }
            SnapshotInventoryEntryKind::RecognizedTemp => {
                // A quiescent observation is stable only if a second
                // nonblocking probe remains quiescent and its usage is exact.
                // An initially active writer may finish or keep growing; the
                // higher layer retains the conservative Active classification.
                if self.temp_kernel_state == Some(SnapshotTempKernelState::Quiescent)
                    && (probe_temp_kernel_state(&file)? != SnapshotTempKernelState::Quiescent
                        || snapshot_file_usage(&file)? != self.usage)
                {
                    return Err(unsafe_inventory_object());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotControlUsage {
    store_marker: SnapshotFileUsage,
    writer_lock: SnapshotFileUsage,
    total: SnapshotFileUsage,
}

impl SnapshotControlUsage {
    pub(crate) const fn store_marker(self) -> SnapshotFileUsage {
        self.store_marker
    }

    pub(crate) const fn writer_lock(self) -> SnapshotFileUsage {
        self.writer_lock
    }

    pub(crate) const fn total(self) -> SnapshotFileUsage {
        self.total
    }
}

/// A complete bounded store observation protected by one writer lease.
///
/// Holding this value excludes legitimate publishers for the entire period in
/// which higher persistence layers match database references. Entry handles
/// are opened and closed sequentially so the 2,048-entry bound does not become
/// a file-descriptor requirement. Its only mutation is a narrow, identity-
/// checked removal of one re-proven quiescent temp; higher persistence must
/// first bind that exact name to durable same-scan retry authority. It offers
/// no generic temp, final-snapshot, or user-data cleanup authority.
pub(crate) struct SnapshotStoreInventoryLease {
    store: Arc<StoreInner>,
    entries: Vec<SnapshotInventoryEntry>,
    entries_usage: SnapshotFileUsage,
    controls: SnapshotControlUsage,
    total_usage: SnapshotFileUsage,
    _writer_lock: SnapshotWriterLock,
}

impl SnapshotStoreInventoryLease {
    pub(crate) fn entries(&self) -> &[SnapshotInventoryEntry] {
        &self.entries
    }

    pub(crate) const fn entries_usage(&self) -> SnapshotFileUsage {
        self.entries_usage
    }

    pub(crate) const fn control_usage(&self) -> SnapshotControlUsage {
        self.controls
    }

    pub(crate) const fn total_usage(&self) -> SnapshotFileUsage {
        self.total_usage
    }

    /// Revalidate the exact controls and retained names after a higher layer
    /// has reconciled this point-in-time observation with SQLite state.
    pub(crate) fn revalidate(&self) -> Result<()> {
        let store = SecureSnapshotStore {
            inner: Arc::clone(&self.store),
        };
        store.validate_controls()?;
        if snapshot_file_usage(&self.store.marker)? != self.controls.store_marker()
            || snapshot_file_usage(&self.store.writer_lock)? != self.controls.writer_lock()
        {
            return Err(unsafe_inventory_object());
        }
        for entry in &self.entries {
            entry.revalidate(&self.store)?;
        }
        Ok(())
    }

    /// Reacquire and remove one exact row-bound temp after a nonblocking
    /// quiescence proof. The caller must keep the matching SQLite row and this
    /// writer lease live until it has durably consumed that row.
    pub(crate) fn remove_quiescent_temp(&mut self, name: &str) -> Result<()> {
        let observed = self
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(unsafe_inventory_object)?;
        if !matches!(observed.kind, SnapshotInventoryEntryKind::RecognizedTemp)
            || observed.temp_kernel_state != Some(SnapshotTempKernelState::Quiescent)
        {
            return Err(unsafe_inventory_object());
        }
        let Some((file, identity)) =
            platform::open_named_temp_for_removal(&self.store.directory, &self.store.path, name)?
        else {
            return Err(unsafe_inventory_object());
        };
        if Identity(identity) != observed.identity {
            return Err(unsafe_inventory_object());
        }
        match FileExt::try_lock(&file) {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
            }
            Err(TryLockError::Error(_)) => {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
        }
        platform::validate_retained(&file, identity, platform::Kind::RegularFile, true)?;
        platform::validate_named(
            &self.store.directory,
            name,
            &file,
            identity,
            platform::Kind::RegularFile,
        )?;
        platform::remove_retained_temp(&self.store.directory, name, &file, identity)?;
        platform::sync_directory(&self.store.directory)
    }
}

struct SnapshotStorageInventory {
    entries: Vec<SnapshotInventoryEntry>,
    entries_usage: SnapshotFileUsage,
    controls: SnapshotControlUsage,
    total_usage: SnapshotFileUsage,
}

fn unsafe_inventory_object() -> SnapshotStorageError {
    SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject)
}

impl SecureSnapshotStore {
    /// Opens exactly `<database parent>/snapshots`.
    ///
    /// `database_path` is configuration, not a discovered path. The containing
    /// SQLite storage layer must remain open for at least this store's lifetime
    /// so its own retained root-replacement guards remain effective.
    /// Returns `None` when a read-only database has no snapshot store yet.
    /// Read-only access never provisions or repairs storage.
    pub(crate) fn open_for_database(
        database_path: &Path,
        access: SnapshotStoreAccess,
    ) -> Result<Option<Self>> {
        if !database_path.is_absolute()
            || database_path.file_name().is_none()
            || database_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::InvalidConfiguration,
            ));
        }
        let parent_path = database_path.parent().ok_or_else(|| {
            SnapshotStorageError::new(SnapshotStorageErrorKind::InvalidConfiguration)
        })?;
        let parent = platform::open_private_directory(parent_path)?;
        let parent_identity = Identity(platform::identity(&parent, platform::Kind::Directory)?);

        let directory_path = parent_path.join(DIRECTORY_NAME);
        platform::validate_retained(&parent, parent_identity.0, platform::Kind::Directory, false)?;

        let store = match platform::open_existing_directory(&parent, parent_path, DIRECTORY_NAME)? {
            Some(directory) => Self::from_open_directory(directory_path, directory)?,
            None if access == SnapshotStoreAccess::ReadOnly => return Ok(None),
            None => Self::provision(parent_path, &parent, parent_identity, directory_path)?,
        };
        // Inventory is meaningful only while the permanent writer lock excludes
        // a legitimate publisher's current temporary file.
        let lock = store.acquire_writer_lock(OPEN_LOCK_TIMEOUT)?;
        store.validate_inventory(None)?;
        drop(lock);
        Ok(Some(store))
    }

    fn provision(
        database_root_path: &Path,
        database_root: &File,
        database_root_identity: Identity,
        directory_path: PathBuf,
    ) -> Result<Self> {
        let publication_parent_path = database_root_path.parent().ok_or_else(|| {
            SnapshotStorageError::new(SnapshotStorageErrorKind::InvalidConfiguration)
        })?;
        let publication_parent = platform::open_publication_parent(publication_parent_path)?;
        for _ in 0..RANDOM_ATTEMPTS {
            let stage_name = random_directory_stage_name()?;
            let Some(directory) = platform::create_private_directory_exclusive(
                &publication_parent,
                publication_parent_path,
                &stage_name,
            )?
            else {
                continue;
            };
            let stage_path = publication_parent_path.join(&stage_name);
            let directory_identity =
                Identity(platform::identity(&directory, platform::Kind::Directory)?);
            let (marker, marker_identity) =
                create_control(&directory, &stage_path, MARKER_NAME, STORE_MARKER)?;
            let (writer_lock, writer_lock_identity) =
                create_control(&directory, &stage_path, WRITER_LOCK_NAME, WRITER_MARKER)?;
            platform::sync_directory(&directory)?;
            platform::validate_retained(
                database_root,
                database_root_identity.0,
                platform::Kind::Directory,
                false,
            )?;
            match platform::publish_directory_no_replace(
                &publication_parent,
                &stage_name,
                &directory,
                directory_identity.0,
                database_root,
                DIRECTORY_NAME,
            )? {
                platform::Publication::Published => {
                    platform::sync_directory(database_root)?;
                    platform::sync_directory(&publication_parent)?;
                    platform::validate_named(
                        database_root,
                        DIRECTORY_NAME,
                        &directory,
                        directory_identity.0,
                        platform::Kind::Directory,
                    )?;
                    return Ok(Self::from_parts(
                        directory_path,
                        directory,
                        directory_identity,
                        marker,
                        marker_identity,
                        writer_lock,
                        writer_lock_identity,
                    ));
                }
                platform::Publication::Collision => {
                    // The bounded marker-complete loser is deliberately not
                    // scavenged. It is outside the SQLite-owned final root and
                    // cannot be mistaken for a published snapshot store.
                    let directory = platform::open_existing_directory(
                        database_root,
                        database_root_path,
                        DIRECTORY_NAME,
                    )?
                    .ok_or_else(|| {
                        SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeRoot)
                    })?;
                    return Self::from_open_directory(directory_path, directory);
                }
            }
        }
        Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::Unavailable,
        ))
    }

    fn from_open_directory(path: PathBuf, directory: File) -> Result<Self> {
        let directory_identity =
            Identity(platform::identity(&directory, platform::Kind::Directory)?);
        platform::validate_retained(
            &directory,
            directory_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        let (marker, marker_identity) =
            open_control(&directory, &path, MARKER_NAME, STORE_MARKER, false)?.ok_or_else(
                || SnapshotStorageError::new(SnapshotStorageErrorKind::UnrecognizedStore),
            )?;
        let (writer_lock, writer_lock_identity) =
            open_control(&directory, &path, WRITER_LOCK_NAME, WRITER_MARKER, true)?
                .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject))?;
        Ok(Self::from_parts(
            path,
            directory,
            directory_identity,
            marker,
            marker_identity,
            writer_lock,
            writer_lock_identity,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        path: PathBuf,
        directory: File,
        directory_identity: Identity,
        marker: File,
        marker_identity: Identity,
        writer_lock: File,
        writer_lock_identity: Identity,
    ) -> Self {
        Self {
            inner: Arc::new(StoreInner {
                path,
                directory,
                directory_identity,
                marker,
                marker_identity,
                writer_lock,
                writer_lock_identity,
                writer_in_use: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn open(&self, name: &SnapshotFileName) -> Result<Option<RetainedSnapshot>> {
        Ok(self
            .open_with_writer_lease(name, OPEN_LOCK_TIMEOUT)?
            .map(SnapshotOpenLease::into_retained))
    }

    /// Open one immutable snapshot while retaining the store-wide writer
    /// exclusion. Database-backed review pins use this to keep validation,
    /// pin insertion, and the future retention tombstone boundary in one
    /// database-before-snapshot critical section.
    pub(crate) fn open_with_writer_lease(
        &self,
        name: &SnapshotFileName,
        timeout: Duration,
    ) -> Result<Option<SnapshotOpenLease>> {
        let lock = self.acquire_writer_lock(timeout)?;
        self.validate_inventory(None)?;
        let Some((file, identity)) = platform::open_named_regular(
            &self.inner.directory,
            &self.inner.path,
            name.as_str(),
            false,
        )?
        else {
            drop(lock);
            return Ok(None);
        };
        let retained = RetainedSnapshot {
            store: Arc::clone(&self.inner),
            name: name.clone(),
            file,
            identity: Identity(identity),
        };
        retained.revalidate()?;
        Ok(Some(SnapshotOpenLease {
            retained,
            _writer_lock: lock,
        }))
    }

    /// Observe every final and recognized temporary object in one bounded
    /// directory pass while retaining the store-wide writer exclusion.
    pub(crate) fn inventory_with_writer_lease(
        &self,
        timeout: Duration,
    ) -> Result<SnapshotStoreInventoryLease> {
        let writer_lock = self.acquire_writer_lock(timeout)?;
        let inventory = self.inventory_locked(None)?;
        Ok(SnapshotStoreInventoryLease {
            store: Arc::clone(&self.inner),
            entries: inventory.entries,
            entries_usage: inventory.entries_usage,
            controls: inventory.controls,
            total_usage: inventory.total_usage,
            _writer_lock: writer_lock,
        })
    }

    pub(crate) fn reserve_stage(
        &self,
        name: SnapshotFileName,
        timeout: Duration,
    ) -> Result<SnapshotStageReservation> {
        let lock = self.acquire_writer_lock(timeout)?;
        let inventory = self.inventory_locked(None)?;
        let existing_temps = inventory
            .entries
            .iter()
            .filter(|entry| matches!(entry.kind, SnapshotInventoryEntryKind::RecognizedTemp))
            .count();
        if existing_temps >= MAX_RECOGNIZED_TEMPS {
            return Err(unsafe_inventory_object());
        }
        for _ in 0..RANDOM_ATTEMPTS {
            let temp_name = random_temp_name(&name)?;
            if inventory
                .entries
                .iter()
                .any(|entry| entry.name == temp_name)
            {
                continue;
            }
            return Ok(SnapshotStageReservation {
                store: Arc::clone(&self.inner),
                final_name: name,
                temp_name,
                lock_timeout: timeout,
                created: false,
                _writer_lock: lock,
            });
        }
        Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::Unavailable,
        ))
    }

    #[cfg(test)]
    pub(crate) fn stage(
        &self,
        name: SnapshotFileName,
        timeout: Duration,
    ) -> Result<StagedSnapshot> {
        let mut reservation = self.reserve_stage(name, timeout)?;
        reservation.create()
    }

    fn acquire_writer_lock(&self, timeout: Duration) -> Result<SnapshotWriterLock> {
        if self
            .inner
            .writer_in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        if let Err(error) = self.validate_controls() {
            self.inner.writer_in_use.store(false, Ordering::Release);
            return Err(error);
        }
        let file = self
            .inner
            .writer_lock
            .try_clone()
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable));
        let file = match file {
            Ok(file) => file,
            Err(error) => {
                self.inner.writer_in_use.store(false, Ordering::Release);
                return Err(error);
            }
        };
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            self.inner.writer_in_use.store(false, Ordering::Release);
            SnapshotStorageError::new(SnapshotStorageErrorKind::InvalidConfiguration)
        })?;
        loop {
            match FileExt::try_lock(&file) {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(
                        LOCK_RETRY_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(TryLockError::WouldBlock) => {
                    self.inner.writer_in_use.store(false, Ordering::Release);
                    return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
                }
                Err(TryLockError::Error(_)) => {
                    self.inner.writer_in_use.store(false, Ordering::Release);
                    return Err(SnapshotStorageError::new(
                        SnapshotStorageErrorKind::Unavailable,
                    ));
                }
            }
        }
        if let Err(error) = self.validate_controls() {
            record_writer_unlock(&self.inner.writer_in_use, FileExt::unlock(&file).is_ok());
            return Err(error);
        }
        Ok(SnapshotWriterLock {
            store: Arc::clone(&self.inner),
            file,
        })
    }

    fn validate_controls(&self) -> Result<()> {
        platform::validate_retained(
            &self.inner.directory,
            self.inner.directory_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        for (name, file, identity, marker) in [
            (
                MARKER_NAME,
                &self.inner.marker,
                self.inner.marker_identity,
                STORE_MARKER,
            ),
            (
                WRITER_LOCK_NAME,
                &self.inner.writer_lock,
                self.inner.writer_lock_identity,
                WRITER_MARKER,
            ),
        ] {
            platform::validate_retained(file, identity.0, platform::Kind::RegularFile, true)?;
            platform::validate_named(
                &self.inner.directory,
                name,
                file,
                identity.0,
                platform::Kind::RegularFile,
            )?;
            prove_marker(file, marker)?;
        }
        Ok(())
    }

    fn validate_inventory(&self, allowed_temp: Option<&str>) -> Result<()> {
        self.inventory_locked(allowed_temp).map(drop)
    }

    fn inventory_locked(&self, allowed_temp: Option<&str>) -> Result<SnapshotStorageInventory> {
        self.validate_controls()?;
        let deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState))?;
        let marker_usage = snapshot_file_usage(&self.inner.marker)?;
        let writer_usage = snapshot_file_usage(&self.inner.writer_lock)?;
        let controls_total = marker_usage.checked_add(writer_usage)?;
        let controls = SnapshotControlUsage {
            store_marker: marker_usage,
            writer_lock: writer_usage,
            total: controls_total,
        };
        let mut entries = Vec::new();
        let mut entries_usage = SnapshotFileUsage::default();
        let mut recognized_temps = 0_usize;
        for name in platform::inventory(
            &self.inner.directory,
            MAX_INVENTORY_ENTRIES,
            MAX_INVENTORY_NAME_BYTES,
            deadline,
        )? {
            if Instant::now() > deadline {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
            if name == MARKER_NAME || name == WRITER_LOCK_NAME {
                continue;
            }
            let kind = if is_recognized_temp_name(&name) {
                recognized_temps = recognized_temps.checked_add(1).ok_or_else(|| {
                    SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject)
                })?;
                if recognized_temps > MAX_RECOGNIZED_TEMPS {
                    return Err(SnapshotStorageError::new(
                        SnapshotStorageErrorKind::UnsafeObject,
                    ));
                }
                SnapshotInventoryEntryKind::RecognizedTemp
            } else {
                SnapshotInventoryEntryKind::Final(SnapshotFileName::parse(&name)?)
            };
            if allowed_temp.is_some_and(|allowed| name == allowed) {
                if !matches!(kind, SnapshotInventoryEntryKind::RecognizedTemp) {
                    return Err(unsafe_inventory_object());
                }
                continue;
            }
            let Some((file, identity)) = platform::open_named_regular(
                &self.inner.directory,
                &self.inner.path,
                &name,
                false,
            )?
            else {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::UnsafeObject,
                ));
            };
            platform::validate_named(
                &self.inner.directory,
                &name,
                &file,
                identity,
                platform::Kind::RegularFile,
            )?;
            let usage = snapshot_file_usage(&file)?;
            let temp_kernel_state = if matches!(kind, SnapshotInventoryEntryKind::RecognizedTemp) {
                Some(probe_temp_kernel_state(&file)?)
            } else {
                None
            };
            if matches!(kind, SnapshotInventoryEntryKind::Final(_))
                && usage.logical_bytes() > MAX_SNAPSHOT_FILE_BYTES
            {
                return Err(unsafe_inventory_object());
            }
            entries_usage = entries_usage.checked_add(usage)?;
            entries.push(SnapshotInventoryEntry {
                name,
                kind,
                identity: Identity(identity),
                usage,
                temp_kernel_state,
            });
            if Instant::now() > deadline {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
        }
        if Instant::now() > deadline {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            ));
        }
        let total_usage = controls.total().checked_add(entries_usage)?;
        Ok(SnapshotStorageInventory {
            entries,
            entries_usage,
            controls,
            total_usage,
        })
    }
}

fn snapshot_file_usage(file: &File) -> Result<SnapshotFileUsage> {
    let (logical_bytes, allocated_bytes) = platform::file_usage(file)?;
    Ok(SnapshotFileUsage::from_sizes(
        logical_bytes,
        allocated_bytes,
    ))
}

fn probe_temp_kernel_state(file: &File) -> Result<SnapshotTempKernelState> {
    match FileExt::try_lock(file) {
        Ok(()) => {
            if FileExt::unlock(file).is_err() {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
            Ok(SnapshotTempKernelState::Quiescent)
        }
        Err(TryLockError::WouldBlock) => Ok(SnapshotTempKernelState::Active),
        Err(TryLockError::Error(_)) => Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::Unavailable,
        )),
    }
}

struct SnapshotWriterLock {
    store: Arc<StoreInner>,
    file: File,
}

impl Drop for SnapshotWriterLock {
    fn drop(&mut self) {
        // An uncertain unlock keeps this process instance permanently busy;
        // descriptor closure still releases this particular OS lease.
        record_writer_unlock(
            &self.store.writer_in_use,
            FileExt::unlock(&self.file).is_ok(),
        );
    }
}

fn record_writer_unlock(in_use: &AtomicBool, unlocked: bool) {
    if unlocked {
        in_use.store(false, Ordering::Release);
    }
}

pub(crate) struct RetainedSnapshot {
    store: Arc<StoreInner>,
    name: SnapshotFileName,
    file: File,
    identity: Identity,
}

impl RetainedSnapshot {
    #[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn name(&self) -> &SnapshotFileName {
        &self.name
    }

    pub(crate) fn try_clone_file(&self) -> Result<File> {
        self.file
            .try_clone()
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))
    }

    pub(crate) fn len(&self) -> Result<u64> {
        self.revalidate()?;
        self.file
            .metadata()
            .map(|metadata| metadata.len())
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))
    }

    pub(crate) fn revalidate(&self) -> Result<()> {
        platform::validate_retained(
            &self.store.directory,
            self.store.directory_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        platform::validate_retained(
            &self.file,
            self.identity.0,
            platform::Kind::RegularFile,
            true,
        )?;
        platform::validate_named(
            &self.store.directory,
            self.name.as_str(),
            &self.file,
            self.identity.0,
            platform::Kind::RegularFile,
        )
    }
}

pub(crate) struct SnapshotPublicationLease {
    retained: RetainedSnapshot,
    _writer_lock: SnapshotWriterLock,
}

/// Snapshot-writer exclusion retained after an exact stage unlink so SQLite
/// can consume the matching durable row before another store mutation begins.
pub(crate) struct SnapshotTempMutationLease {
    _writer_lock: SnapshotWriterLock,
}

/// A retained immutable snapshot plus the store-wide writer exclusion.
///
/// This type deliberately exposes no unlink operation. Its only purpose is to
/// let the database layer commit a review pin before retention can acquire the
/// same exclusion boundary.
pub(crate) struct SnapshotOpenLease {
    retained: RetainedSnapshot,
    _writer_lock: SnapshotWriterLock,
}

impl SnapshotOpenLease {
    #[allow(
        dead_code,
        reason = "retained review leases are wired to Explorer/FFI in a later milestone slice"
    )]
    pub(crate) fn retained(&self) -> &RetainedSnapshot {
        &self.retained
    }

    pub(crate) fn into_retained(self) -> RetainedSnapshot {
        self.retained
    }
}

impl SnapshotPublicationLease {
    pub(crate) fn retained(&self) -> &RetainedSnapshot {
        &self.retained
    }
}

impl std::ops::Deref for SnapshotPublicationLease {
    type Target = RetainedSnapshot;

    fn deref(&self) -> &Self::Target {
        &self.retained
    }
}

pub(crate) enum SnapshotPublication {
    Published(SnapshotPublicationLease),
    Existing(SnapshotPublicationLease),
}

/// One generated staging name reserved while the store-wide writer lock is
/// retained.
///
/// Higher layers persist the exact durable name binding before calling
/// `create`. Keeping creation separate is what makes every physical temp
/// either row-bound or explicit pre-v8 debt after a crash.
#[must_use]
pub(crate) struct SnapshotStageReservation {
    store: Arc<StoreInner>,
    final_name: SnapshotFileName,
    temp_name: String,
    lock_timeout: Duration,
    created: bool,
    _writer_lock: SnapshotWriterLock,
}

impl SnapshotStageReservation {
    pub(crate) fn temp_name(&self) -> &str {
        &self.temp_name
    }

    /// Create the exact private file and retain an exclusive kernel lock for
    /// the complete staging lifetime. The surrounding reservation continues
    /// to hold snapshot-writer exclusion until the caller has reconciled both
    /// the durable row and this physical result.
    pub(crate) fn create(&mut self) -> Result<StagedSnapshot> {
        if self.created {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::InternalState,
            ));
        }
        let Some((file, identity)) = platform::create_private_file_exclusive(
            &self.store.directory,
            &self.store.path,
            &self.temp_name,
        )?
        else {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            ));
        };
        match FileExt::try_lock(&file) {
            Ok(()) => {}
            Err(TryLockError::WouldBlock | TryLockError::Error(_)) => {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
        }
        let staged = StagedSnapshot {
            store: Arc::clone(&self.store),
            final_name: self.final_name.clone(),
            temp_name: self.temp_name.clone(),
            file: Some(file),
            identity: Identity(identity),
            lock_timeout: self.lock_timeout,
        };
        staged.revalidate()?;
        validate_inventory_inner(&self.store, Some(&self.temp_name))?;
        self.created = true;
        Ok(staged)
    }
}

/// A create-new file owned by the current publication call.
///
/// Every normal error path aborts this retained, identity-checked temporary
/// entry while holding the database compatibility fence. Unwinding or dropping
/// without that fence is close-only and leaves one recognized retention temp;
/// Drop must never invert the database-to-snapshot lock order.
#[must_use]
pub(crate) struct StagedSnapshot {
    store: Arc<StoreInner>,
    final_name: SnapshotFileName,
    temp_name: String,
    file: Option<File>,
    identity: Identity,
    lock_timeout: Duration,
}

impl StagedSnapshot {
    pub(crate) fn sync_all(&mut self) -> Result<()> {
        self.file()?
            .sync_all()
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        self.revalidate()
    }

    pub(crate) fn publish_no_replace(mut self) -> Result<SnapshotPublication> {
        self.sync_all()?;
        let lock = SecureSnapshotStore {
            inner: Arc::clone(&self.store),
        }
        .acquire_writer_lock(self.lock_timeout)?;
        validate_inventory_inner(&self.store, Some(&self.temp_name))?;
        self.revalidate()?;
        let result = platform::publish_no_replace(
            &self.store.directory,
            &self.temp_name,
            self.file()?,
            self.identity.0,
            self.final_name.as_str(),
        )?;
        match result {
            platform::Publication::Published => {
                platform::sync_directory(&self.store.directory)?;
                let writable_file = self.file.take().ok_or_else(|| {
                    SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState)
                })?;
                platform::validate_retained(
                    &writable_file,
                    self.identity.0,
                    platform::Kind::RegularFile,
                    true,
                )?;
                platform::validate_named(
                    &self.store.directory,
                    self.final_name.as_str(),
                    &writable_file,
                    self.identity.0,
                    platform::Kind::RegularFile,
                )?;
                // Publication consumes the only DUX-owned write capability.
                // Every durable snapshot returned to higher layers is reopened
                // read-only and matched to the exact published identity.
                drop(writable_file);
                let retained =
                    open_retained_inner(&self.store, &self.final_name)?.ok_or_else(|| {
                        SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject)
                    })?;
                if retained.identity != self.identity {
                    return Err(SnapshotStorageError::new(
                        SnapshotStorageErrorKind::UnsafeObject,
                    ));
                }
                validate_inventory_inner(&self.store, None)?;
                Ok(SnapshotPublication::Published(SnapshotPublicationLease {
                    retained,
                    _writer_lock: lock,
                }))
            }
            platform::Publication::Collision => {
                let existing =
                    open_retained_inner(&self.store, &self.final_name)?.ok_or_else(|| {
                        SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject)
                    })?;
                self.remove_current_temp()?;
                validate_inventory_inner(&self.store, None)?;
                Ok(SnapshotPublication::Existing(SnapshotPublicationLease {
                    retained: existing,
                    _writer_lock: lock,
                }))
            }
        }
    }

    pub(crate) fn abort(mut self) -> Result<SnapshotTempMutationLease> {
        let lock = SecureSnapshotStore {
            inner: Arc::clone(&self.store),
        }
        .acquire_writer_lock(self.lock_timeout)?;
        self.remove_current_temp()?;
        validate_inventory_inner(&self.store, None)?;
        Ok(SnapshotTempMutationLease { _writer_lock: lock })
    }

    /// Close this private temporary file without mutating the snapshot store.
    ///
    /// Callers use this only when the database compatibility fence can no
    /// longer be acquired. The exact recognized temp then remains bounded
    /// retention debt instead of allowing an older process to write after a
    /// newer schema has won.
    pub(crate) fn abandon(mut self) {
        self.file.take();
    }

    fn file(&self) -> Result<&File> {
        self.file
            .as_ref()
            .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState))
    }

    fn revalidate(&self) -> Result<()> {
        let file = self.file()?;
        platform::validate_retained(
            &self.store.directory,
            self.store.directory_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        platform::validate_retained(file, self.identity.0, platform::Kind::RegularFile, true)?;
        platform::validate_named(
            &self.store.directory,
            &self.temp_name,
            file,
            self.identity.0,
            platform::Kind::RegularFile,
        )
    }

    fn remove_current_temp(&mut self) -> Result<()> {
        self.revalidate()?;
        let file = self
            .file
            .take()
            .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState))?;
        platform::remove_retained_temp(
            &self.store.directory,
            &self.temp_name,
            &file,
            self.identity.0,
        )?;
        platform::sync_directory(&self.store.directory)
    }
}

impl Write for StagedSnapshot {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
            .write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
            .flush()
    }
}

fn validate_inventory_inner(store: &Arc<StoreInner>, allowed_temp: Option<&str>) -> Result<()> {
    SecureSnapshotStore {
        inner: Arc::clone(store),
    }
    .validate_inventory(allowed_temp)
}

fn open_retained_inner(
    store: &Arc<StoreInner>,
    name: &SnapshotFileName,
) -> Result<Option<RetainedSnapshot>> {
    let Some((file, identity)) =
        platform::open_named_regular(&store.directory, &store.path, name.as_str(), false)?
    else {
        return Ok(None);
    };
    let retained = RetainedSnapshot {
        store: Arc::clone(store),
        name: name.clone(),
        file,
        identity: Identity(identity),
    };
    retained.revalidate()?;
    Ok(Some(retained))
}

fn create_control(
    directory: &File,
    directory_path: &Path,
    name: &str,
    marker: &[u8; 16],
) -> Result<(File, Identity)> {
    let Some((file, identity)) =
        platform::create_private_file_exclusive(directory, directory_path, name)?
    else {
        return Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::UnsafeObject,
        ));
    };
    write_all_at(&file, marker, 0)
        .and_then(|()| file.sync_all())
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    platform::validate_retained(&file, identity, platform::Kind::RegularFile, true)?;
    platform::validate_named(
        directory,
        name,
        &file,
        identity,
        platform::Kind::RegularFile,
    )?;
    prove_marker(&file, marker)?;
    Ok((file, Identity(identity)))
}

fn open_control(
    directory: &File,
    directory_path: &Path,
    name: &str,
    marker: &[u8; 16],
    writable: bool,
) -> Result<Option<(File, Identity)>> {
    let Some((file, identity)) =
        platform::open_named_regular(directory, directory_path, name, writable)?
    else {
        return Ok(None);
    };
    platform::validate_retained(&file, identity, platform::Kind::RegularFile, true)?;
    platform::validate_named(
        directory,
        name,
        &file,
        identity,
        platform::Kind::RegularFile,
    )?;
    prove_marker(&file, marker)?;
    Ok(Some((file, Identity(identity))))
}

fn prove_marker(file: &File, expected: &[u8; 16]) -> Result<()> {
    if file
        .metadata()
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?
        .len()
        != expected.len() as u64
    {
        return Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::UnsafeObject,
        ));
    }
    let mut actual = [0_u8; 16];
    read_exact_at(file, &mut actual, 0)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject))?;
    if &actual != expected {
        return Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::UnsafeObject,
        ));
    }
    Ok(())
}

fn random_temp_name(final_name: &SnapshotFileName) -> Result<String> {
    let digest_start = FINAL_PREFIX.len();
    let digest_end = digest_start + FINAL_HEX_LENGTH;
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    let mut name = String::with_capacity(128);
    name.push_str(".snapshot-");
    name.push_str(&final_name.as_str()[digest_start..digest_end]);
    name.push('.');
    name.push_str(&std::process::id().to_string());
    name.push('.');
    push_lower_hex(&mut name, &random);
    name.push_str(".tmp");
    Ok(name)
}

fn random_directory_stage_name() -> Result<String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    let mut name = String::with_capacity(64);
    name.push_str(".dux-snapshot-stage-");
    push_lower_hex(&mut name, &random);
    Ok(name)
}

fn is_recognized_temp_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(".snapshot-") else {
        return false;
    };
    let Some(rest) = rest.strip_suffix(".tmp") else {
        return false;
    };
    let mut fields = rest.split('.');
    let (Some(digest), Some(pid), Some(random), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    digest.len() == FINAL_HEX_LENGTH
        && random.len() == 32
        && !pid.is_empty()
        && pid.len() <= 10
        && digest
            .bytes()
            .chain(random.bytes())
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && pid.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buffer, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buffer, offset).and_then(|count| {
        if count == buffer.len() {
            Ok(())
        } else {
            Err(io::Error::from(io::ErrorKind::UnexpectedEof))
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_all_at(file: &File, buffer: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buffer, offset)
}

#[cfg(windows)]
fn write_all_at(file: &File, buffer: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut remaining = buffer;
    while !remaining.is_empty() {
        let written = file.seek_write(remaining, offset)?;
        if written == 0 {
            return Err(io::Error::from(io::ErrorKind::WriteZero));
        }
        offset += written as u64;
        remaining = &remaining[written..];
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
compile_error!("secure snapshot storage is unsupported on this platform");

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod platform {
    use std::ffi::{CString, OsStr};
    use std::fs::File;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use nix::dir::Dir;
    use nix::errno::Errno;
    use nix::fcntl::{OFlag, open, openat};
    use nix::sys::stat::{FchmodatFlags, Mode, SFlag, fchmod, fchmodat, fstat, mkdirat};
    use nix::unistd::geteuid;

    use super::{Result, SnapshotStorageError, SnapshotStorageErrorKind};

    const DIRECTORY_MODE: Mode = Mode::S_IRWXU;
    const FILE_MODE: Mode = Mode::S_IRUSR.union(Mode::S_IWUSR);

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct Identity {
        device: u64,
        inode: u64,
    }

    #[derive(Clone, Copy)]
    pub(super) enum Kind {
        Directory,
        RegularFile,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Publication {
        Published,
        Collision,
    }

    pub(super) fn open_private_directory(path: &Path) -> Result<File> {
        let file = open(
            path,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(map_root_error)?;
        validate_retained(
            &file,
            identity(&file, Kind::Directory)?,
            Kind::Directory,
            false,
        )?;
        Ok(file)
    }

    pub(super) fn open_publication_parent(path: &Path) -> Result<File> {
        let file = open(
            path,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(map_root_error)?;
        let status = fstat(&file)
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != SFlag::S_IFDIR
            || status.st_uid != geteuid().as_raw()
            || status.st_mode & 0o022 != 0
        {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::UnsafeRoot,
            ));
        }
        reject_granting_acl(&file)?;
        Ok(file)
    }

    pub(super) fn open_existing_directory(
        parent: &File,
        _parent_path: &Path,
        name: &str,
    ) -> Result<Option<File>> {
        match openat(
            parent,
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(descriptor) => {
                let directory = File::from(descriptor);
                let object_identity = identity(&directory, Kind::Directory)?;
                validate_retained(&directory, object_identity, Kind::Directory, false)?;
                Ok(Some(directory))
            }
            Err(Errno::ENOENT) => Ok(None),
            Err(error) => Err(map_root_error(error)),
        }
    }

    pub(super) fn create_private_directory_exclusive(
        parent: &File,
        _parent_path: &Path,
        name: &str,
    ) -> Result<Option<File>> {
        match mkdirat(parent, name, DIRECTORY_MODE) {
            Ok(()) => {}
            Err(Errno::EEXIST) => return Ok(None),
            Err(error) => return Err(map_root_error(error)),
        }
        fchmodat(parent, name, DIRECTORY_MODE, FchmodatFlags::NoFollowSymlink)
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        let directory = openat(
            parent,
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(map_root_error)?;
        fchmod(&directory, DIRECTORY_MODE)
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        let object_identity = identity(&directory, Kind::Directory)?;
        validate_retained(&directory, object_identity, Kind::Directory, false)?;
        Ok(Some(directory))
    }

    pub(super) fn create_private_file_exclusive(
        directory: &File,
        _directory_path: &Path,
        name: &str,
    ) -> Result<Option<(File, Identity)>> {
        match openat(
            directory,
            name,
            OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            FILE_MODE,
        ) {
            Ok(descriptor) => {
                let file = File::from(descriptor);
                fchmod(&file, FILE_MODE).map_err(|_| {
                    SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable)
                })?;
                let object_identity = identity(&file, Kind::RegularFile)?;
                validate_retained(&file, object_identity, Kind::RegularFile, true)?;
                Ok(Some((file, object_identity)))
            }
            Err(Errno::EEXIST) => Ok(None),
            Err(Errno::ELOOP | Errno::EMLINK | Errno::EISDIR | Errno::ENOTDIR) => Err(
                SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject),
            ),
            Err(_) => Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            )),
        }
    }

    pub(super) fn open_named_regular(
        directory: &File,
        _directory_path: &Path,
        name: &str,
        writable: bool,
    ) -> Result<Option<(File, Identity)>> {
        let access = if writable {
            OFlag::O_RDWR
        } else {
            OFlag::O_RDONLY
        };
        match openat(
            directory,
            name,
            access | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(descriptor) => {
                let file = File::from(descriptor);
                let object_identity = identity(&file, Kind::RegularFile)?;
                validate_retained(&file, object_identity, Kind::RegularFile, true)?;
                Ok(Some((file, object_identity)))
            }
            Err(Errno::ENOENT) => Ok(None),
            Err(Errno::ELOOP | Errno::EMLINK | Errno::EISDIR | Errno::ENOTDIR) => Err(
                SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject),
            ),
            Err(_) => Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            )),
        }
    }

    pub(super) fn open_named_temp_for_removal(
        directory: &File,
        directory_path: &Path,
        name: &str,
    ) -> Result<Option<(File, Identity)>> {
        open_named_regular(directory, directory_path, name, true)
    }

    pub(super) fn identity(file: &File, kind: Kind) -> Result<Identity> {
        let status = fstat(file).map_err(|_| unavailable_for(kind))?;
        let expected_type = match kind {
            Kind::Directory => SFlag::S_IFDIR,
            Kind::RegularFile => SFlag::S_IFREG,
        };
        if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != expected_type {
            return Err(unsafe_for(kind));
        }
        Ok(Identity {
            device: status.st_dev as u64,
            inode: status.st_ino as u64,
        })
    }

    pub(super) fn file_usage(file: &File) -> Result<(u64, u64)> {
        let status = fstat(file).map_err(|_| unavailable_for(Kind::RegularFile))?;
        let logical_bytes =
            u64::try_from(status.st_size).map_err(|_| unsafe_for(Kind::RegularFile))?;
        let blocks = u64::try_from(status.st_blocks).map_err(|_| unsafe_for(Kind::RegularFile))?;
        let allocated_bytes = blocks
            .checked_mul(512)
            .ok_or_else(|| unsafe_for(Kind::RegularFile))?;
        Ok((logical_bytes, allocated_bytes))
    }

    pub(super) fn validate_retained(
        file: &File,
        expected: Identity,
        kind: Kind,
        require_one_link: bool,
    ) -> Result<()> {
        let status = fstat(file).map_err(|_| unavailable_for(kind))?;
        let actual = identity(file, kind)?;
        let mode = status.st_mode & 0o7777;
        let expected_mode = match kind {
            Kind::Directory => 0o700,
            Kind::RegularFile => 0o600,
        };
        if actual != expected
            || status.st_uid != geteuid().as_raw()
            || mode != expected_mode
            || (require_one_link && status.st_nlink != 1)
        {
            return Err(unsafe_for(kind));
        }
        reject_extended_acl(file, kind)?;
        Ok(())
    }

    pub(super) fn validate_named(
        directory: &File,
        name: &str,
        retained: &File,
        expected: Identity,
        kind: Kind,
    ) -> Result<()> {
        let flags = match kind {
            Kind::Directory => {
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
            }
            Kind::RegularFile => {
                OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
            }
        };
        let opened = openat(directory, name, flags, Mode::empty())
            .map(File::from)
            .map_err(|_| unsafe_for(kind))?;
        validate_retained(retained, expected, kind, matches!(kind, Kind::RegularFile))?;
        validate_retained(&opened, expected, kind, matches!(kind, Kind::RegularFile))
    }

    pub(super) fn inventory(
        directory: &File,
        maximum_entries: usize,
        maximum_name_bytes: usize,
        deadline: std::time::Instant,
    ) -> Result<Vec<String>> {
        let clone = directory
            .try_clone()
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        let owned: OwnedFd = clone.into();
        let mut entries = Dir::from_fd(owned)
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        let mut names = Vec::new();
        let mut name_bytes = 0_usize;
        for entry in entries.iter() {
            if std::time::Instant::now() > deadline || names.len() >= maximum_entries {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
            let entry = entry
                .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            let name = std::str::from_utf8(bytes)
                .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject))?;
            name_bytes = name_bytes
                .checked_add(name.len())
                .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject))?;
            if name_bytes > maximum_name_bytes {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::UnsafeObject,
                ));
            }
            names.push(name.to_owned());
        }
        Ok(names)
    }

    pub(super) fn sync_directory(directory: &File) -> Result<()> {
        directory
            .sync_all()
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))
    }

    pub(super) fn publish_no_replace(
        directory: &File,
        source: &str,
        _source_file: &File,
        _source_identity: Identity,
        destination: &str,
    ) -> Result<Publication> {
        publish_between(
            directory,
            source,
            directory,
            destination,
            PublicationKind::SnapshotFile,
        )
    }

    pub(super) fn publish_directory_no_replace(
        source_parent: &File,
        source: &str,
        _source_directory: &File,
        _source_identity: Identity,
        destination_parent: &File,
        destination: &str,
    ) -> Result<Publication> {
        publish_between(
            source_parent,
            source,
            destination_parent,
            destination,
            PublicationKind::StoreDirectory,
        )
    }

    #[derive(Clone, Copy)]
    enum PublicationKind {
        SnapshotFile,
        StoreDirectory,
    }

    fn publish_between(
        source_parent: &File,
        source: &str,
        destination_parent: &File,
        destination: &str,
        publication_kind: PublicationKind,
    ) -> Result<Publication> {
        match rename_no_replace(
            source_parent,
            OsStr::new(source),
            destination_parent,
            OsStr::new(destination),
            publication_kind,
        ) {
            Ok(()) => Ok(Publication::Published),
            Err(Errno::EEXIST | Errno::ENOTEMPTY) => Ok(Publication::Collision),
            Err(_) => Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            )),
        }
    }

    #[cfg(target_os = "linux")]
    fn rename_no_replace(
        source_parent: &File,
        source: &OsStr,
        destination_parent: &File,
        destination: &OsStr,
        _publication_kind: PublicationKind,
    ) -> std::result::Result<(), Errno> {
        let source = CString::new(source.as_bytes()).map_err(|_| Errno::EINVAL)?;
        let destination = CString::new(destination.as_bytes()).map_err(|_| Errno::EINVAL)?;
        // SAFETY: both names are validated single components beneath retained
        // directories on the same filesystem. RENAME_NOREPLACE forbids overwrite.
        let result = unsafe {
            nix::libc::syscall(
                // DUX-DESTRUCTIVE: allow=snapshot-linux-no-replace-publish -- atomically publish only a retained marker-complete store stage or current create-new snapshot temp without replacing an entry
                nix::libc::SYS_renameat2,
                source_parent.as_raw_fd(),
                source.as_ptr(),
                destination_parent.as_raw_fd(),
                destination.as_ptr(),
                nix::libc::RENAME_NOREPLACE,
            )
        };
        Errno::result(result).map(drop)
    }

    #[cfg(target_os = "macos")]
    fn rename_no_replace(
        source_parent: &File,
        source: &OsStr,
        destination_parent: &File,
        destination: &OsStr,
        _publication_kind: PublicationKind,
    ) -> std::result::Result<(), Errno> {
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
        // SAFETY: both names are validated single components beneath retained
        // directories. EXCL forbids replacement and NOFOLLOW_ANY rejects
        // symlink traversal anywhere in publication.
        let result = unsafe {
            // DUX-DESTRUCTIVE: allow=snapshot-macos-no-replace-publish -- atomically publish only a retained marker-complete store stage or current create-new snapshot temp without replacing an entry
            renameatx_np(
                source_parent.as_raw_fd(),
                source.as_ptr(),
                destination_parent.as_raw_fd(),
                destination.as_ptr(),
                RENAME_EXCL | RENAME_NOFOLLOW_ANY,
            )
        };
        Errno::result(result).map(drop)
    }

    pub(super) fn remove_retained_temp(
        directory: &File,
        name: &str,
        file: &File,
        expected: Identity,
    ) -> Result<()> {
        use nix::unistd::{UnlinkatFlags, unlinkat};
        validate_named(directory, name, file, expected, Kind::RegularFile)?;
        // DUX-DESTRUCTIVE: allow=snapshot-current-temp-unlink -- remove only the current call's retained create-new private snapshot temp after exact identity revalidation
        unlinkat(directory, name, UnlinkatFlags::NoRemoveDir)
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))
    }

    #[cfg(target_os = "macos")]
    fn reject_extended_acl(file: &File, kind: Kind) -> Result<()> {
        use std::ffi::{c_int, c_void};
        const ACL_TYPE_EXTENDED: c_int = 0x100;
        const ACL_FIRST_ENTRY: c_int = 0;
        unsafe extern "C" {
            fn acl_get_fd_np(fd: c_int, acl_type: c_int) -> *mut c_void;
            fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
            fn acl_free(object: *mut c_void) -> c_int;
        }
        // SAFETY: the retained descriptor is live and Darwin allocates the ACL.
        let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
        if acl.is_null() {
            if Errno::last() == Errno::ENOENT {
                return Ok(());
            }
            return Err(unsafe_for(kind));
        }
        let mut entry = std::ptr::null_mut();
        // SAFETY: `acl` is live and the output pointer is writable.
        let status = unsafe { acl_get_entry(acl, ACL_FIRST_ENTRY, &raw mut entry) };
        // SAFETY: `acl` is freed exactly once.
        let freed = unsafe { acl_free(acl) };
        if status < 0 || freed != 0 || !entry.is_null() {
            return Err(unsafe_for(kind));
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn reject_extended_acl(_file: &File, _kind: Kind) -> Result<()> {
        // Linux ACL inspection needs the optional libacl ABI. Exact 0700/0600
        // modes ensure any POSIX ACL mask grants no group/other permission.
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn reject_granting_acl(file: &File) -> Result<()> {
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
        // SAFETY: the retained descriptor is live and Darwin allocates the ACL.
        let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
        if acl.is_null() {
            if Errno::last() == Errno::ENOENT {
                return Ok(());
            }
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::UnsafeRoot,
            ));
        }
        let inspection = (|| {
            let mut selector = ACL_FIRST_ENTRY;
            for _ in 0..ACL_MAX_ENTRIES {
                let mut entry = std::ptr::null_mut();
                // SAFETY: `acl` is live and the output pointer is writable.
                if unsafe { acl_get_entry(acl, selector, &raw mut entry) } < 0 {
                    if selector == ACL_NEXT_ENTRY && Errno::last() == Errno::EINVAL {
                        return Ok(());
                    }
                    return Err(SnapshotStorageError::new(
                        SnapshotStorageErrorKind::UnsafeRoot,
                    ));
                }
                if entry.is_null() {
                    return Ok(());
                }
                let mut tag = 0;
                // SAFETY: `entry` belongs to the live ACL.
                if unsafe { acl_get_tag_type(entry, &raw mut tag) } != 0 || tag != ACL_EXTENDED_DENY
                {
                    return Err(SnapshotStorageError::new(
                        SnapshotStorageErrorKind::UnsafeRoot,
                    ));
                }
                selector = ACL_NEXT_ENTRY;
            }
            Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::UnsafeRoot,
            ))
        })();
        // SAFETY: `acl` is freed exactly once.
        if unsafe { acl_free(acl) } != 0 {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::UnsafeRoot,
            ));
        }
        inspection
    }

    #[cfg(target_os = "linux")]
    fn reject_granting_acl(_file: &File) -> Result<()> {
        Ok(())
    }

    fn map_root_error(error: Errno) -> SnapshotStorageError {
        match error {
            Errno::ELOOP | Errno::ENOTDIR | Errno::EISDIR => {
                SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeRoot)
            }
            _ => SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable),
        }
    }

    fn unsafe_for(kind: Kind) -> SnapshotStorageError {
        SnapshotStorageError::new(match kind {
            Kind::Directory => SnapshotStorageErrorKind::UnsafeRoot,
            Kind::RegularFile => SnapshotStorageErrorKind::UnsafeObject,
        })
    }

    fn unavailable_for(_kind: Kind) -> SnapshotStorageError {
        SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable)
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    // DUX-DESTRUCTIVE: allow=test-snapshot-lock-command-import -- import only the fixed Rust test-harness relaunch primitive used by the bounded cross-process writer-lock regression
    use std::process::Command;
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

    #[test]
    fn typed_name_is_exact_lowercase_single_component() {
        let name = SnapshotFileName::from_scan_id(b"scan-1");
        assert_eq!(name.as_str().len(), 85);
        assert_eq!(SnapshotFileName::parse(name.as_str()).unwrap(), name);
        assert!(SnapshotFileName::parse("../snapshot-deadbeef.duxsnapshot").is_err());
        assert!(SnapshotFileName::parse(
            "snapshot-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.duxsnapshot"
        )
        .is_err());
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
        let temp_name =
            random_temp_name(&SnapshotFileName::from_scan_id(b"oversized-temp")).unwrap();
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
            .arg(
                "persistence::snapshot::storage::tests::cross_process_writer_contention_is_bounded",
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
}

#[cfg(windows)]
#[path = "storage/windows.rs"]
mod platform;
