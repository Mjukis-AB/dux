//! Exact retained-review binding for path-free AI input disclosure.
//!
//! This module is the only engine facade allowed to shape or validate the
//! private AI contract. The separate sealed cache boundary may import frozen
//! revision constants, but creates no provider, request task, candidate, plan,
//! approval, or effect authority.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crate::ScanId;
use crate::ai::{
    AI_METADATA_INPUT_SCHEMA_VERSION, AI_METADATA_OUTPUT_SCHEMA_VERSION, AiMetadataAgeSummaryV1,
    AiMetadataChildV1, AiMetadataExplanationGroupV1, AiMetadataExplanationV1, AiMetadataNodeKindV1,
    AiMetadataOutputError, AiMetadataPreviewV1, AiMetadataProjectionV1, AiMetadataShapeError,
    CacheableAiMetadataExplanationV1, shape_ai_metadata_preview_v1,
};

use super::snapshot_review::{SnapshotReviewError, SnapshotReviewOwner, SnapshotReviewSession};

pub const AI_METADATA_PREVIEW_LIFETIME: Duration = Duration::from_secs(2 * 60);
pub const AI_METADATA_PRIVACY_POLICY_REVISION: u64 = 1;
pub const AI_EXPLANATION_ATTEMPT_LIFETIME: Duration = Duration::from_secs(60);
pub const AI_EXPLANATION_PROVIDER_BINDING_REVISION: u64 = 1;
pub const AI_EXPLANATION_PROVIDER: &str = "anthropic";
pub const AI_EXPLANATION_TRANSPORT: &str = "messages_v1";
pub const AI_EXPLANATION_ADAPTER_ID: &str = "anthropic-messages-v1";
pub const AI_EXPLANATION_MODEL: &str = "claude-sonnet-4-6";

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

/// Stable, bounded rejection categories for one Anthropic Messages v1 output.
/// No variant contains JSON locations, field names, source data, or paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AiExplanationAttemptError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the retained Explorer review belongs to a different engine")]
    WrongReview,
    #[error("the retained Explorer review or explanation attempt is unavailable")]
    ReviewUnavailable,
    #[error("the AI explanation attempt clock is invalid")]
    InvalidClock,
    #[error("the AI explanation output exceeds its bounded size")]
    OutputTooLarge,
    #[error("the AI explanation output is malformed")]
    MalformedOutput,
    #[error("the AI explanation output schema version is unsupported")]
    UnsupportedOutputVersion,
    #[error("the AI explanation task is unsupported")]
    UnsupportedTask,
    #[error("the AI explanation input digest is invalid")]
    InvalidInputDigest,
    #[error("the AI explanation belongs to a different input")]
    WrongInputDigest,
    #[error("the AI explanation exceeds a bounded collection limit")]
    BoundsExceeded,
    #[error("the AI explanation contains disallowed text")]
    InvalidText,
    #[error("the AI explanation contains duplicate values")]
    DuplicateValue,
    #[error("the AI explanation references an invalid node")]
    InvalidNodeReference,
    #[error("the AI explanation groups overlap")]
    OverlappingGroups,
    #[error("the AI explanation attempt reached an invalid internal state")]
    InternalState,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AiExplanationAttemptInfo {
    input_schema_version: u64,
    output_schema_version: u64,
    privacy_policy_revision: u64,
    provider_binding_revision: u64,
    prepared_at: SystemTime,
    effective_expires_at: SystemTime,
    provider: &'static str,
    transport: &'static str,
    model: &'static str,
    input_digest_sha256: String,
    encoded_input_json_utf8: Arc<[u8]>,
    source_scan_id: ScanId,
    selected_root_node_id: u64,
}

impl fmt::Debug for AiExplanationAttemptInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiExplanationAttemptInfo")
            .field("input_schema_version", &self.input_schema_version)
            .field("output_schema_version", &self.output_schema_version)
            .field("privacy_policy_revision", &self.privacy_policy_revision)
            .field("provider_binding_revision", &self.provider_binding_revision)
            .field("prepared_at", &self.prepared_at)
            .field("effective_expires_at", &self.effective_expires_at)
            .field("provider", &self.provider)
            .field("transport", &self.transport)
            .field("model", &self.model)
            .field("encoded_input_bytes", &self.encoded_input_json_utf8.len())
            .finish_non_exhaustive()
    }
}

impl AiExplanationAttemptInfo {
    pub const fn input_schema_version(&self) -> u64 {
        self.input_schema_version
    }

    pub const fn output_schema_version(&self) -> u64 {
        self.output_schema_version
    }

    pub const fn privacy_policy_revision(&self) -> u64 {
        self.privacy_policy_revision
    }

    pub const fn provider_binding_revision(&self) -> u64 {
        self.provider_binding_revision
    }

    pub const fn prepared_at(&self) -> SystemTime {
        self.prepared_at
    }

    pub const fn effective_expires_at(&self) -> SystemTime {
        self.effective_expires_at
    }

    pub const fn provider(&self) -> &'static str {
        self.provider
    }

    pub const fn transport(&self) -> &'static str {
        self.transport
    }

    pub const fn model(&self) -> &'static str {
        self.model
    }

    pub fn input_digest_sha256(&self) -> &str {
        &self.input_digest_sha256
    }

    pub fn encoded_input_json_utf8(&self) -> &[u8] {
        &self.encoded_input_json_utf8
    }

    pub fn source_scan_id(&self) -> &ScanId {
        &self.source_scan_id
    }

    pub const fn selected_root_node_id(&self) -> u64 {
        self.selected_root_node_id
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AiExplanationGroup {
    title: String,
    snapshot_node_ids: Vec<u64>,
    reason: String,
}

impl fmt::Debug for AiExplanationGroup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiExplanationGroup")
            .field("snapshot_node_count", &self.snapshot_node_ids.len())
            .finish_non_exhaustive()
    }
}

impl AiExplanationGroup {
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn snapshot_node_ids(&self) -> &[u64] {
        &self.snapshot_node_ids
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AiExplanationResult {
    input_schema_version: u64,
    output_schema_version: u64,
    privacy_policy_revision: u64,
    provider_binding_revision: u64,
    provider: &'static str,
    transport: &'static str,
    model: &'static str,
    input_digest_sha256: String,
    source_scan_id: ScanId,
    selected_root_node_id: u64,
    summary: String,
    labels: Vec<String>,
    groups: Vec<AiExplanationGroup>,
    questions: Vec<String>,
    uncertainties: Vec<String>,
    research_suggestions: Vec<String>,
}

impl fmt::Debug for AiExplanationResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiExplanationResult")
            .field("input_schema_version", &self.input_schema_version)
            .field("output_schema_version", &self.output_schema_version)
            .field("privacy_policy_revision", &self.privacy_policy_revision)
            .field("provider_binding_revision", &self.provider_binding_revision)
            .field("provider", &self.provider)
            .field("transport", &self.transport)
            .field("model", &self.model)
            .field("label_count", &self.labels.len())
            .field("group_count", &self.groups.len())
            .field("question_count", &self.questions.len())
            .field("uncertainty_count", &self.uncertainties.len())
            .field(
                "research_suggestion_count",
                &self.research_suggestions.len(),
            )
            .finish_non_exhaustive()
    }
}

impl AiExplanationResult {
    pub const fn input_schema_version(&self) -> u64 {
        self.input_schema_version
    }

    pub const fn output_schema_version(&self) -> u64 {
        self.output_schema_version
    }

    pub const fn privacy_policy_revision(&self) -> u64 {
        self.privacy_policy_revision
    }

    pub const fn provider_binding_revision(&self) -> u64 {
        self.provider_binding_revision
    }

    pub const fn provider(&self) -> &'static str {
        self.provider
    }

    pub const fn transport(&self) -> &'static str {
        self.transport
    }

    pub const fn model(&self) -> &'static str {
        self.model
    }

    pub fn input_digest_sha256(&self) -> &str {
        &self.input_digest_sha256
    }

    pub fn source_scan_id(&self) -> &ScanId {
        &self.source_scan_id
    }

    pub const fn selected_root_node_id(&self) -> u64 {
        self.selected_root_node_id
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }

    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    pub fn groups(&self) -> &[AiExplanationGroup] {
        &self.groups
    }

    pub fn questions(&self) -> &[String] {
        &self.questions
    }

    pub fn uncertainties(&self) -> &[String] {
        &self.uncertainties
    }

    pub fn research_suggestions(&self) -> &[String] {
        &self.research_suggestions
    }
}

/// Non-cloneable, engine-bound privacy proof for one exact Explorer review.
/// The FFI wrapper strongly retains the parent review for this object's life.
pub struct AiMetadataPreview {
    owner: Arc<SnapshotReviewOwner>,
    parent_session_identity: u64,
    parent_liveness: Arc<AtomicBool>,
    source_scan_id: ScanId,
    selected_root_node_id: u64,
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

    /// Revalidate one canonical cached document against this exact retained
    /// privacy proof. A row cannot construct a result on its own: the live
    /// review, preview clocks, input digest, request-local mapping, and output
    /// contract are checked again before and after parsing.
    pub(super) fn validate_cached_anthropic_messages_v1_output(
        &self,
        parent: &SnapshotReviewSession,
        canonical_output_json_utf8: &[u8],
    ) -> Result<AiExplanationResult, AiExplanationAttemptError> {
        let wall_before = SystemTime::now();
        let monotonic_before = Instant::now();
        self.info_at(parent, wall_before, monotonic_before)
            .map_err(map_preview_to_attempt_error)?;
        let validated = self
            ._sealed_proof
            .validate_explanation_output_v1(canonical_output_json_utf8)
            .map_err(map_output_error)?;
        self.info_at(parent, SystemTime::now(), Instant::now())
            .map_err(map_preview_to_attempt_error)?;
        if validated.input_digest_sha256 != self.info.input_digest_sha256 {
            return Err(AiExplanationAttemptError::InternalState);
        }
        let info = AiExplanationAttemptInfo {
            input_schema_version: self.info.input_schema_version,
            output_schema_version: AI_METADATA_OUTPUT_SCHEMA_VERSION,
            privacy_policy_revision: self.info.privacy_policy_revision,
            provider_binding_revision: AI_EXPLANATION_PROVIDER_BINDING_REVISION,
            prepared_at: self.info.prepared_at,
            effective_expires_at: self.info.effective_expires_at,
            provider: AI_EXPLANATION_PROVIDER,
            transport: AI_EXPLANATION_TRANSPORT,
            model: AI_EXPLANATION_MODEL,
            input_digest_sha256: self.info.input_digest_sha256.clone(),
            encoded_input_json_utf8: Arc::clone(&self.info.encoded_input_json_utf8),
            source_scan_id: self.source_scan_id.clone(),
            selected_root_node_id: self.selected_root_node_id,
        };
        Ok(public_explanation_result(
            &info,
            self.source_scan_id.clone(),
            self.selected_root_node_id,
            validated,
        ))
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

/// Move-only, engine-bound authorization to validate exactly one output for
/// the fixed Anthropic Messages v1 binding. It contains no transport secret,
/// URL, header, action, plan, persistence, or cleanup authority.
#[must_use = "an AI explanation attempt can validate exactly one provider output"]
pub struct AiExplanationAttempt {
    parent_liveness: Arc<AtomicBool>,
    source_scan_id: ScanId,
    selected_root_node_id: u64,
    monotonic_deadline: Instant,
    info: AiExplanationAttemptInfo,
    sealed_proof: AiMetadataPreviewV1,
}

pub(super) struct CacheableAiExplanationResult {
    pub(super) result: AiExplanationResult,
    pub(super) canonical_output_json_utf8: Box<[u8]>,
}

impl fmt::Debug for AiExplanationAttempt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiExplanationAttempt")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl AiExplanationAttempt {
    pub fn info(&self) -> Result<&AiExplanationAttemptInfo, AiExplanationAttemptError> {
        self.info_at(SystemTime::now(), Instant::now())
    }

    pub(super) fn info_at(
        &self,
        wall_now: SystemTime,
        monotonic_now: Instant,
    ) -> Result<&AiExplanationAttemptInfo, AiExplanationAttemptError> {
        self.validate_bound_state(wall_now, monotonic_now)?;
        Ok(&self.info)
    }

    /// Consume this attempt while validating a bounded untrusted response.
    /// Exact parent liveness and both clocks are checked immediately before
    /// and after parsing and sealed snapshot-ID projection.
    pub fn validate(
        self,
        output_json_utf8: &[u8],
    ) -> Result<AiExplanationResult, AiExplanationAttemptError> {
        self.validate_cacheable(output_json_utf8)
            .map(|validated| validated.result)
    }

    pub(super) fn validate_cacheable(
        self,
        output_json_utf8: &[u8],
    ) -> Result<CacheableAiExplanationResult, AiExplanationAttemptError> {
        let wall_before = SystemTime::now();
        let monotonic_before = Instant::now();
        self.validate_bound_state(wall_before, monotonic_before)?;
        self.validate_cacheable_at(
            output_json_utf8,
            wall_before,
            monotonic_before,
            SystemTime::now,
            Instant::now,
        )
    }

    fn validate_cacheable_at(
        self,
        output_json_utf8: &[u8],
        wall_before: SystemTime,
        monotonic_before: Instant,
        wall_after: impl FnOnce() -> SystemTime,
        monotonic_after: impl FnOnce() -> Instant,
    ) -> Result<CacheableAiExplanationResult, AiExplanationAttemptError> {
        self.validate_bound_state(wall_before, monotonic_before)?;
        let CacheableAiMetadataExplanationV1 {
            explanation: validated,
            canonical_output_json_utf8,
        } = self
            .sealed_proof
            .validate_cacheable_explanation_output_v1(output_json_utf8)
            .map_err(map_output_error)?;
        let wall_after = wall_after();
        let monotonic_after = monotonic_after();
        self.validate_bound_state(wall_after, monotonic_after)?;
        if validated.input_digest_sha256 != self.info.input_digest_sha256 {
            return Err(AiExplanationAttemptError::InternalState);
        }
        Ok(CacheableAiExplanationResult {
            result: public_explanation_result(
                &self.info,
                self.source_scan_id,
                self.selected_root_node_id,
                validated,
            ),
            canonical_output_json_utf8,
        })
    }

    fn validate_bound_state(
        &self,
        wall_now: SystemTime,
        monotonic_now: Instant,
    ) -> Result<(), AiExplanationAttemptError> {
        if !self.parent_liveness.load(Ordering::Acquire)
            || monotonic_now >= self.monotonic_deadline
            || wall_now >= self.info.effective_expires_at
        {
            return Err(AiExplanationAttemptError::ReviewUnavailable);
        }
        Ok(())
    }
}

pub(super) fn begin_anthropic_messages_v1_explanation(
    preview: AiMetadataPreview,
    parent: &SnapshotReviewSession,
) -> Result<AiExplanationAttempt, AiExplanationAttemptError> {
    begin_anthropic_messages_v1_explanation_at(
        preview,
        parent,
        SystemTime::now(),
        Instant::now(),
        SystemTime::now,
        Instant::now,
    )
}

fn begin_anthropic_messages_v1_explanation_at(
    preview: AiMetadataPreview,
    parent: &SnapshotReviewSession,
    prepared_at: SystemTime,
    monotonic_started_at: Instant,
    wall_finished: impl FnOnce() -> SystemTime,
    monotonic_finished: impl FnOnce() -> Instant,
) -> Result<AiExplanationAttempt, AiExplanationAttemptError> {
    preview
        .info_at(parent, prepared_at, monotonic_started_at)
        .map_err(map_preview_to_attempt_error)?;
    let wall_remaining = preview
        .info
        .effective_expires_at
        .duration_since(prepared_at)
        .map_err(|_| AiExplanationAttemptError::InvalidClock)?;
    let monotonic_remaining = preview
        .monotonic_deadline
        .checked_duration_since(monotonic_started_at)
        .ok_or(AiExplanationAttemptError::ReviewUnavailable)?;
    let lifetime = wall_remaining
        .min(monotonic_remaining)
        .min(AI_EXPLANATION_ATTEMPT_LIFETIME);
    if lifetime.is_zero() {
        return Err(AiExplanationAttemptError::ReviewUnavailable);
    }
    let effective_expires_at = prepared_at
        .checked_add(lifetime)
        .ok_or(AiExplanationAttemptError::InvalidClock)?;
    let monotonic_deadline = monotonic_started_at
        .checked_add(lifetime)
        .ok_or(AiExplanationAttemptError::InvalidClock)?;
    let wall_finished = wall_finished();
    let monotonic_finished = monotonic_finished();
    preview
        .validate_parent(parent, wall_finished)
        .map_err(map_preview_to_attempt_error)?;
    if monotonic_finished >= monotonic_deadline || wall_finished >= effective_expires_at {
        return Err(AiExplanationAttemptError::ReviewUnavailable);
    }

    let AiMetadataPreview {
        owner: _,
        parent_session_identity: _,
        parent_liveness,
        source_scan_id,
        selected_root_node_id,
        monotonic_deadline: _,
        info: preview_info,
        _sealed_proof: sealed_proof,
    } = preview;
    let info = AiExplanationAttemptInfo {
        input_schema_version: preview_info.input_schema_version,
        output_schema_version: AI_METADATA_OUTPUT_SCHEMA_VERSION,
        privacy_policy_revision: preview_info.privacy_policy_revision,
        provider_binding_revision: AI_EXPLANATION_PROVIDER_BINDING_REVISION,
        prepared_at,
        effective_expires_at,
        provider: AI_EXPLANATION_PROVIDER,
        transport: AI_EXPLANATION_TRANSPORT,
        model: AI_EXPLANATION_MODEL,
        input_digest_sha256: preview_info.input_digest_sha256,
        encoded_input_json_utf8: preview_info.encoded_input_json_utf8,
        source_scan_id: source_scan_id.clone(),
        selected_root_node_id,
    };
    Ok(AiExplanationAttempt {
        parent_liveness,
        source_scan_id,
        selected_root_node_id,
        monotonic_deadline,
        info,
        sealed_proof,
    })
}

fn public_explanation_result(
    info: &AiExplanationAttemptInfo,
    source_scan_id: ScanId,
    selected_root_node_id: u64,
    source: AiMetadataExplanationV1,
) -> AiExplanationResult {
    AiExplanationResult {
        input_schema_version: info.input_schema_version,
        output_schema_version: info.output_schema_version,
        privacy_policy_revision: info.privacy_policy_revision,
        provider_binding_revision: info.provider_binding_revision,
        provider: info.provider,
        transport: info.transport,
        model: info.model,
        input_digest_sha256: source.input_digest_sha256,
        source_scan_id,
        selected_root_node_id,
        summary: source.summary,
        labels: source.labels,
        groups: source.groups.into_iter().map(public_group).collect(),
        questions: source.questions,
        uncertainties: source.uncertainties,
        research_suggestions: source.research_suggestions,
    }
}

fn public_group(source: AiMetadataExplanationGroupV1) -> AiExplanationGroup {
    AiExplanationGroup {
        title: source.title,
        snapshot_node_ids: source.snapshot_node_ids,
        reason: source.reason,
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
        selected_root_node_id: selected_node_id,
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
        | SnapshotReviewError::InvalidDiskMapBudget
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

const fn map_preview_to_attempt_error(error: AiMetadataPreviewError) -> AiExplanationAttemptError {
    match error {
        AiMetadataPreviewError::Closed => AiExplanationAttemptError::Closed,
        AiMetadataPreviewError::WrongReview => AiExplanationAttemptError::WrongReview,
        AiMetadataPreviewError::InvalidClock => AiExplanationAttemptError::InvalidClock,
        AiMetadataPreviewError::ReviewUnavailable
        | AiMetadataPreviewError::IncompleteCoverage
        | AiMetadataPreviewError::SelectionUnavailable
        | AiMetadataPreviewError::SelectionNotDirectory
        | AiMetadataPreviewError::SensitiveSelection
        | AiMetadataPreviewError::UnsupportedObservation
        | AiMetadataPreviewError::BudgetExceeded
        | AiMetadataPreviewError::UnsafeStorage
        | AiMetadataPreviewError::CorruptData
        | AiMetadataPreviewError::Unavailable => AiExplanationAttemptError::ReviewUnavailable,
        AiMetadataPreviewError::InternalState => AiExplanationAttemptError::InternalState,
    }
}

const fn map_output_error(error: AiMetadataOutputError) -> AiExplanationAttemptError {
    match error {
        AiMetadataOutputError::DocumentTooLarge => AiExplanationAttemptError::OutputTooLarge,
        AiMetadataOutputError::MalformedJson => AiExplanationAttemptError::MalformedOutput,
        AiMetadataOutputError::UnsupportedSchemaVersion => {
            AiExplanationAttemptError::UnsupportedOutputVersion
        }
        AiMetadataOutputError::UnsupportedTask => AiExplanationAttemptError::UnsupportedTask,
        AiMetadataOutputError::InvalidInputDigest => AiExplanationAttemptError::InvalidInputDigest,
        AiMetadataOutputError::InputDigestMismatch => AiExplanationAttemptError::WrongInputDigest,
        AiMetadataOutputError::CollectionLimitExceeded => AiExplanationAttemptError::BoundsExceeded,
        AiMetadataOutputError::InvalidText => AiExplanationAttemptError::InvalidText,
        AiMetadataOutputError::DuplicateValue => AiExplanationAttemptError::DuplicateValue,
        AiMetadataOutputError::InvalidNodeReference => {
            AiExplanationAttemptError::InvalidNodeReference
        }
        AiMetadataOutputError::OverlappingGroups => AiExplanationAttemptError::OverlappingGroups,
        AiMetadataOutputError::RequestLocalIdIncluded => AiExplanationAttemptError::InvalidText,
        AiMetadataOutputError::InvalidMapping => AiExplanationAttemptError::InternalState,
    }
}
