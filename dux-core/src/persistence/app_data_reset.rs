//! Crash-safe coordination for the private whole-app-data reset pipeline.
//!
//! The independently marker-owned coordinator persists the exact checksummed
//! state machine and composes consume-once namespace capabilities. It owns no
//! caller-selected path: after exact durable `Draining` read-back it can mint
//! only the distinct opaque capabilities for one managed-cache payload or one
//! monotonic cache-control/stage-tail effect.
//! Core engine recovery remains private; FFI and native reset effect
//! integration remain separate work.

mod storage;

#[cfg(test)]
pub(crate) use storage::{TestJournalWriteFault, set_test_journal_write_fault};

use std::ffi::OsStr;
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app_data_reset_transaction::AppDataResetTransaction;
use crate::cache::{
    AppDataResetManagedCacheAbsentWitness, AppDataResetManagedCacheDrainBatch,
    AppDataResetManagedCacheDrainCandidate, AppDataResetManagedCacheDrainError,
    AppDataResetManagedCacheStageRetirementBatch, AppDataResetManagedCacheStageRetirementCandidate,
    AppDataResetManagedCacheStageRetirementError, ManagedCacheStoreErrorKind,
};

use self::storage::{ResetCoordinatorEngineLease, ResetCoordinatorStorage};
use super::history::HistoryError;
use super::snapshot::{
    AppDataResetSnapshotPayloadDrainBatch, AppDataResetSnapshotPayloadDrainError,
    AppDataResetSnapshotStoreRetirementBatch, AppDataResetSnapshotStoreRetirementError,
    SnapshotStorageErrorKind,
};
use super::store::{
    AppDataResetCanonicalRootBinding, AppDataResetDataNamespaceAdmission,
    AppDataResetFreshNamespace, AppDataResetOldSnapshotPayloadDrainCandidate,
    AppDataResetOldSnapshotStoreRetirementCandidate, AppDataResetPublishedFreshNamespace,
    AppDataResetReadyToDrainNamespace, AppDataResetRecoveryDataNamespace,
    AppDataResetStoreAdmission, StoreCoordinator,
};
use super::{AppDataResetStoreBlockers, AppDataResetStoreGuard};

const JOURNAL_FORMAT_VERSION: u16 = 2;
const LEGACY_JOURNAL_FORMAT_VERSION: u16 = 1;
const DIGEST_DOMAIN: &[u8] = b"dux-app-data-reset-journal-v1\0";
const MAX_CANONICAL_ROOT_NAME_BYTES: usize = 255;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestAppDataResetSnapshotCoordinatorPostcheckFault {
    ExhaustBeforeCacheReadback,
    ExhaustBeforeJournalReadback,
}

#[cfg(test)]
std::thread_local! {
    static TEST_APP_DATA_RESET_SNAPSHOT_COORDINATOR_POSTCHECK_FAULT:
        std::cell::Cell<Option<TestAppDataResetSnapshotCoordinatorPostcheckFault>> =
            const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_test_app_data_reset_snapshot_coordinator_postcheck_fault(
    fault: TestAppDataResetSnapshotCoordinatorPostcheckFault,
) {
    TEST_APP_DATA_RESET_SNAPSHOT_COORDINATOR_POSTCHECK_FAULT
        .with(|current| current.set(Some(fault)));
}

#[cfg(test)]
fn take_test_app_data_reset_snapshot_coordinator_postcheck_fault(
    expected: TestAppDataResetSnapshotCoordinatorPostcheckFault,
) -> bool {
    TEST_APP_DATA_RESET_SNAPSHOT_COORDINATOR_POSTCHECK_FAULT.with(|current| {
        if current.get() == Some(expected) {
            current.set(None);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
fn exhaust_snapshot_postcheck_deadline(deadline: Instant) {
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(remaining.min(Duration::from_millis(1)));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetCoordinatorErrorKind {
    InvalidConfiguration,
    UnsafeParent,
    UnsafeCoordinator,
    UnsafeObject,
    Busy,
    CorruptJournal,
    IncompatibleJournal,
    InvalidTransition,
    ChangedSinceRead,
    Unavailable,
    OutcomeUnknown,
    InternalState,
}

#[derive(Debug)]
pub(crate) struct AppDataResetCoordinatorError {
    kind: AppDataResetCoordinatorErrorKind,
}

impl AppDataResetCoordinatorError {
    pub(super) const fn new(kind: AppDataResetCoordinatorErrorKind) -> Self {
        Self { kind }
    }

    pub(crate) const fn kind(&self) -> AppDataResetCoordinatorErrorKind {
        self.kind
    }
}

impl fmt::Display for AppDataResetCoordinatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            AppDataResetCoordinatorErrorKind::InvalidConfiguration => {
                "invalid app-data reset coordinator configuration"
            }
            AppDataResetCoordinatorErrorKind::UnsafeParent => {
                "unsafe app-data reset coordinator parent"
            }
            AppDataResetCoordinatorErrorKind::UnsafeCoordinator => {
                "unsafe app-data reset coordinator"
            }
            AppDataResetCoordinatorErrorKind::UnsafeObject => {
                "unsafe app-data reset coordinator object"
            }
            AppDataResetCoordinatorErrorKind::Busy => "app-data reset coordinator is busy",
            AppDataResetCoordinatorErrorKind::CorruptJournal => "app-data reset journal is corrupt",
            AppDataResetCoordinatorErrorKind::IncompatibleJournal => {
                "app-data reset journal is from a newer version"
            }
            AppDataResetCoordinatorErrorKind::InvalidTransition => {
                "invalid app-data reset journal transition"
            }
            AppDataResetCoordinatorErrorKind::ChangedSinceRead => {
                "app-data reset journal changed since it was read"
            }
            AppDataResetCoordinatorErrorKind::Unavailable => {
                "app-data reset coordinator is unavailable"
            }
            AppDataResetCoordinatorErrorKind::OutcomeUnknown => {
                "app-data reset journal update outcome is unknown"
            }
            AppDataResetCoordinatorErrorKind::InternalState => {
                "invalid app-data reset coordinator state"
            }
        })
    }
}

impl std::error::Error for AppDataResetCoordinatorError {}

type Result<T> = std::result::Result<T, AppDataResetCoordinatorError>;

/// Filesystem identity captured before a future reset detaches a namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AppDataResetStoreIdentity {
    device: u64,
    inode: u64,
}

impl AppDataResetStoreIdentity {
    pub(crate) const fn new(device: u64, inode: u64) -> Option<Self> {
        if device == 0 || inode == 0 {
            None
        } else {
            Some(Self { device, inode })
        }
    }

    pub(crate) const fn device(self) -> u64 {
        self.device
    }

    pub(crate) const fn inode(self) -> u64 {
        self.inode
    }
}

/// Durable, strictly forward-only reset phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AppDataResetPhase {
    Prepared,
    CacheDetached,
    DataDetached,
    FreshNamespaceReady,
    Draining,
    Complete,
}

impl AppDataResetPhase {
    const fn successor(self) -> Option<Self> {
        match self {
            Self::Prepared => Some(Self::CacheDetached),
            Self::CacheDetached => Some(Self::DataDetached),
            Self::DataDetached => Some(Self::FreshNamespaceReady),
            Self::FreshNamespaceReady => Some(Self::Draining),
            Self::Draining => Some(Self::Complete),
            Self::Complete => None,
        }
    }
}

/// Exact singleton state retained in the reset journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppDataResetJournal {
    transaction_id: String,
    phase: AppDataResetPhase,
    data_identity: AppDataResetStoreIdentity,
    cache_identity: Option<AppDataResetStoreIdentity>,
    data_stage_name: String,
    cache_stage_name: Option<String>,
    canonical_root_name_hex: Option<String>,
    fresh_stage_name: String,
    fresh_data_identity: Option<AppDataResetStoreIdentity>,
    legacy_without_canonical_root_name: bool,
    legacy_complete_without_fresh_identity: bool,
}

impl AppDataResetJournal {
    pub(crate) fn prepared(
        transaction: &AppDataResetTransaction,
        data_identity: AppDataResetStoreIdentity,
        cache_identity: Option<AppDataResetStoreIdentity>,
        canonical_root: AppDataResetCanonicalRootBinding<'_>,
    ) -> Result<Self> {
        if canonical_root.identity() != data_identity {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        Self::prepared_with_root_name(
            transaction,
            data_identity,
            cache_identity,
            canonical_root.root_name(),
        )
    }

    #[cfg(test)]
    pub(crate) fn prepared_for_test(
        transaction: &AppDataResetTransaction,
        data_identity: AppDataResetStoreIdentity,
        cache_identity: Option<AppDataResetStoreIdentity>,
    ) -> Result<Self> {
        Self::prepared_for_test_root(
            transaction,
            data_identity,
            cache_identity,
            OsStr::new("Dux"),
        )
    }

    #[cfg(test)]
    pub(crate) fn prepared_for_test_root(
        transaction: &AppDataResetTransaction,
        data_identity: AppDataResetStoreIdentity,
        cache_identity: Option<AppDataResetStoreIdentity>,
        canonical_root_name: &OsStr,
    ) -> Result<Self> {
        Self::prepared_with_root_name(
            transaction,
            data_identity,
            cache_identity,
            canonical_root_name,
        )
    }

    fn prepared_with_root_name(
        transaction: &AppDataResetTransaction,
        data_identity: AppDataResetStoreIdentity,
        cache_identity: Option<AppDataResetStoreIdentity>,
        canonical_root_name: &OsStr,
    ) -> Result<Self> {
        let transaction_id = transaction.transaction_id();
        let journal = Self {
            transaction_id: transaction_id.to_owned(),
            phase: AppDataResetPhase::Prepared,
            data_identity,
            cache_identity,
            data_stage_name: transaction.data_stage().as_str().to_owned(),
            cache_stage_name: cache_identity.map(|_| transaction.cache_stage().as_str().to_owned()),
            canonical_root_name_hex: Some(encode_canonical_root_name(canonical_root_name)?),
            fresh_stage_name: transaction.fresh_stage().as_str().to_owned(),
            fresh_data_identity: None,
            legacy_without_canonical_root_name: false,
            legacy_complete_without_fresh_identity: false,
        };
        journal.validate()?;
        Ok(journal)
    }

    pub(crate) fn transaction_id(&self) -> &str {
        &self.transaction_id
    }

    pub(crate) const fn phase(&self) -> AppDataResetPhase {
        self.phase
    }

    pub(crate) const fn data_identity(&self) -> AppDataResetStoreIdentity {
        self.data_identity
    }

    pub(crate) const fn cache_identity(&self) -> Option<AppDataResetStoreIdentity> {
        self.cache_identity
    }

    pub(crate) fn data_stage_name(&self) -> &str {
        &self.data_stage_name
    }

    pub(crate) fn cache_stage_name(&self) -> Option<&str> {
        self.cache_stage_name.as_deref()
    }

    pub(crate) fn fresh_stage_name(&self) -> &str {
        &self.fresh_stage_name
    }

    pub(crate) fn is_bound_to_canonical_root_name(&self, root_name: &OsStr) -> bool {
        self.canonical_root_name_hex.as_deref()
            == encode_canonical_root_name(root_name).ok().as_deref()
    }

    pub(crate) const fn has_canonical_root_name_binding(&self) -> bool {
        self.canonical_root_name_hex.is_some()
    }

    pub(crate) const fn fresh_data_identity(&self) -> Option<AppDataResetStoreIdentity> {
        self.fresh_data_identity
    }

    fn advanced(&self, phase: AppDataResetPhase) -> Result<Self> {
        if matches!(
            phase,
            AppDataResetPhase::FreshNamespaceReady
                | AppDataResetPhase::Draining
                | AppDataResetPhase::Complete
        ) {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        if self.phase.successor() != Some(phase) {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        let mut advanced = self.clone();
        advanced.phase = phase;
        Ok(advanced)
    }

    fn advanced_draining(&self) -> Result<Self> {
        if self.phase != AppDataResetPhase::FreshNamespaceReady
            || self.legacy_without_canonical_root_name
            || self.legacy_complete_without_fresh_identity
            || self.canonical_root_name_hex.is_none()
            || self.fresh_data_identity.is_none()
        {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        let mut advanced = self.clone();
        advanced.phase = AppDataResetPhase::Draining;
        advanced.validate()?;
        Ok(advanced)
    }

    #[cfg(test)]
    fn advanced_late_phase_for_test(&self, phase: AppDataResetPhase) -> Result<Self> {
        if !matches!(
            phase,
            AppDataResetPhase::Draining | AppDataResetPhase::Complete
        ) || self.phase.successor() != Some(phase)
        {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        let mut advanced = self.clone();
        advanced.phase = phase;
        advanced.validate()?;
        Ok(advanced)
    }

    fn advanced_fresh_namespace(&self, fresh_identity: AppDataResetStoreIdentity) -> Result<Self> {
        if self.phase != AppDataResetPhase::DataDetached
            || self.fresh_data_identity.is_some()
            || fresh_identity == self.data_identity
        {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        let mut advanced = self.clone();
        advanced.phase = AppDataResetPhase::FreshNamespaceReady;
        advanced.fresh_data_identity = Some(fresh_identity);
        advanced.validate()?;
        Ok(advanced)
    }

    fn bound_canonical_root(&self, binding: AppDataResetCanonicalRootBinding<'_>) -> Result<Self> {
        if !self.legacy_without_canonical_root_name
            || self.canonical_root_name_hex.is_some()
            || !matches!(
                self.phase,
                AppDataResetPhase::Prepared | AppDataResetPhase::CacheDetached
            )
            || binding.identity() != self.data_identity
        {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        let mut bound = self.clone();
        bound.canonical_root_name_hex = Some(encode_canonical_root_name(binding.root_name())?);
        bound.legacy_without_canonical_root_name = false;
        bound.validate()?;
        Ok(bound)
    }

    /// Reconstruct role-separated namespace names only after proving the
    /// complete journal is canonical. Recovery callers receive no path or
    /// caller-selected stage-name authority.
    pub(crate) fn validated_transaction(&self) -> Result<AppDataResetTransaction> {
        let transaction =
            AppDataResetTransaction::from_canonical_transaction_id(&self.transaction_id)
                .ok_or_else(corrupt)?;
        if self.data_identity.device == 0 || self.data_identity.inode == 0 {
            return Err(corrupt());
        }
        if self
            .cache_identity
            .is_some_and(|identity| identity.device == 0 || identity.inode == 0)
            || self.data_stage_name != transaction.data_stage().as_str()
            || self.fresh_stage_name != transaction.fresh_stage().as_str()
            || self.cache_stage_name.as_deref()
                != self
                    .cache_identity
                    .map(|_| transaction.cache_stage().as_str())
        {
            return Err(corrupt());
        }
        let needs_fresh_identity = matches!(
            self.phase,
            AppDataResetPhase::FreshNamespaceReady
                | AppDataResetPhase::Draining
                | AppDataResetPhase::Complete
        );
        let valid_fresh_identity = if self.legacy_complete_without_fresh_identity {
            self.phase == AppDataResetPhase::Complete && self.fresh_data_identity.is_none()
        } else {
            match (needs_fresh_identity, self.fresh_data_identity) {
                (false, None) => true,
                (true, Some(identity)) => {
                    identity.device != 0 && identity.inode != 0 && identity != self.data_identity
                }
                _ => false,
            }
        };
        if !valid_fresh_identity {
            return Err(corrupt());
        }
        let valid_canonical_root = match self.canonical_root_name_hex.as_deref() {
            Some(encoded) => decode_canonical_root_name(encoded).is_some_and(|root_name| {
                !self.legacy_without_canonical_root_name
                    && root_name.as_slice() != storage::COORDINATOR_DIRECTORY_NAME.as_bytes()
                    && root_name.as_slice() != transaction.data_stage().as_str().as_bytes()
                    && root_name.as_slice() != transaction.cache_stage().as_str().as_bytes()
                    && root_name.as_slice() != transaction.fresh_stage().as_str().as_bytes()
            }),
            None => {
                self.legacy_without_canonical_root_name
                    && !matches!(
                        self.phase,
                        AppDataResetPhase::FreshNamespaceReady | AppDataResetPhase::Draining
                    )
            }
        };
        if !valid_canonical_root {
            return Err(corrupt());
        }
        Ok(transaction)
    }

    fn validate(&self) -> Result<()> {
        self.validated_transaction().map(drop)
    }
}

fn encode_canonical_root_name(root_name: &OsStr) -> Result<String> {
    let bytes = root_name.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_CANONICAL_ROOT_NAME_BYTES
        || bytes == b"."
        || bytes == b".."
        || bytes.contains(&b'/')
        || bytes.contains(&0)
    {
        return Err(error(
            AppDataResetCoordinatorErrorKind::InvalidConfiguration,
        ));
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(encoded)
}

fn decode_canonical_root_name(encoded: &str) -> Option<Vec<u8>> {
    let bytes = encoded.as_bytes();
    if bytes.len() < 2
        || !bytes.len().is_multiple_of(2)
        || bytes.len() > MAX_CANONICAL_ROOT_NAME_BYTES * 2
        || bytes
            .iter()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(byte))
    {
        return None;
    }
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let mut decoded = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        decoded.push(nibble(pair[0])? << 4 | nibble(pair[1])?);
    }
    if decoded == b"." || decoded == b".." || decoded.contains(&b'/') || decoded.contains(&0) {
        None
    } else {
        Some(decoded)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalPayloadV1 {
    transaction_id: String,
    phase: AppDataResetPhase,
    data_identity: AppDataResetStoreIdentity,
    cache_identity: Option<AppDataResetStoreIdentity>,
    data_stage_name: String,
    cache_stage_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalEnvelopeV1 {
    format_version: u16,
    payload: JournalPayloadV1,
    digest_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalPayloadV2 {
    transaction_id: String,
    phase: AppDataResetPhase,
    data_identity: AppDataResetStoreIdentity,
    cache_identity: Option<AppDataResetStoreIdentity>,
    data_stage_name: String,
    cache_stage_name: Option<String>,
    canonical_root_name_hex: String,
    fresh_stage_name: String,
    fresh_data_identity: Option<AppDataResetStoreIdentity>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalEnvelopeV2 {
    format_version: u16,
    payload: JournalPayloadV2,
    digest_sha256: String,
}

#[derive(Deserialize)]
struct JournalVersionProbe {
    format_version: u16,
}

impl From<&AppDataResetJournal> for JournalPayloadV2 {
    fn from(journal: &AppDataResetJournal) -> Self {
        Self {
            transaction_id: journal.transaction_id.clone(),
            phase: journal.phase,
            data_identity: journal.data_identity,
            cache_identity: journal.cache_identity,
            data_stage_name: journal.data_stage_name.clone(),
            cache_stage_name: journal.cache_stage_name.clone(),
            canonical_root_name_hex: journal
                .canonical_root_name_hex
                .clone()
                .expect("validated V2 journal must bind the canonical root"),
            fresh_stage_name: journal.fresh_stage_name.clone(),
            fresh_data_identity: journal.fresh_data_identity,
        }
    }
}

impl From<JournalPayloadV2> for AppDataResetJournal {
    fn from(payload: JournalPayloadV2) -> Self {
        Self {
            transaction_id: payload.transaction_id,
            phase: payload.phase,
            data_identity: payload.data_identity,
            cache_identity: payload.cache_identity,
            data_stage_name: payload.data_stage_name,
            cache_stage_name: payload.cache_stage_name,
            canonical_root_name_hex: Some(payload.canonical_root_name_hex),
            fresh_stage_name: payload.fresh_stage_name,
            fresh_data_identity: payload.fresh_data_identity,
            legacy_without_canonical_root_name: false,
            legacy_complete_without_fresh_identity: false,
        }
    }
}

fn journal_from_v1(payload: JournalPayloadV1) -> Result<AppDataResetJournal> {
    if matches!(
        payload.phase,
        AppDataResetPhase::DataDetached
            | AppDataResetPhase::FreshNamespaceReady
            | AppDataResetPhase::Draining
    ) {
        return Err(error(AppDataResetCoordinatorErrorKind::IncompatibleJournal));
    }
    let transaction =
        AppDataResetTransaction::from_canonical_transaction_id(&payload.transaction_id)
            .ok_or_else(corrupt)?;
    Ok(AppDataResetJournal {
        transaction_id: payload.transaction_id,
        phase: payload.phase,
        data_identity: payload.data_identity,
        cache_identity: payload.cache_identity,
        data_stage_name: payload.data_stage_name,
        cache_stage_name: payload.cache_stage_name,
        canonical_root_name_hex: None,
        fresh_stage_name: transaction.fresh_stage().as_str().to_owned(),
        fresh_data_identity: None,
        legacy_without_canonical_root_name: true,
        legacy_complete_without_fresh_identity: payload.phase == AppDataResetPhase::Complete,
    })
}

/// Independently marker-owned durable singleton coordinator.
pub(crate) struct AppDataResetCoordinator {
    storage: ResetCoordinatorStorage,
}

/// Shared cross-process gate retained for the lifetime of one ordinary engine.
/// A reset cannot acquire the coordinator's exclusive session until every
/// ordinary engine using this data root has quiesced and released its lease.
pub(crate) struct AppDataResetEngineLease {
    _storage: ResetCoordinatorEngineLease,
}

pub(crate) enum AppDataResetEngineLeaseOutcome {
    Admitted(AppDataResetEngineLease),
    RecoveryRequired(Box<AppDataResetRecoveryIntent>),
}

/// Move-only proof that one exact incomplete journal was observed while the
/// original coordinator descriptors were protected by a shared lease.
///
/// Recovery consumes this value, releases the shared lease, and acquires the
/// exclusive lock through the same retained storage object. A missing,
/// replaced, or changed journal therefore cannot be downgraded to a clean
/// ordinary open during the handoff.
pub(crate) struct AppDataResetRecoveryIntent {
    storage: ResetCoordinatorStorage,
    engine_lease: ResetCoordinatorEngineLease,
    journal: AppDataResetJournal,
}

/// One callback-scoped owner of the retained reset-coordinator writer lock.
///
/// The session exposes only typed journal operations. It cannot escape the
/// callback that owns the lock, disclose storage paths, or acquire a second
/// coordinator lock.
pub(crate) struct AppDataResetCoordinatorSession<'a> {
    storage: &'a ResetCoordinatorStorage,
}

/// Consume-once composition of all authority needed for one bounded cache
/// payload batch. Construction is possible only after `Draining` is known
/// durable for the exact current journal and namespace witnesses.
pub(crate) struct AppDataResetDrainingCacheBatch<'data, 'cache> {
    journal: AppDataResetJournal,
    data: AppDataResetReadyToDrainNamespace<'data>,
    cache: AppDataResetManagedCacheDrainCandidate<'cache>,
}

/// Consume-once composition for one detached-cache structural retirement
/// effect. Unlike payload draining, construction requires an already-durable
/// `Draining` journal observed on a later recovery pass.
pub(crate) struct AppDataResetDrainingCacheStageRetirementBatch<'data, 'cache> {
    journal: AppDataResetJournal,
    data: AppDataResetReadyToDrainNamespace<'data>,
    cache: AppDataResetManagedCacheStageRetirementCandidate<'cache>,
}

/// Consume-once composition for one old snapshot payload after cache debt is
/// proven fully absent. The journal remains `Draining`; this token can remove
/// neither snapshot controls nor any data-root object.
pub(crate) struct AppDataResetDrainingSnapshotPayloadBatch<'data, 'cache> {
    journal: AppDataResetJournal,
    data: AppDataResetOldSnapshotPayloadDrainCandidate<'data>,
    cache: AppDataResetManagedCacheAbsentWitness<'cache>,
    pre_effect_deadline: Instant,
}

/// Consume-once composition for one old snapshot-store control or directory
/// after snapshot payloads and managed-cache debt are both proven absent. The
/// journal remains `Draining`; this token cannot touch any database/root tail.
pub(crate) struct AppDataResetDrainingSnapshotStoreRetirementBatch<'data, 'cache> {
    journal: AppDataResetJournal,
    data: AppDataResetOldSnapshotStoreRetirementCandidate<'data>,
    cache: AppDataResetManagedCacheAbsentWitness<'cache>,
    pre_effect_deadline: Instant,
}

/// Opaque proof that the coordinator has re-read the exact durable Draining
/// journal immediately before a cache effect. Its private field prevents any
/// other production layer from calling the cache unlink primitive directly.
pub(crate) struct AppDataResetCacheDrainAuthority {
    _private: (),
}

impl AppDataResetCacheDrainAuthority {
    #[cfg(test)]
    pub(crate) const fn for_test() -> Self {
        Self { _private: () }
    }
}

/// Opaque proof that the coordinator has re-read the exact durable Draining
/// journal immediately before retiring one detached-cache structural object.
/// This capability is deliberately distinct from payload-drain authority.
pub(crate) struct AppDataResetCacheStageRetireAuthority {
    _private: (),
}

impl AppDataResetCacheStageRetireAuthority {
    #[cfg(test)]
    pub(crate) const fn for_test() -> Self {
        Self { _private: () }
    }
}

/// Opaque proof that the coordinator has re-read the exact durable Draining
/// journal, fresh/old namespace join, and managed-cache absence immediately
/// before one old snapshot payload effect. Snapshot storage cannot construct
/// this value and therefore cannot turn an inventory observation into reset
/// cleanup authority by itself.
pub(crate) struct AppDataResetSnapshotPayloadDrainAuthority {
    pre_effect_deadline: Instant,
}

impl AppDataResetSnapshotPayloadDrainAuthority {
    pub(crate) const fn pre_effect_deadline(&self) -> Instant {
        self.pre_effect_deadline
    }

    #[cfg(test)]
    pub(crate) const fn for_test(pre_effect_deadline: Instant) -> Self {
        Self {
            pre_effect_deadline,
        }
    }
}

/// Opaque proof that the coordinator has re-read the exact durable Draining
/// journal, fresh/old namespace join, monotonic snapshot structural state, and
/// managed-cache absence immediately before one snapshot control or directory
/// effect. It is deliberately distinct from snapshot payload authority.
pub(crate) struct AppDataResetSnapshotStoreRetireAuthority {
    pre_effect_deadline: Instant,
}

impl AppDataResetSnapshotStoreRetireAuthority {
    pub(crate) const fn pre_effect_deadline(&self) -> Instant {
        self.pre_effect_deadline
    }

    #[cfg(test)]
    pub(crate) const fn for_test(pre_effect_deadline: Instant) -> Self {
        Self {
            pre_effect_deadline,
        }
    }
}

/// Result of retaining store exclusion inside a coordinator session.
///
/// The callback is never invoked for a blocked store and its move-only guard
/// cannot escape the higher-ranked callback.
pub(crate) enum AppDataResetAdmittedStoreOutcome<T> {
    Blocked(AppDataResetStoreBlockers),
    Admitted(T),
}

/// Bounded physical provisioning debt beside the durable coordinator.
///
/// Marker-owned stages are reconciled before this observation returns.
/// Unproven stages are never removed from a name prefix alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AppDataResetProvisioningDebt {
    unproven_stage_count: u32,
}

impl AppDataResetProvisioningDebt {
    pub(crate) const fn unproven_stage_count(self) -> u32 {
        self.unproven_stage_count
    }
}

impl AppDataResetCoordinator {
    /// Provision the fixed coordinator, take a shared cross-process engine
    /// lease, and inspect the exact journal while reset writers remain
    /// excluded. This never creates or opens the canonical data root.
    /// Acquire the ordinary shared gate within one caller-owned absolute
    /// deadline. An incomplete observation is returned with the original
    /// coordinator descriptors so recovery never reopens or provisions it.
    pub(crate) fn acquire_engine_lease_until(
        data_root: &Path,
        deadline: Instant,
    ) -> Result<AppDataResetEngineLeaseOutcome> {
        if Instant::now() >= deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }
        let storage = match ResetCoordinatorStorage::open_existing_for_engine(data_root)? {
            Some(storage) => storage,
            None => match ResetCoordinatorStorage::open_or_create_until(data_root, deadline) {
                Ok(storage) => storage,
                Err(first_error) => {
                    match ResetCoordinatorStorage::open_existing_for_engine(data_root) {
                        Ok(Some(storage)) => storage,
                        Ok(None) | Err(_) => return Err(first_error),
                    }
                }
            },
        };
        let (lease, journal) = storage.acquire_engine_lease()?;
        let journal = journal.map(|bytes| decode_journal(&bytes)).transpose()?;
        if journal.as_ref().is_some_and(|journal| {
            journal.has_canonical_root_name_binding()
                && !journal.is_bound_to_canonical_root_name(storage.data_root_binding().1)
        }) {
            drop(lease);
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        if let Some(journal) = journal
            && journal.phase() != AppDataResetPhase::Complete
        {
            return Ok(AppDataResetEngineLeaseOutcome::RecoveryRequired(Box::new(
                AppDataResetRecoveryIntent {
                    storage,
                    engine_lease: lease,
                    journal,
                },
            )));
        }
        if Instant::now() >= deadline {
            drop(lease);
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }
        Ok(AppDataResetEngineLeaseOutcome::Admitted(
            AppDataResetEngineLease { _storage: lease },
        ))
    }

    /// Open or provision the fixed sibling coordinator for one data root.
    ///
    /// Provisioning mutates only the fixed private coordinator namespace. It
    /// never opens, renames, or removes the supplied data root.
    pub(crate) fn open_or_create(data_root: &Path) -> Result<Self> {
        Ok(Self {
            storage: ResetCoordinatorStorage::open_or_create(data_root)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn open_existing_for_test(data_root: &Path) -> Result<Self> {
        let storage = ResetCoordinatorStorage::open_existing_for_engine(data_root)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::Unavailable))?;
        Ok(Self { storage })
    }

    /// Open or provision the coordinator while charging reconciliation lock
    /// acquisition to the caller's existing deadline.
    pub(crate) fn open_or_create_until(data_root: &Path, deadline: Instant) -> Result<Self> {
        Ok(Self {
            storage: ResetCoordinatorStorage::open_or_create_until(data_root, deadline)?,
        })
    }

    /// Retain the same exclusive coordinator lock across a complete sequence
    /// of typed journal reads and transitions.
    pub(crate) fn with_exclusive_session<T>(
        &self,
        operation: impl FnOnce(&mut AppDataResetCoordinatorSession<'_>) -> Result<T>,
    ) -> Result<T> {
        self.storage
            .with_lock(|storage| operation(&mut AppDataResetCoordinatorSession { storage }))
    }

    pub(crate) fn with_exclusive_session_with_timeout<T>(
        &self,
        timeout: Duration,
        operation: impl FnOnce(&mut AppDataResetCoordinatorSession<'_>) -> Result<T>,
    ) -> Result<T> {
        self.storage.with_lock_timeout(timeout, |storage| {
            operation(&mut AppDataResetCoordinatorSession { storage })
        })
    }

    pub(crate) fn with_exclusive_session_until<T>(
        &self,
        deadline: Instant,
        operation: impl FnOnce(&mut AppDataResetCoordinatorSession<'_>) -> Result<T>,
    ) -> Result<T> {
        self.storage.with_lock_until(deadline, |storage| {
            operation(&mut AppDataResetCoordinatorSession { storage })
        })
    }

    /// Return the exact current journal, if one exists.
    pub(crate) fn recover(&self) -> Result<Option<AppDataResetJournal>> {
        self.with_exclusive_session(|session| session.recover())
    }

    pub(crate) fn provisioning_debt(&self) -> Result<AppDataResetProvisioningDebt> {
        self.with_exclusive_session(|session| session.provisioning_debt())
    }

    /// Commit the first durable reset intent or replace one completed intent.
    ///
    /// An incomplete transaction can only move through `advance`.
    pub(crate) fn begin(&self, prepared: &AppDataResetJournal) -> Result<()> {
        self.with_exclusive_session(|session| session.begin(prepared))
    }

    /// Exact compare-and-advance of one already durable transaction.
    pub(crate) fn advance(
        &self,
        expected: &AppDataResetJournal,
        next_phase: AppDataResetPhase,
    ) -> Result<AppDataResetJournal> {
        self.with_exclusive_session(|session| session.advance(expected, next_phase))
    }

    /// Construct later-phase journal fixtures without weakening the production
    /// filesystem-witness transition into `FreshNamespaceReady`.
    #[cfg(test)]
    pub(crate) fn advance_fresh_namespace_for_test(
        &self,
        expected: &AppDataResetJournal,
        identity: AppDataResetStoreIdentity,
    ) -> Result<AppDataResetJournal> {
        self.with_exclusive_session(|session| {
            session.advance_fresh_namespace_identity(expected, identity)
        })
    }

    #[cfg(test)]
    pub(crate) fn advance_late_phase_for_test(
        &self,
        expected: &AppDataResetJournal,
        phase: AppDataResetPhase,
    ) -> Result<AppDataResetJournal> {
        self.with_exclusive_session(|session| session.advance_late_phase_for_test(expected, phase))
    }
}

impl AppDataResetRecoveryIntent {
    /// Consume the shared observation and transfer it to the exclusive owner
    /// on the same retained coordinator. The exact journal is re-read and
    /// compared before the recovery callback may inspect or mutate a reset
    /// namespace.
    pub(crate) fn with_exclusive_session_until<T>(
        self,
        deadline: Instant,
        operation: impl FnOnce(
            &mut AppDataResetCoordinatorSession<'_>,
            AppDataResetJournal,
        ) -> Result<T>,
    ) -> Result<T> {
        self.with_exclusive_session_with_handoff_hook_until(deadline, || {}, operation)
    }

    fn with_exclusive_session_with_handoff_hook_until<T>(
        self,
        deadline: Instant,
        after_shared_release: impl FnOnce(),
        operation: impl FnOnce(
            &mut AppDataResetCoordinatorSession<'_>,
            AppDataResetJournal,
        ) -> Result<T>,
    ) -> Result<T> {
        let Self {
            storage,
            engine_lease,
            journal: expected,
        } = self;
        drop(engine_lease);
        after_shared_release();
        storage.with_lock_until(deadline, |storage| {
            let mut session = AppDataResetCoordinatorSession { storage };
            let current = session
                .recover()?
                .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
            if current != expected {
                return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
            }
            operation(&mut session, current)
        })
    }
}

impl AppDataResetCoordinatorSession<'_> {
    /// Reopen the exact canonical-or-detached data namespace named by a
    /// validated journal while retaining this exclusive coordinator session.
    /// The recovery opener is descriptor-only and cannot provision, repair,
    /// migrate, or open SQLite.
    pub(crate) fn with_recovery_data_namespace_until<T>(
        &mut self,
        database_path: &Path,
        journal: &AppDataResetJournal,
        deadline: Instant,
        operation: impl for<'session, 'data> FnOnce(
            &'session mut AppDataResetCoordinatorSession<'_>,
            AppDataResetRecoveryDataNamespace<'data>,
        ) -> T,
    ) -> std::result::Result<T, HistoryError> {
        let current = self
            .storage
            .read_journal()
            .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))?
            .ok_or_else(|| HistoryError::new(super::history::HistoryErrorKind::InternalState))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))
            })?;
        let (_, canonical_root_name) = self.storage.data_root_binding();
        if current != *journal
            || (journal.has_canonical_root_name_binding()
                && !journal.is_bound_to_canonical_root_name(canonical_root_name))
            || database_path.parent().and_then(Path::file_name) != Some(canonical_root_name)
        {
            return Err(HistoryError::new(
                super::history::HistoryErrorKind::InternalState,
            ));
        }
        let transaction = journal
            .validated_transaction()
            .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))?;
        StoreCoordinator::with_app_data_reset_recovery_data_namespace_until(
            database_path,
            &transaction,
            journal.data_identity(),
            deadline,
            |data_namespace| operation(self, data_namespace),
        )
    }

    /// Acquire the same ordered old-store guards for the phase-specific fresh
    /// bootstrap publisher. Every path/name/provenance input is derived from
    /// the validated journal transaction inside this persistence boundary.
    pub(crate) fn with_fresh_data_namespace_until<T>(
        &mut self,
        database_path: &Path,
        journal: &AppDataResetJournal,
        deadline: Instant,
        operation: impl for<'session, 'data> FnOnce(
            &'session mut AppDataResetCoordinatorSession<'_>,
            AppDataResetFreshNamespace<'data>,
        ) -> T,
    ) -> std::result::Result<T, HistoryError> {
        let current = self
            .storage
            .read_journal()
            .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))?
            .ok_or_else(|| HistoryError::new(super::history::HistoryErrorKind::InternalState))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))
            })?;
        let (_, canonical_root_name) = self.storage.data_root_binding();
        if current != *journal
            || !journal.is_bound_to_canonical_root_name(canonical_root_name)
            || database_path.parent().and_then(Path::file_name) != Some(canonical_root_name)
        {
            return Err(HistoryError::new(
                super::history::HistoryErrorKind::InternalState,
            ));
        }
        let transaction = journal
            .validated_transaction()
            .map_err(|_| HistoryError::new(super::history::HistoryErrorKind::InternalState))?;
        StoreCoordinator::with_app_data_reset_fresh_namespace_until(
            database_path,
            &transaction,
            journal.data_identity(),
            journal.phase() == AppDataResetPhase::Draining,
            deadline,
            |fresh_namespace| operation(self, fresh_namespace),
        )
    }

    /// Seal the exact published fresh identity and advance only the matching
    /// `DataDetached` journal. No raw identity can enter from the engine layer.
    pub(crate) fn commit_fresh_namespace(
        &mut self,
        expected: &AppDataResetJournal,
        published: &AppDataResetPublishedFreshNamespace<'_>,
    ) -> Result<AppDataResetJournal> {
        let transaction = expected.validated_transaction()?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        published
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        if !published.is_bound_to(
            &transaction,
            expected.data_identity(),
            publication_parent_identity,
            canonical_root_name,
        ) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        let identity = published
            .fresh_identity()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        self.advance_fresh_namespace_identity(expected, identity)
    }

    /// Admit one bounded cache payload batch only after the exact
    /// `FreshNamespaceReady` journal has durably advanced to `Draining`, or
    /// after an already-`Draining` restart has re-proved the same namespaces.
    /// Journal-write uncertainty returns no batch token, so unlink authority
    /// is unreachable until durability is known.
    pub(crate) fn admit_draining_cache_batch<'data, 'cache>(
        &mut self,
        expected: &AppDataResetJournal,
        data: AppDataResetReadyToDrainNamespace<'data>,
        cache: AppDataResetManagedCacheDrainCandidate<'cache>,
    ) -> Result<AppDataResetDrainingCacheBatch<'data, 'cache>> {
        expected.validate()?;
        if !matches!(
            expected.phase(),
            AppDataResetPhase::FreshNamespaceReady | AppDataResetPhase::Draining
        ) {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }

        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if current != *expected || !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }

        let transaction = expected.validated_transaction()?;
        let fresh_identity = expected
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))?;
        let cache_identity = expected
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        if !data.is_bound_to(
            &transaction,
            expected.data_identity(),
            fresh_identity,
            publication_parent_identity,
            canonical_root_name,
        ) || !cache.is_bound_to(cache_identity, transaction.cache_stage())
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let journal = if expected.phase() == AppDataResetPhase::FreshNamespaceReady {
            let next = expected.advanced_draining()?;
            self.storage.write_journal(&encode_journal(&next)?)?;
            next
        } else {
            expected.clone()
        };

        data.revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        let durable = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if durable != journal || durable.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(AppDataResetDrainingCacheBatch {
            journal,
            data,
            cache,
        })
    }

    /// Consume the coordinator-issued token for exactly one cache object.
    /// No retry is possible from this token after an unlink attempt.
    pub(crate) fn run_draining_cache_batch(
        &mut self,
        batch: AppDataResetDrainingCacheBatch<'_, '_>,
    ) -> Result<AppDataResetManagedCacheDrainBatch> {
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != batch.journal || current.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        batch
            .data
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        batch
            .cache
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let authority = AppDataResetCacheDrainAuthority { _private: () };
        let progress = batch
            .cache
            .drain_one_detached_payload(authority)
            .map_err(map_cache_drain_error)?;

        // The data namespace is unaffected by the cache-only batch and must
        // still match exactly. Any later uncertainty remains recovery debt.
        batch
            .data
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        let durable = self
            .storage
            .read_journal()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            })?;
        if durable != batch.journal {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(progress)
    }

    /// Join the exact fresh/old namespace proof to one structural cache-tail
    /// candidate. This boundary deliberately accepts only a journal that was
    /// already `Draining` when the recovery pass began; the transition from
    /// `FreshNamespaceReady` cannot retire ownership controls in the same pass.
    pub(crate) fn admit_draining_cache_stage_retirement_batch<'data, 'cache>(
        &mut self,
        expected: &AppDataResetJournal,
        data: AppDataResetReadyToDrainNamespace<'data>,
        cache: AppDataResetManagedCacheStageRetirementCandidate<'cache>,
    ) -> Result<AppDataResetDrainingCacheStageRetirementBatch<'data, 'cache>> {
        expected.validate()?;
        if expected.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }

        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if current != *expected || !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }

        let transaction = expected.validated_transaction()?;
        let fresh_identity = expected
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))?;
        let cache_identity = expected
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        if !data.is_bound_to(
            &transaction,
            expected.data_identity(),
            fresh_identity,
            publication_parent_identity,
            canonical_root_name,
        ) || !cache.is_bound_to(cache_identity, transaction.cache_stage())
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let durable = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if durable != *expected || durable.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(AppDataResetDrainingCacheStageRetirementBatch {
            journal: durable,
            data,
            cache,
        })
    }

    /// Consume the coordinator-issued structural capability for exactly one
    /// marker, writer control, or empty detached stage shell. Post-effect
    /// uncertainty remains reset recovery debt and is never retried here.
    pub(crate) fn run_draining_cache_stage_retirement_batch(
        &mut self,
        batch: AppDataResetDrainingCacheStageRetirementBatch<'_, '_>,
    ) -> Result<AppDataResetManagedCacheStageRetirementBatch> {
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != batch.journal || current.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        batch
            .data
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        batch
            .cache
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let authority = AppDataResetCacheStageRetireAuthority { _private: () };
        let progress = batch
            .cache
            .retire_one_structure(authority)
            .map_err(map_cache_stage_retirement_error)?;

        batch
            .data
            .revalidate()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        let durable = self
            .storage
            .read_journal()
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            })?;
        if durable != batch.journal {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(progress)
    }

    /// Join exact cache absence to the first retained old snapshot payload.
    /// This is available only after an already-durable `Draining` journal;
    /// cache payload/control work and snapshot payload work therefore cannot
    /// occur in the same recovery pass.
    pub(crate) fn admit_draining_snapshot_payload_batch<'data, 'cache>(
        &mut self,
        expected: &AppDataResetJournal,
        data: AppDataResetOldSnapshotPayloadDrainCandidate<'data>,
        cache: AppDataResetManagedCacheAbsentWitness<'cache>,
        pre_effect_deadline: Instant,
    ) -> Result<AppDataResetDrainingSnapshotPayloadBatch<'data, 'cache>> {
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }
        expected.validate()?;
        if expected.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }

        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if current != *expected || !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }

        let transaction = expected.validated_transaction()?;
        let fresh_identity = expected
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))?;
        let cache_identity = expected
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        if !data.is_bound_to(
            &transaction,
            expected.data_identity(),
            fresh_identity,
            publication_parent_identity,
            canonical_root_name,
        ) || !cache.is_bound_to(cache_identity, transaction.cache_stage())
            || data.deadline() != pre_effect_deadline
            || cache.deadline() != pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let durable = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if durable != *expected
            || durable.phase() != AppDataResetPhase::Draining
            || Instant::now() >= pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(AppDataResetDrainingSnapshotPayloadBatch {
            journal: durable,
            data,
            cache,
            pre_effect_deadline,
        })
    }

    /// Consume the sole production snapshot-reset capability for one exact
    /// old-store final or quiescent temporary. Every post-effect ambiguity is
    /// durable recovery debt; this batch is never retried.
    pub(crate) fn run_draining_snapshot_payload_batch(
        &mut self,
        batch: AppDataResetDrainingSnapshotPayloadBatch<'_, '_>,
    ) -> Result<AppDataResetSnapshotPayloadDrainBatch> {
        let pre_effect_deadline = batch.pre_effect_deadline;
        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != batch.journal || current.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        batch
            .data
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        batch
            .cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }

        let authority = AppDataResetSnapshotPayloadDrainAuthority {
            pre_effect_deadline,
        };
        let completion = batch
            .data
            .drain_one_snapshot_payload(authority)
            .map_err(map_snapshot_payload_drain_error)?;
        let post_effect_deadline = completion.post_effect_deadline();

        #[cfg(test)]
        if take_test_app_data_reset_snapshot_coordinator_postcheck_fault(
            TestAppDataResetSnapshotCoordinatorPostcheckFault::ExhaustBeforeCacheReadback,
        ) {
            exhaust_snapshot_postcheck_deadline(post_effect_deadline);
        }
        batch
            .cache
            .revalidate_after_effect_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        #[cfg(test)]
        if take_test_app_data_reset_snapshot_coordinator_postcheck_fault(
            TestAppDataResetSnapshotCoordinatorPostcheckFault::ExhaustBeforeJournalReadback,
        ) {
            exhaust_snapshot_postcheck_deadline(post_effect_deadline);
        }
        let durable = self
            .storage
            .read_journal_exact_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            })?;
        if durable != batch.journal || Instant::now() >= post_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(completion.into_progress())
    }

    /// Join exact cache absence to one monotonic old snapshot-store structural
    /// state. This is available only on a pass already observing durable
    /// `Draining`, after payload selection has returned no candidate.
    pub(crate) fn admit_draining_snapshot_store_retirement_batch<'data, 'cache>(
        &mut self,
        expected: &AppDataResetJournal,
        data: AppDataResetOldSnapshotStoreRetirementCandidate<'data>,
        cache: AppDataResetManagedCacheAbsentWitness<'cache>,
        pre_effect_deadline: Instant,
    ) -> Result<AppDataResetDrainingSnapshotStoreRetirementBatch<'data, 'cache>> {
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }
        expected.validate()?;
        if expected.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }

        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        let (publication_parent_identity, canonical_root_name) = self.storage.data_root_binding();
        if current != *expected || !expected.is_bound_to_canonical_root_name(canonical_root_name) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }

        let transaction = expected.validated_transaction()?;
        let fresh_identity = expected
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))?;
        let cache_identity = expected
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        if !data.is_bound_to(
            &transaction,
            expected.data_identity(),
            fresh_identity,
            publication_parent_identity,
            canonical_root_name,
        ) || !cache.is_bound_to(cache_identity, transaction.cache_stage())
            || data.deadline() != pre_effect_deadline
            || cache.deadline() != pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        data.revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;

        let durable = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if durable != *expected
            || durable.phase() != AppDataResetPhase::Draining
            || Instant::now() >= pre_effect_deadline
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(AppDataResetDrainingSnapshotStoreRetirementBatch {
            journal: durable,
            data,
            cache,
            pre_effect_deadline,
        })
    }

    /// Consume the sole production structural capability for one exact old
    /// snapshot marker, locked writer control, or empty directory. Every
    /// post-effect ambiguity remains durable recovery debt.
    pub(crate) fn run_draining_snapshot_store_retirement_batch(
        &mut self,
        batch: AppDataResetDrainingSnapshotStoreRetirementBatch<'_, '_>,
    ) -> Result<AppDataResetSnapshotStoreRetirementBatch> {
        let pre_effect_deadline = batch.pre_effect_deadline;
        let current = self
            .storage
            .read_journal_exact_until(pre_effect_deadline)?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != batch.journal || current.phase() != AppDataResetPhase::Draining {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        batch
            .data
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        batch
            .cache
            .revalidate_until(pre_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))?;
        if Instant::now() >= pre_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::Busy));
        }

        let authority = AppDataResetSnapshotStoreRetireAuthority {
            pre_effect_deadline,
        };
        let completion = batch
            .data
            .retire_one_snapshot_store_structure(authority)
            .map_err(map_snapshot_store_retirement_error)?;
        let post_effect_deadline = completion.post_effect_deadline();

        #[cfg(test)]
        if take_test_app_data_reset_snapshot_coordinator_postcheck_fault(
            TestAppDataResetSnapshotCoordinatorPostcheckFault::ExhaustBeforeCacheReadback,
        ) {
            exhaust_snapshot_postcheck_deadline(post_effect_deadline);
        }
        batch
            .cache
            .revalidate_after_effect_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?;
        #[cfg(test)]
        if take_test_app_data_reset_snapshot_coordinator_postcheck_fault(
            TestAppDataResetSnapshotCoordinatorPostcheckFault::ExhaustBeforeJournalReadback,
        ) {
            exhaust_snapshot_postcheck_deadline(post_effect_deadline);
        }
        let durable = self
            .storage
            .read_journal_exact_until(post_effect_deadline)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            .and_then(|bytes| {
                decode_journal(&bytes)
                    .map_err(|_| error(AppDataResetCoordinatorErrorKind::OutcomeUnknown))
            })?;
        if durable != batch.journal || Instant::now() >= post_effect_deadline {
            return Err(error(AppDataResetCoordinatorErrorKind::OutcomeUnknown));
        }
        Ok(completion.into_progress())
    }

    #[cfg(test)]
    pub(crate) fn replace_journal_for_fresh_binding_test(
        &mut self,
        journal: &AppDataResetJournal,
    ) -> Result<()> {
        self.storage.write_journal(&encode_journal(journal)?)
    }

    /// Acquire the data-root publication fence strictly inside this retained
    /// coordinator session and before database-side reset admission.
    ///
    /// The StoreCoordinator primitive is persistence-private; this is the only
    /// production route that lends its validation-only witness.
    pub(crate) fn with_data_namespace_admission_until<T>(
        &mut self,
        store: &StoreCoordinator,
        transaction: &AppDataResetTransaction,
        deadline: Instant,
        operation: impl for<'session, 'data> FnOnce(
            &'session mut AppDataResetCoordinatorSession<'_>,
            AppDataResetDataNamespaceAdmission<'data>,
        ) -> T,
    ) -> std::result::Result<T, HistoryError> {
        store.with_app_data_reset_data_namespace_admission_until(
            transaction,
            deadline,
            |data_namespace| operation(self, data_namespace),
        )
    }

    /// Acquire database-side reset admission strictly inside this retained
    /// coordinator session.
    ///
    /// The higher-ranked callback prevents an admitted store guard from
    /// escaping this call. Consequently the coordinator lock always outlives
    /// cleanup and database exclusion, and callers cannot invert that order.
    pub(crate) fn with_admitted_store<T>(
        &mut self,
        store: &StoreCoordinator,
        operation: impl for<'session, 'guard> FnOnce(
            &'session mut AppDataResetCoordinatorSession<'_>,
            AppDataResetStoreGuard<'guard>,
        ) -> T,
    ) -> std::result::Result<AppDataResetAdmittedStoreOutcome<T>, HistoryError> {
        match store.begin_app_data_reset_store_admission()? {
            AppDataResetStoreAdmission::Blocked(blockers) => {
                Ok(AppDataResetAdmittedStoreOutcome::Blocked(blockers))
            }
            AppDataResetStoreAdmission::Admitted(guard) => Ok(
                AppDataResetAdmittedStoreOutcome::Admitted(operation(self, guard)),
            ),
        }
    }

    /// Acquire database-side reset admission using the same absolute deadline
    /// as every earlier and later reset lock.
    pub(crate) fn with_admitted_store_until<T>(
        &mut self,
        store: &StoreCoordinator,
        deadline: Instant,
        operation: impl for<'session, 'guard> FnOnce(
            &'session mut AppDataResetCoordinatorSession<'_>,
            AppDataResetStoreGuard<'guard>,
        ) -> T,
    ) -> std::result::Result<AppDataResetAdmittedStoreOutcome<T>, HistoryError> {
        match store.begin_app_data_reset_store_admission_until(deadline)? {
            AppDataResetStoreAdmission::Blocked(blockers) => {
                Ok(AppDataResetAdmittedStoreOutcome::Blocked(blockers))
            }
            AppDataResetStoreAdmission::Admitted(guard) => Ok(
                AppDataResetAdmittedStoreOutcome::Admitted(operation(self, guard)),
            ),
        }
    }

    /// Return the exact current journal without releasing the retained lock.
    pub(crate) fn recover(&mut self) -> Result<Option<AppDataResetJournal>> {
        let Some(bytes) = self.storage.read_journal()? else {
            return Ok(None);
        };
        let journal = decode_journal(&bytes)?;
        if journal.has_canonical_root_name_binding()
            && !journal.is_bound_to_canonical_root_name(self.storage.data_root_binding().1)
        {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        Ok(Some(journal))
    }

    pub(crate) fn provisioning_debt(&mut self) -> Result<AppDataResetProvisioningDebt> {
        self.storage
            .reconcile_provisioning_stages()
            .map(|unproven_stage_count| AppDataResetProvisioningDebt {
                unproven_stage_count,
            })
    }

    /// Commit the first durable reset intent or replace one completed intent
    /// without releasing the retained lock.
    pub(crate) fn begin(&mut self, prepared: &AppDataResetJournal) -> Result<()> {
        prepared.validate()?;
        if prepared.phase != AppDataResetPhase::Prepared
            || !prepared.is_bound_to_canonical_root_name(self.storage.data_root_binding().1)
        {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        if let Some(current) = self
            .storage
            .read_journal()?
            .map(|bytes| decode_journal(&bytes))
            && current?.phase != AppDataResetPhase::Complete
        {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        self.storage.write_journal(&encode_journal(prepared)?)
    }

    /// Upgrade a legacy V1 `Prepared`/`CacheDetached` record only after the
    /// exact canonical old root has been reopened by identity. The V2 binding
    /// is durable before any namespace effect may follow.
    pub(crate) fn bind_legacy_canonical_root(
        &mut self,
        expected: &AppDataResetJournal,
        binding: AppDataResetCanonicalRootBinding<'_>,
    ) -> Result<AppDataResetJournal> {
        if binding.root_name() != self.storage.data_root_binding().1 {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        let next = expected.bound_canonical_root(binding)?;
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != *expected {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        self.storage.write_journal(&encode_journal(&next)?)?;
        Ok(next)
    }

    /// Exact compare-and-advance while retaining the same exclusive lock.
    pub(crate) fn advance(
        &mut self,
        expected: &AppDataResetJournal,
        next_phase: AppDataResetPhase,
    ) -> Result<AppDataResetJournal> {
        expected.validate()?;
        if !expected.is_bound_to_canonical_root_name(self.storage.data_root_binding().1) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        let next = expected.advanced(next_phase)?;
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != *expected {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        self.storage.write_journal(&encode_journal(&next)?)?;
        Ok(next)
    }

    /// Exact compare-and-advance for the only transition that must seal a new
    /// filesystem identity into the durable coordinator.
    fn advance_fresh_namespace_identity(
        &mut self,
        expected: &AppDataResetJournal,
        fresh_identity: AppDataResetStoreIdentity,
    ) -> Result<AppDataResetJournal> {
        expected.validate()?;
        if !expected.is_bound_to_canonical_root_name(self.storage.data_root_binding().1) {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        let next = expected.advanced_fresh_namespace(fresh_identity)?;
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != *expected {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        self.storage.write_journal(&encode_journal(&next)?)?;
        Ok(next)
    }

    #[cfg(test)]
    fn advance_late_phase_for_test(
        &mut self,
        expected: &AppDataResetJournal,
        phase: AppDataResetPhase,
    ) -> Result<AppDataResetJournal> {
        let next = expected.advanced_late_phase_for_test(phase)?;
        let current = self
            .storage
            .read_journal()?
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            .and_then(|bytes| decode_journal(&bytes))?;
        if current != *expected {
            return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
        }
        self.storage.write_journal(&encode_journal(&next)?)?;
        Ok(next)
    }
}

fn map_cache_drain_error(
    drain_error: AppDataResetManagedCacheDrainError,
) -> AppDataResetCoordinatorError {
    match drain_error {
        AppDataResetManagedCacheDrainError::OutcomeUnknown => {
            error(AppDataResetCoordinatorErrorKind::OutcomeUnknown)
        }
        AppDataResetManagedCacheDrainError::BeforeEffect(kind) => {
            map_managed_cache_before_effect_error(kind)
        }
    }
}

fn map_managed_cache_before_effect_error(
    kind: ManagedCacheStoreErrorKind,
) -> AppDataResetCoordinatorError {
    let kind = match kind {
        ManagedCacheStoreErrorKind::Busy => AppDataResetCoordinatorErrorKind::Busy,
        ManagedCacheStoreErrorKind::Unavailable => AppDataResetCoordinatorErrorKind::Unavailable,
        ManagedCacheStoreErrorKind::OutcomeUnknown => {
            AppDataResetCoordinatorErrorKind::OutcomeUnknown
        }
        ManagedCacheStoreErrorKind::InvalidConfiguration
        | ManagedCacheStoreErrorKind::ReadOnly
        | ManagedCacheStoreErrorKind::UnsupportedPlatform
        | ManagedCacheStoreErrorKind::InternalState => {
            AppDataResetCoordinatorErrorKind::InternalState
        }
        ManagedCacheStoreErrorKind::UnsafeContainer
        | ManagedCacheStoreErrorKind::UnsafeStore
        | ManagedCacheStoreErrorKind::UnsafeObject
        | ManagedCacheStoreErrorKind::UnrecognizedStore
        | ManagedCacheStoreErrorKind::BudgetExceeded
        | ManagedCacheStoreErrorKind::ChangedSinceSnapshot
        | ManagedCacheStoreErrorKind::CorruptData => {
            AppDataResetCoordinatorErrorKind::ChangedSinceRead
        }
    };
    error(kind)
}

fn map_cache_stage_retirement_error(
    retirement_error: AppDataResetManagedCacheStageRetirementError,
) -> AppDataResetCoordinatorError {
    match retirement_error {
        AppDataResetManagedCacheStageRetirementError::OutcomeUnknown => {
            error(AppDataResetCoordinatorErrorKind::OutcomeUnknown)
        }
        AppDataResetManagedCacheStageRetirementError::BeforeEffect(kind) => {
            map_managed_cache_before_effect_error(kind)
        }
    }
}

fn map_snapshot_payload_drain_error(
    drain_error: AppDataResetSnapshotPayloadDrainError,
) -> AppDataResetCoordinatorError {
    match drain_error {
        AppDataResetSnapshotPayloadDrainError::OutcomeUnknown => {
            error(AppDataResetCoordinatorErrorKind::OutcomeUnknown)
        }
        AppDataResetSnapshotPayloadDrainError::BeforeEffect(kind) => {
            map_snapshot_before_effect_error(kind)
        }
    }
}

fn map_snapshot_store_retirement_error(
    retirement_error: AppDataResetSnapshotStoreRetirementError,
) -> AppDataResetCoordinatorError {
    match retirement_error {
        AppDataResetSnapshotStoreRetirementError::OutcomeUnknown => {
            error(AppDataResetCoordinatorErrorKind::OutcomeUnknown)
        }
        AppDataResetSnapshotStoreRetirementError::BeforeEffect(kind) => {
            map_snapshot_before_effect_error(kind)
        }
    }
}

fn map_snapshot_before_effect_error(
    kind: SnapshotStorageErrorKind,
) -> AppDataResetCoordinatorError {
    let kind = match kind {
        SnapshotStorageErrorKind::Busy => AppDataResetCoordinatorErrorKind::Busy,
        SnapshotStorageErrorKind::Unavailable => AppDataResetCoordinatorErrorKind::Unavailable,
        SnapshotStorageErrorKind::InvalidConfiguration
        | SnapshotStorageErrorKind::InternalState => {
            AppDataResetCoordinatorErrorKind::InternalState
        }
        SnapshotStorageErrorKind::UnsafeRoot
        | SnapshotStorageErrorKind::UnsafeObject
        | SnapshotStorageErrorKind::UnrecognizedStore => {
            AppDataResetCoordinatorErrorKind::ChangedSinceRead
        }
    };
    error(kind)
}

fn encode_journal(journal: &AppDataResetJournal) -> Result<Vec<u8>> {
    journal.validate()?;
    if journal.legacy_without_canonical_root_name || journal.legacy_complete_without_fresh_identity
    {
        return Err(error(AppDataResetCoordinatorErrorKind::IncompatibleJournal));
    }
    let payload = JournalPayloadV2::from(journal);
    let payload_bytes = serde_json::to_vec(&payload).map_err(|_| internal())?;
    let digest_sha256 = journal_digest(&payload_bytes);
    serde_json::to_vec(&JournalEnvelopeV2 {
        format_version: JOURNAL_FORMAT_VERSION,
        payload,
        digest_sha256,
    })
    .map_err(|_| internal())
}

fn decode_journal(bytes: &[u8]) -> Result<AppDataResetJournal> {
    let probe: JournalVersionProbe = serde_json::from_slice(bytes).map_err(|_| corrupt())?;
    if probe.format_version > JOURNAL_FORMAT_VERSION {
        return Err(error(AppDataResetCoordinatorErrorKind::IncompatibleJournal));
    }
    match probe.format_version {
        LEGACY_JOURNAL_FORMAT_VERSION => {
            let envelope: JournalEnvelopeV1 =
                serde_json::from_slice(bytes).map_err(|_| corrupt())?;
            let payload_bytes = serde_json::to_vec(&envelope.payload).map_err(|_| corrupt())?;
            if envelope.digest_sha256 != journal_digest(&payload_bytes)
                || serde_json::to_vec(&envelope).map_err(|_| corrupt())? != bytes
            {
                return Err(corrupt());
            }
            let journal = journal_from_v1(envelope.payload)?;
            journal.validate()?;
            Ok(journal)
        }
        JOURNAL_FORMAT_VERSION => {
            let envelope: JournalEnvelopeV2 =
                serde_json::from_slice(bytes).map_err(|_| corrupt())?;
            let payload_bytes = serde_json::to_vec(&envelope.payload).map_err(|_| corrupt())?;
            if envelope.digest_sha256 != journal_digest(&payload_bytes) {
                return Err(corrupt());
            }
            let journal = AppDataResetJournal::from(envelope.payload);
            journal.validate()?;
            if encode_journal(&journal)? != bytes {
                return Err(corrupt());
            }
            Ok(journal)
        }
        _ => Err(corrupt()),
    }
}

fn journal_digest(payload: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(DIGEST_DOMAIN);
    digest.update((payload.len() as u64).to_le_bytes());
    digest.update(payload);
    let digest: [u8; 32] = digest.finalize().into();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn error(kind: AppDataResetCoordinatorErrorKind) -> AppDataResetCoordinatorError {
    AppDataResetCoordinatorError::new(kind)
}

fn corrupt() -> AppDataResetCoordinatorError {
    error(AppDataResetCoordinatorErrorKind::CorruptJournal)
}

fn internal() -> AppDataResetCoordinatorError {
    error(AppDataResetCoordinatorErrorKind::InternalState)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier, mpsc};

    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    use tempfile::TempDir;

    use super::*;

    const FIRST_TRANSACTION: &str = "00112233445566778899aabbccddeeff";
    const SECOND_TRANSACTION: &str = "ffeeddccbbaa99887766554433221100";

    fn identities() -> (AppDataResetStoreIdentity, AppDataResetStoreIdentity) {
        (
            AppDataResetStoreIdentity::new(11, 22).unwrap(),
            AppDataResetStoreIdentity::new(33, 44).unwrap(),
        )
    }

    fn coordinator(temp: &TempDir) -> AppDataResetCoordinator {
        let data_root = temp.path().canonicalize().unwrap().join("Dux");
        AppDataResetCoordinator::open_or_create(&data_root).unwrap()
    }

    #[test]
    fn canonical_journal_round_trips_and_digest_detects_changes() {
        let (data, cache) = identities();
        let journal = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
            data,
            Some(cache),
        )
        .unwrap();
        let encoded = encode_journal(&journal).unwrap();

        assert_eq!(decode_journal(&encoded).unwrap(), journal);
        assert_eq!(journal.transaction_id(), FIRST_TRANSACTION);
        assert_eq!(journal.data_identity().device(), 11);
        assert_eq!(journal.data_identity().inode(), 22);
        assert_eq!(journal.cache_identity(), Some(cache));
        let reconstructed = journal.validated_transaction().unwrap();
        assert_eq!(reconstructed.transaction_id(), FIRST_TRANSACTION);
        assert_eq!(
            reconstructed.data_stage().as_str(),
            ".dux-reset-data-00112233445566778899aabbccddeeff"
        );
        assert_eq!(
            reconstructed.cache_stage().as_str(),
            ".dux-reset-cache-00112233445566778899aabbccddeeff"
        );
        assert_eq!(
            reconstructed.fresh_stage().as_str(),
            ".dux-reset-fresh-00112233445566778899aabbccddeeff"
        );
        assert_eq!(
            journal.data_stage_name(),
            ".dux-reset-data-00112233445566778899aabbccddeeff"
        );
        assert_eq!(
            journal.cache_stage_name(),
            Some(".dux-reset-cache-00112233445566778899aabbccddeeff")
        );
        assert_eq!(
            journal.fresh_stage_name(),
            ".dux-reset-fresh-00112233445566778899aabbccddeeff"
        );
        assert_eq!(journal.fresh_data_identity(), None);

        let mut wrong_role_name = journal.clone();
        wrong_role_name.data_stage_name =
            ".dux-reset-cache-00112233445566778899aabbccddeeff".to_owned();
        let error = match wrong_role_name.validated_transaction() {
            Ok(_) => panic!("role-swapped journal name reconstructed a transaction"),
            Err(error) => error,
        };
        assert_eq!(
            error.kind(),
            AppDataResetCoordinatorErrorKind::CorruptJournal
        );

        let mut changed: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        changed["payload"]["phase"] = serde_json::Value::String("cache_detached".into());
        let changed = serde_json::to_vec(&changed).unwrap();
        assert_eq!(
            decode_journal(&changed).unwrap_err().kind(),
            AppDataResetCoordinatorErrorKind::CorruptJournal
        );
    }

    #[test]
    fn canonical_root_codec_is_lossless_and_rejects_ambiguous_components() {
        let non_utf8 = OsStr::from_bytes(b"Dux-\xff");
        let encoded = encode_canonical_root_name(non_utf8).unwrap();
        assert_eq!(encoded, "4475782dff");
        assert_eq!(
            decode_canonical_root_name(&encoded).unwrap(),
            non_utf8.as_bytes()
        );

        for invalid in ["", "0", "00", "2e", "2e2e", "2f", "4A", "zz"] {
            assert_eq!(decode_canonical_root_name(invalid), None, "{invalid}");
        }

        let (data, _) = identities();
        let transaction = AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap();
        for reserved in [
            storage::COORDINATOR_DIRECTORY_NAME,
            transaction.data_stage().as_str(),
            transaction.cache_stage().as_str(),
            transaction.fresh_stage().as_str(),
        ] {
            assert_eq!(
                AppDataResetJournal::prepared_for_test_root(
                    &transaction,
                    data,
                    None,
                    OsStr::new(reserved),
                )
                .unwrap_err()
                .kind(),
                AppDataResetCoordinatorErrorKind::CorruptJournal
            );
        }
    }

    #[test]
    fn legacy_v1_prefix_requires_a_proven_root_upgrade_and_complete_stays_compatible() {
        let (data, cache) = identities();
        let encode_v1 = |phase| {
            let payload = JournalPayloadV1 {
                transaction_id: FIRST_TRANSACTION.to_owned(),
                phase,
                data_identity: data,
                cache_identity: Some(cache),
                data_stage_name: format!(".dux-reset-data-{FIRST_TRANSACTION}"),
                cache_stage_name: Some(format!(".dux-reset-cache-{FIRST_TRANSACTION}")),
            };
            let payload_bytes = serde_json::to_vec(&payload).unwrap();
            serde_json::to_vec(&JournalEnvelopeV1 {
                format_version: LEGACY_JOURNAL_FORMAT_VERSION,
                digest_sha256: journal_digest(&payload_bytes),
                payload,
            })
            .unwrap()
        };
        for phase in [
            AppDataResetPhase::Prepared,
            AppDataResetPhase::CacheDetached,
        ] {
            let journal = decode_journal(&encode_v1(phase)).unwrap();
            assert_eq!(journal.phase(), phase);
            assert!(!journal.has_canonical_root_name_binding());
            assert_eq!(journal.fresh_data_identity(), None);
        }
        assert_eq!(
            decode_journal(&encode_v1(AppDataResetPhase::DataDetached))
                .unwrap_err()
                .kind(),
            AppDataResetCoordinatorErrorKind::IncompatibleJournal
        );

        let complete_encoded = encode_v1(AppDataResetPhase::Complete);
        let complete = decode_journal(&complete_encoded).unwrap();
        assert_eq!(complete.phase(), AppDataResetPhase::Complete);
        assert_eq!(complete.fresh_data_identity(), None);
        assert!(complete.legacy_complete_without_fresh_identity);
        assert_eq!(
            encode_journal(&complete).unwrap_err().kind(),
            AppDataResetCoordinatorErrorKind::IncompatibleJournal
        );

        let temp = TempDir::new().unwrap();
        let data_root = temp.path().canonicalize().unwrap().join("data");
        let complete_coordinator = AppDataResetCoordinator::open_or_create(&data_root).unwrap();
        complete_coordinator
            .with_exclusive_session(|session| session.storage.write_journal(&complete_encoded))
            .unwrap();
        drop(complete_coordinator);
        assert!(matches!(
            AppDataResetCoordinator::acquire_engine_lease_until(
                &data_root,
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap(),
            AppDataResetEngineLeaseOutcome::Admitted(_)
        ));

        let upgrade_temp = TempDir::new().unwrap();
        let upgrade_coordinator = coordinator(&upgrade_temp);
        let legacy_prepared = encode_v1(AppDataResetPhase::Prepared);
        upgrade_coordinator
            .with_exclusive_session(|session| session.storage.write_journal(&legacy_prepared))
            .unwrap();
        let upgraded = upgrade_coordinator
            .with_exclusive_session(|session| {
                let legacy = session.recover()?.unwrap();
                session.bind_legacy_canonical_root(
                    &legacy,
                    AppDataResetCanonicalRootBinding::for_test(OsStr::new("Dux"), data),
                )
            })
            .unwrap();
        assert!(upgraded.has_canonical_root_name_binding());
        assert!(upgraded.is_bound_to_canonical_root_name(OsStr::new("Dux")));
        assert_eq!(upgrade_coordinator.recover().unwrap(), Some(upgraded));

        assert_eq!(
            decode_journal(&encode_v1(AppDataResetPhase::FreshNamespaceReady))
                .unwrap_err()
                .kind(),
            AppDataResetCoordinatorErrorKind::IncompatibleJournal
        );
        assert_eq!(
            decode_journal(&encode_v1(AppDataResetPhase::Draining))
                .unwrap_err()
                .kind(),
            AppDataResetCoordinatorErrorKind::IncompatibleJournal
        );
    }

    #[test]
    fn journal_rejects_noncanonical_unknown_and_newer_shapes() {
        let (data, _) = identities();
        let journal = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
            data,
            None,
        )
        .unwrap();
        let encoded = encode_journal(&journal).unwrap();

        let mut whitespace = encoded.clone();
        whitespace.push(b'\n');
        assert_eq!(
            decode_journal(&whitespace).unwrap_err().kind(),
            AppDataResetCoordinatorErrorKind::CorruptJournal
        );

        let mut unknown: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        unknown["unexpected"] = serde_json::Value::Bool(true);
        assert_eq!(
            decode_journal(&serde_json::to_vec(&unknown).unwrap())
                .unwrap_err()
                .kind(),
            AppDataResetCoordinatorErrorKind::CorruptJournal
        );

        let mut newer: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        newer["format_version"] = serde_json::Value::from(JOURNAL_FORMAT_VERSION + 1);
        assert_eq!(
            decode_journal(&serde_json::to_vec(&newer).unwrap())
                .unwrap_err()
                .kind(),
            AppDataResetCoordinatorErrorKind::IncompatibleJournal
        );
    }

    #[test]
    fn singleton_moves_forward_by_exact_compare_only() {
        let temp = TempDir::new().unwrap();
        let coordinator = coordinator(&temp);
        let (data, cache) = identities();
        let prepared = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
            data,
            Some(cache),
        )
        .unwrap();

        assert_eq!(coordinator.recover().unwrap(), None);
        coordinator.begin(&prepared).unwrap();
        assert_eq!(coordinator.recover().unwrap(), Some(prepared.clone()));

        let cache_detached = coordinator
            .advance(&prepared, AppDataResetPhase::CacheDetached)
            .unwrap();
        assert_eq!(coordinator.recover().unwrap(), Some(cache_detached.clone()));
        assert_eq!(
            coordinator
                .advance(&prepared, AppDataResetPhase::CacheDetached)
                .unwrap_err()
                .kind(),
            AppDataResetCoordinatorErrorKind::ChangedSinceRead
        );
        assert_eq!(
            coordinator
                .advance(&cache_detached, AppDataResetPhase::FreshNamespaceReady)
                .unwrap_err()
                .kind(),
            AppDataResetCoordinatorErrorKind::InvalidTransition
        );

        let data_detached = coordinator
            .advance(&cache_detached, AppDataResetPhase::DataDetached)
            .unwrap();
        let fresh = coordinator
            .with_exclusive_session(|session| {
                session.advance_fresh_namespace_identity(
                    &data_detached,
                    AppDataResetStoreIdentity::new(55, 66).unwrap(),
                )
            })
            .unwrap();
        assert_eq!(
            fresh.fresh_data_identity(),
            AppDataResetStoreIdentity::new(55, 66)
        );
        assert_eq!(
            coordinator
                .advance(&fresh, AppDataResetPhase::Draining)
                .unwrap_err()
                .kind(),
            AppDataResetCoordinatorErrorKind::InvalidTransition
        );
        let draining = coordinator
            .advance_late_phase_for_test(&fresh, AppDataResetPhase::Draining)
            .unwrap();
        assert_eq!(
            coordinator
                .advance(&draining, AppDataResetPhase::Complete)
                .unwrap_err()
                .kind(),
            AppDataResetCoordinatorErrorKind::InvalidTransition
        );
        let complete = coordinator
            .advance_late_phase_for_test(&draining, AppDataResetPhase::Complete)
            .unwrap();
        assert_eq!(complete.phase(), AppDataResetPhase::Complete);

        let replacement = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(SECOND_TRANSACTION).unwrap(),
            data,
            None,
        )
        .unwrap();
        coordinator.begin(&replacement).unwrap();
        assert_eq!(coordinator.recover().unwrap(), Some(replacement));
    }

    #[test]
    fn incomplete_singleton_cannot_be_replaced() {
        let temp = TempDir::new().unwrap();
        let coordinator = coordinator(&temp);
        let (data, cache) = identities();
        let first = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
            data,
            Some(cache),
        )
        .unwrap();
        let second = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(SECOND_TRANSACTION).unwrap(),
            data,
            None,
        )
        .unwrap();
        coordinator.begin(&first).unwrap();

        assert_eq!(
            coordinator.begin(&second).unwrap_err().kind(),
            AppDataResetCoordinatorErrorKind::InvalidTransition
        );
        assert_eq!(coordinator.recover().unwrap(), Some(first));
    }

    #[test]
    fn recovery_intent_rejects_journal_change_during_shared_to_exclusive_handoff() {
        let temp = TempDir::new().unwrap();
        let coordinator = coordinator(&temp);
        let data_root = temp.path().canonicalize().unwrap().join("Dux");
        let (data, _) = identities();
        let prepared = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
            data,
            None,
        )
        .unwrap();
        coordinator.begin(&prepared).unwrap();
        let changed = self::coordinator(&temp);

        let intent = match AppDataResetCoordinator::acquire_engine_lease_until(
            &data_root,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap()
        {
            AppDataResetEngineLeaseOutcome::RecoveryRequired(intent) => *intent,
            AppDataResetEngineLeaseOutcome::Admitted(_) => {
                panic!("incomplete journal unexpectedly admitted ordinary engine")
            }
        };
        let expected_for_hook = prepared.clone();
        let error = intent
            .with_exclusive_session_with_handoff_hook_until(
                Instant::now() + Duration::from_secs(1),
                move || {
                    changed
                        .advance(&expected_for_hook, AppDataResetPhase::CacheDetached)
                        .unwrap();
                },
                |_, _| -> Result<()> { panic!("changed journal entered recovery callback") },
            )
            .unwrap_err();
        assert_eq!(
            error.kind(),
            AppDataResetCoordinatorErrorKind::ChangedSinceRead
        );
        assert_eq!(
            coordinator.recover().unwrap().unwrap().phase(),
            AppDataResetPhase::CacheDetached
        );
    }

    #[test]
    fn retained_session_sequences_journal_transitions_without_relocking() {
        let temp = TempDir::new().unwrap();
        let coordinator = coordinator(&temp);
        let (data, cache) = identities();
        let prepared = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
            data,
            Some(cache),
        )
        .unwrap();

        coordinator
            .with_exclusive_session(|session| {
                assert_eq!(session.recover()?, None);
                session.begin(&prepared)?;
                assert_eq!(session.recover()?, Some(prepared.clone()));
                let cache_detached =
                    session.advance(&prepared, AppDataResetPhase::CacheDetached)?;
                let data_detached =
                    session.advance(&cache_detached, AppDataResetPhase::DataDetached)?;
                assert_eq!(session.recover()?, Some(data_detached));
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn nested_same_instance_session_is_busy_without_unlocking_outer_session() {
        let temp = TempDir::new().unwrap();
        let coordinator = coordinator(&temp);
        let independent = self::coordinator(&temp);
        let (data, _) = identities();
        let prepared = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
            data,
            None,
        )
        .unwrap();

        coordinator
            .with_exclusive_session(|session| {
                assert_eq!(
                    coordinator.recover().unwrap_err().kind(),
                    AppDataResetCoordinatorErrorKind::Busy
                );
                assert_eq!(
                    independent
                        .with_exclusive_session_with_timeout(Duration::ZERO, |other| {
                            other.recover()
                        })
                        .unwrap_err()
                        .kind(),
                    AppDataResetCoordinatorErrorKind::Busy
                );
                session.begin(&prepared)?;
                assert_eq!(session.recover()?, Some(prepared.clone()));
                Ok(())
            })
            .unwrap();
        assert_eq!(coordinator.recover().unwrap(), Some(prepared));
    }

    #[test]
    fn simultaneous_same_instance_sessions_admit_exactly_one_callback() {
        let temp = TempDir::new().unwrap();
        let coordinator = Arc::new(coordinator(&temp));
        let start = Arc::new(Barrier::new(3));
        let release = Arc::new(Barrier::new(2));
        let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
        let (result_sender, result_receiver) = mpsc::sync_channel(2);
        let mut workers = Vec::new();

        for _ in 0..2 {
            let coordinator = Arc::clone(&coordinator);
            let start = Arc::clone(&start);
            let release = Arc::clone(&release);
            let entered_sender = entered_sender.clone();
            let result_sender = result_sender.clone();
            workers.push(std::thread::spawn(move || {
                start.wait();
                let result = coordinator.with_exclusive_session(|_| {
                    entered_sender.send(()).unwrap();
                    release.wait();
                    Ok(())
                });
                result_sender
                    .send(result.map_err(|error| error.kind()))
                    .unwrap();
            }));
        }
        drop(entered_sender);
        drop(result_sender);

        start.wait();
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let first_result = result_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(first_result, Err(AppDataResetCoordinatorErrorKind::Busy));
        assert!(
            entered_receiver.try_recv().is_err(),
            "more than one coordinator callback entered"
        );
        release.wait();
        assert_eq!(
            result_receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            Ok(())
        );
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn transition_error_does_not_release_retained_session() {
        let temp = TempDir::new().unwrap();
        let first = coordinator(&temp);
        let independent = coordinator(&temp);
        let (data, _) = identities();
        let prepared = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
            data,
            None,
        )
        .unwrap();

        first
            .with_exclusive_session(|session| {
                session.begin(&prepared)?;
                assert_eq!(
                    session
                        .advance(&prepared, AppDataResetPhase::DataDetached)
                        .unwrap_err()
                        .kind(),
                    AppDataResetCoordinatorErrorKind::InvalidTransition
                );
                assert_eq!(
                    independent
                        .with_exclusive_session_with_timeout(Duration::ZERO, |other| {
                            other.recover()
                        })
                        .unwrap_err()
                        .kind(),
                    AppDataResetCoordinatorErrorKind::Busy
                );
                let next = session.advance(&prepared, AppDataResetPhase::CacheDetached)?;
                assert_eq!(session.recover()?, Some(next));
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn store_admission_is_nested_inside_retained_coordinator_session() {
        let temp = TempDir::new().unwrap();
        let coordinator = coordinator(&temp);
        let independent = self::coordinator(&temp);
        let store =
            StoreCoordinator::open(&temp.path().canonicalize().unwrap().join("Dux/dux.sqlite3"))
                .unwrap();

        coordinator
            .with_exclusive_session(|session| {
                session
                    .with_admitted_store(&store, |session, guard| {
                        assert!(guard.revalidate().unwrap().is_empty());
                        assert_eq!(
                            independent
                                .with_exclusive_session_with_timeout(Duration::ZERO, |other| {
                                    other.recover()
                                })
                                .unwrap_err()
                                .kind(),
                            AppDataResetCoordinatorErrorKind::Busy
                        );
                        assert_eq!(session.recover().unwrap(), None);
                    })
                    .map(|outcome| match outcome {
                        AppDataResetAdmittedStoreOutcome::Admitted(()) => {}
                        AppDataResetAdmittedStoreOutcome::Blocked(_) => {
                            panic!("empty current store unexpectedly blocked reset admission");
                        }
                    })
                    .unwrap();
                assert_eq!(session.recover()?, None);
                Ok(())
            })
            .unwrap();
        assert_eq!(independent.recover().unwrap(), None);
    }

    #[test]
    fn coordinator_and_retained_session_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<AppDataResetCoordinator>();
        assert_send::<AppDataResetCoordinatorSession<'static>>();
    }

    #[test]
    fn independent_coordinator_stays_excluded_until_retained_session_releases() {
        let temp = TempDir::new().unwrap();
        let first = coordinator(&temp);
        let second = coordinator(&temp);
        let (data, _) = identities();
        let prepared = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
            data,
            None,
        )
        .unwrap();

        first
            .with_exclusive_session(|session| {
                session.begin(&prepared)?;
                assert_eq!(
                    second
                        .with_exclusive_session_with_timeout(Duration::ZERO, |other| {
                            other.recover()
                        })
                        .unwrap_err()
                        .kind(),
                    AppDataResetCoordinatorErrorKind::Busy
                );
                assert_eq!(session.recover()?, Some(prepared.clone()));
                Ok(())
            })
            .unwrap();
        assert_eq!(second.recover().unwrap(), Some(prepared));
    }

    #[test]
    fn callback_error_and_panic_release_session_without_rolling_back_journal() {
        let temp = TempDir::new().unwrap();
        let coordinator = coordinator(&temp);
        let (data, _) = identities();
        let prepared = AppDataResetJournal::prepared_for_test(
            &AppDataResetTransaction::for_test(FIRST_TRANSACTION).unwrap(),
            data,
            None,
        )
        .unwrap();

        let error = coordinator
            .with_exclusive_session(|session| {
                session.begin(&prepared)?;
                Err::<(), _>(error(AppDataResetCoordinatorErrorKind::Unavailable))
            })
            .unwrap_err();
        assert_eq!(error.kind(), AppDataResetCoordinatorErrorKind::Unavailable);
        assert_eq!(coordinator.recover().unwrap(), Some(prepared.clone()));

        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<()> = coordinator.with_exclusive_session(|session| {
                assert_eq!(session.recover()?, Some(prepared.clone()));
                panic!("simulate reset coordinator callback panic");
            });
        }));
        assert!(panic.is_err());
        assert_eq!(coordinator.recover().unwrap(), Some(prepared));
    }

    #[test]
    fn fixed_sibling_is_private_and_does_not_create_data_root() {
        let temp = TempDir::new().unwrap();
        let canonical = temp.path().canonicalize().unwrap();
        let data_root = canonical.join("Dux");
        let coordinator = AppDataResetCoordinator::open_or_create(&data_root).unwrap();

        assert!(!data_root.exists());
        let root = canonical.join(storage::COORDINATOR_DIRECTORY_NAME);
        let metadata = fs::metadata(&root).unwrap();
        assert_eq!(metadata.mode() & 0o7777, 0o700);
        assert_eq!(metadata.uid(), unsafe { nix::libc::geteuid() });
        assert_eq!(coordinator.recover().unwrap(), None);
        assert_eq!(
            coordinator
                .provisioning_debt()
                .unwrap()
                .unproven_stage_count(),
            0
        );
    }

    #[test]
    fn symlink_and_permissive_coordinator_are_rejected() {
        let symlink_case = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        symlink(
            outside.path(),
            symlink_case
                .path()
                .join(storage::COORDINATOR_DIRECTORY_NAME),
        )
        .unwrap();
        let symlink_root = symlink_case.path().canonicalize().unwrap().join("Dux");
        let error = match AppDataResetCoordinator::open_or_create(&symlink_root) {
            Ok(_) => panic!("symlink coordinator unexpectedly opened"),
            Err(error) => error,
        };
        assert_eq!(
            error.kind(),
            AppDataResetCoordinatorErrorKind::UnsafeCoordinator
        );

        let mode_case = TempDir::new().unwrap();
        let coordinator = coordinator(&mode_case);
        drop(coordinator);
        let canonical = mode_case.path().canonicalize().unwrap();
        let root = canonical.join(storage::COORDINATOR_DIRECTORY_NAME);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        let error = match AppDataResetCoordinator::open_or_create(&canonical.join("Dux")) {
            Ok(_) => panic!("permissive coordinator unexpectedly opened"),
            Err(error) => error,
        };
        assert_eq!(
            error.kind(),
            AppDataResetCoordinatorErrorKind::UnsafeCoordinator
        );
    }
}
