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
mod exact_path_review;
mod process_activity;
mod rust_target;
#[cfg(unix)]
mod rust_target_cargo;
mod rust_target_source;

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
mod rust_target_tests;
