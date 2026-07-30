/// Current DUX SQLite schema understood by this binary.
pub const DATABASE_SCHEMA_VERSION: u32 = 17;

/// Write compatibility of the database attached to one engine session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseAccess {
    ReadWriteCurrent,
    ReadOnlyNewer { found: u32, supported: u32 },
}

/// Path-free database compatibility status safe for client presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DatabaseStatus {
    pub schema_version: u32,
    pub access: DatabaseAccess,
}

/// Stable category for an engine database-open failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseOpenErrorKind {
    StorageRootUnavailable,
    UnsafeStorageRoot,
    UnsafeStorageObject,
    OwnershipMismatch,
    UnsafePermissions,
    Busy,
    /// SQLite compatibility work exceeded its fixed operation or time budget.
    InspectionLimitExceeded,
    DatabaseUnavailable,
    UnrecognizedDatabase,
    CorruptDatabase,
    MigrationFailed,
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("database open failed: {kind:?}")]
pub(crate) struct DatabaseOpenError {
    pub(crate) kind: DatabaseOpenErrorKind,
}

impl DatabaseOpenError {
    pub(crate) const fn new(kind: DatabaseOpenErrorKind) -> Self {
        Self { kind }
    }
}
