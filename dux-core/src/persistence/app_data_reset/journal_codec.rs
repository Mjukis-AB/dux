use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::*;

pub(super) fn encode_canonical_root_name(root_name: &OsStr) -> Result<String> {
    let bytes = root_name.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_CANONICAL_ROOT_NAME_BYTES
        || bytes == b"."
        || bytes == b".."
        || bytes.contains(&b'/')
        || bytes.contains(&0)
    {
        return Err(error(
            AppDataResetCoordinatorErrorKind::InvalidConfiguration,
        ));
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(encoded)
}

pub(super) fn decode_canonical_root_name(encoded: &str) -> Option<Vec<u8>> {
    let bytes = encoded.as_bytes();
    if bytes.len() < 2
        || !bytes.len().is_multiple_of(2)
        || bytes.len() > MAX_CANONICAL_ROOT_NAME_BYTES * 2
        || bytes
            .iter()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(byte))
    {
        return None;
    }
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let mut decoded = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        decoded.push(nibble(pair[0])? << 4 | nibble(pair[1])?);
    }
    if decoded == b"." || decoded == b".." || decoded.contains(&b'/') || decoded.contains(&0) {
        None
    } else {
        Some(decoded)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct JournalPayloadV1 {
    pub(super) transaction_id: String,
    pub(super) phase: AppDataResetPhase,
    pub(super) data_identity: AppDataResetStoreIdentity,
    pub(super) cache_identity: Option<AppDataResetStoreIdentity>,
    pub(super) data_stage_name: String,
    pub(super) cache_stage_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct JournalEnvelopeV1 {
    pub(super) format_version: u16,
    pub(super) payload: JournalPayloadV1,
    pub(super) digest_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalPayloadV2 {
    transaction_id: String,
    phase: AppDataResetPhase,
    data_identity: AppDataResetStoreIdentity,
    cache_identity: Option<AppDataResetStoreIdentity>,
    data_stage_name: String,
    cache_stage_name: Option<String>,
    canonical_root_name_hex: String,
    fresh_stage_name: String,
    fresh_data_identity: Option<AppDataResetStoreIdentity>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalEnvelopeV2 {
    format_version: u16,
    payload: JournalPayloadV2,
    digest_sha256: String,
}

#[derive(Deserialize)]
struct JournalVersionProbe {
    format_version: u16,
}

impl From<&AppDataResetJournal> for JournalPayloadV2 {
    fn from(journal: &AppDataResetJournal) -> Self {
        Self {
            transaction_id: journal.transaction_id.clone(),
            phase: journal.phase,
            data_identity: journal.data_identity,
            cache_identity: journal.cache_identity,
            data_stage_name: journal.data_stage_name.clone(),
            cache_stage_name: journal.cache_stage_name.clone(),
            canonical_root_name_hex: journal
                .canonical_root_name_hex
                .clone()
                .expect("validated V2 journal must bind the canonical root"),
            fresh_stage_name: journal.fresh_stage_name.clone(),
            fresh_data_identity: journal.fresh_data_identity,
        }
    }
}

impl From<JournalPayloadV2> for AppDataResetJournal {
    fn from(payload: JournalPayloadV2) -> Self {
        Self {
            transaction_id: payload.transaction_id,
            phase: payload.phase,
            data_identity: payload.data_identity,
            cache_identity: payload.cache_identity,
            data_stage_name: payload.data_stage_name,
            cache_stage_name: payload.cache_stage_name,
            canonical_root_name_hex: Some(payload.canonical_root_name_hex),
            fresh_stage_name: payload.fresh_stage_name,
            fresh_data_identity: payload.fresh_data_identity,
            legacy_without_canonical_root_name: false,
            legacy_complete_without_fresh_identity: false,
        }
    }
}

fn journal_from_v1(payload: JournalPayloadV1) -> Result<AppDataResetJournal> {
    if matches!(
        payload.phase,
        AppDataResetPhase::DataDetached
            | AppDataResetPhase::FreshNamespaceReady
            | AppDataResetPhase::Draining
    ) {
        return Err(error(AppDataResetCoordinatorErrorKind::IncompatibleJournal));
    }
    let transaction =
        AppDataResetTransaction::from_canonical_transaction_id(&payload.transaction_id)
            .ok_or_else(corrupt)?;
    Ok(AppDataResetJournal {
        transaction_id: payload.transaction_id,
        phase: payload.phase,
        data_identity: payload.data_identity,
        cache_identity: payload.cache_identity,
        data_stage_name: payload.data_stage_name,
        cache_stage_name: payload.cache_stage_name,
        canonical_root_name_hex: None,
        fresh_stage_name: transaction.fresh_stage().as_str().to_owned(),
        fresh_data_identity: None,
        legacy_without_canonical_root_name: true,
        legacy_complete_without_fresh_identity: payload.phase == AppDataResetPhase::Complete,
    })
}

pub(super) fn encode_journal(journal: &AppDataResetJournal) -> Result<Vec<u8>> {
    journal.validate()?;
    if journal.legacy_without_canonical_root_name || journal.legacy_complete_without_fresh_identity
    {
        return Err(error(AppDataResetCoordinatorErrorKind::IncompatibleJournal));
    }
    let payload = JournalPayloadV2::from(journal);
    let payload_bytes = serde_json::to_vec(&payload).map_err(|_| internal())?;
    let digest_sha256 = journal_digest(&payload_bytes);
    serde_json::to_vec(&JournalEnvelopeV2 {
        format_version: JOURNAL_FORMAT_VERSION,
        payload,
        digest_sha256,
    })
    .map_err(|_| internal())
}

pub(super) fn decode_journal(bytes: &[u8]) -> Result<AppDataResetJournal> {
    let probe: JournalVersionProbe = serde_json::from_slice(bytes).map_err(|_| corrupt())?;
    if probe.format_version > JOURNAL_FORMAT_VERSION {
        return Err(error(AppDataResetCoordinatorErrorKind::IncompatibleJournal));
    }
    match probe.format_version {
        LEGACY_JOURNAL_FORMAT_VERSION => {
            let envelope: JournalEnvelopeV1 =
                serde_json::from_slice(bytes).map_err(|_| corrupt())?;
            let payload_bytes = serde_json::to_vec(&envelope.payload).map_err(|_| corrupt())?;
            if envelope.digest_sha256 != journal_digest(&payload_bytes)
                || serde_json::to_vec(&envelope).map_err(|_| corrupt())? != bytes
            {
                return Err(corrupt());
            }
            let journal = journal_from_v1(envelope.payload)?;
            journal.validate()?;
            Ok(journal)
        }
        JOURNAL_FORMAT_VERSION => {
            let envelope: JournalEnvelopeV2 =
                serde_json::from_slice(bytes).map_err(|_| corrupt())?;
            let payload_bytes = serde_json::to_vec(&envelope.payload).map_err(|_| corrupt())?;
            if envelope.digest_sha256 != journal_digest(&payload_bytes) {
                return Err(corrupt());
            }
            let journal = AppDataResetJournal::from(envelope.payload);
            journal.validate()?;
            if encode_journal(&journal)? != bytes {
                return Err(corrupt());
            }
            Ok(journal)
        }
        _ => Err(corrupt()),
    }
}

pub(super) fn journal_digest(payload: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(DIGEST_DOMAIN);
    digest.update((payload.len() as u64).to_le_bytes());
    digest.update(payload);
    let digest: [u8; 32] = digest.finalize().into();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}
