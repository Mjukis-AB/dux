use std::fmt;
use std::num::NonZeroU32;
use std::str::FromStr;

use thiserror::Error;

const MAX_ID_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StableIdError {
    #[error("{kind} must not be empty")]
    Empty { kind: &'static str },
    #[error("{kind} must be at most {MAX_ID_BYTES} ASCII bytes")]
    TooLong { kind: &'static str },
    #[error("{kind} contains an invalid character at byte {index}")]
    InvalidCharacter { kind: &'static str, index: usize },
    #[error("{kind} contains an empty dot-separated segment")]
    EmptySegment { kind: &'static str },
    #[error("rule revision must be greater than zero")]
    ZeroRevision,
}

fn validate_token(value: String, kind: &'static str) -> Result<String, StableIdError> {
    if value.is_empty() {
        return Err(StableIdError::Empty { kind });
    }
    if value.len() > MAX_ID_BYTES {
        return Err(StableIdError::TooLong { kind });
    }
    if let Some((index, _)) = value.char_indices().find(|(_, character)| {
        !character.is_ascii_alphanumeric() && !matches!(character, '.' | '_' | '-' | ':')
    }) {
        return Err(StableIdError::InvalidCharacter { kind, index });
    }
    Ok(value)
}

fn validate_dotted_id(value: String, kind: &'static str) -> Result<String, StableIdError> {
    let value = validate_token(value, kind)?;
    let mut offset = 0;
    for segment in value.split('.') {
        if segment.is_empty() {
            return Err(StableIdError::EmptySegment { kind });
        }
        for (segment_index, character) in segment.char_indices() {
            let is_boundary =
                segment_index == 0 || segment_index + character.len_utf8() == segment.len();
            let valid = if is_boundary {
                character.is_ascii_lowercase() || character.is_ascii_digit()
            } else {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '_' | '-')
            };
            if !valid {
                return Err(StableIdError::InvalidCharacter {
                    kind,
                    index: offset + segment_index,
                });
            }
        }
        offset += segment.len() + 1;
    }
    Ok(value)
}

macro_rules! token_id {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, StableIdError> {
                validate_token(value.into(), $kind).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl FromStr for $name {
            type Err = StableIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = StableIdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
    };
}

token_id!(CandidateId, "candidate ID");
token_id!(ScanId, "scan ID");

macro_rules! dotted_id {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, StableIdError> {
                validate_dotted_id(value.into(), $kind).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl FromStr for $name {
            type Err = StableIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = StableIdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
    };
}

dotted_id!(RuleId, "rule ID");
dotted_id!(LocalizedTextKey, "localized text key");

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleRevision(NonZeroU32);

impl RuleRevision {
    pub fn new(value: u32) -> Result<Self, StableIdError> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or(StableIdError::ZeroRevision)
    }

    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl fmt::Display for RuleRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleRef {
    id: RuleId,
    revision: RuleRevision,
}

impl RuleRef {
    pub fn new(id: RuleId, revision: RuleRevision) -> Self {
        Self { id, revision }
    }

    pub fn id(&self) -> &RuleId {
        &self.id
    }

    pub fn revision(&self) -> RuleRevision {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_ids_accept_stable_keys_and_reject_ambiguous_forms() {
        assert_eq!(
            RuleId::new("developer.rust.target").unwrap().as_str(),
            "developer.rust.target"
        );

        for invalid in [
            "",
            ".developer",
            "developer.",
            "developer..rust",
            "Developer.rust",
            "developer.-rust",
            "developer.rust_",
            "developer/rust",
            "developer rust",
            "developer.rüst",
        ] {
            assert!(RuleId::new(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn opaque_ids_are_bounded_ascii_tokens() {
        assert!(CandidateId::new("candidate:01HZ-123").is_ok());
        assert!(ScanId::new("scan_2026-07-15").is_ok());
        assert!(CandidateId::new("candidate/one").is_err());
        assert!(ScanId::new("scan one").is_err());
        assert!(ScanId::new("x".repeat(MAX_ID_BYTES + 1)).is_err());
    }

    #[test]
    fn rule_references_always_include_a_nonzero_revision() {
        assert_eq!(RuleRevision::new(0), Err(StableIdError::ZeroRevision));
        let reference = RuleRef::new(
            RuleId::new("developer.rust.target").unwrap(),
            RuleRevision::new(3).unwrap(),
        );

        assert_eq!(reference.id().as_str(), "developer.rust.target");
        assert_eq!(reference.revision().get(), 3);
    }
}
