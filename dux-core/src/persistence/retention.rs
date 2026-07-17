//! Bounded retention for DUX-owned SQLite telemetry and AI cache rows.
//!
//! This module never mutates scan, candidate, cleanup, rule-outcome, schedule,
//! or settings history. Snapshot-file retention is a separate lock-ordered
//! boundary.

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::domain::VolumeId;

use super::capacity_history::load_capacity_volume_interval_with_caller_budget;
use super::history::{HistoryError, HistoryErrorKind, map_query_sql_error};

const DAY_MS: i64 = 86_400_000;
const RAW_RETENTION_DAYS: i64 = 30;
const DAILY_RETENTION_DAYS: i64 = 365;
const RAW_PRUNE_BATCH_LIMIT: usize = 128;
const DAILY_PRUNE_BATCH_LIMIT: usize = 128;
const AI_PRUNE_BATCH_LIMIT: usize = 16;
const RETENTION_PROGRESS_INTERVAL: i32 = 100;
const RETENTION_MAX_CALLBACKS: u64 = 40_000;
const RETENTION_MAX_ELAPSED: Duration = Duration::from_secs(2);
const MAX_ID_BYTES: i64 = 128;
const MAX_PROVIDER_BYTES: i64 = 128;
const MAX_MODEL_BYTES: i64 = 256;
const MAX_AI_PAYLOAD_BYTES: i64 = 16_777_216;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RetentionBatchResult {
    pub(crate) observed_at: SystemTime,
    pub(crate) daily_rollups_created: u32,
    pub(crate) raw_samples_pruned: u32,
    pub(crate) daily_rollups_pruned: u32,
    pub(crate) ai_insights_pruned: u32,
    pub(crate) has_more: bool,
}

#[derive(Clone, Debug)]
pub(super) struct RetentionReconciliation {
    expected_rollups: Vec<ExpectedRollup>,
    raw_sample_ids_absent: Vec<i64>,
    daily_sample_ids_absent: Vec<i64>,
    ai_insight_ids_absent: Vec<String>,
}

pub(super) struct AppliedRetentionBatch {
    pub(super) result: RetentionBatchResult,
    pub(super) reconciliation: RetentionReconciliation,
}

#[derive(Clone, Copy, Debug)]
struct RetentionCutoffs {
    observed_at: SystemTime,
    observed_at_unix_ms: i64,
    raw_cutoff_unix_ms: i64,
    daily_cutoff_unix_ms: i64,
}

impl RetentionCutoffs {
    fn try_new(observed_at: SystemTime) -> Result<Self, HistoryError> {
        let milliseconds = observed_at
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_millis();
        let observed_at_unix_ms = i64::try_from(milliseconds).map_err(|_| invalid())?;
        let observed_at = UNIX_EPOCH
            .checked_add(Duration::from_millis(milliseconds as u64))
            .ok_or_else(invalid)?;
        let current_day_start_unix_ms = observed_at_unix_ms / DAY_MS * DAY_MS;
        let raw_cutoff_unix_ms =
            observed_at_unix_ms.saturating_sub(RAW_RETENTION_DAYS.saturating_mul(DAY_MS));
        let daily_cutoff_unix_ms =
            current_day_start_unix_ms.saturating_sub(DAILY_RETENTION_DAYS.saturating_mul(DAY_MS));
        Ok(Self {
            observed_at,
            observed_at_unix_ms,
            raw_cutoff_unix_ms,
            daily_cutoff_unix_ms,
        })
    }
}

#[derive(Clone, Debug)]
struct StoredSample {
    sample_id: i64,
    volume_id: String,
    sample_kind: SampleKind,
    sampled_at_unix_ms: i64,
    total_bytes: i64,
    available_bytes: i64,
    important_available_bytes: Option<i64>,
    pressure: String,
    policy_revision: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SampleKind {
    Raw,
    DailyRollup,
}

#[derive(Clone, Debug)]
struct ExpectedRollup {
    representative: StoredSample,
    day_start_unix_ms: i64,
    existed_before: bool,
}

impl ExpectedRollup {
    fn matches(&self, stored: &StoredSample) -> bool {
        stored.sample_kind == SampleKind::DailyRollup
            && stored.volume_id == self.representative.volume_id
            && stored.sampled_at_unix_ms == self.day_start_unix_ms
            && stored.total_bytes == self.representative.total_bytes
            && stored.available_bytes == self.representative.available_bytes
            && stored.important_available_bytes == self.representative.important_available_bytes
            && stored.pressure == self.representative.pressure
            && stored.policy_revision == self.representative.policy_revision
    }
}

#[derive(Debug)]
struct RetentionPlan {
    cutoffs: RetentionCutoffs,
    expected_rollups: Vec<ExpectedRollup>,
    raw_sample_ids: Vec<i64>,
    daily_sample_ids: Vec<i64>,
    ai_insight_ids: Vec<String>,
}

/// Apply one fixed retention batch inside the caller's current-schema,
/// writer-leased immediate transaction.
pub(super) fn apply_retention_batch(
    transaction: &Transaction<'_>,
    observed_at: SystemTime,
) -> Result<AppliedRetentionBatch, HistoryError> {
    let cutoffs = RetentionCutoffs::try_new(observed_at)?;
    run_with_retention_guards(transaction, || {
        let plan = prepare_plan(transaction, cutoffs)?;
        apply_plan(transaction, plan)
    })
}

pub(super) fn reconcile_retention_batch(
    connection: &Connection,
    reconciliation: &RetentionReconciliation,
) -> Result<bool, HistoryError> {
    run_with_progress_budget(connection, || {
        for expected in &reconciliation.expected_rollups {
            let Some(stored) = load_daily_rollup(
                connection,
                &expected.representative.volume_id,
                expected.day_start_unix_ms,
            )?
            else {
                return Ok(false);
            };
            if !expected.matches(&stored) {
                return Ok(false);
            }
        }
        for sample_id in reconciliation
            .raw_sample_ids_absent
            .iter()
            .chain(&reconciliation.daily_sample_ids_absent)
        {
            if sample_id_exists(connection, *sample_id)? {
                return Ok(false);
            }
        }
        for insight_id in &reconciliation.ai_insight_ids_absent {
            if ai_insight_id_exists(connection, insight_id)? {
                return Ok(false);
            }
        }
        Ok(true)
    })
}

fn prepare_plan(
    connection: &Connection,
    cutoffs: RetentionCutoffs,
) -> Result<RetentionPlan, HistoryError> {
    let raw_sample_ids = select_sample_ids(
        connection,
        SampleKind::Raw,
        cutoffs.raw_cutoff_unix_ms,
        RAW_PRUNE_BATCH_LIMIT,
    )?;
    let daily_sample_ids = select_sample_ids(
        connection,
        SampleKind::DailyRollup,
        cutoffs.daily_cutoff_unix_ms,
        DAILY_PRUNE_BATCH_LIMIT,
    )?;
    let ai_insight_ids = select_expired_ai_ids(
        connection,
        cutoffs.observed_at_unix_ms,
        AI_PRUNE_BATCH_LIMIT,
    )?;

    let mut rollup_keys = BTreeMap::<(String, i64), StoredSample>::new();
    for sample_id in &raw_sample_ids {
        let sample = load_sample_by_id(connection, *sample_id)?.ok_or_else(corrupt)?;
        if sample.sample_kind != SampleKind::Raw
            || sample.sampled_at_unix_ms >= cutoffs.raw_cutoff_unix_ms
        {
            return Err(corrupt());
        }
        validate_sample_volume(connection, &sample)?;
        let day_start = day_start(sample.sampled_at_unix_ms)?;
        if day_start >= cutoffs.daily_cutoff_unix_ms {
            let representative = load_last_raw_in_day(connection, &sample.volume_id, day_start)?
                .ok_or_else(corrupt)?;
            validate_sample_volume(connection, &representative)?;
            rollup_keys.insert((sample.volume_id, day_start), representative);
        }
    }

    let mut expected_rollups = Vec::with_capacity(rollup_keys.len());
    for ((volume_id, day_start), representative) in rollup_keys {
        let existing = load_daily_rollup(connection, &volume_id, day_start)?;
        let expected = ExpectedRollup {
            representative,
            day_start_unix_ms: day_start,
            existed_before: existing.is_some(),
        };
        if existing
            .as_ref()
            .is_some_and(|stored| !expected.matches(stored))
        {
            return Err(corrupt());
        }
        expected_rollups.push(expected);
    }

    for sample_id in &daily_sample_ids {
        let sample = load_sample_by_id(connection, *sample_id)?.ok_or_else(corrupt)?;
        if sample.sample_kind != SampleKind::DailyRollup
            || sample.sampled_at_unix_ms >= cutoffs.daily_cutoff_unix_ms
            || sample.sampled_at_unix_ms % DAY_MS != 0
        {
            return Err(corrupt());
        }
        validate_sample_volume(connection, &sample)?;
    }

    for insight_id in &ai_insight_ids {
        validate_expired_ai(connection, insight_id, cutoffs.observed_at_unix_ms)?;
    }

    Ok(RetentionPlan {
        cutoffs,
        expected_rollups,
        raw_sample_ids,
        daily_sample_ids,
        ai_insight_ids,
    })
}

fn apply_plan(
    connection: &Connection,
    plan: RetentionPlan,
) -> Result<AppliedRetentionBatch, HistoryError> {
    for expected in &plan.expected_rollups {
        if !expected.existed_before {
            let changed = connection
                .execute(
                    "INSERT INTO disk_samples (
                        volume_id, sample_kind, sampled_at_unix_ms, total_bytes,
                        available_bytes, important_available_bytes, pressure,
                        policy_revision
                     ) VALUES (?1, 'daily_rollup', ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        expected.representative.volume_id,
                        expected.day_start_unix_ms,
                        expected.representative.total_bytes,
                        expected.representative.available_bytes,
                        expected.representative.important_available_bytes,
                        expected.representative.pressure,
                        expected.representative.policy_revision,
                    ],
                )
                .map_err(map_retention_write_error)?;
            if changed != 1 {
                return Err(internal());
            }
        }
        let stored = load_daily_rollup(
            connection,
            &expected.representative.volume_id,
            expected.day_start_unix_ms,
        )?
        .ok_or_else(corrupt)?;
        if !expected.matches(&stored) {
            return Err(corrupt());
        }
    }

    for sample_id in &plan.raw_sample_ids {
        let changed = connection
            .execute(
                "DELETE FROM disk_samples
                 WHERE sample_id = ?1 AND sample_kind = 'raw'
                   AND sampled_at_unix_ms < ?2",
                params![sample_id, plan.cutoffs.raw_cutoff_unix_ms],
            )
            .map_err(map_retention_write_error)?;
        if changed != 1 {
            return Err(corrupt());
        }
    }
    for sample_id in &plan.daily_sample_ids {
        let changed = connection
            .execute(
                "DELETE FROM disk_samples
                 WHERE sample_id = ?1 AND sample_kind = 'daily_rollup'
                   AND sampled_at_unix_ms < ?2",
                params![sample_id, plan.cutoffs.daily_cutoff_unix_ms],
            )
            .map_err(map_retention_write_error)?;
        if changed != 1 {
            return Err(corrupt());
        }
    }
    for insight_id in &plan.ai_insight_ids {
        let changed = connection
            .execute(
                "DELETE FROM ai_insights
                 WHERE insight_id = ?1 AND expires_at_unix_ms <= ?2",
                params![insight_id, plan.cutoffs.observed_at_unix_ms],
            )
            .map_err(map_retention_write_error)?;
        if changed != 1 {
            return Err(corrupt());
        }
    }

    let result = RetentionBatchResult {
        observed_at: plan.cutoffs.observed_at,
        daily_rollups_created: u32::try_from(
            plan.expected_rollups
                .iter()
                .filter(|rollup| !rollup.existed_before)
                .count(),
        )
        .map_err(|_| internal())?,
        raw_samples_pruned: u32::try_from(plan.raw_sample_ids.len()).map_err(|_| internal())?,
        daily_rollups_pruned: u32::try_from(plan.daily_sample_ids.len()).map_err(|_| internal())?,
        ai_insights_pruned: u32::try_from(plan.ai_insight_ids.len()).map_err(|_| internal())?,
        has_more: eligible_work_exists(connection, plan.cutoffs)?,
    };
    let reconciliation = RetentionReconciliation {
        expected_rollups: plan.expected_rollups,
        raw_sample_ids_absent: plan.raw_sample_ids,
        daily_sample_ids_absent: plan.daily_sample_ids,
        ai_insight_ids_absent: plan.ai_insight_ids,
    };
    Ok(AppliedRetentionBatch {
        result,
        reconciliation,
    })
}

fn select_sample_ids(
    connection: &Connection,
    kind: SampleKind,
    cutoff: i64,
    limit: usize,
) -> Result<Vec<i64>, HistoryError> {
    let limit = i64::try_from(limit).map_err(|_| internal())?;
    let mut statement = connection
        .prepare(
            "SELECT typeof(sample_id), sample_id
             FROM disk_samples INDEXED BY disk_samples_by_kind_time
             WHERE sample_kind = ?1 AND sampled_at_unix_ms < ?2
             ORDER BY sampled_at_unix_ms, sample_id LIMIT ?3",
        )
        .map_err(map_query_sql_error)?;
    let rows = statement
        .query_map(params![kind.as_stored(), cutoff, limit], |row| {
            require_type(row, 0, "integer")?;
            row.get::<_, i64>(1)
        })
        .map_err(map_query_sql_error)?;
    rows.map(|row| row.map_err(map_query_sql_error)).collect()
}

fn load_sample_by_id(
    connection: &Connection,
    sample_id: i64,
) -> Result<Option<StoredSample>, HistoryError> {
    connection
        .query_row(
            &format!("{} WHERE sample_id = ?1", sample_select()),
            [sample_id],
            raw_sample,
        )
        .optional()
        .map_err(map_query_sql_error)?
        .map(decode_sample)
        .transpose()
}

fn load_last_raw_in_day(
    connection: &Connection,
    volume_id: &str,
    day_start_unix_ms: i64,
) -> Result<Option<StoredSample>, HistoryError> {
    let day_end = day_start_unix_ms.checked_add(DAY_MS).ok_or_else(corrupt)?;
    connection
        .query_row(
            &format!(
                "{} WHERE volume_id = ?1 AND sample_kind = 'raw'
                 AND sampled_at_unix_ms >= ?2 AND sampled_at_unix_ms < ?3
                 ORDER BY sampled_at_unix_ms DESC, sample_id DESC LIMIT 1",
                sample_select()
            ),
            params![volume_id, day_start_unix_ms, day_end],
            raw_sample,
        )
        .optional()
        .map_err(map_query_sql_error)?
        .map(decode_sample)
        .transpose()
}

fn load_daily_rollup(
    connection: &Connection,
    volume_id: &str,
    day_start_unix_ms: i64,
) -> Result<Option<StoredSample>, HistoryError> {
    connection
        .query_row(
            &format!(
                "{} WHERE volume_id = ?1 AND sample_kind = 'daily_rollup'
                 AND sampled_at_unix_ms = ?2",
                sample_select()
            ),
            params![volume_id, day_start_unix_ms],
            raw_sample,
        )
        .optional()
        .map_err(map_query_sql_error)?
        .map(decode_sample)
        .transpose()
}

fn sample_select() -> &'static str {
    "SELECT
        typeof(sample_id), sample_id,
        typeof(volume_id), length(CAST(volume_id AS BLOB)), volume_id,
        typeof(sample_kind), length(CAST(sample_kind AS BLOB)), sample_kind,
        typeof(sampled_at_unix_ms), sampled_at_unix_ms,
        typeof(total_bytes), total_bytes,
        typeof(available_bytes), available_bytes,
        typeof(important_available_bytes), important_available_bytes,
        typeof(pressure), length(CAST(pressure AS BLOB)), pressure,
        typeof(policy_revision), policy_revision
     FROM disk_samples"
}

fn raw_sample(row: &Row<'_>) -> rusqlite::Result<StoredSample> {
    require_type(row, 0, "integer")?;
    require_type_length(row, 2, 3, "text", 1, MAX_ID_BYTES)?;
    require_type_length(row, 5, 6, "text", 1, 16)?;
    for column in [8, 10, 12] {
        require_type(row, column, "integer")?;
    }
    let important_type: String = row.get(14)?;
    if !matches!(important_type.as_str(), "null" | "integer") {
        return Err(rusqlite::Error::InvalidQuery);
    }
    require_type_length(row, 16, 17, "text", 1, 16)?;
    require_type(row, 19, "integer")?;
    let kind: String = row.get(7)?;
    let sample_kind = match kind.as_str() {
        "raw" => SampleKind::Raw,
        "daily_rollup" => SampleKind::DailyRollup,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(StoredSample {
        sample_id: row.get(1)?,
        volume_id: row.get(4)?,
        sample_kind,
        sampled_at_unix_ms: row.get(9)?,
        total_bytes: row.get(11)?,
        available_bytes: row.get(13)?,
        important_available_bytes: row.get(15)?,
        pressure: row.get(18)?,
        policy_revision: row.get(20)?,
    })
}

fn decode_sample(sample: StoredSample) -> Result<StoredSample, HistoryError> {
    if sample.sample_id <= 0
        || VolumeId::new(sample.volume_id.clone()).is_err()
        || sample.sampled_at_unix_ms < 0
        || sample.total_bytes <= 0
        || sample.available_bytes < 0
        || sample.available_bytes > sample.total_bytes
        || sample
            .important_available_bytes
            .is_some_and(|value| value < 0 || value > sample.total_bytes)
        || !matches!(
            sample.pressure.as_str(),
            "healthy" | "warning" | "critical" | "unknown"
        )
        || sample.policy_revision < 0
        || (sample.sample_kind == SampleKind::DailyRollup
            && sample.sampled_at_unix_ms % DAY_MS != 0)
    {
        return Err(corrupt());
    }
    Ok(sample)
}

fn validate_sample_volume(
    connection: &Connection,
    sample: &StoredSample,
) -> Result<(), HistoryError> {
    let volume_id = VolumeId::new(sample.volume_id.clone()).map_err(|_| corrupt())?;
    let interval = load_capacity_volume_interval_with_caller_budget(connection, &volume_id)?
        .ok_or_else(corrupt)?;
    let contained = match sample.sample_kind {
        SampleKind::Raw => (interval.0..=interval.1).contains(&sample.sampled_at_unix_ms),
        SampleKind::DailyRollup => {
            let day_end = sample
                .sampled_at_unix_ms
                .checked_add(DAY_MS)
                .ok_or_else(corrupt)?;
            interval.0 < day_end && interval.1 >= sample.sampled_at_unix_ms
        }
    };
    if !contained {
        return Err(corrupt());
    }
    Ok(())
}

fn select_expired_ai_ids(
    connection: &Connection,
    observed_at_unix_ms: i64,
    limit: usize,
) -> Result<Vec<String>, HistoryError> {
    let limit = i64::try_from(limit).map_err(|_| internal())?;
    let mut statement = connection
        .prepare(
            "SELECT typeof(insight_id), length(CAST(insight_id AS BLOB)), insight_id
             FROM ai_insights INDEXED BY ai_insights_by_expiration
             WHERE expires_at_unix_ms <= ?1
             ORDER BY expires_at_unix_ms, rowid LIMIT ?2",
        )
        .map_err(map_query_sql_error)?;
    let rows = statement
        .query_map(params![observed_at_unix_ms, limit], |row| {
            require_type_length(row, 0, 1, "text", 1, MAX_ID_BYTES)?;
            row.get::<_, String>(2)
        })
        .map_err(map_query_sql_error)?;
    rows.map(|row| row.map_err(map_query_sql_error)).collect()
}

fn validate_expired_ai(
    connection: &Connection,
    insight_id: &str,
    observed_at_unix_ms: i64,
) -> Result<(), HistoryError> {
    let valid = connection
        .query_row(
            "SELECT
                typeof(insight_id), length(CAST(insight_id AS BLOB)),
                typeof(input_digest), length(input_digest),
                typeof(provider), length(CAST(provider AS BLOB)),
                typeof(adapter_version), length(CAST(adapter_version AS BLOB)),
                typeof(model_label), COALESCE(length(CAST(model_label AS BLOB)), 0),
                typeof(output_schema_version), output_schema_version,
                typeof(output_payload), length(output_payload),
                typeof(created_at_unix_ms), created_at_unix_ms,
                typeof(expires_at_unix_ms), expires_at_unix_ms
             FROM ai_insights WHERE insight_id = ?1",
            [insight_id],
            |row| {
                require_type_length(row, 0, 1, "text", 1, MAX_ID_BYTES)?;
                require_type_length(row, 2, 3, "blob", 32, 32)?;
                require_type_length(row, 4, 5, "text", 1, MAX_PROVIDER_BYTES)?;
                require_type_length(row, 6, 7, "text", 1, MAX_PROVIDER_BYTES)?;
                let model_type: String = row.get(8)?;
                let model_length: i64 = row.get(9)?;
                if !((model_type == "null" && model_length == 0)
                    || (model_type == "text" && (1..=MAX_MODEL_BYTES).contains(&model_length)))
                {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                require_type(row, 10, "integer")?;
                require_type_length(row, 12, 13, "blob", 1, MAX_AI_PAYLOAD_BYTES)?;
                require_type(row, 14, "integer")?;
                require_type(row, 16, "integer")?;
                Ok((
                    row.get::<_, i64>(11)?,
                    row.get::<_, i64>(15)?,
                    row.get::<_, i64>(17)?,
                ))
            },
        )
        .optional()
        .map_err(map_query_sql_error)?;
    let Some((schema, created, expires)) = valid else {
        return Err(corrupt());
    };
    if schema <= 0 || created < 0 || expires < created || expires > observed_at_unix_ms {
        return Err(corrupt());
    }
    Ok(())
}

fn eligible_work_exists(
    connection: &Connection,
    cutoffs: RetentionCutoffs,
) -> Result<bool, HistoryError> {
    for (sql, cutoff) in [
        (
            "SELECT EXISTS(SELECT 1 FROM disk_samples
             WHERE sample_kind = 'raw' AND sampled_at_unix_ms < ?1)",
            cutoffs.raw_cutoff_unix_ms,
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM disk_samples
             WHERE sample_kind = 'daily_rollup' AND sampled_at_unix_ms < ?1)",
            cutoffs.daily_cutoff_unix_ms,
        ),
        (
            "SELECT EXISTS(SELECT 1 FROM ai_insights
             WHERE expires_at_unix_ms <= ?1)",
            cutoffs.observed_at_unix_ms,
        ),
    ] {
        let exists = connection
            .query_row(sql, [cutoff], |row| row.get(0))
            .map_err(map_query_sql_error)?;
        if stored_bool(exists)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn sample_id_exists(connection: &Connection, sample_id: i64) -> Result<bool, HistoryError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM disk_samples WHERE sample_id = ?1)",
            [sample_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(map_query_sql_error)
        .and_then(stored_bool)
}

fn ai_insight_id_exists(connection: &Connection, insight_id: &str) -> Result<bool, HistoryError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ai_insights WHERE insight_id = ?1)",
            [insight_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(map_query_sql_error)
        .and_then(stored_bool)
}

fn day_start(timestamp: i64) -> Result<i64, HistoryError> {
    if timestamp < 0 {
        return Err(corrupt());
    }
    Ok(timestamp / DAY_MS * DAY_MS)
}

impl SampleKind {
    const fn as_stored(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::DailyRollup => "daily_rollup",
        }
    }
}

fn stored_bool(value: i64) -> Result<bool, HistoryError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(corrupt()),
    }
}

fn require_type(row: &Row<'_>, column: usize, expected: &str) -> rusqlite::Result<()> {
    let actual: String = row.get(column)?;
    if actual == expected {
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

fn run_with_retention_guards<T>(
    connection: &Connection,
    operation: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    let mut authorizer = RetentionAuthorizerGuard::install(connection)?;
    let result = run_with_progress_budget(connection, operation);
    authorizer.remove()?;
    result
}

fn run_with_progress_budget<T>(
    connection: &Connection,
    operation: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    let started_at = Instant::now();
    let mut callbacks = 0_u64;
    connection
        .progress_handler(
            RETENTION_PROGRESS_INTERVAL,
            Some(move || {
                callbacks = callbacks.saturating_add(1);
                callbacks >= RETENTION_MAX_CALLBACKS
                    || started_at.elapsed() >= RETENTION_MAX_ELAPSED
            }),
        )
        .map_err(|_| unavailable())?;
    let mut guard = RetentionProgressGuard {
        connection,
        installed: true,
    };
    let result = operation();
    guard.remove()?;
    let value = result?;
    if started_at.elapsed() >= RETENTION_MAX_ELAPSED {
        return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
    }
    Ok(value)
}

struct RetentionProgressGuard<'a> {
    connection: &'a Connection,
    installed: bool,
}

impl RetentionProgressGuard<'_> {
    fn remove(&mut self) -> Result<(), HistoryError> {
        self.connection
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(|_| unavailable())?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for RetentionProgressGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self.connection.progress_handler(0, None::<fn() -> bool>);
        }
    }
}

struct RetentionAuthorizerGuard<'a> {
    connection: &'a Connection,
    installed: bool,
}

impl RetentionAuthorizerGuard<'_> {
    fn install(connection: &Connection) -> Result<RetentionAuthorizerGuard<'_>, HistoryError> {
        connection
            .authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Read { .. } | AuthAction::Select | AuthAction::Function { .. } => {
                    Authorization::Allow
                }
                AuthAction::Insert {
                    table_name: "disk_samples",
                } => Authorization::Allow,
                AuthAction::Delete {
                    table_name: "disk_samples" | "ai_insights",
                } => Authorization::Allow,
                _ => Authorization::Deny,
            }))
            .map_err(|_| unavailable())?;
        Ok(RetentionAuthorizerGuard {
            connection,
            installed: true,
        })
    }

    fn remove(&mut self) -> Result<(), HistoryError> {
        self.connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|_| unavailable())?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for RetentionAuthorizerGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self
                .connection
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}

fn invalid() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidInput)
}

fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

fn internal() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InternalState)
}

fn unavailable() -> HistoryError {
    HistoryError::new(HistoryErrorKind::DatabaseUnavailable)
}

fn map_retention_write_error(error: rusqlite::Error) -> HistoryError {
    use rusqlite::ErrorCode;

    let kind = match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => HistoryErrorKind::Busy,
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => HistoryErrorKind::CorruptData,
        Some(ErrorCode::OperationInterrupted) => HistoryErrorKind::QueryLimitExceeded,
        _ => HistoryErrorKind::DatabaseUnavailable,
    };
    HistoryError::new(kind)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::TempDir;

    use crate::domain::DiskPressure;

    use super::super::capacity_history::{
        CapacityWriteOutcome, CapacityWriteReason, RawCapacitySample,
    };
    use super::super::status::DATABASE_SCHEMA_VERSION;
    use super::super::store::StoreCoordinator;
    use super::*;

    fn store(temp: &TempDir) -> std::sync::Arc<StoreCoordinator> {
        StoreCoordinator::open(&temp.path().join("data/dux.sqlite3")).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn record_raw(
        store: &StoreCoordinator,
        mount: &Path,
        volume: &str,
        sampled_at_unix_ms: i64,
        total: u64,
        available: u64,
        important: Option<u64>,
        pressure: DiskPressure,
    ) {
        record_raw_with_revision(
            store,
            mount,
            volume,
            sampled_at_unix_ms,
            total,
            available,
            important,
            pressure,
            0,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn record_raw_with_revision(
        store: &StoreCoordinator,
        mount: &Path,
        volume: &str,
        sampled_at_unix_ms: i64,
        total: u64,
        available: u64,
        important: Option<u64>,
        pressure: DiskPressure,
        policy_revision: u64,
    ) {
        let sample = RawCapacitySample::try_new_with_policy_revision(
            VolumeId::new(volume).unwrap(),
            mount.to_path_buf(),
            "Test Volume".to_owned(),
            "apfs".to_owned(),
            true,
            false,
            UNIX_EPOCH + Duration::from_millis(sampled_at_unix_ms as u64),
            total,
            available,
            important,
            pressure,
            policy_revision,
        )
        .unwrap();
        assert_eq!(
            store
                .record_raw_capacity_sample(&sample, CapacityWriteReason::Routine)
                .unwrap(),
            CapacityWriteOutcome::Inserted
        );
    }

    fn sample_count(store: &StoreCoordinator, kind: &str) -> i64 {
        store.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT count(*) FROM disk_samples WHERE sample_kind = ?1",
                    [kind],
                    |row| row.get(0),
                )
                .unwrap()
        })
    }

    #[test]
    fn aging_raw_day_rolls_up_the_last_tuple_exactly_and_is_idempotent() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let today = 500 * DAY_MS;
        let source_day = today - 31 * DAY_MS;
        record_raw(
            &store,
            temp.path(),
            "volume:test",
            source_day + 3_600_000,
            1_000,
            800,
            Some(700),
            DiskPressure::Healthy,
        );
        record_raw_with_revision(
            &store,
            temp.path(),
            "volume:test",
            source_day + 23 * 3_600_000,
            1_000,
            300,
            None,
            DiskPressure::Warning,
            7,
        );
        record_raw(
            &store,
            temp.path(),
            "volume:test",
            today + 3_600_000,
            1_000,
            250,
            Some(200),
            DiskPressure::Warning,
        );
        let observed = UNIX_EPOCH + Duration::from_millis((today + 12 * 3_600_000) as u64);

        let result = store.run_history_retention_batch(observed).unwrap();
        assert_eq!(result.observed_at, observed);
        assert_eq!(result.daily_rollups_created, 1);
        assert_eq!(result.raw_samples_pruned, 2);
        assert!(!result.has_more);
        let daily = store.with_connection(|connection| {
            load_daily_rollup(connection, "volume:test", source_day)
                .unwrap()
                .unwrap()
        });
        assert_eq!(daily.total_bytes, 1_000);
        assert_eq!(daily.available_bytes, 300);
        assert_eq!(daily.important_available_bytes, None);
        assert_eq!(daily.pressure, "warning");
        assert_eq!(daily.policy_revision, 7);
        assert!(store.with_connection(|connection| {
            load_daily_rollup(connection, "volume:test", today)
                .unwrap()
                .is_none()
        }));

        let repeated = store.run_history_retention_batch(observed).unwrap();
        assert_eq!(repeated.daily_rollups_created, 0);
        assert!(!repeated.has_more);
        assert_eq!(sample_count(&store, "daily_rollup"), 1);
    }

    #[test]
    fn exact_retention_boundaries_are_kept_and_expiry_equality_is_pruned() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let today = 800 * DAY_MS;
        let observed_ms = today + 12 * 3_600_000;
        let raw_cutoff = observed_ms - RAW_RETENTION_DAYS * DAY_MS;
        let daily_cutoff = today - DAILY_RETENTION_DAYS * DAY_MS;
        for (timestamp, available) in [
            (today - 400 * DAY_MS + 3_600_000, 900_u64),
            (raw_cutoff - 1, 800),
            (raw_cutoff, 700),
            (today + 3_600_000, 600),
        ] {
            record_raw(
                &store,
                temp.path(),
                "volume:boundaries",
                timestamp,
                1_000,
                available,
                Some(available),
                DiskPressure::Healthy,
            );
        }
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO disk_samples (
                        volume_id, sample_kind, sampled_at_unix_ms, total_bytes,
                        available_bytes, important_available_bytes, pressure
                     ) VALUES
                        ('volume:boundaries', 'daily_rollup', ?1, 1000, 500, NULL, 'healthy'),
                        ('volume:boundaries', 'daily_rollup', ?2, 1000, 500, NULL, 'healthy')",
                    params![daily_cutoff - DAY_MS, daily_cutoff],
                )
                .unwrap();
            for (id, expires) in [("ai:expired", observed_ms), ("ai:future", observed_ms + 1)] {
                connection
                    .execute(
                        "INSERT INTO ai_insights (
                            insight_id, input_digest, provider, adapter_version,
                            model_label, output_schema_version, output_payload,
                            created_at_unix_ms, expires_at_unix_ms
                         ) VALUES (?1, ?2, 'test', '1', NULL, 1, x'01', ?3, ?4)",
                        params![id, vec![id.as_bytes()[3]; 32], expires - 1, expires],
                    )
                    .unwrap();
            }
        });

        let result = store
            .run_history_retention_batch(UNIX_EPOCH + Duration::from_millis(observed_ms as u64))
            .unwrap();
        assert_eq!(result.raw_samples_pruned, 2);
        assert_eq!(result.daily_rollups_pruned, 1);
        assert_eq!(result.ai_insights_pruned, 1);
        store.with_connection(|connection| {
            let retained_raw: Vec<i64> = connection
                .prepare(
                    "SELECT sampled_at_unix_ms FROM disk_samples
                     WHERE sample_kind = 'raw' ORDER BY sampled_at_unix_ms",
                )
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(retained_raw, [raw_cutoff, today + 3_600_000]);
            assert!(
                load_daily_rollup(connection, "volume:boundaries", daily_cutoff)
                    .unwrap()
                    .is_some()
            );
            let retained_ai: Vec<String> = connection
                .prepare("SELECT insight_id FROM ai_insights ORDER BY insight_id")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(retained_ai, ["ai:future"]);
        });
    }

    #[test]
    fn mismatched_existing_rollup_fails_closed_without_pruning_raw_history() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let today = 600 * DAY_MS;
        let source_day = today - 31 * DAY_MS;
        record_raw(
            &store,
            temp.path(),
            "volume:mismatch",
            source_day + 3_600_000,
            1_000,
            400,
            Some(300),
            DiskPressure::Warning,
        );
        record_raw(
            &store,
            temp.path(),
            "volume:mismatch",
            today + 3_600_000,
            1_000,
            350,
            Some(250),
            DiskPressure::Warning,
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO disk_samples (
                        volume_id, sample_kind, sampled_at_unix_ms, total_bytes,
                        available_bytes, important_available_bytes, pressure
                     ) VALUES ('volume:mismatch', 'daily_rollup', ?1, 1000, 999, 999, 'healthy')",
                    [source_day],
                )
                .unwrap();
        });

        let error = store
            .run_history_retention_batch(
                UNIX_EPOCH + Duration::from_millis((today + 12 * 3_600_000) as u64),
            )
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::CorruptData);
        assert_eq!(sample_count(&store, "raw"), 2);
    }

    #[test]
    fn mutation_authorizer_denies_history_tables_outside_the_retention_allowlist() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO rule_outcomes (
                        rule_id, rule_revision, cleaned_at_unix_ms, cleaned_bytes
                     ) VALUES ('rule.test', 1, 1, 2)",
                    [],
                )
                .unwrap();
            let denied = run_with_retention_guards(connection, || {
                connection
                    .execute("DELETE FROM rule_outcomes", [])
                    .map(|_| ())
                    .map_err(map_retention_write_error)
            });
            assert!(denied.is_err());
            for denied_sql in [
                "UPDATE disk_samples SET pressure = 'unknown'",
                "INSERT INTO ai_insights (
                    insight_id, input_digest, provider, adapter_version,
                    model_label, output_schema_version, output_payload,
                    created_at_unix_ms, expires_at_unix_ms
                 ) VALUES ('ai:forbidden', zeroblob(32), 'test', '1', NULL, 1,
                           x'01', 1, 2)",
                "DELETE FROM cleanup_sessions",
            ] {
                let denied = run_with_retention_guards(connection, || {
                    connection
                        .execute(denied_sql, [])
                        .map(|_| ())
                        .map_err(map_retention_write_error)
                });
                assert!(denied.is_err(), "authorizer accepted {denied_sql}");
            }
            let count: i64 = connection
                .query_row("SELECT count(*) FROM rule_outcomes", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 1);
            let ai_count: i64 = connection
                .query_row("SELECT count(*) FROM ai_insights", [], |row| row.get(0))
                .unwrap();
            assert_eq!(ai_count, 0);
        });
    }

    #[test]
    fn exact_post_commit_state_is_reconciled_for_safe_retry() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let today = 500 * DAY_MS;
        record_raw(
            &store,
            temp.path(),
            "volume:reconcile",
            today - 31 * DAY_MS + 3_600_000,
            1_000,
            500,
            None,
            DiskPressure::Healthy,
        );
        record_raw(
            &store,
            temp.path(),
            "volume:reconcile",
            today + 3_600_000,
            1_000,
            450,
            None,
            DiskPressure::Healthy,
        );
        let result = store
            .run_history_retention_batch_after_commit_failure_for_test(
                UNIX_EPOCH + Duration::from_millis((today + 12 * 3_600_000) as u64),
            )
            .unwrap();
        assert_eq!(result.daily_rollups_created, 1);
        assert_eq!(result.raw_samples_pruned, 1);
        assert_eq!(sample_count(&store, "raw"), 1);
        assert_eq!(sample_count(&store, "daily_rollup"), 1);
    }

    #[test]
    fn pre_epoch_clock_is_rejected_before_any_mutation() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let error = store
            .run_history_retention_batch(UNIX_EPOCH - Duration::from_millis(1))
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::InvalidInput);
        assert_eq!(sample_count(&store, "raw"), 0);
    }

    #[test]
    fn raw_pruning_is_bounded_and_reports_remaining_work() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let today = 900 * DAY_MS;
        let first_sample = today - 40 * DAY_MS;
        for offset in 0..130 {
            record_raw(
                &store,
                temp.path(),
                "volume:bounded",
                first_sample + i64::from(offset) * 3_600_000,
                1_000,
                800,
                Some(700),
                DiskPressure::Healthy,
            );
        }
        record_raw(
            &store,
            temp.path(),
            "volume:bounded",
            today + 3_600_000,
            1_000,
            700,
            Some(600),
            DiskPressure::Healthy,
        );
        let observed = UNIX_EPOCH + Duration::from_millis((today + 12 * 3_600_000) as u64);

        let first = store.run_history_retention_batch(observed).unwrap();
        assert_eq!(first.raw_samples_pruned, 128);
        assert!(first.has_more);
        assert_eq!(sample_count(&store, "raw"), 3);

        let second = store.run_history_retention_batch(observed).unwrap();
        assert_eq!(second.raw_samples_pruned, 2);
        assert!(!second.has_more);
        assert_eq!(sample_count(&store, "raw"), 1);
    }

    #[test]
    fn daily_and_ai_pruning_are_independently_bounded() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let today = 900 * DAY_MS;
        record_raw(
            &store,
            temp.path(),
            "volume:bounded-secondary",
            today - 800 * DAY_MS + 3_600_000,
            1_000,
            800,
            Some(700),
            DiskPressure::Healthy,
        );
        record_raw(
            &store,
            temp.path(),
            "volume:bounded-secondary",
            today + 3_600_000,
            1_000,
            700,
            Some(600),
            DiskPressure::Healthy,
        );
        let observed_ms = today + 12 * 3_600_000;
        store.with_connection(|connection| {
            for offset in 0..130_i64 {
                connection
                    .execute(
                        "INSERT INTO disk_samples (
                            volume_id, sample_kind, sampled_at_unix_ms, total_bytes,
                            available_bytes, important_available_bytes, pressure
                         ) VALUES ('volume:bounded-secondary', 'daily_rollup', ?1,
                                   1000, 750, 650, 'healthy')",
                        [today - (700 - offset) * DAY_MS],
                    )
                    .unwrap();
            }
            for offset in 0..17_u8 {
                connection
                    .execute(
                        "INSERT INTO ai_insights (
                            insight_id, input_digest, provider, adapter_version,
                            model_label, output_schema_version, output_payload,
                            created_at_unix_ms, expires_at_unix_ms
                         ) VALUES (?1, ?2, 'test', '1', NULL, 1, x'01', ?3, ?4)",
                        params![
                            format!("ai:bounded:{offset:02}"),
                            vec![offset; 32],
                            observed_ms - 2,
                            observed_ms - 1,
                        ],
                    )
                    .unwrap();
            }
        });
        let observed = UNIX_EPOCH + Duration::from_millis(observed_ms as u64);

        let first = store.run_history_retention_batch(observed).unwrap();
        assert_eq!(first.daily_rollups_pruned, 128);
        assert_eq!(first.ai_insights_pruned, 16);
        assert!(first.has_more);

        let second = store.run_history_retention_batch(observed).unwrap();
        assert_eq!(second.daily_rollups_pruned, 2);
        assert_eq!(second.ai_insights_pruned, 1);
        assert!(!second.has_more);
        assert_eq!(sample_count(&store, "daily_rollup"), 0);
    }

    #[test]
    fn corrupt_expired_ai_row_rolls_back_rollup_and_raw_pruning() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let today = 700 * DAY_MS;
        let old_sample = today - 31 * DAY_MS + 3_600_000;
        record_raw(
            &store,
            temp.path(),
            "volume:atomic",
            old_sample,
            1_000,
            600,
            Some(500),
            DiskPressure::Healthy,
        );
        record_raw(
            &store,
            temp.path(),
            "volume:atomic",
            today + 3_600_000,
            1_000,
            500,
            Some(400),
            DiskPressure::Healthy,
        );
        let observed_ms = today + 12 * 3_600_000;
        store.with_connection(|connection| {
            connection
                .execute_batch("PRAGMA ignore_check_constraints = ON;")
                .unwrap();
            connection
                .execute(
                    "INSERT INTO ai_insights (
                        insight_id, input_digest, provider, adapter_version,
                        model_label, output_schema_version, output_payload,
                        created_at_unix_ms, expires_at_unix_ms
                     ) VALUES ('ai:corrupt', zeroblob(32), 'test', '1', NULL, 0,
                               x'01', ?1, ?2)",
                    params![observed_ms - 2, observed_ms - 1],
                )
                .unwrap();
            connection
                .execute_batch("PRAGMA ignore_check_constraints = OFF;")
                .unwrap();
        });

        let error = store
            .run_history_retention_batch(UNIX_EPOCH + Duration::from_millis(observed_ms as u64))
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::CorruptData);
        assert_eq!(sample_count(&store, "raw"), 2);
        assert_eq!(sample_count(&store, "daily_rollup"), 0);
    }

    #[test]
    fn malformed_volume_metadata_blocks_retention_atomically() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let today = 700 * DAY_MS;
        record_raw(
            &store,
            temp.path(),
            "volume:malformed-metadata",
            today - 31 * DAY_MS,
            1_000,
            600,
            Some(500),
            DiskPressure::Healthy,
        );
        store.with_connection(|connection| {
            connection
                .execute_batch("PRAGMA ignore_check_constraints = ON;")
                .unwrap();
            connection
                .execute(
                    "UPDATE volumes SET mount_path_encoding = 99
                     WHERE volume_id = 'volume:malformed-metadata'",
                    [],
                )
                .unwrap();
            connection
                .execute_batch("PRAGMA ignore_check_constraints = OFF;")
                .unwrap();
        });

        let error = store
            .run_history_retention_batch(
                UNIX_EPOCH + Duration::from_millis((today + 12 * 3_600_000) as u64),
            )
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::CorruptData);
        assert_eq!(sample_count(&store, "raw"), 1);
        assert_eq!(sample_count(&store, "daily_rollup"), 0);
    }

    #[test]
    fn dense_retained_raw_history_cannot_starve_expired_ai_pruning() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let today = 700 * DAY_MS;
        let first = today - 10 * DAY_MS;
        record_raw(
            &store,
            temp.path(),
            "volume:dense-retained",
            first,
            1_000,
            600,
            Some(500),
            DiskPressure::Healthy,
        );
        record_raw(
            &store,
            temp.path(),
            "volume:dense-retained",
            today + 3_600_000,
            1_000,
            500,
            Some(400),
            DiskPressure::Healthy,
        );
        let observed_ms = today + 12 * 3_600_000;
        store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction().unwrap();
            for offset in 1..=2_048_i64 {
                transaction
                    .execute(
                        "INSERT INTO disk_samples (
                            volume_id, sample_kind, sampled_at_unix_ms, total_bytes,
                            available_bytes, important_available_bytes, pressure
                         ) VALUES ('volume:dense-retained', 'raw', ?1,
                                   1000, 550, 450, 'healthy')",
                        [first + offset],
                    )
                    .unwrap();
            }
            transaction
                .execute(
                    "INSERT INTO ai_insights (
                        insight_id, input_digest, provider, adapter_version,
                        model_label, output_schema_version, output_payload,
                        created_at_unix_ms, expires_at_unix_ms
                     ) VALUES ('ai:dense', zeroblob(32), 'test', '1', NULL, 1,
                               x'01', ?1, ?2)",
                    params![observed_ms - 2, observed_ms - 1],
                )
                .unwrap();
            transaction.commit().unwrap();
        });

        let result = store
            .run_history_retention_batch(UNIX_EPOCH + Duration::from_millis(observed_ms as u64))
            .unwrap();
        assert_eq!(result.raw_samples_pruned, 0);
        assert_eq!(result.ai_insights_pruned, 1);
        assert!(!result.has_more);
        assert_eq!(sample_count(&store, "raw"), 2_050);
    }

    #[test]
    fn external_newer_schema_fences_retention_without_mutation() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let today = 500 * DAY_MS;
        record_raw(
            &store,
            temp.path(),
            "volume:fenced-retention",
            today - 31 * DAY_MS,
            1_000,
            800,
            Some(700),
            DiskPressure::Healthy,
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO schema_migrations
                     (version, name, checksum_sha256, applied_at_unix_ms)
                     VALUES (?1, 'future-retention', zeroblob(32), 2)",
                    [i64::from(DATABASE_SCHEMA_VERSION + 1)],
                )
                .unwrap();
            connection
                .pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION + 1)
                .unwrap();
        });

        let error = store
            .run_history_retention_batch(
                UNIX_EPOCH + Duration::from_millis((today + 12 * 3_600_000) as u64),
            )
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::IncompatibleSchema);
        assert_eq!(sample_count(&store, "raw"), 1);
    }
}
