//! Cross-process exclusion for active canonical scan roots.
//!
//! A lease is an observation-work exclusion only. It grants no snapshot,
//! planner, cleanup, recovery, or filesystem-effect authority.

use std::path::{Component, Path};
use std::time::SystemTime;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    system_time_to_unix_ms,
};
use super::process_liveness::{
    ExecutionProvenance, ProcessExecutionIdentity, ProcessInstanceId, ProcessLiveness,
    ProvenanceRelationship, compare_execution_provenance, probe_process_instance,
};
use super::storage::StoreIdentity;

pub(super) const MAX_SCAN_SCOPE_LEASES: usize = 64;
const SCAN_SCOPE_RECOVERY_POLICY: &str = "release_only";
const LEASE_ID_BYTES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanScopeLeaseErrorKind {
    InvalidRoot,
    Busy,
    IncompatibleSchema,
    QueryLimitExceeded,
    UnsafeStorage,
    CorruptData,
    Unavailable,
    InternalState,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("scan scope lease operation failed: {kind:?}")]
pub(crate) struct ScanScopeLeaseError {
    pub(crate) kind: ScanScopeLeaseErrorKind,
}

impl ScanScopeLeaseError {
    pub(super) const fn new(kind: ScanScopeLeaseErrorKind) -> Self {
        Self { kind }
    }
}

/// Move-only proof that this process inserted one exact durable exclusion row.
///
/// The token deliberately has no destructor. Engine task cancellation and
/// terminal paths must explicitly reconcile release.
#[derive(Debug)]
pub(crate) struct ScanScopeLeaseToken {
    store_identity: StoreIdentity,
    lease_id: [u8; LEASE_ID_BYTES],
    root: EncodedBytes,
    owner: ProcessExecutionIdentity,
    acquired_at_unix_ms: i64,
}

impl ScanScopeLeaseToken {
    pub(super) fn new(
        store_identity: StoreIdentity,
        lease_id: [u8; LEASE_ID_BYTES],
        root: EncodedBytes,
        owner: ProcessExecutionIdentity,
        acquired_at_unix_ms: i64,
    ) -> Self {
        Self {
            store_identity,
            lease_id,
            root,
            owner,
            acquired_at_unix_ms,
        }
    }
}

#[derive(Clone)]
struct StoredLease {
    lease_id: [u8; LEASE_ID_BYTES],
    root: EncodedBytes,
    owner: ProcessInstanceId,
    recovery_scope: Option<String>,
    acquired_at_unix_ms: i64,
    provenance: Option<ExecutionProvenance>,
}

pub(super) fn prepare_canonical_root(root: &Path) -> Result<EncodedBytes, ScanScopeLeaseError> {
    if !root.is_absolute()
        || root
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(ScanScopeLeaseError::new(
            ScanScopeLeaseErrorKind::InvalidRoot,
        ));
    }
    let observed = std::fs::canonicalize(root)
        .map_err(|_| ScanScopeLeaseError::new(ScanScopeLeaseErrorKind::InvalidRoot))?;
    if observed != root {
        return Err(ScanScopeLeaseError::new(
            ScanScopeLeaseErrorKind::InvalidRoot,
        ));
    }
    encode_host_path(root)
        .map_err(|_| ScanScopeLeaseError::new(ScanScopeLeaseErrorKind::InvalidRoot))
}

pub(super) fn new_lease_id() -> Result<[u8; LEASE_ID_BYTES], ScanScopeLeaseError> {
    let mut lease_id = [0_u8; LEASE_ID_BYTES];
    getrandom::fill(&mut lease_id)
        .map_err(|_| ScanScopeLeaseError::new(ScanScopeLeaseErrorKind::Unavailable))?;
    Ok(lease_id)
}

fn provenance_parameters(
    provenance: Option<&ExecutionProvenance>,
) -> (Option<&[u8]>, Option<&[u8]>, Option<&'static str>) {
    match provenance {
        Some(provenance) => (
            Some(provenance.stable_host()),
            Some(provenance.boot_scope()),
            Some(SCAN_SCOPE_RECOVERY_POLICY),
        ),
        None => (None, None, None),
    }
}

fn decode_provenance(
    owner: &ProcessInstanceId,
    host: (&str, Option<i64>, Option<&[u8]>),
    boot: (&str, Option<i64>, Option<&[u8]>),
    policy: (&str, Option<i64>, Option<&str>),
) -> Result<Option<ExecutionProvenance>, HistoryError> {
    let null = |storage: &str, length: Option<i64>| storage == "null" && length.is_none();
    match (host.2, boot.2, policy.2) {
        (None, None, None)
            if null(host.0, host.1) && null(boot.0, boot.1) && null(policy.0, policy.1) =>
        {
            Ok(None)
        }
        (Some(host_bytes), Some(boot_bytes), Some(SCAN_SCOPE_RECOVERY_POLICY))
            if host.0 == "blob"
                && host.1 == Some(32)
                && host_bytes.len() == 32
                && boot.0 == "blob"
                && boot.1 == Some(32)
                && boot_bytes.len() == 32
                && policy.0 == "text"
                && policy.1 == i64::try_from(SCAN_SCOPE_RECOVERY_POLICY.len()).ok() =>
        {
            ExecutionProvenance::from_stored(owner, host_bytes, boot_bytes)
                .map(Some)
                .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))
        }
        _ => Err(HistoryError::new(HistoryErrorKind::CorruptData)),
    }
}

fn load_leases(connection: &Connection) -> Result<Vec<StoredLease>, HistoryError> {
    run_bounded_query(connection, || {
        let row_limit = i64::try_from(MAX_SCAN_SCOPE_LEASES + 1)
            .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?;
        let mut statement = connection
            .prepare(
                "SELECT typeof(lease_id), length(lease_id), lease_id,
                        record_format_version,
                        typeof(root_path), length(root_path), root_path,
                        root_path_encoding,
                        typeof(owner_process_instance),
                        length(CAST(owner_process_instance AS BLOB)),
                        owner_process_instance,
                        typeof(recovery_scope),
                        length(CAST(recovery_scope AS BLOB)), recovery_scope,
                        acquired_at_unix_ms,
                        typeof(execution_host_identity_v1_sha256),
                        length(execution_host_identity_v1_sha256),
                        execution_host_identity_v1_sha256,
                        typeof(execution_boot_scope_v1_sha256),
                        length(execution_boot_scope_v1_sha256),
                        execution_boot_scope_v1_sha256,
                        typeof(execution_recovery_policy),
                        length(CAST(execution_recovery_policy AS BLOB)),
                        execution_recovery_policy
                 FROM scan_scope_leases ORDER BY lease_id LIMIT ?1",
            )
            .map_err(map_query_sql_error)?;
        let mut rows = statement.query([row_limit]).map_err(map_query_sql_error)?;
        let mut leases = Vec::new();
        while let Some(row) = rows.next().map_err(map_query_sql_error)? {
            if leases.len() == MAX_SCAN_SCOPE_LEASES {
                return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
            }
            let lease_storage: String = row.get(0).map_err(map_query_sql_error)?;
            let lease_length: i64 = row.get(1).map_err(map_query_sql_error)?;
            let lease: Vec<u8> = row.get(2).map_err(map_query_sql_error)?;
            let format: i64 = row.get(3).map_err(map_query_sql_error)?;
            let root_storage: String = row.get(4).map_err(map_query_sql_error)?;
            let root_length: i64 = row.get(5).map_err(map_query_sql_error)?;
            let root_bytes: Vec<u8> = row.get(6).map_err(map_query_sql_error)?;
            let root_encoding: i64 = row.get(7).map_err(map_query_sql_error)?;
            let owner_storage: String = row.get(8).map_err(map_query_sql_error)?;
            let owner_length: i64 = row.get(9).map_err(map_query_sql_error)?;
            let owner_text: String = row.get(10).map_err(map_query_sql_error)?;
            let scope_storage: String = row.get(11).map_err(map_query_sql_error)?;
            let scope_length: Option<i64> = row.get(12).map_err(map_query_sql_error)?;
            let recovery_scope: Option<String> = row.get(13).map_err(map_query_sql_error)?;
            let acquired_at_unix_ms: i64 = row.get(14).map_err(map_query_sql_error)?;
            let host_storage: String = row.get(15).map_err(map_query_sql_error)?;
            let host_length: Option<i64> = row.get(16).map_err(map_query_sql_error)?;
            let host: Option<Vec<u8>> = row.get(17).map_err(map_query_sql_error)?;
            let boot_storage: String = row.get(18).map_err(map_query_sql_error)?;
            let boot_length: Option<i64> = row.get(19).map_err(map_query_sql_error)?;
            let boot: Option<Vec<u8>> = row.get(20).map_err(map_query_sql_error)?;
            let policy_storage: String = row.get(21).map_err(map_query_sql_error)?;
            let policy_length: Option<i64> = row.get(22).map_err(map_query_sql_error)?;
            let policy: Option<String> = row.get(23).map_err(map_query_sql_error)?;

            if lease_storage != "blob"
                || lease_length != LEASE_ID_BYTES as i64
                || lease.len() != LEASE_ID_BYTES
                || format != 1
                || root_storage != "blob"
                || root_length != i64::try_from(root_bytes.len()).unwrap_or(-1)
                || owner_storage != "text"
                || !(1..=128).contains(&owner_length)
                || owner_text.len() != owner_length as usize
                || acquired_at_unix_ms < 0
            {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            let encoding = match root_encoding {
                1 => StoredEncoding::Utf8HostPath,
                2 => StoredEncoding::Utf16LeHostPath,
                _ => return Err(HistoryError::new(HistoryErrorKind::CorruptData)),
            };
            let root = EncodedBytes {
                encoding,
                bytes: root_bytes,
            };
            let decoded_root = decode_host_path(&root)
                .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
            if !is_canonical_stored_root(&decoded_root) {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            let owner = ProcessInstanceId::from_stored(&owner_text)
                .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
            let expected_scope = owner.recovery_scope_key();
            let valid_scope = match &recovery_scope {
                None => scope_storage == "null" && scope_length.is_none(),
                Some(scope) => {
                    scope_storage == "text"
                        && scope_length == i64::try_from(scope.len()).ok()
                        && scope.len() == 66
                }
            };
            if !valid_scope || recovery_scope != expected_scope {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            let provenance = decode_provenance(
                &owner,
                (&host_storage, host_length, host.as_deref()),
                (&boot_storage, boot_length, boot.as_deref()),
                (&policy_storage, policy_length, policy.as_deref()),
            )?;
            leases.push(StoredLease {
                lease_id: lease
                    .try_into()
                    .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?,
                root,
                owner,
                recovery_scope,
                acquired_at_unix_ms,
                provenance,
            });
        }
        Ok(leases)
    })
}

fn is_canonical_stored_root(root: &Path) -> bool {
    root.is_absolute()
        && !root
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
}

fn roots_overlap(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        return windows_path_is_prefix(left, right) || windows_path_is_prefix(right, left);
    }
    #[cfg(not(windows))]
    {
        left == right || left.starts_with(right) || right.starts_with(left)
    }
}

#[cfg(windows)]
fn windows_path_is_prefix(prefix: &Path, path: &Path) -> bool {
    let prefix_components: Vec<_> = prefix.components().collect();
    let path_components: Vec<_> = path.components().collect();
    prefix_components.len() <= path_components.len()
        && prefix_components
            .iter()
            .zip(path_components.iter())
            .all(|(left, right)| windows_components_equal(*left, *right))
}

#[cfg(windows)]
fn windows_components_equal(left: Component<'_>, right: Component<'_>) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};

    let (left, right) = match (left, right) {
        (Component::Prefix(left), Component::Prefix(right)) => {
            (left.as_os_str(), right.as_os_str())
        }
        (Component::RootDir, Component::RootDir) => return true,
        (Component::Normal(left), Component::Normal(right)) => (left, right),
        _ => return false,
    };
    let left: Vec<u16> = left.encode_wide().collect();
    let right: Vec<u16> = right.encode_wide().collect();
    let Ok(left_len) = i32::try_from(left.len()) else {
        return false;
    };
    let Ok(right_len) = i32::try_from(right.len()) else {
        return false;
    };
    // SAFETY: the UTF-16 buffers remain live for the call and both explicit
    // lengths fit the API. Ordinal ignore-case is lossless and locale-free.
    unsafe {
        CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) == CSTR_EQUAL
    }
}

fn can_recover(lease: &StoredLease, current: &ProcessExecutionIdentity) -> bool {
    match compare_execution_provenance(lease.provenance.as_ref(), current.provenance.as_ref()) {
        ProvenanceRelationship::PriorBoot => true,
        ProvenanceRelationship::ForeignHost => false,
        ProvenanceRelationship::SameBoot | ProvenanceRelationship::Unproven
            if lease.recovery_scope.is_some()
                && lease.recovery_scope == current.owner.recovery_scope_key() =>
        {
            probe_process_instance(&lease.owner) == ProcessLiveness::DefinitelyGone
        }
        ProvenanceRelationship::SameBoot | ProvenanceRelationship::Unproven => false,
    }
}

pub(super) fn acquire(
    transaction: &Transaction<'_>,
    store_identity: StoreIdentity,
    root: EncodedBytes,
    owner: ProcessExecutionIdentity,
    lease_id: [u8; LEASE_ID_BYTES],
    acquired_at: SystemTime,
) -> Result<ScanScopeLeaseToken, ScanScopeLeaseError> {
    let acquired_at_unix_ms = system_time_to_unix_ms(acquired_at, HistoryErrorKind::InvalidInput)
        .map_err(map_history_error)?;
    let requested = decode_host_path(&root)
        .map_err(|_| ScanScopeLeaseError::new(ScanScopeLeaseErrorKind::InvalidRoot))?;
    let current = owner.clone();
    let mut retained_lease_count = 0_usize;
    for lease in load_leases(transaction).map_err(map_history_error)? {
        if can_recover(&lease, &current) {
            delete_exact(transaction, &lease).map_err(map_history_error)?;
            continue;
        }
        retained_lease_count += 1;
        let stored = decode_host_path(&lease.root)
            .map_err(|_| ScanScopeLeaseError::new(ScanScopeLeaseErrorKind::CorruptData))?;
        if roots_overlap(&requested, &stored) {
            return Err(ScanScopeLeaseError::new(ScanScopeLeaseErrorKind::Busy));
        }
    }
    if retained_lease_count == MAX_SCAN_SCOPE_LEASES {
        return Err(ScanScopeLeaseError::new(
            ScanScopeLeaseErrorKind::QueryLimitExceeded,
        ));
    }
    if transitional_scan_scope_conflicts(transaction, &requested).map_err(map_history_error)? {
        return Err(ScanScopeLeaseError::new(ScanScopeLeaseErrorKind::Busy));
    }
    let (host, boot, policy) = provenance_parameters(owner.provenance.as_ref());
    let changed = transaction
        .execute(
            "INSERT INTO scan_scope_leases (
                 lease_id, record_format_version, root_path, root_path_encoding,
                 owner_process_instance, recovery_scope, acquired_at_unix_ms,
                 execution_host_identity_v1_sha256,
                 execution_boot_scope_v1_sha256, execution_recovery_policy
             ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                lease_id.as_slice(),
                root.bytes.as_slice(),
                root.encoding as i64,
                owner.owner.as_str(),
                owner.owner.recovery_scope_key(),
                acquired_at_unix_ms,
                host,
                boot,
                policy,
            ],
        )
        .map_err(map_write_sql_error)
        .map_err(map_history_error)?;
    if changed != 1 {
        return Err(ScanScopeLeaseError::new(
            ScanScopeLeaseErrorKind::InternalState,
        ));
    }
    Ok(ScanScopeLeaseToken::new(
        store_identity,
        lease_id,
        root,
        owner,
        acquired_at_unix_ms,
    ))
}

fn transitional_scan_scope_conflicts(
    connection: &Connection,
    requested: &Path,
) -> Result<bool, HistoryError> {
    run_bounded_query(connection, || {
        let row_limit = i64::try_from(MAX_SCAN_SCOPE_LEASES + 1)
            .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?;
        let mut statement = connection
            .prepare(
                "SELECT typeof(scans.root_path), length(scans.root_path),
                        scans.root_path, scans.root_path_encoding, scans.status
                 FROM scans
                 WHERE scans.status = 'running'
                 ORDER BY scans.scan_id LIMIT ?1",
            )
            .map_err(map_query_sql_error)?;
        let mut rows = statement.query([row_limit]).map_err(map_query_sql_error)?;
        let mut count = 0_usize;
        while let Some(row) = rows.next().map_err(map_query_sql_error)? {
            if count == MAX_SCAN_SCOPE_LEASES {
                return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
            }
            count += 1;
            let storage: String = row.get(0).map_err(map_query_sql_error)?;
            let length: i64 = row.get(1).map_err(map_query_sql_error)?;
            let bytes: Vec<u8> = row.get(2).map_err(map_query_sql_error)?;
            let encoding: i64 = row.get(3).map_err(map_query_sql_error)?;
            let status: String = row.get(4).map_err(map_query_sql_error)?;
            if storage != "blob"
                || length != i64::try_from(bytes.len()).unwrap_or(-1)
                || status != "running"
            {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            let encoding = match encoding {
                1 => StoredEncoding::Utf8HostPath,
                2 => StoredEncoding::Utf16LeHostPath,
                _ => return Err(HistoryError::new(HistoryErrorKind::CorruptData)),
            };
            let root = decode_host_path(&EncodedBytes { encoding, bytes })
                .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
            if !is_canonical_stored_root(&root) {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            if roots_overlap(requested, &root) {
                return Ok(true);
            }
        }
        Ok(false)
    })
}

fn delete_exact(transaction: &Transaction<'_>, lease: &StoredLease) -> Result<(), HistoryError> {
    let (host, boot, policy) = provenance_parameters(lease.provenance.as_ref());
    let changed = transaction
        .execute(
            "DELETE FROM scan_scope_leases
             WHERE lease_id = ?1 AND record_format_version = 1
               AND root_path = ?2 AND root_path_encoding = ?3
               AND owner_process_instance = ?4 AND recovery_scope IS ?5
               AND acquired_at_unix_ms = ?6
               AND execution_host_identity_v1_sha256 IS ?7
               AND execution_boot_scope_v1_sha256 IS ?8
               AND execution_recovery_policy IS ?9",
            params![
                lease.lease_id.as_slice(),
                lease.root.bytes.as_slice(),
                lease.root.encoding as i64,
                lease.owner.as_str(),
                lease.recovery_scope,
                lease.acquired_at_unix_ms,
                host,
                boot,
                policy,
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
    }
}

pub(super) fn release(
    transaction: &Transaction<'_>,
    store_identity: StoreIdentity,
    token: &ScanScopeLeaseToken,
) -> Result<(), ScanScopeLeaseError> {
    if token.store_identity != store_identity {
        return Err(ScanScopeLeaseError::new(
            ScanScopeLeaseErrorKind::InternalState,
        ));
    }
    let (host, boot, policy) = provenance_parameters(token.owner.provenance.as_ref());
    let changed = transaction
        .execute(
            "DELETE FROM scan_scope_leases
             WHERE lease_id = ?1 AND record_format_version = 1
               AND root_path = ?2 AND root_path_encoding = ?3
               AND owner_process_instance = ?4 AND recovery_scope IS ?5
               AND acquired_at_unix_ms = ?6
               AND execution_host_identity_v1_sha256 IS ?7
               AND execution_boot_scope_v1_sha256 IS ?8
               AND execution_recovery_policy IS ?9",
            params![
                token.lease_id.as_slice(),
                token.root.bytes.as_slice(),
                token.root.encoding as i64,
                token.owner.owner.as_str(),
                token.owner.owner.recovery_scope_key(),
                token.acquired_at_unix_ms,
                host,
                boot,
                policy,
            ],
        )
        .map_err(map_write_sql_error)
        .map_err(map_history_error)?;
    if changed == 1 {
        return Ok(());
    }
    let exists: Option<i64> = transaction
        .query_row(
            "SELECT 1 FROM scan_scope_leases WHERE lease_id = ?1",
            [token.lease_id.as_slice()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)
        .map_err(map_history_error)?;
    if exists.is_none() {
        Ok(())
    } else {
        Err(ScanScopeLeaseError::new(
            ScanScopeLeaseErrorKind::CorruptData,
        ))
    }
}

pub(super) fn exact_token_exists(
    connection: &Connection,
    token: &ScanScopeLeaseToken,
) -> Result<bool, ScanScopeLeaseError> {
    let (host, boot, policy) = provenance_parameters(token.owner.provenance.as_ref());
    connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM scan_scope_leases
                 WHERE lease_id = ?1 AND record_format_version = 1
                   AND root_path = ?2 AND root_path_encoding = ?3
                   AND owner_process_instance = ?4 AND recovery_scope IS ?5
                   AND acquired_at_unix_ms = ?6
                   AND execution_host_identity_v1_sha256 IS ?7
                   AND execution_boot_scope_v1_sha256 IS ?8
                   AND execution_recovery_policy IS ?9
             )",
            params![
                token.lease_id.as_slice(),
                token.root.bytes.as_slice(),
                token.root.encoding as i64,
                token.owner.owner.as_str(),
                token.owner.owner.recovery_scope_key(),
                token.acquired_at_unix_ms,
                host,
                boot,
                policy,
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value == 1)
        .map_err(map_query_sql_error)
        .map_err(map_history_error)
}

pub(super) fn map_history_error(error: HistoryError) -> ScanScopeLeaseError {
    let kind = match error.kind {
        HistoryErrorKind::InvalidInput => ScanScopeLeaseErrorKind::InvalidRoot,
        HistoryErrorKind::AlreadyExists | HistoryErrorKind::Busy => ScanScopeLeaseErrorKind::Busy,
        HistoryErrorKind::IncompatibleSchema => ScanScopeLeaseErrorKind::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => ScanScopeLeaseErrorKind::QueryLimitExceeded,
        HistoryErrorKind::UnsafeStorage => ScanScopeLeaseErrorKind::UnsafeStorage,
        HistoryErrorKind::CorruptData | HistoryErrorKind::InvalidTransition => {
            ScanScopeLeaseErrorKind::CorruptData
        }
        HistoryErrorKind::DatabaseUnavailable | HistoryErrorKind::NotFound => {
            ScanScopeLeaseErrorKind::Unavailable
        }
        HistoryErrorKind::InternalState => ScanScopeLeaseErrorKind::InternalState,
        HistoryErrorKind::OutcomeUnknown => ScanScopeLeaseErrorKind::OutcomeUnknown,
    };
    ScanScopeLeaseError::new(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_overlap_is_component_aware() {
        assert!(roots_overlap(Path::new("/a"), Path::new("/a/b")));
        assert!(roots_overlap(Path::new("/a/b"), Path::new("/a")));
        assert!(!roots_overlap(Path::new("/a"), Path::new("/ab")));
        assert!(!roots_overlap(Path::new("/a/b"), Path::new("/a/c")));
    }

    #[test]
    fn recovery_requires_prior_boot_or_definitely_gone_same_boot_evidence() {
        let stored_owner = ProcessInstanceId::from_stored(&format!(
            "1:l:2a:1234:{}:{}",
            "11".repeat(32),
            "22".repeat(16)
        ))
        .unwrap();
        let current_owner = ProcessInstanceId::from_stored(&format!(
            "1:l:2b:5678:{}:{}",
            "33".repeat(32),
            "44".repeat(16)
        ))
        .unwrap();
        let stored_provenance =
            ExecutionProvenance::from_stored(&stored_owner, &[0x55; 32], &[0x11; 32]).unwrap();
        let current_provenance =
            ExecutionProvenance::from_stored(&current_owner, &[0x55; 32], &[0x33; 32]).unwrap();
        let lease = StoredLease {
            lease_id: [1; LEASE_ID_BYTES],
            root: EncodedBytes {
                encoding: StoredEncoding::Utf8HostPath,
                bytes: b"/scope".to_vec(),
            },
            owner: stored_owner,
            recovery_scope: Some(format!("l:{}", "11".repeat(32))),
            acquired_at_unix_ms: 1,
            provenance: Some(stored_provenance),
        };
        let prior_boot = ProcessExecutionIdentity {
            owner: current_owner,
            provenance: Some(current_provenance),
        };
        assert!(can_recover(&lease, &prior_boot));

        let mut unproven = lease;
        unproven.provenance = None;
        assert!(!can_recover(&unproven, &prior_boot));
    }
}
