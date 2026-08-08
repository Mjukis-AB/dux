//! Private immutable snapshot-file storage.
//!
//! This layer owns only publication and file identity. Snapshot decoding,
//! checksum/version validation, retention, and database references belong to
//! higher layers. In particular, an existing exact-ID file is returned to the
//! caller for full codec validation; its name alone never proves idempotence.

use std::fmt;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use fs4::{FileExt, TryLockError};
use sha2::{Digest, Sha256};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::persistence::{
    AppDataResetSnapshotPayloadDrainAuthority, AppDataResetSnapshotStoreRetireAuthority,
};

use super::codec::MAX_SNAPSHOT_FILE_BYTES;

#[path = "storage/app_data_reset.rs"]
mod app_data_reset;
#[path = "storage/inventory_lease.rs"]
mod inventory_lease;

const DIRECTORY_NAME: &str = "snapshots";
const MARKER_NAME: &str = ".dux-snapshot-store";
const WRITER_LOCK_NAME: &str = ".dux-snapshot.writer.lock";
const STORE_MARKER: &[u8; 16] = b"DUXSNAPSTOREV1\0\0";
const WRITER_MARKER: &[u8; 16] = b"DUXSNAPWRITER1\0\0";
const FINAL_PREFIX: &str = "snapshot-";
const FINAL_SUFFIX: &str = ".duxsnapshot";
const FINAL_HEX_LENGTH: usize = 64;
const RANDOM_ATTEMPTS: usize = 16;
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(5);
const OPEN_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const INVENTORY_DEADLINE: Duration = Duration::from_millis(250);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const APP_DATA_RESET_POST_EFFECT_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_INVENTORY_ENTRIES: usize = 2_048;
const MAX_INVENTORY_NAME_BYTES: usize = 256 * 1_024;
const MAX_RECOGNIZED_TEMPS: usize = 64;
const PROVISIONING_STAGE_PREFIX: &str = ".dux-snapshot-stage-";
const PROVISIONING_STAGE_HEX_LENGTH: usize = 32;
const MAX_PROVISIONING_STAGES: usize = 64;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_PAYLOAD_DRAIN_BEFORE_EFFECT: u8 = 1;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_PAYLOAD_DRAIN_AFTER_EFFECT: u8 = 2;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_PAYLOAD_DRAIN_AFTER_DIRECTORY_SYNC: u8 = 3;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_PAYLOAD_DRAIN_DURING_READBACK: u8 = 4;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
const TEST_FAULT_RESET_PAYLOAD_DRAIN_EXHAUST_PRE_EFFECT_DEADLINE: u8 = 5;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
const TEST_FAULT_RESET_PAYLOAD_DRAIN_EXHAUST_POST_EFFECT_DEADLINE: u8 = 6;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_PAYLOAD_DRAIN_FOREIGN_STORE_FILESYSTEM: u8 = 7;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_PAYLOAD_DRAIN_FOREIGN_PAYLOAD_FILESYSTEM: u8 = 8;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_PAYLOAD_DRAIN_FOREIGN_FILESYSTEM_AT_FINAL_GATE: u8 = 9;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
const TEST_FAULT_RESET_PAYLOAD_DRAIN_EXHAUST_DEADLINE_AT_FINAL_GATE: u8 = 10;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_STORE_RETIRE_BEFORE_EFFECT: u8 = 1;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_STORE_RETIRE_AFTER_EFFECT: u8 = 2;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_STORE_RETIRE_AFTER_DIRECTORY_SYNC: u8 = 3;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_STORE_RETIRE_DURING_READBACK: u8 = 4;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
const TEST_FAULT_RESET_STORE_RETIRE_EXHAUST_PRE_EFFECT_DEADLINE: u8 = 5;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
const TEST_FAULT_RESET_STORE_RETIRE_EXHAUST_POST_EFFECT_DEADLINE: u8 = 6;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const TEST_FAULT_RESET_STORE_RETIRE_FOREIGN_FILESYSTEM_AT_FINAL_GATE: u8 = 7;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
const TEST_FAULT_RESET_STORE_RETIRE_EXHAUST_DEADLINE_AT_FINAL_GATE: u8 = 8;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
const TEST_FAULT_RESET_STORE_RETIRE_CASE_ALIAS_AT_FINAL_GATE: u8 = 9;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
std::thread_local! {
    static TEST_RESET_PAYLOAD_DRAIN_FAULT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    static TEST_RESET_STORE_RETIRE_FAULT: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn take_test_reset_payload_drain_fault(expected: u8) -> bool {
    #[cfg(test)]
    {
        TEST_RESET_PAYLOAD_DRAIN_FAULT.with(|fault| {
            if fault.get() == expected {
                fault.set(0);
                true
            } else {
                false
            }
        })
    }
    #[cfg(not(test))]
    {
        let _ = expected;
        false
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn take_test_reset_store_retire_fault(expected: u8) -> bool {
    #[cfg(test)]
    {
        TEST_RESET_STORE_RETIRE_FAULT.with(|fault| {
            if fault.get() == expected {
                fault.set(0);
                true
            } else {
                false
            }
        })
    }
    #[cfg(not(test))]
    {
        let _ = expected;
        false
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestAppDataResetSnapshotPayloadDrainFault {
    BeforeEffect,
    AfterEffect,
    AfterDirectorySync,
    DuringReadback,
    ExhaustPreEffectDeadline,
    ExhaustPostEffectDeadline,
    ForeignStoreFilesystem,
    ForeignPayloadFilesystem,
    ForeignFilesystemAtFinalGate,
    ExhaustDeadlineAtFinalGate,
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn set_test_app_data_reset_snapshot_payload_drain_fault(
    fault: TestAppDataResetSnapshotPayloadDrainFault,
) {
    let value = match fault {
        TestAppDataResetSnapshotPayloadDrainFault::BeforeEffect => {
            TEST_FAULT_RESET_PAYLOAD_DRAIN_BEFORE_EFFECT
        }
        TestAppDataResetSnapshotPayloadDrainFault::AfterEffect => {
            TEST_FAULT_RESET_PAYLOAD_DRAIN_AFTER_EFFECT
        }
        TestAppDataResetSnapshotPayloadDrainFault::AfterDirectorySync => {
            TEST_FAULT_RESET_PAYLOAD_DRAIN_AFTER_DIRECTORY_SYNC
        }
        TestAppDataResetSnapshotPayloadDrainFault::DuringReadback => {
            TEST_FAULT_RESET_PAYLOAD_DRAIN_DURING_READBACK
        }
        TestAppDataResetSnapshotPayloadDrainFault::ExhaustPreEffectDeadline => {
            TEST_FAULT_RESET_PAYLOAD_DRAIN_EXHAUST_PRE_EFFECT_DEADLINE
        }
        TestAppDataResetSnapshotPayloadDrainFault::ExhaustPostEffectDeadline => {
            TEST_FAULT_RESET_PAYLOAD_DRAIN_EXHAUST_POST_EFFECT_DEADLINE
        }
        TestAppDataResetSnapshotPayloadDrainFault::ForeignStoreFilesystem => {
            TEST_FAULT_RESET_PAYLOAD_DRAIN_FOREIGN_STORE_FILESYSTEM
        }
        TestAppDataResetSnapshotPayloadDrainFault::ForeignPayloadFilesystem => {
            TEST_FAULT_RESET_PAYLOAD_DRAIN_FOREIGN_PAYLOAD_FILESYSTEM
        }
        TestAppDataResetSnapshotPayloadDrainFault::ForeignFilesystemAtFinalGate => {
            TEST_FAULT_RESET_PAYLOAD_DRAIN_FOREIGN_FILESYSTEM_AT_FINAL_GATE
        }
        TestAppDataResetSnapshotPayloadDrainFault::ExhaustDeadlineAtFinalGate => {
            TEST_FAULT_RESET_PAYLOAD_DRAIN_EXHAUST_DEADLINE_AT_FINAL_GATE
        }
    };
    TEST_RESET_PAYLOAD_DRAIN_FAULT.with(|current| current.set(value));
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestAppDataResetSnapshotStoreRetirementFault {
    BeforeEffect,
    AfterEffect,
    AfterDirectorySync,
    DuringReadback,
    ExhaustPreEffectDeadline,
    ExhaustPostEffectDeadline,
    ForeignFilesystemAtFinalGate,
    ExhaustDeadlineAtFinalGate,
    CaseAliasAtFinalGate,
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn set_test_app_data_reset_snapshot_store_retirement_fault(
    fault: TestAppDataResetSnapshotStoreRetirementFault,
) {
    let value = match fault {
        TestAppDataResetSnapshotStoreRetirementFault::BeforeEffect => {
            TEST_FAULT_RESET_STORE_RETIRE_BEFORE_EFFECT
        }
        TestAppDataResetSnapshotStoreRetirementFault::AfterEffect => {
            TEST_FAULT_RESET_STORE_RETIRE_AFTER_EFFECT
        }
        TestAppDataResetSnapshotStoreRetirementFault::AfterDirectorySync => {
            TEST_FAULT_RESET_STORE_RETIRE_AFTER_DIRECTORY_SYNC
        }
        TestAppDataResetSnapshotStoreRetirementFault::DuringReadback => {
            TEST_FAULT_RESET_STORE_RETIRE_DURING_READBACK
        }
        TestAppDataResetSnapshotStoreRetirementFault::ExhaustPreEffectDeadline => {
            TEST_FAULT_RESET_STORE_RETIRE_EXHAUST_PRE_EFFECT_DEADLINE
        }
        TestAppDataResetSnapshotStoreRetirementFault::ExhaustPostEffectDeadline => {
            TEST_FAULT_RESET_STORE_RETIRE_EXHAUST_POST_EFFECT_DEADLINE
        }
        TestAppDataResetSnapshotStoreRetirementFault::ForeignFilesystemAtFinalGate => {
            TEST_FAULT_RESET_STORE_RETIRE_FOREIGN_FILESYSTEM_AT_FINAL_GATE
        }
        TestAppDataResetSnapshotStoreRetirementFault::ExhaustDeadlineAtFinalGate => {
            TEST_FAULT_RESET_STORE_RETIRE_EXHAUST_DEADLINE_AT_FINAL_GATE
        }
        TestAppDataResetSnapshotStoreRetirementFault::CaseAliasAtFinalGate => {
            TEST_FAULT_RESET_STORE_RETIRE_CASE_ALIAS_AT_FINAL_GATE
        }
    };
    TEST_RESET_STORE_RETIRE_FAULT.with(|current| current.set(value));
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
fn exhaust_test_deadline(deadline: Instant) {
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(remaining.min(Duration::from_millis(1)));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotStorageErrorKind {
    InvalidConfiguration,
    UnsafeRoot,
    UnsafeObject,
    UnrecognizedStore,
    Unavailable,
    Busy,
    InternalState,
}

#[derive(Debug)]
pub(crate) struct SnapshotStorageError {
    kind: SnapshotStorageErrorKind,
}

/// Exact effect boundary for one observed-final removal. `BeforeEffect`
/// guarantees that no unlink/disposition succeeded. `OutcomeUnknown` means
/// the namespace mutation succeeded but its directory durability could not be
/// established.
#[derive(Debug)]
pub(crate) enum SnapshotFinalRemovalError {
    BeforeEffect(SnapshotStorageError),
    OutcomeUnknown,
}

/// Exact effect boundary for one observed temporary-file removal.
/// `BeforeEffect` guarantees that no unlink/disposition succeeded.
/// `OutcomeUnknown` means the namespace mutation succeeded but its directory
/// durability could not be established.
#[derive(Debug)]
pub(crate) enum SnapshotTempRemovalError {
    BeforeEffect(SnapshotStorageError),
    OutcomeUnknown,
}

/// Exact effect boundary for one marker-owned provisioning-stage removal.
/// `BeforeEffect` guarantees that no child unlink/disposition or directory
/// removal succeeded. After the first namespace mutation, every failure is
/// reported as `OutcomeUnknown` so callers must re-inventory from scratch.
#[derive(Debug)]
pub(crate) enum SnapshotProvisioningStageRemovalError {
    BeforeEffect(SnapshotStorageError),
    OutcomeUnknown,
}

/// Exact effect boundary for one app-data-reset snapshot payload removal.
/// `BeforeEffect` guarantees that no unlink/disposition succeeded. Once an
/// unlink succeeds, any durability or exact-readback failure is outcome
/// unknown and requires a fresh bounded inventory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) enum AppDataResetSnapshotPayloadDrainError {
    BeforeEffect(SnapshotStorageErrorKind),
    OutcomeUnknown,
}

/// Path- and byte-free progress from one bounded snapshot payload effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) struct AppDataResetSnapshotPayloadDrainBatch {
    removed_objects: u8,
    snapshot_payload_has_more: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetSnapshotPayloadDrainBatch {
    pub(crate) const fn removed_objects(self) -> u8 {
        self.removed_objects
    }

    pub(crate) const fn snapshot_payload_has_more(self) -> bool {
        self.snapshot_payload_has_more
    }
}

/// Internal certainty transport for the higher reset coordinator. The fixed
/// post-effect deadline never crosses into path-free progress or presentation.
#[derive(Debug)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) struct AppDataResetSnapshotPayloadDrainCompletion {
    progress: AppDataResetSnapshotPayloadDrainBatch,
    post_effect_deadline: Instant,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetSnapshotPayloadDrainCompletion {
    pub(crate) const fn post_effect_deadline(&self) -> Instant {
        self.post_effect_deadline
    }

    pub(crate) fn into_progress(self) -> AppDataResetSnapshotPayloadDrainBatch {
        self.progress
    }
}

/// Exact monotonic structural tail of the detached old snapshot store.
///
/// `WriterOnly` is the sole accepted partial-control shape because DUX always
/// removes the ownership marker first while retaining the exclusively locked
/// writer control. A marker-only store, controls plus payloads in a partial
/// state, or any other child is unsafe rather than retirement progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) enum AppDataResetSnapshotStoreRetirementState {
    FullControlsEmpty,
    WriterOnly,
    EmptyDirectory,
    Absent,
}

/// Path- and byte-free progress from one bounded snapshot-store structural
/// effect. It does not claim reclaimed capacity or completed reset state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) struct AppDataResetSnapshotStoreRetirementBatch {
    removed_structural_objects: u8,
    snapshot_store_has_more: bool,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetSnapshotStoreRetirementBatch {
    pub(crate) const fn removed_structural_objects(self) -> u8 {
        self.removed_structural_objects
    }

    pub(crate) const fn snapshot_store_has_more(self) -> bool {
        self.snapshot_store_has_more
    }
}

/// Effect certainty for one snapshot-store control or directory retirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) enum AppDataResetSnapshotStoreRetirementError {
    BeforeEffect(SnapshotStorageErrorKind),
    OutcomeUnknown,
}

/// Internal certainty transport for the reset coordinator. The deadline is
/// minted only after a structural unlink/rmdir succeeds and never crosses the
/// path-free progress boundary.
#[derive(Debug)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) struct AppDataResetSnapshotStoreRetirementCompletion {
    progress: AppDataResetSnapshotStoreRetirementBatch,
    post_effect_deadline: Instant,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetSnapshotStoreRetirementCompletion {
    pub(crate) const fn post_effect_deadline(&self) -> Instant {
        self.post_effect_deadline
    }

    pub(crate) fn into_progress(self) -> AppDataResetSnapshotStoreRetirementBatch {
        self.progress
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotProvisioningStageRemoval {
    NoStage,
    DeferredUnproven,
    RemovedMarkerOnly,
    RemovedMarkerComplete,
}

/// One complete bounded observation and, when possible, one durable removal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotProvisioningStageReconciliation {
    outcome: SnapshotProvisioningStageRemoval,
    total_stage_count_before: u64,
    total_stage_count_after: u64,
    marker_owned_count_before: u64,
    marker_owned_count_after: u64,
    unproven_count_before: u64,
    unproven_count_after: u64,
    control_usage_before: SnapshotFileUsage,
    control_usage_after: SnapshotFileUsage,
    removed_control_usage: Option<SnapshotFileUsage>,
    has_more: bool,
}

impl SnapshotProvisioningStageReconciliation {
    pub(crate) const fn outcome(self) -> SnapshotProvisioningStageRemoval {
        self.outcome
    }

    pub(crate) const fn total_stage_count_before(self) -> u64 {
        self.total_stage_count_before
    }

    pub(crate) const fn total_stage_count_after(self) -> u64 {
        self.total_stage_count_after
    }

    pub(crate) const fn marker_owned_count_before(self) -> u64 {
        self.marker_owned_count_before
    }

    pub(crate) const fn marker_owned_count_after(self) -> u64 {
        self.marker_owned_count_after
    }

    pub(crate) const fn unproven_count_before(self) -> u64 {
        self.unproven_count_before
    }

    pub(crate) const fn unproven_count_after(self) -> u64 {
        self.unproven_count_after
    }

    pub(crate) const fn control_usage_before(self) -> SnapshotFileUsage {
        self.control_usage_before
    }

    pub(crate) const fn control_usage_after(self) -> SnapshotFileUsage {
        self.control_usage_after
    }

    pub(crate) const fn removed_control_usage(self) -> Option<SnapshotFileUsage> {
        self.removed_control_usage
    }

    pub(crate) const fn has_more(self) -> bool {
        self.has_more
    }
}

impl SnapshotStorageError {
    const fn new(kind: SnapshotStorageErrorKind) -> Self {
        Self { kind }
    }

    pub(crate) const fn kind(&self) -> SnapshotStorageErrorKind {
        self.kind
    }
}

impl fmt::Display for SnapshotStorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            SnapshotStorageErrorKind::InvalidConfiguration => {
                "invalid snapshot-store configuration"
            }
            SnapshotStorageErrorKind::UnsafeRoot => "unsafe snapshot-store root",
            SnapshotStorageErrorKind::UnsafeObject => "unsafe snapshot-store object",
            SnapshotStorageErrorKind::UnrecognizedStore => "unrecognized snapshot store",
            SnapshotStorageErrorKind::Unavailable => "snapshot store unavailable",
            SnapshotStorageErrorKind::Busy => "snapshot store is busy",
            SnapshotStorageErrorKind::InternalState => "invalid snapshot-store state",
        })
    }
}

impl std::error::Error for SnapshotStorageError {}

type Result<T> = std::result::Result<T, SnapshotStorageError>;

/// A validated, single-component immutable snapshot file name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SnapshotFileName(String);

impl SnapshotFileName {
    pub(crate) fn from_scan_id(scan_id: &[u8]) -> Self {
        let digest = Sha256::digest(scan_id);
        let mut value = String::with_capacity(FINAL_PREFIX.len() + 64 + FINAL_SUFFIX.len());
        value.push_str(FINAL_PREFIX);
        push_lower_hex(&mut value, digest.as_ref());
        value.push_str(FINAL_SUFFIX);
        Self(value)
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        let expected_length = FINAL_PREFIX.len() + FINAL_HEX_LENGTH + FINAL_SUFFIX.len();
        let digest_end = FINAL_PREFIX.len() + FINAL_HEX_LENGTH;
        if value.len() != expected_length
            || !value.starts_with(FINAL_PREFIX)
            || !value.ends_with(FINAL_SUFFIX)
            || !value.as_bytes()[FINAL_PREFIX.len()..digest_end]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::UnsafeObject,
            ));
        }
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

fn push_lower_hex(output: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity(platform::Identity);

struct StoreInner {
    database_root_path: PathBuf,
    database_root: File,
    database_root_identity: Identity,
    path: PathBuf,
    directory: File,
    directory_identity: Identity,
    marker: File,
    marker_identity: Identity,
    writer_lock: File,
    writer_lock_identity: Identity,
    writer_in_use: AtomicBool,
}

/// Independently marker-owned snapshot directory beneath a DUX database root.
#[derive(Clone)]
pub(crate) struct SecureSnapshotStore {
    inner: Arc<StoreInner>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotStoreAccess {
    ReadOnly,
    ReadWrite,
}

/// Exact point-in-time physical usage for one retained store object.
///
/// `charged_bytes` is deliberately conservative: sparse/compressed files can
/// report either logical or allocated size as the larger value on supported
/// filesystems, so retention budgets charge the maximum of both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SnapshotFileUsage {
    logical_bytes: u64,
    allocated_bytes: u64,
    charged_bytes: u64,
}

impl SnapshotFileUsage {
    fn from_sizes(logical_bytes: u64, allocated_bytes: u64) -> Self {
        Self {
            logical_bytes,
            allocated_bytes,
            charged_bytes: logical_bytes.max(allocated_bytes),
        }
    }

    fn checked_add(self, other: Self) -> Result<Self> {
        let logical_bytes = self
            .logical_bytes
            .checked_add(other.logical_bytes)
            .ok_or_else(unsafe_inventory_object)?;
        let allocated_bytes = self
            .allocated_bytes
            .checked_add(other.allocated_bytes)
            .ok_or_else(unsafe_inventory_object)?;
        let charged_bytes = self
            .charged_bytes
            .checked_add(other.charged_bytes)
            .ok_or_else(unsafe_inventory_object)?;
        Ok(Self {
            logical_bytes,
            allocated_bytes,
            charged_bytes,
        })
    }

    fn checked_sub(self, other: Self) -> Result<Self> {
        let logical_bytes = self
            .logical_bytes
            .checked_sub(other.logical_bytes)
            .ok_or_else(unsafe_inventory_object)?;
        let allocated_bytes = self
            .allocated_bytes
            .checked_sub(other.allocated_bytes)
            .ok_or_else(unsafe_inventory_object)?;
        let charged_bytes = self
            .charged_bytes
            .checked_sub(other.charged_bytes)
            .ok_or_else(unsafe_inventory_object)?;
        Ok(Self {
            logical_bytes,
            allocated_bytes,
            charged_bytes,
        })
    }

    pub(crate) const fn logical_bytes(self) -> u64 {
        self.logical_bytes
    }

    pub(crate) const fn allocated_bytes(self) -> u64 {
        self.allocated_bytes
    }

    pub(crate) const fn charged_bytes(self) -> u64 {
        self.charged_bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotInventoryEntryKind {
    Final(SnapshotFileName),
    /// A point-in-time observation whose process liveness is unknown.
    ///
    /// Inventory never treats this as reclaimable and exposes no mutation
    /// operation. A later scavenger needs its own durable liveness proof.
    RecognizedTemp,
}

/// Kernel-observed liveness for a recognized temporary snapshot file.
///
/// This observation is deliberately independent from the durable SQLite row:
/// the row binds a name to one staging operation, while only a contended
/// kernel lock proves that a writer still owns the file now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotTempKernelState {
    Active,
    Quiescent,
}

/// One no-follow, identity-checked object observed under the inventory lease.
///
/// The file descriptor is closed after its facts are captured. Keeping up to
/// 2,048 descriptors would exceed the common macOS launchd soft limit. The
/// store-wide writer lease preserves legitimate-name stability, and the entry
/// is reopened and identity-revalidated sequentially before handoff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotInventoryEntry {
    name: String,
    kind: SnapshotInventoryEntryKind,
    identity: Identity,
    usage: SnapshotFileUsage,
    temp_kernel_state: Option<SnapshotTempKernelState>,
}

impl SnapshotInventoryEntry {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn kind(&self) -> &SnapshotInventoryEntryKind {
        &self.kind
    }

    pub(crate) const fn usage(&self) -> SnapshotFileUsage {
        self.usage
    }

    pub(crate) const fn temp_kernel_state(&self) -> Option<SnapshotTempKernelState> {
        self.temp_kernel_state
    }

    fn revalidate(&self, store: &StoreInner) -> Result<()> {
        platform::validate_retained(
            &store.directory,
            store.directory_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        let Some((file, identity)) =
            platform::open_named_regular(&store.directory, &store.path, &self.name, false)?
        else {
            return Err(unsafe_inventory_object());
        };
        if Identity(identity) != self.identity {
            return Err(unsafe_inventory_object());
        }
        platform::validate_retained(&file, identity, platform::Kind::RegularFile, true)?;
        platform::validate_named(
            &store.directory,
            &self.name,
            &file,
            identity,
            platform::Kind::RegularFile,
        )?;
        match &self.kind {
            SnapshotInventoryEntryKind::Final(_) => {
                if snapshot_file_usage(&file)? != self.usage {
                    return Err(unsafe_inventory_object());
                }
            }
            SnapshotInventoryEntryKind::RecognizedTemp => {
                // A quiescent observation is stable only if a second
                // nonblocking probe remains quiescent and its usage is exact.
                // An initially active writer may finish or keep growing; the
                // higher layer retains the conservative Active classification.
                if self.temp_kernel_state == Some(SnapshotTempKernelState::Quiescent)
                    && (probe_temp_kernel_state(&file)? != SnapshotTempKernelState::Quiescent
                        || snapshot_file_usage(&file)? != self.usage)
                {
                    return Err(unsafe_inventory_object());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotControlUsage {
    store_marker: SnapshotFileUsage,
    writer_lock: SnapshotFileUsage,
    total: SnapshotFileUsage,
}

impl SnapshotControlUsage {
    pub(crate) const fn store_marker(self) -> SnapshotFileUsage {
        self.store_marker
    }

    pub(crate) const fn writer_lock(self) -> SnapshotFileUsage {
        self.writer_lock
    }

    pub(crate) const fn total(self) -> SnapshotFileUsage {
        self.total
    }
}

/// A complete bounded store observation protected by one writer lease.
///
/// Holding this value excludes legitimate publishers for the entire period in
/// which higher persistence layers match database references. Entry handles
/// are opened and closed sequentially so the 2,048-entry bound does not become
/// a file-descriptor requirement. Its mutations are limited to one re-proven
/// quiescent temp or one exact observed final; higher persistence must first
/// bind the name to durable same-scan authority, prove its absence from the
/// complete bounded temp-lease population, or establish tombstone/exact
/// unreferenced-orphan authority. The separate app-data-reset path instead
/// requires its coordinator-only journal/root/cache-absence capability and
/// exact same-filesystem old-store witness. The storage type itself offers no
/// generic temp, snapshot-policy, reset, or user-data cleanup authority.
pub(crate) struct SnapshotStoreInventoryLease {
    store: Arc<StoreInner>,
    entries: Vec<SnapshotInventoryEntry>,
    entries_usage: SnapshotFileUsage,
    controls: SnapshotControlUsage,
    total_usage: SnapshotFileUsage,
    _writer_lock: SnapshotWriterLock,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct AppDataResetSnapshotRetirementControl {
    file: File,
    identity: Identity,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct AppDataResetSnapshotPartialRetirementStore {
    database_root_path: PathBuf,
    database_root: File,
    database_root_identity: Identity,
    directory: File,
    directory_identity: Identity,
    writer: Option<AppDataResetSnapshotRetirementControl>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct AppDataResetSnapshotAbsentStore {
    database_root_path: PathBuf,
    database_root: File,
    database_root_identity: Identity,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
enum AppDataResetSnapshotRecoveryInner {
    Full(SnapshotStoreInventoryLease),
    Partial(AppDataResetSnapshotPartialRetirementStore),
    Absent(AppDataResetSnapshotAbsentStore),
}

/// Descriptor-retained snapshot-store observation used only by reset
/// recovery. Before `Draining` it accepts a complete normal store. During
/// `Draining` it additionally recognizes only the monotonic structural tail.
#[must_use = "the snapshot recovery observation must be consumed or revalidated"]
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) struct AppDataResetSnapshotRecovery {
    inner: AppDataResetSnapshotRecoveryInner,
    state: Option<AppDataResetSnapshotStoreRetirementState>,
    deadline: Instant,
}

/// Consume-once selection of the lexicographically first exact snapshot
/// payload under one retained writer inventory. This value contains no path
/// and grants no mutation without both its originating lease and the opaque
/// coordinator authority.
#[must_use = "the snapshot payload drain candidate must be consumed or revalidated"]
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) struct AppDataResetSnapshotPayloadDrainCandidate {
    store: Arc<StoreInner>,
    selected: SnapshotInventoryEntry,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl AppDataResetSnapshotPayloadDrainCandidate {
    pub(crate) fn is_bound_to(&self, lease: &SnapshotStoreInventoryLease) -> bool {
        Arc::ptr_eq(&self.store, &lease.store)
            && lease
                .entries
                .iter()
                .min_by(|left, right| left.name.cmp(&right.name))
                == Some(&self.selected)
            && self.selected_is_safe()
    }

    pub(crate) fn revalidate_against_until(
        &self,
        lease: &SnapshotStoreInventoryLease,
        deadline: Instant,
    ) -> Result<()> {
        if !self.is_bound_to(lease) {
            return Err(unsafe_inventory_object());
        }
        lease.revalidate_complete_for_app_data_reset_until(deadline)?;
        if self.is_bound_to(lease) {
            Ok(())
        } else {
            Err(unsafe_inventory_object())
        }
    }

    fn selected_is_safe(&self) -> bool {
        matches!(
            (&self.selected.kind, self.selected.temp_kernel_state),
            (SnapshotInventoryEntryKind::Final(_), None)
                | (
                    SnapshotInventoryEntryKind::RecognizedTemp,
                    Some(SnapshotTempKernelState::Quiescent)
                )
        )
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_app_data_reset_final_drain_error(
    error: SnapshotFinalRemovalError,
) -> AppDataResetSnapshotPayloadDrainError {
    match error {
        SnapshotFinalRemovalError::BeforeEffect(error) => {
            AppDataResetSnapshotPayloadDrainError::BeforeEffect(error.kind())
        }
        SnapshotFinalRemovalError::OutcomeUnknown => {
            AppDataResetSnapshotPayloadDrainError::OutcomeUnknown
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn map_app_data_reset_temp_drain_error(
    error: SnapshotTempRemovalError,
) -> AppDataResetSnapshotPayloadDrainError {
    match error {
        SnapshotTempRemovalError::BeforeEffect(error) => {
            AppDataResetSnapshotPayloadDrainError::BeforeEffect(error.kind())
        }
        SnapshotTempRemovalError::OutcomeUnknown => {
            AppDataResetSnapshotPayloadDrainError::OutcomeUnknown
        }
    }
}

struct SnapshotStorageInventory {
    entries: Vec<SnapshotInventoryEntry>,
    entries_usage: SnapshotFileUsage,
    controls: SnapshotControlUsage,
    total_usage: SnapshotFileUsage,
}

enum ProvisioningStageOpen {
    Private(File, platform::Identity),
    #[cfg_attr(windows, allow(dead_code))]
    Deferred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProvisioningStageShape {
    MarkerOnly,
    MarkerComplete,
}

struct RetainedProvisioningStage {
    name: String,
    directory: File,
    directory_identity: platform::Identity,
    marker: File,
    marker_identity: platform::Identity,
    writer_lock: Option<(File, platform::Identity)>,
    shape: ProvisioningStageShape,
    control_usage: SnapshotFileUsage,
}

struct ProvisioningStageInventory {
    total_stage_count: u64,
    marker_owned_count: u64,
    unproven_count: u64,
    control_usage: SnapshotFileUsage,
    first_marker_owned: Option<RetainedProvisioningStage>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_app_data_reset_database_root_path(database_root_path: &Path) -> Result<()> {
    if !database_root_path.is_absolute()
        || database_root_path.file_name().is_none()
        || database_root_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::InvalidConfiguration,
        ))
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_app_data_reset_snapshot_directory(
    database_root: &File,
    database_root_identity: Identity,
    directory: &File,
    directory_identity: Identity,
    deadline: Instant,
) -> Result<()> {
    if Instant::now() >= deadline
        || !platform::exact_name_exists(database_root, DIRECTORY_NAME, deadline)?
    {
        return Err(unsafe_inventory_object());
    }
    platform::validate_retained(
        database_root,
        database_root_identity.0,
        platform::Kind::Directory,
        false,
    )?;
    platform::validate_retained(
        directory,
        directory_identity.0,
        platform::Kind::Directory,
        false,
    )?;
    platform::validate_named(
        database_root,
        DIRECTORY_NAME,
        directory,
        directory_identity.0,
        platform::Kind::Directory,
    )?;
    if platform::same_filesystem(database_root_identity.0, directory_identity.0)
        && Instant::now() < deadline
    {
        Ok(())
    } else {
        Err(unsafe_inventory_object())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn app_data_reset_snapshot_retirement_inventory_names(
    directory: &File,
    deadline: Instant,
) -> Result<Vec<String>> {
    if Instant::now() >= deadline {
        return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
    }
    let mut names = platform::inventory(
        directory,
        MAX_INVENTORY_ENTRIES,
        MAX_INVENTORY_NAME_BYTES,
        deadline,
    )?;
    names.sort_unstable();
    if Instant::now() >= deadline {
        Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy))
    } else {
        Ok(names)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn acquire_app_data_reset_snapshot_retirement_writer_lock(
    writer: &File,
    deadline: Instant,
) -> Result<()> {
    loop {
        if Instant::now() >= deadline {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        match FileExt::try_lock(writer) {
            Ok(()) if Instant::now() >= deadline => {
                let _ = FileExt::unlock(writer);
                return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
            }
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => std::thread::sleep(
                LOCK_RETRY_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
            ),
            Err(TryLockError::Error(_)) => {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_app_data_reset_snapshot_partial_store(
    store: &AppDataResetSnapshotPartialRetirementStore,
    state: Option<AppDataResetSnapshotStoreRetirementState>,
    deadline: Instant,
) -> Result<()> {
    validate_app_data_reset_database_root_path(&store.database_root_path)?;
    validate_app_data_reset_snapshot_directory(
        &store.database_root,
        store.database_root_identity,
        &store.directory,
        store.directory_identity,
        deadline,
    )?;
    let names = app_data_reset_snapshot_retirement_inventory_names(&store.directory, deadline)?;
    match (state, store.writer.as_ref()) {
        (Some(AppDataResetSnapshotStoreRetirementState::WriterOnly), Some(writer))
            if names.as_slice() == [WRITER_LOCK_NAME] =>
        {
            platform::validate_retained(
                &writer.file,
                writer.identity.0,
                platform::Kind::RegularFile,
                true,
            )?;
            platform::validate_named(
                &store.directory,
                WRITER_LOCK_NAME,
                &writer.file,
                writer.identity.0,
                platform::Kind::RegularFile,
            )?;
            prove_marker(&writer.file, WRITER_MARKER)?;
            if !platform::same_filesystem(store.directory_identity.0, writer.identity.0) {
                return Err(unsafe_inventory_object());
            }
        }
        (Some(AppDataResetSnapshotStoreRetirementState::EmptyDirectory), None)
            if names.is_empty() => {}
        _ => return Err(unsafe_inventory_object()),
    }
    if Instant::now() >= deadline {
        Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy))
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_app_data_reset_snapshot_absence(
    store: &AppDataResetSnapshotAbsentStore,
    deadline: Instant,
) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
    }
    validate_app_data_reset_database_root_path(&store.database_root_path)?;
    platform::validate_retained(
        &store.database_root,
        store.database_root_identity.0,
        platform::Kind::Directory,
        false,
    )?;
    if platform::open_existing_directory(
        &store.database_root,
        &store.database_root_path,
        DIRECTORY_NAME,
    )?
    .is_some()
    {
        return Err(unsafe_inventory_object());
    }
    if Instant::now() >= deadline {
        Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy))
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_app_data_reset_snapshot_writer_only_after_marker(
    inventory: &SnapshotStoreInventoryLease,
    deadline: Instant,
) -> Result<()> {
    validate_app_data_reset_snapshot_directory(
        &inventory.store.database_root,
        inventory.store.database_root_identity,
        &inventory.store.directory,
        inventory.store.directory_identity,
        deadline,
    )?;
    if app_data_reset_snapshot_retirement_inventory_names(&inventory.store.directory, deadline)?
        .as_slice()
        != [WRITER_LOCK_NAME]
    {
        return Err(unsafe_inventory_object());
    }
    platform::validate_retained(
        &inventory.store.writer_lock,
        inventory.store.writer_lock_identity.0,
        platform::Kind::RegularFile,
        true,
    )?;
    platform::validate_named(
        &inventory.store.directory,
        WRITER_LOCK_NAME,
        &inventory.store.writer_lock,
        inventory.store.writer_lock_identity.0,
        platform::Kind::RegularFile,
    )?;
    prove_marker(&inventory.store.writer_lock, WRITER_MARKER)?;
    if !platform::same_filesystem(
        inventory.store.directory_identity.0,
        inventory.store.writer_lock_identity.0,
    ) {
        return Err(unsafe_inventory_object());
    }
    if Instant::now() >= deadline {
        Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy))
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_app_data_reset_snapshot_empty_after_writer(
    store: &AppDataResetSnapshotPartialRetirementStore,
    deadline: Instant,
) -> Result<()> {
    validate_app_data_reset_snapshot_directory(
        &store.database_root,
        store.database_root_identity,
        &store.directory,
        store.directory_identity,
        deadline,
    )?;
    if app_data_reset_snapshot_retirement_inventory_names(&store.directory, deadline)?.is_empty() {
        Ok(())
    } else {
        Err(unsafe_inventory_object())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn app_data_reset_snapshot_retirement_post_effect_deadline()
-> std::result::Result<Instant, AppDataResetSnapshotStoreRetirementError> {
    Instant::now()
        .checked_add(APP_DATA_RESET_POST_EFFECT_TIMEOUT)
        .ok_or(AppDataResetSnapshotStoreRetirementError::OutcomeUnknown)
}

fn unsafe_inventory_object() -> SnapshotStorageError {
    SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject)
}

fn snapshot_inventory_deadline_error(caller_deadline: Instant) -> SnapshotStorageError {
    let kind = if Instant::now() >= caller_deadline {
        SnapshotStorageErrorKind::Busy
    } else {
        SnapshotStorageErrorKind::Unavailable
    };
    SnapshotStorageError::new(kind)
}

fn prefer_snapshot_inventory_deadline(
    error: SnapshotStorageError,
    caller_deadline: Instant,
) -> SnapshotStorageError {
    if Instant::now() >= caller_deadline {
        SnapshotStorageError::new(SnapshotStorageErrorKind::Busy)
    } else {
        error
    }
}

#[path = "storage/secure_store.rs"]
mod secure_store;

fn snapshot_file_usage(file: &File) -> Result<SnapshotFileUsage> {
    let (logical_bytes, allocated_bytes) = platform::file_usage(file)?;
    Ok(SnapshotFileUsage::from_sizes(
        logical_bytes,
        allocated_bytes,
    ))
}

fn probe_temp_kernel_state(file: &File) -> Result<SnapshotTempKernelState> {
    match FileExt::try_lock(file) {
        Ok(()) => {
            if FileExt::unlock(file).is_err() {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
            Ok(SnapshotTempKernelState::Quiescent)
        }
        Err(TryLockError::WouldBlock) => Ok(SnapshotTempKernelState::Active),
        Err(TryLockError::Error(_)) => Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::Unavailable,
        )),
    }
}

struct SnapshotWriterLock {
    store: Arc<StoreInner>,
    file: File,
}

impl Drop for SnapshotWriterLock {
    fn drop(&mut self) {
        // An uncertain unlock keeps this process instance permanently busy;
        // descriptor closure still releases this particular OS lease.
        record_writer_unlock(
            &self.store.writer_in_use,
            FileExt::unlock(&self.file).is_ok(),
        );
    }
}

fn record_writer_unlock(in_use: &AtomicBool, unlocked: bool) {
    if unlocked {
        in_use.store(false, Ordering::Release);
    }
}

pub(crate) struct RetainedSnapshot {
    store: Arc<StoreInner>,
    name: SnapshotFileName,
    file: File,
    identity: Identity,
}

impl RetainedSnapshot {
    #[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn name(&self) -> &SnapshotFileName {
        &self.name
    }

    pub(crate) fn try_clone_file(&self) -> Result<File> {
        self.file
            .try_clone()
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))
    }

    pub(crate) fn len(&self) -> Result<u64> {
        self.revalidate()?;
        self.file
            .metadata()
            .map(|metadata| metadata.len())
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))
    }

    pub(crate) fn revalidate(&self) -> Result<()> {
        platform::validate_retained(
            &self.store.directory,
            self.store.directory_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        platform::validate_retained(
            &self.file,
            self.identity.0,
            platform::Kind::RegularFile,
            true,
        )?;
        platform::validate_named(
            &self.store.directory,
            self.name.as_str(),
            &self.file,
            self.identity.0,
            platform::Kind::RegularFile,
        )
    }
}

pub(crate) struct SnapshotPublicationLease {
    retained: RetainedSnapshot,
    _writer_lock: SnapshotWriterLock,
}

/// Snapshot-writer exclusion retained after an exact stage unlink so SQLite
/// can consume the matching durable row before another store mutation begins.
pub(crate) struct SnapshotTempMutationLease {
    _writer_lock: SnapshotWriterLock,
}

/// A retained immutable snapshot plus the store-wide writer exclusion.
///
/// This type deliberately exposes no unlink operation. Its only purpose is to
/// let the database layer commit a review pin before retention can acquire the
/// same exclusion boundary.
pub(crate) struct SnapshotOpenLease {
    retained: RetainedSnapshot,
    _writer_lock: SnapshotWriterLock,
}

impl SnapshotOpenLease {
    #[allow(
        dead_code,
        reason = "retained review leases are wired to Explorer/FFI in a later milestone slice"
    )]
    pub(crate) fn retained(&self) -> &RetainedSnapshot {
        &self.retained
    }

    pub(crate) fn into_retained(self) -> RetainedSnapshot {
        self.retained
    }
}

impl SnapshotPublicationLease {
    pub(crate) fn retained(&self) -> &RetainedSnapshot {
        &self.retained
    }
}

impl std::ops::Deref for SnapshotPublicationLease {
    type Target = RetainedSnapshot;

    fn deref(&self) -> &Self::Target {
        &self.retained
    }
}

pub(crate) enum SnapshotPublication {
    Published(SnapshotPublicationLease),
    Existing(SnapshotPublicationLease),
}

/// One generated staging name reserved while the store-wide writer lock is
/// retained.
///
/// Higher layers persist the exact durable name binding before calling
/// `create`. Keeping creation separate is what makes every physical temp
/// either row-bound or explicit pre-v8 debt after a crash.
#[must_use]
pub(crate) struct SnapshotStageReservation {
    store: Arc<StoreInner>,
    final_name: SnapshotFileName,
    temp_name: String,
    lock_timeout: Duration,
    created: bool,
    _writer_lock: SnapshotWriterLock,
}

impl SnapshotStageReservation {
    pub(crate) fn temp_name(&self) -> &str {
        &self.temp_name
    }

    /// Create the exact private file and retain an exclusive kernel lock for
    /// the complete staging lifetime. The surrounding reservation continues
    /// to hold snapshot-writer exclusion until the caller has reconciled both
    /// the durable row and this physical result.
    pub(crate) fn create(&mut self) -> Result<StagedSnapshot> {
        if self.created {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::InternalState,
            ));
        }
        let Some((file, identity)) = platform::create_private_file_exclusive(
            &self.store.directory,
            &self.store.path,
            &self.temp_name,
        )?
        else {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            ));
        };
        match FileExt::try_lock(&file) {
            Ok(()) => {}
            Err(TryLockError::WouldBlock | TryLockError::Error(_)) => {
                return Err(SnapshotStorageError::new(
                    SnapshotStorageErrorKind::Unavailable,
                ));
            }
        }
        let staged = StagedSnapshot {
            store: Arc::clone(&self.store),
            final_name: self.final_name.clone(),
            temp_name: self.temp_name.clone(),
            file: Some(file),
            identity: Identity(identity),
            lock_timeout: self.lock_timeout,
        };
        staged.revalidate()?;
        validate_inventory_inner(&self.store, Some(&self.temp_name))?;
        self.created = true;
        Ok(staged)
    }
}

/// A create-new file owned by the current publication call.
///
/// Every normal error path aborts this retained, identity-checked temporary
/// entry while holding the database compatibility fence. Unwinding or dropping
/// without that fence is close-only and leaves one recognized retention temp;
/// Drop must never invert the database-to-snapshot lock order.
#[must_use]
pub(crate) struct StagedSnapshot {
    store: Arc<StoreInner>,
    final_name: SnapshotFileName,
    temp_name: String,
    file: Option<File>,
    identity: Identity,
    lock_timeout: Duration,
}

impl StagedSnapshot {
    pub(crate) fn sync_all(&mut self) -> Result<()> {
        self.file()?
            .sync_all()
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        self.revalidate()
    }

    pub(crate) fn publish_no_replace(mut self) -> Result<SnapshotPublication> {
        self.sync_all()?;
        let lock = SecureSnapshotStore {
            inner: Arc::clone(&self.store),
        }
        .acquire_writer_lock(self.lock_timeout)?;
        validate_inventory_inner(&self.store, Some(&self.temp_name))?;
        self.revalidate()?;
        let result = platform::publish_no_replace(
            &self.store.directory,
            &self.temp_name,
            self.file()?,
            self.identity.0,
            self.final_name.as_str(),
        )?;
        match result {
            platform::Publication::Published => {
                platform::sync_directory(&self.store.directory)?;
                let writable_file = self.file.take().ok_or_else(|| {
                    SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState)
                })?;
                platform::validate_retained(
                    &writable_file,
                    self.identity.0,
                    platform::Kind::RegularFile,
                    true,
                )?;
                platform::validate_named(
                    &self.store.directory,
                    self.final_name.as_str(),
                    &writable_file,
                    self.identity.0,
                    platform::Kind::RegularFile,
                )?;
                // Publication consumes the only DUX-owned write capability.
                // Every durable snapshot returned to higher layers is reopened
                // read-only and matched to the exact published identity.
                drop(writable_file);
                let retained =
                    open_retained_inner(&self.store, &self.final_name)?.ok_or_else(|| {
                        SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject)
                    })?;
                if retained.identity != self.identity {
                    return Err(SnapshotStorageError::new(
                        SnapshotStorageErrorKind::UnsafeObject,
                    ));
                }
                validate_inventory_inner(&self.store, None)?;
                Ok(SnapshotPublication::Published(SnapshotPublicationLease {
                    retained,
                    _writer_lock: lock,
                }))
            }
            platform::Publication::Collision => {
                let existing =
                    open_retained_inner(&self.store, &self.final_name)?.ok_or_else(|| {
                        SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject)
                    })?;
                self.remove_current_temp()?;
                validate_inventory_inner(&self.store, None)?;
                Ok(SnapshotPublication::Existing(SnapshotPublicationLease {
                    retained: existing,
                    _writer_lock: lock,
                }))
            }
        }
    }

    pub(crate) fn abort(mut self) -> Result<SnapshotTempMutationLease> {
        let lock = SecureSnapshotStore {
            inner: Arc::clone(&self.store),
        }
        .acquire_writer_lock(self.lock_timeout)?;
        self.remove_current_temp()?;
        validate_inventory_inner(&self.store, None)?;
        Ok(SnapshotTempMutationLease { _writer_lock: lock })
    }

    /// Close this private temporary file without mutating the snapshot store.
    ///
    /// Callers use this only when the database compatibility fence can no
    /// longer be acquired. The exact recognized temp then remains bounded
    /// retention debt instead of allowing an older process to write after a
    /// newer schema has won.
    pub(crate) fn abandon(mut self) {
        self.file.take();
    }

    fn file(&self) -> Result<&File> {
        self.file
            .as_ref()
            .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState))
    }

    fn revalidate(&self) -> Result<()> {
        let file = self.file()?;
        platform::validate_retained(
            &self.store.directory,
            self.store.directory_identity.0,
            platform::Kind::Directory,
            false,
        )?;
        platform::validate_retained(file, self.identity.0, platform::Kind::RegularFile, true)?;
        platform::validate_named(
            &self.store.directory,
            &self.temp_name,
            file,
            self.identity.0,
            platform::Kind::RegularFile,
        )
    }

    fn remove_current_temp(&mut self) -> Result<()> {
        self.revalidate()?;
        let file = self
            .file
            .take()
            .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState))?;
        platform::remove_retained_temp(
            &self.store.directory,
            &self.temp_name,
            file,
            self.identity.0,
        )?;
        platform::sync_directory(&self.store.directory)
    }
}

impl Write for StagedSnapshot {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
            .write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
            .flush()
    }
}

fn validate_inventory_inner(store: &Arc<StoreInner>, allowed_temp: Option<&str>) -> Result<()> {
    SecureSnapshotStore {
        inner: Arc::clone(store),
    }
    .validate_inventory(allowed_temp)
}

fn open_retained_inner(
    store: &Arc<StoreInner>,
    name: &SnapshotFileName,
) -> Result<Option<RetainedSnapshot>> {
    let Some((file, identity)) =
        platform::open_named_regular(&store.directory, &store.path, name.as_str(), false)?
    else {
        return Ok(None);
    };
    let retained = RetainedSnapshot {
        store: Arc::clone(store),
        name: name.clone(),
        file,
        identity: Identity(identity),
    };
    retained.revalidate()?;
    Ok(Some(retained))
}

fn create_control(
    directory: &File,
    directory_path: &Path,
    name: &str,
    marker: &[u8; 16],
) -> Result<(File, Identity)> {
    let Some((file, identity)) =
        platform::create_private_file_exclusive(directory, directory_path, name)?
    else {
        return Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::UnsafeObject,
        ));
    };
    write_all_at(&file, marker, 0)
        .and_then(|()| file.sync_all())
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    platform::validate_retained(&file, identity, platform::Kind::RegularFile, true)?;
    platform::validate_named(
        directory,
        name,
        &file,
        identity,
        platform::Kind::RegularFile,
    )?;
    prove_marker(&file, marker)?;
    Ok((file, Identity(identity)))
}

fn open_control(
    directory: &File,
    directory_path: &Path,
    name: &str,
    marker: &[u8; 16],
    writable: bool,
) -> Result<Option<(File, Identity)>> {
    let Some((file, identity)) =
        platform::open_named_regular(directory, directory_path, name, writable)?
    else {
        return Ok(None);
    };
    platform::validate_retained(&file, identity, platform::Kind::RegularFile, true)?;
    platform::validate_named(
        directory,
        name,
        &file,
        identity,
        platform::Kind::RegularFile,
    )?;
    prove_marker(&file, marker)?;
    Ok(Some((file, Identity(identity))))
}

fn prove_marker(file: &File, expected: &[u8; 16]) -> Result<()> {
    if file
        .metadata()
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?
        .len()
        != expected.len() as u64
    {
        return Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::UnsafeObject,
        ));
    }
    let mut actual = [0_u8; 16];
    read_exact_at(file, &mut actual, 0)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject))?;
    if &actual != expected {
        return Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::UnsafeObject,
        ));
    }
    Ok(())
}

fn random_temp_name(final_name: &SnapshotFileName) -> Result<String> {
    let digest_start = FINAL_PREFIX.len();
    let digest_end = digest_start + FINAL_HEX_LENGTH;
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    let mut name = String::with_capacity(128);
    name.push_str(".snapshot-");
    name.push_str(&final_name.as_str()[digest_start..digest_end]);
    name.push('.');
    name.push_str(&std::process::id().to_string());
    name.push('.');
    push_lower_hex(&mut name, &random);
    name.push_str(".tmp");
    Ok(name)
}

fn random_directory_stage_name() -> Result<String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    let mut name = String::with_capacity(64);
    name.push_str(".dux-snapshot-stage-");
    push_lower_hex(&mut name, &random);
    Ok(name)
}

fn is_provisioning_stage_name_bytes(name: &[u8]) -> bool {
    name.len() == PROVISIONING_STAGE_PREFIX.len() + PROVISIONING_STAGE_HEX_LENGTH
        && name.starts_with(PROVISIONING_STAGE_PREFIX.as_bytes())
        && name[PROVISIONING_STAGE_PREFIX.len()..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn is_recognized_temp_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(".snapshot-") else {
        return false;
    };
    let Some(rest) = rest.strip_suffix(".tmp") else {
        return false;
    };
    let mut fields = rest.split('.');
    let (Some(digest), Some(pid), Some(random), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    digest.len() == FINAL_HEX_LENGTH
        && random.len() == 32
        && !pid.is_empty()
        && pid.len() <= 10
        && !(pid.len() > 1 && pid.starts_with('0'))
        && digest
            .bytes()
            .chain(random.bytes())
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && pid.parse::<u32>().is_ok_and(|pid| pid > 0)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buffer, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buffer, offset).and_then(|count| {
        if count == buffer.len() {
            Ok(())
        } else {
            Err(io::Error::from(io::ErrorKind::UnexpectedEof))
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_all_at(file: &File, buffer: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buffer, offset)
}

#[cfg(windows)]
fn write_all_at(file: &File, buffer: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut remaining = buffer;
    while !remaining.is_empty() {
        let written = file.seek_write(remaining, offset)?;
        if written == 0 {
            return Err(io::Error::from(io::ErrorKind::WriteZero));
        }
        offset += written as u64;
        remaining = &remaining[written..];
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
compile_error!("secure snapshot storage is unsupported on this platform");

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "storage/unix.rs"]
mod platform;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
#[path = "storage/tests.rs"]
mod tests;

#[cfg(windows)]
#[path = "storage/windows.rs"]
mod platform;
