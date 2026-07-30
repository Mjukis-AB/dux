//! Bounded process ownership for durable running scans.
//!
//! Claims are recovery evidence only. They grant no scan, snapshot, planner,
//! or cleanup authority. Recovery exact-CASes at most one pristine row after
//! either a same-boot liveness probe proves its owner gone or complete
//! provenance proves it belongs to the same host and an earlier boot.

use std::time::SystemTime;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::domain::ScanId;

use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::process_liveness::{
    ExecutionProvenance, ProcessExecutionIdentity, ProcessInstanceId, ProcessLiveness,
    ProvenanceRelationship, compare_execution_provenance,
};

pub(super) const MAX_SCAN_PROCESS_CLAIMS: usize = 64;
const SCAN_RECOVERY_POLICY: &str = "interrupt_only";

pub(super) struct ScanProcessClaimPage {
    pub(super) claims: Vec<ScanProcessClaim>,
    pub(super) has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanRecoveryBatchOutcome {
    NoClaim,
    DeferredUnproven,
    Interrupted,
    ChangedConcurrently,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScanRecoveryBatchResult {
    pub(crate) observed_at: SystemTime,
    pub(crate) outcome: ScanRecoveryBatchOutcome,
    pub(crate) claimed_count_before: u32,
    pub(crate) claimed_count_after: u32,
    pub(crate) alive_count: u32,
    pub(crate) unknown_count: u32,
    pub(crate) recoverable_count: u32,
    pub(crate) has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ScanProcessClaim {
    scan_id: ScanId,
    owner: ProcessInstanceId,
    recovery_scope: Option<String>,
    claimed_at_unix_ms: i64,
    provenance: Option<ExecutionProvenance>,
}

impl ScanProcessClaim {
    pub(super) fn owner(&self) -> &ProcessInstanceId {
        &self.owner
    }

    pub(super) fn cursor(&self) -> (i64, &str) {
        (self.claimed_at_unix_ms, self.scan_id.as_str())
    }

    fn provenance_parameters(&self) -> (Option<&[u8]>, Option<&[u8]>, Option<&str>) {
        provenance_parameters(self.provenance.as_ref())
    }
}

fn provenance_parameters(
    provenance: Option<&ExecutionProvenance>,
) -> (Option<&[u8]>, Option<&[u8]>, Option<&'static str>) {
    match provenance {
        Some(provenance) => (
            Some(provenance.stable_host()),
            Some(provenance.boot_scope()),
            Some(SCAN_RECOVERY_POLICY),
        ),
        None => (None, None, None),
    }
}

fn decode_execution_provenance(
    owner: &ProcessInstanceId,
    host: (&str, Option<i64>, Option<&[u8]>),
    boot: (&str, Option<i64>, Option<&[u8]>),
    policy: (&str, Option<i64>, Option<&str>),
) -> Result<Option<ExecutionProvenance>, HistoryError> {
    let is_null = |storage: &str, length: Option<i64>| storage == "null" && length.is_none();
    match (host.2, boot.2, policy.2) {
        (None, None, None)
            if is_null(host.0, host.1)
                && is_null(boot.0, boot.1)
                && is_null(policy.0, policy.1) =>
        {
            Ok(None)
        }
        (Some(host_bytes), Some(boot_bytes), Some(SCAN_RECOVERY_POLICY))
            if host.0 == "blob"
                && host.1 == Some(32)
                && host_bytes.len() == 32
                && boot.0 == "blob"
                && boot.1 == Some(32)
                && boot_bytes.len() == 32
                && policy.0 == "text"
                && policy.1 == i64::try_from(SCAN_RECOVERY_POLICY.len()).ok() =>
        {
            ExecutionProvenance::from_stored(owner, host_bytes, boot_bytes)
                .map(Some)
                .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))
        }
        _ => Err(HistoryError::new(HistoryErrorKind::CorruptData)),
    }
}

pub(super) fn insert_scan_process_claim(
    transaction: &Transaction<'_>,
    scan_id: &ScanId,
    started_at: SystemTime,
    identity: &ProcessExecutionIdentity,
) -> Result<(), HistoryError> {
    let claimed_at_unix_ms = system_time_to_unix_ms(started_at, HistoryErrorKind::InvalidInput)?;
    let owner = &identity.owner;
    let (host_identity, boot_scope, recovery_policy) =
        provenance_parameters(identity.provenance.as_ref());
    let changed = transaction
        .execute(
            "INSERT INTO scan_process_claims (
                 scan_id, record_format_version, owner_process_instance,
                 recovery_scope, claimed_at_unix_ms,
                 execution_host_identity_v1_sha256,
                 execution_boot_scope_v1_sha256, execution_recovery_policy
             ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                scan_id.as_str(),
                owner.as_str(),
                owner.recovery_scope_key(),
                claimed_at_unix_ms,
                host_identity,
                boot_scope,
                recovery_policy,
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
    }
}

/// Consume only this coordinator's exact immutable claim. An unclaimed row is
/// legacy v8 debt and remains completable by the existing typed low-level test
/// paths, but a foreign claim can never be consumed by scan ID alone.
pub(super) fn consume_owned_scan_process_claim(
    transaction: &Transaction<'_>,
    scan_id: &ScanId,
    identity: &ProcessExecutionIdentity,
) -> Result<(), HistoryError> {
    let owner = &identity.owner;
    let stored_claim = transaction
        .query_row(
            "SELECT typeof(owner_process_instance),
                    length(CAST(owner_process_instance AS BLOB)),
                    owner_process_instance, record_format_version,
                    typeof(recovery_scope),
                    length(CAST(recovery_scope AS BLOB)), recovery_scope,
                    claimed_at_unix_ms,
                    typeof(execution_host_identity_v1_sha256),
                    length(CAST(execution_host_identity_v1_sha256 AS BLOB)),
                    execution_host_identity_v1_sha256,
                    typeof(execution_boot_scope_v1_sha256),
                    length(CAST(execution_boot_scope_v1_sha256 AS BLOB)),
                    execution_boot_scope_v1_sha256,
                    typeof(execution_recovery_policy),
                    length(CAST(execution_recovery_policy AS BLOB)),
                    execution_recovery_policy,
                    (SELECT started_at_unix_ms FROM scans AS parent
                     WHERE parent.scan_id = scan_process_claims.scan_id)
             FROM scan_process_claims WHERE scan_id = ?1",
            [scan_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, Option<Vec<u8>>>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, Option<i64>>(12)?,
                    row.get::<_, Option<Vec<u8>>>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, Option<i64>>(15)?,
                    row.get::<_, Option<String>>(16)?,
                    row.get::<_, Option<i64>>(17)?,
                ))
            },
        )
        .optional()
        .map_err(map_query_sql_error)?;
    let Some((
        owner_storage,
        owner_length,
        stored_owner,
        format,
        scope_storage,
        scope_length,
        stored_scope,
        claimed_at_unix_ms,
        host_storage,
        host_length,
        host_identity,
        boot_storage,
        boot_length,
        boot_scope,
        policy_storage,
        policy_length,
        recovery_policy,
        parent_started_at_unix_ms,
    )) = stored_claim
    else {
        return Ok(());
    };
    if owner_storage != "text"
        || !(1..=128).contains(&owner_length)
        || stored_owner.len() != owner_length as usize
        || format != 1
        || claimed_at_unix_ms < 0
        || parent_started_at_unix_ms != Some(claimed_at_unix_ms)
    {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    let decoded = ProcessInstanceId::from_stored(&stored_owner)
        .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
    let expected_scope = decoded.recovery_scope_key();
    let scope_is_well_formed = match &stored_scope {
        None => scope_storage == "null" && scope_length.is_none(),
        Some(scope) => {
            scope_storage == "text"
                && scope_length == i64::try_from(scope.len()).ok()
                && scope.len() == 66
        }
    };
    if !scope_is_well_formed || expected_scope != stored_scope {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    let provenance = decode_execution_provenance(
        &decoded,
        (&host_storage, host_length, host_identity.as_deref()),
        (&boot_storage, boot_length, boot_scope.as_deref()),
        (&policy_storage, policy_length, recovery_policy.as_deref()),
    )?;
    if &decoded != owner || provenance != identity.provenance {
        return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
    }
    let (host_identity, boot_scope, recovery_policy) = provenance_parameters(provenance.as_ref());
    let changed = transaction
        .execute(
            "DELETE FROM scan_process_claims
             WHERE scan_id = ?1 AND owner_process_instance = ?2
               AND recovery_scope IS ?3 AND claimed_at_unix_ms = ?4
               AND record_format_version = 1
               AND execution_host_identity_v1_sha256 IS ?5
               AND execution_boot_scope_v1_sha256 IS ?6
               AND execution_recovery_policy IS ?7",
            params![
                scan_id.as_str(),
                owner.as_str(),
                expected_scope,
                claimed_at_unix_ms,
                host_identity,
                boot_scope,
                recovery_policy,
            ],
        )
        .map_err(map_write_sql_error)?;
    if changed == 1 {
        Ok(())
    } else {
        Err(HistoryError::new(HistoryErrorKind::InvalidTransition))
    }
}

pub(super) fn exact_scan_process_claim_matches(
    connection: &Connection,
    scan_id: &ScanId,
    started_at: SystemTime,
    identity: &ProcessExecutionIdentity,
) -> Result<bool, HistoryError> {
    let owner = &identity.owner;
    let claimed_at = system_time_to_unix_ms(started_at, HistoryErrorKind::InvalidInput)?;
    let recovery_scope = owner.recovery_scope_key();
    let (host_identity, boot_scope, recovery_policy) =
        provenance_parameters(identity.provenance.as_ref());
    let matched: i64 = connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM scan_process_claims
                 WHERE scan_id = ?1 AND owner_process_instance = ?2
                   AND claimed_at_unix_ms = ?3 AND recovery_scope IS ?4
                   AND record_format_version = 1
                   AND execution_host_identity_v1_sha256 IS ?5
                   AND execution_boot_scope_v1_sha256 IS ?6
                   AND execution_recovery_policy IS ?7
             )",
            params![
                scan_id.as_str(),
                owner.as_str(),
                claimed_at,
                recovery_scope,
                host_identity,
                boot_scope,
                recovery_policy,
            ],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    Ok(matched == 1)
}

pub(super) fn scan_process_claim_is_missing(
    connection: &Connection,
    scan_id: &ScanId,
) -> Result<bool, HistoryError> {
    let missing: i64 = connection
        .query_row(
            "SELECT NOT EXISTS(
                 SELECT 1 FROM scan_process_claims WHERE scan_id = ?1
             )",
            [scan_id.as_str()],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    Ok(missing == 1)
}

fn scan_process_claim_page_query() -> &'static str {
    // The global row-value range lets one bounded cursor traverse foreign,
    // migrated, prior-boot, and current claims without any class starving a
    // later actionable row. Classification remains separate from discovery.
    "SELECT
             typeof(claim.scan_id), length(CAST(claim.scan_id AS BLOB)),
             claim.scan_id,
             claim.record_format_version,
             typeof(claim.owner_process_instance),
             length(CAST(claim.owner_process_instance AS BLOB)),
             claim.owner_process_instance,
             typeof(claim.recovery_scope),
             length(CAST(claim.recovery_scope AS BLOB)),
             claim.recovery_scope,
             claim.claimed_at_unix_ms,
             typeof(claim.execution_host_identity_v1_sha256),
             length(CAST(claim.execution_host_identity_v1_sha256 AS BLOB)),
             claim.execution_host_identity_v1_sha256,
             typeof(claim.execution_boot_scope_v1_sha256),
             length(CAST(claim.execution_boot_scope_v1_sha256 AS BLOB)),
             claim.execution_boot_scope_v1_sha256,
             typeof(claim.execution_recovery_policy),
             length(CAST(claim.execution_recovery_policy AS BLOB)),
             claim.execution_recovery_policy,
             CASE WHEN EXISTS (
                 SELECT 1 FROM scans AS parent
                 WHERE parent.scan_id = claim.scan_id
                   AND parent.status = 'running'
                   AND parent.completed_at_unix_ms IS NULL
                   AND parent.directory_count = 0 AND parent.file_count = 0
                   AND parent.logical_bytes = 0 AND parent.allocated_bytes IS NULL
                   AND parent.snapshot_version IS NULL
                   AND parent.snapshot_relative_path IS NULL
                   AND parent.snapshot_relative_path_encoding IS NULL
                   AND parent.snapshot_checksum_sha256 IS NULL
                   AND parent.coverage_status = 'unknown'
                   AND parent.coverage_permille IS NULL
                   AND parent.issue_count = 0
                   AND parent.started_at_unix_ms = claim.claimed_at_unix_ms
                   AND NOT EXISTS (
                       SELECT 1 FROM scan_issues
                       WHERE scan_issues.scan_id = parent.scan_id
                   )
             ) THEN 1 ELSE 0 END
         FROM scan_process_claims AS claim
             INDEXED BY scan_process_claims_by_recovery_time
         WHERE (claim.claimed_at_unix_ms, claim.scan_id) > (?1, ?2)
         ORDER BY claim.claimed_at_unix_ms, claim.scan_id
         LIMIT 65"
}

pub(super) fn load_scan_process_claim_page(
    connection: &Connection,
    after: Option<(i64, &str)>,
) -> Result<ScanProcessClaimPage, HistoryError> {
    run_bounded_query(connection, || {
        let mut statement = connection
            .prepare(scan_process_claim_page_query())
            .map_err(map_query_sql_error)?;
        let (after_time, after_scan_id) = after.unwrap_or((-1, ""));
        let mut rows = statement
            .query(params![after_time, after_scan_id])
            .map_err(map_query_sql_error)?;
        let mut claims = Vec::new();
        while let Some(row) = rows.next().map_err(map_query_sql_error)? {
            let id_storage: String = row.get(0).map_err(map_query_sql_error)?;
            let id_length: i64 = row.get(1).map_err(map_query_sql_error)?;
            let id: String = row.get(2).map_err(map_query_sql_error)?;
            let format: i64 = row.get(3).map_err(map_query_sql_error)?;
            let owner_storage: String = row.get(4).map_err(map_query_sql_error)?;
            let owner_length: i64 = row.get(5).map_err(map_query_sql_error)?;
            let owner: String = row.get(6).map_err(map_query_sql_error)?;
            let scope_storage: String = row.get(7).map_err(map_query_sql_error)?;
            let scope_length: Option<i64> = row.get(8).map_err(map_query_sql_error)?;
            let stored_scope: Option<String> = row.get(9).map_err(map_query_sql_error)?;
            let claimed_at_unix_ms: i64 = row.get(10).map_err(map_query_sql_error)?;
            let host_storage: String = row.get(11).map_err(map_query_sql_error)?;
            let host_length: Option<i64> = row.get(12).map_err(map_query_sql_error)?;
            let host_identity: Option<Vec<u8>> = row
                .get(13)
                .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
            let boot_storage: String = row.get(14).map_err(map_query_sql_error)?;
            let boot_length: Option<i64> = row.get(15).map_err(map_query_sql_error)?;
            let boot_scope: Option<Vec<u8>> = row
                .get(16)
                .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
            let policy_storage: String = row.get(17).map_err(map_query_sql_error)?;
            let policy_length: Option<i64> = row.get(18).map_err(map_query_sql_error)?;
            let recovery_policy: Option<String> = row
                .get(19)
                .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
            let parent_is_pristine: i64 = row.get(20).map_err(map_query_sql_error)?;
            if id_storage != "text"
                || !(1..=128).contains(&id_length)
                || id.len() != id_length as usize
                || format != 1
                || owner_storage != "text"
                || !(1..=128).contains(&owner_length)
                || owner.len() != owner_length as usize
                || claimed_at_unix_ms < 0
                || parent_is_pristine != 1
            {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            let owner = ProcessInstanceId::from_stored(&owner)
                .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?;
            if (stored_scope.is_none() && (scope_storage != "null" || scope_length.is_some()))
                || stored_scope.as_ref().is_some_and(|scope| {
                    scope_storage != "text"
                        || scope_length != i64::try_from(scope.len()).ok()
                        || scope.len() != 66
                })
                || owner.recovery_scope_key() != stored_scope
            {
                return Err(HistoryError::new(HistoryErrorKind::CorruptData));
            }
            let provenance = decode_execution_provenance(
                &owner,
                (&host_storage, host_length, host_identity.as_deref()),
                (&boot_storage, boot_length, boot_scope.as_deref()),
                (&policy_storage, policy_length, recovery_policy.as_deref()),
            )?;
            claims.push(ScanProcessClaim {
                scan_id: ScanId::new(id)
                    .map_err(|_| HistoryError::new(HistoryErrorKind::CorruptData))?,
                owner,
                recovery_scope: stored_scope,
                claimed_at_unix_ms,
                provenance,
            });
        }
        let has_more = claims.len() > MAX_SCAN_PROCESS_CLAIMS;
        claims.truncate(MAX_SCAN_PROCESS_CLAIMS);
        Ok(ScanProcessClaimPage { claims, has_more })
    })
}

pub(super) fn count_remaining_scan_process_claims(
    connection: &Connection,
    claims: &[ScanProcessClaim],
) -> Result<u32, HistoryError> {
    run_bounded_query(connection, || {
        let mut remaining = 0_u32;
        for claim in claims {
            let (host_identity, boot_scope, recovery_policy) = claim.provenance_parameters();
            let exists: i64 = connection
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM scan_process_claims
                         WHERE scan_id = ?1 AND owner_process_instance = ?2
                           AND recovery_scope IS ?3 AND claimed_at_unix_ms = ?4
                           AND execution_host_identity_v1_sha256 IS ?5
                           AND execution_boot_scope_v1_sha256 IS ?6
                           AND execution_recovery_policy IS ?7
                     )",
                    params![
                        claim.scan_id.as_str(),
                        claim.owner.as_str(),
                        claim.recovery_scope.as_deref(),
                        claim.claimed_at_unix_ms,
                        host_identity,
                        boot_scope,
                        recovery_policy,
                    ],
                    |row| row.get(0),
                )
                .map_err(map_query_sql_error)?;
            if exists == 1 {
                remaining = remaining
                    .checked_add(1)
                    .ok_or_else(|| HistoryError::new(HistoryErrorKind::CorruptData))?;
            }
        }
        Ok(remaining)
    })
}

#[cfg(test)]
pub(super) fn load_scan_process_claims(
    connection: &Connection,
) -> Result<Vec<ScanProcessClaim>, HistoryError> {
    Ok(load_scan_process_claim_page(connection, None)?.claims)
}

pub(super) fn interrupt_scan_process_claim(
    transaction: &Transaction<'_>,
    claim: &ScanProcessClaim,
    completed_at_unix_ms: i64,
) -> Result<bool, HistoryError> {
    if completed_at_unix_ms < claim.claimed_at_unix_ms {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    let (host_identity, boot_scope, recovery_policy) = claim.provenance_parameters();
    let deleted = transaction
        .execute(
            "DELETE FROM scan_process_claims
             WHERE scan_id = ?1 AND owner_process_instance = ?2
               AND recovery_scope IS ?3 AND claimed_at_unix_ms = ?4
               AND record_format_version = 1
               AND execution_host_identity_v1_sha256 IS ?5
               AND execution_boot_scope_v1_sha256 IS ?6
               AND execution_recovery_policy IS ?7
               AND EXISTS (
                   SELECT 1 FROM scans AS parent
                   WHERE parent.scan_id = scan_process_claims.scan_id
                     AND parent.status = 'running'
                     AND parent.completed_at_unix_ms IS NULL
                     AND parent.directory_count = 0 AND parent.file_count = 0
                     AND parent.logical_bytes = 0 AND parent.allocated_bytes IS NULL
                     AND parent.snapshot_version IS NULL
                     AND parent.snapshot_relative_path IS NULL
                     AND parent.snapshot_relative_path_encoding IS NULL
                     AND parent.snapshot_checksum_sha256 IS NULL
                     AND parent.coverage_status = 'unknown'
                     AND parent.coverage_permille IS NULL
                     AND parent.issue_count = 0
                     AND parent.started_at_unix_ms = scan_process_claims.claimed_at_unix_ms
                     AND NOT EXISTS (
                         SELECT 1 FROM scan_issues
                         WHERE scan_issues.scan_id = parent.scan_id
                     )
               )",
            params![
                claim.scan_id.as_str(),
                claim.owner.as_str(),
                claim.recovery_scope.as_deref(),
                claim.claimed_at_unix_ms,
                host_identity,
                boot_scope,
                recovery_policy,
            ],
        )
        .map_err(map_write_sql_error)?;
    if deleted == 0 {
        return Ok(false);
    }
    let updated = transaction
        .execute(
            "UPDATE scans
             SET completed_at_unix_ms = ?2, status = 'interrupted'
             WHERE scan_id = ?1 AND status = 'running'
               AND completed_at_unix_ms IS NULL AND started_at_unix_ms <= ?2
               AND directory_count = 0 AND file_count = 0 AND logical_bytes = 0
               AND allocated_bytes IS NULL AND snapshot_version IS NULL
               AND snapshot_relative_path IS NULL
               AND snapshot_relative_path_encoding IS NULL
               AND snapshot_checksum_sha256 IS NULL
               AND coverage_status = 'unknown' AND coverage_permille IS NULL
               AND issue_count = 0
               AND NOT EXISTS (
                   SELECT 1 FROM scan_issues WHERE scan_issues.scan_id = scans.scan_id
               )",
            params![claim.scan_id.as_str(), completed_at_unix_ms],
        )
        .map_err(map_write_sql_error)?;
    if updated != 1 {
        return Err(HistoryError::new(HistoryErrorKind::CorruptData));
    }
    Ok(true)
}

pub(super) fn exact_recovered_scan_matches(
    connection: &Connection,
    claim: &ScanProcessClaim,
    completed_at_unix_ms: i64,
) -> Result<bool, HistoryError> {
    let matched: i64 = connection
        .query_row(
            "SELECT
                 NOT EXISTS (
                     SELECT 1 FROM scan_process_claims WHERE scan_id = ?1
                 ) AND EXISTS (
                     SELECT 1 FROM scans
                     WHERE scan_id = ?1 AND status = 'interrupted'
                       AND completed_at_unix_ms = ?2
                       AND started_at_unix_ms = ?3
                       AND directory_count = 0 AND file_count = 0
                       AND logical_bytes = 0 AND allocated_bytes IS NULL
                       AND snapshot_version IS NULL AND snapshot_relative_path IS NULL
                       AND snapshot_relative_path_encoding IS NULL
                       AND snapshot_checksum_sha256 IS NULL
                       AND coverage_status = 'unknown' AND coverage_permille IS NULL
                       AND issue_count = 0
                       AND NOT EXISTS (
                           SELECT 1 FROM scan_issues
                           WHERE scan_issues.scan_id = scans.scan_id
                       )
                 )",
            params![
                claim.scan_id.as_str(),
                completed_at_unix_ms,
                claim.claimed_at_unix_ms,
            ],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    Ok(matched == 1)
}

pub(super) fn canonical_recovery_time(
    value: SystemTime,
) -> Result<(SystemTime, i64), HistoryError> {
    let millis = system_time_to_unix_ms(value, HistoryErrorKind::InvalidInput)?;
    Ok((unix_ms_to_system_time(millis)?, millis))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScanClaimRecoveryState {
    Alive,
    Unknown,
    Recoverable,
}

pub(super) fn classify_claims(
    claims: &[ScanProcessClaim],
    current: &ProcessExecutionIdentity,
    mut probe: impl FnMut(&ProcessInstanceId) -> ProcessLiveness,
) -> (Vec<ScanClaimRecoveryState>, u32, u32, u32) {
    let current_scope = current.owner.recovery_scope_key();
    let states = claims
        .iter()
        .map(|claim| {
            match compare_execution_provenance(
                claim.provenance.as_ref(),
                current.provenance.as_ref(),
            ) {
                ProvenanceRelationship::PriorBoot => ScanClaimRecoveryState::Recoverable,
                ProvenanceRelationship::ForeignHost => ScanClaimRecoveryState::Unknown,
                ProvenanceRelationship::SameBoot => {
                    recovery_state_from_liveness(probe(claim.owner()))
                }
                ProvenanceRelationship::Unproven
                    if claim.provenance.is_none()
                        && current_scope.is_some()
                        && claim.recovery_scope.as_deref() == current_scope.as_deref() =>
                {
                    // Legacy v9-v15 claims retain the exact same-scope
                    // liveness behavior that was safe before provenance was
                    // added. A different or absent reliable scope cannot be
                    // guessed into the current boot.
                    recovery_state_from_liveness(probe(claim.owner()))
                }
                ProvenanceRelationship::Unproven
                    if claim.provenance.is_none()
                        && current_scope.is_none()
                        && claim.recovery_scope.is_none() =>
                {
                    recovery_state_from_unscoped_liveness(probe(claim.owner()))
                }
                ProvenanceRelationship::Unproven => ScanClaimRecoveryState::Unknown,
            }
        })
        .collect::<Vec<_>>();
    let alive = states
        .iter()
        .filter(|state| **state == ScanClaimRecoveryState::Alive)
        .count() as u32;
    let unknown = states
        .iter()
        .filter(|state| **state == ScanClaimRecoveryState::Unknown)
        .count() as u32;
    let recoverable = states
        .iter()
        .filter(|state| **state == ScanClaimRecoveryState::Recoverable)
        .count() as u32;
    (states, alive, unknown, recoverable)
}

fn recovery_state_from_liveness(liveness: ProcessLiveness) -> ScanClaimRecoveryState {
    match liveness {
        ProcessLiveness::Alive => ScanClaimRecoveryState::Alive,
        ProcessLiveness::Unknown => ScanClaimRecoveryState::Unknown,
        ProcessLiveness::DefinitelyGone => ScanClaimRecoveryState::Recoverable,
    }
}

fn recovery_state_from_unscoped_liveness(liveness: ProcessLiveness) -> ScanClaimRecoveryState {
    match liveness {
        ProcessLiveness::Alive => ScanClaimRecoveryState::Alive,
        ProcessLiveness::DefinitelyGone | ProcessLiveness::Unknown => {
            ScanClaimRecoveryState::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::ScanId;
    use crate::persistence::{
        NewScanRecord, ScanCompletionRecord, ScanCounts, ScanStatus, StoreCoordinator,
        TerminalScanStatus,
    };

    fn fixture() -> (
        TempDir,
        std::sync::Arc<StoreCoordinator>,
        std::path::PathBuf,
    ) {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("scan-root");
        std::fs::create_dir(&root).unwrap();
        let store = StoreCoordinator::open(&temp.path().join("data/dux.sqlite3")).unwrap();
        store.set_scan_process_identity_for_test(scoped_test_identity());
        (temp, store, root)
    }

    fn scoped_test_identity() -> ProcessExecutionIdentity {
        let owner = ProcessInstanceId::from_stored(&format!(
            "1:l:2a:1234:{}:{}",
            "11".repeat(32),
            "22".repeat(16)
        ))
        .unwrap();
        let provenance =
            ExecutionProvenance::from_stored(&owner, &[0x33; 32], &[0x11; 32]).unwrap();
        ProcessExecutionIdentity {
            owner,
            provenance: Some(provenance),
        }
    }

    fn start(
        store: &StoreCoordinator,
        root: &std::path::Path,
        id: &str,
        offset_ms: u64,
    ) -> NewScanRecord {
        let record = NewScanRecord::try_new_without_root_identity(
            ScanId::new(id).unwrap(),
            root.to_path_buf(),
            UNIX_EPOCH + Duration::from_millis(offset_ms),
        )
        .unwrap();
        store.record_scan_started_reconciled(&record).unwrap();
        record
    }

    fn finish(store: &StoreCoordinator, scan: &NewScanRecord, offset_ms: u64) {
        store
            .record_scan_finished_reconciled(
                &ScanCompletionRecord::try_new(
                    scan.id().clone(),
                    UNIX_EPOCH + Duration::from_millis(offset_ms),
                    TerminalScanStatus::Failed,
                    ScanCounts::default(),
                )
                .unwrap(),
            )
            .unwrap();
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RawClaim {
        owner: String,
        recovery_scope: Option<String>,
        claimed_at_unix_ms: i64,
        host_identity: Option<Vec<u8>>,
        boot_scope: Option<Vec<u8>>,
        recovery_policy: Option<String>,
    }

    fn raw_claim(store: &StoreCoordinator, scan: &NewScanRecord) -> RawClaim {
        store.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT owner_process_instance, recovery_scope,
                            claimed_at_unix_ms,
                            execution_host_identity_v1_sha256,
                            execution_boot_scope_v1_sha256,
                            execution_recovery_policy
                     FROM scan_process_claims WHERE scan_id = ?1",
                    [scan.id().as_str()],
                    |row| {
                        Ok(RawClaim {
                            owner: row.get(0)?,
                            recovery_scope: row.get(1)?,
                            claimed_at_unix_ms: row.get(2)?,
                            host_identity: row.get(3)?,
                            boot_scope: row.get(4)?,
                            recovery_policy: row.get(5)?,
                        })
                    },
                )
                .unwrap()
        })
    }

    fn replace_claim(store: &StoreCoordinator, scan: &NewScanRecord, claim: &RawClaim) {
        store.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                    [scan.id().as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO scan_process_claims (
                         scan_id, record_format_version, owner_process_instance,
                         recovery_scope, claimed_at_unix_ms,
                         execution_host_identity_v1_sha256,
                         execution_boot_scope_v1_sha256,
                         execution_recovery_policy
                     ) VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        scan.id().as_str(),
                        claim.owner.as_str(),
                        claim.recovery_scope.as_deref(),
                        claim.claimed_at_unix_ms,
                        claim.host_identity.as_deref(),
                        claim.boot_scope.as_deref(),
                        claim.recovery_policy.as_deref(),
                    ],
                )
                .unwrap();
        });
    }

    fn owner_with_boot_scope(owner: &str, boot_scope: &[u8; 32]) -> (String, String) {
        let scope = boot_scope
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut components = owner.split(':').map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(components.len(), 6);
        assert!(matches!(components[1].as_str(), "l" | "m"));
        components[4] = scope.clone();
        let owner = components.join(":");
        ProcessInstanceId::from_stored(&owner).unwrap();
        (owner, format!("{}:{scope}", components[1]))
    }

    fn owner_with_nonce(owner: &str, nonce: u128) -> String {
        let mut components = owner.split(':').map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(components.len(), 6);
        components[5] = format!("{nonce:032x}");
        let owner = components.join(":");
        ProcessInstanceId::from_stored(&owner).unwrap();
        owner
    }

    #[test]
    fn store_starts_atomically_claim_and_normal_completion_consumes_exact_claim() {
        let (_temp, store, root) = fixture();
        let first = start(&store, &root, "scan:claim-first", 10);
        let second = start(&store, &root, "scan:claim-second", 11);
        let owners = store.with_connection(|connection| {
            connection
                .prepare("SELECT owner_process_instance FROM scan_process_claims ORDER BY scan_id")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        });
        assert_eq!(owners.len(), 2);
        assert_eq!(owners[0], owners[1]);
        store.with_connection(|connection| {
            let provenance: Vec<(i64, i64, String)> = connection
                .prepare(
                    "SELECT length(execution_host_identity_v1_sha256),
                            length(execution_boot_scope_v1_sha256),
                            execution_recovery_policy
                     FROM scan_process_claims ORDER BY scan_id",
                )
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(
                provenance,
                vec![
                    (32, 32, SCAN_RECOVERY_POLICY.to_owned()),
                    (32, 32, SCAN_RECOVERY_POLICY.to_owned()),
                ]
            );
        });

        finish(&store, &first, 20);
        let claims =
            store.with_connection(|connection| load_scan_process_claims(connection).unwrap());
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].scan_id, second.id().clone());
    }

    #[test]
    fn normal_completion_refuses_a_scope_mismatched_claim_without_erasing_it() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:claim-scope-mismatch", 10);
        store.with_connection(|connection| {
            let (owner, scope): (String, Option<String>) = connection
                .query_row(
                    "SELECT owner_process_instance, recovery_scope
                     FROM scan_process_claims WHERE scan_id = ?1",
                    [scan.id().as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let mismatched_scope = if scope.is_some() {
                None
            } else {
                Some(format!("l:{}", "0".repeat(64)))
            };
            connection
                .execute(
                    "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                    [scan.id().as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO scan_process_claims (
                         scan_id, record_format_version, owner_process_instance,
                         recovery_scope, claimed_at_unix_ms
                     ) VALUES (?1, 1, ?2, ?3, 10)",
                    params![scan.id().as_str(), owner, mismatched_scope],
                )
                .unwrap();
        });

        let completion = ScanCompletionRecord::try_new(
            scan.id().clone(),
            UNIX_EPOCH + Duration::from_millis(20),
            TerminalScanStatus::Failed,
            ScanCounts::default(),
        )
        .unwrap();
        assert_eq!(
            store
                .record_scan_finished_reconciled(&completion)
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Running
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM scan_process_claims WHERE scan_id = ?1",
                        [scan.id().as_str()],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1
            );
        });
    }

    #[test]
    fn ambiguous_start_reconciliation_rejects_a_scope_mismatched_claim() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:start-reconcile-scope-mismatch", 10);
        let identity =
            crate::persistence::process_liveness::current_process_execution_identity().unwrap();
        let owner = &identity.owner;
        let expected_scope = owner.recovery_scope_key();
        let mismatched_scope = if expected_scope.is_some() {
            None
        } else {
            Some(format!("l:{}", "0".repeat(64)))
        };
        store.with_connection(|connection| {
            connection
                .execute(
                    "DELETE FROM scan_process_claims WHERE scan_id = ?1",
                    [scan.id().as_str()],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO scan_process_claims (
                         scan_id, record_format_version, owner_process_instance,
                         recovery_scope, claimed_at_unix_ms
                     ) VALUES (?1, 1, ?2, ?3, 10)",
                    params![scan.id().as_str(), owner.as_str(), mismatched_scope],
                )
                .unwrap();
            assert!(
                !exact_scan_process_claim_matches(
                    connection,
                    scan.id(),
                    scan.started_at(),
                    &identity,
                )
                .unwrap()
            );
        });
    }

    #[test]
    fn alive_and_unknown_claims_are_exact_no_ops() {
        for state in [ProcessLiveness::Alive, ProcessLiveness::Unknown] {
            let (_temp, store, root) = fixture();
            let scan = start(&store, &root, "scan:unproven", 10);
            let before = store.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT status, completed_at_unix_ms,
                                (SELECT count(*) FROM scan_process_claims)
                         FROM scans WHERE scan_id = ?1",
                        [scan.id().as_str()],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, Option<i64>>(1)?,
                                row.get::<_, i64>(2)?,
                            ))
                        },
                    )
                    .unwrap()
            });
            let result = store
                .run_scan_recovery_batch_with_hooks_for_test(
                    UNIX_EPOCH + Duration::from_millis(20),
                    |_| state,
                    || Ok(()),
                    || Ok(()),
                )
                .unwrap();
            assert_eq!(result.outcome, ScanRecoveryBatchOutcome::DeferredUnproven);
            assert_eq!(result.claimed_count_before, 1);
            assert_eq!(result.claimed_count_after, 1);
            assert_eq!(
                result.alive_count,
                u32::from(state == ProcessLiveness::Alive)
            );
            assert_eq!(
                result.unknown_count,
                u32::from(state == ProcessLiveness::Unknown)
            );
            let after = store.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT status, completed_at_unix_ms,
                                (SELECT count(*) FROM scan_process_claims)
                         FROM scans WHERE scan_id = ?1",
                        [scan.id().as_str()],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, Option<i64>>(1)?,
                                row.get::<_, i64>(2)?,
                            ))
                        },
                    )
                    .unwrap()
            });
            assert_eq!(after, before);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn same_host_prior_boot_claim_is_interrupted_without_a_process_probe() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:prior-boot", 10);
        let mut claim = raw_claim(&store, &scan);
        assert_eq!(claim.recovery_policy.as_deref(), Some(SCAN_RECOVERY_POLICY));
        let mut prior_boot: [u8; 32] = claim.boot_scope.clone().unwrap().try_into().unwrap();
        prior_boot[0] ^= 0xff;
        let (owner, scope) = owner_with_boot_scope(&claim.owner, &prior_boot);
        claim.owner = owner;
        claim.recovery_scope = Some(scope);
        claim.boot_scope = Some(prior_boot.to_vec());
        replace_claim(&store, &scan, &claim);

        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(20),
                |_| panic!("prior-boot recovery must not probe a PID"),
                || Ok(()),
                || Ok(()),
            )
            .unwrap();

        assert_eq!(result.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(result.alive_count, 0);
        assert_eq!(result.unknown_count, 0);
        assert_eq!(result.recoverable_count, 1);
        assert_eq!(result.claimed_count_before, 1);
        assert_eq!(result.claimed_count_after, 0);
        assert!(!result.has_more);
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Interrupted
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn foreign_host_claim_is_a_typed_byte_for_byte_no_op_without_a_probe() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:foreign-host", 10);
        let mut claim = raw_claim(&store, &scan);
        let host = claim.host_identity.as_mut().unwrap();
        host[0] ^= 0xff;
        replace_claim(&store, &scan, &claim);
        let before = raw_claim(&store, &scan);

        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(20),
                |_| panic!("foreign-host debt must not probe a PID"),
                || Ok(()),
                || Ok(()),
            )
            .unwrap();

        assert_eq!(result.outcome, ScanRecoveryBatchOutcome::DeferredUnproven);
        assert_eq!(result.unknown_count, 1);
        assert_eq!(result.recoverable_count, 0);
        assert_eq!(raw_claim(&store, &scan), before);
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Running
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn migrated_different_scope_claim_is_unproven_and_never_probed() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:migrated-other-boot", 10);
        let mut claim = raw_claim(&store, &scan);
        let mut other_scope: [u8; 32] = claim.boot_scope.clone().unwrap().try_into().unwrap();
        other_scope[0] ^= 0xff;
        let (owner, scope) = owner_with_boot_scope(&claim.owner, &other_scope);
        claim.owner = owner;
        claim.recovery_scope = Some(scope);
        claim.host_identity = None;
        claim.boot_scope = None;
        claim.recovery_policy = None;
        replace_claim(&store, &scan, &claim);
        let before = raw_claim(&store, &scan);

        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(20),
                |_| panic!("unproven cross-scope debt must not probe a PID"),
                || Ok(()),
                || Ok(()),
            )
            .unwrap();

        assert_eq!(result.outcome, ScanRecoveryBatchOutcome::DeferredUnproven);
        assert_eq!(result.unknown_count, 1);
        assert_eq!(raw_claim(&store, &scan), before);
    }

    #[test]
    fn migrated_same_scope_claim_retains_exact_liveness_recovery() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:migrated-same-boot", 10);
        let mut claim = raw_claim(&store, &scan);
        claim.host_identity = None;
        claim.boot_scope = None;
        claim.recovery_policy = None;
        replace_claim(&store, &scan, &claim);
        let mut probes = 0_u32;

        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(20),
                |_| {
                    probes += 1;
                    ProcessLiveness::DefinitelyGone
                },
                || Ok(()),
                || Ok(()),
            )
            .unwrap();

        assert_eq!(probes, 1);
        assert_eq!(result.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(result.recoverable_count, 1);
    }

    #[test]
    fn unscoped_legacy_claim_can_confirm_alive_but_never_recover() {
        let owner =
            ProcessInstanceId::from_stored("1:w:2a:1234:-:00000000000000000000000000000001")
                .unwrap();
        let current = ProcessExecutionIdentity {
            owner: owner.clone(),
            provenance: None,
        };
        let claim = ScanProcessClaim {
            scan_id: ScanId::new("scan:unscoped-unproven").unwrap(),
            owner,
            recovery_scope: None,
            claimed_at_unix_ms: 10,
            provenance: None,
        };

        let (states, alive, unknown, recoverable) =
            classify_claims(std::slice::from_ref(&claim), &current, |_| {
                ProcessLiveness::Alive
            });
        assert_eq!(states, [ScanClaimRecoveryState::Alive]);
        assert_eq!((alive, unknown, recoverable), (1, 0, 0));

        let (states, alive, unknown, recoverable) =
            classify_claims(&[claim], &current, |_| ProcessLiveness::DefinitelyGone);
        assert_eq!(states, [ScanClaimRecoveryState::Unknown]);
        assert_eq!((alive, unknown, recoverable), (0, 1, 0));
    }

    #[test]
    fn complete_claim_is_unproven_when_current_provenance_is_unavailable() {
        let stored = scoped_test_identity();
        let current = ProcessExecutionIdentity {
            owner: stored.owner.clone(),
            provenance: None,
        };
        let claim = ScanProcessClaim {
            scan_id: ScanId::new("scan:current-provenance-unavailable").unwrap(),
            owner: stored.owner.clone(),
            recovery_scope: stored.owner.recovery_scope_key(),
            claimed_at_unix_ms: 10,
            provenance: stored.provenance,
        };

        let (states, alive, unknown, recoverable) = classify_claims(&[claim], &current, |_| {
            panic!("a complete claim needs current host provenance before probing")
        });

        assert_eq!(states, [ScanClaimRecoveryState::Unknown]);
        assert_eq!((alive, unknown, recoverable), (0, 1, 0));
    }

    #[test]
    fn complete_inventory_recovers_past_an_unknown_owner_one_at_a_time() {
        let (_temp, store, root) = fixture();
        let first = start(&store, &root, "scan:recovery-first", 10);
        let second = start(&store, &root, "scan:recovery-second", 11);
        let mut probes = 0;
        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(20),
                |_| {
                    probes += 1;
                    if probes == 1 {
                        ProcessLiveness::Unknown
                    } else {
                        ProcessLiveness::DefinitelyGone
                    }
                },
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(probes, 2);
        assert_eq!(result.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(result.unknown_count, 1);
        assert_eq!(result.recoverable_count, 1);
        assert_eq!(result.claimed_count_before, 2);
        assert_eq!(result.claimed_count_after, 1);
        assert!(!result.has_more);
        assert_eq!(
            store.load_scan(first.id()).unwrap().unwrap().status(),
            ScanStatus::Running
        );
        assert_eq!(
            store.load_scan(second.id()).unwrap().unwrap().status(),
            ScanStatus::Interrupted
        );
    }

    #[test]
    fn terminal_write_winning_after_probe_is_reported_without_overwrite() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:recovery-race", 10);
        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(30),
                |_| ProcessLiveness::DefinitelyGone,
                || {
                    finish(&store, &scan, 20);
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            result.outcome,
            ScanRecoveryBatchOutcome::ChangedConcurrently
        );
        assert_eq!(result.claimed_count_after, 0);
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Failed
        );
    }

    #[test]
    fn recovery_cas_rejects_a_provenance_replacement_after_liveness_probe() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:recovery-scope-race", 10);
        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(30),
                |_| ProcessLiveness::DefinitelyGone,
                || {
                    let mut replacement = raw_claim(&store, &scan);
                    if let Some(host) = replacement.host_identity.as_mut() {
                        host[0] ^= 0xff;
                    } else {
                        replacement.recovery_scope = replacement
                            .recovery_scope
                            .as_ref()
                            .map(|_| format!("l:{}", "0".repeat(64)));
                    }
                    replace_claim(&store, &scan, &replacement);
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            result.outcome,
            ScanRecoveryBatchOutcome::ChangedConcurrently
        );
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Running
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM scan_process_claims WHERE scan_id = ?1",
                        [scan.id().as_str()],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1
            );
        });
    }

    #[test]
    fn successful_recovery_reloads_honest_after_count_across_concurrent_completion() {
        let (_temp, store, root) = fixture();
        let recovered = start(&store, &root, "scan:recovery-count-target", 10);
        let concurrent = start(&store, &root, "scan:recovery-count-concurrent", 11);
        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(30),
                |_| ProcessLiveness::DefinitelyGone,
                || {
                    finish(&store, &concurrent, 20);
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(result.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(result.claimed_count_before, 2);
        assert_eq!(result.claimed_count_after, 0);
        assert_eq!(result.recoverable_count, 2);
        assert!(result.has_more);
        assert_eq!(
            store.load_scan(recovered.id()).unwrap().unwrap().status(),
            ScanStatus::Interrupted
        );
        assert_eq!(
            store.load_scan(concurrent.id()).unwrap().unwrap().status(),
            ScanStatus::Failed
        );
    }

    #[test]
    fn exact_post_commit_reconciliation_returns_interrupted() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:recovery-ambiguous", 10);
        let result = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(20),
                |_| ProcessLiveness::DefinitelyGone,
                || Ok(()),
                || Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable)),
            )
            .unwrap();
        assert_eq!(result.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(result.claimed_count_after, 0);
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Interrupted
        );
    }

    #[test]
    fn malformed_owner_fails_closed_without_mutating_scan_or_claim() {
        let (_temp, store, root) = fixture();
        let scan = NewScanRecord::try_new_without_root_identity(
            ScanId::new("scan:malformed-owner").unwrap(),
            root.clone(),
            UNIX_EPOCH + Duration::from_millis(10),
        )
        .unwrap();
        let observed = crate::persistence::observe_host_path(&root).unwrap();
        let encoding = match observed.encoding() {
            crate::persistence::HostPathObservationEncoding::Utf8 => 1_i64,
            crate::persistence::HostPathObservationEncoding::Utf16LittleEndian => 2_i64,
        };
        let scope = crate::persistence::process_liveness::current_process_instance()
            .unwrap()
            .recovery_scope_key();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO scans (
                         scan_id, root_path, root_path_encoding, started_at_unix_ms,
                         status, coverage_status
                     ) VALUES (?1, ?2, ?3, 10, 'running', 'unknown')",
                    params![scan.id().as_str(), observed.bytes(), encoding],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO scan_process_claims (
                         scan_id, record_format_version, owner_process_instance,
                         recovery_scope, claimed_at_unix_ms
                     ) VALUES (?1, 1, 'malformed', ?2, 10)",
                    params![scan.id().as_str(), scope],
                )
                .unwrap();
        });
        let before = store.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT status, completed_at_unix_ms, owner_process_instance
                     FROM scans JOIN scan_process_claims USING (scan_id)
                     WHERE scan_id = ?1",
                    [scan.id().as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .unwrap()
        });
        assert_eq!(
            store
                .run_scan_recovery_batch_with_hooks_for_test(
                    UNIX_EPOCH + Duration::from_millis(20),
                    |_| ProcessLiveness::DefinitelyGone,
                    || Ok(()),
                    || Ok(()),
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::CorruptData
        );
        let after = store.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT status, completed_at_unix_ms, owner_process_instance
                     FROM scans JOIN scan_process_claims USING (scan_id)
                     WHERE scan_id = ?1",
                    [scan.id().as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .unwrap()
        });
        assert_eq!(after, before);
    }

    #[test]
    fn competing_recoverer_wins_exactly_once() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:two-recoverers", 10);
        let winner = std::cell::RefCell::new(None);
        let outer = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(20),
                |_| ProcessLiveness::DefinitelyGone,
                || {
                    *winner.borrow_mut() = Some(
                        store
                            .run_scan_recovery_batch_with_hooks_for_test(
                                UNIX_EPOCH + Duration::from_millis(20),
                                |_| ProcessLiveness::DefinitelyGone,
                                || Ok(()),
                                || Ok(()),
                            )
                            .unwrap(),
                    );
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            winner.borrow().as_ref().unwrap().outcome,
            ScanRecoveryBatchOutcome::Interrupted
        );
        assert_eq!(outer.outcome, ScanRecoveryBatchOutcome::ChangedConcurrently);
        assert_eq!(outer.claimed_count_after, 0);
        assert_eq!(
            store.load_scan(scan.id()).unwrap().unwrap().status(),
            ScanStatus::Interrupted
        );
    }

    #[test]
    fn schema_upgrade_between_probe_and_cas_preserves_running_claim() {
        let (_temp, store, root) = fixture();
        let scan = start(&store, &root, "scan:recovery-schema-race", 10);
        let result = store.run_scan_recovery_batch_with_hooks_for_test(
            UNIX_EPOCH + Duration::from_millis(20),
            |_| ProcessLiveness::DefinitelyGone,
            || {
                store.with_connection(|connection| {
                    let future = i64::from(crate::DATABASE_SCHEMA_VERSION + 1);
                    connection
                        .execute(
                            "INSERT INTO schema_migrations (
                                 version, name, checksum_sha256, applied_at_unix_ms
                             ) VALUES (?1, 'future-scan-recovery', zeroblob(32), 20)",
                            [future],
                        )
                        .unwrap();
                    connection
                        .pragma_update(None, "user_version", future)
                        .unwrap();
                });
                Ok(())
            },
            || Ok(()),
        );
        assert_eq!(
            result.unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
        store.with_connection(|connection| {
            let row: (String, Option<i64>, i64) = connection
                .query_row(
                    "SELECT status, completed_at_unix_ms,
                            (SELECT count(*) FROM scan_process_claims WHERE scan_id = ?1)
                     FROM scans WHERE scan_id = ?1",
                    [scan.id().as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(row, ("running".to_owned(), None, 1));
        });
    }

    #[test]
    fn claim_relation_is_immutable_and_hard_bounded() {
        let (_temp, store, root) = fixture();
        for index in 0..MAX_SCAN_PROCESS_CLAIMS {
            start(
                &store,
                &root,
                &format!("scan:bounded-{index:02}"),
                10 + index as u64,
            );
        }
        let overflow = NewScanRecord::try_new_without_root_identity(
            ScanId::new("scan:bounded-overflow").unwrap(),
            root,
            UNIX_EPOCH + Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(
            store
                .record_scan_started_reconciled(&overflow)
                .unwrap_err()
                .kind,
            HistoryErrorKind::DatabaseUnavailable
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM scan_process_claims", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap(),
                MAX_SCAN_PROCESS_CLAIMS as i64
            );
            assert_eq!(
                connection
                    .query_row("SELECT count(*) FROM scans", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                MAX_SCAN_PROCESS_CLAIMS as i64
            );
            assert!(
                connection
                    .execute(
                        "UPDATE scan_process_claims SET claimed_at_unix_ms = 999",
                        [],
                    )
                    .is_err()
            );
        });
    }

    #[test]
    fn global_keyset_page_reaches_current_work_past_sixty_five_unproven_claims() {
        let (_temp, store, root) = fixture();
        let observed = crate::persistence::observe_host_path(&root).unwrap();
        let encoding = match observed.encoding() {
            crate::persistence::HostPathObservationEncoding::Utf8 => 1_i64,
            crate::persistence::HostPathObservationEncoding::Utf16LittleEndian => 2_i64,
        };
        store.with_connection(|connection| {
            for index in 0..=MAX_SCAN_PROCESS_CLAIMS {
                let owner = format!("1:l:2a:1234:{}:{index:032x}", "aa".repeat(32),);
                ProcessInstanceId::from_stored(&owner).unwrap();
                connection
                    .execute(
                        "INSERT INTO scans (
                             scan_id, root_path, root_path_encoding, started_at_unix_ms,
                             status, coverage_status
                         ) VALUES (?1, ?2, ?3, ?4, 'running', 'unknown')",
                        params![
                            format!("scan:foreign-debt-{index:03}"),
                            observed.bytes(),
                            encoding,
                            100_i64 + index as i64,
                        ],
                    )
                    .unwrap();
                connection
                    .execute(
                        "INSERT INTO scan_process_claims (
                             scan_id, record_format_version, owner_process_instance,
                             recovery_scope, claimed_at_unix_ms
                         ) VALUES (?1, 1, ?2, ?3, ?4)",
                        params![
                            format!("scan:foreign-debt-{index:03}"),
                            owner,
                            format!("l:{}", "aa".repeat(32)),
                            100_i64 + index as i64,
                        ],
                    )
                    .unwrap();
            }
        });

        let own = start(&store, &root, "scan:own-after-foreign-debt", 1_000);
        let first = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(2_000),
                |_| panic!("cross-scope unproven claims must not be probed"),
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(first.outcome, ScanRecoveryBatchOutcome::DeferredUnproven);
        assert_eq!(first.claimed_count_before, MAX_SCAN_PROCESS_CLAIMS as u32);
        assert!(first.has_more);

        let mut probes = 0_u32;
        let second = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(2_001),
                |_| {
                    probes += 1;
                    ProcessLiveness::DefinitelyGone
                },
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(second.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(second.claimed_count_before, 2);
        assert_eq!(second.claimed_count_after, 1);
        assert_eq!(second.unknown_count, 1);
        assert_eq!(second.recoverable_count, 1);
        assert_eq!(probes, 1);
        assert!(!second.has_more);
        assert_eq!(
            store.load_scan(own.id()).unwrap().unwrap().status(),
            ScanStatus::Interrupted
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn actionable_rows_inside_a_full_page_are_not_skipped_by_cursor_publication() {
        let (_temp, store, root) = fixture();
        let first = start(&store, &root, "scan:cursor-000", 100);
        let mut first_claim = raw_claim(&store, &first);
        assert!(first_claim.recovery_scope.is_some());
        first_claim.host_identity = None;
        first_claim.boot_scope = None;
        first_claim.recovery_policy = None;
        replace_claim(&store, &first, &first_claim);
        let observed = crate::persistence::observe_host_path(&root).unwrap();
        let encoding = match observed.encoding() {
            crate::persistence::HostPathObservationEncoding::Utf8 => 1_i64,
            crate::persistence::HostPathObservationEncoding::Utf16LittleEndian => 2_i64,
        };
        store.with_connection(|connection| {
            for index in 1..=MAX_SCAN_PROCESS_CLAIMS {
                let scan_id = format!("scan:cursor-{index:03}");
                let started_at = 100_i64 + index as i64;
                let owner = owner_with_nonce(&first_claim.owner, index as u128 + 1);
                connection
                    .execute(
                        "INSERT INTO scans (
                             scan_id, root_path, root_path_encoding,
                             started_at_unix_ms, status, coverage_status
                         ) VALUES (?1, ?2, ?3, ?4, 'running', 'unknown')",
                        params![scan_id, observed.bytes(), encoding, started_at],
                    )
                    .unwrap();
                connection
                    .execute(
                        "INSERT INTO scan_process_claims (
                             scan_id, record_format_version,
                             owner_process_instance, recovery_scope,
                             claimed_at_unix_ms
                         ) VALUES (?1, 1, ?2, ?3, ?4)",
                        params![
                            scan_id,
                            owner,
                            first_claim.recovery_scope.as_deref(),
                            started_at,
                        ],
                    )
                    .unwrap();
            }
        });

        let mut first_page_probe = 0_u32;
        let first_batch = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(1_000),
                |_| {
                    first_page_probe += 1;
                    if matches!(first_page_probe, 2 | 3) {
                        ProcessLiveness::DefinitelyGone
                    } else {
                        ProcessLiveness::Alive
                    }
                },
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(first_page_probe, MAX_SCAN_PROCESS_CLAIMS as u32);
        assert_eq!(first_batch.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(first_batch.recoverable_count, 2);
        assert!(first_batch.has_more);

        let mut second_page_probe = 0_u32;
        let second_batch = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(1_001),
                |_| {
                    second_page_probe += 1;
                    if second_page_probe == 1 {
                        ProcessLiveness::DefinitelyGone
                    } else {
                        ProcessLiveness::Alive
                    }
                },
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(second_page_probe, 63);
        assert_eq!(second_batch.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(second_batch.recoverable_count, 1);
        assert!(!second_batch.has_more);
        for index in [1, 2] {
            let scan_id = ScanId::new(format!("scan:cursor-{index:03}")).unwrap();
            assert_eq!(
                store.load_scan(&scan_id).unwrap().unwrap().status(),
                ScanStatus::Interrupted
            );
        }
        assert_eq!(
            store.load_scan(first.id()).unwrap().unwrap().status(),
            ScanStatus::Running
        );
        let tail = ScanId::new("scan:cursor-064").unwrap();
        assert_eq!(
            store.load_scan(&tail).unwrap().unwrap().status(),
            ScanStatus::Running
        );
    }

    #[test]
    fn exhausted_suffix_wraps_to_find_earlier_newly_recoverable_work() {
        let (_temp, store, root) = fixture();
        let first = start(&store, &root, "scan:wrap-000", 100);
        let first_claim = raw_claim(&store, &first);
        let observed = crate::persistence::observe_host_path(&root).unwrap();
        let encoding = match observed.encoding() {
            crate::persistence::HostPathObservationEncoding::Utf8 => 1_i64,
            crate::persistence::HostPathObservationEncoding::Utf16LittleEndian => 2_i64,
        };
        store.with_connection(|connection| {
            for index in 1..=MAX_SCAN_PROCESS_CLAIMS {
                let scan_id = format!("scan:wrap-{index:03}");
                let started_at = 100_i64 + index as i64;
                let owner = owner_with_nonce(&first_claim.owner, index as u128 + 1);
                connection
                    .execute(
                        "INSERT INTO scans (
                             scan_id, root_path, root_path_encoding,
                             started_at_unix_ms, status, coverage_status
                         ) VALUES (?1, ?2, ?3, ?4, 'running', 'unknown')",
                        params![scan_id, observed.bytes(), encoding, started_at],
                    )
                    .unwrap();
                connection
                    .execute(
                        "INSERT INTO scan_process_claims (
                             scan_id, record_format_version,
                             owner_process_instance, recovery_scope,
                             claimed_at_unix_ms
                         ) VALUES (?1, 1, ?2, ?3, ?4)",
                        params![
                            scan_id,
                            owner,
                            first_claim.recovery_scope.as_deref(),
                            started_at,
                        ],
                    )
                    .unwrap();
            }
        });

        let first_batch = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(1_000),
                |_| ProcessLiveness::Alive,
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            first_batch.outcome,
            ScanRecoveryBatchOutcome::DeferredUnproven
        );
        assert_eq!(
            first_batch.claimed_count_before,
            MAX_SCAN_PROCESS_CLAIMS as u32
        );
        assert!(first_batch.has_more);

        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .execute(
                        "DELETE FROM scan_process_claims
                         WHERE scan_id = 'scan:wrap-064'",
                        [],
                    )
                    .unwrap(),
                1
            );
            assert_eq!(
                connection
                    .execute(
                        "UPDATE scans
                         SET status = 'interrupted', completed_at_unix_ms = 2_000
                         WHERE scan_id = 'scan:wrap-064' AND status = 'running'",
                        [],
                    )
                    .unwrap(),
                1
            );
        });

        let mut probes = 0_u32;
        let wrapped = store
            .run_scan_recovery_batch_with_hooks_for_test(
                UNIX_EPOCH + Duration::from_millis(2_001),
                |_| {
                    probes += 1;
                    if probes == 1 {
                        ProcessLiveness::DefinitelyGone
                    } else {
                        ProcessLiveness::Alive
                    }
                },
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(probes, MAX_SCAN_PROCESS_CLAIMS as u32);
        assert_eq!(wrapped.outcome, ScanRecoveryBatchOutcome::Interrupted);
        assert_eq!(wrapped.recoverable_count, 1);
        assert!(!wrapped.has_more);
        assert_eq!(
            store.load_scan(first.id()).unwrap().unwrap().status(),
            ScanStatus::Interrupted
        );
    }

    #[test]
    fn claim_page_query_is_an_index_ordered_range_without_a_temp_sort() {
        let (_temp, store, _root) = fixture();
        store.with_connection(|connection| {
            let query = format!("EXPLAIN QUERY PLAN {}", scan_process_claim_page_query());
            let mut statement = connection.prepare(&query).unwrap();
            let mut rows = statement.query(params![-1_i64, ""]).unwrap();
            let mut details = Vec::new();
            while let Some(row) = rows.next().unwrap() {
                details.push(row.get::<_, String>(3).unwrap());
            }
            assert!(
                details.iter().any(|detail| {
                    detail.contains("SEARCH claim USING INDEX scan_process_claims_by_recovery_time")
                }),
                "query did not seek through the global cursor index: {details:?}"
            );
            assert!(
                details
                    .iter()
                    .all(|detail| !detail.contains("USE TEMP B-TREE")),
                "query introduced an unbounded temp sort: {details:?}"
            );
        });
    }
}
