use std::fs::Metadata;
#[cfg(windows)]
use std::io;
use std::path::Path;

use crate::ScanIssueKind;
use crate::time::cache_serializable_time;

use super::facts::{ScanNodeFacts, ScanObjectIdentity};

#[derive(Debug)]
pub(super) struct AccountingError {
    issue_kind: ScanIssueKind,
}

impl AccountingError {
    fn metadata() -> Self {
        Self {
            issue_kind: ScanIssueKind::MetadataError,
        }
    }

    #[cfg(windows)]
    fn changed() -> Self {
        Self {
            issue_kind: ScanIssueKind::FileChangedDuringScan,
        }
    }

    #[cfg(windows)]
    fn from_io(error: &io::Error) -> Self {
        let issue_kind = match error.kind() {
            io::ErrorKind::PermissionDenied => ScanIssueKind::PermissionDenied,
            io::ErrorKind::TimedOut => ScanIssueKind::TimedOut,
            io::ErrorKind::NotFound => ScanIssueKind::FileChangedDuringScan,
            _ => ScanIssueKind::MetadataError,
        };
        Self { issue_kind }
    }

    pub(super) const fn issue_kind(&self) -> ScanIssueKind {
        self.issue_kind
    }
}

#[cfg(unix)]
pub(super) fn capture_node_facts(
    _path: &Path,
    metadata: &Metadata,
    _follow_final_symlink: bool,
    _expected_directory: Option<bool>,
) -> Result<(ScanNodeFacts, Option<ScanIssueKind>), AccountingError> {
    use std::os::unix::fs::MetadataExt;

    let allocated_bytes = metadata
        .blocks()
        .checked_mul(512)
        .ok_or_else(AccountingError::metadata)?;
    Ok((
        ScanNodeFacts::captured(
            metadata.len(),
            Some(allocated_bytes),
            metadata.modified().ok().and_then(cache_serializable_time),
            metadata.accessed().ok().and_then(cache_serializable_time),
            Some(ScanObjectIdentity::Unix {
                device: metadata.dev(),
                inode: metadata.ino(),
            }),
            Some(metadata.nlink()),
        ),
        None,
    ))
}

#[cfg(windows)]
pub(super) fn capture_node_facts(
    path: &Path,
    metadata: &Metadata,
    follow_final_symlink: bool,
    expected_directory: Option<bool>,
) -> Result<(ScanNodeFacts, Option<ScanIssueKind>), AccountingError> {
    use std::fs::OpenOptions;
    use std::mem::{MaybeUninit, size_of};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, FileIdInfo, FileStandardInfo,
        GetFileInformationByHandleEx,
    };

    let custom_flags = FILE_FLAG_BACKUP_SEMANTICS
        | if follow_final_symlink {
            0
        } else {
            FILE_FLAG_OPEN_REPARSE_POINT
        };
    let file = OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(custom_flags)
        .open(path)
        .map_err(|error| AccountingError::from_io(&error))?;
    let handle = file.as_raw_handle();

    let mut standard = MaybeUninit::<FILE_STANDARD_INFO>::zeroed();
    // SAFETY: as above, with the matching FILE_STANDARD_INFO buffer.
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileStandardInfo,
            standard.as_mut_ptr().cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        return Err(AccountingError::from_io(&error));
    }
    // SAFETY: the successful call initialized the complete structure.
    let standard = unsafe { standard.assume_init() };
    let allocated_bytes =
        u64::try_from(standard.AllocationSize).map_err(|_| AccountingError::metadata())?;
    let logical_bytes =
        u64::try_from(standard.EndOfFile).map_err(|_| AccountingError::metadata())?;
    if logical_bytes != metadata.len()
        || expected_directory.is_some_and(|expected| expected != standard.Directory)
        || standard.DeletePending
        || standard.NumberOfLinks == 0
    {
        return Err(AccountingError::changed());
    }

    let mut id = MaybeUninit::<FILE_ID_INFO>::zeroed();
    // SAFETY: `file` retains a valid handle and the output buffer and length
    // exactly match FILE_ID_INFO.
    let id_query_succeeded = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            id.as_mut_ptr().cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } != 0;
    let (identity, identity_issue) = if id_query_succeeded {
        // SAFETY: the successful call initialized the complete structure.
        let id = unsafe { id.assume_init() };
        (
            Some(ScanObjectIdentity::Windows {
                volume_serial: id.VolumeSerialNumber,
                file_id: id.FileId.Identifier,
            }),
            None,
        )
    } else {
        let error = io::Error::last_os_error();
        (None, Some(AccountingError::from_io(&error).issue_kind()))
    };
    let link_count = u64::from(standard.NumberOfLinks);
    let trustworthy_allocation = (identity.is_some() || link_count <= 1).then_some(allocated_bytes);

    Ok((
        ScanNodeFacts::captured(
            logical_bytes,
            trustworthy_allocation,
            metadata.modified().ok().and_then(cache_serializable_time),
            metadata.accessed().ok().and_then(cache_serializable_time),
            identity,
            Some(link_count),
        ),
        identity_issue,
    ))
}

pub(super) fn logical_only_facts(metadata: &Metadata) -> ScanNodeFacts {
    ScanNodeFacts::captured(
        metadata.len(),
        None,
        metadata.modified().ok().and_then(cache_serializable_time),
        metadata.accessed().ok().and_then(cache_serializable_time),
        None,
        None,
    )
}
