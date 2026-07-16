use std::ffi::{OsStr, OsString, c_void};
use std::fs::{self, File};
use std::mem::{MaybeUninit, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::FileExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, ERROR_FILE_NOT_FOUND,
    ERROR_INSUFFICIENT_BUFFER, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree, OBJ_CASE_INSENSITIVE, UNICODE_STRING,
};
use windows_sys::Win32::Security::Authorization::{
    GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_REVISION, ACL_SIZE_INFORMATION, AclSizeInformation,
    AddAccessAllowedAceEx, CONTAINER_INHERIT_ACE, CopySid, DACL_SECURITY_INFORMATION, EqualSid,
    GetAce, GetAclInformation, GetLengthSid, GetSecurityDescriptorControl, GetTokenInformation,
    INHERITED_ACE, InitializeAcl, InitializeSecurityDescriptor, IsValidAcl, IsValidSid,
    OBJECT_INHERIT_ACE, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSID,
    SE_DACL_PRESENT, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
    SECURITY_DESCRIPTOR_CONTROL, SetSecurityDescriptorControl, SetSecurityDescriptorDacl,
    SetSecurityDescriptorOwner, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateDirectoryW, CreateFileW, DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY,
    FILE_ALL_ACCESS, FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_ID_INFO,
    FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_STANDARD_INFO, FileBasicInfo, FileIdInfo, FileRenameInfo, FileStandardInfo,
    GetFileInformationByHandleEx, OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
};
use windows_sys::Win32::System::SystemServices::{
    ACCESS_ALLOWED_ACE_TYPE, SECURITY_DESCRIPTOR_REVISION,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{
    DatabaseOpenError, DatabaseOpenErrorKind, ObjectKind, PermissionPolicy, PlatformIdentity,
    PreparedRoot, PreparedRootState, RootPublicationResult, object_error, storage_root_error,
};

const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
const STAGING_ATTEMPTS: usize = 8;
const FILE_CREATE_DISPOSITION: u32 = 2;
const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
const FILE_OPEN_REPARSE_POINT_NATIVE: u32 = 0x0020_0000;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;

pub(super) fn sync_directory(directory: &File) -> Result<(), DatabaseOpenError> {
    directory
        .sync_all()
        .map_err(|_| object_error(DatabaseOpenErrorKind::DatabaseUnavailable))
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

pub(super) fn prepare_root_for_probe(root_path: &Path) -> Result<PreparedRoot, DatabaseOpenError> {
    let parent = root_path
        .parent()
        .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
    validate_ancestor_chain(parent)?;

    match open_target_raw(root_path, ObjectKind::Directory) {
        Ok(root) => {
            let identity = validate_file(
                &root,
                ObjectKind::Directory,
                None,
                PermissionPolicy::InspectOnly,
            )?;
            return Ok(PreparedRoot {
                directory: root,
                identity,
                object_path: root_path.to_path_buf(),
                state: PreparedRootState::Existing,
                publication_parent: None,
                publication_parent_identity: None,
            });
        }
        Err(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => {}
        Err(_) => return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot)),
    }

    let publication_parent = open_publication_parent(parent)?;
    let publication_parent_identity =
        validate_structure(&publication_parent, ObjectKind::Directory, None)?;
    for _ in 0..STAGING_ATTEMPTS {
        let staging_path = random_staging_path(parent)?;
        match create_private_directory(&staging_path) {
            Ok(root) => {
                let identity = validate_file(
                    &root,
                    ObjectKind::Directory,
                    None,
                    PermissionPolicy::RequirePrivate,
                )?;
                return Ok(PreparedRoot {
                    directory: root,
                    identity,
                    object_path: staging_path,
                    state: PreparedRootState::FreshStaged,
                    publication_parent: Some(publication_parent),
                    publication_parent_identity: Some(publication_parent_identity),
                });
            }
            Err(ERROR_ALREADY_EXISTS | ERROR_FILE_EXISTS) => {}
            Err(_) => {
                return Err(storage_root_error(
                    DatabaseOpenErrorKind::StorageRootUnavailable,
                ));
            }
        }
    }
    Err(storage_root_error(
        DatabaseOpenErrorKind::StorageRootUnavailable,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn publish_prepared_root(
    prepared: &PreparedRoot,
    final_root_path: &Path,
    database_name: &OsStr,
    database_file: &File,
    database_identity: PlatformIdentity,
    marker_name: &OsStr,
    marker_file: &File,
    marker_identity: PlatformIdentity,
) -> Result<RootPublicationResult, DatabaseOpenError> {
    if prepared.state != PreparedRootState::FreshStaged
        || prepared.object_path.parent() != final_root_path.parent()
    {
        return Err(storage_root_error(DatabaseOpenErrorKind::InternalState));
    }
    validate_retained_file(
        &prepared.directory,
        ObjectKind::Directory,
        prepared.identity,
        PermissionPolicy::RequirePrivate,
    )?;
    for (_name, file, identity) in [
        (database_name, database_file, database_identity),
        (marker_name, marker_file, marker_identity),
    ] {
        validate_retained_file(
            file,
            ObjectKind::RegularFile,
            identity,
            PermissionPolicy::RequirePrivate,
        )?;
    }

    let publication_parent = prepared
        .publication_parent
        .as_ref()
        .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
    let publication_parent_identity = prepared
        .publication_parent_identity
        .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::InternalState))?;
    validate_structure(
        publication_parent,
        ObjectKind::Directory,
        Some(publication_parent_identity),
    )?;
    let final_name = final_root_path
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))?;
    if let Err(error) =
        rename_by_handle_no_replace(&prepared.directory, publication_parent, final_name)
    {
        let destination_is_directory = open_ancestor(final_root_path)
            .and_then(|file| validate_structure(&file, ObjectKind::Directory, None).map(drop))
            .is_ok();
        return if matches!(error, ERROR_ALREADY_EXISTS | ERROR_FILE_EXISTS)
            || destination_is_directory
        {
            Ok(RootPublicationResult::Collision)
        } else {
            Err(storage_root_error(
                DatabaseOpenErrorKind::StorageRootUnavailable,
            ))
        };
    }

    validate_path_identity(
        final_root_path,
        ObjectKind::Directory,
        prepared.identity,
        PermissionPolicy::RequirePrivate,
    )?;
    for (name, file, identity) in [
        (database_name, database_file, database_identity),
        (marker_name, marker_file, marker_identity),
    ] {
        validate_retained_file(
            file,
            ObjectKind::RegularFile,
            identity,
            PermissionPolicy::RequirePrivate,
        )?;
        validate_path_identity(
            &final_root_path.join(name),
            ObjectKind::RegularFile,
            identity,
            PermissionPolicy::RequirePrivate,
        )?;
    }
    Ok(RootPublicationResult::Published)
}

pub(super) fn create_private_file_exclusive(
    root_directory: &File,
    _root_path: &Path,
    name: &OsStr,
) -> Result<(File, PlatformIdentity), DatabaseOpenError> {
    let file = create_private_file_relative(root_directory, name)?;
    let identity = validate_file(
        &file,
        ObjectKind::RegularFile,
        None,
        PermissionPolicy::RequirePrivate,
    )?;
    Ok((file, identity))
}

pub(super) fn open_existing_file(
    _root_directory: &File,
    root_path: &Path,
    name: &OsStr,
    permissions: PermissionPolicy,
) -> Result<Option<(File, PlatformIdentity)>, DatabaseOpenError> {
    let path = root_path.join(name);
    match open_target_raw(&path, ObjectKind::RegularFile) {
        Ok(file) => {
            let identity = validate_file(&file, ObjectKind::RegularFile, None, permissions)?;
            Ok(Some((file, identity)))
        }
        Err(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => Ok(None),
        Err(_) => Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject)),
    }
}

pub(super) fn open_existing_control_file(
    _root_directory: &File,
    root_path: &Path,
    name: &OsStr,
    permissions: PermissionPolicy,
) -> Result<Option<(File, PlatformIdentity)>, DatabaseOpenError> {
    let path = root_path.join(name);
    match open_control_raw(&path) {
        Ok(file) => {
            let identity = validate_file(&file, ObjectKind::RegularFile, None, permissions)?;
            Ok(Some((file, identity)))
        }
        Err(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => Ok(None),
        Err(_) => Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject)),
    }
}

pub(super) fn open_existing_writer_file(
    _root_directory: &File,
    root_path: &Path,
    name: &OsStr,
    permissions: PermissionPolicy,
) -> Result<Option<(File, PlatformIdentity)>, DatabaseOpenError> {
    let path = root_path.join(name);
    match open_control_raw(&path) {
        Ok(file) => {
            let identity = validate_file(&file, ObjectKind::RegularFile, None, permissions)?;
            Ok(Some((file, identity)))
        }
        Err(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => Ok(None),
        Err(_) => Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject)),
    }
}

pub(super) fn resolve_existing_file_name(
    root_directory: &File,
    root_path: &Path,
    _requested_name: &OsStr,
    expected: PlatformIdentity,
) -> Result<OsString, DatabaseOpenError> {
    let root_identity = validate_file(
        root_directory,
        ObjectKind::Directory,
        None,
        PermissionPolicy::InspectOnly,
    )?;
    validate_path_identity(
        root_path,
        ObjectKind::Directory,
        root_identity,
        PermissionPolicy::InspectOnly,
    )?;
    for entry in fs::read_dir(root_path)
        .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?
    {
        let name = entry
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?
            .file_name();
        let path = root_path.join(&name);
        if let Ok(file) = open_target_raw(&path, ObjectKind::RegularFile)
            && validate_file(
                &file,
                ObjectKind::RegularFile,
                Some(expected),
                PermissionPolicy::InspectOnly,
            )
            .is_ok()
        {
            validate_path_identity(
                root_path,
                ObjectKind::Directory,
                root_identity,
                PermissionPolicy::InspectOnly,
            )?;
            return Ok(name);
        }
    }
    validate_path_identity(
        root_path,
        ObjectKind::Directory,
        root_identity,
        PermissionPolicy::InspectOnly,
    )?;
    Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
}

pub(super) fn secure_retained_file(
    file: &File,
    kind: ObjectKind,
    expected: PlatformIdentity,
) -> Result<(), DatabaseOpenError> {
    validate_file(file, kind, Some(expected), PermissionPolicy::RepairPrivate).map(drop)
}

pub(super) fn open_root_rename_guard(
    path: &Path,
    expected: PlatformIdentity,
) -> Result<File, DatabaseOpenError> {
    let wide = wide_path(path)?;
    // SAFETY: `wide` is NUL-terminated and a successful owned handle is moved
    // into File. Excluding FILE_SHARE_DELETE makes this validated final-root
    // handle an OS-enforced rename/delete guard for its retained lifetime.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
    }
    // SAFETY: CreateFileW returned one unique owned handle and File closes it.
    let guard = unsafe { File::from_raw_handle(handle) };
    validate_file(
        &guard,
        ObjectKind::Directory,
        Some(expected),
        PermissionPolicy::RequirePrivate,
    )?;
    Ok(guard)
}

pub(super) fn reopen_published_root(
    publication_directory: File,
    path: &Path,
    expected: PlatformIdentity,
) -> Result<File, DatabaseOpenError> {
    validate_file(
        &publication_directory,
        ObjectKind::Directory,
        Some(expected),
        PermissionPolicy::RequirePrivate,
    )?;
    // `open_target` requests no DELETE access but retains SHARE_ALL. Opening it
    // before dropping the staging-only publication handle preserves an
    // uninterrupted identity proof while making the later no-delete-sharing
    // guard compatible with every remaining root handle.
    let steady_state = open_target(path, ObjectKind::Directory)?;
    validate_file(
        &steady_state,
        ObjectKind::Directory,
        Some(expected),
        PermissionPolicy::RequirePrivate,
    )?;
    drop(publication_directory);
    Ok(steady_state)
}

pub(super) fn validate_retained_file(
    file: &File,
    kind: ObjectKind,
    expected: PlatformIdentity,
    permissions: PermissionPolicy,
) -> Result<(), DatabaseOpenError> {
    validate_file(file, kind, Some(expected), permissions).map(drop)
}

pub(super) fn validate_path_identity(
    path: &Path,
    kind: ObjectKind,
    expected: PlatformIdentity,
    permissions: PermissionPolicy,
) -> Result<(), DatabaseOpenError> {
    let file = open_target(path, kind)?;
    validate_file(&file, kind, Some(expected), permissions).map(drop)
}

pub(super) fn read_exact_at(
    file: &File,
    mut buffer: &mut [u8],
    mut offset: u64,
) -> std::io::Result<()> {
    while !buffer.is_empty() {
        let read = file.seek_read(buffer, offset)?;
        if read == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        offset = offset
            .checked_add(read as u64)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        buffer = &mut buffer[read..];
    }
    Ok(())
}

pub(super) fn write_all_at(file: &File, mut buffer: &[u8], mut offset: u64) -> std::io::Result<()> {
    while !buffer.is_empty() {
        let written = file.seek_write(buffer, offset)?;
        if written == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
        }
        offset = offset
            .checked_add(written as u64)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        buffer = &buffer[written..];
    }
    Ok(())
}

pub(super) fn root_contains_only(
    root_directory: &File,
    root_path: &Path,
    allowed: &[&OsStr],
) -> Result<bool, DatabaseOpenError> {
    let expected = validate_file(
        root_directory,
        ObjectKind::Directory,
        None,
        PermissionPolicy::InspectOnly,
    )?;
    validate_file(
        &open_target(root_path, ObjectKind::Directory)?,
        ObjectKind::Directory,
        Some(expected),
        PermissionPolicy::InspectOnly,
    )?;
    for entry in fs::read_dir(root_path)
        .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?
    {
        let name = entry
            .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?
            .file_name();
        if !allowed.contains(&name.as_os_str()) {
            return Ok(false);
        }
    }
    validate_file(
        &open_target(root_path, ObjectKind::Directory)?,
        ObjectKind::Directory,
        Some(expected),
        PermissionPolicy::InspectOnly,
    )?;
    Ok(true)
}

fn create_private_file(path: &Path) -> Result<File, DatabaseOpenError> {
    create_private_file_raw(path)
        .map_err(|_| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))
}

fn create_private_file_relative(
    root_directory: &File,
    name: &OsStr,
) -> Result<File, DatabaseOpenError> {
    let mut name: Vec<u16> = name.encode_wide().collect();
    if name.is_empty()
        || name.iter().any(|unit| {
            *unit == 0
                || *unit == u16::from(b'/')
                || *unit == u16::from(b'\\')
                || *unit == u16::from(b':')
        })
    {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    let name_bytes = name
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafeStorageObject))?;
    let mut object_name = UNICODE_STRING {
        Length: name_bytes,
        MaximumLength: name_bytes,
        Buffer: name.as_mut_ptr(),
    };
    let mut security = PrivateSecurity::new(ObjectKind::RegularFile)?;
    let mut attributes = ObjectAttributes {
        length: size_of::<ObjectAttributes>() as u32,
        root_directory: root_directory.as_raw_handle(),
        object_name: &mut object_name,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: (&raw mut security.descriptor).cast(),
        security_quality_of_service: null_mut(),
    };
    let mut io_status = IoStatusBlock {
        value: IoStatusValue { status: 0 },
        information: 0,
    };
    let mut handle = INVALID_HANDLE_VALUE;
    // SAFETY: the retained directory handle and every pointer in the object
    // attributes remain live for this call. FILE_CREATE is exclusive, the
    // relative leaf cannot escape RootDirectory, and SYNCHRONOUS_IO_NONALERT
    // makes the resulting handle compatible with std::fs::File's null-
    // OVERLAPPED reads and writes.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | WRITE_DAC,
            &mut attributes,
            &mut io_status,
            null(),
            FILE_ATTRIBUTE_NORMAL,
            SHARE_ALL,
            FILE_CREATE_DISPOSITION,
            FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT_NATIVE | FILE_SYNCHRONOUS_IO_NONALERT,
            null(),
            0,
        )
    };
    if status < 0 || handle == INVALID_HANDLE_VALUE {
        return Err(object_error(DatabaseOpenErrorKind::UnsafeStorageObject));
    }
    // SAFETY: NtCreateFile returned one unique owned handle and File closes it.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn create_private_directory(path: &Path) -> Result<File, u32> {
    let mut security = PrivateSecurity::new(ObjectKind::Directory)
        .map_err(|_| windows_sys::Win32::Foundation::ERROR_INVALID_SECURITY_DESCR)?;
    let wide =
        wide_path_raw(path).map_err(|_| windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    // SAFETY: `wide` is NUL-terminated and the descriptor, ACL, and owner SID
    // remain live for the complete synchronous creation call.
    if unsafe { CreateDirectoryW(wide.as_ptr(), &security.attributes()) } == 0 {
        // SAFETY: GetLastError immediately follows the failed Win32 call.
        return Err(unsafe { GetLastError() });
    }
    open_directory_for_publication(path)
}

fn open_publication_parent(path: &Path) -> Result<File, DatabaseOpenError> {
    let wide = wide_path(path)?;
    // SAFETY: `wide` is NUL-terminated and a successful owned handle is moved
    // into File. Adding a child directory requires FILE_ADD_SUBDIRECTORY.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES | FILE_ADD_SUBDIRECTORY,
            SHARE_ALL,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(storage_root_error(
            DatabaseOpenErrorKind::StorageRootUnavailable,
        ));
    }
    // SAFETY: CreateFileW returned a unique owned handle and File closes it.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn open_directory_for_publication(path: &Path) -> Result<File, u32> {
    let wide =
        wide_path_raw(path).map_err(|_| windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    // SAFETY: `wide` is NUL-terminated and a successful owned handle is moved
    // into File. DELETE is required for handle-bound FileRenameInfo.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES | FILE_ADD_FILE | READ_CONTROL | WRITE_DAC | DELETE,
            SHARE_ALL,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        // SAFETY: GetLastError immediately follows the failed Win32 call.
        return Err(unsafe { GetLastError() });
    }
    // SAFETY: CreateFileW returned a unique owned handle and File closes it.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn rename_by_handle_no_replace(
    source: &File,
    destination_parent: &File,
    destination_name: &OsStr,
) -> Result<(), u32> {
    let name: Vec<u16> = destination_name.encode_wide().collect();
    if name.is_empty() || name.contains(&0) {
        return Err(windows_sys::Win32::Foundation::ERROR_INVALID_NAME);
    }
    let name_bytes = name
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|size| u32::try_from(size).ok())
        .ok_or(windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    let info_size = std::mem::offset_of!(FILE_RENAME_INFO, FileName)
        .checked_add(name_bytes as usize)
        .ok_or(windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    let buffer_words = info_size.div_ceil(size_of::<usize>());
    let mut buffer = vec![0_usize; buffer_words];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: the usize buffer provides sufficient alignment and `info_size`
    // bytes. FILE_RENAME_INFO ends in a flexible UTF-16 array copied below.
    unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = destination_parent.as_raw_handle();
        (*info).FileNameLength = name_bytes;
        std::ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
    }
    let info_size =
        u32::try_from(info_size).map_err(|_| windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    // SAFETY: both handles remain live for the synchronous call and `info`
    // points to an aligned, initialized buffer of exactly `info_size` bytes.
    if unsafe {
        // DUX-DESTRUCTIVE: allow=storage-root-handle-publish -- no-replace rename publishes only the ACL-validated retained staging directory handle
        windows_sys::Win32::Storage::FileSystem::SetFileInformationByHandle(
            source.as_raw_handle(),
            FileRenameInfo,
            info.cast(),
            info_size,
        )
    } == 0
    {
        // SAFETY: GetLastError immediately follows the failed Win32 call.
        return Err(unsafe { GetLastError() });
    }
    Ok(())
}

fn random_staging_path(parent: &Path) -> Result<PathBuf, DatabaseOpenError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| storage_root_error(DatabaseOpenErrorKind::StorageRootUnavailable))?;
    let mut name = OsString::from(".dux-stage-");
    let mut suffix = String::with_capacity(random.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in random {
        suffix.push(char::from(HEX[usize::from(byte >> 4)]));
        suffix.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    name.push(suffix);
    Ok(parent.join(name))
}

fn create_private_file_raw(path: &Path) -> Result<File, u32> {
    let mut security = PrivateSecurity::new(ObjectKind::RegularFile)
        .map_err(|_| windows_sys::Win32::Foundation::ERROR_INVALID_SECURITY_DESCR)?;
    let wide =
        wide_path_raw(path).map_err(|_| windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    // SAFETY: the path is NUL-terminated; the security descriptor, ACL, and
    // SID outlive the call; and a successful owned handle is moved into File.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | WRITE_DAC,
            SHARE_ALL,
            &security.attributes(),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        // SAFETY: GetLastError immediately follows the failed Win32 call.
        return Err(unsafe { GetLastError() });
    }
    // SAFETY: CreateFileW returned a unique owned handle and File closes it.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn open_target(path: &Path, kind: ObjectKind) -> Result<File, DatabaseOpenError> {
    open_target_raw(path, kind).map_err(|_| unsafe_for(kind))
}

fn open_target_raw(path: &Path, kind: ObjectKind) -> Result<File, u32> {
    let wide =
        wide_path_raw(path).map_err(|_| windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    let (access, flags) = match kind {
        ObjectKind::Directory => (
            FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        ),
        ObjectKind::RegularFile => (
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | WRITE_DAC,
            FILE_FLAG_OPEN_REPARSE_POINT,
        ),
    };
    // SAFETY: `wide` is NUL-terminated and a successful owned handle is moved
    // into File. Null security attributes are correct for an existing open.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            SHARE_ALL,
            null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        // SAFETY: GetLastError immediately follows the failed Win32 call.
        return Err(unsafe { GetLastError() });
    }
    // SAFETY: CreateFileW returned a unique owned handle and File closes it.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn open_control_raw(path: &Path) -> Result<File, u32> {
    let wide =
        wide_path_raw(path).map_err(|_| windows_sys::Win32::Foundation::ERROR_INVALID_NAME)?;
    // Retained cleanup control handles deliberately omit FILE_SHARE_DELETE.
    // LockFileEx would otherwise remain on a displaced file while another
    // process opened and locked a replacement at the original pathname.
    // SAFETY: `wide` is NUL-terminated, null security attributes are correct
    // for an existing file, and the successful handle is moved into `File`.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_GENERIC_READ | FILE_GENERIC_WRITE | WRITE_DAC,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        // SAFETY: GetLastError immediately follows the failed Win32 call.
        return Err(unsafe { GetLastError() });
    }
    // SAFETY: CreateFileW returned a unique owned handle and File closes it.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn open_ancestor(path: &Path) -> Result<File, DatabaseOpenError> {
    let wide = wide_path(path)?;
    // Ancestors are structural probes only: no ACL/owner authority is needed.
    // SAFETY: `wide` is NUL-terminated and a successful handle is owned.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            SHARE_ALL,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot));
    }
    // SAFETY: CreateFileW returned a unique owned handle and File closes it.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn validate_ancestor_chain(path: &Path) -> Result<(), DatabaseOpenError> {
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        validate_structure(&open_ancestor(ancestor)?, ObjectKind::Directory, None)?;
    }
    Ok(())
}

fn validate_file(
    file: &File,
    kind: ObjectKind,
    expected: Option<PlatformIdentity>,
    permissions: PermissionPolicy,
) -> Result<PlatformIdentity, DatabaseOpenError> {
    let identity = validate_structure(file, kind, expected)?;
    let owner = OwnedSid::current()?;
    let exact = inspect_private_security(file, kind, &owner)?;
    match permissions {
        PermissionPolicy::InspectOnly => {}
        PermissionPolicy::RepairPrivate if !exact => {
            apply_private_dacl(file, kind, &owner)?;
            if !inspect_private_security(file, kind, &owner)? {
                return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
            }
        }
        PermissionPolicy::RequirePrivate if !exact => {
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        PermissionPolicy::RepairPrivate | PermissionPolicy::RequirePrivate => {}
    }
    Ok(identity)
}

fn validate_structure(
    file: &File,
    kind: ObjectKind,
    expected: Option<PlatformIdentity>,
) -> Result<PlatformIdentity, DatabaseOpenError> {
    let mut id = MaybeUninit::<FILE_ID_INFO>::zeroed();
    let mut basic = MaybeUninit::<FILE_BASIC_INFO>::zeroed();
    let mut standard = MaybeUninit::<FILE_STANDARD_INFO>::zeroed();
    let handle = file.as_raw_handle();
    // SAFETY: `file` retains a valid handle and each output buffer has the
    // exact type and length requested by its information class.
    let id_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            id.as_mut_ptr().cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    // SAFETY: identical reasoning, with the matching basic-info buffer.
    let basic_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            basic.as_mut_ptr().cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    // SAFETY: identical reasoning, with the matching standard-info buffer.
    let standard_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileStandardInfo,
            standard.as_mut_ptr().cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    if id_ok == 0 || basic_ok == 0 || standard_ok == 0 {
        return Err(object_error(DatabaseOpenErrorKind::DatabaseUnavailable));
    }
    // SAFETY: all three successful calls initialized their full buffers.
    let (id, basic, standard) = unsafe {
        (
            id.assume_init(),
            basic.assume_init(),
            standard.assume_init(),
        )
    };
    let attributes = basic.FileAttributes;
    if attributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DEVICE) != 0 {
        return Err(unsafe_for(kind));
    }
    let is_directory = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    if is_directory != matches!(kind, ObjectKind::Directory)
        || (matches!(kind, ObjectKind::RegularFile) && standard.NumberOfLinks != 1)
    {
        return Err(unsafe_for(kind));
    }

    let identity = PlatformIdentity {
        volume_serial: id.VolumeSerialNumber,
        file_id: id.FileId.Identifier,
    };
    if expected.is_some_and(|expected| expected != identity) {
        return Err(unsafe_for(kind));
    }
    Ok(identity)
}

fn inspect_private_security(
    file: &File,
    kind: ObjectKind,
    current_user: &OwnedSid,
) -> Result<bool, DatabaseOpenError> {
    let mut owner: PSID = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: all output pointers are writable and the returned descriptor is
    // retained by `LocalSecurityDescriptor` until its interior pointers are
    // no longer used.
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
        return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
    }
    let _descriptor = descriptor_guard.expect("nonnull descriptor was guarded");
    // SAFETY: both SIDs remain live through this comparison.
    if unsafe { IsValidSid(owner) } == 0 || unsafe { EqualSid(owner, current_user.as_ptr()) } == 0 {
        return Err(object_error(DatabaseOpenErrorKind::OwnershipMismatch));
    }

    let mut control: SECURITY_DESCRIPTOR_CONTROL = 0;
    let mut revision = 0_u32;
    // SAFETY: the descriptor is live and both output pointers are writable.
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
    }
    Ok((control & SE_DACL_PRESENT) != 0
        && (control & SE_DACL_PROTECTED) != 0
        && dacl_is_exact(dacl, kind, current_user))
}

fn dacl_is_exact(dacl: *const ACL, kind: ObjectKind, current_user: &OwnedSid) -> bool {
    if dacl.is_null() {
        return false;
    }
    // SAFETY: the caller retains the security descriptor that owns `dacl`.
    if unsafe { IsValidAcl(dacl) } == 0 {
        return false;
    }
    let mut info = MaybeUninit::<ACL_SIZE_INFORMATION>::zeroed();
    // SAFETY: `dacl` is valid and the output buffer matches the info class.
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
    // SAFETY: GetAclInformation initialized the full output buffer.
    let info = unsafe { info.assume_init() };
    if info.AceCount != 1 {
        return false;
    }

    let mut raw_ace = null_mut();
    // SAFETY: the valid ACL reports exactly one entry and output is writable.
    if unsafe { GetAce(dacl, 0, &mut raw_ace) } == 0 || raw_ace.is_null() {
        return false;
    }
    let expected_flags = ace_flags(kind);
    let sid_length = current_user.len();
    let expected_ace_size = match size_of::<ACCESS_ALLOWED_ACE>()
        .checked_sub(size_of::<u32>())
        .and_then(|base| base.checked_add(sid_length))
        .and_then(|size| u16::try_from(size).ok())
    {
        Some(size) => size,
        None => return false,
    };
    let expected_acl_size = match size_of::<ACL>().checked_add(usize::from(expected_ace_size)) {
        Some(size) => size,
        None => return false,
    };
    // SAFETY: GetAce returned a live pointer to at least an ACE header. Check
    // its validated extent before interpreting the larger allowed-ACE shape.
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
    // SAFETY: IsValidAcl bounded this entry and its checked AceSize now covers
    // ACCESS_ALLOWED_ACE plus the exact current-user SID payload.
    let ace = unsafe { &*raw_ace.cast::<ACCESS_ALLOWED_ACE>() };
    // SAFETY: dacl was validated above and therefore contains a full ACL header.
    let acl = unsafe { &*dacl };
    let ace_sid: PSID = std::ptr::addr_of!(ace.SidStart).cast_mut().cast();
    // SAFETY: the exact checked ACE size bounds precisely current_user.len()
    // bytes beginning at SidStart inside the validated ACL allocation.
    let ace_sid_bytes = unsafe {
        std::slice::from_raw_parts(ace_sid.cast_const().cast::<u8>(), current_user.len())
    };
    acl.AclRevision == ACL_REVISION as u8
        && usize::from(acl.AclSize) == expected_acl_size
        && ace.Mask == FILE_ALL_ACCESS
        && ace_sid_bytes == current_user.as_bytes()
}

fn apply_private_dacl(
    file: &File,
    kind: ObjectKind,
    current_user: &OwnedSid,
) -> Result<(), DatabaseOpenError> {
    let acl = OwnedAcl::new(current_user, kind)?;
    // SAFETY: the file handle has WRITE_DAC, and `acl` remains live for this
    // synchronous call. Owner/group/SACL are intentionally unchanged.
    let result = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl.as_ptr(),
            null(),
        )
    };
    if result != ERROR_SUCCESS {
        return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
    }
    Ok(())
}

struct PrivateSecurity {
    _sid: OwnedSid,
    _acl: OwnedAcl,
    descriptor: SECURITY_DESCRIPTOR,
}

impl PrivateSecurity {
    fn new(kind: ObjectKind) -> Result<Self, DatabaseOpenError> {
        let sid = OwnedSid::current()?;
        let acl = OwnedAcl::new(&sid, kind)?;
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        // SAFETY: the descriptor is writable, and the SID/ACL heap buffers
        // remain stable after their owning Vec values move into this struct.
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
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        Ok(Self {
            _sid: sid,
            _acl: acl,
            descriptor,
        })
    }

    fn attributes(&mut self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: (&raw mut self.descriptor).cast(),
            bInheritHandle: 0,
        }
    }
}

struct OwnedSid {
    words: Vec<usize>,
    length: usize,
}

impl OwnedSid {
    fn current() -> Result<Self, DatabaseOpenError> {
        let mut token = null_mut();
        // SAFETY: the output pointer is writable; the returned real token
        // handle is closed by TokenHandle rather than the pseudo-process handle.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        let _token = TokenHandle(token);
        let mut required = 0_u32;
        // SAFETY: this is the documented size-query call with a null buffer.
        let first = unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut required) };
        // SAFETY: GetLastError immediately follows GetTokenInformation.
        if first != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER || required == 0 {
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        let byte_len = required as usize;
        let word_count = byte_len
            .checked_add(size_of::<usize>() - 1)
            .and_then(|value| value.checked_div(size_of::<usize>()))
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafePermissions))?;
        let mut token_words = vec![0_usize; word_count];
        // SAFETY: the aligned buffer is at least `required` bytes long and the
        // reported TokenUser structure is valid only while this Vec is live.
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
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        // SAFETY: the successful call initialized a TOKEN_USER at the buffer.
        let source = unsafe { (*(token_words.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        // SAFETY: TokenUser promises a SID pointer within the live token buffer.
        if source.is_null() || unsafe { IsValidSid(source) } == 0 {
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        // SAFETY: source is a valid SID.
        let sid_length = unsafe { GetLengthSid(source) } as usize;
        if sid_length == 0 || sid_length > u16::MAX as usize {
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        let sid_word_count = sid_length
            .checked_add(size_of::<usize>() - 1)
            .and_then(|value| value.checked_div(size_of::<usize>()))
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafePermissions))?;
        let mut words = vec![0_usize; sid_word_count];
        // SAFETY: destination holds at least sid_length bytes and source is valid.
        if unsafe { CopySid(sid_length as u32, words.as_mut_ptr().cast(), source) } == 0 {
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        Ok(Self {
            words,
            length: sid_length,
        })
    }

    fn as_ptr(&self) -> PSID {
        self.words.as_ptr().cast_mut().cast()
    }

    const fn len(&self) -> usize {
        self.length
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: `words` owns at least `length` initialized copied SID bytes.
        unsafe { std::slice::from_raw_parts(self.words.as_ptr().cast(), self.length) }
    }
}

struct OwnedAcl {
    words: Vec<usize>,
}

impl OwnedAcl {
    fn new(current_user: &OwnedSid, kind: ObjectKind) -> Result<Self, DatabaseOpenError> {
        Self::new_parts(current_user, ace_flags(kind), FILE_ALL_ACCESS)
    }

    fn new_parts(
        current_user: &OwnedSid,
        flags: u8,
        access_mask: u32,
    ) -> Result<Self, DatabaseOpenError> {
        let ace_size = size_of::<ACCESS_ALLOWED_ACE>()
            .checked_sub(size_of::<u32>())
            .and_then(|base| base.checked_add(current_user.len()))
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafePermissions))?;
        let acl_size = size_of::<ACL>()
            .checked_add(ace_size)
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafePermissions))?;
        let acl_size_u32 = u32::try_from(acl_size)
            .map_err(|_| object_error(DatabaseOpenErrorKind::UnsafePermissions))?;
        let word_count = acl_size
            .checked_add(size_of::<usize>() - 1)
            .and_then(|value| value.checked_div(size_of::<usize>()))
            .ok_or_else(|| object_error(DatabaseOpenErrorKind::UnsafePermissions))?;
        let mut words = vec![0_usize; word_count];
        let acl = words.as_mut_ptr().cast::<ACL>();
        // SAFETY: the aligned buffer is at least acl_size bytes; AddAccess...
        // copies the live current-user SID into the single ACE.
        if unsafe { InitializeAcl(acl, acl_size_u32, ACL_REVISION) } == 0
            || unsafe {
                AddAccessAllowedAceEx(
                    acl,
                    ACL_REVISION,
                    u32::from(flags),
                    access_mask,
                    current_user.as_ptr(),
                )
            } == 0
        {
            return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
        }
        Ok(Self { words })
    }

    fn as_ptr(&self) -> *mut ACL {
        self.words.as_ptr().cast_mut().cast()
    }
}

struct TokenHandle(windows_sys::Win32::Foundation::HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        // SAFETY: OpenProcessToken returned this owned handle exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

struct LocalSecurityDescriptor(*mut c_void);

impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: GetSecurityInfo allocated this descriptor with LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}

const fn ace_flags(kind: ObjectKind) -> u8 {
    match kind {
        ObjectKind::Directory => (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8,
        ObjectKind::RegularFile => 0,
    }
}

fn wide_path(path: &Path) -> Result<Vec<u16>, DatabaseOpenError> {
    wide_path_raw(path).map_err(|_| storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot))
}

fn wide_path_raw(path: &Path) -> Result<Vec<u16>, ()> {
    let mut wide = Vec::new();
    for unit in path.as_os_str().encode_wide() {
        if unit == 0 {
            return Err(());
        }
        wide.push(unit);
    }
    if wide.is_empty() {
        return Err(());
    }
    wide.push(0);
    Ok(wide)
}

fn unsafe_for(kind: ObjectKind) -> DatabaseOpenError {
    match kind {
        ObjectKind::Directory => storage_root_error(DatabaseOpenErrorKind::UnsafeStorageRoot),
        ObjectKind::RegularFile => object_error(DatabaseOpenErrorKind::UnsafeStorageObject),
    }
}

#[cfg(test)]
pub(super) enum TestAclShape {
    Broad,
    Inherited,
    Null,
}

#[cfg(test)]
pub(super) fn replace_acl_for_test(
    path: &Path,
    kind: ObjectKind,
    shape: TestAclShape,
) -> Result<(), DatabaseOpenError> {
    let file = open_target(path, kind)?;
    let sid = OwnedSid::current()?;
    let acl = match shape {
        TestAclShape::Broad => Some(OwnedAcl::new_parts(
            &sid,
            ace_flags(kind),
            FILE_ALL_ACCESS | 0x8000_0000,
        )?),
        TestAclShape::Inherited => Some(OwnedAcl::new_parts(
            &sid,
            ace_flags(kind) | INHERITED_ACE as u8,
            FILE_ALL_ACCESS,
        )?),
        TestAclShape::Null => None,
    };
    let dacl = acl.as_ref().map_or(null(), |acl| acl.as_ptr().cast_const());
    // SAFETY: the target handle has WRITE_DAC and the optional ACL remains
    // live for the call. A null DACL is intentional hostile test input.
    let result = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            dacl,
            null(),
        )
    };
    if result != ERROR_SUCCESS {
        return Err(object_error(DatabaseOpenErrorKind::UnsafePermissions));
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn assert_private_for_test(
    path: &Path,
    kind: ObjectKind,
) -> Result<(), DatabaseOpenError> {
    let file = open_target(path, kind)?;
    let sid = OwnedSid::current()?;
    if inspect_private_security(&file, kind, &sid)? {
        Ok(())
    } else {
        Err(object_error(DatabaseOpenErrorKind::UnsafePermissions))
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::super::SecureStorePaths;
    use super::*;

    const TEST_MARKER: &[u8; 16] = b"DUXWRITERLOCK1\0\0";

    fn add_staged_contents(
        prepared: &PreparedRoot,
    ) -> (
        OsString,
        File,
        PlatformIdentity,
        OsString,
        File,
        PlatformIdentity,
    ) {
        let database_name = OsString::from("dux.sqlite3");
        let marker_name = OsString::from("dux.sqlite3.writer.lock");
        let (marker, marker_identity) =
            create_private_file_exclusive(&prepared.directory, &prepared.object_path, &marker_name)
                .unwrap();
        write_all_at(&marker, TEST_MARKER, 0).unwrap();
        marker.sync_all().unwrap();
        let (database, database_identity) = create_private_file_exclusive(
            &prepared.directory,
            &prepared.object_path,
            &database_name,
        )
        .unwrap();
        database.sync_all().unwrap();
        (
            database_name,
            database,
            database_identity,
            marker_name,
            marker,
            marker_identity,
        )
    }

    #[test]
    fn exact_acl_shape_rejects_null_broad_and_inherited_dacls() {
        let sid = OwnedSid::current().unwrap();
        let exact_file = OwnedAcl::new(&sid, ObjectKind::RegularFile).unwrap();
        let exact_root = OwnedAcl::new(&sid, ObjectKind::Directory).unwrap();
        let broad = OwnedAcl::new_parts(&sid, 0, FILE_ALL_ACCESS | 0x8000_0000).unwrap();
        let inherited = OwnedAcl::new_parts(&sid, INHERITED_ACE as u8, FILE_ALL_ACCESS).unwrap();

        assert!(dacl_is_exact(
            exact_file.as_ptr(),
            ObjectKind::RegularFile,
            &sid
        ));
        assert!(dacl_is_exact(
            exact_root.as_ptr(),
            ObjectKind::Directory,
            &sid
        ));
        assert!(!dacl_is_exact(null(), ObjectKind::RegularFile, &sid));
        assert!(!dacl_is_exact(
            broad.as_ptr(),
            ObjectKind::RegularFile,
            &sid
        ));
        assert!(!dacl_is_exact(
            inherited.as_ptr(),
            ObjectKind::RegularFile,
            &sid
        ));
        assert!(!dacl_is_exact(
            exact_root.as_ptr(),
            ObjectKind::RegularFile,
            &sid
        ));
    }

    #[test]
    fn staged_root_publishes_without_changing_retained_identities() {
        let temp = TempDir::new().unwrap();
        let final_root = temp.path().join("owned");
        let prepared = prepare_root_for_probe(&final_root).unwrap();
        assert_eq!(prepared.state, PreparedRootState::FreshStaged);
        assert_ne!(prepared.object_path, final_root);
        assert!(!final_root.exists());
        assert_private_for_test(&prepared.object_path, ObjectKind::Directory).unwrap();
        let (database_name, database, database_identity, marker_name, marker, marker_identity) =
            add_staged_contents(&prepared);

        assert_eq!(
            publish_prepared_root(
                &prepared,
                &final_root,
                &database_name,
                &database,
                database_identity,
                &marker_name,
                &marker,
                marker_identity,
            )
            .unwrap(),
            RootPublicationResult::Published
        );
        assert!(final_root.is_dir());
        assert!(!prepared.object_path.exists());
        assert_private_for_test(&final_root, ObjectKind::Directory).unwrap();
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn staged_root_source_path_swap_never_publishes_the_replacement() {
        let temp = TempDir::new().unwrap();
        let final_root = temp.path().join("owned");
        let prepared = prepare_root_for_probe(&final_root).unwrap();
        let original_stage_path = prepared.object_path.clone();
        let displaced_stage_path = temp.path().join("displaced-stage");
        // DUX-DESTRUCTIVE: allow=test-storage-root-source-swap -- move only the TempDir-owned stage fixture to prove all subsequent writes remain handle-relative
        fs::rename(&original_stage_path, &displaced_stage_path).unwrap();
        let replacement = create_private_directory(&original_stage_path).unwrap();
        let replacement_identity =
            validate_structure(&replacement, ObjectKind::Directory, None).unwrap();
        let replacement_sentinel_path = original_stage_path.join("replacement-sentinel");
        let replacement_sentinel = create_private_file(&replacement_sentinel_path).unwrap();
        replacement_sentinel.sync_all().unwrap();
        let (database_name, database, database_identity, marker_name, marker, marker_identity) =
            add_staged_contents(&prepared);

        assert!(displaced_stage_path.join(&database_name).is_file());
        assert!(displaced_stage_path.join(&marker_name).is_file());
        assert!(!original_stage_path.join(&database_name).exists());
        assert!(!original_stage_path.join(&marker_name).exists());

        assert_eq!(
            publish_prepared_root(
                &prepared,
                &final_root,
                &database_name,
                &database,
                database_identity,
                &marker_name,
                &marker,
                marker_identity,
            )
            .unwrap(),
            RootPublicationResult::Published
        );
        assert_eq!(
            validate_structure(
                &open_target(&final_root, ObjectKind::Directory).unwrap(),
                ObjectKind::Directory,
                None,
            )
            .unwrap(),
            prepared.identity
        );
        assert!(!displaced_stage_path.exists());
        assert!(replacement_sentinel_path.is_file());
        assert_eq!(
            validate_structure(
                &open_target(&original_stage_path, ObjectKind::Directory).unwrap(),
                ObjectKind::Directory,
                None,
            )
            .unwrap(),
            replacement_identity
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn fresh_store_reopens_before_blocking_external_final_root_rename() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("owned");
        let database = root.join("dux.sqlite3");
        let replacement_path = temp.path().join("moved-owned");
        let storage = SecureStorePaths::prepare(&database).unwrap();

        // DUX-DESTRUCTIVE: allow=test-storage-final-root-rename-guard -- attempt to move only the TempDir-owned live store to prove its retained guard denies external replacement
        assert!(fs::rename(&root, &replacement_path).is_err());
        assert!(root.is_dir());
        assert!(!replacement_path.exists());
        storage.validate_for_database_open().unwrap();
    }

    #[test]
    fn staged_root_collision_never_replaces_the_winner() {
        let temp = TempDir::new().unwrap();
        let final_root = temp.path().join("owned");
        let prepared = prepare_root_for_probe(&final_root).unwrap();
        let (database_name, database, database_identity, marker_name, marker, marker_identity) =
            add_staged_contents(&prepared);
        let winner = create_private_directory(&final_root).unwrap();
        let winner_identity = validate_structure(&winner, ObjectKind::Directory, None).unwrap();

        assert_eq!(
            publish_prepared_root(
                &prepared,
                &final_root,
                &database_name,
                &database,
                database_identity,
                &marker_name,
                &marker,
                marker_identity,
            )
            .unwrap(),
            RootPublicationResult::Collision
        );
        assert!(prepared.object_path.is_dir());
        assert_eq!(
            validate_structure(
                &open_target(&final_root, ObjectKind::Directory).unwrap(),
                ObjectKind::Directory,
                None,
            )
            .unwrap(),
            winner_identity
        );
    }
}
