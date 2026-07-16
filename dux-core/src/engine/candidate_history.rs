//! Bounded presentation and review-intent types for durable candidates.
//!
//! Detail values are explicit local path disclosures requested by the user.
//! They are historical observations only: none of these types can be consumed
//! by the planner or executor, and no command accepts a caller-supplied path.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::domain::{CandidateId, ScanId};

use super::task::{DurableCandidateStatus, DurableCandidateSummary};

pub const MAX_CANDIDATE_DETAIL_PAGE_LIMIT: u16 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DurablePathEncoding {
    Utf8,
    Utf16LittleEndian,
}

/// Lossless accepted-host path plus a display-only rendering. Consumers must
/// use `encoded_bytes` for copying or round-tripping; `display` may be styled
/// or shortened by a UI and is never an execution input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableObservedPath {
    encoding: DurablePathEncoding,
    encoded_bytes: Arc<[u8]>,
    display: Arc<str>,
}

impl DurableObservedPath {
    pub(super) fn new(
        encoding: DurablePathEncoding,
        encoded_bytes: Vec<u8>,
        display: String,
    ) -> Self {
        Self {
            encoding,
            encoded_bytes: encoded_bytes.into(),
            display: display.into(),
        }
    }

    pub const fn encoding(&self) -> DurablePathEncoding {
        self.encoding
    }

    pub fn encoded_bytes(&self) -> &[u8] {
        &self.encoded_bytes
    }

    pub fn display(&self) -> &str {
        &self.display
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCandidatePathItem {
    ordinal: u16,
    path: DurableObservedPath,
}

impl DurableCandidatePathItem {
    pub(super) const fn new(ordinal: u16, path: DurableObservedPath) -> Self {
        Self { ordinal, path }
    }

    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }

    pub const fn path(&self) -> &DurableObservedPath {
        &self.path
    }
}

/// Typed discovery evidence copied into a presentation-only durable DTO. The
/// distinct type prevents stored evidence from satisfying planner validation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DurableCandidateEvidence {
    MatchedPath {
        path: DurableObservedPath,
    },
    RequiredMarker {
        path: DurableObservedPath,
    },
    ForbiddenMarkerAbsent {
        path: DurableObservedPath,
    },
    BundleIdentifier {
        path: DurableObservedPath,
        identifier: Arc<str>,
    },
    MinimumAge {
        newest_mtime: SystemTime,
        minimum_age: Duration,
    },
    MinimumSize {
        observed_bytes: u64,
        minimum_bytes: u64,
    },
    InactiveProcess {
        identifier: Arc<str>,
    },
    CloudUploadComplete {
        path: DurableObservedPath,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCandidateEvidenceItem {
    ordinal: u16,
    evidence: DurableCandidateEvidence,
}

impl DurableCandidateEvidenceItem {
    pub(super) const fn new(ordinal: u16, evidence: DurableCandidateEvidence) -> Self {
        Self { ordinal, evidence }
    }

    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }

    pub const fn evidence(&self) -> &DurableCandidateEvidence {
        &self.evidence
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCandidatePathPage {
    scan_id: ScanId,
    candidate: DurableCandidateSummary,
    cursor: u16,
    next_cursor: Option<u16>,
    total_paths: u16,
    paths: Arc<[DurableCandidatePathItem]>,
}

impl DurableCandidatePathPage {
    pub(super) fn new(
        scan_id: ScanId,
        candidate: DurableCandidateSummary,
        cursor: u16,
        next_cursor: Option<u16>,
        total_paths: u16,
        paths: Vec<DurableCandidatePathItem>,
    ) -> Self {
        Self {
            scan_id,
            candidate,
            cursor,
            next_cursor,
            total_paths,
            paths: paths.into(),
        }
    }

    pub const fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    pub const fn candidate(&self) -> &DurableCandidateSummary {
        &self.candidate
    }

    pub const fn cursor(&self) -> u16 {
        self.cursor
    }

    pub const fn next_cursor(&self) -> Option<u16> {
        self.next_cursor
    }

    pub const fn total_paths(&self) -> u16 {
        self.total_paths
    }

    pub fn paths(&self) -> &[DurableCandidatePathItem] {
        &self.paths
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCandidateEvidencePage {
    scan_id: ScanId,
    candidate: DurableCandidateSummary,
    cursor: u16,
    next_cursor: Option<u16>,
    total_evidence: u16,
    evidence: Arc<[DurableCandidateEvidenceItem]>,
}

impl DurableCandidateEvidencePage {
    pub(super) fn new(
        scan_id: ScanId,
        candidate: DurableCandidateSummary,
        cursor: u16,
        next_cursor: Option<u16>,
        total_evidence: u16,
        evidence: Vec<DurableCandidateEvidenceItem>,
    ) -> Self {
        Self {
            scan_id,
            candidate,
            cursor,
            next_cursor,
            total_evidence,
            evidence: evidence.into(),
        }
    }

    pub const fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    pub const fn candidate(&self) -> &DurableCandidateSummary {
        &self.candidate
    }

    pub const fn cursor(&self) -> u16 {
        self.cursor
    }

    pub const fn next_cursor(&self) -> Option<u16> {
        self.next_cursor
    }

    pub const fn total_evidence(&self) -> u16 {
        self.total_evidence
    }

    pub fn evidence(&self) -> &[DurableCandidateEvidenceItem] {
        &self.evidence
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CandidateDetailError {
    #[error("engine session is closed")]
    Closed,
    #[error("candidate detail page limit must be between 1 and {maximum}")]
    InvalidLimit { maximum: u16 },
    #[error("candidate detail cursor is outside the immutable observation")]
    CursorOutOfRange,
    #[error("the requested durable scan does not exist")]
    ScanNotFound,
    #[error("the requested scan has no successful candidate observation")]
    EvaluationNotSucceeded,
    #[error("the requested candidate does not belong to this scan")]
    CandidateNotFound,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the candidate detail query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable candidate detail is corrupt")]
    CorruptData,
    #[error("durable candidate detail is unavailable")]
    Unavailable,
    #[error("engine candidate-detail state is unavailable")]
    InternalState,
}

/// Semantic review intent. There is deliberately no arbitrary status setter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CandidateReviewCommand {
    Select,
    ClearSelection,
    Dismiss,
    Restore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CandidateReviewError {
    #[error("engine session is closed")]
    Closed,
    #[error("the requested candidate does not belong to this scan")]
    CandidateNotFound,
    #[error("the requested review command is not valid for this candidate")]
    NotReviewable,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("the candidate review query exceeded its fixed resource budget")]
    QueryLimitExceeded,
    #[error("durable candidate review state is corrupt")]
    CorruptData,
    #[error("the candidate review outcome is unknown")]
    OutcomeUnknown,
    #[error("durable candidate review is unavailable")]
    Unavailable,
    #[error("engine candidate-review state is unavailable")]
    InternalState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateReviewResult {
    scan_id: ScanId,
    candidate_id: CandidateId,
    status: DurableCandidateStatus,
}

impl CandidateReviewResult {
    pub(super) const fn new(
        scan_id: ScanId,
        candidate_id: CandidateId,
        status: DurableCandidateStatus,
    ) -> Self {
        Self {
            scan_id,
            candidate_id,
            status,
        }
    }

    pub const fn scan_id(&self) -> &ScanId {
        &self.scan_id
    }

    pub const fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    pub const fn status(&self) -> DurableCandidateStatus {
        self.status
    }
}
