//! Dormant crash-safe coordination for a future whole-app-data reset.
//!
//! This module deliberately owns no reset targets and performs no namespace
//! detach or deletion. It only persists an exact, checksummed state machine in
//! an independently marker-owned sibling of the configured application-data
//! root. Engine, FFI, and native lifecycle integration are separate work.

mod storage;

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use self::storage::ResetCoordinatorStorage;

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

    /// Return the exact current journal, if one exists.
    pub(crate) fn recover(&self) -> Result<Option<AppDataResetJournal>> {
        self.storage.with_lock(|storage| {
            let Some(bytes) = storage.read_journal()? else {
                return Ok(None);
            };
            decode_journal(&bytes).map(Some)
        })
    }

    pub(crate) fn provisioning_debt(&self) -> Result<AppDataResetProvisioningDebt> {
        self.storage
            .with_lock(|storage| storage.reconcile_provisioning_stages())
            .map(|unproven_stage_count| AppDataResetProvisioningDebt {
                unproven_stage_count,
            })
    }

    /// Commit the first durable reset intent or replace one completed intent.
    ///
    /// An incomplete transaction can only move through `advance`.
    pub(crate) fn begin(&self, prepared: &AppDataResetJournal) -> Result<()> {
        prepared.validate()?;
        if prepared.phase != AppDataResetPhase::Prepared {
            return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
        }
        self.storage.with_lock(|storage| {
            if let Some(current) = storage.read_journal()?.map(|bytes| decode_journal(&bytes))
                && current?.phase != AppDataResetPhase::Complete
            {
                return Err(error(AppDataResetCoordinatorErrorKind::InvalidTransition));
            }
            storage.write_journal(&encode_journal(prepared)?)
        })
    }

    /// Exact compare-and-advance of one already durable transaction.
    pub(crate) fn advance(
        &self,
        expected: &AppDataResetJournal,
        next_phase: AppDataResetPhase,
    ) -> Result<AppDataResetJournal> {
        expected.validate()?;
        let next = expected.advanced(next_phase)?;
        self.storage.with_lock(|storage| {
            let current = storage
                .read_journal()?
                .ok_or_else(|| error(AppDataResetCoordinatorErrorKind::ChangedSinceRead))
                .and_then(|bytes| decode_journal(&bytes))?;
            if current != *expected {
                return Err(error(AppDataResetCoordinatorErrorKind::ChangedSinceRead));
            }
            storage.write_journal(&encode_journal(&next)?)?;
            Ok(next)
        })
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
