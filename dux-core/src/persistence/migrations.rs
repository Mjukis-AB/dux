use std::borrow::Cow;
use std::time::{Duration, Instant};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, Transaction, params};
use sha2::{Digest, Sha256};

use super::status::{DATABASE_SCHEMA_VERSION, DatabaseOpenError, DatabaseOpenErrorKind};

pub(crate) const DUX_APPLICATION_ID: u32 = 0x4455_5831;
const MAX_LEDGER_ROWS: i64 = 128;
const MAX_SCHEMA_TYPE_BYTES: i64 = 16;
const MAX_SCHEMA_NAME_BYTES: i64 = 128;
const MAX_SCHEMA_SQL_BYTES: i64 = 64 * 1024;
const MAX_SCHEMA_FINGERPRINT_BYTES: usize = 512 * 1024;
const PROGRESS_OP_INTERVAL: i32 = 1_000;
const STARTUP_INSPECTION_BUDGET: InspectionBudget = InspectionBudget {
    max_callbacks: 250_000,
    max_elapsed: Duration::from_secs(10),
};
const STATUS_INSPECTION_BUDGET: InspectionBudget = InspectionBudget {
    max_callbacks: 1_000,
    max_elapsed: Duration::from_millis(250),
};
const MIGRATION_BUDGET: InspectionBudget = InspectionBudget {
    max_callbacks: 500_000,
    max_elapsed: Duration::from_secs(30),
};

#[derive(Clone, Copy)]
struct InspectionBudget {
    max_callbacks: u64,
    max_elapsed: Duration,
}

#[derive(Clone, Copy)]
struct InspectionClock {
    started_at: Instant,
    max_elapsed: Duration,
}

impl InspectionClock {
    fn checkpoint(self) -> Result<(), DatabaseOpenError> {
        if self.started_at.elapsed() >= self.max_elapsed {
            return Err(DatabaseOpenError::new(
                DatabaseOpenErrorKind::InspectionLimitExceeded,
            ));
        }
        Ok(())
    }
}

struct ProgressHandlerGuard<'connection> {
    connection: &'connection Connection,
    installed: bool,
}

impl ProgressHandlerGuard<'_> {
    fn remove(&mut self) -> rusqlite::Result<()> {
        self.connection.progress_handler(0, None::<fn() -> bool>)?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for ProgressHandlerGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self.connection.progress_handler(0, None::<fn() -> bool>);
        }
    }
}

struct MigrationAuthorizerGuard<'connection> {
    connection: &'connection Connection,
    installed: bool,
}

impl MigrationAuthorizerGuard<'_> {
    fn install(connection: &Connection) -> Result<MigrationAuthorizerGuard<'_>, DatabaseOpenError> {
        connection
            .authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Transaction { .. } | AuthAction::Savepoint { .. } => {
                    Authorization::Deny
                }
                _ => Authorization::Allow,
            }))
            .map_err(map_migration_error)?;
        Ok(MigrationAuthorizerGuard {
            connection,
            installed: true,
        })
    }

    fn remove(&mut self) -> Result<(), DatabaseOpenError> {
        self.connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(map_migration_error)?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for MigrationAuthorizerGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self
                .connection
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}

#[derive(Clone, Copy)]
enum InspectionDepth {
    FullIntegrity,
    Compatibility,
}

#[derive(Clone, Copy)]
pub(crate) struct Migration {
    pub(crate) version: u32,
    pub(crate) name: &'static str,
    pub(crate) checksum_sha256: [u8; 32],
    pub(crate) sql: &'static str,
}

const MIGRATIONS: [Migration; 1] = [Migration {
    version: 1,
    name: "initial-storage-schema",
    checksum_sha256: [
        0xb8, 0x0e, 0x49, 0x87, 0x60, 0x77, 0xda, 0x7f, 0xac, 0x81, 0x27, 0xb0, 0x09, 0xb4, 0xe7,
        0x2c, 0xb2, 0xd7, 0x77, 0x2c, 0x02, 0xfb, 0x3d, 0xb0, 0xe9, 0x93, 0xaa, 0x1a, 0x63, 0xfb,
        0x70, 0x27,
    ],
    sql: include_str!("../../migrations/0001_initial.sql"),
}];

const EXPECTED_SCHEMA_OBJECTS: [(&str, &str); 24] = [
    ("index", "ai_insights_by_expiration"),
    ("index", "ai_insights_by_identity"),
    ("index", "candidates_by_scan_status"),
    ("index", "cleanup_items_by_session"),
    ("index", "cleanup_sessions_by_time"),
    ("index", "disk_samples_by_kind_time"),
    ("index", "disk_samples_by_volume_kind_time"),
    ("index", "rule_outcomes_by_rule_time"),
    ("index", "scan_issues_by_scan_kind"),
    ("index", "scans_by_volume_time"),
    ("index", "schedules_by_next_run"),
    ("table", "ai_insights"),
    ("table", "candidates"),
    ("table", "cleanup_items"),
    ("table", "cleanup_sessions"),
    ("table", "disk_samples"),
    ("table", "rule_outcomes"),
    ("table", "scan_aggregates"),
    ("table", "scan_issues"),
    ("table", "scans"),
    ("table", "schedules"),
    ("table", "schema_migrations"),
    ("table", "settings"),
    ("table", "volumes"),
];

// Canonical sqlite_schema representation produced by v1. A mismatch rejects
// supported databases rather than guessing about drift.
const V1_SCHEMA_FINGERPRINT: [u8; 32] = [
    0xd3, 0x24, 0xcb, 0x24, 0x32, 0xa3, 0xaa, 0x35, 0xc6, 0x82, 0x01, 0x7d, 0x4f, 0x87, 0xac, 0x5e,
    0xae, 0xc1, 0xb1, 0xef, 0x2b, 0xe4, 0x2f, 0x0a, 0x3a, 0xfd, 0x58, 0xf8, 0x94, 0x08, 0x6b, 0x12,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SchemaState {
    Empty,
    Older { found: u32 },
    Current,
    Newer { found: u32 },
}

pub(crate) fn validate_compiled_migrations() -> Result<(), DatabaseOpenError> {
    if MIGRATIONS.is_empty()
        || MIGRATIONS.len() > MAX_LEDGER_ROWS as usize
        || MIGRATIONS.last().map(|migration| migration.version) != Some(DATABASE_SCHEMA_VERSION)
    {
        return Err(migration_error());
    }
    for (index, migration) in MIGRATIONS.iter().enumerate() {
        let Some(compiled_checksum) = canonical_sha256(migration.sql) else {
            return Err(migration_error());
        };
        if migration.version != (index + 1) as u32
            || migration.name.is_empty()
            || migration.name.len() > 128
            || migration.name.chars().any(char::is_control)
            || MIGRATIONS[..index]
                .iter()
                .any(|previous| previous.name == migration.name)
            || migration.sql.as_bytes().contains(&0)
            || compiled_checksum != migration.checksum_sha256
        {
            return Err(migration_error());
        }
    }
    Ok(())
}

pub(crate) fn inspect_schema(connection: &Connection) -> Result<SchemaState, DatabaseOpenError> {
    inspect_schema_with_budget(
        connection,
        InspectionDepth::FullIntegrity,
        STARTUP_INSPECTION_BUDGET,
    )
}

/// Revalidates compatibility facts used for presentation without rescanning
/// every database page or every foreign-key row on each status render.
pub(crate) fn inspect_schema_for_status(
    connection: &Connection,
) -> Result<SchemaState, DatabaseOpenError> {
    inspect_schema_with_budget(
        connection,
        InspectionDepth::Compatibility,
        STATUS_INSPECTION_BUDGET,
    )
}

fn inspect_schema_with_budget(
    connection: &Connection,
    depth: InspectionDepth,
    budget: InspectionBudget,
) -> Result<SchemaState, DatabaseOpenError> {
    run_with_budget(connection, budget, |clock| {
        inspect_schema_inner(connection, depth, clock)
    })
}

fn inspect_schema_inner(
    connection: &Connection,
    depth: InspectionDepth,
    clock: InspectionClock,
) -> Result<SchemaState, DatabaseOpenError> {
    clock.checkpoint()?;
    if matches!(depth, InspectionDepth::FullIntegrity) {
        let quick_check: String = connection
            .pragma_query_value(None, "quick_check", |row| row.get(0))
            .map_err(map_inspection_error)?;
        if quick_check != "ok" {
            return Err(corrupt_error());
        }
    }

    let application_id = pragma_u32(connection, "application_id")?;
    let user_version = pragma_u32(connection, "user_version")?;
    let object_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'",
            [],
            |row| row.get(0),
        )
        .map_err(map_inspection_error)?;
    if !(0..=256).contains(&object_count) {
        return Err(corrupt_error());
    }

    if application_id == 0 && user_version == 0 && object_count == 0 {
        return Ok(SchemaState::Empty);
    }
    if application_id != DUX_APPLICATION_ID {
        return Err(DatabaseOpenError::new(
            DatabaseOpenErrorKind::UnrecognizedDatabase,
        ));
    }
    if user_version == 0 || object_count == 0 {
        return Err(corrupt_error());
    }

    let ledger_type: Option<String> = connection
        .query_row(
            "SELECT type FROM sqlite_schema WHERE name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_inspection_error)?;
    if ledger_type.as_deref() != Some("table") {
        return Err(corrupt_error());
    }

    let ledger_count: i64 = connection
        .query_row("SELECT count(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .map_err(map_inspection_error)?;
    if ledger_count <= 0
        || ledger_count > MAX_LEDGER_ROWS
        || u32::try_from(ledger_count).ok() != Some(user_version)
    {
        return Err(corrupt_error());
    }

    let mut statement = connection
        .prepare(
            "SELECT version, typeof(name), length(CAST(name AS BLOB)), name, \
                    typeof(checksum_sha256), length(checksum_sha256), checksum_sha256, \
                    applied_at_unix_ms \
             FROM schema_migrations ORDER BY version",
        )
        .map_err(map_inspection_error)?;
    let mut rows = statement.query([]).map_err(map_inspection_error)?;
    let mut expected_version = 1_u32;
    while let Some(row) = rows.next().map_err(map_inspection_error)? {
        clock.checkpoint()?;
        let version_i64: i64 = row.get(0).map_err(map_inspection_error)?;
        let version = u32::try_from(version_i64).map_err(|_| corrupt_error())?;
        let name_type: String = row.get(1).map_err(map_inspection_error)?;
        let name_length: i64 = row.get(2).map_err(map_inspection_error)?;
        let checksum_type: String = row.get(4).map_err(map_inspection_error)?;
        let checksum_length: i64 = row.get(5).map_err(map_inspection_error)?;
        let applied_at: i64 = row.get(7).map_err(map_inspection_error)?;
        if version != expected_version
            || name_type != "text"
            || !(1..=128).contains(&name_length)
            || checksum_type != "blob"
            || checksum_length != 32
            || applied_at < 0
        {
            return Err(corrupt_error());
        }
        let name: String = row.get(3).map_err(map_inspection_error)?;
        let checksum: Vec<u8> = row.get(6).map_err(map_inspection_error)?;
        if name.len() != name_length as usize
            || name.chars().any(char::is_control)
            || checksum.len() != 32
        {
            return Err(corrupt_error());
        }
        if let Some(compiled) = MIGRATIONS.get((version - 1) as usize)
            && (name != compiled.name || checksum.as_slice() != compiled.checksum_sha256)
        {
            return Err(corrupt_error());
        }
        expected_version = expected_version.checked_add(1).ok_or_else(corrupt_error)?;
    }
    if expected_version.checked_sub(1) != Some(user_version) {
        return Err(corrupt_error());
    }

    if user_version > DATABASE_SCHEMA_VERSION {
        return Ok(SchemaState::Newer {
            found: user_version,
        });
    }
    if matches!(depth, InspectionDepth::FullIntegrity) {
        validate_foreign_keys(connection)?;
    }
    validate_supported_schema(connection, user_version, clock)?;
    if user_version < DATABASE_SCHEMA_VERSION {
        return Ok(SchemaState::Older {
            found: user_version,
        });
    }
    Ok(SchemaState::Current)
}

pub(crate) fn apply_pending_migrations(
    connection: &mut Connection,
    applied_at_unix_ms: i64,
) -> Result<(), DatabaseOpenError> {
    validate_compiled_migrations()?;
    run_with_budget(connection, MIGRATION_BUDGET, |clock| {
        // The caller owns `&mut Connection`, so the runtime nested-transaction
        // check here retains the same exclusivity while letting the progress
        // budget cover BEGIN, migration statements, validation, and COMMIT.
        let transaction =
            Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
                .map_err(map_migration_error)?;
        match inspect_schema_inner(&transaction, InspectionDepth::FullIntegrity, clock)? {
            SchemaState::Empty => apply_chain(&transaction, &MIGRATIONS, 0, applied_at_unix_ms)?,
            SchemaState::Older { found } => {
                apply_chain(&transaction, &MIGRATIONS, found, applied_at_unix_ms)?;
            }
            SchemaState::Current => {}
            SchemaState::Newer { .. } => return Err(corrupt_error()),
        }
        if inspect_schema_inner(&transaction, InspectionDepth::FullIntegrity, clock)?
            != SchemaState::Current
        {
            return Err(corrupt_error());
        }
        transaction.commit().map_err(map_migration_error)
    })
}

fn apply_chain(
    transaction: &Transaction<'_>,
    migrations: &[Migration],
    applied_version: u32,
    applied_at_unix_ms: i64,
) -> Result<(), DatabaseOpenError> {
    for migration in migrations
        .iter()
        .filter(|migration| migration.version > applied_version)
    {
        execute_migration_sql(transaction, migration.sql)?;
        transaction
            .execute(
                "INSERT INTO schema_migrations \
                 (version, name, checksum_sha256, applied_at_unix_ms) VALUES (?1, ?2, ?3, ?4)",
                params![
                    i64::from(migration.version),
                    migration.name,
                    migration.checksum_sha256.as_slice(),
                    applied_at_unix_ms,
                ],
            )
            .map_err(map_migration_error)?;
        transaction
            .pragma_update(None, "application_id", DUX_APPLICATION_ID)
            .map_err(map_migration_error)?;
        transaction
            .pragma_update(None, "user_version", migration.version)
            .map_err(map_migration_error)?;
    }
    Ok(())
}

fn execute_migration_sql(
    transaction: &Transaction<'_>,
    sql: &str,
) -> Result<(), DatabaseOpenError> {
    // The outer Rust transaction owns atomicity and ledger ordering. Embedded
    // transaction or savepoint control could commit schema changes before the
    // ledger insert, so migration batches never receive that authority.
    let mut authorizer = MigrationAuthorizerGuard::install(transaction)?;
    let result = transaction.execute_batch(sql).map_err(map_migration_error);
    authorizer.remove()?;
    result
}

fn validate_supported_schema(
    connection: &Connection,
    version: u32,
    clock: InspectionClock,
) -> Result<(), DatabaseOpenError> {
    match version {
        1 => validate_v1_schema(connection, clock),
        _ => Err(corrupt_error()),
    }
}

fn validate_v1_schema(
    connection: &Connection,
    clock: InspectionClock,
) -> Result<(), DatabaseOpenError> {
    let mut statement = connection
        .prepare(
            "SELECT typeof(type), length(CAST(type AS BLOB)), type, \
                    typeof(name), length(CAST(name AS BLOB)), name \
             FROM sqlite_schema \
             WHERE name NOT GLOB 'sqlite_*' ORDER BY type, name",
        )
        .map_err(map_inspection_error)?;
    let mut rows = statement.query([]).map_err(map_inspection_error)?;
    let mut actual = Vec::with_capacity(EXPECTED_SCHEMA_OBJECTS.len());
    while let Some(row) = rows.next().map_err(map_inspection_error)? {
        clock.checkpoint()?;
        let type_storage: String = row.get(0).map_err(map_inspection_error)?;
        let type_length: i64 = row.get(1).map_err(map_inspection_error)?;
        let name_storage: String = row.get(3).map_err(map_inspection_error)?;
        let name_length: i64 = row.get(4).map_err(map_inspection_error)?;
        if type_storage != "text"
            || !(1..=MAX_SCHEMA_TYPE_BYTES).contains(&type_length)
            || name_storage != "text"
            || !(1..=MAX_SCHEMA_NAME_BYTES).contains(&name_length)
            || actual.len() >= EXPECTED_SCHEMA_OBJECTS.len()
        {
            return Err(corrupt_error());
        }
        let object_type: String = row.get(2).map_err(map_inspection_error)?;
        let name: String = row.get(5).map_err(map_inspection_error)?;
        if object_type.len() != type_length as usize || name.len() != name_length as usize {
            return Err(corrupt_error());
        }
        actual.push((object_type, name));
    }
    if actual.len() != EXPECTED_SCHEMA_OBJECTS.len()
        || actual.iter().zip(EXPECTED_SCHEMA_OBJECTS).any(
            |((actual_type, actual_name), (expected_type, expected_name))| {
                actual_type != expected_type || actual_name != expected_name
            },
        )
    {
        return Err(corrupt_error());
    }

    let fingerprint = schema_fingerprint_with_clock(connection, clock)?;
    if fingerprint != V1_SCHEMA_FINGERPRINT {
        return Err(corrupt_error());
    }
    Ok(())
}

fn validate_foreign_keys(connection: &Connection) -> Result<(), DatabaseOpenError> {
    let violation: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM pragma_foreign_key_check LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_inspection_error)?;
    if violation.is_some() {
        return Err(corrupt_error());
    }
    Ok(())
}

fn schema_fingerprint_with_clock(
    connection: &Connection,
    clock: InspectionClock,
) -> Result<[u8; 32], DatabaseOpenError> {
    let mut statement = connection
        .prepare(
            "SELECT typeof(type), length(CAST(type AS BLOB)), type, \
                    typeof(name), length(CAST(name AS BLOB)), name, \
                    typeof(sql), length(CAST(sql AS BLOB)), sql \
             FROM sqlite_schema \
             WHERE name NOT GLOB 'sqlite_*' AND sql IS NOT NULL ORDER BY type, name",
        )
        .map_err(map_inspection_error)?;
    let mut rows = statement.query([]).map_err(map_inspection_error)?;
    let mut hasher = Sha256::new();
    let mut count = 0_u32;
    let mut total_bytes = 0_usize;
    while let Some(row) = rows.next().map_err(map_inspection_error)? {
        clock.checkpoint()?;
        let type_storage: String = row.get(0).map_err(map_inspection_error)?;
        let type_length: i64 = row.get(1).map_err(map_inspection_error)?;
        let name_storage: String = row.get(3).map_err(map_inspection_error)?;
        let name_length: i64 = row.get(4).map_err(map_inspection_error)?;
        let sql_storage: String = row.get(6).map_err(map_inspection_error)?;
        let sql_length: i64 = row.get(7).map_err(map_inspection_error)?;
        if type_storage != "text"
            || !(1..=MAX_SCHEMA_TYPE_BYTES).contains(&type_length)
            || name_storage != "text"
            || !(1..=MAX_SCHEMA_NAME_BYTES).contains(&name_length)
            || sql_storage != "text"
            || !(1..=MAX_SCHEMA_SQL_BYTES).contains(&sql_length)
            || count >= 64
        {
            return Err(corrupt_error());
        }
        let row_bytes =
            usize::try_from(type_length + name_length + sql_length).map_err(|_| corrupt_error())?;
        total_bytes = total_bytes
            .checked_add(row_bytes)
            .filter(|total| *total <= MAX_SCHEMA_FINGERPRINT_BYTES)
            .ok_or_else(corrupt_error)?;
        let object_type: String = row.get(2).map_err(map_inspection_error)?;
        let name: String = row.get(5).map_err(map_inspection_error)?;
        let sql: String = row.get(8).map_err(map_inspection_error)?;
        for value in [&object_type, &name, &sql] {
            clock.checkpoint()?;
            let canonical = canonical_lf(value).ok_or_else(corrupt_error)?;
            let length = u32::try_from(canonical.len()).map_err(|_| corrupt_error())?;
            hasher.update(length.to_le_bytes());
            hasher.update(canonical.as_bytes());
        }
        count = count.checked_add(1).ok_or_else(corrupt_error)?;
    }
    Ok(hasher.finalize().into())
}

#[cfg(test)]
pub(crate) fn schema_fingerprint(connection: &Connection) -> Result<[u8; 32], DatabaseOpenError> {
    schema_fingerprint_with_clock(
        connection,
        InspectionClock {
            started_at: Instant::now(),
            max_elapsed: Duration::from_secs(60),
        },
    )
}

fn canonical_sha256(value: &str) -> Option<[u8; 32]> {
    let canonical = canonical_lf(value)?;
    Some(Sha256::digest(canonical.as_bytes()).into())
}

fn canonical_lf(value: &str) -> Option<Cow<'_, str>> {
    if !value.contains('\r') {
        return Some(Cow::Borrowed(value));
    }
    let normalized = value.replace("\r\n", "\n");
    (!normalized.contains('\r')).then_some(Cow::Owned(normalized))
}

fn pragma_u32(connection: &Connection, name: &str) -> Result<u32, DatabaseOpenError> {
    let value: i64 = connection
        .pragma_query_value(None, name, |row| row.get(0))
        .map_err(map_inspection_error)?;
    u32::try_from(value).map_err(|_| corrupt_error())
}

fn run_with_budget<T>(
    connection: &Connection,
    budget: InspectionBudget,
    operation: impl FnOnce(InspectionClock) -> Result<T, DatabaseOpenError>,
) -> Result<T, DatabaseOpenError> {
    debug_assert!(budget.max_callbacks > 0);
    let started_at = Instant::now();
    let clock = InspectionClock {
        started_at,
        max_elapsed: budget.max_elapsed,
    };
    let mut callbacks = 0_u64;
    let mut interrupted = false;
    // SQLite checks the deadline at this VM-instruction cadence. It bounds
    // compute-heavy statements but cannot preempt one blocking filesystem
    // operation between callbacks.
    connection
        .progress_handler(
            PROGRESS_OP_INTERVAL,
            Some(move || {
                if interrupted {
                    return true;
                }
                callbacks = callbacks.saturating_add(1);
                interrupted =
                    callbacks >= budget.max_callbacks || started_at.elapsed() >= budget.max_elapsed;
                interrupted
            }),
        )
        .map_err(map_budget_configuration_error)?;
    let mut guard = ProgressHandlerGuard {
        connection,
        installed: true,
    };

    let result = operation(clock);
    guard.remove().map_err(map_budget_configuration_error)?;
    let value = result?;
    clock.checkpoint()?;
    Ok(value)
}

fn map_inspection_error(error: rusqlite::Error) -> DatabaseOpenError {
    use rusqlite::ErrorCode;
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            DatabaseOpenError::new(DatabaseOpenErrorKind::Busy)
        }
        Some(ErrorCode::OperationInterrupted) => {
            DatabaseOpenError::new(DatabaseOpenErrorKind::InspectionLimitExceeded)
        }
        _ => corrupt_error(),
    }
}

fn map_migration_error(error: rusqlite::Error) -> DatabaseOpenError {
    use rusqlite::ErrorCode;
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            DatabaseOpenError::new(DatabaseOpenErrorKind::Busy)
        }
        Some(ErrorCode::OperationInterrupted) => {
            DatabaseOpenError::new(DatabaseOpenErrorKind::InspectionLimitExceeded)
        }
        _ => migration_error(),
    }
}

fn map_budget_configuration_error(_: rusqlite::Error) -> DatabaseOpenError {
    DatabaseOpenError::new(DatabaseOpenErrorKind::DatabaseUnavailable)
}

fn corrupt_error() -> DatabaseOpenError {
    DatabaseOpenError::new(DatabaseOpenErrorKind::CorruptDatabase)
}

fn migration_error() -> DatabaseOpenError {
    DatabaseOpenError::new(DatabaseOpenErrorKind::MigrationFailed)
}

use rusqlite::OptionalExtension;

#[cfg(test)]
pub(super) fn test_migrations() -> &'static [Migration] {
    &MIGRATIONS
}

#[cfg(test)]
pub(super) const fn test_v1_schema_fingerprint() -> [u8; 32] {
    V1_SCHEMA_FINGERPRINT
}

#[cfg(test)]
pub(super) fn inspect_schema_with_test_budget(
    connection: &Connection,
    max_callbacks: u64,
    max_elapsed: Duration,
) -> Result<SchemaState, DatabaseOpenError> {
    inspect_schema_with_budget(
        connection,
        InspectionDepth::FullIntegrity,
        InspectionBudget {
            max_callbacks,
            max_elapsed,
        },
    )
}

#[cfg(test)]
pub(super) fn panic_with_test_budget(connection: &Connection) {
    let _: Result<(), DatabaseOpenError> = run_with_budget(
        connection,
        InspectionBudget {
            max_callbacks: 1,
            max_elapsed: Duration::ZERO,
        },
        |_| panic!("test panic while a progress handler is installed"),
    );
}

#[cfg(test)]
pub(super) fn apply_test_chain(
    transaction: &Transaction<'_>,
    migrations: &[Migration],
    applied_at_unix_ms: i64,
) -> Result<(), DatabaseOpenError> {
    apply_chain(transaction, migrations, 0, applied_at_unix_ms)
}

#[cfg(test)]
pub(super) fn apply_test_upgrade_chain(
    transaction: &Transaction<'_>,
    migrations: &[Migration],
    applied_version: u32,
    applied_at_unix_ms: i64,
) -> Result<(), DatabaseOpenError> {
    apply_chain(transaction, migrations, applied_version, applied_at_unix_ms)
}
