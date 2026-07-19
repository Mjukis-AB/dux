//! Cleanup effect boundaries.

#[allow(
    dead_code,
    reason = "bounded cleanup capacity verification is staged before journal/effect orchestration"
)]
mod capacity;

#[allow(
    dead_code,
    reason = "journal-fenced Trash admission is consumed by the platform adapter slice"
)]
pub(crate) mod executor;

#[allow(
    dead_code,
    reason = "the permanent-safe driver is deliberately not wired to app/FFI"
)]
pub(crate) mod permanent_safe;

#[doc(hidden)]
pub mod legacy_cli;
