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
use crate::persistence::{AppDataResetCacheDrainAuthority, AppDataResetCacheStageRetireAuthority};

use super::managed_codec::{
    MANAGED_CACHE_HEADER_BYTES, MAX_MANAGED_CACHE_FILE_BYTES, ManagedCacheDocument,
    ManagedCacheEntryKey, decode_managed_cache, encode_managed_cache, managed_cache_entry_key,
    preflight_managed_cache_header,
};
use super::{CacheMetadata, CachedScanConfig};
use crate::tree::DiskTree;

#[path = "managed_store/app_data_reset.rs"]
mod app_data_reset;

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
const TEST_FAULT_RESET_BEFORE_RENAME: u8 = 3;
const TEST_FAULT_RESET_AFTER_RENAME: u8 = 4;
const TEST_FAULT_RESET_AFTER_DIRECTORY_SYNC: u8 = 5;
const TEST_FAULT_RESET_DURING_READBACK: u8 = 6;
const TEST_FAULT_RESET_EXPIRE_BEFORE_RENAME: u8 = 7;
const TEST_FAULT_RESET_DRAIN_BEFORE_UNLINK: u8 = 8;
const TEST_FAULT_RESET_DRAIN_AFTER_UNLINK: u8 = 9;
const TEST_FAULT_RESET_DRAIN_AFTER_DIRECTORY_SYNC: u8 = 10;
const TEST_FAULT_RESET_DRAIN_DURING_READBACK: u8 = 11;
const TEST_FAULT_RESET_RETIRE_BEFORE_EFFECT: u8 = 12;
const TEST_FAULT_RESET_RETIRE_AFTER_EFFECT: u8 = 13;
const TEST_FAULT_RESET_RETIRE_AFTER_DIRECTORY_SYNC: u8 = 14;
const TEST_FAULT_RESET_RETIRE_DURING_READBACK: u8 = 15;

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

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestAppDataResetCacheDetachFault {
    BeforeRename,
    AfterRename,
    AfterDirectorySync,
    DuringReadback,
    ExpireBeforeRename,
}

#[cfg(test)]
pub(crate) fn set_test_app_data_reset_cache_detach_fault(fault: TestAppDataResetCacheDetachFault) {
    set_test_fault(match fault {
        TestAppDataResetCacheDetachFault::BeforeRename => TEST_FAULT_RESET_BEFORE_RENAME,
        TestAppDataResetCacheDetachFault::AfterRename => TEST_FAULT_RESET_AFTER_RENAME,
        TestAppDataResetCacheDetachFault::AfterDirectorySync => {
            TEST_FAULT_RESET_AFTER_DIRECTORY_SYNC
        }
        TestAppDataResetCacheDetachFault::DuringReadback => TEST_FAULT_RESET_DURING_READBACK,
        TestAppDataResetCacheDetachFault::ExpireBeforeRename => {
            TEST_FAULT_RESET_EXPIRE_BEFORE_RENAME
        }
    });
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetCacheDrainFault {
    BeforeUnlink,
    AfterUnlink,
    AfterDirectorySync,
    DuringReadback,
}

#[cfg(test)]
pub(crate) type TestAppDataResetCacheDrainFault = AppDataResetCacheDrainFault;

#[cfg(test)]
pub(crate) fn set_test_app_data_reset_cache_drain_fault(fault: TestAppDataResetCacheDrainFault) {
    set_test_fault(match fault {
        AppDataResetCacheDrainFault::BeforeUnlink => TEST_FAULT_RESET_DRAIN_BEFORE_UNLINK,
        AppDataResetCacheDrainFault::AfterUnlink => TEST_FAULT_RESET_DRAIN_AFTER_UNLINK,
        AppDataResetCacheDrainFault::AfterDirectorySync => {
            TEST_FAULT_RESET_DRAIN_AFTER_DIRECTORY_SYNC
        }
        AppDataResetCacheDrainFault::DuringReadback => TEST_FAULT_RESET_DRAIN_DURING_READBACK,
    });
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestAppDataResetCacheStageRetirementFault {
    BeforeEffect,
    AfterEffect,
    AfterDirectorySync,
    DuringReadback,
}

#[cfg(test)]
pub(crate) fn set_test_app_data_reset_cache_stage_retirement_fault(
    fault: TestAppDataResetCacheStageRetirementFault,
) {
    set_test_fault(match fault {
        TestAppDataResetCacheStageRetirementFault::BeforeEffect => {
            TEST_FAULT_RESET_RETIRE_BEFORE_EFFECT
        }
        TestAppDataResetCacheStageRetirementFault::AfterEffect => {
            TEST_FAULT_RESET_RETIRE_AFTER_EFFECT
        }
        TestAppDataResetCacheStageRetirementFault::AfterDirectorySync => {
            TEST_FAULT_RESET_RETIRE_AFTER_DIRECTORY_SYNC
        }
        TestAppDataResetCacheStageRetirementFault::DuringReadback => {
            TEST_FAULT_RESET_RETIRE_DURING_READBACK
        }
    });
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

/// Opaque, owned publication exclusion retained by completed-reset store
/// admission through its first ordinary store effect.
pub(crate) struct AppDataResetCompletedCacheFence {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    _publication: PublicationFence,
}

impl fmt::Debug for AppDataResetCompletedCacheFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AppDataResetCompletedCacheFence")
    }
}

/// Borrowed, consume-once managed-cache writer admission for app-data reset.
///
/// The owned writer lock remains local to
/// `with_app_data_reset_writer_admission`; the higher-ranked callback cannot
/// return this wrapper or the lock it borrows.
pub(crate) struct AppDataResetManagedCacheAdmission<'scope> {
    state: AppDataResetManagedCacheAdmissionState<'scope>,
    deadline: Instant,
    detached: bool,
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

/// Exact namespace location proven for one journal-bound managed-cache
/// recovery observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetManagedCacheRecoveryLocation {
    Canonical,
    Detached,
    ProvenAbsent,
}

/// Callback-scoped recovery proof for the exact canonical cache child and the
/// transaction-derived detached name.
///
/// A present store retains its descriptor, writer lease, controls, and full
/// bounded inventory. An absent store retains both publication fences and
/// proves that neither exact name exists. Construction never provisions,
/// repairs, or adopts a namespace object.
pub(crate) struct AppDataResetManagedCacheRecoveryAdmission<'scope> {
    store: Option<&'scope ManagedCacheStore>,
    expected_inventory: Option<InventoryFacts>,
    _writer_lock: Option<&'scope WriterLock>,
    publication: &'scope PublicationFence,
    cache_stage: &'scope AppDataResetCacheStageName,
    expected_identity: Option<(u64, u64)>,
    location: AppDataResetManagedCacheRecoveryLocation,
    deadline: Instant,
}

/// Consume-once authority to remove at most one payload object from the exact
/// journal-bound detached managed-cache namespace.
///
/// Construction rejects a canonical cache. A journaled absent cache carries
/// the same capability shape but can only return a proven no-effect batch.
#[must_use = "the cache drain candidate must be consumed or revalidated"]
pub(crate) struct AppDataResetManagedCacheDrainCandidate<'scope> {
    inner: AppDataResetManagedCacheRecoveryAdmission<'scope>,
}

/// Path- and byte-free progress from one bounded managed-cache drain batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AppDataResetManagedCacheDrainBatch {
    removed_objects: u8,
    cache_payload_has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetManagedCacheDrainError {
    BeforeEffect(ManagedCacheStoreErrorKind),
    OutcomeUnknown,
}

/// Exact physical tail state of the journal-bound detached managed cache.
///
/// The order is monotonic. `WriterOnly` is the sole accepted partial-control
/// shape because structural retirement removes the ownership marker while the
/// retained writer control is still locked. `MarkerOnly` is never accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetManagedCacheStageRetirementState {
    FullControlsEmpty,
    WriterOnly,
    EmptyStage,
    Absent,
}

/// Draining-only cache admission. A full marker-owned store with recognized
/// payloads remains on the existing payload boundary; only an empty or exact
/// monotonic structural tail can mint a retirement candidate.
#[must_use = "the draining cache admission must be consumed"]
pub(crate) enum AppDataResetManagedCacheDrainingAdmission<'scope> {
    PayloadsRemain(AppDataResetManagedCacheDrainCandidate<'scope>),
    Retirement(AppDataResetManagedCacheStageRetirementCandidate<'scope>),
    Absent(AppDataResetManagedCacheAbsentWitness<'scope>),
}

/// Exact, no-effect proof that both the canonical managed cache and the
/// transaction-derived detached stage are absent under retained publication
/// fences. This is deliberately distinct from structural-retirement authority:
/// once the stage shell is gone, the next recovery layer must not consume the
/// data witness in a fake retirement batch before old-data draining begins.
#[must_use = "the cache-absence witness must be joined to the next reset-debt batch"]
pub(crate) struct AppDataResetManagedCacheAbsentWitness<'scope> {
    inner: AppDataResetManagedCacheStageRetirementCandidate<'scope>,
}

struct AppDataResetManagedCacheRetirementControl {
    file: File,
    identity: platform::Identity,
}

struct AppDataResetManagedCachePartialRetirementStage {
    directory: File,
    identity: platform::Identity,
    writer: Option<AppDataResetManagedCacheRetirementControl>,
}

enum AppDataResetManagedCacheStageRetirementInner<'scope> {
    FullControlsEmpty {
        store: &'scope ManagedCacheStore,
        expected_inventory: InventoryFacts,
        _writer_lock: &'scope WriterLock,
    },
    Partial(&'scope AppDataResetManagedCachePartialRetirementStage),
    Absent,
}

/// Consume-once structural retirement candidate for one exact detached cache
/// stage. It has no effect authority by itself.
#[must_use = "the cache stage retirement candidate must be consumed or revalidated"]
pub(crate) struct AppDataResetManagedCacheStageRetirementCandidate<'scope> {
    inner: AppDataResetManagedCacheStageRetirementInner<'scope>,
    publication: &'scope PublicationFence,
    cache_stage: &'scope AppDataResetCacheStageName,
    expected_identity: Option<(u64, u64)>,
    state: AppDataResetManagedCacheStageRetirementState,
    deadline: Instant,
}

/// Path- and byte-free progress for one structural retirement effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AppDataResetManagedCacheStageRetirementBatch {
    removed_structural_objects: u8,
    cache_stage_has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetManagedCacheStageRetirementError {
    BeforeEffect(ManagedCacheStoreErrorKind),
    OutcomeUnknown,
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
            let mut in_use = loop {
                if Instant::now() >= deadline {
                    return Err(busy());
                }
                match PUBLICATION_LOCKS_IN_USE.try_lock() {
                    Ok(in_use) => break in_use,
                    Err(std::sync::TryLockError::WouldBlock) => {
                        let now = Instant::now();
                        if now >= deadline {
                            return Err(busy());
                        }
                        std::thread::sleep(LOCK_RETRY_INTERVAL.min(deadline.duration_since(now)));
                    }
                    Err(std::sync::TryLockError::Poisoned(_)) => {
                        return Err(internal_state());
                    }
                }
            };
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
    if let Ok(mut in_use) = PUBLICATION_LOCKS_IN_USE.try_lock() {
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

    fn validate_detached_store(
        &self,
        store: &ManagedCacheStore,
        stage: &AppDataResetCacheStageName,
    ) -> Result<()> {
        self.revalidate()?;
        let Some(container) = self.container.as_ref() else {
            return Err(changed());
        };
        validate_same_filesystem(container.identity, store.inner.directory_identity)?;
        platform::validate_retained(
            &store.inner.directory,
            store.inner.directory_identity,
            platform::Kind::PrivateDirectory,
            false,
        )?;
        if platform::open_existing_private_directory(
            &container.file,
            &self.container_path,
            STORE_DIRECTORY_NAME,
        )?
        .is_some()
        {
            return Err(changed());
        }
        platform::validate_named(
            &container.file,
            stage.as_str(),
            &store.inner.directory,
            store.inner.directory_identity,
            platform::Kind::PrivateDirectory,
        )
    }

    /// Open one exact recovery name without accepting a case-folded alias.
    /// The conventional container remains unowned and is never inventoried;
    /// the bounded descriptor scan is used only to prove this fixed spelling.
    fn open_exact_recovery_directory(&self, name: &str, deadline: Instant) -> Result<Option<File>> {
        if Instant::now() >= deadline {
            return Err(busy());
        }
        self.revalidate()?;
        let Some(container) = self.container.as_ref() else {
            return Ok(None);
        };
        let exact_before = platform::exact_name_exists(&container.file, name, deadline)?;
        let opened =
            platform::open_existing_private_directory(&container.file, &self.container_path, name)?;
        match (exact_before, opened) {
            (false, None) => Ok(None),
            // A case-folding filesystem satisfied the lookup through a
            // differently spelled entry. Recovery never adopts that alias.
            (false, Some(_)) => Err(unsafe_store()),
            (true, None) => Err(changed()),
            (true, Some(directory)) => {
                if !platform::exact_name_exists(&container.file, name, deadline)? {
                    return Err(changed());
                }
                let identity = platform::identity(&directory, platform::Kind::PrivateDirectory)?;
                platform::validate_named(
                    &container.file,
                    name,
                    &directory,
                    identity,
                    platform::Kind::PrivateDirectory,
                )?;
                Ok(Some(directory))
            }
        }
    }

    fn exact_recovery_name_exists(&self, name: &str, deadline: Instant) -> Result<bool> {
        if Instant::now() >= deadline {
            return Err(busy());
        }
        self.revalidate()?;
        let Some(container) = self.container.as_ref() else {
            return Ok(false);
        };
        platform::exact_name_exists(&container.file, name, deadline)
    }

    fn validate_exact_recovery_location(
        &self,
        store: &ManagedCacheStore,
        store_name: &str,
        cache_stage: &AppDataResetCacheStageName,
        location: AppDataResetManagedCacheRecoveryLocation,
        deadline: Instant,
    ) -> Result<()> {
        let canonical = self.open_exact_recovery_directory(STORE_DIRECTORY_NAME, deadline)?;
        let detached = self.open_exact_recovery_directory(cache_stage.as_str(), deadline)?;
        let (current, other) = match location {
            AppDataResetManagedCacheRecoveryLocation::Canonical => (canonical, detached),
            AppDataResetManagedCacheRecoveryLocation::Detached => (detached, canonical),
            AppDataResetManagedCacheRecoveryLocation::ProvenAbsent => {
                return Err(internal_state());
            }
        };
        if other.is_some() {
            return Err(changed());
        }
        let current = current.ok_or_else(changed)?;
        let container = self.container.as_ref().ok_or_else(changed)?;
        platform::validate_named(
            &container.file,
            store_name,
            &store.inner.directory,
            store.inner.directory_identity,
            platform::Kind::PrivateDirectory,
        )?;
        platform::validate_retained(
            &current,
            store.inner.directory_identity,
            platform::Kind::PrivateDirectory,
            false,
        )
    }

    fn validate_exact_recovery_absence(
        &self,
        cache_stage: &AppDataResetCacheStageName,
        deadline: Instant,
    ) -> Result<()> {
        if self
            .open_exact_recovery_directory(STORE_DIRECTORY_NAME, deadline)?
            .is_some()
            || self
                .open_exact_recovery_directory(cache_stage.as_str(), deadline)?
                .is_some()
        {
            Err(changed())
        } else {
            Ok(())
        }
    }

    fn validate_exact_retirement_stage(
        &self,
        directory: &File,
        identity: platform::Identity,
        cache_stage: &AppDataResetCacheStageName,
        deadline: Instant,
    ) -> Result<()> {
        if self
            .open_exact_recovery_directory(STORE_DIRECTORY_NAME, deadline)?
            .is_some()
        {
            return Err(changed());
        }
        let current = self
            .open_exact_recovery_directory(cache_stage.as_str(), deadline)?
            .ok_or_else(changed)?;
        let container = self.container.as_ref().ok_or_else(changed)?;
        validate_same_filesystem(container.identity, identity)?;
        platform::validate_named(
            &container.file,
            cache_stage.as_str(),
            directory,
            identity,
            platform::Kind::PrivateDirectory,
        )?;
        platform::validate_retained(&current, identity, platform::Kind::PrivateDirectory, false)
    }

    fn detach_store(
        &self,
        store: &ManagedCacheStore,
        expected: &InventoryFacts,
        stage: &AppDataResetCacheStageName,
        deadline: Instant,
    ) -> Result<()> {
        self.validate_store(store)?;
        self.validate_cache_stage_absent(stage)?;
        if take_test_fault(TEST_FAULT_RESET_BEFORE_RENAME) {
            return Err(unavailable());
        }
        if take_test_fault(TEST_FAULT_RESET_EXPIRE_BEFORE_RENAME) {
            std::thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .saturating_add(Duration::from_millis(1)),
            );
        }
        if Instant::now() >= deadline {
            return Err(busy());
        }
        let Some(container) = self.container.as_ref() else {
            return Err(changed());
        };
        platform::detach_directory_no_replace(
            &container.file,
            STORE_DIRECTORY_NAME,
            &store.inner.directory,
            store.inner.directory_identity,
            stage.as_str(),
        )?;
        if take_test_fault(TEST_FAULT_RESET_AFTER_RENAME) {
            return Err(unavailable());
        }
        platform::sync_directory(&container.file)?;
        if take_test_fault(TEST_FAULT_RESET_AFTER_DIRECTORY_SYNC) {
            return Err(unavailable());
        }
        self.validate_detached_store(store, stage)?;
        let post_effect_deadline = Instant::now()
            .checked_add(INVENTORY_DEADLINE)
            .ok_or_else(budget)?;
        let current = store
            .inventory_locked_at_name_until(stage.as_str(), post_effect_deadline)?
            .facts();
        if take_test_fault(TEST_FAULT_RESET_DURING_READBACK) {
            return Err(unavailable());
        }
        if current == *expected {
            Ok(())
        } else {
            Err(changed())
        }
    }
}

#[path = "managed_store/operations.rs"]
mod operations;
#[cfg(test)]
use operations::is_optional_completed_cache_object_error;

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

fn retirement_inventory_names(directory: &File, deadline: Instant) -> Result<Vec<String>> {
    if Instant::now() >= deadline {
        return Err(busy());
    }
    let inventory_deadline = Instant::now()
        .checked_add(INVENTORY_DEADLINE)
        .ok_or_else(budget)?
        .min(deadline);
    let mut names = platform::inventory(
        directory,
        RECOVERY_MAX_NON_CONTROL_OBJECTS + 3,
        MAX_INVENTORY_NAME_BYTES,
        inventory_deadline,
    )?;
    names.sort_unstable();
    if Instant::now() >= inventory_deadline {
        return if Instant::now() >= deadline {
            Err(busy())
        } else {
            Err(budget())
        };
    }
    Ok(names)
}

fn acquire_retirement_writer_lock(file: &File, deadline: Instant) -> Result<()> {
    loop {
        if Instant::now() >= deadline {
            return Err(busy());
        }
        match FileExt::try_lock(file) {
            Ok(()) if Instant::now() >= deadline => {
                let _ = FileExt::unlock(file);
                return Err(busy());
            }
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(
                    LOCK_RETRY_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Err(TryLockError::WouldBlock) => return Err(busy()),
            Err(TryLockError::Error(_)) => return Err(unavailable()),
        }
    }
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
#[path = "managed_store/unix.rs"]
mod platform;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[path = "managed_store/tests.rs"]
mod tests;

#[cfg(windows)]
#[path = "managed_store/windows.rs"]
mod platform;
