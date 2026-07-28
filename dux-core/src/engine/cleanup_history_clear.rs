//! Consume-once authority for clearing DUX-owned cleanup history.
//!
//! The opaque preview binds one engine/store to one exact validated history
//! graph. It contains no path, session list, plan, or cleanup authority.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use crate::persistence::{PreparedCleanupHistoryClear, StoreCoordinator};

pub(super) const CLEANUP_HISTORY_CLEAR_PREVIEW_LIFETIME: Duration = Duration::from_secs(2 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupHistoryClearPreviewInfo {
    session_count: u64,
    oldest_started_at: SystemTime,
    newest_started_at: SystemTime,
    prepared_at: SystemTime,
    expires_at: SystemTime,
}

impl CleanupHistoryClearPreviewInfo {
    pub(super) fn new(
        prepared: &PreparedCleanupHistoryClear,
        prepared_at: SystemTime,
        expires_at: SystemTime,
    ) -> Option<Self> {
        if prepared.session_count() == 0
            || prepared.oldest_started_at() > prepared.newest_started_at()
            || prepared_at >= expires_at
        {
            return None;
        }
        Some(Self {
            session_count: prepared.session_count(),
            oldest_started_at: prepared.oldest_started_at(),
            newest_started_at: prepared.newest_started_at(),
            prepared_at,
            expires_at,
        })
    }

    pub const fn session_count(&self) -> u64 {
        self.session_count
    }

    pub const fn oldest_started_at(&self) -> SystemTime {
        self.oldest_started_at
    }

    pub const fn newest_started_at(&self) -> SystemTime {
        self.newest_started_at
    }

    pub const fn prepared_at(&self) -> SystemTime {
        self.prepared_at
    }

    pub const fn expires_at(&self) -> SystemTime {
        self.expires_at
    }
}

pub struct CleanupHistoryClearPreview {
    owner: Weak<StoreCoordinator>,
    prepared: PreparedCleanupHistoryClear,
    info: CleanupHistoryClearPreviewInfo,
    monotonic_expires_at: Instant,
}

impl std::fmt::Debug for CleanupHistoryClearPreview {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CleanupHistoryClearPreview")
            .field("info", &self.info)
            .field("owner_live", &self.owner.strong_count())
            .finish_non_exhaustive()
    }
}

impl CleanupHistoryClearPreview {
    pub(super) fn new(
        store: &Arc<StoreCoordinator>,
        prepared: PreparedCleanupHistoryClear,
        prepared_at: SystemTime,
        monotonic_now: Instant,
    ) -> Option<Self> {
        let monotonic_expires_at =
            monotonic_now.checked_add(CLEANUP_HISTORY_CLEAR_PREVIEW_LIFETIME)?;
        let expires_at = prepared_at.checked_add(CLEANUP_HISTORY_CLEAR_PREVIEW_LIFETIME)?;
        let info = CleanupHistoryClearPreviewInfo::new(&prepared, prepared_at, expires_at)?;
        Some(Self {
            owner: Arc::downgrade(store),
            prepared,
            info,
            monotonic_expires_at,
        })
    }

    pub fn info(&self) -> Result<CleanupHistoryClearPreviewInfo, CleanupHistoryClearError> {
        self.info_at(Instant::now())
    }

    pub(super) fn info_at(
        &self,
        now: Instant,
    ) -> Result<CleanupHistoryClearPreviewInfo, CleanupHistoryClearError> {
        if now >= self.monotonic_expires_at {
            return Err(CleanupHistoryClearError::PreviewExpired);
        }
        Ok(self.info.clone())
    }

    #[cfg(test)]
    pub(super) const fn monotonic_expires_at_for_test(&self) -> Instant {
        self.monotonic_expires_at
    }

    pub(super) fn belongs_to(&self, store: &Arc<StoreCoordinator>) -> bool {
        self.owner
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, store))
    }

    pub(super) fn into_prepared(
        self,
        now: Instant,
    ) -> Result<PreparedCleanupHistoryClear, CleanupHistoryClearError> {
        if now >= self.monotonic_expires_at {
            return Err(CleanupHistoryClearError::PreviewExpired);
        }
        Ok(self.prepared)
    }

    pub fn release(self) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupHistoryClearResult {
    cleared_session_count: u64,
}

impl CleanupHistoryClearResult {
    pub(super) const fn new(cleared_session_count: u64) -> Option<Self> {
        if cleared_session_count == 0 {
            None
        } else {
            Some(Self {
                cleared_session_count,
            })
        }
    }

    pub const fn cleared_session_count(&self) -> u64 {
        self.cleared_session_count
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CleanupHistoryClearError {
    #[error("engine session is closed")]
    Closed,
    #[error("there is no cleanup history to clear")]
    NothingToClear,
    #[error("cleanup history includes unfinished or uncertain work")]
    ActiveCleanup,
    #[error("cleanup history changed after the clear preview")]
    ChangedSincePreview,
    #[error("the cleanup-history clear preview expired")]
    PreviewExpired,
    #[error("the cleanup-history clear preview belongs to another engine")]
    WrongEngine,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("cleanup-history validation exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable cleanup history is corrupt")]
    CorruptData,
    #[error("the result of clearing cleanup history is unknown")]
    OutcomeUnknown,
    #[error("durable cleanup history is unavailable")]
    Unavailable,
    #[error("engine cleanup-history clearing state is unavailable")]
    InternalState,
}
