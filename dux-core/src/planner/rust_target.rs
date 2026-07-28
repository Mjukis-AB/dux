use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::domain::{
    BlockReason, Candidate, CandidateAction, CandidateCategory, CandidateId, Evidence, SafetyTier,
    ScanId, current_rust_target_candidate_id,
};
use crate::path_validation::{
    CanonicalFileDigestError, CanonicalFileDigestSnapshot, CanonicalFilePrefixError,
    CanonicalFilePrefixSnapshot, CanonicalPathError, CanonicalPathSnapshot, CanonicalScanRoot,
    FilesystemEntryKind, LexicalPathError, capture_path_snapshot, capture_regular_file_prefix,
    capture_regular_file_sha256, capture_scan_root, validate_cleanup_path, validate_scan_root,
};
use crate::persistence::{CleanupSessionId, CompleteCandidateRecord};

use super::rust_target_source::{
    RustTargetDurableSource, RustTargetSnapshotBindings, RustTargetSourceError,
};

const RUST_TARGET_RULE_ID: &str = "developer.rust.target";
const RUST_TARGET_RULE_REVISION: u32 = 2;
pub(super) const RUST_TARGET_WITNESS_REVISION: u32 = 1;
const CARGO_CACHE_TAG_SIGNATURE: &[u8; 43] = b"Signature: 8a477f597d28d172789f06886806bc55";
const MAX_CARGO_MANIFEST_BYTES: usize = 4 * 1024 * 1024;

/// Historical source binding supplied by the future planner's durable loader.
///
/// The path remains a locator until `validate_live_rust_target` reconstructs
/// all live filesystem evidence. This type itself grants no authority.
#[cfg(test)]
pub(crate) struct RustTargetValidationSource<'a> {
    source_scan_id: &'a ScanId,
    scan_root: &'a Path,
}

#[cfg(test)]
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
    durable_source: Option<RustTargetDurableSource>,
    protected_path_still_unresolved: ProtectedPathStillUnresolved,
}

struct ProtectedPathStillUnresolved;

/// Rule-specific evidence retained immediately before a future permanent-safe
/// effect. This witness proves the default Cargo target layout, the regular
/// manifest identity/content, and the exact cache-tag signature. It remains
/// non-cloneable and has no mutation or path-export API.
#[must_use = "the Rust-target effect witness must be consumed by the executor"]
pub(crate) struct RustTargetEffectWitness {
    target: CanonicalPathSnapshot,
    manifest: CanonicalFileDigestSnapshot,
    cache_tag: CanonicalFilePrefixSnapshot,
}

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
    #[error("live Rust target objects do not match their exact scan-time identities")]
    ChangedSinceScan,
    #[error("durable Rust target discovery evidence changed during validation")]
    DurableSourceChanged,
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
    source: RustTargetDurableSource,
) -> Result<RustTargetLiveWitness, RustTargetLiveValidationError> {
    source
        .revalidate_current()
        .map_err(|_| RustTargetLiveValidationError::DurableSourceChanged)?;
    let paths = validate_durable_candidate_for_source(source.scan_id(), source.candidate())?;
    let source_scan_id = source.scan_id().clone();
    let candidate_id = source.candidate().id().clone();
    let scan_root = source.scan_root().to_path_buf();
    let bindings = source.bindings();
    validate_live_rust_target_inner(
        source_scan_id,
        candidate_id,
        scan_root,
        paths,
        Some(source),
        Some(bindings),
    )
}

/// Capture the rule-specific evidence required immediately before a
/// permanent-safe Rust-target operation. Unlike the discovery witness, this
/// boundary accepts only the trusted exact target snapshot; it never clears a
/// candidate blocker or creates a plan.
pub(crate) fn validate_rust_target_effect(
    target: CanonicalPathSnapshot,
) -> Result<RustTargetEffectWitness, RustTargetLiveValidationError> {
    #[cfg(not(unix))]
    {
        let _ = target;
        return Err(RustTargetLiveValidationError::UnsupportedPlatform);
    }

    #[cfg(unix)]
    {
        let lexical_root = validate_scan_root(target.scan_root())?;
        let scan_root = capture_scan_root(lexical_root.clone())?;
        let lexical_target = validate_cleanup_path(&lexical_root, target.requested_path())?;
        let current_target = capture_path_snapshot(&scan_root, lexical_target.clone())?;
        if current_target != target {
            return Err(RustTargetLiveValidationError::ChangedDuringValidation);
        }
        if target.target_kind() != FilesystemEntryKind::Directory
            || target.canonical_path().file_name() != Some(OsStr::new("target"))
        {
            return Err(RustTargetLiveValidationError::LayoutMismatch);
        }

        let project_root = target
            .canonical_path()
            .parent()
            .ok_or(RustTargetLiveValidationError::LayoutMismatch)?;
        let manifest_path = project_root.join("Cargo.toml");
        let cache_tag_path = target.canonical_path().join("CACHEDIR.TAG");
        let lexical_manifest = validate_cleanup_path(&lexical_root, &manifest_path)?;
        let lexical_cache_tag = validate_cleanup_path(&lexical_root, &cache_tag_path)?;
        let manifest = capture_regular_file_sha256(
            &scan_root,
            lexical_manifest.clone(),
            MAX_CARGO_MANIFEST_BYTES,
        )?;
        let cache_tag = capture_regular_file_prefix(
            &scan_root,
            lexical_cache_tag.clone(),
            CARGO_CACHE_TAG_SIGNATURE.len(),
        )?;
        if manifest.path().hard_link_count() != 1 || cache_tag.path().hard_link_count() != 1 {
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
        let final_cache_tag = capture_regular_file_prefix(
            &final_root,
            lexical_cache_tag,
            CARGO_CACHE_TAG_SIGNATURE.len(),
        )?;
        if final_root != scan_root
            || final_target != target
            || final_manifest != manifest
            || final_cache_tag != cache_tag
        {
            return Err(RustTargetLiveValidationError::ChangedDuringValidation);
        }

        Ok(RustTargetEffectWitness {
            target,
            manifest,
            cache_tag,
        })
    }
}

#[cfg(test)]
pub(super) fn validate_live_rust_target_for_test(
    source: RustTargetValidationSource<'_>,
    candidate: &Candidate,
) -> Result<RustTargetLiveWitness, RustTargetLiveValidationError> {
    validate_candidate_policy(source.source_scan_id, candidate)?;
    let paths = validate_candidate_layout(candidate.paths(), candidate.evidence())?;
    validate_live_rust_target_inner(
        candidate.source_scan_id().clone(),
        candidate.id().clone(),
        source.scan_root.to_path_buf(),
        paths,
        None,
        None,
    )
}

fn validate_live_rust_target_inner(
    source_scan_id: ScanId,
    candidate_id: CandidateId,
    source_scan_root: PathBuf,
    paths: RustTargetCandidatePaths,
    durable_source: Option<RustTargetDurableSource>,
    snapshot_bindings: Option<RustTargetSnapshotBindings>,
) -> Result<RustTargetLiveWitness, RustTargetLiveValidationError> {
    let RustTargetCandidatePaths {
        target: target_path,
        manifest: manifest_path,
        cache_tag: cache_tag_path,
    } = paths;

    #[cfg(not(unix))]
    {
        let _ = (
            source_scan_id,
            candidate_id,
            source_scan_root,
            target_path,
            manifest_path,
            cache_tag_path,
            durable_source,
            snapshot_bindings,
        );
        return Err(RustTargetLiveValidationError::UnsupportedPlatform);
    }

    #[cfg(unix)]
    {
        let lexical_root = validate_scan_root(&source_scan_root)?;
        let scan_root = capture_scan_root(lexical_root.clone())?;

        let lexical_target = validate_cleanup_path(&lexical_root, &target_path)?;
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

        if let Some(bindings) = snapshot_bindings
            && (!matches_snapshot_identity(scan_root.identity(), bindings.root)
                || !matches_snapshot_ancestors(target.ancestors(), &bindings.target_ancestors)
                || !matches_snapshot_identity(target.target_identity(), bindings.target)
                || !matches_snapshot_identity(manifest.path().target_identity(), bindings.manifest)
                || !matches_snapshot_identity(
                    cache_tag.path().target_identity(),
                    bindings.cache_tag,
                ))
        {
            return Err(RustTargetLiveValidationError::ChangedSinceScan);
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

        if let Some(source) = durable_source.as_ref() {
            source
                .revalidate_current()
                .map_err(|_| RustTargetLiveValidationError::DurableSourceChanged)?;
        }

        Ok(RustTargetLiveWitness {
            witness_revision: RUST_TARGET_WITNESS_REVISION,
            source_scan_id,
            candidate_id,
            scan_root,
            target,
            manifest,
            cache_tag,
            durable_source,
            protected_path_still_unresolved: ProtectedPathStillUnresolved,
        })
    }
}

#[cfg(unix)]
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

    /// Compare every immutable discovery fact against the retained durable
    /// source record. Candidate IDs intentionally do not cover byte totals,
    /// timestamps, or evidence, so promotion must use this full-body join.
    pub(super) fn matches_durable_candidate(&self, candidate: &crate::domain::Candidate) -> bool {
        let Some(source) = self.durable_source.as_ref() else {
            return false;
        };
        let record = source.candidate();
        candidate.id() == record.id()
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
            && candidate.blockers() == record.blockers()
    }

    pub(super) fn protected_path_is_still_unresolved(&self) -> bool {
        let _ = &self.protected_path_still_unresolved;
        true
    }

    pub(super) fn store(&self) -> Option<&std::sync::Arc<crate::persistence::StoreCoordinator>> {
        self.durable_source
            .as_ref()
            .map(RustTargetDurableSource::store)
    }

    pub(super) fn expires_at(
        &self,
    ) -> Result<std::time::SystemTime, RustTargetLiveValidationError> {
        self.durable_source
            .as_ref()
            .ok_or(RustTargetLiveValidationError::DurableSourceChanged)?
            .expires_at()
            .map_err(|_| RustTargetLiveValidationError::DurableSourceChanged)
    }

    pub(super) fn revalidate_current(&self) -> Result<(), RustTargetLiveValidationError> {
        if let Some(source) = self.durable_source.as_ref() {
            source
                .revalidate_current()
                .map_err(|_| RustTargetLiveValidationError::DurableSourceChanged)?;
        }
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
        if let Some(source) = self.durable_source.as_ref() {
            source
                .revalidate_current()
                .map_err(|_| RustTargetLiveValidationError::DurableSourceChanged)?;
        }
        Ok(())
    }

    pub(super) fn bind_trusted_claim(
        &mut self,
        session_id: CleanupSessionId,
        item_ordinal: usize,
    ) -> Result<(), RustTargetLiveValidationError> {
        self.durable_source
            .as_mut()
            .ok_or(RustTargetLiveValidationError::DurableSourceChanged)?
            .bind_trusted_claim(session_id, item_ordinal)
            .map_err(|_| RustTargetLiveValidationError::DurableSourceChanged)?;
        self.revalidate_current()
    }

    pub(super) fn project_root(&self) -> &Path {
        self.manifest
            .path()
            .canonical_path()
            .parent()
            .expect("validated Cargo.toml always has a parent")
    }

    pub(super) fn scan_root(&self) -> &CanonicalScanRoot {
        &self.scan_root
    }

    pub(super) fn manifest_path(&self) -> &Path {
        self.manifest.path().canonical_path()
    }

    pub(super) fn target_path(&self) -> &Path {
        self.target.canonical_path()
    }

    pub(crate) fn release(self) -> Result<(), RustTargetSourceError> {
        match self.durable_source {
            Some(source) => source.release(),
            None => Ok(()),
        }
    }
}

impl RustTargetEffectWitness {
    pub(crate) fn revalidate_current(&self) -> Result<(), RustTargetLiveValidationError> {
        let current = validate_rust_target_effect(self.target.clone())?;
        if current.target != self.target
            || current.manifest != self.manifest
            || current.cache_tag != self.cache_tag
        {
            return Err(RustTargetLiveValidationError::ChangedDuringValidation);
        }
        Ok(())
    }

    pub(crate) fn target_path(&self) -> &Path {
        self.target.canonical_path()
    }

    pub(crate) fn target_identity(&self) -> crate::path_validation::FilesystemIdentity {
        self.target.target_identity()
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

pub(super) struct RustTargetCandidatePaths {
    pub(super) target: PathBuf,
    pub(super) manifest: PathBuf,
    pub(super) cache_tag: PathBuf,
}

pub(super) fn validate_durable_candidate_for_source(
    source_scan_id: &ScanId,
    candidate: &CompleteCandidateRecord,
) -> Result<RustTargetCandidatePaths, RustTargetLiveValidationError> {
    if candidate.source_scan_id() != source_scan_id {
        return Err(RustTargetLiveValidationError::SourceScanMismatch);
    }
    if candidate.rule().id().as_str() != RUST_TARGET_RULE_ID
        || candidate.rule().revision().get() != RUST_TARGET_RULE_REVISION
        || candidate.category() != CandidateCategory::DeveloperArtifact
        || candidate.safety() != SafetyTier::SafeRegenerable
        || candidate.action() != CandidateAction::RemoveKnownRegenerableContents
        || candidate.rule_schedule_eligible()
    {
        return Err(RustTargetLiveValidationError::CandidatePolicyMismatch);
    }
    if candidate.blockers() != [BlockReason::ProtectedPath] {
        return Err(RustTargetLiveValidationError::ProtectedPathNotUnresolved);
    }
    let paths = validate_candidate_layout(candidate.paths(), candidate.evidence())?;
    let expected_id = current_rust_target_candidate_id(source_scan_id, &paths.target)
        .map_err(|_| RustTargetLiveValidationError::CandidatePolicyMismatch)?;
    if candidate.id() != &expected_id {
        return Err(RustTargetLiveValidationError::CandidatePolicyMismatch);
    }
    Ok(paths)
}

fn validate_candidate_layout(
    candidate_paths: &[PathBuf],
    candidate_evidence: &[Evidence],
) -> Result<RustTargetCandidatePaths, RustTargetLiveValidationError> {
    let [target_path] = candidate_paths else {
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
    for evidence in candidate_evidence {
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
    if candidate_evidence.len() != 3
        || matched_target != 1
        || matched_manifest != 1
        || matched_cache_tag != 1
    {
        return Err(RustTargetLiveValidationError::CandidateEvidenceMismatch);
    }

    Ok(RustTargetCandidatePaths {
        target: target_path.clone(),
        manifest: manifest_path,
        cache_tag: cache_tag_path,
    })
}

#[cfg(unix)]
fn matches_snapshot_identity(
    live: crate::path_validation::FilesystemIdentity,
    snapshot: crate::persistence::snapshot::SnapshotUnixIdentity,
) -> bool {
    live.volume() == snapshot.device() && live.object() == u128::from(snapshot.inode())
}

#[cfg(unix)]
fn matches_snapshot_ancestors(
    live: &[crate::path_validation::AncestorIdentity],
    snapshot: &[crate::persistence::snapshot::SnapshotUnixIdentity],
) -> bool {
    live.len() == snapshot.len()
        && live
            .iter()
            .zip(snapshot)
            .all(|(live, snapshot)| matches_snapshot_identity(live.identity(), *snapshot))
}

impl RustTargetLiveWitness {
    pub(super) fn target(&self) -> &CanonicalPathSnapshot {
        &self.target
    }

    #[cfg(test)]
    pub(super) fn manifest(&self) -> &CanonicalPathSnapshot {
        self.manifest.path()
    }

    #[cfg(test)]
    pub(super) fn cache_tag(&self) -> &CanonicalFilePrefixSnapshot {
        &self.cache_tag
    }
}
