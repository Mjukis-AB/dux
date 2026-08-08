use std::ffi::{CString, OsStr};
use std::fs::File;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::{AtFlags, OFlag, open, openat};
use nix::sys::stat::{FchmodatFlags, Mode, SFlag, fchmod, fchmodat, fstat, fstatat, mkdirat};
use nix::unistd::geteuid;

use super::{Result, SnapshotStorageError, SnapshotStorageErrorKind};

const DIRECTORY_MODE: Mode = Mode::S_IRWXU;
const FILE_MODE: Mode = Mode::S_IRUSR.union(Mode::S_IWUSR);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
}

pub(super) const fn same_filesystem(left: Identity, right: Identity) -> bool {
    left.device == right.device
}

#[cfg(test)]
pub(super) const fn different_filesystem_identity_for_test(identity: Identity) -> Identity {
    Identity {
        device: identity.device.wrapping_add(1),
        inode: identity.inode,
    }
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
    let file = open(
        path,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(map_root_error)?;
    validate_retained(
        &file,
        identity(&file, Kind::Directory)?,
        Kind::Directory,
        false,
    )?;
    Ok(file)
}

pub(super) fn open_existing_directory(
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
            let object_identity = identity(&directory, Kind::Directory)?;
            validate_retained(&directory, object_identity, Kind::Directory, false)?;
            Ok(Some(directory))
        }
        Err(Errno::ENOENT) => Ok(None),
        Err(error) => Err(map_root_error(error)),
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
        Err(error) => return Err(map_root_error(error)),
    }
    fchmodat(parent, name, DIRECTORY_MODE, FchmodatFlags::NoFollowSymlink)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    let directory = openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(map_root_error)?;
    fchmod(&directory, DIRECTORY_MODE)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    let object_identity = identity(&directory, Kind::Directory)?;
    validate_retained(&directory, object_identity, Kind::Directory, false)?;
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
            fchmod(&file, FILE_MODE)
                .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
            let object_identity = identity(&file, Kind::RegularFile)?;
            validate_retained(&file, object_identity, Kind::RegularFile, true)?;
            Ok(Some((file, object_identity)))
        }
        Err(Errno::EEXIST) => Ok(None),
        Err(Errno::ELOOP | Errno::EMLINK | Errno::EISDIR | Errno::ENOTDIR) => Err(
            SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject),
        ),
        Err(_) => Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::Unavailable,
        )),
    }
}

pub(super) fn open_named_regular(
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
            let object_identity = identity(&file, Kind::RegularFile)?;
            validate_retained(&file, object_identity, Kind::RegularFile, true)?;
            Ok(Some((file, object_identity)))
        }
        Err(Errno::ENOENT) => Ok(None),
        Err(Errno::ELOOP | Errno::EMLINK | Errno::EISDIR | Errno::ENOTDIR) => Err(
            SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject),
        ),
        Err(_) => Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::Unavailable,
        )),
    }
}

pub(super) fn open_named_temp_for_removal(
    directory: &File,
    directory_path: &Path,
    name: &str,
) -> Result<Option<(File, Identity)>> {
    open_named_regular(directory, directory_path, name, true)
}

pub(super) fn open_named_final_for_removal(
    directory: &File,
    directory_path: &Path,
    name: &str,
) -> Result<Option<(File, Identity)>> {
    open_named_regular(directory, directory_path, name, false)
}

pub(super) fn identity(file: &File, kind: Kind) -> Result<Identity> {
    let status = fstat(file).map_err(|_| unavailable_for(kind))?;
    let expected_type = match kind {
        Kind::Directory => SFlag::S_IFDIR,
        Kind::RegularFile => SFlag::S_IFREG,
    };
    if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != expected_type {
        return Err(unsafe_for(kind));
    }
    Ok(Identity {
        device: status.st_dev as u64,
        inode: status.st_ino as u64,
    })
}

pub(super) fn file_usage(file: &File) -> Result<(u64, u64)> {
    let status = fstat(file).map_err(|_| unavailable_for(Kind::RegularFile))?;
    let logical_bytes = u64::try_from(status.st_size).map_err(|_| unsafe_for(Kind::RegularFile))?;
    let blocks = u64::try_from(status.st_blocks).map_err(|_| unsafe_for(Kind::RegularFile))?;
    let allocated_bytes = blocks
        .checked_mul(512)
        .ok_or_else(|| unsafe_for(Kind::RegularFile))?;
    Ok((logical_bytes, allocated_bytes))
}

pub(super) fn validate_retained(
    file: &File,
    expected: Identity,
    kind: Kind,
    require_one_link: bool,
) -> Result<()> {
    let status = fstat(file).map_err(|_| unavailable_for(kind))?;
    let actual = identity(file, kind)?;
    let mode = status.st_mode & 0o7777;
    let expected_mode = match kind {
        Kind::Directory => 0o700,
        Kind::RegularFile => 0o600,
    };
    if actual != expected
        || status.st_uid != geteuid().as_raw()
        || mode != expected_mode
        || (require_one_link && status.st_nlink != 1)
    {
        return Err(unsafe_for(kind));
    }
    reject_extended_acl(file, kind)?;
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
        Kind::Directory => {
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
        }
        Kind::RegularFile => {
            OFlag::O_RDONLY | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW
        }
    };
    let opened = openat(directory, name, flags, Mode::empty())
        .map(File::from)
        .map_err(|_| unsafe_for(kind))?;
    validate_retained(retained, expected, kind, matches!(kind, Kind::RegularFile))?;
    validate_retained(&opened, expected, kind, matches!(kind, Kind::RegularFile))
}

pub(super) fn inventory(
    directory: &File,
    maximum_entries: usize,
    maximum_name_bytes: usize,
    deadline: std::time::Instant,
) -> Result<Vec<String>> {
    let clone = directory
        .try_clone()
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    let owned: OwnedFd = clone.into();
    let mut entries = Dir::from_fd(owned)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    let mut names = Vec::new();
    let mut name_bytes = 0_usize;
    for entry in entries.iter() {
        if std::time::Instant::now() > deadline || names.len() >= maximum_entries {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            ));
        }
        let entry =
            entry.map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let name = std::str::from_utf8(bytes)
            .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject))?;
        name_bytes = name_bytes
            .checked_add(name.len())
            .ok_or_else(|| SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeObject))?;
        if name_bytes > maximum_name_bytes {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::UnsafeObject,
            ));
        }
        names.push(name.to_owned());
    }
    Ok(names)
}

/// Prove the raw directory-entry spelling rather than accepting a
/// case-folded lookup of the same retained inode.
pub(super) fn exact_name_exists(
    directory: &File,
    expected_name: &str,
    deadline: std::time::Instant,
) -> Result<bool> {
    let clone = directory
        .try_clone()
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    let owned: OwnedFd = clone.into();
    let mut entries = Dir::from_fd(owned)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    for entry in entries.iter() {
        if std::time::Instant::now() >= deadline {
            return Err(SnapshotStorageError::new(SnapshotStorageErrorKind::Busy));
        }
        let entry =
            entry.map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        if entry.file_name().to_bytes() == expected_name.as_bytes() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn inventory_provisioning_stage_names(
    directory: &File,
    maximum_entries: usize,
    maximum_name_bytes: usize,
    maximum_stages: usize,
    deadline: std::time::Instant,
) -> Result<Vec<String>> {
    let clone = directory
        .try_clone()
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    let owned: OwnedFd = clone.into();
    let mut entries = Dir::from_fd(owned)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    let mut names = Vec::new();
    let mut entry_count = 0_usize;
    let mut name_bytes = 0_usize;
    for entry in entries.iter() {
        if std::time::Instant::now() > deadline || entry_count >= maximum_entries {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            ));
        }
        let entry =
            entry.map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
        let bytes = entry.file_name().to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        entry_count = entry_count
            .checked_add(1)
            .ok_or_else(super::unsafe_inventory_object)?;
        name_bytes = name_bytes
            .checked_add(bytes.len())
            .ok_or_else(super::unsafe_inventory_object)?;
        if name_bytes > maximum_name_bytes {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::UnsafeObject,
            ));
        }
        if super::is_provisioning_stage_name_bytes(bytes) {
            if names.len() >= maximum_stages {
                return Err(super::unsafe_inventory_object());
            }
            // The accepted grammar is ASCII, independent of every other
            // root entry's byte encoding.
            names.push(
                String::from_utf8(bytes.to_vec()).map_err(|_| {
                    SnapshotStorageError::new(SnapshotStorageErrorKind::InternalState)
                })?,
            );
        }
    }
    Ok(names)
}

pub(super) fn open_provisioning_stage_for_removal(
    parent: &File,
    _parent_path: &Path,
    name: &str,
) -> Result<Option<super::ProvisioningStageOpen>> {
    let status = match fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(status) => status,
        Err(Errno::ENOENT) => return Ok(None),
        Err(_) => return Err(super::unsafe_inventory_object()),
    };
    let mode = status.st_mode & 0o7777;
    if SFlag::from_bits_truncate(status.st_mode) & SFlag::S_IFMT != SFlag::S_IFDIR
        || status.st_uid != geteuid().as_raw()
        || mode & !0o700 != 0
    {
        return Err(super::unsafe_inventory_object());
    }
    // mkdirat(0700) is filtered through umask before the following chmod.
    // A crash in that interval can leave an opaque owner-owned directory,
    // including mode 000. It is observation-only and grants no mutation.
    if mode != 0o700 {
        return Ok(Some(super::ProvisioningStageOpen::Deferred));
    }
    let descriptor = match openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(Errno::ENOENT) => return Ok(None),
        Err(Errno::ELOOP | Errno::ENOTDIR) => return Err(super::unsafe_inventory_object()),
        Err(_) => {
            return Err(SnapshotStorageError::new(
                SnapshotStorageErrorKind::Unavailable,
            ));
        }
    };
    let directory = File::from(descriptor);
    let object_identity = identity(&directory, Kind::Directory)?;
    validate_named(parent, name, &directory, object_identity, Kind::Directory)?;
    validate_retained(&directory, object_identity, Kind::Directory, false)?;
    Ok(Some(super::ProvisioningStageOpen::Private(
        directory,
        object_identity,
    )))
}

pub(super) fn open_stage_control_for_removal(
    directory: &File,
    directory_path: &Path,
    name: &str,
) -> Result<Option<(File, Identity)>> {
    open_named_regular(directory, directory_path, name, true)
}

pub(super) fn sync_directory(directory: &File) -> Result<()> {
    directory
        .sync_all()
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))
}

pub(super) fn publish_no_replace(
    directory: &File,
    source: &str,
    _source_file: &File,
    _source_identity: Identity,
    destination: &str,
) -> Result<Publication> {
    publish_between(
        directory,
        source,
        directory,
        destination,
        PublicationKind::SnapshotFile,
    )
}

pub(super) fn publish_directory_no_replace(
    source_parent: &File,
    source: &str,
    _source_directory: &File,
    _source_identity: Identity,
    destination_parent: &File,
    destination: &str,
) -> Result<Publication> {
    publish_between(
        source_parent,
        source,
        destination_parent,
        destination,
        PublicationKind::StoreDirectory,
    )
}

#[derive(Clone, Copy)]
enum PublicationKind {
    SnapshotFile,
    StoreDirectory,
}

fn publish_between(
    source_parent: &File,
    source: &str,
    destination_parent: &File,
    destination: &str,
    publication_kind: PublicationKind,
) -> Result<Publication> {
    match rename_no_replace(
        source_parent,
        OsStr::new(source),
        destination_parent,
        OsStr::new(destination),
        publication_kind,
    ) {
        Ok(()) => Ok(Publication::Published),
        Err(Errno::EEXIST | Errno::ENOTEMPTY) => Ok(Publication::Collision),
        Err(_) => Err(SnapshotStorageError::new(
            SnapshotStorageErrorKind::Unavailable,
        )),
    }
}

#[cfg(target_os = "linux")]
fn rename_no_replace(
    source_parent: &File,
    source: &OsStr,
    destination_parent: &File,
    destination: &OsStr,
    _publication_kind: PublicationKind,
) -> std::result::Result<(), Errno> {
    let source = CString::new(source.as_bytes()).map_err(|_| Errno::EINVAL)?;
    let destination = CString::new(destination.as_bytes()).map_err(|_| Errno::EINVAL)?;
    // SAFETY: both names are validated single components beneath retained
    // directories on the same filesystem. RENAME_NOREPLACE forbids overwrite.
    let result = unsafe {
        nix::libc::syscall(
            // DUX-DESTRUCTIVE: allow=snapshot-linux-no-replace-publish -- atomically publish only a retained marker-complete store stage or current create-new snapshot temp without replacing an entry
            nix::libc::SYS_renameat2,
            source_parent.as_raw_fd(),
            source.as_ptr(),
            destination_parent.as_raw_fd(),
            destination.as_ptr(),
            nix::libc::RENAME_NOREPLACE,
        )
    };
    Errno::result(result).map(drop)
}

#[cfg(target_os = "macos")]
fn rename_no_replace(
    source_parent: &File,
    source: &OsStr,
    destination_parent: &File,
    destination: &OsStr,
    _publication_kind: PublicationKind,
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
    let source = CString::new(source.as_bytes()).map_err(|_| Errno::EINVAL)?;
    let destination = CString::new(destination.as_bytes()).map_err(|_| Errno::EINVAL)?;
    // SAFETY: both names are validated single components beneath retained
    // directories. EXCL forbids replacement and NOFOLLOW_ANY rejects
    // symlink traversal anywhere in publication.
    let result = unsafe {
        // DUX-DESTRUCTIVE: allow=snapshot-macos-no-replace-publish -- atomically publish only a retained marker-complete store stage or current create-new snapshot temp without replacing an entry
        renameatx_np(
            source_parent.as_raw_fd(),
            source.as_ptr(),
            destination_parent.as_raw_fd(),
            destination.as_ptr(),
            RENAME_EXCL | RENAME_NOFOLLOW_ANY,
        )
    };
    Errno::result(result).map(drop)
}

pub(super) fn remove_retained_temp(
    directory: &File,
    name: &str,
    file: File,
    expected: Identity,
) -> Result<()> {
    remove_retained_temp_with_before_unlink(directory, name, file, expected, || Ok(()))
}

pub(super) fn remove_retained_temp_with_before_unlink(
    directory: &File,
    name: &str,
    file: File,
    expected: Identity,
    before_unlink: impl FnOnce() -> Result<()>,
) -> Result<()> {
    use nix::unistd::{UnlinkatFlags, unlinkat};
    validate_named(directory, name, &file, expected, Kind::RegularFile)?;
    before_unlink()?;
    // DUX-DESTRUCTIVE: allow=snapshot-current-temp-unlink -- remove only a retained current-call, exact row-bound/unleased quiescent temp, or one same-filesystem reset temp selected by the coordinator-only journal/root/cache-absence capability after complete inventory and identity revalidation
    unlinkat(directory, name, UnlinkatFlags::NoRemoveDir)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    drop(file);
    Ok(())
}

#[cfg(test)]
pub(super) fn remove_retained_final(
    directory: &File,
    name: &str,
    file: File,
    expected: Identity,
) -> Result<()> {
    remove_retained_final_with_before_unlink(directory, name, file, expected, || Ok(()))
}

pub(super) fn remove_retained_final_with_before_unlink(
    directory: &File,
    name: &str,
    file: File,
    expected: Identity,
    before_unlink: impl FnOnce() -> Result<()>,
) -> Result<()> {
    use nix::unistd::{UnlinkatFlags, unlinkat};
    validate_named(directory, name, &file, expected, Kind::RegularFile)?;
    before_unlink()?;
    // DUX-DESTRUCTIVE: allow=snapshot-observed-final-unlink -- remove only an exact typed final observed under the retained inventory writer lease after higher-layer tombstone/orphan authority or the same-filesystem coordinator-only reset capability plus identity and usage revalidation
    unlinkat(directory, name, UnlinkatFlags::NoRemoveDir)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    drop(file);
    Ok(())
}

pub(super) fn remove_app_data_reset_snapshot_control_with_before_unlink(
    directory: &File,
    name: &str,
    file: File,
    expected: Identity,
    before_unlink: impl FnOnce() -> Result<()>,
) -> Result<()> {
    use nix::unistd::{UnlinkatFlags, unlinkat};
    validate_named(directory, name, &file, expected, Kind::RegularFile)?;
    before_unlink()?;
    // DUX-DESTRUCTIVE: allow=app-data-reset-snapshot-control-unlink -- remove exactly one retained snapshot marker or exclusively locked writer control from the journal-bound same-filesystem detached old store after the coordinator-only structural capability repeats the complete monotonic tail at the final effect gate
    unlinkat(directory, name, UnlinkatFlags::NoRemoveDir)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    drop(file);
    Ok(())
}

pub(super) fn remove_app_data_reset_snapshot_directory_with_before_unlink(
    database_root: &File,
    name: &str,
    directory: File,
    expected: Identity,
    before_unlink: impl FnOnce() -> Result<()>,
) -> Result<()> {
    use nix::unistd::{UnlinkatFlags, unlinkat};
    validate_named(database_root, name, &directory, expected, Kind::Directory)?;
    before_unlink()?;
    // DUX-DESTRUCTIVE: allow=app-data-reset-snapshot-directory-unlink -- remove only the exact empty snapshots directory from the journal-bound same-filesystem detached old store after both controls are durably absent and the coordinator-only structural capability repeats that complete tail at the final effect gate
    unlinkat(database_root, name, UnlinkatFlags::RemoveDir)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    drop(directory);
    Ok(())
}

pub(super) fn remove_retained_stage_control(
    directory: &File,
    name: &str,
    file: File,
    expected: Identity,
) -> Result<()> {
    use nix::unistd::{UnlinkatFlags, unlinkat};
    validate_named(directory, name, &file, expected, Kind::RegularFile)?;
    // DUX-DESTRUCTIVE: allow=snapshot-provisioning-stage-control-unlink -- unlink only an exact marker byte string retained inside a completely inventoried canonical private provisioning stage under the retained database root
    unlinkat(directory, name, UnlinkatFlags::NoRemoveDir)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    drop(file);
    Ok(())
}

pub(super) fn remove_retained_provisioning_stage(
    parent: &File,
    name: &str,
    directory: File,
    expected: Identity,
) -> Result<()> {
    use nix::unistd::{UnlinkatFlags, unlinkat};
    validate_named(parent, name, &directory, expected, Kind::Directory)?;
    // DUX-DESTRUCTIVE: allow=snapshot-provisioning-stage-rmdir -- remove only the exact retained now-empty canonical private provisioning directory after its marker-owned controls were individually unlinked
    unlinkat(parent, name, UnlinkatFlags::RemoveDir)
        .map_err(|_| SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable))?;
    drop(directory);
    Ok(())
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
    // SAFETY: the retained descriptor is live and Darwin allocates the ACL.
    let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
    if acl.is_null() {
        if Errno::last() == Errno::ENOENT {
            return Ok(());
        }
        return Err(unsafe_for(kind));
    }
    let mut entry = std::ptr::null_mut();
    // SAFETY: `acl` is live and the output pointer is writable.
    let status = unsafe { acl_get_entry(acl, ACL_FIRST_ENTRY, &raw mut entry) };
    // SAFETY: `acl` is freed exactly once.
    let freed = unsafe { acl_free(acl) };
    if status < 0 || freed != 0 || !entry.is_null() {
        return Err(unsafe_for(kind));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn reject_extended_acl(_file: &File, _kind: Kind) -> Result<()> {
    // Linux ACL inspection needs the optional libacl ABI. Exact 0700/0600
    // modes ensure any POSIX ACL mask grants no group/other permission.
    Ok(())
}

fn map_root_error(error: Errno) -> SnapshotStorageError {
    match error {
        Errno::ELOOP | Errno::ENOTDIR | Errno::EISDIR => {
            SnapshotStorageError::new(SnapshotStorageErrorKind::UnsafeRoot)
        }
        _ => SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable),
    }
}

fn unsafe_for(kind: Kind) -> SnapshotStorageError {
    SnapshotStorageError::new(match kind {
        Kind::Directory => SnapshotStorageErrorKind::UnsafeRoot,
        Kind::RegularFile => SnapshotStorageErrorKind::UnsafeObject,
    })
}

fn unavailable_for(_kind: Kind) -> SnapshotStorageError {
    SnapshotStorageError::new(SnapshotStorageErrorKind::Unavailable)
}
