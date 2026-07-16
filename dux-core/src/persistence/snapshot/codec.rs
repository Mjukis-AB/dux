//! Bounded, checksummed, non-authoritative full-tree snapshot codec.
//!
//! The format is independent from the legacy CLI cache. Node records are
//! depth-first pre-order. Graph, aggregate, and sibling-name validation remains
//! bounded by the explicit node, depth, component, path, and file limits.
//! Decoded names and roots are observations only; they never grant path or
//! cleanup authority.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::{Component, Path};

use sha2::{Digest, Sha256};

use crate::domain::ScanId;

pub(crate) const SNAPSHOT_FORMAT_VERSION: u32 = 1;
pub(crate) const MAX_SNAPSHOT_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_NODES: u64 = 5_000_000;
pub(crate) const MAX_SNAPSHOT_DEPTH: u32 = 4_096;
pub(crate) const MAX_COMPONENT_BYTES: usize = 1_024;

const MAGIC: &[u8; 8] = b"DUXSNAP\0";
const HEADER_SIZE: usize = 96;
const NODE_HEADER_SIZE: usize = 112;
const DIGEST_SIZE: usize = 32;
const MAX_SCAN_ID_BYTES: usize = 128;
const MAX_ROOT_BYTES: usize = 65_536;
const MAX_PATH_BYTES: usize = 65_536;
const NO_PARENT: u64 = u64::MAX;
const FLAG_ALLOCATED: u8 = 1 << 0;
const FLAG_MTIME: u8 = 1 << 1;
const FLAG_ATIME: u8 = 1 << 2;
const FLAG_UNIX_IDENTITY: u8 = 1 << 3;
const KNOWN_NODE_FIELD_FLAGS: u8 = FLAG_ALLOCATED | FLAG_MTIME | FLAG_ATIME | FLAG_UNIX_IDENTITY;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum HostEncoding {
    UnixBytes = 1,
    WindowsUtf16Le = 2,
}

impl HostEncoding {
    fn from_byte(value: u8) -> Result<Self, SnapshotCodecError> {
        match value {
            1 => Ok(Self::UnixBytes),
            2 => Ok(Self::WindowsUtf16Le),
            _ => Err(error(SnapshotCodecErrorKind::CorruptData)),
        }
    }

    const fn separator_bytes(self) -> usize {
        match self {
            Self::UnixBytes => 1,
            Self::WindowsUtf16Le => 2,
        }
    }
}

/// Lossless accepted-host bytes. This is deliberately not a cleanup path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HostValue {
    encoding: HostEncoding,
    bytes: Vec<u8>,
}

impl HostValue {
    pub(crate) fn from_root(path: &Path) -> Result<Self, SnapshotCodecError> {
        if !path.is_absolute() {
            return Err(error(SnapshotCodecErrorKind::InvalidInput));
        }
        let value = encode_host_bounded(
            path.as_os_str(),
            MAX_ROOT_BYTES,
            SnapshotCodecErrorKind::InvalidInput,
        )?;
        validate_root(&value, SnapshotCodecErrorKind::InvalidInput)?;
        Ok(value)
    }

    pub(crate) fn from_component(value: &OsStr) -> Result<Self, SnapshotCodecError> {
        let value = encode_host_bounded(
            value,
            MAX_COMPONENT_BYTES,
            SnapshotCodecErrorKind::InvalidInput,
        )?;
        validate_component(&value, SnapshotCodecErrorKind::InvalidInput)?;
        Ok(value)
    }

    pub(crate) fn matches_path(&self, path: &Path) -> bool {
        encode_host_bounded(
            path.as_os_str(),
            MAX_ROOT_BYTES,
            SnapshotCodecErrorKind::InvalidInput,
        )
        .is_ok_and(|value| value == *self)
    }
}

#[cfg(unix)]
fn encode_host_bounded(
    value: &OsStr,
    maximum_bytes: usize,
    invalid: SnapshotCodecErrorKind,
) -> Result<HostValue, SnapshotCodecError> {
    use std::os::unix::ffi::OsStrExt;

    let source = value.as_bytes();
    if source.len() > maximum_bytes {
        return Err(error(invalid));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(source.len())
        .map_err(|_| error(SnapshotCodecErrorKind::LimitExceeded))?;
    bytes.extend_from_slice(source);
    Ok(HostValue {
        encoding: HostEncoding::UnixBytes,
        bytes,
    })
}

#[cfg(windows)]
fn encode_host_bounded(
    value: &OsStr,
    maximum_bytes: usize,
    invalid: SnapshotCodecErrorKind,
) -> Result<HostValue, SnapshotCodecError> {
    use std::os::windows::ffi::OsStrExt;

    let maximum_units = maximum_bytes / 2;
    let unit_count = value.encode_wide().take(maximum_units + 1).count();
    if unit_count > maximum_units {
        return Err(error(invalid));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(unit_count.saturating_mul(2))
        .map_err(|_| error(SnapshotCodecErrorKind::LimitExceeded))?;
    for unit in value.encode_wide() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(HostValue {
        encoding: HostEncoding::WindowsUtf16Le,
        bytes,
    })
}

#[cfg(not(any(unix, windows)))]
compile_error!("the DUX snapshot codec requires Unix or Windows host-path encoding");

#[cfg(unix)]
fn host_path(value: &HostValue) -> Result<std::path::PathBuf, SnapshotCodecError> {
    use std::os::unix::ffi::OsStringExt;
    if value.encoding != HostEncoding::UnixBytes || value.bytes.contains(&0) {
        return Err(error(SnapshotCodecErrorKind::CorruptData));
    }
    Ok(std::path::PathBuf::from(std::ffi::OsString::from_vec(
        value.bytes.clone(),
    )))
}

#[cfg(windows)]
fn host_path(value: &HostValue) -> Result<std::path::PathBuf, SnapshotCodecError> {
    use std::os::windows::ffi::OsStringExt;
    if value.encoding != HostEncoding::WindowsUtf16Le
        || value.bytes.is_empty()
        || value.bytes.len() % 2 != 0
    {
        return Err(error(SnapshotCodecErrorKind::CorruptData));
    }
    let units = value
        .bytes
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .collect::<Vec<_>>();
    if units.contains(&0) {
        return Err(error(SnapshotCodecErrorKind::CorruptData));
    }
    Ok(std::path::PathBuf::from(std::ffi::OsString::from_wide(
        &units,
    )))
}

fn validate_root(
    value: &HostValue,
    kind: SnapshotCodecErrorKind,
) -> Result<(), SnapshotCodecError> {
    if value.bytes.is_empty() || value.bytes.len() > MAX_ROOT_BYTES {
        return Err(error(kind));
    }
    let path = host_path(value).map_err(|_| error(kind))?;
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(error(kind));
    }
    Ok(())
}

fn validate_component(
    value: &HostValue,
    kind: SnapshotCodecErrorKind,
) -> Result<(), SnapshotCodecError> {
    if value.bytes.is_empty() || value.bytes.len() > MAX_COMPONENT_BYTES {
        return Err(error(kind));
    }
    let path = host_path(value).map_err(|_| error(kind))?;
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(error(kind));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotTimestamp {
    seconds_since_unix_epoch: u64,
    nanoseconds: u32,
}

impl SnapshotTimestamp {
    pub(crate) fn new(
        seconds_since_unix_epoch: u64,
        nanoseconds: u32,
    ) -> Result<Self, SnapshotCodecError> {
        if nanoseconds >= 1_000_000_000 {
            return Err(error(SnapshotCodecErrorKind::InvalidInput));
        }
        Ok(Self {
            seconds_since_unix_epoch,
            nanoseconds,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum SnapshotNodeKind {
    Directory = 1,
    File = 2,
    Symlink = 3,
    Other = 4,
    Error = 5,
}

impl SnapshotNodeKind {
    fn from_byte(value: u8) -> Result<Self, SnapshotCodecError> {
        match value {
            1 => Ok(Self::Directory),
            2 => Ok(Self::File),
            3 => Ok(Self::Symlink),
            4 => Ok(Self::Other),
            5 => Ok(Self::Error),
            _ => Err(error(SnapshotCodecErrorKind::CorruptData)),
        }
    }
}

/// Exact scan observations attached to one snapshot node.
///
/// These flags explain incomplete or deduplicated observations. They are not
/// cleanup authority and unknown wire bits always invalidate the snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SnapshotScanFlags(u32);

impl SnapshotScanFlags {
    pub(crate) const NONE: Self = Self(0);
    pub(crate) const INACCESSIBLE: Self = Self(1 << 0);
    pub(crate) const TIMED_OUT: Self = Self(1 << 1);
    pub(crate) const HARD_LINK_DUPLICATE: Self = Self(1 << 2);
    pub(crate) const MOUNT_BOUNDARY: Self = Self(1 << 3);

    const KNOWN_BITS: u32 = Self::INACCESSIBLE.0
        | Self::TIMED_OUT.0
        | Self::HARD_LINK_DUPLICATE.0
        | Self::MOUNT_BOUNDARY.0;

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }

    const fn bits(self) -> u32 {
        self.0
    }

    fn from_bits(bits: u32) -> Result<Self, SnapshotCodecError> {
        if bits & !Self::KNOWN_BITS != 0 {
            return Err(error(SnapshotCodecErrorKind::CorruptData));
        }
        Ok(Self(bits))
    }

    const fn is_valid(self) -> bool {
        self.0 & !Self::KNOWN_BITS == 0
    }
}

/// Unix device/inode observation used to explain hard-link deduplication.
///
/// It is snapshot evidence only. It must never be used as a current object
/// identity or cleanup witness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotUnixIdentity {
    device: u64,
    inode: u64,
}

impl SnapshotUnixIdentity {
    pub(crate) const fn new(device: u64, inode: u64) -> Self {
        Self { device, inode }
    }

    pub(crate) const fn device(self) -> u64 {
        self.device
    }

    pub(crate) const fn inode(self) -> u64 {
        self.inode
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotNode {
    pub(crate) id: u64,
    pub(crate) parent: Option<u64>,
    pub(crate) depth: u32,
    pub(crate) kind: SnapshotNodeKind,
    pub(crate) name: Option<HostValue>,
    pub(crate) logical_bytes: u64,
    pub(crate) allocated_bytes: Option<u64>,
    pub(crate) file_count: u64,
    pub(crate) child_count: u64,
    pub(crate) modified_at: Option<SnapshotTimestamp>,
    /// Access time is an unreliable filesystem observation, never authority.
    pub(crate) accessed_at: Option<SnapshotTimestamp>,
    pub(crate) scan_flags: SnapshotScanFlags,
    pub(crate) unix_identity: Option<SnapshotUnixIdentity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotTotals {
    pub(crate) directory_count: u64,
    pub(crate) file_count: u64,
    pub(crate) logical_bytes: u64,
    pub(crate) allocated_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotMetadata {
    pub(crate) scan_id: ScanId,
    pub(crate) root: HostValue,
    pub(crate) captured_at: SnapshotTimestamp,
    pub(crate) totals: SnapshotTotals,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotDocument {
    pub(crate) metadata: SnapshotMetadata,
    pub(crate) nodes: Vec<SnapshotNode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SnapshotDigest([u8; DIGEST_SIZE]);

impl SnapshotDigest {
    pub(crate) const fn from_bytes(bytes: [u8; DIGEST_SIZE]) -> Self {
        Self(bytes)
    }

    pub(crate) const fn bytes(self) -> [u8; DIGEST_SIZE] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotCodecErrorKind {
    InvalidInput,
    Io,
    LimitExceeded,
    InvalidMagic,
    IncompatibleVersion,
    InvalidLength,
    ChecksumMismatch,
    CorruptData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("snapshot codec operation failed: {kind:?}")]
pub(crate) struct SnapshotCodecError {
    pub(crate) kind: SnapshotCodecErrorKind,
}

const fn error(kind: SnapshotCodecErrorKind) -> SnapshotCodecError {
    SnapshotCodecError { kind }
}

pub(crate) fn encode_snapshot(
    document: &SnapshotDocument,
    writer: &mut impl Write,
) -> Result<SnapshotDigest, SnapshotCodecError> {
    let layout = validate_document(document, SnapshotCodecErrorKind::InvalidInput)?;
    let mut header = [0_u8; HEADER_SIZE];
    header[..8].copy_from_slice(MAGIC);
    put_u32(&mut header, 8, SNAPSHOT_FORMAT_VERSION);
    put_u32(&mut header, 12, HEADER_SIZE as u32);
    put_u64(&mut header, 16, layout.payload_length);
    put_u64(&mut header, 24, document.nodes.len() as u64);
    put_u64(&mut header, 32, document.metadata.totals.directory_count);
    put_u64(&mut header, 40, document.metadata.totals.file_count);
    put_u64(&mut header, 48, document.metadata.totals.logical_bytes);
    put_u64(
        &mut header,
        56,
        document.metadata.totals.allocated_bytes.unwrap_or(0),
    );
    put_u64(
        &mut header,
        64,
        document.metadata.captured_at.seconds_since_unix_epoch,
    );
    put_u32(&mut header, 72, document.metadata.captured_at.nanoseconds);
    header[76] = if document.metadata.totals.allocated_bytes.is_some() {
        FLAG_ALLOCATED
    } else {
        0
    };
    header[77] = document.metadata.root.encoding as u8;
    put_u32(&mut header, 80, layout.scan_id_length as u32);
    put_u32(&mut header, 84, document.metadata.root.bytes.len() as u32);

    let mut hashing = HashingWriter::new(writer);
    hashing.write_all(&header)?;
    hashing.write_all(document.metadata.scan_id.as_str().as_bytes())?;
    hashing.write_all(&document.metadata.root.bytes)?;
    for node in &document.nodes {
        let mut record = [0_u8; NODE_HEADER_SIZE];
        put_u64(&mut record, 0, node.id);
        put_u64(&mut record, 8, node.parent.unwrap_or(NO_PARENT));
        put_u32(&mut record, 16, node.depth);
        record[20] = node.kind as u8;
        record[21] = (u8::from(node.allocated_bytes.is_some()) * FLAG_ALLOCATED)
            | (u8::from(node.modified_at.is_some()) * FLAG_MTIME)
            | (u8::from(node.accessed_at.is_some()) * FLAG_ATIME)
            | (u8::from(node.unix_identity.is_some()) * FLAG_UNIX_IDENTITY);
        record[22] = node.name.as_ref().map_or(0, |name| name.encoding as u8);
        put_u64(&mut record, 24, node.logical_bytes);
        put_u64(&mut record, 32, node.allocated_bytes.unwrap_or(0));
        put_u64(&mut record, 40, node.file_count);
        put_u64(&mut record, 48, node.child_count);
        put_u64(
            &mut record,
            56,
            node.modified_at
                .map_or(0, |timestamp| timestamp.seconds_since_unix_epoch),
        );
        put_u32(
            &mut record,
            64,
            node.modified_at
                .map_or(0, |timestamp| timestamp.nanoseconds),
        );
        put_u32(
            &mut record,
            68,
            node.name.as_ref().map_or(0, |name| name.bytes.len() as u32),
        );
        put_u64(
            &mut record,
            72,
            node.accessed_at
                .map_or(0, |timestamp| timestamp.seconds_since_unix_epoch),
        );
        put_u32(
            &mut record,
            80,
            node.accessed_at
                .map_or(0, |timestamp| timestamp.nanoseconds),
        );
        put_u32(&mut record, 84, node.scan_flags.bits());
        put_u64(
            &mut record,
            88,
            node.unix_identity.map_or(0, SnapshotUnixIdentity::device),
        );
        put_u64(
            &mut record,
            96,
            node.unix_identity.map_or(0, SnapshotUnixIdentity::inode),
        );
        hashing.write_all(&record)?;
        if let Some(name) = &node.name {
            hashing.write_all(&name.bytes)?;
        }
    }
    if hashing.bytes != HEADER_SIZE as u64 + layout.payload_length {
        return Err(error(SnapshotCodecErrorKind::InvalidInput));
    }
    let (writer, digest) = hashing.finish();
    writer
        .write_all(&digest)
        .map_err(|_| error(SnapshotCodecErrorKind::Io))?;
    Ok(SnapshotDigest(digest))
}

pub(crate) fn validate_snapshot_document(
    document: &SnapshotDocument,
) -> Result<(), SnapshotCodecError> {
    validate_document(document, SnapshotCodecErrorKind::InvalidInput).map(|_| ())
}

pub(crate) fn decode_snapshot(
    reader: &mut impl Read,
) -> Result<(SnapshotDocument, SnapshotDigest), SnapshotCodecError> {
    let mut hashing = HashingReader::new(reader);
    let mut header = [0_u8; HEADER_SIZE];
    hashing.read_exact(&mut header)?;
    if &header[..8] != MAGIC {
        return Err(error(SnapshotCodecErrorKind::InvalidMagic));
    }
    let version = get_u32(&header, 8);
    if version != SNAPSHOT_FORMAT_VERSION {
        return Err(error(SnapshotCodecErrorKind::IncompatibleVersion));
    }
    if get_u32(&header, 12) != HEADER_SIZE as u32
        || header[78..80] != [0, 0]
        || header[88..96] != [0; 8]
        || header[76] & !FLAG_ALLOCATED != 0
    {
        return Err(error(SnapshotCodecErrorKind::CorruptData));
    }
    let payload_length = get_u64(&header, 16);
    let node_count = get_u64(&header, 24);
    let file_length = (HEADER_SIZE as u64)
        .checked_add(payload_length)
        .and_then(|value| value.checked_add(DIGEST_SIZE as u64))
        .ok_or_else(|| error(SnapshotCodecErrorKind::LimitExceeded))?;
    if file_length > MAX_SNAPSHOT_FILE_BYTES || node_count == 0 || node_count > MAX_SNAPSHOT_NODES {
        return Err(error(SnapshotCodecErrorKind::LimitExceeded));
    }
    let scan_id_length = get_u32(&header, 80) as usize;
    let root_length = get_u32(&header, 84) as usize;
    if !(1..=MAX_SCAN_ID_BYTES).contains(&scan_id_length)
        || !(1..=MAX_ROOT_BYTES).contains(&root_length)
    {
        return Err(error(SnapshotCodecErrorKind::LimitExceeded));
    }
    let minimum_payload = (scan_id_length as u64)
        .checked_add(root_length as u64)
        .and_then(|value| value.checked_add(node_count.checked_mul(NODE_HEADER_SIZE as u64)?))
        .ok_or_else(|| error(SnapshotCodecErrorKind::LimitExceeded))?;
    if minimum_payload > payload_length {
        return Err(error(SnapshotCodecErrorKind::InvalidLength));
    }

    let mut scan_id_bytes = allocate_bytes(scan_id_length)?;
    hashing.read_exact(&mut scan_id_bytes)?;
    let scan_id_text = std::str::from_utf8(&scan_id_bytes)
        .map_err(|_| error(SnapshotCodecErrorKind::CorruptData))?;
    let scan_id = ScanId::new(scan_id_text.to_owned())
        .map_err(|_| error(SnapshotCodecErrorKind::CorruptData))?;

    let root_encoding = HostEncoding::from_byte(header[77])?;
    let mut root_bytes = allocate_bytes(root_length)?;
    hashing.read_exact(&mut root_bytes)?;
    let root = HostValue {
        encoding: root_encoding,
        bytes: root_bytes,
    };
    validate_root(&root, SnapshotCodecErrorKind::CorruptData)?;

    let captured_at = timestamp_from_wire(get_u64(&header, 64), get_u32(&header, 72))?;
    let totals = SnapshotTotals {
        directory_count: get_u64(&header, 32),
        file_count: get_u64(&header, 40),
        logical_bytes: get_u64(&header, 48),
        allocated_bytes: if header[76] & FLAG_ALLOCATED != 0 {
            Some(get_u64(&header, 56))
        } else {
            if get_u64(&header, 56) != 0 {
                return Err(error(SnapshotCodecErrorKind::CorruptData));
            }
            None
        },
    };

    let mut nodes = Vec::new();
    let mut remaining_payload = payload_length
        .checked_sub(scan_id_length as u64)
        .and_then(|value| value.checked_sub(root_length as u64))
        .ok_or_else(|| error(SnapshotCodecErrorKind::InvalidLength))?;
    for _ in 0..node_count {
        remaining_payload = remaining_payload
            .checked_sub(NODE_HEADER_SIZE as u64)
            .ok_or_else(|| error(SnapshotCodecErrorKind::InvalidLength))?;
        let mut record = [0_u8; NODE_HEADER_SIZE];
        hashing.read_exact(&mut record)?;
        let flags = record[21];
        if flags & !KNOWN_NODE_FIELD_FLAGS != 0 || record[23] != 0 || record[104..112] != [0; 8] {
            return Err(error(SnapshotCodecErrorKind::CorruptData));
        }
        let name_length = get_u32(&record, 68) as usize;
        let id = get_u64(&record, 0);
        let name = if id == 0 {
            if name_length != 0 || record[22] != 0 {
                return Err(error(SnapshotCodecErrorKind::CorruptData));
            }
            None
        } else {
            if !(1..=MAX_COMPONENT_BYTES).contains(&name_length) {
                return Err(error(SnapshotCodecErrorKind::LimitExceeded));
            }
            remaining_payload = remaining_payload
                .checked_sub(name_length as u64)
                .ok_or_else(|| error(SnapshotCodecErrorKind::InvalidLength))?;
            let encoding = HostEncoding::from_byte(record[22])?;
            let mut bytes = allocate_bytes(name_length)?;
            hashing.read_exact(&mut bytes)?;
            let value = HostValue { encoding, bytes };
            validate_component(&value, SnapshotCodecErrorKind::CorruptData)?;
            Some(value)
        };
        let allocated_bytes = if flags & FLAG_ALLOCATED != 0 {
            Some(get_u64(&record, 32))
        } else {
            if get_u64(&record, 32) != 0 {
                return Err(error(SnapshotCodecErrorKind::CorruptData));
            }
            None
        };
        let modified_at = if flags & FLAG_MTIME != 0 {
            Some(timestamp_from_wire(
                get_u64(&record, 56),
                get_u32(&record, 64),
            )?)
        } else {
            if get_u64(&record, 56) != 0 || get_u32(&record, 64) != 0 {
                return Err(error(SnapshotCodecErrorKind::CorruptData));
            }
            None
        };
        let accessed_at = if flags & FLAG_ATIME != 0 {
            Some(timestamp_from_wire(
                get_u64(&record, 72),
                get_u32(&record, 80),
            )?)
        } else {
            if get_u64(&record, 72) != 0 || get_u32(&record, 80) != 0 {
                return Err(error(SnapshotCodecErrorKind::CorruptData));
            }
            None
        };
        let scan_flags = SnapshotScanFlags::from_bits(get_u32(&record, 84))?;
        let unix_identity = if flags & FLAG_UNIX_IDENTITY != 0 {
            Some(SnapshotUnixIdentity::new(
                get_u64(&record, 88),
                get_u64(&record, 96),
            ))
        } else {
            if get_u64(&record, 88) != 0 || get_u64(&record, 96) != 0 {
                return Err(error(SnapshotCodecErrorKind::CorruptData));
            }
            None
        };
        nodes
            .try_reserve(1)
            .map_err(|_| error(SnapshotCodecErrorKind::LimitExceeded))?;
        nodes.push(SnapshotNode {
            id,
            parent: match get_u64(&record, 8) {
                NO_PARENT => None,
                value => Some(value),
            },
            depth: get_u32(&record, 16),
            kind: SnapshotNodeKind::from_byte(record[20])?,
            name,
            logical_bytes: get_u64(&record, 24),
            allocated_bytes,
            file_count: get_u64(&record, 40),
            child_count: get_u64(&record, 48),
            modified_at,
            accessed_at,
            scan_flags,
            unix_identity,
        });
    }

    let consumed_payload = hashing
        .bytes
        .checked_sub(HEADER_SIZE as u64)
        .ok_or_else(|| error(SnapshotCodecErrorKind::InvalidLength))?;
    if remaining_payload != 0 || consumed_payload != payload_length {
        return Err(error(SnapshotCodecErrorKind::InvalidLength));
    }
    let (reader, actual_digest) = hashing.finish();
    let mut stored_digest = [0_u8; DIGEST_SIZE];
    read_exact_unhashed(reader, &mut stored_digest)?;
    if actual_digest != stored_digest {
        return Err(error(SnapshotCodecErrorKind::ChecksumMismatch));
    }
    let mut trailing = [0_u8; 1];
    match reader.read(&mut trailing) {
        Ok(0) => {}
        Ok(_) => return Err(error(SnapshotCodecErrorKind::InvalidLength)),
        Err(_) => return Err(error(SnapshotCodecErrorKind::Io)),
    }

    let document = SnapshotDocument {
        metadata: SnapshotMetadata {
            scan_id,
            root,
            captured_at,
            totals,
        },
        nodes,
    };
    validate_document(&document, SnapshotCodecErrorKind::CorruptData)?;
    Ok((document, SnapshotDigest(actual_digest)))
}

struct Layout {
    scan_id_length: usize,
    payload_length: u64,
}

fn validate_document(
    document: &SnapshotDocument,
    failure: SnapshotCodecErrorKind,
) -> Result<Layout, SnapshotCodecError> {
    validate_root(&document.metadata.root, failure)?;
    validate_timestamp(document.metadata.captured_at, failure)?;
    let scan_id_length = document.metadata.scan_id.as_str().len();
    if !(1..=MAX_SCAN_ID_BYTES).contains(&scan_id_length)
        || document.nodes.is_empty()
        || document.nodes.len() as u64 > MAX_SNAPSHOT_NODES
    {
        return Err(error(failure));
    }

    let mut payload_length = (scan_id_length as u64)
        .checked_add(document.metadata.root.bytes.len() as u64)
        .ok_or_else(|| error(failure))?;
    let mut directory_count = 0_u64;
    let mut regular_file_count = 0_u64;
    let mut stack: Vec<DirectoryFrame> = Vec::new();
    stack
        .try_reserve_exact((MAX_SNAPSHOT_DEPTH as usize).saturating_add(1))
        .map_err(|_| error(SnapshotCodecErrorKind::LimitExceeded))?;

    for (index, node) in document.nodes.iter().enumerate() {
        let expected_id = index as u64;
        if node.id != expected_id
            || node.depth > MAX_SNAPSHOT_DEPTH
            || !node.scan_flags.is_valid()
            || node.unix_identity.is_some()
                && document.metadata.root.encoding != HostEncoding::UnixBytes
            || node
                .scan_flags
                .contains(SnapshotScanFlags::HARD_LINK_DUPLICATE)
                && node.kind != SnapshotNodeKind::File
            || node.scan_flags.contains(SnapshotScanFlags::MOUNT_BOUNDARY)
                && node.kind != SnapshotNodeKind::Directory
        {
            return Err(error(failure));
        }
        if let Some(timestamp) = node.modified_at {
            validate_timestamp(timestamp, failure)?;
        }
        if let Some(timestamp) = node.accessed_at {
            validate_timestamp(timestamp, failure)?;
        }
        payload_length = payload_length
            .checked_add(NODE_HEADER_SIZE as u64)
            .and_then(|value| {
                value.checked_add(node.name.as_ref().map_or(0, |name| name.bytes.len()) as u64)
            })
            .ok_or_else(|| error(failure))?;

        let node_path_bytes;
        if index == 0 {
            if node.parent.is_some()
                || node.depth != 0
                || node.kind != SnapshotNodeKind::Directory
                || node.name.is_some()
            {
                return Err(error(failure));
            }
            node_path_bytes = document.metadata.root.bytes.len();
        } else {
            let name = node.name.as_ref().ok_or_else(|| error(failure))?;
            validate_component(name, failure)?;
            if name.encoding != document.metadata.root.encoding {
                return Err(error(failure));
            }
            while stack.last().is_some_and(|frame| frame.depth >= node.depth) {
                close_directory(&mut stack, failure)?;
            }
            let parent_path_bytes = stack.last().ok_or_else(|| error(failure))?.path_bytes;
            node_path_bytes = parent_path_bytes
                .checked_add(document.metadata.root.encoding.separator_bytes())
                .and_then(|value| value.checked_add(name.bytes.len()))
                .ok_or_else(|| error(failure))?;
            if node_path_bytes > MAX_PATH_BYTES {
                return Err(error(failure));
            }
            let parent = stack.last_mut().ok_or_else(|| error(failure))?;
            if node.depth != parent.depth + 1
                || node.parent != Some(parent.id)
                || parent.remaining_children == 0
            {
                return Err(error(failure));
            }
            parent.add_child(node, failure)?;
        }

        match node.kind {
            SnapshotNodeKind::Directory => {
                directory_count = directory_count
                    .checked_add(1)
                    .ok_or_else(|| error(failure))?;
                if node_path_bytes > MAX_PATH_BYTES {
                    return Err(error(failure));
                }
                stack.push(DirectoryFrame::new(node, node_path_bytes));
            }
            SnapshotNodeKind::File => {
                regular_file_count = regular_file_count
                    .checked_add(1)
                    .ok_or_else(|| error(failure))?;
                if node.file_count != 1 || node.child_count != 0 {
                    return Err(error(failure));
                }
            }
            SnapshotNodeKind::Symlink | SnapshotNodeKind::Other | SnapshotNodeKind::Error => {
                if node.file_count != 0 || node.child_count != 0 {
                    return Err(error(failure));
                }
            }
        }
    }
    while !stack.is_empty() {
        close_directory(&mut stack, failure)?;
    }

    let root = &document.nodes[0];
    if directory_count != document.metadata.totals.directory_count
        || regular_file_count != document.metadata.totals.file_count
        || root.file_count != document.metadata.totals.file_count
        || root.logical_bytes != document.metadata.totals.logical_bytes
        || root.allocated_bytes != document.metadata.totals.allocated_bytes
    {
        return Err(error(failure));
    }
    let total_file_length = (HEADER_SIZE as u64)
        .checked_add(payload_length)
        .and_then(|value| value.checked_add(DIGEST_SIZE as u64))
        .ok_or_else(|| error(failure))?;
    if total_file_length > MAX_SNAPSHOT_FILE_BYTES {
        return Err(error(SnapshotCodecErrorKind::LimitExceeded));
    }
    Ok(Layout {
        scan_id_length,
        payload_length,
    })
}

struct DirectoryFrame<'document> {
    id: u64,
    depth: u32,
    path_bytes: usize,
    expected_logical: u64,
    expected_allocated: Option<u64>,
    expected_files: u64,
    remaining_children: u64,
    observed_logical: u64,
    observed_allocated: u64,
    observed_files: u64,
    all_children_allocated: bool,
    child_names: HashSet<&'document HostValue>,
}

impl<'document> DirectoryFrame<'document> {
    fn new(node: &'document SnapshotNode, path_bytes: usize) -> Self {
        Self {
            id: node.id,
            depth: node.depth,
            path_bytes,
            expected_logical: node.logical_bytes,
            expected_allocated: node.allocated_bytes,
            expected_files: node.file_count,
            remaining_children: node.child_count,
            observed_logical: 0,
            observed_allocated: 0,
            observed_files: 0,
            all_children_allocated: true,
            child_names: HashSet::new(),
        }
    }

    fn add_child(
        &mut self,
        child: &'document SnapshotNode,
        failure: SnapshotCodecErrorKind,
    ) -> Result<(), SnapshotCodecError> {
        self.remaining_children -= 1;
        let name = child.name.as_ref().ok_or_else(|| error(failure))?;
        self.child_names
            .try_reserve(1)
            .map_err(|_| error(SnapshotCodecErrorKind::LimitExceeded))?;
        if !self.child_names.insert(name) {
            return Err(error(failure));
        }
        self.observed_logical = self
            .observed_logical
            .checked_add(child.logical_bytes)
            .ok_or_else(|| error(failure))?;
        self.observed_files = self
            .observed_files
            .checked_add(child.file_count)
            .ok_or_else(|| error(failure))?;
        if let Some(allocated) = child.allocated_bytes {
            self.observed_allocated = self
                .observed_allocated
                .checked_add(allocated)
                .ok_or_else(|| error(failure))?;
        } else {
            self.all_children_allocated = false;
        }
        Ok(())
    }
}

fn close_directory(
    stack: &mut Vec<DirectoryFrame<'_>>,
    failure: SnapshotCodecErrorKind,
) -> Result<(), SnapshotCodecError> {
    let frame = stack.pop().ok_or_else(|| error(failure))?;
    let allocated_matches = match (frame.expected_allocated, frame.all_children_allocated) {
        (Some(expected), true) => expected == frame.observed_allocated,
        (None, false) => true,
        (Some(_), false) | (None, true) => false,
    };
    if frame.remaining_children != 0
        || frame.expected_logical != frame.observed_logical
        || frame.expected_files != frame.observed_files
        || !allocated_matches
    {
        return Err(error(failure));
    }
    Ok(())
}

fn timestamp_from_wire(seconds: u64, nanos: u32) -> Result<SnapshotTimestamp, SnapshotCodecError> {
    if nanos >= 1_000_000_000 {
        return Err(error(SnapshotCodecErrorKind::CorruptData));
    }
    Ok(SnapshotTimestamp {
        seconds_since_unix_epoch: seconds,
        nanoseconds: nanos,
    })
}

fn validate_timestamp(
    timestamp: SnapshotTimestamp,
    failure: SnapshotCodecErrorKind,
) -> Result<(), SnapshotCodecError> {
    if timestamp.nanoseconds >= 1_000_000_000 {
        return Err(error(failure));
    }
    Ok(())
}

fn allocate_bytes(length: usize) -> Result<Vec<u8>, SnapshotCodecError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| error(SnapshotCodecErrorKind::LimitExceeded))?;
    bytes.resize(length, 0);
    Ok(bytes)
}

struct HashingWriter<'a, W> {
    inner: &'a mut W,
    hasher: Sha256,
    bytes: u64,
}

impl<'a, W: Write> HashingWriter<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            bytes: 0,
        }
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), SnapshotCodecError> {
        self.inner
            .write_all(bytes)
            .map_err(|_| error(SnapshotCodecErrorKind::Io))?;
        self.hasher.update(bytes);
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| error(SnapshotCodecErrorKind::LimitExceeded))?;
        Ok(())
    }

    fn finish(self) -> (&'a mut W, [u8; DIGEST_SIZE]) {
        (self.inner, self.hasher.finalize().into())
    }
}

struct HashingReader<'a, R> {
    inner: &'a mut R,
    hasher: Sha256,
    bytes: u64,
}

impl<'a, R: Read> HashingReader<'a, R> {
    fn new(inner: &'a mut R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            bytes: 0,
        }
    }

    fn read_exact(&mut self, buffer: &mut [u8]) -> Result<(), SnapshotCodecError> {
        read_exact_unhashed(self.inner, buffer)?;
        self.hasher.update(&*buffer);
        self.bytes = self
            .bytes
            .checked_add(buffer.len() as u64)
            .ok_or_else(|| error(SnapshotCodecErrorKind::LimitExceeded))?;
        Ok(())
    }

    fn finish(self) -> (&'a mut R, [u8; DIGEST_SIZE]) {
        (self.inner, self.hasher.finalize().into())
    }
}

fn read_exact_unhashed(
    reader: &mut impl Read,
    buffer: &mut [u8],
) -> Result<(), SnapshotCodecError> {
    reader.read_exact(buffer).map_err(|failure| {
        if failure.kind() == io::ErrorKind::UnexpectedEof {
            error(SnapshotCodecErrorKind::InvalidLength)
        } else {
            error(SnapshotCodecErrorKind::Io)
        }
    })
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(input[offset..offset + 4].try_into().expect("fixed field"))
}

fn get_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(input[offset..offset + 8].try_into().expect("fixed field"))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};
    use std::path::Path;

    use super::*;

    fn component(value: &str) -> HostValue {
        HostValue::from_component(OsStr::new(value)).unwrap()
    }

    fn encoded(document: &SnapshotDocument) -> Vec<u8> {
        let mut bytes = Vec::new();
        encode_snapshot(document, &mut bytes).unwrap();
        bytes
    }

    fn resign(bytes: &mut [u8]) {
        let digest_offset = bytes.len() - DIGEST_SIZE;
        let digest: [u8; DIGEST_SIZE] = Sha256::digest(&bytes[..digest_offset]).into();
        bytes[digest_offset..].copy_from_slice(&digest);
    }

    fn node_offsets(bytes: &[u8]) -> Vec<usize> {
        let node_count = usize::try_from(get_u64(bytes, 24)).unwrap();
        let mut offset = HEADER_SIZE + get_u32(bytes, 80) as usize + get_u32(bytes, 84) as usize;
        let mut offsets = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            offsets.push(offset);
            offset += NODE_HEADER_SIZE + get_u32(bytes, offset + 68) as usize;
        }
        offsets
    }

    fn assert_resigned_corrupt(mut bytes: Vec<u8>, mutate: impl FnOnce(&mut [u8])) {
        mutate(&mut bytes);
        resign(&mut bytes);
        assert_eq!(
            decode_snapshot(&mut Cursor::new(bytes)).unwrap_err().kind,
            SnapshotCodecErrorKind::CorruptData
        );
    }

    fn two_sibling_files() -> SnapshotDocument {
        SnapshotDocument {
            metadata: SnapshotMetadata {
                scan_id: ScanId::new("scan:siblings").unwrap(),
                root: HostValue::from_root(Path::new(if cfg!(windows) {
                    r"C:\siblings"
                } else {
                    "/siblings"
                }))
                .unwrap(),
                captured_at: SnapshotTimestamp::new(1_750_000_000, 0).unwrap(),
                totals: SnapshotTotals {
                    directory_count: 1,
                    file_count: 2,
                    logical_bytes: 2,
                    allocated_bytes: Some(2),
                },
            },
            nodes: vec![
                SnapshotNode {
                    id: 0,
                    parent: None,
                    depth: 0,
                    kind: SnapshotNodeKind::Directory,
                    name: None,
                    logical_bytes: 2,
                    allocated_bytes: Some(2),
                    file_count: 2,
                    child_count: 2,
                    modified_at: None,
                    accessed_at: None,
                    scan_flags: SnapshotScanFlags::NONE,
                    unix_identity: None,
                },
                SnapshotNode {
                    id: 1,
                    parent: Some(0),
                    depth: 1,
                    kind: SnapshotNodeKind::File,
                    name: Some(component("a")),
                    logical_bytes: 1,
                    allocated_bytes: Some(1),
                    file_count: 1,
                    child_count: 0,
                    modified_at: None,
                    accessed_at: None,
                    scan_flags: SnapshotScanFlags::NONE,
                    unix_identity: None,
                },
                SnapshotNode {
                    id: 2,
                    parent: Some(0),
                    depth: 1,
                    kind: SnapshotNodeKind::File,
                    name: Some(component("b")),
                    logical_bytes: 1,
                    allocated_bytes: Some(1),
                    file_count: 1,
                    child_count: 0,
                    modified_at: None,
                    accessed_at: None,
                    scan_flags: SnapshotScanFlags::NONE,
                    unix_identity: None,
                },
            ],
        }
    }

    struct FailingReader {
        inner: Cursor<Vec<u8>>,
        fail_at: u64,
    }

    impl Read for FailingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.inner.position() >= self.fail_at {
                return Err(io::Error::other("injected read failure"));
            }
            let remaining = usize::try_from(self.fail_at - self.inner.position()).unwrap();
            let amount = buffer.len().min(remaining);
            self.inner.read(&mut buffer[..amount])
        }
    }

    fn sample() -> SnapshotDocument {
        SnapshotDocument {
            metadata: SnapshotMetadata {
                scan_id: ScanId::new("scan:snapshot-one").unwrap(),
                root: HostValue::from_root(Path::new(if cfg!(windows) {
                    r"C:\snapshot-root"
                } else {
                    "/snapshot-root"
                }))
                .unwrap(),
                captured_at: SnapshotTimestamp::new(1_750_000_000, 123).unwrap(),
                totals: SnapshotTotals {
                    directory_count: 2,
                    file_count: 1,
                    logical_bytes: 10,
                    allocated_bytes: Some(16),
                },
            },
            nodes: vec![
                SnapshotNode {
                    id: 0,
                    parent: None,
                    depth: 0,
                    kind: SnapshotNodeKind::Directory,
                    name: None,
                    logical_bytes: 10,
                    allocated_bytes: Some(16),
                    file_count: 1,
                    child_count: 1,
                    modified_at: None,
                    accessed_at: None,
                    scan_flags: SnapshotScanFlags::NONE,
                    unix_identity: if cfg!(unix) {
                        Some(SnapshotUnixIdentity::new(7, 10))
                    } else {
                        None
                    },
                },
                SnapshotNode {
                    id: 1,
                    parent: Some(0),
                    depth: 1,
                    kind: SnapshotNodeKind::Directory,
                    name: Some(component("build")),
                    logical_bytes: 10,
                    allocated_bytes: Some(16),
                    file_count: 1,
                    child_count: 1,
                    modified_at: None,
                    accessed_at: None,
                    scan_flags: SnapshotScanFlags::NONE,
                    unix_identity: if cfg!(unix) {
                        Some(SnapshotUnixIdentity::new(7, 11))
                    } else {
                        None
                    },
                },
                SnapshotNode {
                    id: 2,
                    parent: Some(1),
                    depth: 2,
                    kind: SnapshotNodeKind::File,
                    name: Some(component("artifact.o")),
                    logical_bytes: 10,
                    allocated_bytes: Some(16),
                    file_count: 1,
                    child_count: 0,
                    modified_at: Some(SnapshotTimestamp::new(1_750_000_001, 456).unwrap()),
                    accessed_at: Some(SnapshotTimestamp::new(1_750_000_002, 789).unwrap()),
                    scan_flags: SnapshotScanFlags::NONE
                        .union(SnapshotScanFlags::HARD_LINK_DUPLICATE),
                    unix_identity: if cfg!(unix) {
                        Some(SnapshotUnixIdentity::new(7, 12))
                    } else {
                        None
                    },
                },
            ],
        }
    }

    #[test]
    fn round_trip_is_stable_and_checksummed() {
        #[cfg(unix)]
        const GOLDEN_LENGTH: usize = 510;
        #[cfg(unix)]
        const GOLDEN_DIGEST: [u8; DIGEST_SIZE] = [
            96, 169, 39, 248, 241, 138, 55, 41, 81, 254, 191, 84, 60, 233, 97, 249, 39, 106, 113,
            219, 191, 158, 158, 2, 34, 132, 111, 32, 202, 51, 179, 237,
        ];
        #[cfg(windows)]
        const GOLDEN_LENGTH: usize = 543;
        #[cfg(windows)]
        const GOLDEN_DIGEST: [u8; DIGEST_SIZE] = [
            158, 253, 177, 30, 154, 151, 97, 92, 51, 242, 156, 220, 233, 2, 23, 15, 219, 249, 180,
            133, 78, 47, 69, 96, 131, 238, 41, 199, 202, 141, 230, 238,
        ];

        let expected = sample();
        let mut bytes = Vec::new();
        let written_digest = encode_snapshot(&expected, &mut bytes).unwrap();
        let (actual, read_digest) = decode_snapshot(&mut Cursor::new(&bytes)).unwrap();

        assert_eq!(actual, expected);
        assert_eq!(read_digest, written_digest);
        assert_eq!(&bytes[..8], MAGIC);
        assert_eq!(bytes.len() as u64, get_u64(&bytes, 16) + 128);
        assert_eq!(bytes.len(), GOLDEN_LENGTH);
        assert_eq!(written_digest.bytes(), GOLDEN_DIGEST);
    }

    #[test]
    fn corruption_truncation_trailing_data_and_versions_fail_closed() {
        let mut bytes = Vec::new();
        encode_snapshot(&sample(), &mut bytes).unwrap();

        let mut corrupt = bytes.clone();
        let first_node = HEADER_SIZE + get_u32(&bytes, 80) as usize + get_u32(&bytes, 84) as usize;
        corrupt[first_node + 24] ^= 0x40;
        assert_eq!(
            decode_snapshot(&mut Cursor::new(corrupt)).unwrap_err().kind,
            SnapshotCodecErrorKind::ChecksumMismatch
        );

        let mut invalid_magic = bytes.clone();
        invalid_magic[0] ^= 0xff;
        resign(&mut invalid_magic);
        assert_eq!(
            decode_snapshot(&mut Cursor::new(invalid_magic))
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidMagic
        );

        let truncated = &bytes[..bytes.len() - 1];
        assert_eq!(
            decode_snapshot(&mut Cursor::new(truncated))
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidLength
        );

        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            decode_snapshot(&mut Cursor::new(trailing))
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidLength
        );

        for unsupported in [0, SNAPSHOT_FORMAT_VERSION + 1] {
            let mut incompatible = bytes.clone();
            put_u32(&mut incompatible, 8, unsupported);
            assert_eq!(
                decode_snapshot(&mut Cursor::new(incompatible))
                    .unwrap_err()
                    .kind,
                SnapshotCodecErrorKind::IncompatibleVersion
            );
        }
    }

    #[test]
    fn graph_aggregates_and_components_are_rejected_before_write() {
        let mut invalid = sample();
        invalid.nodes[1].logical_bytes += 1;
        let mut output = Vec::new();
        assert_eq!(
            encode_snapshot(&invalid, &mut output).unwrap_err().kind,
            SnapshotCodecErrorKind::InvalidInput
        );
        assert!(output.is_empty());

        assert_eq!(
            HostValue::from_component(OsStr::new("../escape"))
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidInput
        );

        let maximum_component_characters = if cfg!(windows) {
            MAX_COMPONENT_BYTES / 2
        } else {
            MAX_COMPONENT_BYTES
        };
        assert!(
            HostValue::from_component(OsStr::new(&"x".repeat(maximum_component_characters)))
                .is_ok()
        );
        assert_eq!(
            HostValue::from_component(OsStr::new(&"x".repeat(maximum_component_characters + 1),))
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidInput
        );
    }

    #[test]
    fn duplicate_names_flags_timestamps_and_allocated_shape_reject_before_write() {
        let mut duplicate = two_sibling_files();
        duplicate.nodes[2].name = duplicate.nodes[1].name.clone();
        assert_eq!(
            encode_snapshot(&duplicate, &mut Vec::new())
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidInput
        );

        let mut unknown_flags = sample();
        unknown_flags.nodes[0].scan_flags = SnapshotScanFlags(u32::MAX);
        assert_eq!(
            encode_snapshot(&unknown_flags, &mut Vec::new())
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidInput
        );

        let mut invalid_flag_kind = sample();
        invalid_flag_kind.nodes[0].scan_flags = SnapshotScanFlags::HARD_LINK_DUPLICATE;
        assert_eq!(
            encode_snapshot(&invalid_flag_kind, &mut Vec::new())
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidInput
        );
        invalid_flag_kind.nodes[0].scan_flags = SnapshotScanFlags::NONE;
        invalid_flag_kind.nodes[2].scan_flags = SnapshotScanFlags::MOUNT_BOUNDARY;
        assert_eq!(
            encode_snapshot(&invalid_flag_kind, &mut Vec::new())
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidInput
        );

        let mut invalid_time = sample();
        invalid_time.nodes[2].accessed_at = Some(SnapshotTimestamp {
            seconds_since_unix_epoch: 0,
            nanoseconds: 1_000_000_000,
        });
        assert_eq!(
            encode_snapshot(&invalid_time, &mut Vec::new())
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidInput
        );

        let mut parent_unknown_with_known_children = sample();
        parent_unknown_with_known_children.nodes[1].allocated_bytes = None;
        parent_unknown_with_known_children.nodes[0].allocated_bytes = None;
        parent_unknown_with_known_children
            .metadata
            .totals
            .allocated_bytes = None;
        assert_eq!(
            encode_snapshot(&parent_unknown_with_known_children, &mut Vec::new())
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidInput
        );

        let mut canonical_unknown = sample();
        canonical_unknown.nodes[2].allocated_bytes = None;
        canonical_unknown.nodes[1].allocated_bytes = None;
        canonical_unknown.nodes[0].allocated_bytes = None;
        canonical_unknown.metadata.totals.allocated_bytes = None;
        let bytes = encoded(&canonical_unknown);
        assert_eq!(
            decode_snapshot(&mut Cursor::new(bytes)).unwrap().0,
            canonical_unknown
        );

        let mut empty = two_sibling_files();
        empty.metadata.totals.file_count = 0;
        empty.metadata.totals.logical_bytes = 0;
        empty.metadata.totals.allocated_bytes = None;
        empty.nodes.truncate(1);
        empty.nodes[0].logical_bytes = 0;
        empty.nodes[0].allocated_bytes = None;
        empty.nodes[0].file_count = 0;
        empty.nodes[0].child_count = 0;
        assert_eq!(
            encode_snapshot(&empty, &mut Vec::new()).unwrap_err().kind,
            SnapshotCodecErrorKind::InvalidInput
        );
        empty.metadata.totals.allocated_bytes = Some(0);
        empty.nodes[0].allocated_bytes = Some(0);
        assert!(encode_snapshot(&empty, &mut Vec::new()).is_ok());
    }

    #[test]
    fn hostile_lengths_and_checksum_valid_semantic_corruption_are_bounded() {
        let mut bytes = Vec::new();
        encode_snapshot(&sample(), &mut bytes).unwrap();

        let mut oversized_nodes = bytes.clone();
        put_u64(&mut oversized_nodes, 24, MAX_SNAPSHOT_NODES + 1);
        assert_eq!(
            decode_snapshot(&mut Cursor::new(oversized_nodes))
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::LimitExceeded
        );

        let mut oversized_root = bytes.clone();
        put_u32(&mut oversized_root, 84, (MAX_ROOT_BYTES + 1) as u32);
        assert_eq!(
            decode_snapshot(&mut Cursor::new(oversized_root))
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::LimitExceeded
        );

        let mut short_payload = bytes.clone();
        let declared = get_u64(&short_payload, 16);
        put_u64(&mut short_payload, 16, declared - 1);
        assert_eq!(
            decode_snapshot(&mut Cursor::new(short_payload))
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidLength
        );

        let scan_and_root = get_u32(&bytes, 80) as u64 + get_u32(&bytes, 84) as u64;
        let mut huge_claim_without_body =
            bytes[..HEADER_SIZE + usize::try_from(scan_and_root).unwrap()].to_vec();
        put_u64(&mut huge_claim_without_body, 24, MAX_SNAPSHOT_NODES);
        put_u64(
            &mut huge_claim_without_body,
            16,
            scan_and_root + MAX_SNAPSHOT_NODES * NODE_HEADER_SIZE as u64,
        );
        assert_eq!(
            decode_snapshot(&mut Cursor::new(huge_claim_without_body))
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidLength
        );

        let first_node = HEADER_SIZE + get_u32(&bytes, 80) as usize + get_u32(&bytes, 84) as usize;
        put_u64(&mut bytes, first_node + 40, 2);
        resign(&mut bytes);
        assert_eq!(
            decode_snapshot(&mut Cursor::new(bytes)).unwrap_err().kind,
            SnapshotCodecErrorKind::CorruptData
        );
    }

    #[test]
    fn checksum_valid_unknown_flags_reserved_bytes_and_timestamps_fail_closed() {
        let bytes = encoded(&sample());
        let nodes = node_offsets(&bytes);
        let root = nodes[0];
        let file = nodes[2];

        assert_resigned_corrupt(bytes.clone(), |wire| wire[76] |= 1 << 7);
        assert_resigned_corrupt(bytes.clone(), |wire| wire[78] = 1);
        assert_resigned_corrupt(bytes.clone(), |wire| wire[88] = 1);
        assert_resigned_corrupt(bytes.clone(), |wire| wire[root + 21] |= 1 << 7);
        assert_resigned_corrupt(bytes.clone(), |wire| put_u32(wire, root + 84, 1 << 31));
        assert_resigned_corrupt(bytes.clone(), |wire| wire[root + 23] = 1);
        assert_resigned_corrupt(bytes.clone(), |wire| wire[root + 104] = 1);
        assert_resigned_corrupt(bytes.clone(), |wire| {
            wire[root + 21] &= !FLAG_UNIX_IDENTITY;
            put_u64(wire, root + 88, 1);
        });
        assert_resigned_corrupt(bytes.clone(), |wire| put_u64(wire, root + 72, 1));
        assert_resigned_corrupt(bytes.clone(), |wire| {
            wire[file + 21] &= !FLAG_ALLOCATED;
        });
        assert_resigned_corrupt(bytes.clone(), |wire| {
            wire[file + 21] &= !FLAG_MTIME;
        });
        assert_resigned_corrupt(bytes.clone(), |wire| {
            put_u32(wire, file + 64, 1_000_000_000)
        });
        assert_resigned_corrupt(bytes.clone(), |wire| {
            put_u32(wire, file + 80, 1_000_000_000)
        });
        assert_resigned_corrupt(bytes, |wire| put_u32(wire, 72, 1_000_000_000));
    }

    #[test]
    fn checksum_valid_graph_kind_encoding_and_flag_semantics_fail_closed() {
        let bytes = encoded(&sample());
        let nodes = node_offsets(&bytes);
        let root = nodes[0];
        let directory = nodes[1];
        let file = nodes[2];

        assert_resigned_corrupt(bytes.clone(), |wire| put_u64(wire, file + 8, 0));
        assert_resigned_corrupt(bytes.clone(), |wire| put_u32(wire, file + 16, 3));
        assert_resigned_corrupt(bytes.clone(), |wire| put_u64(wire, root + 48, 2));
        assert_resigned_corrupt(bytes.clone(), |wire| wire[file + 20] = 99);
        assert_resigned_corrupt(bytes.clone(), |wire| wire[directory + 22] = 99);
        assert_resigned_corrupt(bytes.clone(), |wire| wire[77] = 99);
        assert_resigned_corrupt(bytes.clone(), |wire| {
            put_u32(
                wire,
                directory + 84,
                SnapshotScanFlags::HARD_LINK_DUPLICATE.bits(),
            );
        });
        assert_resigned_corrupt(bytes.clone(), |wire| {
            put_u32(wire, file + 84, SnapshotScanFlags::MOUNT_BOUNDARY.bits());
        });
        assert_resigned_corrupt(bytes.clone(), |wire| {
            wire[directory + 21] &= !FLAG_ALLOCATED;
            put_u64(wire, directory + 32, 0);
        });
        assert_resigned_corrupt(bytes, |wire| {
            put_u32(wire, file + 16, MAX_SNAPSHOT_DEPTH + 1);
        });
    }

    #[test]
    fn checksum_valid_duplicate_sibling_path_fails_closed() {
        let bytes = encoded(&two_sibling_files());
        let nodes = node_offsets(&bytes);
        let first_name = nodes[1] + NODE_HEADER_SIZE;
        let second_name = nodes[2] + NODE_HEADER_SIZE;
        let name_length = get_u32(&bytes, nodes[1] + 68) as usize;

        assert_resigned_corrupt(bytes, |wire| {
            wire.copy_within(first_name..first_name + name_length, second_name);
        });
    }

    #[test]
    fn component_depth_and_cumulative_path_limits_fail_closed() {
        let bytes = encoded(&sample());
        let file = node_offsets(&bytes)[2];
        let mut oversized_component = bytes;
        put_u32(
            &mut oversized_component,
            file + 68,
            (MAX_COMPONENT_BYTES + 1) as u32,
        );
        assert_eq!(
            decode_snapshot(&mut Cursor::new(oversized_component))
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::LimitExceeded
        );

        let root_characters = if cfg!(windows) {
            MAX_PATH_BYTES / 2 - 3
        } else {
            MAX_PATH_BYTES - 1
        };
        let root_text = if cfg!(windows) {
            format!(r"C:\{}", "x".repeat(root_characters))
        } else {
            format!("/{}", "x".repeat(root_characters))
        };
        let mut too_deep_path = sample();
        too_deep_path.metadata.root = HostValue::from_root(Path::new(&root_text)).unwrap();
        assert_eq!(
            encode_snapshot(&too_deep_path, &mut Vec::new())
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidInput
        );
    }

    #[test]
    fn short_and_non_eof_reader_failures_are_distinct() {
        let bytes = encoded(&sample());
        assert_eq!(
            decode_snapshot(&mut Cursor::new(&bytes[..HEADER_SIZE - 1]))
                .unwrap_err()
                .kind,
            SnapshotCodecErrorKind::InvalidLength
        );

        let mut failing = FailingReader {
            inner: Cursor::new(bytes),
            fail_at: HEADER_SIZE as u64 + 1,
        };
        assert_eq!(
            decode_snapshot(&mut failing).unwrap_err().kind,
            SnapshotCodecErrorKind::Io
        );
    }
}
