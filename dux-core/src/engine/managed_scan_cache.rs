//! Engine-owned access to the private managed scan cache.
//!
//! Cache reads and writes are presentation accelerators, so their failure must
//! never prevent a fresh scan. Footprint and clear operations are different:
//! they report typed failures rather than fabricating an empty store.

use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

use crate::cache::{
    CacheMetadata, CachedScanConfig, ManagedCacheClearError, ManagedCacheClearResult,
    ManagedCacheClearSnapshot, ManagedCacheSaveError, ManagedCacheStorageUsage, ManagedCacheStore,
    ManagedCacheStoreAccess, ManagedCacheStoreErrorKind, ManagedCacheStoreFootprint,
};
use crate::tree::DiskTree;

use super::storage_footprint::{DuxManagedScanCacheFootprint, DuxOwnedStorageUsage};

pub(super) const MANAGED_SCAN_CACHE_CLEAR_PREVIEW_LIFETIME: Duration = Duration::from_secs(2 * 60);

enum ManagedScanCacheState {
    Uninitialized,
    Available(Arc<ManagedCacheStore>),
    AbsentReadOnly,
    PermanentlyUnavailable(ManagedCacheStoreErrorKind),
}

pub(super) struct ManagedScanCache {
    container: PathBuf,
    access: ManagedCacheStoreAccess,
    state: Mutex<ManagedScanCacheState>,
}

impl ManagedScanCache {
    pub(super) fn new(container: PathBuf, access: ManagedCacheStoreAccess) -> Self {
        Self {
            container,
            access,
            state: Mutex::new(ManagedScanCacheState::Uninitialized),
        }
    }

    pub(super) fn load(
        &self,
        root: &Path,
        config: &CachedScanConfig,
    ) -> Result<Option<(CacheMetadata, DiskTree)>, DuxManagedScanCacheError> {
        match self.resolve_store().map_err(map_store_error)? {
            Some(store) => store
                .load(root, config)
                .map(|document| document.map(|document| document.into_parts()))
                .map_err(|error| map_store_error(error.kind())),
            None => Ok(None),
        }
    }

    pub(super) fn save(
        &self,
        root: &Path,
        config: &CachedScanConfig,
        metadata: &CacheMetadata,
        tree: &DiskTree,
    ) -> Result<(), DuxManagedScanCacheError> {
        let store = self.store_for_write()?;
        store
            .save(root, config, metadata, tree)
            .map_err(|error| match error {
                ManagedCacheSaveError::BeforePublication(error) => map_store_error(error.kind()),
                ManagedCacheSaveError::OutcomeUnknown => DuxManagedScanCacheError::OutcomeUnknown,
            })
    }

    pub(super) fn footprint(
        &self,
    ) -> Result<DuxManagedScanCacheFootprint, DuxOwnedStorageFootprintCacheError> {
        match self.resolve_store().map_err(map_footprint_error)? {
            Some(store) => store
                .footprint()
                .map(public_footprint)
                .map_err(|error| map_footprint_error(error.kind())),
            None => Ok(DuxManagedScanCacheFootprint::default()),
        }
    }

    pub(super) fn prepare_clear(
        &self,
        prepared_at: SystemTime,
        monotonic_now: Instant,
    ) -> Result<DuxManagedScanCacheClearPreview, DuxManagedScanCacheClearError> {
        let store = self.store_for_clear()?;
        let prepared = store
            .prepare_clear()
            .map_err(|error| map_clear_store_error(error.kind()))?
            .ok_or(DuxManagedScanCacheClearError::NothingToClear)?;
        DuxManagedScanCacheClearPreview::new(&store, prepared, prepared_at, monotonic_now)
            .ok_or(DuxManagedScanCacheClearError::CorruptData)
    }

    pub(super) fn clear(
        &self,
        preview: DuxManagedScanCacheClearPreview,
        now: Instant,
    ) -> Result<DuxManagedScanCacheClearResult, DuxManagedScanCacheClearError> {
        let store = self.store_for_clear()?;
        clear(&store, preview, now)
    }

    fn store_for_write(&self) -> Result<Arc<ManagedCacheStore>, DuxManagedScanCacheError> {
        match self.resolve_store().map_err(map_store_error)? {
            Some(store) => Ok(store),
            None => Err(DuxManagedScanCacheError::ReadOnlyStore),
        }
    }

    fn store_for_clear(&self) -> Result<Arc<ManagedCacheStore>, DuxManagedScanCacheClearError> {
        match self.resolve_store().map_err(map_clear_store_error)? {
            Some(store) => Ok(store),
            None => Err(DuxManagedScanCacheClearError::ReadOnlyStore),
        }
    }

    fn resolve_store(&self) -> Result<Option<Arc<ManagedCacheStore>>, ManagedCacheStoreErrorKind> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ManagedCacheStoreErrorKind::InternalState)?;
        match &*state {
            ManagedScanCacheState::Available(store) => return Ok(Some(Arc::clone(store))),
            ManagedScanCacheState::AbsentReadOnly => return Ok(None),
            ManagedScanCacheState::PermanentlyUnavailable(kind) => return Err(*kind),
            ManagedScanCacheState::Uninitialized => {}
        }
        match ManagedCacheStore::open(&self.container, self.access) {
            Ok(Some(store)) => {
                let store = Arc::new(store);
                *state = ManagedScanCacheState::Available(Arc::clone(&store));
                Ok(Some(store))
            }
            Ok(None) => {
                *state = ManagedScanCacheState::AbsentReadOnly;
                Ok(None)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    ManagedCacheStoreErrorKind::Busy | ManagedCacheStoreErrorKind::Unavailable
                ) =>
            {
                // Transient startup or contention failures are retried by the
                // next explicit cache operation. They never poison the engine
                // session or delay EngineHandle::open.
                Err(error.kind())
            }
            Err(error) => {
                *state = ManagedScanCacheState::PermanentlyUnavailable(error.kind());
                Err(error.kind())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DuxManagedScanCacheError {
    #[error("engine session is closed")]
    Closed,
    #[error("the managed scan-cache store is read-only")]
    ReadOnlyStore,
    #[error("the managed scan-cache store is busy")]
    Busy,
    #[error("the managed scan-cache store is unsafe")]
    UnsafeStorage,
    #[error("managed scan-cache validation exceeded its resource budget")]
    BudgetExceeded,
    #[error("the managed scan-cache store is corrupt")]
    CorruptData,
    #[error("the result of publishing the managed scan cache is unknown")]
    OutcomeUnknown,
    #[error("the managed scan-cache store is unavailable")]
    Unavailable,
    #[error("managed scan-cache state is invalid")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DuxOwnedStorageFootprintCacheError {
    Busy,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    Unavailable,
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DuxManagedScanCacheClearPreviewInfo {
    entry_count: u32,
    temporary_count: u32,
    clearable_count: u32,
    clearable: DuxOwnedStorageUsage,
    prepared_at: SystemTime,
    expires_at: SystemTime,
}

impl DuxManagedScanCacheClearPreviewInfo {
    pub const fn entry_count(&self) -> u32 {
        self.entry_count
    }

    pub const fn temporary_count(&self) -> u32 {
        self.temporary_count
    }

    pub const fn clearable_count(&self) -> u32 {
        self.clearable_count
    }

    pub const fn clearable(&self) -> DuxOwnedStorageUsage {
        self.clearable
    }

    pub const fn prepared_at(&self) -> SystemTime {
        self.prepared_at
    }

    pub const fn expires_at(&self) -> SystemTime {
        self.expires_at
    }
}

pub struct DuxManagedScanCacheClearPreview {
    owner: Weak<ManagedCacheStore>,
    prepared: ManagedCacheClearSnapshot,
    info: DuxManagedScanCacheClearPreviewInfo,
    monotonic_expires_at: Instant,
}

impl std::fmt::Debug for DuxManagedScanCacheClearPreview {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DuxManagedScanCacheClearPreview")
            .field("info", &self.info)
            .field("owner_live", &self.owner.strong_count())
            .finish_non_exhaustive()
    }
}

impl DuxManagedScanCacheClearPreview {
    fn new(
        store: &Arc<ManagedCacheStore>,
        prepared: ManagedCacheClearSnapshot,
        prepared_at: SystemTime,
        monotonic_now: Instant,
    ) -> Option<Self> {
        let footprint = prepared.footprint();
        let clearable_count = footprint
            .entry_count
            .checked_add(footprint.temporary_count)?;
        if clearable_count == 0 {
            return None;
        }
        let clearable =
            checked_usage_add(footprint.entries, footprint.temporary).map(public_usage)?;
        let monotonic_expires_at =
            monotonic_now.checked_add(MANAGED_SCAN_CACHE_CLEAR_PREVIEW_LIFETIME)?;
        let expires_at = prepared_at.checked_add(MANAGED_SCAN_CACHE_CLEAR_PREVIEW_LIFETIME)?;
        if prepared_at >= expires_at {
            return None;
        }
        Some(Self {
            owner: Arc::downgrade(store),
            prepared,
            info: DuxManagedScanCacheClearPreviewInfo {
                entry_count: footprint.entry_count,
                temporary_count: footprint.temporary_count,
                clearable_count,
                clearable,
                prepared_at,
                expires_at,
            },
            monotonic_expires_at,
        })
    }

    pub fn info(
        &self,
    ) -> Result<DuxManagedScanCacheClearPreviewInfo, DuxManagedScanCacheClearError> {
        self.info_at(Instant::now())
    }

    pub(super) fn info_at(
        &self,
        now: Instant,
    ) -> Result<DuxManagedScanCacheClearPreviewInfo, DuxManagedScanCacheClearError> {
        if now >= self.monotonic_expires_at {
            return Err(DuxManagedScanCacheClearError::PreviewExpired);
        }
        Ok(self.info)
    }

    pub(super) fn belongs_to(&self, store: &Arc<ManagedCacheStore>) -> bool {
        self.owner
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, store))
    }

    pub(super) fn into_prepared(
        self,
        now: Instant,
    ) -> Result<ManagedCacheClearSnapshot, DuxManagedScanCacheClearError> {
        if now >= self.monotonic_expires_at {
            return Err(DuxManagedScanCacheClearError::PreviewExpired);
        }
        Ok(self.prepared)
    }

    #[cfg(test)]
    pub(super) const fn monotonic_expires_at_for_test(&self) -> Instant {
        self.monotonic_expires_at
    }

    pub fn release(self) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DuxManagedScanCacheClearResult {
    cleared_entry_count: u32,
    cleared_temporary_count: u32,
    cleared_count: u32,
    cleared_usage: DuxOwnedStorageUsage,
}

impl DuxManagedScanCacheClearResult {
    pub const fn cleared_entry_count(&self) -> u32 {
        self.cleared_entry_count
    }

    pub const fn cleared_temporary_count(&self) -> u32 {
        self.cleared_temporary_count
    }

    pub const fn cleared_count(&self) -> u32 {
        self.cleared_count
    }

    pub const fn cleared_usage(&self) -> DuxOwnedStorageUsage {
        self.cleared_usage
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DuxManagedScanCacheClearError {
    #[error("engine session is closed")]
    Closed,
    #[error("there is no managed scan cache to clear")]
    NothingToClear,
    #[error("the managed scan-cache store is read-only")]
    ReadOnlyStore,
    #[error("the managed scan cache changed after confirmation")]
    ChangedSincePreview,
    #[error("the managed scan-cache clear preview expired")]
    PreviewExpired,
    #[error("the managed scan-cache clear preview belongs to another engine")]
    WrongEngine,
    #[error("the managed scan-cache store is busy")]
    Busy,
    #[error("the managed scan-cache store is unsafe")]
    UnsafeStorage,
    #[error("managed scan-cache validation exceeded its resource budget")]
    BudgetExceeded,
    #[error("the managed scan-cache store is corrupt")]
    CorruptData,
    #[error("the result of clearing the managed scan cache is unknown")]
    OutcomeUnknown,
    #[error("the managed scan-cache store is unavailable")]
    Unavailable,
    #[error("managed scan-cache clearing state is invalid")]
    InternalState,
}

pub(super) fn clear(
    store: &Arc<ManagedCacheStore>,
    preview: DuxManagedScanCacheClearPreview,
    now: Instant,
) -> Result<DuxManagedScanCacheClearResult, DuxManagedScanCacheClearError> {
    if !preview.belongs_to(store) {
        return Err(DuxManagedScanCacheClearError::WrongEngine);
    }
    let expected = preview.info_at(now)?;
    let prepared = preview.into_prepared(now)?;
    let result = store.clear(prepared).map_err(|error| match error {
        ManagedCacheClearError::BeforeEffect(error) => map_clear_store_error(error.kind()),
        ManagedCacheClearError::OutcomeUnknown => DuxManagedScanCacheClearError::OutcomeUnknown,
    })?;
    public_clear_result(result, expected)
}

fn public_clear_result(
    result: ManagedCacheClearResult,
    expected: DuxManagedScanCacheClearPreviewInfo,
) -> Result<DuxManagedScanCacheClearResult, DuxManagedScanCacheClearError> {
    let cleared_count = result
        .cleared_entries
        .checked_add(result.cleared_temporary)
        .ok_or(DuxManagedScanCacheClearError::OutcomeUnknown)?;
    let cleared_usage = public_usage(result.cleared_usage);
    if result.cleared_entries != expected.entry_count
        || result.cleared_temporary != expected.temporary_count
        || cleared_count != expected.clearable_count
        || cleared_usage != expected.clearable
    {
        return Err(DuxManagedScanCacheClearError::OutcomeUnknown);
    }
    Ok(DuxManagedScanCacheClearResult {
        cleared_entry_count: result.cleared_entries,
        cleared_temporary_count: result.cleared_temporary,
        cleared_count,
        cleared_usage,
    })
}

fn public_footprint(footprint: ManagedCacheStoreFootprint) -> DuxManagedScanCacheFootprint {
    DuxManagedScanCacheFootprint {
        controls: public_usage(footprint.controls),
        entries: public_usage(footprint.entries),
        temporary: public_usage(footprint.temporary),
        total: public_usage(footprint.total),
        entry_count: footprint.entry_count,
        temporary_count: footprint.temporary_count,
    }
}

fn public_usage(usage: ManagedCacheStorageUsage) -> DuxOwnedStorageUsage {
    DuxOwnedStorageUsage {
        logical_bytes: usage.logical_bytes,
        allocated_bytes: usage.allocated_bytes,
        charged_bytes: usage.charged_bytes,
    }
}

fn checked_usage_add(
    left: ManagedCacheStorageUsage,
    right: ManagedCacheStorageUsage,
) -> Option<ManagedCacheStorageUsage> {
    Some(ManagedCacheStorageUsage {
        logical_bytes: left.logical_bytes.checked_add(right.logical_bytes)?,
        allocated_bytes: left.allocated_bytes.checked_add(right.allocated_bytes)?,
        charged_bytes: left.charged_bytes.checked_add(right.charged_bytes)?,
    })
}

fn map_store_error(kind: ManagedCacheStoreErrorKind) -> DuxManagedScanCacheError {
    match kind {
        ManagedCacheStoreErrorKind::ReadOnly => DuxManagedScanCacheError::ReadOnlyStore,
        ManagedCacheStoreErrorKind::UnsafeContainer
        | ManagedCacheStoreErrorKind::UnsafeStore
        | ManagedCacheStoreErrorKind::UnsafeObject
        | ManagedCacheStoreErrorKind::UnrecognizedStore => DuxManagedScanCacheError::UnsafeStorage,
        ManagedCacheStoreErrorKind::Busy => DuxManagedScanCacheError::Busy,
        ManagedCacheStoreErrorKind::BudgetExceeded => DuxManagedScanCacheError::BudgetExceeded,
        ManagedCacheStoreErrorKind::CorruptData
        | ManagedCacheStoreErrorKind::ChangedSinceSnapshot => DuxManagedScanCacheError::CorruptData,
        ManagedCacheStoreErrorKind::OutcomeUnknown => DuxManagedScanCacheError::OutcomeUnknown,
        ManagedCacheStoreErrorKind::Unavailable
        | ManagedCacheStoreErrorKind::UnsupportedPlatform => DuxManagedScanCacheError::Unavailable,
        ManagedCacheStoreErrorKind::InvalidConfiguration
        | ManagedCacheStoreErrorKind::InternalState => DuxManagedScanCacheError::InternalState,
    }
}

fn map_clear_store_error(kind: ManagedCacheStoreErrorKind) -> DuxManagedScanCacheClearError {
    match kind {
        ManagedCacheStoreErrorKind::ReadOnly => DuxManagedScanCacheClearError::ReadOnlyStore,
        ManagedCacheStoreErrorKind::ChangedSinceSnapshot => {
            DuxManagedScanCacheClearError::ChangedSincePreview
        }
        ManagedCacheStoreErrorKind::UnsafeContainer
        | ManagedCacheStoreErrorKind::UnsafeStore
        | ManagedCacheStoreErrorKind::UnsafeObject
        | ManagedCacheStoreErrorKind::UnrecognizedStore => {
            DuxManagedScanCacheClearError::UnsafeStorage
        }
        ManagedCacheStoreErrorKind::Busy => DuxManagedScanCacheClearError::Busy,
        ManagedCacheStoreErrorKind::BudgetExceeded => DuxManagedScanCacheClearError::BudgetExceeded,
        ManagedCacheStoreErrorKind::CorruptData => DuxManagedScanCacheClearError::CorruptData,
        ManagedCacheStoreErrorKind::OutcomeUnknown => DuxManagedScanCacheClearError::OutcomeUnknown,
        ManagedCacheStoreErrorKind::Unavailable
        | ManagedCacheStoreErrorKind::UnsupportedPlatform => {
            DuxManagedScanCacheClearError::Unavailable
        }
        ManagedCacheStoreErrorKind::InvalidConfiguration
        | ManagedCacheStoreErrorKind::InternalState => DuxManagedScanCacheClearError::InternalState,
    }
}

fn map_footprint_error(kind: ManagedCacheStoreErrorKind) -> DuxOwnedStorageFootprintCacheError {
    match kind {
        ManagedCacheStoreErrorKind::ReadOnly => DuxOwnedStorageFootprintCacheError::InternalState,
        ManagedCacheStoreErrorKind::UnsafeContainer
        | ManagedCacheStoreErrorKind::UnsafeStore
        | ManagedCacheStoreErrorKind::UnsafeObject
        | ManagedCacheStoreErrorKind::UnrecognizedStore => {
            DuxOwnedStorageFootprintCacheError::UnsafeStorage
        }
        ManagedCacheStoreErrorKind::Busy => DuxOwnedStorageFootprintCacheError::Busy,
        ManagedCacheStoreErrorKind::BudgetExceeded => {
            DuxOwnedStorageFootprintCacheError::BudgetExceeded
        }
        ManagedCacheStoreErrorKind::CorruptData
        | ManagedCacheStoreErrorKind::ChangedSinceSnapshot => {
            DuxOwnedStorageFootprintCacheError::CorruptData
        }
        ManagedCacheStoreErrorKind::OutcomeUnknown => {
            DuxOwnedStorageFootprintCacheError::Unavailable
        }
        ManagedCacheStoreErrorKind::Unavailable
        | ManagedCacheStoreErrorKind::UnsupportedPlatform => {
            DuxOwnedStorageFootprintCacheError::Unavailable
        }
        ManagedCacheStoreErrorKind::InvalidConfiguration
        | ManagedCacheStoreErrorKind::InternalState => {
            DuxOwnedStorageFootprintCacheError::InternalState
        }
    }
}
