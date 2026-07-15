//! Strict JSON conversion for versioned deterministic rule catalogs.
//!
//! This module deliberately has no filesystem, network, FFI, candidate, plan,
//! or execution API. Parsing proves document shape and domain invariants; it
//! does not prove that bytes came from a trusted signed application bundle.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use super::{
    ActivityGuard, CandidateAction, CandidateCategory, LocalizedTextKey, ProvenanceUrl, Rule,
    RuleDefinition, RuleGuards, RuleId, RuleMatcher, RuleMatcherDefinition, RuleRef, RuleRevision,
    RuleScope, RuleValidationError, SafetyTier, StableIdError,
};

const RULE_CATALOG_SCHEMA_VERSION: u64 = 1;
const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_RULES: usize = 256;
const MAX_MATCHER_VALUES: usize = 64;
const MAX_ACTIVITY_GUARDS: usize = 64;
const MAX_PROVENANCE_URLS: usize = 16;
const MAX_RELATIVE_PATH_BYTES: usize = 1024;
const MAX_PROVENANCE_URL_BYTES: usize = 2048;
const MAX_MINIMUM_AGE_DAYS: u64 = 36_500;
const SECONDS_PER_DAY: u64 = 86_400;

#[derive(Debug)]
pub(crate) struct RuleRegistry {
    rules: BTreeMap<RuleId, Rule>,
}

impl RuleRegistry {
    pub(crate) fn len(&self) -> usize {
        self.rules.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub(crate) fn get(&self, id: &RuleId) -> Option<&Rule> {
        self.rules.get(id)
    }

    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = &Rule> {
        self.rules.values()
    }
}

pub(crate) fn load_rule_registry_json(input: &[u8]) -> Result<RuleRegistry, RuleRegistryLoadError> {
    if input.len() > MAX_CATALOG_BYTES {
        return Err(RuleRegistryLoadError::DocumentTooLarge {
            actual_bytes: input.len(),
            maximum_bytes: MAX_CATALOG_BYTES,
        });
    }

    let document: RuleRegistryDocument =
        serde_json::from_slice(input).map_err(|source| RuleRegistryLoadError::MalformedJson {
            line: source.line(),
            column: source.column(),
            source,
        })?;

    if document.schema_version != RULE_CATALOG_SCHEMA_VERSION {
        return Err(RuleRegistryLoadError::UnsupportedSchemaVersion {
            found: document.schema_version,
            supported: RULE_CATALOG_SCHEMA_VERSION,
        });
    }
    if document.rules.is_empty() {
        return Err(RuleRegistryLoadError::EmptyRegistry);
    }
    if document.rules.len() > MAX_RULES {
        return Err(RuleRegistryLoadError::TooManyRules {
            count: document.rules.len(),
            maximum: MAX_RULES,
        });
    }

    let mut rules = BTreeMap::new();
    let mut first_seen = BTreeMap::<RuleId, (usize, RuleRevision)>::new();
    for (index, document) in document.rules.into_iter().enumerate() {
        let rule = document
            .try_into_rule()
            .map_err(|source| RuleRegistryLoadError::InvalidRule { index, source })?;
        let reference = rule.reference();
        if let Some((first_index, first_revision)) = first_seen.get(reference.id()) {
            if *first_revision == reference.revision() {
                return Err(RuleRegistryLoadError::DuplicateRuleReference {
                    first_index: *first_index,
                    duplicate_index: index,
                    id: reference.id().clone(),
                    revision: reference.revision(),
                });
            }
            return Err(RuleRegistryLoadError::ConflictingRuleRevisions {
                first_index: *first_index,
                duplicate_index: index,
                id: reference.id().clone(),
                first_revision: *first_revision,
                duplicate_revision: reference.revision(),
            });
        }

        let id = reference.id().clone();
        let revision = reference.revision();
        first_seen.insert(id.clone(), (index, revision));
        rules.insert(id, rule);
    }

    Ok(RuleRegistry { rules })
}

#[derive(Debug, Error)]
pub(crate) enum RuleRegistryLoadError {
    #[error("rule catalog is {actual_bytes} bytes; maximum is {maximum_bytes}")]
    DocumentTooLarge {
        actual_bytes: usize,
        maximum_bytes: usize,
    },
    #[error("rule catalog is not valid v1 JSON at line {line}, column {column}")]
    MalformedJson {
        line: usize,
        column: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("unsupported rule catalog schema version {found}; supported version is {supported}")]
    UnsupportedSchemaVersion { found: u64, supported: u64 },
    #[error("rule catalog must contain at least one rule")]
    EmptyRegistry,
    #[error("rule catalog contains {count} rules; maximum is {maximum}")]
    TooManyRules { count: usize, maximum: usize },
    #[error(
        "rule {duplicate_index} duplicates rule {first_index} with ID {id} and revision {revision}"
    )]
    DuplicateRuleReference {
        first_index: usize,
        duplicate_index: usize,
        id: RuleId,
        revision: RuleRevision,
    },
    #[error(
        "rule {duplicate_index} conflicts with rule {first_index} for ID {id}: revisions {first_revision} and {duplicate_revision}"
    )]
    ConflictingRuleRevisions {
        first_index: usize,
        duplicate_index: usize,
        id: RuleId,
        first_revision: RuleRevision,
        duplicate_revision: RuleRevision,
    },
    #[error("rule {index} is invalid")]
    InvalidRule {
        index: usize,
        #[source]
        source: RuleDocumentError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleDocumentField {
    Id,
    Revision,
    TitleKey,
    ExplanationKey,
    RequiredAncestorMarkersAny,
    RequiredMarkersAll,
    ForbiddenMarkersAny,
    ExactBundleIdentifiers,
    ExcludedDescendants,
    ProtectedDescendants,
    InactiveProcesses,
    Provenance,
}

impl fmt::Display for RuleDocumentField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Id => "id",
            Self::Revision => "revision",
            Self::TitleKey => "title_key",
            Self::ExplanationKey => "explanation_key",
            Self::RequiredAncestorMarkersAny => "required_ancestor_markers_any",
            Self::RequiredMarkersAll => "required_markers_all",
            Self::ForbiddenMarkersAny => "forbidden_markers_any",
            Self::ExactBundleIdentifiers => "exact_bundle_identifiers",
            Self::ExcludedDescendants => "excluded_descendants",
            Self::ProtectedDescendants => "protected_descendants",
            Self::InactiveProcesses => "inactive_processes",
            Self::Provenance => "provenance",
        })
    }
}

#[derive(Debug, Error)]
pub(crate) enum RuleDocumentError {
    #[error("invalid stable value for {field}")]
    InvalidStableField {
        field: RuleDocumentField,
        #[source]
        source: StableIdError,
    },
    #[error("{field} contains {count} values; maximum is {maximum}")]
    CollectionLimitExceeded {
        field: RuleDocumentField,
        count: usize,
        maximum: usize,
    },
    #[error("{field} contains duplicate values at indices {first_index} and {duplicate_index}")]
    DuplicateValue {
        field: RuleDocumentField,
        first_index: usize,
        duplicate_index: usize,
    },
    #[error("{field} value at index {index} exceeds {maximum_bytes} bytes")]
    StringLimitExceeded {
        field: RuleDocumentField,
        index: usize,
        maximum_bytes: usize,
    },
    #[error("minimum_age_days must be between 1 and {maximum} when present")]
    MinimumAgeOutOfRange { maximum: u64 },
    #[error("provenance value at index {index} is invalid")]
    InvalidProvenance {
        index: usize,
        #[source]
        source: RuleValidationError,
    },
    #[error("rule matcher is invalid")]
    InvalidMatcher(#[source] RuleValidationError),
    #[error("rule guards are invalid")]
    InvalidGuards(#[source] RuleValidationError),
    #[error("rule policy is invalid")]
    InvalidPolicy(#[source] RuleValidationError),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleRegistryDocument {
    schema_version: u64,
    rules: Vec<RuleDocument>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleDocument {
    id: String,
    revision: u32,
    title_key: String,
    category: CandidateCategoryDocument,
    scope: RuleScopeDocument,
    #[serde(deserialize_with = "deserialize_nullable")]
    path_component: Option<String>,
    required_ancestor_markers_any: Vec<String>,
    required_markers_all: Vec<String>,
    forbidden_markers_any: Vec<String>,
    exact_bundle_identifiers: Vec<String>,
    excluded_descendants: Vec<String>,
    protected_descendants: Vec<String>,
    #[serde(deserialize_with = "deserialize_nullable")]
    minimum_age_days: Option<u64>,
    minimum_bytes: u64,
    inactive_processes: Vec<ActivityGuardDocument>,
    requires_cloud_upload_complete: bool,
    safety: SafetyTierDocument,
    action: CandidateActionDocument,
    schedule_eligible: bool,
    explanation_key: String,
    provenance: Vec<String>,
}

impl RuleDocument {
    fn try_into_rule(self) -> Result<Rule, RuleDocumentError> {
        validate_rule_collections(&self)?;

        let reference = RuleRef::new(
            RuleId::new(self.id).map_err(|source| RuleDocumentError::InvalidStableField {
                field: RuleDocumentField::Id,
                source,
            })?,
            RuleRevision::new(self.revision).map_err(|source| {
                RuleDocumentError::InvalidStableField {
                    field: RuleDocumentField::Revision,
                    source,
                }
            })?,
        );
        let title_key = LocalizedTextKey::new(self.title_key).map_err(|source| {
            RuleDocumentError::InvalidStableField {
                field: RuleDocumentField::TitleKey,
                source,
            }
        })?;
        let explanation_key = LocalizedTextKey::new(self.explanation_key).map_err(|source| {
            RuleDocumentError::InvalidStableField {
                field: RuleDocumentField::ExplanationKey,
                source,
            }
        })?;

        let matcher = RuleMatcher::try_new(RuleMatcherDefinition {
            path_component: self.path_component,
            required_ancestor_markers_any: self.required_ancestor_markers_any,
            required_markers_all: self.required_markers_all,
            forbidden_markers_any: self.forbidden_markers_any,
            exact_bundle_identifiers: self.exact_bundle_identifiers,
            excluded_descendants: self.excluded_descendants,
            protected_descendants: self.protected_descendants,
        })
        .map_err(RuleDocumentError::InvalidMatcher)?;

        let minimum_age = match self.minimum_age_days {
            Some(days @ 1..=MAX_MINIMUM_AGE_DAYS) => {
                Some(Duration::from_secs(days * SECONDS_PER_DAY))
            }
            Some(_) => {
                return Err(RuleDocumentError::MinimumAgeOutOfRange {
                    maximum: MAX_MINIMUM_AGE_DAYS,
                });
            }
            None => None,
        };
        let guards = RuleGuards::try_new(
            minimum_age,
            self.minimum_bytes,
            self.inactive_processes
                .into_iter()
                .map(ActivityGuard::from)
                .collect(),
            self.requires_cloud_upload_complete,
        )
        .map_err(RuleDocumentError::InvalidGuards)?;

        let provenance = self
            .provenance
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                ProvenanceUrl::new(value)
                    .map_err(|source| RuleDocumentError::InvalidProvenance { index, source })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Rule::try_new(RuleDefinition {
            reference,
            title_key,
            category: self.category.into(),
            scope: self.scope.into(),
            matcher,
            guards,
            safety: self.safety.into(),
            action: self.action.into(),
            schedule_eligible: self.schedule_eligible,
            explanation_key,
            provenance,
        })
        .map_err(RuleDocumentError::InvalidPolicy)
    }
}

fn validate_rule_collections(document: &RuleDocument) -> Result<(), RuleDocumentError> {
    validate_string_collection(
        &document.required_ancestor_markers_any,
        RuleDocumentField::RequiredAncestorMarkersAny,
        MAX_MATCHER_VALUES,
        MAX_RELATIVE_PATH_BYTES,
    )?;
    validate_string_collection(
        &document.required_markers_all,
        RuleDocumentField::RequiredMarkersAll,
        MAX_MATCHER_VALUES,
        MAX_RELATIVE_PATH_BYTES,
    )?;
    validate_string_collection(
        &document.forbidden_markers_any,
        RuleDocumentField::ForbiddenMarkersAny,
        MAX_MATCHER_VALUES,
        MAX_RELATIVE_PATH_BYTES,
    )?;
    validate_string_collection(
        &document.exact_bundle_identifiers,
        RuleDocumentField::ExactBundleIdentifiers,
        MAX_MATCHER_VALUES,
        255,
    )?;
    validate_string_collection(
        &document.excluded_descendants,
        RuleDocumentField::ExcludedDescendants,
        MAX_MATCHER_VALUES,
        MAX_RELATIVE_PATH_BYTES,
    )?;
    validate_string_collection(
        &document.protected_descendants,
        RuleDocumentField::ProtectedDescendants,
        MAX_MATCHER_VALUES,
        MAX_RELATIVE_PATH_BYTES,
    )?;
    validate_collection(
        &document.inactive_processes,
        RuleDocumentField::InactiveProcesses,
        MAX_ACTIVITY_GUARDS,
    )?;
    if let Some((index, _)) = document
        .inactive_processes
        .iter()
        .enumerate()
        .find(|(_, guard)| guard.value().len() > 255)
    {
        return Err(RuleDocumentError::StringLimitExceeded {
            field: RuleDocumentField::InactiveProcesses,
            index,
            maximum_bytes: 255,
        });
    }
    validate_string_collection(
        &document.provenance,
        RuleDocumentField::Provenance,
        MAX_PROVENANCE_URLS,
        MAX_PROVENANCE_URL_BYTES,
    )?;
    Ok(())
}

fn validate_string_collection(
    values: &[String],
    field: RuleDocumentField,
    maximum: usize,
    maximum_bytes: usize,
) -> Result<(), RuleDocumentError> {
    validate_collection(values, field, maximum)?;
    if let Some((index, _)) = values
        .iter()
        .enumerate()
        .find(|(_, value)| value.len() > maximum_bytes)
    {
        return Err(RuleDocumentError::StringLimitExceeded {
            field,
            index,
            maximum_bytes,
        });
    }
    Ok(())
}

fn validate_collection<T: PartialEq>(
    values: &[T],
    field: RuleDocumentField,
    maximum: usize,
) -> Result<(), RuleDocumentError> {
    if values.len() > maximum {
        return Err(RuleDocumentError::CollectionLimitExceeded {
            field,
            count: values.len(),
            maximum,
        });
    }
    for (duplicate_index, value) in values.iter().enumerate() {
        if let Some(first_index) = values[..duplicate_index]
            .iter()
            .position(|existing| existing == value)
        {
            return Err(RuleDocumentError::DuplicateValue {
                field,
                first_index,
                duplicate_index,
            });
        }
    }
    Ok(())
}

fn deserialize_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(
    deny_unknown_fields,
    tag = "kind",
    content = "value",
    rename_all = "snake_case"
)]
enum ActivityGuardDocument {
    ProcessName(String),
    BundleIdentifier(String),
}

impl ActivityGuardDocument {
    fn value(&self) -> &str {
        match self {
            Self::ProcessName(value) | Self::BundleIdentifier(value) => value,
        }
    }
}

impl From<ActivityGuardDocument> for ActivityGuard {
    fn from(value: ActivityGuardDocument) -> Self {
        match value {
            ActivityGuardDocument::ProcessName(value) => Self::ProcessName(value),
            ActivityGuardDocument::BundleIdentifier(value) => Self::BundleIdentifier(value),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CandidateCategoryDocument {
    DeveloperArtifact,
    ApplicationCache,
    BrowserCache,
    LogAndDiagnostic,
    InstallerAndDownload,
    DeviceAndSimulatorData,
    CloudFile,
    LargeReviewItem,
    ProtectedSystemData,
    UnknownStorage,
}

impl From<CandidateCategoryDocument> for CandidateCategory {
    fn from(value: CandidateCategoryDocument) -> Self {
        match value {
            CandidateCategoryDocument::DeveloperArtifact => Self::DeveloperArtifact,
            CandidateCategoryDocument::ApplicationCache => Self::ApplicationCache,
            CandidateCategoryDocument::BrowserCache => Self::BrowserCache,
            CandidateCategoryDocument::LogAndDiagnostic => Self::LogAndDiagnostic,
            CandidateCategoryDocument::InstallerAndDownload => Self::InstallerAndDownload,
            CandidateCategoryDocument::DeviceAndSimulatorData => Self::DeviceAndSimulatorData,
            CandidateCategoryDocument::CloudFile => Self::CloudFile,
            CandidateCategoryDocument::LargeReviewItem => Self::LargeReviewItem,
            CandidateCategoryDocument::ProtectedSystemData => Self::ProtectedSystemData,
            CandidateCategoryDocument::UnknownStorage => Self::UnknownStorage,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RuleScopeDocument {
    Home,
    UserCacheDirectory,
    ConfiguredProjectRoots,
    SelectedScanRoot,
}

impl From<RuleScopeDocument> for RuleScope {
    fn from(value: RuleScopeDocument) -> Self {
        match value {
            RuleScopeDocument::Home => Self::Home,
            RuleScopeDocument::UserCacheDirectory => Self::UserCacheDirectory,
            RuleScopeDocument::ConfiguredProjectRoots => Self::ConfiguredProjectRoots,
            RuleScopeDocument::SelectedScanRoot => Self::SelectedScanRoot,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SafetyTierDocument {
    SafeRegenerable,
    SafeEvictable,
    ReviewRequired,
    Informational,
    Protected,
}

impl From<SafetyTierDocument> for SafetyTier {
    fn from(value: SafetyTierDocument) -> Self {
        match value {
            SafetyTierDocument::SafeRegenerable => Self::SafeRegenerable,
            SafetyTierDocument::SafeEvictable => Self::SafeEvictable,
            SafetyTierDocument::ReviewRequired => Self::ReviewRequired,
            SafetyTierDocument::Informational => Self::Informational,
            SafetyTierDocument::Protected => Self::Protected,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CandidateActionDocument {
    RemoveKnownRegenerableContents,
    EvictLocalCopy,
    MoveToTrash,
    RevealOnly,
    NoAction,
}

impl From<CandidateActionDocument> for CandidateAction {
    fn from(value: CandidateActionDocument) -> Self {
        match value {
            CandidateActionDocument::RemoveKnownRegenerableContents => {
                Self::RemoveKnownRegenerableContents
            }
            CandidateActionDocument::EvictLocalCopy => Self::EvictLocalCopy,
            CandidateActionDocument::MoveToTrash => Self::MoveToTrash,
            CandidateActionDocument::RevealOnly => Self::RevealOnly,
            CandidateActionDocument::NoAction => Self::NoAction,
        }
    }
}

#[cfg(test)]
#[path = "rule_document_tests.rs"]
mod tests;
