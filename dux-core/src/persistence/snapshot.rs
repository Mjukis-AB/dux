//! Immutable, private, non-authoritative scan snapshots.
//!
//! The legacy CLI cache is intentionally separate. Snapshot bytes may support
//! presentation and discovery, but never path validation or cleanup authority.

use std::cell::Cell;
use std::io::{Seek, SeekFrom};
use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use std::time::SystemTime;

use rusqlite::TransactionBehavior;

use crate::domain::{ScanCoverage, ScanId};

use super::SnapshotReviewPurpose;
use super::candidate_evaluation_history::{
    CandidateEvaluationCompletion, CandidateEvaluationIdentity, NewCandidateEvaluation,
};
use super::history::{
    HistoryError, HistoryErrorKind, ScanCompletionRecord, ScanCounts, ScanStatus,
    system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::process_liveness::{ProcessIdentityError, ProcessInstanceId, current_process_instance};
use super::snapshot_retention::{SnapshotRetentionState, load_snapshot_retention_state};
use super::snapshot_retention_inventory::{
    DEFAULT_SNAPSHOT_RETENTION_CAP_BYTES, SnapshotRetentionInventory,
    build_snapshot_retention_inventory,
};
#[cfg(test)]
use super::snapshot_review_pin::{MAX_ACTIVE_PINS, MAX_EXPIRED_PRUNE};
use super::snapshot_review_pin::{
    MAX_ACTIVE_PINS_PER_OWNER, PreparedSnapshotReviewPin, SNAPSHOT_REVIEW_PIN_ID_ATTEMPTS,
    SnapshotReviewPinId, SnapshotReviewPinState, insert_snapshot_review_pin_candidates,
    inspect_snapshot_review_pin_population, release_snapshot_review_pin, renew_snapshot_review_pin,
    snapshot_review_pin_exactly_matches, snapshot_review_pin_state, validate_snapshot_review_pin,
};
use super::store::{HistoryConnectionGuard, StoreCoordinator};

mod codec;
pub(crate) mod from_scan;
mod storage;

pub(crate) use codec::{
    HostValue, MAX_SNAPSHOT_DEPTH, MAX_SNAPSHOT_NODES, SNAPSHOT_FORMAT_VERSION, SnapshotCodecError,
    SnapshotCodecErrorKind, SnapshotDigest, SnapshotDocument, SnapshotMetadata, SnapshotNode,
    SnapshotNodeKind, SnapshotScanFlags, SnapshotTimestamp, SnapshotTotals, SnapshotUnixIdentity,
    decode_snapshot, encode_snapshot, validate_snapshot_document,
};
pub(crate) use storage::{
    RetainedSnapshot, SecureSnapshotStore, SnapshotFileName, SnapshotFileUsage,
    SnapshotInventoryEntryKind, SnapshotPublication, SnapshotPublicationLease,
    SnapshotStorageError, SnapshotStorageErrorKind, SnapshotStoreAccess,
    SnapshotStoreInventoryLease, StagedSnapshot,
};

const PUBLICATION_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Stable path-free category for snapshot-store startup failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotOpenErrorKind {
    InvalidConfiguration,
    UnsafeRoot,
    UnsafeObject,
    UnrecognizedStore,
    Unavailable,
    Busy,
    InternalState,
}

/// Typed SQLite snapshot tuple. It proves storage syntax, not file validity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotReference {
    scan_id: ScanId,
    version: NonZeroU32,
    file_name: SnapshotFileName,
    digest: SnapshotDigest,
}

impl SnapshotReference {
    pub(super) fn from_stored(
        scan_id: &ScanId,
        version: u32,
        file_name: &str,
        digest: [u8; 32],
    ) -> Result<Self, SnapshotRepositoryError> {
        let version = NonZeroU32::new(version)
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch))?;
        let file_name = SnapshotFileName::parse(file_name).map_err(map_storage)?;
        if file_name != SnapshotFileName::from_scan_id(scan_id.as_str().as_bytes()) {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        Ok(Self {
            scan_id: scan_id.clone(),
            version,
            file_name,
            digest: SnapshotDigest::from_bytes(digest),
        })
    }

    pub(crate) const fn version(&self) -> u32 {
        self.version.get()
    }

    pub(crate) fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    pub(crate) fn file_name(&self) -> &SnapshotFileName {
        &self.file_name
    }

    pub(crate) const fn digest(&self) -> SnapshotDigest {
        self.digest
    }
}

/// Validated immutable publication retained across the following DB commit.
pub(crate) struct PublishedSnapshot {
    reference: SnapshotReference,
    publication: SnapshotPublicationLease,
}

impl PublishedSnapshot {
    pub(crate) fn reference(&self) -> &SnapshotReference {
        &self.reference
    }

    pub(crate) fn revalidate(&self) -> Result<(), SnapshotRepositoryError> {
        self.publication.revalidate().map_err(map_storage)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotRepositoryErrorKind {
    ReadOnly,
    MissingStore,
    MissingSnapshot,
    SnapshotUnavailable,
    IncompatibleVersion,
    ReferenceMismatch,
    #[allow(
        dead_code,
        reason = "review leases are wired to Explorer/FFI in a later milestone slice"
    )]
    ReviewLeaseExpired,
    Codec(SnapshotCodecErrorKind),
    Storage(SnapshotStorageErrorKind),
    History(HistoryErrorKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("snapshot repository operation failed: {kind:?}")]
pub(crate) struct SnapshotRepositoryError {
    pub(crate) kind: SnapshotRepositoryErrorKind,
}

const fn repository_error(kind: SnapshotRepositoryErrorKind) -> SnapshotRepositoryError {
    SnapshotRepositoryError { kind }
}

fn map_codec(error: SnapshotCodecError) -> SnapshotRepositoryError {
    repository_error(SnapshotRepositoryErrorKind::Codec(error.kind))
}

fn map_storage(error: SnapshotStorageError) -> SnapshotRepositoryError {
    repository_error(SnapshotRepositoryErrorKind::Storage(error.kind()))
}

impl SnapshotRepositoryError {
    pub(crate) const fn open_kind(self) -> SnapshotOpenErrorKind {
        match self.kind {
            SnapshotRepositoryErrorKind::Storage(kind) => match kind {
                SnapshotStorageErrorKind::InvalidConfiguration => {
                    SnapshotOpenErrorKind::InvalidConfiguration
                }
                SnapshotStorageErrorKind::UnsafeRoot => SnapshotOpenErrorKind::UnsafeRoot,
                SnapshotStorageErrorKind::UnsafeObject => SnapshotOpenErrorKind::UnsafeObject,
                SnapshotStorageErrorKind::UnrecognizedStore => {
                    SnapshotOpenErrorKind::UnrecognizedStore
                }
                SnapshotStorageErrorKind::Unavailable => SnapshotOpenErrorKind::Unavailable,
                SnapshotStorageErrorKind::Busy => SnapshotOpenErrorKind::Busy,
                SnapshotStorageErrorKind::InternalState => SnapshotOpenErrorKind::InternalState,
            },
            _ => SnapshotOpenErrorKind::InternalState,
        }
    }
}

fn map_history(error: HistoryError) -> SnapshotRepositoryError {
    repository_error(SnapshotRepositoryErrorKind::History(error.kind))
}

#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
fn map_review_history(error: HistoryError) -> SnapshotRepositoryError {
    match error.kind {
        HistoryErrorKind::InvalidTransition => {
            repository_error(SnapshotRepositoryErrorKind::ReviewLeaseExpired)
        }
        HistoryErrorKind::NotFound => {
            repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch)
        }
        _ => map_history(error),
    }
}

fn map_process_identity(error: ProcessIdentityError) -> SnapshotRepositoryError {
    let kind = match error {
        ProcessIdentityError::InvalidEncoding => HistoryErrorKind::CorruptData,
        ProcessIdentityError::ObservationUnavailable | ProcessIdentityError::RandomUnavailable => {
            HistoryErrorKind::InternalState
        }
    };
    repository_error(SnapshotRepositoryErrorKind::History(kind))
}

/// Coordinates bounded codec validation with immutable private publication.
pub(crate) struct SnapshotRepository {
    database: Arc<StoreCoordinator>,
    store: Option<SecureSnapshotStore>,
    access: SnapshotStoreAccess,
    #[allow(
        dead_code,
        reason = "review leases are wired to Explorer/FFI in a later milestone slice"
    )]
    review: Option<SnapshotReviewContext>,
}

#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
struct SnapshotReviewContext {
    owner: OnceLock<ProcessInstanceId>,
    active_slots: Arc<AtomicUsize>,
}

#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
struct SnapshotReviewSlot {
    active_slots: Arc<AtomicUsize>,
}

#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
impl SnapshotReviewContext {
    fn owner(&self) -> Result<ProcessInstanceId, SnapshotRepositoryError> {
        if let Some(owner) = self.owner.get() {
            return Ok(owner.clone());
        }
        let candidate = current_process_instance().map_err(map_process_identity)?;
        let _ = self.owner.set(candidate);
        self.owner.get().cloned().ok_or_else(|| {
            repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::InternalState,
            ))
        })
    }

    fn reserve(&self) -> Result<SnapshotReviewSlot, SnapshotRepositoryError> {
        self.active_slots
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_ACTIVE_PINS_PER_OWNER).then_some(active + 1)
            })
            .map_err(|_| {
                repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::QueryLimitExceeded,
                ))
            })?;
        Ok(SnapshotReviewSlot {
            active_slots: Arc::clone(&self.active_slots),
        })
    }
}

impl Drop for SnapshotReviewSlot {
    fn drop(&mut self) {
        let previous = self.active_slots.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "snapshot review slot underflow");
    }
}

/// One exact, expiring snapshot review pin plus a retained immutable handle.
///
/// The lease is intentionally non-cloneable and not `Sync`. Dropping it never
/// enters SQLite; an explicit release removes the durable row, while crashes
/// and implicit drops remain conservatively pinned until the fixed expiry.
#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
#[must_use = "keep the lease alive during review or explicitly release it"]
pub(crate) struct SnapshotReviewLease {
    database: Arc<StoreCoordinator>,
    retained: RetainedSnapshot,
    reference: SnapshotReference,
    pin: PreparedSnapshotReviewPin,
    _slot: SnapshotReviewSlot,
    _not_sync: PhantomData<Cell<()>>,
    #[cfg(test)]
    test_fault: Cell<SnapshotReviewTestFault>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SnapshotReviewTestFault {
    None,
    RenewAfterCommit,
    ReleaseAfterCommit,
    ReleaseAfterCommitConflicting,
}

impl SnapshotRepository {
    pub(crate) fn open(
        database: Arc<StoreCoordinator>,
        access: SnapshotStoreAccess,
    ) -> Result<Self, SnapshotRepositoryError> {
        let review = match access {
            SnapshotStoreAccess::ReadWrite => Some(SnapshotReviewContext {
                owner: OnceLock::new(),
                active_slots: Arc::new(AtomicUsize::new(0)),
            }),
            SnapshotStoreAccess::ReadOnly => None,
        };
        let store = match access {
            SnapshotStoreAccess::ReadWrite => {
                let _database_guard = database
                    .lock_current_history_connection()
                    .map_err(map_history)?;
                let database_path = database.validated_database_path().map_err(map_history)?;
                SecureSnapshotStore::open_for_database(&database_path, access)
                    .map_err(map_storage)?
            }
            SnapshotStoreAccess::ReadOnly => {
                let database_path = database.validated_database_path().map_err(map_history)?;
                SecureSnapshotStore::open_for_database(&database_path, access)
                    .map_err(map_storage)?
            }
        };
        Ok(Self {
            database,
            store,
            access,
            review,
        })
    }

    /// Reconcile the complete bounded physical snapshot store with exact
    /// current-schema history and review-pin facts.
    ///
    /// The returned value is metadata-only and cannot authorize a tombstone or
    /// unlink. Read-only/newer-schema repositories are rejected because this
    /// is a retention-maintenance prerequisite, not a generic history query.
    #[allow(
        dead_code,
        reason = "retention maintenance consumes the sealed inventory in the next slice"
    )]
    pub(crate) fn inspect_retention_inventory(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotRetentionInventory, SnapshotRepositoryError> {
        self.inspect_retention_inventory_with_cap(observed_at, DEFAULT_SNAPSHOT_RETENTION_CAP_BYTES)
    }

    fn inspect_retention_inventory_with_cap(
        &self,
        observed_at: SystemTime,
        cap_bytes: u64,
    ) -> Result<SnapshotRetentionInventory, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        // Lock order is permanently database -> snapshot. The inventory lease
        // captures each exact final/temp identity and usage sequentially while
        // keeping the store-wide exclusion live until SQLite reconciliation;
        // every name is reopened and revalidated before handoff.
        let storage = store
            .inventory_with_writer_lease(PUBLICATION_LOCK_TIMEOUT)
            .map_err(map_storage)?;
        let pins = inspect_snapshot_review_pin_population(&database_guard.connection, observed_at)
            .map_err(map_history)?;
        let inventory = build_snapshot_retention_inventory(
            &database_guard.connection,
            &storage,
            pins,
            observed_at,
            cap_bytes,
        )
        .map_err(map_history)?;
        storage.revalidate().map_err(map_storage)?;
        self.database
            .revalidate_current_history_guard(&database_guard)
            .map_err(map_history)?;
        Ok(inventory)
    }

    /// Acquire an explicit cross-process lease for one snapshot-backed UI
    /// review. Candidate and cleanup-session state deliberately do not call
    /// this implicitly.
    #[allow(
        dead_code,
        reason = "review leases are wired to Explorer/FFI in a later milestone slice"
    )]
    pub(crate) fn acquire_review_lease(
        &self,
        reference: &SnapshotReference,
        purpose: SnapshotReviewPurpose,
        observed_at: SystemTime,
    ) -> Result<SnapshotReviewLease, SnapshotRepositoryError> {
        self.acquire_review_lease_with_hook(reference, purpose, observed_at, || Ok(()))
    }

    #[cfg(test)]
    fn acquire_review_lease_after_commit_failure_for_test(
        &self,
        reference: &SnapshotReference,
        purpose: SnapshotReviewPurpose,
        observed_at: SystemTime,
    ) -> Result<SnapshotReviewLease, SnapshotRepositoryError> {
        self.acquire_review_lease_with_hook(reference, purpose, observed_at, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    fn acquire_review_lease_with_hook(
        &self,
        reference: &SnapshotReference,
        purpose: SnapshotReviewPurpose,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<SnapshotReviewLease, SnapshotRepositoryError> {
        self.acquire_review_lease_with_pin_id_source(
            reference,
            purpose,
            observed_at,
            || {
                (0..SNAPSHOT_REVIEW_PIN_ID_ATTEMPTS)
                    .map(|_| SnapshotReviewPinId::random())
                    .collect::<Result<Vec<_>, _>>()
            },
            after_commit,
        )
    }

    #[cfg(test)]
    fn acquire_review_lease_with_pin_ids_for_test(
        &self,
        reference: &SnapshotReference,
        purpose: SnapshotReviewPurpose,
        observed_at: SystemTime,
        pin_ids: Vec<SnapshotReviewPinId>,
    ) -> Result<SnapshotReviewLease, SnapshotRepositoryError> {
        self.acquire_review_lease_with_pin_id_source(
            reference,
            purpose,
            observed_at,
            || Ok(pin_ids),
            || Ok(()),
        )
    }

    fn acquire_review_lease_with_pin_id_source(
        &self,
        reference: &SnapshotReference,
        purpose: SnapshotReviewPurpose,
        observed_at: SystemTime,
        pin_id_source: impl FnOnce() -> Result<Vec<SnapshotReviewPinId>, HistoryError>,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<SnapshotReviewLease, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        if reference.version() != SNAPSHOT_FORMAT_VERSION {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::IncompatibleVersion,
            ));
        }
        let context = self
            .review
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::ReadOnly))?;
        let owner = context.owner()?;
        let slot = context.reserve()?;
        // Generate fallible process-local material before either persistence
        // lock. The stable owner is created once with the repository.
        let pin_ids = pin_id_source().map_err(map_history)?;

        let mut database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let scan = self
            .database
            .load_scan_with_guard(&database_guard, reference.scan_id())
            .map_err(map_history)?
            .ok_or_else(|| {
                repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::NotFound,
                ))
            })?;
        let completed_at = scan
            .completed_at()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch))?;
        if scan.status() != ScanStatus::Succeeded || scan.snapshot() != Some(reference) {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        match load_snapshot_retention_state(&database_guard.connection, reference)
            .map_err(map_history)?
        {
            SnapshotRetentionState::Available => {}
            SnapshotRetentionState::Tombstoned { .. } => {
                return Err(repository_error(
                    SnapshotRepositoryErrorKind::SnapshotUnavailable,
                ));
            }
        }

        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        let opened = store
            .open_with_writer_lease(reference.file_name(), PUBLICATION_LOCK_TIMEOUT)
            .map_err(map_storage)?
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingSnapshot))?;
        let transaction = database_guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(super::history::map_write_sql_error)
            .map_err(map_history)?;
        let pins = pin_ids
            .into_iter()
            .map(|pin_id| {
                PreparedSnapshotReviewPin::prepare(
                    pin_id,
                    reference,
                    completed_at,
                    owner.clone(),
                    purpose,
                    observed_at,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_history)?;
        let pin = insert_snapshot_review_pin_candidates(&transaction, &pins)
            .map_err(map_history)?
            .ok_or_else(|| {
                repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::InternalState,
                ))
            })?;
        let write = transaction
            .commit()
            .map_err(super::history::map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| {
                self.database
                    .revalidate_current_history_guard(&database_guard)
            });
        if let Err(failure) = write
            && !reconcile_review_pin(&self.database, &database_guard, &pin)?
        {
            return Err(map_history(failure));
        }
        opened.retained().revalidate().map_err(map_storage)?;
        let retained = opened.into_retained();
        drop(database_guard);
        Ok(SnapshotReviewLease {
            database: Arc::clone(&self.database),
            retained,
            reference: reference.clone(),
            pin,
            _slot: slot,
            _not_sync: PhantomData,
            #[cfg(test)]
            test_fault: Cell::new(SnapshotReviewTestFault::None),
        })
    }

    fn stage_document(
        &self,
        document: &SnapshotDocument,
    ) -> Result<(StagedSnapshot, SnapshotDigest), SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        let file_name =
            SnapshotFileName::from_scan_id(document.metadata.scan_id.as_str().as_bytes());
        let mut staged = {
            let _database_guard = self
                .database
                .lock_current_history_connection()
                .map_err(map_history)?;
            store
                .stage(file_name, PUBLICATION_LOCK_TIMEOUT)
                .map_err(map_storage)?
        };
        let expected_digest = match encode_snapshot(document, &mut staged) {
            Ok(digest) => digest,
            Err(error) => {
                let database_guard = match self.database.lock_current_history_connection() {
                    Ok(guard) => guard,
                    Err(history) => {
                        staged.abandon();
                        return Err(map_history(history));
                    }
                };
                staged.abort().map_err(map_storage)?;
                drop(database_guard);
                return Err(map_codec(error));
            }
        };
        Ok((staged, expected_digest))
    }

    fn publish_staged(
        &self,
        database_guard: &HistoryConnectionGuard<'_>,
        staged: StagedSnapshot,
        document: &SnapshotDocument,
        expected_digest: SnapshotDigest,
    ) -> Result<PublishedSnapshot, SnapshotRepositoryError> {
        self.database
            .validate_history_guard(database_guard)
            .map_err(map_history)?;
        let file_name =
            SnapshotFileName::from_scan_id(document.metadata.scan_id.as_str().as_bytes());
        let publication = match staged.publish_no_replace().map_err(map_storage)? {
            SnapshotPublication::Published(publication)
            | SnapshotPublication::Existing(publication) => publication,
        };
        validate_retained_document(publication.retained(), document, expected_digest)?;
        let reference = SnapshotReference {
            scan_id: document.metadata.scan_id.clone(),
            version: NonZeroU32::new(SNAPSHOT_FORMAT_VERSION)
                .expect("snapshot format version is nonzero"),
            file_name,
            digest: expected_digest,
        };
        Ok(PublishedSnapshot {
            reference,
            publication,
        })
    }

    #[cfg(test)]
    fn publish_orphan_for_test(
        &self,
        document: &SnapshotDocument,
    ) -> Result<PublishedSnapshot, SnapshotRepositoryError> {
        let (staged, expected_digest) = self.stage_document(document)?;
        let database_guard = match self.database.lock_current_history_connection() {
            Ok(guard) => guard,
            Err(error) => {
                staged.abandon();
                return Err(map_history(error));
            }
        };
        self.publish_staged(&database_guard, staged, document, expected_digest)
    }

    pub(crate) fn load(
        &self,
        reference: &SnapshotReference,
    ) -> Result<SnapshotDocument, SnapshotRepositoryError> {
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let retained = self.open_available_with_guard(&database_guard, reference)?;
        drop(database_guard);
        decode_reference(&retained, reference)
    }

    fn load_with_guard(
        &self,
        database_guard: &HistoryConnectionGuard<'_>,
        reference: &SnapshotReference,
    ) -> Result<SnapshotDocument, SnapshotRepositoryError> {
        let retained = self.open_available_with_guard(database_guard, reference)?;
        decode_reference(&retained, reference)
    }

    /// Check logical availability under the database fence before acquiring a
    /// retained file handle. Future retention must take the same database-first
    /// order before its snapshot writer lock and unlink.
    fn open_available_with_guard(
        &self,
        database_guard: &HistoryConnectionGuard<'_>,
        reference: &SnapshotReference,
    ) -> Result<RetainedSnapshot, SnapshotRepositoryError> {
        if reference.version() != SNAPSHOT_FORMAT_VERSION {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::IncompatibleVersion,
            ));
        }
        self.database
            .validate_history_guard(database_guard)
            .map_err(map_history)?;
        match load_snapshot_retention_state(&database_guard.connection, reference)
            .map_err(map_history)?
        {
            SnapshotRetentionState::Available => {}
            SnapshotRetentionState::Tombstoned { .. } => {
                return Err(repository_error(
                    SnapshotRepositoryErrorKind::SnapshotUnavailable,
                ));
            }
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        store
            .open(reference.file_name())
            .map_err(map_storage)?
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingSnapshot))
    }

    /// Publish the immutable file before exact-CASing the durable scan summary.
    /// A published file without a DB row is a harmless retention orphan; the
    /// reverse ordering is forbidden.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "preserved for callers that explicitly opt out of candidate evaluation"
        )
    )]
    pub(crate) fn complete_scan(
        &self,
        completed_at: SystemTime,
        counts: ScanCounts,
        coverage: &ScanCoverage,
        document: &SnapshotDocument,
    ) -> Result<SnapshotReference, SnapshotRepositoryError> {
        self.complete_scan_inner(completed_at, counts, coverage, document, None)
    }

    /// Publish a snapshot and atomically commit the succeeded scan plus the
    /// evaluator's complete terminal discovery output.
    pub(crate) fn complete_scan_with_candidate_evaluation(
        &self,
        completed_at: SystemTime,
        counts: ScanCounts,
        coverage: &ScanCoverage,
        document: &SnapshotDocument,
        identity: &CandidateEvaluationIdentity,
        evaluation: &CandidateEvaluationCompletion,
    ) -> Result<SnapshotReference, SnapshotRepositoryError> {
        self.complete_scan_inner(
            completed_at,
            counts,
            coverage,
            document,
            Some((identity, evaluation)),
        )
    }

    fn complete_scan_inner(
        &self,
        completed_at: SystemTime,
        counts: ScanCounts,
        coverage: &ScanCoverage,
        document: &SnapshotDocument,
        evaluation: Option<(&CandidateEvaluationIdentity, &CandidateEvaluationCompletion)>,
    ) -> Result<SnapshotReference, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
        counts.validate_for_storage().map_err(map_history)?;
        if coverage.status() == crate::ScanCoverageStatus::Unknown {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        if counts.directory_count != document.metadata.totals.directory_count
            || counts.file_count != document.metadata.totals.file_count
            || counts.logical_bytes != document.metadata.totals.logical_bytes
            || counts.allocated_bytes != document.metadata.totals.allocated_bytes
        {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        if let Some((_, terminal)) = evaluation {
            terminal
                .validate_for_scan(&document.metadata.scan_id, completed_at)
                .map_err(map_history)?;
        }
        let current = self
            .database
            .load_scan(&document.metadata.scan_id)
            .map_err(map_history)?
            .ok_or_else(|| {
                repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::NotFound,
                ))
            })?;
        if !document.metadata.root.matches_path(current.root()) {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        if coverage
            .issues()
            .iter()
            .filter_map(|issue| issue.path())
            .any(|path| !path.starts_with(current.root()))
        {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }

        if current.status() == ScanStatus::Succeeded {
            let reference = current
                .snapshot()
                .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch))?;
            let completion = ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
                document.metadata.scan_id.clone(),
                completed_at,
                counts,
                coverage.clone(),
                reference.clone(),
            )
            .map_err(map_history)?;
            if !current.exactly_matches_completion(&completion)
                || &self.load(reference)? != document
            {
                return Err(repository_error(
                    SnapshotRepositoryErrorKind::ReferenceMismatch,
                ));
            }
            if let Some((identity, terminal)) = evaluation {
                let request =
                    NewCandidateEvaluation::try_new(identity.clone(), reference, completed_at)
                        .map_err(map_history)?;
                let stored = self
                    .database
                    .load_candidate_evaluation(request.scan_id())
                    .map_err(map_history)?
                    .ok_or_else(|| {
                        repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch)
                    })?;
                if !terminal.exactly_matches_record(&stored, &request) {
                    return Err(repository_error(
                        SnapshotRepositoryErrorKind::ReferenceMismatch,
                    ));
                }
            }
            return Ok(reference.clone());
        }
        if current.status() != ScanStatus::Running || current.snapshot().is_some() {
            return Err(repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::InvalidTransition,
            )));
        }

        let (staged, expected_digest) = self.stage_document(document)?;
        let mut database_guard = match self.database.lock_current_history_connection() {
            Ok(guard) => guard,
            Err(error) => {
                staged.abandon();
                return Err(map_history(error));
            }
        };
        let loaded = match self
            .database
            .load_scan_with_guard(&database_guard, &document.metadata.scan_id)
        {
            Ok(loaded) => loaded,
            Err(error) => {
                staged.abort().map_err(map_storage)?;
                return Err(map_history(error));
            }
        };
        let current = match loaded {
            Some(current) => current,
            None => {
                staged.abort().map_err(map_storage)?;
                return Err(repository_error(SnapshotRepositoryErrorKind::History(
                    HistoryErrorKind::NotFound,
                )));
            }
        };
        if !document.metadata.root.matches_path(current.root()) {
            staged.abort().map_err(map_storage)?;
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        if current.status() == ScanStatus::Succeeded {
            staged.abort().map_err(map_storage)?;
            let reference = current
                .snapshot()
                .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch))?;
            let completion = ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
                document.metadata.scan_id.clone(),
                completed_at,
                counts,
                coverage.clone(),
                reference.clone(),
            )
            .map_err(map_history)?;
            if !current.exactly_matches_completion(&completion)
                || &self.load_with_guard(&database_guard, reference)? != document
            {
                return Err(repository_error(
                    SnapshotRepositoryErrorKind::ReferenceMismatch,
                ));
            }
            if let Some((identity, terminal)) = evaluation {
                let request =
                    NewCandidateEvaluation::try_new(identity.clone(), reference, completed_at)
                        .map_err(map_history)?;
                let stored = self
                    .database
                    .load_candidate_evaluation_with_guard(&database_guard, request.scan_id())
                    .map_err(map_history)?
                    .ok_or_else(|| {
                        repository_error(SnapshotRepositoryErrorKind::ReferenceMismatch)
                    })?;
                if !terminal.exactly_matches_record(&stored, &request) {
                    return Err(repository_error(
                        SnapshotRepositoryErrorKind::ReferenceMismatch,
                    ));
                }
            }
            return Ok(reference.clone());
        }
        if current.status() != ScanStatus::Running || current.snapshot().is_some() {
            staged.abort().map_err(map_storage)?;
            return Err(repository_error(SnapshotRepositoryErrorKind::History(
                HistoryErrorKind::InvalidTransition,
            )));
        }

        let published = self.publish_staged(&database_guard, staged, document, expected_digest)?;
        let completion = ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
            document.metadata.scan_id.clone(),
            completed_at,
            counts,
            coverage.clone(),
            published.reference().clone(),
        )
        .map_err(map_history)?;
        if let Some((identity, terminal)) = evaluation {
            let request = NewCandidateEvaluation::try_new(
                identity.clone(),
                published.reference(),
                completed_at,
            )
            .map_err(map_history)?;
            self.database
                .record_scan_finished_with_evaluation_reconciled_with_guard(
                    &mut database_guard,
                    &completion,
                    &request,
                    terminal,
                )
                .map_err(map_history)?;
        } else {
            self.database
                .record_scan_finished_reconciled_with_guard(&mut database_guard, &completion)
                .map_err(map_history)?;
        }
        published.revalidate()?;
        Ok(published.reference().clone())
    }
}

#[allow(
    dead_code,
    reason = "review leases are wired to Explorer/FFI in a later milestone slice"
)]
impl SnapshotReviewLease {
    #[cfg(test)]
    fn fail_next_renew_after_commit_for_test(&self) {
        assert_eq!(self.test_fault.get(), SnapshotReviewTestFault::None);
        self.test_fault
            .set(SnapshotReviewTestFault::RenewAfterCommit);
    }

    #[cfg(test)]
    fn fail_release_after_commit_for_test(&self) {
        assert_eq!(self.test_fault.get(), SnapshotReviewTestFault::None);
        self.test_fault
            .set(SnapshotReviewTestFault::ReleaseAfterCommit);
    }

    #[cfg(test)]
    fn replace_release_with_conflict_after_commit_for_test(&self) {
        assert_eq!(self.test_fault.get(), SnapshotReviewTestFault::None);
        self.test_fault
            .set(SnapshotReviewTestFault::ReleaseAfterCommitConflicting);
    }

    pub(crate) fn reference(&self) -> &SnapshotReference {
        &self.reference
    }

    pub(crate) fn purpose(&self) -> SnapshotReviewPurpose {
        self.pin.purpose()
    }

    pub(crate) fn expires_at(&self) -> Result<SystemTime, SnapshotRepositoryError> {
        unix_ms_to_system_time(self.pin.expires_at_unix_ms()).map_err(map_history)
    }

    /// Decode through the already-retained immutable handle after proving the
    /// durable lease is still exact and unexpired. SQLite is released before
    /// the potentially large decode.
    pub(crate) fn load(
        &self,
        observed_at: SystemTime,
    ) -> Result<SnapshotDocument, SnapshotRepositoryError> {
        self.ensure_unexpired(observed_at)?;
        let database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        validate_snapshot_review_pin(&database_guard.connection, &self.pin, observed_at)
            .map_err(map_review_history)?;
        self.retained.revalidate().map_err(map_storage)?;
        drop(database_guard);
        decode_reference(&self.retained, &self.reference)
    }

    /// Extend this exact live lease by the fixed duration. Expired leases are
    /// never resurrected; callers must reacquire through the repository.
    pub(crate) fn renew(
        &mut self,
        observed_at: SystemTime,
    ) -> Result<SystemTime, SnapshotRepositoryError> {
        self.ensure_unexpired(observed_at)?;
        let mut database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let transaction = database_guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(super::history::map_write_sql_error)
            .map_err(map_history)?;
        let renewed = renew_snapshot_review_pin(&transaction, &self.pin, observed_at)
            .map_err(map_review_history)?;
        let write = transaction
            .commit()
            .map_err(super::history::map_write_sql_error)
            .and_then(|()| self.after_renew_commit())
            .and_then(|()| {
                self.database
                    .revalidate_current_history_guard(&database_guard)
            });
        if let Err(failure) = write {
            revalidate_review_reconciliation(&self.database, &database_guard)?;
            if snapshot_review_pin_exactly_matches(&database_guard.connection, &renewed)
                .map_err(|_| outcome_unknown())?
            {
                // The exact post-renewal tuple is the only state this call can
                // have committed.
            } else if snapshot_review_pin_exactly_matches(&database_guard.connection, &self.pin)
                .map_err(|_| outcome_unknown())?
            {
                return Err(map_history(failure));
            } else {
                return Err(outcome_unknown());
            }
        }
        self.pin = renewed;
        self.expires_at()
    }

    /// Explicitly remove the exact durable pin. Consuming the lease guarantees
    /// the retained handle and local slot are closed on both success and
    /// failure. An unresolved row remains safe and expires naturally.
    pub(crate) fn release(self) -> Result<(), SnapshotRepositoryError> {
        let mut database_guard = self
            .database
            .lock_current_history_connection()
            .map_err(map_history)?;
        let transaction = database_guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(super::history::map_write_sql_error)
            .map_err(map_history)?;
        release_snapshot_review_pin(&transaction, &self.pin).map_err(map_review_history)?;
        let write = transaction
            .commit()
            .map_err(super::history::map_write_sql_error)
            .and_then(|()| self.after_release_commit(&database_guard.connection))
            .and_then(|()| {
                self.database
                    .revalidate_current_history_guard(&database_guard)
            });
        if let Err(failure) = write {
            revalidate_review_reconciliation(&self.database, &database_guard)?;
            match snapshot_review_pin_state(&database_guard.connection, &self.pin)
                .map_err(|_| outcome_unknown())?
            {
                SnapshotReviewPinState::Missing => {}
                SnapshotReviewPinState::Exact => return Err(map_history(failure)),
                SnapshotReviewPinState::Conflicting => return Err(outcome_unknown()),
            }
        }
        Ok(())
    }

    fn after_renew_commit(&self) -> Result<(), HistoryError> {
        #[cfg(test)]
        if self.test_fault.replace(SnapshotReviewTestFault::None)
            == SnapshotReviewTestFault::RenewAfterCommit
        {
            return Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable));
        }
        Ok(())
    }

    fn ensure_unexpired(&self, observed_at: SystemTime) -> Result<(), SnapshotRepositoryError> {
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)
                .map_err(map_history)?;
        if observed_at_unix_ms >= self.pin.expires_at_unix_ms() {
            Err(repository_error(
                SnapshotRepositoryErrorKind::ReviewLeaseExpired,
            ))
        } else {
            Ok(())
        }
    }

    fn after_release_commit(&self, connection: &rusqlite::Connection) -> Result<(), HistoryError> {
        #[cfg(test)]
        match self.test_fault.replace(SnapshotReviewTestFault::None) {
            SnapshotReviewTestFault::ReleaseAfterCommit => {
                return Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable));
            }
            SnapshotReviewTestFault::ReleaseAfterCommitConflicting => {
                let conflicting_purpose = match self.pin.purpose() {
                    SnapshotReviewPurpose::Explorer => "cleanup_review",
                    SnapshotReviewPurpose::CleanupReview => "explorer",
                };
                connection
                    .execute(
                        "INSERT INTO snapshot_review_pins (
                             pin_id, record_format_version, scan_id, scan_status,
                             completed_at_unix_ms, snapshot_version,
                             snapshot_relative_path, snapshot_relative_path_encoding,
                             snapshot_checksum_sha256, owner_process_instance, purpose,
                             created_at_unix_ms, renewed_at_unix_ms, expires_at_unix_ms
                         )
                         SELECT ?2, 1, scan_id, status, completed_at_unix_ms,
                                snapshot_version, snapshot_relative_path,
                                snapshot_relative_path_encoding,
                                snapshot_checksum_sha256, ?3, ?4, ?5, ?6, ?7
                         FROM scans WHERE scan_id = ?1",
                        rusqlite::params![
                            self.reference.scan_id().as_str(),
                            self.pin.id().as_str(),
                            self.pin.owner().as_str(),
                            conflicting_purpose,
                            self.pin.created_at_unix_ms(),
                            self.pin.renewed_at_unix_ms(),
                            self.pin.expires_at_unix_ms(),
                        ],
                    )
                    .map_err(super::history::map_write_sql_error)?;
                return Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable));
            }
            SnapshotReviewTestFault::None | SnapshotReviewTestFault::RenewAfterCommit => {}
        }
        #[cfg(not(test))]
        let _ = connection;
        Ok(())
    }
}

fn reconcile_review_pin(
    database: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    expected: &PreparedSnapshotReviewPin,
) -> Result<bool, SnapshotRepositoryError> {
    revalidate_review_reconciliation(database, guard)?;
    match snapshot_review_pin_state(&guard.connection, expected).map_err(|_| outcome_unknown())? {
        SnapshotReviewPinState::Missing => Ok(false),
        SnapshotReviewPinState::Exact => Ok(true),
        SnapshotReviewPinState::Conflicting => Err(outcome_unknown()),
    }
}

fn revalidate_review_reconciliation(
    database: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
) -> Result<(), SnapshotRepositoryError> {
    database
        .revalidate_current_history_guard(guard)
        .map_err(|_| outcome_unknown())
}

const fn outcome_unknown() -> SnapshotRepositoryError {
    repository_error(SnapshotRepositoryErrorKind::History(
        HistoryErrorKind::OutcomeUnknown,
    ))
}

fn validate_retained_document(
    retained: &RetainedSnapshot,
    expected: &SnapshotDocument,
    expected_digest: SnapshotDigest,
) -> Result<(), SnapshotRepositoryError> {
    let (document, digest) = decode_retained(retained)?;
    if &document != expected || digest != expected_digest {
        return Err(repository_error(
            SnapshotRepositoryErrorKind::ReferenceMismatch,
        ));
    }
    Ok(())
}

fn decode_retained(
    retained: &RetainedSnapshot,
) -> Result<(SnapshotDocument, SnapshotDigest), SnapshotRepositoryError> {
    let length = retained.len().map_err(map_storage)?;
    if length > codec::MAX_SNAPSHOT_FILE_BYTES {
        return Err(repository_error(SnapshotRepositoryErrorKind::Codec(
            SnapshotCodecErrorKind::LimitExceeded,
        )));
    }
    let mut file = retained.try_clone_file().map_err(map_storage)?;
    file.seek(SeekFrom::Start(0)).map_err(|_| {
        repository_error(SnapshotRepositoryErrorKind::Codec(
            SnapshotCodecErrorKind::Io,
        ))
    })?;
    let decoded = decode_snapshot(&mut file).map_err(map_codec)?;
    retained.revalidate().map_err(map_storage)?;
    Ok(decoded)
}

fn decode_reference(
    retained: &RetainedSnapshot,
    reference: &SnapshotReference,
) -> Result<SnapshotDocument, SnapshotRepositoryError> {
    let (document, digest) = decode_retained(retained)?;
    if digest != reference.digest() || &document.metadata.scan_id != reference.scan_id() {
        return Err(repository_error(
            SnapshotRepositoryErrorKind::ReferenceMismatch,
        ));
    }
    Ok(document)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::{
        BlockReason, Candidate, CandidateAction, CandidateCategory, CandidateId, CandidateInput,
        Evidence, LocalizedTextKey, ProvenanceUrl, Rule, RuleDefinition, RuleGuards, RuleId,
        RuleMatcher, RuleMatcherDefinition, RuleRef, RuleRevision, RuleScope, SafetyTier,
    };
    use crate::persistence::candidate_history::{NewCandidateRecord, StoredCandidateRecord};
    use crate::persistence::history::NewScanRecord;
    use crate::{CoveragePermille, ScanIssue, ScanIssueKind};

    fn document(scan_id: &str, root: &Path) -> SnapshotDocument {
        SnapshotDocument {
            metadata: SnapshotMetadata {
                scan_id: ScanId::new(scan_id).unwrap(),
                root: HostValue::from_root(root).unwrap(),
                captured_at: SnapshotTimestamp::new(1_750_000_000, 123).unwrap(),
                totals: SnapshotTotals {
                    directory_count: 1,
                    file_count: 1,
                    logical_bytes: 10,
                    allocated_bytes: Some(16),
                },
            },
            nodes: vec![
                SnapshotNode {
                    id: 0,
                    parent: None,
                    depth: 0,
                    kind: SnapshotNodeKind::Directory,
                    name: None,
                    logical_bytes: 10,
                    allocated_bytes: Some(16),
                    file_count: 1,
                    child_count: 1,
                    modified_at: None,
                    accessed_at: None,
                    scan_flags: SnapshotScanFlags::NONE,
                    unix_identity: if cfg!(unix) {
                        Some(SnapshotUnixIdentity::new(7, 10))
                    } else {
                        None
                    },
                },
                SnapshotNode {
                    id: 1,
                    parent: Some(0),
                    depth: 1,
                    kind: SnapshotNodeKind::File,
                    name: Some(HostValue::from_component(OsStr::new("artifact.o")).unwrap()),
                    logical_bytes: 10,
                    allocated_bytes: Some(16),
                    file_count: 1,
                    child_count: 0,
                    modified_at: None,
                    accessed_at: None,
                    scan_flags: SnapshotScanFlags::NONE,
                    unix_identity: if cfg!(unix) {
                        Some(SnapshotUnixIdentity::new(7, 11))
                    } else {
                        None
                    },
                },
            ],
        }
    }

    fn counts() -> ScanCounts {
        ScanCounts {
            directory_count: 1,
            file_count: 1,
            logical_bytes: 10,
            allocated_bytes: Some(16),
        }
    }

    fn evaluation_identity(seed: u8) -> CandidateEvaluationIdentity {
        CandidateEvaluationIdentity::try_new(1, 1, [seed; 32], 1, [seed.wrapping_add(1); 32])
            .unwrap()
    }

    fn candidate(scan_id: &ScanId, id: &str, path: &Path) -> NewCandidateRecord {
        let rule = Rule::try_new(RuleDefinition {
            reference: RuleRef::new(
                RuleId::new("fixture.snapshot-evaluation").unwrap(),
                RuleRevision::new(1).unwrap(),
            ),
            title_key: LocalizedTextKey::new("fixture.snapshot-evaluation.title").unwrap(),
            category: CandidateCategory::DeveloperArtifact,
            scope: RuleScope::SelectedScanRoot,
            matcher: RuleMatcher::try_new(RuleMatcherDefinition {
                path_component: Some("artifact.o".to_owned()),
                required_ancestor_markers_any: Vec::new(),
                required_markers_all: Vec::new(),
                forbidden_markers_any: Vec::new(),
                exact_bundle_identifiers: Vec::new(),
                excluded_descendants: Vec::new(),
                protected_descendants: Vec::new(),
            })
            .unwrap(),
            guards: RuleGuards::try_new(None, 0, Vec::new(), false).unwrap(),
            safety: SafetyTier::Informational,
            action: CandidateAction::RevealOnly,
            schedule_eligible: false,
            explanation_key: LocalizedTextKey::new("fixture.snapshot-evaluation.explanation")
                .unwrap(),
            provenance: vec![ProvenanceUrl::new("https://example.com/dux-fixture").unwrap()],
        })
        .unwrap();
        let candidate = Candidate::try_from_rule(
            &rule,
            CandidateInput::new(
                CandidateId::new(id).unwrap(),
                vec![path.to_path_buf()],
                10,
                None,
                vec![Evidence::MatchedPath {
                    path: path.to_path_buf(),
                }],
                vec![BlockReason::ProtectedPath],
                scan_id.clone(),
            ),
        )
        .unwrap();
        NewCandidateRecord::try_from_candidate(
            &candidate,
            UNIX_EPOCH + Duration::from_millis(1_750_000_002_500),
        )
        .unwrap()
    }

    fn complete_coverage() -> ScanCoverage {
        ScanCoverage::try_from_terminal(None, Vec::new()).unwrap()
    }

    fn partial_coverage(root: &Path) -> ScanCoverage {
        ScanCoverage::try_from_terminal(
            Some(CoveragePermille::new(700).unwrap()),
            vec![
                ScanIssue::try_new(
                    ScanIssueKind::MetadataError,
                    Some(root.join("artifact.o")),
                    2,
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn counts_for(document: &SnapshotDocument) -> ScanCounts {
        ScanCounts {
            directory_count: document.metadata.totals.directory_count,
            file_count: document.metadata.totals.file_count,
            logical_bytes: document.metadata.totals.logical_bytes,
            allocated_bytes: document.metadata.totals.allocated_bytes,
        }
    }

    fn open_repository(database: &Path) -> (Arc<StoreCoordinator>, SnapshotRepository) {
        let store = StoreCoordinator::open(database).unwrap();
        let repository =
            SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadWrite).unwrap();
        (store, repository)
    }

    fn complete_snapshot(
        store: &StoreCoordinator,
        repository: &SnapshotRepository,
        document: &SnapshotDocument,
        root: &Path,
        completed_at: SystemTime,
    ) -> SnapshotReference {
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root.to_path_buf(),
                    completed_at - Duration::from_secs(2),
                )
                .unwrap(),
            )
            .unwrap();
        repository
            .complete_scan(
                completed_at,
                counts_for(document),
                &complete_coverage(),
                document,
            )
            .unwrap()
    }

    fn review_pin_count(store: &StoreCoordinator) -> i64 {
        store.with_connection(|connection| {
            connection
                .query_row("SELECT count(*) FROM snapshot_review_pins", [], |row| {
                    row.get(0)
                })
                .unwrap()
        })
    }

    #[test]
    fn retention_inventory_reconciles_policy_storage_and_pin_observations() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let base = UNIX_EPOCH + Duration::from_millis(1_750_000_000_000);
        let (store, repository) = open_repository(&database);

        let mut references = Vec::new();
        for (ordinal, scan_id) in [
            "scan:retention-1",
            "scan:retention-2",
            "scan:retention-3",
            "scan:retention-4",
            "scan:retention-retired",
        ]
        .into_iter()
        .enumerate()
        {
            let completed_at = base + Duration::from_secs((ordinal + 1) as u64 * 10);
            let document = document(scan_id, &root);
            references.push(complete_snapshot(
                &store,
                &repository,
                &document,
                &root,
                completed_at,
            ));
        }

        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO snapshot_retention_tombstones (
                         scan_id, record_format_version, scan_status,
                         completed_at_unix_ms, snapshot_version,
                         snapshot_relative_path, snapshot_relative_path_encoding,
                         snapshot_checksum_sha256, committed_at_unix_ms
                     )
                     SELECT scan_id, 1, status, completed_at_unix_ms,
                            snapshot_version, snapshot_relative_path,
                            snapshot_relative_path_encoding,
                            snapshot_checksum_sha256, completed_at_unix_ms + 1
                     FROM scans WHERE scan_id = ?1",
                    [references[4].scan_id().as_str()],
                )
                .unwrap();
        });

        let orphan_document = document("scan:retention-orphan", &root);
        drop(
            repository
                .publish_orphan_for_test(&orphan_document)
                .unwrap(),
        );
        let temp_document = document("scan:retention-live-temp", &root);
        let (staged, _) = repository.stage_document(&temp_document).unwrap();
        staged.abandon();

        let observed_at = base + Duration::from_secs(60);
        let pinned = repository
            .acquire_review_lease(&references[0], SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        let expires_at = pinned.expires_at().unwrap();
        let before_rows = store.with_connection(|connection| {
            (
                connection
                    .query_row("SELECT count(*) FROM scans", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                connection
                    .query_row("SELECT count(*) FROM snapshot_review_pins", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap(),
                connection
                    .query_row(
                        "SELECT count(*) FROM snapshot_retention_tombstones",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
            )
        });

        let inventory = repository
            .inspect_retention_inventory_with_cap(observed_at, 0)
            .unwrap();
        assert_eq!(inventory.entries.len(), 5);
        assert_eq!(inventory.orphan_finals.len(), 1);
        assert_eq!(inventory.temporary_files.len(), 1);
        assert!(inventory.temporary_files[0].liveness_unknown);
        assert!(inventory.accounting_unstable);
        assert!(inventory.non_evictable_over_cap);
        assert_eq!(inventory.totals.active_pin_rows, 1);
        assert_eq!(inventory.totals.expired_pin_rows, 0);
        assert_eq!(
            inventory
                .entries
                .iter()
                .find(|entry| entry.scan_id == references[3].scan_id().clone())
                .unwrap()
                .latest_rank,
            Some(1)
        );
        assert_eq!(
            inventory
                .entries
                .iter()
                .find(|entry| entry.scan_id == references[2].scan_id().clone())
                .unwrap()
                .latest_rank,
            Some(2)
        );
        let oldest = inventory
            .entries
            .iter()
            .find(|entry| entry.scan_id == references[0].scan_id().clone())
            .unwrap();
        assert_eq!(oldest.latest_rank, None);
        assert_eq!(oldest.pins.active, 1);
        assert!(oldest.is_policy_protected());
        assert_eq!(
            inventory
                .eviction_observations
                .iter()
                .map(|candidate| candidate.scan_id.as_str())
                .collect::<Vec<_>>(),
            vec![references[1].scan_id().as_str()]
        );
        assert!(matches!(
            inventory
                .entries
                .iter()
                .find(|entry| entry.scan_id == references[4].scan_id().clone())
                .unwrap()
                .logical_state,
            super::super::snapshot_retention_inventory::SnapshotRetentionLogicalState::Tombstoned { .. }
        ));
        assert!(inventory.totals.tombstoned_residual.charged_bytes > 0);
        assert!(inventory.totals.orphan.charged_bytes > 0);
        assert!(inventory.totals.temporary_unknown_liveness.charged_bytes > 0);
        assert_eq!(
            inventory.totals.store_total.charged_bytes,
            inventory.totals.controls.charged_bytes
                + inventory.totals.available.charged_bytes
                + inventory.totals.tombstoned_residual.charged_bytes
                + inventory.totals.orphan.charged_bytes
                + inventory.totals.temporary_unknown_liveness.charged_bytes
        );

        let after_rows = store.with_connection(|connection| {
            (
                connection
                    .query_row("SELECT count(*) FROM scans", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                connection
                    .query_row("SELECT count(*) FROM snapshot_review_pins", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap(),
                connection
                    .query_row(
                        "SELECT count(*) FROM snapshot_retention_tombstones",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
            )
        });
        assert_eq!(after_rows, before_rows, "inventory must not mutate history");

        let at_expiry = repository
            .inspect_retention_inventory_with_cap(expires_at, u64::MAX)
            .unwrap();
        assert_eq!(at_expiry.totals.active_pin_rows, 0);
        assert_eq!(at_expiry.totals.expired_pin_rows, 1);
        assert_eq!(
            review_pin_count(&store),
            1,
            "read-only inventory must not prune expired pins"
        );
        assert_eq!(
            at_expiry
                .eviction_observations
                .iter()
                .map(|candidate| candidate.scan_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                references[0].scan_id().as_str(),
                references[1].scan_id().as_str()
            ]
        );
        pinned.release().unwrap();

        let read_only =
            SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadOnly).unwrap();
        assert_eq!(
            read_only
                .inspect_retention_inventory(observed_at)
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::ReadOnly
        );
        drop(read_only);
        drop(repository);
        drop(store);
        let (reopened_store, reopened_repository) = open_repository(&database);
        let reopened_inventory = reopened_repository
            .inspect_retention_inventory(expires_at + Duration::from_millis(1))
            .unwrap();
        assert_eq!(reopened_inventory.entries.len(), 5);
        assert_eq!(reopened_inventory.orphan_finals.len(), 1);
        assert_eq!(reopened_inventory.temporary_files.len(), 1);
        drop(reopened_repository);
        drop(reopened_store);
    }

    #[test]
    fn retention_inventory_rejects_duplicate_history_for_one_physical_name() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let (store, repository) = open_repository(&database);
        let document = document("scan:retention-duplicate-source", &root);
        let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO scans (
                         scan_id, volume_id, root_path, root_path_encoding,
                         started_at_unix_ms, completed_at_unix_ms, status,
                         snapshot_version, snapshot_relative_path,
                         snapshot_relative_path_encoding, snapshot_checksum_sha256,
                         directory_count, file_count, logical_bytes, allocated_bytes,
                         coverage_status, coverage_permille, issue_count
                     )
                     SELECT 'scan:retention-duplicate-hostile', volume_id,
                            root_path, root_path_encoding, started_at_unix_ms,
                            completed_at_unix_ms, status, snapshot_version,
                            snapshot_relative_path, snapshot_relative_path_encoding,
                            snapshot_checksum_sha256, directory_count, file_count,
                            logical_bytes, allocated_bytes, coverage_status,
                            coverage_permille, issue_count
                     FROM scans WHERE scan_id = ?1",
                    [reference.scan_id().as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            repository
                .inspect_retention_inventory(completed_at + Duration::from_secs(1))
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
        );
    }

    #[cfg(unix)]
    #[test]
    fn retention_inventory_rejects_lexically_aliased_stored_roots() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let (store, repository) = open_repository(&database);
        let document = document("scan:retention-aliased-root", &root);
        let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);
        for hostile_root in [
            b"/alias/./root".as_slice(),
            b"/alias//root",
            b"/alias/root/",
        ] {
            store.with_connection(|connection| {
                connection
                    .execute(
                        "UPDATE scans
                         SET root_path = ?1, root_path_encoding = 1
                         WHERE scan_id = ?2",
                        rusqlite::params![hostile_root, reference.scan_id().as_str()],
                    )
                    .unwrap();
            });
            assert_eq!(
                repository
                    .inspect_retention_inventory(completed_at + Duration::from_secs(1))
                    .unwrap_err()
                    .kind,
                SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
            );
        }
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn retention_inventory_rejects_inconsistent_active_pin_storage() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("missing/dux.sqlite3");
        let root = temp.path().join("missing/scan-root");
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let observed_at = completed_at + Duration::from_secs(1);
        let (store, repository) = open_repository(&database);
        let missing_document = document("scan:retention-active-missing", &root);
        let reference =
            complete_snapshot(&store, &repository, &missing_document, &root, completed_at);
        let lease = repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        // DUX-DESTRUCTIVE: allow=test-snapshot-inventory-remove-pinned-final -- remove only this TempDir-owned published fixture to prove an active pin cannot hide a missing physical final
        std::fs::remove_file(
            database
                .parent()
                .unwrap()
                .join("snapshots")
                .join(reference.file_name().as_str()),
        )
        .unwrap();
        assert_eq!(
            repository
                .inspect_retention_inventory(observed_at)
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
        );
        lease.release().unwrap();

        let database = temp.path().join("tombstoned/dux.sqlite3");
        let root = temp.path().join("tombstoned/scan-root");
        let (store, repository) = open_repository(&database);
        let document = document("scan:retention-active-tombstone", &root);
        let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);
        let lease = repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO snapshot_retention_tombstones (
                         scan_id, record_format_version, scan_status,
                         completed_at_unix_ms, snapshot_version,
                         snapshot_relative_path, snapshot_relative_path_encoding,
                         snapshot_checksum_sha256, committed_at_unix_ms
                     )
                     SELECT scan_id, 1, status, completed_at_unix_ms,
                            snapshot_version, snapshot_relative_path,
                            snapshot_relative_path_encoding,
                            snapshot_checksum_sha256, completed_at_unix_ms + 1
                     FROM scans WHERE scan_id = ?1",
                    [reference.scan_id().as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            repository
                .inspect_retention_inventory(observed_at)
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
        );
        lease.release().unwrap();
    }

    #[test]
    fn review_lease_is_explicit_renewable_and_drop_expires_without_writing() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-review-lease", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let observed_at = completed_at + Duration::from_secs(1);
        let (store, repository) = open_repository(&database);
        let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);

        let mut lease = repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        assert_eq!(lease.reference(), &reference);
        assert_eq!(lease.purpose(), SnapshotReviewPurpose::Explorer);
        assert_eq!(
            lease.expires_at().unwrap(),
            observed_at + Duration::from_secs(10 * 60)
        );
        assert_eq!(lease.load(observed_at).unwrap(), document);
        assert_eq!(review_pin_count(&store), 1);
        assert_eq!(
            lease
                .renew(observed_at - Duration::from_millis(1))
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
        );

        let renewed_at = observed_at + Duration::from_secs(2 * 60);
        let renewed_expiry = lease.renew(renewed_at).unwrap();
        assert_eq!(renewed_expiry, renewed_at + Duration::from_secs(10 * 60));
        assert_eq!(lease.load(renewed_at).unwrap(), document);
        assert_eq!(
            lease.load(renewed_expiry).unwrap_err().kind,
            SnapshotRepositoryErrorKind::ReviewLeaseExpired
        );
        assert_eq!(
            lease.renew(renewed_expiry).unwrap_err().kind,
            SnapshotRepositoryErrorKind::ReviewLeaseExpired
        );
        lease.release().unwrap();
        assert_eq!(review_pin_count(&store), 0);

        let cross_repository = repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        drop(repository);
        let reopened =
            SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadWrite).unwrap();
        assert_eq!(review_pin_count(&store), 1);
        assert_eq!(cross_repository.load(observed_at).unwrap(), document);
        cross_repository.release().unwrap();

        let dropped = reopened
            .acquire_review_lease(
                &reference,
                SnapshotReviewPurpose::CleanupReview,
                observed_at,
            )
            .unwrap();
        let dropped_expiry = dropped.expires_at().unwrap();
        drop(dropped);
        // Drop is close-only so it remains safe during unwinding or while a
        // caller happens to own another persistence guard.
        assert_eq!(review_pin_count(&store), 1);

        let replacement = reopened
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, dropped_expiry)
            .unwrap();
        // Equality is expired, and acquisition prunes the stale coordination
        // row before inserting its replacement.
        assert_eq!(review_pin_count(&store), 1);
        replacement.release().unwrap();
        assert_eq!(review_pin_count(&store), 0);

        let mut expired_while_retained = reopened
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        let expired_at = expired_while_retained.expires_at().unwrap();
        let concurrent =
            SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadWrite).unwrap();
        let successor = concurrent
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, expired_at)
            .unwrap();
        assert_eq!(
            expired_while_retained.load(expired_at).unwrap_err().kind,
            SnapshotRepositoryErrorKind::ReviewLeaseExpired
        );
        assert_eq!(
            expired_while_retained.renew(expired_at).unwrap_err().kind,
            SnapshotRepositoryErrorKind::ReviewLeaseExpired
        );
        // Pruning by another repository already established the exact release
        // postcondition, so releasing the old retained object is idempotent.
        expired_while_retained.release().unwrap();
        successor.release().unwrap();
        assert_eq!(review_pin_count(&store), 0);
    }

    #[test]
    fn review_lease_reconciles_exact_post_commit_outcomes() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-review-reconcile", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let observed_at = completed_at + Duration::from_secs(1);
        let (store, repository) = open_repository(&database);
        let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);

        let mut lease = repository
            .acquire_review_lease_after_commit_failure_for_test(
                &reference,
                SnapshotReviewPurpose::Explorer,
                observed_at,
            )
            .unwrap();
        assert_eq!(review_pin_count(&store), 1);
        assert_eq!(lease.load(observed_at).unwrap(), document);

        lease.fail_next_renew_after_commit_for_test();
        let renewed_at = observed_at + Duration::from_secs(60);
        assert_eq!(
            lease.renew(renewed_at).unwrap(),
            renewed_at + Duration::from_secs(10 * 60)
        );
        lease.fail_release_after_commit_for_test();
        lease.release().unwrap();
        assert_eq!(review_pin_count(&store), 0);

        let conflicting = repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        conflicting.replace_release_with_conflict_after_commit_for_test();
        assert_eq!(
            conflicting.release().unwrap_err().kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::OutcomeUnknown)
        );
        assert_eq!(review_pin_count(&store), 1);
        store.with_connection(|connection| {
            connection
                .execute("DELETE FROM snapshot_review_pins", [])
                .unwrap();
        });
    }

    #[test]
    fn review_lease_rejects_tombstones_and_hostile_active_pin_rows() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-review-hostile", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let observed_at = completed_at + Duration::from_secs(1);
        let (store, repository) = open_repository(&database);
        let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);

        let read_only =
            SnapshotRepository::open(Arc::clone(&store), SnapshotStoreAccess::ReadOnly).unwrap();
        assert_eq!(
            read_only
                .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
                .err()
                .unwrap()
                .kind,
            SnapshotRepositoryErrorKind::ReadOnly
        );

        store.with_connection(|connection| {
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "INSERT INTO snapshot_review_pins (
                         pin_id, record_format_version, scan_id, scan_status,
                         completed_at_unix_ms, snapshot_version,
                         snapshot_relative_path, snapshot_relative_path_encoding,
                         snapshot_checksum_sha256, owner_process_instance, purpose,
                         created_at_unix_ms, renewed_at_unix_ms, expires_at_unix_ms
                     )
                     SELECT 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 1, scan_id, status,
                            completed_at_unix_ms, snapshot_version,
                            snapshot_relative_path, snapshot_relative_path_encoding,
                            snapshot_checksum_sha256, 'not-a-process-instance', 'explorer',
                            ?2, ?2, ?2 + 600000
                     FROM scans WHERE scan_id = ?1",
                    rusqlite::params![
                        reference.scan_id().as_str(),
                        super::super::history::system_time_to_unix_ms(
                            observed_at,
                            HistoryErrorKind::InvalidInput,
                        )
                        .unwrap(),
                    ],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            repository
                .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
                .err()
                .unwrap()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
        );

        store.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM snapshot_review_pins
                     WHERE pin_id = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
                    [],
                )
                .unwrap();
        });

        let hostile_scan_id = "scan:snapshot-review-hostile-parent";
        let owner = repository.review.as_ref().unwrap().owner().unwrap();
        let observed_ms = super::super::history::system_time_to_unix_ms(
            observed_at,
            HistoryErrorKind::InvalidInput,
        )
        .unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO scans (
                         scan_id, volume_id, root_path, root_path_encoding,
                         started_at_unix_ms, completed_at_unix_ms, status,
                         snapshot_version, snapshot_relative_path,
                         snapshot_relative_path_encoding, snapshot_checksum_sha256,
                         directory_count, file_count, logical_bytes, allocated_bytes,
                         coverage_status, coverage_permille, issue_count
                     )
                     SELECT ?2, volume_id, root_path, root_path_encoding,
                            started_at_unix_ms, completed_at_unix_ms, status,
                            snapshot_version, snapshot_relative_path,
                            snapshot_relative_path_encoding, snapshot_checksum_sha256,
                            directory_count, file_count, logical_bytes, allocated_bytes,
                            coverage_status, coverage_permille, issue_count
                     FROM scans WHERE scan_id = ?1",
                    rusqlite::params![reference.scan_id().as_str(), hostile_scan_id],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO snapshot_review_pins (
                         pin_id, record_format_version, scan_id, scan_status,
                         completed_at_unix_ms, snapshot_version,
                         snapshot_relative_path, snapshot_relative_path_encoding,
                         snapshot_checksum_sha256, owner_process_instance, purpose,
                         created_at_unix_ms, renewed_at_unix_ms, expires_at_unix_ms
                     )
                     SELECT 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', 1, scan_id, status,
                            completed_at_unix_ms, snapshot_version,
                            snapshot_relative_path, snapshot_relative_path_encoding,
                            snapshot_checksum_sha256, ?2, 'explorer', ?3, ?3, ?3 + 600000
                     FROM scans WHERE scan_id = ?1",
                    rusqlite::params![hostile_scan_id, owner.as_str(), observed_ms],
                )
                .unwrap();
        });
        assert_eq!(
            repository
                .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
                .err()
                .unwrap()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM snapshot_review_pins WHERE scan_id = ?1",
                    [hostile_scan_id],
                )
                .unwrap();
            connection
                .execute("DELETE FROM scans WHERE scan_id = ?1", [hostile_scan_id])
                .unwrap();
            connection
                .execute(
                    "INSERT INTO snapshot_retention_tombstones (
                         scan_id, record_format_version, scan_status,
                         completed_at_unix_ms, snapshot_version,
                         snapshot_relative_path, snapshot_relative_path_encoding,
                         snapshot_checksum_sha256, committed_at_unix_ms
                     )
                     SELECT scan_id, 1, status, completed_at_unix_ms,
                            snapshot_version, snapshot_relative_path,
                            snapshot_relative_path_encoding,
                            snapshot_checksum_sha256, completed_at_unix_ms + 1
                     FROM scans WHERE scan_id = ?1",
                    [reference.scan_id().as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            repository
                .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
                .err()
                .unwrap()
                .kind,
            SnapshotRepositoryErrorKind::SnapshotUnavailable
        );
    }

    #[test]
    fn review_lease_local_bound_is_fail_closed_and_releases_slots() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-review-bound", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let observed_at = completed_at + Duration::from_secs(1);
        let (store, repository) = open_repository(&database);
        let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);

        let mut leases = Vec::new();
        for _ in 0..MAX_ACTIVE_PINS_PER_OWNER {
            leases.push(
                repository
                    .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
                    .unwrap(),
            );
        }
        assert_eq!(review_pin_count(&store), MAX_ACTIVE_PINS_PER_OWNER as i64);
        assert_eq!(
            repository
                .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
                .err()
                .unwrap()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::QueryLimitExceeded)
        );
        leases.pop().unwrap().release().unwrap();
        let replacement = repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        drop(replacement);
        drop(leases);
        assert_eq!(review_pin_count(&store), MAX_ACTIVE_PINS_PER_OWNER as i64);
    }

    #[test]
    fn review_population_pruning_is_bounded_and_oversize_fails_corrupt() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-review-population", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let observed_at = completed_at + Duration::from_secs(20 * 60);
        let observed_ms = super::super::history::system_time_to_unix_ms(
            observed_at,
            HistoryErrorKind::InvalidInput,
        )
        .unwrap();
        let (store, repository) = open_repository(&database);
        let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);
        let owner = repository.review.as_ref().unwrap().owner().unwrap();
        let owner_prefix = owner.as_str().rsplit_once(':').unwrap().0.to_owned();

        let insert_population = |count: usize, expired: bool| {
            store.with_connection(|connection| {
                connection.execute_batch("BEGIN IMMEDIATE").unwrap();
                for ordinal in 1..=count {
                    let id = format!("{ordinal:032x}");
                    let row_owner = format!("{owner_prefix}:{ordinal:032x}");
                    let renewed = if expired {
                        observed_ms - 600_000
                    } else {
                        observed_ms
                    };
                    connection
                        .execute(
                            "INSERT INTO snapshot_review_pins (
                                 pin_id, record_format_version, scan_id, scan_status,
                                 completed_at_unix_ms, snapshot_version,
                                 snapshot_relative_path, snapshot_relative_path_encoding,
                                 snapshot_checksum_sha256, owner_process_instance, purpose,
                                 created_at_unix_ms, renewed_at_unix_ms, expires_at_unix_ms
                             )
                             SELECT ?2, 1, scan_id, status, completed_at_unix_ms,
                                    snapshot_version, snapshot_relative_path,
                                    snapshot_relative_path_encoding,
                                    snapshot_checksum_sha256, ?3, 'explorer', ?4, ?4, ?4 + 600000
                             FROM scans WHERE scan_id = ?1",
                            rusqlite::params![
                                reference.scan_id().as_str(),
                                id,
                                row_owner,
                                renewed,
                            ],
                        )
                        .unwrap();
                }
                connection.execute_batch("COMMIT").unwrap();
            });
        };

        insert_population(MAX_EXPIRED_PRUNE + 1, true);
        let colliding =
            SnapshotReviewPinId::from_stored(format!("{:032x}", MAX_EXPIRED_PRUNE + 1)).unwrap();
        let unique =
            SnapshotReviewPinId::from_stored("ffffffffffffffffffffffffffffffff".into()).unwrap();
        let lease = repository
            .acquire_review_lease_with_pin_ids_for_test(
                &reference,
                SnapshotReviewPurpose::Explorer,
                observed_at,
                vec![colliding, unique],
            )
            .unwrap();
        // One bounded batch removes exactly 64 stale rows. The first prepared
        // ID collides with the remaining sentinel and the second succeeds;
        // collision retry must not run pruning a second time.
        assert_eq!(review_pin_count(&store), 2);
        lease.release().unwrap();
        store.with_connection(|connection| {
            connection
                .execute("DELETE FROM snapshot_review_pins", [])
                .unwrap();
        });

        insert_population(MAX_ACTIVE_PINS + 1, false);
        assert_eq!(
            repository
                .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
                .err()
                .unwrap()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
        );
    }

    #[test]
    fn dropped_review_pin_survives_independent_store_reopen_until_expiry() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-review-store-reopen", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let observed_at = completed_at + Duration::from_secs(1);

        let (reference, expiry) = {
            let (store, repository) = open_repository(&database);
            let reference = complete_snapshot(&store, &repository, &document, &root, completed_at);
            let lease = repository
                .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
                .unwrap();
            let expiry = lease.expires_at().unwrap();
            drop(lease);
            assert_eq!(review_pin_count(&store), 1);
            (reference, expiry)
        };

        let (reopened_store, reopened_repository) = open_repository(&database);
        assert_eq!(review_pin_count(&reopened_store), 1);
        let replacement = reopened_repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, expiry)
            .unwrap();
        assert_eq!(review_pin_count(&reopened_store), 1);
        replacement.release().unwrap();
        assert_eq!(review_pin_count(&reopened_store), 0);
    }

    #[test]
    fn review_lease_operations_fail_closed_after_external_newer_schema() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let review_document = document("scan:snapshot-review-schema-fence", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let observed_at = completed_at + Duration::from_secs(1);
        let (store, repository) = open_repository(&database);
        let reference =
            complete_snapshot(&store, &repository, &review_document, &root, completed_at);
        let mut lease = repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        let held_publication = repository
            .publish_orphan_for_test(&document(
                "scan:snapshot-review-schema-fence-orphan",
                &temp.path().join("orphan-root"),
            ))
            .unwrap();

        let current = super::super::status::DATABASE_SCHEMA_VERSION;
        let future = current + 1;
        let external = rusqlite::Connection::open(&database).unwrap();
        external
            .execute(
                "INSERT INTO schema_migrations (
                     version, name, checksum_sha256, applied_at_unix_ms
                 ) VALUES (?1, 'test-future-review-schema', zeroblob(32), 1)",
                [future],
            )
            .unwrap();
        external
            .pragma_update(None, "user_version", future)
            .unwrap();
        drop(external);

        let incompatible =
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::IncompatibleSchema);
        // The current-schema database fence is acquired before snapshot-store
        // inventory. Even though the publication holds that later lock, the
        // newer schema wins as IncompatibleSchema rather than Busy.
        assert_eq!(
            repository
                .inspect_retention_inventory(observed_at)
                .unwrap_err()
                .kind,
            incompatible
        );
        drop(held_publication);
        assert_eq!(
            repository
                .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
                .err()
                .unwrap()
                .kind,
            incompatible
        );
        assert_eq!(
            lease
                .renew(observed_at + Duration::from_secs(1))
                .unwrap_err()
                .kind,
            incompatible
        );
        assert_eq!(lease.release().unwrap_err().kind, incompatible);

        let external = rusqlite::Connection::open(&database).unwrap();
        external
            .execute("DELETE FROM schema_migrations WHERE version = ?1", [future])
            .unwrap();
        external
            .pragma_update(None, "user_version", current)
            .unwrap();
        drop(external);
        store.with_connection(|connection| {
            connection
                .execute("DELETE FROM snapshot_review_pins", [])
                .unwrap();
        });
    }

    #[test]
    fn review_acquisition_obeys_database_before_snapshot_lock_order() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let review_document = document("scan:snapshot-review-lock-order", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let observed_at = completed_at + Duration::from_secs(1);
        let (store, repository) = open_repository(&database);
        let reference =
            complete_snapshot(&store, &repository, &review_document, &root, completed_at);

        let orphan_root = temp.path().join("orphan-root");
        let orphan_document = document("scan:snapshot-review-lock-orphan", &orphan_root);
        let publication = repository
            .publish_orphan_for_test(&orphan_document)
            .unwrap();
        assert_eq!(
            repository
                .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
                .err()
                .unwrap()
                .kind,
            SnapshotRepositoryErrorKind::Storage(SnapshotStorageErrorKind::Busy)
        );
        drop(publication);
        let lease = repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        lease.release().unwrap();
    }

    #[test]
    fn file_first_completion_and_exact_retry_survive_reopen() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-complete", &root);
        let coverage = partial_coverage(&root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);

        let reference = {
            let (store, repository) = open_repository(&database);
            store
                .record_scan_started(
                    &NewScanRecord::try_new(
                        document.metadata.scan_id.clone(),
                        root.clone(),
                        UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                    )
                    .unwrap(),
                )
                .unwrap();
            let reference = repository
                .complete_scan(completed_at, counts(), &coverage, &document)
                .unwrap();
            assert_eq!(
                repository
                    .complete_scan(completed_at, counts(), &coverage, &document)
                    .unwrap(),
                reference
            );
            let durable = store
                .load_scan(&document.metadata.scan_id)
                .unwrap()
                .unwrap();
            assert_eq!(durable.snapshot(), Some(&reference));
            assert_eq!(durable.coverage(), &coverage);
            reference
        };

        let (store, reopened) = open_repository(&database);
        assert_eq!(reopened.load(&reference).unwrap(), document);
        assert_eq!(
            store
                .load_scan(&ScanId::new("scan:snapshot-complete").unwrap())
                .unwrap()
                .unwrap()
                .coverage(),
            &coverage
        );
    }

    #[test]
    fn committed_tombstone_blocks_valid_snapshot_and_survives_reopen() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-tombstoned", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root,
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        let reference = repository
            .complete_scan(completed_at, counts(), &complete_coverage(), &document)
            .unwrap();
        let snapshot_path = database
            .parent()
            .unwrap()
            .join("snapshots")
            .join(reference.file_name().as_str());
        assert!(snapshot_path.is_file());
        let guard = store.lock_current_history_connection().unwrap();
        assert_eq!(
            repository.load_with_guard(&guard, &reference).unwrap(),
            document
        );
        drop(guard);

        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO snapshot_retention_tombstones (
                         scan_id, record_format_version, scan_status,
                         completed_at_unix_ms, snapshot_version,
                         snapshot_relative_path, snapshot_relative_path_encoding,
                         snapshot_checksum_sha256, committed_at_unix_ms
                     )
                     SELECT scan_id, 1, status, completed_at_unix_ms,
                            snapshot_version, snapshot_relative_path,
                            snapshot_relative_path_encoding,
                            snapshot_checksum_sha256, completed_at_unix_ms + 1
                     FROM scans WHERE scan_id = ?1",
                    [reference.scan_id().as_str()],
                )
                .unwrap();
            assert!(
                connection
                    .execute(
                        "UPDATE snapshot_retention_tombstones
                         SET committed_at_unix_ms = committed_at_unix_ms + 1
                         WHERE scan_id = ?1",
                        [reference.scan_id().as_str()],
                    )
                    .is_err()
            );
            assert!(
                connection
                    .execute(
                        "DELETE FROM snapshot_retention_tombstones WHERE scan_id = ?1",
                        [reference.scan_id().as_str()],
                    )
                    .is_err()
            );
        });

        assert!(snapshot_path.is_file());
        assert_eq!(
            repository.load(&reference).unwrap_err().kind,
            SnapshotRepositoryErrorKind::SnapshotUnavailable
        );
        let guard = store.lock_current_history_connection().unwrap();
        assert_eq!(
            repository
                .load_with_guard(&guard, &reference)
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::SnapshotUnavailable
        );
        drop(guard);
        let durable = store.load_scan(reference.scan_id()).unwrap().unwrap();
        assert_eq!(durable.snapshot(), Some(&reference));
        drop(repository);
        drop(store);

        let (reopened_store, reopened) = open_repository(&database);
        assert_eq!(
            reopened.load(&reference).unwrap_err().kind,
            SnapshotRepositoryErrorKind::SnapshotUnavailable
        );
        assert_eq!(
            reopened_store
                .load_scan(reference.scan_id())
                .unwrap()
                .unwrap()
                .snapshot(),
            Some(&reference)
        );
    }

    #[test]
    fn tombstone_mismatched_from_parent_scan_fails_corrupt_before_opening_snapshot() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-hostile-tombstone", &root);
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root,
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        let reference = repository
            .complete_scan(
                UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
                counts(),
                &complete_coverage(),
                &document,
            )
            .unwrap();
        store.with_connection(|connection| {
            connection
                .pragma_update(None, "foreign_keys", false)
                .unwrap();
            connection
                .execute(
                    "INSERT INTO snapshot_retention_tombstones (
                         scan_id, record_format_version, scan_status,
                         completed_at_unix_ms, snapshot_version,
                         snapshot_relative_path, snapshot_relative_path_encoding,
                         snapshot_checksum_sha256, committed_at_unix_ms
                     )
                     SELECT scan_id, 1, status, completed_at_unix_ms + 1,
                            snapshot_version, snapshot_relative_path,
                            snapshot_relative_path_encoding,
                            snapshot_checksum_sha256, completed_at_unix_ms + 2
                     FROM scans WHERE scan_id = ?1",
                    [reference.scan_id().as_str()],
                )
                .unwrap();
            connection
                .pragma_update(None, "foreign_keys", true)
                .unwrap();
        });

        assert_eq!(
            repository.load(&reference).unwrap_err().kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::CorruptData)
        );
    }

    #[test]
    fn orphan_publication_is_adopted_only_after_exact_collision_validation() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-orphan", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root.clone(),
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();

        let orphan = repository.publish_orphan_for_test(&document).unwrap();
        orphan.revalidate().unwrap();
        let orphan_reference = orphan.reference().clone();
        drop(orphan);
        assert_eq!(
            store
                .load_scan(&document.metadata.scan_id)
                .unwrap()
                .unwrap()
                .status(),
            ScanStatus::Running
        );
        let adopted = repository
            .complete_scan(completed_at, counts(), &complete_coverage(), &document)
            .unwrap();
        assert_eq!(adopted, orphan_reference);
    }

    #[test]
    fn same_scan_id_different_orphan_document_is_rejected_without_finishing_scan() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let original = document("scan:snapshot-collision", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    original.metadata.scan_id.clone(),
                    root,
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        let orphan = repository.publish_orphan_for_test(&original).unwrap();
        let original_reference = orphan.reference().clone();
        drop(orphan);

        let mut different = original.clone();
        different.metadata.totals.logical_bytes = 11;
        different.metadata.totals.allocated_bytes = Some(17);
        different.nodes[0].logical_bytes = 11;
        different.nodes[0].allocated_bytes = Some(17);
        different.nodes[1].logical_bytes = 11;
        different.nodes[1].allocated_bytes = Some(17);

        assert_eq!(
            repository
                .complete_scan(
                    completed_at,
                    counts_for(&different),
                    &complete_coverage(),
                    &different,
                )
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::ReferenceMismatch
        );
        assert_eq!(
            store
                .load_scan(&original.metadata.scan_id)
                .unwrap()
                .unwrap()
                .status(),
            ScanStatus::Running
        );
        assert_eq!(repository.load(&original_reference).unwrap(), original);
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn durable_reference_fails_closed_when_snapshot_is_missing_or_corrupt() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-reference-failure", &root);
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root.clone(),
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        let reference = repository
            .complete_scan(
                UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
                counts(),
                &complete_coverage(),
                &document,
            )
            .unwrap();
        let path = database
            .parent()
            .unwrap()
            .join("snapshots")
            .join(reference.file_name().as_str());
        std::fs::write(&path, b"corrupt").unwrap();
        assert!(matches!(
            repository.load(&reference).unwrap_err().kind,
            SnapshotRepositoryErrorKind::Codec(_)
        ));
        let observed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_003_000);
        let corrupt_lease = repository
            .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at)
            .unwrap();
        assert!(matches!(
            corrupt_lease.load(observed_at).unwrap_err().kind,
            SnapshotRepositoryErrorKind::Codec(_)
        ));
        corrupt_lease.release().unwrap();

        // DUX-DESTRUCTIVE: allow=test-snapshot-referenced-file-remove -- remove only this test-owned published fixture to prove a durable reference fails closed when its file disappears
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            repository.load(&reference).unwrap_err().kind,
            SnapshotRepositoryErrorKind::MissingSnapshot
        );
        assert_eq!(
            repository
                .acquire_review_lease(&reference, SnapshotReviewPurpose::Explorer, observed_at,)
                .err()
                .unwrap()
                .kind,
            SnapshotRepositoryErrorKind::MissingSnapshot
        );
        assert_eq!(review_pin_count(&store), 0);

        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO snapshot_retention_tombstones (
                         scan_id, record_format_version, scan_status,
                         completed_at_unix_ms, snapshot_version,
                         snapshot_relative_path, snapshot_relative_path_encoding,
                         snapshot_checksum_sha256, committed_at_unix_ms
                     )
                     SELECT scan_id, 1, status, completed_at_unix_ms,
                            snapshot_version, snapshot_relative_path,
                            snapshot_relative_path_encoding,
                            snapshot_checksum_sha256, completed_at_unix_ms + 1
                     FROM scans WHERE scan_id = ?1",
                    [reference.scan_id().as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            repository.load(&reference).unwrap_err().kind,
            SnapshotRepositoryErrorKind::SnapshotUnavailable
        );
    }

    #[test]
    fn missing_scan_and_mismatched_summary_publish_nothing() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:snapshot-rejected", &root);
        let (store, repository) = open_repository(&database);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);

        assert_eq!(
            repository
                .complete_scan(completed_at, counts(), &complete_coverage(), &document)
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::NotFound)
        );
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root,
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            repository
                .complete_scan(completed_at, counts(), &ScanCoverage::unknown(), &document)
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::ReferenceMismatch
        );
        let outside_coverage = ScanCoverage::try_from_terminal(
            None,
            vec![
                ScanIssue::try_new(
                    ScanIssueKind::MetadataError,
                    Some(temp.path().join("outside")),
                    1,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(
            repository
                .complete_scan(completed_at, counts(), &outside_coverage, &document)
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::ReferenceMismatch
        );
        assert_eq!(
            std::fs::read_dir(database.parent().unwrap().join("snapshots"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("snapshot-"))
                .count(),
            0
        );
    }

    #[test]
    fn out_of_sqlite_range_counts_are_rejected_before_snapshot_publication() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let mut document = document("scan:snapshot-count-overflow", &root);
        let overflow = (i64::MAX as u64) + 1;
        document.metadata.totals.logical_bytes = overflow;
        document.nodes[0].logical_bytes = overflow;
        document.nodes[1].logical_bytes = overflow;
        let counts = counts_for(&document);
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root,
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();

        assert_eq!(
            repository
                .complete_scan(
                    UNIX_EPOCH + Duration::from_millis(1_750_000_002_000),
                    counts,
                    &complete_coverage(),
                    &document,
                )
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
        );
        assert_eq!(
            std::fs::read_dir(database.parent().unwrap().join("snapshots"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("snapshot-"))
                .count(),
            0
        );
        assert_eq!(
            store
                .load_scan(&document.metadata.scan_id)
                .unwrap()
                .unwrap()
                .status(),
            ScanStatus::Running
        );
    }

    #[test]
    fn scan_and_terminal_candidate_batch_commit_atomically_and_retry_exactly() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:evaluation-success", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let evaluation_completed_at = completed_at + Duration::from_millis(500);
        let terminal = CandidateEvaluationCompletion::succeeded(
            evaluation_completed_at,
            vec![candidate(
                &document.metadata.scan_id,
                "candidate:evaluation-success",
                &root.join("artifact.o"),
            )],
        )
        .unwrap();
        let identity = evaluation_identity(7);

        {
            let (store, repository) = open_repository(&database);
            store
                .record_scan_started(
                    &NewScanRecord::try_new(
                        document.metadata.scan_id.clone(),
                        root.clone(),
                        UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                    )
                    .unwrap(),
                )
                .unwrap();
            let reference = repository
                .complete_scan_with_candidate_evaluation(
                    completed_at,
                    counts(),
                    &complete_coverage(),
                    &document,
                    &identity,
                    &terminal,
                )
                .unwrap();
            assert_eq!(
                repository
                    .complete_scan_with_candidate_evaluation(
                        completed_at,
                        counts(),
                        &complete_coverage(),
                        &document,
                        &identity,
                        &terminal,
                    )
                    .unwrap(),
                reference
            );
            assert_eq!(
                store
                    .load_candidate_evaluation(&document.metadata.scan_id)
                    .unwrap()
                    .unwrap()
                    .status(),
                super::super::candidate_evaluation_history::CandidateEvaluationStatus::Succeeded {
                    candidate_count: 1
                }
            );
            assert!(matches!(
                store
                    .load_candidate(&CandidateId::new("candidate:evaluation-success").unwrap())
                    .unwrap(),
                Some(StoredCandidateRecord::Complete(_))
            ));
        }

        let (store, repository) = open_repository(&database);
        repository
            .complete_scan_with_candidate_evaluation(
                completed_at,
                counts(),
                &complete_coverage(),
                &document,
                &identity,
                &terminal,
            )
            .unwrap();
        assert_eq!(
            store
                .load_scan(&document.metadata.scan_id)
                .unwrap()
                .unwrap()
                .status(),
            ScanStatus::Succeeded
        );
        assert_eq!(
            repository
                .complete_scan_with_candidate_evaluation(
                    completed_at,
                    counts(),
                    &complete_coverage(),
                    &document,
                    &evaluation_identity(9),
                    &terminal,
                )
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::ReferenceMismatch
        );
    }

    #[test]
    fn maximum_candidate_evaluation_batch_loads_after_reopen_within_query_budget() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:evaluation-maximum-batch", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let evaluation_completed_at = completed_at + Duration::from_millis(500);
        let candidates = (0..crate::domain::MAX_EVALUATED_CANDIDATES)
            .map(|index| {
                candidate(
                    &document.metadata.scan_id,
                    &format!("candidate:maximum:{index:04}"),
                    &root.join(format!("artifact-{index:04}.o")),
                )
            })
            .collect();
        let terminal =
            CandidateEvaluationCompletion::succeeded(evaluation_completed_at, candidates).unwrap();

        {
            let (store, repository) = open_repository(&database);
            store
                .record_scan_started(
                    &NewScanRecord::try_new(
                        document.metadata.scan_id.clone(),
                        root,
                        UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                    )
                    .unwrap(),
                )
                .unwrap();
            repository
                .complete_scan_with_candidate_evaluation(
                    completed_at,
                    counts(),
                    &complete_coverage(),
                    &document,
                    &evaluation_identity(8),
                    &terminal,
                )
                .unwrap();
        }

        let (store, _repository) = open_repository(&database);
        let evaluation = store
            .load_candidate_evaluation(&document.metadata.scan_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            evaluation.status(),
            super::super::candidate_evaluation_history::CandidateEvaluationStatus::Succeeded {
                candidate_count: crate::domain::MAX_EVALUATED_CANDIDATES as u32,
            }
        );
        assert_eq!(
            evaluation.candidates().len(),
            crate::domain::MAX_EVALUATED_CANDIDATES
        );
    }

    #[test]
    fn typed_evaluation_failure_is_terminal_and_contains_no_candidates() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:evaluation-failed", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let terminal = CandidateEvaluationCompletion::failed(
            completed_at + Duration::from_millis(1),
            super::super::candidate_evaluation_history::CandidateEvaluationFailureKind::Cancelled,
        )
        .unwrap();
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root.clone(),
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        repository
            .complete_scan_with_candidate_evaluation(
                completed_at,
                counts(),
                &complete_coverage(),
                &document,
                &evaluation_identity(11),
                &terminal,
            )
            .unwrap();
        let stored_evaluation = store
            .load_candidate_evaluation(&document.metadata.scan_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            stored_evaluation.status(),
            super::super::candidate_evaluation_history::CandidateEvaluationStatus::Failed {
                kind: super::super::candidate_evaluation_history::CandidateEvaluationFailureKind::Cancelled
            }
        );
        assert!(stored_evaluation.candidates().is_empty());
        assert_eq!(
            store
                .record_candidate_discovered(&candidate(
                    &document.metadata.scan_id,
                    "candidate:after-failed-evaluation",
                    &root.join("artifact.o"),
                ))
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
    }

    #[test]
    fn hostile_terminal_time_and_record_format_are_rejected_by_loader() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:evaluation-corrupt-row", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let terminal = CandidateEvaluationCompletion::succeeded(
            completed_at + Duration::from_millis(500),
            Vec::new(),
        )
        .unwrap();
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root,
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        repository
            .complete_scan_with_candidate_evaluation(
                completed_at,
                counts(),
                &complete_coverage(),
                &document,
                &evaluation_identity(12),
                &terminal,
            )
            .unwrap();

        store.with_connection(|connection| {
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "UPDATE candidate_evaluations
                     SET completed_at_unix_ms = scheduled_at_unix_ms - 1
                     WHERE scan_id = ?1",
                    [document.metadata.scan_id.as_str()],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            store
                .load_candidate_evaluation(&document.metadata.scan_id)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );

        store.with_connection(|connection| {
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "UPDATE candidate_evaluations
                     SET completed_at_unix_ms = ?2, record_format_version = 2
                     WHERE scan_id = ?1",
                    rusqlite::params![document.metadata.scan_id.as_str(), 1_750_000_002_500_i64],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            store
                .load_candidate_evaluation(&document.metadata.scan_id)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn candidate_created_time_must_match_terminal_evaluation_time() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:evaluation-candidate-time", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let evaluation_completed_at = completed_at + Duration::from_millis(500);
        let terminal = CandidateEvaluationCompletion::succeeded(
            evaluation_completed_at,
            vec![candidate(
                &document.metadata.scan_id,
                "candidate:evaluation-candidate-time",
                &root.join("artifact.o"),
            )],
        )
        .unwrap();
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root,
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        repository
            .complete_scan_with_candidate_evaluation(
                completed_at,
                counts(),
                &complete_coverage(),
                &document,
                &evaluation_identity(14),
                &terminal,
            )
            .unwrap();

        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE candidates
                     SET created_at_unix_ms = created_at_unix_ms + 1
                     WHERE scan_id = ?1",
                    [document.metadata.scan_id.as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            store
                .load_candidate_evaluation(&document.metadata.scan_id)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn late_candidate_collision_rolls_back_scan_evaluation_and_earlier_candidate() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let prior_root = temp.path().join("prior-root");
        let prior = document("scan:evaluation-prior", &prior_root);
        let root = temp.path().join("scan-root");
        let target = document("scan:evaluation-rollback", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let (store, repository) = open_repository(&database);

        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    prior.metadata.scan_id.clone(),
                    prior_root.clone(),
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        repository
            .complete_scan(completed_at, counts(), &complete_coverage(), &prior)
            .unwrap();
        store
            .record_candidate_discovered(&candidate(
                &prior.metadata.scan_id,
                "candidate:collision",
                &prior_root.join("artifact.o"),
            ))
            .unwrap();

        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    target.metadata.scan_id.clone(),
                    root.clone(),
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        let mismatched_time_terminal = CandidateEvaluationCompletion::succeeded(
            completed_at + Duration::from_millis(250),
            vec![candidate(
                &target.metadata.scan_id,
                "candidate:mismatched-evaluation-time",
                &root.join("mismatched-time.o"),
            )],
        )
        .unwrap();
        assert_eq!(
            repository
                .complete_scan_with_candidate_evaluation(
                    completed_at,
                    counts(),
                    &complete_coverage(),
                    &target,
                    &evaluation_identity(13),
                    &mismatched_time_terminal,
                )
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
        );
        let cross_scan_terminal = CandidateEvaluationCompletion::succeeded(
            completed_at + Duration::from_millis(250),
            vec![candidate(
                &prior.metadata.scan_id,
                "candidate:cross-scan",
                &root.join("cross-scan.o"),
            )],
        )
        .unwrap();
        assert_eq!(
            repository
                .complete_scan_with_candidate_evaluation(
                    completed_at,
                    counts(),
                    &complete_coverage(),
                    &target,
                    &evaluation_identity(13),
                    &cross_scan_terminal,
                )
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::InvalidInput)
        );
        let terminal = CandidateEvaluationCompletion::succeeded(
            completed_at + Duration::from_millis(500),
            vec![
                candidate(
                    &target.metadata.scan_id,
                    "candidate:inserted-first",
                    &root.join("first.o"),
                ),
                candidate(
                    &target.metadata.scan_id,
                    "candidate:collision",
                    &root.join("artifact.o"),
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            repository
                .complete_scan_with_candidate_evaluation(
                    completed_at,
                    counts(),
                    &complete_coverage(),
                    &target,
                    &evaluation_identity(13),
                    &terminal,
                )
                .unwrap_err()
                .kind,
            SnapshotRepositoryErrorKind::History(HistoryErrorKind::AlreadyExists)
        );
        assert_eq!(
            store
                .load_scan(&target.metadata.scan_id)
                .unwrap()
                .unwrap()
                .status(),
            ScanStatus::Running
        );
        assert!(
            store
                .load_candidate_evaluation(&target.metadata.scan_id)
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .load_candidate(&CandidateId::new("candidate:inserted-first").unwrap())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn exact_post_commit_failure_reconciles_full_terminal_output() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:evaluation-ambiguous", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root,
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        let published = repository.publish_orphan_for_test(&document).unwrap();
        let completion = ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
            document.metadata.scan_id.clone(),
            completed_at,
            counts(),
            complete_coverage(),
            published.reference().clone(),
        )
        .unwrap();
        let request = NewCandidateEvaluation::try_new(
            evaluation_identity(15),
            published.reference(),
            completed_at,
        )
        .unwrap();
        let terminal = CandidateEvaluationCompletion::succeeded(
            completed_at + Duration::from_millis(1) + Duration::from_nanos(789),
            Vec::new(),
        )
        .unwrap();
        store
            .record_scan_finished_with_evaluation_after_commit_failure_for_test(
                &completion,
                &request,
                &terminal,
            )
            .unwrap();
        assert_eq!(
            store
                .load_candidate_evaluation(&document.metadata.scan_id)
                .unwrap()
                .unwrap()
                .status(),
            super::super::candidate_evaluation_history::CandidateEvaluationStatus::Succeeded {
                candidate_count: 0
            }
        );
    }

    #[test]
    fn pending_evaluation_loads_and_standalone_terminal_commit_reconciles() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("scan-root");
        let document = document("scan:evaluation-pending", &root);
        let completed_at = UNIX_EPOCH + Duration::from_millis(1_750_000_002_000);
        let (store, repository) = open_repository(&database);
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    document.metadata.scan_id.clone(),
                    root.clone(),
                    UNIX_EPOCH + Duration::from_millis(1_750_000_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        let published = repository.publish_orphan_for_test(&document).unwrap();
        let completion = ScanCompletionRecord::try_succeeded_with_snapshot_and_coverage(
            document.metadata.scan_id.clone(),
            completed_at,
            counts(),
            complete_coverage(),
            published.reference().clone(),
        )
        .unwrap();
        let request = NewCandidateEvaluation::try_new(
            evaluation_identity(17),
            published.reference(),
            completed_at,
        )
        .unwrap();
        {
            let mut guard = store.lock_current_history_connection().unwrap();
            store
                .record_scan_finished_and_schedule_evaluation_reconciled_with_guard(
                    &mut guard,
                    &completion,
                    &request,
                )
                .unwrap();
        }
        assert_eq!(
            store
                .load_candidate_evaluation(&document.metadata.scan_id)
                .unwrap()
                .unwrap()
                .status(),
            super::super::candidate_evaluation_history::CandidateEvaluationStatus::Pending
        );
        assert_eq!(
            store
                .record_candidate_discovered(&candidate(
                    &document.metadata.scan_id,
                    "candidate:while-pending",
                    &root.join("artifact.o"),
                ))
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        let terminal = CandidateEvaluationCompletion::succeeded(
            completed_at + Duration::from_millis(500),
            vec![candidate(
                &document.metadata.scan_id,
                "candidate:pending",
                &root.join("artifact.o"),
            )],
        )
        .unwrap();
        store
            .record_candidate_evaluation_completed_after_commit_failure_for_test(
                &request, &terminal,
            )
            .unwrap();
        assert_eq!(
            store
                .load_candidate_evaluation(&document.metadata.scan_id)
                .unwrap()
                .unwrap()
                .status(),
            super::super::candidate_evaluation_history::CandidateEvaluationStatus::Succeeded {
                candidate_count: 1
            }
        );
        assert_eq!(
            store
                .record_candidate_discovered(&candidate(
                    &document.metadata.scan_id,
                    "candidate:after-succeeded-evaluation",
                    &root.join("other.o"),
                ))
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
    }
}
