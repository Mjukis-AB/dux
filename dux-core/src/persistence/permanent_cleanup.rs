//! Versioned global permanent-cleanup policy.
//!
//! The switch is stored as typed data behind one exact settings key. Changing
//! it takes the store-wide cleanup exclusion so an in-flight permanent effect
//! cannot race a disable write. The policy itself grants no target authority;
//! the journal checks it while holding the same exclusion immediately before
//! recording `effect_started`.

use std::time::{Duration, SystemTime};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::store::{HistoryConnectionGuard, StoreCoordinator};

pub(crate) const PERMANENT_CLEANUP_KEY: &str = "permanent_cleanup";
const VALUE_SCHEMA_VERSION: i64 = 1;
const MAX_CANONICAL_VALUE_BYTES: usize = 128;
const MAX_POLICY_REVISION: u64 = i64::MAX as u64;
const CLEANUP_POLICY_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PermanentCleanupSettingSource {
    Default,
    Stored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PermanentCleanupSetting {
    pub(crate) enabled: bool,
    pub(crate) source: PermanentCleanupSettingSource,
    pub(crate) revision: u64,
    pub(crate) updated_at: Option<SystemTime>,
}

impl PermanentCleanupSetting {
    /// Permanent cleanup remains enabled by default; the explicit switch is
    /// an emergency/user kill switch and a stored `false` value is required to
    /// block the effect boundary.
    pub(crate) const DEFAULT: Self = Self {
        enabled: true,
        source: PermanentCleanupSettingSource::Default,
        revision: 0,
        updated_at: None,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PermanentCleanupSettingUpdate {
    pub(crate) settings: PermanentCleanupSetting,
    pub(crate) changed: bool,
}

#[derive(Clone, Copy, Debug)]
enum PolicyMutation {
    Set(bool),
    Reset,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StoredPolicySource {
    Default,
    Stored,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PermanentCleanupValueV1 {
    enabled: bool,
    revision: u64,
    source: StoredPolicySource,
}

struct StoredPermanentCleanupSetting {
    setting: PermanentCleanupSetting,
    canonical_json: String,
    updated_at_unix_ms: i64,
}

struct RawPermanentCleanupSetting {
    canonical_json: String,
    value_schema_version: i64,
    updated_at_unix_ms: i64,
}

impl StoreCoordinator {
    pub(crate) fn load_permanent_cleanup_setting(
        &self,
    ) -> Result<PermanentCleanupSetting, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_permanent_cleanup_setting(&guard.connection)
    }

    pub(crate) fn set_permanent_cleanup_enabled(
        &self,
        enabled: bool,
    ) -> Result<PermanentCleanupSettingUpdate, HistoryError> {
        self.mutate_permanent_cleanup(PolicyMutation::Set(enabled), SystemTime::now(), || Ok(()))
    }

    pub(crate) fn reset_permanent_cleanup(
        &self,
    ) -> Result<PermanentCleanupSettingUpdate, HistoryError> {
        self.mutate_permanent_cleanup(PolicyMutation::Reset, SystemTime::now(), || Ok(()))
    }

    fn mutate_permanent_cleanup(
        &self,
        mutation: PolicyMutation,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<PermanentCleanupSettingUpdate, HistoryError> {
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        // The setting and the effect boundary share this exclusion. A disable
        // write therefore cannot interleave between the final enable check
        // and a permanent effect's journal receipt.
        let cleanup_guard = self.acquire_cleanup_lock_for_journal(CLEANUP_POLICY_LOCK_TIMEOUT)?;
        self.validate_cleanup_lock_for_journal(&cleanup_guard)?;
        let mut guard = self.lock_current_history_connection()?;
        self.validate_cleanup_lock_for_journal(&cleanup_guard)?;
        let original = load_stored_permanent_cleanup_setting(&guard.connection)?;
        let current = original
            .as_ref()
            .map_or(PermanentCleanupSetting::DEFAULT, |stored| stored.setting);

        let (enabled, source) = match mutation {
            PolicyMutation::Set(enabled) => {
                if current.source == PermanentCleanupSettingSource::Stored
                    && current.enabled == enabled
                {
                    return Ok(PermanentCleanupSettingUpdate {
                        settings: current,
                        changed: false,
                    });
                }
                (enabled, PermanentCleanupSettingSource::Stored)
            }
            PolicyMutation::Reset => {
                if current.source == PermanentCleanupSettingSource::Default {
                    return Ok(PermanentCleanupSettingUpdate {
                        settings: current,
                        changed: false,
                    });
                }
                (
                    PermanentCleanupSetting::DEFAULT.enabled,
                    PermanentCleanupSettingSource::Default,
                )
            }
        };
        let revision = current
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_POLICY_REVISION)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
        let updated_at_unix_ms = original.as_ref().map_or(observed_at_unix_ms, |stored| {
            observed_at_unix_ms.max(stored.updated_at_unix_ms)
        });
        let expected = PermanentCleanupSetting {
            enabled,
            source,
            revision,
            updated_at: Some(unix_ms_to_system_time(updated_at_unix_ms)?),
        };
        let canonical_json = canonical_json(expected, HistoryErrorKind::InvalidInput)?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let transactional_original = load_stored_permanent_cleanup_setting(&transaction)?;
        if !optional_stored_exact(transactional_original.as_ref(), original.as_ref()) {
            return Err(HistoryError::new(HistoryErrorKind::InternalState));
        }
        write_setting_row(
            &transaction,
            original.as_ref(),
            &canonical_json,
            updated_at_unix_ms,
        )?;
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => {
                return Ok(PermanentCleanupSettingUpdate {
                    settings: expected,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        reconcile_policy_write(self, &guard, original.as_ref(), expected, failure)
    }

    #[cfg(test)]
    pub(super) fn set_permanent_cleanup_enabled_at_for_test(
        &self,
        enabled: bool,
        observed_at: SystemTime,
    ) -> Result<PermanentCleanupSettingUpdate, HistoryError> {
        self.mutate_permanent_cleanup(PolicyMutation::Set(enabled), observed_at, || Ok(()))
    }

    #[cfg(test)]
    pub(super) fn reset_permanent_cleanup_at_for_test(
        &self,
        observed_at: SystemTime,
    ) -> Result<PermanentCleanupSettingUpdate, HistoryError> {
        self.mutate_permanent_cleanup(PolicyMutation::Reset, observed_at, || Ok(()))
    }

    #[cfg(test)]
    pub(super) fn set_permanent_cleanup_after_commit_failure_for_test(
        &self,
        enabled: bool,
        observed_at: SystemTime,
    ) -> Result<PermanentCleanupSettingUpdate, HistoryError> {
        self.mutate_permanent_cleanup(PolicyMutation::Set(enabled), observed_at, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }
}

pub(crate) fn load_permanent_cleanup_setting(
    connection: &Connection,
) -> Result<PermanentCleanupSetting, HistoryError> {
    Ok(load_stored_permanent_cleanup_setting(connection)?
        .map_or(PermanentCleanupSetting::DEFAULT, |stored| stored.setting))
}

fn write_setting_row(
    transaction: &Transaction<'_>,
    original: Option<&StoredPermanentCleanupSetting>,
    canonical_json: &str,
    updated_at_unix_ms: i64,
) -> Result<(), HistoryError> {
    let changed = match original {
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
                    VALUE_SCHEMA_VERSION,
                    updated_at_unix_ms,
                    PERMANENT_CLEANUP_KEY,
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
                    PERMANENT_CLEANUP_KEY,
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

fn reconcile_policy_write(
    coordinator: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    original: Option<&StoredPermanentCleanupSetting>,
    expected: PermanentCleanupSetting,
    failure: HistoryError,
) -> Result<PermanentCleanupSettingUpdate, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored_permanent_cleanup_setting(&guard.connection) {
        Ok(Some(current)) if current.setting == expected => Ok(PermanentCleanupSettingUpdate {
            settings: expected,
            changed: true,
        }),
        Ok(current) if optional_stored_exact(current.as_ref(), original) => Err(failure),
        Ok(_) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn load_stored_permanent_cleanup_setting(
    connection: &Connection,
) -> Result<Option<StoredPermanentCleanupSetting>, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(
                "SELECT value_json, value_schema_version, updated_at_unix_ms
                 FROM settings WHERE setting_key = ?1",
                [PERMANENT_CLEANUP_KEY],
                raw_permanent_cleanup_setting,
            )
            .optional()
            .map_err(map_query_sql_error)?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        if raw.value_schema_version != VALUE_SCHEMA_VERSION {
            return Err(if raw.value_schema_version > VALUE_SCHEMA_VERSION {
                HistoryError::new(HistoryErrorKind::IncompatibleSchema)
            } else {
                corrupt()
            });
        }
        let value: PermanentCleanupValueV1 =
            serde_json::from_str(&raw.canonical_json).map_err(|_| corrupt())?;
        let setting = setting_from_value(&value, raw.updated_at_unix_ms)?;
        if canonical_json(setting, HistoryErrorKind::CorruptData)? != raw.canonical_json {
            return Err(corrupt());
        }
        Ok(Some(StoredPermanentCleanupSetting {
            setting,
            canonical_json: raw.canonical_json,
            updated_at_unix_ms: raw.updated_at_unix_ms,
        }))
    })
}

fn setting_from_value(
    value: &PermanentCleanupValueV1,
    updated_at_unix_ms: i64,
) -> Result<PermanentCleanupSetting, HistoryError> {
    if value.revision == 0 || value.revision > MAX_POLICY_REVISION {
        return Err(corrupt());
    }
    if value.source == StoredPolicySource::Default
        && value.enabled != PermanentCleanupSetting::DEFAULT.enabled
    {
        return Err(corrupt());
    }
    Ok(PermanentCleanupSetting {
        enabled: value.enabled,
        source: match value.source {
            StoredPolicySource::Default => PermanentCleanupSettingSource::Default,
            StoredPolicySource::Stored => PermanentCleanupSettingSource::Stored,
        },
        revision: value.revision,
        updated_at: Some(unix_ms_to_system_time(updated_at_unix_ms)?),
    })
}

fn raw_permanent_cleanup_setting(row: &Row<'_>) -> rusqlite::Result<RawPermanentCleanupSetting> {
    let json_bytes = match row.get_ref(0)? {
        ValueRef::Text(bytes) if (1..=MAX_CANONICAL_VALUE_BYTES).contains(&bytes.len()) => bytes,
        ValueRef::Null
        | ValueRef::Integer(_)
        | ValueRef::Real(_)
        | ValueRef::Text(_)
        | ValueRef::Blob(_) => return Err(rusqlite::Error::InvalidQuery),
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
    Ok(RawPermanentCleanupSetting {
        canonical_json,
        value_schema_version,
        updated_at_unix_ms,
    })
}

fn canonical_json(
    setting: PermanentCleanupSetting,
    kind: HistoryErrorKind,
) -> Result<String, HistoryError> {
    let value = PermanentCleanupValueV1 {
        enabled: setting.enabled,
        revision: setting.revision,
        source: match setting.source {
            PermanentCleanupSettingSource::Default => StoredPolicySource::Default,
            PermanentCleanupSettingSource::Stored => StoredPolicySource::Stored,
        },
    };
    let json = serde_json::to_string(&value).map_err(|_| HistoryError::new(kind))?;
    if json.len() > MAX_CANONICAL_VALUE_BYTES {
        return Err(HistoryError::new(kind));
    }
    Ok(json)
}

fn optional_stored_exact(
    left: Option<&StoredPermanentCleanupSetting>,
    right: Option<&StoredPermanentCleanupSetting>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => stored_exact(left, right),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn stored_exact(
    left: &StoredPermanentCleanupSetting,
    right: &StoredPermanentCleanupSetting,
) -> bool {
    left.setting == right.setting
        && left.canonical_json == right.canonical_json
        && left.updated_at_unix_ms == right.updated_at_unix_ms
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
    fn missing_setting_is_enabled_default_without_writing() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        assert_eq!(
            store.load_permanent_cleanup_setting().unwrap(),
            PermanentCleanupSetting::DEFAULT
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM settings WHERE setting_key = ?1",
                        [PERMANENT_CLEANUP_KEY],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                0
            );
        });
    }

    #[test]
    fn disable_enable_reset_and_retry_preserve_revision_and_provenance() {
        let temp = TempDir::new().unwrap();
        let observed = UNIX_EPOCH + Duration::from_millis(1_750_000_000_000);
        let store = open(&temp);
        let disabled = store
            .set_permanent_cleanup_enabled_at_for_test(false, observed)
            .unwrap();
        assert!(disabled.changed);
        assert!(!disabled.settings.enabled);
        assert_eq!(disabled.settings.revision, 1);
        assert_eq!(
            disabled.settings.source,
            PermanentCleanupSettingSource::Stored
        );
        let retry = store
            .set_permanent_cleanup_enabled_at_for_test(false, observed + Duration::from_secs(1))
            .unwrap();
        assert!(!retry.changed);
        assert_eq!(retry.settings, disabled.settings);
        let enabled = store
            .set_permanent_cleanup_after_commit_failure_for_test(
                true,
                observed + Duration::from_secs(2),
            )
            .unwrap();
        assert!(enabled.changed);
        assert!(enabled.settings.enabled);
        assert_eq!(enabled.settings.revision, 2);
        let reset = store
            .reset_permanent_cleanup_at_for_test(observed + Duration::from_secs(3))
            .unwrap();
        assert!(reset.changed);
        assert!(reset.settings.enabled);
        assert_eq!(
            reset.settings.source,
            PermanentCleanupSettingSource::Default
        );
        assert_eq!(reset.settings.revision, 3);
    }

    #[test]
    fn malformed_or_newer_setting_fails_closed() {
        for value in [
            "{}",
            "{\"enabled\":false,\"revision\":0,\"source\":\"stored\"}",
            "{\"enabled\":false,\"revision\":1,\"source\":\"default\"}",
            "{\"enabled\":false,\"revision\":1,\"source\":\"stored\",\"extra\":1}",
        ] {
            let temp = TempDir::new().unwrap();
            let store = open(&temp);
            store.with_connection(|connection| {
                connection
                    .execute(
                        "INSERT INTO settings (setting_key, value_json, value_schema_version, updated_at_unix_ms)
                         VALUES (?1, ?2, 1, 1)",
                        params![PERMANENT_CLEANUP_KEY, value],
                    )
                    .unwrap();
            });
            assert_eq!(
                store.load_permanent_cleanup_setting().unwrap_err().kind,
                HistoryErrorKind::CorruptData
            );
        }
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (setting_key, value_json, value_schema_version, updated_at_unix_ms)
                     VALUES (?1, '{\"enabled\":false,\"revision\":1,\"source\":\"stored\"}', 2, 1)",
                    [PERMANENT_CLEANUP_KEY],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_permanent_cleanup_setting().unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
        assert_eq!(
            store.set_permanent_cleanup_enabled(false).unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
    }
}
