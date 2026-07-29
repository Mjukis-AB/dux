//! Durable, non-authoritative candidate-evaluation lifecycle.
//!
//! A pending row binds one succeeded immutable snapshot to exact evaluator,
//! catalog, and context identities. Current production completion writes the
//! pending identity and terminal result in one transaction; pending-only state
//! is consumed only by the bounded restart-recovery seam, while startup
//! scheduling and UI/FFI orchestration remain separate. No owner, heartbeat,
//! or durable running state exists. A successful terminal transition inserts the complete
//! bounded candidate batch atomically.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant, SystemTime};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::domain::{
    CANDIDATE_CATALOG_SCHEMA_VERSION, CANDIDATE_CATALOG_SHA256, CANDIDATE_CONTEXT_FORMAT_VERSION,
    CANDIDATE_EVALUATOR_REVISION, CandidateId, MAX_EVALUATED_CANDIDATES, ScanCoverageStatus,
    ScanId, candidate_evaluation_context_digest_for_observation,
};

use super::candidate_history::{
    CandidateHistoryStatus, CompleteCandidateRecord, NewCandidateRecord, PreparedCandidate,
    candidate_record_batch_fits_materialization_budget, complete_candidate_matches_new,
    ensure_prepared_candidate_batch_budget, insert_candidate,
    load_complete_candidate_batch_within_budget,
};
use super::cleanup_history::{CandidateStatusCoupling, CleanupSessionId};
use super::cleanup_journal::{JournalLifecycle, load_cleanup_journal_within_budget};
use super::history::{
    HistoryError, HistoryErrorKind, ScanRecord, ScanStatus, load_scan_record_within_budget,
    map_query_sql_error, map_write_sql_error, system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::snapshot::SnapshotReference;

const MAX_STORED_ID_BYTES: i64 = 128;
const MAX_FAILURE_KIND_BYTES: i64 = 64;
const EVALUATION_QUERY_PROGRESS_INTERVAL: i32 = 100;
const EVALUATION_QUERY_BASE_CALLBACKS: u64 = 1_000;
const EVALUATION_QUERY_CALLBACKS_PER_CANDIDATE: u64 = 12;
const EVALUATION_QUERY_MAX_ELAPSED: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CandidateEvaluationIdentity {
    evaluator_revision: u32,
    rule_catalog_schema_version: u32,
    rule_catalog_sha256: [u8; 32],
    context_format_version: u32,
    context_sha256: [u8; 32],
}

impl CandidateEvaluationIdentity {
    pub(crate) fn try_new(
        evaluator_revision: u32,
        rule_catalog_schema_version: u32,
        rule_catalog_sha256: [u8; 32],
        context_format_version: u32,
        context_sha256: [u8; 32],
    ) -> Result<Self, HistoryError> {
        if evaluator_revision == 0
            || rule_catalog_schema_version == 0
            || context_format_version == 0
        {
            return Err(invalid());
        }
        Ok(Self {
            evaluator_revision,
            rule_catalog_schema_version,
            rule_catalog_sha256,
            context_format_version,
            context_sha256,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NewCandidateEvaluation {
    scan_id: ScanId,
    identity: CandidateEvaluationIdentity,
    snapshot_version: u32,
    snapshot_sha256: [u8; 32],
    scheduled_at: SystemTime,
}

impl NewCandidateEvaluation {
    pub(crate) fn try_new(
        identity: CandidateEvaluationIdentity,
        snapshot: &SnapshotReference,
        scheduled_at: SystemTime,
    ) -> Result<Self, HistoryError> {
        let scheduled_at_unix_ms =
            system_time_to_unix_ms(scheduled_at, HistoryErrorKind::InvalidInput)?;
        Ok(Self {
            scan_id: snapshot.scan_id().clone(),
            identity,
            snapshot_version: snapshot.version(),
            snapshot_sha256: snapshot.digest().bytes(),
            scheduled_at: unix_ms_to_system_time(scheduled_at_unix_ms)?,
        })
    }

    pub(crate) fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CandidateEvaluationFailureKind {
    Cancelled,
    CatalogInvalid,
    ContextInvalid,
    EvaluationFailed,
    CandidateInvalid,
    LimitExceeded,
}

impl CandidateEvaluationFailureKind {
    fn as_stored(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::CatalogInvalid => "catalog_invalid",
            Self::ContextInvalid => "context_invalid",
            Self::EvaluationFailed => "evaluation_failed",
            Self::CandidateInvalid => "candidate_invalid",
            Self::LimitExceeded => "limit_exceeded",
        }
    }

    fn from_stored(value: &str) -> Result<Self, HistoryError> {
        match value {
            "cancelled" => Ok(Self::Cancelled),
            "catalog_invalid" => Ok(Self::CatalogInvalid),
            "context_invalid" => Ok(Self::ContextInvalid),
            "evaluation_failed" => Ok(Self::EvaluationFailed),
            "candidate_invalid" => Ok(Self::CandidateInvalid),
            "limit_exceeded" => Ok(Self::LimitExceeded),
            _ => Err(corrupt()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CandidateEvaluationStatus {
    Pending,
    Succeeded {
        candidate_count: u32,
    },
    Failed {
        kind: CandidateEvaluationFailureKind,
    },
}

/// One evaluator-owned terminal result. This is discovery data only and
/// cannot approve a cleanup plan or grant execution authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CandidateEvaluationCompletion {
    completed_at: SystemTime,
    outcome: CandidateEvaluationOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CandidateEvaluationOutcome {
    Succeeded {
        candidates: Vec<NewCandidateRecord>,
    },
    Failed {
        kind: CandidateEvaluationFailureKind,
    },
}

impl CandidateEvaluationCompletion {
    pub(crate) fn batch_fits_materialization_budget(
        candidates: &[NewCandidateRecord],
    ) -> Result<bool, HistoryError> {
        candidate_record_batch_fits_materialization_budget(candidates)
    }

    pub(crate) fn succeeded(
        completed_at: SystemTime,
        candidates: Vec<NewCandidateRecord>,
    ) -> Result<Self, HistoryError> {
        if candidates.len() > MAX_EVALUATED_CANDIDATES
            || !Self::batch_fits_materialization_budget(&candidates)?
        {
            return Err(invalid());
        }
        let completed_at_unix_ms =
            system_time_to_unix_ms(completed_at, HistoryErrorKind::InvalidInput)?;
        Ok(Self {
            completed_at: unix_ms_to_system_time(completed_at_unix_ms)?,
            outcome: CandidateEvaluationOutcome::Succeeded { candidates },
        })
    }

    pub(crate) fn failed(
        completed_at: SystemTime,
        kind: CandidateEvaluationFailureKind,
    ) -> Result<Self, HistoryError> {
        let completed_at_unix_ms =
            system_time_to_unix_ms(completed_at, HistoryErrorKind::InvalidInput)?;
        Ok(Self {
            completed_at: unix_ms_to_system_time(completed_at_unix_ms)?,
            outcome: CandidateEvaluationOutcome::Failed { kind },
        })
    }

    pub(super) fn prepare_candidates(&self) -> Result<Vec<PreparedCandidate>, HistoryError> {
        let candidates = match &self.outcome {
            CandidateEvaluationOutcome::Succeeded { candidates } => {
                candidates.iter().map(PreparedCandidate::prepare).collect()
            }
            CandidateEvaluationOutcome::Failed { .. } => Ok(Vec::new()),
        }?;
        ensure_prepared_candidate_batch_budget(&candidates)?;
        Ok(candidates)
    }

    pub(super) fn validate_for_request(
        &self,
        request: &NewCandidateEvaluation,
    ) -> Result<(), HistoryError> {
        self.validate_for_scan(request.scan_id(), request.scheduled_at)
    }

    pub(super) fn validate_for_scan(
        &self,
        scan_id: &ScanId,
        scheduled_at: SystemTime,
    ) -> Result<(), HistoryError> {
        if let CandidateEvaluationOutcome::Succeeded { candidates } = &self.outcome {
            if candidates.len() > MAX_EVALUATED_CANDIDATES {
                return Err(invalid());
            }
            let mut ids = HashSet::with_capacity(candidates.len());
            for candidate in candidates {
                if candidate.candidate().source_scan_id() != scan_id
                    || candidate.created_at() != self.completed_at
                    || !ids.insert(candidate.candidate().id().clone())
                {
                    return Err(invalid());
                }
            }
        }
        let scheduled_at = system_time_to_unix_ms(scheduled_at, HistoryErrorKind::InvalidInput)?;
        let completed_at =
            system_time_to_unix_ms(self.completed_at(), HistoryErrorKind::InvalidInput)?;
        if completed_at < scheduled_at {
            return Err(invalid());
        }
        Ok(())
    }

    pub(crate) const fn completed_at(&self) -> SystemTime {
        self.completed_at
    }

    pub(super) fn finalize(
        &self,
        transaction: &Transaction<'_>,
        request: &NewCandidateEvaluation,
        candidates: &[PreparedCandidate],
    ) -> Result<(), HistoryError> {
        match self.outcome {
            CandidateEvaluationOutcome::Succeeded { .. } => finalize_candidate_evaluation_success(
                transaction,
                request,
                self.completed_at,
                candidates,
            ),
            CandidateEvaluationOutcome::Failed { kind } => {
                finalize_candidate_evaluation_failure(transaction, request, self.completed_at, kind)
            }
        }
    }

    pub(super) fn exactly_matches_record(
        &self,
        record: &CandidateEvaluationRecord,
        request: &NewCandidateEvaluation,
    ) -> bool {
        match &self.outcome {
            CandidateEvaluationOutcome::Succeeded { candidates } => {
                record.exactly_matches_success(request, self.completed_at, candidates)
            }
            CandidateEvaluationOutcome::Failed { kind } => {
                record.exactly_matches_failure(request, self.completed_at, *kind)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CandidateEvaluationRecord {
    request: NewCandidateEvaluation,
    completed_at: Option<SystemTime>,
    status: CandidateEvaluationStatus,
    candidates: Vec<CompleteCandidateRecord>,
}

/// One bounded pending evaluator row selected for restart recovery. The row is
/// an immutable discovery request only; it contains no plan, approval, or
/// effect capability. `has_more` is a keyset-free bounded hint for a future
/// idle caller and must not be treated as permission to process unbounded
/// work in one turn.
pub(crate) struct PendingCandidateEvaluation {
    record: CandidateEvaluationRecord,
    has_more: bool,
}

impl PendingCandidateEvaluation {
    pub(crate) fn record(&self) -> &CandidateEvaluationRecord {
        &self.record
    }

    pub(crate) const fn has_more(&self) -> bool {
        self.has_more
    }

    fn new(record: CandidateEvaluationRecord, has_more: bool) -> Self {
        Self { record, has_more }
    }
}

impl CandidateEvaluationRecord {
    pub(crate) fn scan_id(&self) -> &ScanId {
        &self.request.scan_id
    }

    pub(crate) fn request_for_snapshot(
        &self,
        reference: &SnapshotReference,
    ) -> Result<NewCandidateEvaluation, HistoryError> {
        if self.request.snapshot_version != reference.version()
            || self.request.snapshot_sha256 != reference.digest().bytes()
            || self.request.scan_id != *reference.scan_id()
        {
            return Err(corrupt());
        }
        NewCandidateEvaluation::try_new(
            self.request.identity.clone(),
            reference,
            self.request.scheduled_at,
        )
    }

    pub(crate) const fn scheduled_at(&self) -> SystemTime {
        self.request.scheduled_at
    }

    pub(crate) const fn completed_at(&self) -> Option<SystemTime> {
        self.completed_at
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "the pending-only completion path is reserved for a future recovery worker"
        )
    )]
    pub(crate) fn status(&self) -> CandidateEvaluationStatus {
        self.status
    }

    pub(crate) fn candidates(&self) -> &[CompleteCandidateRecord] {
        &self.candidates
    }

    pub(crate) fn matches_current_scan_observation(&self, scan: &ScanRecord) -> bool {
        let Some(snapshot) = scan.snapshot() else {
            return false;
        };
        self.request.scan_id == *scan.id()
            && self.request.identity.evaluator_revision == CANDIDATE_EVALUATOR_REVISION
            && self.request.identity.rule_catalog_schema_version == CANDIDATE_CATALOG_SCHEMA_VERSION
            && self.request.identity.rule_catalog_sha256 == CANDIDATE_CATALOG_SHA256
            && self.request.identity.context_format_version == CANDIDATE_CONTEXT_FORMAT_VERSION
            && self.request.identity.context_sha256
                == candidate_evaluation_context_digest_for_observation(
                    scan.id(),
                    scan.root(),
                    scan.coverage(),
                    self.request.scheduled_at,
                )
            && self.request.snapshot_version == snapshot.version()
            && self.request.snapshot_sha256 == snapshot.digest().bytes()
    }

    pub(super) fn exactly_matches_request(&self, request: &NewCandidateEvaluation) -> bool {
        self.request == *request
    }

    pub(super) fn exactly_matches_success(
        &self,
        request: &NewCandidateEvaluation,
        completed_at: SystemTime,
        candidates: &[NewCandidateRecord],
    ) -> bool {
        let expected = candidates
            .iter()
            .map(|candidate| (candidate.candidate().id(), candidate))
            .collect::<HashMap<_, _>>();
        self.exactly_matches_request(request)
            && self.completed_at == Some(completed_at)
            && self.status
                == CandidateEvaluationStatus::Succeeded {
                    candidate_count: candidates.len() as u32,
                }
            && self.candidates.len() == candidates.len()
            && self.candidates.iter().all(|stored| {
                expected
                    .get(&stored.id)
                    .is_some_and(|expected| complete_candidate_matches_new(stored, expected))
            })
    }

    pub(super) fn exactly_matches_failure(
        &self,
        request: &NewCandidateEvaluation,
        completed_at: SystemTime,
        kind: CandidateEvaluationFailureKind,
    ) -> bool {
        self.exactly_matches_request(request)
            && self.completed_at == Some(completed_at)
            && self.status == CandidateEvaluationStatus::Failed { kind }
            && self.candidates.is_empty()
    }
}

/// One complete current evaluator result joined to its exact succeeded scan.
///
/// This owning record is a sealed durable observation, not cleanup authority.
/// It intentionally has no clone or serialization implementation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CandidateValidationSourceRecord {
    scan: ScanRecord,
    evaluation: CandidateEvaluationRecord,
    candidate: CompleteCandidateRecord,
}

impl CandidateValidationSourceRecord {
    pub(crate) fn scan(&self) -> &ScanRecord {
        &self.scan
    }

    pub(crate) fn candidate(&self) -> &CompleteCandidateRecord {
        &self.candidate
    }

    pub(crate) fn evaluation_candidates(&self) -> &[CompleteCandidateRecord] {
        self.evaluation.candidates()
    }

    pub(crate) const fn evaluation_scheduled_at(&self) -> SystemTime {
        self.evaluation.scheduled_at()
    }

    pub(crate) fn exactly_matches_after_trusted_claim(&self, current: &Self) -> bool {
        self.scan == current.scan
            && self
                .evaluation
                .exactly_matches_after_trusted_claim(&current.evaluation, self.candidate.id())
            && self.candidate.immutable_body_matches(&current.candidate)
            && self.candidate.status() == CandidateHistoryStatus::Discovered
            && current.candidate.status() == CandidateHistoryStatus::Planned
    }
}

impl CandidateEvaluationRecord {
    fn exactly_matches_after_trusted_claim(
        &self,
        current: &Self,
        claimed_candidate_id: &CandidateId,
    ) -> bool {
        self.request == current.request
            && self.completed_at == current.completed_at
            && self.status == current.status
            && self.candidates.len() == current.candidates.len()
            && self.candidates.iter().all(|retained| {
                current
                    .candidates
                    .iter()
                    .find(|candidate| candidate.id() == retained.id())
                    .is_some_and(|candidate| {
                        if retained.id() == claimed_candidate_id {
                            retained.immutable_body_matches(candidate)
                                && retained.status() == CandidateHistoryStatus::Discovered
                                && candidate.status() == CandidateHistoryStatus::Planned
                        } else {
                            retained == candidate
                        }
                    })
            })
    }
}

/// Exact, bounded candidate-evaluation history for one requested scan.
///
/// These variants contain immutable historical observations only. They cannot
/// reconstruct a domain candidate, create a cleanup plan, or grant execution
/// authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CandidateEvaluationObservation {
    MissingScan,
    NotRun { scan_status: ScanStatus },
    Pending(CandidateEvaluationRecord),
    Succeeded(CandidateEvaluationRecord),
    Failed(CandidateEvaluationRecord),
}

pub(super) struct PreparedCandidateEvaluation {
    scan_id: String,
    evaluator_revision: i64,
    rule_catalog_schema_version: i64,
    rule_catalog_sha256: [u8; 32],
    context_format_version: i64,
    context_sha256: [u8; 32],
    snapshot_version: i64,
    snapshot_sha256: [u8; 32],
    scheduled_at_unix_ms: i64,
}

impl PreparedCandidateEvaluation {
    pub(super) fn prepare(request: &NewCandidateEvaluation) -> Result<Self, HistoryError> {
        Ok(Self {
            scan_id: request.scan_id.as_str().to_owned(),
            evaluator_revision: i64::from(request.identity.evaluator_revision),
            rule_catalog_schema_version: i64::from(request.identity.rule_catalog_schema_version),
            rule_catalog_sha256: request.identity.rule_catalog_sha256,
            context_format_version: i64::from(request.identity.context_format_version),
            context_sha256: request.identity.context_sha256,
            snapshot_version: i64::from(request.snapshot_version),
            snapshot_sha256: request.snapshot_sha256,
            scheduled_at_unix_ms: system_time_to_unix_ms(
                request.scheduled_at,
                HistoryErrorKind::InvalidInput,
            )?,
        })
    }
}

pub(super) fn insert_candidate_evaluation_pending(
    transaction: &Transaction<'_>,
    prepared: &PreparedCandidateEvaluation,
) -> Result<(), HistoryError> {
    validate_scan_and_snapshot(transaction, prepared)?;
    if count_candidates(transaction, &prepared.scan_id)? != 0 {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    let changed = transaction
        .execute(
            "INSERT INTO candidate_evaluations (
                 scan_id, record_format_version, evaluator_revision,
                 rule_catalog_schema_version, rule_catalog_sha256,
                 context_format_version, context_sha256,
                 snapshot_version, snapshot_sha256, scheduled_at_unix_ms,
                 status
             ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending')
             ON CONFLICT(scan_id) DO NOTHING",
            params![
                prepared.scan_id,
                prepared.evaluator_revision,
                prepared.rule_catalog_schema_version,
                prepared.rule_catalog_sha256.as_slice(),
                prepared.context_format_version,
                prepared.context_sha256.as_slice(),
                prepared.snapshot_version,
                prepared.snapshot_sha256.as_slice(),
                prepared.scheduled_at_unix_ms,
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
    }
    Ok(())
}

pub(super) fn finalize_candidate_evaluation_success(
    transaction: &Transaction<'_>,
    request: &NewCandidateEvaluation,
    completed_at: SystemTime,
    candidates: &[PreparedCandidate],
) -> Result<(), HistoryError> {
    if candidates.len() > MAX_EVALUATED_CANDIDATES {
        return Err(invalid());
    }
    let prepared = PreparedCandidateEvaluation::prepare(request)?;
    let completed_at_unix_ms =
        system_time_to_unix_ms(completed_at, HistoryErrorKind::InvalidInput)?;
    validate_pending(transaction, &prepared)?;
    if completed_at_unix_ms < prepared.scheduled_at_unix_ms
        || count_candidates(transaction, &prepared.scan_id)? != 0
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    for candidate in candidates {
        insert_candidate(transaction, candidate)?;
    }
    let candidate_count = i64::try_from(candidates.len()).map_err(|_| invalid())?;
    let changed = transaction
        .execute(
            "UPDATE candidate_evaluations
             SET status = 'succeeded', completed_at_unix_ms = ?2,
                 candidate_count = ?3
             WHERE scan_id = ?1 AND status = 'pending'
               AND completed_at_unix_ms IS NULL AND candidate_count IS NULL
               AND failure_kind IS NULL",
            params![prepared.scan_id, completed_at_unix_ms, candidate_count],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok(())
}

pub(super) fn finalize_candidate_evaluation_failure(
    transaction: &Transaction<'_>,
    request: &NewCandidateEvaluation,
    completed_at: SystemTime,
    kind: CandidateEvaluationFailureKind,
) -> Result<(), HistoryError> {
    let prepared = PreparedCandidateEvaluation::prepare(request)?;
    let completed_at_unix_ms =
        system_time_to_unix_ms(completed_at, HistoryErrorKind::InvalidInput)?;
    validate_pending(transaction, &prepared)?;
    if completed_at_unix_ms < prepared.scheduled_at_unix_ms
        || count_candidates(transaction, &prepared.scan_id)? != 0
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    let changed = transaction
        .execute(
            "UPDATE candidate_evaluations
             SET status = 'failed', completed_at_unix_ms = ?2, failure_kind = ?3
             WHERE scan_id = ?1 AND status = 'pending'
               AND completed_at_unix_ms IS NULL AND candidate_count IS NULL
               AND failure_kind IS NULL",
            params![prepared.scan_id, completed_at_unix_ms, kind.as_stored()],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok(())
}

pub(super) fn load_candidate_evaluation(
    connection: &Connection,
    scan_id: &ScanId,
) -> Result<Option<CandidateEvaluationRecord>, HistoryError> {
    run_bounded_evaluation_query(connection, || {
        load_candidate_evaluation_within_budget(connection, scan_id)
    })
}

/// Select at most one pending evaluation in deterministic schedule/ID order.
/// The two-row sentinel keeps discovery bounded while allowing the caller to
/// yield and request another recovery turn. Payload validation is delegated to
/// the exact-row decoder, so malformed state fails closed before any replay.
pub(super) fn load_pending_candidate_evaluation(
    connection: &Connection,
) -> Result<Option<PendingCandidateEvaluation>, HistoryError> {
    run_bounded_evaluation_query(connection, || {
        let mut statement = connection
            .prepare(
                "SELECT typeof(scan_id), length(CAST(scan_id AS BLOB)), scan_id
                 FROM candidate_evaluations
                 WHERE status = 'pending'
                 ORDER BY scheduled_at_unix_ms ASC, scan_id ASC
                 LIMIT 2",
            )
            .map_err(map_query_sql_error)?;
        let mut rows = statement.query([]).map_err(map_query_sql_error)?;
        let mut scan_ids = Vec::new();
        while let Some(row) = rows.next().map_err(map_query_sql_error)? {
            let storage_type: String = row.get(0).map_err(map_query_sql_error)?;
            let length: i64 = row.get(1).map_err(map_query_sql_error)?;
            if storage_type != "text" || !(1..=MAX_STORED_ID_BYTES).contains(&length) {
                return Err(corrupt());
            }
            let value: String = row.get(2).map_err(map_query_sql_error)?;
            scan_ids.push(ScanId::new(value).map_err(|_| corrupt())?);
        }
        let Some(scan_id) = scan_ids.first() else {
            return Ok(None);
        };
        let record =
            load_candidate_evaluation_within_budget(connection, scan_id)?.ok_or_else(corrupt)?;
        if record.status != CandidateEvaluationStatus::Pending {
            return Err(corrupt());
        }
        Ok(Some(PendingCandidateEvaluation::new(
            record,
            scan_ids.len() > 1,
        )))
    })
}

pub(super) fn load_candidate_evaluation_for_scan(
    connection: &Connection,
    scan_id: &ScanId,
) -> Result<CandidateEvaluationObservation, HistoryError> {
    run_bounded_evaluation_query(connection, || {
        let Some(scan) = load_scan_record_within_budget(connection, scan_id)? else {
            return Ok(CandidateEvaluationObservation::MissingScan);
        };
        let Some(record) = load_candidate_evaluation_within_budget(connection, scan_id)? else {
            return Ok(CandidateEvaluationObservation::NotRun {
                scan_status: scan.status(),
            });
        };
        Ok(observation_from_record(record))
    })
}

pub(super) fn load_candidate_validation_source(
    connection: &Connection,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
) -> Result<CandidateValidationSourceRecord, HistoryError> {
    run_bounded_evaluation_query(connection, || {
        load_candidate_validation_source_with_status(
            connection,
            scan_id,
            candidate_id,
            CandidateHistoryStatus::Discovered,
        )
    })
}

pub(super) fn load_candidate_validation_source_for_trusted_claim(
    connection: &Connection,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
    session_id: &CleanupSessionId,
    item_ordinal: usize,
) -> Result<CandidateValidationSourceRecord, HistoryError> {
    run_bounded_evaluation_query(connection, || {
        let journal = load_cleanup_journal_within_budget(connection, session_id)?
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
        if journal.session_id != *session_id
            || journal.source_scan_id != *scan_id
            || journal.candidate_status_coupling
                != CandidateStatusCoupling::TrustedRustTargetPlanClaimsV1
            || !matches!(
                journal.lifecycle,
                JournalLifecycle::Planned | JournalLifecycle::Active { .. }
            )
            || journal.items.len() != 1
            || journal
                .items
                .get(item_ordinal)
                .is_none_or(|item| item.frozen.candidate_id != *candidate_id)
        {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        load_candidate_validation_source_with_status(
            connection,
            scan_id,
            candidate_id,
            CandidateHistoryStatus::Planned,
        )
    })
}

fn load_candidate_validation_source_with_status(
    connection: &Connection,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
    required_status: CandidateHistoryStatus,
) -> Result<CandidateValidationSourceRecord, HistoryError> {
    let scan = load_scan_record_within_budget(connection, scan_id)?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    if scan.status() != ScanStatus::Succeeded
        || scan.coverage().status() != ScanCoverageStatus::Complete
        || scan.snapshot().is_none()
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    let evaluation = load_candidate_evaluation_within_budget(connection, scan_id)?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    if !matches!(
        evaluation.status(),
        CandidateEvaluationStatus::Succeeded { .. }
    ) || !evaluation.matches_current_scan_observation(&scan)
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    let candidate = evaluation
        .candidates()
        .iter()
        .find(|candidate| candidate.id() == candidate_id)
        .cloned()
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    if candidate.source_scan_id() != scan_id || candidate.status() != required_status {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok(CandidateValidationSourceRecord {
        scan,
        evaluation,
        candidate,
    })
}

fn observation_from_record(record: CandidateEvaluationRecord) -> CandidateEvaluationObservation {
    match record.status() {
        CandidateEvaluationStatus::Pending => CandidateEvaluationObservation::Pending(record),
        CandidateEvaluationStatus::Succeeded { .. } => {
            CandidateEvaluationObservation::Succeeded(record)
        }
        CandidateEvaluationStatus::Failed { .. } => CandidateEvaluationObservation::Failed(record),
    }
}

/// Candidate evaluations are capped at 4,096 complete parent graphs. The
/// ordinary exact-record budget is intentionally small and cannot decode that
/// valid maximum even with set-based reads, so this boundary has its own fixed
/// budget: 100,000 base VM steps plus 1,200 per possible candidate. The larger
/// linear term includes scalar-only type/length/materialization preflights;
/// SQL-side `LIMIT max + 1` sentinels remain the primary cardinality bound.
fn run_bounded_evaluation_query<T>(
    connection: &Connection,
    query: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    let started_at = Instant::now();
    let mut callbacks = 0_u64;
    let maximum_callbacks = EVALUATION_QUERY_BASE_CALLBACKS.saturating_add(
        EVALUATION_QUERY_CALLBACKS_PER_CANDIDATE.saturating_mul(MAX_EVALUATED_CANDIDATES as u64),
    );
    connection
        .progress_handler(
            EVALUATION_QUERY_PROGRESS_INTERVAL,
            Some(move || {
                callbacks = callbacks.saturating_add(1);
                callbacks >= maximum_callbacks
                    || started_at.elapsed() >= EVALUATION_QUERY_MAX_ELAPSED
            }),
        )
        .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
    let mut guard = EvaluationQueryProgressGuard {
        connection,
        installed: true,
    };
    let result = query();
    guard.remove()?;
    let value = result?;
    if started_at.elapsed() >= EVALUATION_QUERY_MAX_ELAPSED {
        return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
    }
    Ok(value)
}

struct EvaluationQueryProgressGuard<'connection> {
    connection: &'connection Connection,
    installed: bool,
}

impl EvaluationQueryProgressGuard<'_> {
    fn remove(&mut self) -> Result<(), HistoryError> {
        self.connection
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for EvaluationQueryProgressGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self.connection.progress_handler(0, None::<fn() -> bool>);
        }
    }
}

pub(super) fn load_candidate_evaluation_within_budget(
    connection: &Connection,
    scan_id: &ScanId,
) -> Result<Option<CandidateEvaluationRecord>, HistoryError> {
    let raw = connection
        .query_row(
            "SELECT typeof(scan_id), length(CAST(scan_id AS BLOB)), scan_id,
                    record_format_version, evaluator_revision,
                    rule_catalog_schema_version,
                    typeof(rule_catalog_sha256), length(rule_catalog_sha256),
                    rule_catalog_sha256, context_format_version,
                    typeof(context_sha256), length(context_sha256), context_sha256,
                    snapshot_version, typeof(snapshot_sha256), length(snapshot_sha256),
                    snapshot_sha256, scheduled_at_unix_ms, completed_at_unix_ms,
                    typeof(status), length(CAST(status AS BLOB)), status,
                    candidate_count, typeof(failure_kind),
                    COALESCE(length(CAST(failure_kind AS BLOB)), 0), failure_kind,
                    typeof(record_format_version), typeof(evaluator_revision),
                    typeof(rule_catalog_schema_version), typeof(context_format_version),
                    typeof(snapshot_version), typeof(scheduled_at_unix_ms),
                    typeof(completed_at_unix_ms), typeof(candidate_count)
             FROM candidate_evaluations WHERE scan_id = ?1",
            [scan_id.as_str()],
            raw_evaluation_row,
        )
        .optional()
        .map_err(map_query_sql_error)?;
    raw.map(|raw| decode_evaluation(connection, raw))
        .transpose()
}

fn validate_pending(
    connection: &Connection,
    prepared: &PreparedCandidateEvaluation,
) -> Result<(), HistoryError> {
    let scan_id = ScanId::new(prepared.scan_id.clone()).map_err(|_| corrupt())?;
    let Some(record) = load_candidate_evaluation_within_budget(connection, &scan_id)? else {
        return Err(HistoryError::new(HistoryErrorKind::NotFound));
    };
    let expected = request_from_prepared(prepared)?;
    if !record.exactly_matches_request(&expected)
        || record.status != CandidateEvaluationStatus::Pending
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok(())
}

fn validate_scan_and_snapshot(
    connection: &Connection,
    prepared: &PreparedCandidateEvaluation,
) -> Result<(), HistoryError> {
    let scan_id = ScanId::new(prepared.scan_id.clone()).map_err(|_| invalid())?;
    let Some(scan) = load_scan_record_within_budget(connection, &scan_id)? else {
        return Err(HistoryError::new(HistoryErrorKind::NotFound));
    };
    let snapshot = scan
        .snapshot()
        .filter(|snapshot| {
            i64::from(snapshot.version()) == prepared.snapshot_version
                && snapshot.digest().bytes() == prepared.snapshot_sha256
        })
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
    let completed_at = scan
        .completed_at()
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
    if scan.status() != ScanStatus::Succeeded
        || snapshot.scan_id() != &scan_id
        || system_time_to_unix_ms(completed_at, HistoryErrorKind::CorruptData)?
            > prepared.scheduled_at_unix_ms
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok(())
}

fn count_candidates(connection: &Connection, scan_id: &str) -> Result<usize, HistoryError> {
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM candidates WHERE scan_id = ?1",
            [scan_id],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    let count = usize::try_from(count).map_err(|_| corrupt())?;
    if count > MAX_EVALUATED_CANDIDATES {
        return Err(corrupt());
    }
    Ok(count)
}

struct RawEvaluationRow {
    scan_id: String,
    evaluator_revision: i64,
    rule_catalog_schema_version: i64,
    rule_catalog_sha256: Vec<u8>,
    context_format_version: i64,
    context_sha256: Vec<u8>,
    snapshot_version: i64,
    snapshot_sha256: Vec<u8>,
    scheduled_at_unix_ms: i64,
    completed_at_unix_ms: Option<i64>,
    status: String,
    candidate_count: Option<i64>,
    failure_kind: Option<String>,
}

fn raw_evaluation_row(row: &Row<'_>) -> rusqlite::Result<RawEvaluationRow> {
    if row.get::<_, i64>(3)? != 1 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    for index in 26..=31 {
        if row.get::<_, String>(index)? != "integer" {
            return Err(rusqlite::Error::InvalidQuery);
        }
    }
    for index in 32..=33 {
        let storage_type: String = row.get(index)?;
        if storage_type != "null" && storage_type != "integer" {
            return Err(rusqlite::Error::InvalidQuery);
        }
    }
    validate_type_and_length(row, 0, 1, "text", 1, MAX_STORED_ID_BYTES)?;
    validate_type_and_length(row, 6, 7, "blob", 32, 32)?;
    validate_type_and_length(row, 10, 11, "blob", 32, 32)?;
    validate_type_and_length(row, 14, 15, "blob", 32, 32)?;
    validate_type_and_length(row, 19, 20, "text", 1, 16)?;
    let failure_type: String = row.get(23)?;
    let failure_length: i64 = row.get(24)?;
    if !((failure_type == "null" && failure_length == 0)
        || (failure_type == "text" && (1..=MAX_FAILURE_KIND_BYTES).contains(&failure_length)))
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let scheduled_at_unix_ms: i64 = row.get(17)?;
    let completed_at_unix_ms: Option<i64> = row.get(18)?;
    if completed_at_unix_ms.is_some_and(|completed| completed < scheduled_at_unix_ms) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(RawEvaluationRow {
        scan_id: row.get(2)?,
        evaluator_revision: row.get(4)?,
        rule_catalog_schema_version: row.get(5)?,
        rule_catalog_sha256: row.get(8)?,
        context_format_version: row.get(9)?,
        context_sha256: row.get(12)?,
        snapshot_version: row.get(13)?,
        snapshot_sha256: row.get(16)?,
        scheduled_at_unix_ms,
        completed_at_unix_ms,
        status: row.get(21)?,
        candidate_count: row.get(22)?,
        failure_kind: row.get(25)?,
    })
}

fn decode_evaluation(
    connection: &Connection,
    raw: RawEvaluationRow,
) -> Result<CandidateEvaluationRecord, HistoryError> {
    let scan_id = ScanId::new(raw.scan_id).map_err(|_| corrupt())?;
    let identity = CandidateEvaluationIdentity::try_new(
        u32::try_from(raw.evaluator_revision).map_err(|_| corrupt())?,
        u32::try_from(raw.rule_catalog_schema_version).map_err(|_| corrupt())?,
        array32(raw.rule_catalog_sha256)?,
        u32::try_from(raw.context_format_version).map_err(|_| corrupt())?,
        array32(raw.context_sha256)?,
    )
    .map_err(|_| corrupt())?;
    let scheduled_at = unix_ms_to_system_time(raw.scheduled_at_unix_ms)?;
    let request = NewCandidateEvaluation {
        scan_id: scan_id.clone(),
        identity,
        snapshot_version: u32::try_from(raw.snapshot_version).map_err(|_| corrupt())?,
        snapshot_sha256: array32(raw.snapshot_sha256)?,
        scheduled_at,
    };
    let completed_at = raw
        .completed_at_unix_ms
        .map(unix_ms_to_system_time)
        .transpose()?;
    let status = match raw.status.as_str() {
        "pending"
            if completed_at.is_none()
                && raw.candidate_count.is_none()
                && raw.failure_kind.is_none() =>
        {
            CandidateEvaluationStatus::Pending
        }
        "succeeded" if completed_at.is_some() && raw.failure_kind.is_none() => {
            let count = raw.candidate_count.ok_or_else(corrupt)?;
            CandidateEvaluationStatus::Succeeded {
                candidate_count: u32::try_from(count).map_err(|_| corrupt())?,
            }
        }
        "failed" if completed_at.is_some() && raw.candidate_count.is_none() => {
            CandidateEvaluationStatus::Failed {
                kind: CandidateEvaluationFailureKind::from_stored(
                    raw.failure_kind.as_deref().ok_or_else(corrupt)?,
                )?,
            }
        }
        _ => return Err(corrupt()),
    };
    let expected_count = match status {
        CandidateEvaluationStatus::Succeeded { candidate_count } => {
            let count = usize::try_from(candidate_count).map_err(|_| corrupt())?;
            if count > MAX_EVALUATED_CANDIDATES {
                return Err(corrupt());
            }
            count
        }
        CandidateEvaluationStatus::Pending | CandidateEvaluationStatus::Failed { .. } => 0,
    };
    let prepared = PreparedCandidateEvaluation::prepare(&request)?;
    validate_scan_and_snapshot(connection, &prepared).map_err(|_| corrupt())?;
    let candidates =
        load_complete_candidate_batch_within_budget(connection, &scan_id, expected_count)?;
    if candidates.len() != expected_count {
        return Err(corrupt());
    }
    if let Some(completed_at) = completed_at
        && candidates
            .iter()
            .any(|candidate| candidate.created_at != completed_at)
    {
        return Err(corrupt());
    }
    Ok(CandidateEvaluationRecord {
        request,
        completed_at,
        status,
        candidates,
    })
}

fn request_from_prepared(
    prepared: &PreparedCandidateEvaluation,
) -> Result<NewCandidateEvaluation, HistoryError> {
    Ok(NewCandidateEvaluation {
        scan_id: ScanId::new(prepared.scan_id.clone()).map_err(|_| corrupt())?,
        identity: CandidateEvaluationIdentity::try_new(
            u32::try_from(prepared.evaluator_revision).map_err(|_| corrupt())?,
            u32::try_from(prepared.rule_catalog_schema_version).map_err(|_| corrupt())?,
            prepared.rule_catalog_sha256,
            u32::try_from(prepared.context_format_version).map_err(|_| corrupt())?,
            prepared.context_sha256,
        )
        .map_err(|_| corrupt())?,
        snapshot_version: u32::try_from(prepared.snapshot_version).map_err(|_| corrupt())?,
        snapshot_sha256: prepared.snapshot_sha256,
        scheduled_at: unix_ms_to_system_time(prepared.scheduled_at_unix_ms)?,
    })
}

fn validate_type_and_length(
    row: &Row<'_>,
    type_index: usize,
    length_index: usize,
    expected_type: &str,
    minimum_length: i64,
    maximum_length: i64,
) -> rusqlite::Result<()> {
    let storage_type: String = row.get(type_index)?;
    let length: i64 = row.get(length_index)?;
    if storage_type != expected_type || !(minimum_length..=maximum_length).contains(&length) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn array32(value: Vec<u8>) -> Result<[u8; 32], HistoryError> {
    value.try_into().map_err(|_| corrupt())
}

const fn invalid() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidInput)
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use rusqlite::Connection;

    use super::*;
    use crate::persistence::history::{NewScanRecord, PreparedNewScan, insert_scan_started};
    use crate::persistence::migrations::apply_pending_migrations;

    fn record(status: CandidateEvaluationStatus) -> CandidateEvaluationRecord {
        let scheduled_at = UNIX_EPOCH + Duration::from_secs(10);
        let completed_at = match status {
            CandidateEvaluationStatus::Pending => None,
            CandidateEvaluationStatus::Succeeded { .. }
            | CandidateEvaluationStatus::Failed { .. } => {
                Some(UNIX_EPOCH + Duration::from_secs(11))
            }
        };
        CandidateEvaluationRecord {
            request: NewCandidateEvaluation {
                scan_id: ScanId::new("scan:observation").unwrap(),
                identity: CandidateEvaluationIdentity::try_new(1, 1, [1; 32], 1, [2; 32]).unwrap(),
                snapshot_version: 1,
                snapshot_sha256: [3; 32],
                scheduled_at,
            },
            completed_at,
            status,
            candidates: Vec::new(),
        }
    }

    #[test]
    fn exact_scan_query_distinguishes_missing_and_not_run() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_pending_migrations(&mut connection, 1).unwrap();

        let missing = ScanId::new("scan:missing").unwrap();
        assert_eq!(
            load_candidate_evaluation_for_scan(&connection, &missing).unwrap(),
            CandidateEvaluationObservation::MissingScan
        );

        let running = ScanId::new("scan:not-run").unwrap();
        let scan = NewScanRecord::try_new(
            running.clone(),
            std::env::temp_dir().join("dux-candidate-observation"),
            UNIX_EPOCH + Duration::from_secs(1),
        )
        .unwrap();
        let prepared = PreparedNewScan::prepare(&scan).unwrap();
        let transaction = connection.transaction().unwrap();
        insert_scan_started(&transaction, &prepared).unwrap();
        transaction.commit().unwrap();

        assert_eq!(
            load_candidate_evaluation_for_scan(&connection, &running).unwrap(),
            CandidateEvaluationObservation::NotRun {
                scan_status: ScanStatus::Running
            }
        );
    }

    #[test]
    fn recorded_observation_has_one_explicit_variant_per_lifecycle_state() {
        let pending = record(CandidateEvaluationStatus::Pending);
        assert!(matches!(
            observation_from_record(pending),
            CandidateEvaluationObservation::Pending(_)
        ));

        let succeeded = record(CandidateEvaluationStatus::Succeeded { candidate_count: 0 });
        assert!(matches!(
            observation_from_record(succeeded),
            CandidateEvaluationObservation::Succeeded(_)
        ));

        let failed = record(CandidateEvaluationStatus::Failed {
            kind: CandidateEvaluationFailureKind::EvaluationFailed,
        });
        assert!(matches!(
            observation_from_record(failed),
            CandidateEvaluationObservation::Failed(_)
        ));
    }
}
