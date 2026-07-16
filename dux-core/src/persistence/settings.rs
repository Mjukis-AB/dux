//! Typed settings stored behind exact, versioned keys.
//!
//! Missing rows use versioned core defaults without writing. This module does
//! not expose arbitrary keys or raw JSON, and settings never grant cleanup or
//! filesystem authority.

use std::time::SystemTime;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::store::{HistoryConnectionGuard, StoreCoordinator};

pub(crate) const DEFAULT_SNAPSHOT_RETENTION_CAP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SNAPSHOT_RETENTION_KEY: &str = "snapshot_retention";
const SNAPSHOT_RETENTION_VALUE_SCHEMA_VERSION: i64 = 1;
const MAX_CANONICAL_VALUE_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotRetentionCapSettingSource {
    Default,
    Stored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotRetentionCapSetting {
    pub(crate) cap_bytes: u64,
    pub(crate) source: SnapshotRetentionCapSettingSource,
    pub(crate) updated_at: Option<SystemTime>,
}

impl SnapshotRetentionCapSetting {
    const DEFAULT: Self = Self {
        cap_bytes: DEFAULT_SNAPSHOT_RETENTION_CAP_BYTES,
        source: SnapshotRetentionCapSettingSource::Default,
        updated_at: None,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotRetentionCapSettingUpdate {
    pub(crate) settings: SnapshotRetentionCapSetting,
    pub(crate) changed: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotRetentionValueV1 {
    cap_bytes: u64,
}

struct StoredSnapshotRetentionSetting {
    setting: SnapshotRetentionCapSetting,
    canonical_json: String,
    updated_at_unix_ms: i64,
}

struct RawSnapshotRetentionSetting {
    canonical_json: String,
    value_schema_version: i64,
    updated_at_unix_ms: i64,
}

impl StoreCoordinator {
    pub(crate) fn load_snapshot_retention_cap(
        &self,
    ) -> Result<SnapshotRetentionCapSetting, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_snapshot_retention_cap(&guard.connection)
    }

    pub(super) fn load_snapshot_retention_cap_with_guard(
        &self,
        guard: &HistoryConnectionGuard<'_>,
    ) -> Result<SnapshotRetentionCapSetting, HistoryError> {
        self.validate_history_guard(guard)?;
        load_snapshot_retention_cap(&guard.connection)
    }

    pub(crate) fn set_snapshot_retention_cap(
        &self,
        cap_bytes: u64,
    ) -> Result<SnapshotRetentionCapSettingUpdate, HistoryError> {
        self.set_snapshot_retention_cap_with_hook(cap_bytes, SystemTime::now(), || Ok(()))
    }

    fn set_snapshot_retention_cap_with_hook(
        &self,
        cap_bytes: u64,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<SnapshotRetentionCapSettingUpdate, HistoryError> {
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        let canonical_json = canonical_json(cap_bytes, HistoryErrorKind::InvalidInput)?;
        let mut guard = self.lock_current_history_connection()?;
        let original = load_stored_snapshot_retention_cap(&guard.connection)?;
        if let Some(stored) = original
            .as_ref()
            .filter(|stored| stored.setting.cap_bytes == cap_bytes)
        {
            return Ok(SnapshotRetentionCapSettingUpdate {
                settings: stored.setting,
                changed: false,
            });
        }
        let updated_at_unix_ms = original.as_ref().map_or(observed_at_unix_ms, |stored| {
            observed_at_unix_ms.max(stored.updated_at_unix_ms)
        });
        let expected = SnapshotRetentionCapSetting {
            cap_bytes,
            source: SnapshotRetentionCapSettingSource::Stored,
            updated_at: Some(unix_ms_to_system_time(updated_at_unix_ms)?),
        };

        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let changed = match &original {
            Some(stored) => transaction
                .execute(
                    "UPDATE settings
                     SET value_json = ?1, value_schema_version = ?2,
                         updated_at_unix_ms = ?3
                     WHERE setting_key = ?4
                       AND value_json = ?5
                       AND value_schema_version = ?2
                       AND updated_at_unix_ms = ?6",
                    params![
                        canonical_json,
                        SNAPSHOT_RETENTION_VALUE_SCHEMA_VERSION,
                        updated_at_unix_ms,
                        SNAPSHOT_RETENTION_KEY,
                        stored.canonical_json,
                        stored.updated_at_unix_ms,
                    ],
                )
                .map_err(map_write_sql_error)?,
            None => transaction
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version,
                         updated_at_unix_ms
                     ) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        SNAPSHOT_RETENTION_KEY,
                        canonical_json,
                        SNAPSHOT_RETENTION_VALUE_SCHEMA_VERSION,
                        updated_at_unix_ms,
                    ],
                )
                .map_err(map_write_sql_error)?,
        };
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => {
                return Ok(SnapshotRetentionCapSettingUpdate {
                    settings: expected,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        reconcile_setting_write(self, &guard, original.as_ref(), expected, failure)
    }

    pub(crate) fn reset_snapshot_retention_cap(
        &self,
    ) -> Result<SnapshotRetentionCapSettingUpdate, HistoryError> {
        self.reset_snapshot_retention_cap_with_hook(|| Ok(()))
    }

    fn reset_snapshot_retention_cap_with_hook(
        &self,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<SnapshotRetentionCapSettingUpdate, HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        let Some(original) = load_stored_snapshot_retention_cap(&guard.connection)? else {
            return Ok(SnapshotRetentionCapSettingUpdate {
                settings: SnapshotRetentionCapSetting::DEFAULT,
                changed: false,
            });
        };
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let changed = transaction
            .execute(
                "DELETE FROM settings
                 WHERE setting_key = ?1
                   AND value_json = ?2
                   AND value_schema_version = ?3
                   AND updated_at_unix_ms = ?4",
                params![
                    SNAPSHOT_RETENTION_KEY,
                    original.canonical_json,
                    SNAPSHOT_RETENTION_VALUE_SCHEMA_VERSION,
                    original.updated_at_unix_ms,
                ],
            )
            .map_err(map_write_sql_error)?;
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => {
                return Ok(SnapshotRetentionCapSettingUpdate {
                    settings: SnapshotRetentionCapSetting::DEFAULT,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        if self.revalidate_current_history_guard(&guard).is_err() {
            return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
        }
        match load_stored_snapshot_retention_cap(&guard.connection) {
            Ok(None) => Ok(SnapshotRetentionCapSettingUpdate {
                settings: SnapshotRetentionCapSetting::DEFAULT,
                changed: true,
            }),
            Ok(Some(current)) if stored_exact(&current, &original) => Err(failure),
            Ok(Some(_)) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
        }
    }

    #[cfg(test)]
    pub(super) fn set_snapshot_retention_cap_at_for_test(
        &self,
        cap_bytes: u64,
        observed_at: SystemTime,
    ) -> Result<SnapshotRetentionCapSettingUpdate, HistoryError> {
        self.set_snapshot_retention_cap_with_hook(cap_bytes, observed_at, || Ok(()))
    }

    #[cfg(test)]
    pub(super) fn set_snapshot_retention_cap_after_commit_failure_for_test(
        &self,
        cap_bytes: u64,
        observed_at: SystemTime,
    ) -> Result<SnapshotRetentionCapSettingUpdate, HistoryError> {
        self.set_snapshot_retention_cap_with_hook(cap_bytes, observed_at, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    #[cfg(test)]
    pub(super) fn reset_snapshot_retention_cap_after_commit_failure_for_test(
        &self,
    ) -> Result<SnapshotRetentionCapSettingUpdate, HistoryError> {
        self.reset_snapshot_retention_cap_with_hook(|| {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }
}

fn reconcile_setting_write(
    coordinator: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    original: Option<&StoredSnapshotRetentionSetting>,
    expected: SnapshotRetentionCapSetting,
    failure: HistoryError,
) -> Result<SnapshotRetentionCapSettingUpdate, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored_snapshot_retention_cap(&guard.connection) {
        Ok(Some(current)) if current.setting == expected => Ok(SnapshotRetentionCapSettingUpdate {
            settings: expected,
            changed: true,
        }),
        Ok(current) if optional_stored_exact(current.as_ref(), original) => Err(failure),
        Ok(_) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn optional_stored_exact(
    left: Option<&StoredSnapshotRetentionSetting>,
    right: Option<&StoredSnapshotRetentionSetting>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => stored_exact(left, right),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn stored_exact(
    left: &StoredSnapshotRetentionSetting,
    right: &StoredSnapshotRetentionSetting,
) -> bool {
    left.setting == right.setting
        && left.canonical_json == right.canonical_json
        && left.updated_at_unix_ms == right.updated_at_unix_ms
}

fn load_snapshot_retention_cap(
    connection: &Connection,
) -> Result<SnapshotRetentionCapSetting, HistoryError> {
    Ok(load_stored_snapshot_retention_cap(connection)?
        .map_or(SnapshotRetentionCapSetting::DEFAULT, |stored| {
            stored.setting
        }))
}

fn load_stored_snapshot_retention_cap(
    connection: &Connection,
) -> Result<Option<StoredSnapshotRetentionSetting>, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(
                "SELECT value_json, value_schema_version, updated_at_unix_ms
                 FROM settings
                 WHERE setting_key = ?1",
                [SNAPSHOT_RETENTION_KEY],
                raw_snapshot_retention_setting,
            )
            .optional()
            .map_err(map_query_sql_error)?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        if raw.value_schema_version != SNAPSHOT_RETENTION_VALUE_SCHEMA_VERSION {
            return Err(
                if raw.value_schema_version > SNAPSHOT_RETENTION_VALUE_SCHEMA_VERSION {
                    HistoryError::new(HistoryErrorKind::IncompatibleSchema)
                } else {
                    corrupt()
                },
            );
        }
        let value: SnapshotRetentionValueV1 =
            serde_json::from_str(&raw.canonical_json).map_err(|_| corrupt())?;
        if canonical_json(value.cap_bytes, HistoryErrorKind::CorruptData)? != raw.canonical_json {
            return Err(corrupt());
        }
        let updated_at = unix_ms_to_system_time(raw.updated_at_unix_ms)?;
        Ok(Some(StoredSnapshotRetentionSetting {
            setting: SnapshotRetentionCapSetting {
                cap_bytes: value.cap_bytes,
                source: SnapshotRetentionCapSettingSource::Stored,
                updated_at: Some(updated_at),
            },
            canonical_json: raw.canonical_json,
            updated_at_unix_ms: raw.updated_at_unix_ms,
        }))
    })
}

fn raw_snapshot_retention_setting(row: &Row<'_>) -> rusqlite::Result<RawSnapshotRetentionSetting> {
    let json_bytes = match row.get_ref(0)? {
        ValueRef::Text(bytes) if (1..=MAX_CANONICAL_VALUE_BYTES).contains(&bytes.len()) => bytes,
        ValueRef::Null
        | ValueRef::Integer(_)
        | ValueRef::Real(_)
        | ValueRef::Text(_)
        | ValueRef::Blob(_) => {
            return Err(rusqlite::Error::InvalidQuery);
        }
    };
    let value_schema_version = match row.get_ref(1)? {
        ValueRef::Integer(value) => value,
        ValueRef::Null | ValueRef::Real(_) | ValueRef::Text(_) | ValueRef::Blob(_) => {
            return Err(rusqlite::Error::InvalidQuery);
        }
    };
    let updated_at_unix_ms = match row.get_ref(2)? {
        ValueRef::Integer(value) => value,
        ValueRef::Null | ValueRef::Real(_) | ValueRef::Text(_) | ValueRef::Blob(_) => {
            return Err(rusqlite::Error::InvalidQuery);
        }
    };
    let canonical_json = std::str::from_utf8(json_bytes)
        .map_err(|_| rusqlite::Error::InvalidQuery)?
        .to_owned();
    Ok(RawSnapshotRetentionSetting {
        canonical_json,
        value_schema_version,
        updated_at_unix_ms,
    })
}

fn canonical_json(cap_bytes: u64, kind: HistoryErrorKind) -> Result<String, HistoryError> {
    let json = serde_json::to_string(&SnapshotRetentionValueV1 { cap_bytes })
        .map_err(|_| HistoryError::new(kind))?;
    if json.len() > MAX_CANONICAL_VALUE_BYTES {
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
    fn absent_setting_uses_default_without_writing_or_touching_unknown_keys() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version,
                         updated_at_unix_ms
                     ) VALUES ('future-setting', '{\"kept\":true}', 7, 1)",
                    [],
                )
                .unwrap();
        });

        assert_eq!(
            store.load_snapshot_retention_cap().unwrap(),
            SnapshotRetentionCapSetting::DEFAULT
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM settings
                         WHERE setting_key = ?1",
                        [SNAPSHOT_RETENTION_KEY],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                0
            );
            assert_eq!(
                connection
                    .query_row(
                        "SELECT value_json FROM settings
                         WHERE setting_key = 'future-setting'",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap(),
                "{\"kept\":true}"
            );
        });
    }

    #[test]
    fn set_exact_retry_reopen_and_reset_cover_the_full_u64_domain() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let observed = UNIX_EPOCH + Duration::from_millis(1_750_000_000_000);
        let store = StoreCoordinator::open(&database).unwrap();

        let explicit_default = store
            .set_snapshot_retention_cap_at_for_test(DEFAULT_SNAPSHOT_RETENTION_CAP_BYTES, observed)
            .unwrap();
        assert!(explicit_default.changed);
        assert_eq!(
            explicit_default.settings.source,
            SnapshotRetentionCapSettingSource::Stored
        );
        let default_retry = store
            .set_snapshot_retention_cap_at_for_test(
                DEFAULT_SNAPSHOT_RETENTION_CAP_BYTES,
                observed + Duration::from_millis(1),
            )
            .unwrap();
        assert!(!default_retry.changed);
        assert_eq!(default_retry.settings, explicit_default.settings);

        let zero = store
            .set_snapshot_retention_cap_at_for_test(0, observed + Duration::from_millis(2))
            .unwrap();
        assert!(zero.changed);
        assert_eq!(zero.settings.cap_bytes, 0);
        assert_eq!(
            zero.settings.source,
            SnapshotRetentionCapSettingSource::Stored
        );
        assert_eq!(
            zero.settings.updated_at,
            Some(observed + Duration::from_millis(2))
        );
        let retry = store
            .set_snapshot_retention_cap_at_for_test(0, observed + Duration::from_secs(1))
            .unwrap();
        assert!(!retry.changed);
        assert_eq!(retry.settings, zero.settings);

        let maximum = store
            .set_snapshot_retention_cap_after_commit_failure_for_test(
                u64::MAX,
                observed + Duration::from_secs(2),
            )
            .unwrap();
        assert!(maximum.changed);
        assert_eq!(maximum.settings.cap_bytes, u64::MAX);
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT value_json FROM settings
                         WHERE setting_key = ?1",
                        [SNAPSHOT_RETENTION_KEY],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap(),
                format!("{{\"cap_bytes\":{}}}", u64::MAX)
            );
        });
        drop(store);

        let reopened = StoreCoordinator::open(&database).unwrap();
        assert_eq!(
            reopened.load_snapshot_retention_cap().unwrap(),
            maximum.settings
        );
        let reset = reopened
            .reset_snapshot_retention_cap_after_commit_failure_for_test()
            .unwrap();
        assert!(reset.changed);
        assert_eq!(reset.settings, SnapshotRetentionCapSetting::DEFAULT);
        let exact_reset = reopened.reset_snapshot_retention_cap().unwrap();
        assert!(!exact_reset.changed);
        assert_eq!(exact_reset.settings, SnapshotRetentionCapSetting::DEFAULT);
    }

    #[test]
    fn malformed_or_newer_known_setting_never_falls_back_or_gets_overwritten() {
        let malformed = [
            "{\"cap_bytes\":1,\"extra\":2}",
            " {\"cap_bytes\":1}",
            "{\"cap_bytes\":-1}",
            "{\"cap_bytes\":1.0}",
            "{\"cap_bytes\":\"1\"}",
            "{}",
            "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        ];
        for (index, value) in malformed.into_iter().enumerate() {
            let temp = TempDir::new().unwrap();
            let store = open(&temp);
            store.with_connection(|connection| {
                connection
                    .execute(
                        "INSERT INTO settings (
                             setting_key, value_json, value_schema_version,
                             updated_at_unix_ms
                         ) VALUES (?1, ?2, 1, ?3)",
                        params![SNAPSHOT_RETENTION_KEY, value, index as i64],
                    )
                    .unwrap();
            });
            assert_eq!(
                store.load_snapshot_retention_cap().unwrap_err().kind,
                HistoryErrorKind::CorruptData,
                "fixture {index}"
            );
        }

        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let canonical = canonical_json(123, HistoryErrorKind::InvalidInput).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version,
                         updated_at_unix_ms
                     ) VALUES (?1, ?2, 2, 1)",
                    params![SNAPSHOT_RETENTION_KEY, canonical],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_snapshot_retention_cap().unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
        assert_eq!(
            store.set_snapshot_retention_cap(456).unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
        assert_eq!(
            store.reset_snapshot_retention_cap().unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT value_schema_version FROM settings
                         WHERE setting_key = ?1",
                        [SNAPSHOT_RETENTION_KEY],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                2
            );
        });
    }

    #[test]
    fn invalid_clock_precedes_mutation_and_exact_post_commit_states_are_adopted() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        assert_eq!(
            store
                .set_snapshot_retention_cap_at_for_test(1, UNIX_EPOCH - Duration::from_millis(1),)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidInput
        );
        assert_eq!(
            store.load_snapshot_retention_cap().unwrap(),
            SnapshotRetentionCapSetting::DEFAULT
        );

        let observed = UNIX_EPOCH + Duration::from_millis(1_750_000_000_000);
        let update = store
            .set_snapshot_retention_cap_after_commit_failure_for_test(42, observed)
            .unwrap();
        assert!(update.changed);
        assert_eq!(update.settings.cap_bytes, 42);
        let reset = store
            .reset_snapshot_retention_cap_after_commit_failure_for_test()
            .unwrap();
        assert!(reset.changed);
        assert_eq!(reset.settings, SnapshotRetentionCapSetting::DEFAULT);
    }
}
