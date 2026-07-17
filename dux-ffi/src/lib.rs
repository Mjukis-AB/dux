//! Owned, versioned UniFFI boundary for the DUX macOS application.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dux_core::engine::{
    CancelOutcome as CoreCancelOutcome, CapacityHistoryDisposition as CoreHistoryDisposition,
    DiskPressurePolicy as CorePressurePolicy, DiskPressurePolicyError as CorePressurePolicyError,
    DiskPressurePolicySource as CorePressurePolicySource,
    DiskPressurePolicyUpdate as CorePressurePolicyUpdate, EngineConfig, EngineHandle,
    EngineOpenError, HistoryMaintenanceStartOutcome,
    ScanRecoveryMaintenanceOutcome as CoreScanRecoveryOutcome, ScanRecoveryMaintenanceStartOutcome,
    SnapshotOrphanMaintenanceOutcome as CoreOrphanOutcome, SnapshotOrphanMaintenanceStartOutcome,
    SnapshotProvisioningStageMaintenanceOutcome as CoreStageOutcome,
    SnapshotProvisioningStageMaintenanceStartOutcome,
    SnapshotRetentionOutcome as CoreRetentionOutcome, SnapshotRetentionStartOutcome,
    SnapshotReviewError as CoreReviewError,
    SnapshotReviewReleaseOutcome as CoreReviewReleaseOutcome,
    SnapshotReviewSession as CoreReviewSession,
    SnapshotTerminalTempMaintenanceOutcome as CoreTerminalTempOutcome,
    SnapshotTerminalTempMaintenanceStartOutcome,
    SnapshotUnleasedTempMaintenanceOutcome as CoreUnleasedTempOutcome,
    SnapshotUnleasedTempMaintenanceStartOutcome, StartTaskError, TaskFailureKind, TaskId,
    TaskPhase as CoreTaskPhase, VolumeCapacityObservation as CoreVolumeObservation,
    VolumeCapacityStatusError as CoreVolumeStatusError,
};
use dux_core::{
    AvailableCapacitySource as CoreCapacitySource, DatabaseOpenErrorKind,
    DiskPressure as CoreDiskPressure, DiskPressureConfig, DiskPressureConfigError,
    DiskPressureRecoveryMargin, DiskPressureThreshold, ScanId, SnapshotOpenErrorKind,
    VolumeCapacity, VolumeId,
};

const FFI_CONTRACT_VERSION: u32 = 6;
const FFI_RECORD_VERSION: u32 = 1;
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
static LIVE_ENGINE_INSTANCE_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LibraryVersion {
    pub library_version: String,
    pub ffi_contract_version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FormattedSize {
    pub bytes: u64,
    pub display: String,
}

/// Foundation-derived startup-volume facts. The mount path is intentionally
/// fixed inside this adapter and never crosses the FFI boundary.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StartupVolumeObservation {
    pub record_version: u32,
    pub stable_volume_id: Option<String>,
    pub display_name: Option<String>,
    pub filesystem: Option<String>,
    pub is_internal: Option<bool>,
    pub is_removable: Option<bool>,
    pub sampled_at_unix_ms: i64,
    pub total_bytes: u64,
    pub ordinary_available_bytes: Option<u64>,
    pub important_available_bytes: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum VolumePressure {
    Healthy,
    Warning,
    Critical,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum VolumeCapacitySource {
    ImportantUsage,
    Ordinary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum VolumeHistoryDisposition {
    Stored,
    ExistingExact,
    SuppressedByHourlyCadence,
    NotStoredMissingOrdinaryAvailability,
    NotStoredMissingStableIdentity,
    NotStoredIncompleteMetadata,
}

/// Canonical path-free result. It is presentation telemetry and carries no
/// cleanup target or authority.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StartupVolumeStatus {
    pub record_version: u32,
    pub stable_volume_id: Option<String>,
    pub sampled_at_unix_ms: i64,
    pub total_bytes: u64,
    pub ordinary_available_bytes: Option<u64>,
    pub important_available_bytes: Option<u64>,
    pub headline_available_bytes: u64,
    pub headline_source: VolumeCapacitySource,
    pub pressure: VolumePressure,
    pub previous_durable_pressure: Option<VolumePressure>,
    pub critical_boundary_bytes: u64,
    pub warning_boundary_bytes: u64,
    pub history_disposition: VolumeHistoryDisposition,
}

/// Exact integer policy input. Basis points retain two decimal percentage
/// places without floating-point conversion.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PressurePolicyInput {
    pub record_version: u32,
    pub critical_available_bytes: u64,
    pub critical_available_basis_points: u16,
    pub warning_available_bytes: u64,
    pub warning_available_basis_points: u16,
    pub recovery_bytes: u64,
    pub recovery_basis_points: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PressurePolicySource {
    Default,
    Stored,
}

/// Versioned, path-free effective pressure policy.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PressurePolicyStatus {
    pub record_version: u32,
    pub source: PressurePolicySource,
    pub revision: u64,
    pub critical_available_bytes: u64,
    pub critical_available_basis_points: u16,
    pub warning_available_bytes: u64,
    pub warning_available_basis_points: u16,
    pub recovery_bytes: u64,
    pub recovery_basis_points: u16,
    pub updated_at_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PressurePolicyUpdate {
    pub record_version: u32,
    pub policy: PressurePolicyStatus,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum PressurePolicyError {
    #[error("engine session is closed")]
    Closed,
    #[error("pressure policy record version is unsupported")]
    InvalidRecordVersion,
    #[error("a pressure threshold byte value must be positive")]
    ThresholdBytesZero,
    #[error("pressure threshold basis points are outside 1 through 10000")]
    ThresholdBasisPointsOutOfRange,
    #[error("the warning byte threshold is below the critical threshold")]
    WarningBytesBelowCritical,
    #[error("the warning percentage threshold is below the critical threshold")]
    WarningBasisPointsBelowCritical,
    #[error("warning and critical thresholds are identical")]
    WarningThresholdMatchesCritical,
    #[error("the pressure recovery byte margin must be positive")]
    RecoveryBytesZero,
    #[error("pressure recovery basis points are outside 1 through 10000")]
    RecoveryBasisPointsOutOfRange,
    #[error("the pressure policy revision cannot advance")]
    RevisionExhausted,
    #[error("the system clock cannot be represented")]
    InvalidClock,
    #[error("the durable schema is incompatible")]
    IncompatibleSchema,
    #[error("the durable store is temporarily busy")]
    Busy,
    #[error("storage failed its safety checks")]
    UnsafeStorage,
    #[error("the bounded settings query exceeded its budget")]
    BudgetExceeded,
    #[error("disk-pressure settings are corrupt")]
    CorruptData,
    #[error("disk-pressure settings are unavailable")]
    Unavailable,
    #[error("the settings write outcome is unknown")]
    OutcomeUnknown,
    #[error("internal pressure-policy state is invalid")]
    InternalState,
}

/// Input-only adapter storage roots. No path is returned by this contract.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct EngineStorageRoots {
    pub data_root: String,
    pub cache_root: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum EngineError {
    #[error("engine session is closed")]
    Closed,
    #[error("engine storage configuration is invalid")]
    InvalidStorage,
    #[error("engine storage is unavailable")]
    StorageUnavailable,
    #[error("engine task registry is unavailable")]
    RegistryUnavailable,
    #[error("scan identity is invalid")]
    InvalidScanId,
    #[error("volume capacity observation is invalid")]
    InvalidCapacityObservation,
    #[error("a different capacity observation already exists at this time")]
    ConflictingCapacityObservation,
    #[error("a newer capacity observation already exists")]
    SupersededCapacityObservation,
    #[error("scan does not exist")]
    ScanNotFound,
    #[error("scan snapshot is unavailable")]
    SnapshotUnavailable,
    #[error("snapshot review lease expired")]
    ReviewExpired,
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
    #[error("snapshot format is incompatible")]
    IncompatibleSnapshot,
    #[error("operation outcome is unknown")]
    OutcomeUnknown,
    #[error("internal engine state is invalid")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MaintenanceKind {
    ScanRecovery,
    History,
    SnapshotRetention,
    SnapshotOrphan,
    SnapshotProvisioningStage,
    SnapshotTerminalTemp,
    SnapshotUnleasedTemp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MaintenanceStartDisposition {
    Started,
    AlreadyActive,
    DeferredBusy,
}

#[derive(Clone, uniffi::Record)]
pub struct MaintenanceStart {
    pub record_version: u32,
    pub disposition: MaintenanceStartDisposition,
    pub task: Option<Arc<MaintenanceTask>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum TaskPhase {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MaintenanceFailure {
    InvalidClock,
    IncompatibleSchema,
    Busy,
    UnsafeStorage,
    BudgetExceeded,
    CorruptData,
    IncompatibleSnapshot,
    Unavailable,
    OutcomeUnknown,
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MaintenanceOutcome {
    ScanRecoveryNone,
    ScanRecoveryDeferredUnproven,
    ScanRecoveryInterrupted,
    ScanRecoveryChangedConcurrently,
    HistoryApplied,
    RetentionUnderCap,
    RetentionDeferredUnstable,
    RetentionDeferredNoEligibleSnapshot,
    RetentionRemovedTombstonedResidual,
    RetentionTombstonedAndRemoved,
    OrphanNone,
    OrphanRemoved,
    StageNone,
    StageDeferredUnproven,
    StageRemovedMarkerOnly,
    StageRemovedMarkerComplete,
    TerminalTempNone,
    TerminalTempDeferredActive,
    TerminalTempReconciledRowOnly,
    TerminalTempRemoved,
    UnleasedTempNone,
    UnleasedTempDeferredActive,
    UnleasedTempRemoved,
}

/// One path-free terminal observation. Fields not used by a kind are zero.
/// Their meanings are fixed by `kind` and `outcome`; no field carries cleanup
/// authority. History uses the four `*_count_after` fields for created daily
/// rollups, pruned raw samples, pruned daily rollups, and pruned AI insights.
/// Scan recovery uses primary before/after for the inspected claimed-running
/// page, then secondary/tertiary/quaternary before for alive, unknown, and
/// definitely gone owners; process identities never cross this boundary.
/// Snapshot residual kinds use count pairs for their documented inventories;
/// byte fields always contain bytes and never row counts.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MaintenanceResult {
    pub record_version: u32,
    pub kind: MaintenanceKind,
    pub observed_at_unix_ms: i64,
    pub outcome: MaintenanceOutcome,
    pub primary_count_before: u64,
    pub primary_count_after: u64,
    pub secondary_count_before: u64,
    pub secondary_count_after: u64,
    pub tertiary_count_before: u64,
    pub tertiary_count_after: u64,
    pub quaternary_count_before: u64,
    pub quaternary_count_after: u64,
    pub charged_bytes_before: u64,
    pub charged_bytes_after: u64,
    pub removed_bytes: u64,
    pub cap_bytes: u64,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MaintenancePoll {
    pub record_version: u32,
    pub kind: MaintenanceKind,
    pub phase: TaskPhase,
    pub cancellation_requested: bool,
    pub revision: u64,
    pub failure: Option<MaintenanceFailure>,
    pub result: Option<MaintenanceResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MaintenanceCancelOutcome {
    CancelledBeforeStart,
    Requested,
    AlreadyRequested,
    AlreadyTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ReviewReleaseOutcome {
    Released,
    AlreadyReleased,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SnapshotReviewInfo {
    pub record_version: u32,
    pub scan_id: String,
    pub expires_at_unix_ms: i64,
    pub released: bool,
}

#[derive(uniffi::Object)]
pub struct SnapshotReviewSession {
    inner: Mutex<CoreReviewSession>,
    engine_closed: Arc<AtomicBool>,
}

#[uniffi::export]
impl SnapshotReviewSession {
    pub fn info(&self) -> Result<SnapshotReviewInfo, EngineError> {
        let session = self.inner.lock().map_err(|_| EngineError::InternalState)?;
        let released = session.is_released();
        Ok(SnapshotReviewInfo {
            record_version: FFI_RECORD_VERSION,
            scan_id: session.scan_id().as_str().to_owned(),
            expires_at_unix_ms: if released {
                0
            } else {
                session.expires_at_unix_ms().map_err(map_review_error)?
            },
            released,
        })
    }

    pub fn renew(&self) -> Result<SnapshotReviewInfo, EngineError> {
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(EngineError::Closed);
        }
        let mut session = self.inner.lock().map_err(|_| EngineError::InternalState)?;
        if self.engine_closed.load(Ordering::Acquire) {
            return Err(EngineError::Closed);
        }
        session.renew_unix_ms().map_err(map_review_error)?;
        Ok(SnapshotReviewInfo {
            record_version: FFI_RECORD_VERSION,
            scan_id: session.scan_id().as_str().to_owned(),
            expires_at_unix_ms: session.expires_at_unix_ms().map_err(map_review_error)?,
            released: false,
        })
    }

    pub fn release(&self) -> Result<ReviewReleaseOutcome, EngineError> {
        self.release_inner()
    }
}

impl SnapshotReviewSession {
    fn release_inner(&self) -> Result<ReviewReleaseOutcome, EngineError> {
        let mut session = self.inner.lock().map_err(|_| EngineError::InternalState)?;
        match session.release().map_err(map_review_error)? {
            CoreReviewReleaseOutcome::Released => Ok(ReviewReleaseOutcome::Released),
            CoreReviewReleaseOutcome::AlreadyReleased => Ok(ReviewReleaseOutcome::AlreadyReleased),
        }
    }
}

#[derive(uniffi::Object)]
pub struct MaintenanceTask {
    engine: EngineHandle,
    id: TaskId,
    kind: MaintenanceKind,
}

#[uniffi::export]
impl MaintenanceTask {
    pub fn poll(&self) -> Result<MaintenancePoll, EngineError> {
        let snapshot = self
            .engine
            .task_snapshot(self.id)
            .map_err(map_task_access_error)?;
        let result = if snapshot.result_available {
            maintenance_result(&self.engine, self.id, self.kind)?
        } else {
            None
        };
        Ok(MaintenancePoll {
            record_version: FFI_RECORD_VERSION,
            kind: self.kind,
            phase: map_phase(snapshot.phase),
            cancellation_requested: snapshot.cancellation_requested,
            revision: snapshot.revision,
            failure: snapshot.failure.map(map_failure),
            result,
        })
    }

    pub fn cancel(&self) -> Result<MaintenanceCancelOutcome, EngineError> {
        match self
            .engine
            .cancel_task(self.id)
            .map_err(map_task_access_error)?
        {
            CoreCancelOutcome::CancelledBeforeStart => {
                Ok(MaintenanceCancelOutcome::CancelledBeforeStart)
            }
            CoreCancelOutcome::Requested => Ok(MaintenanceCancelOutcome::Requested),
            CoreCancelOutcome::AlreadyRequested => Ok(MaintenanceCancelOutcome::AlreadyRequested),
            CoreCancelOutcome::AlreadyTerminal => Ok(MaintenanceCancelOutcome::AlreadyTerminal),
        }
    }
}

enum EngineState {
    Open(EngineHandle),
    Closing,
    Closed { quiesced: bool },
}

#[derive(uniffi::Object)]
pub struct DuxEngine {
    state: Mutex<EngineState>,
    close_completed: Condvar,
    reviews: Mutex<Vec<Weak<SnapshotReviewSession>>>,
    closed: Arc<AtomicBool>,
}

#[uniffi::export]
impl DuxEngine {
    #[uniffi::constructor]
    pub fn new(storage: EngineStorageRoots) -> Result<Self, EngineError> {
        let data_root = PathBuf::from(storage.data_root);
        let cache_root = PathBuf::from(storage.cache_root);
        let config = EngineConfig::new(
            data_root.join("dux.sqlite3"),
            data_root.join("snapshots"),
            cache_root,
        )
        .map_err(|_| EngineError::InvalidStorage)?;
        let engine = EngineHandle::open(config).map_err(map_open_error)?;
        LIVE_ENGINE_INSTANCE_COUNT.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            state: Mutex::new(EngineState::Open(engine)),
            close_completed: Condvar::new(),
            reviews: Mutex::new(Vec::new()),
            closed: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn library_version(&self) -> Result<LibraryVersion, EngineError> {
        self.with_engine(|_| Ok(library_version()))
    }

    pub fn format_size(&self, bytes: u64) -> Result<FormattedSize, EngineError> {
        self.with_engine(|_| {
            Ok(FormattedSize {
                bytes,
                display: dux_core::format_size(bytes),
            })
        })
    }

    pub fn observe_startup_volume(
        &self,
        observation: StartupVolumeObservation,
    ) -> Result<StartupVolumeStatus, EngineError> {
        if observation.record_version != FFI_RECORD_VERSION {
            return Err(EngineError::InvalidCapacityObservation);
        }
        let stable_volume_id = parse_macos_volume_id(observation.stable_volume_id)?;
        let sampled_at = unix_ms_to_system_time(observation.sampled_at_unix_ms)?;
        let capacity = VolumeCapacity::new(
            observation.total_bytes,
            observation.ordinary_available_bytes,
            observation.important_available_bytes,
        )
        .map_err(|_| EngineError::InvalidCapacityObservation)?;
        let core = CoreVolumeObservation::try_new(
            stable_volume_id,
            PathBuf::from("/"),
            observation.display_name,
            observation.filesystem,
            observation.is_internal,
            observation.is_removable,
            sampled_at,
            capacity,
        )
        .map_err(map_volume_status_error)?;
        self.with_engine(|engine| {
            let status = engine
                .observe_volume_capacity(core)
                .map_err(map_volume_status_error)?;
            Ok(StartupVolumeStatus {
                record_version: FFI_RECORD_VERSION,
                stable_volume_id: status.volume_id().map(ToString::to_string),
                sampled_at_unix_ms: system_time_ms(status.sampled_at())?,
                total_bytes: status.total_bytes(),
                ordinary_available_bytes: status.ordinary_available_bytes(),
                important_available_bytes: status.important_available_bytes(),
                headline_available_bytes: status.headline_available_bytes(),
                headline_source: map_capacity_source(status.headline_source()),
                pressure: map_volume_pressure(status.pressure()),
                previous_durable_pressure: status
                    .previous_durable_pressure()
                    .map(map_volume_pressure),
                critical_boundary_bytes: status.critical_boundary_bytes(),
                warning_boundary_bytes: status.warning_boundary_bytes(),
                history_disposition: map_history_disposition(status.history_disposition()),
            })
        })
    }

    pub fn get_disk_pressure_policy(&self) -> Result<PressurePolicyStatus, PressurePolicyError> {
        self.with_pressure_engine(|engine| {
            engine
                .disk_pressure_policy()
                .map_err(map_pressure_policy_error)
                .and_then(pressure_policy_status)
        })
    }

    pub fn set_disk_pressure_policy(
        &self,
        input: PressurePolicyInput,
    ) -> Result<PressurePolicyUpdate, PressurePolicyError> {
        let config = pressure_policy_config(input)?;
        self.with_pressure_engine(|engine| {
            engine
                .set_disk_pressure_policy(config)
                .map_err(map_pressure_policy_error)
                .and_then(pressure_policy_update)
        })
    }

    pub fn reset_disk_pressure_policy(&self) -> Result<PressurePolicyUpdate, PressurePolicyError> {
        self.with_pressure_engine(|engine| {
            engine
                .reset_disk_pressure_policy()
                .map_err(map_pressure_policy_error)
                .and_then(pressure_policy_update)
        })
    }

    pub fn acquire_explorer_snapshot_review(
        &self,
        scan_id: String,
    ) -> Result<Arc<SnapshotReviewSession>, EngineError> {
        let scan_id = ScanId::new(scan_id).map_err(|_| EngineError::InvalidScanId)?;
        let state = self.state.lock().map_err(|_| EngineError::InternalState)?;
        let EngineState::Open(engine) = &*state else {
            return Err(EngineError::Closed);
        };
        let session = engine
            .acquire_explorer_snapshot_review(&scan_id)
            .map_err(map_review_error)?;
        let review = Arc::new(SnapshotReviewSession {
            inner: Mutex::new(session),
            engine_closed: Arc::clone(&self.closed),
        });
        if self.closed.load(Ordering::Acquire) {
            let _ = review.release_inner();
            return Err(EngineError::Closed);
        }
        let mut reviews = match self.reviews.lock() {
            Ok(reviews) => reviews,
            Err(_) => {
                let _ = review.release_inner();
                return Err(EngineError::InternalState);
            }
        };
        reviews.retain(|review| review.strong_count() != 0);
        reviews.push(Arc::downgrade(&review));
        Ok(review)
    }

    pub fn start_maintenance(
        &self,
        kind: MaintenanceKind,
    ) -> Result<MaintenanceStart, EngineError> {
        self.with_engine(|engine| start_maintenance(engine, kind))
    }

    /// Close the engine and wait for at most five seconds for worker quiescence.
    /// Returns whether all workers have quiesced; repeated calls return the
    /// first call's final observation without reopening storage.
    pub fn close(&self) -> bool {
        self.closed.store(true, Ordering::Release);
        let engine = loop {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &*state {
                EngineState::Open(_) => {
                    let EngineState::Open(engine) =
                        std::mem::replace(&mut *state, EngineState::Closing)
                    else {
                        unreachable!("open state was just matched")
                    };
                    break engine;
                }
                EngineState::Closing => {
                    let state = self
                        .close_completed
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if let EngineState::Closed { quiesced } = *state {
                        return quiesced;
                    }
                }
                EngineState::Closed { quiesced } => return *quiesced,
            }
        };
        self.release_registered_reviews();
        engine.close();
        let quiesced = engine.wait_until_closed(CLOSE_TIMEOUT);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *state = EngineState::Closed { quiesced };
        self.close_completed.notify_all();
        quiesced
    }
}

impl DuxEngine {
    fn with_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let state = self.state.lock().map_err(|_| EngineError::InternalState)?;
        match &*state {
            EngineState::Open(engine) => operation(engine),
            EngineState::Closing | EngineState::Closed { .. } => Err(EngineError::Closed),
        }
    }

    fn with_pressure_engine<T>(
        &self,
        operation: impl FnOnce(&EngineHandle) -> Result<T, PressurePolicyError>,
    ) -> Result<T, PressurePolicyError> {
        let state = self
            .state
            .lock()
            .map_err(|_| PressurePolicyError::InternalState)?;
        match &*state {
            EngineState::Open(engine) => operation(engine),
            EngineState::Closing | EngineState::Closed { .. } => Err(PressurePolicyError::Closed),
        }
    }

    fn release_registered_reviews(&self) {
        let reviews = match self.reviews.lock() {
            Ok(mut reviews) => std::mem::take(&mut *reviews),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        };
        for review in reviews.into_iter().filter_map(|review| review.upgrade()) {
            let _ = review.release_inner();
        }
    }
}

impl Drop for DuxEngine {
    fn drop(&mut self) {
        let _ = self.close();
        LIVE_ENGINE_INSTANCE_COUNT.fetch_sub(1, Ordering::Relaxed);
    }
}

#[uniffi::export]
pub fn library_version() -> LibraryVersion {
    LibraryVersion {
        library_version: env!("CARGO_PKG_VERSION").to_owned(),
        ffi_contract_version: FFI_CONTRACT_VERSION,
    }
}

#[uniffi::export]
pub fn live_engine_instance_count() -> u64 {
    LIVE_ENGINE_INSTANCE_COUNT.load(Ordering::Relaxed)
}

fn start_maintenance(
    engine: &EngineHandle,
    kind: MaintenanceKind,
) -> Result<MaintenanceStart, EngineError> {
    macro_rules! map_start {
        ($outcome:expr, $started:path, $active:path, $busy:path) => {
            match $outcome.map_err(map_start_error)? {
                $started(id) => {
                    maintenance_start(engine, kind, MaintenanceStartDisposition::Started, Some(id))
                }
                $active(id) => maintenance_start(
                    engine,
                    kind,
                    MaintenanceStartDisposition::AlreadyActive,
                    Some(id),
                ),
                $busy => maintenance_start(
                    engine,
                    kind,
                    MaintenanceStartDisposition::DeferredBusy,
                    None,
                ),
            }
        };
    }
    Ok(match kind {
        MaintenanceKind::ScanRecovery => map_start!(
            engine.start_scan_recovery_maintenance(),
            ScanRecoveryMaintenanceStartOutcome::Started,
            ScanRecoveryMaintenanceStartOutcome::AlreadyActive,
            ScanRecoveryMaintenanceStartOutcome::DeferredBusy
        ),
        MaintenanceKind::History => map_start!(
            engine.start_history_maintenance(),
            HistoryMaintenanceStartOutcome::Started,
            HistoryMaintenanceStartOutcome::AlreadyActive,
            HistoryMaintenanceStartOutcome::DeferredBusy
        ),
        MaintenanceKind::SnapshotRetention => map_start!(
            engine.start_snapshot_retention(),
            SnapshotRetentionStartOutcome::Started,
            SnapshotRetentionStartOutcome::AlreadyActive,
            SnapshotRetentionStartOutcome::DeferredBusy
        ),
        MaintenanceKind::SnapshotOrphan => map_start!(
            engine.start_snapshot_orphan_maintenance(),
            SnapshotOrphanMaintenanceStartOutcome::Started,
            SnapshotOrphanMaintenanceStartOutcome::AlreadyActive,
            SnapshotOrphanMaintenanceStartOutcome::DeferredBusy
        ),
        MaintenanceKind::SnapshotProvisioningStage => map_start!(
            engine.start_snapshot_provisioning_stage_maintenance(),
            SnapshotProvisioningStageMaintenanceStartOutcome::Started,
            SnapshotProvisioningStageMaintenanceStartOutcome::AlreadyActive,
            SnapshotProvisioningStageMaintenanceStartOutcome::DeferredBusy
        ),
        MaintenanceKind::SnapshotTerminalTemp => map_start!(
            engine.start_snapshot_terminal_temp_maintenance(),
            SnapshotTerminalTempMaintenanceStartOutcome::Started,
            SnapshotTerminalTempMaintenanceStartOutcome::AlreadyActive,
            SnapshotTerminalTempMaintenanceStartOutcome::DeferredBusy
        ),
        MaintenanceKind::SnapshotUnleasedTemp => map_start!(
            engine.start_snapshot_unleased_temp_maintenance(),
            SnapshotUnleasedTempMaintenanceStartOutcome::Started,
            SnapshotUnleasedTempMaintenanceStartOutcome::AlreadyActive,
            SnapshotUnleasedTempMaintenanceStartOutcome::DeferredBusy
        ),
    })
}

fn maintenance_start(
    engine: &EngineHandle,
    kind: MaintenanceKind,
    disposition: MaintenanceStartDisposition,
    id: Option<TaskId>,
) -> MaintenanceStart {
    MaintenanceStart {
        record_version: FFI_RECORD_VERSION,
        disposition,
        task: id.map(|id| {
            Arc::new(MaintenanceTask {
                engine: engine.clone(),
                id,
                kind,
            })
        }),
    }
}

fn empty_result(
    kind: MaintenanceKind,
    observed_at: SystemTime,
    outcome: MaintenanceOutcome,
    has_more: bool,
) -> Result<MaintenanceResult, EngineError> {
    Ok(MaintenanceResult {
        record_version: FFI_RECORD_VERSION,
        kind,
        observed_at_unix_ms: system_time_ms(observed_at)?,
        outcome,
        primary_count_before: 0,
        primary_count_after: 0,
        secondary_count_before: 0,
        secondary_count_after: 0,
        tertiary_count_before: 0,
        tertiary_count_after: 0,
        quaternary_count_before: 0,
        quaternary_count_after: 0,
        charged_bytes_before: 0,
        charged_bytes_after: 0,
        removed_bytes: 0,
        cap_bytes: 0,
        has_more,
    })
}

fn maintenance_result(
    engine: &EngineHandle,
    id: TaskId,
    kind: MaintenanceKind,
) -> Result<Option<MaintenanceResult>, EngineError> {
    let result = match kind {
        MaintenanceKind::ScanRecovery => engine
            .scan_recovery_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let outcome = match r.outcome() {
                    CoreScanRecoveryOutcome::NoClaim => MaintenanceOutcome::ScanRecoveryNone,
                    CoreScanRecoveryOutcome::DeferredUnproven => {
                        MaintenanceOutcome::ScanRecoveryDeferredUnproven
                    }
                    CoreScanRecoveryOutcome::Interrupted => {
                        MaintenanceOutcome::ScanRecoveryInterrupted
                    }
                    CoreScanRecoveryOutcome::ChangedConcurrently => {
                        MaintenanceOutcome::ScanRecoveryChangedConcurrently
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.primary_count_before = u64::from(r.claimed_count_before());
                out.primary_count_after = u64::from(r.claimed_count_after());
                out.secondary_count_before = u64::from(r.alive_count());
                out.tertiary_count_before = u64::from(r.unknown_count());
                out.quaternary_count_before = u64::from(r.recoverable_count());
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::History => engine
            .history_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let mut out = empty_result(
                    kind,
                    r.observed_at(),
                    MaintenanceOutcome::HistoryApplied,
                    r.has_more(),
                )?;
                out.primary_count_after = u64::from(r.daily_rollups_created());
                out.secondary_count_after = u64::from(r.raw_samples_pruned());
                out.tertiary_count_after = u64::from(r.daily_rollups_pruned());
                out.quaternary_count_after = u64::from(r.ai_insights_pruned());
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::SnapshotRetention => engine
            .snapshot_retention_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let (outcome, removed) = match r.outcome() {
                    CoreRetentionOutcome::UnderCap => (MaintenanceOutcome::RetentionUnderCap, 0),
                    CoreRetentionOutcome::DeferredUnstable => {
                        (MaintenanceOutcome::RetentionDeferredUnstable, 0)
                    }
                    CoreRetentionOutcome::DeferredNoEligibleSnapshot => {
                        (MaintenanceOutcome::RetentionDeferredNoEligibleSnapshot, 0)
                    }
                    CoreRetentionOutcome::RemovedTombstonedResidual { bytes } => (
                        MaintenanceOutcome::RetentionRemovedTombstonedResidual,
                        bytes,
                    ),
                    CoreRetentionOutcome::TombstonedAndRemoved { bytes } => {
                        (MaintenanceOutcome::RetentionTombstonedAndRemoved, bytes)
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.charged_bytes_before = r.charged_bytes_before();
                out.charged_bytes_after = r.charged_bytes_after();
                out.removed_bytes = removed;
                out.cap_bytes = r.cap_bytes();
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::SnapshotOrphan => engine
            .snapshot_orphan_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let (outcome, removed) = match r.outcome() {
                    CoreOrphanOutcome::NoOrphan => (MaintenanceOutcome::OrphanNone, 0),
                    CoreOrphanOutcome::Removed { bytes } => {
                        (MaintenanceOutcome::OrphanRemoved, bytes)
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.primary_count_before = u64::from(r.orphan_count_before());
                out.primary_count_after = u64::from(r.orphan_count_after());
                out.charged_bytes_before = r.orphan_charged_bytes_before();
                out.charged_bytes_after = r.orphan_charged_bytes_after();
                out.removed_bytes = removed;
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::SnapshotProvisioningStage => engine
            .snapshot_provisioning_stage_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let (outcome, removed) = match r.outcome() {
                    CoreStageOutcome::NoStage => (MaintenanceOutcome::StageNone, 0),
                    CoreStageOutcome::DeferredUnproven => {
                        (MaintenanceOutcome::StageDeferredUnproven, 0)
                    }
                    CoreStageOutcome::RemovedMarkerOnly { bytes } => {
                        (MaintenanceOutcome::StageRemovedMarkerOnly, bytes)
                    }
                    CoreStageOutcome::RemovedMarkerComplete { bytes } => {
                        (MaintenanceOutcome::StageRemovedMarkerComplete, bytes)
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.primary_count_before = r.total_stage_count_before();
                out.primary_count_after = r.total_stage_count_after();
                out.secondary_count_before = r.marker_owned_count_before();
                out.secondary_count_after = r.marker_owned_count_after();
                out.tertiary_count_before = r.unproven_count_before();
                out.tertiary_count_after = r.unproven_count_after();
                out.charged_bytes_before = r.control_charged_bytes_before();
                out.charged_bytes_after = r.control_charged_bytes_after();
                out.removed_bytes = removed;
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::SnapshotTerminalTemp => engine
            .snapshot_terminal_temp_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let (outcome, removed) = match r.outcome() {
                    CoreTerminalTempOutcome::NoTerminalResidual => {
                        (MaintenanceOutcome::TerminalTempNone, 0)
                    }
                    CoreTerminalTempOutcome::DeferredActive => {
                        (MaintenanceOutcome::TerminalTempDeferredActive, 0)
                    }
                    CoreTerminalTempOutcome::ReconciledRowOnly => {
                        (MaintenanceOutcome::TerminalTempReconciledRowOnly, 0)
                    }
                    CoreTerminalTempOutcome::RemovedTemp { bytes } => {
                        (MaintenanceOutcome::TerminalTempRemoved, bytes)
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.primary_count_before = u64::from(r.terminal_lease_count_before());
                out.primary_count_after = u64::from(r.terminal_lease_count_after());
                out.secondary_count_before = u64::from(r.active_terminal_lease_count_before());
                out.secondary_count_after = u64::from(r.active_terminal_lease_count_after());
                out.charged_bytes_before = r.terminal_charged_bytes_before();
                out.charged_bytes_after = r.terminal_charged_bytes_after();
                out.removed_bytes = removed;
                Ok(out)
            })
            .transpose()?,
        MaintenanceKind::SnapshotUnleasedTemp => engine
            .snapshot_unleased_temp_maintenance_result(id)
            .map_err(map_task_access_error)?
            .map(|r| {
                let (outcome, removed) = match r.outcome() {
                    CoreUnleasedTempOutcome::NoUnleasedTemp => {
                        (MaintenanceOutcome::UnleasedTempNone, 0)
                    }
                    CoreUnleasedTempOutcome::DeferredActive => {
                        (MaintenanceOutcome::UnleasedTempDeferredActive, 0)
                    }
                    CoreUnleasedTempOutcome::Removed { bytes } => {
                        (MaintenanceOutcome::UnleasedTempRemoved, bytes)
                    }
                    _ => return Err(EngineError::InternalState),
                };
                let mut out = empty_result(kind, r.observed_at(), outcome, r.has_more())?;
                out.primary_count_before = u64::from(r.unleased_temp_count_before());
                out.primary_count_after = u64::from(r.unleased_temp_count_after());
                out.secondary_count_before = u64::from(r.active_unleased_temp_count_before());
                out.secondary_count_after = u64::from(r.active_unleased_temp_count_after());
                out.charged_bytes_before = r.unleased_charged_bytes_before();
                out.charged_bytes_after = r.unleased_charged_bytes_after();
                out.removed_bytes = removed;
                Ok(out)
            })
            .transpose()?,
    };
    Ok(result)
}

fn map_phase(phase: CoreTaskPhase) -> TaskPhase {
    match phase {
        CoreTaskPhase::Queued => TaskPhase::Queued,
        CoreTaskPhase::Running => TaskPhase::Running,
        CoreTaskPhase::Succeeded => TaskPhase::Succeeded,
        CoreTaskPhase::Failed => TaskPhase::Failed,
        CoreTaskPhase::Cancelled => TaskPhase::Cancelled,
    }
}

fn map_failure(failure: TaskFailureKind) -> MaintenanceFailure {
    use dux_core::engine::{
        HistoryMaintenanceFailureKind as H, ScanRecoveryMaintenanceFailureKind as S,
        SnapshotOrphanMaintenanceFailureKind as O,
        SnapshotProvisioningStageMaintenanceFailureKind as P, SnapshotRetentionFailureKind as R,
        SnapshotTerminalTempMaintenanceFailureKind as T,
        SnapshotUnleasedTempMaintenanceFailureKind as U,
    };
    macro_rules! map_typed {
        ($value:expr, $kind:ident) => {
            match $value {
                $kind::InvalidClock => MaintenanceFailure::InvalidClock,
                $kind::IncompatibleSchema => MaintenanceFailure::IncompatibleSchema,
                $kind::Busy => MaintenanceFailure::Busy,
                $kind::UnsafeStorage => MaintenanceFailure::UnsafeStorage,
                $kind::BudgetExceeded => MaintenanceFailure::BudgetExceeded,
                $kind::CorruptData => MaintenanceFailure::CorruptData,
                $kind::Unavailable => MaintenanceFailure::Unavailable,
                $kind::OutcomeUnknown => MaintenanceFailure::OutcomeUnknown,
                $kind::InternalState => MaintenanceFailure::InternalState,
                _ => MaintenanceFailure::InternalState,
            }
        };
    }
    match failure {
        TaskFailureKind::ScanRecoveryMaintenance(value) => map_typed!(value, S),
        TaskFailureKind::HistoryMaintenance(value) => map_typed!(value, H),
        TaskFailureKind::SnapshotProvisioningStageMaintenance(value) => map_typed!(value, P),
        TaskFailureKind::SnapshotTerminalTempMaintenance(value) => map_typed!(value, T),
        TaskFailureKind::SnapshotUnleasedTempMaintenance(value) => map_typed!(value, U),
        TaskFailureKind::SnapshotRetention(value) => match value {
            R::IncompatibleSnapshot => MaintenanceFailure::IncompatibleSnapshot,
            other => map_typed!(other, R),
        },
        TaskFailureKind::SnapshotOrphanMaintenance(value) => match value {
            O::IncompatibleSnapshot => MaintenanceFailure::IncompatibleSnapshot,
            other => map_typed!(other, O),
        },
        _ => MaintenanceFailure::InternalState,
    }
}

fn map_review_error(error: CoreReviewError) -> EngineError {
    match error {
        CoreReviewError::Closed => EngineError::Closed,
        CoreReviewError::ScanNotFound => EngineError::ScanNotFound,
        CoreReviewError::SnapshotUnavailable => EngineError::SnapshotUnavailable,
        CoreReviewError::LeaseExpired => EngineError::ReviewExpired,
        CoreReviewError::ReadOnlyStore => EngineError::ReadOnlyStore,
        CoreReviewError::IncompatibleSchema => EngineError::IncompatibleSchema,
        CoreReviewError::Busy => EngineError::Busy,
        CoreReviewError::UnsafeStorage => EngineError::UnsafeStorage,
        CoreReviewError::BudgetExceeded => EngineError::BudgetExceeded,
        CoreReviewError::CorruptData => EngineError::CorruptData,
        CoreReviewError::IncompatibleSnapshot => EngineError::IncompatibleSnapshot,
        CoreReviewError::Unavailable => EngineError::StorageUnavailable,
        CoreReviewError::OutcomeUnknown => EngineError::OutcomeUnknown,
        CoreReviewError::InternalState => EngineError::InternalState,
        _ => EngineError::InternalState,
    }
}

fn map_volume_status_error(error: CoreVolumeStatusError) -> EngineError {
    match error {
        CoreVolumeStatusError::Closed => EngineError::Closed,
        CoreVolumeStatusError::InvalidObservation => EngineError::InvalidCapacityObservation,
        CoreVolumeStatusError::ConflictingObservation => {
            EngineError::ConflictingCapacityObservation
        }
        CoreVolumeStatusError::SupersededObservation => EngineError::SupersededCapacityObservation,
        CoreVolumeStatusError::ReadOnlyStore => EngineError::ReadOnlyStore,
        CoreVolumeStatusError::IncompatibleSchema => EngineError::IncompatibleSchema,
        CoreVolumeStatusError::Busy => EngineError::Busy,
        CoreVolumeStatusError::UnsafeStorage => EngineError::UnsafeStorage,
        CoreVolumeStatusError::BudgetExceeded => EngineError::BudgetExceeded,
        CoreVolumeStatusError::CorruptData => EngineError::CorruptData,
        CoreVolumeStatusError::Unavailable => EngineError::StorageUnavailable,
        CoreVolumeStatusError::OutcomeUnknown => EngineError::OutcomeUnknown,
        CoreVolumeStatusError::InternalState => EngineError::InternalState,
    }
}

fn pressure_policy_config(
    input: PressurePolicyInput,
) -> Result<DiskPressureConfig, PressurePolicyError> {
    if input.record_version != FFI_RECORD_VERSION {
        return Err(PressurePolicyError::InvalidRecordVersion);
    }
    let critical = DiskPressureThreshold::new(
        input.critical_available_bytes,
        input.critical_available_basis_points,
    )
    .map_err(map_pressure_config_error)?;
    let warning = DiskPressureThreshold::new(
        input.warning_available_bytes,
        input.warning_available_basis_points,
    )
    .map_err(map_pressure_config_error)?;
    let recovery =
        DiskPressureRecoveryMargin::new(input.recovery_bytes, input.recovery_basis_points)
            .map_err(map_pressure_config_error)?;
    DiskPressureConfig::new(critical, warning, recovery).map_err(map_pressure_config_error)
}

fn map_pressure_config_error(error: DiskPressureConfigError) -> PressurePolicyError {
    match error {
        DiskPressureConfigError::ThresholdBytesZero => PressurePolicyError::ThresholdBytesZero,
        DiskPressureConfigError::ThresholdBasisPointsOutOfRange => {
            PressurePolicyError::ThresholdBasisPointsOutOfRange
        }
        DiskPressureConfigError::WarningBytesBelowCritical => {
            PressurePolicyError::WarningBytesBelowCritical
        }
        DiskPressureConfigError::WarningBasisPointsBelowCritical => {
            PressurePolicyError::WarningBasisPointsBelowCritical
        }
        DiskPressureConfigError::WarningThresholdMatchesCritical => {
            PressurePolicyError::WarningThresholdMatchesCritical
        }
        DiskPressureConfigError::RecoveryBytesZero => PressurePolicyError::RecoveryBytesZero,
        DiskPressureConfigError::RecoveryBasisPointsOutOfRange => {
            PressurePolicyError::RecoveryBasisPointsOutOfRange
        }
    }
}

fn map_pressure_policy_error(error: CorePressurePolicyError) -> PressurePolicyError {
    match error {
        CorePressurePolicyError::Closed => PressurePolicyError::Closed,
        CorePressurePolicyError::RevisionExhausted => PressurePolicyError::RevisionExhausted,
        CorePressurePolicyError::InvalidClock => PressurePolicyError::InvalidClock,
        CorePressurePolicyError::IncompatibleSchema => PressurePolicyError::IncompatibleSchema,
        CorePressurePolicyError::Busy => PressurePolicyError::Busy,
        CorePressurePolicyError::UnsafeStorage => PressurePolicyError::UnsafeStorage,
        CorePressurePolicyError::QueryLimitExceeded => PressurePolicyError::BudgetExceeded,
        CorePressurePolicyError::CorruptData => PressurePolicyError::CorruptData,
        CorePressurePolicyError::Unavailable => PressurePolicyError::Unavailable,
        CorePressurePolicyError::OutcomeUnknown => PressurePolicyError::OutcomeUnknown,
        CorePressurePolicyError::InternalState => PressurePolicyError::InternalState,
        _ => PressurePolicyError::InternalState,
    }
}

fn pressure_policy_status(
    policy: CorePressurePolicy,
) -> Result<PressurePolicyStatus, PressurePolicyError> {
    let critical = policy.config.critical_threshold();
    let warning = policy.config.warning_threshold();
    let recovery = policy.config.recovery_margin();
    let updated_at_unix_ms = policy.updated_at.map(pressure_policy_time_ms).transpose()?;
    if (policy.revision == 0) != updated_at_unix_ms.is_none()
        || (policy.revision == 0 && policy.source != CorePressurePolicySource::Default)
    {
        return Err(PressurePolicyError::InternalState);
    }
    Ok(PressurePolicyStatus {
        record_version: FFI_RECORD_VERSION,
        source: match policy.source {
            CorePressurePolicySource::Default => PressurePolicySource::Default,
            CorePressurePolicySource::Stored => PressurePolicySource::Stored,
        },
        revision: policy.revision,
        critical_available_bytes: critical.maximum_available_bytes(),
        critical_available_basis_points: critical.maximum_available_basis_points(),
        warning_available_bytes: warning.maximum_available_bytes(),
        warning_available_basis_points: warning.maximum_available_basis_points(),
        recovery_bytes: recovery.bytes(),
        recovery_basis_points: recovery.basis_points(),
        updated_at_unix_ms,
    })
}

fn pressure_policy_update(
    update: CorePressurePolicyUpdate,
) -> Result<PressurePolicyUpdate, PressurePolicyError> {
    Ok(PressurePolicyUpdate {
        record_version: FFI_RECORD_VERSION,
        policy: pressure_policy_status(update.settings)?,
        changed: update.changed,
    })
}

fn pressure_policy_time_ms(value: SystemTime) -> Result<i64, PressurePolicyError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PressurePolicyError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| PressurePolicyError::InternalState)
}

fn map_volume_pressure(pressure: CoreDiskPressure) -> VolumePressure {
    match pressure {
        CoreDiskPressure::Healthy => VolumePressure::Healthy,
        CoreDiskPressure::Warning => VolumePressure::Warning,
        CoreDiskPressure::Critical => VolumePressure::Critical,
        CoreDiskPressure::Unknown => VolumePressure::Unknown,
    }
}

fn map_capacity_source(source: CoreCapacitySource) -> VolumeCapacitySource {
    match source {
        CoreCapacitySource::ImportantUsage => VolumeCapacitySource::ImportantUsage,
        CoreCapacitySource::Ordinary => VolumeCapacitySource::Ordinary,
    }
}

fn map_history_disposition(disposition: CoreHistoryDisposition) -> VolumeHistoryDisposition {
    match disposition {
        CoreHistoryDisposition::Stored => VolumeHistoryDisposition::Stored,
        CoreHistoryDisposition::ExistingExact => VolumeHistoryDisposition::ExistingExact,
        CoreHistoryDisposition::SuppressedByHourlyCadence => {
            VolumeHistoryDisposition::SuppressedByHourlyCadence
        }
        CoreHistoryDisposition::NotStoredMissingOrdinaryAvailability => {
            VolumeHistoryDisposition::NotStoredMissingOrdinaryAvailability
        }
        CoreHistoryDisposition::NotStoredMissingStableIdentity => {
            VolumeHistoryDisposition::NotStoredMissingStableIdentity
        }
        CoreHistoryDisposition::NotStoredIncompleteMetadata => {
            VolumeHistoryDisposition::NotStoredIncompleteMetadata
        }
    }
}

fn parse_macos_volume_id(value: Option<String>) -> Result<Option<VolumeId>, EngineError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    let bytes = value.as_bytes();
    let canonical_shape = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        });
    if !canonical_shape {
        return Err(EngineError::InvalidCapacityObservation);
    }
    VolumeId::new(format!("volume:macos:{}", value.to_ascii_lowercase()))
        .map(Some)
        .map_err(|_| EngineError::InvalidCapacityObservation)
}

fn unix_ms_to_system_time(value: i64) -> Result<SystemTime, EngineError> {
    let millis = u64::try_from(value).map_err(|_| EngineError::InvalidCapacityObservation)?;
    UNIX_EPOCH
        .checked_add(Duration::from_millis(millis))
        .ok_or(EngineError::InvalidCapacityObservation)
}

fn map_open_error(error: EngineOpenError) -> EngineError {
    match error {
        EngineOpenError::CandidateCatalogInvalid => EngineError::InternalState,
        EngineOpenError::WorkerUnavailable => EngineError::RegistryUnavailable,
        EngineOpenError::Database(kind) => match kind {
            DatabaseOpenErrorKind::UnsafeStorageRoot
            | DatabaseOpenErrorKind::UnsafeStorageObject
            | DatabaseOpenErrorKind::OwnershipMismatch
            | DatabaseOpenErrorKind::UnsafePermissions
            | DatabaseOpenErrorKind::UnrecognizedDatabase => EngineError::UnsafeStorage,
            DatabaseOpenErrorKind::Busy => EngineError::Busy,
            DatabaseOpenErrorKind::InspectionLimitExceeded => EngineError::BudgetExceeded,
            DatabaseOpenErrorKind::CorruptDatabase => EngineError::CorruptData,
            DatabaseOpenErrorKind::StorageRootUnavailable
            | DatabaseOpenErrorKind::DatabaseUnavailable
            | DatabaseOpenErrorKind::MigrationFailed => EngineError::StorageUnavailable,
            DatabaseOpenErrorKind::InternalState => EngineError::InternalState,
        },
        EngineOpenError::Snapshot(kind) => match kind {
            SnapshotOpenErrorKind::InvalidConfiguration => EngineError::InvalidStorage,
            SnapshotOpenErrorKind::UnsafeRoot
            | SnapshotOpenErrorKind::UnsafeObject
            | SnapshotOpenErrorKind::UnrecognizedStore => EngineError::UnsafeStorage,
            SnapshotOpenErrorKind::Unavailable => EngineError::StorageUnavailable,
            SnapshotOpenErrorKind::Busy => EngineError::Busy,
            SnapshotOpenErrorKind::InternalState => EngineError::InternalState,
        },
    }
}
fn map_start_error(error: StartTaskError) -> EngineError {
    match error {
        StartTaskError::Closed => EngineError::Closed,
        StartTaskError::ReadOnlyStore => EngineError::ReadOnlyStore,
        StartTaskError::PersistenceUnavailable => EngineError::StorageUnavailable,
        StartTaskError::InternalState | StartTaskError::TaskIdExhausted => {
            EngineError::RegistryUnavailable
        }
        _ => EngineError::InternalState,
    }
}
fn map_task_access_error(error: dux_core::engine::TaskAccessError) -> EngineError {
    match error {
        dux_core::engine::TaskAccessError::Closed => EngineError::Closed,
        dux_core::engine::TaskAccessError::InternalState => EngineError::RegistryUnavailable,
        _ => EngineError::InternalState,
    }
}
fn system_time_ms(value: SystemTime) -> Result<i64, EngineError> {
    i64::try_from(
        value
            .duration_since(UNIX_EPOCH)
            .map_err(|_| EngineError::InternalState)?
            .as_millis(),
    )
    .map_err(|_| EngineError::InternalState)
}

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    static ENGINE_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn engine() -> (TempDir, DuxEngine) {
        let temp = TempDir::new().unwrap();
        let engine = DuxEngine::new(EngineStorageRoots {
            data_root: temp.path().join("data").to_string_lossy().into_owned(),
            cache_root: temp.path().join("cache").to_string_lossy().into_owned(),
        })
        .unwrap();
        (temp, engine)
    }

    #[test]
    fn reports_contract_six_and_preserves_legacy_formatting() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        assert_eq!(library_version().ffi_contract_version, 6);
        assert_eq!(engine.library_version().unwrap(), library_version());
        assert_eq!(engine.format_size(1536).unwrap().display, "1.5 KB");
        assert!(engine.close());
        assert_eq!(engine.format_size(1), Err(EngineError::Closed));
    }

    fn startup_observation(
        sampled_at_unix_ms: i64,
        ordinary_available_bytes: Option<u64>,
        important_available_bytes: Option<u64>,
    ) -> StartupVolumeObservation {
        StartupVolumeObservation {
            record_version: FFI_RECORD_VERSION,
            stable_volume_id: Some("01234567-89AB-CDEF-0123-456789ABCDEF".into()),
            display_name: Some("Macintosh HD".into()),
            filesystem: Some("APFS".into()),
            is_internal: Some(true),
            is_removable: Some(false),
            sampled_at_unix_ms,
            total_bytes: 1_024 * 1_024 * 1_024 * 1_024,
            ordinary_available_bytes,
            important_available_bytes,
        }
    }

    fn default_pressure_policy_input() -> PressurePolicyInput {
        let gib = 1_024 * 1_024 * 1_024;
        PressurePolicyInput {
            record_version: FFI_RECORD_VERSION,
            critical_available_bytes: 10 * gib,
            critical_available_basis_points: 500,
            warning_available_bytes: 30 * gib,
            warning_available_basis_points: 1_000,
            recovery_bytes: 2 * gib,
            recovery_basis_points: 100,
        }
    }

    #[test]
    fn pressure_policy_get_set_reset_is_versioned_typed_and_path_free() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let initial = engine.get_disk_pressure_policy().unwrap();
        assert_eq!(initial.record_version, 1);
        assert_eq!(initial.source, PressurePolicySource::Default);
        assert_eq!(initial.revision, 0);
        assert_eq!(initial.updated_at_unix_ms, None);

        let stored = engine
            .set_disk_pressure_policy(default_pressure_policy_input())
            .unwrap();
        assert!(stored.changed);
        assert_eq!(stored.record_version, 1);
        assert_eq!(stored.policy.source, PressurePolicySource::Stored);
        assert_eq!(stored.policy.revision, 1);
        assert!(stored.policy.updated_at_unix_ms.is_some());
        assert_eq!(engine.get_disk_pressure_policy().unwrap(), stored.policy);

        let exact = engine
            .set_disk_pressure_policy(default_pressure_policy_input())
            .unwrap();
        assert!(!exact.changed);
        assert_eq!(exact.policy, stored.policy);

        let reset = engine.reset_disk_pressure_policy().unwrap();
        assert!(reset.changed);
        assert_eq!(reset.policy.source, PressurePolicySource::Default);
        assert_eq!(reset.policy.revision, 2);
        assert!(reset.policy.updated_at_unix_ms.is_some());
        let exact_reset = engine.reset_disk_pressure_policy().unwrap();
        assert!(!exact_reset.changed);
        assert_eq!(exact_reset.policy, reset.policy);

        assert!(engine.close());
        assert_eq!(
            engine.get_disk_pressure_policy(),
            Err(PressurePolicyError::Closed)
        );
        assert_eq!(
            engine.set_disk_pressure_policy(default_pressure_policy_input()),
            Err(PressurePolicyError::Closed)
        );
        assert_eq!(
            engine.reset_disk_pressure_policy(),
            Err(PressurePolicyError::Closed)
        );
    }

    #[test]
    fn pressure_policy_input_maps_every_validation_boundary() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();

        let mut value = default_pressure_policy_input();
        value.record_version += 1;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::InvalidRecordVersion)
        );
        let mut value = default_pressure_policy_input();
        value.critical_available_bytes = 0;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::ThresholdBytesZero)
        );
        let mut value = default_pressure_policy_input();
        value.critical_available_basis_points = 0;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::ThresholdBasisPointsOutOfRange)
        );
        let mut value = default_pressure_policy_input();
        value.warning_available_bytes = value.critical_available_bytes - 1;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::WarningBytesBelowCritical)
        );
        let mut value = default_pressure_policy_input();
        value.warning_available_basis_points = value.critical_available_basis_points - 1;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::WarningBasisPointsBelowCritical)
        );
        let mut value = default_pressure_policy_input();
        value.warning_available_bytes = value.critical_available_bytes;
        value.warning_available_basis_points = value.critical_available_basis_points;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::WarningThresholdMatchesCritical)
        );
        let mut value = default_pressure_policy_input();
        value.recovery_bytes = 0;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::RecoveryBytesZero)
        );
        let mut value = default_pressure_policy_input();
        value.recovery_basis_points = 0;
        assert_eq!(
            engine.set_disk_pressure_policy(value),
            Err(PressurePolicyError::RecoveryBasisPointsOutOfRange)
        );
    }

    #[test]
    fn startup_volume_round_trip_is_path_free_versioned_and_honest() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let gib = 1_024 * 1_024 * 1_024;
        let stored = engine
            .observe_startup_volume(startup_observation(
                3_600_000,
                Some(100 * gib),
                Some(20 * gib),
            ))
            .unwrap();
        assert_eq!(stored.record_version, 1);
        assert_eq!(
            stored.stable_volume_id.as_deref(),
            Some("volume:macos:01234567-89ab-cdef-0123-456789abcdef")
        );
        assert_eq!(stored.pressure, VolumePressure::Warning);
        assert_eq!(stored.headline_source, VolumeCapacitySource::ImportantUsage);
        assert_eq!(stored.history_disposition, VolumeHistoryDisposition::Stored);
        assert_eq!(stored.previous_durable_pressure, None);

        let important_only = engine
            .observe_startup_volume(startup_observation(3_600_001, None, Some(5 * gib)))
            .unwrap();
        assert_eq!(important_only.ordinary_available_bytes, None);
        assert_eq!(important_only.pressure, VolumePressure::Critical);
        assert_eq!(
            important_only.history_disposition,
            VolumeHistoryDisposition::NotStoredMissingOrdinaryAvailability
        );
        assert_eq!(
            important_only.previous_durable_pressure,
            Some(VolumePressure::Warning)
        );
    }

    #[test]
    fn startup_volume_rejects_malformed_input_and_use_after_close() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let gib = 1_024 * 1_024 * 1_024;
        let mut invalid_id = startup_observation(1, Some(gib), Some(gib));
        invalid_id.stable_volume_id = Some("not-a-uuid".into());
        assert_eq!(
            engine.observe_startup_volume(invalid_id),
            Err(EngineError::InvalidCapacityObservation)
        );
        let mut wrong_version = startup_observation(1, Some(gib), Some(gib));
        wrong_version.record_version = FFI_RECORD_VERSION + 1;
        assert_eq!(
            engine.observe_startup_volume(wrong_version),
            Err(EngineError::InvalidCapacityObservation)
        );
        assert_eq!(
            engine.observe_startup_volume(startup_observation(-1, Some(gib), Some(gib))),
            Err(EngineError::InvalidCapacityObservation)
        );
        assert_eq!(
            engine.observe_startup_volume(startup_observation(2, None, None)),
            Err(EngineError::InvalidCapacityObservation)
        );
        assert!(engine.close());
        assert_eq!(
            engine.observe_startup_volume(startup_observation(3, Some(gib), Some(gib))),
            Err(EngineError::Closed)
        );
    }

    #[test]
    fn all_maintenance_kinds_submit_and_poll_path_free_results() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        for kind in [
            MaintenanceKind::ScanRecovery,
            MaintenanceKind::History,
            MaintenanceKind::SnapshotRetention,
            MaintenanceKind::SnapshotOrphan,
            MaintenanceKind::SnapshotProvisioningStage,
            MaintenanceKind::SnapshotTerminalTemp,
            MaintenanceKind::SnapshotUnleasedTemp,
        ] {
            let start = engine.start_maintenance(kind).unwrap();
            assert_eq!(start.record_version, 1);
            let task = start.task.expect("started task");
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let poll = task.poll().unwrap();
                assert_eq!(poll.kind, kind);
                if matches!(
                    poll.phase,
                    TaskPhase::Succeeded | TaskPhase::Failed | TaskPhase::Cancelled
                ) {
                    assert!(poll.phase != TaskPhase::Succeeded || poll.result.is_some());
                    if kind == MaintenanceKind::ScanRecovery && poll.phase == TaskPhase::Succeeded {
                        let result = poll.result.as_ref().unwrap();
                        assert_eq!(result.outcome, MaintenanceOutcome::ScanRecoveryNone);
                        assert_eq!(result.primary_count_before, 0);
                        assert_eq!(result.primary_count_after, 0);
                        assert_eq!(result.secondary_count_before, 0);
                        assert_eq!(result.tertiary_count_before, 0);
                        assert_eq!(result.quaternary_count_before, 0);
                        assert_eq!(result.removed_bytes, 0);
                    }
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
        assert!(engine.close());
    }

    #[test]
    fn invalid_storage_scan_ids_and_missing_scans_are_typed() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        assert!(matches!(
            DuxEngine::new(EngineStorageRoots {
                data_root: "relative".into(),
                cache_root: "relative".into()
            }),
            Err(EngineError::InvalidStorage)
        ));
        let (_temp, engine) = engine();
        assert!(matches!(
            engine.acquire_explorer_snapshot_review("bad scan".into()),
            Err(EngineError::InvalidScanId)
        ));
        assert!(matches!(
            engine.acquire_explorer_snapshot_review("scan:missing".into()),
            Err(EngineError::ScanNotFound)
        ));
        assert!(engine.close());
    }

    #[test]
    fn opaque_review_session_renews_and_releases_exact_scan() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (temp, engine) = engine();
        let root = temp.path().join("review-root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("payload"), b"ffi review").unwrap();
        let task = engine
            .with_engine(|core| core.start_scan(root).map_err(map_start_error))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let scan_id = loop {
            let observation = engine
                .with_engine(|core| {
                    let snapshot = core.task_snapshot(task).map_err(map_task_access_error)?;
                    if snapshot.phase == CoreTaskPhase::Succeeded {
                        Ok(core
                            .scan_result(task)
                            .map_err(map_task_access_error)?
                            .map(|result| result.scan_id().as_str().to_owned()))
                    } else {
                        Ok(None)
                    }
                })
                .unwrap();
            if let Some(scan_id) = observation {
                break scan_id;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        };

        let review = engine
            .acquire_explorer_snapshot_review(scan_id.clone())
            .unwrap();
        let initial = review.info().unwrap();
        assert_eq!(initial.scan_id, scan_id);
        assert!(!initial.released);
        assert!(initial.expires_at_unix_ms > 0);
        let renewed = review.renew().unwrap();
        assert!(renewed.expires_at_unix_ms >= initial.expires_at_unix_ms);
        let close_drained = engine
            .acquire_explorer_snapshot_review(scan_id.clone())
            .unwrap();
        assert_eq!(review.release().unwrap(), ReviewReleaseOutcome::Released);
        assert_eq!(
            review.release().unwrap(),
            ReviewReleaseOutcome::AlreadyReleased
        );
        let ended = review.info().unwrap();
        assert!(ended.released);
        assert_eq!(ended.expires_at_unix_ms, 0);
        assert!(engine.close());
        assert_eq!(close_drained.renew(), Err(EngineError::Closed));
        assert!(close_drained.info().unwrap().released);
        assert_eq!(
            close_drained.release().unwrap(),
            ReviewReleaseOutcome::AlreadyReleased
        );
    }

    #[test]
    fn concurrent_close_callers_observe_the_same_result() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let (_temp, engine) = engine();
        let engine = Arc::new(engine);
        let barrier = Arc::new(std::sync::Barrier::new(9));
        let callers = (0..8)
            .map(|_| {
                let engine = Arc::clone(&engine);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    engine.close()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        for caller in callers {
            assert!(caller.join().unwrap());
        }
        assert!(engine.close());
    }

    #[test]
    fn close_is_idempotent_and_live_count_tracks_objects() {
        let _guard = ENGINE_TEST_LOCK.lock().unwrap();
        let baseline = live_engine_instance_count();
        let (_temp, engine) = engine();
        assert_eq!(live_engine_instance_count(), baseline + 1);
        assert!(engine.close());
        assert!(engine.close());
        drop(engine);
        assert_eq!(live_engine_instance_count(), baseline);
    }
}
