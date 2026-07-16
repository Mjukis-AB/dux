//! Typed, non-authoritative candidate history stored in SQLite schema v2.
//!
//! Complete records preserve deterministic discovery facts for presentation and
//! comparison. They are not current filesystem evidence, planner input, or an
//! executable capability. Migrated v1 summaries remain explicitly incomplete.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::domain::{
    BlockReason, Candidate, CandidateAction, CandidateCategory, CandidateId, Evidence, RuleId,
    RuleRef, RuleRevision, SafetyTier, ScanId,
};

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
};

const MAX_PATHS: usize = 256;
const MAX_EVIDENCE: usize = 512;
const MAX_BLOCKERS: usize = 64;
const MAX_TEXT_BYTES: usize = 4_096;
const MAX_STORED_ID_BYTES: i64 = 128;
const MAX_STORED_PATH_BYTES: i64 = 65_536;
const MAX_STORED_POLICY_BYTES: i64 = 64;
const MAX_STORED_TEXT_BYTES: i64 = MAX_TEXT_BYTES as i64;

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
    fn from_stored(value: &str) -> Result<Self, HistoryError> {
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
}

struct PreparedEvidence {
    kind: &'static str,
    path: Option<EncodedBytes>,
    text: Option<String>,
    observed_time: Option<TimeParts>,
    duration: Option<TimeParts>,
    observed_bytes: Option<i64>,
    minimum_bytes: Option<i64>,
}

impl PreparedEvidence {
    fn prepare(evidence: &Evidence) -> Result<Self, HistoryError> {
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

#[derive(Clone, Copy)]
struct TimeParts {
    seconds: i64,
    nanoseconds: i64,
}

pub(super) fn insert_candidate(
    transaction: &Transaction<'_>,
    candidate: &PreparedCandidate,
) -> Result<(), HistoryError> {
    let scan_exists: Option<i64> = transaction
        .query_row(
            "SELECT 1 FROM scans WHERE scan_id = ?1",
            [&candidate.source_scan_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    if scan_exists.is_none() {
        return Err(HistoryError::new(HistoryErrorKind::NotFound));
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

pub(super) fn load_candidate_record(
    connection: &Connection,
    id: &CandidateId,
) -> Result<Option<StoredCandidateRecord>, HistoryError> {
    run_bounded_query(connection, || {
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
        ensure_source_scan_exists(connection, &common.source_scan_id)?;
        match raw.record_format_version {
            1 => Ok(Some(StoredCandidateRecord::LegacySummary({
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
    })
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

struct RawEvidence {
    kind: String,
    path: Option<Vec<u8>>,
    path_encoding: Option<i64>,
    text: Option<String>,
    observed_seconds: Option<i64>,
    observed_nanoseconds: Option<i64>,
    duration_seconds: Option<i64>,
    duration_nanoseconds: Option<i64>,
    observed_bytes: Option<i64>,
    minimum_bytes: Option<i64>,
}

fn decode_evidence(raw: RawEvidence) -> Result<Evidence, HistoryError> {
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

fn validate_complete_children(
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

fn validate_policy(
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

fn validate_required_value(
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

fn validate_optional_value(
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

fn validate_null_value(
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

fn decode_absolute_path(bytes: Vec<u8>, encoding: i64) -> Result<PathBuf, HistoryError> {
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

fn validate_text(value: &str, kind: HistoryErrorKind) -> Result<(), HistoryError> {
    if value.is_empty() || value.len() > MAX_TEXT_BYTES || value.chars().any(char::is_control) {
        return Err(HistoryError::new(kind));
    }
    Ok(())
}

fn time_parts(value: SystemTime, kind: HistoryErrorKind) -> Result<TimeParts, HistoryError> {
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

fn decode_optional_time(
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

fn to_i64(value: u64, kind: HistoryErrorKind) -> Result<i64, HistoryError> {
    i64::try_from(value).map_err(|_| HistoryError::new(kind))
}

fn from_i64(value: i64) -> Result<u64, HistoryError> {
    u64::try_from(value).map_err(|_| corrupt())
}

fn stored_bool(value: i64) -> Result<bool, HistoryError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(corrupt()),
    }
}

fn category_as_stored(value: CandidateCategory) -> &'static str {
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

fn category_from_stored(value: &str) -> Result<CandidateCategory, HistoryError> {
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

fn safety_as_stored(value: SafetyTier) -> &'static str {
    match value {
        SafetyTier::SafeRegenerable => "safe_regenerable",
        SafetyTier::SafeEvictable => "safe_evictable",
        SafetyTier::ReviewRequired => "review_required",
        SafetyTier::Informational => "informational",
        SafetyTier::Protected => "protected",
    }
}

fn safety_from_stored(value: &str) -> Result<SafetyTier, HistoryError> {
    match value {
        "safe_regenerable" => Ok(SafetyTier::SafeRegenerable),
        "safe_evictable" => Ok(SafetyTier::SafeEvictable),
        "review_required" => Ok(SafetyTier::ReviewRequired),
        "informational" => Ok(SafetyTier::Informational),
        "protected" => Ok(SafetyTier::Protected),
        _ => Err(corrupt()),
    }
}

fn action_as_stored(value: CandidateAction) -> &'static str {
    match value {
        CandidateAction::RemoveKnownRegenerableContents => "remove_known_regenerable_contents",
        CandidateAction::EvictLocalCopy => "evict_local_copy",
        CandidateAction::MoveToTrash => "move_to_trash",
        CandidateAction::RevealOnly => "reveal_only",
        CandidateAction::NoAction => "no_action",
    }
}

fn action_from_stored(value: &str) -> Result<CandidateAction, HistoryError> {
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
                .load_candidate(candidate.candidate().id())
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
