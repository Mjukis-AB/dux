//! Crash-safe coordination for the private whole-app-data reset pipeline.
//!
//! The independently marker-owned coordinator persists the exact checksummed
//! state machine and composes consume-once namespace capabilities. It owns no
//! caller-selected path: after exact durable `Draining` read-back it can mint
//! only distinct opaque capabilities for one managed-cache payload/structure,
//! one old snapshot payload/structure, one old SQLite payload, or one
//! protocol-ordered old-store structure.
//! Core engine recovery remains private; FFI and native reset effect
//! integration remain separate work.

mod journal_codec;
mod session;
mod storage;

#[cfg(test)]
use journal_codec::{JournalEnvelopeV1, JournalPayloadV1, journal_digest};
use journal_codec::{
    decode_canonical_root_name, decode_journal, encode_canonical_root_name, encode_journal,
};

#[cfg(test)]
pub(crate) use storage::{TestJournalWriteFault, set_test_journal_write_fault};

use std::ffi::OsStr;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::app_data_reset_transaction::AppDataResetTransaction;
use crate::cache::{
    AppDataResetManagedCacheAbsentWitness, AppDataResetManagedCacheDrainBatch,
    AppDataResetManagedCacheDrainCandidate, AppDataResetManagedCacheDrainError,
    AppDataResetManagedCacheStageRetirementBatch, AppDataResetManagedCacheStageRetirementCandidate,
    AppDataResetManagedCacheStageRetirementError, ManagedCacheStore, ManagedCacheStoreErrorKind,
};

use self::storage::{ResetCoordinatorEngineLease, ResetCoordinatorStorage};
use super::history::{self, HistoryError};
use super::snapshot::{
    AppDataResetSnapshotPayloadDrainBatch, AppDataResetSnapshotPayloadDrainError,
    AppDataResetSnapshotStoreRetirementBatch, AppDataResetSnapshotStoreRetirementError,
    SnapshotStorageErrorKind,
};
use super::status::{DatabaseOpenError, DatabaseOpenErrorKind};
use super::store::{
    AppDataResetCanonicalRootBinding, AppDataResetCompletedStoreOpenHooks,
    AppDataResetDataNamespaceAdmission, AppDataResetFreshNamespace,
    AppDataResetFreshOriginRetirementError, AppDataResetOldDatabaseDrainingAdmission,
    AppDataResetOldDatabasePayloadDrainBatch, AppDataResetOldDatabasePayloadDrainCandidate,
    AppDataResetOldDatabasePayloadDrainError, AppDataResetOldDatabasePayloadState,
    AppDataResetOldDatabaseStoreAbsentWitness, AppDataResetOldDatabaseStoreRetirementBatch,
    AppDataResetOldDatabaseStoreRetirementCandidate, AppDataResetOldDatabaseStoreRetirementError,
    AppDataResetOldSnapshotPayloadDrainCandidate, AppDataResetOldSnapshotStoreRetirementCandidate,
    AppDataResetPublishedFreshNamespace, AppDataResetReadyToDrainNamespace,
    AppDataResetRecoveryDataNamespace, AppDataResetStoreAdmission, StoreCoordinator,
};
use super::{AppDataResetStoreBlockers, AppDataResetStoreGuard};

const JOURNAL_FORMAT_VERSION: u16 = 2;
const LEGACY_JOURNAL_FORMAT_VERSION: u16 = 1;
const DIGEST_DOMAIN: &[u8] = b"dux-app-data-reset-journal-v1\0";
const MAX_CANONICAL_ROOT_NAME_BYTES: usize = 255;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestAppDataResetCoordinatorPostcheckFault {
    ExhaustBeforeCacheReadback,
    ExhaustBeforeJournalReadback,
}

#[cfg(test)]
std::thread_local! {
    static TEST_APP_DATA_RESET_COORDINATOR_POSTCHECK_FAULT:
        std::cell::Cell<Option<TestAppDataResetCoordinatorPostcheckFault>> =
            const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_test_app_data_reset_coordinator_postcheck_fault(
    fault: TestAppDataResetCoordinatorPostcheckFault,
) {
    TEST_APP_DATA_RESET_COORDINATOR_POSTCHECK_FAULT.with(|current| current.set(Some(fault)));
}

#[cfg(test)]
fn take_test_app_data_reset_coordinator_postcheck_fault(
    expected: TestAppDataResetCoordinatorPostcheckFault,
) -> bool {
    TEST_APP_DATA_RESET_COORDINATOR_POSTCHECK_FAULT.with(|current| {
        if current.get() == Some(expected) {
            current.set(None);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
fn exhaust_coordinator_postcheck_deadline(deadline: Instant) {
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

    fn advanced_complete(&self) -> Result<Self> {
        if self.phase != AppDataResetPhase::Draining
            || self.legacy_without_canonical_root_name
            || self.legacy_complete_without_fresh_identity
            || self.canonical_root_name_hex.is_none()
            || self.fresh_data_identity.is_none()
        {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        let mut advanced = self.clone();
        advanced.phase = AppDataResetPhase::Complete;
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
    CompletedStateValidationRequired(Box<AppDataResetCompletedEngineIntent>),
    RecoveryRequired(Box<AppDataResetRecoveryIntent>),
}

/// Move-only V2 completed-state intent. It retains the exact coordinator
/// storage and the original shared lease until the physical envelope has been
/// validated and the existing canonical store has opened without provisioning.
pub(crate) struct AppDataResetCompletedEngineIntent {
    storage: ResetCoordinatorStorage,
    engine_lease: ResetCoordinatorEngineLease,
    journal: AppDataResetJournal,
}

#[derive(Debug)]
pub(crate) enum AppDataResetCompletedEngineOpenError {
    Reset(AppDataResetCoordinatorError),
    Database(DatabaseOpenErrorKind),
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

/// Consume-once composition for one detached old SQLite sidecar or the main
/// database after both managed cache and snapshot storage are exactly absent.
/// Initialization/lock controls and the detached root remain unreachable.
pub(crate) struct AppDataResetDrainingOldDatabasePayloadBatch<'data, 'cache> {
    journal: AppDataResetJournal,
    data: AppDataResetOldDatabasePayloadDrainCandidate<'data>,
    cache: AppDataResetManagedCacheAbsentWitness<'cache>,
    pre_effect_deadline: Instant,
}

/// Consume-once composition for one detached old-store control or empty root
/// after every payload and managed-cache stage are exactly absent.
pub(crate) struct AppDataResetDrainingOldDatabaseStoreRetirementBatch<'data, 'cache> {
    journal: AppDataResetJournal,
    data: AppDataResetOldDatabaseStoreRetirementCandidate<'data>,
    cache: AppDataResetManagedCacheAbsentWitness<'cache>,
    pre_effect_deadline: Instant,
}

/// Consume-once completion batch after both detached stages are exactly
/// absent. A present transaction-origin record may be retired; only a later
/// origin-absent pass can publish the durable `Complete` tombstone.
pub(crate) struct AppDataResetCompletionBatch<'data, 'cache> {
    journal: AppDataResetJournal,
    data: AppDataResetOldDatabaseStoreAbsentWitness<'data>,
    cache: AppDataResetManagedCacheAbsentWitness<'cache>,
    pre_effect_deadline: Instant,
}

pub(crate) enum AppDataResetCompletionBatchOutcome {
    OriginRetired,
    Complete(AppDataResetJournal),
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

/// Opaque coordinator-only capability for one old SQLite payload unlink. The
/// storage layer cannot construct it from a database inventory observation.
pub(crate) struct AppDataResetOldDatabasePayloadDrainAuthority {
    pre_effect_deadline: Instant,
}

/// Opaque coordinator-only capability for one old-store control or root-shell
/// removal. It is deliberately distinct from SQLite payload authority.
pub(crate) struct AppDataResetOldDatabaseStoreRetireAuthority {
    pre_effect_deadline: Instant,
}

/// Opaque coordinator-only capability for removing the transaction origin
/// from an otherwise complete fresh bootstrap. It cannot reach any database,
/// cache, snapshot, or user payload.
pub(crate) struct AppDataResetFreshOriginRetireAuthority {
    pre_effect_deadline: Instant,
}

impl AppDataResetFreshOriginRetireAuthority {
    pub(crate) const fn pre_effect_deadline(&self) -> Instant {
        self.pre_effect_deadline
    }
}

impl AppDataResetOldDatabaseStoreRetireAuthority {
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

impl AppDataResetOldDatabasePayloadDrainAuthority {
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
        if let Some(journal) = journal {
            if journal.phase() != AppDataResetPhase::Complete {
                return Ok(AppDataResetEngineLeaseOutcome::RecoveryRequired(Box::new(
                    AppDataResetRecoveryIntent {
                        storage,
                        engine_lease: lease,
                        journal,
                    },
                )));
            }
            if !journal.legacy_complete_without_fresh_identity {
                return Ok(
                    AppDataResetEngineLeaseOutcome::CompletedStateValidationRequired(Box::new(
                        AppDataResetCompletedEngineIntent {
                            storage,
                            engine_lease: lease,
                            journal,
                        },
                    )),
                );
            }
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

    #[cfg(test)]
    pub(crate) fn install_legacy_complete_for_test(
        &self,
        transaction: &AppDataResetTransaction,
        data_identity: AppDataResetStoreIdentity,
        cache_identity: Option<AppDataResetStoreIdentity>,
    ) -> Result<AppDataResetJournal> {
        let payload = JournalPayloadV1 {
            transaction_id: transaction.transaction_id().to_owned(),
            phase: AppDataResetPhase::Complete,
            data_identity,
            cache_identity,
            data_stage_name: transaction.data_stage().as_str().to_owned(),
            cache_stage_name: Some(transaction.cache_stage().as_str().to_owned()),
        };
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|_| error(AppDataResetCoordinatorErrorKind::InternalState))?;
        let encoded = serde_json::to_vec(&JournalEnvelopeV1 {
            format_version: LEGACY_JOURNAL_FORMAT_VERSION,
            digest_sha256: journal_digest(&payload_bytes),
            payload,
        })
        .map_err(|_| error(AppDataResetCoordinatorErrorKind::InternalState))?;
        self.with_exclusive_session(|session| session.storage.write_journal(&encoded))?;
        decode_journal(&encoded)
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

impl AppDataResetCompletedEngineIntent {
    /// Validate the durable V2 completion envelope, open only its existing
    /// identity-bound canonical store, then repeat the envelope before
    /// converting the retained shared exclusion into an ordinary engine lease.
    pub(crate) fn open_store_until(
        self,
        database_path: &Path,
        cache_directory: &Path,
        deadline: Instant,
    ) -> std::result::Result<
        (AppDataResetEngineLease, Arc<StoreCoordinator>),
        AppDataResetCompletedEngineOpenError,
    > {
        self.open_store_until_inner(
            database_path,
            cache_directory,
            deadline,
            || {},
            || {},
            || {},
        )
    }

    #[cfg(test)]
    pub(crate) fn open_store_until_with_pre_effect_hook(
        self,
        database_path: &Path,
        cache_directory: &Path,
        deadline: Instant,
        before_store_effect: impl FnOnce(),
    ) -> std::result::Result<
        (AppDataResetEngineLease, Arc<StoreCoordinator>),
        AppDataResetCompletedEngineOpenError,
    > {
        self.open_store_until_inner(
            database_path,
            cache_directory,
            deadline,
            || {},
            before_store_effect,
            || {},
        )
    }

    #[cfg(test)]
    pub(crate) fn open_store_until_with_preparation_hook(
        self,
        database_path: &Path,
        cache_directory: &Path,
        deadline: Instant,
        after_envelope_validation: impl FnOnce(),
    ) -> std::result::Result<
        (AppDataResetEngineLease, Arc<StoreCoordinator>),
        AppDataResetCompletedEngineOpenError,
    > {
        self.open_store_until_inner(
            database_path,
            cache_directory,
            deadline,
            after_envelope_validation,
            || {},
            || {},
        )
    }

    #[cfg(test)]
    pub(crate) fn open_store_until_with_final_cache_fence_hook(
        self,
        database_path: &Path,
        cache_directory: &Path,
        deadline: Instant,
        after_cache_fence: impl FnOnce(),
    ) -> std::result::Result<
        (AppDataResetEngineLease, Arc<StoreCoordinator>),
        AppDataResetCompletedEngineOpenError,
    > {
        self.open_store_until_inner(
            database_path,
            cache_directory,
            deadline,
            || {},
            || {},
            after_cache_fence,
        )
    }

    fn open_store_until_inner(
        self,
        database_path: &Path,
        cache_directory: &Path,
        deadline: Instant,
        after_envelope_validation: impl FnOnce(),
        before_store_effect: impl FnOnce(),
        after_cache_fence: impl FnOnce(),
    ) -> std::result::Result<
        (AppDataResetEngineLease, Arc<StoreCoordinator>),
        AppDataResetCompletedEngineOpenError,
    > {
        let Self {
            storage,
            engine_lease,
            journal,
        } = self;
        if Instant::now() >= deadline {
            return Err(AppDataResetCompletedEngineOpenError::Reset(error(
                AppDataResetCoordinatorErrorKind::Busy,
            )));
        }
        if journal.phase() != AppDataResetPhase::Complete
            || journal.legacy_complete_without_fresh_identity
        {
            return Err(AppDataResetCompletedEngineOpenError::Reset(error(
                AppDataResetCoordinatorErrorKind::InvalidTransition,
            )));
        }
        let validate_journal = || -> Result<()> {
            let current = storage
                .read_journal_for_engine_validation_until(deadline)?
                .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
                .and_then(|bytes| decode_journal(&bytes))?;
            if current == journal {
                Ok(())
            } else {
                Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
            }
        };
        validate_journal().map_err(AppDataResetCompletedEngineOpenError::Reset)?;
        let transaction = journal
            .validated_transaction()
            .map_err(AppDataResetCompletedEngineOpenError::Reset)?;
        let fresh_identity = journal
            .fresh_data_identity()
            .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::InvalidTransition))
            .map_err(AppDataResetCompletedEngineOpenError::Reset)?;
        let retired_cache_identity = journal
            .cache_identity()
            .map(|identity| (identity.device(), identity.inode()));
        let allow_canonical_cache =
            StoreCoordinator::app_data_reset_completed_root_is_initialized_until(
                database_path,
                &transaction,
                fresh_identity,
                deadline,
            )
            .map_err(map_completed_store_envelope_error)?;
        {
            let _cache_fence =
                ManagedCacheStore::validate_app_data_reset_completed_namespace_until(
                    cache_directory,
                    retired_cache_identity,
                    transaction.cache_stage(),
                    allow_canonical_cache,
                    deadline,
                )
                .map_err(|error| {
                    AppDataResetCompletedEngineOpenError::Reset(
                        map_managed_cache_before_effect_error(error.kind()),
                    )
                })?;
            validate_journal().map_err(AppDataResetCompletedEngineOpenError::Reset)?;
        }
        after_envelope_validation();
        let store = StoreCoordinator::open_after_app_data_reset_completion_with_pre_effect_hook(
            database_path,
            &transaction,
            fresh_identity,
            allow_canonical_cache,
            deadline,
            AppDataResetCompletedStoreOpenHooks::new(
                || {
                    before_store_effect();
                    Ok(())
                },
                || {
                    let cache_fence =
                        ManagedCacheStore::validate_app_data_reset_completed_namespace_until(
                            cache_directory,
                            retired_cache_identity,
                            transaction.cache_stage(),
                            allow_canonical_cache,
                            deadline,
                        )
                        .map_err(|error| {
                            completed_reset_envelope_as_database_error(
                                map_managed_cache_before_effect_error(error.kind()),
                            )
                        })?;
                    validate_journal().map_err(completed_reset_envelope_as_database_error)?;
                    Ok(cache_fence)
                },
                || {
                    after_cache_fence();
                    Ok(())
                },
            ),
        )
        .map_err(map_completed_store_admission_error)?;
        store
            .revalidate_app_data_reset_completed_namespace_until(
                &transaction,
                fresh_identity,
                allow_canonical_cache,
                deadline,
            )
            .map_err(map_completed_store_envelope_error)?;
        {
            let _cache_fence =
                ManagedCacheStore::validate_app_data_reset_completed_namespace_until(
                    cache_directory,
                    retired_cache_identity,
                    transaction.cache_stage(),
                    allow_canonical_cache,
                    deadline,
                )
                .map_err(|error| {
                    AppDataResetCompletedEngineOpenError::Reset(
                        map_managed_cache_before_effect_error(error.kind()),
                    )
                })?;
            validate_journal().map_err(AppDataResetCompletedEngineOpenError::Reset)?;
        }
        if Instant::now() >= deadline {
            return Err(AppDataResetCompletedEngineOpenError::Reset(error(
                AppDataResetCoordinatorErrorKind::Busy,
            )));
        }
        Ok((
            AppDataResetEngineLease {
                _storage: engine_lease,
            },
            store,
        ))
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

fn map_old_database_payload_drain_error(
    drain_error: AppDataResetOldDatabasePayloadDrainError,
) -> AppDataResetCoordinatorError {
    match drain_error {
        AppDataResetOldDatabasePayloadDrainError::OutcomeUnknown => {
            error(AppDataResetCoordinatorErrorKind::OutcomeUnknown)
        }
        AppDataResetOldDatabasePayloadDrainError::BeforeEffect(kind) => {
            let kind = match kind {
                DatabaseOpenErrorKind::Busy => AppDataResetCoordinatorErrorKind::Busy,
                DatabaseOpenErrorKind::StorageRootUnavailable
                | DatabaseOpenErrorKind::DatabaseUnavailable
                | DatabaseOpenErrorKind::InspectionLimitExceeded
                | DatabaseOpenErrorKind::MigrationFailed => {
                    AppDataResetCoordinatorErrorKind::Unavailable
                }
                DatabaseOpenErrorKind::InternalState => {
                    AppDataResetCoordinatorErrorKind::InternalState
                }
                DatabaseOpenErrorKind::UnsafeStorageRoot
                | DatabaseOpenErrorKind::UnsafeStorageObject
                | DatabaseOpenErrorKind::UnsafePermissions
                | DatabaseOpenErrorKind::OwnershipMismatch
                | DatabaseOpenErrorKind::UnrecognizedDatabase
                | DatabaseOpenErrorKind::CorruptDatabase => {
                    AppDataResetCoordinatorErrorKind::ChangedSinceRead
                }
            };
            error(kind)
        }
    }
}

fn map_old_database_store_retirement_error(
    retirement_error: AppDataResetOldDatabaseStoreRetirementError,
) -> AppDataResetCoordinatorError {
    match retirement_error {
        AppDataResetOldDatabaseStoreRetirementError::OutcomeUnknown => {
            error(AppDataResetCoordinatorErrorKind::OutcomeUnknown)
        }
        AppDataResetOldDatabaseStoreRetirementError::BeforeEffect(kind) => {
            let kind = match kind {
                DatabaseOpenErrorKind::Busy => AppDataResetCoordinatorErrorKind::Busy,
                DatabaseOpenErrorKind::StorageRootUnavailable
                | DatabaseOpenErrorKind::DatabaseUnavailable
                | DatabaseOpenErrorKind::InspectionLimitExceeded
                | DatabaseOpenErrorKind::MigrationFailed => {
                    AppDataResetCoordinatorErrorKind::Unavailable
                }
                DatabaseOpenErrorKind::InternalState => {
                    AppDataResetCoordinatorErrorKind::InternalState
                }
                DatabaseOpenErrorKind::UnsafeStorageRoot
                | DatabaseOpenErrorKind::UnsafeStorageObject
                | DatabaseOpenErrorKind::UnsafePermissions
                | DatabaseOpenErrorKind::OwnershipMismatch
                | DatabaseOpenErrorKind::UnrecognizedDatabase
                | DatabaseOpenErrorKind::CorruptDatabase => {
                    AppDataResetCoordinatorErrorKind::ChangedSinceRead
                }
            };
            error(kind)
        }
    }
}

fn map_fresh_origin_retirement_error(
    retirement_error: AppDataResetFreshOriginRetirementError,
) -> AppDataResetCoordinatorError {
    match retirement_error {
        AppDataResetFreshOriginRetirementError::OutcomeUnknown => {
            error(AppDataResetCoordinatorErrorKind::OutcomeUnknown)
        }
        AppDataResetFreshOriginRetirementError::BeforeEffect(kind) => {
            let kind = match kind {
                DatabaseOpenErrorKind::Busy => AppDataResetCoordinatorErrorKind::Busy,
                DatabaseOpenErrorKind::StorageRootUnavailable
                | DatabaseOpenErrorKind::DatabaseUnavailable
                | DatabaseOpenErrorKind::InspectionLimitExceeded
                | DatabaseOpenErrorKind::MigrationFailed => {
                    AppDataResetCoordinatorErrorKind::Unavailable
                }
                DatabaseOpenErrorKind::InternalState => {
                    AppDataResetCoordinatorErrorKind::InternalState
                }
                DatabaseOpenErrorKind::UnsafeStorageRoot
                | DatabaseOpenErrorKind::UnsafeStorageObject
                | DatabaseOpenErrorKind::UnsafePermissions
                | DatabaseOpenErrorKind::OwnershipMismatch
                | DatabaseOpenErrorKind::UnrecognizedDatabase
                | DatabaseOpenErrorKind::CorruptDatabase => {
                    AppDataResetCoordinatorErrorKind::ChangedSinceRead
                }
            };
            error(kind)
        }
    }
}

fn map_completed_store_envelope_error(
    open_error: super::status::DatabaseOpenError,
) -> AppDataResetCompletedEngineOpenError {
    let kind = match open_error.kind {
        DatabaseOpenErrorKind::Busy => AppDataResetCoordinatorErrorKind::Busy,
        DatabaseOpenErrorKind::StorageRootUnavailable
        | DatabaseOpenErrorKind::DatabaseUnavailable
        | DatabaseOpenErrorKind::InspectionLimitExceeded
        | DatabaseOpenErrorKind::MigrationFailed => AppDataResetCoordinatorErrorKind::Unavailable,
        DatabaseOpenErrorKind::InternalState => AppDataResetCoordinatorErrorKind::InternalState,
        DatabaseOpenErrorKind::UnsafeStorageRoot
        | DatabaseOpenErrorKind::UnsafeStorageObject
        | DatabaseOpenErrorKind::UnsafePermissions
        | DatabaseOpenErrorKind::OwnershipMismatch
        | DatabaseOpenErrorKind::UnrecognizedDatabase
        | DatabaseOpenErrorKind::CorruptDatabase => {
            AppDataResetCoordinatorErrorKind::ChangedSinceRead
        }
    };
    AppDataResetCompletedEngineOpenError::Reset(error(kind))
}

fn completed_reset_envelope_as_database_error(
    envelope_error: AppDataResetCoordinatorError,
) -> DatabaseOpenError {
    let kind = match envelope_error.kind() {
        AppDataResetCoordinatorErrorKind::Busy => DatabaseOpenErrorKind::Busy,
        AppDataResetCoordinatorErrorKind::Unavailable => {
            DatabaseOpenErrorKind::StorageRootUnavailable
        }
        AppDataResetCoordinatorErrorKind::InternalState => DatabaseOpenErrorKind::InternalState,
        AppDataResetCoordinatorErrorKind::InvalidConfiguration
        | AppDataResetCoordinatorErrorKind::UnsafeParent
        | AppDataResetCoordinatorErrorKind::UnsafeCoordinator
        | AppDataResetCoordinatorErrorKind::UnsafeObject
        | AppDataResetCoordinatorErrorKind::CorruptJournal
        | AppDataResetCoordinatorErrorKind::IncompatibleJournal
        | AppDataResetCoordinatorErrorKind::InvalidTransition
        | AppDataResetCoordinatorErrorKind::ChangedSinceRead
        | AppDataResetCoordinatorErrorKind::OutcomeUnknown => {
            DatabaseOpenErrorKind::UnsafeStorageRoot
        }
    };
    DatabaseOpenError::new(kind)
}

fn map_completed_store_admission_error(
    open_error: super::status::DatabaseOpenError,
) -> AppDataResetCompletedEngineOpenError {
    match open_error.kind {
        DatabaseOpenErrorKind::CorruptDatabase
        | DatabaseOpenErrorKind::MigrationFailed
        | DatabaseOpenErrorKind::DatabaseUnavailable => {
            AppDataResetCompletedEngineOpenError::Database(open_error.kind)
        }
        _ => map_completed_store_envelope_error(open_error),
    }
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
#[path = "app_data_reset/tests.rs"]
mod tests;
