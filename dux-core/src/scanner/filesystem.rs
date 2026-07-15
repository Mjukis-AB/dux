use std::path::Path;

/// Coarse classification used to decide whether crossing a mount is safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
pub(super) enum FilesystemDisposition {
    Local,
    NetworkOrVirtual,
    Unknown,
}

/// Classify the filesystem containing `path`.
///
/// Probe failures are deliberately unknown so callers can preserve the
/// scanner's existing fail-open behavior. The caller is responsible for
/// bounding how long this potentially blocking system call may take.
#[cfg(target_os = "macos")]
pub(super) fn probe(path: &Path) -> FilesystemDisposition {
    use nix::mount::MntFlags;

    let Ok(stats) = nix::sys::statfs::statfs(path) else {
        return FilesystemDisposition::Unknown;
    };

    classify_macos(
        stats.filesystem_type_name(),
        stats.flags().contains(MntFlags::MNT_LOCAL),
    )
}

#[cfg(target_os = "macos")]
fn classify_macos(type_name: &str, is_local: bool) -> FilesystemDisposition {
    let is_known_network_or_virtual = matches!(
        type_name.to_ascii_lowercase().as_str(),
        "afpfs"
            | "autofs"
            | "devfs"
            | "fdesc"
            | "fusefs"
            | "macfuse"
            | "nfs"
            | "osxfuse"
            | "smbfs"
            | "webdav"
    );

    if is_known_network_or_virtual || !is_local {
        FilesystemDisposition::NetworkOrVirtual
    } else {
        FilesystemDisposition::Local
    }
}

#[cfg(target_os = "linux")]
pub(super) fn probe(path: &Path) -> FilesystemDisposition {
    let Ok(stats) = nix::sys::statfs::statfs(path) else {
        return FilesystemDisposition::Unknown;
    };

    classify_linux(stats.filesystem_type())
}

#[cfg(target_os = "linux")]
fn classify_linux(fs_type: nix::sys::statfs::FsType) -> FilesystemDisposition {
    use nix::sys::statfs::{
        AUTOFS_SUPER_MAGIC, DEVPTS_SUPER_MAGIC, FUSE_SUPER_MAGIC, NFS_SUPER_MAGIC,
        PROC_SUPER_MAGIC, SMB_SUPER_MAGIC, SYSFS_MAGIC,
    };

    // The nix crate exposes the legacy SMB magic but not the magic used by
    // modern Linux CIFS mounts.
    const CIFS_MAGIC_NUMBER: nix::sys::statfs::FsType = nix::sys::statfs::FsType(0xff53_4d42);

    if matches!(
        fs_type,
        AUTOFS_SUPER_MAGIC
            | DEVPTS_SUPER_MAGIC
            | FUSE_SUPER_MAGIC
            | NFS_SUPER_MAGIC
            | PROC_SUPER_MAGIC
            | SMB_SUPER_MAGIC
            | SYSFS_MAGIC
            | CIFS_MAGIC_NUMBER
    ) {
        FilesystemDisposition::NetworkOrVirtual
    } else {
        FilesystemDisposition::Local
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn probe(_path: &Path) -> FilesystemDisposition {
    FilesystemDisposition::Unknown
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_classifies_local_and_problematic_filesystems() {
        assert_eq!(classify_macos("apfs", true), FilesystemDisposition::Local);
        assert_eq!(classify_macos("exfat", true), FilesystemDisposition::Local);
        for type_name in [
            "afpfs", "autofs", "devfs", "fdesc", "fusefs", "macfuse", "nfs", "osxfuse", "smbfs",
            "webdav",
        ] {
            assert_eq!(
                classify_macos(type_name, true),
                FilesystemDisposition::NetworkOrVirtual,
                "{type_name}"
            );
        }
        assert_eq!(
            classify_macos("unrecognized", false),
            FilesystemDisposition::NetworkOrVirtual
        );
        assert_eq!(classify_macos("APFS", true), FilesystemDisposition::Local);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_classifies_local_and_problematic_filesystems() {
        use nix::sys::statfs::{
            EXT4_SUPER_MAGIC, FUSE_SUPER_MAGIC, NFS_SUPER_MAGIC, PROC_SUPER_MAGIC, SMB_SUPER_MAGIC,
            SYSFS_MAGIC,
        };
        const CIFS_MAGIC_NUMBER: nix::sys::statfs::FsType = nix::sys::statfs::FsType(0xff53_4d42);

        assert_eq!(
            classify_linux(EXT4_SUPER_MAGIC),
            FilesystemDisposition::Local
        );
        for fs_type in [
            FUSE_SUPER_MAGIC,
            NFS_SUPER_MAGIC,
            PROC_SUPER_MAGIC,
            SMB_SUPER_MAGIC,
            SYSFS_MAGIC,
            CIFS_MAGIC_NUMBER,
        ] {
            assert_eq!(
                classify_linux(fs_type),
                FilesystemDisposition::NetworkOrVirtual
            );
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn probe_identifies_the_test_directory_filesystem() {
        let temp = tempfile::TempDir::new().unwrap();
        assert_ne!(probe(temp.path()), FilesystemDisposition::Unknown);
    }
}
