use std::path::PathBuf;
use std::time::SystemTime;

use thiserror::Error;

use super::task::{
    DurableCandidateEvaluation, DurableScanSummary, ScanRootErrorKind, TaskId, TaskPhase,
};
use crate::domain::VolumeId;
use crate::persistence::{HistoryErrorKind, StoredPressureEpisode};

/// Maximum number of adjacent pressure episodes inspected for one targeted
/// recommendation admission.
pub const MAX_TARGETED_PRESSURE_CHAIN_EPISODES: usize = 64;

/// Per-root and aggregate retained-node bounds for one complete configured
/// registry pass. Admission divides the aggregate cap across every configured
/// root, with the per-root ceiling applied for smaller registries.
pub const MAX_TARGETED_PROJECT_SCAN_NODES: usize = 50_000;
pub const MAX_TARGETED_PROJECT_SCAN_PASS_NODES: usize = 200_000;

/// Current durable low-space level proven at the caller's exact accepted
/// capacity observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetedProjectScanPressure {
    Warning,
    Critical,
}

/// Stable pressure context shared by every selected-root outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedProjectScanPressureContext {
    pub volume_id: VolumeId,
    pub capacity_anchor: SystemTime,
    pub pressure: TargetedProjectScanPressure,
    pub current_episode_started_at: SystemTime,
    pub pressure_started_at: SystemTime,
    pub policy_revision: u64,
}

/// One lossless configured-root selection. The path was loaded from DUX's
/// authoritative settings store; callers never supply a path to admission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedProjectScanSelection {
    pub ordinal: u16,
    pub root: PathBuf,
    pub max_nodes: u32,
}

/// Fully validated durable observation returned instead of starting another
/// scan for the same root and contiguous low-pressure interval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedProjectScanCurrent {
    pub scan: DurableScanSummary,
    pub candidate_evaluation: DurableCandidateEvaluation,
}

/// Observation-only result of one bounded configured-root admission.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TargetedProjectScanDisposition {
    EmptyRegistry,
    NoPressure,
    RootUnavailable { reason: ScanRootErrorKind },
    ExistingTask { task_id: TaskId, phase: TaskPhase },
    Current(Box<TargetedProjectScanCurrent>),
    Started { task_id: TaskId },
}

/// One admission response. `selection` is present for every valid ordinal,
/// including no-pressure and unavailable-root outcomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedProjectScanAdmission {
    pub configured_roots_revision: u64,
    pub root_count: u16,
    pub selection: Option<TargetedProjectScanSelection>,
    pub pressure: Option<TargetedProjectScanPressureContext>,
    pub disposition: TargetedProjectScanDisposition,
}

/// Path-free proof that the registry and current low-pressure episode still
/// match after a bounded caller finishes its selected-root pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedProjectScanCheckpoint {
    pub configured_roots_revision: u64,
    pub root_count: u16,
    pub pressure: TargetedProjectScanPressureContext,
}

/// Stable failure taxonomy for the global admission boundary. Root-local
/// filesystem failures are dispositions so a bounded caller can continue to
/// later ordinals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum TargetedProjectScanError {
    #[error("engine session is closed")]
    Closed,
    #[error("the durable engine store is read-only")]
    ReadOnlyStore,
    #[error(
        "configured project roots changed from revision {expected_revision} to {actual_revision}"
    )]
    RegistryChanged {
        expected_revision: u64,
        actual_revision: u64,
    },
    #[error("configured project root ordinal {ordinal} is outside the {root_count} roots")]
    InvalidOrdinal { ordinal: u16, root_count: u16 },
    #[error("capacity anchor is not an exact accepted durable observation")]
    InvalidAnchor,
    #[error("the low-pressure context changed during the targeted scan pass")]
    PressureChanged,
    #[error("durable schema is incompatible")]
    IncompatibleSchema,
    #[error("operation is temporarily busy")]
    Busy,
    #[error("storage failed its safety checks")]
    UnsafeStorage,
    #[error("bounded operation exceeded its resource budget")]
    BudgetExceeded,
    #[error("durable targeted-scan state is corrupt")]
    CorruptData,
    #[error("durable targeted-scan state is unavailable")]
    Unavailable,
    #[error("operation outcome is unknown")]
    OutcomeUnknown,
    #[error("engine task queue is full")]
    QueueFull,
    #[error("engine task identifiers are exhausted")]
    TaskIdExhausted,
    #[error("internal engine state is unavailable")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LowPressureChain {
    pub(super) pressure: TargetedProjectScanPressure,
    pub(super) current_episode_started_at: SystemTime,
    pub(super) started_at: SystemTime,
    pub(super) policy_revision: u64,
}

pub(super) fn targeted_scan_node_limit(root_count: u16) -> usize {
    if root_count == 0 {
        return 0;
    }
    (MAX_TARGETED_PROJECT_SCAN_PASS_NODES / usize::from(root_count))
        .clamp(1, MAX_TARGETED_PROJECT_SCAN_NODES)
}

pub(super) fn derive_low_pressure_chain(
    episodes: &[StoredPressureEpisode],
    anchor: SystemTime,
) -> Result<Option<LowPressureChain>, HistoryErrorKind> {
    let Some(current) = episodes.first() else {
        return Ok(None);
    };
    if current.entered_at > anchor || current.exited_at.is_some() {
        return Ok(None);
    }
    let pressure = match current.pressure {
        crate::domain::DiskPressure::Warning => TargetedProjectScanPressure::Warning,
        crate::domain::DiskPressure::Critical => TargetedProjectScanPressure::Critical,
        crate::domain::DiskPressure::Healthy | crate::domain::DiskPressure::Unknown => {
            return Err(HistoryErrorKind::CorruptData);
        }
    };
    let mut started_at = current.entered_at;
    let mut previous = current;
    let mut chain_len = 1_usize;
    for older in episodes.iter().skip(1) {
        let adjacent = older.exited_at == Some(previous.entered_at)
            && older.policy_revision == current.policy_revision;
        if !adjacent {
            break;
        }
        chain_len += 1;
        if chain_len > MAX_TARGETED_PRESSURE_CHAIN_EPISODES {
            return Err(HistoryErrorKind::QueryLimitExceeded);
        }
        started_at = older.entered_at;
        previous = older;
    }
    Ok(Some(LowPressureChain {
        pressure,
        current_episode_started_at: current.entered_at,
        started_at,
        policy_revision: current.policy_revision,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DiskPressure;
    use std::time::{Duration, UNIX_EPOCH};

    fn episode(
        pressure: DiskPressure,
        entered_ms: u64,
        exited_ms: Option<u64>,
        revision: u64,
    ) -> StoredPressureEpisode {
        StoredPressureEpisode {
            volume_id: VolumeId::new("volume:targeted-chain").unwrap(),
            pressure,
            entered_at: UNIX_EPOCH + Duration::from_millis(entered_ms),
            exited_at: exited_ms.map(|value| UNIX_EPOCH + Duration::from_millis(value)),
            policy_revision: revision,
        }
    }

    #[test]
    fn derives_adjacent_same_policy_low_pressure_chain() {
        let episodes = vec![
            episode(DiskPressure::Critical, 30, None, 7),
            episode(DiskPressure::Warning, 10, Some(30), 7),
            episode(DiskPressure::Warning, 1, Some(9), 7),
        ];
        let chain =
            derive_low_pressure_chain(&episodes, UNIX_EPOCH + Duration::from_millis(40)).unwrap();
        assert_eq!(
            chain,
            Some(LowPressureChain {
                pressure: TargetedProjectScanPressure::Critical,
                current_episode_started_at: UNIX_EPOCH + Duration::from_millis(30),
                started_at: UNIX_EPOCH + Duration::from_millis(10),
                policy_revision: 7,
            })
        );
    }

    #[test]
    fn rejects_a_contiguous_chain_beyond_the_fixed_bound() {
        let episodes = (0..=MAX_TARGETED_PRESSURE_CHAIN_EPISODES)
            .map(|index| {
                let entered = u64::try_from(MAX_TARGETED_PRESSURE_CHAIN_EPISODES - index).unwrap();
                episode(
                    DiskPressure::Warning,
                    entered,
                    (index != 0).then_some(entered + 1),
                    3,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            derive_low_pressure_chain(
                &episodes,
                UNIX_EPOCH
                    + Duration::from_millis(
                        u64::try_from(MAX_TARGETED_PRESSURE_CHAIN_EPISODES + 1).unwrap()
                    )
            ),
            Err(HistoryErrorKind::QueryLimitExceeded)
        );
    }

    #[test]
    fn aggregate_node_budget_scales_across_the_bounded_registry() {
        assert_eq!(targeted_scan_node_limit(1), 50_000);
        assert_eq!(targeted_scan_node_limit(4), 50_000);
        assert_eq!(targeted_scan_node_limit(16), 12_500);
        assert_eq!(targeted_scan_node_limit(0), 0);
    }
}
