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
    HistoryError, HistoryErrorKind, ScanStatus, from_i64, load_scan_record,
    load_scan_record_within_budget, map_query_sql_error, map_write_sql_error, run_bounded_query,
    stored_bool, system_time_to_unix_ms, to_i64, unix_ms_to_system_time,
};

mod loading;

pub(super) use loading::{
    RawEvidence, action_as_stored, action_from_stored, category_as_stored, category_from_stored,
    decode_absolute_path, decode_evidence, decode_optional_time, load_candidate_record,
    load_candidate_record_within_budget, load_complete_candidate_batch_within_budget,
    safety_as_stored, safety_from_stored, time_parts, validate_complete_children,
    validate_null_value, validate_optional_value, validate_policy, validate_required_value,
    validate_text,
};
use loading::{
    blocker_as_stored, corrupt, duration_parts, evidence_kind_as_stored, prepare_absolute_path,
    validate_candidate,
};
#[cfg(test)]
use loading::{
    blocker_from_stored, ensure_stored_candidate_batch_budget, ensure_stored_candidate_budget,
    load_candidate_record_within_budget_and_hook,
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
        scalar_evidence_count: usize,
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
                    .and_then(|count| count.checked_add(u64::try_from(scalar_evidence_count).ok()?))
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
    pub(super) fn immutable_body_matches(&self, other: &Self) -> bool {
        self.id == other.id
            && self.source_scan_id == other.source_scan_id
            && self.rule == other.rule
            && self.category == other.category
            && self.paths == other.paths
            && self.estimated_bytes == other.estimated_bytes
            && self.newest_mtime == other.newest_mtime
            && self.evidence == other.evidence
            && self.safety == other.safety
            && self.action == other.action
            && self.rule_schedule_eligible == other.rule_schedule_eligible
            && self.blockers == other.blockers
            && self.created_at == other.created_at
    }

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

/// Claim the one allowlisted deterministic rule whose discovery blocker is
/// intentionally retained until the trusted planner boundary. The blocker is
/// a safety fact, not a user-review failure: only the private Rust-target
/// coupling may move this exact shape into `planned`.
pub(super) fn mark_trusted_rust_target_candidate_planned(
    transaction: &Transaction<'_>,
    candidate: &CompleteCandidateRecord,
) -> Result<CandidatePriorReviewStatus, HistoryError> {
    if candidate.rule.id().as_str() != "developer.rust.target"
        || candidate.rule.revision().get() != crate::domain::SAFE_RUST_RULE_REVISION
        || candidate.category != CandidateCategory::DeveloperArtifact
        || candidate.safety != SafetyTier::SafeRegenerable
        || candidate.action != CandidateAction::RemoveKnownRegenerableContents
        || candidate.rule_schedule_eligible
        || candidate.blockers != [BlockReason::ProtectedPath]
    {
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
const fn invalid() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidInput)
}

#[cfg(test)]
#[path = "candidate_history/tests.rs"]
mod tests;
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
