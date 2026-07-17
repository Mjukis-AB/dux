//! Typed, non-authoritative volume-capacity observations in SQLite schema v2.
//!
//! These records support presentation, pressure notifications, and trend
//! comparison. Neither a volume observation nor a capacity sample grants
//! cleanup authority.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::domain::{DiskPressure, DiskPressureEvaluation, VolumeCapacity, VolumeId};

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
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
const UTC_HOUR_MS: i64 = 3_600_000;

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
    }
    Ok(outcome)
}

pub(super) fn load_exact_raw_capacity_sample(
    connection: &Connection,
    volume_id: &VolumeId,
    sampled_at: SystemTime,
) -> Result<Option<StoredCapacitySample>, HistoryError> {
    let sampled_at_unix_ms = system_time_to_unix_ms(sampled_at, HistoryErrorKind::InvalidInput)?;
    run_bounded_query(connection, || {
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
    let encoding = stored_host_encoding(raw.mount_path_encoding)?;
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
    if raw.sample_kind != "raw" {
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

fn stored_host_encoding(value: i64) -> Result<StoredEncoding, HistoryError> {
    match value {
        1 => Ok(StoredEncoding::Utf8HostPath),
        2 => Ok(StoredEncoding::Utf16LeHostPath),
        _ => Err(corrupt()),
    }
}

fn system_time_to_unix_ms(value: SystemTime, kind: HistoryErrorKind) -> Result<i64, HistoryError> {
    let milliseconds = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HistoryError::new(kind))?
        .as_millis();
    i64::try_from(milliseconds).map_err(|_| HistoryError::new(kind))
}

fn unix_ms_to_system_time(value: i64, kind: HistoryErrorKind) -> Result<SystemTime, HistoryError> {
    let milliseconds = u64::try_from(value).map_err(|_| HistoryError::new(kind))?;
    UNIX_EPOCH
        .checked_add(Duration::from_millis(milliseconds))
        .ok_or_else(|| HistoryError::new(kind))
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

const fn invalid() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidInput)
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use crate::DATABASE_SCHEMA_VERSION;
    use crate::persistence::StoreCoordinator;
    use tempfile::TempDir;

    const BASE_HOUR_MS: u64 = 1_800_000_000_000;

    fn sample(pressure: DiskPressure) -> RawCapacitySample {
        sample_at("volume:test", BASE_HOUR_MS + 123, 200, None, pressure)
    }

    fn sample_at(
        volume_id: &str,
        sampled_at_unix_ms: u64,
        available_bytes: u64,
        important_available_bytes: Option<u64>,
        pressure: DiskPressure,
    ) -> RawCapacitySample {
        RawCapacitySample::try_new(
            VolumeId::new(volume_id).unwrap(),
            PathBuf::from("/"),
            "Startup".to_owned(),
            "apfs".to_owned(),
            true,
            false,
            UNIX_EPOCH + Duration::from_millis(sampled_at_unix_ms),
            1_000,
            available_bytes,
            important_available_bytes,
            pressure,
        )
        .unwrap()
    }

    fn observation_at(sampled_at_unix_ms: u64, available_bytes: u64) -> RawCapacityObservation {
        RawCapacityObservation::try_new(
            VolumeId::new("volume:observed").unwrap(),
            PathBuf::from("/"),
            "Startup".to_owned(),
            "apfs".to_owned(),
            true,
            false,
            UNIX_EPOCH + Duration::from_millis(sampled_at_unix_ms),
            VolumeCapacity::new(1_024 * 1_024 * 1_024 * 1_024, Some(available_bytes), None)
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn atomic_observation_reconciles_ambiguous_insert_exact_suppression_and_transition() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let gib = 1_024 * 1_024 * 1_024;
        let first = observation_at(BASE_HOUR_MS, 100 * gib);
        let inserted = store
            .observe_capacity_after_commit_failure_for_test(&first)
            .unwrap();
        assert_eq!(inserted.write, Some(CapacityWriteOutcome::Inserted));

        let exact = store
            .observe_capacity_after_commit_failure_for_test(&first)
            .unwrap();
        assert_eq!(exact.write, Some(CapacityWriteOutcome::ExistingExact));

        let routine = observation_at(BASE_HOUR_MS + 60_000, 90 * gib);
        let suppressed = store
            .observe_capacity_after_commit_failure_for_test(&routine)
            .unwrap();
        assert_eq!(suppressed.write, Some(CapacityWriteOutcome::Suppressed));

        let transition = observation_at(BASE_HOUR_MS + 120_000, 5 * gib);
        let transitioned = store
            .observe_capacity_after_commit_failure_for_test(&transition)
            .unwrap();
        assert_eq!(transitioned.write, Some(CapacityWriteOutcome::Inserted));
        assert_eq!(transitioned.evaluation.pressure(), DiskPressure::Critical);
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM disk_samples", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap(),
                2
            );
        });
    }

    #[test]
    fn input_is_validated_and_millisecond_canonical() {
        let mut value = sample(DiskPressure::Healthy);
        value.sampled_at += Duration::from_nanos(999_999);
        let normalized = RawCapacitySample::try_new(
            value.volume_id.clone(),
            value.mount_path.clone(),
            value.display_name.clone(),
            value.filesystem.clone(),
            value.is_internal,
            value.is_removable,
            value.sampled_at,
            value.total_bytes,
            value.available_bytes,
            value.important_available_bytes,
            value.pressure,
        )
        .unwrap();
        assert_eq!(
            normalized.sampled_at,
            sample(DiskPressure::Healthy).sampled_at
        );

        for (total, available, important) in [
            (0, 0, None),
            (10, 11, None),
            (10, 1, Some(11)),
            (i64::MAX as u64 + 1, 1, None),
        ] {
            assert_eq!(
                RawCapacitySample::try_new(
                    VolumeId::new("volume:test").unwrap(),
                    PathBuf::from("/"),
                    "Startup".to_owned(),
                    "apfs".to_owned(),
                    true,
                    false,
                    UNIX_EPOCH,
                    total,
                    available,
                    important,
                    DiskPressure::Unknown,
                )
                .unwrap_err()
                .kind,
                HistoryErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn paths_text_and_time_are_rejected_before_preparation() {
        let build = |path: PathBuf, name: String, filesystem: String, time: SystemTime| {
            RawCapacitySample::try_new(
                VolumeId::new("volume:test").unwrap(),
                path,
                name,
                filesystem,
                true,
                false,
                time,
                10,
                1,
                None,
                DiskPressure::Unknown,
            )
        };
        assert!(
            build(
                PathBuf::from("relative"),
                "x".into(),
                "apfs".into(),
                UNIX_EPOCH
            )
            .is_err()
        );
        assert!(build(PathBuf::from("/"), "".into(), "apfs".into(), UNIX_EPOCH).is_err());
        assert!(build(PathBuf::from("/"), " \t".into(), "apfs".into(), UNIX_EPOCH).is_err());
        assert!(build(PathBuf::from("/"), "x\n".into(), "apfs".into(), UNIX_EPOCH).is_err());
        assert!(build(PathBuf::from("/"), "x".into(), "".into(), UNIX_EPOCH).is_err());
        assert!(build(PathBuf::from("/"), "x".into(), "   ".into(), UNIX_EPOCH).is_err());
        assert!(
            build(
                PathBuf::from("/"),
                "x".into(),
                "apfs".into(),
                UNIX_EPOCH - Duration::from_millis(1),
            )
            .is_err()
        );
    }

    #[test]
    fn all_pressure_values_have_stable_storage_names() {
        for (pressure, stored) in [
            (DiskPressure::Healthy, "healthy"),
            (DiskPressure::Warning, "warning"),
            (DiskPressure::Critical, "critical"),
            (DiskPressure::Unknown, "unknown"),
        ] {
            assert_eq!(pressure_as_stored(pressure), stored);
            assert_eq!(pressure_from_stored(stored).unwrap(), pressure);
        }
        assert!(pressure_from_stored("urgent").is_err());
    }

    #[test]
    fn preparation_preserves_missing_important_capacity() {
        let prepared = PreparedCapacitySample::prepare(&sample(DiskPressure::Unknown)).unwrap();
        assert_eq!(prepared.important_available_bytes, None);
    }

    #[test]
    fn page_limit_is_bounded_before_querying() {
        // The guard executes before the connection is used.
        let connection = Connection::open_in_memory().unwrap();
        let id = VolumeId::new("volume:test").unwrap();
        for limit in [0, MAX_PAGE_SIZE + 1] {
            assert_eq!(
                load_raw_capacity_page(&connection, &id, None, limit)
                    .unwrap_err()
                    .kind,
                HistoryErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn raw_samples_round_trip_all_pressures_and_survive_reopen() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store").join("dux.sqlite3");
        let pressures = [
            DiskPressure::Healthy,
            DiskPressure::Warning,
            DiskPressure::Critical,
            DiskPressure::Unknown,
        ];
        {
            let store = StoreCoordinator::open(&database).unwrap();
            for (index, pressure) in pressures.into_iter().enumerate() {
                let sample = sample_at(
                    "volume:roundtrip",
                    BASE_HOUR_MS + u64::try_from(index).unwrap() * UTC_HOUR_MS as u64,
                    400 - u64::try_from(index).unwrap(),
                    (index == 1).then_some(450),
                    pressure,
                );
                assert_eq!(sample.mount_path(), Path::new("/"));
                assert_eq!(
                    store
                        .record_raw_capacity_sample(&sample, CapacityWriteReason::Routine)
                        .unwrap(),
                    CapacityWriteOutcome::Inserted
                );
            }
        }

        let reopened = StoreCoordinator::open(&database).unwrap();
        let volume_id = VolumeId::new("volume:roundtrip").unwrap();
        let latest = reopened
            .load_latest_raw_capacity_sample(&volume_id)
            .unwrap()
            .unwrap();
        assert_eq!(latest.pressure, DiskPressure::Unknown);
        assert_eq!(latest.important_available_bytes, None);
        assert_eq!(latest.total_bytes, 1_000);
        let page = reopened
            .load_raw_capacity_page(&volume_id, None, pressures.len())
            .unwrap();
        assert_eq!(page.samples.len(), pressures.len());
        assert_eq!(
            page.samples
                .iter()
                .rev()
                .map(|sample| sample.pressure)
                .collect::<Vec<_>>(),
            pressures
        );
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn cadence_suppresses_routine_rows_but_keeps_real_transitions() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let first = sample_at(
            "volume:cadence",
            BASE_HOUR_MS + 10,
            300,
            None,
            DiskPressure::Healthy,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
                .unwrap(),
            CapacityWriteOutcome::Inserted
        );
        let routine = sample_at(
            "volume:cadence",
            BASE_HOUR_MS + 1_000,
            290,
            None,
            DiskPressure::Healthy,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&routine, CapacityWriteReason::Routine)
                .unwrap(),
            CapacityWriteOutcome::Suppressed
        );
        let false_transition = sample_at(
            "volume:cadence",
            BASE_HOUR_MS + 2_000,
            280,
            None,
            DiskPressure::Healthy,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(
                    &false_transition,
                    CapacityWriteReason::PressureTransition,
                )
                .unwrap(),
            CapacityWriteOutcome::Suppressed
        );
        let transition = sample_at(
            "volume:cadence",
            BASE_HOUR_MS + 3_000,
            270,
            Some(300),
            DiskPressure::Warning,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&transition, CapacityWriteReason::PressureTransition,)
                .unwrap(),
            CapacityWriteOutcome::Inserted
        );
        let next_hour = sample_at(
            "volume:cadence",
            BASE_HOUR_MS + UTC_HOUR_MS as u64,
            260,
            None,
            DiskPressure::Warning,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&next_hour, CapacityWriteReason::Routine)
                .unwrap(),
            CapacityWriteOutcome::Inserted
        );

        let page = store
            .load_raw_capacity_page(first.volume_id(), None, 10)
            .unwrap();
        assert_eq!(page.samples.len(), 3);
        assert_eq!(page.samples[0].sampled_at, next_hour.sampled_at());
        assert_eq!(page.samples[1].sampled_at, transition.sampled_at());
        assert_eq!(page.samples[2].sampled_at, first.sampled_at());
    }

    #[test]
    fn transition_reason_requires_a_stored_pressure_baseline() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let value = sample_at(
            "volume:no-baseline",
            BASE_HOUR_MS,
            300,
            None,
            DiskPressure::Warning,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&value, CapacityWriteReason::PressureTransition)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        store.with_connection(|connection| {
            for table in ["volumes", "disk_samples"] {
                let count: i64 = connection
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(count, 0);
            }
        });
    }

    #[test]
    fn exact_retry_conflict_and_out_of_order_writes_are_distinct() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let first = sample_at(
            "volume:retry",
            BASE_HOUR_MS + 10,
            300,
            None,
            DiskPressure::Healthy,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
                .unwrap(),
            CapacityWriteOutcome::Inserted
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
                .unwrap(),
            CapacityWriteOutcome::ExistingExact
        );

        let conflicting = sample_at(
            "volume:retry",
            BASE_HOUR_MS + 10,
            299,
            None,
            DiskPressure::Healthy,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&conflicting, CapacityWriteReason::Routine)
                .unwrap_err()
                .kind,
            HistoryErrorKind::AlreadyExists
        );
        let newer = sample_at(
            "volume:retry",
            BASE_HOUR_MS + UTC_HOUR_MS as u64,
            280,
            None,
            DiskPressure::Healthy,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&newer, CapacityWriteReason::Routine)
                .unwrap(),
            CapacityWriteOutcome::Inserted
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
                .unwrap(),
            CapacityWriteOutcome::ExistingExact
        );
        let stale = sample_at(
            "volume:retry",
            BASE_HOUR_MS + 20,
            295,
            None,
            DiskPressure::Healthy,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&stale, CapacityWriteReason::Routine)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        assert_eq!(
            store
                .load_latest_raw_capacity_sample(first.volume_id())
                .unwrap()
                .unwrap()
                .available_bytes,
            280
        );
    }

    #[test]
    fn samples_outside_volume_observation_interval_fail_closed() {
        for (case, first_offset, last_offset) in
            [("before-first", 1_i64, 2_i64), ("after-last", -2, -1)]
        {
            let temp = TempDir::new().unwrap();
            let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
            let value = sample_at(
                &format!("volume:{case}"),
                BASE_HOUR_MS,
                300,
                None,
                DiskPressure::Healthy,
            );
            store
                .record_raw_capacity_sample(&value, CapacityWriteReason::Routine)
                .unwrap();

            let sampled_at = i64::try_from(BASE_HOUR_MS).unwrap();
            store.with_connection(|connection| {
                connection
                    .execute(
                        "UPDATE volumes
                         SET first_seen_unix_ms = ?2, last_seen_unix_ms = ?3
                         WHERE volume_id = ?1",
                        params![
                            value.volume_id.as_str(),
                            sampled_at + first_offset,
                            sampled_at + last_offset,
                        ],
                    )
                    .unwrap();
            });

            assert_eq!(
                store
                    .record_raw_capacity_sample(&value, CapacityWriteReason::Routine)
                    .unwrap_err()
                    .kind,
                HistoryErrorKind::CorruptData,
                "exact retry must reject {case} ordering",
            );
            assert_eq!(
                store
                    .load_latest_raw_capacity_sample(value.volume_id())
                    .unwrap_err()
                    .kind,
                HistoryErrorKind::CorruptData,
                "latest read must reject {case} ordering",
            );
            assert_eq!(
                store
                    .load_raw_capacity_page(value.volume_id(), None, 10)
                    .unwrap_err()
                    .kind,
                HistoryErrorKind::CorruptData,
                "paged read must reject {case} ordering",
            );
        }
    }

    #[test]
    fn volume_metadata_advances_monotonically_and_collision_rolls_back() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let first = sample_at(
            "volume:metadata",
            BASE_HOUR_MS + 1,
            400,
            None,
            DiskPressure::Healthy,
        );
        store
            .record_raw_capacity_sample(&first, CapacityWriteReason::Routine)
            .unwrap();
        let mut refreshed = sample_at(
            "volume:metadata",
            BASE_HOUR_MS + 2,
            390,
            None,
            DiskPressure::Healthy,
        );
        refreshed.mount_path = PathBuf::from("/Volumes/Renamed");
        refreshed.display_name = "Renamed".to_owned();
        assert_eq!(
            store
                .record_raw_capacity_sample(&refreshed, CapacityWriteReason::Routine)
                .unwrap(),
            CapacityWriteOutcome::Suppressed
        );

        let conflicting = RawCapacitySample::try_new(
            first.volume_id.clone(),
            PathBuf::from("/Volumes/Collision"),
            "Collision".to_owned(),
            "apfs".to_owned(),
            true,
            false,
            first.sampled_at,
            first.total_bytes,
            first.available_bytes + 1,
            first.important_available_bytes,
            first.pressure,
        )
        .unwrap();
        assert_eq!(
            store
                .record_raw_capacity_sample(&conflicting, CapacityWriteReason::Routine)
                .unwrap_err()
                .kind,
            HistoryErrorKind::AlreadyExists
        );

        store.with_connection(|connection| {
            let row: (String, i64, i64) = connection
                .query_row(
                    "SELECT display_name, first_seen_unix_ms, last_seen_unix_ms
                     FROM volumes WHERE volume_id = ?1",
                    [first.volume_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(row.0, "Renamed");
            assert_eq!(row.1, i64::try_from(BASE_HOUR_MS + 1).unwrap());
            assert_eq!(row.2, i64::try_from(BASE_HOUR_MS + 2).unwrap());
        });
    }

    #[test]
    fn pages_are_bounded_stable_and_volume_isolated() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let pressures = [
            DiskPressure::Healthy,
            DiskPressure::Warning,
            DiskPressure::Healthy,
            DiskPressure::Warning,
            DiskPressure::Healthy,
        ];
        for (index, pressure) in pressures.into_iter().enumerate() {
            let value = sample_at(
                "volume:page-a",
                BASE_HOUR_MS + u64::try_from(index).unwrap(),
                500 - u64::try_from(index).unwrap(),
                None,
                pressure,
            );
            let reason = if index == 0 {
                CapacityWriteReason::Routine
            } else {
                CapacityWriteReason::PressureTransition
            };
            assert_eq!(
                store.record_raw_capacity_sample(&value, reason).unwrap(),
                CapacityWriteOutcome::Inserted
            );
        }
        let other = sample_at(
            "volume:page-b",
            BASE_HOUR_MS + 100,
            100,
            None,
            DiskPressure::Critical,
        );
        store
            .record_raw_capacity_sample(&other, CapacityWriteReason::Routine)
            .unwrap();

        let id = VolumeId::new("volume:page-a").unwrap();
        let first = store.load_raw_capacity_page(&id, None, 2).unwrap();
        assert_eq!(first.samples.len(), 2);
        let second = store
            .load_raw_capacity_page(&id, first.next_cursor, 2)
            .unwrap();
        assert_eq!(second.samples.len(), 2);
        let third = store
            .load_raw_capacity_page(&id, second.next_cursor, 2)
            .unwrap();
        assert_eq!(third.samples.len(), 1);
        assert!(third.next_cursor.is_none());
        let observed = first
            .samples
            .into_iter()
            .chain(second.samples)
            .chain(third.samples)
            .map(|sample| {
                system_time_to_unix_ms(sample.sampled_at, HistoryErrorKind::CorruptData).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            observed,
            (0..5)
                .rev()
                .map(|offset| i64::try_from(BASE_HOUR_MS).unwrap() + offset)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            store
                .load_raw_capacity_page(other.volume_id(), None, 10)
                .unwrap()
                .samples
                .len(),
            1
        );
    }

    #[test]
    fn concurrent_exact_writers_have_one_insert_and_one_adoption() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let sample = sample_at(
            "volume:race",
            BASE_HOUR_MS,
            300,
            None,
            DiskPressure::Healthy,
        );
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let store = Arc::clone(&store);
            let sample = sample.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                store
                    .record_raw_capacity_sample(&sample, CapacityWriteReason::Routine)
                    .unwrap()
            }));
        }
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == CapacityWriteOutcome::Inserted)
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == CapacityWriteOutcome::ExistingExact)
                .count(),
            1
        );
    }

    #[test]
    fn post_commit_failures_reconcile_inserted_and_suppressed_outcomes() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let first = sample_at(
            "volume:ambiguous",
            BASE_HOUR_MS + 1,
            300,
            None,
            DiskPressure::Healthy,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample_after_commit_failure_for_test(
                    &first,
                    CapacityWriteReason::Routine,
                )
                .unwrap(),
            CapacityWriteOutcome::Inserted
        );
        let suppressed = sample_at(
            "volume:ambiguous",
            BASE_HOUR_MS + 2,
            299,
            None,
            DiskPressure::Healthy,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample_after_commit_failure_for_test(
                    &suppressed,
                    CapacityWriteReason::Routine,
                )
                .unwrap(),
            CapacityWriteOutcome::Suppressed
        );
        assert_eq!(
            store
                .load_raw_capacity_page(first.volume_id(), None, 10)
                .unwrap()
                .samples
                .len(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn post_commit_fact_match_cannot_mask_unsafe_storage() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let marker = database.with_extension("sqlite3.writer.lock");
        let store = StoreCoordinator::open(&database).unwrap();
        let value = sample_at(
            "volume:unsafe-post-commit",
            BASE_HOUR_MS,
            300,
            None,
            DiskPressure::Healthy,
        );

        let error = store
            .record_raw_capacity_sample_with_after_commit_hook_for_test(
                &value,
                CapacityWriteReason::Routine,
                || {
                    std::fs::write(&marker, b"NOT-A-DUX-MARKER")
                        .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))
                },
            )
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::OutcomeUnknown);

        // The exact row committed, so a fact-only recovery would have returned
        // success despite the now-invalid retained storage marker.
        store.with_connection(|connection| {
            let count: i64 = connection
                .query_row(
                    "SELECT count(*) FROM disk_samples WHERE volume_id = ?1",
                    [value.volume_id.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        });
    }

    #[test]
    fn malformed_rows_fail_closed_and_leave_query_budget_reusable() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let value = sample_at(
            "volume:corrupt",
            BASE_HOUR_MS,
            300,
            None,
            DiskPressure::Healthy,
        );
        store
            .record_raw_capacity_sample(&value, CapacityWriteReason::Routine)
            .unwrap();
        store.with_connection(|connection| {
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "UPDATE disk_samples SET pressure = 'urgent' WHERE volume_id = ?1",
                    [value.volume_id.as_str()],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            store
                .load_latest_raw_capacity_sample(value.volume_id())
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                1
            );
        });
    }

    #[test]
    fn malformed_volume_metadata_is_rejected_by_history_reads() {
        let temp = TempDir::new().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        let value = sample_at(
            "volume:bad-metadata",
            BASE_HOUR_MS,
            300,
            None,
            DiskPressure::Healthy,
        );
        store
            .record_raw_capacity_sample(&value, CapacityWriteReason::Routine)
            .unwrap();
        store.with_connection(|connection| {
            connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            connection
                .execute(
                    "UPDATE volumes SET mount_path_encoding = 99 WHERE volume_id = ?1",
                    [value.volume_id.as_str()],
                )
                .unwrap();
            connection
                .pragma_update(None, "ignore_check_constraints", false)
                .unwrap();
        });
        assert_eq!(
            store
                .load_raw_capacity_page(value.volume_id(), None, 10)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn external_newer_schema_fences_capacity_writes() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let store = StoreCoordinator::open(&database).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO schema_migrations
                     (version, name, checksum_sha256, applied_at_unix_ms)
                     VALUES (?1, 'future-capacity', zeroblob(32), 2)",
                    [i64::from(DATABASE_SCHEMA_VERSION + 1)],
                )
                .unwrap();
            connection
                .pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION + 1)
                .unwrap();
        });
        let value = sample_at(
            "volume:fenced",
            BASE_HOUR_MS,
            300,
            None,
            DiskPressure::Unknown,
        );
        assert_eq!(
            store
                .record_raw_capacity_sample(&value, CapacityWriteReason::Routine)
                .unwrap_err()
                .kind,
            HistoryErrorKind::IncompatibleSchema
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM disk_samples", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        });
    }
}
