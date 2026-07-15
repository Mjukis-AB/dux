use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::config::DbConfig;
use rusqlite::limits::Limit;
use rusqlite::{Connection, OpenFlags};

use super::migrations::{
    SchemaState, apply_pending_migrations, inspect_schema, inspect_schema_for_status,
};
use super::status::{
    DATABASE_SCHEMA_VERSION, DatabaseAccess, DatabaseOpenError, DatabaseOpenErrorKind,
    DatabaseStatus,
};
use super::storage::{SecureStorePaths, StoreIdentity};

const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

static COORDINATORS: OnceLock<Mutex<HashMap<StoreIdentity, Weak<StoreCoordinator>>>> =
    OnceLock::new();

/// One serialized SQLite owner per physical store and process.
pub(crate) struct StoreCoordinator {
    status: Mutex<DatabaseStatus>,
    paths: SecureStorePaths,
    connection: Mutex<Connection>,
}

impl StoreCoordinator {
    pub(crate) fn open(database_path: &Path) -> Result<Arc<Self>, DatabaseOpenError> {
        super::migrations::validate_compiled_migrations()?;
        let paths = SecureStorePaths::prepare(database_path)?;
        let key = paths.identity();
        let registry = COORDINATORS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut coordinators = registry
            .lock()
            .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?;
        let existing = coordinators.get(&key).and_then(Weak::upgrade);
        if let Some(existing) = existing {
            drop(coordinators);
            existing.refresh_compatibility()?;
            return Ok(existing);
        }
        coordinators.retain(|_, coordinator| coordinator.strong_count() > 0);

        let sqlite_path = paths.sqlite_path()?;
        let coordinator = Arc::new(Self::open_unregistered(paths, &sqlite_path)?);
        coordinators.insert(key, Arc::downgrade(&coordinator));
        Ok(coordinator)
    }

    fn open_unregistered(
        paths: SecureStorePaths,
        sqlite_path: &Path,
    ) -> Result<Self, DatabaseOpenError> {
        Self::open_unregistered_with_hook(paths, sqlite_path, || Ok(()))
    }

    fn open_unregistered_with_hook(
        paths: SecureStorePaths,
        sqlite_path: &Path,
        between_probe_and_lock: impl FnOnce() -> Result<(), DatabaseOpenError>,
    ) -> Result<Self, DatabaseOpenError> {
        paths.validate_all_existing()?;
        between_probe_and_lock()?;
        let _writer_lock = paths.acquire_writer_lock(MIGRATION_LOCK_TIMEOUT)?;
        paths.repair_sqlite_sidecars()?;
        paths.validate_all_existing()?;

        // A marker-owned database with a rollback journal or WAL must be
        // opened RW so SQLite can recover or recreate shared-memory state
        // before compatibility inspection. Without a recovery artifact, the
        // initial classification remains a strict RO open.
        let needs_recovery = paths.requires_initialization() || paths.recovery_artifact_exists()?;
        let mut read_only = None;
        let mut read_write = None;
        let schema = if needs_recovery {
            let connection = open_connection(sqlite_path, false)?;
            configure_connection(&connection, false)?;
            let schema = inspect_schema(&connection);
            paths.repair_sqlite_sidecars()?;
            read_write = Some(connection);
            schema?
        } else {
            let connection = open_connection(sqlite_path, true)?;
            configure_connection(&connection, true)?;
            let schema = inspect_schema(&connection)?;
            read_only = Some(connection);
            schema
        };

        if let SchemaState::Newer { found } = schema {
            drop(read_write);
            let connection = if let Some(connection) = read_only {
                connection
            } else {
                let connection = open_connection(sqlite_path, true)?;
                configure_connection(&connection, true)?;
                connection
            };
            if inspect_schema(&connection)? != (SchemaState::Newer { found }) {
                return Err(DatabaseOpenError::new(
                    DatabaseOpenErrorKind::CorruptDatabase,
                ));
            }
            paths.validate_all_existing()?;
            return Ok(Self {
                status: Mutex::new(DatabaseStatus {
                    schema_version: found,
                    access: DatabaseAccess::ReadOnlyNewer {
                        found,
                        supported: DATABASE_SCHEMA_VERSION,
                    },
                }),
                paths,
                connection: Mutex::new(connection),
            });
        }

        drop(read_only);
        let mut connection = if let Some(connection) = read_write {
            connection
        } else {
            let connection = open_connection(sqlite_path, false)?;
            configure_connection(&connection, false)?;
            connection
        };
        apply_pending_migrations(&mut connection, unix_time_ms()?)?;
        configure_write_ahead_log(&connection)?;
        paths.repair_sqlite_sidecars()?;
        paths.validate_all_existing()?;
        paths.mark_initialized()?;
        Ok(Self {
            status: Mutex::new(DatabaseStatus {
                schema_version: DATABASE_SCHEMA_VERSION,
                access: DatabaseAccess::ReadWriteCurrent,
            }),
            paths,
            connection: Mutex::new(connection),
        })
    }

    fn refresh_compatibility(&self) -> Result<(), DatabaseOpenError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))?;
        let _writer_lock = self.paths.acquire_writer_lock(MIGRATION_LOCK_TIMEOUT)?;
        self.paths.repair_sqlite_sidecars()?;
        self.paths.validate_all_existing()?;
        // A live coordinator already owns a recovery-capable RW connection
        // when its schema is current. Its healthy WAL is ordinary connection
        // state, not evidence that status should churn the connection or run
        // startup integrity inspection. A newer coordinator is already bound
        // to a validated RO connection and must never be reopened RW.
        let refreshed_schema = inspect_schema_for_status(&connection);
        self.paths.repair_sqlite_sidecars()?;
        match refreshed_schema? {
            SchemaState::Current => {
                if matches!(
                    self.cached_status().access,
                    DatabaseAccess::ReadOnlyNewer { .. }
                ) {
                    return Err(DatabaseOpenError::new(
                        DatabaseOpenErrorKind::CorruptDatabase,
                    ));
                }
                *self
                    .status
                    .lock()
                    .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))? =
                    DatabaseStatus {
                        schema_version: DATABASE_SCHEMA_VERSION,
                        access: DatabaseAccess::ReadWriteCurrent,
                    };
            }
            SchemaState::Newer { found } => {
                let sqlite_path = self.paths.sqlite_path()?;
                let read_only = open_connection(&sqlite_path, true)?;
                configure_connection(&read_only, true)?;
                if inspect_schema_for_status(&read_only)? != (SchemaState::Newer { found }) {
                    return Err(DatabaseOpenError::new(
                        DatabaseOpenErrorKind::CorruptDatabase,
                    ));
                }
                self.paths.validate_all_existing()?;
                *connection = read_only;
                *self
                    .status
                    .lock()
                    .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::InternalState))? =
                    DatabaseStatus {
                        schema_version: found,
                        access: DatabaseAccess::ReadOnlyNewer {
                            found,
                            supported: DATABASE_SCHEMA_VERSION,
                        },
                    };
            }
            SchemaState::Empty | SchemaState::Older { .. } => {
                return Err(DatabaseOpenError::new(
                    DatabaseOpenErrorKind::CorruptDatabase,
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn status(&self) -> Result<DatabaseStatus, DatabaseOpenError> {
        self.refresh_compatibility()?;
        Ok(self.cached_status())
    }

    fn cached_status(&self) -> DatabaseStatus {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(super) fn open_unregistered_for_test(
        paths: SecureStorePaths,
        sqlite_path: &Path,
        between_probe_and_lock: impl FnOnce() -> Result<(), DatabaseOpenError>,
    ) -> Result<Self, DatabaseOpenError> {
        Self::open_unregistered_with_hook(paths, sqlite_path, between_probe_and_lock)
    }

    #[cfg(test)]
    pub(super) fn with_connection<T>(&self, inspect: impl FnOnce(&Connection) -> T) -> T {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inspect(&connection)
    }
}

fn open_connection(path: &Path, read_only: bool) -> Result<Connection, DatabaseOpenError> {
    let access = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    Connection::open_with_flags(
        path,
        access
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(map_open_error)
}

fn configure_connection(connection: &Connection, read_only: bool) -> Result<(), DatabaseOpenError> {
    connection
        .busy_timeout(DATABASE_BUSY_TIMEOUT)
        .map_err(map_configuration_error)?;

    for (limit, value) in [
        (Limit::SQLITE_LIMIT_LENGTH, 32 * 1024 * 1024),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, 1024 * 1024),
        (Limit::SQLITE_LIMIT_COLUMN, 128),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 256),
        (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 16),
        (Limit::SQLITE_LIMIT_FUNCTION_ARG, 64),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
        (Limit::SQLITE_LIMIT_LIKE_PATTERN_LENGTH, 4096),
        (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 1024),
        (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 0),
        (Limit::SQLITE_LIMIT_WORKER_THREADS, 2),
    ] {
        connection
            .set_limit(limit, value)
            .map_err(map_configuration_error)?;
    }

    for (config, enabled) in [
        (DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false),
        (DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true),
        (DbConfig::SQLITE_DBCONFIG_WRITABLE_SCHEMA, false),
        (DbConfig::SQLITE_DBCONFIG_DQS_DML, false),
        (DbConfig::SQLITE_DBCONFIG_DQS_DDL, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false),
        (DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_COMMENTS, false),
    ] {
        let actual = connection
            .set_db_config(config, enabled)
            .map_err(map_configuration_error)?;
        if actual != enabled {
            return Err(DatabaseOpenError::new(
                DatabaseOpenErrorKind::DatabaseUnavailable,
            ));
        }
    }

    connection
        .pragma_update(None, "foreign_keys", true)
        .and_then(|()| connection.pragma_update(None, "query_only", read_only))
        .and_then(|()| connection.pragma_update(None, "cell_size_check", true))
        .and_then(|()| connection.pragma_update(None, "mmap_size", 0_i64))
        .and_then(|()| connection.pragma_update(None, "temp_store", "MEMORY"))
        .and_then(|()| connection.pragma_update(None, "locking_mode", "NORMAL"))
        .map_err(map_configuration_error)?;
    if !read_only {
        configure_full_synchronous(connection)?;
    }
    Ok(())
}

fn configure_write_ahead_log(connection: &Connection) -> Result<(), DatabaseOpenError> {
    let mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(map_configuration_error)?;
    if !mode.eq_ignore_ascii_case("wal") {
        let changed: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(map_configuration_error)?;
        if !changed.eq_ignore_ascii_case("wal") {
            return Err(DatabaseOpenError::new(
                DatabaseOpenErrorKind::DatabaseUnavailable,
            ));
        }
    }
    configure_full_synchronous(connection)
}

fn configure_full_synchronous(connection: &Connection) -> Result<(), DatabaseOpenError> {
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(map_configuration_error)?;
    let synchronous: i64 = connection
        .pragma_query_value(None, "synchronous", |row| row.get(0))
        .map_err(map_configuration_error)?;
    if synchronous != 2 {
        return Err(DatabaseOpenError::new(
            DatabaseOpenErrorKind::DatabaseUnavailable,
        ));
    }
    Ok(())
}

fn unix_time_ms() -> Result<i64, DatabaseOpenError> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::MigrationFailed))?
        .as_millis();
    i64::try_from(milliseconds)
        .map_err(|_| DatabaseOpenError::new(DatabaseOpenErrorKind::MigrationFailed))
}

fn map_open_error(error: rusqlite::Error) -> DatabaseOpenError {
    use rusqlite::ErrorCode;
    let kind = match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => DatabaseOpenErrorKind::Busy,
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => {
            DatabaseOpenErrorKind::CorruptDatabase
        }
        _ => DatabaseOpenErrorKind::DatabaseUnavailable,
    };
    DatabaseOpenError::new(kind)
}

fn map_configuration_error(error: rusqlite::Error) -> DatabaseOpenError {
    use rusqlite::ErrorCode;
    let kind = match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => DatabaseOpenErrorKind::Busy,
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => {
            DatabaseOpenErrorKind::CorruptDatabase
        }
        _ => DatabaseOpenErrorKind::DatabaseUnavailable,
    };
    DatabaseOpenError::new(kind)
}
