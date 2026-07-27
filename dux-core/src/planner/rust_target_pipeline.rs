//! Private acquisition of live Rust-target planner input.
//!
//! This is the first production-shaped join between durable evaluator history
//! and the rule-specific live witness. It retains the exact snapshot lease
//! inside the non-cloneable witness and intentionally stops before Cargo
//! provenance, protected-root grants, plans, approval, journal, FFI, or an
//! effect boundary.

#![cfg(unix)]

use std::sync::Arc;

use thiserror::Error;

use crate::domain::{Candidate, CandidateId, ScanId};
use crate::path_validation::TrustedHomeMountWitness;
use crate::persistence::StoreCoordinator;
use crate::persistence::snapshot::SnapshotRepository;

#[cfg(target_os = "macos")]
use super::rule_scope_grant::{RuleScopeGrantError, authorize_rust_target};
use super::rust_target::{
    RustTargetLiveValidationError, RustTargetLiveWitness, validate_live_rust_target,
};
#[cfg(target_os = "macos")]
use super::rust_target_cargo::{
    CargoMetadataValidationError, RustTargetCargoPlanningProvenanceError,
    RustTargetRuleBoundaryError, validate_enrolled_cargo_metadata,
};
#[cfg(target_os = "macos")]
use super::rust_target_promotion::{
    RustTargetPromotion, RustTargetPromotionError, admit_rust_target_candidate,
};
use super::rust_target_source::{RustTargetSourceError, acquire_rust_target_durable_source};

#[derive(Debug, Error)]
pub(crate) enum RustTargetPipelineError {
    #[error("the Rust-target planner pipeline is closed")]
    Closed,
    #[error("durable Rust-target source acquisition failed: {0}")]
    Source(#[source] RustTargetSourceError),
    #[error("live Rust-target validation failed: {0}")]
    Live(#[source] RustTargetLiveValidationError),
    #[cfg(target_os = "macos")]
    #[error("enrolled Cargo metadata validation failed: {0}")]
    Cargo(#[source] CargoMetadataValidationError),
    #[cfg(target_os = "macos")]
    #[error("Rust-target Cargo provenance could not be retained: {0}")]
    Provenance(#[source] RustTargetCargoPlanningProvenanceError),
    #[cfg(target_os = "macos")]
    #[error("current-account home/mount evidence failed: {0}")]
    Location(#[source] crate::path_validation::TrustedHomeMountError),
    #[cfg(target_os = "macos")]
    #[error("Rust-target rule-boundary evidence failed: {0}")]
    Boundary(#[source] RustTargetRuleBoundaryError),
    #[cfg(target_os = "macos")]
    #[error("Rust-target scope authorization failed: {0}")]
    Authorization(#[source] RuleScopeGrantError),
    #[cfg(target_os = "macos")]
    #[error("Rust-target promotion admission failed: {0}")]
    Promotion(#[source] RustTargetPromotionError),
}

/// Acquire one exact durable candidate, rehydrate its domain policy, and turn
/// it into a fresh live witness. The witness owns the bounded cleanup-review
/// lease acquired by the source boundary; both values remain non-cloneable and
/// stop before Cargo authority, plans, journal claims, FFI, or effects.
#[must_use = "the live Rust-target input must be consumed by the next planner boundary"]
pub(crate) fn prepare_rust_target_live_input(
    store: Arc<StoreCoordinator>,
    snapshots: &SnapshotRepository,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
) -> Result<(Candidate, RustTargetLiveWitness), RustTargetPipelineError> {
    let source = acquire_rust_target_durable_source(store, snapshots, scan_id, candidate_id)
        .map_err(RustTargetPipelineError::Source)?;
    let candidate = source
        .candidate_for_promotion()
        .map_err(RustTargetPipelineError::Source)?;
    let witness = validate_live_rust_target(source).map_err(RustTargetPipelineError::Live)?;
    Ok((candidate, witness))
}

/// Consume the lease-backed live witness through enrolled Cargo, current
/// home/mount, protected-root, process, and descendant-policy boundaries. The
/// returned promotion token still retains the `ProtectedPath` blocker and has
/// no plan, approval, journal, FFI, scheduling, AI, or effect operation.
#[cfg(target_os = "macos")]
#[must_use = "the Rust-target promotion must be consumed by the next planner boundary"]
pub(crate) fn prepare_rust_target_promotion(
    store: Arc<StoreCoordinator>,
    snapshots: &SnapshotRepository,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
) -> Result<RustTargetPromotion, RustTargetPipelineError> {
    let (candidate, live) =
        prepare_rust_target_live_input(store, snapshots, scan_id, candidate_id)?;
    let scan_root = live.scan_root().clone();
    let target = live.target().clone();
    let cargo = validate_enrolled_cargo_metadata(live).map_err(RustTargetPipelineError::Cargo)?;
    let provenance = cargo
        .into_planning_provenance()
        .map_err(RustTargetPipelineError::Provenance)?;
    let location =
        TrustedHomeMountWitness::capture(&scan_root).map_err(RustTargetPipelineError::Location)?;
    let boundary = provenance
        .into_rule_boundary_evidence(location)
        .map_err(RustTargetPipelineError::Boundary)?;
    let authorization = authorize_rust_target(
        boundary,
        scan_id,
        candidate.id(),
        &scan_root,
        target.clone(),
        candidate.rule(),
    )
    .map_err(RustTargetPipelineError::Authorization)?;
    admit_rust_target_candidate(candidate, authorization, scan_id, &scan_root, &target)
        .map_err(RustTargetPipelineError::Promotion)
}
