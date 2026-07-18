//! Ephemeral, planner-owned live observations.
//!
//! Nothing in this module grants protected-root authority or can be converted
//! into a cleanup plan. The first witness is deliberately staged while the
//! remaining Cargo, process, mount, approval, and executor guards are absent.

mod rust_target;
#[cfg(unix)]
mod rust_target_cargo;

#[cfg(all(test, unix))]
mod rust_target_cargo_tests;
#[cfg(test)]
mod rust_target_tests;
