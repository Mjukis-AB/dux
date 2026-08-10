//! Versioned, deny-by-default global automation control.
//!
//! This setting is saved consent only. It grants no schedule, plan, task, or
//! cleanup authority, and no production scheduler consumes it in this slice.

use std::time::SystemTime;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::store::{HistoryConnectionGuard, StoreCoordinator};
use crate::domain::MAX_AUTOMATION_UNIX_MS;

pub(crate) const AUTOMATION_GLOBAL_CONTROL_KEY: &str = "automation_global_control";
const VALUE_SCHEMA_VERSION: i64 = 1;
const MAX_CANONICAL_VALUE_BYTES: usize = 128;
const MAX_REVISION: u64 = i64::MAX as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomationGlobalControlSource {
    Default,
    Stored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AutomationGlobalControl {
    pub(crate) enabled: bool,
    pub(crate) source: AutomationGlobalControlSource,
    pub(crate) revision: u64,
    pub(crate) updated_at: Option<SystemTime>,
}

impl AutomationGlobalControl {
    pub(crate) const DEFAULT: Self = Self {
        enabled: false,
        source: AutomationGlobalControlSource::Default,
        revision: 0,
        updated_at: None,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AutomationGlobalControlUpdate {
    pub(crate) control: AutomationGlobalControl,
    pub(crate) changed: bool,
}

#[derive(Clone, Copy)]
enum Mutation {
    Set(bool),
    Reset,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StoredSource {
    Default,
    Stored,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredValue {
    enabled: bool,
    revision: u64,
    source: StoredSource,
}

#[derive(Debug)]
struct StoredControl {
    control: AutomationGlobalControl,
    canonical_json: String,
    value_schema_version: i64,
    updated_at_unix_ms: i64,
}

#[derive(Debug)]
struct RawControl {
    canonical_json: String,
    value_schema_version: i64,
    updated_at_unix_ms: i64,
}

impl StoreCoordinator {
    pub(crate) fn load_automation_global_control(
        &self,
    ) -> Result<AutomationGlobalControl, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_automation_global_control(&guard.connection)
    }

    pub(crate) fn set_automation_global_enabled(
        &self,
        expected_revision: u64,
        enabled: bool,
    ) -> Result<AutomationGlobalControlUpdate, HistoryError> {
        self.mutate_automation_global_control(
            expected_revision,
            Mutation::Set(enabled),
            SystemTime::now(),
            || Ok(()),
        )
    }

    pub(crate) fn reset_automation_global_control(
        &self,
        expected_revision: u64,
    ) -> Result<AutomationGlobalControlUpdate, HistoryError> {
        self.mutate_automation_global_control(
            expected_revision,
            Mutation::Reset,
            SystemTime::now(),
            || Ok(()),
        )
    }

    fn mutate_automation_global_control(
        &self,
        expected_revision: u64,
        mutation: Mutation,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<AutomationGlobalControlUpdate, HistoryError> {
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        if observed_at_unix_ms > MAX_AUTOMATION_UNIX_MS {
            return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
        }
        let mut guard = self.lock_current_history_connection()?;
        let original = load_stored(&guard.connection)?;
        let current = original
            .as_ref()
            .map_or(AutomationGlobalControl::DEFAULT, |stored| stored.control);
        if current.revision != expected_revision {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let (enabled, source) = match mutation {
            Mutation::Set(enabled)
                if current.source == AutomationGlobalControlSource::Stored
                    && current.enabled == enabled =>
            {
                return Ok(AutomationGlobalControlUpdate {
                    control: current,
                    changed: false,
                });
            }
            Mutation::Set(enabled) => (enabled, AutomationGlobalControlSource::Stored),
            Mutation::Reset if current.source == AutomationGlobalControlSource::Default => {
                return Ok(AutomationGlobalControlUpdate {
                    control: current,
                    changed: false,
                });
            }
            Mutation::Reset => (false, AutomationGlobalControlSource::Default),
        };
        let revision = current
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_REVISION)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
        let updated_at_unix_ms = original.as_ref().map_or(observed_at_unix_ms, |stored| {
            observed_at_unix_ms.max(stored.updated_at_unix_ms)
        });
        let expected = AutomationGlobalControl {
            enabled,
            source,
            revision,
            updated_at: Some(unix_ms_to_system_time(updated_at_unix_ms)?),
        };
        let expected_json = canonical_json(expected, HistoryErrorKind::InvalidInput)?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let transactional_original = load_stored(&transaction)?;
        if !optional_exact(transactional_original.as_ref(), original.as_ref()) {
            return Err(HistoryError::new(HistoryErrorKind::InternalState));
        }
        write_row(
            &transaction,
            original.as_ref(),
            &expected_json,
            updated_at_unix_ms,
        )?;
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => {
                return Ok(AutomationGlobalControlUpdate {
                    control: expected,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        reconcile(self, &guard, original.as_ref(), expected, failure)
    }

    #[cfg(test)]
    pub(super) fn set_automation_global_enabled_at_for_test(
        &self,
        expected_revision: u64,
        enabled: bool,
        observed_at: SystemTime,
    ) -> Result<AutomationGlobalControlUpdate, HistoryError> {
        self.mutate_automation_global_control(
            expected_revision,
            Mutation::Set(enabled),
            observed_at,
            || Ok(()),
        )
    }

    #[cfg(test)]
    pub(super) fn set_automation_global_enabled_after_commit_failure_for_test(
        &self,
        expected_revision: u64,
        enabled: bool,
        observed_at: SystemTime,
    ) -> Result<AutomationGlobalControlUpdate, HistoryError> {
        self.mutate_automation_global_control(
            expected_revision,
            Mutation::Set(enabled),
            observed_at,
            || Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable)),
        )
    }
}

pub(crate) fn load_automation_global_control(
    connection: &Connection,
) -> Result<AutomationGlobalControl, HistoryError> {
    Ok(load_stored(connection)?.map_or(AutomationGlobalControl::DEFAULT, |stored| stored.control))
}

fn load_stored(connection: &Connection) -> Result<Option<StoredControl>, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(
                "SELECT value_json, value_schema_version, updated_at_unix_ms
                 FROM settings WHERE setting_key = ?1",
                [AUTOMATION_GLOBAL_CONTROL_KEY],
                raw_control,
            )
            .optional()
            .map_err(map_query_sql_error)?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        if raw.value_schema_version > VALUE_SCHEMA_VERSION {
            return Err(HistoryError::new(HistoryErrorKind::IncompatibleSchema));
        }
        if raw.value_schema_version != VALUE_SCHEMA_VERSION
            || !(0..=MAX_AUTOMATION_UNIX_MS).contains(&raw.updated_at_unix_ms)
        {
            return Err(corrupt());
        }
        let value: StoredValue =
            serde_json::from_str(&raw.canonical_json).map_err(|_| corrupt())?;
        if canonical_value_json(&value, HistoryErrorKind::CorruptData)? != raw.canonical_json
            || value.revision == 0
            || value.revision > MAX_REVISION
        {
            return Err(corrupt());
        }
        let (enabled, source) = match (value.source, value.enabled) {
            (StoredSource::Default, false) => (false, AutomationGlobalControlSource::Default),
            (StoredSource::Stored, enabled) => (enabled, AutomationGlobalControlSource::Stored),
            _ => return Err(corrupt()),
        };
        Ok(Some(StoredControl {
            control: AutomationGlobalControl {
                enabled,
                source,
                revision: value.revision,
                updated_at: Some(unix_ms_to_system_time(raw.updated_at_unix_ms)?),
            },
            canonical_json: raw.canonical_json,
            value_schema_version: raw.value_schema_version,
            updated_at_unix_ms: raw.updated_at_unix_ms,
        }))
    })
}

fn raw_control(row: &Row<'_>) -> rusqlite::Result<RawControl> {
    let bytes = match row.get_ref(0)? {
        ValueRef::Text(bytes) if (1..=MAX_CANONICAL_VALUE_BYTES).contains(&bytes.len()) => bytes,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let value_schema_version = match row.get_ref(1)? {
        ValueRef::Integer(value) => value,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let updated_at_unix_ms = match row.get_ref(2)? {
        ValueRef::Integer(value) => value,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(RawControl {
        canonical_json: std::str::from_utf8(bytes)
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            .to_owned(),
        value_schema_version,
        updated_at_unix_ms,
    })
}

fn write_row(
    transaction: &Transaction<'_>,
    original: Option<&StoredControl>,
    canonical_json: &str,
    updated_at_unix_ms: i64,
) -> Result<(), HistoryError> {
    let changed = match original {
        Some(original) => transaction
            .execute(
                "UPDATE settings SET value_json = ?1, value_schema_version = ?2,
                     updated_at_unix_ms = ?3
                 WHERE setting_key = ?4 AND value_json = ?5
                   AND value_schema_version = ?6 AND updated_at_unix_ms = ?7",
                params![
                    canonical_json,
                    VALUE_SCHEMA_VERSION,
                    updated_at_unix_ms,
                    AUTOMATION_GLOBAL_CONTROL_KEY,
                    original.canonical_json,
                    original.value_schema_version,
                    original.updated_at_unix_ms,
                ],
            )
            .map_err(map_write_sql_error)?,
        None => transaction
            .execute(
                "INSERT INTO settings (
                     setting_key, value_json, value_schema_version, updated_at_unix_ms
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    AUTOMATION_GLOBAL_CONTROL_KEY,
                    canonical_json,
                    VALUE_SCHEMA_VERSION,
                    updated_at_unix_ms,
                ],
            )
            .map_err(map_write_sql_error)?,
    };
    if changed != 1 {
        return Err(HistoryError::new(HistoryErrorKind::InternalState));
    }
    Ok(())
}

fn reconcile(
    coordinator: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    original: Option<&StoredControl>,
    expected: AutomationGlobalControl,
    failure: HistoryError,
) -> Result<AutomationGlobalControlUpdate, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored(&guard.connection) {
        Ok(Some(current)) if current.control == expected => Ok(AutomationGlobalControlUpdate {
            control: expected,
            changed: true,
        }),
        Ok(current) if optional_exact(current.as_ref(), original) => Err(failure),
        Ok(_) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn optional_exact(left: Option<&StoredControl>, right: Option<&StoredControl>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.control == right.control
                && left.canonical_json == right.canonical_json
                && left.value_schema_version == right.value_schema_version
                && left.updated_at_unix_ms == right.updated_at_unix_ms
        }
        _ => false,
    }
}

fn canonical_json(
    control: AutomationGlobalControl,
    kind: HistoryErrorKind,
) -> Result<String, HistoryError> {
    canonical_value_json(
        &StoredValue {
            enabled: control.enabled,
            revision: control.revision,
            source: match control.source {
                AutomationGlobalControlSource::Default => StoredSource::Default,
                AutomationGlobalControlSource::Stored => StoredSource::Stored,
            },
        },
        kind,
    )
}

fn canonical_value_json(
    value: &StoredValue,
    kind: HistoryErrorKind,
) -> Result<String, HistoryError> {
    let json = serde_json::to_string(value).map_err(|_| HistoryError::new(kind))?;
    if json.is_empty() || json.len() > MAX_CANONICAL_VALUE_BYTES {
        return Err(HistoryError::new(kind));
    }
    Ok(json)
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;

    fn open(temp: &TempDir) -> Arc<StoreCoordinator> {
        StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap()
    }

    #[test]
    fn missing_control_is_disabled_without_a_write() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        assert_eq!(
            store.load_automation_global_control().unwrap(),
            AutomationGlobalControl::DEFAULT
        );
        let reset = store.reset_automation_global_control(0).unwrap();
        assert!(!reset.changed);
        assert_eq!(reset.control, AutomationGlobalControl::DEFAULT);
        store.with_connection(|connection| {
            let count: i64 = connection
                .query_row(
                    "SELECT count(*) FROM settings WHERE setting_key = ?1",
                    [AUTOMATION_GLOBAL_CONTROL_KEY],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn explicit_control_is_revision_checked_monotonic_and_reconciled() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let first = store
            .set_automation_global_enabled_after_commit_failure_for_test(
                0,
                true,
                UNIX_EPOCH + Duration::from_millis(2_000),
            )
            .unwrap();
        assert!(first.changed);
        assert!(first.control.enabled);
        assert_eq!(first.control.revision, 1);
        assert_eq!(first.control.source, AutomationGlobalControlSource::Stored);
        let retry = store.set_automation_global_enabled(1, true).unwrap();
        assert!(!retry.changed);
        assert_eq!(retry.control, first.control);
        assert_eq!(
            store
                .set_automation_global_enabled(0, false)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        let disabled = store
            .set_automation_global_enabled_at_for_test(
                1,
                false,
                UNIX_EPOCH + Duration::from_millis(1_000),
            )
            .unwrap();
        assert_eq!(disabled.control.revision, 2);
        assert_eq!(disabled.control.updated_at, first.control.updated_at);
    }

    #[test]
    fn reset_after_stored_persists_default_with_next_revision_and_reopens() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let store = StoreCoordinator::open(&database).unwrap();
        let stored = store
            .set_automation_global_enabled_at_for_test(
                0,
                true,
                UNIX_EPOCH + Duration::from_millis(10),
            )
            .unwrap();
        let reset = store
            .reset_automation_global_control(stored.control.revision)
            .unwrap();
        assert!(reset.changed);
        assert!(!reset.control.enabled);
        assert_eq!(reset.control.source, AutomationGlobalControlSource::Default);
        assert_eq!(reset.control.revision, stored.control.revision + 1);
        store.with_connection(|connection| {
            let (value, count): (String, i64) = connection
                .query_row(
                    "SELECT value_json,
                            (SELECT count(*) FROM settings WHERE setting_key = ?1)
                     FROM settings WHERE setting_key = ?1",
                    [AUTOMATION_GLOBAL_CONTROL_KEY],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(count, 1);
            assert_eq!(
                value,
                "{\"enabled\":false,\"revision\":2,\"source\":\"default\"}"
            );
        });

        drop(store);
        let reopened = StoreCoordinator::open(&database).unwrap();
        assert_eq!(
            reopened.load_automation_global_control().unwrap(),
            reset.control
        );
    }

    #[test]
    fn corrupt_or_future_control_fails_closed() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version, updated_at_unix_ms
                     ) VALUES (?1, '{\"enabled\":true}', 1, 1)",
                    [AUTOMATION_GLOBAL_CONTROL_KEY],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_automation_global_control().unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE settings SET value_schema_version = 2 WHERE setting_key = ?1",
                    [AUTOMATION_GLOBAL_CONTROL_KEY],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_automation_global_control().unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
    }
}
