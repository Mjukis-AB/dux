//! Lossless byte encodings frozen by the v1 SQLite schema.
//!
//! These values describe stored observations only. Decoding a path does not
//! validate it, make it current, or grant cleanup authority.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

const MAX_PATH_ENCODED_UNITS: usize = 32 * 1024;
const MAX_LOGICAL_KEY_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i64)]
pub(crate) enum StoredEncoding {
    Utf8LogicalKey = 0,
    Utf8HostPath = 1,
    Utf16LeHostPath = 2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EncodedBytes {
    pub(crate) encoding: StoredEncoding,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CodecError {
    Empty,
    InvalidEncoding,
    TooLong,
    UnsupportedHostEncoding,
}

pub(crate) fn encode_logical_key(value: &str) -> Result<EncodedBytes, CodecError> {
    if value.is_empty() {
        return Err(CodecError::Empty);
    }
    if value.len() > MAX_LOGICAL_KEY_BYTES {
        return Err(CodecError::TooLong);
    }
    Ok(EncodedBytes {
        encoding: StoredEncoding::Utf8LogicalKey,
        bytes: value.as_bytes().to_vec(),
    })
}

pub(crate) fn decode_logical_key(value: &EncodedBytes) -> Result<String, CodecError> {
    if value.encoding != StoredEncoding::Utf8LogicalKey {
        return Err(CodecError::InvalidEncoding);
    }
    if value.bytes.is_empty() {
        return Err(CodecError::Empty);
    }
    if value.bytes.len() > MAX_LOGICAL_KEY_BYTES {
        return Err(CodecError::TooLong);
    }
    String::from_utf8(value.bytes.clone()).map_err(|_| CodecError::InvalidEncoding)
}

#[cfg(unix)]
pub(crate) fn encode_host_path(path: &Path) -> Result<EncodedBytes, CodecError> {
    use std::os::unix::ffi::OsStrExt;

    let bytes = path.as_os_str().as_bytes();
    validate_utf8_path_bytes(bytes)?;
    Ok(EncodedBytes {
        encoding: StoredEncoding::Utf8HostPath,
        bytes: bytes.to_vec(),
    })
}

#[cfg(windows)]
pub(crate) fn encode_host_path(path: &Path) -> Result<EncodedBytes, CodecError> {
    use std::os::windows::ffi::OsStrExt;

    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    if units.is_empty() {
        return Err(CodecError::Empty);
    }
    if units.len() > MAX_PATH_ENCODED_UNITS {
        return Err(CodecError::TooLong);
    }
    String::from_utf16(&units).map_err(|_| CodecError::InvalidEncoding)?;
    let mut bytes = Vec::with_capacity(units.len() * 2);
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(EncodedBytes {
        encoding: StoredEncoding::Utf16LeHostPath,
        bytes,
    })
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn encode_host_path(path: &Path) -> Result<EncodedBytes, CodecError> {
    let value = path.to_str().ok_or(CodecError::InvalidEncoding)?;
    validate_utf8_path_bytes(value.as_bytes())?;
    Ok(EncodedBytes {
        encoding: StoredEncoding::Utf8HostPath,
        bytes: value.as_bytes().to_vec(),
    })
}

#[cfg(unix)]
pub(crate) fn decode_host_path(value: &EncodedBytes) -> Result<PathBuf, CodecError> {
    use std::os::unix::ffi::OsStringExt;

    if value.encoding != StoredEncoding::Utf8HostPath {
        return Err(CodecError::UnsupportedHostEncoding);
    }
    validate_utf8_path_bytes(&value.bytes)?;
    Ok(PathBuf::from(OsString::from_vec(value.bytes.clone())))
}

#[cfg(windows)]
pub(crate) fn decode_host_path(value: &EncodedBytes) -> Result<PathBuf, CodecError> {
    use std::os::windows::ffi::OsStringExt;

    if value.encoding != StoredEncoding::Utf16LeHostPath {
        return Err(CodecError::UnsupportedHostEncoding);
    }
    if value.bytes.is_empty() {
        return Err(CodecError::Empty);
    }
    if value.bytes.len() > MAX_PATH_ENCODED_UNITS * 2 || value.bytes.len() % 2 != 0 {
        return Err(CodecError::TooLong);
    }
    let units: Vec<u16> = value
        .bytes
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect();
    String::from_utf16(&units).map_err(|_| CodecError::InvalidEncoding)?;
    Ok(PathBuf::from(OsString::from_wide(&units)))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn decode_host_path(value: &EncodedBytes) -> Result<PathBuf, CodecError> {
    if value.encoding != StoredEncoding::Utf8HostPath {
        return Err(CodecError::UnsupportedHostEncoding);
    }
    validate_utf8_path_bytes(&value.bytes)?;
    let text = String::from_utf8(value.bytes.clone()).map_err(|_| CodecError::InvalidEncoding)?;
    Ok(PathBuf::from(text))
}

fn validate_utf8_path_bytes(bytes: &[u8]) -> Result<(), CodecError> {
    if bytes.is_empty() {
        return Err(CodecError::Empty);
    }
    if bytes.len() > MAX_PATH_ENCODED_UNITS {
        return Err(CodecError::TooLong);
    }
    std::str::from_utf8(bytes).map_err(|_| CodecError::InvalidEncoding)?;
    Ok(())
}
