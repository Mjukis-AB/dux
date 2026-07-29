//! Ephemeral, planner-owned live observations.
//!
//! Nothing in this module grants protected-root authority or can be converted
//! into a cleanup plan. The first witness is deliberately staged while the
//! remaining Cargo, process, mount, approval, and executor guards are absent.

#[cfg(target_os = "macos")]
mod cargo_code_signature_macos;
#[cfg(unix)]
mod cargo_config;
#[cfg(unix)]
mod cargo_config_closure;
#[cfg(unix)]
mod cargo_manifest_probes;
#[cfg(unix)]
mod cargo_package_metadata;
#[cfg(target_os = "macos")]
mod cargo_spawn_macos;
#[cfg(unix)]
mod cargo_target_namespace;
#[cfg(unix)]
mod cargo_workspace;
#[cfg(unix)]
mod cargo_workspace_glob;
mod descendant_policy;
mod exact_path_review;
mod process_activity;
mod rule_scope_grant;
mod rust_target;
#[cfg(unix)]
mod rust_target_cargo;
#[cfg(unix)]
mod rust_target_pipeline;
#[cfg(unix)]
mod rust_target_promotion;
mod rust_target_source;

pub(crate) use exact_path_review::ExactPathApprovalError;
#[cfg(test)]
pub(crate) use exact_path_review::review_exact_paths;
#[cfg(unix)]
pub(crate) use exact_path_review::review_rust_target_plan_facts;
pub(crate) use exact_path_review::{
    ApprovedCleanupSession, ApprovedTrustedReviewedCleanupPlan, CleanupSessionStartError,
    ExactPathHandoffError, ExactPathPlanError, TrustedReviewedCleanupPlan,
};
#[cfg(all(test, target_os = "macos"))]
pub(crate) use exact_path_review::{RustTargetJournalRequest, begin_rust_target_cleanup_session};
#[cfg(test)]
pub(crate) use rule_scope_grant::authorize_rule_target;
pub(crate) use rust_target::{RustTargetEffectWitness, RustTargetLiveWitness};
#[cfg(test)]
pub(crate) use rust_target::{RustTargetLiveValidationError, validate_rust_target_effect};
#[cfg(unix)]
pub(crate) use rust_target_pipeline::{
    RustTargetPipelineError, RustTargetPlanReviewFailure, prepare_rust_target_live_input,
};
#[cfg(target_os = "macos")]
pub(crate) use rust_target_pipeline::{
    prepare_rust_target_plan_facts, prepare_rust_target_promotion,
};
#[cfg(unix)]
pub(crate) use rust_target_promotion::{RustTargetPlanFacts, RustTargetPromotion};

#[cfg(target_os = "macos")]
pub(crate) use rust_target_cargo::{
    CargoMetadataValidationError, DirectCargoEnrollmentCommitError, DirectCargoEnrollmentPreview,
    commit_direct_cargo_enrollment, inspect_direct_cargo_enrollment,
};

#[cfg(all(test, unix))]
mod rust_target_cargo_tests;
#[cfg(all(test, unix))]
mod rust_target_source_tests;
#[cfg(test)]
pub(crate) mod rust_target_tests;
