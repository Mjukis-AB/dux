//! Explicit, all-or-nothing cleanup-history clearing.
//!
//! The boundary accepts no session, candidate, path, plan, or effect input.
//! Preparation validates every terminal cleanup journal and fingerprints its
//! complete five-table history graph. Commit recomputes that witness under the
//! same cross-process cleanup exclusion and deletes only unchanged terminal
//! history; active and recovery evidence remains untouched.

use std::time::{Duration, Instant, SystemTime};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Transaction};
use sha2::{Digest, Sha256};

use super::cleanup_history::CleanupSessionId;
use super::cleanup_history_query::{
    StoredCleanupItemStatus, StoredCleanupSessionStatus, cleanup_history_session,
};
use super::history::{HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error};

const VALIDATION_PAGE_LIMIT: usize = 64;
const PROGRESS_OP_INTERVAL: i32 = 1_000;
const MAX_PROGRESS_CALLBACKS: u64 = 500_000;
const MAX_ELAPSED: Duration = Duration::from_secs(10);
const TERMINAL_STATUS_SQL: &str = "'completed', 'partially_completed', 'failed', 'cancelled', \
     'interrupted', 'rejected', 'dry_run'";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedCleanupHistoryClear {
    witness: CleanupHistoryClearWitness,
    session_count: u64,
    oldest_started_at: SystemTime,
    newest_started_at: SystemTime,
}

impl PreparedCleanupHistoryClear {
    pub(crate) const fn session_count(&self) -> u64 {
        self.session_count
    }

    pub(crate) const fn oldest_started_at(&self) -> SystemTime {
        self.oldest_started_at
    }

    pub(crate) const fn newest_started_at(&self) -> SystemTime {
        self.newest_started_at
    }

    pub(super) const fn witness(&self) -> &CleanupHistoryClearWitness {
        &self.witness
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CleanupHistoryClearResult {
    pub(crate) sessions_removed: u64,
    pub(crate) items_removed: u64,
    pub(crate) paths_removed: u64,
    pub(crate) evidence_removed: u64,
    pub(crate) warnings_removed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CleanupHistoryClearStoreError {
    NothingToClear,
    ActiveCleanup,
    ChangedSincePreview,
    History(HistoryError),
}

impl From<HistoryError> for CleanupHistoryClearStoreError {
    fn from(value: HistoryError) -> Self {
        Self::History(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CleanupHistoryClearWitness([u8; 32]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CleanupHistoryClearReconciliation {
    Applied,
    NotApplied,
    Ambiguous,
}

pub(super) fn prepare_cleanup_history_clear(
    connection: &Connection,
) -> Result<PreparedCleanupHistoryClear, CleanupHistoryClearStoreError> {
    let summary = validate_every_terminal_session(connection)?;
    if summary.session_count == 0 {
        return Err(CleanupHistoryClearStoreError::NothingToClear);
    }
    let witness = fingerprint_terminal_history_graph(connection)?;
    Ok(PreparedCleanupHistoryClear {
        witness,
        session_count: summary.session_count,
        oldest_started_at: summary.oldest_started_at.ok_or_else(corrupt)?,
        newest_started_at: summary.newest_started_at.ok_or_else(corrupt)?,
    })
}

pub(super) fn apply_cleanup_history_clear(
    transaction: &Transaction<'_>,
    expected: &CleanupHistoryClearWitness,
) -> Result<CleanupHistoryClearResult, CleanupHistoryClearStoreError> {
    let current = prepare_cleanup_history_clear(transaction)?;
    if current.witness() != expected {
        return Err(CleanupHistoryClearStoreError::ChangedSincePreview);
    }

    let mut authorizer = CleanupHistoryClearAuthorizerGuard::install(transaction)?;
    let result = delete_terminal_history(transaction, current.session_count);
    authorizer.remove()?;
    result
}

pub(super) fn reconcile_cleanup_history_clear(
    connection: &Connection,
    expected: &CleanupHistoryClearWitness,
) -> Result<CleanupHistoryClearReconciliation, HistoryError> {
    let first_page = run_with_progress_budget(connection, || {
        load_terminal_session_id_page(connection, None)
    })
    .map_err(store_error_history)?;
    if first_page.is_empty() {
        return Ok(CleanupHistoryClearReconciliation::Applied);
    }
    match prepare_cleanup_history_clear(connection) {
        Ok(current) if current.witness() == expected => {
            Ok(CleanupHistoryClearReconciliation::NotApplied)
        }
        Ok(_) | Err(CleanupHistoryClearStoreError::NothingToClear) => {
            Ok(CleanupHistoryClearReconciliation::Ambiguous)
        }
        Err(CleanupHistoryClearStoreError::History(error)) => Err(error),
        Err(
            CleanupHistoryClearStoreError::ActiveCleanup
            | CleanupHistoryClearStoreError::ChangedSincePreview,
        ) => Ok(CleanupHistoryClearReconciliation::Ambiguous),
    }
}

#[derive(Clone, Copy)]
struct TerminalHistorySummary {
    session_count: u64,
    oldest_started_at: Option<SystemTime>,
    newest_started_at: Option<SystemTime>,
}

fn refuse_session_claims(
    connection: &Connection,
    session_id: &CleanupSessionId,
) -> Result<(), CleanupHistoryClearStoreError> {
    let count: i64 = connection
        .query_row(
            "SELECT (
                 SELECT COUNT(*) FROM candidate_plan_claims WHERE session_id = ?1
             ) + (
                 SELECT COUNT(*) FROM trusted_rust_target_plan_claims WHERE session_id = ?1
             )",
            [session_id.as_str()],
            |row| row.get(0),
        )
        .map_err(map_query_sql_error)?;
    if count == 0 {
        Ok(())
    } else {
        Err(corrupt().into())
    }
}

fn validate_every_terminal_session(
    connection: &Connection,
) -> Result<TerminalHistorySummary, CleanupHistoryClearStoreError> {
    let mut summary = TerminalHistorySummary {
        session_count: 0,
        oldest_started_at: None,
        newest_started_at: None,
    };
    let mut cursor: Option<String> = None;
    loop {
        let page = run_with_progress_budget(connection, || {
            load_terminal_session_id_page(connection, cursor.as_deref())
        })?;
        if page.is_empty() {
            return Ok(summary);
        }
        for session_id in &page {
            let observation =
                cleanup_history_session(connection, session_id)?.ok_or_else(corrupt)?;
            if matches!(
                observation.summary.status,
                StoredCleanupSessionStatus::Planned
                    | StoredCleanupSessionStatus::Running
                    | StoredCleanupSessionStatus::Recovering
            ) || observation.items.iter().any(|item| {
                matches!(
                    item.status,
                    StoredCleanupItemStatus::Planned
                        | StoredCleanupItemStatus::Validating
                        | StoredCleanupItemStatus::EffectStarted
                        | StoredCleanupItemStatus::OutcomeUnknown
                )
            }) {
                return Err(CleanupHistoryClearStoreError::ActiveCleanup);
            }
            run_with_progress_budget(connection, || refuse_session_claims(connection, session_id))?;
            summary.session_count = summary.session_count.checked_add(1).ok_or_else(corrupt)?;
            summary.oldest_started_at = Some(
                summary
                    .oldest_started_at
                    .map_or(observation.summary.started_at, |oldest| {
                        oldest.min(observation.summary.started_at)
                    }),
            );
            summary.newest_started_at = Some(
                summary
                    .newest_started_at
                    .map_or(observation.summary.started_at, |newest| {
                        newest.max(observation.summary.started_at)
                    }),
            );
        }
        cursor = page.last().map(|id| id.as_str().to_owned());
        if page.len() < VALIDATION_PAGE_LIMIT {
            return Ok(summary);
        }
    }
}

fn load_terminal_session_id_page(
    connection: &Connection,
    cursor: Option<&str>,
) -> Result<Vec<CleanupSessionId>, CleanupHistoryClearStoreError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT session_id FROM cleanup_sessions
             WHERE status IN ({TERMINAL_STATUS_SQL})
               AND (?1 IS NULL OR session_id > ?1)
             ORDER BY session_id ASC LIMIT {VALIDATION_PAGE_LIMIT}"
        ))
        .map_err(map_query_sql_error)?;
    let mut rows = statement.query([cursor]).map_err(map_query_sql_error)?;
    let mut page = Vec::with_capacity(VALIDATION_PAGE_LIMIT);
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        let value: String = row.get(0).map_err(map_query_sql_error)?;
        page.push(CleanupSessionId::new(value).map_err(|_| corrupt())?);
    }
    Ok(page)
}

fn fingerprint_terminal_history_graph(
    connection: &Connection,
) -> Result<CleanupHistoryClearWitness, CleanupHistoryClearStoreError> {
    let mut hasher = Sha256::new();
    hasher.update(b"dux.cleanup-history-clear.terminal.v1");
    let mut cursor: Option<String> = None;
    loop {
        let page = run_with_progress_budget(connection, || {
            load_terminal_session_id_page(connection, cursor.as_deref())
        })?;
        if page.is_empty() {
            break;
        }
        for session_id in &page {
            run_with_progress_budget(connection, || {
                hash_terminal_session_graph(connection, &mut hasher, session_id)
            })?;
        }
        cursor = page.last().map(|id| id.as_str().to_owned());
        if page.len() < VALIDATION_PAGE_LIMIT {
            break;
        }
    }
    Ok(CleanupHistoryClearWitness(hasher.finalize().into()))
}

fn hash_terminal_session_graph(
    connection: &Connection,
    hasher: &mut Sha256,
    session_id: &CleanupSessionId,
) -> Result<(), HistoryError> {
    hash_session_query(
        connection,
        hasher,
        session_id,
        "cleanup_sessions",
        "SELECT * FROM cleanup_sessions WHERE session_id = ?1",
    )?;
    hash_session_query(
        connection,
        hasher,
        session_id,
        "cleanup_items",
        "SELECT * FROM cleanup_items
         WHERE session_id = ?1 ORDER BY item_ordinal",
    )?;
    hash_session_query(
        connection,
        hasher,
        session_id,
        "cleanup_item_paths",
        "SELECT * FROM cleanup_item_paths
         WHERE session_id = ?1 ORDER BY item_ordinal, path_ordinal",
    )?;
    hash_session_query(
        connection,
        hasher,
        session_id,
        "cleanup_item_evidence",
        "SELECT * FROM cleanup_item_evidence
         WHERE session_id = ?1 ORDER BY item_ordinal, evidence_ordinal",
    )?;
    hash_session_query(
        connection,
        hasher,
        session_id,
        "cleanup_plan_warnings",
        "SELECT * FROM cleanup_plan_warnings
         WHERE session_id = ?1 ORDER BY warning_ordinal",
    )
}

fn hash_session_query(
    connection: &Connection,
    hasher: &mut Sha256,
    session_id: &CleanupSessionId,
    table: &str,
    sql: &str,
) -> Result<(), HistoryError> {
    hasher.update((table.len() as u64).to_le_bytes());
    hasher.update(table.as_bytes());
    let mut statement = connection.prepare(sql).map_err(map_query_sql_error)?;
    let column_count = statement.column_count();
    let mut rows = statement
        .query([session_id.as_str()])
        .map_err(map_query_sql_error)?;
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        hasher.update((column_count as u64).to_le_bytes());
        for column in 0..column_count {
            match row.get_ref(column).map_err(map_query_sql_error)? {
                ValueRef::Null => hasher.update([0]),
                ValueRef::Integer(value) => {
                    hasher.update([1]);
                    hasher.update(value.to_le_bytes());
                }
                ValueRef::Real(value) => {
                    hasher.update([2]);
                    hasher.update(value.to_bits().to_le_bytes());
                }
                ValueRef::Text(value) => {
                    hasher.update([3]);
                    hash_bytes(hasher, value)?;
                }
                ValueRef::Blob(value) => {
                    hasher.update([4]);
                    hash_bytes(hasher, value)?;
                }
            }
        }
    }
    Ok(())
}

fn hash_bytes(hasher: &mut Sha256, value: &[u8]) -> Result<(), HistoryError> {
    let length = u64::try_from(value.len()).map_err(|_| corrupt())?;
    hasher.update(length.to_le_bytes());
    hasher.update(value);
    Ok(())
}

fn delete_terminal_history(
    connection: &Connection,
    expected_session_count: u64,
) -> Result<CleanupHistoryClearResult, CleanupHistoryClearStoreError> {
    let mut result = CleanupHistoryClearResult {
        sessions_removed: 0,
        items_removed: 0,
        paths_removed: 0,
        evidence_removed: 0,
        warnings_removed: 0,
    };
    let mut cursor: Option<String> = None;
    loop {
        let page = run_with_progress_budget(connection, || {
            load_terminal_session_id_page(connection, cursor.as_deref())
        })?;
        if page.is_empty() {
            break;
        }
        for session_id in &page {
            run_with_progress_budget(connection, || {
                result.evidence_removed = checked_add_removed(
                    result.evidence_removed,
                    delete_for_session(connection, "cleanup_item_evidence", session_id)?,
                )?;
                result.paths_removed = checked_add_removed(
                    result.paths_removed,
                    delete_for_session(connection, "cleanup_item_paths", session_id)?,
                )?;
                result.warnings_removed = checked_add_removed(
                    result.warnings_removed,
                    delete_for_session(connection, "cleanup_plan_warnings", session_id)?,
                )?;
                result.items_removed = checked_add_removed(
                    result.items_removed,
                    delete_for_session(connection, "cleanup_items", session_id)?,
                )?;
                result.sessions_removed = checked_add_removed(
                    result.sessions_removed,
                    delete_for_session(connection, "cleanup_sessions", session_id)?,
                )?;
                Ok::<_, CleanupHistoryClearStoreError>(())
            })?;
        }
        cursor = page.last().map(|id| id.as_str().to_owned());
        if page.len() < VALIDATION_PAGE_LIMIT {
            break;
        }
    }
    if result.sessions_removed != expected_session_count {
        return Err(corrupt().into());
    }
    Ok(result)
}

fn delete_for_session(
    connection: &Connection,
    table: &'static str,
    session_id: &CleanupSessionId,
) -> Result<u64, CleanupHistoryClearStoreError> {
    let removed = connection
        .execute(
            &format!("DELETE FROM {table} WHERE session_id = ?1"),
            [session_id.as_str()],
        )
        .map_err(map_clear_write_sql_error)?;
    u64::try_from(removed).map_err(|_| corrupt().into())
}

fn checked_add_removed(total: u64, removed: u64) -> Result<u64, CleanupHistoryClearStoreError> {
    total.checked_add(removed).ok_or_else(|| corrupt().into())
}

fn map_clear_write_sql_error(error: rusqlite::Error) -> HistoryError {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) {
        HistoryError::new(HistoryErrorKind::QueryLimitExceeded)
    } else {
        map_write_sql_error(error)
    }
}

fn run_with_progress_budget<T, E>(
    connection: &Connection,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, E>
where
    E: From<HistoryError>,
{
    let started_at = Instant::now();
    let mut callbacks = 0_u64;
    connection
        .progress_handler(
            PROGRESS_OP_INTERVAL,
            Some(move || {
                callbacks = callbacks.saturating_add(1);
                callbacks >= MAX_PROGRESS_CALLBACKS || started_at.elapsed() >= MAX_ELAPSED
            }),
        )
        .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
    let mut guard = CleanupHistoryClearProgressGuard {
        connection,
        installed: true,
    };
    let result = operation();
    guard.remove()?;
    let value = result?;
    if started_at.elapsed() >= MAX_ELAPSED {
        return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded).into());
    }
    Ok(value)
}

struct CleanupHistoryClearProgressGuard<'a> {
    connection: &'a Connection,
    installed: bool,
}

impl CleanupHistoryClearProgressGuard<'_> {
    fn remove(&mut self) -> Result<(), HistoryError> {
        self.connection
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for CleanupHistoryClearProgressGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self.connection.progress_handler(0, None::<fn() -> bool>);
        }
    }
}

struct CleanupHistoryClearAuthorizerGuard<'a> {
    connection: &'a Connection,
    installed: bool,
}

impl CleanupHistoryClearAuthorizerGuard<'_> {
    fn install(
        connection: &Connection,
    ) -> Result<CleanupHistoryClearAuthorizerGuard<'_>, CleanupHistoryClearStoreError> {
        connection
            .authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Read { .. } | AuthAction::Select | AuthAction::Function { .. } => {
                    Authorization::Allow
                }
                AuthAction::Delete {
                    table_name:
                        "cleanup_item_evidence"
                        | "cleanup_item_paths"
                        | "cleanup_plan_warnings"
                        | "cleanup_items"
                        | "cleanup_sessions",
                } => Authorization::Allow,
                _ => Authorization::Deny,
            }))
            .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
        Ok(CleanupHistoryClearAuthorizerGuard {
            connection,
            installed: true,
        })
    }

    fn remove(&mut self) -> Result<(), CleanupHistoryClearStoreError> {
        self.connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for CleanupHistoryClearAuthorizerGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self
                .connection
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}

fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

fn store_error_history(error: CleanupHistoryClearStoreError) -> HistoryError {
    match error {
        CleanupHistoryClearStoreError::History(error) => error,
        CleanupHistoryClearStoreError::NothingToClear
        | CleanupHistoryClearStoreError::ActiveCleanup
        | CleanupHistoryClearStoreError::ChangedSincePreview => corrupt(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::StoreCoordinator;

    #[test]
    fn clear_authorizer_rejects_every_non_history_table() {
        let temp = tempfile::tempdir().unwrap();
        let store = StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap();
        store.with_connection(|connection| {
            let tables = {
                let mut statement = connection
                    .prepare(
                        "SELECT name FROM sqlite_schema
                         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                         ORDER BY name",
                    )
                    .unwrap();
                statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            };
            let allowed = [
                "cleanup_item_evidence",
                "cleanup_item_paths",
                "cleanup_plan_warnings",
                "cleanup_items",
                "cleanup_sessions",
            ];
            let transaction = connection.unchecked_transaction().unwrap();
            let mut guard = CleanupHistoryClearAuthorizerGuard::install(&transaction).unwrap();
            for table in &tables {
                let result = transaction.execute(&format!("DELETE FROM \"{table}\""), []);
                if allowed.contains(&table.as_str()) {
                    assert!(result.is_ok(), "history delete rejected for {table}");
                } else {
                    assert!(
                        result.is_err(),
                        "clear authorizer unexpectedly allowed DELETE from {table}"
                    );
                }
            }
            guard.remove().unwrap();
            drop(guard);
            transaction.rollback().unwrap();
        });
    }

    #[test]
    fn clear_write_interruption_maps_to_the_fixed_budget_error() {
        let error = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_INTERRUPT),
            None,
        );

        assert_eq!(
            map_clear_write_sql_error(error).kind,
            HistoryErrorKind::QueryLimitExceeded
        );
    }
}
