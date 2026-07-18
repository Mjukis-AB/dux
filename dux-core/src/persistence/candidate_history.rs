//! Typed, non-authoritative candidate history stored in SQLite schema v2.
//!
//! Complete records preserve deterministic discovery facts for presentation and
//! comparison. They are not current filesystem evidence, planner input, or an
//! executable capability. Migrated v1 summaries remain explicitly incomplete.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::domain::{
    BlockReason, Candidate, CandidateAction, CandidateCategory, CandidateId, Evidence, RuleId,
    RuleRef, RuleRevision, SafetyTier, ScanId,
};

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::history::{
    HistoryError, HistoryErrorKind, ScanStatus, load_scan_record, load_scan_record_within_budget,
    map_query_sql_error, map_write_sql_error, run_bounded_query,
};

const MAX_PATHS: usize = 256;
const MAX_EVIDENCE: usize = 512;
const MAX_BLOCKERS: usize = 64;
const MAX_TEXT_BYTES: usize = 4_096;
const MAX_STORED_ID_BYTES: i64 = 128;
const MAX_STORED_PATH_BYTES: i64 = 65_536;
const MAX_STORED_POLICY_BYTES: i64 = 64;
const MAX_STORED_TEXT_BYTES: i64 = MAX_TEXT_BYTES as i64;

// The evaluator cardinality bounds alone permit schema-valid graphs far too
// large to decode safely in one process (paths and evidence may each carry a
// 64 KiB host value). This conservative aggregate charge covers decoded
// objects, query-row copies, collection overhead, and three copies of every
// variable payload. Reads preflight the charge before payload-bearing
// materialized CTEs; evaluation preparation and the store writer use the same
// calculation before mutation so every accepted batch remains reloadable.
const MAX_CANDIDATE_BATCH_MATERIALIZED_BYTES: u64 = 32 * 1024 * 1024;
const MATERIALIZED_CANDIDATE_CHARGE: u64 = 2_048;
const MATERIALIZED_PATH_CHARGE: u64 = 384;
const MATERIALIZED_EVIDENCE_CHARGE: u64 = 512;
const MATERIALIZED_BLOCKER_CHARGE: u64 = 256;
const MATERIALIZED_PAYLOAD_MULTIPLIER: u64 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CandidateBatchUsage {
    candidates: u64,
    paths: u64,
    path_payload_bytes: u64,
    evidence: u64,
    evidence_payload_bytes: u64,
    blockers: u64,
}

#[derive(Default)]
pub(crate) struct CandidateBatchMaterializationBudget {
    usage: CandidateBatchUsage,
}

impl CandidateBatchMaterializationBudget {
    pub(crate) fn charge_observed_candidate(
        &mut self,
        path: &Path,
        evidence_paths: &[PathBuf],
        blocker_count: usize,
    ) -> Result<bool, HistoryError> {
        let path_bytes = prepare_absolute_path(path)?;
        let evidence_payload_bytes = evidence_paths.iter().try_fold(
            u64::try_from(path_bytes.bytes.len()).map_err(|_| invalid())?,
            |total, evidence| {
                let encoded = prepare_absolute_path(evidence)?;
                total
                    .checked_add(u64::try_from(encoded.bytes.len()).map_err(|_| invalid())?)
                    .ok_or_else(invalid)
            },
        )?;
        let next = self.usage.checked_add(
            CandidateBatchUsage {
                candidates: 1,
                paths: 1,
                path_payload_bytes: u64::try_from(path_bytes.bytes.len()).map_err(|_| invalid())?,
                evidence: u64::try_from(evidence_paths.len())
                    .map_err(|_| invalid())?
                    .checked_add(1)
                    .ok_or_else(invalid)?,
                evidence_payload_bytes,
                blockers: u64::try_from(blocker_count).map_err(|_| invalid())?,
            },
            HistoryErrorKind::InvalidInput,
        )?;
        if next.materialized_bytes(HistoryErrorKind::InvalidInput)?
            > MAX_CANDIDATE_BATCH_MATERIALIZED_BYTES
        {
            return Ok(false);
        }
        self.usage = next;
        Ok(true)
    }
}

impl CandidateBatchUsage {
    fn checked_add(self, other: Self, kind: HistoryErrorKind) -> Result<Self, HistoryError> {
        let add = |left: u64, right: u64| {
            left.checked_add(right)
                .ok_or_else(|| HistoryError::new(kind))
        };
        Ok(Self {
            candidates: add(self.candidates, other.candidates)?,
            paths: add(self.paths, other.paths)?,
            path_payload_bytes: add(self.path_payload_bytes, other.path_payload_bytes)?,
            evidence: add(self.evidence, other.evidence)?,
            evidence_payload_bytes: add(self.evidence_payload_bytes, other.evidence_payload_bytes)?,
            blockers: add(self.blockers, other.blockers)?,
        })
    }

    fn materialized_bytes(self, kind: HistoryErrorKind) -> Result<u64, HistoryError> {
        let multiply = |value: u64, factor: u64| {
            value
                .checked_mul(factor)
                .ok_or_else(|| HistoryError::new(kind))
        };
        let mut bytes = multiply(self.candidates, MATERIALIZED_CANDIDATE_CHARGE)?;
        for charge in [
            multiply(self.paths, MATERIALIZED_PATH_CHARGE)?,
            multiply(self.evidence, MATERIALIZED_EVIDENCE_CHARGE)?,
            multiply(self.blockers, MATERIALIZED_BLOCKER_CHARGE)?,
            multiply(self.path_payload_bytes, MATERIALIZED_PAYLOAD_MULTIPLIER)?,
            multiply(self.evidence_payload_bytes, MATERIALIZED_PAYLOAD_MULTIPLIER)?,
        ] {
            bytes = bytes
                .checked_add(charge)
                .ok_or_else(|| HistoryError::new(kind))?;
        }
        Ok(bytes)
    }

    fn ensure_within_budget(self, kind: HistoryErrorKind) -> Result<(), HistoryError> {
        if self.materialized_bytes(kind)? > MAX_CANDIDATE_BATCH_MATERIALIZED_BYTES {
            return Err(HistoryError::new(kind));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CandidateHistoryStatus {
    Discovered,
    Selected,
    Dismissed,
    Stale,
    Planned,
    Completed,
    Failed,
    Unavailable,
}

impl CandidateHistoryStatus {
    fn as_stored(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Selected => "selected",
            Self::Dismissed => "dismissed",
            Self::Stale => "stale",
            Self::Planned => "planned",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Unavailable => "unavailable",
        }
    }

    pub(super) fn from_stored(value: &str) -> Result<Self, HistoryError> {
        match value {
            "discovered" => Ok(Self::Discovered),
            "selected" => Ok(Self::Selected),
            "dismissed" => Ok(Self::Dismissed),
            "stale" => Ok(Self::Stale),
            "planned" => Ok(Self::Planned),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "unavailable" => Ok(Self::Unavailable),
            _ => Err(corrupt()),
        }
    }
}

/// User review intent only. These transitions do not approve a plan or effect.
/// Evaluator, planner, and journal-owned statuses deliberately have no generic
/// transition surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CandidateReviewTransition {
    Select,
    ClearSelection,
    DismissDiscovered,
    DismissSelected,
    Restore,
}

/// Evaluator-owned invalidation of one scan-bound observation. These terminal
/// projections cannot be used to select, plan, or complete cleanup work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CandidateEvaluationTransition {
    DiscoveredToStale,
    SelectedToStale,
    DismissedToStale,
    UnavailableToStale,
    DiscoveredToUnavailable,
    SelectedToUnavailable,
    DismissedToUnavailable,
}

impl CandidateEvaluationTransition {
    fn statuses(self) -> (CandidateHistoryStatus, CandidateHistoryStatus) {
        match self {
            Self::DiscoveredToStale => (
                CandidateHistoryStatus::Discovered,
                CandidateHistoryStatus::Stale,
            ),
            Self::SelectedToStale => (
                CandidateHistoryStatus::Selected,
                CandidateHistoryStatus::Stale,
            ),
            Self::DismissedToStale => (
                CandidateHistoryStatus::Dismissed,
                CandidateHistoryStatus::Stale,
            ),
            Self::UnavailableToStale => (
                CandidateHistoryStatus::Unavailable,
                CandidateHistoryStatus::Stale,
            ),
            Self::DiscoveredToUnavailable => (
                CandidateHistoryStatus::Discovered,
                CandidateHistoryStatus::Unavailable,
            ),
            Self::SelectedToUnavailable => (
                CandidateHistoryStatus::Selected,
                CandidateHistoryStatus::Unavailable,
            ),
            Self::DismissedToUnavailable => (
                CandidateHistoryStatus::Dismissed,
                CandidateHistoryStatus::Unavailable,
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CandidatePriorReviewStatus {
    Discovered,
    Selected,
}

impl CandidatePriorReviewStatus {
    pub(super) fn as_stored(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Selected => "selected",
        }
    }

    pub(super) fn from_stored(value: &str) -> Result<Self, HistoryError> {
        match value {
            "discovered" => Ok(Self::Discovered),
            "selected" => Ok(Self::Selected),
            _ => Err(corrupt()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CandidatePlanSettlement {
    Restore(CandidatePriorReviewStatus),
    Completed,
    Failed,
}

impl CandidateReviewTransition {
    fn statuses(self) -> (CandidateHistoryStatus, CandidateHistoryStatus) {
        match self {
            Self::Select => (
                CandidateHistoryStatus::Discovered,
                CandidateHistoryStatus::Selected,
            ),
            Self::ClearSelection => (
                CandidateHistoryStatus::Selected,
                CandidateHistoryStatus::Discovered,
            ),
            Self::DismissDiscovered => (
                CandidateHistoryStatus::Discovered,
                CandidateHistoryStatus::Dismissed,
            ),
            Self::DismissSelected => (
                CandidateHistoryStatus::Selected,
                CandidateHistoryStatus::Dismissed,
            ),
            Self::Restore => (
                CandidateHistoryStatus::Dismissed,
                CandidateHistoryStatus::Discovered,
            ),
        }
    }

    fn validate_candidate(self, candidate: &CompleteCandidateRecord) -> Result<(), HistoryError> {
        let (expected, target) = self.statuses();
        if (self == Self::Select
            && (!candidate.action.is_cleanup_operation() || !candidate.blockers.is_empty()))
            || (candidate.status != expected && candidate.status != target)
        {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NewCandidateRecord {
    candidate: Candidate,
    created_at: SystemTime,
}

impl NewCandidateRecord {
    pub(crate) fn try_from_candidate(
        candidate: &Candidate,
        created_at: SystemTime,
    ) -> Result<Self, HistoryError> {
        validate_candidate(candidate, HistoryErrorKind::InvalidInput)?;
        let created_at_unix_ms =
            system_time_to_unix_ms(created_at, HistoryErrorKind::InvalidInput)?;
        Ok(Self {
            candidate: candidate.clone(),
            // The inherited v1 parent column is millisecond-granular. Keep
            // the normalized value here so sub-millisecond input is never
            // mistaken for data that the record can round-trip.
            created_at: unix_ms_to_system_time(created_at_unix_ms)
                .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?,
        })
    }

    pub(crate) fn candidate(&self) -> &Candidate {
        &self.candidate
    }

    pub(crate) fn created_at(&self) -> SystemTime {
        self.created_at
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StoredCandidateRecord {
    LegacySummary(LegacyCandidateSummary),
    Complete(CompleteCandidateRecord),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LegacyCandidateSummary {
    pub(crate) id: CandidateId,
    pub(crate) source_scan_id: ScanId,
    pub(crate) rule: RuleRef,
    pub(crate) safety: SafetyTier,
    pub(crate) estimated_bytes: u64,
    pub(crate) created_at: SystemTime,
    pub(crate) status: CandidateHistoryStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompleteCandidateRecord {
    pub(crate) id: CandidateId,
    pub(crate) source_scan_id: ScanId,
    pub(crate) rule: RuleRef,
    pub(crate) category: CandidateCategory,
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) estimated_bytes: u64,
    pub(crate) newest_mtime: Option<SystemTime>,
    pub(crate) evidence: Vec<Evidence>,
    pub(crate) safety: SafetyTier,
    pub(crate) action: CandidateAction,
    pub(crate) rule_schedule_eligible: bool,
    pub(crate) blockers: Vec<BlockReason>,
    pub(crate) created_at: SystemTime,
    pub(crate) status: CandidateHistoryStatus,
}

impl CompleteCandidateRecord {
    pub(crate) fn id(&self) -> &CandidateId {
        &self.id
    }

    pub(crate) fn source_scan_id(&self) -> &ScanId {
        &self.source_scan_id
    }

    pub(crate) fn rule(&self) -> &RuleRef {
        &self.rule
    }

    pub(crate) const fn category(&self) -> CandidateCategory {
        self.category
    }

    pub(crate) fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub(crate) const fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }

    pub(crate) const fn newest_mtime(&self) -> Option<SystemTime> {
        self.newest_mtime
    }

    pub(crate) fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }

    pub(crate) const fn safety(&self) -> SafetyTier {
        self.safety
    }

    pub(crate) const fn action(&self) -> CandidateAction {
        self.action
    }

    pub(crate) const fn rule_schedule_eligible(&self) -> bool {
        self.rule_schedule_eligible
    }

    pub(crate) fn blockers(&self) -> &[BlockReason] {
        &self.blockers
    }

    pub(crate) const fn created_at(&self) -> SystemTime {
        self.created_at
    }

    pub(crate) const fn status(&self) -> CandidateHistoryStatus {
        self.status
    }
}

pub(super) struct PreparedCandidate {
    id: String,
    source_scan_id: String,
    rule_id: String,
    rule_revision: i64,
    category: &'static str,
    paths: Vec<EncodedBytes>,
    estimated_bytes: i64,
    newest_mtime: Option<TimeParts>,
    evidence: Vec<PreparedEvidence>,
    safety: &'static str,
    action: &'static str,
    rule_schedule_eligible: bool,
    blockers: Vec<&'static str>,
    created_at_unix_ms: i64,
}

impl PreparedCandidate {
    pub(super) fn prepare(record: &NewCandidateRecord) -> Result<Self, HistoryError> {
        let candidate = &record.candidate;
        validate_candidate(candidate, HistoryErrorKind::InvalidInput)?;
        let paths = candidate
            .paths()
            .iter()
            .map(|path| {
                encode_host_path(path)
                    .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let evidence = candidate
            .evidence()
            .iter()
            .map(PreparedEvidence::prepare)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            id: candidate.id().as_str().to_owned(),
            source_scan_id: candidate.source_scan_id().as_str().to_owned(),
            rule_id: candidate.rule().id().as_str().to_owned(),
            rule_revision: i64::from(candidate.rule().revision().get()),
            category: category_as_stored(candidate.category()),
            paths,
            estimated_bytes: to_i64(candidate.estimated_bytes(), HistoryErrorKind::InvalidInput)?,
            newest_mtime: candidate
                .newest_mtime()
                .map(|value| time_parts(value, HistoryErrorKind::InvalidInput))
                .transpose()?,
            evidence,
            safety: safety_as_stored(candidate.safety()),
            action: action_as_stored(candidate.action()),
            rule_schedule_eligible: candidate.rule_marks_schedule_eligible(),
            blockers: candidate
                .blockers()
                .iter()
                .map(|reason| blocker_as_stored(reason))
                .collect(),
            created_at_unix_ms: system_time_to_unix_ms(
                record.created_at,
                HistoryErrorKind::InvalidInput,
            )?,
        })
    }

    fn batch_usage(&self) -> Result<CandidateBatchUsage, HistoryError> {
        let path_payload_bytes = self.paths.iter().try_fold(0_u64, |total, path| {
            total
                .checked_add(u64::try_from(path.bytes.len()).map_err(|_| invalid())?)
                .ok_or_else(invalid)
        })?;
        let evidence_payload_bytes = self.evidence.iter().try_fold(
            0_u64,
            |total, evidence| -> Result<u64, HistoryError> {
                let path_bytes = evidence.path.as_ref().map_or(Ok(0_u64), |path| {
                    u64::try_from(path.bytes.len()).map_err(|_| invalid())
                })?;
                let text_bytes = evidence.text.as_ref().map_or(Ok(0_u64), |text| {
                    u64::try_from(text.len()).map_err(|_| invalid())
                })?;
                total
                    .checked_add(path_bytes)
                    .and_then(|value| value.checked_add(text_bytes))
                    .ok_or_else(invalid)
            },
        )?;
        Ok(CandidateBatchUsage {
            candidates: 1,
            paths: u64::try_from(self.paths.len()).map_err(|_| invalid())?,
            path_payload_bytes,
            evidence: u64::try_from(self.evidence.len()).map_err(|_| invalid())?,
            evidence_payload_bytes,
            blockers: u64::try_from(self.blockers.len()).map_err(|_| invalid())?,
        })
    }
}

pub(super) fn ensure_prepared_candidate_batch_budget(
    candidates: &[PreparedCandidate],
) -> Result<(), HistoryError> {
    let usage =
        candidates
            .iter()
            .try_fold(CandidateBatchUsage::default(), |usage, candidate| {
                usage.checked_add(candidate.batch_usage()?, HistoryErrorKind::InvalidInput)
            })?;
    usage.ensure_within_budget(HistoryErrorKind::InvalidInput)
}

pub(super) fn candidate_record_batch_fits_materialization_budget(
    candidates: &[NewCandidateRecord],
) -> Result<bool, HistoryError> {
    let usage = candidates
        .iter()
        .try_fold(CandidateBatchUsage::default(), |usage, record| {
            let candidate = record.candidate();
            let path_payload_bytes = candidate.paths().iter().try_fold(
                0_u64,
                |total, path| -> Result<u64, HistoryError> {
                    let encoded = prepare_absolute_path(path)?;
                    total
                        .checked_add(u64::try_from(encoded.bytes.len()).map_err(|_| invalid())?)
                        .ok_or_else(invalid)
                },
            )?;
            let evidence_payload_bytes = candidate.evidence().iter().try_fold(
                0_u64,
                |total, evidence| -> Result<u64, HistoryError> {
                    let prepared = PreparedEvidence::prepare(evidence)?;
                    let path_bytes = prepared.path.as_ref().map_or(Ok(0_u64), |path| {
                        u64::try_from(path.bytes.len()).map_err(|_| invalid())
                    })?;
                    let text_bytes = prepared.text.as_ref().map_or(Ok(0_u64), |text| {
                        u64::try_from(text.len()).map_err(|_| invalid())
                    })?;
                    total
                        .checked_add(path_bytes)
                        .and_then(|value| value.checked_add(text_bytes))
                        .ok_or_else(invalid)
                },
            )?;
            usage.checked_add(
                CandidateBatchUsage {
                    candidates: 1,
                    paths: u64::try_from(candidate.paths().len()).map_err(|_| invalid())?,
                    path_payload_bytes,
                    evidence: u64::try_from(candidate.evidence().len()).map_err(|_| invalid())?,
                    evidence_payload_bytes,
                    blockers: u64::try_from(candidate.blockers().len()).map_err(|_| invalid())?,
                },
                HistoryErrorKind::InvalidInput,
            )
        })?;
    Ok(usage.materialized_bytes(HistoryErrorKind::InvalidInput)?
        <= MAX_CANDIDATE_BATCH_MATERIALIZED_BYTES)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct PreparedEvidence {
    pub(super) kind: &'static str,
    pub(super) path: Option<EncodedBytes>,
    pub(super) text: Option<String>,
    pub(super) observed_time: Option<TimeParts>,
    pub(super) duration: Option<TimeParts>,
    pub(super) observed_bytes: Option<i64>,
    pub(super) minimum_bytes: Option<i64>,
}

impl PreparedEvidence {
    pub(super) fn prepare(evidence: &Evidence) -> Result<Self, HistoryError> {
        let mut prepared = Self {
            kind: evidence_kind_as_stored(evidence),
            path: None,
            text: None,
            observed_time: None,
            duration: None,
            observed_bytes: None,
            minimum_bytes: None,
        };
        match evidence {
            Evidence::MatchedPath { path }
            | Evidence::RequiredMarker { path }
            | Evidence::ForbiddenMarkerAbsent { path }
            | Evidence::CloudUploadComplete { path } => {
                prepared.path = Some(prepare_absolute_path(path)?);
            }
            Evidence::BundleIdentifier { path, identifier } => {
                prepared.path = Some(prepare_absolute_path(path)?);
                validate_text(identifier, HistoryErrorKind::InvalidInput)?;
                prepared.text = Some(identifier.clone());
            }
            Evidence::MinimumAge {
                newest_mtime,
                minimum_age,
            } => {
                prepared.observed_time =
                    Some(time_parts(*newest_mtime, HistoryErrorKind::InvalidInput)?);
                prepared.duration = Some(duration_parts(
                    *minimum_age,
                    HistoryErrorKind::InvalidInput,
                )?);
            }
            Evidence::MinimumSize {
                observed_bytes,
                minimum_bytes,
            } => {
                prepared.observed_bytes =
                    Some(to_i64(*observed_bytes, HistoryErrorKind::InvalidInput)?);
                prepared.minimum_bytes =
                    Some(to_i64(*minimum_bytes, HistoryErrorKind::InvalidInput)?);
            }
            Evidence::InactiveProcess { identifier } => {
                validate_text(identifier, HistoryErrorKind::InvalidInput)?;
                prepared.text = Some(identifier.clone());
            }
        }
        Ok(prepared)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TimeParts {
    pub(super) seconds: i64,
    pub(super) nanoseconds: i64,
}

pub(super) fn insert_candidate(
    transaction: &Transaction<'_>,
    candidate: &PreparedCandidate,
) -> Result<(), HistoryError> {
    let scan_id = ScanId::new(candidate.source_scan_id.clone())
        .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    let Some(scan) = load_scan_record(transaction, &scan_id)? else {
        return Err(HistoryError::new(HistoryErrorKind::NotFound));
    };
    if scan.status() != ScanStatus::Succeeded {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }

    let changed = transaction
        .execute(
            "INSERT INTO candidates (
                 candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                 estimated_bytes, created_at_unix_ms, status, record_format_version,
                 category, proposed_action, rule_schedule_eligible,
                 newest_mtime_unix_seconds, newest_mtime_nanoseconds
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'discovered', 2,
                 ?8, ?9, ?10, ?11, ?12
             ) ON CONFLICT(candidate_id) DO NOTHING",
            params![
                candidate.id,
                candidate.source_scan_id,
                candidate.rule_id,
                candidate.rule_revision,
                candidate.safety,
                candidate.estimated_bytes,
                candidate.created_at_unix_ms,
                candidate.category,
                candidate.action,
                candidate.rule_schedule_eligible,
                candidate.newest_mtime.map(|value| value.seconds),
                candidate.newest_mtime.map(|value| value.nanoseconds),
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
    }

    for (ordinal, path) in candidate.paths.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO candidate_paths (
                     candidate_id, path_ordinal, observed_path, observed_path_encoding
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    candidate.id,
                    ordinal as i64,
                    path.bytes,
                    path.encoding as i64,
                ],
            )
            .map_err(map_write_sql_error)?;
    }
    for (ordinal, evidence) in candidate.evidence.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO candidate_evidence (
                     candidate_id, evidence_ordinal, evidence_kind,
                     path_value, path_value_encoding, text_value,
                     observed_unix_seconds, observed_nanoseconds,
                     duration_seconds, duration_nanoseconds,
                     observed_bytes, minimum_bytes
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    candidate.id,
                    ordinal as i64,
                    evidence.kind,
                    evidence.path.as_ref().map(|value| value.bytes.as_slice()),
                    evidence.path.as_ref().map(|value| value.encoding as i64),
                    evidence.text,
                    evidence.observed_time.map(|value| value.seconds),
                    evidence.observed_time.map(|value| value.nanoseconds),
                    evidence.duration.map(|value| value.seconds),
                    evidence.duration.map(|value| value.nanoseconds),
                    evidence.observed_bytes,
                    evidence.minimum_bytes,
                ],
            )
            .map_err(map_write_sql_error)?;
    }
    for (ordinal, blocker) in candidate.blockers.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO candidate_blockers (
                     candidate_id, blocker_ordinal, blocker_kind
                 ) VALUES (?1, ?2, ?3)",
                params![candidate.id, ordinal as i64, blocker],
            )
            .map_err(map_write_sql_error)?;
    }
    Ok(())
}

pub(super) fn complete_candidate_matches_new(
    stored: &CompleteCandidateRecord,
    expected: &NewCandidateRecord,
) -> bool {
    let candidate = expected.candidate();
    stored.id == *candidate.id()
        && stored.source_scan_id == *candidate.source_scan_id()
        && stored.rule == *candidate.rule()
        && stored.category == candidate.category()
        && stored.paths == candidate.paths()
        && stored.estimated_bytes == candidate.estimated_bytes()
        && stored.newest_mtime == candidate.newest_mtime()
        && stored.evidence == candidate.evidence()
        && stored.safety == candidate.safety()
        && stored.action == candidate.action()
        && stored.rule_schedule_eligible == candidate.rule_marks_schedule_eligible()
        && stored.blockers == candidate.blockers()
        && stored.created_at == expected.created_at()
}

pub(super) fn transition_candidate_review(
    transaction: &Transaction<'_>,
    id: &CandidateId,
    transition: CandidateReviewTransition,
) -> Result<(CandidateHistoryStatus, CandidateHistoryStatus), HistoryError> {
    let (previous, next) = transition.statuses();
    transition_candidate_status(transaction, id, previous, next, |candidate| {
        transition.validate_candidate(candidate)
    })
}

pub(super) fn transition_candidate_evaluation(
    transaction: &Transaction<'_>,
    id: &CandidateId,
    transition: CandidateEvaluationTransition,
) -> Result<(CandidateHistoryStatus, CandidateHistoryStatus), HistoryError> {
    let (previous, next) = transition.statuses();
    transition_candidate_status(transaction, id, previous, next, |_| Ok(()))
}

fn transition_candidate_status(
    transaction: &Transaction<'_>,
    id: &CandidateId,
    previous: CandidateHistoryStatus,
    next: CandidateHistoryStatus,
    validate: impl FnOnce(&CompleteCandidateRecord) -> Result<(), HistoryError>,
) -> Result<(CandidateHistoryStatus, CandidateHistoryStatus), HistoryError> {
    let Some(record) = load_candidate_record(transaction, id)? else {
        return Err(HistoryError::new(HistoryErrorKind::NotFound));
    };
    let StoredCandidateRecord::Complete(candidate) = record else {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    };
    validate(&candidate)?;
    if candidate.status == next {
        return Ok((previous, next));
    }
    if candidate.status != previous {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    let changed = transaction
        .execute(
            "UPDATE candidates
             SET status = ?1
             WHERE candidate_id = ?2 AND record_format_version = 2 AND status = ?3",
            params![next.as_stored(), id.as_str(), previous.as_stored()],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok((previous, next))
}

pub(super) fn mark_candidate_planned(
    transaction: &Transaction<'_>,
    candidate: &CompleteCandidateRecord,
) -> Result<CandidatePriorReviewStatus, HistoryError> {
    if !candidate.action.is_cleanup_operation() || !candidate.blockers.is_empty() {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    let prior = match candidate.status {
        CandidateHistoryStatus::Discovered => CandidatePriorReviewStatus::Discovered,
        CandidateHistoryStatus::Selected => CandidatePriorReviewStatus::Selected,
        _ => return Err(HistoryError::new(HistoryErrorKind::InvalidTransition)),
    };
    let changed = transaction
        .execute(
            "UPDATE candidates
             SET status = 'planned'
             WHERE candidate_id = ?1 AND record_format_version = 2 AND status = ?2",
            params![candidate.id.as_str(), prior.as_stored()],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok(prior)
}

pub(super) fn settle_planned_candidate(
    transaction: &Transaction<'_>,
    id: &CandidateId,
    settlement: CandidatePlanSettlement,
) -> Result<(), HistoryError> {
    let target = match settlement {
        CandidatePlanSettlement::Restore(prior) => prior.as_stored(),
        CandidatePlanSettlement::Completed => "completed",
        CandidatePlanSettlement::Failed => "failed",
    };
    let changed = transaction
        .execute(
            "UPDATE candidates
             SET status = ?2
             WHERE candidate_id = ?1 AND record_format_version = 2 AND status = 'planned'",
            params![id.as_str(), target],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok(())
}

pub(super) fn load_candidate_record(
    connection: &Connection,
    id: &CandidateId,
) -> Result<Option<StoredCandidateRecord>, HistoryError> {
    // Product-facing exact loads own a fresh query envelope and preflight the
    // decoded graph before copying payloads. Cleanup-history callers use the
    // `_within_budget` form only after their enclosing aggregate query has
    // already bounded the same data; repeating these scans there would reject
    // the maximum legal cleanup/journal contracts under the shared VM limit.
    run_bounded_query(connection, || {
        ensure_stored_candidate_budget(connection, id)?;
        load_candidate_record_within_budget(connection, id)
    })
}

pub(super) fn load_candidate_record_within_budget(
    connection: &Connection,
    id: &CandidateId,
) -> Result<Option<StoredCandidateRecord>, HistoryError> {
    load_candidate_record_within_budget_and_hook(connection, id, |_| Ok(()))
}

/// Load one evaluator-owned scan batch without issuing one parent/child query
/// per candidate. The caller owns the shared query progress budget, so every
/// statement is set-based and carries a SQL-side upper bound before rows are
/// materialized.
pub(super) fn load_complete_candidate_batch_within_budget(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
) -> Result<Vec<CompleteCandidateRecord>, HistoryError> {
    ensure_stored_candidate_batch_budget(connection, scan_id, maximum)?;
    let row_limit =
        i64::try_from(maximum.checked_add(1).ok_or_else(corrupt)?).map_err(|_| corrupt())?;
    let mut statement = connection
        .prepare(
            "SELECT record_format_version,
                    typeof(candidate_id), length(CAST(candidate_id AS BLOB)), candidate_id,
                    typeof(scan_id), length(CAST(scan_id AS BLOB)), scan_id,
                    typeof(rule_id), length(CAST(rule_id AS BLOB)), rule_id,
                    rule_revision,
                    typeof(safety_tier), length(CAST(safety_tier AS BLOB)), safety_tier,
                    estimated_bytes, created_at_unix_ms,
                    typeof(status), length(CAST(status AS BLOB)), status,
                    typeof(category), length(CAST(category AS BLOB)), category,
                    typeof(proposed_action), length(CAST(proposed_action AS BLOB)), proposed_action,
                    rule_schedule_eligible,
                    newest_mtime_unix_seconds, newest_mtime_nanoseconds
             FROM candidates INDEXED BY candidates_by_scan_time
             WHERE scan_id = ?1
             ORDER BY created_at_unix_ms DESC, candidate_id
             LIMIT ?2",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![scan_id.as_str(), row_limit])
        .map_err(map_query_sql_error)?;
    let mut batch = Vec::with_capacity(maximum.min(256));
    let mut indices = HashMap::with_capacity(maximum.min(256));
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if batch.len() >= maximum {
            return Err(corrupt());
        }
        let raw = raw_candidate_row(row).map_err(map_query_sql_error)?;
        if raw.record_format_version != 2 {
            return Err(corrupt());
        }
        let common = decode_common(&raw)?;
        if common.source_scan_id != *scan_id || indices.contains_key(&common.id) {
            return Err(corrupt());
        }
        let category = category_from_stored(raw.category.as_deref().ok_or_else(corrupt)?)?;
        let action = action_from_stored(raw.action.as_deref().ok_or_else(corrupt)?)?;
        let rule_schedule_eligible = stored_bool(raw.rule_schedule_eligible.ok_or_else(corrupt)?)?;
        validate_policy(common.safety, action, rule_schedule_eligible, corrupt)?;
        let newest_mtime =
            decode_optional_time(raw.newest_mtime_seconds, raw.newest_mtime_nanoseconds)?;
        let index = batch.len();
        indices.insert(common.id.clone(), index);
        batch.push(BatchCandidate {
            common,
            category,
            action,
            rule_schedule_eligible,
            newest_mtime,
            paths: Vec::new(),
            evidence: Vec::new(),
            blockers: Vec::new(),
        });
    }
    drop(rows);
    drop(statement);
    if batch.is_empty() {
        return Ok(Vec::new());
    }

    load_batch_paths(connection, scan_id, maximum, &indices, &mut batch)?;
    load_batch_evidence(connection, scan_id, maximum, &indices, &mut batch)?;
    load_batch_blockers(connection, scan_id, maximum, &indices, &mut batch)?;
    validate_batch_plan_claims(connection, scan_id, maximum, &indices, &batch)?;

    batch
        .into_iter()
        .map(|candidate| {
            if candidate.paths.is_empty() || candidate.evidence.is_empty() {
                return Err(corrupt());
            }
            validate_complete_children(
                candidate.common.safety,
                candidate.action,
                &candidate.paths,
                &candidate.evidence,
            )?;
            Ok(CompleteCandidateRecord {
                id: candidate.common.id,
                source_scan_id: candidate.common.source_scan_id,
                rule: candidate.common.rule,
                category: candidate.category,
                paths: candidate.paths,
                estimated_bytes: candidate.common.estimated_bytes,
                newest_mtime: candidate.newest_mtime,
                evidence: candidate.evidence,
                safety: candidate.common.safety,
                action: candidate.action,
                rule_schedule_eligible: candidate.rule_schedule_eligible,
                blockers: candidate.blockers,
                created_at: candidate.common.created_at,
                status: candidate.common.status,
            })
        })
        .collect()
}

fn ensure_stored_candidate_batch_budget(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
) -> Result<(), HistoryError> {
    let parent_limit = batch_parent_limit(maximum)?;
    let (candidates, invalid_candidates) = bounded_batch_validation(
        connection,
        "SELECT count(*), COALESCE(sum(invalid), 0) FROM (
             SELECT CASE WHEN
                        typeof(record_format_version) != 'integer' OR
                        record_format_version != 2 OR
                        typeof(candidate_id) != 'text' OR
                        length(CAST(candidate_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                        typeof(scan_id) != 'text' OR
                        length(CAST(scan_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                        typeof(rule_id) != 'text' OR
                        length(CAST(rule_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                        typeof(rule_revision) != 'integer' OR
                        typeof(safety_tier) != 'text' OR
                        length(CAST(safety_tier AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(estimated_bytes) != 'integer' OR
                        typeof(created_at_unix_ms) != 'integer' OR
                        typeof(status) != 'text' OR
                        length(CAST(status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(category) != 'text' OR
                        length(CAST(category AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(proposed_action) != 'text' OR
                        length(CAST(proposed_action AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(rule_schedule_eligible) != 'integer' OR
                        typeof(newest_mtime_unix_seconds) NOT IN ('null', 'integer') OR
                        typeof(newest_mtime_nanoseconds) NOT IN ('null', 'integer')
                    THEN 1 ELSE 0 END AS invalid
             FROM candidates INDEXED BY candidates_by_scan_time
             WHERE scan_id = ?1 LIMIT ?2
         )",
        scan_id,
        parent_limit,
    )?;
    if invalid_candidates != 0 || candidates > u64::try_from(maximum).map_err(|_| corrupt())? {
        return Err(corrupt());
    }

    let (paths, path_payload_bytes, invalid_paths) = bounded_batch_payload_usage(
        connection,
        "SELECT count(*), COALESCE(sum(payload_bytes), 0),
                COALESCE(sum(invalid), 0)
         FROM (
             SELECT CASE WHEN typeof(path.observed_path) = 'blob'
                         THEN length(path.observed_path) ELSE 0 END AS payload_bytes,
                    CASE WHEN typeof(path.path_ordinal) != 'integer' OR
                                   typeof(path.observed_path) != 'blob' OR
                                   length(path.observed_path) NOT BETWEEN 1 AND 65536
                                   OR typeof(path.observed_path_encoding) != 'integer'
                         THEN 1 ELSE 0 END AS invalid
             FROM candidates AS candidate INDEXED BY candidates_by_scan_time
             JOIN candidate_paths AS path USING (candidate_id)
             WHERE candidate.scan_id = ?1 LIMIT ?2
         )",
        scan_id,
        batch_child_limit(maximum, MAX_PATHS)?,
    )?;
    let (evidence, evidence_payload_bytes, invalid_evidence) = bounded_batch_payload_usage(
        connection,
        "SELECT count(*), COALESCE(sum(payload_bytes), 0),
                COALESCE(sum(invalid), 0)
         FROM (
             SELECT CASE WHEN typeof(evidence.path_value) = 'blob'
                         THEN length(evidence.path_value) ELSE 0 END +
                    CASE WHEN typeof(evidence.text_value) = 'text'
                         THEN length(CAST(evidence.text_value AS BLOB)) ELSE 0 END
                        AS payload_bytes,
                    CASE WHEN
                        typeof(evidence.evidence_kind) != 'text' OR
                        length(CAST(evidence.evidence_kind AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(evidence.evidence_ordinal) != 'integer' OR
                        NOT (
                            (typeof(evidence.path_value) = 'null' AND
                             length(evidence.path_value) IS NULL) OR
                            (typeof(evidence.path_value) = 'blob' AND
                             length(evidence.path_value) BETWEEN 1 AND 65536)
                        ) OR
                        NOT (
                            (typeof(evidence.text_value) = 'null' AND
                             length(CAST(evidence.text_value AS BLOB)) IS NULL) OR
                            (typeof(evidence.text_value) = 'text' AND
                             length(CAST(evidence.text_value AS BLOB)) BETWEEN 1 AND 4096)
                        ) OR
                        typeof(evidence.path_value_encoding) NOT IN ('null', 'integer') OR
                        typeof(evidence.observed_unix_seconds) NOT IN ('null', 'integer') OR
                        typeof(evidence.observed_nanoseconds) NOT IN ('null', 'integer') OR
                        typeof(evidence.duration_seconds) NOT IN ('null', 'integer') OR
                        typeof(evidence.duration_nanoseconds) NOT IN ('null', 'integer') OR
                        typeof(evidence.observed_bytes) NOT IN ('null', 'integer') OR
                        typeof(evidence.minimum_bytes) NOT IN ('null', 'integer')
                    THEN 1 ELSE 0 END AS invalid
             FROM candidates AS candidate INDEXED BY candidates_by_scan_time
             JOIN candidate_evidence AS evidence USING (candidate_id)
             WHERE candidate.scan_id = ?1 LIMIT ?2
         )",
        scan_id,
        batch_child_limit(maximum, MAX_EVIDENCE)?,
    )?;
    let (blockers, invalid_blockers) = bounded_batch_validation(
        connection,
        "SELECT count(*), COALESCE(sum(invalid), 0) FROM (
             SELECT CASE WHEN typeof(blocker.blocker_ordinal) != 'integer' OR
                                   typeof(blocker.blocker_kind) != 'text' OR
                                   length(CAST(blocker.blocker_kind AS BLOB)) NOT BETWEEN 1 AND 64
                         THEN 1 ELSE 0 END AS invalid
             FROM candidates AS candidate INDEXED BY candidates_by_scan_time
             JOIN candidate_blockers AS blocker USING (candidate_id)
             WHERE candidate.scan_id = ?1 LIMIT ?2
         )",
        scan_id,
        batch_child_limit(maximum, MAX_BLOCKERS)?,
    )?;
    let (_, invalid_claims) = bounded_batch_validation(
        connection,
        "SELECT count(*), COALESCE(sum(invalid), 0) FROM (
             SELECT CASE WHEN
                        typeof(claim.prior_review_status) != 'text' OR
                        length(CAST(claim.prior_review_status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(session.candidate_status_coupling_version) != 'integer' OR
                        typeof(session.status) != 'text' OR
                        length(CAST(session.status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(session.record_format_version) != 'integer' OR
                        typeof(item.candidate_id) != 'text' OR
                        length(CAST(item.candidate_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                        typeof(item.record_format_version) != 'integer'
                    THEN 1 ELSE 0 END AS invalid
             FROM candidates AS candidate INDEXED BY candidates_by_scan_time
             JOIN candidate_plan_claims AS claim USING (candidate_id)
             LEFT JOIN cleanup_sessions AS session
               ON session.session_id = claim.session_id
             LEFT JOIN cleanup_items AS item
               ON item.session_id = claim.session_id
              AND item.item_ordinal = claim.item_ordinal
             WHERE candidate.scan_id = ?1 LIMIT ?2
         )",
        scan_id,
        parent_limit,
    )?;
    if invalid_paths != 0
        || invalid_evidence != 0
        || invalid_blockers != 0
        || invalid_claims != 0
        || paths > u64::try_from(maximum.saturating_mul(MAX_PATHS)).map_err(|_| corrupt())?
        || evidence > u64::try_from(maximum.saturating_mul(MAX_EVIDENCE)).map_err(|_| corrupt())?
        || blockers > u64::try_from(maximum.saturating_mul(MAX_BLOCKERS)).map_err(|_| corrupt())?
    {
        return Err(corrupt());
    }
    CandidateBatchUsage {
        candidates,
        paths,
        path_payload_bytes,
        evidence,
        evidence_payload_bytes,
        blockers,
    }
    .ensure_within_budget(HistoryErrorKind::QueryLimitExceeded)
}

fn bounded_batch_validation(
    connection: &Connection,
    sql: &str,
    scan_id: &ScanId,
    limit: i64,
) -> Result<(u64, u64), HistoryError> {
    let (count, invalid) = connection
        .query_row(sql, params![scan_id.as_str(), limit], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(map_query_sql_error)?;
    Ok((
        u64::try_from(count).map_err(|_| corrupt())?,
        u64::try_from(invalid).map_err(|_| corrupt())?,
    ))
}

fn bounded_batch_payload_usage(
    connection: &Connection,
    sql: &str,
    scan_id: &ScanId,
    limit: i64,
) -> Result<(u64, u64, u64), HistoryError> {
    let (count, bytes, invalid) = connection
        .query_row(sql, params![scan_id.as_str(), limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(map_query_sql_error)?;
    Ok((
        u64::try_from(count).map_err(|_| corrupt())?,
        u64::try_from(bytes).map_err(|_| corrupt())?,
        u64::try_from(invalid).map_err(|_| corrupt())?,
    ))
}

/// Preflight one exact candidate before any payload-bearing child query. The
/// scan-wide loader has the same aggregate guard, but review/detail operations
/// also call this exact loader directly and must not be able to materialize a
/// schema-shaped graph above the shared decoded-memory budget.
fn ensure_stored_candidate_budget(
    connection: &Connection,
    id: &CandidateId,
) -> Result<(), HistoryError> {
    let (count, invalid, format): (i64, i64, i64) = connection
        .query_row(
            "SELECT count(*), COALESCE(sum(invalid), 0), COALESCE(max(format), 0)
             FROM (
                 SELECT CASE WHEN typeof(record_format_version) = 'integer'
                                  THEN record_format_version ELSE -1 END AS format,
                        CASE WHEN
                            typeof(record_format_version) != 'integer' OR
                            record_format_version NOT IN (1, 2) OR
                            typeof(candidate_id) != 'text' OR
                            length(CAST(candidate_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                            typeof(scan_id) != 'text' OR
                            length(CAST(scan_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                            typeof(rule_id) != 'text' OR
                            length(CAST(rule_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                            typeof(rule_revision) != 'integer' OR
                            typeof(safety_tier) != 'text' OR
                            length(CAST(safety_tier AS BLOB)) NOT BETWEEN 1 AND 64 OR
                            typeof(estimated_bytes) != 'integer' OR
                            typeof(created_at_unix_ms) != 'integer' OR
                            typeof(status) != 'text' OR
                            length(CAST(status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                            (record_format_version = 1 AND (
                                typeof(category) != 'null' OR
                                typeof(proposed_action) != 'null' OR
                                typeof(rule_schedule_eligible) != 'null' OR
                                typeof(newest_mtime_unix_seconds) != 'null' OR
                                typeof(newest_mtime_nanoseconds) != 'null'
                            )) OR
                            (record_format_version = 2 AND (
                                typeof(category) != 'text' OR
                                length(CAST(category AS BLOB)) NOT BETWEEN 1 AND 64 OR
                                typeof(proposed_action) != 'text' OR
                                length(CAST(proposed_action AS BLOB)) NOT BETWEEN 1 AND 64 OR
                                typeof(rule_schedule_eligible) != 'integer' OR
                                typeof(newest_mtime_unix_seconds) NOT IN ('null', 'integer') OR
                                typeof(newest_mtime_nanoseconds) NOT IN ('null', 'integer')
                            ))
                        THEN 1 ELSE 0 END AS invalid
                 FROM candidates WHERE candidate_id = ?1 LIMIT 2
             )",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(map_query_sql_error)?;
    if count == 0 {
        return Ok(());
    }
    if count != 1 || invalid != 0 || !matches!(format, 1 | 2) {
        return Err(corrupt());
    }
    if format == 1 {
        return Ok(());
    }

    let (paths, path_payload_bytes, invalid_paths) = exact_payload_usage(
        connection,
        "SELECT count(*), COALESCE(sum(payload_bytes), 0),
                COALESCE(sum(invalid), 0)
         FROM (
             SELECT CASE WHEN typeof(observed_path) = 'blob'
                         THEN length(observed_path) ELSE 0 END AS payload_bytes,
                    CASE WHEN typeof(path_ordinal) != 'integer' OR
                                   typeof(observed_path) != 'blob' OR
                                   length(observed_path) NOT BETWEEN 1 AND 65536 OR
                                   typeof(observed_path_encoding) != 'integer'
                         THEN 1 ELSE 0 END AS invalid
             FROM candidate_paths WHERE candidate_id = ?1 LIMIT ?2
         )",
        id,
        MAX_PATHS + 1,
    )?;
    let (evidence, evidence_payload_bytes, invalid_evidence) = exact_payload_usage(
        connection,
        "SELECT count(*), COALESCE(sum(payload_bytes), 0),
                COALESCE(sum(invalid), 0)
         FROM (
             SELECT CASE WHEN typeof(path_value) = 'blob'
                         THEN length(path_value) ELSE 0 END +
                    CASE WHEN typeof(text_value) = 'text'
                         THEN length(CAST(text_value AS BLOB)) ELSE 0 END
                        AS payload_bytes,
                    CASE WHEN
                        typeof(evidence_kind) != 'text' OR
                        length(CAST(evidence_kind AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(evidence_ordinal) != 'integer' OR
                        NOT (
                            (typeof(path_value) = 'null' AND length(path_value) IS NULL) OR
                            (typeof(path_value) = 'blob' AND
                             length(path_value) BETWEEN 1 AND 65536)
                        ) OR
                        NOT (
                            (typeof(text_value) = 'null' AND
                             length(CAST(text_value AS BLOB)) IS NULL) OR
                            (typeof(text_value) = 'text' AND
                             length(CAST(text_value AS BLOB)) BETWEEN 1 AND 4096)
                        ) OR
                        typeof(path_value_encoding) NOT IN ('null', 'integer') OR
                        typeof(observed_unix_seconds) NOT IN ('null', 'integer') OR
                        typeof(observed_nanoseconds) NOT IN ('null', 'integer') OR
                        typeof(duration_seconds) NOT IN ('null', 'integer') OR
                        typeof(duration_nanoseconds) NOT IN ('null', 'integer') OR
                        typeof(observed_bytes) NOT IN ('null', 'integer') OR
                        typeof(minimum_bytes) NOT IN ('null', 'integer')
                    THEN 1 ELSE 0 END AS invalid
             FROM candidate_evidence WHERE candidate_id = ?1 LIMIT ?2
         )",
        id,
        MAX_EVIDENCE + 1,
    )?;
    let (blockers, invalid_blockers) = exact_validation_usage(
        connection,
        "SELECT count(*), COALESCE(sum(invalid), 0) FROM (
             SELECT CASE WHEN typeof(blocker_ordinal) != 'integer' OR
                                   typeof(blocker_kind) != 'text' OR
                                   length(CAST(blocker_kind AS BLOB)) NOT BETWEEN 1 AND 64
                         THEN 1 ELSE 0 END AS invalid
             FROM candidate_blockers WHERE candidate_id = ?1 LIMIT ?2
         )",
        id,
        MAX_BLOCKERS + 1,
    )?;
    let (claims, invalid_claims) = exact_validation_usage(
        connection,
        "SELECT count(*), COALESCE(sum(invalid), 0) FROM (
             SELECT CASE WHEN
                        typeof(claim.prior_review_status) != 'text' OR
                        length(CAST(claim.prior_review_status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(session.candidate_status_coupling_version) != 'integer' OR
                        typeof(session.status) != 'text' OR
                        length(CAST(session.status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(session.record_format_version) != 'integer' OR
                        typeof(item.candidate_id) != 'text' OR
                        length(CAST(item.candidate_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                        typeof(item.record_format_version) != 'integer'
                    THEN 1 ELSE 0 END AS invalid
             FROM candidate_plan_claims AS claim
             LEFT JOIN cleanup_sessions AS session
               ON session.session_id = claim.session_id
             LEFT JOIN cleanup_items AS item
               ON item.session_id = claim.session_id
              AND item.item_ordinal = claim.item_ordinal
             WHERE claim.candidate_id = ?1 LIMIT ?2
         )",
        id,
        2,
    )?;
    if invalid_paths != 0
        || invalid_evidence != 0
        || invalid_blockers != 0
        || invalid_claims != 0
        || paths > MAX_PATHS as u64
        || evidence > MAX_EVIDENCE as u64
        || blockers > MAX_BLOCKERS as u64
        || claims > 1
    {
        return Err(corrupt());
    }
    CandidateBatchUsage {
        candidates: 1,
        paths,
        path_payload_bytes,
        evidence,
        evidence_payload_bytes,
        blockers,
    }
    .ensure_within_budget(HistoryErrorKind::QueryLimitExceeded)
}

fn exact_validation_usage(
    connection: &Connection,
    sql: &str,
    id: &CandidateId,
    limit: usize,
) -> Result<(u64, u64), HistoryError> {
    let limit = i64::try_from(limit).map_err(|_| corrupt())?;
    let (count, invalid) = connection
        .query_row(sql, params![id.as_str(), limit], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(map_query_sql_error)?;
    Ok((
        u64::try_from(count).map_err(|_| corrupt())?,
        u64::try_from(invalid).map_err(|_| corrupt())?,
    ))
}

fn exact_payload_usage(
    connection: &Connection,
    sql: &str,
    id: &CandidateId,
    limit: usize,
) -> Result<(u64, u64, u64), HistoryError> {
    let limit = i64::try_from(limit).map_err(|_| corrupt())?;
    let (count, bytes, invalid) = connection
        .query_row(sql, params![id.as_str(), limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(map_query_sql_error)?;
    Ok((
        u64::try_from(count).map_err(|_| corrupt())?,
        u64::try_from(bytes).map_err(|_| corrupt())?,
        u64::try_from(invalid).map_err(|_| corrupt())?,
    ))
}

fn load_candidate_record_within_budget_and_hook(
    connection: &Connection,
    id: &CandidateId,
    after_source_scan: impl FnOnce(&Connection) -> Result<(), HistoryError>,
) -> Result<Option<StoredCandidateRecord>, HistoryError> {
    let raw = connection
            .query_row(
                "SELECT record_format_version,
                        typeof(candidate_id), length(CAST(candidate_id AS BLOB)), candidate_id,
                        typeof(scan_id), length(CAST(scan_id AS BLOB)), scan_id,
                        typeof(rule_id), length(CAST(rule_id AS BLOB)), rule_id,
                        rule_revision,
                        typeof(safety_tier), length(CAST(safety_tier AS BLOB)), safety_tier,
                        estimated_bytes, created_at_unix_ms,
                        typeof(status), length(CAST(status AS BLOB)), status,
                        typeof(category), length(CAST(category AS BLOB)), category,
                        typeof(proposed_action), length(CAST(proposed_action AS BLOB)), proposed_action,
                        rule_schedule_eligible,
                        newest_mtime_unix_seconds, newest_mtime_nanoseconds
                 FROM candidates WHERE candidate_id = ?1",
                [id.as_str()],
                raw_candidate_row,
            )
            .optional()
            .map_err(map_query_sql_error)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let common = decode_common(&raw)?;
    match raw.record_format_version {
        1 => Ok(Some(StoredCandidateRecord::LegacySummary({
            ensure_source_scan_exists(connection, &common.source_scan_id)?;
            if raw.category.is_some()
                || raw.action.is_some()
                || raw.rule_schedule_eligible.is_some()
                || raw.newest_mtime_seconds.is_some()
                || raw.newest_mtime_nanoseconds.is_some()
            {
                return Err(corrupt());
            }
            ensure_no_candidate_children(connection, id)?;
            LegacyCandidateSummary {
                id: common.id,
                source_scan_id: common.source_scan_id,
                rule: common.rule,
                safety: common.safety,
                estimated_bytes: common.estimated_bytes,
                created_at: common.created_at,
                status: common.status,
            }
        }))),
        2 => {
            ensure_source_scan_succeeded(connection, &common.source_scan_id)?;
            after_source_scan(connection)?;
            let paths = load_paths(connection, id)?;
            let evidence = load_evidence(connection, id)?;
            let blockers = load_blockers(connection, id)?;
            let category = category_from_stored(raw.category.as_deref().ok_or_else(corrupt)?)?;
            let action = action_from_stored(raw.action.as_deref().ok_or_else(corrupt)?)?;
            let schedule = raw.rule_schedule_eligible.ok_or_else(corrupt)?;
            let rule_schedule_eligible = stored_bool(schedule)?;
            validate_policy(common.safety, action, rule_schedule_eligible, corrupt)?;
            let newest_mtime =
                decode_optional_time(raw.newest_mtime_seconds, raw.newest_mtime_nanoseconds)?;
            validate_complete_children(common.safety, action, &paths, &evidence)?;
            validate_candidate_plan_claim_state(connection, &common.id, common.status)?;
            Ok(Some(StoredCandidateRecord::Complete(
                CompleteCandidateRecord {
                    id: common.id,
                    source_scan_id: common.source_scan_id,
                    rule: common.rule,
                    category,
                    paths,
                    estimated_bytes: common.estimated_bytes,
                    newest_mtime,
                    evidence,
                    safety: common.safety,
                    action,
                    rule_schedule_eligible,
                    blockers,
                    created_at: common.created_at,
                    status: common.status,
                },
            )))
        }
        _ => Err(corrupt()),
    }
}

fn validate_candidate_plan_claim_state(
    connection: &Connection,
    id: &CandidateId,
    status: CandidateHistoryStatus,
) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT typeof(claim.prior_review_status),
                    length(CAST(claim.prior_review_status AS BLOB)),
                    claim.prior_review_status,
                    session.candidate_status_coupling_version,
                    typeof(session.status), length(CAST(session.status AS BLOB)),
                    session.status, session.record_format_version,
                    typeof(item.candidate_id), length(CAST(item.candidate_id AS BLOB)),
                    item.candidate_id, item.record_format_version
             FROM candidate_plan_claims AS claim
             LEFT JOIN cleanup_sessions AS session
               ON session.session_id = claim.session_id
             LEFT JOIN cleanup_items AS item
               ON item.session_id = claim.session_id
              AND item.item_ordinal = claim.item_ordinal
             WHERE claim.candidate_id = ?1
             LIMIT 2",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([id.as_str()])
        .map_err(map_query_sql_error)?;
    let first = rows.next().map_err(map_query_sql_error)?;
    if status != CandidateHistoryStatus::Planned {
        return if first.is_none() {
            Ok(())
        } else {
            Err(corrupt())
        };
    }
    let row = first.ok_or_else(corrupt)?;
    validate_required_value(row, 0, 1, "text", MAX_STORED_POLICY_BYTES)
        .map_err(map_query_sql_error)?;
    validate_required_value(row, 4, 5, "text", MAX_STORED_POLICY_BYTES)
        .map_err(map_query_sql_error)?;
    validate_required_value(row, 8, 9, "text", MAX_STORED_ID_BYTES).map_err(map_query_sql_error)?;
    let prior: String = row.get(2).map_err(map_query_sql_error)?;
    let coupling: Option<i64> = row.get(3).map_err(map_query_sql_error)?;
    let session_status: String = row.get(6).map_err(map_query_sql_error)?;
    let session_version: Option<i64> = row.get(7).map_err(map_query_sql_error)?;
    let item_candidate_id: String = row.get(10).map_err(map_query_sql_error)?;
    let item_version: Option<i64> = row.get(11).map_err(map_query_sql_error)?;
    if CandidatePriorReviewStatus::from_stored(&prior).is_err()
        || coupling != Some(2)
        || !matches!(
            session_status.as_str(),
            "planned" | "running" | "recovering"
        )
        || session_version != Some(2)
        || item_candidate_id != id.as_str()
        || item_version != Some(2)
        || rows.next().map_err(map_query_sql_error)?.is_some()
    {
        return Err(corrupt());
    }
    Ok(())
}

struct RawCandidateRow {
    record_format_version: i64,
    id: String,
    source_scan_id: String,
    rule_id: String,
    rule_revision: i64,
    safety: String,
    estimated_bytes: i64,
    created_at_unix_ms: i64,
    status: String,
    category: Option<String>,
    action: Option<String>,
    rule_schedule_eligible: Option<i64>,
    newest_mtime_seconds: Option<i64>,
    newest_mtime_nanoseconds: Option<i64>,
}

fn raw_candidate_row(row: &Row<'_>) -> rusqlite::Result<RawCandidateRow> {
    validate_required_value(row, 1, 2, "text", MAX_STORED_ID_BYTES)?;
    validate_required_value(row, 4, 5, "text", MAX_STORED_ID_BYTES)?;
    validate_required_value(row, 7, 8, "text", MAX_STORED_ID_BYTES)?;
    validate_required_value(row, 11, 12, "text", MAX_STORED_POLICY_BYTES)?;
    validate_required_value(row, 16, 17, "text", MAX_STORED_POLICY_BYTES)?;
    let record_format_version: i64 = row.get(0)?;
    match record_format_version {
        1 => {
            validate_null_value(row, 19, 20)?;
            validate_null_value(row, 22, 23)?;
        }
        2 => {
            validate_required_value(row, 19, 20, "text", MAX_STORED_POLICY_BYTES)?;
            validate_required_value(row, 22, 23, "text", MAX_STORED_POLICY_BYTES)?;
        }
        _ => return Err(rusqlite::Error::InvalidQuery),
    }
    Ok(RawCandidateRow {
        record_format_version,
        id: row.get(3)?,
        source_scan_id: row.get(6)?,
        rule_id: row.get(9)?,
        rule_revision: row.get(10)?,
        safety: row.get(13)?,
        estimated_bytes: row.get(14)?,
        created_at_unix_ms: row.get(15)?,
        status: row.get(18)?,
        category: row.get(21)?,
        action: row.get(24)?,
        rule_schedule_eligible: row.get(25)?,
        newest_mtime_seconds: row.get(26)?,
        newest_mtime_nanoseconds: row.get(27)?,
    })
}

struct DecodedCommon {
    id: CandidateId,
    source_scan_id: ScanId,
    rule: RuleRef,
    safety: SafetyTier,
    estimated_bytes: u64,
    created_at: SystemTime,
    status: CandidateHistoryStatus,
}

struct BatchCandidate {
    common: DecodedCommon,
    category: CandidateCategory,
    action: CandidateAction,
    rule_schedule_eligible: bool,
    newest_mtime: Option<SystemTime>,
    paths: Vec<PathBuf>,
    evidence: Vec<Evidence>,
    blockers: Vec<BlockReason>,
}

fn decode_common(raw: &RawCandidateRow) -> Result<DecodedCommon, HistoryError> {
    let id = CandidateId::new(raw.id.clone()).map_err(|_| corrupt())?;
    let source_scan_id = ScanId::new(raw.source_scan_id.clone()).map_err(|_| corrupt())?;
    let rule_id = RuleId::new(raw.rule_id.clone()).map_err(|_| corrupt())?;
    let revision = u32::try_from(raw.rule_revision).map_err(|_| corrupt())?;
    let rule_revision = RuleRevision::new(revision).map_err(|_| corrupt())?;
    Ok(DecodedCommon {
        id,
        source_scan_id,
        rule: RuleRef::new(rule_id, rule_revision),
        safety: safety_from_stored(&raw.safety)?,
        estimated_bytes: from_i64(raw.estimated_bytes)?,
        created_at: unix_ms_to_system_time(raw.created_at_unix_ms)?,
        status: CandidateHistoryStatus::from_stored(&raw.status)?,
    })
}

fn batch_child_limit(maximum: usize, per_candidate: usize) -> Result<i64, HistoryError> {
    i64::try_from(
        maximum
            .checked_mul(per_candidate)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(corrupt)?,
    )
    .map_err(|_| corrupt())
}

fn batch_parent_limit(maximum: usize) -> Result<i64, HistoryError> {
    i64::try_from(maximum.checked_add(1).ok_or_else(corrupt)?).map_err(|_| corrupt())
}

fn load_batch_paths(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
    indices: &HashMap<CandidateId, usize>,
    batch: &mut [BatchCandidate],
) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "WITH batch(candidate_id) AS MATERIALIZED (
                 SELECT candidate_id
                 FROM candidates INDEXED BY candidates_by_scan_time
                 WHERE scan_id = ?1
                 ORDER BY created_at_unix_ms DESC, candidate_id
                 LIMIT ?2
             ), bounded_path AS MATERIALIZED (
                 SELECT path.candidate_id, path.path_ordinal,
                        path.observed_path, path.observed_path_encoding
                 FROM batch
                 JOIN candidate_paths AS path USING (candidate_id)
                 LIMIT ?3
             )
             SELECT typeof(candidate_id),
                    length(CAST(candidate_id AS BLOB)), candidate_id,
                    path_ordinal, typeof(observed_path),
                    length(observed_path), observed_path,
                    observed_path_encoding
             FROM bounded_path
             ORDER BY candidate_id, path_ordinal",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![
            scan_id.as_str(),
            batch_parent_limit(maximum)?,
            batch_child_limit(maximum, MAX_PATHS)?,
        ])
        .map_err(map_query_sql_error)?;
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        validate_required_value(row, 0, 1, "text", MAX_STORED_ID_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 4, 5, "blob", MAX_STORED_PATH_BYTES)
            .map_err(map_query_sql_error)?;
        let id = CandidateId::new(row.get::<_, String>(2).map_err(map_query_sql_error)?)
            .map_err(|_| corrupt())?;
        let index = *indices.get(&id).ok_or_else(corrupt)?;
        let paths = &mut batch[index].paths;
        if paths.len() >= MAX_PATHS
            || row.get::<_, i64>(3).map_err(map_query_sql_error)? != paths.len() as i64
        {
            return Err(corrupt());
        }
        let path = decode_absolute_path(
            row.get(6).map_err(map_query_sql_error)?,
            row.get(7).map_err(map_query_sql_error)?,
        )?;
        if paths.contains(&path) {
            return Err(corrupt());
        }
        paths.push(path);
    }
    Ok(())
}

fn load_batch_evidence(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
    indices: &HashMap<CandidateId, usize>,
    batch: &mut [BatchCandidate],
) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "WITH batch(candidate_id) AS MATERIALIZED (
                 SELECT candidate_id
                 FROM candidates INDEXED BY candidates_by_scan_time
                 WHERE scan_id = ?1
                 ORDER BY created_at_unix_ms DESC, candidate_id
                 LIMIT ?2
             ), bounded_evidence AS MATERIALIZED (
                 SELECT evidence.candidate_id, evidence.evidence_ordinal,
                        evidence.evidence_kind, evidence.path_value,
                        evidence.path_value_encoding, evidence.text_value,
                        evidence.observed_unix_seconds, evidence.observed_nanoseconds,
                        evidence.duration_seconds, evidence.duration_nanoseconds,
                        evidence.observed_bytes, evidence.minimum_bytes
                 FROM batch
                 JOIN candidate_evidence AS evidence USING (candidate_id)
                 LIMIT ?3
             )
             SELECT typeof(candidate_id),
                    length(CAST(candidate_id AS BLOB)), candidate_id,
                    evidence_ordinal, typeof(evidence_kind),
                    length(CAST(evidence_kind AS BLOB)), evidence_kind,
                    typeof(path_value), length(path_value), path_value,
                    path_value_encoding, typeof(text_value),
                    length(CAST(text_value AS BLOB)), text_value,
                    observed_unix_seconds, observed_nanoseconds,
                    duration_seconds, duration_nanoseconds,
                    observed_bytes, minimum_bytes
             FROM bounded_evidence
             ORDER BY candidate_id, evidence_ordinal",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![
            scan_id.as_str(),
            batch_parent_limit(maximum)?,
            batch_child_limit(maximum, MAX_EVIDENCE)?,
        ])
        .map_err(map_query_sql_error)?;
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        validate_required_value(row, 0, 1, "text", MAX_STORED_ID_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 4, 5, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 7, 8, "blob", MAX_STORED_PATH_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 11, 12, "text", MAX_STORED_TEXT_BYTES)
            .map_err(map_query_sql_error)?;
        let id = CandidateId::new(row.get::<_, String>(2).map_err(map_query_sql_error)?)
            .map_err(|_| corrupt())?;
        let index = *indices.get(&id).ok_or_else(corrupt)?;
        let evidence = &mut batch[index].evidence;
        if evidence.len() >= MAX_EVIDENCE
            || row.get::<_, i64>(3).map_err(map_query_sql_error)? != evidence.len() as i64
        {
            return Err(corrupt());
        }
        evidence.push(decode_evidence(RawEvidence {
            kind: row.get(6).map_err(map_query_sql_error)?,
            path: row.get(9).map_err(map_query_sql_error)?,
            path_encoding: row.get(10).map_err(map_query_sql_error)?,
            text: row.get(13).map_err(map_query_sql_error)?,
            observed_seconds: row.get(14).map_err(map_query_sql_error)?,
            observed_nanoseconds: row.get(15).map_err(map_query_sql_error)?,
            duration_seconds: row.get(16).map_err(map_query_sql_error)?,
            duration_nanoseconds: row.get(17).map_err(map_query_sql_error)?,
            observed_bytes: row.get(18).map_err(map_query_sql_error)?,
            minimum_bytes: row.get(19).map_err(map_query_sql_error)?,
        })?);
    }
    Ok(())
}

fn load_batch_blockers(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
    indices: &HashMap<CandidateId, usize>,
    batch: &mut [BatchCandidate],
) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "WITH batch(candidate_id) AS MATERIALIZED (
                 SELECT candidate_id
                 FROM candidates INDEXED BY candidates_by_scan_time
                 WHERE scan_id = ?1
                 ORDER BY created_at_unix_ms DESC, candidate_id
                 LIMIT ?2
             ), bounded_blocker AS MATERIALIZED (
                 SELECT blocker.candidate_id, blocker.blocker_ordinal,
                        blocker.blocker_kind
                 FROM batch
                 JOIN candidate_blockers AS blocker USING (candidate_id)
                 LIMIT ?3
             )
             SELECT typeof(candidate_id),
                    length(CAST(candidate_id AS BLOB)), candidate_id,
                    blocker_ordinal, typeof(blocker_kind),
                    length(CAST(blocker_kind AS BLOB)), blocker_kind
             FROM bounded_blocker
             ORDER BY candidate_id, blocker_ordinal",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![
            scan_id.as_str(),
            batch_parent_limit(maximum)?,
            batch_child_limit(maximum, MAX_BLOCKERS)?,
        ])
        .map_err(map_query_sql_error)?;
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        validate_required_value(row, 0, 1, "text", MAX_STORED_ID_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 4, 5, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        let id = CandidateId::new(row.get::<_, String>(2).map_err(map_query_sql_error)?)
            .map_err(|_| corrupt())?;
        let index = *indices.get(&id).ok_or_else(corrupt)?;
        let blockers = &mut batch[index].blockers;
        if blockers.len() >= MAX_BLOCKERS
            || row.get::<_, i64>(3).map_err(map_query_sql_error)? != blockers.len() as i64
        {
            return Err(corrupt());
        }
        blockers.push(blocker_from_stored(
            &row.get::<_, String>(6).map_err(map_query_sql_error)?,
        )?);
    }
    Ok(())
}

fn validate_batch_plan_claims(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
    indices: &HashMap<CandidateId, usize>,
    batch: &[BatchCandidate],
) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "WITH batch(candidate_id) AS MATERIALIZED (
                 SELECT candidate_id
                 FROM candidates INDEXED BY candidates_by_scan_time
                 WHERE scan_id = ?1
                 ORDER BY created_at_unix_ms DESC, candidate_id
                 LIMIT ?2
             ), bounded_claim AS MATERIALIZED (
                 SELECT claim.candidate_id, claim.prior_review_status,
                        session.candidate_status_coupling_version,
                        session.status AS session_status,
                        session.record_format_version AS session_record_format_version,
                        item.candidate_id AS item_candidate_id,
                        item.record_format_version AS item_record_format_version
                 FROM batch
                 JOIN candidate_plan_claims AS claim USING (candidate_id)
                 LEFT JOIN cleanup_sessions AS session
                   ON session.session_id = claim.session_id
                 LEFT JOIN cleanup_items AS item
                   ON item.session_id = claim.session_id
                  AND item.item_ordinal = claim.item_ordinal
                 LIMIT ?3
             )
             SELECT typeof(candidate_id),
                    length(CAST(candidate_id AS BLOB)), candidate_id,
                    typeof(prior_review_status),
                    length(CAST(prior_review_status AS BLOB)), prior_review_status,
                    candidate_status_coupling_version,
                    typeof(session_status), length(CAST(session_status AS BLOB)),
                    session_status, session_record_format_version,
                    typeof(item_candidate_id), length(CAST(item_candidate_id AS BLOB)),
                    item_candidate_id, item_record_format_version
             FROM bounded_claim
             ORDER BY candidate_id",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![
            scan_id.as_str(),
            batch_parent_limit(maximum)?,
            batch_parent_limit(maximum)?,
        ])
        .map_err(map_query_sql_error)?;
    let mut claimed = HashSet::with_capacity(batch.len().min(256));
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        validate_required_value(row, 0, 1, "text", MAX_STORED_ID_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 3, 4, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 7, 8, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 11, 12, "text", MAX_STORED_ID_BYTES)
            .map_err(map_query_sql_error)?;
        let id = CandidateId::new(row.get::<_, String>(2).map_err(map_query_sql_error)?)
            .map_err(|_| corrupt())?;
        let index = *indices.get(&id).ok_or_else(corrupt)?;
        let prior: String = row.get(5).map_err(map_query_sql_error)?;
        let coupling: Option<i64> = row.get(6).map_err(map_query_sql_error)?;
        let session_status: String = row.get(9).map_err(map_query_sql_error)?;
        let session_version: Option<i64> = row.get(10).map_err(map_query_sql_error)?;
        let item_candidate_id: String = row.get(13).map_err(map_query_sql_error)?;
        let item_version: Option<i64> = row.get(14).map_err(map_query_sql_error)?;
        if batch[index].common.status != CandidateHistoryStatus::Planned
            || !claimed.insert(id.clone())
            || CandidatePriorReviewStatus::from_stored(&prior).is_err()
            || coupling != Some(2)
            || !matches!(
                session_status.as_str(),
                "planned" | "running" | "recovering"
            )
            || session_version != Some(2)
            || item_candidate_id != id.as_str()
            || item_version != Some(2)
        {
            return Err(corrupt());
        }
    }
    if batch.iter().any(|candidate| {
        (candidate.common.status == CandidateHistoryStatus::Planned)
            != claimed.contains(&candidate.common.id)
    }) {
        return Err(corrupt());
    }
    Ok(())
}

fn load_paths(connection: &Connection, id: &CandidateId) -> Result<Vec<PathBuf>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT path_ordinal, typeof(observed_path), length(observed_path),
                    observed_path, observed_path_encoding
             FROM candidate_paths WHERE candidate_id = ?1
             ORDER BY path_ordinal LIMIT 257",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut paths = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if paths.len() >= MAX_PATHS {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != paths.len() as i64 {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "blob", MAX_STORED_PATH_BYTES)
            .map_err(map_query_sql_error)?;
        let bytes: Vec<u8> = row.get(3).map_err(map_query_sql_error)?;
        let encoding: i64 = row.get(4).map_err(map_query_sql_error)?;
        let path = decode_absolute_path(bytes, encoding)?;
        if paths.contains(&path) {
            return Err(corrupt());
        }
        paths.push(path);
    }
    if paths.is_empty() {
        return Err(corrupt());
    }
    Ok(paths)
}

fn load_evidence(connection: &Connection, id: &CandidateId) -> Result<Vec<Evidence>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT evidence_ordinal,
                    typeof(evidence_kind), length(CAST(evidence_kind AS BLOB)), evidence_kind,
                    typeof(path_value), length(path_value), path_value, path_value_encoding,
                    typeof(text_value), length(CAST(text_value AS BLOB)), text_value,
                    observed_unix_seconds, observed_nanoseconds,
                    duration_seconds, duration_nanoseconds, observed_bytes, minimum_bytes
             FROM candidate_evidence WHERE candidate_id = ?1
             ORDER BY evidence_ordinal LIMIT 513",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut evidence = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if evidence.len() >= MAX_EVIDENCE {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != evidence.len() as i64 {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 4, 5, "blob", MAX_STORED_PATH_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 8, 9, "text", MAX_STORED_TEXT_BYTES)
            .map_err(map_query_sql_error)?;
        let raw = RawEvidence {
            kind: row.get(3).map_err(map_query_sql_error)?,
            path: row.get(6).map_err(map_query_sql_error)?,
            path_encoding: row.get(7).map_err(map_query_sql_error)?,
            text: row.get(10).map_err(map_query_sql_error)?,
            observed_seconds: row.get(11).map_err(map_query_sql_error)?,
            observed_nanoseconds: row.get(12).map_err(map_query_sql_error)?,
            duration_seconds: row.get(13).map_err(map_query_sql_error)?,
            duration_nanoseconds: row.get(14).map_err(map_query_sql_error)?,
            observed_bytes: row.get(15).map_err(map_query_sql_error)?,
            minimum_bytes: row.get(16).map_err(map_query_sql_error)?,
        };
        evidence.push(decode_evidence(raw)?);
    }
    if evidence.is_empty() {
        return Err(corrupt());
    }
    Ok(evidence)
}

pub(super) struct RawEvidence {
    pub(super) kind: String,
    pub(super) path: Option<Vec<u8>>,
    pub(super) path_encoding: Option<i64>,
    pub(super) text: Option<String>,
    pub(super) observed_seconds: Option<i64>,
    pub(super) observed_nanoseconds: Option<i64>,
    pub(super) duration_seconds: Option<i64>,
    pub(super) duration_nanoseconds: Option<i64>,
    pub(super) observed_bytes: Option<i64>,
    pub(super) minimum_bytes: Option<i64>,
}

pub(super) fn decode_evidence(raw: RawEvidence) -> Result<Evidence, HistoryError> {
    let no_path = raw.path.is_none() && raw.path_encoding.is_none();
    let no_text = raw.text.is_none();
    let no_observed_time = raw.observed_seconds.is_none() && raw.observed_nanoseconds.is_none();
    let no_duration = raw.duration_seconds.is_none() && raw.duration_nanoseconds.is_none();
    let no_bytes = raw.observed_bytes.is_none() && raw.minimum_bytes.is_none();
    match raw.kind.as_str() {
        "matched_path" if no_text && no_observed_time && no_duration && no_bytes => {
            Ok(Evidence::MatchedPath {
                path: decode_required_evidence_path(raw.path, raw.path_encoding)?,
            })
        }
        "required_marker" if no_text && no_observed_time && no_duration && no_bytes => {
            Ok(Evidence::RequiredMarker {
                path: decode_required_evidence_path(raw.path, raw.path_encoding)?,
            })
        }
        "forbidden_marker_absent" if no_text && no_observed_time && no_duration && no_bytes => {
            Ok(Evidence::ForbiddenMarkerAbsent {
                path: decode_required_evidence_path(raw.path, raw.path_encoding)?,
            })
        }
        "cloud_upload_complete" if no_text && no_observed_time && no_duration && no_bytes => {
            Ok(Evidence::CloudUploadComplete {
                path: decode_required_evidence_path(raw.path, raw.path_encoding)?,
            })
        }
        "bundle_identifier" if no_observed_time && no_duration && no_bytes => {
            let identifier = raw.text.ok_or_else(corrupt)?;
            validate_text(&identifier, HistoryErrorKind::CorruptData)?;
            Ok(Evidence::BundleIdentifier {
                path: decode_required_evidence_path(raw.path, raw.path_encoding)?,
                identifier,
            })
        }
        "minimum_age" if no_path && no_text && no_bytes => Ok(Evidence::MinimumAge {
            newest_mtime: decode_required_time(raw.observed_seconds, raw.observed_nanoseconds)?,
            minimum_age: decode_required_duration(raw.duration_seconds, raw.duration_nanoseconds)?,
        }),
        "minimum_size" if no_path && no_text && no_observed_time && no_duration => {
            Ok(Evidence::MinimumSize {
                observed_bytes: from_i64(raw.observed_bytes.ok_or_else(corrupt)?)?,
                minimum_bytes: from_i64(raw.minimum_bytes.ok_or_else(corrupt)?)?,
            })
        }
        "inactive_process" if no_path && no_observed_time && no_duration && no_bytes => {
            let identifier = raw.text.ok_or_else(corrupt)?;
            validate_text(&identifier, HistoryErrorKind::CorruptData)?;
            Ok(Evidence::InactiveProcess { identifier })
        }
        _ => Err(corrupt()),
    }
}

fn load_blockers(
    connection: &Connection,
    id: &CandidateId,
) -> Result<Vec<BlockReason>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT blocker_ordinal, typeof(blocker_kind),
                    length(CAST(blocker_kind AS BLOB)), blocker_kind
             FROM candidate_blockers WHERE candidate_id = ?1
             ORDER BY blocker_ordinal LIMIT 65",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut blockers = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if blockers.len() >= MAX_BLOCKERS {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != blockers.len() as i64 {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        let value: String = row.get(3).map_err(map_query_sql_error)?;
        blockers.push(blocker_from_stored(&value)?);
    }
    Ok(blockers)
}

fn ensure_no_candidate_children(
    connection: &Connection,
    id: &CandidateId,
) -> Result<(), HistoryError> {
    let child_exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM candidate_paths WHERE candidate_id = ?1
             UNION ALL
             SELECT 1 FROM candidate_evidence WHERE candidate_id = ?1
             UNION ALL
             SELECT 1 FROM candidate_blockers WHERE candidate_id = ?1
             LIMIT 1",
            [id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    if child_exists.is_some() {
        return Err(corrupt());
    }
    Ok(())
}

fn ensure_source_scan_exists(connection: &Connection, id: &ScanId) -> Result<(), HistoryError> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM scans WHERE scan_id = ?1",
            [id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    if exists.is_none() {
        return Err(corrupt());
    }
    Ok(())
}

fn ensure_source_scan_succeeded(connection: &Connection, id: &ScanId) -> Result<(), HistoryError> {
    let Some(scan) = load_scan_record_within_budget(connection, id)? else {
        return Err(corrupt());
    };
    if scan.status() != ScanStatus::Succeeded {
        return Err(corrupt());
    }
    Ok(())
}

fn validate_candidate(candidate: &Candidate, kind: HistoryErrorKind) -> Result<(), HistoryError> {
    if candidate.paths().is_empty()
        || candidate.paths().len() > MAX_PATHS
        || candidate.evidence().is_empty()
        || candidate.evidence().len() > MAX_EVIDENCE
        || candidate.blockers().len() > MAX_BLOCKERS
    {
        return Err(HistoryError::new(kind));
    }
    let mut paths = HashSet::with_capacity(candidate.paths().len());
    for path in candidate.paths() {
        if !path.is_absolute() || !paths.insert(path) || encode_host_path(path).is_err() {
            return Err(HistoryError::new(kind));
        }
    }
    to_i64(candidate.estimated_bytes(), kind)?;
    if let Some(value) = candidate.newest_mtime() {
        time_parts(value, kind)?;
    }
    for evidence in candidate.evidence() {
        PreparedEvidence::prepare(evidence)?;
    }
    validate_policy(
        candidate.safety(),
        candidate.action(),
        candidate.rule_marks_schedule_eligible(),
        || HistoryError::new(kind),
    )?;
    validate_complete_children(
        candidate.safety(),
        candidate.action(),
        candidate.paths(),
        candidate.evidence(),
    )
    .map_err(|_| HistoryError::new(kind))
}

pub(super) fn validate_complete_children(
    safety: SafetyTier,
    action: CandidateAction,
    paths: &[PathBuf],
    evidence: &[Evidence],
) -> Result<(), HistoryError> {
    if paths.is_empty() || evidence.is_empty() {
        return Err(corrupt());
    }
    if safety == SafetyTier::SafeEvictable || action == CandidateAction::EvictLocalCopy {
        let confirmed = evidence
            .iter()
            .filter_map(|fact| match fact {
                Evidence::CloudUploadComplete { path } => Some(path),
                _ => None,
            })
            .collect::<Vec<_>>();
        let unique = confirmed.iter().copied().collect::<HashSet<_>>();
        let expected = paths.iter().collect::<HashSet<_>>();
        if confirmed.len() != paths.len() || unique != expected {
            return Err(corrupt());
        }
    }
    Ok(())
}

pub(super) fn validate_policy(
    safety: SafetyTier,
    action: CandidateAction,
    schedule_eligible: bool,
    error: impl Fn() -> HistoryError,
) -> Result<(), HistoryError> {
    if !action.is_compatible_with(safety)
        || (schedule_eligible && !safety.is_schedule_policy_pair(action))
    {
        return Err(error());
    }
    Ok(())
}

pub(super) fn validate_required_value(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
    expected_type: &str,
    maximum_length: i64,
) -> rusqlite::Result<()> {
    let storage_type: String = row.get(type_column)?;
    let length: i64 = row.get(length_column)?;
    if storage_type != expected_type || !(1..=maximum_length).contains(&length) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

pub(super) fn validate_optional_value(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
    expected_type: &str,
    maximum_length: i64,
) -> rusqlite::Result<()> {
    let storage_type: String = row.get(type_column)?;
    let length: Option<i64> = row.get(length_column)?;
    if storage_type == "null" && length.is_none() {
        return Ok(());
    }
    if storage_type != expected_type
        || !length.is_some_and(|value| (1..=maximum_length).contains(&value))
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

pub(super) fn validate_null_value(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
) -> rusqlite::Result<()> {
    let storage_type: String = row.get(type_column)?;
    let length: Option<i64> = row.get(length_column)?;
    if storage_type != "null" || length.is_some() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn prepare_absolute_path(path: &Path) -> Result<EncodedBytes, HistoryError> {
    if !path.is_absolute() {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    encode_host_path(path).map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))
}

fn decode_required_evidence_path(
    bytes: Option<Vec<u8>>,
    encoding: Option<i64>,
) -> Result<PathBuf, HistoryError> {
    decode_absolute_path(bytes.ok_or_else(corrupt)?, encoding.ok_or_else(corrupt)?)
}

pub(super) fn decode_absolute_path(bytes: Vec<u8>, encoding: i64) -> Result<PathBuf, HistoryError> {
    let encoding = match encoding {
        1 => StoredEncoding::Utf8HostPath,
        2 => StoredEncoding::Utf16LeHostPath,
        _ => return Err(corrupt()),
    };
    let path = decode_host_path(&EncodedBytes { bytes, encoding }).map_err(|_| corrupt())?;
    if !path.is_absolute() {
        return Err(corrupt());
    }
    Ok(path)
}

pub(super) fn validate_text(value: &str, kind: HistoryErrorKind) -> Result<(), HistoryError> {
    if value.is_empty() || value.len() > MAX_TEXT_BYTES || value.chars().any(char::is_control) {
        return Err(HistoryError::new(kind));
    }
    Ok(())
}

pub(super) fn time_parts(
    value: SystemTime,
    kind: HistoryErrorKind,
) -> Result<TimeParts, HistoryError> {
    let duration = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HistoryError::new(kind))?;
    duration_parts(duration, kind)
}

fn duration_parts(value: Duration, kind: HistoryErrorKind) -> Result<TimeParts, HistoryError> {
    Ok(TimeParts {
        seconds: i64::try_from(value.as_secs()).map_err(|_| HistoryError::new(kind))?,
        nanoseconds: i64::from(value.subsec_nanos()),
    })
}

fn decode_required_time(
    seconds: Option<i64>,
    nanoseconds: Option<i64>,
) -> Result<SystemTime, HistoryError> {
    decode_time(
        seconds.ok_or_else(corrupt)?,
        nanoseconds.ok_or_else(corrupt)?,
    )
}

pub(super) fn decode_optional_time(
    seconds: Option<i64>,
    nanoseconds: Option<i64>,
) -> Result<Option<SystemTime>, HistoryError> {
    match (seconds, nanoseconds) {
        (None, None) => Ok(None),
        (Some(seconds), Some(nanoseconds)) => decode_time(seconds, nanoseconds).map(Some),
        _ => Err(corrupt()),
    }
}

fn decode_time(seconds: i64, nanoseconds: i64) -> Result<SystemTime, HistoryError> {
    let seconds = u64::try_from(seconds).map_err(|_| corrupt())?;
    let nanoseconds = u32::try_from(nanoseconds).map_err(|_| corrupt())?;
    if nanoseconds >= 1_000_000_000 {
        return Err(corrupt());
    }
    UNIX_EPOCH
        .checked_add(Duration::new(seconds, nanoseconds))
        .ok_or_else(corrupt)
}

fn decode_required_duration(
    seconds: Option<i64>,
    nanoseconds: Option<i64>,
) -> Result<Duration, HistoryError> {
    let seconds = u64::try_from(seconds.ok_or_else(corrupt)?).map_err(|_| corrupt())?;
    let nanoseconds = u32::try_from(nanoseconds.ok_or_else(corrupt)?).map_err(|_| corrupt())?;
    if nanoseconds >= 1_000_000_000 {
        return Err(corrupt());
    }
    Ok(Duration::new(seconds, nanoseconds))
}

fn system_time_to_unix_ms(value: SystemTime, kind: HistoryErrorKind) -> Result<i64, HistoryError> {
    let milliseconds = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HistoryError::new(kind))?
        .as_millis();
    i64::try_from(milliseconds).map_err(|_| HistoryError::new(kind))
}

fn unix_ms_to_system_time(value: i64) -> Result<SystemTime, HistoryError> {
    let milliseconds = u64::try_from(value).map_err(|_| corrupt())?;
    UNIX_EPOCH
        .checked_add(Duration::from_millis(milliseconds))
        .ok_or_else(corrupt)
}

pub(super) fn to_i64(value: u64, kind: HistoryErrorKind) -> Result<i64, HistoryError> {
    i64::try_from(value).map_err(|_| HistoryError::new(kind))
}

pub(super) fn from_i64(value: i64) -> Result<u64, HistoryError> {
    u64::try_from(value).map_err(|_| corrupt())
}

pub(super) fn stored_bool(value: i64) -> Result<bool, HistoryError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(corrupt()),
    }
}

pub(super) fn category_as_stored(value: CandidateCategory) -> &'static str {
    match value {
        CandidateCategory::DeveloperArtifact => "developer_artifact",
        CandidateCategory::ApplicationCache => "application_cache",
        CandidateCategory::BrowserCache => "browser_cache",
        CandidateCategory::LogAndDiagnostic => "log_and_diagnostic",
        CandidateCategory::InstallerAndDownload => "installer_and_download",
        CandidateCategory::DeviceAndSimulatorData => "device_and_simulator_data",
        CandidateCategory::CloudFile => "cloud_file",
        CandidateCategory::LargeReviewItem => "large_review_item",
        CandidateCategory::ProtectedSystemData => "protected_system_data",
        CandidateCategory::UnknownStorage => "unknown_storage",
    }
}

pub(super) fn category_from_stored(value: &str) -> Result<CandidateCategory, HistoryError> {
    match value {
        "developer_artifact" => Ok(CandidateCategory::DeveloperArtifact),
        "application_cache" => Ok(CandidateCategory::ApplicationCache),
        "browser_cache" => Ok(CandidateCategory::BrowserCache),
        "log_and_diagnostic" => Ok(CandidateCategory::LogAndDiagnostic),
        "installer_and_download" => Ok(CandidateCategory::InstallerAndDownload),
        "device_and_simulator_data" => Ok(CandidateCategory::DeviceAndSimulatorData),
        "cloud_file" => Ok(CandidateCategory::CloudFile),
        "large_review_item" => Ok(CandidateCategory::LargeReviewItem),
        "protected_system_data" => Ok(CandidateCategory::ProtectedSystemData),
        "unknown_storage" => Ok(CandidateCategory::UnknownStorage),
        _ => Err(corrupt()),
    }
}

pub(super) fn safety_as_stored(value: SafetyTier) -> &'static str {
    match value {
        SafetyTier::SafeRegenerable => "safe_regenerable",
        SafetyTier::SafeEvictable => "safe_evictable",
        SafetyTier::ReviewRequired => "review_required",
        SafetyTier::Informational => "informational",
        SafetyTier::Protected => "protected",
    }
}

pub(super) fn safety_from_stored(value: &str) -> Result<SafetyTier, HistoryError> {
    match value {
        "safe_regenerable" => Ok(SafetyTier::SafeRegenerable),
        "safe_evictable" => Ok(SafetyTier::SafeEvictable),
        "review_required" => Ok(SafetyTier::ReviewRequired),
        "informational" => Ok(SafetyTier::Informational),
        "protected" => Ok(SafetyTier::Protected),
        _ => Err(corrupt()),
    }
}

pub(super) fn action_as_stored(value: CandidateAction) -> &'static str {
    match value {
        CandidateAction::RemoveKnownRegenerableContents => "remove_known_regenerable_contents",
        CandidateAction::EvictLocalCopy => "evict_local_copy",
        CandidateAction::MoveToTrash => "move_to_trash",
        CandidateAction::RevealOnly => "reveal_only",
        CandidateAction::NoAction => "no_action",
    }
}

pub(super) fn action_from_stored(value: &str) -> Result<CandidateAction, HistoryError> {
    match value {
        "remove_known_regenerable_contents" => Ok(CandidateAction::RemoveKnownRegenerableContents),
        "evict_local_copy" => Ok(CandidateAction::EvictLocalCopy),
        "move_to_trash" => Ok(CandidateAction::MoveToTrash),
        "reveal_only" => Ok(CandidateAction::RevealOnly),
        "no_action" => Ok(CandidateAction::NoAction),
        _ => Err(corrupt()),
    }
}

fn evidence_kind_as_stored(value: &Evidence) -> &'static str {
    match value {
        Evidence::MatchedPath { .. } => "matched_path",
        Evidence::RequiredMarker { .. } => "required_marker",
        Evidence::ForbiddenMarkerAbsent { .. } => "forbidden_marker_absent",
        Evidence::BundleIdentifier { .. } => "bundle_identifier",
        Evidence::MinimumAge { .. } => "minimum_age",
        Evidence::MinimumSize { .. } => "minimum_size",
        Evidence::InactiveProcess { .. } => "inactive_process",
        Evidence::CloudUploadComplete { .. } => "cloud_upload_complete",
    }
}

fn blocker_as_stored(value: &BlockReason) -> &'static str {
    match value {
        BlockReason::MissingOrIncompleteEvidence => "missing_or_incomplete_evidence",
        BlockReason::MissingModificationTime => "missing_modification_time",
        BlockReason::PartialScanCoverage => "partial_scan_coverage",
        BlockReason::RecentActivity => "recent_activity",
        BlockReason::BelowMinimumBytes => "below_minimum_bytes",
        BlockReason::ActiveUse => "active_use",
        BlockReason::AccessDenied => "access_denied",
        BlockReason::ProtectedPath => "protected_path",
        BlockReason::ProtectedDescendant => "protected_descendant",
        BlockReason::SymlinkBoundary => "symlink_boundary",
        BlockReason::VolumeBoundary => "volume_boundary",
        BlockReason::ChangedSinceScan => "changed_since_scan",
        BlockReason::UnsupportedPlatform => "unsupported_platform",
        BlockReason::CloudUploadUnconfirmed => "cloud_upload_unconfirmed",
    }
}

fn blocker_from_stored(value: &str) -> Result<BlockReason, HistoryError> {
    match value {
        "missing_or_incomplete_evidence" => Ok(BlockReason::MissingOrIncompleteEvidence),
        "missing_modification_time" => Ok(BlockReason::MissingModificationTime),
        "partial_scan_coverage" => Ok(BlockReason::PartialScanCoverage),
        "recent_activity" => Ok(BlockReason::RecentActivity),
        "below_minimum_bytes" => Ok(BlockReason::BelowMinimumBytes),
        "active_use" => Ok(BlockReason::ActiveUse),
        "access_denied" => Ok(BlockReason::AccessDenied),
        "protected_path" => Ok(BlockReason::ProtectedPath),
        "protected_descendant" => Ok(BlockReason::ProtectedDescendant),
        "symlink_boundary" => Ok(BlockReason::SymlinkBoundary),
        "volume_boundary" => Ok(BlockReason::VolumeBoundary),
        "changed_since_scan" => Ok(BlockReason::ChangedSinceScan),
        "unsupported_platform" => Ok(BlockReason::UnsupportedPlatform),
        "cloud_upload_unconfirmed" => Ok(BlockReason::CloudUploadUnconfirmed),
        _ => Err(corrupt()),
    }
}

fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

const fn invalid() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidInput)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        CandidateInput, LocalizedTextKey, ProvenanceUrl, Rule, RuleDefinition, RuleGuards,
        RuleMatcher, RuleMatcherDefinition, RuleScope,
    };
    use crate::persistence::StoreCoordinator;
    use crate::persistence::history::{
        NewScanRecord, ScanCompletionRecord, ScanCounts, TerminalScanStatus,
    };
    use tempfile::TempDir;

    const ALL_BLOCKERS: [BlockReason; 14] = [
        BlockReason::MissingOrIncompleteEvidence,
        BlockReason::MissingModificationTime,
        BlockReason::PartialScanCoverage,
        BlockReason::RecentActivity,
        BlockReason::BelowMinimumBytes,
        BlockReason::ActiveUse,
        BlockReason::AccessDenied,
        BlockReason::ProtectedPath,
        BlockReason::ProtectedDescendant,
        BlockReason::SymlinkBoundary,
        BlockReason::VolumeBoundary,
        BlockReason::ChangedSinceScan,
        BlockReason::UnsupportedPlatform,
        BlockReason::CloudUploadUnconfirmed,
    ];

    fn matcher() -> RuleMatcher {
        RuleMatcher::try_new(RuleMatcherDefinition {
            path_component: Some("candidate-fixture".to_owned()),
            required_ancestor_markers_any: Vec::new(),
            required_markers_all: Vec::new(),
            forbidden_markers_any: Vec::new(),
            exact_bundle_identifiers: Vec::new(),
            excluded_descendants: Vec::new(),
            protected_descendants: Vec::new(),
        })
        .unwrap()
    }

    fn rule(
        id: &str,
        category: CandidateCategory,
        safety: SafetyTier,
        action: CandidateAction,
        schedule_eligible: bool,
    ) -> Rule {
        Rule::try_new(RuleDefinition {
            reference: RuleRef::new(RuleId::new(id).unwrap(), RuleRevision::new(7).unwrap()),
            title_key: LocalizedTextKey::new("fixture.candidate.title").unwrap(),
            category,
            scope: RuleScope::ConfiguredProjectRoots,
            matcher: matcher(),
            guards: RuleGuards::try_new(
                None,
                0,
                Vec::new(),
                action == CandidateAction::EvictLocalCopy,
            )
            .unwrap(),
            safety,
            action,
            schedule_eligible,
            explanation_key: LocalizedTextKey::new("fixture.candidate.explanation").unwrap(),
            provenance: vec![ProvenanceUrl::new("https://example.com/candidate").unwrap()],
        })
        .unwrap()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the persistence fixture keeps every frozen candidate fact explicit at call sites"
    )]
    fn candidate(
        id: &str,
        scan_id: &str,
        rule: &Rule,
        paths: Vec<PathBuf>,
        evidence: Vec<Evidence>,
        blockers: Vec<BlockReason>,
        newest_mtime: Option<SystemTime>,
        estimated_bytes: u64,
    ) -> Candidate {
        Candidate::try_from_rule(
            rule,
            CandidateInput::new(
                CandidateId::new(id).unwrap(),
                paths,
                estimated_bytes,
                newest_mtime,
                evidence,
                blockers,
                ScanId::new(scan_id).unwrap(),
            ),
        )
        .unwrap()
    }

    fn start_and_finish_scan(store: &StoreCoordinator, root: &Path, id: &str) {
        let scan_id = ScanId::new(id).unwrap();
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    scan_id.clone(),
                    root.to_path_buf(),
                    UNIX_EPOCH + Duration::from_secs(1_750_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        store
            .record_scan_finished(
                &ScanCompletionRecord::try_new(
                    scan_id,
                    UNIX_EPOCH + Duration::from_secs(1_750_000_001),
                    TerminalScanStatus::Succeeded,
                    ScanCounts::default(),
                )
                .unwrap(),
            )
            .unwrap();
    }

    fn load_complete(store: &StoreCoordinator, id: &CandidateId) -> CompleteCandidateRecord {
        let StoredCandidateRecord::Complete(record) =
            store.load_candidate(id).unwrap().expect("candidate exists")
        else {
            panic!("format-2 candidate became a legacy summary");
        };
        record
    }

    fn persist_review_candidate(
        store: &StoreCoordinator,
        root: &Path,
        id: &str,
        scan_id: &str,
        policy: &Rule,
        blockers: Vec<BlockReason>,
    ) -> CandidateId {
        let path = root.join(format!("candidate-fixture-{id}"));
        let candidate = candidate(
            id,
            scan_id,
            policy,
            vec![path.clone()],
            vec![Evidence::MatchedPath { path }],
            blockers,
            None,
            4_096,
        );
        store
            .record_candidate_discovered(
                &NewCandidateRecord::try_from_candidate(
                    &candidate,
                    UNIX_EPOCH + Duration::from_secs(1_750_000_002),
                )
                .unwrap(),
            )
            .unwrap();
        candidate.id().clone()
    }

    #[test]
    fn review_status_is_typed_idempotent_and_preserves_frozen_facts_after_reopen() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("root");
        let store = StoreCoordinator::open(&database).unwrap();
        start_and_finish_scan(&store, &root, "scan:review-status");
        let policy = rule(
            "fixture.candidate.review-status",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let id = persist_review_candidate(
            &store,
            &root,
            "candidate:review-status",
            "scan:review-status",
            &policy,
            Vec::new(),
        );
        let discovered = load_complete(&store, &id);

        assert_eq!(
            store
                .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
                .unwrap(),
            CandidateHistoryStatus::Selected
        );
        assert_eq!(
            store
                .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
                .unwrap(),
            CandidateHistoryStatus::Selected
        );
        let mut expected = discovered.clone();
        expected.status = CandidateHistoryStatus::Selected;
        assert_eq!(load_complete(&store, &id), expected);

        assert_eq!(
            store
                .transition_candidate_review_status(&id, CandidateReviewTransition::ClearSelection,)
                .unwrap(),
            CandidateHistoryStatus::Discovered
        );
        assert_eq!(load_complete(&store, &id), discovered);
        assert_eq!(
            store
                .transition_candidate_review_status(
                    &id,
                    CandidateReviewTransition::DismissDiscovered,
                )
                .unwrap(),
            CandidateHistoryStatus::Dismissed
        );
        assert_eq!(
            store
                .transition_candidate_review_status(
                    &id,
                    CandidateReviewTransition::DismissDiscovered,
                )
                .unwrap(),
            CandidateHistoryStatus::Dismissed
        );
        assert_eq!(
            store
                .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        assert_eq!(
            store
                .transition_candidate_review_status(&id, CandidateReviewTransition::Restore)
                .unwrap(),
            CandidateHistoryStatus::Discovered
        );
        assert_eq!(
            store
                .transition_candidate_review_status(&id, CandidateReviewTransition::Restore)
                .unwrap(),
            CandidateHistoryStatus::Discovered
        );
        store
            .transition_candidate_review_status(&id, CandidateReviewTransition::DismissDiscovered)
            .unwrap();

        let selected_dismissal = persist_review_candidate(
            &store,
            &root,
            "candidate:review-selected-dismissal",
            "scan:review-status",
            &policy,
            Vec::new(),
        );
        store
            .transition_candidate_review_status(
                &selected_dismissal,
                CandidateReviewTransition::Select,
            )
            .unwrap();
        assert_eq!(
            store
                .transition_candidate_review_status(
                    &selected_dismissal,
                    CandidateReviewTransition::DismissSelected,
                )
                .unwrap(),
            CandidateHistoryStatus::Dismissed
        );

        drop(store);
        let reopened = StoreCoordinator::open(&database).unwrap();
        expected.status = CandidateHistoryStatus::Dismissed;
        assert_eq!(load_complete(&reopened, &id), expected);
    }

    #[test]
    fn evaluator_status_is_source_typed_terminal_and_unavailable_refines_to_stale() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let root = temp.path().join("root");
        let store = StoreCoordinator::open(&database).unwrap();
        let scan_id = "scan:evaluator-status";
        start_and_finish_scan(&store, &root, scan_id);
        let policy = rule(
            "fixture.candidate.evaluator-status",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let cases = [
            (
                "discovered-stale",
                None,
                CandidateEvaluationTransition::DiscoveredToStale,
                CandidateHistoryStatus::Stale,
            ),
            (
                "selected-stale",
                Some(CandidateReviewTransition::Select),
                CandidateEvaluationTransition::SelectedToStale,
                CandidateHistoryStatus::Stale,
            ),
            (
                "dismissed-stale",
                Some(CandidateReviewTransition::DismissDiscovered),
                CandidateEvaluationTransition::DismissedToStale,
                CandidateHistoryStatus::Stale,
            ),
            (
                "discovered-unavailable",
                None,
                CandidateEvaluationTransition::DiscoveredToUnavailable,
                CandidateHistoryStatus::Unavailable,
            ),
            (
                "selected-unavailable",
                Some(CandidateReviewTransition::Select),
                CandidateEvaluationTransition::SelectedToUnavailable,
                CandidateHistoryStatus::Unavailable,
            ),
            (
                "dismissed-unavailable",
                Some(CandidateReviewTransition::DismissDiscovered),
                CandidateEvaluationTransition::DismissedToUnavailable,
                CandidateHistoryStatus::Unavailable,
            ),
        ];

        for (label, review, transition, target) in cases {
            let id = persist_review_candidate(
                &store,
                &root,
                &format!("candidate:evaluator-{label}"),
                scan_id,
                &policy,
                Vec::new(),
            );
            if let Some(review) = review {
                store
                    .transition_candidate_review_status(&id, review)
                    .unwrap();
            }
            let before = load_complete(&store, &id);
            assert_eq!(
                store
                    .transition_candidate_evaluation_status(&id, transition)
                    .unwrap(),
                target
            );
            assert_eq!(
                store
                    .transition_candidate_evaluation_status(&id, transition)
                    .unwrap(),
                target
            );
            let mut expected = before;
            expected.status = target;
            assert_eq!(load_complete(&store, &id), expected);
            assert_eq!(
                store
                    .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
                    .unwrap_err()
                    .kind,
                HistoryErrorKind::InvalidTransition
            );
        }

        let refined = persist_review_candidate(
            &store,
            &root,
            "candidate:evaluator-refined",
            scan_id,
            &policy,
            Vec::new(),
        );
        store
            .transition_candidate_evaluation_status(
                &refined,
                CandidateEvaluationTransition::DiscoveredToUnavailable,
            )
            .unwrap();
        assert_eq!(
            store
                .transition_candidate_evaluation_status(
                    &refined,
                    CandidateEvaluationTransition::UnavailableToStale,
                )
                .unwrap(),
            CandidateHistoryStatus::Stale
        );
        assert_eq!(
            store
                .transition_candidate_evaluation_status(
                    &refined,
                    CandidateEvaluationTransition::DiscoveredToUnavailable,
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );

        let wrong_source = persist_review_candidate(
            &store,
            &root,
            "candidate:evaluator-wrong-source",
            scan_id,
            &policy,
            Vec::new(),
        );
        assert_eq!(
            store
                .transition_candidate_evaluation_status(
                    &wrong_source,
                    CandidateEvaluationTransition::SelectedToStale,
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        assert_eq!(
            load_complete(&store, &wrong_source).status,
            CandidateHistoryStatus::Discovered
        );

        let reconciled = persist_review_candidate(
            &store,
            &root,
            "candidate:evaluator-reconciled",
            scan_id,
            &policy,
            Vec::new(),
        );
        assert_eq!(
            store
                .transition_candidate_evaluation_status_after_commit_failure_for_test(
                    &reconciled,
                    CandidateEvaluationTransition::DiscoveredToStale,
                )
                .unwrap(),
            CandidateHistoryStatus::Stale
        );

        drop(store);
        let reopened = StoreCoordinator::open(&database).unwrap();
        assert_eq!(
            load_complete(&reopened, &reconciled).status,
            CandidateHistoryStatus::Stale
        );
    }

    #[test]
    fn review_selection_rejects_blocked_and_non_cleanup_candidates_without_erasing_facts() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        start_and_finish_scan(&store, &root, "scan:review-policy");
        let cleanup = rule(
            "fixture.candidate.review-cleanup",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let informational = rule(
            "fixture.candidate.review-info",
            CandidateCategory::UnknownStorage,
            SafetyTier::Informational,
            CandidateAction::RevealOnly,
            false,
        );
        let blocked = persist_review_candidate(
            &store,
            &root,
            "candidate:review-blocked",
            "scan:review-policy",
            &cleanup,
            vec![BlockReason::PartialScanCoverage],
        );
        let info = persist_review_candidate(
            &store,
            &root,
            "candidate:review-info",
            "scan:review-policy",
            &informational,
            Vec::new(),
        );

        for id in [&blocked, &info] {
            assert_eq!(
                store
                    .transition_candidate_review_status(id, CandidateReviewTransition::Select)
                    .unwrap_err()
                    .kind,
                HistoryErrorKind::InvalidTransition
            );
            assert_eq!(
                load_complete(&store, id).status,
                CandidateHistoryStatus::Discovered
            );
            store.with_connection(|connection| {
                connection
                    .execute(
                        "UPDATE candidates SET status = 'selected' WHERE candidate_id = ?1",
                        [id.as_str()],
                    )
                    .unwrap();
            });
            assert_eq!(
                store
                    .transition_candidate_review_status(id, CandidateReviewTransition::Select)
                    .unwrap_err()
                    .kind,
                HistoryErrorKind::InvalidTransition
            );
            assert_eq!(
                store
                    .transition_candidate_review_status(
                        id,
                        CandidateReviewTransition::DismissSelected,
                    )
                    .unwrap(),
                CandidateHistoryStatus::Dismissed
            );
            assert_eq!(
                store
                    .transition_candidate_evaluation_status(
                        id,
                        CandidateEvaluationTransition::DismissedToStale,
                    )
                    .unwrap(),
                CandidateHistoryStatus::Stale
            );
        }
        assert_eq!(
            load_complete(&store, &blocked).blockers,
            vec![BlockReason::PartialScanCoverage]
        );
    }

    #[test]
    fn review_selection_policy_matrix_is_exhaustive_and_all_blockers_fail_closed() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        let scan_id = "scan:review-policy-matrix";
        start_and_finish_scan(&store, &root, scan_id);

        let cleanup_policies = [
            (
                "regenerable",
                rule(
                    "fixture.candidate.matrix.regenerable",
                    CandidateCategory::ApplicationCache,
                    SafetyTier::SafeRegenerable,
                    CandidateAction::RemoveKnownRegenerableContents,
                    false,
                ),
            ),
            (
                "evictable",
                rule(
                    "fixture.candidate.matrix.evictable",
                    CandidateCategory::CloudFile,
                    SafetyTier::SafeEvictable,
                    CandidateAction::EvictLocalCopy,
                    false,
                ),
            ),
            (
                "trash",
                rule(
                    "fixture.candidate.matrix.trash",
                    CandidateCategory::LargeReviewItem,
                    SafetyTier::ReviewRequired,
                    CandidateAction::MoveToTrash,
                    false,
                ),
            ),
        ];
        for (label, policy) in cleanup_policies {
            let path = root.join(format!("candidate-fixture-matrix-{label}"));
            let evidence = if policy.action() == CandidateAction::EvictLocalCopy {
                vec![Evidence::CloudUploadComplete { path: path.clone() }]
            } else {
                vec![Evidence::MatchedPath { path: path.clone() }]
            };
            let candidate = candidate(
                &format!("candidate:matrix-{label}"),
                scan_id,
                &policy,
                vec![path],
                evidence,
                Vec::new(),
                None,
                1,
            );
            store
                .record_candidate_discovered(
                    &NewCandidateRecord::try_from_candidate(
                        &candidate,
                        UNIX_EPOCH + Duration::from_secs(1_750_000_002),
                    )
                    .unwrap(),
                )
                .unwrap();
            assert_eq!(
                store
                    .transition_candidate_review_status(
                        candidate.id(),
                        CandidateReviewTransition::Select,
                    )
                    .unwrap(),
                CandidateHistoryStatus::Selected
            );
        }

        for (label, safety, action) in [
            (
                "informational",
                SafetyTier::Informational,
                CandidateAction::RevealOnly,
            ),
            (
                "protected",
                SafetyTier::Protected,
                CandidateAction::NoAction,
            ),
        ] {
            let policy = rule(
                &format!("fixture.candidate.matrix.{label}"),
                CandidateCategory::UnknownStorage,
                safety,
                action,
                false,
            );
            let id = persist_review_candidate(
                &store,
                &root,
                &format!("candidate:matrix-{label}"),
                scan_id,
                &policy,
                Vec::new(),
            );
            assert_eq!(
                store
                    .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
                    .unwrap_err()
                    .kind,
                HistoryErrorKind::InvalidTransition
            );
        }

        let cleanup = rule(
            "fixture.candidate.matrix.blocked",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        for (index, blocker) in ALL_BLOCKERS.iter().enumerate() {
            let id = persist_review_candidate(
                &store,
                &root,
                &format!("candidate:matrix-blocker-{index}"),
                scan_id,
                &cleanup,
                vec![blocker.clone()],
            );
            assert_eq!(
                store
                    .transition_candidate_review_status(&id, CandidateReviewTransition::Select)
                    .unwrap_err()
                    .kind,
                HistoryErrorKind::InvalidTransition,
                "selection accepted blocker {blocker:?}"
            );
        }
    }

    #[test]
    fn review_status_reconciles_post_commit_failure_and_exact_cas_races() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        start_and_finish_scan(&store, &root, "scan:review-race");
        let policy = rule(
            "fixture.candidate.review-race",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let reconciled = persist_review_candidate(
            &store,
            &root,
            "candidate:review-reconciled",
            "scan:review-race",
            &policy,
            Vec::new(),
        );
        assert_eq!(
            store
                .transition_candidate_review_status_after_commit_failure_for_test(
                    &reconciled,
                    CandidateReviewTransition::Select,
                )
                .unwrap(),
            CandidateHistoryStatus::Selected
        );

        let raced = persist_review_candidate(
            &store,
            &root,
            "candidate:review-raced",
            "scan:review-race",
            &policy,
            Vec::new(),
        );
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let first_store = std::sync::Arc::clone(&store);
        let first_id = raced.clone();
        let first_barrier = std::sync::Arc::clone(&barrier);
        let first = std::thread::spawn(move || {
            first_barrier.wait();
            first_store
                .transition_candidate_review_status(&first_id, CandidateReviewTransition::Select)
        });
        let second_store = std::sync::Arc::clone(&store);
        let second_id = raced.clone();
        let second_barrier = std::sync::Arc::clone(&barrier);
        let second = std::thread::spawn(move || {
            second_barrier.wait();
            second_store.transition_candidate_review_status(
                &second_id,
                CandidateReviewTransition::DismissDiscovered,
            )
        });
        barrier.wait();
        let results = [first.join().unwrap(), second.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    result
                        .as_ref()
                        .is_err_and(|error| error.kind == HistoryErrorKind::InvalidTransition)
                })
                .count(),
            1
        );
        assert!(matches!(
            load_complete(&store, &raced).status,
            CandidateHistoryStatus::Selected | CandidateHistoryStatus::Dismissed
        ));

        let evaluator_race = persist_review_candidate(
            &store,
            &root,
            "candidate:review-evaluator-race",
            "scan:review-race",
            &policy,
            Vec::new(),
        );
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let review_store = std::sync::Arc::clone(&store);
        let review_id = evaluator_race.clone();
        let review_barrier = std::sync::Arc::clone(&barrier);
        let review = std::thread::spawn(move || {
            review_barrier.wait();
            review_store
                .transition_candidate_review_status(&review_id, CandidateReviewTransition::Select)
        });
        let evaluator_store = std::sync::Arc::clone(&store);
        let evaluator_id = evaluator_race.clone();
        let evaluator_barrier = std::sync::Arc::clone(&barrier);
        let evaluator = std::thread::spawn(move || {
            evaluator_barrier.wait();
            evaluator_store.transition_candidate_evaluation_status(
                &evaluator_id,
                CandidateEvaluationTransition::DiscoveredToStale,
            )
        });
        barrier.wait();
        let results = [review.join().unwrap(), evaluator.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    result
                        .as_ref()
                        .is_err_and(|error| error.kind == HistoryErrorKind::InvalidTransition)
                })
                .count(),
            1
        );
        assert!(matches!(
            load_complete(&store, &evaluator_race).status,
            CandidateHistoryStatus::Selected | CandidateHistoryStatus::Stale
        ));

        let evaluator_refinement = persist_review_candidate(
            &store,
            &root,
            "candidate:evaluator-refinement-race",
            "scan:review-race",
            &policy,
            Vec::new(),
        );
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let unavailable_store = std::sync::Arc::clone(&store);
        let unavailable_id = evaluator_refinement.clone();
        let unavailable_barrier = std::sync::Arc::clone(&barrier);
        let unavailable = std::thread::spawn(move || {
            unavailable_barrier.wait();
            unavailable_store.transition_candidate_evaluation_status(
                &unavailable_id,
                CandidateEvaluationTransition::DiscoveredToUnavailable,
            )
        });
        let stale_store = std::sync::Arc::clone(&store);
        let stale_id = evaluator_refinement.clone();
        let stale_barrier = std::sync::Arc::clone(&barrier);
        let stale = std::thread::spawn(move || {
            stale_barrier.wait();
            stale_store.transition_candidate_evaluation_status(
                &stale_id,
                CandidateEvaluationTransition::DiscoveredToStale,
            )
        });
        barrier.wait();
        let results = [unavailable.join().unwrap(), stale.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    result
                        .as_ref()
                        .is_err_and(|error| error.kind == HistoryErrorKind::InvalidTransition)
                })
                .count(),
            1
        );
        if load_complete(&store, &evaluator_refinement).status
            == CandidateHistoryStatus::Unavailable
        {
            store
                .transition_candidate_evaluation_status(
                    &evaluator_refinement,
                    CandidateEvaluationTransition::UnavailableToStale,
                )
                .unwrap();
        }
        assert_eq!(
            load_complete(&store, &evaluator_refinement).status,
            CandidateHistoryStatus::Stale
        );
    }

    #[cfg(unix)]
    #[test]
    fn review_status_exact_match_cannot_mask_unsafe_post_commit_storage() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let marker = database.with_extension("sqlite3.writer.lock");
        let store = StoreCoordinator::open(&database).unwrap();
        let root = temp.path().join("root");
        start_and_finish_scan(&store, &root, "scan:review-unsafe");
        let policy = rule(
            "fixture.candidate.review-unsafe",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let id = persist_review_candidate(
            &store,
            &root,
            "candidate:review-unsafe",
            "scan:review-unsafe",
            &policy,
            Vec::new(),
        );

        let error = store
            .transition_candidate_review_status_with_after_commit_hook_for_test(
                &id,
                CandidateReviewTransition::Select,
                || {
                    std::fs::write(&marker, b"NOT-A-DUX-MARKER")
                        .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))
                },
            )
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::OutcomeUnknown);
        store.with_connection(|connection| {
            let durable: String = connection
                .query_row(
                    "SELECT status FROM candidates WHERE candidate_id = ?1",
                    [id.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(durable, "selected");
        });
    }

    #[cfg(unix)]
    #[test]
    fn evaluator_status_exact_match_cannot_mask_unsafe_post_commit_storage() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let marker = database.with_extension("sqlite3.writer.lock");
        let store = StoreCoordinator::open(&database).unwrap();
        let root = temp.path().join("root");
        start_and_finish_scan(&store, &root, "scan:evaluator-unsafe");
        let policy = rule(
            "fixture.candidate.evaluator-unsafe",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let id = persist_review_candidate(
            &store,
            &root,
            "candidate:evaluator-unsafe",
            "scan:evaluator-unsafe",
            &policy,
            Vec::new(),
        );

        let error = store
            .transition_candidate_evaluation_status_with_after_commit_hook_for_test(
                &id,
                CandidateEvaluationTransition::DiscoveredToStale,
                || {
                    std::fs::write(&marker, b"NOT-A-DUX-MARKER")
                        .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))
                },
            )
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::OutcomeUnknown);
        store.with_connection(|connection| {
            let durable: String = connection
                .query_row(
                    "SELECT status FROM candidates WHERE candidate_id = ?1",
                    [id.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(durable, "stale");
        });
    }

    #[test]
    fn candidate_status_boundaries_refuse_missing_legacy_corrupt_and_foreign_owned_rows() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        start_and_finish_scan(&store, &root, "scan:review-refusal");
        let missing = CandidateId::new("candidate:review-missing").unwrap();
        assert_eq!(
            store
                .transition_candidate_review_status(&missing, CandidateReviewTransition::Select,)
                .unwrap_err()
                .kind,
            HistoryErrorKind::NotFound
        );
        assert_eq!(
            store
                .transition_candidate_evaluation_status(
                    &missing,
                    CandidateEvaluationTransition::DiscoveredToStale,
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::NotFound
        );

        let legacy = CandidateId::new("candidate:review-legacy").unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO candidates (
                         candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                         estimated_bytes, created_at_unix_ms, status, record_format_version
                     ) VALUES (?1, 'scan:review-refusal', 'fixture.legacy', 1,
                         'review_required', 1, 1, 'discovered', 1)",
                    [legacy.as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            store
                .transition_candidate_review_status(&legacy, CandidateReviewTransition::Select)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        assert_eq!(
            store
                .transition_candidate_evaluation_status(
                    &legacy,
                    CandidateEvaluationTransition::DiscoveredToStale,
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        assert_eq!(
            store.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT status FROM candidates WHERE candidate_id = ?1",
                        [legacy.as_str()],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap()
            }),
            "discovered"
        );

        let policy = rule(
            "fixture.candidate.review-refusal",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let lifecycle = persist_review_candidate(
            &store,
            &root,
            "candidate:review-lifecycle",
            "scan:review-refusal",
            &policy,
            Vec::new(),
        );
        for status in ["stale", "planned", "completed", "failed", "unavailable"] {
            store.with_connection(|connection| {
                connection
                    .execute(
                        "UPDATE candidates SET status = ?1 WHERE candidate_id = ?2",
                        params![status, lifecycle.as_str()],
                    )
                    .unwrap();
            });
            let expected = if status == "planned" {
                HistoryErrorKind::CorruptData
            } else {
                HistoryErrorKind::InvalidTransition
            };
            assert_eq!(
                store
                    .transition_candidate_review_status(
                        &lifecycle,
                        CandidateReviewTransition::Select,
                    )
                    .unwrap_err()
                    .kind,
                expected,
                "review API accepted lifecycle-owned {status} status"
            );
            if matches!(status, "planned" | "completed" | "failed") {
                assert_eq!(
                    store
                        .transition_candidate_evaluation_status(
                            &lifecycle,
                            CandidateEvaluationTransition::DiscoveredToStale,
                        )
                        .unwrap_err()
                        .kind,
                    expected,
                    "evaluator API accepted planner/journal-owned {status} status"
                );
            }
        }

        let corrupt = persist_review_candidate(
            &store,
            &root,
            "candidate:review-corrupt",
            "scan:review-refusal",
            &policy,
            Vec::new(),
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM candidate_evidence WHERE candidate_id = ?1",
                    [corrupt.as_str()],
                )
                .unwrap();
        });
        assert_eq!(
            store
                .transition_candidate_review_status(&corrupt, CandidateReviewTransition::Select,)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
        assert_eq!(
            store
                .transition_candidate_evaluation_status(
                    &corrupt,
                    CandidateEvaluationTransition::DiscoveredToStale,
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
        assert_eq!(
            store.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT status FROM candidates WHERE candidate_id = ?1",
                        [corrupt.as_str()],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap()
            }),
            "discovered"
        );
    }

    #[test]
    fn complete_candidates_require_a_durably_succeeded_source_scan() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        let policy = rule(
            "fixture.candidate.source-scan",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );

        let cases = [
            ("failed", Some(TerminalScanStatus::Failed)),
            ("cancelled", Some(TerminalScanStatus::Cancelled)),
            ("interrupted", Some(TerminalScanStatus::Interrupted)),
            ("running", None),
        ];
        for (label, terminal) in cases {
            let scan_id = ScanId::new(format!("scan:candidate-source-{label}")).unwrap();
            store
                .record_scan_started(
                    &NewScanRecord::try_new(
                        scan_id.clone(),
                        root.clone(),
                        UNIX_EPOCH + Duration::from_secs(1_750_000_000),
                    )
                    .unwrap(),
                )
                .unwrap();
            if let Some(terminal) = terminal {
                store
                    .record_scan_finished(
                        &ScanCompletionRecord::try_new(
                            scan_id.clone(),
                            UNIX_EPOCH + Duration::from_secs(1_750_000_001),
                            terminal,
                            ScanCounts::default(),
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
            let path = root.join(format!("candidate-fixture-{label}"));
            let candidate = candidate(
                &format!("candidate:source-{label}"),
                scan_id.as_str(),
                &policy,
                vec![path.clone()],
                vec![Evidence::MatchedPath { path }],
                Vec::new(),
                None,
                1,
            );
            assert_eq!(
                store
                    .record_candidate_discovered(
                        &NewCandidateRecord::try_from_candidate(
                            &candidate,
                            UNIX_EPOCH + Duration::from_secs(1_750_000_002),
                        )
                        .unwrap(),
                    )
                    .unwrap_err()
                    .kind,
                HistoryErrorKind::InvalidTransition,
                "candidate accepted {label} source scan"
            );
        }

        let queued_scan = ScanId::new("scan:candidate-source-queued").unwrap();
        store
            .record_scan_started(
                &NewScanRecord::try_new(
                    queued_scan.clone(),
                    root.clone(),
                    UNIX_EPOCH + Duration::from_secs(1_750_000_000),
                )
                .unwrap(),
            )
            .unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                    [queued_scan.as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE scans SET status = 'queued' WHERE scan_id = ?1",
                    [queued_scan.as_str()],
                )
                .unwrap();
        });
        let queued_path = root.join("candidate-fixture-queued");
        let queued_candidate = candidate(
            "candidate:source-queued",
            queued_scan.as_str(),
            &policy,
            vec![queued_path.clone()],
            vec![Evidence::MatchedPath { path: queued_path }],
            Vec::new(),
            None,
            1,
        );
        assert_eq!(
            store
                .record_candidate_discovered(
                    &NewCandidateRecord::try_from_candidate(
                        &queued_candidate,
                        UNIX_EPOCH + Duration::from_secs(1_750_000_002),
                    )
                    .unwrap(),
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );

        start_and_finish_scan(&store, &root, "scan:candidate-source-succeeded");
        let accepted = persist_review_candidate(
            &store,
            &root,
            "candidate:source-succeeded",
            "scan:candidate-source-succeeded",
            &policy,
            Vec::new(),
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE scans SET status = 'failed'
                     WHERE scan_id = 'scan:candidate-source-succeeded'",
                    [],
                )
                .unwrap();
        });
        assert_eq!(
            store
                .load_scan(&ScanId::new("scan:candidate-source-succeeded").unwrap())
                .unwrap()
                .unwrap()
                .status(),
            ScanStatus::Failed
        );
        assert_eq!(
            store.load_candidate(&accepted).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn candidate_query_budget_survives_the_nested_source_scan_read() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let root = temp.path().join("root");
        start_and_finish_scan(&store, &root, "scan:candidate-budget");
        let policy = rule(
            "fixture.candidate.budget",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let id = persist_review_candidate(
            &store,
            &root,
            "candidate:budget",
            "scan:candidate-budget",
            &policy,
            Vec::new(),
        );

        store.with_connection(|connection| {
            let interrupt = connection.get_interrupt_handle();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let failsafe = std::thread::spawn(move || {
                if done_rx.recv_timeout(Duration::from_secs(2)).is_err() {
                    interrupt.interrupt();
                }
            });
            let started = std::time::Instant::now();
            let error = run_bounded_query(connection, || {
                load_candidate_record_within_budget_and_hook(connection, &id, |connection| {
                    connection
                        .query_row(
                            "WITH RECURSIVE counter(value) AS (
                                 VALUES(0)
                                 UNION ALL
                                 SELECT value + 1 FROM counter WHERE value < 100000000
                             )
                             SELECT sum(value) FROM counter",
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .map(|_| ())
                        .map_err(map_query_sql_error)
                })
                .map(|_| ())
            })
            .unwrap_err();
            let elapsed = started.elapsed();
            let _ = done_tx.send(());
            failsafe.join().unwrap();

            assert_eq!(error.kind, HistoryErrorKind::QueryLimitExceeded);
            assert!(
                elapsed < Duration::from_secs(1),
                "candidate query budget was removed before child loading: {elapsed:?}"
            );
        });
    }

    #[test]
    fn complete_candidate_round_trip_preserves_ordered_facts_after_reopen() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("scan-root-å");
        let first = root.join("candidate-fixture-a");
        let second = root.join("candidate-fixture-b");
        let marker = root.join("Cargo.toml");
        let newest = UNIX_EPOCH + Duration::new(1_750_000_010, 123_456_789);
        let evidence = vec![
            Evidence::MatchedPath {
                path: first.clone(),
            },
            Evidence::RequiredMarker {
                path: marker.clone(),
            },
            Evidence::ForbiddenMarkerAbsent {
                path: root.join("keep"),
            },
            Evidence::BundleIdentifier {
                path: second.clone(),
                identifier: "com.example.fixture".to_owned(),
            },
            Evidence::MinimumAge {
                newest_mtime: newest,
                minimum_age: Duration::new(86_400, 987_654_321),
            },
            Evidence::MinimumSize {
                observed_bytes: 123_456,
                minimum_bytes: 65_536,
            },
            Evidence::InactiveProcess {
                identifier: "com.example.fixture".to_owned(),
            },
            Evidence::CloudUploadComplete {
                path: second.clone(),
            },
        ];
        let policy = rule(
            "fixture.candidate.complete",
            CandidateCategory::DeveloperArtifact,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            true,
        );
        let candidate = candidate(
            "candidate:complete",
            "scan:complete",
            &policy,
            vec![first.clone(), second.clone()],
            evidence.clone(),
            ALL_BLOCKERS.to_vec(),
            Some(newest),
            123_456,
        );
        let created_at = UNIX_EPOCH + Duration::from_millis(1_750_000_020_123);
        let new = NewCandidateRecord::try_from_candidate(&candidate, created_at).unwrap();
        assert_eq!(new.candidate(), &candidate);
        assert_eq!(new.created_at(), created_at);

        {
            let store = StoreCoordinator::open(&database).unwrap();
            start_and_finish_scan(&store, &root, "scan:complete");
            store.record_candidate_discovered(&new).unwrap();
        }

        let store = StoreCoordinator::open(&database).unwrap();
        let stored = store.load_candidate(candidate.id()).unwrap().unwrap();
        let StoredCandidateRecord::Complete(stored) = stored else {
            panic!("format-2 candidate was not returned as complete");
        };
        assert_eq!(stored.id, candidate.id().clone());
        assert_eq!(stored.source_scan_id, candidate.source_scan_id().clone());
        assert_eq!(stored.rule, candidate.rule().clone());
        assert_eq!(stored.category, candidate.category());
        assert_eq!(stored.paths, [first, second]);
        assert_eq!(stored.estimated_bytes, candidate.estimated_bytes());
        assert_eq!(stored.newest_mtime, Some(newest));
        assert_eq!(stored.evidence, evidence);
        assert_eq!(stored.safety, candidate.safety());
        assert_eq!(stored.action, candidate.action());
        assert!(stored.rule_schedule_eligible);
        assert_eq!(stored.blockers, ALL_BLOCKERS);
        assert_eq!(stored.created_at, created_at);
        assert_eq!(stored.status, CandidateHistoryStatus::Discovered);
    }

    #[test]
    fn legal_policy_and_category_mappings_are_exhaustive() {
        let categories = [
            CandidateCategory::DeveloperArtifact,
            CandidateCategory::ApplicationCache,
            CandidateCategory::BrowserCache,
            CandidateCategory::LogAndDiagnostic,
            CandidateCategory::InstallerAndDownload,
            CandidateCategory::DeviceAndSimulatorData,
            CandidateCategory::CloudFile,
            CandidateCategory::LargeReviewItem,
            CandidateCategory::ProtectedSystemData,
            CandidateCategory::UnknownStorage,
        ];
        for category in categories {
            assert_eq!(
                category_from_stored(category_as_stored(category)).unwrap(),
                category
            );
        }
        let pairs = [
            (
                SafetyTier::SafeRegenerable,
                CandidateAction::RemoveKnownRegenerableContents,
            ),
            (SafetyTier::SafeEvictable, CandidateAction::EvictLocalCopy),
            (SafetyTier::ReviewRequired, CandidateAction::MoveToTrash),
            (SafetyTier::Informational, CandidateAction::RevealOnly),
            (SafetyTier::Informational, CandidateAction::NoAction),
            (SafetyTier::Protected, CandidateAction::RevealOnly),
            (SafetyTier::Protected, CandidateAction::NoAction),
        ];
        for (safety, action) in pairs {
            assert_eq!(
                safety_from_stored(safety_as_stored(safety)).unwrap(),
                safety
            );
            assert_eq!(
                action_from_stored(action_as_stored(action)).unwrap(),
                action
            );
            validate_policy(safety, action, false, corrupt).unwrap();
        }
        for blocker in ALL_BLOCKERS {
            assert_eq!(
                blocker_from_stored(blocker_as_stored(&blocker)).unwrap(),
                blocker
            );
        }
        let statuses = [
            ("discovered", CandidateHistoryStatus::Discovered),
            ("selected", CandidateHistoryStatus::Selected),
            ("dismissed", CandidateHistoryStatus::Dismissed),
            ("stale", CandidateHistoryStatus::Stale),
            ("planned", CandidateHistoryStatus::Planned),
            ("completed", CandidateHistoryStatus::Completed),
            ("failed", CandidateHistoryStatus::Failed),
            ("unavailable", CandidateHistoryStatus::Unavailable),
        ];
        for (stored, status) in statuses {
            assert_eq!(CandidateHistoryStatus::from_stored(stored).unwrap(), status);
        }
    }

    #[test]
    fn duplicate_and_missing_scan_writes_leave_no_partial_children() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let path = root.join("candidate-fixture");
        let policy = rule(
            "fixture.candidate.duplicate",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let missing = candidate(
            "candidate:missing-scan",
            "scan:missing",
            &policy,
            vec![path.clone()],
            vec![Evidence::MatchedPath { path: path.clone() }],
            Vec::new(),
            None,
            10,
        );
        let store = StoreCoordinator::open(&database).unwrap();
        let missing =
            NewCandidateRecord::try_from_candidate(&missing, UNIX_EPOCH + Duration::from_secs(1))
                .unwrap();
        assert_eq!(
            store
                .record_candidate_discovered(&missing)
                .unwrap_err()
                .kind,
            HistoryErrorKind::NotFound
        );
        start_and_finish_scan(&store, &root, "scan:duplicate");
        let duplicate = candidate(
            "candidate:duplicate",
            "scan:duplicate",
            &policy,
            vec![path.clone()],
            vec![Evidence::MatchedPath { path }],
            Vec::new(),
            None,
            10,
        );
        let duplicate =
            NewCandidateRecord::try_from_candidate(&duplicate, UNIX_EPOCH + Duration::from_secs(2))
                .unwrap();
        store.record_candidate_discovered(&duplicate).unwrap();
        assert_eq!(
            store
                .record_candidate_discovered(&duplicate)
                .unwrap_err()
                .kind,
            HistoryErrorKind::AlreadyExists
        );
        store.with_connection(|connection| {
            for table in ["candidates", "candidate_paths", "candidate_evidence"] {
                let sql = format!("SELECT count(*) FROM {table}");
                let count: i64 = connection.query_row(&sql, [], |row| row.get(0)).unwrap();
                assert_eq!(count, 1, "unexpected row count in {table}");
            }
        });
    }

    #[test]
    fn input_bounds_and_non_authoritative_paths_fail_before_writing() {
        let policy = rule(
            "fixture.candidate.invalid",
            CandidateCategory::UnknownStorage,
            SafetyTier::Informational,
            CandidateAction::RevealOnly,
            false,
        );
        let absolute = PathBuf::from("/candidate-fixture");
        let cases = [
            candidate(
                "candidate:relative",
                "scan:invalid",
                &policy,
                vec![PathBuf::from("relative")],
                vec![Evidence::MatchedPath {
                    path: absolute.clone(),
                }],
                Vec::new(),
                None,
                1,
            ),
            candidate(
                "candidate:pre-epoch",
                "scan:invalid",
                &policy,
                vec![absolute.clone()],
                vec![Evidence::MatchedPath {
                    path: absolute.clone(),
                }],
                Vec::new(),
                UNIX_EPOCH.checked_sub(Duration::from_nanos(1)),
                1,
            ),
            candidate(
                "candidate:large-bytes",
                "scan:invalid",
                &policy,
                vec![absolute.clone()],
                vec![Evidence::MatchedPath {
                    path: absolute.clone(),
                }],
                Vec::new(),
                None,
                u64::MAX,
            ),
            candidate(
                "candidate:control-text",
                "scan:invalid",
                &policy,
                vec![absolute.clone()],
                vec![Evidence::InactiveProcess {
                    identifier: "bad\nidentifier".to_owned(),
                }],
                Vec::new(),
                None,
                1,
            ),
        ];
        for invalid in cases {
            assert_eq!(
                NewCandidateRecord::try_from_candidate(
                    &invalid,
                    UNIX_EPOCH + Duration::from_secs(1)
                )
                .unwrap_err()
                .kind,
                HistoryErrorKind::InvalidInput
            );
        }

        let too_many_paths = (0..=MAX_PATHS)
            .map(|index| PathBuf::from(format!("/candidate-fixture-{index}")))
            .collect::<Vec<_>>();
        let too_many = candidate(
            "candidate:too-many",
            "scan:invalid",
            &policy,
            too_many_paths,
            vec![Evidence::MatchedPath {
                path: absolute.clone(),
            }],
            Vec::new(),
            None,
            1,
        );
        assert_eq!(
            NewCandidateRecord::try_from_candidate(&too_many, UNIX_EPOCH + Duration::from_secs(1))
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidInput
        );
        let valid = candidate(
            "candidate:valid",
            "scan:invalid",
            &policy,
            vec![absolute.clone()],
            vec![Evidence::MatchedPath { path: absolute }],
            Vec::new(),
            None,
            1,
        );
        assert_eq!(
            NewCandidateRecord::try_from_candidate(
                &valid,
                UNIX_EPOCH.checked_sub(Duration::from_nanos(1)).unwrap()
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );
        let normalized = NewCandidateRecord::try_from_candidate(
            &valid,
            UNIX_EPOCH + Duration::new(1, 123_456_789),
        )
        .unwrap();
        assert_eq!(
            normalized.created_at(),
            UNIX_EPOCH + Duration::from_millis(1_123)
        );
    }

    #[test]
    fn legacy_summaries_are_explicit_and_reject_v2_child_pollution() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let store = StoreCoordinator::open(&database).unwrap();
        start_and_finish_scan(&store, &root, "scan:legacy-candidate");
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO candidates (
                         candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                         estimated_bytes, created_at_unix_ms, status, record_format_version
                     ) VALUES (
                         'candidate:legacy-summary', 'scan:legacy-candidate',
                         'fixture.candidate.legacy', 3, 'review_required',
                         42, 1234, 'dismissed', 1
                     )",
                    [],
                )
                .unwrap();
        });
        let id = CandidateId::new("candidate:legacy-summary").unwrap();
        let stored = store.load_candidate(&id).unwrap().unwrap();
        let StoredCandidateRecord::LegacySummary(summary) = stored else {
            panic!("legacy candidate was presented as complete");
        };
        assert_eq!(summary.id, id);
        assert_eq!(summary.source_scan_id.as_str(), "scan:legacy-candidate");
        assert_eq!(summary.rule.id().as_str(), "fixture.candidate.legacy");
        assert_eq!(summary.rule.revision().get(), 3);
        assert_eq!(summary.safety, SafetyTier::ReviewRequired);
        assert_eq!(summary.estimated_bytes, 42);
        assert_eq!(summary.created_at, UNIX_EPOCH + Duration::from_millis(1234));
        assert_eq!(summary.status, CandidateHistoryStatus::Dismissed);

        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO candidate_paths (
                         candidate_id, path_ordinal, observed_path, observed_path_encoding
                     ) VALUES ('candidate:legacy-summary', 0, ?1, 1)",
                    [b"/polluted".as_slice()],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_candidate(&id).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn malformed_ordinals_and_eviction_evidence_fail_closed_on_load() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let first = root.join("candidate-fixture-a");
        let second = root.join("candidate-fixture-b");
        let store = StoreCoordinator::open(&database).unwrap();
        start_and_finish_scan(&store, &root, "scan:gapped");
        let regular_policy = rule(
            "fixture.candidate.gapped",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let regular = candidate(
            "candidate:gapped",
            "scan:gapped",
            &regular_policy,
            vec![first.clone(), second.clone()],
            vec![Evidence::MatchedPath {
                path: first.clone(),
            }],
            Vec::new(),
            None,
            2,
        );
        store
            .record_candidate_discovered(
                &NewCandidateRecord::try_from_candidate(
                    &regular,
                    UNIX_EPOCH + Duration::from_secs(1),
                )
                .unwrap(),
            )
            .unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM candidate_paths
                     WHERE candidate_id = 'candidate:gapped' AND path_ordinal = 0",
                    [],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_candidate(regular.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );

        start_and_finish_scan(&store, &root, "scan:eviction");
        let eviction_policy = rule(
            "fixture.candidate.eviction",
            CandidateCategory::CloudFile,
            SafetyTier::SafeEvictable,
            CandidateAction::EvictLocalCopy,
            false,
        );
        let eviction = candidate(
            "candidate:eviction",
            "scan:eviction",
            &eviction_policy,
            vec![first.clone()],
            vec![Evidence::CloudUploadComplete {
                path: first.clone(),
            }],
            Vec::new(),
            None,
            1,
        );
        store
            .record_candidate_discovered(
                &NewCandidateRecord::try_from_candidate(
                    &eviction,
                    UNIX_EPOCH + Duration::from_secs(1),
                )
                .unwrap(),
            )
            .unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE candidate_evidence
                     SET path_value = ?2
                     WHERE candidate_id = ?1 AND evidence_ordinal = 0",
                    params![eviction.id().as_str(), b"/unrelated-cloud-path".as_slice()],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_candidate(eviction.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn oversized_child_blob_is_rejected_before_path_materialization() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let path = root.join("candidate-fixture");
        let store = StoreCoordinator::open(&database).unwrap();
        start_and_finish_scan(&store, &root, "scan:oversized-candidate");
        let policy = rule(
            "fixture.candidate.oversized",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let candidate = candidate(
            "candidate:oversized",
            "scan:oversized-candidate",
            &policy,
            vec![path.clone()],
            vec![Evidence::MatchedPath { path }],
            Vec::new(),
            None,
            1,
        );
        store
            .record_candidate_discovered(
                &NewCandidateRecord::try_from_candidate(
                    &candidate,
                    UNIX_EPOCH + Duration::from_secs(1),
                )
                .unwrap(),
            )
            .unwrap();
        store.with_connection(|connection| {
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "UPDATE candidate_paths SET observed_path = zeroblob(65537)
                     WHERE candidate_id = 'candidate:oversized'",
                    [],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            store.load_candidate(candidate.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
        let encoded_path = encode_host_path(&candidate.paths()[0]).unwrap();
        store.with_connection(|connection| {
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "UPDATE candidate_paths
                     SET observed_path = ?2, observed_path_encoding = ?3
                     WHERE candidate_id = ?1",
                    params![
                        candidate.id().as_str(),
                        encoded_path.bytes,
                        encoded_path.encoding as i64
                    ],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE candidate_evidence
                     SET text_value = printf('%.*c', 4097, 'x')
                     WHERE candidate_id = ?1 AND evidence_ordinal = 0",
                    [candidate.id().as_str()],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            store.load_candidate(candidate.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn maximum_legal_child_counts_fit_the_bounded_reader() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let paths = (0..MAX_PATHS)
            .map(|index| root.join(format!("candidate-fixture-{index}")))
            .collect::<Vec<_>>();
        let evidence = (0..MAX_EVIDENCE)
            .map(|index| Evidence::MinimumSize {
                observed_bytes: index as u64 + 1,
                minimum_bytes: index as u64,
            })
            .collect::<Vec<_>>();
        let blockers = (0..MAX_BLOCKERS)
            .map(|index| ALL_BLOCKERS[index % ALL_BLOCKERS.len()].clone())
            .collect::<Vec<_>>();
        let store = StoreCoordinator::open(&database).unwrap();
        start_and_finish_scan(&store, &root, "scan:max-candidate");
        let policy = rule(
            "fixture.candidate.maximum",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let candidate = candidate(
            "candidate:maximum",
            "scan:max-candidate",
            &policy,
            paths,
            evidence,
            blockers,
            None,
            1,
        );
        store
            .record_candidate_discovered(
                &NewCandidateRecord::try_from_candidate(
                    &candidate,
                    UNIX_EPOCH + Duration::from_secs(1),
                )
                .unwrap(),
            )
            .unwrap();
        let StoredCandidateRecord::Complete(stored) =
            store.load_candidate(candidate.id()).unwrap().unwrap()
        else {
            panic!("maximum format-2 record became incomplete");
        };
        assert_eq!(stored.paths.len(), MAX_PATHS);
        assert_eq!(stored.evidence.len(), MAX_EVIDENCE);
        assert_eq!(stored.blockers.len(), MAX_BLOCKERS);

        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO candidate_evidence (
                         candidate_id, evidence_ordinal, evidence_kind,
                         observed_bytes, minimum_bytes
                     ) VALUES ('candidate:maximum', 512, 'minimum_size', 1, 1)",
                    [],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_candidate(candidate.id()).unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn external_schema_upgrade_blocks_candidate_reads_and_writes() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let root = temp.path().join("root");
        let path = root.join("candidate-fixture");
        let store = StoreCoordinator::open(&database).unwrap();
        start_and_finish_scan(&store, &root, "scan:blocked-candidate");
        let policy = rule(
            "fixture.candidate.blocked",
            CandidateCategory::ApplicationCache,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            false,
        );
        let candidate = candidate(
            "candidate:blocked",
            "scan:blocked-candidate",
            &policy,
            vec![path.clone()],
            vec![Evidence::MatchedPath { path }],
            Vec::new(),
            None,
            1,
        );
        let candidate =
            NewCandidateRecord::try_from_candidate(&candidate, UNIX_EPOCH + Duration::from_secs(1))
                .unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO schema_migrations (
                         version, name, checksum_sha256, applied_at_unix_ms
                     ) VALUES (?1, 'future-candidate-schema', zeroblob(32), 2)",
                    [i64::from(crate::DATABASE_SCHEMA_VERSION + 1)],
                )
                .unwrap();
            connection
                .pragma_update(None, "user_version", crate::DATABASE_SCHEMA_VERSION + 1)
                .unwrap();
        });
        assert_eq!(
            store
                .record_candidate_discovered(&candidate)
                .unwrap_err()
                .kind,
            HistoryErrorKind::IncompatibleSchema
        );
        assert_eq!(
            store
                .transition_candidate_evaluation_status(
                    &CandidateId::new("candidate:future-evaluator").unwrap(),
                    CandidateEvaluationTransition::DiscoveredToStale,
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::IncompatibleSchema
        );
        assert_eq!(
            store
                .load_candidate(candidate.candidate().id())
                .unwrap_err()
                .kind,
            HistoryErrorKind::IncompatibleSchema
        );
        assert_eq!(
            store
                .transition_candidate_review_status(
                    &CandidateId::new("candidate:future-review").unwrap(),
                    CandidateReviewTransition::Select,
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::IncompatibleSchema
        );
        assert!(
            !format!(
                "{:?}",
                store.record_candidate_discovered(&candidate).unwrap_err()
            )
            .contains(database.to_str().unwrap())
        );
    }
}
#[test]
fn aggregate_candidate_batch_budget_has_an_exact_boundary() {
    let fixed = MATERIALIZED_CANDIDATE_CHARGE;
    let remaining = MAX_CANDIDATE_BATCH_MATERIALIZED_BYTES - fixed;
    assert_eq!(remaining % MATERIALIZED_PAYLOAD_MULTIPLIER, 0);
    let at_limit = CandidateBatchUsage {
        candidates: 1,
        path_payload_bytes: remaining / MATERIALIZED_PAYLOAD_MULTIPLIER,
        ..CandidateBatchUsage::default()
    };
    assert_eq!(
        at_limit
            .materialized_bytes(HistoryErrorKind::QueryLimitExceeded)
            .unwrap(),
        MAX_CANDIDATE_BATCH_MATERIALIZED_BYTES
    );
    at_limit
        .ensure_within_budget(HistoryErrorKind::QueryLimitExceeded)
        .unwrap();

    let one_over = CandidateBatchUsage {
        path_payload_bytes: at_limit.path_payload_bytes + 1,
        ..at_limit
    };
    assert_eq!(
        one_over
            .ensure_within_budget(HistoryErrorKind::QueryLimitExceeded)
            .unwrap_err()
            .kind,
        HistoryErrorKind::QueryLimitExceeded
    );
    assert_eq!(
        one_over
            .ensure_within_budget(HistoryErrorKind::InvalidInput)
            .unwrap_err()
            .kind,
        HistoryErrorKind::InvalidInput
    );

    let tiny_children_overhead = CandidateBatchUsage {
        paths: MAX_CANDIDATE_BATCH_MATERIALIZED_BYTES / MATERIALIZED_PATH_CHARGE + 1,
        ..CandidateBatchUsage::default()
    };
    assert_eq!(
        tiny_children_overhead
            .ensure_within_budget(HistoryErrorKind::QueryLimitExceeded)
            .unwrap_err()
            .kind,
        HistoryErrorKind::QueryLimitExceeded
    );
}

#[test]
fn batch_preflight_rejects_large_blob_in_integer_field_before_child_loading() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE candidates (
                     candidate_id TEXT, scan_id TEXT, rule_id TEXT,
                     rule_revision INTEGER, safety_tier TEXT,
                     estimated_bytes INTEGER, created_at_unix_ms INTEGER,
                     status TEXT, record_format_version INTEGER,
                     category TEXT, proposed_action TEXT,
                     rule_schedule_eligible INTEGER,
                     newest_mtime_unix_seconds INTEGER,
                     newest_mtime_nanoseconds INTEGER
                 );
                 CREATE INDEX candidates_by_scan_time
                   ON candidates(scan_id, created_at_unix_ms DESC, candidate_id);
                 CREATE TABLE candidate_paths (
                     candidate_id TEXT, path_ordinal,
                     observed_path BLOB, observed_path_encoding INTEGER
                 );
                 CREATE TABLE candidate_evidence (
                     candidate_id TEXT, evidence_ordinal,
                     evidence_kind TEXT, path_value BLOB,
                     path_value_encoding, text_value TEXT,
                     observed_unix_seconds, observed_nanoseconds,
                     duration_seconds, duration_nanoseconds,
                     observed_bytes, minimum_bytes
                 );
                 CREATE TABLE candidate_blockers (
                     candidate_id TEXT, blocker_ordinal, blocker_kind TEXT
                 );
                 CREATE TABLE candidate_plan_claims (
                     candidate_id TEXT, prior_review_status TEXT,
                     session_id TEXT, item_ordinal INTEGER
                 );
                 CREATE TABLE cleanup_sessions (
                     session_id TEXT, candidate_status_coupling_version,
                     status TEXT, record_format_version
                 );
                 CREATE TABLE cleanup_items (
                     session_id TEXT, item_ordinal INTEGER,
                     candidate_id TEXT, record_format_version
                 );
                 INSERT INTO candidates (
                     candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                     estimated_bytes, created_at_unix_ms, status,
                     record_format_version, category, proposed_action,
                     rule_schedule_eligible
                 ) VALUES (
                     'candidate:hostile-scalar', 'scan:hostile-scalar',
                     'fixture.hostile-scalar', 1, 'informational', 1, 1,
                     'discovered', 2, 'developer_artifact', 'reveal_only', 0
                 );
                 INSERT INTO candidate_paths (
                     candidate_id, path_ordinal, observed_path,
                     observed_path_encoding
                 ) VALUES (
                     'candidate:hostile-scalar', zeroblob(16777216), x'2F6F6B', 1
                 );",
        )
        .unwrap();

    assert_eq!(
        ensure_stored_candidate_batch_budget(
            &connection,
            &ScanId::new("scan:hostile-scalar").unwrap(),
            1,
        )
        .unwrap_err()
        .kind,
        HistoryErrorKind::CorruptData
    );
}

#[cfg(test)]
fn exact_candidate_preflight_connection() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE candidates (
                 candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                 estimated_bytes, created_at_unix_ms, status,
                 record_format_version, category, proposed_action,
                 rule_schedule_eligible, newest_mtime_unix_seconds,
                 newest_mtime_nanoseconds
             );
             CREATE TABLE candidate_paths (
                 candidate_id, path_ordinal, observed_path,
                 observed_path_encoding
             );
             CREATE TABLE candidate_evidence (
                 candidate_id, evidence_ordinal, evidence_kind,
                 path_value, path_value_encoding, text_value,
                 observed_unix_seconds, observed_nanoseconds,
                 duration_seconds, duration_nanoseconds,
                 observed_bytes, minimum_bytes
             );
             CREATE TABLE candidate_blockers (
                 candidate_id, blocker_ordinal, blocker_kind
             );
             CREATE TABLE candidate_plan_claims (
                 candidate_id, prior_review_status, session_id, item_ordinal
             );
             CREATE TABLE cleanup_sessions (
                 session_id, candidate_status_coupling_version, status,
                 record_format_version
             );
             CREATE TABLE cleanup_items (
                 session_id, item_ordinal, candidate_id, record_format_version
             );",
        )
        .unwrap();
    connection
}

#[cfg(test)]
fn insert_exact_candidate_preflight_parent(connection: &Connection, id: &CandidateId) {
    connection
        .execute(
            "INSERT INTO candidates (
                 candidate_id, scan_id, rule_id, rule_revision, safety_tier,
                 estimated_bytes, created_at_unix_ms, status,
                 record_format_version, category, proposed_action,
                 rule_schedule_eligible
             ) VALUES (
                 ?1, 'scan:exact-budget', 'fixture.exact-budget', 1,
                 'informational', 1, 1, 'discovered', 2,
                 'developer_artifact', 'reveal_only', 0
             )",
            [id.as_str()],
        )
        .unwrap();
}

#[test]
fn exact_candidate_preflight_has_an_exact_aggregate_byte_boundary() {
    let mut connection = exact_candidate_preflight_connection();
    let id = CandidateId::new("candidate:exact-budget").unwrap();
    insert_exact_candidate_preflight_parent(&connection, &id);
    connection
        .execute(
            "INSERT INTO candidate_paths (
                 candidate_id, path_ordinal, observed_path,
                 observed_path_encoding
             ) VALUES (?1, 0, x'2F', 1)",
            [id.as_str()],
        )
        .unwrap();

    let (evidence_count, evidence_payload_bytes) = (1..=MAX_EVIDENCE)
        .find_map(|evidence_count| {
            let fixed = CandidateBatchUsage {
                candidates: 1,
                paths: 1,
                path_payload_bytes: 1,
                evidence: evidence_count as u64,
                ..CandidateBatchUsage::default()
            }
            .materialized_bytes(HistoryErrorKind::InternalState)
            .unwrap();
            let remaining_charge = MAX_CANDIDATE_BATCH_MATERIALIZED_BYTES.checked_sub(fixed)?;
            if remaining_charge % MATERIALIZED_PAYLOAD_MULTIPLIER != 0 {
                return None;
            }
            let payload_bytes = remaining_charge / MATERIALIZED_PAYLOAD_MULTIPLIER;
            (payload_bytes >= evidence_count as u64
                && payload_bytes <= evidence_count as u64 * MAX_STORED_PATH_BYTES as u64)
                .then_some((evidence_count, payload_bytes))
        })
        .expect("the legal evidence cardinality must admit the exact byte boundary");

    let transaction = connection.transaction().unwrap();
    let mut remaining = evidence_payload_bytes;
    let mut last_length = 0_u64;
    for ordinal in 0..evidence_count {
        let remaining_rows = (evidence_count - ordinal - 1) as u64;
        let length = (remaining - remaining_rows).min(MAX_STORED_PATH_BYTES as u64);
        assert!(length > 0);
        transaction
            .execute(
                "INSERT INTO candidate_evidence (
                     candidate_id, evidence_ordinal, evidence_kind,
                     path_value, path_value_encoding
                 ) VALUES (?1, ?2, 'matched_path', zeroblob(?3), 1)",
                params![id.as_str(), ordinal as i64, length as i64],
            )
            .unwrap();
        remaining -= length;
        last_length = length;
    }
    assert_eq!(remaining, 0);
    transaction.commit().unwrap();

    ensure_stored_candidate_budget(&connection, &id).unwrap();
    assert!(last_length < MAX_STORED_PATH_BYTES as u64);
    connection
        .execute(
            "UPDATE candidate_evidence
             SET path_value = zeroblob(?2)
             WHERE candidate_id = ?1 AND evidence_ordinal = ?3",
            params![
                id.as_str(),
                (last_length + 1) as i64,
                (evidence_count - 1) as i64
            ],
        )
        .unwrap();
    assert_eq!(
        ensure_stored_candidate_budget(&connection, &id)
            .unwrap_err()
            .kind,
        HistoryErrorKind::QueryLimitExceeded
    );
}

#[test]
fn exact_candidate_preflight_rejects_large_blob_in_child_scalar_before_payload_loading() {
    let connection = exact_candidate_preflight_connection();
    let id = CandidateId::new("candidate:exact-hostile-scalar").unwrap();
    insert_exact_candidate_preflight_parent(&connection, &id);
    connection
        .execute(
            "INSERT INTO candidate_paths (
                 candidate_id, path_ordinal, observed_path,
                 observed_path_encoding
             ) VALUES (?1, zeroblob(16777216), x'2F6F6B', 1)",
            [id.as_str()],
        )
        .unwrap();

    assert_eq!(
        ensure_stored_candidate_budget(&connection, &id)
            .unwrap_err()
            .kind,
        HistoryErrorKind::CorruptData
    );
}

#[test]
fn exact_candidate_preflight_rejects_large_blob_in_parent_scalar_before_child_loading() {
    let connection = exact_candidate_preflight_connection();
    let id = CandidateId::new("candidate:exact-hostile-parent").unwrap();
    insert_exact_candidate_preflight_parent(&connection, &id);
    connection
        .execute(
            "UPDATE candidates SET rule_revision = zeroblob(16777216)
             WHERE candidate_id = ?1",
            [id.as_str()],
        )
        .unwrap();

    assert_eq!(
        ensure_stored_candidate_budget(&connection, &id)
            .unwrap_err()
            .kind,
        HistoryErrorKind::CorruptData
    );
}
