#[cfg(unix)]
use std::path::Component;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use thiserror::Error;

#[cfg(unix)]
use crate::domain::verify_snapshot_candidate_evaluation;
use crate::domain::{Candidate, CandidateId, ScanId, candidate_from_complete_record};
#[cfg(unix)]
use crate::persistence::snapshot::{
    HostValue, SnapshotNode, SnapshotNodeKind, SnapshotReviewDocument,
};
use crate::persistence::snapshot::{
    SnapshotRepository, SnapshotRepositoryErrorKind, SnapshotReviewLease, SnapshotUnixIdentity,
};
use crate::persistence::{
    CandidateValidationSourceRecord, CleanupSessionId, CompleteCandidateRecord, HistoryErrorKind,
    SnapshotReviewPurpose, StoreCoordinator,
};

#[cfg(unix)]
use super::rust_target::{RustTargetCandidatePaths, validate_durable_candidate_for_source};

/// Exact durable discovery input retained for the lifetime of live evidence.
///
/// This source cannot be constructed from caller-provided paths, cloned,
/// serialized, converted to a plan, or used to remove a blocker. Its snapshot
/// lease pins the exact immutable snapshot object only until its bounded expiry. The
/// source must be revalidated immediately before every dependent live check;
/// merely holding this Rust value does not extend the durable pin.
#[must_use = "durable discovery evidence must remain alive during validation"]
pub(crate) struct RustTargetDurableSource {
    store: Arc<StoreCoordinator>,
    record: CandidateValidationSourceRecord,
    lease: AutoReleasingReviewLease,
    bindings: RustTargetSnapshotBindings,
    trusted_claim: Option<TrustedRustTargetClaimBinding>,
}

struct TrustedRustTargetClaimBinding {
    session_id: CleanupSessionId,
    item_ordinal: usize,
}

struct AutoReleasingReviewLease(Option<SnapshotReviewLease>);

impl AutoReleasingReviewLease {
    fn new(lease: SnapshotReviewLease) -> Self {
        Self(Some(lease))
    }

    fn lease(&self) -> &SnapshotReviewLease {
        self.0
            .as_ref()
            .expect("managed review lease remains present until consuming release")
    }

    fn expires_at(&self) -> Result<SystemTime, RustTargetSourceError> {
        self.lease().expires_at().map_err(map_snapshot)
    }

    fn release(mut self) -> Result<(), RustTargetSourceError> {
        self.0
            .take()
            .expect("managed review lease is present")
            .release()
            .map_err(map_snapshot)
    }
}

impl Drop for AutoReleasingReviewLease {
    fn drop(&mut self) {
        if let Some(lease) = self.0.take() {
            let _ = lease.release();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RustTargetSnapshotBindings {
    pub(super) root: SnapshotUnixIdentity,
    pub(super) target_ancestors: Vec<SnapshotUnixIdentity>,
    pub(super) target: SnapshotUnixIdentity,
    pub(super) manifest: SnapshotUnixIdentity,
    pub(super) cache_tag: SnapshotUnixIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(crate) enum RustTargetSourceError {
    #[cfg(not(unix))]
    #[error("durable Rust target validation is unsupported on this platform")]
    UnsupportedPlatform,
    #[cfg(unix)]
    #[error("snapshot repository and history store do not share one coordinator")]
    StoreMismatch,
    #[error("durable candidate history is unavailable: {kind:?}")]
    History { kind: HistoryErrorKind },
    #[error("the exact retained snapshot is unavailable: {kind:?}")]
    Snapshot { kind: SnapshotRepositoryErrorKind },
    #[cfg(unix)]
    #[error("the durable candidate is not the current staged Rust target rule")]
    CandidateMismatch,
    #[cfg(unix)]
    #[error("the snapshot does not contain the exact Rust target observation")]
    SnapshotMismatch,
    #[cfg(unix)]
    #[error("the retained snapshot does not reproduce the durable candidate evaluation")]
    EvaluatorReplayMismatch,
    #[error("durable discovery evidence changed during acquisition or use")]
    SourceChanged,
}

pub(crate) fn acquire_rust_target_durable_source(
    store: Arc<StoreCoordinator>,
    snapshots: &SnapshotRepository,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
) -> Result<RustTargetDurableSource, RustTargetSourceError> {
    #[cfg(not(unix))]
    {
        let _ = (store, snapshots, scan_id, candidate_id);
        return Err(RustTargetSourceError::UnsupportedPlatform);
    }
    #[cfg(unix)]
    acquire_rust_target_durable_source_with_clock_and_hook(
        store,
        snapshots,
        scan_id,
        candidate_id,
        SystemTime::now,
        || {},
    )
}

#[cfg(unix)]
fn acquire_rust_target_durable_source_with_clock_and_hook(
    store: Arc<StoreCoordinator>,
    snapshots: &SnapshotRepository,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
    mut current_time: impl FnMut() -> SystemTime,
    after_snapshot_load: impl FnOnce(),
) -> Result<RustTargetDurableSource, RustTargetSourceError> {
    if !snapshots.coordinates_store(&store) {
        return Err(RustTargetSourceError::StoreMismatch);
    }

    let before = load_source(&store, scan_id, candidate_id)?;
    let paths = validate_durable_candidate_for_source(scan_id, before.candidate())
        .map_err(|_| RustTargetSourceError::CandidateMismatch)?;
    let reference = before
        .scan()
        .snapshot()
        .ok_or(RustTargetSourceError::SnapshotMismatch)?
        .clone();
    let lease = AutoReleasingReviewLease::new(
        snapshots
            .acquire_review_lease(
                &reference,
                SnapshotReviewPurpose::CleanupReview,
                current_time(),
            )
            .map_err(map_snapshot)?,
    );
    let document = lease
        .lease()
        .load_for_review(current_time())
        .map_err(map_snapshot)?;
    verify_snapshot_candidate_evaluation(
        &document,
        before.scan().coverage(),
        before.evaluation_candidates(),
    )
    .map_err(|_| RustTargetSourceError::EvaluatorReplayMismatch)?;
    let bindings = snapshot_bindings(&document, before.scan().root(), &paths)?;
    lease
        .lease()
        .validate(current_time())
        .map_err(map_snapshot)?;

    after_snapshot_load();

    let after = load_source(&store, scan_id, candidate_id)?;
    if before != after
        || lease.lease().reference()
            != after
                .scan()
                .snapshot()
                .ok_or(RustTargetSourceError::SourceChanged)?
    {
        return Err(RustTargetSourceError::SourceChanged);
    }
    lease
        .lease()
        .validate(current_time())
        .map_err(map_snapshot)?;

    Ok(RustTargetDurableSource {
        store,
        record: after,
        lease,
        bindings,
        trusted_claim: None,
    })
}

impl RustTargetDurableSource {
    pub(super) fn store(&self) -> &Arc<StoreCoordinator> {
        &self.store
    }

    pub(super) fn scan_id(&self) -> &ScanId {
        self.record.scan().id()
    }

    pub(super) fn scan_root(&self) -> &Path {
        self.record.scan().root()
    }

    pub(super) fn candidate(&self) -> &CompleteCandidateRecord {
        self.record.candidate()
    }

    /// Rehydrate the exact domain candidate for the next planner join. The
    /// current bundled catalog supplies policy fields; the helper compares
    /// every immutable body field back to this source's retained record.
    pub(crate) fn candidate_for_promotion(&self) -> Result<Candidate, RustTargetSourceError> {
        candidate_from_complete_record(self.candidate())
            .map_err(|_| RustTargetSourceError::CandidateMismatch)
    }

    pub(super) fn bindings(&self) -> RustTargetSnapshotBindings {
        self.bindings.clone()
    }

    pub(super) fn expires_at(&self) -> Result<SystemTime, RustTargetSourceError> {
        self.lease.expires_at()
    }

    #[cfg(test)]
    pub(super) fn purpose(&self) -> SnapshotReviewPurpose {
        self.lease.lease().purpose()
    }

    pub(super) fn revalidate_current(&self) -> Result<(), RustTargetSourceError> {
        self.lease
            .lease()
            .validate(SystemTime::now())
            .map_err(map_snapshot)?;
        let current = match self.trusted_claim.as_ref() {
            Some(binding) => load_source_for_trusted_claim(
                &self.store,
                self.record.scan().id(),
                self.record.candidate().id(),
                &binding.session_id,
                binding.item_ordinal,
            )?,
            None => load_source(
                &self.store,
                self.record.scan().id(),
                self.record.candidate().id(),
            )?,
        };
        let source_matches = if self.trusted_claim.is_some() {
            self.record.exactly_matches_after_trusted_claim(&current)
        } else {
            current == self.record
        };
        if !source_matches || current.scan().snapshot() != Some(self.lease.lease().reference()) {
            return Err(RustTargetSourceError::SourceChanged);
        }
        self.lease
            .lease()
            .validate(SystemTime::now())
            .map_err(map_snapshot)
    }

    pub(super) fn bind_trusted_claim(
        &mut self,
        session_id: CleanupSessionId,
        item_ordinal: usize,
    ) -> Result<(), RustTargetSourceError> {
        if self.trusted_claim.is_some() {
            return Err(RustTargetSourceError::SourceChanged);
        }
        let current = load_source_for_trusted_claim(
            &self.store,
            self.record.scan().id(),
            self.record.candidate().id(),
            &session_id,
            item_ordinal,
        )?;
        if !self.record.exactly_matches_after_trusted_claim(&current)
            || current.scan().snapshot() != Some(self.lease.lease().reference())
        {
            return Err(RustTargetSourceError::SourceChanged);
        }
        self.trusted_claim = Some(TrustedRustTargetClaimBinding {
            session_id,
            item_ordinal,
        });
        self.revalidate_current()
    }

    pub(crate) fn release(self) -> Result<(), RustTargetSourceError> {
        self.lease.release()
    }
}

fn load_source(
    store: &StoreCoordinator,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
) -> Result<CandidateValidationSourceRecord, RustTargetSourceError> {
    store
        .load_candidate_validation_source(scan_id, candidate_id)
        .map_err(|error| RustTargetSourceError::History { kind: error.kind })
}

fn load_source_for_trusted_claim(
    store: &StoreCoordinator,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
    session_id: &CleanupSessionId,
    item_ordinal: usize,
) -> Result<CandidateValidationSourceRecord, RustTargetSourceError> {
    store
        .load_candidate_validation_source_for_trusted_claim(
            scan_id,
            candidate_id,
            session_id,
            item_ordinal,
        )
        .map_err(|error| RustTargetSourceError::History { kind: error.kind })
}

fn map_snapshot(
    error: crate::persistence::snapshot::SnapshotRepositoryError,
) -> RustTargetSourceError {
    RustTargetSourceError::Snapshot { kind: error.kind }
}

#[cfg(unix)]
fn snapshot_bindings(
    document: &SnapshotReviewDocument,
    scan_root: &Path,
    paths: &RustTargetCandidatePaths,
) -> Result<RustTargetSnapshotBindings, RustTargetSourceError> {
    if !document.metadata.root.matches_path(scan_root) {
        return Err(RustTargetSourceError::SnapshotMismatch);
    }
    let root = document
        .nodes
        .first()
        .filter(|node| {
            node.id == 0
                && node.parent.is_none()
                && node.depth == 0
                && node.kind == SnapshotNodeKind::Directory
                && node.name.is_none()
        })
        .and_then(|node| node.unix_identity)
        .ok_or(RustTargetSourceError::SnapshotMismatch)?;

    let (target, target_ancestors) = locate_node(
        document,
        scan_root,
        &paths.target,
        SnapshotNodeKind::Directory,
    )?;
    let (manifest, _) = locate_node(document, scan_root, &paths.manifest, SnapshotNodeKind::File)?;
    let (cache_tag, _) = locate_node(
        document,
        scan_root,
        &paths.cache_tag,
        SnapshotNodeKind::File,
    )?;

    if manifest.parent != target.parent || cache_tag.parent != Some(target.id) {
        return Err(RustTargetSourceError::SnapshotMismatch);
    }

    Ok(RustTargetSnapshotBindings {
        root,
        target_ancestors,
        target: target
            .unix_identity
            .ok_or(RustTargetSourceError::SnapshotMismatch)?,
        manifest: manifest
            .unix_identity
            .ok_or(RustTargetSourceError::SnapshotMismatch)?,
        cache_tag: cache_tag
            .unix_identity
            .ok_or(RustTargetSourceError::SnapshotMismatch)?,
    })
}

#[cfg(unix)]
fn locate_node<'a>(
    document: &'a SnapshotReviewDocument,
    scan_root: &Path,
    path: &Path,
    expected_kind: SnapshotNodeKind,
) -> Result<(&'a SnapshotNode, Vec<SnapshotUnixIdentity>), RustTargetSourceError> {
    let relative = path
        .strip_prefix(scan_root)
        .map_err(|_| RustTargetSourceError::SnapshotMismatch)?;
    let mut node_index = 0_usize;
    let mut located = None;
    let mut ancestors = Vec::new();
    ancestors
        .try_reserve_exact(relative.components().count())
        .map_err(|_| RustTargetSourceError::SnapshotMismatch)?;
    ancestors.push(
        document.nodes[0]
            .unix_identity
            .ok_or(RustTargetSourceError::SnapshotMismatch)?,
    );
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(RustTargetSourceError::SnapshotMismatch);
        };
        let name = HostValue::from_component(component)
            .map_err(|_| RustTargetSourceError::SnapshotMismatch)?;
        let child_indices = document
            .direct_child_indices(node_index)
            .map_err(map_snapshot)?;
        let mut matches = child_indices.iter().filter_map(|index| {
            let index = usize::try_from(*index).ok()?;
            let node = document.nodes.get(index)?;
            (node.name.as_ref() == Some(&name)).then_some((index, node))
        });
        let (next_index, node) = matches
            .next()
            .ok_or(RustTargetSourceError::SnapshotMismatch)?;
        if matches.next().is_some() {
            return Err(RustTargetSourceError::SnapshotMismatch);
        }
        if located.is_some() {
            ancestors.push(
                document.nodes[node_index]
                    .unix_identity
                    .ok_or(RustTargetSourceError::SnapshotMismatch)?,
            );
        }
        node_index = next_index;
        located = Some(node);
    }
    let node = located.ok_or(RustTargetSourceError::SnapshotMismatch)?;
    if node.kind != expected_kind {
        return Err(RustTargetSourceError::SnapshotMismatch);
    }
    Ok((node, ancestors))
}

#[cfg(all(test, unix))]
pub(super) fn acquire_rust_target_durable_source_after_snapshot_hook_for_test(
    store: Arc<StoreCoordinator>,
    snapshots: &SnapshotRepository,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
    observed_at: SystemTime,
    after_snapshot_load: impl FnOnce(),
) -> Result<RustTargetDurableSource, RustTargetSourceError> {
    acquire_rust_target_durable_source_with_clock_and_hook(
        store,
        snapshots,
        scan_id,
        candidate_id,
        || observed_at,
        after_snapshot_load,
    )
}

#[cfg(all(test, unix))]
pub(super) fn acquire_rust_target_durable_source_with_clock_for_test(
    store: Arc<StoreCoordinator>,
    snapshots: &SnapshotRepository,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
    current_time: impl FnMut() -> SystemTime,
) -> Result<RustTargetDurableSource, RustTargetSourceError> {
    acquire_rust_target_durable_source_with_clock_and_hook(
        store,
        snapshots,
        scan_id,
        candidate_id,
        current_time,
        || {},
    )
}
