//! Bounded, presentation-only details for durable scan coverage.
//!
//! Locations are historical observations shortened relative to the scan root.
//! They are deliberately componentized and cannot be passed back as cleanup or
//! filesystem authority.

use std::sync::Arc;

use crate::domain::{CoveragePermille, ScanCoverageStatus, ScanId, ScanIssueKind};

pub const MAX_SCAN_COVERAGE_DETAIL_PAGE_LIMIT: u16 = 64;
pub(crate) const MAX_SCAN_COVERAGE_LOCATION_COMPONENTS: usize = 8;
pub(crate) const MAX_SCAN_COVERAGE_LOCATION_COMPONENT_CHARS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DurableScanIssueKind {
    PermissionDenied,
    TimedOut,
    DifferentFilesystem,
    NetworkOrVirtualFilesystem,
    SymlinkSkipped,
    FileChangedDuringScan,
    MetadataError,
    Cancelled,
    PolicyExcluded,
    DepthLimited,
    ProbePoolExhausted,
    FilesystemBoundaryUnknown,
    IssueLimitReached,
}

impl From<ScanIssueKind> for DurableScanIssueKind {
    fn from(value: ScanIssueKind) -> Self {
        match value {
            ScanIssueKind::PermissionDenied => Self::PermissionDenied,
            ScanIssueKind::TimedOut => Self::TimedOut,
            ScanIssueKind::DifferentFilesystem => Self::DifferentFilesystem,
            ScanIssueKind::NetworkOrVirtualFilesystem => Self::NetworkOrVirtualFilesystem,
            ScanIssueKind::SymlinkSkipped => Self::SymlinkSkipped,
            ScanIssueKind::FileChangedDuringScan => Self::FileChangedDuringScan,
            ScanIssueKind::MetadataError => Self::MetadataError,
            ScanIssueKind::Cancelled => Self::Cancelled,
            ScanIssueKind::PolicyExcluded => Self::PolicyExcluded,
            ScanIssueKind::DepthLimited => Self::DepthLimited,
            ScanIssueKind::ProbePoolExhausted => Self::ProbePoolExhausted,
            ScanIssueKind::FilesystemBoundaryUnknown => Self::FilesystemBoundaryUnknown,
            ScanIssueKind::IssueLimitReached => Self::IssueLimitReached,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableScanIssueLocation {
    is_scan_root: bool,
    context_truncated: bool,
    components: Arc<[Arc<str>]>,
}

impl DurableScanIssueLocation {
    pub(super) fn new(
        is_scan_root: bool,
        context_truncated: bool,
        components: Vec<Arc<str>>,
    ) -> Self {
        Self {
            is_scan_root,
            context_truncated,
            components: components.into(),
        }
    }

    pub const fn is_scan_root(&self) -> bool {
        self.is_scan_root
    }

    pub const fn context_truncated(&self) -> bool {
        self.context_truncated
    }

    pub fn components(&self) -> &[Arc<str>] {
        &self.components
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableScanIssue {
    ordinal: u16,
    kind: DurableScanIssueKind,
    occurrence_count: u32,
    location: Option<DurableScanIssueLocation>,
}

impl DurableScanIssue {
    pub(super) const fn new(
        ordinal: u16,
        kind: DurableScanIssueKind,
        occurrence_count: u32,
        location: Option<DurableScanIssueLocation>,
    ) -> Self {
        Self {
            ordinal,
            kind,
            occurrence_count,
            location,
        }
    }

    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }

    pub const fn kind(&self) -> DurableScanIssueKind {
        self.kind
    }

    pub const fn occurrence_count(&self) -> u32 {
        self.occurrence_count
    }

    pub const fn location(&self) -> Option<&DurableScanIssueLocation> {
        self.location.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableScanCoverageDetailsPage {
    scan_id: ScanId,
    status: ScanCoverageStatus,
    measured_permille: Option<CoveragePermille>,
    offset: u16,
    total_issue_records: u16,
    total_issue_occurrences: u64,
    has_more: bool,
    issues: Arc<[DurableScanIssue]>,
}

impl DurableScanCoverageDetailsPage {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        scan_id: ScanId,
        status: ScanCoverageStatus,
        measured_permille: Option<CoveragePermille>,
        offset: u16,
        total_issue_records: u16,
        total_issue_occurrences: u64,
        has_more: bool,
        issues: Vec<DurableScanIssue>,
    ) -> Self {
        Self {
            scan_id,
            status,
            measured_permille,
            offset,
            total_issue_records,
            total_issue_occurrences,
            has_more,
            issues: issues.into(),
        }
    }

    pub const fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }
    pub const fn status(&self) -> ScanCoverageStatus {
        self.status
    }
    pub const fn measured_permille(&self) -> Option<CoveragePermille> {
        self.measured_permille
    }
    pub const fn offset(&self) -> u16 {
        self.offset
    }
    pub const fn total_issue_records(&self) -> u16 {
        self.total_issue_records
    }
    pub const fn total_issue_occurrences(&self) -> u64 {
        self.total_issue_occurrences
    }
    pub const fn has_more(&self) -> bool {
        self.has_more
    }
    pub fn issues(&self) -> &[DurableScanIssue] {
        &self.issues
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ScanCoverageDetailsError {
    #[error("coverage detail limit must be between 1 and {max}")]
    InvalidLimit { max: u16 },
    #[error("scan coverage detail offset is outside the retained issue page")]
    InvalidOffset,
    #[error("engine session is closed")]
    Closed,
    #[error("scan does not exist")]
    ScanNotFound,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the coverage detail query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable scan coverage is corrupt")]
    CorruptData,
    #[error("durable scan coverage is unavailable")]
    Unavailable,
    #[error("engine history state is unavailable")]
    InternalState,
}
