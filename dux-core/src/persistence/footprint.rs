//! Bounded, read-only accounting for storage owned by DUX.
//!
//! Physical SQLite/control usage is observed separately by the descriptor-
//! backed storage layer. AI insight bytes are a logical-content subset of that
//! SQLite usage, never an additional physical total or cleanup authority.

use std::time::SystemTime;
#[cfg(test)]
use std::time::{Duration, UNIX_EPOCH};

use rusqlite::Connection;

use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, run_bounded_owned_storage_footprint_query,
    system_time_to_unix_ms,
};

const AI_INSIGHT_QUERY_PAGE_ROWS: i64 = 256;
const MAX_ID_BYTES: i64 = 128;
const MAX_PROVIDER_BYTES: i64 = 128;
const MAX_ADAPTER_VERSION_BYTES: i64 = 128;
const MAX_MODEL_BYTES: i64 = 256;
const MAX_AI_PAYLOAD_BYTES: i64 = 16_777_216;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OwnedStorageUsage {
    pub(crate) logical_bytes: u64,
    pub(crate) allocated_bytes: u64,
    pub(crate) charged_bytes: u64,
}

impl OwnedStorageUsage {
    pub(crate) fn from_sizes(logical_bytes: u64, allocated_bytes: u64) -> Self {
        Self {
            logical_bytes,
            allocated_bytes,
            charged_bytes: logical_bytes.max(allocated_bytes),
        }
    }

    pub(crate) fn checked_add(self, other: Self) -> Result<Self, HistoryError> {
        Ok(Self {
            logical_bytes: self
                .logical_bytes
                .checked_add(other.logical_bytes)
                .ok_or_else(corrupt)?,
            allocated_bytes: self
                .allocated_bytes
                .checked_add(other.allocated_bytes)
                .ok_or_else(corrupt)?,
            charged_bytes: self
                .charged_bytes
                .checked_add(other.charged_bytes)
                .ok_or_else(corrupt)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AiCacheFootprint {
    pub(crate) record_count: u32,
    pub(crate) logical_content_bytes: u64,
    pub(crate) expired_record_count: u32,
    pub(crate) expired_logical_content_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OwnedSnapshotStorageFootprint {
    pub(crate) cap_bytes: u64,
    pub(crate) cap_excess_bytes: u64,
    pub(crate) controls: OwnedStorageUsage,
    pub(crate) available: OwnedStorageUsage,
    pub(crate) protected: OwnedStorageUsage,
    pub(crate) retention_eligible: OwnedStorageUsage,
    pub(crate) tombstoned_residual: OwnedStorageUsage,
    pub(crate) orphan: OwnedStorageUsage,
    pub(crate) temporary_active: OwnedStorageUsage,
    pub(crate) temporary_quiescent: OwnedStorageUsage,
    pub(crate) temporary_unleased: OwnedStorageUsage,
    pub(crate) total: OwnedStorageUsage,
    pub(crate) available_count: u32,
    pub(crate) protected_count: u32,
    pub(crate) retention_eligible_count: u32,
    pub(crate) tombstoned_residual_count: u32,
    pub(crate) orphan_count: u32,
    pub(crate) active_temporary_count: u32,
    pub(crate) quiescent_temporary_count: u32,
    pub(crate) unleased_temporary_count: u32,
    pub(crate) residual_temporary_lease_count: u32,
    pub(crate) active_pin_rows: u32,
    pub(crate) expired_pin_rows: u32,
    pub(crate) non_evictable_over_cap: bool,
    pub(crate) accounting_unstable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DuxOwnedStorageFootprint {
    pub(crate) observed_at: SystemTime,
    pub(crate) database: OwnedStorageUsage,
    pub(crate) snapshots: OwnedSnapshotStorageFootprint,
    /// Logical content embedded in `database`, never an additive physical
    /// component.
    pub(crate) embedded_ai_cache: AiCacheFootprint,
    pub(crate) physical_total: OwnedStorageUsage,
}

pub(super) fn inspect_ai_cache_footprint(
    connection: &Connection,
    observed_at: SystemTime,
) -> Result<AiCacheFootprint, HistoryError> {
    let observed_at_unix_ms = system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
    run_bounded_owned_storage_footprint_query(connection, || {
        let mut result = AiCacheFootprint::default();
        let mut after_rowid: Option<i64> = None;
        loop {
            let mut statement = connection
                .prepare(
                    "SELECT
                    typeof(insight_id), length(CAST(insight_id AS BLOB)),
                    typeof(input_digest), length(input_digest),
                    typeof(provider), length(CAST(provider AS BLOB)),
                    typeof(adapter_version), length(CAST(adapter_version AS BLOB)),
                    typeof(model_label),
                    COALESCE(length(CAST(model_label AS BLOB)), 0),
                    typeof(output_schema_version), output_schema_version,
                    typeof(output_payload), length(output_payload),
                    typeof(created_at_unix_ms), created_at_unix_ms,
                    typeof(expires_at_unix_ms), expires_at_unix_ms,
                    rowid
                 FROM ai_insights
                 WHERE ?1 IS NULL OR rowid > ?1
                 ORDER BY rowid
                 LIMIT ?2",
                )
                .map_err(map_query_sql_error)?;
            let mut rows = statement
                .query(rusqlite::params![after_rowid, AI_INSIGHT_QUERY_PAGE_ROWS])
                .map_err(map_query_sql_error)?;
            let mut page_count = 0_i64;
            let mut last_rowid = after_rowid;
            while let Some(row) = rows.next().map_err(map_query_sql_error)? {
                page_count = page_count.checked_add(1).ok_or_else(corrupt)?;
                require_type_length(row, 0, 1, "text", 1, MAX_ID_BYTES)?;
                require_type_length(row, 2, 3, "blob", 32, 32)?;
                require_type_length(row, 4, 5, "text", 1, MAX_PROVIDER_BYTES)?;
                require_type_length(row, 6, 7, "text", 1, MAX_ADAPTER_VERSION_BYTES)?;
                let model_type: String = row.get(8).map_err(map_query_sql_error)?;
                let model_length: i64 = row.get(9).map_err(map_query_sql_error)?;
                if !((model_type == "null" && model_length == 0)
                    || (model_type == "text" && (1..=MAX_MODEL_BYTES).contains(&model_length)))
                {
                    return Err(corrupt());
                }
                require_type(row, 10, "integer")?;
                let output_schema_version: i64 = row.get(11).map_err(map_query_sql_error)?;
                require_type_length(row, 12, 13, "blob", 1, MAX_AI_PAYLOAD_BYTES)?;
                require_type(row, 14, "integer")?;
                let created_at_unix_ms: i64 = row.get(15).map_err(map_query_sql_error)?;
                require_type(row, 16, "integer")?;
                let expires_at_unix_ms: i64 = row.get(17).map_err(map_query_sql_error)?;
                let rowid: i64 = row.get(18).map_err(map_query_sql_error)?;
                if output_schema_version <= 0
                    || created_at_unix_ms < 0
                    || expires_at_unix_ms < created_at_unix_ms
                    || last_rowid.is_some_and(|previous| rowid <= previous)
                {
                    return Err(corrupt());
                }

                let logical_content_bytes =
                    [1, 3, 5, 7, 9, 13]
                        .into_iter()
                        .try_fold(0_u64, |total, column| {
                            let length: i64 = row.get(column).map_err(map_query_sql_error)?;
                            let length = u64::try_from(length).map_err(|_| corrupt())?;
                            total.checked_add(length).ok_or_else(corrupt)
                        })?;
                result.record_count = result.record_count.checked_add(1).ok_or_else(limit)?;
                result.logical_content_bytes = result
                    .logical_content_bytes
                    .checked_add(logical_content_bytes)
                    .ok_or_else(corrupt)?;
                if expires_at_unix_ms <= observed_at_unix_ms {
                    result.expired_record_count = result
                        .expired_record_count
                        .checked_add(1)
                        .ok_or_else(limit)?;
                    result.expired_logical_content_bytes = result
                        .expired_logical_content_bytes
                        .checked_add(logical_content_bytes)
                        .ok_or_else(corrupt)?;
                }
                last_rowid = Some(rowid);
            }
            if page_count < AI_INSIGHT_QUERY_PAGE_ROWS {
                break;
            }
            after_rowid = last_rowid;
        }
        Ok(result)
    })
}

fn require_type(
    row: &rusqlite::Row<'_>,
    type_column: usize,
    expected: &str,
) -> Result<(), HistoryError> {
    let actual: String = row.get(type_column).map_err(map_query_sql_error)?;
    if actual == expected {
        Ok(())
    } else {
        Err(corrupt())
    }
}

fn require_type_length(
    row: &rusqlite::Row<'_>,
    type_column: usize,
    length_column: usize,
    expected_type: &str,
    minimum: i64,
    maximum: i64,
) -> Result<(), HistoryError> {
    require_type(row, type_column, expected_type)?;
    let length: i64 = row.get(length_column).map_err(map_query_sql_error)?;
    if (minimum..=maximum).contains(&length) {
        Ok(())
    } else {
        Err(corrupt())
    }
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

const fn limit() -> HistoryError {
    HistoryError::new(HistoryErrorKind::QueryLimitExceeded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ai_insights (
                    insight_id TEXT PRIMARY KEY,
                    input_digest BLOB NOT NULL,
                    provider TEXT NOT NULL,
                    adapter_version TEXT NOT NULL,
                    model_label TEXT,
                    output_schema_version INTEGER NOT NULL,
                    output_payload BLOB NOT NULL,
                    created_at_unix_ms INTEGER NOT NULL,
                    expires_at_unix_ms INTEGER NOT NULL
                ) STRICT;",
            )
            .unwrap();
        connection
    }

    #[test]
    fn ai_cache_footprint_is_exact_non_additive_content_accounting() {
        let connection = connection();
        connection
            .execute(
                "INSERT INTO ai_insights VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    "one",
                    vec![0_u8; 32],
                    "local",
                    "v1",
                    "model",
                    1_i64,
                    vec![1_u8; 10],
                    1_000_i64,
                    2_000_i64,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ai_insights VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8)",
                params![
                    "two",
                    vec![0_u8; 32],
                    "remote",
                    "v2",
                    1_i64,
                    vec![2_u8; 20],
                    2_000_i64,
                    4_000_i64,
                ],
            )
            .unwrap();

        let result =
            inspect_ai_cache_footprint(&connection, UNIX_EPOCH + Duration::from_millis(3_000))
                .unwrap();
        assert_eq!(result.record_count, 2);
        assert_eq!(result.expired_record_count, 1);
        assert_eq!(result.logical_content_bytes, 120);
        assert_eq!(result.expired_logical_content_bytes, 57);
    }

    #[test]
    fn ai_cache_footprint_rejects_malformed_rows_and_invalid_clock() {
        let connection = connection();
        connection
            .execute(
                "INSERT INTO ai_insights VALUES ('bad', zeroblob(31), 'p', 'v', NULL, 1, x'01', 1, 2)",
                [],
            )
            .unwrap();
        assert_eq!(
            inspect_ai_cache_footprint(&connection, UNIX_EPOCH + Duration::from_secs(1))
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
        assert_eq!(
            inspect_ai_cache_footprint(
                &Connection::open_in_memory().unwrap(),
                UNIX_EPOCH - Duration::from_millis(1),
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );
    }

    #[test]
    fn ai_cache_footprint_pages_beyond_the_old_arbitrary_row_limit() {
        let mut connection = connection();
        let transaction = connection.transaction().unwrap();
        {
            let mut insert = transaction
                .prepare(
                    "INSERT INTO ai_insights VALUES (?1, zeroblob(32), 'p', 'v', NULL, 1, x'01', 1, 2)",
                )
                .unwrap();
            for index in 0..4_097_u32 {
                insert.execute([format!("insight-{index}")]).unwrap();
            }
        }
        transaction.commit().unwrap();

        let result =
            inspect_ai_cache_footprint(&connection, UNIX_EPOCH + Duration::from_millis(1)).unwrap();
        assert_eq!(result.record_count, 4_097);
        assert_eq!(result.expired_record_count, 0);
        assert!(result.logical_content_bytes > u64::from(result.record_count));
    }
}
