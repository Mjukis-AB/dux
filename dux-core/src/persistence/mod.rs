//! Private, versioned SQLite storage owned by the shared engine.
//!
//! This checkpoint installs schema and compatibility infrastructure only. It
//! exposes no raw SQL, domain writes, cleanup authority, or history queries.

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the frozen v1 byte codec is consumed by the next typed persistence slice"
    )
)]
mod codec;
mod migrations;
mod status;
mod storage;
mod store;

pub use status::{DATABASE_SCHEMA_VERSION, DatabaseAccess, DatabaseOpenErrorKind, DatabaseStatus};
pub(crate) use store::StoreCoordinator;

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
