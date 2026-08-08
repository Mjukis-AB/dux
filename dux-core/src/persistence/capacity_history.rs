//! Typed, non-authoritative volume-capacity observations in SQLite schema v2.
//!
//! These records support presentation, pressure notifications, and trend
//! comparison. Neither a volume observation nor a capacity sample grants
//! cleanup authority.

use std::path::{Path, PathBuf};
use std::time::SystemTime;
#[cfg(test)]
use std::time::{Duration, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::domain::{DiskPressure, DiskPressureEvaluation, VolumeCapacity, VolumeId};

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::history::{
    HistoryError, HistoryErrorKind, from_i64, map_query_sql_error, map_write_sql_error,
    run_bounded_query, stored_bool, system_time_to_unix_ms, to_i64,
    unix_ms_to_system_time_with_kind as unix_ms_to_system_time,
};
use super::pressure_settings::DiskPressurePolicySetting;

const MAX_DISPLAY_NAME_BYTES: usize = 512;
const MAX_FILESYSTEM_BYTES: usize = 128;
const MAX_STORED_ID_BYTES: i64 = 128;
const MAX_STORED_PATH_BYTES: i64 = 65_536;
const MAX_STORED_DISPLAY_NAME_BYTES: i64 = MAX_DISPLAY_NAME_BYTES as i64;
const MAX_STORED_FILESYSTEM_BYTES: i64 = MAX_FILESYSTEM_BYTES as i64;
const MAX_STORED_KIND_BYTES: i64 = 16;
const MAX_STORED_PRESSURE_BYTES: i64 = 16;
const MAX_PAGE_SIZE: usize = 1_024;
const MAX_PRESSURE_EPISODE_PAGE_SIZE: usize = 256;
const UTC_HOUR_MS: i64 = 3_600_000;
const UTC_DAY_MS: i64 = 86_400_000;
const TREND_24_HOURS_MS: i64 = UTC_DAY_MS;
const TREND_7_DAYS_MS: i64 = 7 * UTC_DAY_MS;
const TREND_30_DAYS_MS: i64 = 30 * UTC_DAY_MS;
const MAX_TREND_POINTS: usize = 31;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapacityWriteReason {
    Routine,
    PressureTransition,
    PolicyBaseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapacityWriteOutcome {
    Inserted,
    ExistingExact,
    Suppressed,
}

/// The one non-durable pressure baseline retained by an engine session.
/// Capacity facts accompany the pressure so equal-timestamp durable and
/// session observations can be reconciled deterministically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CapacityPressureBaseline {
    pub(crate) sampled_at: SystemTime,
    pub(crate) capacity: VolumeCapacity,
    pub(crate) pressure: DiskPressure,
    pub(crate) policy_revision: u64,
}

impl CapacityPressureBaseline {
    pub(crate) const fn new(
        sampled_at: SystemTime,
        capacity: VolumeCapacity,
        pressure: DiskPressure,
        policy_revision: u64,
    ) -> Self {
        Self {
            sampled_at,
            capacity,
            pressure,
            policy_revision,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawCapacityObservation {
    volume_id: VolumeId,
    mount_path: PathBuf,
    display_name: String,
    filesystem: String,
    is_internal: bool,
    is_removable: bool,
    sampled_at: SystemTime,
    capacity: VolumeCapacity,
}

impl RawCapacityObservation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new(
        volume_id: VolumeId,
        mount_path: PathBuf,
        display_name: String,
        filesystem: String,
        is_internal: bool,
        is_removable: bool,
        sampled_at: SystemTime,
        capacity: VolumeCapacity,
    ) -> Result<Self, HistoryError> {
        if !mount_path.is_absolute() {
            return Err(invalid());
        }
        encode_host_path(&mount_path).map_err(|_| invalid())?;
        validate_text(
            &display_name,
            MAX_DISPLAY_NAME_BYTES,
            HistoryErrorKind::InvalidInput,
        )?;
        validate_text(
            &filesystem,
            MAX_FILESYSTEM_BYTES,
            HistoryErrorKind::InvalidInput,
        )?;
        let sampled_ms = system_time_to_unix_ms(sampled_at, HistoryErrorKind::InvalidInput)?;
        let sampled_at = unix_ms_to_system_time(sampled_ms, HistoryErrorKind::InvalidInput)?;
        // SQLite's frozen raw schema uses signed integers. Important-only
        // status is never persisted, so it need not be rejected solely for
        // exceeding that storage representation.
        if capacity.available_bytes().is_some() && capacity.total_bytes() > i64::MAX as u64 {
            return Err(invalid());
        }
        Ok(Self {
            volume_id,
            mount_path,
            display_name,
            filesystem,
            is_internal,
            is_removable,
            sampled_at,
            capacity,
        })
    }

    pub(super) fn with_pressure(
        &self,
        pressure: DiskPressure,
        policy_revision: u64,
    ) -> Result<Option<RawCapacitySample>, HistoryError> {
        let Some(available_bytes) = self.capacity.available_bytes() else {
            return Ok(None);
        };
        RawCapacitySample::try_new_with_policy_revision(
            self.volume_id.clone(),
            self.mount_path.clone(),
            self.display_name.clone(),
            self.filesystem.clone(),
            self.is_internal,
            self.is_removable,
            self.sampled_at,
            self.capacity.total_bytes(),
            available_bytes,
            self.capacity.important_available_bytes(),
            pressure,
            policy_revision,
        )
        .map(Some)
    }

    pub(crate) fn volume_id(&self) -> &VolumeId {
        &self.volume_id
    }

    pub(crate) const fn capacity(&self) -> VolumeCapacity {
        self.capacity
    }

    pub(crate) const fn sampled_at(&self) -> SystemTime {
        self.sampled_at
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CapacityObservationOutcome {
    pub(crate) evaluation: DiskPressureEvaluation,
    pub(crate) previous_durable_pressure: Option<DiskPressure>,
    pub(crate) write: Option<CapacityWriteOutcome>,
    pub(crate) effective_policy: DiskPressurePolicySetting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawCapacitySample {
    volume_id: VolumeId,
    mount_path: PathBuf,
    display_name: String,
    filesystem: String,
    is_internal: bool,
    is_removable: bool,
    sampled_at: SystemTime,
    total_bytes: u64,
    available_bytes: u64,
    important_available_bytes: Option<u64>,
    pressure: DiskPressure,
    policy_revision: u64,
}

impl RawCapacitySample {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new(
        volume_id: VolumeId,
        mount_path: PathBuf,
        display_name: String,
        filesystem: String,
        is_internal: bool,
        is_removable: bool,
        sampled_at: SystemTime,
        total_bytes: u64,
        available_bytes: u64,
        important_available_bytes: Option<u64>,
        pressure: DiskPressure,
    ) -> Result<Self, HistoryError> {
        Self::try_new_with_policy_revision(
            volume_id,
            mount_path,
            display_name,
            filesystem,
            is_internal,
            is_removable,
            sampled_at,
            total_bytes,
            available_bytes,
            important_available_bytes,
            pressure,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new_with_policy_revision(
        volume_id: VolumeId,
        mount_path: PathBuf,
        display_name: String,
        filesystem: String,
        is_internal: bool,
        is_removable: bool,
        sampled_at: SystemTime,
        total_bytes: u64,
        available_bytes: u64,
        important_available_bytes: Option<u64>,
        pressure: DiskPressure,
        policy_revision: u64,
    ) -> Result<Self, HistoryError> {
        if !mount_path.is_absolute() {
            return Err(invalid());
        }
        encode_host_path(&mount_path).map_err(|_| invalid())?;
        validate_text(
            &display_name,
            MAX_DISPLAY_NAME_BYTES,
            HistoryErrorKind::InvalidInput,
        )?;
        validate_text(
            &filesystem,
            MAX_FILESYSTEM_BYTES,
            HistoryErrorKind::InvalidInput,
        )?;
        if total_bytes == 0
            || total_bytes > i64::MAX as u64
            || available_bytes > total_bytes
            || important_available_bytes.is_some_and(|available| available > total_bytes)
            || policy_revision > i64::MAX as u64
        {
            return Err(invalid());
        }
        let sampled_ms = system_time_to_unix_ms(sampled_at, HistoryErrorKind::InvalidInput)?;
        let sampled_at = unix_ms_to_system_time(sampled_ms, HistoryErrorKind::InvalidInput)?;
        Ok(Self {
            volume_id,
            mount_path,
            display_name,
            filesystem,
            is_internal,
            is_removable,
            sampled_at,
            total_bytes,
            available_bytes,
            important_available_bytes,
            pressure,
            policy_revision,
        })
    }

    pub(crate) fn volume_id(&self) -> &VolumeId {
        &self.volume_id
    }

    pub(crate) fn mount_path(&self) -> &Path {
        &self.mount_path
    }

    pub(crate) fn sampled_at(&self) -> SystemTime {
        self.sampled_at
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredCapacitySample {
    pub(crate) volume_id: VolumeId,
    pub(crate) sampled_at: SystemTime,
    pub(crate) total_bytes: u64,
    pub(crate) available_bytes: u64,
    pub(crate) important_available_bytes: Option<u64>,
    pub(crate) pressure: DiskPressure,
    pub(crate) policy_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CapacityPageCursor {
    sampled_at_unix_ms: i64,
}

impl CapacityPageCursor {
    pub(crate) fn try_from_sampled_at(sampled_at: SystemTime) -> Result<Self, HistoryError> {
        Ok(Self {
            sampled_at_unix_ms: system_time_to_unix_ms(sampled_at, HistoryErrorKind::InvalidInput)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CapacityPage {
    pub(crate) samples: Vec<StoredCapacitySample>,
    pub(crate) next_cursor: Option<CapacityPageCursor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CapacityChange {
    pub(crate) from: SystemTime,
    pub(crate) to: SystemTime,
    pub(crate) total_bytes: i64,
    pub(crate) available_bytes: i64,
    pub(crate) important_available_bytes: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapacityTrendPointSource {
    Raw,
    DailyRollup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CapacityTrendPoint {
    pub(crate) sample: StoredCapacitySample,
    pub(crate) source: CapacityTrendPointSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CapacityTrend {
    pub(crate) anchor: StoredCapacitySample,
    pub(crate) change_24h: Option<CapacityChange>,
    pub(crate) change_7d: Option<CapacityChange>,
    pub(crate) points: Vec<CapacityTrendPoint>,
}

/// One durable warning/critical pressure episode. Episodes are telemetry
/// history only; they never select a cleanup target or grant mutation
/// authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredPressureEpisode {
    pub(crate) volume_id: VolumeId,
    pub(crate) pressure: DiskPressure,
    pub(crate) entered_at: SystemTime,
    pub(crate) exited_at: Option<SystemTime>,
    pub(crate) policy_revision: u64,
}

pub(super) struct PreparedCapacitySample {
    sample: RawCapacitySample,
    mount_path: EncodedBytes,
    sampled_at_unix_ms: i64,
    total_bytes: i64,
    available_bytes: i64,
    important_available_bytes: Option<i64>,
}

impl PreparedCapacitySample {
    pub(super) fn prepare(sample: &RawCapacitySample) -> Result<Self, HistoryError> {
        // Revalidate at the persistence boundary even though construction is
        // private, so future internal callers cannot bypass stored invariants.
        let canonical = RawCapacitySample::try_new_with_policy_revision(
            sample.volume_id.clone(),
            sample.mount_path.clone(),
            sample.display_name.clone(),
            sample.filesystem.clone(),
            sample.is_internal,
            sample.is_removable,
            sample.sampled_at,
            sample.total_bytes,
            sample.available_bytes,
            sample.important_available_bytes,
            sample.pressure,
            sample.policy_revision,
        )?;
        Ok(Self {
            mount_path: encode_host_path(&canonical.mount_path).map_err(|_| invalid())?,
            sampled_at_unix_ms: system_time_to_unix_ms(
                canonical.sampled_at,
                HistoryErrorKind::InvalidInput,
            )?,
            total_bytes: to_i64(canonical.total_bytes, HistoryErrorKind::InvalidInput)?,
            available_bytes: to_i64(canonical.available_bytes, HistoryErrorKind::InvalidInput)?,
            important_available_bytes: canonical
                .important_available_bytes
                .map(|value| to_i64(value, HistoryErrorKind::InvalidInput))
                .transpose()?,
            sample: canonical,
        })
    }
}

/// Apply admission, monotonic volume metadata, and an optional raw insert in
/// the caller's one immediate transaction.
pub(super) fn write_raw_capacity_sample(
    transaction: &Transaction<'_>,
    prepared: &PreparedCapacitySample,
    reason: CapacityWriteReason,
) -> Result<CapacityWriteOutcome, HistoryError> {
    let volume = load_volume_observation(transaction, prepared.sample.volume_id())?;
    let exact = load_exact_raw_capacity_sample(
        transaction,
        prepared.sample.volume_id(),
        prepared.sample.sampled_at,
    )?;
    if let Some(exact) = exact {
        let Some(volume) = volume.as_ref() else {
            return Err(corrupt());
        };
        if !volume.contains_sampled_ms(prepared.sampled_at_unix_ms) {
            return Err(corrupt());
        }
        if stored_sample_matches(&exact, prepared) && volume.compatible_with_exact_sample(prepared)
        {
            return Ok(CapacityWriteOutcome::ExistingExact);
        }
        return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
    }

    if let Some(volume) = &volume {
        if volume.last_seen_unix_ms > prepared.sampled_at_unix_ms {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        // A missing exact raw row at last_seen means the earlier observation
        // was cadence-suppressed. The volumes row intentionally carries only
        // mutable identity metadata, not the suppressed capacity facts, so an
        // equal-time retry cannot be proven exact after a restart. Fail closed
        // instead of adopting different capacity under the same timestamp.
        if volume.last_seen_unix_ms == prepared.sampled_at_unix_ms {
            return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
        }
    }

    let latest = load_latest_raw_capacity_sample(transaction, prepared.sample.volume_id())?;
    if let Some(latest) = latest.as_ref() {
        let Some(volume) = volume.as_ref() else {
            return Err(corrupt());
        };
        if !volume.contains_sample(latest)? {
            return Err(corrupt());
        }
    }
    if latest.as_ref().is_some_and(|sample| {
        system_time_to_unix_ms(sample.sampled_at, HistoryErrorKind::CorruptData)
            .is_ok_and(|latest_ms| latest_ms >= prepared.sampled_at_unix_ms)
    }) {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }

    let outcome = match reason {
        CapacityWriteReason::Routine => {
            if raw_sample_exists_in_hour(transaction, prepared)? {
                CapacityWriteOutcome::Suppressed
            } else {
                CapacityWriteOutcome::Inserted
            }
        }
        CapacityWriteReason::PressureTransition => match latest.as_ref() {
            Some(sample) if sample.pressure != prepared.sample.pressure => {
                CapacityWriteOutcome::Inserted
            }
            Some(_) => CapacityWriteOutcome::Suppressed,
            None => return Err(HistoryError::new(HistoryErrorKind::InvalidTransition)),
        },
        CapacityWriteReason::PolicyBaseline => CapacityWriteOutcome::Inserted,
    };

    upsert_volume_observation(transaction, prepared, volume.as_ref())?;
    if outcome == CapacityWriteOutcome::Inserted {
        insert_raw_capacity_sample(transaction, prepared)?;
        update_pressure_episode(
            transaction,
            prepared,
            latest.as_ref().map(|sample| sample.pressure),
            reason,
        )?;
    }
    Ok(outcome)
}

/// Update the durable pressure episode state in the same transaction as the
/// raw sample. Unknown pressure never opens or closes an episode. A policy
/// revision boundary is treated as a new observation boundary, so historical
/// episodes retain the revision under which they were classified.
fn update_pressure_episode(
    transaction: &Transaction<'_>,
    prepared: &PreparedCapacitySample,
    previous_pressure: Option<DiskPressure>,
    reason: CapacityWriteReason,
) -> Result<(), HistoryError> {
    let current = load_open_pressure_episode(transaction, prepared.sample.volume_id())?;
    let new_pressure = prepared.sample.pressure;
    let sampled_at = prepared.sampled_at_unix_ms;
    let policy_revision = i64::try_from(prepared.sample.policy_revision)
        .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?;

    match (current, new_pressure) {
        (Some(open), DiskPressure::Healthy) => {
            if sampled_at < open.entered_at_unix_ms {
                return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
            }
            close_pressure_episode(transaction, open.episode_id, sampled_at)?;
        }
        (Some(open), DiskPressure::Warning | DiskPressure::Critical)
            if pressure_as_stored(new_pressure) != open.pressure
                || (reason == CapacityWriteReason::PolicyBaseline
                    && open.policy_revision != policy_revision) =>
        {
            if sampled_at < open.entered_at_unix_ms {
                return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
            }
            close_pressure_episode(transaction, open.episode_id, sampled_at)?;
            insert_pressure_episode(
                transaction,
                prepared.sample.volume_id(),
                new_pressure,
                sampled_at,
                policy_revision,
            )?;
        }
        (None, DiskPressure::Warning | DiskPressure::Critical) => {
            // A v11 store may be upgraded with older non-healthy samples but
            // no episode rows. Start an honest episode at the first durable
            // sample observed by the new schema rather than inventing history.
            let _ = previous_pressure;
            insert_pressure_episode(
                transaction,
                prepared.sample.volume_id(),
                new_pressure,
                sampled_at,
                policy_revision,
            )?;
        }
        (Some(_), DiskPressure::Unknown)
        | (None, DiskPressure::Unknown)
        | (None, DiskPressure::Healthy)
        | (Some(_), DiskPressure::Warning | DiskPressure::Critical) => {}
    }
    Ok(())
}

#[derive(Debug)]
struct OpenPressureEpisode {
    episode_id: i64,
    pressure: String,
    entered_at_unix_ms: i64,
    policy_revision: i64,
}

fn load_open_pressure_episode(
    connection: &Connection,
    volume_id: &VolumeId,
) -> Result<Option<OpenPressureEpisode>, HistoryError> {
    connection
        .query_row(
            "SELECT episode_id, pressure, entered_at_unix_ms, policy_revision
             FROM disk_pressure_episodes
             WHERE volume_id = ?1 AND exited_at_unix_ms IS NULL",
            [volume_id.as_str()],
            |row| {
                Ok(OpenPressureEpisode {
                    episode_id: row.get(0)?,
                    pressure: row.get(1)?,
                    entered_at_unix_ms: row.get(2)?,
                    policy_revision: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(map_query_sql_error)
}

fn close_pressure_episode(
    transaction: &Transaction<'_>,
    episode_id: i64,
    exited_at_unix_ms: i64,
) -> Result<(), HistoryError> {
    let changed = transaction
        .execute(
            "UPDATE disk_pressure_episodes
             SET exited_at_unix_ms = ?2
             WHERE episode_id = ?1 AND exited_at_unix_ms IS NULL",
            params![episode_id, exited_at_unix_ms],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    Ok(())
}

fn insert_pressure_episode(
    transaction: &Transaction<'_>,
    volume_id: &VolumeId,
    pressure: DiskPressure,
    entered_at_unix_ms: i64,
    policy_revision: i64,
) -> Result<(), HistoryError> {
    transaction
        .execute(
            "INSERT INTO disk_pressure_episodes (
                volume_id, pressure, entered_at_unix_ms, policy_revision
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                volume_id.as_str(),
                pressure_as_stored(pressure),
                entered_at_unix_ms,
                policy_revision,
            ],
        )
        .map_err(map_write_sql_error)?;
    Ok(())
}

pub(super) fn load_exact_raw_capacity_sample(
    connection: &Connection,
    volume_id: &VolumeId,
    sampled_at: SystemTime,
) -> Result<Option<StoredCapacitySample>, HistoryError> {
    let sampled_at_unix_ms = system_time_to_unix_ms(sampled_at, HistoryErrorKind::InvalidInput)?;
    run_bounded_query(connection, || {
        load_exact_raw_capacity_sample_with_caller_budget(connection, volume_id, sampled_at_unix_ms)
    })
}

pub(super) fn load_latest_raw_capacity_sample(
    connection: &Connection,
    volume_id: &VolumeId,
) -> Result<Option<StoredCapacitySample>, HistoryError> {
    run_bounded_query(connection, || {
        connection
            .query_row(
                &format!(
                    "{} WHERE volume_id = ?1 AND sample_kind = 'raw' ORDER BY sampled_at_unix_ms DESC LIMIT 1",
                    sample_select()
                ),
                [volume_id.as_str()],
                raw_sample_row,
            )
            .optional()
            .map_err(map_query_sql_error)?
            .map(decode_sample_row)
            .transpose()
    })
}

/// Validate the current metadata row associated with capacity history. Sample
/// pages return only capacity facts, but must still fail closed when their
/// referenced volume observation is malformed or absent.
pub(super) fn validate_capacity_volume(
    connection: &Connection,
    volume_id: &VolumeId,
    samples: &[StoredCapacitySample],
) -> Result<bool, HistoryError> {
    let Some(volume) = load_volume_observation(connection, volume_id)? else {
        return Ok(false);
    };
    for sample in samples {
        if &sample.volume_id != volume_id || !volume.contains_sample(sample)? {
            return Err(corrupt());
        }
    }
    Ok(true)
}

/// Validate the durable volume interval and reject an ephemeral observation
/// that is not strictly newer than its last complete observation.
///
/// `volumes.last_seen` advances for cadence-suppressed observations, while the
/// corresponding capacity facts are deliberately not retained. Consequently
/// an equal timestamp cannot be shown to be an exact retry after an engine
/// restart and must fail closed as a collision.
pub(super) fn validate_ephemeral_capacity_observation(
    connection: &Connection,
    volume_id: &VolumeId,
    samples: &[StoredCapacitySample],
    sampled_at: SystemTime,
) -> Result<bool, HistoryError> {
    let Some(volume) = load_volume_observation(connection, volume_id)? else {
        return Ok(false);
    };
    for sample in samples {
        if &sample.volume_id != volume_id || !volume.contains_sample(sample)? {
            return Err(corrupt());
        }
    }
    let sampled_at_unix_ms = system_time_to_unix_ms(sampled_at, HistoryErrorKind::InvalidInput)?;
    if sampled_at_unix_ms < volume.last_seen_unix_ms {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    if sampled_at_unix_ms == volume.last_seen_unix_ms {
        return Err(HistoryError::new(HistoryErrorKind::AlreadyExists));
    }
    Ok(true)
}

pub(super) fn load_raw_capacity_page(
    connection: &Connection,
    volume_id: &VolumeId,
    cursor: Option<CapacityPageCursor>,
    limit: usize,
) -> Result<CapacityPage, HistoryError> {
    if !(1..=MAX_PAGE_SIZE).contains(&limit) {
        return Err(invalid());
    }
    let sql_limit = i64::try_from(limit + 1).map_err(|_| invalid())?;
    run_bounded_query(connection, || {
        let sql = format!(
            "{} WHERE volume_id = ?1 AND sample_kind = 'raw' AND (?2 IS NULL OR sampled_at_unix_ms < ?2) ORDER BY sampled_at_unix_ms DESC LIMIT ?3",
            sample_select()
        );
        let mut statement = connection.prepare(&sql).map_err(map_query_sql_error)?;
        let rows = statement
            .query_map(
                params![
                    volume_id.as_str(),
                    cursor.map(|cursor| cursor.sampled_at_unix_ms),
                    sql_limit
                ],
                raw_sample_row,
            )
            .map_err(map_query_sql_error)?;
        let mut samples = Vec::with_capacity(limit.min(64));
        for row in rows {
            samples.push(decode_sample_row(row.map_err(map_query_sql_error)?)?);
        }
        let has_more = samples.len() > limit;
        if has_more {
            samples.pop();
        }
        let next_cursor = if has_more {
            samples
                .last()
                .map(|sample| CapacityPageCursor::try_from_sampled_at(sample.sampled_at))
                .transpose()?
        } else {
            None
        };
        Ok(CapacityPage {
            samples,
            next_cursor,
        })
    })
}

/// Build a bounded, deterministic trend view from durable capacity facts.
/// Changes compare the newest raw sample at or before each exact cutoff; chart
/// points choose one sample per UTC day, preferring the exact daily rollup for
/// completed days and the current raw anchor for the current day.
pub(super) fn load_capacity_trend(
    connection: &Connection,
    volume_id: &VolumeId,
    anchor_at: SystemTime,
) -> Result<Option<CapacityTrend>, HistoryError> {
    let anchor_at_ms = system_time_to_unix_ms(anchor_at, HistoryErrorKind::InvalidInput)?;
    let Some(anchor) = load_raw_sample_at_or_before(connection, volume_id, anchor_at_ms)? else {
        return Ok(None);
    };
    if !validate_capacity_volume(connection, volume_id, std::slice::from_ref(&anchor))? {
        return Err(corrupt());
    }
    let anchor_ms = system_time_to_unix_ms(anchor.sampled_at, HistoryErrorKind::CorruptData)?;
    let change_24h = load_capacity_change(
        connection,
        volume_id,
        &anchor,
        anchor_ms
            .checked_sub(TREND_24_HOURS_MS)
            .ok_or_else(corrupt)?,
    )?;
    let change_7d = load_capacity_change(
        connection,
        volume_id,
        &anchor,
        anchor_ms.checked_sub(TREND_7_DAYS_MS).ok_or_else(corrupt)?,
    )?;

    let chart_start = anchor_ms
        .checked_sub(TREND_30_DAYS_MS)
        .ok_or_else(corrupt)?;
    let mut points = load_trend_points(connection, volume_id, chart_start, anchor_ms)?;
    if points.len() > MAX_TREND_POINTS {
        return Err(corrupt());
    }
    Ok(Some(CapacityTrend {
        anchor,
        change_24h,
        change_7d,
        points: std::mem::take(&mut points),
    }))
}

fn load_capacity_change(
    connection: &Connection,
    volume_id: &VolumeId,
    anchor: &StoredCapacitySample,
    cutoff_ms: i64,
) -> Result<Option<CapacityChange>, HistoryError> {
    let Some(baseline) = load_raw_sample_at_or_before(connection, volume_id, cutoff_ms)? else {
        return Ok(None);
    };
    if !validate_capacity_volume(connection, volume_id, std::slice::from_ref(&baseline))? {
        return Err(corrupt());
    }
    let from = system_time_to_unix_ms(baseline.sampled_at, HistoryErrorKind::CorruptData)?;
    let to = system_time_to_unix_ms(anchor.sampled_at, HistoryErrorKind::CorruptData)?;
    if from > to {
        return Err(corrupt());
    }
    Ok(Some(CapacityChange {
        from: baseline.sampled_at,
        to: anchor.sampled_at,
        total_bytes: signed_delta(anchor.total_bytes, baseline.total_bytes)?,
        available_bytes: signed_delta(anchor.available_bytes, baseline.available_bytes)?,
        important_available_bytes: anchor
            .important_available_bytes
            .zip(baseline.important_available_bytes)
            .map(|(current, previous)| signed_delta(current, previous))
            .transpose()?,
    }))
}

fn load_raw_sample_at_or_before(
    connection: &Connection,
    volume_id: &VolumeId,
    at_or_before_ms: i64,
) -> Result<Option<StoredCapacitySample>, HistoryError> {
    run_bounded_query(connection, || {
        load_raw_sample_at_or_before_with_caller_budget(connection, volume_id, at_or_before_ms)
    })
}

fn load_trend_points(
    connection: &Connection,
    volume_id: &VolumeId,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<CapacityTrendPoint>, HistoryError> {
    let mut points = std::collections::BTreeMap::<i64, CapacityTrendPoint>::new();
    for (kind, source) in [
        ("daily_rollup", CapacityTrendPointSource::DailyRollup),
        ("raw", CapacityTrendPointSource::Raw),
    ] {
        let rows = run_bounded_query(connection, || {
            let sql = if kind == "raw" {
                format!(
                    "{} WHERE volume_id = ?1 AND sample_kind = 'raw'
                     AND sampled_at_unix_ms >= ?2 AND sampled_at_unix_ms <= ?3
                     AND sampled_at_unix_ms IN (
                         SELECT MAX(sampled_at_unix_ms) FROM disk_samples
                         WHERE volume_id = ?1 AND sample_kind = 'raw'
                           AND sampled_at_unix_ms >= ?2 AND sampled_at_unix_ms <= ?3
                         GROUP BY sampled_at_unix_ms / {UTC_DAY_MS}
                     )
                     ORDER BY sampled_at_unix_ms DESC LIMIT ?4",
                    sample_select()
                )
            } else {
                format!(
                    "{} WHERE volume_id = ?1 AND sample_kind = 'daily_rollup'
                     AND sampled_at_unix_ms >= ?2 AND sampled_at_unix_ms <= ?3
                     ORDER BY sampled_at_unix_ms DESC LIMIT ?4",
                    sample_select()
                )
            };
            let mut statement = connection.prepare(&sql).map_err(map_query_sql_error)?;
            let rows = statement
                .query_map(
                    params![
                        volume_id.as_str(),
                        start_ms,
                        end_ms,
                        MAX_TREND_POINTS as i64
                    ],
                    raw_sample_row,
                )
                .map_err(map_query_sql_error)?;
            let mut decoded = Vec::with_capacity(MAX_TREND_POINTS);
            for row in rows {
                decoded.push(decode_sample_row_kind(
                    row.map_err(map_query_sql_error)?,
                    kind,
                )?);
            }
            Ok(decoded)
        })?;
        for sample in rows {
            if !validate_capacity_volume(connection, volume_id, std::slice::from_ref(&sample))? {
                return Err(corrupt());
            }
            let sampled_ms =
                system_time_to_unix_ms(sample.sampled_at, HistoryErrorKind::CorruptData)?;
            let day = sampled_ms / UTC_DAY_MS;
            let candidate = CapacityTrendPoint { sample, source };
            let replace = match points.get(&day) {
                None => true,
                Some(existing) if day == end_ms / UTC_DAY_MS => {
                    candidate.source == CapacityTrendPointSource::Raw
                        && (existing.source != CapacityTrendPointSource::Raw
                            || candidate.sample.sampled_at > existing.sample.sampled_at)
                }
                Some(existing) => {
                    (candidate.source == CapacityTrendPointSource::DailyRollup
                        && existing.source == CapacityTrendPointSource::Raw)
                        || (candidate.source == existing.source
                            && candidate.sample.sampled_at > existing.sample.sampled_at)
                }
            };
            if replace {
                points.insert(day, candidate);
            }
        }
    }
    Ok(points.into_values().collect())
}

fn signed_delta(current: u64, previous: u64) -> Result<i64, HistoryError> {
    i64::try_from(i128::from(current) - i128::from(previous)).map_err(|_| corrupt())
}

/// Load a bounded newest-first page of pressure episodes for one volume. The
/// query validates every returned row and rejects overlapping or malformed
/// state before exposing it to later trend/notification consumers.
pub(super) fn load_pressure_episode_page(
    connection: &Connection,
    volume_id: &VolumeId,
    limit: usize,
) -> Result<Vec<StoredPressureEpisode>, HistoryError> {
    load_pressure_episode_page_at(connection, volume_id, None, limit)
}

/// Load pressure history as it was known at one observation anchor. Episodes
/// that start later are excluded; callers may project an exit after the anchor
/// as still open at that point in time.
pub(super) fn load_pressure_episode_page_at_anchor(
    connection: &Connection,
    volume_id: &VolumeId,
    anchor_at: SystemTime,
    limit: usize,
) -> Result<Vec<StoredPressureEpisode>, HistoryError> {
    let anchor_at_unix_ms = system_time_to_unix_ms(anchor_at, HistoryErrorKind::InvalidInput)?;
    load_pressure_episode_page_at(connection, volume_id, Some(anchor_at_unix_ms), limit)
}

pub(super) fn load_volume_mount_path_at_anchor(
    connection: &Connection,
    volume_id: &VolumeId,
    anchor_at: SystemTime,
) -> Result<PathBuf, HistoryError> {
    let anchor_at_unix_ms = system_time_to_unix_ms(anchor_at, HistoryErrorKind::InvalidInput)?;
    run_bounded_query(connection, || {
        let volume = load_volume_observation_with_caller_budget(connection, volume_id)?
            .ok_or_else(corrupt)?;
        if anchor_at_unix_ms != volume.last_seen_unix_ms {
            return Err(invalid());
        }
        validate_observation_anchor_within_budget(
            connection,
            volume_id,
            &volume,
            anchor_at_unix_ms,
        )?;
        decode_host_path(&volume.mount_path).map_err(|_| corrupt())
    })
}

fn load_pressure_episode_page_at(
    connection: &Connection,
    volume_id: &VolumeId,
    anchor_at_unix_ms: Option<i64>,
    limit: usize,
) -> Result<Vec<StoredPressureEpisode>, HistoryError> {
    if !(1..=MAX_PRESSURE_EPISODE_PAGE_SIZE).contains(&limit) {
        return Err(invalid());
    }
    let sql_limit = i64::try_from(limit + 1).map_err(|_| invalid())?;
    run_bounded_query(connection, || {
        let volume = load_volume_observation_with_caller_budget(connection, volume_id)?
            .ok_or_else(corrupt)?;
        if let Some(anchor) = anchor_at_unix_ms {
            validate_observation_anchor_within_budget(connection, volume_id, &volume, anchor)?;
        }
        let mut statement = connection
            .prepare(
                "SELECT volume_id, pressure, entered_at_unix_ms, exited_at_unix_ms,
                        policy_revision
                 FROM disk_pressure_episodes
                 WHERE volume_id = ?1
                   AND (?3 IS NULL OR entered_at_unix_ms <= ?3)
                 ORDER BY entered_at_unix_ms DESC, episode_id DESC
                 LIMIT ?2",
            )
            .map_err(map_query_sql_error)?;
        let rows = statement
            .query_map(
                params![volume_id.as_str(), sql_limit, anchor_at_unix_ms],
                |row| {
                    let volume_id_text: String = row.get(0)?;
                    let pressure: String = row.get(1)?;
                    let entered_at_unix_ms: i64 = row.get(2)?;
                    let exited_at_unix_ms: Option<i64> = row.get(3)?;
                    let policy_revision: i64 = row.get(4)?;
                    Ok((
                        volume_id_text,
                        pressure,
                        entered_at_unix_ms,
                        exited_at_unix_ms,
                        policy_revision,
                    ))
                },
            )
            .map_err(map_query_sql_error)?;
        let mut episodes = Vec::with_capacity(limit.min(64));
        for row in rows {
            let (volume_id_text, pressure, entered_at_unix_ms, exited_at_unix_ms, policy_revision) =
                row.map_err(map_query_sql_error)?;
            let decoded_volume_id = VolumeId::new(volume_id_text).map_err(|_| corrupt())?;
            if decoded_volume_id != *volume_id
                || entered_at_unix_ms < 0
                || policy_revision < 0
                || exited_at_unix_ms.is_some_and(|value| value < entered_at_unix_ms)
            {
                return Err(corrupt());
            }
            episodes.push(StoredPressureEpisode {
                volume_id: decoded_volume_id,
                pressure: pressure_from_stored(&pressure)?,
                entered_at: unix_ms_to_system_time(
                    entered_at_unix_ms,
                    HistoryErrorKind::CorruptData,
                )?,
                exited_at: exited_at_unix_ms
                    .map(|value| unix_ms_to_system_time(value, HistoryErrorKind::CorruptData))
                    .transpose()?,
                policy_revision: u64::try_from(policy_revision).map_err(|_| corrupt())?,
            });
        }
        for episode in &episodes {
            let entered_at_unix_ms =
                system_time_to_unix_ms(episode.entered_at, HistoryErrorKind::CorruptData)?;
            let exited_at_unix_ms = episode
                .exited_at
                .map(|value| system_time_to_unix_ms(value, HistoryErrorKind::CorruptData))
                .transpose()?;
            if !volume.contains_sampled_ms(entered_at_unix_ms)
                || exited_at_unix_ms.is_some_and(|value| !volume.contains_sampled_ms(value))
            {
                return Err(corrupt());
            }
        }
        // Validate the lookahead row too, so corruption exactly across the
        // requested-page boundary cannot be hidden by truncation.
        for (index, episode) in episodes.iter().enumerate() {
            if index > 0 && episode.exited_at.is_none() {
                return Err(corrupt());
            }
        }
        for pair in episodes.windows(2) {
            if pair[0].entered_at <= pair[1].entered_at
                || pair[1]
                    .exited_at
                    .is_none_or(|older_exit| older_exit > pair[0].entered_at)
            {
                return Err(corrupt());
            }
        }
        episodes.truncate(limit);
        Ok(episodes)
    })
}

fn validate_observation_anchor_within_budget(
    connection: &Connection,
    volume_id: &VolumeId,
    volume: &StoredVolumeObservation,
    anchor: i64,
) -> Result<(), HistoryError> {
    if !volume.contains_sampled_ms(anchor) {
        return Err(invalid());
    }
    // Every raw observation is a durable accepted anchor. Routine
    // observations suppressed by the hourly cadence are represented only by
    // the current volume row's exact last_seen timestamp.
    match load_exact_raw_capacity_sample_with_caller_budget(connection, volume_id, anchor)? {
        Some(sample) => {
            if sample.volume_id != *volume_id || !volume.contains_sample(&sample)? {
                return Err(corrupt());
            }
        }
        None if anchor != volume.last_seen_unix_ms => return Err(invalid()),
        None => {
            let Some(sample) =
                load_raw_sample_at_or_before_with_caller_budget(connection, volume_id, anchor)?
            else {
                return Err(corrupt());
            };
            if sample.volume_id != *volume_id || !volume.contains_sample(&sample)? {
                return Err(corrupt());
            }
        }
    }
    Ok(())
}

/// Reconcile a post-commit failure for outcomes that created or adopted an
/// exact raw row. An older retry compares its immutable raw facts and verifies
/// that its timestamp remains inside the volume's observation interval; newer
/// mutable volume labels are not historical facts. Suppressed writes are
/// reconciled by StoreCoordinator while it still knows the admission result.
pub(super) fn exact_raw_and_volume_match(
    connection: &Connection,
    prepared: &PreparedCapacitySample,
) -> Result<bool, HistoryError> {
    let sample = load_exact_raw_capacity_sample(
        connection,
        prepared.sample.volume_id(),
        prepared.sample.sampled_at,
    )?;
    let volume = load_volume_observation(connection, prepared.sample.volume_id())?;
    let Some(sample) = sample.as_ref() else {
        return Ok(false);
    };
    let Some(volume) = volume.as_ref() else {
        return Err(corrupt());
    };
    if !volume.contains_sample(sample)? {
        return Err(corrupt());
    }
    Ok(stored_sample_matches(sample, prepared) && volume.compatible_with_exact_sample(prepared))
}

/// Reconcile a suppressed write, which advances only the current volume
/// observation. The caller holds the writer lease, so an exact last-seen value
/// cannot be displaced by another process during this check.
pub(super) fn exact_volume_observation_match(
    connection: &Connection,
    prepared: &PreparedCapacitySample,
) -> Result<bool, HistoryError> {
    Ok(
        load_volume_observation(connection, prepared.sample.volume_id())?
            .as_ref()
            .is_some_and(|volume| volume.exactly_matches(prepared)),
    )
}

fn insert_raw_capacity_sample(
    transaction: &Transaction<'_>,
    prepared: &PreparedCapacitySample,
) -> Result<(), HistoryError> {
    let changed = transaction
        .execute(
            "INSERT INTO disk_samples (
                volume_id, sample_kind, sampled_at_unix_ms, total_bytes,
                available_bytes, important_available_bytes, pressure,
                policy_revision
             ) VALUES (?1, 'raw', ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                prepared.sample.volume_id.as_str(),
                prepared.sampled_at_unix_ms,
                prepared.total_bytes,
                prepared.available_bytes,
                prepared.important_available_bytes,
                pressure_as_stored(prepared.sample.pressure),
                i64::try_from(prepared.sample.policy_revision)
                    .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?,
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::InternalState));
    }
    Ok(())
}

fn raw_sample_exists_in_hour(
    transaction: &Transaction<'_>,
    prepared: &PreparedCapacitySample,
) -> Result<bool, HistoryError> {
    let hour_start = prepared.sampled_at_unix_ms / UTC_HOUR_MS * UTC_HOUR_MS;
    let hour_end = hour_start.checked_add(UTC_HOUR_MS).ok_or_else(invalid)?;
    transaction
        .query_row(
            "SELECT EXISTS (
                SELECT 1 FROM disk_samples
                WHERE volume_id = ?1 AND sample_kind = 'raw'
                  AND sampled_at_unix_ms >= ?2 AND sampled_at_unix_ms < ?3
             )",
            params![prepared.sample.volume_id.as_str(), hour_start, hour_end],
            |row| row.get::<_, i64>(0),
        )
        .map_err(map_query_sql_error)
        .and_then(stored_bool)
}

#[derive(Debug)]
struct StoredVolumeObservation {
    mount_path: EncodedBytes,
    display_name: String,
    filesystem: String,
    is_internal: bool,
    is_removable: bool,
    first_seen_unix_ms: i64,
    last_seen_unix_ms: i64,
}

impl StoredVolumeObservation {
    fn contains_sampled_ms(&self, sampled_at_unix_ms: i64) -> bool {
        (self.first_seen_unix_ms..=self.last_seen_unix_ms).contains(&sampled_at_unix_ms)
    }

    fn contains_sample(&self, sample: &StoredCapacitySample) -> Result<bool, HistoryError> {
        let sampled_at_unix_ms =
            system_time_to_unix_ms(sample.sampled_at, HistoryErrorKind::CorruptData)?;
        Ok(self.contains_sampled_ms(sampled_at_unix_ms))
    }

    fn exactly_matches(&self, prepared: &PreparedCapacitySample) -> bool {
        self.last_seen_unix_ms == prepared.sampled_at_unix_ms
            && self.mount_path == prepared.mount_path
            && self.display_name == prepared.sample.display_name
            && self.filesystem == prepared.sample.filesystem
            && self.is_internal == prepared.sample.is_internal
            && self.is_removable == prepared.sample.is_removable
    }

    fn compatible_with_exact_sample(&self, prepared: &PreparedCapacitySample) -> bool {
        self.contains_sampled_ms(prepared.sampled_at_unix_ms)
            && (self.last_seen_unix_ms > prepared.sampled_at_unix_ms
                || self.exactly_matches(prepared))
    }
}

fn upsert_volume_observation(
    transaction: &Transaction<'_>,
    prepared: &PreparedCapacitySample,
    existing: Option<&StoredVolumeObservation>,
) -> Result<(), HistoryError> {
    match existing {
        None => {
            transaction
                .execute(
                    "INSERT INTO volumes (
                        volume_id, mount_path, mount_path_encoding, display_name,
                        filesystem, is_internal, is_removable,
                        first_seen_unix_ms, last_seen_unix_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    params![
                        prepared.sample.volume_id.as_str(),
                        prepared.mount_path.bytes,
                        prepared.mount_path.encoding as i64,
                        prepared.sample.display_name,
                        prepared.sample.filesystem,
                        i64::from(prepared.sample.is_internal),
                        i64::from(prepared.sample.is_removable),
                        prepared.sampled_at_unix_ms,
                    ],
                )
                .map_err(map_write_sql_error)?;
        }
        Some(existing) if existing.last_seen_unix_ms < prepared.sampled_at_unix_ms => {
            let changed = transaction
                .execute(
                    "UPDATE volumes SET
                        mount_path = ?2, mount_path_encoding = ?3,
                        display_name = ?4, filesystem = ?5,
                        is_internal = ?6, is_removable = ?7,
                        last_seen_unix_ms = ?8
                     WHERE volume_id = ?1 AND last_seen_unix_ms < ?8",
                    params![
                        prepared.sample.volume_id.as_str(),
                        prepared.mount_path.bytes,
                        prepared.mount_path.encoding as i64,
                        prepared.sample.display_name,
                        prepared.sample.filesystem,
                        i64::from(prepared.sample.is_internal),
                        i64::from(prepared.sample.is_removable),
                        prepared.sampled_at_unix_ms,
                    ],
                )
                .map_err(map_write_sql_error)?;
            if changed != 1 {
                return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
            }
        }
        Some(_) => {}
    }
    Ok(())
}

fn load_volume_observation(
    connection: &Connection,
    volume_id: &VolumeId,
) -> Result<Option<StoredVolumeObservation>, HistoryError> {
    run_bounded_query(connection, || {
        load_volume_observation_with_caller_budget(connection, volume_id)
    })
}

/// Load and fully validate one capacity-volume observation while relying on an
/// already-installed caller budget. Retention uses this instead of nesting the
/// ordinary capacity-query progress handler inside its transaction-wide guard.
pub(super) fn load_capacity_volume_interval_with_caller_budget(
    connection: &Connection,
    volume_id: &VolumeId,
) -> Result<Option<(i64, i64)>, HistoryError> {
    load_volume_observation_with_caller_budget(connection, volume_id)
        .map(|volume| volume.map(|volume| (volume.first_seen_unix_ms, volume.last_seen_unix_ms)))
}

fn load_volume_observation_with_caller_budget(
    connection: &Connection,
    volume_id: &VolumeId,
) -> Result<Option<StoredVolumeObservation>, HistoryError> {
    connection
        .query_row(
            "SELECT
                typeof(mount_path), length(mount_path), mount_path,
                typeof(mount_path_encoding), mount_path_encoding,
                typeof(display_name), length(CAST(display_name AS BLOB)), display_name,
                typeof(filesystem), length(CAST(filesystem AS BLOB)), filesystem,
                typeof(is_internal), is_internal,
                typeof(is_removable), is_removable,
                typeof(first_seen_unix_ms), first_seen_unix_ms,
                typeof(last_seen_unix_ms), last_seen_unix_ms
             FROM volumes WHERE volume_id = ?1",
            [volume_id.as_str()],
            raw_volume_row,
        )
        .optional()
        .map_err(map_query_sql_error)?
        .map(decode_volume_row)
        .transpose()
}

fn load_exact_raw_capacity_sample_with_caller_budget(
    connection: &Connection,
    volume_id: &VolumeId,
    sampled_at_unix_ms: i64,
) -> Result<Option<StoredCapacitySample>, HistoryError> {
    connection
        .query_row(
            &format!(
                "{} WHERE volume_id = ?1 AND sample_kind = 'raw' AND sampled_at_unix_ms = ?2",
                sample_select()
            ),
            params![volume_id.as_str(), sampled_at_unix_ms],
            raw_sample_row,
        )
        .optional()
        .map_err(map_query_sql_error)?
        .map(decode_sample_row)
        .transpose()
}

fn load_raw_sample_at_or_before_with_caller_budget(
    connection: &Connection,
    volume_id: &VolumeId,
    at_or_before_ms: i64,
) -> Result<Option<StoredCapacitySample>, HistoryError> {
    connection
        .query_row(
            &format!(
                "{} WHERE volume_id = ?1 AND sample_kind = 'raw'
                 AND sampled_at_unix_ms <= ?2
                 ORDER BY sampled_at_unix_ms DESC LIMIT 1",
                sample_select()
            ),
            params![volume_id.as_str(), at_or_before_ms],
            raw_sample_row,
        )
        .optional()
        .map_err(map_query_sql_error)?
        .map(decode_sample_row)
        .transpose()
}

struct RawVolumeRow {
    mount_path: Vec<u8>,
    mount_path_encoding: i64,
    display_name: String,
    filesystem: String,
    is_internal: i64,
    is_removable: i64,
    first_seen_unix_ms: i64,
    last_seen_unix_ms: i64,
}

fn raw_volume_row(row: &Row<'_>) -> rusqlite::Result<RawVolumeRow> {
    validate_type_length(row, 0, 1, "blob", 1, MAX_STORED_PATH_BYTES)?;
    validate_type(row, 3, "integer")?;
    validate_type_length(row, 5, 6, "text", 1, MAX_STORED_DISPLAY_NAME_BYTES)?;
    validate_type_length(row, 8, 9, "text", 1, MAX_STORED_FILESYSTEM_BYTES)?;
    for column in [11, 13, 15, 17] {
        validate_type(row, column, "integer")?;
    }
    Ok(RawVolumeRow {
        mount_path: row.get(2)?,
        mount_path_encoding: row.get(4)?,
        display_name: row.get(7)?,
        filesystem: row.get(10)?,
        is_internal: row.get(12)?,
        is_removable: row.get(14)?,
        first_seen_unix_ms: row.get(16)?,
        last_seen_unix_ms: row.get(18)?,
    })
}

fn decode_volume_row(raw: RawVolumeRow) -> Result<StoredVolumeObservation, HistoryError> {
    let encoding =
        StoredEncoding::host_path_from_stored(raw.mount_path_encoding).map_err(|_| corrupt())?;
    let mount_path = EncodedBytes {
        bytes: raw.mount_path,
        encoding,
    };
    let decoded = decode_host_path(&mount_path).map_err(|_| corrupt())?;
    if !decoded.is_absolute() {
        return Err(corrupt());
    }
    validate_text(
        &raw.display_name,
        MAX_DISPLAY_NAME_BYTES,
        HistoryErrorKind::CorruptData,
    )?;
    validate_text(
        &raw.filesystem,
        MAX_FILESYSTEM_BYTES,
        HistoryErrorKind::CorruptData,
    )?;
    if raw.first_seen_unix_ms < 0 || raw.last_seen_unix_ms < raw.first_seen_unix_ms {
        return Err(corrupt());
    }
    Ok(StoredVolumeObservation {
        mount_path,
        display_name: raw.display_name,
        filesystem: raw.filesystem,
        is_internal: stored_bool(raw.is_internal)?,
        is_removable: stored_bool(raw.is_removable)?,
        first_seen_unix_ms: raw.first_seen_unix_ms,
        last_seen_unix_ms: raw.last_seen_unix_ms,
    })
}

struct RawSampleRow {
    volume_id: String,
    sample_kind: String,
    sampled_at_unix_ms: i64,
    total_bytes: i64,
    available_bytes: i64,
    important_available_bytes: Option<i64>,
    pressure: String,
    policy_revision: i64,
}

fn sample_select() -> &'static str {
    "SELECT
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

fn raw_sample_row(row: &Row<'_>) -> rusqlite::Result<RawSampleRow> {
    validate_type_length(row, 0, 1, "text", 1, MAX_STORED_ID_BYTES)?;
    validate_type_length(row, 3, 4, "text", 1, MAX_STORED_KIND_BYTES)?;
    for column in [6, 8, 10] {
        validate_type(row, column, "integer")?;
    }
    let important_type: String = row.get(12)?;
    if important_type != "null" && important_type != "integer" {
        return Err(rusqlite::Error::InvalidQuery);
    }
    validate_type_length(row, 14, 15, "text", 1, MAX_STORED_PRESSURE_BYTES)?;
    validate_type(row, 17, "integer")?;
    Ok(RawSampleRow {
        volume_id: row.get(2)?,
        sample_kind: row.get(5)?,
        sampled_at_unix_ms: row.get(7)?,
        total_bytes: row.get(9)?,
        available_bytes: row.get(11)?,
        important_available_bytes: row.get(13)?,
        pressure: row.get(16)?,
        policy_revision: row.get(18)?,
    })
}

fn decode_sample_row(raw: RawSampleRow) -> Result<StoredCapacitySample, HistoryError> {
    decode_sample_row_kind(raw, "raw")
}

fn decode_sample_row_kind(
    raw: RawSampleRow,
    expected_kind: &str,
) -> Result<StoredCapacitySample, HistoryError> {
    if raw.sample_kind != expected_kind {
        return Err(corrupt());
    }
    let volume_id = VolumeId::new(raw.volume_id).map_err(|_| corrupt())?;
    let sampled_at = unix_ms_to_system_time(raw.sampled_at_unix_ms, HistoryErrorKind::CorruptData)?;
    let total_bytes = from_i64(raw.total_bytes)?;
    let available_bytes = from_i64(raw.available_bytes)?;
    let important_available_bytes = raw.important_available_bytes.map(from_i64).transpose()?;
    if total_bytes == 0
        || available_bytes > total_bytes
        || important_available_bytes.is_some_and(|available| available > total_bytes)
        || raw.policy_revision < 0
    {
        return Err(corrupt());
    }
    Ok(StoredCapacitySample {
        volume_id,
        sampled_at,
        total_bytes,
        available_bytes,
        important_available_bytes,
        pressure: pressure_from_stored(&raw.pressure)?,
        policy_revision: u64::try_from(raw.policy_revision).map_err(|_| corrupt())?,
    })
}

fn stored_sample_matches(stored: &StoredCapacitySample, prepared: &PreparedCapacitySample) -> bool {
    stored.volume_id == prepared.sample.volume_id
        && stored.sampled_at == prepared.sample.sampled_at
        && stored.total_bytes == prepared.sample.total_bytes
        && stored.available_bytes == prepared.sample.available_bytes
        && stored.important_available_bytes == prepared.sample.important_available_bytes
        && stored.pressure == prepared.sample.pressure
        && stored.policy_revision == prepared.sample.policy_revision
}

fn pressure_as_stored(pressure: DiskPressure) -> &'static str {
    match pressure {
        DiskPressure::Healthy => "healthy",
        DiskPressure::Warning => "warning",
        DiskPressure::Critical => "critical",
        DiskPressure::Unknown => "unknown",
    }
}

fn pressure_from_stored(value: &str) -> Result<DiskPressure, HistoryError> {
    match value {
        "healthy" => Ok(DiskPressure::Healthy),
        "warning" => Ok(DiskPressure::Warning),
        "critical" => Ok(DiskPressure::Critical),
        "unknown" => Ok(DiskPressure::Unknown),
        _ => Err(corrupt()),
    }
}

fn validate_text(value: &str, maximum: usize, kind: HistoryErrorKind) -> Result<(), HistoryError> {
    if value.trim().is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(HistoryError::new(kind));
    }
    Ok(())
}

fn validate_type(row: &Row<'_>, column: usize, expected: &str) -> rusqlite::Result<()> {
    if row.get::<_, String>(column)? != expected {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn validate_type_length(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
    expected_type: &str,
    minimum: i64,
    maximum: i64,
) -> rusqlite::Result<()> {
    validate_type(row, type_column, expected_type)?;
    let length: i64 = row.get(length_column)?;
    if !(minimum..=maximum).contains(&length) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

const fn invalid() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidInput)
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
#[path = "capacity_history/tests.rs"]
mod tests;
