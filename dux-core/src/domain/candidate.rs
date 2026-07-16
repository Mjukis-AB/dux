use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use thiserror::Error;

use super::{CandidateAction, CandidateCategory, CandidateId, Rule, RuleRef, SafetyTier, ScanId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EvidenceKind {
    MatchedPath,
    RequiredMarker,
    ForbiddenMarkerAbsent,
    BundleIdentifier,
    MinimumAge,
    MinimumSize,
    InactiveProcess,
    CloudUploadComplete,
}

/// A typed fact observed during discovery.
///
/// Evidence belongs to a scan snapshot. It is never sufficient authorization
/// for mutation and must be re-evaluated by a future planner/executor boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    MatchedPath {
        path: PathBuf,
    },
    RequiredMarker {
        path: PathBuf,
    },
    ForbiddenMarkerAbsent {
        path: PathBuf,
    },
    BundleIdentifier {
        path: PathBuf,
        identifier: String,
    },
    MinimumAge {
        newest_mtime: SystemTime,
        minimum_age: Duration,
    },
    MinimumSize {
        observed_bytes: u64,
        minimum_bytes: u64,
    },
    InactiveProcess {
        identifier: String,
    },
    CloudUploadComplete {
        path: PathBuf,
    },
}

impl Evidence {
    pub fn kind(&self) -> EvidenceKind {
        match self {
            Self::MatchedPath { .. } => EvidenceKind::MatchedPath,
            Self::RequiredMarker { .. } => EvidenceKind::RequiredMarker,
            Self::ForbiddenMarkerAbsent { .. } => EvidenceKind::ForbiddenMarkerAbsent,
            Self::BundleIdentifier { .. } => EvidenceKind::BundleIdentifier,
            Self::MinimumAge { .. } => EvidenceKind::MinimumAge,
            Self::MinimumSize { .. } => EvidenceKind::MinimumSize,
            Self::InactiveProcess { .. } => EvidenceKind::InactiveProcess,
            Self::CloudUploadComplete { .. } => EvidenceKind::CloudUploadComplete,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    MissingOrIncompleteEvidence,
    MissingModificationTime,
    PartialScanCoverage,
    RecentActivity,
    BelowMinimumBytes,
    ActiveUse,
    AccessDenied,
    ProtectedPath,
    ProtectedDescendant,
    SymlinkBoundary,
    VolumeBoundary,
    ChangedSinceScan,
    UnsupportedPlatform,
    CloudUploadUnconfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    id: CandidateId,
    rule: RuleRef,
    category: CandidateCategory,
    paths: Vec<PathBuf>,
    estimated_bytes: u64,
    newest_mtime: Option<SystemTime>,
    evidence: Vec<Evidence>,
    safety: SafetyTier,
    action: CandidateAction,
    rule_schedule_eligible: bool,
    blockers: Vec<BlockReason>,
    source_scan_id: ScanId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateInput {
    id: CandidateId,
    paths: Vec<PathBuf>,
    estimated_bytes: u64,
    newest_mtime: Option<SystemTime>,
    evidence: Vec<Evidence>,
    blockers: Vec<BlockReason>,
    source_scan_id: ScanId,
}

impl CandidateInput {
    pub(crate) fn new(
        id: CandidateId,
        paths: Vec<PathBuf>,
        estimated_bytes: u64,
        newest_mtime: Option<SystemTime>,
        evidence: Vec<Evidence>,
        blockers: Vec<BlockReason>,
        source_scan_id: ScanId,
    ) -> Self {
        Self {
            id,
            paths,
            estimated_bytes,
            newest_mtime,
            evidence,
            blockers,
            source_scan_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CandidateValidationError {
    #[error("candidate must contain at least one observed path")]
    MissingPaths,
    #[error("candidate paths must be unique")]
    DuplicatePaths,
    #[error("candidate must contain at least one typed evidence fact")]
    MissingEvidence,
    #[error("safe eviction candidates require one exact confirmed-upload fact per path")]
    InvalidCloudUploadEvidence,
}

impl Candidate {
    /// Construct a non-authoritative discovery candidate from validated rule policy.
    ///
    /// This copies every policy field from `rule`; callers cannot supply or
    /// override safety, action, scheduling, category, ID, or revision. It does
    /// not validate filesystem paths. A future validator/planner must do that
    /// before any cleanup plan can exist.
    pub(crate) fn try_from_rule(
        rule: &Rule,
        input: CandidateInput,
    ) -> Result<Self, CandidateValidationError> {
        if input.paths.is_empty() {
            return Err(CandidateValidationError::MissingPaths);
        }
        let candidate_paths = input.paths.iter().collect::<HashSet<_>>();
        if candidate_paths.len() != input.paths.len() {
            return Err(CandidateValidationError::DuplicatePaths);
        }
        if input.evidence.is_empty() {
            return Err(CandidateValidationError::MissingEvidence);
        }
        if rule.action() == CandidateAction::EvictLocalCopy {
            let confirmed_paths = input
                .evidence
                .iter()
                .filter_map(|evidence| match evidence {
                    Evidence::CloudUploadComplete { path } => Some(path),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let unique_confirmed_paths = confirmed_paths.iter().copied().collect::<HashSet<_>>();
            if confirmed_paths.len() != candidate_paths.len()
                || unique_confirmed_paths.len() != candidate_paths.len()
                || unique_confirmed_paths != candidate_paths
            {
                return Err(CandidateValidationError::InvalidCloudUploadEvidence);
            }
        }

        Ok(Self {
            id: input.id,
            rule: rule.reference().clone(),
            category: rule.category(),
            paths: input.paths,
            estimated_bytes: input.estimated_bytes,
            newest_mtime: input.newest_mtime,
            evidence: input.evidence,
            safety: rule.safety(),
            action: rule.action(),
            rule_schedule_eligible: rule.schedule_eligible(),
            blockers: input.blockers,
            source_scan_id: input.source_scan_id,
        })
    }

    pub fn id(&self) -> &CandidateId {
        &self.id
    }

    pub fn rule(&self) -> &RuleRef {
        &self.rule
    }

    pub fn category(&self) -> CandidateCategory {
        self.category
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }

    pub fn newest_mtime(&self) -> Option<SystemTime> {
        self.newest_mtime
    }

    pub fn evidence(&self) -> &[Evidence] {
        &self.evidence
    }

    pub fn safety(&self) -> SafetyTier {
        self.safety
    }

    pub fn action(&self) -> CandidateAction {
        self.action
    }

    pub fn rule_marks_schedule_eligible(&self) -> bool {
        self.rule_schedule_eligible
    }

    pub fn blockers(&self) -> &[BlockReason] {
        &self.blockers
    }

    pub fn source_scan_id(&self) -> &ScanId {
        &self.source_scan_id
    }

    pub fn has_cleanup_operation(&self) -> bool {
        self.action.is_cleanup_operation()
    }

    pub fn has_no_known_blockers(&self) -> bool {
        self.blockers.is_empty()
    }

    pub fn contains_path(&self, path: &Path) -> bool {
        self.paths.iter().any(|candidate| candidate == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        LocalizedTextKey, ProvenanceUrl, RuleDefinition, RuleGuards, RuleId, RuleMatcher,
        RuleMatcherDefinition, RuleRevision, RuleScope,
    };

    fn path_matcher(component: &str) -> RuleMatcher {
        RuleMatcher::try_new(RuleMatcherDefinition {
            path_component: Some(component.to_owned()),
            required_ancestor_markers_any: Vec::new(),
            required_markers_all: Vec::new(),
            forbidden_markers_any: Vec::new(),
            exact_bundle_identifiers: Vec::new(),
            excluded_descendants: Vec::new(),
            protected_descendants: Vec::new(),
        })
        .unwrap()
    }

    fn rule(schedule_eligible: bool) -> Rule {
        Rule::try_new(RuleDefinition {
            reference: RuleRef::new(
                RuleId::new("developer.rust.target").unwrap(),
                RuleRevision::new(7).unwrap(),
            ),
            title_key: LocalizedTextKey::new("rule.developer.rust.target.title").unwrap(),
            category: CandidateCategory::DeveloperArtifact,
            scope: RuleScope::ConfiguredProjectRoots,
            matcher: path_matcher("target"),
            guards: RuleGuards::try_new(None, 0, Vec::new(), false).unwrap(),
            safety: SafetyTier::SafeRegenerable,
            action: CandidateAction::RemoveKnownRegenerableContents,
            schedule_eligible,
            explanation_key: LocalizedTextKey::new("rule.developer.rust.target.explanation")
                .unwrap(),
            provenance: vec![ProvenanceUrl::new("https://example.com/rust-target").unwrap()],
        })
        .unwrap()
    }

    fn eviction_rule() -> Rule {
        Rule::try_new(RuleDefinition {
            reference: RuleRef::new(
                RuleId::new("cloud.icloud.evict").unwrap(),
                RuleRevision::new(1).unwrap(),
            ),
            title_key: LocalizedTextKey::new("rule.cloud.icloud.evict.title").unwrap(),
            category: CandidateCategory::CloudFile,
            scope: RuleScope::Home,
            matcher: path_matcher("Mobile Documents"),
            guards: RuleGuards::try_new(None, 0, Vec::new(), true).unwrap(),
            safety: SafetyTier::SafeEvictable,
            action: CandidateAction::EvictLocalCopy,
            schedule_eligible: false,
            explanation_key: LocalizedTextKey::new("rule.cloud.icloud.evict.explanation").unwrap(),
            provenance: vec![ProvenanceUrl::new("https://developer.apple.com/icloud").unwrap()],
        })
        .unwrap()
    }

    fn input(blockers: Vec<BlockReason>) -> CandidateInput {
        CandidateInput::new(
            CandidateId::new("candidate:01").unwrap(),
            vec![PathBuf::from("/projects/dux/target")],
            1024,
            None,
            vec![Evidence::RequiredMarker {
                path: PathBuf::from("/projects/dux/Cargo.toml"),
            }],
            blockers,
            ScanId::new("scan:01").unwrap(),
        )
    }

    #[test]
    fn candidate_copies_rule_owned_policy_and_revision() {
        let candidate = Candidate::try_from_rule(&rule(true), input(Vec::new())).unwrap();

        assert_eq!(candidate.rule().id().as_str(), "developer.rust.target");
        assert_eq!(candidate.rule().revision().get(), 7);
        assert_eq!(candidate.category(), CandidateCategory::DeveloperArtifact);
        assert_eq!(candidate.safety(), SafetyTier::SafeRegenerable);
        assert_eq!(
            candidate.action(),
            CandidateAction::RemoveKnownRegenerableContents
        );
        assert!(candidate.has_cleanup_operation());
        assert!(candidate.has_no_known_blockers());
        assert!(candidate.rule_marks_schedule_eligible());
    }

    #[test]
    fn multiple_blockers_are_preserved_as_separate_discovery_facts() {
        let blockers = vec![
            BlockReason::MissingModificationTime,
            BlockReason::PartialScanCoverage,
            BlockReason::ActiveUse,
        ];
        let candidate = Candidate::try_from_rule(&rule(true), input(blockers.clone())).unwrap();

        assert_eq!(candidate.blockers(), blockers);
        assert!(candidate.has_cleanup_operation());
        assert!(!candidate.has_no_known_blockers());
        assert!(candidate.rule_marks_schedule_eligible());
    }

    #[test]
    fn construction_requires_paths_and_typed_evidence() {
        let mut no_paths = input(Vec::new());
        no_paths.paths.clear();
        assert_eq!(
            Candidate::try_from_rule(&rule(false), no_paths).unwrap_err(),
            CandidateValidationError::MissingPaths
        );

        let mut no_evidence = input(Vec::new());
        no_evidence.evidence.clear();
        assert_eq!(
            Candidate::try_from_rule(&rule(false), no_evidence).unwrap_err(),
            CandidateValidationError::MissingEvidence
        );
    }

    #[test]
    fn construction_rejects_duplicate_cleanup_targets() {
        let path = PathBuf::from("/cloud/first");
        let mut duplicate = input(Vec::new());
        duplicate.paths = vec![path.clone(), path.clone()];
        duplicate.evidence = vec![
            Evidence::CloudUploadComplete { path },
            Evidence::CloudUploadComplete {
                path: PathBuf::from("/cloud/unrelated"),
            },
        ];

        assert_eq!(
            Candidate::try_from_rule(&eviction_rule(), duplicate).unwrap_err(),
            CandidateValidationError::DuplicatePaths
        );
    }

    #[test]
    fn evidence_kinds_are_stable_semantic_facts() {
        let candidate = Candidate::try_from_rule(&rule(false), input(Vec::new())).unwrap();
        assert_eq!(candidate.evidence()[0].kind(), EvidenceKind::RequiredMarker);
        assert!(candidate.contains_path(Path::new("/projects/dux/target")));
        assert_eq!(candidate.estimated_bytes(), 1024);
        assert_eq!(candidate.source_scan_id().as_str(), "scan:01");
    }

    #[test]
    fn eviction_candidates_require_confirmed_upload_evidence() {
        assert_eq!(
            Candidate::try_from_rule(&eviction_rule(), input(Vec::new())).unwrap_err(),
            CandidateValidationError::InvalidCloudUploadEvidence
        );

        let report = PathBuf::from("/Users/test/Library/Mobile Documents/report.pdf");
        let mut confirmed = input(Vec::new());
        confirmed.paths = vec![report.clone()];
        confirmed.evidence = vec![Evidence::CloudUploadComplete { path: report }];
        let candidate = Candidate::try_from_rule(&eviction_rule(), confirmed).unwrap();
        assert!(candidate.has_cleanup_operation());
        assert!(candidate.has_no_known_blockers());
        assert!(!candidate.rule_marks_schedule_eligible());
    }

    #[test]
    fn eviction_evidence_is_one_to_one_with_every_candidate_path() {
        let first = PathBuf::from("/cloud/first");
        let second = PathBuf::from("/cloud/second");
        let unrelated = PathBuf::from("/cloud/unrelated");

        let mut mismatched = input(Vec::new());
        mismatched.paths = vec![first.clone(), second.clone()];
        mismatched.evidence = vec![
            Evidence::CloudUploadComplete {
                path: first.clone(),
            },
            Evidence::CloudUploadComplete { path: unrelated },
        ];
        assert_eq!(
            Candidate::try_from_rule(&eviction_rule(), mismatched).unwrap_err(),
            CandidateValidationError::InvalidCloudUploadEvidence
        );

        let mut duplicate = input(Vec::new());
        duplicate.paths = vec![first.clone(), second.clone()];
        duplicate.evidence = vec![
            Evidence::CloudUploadComplete {
                path: first.clone(),
            },
            Evidence::CloudUploadComplete { path: first },
        ];
        assert_eq!(
            Candidate::try_from_rule(&eviction_rule(), duplicate).unwrap_err(),
            CandidateValidationError::InvalidCloudUploadEvidence
        );

        let mut complete = input(Vec::new());
        complete.paths = vec![PathBuf::from("/cloud/first"), second.clone()];
        complete.evidence = vec![
            Evidence::CloudUploadComplete {
                path: PathBuf::from("/cloud/first"),
            },
            Evidence::CloudUploadComplete { path: second },
        ];
        assert!(Candidate::try_from_rule(&eviction_rule(), complete).is_ok());
    }
}
