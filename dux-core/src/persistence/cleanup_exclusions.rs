//! Typed, deny-only user cleanup exclusions.
//!
//! Exclusions are exact lexical path prefixes stored losslessly behind one
//! versioned settings key. They can only suppress a cleanup effect; they never
//! grant access, bypass protected-root policy, or turn a path into a plan.
//! Writes take the store-wide cleanup exclusion so a changed exclusion cannot
//! race the journal's final pre-effect check.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::store::{HistoryConnectionGuard, StoreCoordinator};

pub(crate) const CLEANUP_EXCLUSIONS_KEY: &str = "cleanup_exclusions";
const VALUE_SCHEMA_VERSION: i64 = 1;
const MAX_EXCLUSIONS: usize = 64;
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_CANONICAL_VALUE_BYTES: usize = 128 * 1024;
const MAX_POLICY_REVISION: u64 = i64::MAX as u64;
const CLEANUP_POLICY_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CleanupExclusionSettingSource {
    Default,
    Stored,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CleanupExclusionSetting {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) source: CleanupExclusionSettingSource,
    pub(crate) revision: u64,
    pub(crate) updated_at: Option<SystemTime>,
}

impl CleanupExclusionSetting {
    pub(crate) fn default_value() -> Self {
        Self {
            paths: Vec::new(),
            source: CleanupExclusionSettingSource::Default,
            revision: 0,
            updated_at: None,
        }
    }

    /// An exclusion is a deny-only lexical prefix. No canonicalization or
    /// symlink traversal occurs here; the planner and executor retain their
    /// independent live identity checks.
    pub(crate) fn contains_path(&self, path: &Path) -> bool {
        self.paths
            .iter()
            .any(|excluded| path == excluded || path.starts_with(excluded))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CleanupExclusionSettingUpdate {
    pub(crate) settings: CleanupExclusionSetting,
    pub(crate) changed: bool,
}

struct StoredCleanupExclusions {
    setting: CleanupExclusionSetting,
    canonical_json: String,
    updated_at_unix_ms: i64,
}

struct RawCleanupExclusions {
    canonical_json: String,
    value_schema_version: i64,
    updated_at_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CleanupExclusionValueV1 {
    paths: Vec<StoredExclusionPath>,
    revision: u64,
    source: StoredPolicySource,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StoredExclusionPath {
    encoding: i64,
    path_hex: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StoredPolicySource {
    Default,
    Stored,
}

impl StoreCoordinator {
    pub(crate) fn load_cleanup_exclusions(&self) -> Result<CleanupExclusionSetting, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_cleanup_exclusions(&guard.connection)
    }

    pub(crate) fn set_cleanup_exclusions(
        &self,
        paths: Vec<PathBuf>,
    ) -> Result<CleanupExclusionSettingUpdate, HistoryError> {
        let observed_at = SystemTime::now();
        let normalized = normalize_paths(paths)?;
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        let cleanup_guard = self.acquire_cleanup_lock_for_journal(CLEANUP_POLICY_LOCK_TIMEOUT)?;
        self.validate_cleanup_lock_for_journal(&cleanup_guard)?;
        let mut guard = self.lock_current_history_connection()?;
        self.validate_cleanup_lock_for_journal(&cleanup_guard)?;
        let original = load_stored_cleanup_exclusions(&guard.connection)?;
        let current = original
            .as_ref()
            .map_or_else(CleanupExclusionSetting::default_value, |stored| {
                stored.setting.clone()
            });
        if current.paths == normalized {
            return Ok(CleanupExclusionSettingUpdate {
                settings: current,
                changed: false,
            });
        }
        let revision = current
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_POLICY_REVISION)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
        let updated_at_unix_ms = original.as_ref().map_or(observed_at_unix_ms, |stored| {
            observed_at_unix_ms.max(stored.updated_at_unix_ms)
        });
        let expected = CleanupExclusionSetting {
            paths: normalized,
            source: CleanupExclusionSettingSource::Stored,
            revision,
            updated_at: Some(unix_ms_to_system_time(updated_at_unix_ms)?),
        };
        let canonical_json = canonical_json(&expected, HistoryErrorKind::InvalidInput)?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let transactional_original = load_stored_cleanup_exclusions(&transaction)?;
        if !optional_stored_exact(transactional_original.as_ref(), original.as_ref()) {
            return Err(HistoryError::new(HistoryErrorKind::InternalState));
        }
        write_setting_row(
            &transaction,
            original.as_ref(),
            &canonical_json,
            updated_at_unix_ms,
        )?;
        let failure = transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| self.revalidate_current_history_guard(&guard));
        match failure {
            Ok(()) => Ok(CleanupExclusionSettingUpdate {
                settings: expected,
                changed: true,
            }),
            Err(failure) => reconcile_set_write(self, &guard, original.as_ref(), expected, failure),
        }
    }

    pub(crate) fn reset_cleanup_exclusions(
        &self,
    ) -> Result<CleanupExclusionSettingUpdate, HistoryError> {
        let cleanup_guard = self.acquire_cleanup_lock_for_journal(CLEANUP_POLICY_LOCK_TIMEOUT)?;
        self.validate_cleanup_lock_for_journal(&cleanup_guard)?;
        let mut guard = self.lock_current_history_connection()?;
        self.validate_cleanup_lock_for_journal(&cleanup_guard)?;
        let Some(original) = load_stored_cleanup_exclusions(&guard.connection)? else {
            return Ok(CleanupExclusionSettingUpdate {
                settings: CleanupExclusionSetting::default_value(),
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
                    CLEANUP_EXCLUSIONS_KEY,
                    original.canonical_json,
                    VALUE_SCHEMA_VERSION,
                    original.updated_at_unix_ms,
                ],
            )
            .map_err(map_write_sql_error)?;
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let expected = CleanupExclusionSetting::default_value();
        let failure = transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| self.revalidate_current_history_guard(&guard));
        match failure {
            Ok(()) => Ok(CleanupExclusionSettingUpdate {
                settings: expected,
                changed: true,
            }),
            Err(failure) => reconcile_reset_write(self, &guard, &original, expected, failure),
        }
    }
}

pub(crate) fn load_cleanup_exclusions(
    connection: &Connection,
) -> Result<CleanupExclusionSetting, HistoryError> {
    Ok(load_stored_cleanup_exclusions(connection)?
        .map_or_else(CleanupExclusionSetting::default_value, |stored| {
            stored.setting
        }))
}

fn reconcile_set_write(
    coordinator: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    original: Option<&StoredCleanupExclusions>,
    expected: CleanupExclusionSetting,
    failure: HistoryError,
) -> Result<CleanupExclusionSettingUpdate, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored_cleanup_exclusions(&guard.connection) {
        Ok(Some(current)) if current.setting == expected => Ok(CleanupExclusionSettingUpdate {
            settings: expected,
            changed: true,
        }),
        Ok(current) if optional_stored_exact(current.as_ref(), original) => Err(failure),
        Ok(_) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn reconcile_reset_write(
    coordinator: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    original: &StoredCleanupExclusions,
    expected: CleanupExclusionSetting,
    failure: HistoryError,
) -> Result<CleanupExclusionSettingUpdate, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored_cleanup_exclusions(&guard.connection) {
        Ok(None) => Ok(CleanupExclusionSettingUpdate {
            settings: expected,
            changed: true,
        }),
        Ok(Some(current)) if stored_exact(&current, original) => Err(failure),
        Ok(Some(_)) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn normalize_paths(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, HistoryError> {
    if paths.len() > MAX_EXCLUSIONS {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    let mut encoded = paths
        .into_iter()
        .map(|path| {
            let encoded = validate_exclusion_path(&path)?;
            Ok((encoded.encoding as i64, hex_lower(&encoded.bytes), path))
        })
        .collect::<Result<Vec<_>, HistoryError>>()?;
    encoded.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    if encoded
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1)
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    Ok(encoded.into_iter().map(|(_, _, path)| path).collect())
}

fn validate_exclusion_path(path: &Path) -> Result<EncodedBytes, HistoryError> {
    if !path.is_absolute()
        || path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    let encoded =
        encode_host_path(path).map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))?;
    if encoded.bytes.len() > MAX_PATH_BYTES {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    Ok(encoded)
}

fn write_setting_row(
    transaction: &Transaction<'_>,
    original: Option<&StoredCleanupExclusions>,
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
                    CLEANUP_EXCLUSIONS_KEY,
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
                    CLEANUP_EXCLUSIONS_KEY,
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

fn load_stored_cleanup_exclusions(
    connection: &Connection,
) -> Result<Option<StoredCleanupExclusions>, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(
                "SELECT value_json, value_schema_version, updated_at_unix_ms
                 FROM settings WHERE setting_key = ?1",
                [CLEANUP_EXCLUSIONS_KEY],
                raw_cleanup_exclusions,
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
        let value: CleanupExclusionValueV1 =
            serde_json::from_str(&raw.canonical_json).map_err(|_| corrupt())?;
        let paths = value
            .paths
            .iter()
            .map(decode_stored_path)
            .collect::<Result<Vec<_>, _>>()?;
        let setting = CleanupExclusionSetting {
            paths,
            source: match value.source {
                StoredPolicySource::Default => CleanupExclusionSettingSource::Default,
                StoredPolicySource::Stored => CleanupExclusionSettingSource::Stored,
            },
            revision: value.revision,
            updated_at: Some(unix_ms_to_system_time(raw.updated_at_unix_ms)?),
        };
        if setting.source == CleanupExclusionSettingSource::Default
            || setting.revision == 0
            || setting.revision > MAX_POLICY_REVISION
            || !is_canonical_paths(&setting.paths)
            || canonical_json(&setting, HistoryErrorKind::CorruptData)? != raw.canonical_json
        {
            return Err(corrupt());
        }
        Ok(Some(StoredCleanupExclusions {
            setting,
            canonical_json: raw.canonical_json,
            updated_at_unix_ms: raw.updated_at_unix_ms,
        }))
    })
}

fn is_canonical_paths(paths: &[PathBuf]) -> bool {
    if paths.len() > MAX_EXCLUSIONS {
        return false;
    }
    let mut keys = Vec::with_capacity(paths.len());
    for path in paths {
        let Ok(encoded) = validate_exclusion_path(path) else {
            return false;
        };
        keys.push((encoded.encoding as i64, hex_lower(&encoded.bytes)));
    }
    keys.windows(2).all(|pair| pair[0] < pair[1])
}

fn decode_stored_path(value: &StoredExclusionPath) -> Result<PathBuf, HistoryError> {
    let encoding = StoredEncoding::host_path_from_stored(value.encoding).map_err(|_| corrupt())?;
    let bytes = decode_hex(&value.path_hex)?;
    decode_host_path(&EncodedBytes { encoding, bytes }).map_err(|_| corrupt())
}

fn raw_cleanup_exclusions(row: &Row<'_>) -> rusqlite::Result<RawCleanupExclusions> {
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
    Ok(RawCleanupExclusions {
        canonical_json,
        value_schema_version,
        updated_at_unix_ms,
    })
}

fn canonical_json(
    setting: &CleanupExclusionSetting,
    kind: HistoryErrorKind,
) -> Result<String, HistoryError> {
    let paths = setting
        .paths
        .iter()
        .map(|path| {
            let encoded = validate_exclusion_path(path).map_err(|_| HistoryError::new(kind))?;
            Ok(StoredExclusionPath {
                encoding: encoded.encoding as i64,
                path_hex: hex_lower(&encoded.bytes),
            })
        })
        .collect::<Result<Vec<_>, HistoryError>>()?;
    let value = CleanupExclusionValueV1 {
        paths,
        revision: setting.revision,
        source: StoredPolicySource::Stored,
    };
    let json = serde_json::to_string(&value).map_err(|_| HistoryError::new(kind))?;
    if json.len() > MAX_CANONICAL_VALUE_BYTES {
        return Err(HistoryError::new(kind));
    }
    Ok(json)
}

fn optional_stored_exact(
    left: Option<&StoredCleanupExclusions>,
    right: Option<&StoredCleanupExclusions>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.setting == right.setting
                && left.canonical_json == right.canonical_json
                && left.updated_at_unix_ms == right.updated_at_unix_ms
        }
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn stored_exact(left: &StoredCleanupExclusions, right: &StoredCleanupExclusions) -> bool {
    left.setting == right.setting
        && left.canonical_json == right.canonical_json
        && left.updated_at_unix_ms == right.updated_at_unix_ms
}

fn decode_hex(value: &str) -> Result<Vec<u8>, HistoryError> {
    if value.is_empty() || !value.len().is_multiple_of(2) || !value.bytes().all(is_lower_hex) {
        return Err(corrupt());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok((hex_value(pair[0]) << 4) | hex_value(pair[1])))
        .collect()
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn is_lower_hex(value: u8) -> bool {
    value.is_ascii_digit() || (b'a'..=b'f').contains(&value)
}

fn hex_value(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => unreachable!("validated hexadecimal digit"),
    }
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;

    fn open(temp: &TempDir) -> Arc<StoreCoordinator> {
        StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap()
    }

    #[test]
    fn missing_setting_is_empty_and_does_not_write() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        assert_eq!(
            store.load_cleanup_exclusions().unwrap(),
            CleanupExclusionSetting::default_value()
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM settings WHERE setting_key = ?1",
                        [CLEANUP_EXCLUSIONS_KEY],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                0
            );
        });
    }

    #[test]
    fn set_sort_dedupe_match_prefix_and_reset() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let root = temp.path().join("root");
        let child = root.join("cache");
        let update = store
            .set_cleanup_exclusions(vec![child.clone(), root.clone()])
            .unwrap();
        assert!(update.changed);
        assert_eq!(update.settings.paths, vec![root.clone(), child]);
        assert!(update.settings.contains_path(&root.join("cache/file")));
        assert!(!update.settings.contains_path(&temp.path().join("other")));
        let retry = store
            .set_cleanup_exclusions(vec![root.clone(), root.join("cache")])
            .unwrap();
        assert!(!retry.changed);
        let reset = store.reset_cleanup_exclusions().unwrap();
        assert!(reset.changed);
        assert_eq!(reset.settings, CleanupExclusionSetting::default_value());
    }

    #[test]
    fn rejects_relative_parent_duplicate_and_malformed_values() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        for path in [PathBuf::from("relative"), PathBuf::from("/tmp/../unsafe")] {
            assert_eq!(
                store.set_cleanup_exclusions(vec![path]).unwrap_err().kind,
                HistoryErrorKind::InvalidInput
            );
        }
        let root = temp.path().join("root");
        assert_eq!(
            store
                .set_cleanup_exclusions(vec![root.clone(), root])
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidInput
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (setting_key, value_json, value_schema_version, updated_at_unix_ms)
                     VALUES (?1, '{\"paths\":[],\"revision\":1,\"source\":\"stored\"}', 2, 1)",
                    [CLEANUP_EXCLUSIONS_KEY],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_cleanup_exclusions().unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
    }
}
