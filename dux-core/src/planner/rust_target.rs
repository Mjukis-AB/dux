use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::domain::{
    BlockReason, Candidate, CandidateAction, CandidateCategory, CandidateId, Evidence, SafetyTier,
    ScanId,
};
use crate::path_validation::{
    CanonicalFileDigestError, CanonicalFileDigestSnapshot, CanonicalFilePrefixError,
    CanonicalFilePrefixSnapshot, CanonicalPathError, CanonicalPathSnapshot, CanonicalScanRoot,
    FilesystemEntryKind, LexicalPathError, capture_path_snapshot, capture_regular_file_prefix,
    capture_regular_file_sha256, capture_scan_root, validate_cleanup_path, validate_scan_root,
};

const RUST_TARGET_RULE_ID: &str = "developer.rust.target";
const RUST_TARGET_RULE_REVISION: u32 = 2;
const RUST_TARGET_WITNESS_REVISION: u32 = 1;
const CARGO_CACHE_TAG_SIGNATURE: &[u8; 43] = b"Signature: 8a477f597d28d172789f06886806bc55";
const MAX_CARGO_MANIFEST_BYTES: usize = 4 * 1024 * 1024;

/// Historical source binding supplied by the future planner's durable loader.
///
/// The path remains a locator until `validate_live_rust_target` reconstructs
/// all live filesystem evidence. This type itself grants no authority.
pub(crate) struct RustTargetValidationSource<'a> {
    source_scan_id: &'a ScanId,
    scan_root: &'a Path,
}

impl<'a> RustTargetValidationSource<'a> {
    pub(crate) fn new(source_scan_id: &'a ScanId, scan_root: &'a Path) -> Self {
        Self {
            source_scan_id,
            scan_root,
        }
    }
}

/// Current default-layout Cargo evidence that cannot authorize cleanup.
///
/// This witness deliberately has no `Clone`, serialization, plan conversion,
/// blocker-removal, or execution method. It does not prove Cargo configuration,
/// process inactivity, protected-root authority, or descendant ownership.
#[must_use = "live evidence is ephemeral and does not authorize cleanup"]
pub(crate) struct RustTargetLiveWitness {
    witness_revision: u32,
    source_scan_id: ScanId,
    candidate_id: CandidateId,
    scan_root: CanonicalScanRoot,
    target: CanonicalPathSnapshot,
    manifest: CanonicalFileDigestSnapshot,
    cache_tag: CanonicalFilePrefixSnapshot,
    protected_path_still_unresolved: ProtectedPathStillUnresolved,
}

struct ProtectedPathStillUnresolved;

#[derive(Debug, Error)]
pub(crate) enum RustTargetLiveValidationError {
    #[error("candidate does not have the exact staged Rust target policy")]
    CandidatePolicyMismatch,
    #[error("candidate is not bound to the supplied source scan")]
    SourceScanMismatch,
    #[error("candidate does not retain exactly the unresolved protected-path blocker")]
    ProtectedPathNotUnresolved,
    #[error("candidate does not contain the exact staged Rust target evidence")]
    CandidateEvidenceMismatch,
    #[error("candidate paths do not form the exact default Cargo target layout")]
    LayoutMismatch,
    #[error("live Rust target validation is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("a required marker has more than one hard link")]
    MultiplyLinkedMarker,
    #[error("the cache tag does not begin with the standard signature")]
    InvalidCacheTagSignature,
    #[error("live Rust target evidence changed during validation")]
    ChangedDuringValidation,
    #[error(transparent)]
    Lexical(#[from] LexicalPathError),
    #[error(transparent)]
    Filesystem(#[from] CanonicalPathError),
    #[error(transparent)]
    FilePrefix(#[from] CanonicalFilePrefixError),
    #[error(transparent)]
    FileDigest(#[from] CanonicalFileDigestError),
}

pub(crate) fn validate_live_rust_target(
    source: RustTargetValidationSource<'_>,
    candidate: &Candidate,
) -> Result<RustTargetLiveWitness, RustTargetLiveValidationError> {
    validate_candidate_policy(source.source_scan_id, candidate)?;
    let (target_path, manifest_path, cache_tag_path) = validate_candidate_layout(candidate)?;

    #[cfg(not(unix))]
    {
        let _ = (source.scan_root, target_path, manifest_path, cache_tag_path);
        return Err(RustTargetLiveValidationError::UnsupportedPlatform);
    }

    #[cfg(unix)]
    {
        let lexical_root = validate_scan_root(source.scan_root)?;
        let scan_root = capture_scan_root(lexical_root.clone())?;

        let lexical_target = validate_cleanup_path(&lexical_root, target_path)?;
        let lexical_manifest = validate_cleanup_path(&lexical_root, &manifest_path)?;
        let lexical_cache_tag = validate_cleanup_path(&lexical_root, &cache_tag_path)?;

        let target = capture_path_snapshot(&scan_root, lexical_target.clone())?;
        if target.target_kind() != FilesystemEntryKind::Directory {
            return Err(RustTargetLiveValidationError::LayoutMismatch);
        }

        let manifest = capture_regular_file_sha256(
            &scan_root,
            lexical_manifest.clone(),
            MAX_CARGO_MANIFEST_BYTES,
        )?;
        if manifest.path().hard_link_count() != 1 {
            return Err(RustTargetLiveValidationError::MultiplyLinkedMarker);
        }

        let cache_tag = capture_regular_file_prefix(
            &scan_root,
            lexical_cache_tag.clone(),
            CARGO_CACHE_TAG_SIGNATURE.len(),
        )?;
        if cache_tag.path().hard_link_count() != 1 {
            return Err(RustTargetLiveValidationError::MultiplyLinkedMarker);
        }
        if cache_tag.prefix() != CARGO_CACHE_TAG_SIGNATURE {
            return Err(RustTargetLiveValidationError::InvalidCacheTagSignature);
        }

        let target_parent = target
            .ancestors()
            .last()
            .ok_or(RustTargetLiveValidationError::LayoutMismatch)?
            .identity();
        let manifest_parent = manifest
            .path()
            .ancestors()
            .last()
            .ok_or(RustTargetLiveValidationError::LayoutMismatch)?
            .identity();
        let cache_tag_parent = cache_tag
            .path()
            .ancestors()
            .last()
            .ok_or(RustTargetLiveValidationError::LayoutMismatch)?
            .identity();
        if target_parent != manifest_parent || cache_tag_parent != target.target_identity() {
            return Err(RustTargetLiveValidationError::LayoutMismatch);
        }

        let final_root = capture_scan_root(lexical_root)?;
        let final_target = capture_path_snapshot(&final_root, lexical_target)?;
        let final_manifest =
            capture_regular_file_sha256(&final_root, lexical_manifest, MAX_CARGO_MANIFEST_BYTES)?;
        let final_cache_tag = capture_path_snapshot(&final_root, lexical_cache_tag)?;
        if final_root != scan_root
            || final_target != target
            || final_manifest != manifest
            || final_cache_tag != *cache_tag.path()
        {
            return Err(RustTargetLiveValidationError::ChangedDuringValidation);
        }

        Ok(RustTargetLiveWitness {
            witness_revision: RUST_TARGET_WITNESS_REVISION,
            source_scan_id: candidate.source_scan_id().clone(),
            candidate_id: candidate.id().clone(),
            scan_root,
            target,
            manifest,
            cache_tag,
            protected_path_still_unresolved: ProtectedPathStillUnresolved,
        })
    }
}

#[cfg(unix)]
impl RustTargetLiveWitness {
    pub(super) fn revalidate_current(&self) -> Result<(), RustTargetLiveValidationError> {
        let lexical_root = validate_scan_root(self.scan_root.canonical_path())?;
        let current_root = capture_scan_root(lexical_root.clone())?;
        if current_root != self.scan_root {
            return Err(RustTargetLiveValidationError::ChangedDuringValidation);
        }

        let lexical_target = validate_cleanup_path(&lexical_root, self.target.canonical_path())?;
        let lexical_manifest =
            validate_cleanup_path(&lexical_root, self.manifest.path().canonical_path())?;
        let lexical_cache_tag =
            validate_cleanup_path(&lexical_root, self.cache_tag.path().canonical_path())?;
        let current_target = capture_path_snapshot(&current_root, lexical_target)?;
        let current_manifest =
            capture_regular_file_sha256(&current_root, lexical_manifest, MAX_CARGO_MANIFEST_BYTES)?;
        let current_cache_tag = capture_regular_file_prefix(
            &current_root,
            lexical_cache_tag,
            CARGO_CACHE_TAG_SIGNATURE.len(),
        )?;
        if current_target != self.target
            || current_manifest != self.manifest
            || current_cache_tag != self.cache_tag
            || current_cache_tag.prefix() != CARGO_CACHE_TAG_SIGNATURE
        {
            return Err(RustTargetLiveValidationError::ChangedDuringValidation);
        }
        Ok(())
    }

    pub(super) fn project_root(&self) -> &Path {
        self.manifest
            .path()
            .canonical_path()
            .parent()
            .expect("validated Cargo.toml always has a parent")
    }

    pub(super) fn manifest_path(&self) -> &Path {
        self.manifest.path().canonical_path()
    }

    pub(super) fn target_path(&self) -> &Path {
        self.target.canonical_path()
    }
}

fn validate_candidate_policy(
    source_scan_id: &ScanId,
    candidate: &Candidate,
) -> Result<(), RustTargetLiveValidationError> {
    if candidate.source_scan_id() != source_scan_id {
        return Err(RustTargetLiveValidationError::SourceScanMismatch);
    }
    if candidate.rule().id().as_str() != RUST_TARGET_RULE_ID
        || candidate.rule().revision().get() != RUST_TARGET_RULE_REVISION
        || candidate.category() != CandidateCategory::DeveloperArtifact
        || candidate.safety() != SafetyTier::SafeRegenerable
        || candidate.action() != CandidateAction::RemoveKnownRegenerableContents
        || candidate.rule_marks_schedule_eligible()
    {
        return Err(RustTargetLiveValidationError::CandidatePolicyMismatch);
    }
    if candidate.blockers() != [BlockReason::ProtectedPath] {
        return Err(RustTargetLiveValidationError::ProtectedPathNotUnresolved);
    }
    Ok(())
}

fn validate_candidate_layout(
    candidate: &Candidate,
) -> Result<(&Path, PathBuf, PathBuf), RustTargetLiveValidationError> {
    let [target_path] = candidate.paths() else {
        return Err(RustTargetLiveValidationError::CandidateEvidenceMismatch);
    };
    if target_path.file_name() != Some(OsStr::new("target")) {
        return Err(RustTargetLiveValidationError::LayoutMismatch);
    }
    let project_root = target_path
        .parent()
        .ok_or(RustTargetLiveValidationError::LayoutMismatch)?;
    let manifest_path = project_root.join("Cargo.toml");
    let cache_tag_path = target_path.join("CACHEDIR.TAG");

    let mut matched_target = 0_u8;
    let mut matched_manifest = 0_u8;
    let mut matched_cache_tag = 0_u8;
    for evidence in candidate.evidence() {
        match evidence {
            Evidence::MatchedPath { path } if path == target_path => {
                matched_target = matched_target.saturating_add(1);
            }
            Evidence::RequiredMarker { path } if path == &manifest_path => {
                matched_manifest = matched_manifest.saturating_add(1);
            }
            Evidence::RequiredMarker { path } if path == &cache_tag_path => {
                matched_cache_tag = matched_cache_tag.saturating_add(1);
            }
            _ => return Err(RustTargetLiveValidationError::CandidateEvidenceMismatch),
        }
    }
    if candidate.evidence().len() != 3
        || matched_target != 1
        || matched_manifest != 1
        || matched_cache_tag != 1
    {
        return Err(RustTargetLiveValidationError::CandidateEvidenceMismatch);
    }

    Ok((target_path, manifest_path, cache_tag_path))
}

#[cfg(test)]
impl RustTargetLiveWitness {
    pub(super) fn witness_revision(&self) -> u32 {
        self.witness_revision
    }

    pub(super) fn source_scan_id(&self) -> &ScanId {
        &self.source_scan_id
    }

    pub(super) fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    pub(super) fn scan_root(&self) -> &CanonicalScanRoot {
        &self.scan_root
    }

    pub(super) fn target(&self) -> &CanonicalPathSnapshot {
        &self.target
    }

    pub(super) fn manifest(&self) -> &CanonicalPathSnapshot {
        self.manifest.path()
    }

    pub(super) fn cache_tag(&self) -> &CanonicalFilePrefixSnapshot {
        &self.cache_tag
    }

    pub(super) fn protected_path_is_still_unresolved(&self) -> bool {
        let _ = &self.protected_path_still_unresolved;
        true
    }
}
