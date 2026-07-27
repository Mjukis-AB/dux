//! Pure, deterministic conversion of marker-verified scan artifacts into findings.
//!
//! This module has no filesystem, persistence, planning, or execution access.
//! Bundled rules may describe a proposed cleanup action, but every candidate
//! remains blocked scan evidence until independent live authority exists.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::candidate::CandidateInput;
use super::rule_document::{RuleRegistry, load_rule_registry_json};
use super::{
    BlockReason, Candidate, CandidateAction, CandidateCategory, CandidateId,
    CandidateValidationError, Evidence, Rule, RuleId, RuleScope, SafetyTier, ScanCoverage,
    ScanCoverageStatus, ScanId,
};
use crate::persistence::CompleteCandidateRecord;
use crate::projection::{
    ArtifactKind, BuildArtifactEntry, StaleThreshold, project_build_artifacts_bounded_at,
};
use crate::scanner::CompletedScanArtifact;
use crate::tree::DiskTree;

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
const UNRESOLVED_PROTECTION: &[u8] = b"protected_path_authority:unresolved";

pub(crate) const CANDIDATE_EVALUATOR_REVISION: u32 = 2;
pub(crate) const CANDIDATE_CATALOG_SCHEMA_VERSION: u32 = 1;
pub(crate) const CANDIDATE_CONTEXT_FORMAT_VERSION: u32 = 1;
pub(crate) const CANDIDATE_CATALOG_SHA256: [u8; 32] = [
    0xdd, 0x91, 0x55, 0xd3, 0x99, 0x98, 0x59, 0x22, 0x44, 0xc9, 0x4c, 0x55, 0xfb, 0xc8, 0x17, 0xb7,
    0x16, 0xd0, 0xeb, 0xfd, 0x40, 0xc3, 0x82, 0x13, 0xe1, 0x44, 0x66, 0x24, 0x4a, 0x0a, 0x0c, 0x91,
];
const SAFE_RUST_RULE_ID: &str = "developer.rust.target";
const SAFE_PYTHON_PYCACHE_RULE_ID: &str = "developer.python.pycache";

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
    component: &'static str,
    kind: ArtifactKind,
    size: u64,
    newest_mtime: Option<SystemTime>,
    evidence_paths: Vec<PathBuf>,
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
) -> Result<CandidateBatch, CandidateEvaluationError> {
    let (tree, _, coverage) = artifact.parts();
    evaluate_artifact_candidates(source_scan_id, tree, coverage)
}

/// Convert existing marker-verified artifact projections into discovery-only candidates.
fn evaluate_artifact_candidates(
    source_scan_id: &ScanId,
    tree: &DiskTree,
    coverage: &ScanCoverage,
) -> Result<CandidateBatch, CandidateEvaluationError> {
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
        Ok(ObservedArtifact {
            path: node.path.clone(),
            component: binding.component,
            kind: entry.kind,
            size: entry.size,
            newest_mtime: entry.newest_mtime,
            evidence_paths: entry.evidence_paths,
        })
    })
    .collect::<Result<Vec<_>, CandidateEvaluationError>>()?;
    evaluate_observed_artifacts(source_scan_id, tree.root_path(), coverage, entries)
}

pub(super) fn evaluate_observed_artifacts(
    source_scan_id: &ScanId,
    root: &Path,
    coverage: &ScanCoverage,
    entries: Vec<ObservedArtifact>,
) -> Result<CandidateBatch, CandidateEvaluationError> {
    let catalog = load_and_validate_catalog()?;
    let catalog_digest_sha256 = bundled_candidate_catalog_digest_sha256();
    let context_digest_sha256 =
        candidate_evaluation_context_digest_for_observation(source_scan_id, root, coverage);
    let mut entries = entries
        .into_iter()
        .map(|entry| {
            let binding = binding_for_observation(entry.kind, entry.component)?;
            Ok((native_path_bytes(&entry.path), binding, entry))
        })
        .collect::<Result<Vec<_>, CandidateEvaluationError>>()?;
    entries.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.rule_id.cmp(right.1.rule_id))
    });
    if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(CandidateEvaluationError::InvalidArtifactProjection);
    }

    let mut candidates = Vec::with_capacity(entries.len());
    for (path_bytes, binding, mut entry) in entries {
        let rule = rule_for(catalog, binding)?;
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

        let mut blockers = Vec::with_capacity(2);
        if coverage.status() != ScanCoverageStatus::Complete {
            blockers.push(BlockReason::PartialScanCoverage);
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

/// Validate the exact catalog compiled into this process before publishing an
/// engine that could create durable evaluation records.
pub(crate) fn validate_bundled_candidate_catalog() -> Result<(), CandidateEvaluationError> {
    load_and_validate_catalog().map(|_| ())
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
    if catalog.len() != CATALOG_BINDINGS.len() || catalog.is_empty() {
        return Err(CandidateEvaluationError::InvalidBundledCatalog);
    }
    for binding in CATALOG_BINDINGS {
        let id = RuleId::new(binding.rule_id)
            .map_err(|_| CandidateEvaluationError::InvalidBundledCatalog)?;
        let Some(rule) = catalog.get(&id) else {
            return Err(CandidateEvaluationError::InvalidBundledCatalog);
        };
        let (required_ancestor_markers_any, required_markers_all) =
            expected_catalog_markers(binding.rule_id)?;
        let (expected_revision, expected_safety, expected_action) =
            expected_catalog_policy(binding.rule_id)?;
        if rule.category() != CandidateCategory::DeveloperArtifact
            || rule.reference().revision().get() != expected_revision
            || rule.scope() != RuleScope::SelectedScanRoot
            || rule.matcher().path_component() != Some(binding.component)
            || !matches_exact_strings(
                rule.matcher().required_ancestor_markers_any(),
                required_ancestor_markers_any,
            )
            || !matches_exact_strings(rule.matcher().required_markers_all(), required_markers_all)
            || !rule.matcher().exact_bundle_identifiers().is_empty()
            || !rule.matcher().forbidden_markers_any().is_empty()
            || !rule.matcher().excluded_descendants().is_empty()
            || !rule.matcher().protected_descendants().is_empty()
            || rule.guards().minimum_age().is_some()
            || rule.guards().minimum_bytes() != 0
            || !rule.guards().inactive_processes().is_empty()
            || rule.guards().requires_cloud_upload_complete()
            || rule.safety() != expected_safety
            || rule.action() != expected_action
            || rule.schedule_eligible()
            || !has_expected_provenance(rule, binding.rule_id)
        {
            return Err(CandidateEvaluationError::InvalidBundledCatalog);
        }
    }
    Ok(catalog)
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
        SAFE_RUST_RULE_ID | SAFE_PYTHON_PYCACHE_RULE_ID => Ok((
            2,
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        )),
        _ => Ok((1, SafetyTier::Informational, CandidateAction::RevealOnly)),
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
) -> [u8; 32] {
    let (tree, _, coverage) = artifact.parts();
    candidate_evaluation_context_digest_for_observation(source_scan_id, tree.root_path(), coverage)
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
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(EVALUATION_CONTEXT_DOMAIN);
    hasher.update(CANDIDATE_CONTEXT_FORMAT_VERSION.to_le_bytes());
    hasher.update(CANDIDATE_EVALUATOR_REVISION.to_le_bytes());
    hasher.update(CANDIDATE_CATALOG_SCHEMA_VERSION.to_le_bytes());
    hasher.update(CANDIDATE_CATALOG_SHA256);
    update_length_prefixed(&mut hasher, SELECTED_SCAN_ROOT_SCOPE);
    update_length_prefixed(&mut hasher, UNRESOLVED_PROTECTION);
    update_length_prefixed(&mut hasher, source_scan_id.as_str().as_bytes());
    update_length_prefixed(&mut hasher, &native_path_bytes(root));
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

fn binding_for_observation(
    kind: ArtifactKind,
    component: &str,
) -> Result<&'static CatalogBinding, CandidateEvaluationError> {
    CATALOG_BINDINGS
        .iter()
        .find(|binding| binding.kind == kind && binding.component == component)
        .ok_or(CandidateEvaluationError::InvalidArtifactProjection)
}

fn rule_for<'a>(
    catalog: &'a RuleRegistry,
    binding: &CatalogBinding,
) -> Result<&'a Rule, CandidateEvaluationError> {
    let id = RuleId::new(binding.rule_id)
        .map_err(|_| CandidateEvaluationError::InvalidBundledCatalog)?;
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
