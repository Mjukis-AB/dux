//! Dormant crash-safe coordination for a future whole-app-data reset.
//!
//! This module deliberately owns no reset targets and performs no namespace
//! detach or deletion. It only persists an exact, checksummed state machine in
//! an independently marker-owned sibling of the configured application-data
//! root. Engine, FFI, and native lifecycle integration are separate work.

mod storage;

use std::fmt;
use std::path::Path;
#[cfg(test)]
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use self::storage::ResetCoordinatorStorage;
use super::history::HistoryError;
use super::store::{AppDataResetStoreAdmission, StoreCoordinator};

const JOURNAL_FORMAT_VERSION: u16 = 1;
const DIGEST_DOMAIN: &[u8] = b"dux-app-data-reset-journal-v1\0";
const TRANSACTION_HEX_LENGTH: usize = 32;

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
}

impl AppDataResetJournal {
    pub(crate) fn prepared(
        transaction_id: &str,
        data_identity: AppDataResetStoreIdentity,
        cache_identity: Option<AppDataResetStoreIdentity>,
    ) -> Result<Self> {
        validate_transaction_id(transaction_id)?;
        Ok(Self {
            transaction_id: transaction_id.to_owned(),
            phase: AppDataResetPhase::Prepared,
            data_identity,
            cache_identity,
            data_stage_name: stage_name("data", transaction_id),
            cache_stage_name: cache_identity.map(|_| stage_name("cache", transaction_id)),
        })
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

    fn advanced(&self, phase: AppDataResetPhase) -> Result<Self> {
        if self.phase.successor() != Some(phase) {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        let mut advanced = self.clone();
        advanced.phase = phase;
        Ok(advanced)
    }

    fn validate(&self) -> Result<()> {
        validate_transaction_id(&self.transaction_id)?;
        if self.data_identity.device == 0 || self.data_identity.inode == 0 {
            return Err(corrupt());
        }
        if self
            .cache_identity
            .is_some_and(|identity| identity.device == 0 || identity.inode == 0)
            || self.data_stage_name != stage_name("data", &self.transaction_id)
            || self.cache_stage_name
                != self
                    .cache_identity
                    .map(|_| stage_name("cache", &self.transaction_id))
        {
            return Err(corrupt());
        }
        Ok(())
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
struct JournalEnvelope {
    format_version: u16,
    payload: JournalPayloadV1,
    digest_sha256: String,
}

impl From<&AppDataResetJournal> for JournalPayloadV1 {
    fn from(journal: &AppDataResetJournal) -> Self {
        Self {
            transaction_id: journal.transaction_id.clone(),
            phase: journal.phase,
            data_identity: journal.data_identity,
            cache_identity: journal.cache_identity,
            data_stage_name: journal.data_stage_name.clone(),
            cache_stage_name: journal.cache_stage_name.clone(),
        }
    }
}

impl From<JournalPayloadV1> for AppDataResetJournal {
    fn from(payload: JournalPayloadV1) -> Self {
        Self {
            transaction_id: payload.transaction_id,
            phase: payload.phase,
            data_identity: payload.data_identity,
            cache_identity: payload.cache_identity,
            data_stage_name: payload.data_stage_name,
            cache_stage_name: payload.cache_stage_name,
        }
    }
}

/// Independently marker-owned durable singleton coordinator.
pub(crate) struct AppDataResetCoordinator {
    storage: ResetCoordinatorStorage,
}

/// One callback-scoped owner of the retained reset-coordinator writer lock.
///
/// The session exposes only typed journal operations. It cannot escape the
/// callback that owns the lock, disclose storage paths, or acquire a second
/// coordinator lock.
pub(crate) struct AppDataResetCoordinatorSession<'a> {
    storage: &'a ResetCoordinatorStorage,
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
    /// Open or provision the fixed sibling coordinator for one data root.
    ///
    /// Provisioning mutates only the fixed private coordinator namespace. It
    /// never opens, renames, or removes the supplied data root.
    pub(crate) fn open_or_create(data_root: &Path) -> Result<Self> {
        Ok(Self {
            storage: ResetCoordinatorStorage::open_or_create(data_root)?,
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

    #[cfg(test)]
    fn with_exclusive_session_with_timeout<T>(
        &self,
        timeout: Duration,
        operation: impl FnOnce(&mut AppDataResetCoordinatorSession<'_>) -> Result<T>,
    ) -> Result<T> {
        self.storage.with_lock_timeout(timeout, |storage| {
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
}

impl AppDataResetCoordinatorSession<'_> {
    /// Acquire database-side reset admission strictly inside this retained
    /// coordinator session.
    ///
    /// The higher-ranked callback prevents an admitted store guard from
    /// escaping this call. Consequently the coordinator lock always outlives
    /// cleanup and database exclusion, and callers cannot invert that order.
    pub(crate) fn with_store_admission<T>(
        &mut self,
        store: &StoreCoordinator,
        operation: impl for<'guard> FnOnce(AppDataResetStoreAdmission<'guard>) -> T,
    ) -> std::result::Result<T, HistoryError> {
        let admission = store.begin_app_data_reset_store_admission()?;
        Ok(operation(admission))
    }

    /// Return the exact current journal without releasing the retained lock.
    pub(crate) fn recover(&mut self) -> Result<Option<AppDataResetJournal>> {
        let Some(bytes) = self.storage.read_journal()? else {
            return Ok(None);
        };
        decode_journal(&bytes).map(Some)
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
        if prepared.phase != AppDataResetPhase::Prepared {
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

    /// Exact compare-and-advance while retaining the same exclusive lock.
    pub(crate) fn advance(
        &mut self,
        expected: &AppDataResetJournal,
        next_phase: AppDataResetPhase,
    ) -> Result<AppDataResetJournal> {
        expected.validate()?;
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
}

fn validate_transaction_id(value: &str) -> Result<()> {
    if value.len() != TRANSACTION_HEX_LENGTH
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(corrupt());
    }
    Ok(())
}

fn stage_name(kind: &str, transaction_id: &str) -> String {
    format!(".dux-reset-{kind}-{transaction_id}")
}

fn encode_journal(journal: &AppDataResetJournal) -> Result<Vec<u8>> {
    journal.validate()?;
    let payload = JournalPayloadV1::from(journal);
    let payload_bytes = serde_json::to_vec(&payload).map_err(|_| internal())?;
    let digest_sha256 = journal_digest(&payload_bytes);
    serde_json::to_vec(&JournalEnvelope {
        format_version: JOURNAL_FORMAT_VERSION,
        payload,
        digest_sha256,
    })
    .map_err(|_| internal())
}

fn decode_journal(bytes: &[u8]) -> Result<AppDataResetJournal> {
    let envelope: JournalEnvelope = serde_json::from_slice(bytes).map_err(|_| corrupt())?;
    if envelope.format_version > JOURNAL_FORMAT_VERSION {
        return Err(error(AppDataResetCoordinatorErrorKind::IncompatibleJournal));
    }
    if envelope.format_version != JOURNAL_FORMAT_VERSION {
        return Err(corrupt());
    }
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
        let journal = AppDataResetJournal::prepared(FIRST_TRANSACTION, data, Some(cache)).unwrap();
        let encoded = encode_journal(&journal).unwrap();

        assert_eq!(decode_journal(&encoded).unwrap(), journal);
        assert_eq!(journal.transaction_id(), FIRST_TRANSACTION);
        assert_eq!(journal.data_identity().device(), 11);
        assert_eq!(journal.data_identity().inode(), 22);
        assert_eq!(journal.cache_identity(), Some(cache));
        assert_eq!(
            journal.data_stage_name(),
            ".dux-reset-data-00112233445566778899aabbccddeeff"
        );
        assert_eq!(
            journal.cache_stage_name(),
            Some(".dux-reset-cache-00112233445566778899aabbccddeeff")
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
    fn journal_rejects_noncanonical_unknown_and_newer_shapes() {
        let (data, _) = identities();
        let journal = AppDataResetJournal::prepared(FIRST_TRANSACTION, data, None).unwrap();
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
        let prepared = AppDataResetJournal::prepared(FIRST_TRANSACTION, data, Some(cache)).unwrap();

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
            .advance(&data_detached, AppDataResetPhase::FreshNamespaceReady)
            .unwrap();
        let draining = coordinator
            .advance(&fresh, AppDataResetPhase::Draining)
            .unwrap();
        let complete = coordinator
            .advance(&draining, AppDataResetPhase::Complete)
            .unwrap();
        assert_eq!(complete.phase(), AppDataResetPhase::Complete);

        let replacement = AppDataResetJournal::prepared(SECOND_TRANSACTION, data, None).unwrap();
        coordinator.begin(&replacement).unwrap();
        assert_eq!(coordinator.recover().unwrap(), Some(replacement));
    }

    #[test]
    fn incomplete_singleton_cannot_be_replaced() {
        let temp = TempDir::new().unwrap();
        let coordinator = coordinator(&temp);
        let (data, cache) = identities();
        let first = AppDataResetJournal::prepared(FIRST_TRANSACTION, data, Some(cache)).unwrap();
        let second = AppDataResetJournal::prepared(SECOND_TRANSACTION, data, None).unwrap();
        coordinator.begin(&first).unwrap();

        assert_eq!(
            coordinator.begin(&second).unwrap_err().kind(),
            AppDataResetCoordinatorErrorKind::InvalidTransition
        );
        assert_eq!(coordinator.recover().unwrap(), Some(first));
    }

    #[test]
    fn retained_session_sequences_journal_transitions_without_relocking() {
        let temp = TempDir::new().unwrap();
        let coordinator = coordinator(&temp);
        let (data, cache) = identities();
        let prepared = AppDataResetJournal::prepared(FIRST_TRANSACTION, data, Some(cache)).unwrap();

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
        let prepared = AppDataResetJournal::prepared(FIRST_TRANSACTION, data, None).unwrap();

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
        let prepared = AppDataResetJournal::prepared(FIRST_TRANSACTION, data, None).unwrap();

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
                    .with_store_admission(&store, |admission| {
                        let AppDataResetStoreAdmission::Admitted(guard) = admission else {
                            panic!("empty current store unexpectedly blocked reset admission");
                        };
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
        let prepared = AppDataResetJournal::prepared(FIRST_TRANSACTION, data, None).unwrap();

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
        let prepared = AppDataResetJournal::prepared(FIRST_TRANSACTION, data, None).unwrap();

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
