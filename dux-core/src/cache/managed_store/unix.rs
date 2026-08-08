use std::ffi::CString;
use std::fs::File;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use std::time::Instant;

use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::{OFlag, open, openat};
use nix::sys::stat::{Mode, SFlag, fchmod, fstat, mkdirat};
use nix::unistd::geteuid;

use super::{
    ManagedCacheStoreAccess, ManagedCacheStoreError, ManagedCacheStoreErrorKind, Result, budget,
    unavailable, unsafe_object, unsafe_store,
};

const DIRECTORY_MODE: Mode = Mode::S_IRWXU;
const FILE_MODE: Mode = Mode::S_IRUSR.union(Mode::S_IWUSR);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
}

pub(super) const fn identity_parts(identity: Identity) -> (u64, u64) {
    (identity.device, identity.inode)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ChangeToken {
    change_seconds: i64,
    change_nanoseconds: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    ContainerDirectory,
    PrivateDirectory,
    PrivateFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Publication {
    Published,
    Collision,
}

pub(super) fn open_container_parent(path: &Path) -> Result<(File, Identity)> {
    let parent_path = path.parent().ok_or_else(unsafe_container)?;
    let parent = open(
        parent_path,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(map_container_error)?;
    validate_publication_parent(&parent)?;
    let identity = identity(&parent, Kind::ContainerDirectory)?;
    validate_retained(&parent, identity, Kind::ContainerDirectory, false)?;
    Ok((parent, identity))
}

pub(super) fn open_or_create_child_container(
    parent: &File,
    path: &Path,
    access: ManagedCacheStoreAccess,
) -> Result<Option<(File, Identity)>> {
    let name = path.file_name().ok_or_else(unsafe_container)?;
    match openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(descriptor) => {
            let directory = File::from(descriptor);
            let identity = identity(&directory, Kind::ContainerDirectory)?;
            validate_retained(&directory, identity, Kind::ContainerDirectory, false)?;
            Ok(Some((directory, identity)))
        }
        Err(Errno::ENOENT) if access == ManagedCacheStoreAccess::ReadOnly => Ok(None),
        Err(Errno::ENOENT) => {
            let created = match mkdirat(parent, name, DIRECTORY_MODE) {
                Ok(()) => true,
                Err(Errno::EEXIST) => false,
                Err(error) => return Err(map_container_error(error)),
            };
            let directory = openat(
                parent,
                name,
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
                Mode::empty(),
            )
            .map(File::from)
            .map_err(map_container_error)?;
            let kind = if created {
                fchmod(&directory, DIRECTORY_MODE).map_err(|_| unavailable())?;
                Kind::PrivateDirectory
            } else {
                Kind::ContainerDirectory
            };
            let identity = identity(&directory, kind)?;
            validate_retained(&directory, identity, kind, false)?;
            Ok(Some((directory, identity)))
        }
        Err(error) => Err(map_container_error(error)),
    }
}

pub(super) fn open_existing_container(parent: &File, name: &str) -> Result<Option<File>> {
    match openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(descriptor) => {
            let directory = File::from(descriptor);
            let object_identity = identity(&directory, Kind::ContainerDirectory)?;
            validate_retained(&directory, object_identity, Kind::ContainerDirectory, false)?;
            Ok(Some(directory))
        }
        Err(Errno::ENOENT) => Ok(None),
        Err(Errno::ELOOP | Errno::ENOTDIR) => Err(unsafe_container()),
        Err(_) => Err(unavailable()),
    }
}

pub(super) fn validate_path(
    path: &Path,
    retained: &File,
    expected: Identity,
    kind: Kind,
) -> Result<()> {
    let flags = match kind {
        Kind::ContainerDirectory | Kind::PrivateDirectory => {
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
        }
        Kind::PrivateFile => {
            OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
        }
    };
    let opened = open(path, flags, Mode::empty())
        .map(File::from)
        .map_err(|_| unsafe_for(kind))?;
    validate_retained(retained, expected, kind, matches!(kind, Kind::PrivateFile))?;
    validate_retained(&opened, expected, kind, matches!(kind, Kind::PrivateFile))
}

pub(super) fn open_existing_private_directory(
    parent: &File,
    _parent_path: &Path,
    name: &str,
) -> Result<Option<File>> {
    match openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(descriptor) => {
            let directory = File::from(descriptor);
            let object_identity = identity(&directory, Kind::PrivateDirectory)?;
            validate_retained(&directory, object_identity, Kind::PrivateDirectory, false)?;
            Ok(Some(directory))
        }
        Err(Errno::ENOENT) => Ok(None),
        Err(Errno::ELOOP | Errno::ENOTDIR) => Err(unsafe_store()),
        Err(_) => Err(unavailable()),
    }
}

pub(super) fn create_private_directory_exclusive(
    parent: &File,
    _parent_path: &Path,
    name: &str,
) -> Result<Option<File>> {
    match mkdirat(parent, name, DIRECTORY_MODE) {
        Ok(()) => {}
        Err(Errno::EEXIST) => return Ok(None),
        Err(_) => return Err(unavailable()),
    }
    let directory = openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| unavailable())?;
    fchmod(&directory, DIRECTORY_MODE).map_err(|_| unavailable())?;
    let object_identity = identity(&directory, Kind::PrivateDirectory)?;
    validate_retained(&directory, object_identity, Kind::PrivateDirectory, false)?;
    Ok(Some(directory))
}

pub(super) fn create_private_file_exclusive(
    directory: &File,
    _directory_path: &Path,
    name: &str,
) -> Result<Option<(File, Identity)>> {
    match openat(
        directory,
        name,
        OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        FILE_MODE,
    ) {
        Ok(descriptor) => {
            let file = File::from(descriptor);
            fchmod(&file, FILE_MODE).map_err(|_| unavailable())?;
            let object_identity = identity(&file, Kind::PrivateFile)?;
            validate_retained(&file, object_identity, Kind::PrivateFile, true)?;
            Ok(Some((file, object_identity)))
        }
        Err(Errno::EEXIST) => Ok(None),
        Err(Errno::ELOOP | Errno::EMLINK | Errno::EISDIR | Errno::ENOTDIR) => Err(unsafe_object()),
        Err(_) => Err(unavailable()),
    }
}

pub(super) fn open_named_private_file(
    directory: &File,
    _directory_path: &Path,
    name: &str,
    writable: bool,
) -> Result<Option<(File, Identity)>> {
    let access = if writable {
        OFlag::O_RDWR
    } else {
        OFlag::O_RDONLY
    };
    match openat(
        directory,
        name,
        access | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(descriptor) => {
            let file = File::from(descriptor);
            let object_identity = identity(&file, Kind::PrivateFile)?;
            validate_retained(&file, object_identity, Kind::PrivateFile, true)?;
            Ok(Some((file, object_identity)))
        }
        Err(Errno::ENOENT) => Ok(None),
        Err(Errno::ELOOP | Errno::EMLINK | Errno::EISDIR | Errno::ENOTDIR) => Err(unsafe_object()),
        Err(_) => Err(unavailable()),
    }
}

pub(super) fn identity(file: &File, kind: Kind) -> Result<Identity> {
    let status = fstat(file).map_err(|_| unavailable())?;
    let expected_type = match kind {
        Kind::ContainerDirectory | Kind::PrivateDirectory => SFlag::S_IFDIR,
        Kind::PrivateFile => SFlag::S_IFREG,
    };
    if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != expected_type {
        return Err(unsafe_for(kind));
    }
    Ok(Identity {
        device: status.st_dev as u64,
        inode: status.st_ino as u64,
    })
}

pub(super) const fn same_filesystem(left: Identity, right: Identity) -> bool {
    left.device == right.device
}

#[cfg(test)]
pub(super) const fn different_filesystem_identity(identity: Identity) -> Identity {
    Identity {
        device: identity.device.wrapping_add(1),
        inode: identity.inode,
    }
}

pub(super) fn change_token(file: &File) -> Result<ChangeToken> {
    let status = fstat(file).map_err(|_| unavailable())?;
    Ok(ChangeToken {
        change_seconds: status.st_ctime,
        change_nanoseconds: status.st_ctime_nsec,
    })
}

pub(super) fn file_usage(file: &File) -> Result<(u64, u64)> {
    let status = fstat(file).map_err(|_| unavailable())?;
    let logical_bytes = u64::try_from(status.st_size).map_err(|_| unsafe_object())?;
    let blocks = u64::try_from(status.st_blocks).map_err(|_| unsafe_object())?;
    let allocated_bytes = blocks.checked_mul(512).ok_or_else(unsafe_object)?;
    Ok((logical_bytes, allocated_bytes))
}

pub(super) fn validate_retained(
    file: &File,
    expected: Identity,
    kind: Kind,
    require_one_link: bool,
) -> Result<()> {
    let status = fstat(file).map_err(|_| unavailable())?;
    let actual = identity(file, kind)?;
    let mode = status.st_mode & 0o7777;
    let mode_ok = match kind {
        Kind::ContainerDirectory => mode & 0o022 == 0,
        Kind::PrivateDirectory => mode == 0o700,
        Kind::PrivateFile => mode == 0o600,
    };
    if actual != expected
        || status.st_uid != geteuid().as_raw()
        || !mode_ok
        || (require_one_link && status.st_nlink != 1)
    {
        return Err(unsafe_for(kind));
    }
    match kind {
        Kind::ContainerDirectory => reject_granting_acl(file)?,
        Kind::PrivateDirectory | Kind::PrivateFile => reject_extended_acl(file, kind)?,
    }
    Ok(())
}

pub(super) fn validate_named(
    directory: &File,
    name: &str,
    retained: &File,
    expected: Identity,
    kind: Kind,
) -> Result<()> {
    let flags = match kind {
        Kind::ContainerDirectory | Kind::PrivateDirectory => {
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
        }
        Kind::PrivateFile => {
            OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
        }
    };
    let opened = openat(directory, name, flags, Mode::empty())
        .map(File::from)
        .map_err(|_| unsafe_for(kind))?;
    validate_retained(retained, expected, kind, matches!(kind, Kind::PrivateFile))?;
    validate_retained(&opened, expected, kind, matches!(kind, Kind::PrivateFile))
}

pub(super) fn inventory(
    directory: &File,
    maximum_entries: usize,
    maximum_name_bytes: usize,
    deadline: Instant,
) -> Result<Vec<String>> {
    let clone = directory.try_clone().map_err(|_| unavailable())?;
    let owned: OwnedFd = clone.into();
    let mut entries = Dir::from_fd(owned).map_err(|_| unavailable())?;
    let mut names = Vec::new();
    let mut name_bytes = 0_usize;
    for entry in entries.iter() {
        if Instant::now() > deadline || names.len() >= maximum_entries {
            return Err(budget());
        }
        let entry = entry.map_err(|_| unavailable())?;
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let name = std::str::from_utf8(bytes).map_err(|_| unsafe_object())?;
        name_bytes = name_bytes.checked_add(name.len()).ok_or_else(budget)?;
        if name_bytes > maximum_name_bytes {
            return Err(budget());
        }
        names.push(name.to_owned());
    }
    Ok(names)
}

pub(super) fn exact_name_exists(
    directory: &File,
    expected_name: &str,
    deadline: Instant,
) -> Result<bool> {
    let clone = directory.try_clone().map_err(|_| unavailable())?;
    let owned: OwnedFd = clone.into();
    let mut entries = Dir::from_fd(owned).map_err(|_| unavailable())?;
    for entry in entries.iter() {
        if Instant::now() >= deadline {
            return Err(super::busy());
        }
        let entry = entry.map_err(|_| unavailable())?;
        if entry.file_name().to_bytes() == expected_name.as_bytes() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn sync_directory(directory: &File) -> Result<()> {
    directory.sync_all().map_err(|_| unavailable())
}

pub(super) fn publish_directory_no_replace(
    parent: &File,
    source: &str,
    source_directory: &File,
    source_identity: Identity,
    destination: &str,
) -> Result<Publication> {
    validate_retained(
        source_directory,
        source_identity,
        Kind::PrivateDirectory,
        false,
    )?;
    match rename_no_replace(parent, source, destination) {
        Ok(()) => Ok(Publication::Published),
        Err(Errno::EEXIST | Errno::ENOTEMPTY) => Ok(Publication::Collision),
        Err(_) => Err(unavailable()),
    }
}

pub(super) fn detach_directory_no_replace(
    parent: &File,
    source: &str,
    source_directory: &File,
    source_identity: Identity,
    destination: &str,
) -> Result<()> {
    validate_named(
        parent,
        source,
        source_directory,
        source_identity,
        Kind::PrivateDirectory,
    )?;
    // A reset caller has already committed durable intent. Every syscall
    // attempt failure is therefore recovery-required at the engine layer;
    // no collision or not-found result is exposed as retryable.
    rename_no_replace(parent, source, destination).map_err(|_| unavailable())
}

pub(super) fn publish_file_replace(
    directory: &File,
    source: &str,
    source_file: &File,
    source_identity: Identity,
    destination: &str,
) -> Result<Publication> {
    validate_named(
        directory,
        source,
        source_file,
        source_identity,
        Kind::PrivateFile,
    )?;
    let source = CString::new(source).map_err(|_| unsafe_object())?;
    let destination = CString::new(destination).map_err(|_| unsafe_object())?;
    // SAFETY: both values are validated single components beneath the same
    // retained private store. renameat atomically installs the complete
    // create-new temp and never follows a displaced destination symlink.
    let result = unsafe {
        // DUX-DESTRUCTIVE: allow=managed-cache-entry-publish -- atomically replace only one codec-keyed final with its retained, fully validated create-new temporary
        nix::libc::renameat(
            directory.as_raw_fd(),
            source.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
        )
    };
    Errno::result(result)
        .map(|_| Publication::Published)
        .map_err(|_| unavailable())
}

pub(super) fn remove_retained_file(
    directory: &File,
    name: &str,
    file: File,
    expected: Identity,
) -> Result<()> {
    use nix::unistd::{UnlinkatFlags, unlinkat};
    validate_named(directory, name, &file, expected, Kind::PrivateFile)?;
    // DUX-DESTRUCTIVE: allow=managed-cache-exact-clear -- unlink only one exact retained private single-link object from a managed inventory, pending-temp guard, or create-new stage guard
    unlinkat(directory, name, UnlinkatFlags::NoRemoveDir).map_err(|_| unavailable())?;
    drop(file);
    Ok(())
}

pub(super) fn remove_app_data_reset_stage_control(
    directory: &File,
    name: &str,
    file: File,
    expected: Identity,
) -> Result<()> {
    use nix::unistd::{UnlinkatFlags, unlinkat};
    validate_named(directory, name, &file, expected, Kind::PrivateFile)?;
    // DUX-DESTRUCTIVE: allow=managed-cache-reset-stage-control-unlink -- unlink only one exact retained reset-stage control selected by the durable-Draining structural typestate
    unlinkat(directory, name, UnlinkatFlags::NoRemoveDir).map_err(|_| unavailable())?;
    drop(file);
    Ok(())
}

pub(super) fn remove_retained_directory(
    parent: &File,
    name: &str,
    directory: File,
    expected: Identity,
) -> Result<()> {
    use nix::unistd::{UnlinkatFlags, unlinkat};
    validate_named(parent, name, &directory, expected, Kind::PrivateDirectory)?;
    // DUX-DESTRUCTIVE: allow=managed-cache-exact-stage-remove -- remove only the exact retained create-new provisioning stage after all retained controls were removed and no unproven child remains
    unlinkat(parent, name, UnlinkatFlags::RemoveDir).map_err(|_| unavailable())?;
    drop(directory);
    Ok(())
}

pub(super) fn remove_app_data_reset_retired_stage_directory(
    parent: &File,
    name: &str,
    directory: File,
    expected: Identity,
) -> Result<()> {
    use nix::unistd::{UnlinkatFlags, unlinkat};
    validate_named(parent, name, &directory, expected, Kind::PrivateDirectory)?;
    // DUX-DESTRUCTIVE: allow=managed-cache-reset-stage-rmdir -- remove only the empty exact journal-identity-bound detached cache stage after durable-Draining structural admission
    unlinkat(parent, name, UnlinkatFlags::RemoveDir).map_err(|_| unavailable())?;
    drop(directory);
    Ok(())
}

fn validate_publication_parent(parent: &File) -> Result<()> {
    let status = fstat(parent).map_err(|_| unsafe_container())?;
    if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != SFlag::S_IFDIR
        || status.st_uid != geteuid().as_raw()
        || status.st_mode & 0o022 != 0
    {
        return Err(unsafe_container());
    }
    reject_granting_acl(parent)
}

#[cfg(target_os = "linux")]
fn rename_no_replace(
    parent: &File,
    source: &str,
    destination: &str,
) -> std::result::Result<(), Errno> {
    let source = CString::new(source).map_err(|_| Errno::EINVAL)?;
    let destination = CString::new(destination).map_err(|_| Errno::EINVAL)?;
    // SAFETY: fixed/generated single components are published beneath one
    // retained current-user container. RENAME_NOREPLACE forbids adoption.
    let result = unsafe {
        nix::libc::syscall(
            // DUX-DESTRUCTIVE: allow=managed-cache-linux-store-publish -- atomically publish only the retained marker-complete private managed-cache child without replacing an existing entry
            nix::libc::SYS_renameat2,
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            destination.as_ptr(),
            nix::libc::RENAME_NOREPLACE,
        )
    };
    Errno::result(result).map(drop)
}

#[cfg(target_os = "macos")]
fn rename_no_replace(
    parent: &File,
    source: &str,
    destination: &str,
) -> std::result::Result<(), Errno> {
    use std::ffi::{c_char, c_int, c_uint};
    const RENAME_EXCL: c_uint = 0x0000_0004;
    const RENAME_NOFOLLOW_ANY: c_uint = 0x0000_0010;
    unsafe extern "C" {
        fn renameatx_np(
            from_fd: c_int,
            from: *const c_char,
            to_fd: c_int,
            to: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }
    let source = CString::new(source).map_err(|_| Errno::EINVAL)?;
    let destination = CString::new(destination).map_err(|_| Errno::EINVAL)?;
    // SAFETY: generated names are single components beneath the retained
    // container; EXCL forbids replacement and NOFOLLOW_ANY rejects aliases.
    let result = unsafe {
        // DUX-DESTRUCTIVE: allow=managed-cache-macos-store-publish -- atomically publish only the retained marker-complete private managed-cache child without replacing an existing entry
        renameatx_np(
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            destination.as_ptr(),
            RENAME_EXCL | RENAME_NOFOLLOW_ANY,
        )
    };
    Errno::result(result).map(drop)
}

#[cfg(target_os = "macos")]
fn reject_extended_acl(file: &File, kind: Kind) -> Result<()> {
    use std::ffi::{c_int, c_void};
    const ACL_TYPE_EXTENDED: c_int = 0x100;
    const ACL_FIRST_ENTRY: c_int = 0;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: c_int, acl_type: c_int) -> *mut c_void;
        fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
        fn acl_free(object: *mut c_void) -> c_int;
    }
    // SAFETY: Darwin returns an independently allocated ACL for this live fd.
    let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        return if Errno::last() == Errno::ENOENT {
            Ok(())
        } else {
            Err(unsafe_for(kind))
        };
    }
    let mut entry = std::ptr::null_mut();
    // SAFETY: the ACL is live and the output pointer is writable.
    let status = unsafe { acl_get_entry(acl, ACL_FIRST_ENTRY, &raw mut entry) };
    // SAFETY: this frees the independently allocated ACL exactly once.
    let freed = unsafe { acl_free(acl) };
    if status < 0 || freed != 0 || !entry.is_null() {
        return Err(unsafe_for(kind));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn reject_extended_acl(file: &File, kind: Kind) -> Result<()> {
    reject_linux_acl(file).map_err(|_| unsafe_for(kind))
}

#[cfg(target_os = "macos")]
fn reject_granting_acl(file: &File) -> Result<()> {
    use std::ffi::{c_int, c_void};
    const ACL_TYPE_EXTENDED: c_int = 0x100;
    const ACL_FIRST_ENTRY: c_int = 0;
    const ACL_NEXT_ENTRY: c_int = -1;
    const ACL_EXTENDED_DENY: c_int = 2;
    const ACL_MAX_ENTRIES: usize = 128;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: c_int, acl_type: c_int) -> *mut c_void;
        fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
        fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_int) -> c_int;
        fn acl_free(object: *mut c_void) -> c_int;
    }
    // SAFETY: Darwin returns an independently allocated ACL for this live fd.
    let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        return if Errno::last() == Errno::ENOENT {
            Ok(())
        } else {
            Err(unsafe_container())
        };
    }
    let inspection = (|| {
        let mut selector = ACL_FIRST_ENTRY;
        for _ in 0..ACL_MAX_ENTRIES {
            let mut entry = std::ptr::null_mut();
            // SAFETY: the ACL is live and the output pointer is writable.
            if unsafe { acl_get_entry(acl, selector, &raw mut entry) } < 0 {
                if selector == ACL_NEXT_ENTRY && Errno::last() == Errno::EINVAL {
                    return Ok(());
                }
                return Err(unsafe_container());
            }
            if entry.is_null() {
                return Ok(());
            }
            let mut tag = 0;
            // SAFETY: entry belongs to the live ACL and tag is writable.
            if unsafe { acl_get_tag_type(entry, &raw mut tag) } != 0 || tag != ACL_EXTENDED_DENY {
                return Err(unsafe_container());
            }
            selector = ACL_NEXT_ENTRY;
        }
        Err(unsafe_container())
    })();
    // SAFETY: this frees the independently allocated ACL exactly once.
    if unsafe { acl_free(acl) } != 0 {
        return Err(unsafe_container());
    }
    inspection
}

#[cfg(target_os = "linux")]
fn reject_granting_acl(file: &File) -> Result<()> {
    reject_linux_acl(file).map_err(|_| unsafe_container())
}

#[cfg(target_os = "linux")]
fn reject_linux_acl(file: &File) -> std::result::Result<(), ()> {
    for name in [
        b"system.posix_acl_access\0".as_slice(),
        b"system.posix_acl_default\0".as_slice(),
    ] {
        // SAFETY: `name` is NUL terminated and the live descriptor is only queried.
        let length = unsafe {
            nix::libc::fgetxattr(
                file.as_raw_fd(),
                name.as_ptr().cast(),
                std::ptr::null_mut(),
                0,
            )
        };
        if length >= 0 {
            return Err(());
        }
        match Errno::last() {
            Errno::ENODATA | Errno::ENOTSUP => {}
            _ => return Err(()),
        }
    }
    Ok(())
}

fn map_container_error(error: Errno) -> ManagedCacheStoreError {
    match error {
        Errno::ELOOP | Errno::ENOTDIR | Errno::EISDIR => unsafe_container(),
        _ => unavailable(),
    }
}

fn unsafe_for(kind: Kind) -> ManagedCacheStoreError {
    match kind {
        Kind::ContainerDirectory => unsafe_container(),
        Kind::PrivateDirectory => unsafe_store(),
        Kind::PrivateFile => unsafe_object(),
    }
}

fn unsafe_container() -> ManagedCacheStoreError {
    ManagedCacheStoreError::new(ManagedCacheStoreErrorKind::UnsafeContainer)
}
