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
use crate::persistence::StoreCoordinator;
use crate::persistence::snapshot::SnapshotRepository;

use super::rust_target::{
    RustTargetLiveValidationError, RustTargetLiveWitness, validate_live_rust_target,
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
