//! Sealed, revision-exact persistence for canonical validated AI insights.
//!
//! The table is a cache only. Exact binding fields are always supplied as one
//! typed value, raw provider responses are never accepted, and neither reads
//! nor explicit clearing expose a row selector or filesystem path.

use std::fmt::Write as _;
use std::time::{Duration, Instant, SystemTime};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};

use super::history::{
    HistoryError, HistoryErrorKind, map_query_sql_error, map_write_sql_error,
    system_time_to_unix_ms, unix_ms_to_system_time,
};

pub(crate) const AI_INSIGHT_CACHE_TTL: Duration = Duration::from_secs(30 * 86_400);
pub(crate) const MAX_AI_INSIGHT_PAYLOAD_BYTES: usize = 64 * 1024;
const MAX_INSIGHT_ID_BYTES: usize = 128;
const MAX_PROVIDER_BYTES: usize = 128;
const MAX_ADAPTER_ID_BYTES: usize = 128;
const MAX_MODEL_REVISION_BYTES: usize = 256;
const CLEAR_PAGE_ROWS: usize = 64;
const MAX_CLEAR_RECORDS: usize = 1_024;
const PROGRESS_OP_INTERVAL: i32 = 1_000;
const MAX_PROGRESS_CALLBACKS: u64 = 500_000;
const MAX_ELAPSED: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AiInsightCacheBinding {
    input_digest: [u8; 32],
    privacy_policy_revision: u64,
    input_schema_version: u64,
    input_digest_revision: u64,
    output_schema_version: u64,
    provider: String,
    adapter_id: String,
    adapter_revision: u64,
    model_revision: String,
}

impl AiInsightCacheBinding {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        input_digest: [u8; 32],
        privacy_policy_revision: u64,
        input_schema_version: u64,
        input_digest_revision: u64,
        output_schema_version: u64,
        provider: impl Into<String>,
        adapter_id: impl Into<String>,
        adapter_revision: u64,
        model_revision: impl Into<String>,
    ) -> Result<Self, HistoryError> {
        let provider = provider.into();
        let adapter_id = adapter_id.into();
        let model_revision = model_revision.into();
        if !valid_positive_revision(privacy_policy_revision)
            || !valid_positive_revision(input_schema_version)
            || !valid_positive_revision(input_digest_revision)
            || !valid_positive_revision(output_schema_version)
            || !valid_positive_revision(adapter_revision)
            || !valid_fixed_text(&provider, MAX_PROVIDER_BYTES)
            || !valid_fixed_text(&adapter_id, MAX_ADAPTER_ID_BYTES)
            || !valid_fixed_text(&model_revision, MAX_MODEL_REVISION_BYTES)
        {
            return Err(invalid());
        }
        Ok(Self {
            input_digest,
            privacy_policy_revision,
            input_schema_version,
            input_digest_revision,
            output_schema_version,
            provider,
            adapter_id,
            adapter_revision,
            model_revision,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NewAiInsightCacheRecord {
    binding: AiInsightCacheBinding,
    canonical_payload: Box<[u8]>,
    created_at: SystemTime,
    expires_at: SystemTime,
    created_at_unix_ms: i64,
    expires_at_unix_ms: i64,
    insight_id: String,
}

impl NewAiInsightCacheRecord {
    /// Admit only the canonical, already validated contract representation.
    /// The reviewed v1 cache policy owns the exact 30-day TTL.
    pub(crate) fn new(
        binding: AiInsightCacheBinding,
        canonical_payload: Box<[u8]>,
        created_at: SystemTime,
    ) -> Result<Self, HistoryError> {
        if canonical_payload.is_empty() || canonical_payload.len() > MAX_AI_INSIGHT_PAYLOAD_BYTES {
            return Err(invalid());
        }
        let created_at_unix_ms =
            system_time_to_unix_ms(created_at, HistoryErrorKind::InvalidInput)?;
        let ttl_ms = i64::try_from(AI_INSIGHT_CACHE_TTL.as_millis()).map_err(|_| invalid())?;
        let expires_at_unix_ms = created_at_unix_ms.checked_add(ttl_ms).ok_or_else(invalid)?;
        let created_at = unix_ms_to_system_time(created_at_unix_ms).map_err(|_| invalid())?;
        let expires_at = unix_ms_to_system_time(expires_at_unix_ms).map_err(|_| invalid())?;
        let insight_id = generated_insight_id(
            &binding,
            &canonical_payload,
            created_at_unix_ms,
            expires_at_unix_ms,
        );
        Ok(Self {
            binding,
            canonical_payload,
            created_at,
            expires_at,
            created_at_unix_ms,
            expires_at_unix_ms,
            insight_id,
        })
    }

    pub(crate) fn canonical_payload(&self) -> &[u8] {
        &self.canonical_payload
    }

    pub(crate) const fn binding(&self) -> &AiInsightCacheBinding {
        &self.binding
    }

    pub(crate) const fn created_at(&self) -> SystemTime {
        self.created_at
    }

    pub(crate) const fn expires_at(&self) -> SystemTime {
        self.expires_at
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredAiInsightCacheRecord {
    binding: AiInsightCacheBinding,
    canonical_payload: Box<[u8]>,
    created_at: SystemTime,
    expires_at: SystemTime,
    created_at_unix_ms: i64,
    expires_at_unix_ms: i64,
    insight_id: String,
}

impl StoredAiInsightCacheRecord {
    pub(crate) fn canonical_payload(&self) -> &[u8] {
        &self.canonical_payload
    }

    pub(crate) const fn created_at(&self) -> SystemTime {
        self.created_at
    }

    pub(crate) const fn expires_at(&self) -> SystemTime {
        self.expires_at
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AiInsightCacheInsertOutcome {
    Inserted(StoredAiInsightCacheRecord),
    ExistingUnexpired(StoredAiInsightCacheRecord),
}

impl AiInsightCacheInsertOutcome {
    pub(super) const fn record(&self) -> &StoredAiInsightCacheRecord {
        match self {
            Self::Inserted(record) | Self::ExistingUnexpired(record) => record,
        }
    }
}

/// Path-free proof of one complete, validated AI-cache population.
///
/// The engine wraps this move-only value in its own two-minute, engine-bound
/// preview. Persistence accepts no row ID, provider, or other selector when it
/// is consumed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedAiInsightCacheClear {
    witness: [u8; 32],
    record_count: u32,
    logical_content_bytes: u64,
    expired_record_count: u32,
    expired_logical_content_bytes: u64,
    prepared_at: SystemTime,
}

impl PreparedAiInsightCacheClear {
    pub(crate) const fn record_count(&self) -> u32 {
        self.record_count
    }

    pub(crate) const fn logical_content_bytes(&self) -> u64 {
        self.logical_content_bytes
    }

    pub(crate) const fn expired_record_count(&self) -> u32 {
        self.expired_record_count
    }

    pub(crate) const fn expired_logical_content_bytes(&self) -> u64 {
        self.expired_logical_content_bytes
    }

    pub(crate) const fn prepared_at(&self) -> SystemTime {
        self.prepared_at
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AiInsightCacheClearResult {
    pub(crate) records_removed: u32,
    pub(crate) logical_content_bytes_removed: u64,
    pub(crate) expired_records_removed: u32,
    pub(crate) expired_logical_content_bytes_removed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AiInsightCacheClearStoreError {
    ChangedSincePreview,
    History(HistoryError),
}

impl From<HistoryError> for AiInsightCacheClearStoreError {
    fn from(value: HistoryError) -> Self {
        Self::History(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AiInsightCacheClearReconciliation {
    Applied,
    NotApplied,
    Ambiguous,
}

pub(super) fn load_ai_insight_cache(
    connection: &Connection,
    binding: &AiInsightCacheBinding,
    observed_at: SystemTime,
) -> Result<Option<StoredAiInsightCacheRecord>, HistoryError> {
    let observed_at_unix_ms = system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
    Ok(load_exact_row(connection, binding)?
        .filter(|record| record.expires_at_unix_ms > observed_at_unix_ms))
}

pub(super) fn insert_ai_insight_cache(
    transaction: &Transaction<'_>,
    record: &NewAiInsightCacheRecord,
) -> Result<AiInsightCacheInsertOutcome, HistoryError> {
    if let Some(existing) = load_exact_row(transaction, &record.binding)? {
        if existing.expires_at_unix_ms > record.created_at_unix_ms {
            return Ok(AiInsightCacheInsertOutcome::ExistingUnexpired(existing));
        }
        let removed = transaction
            .execute(
                "DELETE FROM ai_insights
                 WHERE insight_id = ?1
                   AND input_digest = ?2
                   AND privacy_policy_revision = ?3
                   AND input_schema_version = ?4
                   AND input_digest_revision = ?5
                   AND output_schema_version = ?6
                   AND provider = ?7
                   AND adapter_id = ?8
                   AND adapter_revision = ?9
                   AND model_revision = ?10
                   AND 'DUX-DESTRUCTIVE: allow=ai-insight-cache-replace-expired -- replace only the exact revision-bound identity row after proving it expired at the new canonical record time' != ''
                   AND expires_at_unix_ms <= ?11",
                params![
                    existing.insight_id,
                    record.binding.input_digest.as_slice(),
                    to_i64(record.binding.privacy_policy_revision)?,
                    to_i64(record.binding.input_schema_version)?,
                    to_i64(record.binding.input_digest_revision)?,
                    to_i64(record.binding.output_schema_version)?,
                    record.binding.provider,
                    record.binding.adapter_id,
                    to_i64(record.binding.adapter_revision)?,
                    record.binding.model_revision,
                    record.created_at_unix_ms,
                ],
            )
            .map_err(map_write_sql_error)?;
        if removed != 1 {
            return Err(corrupt());
        }
    }

    transaction
        .execute(
            "INSERT INTO ai_insights (
                 insight_id, input_digest, privacy_policy_revision,
                 input_schema_version, input_digest_revision,
                 output_schema_version, provider, adapter_id,
                 adapter_revision, model_revision, output_payload,
                 created_at_unix_ms, expires_at_unix_ms
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13
             )",
            params![
                record.insight_id,
                record.binding.input_digest.as_slice(),
                to_i64(record.binding.privacy_policy_revision)?,
                to_i64(record.binding.input_schema_version)?,
                to_i64(record.binding.input_digest_revision)?,
                to_i64(record.binding.output_schema_version)?,
                record.binding.provider,
                record.binding.adapter_id,
                to_i64(record.binding.adapter_revision)?,
                record.binding.model_revision,
                record.canonical_payload.as_ref(),
                record.created_at_unix_ms,
                record.expires_at_unix_ms,
            ],
        )
        .map_err(map_insert_error)?;
    Ok(AiInsightCacheInsertOutcome::Inserted(stored_from_new(
        record,
    )))
}

pub(super) fn prepare_ai_insight_cache_clear(
    connection: &Connection,
    observed_at: SystemTime,
) -> Result<Option<PreparedAiInsightCacheClear>, HistoryError> {
    let observed_at_unix_ms = system_time_to_unix_ms(observed_at, HistoryErrorKind::InvalidInput)?;
    let prepared_at = unix_ms_to_system_time(observed_at_unix_ms).map_err(|_| invalid())?;
    let population = inspect_population(connection, observed_at_unix_ms)?;
    if population.record_count == 0 {
        return Ok(None);
    }
    Ok(Some(PreparedAiInsightCacheClear {
        witness: population.witness,
        record_count: population.record_count,
        logical_content_bytes: population.logical_content_bytes,
        expired_record_count: population.expired_record_count,
        expired_logical_content_bytes: population.expired_logical_content_bytes,
        prepared_at,
    }))
}

pub(super) fn apply_ai_insight_cache_clear(
    transaction: &Transaction<'_>,
    prepared: &PreparedAiInsightCacheClear,
) -> Result<AiInsightCacheClearResult, AiInsightCacheClearStoreError> {
    let observed_at_unix_ms =
        system_time_to_unix_ms(prepared.prepared_at, HistoryErrorKind::CorruptData)?;
    let current = inspect_population(transaction, observed_at_unix_ms)?;
    if !current.matches(prepared) {
        return Err(AiInsightCacheClearStoreError::ChangedSincePreview);
    }
    let mut authorizer = AiInsightCacheClearAuthorizerGuard::install(transaction)?;
    let removed = transaction
        .execute(
            "DELETE FROM ai_insights
             WHERE 'DUX-DESTRUCTIVE: allow=ai-insight-cache-explicit-clear -- explicit path-free clear consumes an exact complete AI-only population witness under the narrow SQLite authorizer' != ''",
            [],
        )
        .map_err(map_clear_write_error);
    authorizer.remove()?;
    let removed = u32::try_from(removed?).map_err(|_| corrupt())?;
    if removed != prepared.record_count {
        return Err(corrupt().into());
    }
    Ok(AiInsightCacheClearResult {
        records_removed: prepared.record_count,
        logical_content_bytes_removed: prepared.logical_content_bytes,
        expired_records_removed: prepared.expired_record_count,
        expired_logical_content_bytes_removed: prepared.expired_logical_content_bytes,
    })
}

pub(super) fn reconcile_ai_insight_cache_clear(
    connection: &Connection,
    prepared: &PreparedAiInsightCacheClear,
) -> Result<AiInsightCacheClearReconciliation, HistoryError> {
    let observed_at_unix_ms =
        system_time_to_unix_ms(prepared.prepared_at, HistoryErrorKind::CorruptData)?;
    let current = inspect_population(connection, observed_at_unix_ms)?;
    if current.record_count == 0 {
        Ok(AiInsightCacheClearReconciliation::Applied)
    } else if current.matches(prepared) {
        Ok(AiInsightCacheClearReconciliation::NotApplied)
    } else {
        Ok(AiInsightCacheClearReconciliation::Ambiguous)
    }
}

fn load_exact_row(
    connection: &Connection,
    binding: &AiInsightCacheBinding,
) -> Result<Option<StoredAiInsightCacheRecord>, HistoryError> {
    connection
        .query_row(
            "SELECT insight_id, input_digest, privacy_policy_revision,
                    input_schema_version, input_digest_revision,
                    output_schema_version, provider, adapter_id,
                    adapter_revision, model_revision, output_payload,
                    created_at_unix_ms, expires_at_unix_ms
             FROM ai_insights INDEXED BY ai_insights_by_identity
             WHERE input_digest = ?1
               AND privacy_policy_revision = ?2
               AND input_schema_version = ?3
               AND input_digest_revision = ?4
               AND output_schema_version = ?5
               AND provider = ?6
               AND adapter_id = ?7
               AND adapter_revision = ?8
               AND model_revision = ?9",
            params![
                binding.input_digest.as_slice(),
                to_i64(binding.privacy_policy_revision)?,
                to_i64(binding.input_schema_version)?,
                to_i64(binding.input_digest_revision)?,
                to_i64(binding.output_schema_version)?,
                binding.provider,
                binding.adapter_id,
                to_i64(binding.adapter_revision)?,
                binding.model_revision,
            ],
            decode_row,
        )
        .optional()
        .map_err(map_query_sql_error)
        .and_then(|record| {
            if record
                .as_ref()
                .is_some_and(|record| record.binding != *binding)
            {
                Err(corrupt())
            } else {
                Ok(record)
            }
        })
}

fn decode_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredAiInsightCacheRecord> {
    let insight_id: String = row.get(0)?;
    let input_digest: Vec<u8> = row.get(1)?;
    let privacy_policy_revision: i64 = row.get(2)?;
    let input_schema_version: i64 = row.get(3)?;
    let input_digest_revision: i64 = row.get(4)?;
    let output_schema_version: i64 = row.get(5)?;
    let provider: String = row.get(6)?;
    let adapter_id: String = row.get(7)?;
    let adapter_revision: i64 = row.get(8)?;
    let model_revision: String = row.get(9)?;
    let canonical_payload: Vec<u8> = row.get(10)?;
    let created_at_unix_ms: i64 = row.get(11)?;
    let expires_at_unix_ms: i64 = row.get(12)?;
    decode_values(
        insight_id,
        input_digest,
        privacy_policy_revision,
        input_schema_version,
        input_digest_revision,
        output_schema_version,
        provider,
        adapter_id,
        adapter_revision,
        model_revision,
        canonical_payload,
        created_at_unix_ms,
        expires_at_unix_ms,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)
}

#[allow(clippy::too_many_arguments)]
fn decode_values(
    insight_id: String,
    input_digest: Vec<u8>,
    privacy_policy_revision: i64,
    input_schema_version: i64,
    input_digest_revision: i64,
    output_schema_version: i64,
    provider: String,
    adapter_id: String,
    adapter_revision: i64,
    model_revision: String,
    canonical_payload: Vec<u8>,
    created_at_unix_ms: i64,
    expires_at_unix_ms: i64,
) -> Result<StoredAiInsightCacheRecord, HistoryError> {
    let input_digest: [u8; 32] = input_digest.try_into().map_err(|_| corrupt())?;
    let binding = AiInsightCacheBinding::new(
        input_digest,
        from_i64(privacy_policy_revision)?,
        from_i64(input_schema_version)?,
        from_i64(input_digest_revision)?,
        from_i64(output_schema_version)?,
        provider,
        adapter_id,
        from_i64(adapter_revision)?,
        model_revision,
    )
    .map_err(|_| corrupt())?;
    if insight_id.is_empty()
        || insight_id.len() > MAX_INSIGHT_ID_BYTES
        || insight_id.chars().any(char::is_control)
        || canonical_payload.is_empty()
        || canonical_payload.len() > MAX_AI_INSIGHT_PAYLOAD_BYTES
        || created_at_unix_ms < 0
        || expires_at_unix_ms <= created_at_unix_ms
        || expires_at_unix_ms - created_at_unix_ms
            != i64::try_from(AI_INSIGHT_CACHE_TTL.as_millis()).map_err(|_| corrupt())?
    {
        return Err(corrupt());
    }
    Ok(StoredAiInsightCacheRecord {
        binding,
        canonical_payload: canonical_payload.into_boxed_slice(),
        created_at: unix_ms_to_system_time(created_at_unix_ms)?,
        expires_at: unix_ms_to_system_time(expires_at_unix_ms)?,
        created_at_unix_ms,
        expires_at_unix_ms,
        insight_id,
    })
}

#[derive(Clone, Copy)]
struct Population {
    witness: [u8; 32],
    record_count: u32,
    logical_content_bytes: u64,
    expired_record_count: u32,
    expired_logical_content_bytes: u64,
}

impl Population {
    fn matches(self, prepared: &PreparedAiInsightCacheClear) -> bool {
        self.witness == prepared.witness
            && self.record_count == prepared.record_count
            && self.logical_content_bytes == prepared.logical_content_bytes
            && self.expired_record_count == prepared.expired_record_count
            && self.expired_logical_content_bytes == prepared.expired_logical_content_bytes
    }
}

fn inspect_population(
    connection: &Connection,
    observed_at_unix_ms: i64,
) -> Result<Population, HistoryError> {
    run_with_progress_budget(connection, || {
        let mut hasher = Sha256::new();
        hasher.update(b"dux.ai-insight-cache.clear-population.v1");
        let mut record_count = 0_u32;
        let mut logical_content_bytes = 0_u64;
        let mut expired_record_count = 0_u32;
        let mut expired_logical_content_bytes = 0_u64;
        let mut cursor: Option<String> = None;
        loop {
            let mut statement = connection
                .prepare(
                    "SELECT insight_id, input_digest, privacy_policy_revision,
                            input_schema_version, input_digest_revision,
                            output_schema_version, provider, adapter_id,
                            adapter_revision, model_revision, output_payload,
                            created_at_unix_ms, expires_at_unix_ms
                     FROM ai_insights
                     WHERE ?1 IS NULL OR insight_id > ?1
                     ORDER BY insight_id
                     LIMIT ?2",
                )
                .map_err(map_query_sql_error)?;
            let mut rows = statement
                .query(params![cursor, (CLEAR_PAGE_ROWS + 1) as i64])
                .map_err(map_query_sql_error)?;
            let mut page = Vec::with_capacity(CLEAR_PAGE_ROWS);
            while let Some(row) = rows.next().map_err(map_query_sql_error)? {
                if page.len() == CLEAR_PAGE_ROWS {
                    break;
                }
                page.push(decode_row(row).map_err(map_query_sql_error)?);
            }
            if page.is_empty() {
                break;
            }
            for record in &page {
                if usize::try_from(record_count).map_err(|_| limit())? >= MAX_CLEAR_RECORDS {
                    return Err(limit());
                }
                let logical = logical_content_size(record)?;
                hash_record(&mut hasher, record)?;
                record_count = record_count.checked_add(1).ok_or_else(limit)?;
                logical_content_bytes = logical_content_bytes
                    .checked_add(logical)
                    .ok_or_else(corrupt)?;
                if record.expires_at_unix_ms <= observed_at_unix_ms {
                    expired_record_count = expired_record_count.checked_add(1).ok_or_else(limit)?;
                    expired_logical_content_bytes = expired_logical_content_bytes
                        .checked_add(logical)
                        .ok_or_else(corrupt)?;
                }
            }
            cursor = page.last().map(|record| record.insight_id.clone());
            if page.len() < CLEAR_PAGE_ROWS {
                break;
            }
        }
        hasher.update(record_count.to_le_bytes());
        Ok(Population {
            witness: hasher.finalize().into(),
            record_count,
            logical_content_bytes,
            expired_record_count,
            expired_logical_content_bytes,
        })
    })
}

fn hash_record(
    hasher: &mut Sha256,
    record: &StoredAiInsightCacheRecord,
) -> Result<(), HistoryError> {
    hash_bytes(hasher, record.insight_id.as_bytes())?;
    hash_bytes(hasher, &record.binding.input_digest)?;
    for revision in [
        record.binding.privacy_policy_revision,
        record.binding.input_schema_version,
        record.binding.input_digest_revision,
        record.binding.output_schema_version,
        record.binding.adapter_revision,
    ] {
        hasher.update(revision.to_le_bytes());
    }
    for value in [
        record.binding.provider.as_bytes(),
        record.binding.adapter_id.as_bytes(),
        record.binding.model_revision.as_bytes(),
        record.canonical_payload.as_ref(),
    ] {
        hash_bytes(hasher, value)?;
    }
    hasher.update(record.created_at_unix_ms.to_le_bytes());
    hasher.update(record.expires_at_unix_ms.to_le_bytes());
    Ok(())
}

pub(super) fn logical_content_size(
    record: &StoredAiInsightCacheRecord,
) -> Result<u64, HistoryError> {
    [
        record.insight_id.len(),
        record.binding.input_digest.len(),
        record.binding.provider.len(),
        record.binding.adapter_id.len(),
        record.binding.model_revision.len(),
        record.canonical_payload.len(),
    ]
    .into_iter()
    .try_fold(0_u64, |total, length| {
        total
            .checked_add(u64::try_from(length).map_err(|_| corrupt())?)
            .ok_or_else(corrupt)
    })
}

fn stored_from_new(record: &NewAiInsightCacheRecord) -> StoredAiInsightCacheRecord {
    StoredAiInsightCacheRecord {
        binding: record.binding.clone(),
        canonical_payload: record.canonical_payload.clone(),
        created_at: record.created_at,
        expires_at: record.expires_at,
        created_at_unix_ms: record.created_at_unix_ms,
        expires_at_unix_ms: record.expires_at_unix_ms,
        insight_id: record.insight_id.clone(),
    }
}

fn generated_insight_id(
    binding: &AiInsightCacheBinding,
    payload: &[u8],
    created_at_unix_ms: i64,
    expires_at_unix_ms: i64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"dux.ai-insight-cache.record-id.v1");
    hasher.update(binding.input_digest);
    for revision in [
        binding.privacy_policy_revision,
        binding.input_schema_version,
        binding.input_digest_revision,
        binding.output_schema_version,
        binding.adapter_revision,
    ] {
        hasher.update(revision.to_le_bytes());
    }
    for value in [
        binding.provider.as_bytes(),
        binding.adapter_id.as_bytes(),
        binding.model_revision.as_bytes(),
        payload,
    ] {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value);
    }
    hasher.update(created_at_unix_ms.to_le_bytes());
    hasher.update(expires_at_unix_ms.to_le_bytes());
    let digest = hasher.finalize();
    let mut id = String::with_capacity(5 + digest.len() * 2);
    id.push_str("ai:v1:");
    for byte in digest {
        write!(&mut id, "{byte:02x}").expect("writing to String cannot fail");
    }
    id
}

fn hash_bytes(hasher: &mut Sha256, value: &[u8]) -> Result<(), HistoryError> {
    hasher.update(
        u64::try_from(value.len())
            .map_err(|_| corrupt())?
            .to_le_bytes(),
    );
    hasher.update(value);
    Ok(())
}

fn valid_positive_revision(value: u64) -> bool {
    value > 0 && i64::try_from(value).is_ok()
}

fn valid_fixed_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

fn to_i64(value: u64) -> Result<i64, HistoryError> {
    i64::try_from(value).map_err(|_| invalid())
}

fn from_i64(value: i64) -> Result<u64, HistoryError> {
    let value = u64::try_from(value).map_err(|_| corrupt())?;
    if value == 0 {
        Err(corrupt())
    } else {
        Ok(value)
    }
}

fn map_insert_error(error: rusqlite::Error) -> HistoryError {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
        HistoryError::new(HistoryErrorKind::AlreadyExists)
    } else {
        map_write_sql_error(error)
    }
}

fn map_clear_write_error(error: rusqlite::Error) -> HistoryError {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::OperationInterrupted) {
        limit()
    } else {
        map_write_sql_error(error)
    }
}

fn run_with_progress_budget<T>(
    connection: &Connection,
    operation: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    let started_at = Instant::now();
    let mut callbacks = 0_u64;
    connection
        .progress_handler(
            PROGRESS_OP_INTERVAL,
            Some(move || {
                callbacks = callbacks.saturating_add(1);
                callbacks >= MAX_PROGRESS_CALLBACKS || started_at.elapsed() >= MAX_ELAPSED
            }),
        )
        .map_err(|_| unavailable())?;
    let mut guard = AiInsightCacheProgressGuard {
        connection,
        installed: true,
    };
    let result = operation();
    guard.remove()?;
    let value = result?;
    if started_at.elapsed() >= MAX_ELAPSED {
        return Err(limit());
    }
    Ok(value)
}

struct AiInsightCacheProgressGuard<'a> {
    connection: &'a Connection,
    installed: bool,
}

impl AiInsightCacheProgressGuard<'_> {
    fn remove(&mut self) -> Result<(), HistoryError> {
        self.connection
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(|_| unavailable())?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for AiInsightCacheProgressGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self.connection.progress_handler(0, None::<fn() -> bool>);
        }
    }
}

struct AiInsightCacheClearAuthorizerGuard<'a> {
    connection: &'a Connection,
    installed: bool,
}

impl AiInsightCacheClearAuthorizerGuard<'_> {
    fn install(
        connection: &Connection,
    ) -> Result<AiInsightCacheClearAuthorizerGuard<'_>, HistoryError> {
        connection
            .authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Read { .. } | AuthAction::Select | AuthAction::Function { .. } => {
                    Authorization::Allow
                }
                AuthAction::Delete {
                    table_name: "ai_insights",
                } => Authorization::Allow,
                _ => Authorization::Deny,
            }))
            .map_err(|_| unavailable())?;
        Ok(AiInsightCacheClearAuthorizerGuard {
            connection,
            installed: true,
        })
    }

    fn remove(&mut self) -> Result<(), HistoryError> {
        self.connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .map_err(|_| unavailable())?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for AiInsightCacheClearAuthorizerGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self
                .connection
                .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
    }
}

fn invalid() -> HistoryError {
    HistoryError::new(HistoryErrorKind::InvalidInput)
}

fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

fn limit() -> HistoryError {
    HistoryError::new(HistoryErrorKind::QueryLimitExceeded)
}

fn unavailable() -> HistoryError {
    HistoryError::new(HistoryErrorKind::DatabaseUnavailable)
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use tempfile::TempDir;

    use super::*;
    use crate::persistence::StoreCoordinator;

    fn store(temp: &TempDir) -> std::sync::Arc<StoreCoordinator> {
        StoreCoordinator::open(&temp.path().join("data/dux.sqlite3")).unwrap()
    }

    fn binding(digest: u8) -> AiInsightCacheBinding {
        AiInsightCacheBinding::new(
            [digest; 32],
            1,
            1,
            1,
            1,
            "anthropic",
            "anthropic-messages-v1",
            1,
            "claude-sonnet-4-6",
        )
        .unwrap()
    }

    fn record(
        binding: AiInsightCacheBinding,
        payload: &[u8],
        created_at_ms: u64,
    ) -> NewAiInsightCacheRecord {
        NewAiInsightCacheRecord::new(
            binding,
            payload.to_vec().into_boxed_slice(),
            UNIX_EPOCH + Duration::from_millis(created_at_ms),
        )
        .unwrap()
    }

    #[test]
    fn binding_and_record_constructors_enforce_all_fixed_bounds_and_exact_ttl() {
        assert_eq!(
            AiInsightCacheBinding::new(
                [0; 32],
                0,
                1,
                1,
                1,
                "anthropic",
                "anthropic-messages-v1",
                1,
                "claude-sonnet-4-6",
            )
            .unwrap_err()
            .kind,
            HistoryErrorKind::InvalidInput
        );
        assert!(
            AiInsightCacheBinding::new(
                [0; 32],
                1,
                1,
                1,
                1,
                "anthropic\nother",
                "anthropic-messages-v1",
                1,
                "claude-sonnet-4-6",
            )
            .is_err()
        );
        assert!(NewAiInsightCacheRecord::new(binding(1), Box::new([]), UNIX_EPOCH).is_err());
        assert!(
            NewAiInsightCacheRecord::new(
                binding(1),
                vec![0; MAX_AI_INSIGHT_PAYLOAD_BYTES + 1].into_boxed_slice(),
                UNIX_EPOCH,
            )
            .is_err()
        );
        let record = record(binding(1), b"{}", 1_234);
        assert_eq!(
            record.created_at(),
            UNIX_EPOCH + Duration::from_millis(1_234)
        );
        assert_eq!(
            record.expires_at(),
            record.created_at() + AI_INSIGHT_CACHE_TTL
        );
    }

    #[test]
    fn exact_binding_load_and_first_valid_writer_wins_until_expiry() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let exact = binding(1);
        let first = record(exact.clone(), b"{\"summary\":\"first\"}", 1_000);
        assert!(matches!(
            store.insert_ai_insight_cache(&first).unwrap(),
            AiInsightCacheInsertOutcome::Inserted(_)
        ));

        let competing = record(exact.clone(), b"{\"summary\":\"second\"}", 2_000);
        let existing = store.insert_ai_insight_cache(&competing).unwrap();
        let AiInsightCacheInsertOutcome::ExistingUnexpired(existing) = existing else {
            panic!("unexpired first writer must win")
        };
        assert_eq!(existing.canonical_payload(), first.canonical_payload());
        assert!(
            store
                .load_ai_insight_cache(&binding(2), first.created_at())
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .load_ai_insight_cache(&exact, first.expires_at())
                .unwrap()
                .is_none()
        );

        let replacement_created_ms = u64::try_from(
            first
                .expires_at()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        let replacement = record(
            exact.clone(),
            b"{\"summary\":\"replacement\"}",
            replacement_created_ms,
        );
        assert!(matches!(
            store.insert_ai_insight_cache(&replacement).unwrap(),
            AiInsightCacheInsertOutcome::Inserted(_)
        ));
        let loaded = store
            .load_ai_insight_cache(&exact, replacement.created_at())
            .unwrap()
            .unwrap();
        assert_eq!(loaded.canonical_payload(), replacement.canonical_payload());
        assert_eq!(loaded.created_at(), replacement.created_at());
        assert_eq!(loaded.expires_at(), replacement.expires_at());
    }

    #[test]
    fn insert_reconciles_an_injected_post_commit_failure_without_duplication() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let record = record(binding(3), b"{\"summary\":\"committed\"}", 5_000);
        assert!(matches!(
            store
                .insert_ai_insight_cache_after_commit_failure_for_test(&record)
                .unwrap(),
            AiInsightCacheInsertOutcome::Inserted(_)
        ));
        let count = store.with_connection(|connection| {
            connection
                .query_row("SELECT count(*) FROM ai_insights", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        });
        assert_eq!(count, 1);
    }

    #[test]
    fn clear_preview_is_path_free_exact_and_reports_expired_content() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let first = record(binding(4), b"one", 1_000);
        let second = record(binding(5), b"second", 2_000);
        store.insert_ai_insight_cache(&first).unwrap();
        store.insert_ai_insight_cache(&second).unwrap();
        let observed_at = first.expires_at();
        let prepared = store
            .prepare_ai_insight_cache_clear(observed_at)
            .unwrap()
            .unwrap();
        assert_eq!(prepared.record_count(), 2);
        assert_eq!(prepared.expired_record_count(), 1);
        assert!(prepared.logical_content_bytes() > 2);
        assert!(prepared.expired_logical_content_bytes() < prepared.logical_content_bytes());
        assert_eq!(prepared.prepared_at(), observed_at);

        let result = store.clear_ai_insight_cache(prepared).unwrap();
        assert_eq!(result.records_removed, 2);
        assert_eq!(result.expired_records_removed, 1);
        assert!(
            store
                .prepare_ai_insight_cache_clear(observed_at)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn clear_rejects_population_drift_and_reconciles_post_commit_failure() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let observed_at = UNIX_EPOCH + Duration::from_secs(60);
        store
            .insert_ai_insight_cache(&record(binding(6), b"one", 1_000))
            .unwrap();
        let stale = store
            .prepare_ai_insight_cache_clear(observed_at)
            .unwrap()
            .unwrap();
        store
            .insert_ai_insight_cache(&record(binding(7), b"two", 2_000))
            .unwrap();
        assert_eq!(
            store.clear_ai_insight_cache(stale),
            Err(AiInsightCacheClearStoreError::ChangedSincePreview)
        );

        let current = store
            .prepare_ai_insight_cache_clear(observed_at)
            .unwrap()
            .unwrap();
        let result = store
            .clear_ai_insight_cache_after_commit_failure_for_test(current)
            .unwrap();
        assert_eq!(result.records_removed, 2);
        assert!(
            store
                .prepare_ai_insight_cache_clear(observed_at)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn clear_authorizer_denies_every_non_ai_table_and_non_delete_ai_mutation() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        store.with_connection(|connection| {
            let transaction = connection.unchecked_transaction().unwrap();
            let mut guard = AiInsightCacheClearAuthorizerGuard::install(&transaction).unwrap();
            assert!(transaction.execute("DELETE FROM ai_insights", []).is_ok());
            assert!(transaction.execute("DELETE FROM settings", []).is_err());
            assert!(
                transaction
                    .execute("UPDATE ai_insights SET output_payload = x'01'", [])
                    .is_err()
            );
            assert!(
                transaction
                    .execute(
                        "INSERT INTO settings (
                             setting_key, value_json, value_schema_version,
                             updated_at_unix_ms
                         ) VALUES ('forbidden', '{}', 1, 1)",
                        [],
                    )
                    .is_err()
            );
            guard.remove().unwrap();
            drop(guard);
            transaction.rollback().unwrap();
        });
    }

    #[test]
    fn uncertain_clear_reconciliation_is_outcome_unknown_without_retry() {
        let temp = TempDir::new().unwrap();
        let store = store(&temp);
        let observed_at = UNIX_EPOCH + Duration::from_secs(60);
        store
            .insert_ai_insight_cache(&record(binding(8), b"one", 1_000))
            .unwrap();
        let prepared = store
            .prepare_ai_insight_cache_clear(observed_at)
            .unwrap()
            .unwrap();
        assert_eq!(
            store.clear_ai_insight_cache_after_reconciliation_failure_for_test(prepared),
            Err(AiInsightCacheClearStoreError::History(HistoryError::new(
                HistoryErrorKind::OutcomeUnknown,
            )))
        );
        let count = store.with_connection(|connection| {
            connection
                .query_row("SELECT count(*) FROM ai_insights", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        });
        assert_eq!(count, 0);
    }
}
