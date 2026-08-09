//! Sealed engine boundary for revision-exact AI explanation caching.
//!
//! Cache identity and canonical provider output remain private to Rust. A hit
//! is useful only after it has been revalidated against the live retained
//! review and remapped to fresh request-local snapshot node IDs.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use crate::ai::AI_METADATA_INPUT_DIGEST_REVISION;
use crate::persistence::{
    AiInsightCacheBinding, AiInsightCacheClearResult as StoredAiInsightCacheClearResult,
    AiInsightCacheClearStoreError, HistoryErrorKind, NewAiInsightCacheRecord,
    PreparedAiInsightCacheClear, StoreCoordinator,
};

use super::ai_metadata_preview::{
    AI_EXPLANATION_ADAPTER_ID, AI_EXPLANATION_MODEL, AI_EXPLANATION_PROVIDER,
    AI_EXPLANATION_PROVIDER_BINDING_REVISION, AiExplanationAttempt, AiExplanationAttemptError,
    AiExplanationAttemptInfo, AiExplanationResult, AiMetadataPreviewError, AiMetadataPreviewInfo,
};

pub const AI_INSIGHT_CACHE_CLEAR_PREVIEW_LIFETIME: Duration = Duration::from_secs(2 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiCachedExplanation {
    result: AiExplanationResult,
    created_at: SystemTime,
    expires_at: SystemTime,
}

impl AiCachedExplanation {
    pub(super) const fn new(
        result: AiExplanationResult,
        created_at: SystemTime,
        expires_at: SystemTime,
    ) -> Self {
        Self {
            result,
            created_at,
            expires_at,
        }
    }

    pub const fn result(&self) -> &AiExplanationResult {
        &self.result
    }

    pub const fn created_at(&self) -> SystemTime {
        self.created_at
    }

    pub const fn expires_at(&self) -> SystemTime {
        self.expires_at
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AiInsightCacheError {
    #[error("the engine session is closed")]
    Closed,
    #[error("the retained Explorer review belongs to a different engine")]
    WrongReview,
    #[error("the retained Explorer review or AI metadata preview is unavailable")]
    ReviewUnavailable,
    #[error("the AI cache clock is invalid")]
    InvalidClock,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("AI-cache validation exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("the AI cache is corrupt")]
    CorruptData,
    #[error("the AI cache is unavailable")]
    Unavailable,
    #[error("the AI cache reached an invalid internal state")]
    InternalState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiInsightCacheClearPreviewInfo {
    record_count: u32,
    logical_content_bytes: u64,
    expired_record_count: u32,
    expired_logical_content_bytes: u64,
    prepared_at: SystemTime,
    expires_at: SystemTime,
}

impl AiInsightCacheClearPreviewInfo {
    pub const fn record_count(&self) -> u32 {
        self.record_count
    }

    pub const fn logical_content_bytes(&self) -> u64 {
        self.logical_content_bytes
    }

    pub const fn expired_record_count(&self) -> u32 {
        self.expired_record_count
    }

    pub const fn expired_logical_content_bytes(&self) -> u64 {
        self.expired_logical_content_bytes
    }

    pub const fn prepared_at(&self) -> SystemTime {
        self.prepared_at
    }

    pub const fn expires_at(&self) -> SystemTime {
        self.expires_at
    }
}

pub struct AiInsightCacheClearPreview {
    owner: Weak<StoreCoordinator>,
    prepared: PreparedAiInsightCacheClear,
    info: AiInsightCacheClearPreviewInfo,
    monotonic_expires_at: Instant,
}

impl std::fmt::Debug for AiInsightCacheClearPreview {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AiInsightCacheClearPreview")
            .field("info", &self.info)
            .field("owner_live", &self.owner.strong_count())
            .finish_non_exhaustive()
    }
}

impl AiInsightCacheClearPreview {
    pub(super) fn new(
        store: &Arc<StoreCoordinator>,
        prepared: PreparedAiInsightCacheClear,
        monotonic_now: Instant,
    ) -> Option<Self> {
        let prepared_at = prepared.prepared_at();
        let expires_at = prepared_at.checked_add(AI_INSIGHT_CACHE_CLEAR_PREVIEW_LIFETIME)?;
        let monotonic_expires_at =
            monotonic_now.checked_add(AI_INSIGHT_CACHE_CLEAR_PREVIEW_LIFETIME)?;
        if prepared.record_count() == 0
            || prepared.expired_record_count() > prepared.record_count()
            || prepared.expired_logical_content_bytes() > prepared.logical_content_bytes()
            || prepared_at >= expires_at
        {
            return None;
        }
        let info = AiInsightCacheClearPreviewInfo {
            record_count: prepared.record_count(),
            logical_content_bytes: prepared.logical_content_bytes(),
            expired_record_count: prepared.expired_record_count(),
            expired_logical_content_bytes: prepared.expired_logical_content_bytes(),
            prepared_at,
            expires_at,
        };
        Some(Self {
            owner: Arc::downgrade(store),
            prepared,
            info,
            monotonic_expires_at,
        })
    }

    pub fn info(&self) -> Result<AiInsightCacheClearPreviewInfo, AiInsightCacheClearError> {
        self.info_at(Instant::now())
    }

    pub(super) fn info_at(
        &self,
        now: Instant,
    ) -> Result<AiInsightCacheClearPreviewInfo, AiInsightCacheClearError> {
        if now >= self.monotonic_expires_at {
            return Err(AiInsightCacheClearError::PreviewExpired);
        }
        Ok(self.info)
    }

    pub(super) fn belongs_to(&self, store: &Arc<StoreCoordinator>) -> bool {
        self.owner
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, store))
    }

    pub(super) fn into_prepared(
        self,
        now: Instant,
    ) -> Result<PreparedAiInsightCacheClear, AiInsightCacheClearError> {
        if now >= self.monotonic_expires_at {
            return Err(AiInsightCacheClearError::PreviewExpired);
        }
        Ok(self.prepared)
    }

    #[cfg(test)]
    pub(super) const fn monotonic_expires_at_for_test(&self) -> Instant {
        self.monotonic_expires_at
    }

    pub fn release(self) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiInsightCacheClearResult {
    cleared_record_count: u32,
    cleared_logical_content_bytes: u64,
}

impl AiInsightCacheClearResult {
    pub const fn cleared_record_count(&self) -> u32 {
        self.cleared_record_count
    }

    pub const fn cleared_logical_content_bytes(&self) -> u64 {
        self.cleared_logical_content_bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AiInsightCacheClearError {
    #[error("the engine session is closed")]
    Closed,
    #[error("there are no cached AI explanations to clear")]
    NothingToClear,
    #[error("the AI cache is read-only")]
    ReadOnlyStore,
    #[error("the durable store schema is newer than this engine")]
    IncompatibleSchema,
    #[error("the AI cache changed after the clear preview")]
    ChangedSincePreview,
    #[error("the AI-cache clear preview expired")]
    PreviewExpired,
    #[error("the AI-cache clear preview belongs to another engine")]
    WrongEngine,
    #[error("the durable store is busy")]
    Busy,
    #[error("the durable store is unsafe")]
    UnsafeStorage,
    #[error("AI-cache clearing exceeded its fixed resource budget")]
    BudgetExceeded,
    #[error("the AI cache is corrupt")]
    CorruptData,
    #[error("the result of clearing the AI cache is unknown")]
    OutcomeUnknown,
    #[error("the AI cache is unavailable")]
    Unavailable,
    #[error("AI-cache clearing reached an invalid internal state")]
    InternalState,
}

pub(super) fn cache_binding_from_preview(
    info: &AiMetadataPreviewInfo,
) -> Result<AiInsightCacheBinding, AiInsightCacheError> {
    AiInsightCacheBinding::new(
        decode_sha256(info.input_digest_sha256())?,
        info.privacy_policy_revision(),
        info.input_schema_version(),
        AI_METADATA_INPUT_DIGEST_REVISION,
        crate::ai::AI_METADATA_OUTPUT_SCHEMA_VERSION,
        AI_EXPLANATION_PROVIDER,
        AI_EXPLANATION_ADAPTER_ID,
        AI_EXPLANATION_PROVIDER_BINDING_REVISION,
        AI_EXPLANATION_MODEL,
    )
    .map_err(|_| AiInsightCacheError::InternalState)
}

pub(super) fn cache_binding_from_attempt(
    info: &AiExplanationAttemptInfo,
) -> Result<AiInsightCacheBinding, AiExplanationAttemptError> {
    AiInsightCacheBinding::new(
        decode_sha256(info.input_digest_sha256())
            .map_err(|_| AiExplanationAttemptError::InternalState)?,
        info.privacy_policy_revision(),
        info.input_schema_version(),
        AI_METADATA_INPUT_DIGEST_REVISION,
        info.output_schema_version(),
        info.provider(),
        AI_EXPLANATION_ADAPTER_ID,
        info.provider_binding_revision(),
        info.model(),
    )
    .map_err(|_| AiExplanationAttemptError::InternalState)
}

pub(super) fn validate_and_prepare_cache_record(
    attempt: AiExplanationAttempt,
    output_json_utf8: &[u8],
    created_at: SystemTime,
) -> Result<(AiExplanationResult, Option<NewAiInsightCacheRecord>), AiExplanationAttemptError> {
    let binding = cache_binding_from_attempt(attempt.info()?)?;
    let cacheable = attempt.validate_cacheable(output_json_utf8)?;
    // Cache creation is deliberately best effort after validation. Even an
    // invalid wall clock or unavailable writable store cannot turn a safe
    // provider result into an AI failure or trigger another remote request.
    let record =
        NewAiInsightCacheRecord::new(binding, cacheable.canonical_output_json_utf8, created_at)
            .ok();
    Ok((cacheable.result, record))
}

pub(super) fn public_clear_result(
    stored: StoredAiInsightCacheClearResult,
    expected: AiInsightCacheClearPreviewInfo,
) -> Result<AiInsightCacheClearResult, AiInsightCacheClearError> {
    if stored.records_removed != expected.record_count
        || stored.logical_content_bytes_removed != expected.logical_content_bytes
        || stored.expired_records_removed != expected.expired_record_count
        || stored.expired_logical_content_bytes_removed != expected.expired_logical_content_bytes
    {
        return Err(AiInsightCacheClearError::OutcomeUnknown);
    }
    Ok(AiInsightCacheClearResult {
        cleared_record_count: stored.records_removed,
        cleared_logical_content_bytes: stored.logical_content_bytes_removed,
    })
}

pub(super) const fn map_store_error(error: HistoryErrorKind) -> AiInsightCacheError {
    match error {
        HistoryErrorKind::IncompatibleSchema => AiInsightCacheError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => AiInsightCacheError::BudgetExceeded,
        HistoryErrorKind::Busy => AiInsightCacheError::Busy,
        HistoryErrorKind::UnsafeStorage => AiInsightCacheError::UnsafeStorage,
        HistoryErrorKind::CorruptData => AiInsightCacheError::CorruptData,
        HistoryErrorKind::DatabaseUnavailable | HistoryErrorKind::OutcomeUnknown => {
            AiInsightCacheError::Unavailable
        }
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::InternalState => AiInsightCacheError::InternalState,
    }
}

pub(super) const fn map_cached_validation_error(
    error: AiExplanationAttemptError,
) -> AiInsightCacheError {
    match error {
        AiExplanationAttemptError::Closed => AiInsightCacheError::Closed,
        AiExplanationAttemptError::WrongReview => AiInsightCacheError::WrongReview,
        AiExplanationAttemptError::ReviewUnavailable => AiInsightCacheError::ReviewUnavailable,
        AiExplanationAttemptError::InvalidClock => AiInsightCacheError::InvalidClock,
        AiExplanationAttemptError::OutputTooLarge
        | AiExplanationAttemptError::MalformedOutput
        | AiExplanationAttemptError::UnsupportedOutputVersion
        | AiExplanationAttemptError::UnsupportedTask
        | AiExplanationAttemptError::InvalidInputDigest
        | AiExplanationAttemptError::WrongInputDigest
        | AiExplanationAttemptError::BoundsExceeded
        | AiExplanationAttemptError::InvalidText
        | AiExplanationAttemptError::DuplicateValue
        | AiExplanationAttemptError::InvalidNodeReference
        | AiExplanationAttemptError::OverlappingGroups => AiInsightCacheError::CorruptData,
        AiExplanationAttemptError::InternalState => AiInsightCacheError::InternalState,
    }
}

pub(super) const fn map_preview_error(error: AiMetadataPreviewError) -> AiInsightCacheError {
    match error {
        AiMetadataPreviewError::Closed => AiInsightCacheError::Closed,
        AiMetadataPreviewError::WrongReview => AiInsightCacheError::WrongReview,
        AiMetadataPreviewError::ReviewUnavailable => AiInsightCacheError::ReviewUnavailable,
        AiMetadataPreviewError::InvalidClock => AiInsightCacheError::InvalidClock,
        AiMetadataPreviewError::UnsafeStorage => AiInsightCacheError::UnsafeStorage,
        AiMetadataPreviewError::CorruptData => AiInsightCacheError::CorruptData,
        AiMetadataPreviewError::BudgetExceeded => AiInsightCacheError::BudgetExceeded,
        AiMetadataPreviewError::Unavailable => AiInsightCacheError::Unavailable,
        AiMetadataPreviewError::IncompleteCoverage
        | AiMetadataPreviewError::SelectionUnavailable
        | AiMetadataPreviewError::SelectionNotDirectory
        | AiMetadataPreviewError::SensitiveSelection
        | AiMetadataPreviewError::UnsupportedObservation
        | AiMetadataPreviewError::InternalState => AiInsightCacheError::InternalState,
    }
}

pub(super) const fn map_clear_store_error(
    error: AiInsightCacheClearStoreError,
) -> AiInsightCacheClearError {
    match error {
        AiInsightCacheClearStoreError::ChangedSincePreview => {
            AiInsightCacheClearError::ChangedSincePreview
        }
        AiInsightCacheClearStoreError::History(history) => match history.kind {
            HistoryErrorKind::IncompatibleSchema => AiInsightCacheClearError::IncompatibleSchema,
            HistoryErrorKind::QueryLimitExceeded => AiInsightCacheClearError::BudgetExceeded,
            HistoryErrorKind::Busy => AiInsightCacheClearError::Busy,
            HistoryErrorKind::UnsafeStorage => AiInsightCacheClearError::UnsafeStorage,
            HistoryErrorKind::CorruptData => AiInsightCacheClearError::CorruptData,
            HistoryErrorKind::OutcomeUnknown => AiInsightCacheClearError::OutcomeUnknown,
            HistoryErrorKind::DatabaseUnavailable => AiInsightCacheClearError::Unavailable,
            HistoryErrorKind::InvalidInput
            | HistoryErrorKind::AlreadyExists
            | HistoryErrorKind::NotFound
            | HistoryErrorKind::InvalidTransition
            | HistoryErrorKind::InternalState => AiInsightCacheClearError::InternalState,
        },
    }
}

pub(super) const fn map_prepare_clear_error(error: HistoryErrorKind) -> AiInsightCacheClearError {
    match error {
        HistoryErrorKind::IncompatibleSchema => AiInsightCacheClearError::IncompatibleSchema,
        HistoryErrorKind::QueryLimitExceeded => AiInsightCacheClearError::BudgetExceeded,
        HistoryErrorKind::Busy => AiInsightCacheClearError::Busy,
        HistoryErrorKind::UnsafeStorage => AiInsightCacheClearError::UnsafeStorage,
        HistoryErrorKind::CorruptData => AiInsightCacheClearError::CorruptData,
        HistoryErrorKind::OutcomeUnknown => AiInsightCacheClearError::OutcomeUnknown,
        HistoryErrorKind::DatabaseUnavailable => AiInsightCacheClearError::Unavailable,
        HistoryErrorKind::InvalidInput
        | HistoryErrorKind::AlreadyExists
        | HistoryErrorKind::NotFound
        | HistoryErrorKind::InvalidTransition
        | HistoryErrorKind::InternalState => AiInsightCacheClearError::InternalState,
    }
}

fn decode_sha256(value: &str) -> Result<[u8; 32], AiInsightCacheError> {
    let encoded = value.as_bytes();
    if encoded.len() != 64 {
        return Err(AiInsightCacheError::InternalState);
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in encoded.chunks_exact(2).enumerate() {
        decoded[index] = hex_nibble(pair[0])?
            .checked_mul(16)
            .and_then(|high| high.checked_add(hex_nibble(pair[1]).ok()?))
            .ok_or(AiInsightCacheError::InternalState)?;
    }
    Ok(decoded)
}

const fn hex_nibble(value: u8) -> Result<u8, AiInsightCacheError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(AiInsightCacheError::InternalState),
    }
}
