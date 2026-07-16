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
mod status;
mod storage;
mod store;

pub use status::{DATABASE_SCHEMA_VERSION, DatabaseAccess, DatabaseOpenErrorKind, DatabaseStatus};
pub(crate) use store::StoreCoordinator;

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
