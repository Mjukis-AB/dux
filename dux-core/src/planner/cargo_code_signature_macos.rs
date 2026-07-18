//! Bounded macOS static-code integrity evidence for an enrolled Cargo binary.
//!
//! A valid ad-hoc signature is integrity metadata, not publisher identity.
//! Trust comes only from explicit enrollment of the exact executable bytes.

use std::ffi::c_void;
use std::path::Path;
use std::ptr;

use core_foundation::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex};
use core_foundation::base::{CFGetTypeID, CFRange, CFTypeRef, TCFType};
use core_foundation::data::{CFData, CFDataGetLength, CFDataGetTypeID, CFDataRef};
use core_foundation::dictionary::{CFDictionary, CFDictionaryGetValueIfPresent, CFDictionaryRef};
use core_foundation::number::{CFNumberGetTypeID, CFNumberGetValue, kCFNumberSInt64Type};
use core_foundation::string::{
    CFStringGetBytes, CFStringGetLength, CFStringGetTypeID, CFStringRef, kCFStringEncodingUTF8,
};
use core_foundation::url::CFURL;
use security_framework::os::macos::code_signing::{Flags, SecStaticCode};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::persistence::{
    CARGO_CODE_SIGN_ADHOC_FLAG, CARGO_SIGNATURE_POLICY_REVISION, CargoCodeSignatureRecord,
    CargoSignatureClass,
};

const SIGNING_INFORMATION: u32 = 1 << 1;
const REQUIREMENT_INFORMATION: u32 = 1 << 2;
const MAX_CODE_DIRECTORY_HASHES: usize = 16;
const MIN_CODE_DIRECTORY_HASH_BYTES: usize = 20;
const MAX_CODE_DIRECTORY_HASH_BYTES: usize = 64;
const MAX_SIGNING_IDENTIFIER_BYTES: usize = 512;
const MAX_TEAM_IDENTIFIER_BYTES: usize = 128;
const MAX_DESIGNATED_REQUIREMENT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(super) enum CargoCodeSignatureError {
    #[error("Cargo does not have a valid embedded macOS code signature")]
    Invalid,
    #[error("Cargo code-signing information is malformed or outside its fixed bounds")]
    Malformed,
}

pub(super) fn inspect_cargo_code_signature(
    executable: &Path,
) -> Result<CargoCodeSignatureRecord, CargoCodeSignatureError> {
    let url = CFURL::from_path(executable, false).ok_or(CargoCodeSignatureError::Malformed)?;
    let code = SecStaticCode::from_path(&url, Flags::NONE)
        .map_err(|_| CargoCodeSignatureError::Invalid)?;
    let validation_flags = Flags::CHECK_ALL_ARCHITECTURES
        | Flags::STRICT_VALIDATE
        | Flags::NO_NETWORK_ACCESS
        | Flags::SINGLE_THREADED;
    // A null requirement requests normal static-code validation without
    // pretending to authenticate an ad-hoc Cargo publisher identity.
    let status = unsafe {
        SecStaticCodeCheckValidity(
            code.as_CFTypeRef().cast_mut(),
            validation_flags.bits(),
            ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(CargoCodeSignatureError::Invalid);
    }

    let mut raw_information: CFDictionaryRef = ptr::null();
    let status = unsafe {
        SecCodeCopySigningInformation(
            code.as_CFTypeRef().cast_mut(),
            SIGNING_INFORMATION | REQUIREMENT_INFORMATION,
            &mut raw_information,
        )
    };
    if raw_information.is_null() {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let information: CFDictionary = unsafe { TCFType::wrap_under_create_rule(raw_information) };
    if status != 0 {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let flags = dictionary_u32(&information, unsafe { kSecCodeInfoFlags })?;
    let signing_identifier = dictionary_string(
        &information,
        unsafe { kSecCodeInfoIdentifier },
        MAX_SIGNING_IDENTIFIER_BYTES,
    )?
    .ok_or(CargoCodeSignatureError::Malformed)?;
    let team_identifier = dictionary_string(
        &information,
        unsafe { kSecCodeInfoTeamIdentifier },
        MAX_TEAM_IDENTIFIER_BYTES,
    )?;
    let mut code_directory_hashes = dictionary_data_array(
        &information,
        unsafe { kSecCodeInfoCdHashes },
        MAX_CODE_DIRECTORY_HASHES,
    )?;
    if code_directory_hashes.is_empty()
        && let Some(unique) = dictionary_data(
            &information,
            unsafe { kSecCodeInfoUnique },
            MIN_CODE_DIRECTORY_HASH_BYTES,
            MAX_CODE_DIRECTORY_HASH_BYTES,
        )?
    {
        code_directory_hashes.push(unique);
    }
    if code_directory_hashes.iter().any(|value| {
        !(MIN_CODE_DIRECTORY_HASH_BYTES..=MAX_CODE_DIRECTORY_HASH_BYTES).contains(&value.len())
    }) {
        return Err(CargoCodeSignatureError::Malformed);
    }
    code_directory_hashes.sort_unstable();
    code_directory_hashes.dedup();
    if code_directory_hashes.is_empty() || code_directory_hashes.len() > MAX_CODE_DIRECTORY_HASHES {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let designated_requirement_sha256 =
        dictionary_requirement_digest(&information, unsafe { kSecCodeInfoDesignatedRequirement })?;
    let class = if flags & CARGO_CODE_SIGN_ADHOC_FLAG != 0 {
        if team_identifier.is_some() {
            return Err(CargoCodeSignatureError::Malformed);
        }
        CargoSignatureClass::AdHoc
    } else {
        CargoSignatureClass::Cms
    };
    let record = CargoCodeSignatureRecord {
        policy_revision: CARGO_SIGNATURE_POLICY_REVISION,
        class,
        flags,
        code_directory_hashes,
        signing_identifier,
        team_identifier,
        designated_requirement_sha256,
    };
    record
        .validate()
        .map_err(|_| CargoCodeSignatureError::Malformed)?;
    Ok(record)
}

fn dictionary_value(dictionary: &CFDictionary, key: CFStringRef) -> Option<CFTypeRef> {
    if key.is_null() {
        return None;
    }
    let mut value: *const c_void = ptr::null();
    let present = unsafe {
        CFDictionaryGetValueIfPresent(dictionary.as_concrete_TypeRef(), key.cast(), &mut value)
    };
    (present != 0 && !value.is_null()).then_some(value.cast())
}

fn dictionary_u32(
    dictionary: &CFDictionary,
    key: CFStringRef,
) -> Result<u32, CargoCodeSignatureError> {
    let value = dictionary_value(dictionary, key).ok_or(CargoCodeSignatureError::Malformed)?;
    if unsafe { CFGetTypeID(value) } != unsafe { CFNumberGetTypeID() } {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let mut number = 0_i64;
    let converted =
        unsafe { CFNumberGetValue(value.cast(), kCFNumberSInt64Type, (&raw mut number).cast()) };
    if !converted {
        return Err(CargoCodeSignatureError::Malformed);
    }
    u32::try_from(number).map_err(|_| CargoCodeSignatureError::Malformed)
}

fn dictionary_string(
    dictionary: &CFDictionary,
    key: CFStringRef,
    maximum_bytes: usize,
) -> Result<Option<String>, CargoCodeSignatureError> {
    let Some(value) = dictionary_value(dictionary, key) else {
        return Ok(None);
    };
    if unsafe { CFGetTypeID(value) } != unsafe { CFStringGetTypeID() } {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let string_ref: CFStringRef = value.cast();
    let length = unsafe { CFStringGetLength(string_ref) };
    let maximum_buffer =
        isize::try_from(maximum_bytes).map_err(|_| CargoCodeSignatureError::Malformed)?;
    if length <= 0 || length > maximum_buffer {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let mut bytes = vec![0_u8; maximum_bytes];
    let mut used = 0_isize;
    let converted = unsafe {
        CFStringGetBytes(
            string_ref,
            CFRange::init(0, length),
            kCFStringEncodingUTF8,
            0,
            0,
            bytes.as_mut_ptr(),
            maximum_buffer,
            &mut used,
        )
    };
    if converted != length || used <= 0 {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let used = usize::try_from(used).map_err(|_| CargoCodeSignatureError::Malformed)?;
    bytes.truncate(used);
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| CargoCodeSignatureError::Malformed)
}

fn dictionary_data(
    dictionary: &CFDictionary,
    key: CFStringRef,
    minimum_bytes: usize,
    maximum_bytes: usize,
) -> Result<Option<Vec<u8>>, CargoCodeSignatureError> {
    let Some(value) = dictionary_value(dictionary, key) else {
        return Ok(None);
    };
    if unsafe { CFGetTypeID(value) } != unsafe { CFDataGetTypeID() } {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let data_ref: CFDataRef = value.cast();
    let length = usize::try_from(unsafe { CFDataGetLength(data_ref) })
        .map_err(|_| CargoCodeSignatureError::Malformed)?;
    if !(minimum_bytes..=maximum_bytes).contains(&length) {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let data = unsafe { CFData::wrap_under_get_rule(data_ref) };
    Ok(Some(data.bytes().to_vec()))
}

fn dictionary_data_array(
    dictionary: &CFDictionary,
    key: CFStringRef,
    maximum_count: usize,
) -> Result<Vec<Vec<u8>>, CargoCodeSignatureError> {
    let Some(value) = dictionary_value(dictionary, key) else {
        return Ok(Vec::new());
    };
    if unsafe { CFGetTypeID(value) } != unsafe { CFArrayGetTypeID() } {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let count = usize::try_from(unsafe { CFArrayGetCount(value.cast()) })
        .map_err(|_| CargoCodeSignatureError::Malformed)?;
    if count > maximum_count {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let mut hashes = Vec::new();
    hashes
        .try_reserve_exact(count)
        .map_err(|_| CargoCodeSignatureError::Malformed)?;
    for index in 0..count {
        let index = isize::try_from(index).map_err(|_| CargoCodeSignatureError::Malformed)?;
        let item = unsafe { CFArrayGetValueAtIndex(value.cast(), index) };
        if item.is_null() || unsafe { CFGetTypeID(item.cast()) } != unsafe { CFDataGetTypeID() } {
            return Err(CargoCodeSignatureError::Malformed);
        }
        let data_ref: CFDataRef = item.cast();
        let length = usize::try_from(unsafe { CFDataGetLength(data_ref) })
            .map_err(|_| CargoCodeSignatureError::Malformed)?;
        if !(MIN_CODE_DIRECTORY_HASH_BYTES..=MAX_CODE_DIRECTORY_HASH_BYTES).contains(&length) {
            return Err(CargoCodeSignatureError::Malformed);
        }
        let data = unsafe { CFData::wrap_under_get_rule(data_ref) };
        hashes.push(data.bytes().to_vec());
    }
    Ok(hashes)
}

fn dictionary_requirement_digest(
    dictionary: &CFDictionary,
    key: CFStringRef,
) -> Result<Option<[u8; 32]>, CargoCodeSignatureError> {
    let Some(requirement) = dictionary_value(dictionary, key) else {
        return Ok(None);
    };
    if unsafe { CFGetTypeID(requirement) } != unsafe { SecRequirementGetTypeID() } {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let mut raw_data: CFDataRef = ptr::null();
    let status = unsafe { SecRequirementCopyData(requirement.cast_mut(), 0, &mut raw_data) };
    if raw_data.is_null() {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let data = unsafe { CFData::wrap_under_create_rule(raw_data) };
    if status != 0 {
        return Err(CargoCodeSignatureError::Malformed);
    }
    let length = usize::try_from(unsafe { CFDataGetLength(data.as_concrete_TypeRef()) })
        .map_err(|_| CargoCodeSignatureError::Malformed)?;
    if !(1..=MAX_DESIGNATED_REQUIREMENT_BYTES).contains(&length) {
        return Err(CargoCodeSignatureError::Malformed);
    }
    Ok(Some(Sha256::digest(data.bytes()).into()))
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecStaticCodeCheckValidity(code: *mut c_void, flags: u32, requirement: *mut c_void) -> i32;
    fn SecCodeCopySigningInformation(
        code: *mut c_void,
        flags: u32,
        information: *mut CFDictionaryRef,
    ) -> i32;
    fn SecRequirementGetTypeID() -> usize;
    fn SecRequirementCopyData(requirement: *mut c_void, flags: u32, data: *mut CFDataRef) -> i32;

    static kSecCodeInfoFlags: CFStringRef;
    static kSecCodeInfoIdentifier: CFStringRef;
    static kSecCodeInfoTeamIdentifier: CFStringRef;
    static kSecCodeInfoUnique: CFStringRef;
    static kSecCodeInfoCdHashes: CFStringRef;
    static kSecCodeInfoDesignatedRequirement: CFStringRef;
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn system_binary_has_strict_bounded_signature_evidence() {
        let signature = inspect_cargo_code_signature(Path::new("/usr/bin/true")).unwrap();
        assert_eq!(signature.policy_revision, CARGO_SIGNATURE_POLICY_REVISION);
        assert!(!signature.signing_identifier.is_empty());
        assert!(!signature.code_directory_hashes.is_empty());
        assert!(signature.validate().is_ok());
    }

    #[test]
    fn unsigned_executable_is_rejected() {
        let temp = TempDir::new().unwrap();
        let executable = temp.path().join("cargo");
        std::fs::write(&executable, b"#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(inspect_cargo_code_signature(&executable).is_err());
    }
}
