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
use crate::path_validation::{CanonicalPathSnapshot, CanonicalScanRoot};
use crate::persistence::HistoryErrorKind;
use crate::persistence::StoreCoordinator;
use crate::persistence::snapshot::{
    SnapshotCodecErrorKind, SnapshotRepository, SnapshotRepositoryErrorKind,
    SnapshotStorageErrorKind,
};

#[cfg(target_os = "macos")]
use super::process_activity::ProcessActivityError;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustTargetPlanReviewFailure {
    Closed,
    CandidateUnavailable,
    CargoNotEnrolled,
    ActiveProcesses,
    ChangedDuringReview,
    UnsupportedPlatform,
    BudgetExceeded,
    Busy,
    UnsafeStorage,
    CorruptData,
    Unavailable,
    InternalState,
}

impl RustTargetPipelineError {
    pub(crate) fn plan_review_failure(&self) -> RustTargetPlanReviewFailure {
        match self {
            Self::Closed => RustTargetPlanReviewFailure::Closed,
            Self::Source(RustTargetSourceError::History { kind }) => {
                map_history_review_failure(*kind)
            }
            Self::Source(RustTargetSourceError::Snapshot { kind }) => match kind {
                SnapshotRepositoryErrorKind::ReviewLeaseExpired
                | SnapshotRepositoryErrorKind::MissingSnapshot
                | SnapshotRepositoryErrorKind::MissingStore
                | SnapshotRepositoryErrorKind::SnapshotUnavailable
                | SnapshotRepositoryErrorKind::ReferenceMismatch => {
                    RustTargetPlanReviewFailure::CandidateUnavailable
                }
                SnapshotRepositoryErrorKind::IncompatibleVersion => {
                    RustTargetPlanReviewFailure::CorruptData
                }
                SnapshotRepositoryErrorKind::Codec(kind) => map_codec_review_failure(*kind),
                SnapshotRepositoryErrorKind::Storage(kind) => {
                    map_snapshot_storage_review_failure(*kind)
                }
                SnapshotRepositoryErrorKind::History(kind) => map_history_review_failure(*kind),
                SnapshotRepositoryErrorKind::ReadOnly => RustTargetPlanReviewFailure::Unavailable,
            },
            Self::Source(_) => RustTargetPlanReviewFailure::CandidateUnavailable,
            Self::Live(RustTargetLiveValidationError::UnsupportedPlatform) => {
                RustTargetPlanReviewFailure::UnsupportedPlatform
            }
            Self::Live(_) => RustTargetPlanReviewFailure::ChangedDuringReview,
            #[cfg(target_os = "macos")]
            Self::Cargo(CargoMetadataValidationError::CargoNotEnrolled) => {
                RustTargetPlanReviewFailure::CargoNotEnrolled
            }
            #[cfg(target_os = "macos")]
            Self::Cargo(CargoMetadataValidationError::CargoEnrollmentStore { kind }) => {
                map_history_review_failure(*kind)
            }
            #[cfg(target_os = "macos")]
            Self::Boundary(error) => map_boundary_review_failure(error),
            #[cfg(target_os = "macos")]
            Self::Authorization(error) => map_authorization_review_failure(error),
            #[cfg(target_os = "macos")]
            Self::Promotion(error) => map_promotion_review_failure(error),
            #[cfg(target_os = "macos")]
            Self::Cargo(_) | Self::Provenance(_) | Self::Location(_) => {
                RustTargetPlanReviewFailure::ChangedDuringReview
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn map_boundary_review_failure(error: &RustTargetRuleBoundaryError) -> RustTargetPlanReviewFailure {
    match error {
        RustTargetRuleBoundaryError::ProcessActivity(
            ProcessActivityError::Active | ProcessActivityError::RequiredCargoQuiescence,
        ) => RustTargetPlanReviewFailure::ActiveProcesses,
        RustTargetRuleBoundaryError::ProcessActivity(
            ProcessActivityError::ProcessLimitExceeded,
        ) => RustTargetPlanReviewFailure::BudgetExceeded,
        RustTargetRuleBoundaryError::ProcessActivity(ProcessActivityError::UnsupportedPlatform) => {
            RustTargetPlanReviewFailure::UnsupportedPlatform
        }
        _ => RustTargetPlanReviewFailure::ChangedDuringReview,
    }
}

#[cfg(target_os = "macos")]
fn map_authorization_review_failure(error: &RuleScopeGrantError) -> RustTargetPlanReviewFailure {
    match error {
        RuleScopeGrantError::CargoBoundary(error) => map_boundary_review_failure(error),
        _ => RustTargetPlanReviewFailure::ChangedDuringReview,
    }
}

#[cfg(target_os = "macos")]
fn map_promotion_review_failure(error: &RustTargetPromotionError) -> RustTargetPlanReviewFailure {
    match error {
        RustTargetPromotionError::Authorization(error) => map_authorization_review_failure(error),
        _ => RustTargetPlanReviewFailure::ChangedDuringReview,
    }
}

fn map_history_review_failure(kind: HistoryErrorKind) -> RustTargetPlanReviewFailure {
    match kind {
        HistoryErrorKind::NotFound | HistoryErrorKind::InvalidTransition => {
            RustTargetPlanReviewFailure::CandidateUnavailable
        }
        HistoryErrorKind::QueryLimitExceeded => RustTargetPlanReviewFailure::BudgetExceeded,
        HistoryErrorKind::Busy => RustTargetPlanReviewFailure::Busy,
        HistoryErrorKind::UnsafeStorage => RustTargetPlanReviewFailure::UnsafeStorage,
        HistoryErrorKind::CorruptData | HistoryErrorKind::IncompatibleSchema => {
            RustTargetPlanReviewFailure::CorruptData
        }
        HistoryErrorKind::DatabaseUnavailable | HistoryErrorKind::OutcomeUnknown => {
            RustTargetPlanReviewFailure::Unavailable
        }
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::InternalState => RustTargetPlanReviewFailure::ChangedDuringReview,
    }
}

fn map_codec_review_failure(kind: SnapshotCodecErrorKind) -> RustTargetPlanReviewFailure {
    match kind {
        SnapshotCodecErrorKind::LimitExceeded => RustTargetPlanReviewFailure::BudgetExceeded,
        SnapshotCodecErrorKind::Io => RustTargetPlanReviewFailure::Unavailable,
        SnapshotCodecErrorKind::IncompatibleVersion
        | SnapshotCodecErrorKind::InvalidMagic
        | SnapshotCodecErrorKind::InvalidLength
        | SnapshotCodecErrorKind::ChecksumMismatch
        | SnapshotCodecErrorKind::CorruptData => RustTargetPlanReviewFailure::CorruptData,
        SnapshotCodecErrorKind::InvalidInput => RustTargetPlanReviewFailure::InternalState,
    }
}

fn map_snapshot_storage_review_failure(
    kind: SnapshotStorageErrorKind,
) -> RustTargetPlanReviewFailure {
    match kind {
        SnapshotStorageErrorKind::UnsafeRoot
        | SnapshotStorageErrorKind::UnsafeObject
        | SnapshotStorageErrorKind::UnrecognizedStore => RustTargetPlanReviewFailure::UnsafeStorage,
        SnapshotStorageErrorKind::Busy => RustTargetPlanReviewFailure::Busy,
        SnapshotStorageErrorKind::Unavailable => RustTargetPlanReviewFailure::Unavailable,
        SnapshotStorageErrorKind::InvalidConfiguration
        | SnapshotStorageErrorKind::InternalState => RustTargetPlanReviewFailure::InternalState,
    }
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
    let (promotion, _, _) =
        prepare_rust_target_promotion_with_witness(store, snapshots, scan_id, candidate_id)?;
    Ok(promotion)
}

#[cfg(target_os = "macos")]
fn prepare_rust_target_promotion_with_witness(
    store: Arc<StoreCoordinator>,
    snapshots: &SnapshotRepository,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
) -> Result<
    (
        RustTargetPromotion,
        CanonicalScanRoot,
        CanonicalPathSnapshot,
    ),
    RustTargetPipelineError,
> {
    let (candidate, live) =
        prepare_rust_target_live_input(store, snapshots, scan_id, candidate_id)?;
    let authority_expires_at = live.expires_at().map_err(RustTargetPipelineError::Live)?;
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
    let promotion = admit_rust_target_candidate(
        candidate,
        authorization,
        scan_id,
        &scan_root,
        &target,
        authority_expires_at,
    )
    .map_err(RustTargetPipelineError::Promotion)?;
    Ok((promotion, scan_root, target))
}

/// Consume the private promotion token into the typed permanent-safe facts
/// boundary. This repeats grant validation and plan-shape checks while
/// retaining the original blocked candidate; it still creates no reviewed
/// plan, approval, journal claim, FFI value, schedule, AI request, or effect.
#[cfg(target_os = "macos")]
#[must_use = "Rust-target plan facts must be consumed by the reviewed planner boundary"]
pub(crate) fn prepare_rust_target_plan_facts(
    store: Arc<StoreCoordinator>,
    snapshots: &SnapshotRepository,
    scan_id: &ScanId,
    candidate_id: &CandidateId,
) -> Result<super::rust_target_promotion::RustTargetPlanFacts, RustTargetPipelineError> {
    let (promotion, scan_root, target) =
        prepare_rust_target_promotion_with_witness(store, snapshots, scan_id, candidate_id)?;
    promotion
        .into_plan_facts(scan_root, target)
        .map_err(RustTargetPipelineError::Promotion)
}

#[cfg(test)]
mod review_failure_tests {
    use super::*;

    #[test]
    fn nested_snapshot_failures_preserve_retry_and_safety_categories() {
        assert_eq!(
            map_history_review_failure(HistoryErrorKind::QueryLimitExceeded),
            RustTargetPlanReviewFailure::BudgetExceeded
        );
        assert_eq!(
            map_history_review_failure(HistoryErrorKind::Busy),
            RustTargetPlanReviewFailure::Busy
        );
        assert_eq!(
            map_history_review_failure(HistoryErrorKind::UnsafeStorage),
            RustTargetPlanReviewFailure::UnsafeStorage
        );
        assert_eq!(
            map_history_review_failure(HistoryErrorKind::CorruptData),
            RustTargetPlanReviewFailure::CorruptData
        );
        assert_eq!(
            map_codec_review_failure(SnapshotCodecErrorKind::LimitExceeded),
            RustTargetPlanReviewFailure::BudgetExceeded
        );
        assert_eq!(
            map_codec_review_failure(SnapshotCodecErrorKind::Io),
            RustTargetPlanReviewFailure::Unavailable
        );
        assert_eq!(
            map_snapshot_storage_review_failure(SnapshotStorageErrorKind::Busy),
            RustTargetPlanReviewFailure::Busy
        );
        assert_eq!(
            map_snapshot_storage_review_failure(SnapshotStorageErrorKind::UnsafeRoot),
            RustTargetPlanReviewFailure::UnsafeStorage
        );
        assert_eq!(
            map_snapshot_storage_review_failure(SnapshotStorageErrorKind::Unavailable),
            RustTargetPlanReviewFailure::Unavailable
        );
    }
}
