#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateCategory {
    DeveloperArtifact,
    ApplicationCache,
    BrowserCache,
    LogAndDiagnostic,
    InstallerAndDownload,
    DeviceAndSimulatorData,
    CloudFile,
    LargeReviewItem,
    ProtectedSystemData,
    UnknownStorage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SafetyTier {
    SafeRegenerable,
    SafeEvictable,
    ReviewRequired,
    Informational,
    Protected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateAction {
    RemoveKnownRegenerableContents,
    EvictLocalCopy,
    MoveToTrash,
    RevealOnly,
    NoAction,
}

impl CandidateAction {
    pub fn is_cleanup_operation(self) -> bool {
        matches!(
            self,
            Self::RemoveKnownRegenerableContents | Self::EvictLocalCopy | Self::MoveToTrash
        )
    }

    pub fn is_permanent_removal(self) -> bool {
        self == Self::RemoveKnownRegenerableContents
    }

    pub fn is_compatible_with(self, safety: SafetyTier) -> bool {
        match safety {
            SafetyTier::SafeRegenerable => self == Self::RemoveKnownRegenerableContents,
            SafetyTier::SafeEvictable => self == Self::EvictLocalCopy,
            SafetyTier::ReviewRequired => self == Self::MoveToTrash,
            SafetyTier::Informational | SafetyTier::Protected => {
                matches!(self, Self::RevealOnly | Self::NoAction)
            }
        }
    }
}

impl SafetyTier {
    pub fn is_schedule_policy_pair(self, action: CandidateAction) -> bool {
        self == Self::SafeRegenerable && action.is_permanent_removal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAFETY_TIERS: [SafetyTier; 5] = [
        SafetyTier::SafeRegenerable,
        SafetyTier::SafeEvictable,
        SafetyTier::ReviewRequired,
        SafetyTier::Informational,
        SafetyTier::Protected,
    ];
    const ACTIONS: [CandidateAction; 5] = [
        CandidateAction::RemoveKnownRegenerableContents,
        CandidateAction::EvictLocalCopy,
        CandidateAction::MoveToTrash,
        CandidateAction::RevealOnly,
        CandidateAction::NoAction,
    ];

    #[test]
    fn cleanup_actions_are_explicit() {
        assert!(CandidateAction::RemoveKnownRegenerableContents.is_cleanup_operation());
        assert!(CandidateAction::EvictLocalCopy.is_cleanup_operation());
        assert!(CandidateAction::MoveToTrash.is_cleanup_operation());
        assert!(!CandidateAction::RevealOnly.is_cleanup_operation());
        assert!(!CandidateAction::NoAction.is_cleanup_operation());
    }

    #[test]
    fn action_compatibility_fails_closed_for_every_policy_pair() {
        for safety in SAFETY_TIERS {
            for action in ACTIONS {
                let expected = match action {
                    CandidateAction::RemoveKnownRegenerableContents => {
                        safety == SafetyTier::SafeRegenerable
                    }
                    CandidateAction::EvictLocalCopy => safety == SafetyTier::SafeEvictable,
                    CandidateAction::MoveToTrash => safety == SafetyTier::ReviewRequired,
                    CandidateAction::RevealOnly | CandidateAction::NoAction => {
                        matches!(safety, SafetyTier::Informational | SafetyTier::Protected)
                    }
                };
                assert_eq!(action.is_compatible_with(safety), expected);
            }
        }
    }

    #[test]
    fn only_permanent_safe_regenerable_rules_support_scheduling() {
        for safety in SAFETY_TIERS {
            for action in ACTIONS {
                assert_eq!(
                    safety.is_schedule_policy_pair(action),
                    safety == SafetyTier::SafeRegenerable
                        && action == CandidateAction::RemoveKnownRegenerableContents
                );
            }
        }
    }
}
