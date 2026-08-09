//! Exact retained-review binding for path-free AI input disclosure.
//!
//! This module is the only engine code allowed to import the private AI
//! shaper. It creates no provider, request task, cache row, candidate, plan,
//! approval, or effect authority.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crate::ScanId;
use crate::ai::{
    AI_METADATA_INPUT_SCHEMA_VERSION, AiMetadataAgeSummaryV1, AiMetadataChildV1,
    AiMetadataNodeKindV1, AiMetadataPreviewV1, AiMetadataProjectionV1, AiMetadataShapeError,
    shape_ai_metadata_preview_v1,
};

use super::snapshot_review::{SnapshotReviewError, SnapshotReviewOwner, SnapshotReviewSession};

pub const AI_METADATA_PREVIEW_LIFETIME: Duration = Duration::from_secs(2 * 60);
pub const AI_METADATA_PRIVACY_POLICY_REVISION: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiMetadataPreviewNodeKind {
    Directory,
    File,
    Symlink,
    Other,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiMetadataPreviewAgeSummary {
    pub within_7_days_logical_bytes: u64,
    pub days_8_to_30_logical_bytes: u64,
    pub days_31_to_90_logical_bytes: u64,
    pub older_than_90_days_logical_bytes: u64,
    pub unknown_age_logical_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiMetadataPreviewChild {
    pub input_node_id: String,
    pub label: String,
    pub kind: AiMetadataPreviewNodeKind,
    pub logical_bytes: u64,
    pub age_summary: AiMetadataPreviewAgeSummary,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AiMetadataPreviewInfo {
    input_schema_version: u64,
    privacy_policy_revision: u64,
    prepared_at: SystemTime,
    effective_expires_at: SystemTime,
    input_digest_sha256: String,
    encoded_input_json_utf8: Arc<[u8]>,
    inspected_node_count: u64,
    included_direct_child_count: u64,
    excluded_sensitive_direct_child_count: u64,
    omitted_eligible_direct_child_count: u64,
    root_label: String,
    total_logical_bytes: u64,
    age_summary: AiMetadataPreviewAgeSummary,
    children_complete: bool,
    omitted_child_count: u64,
    omitted_logical_bytes: u64,
    omitted_age_summary: AiMetadataPreviewAgeSummary,
    children: Vec<AiMetadataPreviewChild>,
}

impl fmt::Debug for AiMetadataPreviewInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiMetadataPreviewInfo")
            .field("input_schema_version", &self.input_schema_version)
            .field("privacy_policy_revision", &self.privacy_policy_revision)
            .field("prepared_at", &self.prepared_at)
            .field("effective_expires_at", &self.effective_expires_at)
            .field("encoded_input_bytes", &self.encoded_input_json_utf8.len())
            .field("inspected_node_count", &self.inspected_node_count)
            .field(
                "included_direct_child_count",
                &self.included_direct_child_count,
            )
            .field(
                "excluded_sensitive_direct_child_count",
                &self.excluded_sensitive_direct_child_count,
            )
            .field(
                "omitted_eligible_direct_child_count",
                &self.omitted_eligible_direct_child_count,
            )
            .finish_non_exhaustive()
    }
}

impl AiMetadataPreviewInfo {
    pub const fn input_schema_version(&self) -> u64 {
        self.input_schema_version
    }

    pub const fn privacy_policy_revision(&self) -> u64 {
        self.privacy_policy_revision
    }

    pub const fn prepared_at(&self) -> SystemTime {
        self.prepared_at
    }

    pub const fn effective_expires_at(&self) -> SystemTime {
        self.effective_expires_at
    }

    pub fn input_digest_sha256(&self) -> &str {
        &self.input_digest_sha256
    }

    pub fn encoded_input_json_utf8(&self) -> &[u8] {
        &self.encoded_input_json_utf8
    }

    pub const fn inspected_node_count(&self) -> u64 {
        self.inspected_node_count
    }

    pub const fn included_direct_child_count(&self) -> u64 {
        self.included_direct_child_count
    }

    pub const fn excluded_sensitive_direct_child_count(&self) -> u64 {
        self.excluded_sensitive_direct_child_count
    }

    pub const fn omitted_eligible_direct_child_count(&self) -> u64 {
        self.omitted_eligible_direct_child_count
    }

    pub fn root_label(&self) -> &str {
        &self.root_label
    }

    pub const fn total_logical_bytes(&self) -> u64 {
        self.total_logical_bytes
    }

    pub const fn age_summary(&self) -> AiMetadataPreviewAgeSummary {
        self.age_summary
    }

    pub const fn children_complete(&self) -> bool {
        self.children_complete
    }

    pub const fn omitted_child_count(&self) -> u64 {
        self.omitted_child_count
    }

    pub const fn omitted_logical_bytes(&self) -> u64 {
        self.omitted_logical_bytes
    }

    pub const fn omitted_age_summary(&self) -> AiMetadataPreviewAgeSummary {
        self.omitted_age_summary
    }

    pub fn children(&self) -> &[AiMetadataPreviewChild] {
        &self.children
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AiMetadataPreviewError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the retained Explorer review belongs to a different engine")]
    WrongReview,
    #[error("the retained Explorer review is unavailable")]
    ReviewUnavailable,
    #[error("AI metadata requires complete scan coverage")]
    IncompleteCoverage,
    #[error("the selected snapshot observation is unavailable")]
    SelectionUnavailable,
    #[error("AI metadata requires a directory selection")]
    SelectionNotDirectory,
    #[error("the selected observation is sensitive")]
    SensitiveSelection,
    #[error("the selected observation cannot be represented safely")]
    UnsupportedObservation,
    #[error("AI metadata shaping exceeded its bounded budget")]
    BudgetExceeded,
    #[error("the AI metadata preview clock is invalid")]
    InvalidClock,
    #[error("the retained snapshot store failed its safety checks")]
    UnsafeStorage,
    #[error("the retained snapshot data is corrupt")]
    CorruptData,
    #[error("the AI metadata preview is unavailable")]
    Unavailable,
    #[error("the AI metadata preview reached an invalid internal state")]
    InternalState,
}

/// Non-cloneable, engine-bound privacy proof for one exact Explorer review.
/// The FFI wrapper strongly retains the parent review for this object's life.
pub struct AiMetadataPreview {
    owner: Arc<SnapshotReviewOwner>,
    parent_session_identity: u64,
    parent_liveness: Arc<AtomicBool>,
    source_scan_id: ScanId,
    monotonic_deadline: Instant,
    info: AiMetadataPreviewInfo,
    _sealed_proof: AiMetadataPreviewV1,
}

impl fmt::Debug for AiMetadataPreview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiMetadataPreview")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl AiMetadataPreview {
    pub fn info(
        &self,
        parent: &SnapshotReviewSession,
    ) -> Result<&AiMetadataPreviewInfo, AiMetadataPreviewError> {
        self.info_at(parent, SystemTime::now(), Instant::now())
    }

    pub(super) fn info_at(
        &self,
        parent: &SnapshotReviewSession,
        wall_now: SystemTime,
        monotonic_now: Instant,
    ) -> Result<&AiMetadataPreviewInfo, AiMetadataPreviewError> {
        self.validate_parent(parent, wall_now)?;
        if monotonic_now >= self.monotonic_deadline || wall_now >= self.info.effective_expires_at {
            return Err(AiMetadataPreviewError::ReviewUnavailable);
        }
        Ok(&self.info)
    }

    fn validate_parent(
        &self,
        parent: &SnapshotReviewSession,
        observed_at: SystemTime,
    ) -> Result<(), AiMetadataPreviewError> {
        if !parent.belongs_to(&self.owner)
            || parent.session_identity() != self.parent_session_identity
            || parent.scan_id() != &self.source_scan_id
        {
            return Err(AiMetadataPreviewError::WrongReview);
        }
        if !self.parent_liveness.load(Ordering::Acquire) || parent.is_released() {
            return Err(AiMetadataPreviewError::ReviewUnavailable);
        }
        parent
            .validate_and_expires_at(observed_at)
            .map(|_| ())
            .map_err(map_review_error)
    }
}

pub(super) fn prepare_ai_metadata_preview(
    owner: &Arc<SnapshotReviewOwner>,
    parent: &mut SnapshotReviewSession,
    selected_node_id: u64,
) -> Result<AiMetadataPreview, AiMetadataPreviewError> {
    if !parent.belongs_to(owner) {
        return Err(AiMetadataPreviewError::WrongReview);
    }
    if parent.is_released() {
        return Err(AiMetadataPreviewError::ReviewUnavailable);
    }

    let prepared_at = SystemTime::now();
    let monotonic_started_at = Instant::now();
    let parent_expires_at = parent
        .validate_and_expires_at(prepared_at)
        .map_err(map_review_error)?;
    let remaining_parent_lifetime = parent_expires_at
        .duration_since(prepared_at)
        .map_err(|_| AiMetadataPreviewError::InvalidClock)?;
    let lifetime = remaining_parent_lifetime.min(AI_METADATA_PREVIEW_LIFETIME);
    if lifetime.is_zero() {
        return Err(AiMetadataPreviewError::ReviewUnavailable);
    }
    let effective_expires_at = prepared_at
        .checked_add(lifetime)
        .ok_or(AiMetadataPreviewError::InvalidClock)?;
    let monotonic_deadline = monotonic_started_at
        .checked_add(lifetime)
        .ok_or(AiMetadataPreviewError::InvalidClock)?;

    let (document, coverage) = parent.ai_source(prepared_at).map_err(map_review_error)?;
    let shaped = shape_ai_metadata_preview_v1(document, &coverage, selected_node_id)
        .map_err(map_shape_error)?;
    let finished_at = SystemTime::now();
    if Instant::now() >= monotonic_deadline || finished_at >= effective_expires_at {
        return Err(AiMetadataPreviewError::ReviewUnavailable);
    }
    parent
        .validate_ai_source(&coverage, finished_at)
        .map_err(map_review_error)?;

    let disclosure = shaped.disclosure_values();
    if disclosure[0] != AI_METADATA_PRIVACY_POLICY_REVISION {
        return Err(AiMetadataPreviewError::InternalState);
    }
    let projection = public_projection(shaped.projection());
    let info = AiMetadataPreviewInfo {
        input_schema_version: AI_METADATA_INPUT_SCHEMA_VERSION,
        privacy_policy_revision: disclosure[0],
        prepared_at,
        effective_expires_at,
        input_digest_sha256: shaped.input_digest_sha256().to_owned(),
        encoded_input_json_utf8: shaped.share_encoded_input_json(),
        inspected_node_count: disclosure[1],
        included_direct_child_count: disclosure[2],
        excluded_sensitive_direct_child_count: disclosure[3],
        omitted_eligible_direct_child_count: disclosure[4],
        root_label: projection.root_label,
        total_logical_bytes: projection.total_logical_bytes,
        age_summary: projection.age_summary,
        children_complete: projection.children_complete,
        omitted_child_count: projection.omitted_child_count,
        omitted_logical_bytes: projection.omitted_logical_bytes,
        omitted_age_summary: projection.omitted_age_summary,
        children: projection.children,
    };
    Ok(AiMetadataPreview {
        owner: Arc::clone(owner),
        parent_session_identity: parent.session_identity(),
        parent_liveness: parent.plan_review_liveness(),
        source_scan_id: parent.scan_id().clone(),
        monotonic_deadline,
        info,
        _sealed_proof: shaped,
    })
}

struct PublicProjection {
    root_label: String,
    total_logical_bytes: u64,
    age_summary: AiMetadataPreviewAgeSummary,
    children_complete: bool,
    omitted_child_count: u64,
    omitted_logical_bytes: u64,
    omitted_age_summary: AiMetadataPreviewAgeSummary,
    children: Vec<AiMetadataPreviewChild>,
}

fn public_projection(source: &AiMetadataProjectionV1) -> PublicProjection {
    PublicProjection {
        root_label: source.root_label.clone(),
        total_logical_bytes: source.total_logical_bytes,
        age_summary: public_age(source.age_summary),
        children_complete: source.children_complete,
        omitted_child_count: source.omitted_child_count,
        omitted_logical_bytes: source.omitted_logical_bytes,
        omitted_age_summary: public_age(source.omitted_age_summary),
        children: source.children.iter().map(public_child).collect(),
    }
}

fn public_child(source: &AiMetadataChildV1) -> AiMetadataPreviewChild {
    AiMetadataPreviewChild {
        input_node_id: source.input_node_id.clone(),
        label: source.label.clone(),
        kind: match source.kind {
            AiMetadataNodeKindV1::Directory => AiMetadataPreviewNodeKind::Directory,
            AiMetadataNodeKindV1::File => AiMetadataPreviewNodeKind::File,
            AiMetadataNodeKindV1::Symlink => AiMetadataPreviewNodeKind::Symlink,
            AiMetadataNodeKindV1::Other => AiMetadataPreviewNodeKind::Other,
            AiMetadataNodeKindV1::Unavailable => AiMetadataPreviewNodeKind::Unavailable,
        },
        logical_bytes: source.logical_bytes,
        age_summary: public_age(source.age_summary),
    }
}

const fn public_age(source: AiMetadataAgeSummaryV1) -> AiMetadataPreviewAgeSummary {
    AiMetadataPreviewAgeSummary {
        within_7_days_logical_bytes: source.within_7_days_logical_bytes,
        days_8_to_30_logical_bytes: source.days_8_to_30_logical_bytes,
        days_31_to_90_logical_bytes: source.days_31_to_90_logical_bytes,
        older_than_90_days_logical_bytes: source.older_than_90_days_logical_bytes,
        unknown_age_logical_bytes: source.unknown_age_logical_bytes,
    }
}

const fn map_shape_error(error: AiMetadataShapeError) -> AiMetadataPreviewError {
    match error {
        AiMetadataShapeError::IncompleteCoverage => AiMetadataPreviewError::IncompleteCoverage,
        AiMetadataShapeError::SelectionUnavailable => AiMetadataPreviewError::SelectionUnavailable,
        AiMetadataShapeError::SelectionNotDirectory => {
            AiMetadataPreviewError::SelectionNotDirectory
        }
        AiMetadataShapeError::SensitiveSelection => AiMetadataPreviewError::SensitiveSelection,
        AiMetadataShapeError::UnsupportedObservation => {
            AiMetadataPreviewError::UnsupportedObservation
        }
        AiMetadataShapeError::InspectionLimitExceeded => AiMetadataPreviewError::BudgetExceeded,
        AiMetadataShapeError::InvalidAccounting | AiMetadataShapeError::ContractRejected => {
            AiMetadataPreviewError::InternalState
        }
    }
}

const fn map_review_error(error: SnapshotReviewError) -> AiMetadataPreviewError {
    match error {
        SnapshotReviewError::Closed => AiMetadataPreviewError::Closed,
        SnapshotReviewError::WrongParentReview => AiMetadataPreviewError::WrongReview,
        SnapshotReviewError::LeaseExpired
        | SnapshotReviewError::ScanNotFound
        | SnapshotReviewError::SnapshotUnavailable
        | SnapshotReviewError::ComparableSnapshotUnavailable
        | SnapshotReviewError::NodeNotFound
        | SnapshotReviewError::NodeNotDirectory => AiMetadataPreviewError::ReviewUnavailable,
        SnapshotReviewError::BudgetExceeded => AiMetadataPreviewError::BudgetExceeded,
        SnapshotReviewError::ReadOnlyStore
        | SnapshotReviewError::IncompatibleSchema
        | SnapshotReviewError::UnsafeStorage => AiMetadataPreviewError::UnsafeStorage,
        SnapshotReviewError::CorruptData | SnapshotReviewError::IncompatibleSnapshot => {
            AiMetadataPreviewError::CorruptData
        }
        SnapshotReviewError::Busy
        | SnapshotReviewError::Unavailable
        | SnapshotReviewError::OutcomeUnknown => AiMetadataPreviewError::Unavailable,
        SnapshotReviewError::InvalidPage
        | SnapshotReviewError::InvalidTreemapBudget
        | SnapshotReviewError::InvalidLargeFileRequest
        | SnapshotReviewError::InvalidICloudObservationSourceRequest
        | SnapshotReviewError::LiveTargetUnsupported
        | SnapshotReviewError::LivePathUnavailable
        | SnapshotReviewError::LivePathMissing
        | SnapshotReviewError::LivePathSymlink
        | SnapshotReviewError::LivePathCrossVolume
        | SnapshotReviewError::LivePathChanged
        | SnapshotReviewError::LivePathAccessDenied
        | SnapshotReviewError::InternalState => AiMetadataPreviewError::InternalState,
    }
}
