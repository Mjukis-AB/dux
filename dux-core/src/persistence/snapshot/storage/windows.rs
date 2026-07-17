//! Windows retained-handle implementation for private snapshot storage.
//!
//! Every child lookup is relative to an already-retained directory handle.
//! Paths are used only to acquire configured ancestors before any child name
//! becomes authoritative.

use std::ffi::{OsStr, c_void};
use std::fs::File;
use std::mem::{MaybeUninit, offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::Instant;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS,
    ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_FILES, ERROR_SUCCESS, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree, OBJ_CASE_INSENSITIVE, UNICODE_STRING,
};
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_REVISION, ACL_SIZE_INFORMATION, AclSizeInformation,
    AddAccessAllowedAceEx, CONTAINER_INHERIT_ACE, CopySid, DACL_SECURITY_INFORMATION, EqualSid,
    GetAce, GetAclInformation, GetLengthSid, GetSecurityDescriptorControl, GetTokenInformation,
    INHERITED_ACE, InitializeAcl, InitializeSecurityDescriptor, IsValidAcl, IsValidSid,
    OBJECT_INHERIT_ACE, OWNER_SECURITY_INFORMATION, PSID, SE_DACL_PRESENT, SE_DACL_PROTECTED,
    SECURITY_DESCRIPTOR, SECURITY_DESCRIPTOR_CONTROL, SetSecurityDescriptorControl,
    SetSecurityDescriptorDacl, SetSecurityDescriptorOwner, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_ADD_SUBDIRECTORY, FILE_ALL_ACCESS, FILE_ATTRIBUTE_DEVICE,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_ID_BOTH_DIR_INFO, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
    FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO,
    FileBasicInfo, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, FileIdInfo,
    FileRenameInfo, FileStandardInfo, GetFileInformationByHandleEx, OPEN_EXISTING, READ_CONTROL,
    WRITE_DAC,
};
use windows_sys::Win32::System::SystemServices::{
    ACCESS_ALLOWED_ACE_TYPE, SECURITY_DESCRIPTOR_REVISION,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{Result, SnapshotStorageError, SnapshotStorageErrorKind};

const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
const SHARE_WITHOUT_DELETE: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE;
const FILE_OPEN_DISPOSITION: u32 = 1;
const FILE_CREATE_DISPOSITION: u32 = 2;
const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
const FILE_OPEN_REPARSE_POINT_NATIVE: u32 = 0x0020_0000;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
const STATUS_OBJECT_NAME_NOT_FOUND: i32 = 0xc000_0034_u32 as i32;
const STATUS_OBJECT_NAME_COLLISION: i32 = 0xc000_0035_u32 as i32;
const STATUS_OBJECT_PATH_NOT_FOUND: i32 = 0xc000_003a_u32 as i32;
const DIRECTORY_BUFFER_BYTES: usize = 64 * 1024;
const DELETE_DISPOSITION_CLASS: i32 = 21;
const DELETE_DISPOSITION_FLAG: u32 = 1;
const POSIX_DISPOSITION_FLAG: u32 = 2;

#[repr(C)]
struct DeleteDisposition {
    flags: u32,
}

#[repr(C)]
union IoStatusValue {
    status: i32,
    pointer: *mut c_void,
}

#[repr(C)]
struct IoStatusBlock {
    value: IoStatusValue,
    information: usize,
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: HANDLE,
    object_name: *mut UNICODE_STRING,
    attributes: u32,
    security_descriptor: *mut c_void,
    security_quality_of_service: *mut c_void,
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtCreateFile(
        file_handle: *mut HANDLE,
        desired_access: u32,
        object_attributes: *const ObjectAttributes,
        io_status_block: *mut IoStatusBlock,
        allocation_size: *const i64,
        file_attributes: u32,
        share_access: u32,
        create_disposition: u32,
        create_options: u32,
        ea_buffer: *const c_void,
        ea_length: u32,
    ) -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    volume_serial: u64,
    file_id: [u8; 16],
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Directory,
    RegularFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Publication {
    Published,
    Collision,
}

pub(super) fn open_private_directory(path: &Path) -> Result<File> {
    let parent = path.parent().ok_or_else(unsafe_root)?;
    validate_ancestor_chain(parent)?;
    let file = open_path(
        path,
        Kind::Directory,
        private_directory_access(),
        SHARE_WITHOUT_DELETE,
    )?;
    validate_private(&file, Kind::Directory, None, false)?;
    Ok(file)
}

pub(super) fn open_publication_parent(path: &Path) -> Result<File> {
    let parent = path.parent().ok_or_else(unsafe_root)?;
    validate_ancestor_chain(parent)?;
    let file = open_path(
        path,
        Kind::Directory,
        FILE_READ_ATTRIBUTES | FILE_ADD_SUBDIRECTORY | FILE_GENERIC_WRITE,
        SHARE_ALL,
    )?;
    validate_structure(&file, Kind::Directory, None, false)?;
    Ok(file)
}

pub(super) fn open_existing_directory(
    parent: &File,
    _parent_path: &Path,
    name: &str,
) -> Result<Option<File>> {
    let Some(file) = open_relative(
        parent,
        name,
        Kind::Directory,
        private_directory_access(),
        SHARE_WITHOUT_DELETE,
    )?
    else {
        return Ok(None);
    };
    validate_private(&file, Kind::Directory, None, false)?;
    Ok(Some(file))
}

pub(super) fn create_private_directory_exclusive(
    parent: &File,
    _parent_path: &Path,
    name: &str,
) -> Result<Option<File>> {
    let Some(file) = create_relative_private(
        parent,
        name,
        Kind::Directory,
        private_directory_access() | DELETE,
        SHARE_WITHOUT_DELETE,
    )?
    else {
        return Ok(None);
    };
    validate_private(&file, Kind::Directory, None, false)?;
    Ok(Some(file))
}

pub(super) fn create_private_file_exclusive(
    directory: &File,
    _directory_path: &Path,
    name: &str,
) -> Result<Option<(File, Identity)>> {
    let Some(file) = create_relative_private(
        directory,
        name,
        Kind::RegularFile,
        private_file_access(true) | DELETE,
        SHARE_WITHOUT_DELETE,
    )?
    else {
        return Ok(None);
    };
    let identity = validate_private(&file, Kind::RegularFile, None, true)?;
    Ok(Some((file, identity)))
}

pub(super) fn open_named_regular(
    directory: &File,
    _directory_path: &Path,
    name: &str,
    writable: bool,
) -> Result<Option<(File, Identity)>> {
    let share = if writable {
        SHARE_WITHOUT_DELETE
    } else {
        SHARE_ALL
    };
    let Some(file) = open_relative(
        directory,
        name,
        Kind::RegularFile,
        private_file_access(writable),
        share,
    )?
    else {
        return Ok(None);
    };
    let identity = validate_private(&file, Kind::RegularFile, None, true)?;
    Ok(Some((file, identity)))
}

pub(super) fn open_named_temp_for_removal(
    directory: &File,
    _directory_path: &Path,
    name: &str,
) -> Result<Option<(File, Identity)>> {
    let Some(file) = open_relative(
        directory,
        name,
        Kind::RegularFile,
        private_file_access(true) | DELETE,
        SHARE_WITHOUT_DELETE,
    )?
    else {
        return Ok(None);
    };
    let identity = validate_private(&file, Kind::RegularFile, None, true)?;
    Ok(Some((file, identity)))
}

pub(super) fn open_named_final_for_removal(
    directory: &File,
    _directory_path: &Path,
    name: &str,
) -> Result<Option<(File, Identity)>> {
    let Some(file) = open_relative(
        directory,
        name,
        Kind::RegularFile,
        private_file_access(false) | DELETE,
        SHARE_ALL,
    )?
    else {
        return Ok(None);
    };
    let identity = validate_private(&file, Kind::RegularFile, None, true)?;
    Ok(Some((file, identity)))
}

pub(super) fn identity(file: &File, kind: Kind) -> Result<Identity> {
    validate_structure(file, kind, None, matches!(kind, Kind::RegularFile))
}

pub(super) fn file_usage(file: &File) -> Result<(u64, u64)> {
    let mut standard = MaybeUninit::<FILE_STANDARD_INFO>::zeroed();
    // SAFETY: the output buffer exactly matches FileStandardInfo and the
    // retained file handle remains live for the synchronous call.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileStandardInfo,
            standard.as_mut_ptr().cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(unavailable());
    }
    // SAFETY: the successful call initialized the complete output buffer.
    let standard = unsafe { standard.assume_init() };
    let logical_bytes =
        u64::try_from(standard.EndOfFile).map_err(|_| unsafe_for(Kind::RegularFile))?;
    let allocated_bytes =
        u64::try_from(standard.AllocationSize).map_err(|_| unsafe_for(Kind::RegularFile))?;
    Ok((logical_bytes, allocated_bytes))
}

pub(super) fn validate_retained(
    file: &File,
    expected: Identity,
    kind: Kind,
    require_one_link: bool,
) -> Result<()> {
    validate_private(file, kind, Some(expected), require_one_link).map(drop)
}

pub(super) fn validate_named(
    directory: &File,
    name: &str,
    retained: &File,
    expected: Identity,
    kind: Kind,
) -> Result<()> {
    validate_named_entry(directory, name, retained, expected, kind, true)
}

fn validate_named_entry(
    directory: &File,
    name: &str,
    retained: &File,
    expected: Identity,
    kind: Kind,
    directory_is_private: bool,
) -> Result<()> {
    if directory_is_private {
        validate_private(directory, Kind::Directory, None, false)?;
    } else {
        validate_structure(directory, Kind::Directory, None, false)?;
    }
    validate_private(
        retained,
        kind,
        Some(expected),
        matches!(kind, Kind::RegularFile),
    )?;
    let Some(probe) = open_relative(
        directory,
        name,
        kind,
        match kind {
            Kind::Directory => FILE_GENERIC_READ | READ_CONTROL,
            Kind::RegularFile => private_file_access(false),
        },
        SHARE_ALL,
    )?
    else {
        return Err(unsafe_for(kind));
    };
    validate_private(
        &probe,
        kind,
        Some(expected),
        matches!(kind, Kind::RegularFile),
    )
    .map(drop)
}

pub(super) fn inventory(
    directory: &File,
    maximum_entries: usize,
    maximum_name_bytes: usize,
    deadline: Instant,
) -> Result<Vec<String>> {
    validate_private(directory, Kind::Directory, None, false)?;
    let words = DIRECTORY_BUFFER_BYTES.div_ceil(size_of::<usize>());
    let mut buffer = vec![0_usize; words];
    let mut names = Vec::new();
    let mut total_name_bytes = 0_usize;
    let mut restart = true;
    loop {
        if Instant::now() > deadline {
            return Err(unsafe_root());
        }
        buffer.fill(0);
        let class = if restart {
            FileIdBothDirectoryRestartInfo
        } else {
            FileIdBothDirectoryInfo
        };
        restart = false;
        // SAFETY: the retained directory handle is live and the aligned buffer
        // is writable for exactly the advertised byte length.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                directory.as_raw_handle(),
                class,
                buffer.as_mut_ptr().cast(),
                DIRECTORY_BUFFER_BYTES as u32,
            )
        };
        if ok == 0 {
            // SAFETY: GetLastError immediately follows the failed Win32 call.
            if unsafe { GetLastError() } == ERROR_NO_MORE_FILES {
                break;
            }
            return Err(unavailable());
        }

        let bytes = buffer.as_ptr().cast::<u8>();
        let mut offset = 0_usize;
        loop {
            let fixed = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
            if offset
                .checked_add(size_of::<FILE_ID_BOTH_DIR_INFO>())
                .is_none_or(|end| end > DIRECTORY_BUFFER_BYTES)
            {
                return Err(unsafe_root());
            }
            // SAFETY: the checked fixed prefix lies inside the aligned byte
            // buffer; read_unaligned avoids relying on directory-record offsets.
            let record = unsafe {
                std::ptr::read_unaligned(bytes.add(offset).cast::<FILE_ID_BOTH_DIR_INFO>())
            };
            let name_bytes = usize::try_from(record.FileNameLength).map_err(|_| unsafe_root())?;
            if name_bytes == 0 || name_bytes % size_of::<u16>() != 0 {
                return Err(unsafe_root());
            }
            let name_start = offset.checked_add(fixed).ok_or_else(unsafe_root)?;
            let name_end = name_start.checked_add(name_bytes).ok_or_else(unsafe_root)?;
            if name_end > DIRECTORY_BUFFER_BYTES || name_start % size_of::<u16>() != 0 {
                return Err(unsafe_root());
            }
            // Windows directory information stores UTF-16 names at a naturally
            // aligned offset. The fixed layout assertion makes that explicit.
            const _: () = assert!(offset_of!(FILE_ID_BOTH_DIR_INFO, FileName) % 2 == 0);
            // SAFETY: bounds and u16 alignment are proven above.
            let units = unsafe {
                std::slice::from_raw_parts(bytes.add(name_start).cast::<u16>(), name_bytes / 2)
            };
            let name = String::from_utf16(units).map_err(|_| unsafe_root())?;
            if name != "." && name != ".." {
                validate_component(&name)?;
                if names.len() >= maximum_entries {
                    return Err(unsafe_root());
                }
                total_name_bytes = total_name_bytes
                    .checked_add(name.len())
                    .ok_or_else(unsafe_root)?;
                if total_name_bytes > maximum_name_bytes {
                    return Err(unsafe_root());
                }
                names.push(name);
            }
            if record.NextEntryOffset == 0 {
                break;
            }
            let next = usize::try_from(record.NextEntryOffset).map_err(|_| unsafe_root())?;
            if next < fixed
                || offset
                    .checked_add(next)
                    .is_none_or(|next| next >= DIRECTORY_BUFFER_BYTES)
            {
                return Err(unsafe_root());
            }
            offset += next;
        }
    }
    validate_private(directory, Kind::Directory, None, false)?;
    Ok(names)
}

pub(super) fn sync_directory(directory: &File) -> Result<()> {
    directory.sync_all().map_err(|_| unavailable())
}

pub(super) fn publish_no_replace(
    directory: &File,
    source: &str,
    source_file: &File,
    source_identity: Identity,
    destination: &str,
) -> Result<Publication> {
    publish_between(
        directory,
        source,
        source_file,
        source_identity,
        Kind::RegularFile,
        true,
        directory,
        destination,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn publish_directory_no_replace(
    source_parent: &File,
    source: &str,
    source_directory: &File,
    source_identity: Identity,
    destination_parent: &File,
    destination: &str,
) -> Result<Publication> {
    publish_between(
        source_parent,
        source,
        source_directory,
        source_identity,
        Kind::Directory,
        false,
        destination_parent,
        destination,
    )
}

pub(super) fn remove_retained_temp(
    directory: &File,
    name: &str,
    file: File,
    expected: Identity,
) -> Result<()> {
    validate_named(directory, name, &file, expected, Kind::RegularFile)?;
    let disposition = DeleteDisposition {
        flags: DELETE_DISPOSITION_FLAG | POSIX_DISPOSITION_FLAG,
    };
    // SAFETY: the retained exact-identity current-call, row-bound quiescent, or
    // complete-inventory-proven unleased quiescent temp handle has DELETE
    // access and the fixed disposition buffer is live for the synchronous
    // call.
    let removed = unsafe {
        // DUX-DESTRUCTIVE: allow=snapshot-windows-current-temp-delete -- unlink only a retained current-call, exact row-bound quiescent temp, or exact quiescent unleased temp selected from complete bounded lease and physical inventories after handle-relative identity revalidation
        windows_sys::Win32::Storage::FileSystem::SetFileInformationByHandle(
            file.as_raw_handle(),
            DELETE_DISPOSITION_CLASS,
            (&raw const disposition).cast(),
            size_of::<DeleteDisposition>() as u32,
        )
    };
    if removed == 0 {
        return Err(unavailable());
    }
    // POSIX disposition removes the link when this delete-capable handle
    // closes. Close it synchronously before the caller flushes the directory;
    // there is no fallible work after the successful disposition.
    drop(file);
    Ok(())
}

pub(super) fn remove_retained_final(
    directory: &File,
    name: &str,
    file: File,
    expected: Identity,
) -> Result<()> {
    validate_named(directory, name, &file, expected, Kind::RegularFile)?;
    let disposition = DeleteDisposition {
        flags: DELETE_DISPOSITION_FLAG | POSIX_DISPOSITION_FLAG,
    };
    // SAFETY: the retained exact-identity final handle has DELETE access and
    // the fixed disposition buffer is live for the synchronous call.
    let removed = unsafe {
        // DUX-DESTRUCTIVE: allow=snapshot-windows-observed-final-delete -- delete only an exact typed final observed under the retained inventory writer lease after higher-layer tombstone or orphan authority plus identity and usage revalidation
        windows_sys::Win32::Storage::FileSystem::SetFileInformationByHandle(
            file.as_raw_handle(),
            DELETE_DISPOSITION_CLASS,
            (&raw const disposition).cast(),
            size_of::<DeleteDisposition>() as u32,
        )
    };
    if removed == 0 {
        return Err(unavailable());
    }
    // POSIX disposition removes the link when this delete-capable handle
    // closes. Close it synchronously before returning so the caller's
    // directory flush necessarily follows the namespace mutation. A safe
    // owned `File` cannot become an invalid handle between the successful
    // disposition and this close, and there is no fallible work in between.
    drop(file);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn publish_between(
    source_parent: &File,
    source_name: &str,
    source: &File,
    source_identity: Identity,
    kind: Kind,
    source_parent_is_private: bool,
    destination_parent: &File,
    destination_name: &str,
) -> Result<Publication> {
    validate_component(destination_name)?;
    validate_named_entry(
        source_parent,
        source_name,
        source,
        source_identity,
        kind,
        source_parent_is_private,
    )?;
    validate_structure(destination_parent, Kind::Directory, None, false)?;
    match rename_by_handle_no_replace(source, destination_parent, destination_name) {
        Ok(()) => {
            validate_named(
                destination_parent,
                destination_name,
                source,
                source_identity,
                kind,
            )?;
            Ok(Publication::Published)
        }
        Err(ERROR_ALREADY_EXISTS | ERROR_FILE_EXISTS) => Ok(Publication::Collision),
        Err(ERROR_ACCESS_DENIED)
            if relative_entry_exists(destination_parent, destination_name)? =>
        {
            Ok(Publication::Collision)
        }
        Err(_) => Err(unavailable()),
    }
}

fn relative_entry_exists(parent: &File, name: &str) -> Result<bool> {
    for kind in [Kind::RegularFile, Kind::Directory] {
        match open_relative(
            parent,
            name,
            kind,
            FILE_READ_ATTRIBUTES | READ_CONTROL,
            SHARE_ALL,
        ) {
            Ok(Some(file)) => {
                validate_structure(&file, kind, None, matches!(kind, Kind::RegularFile))?;
                return Ok(true);
            }
            Ok(None) => {}
            Err(error) if error.kind() == SnapshotStorageErrorKind::UnsafeObject => {}
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

fn rename_by_handle_no_replace(
    source: &File,
    destination_parent: &File,
    destination_name: &str,
) -> std::result::Result<(), u32> {
    let name = component_units(destination_name)
        .map_err(|_| windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    let name_bytes = name
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|size| u32::try_from(size).ok())
        .ok_or(windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    let info_size = offset_of!(FILE_RENAME_INFO, FileName)
        .checked_add(name_bytes as usize)
        .ok_or(windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    let mut buffer = vec![0_usize; info_size.div_ceil(size_of::<usize>())];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: the aligned buffer covers the header and full flexible UTF-16 name.
    unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = destination_parent.as_raw_handle();
        (*info).FileNameLength = name_bytes;
        std::ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
    }
    let info_size =
        u32::try_from(info_size).map_err(|_| windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    // SAFETY: both retained handles and the initialized info buffer remain live.
    let renamed = unsafe {
        // DUX-DESTRUCTIVE: allow=snapshot-windows-handle-publish -- atomically no-replace publish only an exact-identity retained marker-complete stage directory or current create-new snapshot temp
        windows_sys::Win32::Storage::FileSystem::SetFileInformationByHandle(
            source.as_raw_handle(),
            FileRenameInfo,
            info.cast(),
            info_size,
        )
    };
    if renamed == 0 {
        // SAFETY: GetLastError immediately follows the failed Win32 call.
        return Err(unsafe { GetLastError() });
    }
    Ok(())
}

fn create_relative_private(
    parent: &File,
    name: &str,
    kind: Kind,
    desired_access: u32,
    share_access: u32,
) -> Result<Option<File>> {
    let mut security = PrivateSecurity::new(kind)?;
    match relative_file(
        parent,
        name,
        kind,
        desired_access,
        share_access,
        FILE_CREATE_DISPOSITION,
        (&raw mut security.descriptor).cast(),
    ) {
        Ok(file) => Ok(Some(file)),
        Err(STATUS_OBJECT_NAME_COLLISION) => Ok(None),
        Err(_) => Err(unavailable()),
    }
}

fn open_relative(
    parent: &File,
    name: &str,
    kind: Kind,
    desired_access: u32,
    share_access: u32,
) -> Result<Option<File>> {
    match relative_file(
        parent,
        name,
        kind,
        desired_access,
        share_access,
        FILE_OPEN_DISPOSITION,
        null_mut(),
    ) {
        Ok(file) => Ok(Some(file)),
        Err(STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND) => Ok(None),
        Err(_) => Err(unsafe_for(kind)),
    }
}

#[allow(clippy::too_many_arguments)]
fn relative_file(
    parent: &File,
    name: &str,
    kind: Kind,
    desired_access: u32,
    share_access: u32,
    disposition: u32,
    security_descriptor: *mut c_void,
) -> std::result::Result<File, i32> {
    let mut units = component_units(name).map_err(|_| STATUS_OBJECT_NAME_NOT_FOUND)?;
    let name_bytes = units
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or(STATUS_OBJECT_NAME_NOT_FOUND)?;
    let mut object_name = UNICODE_STRING {
        Length: name_bytes,
        MaximumLength: name_bytes,
        Buffer: units.as_mut_ptr(),
    };
    let attributes = ObjectAttributes {
        length: size_of::<ObjectAttributes>() as u32,
        root_directory: parent.as_raw_handle(),
        object_name: &mut object_name,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor,
        security_quality_of_service: null_mut(),
    };
    let mut io_status = IoStatusBlock {
        value: IoStatusValue { status: 0 },
        information: 0,
    };
    let mut handle = INVALID_HANDLE_VALUE;
    let options = match kind {
        Kind::Directory => FILE_DIRECTORY_FILE,
        Kind::RegularFile => FILE_NON_DIRECTORY_FILE,
    } | FILE_OPEN_REPARSE_POINT_NATIVE
        | FILE_SYNCHRONOUS_IO_NONALERT;
    // SAFETY: every pointer and the retained RootDirectory remain live for this
    // synchronous call; the validated leaf cannot escape the retained parent.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            desired_access,
            &attributes,
            &mut io_status,
            null(),
            FILE_ATTRIBUTE_NORMAL,
            share_access,
            disposition,
            options,
            null(),
            0,
        )
    };
    if status < 0 || handle == INVALID_HANDLE_VALUE {
        return Err(status);
    }
    // SAFETY: NtCreateFile returned one uniquely owned handle and File closes it.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn open_path(path: &Path, kind: Kind, desired_access: u32, share_access: u32) -> Result<File> {
    let wide = wide_path(path)?;
    let flags = FILE_FLAG_OPEN_REPARSE_POINT
        | if matches!(kind, Kind::Directory) {
            FILE_FLAG_BACKUP_SEMANTICS
        } else {
            0
        };
    // SAFETY: the NUL-terminated path remains live and a successful handle is owned.
    let handle = unsafe {
        windows_sys::Win32::Storage::FileSystem::CreateFileW(
            wide.as_ptr(),
            desired_access,
            share_access,
            null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(unsafe_for(kind));
    }
    // SAFETY: CreateFileW returned one uniquely owned handle and File closes it.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn open_ancestor(path: &Path) -> Result<File> {
    open_path(path, Kind::Directory, 0, SHARE_ALL)
}

fn validate_ancestor_chain(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        validate_structure(&open_ancestor(ancestor)?, Kind::Directory, None, false)?;
    }
    Ok(())
}

fn validate_private(
    file: &File,
    kind: Kind,
    expected: Option<Identity>,
    require_one_link: bool,
) -> Result<Identity> {
    let identity = validate_structure(file, kind, expected, require_one_link)?;
    let owner = OwnedSid::current().map_err(|_| unsafe_for(kind))?;
    if !inspect_private_security(file, kind, &owner)? {
        return Err(unsafe_for(kind));
    }
    Ok(identity)
}

fn validate_structure(
    file: &File,
    kind: Kind,
    expected: Option<Identity>,
    require_one_link: bool,
) -> Result<Identity> {
    let mut id = MaybeUninit::<FILE_ID_INFO>::zeroed();
    let mut basic = MaybeUninit::<FILE_BASIC_INFO>::zeroed();
    let mut standard = MaybeUninit::<FILE_STANDARD_INFO>::zeroed();
    let handle = file.as_raw_handle();
    // SAFETY: each buffer exactly matches its requested information class.
    let id_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            id.as_mut_ptr().cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    // SAFETY: as above for basic information.
    let basic_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            basic.as_mut_ptr().cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    // SAFETY: as above for standard information.
    let standard_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileStandardInfo,
            standard.as_mut_ptr().cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    if id_ok == 0 || basic_ok == 0 || standard_ok == 0 {
        return Err(unavailable());
    }
    // SAFETY: all successful calls initialized their complete output buffers.
    let (id, basic, standard) = unsafe {
        (
            id.assume_init(),
            basic.assume_init(),
            standard.assume_init(),
        )
    };
    if basic.FileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DEVICE) != 0 {
        return Err(unsafe_for(kind));
    }
    let is_directory = basic.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    if is_directory != matches!(kind, Kind::Directory)
        || (require_one_link && standard.NumberOfLinks != 1)
    {
        return Err(unsafe_for(kind));
    }
    let identity = Identity {
        volume_serial: id.VolumeSerialNumber,
        file_id: id.FileId.Identifier,
    };
    if expected.is_some_and(|expected| expected != identity) {
        return Err(unsafe_for(kind));
    }
    Ok(identity)
}

fn inspect_private_security(file: &File, kind: Kind, current_user: &OwnedSid) -> Result<bool> {
    let mut owner: PSID = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: all output pointers are writable; the returned allocation is guarded.
    let result = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    let descriptor_guard = (!descriptor.is_null()).then(|| LocalSecurityDescriptor(descriptor));
    if result != ERROR_SUCCESS || descriptor.is_null() || owner.is_null() {
        return Err(unsafe_for(kind));
    }
    let _descriptor = descriptor_guard.ok_or_else(|| unsafe_for(kind))?;
    // SAFETY: both SID allocations remain live for the comparison.
    if unsafe { IsValidSid(owner) } == 0 || unsafe { EqualSid(owner, current_user.as_ptr()) } == 0 {
        return Err(unsafe_for(kind));
    }
    let mut control: SECURITY_DESCRIPTOR_CONTROL = 0;
    let mut revision = 0_u32;
    // SAFETY: the descriptor is live and both outputs are writable.
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(unsafe_for(kind));
    }
    Ok((control & SE_DACL_PRESENT) != 0
        && (control & SE_DACL_PROTECTED) != 0
        && dacl_is_exact(dacl, kind, current_user))
}

fn dacl_is_exact(dacl: *const ACL, kind: Kind, current_user: &OwnedSid) -> bool {
    if dacl.is_null() || unsafe { IsValidAcl(dacl) } == 0 {
        return false;
    }
    let mut info = MaybeUninit::<ACL_SIZE_INFORMATION>::zeroed();
    // SAFETY: dacl is valid and the output matches AclSizeInformation.
    if unsafe {
        GetAclInformation(
            dacl,
            info.as_mut_ptr().cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return false;
    }
    // SAFETY: the successful call initialized the complete output.
    let info = unsafe { info.assume_init() };
    if info.AceCount != 1 {
        return false;
    }
    let mut raw_ace = null_mut();
    // SAFETY: the validated ACL reports exactly one entry.
    if unsafe { GetAce(dacl, 0, &mut raw_ace) } == 0 || raw_ace.is_null() {
        return false;
    }
    let expected_flags = ace_flags(kind);
    let expected_ace_size = match size_of::<ACCESS_ALLOWED_ACE>()
        .checked_sub(size_of::<u32>())
        .and_then(|base| base.checked_add(current_user.len()))
        .and_then(|size| u16::try_from(size).ok())
    {
        Some(size) => size,
        None => return false,
    };
    let expected_acl_size = match size_of::<ACL>().checked_add(usize::from(expected_ace_size)) {
        Some(size) => size,
        None => return false,
    };
    // SAFETY: GetAce returned a pointer to at least the header.
    let header = unsafe { &*raw_ace.cast::<ACE_HEADER>() };
    if header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8
        || header.AceFlags != expected_flags
        || header.AceSize != expected_ace_size
        || header.AceFlags & INHERITED_ACE as u8 != 0
        || info.AclBytesInUse as usize != expected_acl_size
        || info.AclBytesFree != 0
    {
        return false;
    }
    // SAFETY: exact AceSize bounds the ACCESS_ALLOWED_ACE and SID payload.
    let ace = unsafe { &*raw_ace.cast::<ACCESS_ALLOWED_ACE>() };
    // SAFETY: IsValidAcl proved a complete ACL header.
    let acl = unsafe { &*dacl };
    let ace_sid: PSID = std::ptr::addr_of!(ace.SidStart).cast_mut().cast();
    // SAFETY: exact AceSize bounds current_user.len() bytes at SidStart.
    let ace_sid_bytes = unsafe {
        std::slice::from_raw_parts(ace_sid.cast_const().cast::<u8>(), current_user.len())
    };
    acl.AclRevision == ACL_REVISION as u8
        && usize::from(acl.AclSize) == expected_acl_size
        && ace.Mask == FILE_ALL_ACCESS
        && ace_sid_bytes == current_user.as_bytes()
}

struct PrivateSecurity {
    _sid: OwnedSid,
    _acl: OwnedAcl,
    descriptor: SECURITY_DESCRIPTOR,
}

impl PrivateSecurity {
    fn new(kind: Kind) -> Result<Self> {
        let sid = OwnedSid::current().map_err(|_| unsafe_for(kind))?;
        let acl = OwnedAcl::new(&sid, kind).map_err(|_| unsafe_for(kind))?;
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        // SAFETY: descriptor is writable and the SID/ACL owners outlive its use.
        let initialized = unsafe {
            InitializeSecurityDescriptor((&raw mut descriptor).cast(), SECURITY_DESCRIPTOR_REVISION)
                != 0
                && SetSecurityDescriptorOwner((&raw mut descriptor).cast(), sid.as_ptr(), 0) != 0
                && SetSecurityDescriptorDacl((&raw mut descriptor).cast(), 1, acl.as_ptr(), 0) != 0
                && SetSecurityDescriptorControl(
                    (&raw mut descriptor).cast(),
                    SE_DACL_PROTECTED,
                    SE_DACL_PROTECTED,
                ) != 0
        };
        if !initialized {
            return Err(unsafe_for(kind));
        }
        Ok(Self {
            _sid: sid,
            _acl: acl,
            descriptor,
        })
    }
}

struct OwnedSid {
    words: Vec<usize>,
    length: usize,
}

impl OwnedSid {
    fn current() -> Result<Self> {
        let mut token = null_mut();
        // SAFETY: the returned real token handle is owned by TokenHandle.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(unsafe_object());
        }
        let _token = TokenHandle(token);
        let mut required = 0_u32;
        // SAFETY: documented size query with null buffer.
        let first = unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut required) };
        if first != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER || required == 0 {
            return Err(unsafe_object());
        }
        let word_count = (required as usize)
            .checked_add(size_of::<usize>() - 1)
            .and_then(|value| value.checked_div(size_of::<usize>()))
            .ok_or_else(unsafe_object)?;
        let mut token_words = vec![0_usize; word_count];
        // SAFETY: aligned buffer is at least `required` bytes.
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                token_words.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(unsafe_object());
        }
        // SAFETY: successful call initialized TOKEN_USER at buffer start.
        let source = unsafe { (*(token_words.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        if source.is_null() || unsafe { IsValidSid(source) } == 0 {
            return Err(unsafe_object());
        }
        // SAFETY: source is a valid SID.
        let length = unsafe { GetLengthSid(source) } as usize;
        if length == 0 || length > u16::MAX as usize {
            return Err(unsafe_object());
        }
        let sid_words = length
            .checked_add(size_of::<usize>() - 1)
            .and_then(|value| value.checked_div(size_of::<usize>()))
            .ok_or_else(unsafe_object)?;
        let mut words = vec![0_usize; sid_words];
        // SAFETY: destination covers the full validated SID length.
        if unsafe { CopySid(length as u32, words.as_mut_ptr().cast(), source) } == 0 {
            return Err(unsafe_object());
        }
        Ok(Self { words, length })
    }

    fn as_ptr(&self) -> PSID {
        self.words.as_ptr().cast_mut().cast()
    }

    const fn len(&self) -> usize {
        self.length
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: words owns at least length initialized copied SID bytes.
        unsafe { std::slice::from_raw_parts(self.words.as_ptr().cast(), self.length) }
    }
}

struct OwnedAcl {
    words: Vec<usize>,
}

impl OwnedAcl {
    fn new(current_user: &OwnedSid, kind: Kind) -> Result<Self> {
        let ace_size = size_of::<ACCESS_ALLOWED_ACE>()
            .checked_sub(size_of::<u32>())
            .and_then(|base| base.checked_add(current_user.len()))
            .ok_or_else(unsafe_object)?;
        let acl_size = size_of::<ACL>()
            .checked_add(ace_size)
            .ok_or_else(unsafe_object)?;
        let acl_size_u32 = u32::try_from(acl_size).map_err(|_| unsafe_object())?;
        let word_count = acl_size
            .checked_add(size_of::<usize>() - 1)
            .and_then(|value| value.checked_div(size_of::<usize>()))
            .ok_or_else(unsafe_object)?;
        let mut words = vec![0_usize; word_count];
        let acl = words.as_mut_ptr().cast::<ACL>();
        // SAFETY: aligned allocation covers acl_size; AddAccess copies the SID.
        if unsafe { InitializeAcl(acl, acl_size_u32, ACL_REVISION) } == 0
            || unsafe {
                AddAccessAllowedAceEx(
                    acl,
                    ACL_REVISION,
                    u32::from(ace_flags(kind)),
                    FILE_ALL_ACCESS,
                    current_user.as_ptr(),
                )
            } == 0
        {
            return Err(unsafe_object());
        }
        Ok(Self { words })
    }

    fn as_ptr(&self) -> *mut ACL {
        self.words.as_ptr().cast_mut().cast()
    }
}

struct TokenHandle(HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        // SAFETY: OpenProcessToken returned this owned handle exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

struct LocalSecurityDescriptor(*mut c_void);

impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: GetSecurityInfo allocated the descriptor with LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}

const fn ace_flags(kind: Kind) -> u8 {
    match kind {
        Kind::Directory => (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8,
        Kind::RegularFile => 0,
    }
}

const fn private_directory_access() -> u32 {
    FILE_GENERIC_READ | FILE_GENERIC_WRITE | READ_CONTROL | WRITE_DAC
}

const fn private_file_access(writable: bool) -> u32 {
    if writable {
        FILE_GENERIC_READ | FILE_GENERIC_WRITE | READ_CONTROL | WRITE_DAC
    } else {
        FILE_GENERIC_READ | READ_CONTROL
    }
}

fn component_units(name: &str) -> Result<Vec<u16>> {
    validate_component(name)?;
    let units: Vec<u16> = OsStr::new(name).encode_wide().collect();
    if units.is_empty()
        || units
            .len()
            .checked_mul(2)
            .is_none_or(|bytes| bytes > u16::MAX as usize)
    {
        return Err(unsafe_object());
    }
    Ok(units)
}

fn validate_component(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.encode_utf16().any(|unit| {
            unit == 0 || unit == b'/' as u16 || unit == b'\\' as u16 || unit == b':' as u16
        })
    {
        return Err(unsafe_object());
    }
    Ok(())
}

fn wide_path(path: &Path) -> Result<Vec<u16>> {
    let mut wide = Vec::new();
    for unit in path.as_os_str().encode_wide() {
        if unit == 0 {
            return Err(unsafe_root());
        }
        wide.push(unit);
    }
    if wide.is_empty() {
        return Err(unsafe_root());
    }
    wide.push(0);
    Ok(wide)
}

fn unsafe_for(kind: Kind) -> SnapshotStorageError {
    match kind {
        Kind::Directory => unsafe_root(),
        Kind::RegularFile => unsafe_object(),
    }
}

fn unsafe_root() -> SnapshotStorageError {
    SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeRoot)
}

fn unsafe_object() -> SnapshotStorageError {
    SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject)
}

fn unavailable() -> SnapshotStorageError {
    SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

    fn private_directory(temp: &TempDir) -> (File, PathBuf) {
        let parent = open_publication_parent(temp.path()).unwrap();
        let path = temp.path().join("snapshots");
        let directory = create_private_directory_exclusive(&parent, temp.path(), "snapshots")
            .unwrap()
            .expect("unique test directory");
        (directory, path)
    }

    #[test]
    fn created_directory_and_file_have_exact_protected_owner_dacls() {
        let temp = TempDir::new().unwrap();
        let (directory, path) = private_directory(&temp);
        let directory_identity = identity(&directory, Kind::Directory).unwrap();
        validate_retained(&directory, directory_identity, Kind::Directory, false).unwrap();

        let (file, file_identity) =
            create_private_file_exclusive(&directory, &path, "snapshot-stage.tmp")
                .unwrap()
                .expect("unique test file");
        validate_retained(&file, file_identity, Kind::RegularFile, true).unwrap();
        validate_named(
            &directory,
            "snapshot-stage.tmp",
            &file,
            file_identity,
            Kind::RegularFile,
        )
        .unwrap();

        let owner = OwnedSid::current().unwrap();
        assert!(inspect_private_security(&directory, Kind::Directory, &owner).unwrap());
        assert!(inspect_private_security(&file, Kind::RegularFile, &owner).unwrap());
    }

    #[test]
    fn retained_file_usage_comes_from_standard_handle_information() {
        let temp = TempDir::new().unwrap();
        let (directory, path) = private_directory(&temp);
        let (file, _) = create_private_file_exclusive(&directory, &path, "usage.tmp")
            .unwrap()
            .expect("unique usage file");
        (&file).write_all(b"physical usage").unwrap();
        file.sync_all().unwrap();

        let (logical_bytes, allocated_bytes) = file_usage(&file).unwrap();
        assert_eq!(logical_bytes, 14);
        assert!(allocated_bytes >= logical_bytes);
    }

    #[test]
    fn inherited_child_dacl_is_rejected_instead_of_repaired() {
        let temp = TempDir::new().unwrap();
        let (directory, path) = private_directory(&temp);
        fs::write(path.join("inherited.tmp"), b"inherited security").unwrap();

        let error = open_named_regular(&directory, &path, "inherited.tmp", false).unwrap_err();
        assert_eq!(error.kind(), SnapshotStorageErrorKind::UnsafeObject);
    }

    #[test]
    fn hard_linked_snapshot_file_is_rejected() {
        let temp = TempDir::new().unwrap();
        let (directory, path) = private_directory(&temp);
        let (file, file_identity) = create_private_file_exclusive(&directory, &path, "linked.tmp")
            .unwrap()
            .expect("unique test file");
        fs::hard_link(path.join("linked.tmp"), path.join("linked-alias.tmp")).unwrap();

        let error = validate_retained(&file, file_identity, Kind::RegularFile, true).unwrap_err();
        assert_eq!(error.kind(), SnapshotStorageErrorKind::UnsafeObject);
        assert_eq!(
            open_named_regular(&directory, &path, "linked.tmp", false)
                .unwrap_err()
                .kind(),
            SnapshotStorageErrorKind::UnsafeObject
        );
    }

    #[test]
    fn reparse_point_snapshot_entry_is_rejected_when_symlink_creation_is_available() {
        use std::os::windows::fs::symlink_file;

        let temp = TempDir::new().unwrap();
        let (directory, path) = private_directory(&temp);
        let target = temp.path().join("target.txt");
        fs::write(&target, b"outside target").unwrap();
        if let Err(error) = symlink_file(&target, path.join("reparse.tmp")) {
            if matches!(error.raw_os_error(), Some(5 | 1_314)) {
                return;
            }
            panic!("unexpected symlink creation failure: {error}");
        }

        let error = open_named_regular(&directory, &path, "reparse.tmp", false).unwrap_err();
        assert_eq!(error.kind(), SnapshotStorageErrorKind::UnsafeObject);
    }

    #[test]
    fn publication_is_no_replace_reopens_read_only_and_deletes_only_retained_temp() {
        let temp = TempDir::new().unwrap();
        let (directory, path) = private_directory(&temp);

        let (first, first_identity) = create_private_file_exclusive(&directory, &path, "first.tmp")
            .unwrap()
            .expect("unique first stage");
        (&first).write_all(b"first publication").unwrap();
        first.sync_all().unwrap();
        assert_eq!(
            publish_no_replace(
                &directory,
                "first.tmp",
                &first,
                first_identity,
                "snapshot-final.duxsnapshot",
            )
            .unwrap(),
            Publication::Published
        );
        validate_named(
            &directory,
            "snapshot-final.duxsnapshot",
            &first,
            first_identity,
            Kind::RegularFile,
        )
        .unwrap();
        drop(first);

        let (mut read_only, final_identity) =
            open_named_regular(&directory, &path, "snapshot-final.duxsnapshot", false)
                .unwrap()
                .expect("published file");
        assert!(read_only.write_all(b"must not mutate").is_err());
        validate_retained(&read_only, final_identity, Kind::RegularFile, true).unwrap();
        let mut published_bytes = Vec::new();
        read_only.read_to_end(&mut published_bytes).unwrap();
        assert_eq!(published_bytes, b"first publication");

        let (collision, collision_identity) =
            create_private_file_exclusive(&directory, &path, "collision.tmp")
                .unwrap()
                .expect("unique collision stage");
        (&collision)
            .write_all(b"must not replace the winner")
            .unwrap();
        collision.sync_all().unwrap();
        assert_eq!(
            publish_no_replace(
                &directory,
                "collision.tmp",
                &collision,
                collision_identity,
                "snapshot-final.duxsnapshot",
            )
            .unwrap(),
            Publication::Collision
        );
        remove_retained_temp(&directory, "collision.tmp", collision, collision_identity).unwrap();
        assert!(
            open_named_regular(&directory, &path, "collision.tmp", false)
                .unwrap()
                .is_none()
        );

        let (mut winner, winner_identity) =
            open_named_regular(&directory, &path, "snapshot-final.duxsnapshot", false)
                .unwrap()
                .expect("collision winner");
        assert_eq!(winner_identity, final_identity);
        let mut winner_bytes = Vec::new();
        winner.read_to_end(&mut winner_bytes).unwrap();
        assert_eq!(winner_bytes, b"first publication");
    }
}
