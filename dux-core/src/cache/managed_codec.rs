//! Bounded wire format for the marker-owned managed scan cache.
//!
//! This format is intentionally unrelated to the legacy `DUXC`/v7 cache. A
//! managed reader accepts only this envelope and never falls back to legacy
//! decoding after any failure. All attacker-controlled lengths are checked
//! before allocation.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use super::{CACHE_VERSION, CacheMetadata, CachedScanConfig};
use crate::tree::{
    DiskTree, ManagedCacheNodeRecord, ManagedCacheTreeValidationError, NodeKind, TreeNode,
};

const MANAGED_CACHE_MAGIC: [u8; 8] = *b"DUXMCACH";
const MANAGED_CACHE_WIRE_VERSION: u32 = 1;
const MANAGED_CACHE_KEY_DOMAIN: &[u8] = b"dux-managed-scan-cache-key-v1\0";
pub(crate) const MANAGED_CACHE_HEADER_BYTES: usize = 64;
// The app's largest bounded home scan is 200k nodes. The aggregate limits
// admit ordinary names and paths at that ceiling while keeping the modeled
// decode peak below 192 MiB, including the input file and validation scratch.
pub(crate) const MAX_MANAGED_CACHE_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_MANAGED_CACHE_NODES: usize = 200_000;
pub(crate) const MAX_MANAGED_CACHE_METADATA_BYTES: usize = 8 * 1024;
pub(crate) const MAX_MANAGED_CACHE_ROOT_BYTES: usize = 4_096;
pub(crate) const MAX_MANAGED_CACHE_COMPONENT_BYTES: usize = 1_024;
pub(crate) const MAX_MANAGED_CACHE_TOTAL_NAME_BYTES: usize = 24 * 1024 * 1024;
pub(crate) const MAX_MANAGED_CACHE_TOTAL_PATH_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_MANAGED_CACHE_RESIDENT_BYTES: usize = 192 * 1024 * 1024;
pub(crate) const MAX_MANAGED_CACHE_TREE_DEPTH: u16 = 512;

const MAGIC_OFFSET: usize = 0;
const VERSION_OFFSET: usize = 8;
const HEADER_LENGTH_OFFSET: usize = 12;
const BODY_LENGTH_OFFSET: usize = 16;
const NODE_COUNT_OFFSET: usize = 24;
const BODY_DIGEST_OFFSET: usize = 32;
const BINDING_BYTES: usize = 32;
const METADATA_LENGTH_BYTES: usize = 4;
const MINIMUM_METADATA_BYTES: usize = 59;
const FIXED_NODE_RECORD_BYTES: usize = 57;
const MINIMUM_BODY_BYTES: usize =
    BINDING_BYTES + METADATA_LENGTH_BYTES + MINIMUM_METADATA_BYTES + FIXED_NODE_RECORD_BYTES;
const NO_PARENT: u64 = u64::MAX;
const DECODE_AUXILIARY_BYTES_PER_NODE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ManagedCacheEncodeShape {
    node_count: usize,
    metadata_bytes: usize,
    file_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ManagedCacheEntryKey([u8; 32]);

impl ManagedCacheEntryKey {
    pub(crate) const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub(crate) fn to_lower_hex(self) -> String {
        let mut output = String::with_capacity(64);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in self.0 {
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        output
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManagedCacheCodecErrorKind {
    InvalidHeader,
    UnsupportedVersion,
    LimitExceeded,
    ChecksumMismatch,
    BindingMismatch,
    InvalidMetadata,
    InvalidTree,
    AllocationFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("managed scan-cache bytes are invalid: {kind:?}")]
pub(crate) struct ManagedCacheCodecError {
    pub(crate) kind: ManagedCacheCodecErrorKind,
}

fn error(kind: ManagedCacheCodecErrorKind) -> ManagedCacheCodecError {
    ManagedCacheCodecError { kind }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ManagedCachePreflight {
    body_bytes: usize,
    file_bytes: usize,
    node_count: usize,
}

impl ManagedCachePreflight {
    pub(crate) const fn file_bytes(self) -> usize {
        self.file_bytes
    }
}

#[derive(Debug)]
pub(crate) struct ManagedCacheDocument {
    metadata: CacheMetadata,
    tree: DiskTree,
}

impl ManagedCacheDocument {
    pub(crate) fn into_parts(self) -> (CacheMetadata, DiskTree) {
        (self.metadata, self.tree)
    }
}

pub(crate) fn managed_cache_entry_key(
    canonical_root: &Path,
    config: &CachedScanConfig,
) -> Result<ManagedCacheEntryKey, ManagedCacheCodecError> {
    validate_canonical_root(canonical_root)?;
    let max_depth = config
        .max_depth
        .map(u64::try_from)
        .transpose()
        .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    if max_depth.is_some_and(|depth| depth > u64::from(MAX_MANAGED_CACHE_TREE_DEPTH)) {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }

    let mut digest = Sha256::new();
    digest.update(MANAGED_CACHE_KEY_DOMAIN);
    update_path_digest(&mut digest, canonical_root)?;
    digest.update([u8::from(config.follow_symlinks)]);
    digest.update([u8::from(config.same_filesystem)]);
    match max_depth {
        Some(depth) => {
            digest.update([1]);
            digest.update(depth.to_le_bytes());
        }
        None => digest.update([0]),
    }
    Ok(ManagedCacheEntryKey(digest.finalize().into()))
}

pub(crate) fn encode_managed_cache(
    canonical_root: &Path,
    config: &CachedScanConfig,
    metadata: &CacheMetadata,
    tree: &DiskTree,
) -> Result<Vec<u8>, ManagedCacheCodecError> {
    let key = managed_cache_entry_key(canonical_root, config)?;
    validate_metadata(metadata, canonical_root, config)?;
    validate_tree(tree, canonical_root)?;
    validate_metadata_tree_binding(metadata, tree)?;
    let shape = preflight_encode_shape(tree, canonical_root)?;

    let mut metadata_writer = Writer::new();
    metadata_writer.write_u32(metadata.version)?;
    write_native_path(&mut metadata_writer, &metadata.root_path)?;
    let config_flags = u8::from(metadata.config.follow_symlinks)
        | (u8::from(metadata.config.same_filesystem) << 1);
    metadata_writer.write_u8(config_flags)?;
    match metadata.config.max_depth {
        Some(depth) => {
            metadata_writer.write_u8(1)?;
            metadata_writer.write_u64(
                u64::try_from(depth)
                    .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?,
            )?;
        }
        None => {
            metadata_writer.write_u8(0)?;
            metadata_writer.write_u64(0)?;
        }
    }
    write_timestamp(
        &mut metadata_writer,
        metadata.scan_time,
        ManagedCacheCodecErrorKind::InvalidMetadata,
    )?;
    write_timestamp(
        &mut metadata_writer,
        metadata.root_mtime,
        ManagedCacheCodecErrorKind::InvalidMetadata,
    )?;
    metadata_writer.write_u64(metadata.total_size)?;
    metadata_writer.write_u64(
        u64::try_from(metadata.node_count)
            .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?,
    )?;
    let metadata_bytes = metadata_writer.into_vec();
    if metadata_bytes.len() != shape.metadata_bytes {
        return Err(error(ManagedCacheCodecErrorKind::InvalidMetadata));
    }

    let ordinals = tree.managed_cache_ordinals().map_err(map_tree_error)?;
    let mut output = Writer::with_capacity(shape.file_bytes)?;
    output.write_zeroes(MANAGED_CACHE_HEADER_BYTES)?;
    output.write_bytes(key.as_bytes())?;
    output.write_u32(
        u32::try_from(metadata_bytes.len())
            .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?,
    )?;
    output.write_bytes(&metadata_bytes)?;
    for node in tree.iter() {
        let ordinal = *ordinals
            .get(node.id.index())
            .ok_or_else(|| error(ManagedCacheCodecErrorKind::InvalidTree))?;
        write_node_record(&mut output, node, ordinal)?;
    }

    let body_length = output
        .len()
        .checked_sub(MANAGED_CACHE_HEADER_BYTES)
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::InvalidHeader))?;
    let file_length = output.len();
    if body_length < MINIMUM_BODY_BYTES || file_length != shape.file_bytes {
        return Err(error(ManagedCacheCodecErrorKind::InvalidHeader));
    }
    let body_digest: [u8; 32] =
        Sha256::digest(&output.as_slice()[MANAGED_CACHE_HEADER_BYTES..]).into();
    let node_count = u64::try_from(shape.node_count)
        .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    let bytes = output.as_mut_slice();
    bytes[MAGIC_OFFSET..VERSION_OFFSET].copy_from_slice(&MANAGED_CACHE_MAGIC);
    bytes[VERSION_OFFSET..HEADER_LENGTH_OFFSET]
        .copy_from_slice(&MANAGED_CACHE_WIRE_VERSION.to_le_bytes());
    bytes[HEADER_LENGTH_OFFSET..BODY_LENGTH_OFFSET]
        .copy_from_slice(&(MANAGED_CACHE_HEADER_BYTES as u32).to_le_bytes());
    bytes[BODY_LENGTH_OFFSET..NODE_COUNT_OFFSET]
        .copy_from_slice(&(body_length as u64).to_le_bytes());
    bytes[NODE_COUNT_OFFSET..BODY_DIGEST_OFFSET].copy_from_slice(&node_count.to_le_bytes());
    bytes[BODY_DIGEST_OFFSET..MANAGED_CACHE_HEADER_BYTES].copy_from_slice(&body_digest);
    Ok(output.into_vec())
}

/// Validate the fixed header and handle-derived file size before a storage
/// caller allocates the body buffer.
pub(crate) fn preflight_managed_cache_header(
    header: &[u8],
    file_length: u64,
) -> Result<ManagedCachePreflight, ManagedCacheCodecError> {
    if header.len() != MANAGED_CACHE_HEADER_BYTES
        || header[MAGIC_OFFSET..VERSION_OFFSET] != MANAGED_CACHE_MAGIC
    {
        return Err(error(ManagedCacheCodecErrorKind::InvalidHeader));
    }
    let version = read_header_u32(header, VERSION_OFFSET)?;
    if version != MANAGED_CACHE_WIRE_VERSION {
        return Err(error(ManagedCacheCodecErrorKind::UnsupportedVersion));
    }
    if read_header_u32(header, HEADER_LENGTH_OFFSET)? != MANAGED_CACHE_HEADER_BYTES as u32 {
        return Err(error(ManagedCacheCodecErrorKind::InvalidHeader));
    }
    if file_length > MAX_MANAGED_CACHE_FILE_BYTES {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }
    let body_length_u64 = read_header_u64(header, BODY_LENGTH_OFFSET)?;
    let expected_file_length = u64::try_from(MANAGED_CACHE_HEADER_BYTES)
        .ok()
        .and_then(|header_bytes| header_bytes.checked_add(body_length_u64))
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    if expected_file_length != file_length || body_length_u64 < MINIMUM_BODY_BYTES as u64 {
        return Err(error(ManagedCacheCodecErrorKind::InvalidHeader));
    }
    let node_count_u64 = read_header_u64(header, NODE_COUNT_OFFSET)?;
    if node_count_u64 == 0 || node_count_u64 > MAX_MANAGED_CACHE_NODES as u64 {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }
    let minimum_records = node_count_u64
        .checked_mul(FIXED_NODE_RECORD_BYTES as u64)
        .and_then(|bytes| {
            bytes.checked_add(
                (BINDING_BYTES + METADATA_LENGTH_BYTES + MINIMUM_METADATA_BYTES) as u64,
            )
        })
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    if body_length_u64 < minimum_records {
        return Err(error(ManagedCacheCodecErrorKind::InvalidHeader));
    }
    let body_bytes = usize::try_from(body_length_u64)
        .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    let file_bytes = usize::try_from(file_length)
        .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    let node_count = usize::try_from(node_count_u64)
        .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    validate_resident_budget(file_bytes, node_count, 0, 0)?;
    Ok(ManagedCachePreflight {
        body_bytes,
        file_bytes,
        node_count,
    })
}

pub(crate) fn decode_managed_cache(
    bytes: &[u8],
    canonical_root: &Path,
    config: &CachedScanConfig,
) -> Result<ManagedCacheDocument, ManagedCacheCodecError> {
    let file_length =
        u64::try_from(bytes.len()).map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    let header = bytes
        .get(..MANAGED_CACHE_HEADER_BYTES)
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::InvalidHeader))?;
    let preflight = preflight_managed_cache_header(header, file_length)?;
    let body = bytes
        .get(MANAGED_CACHE_HEADER_BYTES..)
        .filter(|body| body.len() == preflight.body_bytes)
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::InvalidHeader))?;
    let expected_digest = header
        .get(BODY_DIGEST_OFFSET..MANAGED_CACHE_HEADER_BYTES)
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::InvalidHeader))?;
    if Sha256::digest(body).as_slice() != expected_digest {
        return Err(error(ManagedCacheCodecErrorKind::ChecksumMismatch));
    }

    let expected_key = managed_cache_entry_key(canonical_root, config)?;
    let mut reader = Reader::new(body);
    if reader.take(BINDING_BYTES, ManagedCacheCodecErrorKind::InvalidHeader)?
        != expected_key.as_bytes()
    {
        return Err(error(ManagedCacheCodecErrorKind::BindingMismatch));
    }
    let metadata_length =
        usize::try_from(reader.read_u32(ManagedCacheCodecErrorKind::InvalidHeader)?)
            .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    if !(MINIMUM_METADATA_BYTES..=MAX_MANAGED_CACHE_METADATA_BYTES).contains(&metadata_length) {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }
    let metadata_bytes = reader.take(metadata_length, ManagedCacheCodecErrorKind::InvalidHeader)?;
    let metadata = decode_metadata(metadata_bytes, canonical_root, config, preflight.node_count)?;

    let mut records = Vec::new();
    records
        .try_reserve_exact(preflight.node_count)
        .map_err(|_| error(ManagedCacheCodecErrorKind::AllocationFailed))?;
    let mut total_name_bytes = 0_usize;
    for index in 0..preflight.node_count {
        let record = read_node_record(&mut reader, index, &mut total_name_bytes)?;
        records.push(record);
    }
    validate_resident_budget(
        preflight.file_bytes,
        preflight.node_count,
        total_name_bytes,
        0,
    )?;
    if !reader.is_empty() {
        return Err(error(ManagedCacheCodecErrorKind::InvalidTree));
    }
    let total_path_bytes = validate_decoded_path_budget(canonical_root, &records)?;
    validate_resident_budget(
        preflight.file_bytes,
        preflight.node_count,
        total_name_bytes,
        total_path_bytes,
    )?;
    let owned_root = try_clone_native_path(canonical_root)?;
    let tree = DiskTree::from_managed_cache_records(owned_root, records).map_err(map_tree_error)?;
    validate_tree(&tree, canonical_root)?;
    validate_metadata_tree_binding(&metadata, &tree)?;
    Ok(ManagedCacheDocument { metadata, tree })
}

fn decode_metadata(
    bytes: &[u8],
    canonical_root: &Path,
    config: &CachedScanConfig,
    header_node_count: usize,
) -> Result<CacheMetadata, ManagedCacheCodecError> {
    let mut reader = Reader::new(bytes);
    let version = reader.read_u32(ManagedCacheCodecErrorKind::InvalidMetadata)?;
    let root_path = read_native_path(&mut reader)?;
    let config_flags = reader.read_u8(ManagedCacheCodecErrorKind::InvalidMetadata)?;
    if config_flags & !0b11 != 0 {
        return Err(error(ManagedCacheCodecErrorKind::InvalidMetadata));
    }
    let depth_tag = reader.read_u8(ManagedCacheCodecErrorKind::InvalidMetadata)?;
    let depth_value = reader.read_u64(ManagedCacheCodecErrorKind::InvalidMetadata)?;
    let max_depth = match depth_tag {
        0 if depth_value == 0 => None,
        1 if depth_value <= u64::from(MAX_MANAGED_CACHE_TREE_DEPTH) => Some(
            usize::try_from(depth_value)
                .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?,
        ),
        1 => return Err(error(ManagedCacheCodecErrorKind::LimitExceeded)),
        _ => return Err(error(ManagedCacheCodecErrorKind::InvalidMetadata)),
    };
    let scan_time = read_timestamp(&mut reader, ManagedCacheCodecErrorKind::InvalidMetadata)?;
    let root_mtime = read_timestamp(&mut reader, ManagedCacheCodecErrorKind::InvalidMetadata)?;
    let total_size = reader.read_u64(ManagedCacheCodecErrorKind::InvalidMetadata)?;
    let encoded_node_count = reader.read_u64(ManagedCacheCodecErrorKind::InvalidMetadata)?;
    if !reader.is_empty()
        || encoded_node_count
            != u64::try_from(header_node_count)
                .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?
    {
        return Err(error(ManagedCacheCodecErrorKind::InvalidMetadata));
    }
    let metadata = CacheMetadata {
        version,
        root_path,
        scan_time,
        root_mtime,
        total_size,
        node_count: header_node_count,
        config: CachedScanConfig {
            follow_symlinks: config_flags & 1 != 0,
            same_filesystem: config_flags & 2 != 0,
            max_depth,
        },
    };
    validate_metadata(&metadata, canonical_root, config)?;
    Ok(metadata)
}

fn write_node_record(
    writer: &mut Writer,
    node: &TreeNode,
    sibling_ordinal: u32,
) -> Result<(), ManagedCacheCodecError> {
    if node.name.len() > MAX_MANAGED_CACHE_COMPONENT_BYTES {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }
    writer.write_u64(
        u64::try_from(node.id.index())
            .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?,
    )?;
    writer.write_u64(
        node.parent
            .map(|parent| u64::try_from(parent.index()))
            .transpose()
            .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?
            .unwrap_or(NO_PARENT),
    )?;
    writer.write_u32(sibling_ordinal)?;
    writer.write_u8(kind_to_wire(node.kind))?;
    writer.write_u8(u8::from(node.path_is_symlink))?;
    writer.write_u16(node.depth)?;
    writer.write_u64(node.size)?;
    writer.write_u64(node.file_count)?;
    match node.mtime {
        Some(mtime) => {
            writer.write_u8(1)?;
            write_timestamp(writer, mtime, ManagedCacheCodecErrorKind::InvalidTree)?;
        }
        None => {
            writer.write_u8(0)?;
            writer.write_u64(0)?;
            writer.write_u32(0)?;
        }
    }
    writer.write_u32(
        u32::try_from(node.name.len())
            .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?,
    )?;
    writer.write_bytes(node.name.as_bytes())
}

fn read_node_record(
    reader: &mut Reader<'_>,
    expected_index: usize,
    total_name_bytes: &mut usize,
) -> Result<ManagedCacheNodeRecord, ManagedCacheCodecError> {
    let id = reader.read_u64(ManagedCacheCodecErrorKind::InvalidTree)?;
    if id
        != u64::try_from(expected_index)
            .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?
    {
        return Err(error(ManagedCacheCodecErrorKind::InvalidTree));
    }
    let parent_value = reader.read_u64(ManagedCacheCodecErrorKind::InvalidTree)?;
    let parent = if parent_value == NO_PARENT {
        None
    } else {
        Some(parent_value)
    };
    if (expected_index == 0 && parent.is_some())
        || (expected_index != 0 && parent.is_none_or(|value| value >= id))
    {
        return Err(error(ManagedCacheCodecErrorKind::InvalidTree));
    }
    let sibling_ordinal = reader.read_u32(ManagedCacheCodecErrorKind::InvalidTree)?;
    let kind = kind_from_wire(reader.read_u8(ManagedCacheCodecErrorKind::InvalidTree)?)?;
    let flags = reader.read_u8(ManagedCacheCodecErrorKind::InvalidTree)?;
    if flags & !1 != 0 {
        return Err(error(ManagedCacheCodecErrorKind::InvalidTree));
    }
    let depth = reader.read_u16(ManagedCacheCodecErrorKind::InvalidTree)?;
    if depth > MAX_MANAGED_CACHE_TREE_DEPTH {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }
    let size = reader.read_u64(ManagedCacheCodecErrorKind::InvalidTree)?;
    let file_count = reader.read_u64(ManagedCacheCodecErrorKind::InvalidTree)?;
    let mtime_tag = reader.read_u8(ManagedCacheCodecErrorKind::InvalidTree)?;
    let mtime_seconds = reader.read_u64(ManagedCacheCodecErrorKind::InvalidTree)?;
    let mtime_nanoseconds = reader.read_u32(ManagedCacheCodecErrorKind::InvalidTree)?;
    let mtime = match mtime_tag {
        0 if mtime_seconds == 0 && mtime_nanoseconds == 0 => None,
        1 => Some(system_time_from_parts(
            mtime_seconds,
            mtime_nanoseconds,
            ManagedCacheCodecErrorKind::InvalidTree,
        )?),
        _ => return Err(error(ManagedCacheCodecErrorKind::InvalidTree)),
    };
    let name_length = usize::try_from(reader.read_u32(ManagedCacheCodecErrorKind::InvalidTree)?)
        .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    if name_length > MAX_MANAGED_CACHE_COMPONENT_BYTES {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }
    *total_name_bytes = total_name_bytes
        .checked_add(name_length)
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    if *total_name_bytes > MAX_MANAGED_CACHE_TOTAL_NAME_BYTES {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }
    let name_bytes = reader.take(name_length, ManagedCacheCodecErrorKind::InvalidTree)?;
    let name_text = std::str::from_utf8(name_bytes)
        .map_err(|_| error(ManagedCacheCodecErrorKind::InvalidTree))?;
    if expected_index != 0 {
        let mut components = Path::new(name_text).components();
        if name_text.is_empty()
            || name_text.chars().any(char::is_control)
            || !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(error(ManagedCacheCodecErrorKind::InvalidTree));
        }
    }
    let mut name = String::new();
    name.try_reserve_exact(name_length)
        .map_err(|_| error(ManagedCacheCodecErrorKind::AllocationFailed))?;
    name.push_str(name_text);
    Ok(ManagedCacheNodeRecord {
        id,
        parent,
        sibling_ordinal,
        name,
        kind,
        size,
        file_count,
        depth,
        mtime,
        path_is_symlink: flags & 1 != 0,
    })
}

fn validate_decoded_path_budget(
    canonical_root: &Path,
    records: &[ManagedCacheNodeRecord],
) -> Result<usize, ManagedCacheCodecError> {
    let root_bytes = native_path_bytes(canonical_root);
    let mut path_bytes: Vec<usize> = Vec::new();
    path_bytes
        .try_reserve_exact(records.len())
        .map_err(|_| error(ManagedCacheCodecErrorKind::AllocationFailed))?;
    path_bytes.push(root_bytes);
    let mut total_path_bytes = root_bytes;
    for (index, record) in records.iter().enumerate().skip(1) {
        let parent = usize::try_from(
            record
                .parent
                .ok_or_else(|| error(ManagedCacheCodecErrorKind::InvalidTree))?,
        )
        .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
        if parent >= index {
            return Err(error(ManagedCacheCodecErrorKind::InvalidTree));
        }
        let length = path_bytes[parent]
            .checked_add(native_separator_bytes())
            .and_then(|length| length.checked_add(native_path_bytes(Path::new(&record.name))))
            .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
        if length > MAX_MANAGED_CACHE_ROOT_BYTES {
            return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
        }
        total_path_bytes = total_path_bytes
            .checked_add(length)
            .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
        if total_path_bytes > MAX_MANAGED_CACHE_TOTAL_PATH_BYTES {
            return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
        }
        path_bytes.push(length);
    }
    Ok(total_path_bytes)
}

fn preflight_encode_shape(
    tree: &DiskTree,
    canonical_root: &Path,
) -> Result<ManagedCacheEncodeShape, ManagedCacheCodecError> {
    let node_count = tree.len();
    let metadata_bytes = MINIMUM_METADATA_BYTES
        .checked_add(native_path_bytes(canonical_root))
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    if metadata_bytes > MAX_MANAGED_CACHE_METADATA_BYTES {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }

    let mut path_bytes: Vec<usize> = Vec::new();
    path_bytes
        .try_reserve_exact(node_count)
        .map_err(|_| error(ManagedCacheCodecErrorKind::AllocationFailed))?;
    let mut total_name_bytes = 0_usize;
    let mut total_path_bytes = 0_usize;
    for node in tree.iter() {
        let index = node.id.index();
        if index != path_bytes.len() {
            return Err(error(ManagedCacheCodecErrorKind::InvalidTree));
        }
        total_name_bytes = total_name_bytes
            .checked_add(node.name.len())
            .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
        if total_name_bytes > MAX_MANAGED_CACHE_TOTAL_NAME_BYTES {
            return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
        }
        let path_length = if index == 0 {
            native_path_bytes(canonical_root)
        } else {
            let parent = node
                .parent
                .ok_or_else(|| error(ManagedCacheCodecErrorKind::InvalidTree))?
                .index();
            path_bytes
                .get(parent)
                .copied()
                .ok_or_else(|| error(ManagedCacheCodecErrorKind::InvalidTree))?
                .checked_add(native_separator_bytes())
                .and_then(|length| length.checked_add(native_path_bytes(Path::new(&node.name))))
                .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?
        };
        if path_length > MAX_MANAGED_CACHE_ROOT_BYTES {
            return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
        }
        total_path_bytes = total_path_bytes
            .checked_add(path_length)
            .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
        if total_path_bytes > MAX_MANAGED_CACHE_TOTAL_PATH_BYTES {
            return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
        }
        path_bytes.push(path_length);
    }

    let file_bytes = MANAGED_CACHE_HEADER_BYTES
        .checked_add(BINDING_BYTES)
        .and_then(|length| length.checked_add(METADATA_LENGTH_BYTES))
        .and_then(|length| length.checked_add(metadata_bytes))
        .and_then(|length| length.checked_add(node_count.checked_mul(FIXED_NODE_RECORD_BYTES)?))
        .and_then(|length| length.checked_add(total_name_bytes))
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    if u64::try_from(file_bytes).map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?
        > MAX_MANAGED_CACHE_FILE_BYTES
    {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }
    validate_resident_budget(file_bytes, node_count, total_name_bytes, total_path_bytes)?;
    Ok(ManagedCacheEncodeShape {
        node_count,
        metadata_bytes,
        file_bytes,
    })
}

fn validate_resident_budget(
    file_bytes: usize,
    node_count: usize,
    total_name_bytes: usize,
    total_path_bytes: usize,
) -> Result<(), ManagedCacheCodecError> {
    let fixed_node_bytes = std::mem::size_of::<ManagedCacheNodeRecord>()
        .checked_add(std::mem::size_of::<Option<TreeNode>>())
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<crate::tree::NodeId>()))
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<u32>()))
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<usize>()))
        .and_then(|bytes| bytes.checked_add(DECODE_AUXILIARY_BYTES_PER_NODE))
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    let resident_bytes = node_count
        .checked_mul(fixed_node_bytes)
        .and_then(|bytes| bytes.checked_add(file_bytes))
        .and_then(|bytes| bytes.checked_add(total_name_bytes))
        .and_then(|bytes| bytes.checked_add(total_path_bytes))
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    if resident_bytes > MAX_MANAGED_CACHE_RESIDENT_BYTES {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }
    Ok(())
}

fn kind_to_wire(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::Directory => 0,
        NodeKind::File => 1,
        NodeKind::Symlink => 2,
        NodeKind::Other => 3,
        NodeKind::Error => 4,
    }
}

fn kind_from_wire(value: u8) -> Result<NodeKind, ManagedCacheCodecError> {
    match value {
        0 => Ok(NodeKind::Directory),
        1 => Ok(NodeKind::File),
        2 => Ok(NodeKind::Symlink),
        3 => Ok(NodeKind::Other),
        4 => Ok(NodeKind::Error),
        _ => Err(error(ManagedCacheCodecErrorKind::InvalidTree)),
    }
}

fn validate_metadata(
    metadata: &CacheMetadata,
    canonical_root: &Path,
    config: &CachedScanConfig,
) -> Result<(), ManagedCacheCodecError> {
    if !canonical_root_is_valid(&metadata.root_path)
        || metadata.version != CACHE_VERSION
        || metadata.root_path != canonical_root
        || metadata.config != *config
        || metadata.node_count == 0
        || metadata.node_count > MAX_MANAGED_CACHE_NODES
        || metadata
            .config
            .max_depth
            .is_some_and(|depth| depth > usize::from(MAX_MANAGED_CACHE_TREE_DEPTH))
        || metadata.scan_time.duration_since(UNIX_EPOCH).is_err()
        || metadata.root_mtime.duration_since(UNIX_EPOCH).is_err()
    {
        return Err(error(ManagedCacheCodecErrorKind::InvalidMetadata));
    }
    Ok(())
}

fn validate_metadata_tree_binding(
    metadata: &CacheMetadata,
    tree: &DiskTree,
) -> Result<(), ManagedCacheCodecError> {
    if metadata.node_count != tree.len()
        || metadata.total_size != tree.total_size()
        || metadata.root_path != tree.root_path()
    {
        return Err(error(ManagedCacheCodecErrorKind::InvalidMetadata));
    }
    Ok(())
}

fn validate_tree(tree: &DiskTree, canonical_root: &Path) -> Result<(), ManagedCacheCodecError> {
    tree.validate_managed_cache_tree(
        canonical_root,
        MAX_MANAGED_CACHE_NODES,
        MAX_MANAGED_CACHE_TREE_DEPTH,
        MAX_MANAGED_CACHE_COMPONENT_BYTES,
        MAX_MANAGED_CACHE_ROOT_BYTES,
    )
    .map_err(map_tree_error)
}

fn map_tree_error(kind: ManagedCacheTreeValidationError) -> ManagedCacheCodecError {
    match kind {
        ManagedCacheTreeValidationError::LimitExceeded => {
            error(ManagedCacheCodecErrorKind::LimitExceeded)
        }
        ManagedCacheTreeValidationError::AllocationFailed => {
            error(ManagedCacheCodecErrorKind::AllocationFailed)
        }
        ManagedCacheTreeValidationError::InvalidGraph
        | ManagedCacheTreeValidationError::InvalidName
        | ManagedCacheTreeValidationError::InvalidAggregate => {
            error(ManagedCacheCodecErrorKind::InvalidTree)
        }
    }
}

fn validate_canonical_root(path: &Path) -> Result<(), ManagedCacheCodecError> {
    if !canonical_root_is_valid(path) {
        return Err(error(ManagedCacheCodecErrorKind::BindingMismatch));
    }
    Ok(())
}

fn canonical_root_is_valid(path: &Path) -> bool {
    path.is_absolute()
        && native_path_bytes(path) != 0
        && native_path_bytes(path) <= MAX_MANAGED_CACHE_ROOT_BYTES
        && !path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
}

fn update_path_digest(digest: &mut Sha256, path: &Path) -> Result<(), ManagedCacheCodecError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        let bytes = path.as_os_str().as_bytes();
        digest.update([1]);
        digest.update(
            u64::try_from(bytes.len())
                .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?
                .to_le_bytes(),
        );
        digest.update(bytes);
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        let unit_count = path.as_os_str().encode_wide().count();
        digest.update([2]);
        digest.update(
            u64::try_from(unit_count)
                .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?
                .to_le_bytes(),
        );
        for unit in path.as_os_str().encode_wide() {
            digest.update(unit.to_le_bytes());
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let text = path
            .to_str()
            .ok_or_else(|| error(ManagedCacheCodecErrorKind::BindingMismatch))?;
        digest.update([3]);
        digest.update(
            u64::try_from(text.len())
                .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?
                .to_le_bytes(),
        );
        digest.update(text.as_bytes());
    }
    Ok(())
}

fn write_native_path(writer: &mut Writer, path: &Path) -> Result<(), ManagedCacheCodecError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        let bytes = path.as_os_str().as_bytes();
        writer.write_u8(1)?;
        writer.write_u32(
            u32::try_from(bytes.len())
                .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?,
        )?;
        writer.write_bytes(bytes)?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        let bytes = path
            .as_os_str()
            .encode_wide()
            .count()
            .checked_mul(2)
            .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
        writer.write_u8(2)?;
        writer.write_u32(
            u32::try_from(bytes).map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?,
        )?;
        for unit in path.as_os_str().encode_wide() {
            writer.write_u16(unit)?;
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let text = path
            .to_str()
            .ok_or_else(|| error(ManagedCacheCodecErrorKind::BindingMismatch))?;
        writer.write_u8(3)?;
        writer.write_u32(
            u32::try_from(text.len())
                .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?,
        )?;
        writer.write_bytes(text.as_bytes())?;
    }
    Ok(())
}

fn read_native_path(reader: &mut Reader<'_>) -> Result<PathBuf, ManagedCacheCodecError> {
    let encoding = reader.read_u8(ManagedCacheCodecErrorKind::InvalidMetadata)?;
    let byte_length =
        usize::try_from(reader.read_u32(ManagedCacheCodecErrorKind::InvalidMetadata)?)
            .map_err(|_| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
    if byte_length == 0 || byte_length > MAX_MANAGED_CACHE_ROOT_BYTES {
        return Err(error(ManagedCacheCodecErrorKind::LimitExceeded));
    }
    let bytes = reader.take(byte_length, ManagedCacheCodecErrorKind::InvalidMetadata)?;
    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;

        if encoding != 1 {
            return Err(error(ManagedCacheCodecErrorKind::InvalidMetadata));
        }
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(byte_length)
            .map_err(|_| error(ManagedCacheCodecErrorKind::AllocationFailed))?;
        owned.extend_from_slice(bytes);
        Ok(PathBuf::from(OsString::from_vec(owned)))
    }
    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt as _;

        if encoding != 2 || byte_length % 2 != 0 {
            return Err(error(ManagedCacheCodecErrorKind::InvalidMetadata));
        }
        let mut units = Vec::new();
        units
            .try_reserve_exact(byte_length / 2)
            .map_err(|_| error(ManagedCacheCodecErrorKind::AllocationFailed))?;
        for pair in bytes.chunks_exact(2) {
            units.push(u16::from_le_bytes([pair[0], pair[1]]));
        }
        Ok(PathBuf::from(OsString::from_wide(&units)))
    }
    #[cfg(not(any(unix, windows)))]
    {
        if encoding != 3 {
            return Err(error(ManagedCacheCodecErrorKind::InvalidMetadata));
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|_| error(ManagedCacheCodecErrorKind::InvalidMetadata))?;
        Ok(PathBuf::from(text))
    }
}

fn try_clone_native_path(path: &Path) -> Result<PathBuf, ManagedCacheCodecError> {
    let mut value = std::ffi::OsString::new();
    value
        .try_reserve_exact(path.as_os_str().len())
        .map_err(|_| error(ManagedCacheCodecErrorKind::AllocationFailed))?;
    value.push(path.as_os_str());
    Ok(PathBuf::from(value))
}

#[cfg(unix)]
fn native_path_bytes(path: &Path) -> usize {
    use std::os::unix::ffi::OsStrExt as _;

    path.as_os_str().as_bytes().len()
}

#[cfg(windows)]
fn native_path_bytes(path: &Path) -> usize {
    use std::os::windows::ffi::OsStrExt as _;

    path.as_os_str().encode_wide().count().saturating_mul(2)
}

#[cfg(not(any(unix, windows)))]
fn native_path_bytes(path: &Path) -> usize {
    path.as_os_str().to_string_lossy().len()
}

#[cfg(unix)]
const fn native_separator_bytes() -> usize {
    1
}

#[cfg(windows)]
const fn native_separator_bytes() -> usize {
    2
}

#[cfg(not(any(unix, windows)))]
const fn native_separator_bytes() -> usize {
    1
}

fn write_timestamp(
    writer: &mut Writer,
    value: SystemTime,
    invalid_kind: ManagedCacheCodecErrorKind,
) -> Result<(), ManagedCacheCodecError> {
    let duration = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| error(invalid_kind))?;
    writer.write_u64(duration.as_secs())?;
    writer.write_u32(duration.subsec_nanos())
}

fn read_timestamp(
    reader: &mut Reader<'_>,
    invalid_kind: ManagedCacheCodecErrorKind,
) -> Result<SystemTime, ManagedCacheCodecError> {
    let seconds = reader.read_u64(invalid_kind)?;
    let nanoseconds = reader.read_u32(invalid_kind)?;
    system_time_from_parts(seconds, nanoseconds, invalid_kind)
}

fn system_time_from_parts(
    seconds: u64,
    nanoseconds: u32,
    invalid_kind: ManagedCacheCodecErrorKind,
) -> Result<SystemTime, ManagedCacheCodecError> {
    if nanoseconds >= 1_000_000_000 {
        return Err(error(invalid_kind));
    }
    UNIX_EPOCH
        .checked_add(Duration::new(seconds, nanoseconds))
        .ok_or_else(|| error(invalid_kind))
}

fn read_header_u32(bytes: &[u8], offset: usize) -> Result<u32, ManagedCacheCodecError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::InvalidHeader))?;
    Ok(u32::from_le_bytes(value.try_into().map_err(|_| {
        error(ManagedCacheCodecErrorKind::InvalidHeader)
    })?))
}

fn read_header_u64(bytes: &[u8], offset: usize) -> Result<u64, ManagedCacheCodecError> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| error(ManagedCacheCodecErrorKind::InvalidHeader))?;
    Ok(u64::from_le_bytes(value.try_into().map_err(|_| {
        error(ManagedCacheCodecErrorKind::InvalidHeader)
    })?))
}

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn with_capacity(capacity: usize) -> Result<Self, ManagedCacheCodecError> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| error(ManagedCacheCodecErrorKind::AllocationFailed))?;
        Ok(Self { bytes })
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    fn into_vec(self) -> Vec<u8> {
        self.bytes
    }

    fn write_zeroes(&mut self, length: usize) -> Result<(), ManagedCacheCodecError> {
        self.reserve(length)?;
        self.bytes.resize(self.bytes.len() + length, 0);
        Ok(())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), ManagedCacheCodecError> {
        self.reserve(bytes.len())?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn write_u8(&mut self, value: u8) -> Result<(), ManagedCacheCodecError> {
        self.write_bytes(&[value])
    }

    fn write_u16(&mut self, value: u16) -> Result<(), ManagedCacheCodecError> {
        self.write_bytes(&value.to_le_bytes())
    }

    fn write_u32(&mut self, value: u32) -> Result<(), ManagedCacheCodecError> {
        self.write_bytes(&value.to_le_bytes())
    }

    fn write_u64(&mut self, value: u64) -> Result<(), ManagedCacheCodecError> {
        self.write_bytes(&value.to_le_bytes())
    }

    fn reserve(&mut self, additional: usize) -> Result<(), ManagedCacheCodecError> {
        self.bytes
            .try_reserve(additional)
            .map_err(|_| error(ManagedCacheCodecErrorKind::AllocationFailed))
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn take(
        &mut self,
        length: usize,
        invalid_kind: ManagedCacheCodecErrorKind,
    ) -> Result<&'a [u8], ManagedCacheCodecError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| error(ManagedCacheCodecErrorKind::LimitExceeded))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| error(invalid_kind))?;
        self.offset = end;
        Ok(value)
    }

    fn read_u8(
        &mut self,
        invalid_kind: ManagedCacheCodecErrorKind,
    ) -> Result<u8, ManagedCacheCodecError> {
        Ok(self.take(1, invalid_kind)?[0])
    }

    fn read_u16(
        &mut self,
        invalid_kind: ManagedCacheCodecErrorKind,
    ) -> Result<u16, ManagedCacheCodecError> {
        let value = self.take(2, invalid_kind)?;
        Ok(u16::from_le_bytes([value[0], value[1]]))
    }

    fn read_u32(
        &mut self,
        invalid_kind: ManagedCacheCodecErrorKind,
    ) -> Result<u32, ManagedCacheCodecError> {
        let value = self.take(4, invalid_kind)?;
        Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
    }

    fn read_u64(
        &mut self,
        invalid_kind: ManagedCacheCodecErrorKind,
    ) -> Result<u64, ManagedCacheCodecError> {
        let value = self.take(8, invalid_kind)?;
        Ok(u64::from_le_bytes([
            value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::NodeId;

    fn fixture() -> (PathBuf, CachedScanConfig, CacheMetadata, DiskTree) {
        let root = PathBuf::from("/tmp/dux-managed-cache-fixture");
        let config = CachedScanConfig {
            follow_symlinks: false,
            same_filesystem: true,
            max_depth: Some(12),
        };
        let mut tree = DiskTree::new(root.clone());
        let directory = tree.add_node(
            "Library".to_owned(),
            NodeKind::Directory,
            root.join("Library"),
            NodeId::ROOT,
        );
        let file = tree.add_node(
            "Cache.db".to_owned(),
            NodeKind::File,
            root.join("Library/Cache.db"),
            directory,
        );
        tree.set_size(file, 42);
        tree.get_mut(file).expect("file").mtime = Some(UNIX_EPOCH + Duration::from_secs(4));
        tree.aggregate_sizes();
        let metadata = CacheMetadata {
            version: CACHE_VERSION,
            root_path: root.clone(),
            scan_time: UNIX_EPOCH + Duration::new(10, 20),
            root_mtime: UNIX_EPOCH + Duration::new(30, 40),
            total_size: tree.total_size(),
            node_count: tree.len(),
            config: config.clone(),
        };
        (root, config, metadata, tree)
    }

    fn assert_kind<T: std::fmt::Debug>(
        result: Result<T, ManagedCacheCodecError>,
        expected: ManagedCacheCodecErrorKind,
    ) {
        assert_eq!(result.expect_err("expected codec error").kind, expected);
    }

    fn refresh_digest(bytes: &mut [u8]) {
        let digest: [u8; 32] = Sha256::digest(&bytes[MANAGED_CACHE_HEADER_BYTES..]).into();
        bytes[BODY_DIGEST_OFFSET..MANAGED_CACHE_HEADER_BYTES].copy_from_slice(&digest);
    }

    #[test]
    fn production_caps_are_small_and_resident_budget_is_conjunctive() {
        assert_eq!(MAX_MANAGED_CACHE_FILE_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_MANAGED_CACHE_NODES, 200_000);
        assert_eq!(MAX_MANAGED_CACHE_TOTAL_NAME_BYTES, 24 * 1024 * 1024);
        assert_eq!(MAX_MANAGED_CACHE_TOTAL_PATH_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_MANAGED_CACHE_RESIDENT_BYTES, 192 * 1024 * 1024);
        assert_eq!(MAX_MANAGED_CACHE_TREE_DEPTH, 512);

        assert_kind(
            validate_resident_budget(
                MAX_MANAGED_CACHE_FILE_BYTES as usize,
                MAX_MANAGED_CACHE_NODES,
                MAX_MANAGED_CACHE_TOTAL_NAME_BYTES,
                MAX_MANAGED_CACHE_TOTAL_PATH_BYTES,
            ),
            ManagedCacheCodecErrorKind::LimitExceeded,
        );
        validate_resident_budget(
            32 * 1024 * 1024,
            MAX_MANAGED_CACHE_NODES,
            12 * 1024 * 1024,
            32 * 1024 * 1024,
        )
        .expect("a realistic maximum-node cache remains admitted");
    }

    #[test]
    fn custom_wire_round_trips_all_bound_facts() {
        let (root, config, metadata, tree) = fixture();
        let encoded = encode_managed_cache(&root, &config, &metadata, &tree).expect("encode");
        let decoded = decode_managed_cache(&encoded, &root, &config).expect("decode");
        let (decoded_metadata, decoded_tree) = decoded.into_parts();

        assert_eq!(decoded_metadata.version, metadata.version);
        assert_eq!(decoded_metadata.root_path, metadata.root_path);
        assert_eq!(decoded_metadata.scan_time, metadata.scan_time);
        assert_eq!(decoded_metadata.root_mtime, metadata.root_mtime);
        assert_eq!(decoded_metadata.total_size, metadata.total_size);
        assert_eq!(decoded_metadata.node_count, metadata.node_count);
        assert_eq!(decoded_metadata.config, metadata.config);
        assert_eq!(decoded_tree.len(), tree.len());
        assert_eq!(decoded_tree.total_size(), tree.total_size());
        assert_eq!(
            decoded_tree.get(NodeId(2)).expect("file").mtime,
            tree.get(NodeId(2)).expect("file").mtime
        );
    }

    #[test]
    fn entry_key_binds_root_and_every_scan_option() {
        let (root, config, _, _) = fixture();
        let base = managed_cache_entry_key(&root, &config).expect("key");
        assert_eq!(base.to_lower_hex().len(), 64);
        assert_eq!(
            base,
            managed_cache_entry_key(&root, &config).expect("stable key")
        );

        let mut changed = config.clone();
        changed.follow_symlinks = true;
        assert_ne!(base, managed_cache_entry_key(&root, &changed).expect("key"));
        changed = config.clone();
        changed.same_filesystem = false;
        assert_ne!(base, managed_cache_entry_key(&root, &changed).expect("key"));
        changed = config.clone();
        changed.max_depth = None;
        assert_ne!(base, managed_cache_entry_key(&root, &changed).expect("key"));
        assert_ne!(
            base,
            managed_cache_entry_key(Path::new("/tmp/other"), &config).expect("key")
        );
    }

    #[test]
    fn legacy_v7_prefix_is_never_a_managed_cache_fallback() {
        let (root, config, _, _) = fixture();
        let mut legacy = vec![0_u8; MANAGED_CACHE_HEADER_BYTES];
        legacy[..4].copy_from_slice(b"DUXC");
        legacy[4..8].copy_from_slice(&CACHE_VERSION.to_le_bytes());
        assert_kind(
            decode_managed_cache(&legacy, &root, &config),
            ManagedCacheCodecErrorKind::InvalidHeader,
        );
    }

    #[test]
    fn header_preflight_rejects_version_lengths_and_node_limits() {
        let (root, config, metadata, tree) = fixture();
        let encoded = encode_managed_cache(&root, &config, &metadata, &tree).expect("encode");
        let mut header = encoded[..MANAGED_CACHE_HEADER_BYTES].to_vec();

        header[VERSION_OFFSET..HEADER_LENGTH_OFFSET].copy_from_slice(&99_u32.to_le_bytes());
        assert_kind(
            preflight_managed_cache_header(&header, encoded.len() as u64),
            ManagedCacheCodecErrorKind::UnsupportedVersion,
        );
        header[VERSION_OFFSET..HEADER_LENGTH_OFFSET]
            .copy_from_slice(&MANAGED_CACHE_WIRE_VERSION.to_le_bytes());
        header[HEADER_LENGTH_OFFSET..BODY_LENGTH_OFFSET].copy_from_slice(&63_u32.to_le_bytes());
        assert_kind(
            preflight_managed_cache_header(&header, encoded.len() as u64),
            ManagedCacheCodecErrorKind::InvalidHeader,
        );
        header[HEADER_LENGTH_OFFSET..BODY_LENGTH_OFFSET]
            .copy_from_slice(&(MANAGED_CACHE_HEADER_BYTES as u32).to_le_bytes());
        header[NODE_COUNT_OFFSET..BODY_DIGEST_OFFSET].copy_from_slice(&0_u64.to_le_bytes());
        assert_kind(
            preflight_managed_cache_header(&header, encoded.len() as u64),
            ManagedCacheCodecErrorKind::LimitExceeded,
        );
        header[NODE_COUNT_OFFSET..BODY_DIGEST_OFFSET]
            .copy_from_slice(&((MAX_MANAGED_CACHE_NODES as u64) + 1).to_le_bytes());
        assert_kind(
            preflight_managed_cache_header(&header, encoded.len() as u64),
            ManagedCacheCodecErrorKind::LimitExceeded,
        );
        assert_kind(
            preflight_managed_cache_header(
                &encoded[..MANAGED_CACHE_HEADER_BYTES],
                encoded.len() as u64 + 1,
            ),
            ManagedCacheCodecErrorKind::InvalidHeader,
        );
        assert_kind(
            preflight_managed_cache_header(
                &encoded[..MANAGED_CACHE_HEADER_BYTES],
                MAX_MANAGED_CACHE_FILE_BYTES + 1,
            ),
            ManagedCacheCodecErrorKind::LimitExceeded,
        );
    }

    #[test]
    fn body_checksum_and_binding_are_mandatory() {
        let (root, config, metadata, tree) = fixture();
        let mut encoded = encode_managed_cache(&root, &config, &metadata, &tree).expect("encode");
        let last = encoded.len() - 1;
        encoded[last] ^= 1;
        assert_kind(
            decode_managed_cache(&encoded, &root, &config),
            ManagedCacheCodecErrorKind::ChecksumMismatch,
        );

        let encoded = encode_managed_cache(&root, &config, &metadata, &tree).expect("encode");
        let mut changed = config;
        changed.follow_symlinks = true;
        assert_kind(
            decode_managed_cache(&encoded, &root, &changed),
            ManagedCacheCodecErrorKind::BindingMismatch,
        );
    }

    #[test]
    fn root_length_is_rejected_before_path_allocation() {
        let (root, config, metadata, tree) = fixture();
        let mut encoded = encode_managed_cache(&root, &config, &metadata, &tree).expect("encode");
        let root_length_offset =
            MANAGED_CACHE_HEADER_BYTES + BINDING_BYTES + METADATA_LENGTH_BYTES + 4 + 1;
        encoded[root_length_offset..root_length_offset + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        refresh_digest(&mut encoded);
        assert_kind(
            decode_managed_cache(&encoded, &root, &config),
            ManagedCacheCodecErrorKind::LimitExceeded,
        );
    }

    #[test]
    fn component_length_is_rejected_before_name_allocation() {
        let (root, config, metadata, tree) = fixture();
        let mut encoded = encode_managed_cache(&root, &config, &metadata, &tree).expect("encode");
        let metadata_length = usize::try_from(
            read_header_u32(&encoded[MANAGED_CACHE_HEADER_BYTES..], BINDING_BYTES)
                .expect("metadata length"),
        )
        .expect("usize");
        let first_record =
            MANAGED_CACHE_HEADER_BYTES + BINDING_BYTES + METADATA_LENGTH_BYTES + metadata_length;
        let name_length_offset = first_record + FIXED_NODE_RECORD_BYTES - 4;
        encoded[name_length_offset..name_length_offset + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        refresh_digest(&mut encoded);
        assert_kind(
            decode_managed_cache(&encoded, &root, &config),
            ManagedCacheCodecErrorKind::LimitExceeded,
        );
    }

    #[test]
    fn metadata_node_count_must_match_header() {
        let (root, config, metadata, tree) = fixture();
        let mut encoded = encode_managed_cache(&root, &config, &metadata, &tree).expect("encode");
        encoded[NODE_COUNT_OFFSET..BODY_DIGEST_OFFSET].copy_from_slice(&2_u64.to_le_bytes());
        assert_kind(
            decode_managed_cache(&encoded, &root, &config),
            ManagedCacheCodecErrorKind::InvalidMetadata,
        );
    }

    #[test]
    fn encode_rejects_metadata_and_tree_semantic_drift() {
        let (root, config, mut metadata, mut tree) = fixture();
        metadata.total_size += 1;
        assert_kind(
            encode_managed_cache(&root, &config, &metadata, &tree),
            ManagedCacheCodecErrorKind::InvalidMetadata,
        );

        metadata.total_size = tree.total_size();
        tree.get_mut(NodeId(1)).expect("directory").size += 1;
        assert_kind(
            encode_managed_cache(&root, &config, &metadata, &tree),
            ManagedCacheCodecErrorKind::InvalidTree,
        );
    }

    #[test]
    fn encode_rejects_duplicate_sibling_names_and_invalid_components() {
        let (root, config, mut metadata, mut tree) = fixture();
        let duplicate = tree.add_node(
            "Library".to_owned(),
            NodeKind::Directory,
            root.join("Library"),
            NodeId::ROOT,
        );
        tree.get_mut(duplicate).expect("duplicate").file_count = 0;
        tree.aggregate_sizes();
        metadata.node_count = tree.len();
        metadata.total_size = tree.total_size();
        assert_kind(
            encode_managed_cache(&root, &config, &metadata, &tree),
            ManagedCacheCodecErrorKind::InvalidTree,
        );

        let (root, config, mut metadata, mut tree) = fixture();
        tree.get_mut(NodeId(2)).expect("file").name = "../escape".to_owned();
        metadata.total_size = tree.total_size();
        assert_kind(
            encode_managed_cache(&root, &config, &metadata, &tree),
            ManagedCacheCodecErrorKind::InvalidTree,
        );
    }
}
