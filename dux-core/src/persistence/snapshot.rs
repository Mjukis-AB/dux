//! Immutable, private, non-authoritative scan snapshots.
//!
//! The legacy CLI cache is intentionally separate. Snapshot bytes may support
//! presentation and discovery, but never path validation or cleanup authority.

use std::io::{Seek, SeekFrom};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use crate::domain::{ScanCoverage, ScanId};

use super::history::{
    HistoryError, HistoryErrorKind, ScanCompletionRecord, ScanCounts, ScanStatus,
};
use super::store::{HistoryConnectionGuard, StoreCoordinator};

mod codec;
mod storage;

pub(crate) use codec::{
    HostValue, SNAPSHOT_FORMAT_VERSION, SnapshotCodecError, SnapshotCodecErrorKind, SnapshotDigest,
    SnapshotDocument, SnapshotMetadata, SnapshotNode, SnapshotNodeKind, SnapshotScanFlags,
    SnapshotTimestamp, SnapshotTotals, SnapshotUnixIdentity, decode_snapshot, encode_snapshot,
};
pub(crate) use storage::{
    RetainedSnapshot, SecureSnapshotStore, SnapshotFileName, SnapshotPublication,
    SnapshotPublicationLease, SnapshotStorageError, SnapshotStorageErrorKind, SnapshotStoreAccess,
    StagedSnapshot,
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
    IncompatibleVersion,
    ReferenceMismatch,
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

/// Coordinates bounded codec validation with immutable private publication.
pub(crate) struct SnapshotRepository {
    database: Arc<StoreCoordinator>,
    store: Option<SecureSnapshotStore>,
    access: SnapshotStoreAccess,
}

impl SnapshotRepository {
    pub(crate) fn open(
        database: Arc<StoreCoordinator>,
        access: SnapshotStoreAccess,
    ) -> Result<Self, SnapshotRepositoryError> {
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
        if reference.version() != SNAPSHOT_FORMAT_VERSION {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::IncompatibleVersion,
            ));
        }
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingStore))?;
        let retained = store
            .open(reference.file_name())
            .map_err(map_storage)?
            .ok_or_else(|| repository_error(SnapshotRepositoryErrorKind::MissingSnapshot))?;
        let (document, digest) = decode_retained(&retained)?;
        if digest != reference.digest() || &document.metadata.scan_id != reference.scan_id() {
            return Err(repository_error(
                SnapshotRepositoryErrorKind::ReferenceMismatch,
            ));
        }
        Ok(document)
    }

    /// Publish the immutable file before exact-CASing the durable scan summary.
    /// A published file without a DB row is a harmless retention orphan; the
    /// reverse ordering is forbidden.
    pub(crate) fn complete_scan(
        &self,
        completed_at: SystemTime,
        counts: ScanCounts,
        coverage: &ScanCoverage,
        document: &SnapshotDocument,
    ) -> Result<SnapshotReference, SnapshotRepositoryError> {
        if self.access != SnapshotStoreAccess::ReadWrite {
            return Err(repository_error(SnapshotRepositoryErrorKind::ReadOnly));
        }
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
                || &self.load(reference)? != document
            {
                return Err(repository_error(
                    SnapshotRepositoryErrorKind::ReferenceMismatch,
                ));
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
        self.database
            .record_scan_finished_reconciled_with_guard(&mut database_guard, &completion)
            .map_err(map_history)?;
        published.revalidate()?;
        Ok(published.reference().clone())
    }
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

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;
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
                    root,
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

        // DUX-DESTRUCTIVE: allow=test-snapshot-referenced-file-remove -- remove only this test-owned published fixture to prove a durable reference fails closed when its file disappears
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            repository.load(&reference).unwrap_err().kind,
            SnapshotRepositoryErrorKind::MissingSnapshot
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
}
