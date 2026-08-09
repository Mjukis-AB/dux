//! Pure, deterministic conversion of marker-verified scan artifacts into findings.
//!
//! This module has no filesystem, persistence, planning, or execution access.
//! Bundled rules may describe a proposed cleanup action, but every candidate
//! remains blocked scan evidence until independent live authority exists.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::candidate::CandidateInput;
use super::rule_document::{RuleRegistry, load_rule_registry_json};
use super::{
    AutomationDraftPolicyPreflight, AutomationScheduleDraftConfig, BlockReason, Candidate,
    CandidateAction, CandidateCategory, CandidateId, CandidateValidationError, Evidence, Rule,
    RuleId, RuleScope, SafetyTier, ScanCoverage, ScanCoverageStatus, ScanId,
    assess_automation_draft_policy,
};
use crate::persistence::CompleteCandidateRecord;
use crate::projection::{
    ArtifactKind, BuildArtifactEntry, StaleThreshold, project_build_artifacts_bounded_at,
};
use crate::scanner::{CompletedScanArtifact, FreshScanFacts};
use crate::tree::{DiskTree, NodeId, NodeKind};

#[path = "candidate_evaluator_snapshot_replay.rs"]
mod candidate_evaluator_snapshot_replay;
pub(crate) use candidate_evaluator_snapshot_replay::{
    CandidateSnapshotReplayError, replay_snapshot_candidate_evaluation,
    verify_snapshot_candidate_evaluation,
};

const BUNDLED_CATALOG: &[u8] = include_bytes!("../../catalogs/candidate-rules-v1.json");
static VALIDATED_BUNDLED_CATALOG: OnceLock<Result<RuleRegistry, CandidateEvaluationError>> =
    OnceLock::new();
const CANDIDATE_ID_DOMAIN: &[u8] = b"dux-candidate-id-v1\0";
const EVALUATION_CONTEXT_DOMAIN: &[u8] = b"dux-candidate-evaluation-context-v1\0";
const SELECTED_SCAN_ROOT_SCOPE: &[u8] = b"scope:selected_scan_root";
const USER_CACHE_DIRECTORY_SCOPE: &[u8] = b"scope:user_cache_directory";
const UNRESOLVED_PROTECTION: &[u8] = b"protected_path_authority:unresolved";

/// Engine-minted scan IDs carrying this prefix are automatic observations of
/// the current account's code-owned user-cache root. They must never borrow
/// the bundled selected-root rules merely because the same directory names
/// happen to appear below that root.
pub(crate) const KNOWN_USER_CACHE_SCAN_ID_PREFIX: &str = "scan:targeted:known-user-cache:";

pub(crate) const CANDIDATE_EVALUATOR_REVISION: u32 = 5;
pub(crate) const CANDIDATE_CATALOG_SCHEMA_VERSION: u32 = 1;
pub(crate) const CANDIDATE_CONTEXT_FORMAT_VERSION: u32 = 2;
pub(crate) const CANDIDATE_CATALOG_SHA256: [u8; 32] = [
    0x8e, 0xbb, 0x1d, 0x34, 0xc9, 0x36, 0x2e, 0x13, 0x41, 0x1b, 0x29, 0xbb, 0xa2, 0xbc, 0xfa, 0xe6,
    0xf1, 0x22, 0x5d, 0x1e, 0x2a, 0xfe, 0xc2, 0xe2, 0x22, 0x34, 0x91, 0xc9, 0xcb, 0x28, 0xa7, 0xef,
];
const SAFE_RUST_RULE_ID: &str = "developer.rust.target";
pub(crate) const SAFE_RUST_RULE_REVISION: u32 = 3;
pub(crate) const SAFE_RUST_RULE_MINIMUM_AGE: Duration = Duration::from_secs(7 * 86_400);
const SAFE_PYTHON_PYCACHE_RULE_ID: &str = "developer.python.pycache";
pub(crate) const SAFE_USER_CACHE_MINIMUM_AGE: Duration = Duration::from_secs(7 * 86_400);

/// Maximum findings returned by one evaluator invocation.
pub(crate) const MAX_EVALUATED_CANDIDATES: usize = 4_096;

#[derive(Clone, Copy)]
struct CatalogBinding {
    kind: ArtifactKind,
    component: &'static str,
    rule_id: &'static str,
}

pub(super) struct ObservedArtifact {
    path: PathBuf,
    rule_id: &'static str,
    size: u64,
    newest_mtime: Option<SystemTime>,
    mtime_coverage_complete: bool,
    evidence_paths: Vec<PathBuf>,
    additional_blockers: Vec<BlockReason>,
}

#[derive(Clone, Copy)]
pub(super) struct UserCacheBinding {
    pub(super) component: &'static str,
    pub(super) rule_id: &'static str,
}

const CATALOG_BINDINGS: &[CatalogBinding] = &[
    CatalogBinding {
        kind: ArtifactKind::CocoaPods,
        component: "Pods",
        rule_id: "developer.cocoapods.pods",
    },
    CatalogBinding {
        kind: ArtifactKind::Gradle,
        component: ".gradle",
        rule_id: "developer.gradle.dot_gradle",
    },
    CatalogBinding {
        kind: ArtifactKind::Gradle,
        component: "build",
        rule_id: "developer.gradle.build",
    },
    CatalogBinding {
        kind: ArtifactKind::NextNuxt,
        component: ".next",
        rule_id: "developer.next.output",
    },
    CatalogBinding {
        kind: ArtifactKind::Node,
        component: "node_modules",
        rule_id: "developer.node.modules",
    },
    CatalogBinding {
        kind: ArtifactKind::NextNuxt,
        component: ".nuxt",
        rule_id: "developer.nuxt.output",
    },
    CatalogBinding {
        kind: ArtifactKind::Python,
        component: "__pycache__",
        rule_id: "developer.python.pycache",
    },
    CatalogBinding {
        kind: ArtifactKind::Python,
        component: ".tox",
        rule_id: "developer.python.tox",
    },
    CatalogBinding {
        kind: ArtifactKind::Python,
        component: "venv",
        rule_id: "developer.python.venv",
    },
    CatalogBinding {
        kind: ArtifactKind::Python,
        component: ".venv",
        rule_id: "developer.python.venv_hidden",
    },
    CatalogBinding {
        kind: ArtifactKind::Rust,
        component: "target",
        rule_id: "developer.rust.target",
    },
];

pub(super) const USER_CACHE_BINDINGS: &[UserCacheBinding] = &[
    UserCacheBinding {
        component: "Homebrew",
        rule_id: "developer.homebrew.cache",
    },
    UserCacheBinding {
        component: "pip",
        rule_id: "developer.python.pip_cache",
    },
];

/// A bounded set of deterministic, non-authoritative findings for one scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateBatch {
    evaluator_revision: u32,
    catalog_schema_version: u32,
    catalog_digest_sha256: [u8; 32],
    context_format_version: u32,
    context_digest_sha256: [u8; 32],
    candidates: Vec<Candidate>,
}

impl CandidateBatch {
    pub(crate) fn evaluator_revision(&self) -> u32 {
        self.evaluator_revision
    }

    pub(crate) fn catalog_schema_version(&self) -> u32 {
        self.catalog_schema_version
    }

    pub(crate) fn catalog_digest_sha256(&self) -> [u8; 32] {
        self.catalog_digest_sha256
    }

    pub(crate) fn context_format_version(&self) -> u32 {
        self.context_format_version
    }

    pub(crate) fn context_digest_sha256(&self) -> [u8; 32] {
        self.context_digest_sha256
    }

    #[cfg(test)]
    pub(crate) fn observed_match_count(&self) -> usize {
        self.candidates.len()
    }

    #[cfg(test)]
    pub(crate) fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    pub(crate) fn into_candidates(self) -> Vec<Candidate> {
        self.candidates
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum CandidateEvaluationError {
    #[error("the embedded candidate catalog violates its v1 contract")]
    InvalidBundledCatalog,
    #[error("the artifact projection does not map to the embedded catalog")]
    InvalidArtifactProjection,
    #[error("the scan produced at least {observed_at_least} candidates; maximum is {maximum}")]
    CandidateLimitExceeded {
        observed_at_least: usize,
        maximum: usize,
    },
    #[error("the evaluator produced an invalid discovery candidate: {0}")]
    InvalidCandidate(#[from] CandidateValidationError),
}

/// SHA-256 of the exact JSON bytes compiled into this build.
pub(crate) fn bundled_candidate_catalog_digest_sha256() -> [u8; 32] {
    CANDIDATE_CATALOG_SHA256
}

/// Evaluate only a genuinely completed fresh traversal. Cached or caller-built
/// trees cannot enter the durable production candidate path.
pub(crate) fn evaluate_completed_scan_candidates(
    source_scan_id: &ScanId,
    artifact: &CompletedScanArtifact,
    evaluated_at: SystemTime,
    scope: CandidateEvaluationScope,
) -> Result<CandidateBatch, CandidateEvaluationError> {
    if candidate_evaluation_scope(source_scan_id) != scope {
        return Err(CandidateEvaluationError::InvalidArtifactProjection);
    }
    let (tree, facts, coverage) = artifact.parts();
    if scope == CandidateEvaluationScope::UserCacheDirectory {
        return evaluate_user_cache_candidates(source_scan_id, tree, coverage, evaluated_at);
    }
    evaluate_artifact_candidates_with_facts(
        source_scan_id,
        tree,
        Some(facts),
        coverage,
        evaluated_at,
    )
}

/// Convert existing marker-verified artifact projections into discovery-only candidates.
#[cfg(test)]
fn evaluate_artifact_candidates(
    source_scan_id: &ScanId,
    tree: &DiskTree,
    coverage: &ScanCoverage,
    evaluated_at: SystemTime,
) -> Result<CandidateBatch, CandidateEvaluationError> {
    evaluate_artifact_candidates_with_facts(source_scan_id, tree, None, coverage, evaluated_at)
}

fn evaluate_artifact_candidates_with_facts(
    source_scan_id: &ScanId,
    tree: &DiskTree,
    fresh_facts: Option<&FreshScanFacts>,
    coverage: &ScanCoverage,
    evaluated_at: SystemTime,
) -> Result<CandidateBatch, CandidateEvaluationError> {
    if candidate_evaluation_scope(source_scan_id) == CandidateEvaluationScope::UserCacheDirectory {
        return evaluate_user_cache_candidates(source_scan_id, tree, coverage, evaluated_at);
    }
    if fresh_facts.is_some_and(|facts| facts.len() != tree.len()) {
        return Err(CandidateEvaluationError::InvalidArtifactProjection);
    }
    let entries = project_build_artifacts_bounded_at(
        tree,
        StaleThreshold::All,
        SystemTime::UNIX_EPOCH,
        MAX_EVALUATED_CANDIDATES,
    )
    .map_err(
        |observed_at_least| CandidateEvaluationError::CandidateLimitExceeded {
            observed_at_least,
            maximum: MAX_EVALUATED_CANDIDATES,
        },
    )?
    .into_iter()
    .map(|entry| {
        let node = tree
            .get(entry.node_id)
            .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?;
        let binding = binding_for(tree, &entry)?;
        let size = estimated_reclaimable_bytes(tree, &entry, binding, fresh_facts)?;
        Ok(ObservedArtifact {
            path: node.path.clone(),
            rule_id: binding.rule_id,
            size,
            newest_mtime: entry.newest_mtime,
            mtime_coverage_complete: entry.mtime_coverage_complete,
            evidence_paths: entry.evidence_paths,
            additional_blockers: Vec::new(),
        })
    })
    .collect::<Result<Vec<_>, CandidateEvaluationError>>()?;
    evaluate_observed_artifacts(
        source_scan_id,
        tree.root_path(),
        coverage,
        entries,
        evaluated_at,
    )
}

/// Match only reviewed, exact direct children of the code-owned current-user
/// cache root. A same-named nested directory or selected-root scan cannot enter
/// this scope.
fn evaluate_user_cache_candidates(
    source_scan_id: &ScanId,
    tree: &DiskTree,
    coverage: &ScanCoverage,
    evaluated_at: SystemTime,
) -> Result<CandidateBatch, CandidateEvaluationError> {
    if candidate_evaluation_scope(source_scan_id) != CandidateEvaluationScope::UserCacheDirectory {
        return Err(CandidateEvaluationError::InvalidArtifactProjection);
    }
    let root = tree.root();
    let mut entries = Vec::new();
    for child_id in &root.children {
        let child = tree
            .get(*child_id)
            .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?;
        let Some(binding) = USER_CACHE_BINDINGS
            .iter()
            .find(|binding| binding.component == child.name)
        else {
            continue;
        };
        if child.parent != Some(NodeId::ROOT)
            || child.depth != 1
            || child.kind != NodeKind::Directory
            || child.path_is_symlink
            || child.path != tree.root_path().join(binding.component)
        {
            continue;
        }
        if entries.len() == MAX_EVALUATED_CANDIDATES {
            return Err(CandidateEvaluationError::CandidateLimitExceeded {
                observed_at_least: MAX_EVALUATED_CANDIDATES + 1,
                maximum: MAX_EVALUATED_CANDIDATES,
            });
        }
        entries.push(user_cache_observation(tree, *child_id, binding)?);
    }
    evaluate_observed_artifacts(
        source_scan_id,
        tree.root_path(),
        coverage,
        entries,
        evaluated_at,
    )
}

fn user_cache_observation(
    tree: &DiskTree,
    root_id: NodeId,
    binding: &UserCacheBinding,
) -> Result<ObservedArtifact, CandidateEvaluationError> {
    let root = tree
        .get(root_id)
        .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?;
    let mut newest_mtime = None;
    let mut mtime_coverage_complete = true;
    let mut saw_symlink_boundary = false;
    let mut stack = vec![root_id];
    let mut visited = 0_usize;

    while let Some(node_id) = stack.pop() {
        visited = visited.saturating_add(1);
        if visited > tree.len() {
            return Err(CandidateEvaluationError::InvalidArtifactProjection);
        }
        let node = tree
            .get(node_id)
            .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?;
        match node.kind {
            NodeKind::Directory | NodeKind::File => {
                if node.path_is_symlink {
                    saw_symlink_boundary = true;
                }
                match node.mtime {
                    Some(mtime) => {
                        if newest_mtime.is_none_or(|current| mtime > current) {
                            newest_mtime = Some(mtime);
                        }
                    }
                    None => mtime_coverage_complete = false,
                }
            }
            NodeKind::Symlink => saw_symlink_boundary = true,
            NodeKind::Other | NodeKind::Error => {}
        }
        stack.extend(node.children.iter().copied());
    }

    // Recency is useful prioritization evidence, not a proof that the owning
    // tool is idle. No provider-specific process/activity witness exists yet.
    let mut additional_blockers = vec![BlockReason::MissingOrIncompleteEvidence];
    if saw_symlink_boundary {
        additional_blockers.push(BlockReason::SymlinkBoundary);
    }
    Ok(ObservedArtifact {
        path: root.path.clone(),
        rule_id: binding.rule_id,
        size: root.size,
        newest_mtime,
        mtime_coverage_complete,
        evidence_paths: Vec::new(),
        additional_blockers,
    })
}

pub(super) fn evaluate_observed_artifacts(
    source_scan_id: &ScanId,
    root: &Path,
    coverage: &ScanCoverage,
    entries: Vec<ObservedArtifact>,
    evaluated_at: SystemTime,
) -> Result<CandidateBatch, CandidateEvaluationError> {
    let catalog = load_and_validate_catalog()?;
    let catalog_digest_sha256 = bundled_candidate_catalog_digest_sha256();
    let context_digest_sha256 = candidate_evaluation_context_digest_for_observation(
        source_scan_id,
        root,
        coverage,
        evaluated_at,
    );
    let mut entries = entries
        .into_iter()
        .map(|entry| {
            let rule = rule_for_id(catalog, entry.rule_id)?;
            Ok((native_path_bytes(&entry.path), rule, entry))
        })
        .collect::<Result<Vec<_>, CandidateEvaluationError>>()?;
    entries.sort_by(|left, right| {
        left.0.cmp(&right.0).then_with(|| {
            left.1
                .reference()
                .id()
                .as_str()
                .cmp(right.1.reference().id().as_str())
        })
    });
    if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(CandidateEvaluationError::InvalidArtifactProjection);
    }

    let mut candidates = Vec::with_capacity(entries.len());
    for (path_bytes, rule, mut entry) in entries {
        entry
            .evidence_paths
            .sort_by_key(|evidence_path| native_path_bytes(evidence_path));
        let mut evidence = Vec::with_capacity(entry.evidence_paths.len() + 1);
        evidence.push(Evidence::MatchedPath {
            path: entry.path.clone(),
        });
        evidence.extend(
            entry
                .evidence_paths
                .into_iter()
                .map(|path| Evidence::RequiredMarker { path }),
        );

        let partial_coverage = candidate_has_partial_coverage(
            candidate_evaluation_scope(source_scan_id),
            coverage,
            root,
            &entry.path,
        );
        let mut blockers = Vec::with_capacity(5);
        if partial_coverage {
            blockers.push(BlockReason::PartialScanCoverage);
        }
        if let Some(minimum_age) = rule.guards().minimum_age() {
            match (
                !partial_coverage && entry.mtime_coverage_complete,
                entry.newest_mtime,
            ) {
                (true, Some(newest_mtime))
                    if evaluated_at
                        .duration_since(newest_mtime)
                        .is_ok_and(|age| age >= minimum_age) =>
                {
                    evidence.push(Evidence::MinimumAge {
                        newest_mtime,
                        minimum_age,
                    });
                }
                (true, Some(_)) => blockers.push(BlockReason::RecentActivity),
                (true, None) | (false, _) if !partial_coverage => {
                    blockers.push(BlockReason::MissingModificationTime);
                }
                _ => {}
            }
        }
        for blocker in entry.additional_blockers {
            if !blockers.contains(&blocker) {
                blockers.push(blocker);
            }
        }
        // The evaluator has no trusted home, volume, canonical ancestry, or
        // protected-root grant witness. Keep that unresolved authority explicit.
        blockers.push(BlockReason::ProtectedPath);

        let id = candidate_id(source_scan_id, rule, &path_bytes);
        candidates.push(Candidate::try_from_rule(
            rule,
            CandidateInput::new(
                id,
                vec![entry.path],
                entry.size,
                entry.newest_mtime,
                evidence,
                blockers,
                source_scan_id.clone(),
            ),
        )?);
    }

    Ok(CandidateBatch {
        evaluator_revision: CANDIDATE_EVALUATOR_REVISION,
        catalog_schema_version: CANDIDATE_CATALOG_SCHEMA_VERSION,
        catalog_digest_sha256,
        context_format_version: CANDIDATE_CONTEXT_FORMAT_VERSION,
        context_digest_sha256,
        candidates,
    })
}

pub(super) fn candidate_has_partial_coverage(
    scope: CandidateEvaluationScope,
    coverage: &ScanCoverage,
    root: &Path,
    candidate: &Path,
) -> bool {
    if scope == CandidateEvaluationScope::SelectedScanRoot {
        return coverage.status() != ScanCoverageStatus::Complete;
    }
    if coverage.status() == ScanCoverageStatus::Unknown {
        return true;
    }
    if coverage.status() == ScanCoverageStatus::Complete {
        return false;
    }
    coverage.issues().iter().any(|issue| {
        issue.path().is_none_or(|path| {
            path == root || path.starts_with(candidate) || candidate.starts_with(path)
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CandidateEvaluationScope {
    SelectedScanRoot,
    UserCacheDirectory,
}

pub(super) fn candidate_evaluation_scope(source_scan_id: &ScanId) -> CandidateEvaluationScope {
    if source_scan_id
        .as_str()
        .starts_with(KNOWN_USER_CACHE_SCAN_ID_PREFIX)
    {
        CandidateEvaluationScope::UserCacheDirectory
    } else {
        CandidateEvaluationScope::SelectedScanRoot
    }
}

/// Validate the exact catalog compiled into this process before publishing an
/// engine that could create durable evaluation records.
pub(crate) fn validate_bundled_candidate_catalog() -> Result<(), CandidateEvaluationError> {
    load_and_validate_catalog().map(|_| ())
}

/// Count only rules whose shipped policy is structurally eligible for future
/// automation. Runtime history and fresh-candidate gates are intentionally not
/// evaluated here, so this count never grants execution authority.
pub(crate) fn bundled_automation_eligible_rule_count() -> Result<u16, CandidateEvaluationError> {
    let count = load_and_validate_catalog()?
        .iter()
        .filter(|rule| {
            rule.schedule_eligible() && rule.safety().is_schedule_policy_pair(rule.action())
        })
        .count();
    u16::try_from(count).map_err(|_| CandidateEvaluationError::InvalidBundledCatalog)
}

/// Return the exact shipped rules that may be considered by the read-only
/// automation-history suggestion query. Historical evidence cannot widen
/// this set or relax any current matcher policy.
pub(crate) fn bundled_automation_history_suggestion_rules()
-> Result<Vec<Rule>, CandidateEvaluationError> {
    Ok(load_and_validate_catalog()?
        .iter()
        .filter(|rule| {
            rule.schedule_eligible()
                && rule.scope() == RuleScope::UserCacheDirectory
                && rule.safety() == SafetyTier::SafeRegenerable
                && rule.action() == CandidateAction::RemoveKnownRegenerableContents
                && rule.matcher().protected_descendants().is_empty()
        })
        .cloned()
        .collect())
}

pub(crate) fn bundled_automation_draft_policy_preflight(
    config: &AutomationScheduleDraftConfig,
) -> Result<AutomationDraftPolicyPreflight, CandidateEvaluationError> {
    Ok(assess_automation_draft_policy(
        config,
        load_and_validate_catalog()?.iter(),
    ))
}

fn load_and_validate_catalog() -> Result<&'static RuleRegistry, CandidateEvaluationError> {
    VALIDATED_BUNDLED_CATALOG
        .get_or_init(validate_catalog_bytes)
        .as_ref()
        .map_err(|error| *error)
}

fn validate_catalog_bytes() -> Result<RuleRegistry, CandidateEvaluationError> {
    if <[u8; 32]>::from(Sha256::digest(BUNDLED_CATALOG)) != CANDIDATE_CATALOG_SHA256 {
        return Err(CandidateEvaluationError::InvalidBundledCatalog);
    }
    let catalog = load_rule_registry_json(BUNDLED_CATALOG)
        .map_err(|_| CandidateEvaluationError::InvalidBundledCatalog)?;
    if catalog.len() != CATALOG_BINDINGS.len() + USER_CACHE_BINDINGS.len() || catalog.is_empty() {
        return Err(CandidateEvaluationError::InvalidBundledCatalog);
    }
    for binding in CATALOG_BINDINGS {
        validate_catalog_rule(
            &catalog,
            binding.rule_id,
            binding.component,
            RuleScope::SelectedScanRoot,
        )?;
    }
    for binding in USER_CACHE_BINDINGS {
        validate_catalog_rule(
            &catalog,
            binding.rule_id,
            binding.component,
            RuleScope::UserCacheDirectory,
        )?;
    }
    Ok(catalog)
}

fn validate_catalog_rule(
    catalog: &RuleRegistry,
    rule_id: &str,
    component: &str,
    expected_scope: RuleScope,
) -> Result<(), CandidateEvaluationError> {
    let id = RuleId::new(rule_id).map_err(|_| CandidateEvaluationError::InvalidBundledCatalog)?;
    let Some(rule) = catalog.get(&id) else {
        return Err(CandidateEvaluationError::InvalidBundledCatalog);
    };
    let (required_ancestor_markers_any, required_markers_all) = expected_catalog_markers(rule_id)?;
    let (expected_revision, expected_safety, expected_action) = expected_catalog_policy(rule_id)?;
    if rule.category() != CandidateCategory::DeveloperArtifact
        || rule.reference().revision().get() != expected_revision
        || rule.scope() != expected_scope
        || rule.matcher().path_component() != Some(component)
        || !matches_exact_strings(
            rule.matcher().required_ancestor_markers_any(),
            required_ancestor_markers_any,
        )
        || !matches_exact_strings(rule.matcher().required_markers_all(), required_markers_all)
        || !rule.matcher().exact_bundle_identifiers().is_empty()
        || !rule.matcher().forbidden_markers_any().is_empty()
        || !rule.matcher().excluded_descendants().is_empty()
        || !rule.matcher().protected_descendants().is_empty()
        || rule.guards().minimum_age() != expected_minimum_age(rule_id)
        || rule.guards().minimum_bytes() != 0
        || !rule.guards().inactive_processes().is_empty()
        || rule.guards().requires_cloud_upload_complete()
        || rule.safety() != expected_safety
        || rule.action() != expected_action
        || rule.schedule_eligible()
        || !has_expected_provenance(rule, rule_id)
    {
        return Err(CandidateEvaluationError::InvalidBundledCatalog);
    }
    Ok(())
}

fn has_expected_provenance(rule: &Rule, rule_id: &str) -> bool {
    let expected: &[&str] = match rule_id {
        SAFE_RUST_RULE_ID => &[
            "https://doc.rust-lang.org/cargo/reference/build-cache.html",
            "https://doc.rust-lang.org/cargo/commands/cargo-clean.html",
        ],
        SAFE_PYTHON_PYCACHE_RULE_ID => &[
            "https://docs.python.org/3/reference/import.html#cached-bytecode-invalidation",
            "https://docs.python.org/3/faq/programming.html#how-do-i-create-a-pyc-file",
            "https://peps.python.org/pep-3147/",
        ],
        "developer.homebrew.cache" => &["https://docs.brew.sh/Manpage#cache-options"],
        "developer.python.pip_cache" => &["https://pip.pypa.io/en/stable/topics/caching/"],
        _ => return true,
    };
    rule.provenance().len() == expected.len()
        && rule
            .provenance()
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.as_str() == *expected)
}

fn expected_catalog_policy(
    rule_id: &str,
) -> Result<(u32, SafetyTier, CandidateAction), CandidateEvaluationError> {
    match rule_id {
        SAFE_RUST_RULE_ID => Ok((
            SAFE_RUST_RULE_REVISION,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        )),
        SAFE_PYTHON_PYCACHE_RULE_ID => Ok((
            2,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        )),
        "developer.homebrew.cache" | "developer.python.pip_cache" => Ok((
            1,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        )),
        _ => Ok((1, SafetyTier::Informational, CandidateAction::RevealOnly)),
    }
}

fn expected_minimum_age(rule_id: &str) -> Option<Duration> {
    if rule_id == SAFE_RUST_RULE_ID {
        Some(SAFE_RUST_RULE_MINIMUM_AGE)
    } else if USER_CACHE_BINDINGS
        .iter()
        .any(|binding| binding.rule_id == rule_id)
    {
        Some(SAFE_USER_CACHE_MINIMUM_AGE)
    } else {
        None
    }
}

fn expected_catalog_markers(
    rule_id: &str,
) -> Result<(&'static [&'static str], &'static [&'static str]), CandidateEvaluationError> {
    const GRADLE: &[&str] = &[
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
    ];
    match rule_id {
        "developer.cocoapods.pods" => Ok((&["Podfile"], &["Manifest.lock"])),
        "developer.gradle.dot_gradle" | "developer.gradle.build" => Ok((GRADLE, &[])),
        "developer.next.output" | "developer.node.modules" | "developer.nuxt.output" => {
            Ok((&["package.json"], &[]))
        }
        "developer.python.pycache" => Ok((&[], &[])),
        "developer.python.tox" => Ok((&["tox.ini"], &[])),
        "developer.python.venv" | "developer.python.venv_hidden" => Ok((&[], &["pyvenv.cfg"])),
        "developer.rust.target" => Ok((&["Cargo.toml"], &["CACHEDIR.TAG"])),
        "developer.homebrew.cache" | "developer.python.pip_cache" => Ok((&[], &[])),
        _ => Err(CandidateEvaluationError::InvalidBundledCatalog),
    }
}

fn matches_exact_strings(actual: &[String], expected: &[&str]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual == expected)
}

/// Canonical evaluator context available before evaluation starts, so durable
/// pending and failed attempts can bind the same identity as successful ones.
pub(crate) fn candidate_evaluation_context_digest_sha256(
    source_scan_id: &ScanId,
    artifact: &CompletedScanArtifact,
    evaluated_at: SystemTime,
) -> [u8; 32] {
    let (tree, _, coverage) = artifact.parts();
    candidate_evaluation_context_digest_for_observation(
        source_scan_id,
        tree.root_path(),
        coverage,
        evaluated_at,
    )
}

/// Recompute the current evaluator context from one durable scan observation.
///
/// The immutable snapshot digest is bound separately by persistence. This
/// digest deliberately covers only the evaluator policy, scan identity/root,
/// and exact coverage facts available before candidate evaluation starts.
pub(crate) fn candidate_evaluation_context_digest_for_observation(
    source_scan_id: &ScanId,
    root: &Path,
    coverage: &ScanCoverage,
    evaluated_at: SystemTime,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(EVALUATION_CONTEXT_DOMAIN);
    hasher.update(CANDIDATE_CONTEXT_FORMAT_VERSION.to_le_bytes());
    hasher.update(CANDIDATE_EVALUATOR_REVISION.to_le_bytes());
    hasher.update(CANDIDATE_CATALOG_SCHEMA_VERSION.to_le_bytes());
    hasher.update(CANDIDATE_CATALOG_SHA256);
    let scope = match candidate_evaluation_scope(source_scan_id) {
        CandidateEvaluationScope::SelectedScanRoot => SELECTED_SCAN_ROOT_SCOPE,
        CandidateEvaluationScope::UserCacheDirectory => USER_CACHE_DIRECTORY_SCOPE,
    };
    update_length_prefixed(&mut hasher, scope);
    update_length_prefixed(&mut hasher, UNRESOLVED_PROTECTION);
    update_length_prefixed(&mut hasher, source_scan_id.as_str().as_bytes());
    update_length_prefixed(&mut hasher, &native_path_bytes(root));
    update_system_time(&mut hasher, evaluated_at);
    hasher.update([coverage_status_rank(coverage.status())]);
    match coverage.measured_permille() {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.get().to_le_bytes());
        }
        None => hasher.update([0]),
    }
    hasher.update((coverage.issues().len() as u64).to_le_bytes());
    for issue in coverage.issues() {
        hasher.update([issue.kind().canonical_rank()]);
        match issue.path() {
            Some(path) => {
                hasher.update([1]);
                update_length_prefixed(&mut hasher, &native_path_bytes(path));
            }
            None => hasher.update([0]),
        }
        hasher.update(issue.occurrence_count().to_le_bytes());
    }
    hasher.finalize().into()
}

fn update_system_time(hasher: &mut Sha256, value: SystemTime) {
    match value.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => {
            hasher.update([0]);
            hasher.update(duration.as_secs().to_le_bytes());
            hasher.update(duration.subsec_nanos().to_le_bytes());
        }
        Err(error) => {
            let duration = error.duration();
            hasher.update([1]);
            hasher.update(duration.as_secs().to_le_bytes());
            hasher.update(duration.subsec_nanos().to_le_bytes());
        }
    }
}

/// Deterministic ID expected for the current bundled Rust-target rule.
pub(crate) fn current_rust_target_candidate_id(
    source_scan_id: &ScanId,
    path: &Path,
) -> Result<CandidateId, CandidateEvaluationError> {
    let catalog = load_and_validate_catalog()?;
    let binding = CATALOG_BINDINGS
        .iter()
        .find(|binding| binding.rule_id == SAFE_RUST_RULE_ID)
        .ok_or(CandidateEvaluationError::InvalidBundledCatalog)?;
    let rule = rule_for(catalog, binding)?;
    Ok(candidate_id(source_scan_id, rule, &native_path_bytes(path)))
}

/// Rehydrate one domain candidate from a complete durable record only after
/// the current bundled rule catalog has been loaded. Every immutable field is
/// compared back to the record so history cannot smuggle policy-shaped data
/// into the planner when the catalog or storage has drifted.
pub(crate) fn candidate_from_complete_record(
    record: &CompleteCandidateRecord,
) -> Result<Candidate, CandidateEvaluationError> {
    let catalog = load_and_validate_catalog()?;
    let rule = catalog
        .get(record.rule().id())
        .ok_or(CandidateEvaluationError::InvalidBundledCatalog)?;
    if rule.reference() != record.rule() {
        return Err(CandidateEvaluationError::InvalidArtifactProjection);
    }
    let candidate = Candidate::try_from_rule(
        rule,
        CandidateInput::new(
            record.id().clone(),
            record.paths().to_vec(),
            record.estimated_bytes(),
            record.newest_mtime(),
            record.evidence().to_vec(),
            record.blockers().to_vec(),
            record.source_scan_id().clone(),
        ),
    )?;
    let matches = candidate.id() == record.id()
        && candidate.source_scan_id() == record.source_scan_id()
        && candidate.rule() == record.rule()
        && candidate.category() == record.category()
        && candidate.paths() == record.paths()
        && candidate.estimated_bytes() == record.estimated_bytes()
        && candidate.newest_mtime() == record.newest_mtime()
        && candidate.evidence() == record.evidence()
        && candidate.safety() == record.safety()
        && candidate.action() == record.action()
        && candidate.rule_marks_schedule_eligible() == record.rule_schedule_eligible()
        && candidate.blockers() == record.blockers();
    matches
        .then_some(candidate)
        .ok_or(CandidateEvaluationError::InvalidArtifactProjection)
}

const fn coverage_status_rank(status: ScanCoverageStatus) -> u8 {
    match status {
        ScanCoverageStatus::Unknown => 0,
        ScanCoverageStatus::Complete => 1,
        ScanCoverageStatus::LimitedAccess => 2,
        ScanCoverageStatus::Partial => 3,
    }
}

fn binding_for(
    tree: &DiskTree,
    entry: &BuildArtifactEntry,
) -> Result<&'static CatalogBinding, CandidateEvaluationError> {
    let node = tree
        .get(entry.node_id)
        .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?;
    CATALOG_BINDINGS
        .iter()
        .find(|binding| binding.kind == entry.kind && binding.component == node.name)
        .ok_or(CandidateEvaluationError::InvalidArtifactProjection)
}

fn estimated_reclaimable_bytes(
    tree: &DiskTree,
    entry: &BuildArtifactEntry,
    binding: &CatalogBinding,
    fresh_facts: Option<&FreshScanFacts>,
) -> Result<u64, CandidateEvaluationError> {
    if binding.rule_id != SAFE_RUST_RULE_ID {
        return Ok(entry.size);
    }
    let target = tree
        .get(entry.node_id)
        .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?;
    let mut named_tags = target.children.iter().filter_map(|child_id| {
        tree.get(*child_id)
            .filter(|child| child.name == "CACHEDIR.TAG")
            .map(|child| (*child_id, child))
    });
    let (tag_id, tag) = named_tags
        .next()
        .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?;
    if named_tags.next().is_some()
        || tag.parent != Some(entry.node_id)
        || tag.depth
            != target
                .depth
                .checked_add(1)
                .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?
        || tag.kind != NodeKind::File
        || tag.path_is_symlink
        || tag.path != target.path.join("CACHEDIR.TAG")
        || target
            .children
            .iter()
            .filter(|child_id| **child_id == tag_id)
            .count()
            != 1
        || entry
            .evidence_paths
            .iter()
            .filter(|path| **path == tag.path)
            .count()
            != 1
    {
        return Err(CandidateEvaluationError::InvalidArtifactProjection);
    }
    if let Some(facts) = fresh_facts {
        let tag_allocated = facts
            .node(tag_id)
            .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?
            .allocated_bytes;
        if tag_allocated.unwrap_or(0) != tag.size {
            return Err(CandidateEvaluationError::InvalidArtifactProjection);
        }
        let mut pending = Vec::new();
        pending
            .try_reserve(target.children.len())
            .map_err(|_| CandidateEvaluationError::InvalidArtifactProjection)?;
        pending.extend(
            target
                .children
                .iter()
                .copied()
                .filter(|child| *child != tag_id),
        );
        while let Some(node_id) = pending.pop() {
            let node = tree
                .get(node_id)
                .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?;
            let allocated = facts
                .node(node_id)
                .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?
                .allocated_bytes
                .ok_or(CandidateEvaluationError::InvalidArtifactProjection)?;
            if allocated != node.size {
                return Err(CandidateEvaluationError::InvalidArtifactProjection);
            }
            pending
                .try_reserve(node.children.len())
                .map_err(|_| CandidateEvaluationError::InvalidArtifactProjection)?;
            pending.extend(node.children.iter().copied());
        }
    }
    entry
        .size
        .checked_sub(tag.size)
        .ok_or(CandidateEvaluationError::InvalidArtifactProjection)
}

pub(super) fn project_rule_id(
    kind: ArtifactKind,
    component: &str,
) -> Result<&'static str, CandidateEvaluationError> {
    CATALOG_BINDINGS
        .iter()
        .find(|binding| binding.kind == kind && binding.component == component)
        .map(|binding| binding.rule_id)
        .ok_or(CandidateEvaluationError::InvalidArtifactProjection)
}

fn rule_for<'a>(
    catalog: &'a RuleRegistry,
    binding: &CatalogBinding,
) -> Result<&'a Rule, CandidateEvaluationError> {
    rule_for_id(catalog, binding.rule_id)
}

fn rule_for_id<'a>(
    catalog: &'a RuleRegistry,
    rule_id: &str,
) -> Result<&'a Rule, CandidateEvaluationError> {
    let id = RuleId::new(rule_id).map_err(|_| CandidateEvaluationError::InvalidBundledCatalog)?;
    catalog
        .get(&id)
        .ok_or(CandidateEvaluationError::InvalidBundledCatalog)
}

fn candidate_id(source_scan_id: &ScanId, rule: &Rule, path_bytes: &[u8]) -> CandidateId {
    let mut hasher = Sha256::new();
    hasher.update(CANDIDATE_ID_DOMAIN);
    update_length_prefixed(&mut hasher, source_scan_id.as_str().as_bytes());
    update_length_prefixed(&mut hasher, rule.reference().id().as_str().as_bytes());
    hasher.update(rule.reference().revision().get().to_le_bytes());
    update_length_prefixed(&mut hasher, path_bytes);
    let digest: [u8; 32] = hasher.finalize().into();
    let mut value = String::with_capacity("candidate:v1:".len() + 64);
    value.push_str("candidate:v1:");
    push_lower_hex(&mut value, &digest);
    CandidateId::new(value).expect("fixed candidate ID encoding must satisfy the stable grammar")
}

fn update_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn push_lower_hex(output: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

#[cfg(unix)]
fn native_path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn native_path_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(not(any(unix, windows)))]
fn native_path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().to_string_lossy().as_bytes().to_vec()
}

#[cfg(test)]
#[path = "candidate_evaluator_tests.rs"]
mod tests;
