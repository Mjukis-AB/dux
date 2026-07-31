//! Private marker-owned storage for the managed CLI scan cache.
//!
//! The configured `Dux` cache directory is only a conventional container. It
//! may already be the case-insensitive alias of the legacy unmarked `dux`
//! directory, so this module never marks, inventories, attributes, or clears
//! that outer directory. Ownership begins at its fixed `scan-cache-v1` child.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use fs4::{FileExt, TryLockError};

use crate::app_data_reset_transaction::{AppDataResetCacheStageName, AppDataResetTransaction};

use super::managed_codec::{
    MANAGED_CACHE_HEADER_BYTES, MAX_MANAGED_CACHE_FILE_BYTES, ManagedCacheDocument,
    ManagedCacheEntryKey, decode_managed_cache, encode_managed_cache, managed_cache_entry_key,
    preflight_managed_cache_header,
};
use super::{CacheMetadata, CachedScanConfig};
use crate::tree::DiskTree;

const STORE_DIRECTORY_NAME: &str = "scan-cache-v1";
const MARKER_NAME: &str = ".dux-cache-store";
const WRITER_LOCK_NAME: &str = ".dux-cache.writer.lock";
const STORE_MARKER: &[u8; 16] = b"DUXCACHESTOREV1\0";
const WRITER_MARKER: &[u8; 16] = b"DUXCACHELOCKV1\0\0";
const FINAL_SUFFIX: &str = ".dux";
const KEY_HEX_BYTES: usize = 64;
const RANDOM_ATTEMPTS: usize = 16;
const STAGE_PREFIX: &str = ".dux-cache-stage-";
const RANDOM_HEX_BYTES: usize = 32;
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(5);
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const INVENTORY_DEADLINE: Duration = Duration::from_millis(250);
const MAX_NON_CONTROL_OBJECTS: usize = 2_048;
const MAX_TEMPORARY_OBJECTS: usize = 64;
const RECOVERY_MAX_NON_CONTROL_OBJECTS: usize = MAX_NON_CONTROL_OBJECTS + 1;
const RECOVERY_MAX_TEMPORARY_OBJECTS: usize = MAX_TEMPORARY_OBJECTS + 1;
const MAX_INVENTORY_NAME_BYTES: usize = 256 * 1_024;
const TEST_FAULT_PRE_PUBLICATION: u8 = 1;
const TEST_FAULT_POST_PUBLICATION: u8 = 2;

static PUBLICATION_LOCKS_IN_USE: LazyLock<Mutex<BTreeSet<platform::Identity>>> =
    LazyLock::new(|| Mutex::new(BTreeSet::new()));

#[cfg(test)]
std::thread_local! {
    static TEST_FAULT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    static TEST_INVENTORY_DELAY_MS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn take_test_fault(expected: u8) -> bool {
    #[cfg(test)]
    {
        TEST_FAULT.with(|fault| {
            if fault.get() == expected {
                fault.set(0);
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
fn set_test_fault(fault: u8) {
    TEST_FAULT.with(|current| current.set(fault));
}

fn take_test_inventory_delay() -> Duration {
    #[cfg(test)]
    {
        TEST_INVENTORY_DELAY_MS.with(|delay| {
            let delay = delay.replace(0);
            Duration::from_millis(delay)
        })
    }
    #[cfg(not(test))]
    {
        Duration::ZERO
    }
}

#[cfg(test)]
fn set_test_inventory_delay(delay: Duration) {
    TEST_INVENTORY_DELAY_MS.with(|current| {
        current.set(u64::try_from(delay.as_millis()).unwrap_or(u64::MAX));
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManagedCacheStoreAccess {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManagedCacheStoreErrorKind {
    InvalidConfiguration,
    ReadOnly,
    UnsafeContainer,
    UnsafeStore,
    UnsafeObject,
    UnrecognizedStore,
    Busy,
    BudgetExceeded,
    ChangedSinceSnapshot,
    CorruptData,
    Unavailable,
    OutcomeUnknown,
    #[allow(dead_code, reason = "constructed by the fail-closed non-Unix backend")]
    UnsupportedPlatform,
    InternalState,
}

#[derive(Debug)]
pub(crate) struct ManagedCacheStoreError {
    kind: ManagedCacheStoreErrorKind,
}

impl ManagedCacheStoreError {
    const fn new(kind: ManagedCacheStoreErrorKind) -> Self {
        Self { kind }
    }

    pub(crate) const fn kind(&self) -> ManagedCacheStoreErrorKind {
        self.kind
    }
}

impl fmt::Display for ManagedCacheStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ManagedCacheStoreErrorKind::InvalidConfiguration => {
                "invalid managed-cache configuration"
            }
            ManagedCacheStoreErrorKind::ReadOnly => "managed cache is read-only",
            ManagedCacheStoreErrorKind::UnsafeContainer => {
                "unsafe managed-cache publication container"
            }
            ManagedCacheStoreErrorKind::UnsafeStore => "unsafe managed-cache store",
            ManagedCacheStoreErrorKind::UnsafeObject => "unsafe managed-cache object",
            ManagedCacheStoreErrorKind::UnrecognizedStore => "unrecognized managed-cache store",
            ManagedCacheStoreErrorKind::Busy => "managed cache is busy",
            ManagedCacheStoreErrorKind::BudgetExceeded => "managed-cache resource budget exceeded",
            ManagedCacheStoreErrorKind::ChangedSinceSnapshot => {
                "managed cache changed since it was inspected"
            }
            ManagedCacheStoreErrorKind::CorruptData => "managed-cache data is corrupt",
            ManagedCacheStoreErrorKind::Unavailable => "managed cache is unavailable",
            ManagedCacheStoreErrorKind::OutcomeUnknown => {
                "the managed-cache operation outcome is unknown"
            }
            ManagedCacheStoreErrorKind::UnsupportedPlatform => {
                "secure managed-cache storage is unsupported on this platform"
            }
            ManagedCacheStoreErrorKind::InternalState => "invalid managed-cache internal state",
        })
    }
}

impl std::error::Error for ManagedCacheStoreError {}

type Result<T> = std::result::Result<T, ManagedCacheStoreError>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ManagedCacheStorageUsage {
    pub(crate) logical_bytes: u64,
    pub(crate) allocated_bytes: u64,
    pub(crate) charged_bytes: u64,
}

impl ManagedCacheStorageUsage {
    fn from_file(file: &File) -> Result<Self> {
        let (logical_bytes, allocated_bytes) = platform::file_usage(file)?;
        Ok(Self {
            logical_bytes,
            allocated_bytes,
            charged_bytes: logical_bytes.max(allocated_bytes),
        })
    }

    fn checked_add(self, other: Self) -> Result<Self> {
        Ok(Self {
            logical_bytes: self
                .logical_bytes
                .checked_add(other.logical_bytes)
                .ok_or_else(corrupt)?,
            allocated_bytes: self
                .allocated_bytes
                .checked_add(other.allocated_bytes)
                .ok_or_else(corrupt)?,
            charged_bytes: self
                .charged_bytes
                .checked_add(other.charged_bytes)
                .ok_or_else(corrupt)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ManagedCacheStoreFootprint {
    pub(crate) controls: ManagedCacheStorageUsage,
    pub(crate) entries: ManagedCacheStorageUsage,
    pub(crate) temporary: ManagedCacheStorageUsage,
    pub(crate) total: ManagedCacheStorageUsage,
    pub(crate) entry_count: u32,
    pub(crate) temporary_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ManagedCacheClearResult {
    pub(crate) cleared_entries: u32,
    pub(crate) cleared_temporary: u32,
    pub(crate) cleared_usage: ManagedCacheStorageUsage,
}

#[derive(Debug)]
pub(crate) enum ManagedCacheSaveError {
    BeforePublication(ManagedCacheStoreError),
    OutcomeUnknown,
}

#[derive(Debug)]
pub(crate) enum ManagedCacheClearError {
    BeforeEffect(ManagedCacheStoreError),
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InventoryKind {
    Entry,
    Temporary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObjectFacts {
    identity: platform::Identity,
    change: platform::ChangeToken,
    usage: ManagedCacheStorageUsage,
}

struct InventoryObject {
    name: String,
    kind: InventoryKind,
    file: File,
    facts: ObjectFacts,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InventoryFacts {
    controls: ManagedCacheStorageUsage,
    objects: Vec<(String, InventoryKind, ObjectFacts)>,
}

struct ManagedCacheInventory {
    controls: ManagedCacheStorageUsage,
    entries: ManagedCacheStorageUsage,
    temporary: ManagedCacheStorageUsage,
    total: ManagedCacheStorageUsage,
    objects: Vec<InventoryObject>,
}

impl ManagedCacheInventory {
    fn footprint(&self) -> Result<ManagedCacheStoreFootprint> {
        let entry_count = self
            .objects
            .iter()
            .filter(|object| object.kind == InventoryKind::Entry)
            .count();
        let temporary_count = self.objects.len().saturating_sub(entry_count);
        Ok(ManagedCacheStoreFootprint {
            controls: self.controls,
            entries: self.entries,
            temporary: self.temporary,
            total: self.total,
            entry_count: u32::try_from(entry_count).map_err(|_| budget())?,
            temporary_count: u32::try_from(temporary_count).map_err(|_| budget())?,
        })
    }

    fn facts(&self) -> InventoryFacts {
        InventoryFacts {
            controls: self.controls,
            objects: self
                .objects
                .iter()
                .map(|object| (object.name.clone(), object.kind, object.facts))
                .collect(),
        }
    }

    fn clearable_usage(&self) -> Result<ManagedCacheStorageUsage> {
        self.entries.checked_add(self.temporary)
    }

    fn reserve_save_capacity(&self, destination: &str) -> Result<()> {
        let replacing = self.objects.iter().any(|object| {
            object.kind == InventoryKind::Entry && object.name.as_str() == destination
        });
        let temporary_count = self
            .objects
            .iter()
            .filter(|object| object.kind == InventoryKind::Temporary)
            .count();
        validate_save_capacity(self.objects.len(), temporary_count, replacing)
    }
}

fn validate_save_capacity(
    object_count: usize,
    temporary_count: usize,
    replacing: bool,
) -> Result<()> {
    if object_count > MAX_NON_CONTROL_OBJECTS
        || temporary_count >= MAX_TEMPORARY_OBJECTS
        || (!replacing && object_count >= MAX_NON_CONTROL_OBJECTS)
    {
        return Err(budget());
    }
    Ok(())
}

struct StoreInner {
    parent_path: PathBuf,
    parent: File,
    parent_identity: platform::Identity,
    container_path: PathBuf,
    container: File,
    container_identity: platform::Identity,
    path: PathBuf,
    directory: File,
    directory_identity: platform::Identity,
    marker: File,
    marker_identity: platform::Identity,
    writer_lock: File,
    writer_lock_identity: platform::Identity,
    writer_in_use: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct ManagedCacheStore {
    inner: Arc<StoreInner>,
    access: ManagedCacheStoreAccess,
}

pub(crate) struct ManagedCacheClearSnapshot {
    store: Arc<StoreInner>,
    facts: InventoryFacts,
    footprint: ManagedCacheStoreFootprint,
}

impl ManagedCacheClearSnapshot {
    pub(crate) const fn footprint(&self) -> ManagedCacheStoreFootprint {
        self.footprint
    }
}

struct WriterLock {
    store: Arc<StoreInner>,
    file: File,
}

struct PublicationDirectoryLock {
    identity: platform::Identity,
    file: File,
}

struct PublicationContainer {
    file: File,
    identity: platform::Identity,
    lock: PublicationDirectoryLock,
}

/// Retained, descriptor-backed exclusion for the only two namespace
/// publications performed by this store. The conventional outer container is
/// never marked or inventoried: its directory descriptor itself is the
/// advisory lock object.
struct PublicationFence {
    // Container lock drops before the parent lock.
    container: Option<PublicationContainer>,
    parent_lock: PublicationDirectoryLock,
    parent_path: PathBuf,
    parent: File,
    parent_identity: platform::Identity,
    container_path: PathBuf,
}

/// Borrowed, non-mutating managed-cache writer admission for app-data reset.
///
/// The owned writer lock remains local to
/// `with_app_data_reset_writer_admission`; the higher-ranked callback cannot
/// return this wrapper or the lock it borrows.
pub(crate) struct AppDataResetManagedCacheAdmission<'scope> {
    state: AppDataResetManagedCacheAdmissionState<'scope>,
    deadline: Instant,
}

enum AppDataResetManagedCacheAdmissionState<'scope> {
    Present {
        store: &'scope ManagedCacheStore,
        expected: InventoryFacts,
        _writer_lock: &'scope WriterLock,
        publication: &'scope PublicationFence,
        cache_stage: &'scope AppDataResetCacheStageName,
    },
    Absent {
        publication: &'scope PublicationFence,
        cache_stage: &'scope AppDataResetCacheStageName,
    },
}

impl AppDataResetManagedCacheAdmission<'_> {
    pub(crate) const fn is_present(&self) -> bool {
        matches!(
            self.state,
            AppDataResetManagedCacheAdmissionState::Present { .. }
        )
    }

    pub(crate) fn revalidate(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            return Err(busy());
        }
        match &self.state {
            AppDataResetManagedCacheAdmissionState::Present {
                store,
                expected,
                publication,
                cache_stage,
                ..
            } => {
                publication.revalidate()?;
                publication.validate_cache_stage_absent(cache_stage)?;
                publication.validate_store(store)?;
                let current = store.inventory_locked_until(self.deadline)?.facts();
                if current == *expected {
                    if Instant::now() >= self.deadline {
                        Err(busy())
                    } else {
                        Ok(())
                    }
                } else {
                    Err(changed())
                }
            }
            AppDataResetManagedCacheAdmissionState::Absent {
                publication,
                cache_stage,
            } => {
                publication.revalidate()?;
                publication.validate_cache_stage_absent(cache_stage)?;
                if let Some(container) = publication.container.as_ref()
                    && platform::open_existing_private_directory(
                        &container.file,
                        &publication.container_path,
                        STORE_DIRECTORY_NAME,
                    )?
                    .is_some()
                {
                    return Err(changed());
                }
                if Instant::now() >= self.deadline {
                    Err(busy())
                } else {
                    Ok(())
                }
            }
        }
    }
}

struct PendingTemp {
    name: String,
    file: Option<File>,
    identity: platform::Identity,
}

struct StageControl {
    name: &'static str,
    file: File,
    identity: platform::Identity,
}

struct ProvisioningStage {
    name: String,
    directory: File,
    identity: platform::Identity,
    controls: Vec<StageControl>,
}

impl ProvisioningStage {
    fn create_control(
        &mut self,
        path: &Path,
        name: &'static str,
        contents: &[u8],
    ) -> Result<(File, platform::Identity)> {
        let (file, identity) =
            platform::create_private_file_exclusive(&self.directory, path, name)?
                .ok_or_else(unsafe_object)?;
        let live = file.try_clone().map_err(|_| unavailable());
        self.controls.push(StageControl {
            name,
            file,
            identity,
        });
        let mut live = live?;
        live.write_all(contents).map_err(|_| unavailable())?;
        live.sync_all().map_err(|_| unavailable())?;
        platform::validate_named(
            &self.directory,
            name,
            &live,
            identity,
            platform::Kind::PrivateFile,
        )?;
        prove_control(&live, contents)?;
        Ok((live, identity))
    }

    fn cleanup(self, parent: &File) -> Result<()> {
        for control in self.controls.into_iter().rev() {
            platform::remove_retained_file(
                &self.directory,
                control.name,
                control.file,
                control.identity,
            )?;
        }
        platform::remove_retained_directory(parent, &self.name, self.directory, self.identity)?;
        platform::sync_directory(parent)
    }
}

impl PendingTemp {
    fn file(&self) -> Result<&File> {
        self.file.as_ref().ok_or_else(internal_state)
    }

    fn file_mut(&mut self) -> Result<&mut File> {
        self.file.as_mut().ok_or_else(internal_state)
    }

    fn cleanup(mut self, directory: &File) -> Result<()> {
        let file = self.file.take().ok_or_else(internal_state)?;
        platform::remove_retained_file(directory, &self.name, file, self.identity)?;
        platform::sync_directory(directory)
    }

    fn into_published_file(mut self) -> Result<File> {
        self.file.take().ok_or_else(internal_state)
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let unlocked = FileExt::unlock(&self.file).is_ok();
        if unlocked {
            self.store.writer_in_use.store(false, Ordering::Release);
        }
    }
}

impl PublicationDirectoryLock {
    fn acquire(directory: &File, identity: platform::Identity, deadline: Instant) -> Result<Self> {
        if Instant::now() >= deadline {
            return Err(busy());
        }
        {
            let mut in_use = PUBLICATION_LOCKS_IN_USE
                .lock()
                .map_err(|_| internal_state())?;
            if !in_use.insert(identity) {
                return Err(busy());
            }
        }
        let file = match directory.try_clone() {
            Ok(file) => file,
            Err(_) => {
                release_publication_identity(identity);
                return Err(unavailable());
            }
        };
        loop {
            if Instant::now() >= deadline {
                release_publication_identity(identity);
                return Err(busy());
            }
            match FileExt::try_lock(&file) {
                Ok(()) if Instant::now() >= deadline => {
                    if FileExt::unlock(&file).is_ok() {
                        release_publication_identity(identity);
                    }
                    return Err(busy());
                }
                Ok(()) => return Ok(Self { identity, file }),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(
                        LOCK_RETRY_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(TryLockError::WouldBlock) => {
                    release_publication_identity(identity);
                    return Err(busy());
                }
                Err(TryLockError::Error(_)) => {
                    release_publication_identity(identity);
                    return Err(unavailable());
                }
            }
        }
    }
}

impl Drop for PublicationDirectoryLock {
    fn drop(&mut self) {
        if FileExt::unlock(&self.file).is_ok() {
            release_publication_identity(self.identity);
        }
    }
}

fn release_publication_identity(identity: platform::Identity) {
    if let Ok(mut in_use) = PUBLICATION_LOCKS_IN_USE.lock() {
        in_use.remove(&identity);
    }
}

impl PublicationFence {
    fn acquire(
        conventional_container: &Path,
        access: ManagedCacheStoreAccess,
        deadline: Instant,
    ) -> Result<Self> {
        validate_container_configuration(conventional_container)?;
        let parent_path = conventional_container
            .parent()
            .ok_or_else(unsafe_container)?
            .to_path_buf();
        let (parent, parent_identity) = platform::open_container_parent(conventional_container)?;
        let parent_lock = PublicationDirectoryLock::acquire(&parent, parent_identity, deadline)?;
        platform::validate_path(
            &parent_path,
            &parent,
            parent_identity,
            platform::Kind::ContainerDirectory,
        )?;
        let container =
            platform::open_or_create_child_container(&parent, conventional_container, access)?
                .map(|(file, identity)| {
                    let lock = PublicationDirectoryLock::acquire(&file, identity, deadline)?;
                    Ok(PublicationContainer {
                        file,
                        identity,
                        lock,
                    })
                })
                .transpose()?;
        let fence = Self {
            container,
            parent_lock,
            parent_path,
            parent,
            parent_identity,
            container_path: conventional_container.to_path_buf(),
        };
        fence.revalidate()?;
        Ok(fence)
    }

    fn revalidate(&self) -> Result<()> {
        platform::validate_path(
            &self.parent_path,
            &self.parent,
            self.parent_identity,
            platform::Kind::ContainerDirectory,
        )?;
        platform::validate_retained(
            &self.parent_lock.file,
            self.parent_identity,
            platform::Kind::ContainerDirectory,
            false,
        )?;
        match &self.container {
            Some(container) => {
                platform::validate_named(
                    &self.parent,
                    "Dux",
                    &container.file,
                    container.identity,
                    platform::Kind::ContainerDirectory,
                )?;
                platform::validate_retained(
                    &container.lock.file,
                    container.identity,
                    platform::Kind::ContainerDirectory,
                    false,
                )
            }
            None => {
                if platform::open_existing_container(&self.parent, "Dux")?.is_some() {
                    Err(changed())
                } else {
                    Ok(())
                }
            }
        }
    }

    fn validate_store(&self, store: &ManagedCacheStore) -> Result<()> {
        self.revalidate()?;
        let Some(container) = self.container.as_ref() else {
            return Err(changed());
        };
        validate_same_filesystem(container.identity, store.inner.directory_identity)?;
        platform::validate_named(
            &container.file,
            STORE_DIRECTORY_NAME,
            &store.inner.directory,
            store.inner.directory_identity,
            platform::Kind::PrivateDirectory,
        )
    }

    fn validate_cache_stage_absent(&self, stage: &AppDataResetCacheStageName) -> Result<()> {
        let Some(container) = self.container.as_ref() else {
            return Ok(());
        };
        if platform::open_existing_private_directory(
            &container.file,
            &self.container_path,
            stage.as_str(),
        )?
        .is_some()
        {
            Err(changed())
        } else {
            Ok(())
        }
    }
}

impl ManagedCacheStore {
    /// Open the fixed marker-owned child beneath the conventional cache
    /// container. Read-only access never creates either directory.
    pub(crate) fn open(
        conventional_container: &Path,
        access: ManagedCacheStoreAccess,
    ) -> Result<Option<Self>> {
        let deadline = Instant::now().checked_add(LOCK_TIMEOUT).ok_or_else(|| {
            ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::InternalState)
        })?;
        Self::open_until(conventional_container, access, deadline)
    }

    pub(crate) fn open_until(
        conventional_container: &Path,
        access: ManagedCacheStoreAccess,
        deadline: Instant,
    ) -> Result<Option<Self>> {
        let publication = PublicationFence::acquire(conventional_container, access, deadline)?;
        let Some(store) = Self::open_under_publication_fence(&publication, access)? else {
            return Ok(None);
        };
        let lock = store.acquire_writer_lock_until(deadline)?;
        store.inventory_locked()?;
        drop(lock);
        Ok(Some(store))
    }

    fn open_under_publication_fence(
        publication: &PublicationFence,
        access: ManagedCacheStoreAccess,
    ) -> Result<Option<Self>> {
        publication.revalidate()?;
        let Some(container) = publication.container.as_ref() else {
            return Ok(None);
        };
        let path = publication.container_path.join(STORE_DIRECTORY_NAME);
        let store = match platform::open_existing_private_directory(
            &container.file,
            &publication.container_path,
            STORE_DIRECTORY_NAME,
        )? {
            Some(directory) => Self::from_open_directory(publication, path, directory, access)?,
            None if access == ManagedCacheStoreAccess::ReadOnly => return Ok(None),
            None => Self::provision(publication, path, access)?,
        };
        publication.validate_store(&store)?;
        Ok(Some(store))
    }

    fn provision(
        publication: &PublicationFence,
        path: PathBuf,
        access: ManagedCacheStoreAccess,
    ) -> Result<Self> {
        let container = publication.container.as_ref().ok_or_else(internal_state)?;
        for _ in 0..RANDOM_ATTEMPTS {
            let stage_name = random_stage_name()?;
            let Some(directory) = platform::create_private_directory_exclusive(
                &container.file,
                &publication.container_path,
                &stage_name,
            )?
            else {
                continue;
            };
            let stage_path = publication.container_path.join(&stage_name);
            let directory_identity =
                platform::identity(&directory, platform::Kind::PrivateDirectory)?;
            let mut stage = ProvisioningStage {
                name: stage_name,
                directory,
                identity: directory_identity,
                controls: Vec::with_capacity(2),
            };
            let prepared = (|| {
                let (marker, marker_identity) =
                    stage.create_control(&stage_path, MARKER_NAME, STORE_MARKER)?;
                let (writer_lock, writer_lock_identity) =
                    stage.create_control(&stage_path, WRITER_LOCK_NAME, WRITER_MARKER)?;
                platform::sync_directory(&stage.directory)?;
                publication.revalidate()?;
                Ok((marker, marker_identity, writer_lock, writer_lock_identity))
            })();
            let (marker, marker_identity, writer_lock, writer_lock_identity) = match prepared {
                Ok(prepared) => prepared,
                Err(error) => {
                    return if stage.cleanup(&container.file).is_ok() {
                        Err(error)
                    } else {
                        Err(unsafe_store())
                    };
                }
            };
            let published = platform::publish_directory_no_replace(
                &container.file,
                &stage.name,
                &stage.directory,
                stage.identity,
                STORE_DIRECTORY_NAME,
            );
            match published {
                Err(error) => {
                    return if stage.cleanup(&container.file).is_ok() {
                        Err(error)
                    } else {
                        Err(unsafe_store())
                    };
                }
                Ok(platform::Publication::Published) => {
                    platform::sync_directory(&container.file).map_err(|_| outcome_unknown())?;
                    platform::validate_named(
                        &container.file,
                        STORE_DIRECTORY_NAME,
                        &stage.directory,
                        stage.identity,
                        platform::Kind::PrivateDirectory,
                    )
                    .map_err(|_| outcome_unknown())?;
                    return Ok(Self {
                        inner: Arc::new(StoreInner {
                            parent_path: publication.parent_path.clone(),
                            parent: publication
                                .parent
                                .try_clone()
                                .map_err(|_| outcome_unknown())?,
                            parent_identity: publication.parent_identity,
                            container_path: publication.container_path.clone(),
                            container: container.file.try_clone().map_err(|_| outcome_unknown())?,
                            container_identity: container.identity,
                            path,
                            directory: stage.directory,
                            directory_identity: stage.identity,
                            marker,
                            marker_identity,
                            writer_lock,
                            writer_lock_identity,
                            writer_in_use: AtomicBool::new(false),
                        }),
                        access,
                    });
                }
                Ok(platform::Publication::Collision) => {
                    stage.cleanup(&container.file)?;
                    let directory = platform::open_existing_private_directory(
                        &container.file,
                        &publication.container_path,
                        STORE_DIRECTORY_NAME,
                    )?
                    .ok_or_else(unsafe_store)?;
                    return Self::from_open_directory(publication, path, directory, access);
                }
            }
        }
        Err(unavailable())
    }

    fn from_open_directory(
        publication: &PublicationFence,
        path: PathBuf,
        directory: File,
        access: ManagedCacheStoreAccess,
    ) -> Result<Self> {
        let container = publication.container.as_ref().ok_or_else(internal_state)?;
        let directory_identity = platform::identity(&directory, platform::Kind::PrivateDirectory)?;
        platform::validate_retained(
            &directory,
            directory_identity,
            platform::Kind::PrivateDirectory,
            false,
        )?;
        let (marker, marker_identity) =
            open_control(&directory, &path, MARKER_NAME, STORE_MARKER, false)?
                .ok_or_else(unrecognized)?;
        let (writer_lock, writer_lock_identity) =
            open_control(&directory, &path, WRITER_LOCK_NAME, WRITER_MARKER, true)?
                .ok_or_else(unsafe_object)?;
        Ok(Self {
            inner: Arc::new(StoreInner {
                parent_path: publication.parent_path.clone(),
                parent: publication.parent.try_clone().map_err(|_| unavailable())?,
                parent_identity: publication.parent_identity,
                container_path: publication.container_path.clone(),
                container: container.file.try_clone().map_err(|_| unavailable())?,
                container_identity: container.identity,
                path,
                directory,
                directory_identity,
                marker,
                marker_identity,
                writer_lock,
                writer_lock_identity,
                writer_in_use: AtomicBool::new(false),
            }),
            access,
        })
    }

    pub(crate) fn load(
        &self,
        canonical_root: &Path,
        config: &CachedScanConfig,
    ) -> Result<Option<ManagedCacheDocument>> {
        let key = managed_cache_entry_key(canonical_root, config).map_err(|_| corrupt())?;
        let name = final_name(&key);
        let lock = self.acquire_writer_lock(LOCK_TIMEOUT)?;
        let inventory = self.inventory_locked()?;
        let Some(object) = inventory
            .objects
            .iter()
            .find(|object| object.kind == InventoryKind::Entry && object.name == name)
        else {
            drop(lock);
            return Ok(None);
        };
        platform::validate_named(
            &self.inner.directory,
            &object.name,
            &object.file,
            object.facts.identity,
            platform::Kind::PrivateFile,
        )?;
        let document = decode_validated_entry(&object.file, object.facts, canonical_root, config)?;
        platform::validate_named(
            &self.inner.directory,
            &object.name,
            &object.file,
            object.facts.identity,
            platform::Kind::PrivateFile,
        )?;
        drop(lock);
        Ok(Some(document))
    }

    pub(crate) fn save(
        &self,
        canonical_root: &Path,
        config: &CachedScanConfig,
        metadata: &CacheMetadata,
        tree: &DiskTree,
    ) -> std::result::Result<(), ManagedCacheSaveError> {
        if self.access != ManagedCacheStoreAccess::ReadWrite {
            return Err(ManagedCacheSaveError::BeforePublication(
                ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::ReadOnly),
            ));
        }
        let key = managed_cache_entry_key(canonical_root, config)
            .map_err(|_| ManagedCacheSaveError::BeforePublication(corrupt()))?;
        let bytes = encode_managed_cache(canonical_root, config, metadata, tree)
            .map_err(|_| ManagedCacheSaveError::BeforePublication(corrupt()))?;
        if u64::try_from(bytes.len()).ok().is_none_or(|length| {
            length > MAX_MANAGED_CACHE_FILE_BYTES
                || length < u64::try_from(MANAGED_CACHE_HEADER_BYTES).unwrap_or(u64::MAX)
        }) {
            return Err(ManagedCacheSaveError::BeforePublication(budget()));
        }
        let lock = self
            .acquire_writer_lock(LOCK_TIMEOUT)
            .map_err(ManagedCacheSaveError::BeforePublication)?;
        let final_name = final_name(&key);
        let inventory = self
            .inventory_locked()
            .map_err(ManagedCacheSaveError::BeforePublication)?;
        inventory
            .reserve_save_capacity(&final_name)
            .map_err(ManagedCacheSaveError::BeforePublication)?;
        let mut temp = self
            .create_temp(&key)
            .map_err(ManagedCacheSaveError::BeforePublication)?;
        let prepublication = (|| -> Result<()> {
            if take_test_fault(TEST_FAULT_PRE_PUBLICATION) {
                return Err(unavailable());
            }
            let file = temp.file_mut()?;
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|_| unavailable())?;
            let file = temp.file()?;
            let temp_facts = ObjectFacts {
                identity: temp.identity,
                change: platform::change_token(file)?,
                usage: ManagedCacheStorageUsage::from_file(file)?,
            };
            if temp_facts.usage.logical_bytes != u64::try_from(bytes.len()).unwrap_or(u64::MAX) {
                return Err(unsafe_object());
            }
            platform::validate_named(
                &self.inner.directory,
                &temp.name,
                file,
                temp.identity,
                platform::Kind::PrivateFile,
            )?;
            decode_validated_entry(file, temp_facts, canonical_root, config)?;
            let publication = platform::publish_file_replace(
                &self.inner.directory,
                &temp.name,
                file,
                temp.identity,
                &final_name,
            )?;
            if publication != platform::Publication::Published {
                return Err(internal_state());
            }
            Ok(())
        })();
        if let Err(error) = prepublication {
            let cleaned = temp.cleanup(&self.inner.directory).is_ok();
            drop(lock);
            return Err(ManagedCacheSaveError::BeforePublication(if cleaned {
                error
            } else {
                unsafe_object()
            }));
        }
        let published_identity = temp.identity;
        let published = temp
            .into_published_file()
            .map_err(|_| ManagedCacheSaveError::OutcomeUnknown)?;
        if take_test_fault(TEST_FAULT_POST_PUBLICATION) {
            drop(lock);
            return Err(ManagedCacheSaveError::OutcomeUnknown);
        }
        platform::sync_directory(&self.inner.directory)
            .map_err(|_| ManagedCacheSaveError::OutcomeUnknown)?;
        platform::validate_named(
            &self.inner.directory,
            &final_name,
            &published,
            published_identity,
            platform::Kind::PrivateFile,
        )
        .map_err(|_| ManagedCacheSaveError::OutcomeUnknown)?;
        self.inventory_locked()
            .map_err(|_| ManagedCacheSaveError::OutcomeUnknown)?;
        drop(lock);
        Ok(())
    }

    pub(crate) fn footprint(&self) -> Result<ManagedCacheStoreFootprint> {
        let lock = self.acquire_writer_lock(LOCK_TIMEOUT)?;
        let footprint = self.inventory_locked()?.footprint()?;
        drop(lock);
        Ok(footprint)
    }

    /// Retain the present store's writer lock and complete bounded inventory
    /// during one reset callback without exposing either owned capability.
    #[cfg(test)]
    pub(crate) fn with_app_data_reset_writer_admission<T>(
        &self,
        timeout: Duration,
        admitted: impl for<'scope> FnOnce(AppDataResetManagedCacheAdmission<'scope>) -> T,
    ) -> Result<T> {
        let transaction = AppDataResetTransaction::for_test("00112233445566778899aabbccddeeff")
            .ok_or_else(internal_state)?;
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::InternalState)
        })?;
        let publication = PublicationFence::acquire(
            &self.inner.container_path,
            ManagedCacheStoreAccess::ReadOnly,
            deadline,
        )?;
        publication.validate_store(self)?;
        let writer_lock = self.acquire_writer_lock_until_mode(deadline, true)?;
        self.with_retained_app_data_reset_writer(
            &publication,
            transaction.cache_stage(),
            writer_lock,
            deadline,
            admitted,
        )
    }

    /// Test-only probe for the child writer independent of publication fences.
    #[cfg(test)]
    pub(crate) fn with_test_child_writer_until<T>(
        &self,
        deadline: Instant,
        operation: impl FnOnce() -> T,
    ) -> Result<T> {
        let writer = self.acquire_writer_lock_until(deadline)?;
        let result = operation();
        drop(writer);
        Ok(result)
    }

    /// Test-only probe for publication exclusion independent of the child
    /// writer.
    #[cfg(test)]
    pub(crate) fn with_test_publication_fence_until<T>(
        conventional_container: &Path,
        deadline: Instant,
        operation: impl FnOnce() -> T,
    ) -> Result<T> {
        let publication = PublicationFence::acquire(
            conventional_container,
            ManagedCacheStoreAccess::ReadOnly,
            deadline,
        )?;
        publication.revalidate()?;
        let result = operation();
        drop(publication);
        Ok(result)
    }

    /// Inspect the exact conventional namespace under retained publication
    /// fences without provisioning it, then lend either a present or absent
    /// witness to one higher-ranked callback.
    pub(crate) fn with_app_data_reset_admission_until<T>(
        conventional_container: &Path,
        transaction: &AppDataResetTransaction,
        deadline: Instant,
        admitted: impl for<'scope> FnOnce(AppDataResetManagedCacheAdmission<'scope>) -> T,
    ) -> Result<T> {
        let publication = PublicationFence::acquire(
            conventional_container,
            ManagedCacheStoreAccess::ReadOnly,
            deadline,
        )?;
        let Some(store) =
            Self::open_under_publication_fence(&publication, ManagedCacheStoreAccess::ReadOnly)?
        else {
            let admission = AppDataResetManagedCacheAdmission {
                state: AppDataResetManagedCacheAdmissionState::Absent {
                    publication: &publication,
                    cache_stage: transaction.cache_stage(),
                },
                deadline,
            };
            admission.revalidate()?;
            if Instant::now() >= deadline {
                return Err(busy());
            }
            return Ok(admitted(admission));
        };
        let writer_lock = store.acquire_writer_lock_until(deadline)?;
        store.with_retained_app_data_reset_writer(
            &publication,
            transaction.cache_stage(),
            writer_lock,
            deadline,
            admitted,
        )
    }

    fn with_retained_app_data_reset_writer<T>(
        &self,
        publication: &PublicationFence,
        cache_stage: &AppDataResetCacheStageName,
        writer_lock: WriterLock,
        deadline: Instant,
        admitted: impl for<'scope> FnOnce(AppDataResetManagedCacheAdmission<'scope>) -> T,
    ) -> Result<T> {
        let expected = self.inventory_locked_until(deadline)?.facts();
        let admission = AppDataResetManagedCacheAdmission {
            state: AppDataResetManagedCacheAdmissionState::Present {
                store: self,
                expected,
                _writer_lock: &writer_lock,
                publication,
                cache_stage,
            },
            deadline,
        };
        admission.revalidate()?;
        if Instant::now() >= deadline {
            return Err(busy());
        }
        Ok(admitted(admission))
    }

    pub(crate) fn prepare_clear(&self) -> Result<Option<ManagedCacheClearSnapshot>> {
        if self.access != ManagedCacheStoreAccess::ReadWrite {
            return Err(ManagedCacheStoreError::new(
                ManagedCacheStoreErrorKind::ReadOnly,
            ));
        }
        let lock = self.acquire_writer_lock(LOCK_TIMEOUT)?;
        let inventory = self.inventory_locked()?;
        let footprint = inventory.footprint()?;
        if footprint.entry_count == 0 && footprint.temporary_count == 0 {
            drop(lock);
            return Ok(None);
        }
        let snapshot = ManagedCacheClearSnapshot {
            store: Arc::clone(&self.inner),
            facts: inventory.facts(),
            footprint,
        };
        drop(lock);
        Ok(Some(snapshot))
    }

    pub(crate) fn clear(
        &self,
        snapshot: ManagedCacheClearSnapshot,
    ) -> std::result::Result<ManagedCacheClearResult, ManagedCacheClearError> {
        if self.access != ManagedCacheStoreAccess::ReadWrite {
            return Err(ManagedCacheClearError::BeforeEffect(
                ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::ReadOnly),
            ));
        }
        if !Arc::ptr_eq(&snapshot.store, &self.inner) {
            return Err(ManagedCacheClearError::BeforeEffect(
                ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::InvalidConfiguration),
            ));
        }
        let lock = self
            .acquire_writer_lock(LOCK_TIMEOUT)
            .map_err(ManagedCacheClearError::BeforeEffect)?;
        let mut current = self
            .inventory_locked()
            .map_err(ManagedCacheClearError::BeforeEffect)?;
        if current.facts() != snapshot.facts {
            return Err(ManagedCacheClearError::BeforeEffect(
                ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::ChangedSinceSnapshot),
            ));
        }
        let cleared_usage = current
            .clearable_usage()
            .map_err(ManagedCacheClearError::BeforeEffect)?;
        let mut mutated = false;
        for object in current.objects.drain(..) {
            let removal = platform::remove_retained_file(
                &self.inner.directory,
                &object.name,
                object.file,
                object.facts.identity,
            );
            if removal.is_err() {
                return if mutated {
                    Err(ManagedCacheClearError::OutcomeUnknown)
                } else {
                    Err(ManagedCacheClearError::BeforeEffect(unsafe_object()))
                };
            }
            mutated = true;
        }
        if platform::sync_directory(&self.inner.directory).is_err() {
            return Err(ManagedCacheClearError::OutcomeUnknown);
        }
        let after = self
            .inventory_locked()
            .map_err(|_| ManagedCacheClearError::OutcomeUnknown)?;
        if !after.objects.is_empty() {
            return Err(ManagedCacheClearError::OutcomeUnknown);
        }
        drop(lock);
        Ok(ManagedCacheClearResult {
            cleared_entries: snapshot.footprint.entry_count,
            cleared_temporary: snapshot.footprint.temporary_count,
            cleared_usage,
        })
    }

    fn create_temp(&self, key: &ManagedCacheEntryKey) -> Result<PendingTemp> {
        for _ in 0..RANDOM_ATTEMPTS {
            let name = random_temp_name(key)?;
            if let Some((file, identity)) = platform::create_private_file_exclusive(
                &self.inner.directory,
                &self.inner.path,
                &name,
            )? {
                return Ok(PendingTemp {
                    name,
                    file: Some(file),
                    identity,
                });
            }
        }
        Err(unavailable())
    }

    fn acquire_writer_lock(&self, timeout: Duration) -> Result<WriterLock> {
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::InvalidConfiguration)
        })?;
        self.acquire_writer_lock_until_mode(deadline, true)
    }

    fn acquire_writer_lock_until(&self, deadline: Instant) -> Result<WriterLock> {
        self.acquire_writer_lock_until_mode(deadline, false)
    }

    fn acquire_writer_lock_until_mode(
        &self,
        deadline: Instant,
        allow_expired_initial_try: bool,
    ) -> Result<WriterLock> {
        if !allow_expired_initial_try && Instant::now() >= deadline {
            return Err(ManagedCacheStoreError::new(
                ManagedCacheStoreErrorKind::Busy,
            ));
        }
        if self
            .inner
            .writer_in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ManagedCacheStoreError::new(
                ManagedCacheStoreErrorKind::Busy,
            ));
        }
        if let Err(error) = self.validate_controls() {
            self.inner.writer_in_use.store(false, Ordering::Release);
            return Err(error);
        }
        let file = match self.inner.writer_lock.try_clone() {
            Ok(file) => file,
            Err(_) => {
                self.inner.writer_in_use.store(false, Ordering::Release);
                return Err(unavailable());
            }
        };
        let mut first_attempt = true;
        loop {
            if Instant::now() >= deadline && !(allow_expired_initial_try && first_attempt) {
                self.inner.writer_in_use.store(false, Ordering::Release);
                return Err(ManagedCacheStoreError::new(
                    ManagedCacheStoreErrorKind::Busy,
                ));
            }
            first_attempt = false;
            match FileExt::try_lock(&file) {
                Ok(()) if !allow_expired_initial_try && Instant::now() >= deadline => {
                    let unlocked = FileExt::unlock(&file).is_ok();
                    if unlocked {
                        self.inner.writer_in_use.store(false, Ordering::Release);
                    }
                    return Err(ManagedCacheStoreError::new(
                        ManagedCacheStoreErrorKind::Busy,
                    ));
                }
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(
                        LOCK_RETRY_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(TryLockError::WouldBlock) => {
                    self.inner.writer_in_use.store(false, Ordering::Release);
                    return Err(ManagedCacheStoreError::new(
                        ManagedCacheStoreErrorKind::Busy,
                    ));
                }
                Err(TryLockError::Error(_)) => {
                    self.inner.writer_in_use.store(false, Ordering::Release);
                    return Err(unavailable());
                }
            }
        }
        if let Err(error) = self.validate_controls() {
            let unlocked = FileExt::unlock(&file).is_ok();
            if unlocked {
                self.inner.writer_in_use.store(false, Ordering::Release);
            }
            return Err(error);
        }
        if !allow_expired_initial_try && Instant::now() >= deadline {
            let unlocked = FileExt::unlock(&file).is_ok();
            if unlocked {
                self.inner.writer_in_use.store(false, Ordering::Release);
            }
            return Err(ManagedCacheStoreError::new(
                ManagedCacheStoreErrorKind::Busy,
            ));
        }
        Ok(WriterLock {
            store: Arc::clone(&self.inner),
            file,
        })
    }

    fn validate_controls(&self) -> Result<()> {
        platform::validate_path(
            &self.inner.parent_path,
            &self.inner.parent,
            self.inner.parent_identity,
            platform::Kind::ContainerDirectory,
        )?;
        platform::validate_named(
            &self.inner.parent,
            "Dux",
            &self.inner.container,
            self.inner.container_identity,
            platform::Kind::ContainerDirectory,
        )?;
        platform::validate_path(
            &self.inner.container_path,
            &self.inner.container,
            self.inner.container_identity,
            platform::Kind::ContainerDirectory,
        )?;
        validate_same_filesystem(self.inner.container_identity, self.inner.directory_identity)?;
        platform::validate_retained(
            &self.inner.container,
            self.inner.container_identity,
            platform::Kind::ContainerDirectory,
            false,
        )?;
        platform::validate_named(
            &self.inner.container,
            STORE_DIRECTORY_NAME,
            &self.inner.directory,
            self.inner.directory_identity,
            platform::Kind::PrivateDirectory,
        )?;
        for (name, file, identity, expected) in [
            (
                MARKER_NAME,
                &self.inner.marker,
                self.inner.marker_identity,
                STORE_MARKER.as_slice(),
            ),
            (
                WRITER_LOCK_NAME,
                &self.inner.writer_lock,
                self.inner.writer_lock_identity,
                WRITER_MARKER.as_slice(),
            ),
        ] {
            platform::validate_named(
                &self.inner.directory,
                name,
                file,
                identity,
                platform::Kind::PrivateFile,
            )?;
            prove_control(file, expected)?;
        }
        Ok(())
    }

    fn inventory_locked(&self) -> Result<ManagedCacheInventory> {
        let deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or_else(budget)?;
        self.inventory_locked_until(deadline)
    }

    fn inventory_locked_until(&self, outer_deadline: Instant) -> Result<ManagedCacheInventory> {
        if Instant::now() >= outer_deadline {
            return Err(busy());
        }
        std::thread::sleep(take_test_inventory_delay());
        self.validate_controls()?;
        if Instant::now() >= outer_deadline {
            return Err(busy());
        }
        let inventory_deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or_else(budget)?
            .min(outer_deadline);
        let names = platform::inventory(
            &self.inner.directory,
            RECOVERY_MAX_NON_CONTROL_OBJECTS + 3,
            MAX_INVENTORY_NAME_BYTES,
            inventory_deadline,
        );
        let mut names = match names {
            Err(_) if Instant::now() >= outer_deadline => return Err(busy()),
            result => result?,
        };
        names.sort_unstable();
        if names
            .iter()
            .filter(|name| name.as_str() == MARKER_NAME)
            .count()
            != 1
            || names
                .iter()
                .filter(|name| name.as_str() == WRITER_LOCK_NAME)
                .count()
                != 1
        {
            return Err(unrecognized());
        }
        let controls = ManagedCacheStorageUsage::from_file(&self.inner.marker)?.checked_add(
            ManagedCacheStorageUsage::from_file(&self.inner.writer_lock)?,
        )?;
        let mut entries = ManagedCacheStorageUsage::default();
        let mut temporary = ManagedCacheStorageUsage::default();
        let mut objects = Vec::new();
        let mut temporary_count = 0_usize;
        for name in names {
            if Instant::now() >= inventory_deadline {
                return if Instant::now() >= outer_deadline {
                    Err(busy())
                } else {
                    Err(budget())
                };
            }
            if name == MARKER_NAME || name == WRITER_LOCK_NAME {
                continue;
            }
            if objects.len() >= RECOVERY_MAX_NON_CONTROL_OBJECTS {
                return Err(budget());
            }
            let kind = if is_final_name(&name) {
                InventoryKind::Entry
            } else if is_temp_name(&name) {
                temporary_count = temporary_count.checked_add(1).ok_or_else(budget)?;
                if temporary_count > RECOVERY_MAX_TEMPORARY_OBJECTS {
                    return Err(budget());
                }
                InventoryKind::Temporary
            } else {
                return Err(unsafe_object());
            };
            let (file, identity) = platform::open_named_private_file(
                &self.inner.directory,
                &self.inner.path,
                &name,
                false,
            )?
            .ok_or_else(unsafe_object)?;
            platform::validate_named(
                &self.inner.directory,
                &name,
                &file,
                identity,
                platform::Kind::PrivateFile,
            )?;
            let usage = ManagedCacheStorageUsage::from_file(&file)?;
            if usage.logical_bytes > MAX_MANAGED_CACHE_FILE_BYTES
                || (kind == InventoryKind::Entry
                    && usage.logical_bytes
                        < u64::try_from(MANAGED_CACHE_HEADER_BYTES).unwrap_or(u64::MAX))
            {
                return Err(budget());
            }
            match kind {
                InventoryKind::Entry => entries = entries.checked_add(usage)?,
                InventoryKind::Temporary => temporary = temporary.checked_add(usage)?,
            }
            let change = platform::change_token(&file)?;
            objects.push(InventoryObject {
                name,
                kind,
                file,
                facts: ObjectFacts {
                    identity,
                    change,
                    usage,
                },
            });
        }
        platform::validate_retained(
            &self.inner.directory,
            self.inner.directory_identity,
            platform::Kind::PrivateDirectory,
            false,
        )?;
        if Instant::now() >= inventory_deadline {
            return if Instant::now() >= outer_deadline {
                Err(busy())
            } else {
                Err(budget())
            };
        }
        let total = controls.checked_add(entries)?.checked_add(temporary)?;
        Ok(ManagedCacheInventory {
            controls,
            entries,
            temporary,
            total,
            objects,
        })
    }
}

fn validate_container_configuration(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path.file_name().and_then(|name| name.to_str()) != Some("Dux")
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(ManagedCacheStoreError::new(
            ManagedCacheStoreErrorKind::InvalidConfiguration,
        ));
    }
    Ok(())
}

fn validate_same_filesystem(
    container: platform::Identity,
    child: platform::Identity,
) -> Result<()> {
    if platform::same_filesystem(container, child) {
        Ok(())
    } else {
        Err(unsafe_store())
    }
}

fn open_control(
    directory: &File,
    directory_path: &Path,
    name: &str,
    contents: &[u8],
    writable: bool,
) -> Result<Option<(File, platform::Identity)>> {
    let Some((file, identity)) =
        platform::open_named_private_file(directory, directory_path, name, writable)?
    else {
        return Ok(None);
    };
    platform::validate_named(
        directory,
        name,
        &file,
        identity,
        platform::Kind::PrivateFile,
    )?;
    prove_control(&file, contents)?;
    Ok(Some((file, identity)))
}

fn prove_control(file: &File, expected: &[u8]) -> Result<()> {
    let usage = ManagedCacheStorageUsage::from_file(file)?;
    if usage.logical_bytes != u64::try_from(expected.len()).unwrap_or(u64::MAX) {
        return Err(unsafe_object());
    }
    let mut actual = vec![0_u8; expected.len()];
    read_exact_at(file, &mut actual, 0).map_err(|_| unsafe_object())?;
    if actual != expected {
        return Err(unrecognized());
    }
    Ok(())
}

fn decode_validated_entry(
    file: &File,
    expected: ObjectFacts,
    canonical_root: &Path,
    config: &CachedScanConfig,
) -> Result<ManagedCacheDocument> {
    platform::validate_retained(file, expected.identity, platform::Kind::PrivateFile, true)?;
    let usage = ManagedCacheStorageUsage::from_file(file)?;
    if usage != expected.usage
        || usage.logical_bytes > MAX_MANAGED_CACHE_FILE_BYTES
        || usage.logical_bytes < u64::try_from(MANAGED_CACHE_HEADER_BYTES).unwrap_or(u64::MAX)
    {
        return Err(corrupt());
    }
    let mut header = [0_u8; MANAGED_CACHE_HEADER_BYTES];
    read_exact_at(file, &mut header, 0).map_err(|_| corrupt())?;
    let preflight =
        preflight_managed_cache_header(&header, usage.logical_bytes).map_err(|_| corrupt())?;
    if u64::try_from(preflight.file_bytes()).ok() != Some(usage.logical_bytes) {
        return Err(corrupt());
    }
    let length = usize::try_from(usage.logical_bytes).map_err(|_| budget())?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| budget())?;
    bytes.resize(length, 0);
    read_exact_at(file, &mut bytes, 0).map_err(|_| corrupt())?;
    if ManagedCacheStorageUsage::from_file(file)? != usage
        || platform::change_token(file)? != expected.change
    {
        return Err(corrupt());
    }
    decode_managed_cache(&bytes, canonical_root, config).map_err(|_| corrupt())
}

fn final_name(key: &ManagedCacheEntryKey) -> String {
    let mut name = key.to_lower_hex();
    name.push_str(FINAL_SUFFIX);
    name
}

fn is_final_name(name: &str) -> bool {
    let Some(hex) = name.strip_suffix(FINAL_SUFFIX) else {
        return false;
    };
    is_lower_hex(hex, KEY_HEX_BYTES)
}

fn random_temp_name(key: &ManagedCacheEntryKey) -> Result<String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| unavailable())?;
    let final_name = final_name(key);
    let mut name = String::with_capacity(128);
    name.push('.');
    name.push_str(&final_name);
    name.push('.');
    name.push_str(&std::process::id().to_string());
    name.push('.');
    push_lower_hex(&mut name, &random);
    name.push_str(".tmp");
    Ok(name)
}

fn is_temp_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let Some(rest) = rest.strip_suffix(".tmp") else {
        return false;
    };
    let mut fields = rest.rsplitn(3, '.');
    let (Some(random), Some(pid), Some(final_name)) = (fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    is_final_name(final_name)
        && is_lower_hex(random, RANDOM_HEX_BYTES)
        && !pid.is_empty()
        && pid.len() <= 10
        && !(pid.len() > 1 && pid.starts_with('0'))
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && pid.parse::<u32>().is_ok_and(|pid| pid > 0)
}

fn random_stage_name() -> Result<String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| unavailable())?;
    let mut name = String::with_capacity(STAGE_PREFIX.len() + RANDOM_HEX_BYTES);
    name.push_str(STAGE_PREFIX);
    push_lower_hex(&mut name, &random);
    Ok(name)
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn push_lower_hex(output: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

fn corrupt() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::CorruptData)
}

fn budget() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::BudgetExceeded)
}

fn unavailable() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::Unavailable)
}

fn busy() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::Busy)
}

fn changed() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::ChangedSinceSnapshot)
}

fn unsafe_container() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::UnsafeContainer)
}

fn unsafe_store() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::UnsafeStore)
}

fn unsafe_object() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::UnsafeObject)
}

fn unrecognized() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::UnrecognizedStore)
}

fn internal_state() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::InternalState)
}

fn outcome_unknown() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::OutcomeUnknown)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buffer, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buffer, offset).and_then(|read| {
        if read == buffer.len() {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "short managed-cache read",
            ))
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod platform {
    use std::ffi::CString;
    use std::fs::File;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::path::Path;
    use std::time::Instant;

    use nix::dir::Dir;
    use nix::errno::Errno;
    use nix::fcntl::{OFlag, open, openat};
    use nix::sys::stat::{Mode, SFlag, fchmod, fstat, mkdirat};
    use nix::unistd::geteuid;

    use super::{
        ManagedCacheStoreAccess, ManagedCacheStoreError, ManagedCacheStoreErrorKind, Result,
        budget, unavailable, unsafe_object, unsafe_store,
    };

    const DIRECTORY_MODE: Mode = Mode::S_IRWXU;
    const FILE_MODE: Mode = Mode::S_IRUSR.union(Mode::S_IWUSR);

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub(super) struct Identity {
        device: u64,
        inode: u64,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub(super) struct ChangeToken {
        change_seconds: i64,
        change_nanoseconds: i64,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Kind {
        ContainerDirectory,
        PrivateDirectory,
        PrivateFile,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Publication {
        Published,
        Collision,
    }

    pub(super) fn open_container_parent(path: &Path) -> Result<(File, Identity)> {
        let parent_path = path.parent().ok_or_else(unsafe_container)?;
        let parent = open(
            parent_path,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(map_container_error)?;
        validate_publication_parent(&parent)?;
        let identity = identity(&parent, Kind::ContainerDirectory)?;
        validate_retained(&parent, identity, Kind::ContainerDirectory, false)?;
        Ok((parent, identity))
    }

    pub(super) fn open_or_create_child_container(
        parent: &File,
        path: &Path,
        access: ManagedCacheStoreAccess,
    ) -> Result<Option<(File, Identity)>> {
        let name = path.file_name().ok_or_else(unsafe_container)?;
        match openat(
            parent,
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(descriptor) => {
                let directory = File::from(descriptor);
                let identity = identity(&directory, Kind::ContainerDirectory)?;
                validate_retained(&directory, identity, Kind::ContainerDirectory, false)?;
                Ok(Some((directory, identity)))
            }
            Err(Errno::ENOENT) if access == ManagedCacheStoreAccess::ReadOnly => Ok(None),
            Err(Errno::ENOENT) => {
                let created = match mkdirat(parent, name, DIRECTORY_MODE) {
                    Ok(()) => true,
                    Err(Errno::EEXIST) => false,
                    Err(error) => return Err(map_container_error(error)),
                };
                let directory = openat(
                    parent,
                    name,
                    OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
                    Mode::empty(),
                )
                .map(File::from)
                .map_err(map_container_error)?;
                let kind = if created {
                    fchmod(&directory, DIRECTORY_MODE).map_err(|_| unavailable())?;
                    Kind::PrivateDirectory
                } else {
                    Kind::ContainerDirectory
                };
                let identity = identity(&directory, kind)?;
                validate_retained(&directory, identity, kind, false)?;
                Ok(Some((directory, identity)))
            }
            Err(error) => Err(map_container_error(error)),
        }
    }

    pub(super) fn open_existing_container(parent: &File, name: &str) -> Result<Option<File>> {
        match openat(
            parent,
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(descriptor) => {
                let directory = File::from(descriptor);
                let object_identity = identity(&directory, Kind::ContainerDirectory)?;
                validate_retained(&directory, object_identity, Kind::ContainerDirectory, false)?;
                Ok(Some(directory))
            }
            Err(Errno::ENOENT) => Ok(None),
            Err(Errno::ELOOP | Errno::ENOTDIR) => Err(unsafe_container()),
            Err(_) => Err(unavailable()),
        }
    }

    pub(super) fn validate_path(
        path: &Path,
        retained: &File,
        expected: Identity,
        kind: Kind,
    ) -> Result<()> {
        let flags = match kind {
            Kind::ContainerDirectory | Kind::PrivateDirectory => {
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
            }
            Kind::PrivateFile => {
                OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
            }
        };
        let opened = open(path, flags, Mode::empty())
            .map(File::from)
            .map_err(|_| unsafe_for(kind))?;
        validate_retained(retained, expected, kind, matches!(kind, Kind::PrivateFile))?;
        validate_retained(&opened, expected, kind, matches!(kind, Kind::PrivateFile))
    }

    pub(super) fn open_existing_private_directory(
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
                let object_identity = identity(&directory, Kind::PrivateDirectory)?;
                validate_retained(&directory, object_identity, Kind::PrivateDirectory, false)?;
                Ok(Some(directory))
            }
            Err(Errno::ENOENT) => Ok(None),
            Err(Errno::ELOOP | Errno::ENOTDIR) => Err(unsafe_store()),
            Err(_) => Err(unavailable()),
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
            Err(_) => return Err(unavailable()),
        }
        let directory = openat(
            parent,
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| unavailable())?;
        fchmod(&directory, DIRECTORY_MODE).map_err(|_| unavailable())?;
        let object_identity = identity(&directory, Kind::PrivateDirectory)?;
        validate_retained(&directory, object_identity, Kind::PrivateDirectory, false)?;
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
                fchmod(&file, FILE_MODE).map_err(|_| unavailable())?;
                let object_identity = identity(&file, Kind::PrivateFile)?;
                validate_retained(&file, object_identity, Kind::PrivateFile, true)?;
                Ok(Some((file, object_identity)))
            }
            Err(Errno::EEXIST) => Ok(None),
            Err(Errno::ELOOP | Errno::EMLINK | Errno::EISDIR | Errno::ENOTDIR) => {
                Err(unsafe_object())
            }
            Err(_) => Err(unavailable()),
        }
    }

    pub(super) fn open_named_private_file(
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
                let object_identity = identity(&file, Kind::PrivateFile)?;
                validate_retained(&file, object_identity, Kind::PrivateFile, true)?;
                Ok(Some((file, object_identity)))
            }
            Err(Errno::ENOENT) => Ok(None),
            Err(Errno::ELOOP | Errno::EMLINK | Errno::EISDIR | Errno::ENOTDIR) => {
                Err(unsafe_object())
            }
            Err(_) => Err(unavailable()),
        }
    }

    pub(super) fn identity(file: &File, kind: Kind) -> Result<Identity> {
        let status = fstat(file).map_err(|_| unavailable())?;
        let expected_type = match kind {
            Kind::ContainerDirectory | Kind::PrivateDirectory => SFlag::S_IFDIR,
            Kind::PrivateFile => SFlag::S_IFREG,
        };
        if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != expected_type {
            return Err(unsafe_for(kind));
        }
        Ok(Identity {
            device: status.st_dev as u64,
            inode: status.st_ino as u64,
        })
    }

    pub(super) const fn same_filesystem(left: Identity, right: Identity) -> bool {
        left.device == right.device
    }

    #[cfg(test)]
    pub(super) const fn different_filesystem_identity(identity: Identity) -> Identity {
        Identity {
            device: identity.device.wrapping_add(1),
            inode: identity.inode,
        }
    }

    pub(super) fn change_token(file: &File) -> Result<ChangeToken> {
        let status = fstat(file).map_err(|_| unavailable())?;
        Ok(ChangeToken {
            change_seconds: status.st_ctime,
            change_nanoseconds: status.st_ctime_nsec,
        })
    }

    pub(super) fn file_usage(file: &File) -> Result<(u64, u64)> {
        let status = fstat(file).map_err(|_| unavailable())?;
        let logical_bytes = u64::try_from(status.st_size).map_err(|_| unsafe_object())?;
        let blocks = u64::try_from(status.st_blocks).map_err(|_| unsafe_object())?;
        let allocated_bytes = blocks.checked_mul(512).ok_or_else(unsafe_object)?;
        Ok((logical_bytes, allocated_bytes))
    }

    pub(super) fn validate_retained(
        file: &File,
        expected: Identity,
        kind: Kind,
        require_one_link: bool,
    ) -> Result<()> {
        let status = fstat(file).map_err(|_| unavailable())?;
        let actual = identity(file, kind)?;
        let mode = status.st_mode & 0o7777;
        let mode_ok = match kind {
            Kind::ContainerDirectory => mode & 0o022 == 0,
            Kind::PrivateDirectory => mode == 0o700,
            Kind::PrivateFile => mode == 0o600,
        };
        if actual != expected
            || status.st_uid != geteuid().as_raw()
            || !mode_ok
            || (require_one_link && status.st_nlink != 1)
        {
            return Err(unsafe_for(kind));
        }
        match kind {
            Kind::ContainerDirectory => reject_granting_acl(file)?,
            Kind::PrivateDirectory | Kind::PrivateFile => reject_extended_acl(file, kind)?,
        }
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
            Kind::ContainerDirectory | Kind::PrivateDirectory => {
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
            }
            Kind::PrivateFile => {
                OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
            }
        };
        let opened = openat(directory, name, flags, Mode::empty())
            .map(File::from)
            .map_err(|_| unsafe_for(kind))?;
        validate_retained(retained, expected, kind, matches!(kind, Kind::PrivateFile))?;
        validate_retained(&opened, expected, kind, matches!(kind, Kind::PrivateFile))
    }

    pub(super) fn inventory(
        directory: &File,
        maximum_entries: usize,
        maximum_name_bytes: usize,
        deadline: Instant,
    ) -> Result<Vec<String>> {
        let clone = directory.try_clone().map_err(|_| unavailable())?;
        let owned: OwnedFd = clone.into();
        let mut entries = Dir::from_fd(owned).map_err(|_| unavailable())?;
        let mut names = Vec::new();
        let mut name_bytes = 0_usize;
        for entry in entries.iter() {
            if Instant::now() > deadline || names.len() >= maximum_entries {
                return Err(budget());
            }
            let entry = entry.map_err(|_| unavailable())?;
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            let name = std::str::from_utf8(bytes).map_err(|_| unsafe_object())?;
            name_bytes = name_bytes.checked_add(name.len()).ok_or_else(budget)?;
            if name_bytes > maximum_name_bytes {
                return Err(budget());
            }
            names.push(name.to_owned());
        }
        Ok(names)
    }

    pub(super) fn sync_directory(directory: &File) -> Result<()> {
        directory.sync_all().map_err(|_| unavailable())
    }

    pub(super) fn publish_directory_no_replace(
        parent: &File,
        source: &str,
        source_directory: &File,
        source_identity: Identity,
        destination: &str,
    ) -> Result<Publication> {
        validate_retained(
            source_directory,
            source_identity,
            Kind::PrivateDirectory,
            false,
        )?;
        match rename_no_replace(parent, source, destination) {
            Ok(()) => Ok(Publication::Published),
            Err(Errno::EEXIST | Errno::ENOTEMPTY) => Ok(Publication::Collision),
            Err(_) => Err(unavailable()),
        }
    }

    pub(super) fn publish_file_replace(
        directory: &File,
        source: &str,
        source_file: &File,
        source_identity: Identity,
        destination: &str,
    ) -> Result<Publication> {
        validate_named(
            directory,
            source,
            source_file,
            source_identity,
            Kind::PrivateFile,
        )?;
        let source = CString::new(source).map_err(|_| unsafe_object())?;
        let destination = CString::new(destination).map_err(|_| unsafe_object())?;
        // SAFETY: both values are validated single components beneath the same
        // retained private store. renameat atomically installs the complete
        // create-new temp and never follows a displaced destination symlink.
        let result = unsafe {
            // DUX-DESTRUCTIVE: allow=managed-cache-entry-publish -- atomically replace only one codec-keyed final with its retained, fully validated create-new temporary
            nix::libc::renameat(
                directory.as_raw_fd(),
                source.as_ptr(),
                directory.as_raw_fd(),
                destination.as_ptr(),
            )
        };
        Errno::result(result)
            .map(|_| Publication::Published)
            .map_err(|_| unavailable())
    }

    pub(super) fn remove_retained_file(
        directory: &File,
        name: &str,
        file: File,
        expected: Identity,
    ) -> Result<()> {
        use nix::unistd::{UnlinkatFlags, unlinkat};
        validate_named(directory, name, &file, expected, Kind::PrivateFile)?;
        // DUX-DESTRUCTIVE: allow=managed-cache-exact-clear -- unlink only one exact retained private single-link object from a managed inventory, pending-temp guard, or create-new stage guard
        unlinkat(directory, name, UnlinkatFlags::NoRemoveDir).map_err(|_| unavailable())?;
        drop(file);
        Ok(())
    }

    pub(super) fn remove_retained_directory(
        parent: &File,
        name: &str,
        directory: File,
        expected: Identity,
    ) -> Result<()> {
        use nix::unistd::{UnlinkatFlags, unlinkat};
        validate_named(parent, name, &directory, expected, Kind::PrivateDirectory)?;
        // DUX-DESTRUCTIVE: allow=managed-cache-exact-stage-remove -- remove only the exact retained create-new provisioning stage after all retained controls were removed and no unproven child remains
        unlinkat(parent, name, UnlinkatFlags::RemoveDir).map_err(|_| unavailable())?;
        drop(directory);
        Ok(())
    }

    fn validate_publication_parent(parent: &File) -> Result<()> {
        let status = fstat(parent).map_err(|_| unsafe_container())?;
        if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != SFlag::S_IFDIR
            || status.st_uid != geteuid().as_raw()
            || status.st_mode & 0o022 != 0
        {
            return Err(unsafe_container());
        }
        reject_granting_acl(parent)
    }

    #[cfg(target_os = "linux")]
    fn rename_no_replace(
        parent: &File,
        source: &str,
        destination: &str,
    ) -> std::result::Result<(), Errno> {
        let source = CString::new(source).map_err(|_| Errno::EINVAL)?;
        let destination = CString::new(destination).map_err(|_| Errno::EINVAL)?;
        // SAFETY: fixed/generated single components are published beneath one
        // retained current-user container. RENAME_NOREPLACE forbids adoption.
        let result = unsafe {
            nix::libc::syscall(
                // DUX-DESTRUCTIVE: allow=managed-cache-linux-store-publish -- atomically publish only the retained marker-complete private managed-cache child without replacing an existing entry
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
    fn rename_no_replace(
        parent: &File,
        source: &str,
        destination: &str,
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
        let source = CString::new(source).map_err(|_| Errno::EINVAL)?;
        let destination = CString::new(destination).map_err(|_| Errno::EINVAL)?;
        // SAFETY: generated names are single components beneath the retained
        // container; EXCL forbids replacement and NOFOLLOW_ANY rejects aliases.
        let result = unsafe {
            // DUX-DESTRUCTIVE: allow=managed-cache-macos-store-publish -- atomically publish only the retained marker-complete private managed-cache child without replacing an existing entry
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
        // SAFETY: Darwin returns an independently allocated ACL for this live fd.
        let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
        if acl.is_null() {
            return if Errno::last() == Errno::ENOENT {
                Ok(())
            } else {
                Err(unsafe_for(kind))
            };
        }
        let mut entry = std::ptr::null_mut();
        // SAFETY: the ACL is live and the output pointer is writable.
        let status = unsafe { acl_get_entry(acl, ACL_FIRST_ENTRY, &raw mut entry) };
        // SAFETY: this frees the independently allocated ACL exactly once.
        let freed = unsafe { acl_free(acl) };
        if status < 0 || freed != 0 || !entry.is_null() {
            return Err(unsafe_for(kind));
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn reject_extended_acl(file: &File, kind: Kind) -> Result<()> {
        reject_linux_acl(file).map_err(|_| unsafe_for(kind))
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
        // SAFETY: Darwin returns an independently allocated ACL for this live fd.
        let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
        if acl.is_null() {
            return if Errno::last() == Errno::ENOENT {
                Ok(())
            } else {
                Err(unsafe_container())
            };
        }
        let inspection = (|| {
            let mut selector = ACL_FIRST_ENTRY;
            for _ in 0..ACL_MAX_ENTRIES {
                let mut entry = std::ptr::null_mut();
                // SAFETY: the ACL is live and the output pointer is writable.
                if unsafe { acl_get_entry(acl, selector, &raw mut entry) } < 0 {
                    if selector == ACL_NEXT_ENTRY && Errno::last() == Errno::EINVAL {
                        return Ok(());
                    }
                    return Err(unsafe_container());
                }
                if entry.is_null() {
                    return Ok(());
                }
                let mut tag = 0;
                // SAFETY: entry belongs to the live ACL and tag is writable.
                if unsafe { acl_get_tag_type(entry, &raw mut tag) } != 0 || tag != ACL_EXTENDED_DENY
                {
                    return Err(unsafe_container());
                }
                selector = ACL_NEXT_ENTRY;
            }
            Err(unsafe_container())
        })();
        // SAFETY: this frees the independently allocated ACL exactly once.
        if unsafe { acl_free(acl) } != 0 {
            return Err(unsafe_container());
        }
        inspection
    }

    #[cfg(target_os = "linux")]
    fn reject_granting_acl(file: &File) -> Result<()> {
        reject_linux_acl(file).map_err(|_| unsafe_container())
    }

    #[cfg(target_os = "linux")]
    fn reject_linux_acl(file: &File) -> std::result::Result<(), ()> {
        for name in [
            b"system.posix_acl_access\0".as_slice(),
            b"system.posix_acl_default\0".as_slice(),
        ] {
            // SAFETY: `name` is NUL terminated and the live descriptor is only queried.
            let length = unsafe {
                nix::libc::fgetxattr(
                    file.as_raw_fd(),
                    name.as_ptr().cast(),
                    std::ptr::null_mut(),
                    0,
                )
            };
            if length >= 0 {
                return Err(());
            }
            match Errno::last() {
                Errno::ENODATA | Errno::ENOTSUP => {}
                _ => return Err(()),
            }
        }
        Ok(())
    }

    fn map_container_error(error: Errno) -> ManagedCacheStoreError {
        match error {
            Errno::ELOOP | Errno::ENOTDIR | Errno::EISDIR => unsafe_container(),
            _ => unavailable(),
        }
    }

    fn unsafe_for(kind: Kind) -> ManagedCacheStoreError {
        match kind {
            Kind::ContainerDirectory => unsafe_container(),
            Kind::PrivateDirectory => unsafe_store(),
            Kind::PrivateFile => unsafe_object(),
        }
    }

    fn unsafe_container() -> ManagedCacheStoreError {
        ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::UnsafeContainer)
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
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

    #[test]
    fn configured_container_is_exact_absolute_dux_component() {
        let relative = Path::new("Dux");
        let Err(relative_error) =
            ManagedCacheStore::open(relative, ManagedCacheStoreAccess::ReadOnly)
        else {
            panic!("relative container was accepted");
        };
        assert_eq!(
            relative_error.kind(),
            ManagedCacheStoreErrorKind::InvalidConfiguration
        );

        let temp = TempDir::new().unwrap();
        let wrong_case = temp.path().join("dux");
        let Err(case_error) =
            ManagedCacheStore::open(&wrong_case, ManagedCacheStoreAccess::ReadOnly)
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

        let error =
            validate_same_filesystem(store.inner.container_identity, mounted_child_identity)
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
            .arg(
                "cache::managed_store::tests::reset_writer_admission_excludes_an_independent_process",
            )
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
                let identity =
                    platform::identity(&retained, platform::Kind::PrivateDirectory).unwrap();
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
                let identity =
                    platform::identity(&retained, platform::Kind::PrivateDirectory).unwrap();
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
        let insertion_name =
            final_name(&managed_cache_entry_key(&insertion.0, &insertion.1).unwrap());
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
        let directory = platform::create_private_directory_exclusive(
            &store.inner.container,
            &path,
            &stage_name,
        )
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
        platform::remove_retained_file(&stage.directory, MARKER_NAME, marker, marker_identity)
            .unwrap();
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
}

#[cfg(windows)]
mod platform {
    use std::fs::File;
    use std::path::Path;
    use std::time::Instant;

    use super::{
        ManagedCacheStoreAccess, ManagedCacheStoreError, ManagedCacheStoreErrorKind, Result,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub(super) struct Identity;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub(super) struct ChangeToken;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Kind {
        ContainerDirectory,
        PrivateDirectory,
        PrivateFile,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Publication {
        Published,
        Collision,
    }

    fn unsupported<T>() -> Result<T> {
        Err(ManagedCacheStoreError::new(
            ManagedCacheStoreErrorKind::UnsupportedPlatform,
        ))
    }

    pub(super) fn open_container_parent(_path: &Path) -> Result<(File, Identity)> {
        unsupported()
    }

    pub(super) fn open_or_create_child_container(
        _parent: &File,
        _path: &Path,
        _access: ManagedCacheStoreAccess,
    ) -> Result<Option<(File, Identity)>> {
        unsupported()
    }

    pub(super) fn open_existing_container(_parent: &File, _name: &str) -> Result<Option<File>> {
        unsupported()
    }

    pub(super) fn validate_path(
        _path: &Path,
        _retained: &File,
        _expected: Identity,
        _kind: Kind,
    ) -> Result<()> {
        unsupported()
    }

    pub(super) fn open_existing_private_directory(
        _parent: &File,
        _parent_path: &Path,
        _name: &str,
    ) -> Result<Option<File>> {
        unsupported()
    }

    pub(super) fn create_private_directory_exclusive(
        _parent: &File,
        _parent_path: &Path,
        _name: &str,
    ) -> Result<Option<File>> {
        unsupported()
    }

    pub(super) fn create_private_file_exclusive(
        _directory: &File,
        _directory_path: &Path,
        _name: &str,
    ) -> Result<Option<(File, Identity)>> {
        unsupported()
    }

    pub(super) fn open_named_private_file(
        _directory: &File,
        _directory_path: &Path,
        _name: &str,
        _writable: bool,
    ) -> Result<Option<(File, Identity)>> {
        unsupported()
    }

    pub(super) fn identity(_file: &File, _kind: Kind) -> Result<Identity> {
        unsupported()
    }

    pub(super) const fn same_filesystem(_left: Identity, _right: Identity) -> bool {
        false
    }

    pub(super) fn change_token(_file: &File) -> Result<ChangeToken> {
        unsupported()
    }

    pub(super) fn file_usage(_file: &File) -> Result<(u64, u64)> {
        unsupported()
    }

    pub(super) fn validate_retained(
        _file: &File,
        _expected: Identity,
        _kind: Kind,
        _require_one_link: bool,
    ) -> Result<()> {
        unsupported()
    }

    pub(super) fn validate_named(
        _directory: &File,
        _name: &str,
        _retained: &File,
        _expected: Identity,
        _kind: Kind,
    ) -> Result<()> {
        unsupported()
    }

    pub(super) fn inventory(
        _directory: &File,
        _maximum_entries: usize,
        _maximum_name_bytes: usize,
        _deadline: Instant,
    ) -> Result<Vec<String>> {
        unsupported()
    }

    pub(super) fn sync_directory(_directory: &File) -> Result<()> {
        unsupported()
    }

    pub(super) fn publish_directory_no_replace(
        _parent: &File,
        _source: &str,
        _source_directory: &File,
        _source_identity: Identity,
        _destination: &str,
    ) -> Result<Publication> {
        unsupported()
    }

    pub(super) fn publish_file_replace(
        _directory: &File,
        _source: &str,
        _source_file: &File,
        _source_identity: Identity,
        _destination: &str,
    ) -> Result<Publication> {
        unsupported()
    }

    pub(super) fn remove_retained_file(
        _directory: &File,
        _name: &str,
        _file: File,
        _expected: Identity,
    ) -> Result<()> {
        unsupported()
    }

    pub(super) fn remove_retained_directory(
        _parent: &File,
        _name: &str,
        _directory: File,
        _expected: Identity,
    ) -> Result<()> {
        unsupported()
    }
}
