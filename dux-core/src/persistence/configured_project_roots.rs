//! Durable, discovery-only configured project roots.
//!
//! These roots are scan hints only. They do not grant read access, select a
//! cleanup target, bypass protected-root policy, or carry effect authority.

use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(windows)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::store::{HistoryConnectionGuard, StoreCoordinator};

pub(crate) const CONFIGURED_PROJECT_ROOTS_KEY: &str = "configured_project_roots";
const VALUE_SCHEMA_VERSION: i64 = 1;
const MAX_PROJECT_ROOTS: usize = 16;
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_CANONICAL_VALUE_BYTES: usize = (MAX_PROJECT_ROOTS * MAX_PATH_BYTES * 2) + (4 * 1024);
const MAX_REGISTRY_REVISION: u64 = i64::MAX as u64;
#[cfg(not(windows))]
const UNIX_PATH_BYTES_ENCODING: i64 = 1;
#[cfg(windows)]
const WINDOWS_UTF16_LE_ENCODING: i64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfiguredProjectRootSettingSource {
    Default,
    Stored,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfiguredProjectRootSetting {
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) source: ConfiguredProjectRootSettingSource,
    pub(crate) revision: u64,
    pub(crate) updated_at: Option<SystemTime>,
}

impl ConfiguredProjectRootSetting {
    pub(crate) fn default_value() -> Self {
        Self {
            roots: Vec::new(),
            source: ConfiguredProjectRootSettingSource::Default,
            revision: 0,
            updated_at: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfiguredProjectRootSettingUpdate {
    pub(crate) settings: ConfiguredProjectRootSetting,
    pub(crate) changed: bool,
}

struct StoredConfiguredProjectRoots {
    setting: ConfiguredProjectRootSetting,
    canonical_json: String,
    updated_at_unix_ms: i64,
}

struct RawConfiguredProjectRoots {
    canonical_json: String,
    value_schema_version: i64,
    updated_at_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ConfiguredProjectRootsValueV1 {
    revision: u64,
    roots: Vec<StoredProjectRoot>,
    source: StoredRegistrySource,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StoredProjectRoot {
    encoding: i64,
    path_hex: String,
}

struct EncodedProjectRoot {
    encoding: i64,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StoredRegistrySource {
    Default,
    Stored,
}

impl StoreCoordinator {
    pub(crate) fn load_configured_project_roots(
        &self,
    ) -> Result<ConfiguredProjectRootSetting, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_configured_project_roots(&guard.connection)
    }

    pub(crate) fn set_configured_project_roots(
        &self,
        roots: Vec<PathBuf>,
    ) -> Result<ConfiguredProjectRootSettingUpdate, HistoryError> {
        self.set_configured_project_roots_with_hook(roots, SystemTime::now(), || Ok(()))
    }

    fn set_configured_project_roots_with_hook(
        &self,
        roots: Vec<PathBuf>,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<ConfiguredProjectRootSettingUpdate, HistoryError> {
        let roots = normalize_roots(roots)?;
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        let mut guard = self.lock_current_history_connection()?;
        let original = load_stored_configured_project_roots(&guard.connection)?;
        if let Some(stored) = original
            .as_ref()
            .filter(|stored| stored.setting.roots == roots)
        {
            return Ok(ConfiguredProjectRootSettingUpdate {
                settings: stored.setting.clone(),
                changed: false,
            });
        }
        let revision = original.as_ref().map_or(Ok(1), |stored| {
            stored
                .setting
                .revision
                .checked_add(1)
                .filter(|revision| *revision <= MAX_REGISTRY_REVISION)
                .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))
        })?;
        let updated_at_unix_ms = original.as_ref().map_or(observed_at_unix_ms, |stored| {
            observed_at_unix_ms.max(stored.updated_at_unix_ms)
        });
        let expected = ConfiguredProjectRootSetting {
            roots,
            source: ConfiguredProjectRootSettingSource::Stored,
            revision,
            updated_at: Some(unix_ms_to_system_time(updated_at_unix_ms)?),
        };
        let canonical_json = canonical_json(&expected, HistoryErrorKind::InvalidInput)?;
        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let transactional_original = load_stored_configured_project_roots(&transaction)?;
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
                return Ok(ConfiguredProjectRootSettingUpdate {
                    settings: expected,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        reconcile_set_write(self, &guard, original.as_ref(), expected, failure)
    }

    pub(crate) fn reset_configured_project_roots(
        &self,
    ) -> Result<ConfiguredProjectRootSettingUpdate, HistoryError> {
        self.reset_configured_project_roots_with_hook(|| Ok(()))
    }

    fn reset_configured_project_roots_with_hook(
        &self,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<ConfiguredProjectRootSettingUpdate, HistoryError> {
        let mut guard = self.lock_current_history_connection()?;
        let Some(original) = load_stored_configured_project_roots(&guard.connection)? else {
            return Ok(ConfiguredProjectRootSettingUpdate {
                settings: ConfiguredProjectRootSetting::default_value(),
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
                    CONFIGURED_PROJECT_ROOTS_KEY,
                    original.canonical_json,
                    VALUE_SCHEMA_VERSION,
                    original.updated_at_unix_ms,
                ],
            )
            .map_err(map_write_sql_error)?;
        if changed != 1 {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let expected = ConfiguredProjectRootSetting::default_value();
        let failure = match transaction
            .commit()
            .map_err(map_write_sql_error)
            .and_then(|()| after_commit())
            .and_then(|()| self.revalidate_current_history_guard(&guard))
        {
            Ok(()) => {
                return Ok(ConfiguredProjectRootSettingUpdate {
                    settings: expected,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        reconcile_reset_write(self, &guard, &original, expected, failure)
    }

    #[cfg(test)]
    fn set_configured_project_roots_at_for_test(
        &self,
        roots: Vec<PathBuf>,
        observed_at: SystemTime,
    ) -> Result<ConfiguredProjectRootSettingUpdate, HistoryError> {
        self.set_configured_project_roots_with_hook(roots, observed_at, || Ok(()))
    }

    #[cfg(test)]
    fn set_configured_project_roots_after_commit_failure_for_test(
        &self,
        roots: Vec<PathBuf>,
        observed_at: SystemTime,
    ) -> Result<ConfiguredProjectRootSettingUpdate, HistoryError> {
        self.set_configured_project_roots_with_hook(roots, observed_at, || {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }

    #[cfg(test)]
    fn reset_configured_project_roots_after_commit_failure_for_test(
        &self,
    ) -> Result<ConfiguredProjectRootSettingUpdate, HistoryError> {
        self.reset_configured_project_roots_with_hook(|| {
            Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable))
        })
    }
}

pub(crate) fn load_configured_project_roots(
    connection: &Connection,
) -> Result<ConfiguredProjectRootSetting, HistoryError> {
    Ok(load_stored_configured_project_roots(connection)?
        .map_or_else(ConfiguredProjectRootSetting::default_value, |stored| {
            stored.setting
        }))
}

pub(crate) fn validate_configured_project_roots(roots: &[PathBuf]) -> Result<(), HistoryError> {
    normalize_roots(roots.to_vec()).map(|_| ())
}

fn reconcile_set_write(
    coordinator: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    original: Option<&StoredConfiguredProjectRoots>,
    expected: ConfiguredProjectRootSetting,
    failure: HistoryError,
) -> Result<ConfiguredProjectRootSettingUpdate, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored_configured_project_roots(&guard.connection) {
        Ok(Some(current)) if current.setting == expected => {
            Ok(ConfiguredProjectRootSettingUpdate {
                settings: expected,
                changed: true,
            })
        }
        Ok(current) if optional_stored_exact(current.as_ref(), original) => Err(failure),
        Ok(_) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn reconcile_reset_write(
    coordinator: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    original: &StoredConfiguredProjectRoots,
    expected: ConfiguredProjectRootSetting,
    failure: HistoryError,
) -> Result<ConfiguredProjectRootSettingUpdate, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored_configured_project_roots(&guard.connection) {
        Ok(None) => Ok(ConfiguredProjectRootSettingUpdate {
            settings: expected,
            changed: true,
        }),
        Ok(Some(current)) if stored_exact(&current, original) => Err(failure),
        Ok(Some(_)) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn normalize_roots(roots: Vec<PathBuf>) -> Result<Vec<PathBuf>, HistoryError> {
    if roots.len() > MAX_PROJECT_ROOTS {
        return Err(invalid_input());
    }
    let mut encoded = roots
        .into_iter()
        .map(|root| {
            let encoded = validate_project_root(&root)?;
            Ok((encoded.encoding, encoded.bytes, root))
        })
        .collect::<Result<Vec<_>, HistoryError>>()?;
    encoded.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    if encoded
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1)
    {
        return Err(invalid_input());
    }
    let roots = encoded
        .into_iter()
        .map(|(_, _, root)| root)
        .collect::<Vec<_>>();
    if roots.iter().enumerate().any(|(index, root)| {
        roots
            .iter()
            .skip(index + 1)
            .any(|other| root.starts_with(other) || other.starts_with(root))
    }) {
        return Err(invalid_input());
    }
    Ok(roots)
}

fn validate_project_root(path: &Path) -> Result<EncodedProjectRoot, HistoryError> {
    let normalized: PathBuf = path.components().collect();
    if !path.is_absolute()
        || path.parent().is_none()
        || normalized.as_os_str() != path.as_os_str()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || contains_control_component(path)
        || has_unsupported_project_root_prefix(path)
    {
        return Err(invalid_input());
    }
    let encoded = encode_project_root_path(path)?;
    if encoded.bytes.len() > MAX_PATH_BYTES {
        return Err(invalid_input());
    }
    Ok(encoded)
}

#[cfg(unix)]
fn encode_project_root_path(path: &Path) -> Result<EncodedProjectRoot, HistoryError> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES {
        return Err(invalid_input());
    }
    Ok(EncodedProjectRoot {
        encoding: UNIX_PATH_BYTES_ENCODING,
        bytes: bytes.to_vec(),
    })
}

#[cfg(windows)]
fn encode_project_root_path(path: &Path) -> Result<EncodedProjectRoot, HistoryError> {
    let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if units.is_empty() || units.len().saturating_mul(2) > MAX_PATH_BYTES {
        return Err(invalid_input());
    }
    let mut bytes = Vec::with_capacity(units.len().saturating_mul(2));
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(EncodedProjectRoot {
        encoding: WINDOWS_UTF16_LE_ENCODING,
        bytes,
    })
}

#[cfg(not(any(unix, windows)))]
fn encode_project_root_path(path: &Path) -> Result<EncodedProjectRoot, HistoryError> {
    let bytes = path.to_str().ok_or_else(invalid_input)?.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PATH_BYTES {
        return Err(invalid_input());
    }
    Ok(EncodedProjectRoot {
        encoding: UNIX_PATH_BYTES_ENCODING,
        bytes: bytes.to_vec(),
    })
}

#[cfg(unix)]
fn contains_control_component(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    path.components().any(|component| match component {
        Component::Normal(value) => value.as_bytes().iter().any(|byte| byte.is_ascii_control()),
        Component::Prefix(_) | Component::RootDir | Component::CurDir | Component::ParentDir => {
            false
        }
    })
}

#[cfg(windows)]
fn contains_control_component(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str()
        .encode_wide()
        .any(|unit| unit <= 0x1f || unit == 0x7f)
}

#[cfg(not(any(unix, windows)))]
fn contains_control_component(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(value) => value.to_string_lossy().chars().any(char::is_control),
        Component::Prefix(_) | Component::RootDir | Component::CurDir | Component::ParentDir => {
            false
        }
    })
}

#[cfg(windows)]
fn has_unsupported_project_root_prefix(path: &Path) -> bool {
    use std::path::Prefix;

    !matches!(
        path.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::UNC(_, _))
    )
}

#[cfg(not(windows))]
const fn has_unsupported_project_root_prefix(_: &Path) -> bool {
    false
}

fn write_setting_row(
    transaction: &Transaction<'_>,
    original: Option<&StoredConfiguredProjectRoots>,
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
                    CONFIGURED_PROJECT_ROOTS_KEY,
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
                    CONFIGURED_PROJECT_ROOTS_KEY,
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

fn load_stored_configured_project_roots(
    connection: &Connection,
) -> Result<Option<StoredConfiguredProjectRoots>, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(
                "SELECT value_json, value_schema_version, updated_at_unix_ms
                 FROM settings WHERE setting_key = ?1",
                [CONFIGURED_PROJECT_ROOTS_KEY],
                raw_configured_project_roots,
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
        let value: ConfiguredProjectRootsValueV1 =
            serde_json::from_str(&raw.canonical_json).map_err(|_| corrupt())?;
        let roots = value
            .roots
            .iter()
            .map(decode_stored_root)
            .collect::<Result<Vec<_>, _>>()?;
        let setting = ConfiguredProjectRootSetting {
            roots,
            source: match value.source {
                StoredRegistrySource::Default => ConfiguredProjectRootSettingSource::Default,
                StoredRegistrySource::Stored => ConfiguredProjectRootSettingSource::Stored,
            },
            revision: value.revision,
            updated_at: Some(unix_ms_to_system_time(raw.updated_at_unix_ms)?),
        };
        if setting.source == ConfiguredProjectRootSettingSource::Default
            || setting.revision == 0
            || setting.revision > MAX_REGISTRY_REVISION
            || !is_canonical_roots(&setting.roots)
            || canonical_json(&setting, HistoryErrorKind::CorruptData)? != raw.canonical_json
        {
            return Err(corrupt());
        }
        Ok(Some(StoredConfiguredProjectRoots {
            setting,
            canonical_json: raw.canonical_json,
            updated_at_unix_ms: raw.updated_at_unix_ms,
        }))
    })
}

fn is_canonical_roots(roots: &[PathBuf]) -> bool {
    normalize_roots(roots.to_vec()).is_ok_and(|normalized| normalized.as_slice() == roots)
}

fn decode_stored_root(value: &StoredProjectRoot) -> Result<PathBuf, HistoryError> {
    let bytes = decode_hex(&value.path_hex)?;
    let path = decode_project_root_path(value.encoding, bytes)?;
    validate_project_root(&path).map_err(|_| corrupt())?;
    Ok(path)
}

#[cfg(unix)]
fn decode_project_root_path(encoding: i64, bytes: Vec<u8>) -> Result<PathBuf, HistoryError> {
    if encoding != UNIX_PATH_BYTES_ENCODING || bytes.is_empty() || bytes.len() > MAX_PATH_BYTES {
        return Err(corrupt());
    }
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(windows)]
fn decode_project_root_path(encoding: i64, bytes: Vec<u8>) -> Result<PathBuf, HistoryError> {
    if encoding != WINDOWS_UTF16_LE_ENCODING
        || bytes.is_empty()
        || bytes.len() > MAX_PATH_BYTES
        || !bytes.len().is_multiple_of(2)
    {
        return Err(corrupt());
    }
    let units = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    Ok(PathBuf::from(OsString::from_wide(&units)))
}

#[cfg(not(any(unix, windows)))]
fn decode_project_root_path(encoding: i64, bytes: Vec<u8>) -> Result<PathBuf, HistoryError> {
    if encoding != UNIX_PATH_BYTES_ENCODING || bytes.is_empty() || bytes.len() > MAX_PATH_BYTES {
        return Err(corrupt());
    }
    let text = String::from_utf8(bytes).map_err(|_| corrupt())?;
    Ok(PathBuf::from(text))
}

fn raw_configured_project_roots(row: &Row<'_>) -> rusqlite::Result<RawConfiguredProjectRoots> {
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
    Ok(RawConfiguredProjectRoots {
        canonical_json,
        value_schema_version,
        updated_at_unix_ms,
    })
}

fn canonical_json(
    setting: &ConfiguredProjectRootSetting,
    kind: HistoryErrorKind,
) -> Result<String, HistoryError> {
    let roots = setting
        .roots
        .iter()
        .map(|path| {
            let encoded = validate_project_root(path).map_err(|_| HistoryError::new(kind))?;
            Ok(StoredProjectRoot {
                encoding: encoded.encoding,
                path_hex: hex_lower(&encoded.bytes),
            })
        })
        .collect::<Result<Vec<_>, HistoryError>>()?;
    let value = ConfiguredProjectRootsValueV1 {
        revision: setting.revision,
        roots,
        source: StoredRegistrySource::Stored,
    };
    let json = serde_json::to_string(&value).map_err(|_| HistoryError::new(kind))?;
    if json.len() > MAX_CANONICAL_VALUE_BYTES {
        return Err(HistoryError::new(kind));
    }
    Ok(json)
}

fn optional_stored_exact(
    left: Option<&StoredConfiguredProjectRoots>,
    right: Option<&StoredConfiguredProjectRoots>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => stored_exact(left, right),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn stored_exact(left: &StoredConfiguredProjectRoots, right: &StoredConfiguredProjectRoots) -> bool {
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

const fn invalid_input() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidInput)
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;

    fn open(temp: &TempDir) -> Arc<StoreCoordinator> {
        StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap()
    }

    #[test]
    fn missing_row_is_empty_default_and_does_not_write() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        assert_eq!(
            store.load_configured_project_roots().unwrap(),
            ConfiguredProjectRootSetting::default_value()
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM settings WHERE setting_key = ?1",
                        [CONFIGURED_PROJECT_ROOTS_KEY],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                0
            );
        });
    }

    #[test]
    fn replacement_is_sorted_revisioned_exact_noop_and_reset_deletes() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let later = temp.path().join("z-project");
        let earlier = temp.path().join("a-project");
        let observed = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let first = store
            .set_configured_project_roots_at_for_test(
                vec![later.clone(), earlier.clone()],
                observed,
            )
            .unwrap();
        assert!(first.changed);
        assert_eq!(first.settings.roots, vec![earlier.clone(), later.clone()]);
        assert_eq!(
            first.settings.source,
            ConfiguredProjectRootSettingSource::Stored
        );
        assert_eq!(first.settings.revision, 1);

        let retry = store
            .set_configured_project_roots(vec![later, earlier])
            .unwrap();
        assert!(!retry.changed);
        assert_eq!(retry.settings, first.settings);

        let empty = store.set_configured_project_roots(Vec::new()).unwrap();
        assert!(empty.changed);
        assert_eq!(
            empty.settings.source,
            ConfiguredProjectRootSettingSource::Stored
        );
        assert_eq!(empty.settings.revision, 2);
        assert!(empty.settings.roots.is_empty());

        let reset = store.reset_configured_project_roots().unwrap();
        assert!(reset.changed);
        assert_eq!(
            reset.settings,
            ConfiguredProjectRootSetting::default_value()
        );
        store.with_connection(|connection| {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM settings WHERE setting_key = ?1",
                        [CONFIGURED_PROJECT_ROOTS_KEY],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                0
            );
        });
        assert!(!store.reset_configured_project_roots().unwrap().changed);
    }

    #[test]
    fn rejects_non_normal_control_duplicate_overlapping_and_oversized_sets() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let root = temp.path().join("project");
        for roots in [
            vec![PathBuf::from("relative")],
            vec![PathBuf::from("/")],
            vec![PathBuf::from("/tmp/../unsafe")],
            vec![PathBuf::from("/tmp/./ambiguous")],
            vec![PathBuf::from("/tmp/control\nname")],
            vec![root.clone(), root.clone()],
            vec![root.clone(), root.join("nested")],
        ] {
            assert_eq!(
                store.set_configured_project_roots(roots).unwrap_err().kind,
                HistoryErrorKind::InvalidInput
            );
        }
        assert_eq!(
            store
                .set_configured_project_roots(
                    (0..=MAX_PROJECT_ROOTS)
                        .map(|index| temp.path().join(format!("project-{index}")))
                        .collect(),
                )
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidInput
        );
    }

    #[test]
    fn post_commit_failures_reconcile_set_and_reset() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let root = temp.path().join("project");
        let set = store
            .set_configured_project_roots_after_commit_failure_for_test(
                vec![root],
                SystemTime::UNIX_EPOCH + Duration::from_secs(20),
            )
            .unwrap();
        assert!(set.changed);
        assert_eq!(set.settings.revision, 1);
        let reset = store
            .reset_configured_project_roots_after_commit_failure_for_test()
            .unwrap();
        assert!(reset.changed);
        assert_eq!(
            reset.settings,
            ConfiguredProjectRootSetting::default_value()
        );
    }

    #[test]
    fn malformed_and_newer_rows_are_not_accepted() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        store.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version, updated_at_unix_ms
                     ) VALUES (?1, ?2, 2, 1)",
                    params![
                        CONFIGURED_PROJECT_ROOTS_KEY,
                        "{\"revision\":1,\"roots\":[],\"source\":\"stored\"}"
                    ],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_configured_project_roots().unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
        store.with_connection(|connection| {
            connection
                .execute(
                    "UPDATE settings
                     SET value_json = ?1, value_schema_version = 1
                     WHERE setting_key = ?2",
                    params![
                        "{\"roots\":[],\"revision\":1,\"source\":\"stored\"}",
                        CONFIGURED_PROJECT_ROOTS_KEY
                    ],
                )
                .unwrap();
        });
        assert_eq!(
            store.load_configured_project_roots().unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn paths_round_trip_losslessly_in_native_byte_order() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let first = temp.path().join("project-å");
        let second = temp.path().join("project-ö");
        let update = store
            .set_configured_project_roots(vec![second.clone(), first.clone()])
            .unwrap();
        assert_eq!(update.settings.roots, vec![first, second]);
        assert_eq!(
            store.load_configured_project_roots().unwrap(),
            update.settings
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_unix_path_round_trips_losslessly() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let temp = TempDir::new().unwrap();
        let store = open(&temp);
        let mut bytes = temp.path().as_os_str().as_bytes().to_vec();
        bytes.extend_from_slice(b"/project-\xff");
        let root = PathBuf::from(OsString::from_vec(bytes.clone()));

        let update = store
            .set_configured_project_roots(vec![root.clone()])
            .unwrap();

        assert_eq!(update.settings.roots, vec![root]);
        assert_eq!(
            store.load_configured_project_roots().unwrap().roots[0]
                .as_os_str()
                .as_bytes(),
            bytes
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_roots_reject_control_and_device_or_verbatim_prefixes() {
        let temp = TempDir::new().unwrap();
        let store = open(&temp);

        for path in [
            PathBuf::from("C:\\project-\u{001f}"),
            PathBuf::from("\\\\?\\C:\\project"),
            PathBuf::from("\\\\.\\C:\\project"),
            PathBuf::from("\\\\?\\UNC\\server\\share\\project"),
        ] {
            assert_eq!(
                store
                    .set_configured_project_roots(vec![path])
                    .unwrap_err()
                    .kind,
                HistoryErrorKind::InvalidInput
            );
        }
    }
}
