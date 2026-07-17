use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::ScanId;
use crate::persistence::HistoryErrorKind;
use crate::persistence::snapshot::{
    SnapshotCodecErrorKind, SnapshotRepositoryErrorKind, SnapshotReviewLease as StoredReviewLease,
    SnapshotStorageErrorKind,
};

/// Stable, path-free failures from an Explorer snapshot-review session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SnapshotReviewError {
    #[error("engine session is closed")]
    Closed,
    #[error("the scan does not exist")]
    ScanNotFound,
    #[error("the scan has no available snapshot")]
    SnapshotUnavailable,
    #[error("the review lease expired")]
    LeaseExpired,
    #[error("the durable engine store is read-only")]
    ReadOnlyStore,
    #[error("the durable engine schema is incompatible")]
    IncompatibleSchema,
    #[error("snapshot review is temporarily busy")]
    Busy,
    #[error("snapshot storage failed its safety checks")]
    UnsafeStorage,
    #[error("the bounded review operation exceeded its budget")]
    BudgetExceeded,
    #[error("durable snapshot state is corrupt")]
    CorruptData,
    #[error("the snapshot format is incompatible")]
    IncompatibleSnapshot,
    #[error("snapshot review storage is unavailable")]
    Unavailable,
    #[error("the review operation outcome is unknown")]
    OutcomeUnknown,
    #[error("snapshot review reached an invalid internal state")]
    InternalState,
}

/// Result of explicitly ending an Explorer review session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotReviewReleaseOutcome {
    Released,
    AlreadyReleased,
}

/// One exact expiring review pin and retained immutable snapshot handle.
///
/// This type intentionally exposes no snapshot path, digest, file handle, or
/// decoded tree. Dropping it closes local resources without entering SQLite;
/// callers should explicitly release it when review ends.
#[must_use = "retain the session while Explorer is reviewing the snapshot"]
pub struct SnapshotReviewSession {
    scan_id: ScanId,
    lease: Option<StoredReviewLease>,
}

impl SnapshotReviewSession {
    pub(super) fn new(scan_id: ScanId, lease: StoredReviewLease) -> Self {
        Self {
            scan_id,
            lease: Some(lease),
        }
    }

    pub fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    pub fn expires_at(&self) -> Result<SystemTime, SnapshotReviewError> {
        self.lease
            .as_ref()
            .ok_or(SnapshotReviewError::LeaseExpired)?
            .expires_at()
            .map_err(|error| map_repository_error(error.kind))
    }

    pub fn expires_at_unix_ms(&self) -> Result<i64, SnapshotReviewError> {
        let duration = self
            .expires_at()?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SnapshotReviewError::InternalState)?;
        i64::try_from(duration.as_millis()).map_err(|_| SnapshotReviewError::InternalState)
    }

    /// Renew this exact unexpired lease using the core-owned clock.
    pub fn renew(&mut self) -> Result<SystemTime, SnapshotReviewError> {
        self.renew_at(SystemTime::now())
    }

    pub fn renew_unix_ms(&mut self) -> Result<i64, SnapshotReviewError> {
        self.renew()?;
        self.expires_at_unix_ms()
    }

    pub fn release(&mut self) -> Result<SnapshotReviewReleaseOutcome, SnapshotReviewError> {
        let Some(lease) = self.lease.take() else {
            return Ok(SnapshotReviewReleaseOutcome::AlreadyReleased);
        };
        lease
            .release()
            .map_err(|error| map_repository_error(error.kind))?;
        Ok(SnapshotReviewReleaseOutcome::Released)
    }

    pub fn is_released(&self) -> bool {
        self.lease.is_none()
    }

    fn renew_at(&mut self, observed_at: SystemTime) -> Result<SystemTime, SnapshotReviewError> {
        self.lease
            .as_mut()
            .ok_or(SnapshotReviewError::LeaseExpired)?
            .renew(observed_at)
            .map_err(|error| map_repository_error(error.kind))
    }

    #[cfg(test)]
    pub(super) fn renew_at_for_test(
        &mut self,
        observed_at: SystemTime,
    ) -> Result<SystemTime, SnapshotReviewError> {
        self.renew_at(observed_at)
    }
}

pub(super) const fn map_repository_error(kind: SnapshotRepositoryErrorKind) -> SnapshotReviewError {
    match kind {
        SnapshotRepositoryErrorKind::ReadOnly => SnapshotReviewError::ReadOnlyStore,
        SnapshotRepositoryErrorKind::MissingStore
        | SnapshotRepositoryErrorKind::MissingSnapshot
        | SnapshotRepositoryErrorKind::SnapshotUnavailable
        | SnapshotRepositoryErrorKind::ReferenceMismatch => {
            SnapshotReviewError::SnapshotUnavailable
        }
        SnapshotRepositoryErrorKind::IncompatibleVersion => {
            SnapshotReviewError::IncompatibleSnapshot
        }
        SnapshotRepositoryErrorKind::ReviewLeaseExpired => SnapshotReviewError::LeaseExpired,
        SnapshotRepositoryErrorKind::Codec(kind) => match kind {
            SnapshotCodecErrorKind::LimitExceeded => SnapshotReviewError::BudgetExceeded,
            SnapshotCodecErrorKind::IncompatibleVersion => {
                SnapshotReviewError::IncompatibleSnapshot
            }
            SnapshotCodecErrorKind::Io => SnapshotReviewError::Unavailable,
            SnapshotCodecErrorKind::InvalidMagic
            | SnapshotCodecErrorKind::InvalidLength
            | SnapshotCodecErrorKind::ChecksumMismatch
            | SnapshotCodecErrorKind::CorruptData => SnapshotReviewError::CorruptData,
            SnapshotCodecErrorKind::InvalidInput => SnapshotReviewError::InternalState,
        },
        SnapshotRepositoryErrorKind::Storage(kind) => match kind {
            SnapshotStorageErrorKind::UnsafeRoot
            | SnapshotStorageErrorKind::UnsafeObject
            | SnapshotStorageErrorKind::UnrecognizedStore => SnapshotReviewError::UnsafeStorage,
            SnapshotStorageErrorKind::Busy => SnapshotReviewError::Busy,
            SnapshotStorageErrorKind::Unavailable => SnapshotReviewError::Unavailable,
            SnapshotStorageErrorKind::InvalidConfiguration
            | SnapshotStorageErrorKind::InternalState => SnapshotReviewError::InternalState,
        },
        SnapshotRepositoryErrorKind::History(kind) => match kind {
            HistoryErrorKind::IncompatibleSchema => SnapshotReviewError::IncompatibleSchema,
            HistoryErrorKind::QueryLimitExceeded => SnapshotReviewError::BudgetExceeded,
            HistoryErrorKind::Busy => SnapshotReviewError::Busy,
            HistoryErrorKind::UnsafeStorage => SnapshotReviewError::UnsafeStorage,
            HistoryErrorKind::CorruptData => SnapshotReviewError::CorruptData,
            HistoryErrorKind::DatabaseUnavailable => SnapshotReviewError::Unavailable,
            HistoryErrorKind::OutcomeUnknown => SnapshotReviewError::OutcomeUnknown,
            HistoryErrorKind::NotFound => SnapshotReviewError::ScanNotFound,
            HistoryErrorKind::InvalidInput
            | HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::InvalidTransition
            | HistoryErrorKind::InternalState => SnapshotReviewError::InternalState,
        },
    }
}
