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

/// Fixed v1 targeted-reclaim root policy.
///
/// The only code-owned root in this revision is the current macOS account's
/// OS-derived `Library/Caches` directory. Configured project roots remain
/// separately revisioned durable settings.
pub const TARGETED_RECLAIM_ROOT_POLICY_REVISION: u32 = 1;
pub const MAX_TARGETED_CONFIGURED_PROJECT_ROOTS: u16 = 16;
pub const MAX_TARGETED_RECLAIM_ROOTS: u16 = MAX_TARGETED_CONFIGURED_PROJECT_ROOTS + 1;
pub const MAX_TARGETED_USER_LIBRARY_CACHES_SCAN_NODES: usize = 100_000;
pub const MIN_TARGETED_CONFIGURED_PROJECT_SCAN_NODES: usize = 10_000;

/// Stable source taxonomy for the unified targeted-reclaim catalog.
///
/// Ordering is not inferred from the discriminant. A catalog layout always
/// places the known cache root first, followed by configured roots in their
/// authoritative stored order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TargetedReclaimRootKind {
    KnownUserLibraryCaches,
    ConfiguredProject,
}

/// Path-free identity for one exact derived targeted-root catalog.
///
/// The digest is computed by the engine from the ordered source tags, lossless
/// roots, retained filesystem identities, and both policy revisions. Callers
/// may only echo this observation; it grants no path, plan, or effect authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetedReclaimRootCatalogStamp {
    pub known_roots_policy_revision: u32,
    pub configured_roots_revision: u64,
    pub known_user_library_caches_included: bool,
    pub root_count: u16,
    pub digest_sha256: [u8; 32],
}

/// One path-free slot in the deterministic catalog layout. The configured
/// ordinal is populated only for `ConfiguredProject`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetedReclaimRootCatalogSlot {
    pub ordinal: u16,
    pub kind: TargetedReclaimRootKind,
    pub configured_root_ordinal: Option<u16>,
    pub max_nodes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetedReclaimScanBudget {
    pub known_user_library_caches_max_nodes: u32,
    pub configured_project_max_nodes: u32,
    pub configured_project_root_count: u16,
    pub aggregate_max_nodes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum TargetedReclaimCatalogError {
    #[error("the configured project root count exceeds the targeted-reclaim bound")]
    TooManyConfiguredRoots,
    #[error("targeted-reclaim budget arithmetic overflowed")]
    BudgetOverflow,
}

/// Allocate one deterministic bounded pass.
///
/// The broad known cache root receives at most 100,000 nodes first, while each
/// configured project retains at least 10,000 nodes at the maximum 16-root
/// registry size. Configured roots remain capped at 50,000 nodes apiece. The
/// returned aggregate can be below 200,000 when all per-root caps are reached.
pub fn targeted_reclaim_scan_budget(
    configured_root_count: u16,
    include_known_user_library_caches: bool,
) -> Result<TargetedReclaimScanBudget, TargetedReclaimCatalogError> {
    if configured_root_count > MAX_TARGETED_CONFIGURED_PROJECT_ROOTS {
        return Err(TargetedReclaimCatalogError::TooManyConfiguredRoots);
    }
    let configured_count = usize::from(configured_root_count);
    let known_max_nodes = if include_known_user_library_caches {
        let configured_reservation = configured_count
            .checked_mul(MIN_TARGETED_CONFIGURED_PROJECT_SCAN_NODES)
            .ok_or(TargetedReclaimCatalogError::BudgetOverflow)?;
        MAX_TARGETED_USER_LIBRARY_CACHES_SCAN_NODES.min(
            MAX_TARGETED_PROJECT_SCAN_PASS_NODES
                .checked_sub(configured_reservation)
                .ok_or(TargetedReclaimCatalogError::BudgetOverflow)?,
        )
    } else {
        0
    };
    let configured_max_nodes = if configured_count == 0 {
        0
    } else {
        let remaining = MAX_TARGETED_PROJECT_SCAN_PASS_NODES
            .checked_sub(known_max_nodes)
            .ok_or(TargetedReclaimCatalogError::BudgetOverflow)?;
        remaining
            .checked_div(configured_count)
            .ok_or(TargetedReclaimCatalogError::BudgetOverflow)?
            .clamp(1, MAX_TARGETED_PROJECT_SCAN_NODES)
    };
    let aggregate_max_nodes = known_max_nodes
        .checked_add(
            configured_max_nodes
                .checked_mul(configured_count)
                .ok_or(TargetedReclaimCatalogError::BudgetOverflow)?,
        )
        .ok_or(TargetedReclaimCatalogError::BudgetOverflow)?;
    if aggregate_max_nodes > MAX_TARGETED_PROJECT_SCAN_PASS_NODES {
        return Err(TargetedReclaimCatalogError::BudgetOverflow);
    }
    Ok(TargetedReclaimScanBudget {
        known_user_library_caches_max_nodes: u32::try_from(known_max_nodes)
            .map_err(|_| TargetedReclaimCatalogError::BudgetOverflow)?,
        configured_project_max_nodes: u32::try_from(configured_max_nodes)
            .map_err(|_| TargetedReclaimCatalogError::BudgetOverflow)?,
        configured_project_root_count: configured_root_count,
        aggregate_max_nodes: u32::try_from(aggregate_max_nodes)
            .map_err(|_| TargetedReclaimCatalogError::BudgetOverflow)?,
    })
}

/// Produce the path-free cache-first slot layout that the engine later joins
/// with its private derived/stored paths.
pub fn targeted_reclaim_root_catalog_layout(
    configured_root_count: u16,
    include_known_user_library_caches: bool,
) -> Result<Vec<TargetedReclaimRootCatalogSlot>, TargetedReclaimCatalogError> {
    let budget =
        targeted_reclaim_scan_budget(configured_root_count, include_known_user_library_caches)?;
    let root_count = configured_root_count
        .checked_add(u16::from(include_known_user_library_caches))
        .ok_or(TargetedReclaimCatalogError::BudgetOverflow)?;
    let mut slots = Vec::new();
    slots
        .try_reserve_exact(usize::from(root_count))
        .map_err(|_| TargetedReclaimCatalogError::BudgetOverflow)?;
    if include_known_user_library_caches {
        slots.push(TargetedReclaimRootCatalogSlot {
            ordinal: 0,
            kind: TargetedReclaimRootKind::KnownUserLibraryCaches,
            configured_root_ordinal: None,
            max_nodes: budget.known_user_library_caches_max_nodes,
        });
    }
    for configured_root_ordinal in 0..configured_root_count {
        let ordinal = configured_root_ordinal
            .checked_add(u16::from(include_known_user_library_caches))
            .ok_or(TargetedReclaimCatalogError::BudgetOverflow)?;
        slots.push(TargetedReclaimRootCatalogSlot {
            ordinal,
            kind: TargetedReclaimRootKind::ConfiguredProject,
            configured_root_ordinal: Some(configured_root_ordinal),
            max_nodes: budget.configured_project_max_nodes,
        });
    }
    Ok(slots)
}

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
    pub kind: TargetedReclaimRootKind,
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
    pub root_catalog: TargetedReclaimRootCatalogStamp,
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
    pub root_catalog: TargetedReclaimRootCatalogStamp,
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
    #[error("the targeted-reclaim root catalog request is incomplete or malformed")]
    InvalidCatalog,
    #[error("the derived targeted-reclaim root catalog changed during the scan pass")]
    CatalogChanged,
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
    fn unified_budget_prioritizes_cache_without_exceeding_the_pass_cap() {
        let cache_only = targeted_reclaim_scan_budget(0, true).unwrap();
        assert_eq!(cache_only.known_user_library_caches_max_nodes, 100_000);
        assert_eq!(cache_only.configured_project_max_nodes, 0);
        assert_eq!(cache_only.aggregate_max_nodes, 100_000);

        let cache_and_one = targeted_reclaim_scan_budget(1, true).unwrap();
        assert_eq!(
            cache_and_one,
            TargetedReclaimScanBudget {
                known_user_library_caches_max_nodes: 100_000,
                configured_project_max_nodes: 50_000,
                configured_project_root_count: 1,
                aggregate_max_nodes: 150_000,
            }
        );

        let full = targeted_reclaim_scan_budget(16, true).unwrap();
        assert_eq!(full.known_user_library_caches_max_nodes, 40_000);
        assert_eq!(full.configured_project_max_nodes, 10_000);
        assert_eq!(full.aggregate_max_nodes, 200_000);

        for configured_count in 0..=MAX_TARGETED_CONFIGURED_PROJECT_ROOTS {
            for include_known in [false, true] {
                let budget = targeted_reclaim_scan_budget(configured_count, include_known).unwrap();
                assert!(
                    usize::try_from(budget.aggregate_max_nodes).unwrap()
                        <= MAX_TARGETED_PROJECT_SCAN_PASS_NODES
                );
                if configured_count > 0 {
                    assert!(
                        usize::try_from(budget.configured_project_max_nodes).unwrap()
                            <= MAX_TARGETED_PROJECT_SCAN_NODES
                    );
                    if include_known {
                        assert!(
                            usize::try_from(budget.configured_project_max_nodes).unwrap()
                                >= MIN_TARGETED_CONFIGURED_PROJECT_SCAN_NODES
                        );
                    }
                }
            }
        }
        assert_eq!(
            targeted_reclaim_scan_budget(MAX_TARGETED_CONFIGURED_PROJECT_ROOTS + 1, true),
            Err(TargetedReclaimCatalogError::TooManyConfiguredRoots)
        );
    }

    #[test]
    fn unified_catalog_layout_is_cache_first_and_preserves_configured_ordinals() {
        let layout = targeted_reclaim_root_catalog_layout(3, true).unwrap();
        assert_eq!(
            layout,
            vec![
                TargetedReclaimRootCatalogSlot {
                    ordinal: 0,
                    kind: TargetedReclaimRootKind::KnownUserLibraryCaches,
                    configured_root_ordinal: None,
                    max_nodes: 100_000,
                },
                TargetedReclaimRootCatalogSlot {
                    ordinal: 1,
                    kind: TargetedReclaimRootKind::ConfiguredProject,
                    configured_root_ordinal: Some(0),
                    max_nodes: 33_333,
                },
                TargetedReclaimRootCatalogSlot {
                    ordinal: 2,
                    kind: TargetedReclaimRootKind::ConfiguredProject,
                    configured_root_ordinal: Some(1),
                    max_nodes: 33_333,
                },
                TargetedReclaimRootCatalogSlot {
                    ordinal: 3,
                    kind: TargetedReclaimRootKind::ConfiguredProject,
                    configured_root_ordinal: Some(2),
                    max_nodes: 33_333,
                },
            ]
        );
        let configured_only = targeted_reclaim_root_catalog_layout(2, false).unwrap();
        assert_eq!(
            configured_only
                .iter()
                .map(|slot| (
                    slot.ordinal,
                    slot.kind,
                    slot.configured_root_ordinal,
                    slot.max_nodes
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    0,
                    TargetedReclaimRootKind::ConfiguredProject,
                    Some(0),
                    50_000
                ),
                (
                    1,
                    TargetedReclaimRootKind::ConfiguredProject,
                    Some(1),
                    50_000
                ),
            ]
        );
    }
}
