use std::ffi::OsStr;
use std::fs::File;
use std::os::fd::OwnedFd;
use std::path::{Component, Path, PathBuf};

use nix::fcntl::{AtFlags, OFlag, open, openat};
use nix::sys::stat::{Mode, SFlag, fstat, fstatat};

use super::filesystem::{
    AncestorIdentity, CanonicalPathError, FilesystemEntryKind, FilesystemIdentity,
    PlatformEntrySnapshot, PlatformPathSnapshot, PlatformRootSnapshot, map_io_error,
};

const DIRECTORY_FLAGS: OFlag = OFlag::O_RDONLY
    .union(OFlag::O_DIRECTORY)
    .union(OFlag::O_NOFOLLOW)
    .union(OFlag::O_CLOEXEC);
const REGULAR_FILE_FLAGS: OFlag = OFlag::O_RDONLY
    .union(OFlag::O_NOFOLLOW)
    // A regular file ignores this flag. If the inspected entry is replaced by
    // a FIFO before openat, it prevents validation from blocking indefinitely
    // before the retained descriptor can be fstat-checked.
    .union(OFlag::O_NONBLOCK)
    .union(OFlag::O_CLOEXEC);

pub(super) fn capture_root(path: &Path) -> Result<PlatformRootSnapshot, CanonicalPathError> {
    let mut directory = open(Path::new("/"), DIRECTORY_FLAGS, Mode::empty())
        .map_err(|error| map_nix_error(0, error))?;
    let mut final_stat = fstat(&directory).map_err(|error| map_nix_error(0, error))?;

    let components: Vec<&OsStr> = normal_components(path)?.collect();
    for (component_index, component) in components.iter().enumerate() {
        let snapshot = inspect_at(&directory, component, component_index, false)?;
        if snapshot.kind != FilesystemEntryKind::Directory {
            return if component_index + 1 == components.len() {
                Err(CanonicalPathError::ScanRootNotDirectory)
            } else {
                Err(CanonicalPathError::NonDirectoryAncestor { component_index })
            };
        }
        let next = open_directory_at(&directory, component, component_index)?;
        final_stat = fstat(&next).map_err(|error| map_nix_error(component_index, error))?;
        if identity(&final_stat) != snapshot.identity {
            return Err(CanonicalPathError::ChangedDuringValidation { component_index });
        }
        directory = next;
    }

    if kind(&final_stat) != Some(FilesystemEntryKind::Directory) {
        return Err(CanonicalPathError::ScanRootNotDirectory);
    }

    Ok(PlatformRootSnapshot {
        identity: identity(&final_stat),
    })
}

pub(super) fn capture_descendant(
    root: &Path,
    relative_path: &Path,
) -> Result<PlatformPathSnapshot, CanonicalPathError> {
    let mut directory =
        open(root, DIRECTORY_FLAGS, Mode::empty()).map_err(|error| map_nix_error(0, error))?;
    let root_stat = fstat(&directory).map_err(|error| map_nix_error(0, error))?;
    if kind(&root_stat) != Some(FilesystemEntryKind::Directory) {
        return Err(CanonicalPathError::ScanRootNotDirectory);
    }
    let root_identity = identity(&root_stat);
    let mut ancestors = vec![AncestorIdentity::new(PathBuf::new(), root_identity)];
    let components: Vec<&OsStr> = normal_components(relative_path)?.collect();
    let mut relative = PathBuf::new();

    for (component_index, component) in components.iter().enumerate() {
        let target = component_index + 1 == components.len();
        let snapshot = inspect_at(&directory, component, component_index + 1, target)?;
        if snapshot.identity.volume() != root_identity.volume() {
            return Err(CanonicalPathError::CrossVolume {
                component_index: component_index + 1,
                expected: root_identity.volume(),
                observed: snapshot.identity.volume(),
            });
        }
        relative.push(component);

        if target {
            return Ok(PlatformPathSnapshot {
                target: snapshot,
                ancestors,
            });
        }
        if snapshot.kind != FilesystemEntryKind::Directory {
            return Err(CanonicalPathError::NonDirectoryAncestor {
                component_index: component_index + 1,
            });
        }
        let next = open_directory_at(&directory, component, component_index + 1)?;
        let opened = fstat(&next).map_err(|error| map_nix_error(component_index + 1, error))?;
        if identity(&opened) != snapshot.identity {
            return Err(CanonicalPathError::ChangedDuringValidation {
                component_index: component_index + 1,
            });
        }
        ancestors.push(AncestorIdentity::new(relative.clone(), snapshot.identity));
        directory = next;
    }

    Err(CanonicalPathError::CanonicalEscapesScanRoot)
}

pub(super) fn open_regular_descendant(
    root: &Path,
    relative_path: &Path,
) -> Result<(File, PlatformEntrySnapshot), CanonicalPathError> {
    let mut directory =
        open(root, DIRECTORY_FLAGS, Mode::empty()).map_err(|error| map_nix_error(0, error))?;
    let root_stat = fstat(&directory).map_err(|error| map_nix_error(0, error))?;
    if kind(&root_stat) != Some(FilesystemEntryKind::Directory) {
        return Err(CanonicalPathError::ScanRootNotDirectory);
    }
    let root_identity = identity(&root_stat);
    let components: Vec<&OsStr> = normal_components(relative_path)?.collect();

    for (index, component) in components.iter().enumerate() {
        let component_index = index + 1;
        let target = index + 1 == components.len();
        let snapshot = inspect_at(&directory, component, component_index, target)?;
        if snapshot.identity.volume() != root_identity.volume() {
            return Err(CanonicalPathError::CrossVolume {
                component_index,
                expected: root_identity.volume(),
                observed: snapshot.identity.volume(),
            });
        }
        if target {
            if snapshot.kind != FilesystemEntryKind::RegularFile {
                return Err(CanonicalPathError::UnsupportedTargetKind);
            }
            let opened = openat(&directory, *component, REGULAR_FILE_FLAGS, Mode::empty())
                .map_err(|error| {
                    if error == nix::errno::Errno::ELOOP {
                        CanonicalPathError::SymlinkOrReparsePoint {
                            component_index,
                            target: true,
                        }
                    } else {
                        map_nix_error(component_index, error)
                    }
                })?;
            let opened_stat =
                fstat(&opened).map_err(|error| map_nix_error(component_index, error))?;
            let opened_snapshot = PlatformEntrySnapshot {
                identity: identity(&opened_stat),
                kind: kind(&opened_stat).ok_or(CanonicalPathError::UnsupportedTargetKind)?,
                hard_link_count: hard_link_count(&opened_stat),
            };
            if opened_snapshot != snapshot {
                return Err(CanonicalPathError::ChangedDuringValidation { component_index });
            }
            return Ok((File::from(opened), opened_snapshot));
        }
        if snapshot.kind != FilesystemEntryKind::Directory {
            return Err(CanonicalPathError::NonDirectoryAncestor { component_index });
        }
        let next = open_directory_at(&directory, component, component_index)?;
        let opened = fstat(&next).map_err(|error| map_nix_error(component_index, error))?;
        if identity(&opened) != snapshot.identity {
            return Err(CanonicalPathError::ChangedDuringValidation { component_index });
        }
        directory = next;
    }

    Err(CanonicalPathError::CanonicalEscapesScanRoot)
}

pub(super) fn paths_equivalent(requested: &Path, canonical: &Path) -> bool {
    requested == canonical
}

fn normal_components(path: &Path) -> Result<impl Iterator<Item = &OsStr>, CanonicalPathError> {
    if path.components().any(|component| {
        !matches!(
            component,
            Component::RootDir | Component::Normal(_) | Component::Prefix(_)
        )
    }) {
        return Err(CanonicalPathError::CanonicalEscapesScanRoot);
    }
    Ok(path.components().filter_map(|component| match component {
        Component::Normal(component) => Some(component),
        _ => None,
    }))
}

fn open_directory_at(
    directory: &OwnedFd,
    component: &OsStr,
    component_index: usize,
) -> Result<OwnedFd, CanonicalPathError> {
    openat(directory, component, DIRECTORY_FLAGS, Mode::empty()).map_err(|error| {
        if error == nix::errno::Errno::ELOOP {
            CanonicalPathError::SymlinkOrReparsePoint {
                component_index,
                target: false,
            }
        } else {
            map_nix_error(component_index, error)
        }
    })
}

fn inspect_at(
    directory: &OwnedFd,
    component: &OsStr,
    component_index: usize,
    target: bool,
) -> Result<PlatformEntrySnapshot, CanonicalPathError> {
    let stat = fstatat(directory, component, AtFlags::AT_SYMLINK_NOFOLLOW)
        .map_err(|error| map_nix_error(component_index, error))?;
    let flags = SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT;
    if flags == SFlag::S_IFLNK {
        return Err(CanonicalPathError::SymlinkOrReparsePoint {
            component_index,
            target,
        });
    }
    let kind = kind(&stat).ok_or(CanonicalPathError::UnsupportedTargetKind)?;
    Ok(PlatformEntrySnapshot {
        identity: identity(&stat),
        kind,
        hard_link_count: hard_link_count(&stat),
    })
}

fn identity(stat: &nix::libc::stat) -> FilesystemIdentity {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let volume = stat.st_dev;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let volume = stat.st_dev as u64;

    FilesystemIdentity::new(volume, u128::from(stat.st_ino))
}

fn hard_link_count(stat: &nix::libc::stat) -> u64 {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        stat.st_nlink
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        stat.st_nlink as u64
    }
}

fn kind(stat: &nix::libc::stat) -> Option<FilesystemEntryKind> {
    let flags = SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT;
    if flags == SFlag::S_IFDIR {
        Some(FilesystemEntryKind::Directory)
    } else if flags == SFlag::S_IFREG {
        Some(FilesystemEntryKind::RegularFile)
    } else {
        None
    }
}

fn map_nix_error(component_index: usize, error: nix::errno::Errno) -> CanonicalPathError {
    map_io_error(
        component_index,
        std::io::Error::from_raw_os_error(error as i32),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_regular_file_open_is_nonblocking_against_fifo_replacement() {
        assert!(REGULAR_FILE_FLAGS.contains(OFlag::O_NONBLOCK));
    }
}
