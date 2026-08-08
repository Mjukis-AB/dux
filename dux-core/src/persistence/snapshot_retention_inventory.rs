//! Bounded, read-only reconciliation of snapshot history and physical storage.
//!
//! This is a policy observation, never retention authority. It deliberately
//! exposes no tombstone writer, unlink handle, or temporary-file scavenger.
//! The production mutation path repeats every eligibility proof while holding
//! the same database-before-snapshot locks and an exact retained file handle.

#![allow(
    dead_code,
    reason = "sealed retention metadata remains internal before app/FFI presentation"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;
#[cfg(test)]
use std::time::UNIX_EPOCH;

use rusqlite::{Connection, Row, params};

use crate::domain::ScanId;
use crate::path_validation::validate_scan_root;

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error,
    run_bounded_snapshot_retention_inventory_query, unix_ms_to_system_time,
};
#[cfg(test)]
use super::snapshot::SnapshotDigest;
use super::snapshot::{
    SnapshotFileName, SnapshotFileUsage, SnapshotInventoryEntryKind, SnapshotReference,
    SnapshotStoreInventoryLease, SnapshotTempKernelState,
};
use super::snapshot_review_pin::{SnapshotReviewPinPopulation, SnapshotReviewPinSummary};
use super::snapshot_temp_lease::{SnapshotTempLeasePopulation, StoredSnapshotTempLease};

const MAX_ID_BYTES: i64 = 128;
const MAX_STATUS_BYTES: i64 = 16;
const MAX_PATH_BYTES: i64 = 65_536;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SnapshotRetentionUsage {
    pub(crate) logical_bytes: u64,
    pub(crate) allocated_bytes: u64,
    pub(crate) charged_bytes: u64,
}

impl SnapshotRetentionUsage {
    fn checked_add_file(&mut self, usage: SnapshotFileUsage) -> Result<(), HistoryError> {
        self.logical_bytes = checked_add(self.logical_bytes, usage.logical_bytes())?;
        self.allocated_bytes = checked_add(self.allocated_bytes, usage.allocated_bytes())?;
        self.charged_bytes = checked_add(self.charged_bytes, usage.charged_bytes())?;
        Ok(())
    }

    fn checked_add_usage(&mut self, usage: Self) -> Result<(), HistoryError> {
        self.logical_bytes = checked_add(self.logical_bytes, usage.logical_bytes)?;
        self.allocated_bytes = checked_add(self.allocated_bytes, usage.allocated_bytes)?;
        self.charged_bytes = checked_add(self.charged_bytes, usage.charged_bytes)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotRetentionLogicalState {
    Available,
    Tombstoned { committed_at: SystemTime },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotRetentionEntry {
    pub(crate) scan_id: ScanId,
    pub(crate) root: PathBuf,
    pub(crate) started_at: SystemTime,
    pub(crate) completed_at: SystemTime,
    pub(crate) reference: SnapshotReference,
    pub(crate) usage: SnapshotFileUsage,
    pub(crate) logical_state: SnapshotRetentionLogicalState,
    pub(crate) pins: SnapshotReviewPinSummary,
    /// One or two only for physically present, logically available snapshots.
    pub(crate) latest_rank: Option<u8>,
    root_key: SnapshotRootKey,
}

impl SnapshotRetentionEntry {
    pub(crate) fn is_policy_protected(&self) -> bool {
        self.latest_rank.is_some() || self.pins.active > 0
    }

    pub(crate) fn is_eviction_observation(&self) -> bool {
        self.logical_state == SnapshotRetentionLogicalState::Available
            && !self.is_policy_protected()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotOrphanFinal {
    pub(crate) file_name: SnapshotFileName,
    pub(crate) usage: SnapshotFileUsage,
}

/// Complete, bounded observation of physical finals that have no scan-row
/// snapshot-path reference. This classifier deliberately ignores cap policy,
/// pins, tombstones, and temporary files: those facts cannot grant or withhold
/// authority to reconcile a physically published orphan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SnapshotPhysicalOrphanInventory {
    pub(super) finals: Vec<SnapshotOrphanFinal>,
    pub(super) usage: SnapshotRetentionUsage,
}

/// Classify only physical finals against the indexed snapshot-path column.
/// The surrounding repository owns the current-schema database guard and the
/// snapshot inventory writer lease for the complete proof and mutation.
pub(super) fn build_snapshot_physical_orphan_inventory(
    connection: &Connection,
    storage: &SnapshotStoreInventoryLease,
) -> Result<SnapshotPhysicalOrphanInventory, HistoryError> {
    let final_count = storage
        .entries()
        .iter()
        .filter(|entry| matches!(entry.kind(), SnapshotInventoryEntryKind::Final(_)))
        .count();
    run_bounded_snapshot_retention_inventory_query(connection, final_count, || {
        let mut statement = connection
            .prepare(CATALOG_BY_SNAPSHOT_PATH)
            .map_err(map_query_sql_error)?;
        let mut finals = Vec::new();
        let mut usage = SnapshotRetentionUsage::default();
        let mut seen_scan_ids = BTreeSet::new();
        for physical in storage.entries() {
            let SnapshotInventoryEntryKind::Final(file_name) = physical.kind() else {
                continue;
            };
            let encoded = encode_host_path(Path::new(file_name.as_str())).map_err(|_| corrupt())?;
            let mut rows = statement
                .query(params![encoded.encoding as i64, encoded.bytes])
                .map_err(map_query_sql_error)?;
            let Some(first) = rows.next().map_err(map_query_sql_error)? else {
                usage.checked_add_file(physical.usage())?;
                finals.push(SnapshotOrphanFinal {
                    file_name: file_name.clone(),
                    usage: physical.usage(),
                });
                continue;
            };
            let raw = raw_catalog_row(first).map_err(map_query_sql_error)?;
            if rows.next().map_err(map_query_sql_error)?.is_some() {
                return Err(corrupt());
            }
            let row = decode_catalog_row(raw, file_name)?;
            if !seen_scan_ids.insert(row.scan_id) {
                return Err(corrupt());
            }
        }
        finals.sort_by(|left, right| left.file_name.cmp(&right.file_name));
        Ok(SnapshotPhysicalOrphanInventory { finals, usage })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotTemporaryState {
    Active,
    QuiescentAtObservation,
    Unleased,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotTemporaryObservation {
    pub(crate) temp_name: String,
    pub(crate) usage: SnapshotFileUsage,
    pub(crate) state: SnapshotTemporaryState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotTempLeaseResidual {
    pub(crate) scan_id: ScanId,
    pub(crate) temp_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotEvictionObservation {
    pub(crate) scan_id: ScanId,
    pub(crate) usage: SnapshotFileUsage,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SnapshotRetentionTotals {
    pub(crate) store_marker: SnapshotRetentionUsage,
    pub(crate) writer_lock: SnapshotRetentionUsage,
    pub(crate) controls: SnapshotRetentionUsage,
    pub(crate) available: SnapshotRetentionUsage,
    pub(crate) protected: SnapshotRetentionUsage,
    pub(crate) eligible: SnapshotRetentionUsage,
    pub(crate) tombstoned_residual: SnapshotRetentionUsage,
    pub(crate) orphan: SnapshotRetentionUsage,
    pub(crate) temporary_active: SnapshotRetentionUsage,
    pub(crate) temporary_quiescent: SnapshotRetentionUsage,
    pub(crate) temporary_unleased: SnapshotRetentionUsage,
    pub(crate) store_total: SnapshotRetentionUsage,
    pub(crate) active_pin_rows: u32,
    pub(crate) expired_pin_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotRetentionInventory {
    pub(crate) observed_at: SystemTime,
    pub(crate) cap_bytes: u64,
    pub(crate) entries: Vec<SnapshotRetentionEntry>,
    pub(crate) orphan_finals: Vec<SnapshotOrphanFinal>,
    pub(crate) temporary_files: Vec<SnapshotTemporaryObservation>,
    pub(crate) residual_temp_leases: Vec<SnapshotTempLeaseResidual>,
    pub(crate) eviction_observations: Vec<SnapshotEvictionObservation>,
    pub(crate) totals: SnapshotRetentionTotals,
    pub(crate) cap_excess_bytes: u64,
    /// Controls, unleased temps, residuals, orphans, and protected snapshots
    /// alone exceed the cap. Normal candidate eviction cannot satisfy it.
    pub(crate) non_evictable_over_cap: bool,
    /// True whenever a recognized temp can still be growing outside the lock.
    pub(crate) accounting_unstable: bool,
}

impl SnapshotRetentionInventory {
    /// Return the first exact physical residual whose logical tombstone is
    /// already durable. Residual removal is ordered by tombstone commit, then
    /// immutable scan chronology and ID, so retries are deterministic without
    /// treating age as deletion authority.
    pub(crate) fn oldest_tombstoned_residual(&self) -> Option<&SnapshotRetentionEntry> {
        self.entries
            .iter()
            .filter_map(|entry| {
                if let SnapshotRetentionLogicalState::Tombstoned { committed_at } =
                    entry.logical_state
                {
                    Some((entry, committed_at))
                } else {
                    None
                }
            })
            .min_by(|(left, left_committed), (right, right_committed)| {
                left_committed
                    .cmp(right_committed)
                    .then_with(|| left.completed_at.cmp(&right.completed_at))
                    .then_with(|| left.started_at.cmp(&right.started_at))
                    .then_with(|| left.scan_id.cmp(&right.scan_id))
            })
            .map(|(entry, _)| entry)
    }

    /// Resolve the inventory's already policy-sorted observation back to its
    /// complete immutable entry. The mutation layer still reuses this same
    /// locked inventory and repeats all database and physical validations.
    pub(crate) fn oldest_eviction_candidate(&self) -> Option<&SnapshotRetentionEntry> {
        let scan_id = &self.eviction_observations.first()?.scan_id;
        self.entries.iter().find(|entry| &entry.scan_id == scan_id)
    }

    pub(crate) fn has_additional_work_after(&self, removed_scan_id: &ScanId) -> bool {
        self.entries.iter().any(|entry| {
            &entry.scan_id != removed_scan_id
                && matches!(
                    entry.logical_state,
                    SnapshotRetentionLogicalState::Tombstoned { .. }
                )
        }) || (self.totals.store_total.charged_bytes.saturating_sub(
            self.entries
                .iter()
                .find(|entry| &entry.scan_id == removed_scan_id)
                .map_or(0, |entry| entry.usage.charged_bytes()),
        ) > self.cap_bytes
            && self
                .eviction_observations
                .iter()
                .any(|candidate| &candidate.scan_id != removed_scan_id))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SnapshotRootKey {
    encoding: i64,
    bytes: Vec<u8>,
}

struct CatalogRow {
    scan_id: ScanId,
    root: PathBuf,
    root_key: SnapshotRootKey,
    started_at: SystemTime,
    completed_at: SystemTime,
    reference: SnapshotReference,
    logical_state: SnapshotRetentionLogicalState,
}

struct RawCatalogRow {
    scan_id: String,
    root: Vec<u8>,
    root_encoding: i64,
    started_at_unix_ms: i64,
    completed_at_unix_ms: i64,
    status: String,
    snapshot_version: i64,
    snapshot_relative_path: Vec<u8>,
    snapshot_relative_path_encoding: i64,
    snapshot_checksum_sha256: Vec<u8>,
    tombstone: Option<RawTombstone>,
}

struct RawTombstone {
    scan_id: String,
    record_format_version: i64,
    scan_status: String,
    completed_at_unix_ms: i64,
    snapshot_version: i64,
    snapshot_relative_path: Vec<u8>,
    snapshot_relative_path_encoding: i64,
    snapshot_checksum_sha256: Vec<u8>,
    committed_at_unix_ms: i64,
}

pub(super) fn build_snapshot_retention_inventory(
    connection: &Connection,
    storage: &SnapshotStoreInventoryLease,
    pins: SnapshotReviewPinPopulation,
    temp_leases: SnapshotTempLeasePopulation,
    observed_at: SystemTime,
    cap_bytes: u64,
) -> Result<SnapshotRetentionInventory, HistoryError> {
    let final_count = storage
        .entries()
        .iter()
        .filter(|entry| matches!(entry.kind(), SnapshotInventoryEntryKind::Final(_)))
        .count();
    run_bounded_snapshot_retention_inventory_query(connection, final_count, || {
        build_snapshot_retention_inventory_bounded(
            connection,
            storage,
            pins,
            temp_leases,
            observed_at,
            cap_bytes,
        )
    })
}

fn build_snapshot_retention_inventory_bounded(
    connection: &Connection,
    storage: &SnapshotStoreInventoryLease,
    mut pins: SnapshotReviewPinPopulation,
    temp_leases: SnapshotTempLeasePopulation,
    observed_at: SystemTime,
    cap_bytes: u64,
) -> Result<SnapshotRetentionInventory, HistoryError> {
    let mut statement = connection
        .prepare(CATALOG_BY_SNAPSHOT_PATH)
        .map_err(map_query_sql_error)?;
    let mut entries = Vec::new();
    let mut orphan_finals = Vec::new();
    let mut temporary_files = Vec::new();
    let mut temp_leases_by_name = temp_leases
        .rows()
        .iter()
        .cloned()
        .map(|row| (row.temp_name().to_owned(), row))
        .collect::<BTreeMap<String, StoredSnapshotTempLease>>();
    let mut seen_scan_ids = BTreeSet::new();

    for physical in storage.entries() {
        match physical.kind() {
            SnapshotInventoryEntryKind::RecognizedTemp => {
                let state = match temp_leases_by_name.remove(physical.name()) {
                    Some(_) => match physical.temp_kernel_state() {
                        Some(SnapshotTempKernelState::Active) => SnapshotTemporaryState::Active,
                        Some(SnapshotTempKernelState::Quiescent) => {
                            SnapshotTemporaryState::QuiescentAtObservation
                        }
                        None => return Err(corrupt()),
                    },
                    None => SnapshotTemporaryState::Unleased,
                };
                temporary_files.push(SnapshotTemporaryObservation {
                    temp_name: physical.name().to_owned(),
                    usage: physical.usage(),
                    state,
                });
            }
            SnapshotInventoryEntryKind::Final(file_name) => {
                let encoded =
                    encode_host_path(Path::new(file_name.as_str())).map_err(|_| corrupt())?;
                let mut rows = statement
                    .query(params![encoded.encoding as i64, encoded.bytes])
                    .map_err(map_query_sql_error)?;
                let first = rows.next().map_err(map_query_sql_error)?;
                let Some(first) = first else {
                    orphan_finals.push(SnapshotOrphanFinal {
                        file_name: file_name.clone(),
                        usage: physical.usage(),
                    });
                    continue;
                };
                let raw = raw_catalog_row(first).map_err(map_query_sql_error)?;
                if rows.next().map_err(map_query_sql_error)?.is_some() {
                    return Err(corrupt());
                }
                let row = decode_catalog_row(raw, file_name)?;
                if !seen_scan_ids.insert(row.scan_id.clone()) {
                    return Err(corrupt());
                }
                let pin_summary = pins.by_scan.remove(&row.scan_id).unwrap_or_default();
                entries.push(SnapshotRetentionEntry {
                    scan_id: row.scan_id,
                    root: row.root,
                    started_at: row.started_at,
                    completed_at: row.completed_at,
                    reference: row.reference,
                    usage: physical.usage(),
                    logical_state: row.logical_state,
                    pins: pin_summary,
                    latest_rank: None,
                    root_key: row.root_key,
                });
            }
        }
    }
    drop(statement);

    let residual_temp_leases = temp_leases_by_name
        .into_values()
        .map(|row| SnapshotTempLeaseResidual {
            scan_id: row.scan_id().clone(),
            temp_name: row.temp_name().to_owned(),
        })
        .collect::<Vec<_>>();

    // An active, exact pin whose named final is absent cannot safely disappear
    // from retention policy. Expired rows remain coordination debt only.
    if pins.by_scan.values().any(|summary| summary.active > 0) {
        return Err(corrupt());
    }

    assign_latest_two(&mut entries)?;
    entries.sort_by(|left, right| left.scan_id.cmp(&right.scan_id));
    orphan_finals.sort_by(|left, right| left.file_name.cmp(&right.file_name));

    let mut eviction_indices = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.is_eviction_observation().then_some(index))
        .collect::<Vec<_>>();
    eviction_indices.sort_by(|left, right| {
        let left = &entries[*left];
        let right = &entries[*right];
        left.completed_at
            .cmp(&right.completed_at)
            .then_with(|| left.started_at.cmp(&right.started_at))
            .then_with(|| left.scan_id.cmp(&right.scan_id))
    });
    let eviction_observations = eviction_indices
        .into_iter()
        .map(|index| SnapshotEvictionObservation {
            scan_id: entries[index].scan_id.clone(),
            usage: entries[index].usage,
        })
        .collect::<Vec<_>>();

    let totals = calculate_totals(storage, &entries, &orphan_finals, &temporary_files, &pins)?;
    let cap_excess_bytes = totals.store_total.charged_bytes.saturating_sub(cap_bytes);
    let mut non_evictable = totals.controls;
    for usage in [
        totals.protected,
        totals.tombstoned_residual,
        totals.orphan,
        totals.temporary_active,
        totals.temporary_quiescent,
        totals.temporary_unleased,
    ] {
        non_evictable.checked_add_usage(usage)?;
    }

    Ok(SnapshotRetentionInventory {
        observed_at,
        cap_bytes,
        entries,
        orphan_finals,
        accounting_unstable: temporary_files.iter().any(|temp| {
            matches!(
                temp.state,
                SnapshotTemporaryState::Active | SnapshotTemporaryState::Unleased
            )
        }),
        temporary_files,
        residual_temp_leases,
        eviction_observations,
        totals,
        cap_excess_bytes,
        non_evictable_over_cap: non_evictable.charged_bytes > cap_bytes,
    })
}

fn assign_latest_two(entries: &mut [SnapshotRetentionEntry]) -> Result<(), HistoryError> {
    let mut by_root: BTreeMap<SnapshotRootKey, Vec<usize>> = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.logical_state == SnapshotRetentionLogicalState::Available {
            by_root
                .entry(entry.root_key.clone())
                .or_default()
                .push(index);
        }
    }
    for indices in by_root.values_mut() {
        indices.sort_by(|left, right| {
            let left = &entries[*left];
            let right = &entries[*right];
            right
                .completed_at
                .cmp(&left.completed_at)
                .then_with(|| right.started_at.cmp(&left.started_at))
                .then_with(|| left.scan_id.cmp(&right.scan_id))
        });
        for (rank, index) in indices.iter().take(2).enumerate() {
            entries[*index].latest_rank = Some(
                u8::try_from(rank + 1)
                    .map_err(|_| HistoryError::new(HistoryErrorKind::InternalState))?,
            );
        }
    }
    Ok(())
}

fn calculate_totals(
    storage: &SnapshotStoreInventoryLease,
    entries: &[SnapshotRetentionEntry],
    orphans: &[SnapshotOrphanFinal],
    temps: &[SnapshotTemporaryObservation],
    remaining_pins: &SnapshotReviewPinPopulation,
) -> Result<SnapshotRetentionTotals, HistoryError> {
    let mut totals = SnapshotRetentionTotals::default();
    let controls = storage.control_usage();
    totals
        .store_marker
        .checked_add_file(controls.store_marker())?;
    totals
        .writer_lock
        .checked_add_file(controls.writer_lock())?;
    totals.controls = totals.store_marker;
    totals.controls.checked_add_usage(totals.writer_lock)?;
    for entry in entries {
        totals.active_pin_rows = totals
            .active_pin_rows
            .checked_add(entry.pins.active)
            .ok_or_else(limit)?;
        totals.expired_pin_rows = totals
            .expired_pin_rows
            .checked_add(entry.pins.expired)
            .ok_or_else(limit)?;
        match entry.logical_state {
            SnapshotRetentionLogicalState::Available => {
                totals.available.checked_add_file(entry.usage)?;
                if entry.is_policy_protected() {
                    totals.protected.checked_add_file(entry.usage)?;
                } else {
                    totals.eligible.checked_add_file(entry.usage)?;
                }
            }
            SnapshotRetentionLogicalState::Tombstoned { .. } => {
                totals.tombstoned_residual.checked_add_file(entry.usage)?;
            }
        }
    }
    for orphan in orphans {
        totals.orphan.checked_add_file(orphan.usage)?;
    }
    for temp in temps {
        match temp.state {
            SnapshotTemporaryState::Active => {
                totals.temporary_active.checked_add_file(temp.usage)?;
            }
            SnapshotTemporaryState::QuiescentAtObservation => {
                totals.temporary_quiescent.checked_add_file(temp.usage)?;
            }
            SnapshotTemporaryState::Unleased => {
                totals.temporary_unleased.checked_add_file(temp.usage)?;
            }
        }
    }
    for summary in remaining_pins.by_scan.values() {
        totals.active_pin_rows = totals
            .active_pin_rows
            .checked_add(summary.active)
            .ok_or_else(limit)?;
        totals.expired_pin_rows = totals
            .expired_pin_rows
            .checked_add(summary.expired)
            .ok_or_else(limit)?;
    }
    if totals.active_pin_rows != remaining_pins.active
        || totals.expired_pin_rows != remaining_pins.expired
    {
        return Err(HistoryError::new(HistoryErrorKind::InternalState));
    }
    totals.store_total = totals.controls;
    let mut classified_entries = SnapshotRetentionUsage::default();
    for usage in [
        totals.available,
        totals.tombstoned_residual,
        totals.orphan,
        totals.temporary_active,
        totals.temporary_quiescent,
        totals.temporary_unleased,
    ] {
        classified_entries.checked_add_usage(usage)?;
        totals.store_total.checked_add_usage(usage)?;
    }
    // Storage independently checked the same complete physical sum. This
    // cross-check catches accidental classification omission or double count.
    let observed = storage.total_usage();
    let observed_entries = storage.entries_usage();
    if classified_entries.logical_bytes != observed_entries.logical_bytes()
        || classified_entries.allocated_bytes != observed_entries.allocated_bytes()
        || classified_entries.charged_bytes != observed_entries.charged_bytes()
    {
        return Err(HistoryError::new(HistoryErrorKind::InternalState));
    }
    if totals.store_total.logical_bytes != observed.logical_bytes()
        || totals.store_total.allocated_bytes != observed.allocated_bytes()
        || totals.store_total.charged_bytes != observed.charged_bytes()
    {
        return Err(HistoryError::new(HistoryErrorKind::InternalState));
    }
    Ok(totals)
}

fn decode_catalog_row(
    raw: RawCatalogRow,
    expected_file_name: &SnapshotFileName,
) -> Result<CatalogRow, HistoryError> {
    let scan_id = ScanId::new(raw.scan_id).map_err(|_| corrupt())?;
    if SnapshotFileName::from_scan_id(scan_id.as_str().as_bytes()) != *expected_file_name {
        return Err(corrupt());
    }
    let root_encoded = EncodedBytes {
        bytes: raw.root,
        encoding: StoredEncoding::host_path_from_stored(raw.root_encoding)
            .map_err(|_| corrupt())?,
    };
    let root = decode_host_path(&root_encoded).map_err(|_| corrupt())?;
    if validate_scan_root(&root).is_err()
        || encode_host_path(&root).map_err(|_| corrupt())? != root_encoded
    {
        return Err(corrupt());
    }
    let started_at = unix_ms_to_system_time(raw.started_at_unix_ms)?;
    let completed_at = unix_ms_to_system_time(raw.completed_at_unix_ms)?;
    if completed_at < started_at || raw.status != "succeeded" {
        return Err(corrupt());
    }
    let version = u32::try_from(raw.snapshot_version)
        .ok()
        .filter(|version| *version > 0)
        .ok_or_else(corrupt)?;
    let path_encoded = EncodedBytes {
        bytes: raw.snapshot_relative_path,
        encoding: StoredEncoding::host_path_from_stored(raw.snapshot_relative_path_encoding)
            .map_err(|_| corrupt())?,
    };
    let relative_path = decode_host_path(&path_encoded).map_err(|_| corrupt())?;
    if encode_host_path(&relative_path).map_err(|_| corrupt())? != path_encoded {
        return Err(corrupt());
    }
    let mut components = relative_path.components();
    let Some(Component::Normal(file_name)) = components.next() else {
        return Err(corrupt());
    };
    if components.next().is_some() || file_name != expected_file_name.as_str() {
        return Err(corrupt());
    }
    let digest: [u8; 32] = raw
        .snapshot_checksum_sha256
        .try_into()
        .map_err(|_| corrupt())?;
    let reference =
        SnapshotReference::from_stored(&scan_id, version, expected_file_name.as_str(), digest)
            .map_err(|_| corrupt())?;
    let logical_state = raw
        .tombstone
        .map(|tombstone| decode_tombstone(tombstone, &reference, raw.completed_at_unix_ms))
        .transpose()?
        .unwrap_or(SnapshotRetentionLogicalState::Available);
    Ok(CatalogRow {
        scan_id,
        root,
        root_key: SnapshotRootKey {
            encoding: raw.root_encoding,
            bytes: root_encoded.bytes,
        },
        started_at,
        completed_at,
        reference,
        logical_state,
    })
}

fn decode_tombstone(
    raw: RawTombstone,
    reference: &SnapshotReference,
    completed_at_unix_ms: i64,
) -> Result<SnapshotRetentionLogicalState, HistoryError> {
    if raw.record_format_version != 1
        || raw.scan_id != reference.scan_id().as_str()
        || raw.scan_status != "succeeded"
        || raw.completed_at_unix_ms != completed_at_unix_ms
        || raw.snapshot_version != i64::from(reference.version())
        || raw.committed_at_unix_ms < completed_at_unix_ms
    {
        return Err(corrupt());
    }
    let expected_path =
        encode_host_path(Path::new(reference.file_name().as_str())).map_err(|_| corrupt())?;
    if raw.snapshot_relative_path != expected_path.bytes
        || raw.snapshot_relative_path_encoding != expected_path.encoding as i64
        || raw.snapshot_checksum_sha256.as_slice() != reference.digest().bytes()
    {
        return Err(corrupt());
    }
    Ok(SnapshotRetentionLogicalState::Tombstoned {
        committed_at: unix_ms_to_system_time(raw.committed_at_unix_ms)?,
    })
}

fn raw_catalog_row(row: &Row<'_>) -> rusqlite::Result<RawCatalogRow> {
    require_type_length(row, 0, 1, "text", 1, MAX_ID_BYTES)?;
    require_type_length(row, 3, 4, "blob", 1, MAX_PATH_BYTES)?;
    require_type(row, 6, "integer")?;
    require_type(row, 8, "integer")?;
    require_type(row, 10, "integer")?;
    require_type_length(row, 12, 13, "text", 1, MAX_STATUS_BYTES)?;
    require_type(row, 15, "integer")?;
    require_type_length(row, 17, 18, "blob", 1, MAX_PATH_BYTES)?;
    require_type(row, 20, "integer")?;
    require_type_length(row, 22, 23, "blob", 32, 32)?;
    let tombstone_type: String = row.get(25)?;
    let tombstone = if tombstone_type == "null" {
        for column in [28_usize, 30, 33, 35, 37, 40, 42, 45] {
            if row.get::<_, String>(column)? != "null" {
                return Err(rusqlite::Error::InvalidQuery);
            }
        }
        None
    } else {
        require_type_length(row, 25, 26, "text", 1, MAX_ID_BYTES)?;
        require_type(row, 28, "integer")?;
        require_type_length(row, 30, 31, "text", 1, MAX_STATUS_BYTES)?;
        require_type(row, 33, "integer")?;
        require_type(row, 35, "integer")?;
        require_type_length(row, 37, 38, "blob", 1, MAX_PATH_BYTES)?;
        require_type(row, 40, "integer")?;
        require_type_length(row, 42, 43, "blob", 32, 32)?;
        require_type(row, 45, "integer")?;
        Some(RawTombstone {
            scan_id: row.get(27)?,
            record_format_version: row.get(29)?,
            scan_status: row.get(32)?,
            completed_at_unix_ms: row.get(34)?,
            snapshot_version: row.get(36)?,
            snapshot_relative_path: row.get(39)?,
            snapshot_relative_path_encoding: row.get(41)?,
            snapshot_checksum_sha256: row.get(44)?,
            committed_at_unix_ms: row.get(46)?,
        })
    };
    Ok(RawCatalogRow {
        scan_id: row.get(2)?,
        root: row.get(5)?,
        root_encoding: row.get(7)?,
        started_at_unix_ms: row.get(9)?,
        completed_at_unix_ms: row.get(11)?,
        status: row.get(14)?,
        snapshot_version: row.get(16)?,
        snapshot_relative_path: row.get(19)?,
        snapshot_relative_path_encoding: row.get(21)?,
        snapshot_checksum_sha256: row.get(24)?,
        tombstone,
    })
}

const CATALOG_BY_SNAPSHOT_PATH: &str = "SELECT
    typeof(scan.scan_id), length(CAST(scan.scan_id AS BLOB)), scan.scan_id,
    typeof(scan.root_path), length(scan.root_path), scan.root_path,
    typeof(scan.root_path_encoding), scan.root_path_encoding,
    typeof(scan.started_at_unix_ms), scan.started_at_unix_ms,
    typeof(scan.completed_at_unix_ms), scan.completed_at_unix_ms,
    typeof(scan.status), length(CAST(scan.status AS BLOB)), scan.status,
    typeof(scan.snapshot_version), scan.snapshot_version,
    typeof(scan.snapshot_relative_path), length(scan.snapshot_relative_path),
    scan.snapshot_relative_path,
    typeof(scan.snapshot_relative_path_encoding), scan.snapshot_relative_path_encoding,
    typeof(scan.snapshot_checksum_sha256), length(scan.snapshot_checksum_sha256),
    scan.snapshot_checksum_sha256,
    typeof(tombstone.scan_id), length(CAST(tombstone.scan_id AS BLOB)), tombstone.scan_id,
    typeof(tombstone.record_format_version), tombstone.record_format_version,
    typeof(tombstone.scan_status), length(CAST(tombstone.scan_status AS BLOB)),
    tombstone.scan_status,
    typeof(tombstone.completed_at_unix_ms), tombstone.completed_at_unix_ms,
    typeof(tombstone.snapshot_version), tombstone.snapshot_version,
    typeof(tombstone.snapshot_relative_path), length(tombstone.snapshot_relative_path),
    tombstone.snapshot_relative_path,
    typeof(tombstone.snapshot_relative_path_encoding),
    tombstone.snapshot_relative_path_encoding,
    typeof(tombstone.snapshot_checksum_sha256), length(tombstone.snapshot_checksum_sha256),
    tombstone.snapshot_checksum_sha256,
    typeof(tombstone.committed_at_unix_ms), tombstone.committed_at_unix_ms
 FROM scans AS scan
 LEFT JOIN snapshot_retention_tombstones AS tombstone ON tombstone.scan_id = scan.scan_id
 WHERE scan.snapshot_relative_path_encoding = ?1
   AND scan.snapshot_relative_path = ?2
 ORDER BY scan.scan_id ASC
 LIMIT 2";

fn require_type(row: &Row<'_>, column: usize, expected: &str) -> rusqlite::Result<()> {
    let actual: String = row.get(column)?;
    if actual == expected {
        Ok(())
    } else {
        Err(rusqlite::Error::InvalidQuery)
    }
}

fn require_type_length(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
    expected_type: &str,
    minimum: i64,
    maximum: i64,
) -> rusqlite::Result<()> {
    require_type(row, type_column, expected_type)?;
    let length: i64 = row.get(length_column)?;
    if (minimum..=maximum).contains(&length) {
        Ok(())
    } else {
        Err(rusqlite::Error::InvalidQuery)
    }
}

fn checked_add(left: u64, right: u64) -> Result<u64, HistoryError> {
    left.checked_add(right).ok_or_else(limit)
}

const fn limit() -> HistoryError {
    HistoryError::new(HistoryErrorKind::QueryLimitExceeded)
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_root_ranking_and_eviction_order_are_deterministic() {
        fn entry(scan: &str, root: &[u8], started: u64, completed: u64) -> SnapshotRetentionEntry {
            let scan_id = ScanId::new(scan).unwrap();
            let file_name = SnapshotFileName::from_scan_id(scan.as_bytes());
            SnapshotRetentionEntry {
                scan_id: scan_id.clone(),
                root: PathBuf::from("/fixture"),
                started_at: UNIX_EPOCH + std::time::Duration::from_millis(started),
                completed_at: UNIX_EPOCH + std::time::Duration::from_millis(completed),
                reference: SnapshotReference::from_stored(
                    &scan_id,
                    1,
                    file_name.as_str(),
                    SnapshotDigest::from_bytes([1; 32]).bytes(),
                )
                .unwrap(),
                usage: SnapshotFileUsage::default(),
                logical_state: SnapshotRetentionLogicalState::Available,
                pins: SnapshotReviewPinSummary::default(),
                latest_rank: None,
                root_key: SnapshotRootKey {
                    encoding: 1,
                    bytes: root.to_vec(),
                },
            }
        }

        let mut entries = vec![
            entry("scan:c", b"/same", 3, 10),
            entry("scan:b", b"/same", 2, 10),
            entry("scan:a", b"/same", 1, 9),
            entry("scan:z", b"/other", 1, 1),
        ];
        assign_latest_two(&mut entries).unwrap();
        assert_eq!(entries[0].latest_rank, Some(1));
        assert_eq!(entries[1].latest_rank, Some(2));
        assert_eq!(entries[2].latest_rank, None);
        assert_eq!(entries[3].latest_rank, Some(1));
    }
}
