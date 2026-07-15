use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FileIdentity {
    volume: u64,
    object: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKind {
    Directory,
    #[cfg(windows)]
    SymlinkDirectory,
    RegularFile,
    Other,
}

#[derive(Debug)]
struct PlannedAncestor {
    path: PathBuf,
    identity: FileIdentity,
}

#[derive(Debug)]
struct PlannedEvidence {
    path: PathBuf,
    identity: FileIdentity,
}

#[derive(Debug)]
pub(super) struct PlannedDelete {
    target_identity: FileIdentity,
    ancestors: Vec<PlannedAncestor>,
    evidence: Vec<PlannedEvidence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EntrySnapshot {
    identity: FileIdentity,
    kind: EntryKind,
}

#[derive(Debug)]
pub(super) enum DeleteError {
    ChangedSincePlan { path: PathBuf, details: String },
    Remove { path: PathBuf, source: io::Error },
}

impl fmt::Display for DeleteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChangedSincePlan { path, details } => write!(
                formatter,
                "Skipped {}: it changed or could no longer be inspected since confirmation ({details}). Rescan before trying again.",
                path.display()
            ),
            Self::Remove { path, source } => {
                write!(formatter, "Could not delete {}: {source}", path.display())
            }
        }
    }
}

pub(super) fn capture_delete_plan(
    scan_root: &Path,
    path: &Path,
    evidence_paths: &[PathBuf],
) -> io::Result<PlannedDelete> {
    validate_delete_target(scan_root, path)?;
    let ancestors = capture_ancestors(scan_root, path)?;
    let scan_volume = ancestors
        .first()
        .expect("validated delete ancestry includes the scan root")
        .identity
        .volume;
    if ancestors
        .iter()
        .any(|ancestor| ancestor.identity.volume != scan_volume)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "delete target crosses a filesystem boundary",
        ));
    }

    let target = capture_entry(path)?;
    if target.identity.volume != scan_volume {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "delete target crosses a filesystem boundary",
        ));
    }

    let mut evidence = Vec::with_capacity(evidence_paths.len());
    for evidence_path in evidence_paths {
        let snapshot = capture_entry(evidence_path)?;
        if snapshot.kind != EntryKind::RegularFile || snapshot.identity.volume != scan_volume {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "artifact evidence is not a regular non-symlink file on the scan volume",
            ));
        }
        evidence.push(PlannedEvidence {
            path: evidence_path.clone(),
            identity: snapshot.identity,
        });
    }

    Ok(PlannedDelete {
        target_identity: target.identity,
        ancestors,
        evidence,
    })
}

fn validate_delete_target(scan_root: &Path, path: &Path) -> io::Result<()> {
    if !scan_root.is_absolute() || !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "delete target and scan root must be absolute",
        ));
    }

    let relative = path.strip_prefix(scan_root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "delete target is outside the scan root",
        )
    })?;
    let mut component_count = 0usize;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "delete target contains an unsafe path component",
            ));
        };
        let name = name.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "delete target contains invalid text encoding",
            )
        })?;
        if name.chars().any(char::is_control) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "delete target contains a control character",
            ));
        }
        component_count += 1;
    }
    if component_count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "delete target must be a strict descendant of the scan root",
        ));
    }
    Ok(())
}

fn capture_ancestors(scan_root: &Path, path: &Path) -> io::Result<Vec<PlannedAncestor>> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "delete target has no parent")
    })?;
    let relative_parent = parent.strip_prefix(scan_root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "delete target {} is outside scan root {}",
                path.display(),
                scan_root.display()
            ),
        )
    })?;

    let mut current = scan_root.to_path_buf();
    let mut paths = vec![current.clone()];
    for component in relative_parent.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "delete target contains an unsafe path component: {}",
                    path.display()
                ),
            ));
        };
        current.push(component);
        paths.push(current.clone());
    }

    let mut ancestors = Vec::with_capacity(paths.len());
    for ancestor_path in paths {
        let snapshot = capture_entry(&ancestor_path)?;
        if snapshot.kind != EntryKind::Directory {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "delete target has a non-directory or symlinked ancestor: {}",
                    ancestor_path.display()
                ),
            ));
        }
        ancestors.push(PlannedAncestor {
            path: ancestor_path,
            identity: snapshot.identity,
        });
    }
    Ok(ancestors)
}

#[expect(
    clippy::disallowed_methods,
    reason = "temporary legacy cleanup adapter is the only product deletion boundary"
)]
pub(super) fn execute_planned_delete(path: &Path, plan: PlannedDelete) -> Result<(), DeleteError> {
    for evidence in plan.evidence {
        let current =
            capture_entry(&evidence.path).map_err(|source| DeleteError::ChangedSincePlan {
                path: path.to_path_buf(),
                details: format!(
                    "artifact evidence {} could no longer be inspected: {source}",
                    evidence.path.display()
                ),
            })?;
        if current.kind != EntryKind::RegularFile || current.identity != evidence.identity {
            return Err(DeleteError::ChangedSincePlan {
                path: path.to_path_buf(),
                details: format!("artifact evidence {} changed", evidence.path.display()),
            });
        }
    }

    for ancestor in plan.ancestors {
        let current =
            capture_entry(&ancestor.path).map_err(|source| DeleteError::ChangedSincePlan {
                path: path.to_path_buf(),
                details: format!(
                    "ancestor {} could no longer be inspected: {source}",
                    ancestor.path.display()
                ),
            })?;
        if current.kind != EntryKind::Directory || current.identity != ancestor.identity {
            return Err(DeleteError::ChangedSincePlan {
                path: path.to_path_buf(),
                details: format!("ancestor {} changed", ancestor.path.display()),
            });
        }
    }

    let current = capture_entry(path).map_err(|source| DeleteError::ChangedSincePlan {
        path: path.to_path_buf(),
        details: source.to_string(),
    })?;

    if current.identity != plan.target_identity {
        return Err(DeleteError::ChangedSincePlan {
            path: path.to_path_buf(),
            details: "filesystem identity no longer matches".to_string(),
        });
    }

    let result = match current.kind {
        // DUX-DESTRUCTIVE: allow=legacy-delete-directory -- reviewed legacy adapter deletes the identity-checked planned directory
        EntryKind::Directory => std::fs::remove_dir_all(path),
        #[cfg(windows)]
        // DUX-DESTRUCTIVE: allow=legacy-delete-windows-link -- reviewed legacy adapter deletes the identity-checked planned Windows link
        EntryKind::SymlinkDirectory => std::fs::remove_dir_all(path),
        // DUX-DESTRUCTIVE: allow=legacy-delete-file -- reviewed legacy adapter deletes the identity-checked planned file
        EntryKind::RegularFile | EntryKind::Other => std::fs::remove_file(path),
    };

    result.map_err(|source| DeleteError::Remove {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(unix)]
fn capture_entry(path: &Path) -> io::Result<EntrySnapshot> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path)?;
    Ok(EntrySnapshot {
        identity: FileIdentity {
            volume: metadata.dev(),
            object: u128::from(metadata.ino()),
        },
        kind: if metadata.file_type().is_dir() {
            EntryKind::Directory
        } else if metadata.file_type().is_file() {
            EntryKind::RegularFile
        } else {
            EntryKind::Other
        },
    })
}

#[cfg(windows)]
fn capture_entry(path: &Path) -> io::Result<EntrySnapshot> {
    use std::fs::OpenOptions;
    use std::mem::{MaybeUninit, size_of};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileBasicInfo, FileIdInfo,
        GetFileInformationByHandleEx,
    };

    let file = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let handle = file.as_raw_handle();

    let mut id_info = MaybeUninit::<FILE_ID_INFO>::zeroed();
    // SAFETY: `file` owns a valid handle for the duration of the call and the
    // output buffer has the exact size required for `FILE_ID_INFO`.
    let id_result = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            id_info.as_mut_ptr().cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if id_result == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the successful call initialized the complete output structure.
    let id_info = unsafe { id_info.assume_init() };

    let mut basic_info = MaybeUninit::<FILE_BASIC_INFO>::zeroed();
    // SAFETY: identical to the `FILE_ID_INFO` call above, with the matching
    // output type and size.
    let basic_result = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            basic_info.as_mut_ptr().cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    if basic_result == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the successful call initialized the complete output structure.
    let basic_info = unsafe { basic_info.assume_init() };

    Ok(EntrySnapshot {
        identity: FileIdentity {
            volume: id_info.VolumeSerialNumber,
            object: u128::from_le_bytes(id_info.FileId.Identifier),
        },
        kind: if basic_info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            if basic_info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                EntryKind::SymlinkDirectory
            } else {
                EntryKind::Other
            }
        } else if basic_info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            EntryKind::Directory
        } else if basic_info.FileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DEVICE)
            == 0
        {
            EntryKind::RegularFile
        } else {
            EntryKind::Other
        },
    })
}

#[cfg(not(any(unix, windows)))]
fn capture_entry(_path: &Path) -> io::Result<EntrySnapshot> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "filesystem identity checks are not supported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn plan(path: &Path) -> PlannedDelete {
        capture_delete_plan(path.parent().unwrap(), path, &[]).unwrap()
    }

    #[test]
    fn unchanged_file_is_deleted() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("file.txt");
        std::fs::write(&path, b"original").unwrap();
        let plan = plan(&path);

        execute_planned_delete(&path, plan).unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn terminal_dot_components_cannot_escape_or_alias_the_scan_root() {
        let temp = TempDir::new().unwrap();
        let scan_root = temp.path().join("scan-root");
        std::fs::create_dir(&scan_root).unwrap();
        let outside = temp.path().join("outside-sentinel");
        let inside = scan_root.join("inside-sentinel");
        std::fs::write(&outside, b"keep").unwrap();
        std::fs::write(&inside, b"keep").unwrap();

        for target in [
            scan_root.clone(),
            scan_root.join("."),
            scan_root.join(".."),
            scan_root.join("child").join(".."),
            scan_root.join("..").join("outside-sentinel"),
        ] {
            assert!(
                capture_delete_plan(&scan_root, &target, &[]).is_err(),
                "unsafe target was admitted: {}",
                target.display()
            );
        }

        assert!(outside.exists());
        assert!(inside.exists());
        assert!(scan_root.exists());
    }

    #[test]
    fn control_character_target_is_rejected_before_confirmation() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("line\nbreak");
        std::fs::write(&path, b"keep").unwrap();

        assert!(capture_delete_plan(temp.path(), &path, &[]).is_err());
        assert!(path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn invalid_text_target_is_rejected_before_filesystem_inspection() {
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let temp = TempDir::new().unwrap();
        let mut bytes = temp.path().as_os_str().as_bytes().to_vec();
        bytes.extend_from_slice(b"/");
        bytes.push(0xff);
        let path = PathBuf::from(OsString::from_vec(bytes));

        assert!(capture_delete_plan(temp.path(), &path, &[]).is_err());
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test replaces a TempDir-owned fixture to exercise TOCTOU rejection"
    )]
    fn replaced_file_is_not_deleted() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("file.txt");
        let original = temp.path().join("original.txt");
        std::fs::write(&path, b"original").unwrap();
        let plan = plan(&path);
        // DUX-DESTRUCTIVE: allow=test-delete-replaced-file -- replace a TempDir-owned fixture to verify stale identity rejection
        std::fs::rename(&path, &original).unwrap();
        std::fs::write(&path, b"replacement").unwrap();

        let error = execute_planned_delete(&path, plan).unwrap_err();

        assert!(matches!(error, DeleteError::ChangedSincePlan { .. }));
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(std::fs::read(&original).unwrap(), b"original");
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test replaces a TempDir-owned fixture to exercise TOCTOU rejection"
    )]
    fn replaced_directory_is_not_deleted() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("directory");
        let original = temp.path().join("original-directory");
        std::fs::create_dir(&path).unwrap();
        let plan = plan(&path);
        // DUX-DESTRUCTIVE: allow=test-delete-replaced-directory -- replace a TempDir-owned directory to verify stale identity rejection
        std::fs::rename(&path, &original).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("sentinel"), b"keep").unwrap();

        let error = execute_planned_delete(&path, plan).unwrap_err();

        assert!(matches!(error, DeleteError::ChangedSincePlan { .. }));
        assert!(path.join("sentinel").exists());
        assert!(original.exists());
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test removes a TempDir-owned fixture to exercise disappearance handling"
    )]
    fn missing_entry_is_not_counted_as_deleted() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("file.txt");
        std::fs::write(&path, b"original").unwrap();
        let plan = plan(&path);
        // DUX-DESTRUCTIVE: allow=test-delete-missing-entry -- remove a TempDir-owned fixture to verify disappearance is not credited
        std::fs::remove_file(&path).unwrap();

        let error = execute_planned_delete(&path, plan).unwrap_err();

        assert!(matches!(error, DeleteError::ChangedSincePlan { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn unchanged_symlink_deletes_only_the_link() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target");
        let link = temp.path().join("link");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("sentinel"), b"keep").unwrap();
        symlink(&target, &link).unwrap();
        let plan = plan(&link);

        execute_planned_delete(&link, plan).unwrap();

        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(target.join("sentinel").exists());
    }

    #[cfg(unix)]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test replaces a TempDir-owned symlink to exercise TOCTOU rejection"
    )]
    fn replaced_symlink_is_not_deleted() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let first_target = temp.path().join("first");
        let second_target = temp.path().join("second");
        let link = temp.path().join("link");
        let original_link = temp.path().join("original-link");
        std::fs::create_dir(&first_target).unwrap();
        std::fs::create_dir(&second_target).unwrap();
        symlink(&first_target, &link).unwrap();
        let plan = plan(&link);
        // DUX-DESTRUCTIVE: allow=test-delete-replaced-symlink -- replace a TempDir-owned symlink to verify stale identity rejection
        std::fs::rename(&link, &original_link).unwrap();
        symlink(&second_target, &link).unwrap();

        let error = execute_planned_delete(&link, plan).unwrap_err();

        assert!(matches!(error, DeleteError::ChangedSincePlan { .. }));
        assert_eq!(std::fs::read_link(&link).unwrap(), second_target);
        assert_eq!(std::fs::read_link(&original_link).unwrap(), first_target);
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test replaces TempDir-owned evidence to exercise TOCTOU rejection"
    )]
    fn changed_artifact_evidence_blocks_delete() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target");
        let evidence = temp.path().join("Cargo.toml");
        let original_evidence = temp.path().join("original-Cargo.toml");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(&evidence, b"original").unwrap();
        let plan =
            capture_delete_plan(temp.path(), &target, std::slice::from_ref(&evidence)).unwrap();
        // DUX-DESTRUCTIVE: allow=test-delete-changed-evidence -- replace TempDir-owned evidence to verify stale artifact rejection
        std::fs::rename(&evidence, &original_evidence).unwrap();
        std::fs::write(&evidence, b"replacement").unwrap();

        let error = execute_planned_delete(&target, plan).unwrap_err();

        assert!(matches!(error, DeleteError::ChangedSincePlan { .. }));
        assert!(target.exists());
        assert_eq!(std::fs::read(&evidence).unwrap(), b"replacement");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_artifact_evidence_is_rejected_at_plan_time() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target");
        let real_evidence = temp.path().join("real-Cargo.toml");
        let evidence = temp.path().join("Cargo.toml");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(&real_evidence, b"manifest").unwrap();
        symlink(&real_evidence, &evidence).unwrap();

        assert!(capture_delete_plan(temp.path(), &target, &[evidence]).is_err());
        assert!(target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn existing_symlink_ancestor_is_rejected_at_plan_time() {
        use std::os::unix::fs::symlink;

        let scan = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        let external_target = external.path().join("target");
        std::fs::create_dir(&external_target).unwrap();
        let project = scan.path().join("project");
        symlink(external.path(), &project).unwrap();

        assert!(capture_delete_plan(scan.path(), &project.join("target"), &[]).is_err());
        assert!(external_target.exists());
    }

    #[cfg(unix)]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test replaces a TempDir-owned ancestor to exercise redirect rejection"
    )]
    fn replaced_ancestor_cannot_redirect_delete_outside_scan_root() {
        use std::os::unix::fs::symlink;

        let scan = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        let project = scan.path().join("project");
        let original_project = scan.path().join("original-project");
        let target = project.join("target");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("original-sentinel"), b"keep").unwrap();
        let plan = capture_delete_plan(scan.path(), &target, &[]).unwrap();

        let external_target = external.path().join("target");
        std::fs::create_dir(&external_target).unwrap();
        let external_sentinel = external_target.join("external-sentinel");
        std::fs::write(&external_sentinel, b"keep").unwrap();
        // DUX-DESTRUCTIVE: allow=test-delete-replaced-ancestor -- replace a TempDir-owned ancestor to verify redirect rejection
        std::fs::rename(&project, &original_project).unwrap();
        symlink(external.path(), &project).unwrap();

        let error = execute_planned_delete(&target, plan).unwrap_err();

        assert!(matches!(error, DeleteError::ChangedSincePlan { .. }));
        assert!(external_sentinel.exists());
        assert!(original_project.join("target/original-sentinel").exists());
    }
}
