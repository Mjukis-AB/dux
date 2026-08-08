//! Durable, expiring cross-process leases for snapshot-backed review.
//!
//! These rows are mutable coordination state, not history and not cleanup
//! authority. A candidate selection or cleanup plan never implies a live pin.

#![allow(
    dead_code,
    reason = "sealed review leases are wired to Explorer/FFI in a later milestone slice"
)]

use std::collections::BTreeMap;
use std::path::{Component, Path};
use std::time::SystemTime;

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::domain::ScanId;

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    run_bounded_snapshot_pin_query, system_time_to_unix_ms,
};
use super::process_liveness::{ProcessIdentityError, ProcessInstanceId};
use super::snapshot::{SnapshotFileName, SnapshotReference};

const RECORD_FORMAT_VERSION: i64 = 1;
const LEASE_DURATION_MS: i64 = 10 * 60 * 1_000;
pub(super) const MAX_ACTIVE_PINS: usize = 1_024;
pub(super) const MAX_ACTIVE_PINS_PER_OWNER: usize = 64;
pub(super) const MAX_EXPIRED_PRUNE: usize = 64;
const MAX_ID_BYTES: i64 = 128;
const MAX_STATUS_BYTES: i64 = 16;
const MAX_PATH_BYTES: i64 = 65_536;
const MAX_OWNER_BYTES: i64 = 128;
const MAX_PURPOSE_BYTES: i64 = 32;
pub(super) const SNAPSHOT_REVIEW_PIN_ID_ATTEMPTS: usize = 16;

/// Read-only, point-in-time protection facts for one exact snapshot.
///
/// These counts are observations only. They deliberately carry no retained
/// file handle and cannot authorize a retention transition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SnapshotReviewPinSummary {
    pub(crate) active: u32,
    pub(crate) active_explorer: u32,
    pub(crate) active_cleanup_review: u32,
    pub(crate) expired: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SnapshotReviewPinPopulation {
    pub(super) by_scan: BTreeMap<ScanId, SnapshotReviewPinSummary>,
    pub(super) active: u32,
    pub(super) expired: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotReviewPurpose {
    Explorer,
    CleanupReview,
}

impl SnapshotReviewPurpose {
    const fn as_stored(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::CleanupReview => "cleanup_review",
        }
    }

    fn from_stored(value: &str) -> Result<Self, HistoryError> {
        match value {
            "explorer" => Ok(Self::Explorer),
            "cleanup_review" => Ok(Self::CleanupReview),
            _ => Err(corrupt()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SnapshotReviewPinId(String);

impl SnapshotReviewPinId {
    pub(super) fn random() -> Result<Self, HistoryError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?;
        let mut encoded = String::with_capacity(32);
        push_lower_hex(&mut encoded, &random);
        Ok(Self(encoded))
    }

    pub(super) fn from_stored(value: String) -> Result<Self, HistoryError> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(corrupt());
        }
        Ok(Self(value))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone)]
pub(super) struct PreparedSnapshotReviewPin {
    id: SnapshotReviewPinId,
    scan_id: String,
    scan_status: &'static str,
    completed_at_unix_ms: i64,
    snapshot_version: i64,
    snapshot_relative_path: Vec<u8>,
    snapshot_relative_path_encoding: i64,
    snapshot_checksum_sha256: [u8; 32],
    owner: ProcessInstanceId,
    purpose: SnapshotReviewPurpose,
    created_at_unix_ms: i64,
    renewed_at_unix_ms: i64,
    expires_at_unix_ms: i64,
}

impl PreparedSnapshotReviewPin {
    pub(super) fn prepare(
        id: SnapshotReviewPinId,
        reference: &SnapshotReference,
        completed_at: SystemTime,
        owner: ProcessInstanceId,
        purpose: SnapshotReviewPurpose,
        observed_at: SystemTime,
    ) -> Result<Self, HistoryError> {
        let completed_at_unix_ms =
            system_time_to_unix_ms(completed_at, HistoryErrorKind::InvalidInput)?;
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        if observed_at_unix_ms < completed_at_unix_ms {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let expires_at_unix_ms = observed_at_unix_ms
            .checked_add(LEASE_DURATION_MS)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        let path = encode_host_path(Path::new(reference.file_name().as_str()))
            .map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
        Ok(Self {
            id,
            scan_id: reference.scan_id().as_str().to_owned(),
            scan_status: "succeeded",
            completed_at_unix_ms,
            snapshot_version: i64::from(reference.version()),
            snapshot_relative_path: path.bytes,
            snapshot_relative_path_encoding: path.encoding as i64,
            snapshot_checksum_sha256: reference.digest().bytes(),
            owner,
            purpose,
            created_at_unix_ms: observed_at_unix_ms,
            renewed_at_unix_ms: observed_at_unix_ms,
            expires_at_unix_ms,
        })
    }

    pub(super) fn id(&self) -> &SnapshotReviewPinId {
        &self.id
    }

    pub(super) fn expires_at_unix_ms(&self) -> i64 {
        self.expires_at_unix_ms
    }

    pub(super) const fn purpose(&self) -> SnapshotReviewPurpose {
        self.purpose
    }

    #[cfg(test)]
    pub(super) fn owner(&self) -> &ProcessInstanceId {
        &self.owner
    }

    #[cfg(test)]
    pub(super) const fn created_at_unix_ms(&self) -> i64 {
        self.created_at_unix_ms
    }

    #[cfg(test)]
    pub(super) const fn renewed_at_unix_ms(&self) -> i64 {
        self.renewed_at_unix_ms
    }
}

#[derive(Clone)]
pub(super) struct StoredSnapshotReviewPin {
    id: SnapshotReviewPinId,
    record_format_version: i64,
    scan_id: String,
    scan_status: String,
    completed_at_unix_ms: i64,
    snapshot_version: i64,
    snapshot_relative_path: Vec<u8>,
    snapshot_relative_path_encoding: i64,
    snapshot_checksum_sha256: [u8; 32],
    owner: ProcessInstanceId,
    purpose: SnapshotReviewPurpose,
    created_at_unix_ms: i64,
    renewed_at_unix_ms: i64,
    expires_at_unix_ms: i64,
    parent_matches: bool,
    tombstone_present: bool,
    tombstone_exact: bool,
}

impl StoredSnapshotReviewPin {
    fn exactly_matches(&self, expected: &PreparedSnapshotReviewPin) -> bool {
        self.id == expected.id
            && self.record_format_version == RECORD_FORMAT_VERSION
            && self.scan_id == expected.scan_id
            && self.scan_status == expected.scan_status
            && self.completed_at_unix_ms == expected.completed_at_unix_ms
            && self.snapshot_version == expected.snapshot_version
            && self.snapshot_relative_path == expected.snapshot_relative_path
            && self.snapshot_relative_path_encoding == expected.snapshot_relative_path_encoding
            && self.snapshot_checksum_sha256 == expected.snapshot_checksum_sha256
            && self.owner == expected.owner
            && self.purpose == expected.purpose
            && self.created_at_unix_ms == expected.created_at_unix_ms
            && self.renewed_at_unix_ms == expected.renewed_at_unix_ms
            && self.expires_at_unix_ms == expected.expires_at_unix_ms
            && self.parent_matches
    }

    fn validate_semantics(&self) -> Result<(), HistoryError> {
        if self.record_format_version != RECORD_FORMAT_VERSION
            || self.scan_status != "succeeded"
            || !self.parent_matches
            || self.tombstone_present != self.tombstone_exact
            || self.completed_at_unix_ms < 0
            || self.created_at_unix_ms < 0
            || self.created_at_unix_ms < self.completed_at_unix_ms
            || self.created_at_unix_ms > self.renewed_at_unix_ms
            || self.renewed_at_unix_ms >= self.expires_at_unix_ms
            || self.expires_at_unix_ms - self.renewed_at_unix_ms != LEASE_DURATION_MS
        {
            return Err(corrupt());
        }
        let snapshot_version = u32::try_from(self.snapshot_version)
            .ok()
            .filter(|version| *version > 0)
            .ok_or_else(corrupt)?;
        let scan_id = ScanId::new(self.scan_id.clone()).map_err(|_| corrupt())?;
        let encoding = StoredEncoding::host_path_from_stored(self.snapshot_relative_path_encoding)
            .map_err(|_| corrupt())?;
        let encoded = EncodedBytes {
            encoding,
            bytes: self.snapshot_relative_path.clone(),
        };
        let path = decode_host_path(&encoded).map_err(|_| corrupt())?;
        if encode_host_path(&path).map_err(|_| corrupt())? != encoded {
            return Err(corrupt());
        }
        let mut components = path.components();
        let Some(Component::Normal(file_name)) = components.next() else {
            return Err(corrupt());
        };
        if components.next().is_some() {
            return Err(corrupt());
        }
        let file_name = file_name.to_str().ok_or_else(corrupt)?;
        let parsed = SnapshotFileName::parse(file_name).map_err(|_| corrupt())?;
        SnapshotReference::from_stored(
            &scan_id,
            snapshot_version,
            parsed.as_str(),
            self.snapshot_checksum_sha256,
        )
        .map_err(|_| corrupt())?;
        if path.as_os_str() != Path::new(parsed.as_str()).as_os_str() {
            return Err(corrupt());
        }
        Ok(())
    }
}

pub(super) fn insert_snapshot_review_pin_candidates(
    transaction: &Transaction<'_>,
    pins: &[PreparedSnapshotReviewPin],
) -> Result<Option<PreparedSnapshotReviewPin>, HistoryError> {
    let Some(first) = pins.first() else {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    };
    if pins.len() > SNAPSHOT_REVIEW_PIN_ID_ATTEMPTS
        || pins.iter().any(|pin| {
            pin.scan_id != first.scan_id
                || pin.scan_status != first.scan_status
                || pin.completed_at_unix_ms != first.completed_at_unix_ms
                || pin.snapshot_version != first.snapshot_version
                || pin.snapshot_relative_path != first.snapshot_relative_path
                || pin.snapshot_relative_path_encoding != first.snapshot_relative_path_encoding
                || pin.snapshot_checksum_sha256 != first.snapshot_checksum_sha256
                || pin.owner != first.owner
                || pin.purpose != first.purpose
                || pin.created_at_unix_ms != first.created_at_unix_ms
                || pin.renewed_at_unix_ms != first.renewed_at_unix_ms
                || pin.expires_at_unix_ms != first.expires_at_unix_ms
        })
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    run_bounded_snapshot_pin_query(transaction, || {
        validate_and_prune_population(transaction, first.created_at_unix_ms, &first.owner)?;
        for pin in pins {
            if insert_snapshot_review_pin_bounded(transaction, pin)? {
                return Ok(Some(pin.clone()));
            }
        }
        Ok(None)
    })
}

fn insert_snapshot_review_pin_bounded(
    transaction: &Transaction<'_>,
    pin: &PreparedSnapshotReviewPin,
) -> Result<bool, HistoryError> {
    transaction
        .execute(
            "INSERT INTO snapshot_review_pins (
                pin_id, record_format_version, scan_id, scan_status,
                completed_at_unix_ms, snapshot_version,
                snapshot_relative_path, snapshot_relative_path_encoding,
                snapshot_checksum_sha256, owner_process_instance, purpose,
                created_at_unix_ms, renewed_at_unix_ms, expires_at_unix_ms
             ) VALUES (
                ?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12
             ) ON CONFLICT(pin_id) DO NOTHING",
            params![
                pin.id.as_str(),
                pin.scan_id,
                pin.scan_status,
                pin.completed_at_unix_ms,
                pin.snapshot_version,
                pin.snapshot_relative_path,
                pin.snapshot_relative_path_encoding,
                pin.snapshot_checksum_sha256,
                pin.owner.as_str(),
                pin.purpose.as_stored(),
                pin.created_at_unix_ms,
                pin.expires_at_unix_ms,
            ],
        )
        .map_err(map_write_sql_error)
        .and_then(|changed| match changed {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(HistoryError::new(HistoryErrorKind::InternalState)),
        })
}

pub(super) fn validate_snapshot_review_pin(
    connection: &Connection,
    expected: &PreparedSnapshotReviewPin,
    observed_at: SystemTime,
) -> Result<StoredSnapshotReviewPin, HistoryError> {
    let observed_at_unix_ms = system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
    let stored = load_snapshot_review_pin(connection, expected.id())?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    if !stored.exactly_matches(expected) {
        return Err(corrupt());
    }
    if stored.tombstone_exact {
        return Err(corrupt());
    }
    if observed_at_unix_ms >= stored.expires_at_unix_ms {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok(stored)
}

pub(super) fn renew_snapshot_review_pin(
    transaction: &Transaction<'_>,
    expected: &PreparedSnapshotReviewPin,
    observed_at: SystemTime,
) -> Result<PreparedSnapshotReviewPin, HistoryError> {
    let observed_at_unix_ms = system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
    let stored = load_snapshot_review_pin(transaction, expected.id())?
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::NotFound))?;
    if !stored.exactly_matches(expected) {
        return Err(corrupt());
    }
    if stored.tombstone_exact {
        return Err(corrupt());
    }
    if observed_at_unix_ms < stored.renewed_at_unix_ms {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    if observed_at_unix_ms >= stored.expires_at_unix_ms {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    let expires_at_unix_ms = observed_at_unix_ms
        .checked_add(LEASE_DURATION_MS)
        .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    let changed = transaction
        .execute(
            "UPDATE snapshot_review_pins
             SET renewed_at_unix_ms = ?1, expires_at_unix_ms = ?2
             WHERE pin_id = ?3
               AND owner_process_instance = ?4
               AND renewed_at_unix_ms = ?5
               AND expires_at_unix_ms = ?6",
            params![
                observed_at_unix_ms,
                expires_at_unix_ms,
                expected.id.as_str(),
                expected.owner.as_str(),
                expected.renewed_at_unix_ms,
                expected.expires_at_unix_ms,
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    let mut renewed = expected.clone();
    renewed.renewed_at_unix_ms = observed_at_unix_ms;
    renewed.expires_at_unix_ms = expires_at_unix_ms;
    Ok(renewed)
}

pub(super) fn release_snapshot_review_pin(
    transaction: &Transaction<'_>,
    expected: &PreparedSnapshotReviewPin,
) -> Result<(), HistoryError> {
    let Some(stored) = load_snapshot_review_pin(transaction, expected.id())? else {
        return Ok(());
    };
    if !stored.exactly_matches(expected) {
        return Err(corrupt());
    }
    let changed = transaction
        .execute(
            "DELETE FROM snapshot_review_pins
             WHERE pin_id = ?1
               AND owner_process_instance = ?2
               AND renewed_at_unix_ms = ?3
               AND expires_at_unix_ms = ?4",
            params![
                expected.id.as_str(),
                expected.owner.as_str(),
                expected.renewed_at_unix_ms,
                expected.expires_at_unix_ms,
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown))
    }
}

pub(super) fn load_snapshot_review_pin(
    connection: &Connection,
    id: &SnapshotReviewPinId,
) -> Result<Option<StoredSnapshotReviewPin>, HistoryError> {
    run_bounded_query(connection, || {
        let sql = format!("{PIN_SELECT_PREFIX} WHERE pin.pin_id = ?1");
        connection
            .query_row(&sql, [id.as_str()], raw_pin)
            .optional()
            .map_err(map_query_sql_error)?
            .map(decode_pin)
            .transpose()
    })
}

pub(super) fn snapshot_review_pin_exactly_matches(
    connection: &Connection,
    expected: &PreparedSnapshotReviewPin,
) -> Result<bool, HistoryError> {
    load_snapshot_review_pin(connection, expected.id()).map(|stored| {
        stored.is_some_and(|stored| stored.exactly_matches(expected) && !stored.tombstone_exact)
    })
}

pub(super) enum SnapshotReviewPinState {
    Missing,
    Exact,
    Conflicting,
}

pub(super) fn snapshot_review_pin_state(
    connection: &Connection,
    expected: &PreparedSnapshotReviewPin,
) -> Result<SnapshotReviewPinState, HistoryError> {
    load_snapshot_review_pin(connection, expected.id()).map(|stored| match stored {
        None => SnapshotReviewPinState::Missing,
        Some(stored) if stored.exactly_matches(expected) && !stored.tombstone_exact => {
            SnapshotReviewPinState::Exact
        }
        Some(_) => SnapshotReviewPinState::Conflicting,
    })
}

fn validate_and_prune_population(
    transaction: &Transaction<'_>,
    observed_at_unix_ms: i64,
    owner: &ProcessInstanceId,
) -> Result<(), HistoryError> {
    let mut statement = transaction
        .prepare(&format!(
            "{PIN_SELECT_PREFIX}
             ORDER BY pin.expires_at_unix_ms ASC, pin.pin_id ASC
             LIMIT {}",
            MAX_ACTIVE_PINS + 1
        ))
        .map_err(map_query_sql_error)?;
    let rows = statement
        .query_map([], raw_pin)
        .map_err(map_query_sql_error)?;
    let mut expired = Vec::with_capacity(MAX_EXPIRED_PRUNE);
    let mut count = 0_usize;
    let mut active_count = 0_usize;
    let mut owner_count = 0_usize;
    for row in rows {
        let pin = decode_pin(row.map_err(map_query_sql_error)?)?;
        count = count
            .checked_add(1)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
        if pin.expires_at_unix_ms <= observed_at_unix_ms {
            if expired.len() < MAX_EXPIRED_PRUNE {
                expired.push((pin.id, pin.expires_at_unix_ms));
            }
        } else {
            if pin.tombstone_exact {
                return Err(corrupt());
            }
            active_count = active_count
                .checked_add(1)
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
            if pin.owner == *owner {
                owner_count = owner_count
                    .checked_add(1)
                    .ok_or_else(|| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
            }
        }
    }
    if count > MAX_ACTIVE_PINS {
        return Err(corrupt());
    }
    if active_count >= MAX_ACTIVE_PINS || owner_count >= MAX_ACTIVE_PINS_PER_OWNER {
        return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
    }
    drop(statement);
    for (id, expires_at_unix_ms) in expired {
        let changed = transaction
            .execute(
                "DELETE FROM snapshot_review_pins
                 WHERE pin_id = ?1 AND expires_at_unix_ms = ?2",
                params![id.as_str(), expires_at_unix_ms],
            )
            .map_err(map_write_sql_error)?;
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
    }
    Ok(())
}

/// Strictly inspect the complete bounded pin population without pruning it.
///
/// Expiry equality is inactive. An active pin attached to a tombstoned
/// snapshot is corruption rather than protection evidence. Expired rows are
/// still decoded and relationship-validated so hostile SQLite values cannot
/// disappear from both sides of a later retention decision.
pub(super) fn inspect_snapshot_review_pin_population(
    connection: &Connection,
    observed_at: SystemTime,
) -> Result<SnapshotReviewPinPopulation, HistoryError> {
    let observed_at_unix_ms = system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
    run_bounded_snapshot_pin_query(connection, || {
        let mut statement = connection
            .prepare(&format!(
                "{PIN_SELECT_PREFIX}\n                 ORDER BY pin.expires_at_unix_ms ASC, pin.pin_id ASC\n                 LIMIT {}",
                MAX_ACTIVE_PINS + 1
            ))
            .map_err(map_query_sql_error)?;
        let rows = statement
            .query_map([], raw_pin)
            .map_err(map_query_sql_error)?;
        let mut population = SnapshotReviewPinPopulation::default();
        let mut count = 0_usize;
        for row in rows {
            let pin = decode_pin(row.map_err(map_query_sql_error)?)?;
            count = count
                .checked_add(1)
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
            if count > MAX_ACTIVE_PINS {
                return Err(corrupt());
            }
            let scan_id = ScanId::new(pin.scan_id.clone()).map_err(|_| corrupt())?;
            let summary = population.by_scan.entry(scan_id).or_default();
            if pin.expires_at_unix_ms <= observed_at_unix_ms {
                summary.expired = summary
                    .expired
                    .checked_add(1)
                    .ok_or_else(|| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
                population.expired = population
                    .expired
                    .checked_add(1)
                    .ok_or_else(|| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
                continue;
            }
            if pin.tombstone_exact {
                return Err(corrupt());
            }
            summary.active = summary
                .active
                .checked_add(1)
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
            match pin.purpose {
                SnapshotReviewPurpose::Explorer => {
                    summary.active_explorer = summary
                        .active_explorer
                        .checked_add(1)
                        .ok_or_else(|| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
                }
                SnapshotReviewPurpose::CleanupReview => {
                    summary.active_cleanup_review = summary
                        .active_cleanup_review
                        .checked_add(1)
                        .ok_or_else(|| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
                }
            }
            population.active = population
                .active
                .checked_add(1)
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::QueryLimitExceeded))?;
        }
        Ok(population)
    })
}

/*
 * The population inspection above intentionally covers every row rather than
 * relying on SQLite's coercion rules in separate active/expired predicates.
 * A hostile NULL/text expiry must be decoded as corruption, never disappear
 * from both sides of a retention decision.
 */

const PIN_SELECT_PREFIX: &str = "SELECT
        typeof(pin.pin_id), length(CAST(pin.pin_id AS BLOB)), pin.pin_id,
        typeof(pin.record_format_version), pin.record_format_version,
        typeof(pin.scan_id), length(CAST(pin.scan_id AS BLOB)), pin.scan_id,
        typeof(pin.scan_status), length(CAST(pin.scan_status AS BLOB)), pin.scan_status,
        typeof(pin.completed_at_unix_ms), pin.completed_at_unix_ms,
        typeof(pin.snapshot_version), pin.snapshot_version,
        typeof(pin.snapshot_relative_path), length(pin.snapshot_relative_path),
        pin.snapshot_relative_path,
        typeof(pin.snapshot_relative_path_encoding), pin.snapshot_relative_path_encoding,
        typeof(pin.snapshot_checksum_sha256), length(pin.snapshot_checksum_sha256),
        pin.snapshot_checksum_sha256,
        typeof(pin.owner_process_instance),
        length(CAST(pin.owner_process_instance AS BLOB)), pin.owner_process_instance,
        typeof(pin.purpose), length(CAST(pin.purpose AS BLOB)), pin.purpose,
        typeof(pin.created_at_unix_ms), pin.created_at_unix_ms,
        typeof(pin.renewed_at_unix_ms), pin.renewed_at_unix_ms,
        typeof(pin.expires_at_unix_ms), pin.expires_at_unix_ms,
        EXISTS (
            SELECT 1 FROM scans AS scan
            WHERE scan.scan_id = pin.scan_id
              AND typeof(scan.scan_id) = 'text'
              AND length(CAST(scan.scan_id AS BLOB)) BETWEEN 1 AND 128
              AND typeof(scan.status) = 'text'
              AND length(CAST(scan.status AS BLOB)) BETWEEN 1 AND 16
              AND typeof(scan.completed_at_unix_ms) = 'integer'
              AND typeof(scan.snapshot_version) = 'integer'
              AND typeof(scan.snapshot_relative_path) = 'blob'
              AND length(scan.snapshot_relative_path) BETWEEN 1 AND 65536
              AND typeof(scan.snapshot_relative_path_encoding) = 'integer'
              AND typeof(scan.snapshot_checksum_sha256) = 'blob'
              AND length(scan.snapshot_checksum_sha256) = 32
              AND scan.status = pin.scan_status
              AND scan.completed_at_unix_ms = pin.completed_at_unix_ms
              AND scan.snapshot_version = pin.snapshot_version
              AND scan.snapshot_relative_path = pin.snapshot_relative_path
              AND scan.snapshot_relative_path_encoding = pin.snapshot_relative_path_encoding
              AND scan.snapshot_checksum_sha256 = pin.snapshot_checksum_sha256
        ),
        EXISTS (
            SELECT 1 FROM snapshot_retention_tombstones AS tombstone
            WHERE tombstone.scan_id = pin.scan_id
        ),
        EXISTS (
            SELECT 1 FROM snapshot_retention_tombstones AS tombstone
            WHERE tombstone.scan_id = pin.scan_id
              AND typeof(tombstone.scan_id) = 'text'
              AND length(CAST(tombstone.scan_id AS BLOB)) BETWEEN 1 AND 128
              AND typeof(tombstone.record_format_version) = 'integer'
              AND tombstone.record_format_version = 1
              AND typeof(tombstone.scan_status) = 'text'
              AND tombstone.scan_status = pin.scan_status
              AND typeof(tombstone.completed_at_unix_ms) = 'integer'
              AND tombstone.completed_at_unix_ms = pin.completed_at_unix_ms
              AND typeof(tombstone.snapshot_version) = 'integer'
              AND tombstone.snapshot_version = pin.snapshot_version
              AND typeof(tombstone.snapshot_relative_path) = 'blob'
              AND tombstone.snapshot_relative_path = pin.snapshot_relative_path
              AND typeof(tombstone.snapshot_relative_path_encoding) = 'integer'
              AND tombstone.snapshot_relative_path_encoding = pin.snapshot_relative_path_encoding
              AND typeof(tombstone.snapshot_checksum_sha256) = 'blob'
              AND length(tombstone.snapshot_checksum_sha256) = 32
              AND tombstone.snapshot_checksum_sha256 = pin.snapshot_checksum_sha256
              AND typeof(tombstone.committed_at_unix_ms) = 'integer'
              AND tombstone.committed_at_unix_ms >= tombstone.completed_at_unix_ms
        )
     FROM snapshot_review_pins AS pin";

fn raw_pin(row: &Row<'_>) -> rusqlite::Result<RawSnapshotReviewPin> {
    require_type_length(row, 0, 1, "text", 32, 32)?;
    require_type(row, 3, "integer")?;
    require_type_length(row, 5, 6, "text", 1, MAX_ID_BYTES)?;
    require_type_length(row, 8, 9, "text", 1, MAX_STATUS_BYTES)?;
    require_type(row, 11, "integer")?;
    require_type(row, 13, "integer")?;
    require_type_length(row, 15, 16, "blob", 1, MAX_PATH_BYTES)?;
    require_type(row, 18, "integer")?;
    require_type_length(row, 20, 21, "blob", 32, 32)?;
    require_type_length(row, 23, 24, "text", 1, MAX_OWNER_BYTES)?;
    require_type_length(row, 26, 27, "text", 1, MAX_PURPOSE_BYTES)?;
    require_type(row, 29, "integer")?;
    require_type(row, 31, "integer")?;
    require_type(row, 33, "integer")?;
    let digest: Vec<u8> = row.get(22)?;
    let digest: [u8; 32] = digest
        .try_into()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(RawSnapshotReviewPin {
        id: row.get(2)?,
        record_format_version: row.get(4)?,
        scan_id: row.get(7)?,
        scan_status: row.get(10)?,
        completed_at_unix_ms: row.get(12)?,
        snapshot_version: row.get(14)?,
        snapshot_relative_path: row.get(17)?,
        snapshot_relative_path_encoding: row.get(19)?,
        snapshot_checksum_sha256: digest,
        owner: row.get(25)?,
        purpose: row.get(28)?,
        created_at_unix_ms: row.get(30)?,
        renewed_at_unix_ms: row.get(32)?,
        expires_at_unix_ms: row.get(34)?,
        parent_matches: row.get(35)?,
        tombstone_present: row.get(36)?,
        tombstone_exact: row.get(37)?,
    })
}

struct RawSnapshotReviewPin {
    id: String,
    record_format_version: i64,
    scan_id: String,
    scan_status: String,
    completed_at_unix_ms: i64,
    snapshot_version: i64,
    snapshot_relative_path: Vec<u8>,
    snapshot_relative_path_encoding: i64,
    snapshot_checksum_sha256: [u8; 32],
    owner: String,
    purpose: String,
    created_at_unix_ms: i64,
    renewed_at_unix_ms: i64,
    expires_at_unix_ms: i64,
    parent_matches: i64,
    tombstone_present: i64,
    tombstone_exact: i64,
}

fn decode_pin(raw: RawSnapshotReviewPin) -> Result<StoredSnapshotReviewPin, HistoryError> {
    let pin = StoredSnapshotReviewPin {
        id: SnapshotReviewPinId::from_stored(raw.id)?,
        record_format_version: raw.record_format_version,
        scan_id: raw.scan_id,
        scan_status: raw.scan_status,
        completed_at_unix_ms: raw.completed_at_unix_ms,
        snapshot_version: raw.snapshot_version,
        snapshot_relative_path: raw.snapshot_relative_path,
        snapshot_relative_path_encoding: raw.snapshot_relative_path_encoding,
        snapshot_checksum_sha256: raw.snapshot_checksum_sha256,
        owner: ProcessInstanceId::from_stored(&raw.owner).map_err(map_process_identity_error)?,
        purpose: SnapshotReviewPurpose::from_stored(&raw.purpose)?,
        created_at_unix_ms: raw.created_at_unix_ms,
        renewed_at_unix_ms: raw.renewed_at_unix_ms,
        expires_at_unix_ms: raw.expires_at_unix_ms,
        parent_matches: decode_boolean(raw.parent_matches)?,
        tombstone_present: decode_boolean(raw.tombstone_present)?,
        tombstone_exact: decode_boolean(raw.tombstone_exact)?,
    };
    pin.validate_semantics()?;
    Ok(pin)
}

fn decode_boolean(value: i64) -> Result<bool, HistoryError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(corrupt()),
    }
}

fn require_type(row: &Row<'_>, column: usize, expected: &str) -> rusqlite::Result<()> {
    let stored: String = row.get(column)?;
    if stored == expected {
        Ok(())
    } else {
        Err(rusqlite::Error::InvalidQuery)
    }
}

fn require_type_length(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
    expected: &str,
    minimum: i64,
    maximum: i64,
) -> rusqlite::Result<()> {
    require_type(row, type_column, expected)?;
    let length: i64 = row.get(length_column)?;
    if (minimum..=maximum).contains(&length) {
        Ok(())
    } else {
        Err(rusqlite::Error::InvalidQuery)
    }
}

fn map_process_identity_error(error: ProcessIdentityError) -> HistoryError {
    match error {
        ProcessIdentityError::InvalidEncoding => corrupt(),
        ProcessIdentityError::ObservationUnavailable | ProcessIdentityError::RandomUnavailable => {
            HistoryError::new(HistoryErrorKind::InternalState)
        }
    }
}

fn push_lower_hex(output: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}
