use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use thiserror::Error;

use super::{
    DurableCandidateStatus, DurableCandidateSummary, TargetedProjectScanPressureContext,
    TargetedReclaimRootCatalogStamp,
};
use crate::domain::{
    BlockReason, CandidateAction, CandidateCategory, EvidenceKind, RuleRef, SafetyTier, ScanId,
};

/// Version of the deterministic Critical-pressure recovery ordering policy.
pub const EMERGENCY_RECOVERY_POLICY_REVISION: u32 = 1;

/// Current-evidence window relative to the exact requested capacity anchor.
///
/// A scan completed after the anchor while the focused pass was running is
/// current. At or before the anchor, only the latest hour remains current.
pub const EMERGENCY_RECOVERY_MAX_EVIDENCE_AGE: Duration = Duration::from_secs(60 * 60);

/// Maximum number of path-free groups returned by one bounded projection.
pub const MAX_EMERGENCY_RECOVERY_GROUPS: usize = 64;

/// Fixed §13.3 recovery taxonomy. Declaration order has no policy meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EmergencyRecoveryLane {
    EvictableCloud,
    StaleSafeRegenerable,
    TrashInformation,
    ReviewableInstallerArchive,
    LargeFile,
    GuidedExploration,
    PermissionGap,
}

impl EmergencyRecoveryLane {
    /// Stable roadmap priority, independent of Rust enum layout.
    pub const fn priority(self) -> u8 {
        match self {
            Self::EvictableCloud => 0,
            Self::StaleSafeRegenerable => 1,
            Self::TrashInformation => 2,
            Self::ReviewableInstallerArchive => 3,
            Self::LargeFile => 4,
            Self::GuidedExploration => 5,
            Self::PermissionGap => 6,
        }
    }
}

/// One path-free exact-scan navigation source.
///
/// Optional metrics are lane-specific: stale-safe sources carry candidate
/// counts, guided exploration carries none, and permission-gap sources carry
/// only permission issue occurrences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmergencyRecoverySource {
    pub root_ordinal: u16,
    pub scan_id: ScanId,
    pub observed_at: SystemTime,
    pub candidate_count: Option<u32>,
    pub blocked_candidate_count: Option<u32>,
    pub permission_issue_count: Option<u64>,
}

/// One deterministic path-free recommendation group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmergencyRecoveryGroup {
    pub rank: u16,
    pub lane: EmergencyRecoveryLane,
    pub rule: Option<RuleRef>,
    pub category: Option<CandidateCategory>,
    pub unavailable_root_count: u16,
    pub sources: Vec<EmergencyRecoverySource>,
}

/// Atomic projection bound to one exact Critical pressure proof and catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmergencyRecoveryOrdering {
    pub policy_revision: u32,
    pub pressure: TargetedProjectScanPressureContext,
    pub root_catalog: TargetedReclaimRootCatalogStamp,
    pub observed_root_count: u16,
    pub candidate_evaluated_root_count: u16,
    pub unavailable_root_count: u16,
    pub groups: Vec<EmergencyRecoveryGroup>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum EmergencyRecoveryError {
    #[error("engine session is closed")]
    Closed,
    #[error("the durable engine store is read-only")]
    ReadOnlyStore,
    #[error("emergency recovery ordering requires Critical pressure")]
    NotCritical,
    #[error("configured project roots changed")]
    RegistryChanged,
    #[error("the targeted-reclaim root catalog is invalid")]
    InvalidCatalog,
    #[error("the targeted-reclaim root catalog changed")]
    CatalogChanged,
    #[error("the exact pressure proof changed")]
    PressureChanged,
    #[error("durable schema is incompatible")]
    IncompatibleSchema,
    #[error("operation is temporarily busy")]
    Busy,
    #[error("storage failed its safety checks")]
    UnsafeStorage,
    #[error("bounded operation exceeded its resource budget")]
    BudgetExceeded,
    #[error("durable emergency-recovery state is corrupt")]
    CorruptData,
    #[error("durable emergency-recovery state is unavailable")]
    Unavailable,
    #[error("operation outcome is unknown")]
    OutcomeUnknown,
    #[error("internal engine state is unavailable")]
    InternalState,
}

#[derive(Clone, Debug)]
pub(super) struct EmergencyRecoveryScanObservation {
    pub root_ordinal: u16,
    pub scan_id: ScanId,
    pub observed_at: SystemTime,
    pub permission_issue_count: u64,
    pub candidates: Vec<DurableCandidateSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RuleGroupKey {
    rule: RuleRef,
    category_rank: u8,
}

struct RuleGroupValue {
    category: CandidateCategory,
    sources: BTreeMap<(u16, String), (SystemTime, u32, u32)>,
}

pub(super) fn build_emergency_recovery_groups(
    observations: Vec<EmergencyRecoveryScanObservation>,
    unavailable_root_count: u16,
) -> Result<Vec<EmergencyRecoveryGroup>, EmergencyRecoveryError> {
    let mut observations = observations;
    observations.sort_by(|left, right| {
        left.root_ordinal
            .cmp(&right.root_ordinal)
            .then_with(|| left.scan_id.as_str().cmp(right.scan_id.as_str()))
    });
    if observations
        .windows(2)
        .any(|pair| pair[0].root_ordinal == pair[1].root_ordinal)
    {
        return Err(EmergencyRecoveryError::CorruptData);
    }

    let mut stale_groups = BTreeMap::<RuleGroupKey, RuleGroupValue>::new();
    let mut rule_categories = BTreeMap::<RuleRef, CandidateCategory>::new();
    for observation in &observations {
        for candidate in &observation.candidates {
            if !is_stale_safe_regenerable(candidate) {
                continue;
            }
            if let Some(category) =
                rule_categories.insert(candidate.rule().clone(), candidate.category())
                && category != candidate.category()
            {
                return Err(EmergencyRecoveryError::CorruptData);
            }
            let key = RuleGroupKey {
                rule: candidate.rule().clone(),
                category_rank: category_priority(candidate.category()),
            };
            let value = stale_groups.entry(key).or_insert_with(|| RuleGroupValue {
                category: candidate.category(),
                sources: BTreeMap::new(),
            });
            if value.category != candidate.category() {
                return Err(EmergencyRecoveryError::CorruptData);
            }
            let source_key = (
                observation.root_ordinal,
                observation.scan_id.as_str().to_owned(),
            );
            let source = value
                .sources
                .entry(source_key)
                .or_insert((observation.observed_at, 0, 0));
            if source.0 != observation.observed_at {
                return Err(EmergencyRecoveryError::CorruptData);
            }
            source.1 = source
                .1
                .checked_add(1)
                .ok_or(EmergencyRecoveryError::BudgetExceeded)?;
            if !candidate.blockers().is_empty() {
                source.2 = source
                    .2
                    .checked_add(1)
                    .ok_or(EmergencyRecoveryError::BudgetExceeded)?;
            }
        }
    }

    let mut groups = Vec::new();
    for (key, value) in stale_groups {
        let sources = value
            .sources
            .into_iter()
            .map(
                |((root_ordinal, scan_id), (observed_at, candidate_count, blocked_count))| {
                    Ok(EmergencyRecoverySource {
                        root_ordinal,
                        scan_id: ScanId::new(scan_id)
                            .map_err(|_| EmergencyRecoveryError::CorruptData)?,
                        observed_at,
                        candidate_count: Some(candidate_count),
                        blocked_candidate_count: Some(blocked_count),
                        permission_issue_count: None,
                    })
                },
            )
            .collect::<Result<Vec<_>, EmergencyRecoveryError>>()?;
        groups.push(EmergencyRecoveryGroup {
            rank: 0,
            lane: EmergencyRecoveryLane::StaleSafeRegenerable,
            rule: Some(key.rule),
            category: Some(value.category),
            unavailable_root_count: 0,
            sources,
        });
    }

    if !observations.is_empty() {
        groups.push(EmergencyRecoveryGroup {
            rank: 0,
            lane: EmergencyRecoveryLane::GuidedExploration,
            rule: None,
            category: None,
            unavailable_root_count: 0,
            sources: observations
                .iter()
                .map(|observation| EmergencyRecoverySource {
                    root_ordinal: observation.root_ordinal,
                    scan_id: observation.scan_id.clone(),
                    observed_at: observation.observed_at,
                    candidate_count: None,
                    blocked_candidate_count: None,
                    permission_issue_count: None,
                })
                .collect(),
        });
    }

    let permission_sources = observations
        .iter()
        .filter(|observation| observation.permission_issue_count > 0)
        .map(|observation| EmergencyRecoverySource {
            root_ordinal: observation.root_ordinal,
            scan_id: observation.scan_id.clone(),
            observed_at: observation.observed_at,
            candidate_count: None,
            blocked_candidate_count: None,
            permission_issue_count: Some(observation.permission_issue_count),
        })
        .collect::<Vec<_>>();
    if !permission_sources.is_empty() || unavailable_root_count > 0 {
        groups.push(EmergencyRecoveryGroup {
            rank: 0,
            lane: EmergencyRecoveryLane::PermissionGap,
            rule: None,
            category: None,
            unavailable_root_count,
            sources: permission_sources,
        });
    }

    groups.sort_by(|left, right| {
        left.lane
            .priority()
            .cmp(&right.lane.priority())
            .then_with(|| left.rule.cmp(&right.rule))
            .then_with(|| {
                left.category
                    .map(category_priority)
                    .cmp(&right.category.map(category_priority))
            })
    });
    if groups.len() > MAX_EMERGENCY_RECOVERY_GROUPS {
        return Err(EmergencyRecoveryError::BudgetExceeded);
    }
    for (index, group) in groups.iter_mut().enumerate() {
        group.rank = u16::try_from(index).map_err(|_| EmergencyRecoveryError::BudgetExceeded)?;
    }
    Ok(groups)
}

pub(super) fn emergency_recovery_evidence_is_fresh(
    observed_at: SystemTime,
    capacity_anchor: SystemTime,
) -> bool {
    if observed_at >= capacity_anchor {
        return true;
    }
    capacity_anchor
        .duration_since(observed_at)
        .is_ok_and(|age| age <= EMERGENCY_RECOVERY_MAX_EVIDENCE_AGE)
}

fn is_stale_safe_regenerable(candidate: &DurableCandidateSummary) -> bool {
    matches!(
        candidate.status(),
        DurableCandidateStatus::Discovered | DurableCandidateStatus::Selected
    ) && candidate.safety() == SafetyTier::SafeRegenerable
        && candidate.action() == CandidateAction::RemoveKnownRegenerableContents
        && candidate
            .evidence_kinds()
            .contains(&EvidenceKind::MinimumAge)
        && !candidate.blockers().contains(&BlockReason::RecentActivity)
        && !candidate
            .blockers()
            .contains(&BlockReason::MissingModificationTime)
}

const fn category_priority(category: CandidateCategory) -> u8 {
    match category {
        CandidateCategory::DeveloperArtifact => 0,
        CandidateCategory::ApplicationCache => 1,
        CandidateCategory::BrowserCache => 2,
        CandidateCategory::LogAndDiagnostic => 3,
        CandidateCategory::InstallerAndDownload => 4,
        CandidateCategory::DeviceAndSimulatorData => 5,
        CandidateCategory::CloudFile => 6,
        CandidateCategory::LargeReviewItem => 7,
        CandidateCategory::ProtectedSystemData => 8,
        CandidateCategory::UnknownStorage => 9,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;
    use crate::domain::{BlockReason, CandidateId, RuleId, RuleRevision, VolumeId};
    use crate::engine::{TargetedProjectScanPressure, TargetedReclaimRootCatalogStamp};

    fn summary(
        id: &str,
        rule: &str,
        status: DurableCandidateStatus,
        safety: SafetyTier,
        action: CandidateAction,
        evidence: Vec<EvidenceKind>,
        blockers: Vec<BlockReason>,
    ) -> DurableCandidateSummary {
        DurableCandidateSummary::new(
            CandidateId::new(id).unwrap(),
            RuleRef::new(RuleId::new(rule).unwrap(), RuleRevision::new(1).unwrap()),
            CandidateCategory::DeveloperArtifact,
            1,
            Some(UNIX_EPOCH),
            safety,
            action,
            false,
            1,
            evidence,
            blockers,
            UNIX_EPOCH,
            status,
        )
    }

    fn observation(
        ordinal: u16,
        candidates: Vec<DurableCandidateSummary>,
        permission_issue_count: u64,
    ) -> EmergencyRecoveryScanObservation {
        EmergencyRecoveryScanObservation {
            root_ordinal: ordinal,
            scan_id: ScanId::new(format!("scan:{ordinal}")).unwrap(),
            observed_at: UNIX_EPOCH + Duration::from_secs(u64::from(ordinal) + 1),
            permission_issue_count,
            candidates,
        }
    }

    #[test]
    fn lane_priority_is_explicit_and_matches_roadmap() {
        assert_eq!(
            [
                EmergencyRecoveryLane::EvictableCloud,
                EmergencyRecoveryLane::StaleSafeRegenerable,
                EmergencyRecoveryLane::TrashInformation,
                EmergencyRecoveryLane::ReviewableInstallerArchive,
                EmergencyRecoveryLane::LargeFile,
                EmergencyRecoveryLane::GuidedExploration,
                EmergencyRecoveryLane::PermissionGap,
            ]
            .map(EmergencyRecoveryLane::priority),
            [0, 1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn emits_only_evidence_backed_lanes_in_deterministic_order() {
        let eligible = summary(
            "candidate:eligible",
            "developer.rust.target",
            DurableCandidateStatus::Discovered,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            vec![EvidenceKind::MatchedPath, EvidenceKind::MinimumAge],
            vec![
                BlockReason::MissingOrIncompleteEvidence,
                BlockReason::ProtectedPath,
            ],
        );
        let mut input = vec![observation(1, vec![], 3), observation(0, vec![eligible], 0)];
        let expected = build_emergency_recovery_groups(input.clone(), 0).unwrap();
        input.reverse();
        assert_eq!(build_emergency_recovery_groups(input, 0).unwrap(), expected);
        assert_eq!(
            expected.iter().map(|group| group.lane).collect::<Vec<_>>(),
            vec![
                EmergencyRecoveryLane::StaleSafeRegenerable,
                EmergencyRecoveryLane::GuidedExploration,
                EmergencyRecoveryLane::PermissionGap,
            ]
        );
        assert_eq!(
            expected.iter().map(|group| group.rank).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(expected[0].sources[0].candidate_count, Some(1));
        assert_eq!(expected[0].sources[0].blocked_candidate_count, Some(1));
        assert_eq!(expected[2].sources[0].permission_issue_count, Some(3));
    }

    #[test]
    fn status_age_and_safety_eligibility_fail_closed() {
        let mut candidates = Vec::new();
        for (index, status) in [
            DurableCandidateStatus::Dismissed,
            DurableCandidateStatus::Stale,
            DurableCandidateStatus::Planned,
            DurableCandidateStatus::Completed,
            DurableCandidateStatus::Failed,
            DurableCandidateStatus::Unavailable,
        ]
        .into_iter()
        .enumerate()
        {
            candidates.push(summary(
                &format!("candidate:status:{index}"),
                "developer.rust.target",
                status,
                SafetyTier::SafeRegenerable,
                CandidateAction::RemoveKnownRegenerableContents,
                vec![EvidenceKind::MinimumAge],
                vec![],
            ));
        }
        candidates.push(summary(
            "candidate:no-age",
            "developer.rust.target",
            DurableCandidateStatus::Discovered,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            vec![EvidenceKind::MatchedPath],
            vec![],
        ));
        candidates.push(summary(
            "candidate:review",
            "developer.rust.target",
            DurableCandidateStatus::Discovered,
            SafetyTier::ReviewRequired,
            CandidateAction::MoveToTrash,
            vec![EvidenceKind::MinimumAge],
            vec![],
        ));
        candidates.push(summary(
            "candidate:contradictory-age",
            "developer.rust.target",
            DurableCandidateStatus::Discovered,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            vec![EvidenceKind::MinimumAge],
            vec![BlockReason::RecentActivity],
        ));
        candidates.push(summary(
            "candidate:contradictory-mtime",
            "developer.rust.target",
            DurableCandidateStatus::Discovered,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
            vec![EvidenceKind::MinimumAge],
            vec![BlockReason::MissingModificationTime],
        ));
        let groups =
            build_emergency_recovery_groups(vec![observation(0, candidates, 0)], 0).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lane, EmergencyRecoveryLane::GuidedExploration);
    }

    #[test]
    fn unavailable_roots_are_explicit_without_fake_sources() {
        let groups = build_emergency_recovery_groups(Vec::new(), 2).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lane, EmergencyRecoveryLane::PermissionGap);
        assert_eq!(groups[0].unavailable_root_count, 2);
        assert!(groups[0].sources.is_empty());
    }

    #[test]
    fn evidence_freshness_is_inclusive_and_accepts_post_anchor_completion() {
        let anchor = UNIX_EPOCH + Duration::from_secs(10_000);
        assert!(emergency_recovery_evidence_is_fresh(
            anchor - EMERGENCY_RECOVERY_MAX_EVIDENCE_AGE,
            anchor
        ));
        assert!(!emergency_recovery_evidence_is_fresh(
            anchor - EMERGENCY_RECOVERY_MAX_EVIDENCE_AGE - Duration::from_nanos(1),
            anchor
        ));
        assert!(emergency_recovery_evidence_is_fresh(anchor, anchor));
        assert!(emergency_recovery_evidence_is_fresh(
            anchor + Duration::from_secs(1),
            anchor
        ));
    }

    #[test]
    fn ordering_shape_retains_exact_proofs_without_authority() {
        let pressure = TargetedProjectScanPressureContext {
            volume_id: VolumeId::new("volume:test").unwrap(),
            capacity_anchor: UNIX_EPOCH,
            pressure: TargetedProjectScanPressure::Critical,
            current_episode_started_at: UNIX_EPOCH,
            pressure_started_at: UNIX_EPOCH,
            policy_revision: 1,
        };
        let root_catalog = TargetedReclaimRootCatalogStamp {
            known_roots_policy_revision: 1,
            configured_roots_revision: 0,
            known_user_library_caches_included: false,
            root_count: 0,
            digest_sha256: [0; 32],
        };
        let ordering = EmergencyRecoveryOrdering {
            policy_revision: EMERGENCY_RECOVERY_POLICY_REVISION,
            pressure: pressure.clone(),
            root_catalog: root_catalog.clone(),
            observed_root_count: 0,
            candidate_evaluated_root_count: 0,
            unavailable_root_count: 0,
            groups: vec![],
        };
        assert_eq!(ordering.pressure, pressure);
        assert_eq!(ordering.root_catalog, root_catalog);
    }
}
