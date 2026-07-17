//! Bounded classification of row-bound terminal snapshot temporary debt.
//!
//! This module deliberately ignores unleased temporary files, provisioning
//! stages, final snapshots, cap policy, pins, and tombstones. Its output is an
//! observation only; the repository must still fully decode the selected scan
//! parent and repeat the physical proof under the retained locks.

use super::history::{HistoryError, HistoryErrorKind};
use super::snapshot::{
    SnapshotFileUsage, SnapshotInventoryEntryKind, SnapshotStoreInventoryLease,
    SnapshotTempKernelState,
};
use super::snapshot_temp_lease::{
    SnapshotTempLeaseParentStatus, SnapshotTempLeasePopulation, StoredSnapshotTempLease,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapshotTerminalTempPhysicalState {
    Missing,
    Active,
    Quiescent(SnapshotFileUsage),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SnapshotTerminalTempObservation {
    pub(super) lease: StoredSnapshotTempLease,
    pub(super) physical: SnapshotTerminalTempPhysicalState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SnapshotTerminalTempInventory {
    pub(super) entries: Vec<SnapshotTerminalTempObservation>,
    pub(super) active_count: u32,
    pub(super) charged_bytes: u64,
}

impl SnapshotTerminalTempInventory {
    pub(super) fn first_actionable(&self) -> Option<&SnapshotTerminalTempObservation> {
        self.entries
            .iter()
            .find(|entry| !matches!(entry.physical, SnapshotTerminalTempPhysicalState::Active))
    }
}

/// Match the complete bounded immutable lease population to the complete
/// bounded physical store observation. Only exact terminal-parent rows enter
/// this inventory. A physical recognized temp with no row remains deliberately
/// unclassified and cannot affect selection or grant removal authority.
pub(super) fn build_snapshot_terminal_temp_inventory(
    storage: &SnapshotStoreInventoryLease,
    leases: SnapshotTempLeasePopulation,
) -> Result<SnapshotTerminalTempInventory, HistoryError> {
    let mut entries = Vec::new();
    let mut active_count = 0_u32;
    let mut charged_bytes = 0_u64;

    for lease in leases.rows() {
        if !matches!(
            lease.parent_status(),
            SnapshotTempLeaseParentStatus::Failed
                | SnapshotTempLeaseParentStatus::Cancelled
                | SnapshotTempLeaseParentStatus::Interrupted
        ) {
            continue;
        }
        let physical = match storage
            .entries()
            .iter()
            .find(|entry| entry.name() == lease.temp_name())
        {
            None => SnapshotTerminalTempPhysicalState::Missing,
            Some(entry) => {
                if !matches!(entry.kind(), SnapshotInventoryEntryKind::RecognizedTemp) {
                    return Err(corrupt());
                }
                let usage = entry.usage();
                charged_bytes = charged_bytes
                    .checked_add(usage.charged_bytes())
                    .ok_or_else(limit)?;
                match entry.temp_kernel_state() {
                    Some(SnapshotTempKernelState::Active) => {
                        active_count = active_count.checked_add(1).ok_or_else(limit)?;
                        SnapshotTerminalTempPhysicalState::Active
                    }
                    Some(SnapshotTempKernelState::Quiescent) => {
                        SnapshotTerminalTempPhysicalState::Quiescent(usage)
                    }
                    None => return Err(corrupt()),
                }
            }
        };
        entries.push(SnapshotTerminalTempObservation {
            lease: lease.clone(),
            physical,
        });
    }

    entries.sort_by(|left, right| {
        left.lease
            .created_at_unix_ms()
            .cmp(&right.lease.created_at_unix_ms())
            .then_with(|| left.lease.scan_id().cmp(right.lease.scan_id()))
            .then_with(|| left.lease.id().as_str().cmp(right.lease.id().as_str()))
    });
    Ok(SnapshotTerminalTempInventory {
        entries,
        active_count,
        charged_bytes,
    })
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

const fn limit() -> HistoryError {
    HistoryError::new(HistoryErrorKind::QueryLimitExceeded)
}
