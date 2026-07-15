use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io;
use std::mem::{MaybeUninit, size_of};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf, Prefix};

use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, FileBasicInfo, FileIdInfo,
    FileStandardInfo, GetFileInformationByHandleEx,
};

use super::filesystem::{
    AncestorIdentity, CanonicalPathError, FilesystemEntryKind, FilesystemIdentity,
    PlatformEntrySnapshot, PlatformPathSnapshot, PlatformRootSnapshot, map_io_error,
};

// This backend reopens cumulative full paths. OPEN_REPARSE_POINT prevents the
// final component of each individual open from being traversed, but retained-
// handle relative traversal is still required before Windows evidence may
// authorize cleanup. Repeated snapshots are discovery evidence only.

pub(super) fn capture_root(path: &Path) -> Result<PlatformRootSnapshot, CanonicalPathError> {
    let mut current = PathBuf::new();
    let mut final_snapshot = None;

    for (component_index, component) in path.components().enumerate() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let snapshot = capture_entry(&current, component_index, false)?;
        if snapshot.kind != FilesystemEntryKind::Directory {
            return Err(CanonicalPathError::ScanRootNotDirectory);
        }
        final_snapshot = Some(snapshot);
    }

    let snapshot = final_snapshot.ok_or(CanonicalPathError::ScanRootNotDirectory)?;
    Ok(PlatformRootSnapshot {
        identity: snapshot.identity,
    })
}

pub(super) fn capture_descendant(
    root: &Path,
    relative_path: &Path,
) -> Result<PlatformPathSnapshot, CanonicalPathError> {
    let root_snapshot = capture_entry(root, 0, false)?;
    if root_snapshot.kind != FilesystemEntryKind::Directory {
        return Err(CanonicalPathError::ScanRootNotDirectory);
    }
    let mut ancestors = vec![AncestorIdentity::new(
        PathBuf::new(),
        root_snapshot.identity,
    )];
    let components: Vec<_> = relative_path.components().collect();
    let mut current = root.to_path_buf();
    let mut relative = PathBuf::new();

    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            return Err(CanonicalPathError::CanonicalEscapesScanRoot);
        };
        let component_index = index + 1;
        let target = index + 1 == components.len();
        current.push(component);
        relative.push(component);
        let snapshot = capture_entry(&current, component_index, target)?;
        if snapshot.identity.volume() != root_snapshot.identity.volume() {
            return Err(CanonicalPathError::CrossVolume {
                component_index,
                expected: root_snapshot.identity.volume(),
                observed: snapshot.identity.volume(),
            });
        }
        if target {
            return Ok(PlatformPathSnapshot {
                target: snapshot,
                ancestors,
            });
        }
        if snapshot.kind != FilesystemEntryKind::Directory {
            return Err(CanonicalPathError::NonDirectoryAncestor { component_index });
        }
        ancestors.push(AncestorIdentity::new(relative.clone(), snapshot.identity));
    }

    Err(CanonicalPathError::CanonicalEscapesScanRoot)
}

/// Compares the ordinary drive path accepted by the lexical validator with
/// the equivalent verbatim drive path commonly returned by `canonicalize`.
/// Component comparison is deliberately fail-closed for non-ASCII case-only
/// aliases; rejecting a valid spelling is safer than accepting a different one.
pub(super) fn paths_equivalent(requested: &Path, canonical: &Path) -> bool {
    let Some((requested_drive, requested_components)) = comparable_components(requested) else {
        return false;
    };
    let Some((canonical_drive, canonical_components)) = comparable_components(canonical) else {
        return false;
    };

    requested_drive.eq_ignore_ascii_case(&canonical_drive)
        && requested_components.len() == canonical_components.len()
        && requested_components
            .iter()
            .zip(canonical_components)
            .all(|(requested, canonical)| equivalent_component(requested, canonical))
}

fn comparable_components(path: &Path) -> Option<(char, Vec<&OsStr>)> {
    let mut components = path.components();
    let Component::Prefix(prefix) = components.next()? else {
        return None;
    };
    let drive = match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => char::from(drive),
        _ => return None,
    };
    if !matches!(components.next(), Some(Component::RootDir)) {
        return None;
    }
    let components = components
        .map(|component| match component {
            Component::Normal(component) => Some(component),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some((drive, components))
}

fn equivalent_component(left: &OsStr, right: &OsStr) -> bool {
    match (left.to_str(), right.to_str()) {
        (Some(left), Some(right)) if left.is_ascii() && right.is_ascii() => {
            left.eq_ignore_ascii_case(right)
        }
        _ => left == right,
    }
}

fn capture_entry(
    path: &Path,
    component_index: usize,
    target: bool,
) -> Result<PlatformEntrySnapshot, CanonicalPathError> {
    let file = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|source| map_io_error(component_index, source))?;
    let handle = file.as_raw_handle();

    let mut id_info = MaybeUninit::<FILE_ID_INFO>::zeroed();
    // SAFETY: `file` owns a valid handle for the call and the output buffer has
    // the exact size required for `FILE_ID_INFO`.
    let id_result = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            id_info.as_mut_ptr().cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if id_result == 0 {
        return Err(map_io_error(component_index, io::Error::last_os_error()));
    }
    // SAFETY: the successful call initialized the complete output structure.
    let id_info = unsafe { id_info.assume_init() };

    let mut basic_info = MaybeUninit::<FILE_BASIC_INFO>::zeroed();
    // SAFETY: as above, using the matching output type and exact buffer size.
    let basic_result = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            basic_info.as_mut_ptr().cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    if basic_result == 0 {
        return Err(map_io_error(component_index, io::Error::last_os_error()));
    }
    // SAFETY: the successful call initialized the complete output structure.
    let basic_info = unsafe { basic_info.assume_init() };

    let mut standard_info = MaybeUninit::<FILE_STANDARD_INFO>::zeroed();
    // SAFETY: as above, using the matching output type and exact buffer size.
    let standard_result = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileStandardInfo,
            standard_info.as_mut_ptr().cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    if standard_result == 0 {
        return Err(map_io_error(component_index, io::Error::last_os_error()));
    }
    // SAFETY: the successful call initialized the complete output structure.
    let standard_info = unsafe { standard_info.assume_init() };
    let attributes = basic_info.FileAttributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(CanonicalPathError::SymlinkOrReparsePoint {
            component_index,
            target,
        });
    }
    let kind = if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        FilesystemEntryKind::Directory
    } else if attributes & FILE_ATTRIBUTE_DEVICE == 0 {
        FilesystemEntryKind::RegularFile
    } else {
        return Err(CanonicalPathError::UnsupportedTargetKind);
    };

    Ok(PlatformEntrySnapshot {
        identity: FilesystemIdentity::new(
            id_info.VolumeSerialNumber,
            u128::from_le_bytes(id_info.FileId.Identifier),
        ),
        kind,
        hard_link_count: u64::from(standard_info.NumberOfLinks),
    })
}
