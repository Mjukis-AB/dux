//! Append-only logical unavailability for immutable snapshot history.
//!
//! Absence of a tombstone means the immutable file may still be opened. A
//! matching tombstone means retention committed before any later unlink. This
//! module deliberately exposes no production tombstone writer until latest-two,
//! active-review pin, and total-cap eligibility are implemented together.

use std::path::Path;
use std::time::SystemTime;

use rusqlite::{Connection, OptionalExtension, Row};

use super::codec::encode_host_path;
use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, run_bounded_query, unix_ms_to_system_time,
};
use super::snapshot::SnapshotReference;

const RECORD_FORMAT_VERSION: i64 = 1;
const MAX_ID_BYTES: i64 = 128;
const MAX_STATUS_BYTES: i64 = 16;
const MAX_PATH_BYTES: i64 = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapshotRetentionState {
    Available,
    Tombstoned { committed_at: SystemTime },
}

struct RawTombstone {
    scan_id: String,
    record_format_version: i64,
    scan_status: String,
    completed_at_unix_ms: i64,
    snapshot_version: i64,
    snapshot_relative_path: Vec<u8>,
    snapshot_relative_path_encoding: i64,
    snapshot_checksum_sha256: Vec<u8>,
    committed_at_unix_ms: i64,
    parent_scan_status: String,
    parent_completed_at_unix_ms: i64,
    parent_snapshot_version: i64,
    parent_snapshot_relative_path: Vec<u8>,
    parent_snapshot_relative_path_encoding: i64,
    parent_snapshot_checksum_sha256: Vec<u8>,
}

pub(super) fn load_snapshot_retention_state(
    connection: &Connection,
    reference: &SnapshotReference,
) -> Result<SnapshotRetentionState, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(
                "SELECT
                    typeof(tombstone.scan_id),
                    length(CAST(tombstone.scan_id AS BLOB)), tombstone.scan_id,
                    typeof(tombstone.record_format_version),
                    tombstone.record_format_version,
                    typeof(tombstone.scan_status),
                    length(CAST(tombstone.scan_status AS BLOB)), tombstone.scan_status,
                    typeof(tombstone.completed_at_unix_ms),
                    tombstone.completed_at_unix_ms,
                    typeof(tombstone.snapshot_version), tombstone.snapshot_version,
                    typeof(tombstone.snapshot_relative_path),
                    length(tombstone.snapshot_relative_path),
                    tombstone.snapshot_relative_path,
                    typeof(tombstone.snapshot_relative_path_encoding),
                    tombstone.snapshot_relative_path_encoding,
                    typeof(tombstone.snapshot_checksum_sha256),
                    length(tombstone.snapshot_checksum_sha256),
                    tombstone.snapshot_checksum_sha256,
                    typeof(tombstone.committed_at_unix_ms),
                    tombstone.committed_at_unix_ms,
                    typeof(scan.status), length(CAST(scan.status AS BLOB)), scan.status,
                    typeof(scan.completed_at_unix_ms), scan.completed_at_unix_ms,
                    typeof(scan.snapshot_version), scan.snapshot_version,
                    typeof(scan.snapshot_relative_path), length(scan.snapshot_relative_path),
                    scan.snapshot_relative_path,
                    typeof(scan.snapshot_relative_path_encoding),
                    scan.snapshot_relative_path_encoding,
                    typeof(scan.snapshot_checksum_sha256),
                    length(scan.snapshot_checksum_sha256), scan.snapshot_checksum_sha256
                 FROM snapshot_retention_tombstones AS tombstone
                 LEFT JOIN scans AS scan ON scan.scan_id = tombstone.scan_id
                 WHERE tombstone.scan_id = ?1",
                [reference.scan_id().as_str()],
                raw_tombstone,
            )
            .optional()
            .map_err(map_query_sql_error)?;
        raw.map(|row| decode_tombstone(row, reference))
            .transpose()
            .map(|tombstone| tombstone.unwrap_or(SnapshotRetentionState::Available))
    })
}

fn raw_tombstone(row: &Row<'_>) -> rusqlite::Result<RawTombstone> {
    require_type_length(row, 0, 1, "text", 1, MAX_ID_BYTES)?;
    require_type(row, 3, "integer")?;
    require_type_length(row, 5, 6, "text", 1, MAX_STATUS_BYTES)?;
    require_type(row, 8, "integer")?;
    require_type(row, 10, "integer")?;
    require_type_length(row, 12, 13, "blob", 1, MAX_PATH_BYTES)?;
    require_type(row, 15, "integer")?;
    require_type_length(row, 17, 18, "blob", 32, 32)?;
    require_type(row, 20, "integer")?;
    require_type_length(row, 22, 23, "text", 1, MAX_STATUS_BYTES)?;
    require_type(row, 25, "integer")?;
    require_type(row, 27, "integer")?;
    require_type_length(row, 29, 30, "blob", 1, MAX_PATH_BYTES)?;
    require_type(row, 32, "integer")?;
    require_type_length(row, 34, 35, "blob", 32, 32)?;
    Ok(RawTombstone {
        scan_id: row.get(2)?,
        record_format_version: row.get(4)?,
        scan_status: row.get(7)?,
        completed_at_unix_ms: row.get(9)?,
        snapshot_version: row.get(11)?,
        snapshot_relative_path: row.get(14)?,
        snapshot_relative_path_encoding: row.get(16)?,
        snapshot_checksum_sha256: row.get(19)?,
        committed_at_unix_ms: row.get(21)?,
        parent_scan_status: row.get(24)?,
        parent_completed_at_unix_ms: row.get(26)?,
        parent_snapshot_version: row.get(28)?,
        parent_snapshot_relative_path: row.get(31)?,
        parent_snapshot_relative_path_encoding: row.get(33)?,
        parent_snapshot_checksum_sha256: row.get(36)?,
    })
}

fn decode_tombstone(
    raw: RawTombstone,
    reference: &SnapshotReference,
) -> Result<SnapshotRetentionState, HistoryError> {
    let expected_path =
        encode_host_path(Path::new(reference.file_name().as_str())).map_err(|_| corrupt())?;
    let completed_at = unix_ms_to_system_time(raw.completed_at_unix_ms)?;
    let committed_at = unix_ms_to_system_time(raw.committed_at_unix_ms)?;
    if raw.scan_id != reference.scan_id().as_str()
        || raw.record_format_version != RECORD_FORMAT_VERSION
        || raw.scan_status != "succeeded"
        || raw.parent_scan_status != raw.scan_status
        || raw.parent_completed_at_unix_ms != raw.completed_at_unix_ms
        || raw.parent_snapshot_version != raw.snapshot_version
        || raw.parent_snapshot_relative_path != raw.snapshot_relative_path
        || raw.parent_snapshot_relative_path_encoding != raw.snapshot_relative_path_encoding
        || raw.parent_snapshot_checksum_sha256 != raw.snapshot_checksum_sha256
        || raw.snapshot_version != i64::from(reference.version())
        || raw.snapshot_relative_path != expected_path.bytes
        || raw.snapshot_relative_path_encoding != expected_path.encoding as i64
        || raw.snapshot_checksum_sha256.as_slice() != reference.digest().bytes()
        || committed_at < completed_at
    {
        return Err(corrupt());
    }
    Ok(SnapshotRetentionState::Tombstoned { committed_at })
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

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}
