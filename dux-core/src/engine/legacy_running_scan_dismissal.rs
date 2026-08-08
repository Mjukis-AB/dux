//! Consume-once user authority for dismissing legacy unfinished bookkeeping.
//!
//! The public preview is path- and identity-free. Its private prepared value
//! can annotate exact durable history, but carries no filesystem authority.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use crate::persistence::{PreparedLegacyRunningScanDismissal, StoreCoordinator};

pub const MAX_LEGACY_RUNNING_SCAN_DISMISSAL_ROWS: u16 = 64;
pub const LEGACY_RUNNING_SCAN_DISMISSAL_PREVIEW_LIFETIME: Duration = Duration::from_secs(2 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyRunningScanDismissalPreviewInfo {
    eligible_count: u16,
    has_more: bool,
    prepared_at: SystemTime,
    expires_at: SystemTime,
}

impl LegacyRunningScanDismissalPreviewInfo {
    pub(super) fn new(
        prepared: &PreparedLegacyRunningScanDismissal,
        prepared_at: SystemTime,
        expires_at: SystemTime,
    ) -> Option<Self> {
        if prepared.eligible_count() == 0
            || prepared.eligible_count() > MAX_LEGACY_RUNNING_SCAN_DISMISSAL_ROWS
            || prepared_at >= expires_at
        {
            return None;
        }
        Some(Self {
            eligible_count: prepared.eligible_count(),
            has_more: prepared.has_more(),
            prepared_at,
            expires_at,
        })
    }

    pub const fn eligible_count(&self) -> u16 {
        self.eligible_count
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }

    pub const fn prepared_at(&self) -> SystemTime {
        self.prepared_at
    }

    pub const fn expires_at(&self) -> SystemTime {
        self.expires_at
    }
}

pub struct LegacyRunningScanDismissalPreview {
    owner: Weak<StoreCoordinator>,
    prepared: PreparedLegacyRunningScanDismissal,
    info: LegacyRunningScanDismissalPreviewInfo,
    monotonic_expires_at: Instant,
}

impl std::fmt::Debug for LegacyRunningScanDismissalPreview {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LegacyRunningScanDismissalPreview")
            .field("info", &self.info)
            .field("owner_live", &self.owner.strong_count())
            .finish_non_exhaustive()
    }
}

impl LegacyRunningScanDismissalPreview {
    pub(super) fn new(
        store: &Arc<StoreCoordinator>,
        prepared: PreparedLegacyRunningScanDismissal,
        prepared_at: SystemTime,
        monotonic_now: Instant,
    ) -> Option<Self> {
        let monotonic_expires_at =
            monotonic_now.checked_add(LEGACY_RUNNING_SCAN_DISMISSAL_PREVIEW_LIFETIME)?;
        let expires_at = prepared_at.checked_add(LEGACY_RUNNING_SCAN_DISMISSAL_PREVIEW_LIFETIME)?;
        let info = LegacyRunningScanDismissalPreviewInfo::new(&prepared, prepared_at, expires_at)?;
        Some(Self {
            owner: Arc::downgrade(store),
            prepared,
            info,
            monotonic_expires_at,
        })
    }

    pub fn info(
        &self,
    ) -> Result<LegacyRunningScanDismissalPreviewInfo, LegacyRunningScanDismissalError> {
        self.info_at(Instant::now())
    }

    pub(super) fn info_at(
        &self,
        now: Instant,
    ) -> Result<LegacyRunningScanDismissalPreviewInfo, LegacyRunningScanDismissalError> {
        if now >= self.monotonic_expires_at {
            return Err(LegacyRunningScanDismissalError::PreviewExpired);
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
    ) -> Result<PreparedLegacyRunningScanDismissal, LegacyRunningScanDismissalError> {
        if now >= self.monotonic_expires_at {
            return Err(LegacyRunningScanDismissalError::PreviewExpired);
        }
        Ok(self.prepared)
    }

    pub fn release(self) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyRunningScanDismissalResult {
    dismissed_count: u16,
    has_more: bool,
}

impl LegacyRunningScanDismissalResult {
    pub(super) const fn new(dismissed_count: u16, has_more: bool) -> Option<Self> {
        if dismissed_count == 0 || dismissed_count > MAX_LEGACY_RUNNING_SCAN_DISMISSAL_ROWS {
            None
        } else {
            Some(Self {
                dismissed_count,
                has_more,
            })
        }
    }

    pub const fn dismissed_count(&self) -> u16 {
        self.dismissed_count
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LegacyRunningScanDismissalError {
    #[error("engine session is closed")]
    Closed,
    #[error("there is no eligible legacy unfinished scan bookkeeping to dismiss")]
    NothingEligible,
    #[error("legacy unfinished scan bookkeeping changed after the preview")]
    ChangedSincePreview,
    #[error("the legacy dismissal preview expired")]
    PreviewExpired,
    #[error("the legacy dismissal preview belongs to another engine")]
    WrongEngine,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("legacy dismissal validation exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable legacy running-scan bookkeeping is corrupt")]
    CorruptData,
    #[error("the result of dismissing legacy scan bookkeeping is unknown")]
    OutcomeUnknown,
    #[error("durable legacy running-scan bookkeeping is unavailable")]
    Unavailable,
    #[error("engine legacy dismissal state is unavailable")]
    InternalState,
}
