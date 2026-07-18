//! Revisioned explicit trust enrollment for one direct Cargo executable.
//!
//! This row records local user intent. It is not filesystem freshness or
//! cleanup authority. Revocation remains as a revisioned tombstone so later
//! identical bytes cannot recreate an older authority state.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::codec::{EncodedBytes, StoredEncoding, decode_host_path, encode_host_path};
use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error, run_bounded_query,
    system_time_to_unix_ms, unix_ms_to_system_time,
};
use super::store::{HistoryConnectionGuard, StoreCoordinator};

pub(super) const CARGO_ENROLLMENT_KEY: &str = "developer_rust_target_cargo_enrollment";
const VALUE_SCHEMA_VERSION: i64 = 1;
const MAX_CANONICAL_VALUE_BYTES: usize = 96 * 1024;
const MAX_ENROLLMENT_REVISION: u64 = i64::MAX as u64;
const MAX_SIGNING_IDENTIFIER_BYTES: usize = 512;
const MAX_TEAM_IDENTIFIER_BYTES: usize = 128;
const MAX_CODE_DIRECTORY_HASHES: usize = 16;
const MIN_CODE_DIRECTORY_HASH_BYTES: usize = 20;
const MAX_CODE_DIRECTORY_HASH_BYTES: usize = 64;
pub(crate) const CARGO_SIGNATURE_POLICY_REVISION: u32 = 1;
pub(crate) const CARGO_CODE_SIGN_ADHOC_FLAG: u32 = 1 << 1;
pub(crate) const CARGO_ENROLLMENT_SUPPORTED_RELEASE: [u32; 3] = [1, 96, 0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CargoSignatureClass {
    AdHoc,
    Cms,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CargoCodeSignatureRecord {
    pub(crate) policy_revision: u32,
    pub(crate) class: CargoSignatureClass,
    pub(crate) flags: u32,
    pub(crate) code_directory_hashes: Vec<Vec<u8>>,
    pub(crate) signing_identifier: String,
    pub(crate) team_identifier: Option<String>,
    pub(crate) designated_requirement_sha256: Option<[u8; 32]>,
}

impl CargoCodeSignatureRecord {
    pub(crate) fn validate(&self) -> Result<(), HistoryError> {
        if self.policy_revision != CARGO_SIGNATURE_POLICY_REVISION
            || self.signing_identifier.is_empty()
            || self.signing_identifier.len() > MAX_SIGNING_IDENTIFIER_BYTES
            || self
                .team_identifier
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.len() > MAX_TEAM_IDENTIFIER_BYTES)
            || !(1..=MAX_CODE_DIRECTORY_HASHES).contains(&self.code_directory_hashes.len())
            || self.code_directory_hashes.iter().any(|hash| {
                !(MIN_CODE_DIRECTORY_HASH_BYTES..=MAX_CODE_DIRECTORY_HASH_BYTES)
                    .contains(&hash.len())
            })
            || !self
                .code_directory_hashes
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        {
            return Err(corrupt());
        }
        if self.class == CargoSignatureClass::AdHoc && self.team_identifier.is_some() {
            return Err(corrupt());
        }
        if (self.class == CargoSignatureClass::AdHoc)
            != (self.flags & CARGO_CODE_SIGN_ADHOC_FLAG != 0)
        {
            return Err(corrupt());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CargoExecutableEnrollmentIdentity {
    path: PathBuf,
    pub(crate) executable_sha256: [u8; 32],
    pub(crate) version_sha256: [u8; 32],
    pub(crate) cargo_release: [u32; 3],
    pub(crate) signature: CargoCodeSignatureRecord,
}

impl CargoExecutableEnrollmentIdentity {
    pub(crate) fn new(
        path: PathBuf,
        executable_sha256: [u8; 32],
        version_sha256: [u8; 32],
        cargo_release: [u32; 3],
        signature: CargoCodeSignatureRecord,
    ) -> Result<Self, HistoryError> {
        Self::validate_path(&path)?;
        if cargo_release != CARGO_ENROLLMENT_SUPPORTED_RELEASE {
            return Err(invalid_input());
        }
        signature.validate().map_err(|_| invalid_input())?;
        Ok(Self {
            path,
            executable_sha256,
            version_sha256,
            cargo_release,
            signature,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn validate_path(path: &Path) -> Result<(), HistoryError> {
        encode_host_path(path).map_err(|_| invalid_input())?;
        let normalized: PathBuf = path.components().collect();
        if !path.is_absolute()
            || normalized.as_os_str() != path.as_os_str()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
            || path.file_name().and_then(|value| value.to_str()) != Some("cargo")
        {
            return Err(invalid_input());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CargoEnrollmentState {
    NotEnrolled,
    Enrolled(Box<CargoExecutableEnrollmentIdentity>),
    Revoked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CargoEnrollmentSetting {
    pub(crate) revision: u64,
    pub(crate) state: CargoEnrollmentState,
    pub(crate) updated_at: Option<SystemTime>,
}

impl CargoEnrollmentSetting {
    pub(crate) const NOT_ENROLLED: Self = Self {
        revision: 0,
        state: CargoEnrollmentState::NotEnrolled,
        updated_at: None,
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CargoEnrollmentSettingUpdate {
    pub(crate) setting: CargoEnrollmentSetting,
    pub(crate) changed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StoredEnrollmentState {
    Enrolled,
    Revoked,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StoredSignatureClass {
    AdHoc,
    Cms,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CargoEnrollmentValueV1 {
    revision: u64,
    state: StoredEnrollmentState,
    path_encoding: Option<i64>,
    path_hex: Option<String>,
    executable_sha256: Option<String>,
    version_sha256: Option<String>,
    cargo_major: Option<u32>,
    cargo_minor: Option<u32>,
    cargo_patch: Option<u32>,
    signature_policy_revision: Option<u32>,
    signature_class: Option<StoredSignatureClass>,
    signature_flags: Option<u32>,
    code_directory_hashes: Option<Vec<String>>,
    signing_identifier: Option<String>,
    team_identifier: Option<String>,
    designated_requirement_sha256: Option<String>,
}

struct StoredCargoEnrollment {
    setting: CargoEnrollmentSetting,
    canonical_json: String,
    updated_at_unix_ms: i64,
}

struct RawCargoEnrollment {
    canonical_json: String,
    value_schema_version: i64,
    updated_at_unix_ms: i64,
}

enum EnrollmentMutation {
    Enroll(Box<CargoExecutableEnrollmentIdentity>),
    Revoke,
}

impl StoreCoordinator {
    pub(crate) fn load_cargo_enrollment(&self) -> Result<CargoEnrollmentSetting, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        load_cargo_enrollment(&guard.connection)
    }

    #[cfg(test)]
    pub(crate) fn enroll_cargo_executable(
        &self,
        identity: CargoExecutableEnrollmentIdentity,
    ) -> Result<CargoEnrollmentSettingUpdate, HistoryError> {
        self.mutate_cargo_enrollment(
            EnrollmentMutation::Enroll(Box::new(identity)),
            None,
            SystemTime::now(),
            || Ok(()),
        )
    }

    pub(crate) fn enroll_cargo_executable_if_current(
        &self,
        identity: CargoExecutableEnrollmentIdentity,
        expected_current: &CargoEnrollmentSetting,
    ) -> Result<CargoEnrollmentSettingUpdate, HistoryError> {
        self.mutate_cargo_enrollment(
            EnrollmentMutation::Enroll(Box::new(identity)),
            Some(expected_current),
            SystemTime::now(),
            || Ok(()),
        )
    }

    pub(crate) fn revoke_cargo_executable(
        &self,
    ) -> Result<CargoEnrollmentSettingUpdate, HistoryError> {
        self.mutate_cargo_enrollment(EnrollmentMutation::Revoke, None, SystemTime::now(), || {
            Ok(())
        })
    }

    fn mutate_cargo_enrollment(
        &self,
        mutation: EnrollmentMutation,
        expected_current: Option<&CargoEnrollmentSetting>,
        observed_at: SystemTime,
        after_commit: impl FnOnce() -> Result<(), HistoryError>,
    ) -> Result<CargoEnrollmentSettingUpdate, HistoryError> {
        let observed_at_unix_ms =
            system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
        let mut guard = self.lock_current_history_connection()?;
        let original = load_stored_cargo_enrollment(&guard.connection)?;
        let current = original
            .as_ref()
            .map_or(CargoEnrollmentSetting::NOT_ENROLLED, |stored| {
                stored.setting.clone()
            });
        if expected_current.is_some_and(|expected| expected != &current) {
            return Err(HistoryError::new(HistoryErrorKind::InvalidTransition));
        }
        let state = match mutation {
            EnrollmentMutation::Enroll(identity) => {
                if current.state == CargoEnrollmentState::Enrolled(identity.clone()) {
                    return Ok(CargoEnrollmentSettingUpdate {
                        setting: current,
                        changed: false,
                    });
                }
                CargoEnrollmentState::Enrolled(identity)
            }
            EnrollmentMutation::Revoke => {
                if matches!(
                    current.state,
                    CargoEnrollmentState::NotEnrolled | CargoEnrollmentState::Revoked
                ) {
                    return Ok(CargoEnrollmentSettingUpdate {
                        setting: current,
                        changed: false,
                    });
                }
                CargoEnrollmentState::Revoked
            }
        };
        let revision = current
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_ENROLLMENT_REVISION)
            .ok_or_else(|| HistoryError::new(HistoryErrorKind::InvalidTransition))?;
        let updated_at_unix_ms = original.as_ref().map_or(observed_at_unix_ms, |stored| {
            observed_at_unix_ms.max(stored.updated_at_unix_ms)
        });
        let expected = CargoEnrollmentSetting {
            revision,
            state,
            updated_at: Some(unix_ms_to_system_time(updated_at_unix_ms)?),
        };
        let canonical_json = canonical_json(&expected, HistoryErrorKind::InvalidInput)?;

        let transaction = guard
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_write_sql_error)?;
        let transactional_original = load_stored_cargo_enrollment(&transaction)?;
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
                return Ok(CargoEnrollmentSettingUpdate {
                    setting: expected,
                    changed: true,
                });
            }
            Err(error) => error,
        };
        reconcile_write(self, &guard, original.as_ref(), &expected, failure)
    }

    #[cfg(test)]
    pub(super) fn enroll_cargo_executable_at_for_test(
        &self,
        identity: CargoExecutableEnrollmentIdentity,
        observed_at: SystemTime,
    ) -> Result<CargoEnrollmentSettingUpdate, HistoryError> {
        self.mutate_cargo_enrollment(
            EnrollmentMutation::Enroll(Box::new(identity)),
            None,
            observed_at,
            || Ok(()),
        )
    }

    #[cfg(test)]
    pub(super) fn revoke_cargo_executable_at_for_test(
        &self,
        observed_at: SystemTime,
    ) -> Result<CargoEnrollmentSettingUpdate, HistoryError> {
        self.mutate_cargo_enrollment(EnrollmentMutation::Revoke, None, observed_at, || Ok(()))
    }

    #[cfg(test)]
    pub(super) fn enroll_cargo_executable_after_commit_failure_for_test(
        &self,
        identity: CargoExecutableEnrollmentIdentity,
        observed_at: SystemTime,
    ) -> Result<CargoEnrollmentSettingUpdate, HistoryError> {
        self.mutate_cargo_enrollment(
            EnrollmentMutation::Enroll(Box::new(identity)),
            None,
            observed_at,
            || Err(HistoryError::new(HistoryErrorKind::DatabaseUnavailable)),
        )
    }
}

fn write_setting_row(
    transaction: &Transaction<'_>,
    original: Option<&StoredCargoEnrollment>,
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
                    CARGO_ENROLLMENT_KEY,
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
                    CARGO_ENROLLMENT_KEY,
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

fn reconcile_write(
    coordinator: &StoreCoordinator,
    guard: &HistoryConnectionGuard<'_>,
    original: Option<&StoredCargoEnrollment>,
    expected: &CargoEnrollmentSetting,
    failure: HistoryError,
) -> Result<CargoEnrollmentSettingUpdate, HistoryError> {
    if coordinator.revalidate_current_history_guard(guard).is_err() {
        return Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown));
    }
    match load_stored_cargo_enrollment(&guard.connection) {
        Ok(Some(current)) if &current.setting == expected => Ok(CargoEnrollmentSettingUpdate {
            setting: expected.clone(),
            changed: true,
        }),
        Ok(current) if optional_stored_exact(current.as_ref(), original) => Err(failure),
        Ok(_) | Err(_) => Err(HistoryError::new(HistoryErrorKind::OutcomeUnknown)),
    }
}

fn load_cargo_enrollment(connection: &Connection) -> Result<CargoEnrollmentSetting, HistoryError> {
    Ok(load_stored_cargo_enrollment(connection)?
        .map_or(CargoEnrollmentSetting::NOT_ENROLLED, |stored| {
            stored.setting
        }))
}

fn load_stored_cargo_enrollment(
    connection: &Connection,
) -> Result<Option<StoredCargoEnrollment>, HistoryError> {
    run_bounded_query(connection, || {
        let raw = connection
            .query_row(
                "SELECT value_json, value_schema_version, updated_at_unix_ms
                 FROM settings WHERE setting_key = ?1",
                [CARGO_ENROLLMENT_KEY],
                raw_cargo_enrollment,
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
        let value: CargoEnrollmentValueV1 =
            serde_json::from_str(&raw.canonical_json).map_err(|_| corrupt())?;
        let setting = setting_from_value(value, raw.updated_at_unix_ms)?;
        if canonical_json(&setting, HistoryErrorKind::CorruptData)? != raw.canonical_json {
            return Err(corrupt());
        }
        Ok(Some(StoredCargoEnrollment {
            setting,
            canonical_json: raw.canonical_json,
            updated_at_unix_ms: raw.updated_at_unix_ms,
        }))
    })
}

fn setting_from_value(
    value: CargoEnrollmentValueV1,
    updated_at_unix_ms: i64,
) -> Result<CargoEnrollmentSetting, HistoryError> {
    if value.revision == 0 || value.revision > MAX_ENROLLMENT_REVISION {
        return Err(corrupt());
    }
    let state = match value.state {
        StoredEnrollmentState::Revoked => {
            if enrolled_fields_present(&value) {
                return Err(corrupt());
            }
            CargoEnrollmentState::Revoked
        }
        StoredEnrollmentState::Enrolled => {
            let encoded = EncodedBytes {
                encoding: stored_encoding(value.path_encoding.ok_or_else(corrupt)?)?,
                bytes: decode_hex(value.path_hex.as_deref().ok_or_else(corrupt)?, None)?,
            };
            let path = decode_host_path(&encoded).map_err(|_| corrupt())?;
            let executable_sha256 =
                decode_fixed_hex(value.executable_sha256.as_deref().ok_or_else(corrupt)?)?;
            let version_sha256 =
                decode_fixed_hex(value.version_sha256.as_deref().ok_or_else(corrupt)?)?;
            let cargo_release = [
                value.cargo_major.ok_or_else(corrupt)?,
                value.cargo_minor.ok_or_else(corrupt)?,
                value.cargo_patch.ok_or_else(corrupt)?,
            ];
            let code_directory_hashes = value
                .code_directory_hashes
                .as_ref()
                .ok_or_else(corrupt)?
                .iter()
                .map(|value| {
                    decode_hex(
                        value,
                        Some((MIN_CODE_DIRECTORY_HASH_BYTES, MAX_CODE_DIRECTORY_HASH_BYTES)),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let signature = CargoCodeSignatureRecord {
                policy_revision: value.signature_policy_revision.ok_or_else(corrupt)?,
                class: match value.signature_class.ok_or_else(corrupt)? {
                    StoredSignatureClass::AdHoc => CargoSignatureClass::AdHoc,
                    StoredSignatureClass::Cms => CargoSignatureClass::Cms,
                },
                flags: value.signature_flags.ok_or_else(corrupt)?,
                code_directory_hashes,
                signing_identifier: value.signing_identifier.clone().ok_or_else(corrupt)?,
                team_identifier: value.team_identifier.clone(),
                designated_requirement_sha256: value
                    .designated_requirement_sha256
                    .as_deref()
                    .map(decode_fixed_hex)
                    .transpose()?,
            };
            signature.validate()?;
            CargoEnrollmentState::Enrolled(Box::new(
                CargoExecutableEnrollmentIdentity::new(
                    path,
                    executable_sha256,
                    version_sha256,
                    cargo_release,
                    signature,
                )
                .map_err(|_| corrupt())?,
            ))
        }
    };
    Ok(CargoEnrollmentSetting {
        revision: value.revision,
        state,
        updated_at: Some(unix_ms_to_system_time(updated_at_unix_ms)?),
    })
}

fn enrolled_fields_present(value: &CargoEnrollmentValueV1) -> bool {
    value.path_encoding.is_some()
        || value.path_hex.is_some()
        || value.executable_sha256.is_some()
        || value.version_sha256.is_some()
        || value.cargo_major.is_some()
        || value.cargo_minor.is_some()
        || value.cargo_patch.is_some()
        || value.signature_policy_revision.is_some()
        || value.signature_class.is_some()
        || value.signature_flags.is_some()
        || value.code_directory_hashes.is_some()
        || value.signing_identifier.is_some()
        || value.team_identifier.is_some()
        || value.designated_requirement_sha256.is_some()
}

fn canonical_json(
    setting: &CargoEnrollmentSetting,
    kind: HistoryErrorKind,
) -> Result<String, HistoryError> {
    if setting.revision == 0 || setting.revision > MAX_ENROLLMENT_REVISION {
        return Err(HistoryError::new(kind));
    }
    let value = match &setting.state {
        CargoEnrollmentState::Enrolled(identity) => {
            enrolled_value(setting.revision, identity, kind)?
        }
        CargoEnrollmentState::Revoked => CargoEnrollmentValueV1 {
            revision: setting.revision,
            state: StoredEnrollmentState::Revoked,
            path_encoding: None,
            path_hex: None,
            executable_sha256: None,
            version_sha256: None,
            cargo_major: None,
            cargo_minor: None,
            cargo_patch: None,
            signature_policy_revision: None,
            signature_class: None,
            signature_flags: None,
            code_directory_hashes: None,
            signing_identifier: None,
            team_identifier: None,
            designated_requirement_sha256: None,
        },
        CargoEnrollmentState::NotEnrolled => return Err(HistoryError::new(kind)),
    };
    let json = serde_json::to_string(&value).map_err(|_| HistoryError::new(kind))?;
    if json.len() > MAX_CANONICAL_VALUE_BYTES {
        return Err(HistoryError::new(kind));
    }
    Ok(json)
}

fn enrolled_value(
    revision: u64,
    identity: &CargoExecutableEnrollmentIdentity,
    kind: HistoryErrorKind,
) -> Result<CargoEnrollmentValueV1, HistoryError> {
    let encoded = encode_host_path(identity.path()).map_err(|_| HistoryError::new(kind))?;
    identity
        .signature
        .validate()
        .map_err(|_| HistoryError::new(kind))?;
    Ok(CargoEnrollmentValueV1 {
        revision,
        state: StoredEnrollmentState::Enrolled,
        path_encoding: Some(encoded.encoding as i64),
        path_hex: Some(hex_lower(&encoded.bytes)),
        executable_sha256: Some(hex_lower(&identity.executable_sha256)),
        version_sha256: Some(hex_lower(&identity.version_sha256)),
        cargo_major: Some(identity.cargo_release[0]),
        cargo_minor: Some(identity.cargo_release[1]),
        cargo_patch: Some(identity.cargo_release[2]),
        signature_policy_revision: Some(identity.signature.policy_revision),
        signature_class: Some(match identity.signature.class {
            CargoSignatureClass::AdHoc => StoredSignatureClass::AdHoc,
            CargoSignatureClass::Cms => StoredSignatureClass::Cms,
        }),
        signature_flags: Some(identity.signature.flags),
        code_directory_hashes: Some(
            identity
                .signature
                .code_directory_hashes
                .iter()
                .map(|value| hex_lower(value))
                .collect(),
        ),
        signing_identifier: Some(identity.signature.signing_identifier.clone()),
        team_identifier: identity.signature.team_identifier.clone(),
        designated_requirement_sha256: identity
            .signature
            .designated_requirement_sha256
            .map(|value| hex_lower(&value)),
    })
}

fn raw_cargo_enrollment(row: &Row<'_>) -> rusqlite::Result<RawCargoEnrollment> {
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
    Ok(RawCargoEnrollment {
        canonical_json,
        value_schema_version,
        updated_at_unix_ms,
    })
}

fn stored_encoding(value: i64) -> Result<StoredEncoding, HistoryError> {
    match value {
        value if value == StoredEncoding::Utf8HostPath as i64 => Ok(StoredEncoding::Utf8HostPath),
        value if value == StoredEncoding::Utf16LeHostPath as i64 => {
            Ok(StoredEncoding::Utf16LeHostPath)
        }
        _ => Err(corrupt()),
    }
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], HistoryError> {
    decode_hex(value, Some((N, N)))?
        .try_into()
        .map_err(|_| corrupt())
}

fn decode_hex(value: &str, byte_bounds: Option<(usize, usize)>) -> Result<Vec<u8>, HistoryError> {
    if value.is_empty()
        || !value.len().is_multiple_of(2)
        || !value.bytes().all(is_lower_hex)
        || byte_bounds.is_some_and(|(minimum, maximum)| {
            let bytes = value.len() / 2;
            bytes < minimum || bytes > maximum
        })
    {
        return Err(corrupt());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok((hex_value(pair[0]) << 4) | hex_value(pair[1])))
        .collect()
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

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn optional_stored_exact(
    left: Option<&StoredCargoEnrollment>,
    right: Option<&StoredCargoEnrollment>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => stored_exact(left, right),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn stored_exact(left: &StoredCargoEnrollment, right: &StoredCargoEnrollment) -> bool {
    left.setting == right.setting
        && left.canonical_json == right.canonical_json
        && left.updated_at_unix_ms == right.updated_at_unix_ms
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
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::TempDir;

    use super::*;

    fn open(temp: &TempDir) -> Arc<StoreCoordinator> {
        StoreCoordinator::open(&temp.path().join("store/dux.sqlite3")).unwrap()
    }

    fn identity(path: &Path, seed: u8) -> CargoExecutableEnrollmentIdentity {
        CargoExecutableEnrollmentIdentity::new(
            path.to_path_buf(),
            [seed; 32],
            [seed.wrapping_add(1); 32],
            CARGO_ENROLLMENT_SUPPORTED_RELEASE,
            CargoCodeSignatureRecord {
                policy_revision: CARGO_SIGNATURE_POLICY_REVISION,
                class: CargoSignatureClass::AdHoc,
                flags: 0x2_0002,
                code_directory_hashes: vec![vec![seed; 20], vec![seed.wrapping_add(1); 20]],
                signing_identifier: format!("cargo-{seed:02x}"),
                team_identifier: None,
                designated_requirement_sha256: Some([seed.wrapping_add(2); 32]),
            },
        )
        .unwrap()
    }

    #[test]
    fn missing_enroll_revoke_reenroll_and_reopen_preserve_monotonic_intent() {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("store/dux.sqlite3");
        let cargo = temp.path().join("toolchain/bin/cargo");
        let start = UNIX_EPOCH + Duration::from_millis(1_750_000_000_000);
        let store = StoreCoordinator::open(&database).unwrap();
        assert_eq!(
            store.load_cargo_enrollment().unwrap(),
            CargoEnrollmentSetting::NOT_ENROLLED
        );
        assert!(
            !store
                .revoke_cargo_executable_at_for_test(start)
                .unwrap()
                .changed
        );

        let first = store
            .enroll_cargo_executable_at_for_test(identity(&cargo, 1), start)
            .unwrap();
        assert!(first.changed);
        assert_eq!(first.setting.revision, 1);
        let retry = store
            .enroll_cargo_executable_at_for_test(
                identity(&cargo, 1),
                start + Duration::from_millis(1),
            )
            .unwrap();
        assert!(!retry.changed);
        assert_eq!(retry.setting, first.setting);

        let replaced = store
            .enroll_cargo_executable_after_commit_failure_for_test(
                identity(&cargo, 2),
                start + Duration::from_millis(2),
            )
            .unwrap();
        assert_eq!(replaced.setting.revision, 2);
        let revoked = store
            .revoke_cargo_executable_at_for_test(start + Duration::from_millis(3))
            .unwrap();
        assert_eq!(revoked.setting.revision, 3);
        assert_eq!(revoked.setting.state, CargoEnrollmentState::Revoked);
        assert!(
            !store
                .revoke_cargo_executable_at_for_test(start + Duration::from_millis(4))
                .unwrap()
                .changed
        );
        let reenrolled = store
            .enroll_cargo_executable_at_for_test(
                identity(&cargo, 1),
                start + Duration::from_millis(5),
            )
            .unwrap();
        assert_eq!(reenrolled.setting.revision, 4);
        drop(store);
        assert_eq!(
            StoreCoordinator::open(&database)
                .unwrap()
                .load_cargo_enrollment()
                .unwrap(),
            reenrolled.setting
        );
    }

    #[test]
    fn canonical_round_trip_and_malformed_rows_fail_closed() {
        let temp = TempDir::new().unwrap();
        let cargo = temp.path().join("toolchain/bin/cargo");
        let setting = CargoEnrollmentSetting {
            revision: 1,
            state: CargoEnrollmentState::Enrolled(Box::new(identity(&cargo, 7))),
            updated_at: Some(UNIX_EPOCH + Duration::from_millis(1)),
        };
        let json = canonical_json(&setting, HistoryErrorKind::InvalidInput).unwrap();
        let value: CargoEnrollmentValueV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(setting_from_value(value, 1).unwrap(), setting);

        let malformed = [
            "{}",
            "{\"revision\":0,\"state\":\"revoked\",\"path_encoding\":null,\"path_hex\":null,\"executable_sha256\":null,\"version_sha256\":null,\"cargo_major\":null,\"cargo_minor\":null,\"cargo_patch\":null,\"signature_policy_revision\":null,\"signature_class\":null,\"signature_flags\":null,\"code_directory_hashes\":null,\"signing_identifier\":null,\"team_identifier\":null,\"designated_requirement_sha256\":null}",
            "{\"revision\":1,\"state\":\"revoked\",\"path_encoding\":null,\"path_hex\":null,\"executable_sha256\":null,\"version_sha256\":null,\"cargo_major\":null,\"cargo_minor\":null,\"cargo_patch\":null,\"signature_policy_revision\":null,\"signature_class\":null,\"signature_flags\":null,\"code_directory_hashes\":null,\"signing_identifier\":null,\"team_identifier\":null,\"designated_requirement_sha256\":null,\"unknown\":true}",
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
                        params![CARGO_ENROLLMENT_KEY, value],
                    )
                    .unwrap();
            });
            assert_eq!(
                store.load_cargo_enrollment().unwrap_err().kind,
                HistoryErrorKind::CorruptData
            );
        }
    }

    #[test]
    fn revoked_row_is_tombstone_and_newer_schema_is_preserved() {
        let temp = TempDir::new().unwrap();
        let cargo = temp.path().join("toolchain/bin/cargo");
        let store = open(&temp);
        store.enroll_cargo_executable(identity(&cargo, 1)).unwrap();
        store.revoke_cargo_executable().unwrap();
        store.with_connection(|connection| {
            let json = connection
                .query_row(
                    "SELECT value_json FROM settings WHERE setting_key = ?1",
                    [CARGO_ENROLLMENT_KEY],
                    |row| row.get::<_, String>(0),
                )
                .unwrap();
            assert!(json.contains("\"state\":\"revoked\""));
            assert!(!json.contains(cargo.to_string_lossy().as_ref()));
        });

        let newer_temp = TempDir::new().unwrap();
        let newer = open(&newer_temp);
        newer.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version,
                         updated_at_unix_ms
                     ) VALUES (?1, '{}', 2, 1)",
                    [CARGO_ENROLLMENT_KEY],
                )
                .unwrap();
        });
        assert_eq!(
            newer.load_cargo_enrollment().unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
        assert_eq!(
            newer.revoke_cargo_executable().unwrap_err().kind,
            HistoryErrorKind::IncompatibleSchema
        );
    }

    #[test]
    fn conditional_enrollment_rejects_a_stale_pre_revocation_revision() {
        let temp = TempDir::new().unwrap();
        let cargo = temp.path().join("toolchain/bin/cargo");
        let store = open(&temp);
        let enrolled = store.enroll_cargo_executable(identity(&cargo, 1)).unwrap();
        let revoked = store.revoke_cargo_executable().unwrap();
        assert_eq!(revoked.setting.revision, 2);

        let error = store
            .enroll_cargo_executable_if_current(identity(&cargo, 1), &enrolled.setting)
            .unwrap_err();
        assert_eq!(error.kind, HistoryErrorKind::InvalidTransition);
        assert_eq!(store.load_cargo_enrollment().unwrap(), revoked.setting);
    }

    #[test]
    fn malformed_enrolled_fields_and_oversized_rows_fail_closed() {
        let temp = TempDir::new().unwrap();
        let cargo = temp.path().join("toolchain/bin/cargo");
        let setting = CargoEnrollmentSetting {
            revision: 1,
            state: CargoEnrollmentState::Enrolled(Box::new(identity(&cargo, 7))),
            updated_at: Some(UNIX_EPOCH + Duration::from_millis(1)),
        };
        let canonical = canonical_json(&setting, HistoryErrorKind::InvalidInput).unwrap();
        let original: serde_json::Value = serde_json::from_str(&canonical).unwrap();
        let mutations: [fn(&mut serde_json::Value); 6] = [
            |value: &mut serde_json::Value| {
                value["executable_sha256"] = serde_json::Value::String("AA".repeat(32));
            },
            |value: &mut serde_json::Value| {
                value["code_directory_hashes"]
                    .as_array_mut()
                    .unwrap()
                    .reverse();
            },
            |value: &mut serde_json::Value| {
                value["team_identifier"] = serde_json::Value::String("unexpected".into());
            },
            |value: &mut serde_json::Value| {
                value["state"] = serde_json::Value::String("revoked".into());
            },
            |value: &mut serde_json::Value| {
                value["signature_flags"] = serde_json::Value::from(0_u32);
            },
            |value: &mut serde_json::Value| {
                value["cargo_patch"] = serde_json::Value::from(1_u32);
            },
        ];
        for mutate in mutations {
            let mut value = original.clone();
            mutate(&mut value);
            let decoded: CargoEnrollmentValueV1 = serde_json::from_value(value).unwrap();
            assert_eq!(
                setting_from_value(decoded, 1).unwrap_err().kind,
                HistoryErrorKind::CorruptData
            );
        }

        let oversized_temp = TempDir::new().unwrap();
        let oversized = open(&oversized_temp);
        oversized.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version,
                         updated_at_unix_ms
                     ) VALUES (?1, ?2, 1, 1)",
                    params![
                        CARGO_ENROLLMENT_KEY,
                        "x".repeat(MAX_CANONICAL_VALUE_BYTES + 1)
                    ],
                )
                .unwrap();
        });
        assert_eq!(
            oversized.load_cargo_enrollment().unwrap_err().kind,
            HistoryErrorKind::CorruptData
        );
    }

    #[test]
    fn timestamps_are_monotonic_and_maximum_revision_cannot_advance() {
        let temp = TempDir::new().unwrap();
        let cargo = temp.path().join("toolchain/bin/cargo");
        let store = open(&temp);
        let later = UNIX_EPOCH + Duration::from_millis(500);
        let first = store
            .enroll_cargo_executable_at_for_test(identity(&cargo, 1), later)
            .unwrap();
        let rollback = store
            .enroll_cargo_executable_at_for_test(
                identity(&cargo, 2),
                UNIX_EPOCH + Duration::from_millis(100),
            )
            .unwrap();
        assert_eq!(rollback.setting.updated_at, first.setting.updated_at);

        let exhausted_temp = TempDir::new().unwrap();
        let exhausted = open(&exhausted_temp);
        let maximum = CargoEnrollmentSetting {
            revision: MAX_ENROLLMENT_REVISION,
            state: CargoEnrollmentState::Enrolled(Box::new(identity(&cargo, 3))),
            updated_at: Some(later),
        };
        let json = canonical_json(&maximum, HistoryErrorKind::InvalidInput).unwrap();
        exhausted.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO settings (
                         setting_key, value_json, value_schema_version,
                         updated_at_unix_ms
                     ) VALUES (?1, ?2, 1, 500)",
                    params![CARGO_ENROLLMENT_KEY, json],
                )
                .unwrap();
        });
        assert_eq!(exhausted.load_cargo_enrollment().unwrap(), maximum);
        assert_eq!(
            exhausted
                .enroll_cargo_executable_if_current(identity(&cargo, 4), &maximum)
                .unwrap_err()
                .kind,
            HistoryErrorKind::InvalidTransition
        );
        assert_eq!(exhausted.load_cargo_enrollment().unwrap(), maximum);
    }

    #[test]
    fn enrollment_identity_rejects_noncanonical_paths_and_unreviewed_releases() {
        let signature = CargoCodeSignatureRecord {
            policy_revision: CARGO_SIGNATURE_POLICY_REVISION,
            class: CargoSignatureClass::AdHoc,
            flags: CARGO_CODE_SIGN_ADHOC_FLAG,
            code_directory_hashes: vec![vec![1; 20]],
            signing_identifier: "cargo-test".into(),
            team_identifier: None,
            designated_requirement_sha256: None,
        };
        for path in [
            PathBuf::from("/tmp/../tmp/cargo"),
            PathBuf::from("/tmp//tool/cargo"),
            PathBuf::from("/tmp/./tool/cargo"),
            PathBuf::from("relative/cargo"),
            PathBuf::from("/tmp/not-cargo"),
        ] {
            assert_eq!(
                CargoExecutableEnrollmentIdentity::new(
                    path,
                    [1; 32],
                    [2; 32],
                    CARGO_ENROLLMENT_SUPPORTED_RELEASE,
                    signature.clone(),
                )
                .unwrap_err()
                .kind,
                HistoryErrorKind::InvalidInput
            );
        }
        assert_eq!(
            CargoExecutableEnrollmentIdentity::new(
                PathBuf::from("/tmp/cargo"),
                [1; 32],
                [2; 32],
                [1, 96, 1],
                signature,
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );
    }
}
