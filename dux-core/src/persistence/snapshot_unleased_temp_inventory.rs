//! Bounded classification of marker-owned snapshot temps without lease rows.
//!
//! This is a physical namespace observation, not adoption of a temp and not
//! cleanup authority by itself. The repository must retain the current-schema
//! database guard and snapshot writer lease, then repeat the selected physical
//! proof at its effect boundary.

use super::history::{HistoryError, HistoryErrorKind};
use super::snapshot::{
    SnapshotFileUsage, SnapshotInventoryEntryKind, SnapshotStoreInventoryLease,
    SnapshotTempKernelState,
};
use super::snapshot_temp_lease::SnapshotTempLeasePopulation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SnapshotUnleasedTempPhysicalState {
    Active(SnapshotFileUsage),
    Quiescent(SnapshotFileUsage),
}

impl SnapshotUnleasedTempPhysicalState {
    #[cfg(test)]
    pub(super) const fn usage(self) -> SnapshotFileUsage {
        match self {
            Self::Active(usage) | Self::Quiescent(usage) => usage,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SnapshotUnleasedTempObservation {
    pub(super) name: String,
    pub(super) state: SnapshotUnleasedTempPhysicalState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SnapshotUnleasedTempInventory {
    pub(super) entries: Vec<SnapshotUnleasedTempObservation>,
    pub(super) active_count: u32,
    pub(super) charged_bytes: u64,
}

impl SnapshotUnleasedTempInventory {
    pub(super) fn first_actionable(&self) -> Option<&SnapshotUnleasedTempObservation> {
        self.entries
            .iter()
            .find(|entry| matches!(entry.state, SnapshotUnleasedTempPhysicalState::Quiescent(_)))
    }
}

/// Subtract the complete bounded lease-name population from the complete
/// bounded physical store observation. Matching is exact and case-sensitive.
/// Entries are ordered by their recognized ASCII names so an active entry can
/// be skipped without starving a later quiescent entry.
pub(super) fn build_snapshot_unleased_temp_inventory(
    storage: &SnapshotStoreInventoryLease,
    leases: &SnapshotTempLeasePopulation,
) -> Result<SnapshotUnleasedTempInventory, HistoryError> {
    let mut entries = Vec::new();
    let mut active_count = 0_u32;
    let mut charged_bytes = 0_u64;

    for physical in storage.entries() {
        if !matches!(physical.kind(), SnapshotInventoryEntryKind::RecognizedTemp)
            || leases
                .rows()
                .iter()
                .any(|lease| lease.temp_name() == physical.name())
        {
            continue;
        }

        let usage = physical.usage();
        charged_bytes = charged_bytes
            .checked_add(usage.charged_bytes())
            .ok_or_else(limit)?;
        let state = match physical.temp_kernel_state() {
            Some(SnapshotTempKernelState::Active) => {
                active_count = active_count.checked_add(1).ok_or_else(limit)?;
                SnapshotUnleasedTempPhysicalState::Active(usage)
            }
            Some(SnapshotTempKernelState::Quiescent) => {
                SnapshotUnleasedTempPhysicalState::Quiescent(usage)
            }
            None => return Err(corrupt()),
        };
        entries.push(SnapshotUnleasedTempObservation {
            name: physical.name().to_owned(),
            state,
        });
    }

    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(SnapshotUnleasedTempInventory {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn usage() -> SnapshotFileUsage {
        SnapshotFileUsage::default()
    }

    #[test]
    fn first_actionable_skips_active_entries_without_reordering() {
        let inventory = SnapshotUnleasedTempInventory {
            entries: vec![
                SnapshotUnleasedTempObservation {
                    name: ".snapshot-a.tmp".to_owned(),
                    state: SnapshotUnleasedTempPhysicalState::Active(usage()),
                },
                SnapshotUnleasedTempObservation {
                    name: ".snapshot-b.tmp".to_owned(),
                    state: SnapshotUnleasedTempPhysicalState::Quiescent(usage()),
                },
            ],
            active_count: 1,
            charged_bytes: 0,
        };

        let actionable = inventory.first_actionable().unwrap();
        assert_eq!(actionable.name, ".snapshot-b.tmp");
        assert_eq!(actionable.state.usage(), usage());
    }

    #[test]
    fn active_only_and_empty_inventories_have_no_actionable_entry() {
        let active = SnapshotUnleasedTempInventory {
            entries: vec![SnapshotUnleasedTempObservation {
                name: ".snapshot-a.tmp".to_owned(),
                state: SnapshotUnleasedTempPhysicalState::Active(usage()),
            }],
            active_count: 1,
            charged_bytes: 0,
        };

        assert!(active.first_actionable().is_none());
        assert!(
            SnapshotUnleasedTempInventory::default()
                .first_actionable()
                .is_none()
        );
    }
}
