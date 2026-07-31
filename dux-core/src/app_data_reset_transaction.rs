//! Sealed namespace names for one future app-data reset transaction.
//!
//! This module generates names only. It cannot inspect storage, write the
//! reset journal, detach a namespace, or perform cleanup.

const TRANSACTION_HEX_LENGTH: usize = 32;
const DATA_STAGE_PREFIX: &str = ".dux-reset-data-";
const CACHE_STAGE_PREFIX: &str = ".dux-reset-cache-";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDataResetTransactionErrorKind {
    RandomUnavailable,
}

/// One process-generated reset transaction and its role-separated namespace
/// components.
///
/// The distinct stage-name types prevent a caller from accidentally using the
/// cache destination for the data namespace or vice versa. Construction is
/// private and the value carries no storage or effect authority.
pub(crate) struct AppDataResetTransaction {
    transaction_id: Box<str>,
    data_stage: AppDataResetDataStageName,
    cache_stage: AppDataResetCacheStageName,
}

pub(crate) struct AppDataResetDataStageName(Box<str>);

pub(crate) struct AppDataResetCacheStageName(Box<str>);

impl AppDataResetTransaction {
    pub(crate) fn generate() -> Result<Self, AppDataResetTransactionErrorKind> {
        let mut random = [0_u8; TRANSACTION_HEX_LENGTH / 2];
        getrandom::fill(&mut random)
            .map_err(|_| AppDataResetTransactionErrorKind::RandomUnavailable)?;
        let mut transaction_id = String::with_capacity(TRANSACTION_HEX_LENGTH);
        for byte in random {
            use std::fmt::Write as _;
            write!(&mut transaction_id, "{byte:02x}")
                .expect("writing a fixed-size transaction identifier cannot fail");
        }
        Ok(Self::from_valid_transaction_id(transaction_id))
    }

    /// Reconstruct the typed reset namespace names from one canonical journal
    /// transaction identifier.
    ///
    /// This accepts no path or stage-name input. Callers must obtain the
    /// identifier from a fully validated reset journal; validation is repeated
    /// here so a malformed identifier can never mint typed namespace names.
    pub(crate) fn from_canonical_transaction_id(transaction_id: &str) -> Option<Self> {
        if transaction_id.len() != TRANSACTION_HEX_LENGTH
            || !transaction_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return None;
        }
        Some(Self::from_valid_transaction_id(transaction_id.to_owned()))
    }

    pub(crate) fn transaction_id(&self) -> &str {
        &self.transaction_id
    }

    pub(crate) const fn data_stage(&self) -> &AppDataResetDataStageName {
        &self.data_stage
    }

    pub(crate) const fn cache_stage(&self) -> &AppDataResetCacheStageName {
        &self.cache_stage
    }

    fn from_valid_transaction_id(transaction_id: String) -> Self {
        debug_assert_eq!(transaction_id.len(), TRANSACTION_HEX_LENGTH);
        debug_assert!(transaction_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        Self {
            data_stage: AppDataResetDataStageName(
                format!("{DATA_STAGE_PREFIX}{transaction_id}").into_boxed_str(),
            ),
            cache_stage: AppDataResetCacheStageName(
                format!("{CACHE_STAGE_PREFIX}{transaction_id}").into_boxed_str(),
            ),
            transaction_id: transaction_id.into_boxed_str(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(transaction_id: &str) -> Option<Self> {
        Self::from_canonical_transaction_id(transaction_id)
    }
}

impl AppDataResetDataStageName {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl AppDataResetCacheStageName {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_seals_role_specific_canonical_components() {
        let transaction = AppDataResetTransaction::from_canonical_transaction_id(
            "00112233445566778899aabbccddeeff",
        )
        .unwrap();

        assert_eq!(
            transaction.transaction_id(),
            "00112233445566778899aabbccddeeff"
        );
        assert_eq!(
            transaction.data_stage().as_str(),
            ".dux-reset-data-00112233445566778899aabbccddeeff"
        );
        assert_eq!(
            transaction.cache_stage().as_str(),
            ".dux-reset-cache-00112233445566778899aabbccddeeff"
        );
    }

    #[test]
    fn journal_constructor_rejects_noncanonical_identifiers() {
        for invalid in [
            "",
            "00112233445566778899aabbccddeef",
            "00112233445566778899aabbccddeeff0",
            "00112233445566778899AABBCCDDEEFF",
            "00112233445566778899aabbccddeef/",
        ] {
            assert!(AppDataResetTransaction::from_canonical_transaction_id(invalid).is_none());
        }
    }

    #[test]
    fn generated_transaction_has_exact_lower_hex_shape() {
        let transaction = AppDataResetTransaction::generate().unwrap();
        assert_eq!(transaction.transaction_id().len(), TRANSACTION_HEX_LENGTH);
        assert!(
            transaction
                .transaction_id()
                .bytes()
                .all(|byte| { byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) })
        );
        assert_eq!(
            transaction.data_stage().as_str().len(),
            DATA_STAGE_PREFIX.len() + TRANSACTION_HEX_LENGTH
        );
        assert_eq!(
            transaction.cache_stage().as_str().len(),
            CACHE_STAGE_PREFIX.len() + TRANSACTION_HEX_LENGTH
        );
    }
}
