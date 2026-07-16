//! Private, versioned SQLite storage owned by the shared engine.
//!
//! Raw SQL and connections remain private. Typed history values are
//! presentation observations only and never cleanup authority.

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "typed candidate persistence is integrated by the later evaluator task slice"
    )
)]
mod candidate_history;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "typed capacity persistence integrates with the volume monitor in a later slice"
    )
)]
mod capacity_history;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "typed cleanup history is integrated by the later planner/executor slices"
    )
)]
mod cleanup_history;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "cleanup journal state integrates with the engine executor in a later slice"
    )
)]
mod cleanup_journal;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the frozen v1 byte codec is consumed by the next typed persistence slice"
    )
)]
mod codec;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "typed scan persistence is integrated by the later scan task slice"
    )
)]
mod history;
mod migrations;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "process-instance liveness is consumed by the next fenced journal-transition slice"
    )
)]
mod process_liveness;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "typed scan coverage integrates with the scan task in a later slice"
    )
)]
mod scan_coverage_history;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        unused_imports,
        reason = "immutable snapshots integrate with scan tasks in this milestone"
    )
)]
pub(crate) mod snapshot;
mod status;
mod storage;
mod store;

pub use snapshot::SnapshotOpenErrorKind;
pub use status::{DATABASE_SCHEMA_VERSION, DatabaseAccess, DatabaseOpenErrorKind, DatabaseStatus};
pub(crate) use store::StoreCoordinator;

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
