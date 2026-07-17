//! Versioned disk-pressure policy stored behind one exact settings key.
//!
//! Missing state is the core default at revision zero. Explicit set/reset
//! operations retain provenance and monotonically advance a bounded revision;
//! they never grant scan, cleanup, notification, or scheduling authority.

use std::time::SystemTime;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::domain::{DiskPressureConfig, DiskPressureRecoveryMargin, DiskPressureThreshold};

use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::store::{HistoryConnectionGuard, StoreCoordinator};

pub(super) const DISK_PRESSURE_POLICY_KEY: &str = "disk_pressure_policy";
const VALUE_SCHEMA_VERSION: i64 = 1;
const MAX_CANONICAL_VALUE_BYTES: usize = 512;
const MAX_POLICY_REVISION: u64 = i64::MAX as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiskPressurePolicySettingSource {
    Default,
    Stored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiskPressurePolicySetting {
    pub(crate) config: DiskPressureConfig,
    pub(crate) source: DiskPressurePolicySettingSource,
    pub(crate) revision: u64,
    pub(crate) updated_at: Option<SystemTime>,
}

impl DiskPressurePolicySetting {
    pub(crate) const DEFAULT: Self = Self {
        config: DiskPressureConfig::DEFAULT,
        source: DiskPressurePolicySettingSource::Default,
        revision: 0,
        updated_at: None,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiskPressurePolicySettingUpdate {
    pub(crate) settings: DiskPressurePolicySetting,
    pub(crate) changed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StoredPolicySource {
    Default,
    Stored,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DiskPressurePolicyValueV1 {
    revision: u64,
    source: StoredPolicySource,
    critical_available_bytes: u64,
    critical_available_basis_points: u16,
    warning_available_bytes: u64,
    warning_available_basis_points: u16,
    recovery_bytes: u64,
    recovery_basis_points: u16,
}

struct StoredDiskPressurePolicy {
    setting: DiskPressurePolicySetting,
    canonical_json: String,
    updated_at_unix_ms: i64,
}

struct RawDiskPressurePolicy {
    canonical_json: String,
    value_schema_version: i64,
    updated_at_unix_ms: i64,
}

#[derive(Clone, Copy)]
enum PolicyMutation {
    Set(DiskPressureConfig),
    Reset,
}

impl StoreCoordinator {
    pub(crate) fn load_disk_pressure_policy(
        &self,
    ) -> Result<DiskPressurePolicySetting, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_disk_pressure_policy(&guard.connection)
    }

    pub(crate) fn set_disk_pressure_policy(
        &self,
        config: DiskPressureConfig,
    ) -> Result<DiskPressurePolicySettingUpdate, HistoryError> {
        self.mutate_disk_pressure_policy(PolicyMutation::Set(config), SystemTime::now(), || Ok(()))
    }

    pub(crate) fn reset_disk_pressure_policy(
        &self,
    ) -> Result<DiskPressurePolicySettingUpdate, HistoryError> {
        self.mutate_disk_pressure_policy(PolicyMutation::Reset, SystemTime::now(), || Ok(()))
    }

    fn mutate_disk_pressure_policy(
        &self,
        mutation: PolicyMutation,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<DiskPressurePolicySettingUpdate, HistoryError> {
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        let mut guard = self.lock_current_history_connection()?;
        let original = load_stored_disk_pressure_policy(&guard.connection)?;
        let current = original
            .as_ref()
            .map_or(DiskPressurePolicySetting::DEFAULT, |value| value.setting);

        let (config, source) = match mutation {
            PolicyMutation::Set(config) => {
                if current.source == DiskPressurePolicySettingSource::Stored
                    && current.config == config
                {
                    return Ok(DiskPressurePolicySettingUpdate {
                        settings: current,
                        changed: false,
                    });
                }
                (config, DiskPressurePolicySettingSource::Stored)
            }
            PolicyMutation::Reset => {
                if current.source == DiskPressurePolicySettingSource::Default {
                    return Ok(DiskPressurePolicySettingUpdate {
                        settings: current,
                        changed: false,
                    });
                }
                (
                    DiskPressureConfig::DEFAULT,
                    DiskPressurePolicySettingSource::Default,
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
        let expected = DiskPressurePolicySetting {
            config,
            source,
            revision,
            updated_at: Some(unix_ms_to_system_time(updated_at_unix_ms)?),
        };
        let canonical_json = canonical_json(expected, HistoryErrorKind::InvalidInput)?;

        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        // Re-read inside the final transaction. The retained writer lease
        // should make this exact, while the equality check keeps that
        // dependency explicit and fails closed if storage changed.
        let transactional_original = load_stored_disk_pressure_policy(&transaction)?;
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
                return Ok(DiskPressurePolicySettingUpdate {
                    settings: expected,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        reconcile_policy_write(self, &guard, original.as_ref(), expected, failure)
    }

    #[cfg(test)]
    pub(super) fn set_disk_pressure_policy_at_for_test(
        &self,
        config: DiskPressureConfig,
        observed_at: SystemTime,
    ) -> Result<DiskPressurePolicySettingUpdate, HistoryError> {
        self.mutate_disk_pressure_policy(PolicyMutation::Set(config), observed_at, || Ok(()))
    }

    #[cfg(test)]
    pub(super) fn reset_disk_pressure_policy_at_for_test(
        &self,
        observed_at: SystemTime,
    ) -> Result<DiskPressurePolicySettingUpdate, HistoryError> {
        self.mutate_disk_pressure_policy(PolicyMutation::Reset, observed_at, || Ok(()))
    }

    #[cfg(all(test, unix))]
    pub(super) fn set_disk_pressure_policy_with_after_commit_hook_for_test(
        &self,
        config: DiskPressureConfig,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<DiskPressurePolicySettingUpdate, HistoryError> {
        self.mutate_disk_pressure_policy(PolicyMutation::Set(config), observed_at, after_commit)
    }

    #[cfg(test)]
    pub(super) fn set_disk_pressure_policy_after_commit_failure_for_test(
        &self,
        config: DiskPressureConfig,
        observed_at: SystemTime,
    ) -> Result<DiskPressurePolicySettingUpdate, HistoryError> {
        self.mutate_disk_pressure_policy(PolicyMutation::Set(config), observed_at, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    #[cfg(test)]
    pub(super) fn reset_disk_pressure_policy_after_commit_failure_for_test(
        &self,
        observed_at: SystemTime,
    ) -> Result<DiskPressurePolicySettingUpdate, HistoryError> {
        self.mutate_disk_pressure_policy(PolicyMutation::Reset, observed_at, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }
}

pub(super) fn load_disk_pressure_policy(
    connection: &Connection,
) -> Result<DiskPressurePolicySetting, HistoryError> {
    Ok(load_stored_disk_pressure_policy(connection)?
        .map_or(DiskPressurePolicySetting::DEFAULT, |stored| stored.setting))
}

fn write_setting_row(
    transaction: &Transaction<'_>,
    original: Option<&StoredDiskPressurePolicy>,
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
                    DISK_PRESSURE_POLICY_KEY,
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
                    DISK_PRESSURE_POLICY_KEY,
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
    original: Option<&StoredDiskPressurePolicy>,
    expected: DiskPressurePolicySetting,
    failure: HistoryError,
) -> Result<DiskPressurePolicySettingUpdate, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored_disk_pressure_policy(&guard.connection) {
        Ok(Some(current)) if current.setting == expected => Ok(DiskPressurePolicySettingUpdate {
            settings: expected,
            changed: true,
        }),
        Ok(current) if optional_stored_exact(current.as_ref(), original) => Err(failure),
        Ok(_) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn load_stored_disk_pressure_policy(
    connection: &Connection,
) -> Result<Option<StoredDiskPressurePolicy>, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(
                "SELECT value_json, value_schema_version, updated_at_unix_ms
                 FROM settings WHERE setting_key = ?1",
                [DISK_PRESSURE_POLICY_KEY],
                raw_disk_pressure_policy,
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
        let value: DiskPressurePolicyValueV1 =
            serde_json::from_str(&raw.canonical_json).map_err(|_| corrupt())?;
        let setting = setting_from_value(&value, raw.updated_at_unix_ms)?;
        if canonical_json(setting, HistoryErrorKind::CorruptData)? != raw.canonical_json {
            return Err(corrupt());
        }
        Ok(Some(StoredDiskPressurePolicy {
            setting,
            canonical_json: raw.canonical_json,
            updated_at_unix_ms: raw.updated_at_unix_ms,
        }))
    })
}

fn setting_from_value(
    value: &DiskPressurePolicyValueV1,
    updated_at_unix_ms: i64,
) -> Result<DiskPressurePolicySetting, HistoryError> {
    if value.revision == 0 || value.revision > MAX_POLICY_REVISION {
        return Err(corrupt());
    }
    let kind = HistoryErrorKind::CorruptData;
    let critical = DiskPressureThreshold::new(
        value.critical_available_bytes,
        value.critical_available_basis_points,
    )
    .map_err(|_| HistoryError::new(kind))?;
    let warning = DiskPressureThreshold::new(
        value.warning_available_bytes,
        value.warning_available_basis_points,
    )
    .map_err(|_| HistoryError::new(kind))?;
    let recovery =
        DiskPressureRecoveryMargin::new(value.recovery_bytes, value.recovery_basis_points)
            .map_err(|_| HistoryError::new(kind))?;
    let config = DiskPressureConfig::new(critical, warning, recovery)
        .map_err(|_| HistoryError::new(kind))?;
    if value.source == StoredPolicySource::Default && config != DiskPressureConfig::DEFAULT {
        return Err(corrupt());
    }
    Ok(DiskPressurePolicySetting {
        config,
        source: match value.source {
            StoredPolicySource::Default => DiskPressurePolicySettingSource::Default,
            StoredPolicySource::Stored => DiskPressurePolicySettingSource::Stored,
        },
        revision: value.revision,
        updated_at: Some(unix_ms_to_system_time(updated_at_unix_ms)?),
    })
}

fn raw_disk_pressure_policy(row: &Row<'_>) -> rusqlite::Result<RawDiskPressurePolicy> {
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
    Ok(RawDiskPressurePolicy {
        canonical_json,
        value_schema_version,
        updated_at_unix_ms,
    })
}

fn canonical_json(
    setting: DiskPressurePolicySetting,
    kind: HistoryErrorKind,
) -> Result<String, HistoryError> {
    let critical = setting.config.critical_threshold();
    let warning = setting.config.warning_threshold();
    let recovery = setting.config.recovery_margin();
    let value = DiskPressurePolicyValueV1 {
        revision: setting.revision,
        source: match setting.source {
            DiskPressurePolicySettingSource::Default => StoredPolicySource::Default,
            DiskPressurePolicySettingSource::Stored => StoredPolicySource::Stored,
        },
        critical_available_bytes: critical.maximum_available_bytes(),
        critical_available_basis_points: critical.maximum_available_basis_points(),
        warning_available_bytes: warning.maximum_available_bytes(),
        warning_available_basis_points: warning.maximum_available_basis_points(),
        recovery_bytes: recovery.bytes(),
        recovery_basis_points: recovery.basis_points(),
    };
    let json = serde_json::to_string(&value).map_err(|_| HistoryError::new(kind))?;
    if json.len() > MAX_CANONICAL_VALUE_BYTES {
        return Err(HistoryError::new(kind));
    }
    Ok(json)
}

fn optional_stored_exact(
    left: Option<&StoredDiskPressurePolicy>,
    right: Option<&StoredDiskPressurePolicy>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => stored_exact(left, right),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn stored_exact(left: &StoredDiskPressurePolicy, right: &StoredDiskPressurePolicy) -> bool {
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

    fn custom_config() -> DiskPressureConfig {
        DiskPressureConfig::new(
            DiskPressureThreshold::new(8 * 1024 * 1024 * 1024, 450).unwrap(),
            DiskPressureThreshold::new(24 * 1024 * 1024 * 1024, 900).unwrap(),
            DiskPressureRecoveryMargin::new(3 * 1024 * 1024 * 1024, 125).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn missing_row_is_default_revision_zero_without_a_write() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        assert_eq!(
            store.load_disk_pressure_policy().unwrap(),
            DiskPressurePolicySetting::DEFAULT
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM settings WHERE setting_key = ?1",
                        [DISK_PRESSURE_POLICY_KEY],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                0
            );
        });
    }

    #[test]
    fn explicit_default_custom_reset_and_reopen_preserve_provenance_and_revision() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let start = UNIX_EPOCH + Duration::from_millis(1_750_000_000_000);
        let store = StoreCoordinator::open(&database).unwrap();

        let explicit_default = store
            .set_disk_pressure_policy_at_for_test(DiskPressureConfig::DEFAULT, start)
            .unwrap();
        assert!(explicit_default.changed);
        assert_eq!(explicit_default.settings.revision, 1);
        assert_eq!(
            explicit_default.settings.source,
            DiskPressurePolicySettingSource::Stored
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT value_json FROM settings WHERE setting_key = ?1",
                        [DISK_PRESSURE_POLICY_KEY],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap(),
                "{\"revision\":1,\"source\":\"stored\",\"critical_available_bytes\":10737418240,\"critical_available_basis_points\":500,\"warning_available_bytes\":32212254720,\"warning_available_basis_points\":1000,\"recovery_bytes\":2147483648,\"recovery_basis_points\":100}"
            );
        });
        let exact = store
            .set_disk_pressure_policy_at_for_test(
                DiskPressureConfig::DEFAULT,
                start + Duration::from_millis(1),
            )
            .unwrap();
        assert!(!exact.changed);
        assert_eq!(exact.settings, explicit_default.settings);

        let custom = store
            .set_disk_pressure_policy_after_commit_failure_for_test(
                custom_config(),
                start + Duration::from_millis(2),
            )
            .unwrap();
        assert!(custom.changed);
        assert_eq!(custom.settings.revision, 2);
        assert_eq!(custom.settings.config, custom_config());

        let reset = store
            .reset_disk_pressure_policy_after_commit_failure_for_test(
                start + Duration::from_millis(3),
            )
            .unwrap();
        assert!(reset.changed);
        assert_eq!(reset.settings.revision, 3);
        assert_eq!(reset.settings.config, DiskPressureConfig::DEFAULT);
        assert_eq!(
            reset.settings.source,
            DiskPressurePolicySettingSource::Default
        );
        let reset_retry = store
            .reset_disk_pressure_policy_at_for_test(start + Duration::from_millis(4))
            .unwrap();
        assert!(!reset_retry.changed);
        assert_eq!(reset_retry.settings, reset.settings);
        drop(store);

        assert_eq!(
            StoreCoordinator::open(&database)
                .unwrap()
                .load_disk_pressure_policy()
                .unwrap(),
            reset.settings
        );
    }

    #[test]
    fn malformed_newer_schema_and_revision_exhaustion_fail_closed() {
        let malformed = [
            "{}",
            " {\"revision\":1}",
            "{\"revision\":0,\"source\":\"default\",\"critical_available_bytes\":1,\"critical_available_basis_points\":1,\"warning_available_bytes\":2,\"warning_available_basis_points\":2,\"recovery_bytes\":1,\"recovery_basis_points\":1}",
            "{\"source\":\"stored\",\"revision\":1,\"critical_available_bytes\":1,\"critical_available_basis_points\":1,\"warning_available_bytes\":2,\"warning_available_basis_points\":2,\"recovery_bytes\":1,\"recovery_basis_points\":1}",
            "{\"revision\":1,\"source\":\"stored\",\"critical_available_bytes\":1,\"critical_available_basis_points\":1,\"warning_available_bytes\":2,\"warning_available_basis_points\":2,\"recovery_bytes\":1,\"recovery_basis_points\":1,\"unknown\":1}",
            "{\"revision\":1,\"source\":\"stored\",\"critical_available_bytes\":1.0,\"critical_available_basis_points\":1,\"warning_available_bytes\":2,\"warning_available_basis_points\":2,\"recovery_bytes\":1,\"recovery_basis_points\":1}",
            "{\"revision\":1,\"source\":\"stored\",\"critical_available_bytes\":\"1\",\"critical_available_basis_points\":1,\"warning_available_bytes\":2,\"warning_available_basis_points\":2,\"recovery_bytes\":1,\"recovery_basis_points\":1}",
            "{\"revision\":1,\"source\":\"stored\",\"critical_available_bytes\":-1,\"critical_available_basis_points\":1,\"warning_available_bytes\":2,\"warning_available_basis_points\":2,\"recovery_bytes\":1,\"recovery_basis_points\":1}",
        ];
        for value in malformed {
            let temp = TempDir::new().unwrap();
            let store = open(&temp);
            store.with_connection(|connection| {
                connection
                    .execute(
                        "INSERT INTO settings (
                             setting_key, value_json, value_schema_version,
                             updated_at_unix_ms
                         ) VALUES (?1, ?2, 1, 1)",
                        params![DISK_PRESSURE_POLICY_KEY, value],
                    )
                    .unwrap();
            });
            assert_eq!(
                store.load_disk_pressure_policy().unwrap_err().kind,
                HistoryErrorKind::CorruptData
            );
        }

        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let oversized = format!("\"{}\"", "x".repeat(MAX_CANONICAL_VALUE_BYTES));
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version,
                         updated_at_unix_ms
                     ) VALUES (?1, ?2, 1, 1)",
                    params![DISK_PRESSURE_POLICY_KEY, oversized],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_disk_pressure_policy().unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );

        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let false_default = DiskPressurePolicySetting {
            config: custom_config(),
            source: DiskPressurePolicySettingSource::Default,
            revision: 1,
            updated_at: Some(UNIX_EPOCH + Duration::from_millis(1)),
        };
        let json = canonical_json(false_default, HistoryErrorKind::InvalidInput).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version,
                         updated_at_unix_ms
                     ) VALUES (?1, ?2, 1, 1)",
                    params![DISK_PRESSURE_POLICY_KEY, json],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_disk_pressure_policy().unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );

        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version,
                         updated_at_unix_ms
                     ) VALUES (?1, '{}', 2, 1)",
                    [DISK_PRESSURE_POLICY_KEY],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_disk_pressure_policy().unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
        assert_eq!(
            store
                .set_disk_pressure_policy(DiskPressureConfig::DEFAULT)
                .unwrap_err()
                .kind,
            HistoryErrorKind::IncompatibleSchema
        );
        assert_eq!(
            store.reset_disk_pressure_policy().unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT value_json, value_schema_version FROM settings
                         WHERE setting_key = ?1",
                        [DISK_PRESSURE_POLICY_KEY],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .unwrap(),
                ("{}".to_owned(), 2)
            );
        });

        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let exhausted = DiskPressurePolicySetting {
            config: custom_config(),
            source: DiskPressurePolicySettingSource::Stored,
            revision: MAX_POLICY_REVISION,
            updated_at: Some(UNIX_EPOCH + Duration::from_millis(1)),
        };
        let json = canonical_json(exhausted, HistoryErrorKind::InvalidInput).unwrap();
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version,
                         updated_at_unix_ms
                     ) VALUES (?1, ?2, 1, 1)",
                    params![DISK_PRESSURE_POLICY_KEY, json],
                )
                .unwrap();
        });
        assert_eq!(
            store
                .set_disk_pressure_policy(DiskPressureConfig::DEFAULT)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
    }
}
