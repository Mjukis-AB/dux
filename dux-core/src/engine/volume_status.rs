//! Path-free startup-volume status derived by the shared Rust engine.
//!
//! These observations are telemetry only. They cannot select a cleanup target
//! or grant cleanup authority.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::domain::{AvailableCapacitySource, DiskPressure, VolumeCapacity, VolumeId};
use crate::persistence::{
    CapacityChange, CapacityPressureBaseline, CapacityTrend as StoredCapacityTrend,
    CapacityTrendPointSource as StoredTrendPointSource, CapacityWriteOutcome, HistoryErrorKind,
    RawCapacityObservation, StoreCoordinator, StoredPressureEpisode,
};

pub const MAX_PRESSURE_EPISODE_HISTORY_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapacityTrendChange {
    from: SystemTime,
    to: SystemTime,
    total_bytes: i64,
    available_bytes: i64,
    important_available_bytes: Option<i64>,
}

impl CapacityTrendChange {
    pub const fn from(&self) -> SystemTime {
        self.from
    }

    pub const fn to(&self) -> SystemTime {
        self.to
    }

    pub const fn total_bytes(&self) -> i64 {
        self.total_bytes
    }

    pub const fn available_bytes(&self) -> i64 {
        self.available_bytes
    }

    pub const fn important_available_bytes(&self) -> Option<i64> {
        self.important_available_bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapacityTrendPointSource {
    Raw,
    DailyRollup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapacityTrendPoint {
    sampled_at: SystemTime,
    total_bytes: u64,
    available_bytes: u64,
    important_available_bytes: Option<u64>,
    pressure: DiskPressure,
    source: CapacityTrendPointSource,
}

impl CapacityTrendPoint {
    pub const fn sampled_at(&self) -> SystemTime {
        self.sampled_at
    }

    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub const fn available_bytes(&self) -> u64 {
        self.available_bytes
    }

    pub const fn important_available_bytes(&self) -> Option<u64> {
        self.important_available_bytes
    }

    pub const fn pressure(&self) -> DiskPressure {
        self.pressure
    }

    pub const fn source(&self) -> CapacityTrendPointSource {
        self.source
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapacityTrend {
    volume_id: VolumeId,
    sampled_at: SystemTime,
    total_bytes: u64,
    available_bytes: u64,
    important_available_bytes: Option<u64>,
    pressure: DiskPressure,
    change_24h: Option<CapacityTrendChange>,
    change_7d: Option<CapacityTrendChange>,
    points: Vec<CapacityTrendPoint>,
}

impl CapacityTrend {
    pub fn volume_id(&self) -> &VolumeId {
        &self.volume_id
    }

    pub const fn sampled_at(&self) -> SystemTime {
        self.sampled_at
    }

    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub const fn available_bytes(&self) -> u64 {
        self.available_bytes
    }

    pub const fn important_available_bytes(&self) -> Option<u64> {
        self.important_available_bytes
    }

    pub const fn pressure(&self) -> DiskPressure {
        self.pressure
    }

    pub fn change_24h(&self) -> Option<&CapacityTrendChange> {
        self.change_24h.as_ref()
    }

    pub fn change_7d(&self) -> Option<&CapacityTrendChange> {
        self.change_7d.as_ref()
    }

    pub fn points(&self) -> &[CapacityTrendPoint] {
        &self.points
    }
}

fn public_capacity_change(value: CapacityChange) -> CapacityTrendChange {
    CapacityTrendChange {
        from: value.from,
        to: value.to,
        total_bytes: value.total_bytes,
        available_bytes: value.available_bytes,
        important_available_bytes: value.important_available_bytes,
    }
}

fn public_capacity_trend(value: StoredCapacityTrend) -> CapacityTrend {
    let anchor = value.anchor;
    CapacityTrend {
        volume_id: anchor.volume_id,
        sampled_at: anchor.sampled_at,
        total_bytes: anchor.total_bytes,
        available_bytes: anchor.available_bytes,
        important_available_bytes: anchor.important_available_bytes,
        pressure: anchor.pressure,
        change_24h: value.change_24h.map(public_capacity_change),
        change_7d: value.change_7d.map(public_capacity_change),
        points: value
            .points
            .into_iter()
            .map(|point| CapacityTrendPoint {
                sampled_at: point.sample.sampled_at,
                total_bytes: point.sample.total_bytes,
                available_bytes: point.sample.available_bytes,
                important_available_bytes: point.sample.important_available_bytes,
                pressure: point.sample.pressure,
                source: match point.source {
                    StoredTrendPointSource::Raw => CapacityTrendPointSource::Raw,
                    StoredTrendPointSource::DailyRollup => CapacityTrendPointSource::DailyRollup,
                },
            })
            .collect(),
    }
}

/// One durable interval where a volume was in Warning or Critical pressure.
///
/// This path-free telemetry record describes only an observed state transition.
/// It cannot select a filesystem object or grant cleanup authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressureEpisodeLevel {
    Warning,
    Critical,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PressureEpisode {
    level: PressureEpisodeLevel,
    entered_at: SystemTime,
    exited_at: Option<SystemTime>,
    policy_revision: u64,
}

impl PressureEpisode {
    pub const fn level(&self) -> PressureEpisodeLevel {
        self.level
    }

    pub const fn entered_at(&self) -> SystemTime {
        self.entered_at
    }

    pub const fn exited_at(&self) -> Option<SystemTime> {
        self.exited_at
    }

    pub const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }
}

/// A bounded newest-first page of durable pressure episodes for one volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PressureEpisodeHistory {
    volume_id: VolumeId,
    anchor_at: SystemTime,
    episodes: Vec<PressureEpisode>,
    has_more: bool,
}

impl PressureEpisodeHistory {
    pub fn volume_id(&self) -> &VolumeId {
        &self.volume_id
    }

    pub const fn anchor_at(&self) -> SystemTime {
        self.anchor_at
    }

    pub fn episodes(&self) -> &[PressureEpisode] {
        &self.episodes
    }

    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

fn public_pressure_episode(
    value: StoredPressureEpisode,
    anchor_at: SystemTime,
) -> Result<PressureEpisode, PressureEpisodeHistoryError> {
    let level = match value.pressure {
        DiskPressure::Warning => PressureEpisodeLevel::Warning,
        DiskPressure::Critical => PressureEpisodeLevel::Critical,
        DiskPressure::Healthy | DiskPressure::Unknown => {
            return Err(PressureEpisodeHistoryError::CorruptData);
        }
    };
    Ok(PressureEpisode {
        level,
        entered_at: value.entered_at,
        exited_at: value.exited_at.filter(|exited_at| *exited_at <= anchor_at),
        policy_revision: value.policy_revision,
    })
}

/// One platform capacity observation. Optional metadata remains optional so a
/// truthful ephemeral result can be shown when Foundation cannot supply every
/// field required by durable history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeCapacityObservation {
    volume_id: Option<VolumeId>,
    mount_path: PathBuf,
    display_name: Option<String>,
    filesystem: Option<String>,
    is_internal: Option<bool>,
    is_removable: Option<bool>,
    sampled_at: SystemTime,
    capacity: VolumeCapacity,
}

impl VolumeCapacityObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        volume_id: Option<VolumeId>,
        mount_path: PathBuf,
        display_name: Option<String>,
        filesystem: Option<String>,
        is_internal: Option<bool>,
        is_removable: Option<bool>,
        sampled_at: SystemTime,
        capacity: VolumeCapacity,
    ) -> Result<Self, VolumeCapacityStatusError> {
        let Some(sampled_millis) = unix_millis(sampled_at) else {
            return Err(VolumeCapacityStatusError::InvalidObservation);
        };
        if !mount_path.is_absolute() {
            return Err(VolumeCapacityStatusError::InvalidObservation);
        }
        let sampled_at = UNIX_EPOCH + Duration::from_millis(sampled_millis as u64);
        Ok(Self {
            volume_id,
            mount_path,
            display_name: normalize_optional_text(display_name),
            filesystem: normalize_optional_text(filesystem),
            is_internal,
            is_removable,
            sampled_at,
            capacity,
        })
    }

    pub fn volume_id(&self) -> Option<&VolumeId> {
        self.volume_id.as_ref()
    }

    pub const fn sampled_at(&self) -> SystemTime {
        self.sampled_at
    }

    pub const fn capacity(&self) -> VolumeCapacity {
        self.capacity
    }
}

/// Why one observation was or was not retained in durable history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CapacityHistoryDisposition {
    Stored,
    ExistingExact,
    SuppressedByHourlyCadence,
    NotStoredMissingOrdinaryAvailability,
    NotStoredMissingStableIdentity,
    NotStoredIncompleteMetadata,
}

/// Canonical path-free volume status returned to UI clients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeCapacityStatus {
    volume_id: Option<VolumeId>,
    sampled_at: SystemTime,
    total_bytes: u64,
    ordinary_available_bytes: Option<u64>,
    important_available_bytes: Option<u64>,
    headline_available_bytes: u64,
    headline_source: AvailableCapacitySource,
    pressure: DiskPressure,
    previous_durable_pressure: Option<DiskPressure>,
    critical_boundary_bytes: u64,
    warning_boundary_bytes: u64,
    history_disposition: CapacityHistoryDisposition,
}

impl VolumeCapacityStatus {
    pub fn volume_id(&self) -> Option<&VolumeId> {
        self.volume_id.as_ref()
    }

    pub const fn sampled_at(&self) -> SystemTime {
        self.sampled_at
    }

    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub const fn ordinary_available_bytes(&self) -> Option<u64> {
        self.ordinary_available_bytes
    }

    pub const fn important_available_bytes(&self) -> Option<u64> {
        self.important_available_bytes
    }

    pub const fn headline_available_bytes(&self) -> u64 {
        self.headline_available_bytes
    }

    pub const fn headline_source(&self) -> AvailableCapacitySource {
        self.headline_source
    }

    pub const fn pressure(&self) -> DiskPressure {
        self.pressure
    }

    pub const fn previous_durable_pressure(&self) -> Option<DiskPressure> {
        self.previous_durable_pressure
    }

    pub const fn critical_boundary_bytes(&self) -> u64 {
        self.critical_boundary_bytes
    }

    pub const fn warning_boundary_bytes(&self) -> u64 {
        self.warning_boundary_bytes
    }

    pub const fn history_disposition(&self) -> CapacityHistoryDisposition {
        self.history_disposition
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum VolumeCapacityStatusError {
    #[error("engine session is closed")]
    Closed,
    #[error("volume capacity observation is invalid")]
    InvalidObservation,
    #[error("a different observation already exists at this time")]
    ConflictingObservation,
    #[error("a newer capacity observation already exists")]
    SupersededObservation,
    #[error("durable store is read-only")]
    ReadOnlyStore,
    #[error("durable schema is incompatible")]
    IncompatibleSchema,
    #[error("operation is temporarily busy")]
    Busy,
    #[error("storage failed its safety checks")]
    UnsafeStorage,
    #[error("bounded operation exceeded its budget")]
    BudgetExceeded,
    #[error("durable state is corrupt")]
    CorruptData,
    #[error("durable storage is unavailable")]
    Unavailable,
    #[error("operation outcome is unknown")]
    OutcomeUnknown,
    #[error("internal engine state is invalid")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum PressureEpisodeHistoryError {
    #[error("engine session is closed")]
    Closed,
    #[error("pressure episode page limit must be between 1 and {maximum}")]
    InvalidLimit { maximum: usize },
    #[error("pressure episode anchor is invalid")]
    InvalidAnchor,
    #[error("durable schema is incompatible")]
    IncompatibleSchema,
    #[error("operation is temporarily busy")]
    Busy,
    #[error("storage failed its safety checks")]
    UnsafeStorage,
    #[error("bounded operation exceeded its budget")]
    BudgetExceeded,
    #[error("durable state is corrupt")]
    CorruptData,
    #[error("durable storage is unavailable")]
    Unavailable,
    #[error("operation outcome is unknown")]
    OutcomeUnknown,
    #[error("internal engine state is invalid")]
    InternalState,
}

/// One bounded startup-volume pressure baseline shared by every clone of an
/// engine handle. Durable history remains authoritative when it exists; this
/// state only preserves hysteresis across observations that cannot be stored.
#[derive(Debug)]
pub(super) struct StartupVolumePressureBaseline {
    known_volume_id: Option<VolumeId>,
    latest: Option<SessionCapacityBaseline>,
}

#[derive(Clone, Debug)]
struct SessionCapacityBaseline {
    observation: VolumeCapacityObservation,
    pressure: DiskPressure,
    policy_revision: u64,
}

impl StartupVolumePressureBaseline {
    pub(super) const fn new() -> Self {
        Self {
            known_volume_id: None,
            latest: None,
        }
    }

    fn baseline_for(
        &self,
        observation: &VolumeCapacityObservation,
    ) -> Result<Option<CapacityPressureBaseline>, VolumeCapacityStatusError> {
        if matches!(
            (&self.known_volume_id, observation.volume_id.as_ref()),
            (Some(known), Some(observed)) if known != observed
        ) {
            return Ok(None);
        }

        let Some(latest) = &self.latest else {
            return Ok(None);
        };
        match observation.sampled_at.cmp(&latest.observation.sampled_at) {
            std::cmp::Ordering::Less => Err(VolumeCapacityStatusError::SupersededObservation),
            std::cmp::Ordering::Equal if observation != &latest.observation => {
                Err(VolumeCapacityStatusError::ConflictingObservation)
            }
            std::cmp::Ordering::Equal | std::cmp::Ordering::Greater => {
                Ok(Some(CapacityPressureBaseline::new(
                    latest.observation.sampled_at,
                    latest.observation.capacity,
                    latest.pressure,
                    latest.policy_revision,
                )))
            }
        }
    }

    fn record_success(
        &mut self,
        observation: &VolumeCapacityObservation,
        pressure: DiskPressure,
        policy_revision: u64,
    ) {
        if let Some(observed_volume_id) = &observation.volume_id {
            self.known_volume_id = Some(observed_volume_id.clone());
        }
        self.latest = Some(SessionCapacityBaseline {
            observation: observation.clone(),
            pressure,
            policy_revision,
        });
    }
}

pub(super) fn observe_volume_capacity(
    store: &StoreCoordinator,
    session_baseline: &Mutex<StartupVolumePressureBaseline>,
    observation: VolumeCapacityObservation,
) -> Result<VolumeCapacityStatus, VolumeCapacityStatusError> {
    // Hold the single session lock through durable lookup/evaluation and the
    // baseline update. Concurrent callers therefore observe a serial pressure
    // history instead of racing read/modify/write updates.
    let mut session_baseline = session_baseline
        .lock()
        .map_err(|_| VolumeCapacityStatusError::InternalState)?;
    let capacity = observation.capacity;
    let session_previous = session_baseline.baseline_for(&observation)?;
    let missing_ordinary = capacity.available_bytes().is_none();
    let missing_identity = observation.volume_id.is_none();
    let incomplete_metadata = observation.display_name.is_none()
        || observation.filesystem.is_none()
        || observation.is_internal.is_none()
        || observation.is_removable.is_none();

    let (outcome, disposition) = if missing_identity {
        let outcome = store
            .evaluate_unidentified_capacity(observation.sampled_at, capacity, session_previous)
            .map_err(|error| map_history_error(error.kind))?;
        (
            outcome,
            CapacityHistoryDisposition::NotStoredMissingStableIdentity,
        )
    } else if missing_ordinary || incomplete_metadata {
        let outcome = store
            .evaluate_capacity(
                observation
                    .volume_id
                    .as_ref()
                    .expect("missing identity handled above"),
                observation.sampled_at,
                capacity,
                session_previous,
            )
            .map_err(|error| map_history_error(error.kind))?;
        let disposition = if missing_ordinary {
            CapacityHistoryDisposition::NotStoredMissingOrdinaryAvailability
        } else {
            CapacityHistoryDisposition::NotStoredIncompleteMetadata
        };
        (outcome, disposition)
    } else {
        let raw = RawCapacityObservation::try_new(
            observation
                .volume_id
                .clone()
                .expect("missing identity handled above"),
            observation.mount_path.clone(),
            observation
                .display_name
                .clone()
                .expect("incomplete metadata handled above"),
            observation
                .filesystem
                .clone()
                .expect("incomplete metadata handled above"),
            observation
                .is_internal
                .expect("incomplete metadata handled above"),
            observation
                .is_removable
                .expect("incomplete metadata handled above"),
            observation.sampled_at,
            capacity,
        )
        .map_err(|error| map_history_error(error.kind))?;
        let outcome = store
            .observe_capacity(&raw, session_previous)
            .map_err(|error| map_history_error(error.kind))?;
        let disposition = match outcome.write {
            Some(CapacityWriteOutcome::Inserted) => CapacityHistoryDisposition::Stored,
            Some(CapacityWriteOutcome::ExistingExact) => CapacityHistoryDisposition::ExistingExact,
            Some(CapacityWriteOutcome::Suppressed) => {
                CapacityHistoryDisposition::SuppressedByHourlyCadence
            }
            None => return Err(VolumeCapacityStatusError::InternalState),
        };
        (outcome, disposition)
    };

    let evaluation = outcome.evaluation;
    session_baseline.record_success(
        &observation,
        evaluation.pressure(),
        outcome.effective_policy.revision,
    );
    Ok(VolumeCapacityStatus {
        volume_id: observation.volume_id,
        sampled_at: observation.sampled_at,
        total_bytes: capacity.total_bytes(),
        ordinary_available_bytes: capacity.available_bytes(),
        important_available_bytes: capacity.important_available_bytes(),
        headline_available_bytes: evaluation.headline_available_bytes(),
        headline_source: evaluation.headline_source(),
        pressure: evaluation.pressure(),
        previous_durable_pressure: outcome.previous_durable_pressure,
        critical_boundary_bytes: evaluation.critical_boundary_bytes(),
        warning_boundary_bytes: evaluation.warning_boundary_bytes(),
        history_disposition: disposition,
    })
}

pub(super) fn load_capacity_trend(
    store: &StoreCoordinator,
    volume_id: &VolumeId,
    anchor_at: SystemTime,
) -> Result<CapacityTrend, VolumeCapacityStatusError> {
    store
        .load_capacity_trend(volume_id, anchor_at)
        .map_err(|error| map_history_error(error.kind))?
        .map(public_capacity_trend)
        .ok_or(VolumeCapacityStatusError::Unavailable)
}

pub(super) fn load_pressure_episode_history(
    store: &StoreCoordinator,
    volume_id: &VolumeId,
    anchor_at: SystemTime,
    limit: usize,
) -> Result<PressureEpisodeHistory, PressureEpisodeHistoryError> {
    if !(1..=MAX_PRESSURE_EPISODE_HISTORY_LIMIT).contains(&limit) {
        return Err(PressureEpisodeHistoryError::InvalidLimit {
            maximum: MAX_PRESSURE_EPISODE_HISTORY_LIMIT,
        });
    }
    let mut stored = store
        .load_pressure_episode_page_at_anchor(volume_id, anchor_at, limit + 1)
        .map_err(|error| map_pressure_episode_history_error(error.kind))?;
    let has_more = stored.len() > limit;
    stored.truncate(limit);
    Ok(PressureEpisodeHistory {
        volume_id: volume_id.clone(),
        anchor_at,
        episodes: stored
            .into_iter()
            .map(|episode| public_pressure_episode(episode, anchor_at))
            .collect::<Result<Vec<_>, _>>()?,
        has_more,
    })
}

fn map_pressure_episode_history_error(kind: HistoryErrorKind) -> PressureEpisodeHistoryError {
    match kind {
        HistoryErrorKind::InvalidInput => PressureEpisodeHistoryError::InvalidAnchor,
        HistoryErrorKind::IncompatibleSchema => PressureEpisodeHistoryError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => PressureEpisodeHistoryError::BudgetExceeded,
        HistoryErrorKind::Busy => PressureEpisodeHistoryError::Busy,
        HistoryErrorKind::UnsafeStorage => PressureEpisodeHistoryError::UnsafeStorage,
        HistoryErrorKind::CorruptData | HistoryErrorKind::NotFound => {
            PressureEpisodeHistoryError::CorruptData
        }
        HistoryErrorKind::DatabaseUnavailable => PressureEpisodeHistoryError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => PressureEpisodeHistoryError::OutcomeUnknown,
        HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::InternalState => PressureEpisodeHistoryError::InternalState,
    }
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn unix_millis(time: SystemTime) -> Option<i64> {
    let duration = time.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(duration.as_millis()).ok()
}

fn map_history_error(kind: HistoryErrorKind) -> VolumeCapacityStatusError {
    match kind {
        HistoryErrorKind::InvalidInput => VolumeCapacityStatusError::InvalidObservation,
        HistoryErrorKind::AlreadyExists => VolumeCapacityStatusError::ConflictingObservation,
        HistoryErrorKind::InvalidTransition => VolumeCapacityStatusError::SupersededObservation,
        HistoryErrorKind::IncompatibleSchema => VolumeCapacityStatusError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => VolumeCapacityStatusError::BudgetExceeded,
        HistoryErrorKind::Busy => VolumeCapacityStatusError::Busy,
        HistoryErrorKind::UnsafeStorage => VolumeCapacityStatusError::UnsafeStorage,
        HistoryErrorKind::CorruptData => VolumeCapacityStatusError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable => VolumeCapacityStatusError::Unavailable,
        HistoryErrorKind::OutcomeUnknown => VolumeCapacityStatusError::OutcomeUnknown,
        HistoryErrorKind::NotFound | HistoryErrorKind::InternalState => {
            VolumeCapacityStatusError::InternalState
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    use rusqlite::Connection;
    use tempfile::TempDir;

    use super::*;
    use crate::domain::{DiskPressureConfig, DiskPressureRecoveryMargin, DiskPressureThreshold};
    use crate::engine::{EngineConfig, EngineHandle};

    const GIB: u64 = 1_024 * 1_024 * 1_024;
    const TIB: u64 = 1_024 * GIB;

    fn config(temp: &TempDir) -> EngineConfig {
        config_at(temp.path())
    }

    fn config_at(root: &Path) -> EngineConfig {
        EngineConfig::new(
            root.join("data/dux.sqlite3"),
            root.join("data/snapshots"),
            root.join("cache"),
        )
        .unwrap()
    }

    fn observation(
        id: Option<VolumeId>,
        sampled_at: SystemTime,
        ordinary: Option<u64>,
        important: Option<u64>,
    ) -> VolumeCapacityObservation {
        VolumeCapacityObservation::try_new(
            id,
            PathBuf::from("/"),
            Some("Startup Disk".into()),
            Some("APFS".into()),
            Some(true),
            Some(false),
            sampled_at,
            VolumeCapacity::new(TIB, ordinary, important).unwrap(),
        )
        .unwrap()
    }

    fn incomplete_observation(
        id: VolumeId,
        sampled_at: SystemTime,
        available: u64,
    ) -> VolumeCapacityObservation {
        VolumeCapacityObservation::try_new(
            Some(id),
            PathBuf::from("/"),
            None,
            Some("APFS".into()),
            Some(true),
            Some(false),
            sampled_at,
            VolumeCapacity::new(TIB, Some(available), Some(available)).unwrap(),
        )
        .unwrap()
    }

    fn row_counts(path: &Path) -> (i64, i64) {
        let connection = Connection::open(path).unwrap();
        let volumes = connection
            .query_row("SELECT count(*) FROM volumes", [], |row| row.get(0))
            .unwrap();
        let samples = connection
            .query_row("SELECT count(*) FROM disk_samples", [], |row| row.get(0))
            .unwrap();
        (volumes, samples)
    }

    fn warning_at_fifty_gib() -> DiskPressureConfig {
        DiskPressureConfig::new(
            DiskPressureThreshold::new(10 * GIB, 500).unwrap(),
            DiskPressureThreshold::new(50 * GIB, 1_000).unwrap(),
            DiskPressureRecoveryMargin::new(2 * GIB, 100).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn policy_revision_forces_baselines_and_resets_old_hysteresis() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let id = VolumeId::new("volume:policy-baseline").unwrap();
        let start = UNIX_EPOCH + Duration::from_secs(10_800);

        let initial = engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                start,
                Some(40 * GIB),
                Some(40 * GIB),
            ))
            .unwrap();
        assert_eq!(initial.pressure(), DiskPressure::Healthy);
        assert_eq!(
            initial.history_disposition(),
            CapacityHistoryDisposition::Stored
        );

        let changed = engine
            .set_disk_pressure_policy(warning_at_fifty_gib())
            .unwrap();
        assert!(changed.changed);
        assert_eq!(changed.settings.revision, 1);
        let policy_baseline = engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                start + Duration::from_secs(60),
                Some(40 * GIB),
                Some(40 * GIB),
            ))
            .unwrap();
        assert_eq!(policy_baseline.pressure(), DiskPressure::Warning);
        assert_eq!(
            policy_baseline.history_disposition(),
            CapacityHistoryDisposition::Stored
        );
        let same_revision = engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                start + Duration::from_secs(120),
                Some(40 * GIB),
                Some(40 * GIB),
            ))
            .unwrap();
        assert_eq!(same_revision.pressure(), DiskPressure::Warning);
        assert_eq!(
            same_revision.history_disposition(),
            CapacityHistoryDisposition::SuppressedByHourlyCadence
        );

        let reset = engine.reset_disk_pressure_policy().unwrap();
        assert!(reset.changed);
        assert_eq!(reset.settings.revision, 2);
        let reset_baseline = engine
            .observe_volume_capacity(observation(
                Some(id),
                start + Duration::from_secs(180),
                Some(40 * GIB),
                Some(40 * GIB),
            ))
            .unwrap();
        assert_eq!(reset_baseline.pressure(), DiskPressure::Healthy);
        assert_eq!(
            reset_baseline.history_disposition(),
            CapacityHistoryDisposition::Stored
        );
        assert_eq!(row_counts(engine.config().database_path()), (1, 3));
        let revisions = Connection::open(engine.config().database_path())
            .unwrap()
            .prepare("SELECT policy_revision FROM disk_samples ORDER BY sampled_at_unix_ms")
            .unwrap()
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(revisions, vec![0, 1, 2]);
    }

    #[test]
    fn pressure_episode_history_is_bounded_newest_first_and_path_free() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        engine
            .set_disk_pressure_policy(warning_at_fifty_gib())
            .unwrap();
        let id = VolumeId::new("volume:pressure-history").unwrap();
        let start = UNIX_EPOCH + Duration::from_secs(18_000);

        for (offset, available, expected) in [
            (0, 40 * GIB, DiskPressure::Warning),
            (60, 5 * GIB, DiskPressure::Critical),
            (120, 100 * GIB, DiskPressure::Healthy),
        ] {
            let status = engine
                .observe_volume_capacity(observation(
                    Some(id.clone()),
                    start + Duration::from_secs(offset),
                    Some(available),
                    Some(available),
                ))
                .unwrap();
            assert_eq!(status.pressure(), expected);
        }

        let history = engine
            .pressure_episode_history(&id, start + Duration::from_secs(120), 1)
            .unwrap();
        assert_eq!(history.volume_id(), &id);
        assert_eq!(history.anchor_at(), start + Duration::from_secs(120));
        assert!(history.has_more());
        assert_eq!(history.episodes().len(), 1);
        let newest = &history.episodes()[0];
        assert_eq!(newest.level(), PressureEpisodeLevel::Critical);
        assert_eq!(newest.entered_at(), start + Duration::from_secs(60));
        assert_eq!(newest.exited_at(), Some(start + Duration::from_secs(120)));
        assert_eq!(newest.policy_revision(), 1);

        let complete = engine
            .pressure_episode_history(&id, start + Duration::from_secs(120), 2)
            .unwrap();
        assert!(!complete.has_more());
        assert_eq!(
            complete
                .episodes()
                .iter()
                .map(PressureEpisode::level)
                .collect::<Vec<_>>(),
            vec![
                PressureEpisodeLevel::Critical,
                PressureEpisodeLevel::Warning
            ]
        );
        assert_eq!(
            complete.episodes()[1].exited_at(),
            Some(start + Duration::from_secs(60))
        );

        assert_eq!(
            engine.pressure_episode_history(&id, start, 0),
            Err(PressureEpisodeHistoryError::InvalidLimit {
                maximum: MAX_PRESSURE_EPISODE_HISTORY_LIMIT
            })
        );
        assert_eq!(
            engine.pressure_episode_history(&id, start, MAX_PRESSURE_EPISODE_HISTORY_LIMIT + 1),
            Err(PressureEpisodeHistoryError::InvalidLimit {
                maximum: MAX_PRESSURE_EPISODE_HISTORY_LIMIT
            })
        );
        engine.close();
        assert_eq!(
            engine.pressure_episode_history(&id, start, 1),
            Err(PressureEpisodeHistoryError::Closed)
        );
    }

    #[test]
    fn custom_policy_classifies_every_ephemeral_evidence_shape_without_writes() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        engine
            .set_disk_pressure_policy(warning_at_fifty_gib())
            .unwrap();
        let id = VolumeId::new("volume:ephemeral-policy").unwrap();
        let start = UNIX_EPOCH + Duration::from_secs(14_400);

        let missing_id = engine
            .observe_volume_capacity(observation(None, start, Some(40 * GIB), Some(40 * GIB)))
            .unwrap();
        assert_eq!(missing_id.pressure(), DiskPressure::Warning);
        let incomplete = engine
            .observe_volume_capacity(incomplete_observation(
                id.clone(),
                start + Duration::from_secs(1),
                40 * GIB,
            ))
            .unwrap();
        assert_eq!(incomplete.pressure(), DiskPressure::Warning);
        let important_only = engine
            .observe_volume_capacity(observation(
                Some(id),
                start + Duration::from_secs(2),
                None,
                Some(40 * GIB),
            ))
            .unwrap();
        assert_eq!(important_only.pressure(), DiskPressure::Warning);
        assert_eq!(row_counts(engine.config().database_path()), (0, 0));
    }

    #[test]
    fn engine_stores_baseline_suppresses_hourly_repeat_and_keeps_transitions() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let id = VolumeId::new("volume:test-startup").unwrap();
        let start = UNIX_EPOCH + Duration::from_secs(3_600);

        let healthy = engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                start,
                Some(100 * GIB),
                Some(100 * GIB),
            ))
            .unwrap();
        assert_eq!(healthy.pressure(), DiskPressure::Healthy);
        assert_eq!(
            healthy.history_disposition(),
            CapacityHistoryDisposition::Stored
        );
        assert_eq!(healthy.previous_durable_pressure(), None);

        let exact = engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                start,
                Some(100 * GIB),
                Some(100 * GIB),
            ))
            .unwrap();
        assert_eq!(
            exact.history_disposition(),
            CapacityHistoryDisposition::ExistingExact
        );
        assert_eq!(
            engine.observe_volume_capacity(observation(
                Some(id.clone()),
                start,
                Some(99 * GIB),
                Some(99 * GIB),
            )),
            Err(VolumeCapacityStatusError::ConflictingObservation)
        );

        let repeat = engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                start + Duration::from_secs(60),
                Some(90 * GIB),
                Some(90 * GIB),
            ))
            .unwrap();
        assert_eq!(
            repeat.history_disposition(),
            CapacityHistoryDisposition::SuppressedByHourlyCadence
        );
        assert_eq!(
            repeat.previous_durable_pressure(),
            Some(DiskPressure::Healthy)
        );

        let warning = engine
            .observe_volume_capacity(observation(
                Some(id),
                start + Duration::from_secs(120),
                Some(20 * GIB),
                Some(20 * GIB),
            ))
            .unwrap();
        assert_eq!(warning.pressure(), DiskPressure::Warning);
        assert_eq!(
            warning.history_disposition(),
            CapacityHistoryDisposition::Stored
        );
        assert_eq!(row_counts(engine.config().database_path()), (1, 2));

        let id = VolumeId::new("volume:test-startup").unwrap();
        let next_hour = engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                start + Duration::from_secs(3_600),
                Some(20 * GIB),
                Some(20 * GIB),
            ))
            .unwrap();
        assert_eq!(next_hour.pressure(), DiskPressure::Warning);
        assert_eq!(
            next_hour.history_disposition(),
            CapacityHistoryDisposition::Stored
        );

        let critical = engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                start + Duration::from_secs(3_660),
                Some(5 * GIB),
                Some(5 * GIB),
            ))
            .unwrap();
        assert_eq!(critical.pressure(), DiskPressure::Critical);

        let recovered = engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                start + Duration::from_secs(3_720),
                Some(45 * GIB),
                Some(45 * GIB),
            ))
            .unwrap();
        assert_eq!(recovered.pressure(), DiskPressure::Healthy);
        assert_eq!(row_counts(engine.config().database_path()), (1, 5));

        assert_eq!(
            engine.observe_volume_capacity(observation(
                Some(id),
                start + Duration::from_secs(3_700),
                Some(50 * GIB),
                Some(50 * GIB),
            )),
            Err(VolumeCapacityStatusError::SupersededObservation)
        );
    }

    #[test]
    fn new_session_fails_closed_at_or_before_a_suppressed_durable_observation() {
        let temp = TempDir::new().unwrap();
        let first_session = EngineHandle::open(config(&temp)).unwrap();
        let id = VolumeId::new("volume:suppressed-ordering").unwrap();
        let start = UNIX_EPOCH + Duration::from_secs(7_200);

        first_session
            .observe_volume_capacity(observation(
                Some(id.clone()),
                start,
                Some(100 * GIB),
                Some(100 * GIB),
            ))
            .unwrap();
        let suppressed_at = start + Duration::from_secs(60);
        assert_eq!(
            first_session
                .observe_volume_capacity(observation(
                    Some(id.clone()),
                    suppressed_at,
                    Some(90 * GIB),
                    Some(90 * GIB),
                ))
                .unwrap()
                .history_disposition(),
            CapacityHistoryDisposition::SuppressedByHourlyCadence
        );
        assert_eq!(row_counts(first_session.config().database_path()), (1, 1));

        // A separate engine has no in-memory session baseline. The durable
        // last_seen value is therefore the only evidence that the suppressed
        // timestamp was already observed; its unretained capacity facts must
        // never be guessed or replaced.
        let reopened = EngineHandle::open(config(&temp)).unwrap();
        assert_eq!(
            reopened.observe_volume_capacity(observation(
                Some(id.clone()),
                suppressed_at,
                Some(80 * GIB),
                Some(80 * GIB),
            )),
            Err(VolumeCapacityStatusError::ConflictingObservation)
        );
        assert_eq!(
            reopened.observe_volume_capacity(observation(
                Some(id.clone()),
                suppressed_at,
                Some(90 * GIB),
                Some(90 * GIB),
            )),
            Err(VolumeCapacityStatusError::ConflictingObservation)
        );
        assert_eq!(
            reopened.observe_volume_capacity(observation(
                Some(id.clone()),
                start + Duration::from_secs(30),
                None,
                Some(95 * GIB),
            )),
            Err(VolumeCapacityStatusError::SupersededObservation)
        );
        assert_eq!(
            reopened.observe_volume_capacity(observation(
                Some(id.clone()),
                suppressed_at,
                None,
                Some(90 * GIB),
            )),
            Err(VolumeCapacityStatusError::ConflictingObservation)
        );

        let newer_ephemeral = reopened
            .observe_volume_capacity(observation(
                Some(id),
                suppressed_at + Duration::from_secs(1),
                None,
                Some(89 * GIB),
            ))
            .unwrap();
        assert_eq!(
            newer_ephemeral.history_disposition(),
            CapacityHistoryDisposition::NotStoredMissingOrdinaryAvailability
        );
        assert_eq!(row_counts(reopened.config().database_path()), (1, 1));
    }

    #[test]
    fn important_only_and_missing_identity_are_truthful_and_never_persisted() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let sampled_at = UNIX_EPOCH + Duration::from_secs(3_600);
        let id = VolumeId::new("volume:important-only").unwrap();

        let important_only = engine
            .observe_volume_capacity(observation(Some(id), sampled_at, None, Some(5 * GIB)))
            .unwrap();
        assert_eq!(important_only.pressure(), DiskPressure::Critical);
        assert_eq!(important_only.ordinary_available_bytes(), None);
        assert_eq!(
            important_only.history_disposition(),
            CapacityHistoryDisposition::NotStoredMissingOrdinaryAvailability
        );

        let missing_id = engine
            .observe_volume_capacity(observation(
                None,
                sampled_at + Duration::from_secs(1),
                Some(100 * GIB),
                Some(100 * GIB),
            ))
            .unwrap();
        assert_eq!(missing_id.volume_id(), None);
        assert_eq!(
            missing_id.history_disposition(),
            CapacityHistoryDisposition::NotStoredMissingStableIdentity
        );
        assert_eq!(row_counts(engine.config().database_path()), (0, 0));
    }

    #[test]
    fn repeated_missing_identity_samples_preserve_entry_and_recovery_hysteresis() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let sampled_at = UNIX_EPOCH + Duration::from_secs(3_600);

        for (index, (available_gib, expected)) in [
            (10, DiskPressure::Critical),
            (20, DiskPressure::Critical),
            (21, DiskPressure::Warning),
            (40, DiskPressure::Warning),
            (41, DiskPressure::Healthy),
        ]
        .into_iter()
        .enumerate()
        {
            let status = engine
                .observe_volume_capacity(observation(
                    None,
                    sampled_at + Duration::from_secs(index as u64),
                    Some(available_gib * GIB),
                    Some(available_gib * GIB),
                ))
                .unwrap();
            assert_eq!(status.pressure(), expected, "sample {index}");
            assert_eq!(status.previous_durable_pressure(), None);
        }

        assert_eq!(row_counts(engine.config().database_path()), (0, 0));
    }

    #[test]
    fn repeated_important_only_samples_use_session_hysteresis_without_writes() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let sampled_at = UNIX_EPOCH + Duration::from_secs(3_600);
        let id = VolumeId::new("volume:important-hysteresis").unwrap();

        for (index, (available_gib, expected)) in [
            (10, DiskPressure::Critical),
            (20, DiskPressure::Critical),
            (21, DiskPressure::Warning),
            (40, DiskPressure::Warning),
            (41, DiskPressure::Healthy),
        ]
        .into_iter()
        .enumerate()
        {
            let status = engine
                .observe_volume_capacity(observation(
                    Some(id.clone()),
                    sampled_at + Duration::from_secs(index as u64),
                    None,
                    Some(available_gib * GIB),
                ))
                .unwrap();
            assert_eq!(status.pressure(), expected, "sample {index}");
            assert_eq!(status.previous_durable_pressure(), None);
            assert_eq!(
                status.history_disposition(),
                CapacityHistoryDisposition::NotStoredMissingOrdinaryAvailability
            );
        }

        assert_eq!(row_counts(engine.config().database_path()), (0, 0));
    }

    #[test]
    fn known_identity_change_resets_session_baseline_but_missing_identity_does_not() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let sampled_at = UNIX_EPOCH + Duration::from_secs(3_600);
        let first_id = VolumeId::new("volume:first").unwrap();
        let second_id = VolumeId::new("volume:second").unwrap();

        let first = engine
            .observe_volume_capacity(observation(
                Some(first_id.clone()),
                sampled_at,
                None,
                Some(10 * GIB),
            ))
            .unwrap();
        assert_eq!(first.pressure(), DiskPressure::Critical);

        let missing_id = engine
            .observe_volume_capacity(observation(
                None,
                sampled_at + Duration::from_secs(1),
                Some(20 * GIB),
                Some(20 * GIB),
            ))
            .unwrap();
        assert_eq!(missing_id.pressure(), DiskPressure::Critical);

        let changed = engine
            .observe_volume_capacity(observation(
                Some(second_id),
                sampled_at + Duration::from_secs(2),
                None,
                Some(20 * GIB),
            ))
            .unwrap();
        assert_eq!(changed.pressure(), DiskPressure::Warning);

        let missing_after_change = engine
            .observe_volume_capacity(observation(
                None,
                sampled_at + Duration::from_secs(3),
                Some(40 * GIB),
                Some(40 * GIB),
            ))
            .unwrap();
        assert_eq!(missing_after_change.pressure(), DiskPressure::Warning);

        let changed_back = engine
            .observe_volume_capacity(observation(
                Some(first_id),
                sampled_at + Duration::from_secs(4),
                None,
                Some(40 * GIB),
            ))
            .unwrap();
        assert_eq!(changed_back.pressure(), DiskPressure::Healthy);
        assert_eq!(row_counts(engine.config().database_path()), (0, 0));
    }

    #[test]
    fn newer_ephemeral_pressure_takes_precedence_over_older_durable_history() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let sampled_at = UNIX_EPOCH + Duration::from_secs(3_600);
        let id = VolumeId::new("volume:newer-ephemeral").unwrap();

        assert_eq!(
            engine
                .observe_volume_capacity(observation(
                    Some(id.clone()),
                    sampled_at,
                    Some(100 * GIB),
                    Some(100 * GIB),
                ))
                .unwrap()
                .pressure(),
            DiskPressure::Healthy
        );
        assert_eq!(row_counts(engine.config().database_path()), (1, 1));

        let warning = engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                sampled_at + Duration::from_secs(1),
                None,
                Some(20 * GIB),
            ))
            .unwrap();
        assert_eq!(warning.pressure(), DiskPressure::Warning);
        assert_eq!(
            warning.previous_durable_pressure(),
            Some(DiskPressure::Healthy)
        );

        let held = engine
            .observe_volume_capacity(observation(
                Some(id),
                sampled_at + Duration::from_secs(2),
                None,
                Some(31 * GIB),
            ))
            .unwrap();
        assert_eq!(held.pressure(), DiskPressure::Warning);
        assert_eq!(
            held.previous_durable_pressure(),
            Some(DiskPressure::Healthy)
        );
        assert_eq!(row_counts(engine.config().database_path()), (1, 1));
    }

    #[test]
    fn incomplete_metadata_samples_preserve_newer_session_hysteresis() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let sampled_at = UNIX_EPOCH + Duration::from_secs(3_600);
        let id = VolumeId::new("volume:incomplete-hysteresis").unwrap();

        engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                sampled_at,
                Some(100 * GIB),
                Some(100 * GIB),
            ))
            .unwrap();
        let warning = engine
            .observe_volume_capacity(incomplete_observation(
                id.clone(),
                sampled_at + Duration::from_secs(1),
                20 * GIB,
            ))
            .unwrap();
        assert_eq!(warning.pressure(), DiskPressure::Warning);
        assert_eq!(
            warning.history_disposition(),
            CapacityHistoryDisposition::NotStoredIncompleteMetadata
        );

        let held = engine
            .observe_volume_capacity(incomplete_observation(
                id,
                sampled_at + Duration::from_secs(2),
                31 * GIB,
            ))
            .unwrap();
        assert_eq!(held.pressure(), DiskPressure::Warning);
        assert_eq!(row_counts(engine.config().database_path()), (1, 1));
    }

    #[test]
    fn later_complete_sample_uses_newer_ephemeral_pressure_and_persists_transition() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let sampled_at = UNIX_EPOCH + Duration::from_secs(3_600);
        let id = VolumeId::new("volume:ephemeral-to-durable").unwrap();

        engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                sampled_at,
                Some(100 * GIB),
                Some(100 * GIB),
            ))
            .unwrap();
        assert_eq!(
            engine
                .observe_volume_capacity(observation(
                    Some(id.clone()),
                    sampled_at + Duration::from_secs(1),
                    None,
                    Some(20 * GIB),
                ))
                .unwrap()
                .pressure(),
            DiskPressure::Warning
        );

        let complete = engine
            .observe_volume_capacity(observation(
                Some(id),
                sampled_at + Duration::from_secs(2),
                Some(31 * GIB),
                Some(31 * GIB),
            ))
            .unwrap();
        assert_eq!(complete.pressure(), DiskPressure::Warning);
        assert_eq!(
            complete.history_disposition(),
            CapacityHistoryDisposition::Stored
        );
        assert_eq!(
            complete.previous_durable_pressure(),
            Some(DiskPressure::Healthy)
        );
        assert_eq!(row_counts(engine.config().database_path()), (1, 2));
    }

    #[test]
    fn session_retries_are_exact_and_stale_or_same_time_differences_are_rejected() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let sampled_at = UNIX_EPOCH + Duration::from_secs(3_600);
        let id = VolumeId::new("volume:session-ordering").unwrap();
        let exact = observation(Some(id.clone()), sampled_at, None, Some(20 * GIB));

        let first = engine.observe_volume_capacity(exact.clone()).unwrap();
        let retry = engine.observe_volume_capacity(exact).unwrap();
        assert_eq!(first, retry);
        assert_eq!(
            engine.observe_volume_capacity(observation(
                Some(id.clone()),
                sampled_at,
                None,
                Some(21 * GIB),
            )),
            Err(VolumeCapacityStatusError::ConflictingObservation)
        );
        assert_eq!(
            engine.observe_volume_capacity(observation(
                Some(id),
                sampled_at - Duration::from_secs(1),
                None,
                Some(20 * GIB),
            )),
            Err(VolumeCapacityStatusError::SupersededObservation)
        );
        assert_eq!(row_counts(engine.config().database_path()), (0, 0));
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn newer_cross_process_durable_pressure_supersedes_local_ephemeral_baseline() {
        const CHILD_ROLE: &str = "DUX_CAPACITY_DURABLE_NEWER_CHILD";
        const CHILD_ROOT: &str = "DUX_CAPACITY_DURABLE_NEWER_ROOT";

        let sampled_at = UNIX_EPOCH + Duration::from_secs(3_600);
        let id = VolumeId::new("volume:cross-process").unwrap();
        if std::env::var_os(CHILD_ROLE).is_some() {
            let root = PathBuf::from(std::env::var_os(CHILD_ROOT).unwrap());
            let engine = EngineHandle::open(config_at(&root)).unwrap();
            assert_eq!(
                engine
                    .observe_volume_capacity(observation(
                        Some(id),
                        sampled_at + Duration::from_secs(3),
                        Some(5 * GIB),
                        Some(5 * GIB),
                    ))
                    .unwrap()
                    .pressure(),
                DiskPressure::Critical
            );
            return;
        }

        let temp = TempDir::new().unwrap();
        let first_engine = EngineHandle::open(config(&temp)).unwrap();

        first_engine
            .observe_volume_capacity(observation(
                Some(id.clone()),
                sampled_at,
                Some(100 * GIB),
                Some(100 * GIB),
            ))
            .unwrap();
        assert_eq!(
            first_engine
                .observe_volume_capacity(observation(
                    Some(id.clone()),
                    sampled_at + Duration::from_secs(1),
                    None,
                    Some(20 * GIB),
                ))
                .unwrap()
                .pressure(),
            DiskPressure::Warning
        );

        // DUX-DESTRUCTIVE: allow=test-capacity-cross-process-helper-spawn -- relaunch only this exact unit test against its TempDir-owned database to prove a newer durable pressure written by another process supersedes session-only state
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "engine::volume_status::tests::newer_cross_process_durable_pressure_supersedes_local_ephemeral_baseline",
                "--nocapture",
            ])
            .env(CHILD_ROLE, "1")
            .env(CHILD_ROOT, temp.path())
            .status()
            .unwrap();
        assert!(status.success());

        assert_eq!(
            first_engine.observe_volume_capacity(observation(
                Some(id.clone()),
                sampled_at + Duration::from_secs(2),
                None,
                Some(20 * GIB),
            )),
            Err(VolumeCapacityStatusError::SupersededObservation)
        );
        let durable_wins = first_engine
            .observe_volume_capacity(observation(
                Some(id),
                sampled_at + Duration::from_secs(4),
                None,
                Some(20 * GIB),
            ))
            .unwrap();
        assert_eq!(
            durable_wins.previous_durable_pressure(),
            Some(DiskPressure::Critical)
        );
        assert_eq!(durable_wins.pressure(), DiskPressure::Critical);
        assert_eq!(row_counts(first_engine.config().database_path()), (1, 2));
    }

    #[test]
    fn cloned_handles_serialize_ephemeral_pressure_updates() {
        const CALLERS: usize = 12;

        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let sampled_at = UNIX_EPOCH + Duration::from_secs(3_600);
        assert_eq!(
            engine
                .observe_volume_capacity(observation(
                    None,
                    sampled_at,
                    Some(10 * GIB),
                    Some(10 * GIB),
                ))
                .unwrap()
                .pressure(),
            DiskPressure::Critical
        );

        let barrier = Arc::new(Barrier::new(CALLERS));
        let mut callers = Vec::with_capacity(CALLERS);
        for _ in 0..CALLERS {
            let engine = engine.clone();
            let barrier = Arc::clone(&barrier);
            callers.push(thread::spawn(move || {
                barrier.wait();
                engine
                    .observe_volume_capacity(observation(
                        None,
                        sampled_at + Duration::from_secs(1),
                        Some(21 * GIB),
                        Some(21 * GIB),
                    ))
                    .unwrap()
            }));
        }
        for caller in callers {
            caller.join().unwrap();
        }

        // Every exact retry observes the same serialized result, and the
        // successful recovery remains the session baseline.
        let follow_up = engine
            .observe_volume_capacity(observation(
                None,
                sampled_at + Duration::from_secs(CALLERS as u64 + 1),
                Some(20 * GIB),
                Some(20 * GIB),
            ))
            .unwrap();
        assert_eq!(follow_up.pressure(), DiskPressure::Warning);
        assert_eq!(row_counts(engine.config().database_path()), (0, 0));
    }

    #[test]
    fn reopened_engine_uses_durable_pressure_for_hysteresis() {
        let temp = TempDir::new().unwrap();
        let id = VolumeId::new("volume:hysteresis").unwrap();
        let start = UNIX_EPOCH + Duration::from_secs(7_200);
        {
            let engine = EngineHandle::open(config(&temp)).unwrap();
            let critical = engine
                .observe_volume_capacity(observation(
                    Some(id.clone()),
                    start,
                    Some(5 * GIB),
                    Some(5 * GIB),
                ))
                .unwrap();
            assert_eq!(critical.pressure(), DiskPressure::Critical);
            engine.close();
            assert!(engine.wait_until_closed(Duration::from_secs(5)));
        }

        let engine = EngineHandle::open(config(&temp)).unwrap();
        let held = engine
            .observe_volume_capacity(observation(
                Some(id),
                start + Duration::from_secs(60),
                Some(11 * GIB),
                Some(11 * GIB),
            ))
            .unwrap();
        assert_eq!(
            held.previous_durable_pressure(),
            Some(DiskPressure::Critical)
        );
        assert_eq!(held.pressure(), DiskPressure::Critical);
    }

    #[test]
    fn incomplete_metadata_is_ephemeral_and_closed_engine_rejects_use() {
        let temp = TempDir::new().unwrap();
        let engine = EngineHandle::open(config(&temp)).unwrap();
        let value = VolumeCapacityObservation::try_new(
            Some(VolumeId::new("volume:incomplete").unwrap()),
            PathBuf::from("/"),
            None,
            Some("APFS".into()),
            Some(true),
            Some(false),
            UNIX_EPOCH + Duration::from_secs(3_600),
            VolumeCapacity::new(TIB, Some(100 * GIB), None).unwrap(),
        )
        .unwrap();
        assert_eq!(
            engine
                .observe_volume_capacity(value.clone())
                .unwrap()
                .history_disposition(),
            CapacityHistoryDisposition::NotStoredIncompleteMetadata
        );
        assert_eq!(row_counts(engine.config().database_path()), (0, 0));
        engine.close();
        assert_eq!(
            engine.observe_volume_capacity(value),
            Err(VolumeCapacityStatusError::Closed)
        );
    }
}
