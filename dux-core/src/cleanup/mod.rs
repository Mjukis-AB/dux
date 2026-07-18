//! Cleanup effect boundaries.

#[allow(
    dead_code,
    reason = "journal-fenced Trash admission is consumed by the platform adapter slice"
)]
pub(crate) mod executor;

#[doc(hidden)]
pub mod legacy_cli;
