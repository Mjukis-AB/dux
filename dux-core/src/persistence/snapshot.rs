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

use super::candidate_evaluation_history::{
    CandidateEvaluationCompletion, CandidateEvaluationIdentity, NewCandidateEvaluation,
};
use super::history::{
    HistoryError, HistoryErrorKind, ScanCompletionRecord, ScanCounts, ScanStatus,
};
use super::snapshot_retention::{SnapshotRetentionState, load_snapshot_retention_state};
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
    SnapshotUnavailable,
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

        // DUX-DESTRUCTIVE: allow=test-snapshot-referenced-file-remove -- remove only this test-owned published fixture to prove a durable reference fails closed when its file disappears
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            repository.load(&reference).unwrap_err().kind,
            SnapshotRepositoryErrorKind::MissingSnapshot
        );

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
