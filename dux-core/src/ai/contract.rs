//! Strict, versioned JSON contracts for presentation-only AI explanations.
//!
//! These types carry no path, candidate, safety, action, plan, approval,
//! schedule, provider command, or execution capability. Parsing an input is a
//! structural check only; the later privacy boundary must separately derive
//! and authorize the redacted metadata before any provider can receive it.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub(super) const AI_EXPLANATION_INPUT_SCHEMA_VERSION: u64 = 1;
pub(super) const AI_EXPLANATION_OUTPUT_SCHEMA_VERSION: u64 = 1;
pub(super) const AI_EXPLANATION_TASK: &str = "explain_storage_cluster";
pub(super) const MAX_AI_INPUT_BYTES: usize = 256 * 1024;
pub(super) const MAX_AI_OUTPUT_BYTES: usize = 64 * 1024;
pub(super) const MAX_AI_INPUT_CHILDREN: usize = 128;
pub(super) const MAX_AI_INPUT_CLASSIFICATIONS: usize = 128;
pub(super) const MAX_AI_OUTPUT_LABELS: usize = 16;
pub(super) const MAX_AI_OUTPUT_GROUPS: usize = 32;
pub(super) const MAX_AI_OUTPUT_GROUP_NODE_IDS: usize = 128;
pub(super) const MAX_AI_OUTPUT_TEXT_ITEMS: usize = 16;
pub(super) const MAX_AI_OUTPUT_RESEARCH_SUGGESTIONS: usize = 8;
pub(super) const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;

pub(super) const MAX_NODE_ID_BYTES: usize = 64;
pub(super) const MAX_CLASSIFICATION_ID_BYTES: usize = 128;
pub(super) const MAX_INPUT_TEXT_BYTES: usize = 512;
pub(super) const MAX_OUTPUT_SUMMARY_BYTES: usize = 4096;
pub(super) const MAX_OUTPUT_LABEL_BYTES: usize = 64;
pub(super) const MAX_OUTPUT_GROUP_TITLE_BYTES: usize = 256;
pub(super) const MAX_OUTPUT_GROUP_REASON_BYTES: usize = 1024;
pub(super) const MAX_OUTPUT_LIST_TEXT_BYTES: usize = 512;
const AI_INPUT_DIGEST_DOMAIN: &[u8] = b"dux-ai-explanation-input-v1\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AiJsonErrorKind {
    Io,
    Syntax,
    Data,
    Eof,
}

impl From<serde_json::error::Category> for AiJsonErrorKind {
    fn from(value: serde_json::error::Category) -> Self {
        match value {
            serde_json::error::Category::Io => Self::Io,
            serde_json::error::Category::Syntax => Self::Syntax,
            serde_json::error::Category::Data => Self::Data,
            serde_json::error::Category::Eof => Self::Eof,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AiContractField {
    RootLabel,
    TotalLogicalBytes,
    AgeSummary,
    OmittedChildCount,
    OmittedLogicalBytes,
    OmittedAgeSummary,
    Children,
    ChildLabel,
    ChildLogicalBytes,
    KnownClassifications,
    ClassificationId,
    ClassificationLabel,
    Summary,
    Labels,
    Label,
    Groups,
    GroupTitle,
    GroupNodeIds,
    GroupReason,
    Questions,
    Uncertainties,
    ResearchSuggestions,
}

impl fmt::Display for AiContractField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::RootLabel => "root_label",
            Self::TotalLogicalBytes => "total_logical_bytes",
            Self::AgeSummary => "age_summary",
            Self::OmittedChildCount => "omitted_child_count",
            Self::OmittedLogicalBytes => "omitted_logical_bytes",
            Self::OmittedAgeSummary => "omitted_age_summary",
            Self::Children => "children",
            Self::ChildLabel => "child.label",
            Self::ChildLogicalBytes => "child.logical_bytes",
            Self::KnownClassifications => "known_classifications",
            Self::ClassificationId => "classification.classification_id",
            Self::ClassificationLabel => "classification.label",
            Self::Summary => "summary",
            Self::Labels => "labels",
            Self::Label => "label",
            Self::Groups => "groups",
            Self::GroupTitle => "group.title",
            Self::GroupNodeIds => "group.input_node_ids",
            Self::GroupReason => "group.reason",
            Self::Questions => "questions",
            Self::Uncertainties => "uncertainties",
            Self::ResearchSuggestions => "research_suggestions",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AiTextErrorReason {
    Empty,
    TooLong,
    SurroundingWhitespace,
    ControlCharacter,
    InvisibleFormatting,
    PathLikeText,
    ActionLanguage,
    InvalidStableIdentifier,
}

impl fmt::Display for AiTextErrorReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "is empty",
            Self::TooLong => "exceeds its byte limit",
            Self::SurroundingWhitespace => "has leading or trailing whitespace",
            Self::ControlCharacter => "contains a control character",
            Self::InvisibleFormatting => "contains invisible formatting",
            Self::PathLikeText => "contains path-like text",
            Self::ActionLanguage => "contains cleanup or execution language",
            Self::InvalidStableIdentifier => "is not a stable identifier",
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(super) enum AiInputContractError {
    #[error("AI input is {actual_bytes} bytes; maximum is {maximum_bytes}")]
    DocumentTooLarge {
        actual_bytes: usize,
        maximum_bytes: usize,
    },
    #[error("AI input is not valid JSON at line {line}, column {column}: {kind:?}")]
    MalformedJson {
        line: usize,
        column: usize,
        kind: AiJsonErrorKind,
    },
    #[error("unsupported AI input schema version {found}")]
    UnsupportedSchemaVersion { found: u64 },
    #[error("unsupported AI input task")]
    UnsupportedTask,
    #[error("AI input v1 cannot contain file content")]
    ContentIncluded,
    #[error("AI input v1 cannot contain protected metadata")]
    ProtectedMetadata,
    #[error("{field} contains {count} values; maximum is {maximum}")]
    CollectionLimitExceeded {
        field: AiContractField,
        count: usize,
        maximum: usize,
    },
    #[error("{field} exceeds the exact interoperable JSON integer range")]
    IntegerLimitExceeded { field: AiContractField },
    #[error("{field} {reason}")]
    InvalidText {
        field: AiContractField,
        reason: AiTextErrorReason,
    },
    #[error("input child {index} has an invalid request-local node ID")]
    InvalidNodeId { index: usize },
    #[error("input children {first_index} and {duplicate_index} duplicate a node ID")]
    DuplicateNodeId {
        first_index: usize,
        duplicate_index: usize,
    },
    #[error("age summary does not exactly account for logical bytes")]
    InvalidAgeSummary,
    #[error("children_complete conflicts with omitted-child accounting")]
    InvalidOmissionAccounting,
    #[error("child and omission facts do not exactly account for the root observation")]
    InvalidChildAccounting,
    #[error("classification {index} references an unknown input node")]
    UnknownClassificationNode { index: usize },
    #[error(
        "classifications {first_index} and {duplicate_index} duplicate a node/classification pair"
    )]
    DuplicateClassification {
        first_index: usize,
        duplicate_index: usize,
    },
    #[error("input digest is not canonical lowercase SHA-256")]
    InvalidInputDigest,
    #[error("input digest does not match the canonical metadata payload")]
    InputDigestMismatch,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(super) enum AiOutputContractError {
    #[error("AI output is {actual_bytes} bytes; maximum is {maximum_bytes}")]
    DocumentTooLarge {
        actual_bytes: usize,
        maximum_bytes: usize,
    },
    #[error("AI output is not valid JSON at line {line}, column {column}: {kind:?}")]
    MalformedJson {
        line: usize,
        column: usize,
        kind: AiJsonErrorKind,
    },
    #[error("unsupported AI output schema version {found}")]
    UnsupportedSchemaVersion { found: u64 },
    #[error("unsupported AI output task")]
    UnsupportedTask,
    #[error("AI output digest is not canonical lowercase SHA-256")]
    InvalidInputDigest,
    #[error("AI output belongs to a different input payload")]
    InputDigestMismatch,
    #[error("{field} contains {count} values; maximum is {maximum}")]
    CollectionLimitExceeded {
        field: AiContractField,
        count: usize,
        maximum: usize,
    },
    #[error("{field} {reason}")]
    InvalidText {
        field: AiContractField,
        reason: AiTextErrorReason,
    },
    #[error("{field} contains duplicate values at indices {first_index} and {duplicate_index}")]
    DuplicateValue {
        field: AiContractField,
        first_index: usize,
        duplicate_index: usize,
    },
    #[error("AI group {group_index} contains an invalid request-local node ID")]
    InvalidNodeId { group_index: usize },
    #[error("AI group {group_index} references an unknown input node at index {reference_index}")]
    UnknownNodeReference {
        group_index: usize,
        reference_index: usize,
    },
    #[error("AI groups {first_group_index} and {duplicate_group_index} overlap")]
    OverlappingGroups {
        first_group_index: usize,
        duplicate_group_index: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AiInputNodeKindV1 {
    Directory,
    File,
    Symlink,
    Other,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AiObservationCoverageV1 {
    Complete,
    Partial,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AiAgeSummaryV1 {
    within_7_days_logical_bytes: u64,
    days_8_to_30_logical_bytes: u64,
    days_31_to_90_logical_bytes: u64,
    older_than_90_days_logical_bytes: u64,
    unknown_age_logical_bytes: u64,
}

impl AiAgeSummaryV1 {
    fn checked_total(&self) -> Option<u64> {
        self.within_7_days_logical_bytes
            .checked_add(self.days_8_to_30_logical_bytes)?
            .checked_add(self.days_31_to_90_logical_bytes)?
            .checked_add(self.older_than_90_days_logical_bytes)?
            .checked_add(self.unknown_age_logical_bytes)
    }

    fn values(&self) -> [u64; 5] {
        [
            self.within_7_days_logical_bytes,
            self.days_8_to_30_logical_bytes,
            self.days_31_to_90_logical_bytes,
            self.older_than_90_days_logical_bytes,
            self.unknown_age_logical_bytes,
        ]
    }

    fn is_zero(&self) -> bool {
        self.values().iter().all(|value| *value == 0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AiInputChildV1 {
    input_node_id: String,
    label: String,
    kind: AiInputNodeKindV1,
    logical_bytes: u64,
    age_summary: AiAgeSummaryV1,
    coverage: AiObservationCoverageV1,
    protected: bool,
}

impl AiInputChildV1 {
    pub(super) fn input_node_id(&self) -> &str {
        &self.input_node_id
    }

    pub(super) fn label(&self) -> &str {
        &self.label
    }

    pub(super) fn logical_bytes(&self) -> u64 {
        self.logical_bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AiKnownClassificationV1 {
    input_node_id: String,
    classification_id: String,
    label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AiInputMetadataV1 {
    root_label: String,
    total_logical_bytes: u64,
    age_summary: AiAgeSummaryV1,
    coverage: AiObservationCoverageV1,
    children_complete: bool,
    omitted_child_count: u64,
    omitted_logical_bytes: u64,
    omitted_age_summary: AiAgeSummaryV1,
    children: Vec<AiInputChildV1>,
    known_classifications: Vec<AiKnownClassificationV1>,
    protected: bool,
    content_included: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AiInputDocumentV1 {
    schema_version: u64,
    task: String,
    input_digest_sha256: String,
    metadata: AiInputMetadataV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AiExplanationInputV1 {
    input_digest_sha256: String,
    metadata: AiInputMetadataV1,
}

impl AiExplanationInputV1 {
    pub(super) fn input_digest_sha256(&self) -> &str {
        &self.input_digest_sha256
    }

    pub(super) fn root_label(&self) -> &str {
        &self.metadata.root_label
    }

    pub(super) fn total_logical_bytes(&self) -> u64 {
        self.metadata.total_logical_bytes
    }

    pub(super) fn children(&self) -> &[AiInputChildV1] {
        &self.metadata.children
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct AiOutputGroupDocumentV1 {
    title: String,
    input_node_ids: Vec<String>,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AiOutputDocumentV1 {
    schema_version: u64,
    task: String,
    input_digest_sha256: String,
    summary: String,
    labels: Vec<String>,
    groups: Vec<AiOutputGroupDocumentV1>,
    questions: Vec<String>,
    uncertainties: Vec<String>,
    research_suggestions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AiExplanationGroupV1 {
    title: String,
    input_node_ids: Vec<String>,
    reason: String,
}

impl AiExplanationGroupV1 {
    pub(super) fn title(&self) -> &str {
        &self.title
    }

    pub(super) fn input_node_ids(&self) -> &[String] {
        &self.input_node_ids
    }

    pub(super) fn reason(&self) -> &str {
        &self.reason
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AiExplanationOutputV1 {
    input_digest_sha256: String,
    summary: String,
    labels: Vec<String>,
    groups: Vec<AiExplanationGroupV1>,
    questions: Vec<String>,
    uncertainties: Vec<String>,
    research_suggestions: Vec<String>,
}

impl AiExplanationOutputV1 {
    pub(super) fn input_digest_sha256(&self) -> &str {
        &self.input_digest_sha256
    }

    pub(super) fn summary(&self) -> &str {
        &self.summary
    }

    pub(super) fn labels(&self) -> &[String] {
        &self.labels
    }

    pub(super) fn groups(&self) -> &[AiExplanationGroupV1] {
        &self.groups
    }

    pub(super) fn questions(&self) -> &[String] {
        &self.questions
    }

    pub(super) fn uncertainties(&self) -> &[String] {
        &self.uncertainties
    }

    pub(super) fn research_suggestions(&self) -> &[String] {
        &self.research_suggestions
    }
}

pub(super) fn parse_ai_explanation_input_v1(
    input: &[u8],
) -> Result<AiExplanationInputV1, AiInputContractError> {
    if input.len() > MAX_AI_INPUT_BYTES {
        return Err(AiInputContractError::DocumentTooLarge {
            actual_bytes: input.len(),
            maximum_bytes: MAX_AI_INPUT_BYTES,
        });
    }
    let document: AiInputDocumentV1 =
        serde_json::from_slice(input).map_err(map_input_json_error)?;
    if document.schema_version != AI_EXPLANATION_INPUT_SCHEMA_VERSION {
        return Err(AiInputContractError::UnsupportedSchemaVersion {
            found: document.schema_version,
        });
    }
    if document.task != AI_EXPLANATION_TASK {
        return Err(AiInputContractError::UnsupportedTask);
    }
    if !is_sha256_hex(&document.input_digest_sha256) {
        return Err(AiInputContractError::InvalidInputDigest);
    }
    validate_input_metadata(&document.metadata)?;
    let expected_digest = canonical_input_digest(&document.metadata);
    if document.input_digest_sha256 != expected_digest {
        return Err(AiInputContractError::InputDigestMismatch);
    }

    Ok(AiExplanationInputV1 {
        input_digest_sha256: document.input_digest_sha256,
        metadata: document.metadata,
    })
}

pub(super) fn parse_ai_explanation_output_v1(
    input: &AiExplanationInputV1,
    output: &[u8],
) -> Result<AiExplanationOutputV1, AiOutputContractError> {
    if output.len() > MAX_AI_OUTPUT_BYTES {
        return Err(AiOutputContractError::DocumentTooLarge {
            actual_bytes: output.len(),
            maximum_bytes: MAX_AI_OUTPUT_BYTES,
        });
    }
    let document: AiOutputDocumentV1 =
        serde_json::from_slice(output).map_err(map_output_json_error)?;
    if document.schema_version != AI_EXPLANATION_OUTPUT_SCHEMA_VERSION {
        return Err(AiOutputContractError::UnsupportedSchemaVersion {
            found: document.schema_version,
        });
    }
    if document.task != AI_EXPLANATION_TASK {
        return Err(AiOutputContractError::UnsupportedTask);
    }
    if !is_sha256_hex(&document.input_digest_sha256) {
        return Err(AiOutputContractError::InvalidInputDigest);
    }
    if document.input_digest_sha256 != input.input_digest_sha256 {
        return Err(AiOutputContractError::InputDigestMismatch);
    }

    validate_collection_limit(
        AiContractField::Labels,
        document.labels.len(),
        MAX_AI_OUTPUT_LABELS,
    )?;
    validate_collection_limit(
        AiContractField::Groups,
        document.groups.len(),
        MAX_AI_OUTPUT_GROUPS,
    )?;
    validate_collection_limit(
        AiContractField::Questions,
        document.questions.len(),
        MAX_AI_OUTPUT_TEXT_ITEMS,
    )?;
    validate_collection_limit(
        AiContractField::Uncertainties,
        document.uncertainties.len(),
        MAX_AI_OUTPUT_TEXT_ITEMS,
    )?;
    validate_collection_limit(
        AiContractField::ResearchSuggestions,
        document.research_suggestions.len(),
        MAX_AI_OUTPUT_RESEARCH_SUGGESTIONS,
    )?;
    validate_text(
        &document.summary,
        AiContractField::Summary,
        MAX_OUTPUT_SUMMARY_BYTES,
        AiTextPolicy::ProviderOutput,
    )
    .map_err(map_output_text_error)?;
    validate_unique_stable_texts(
        &document.labels,
        AiContractField::Label,
        MAX_OUTPUT_LABEL_BYTES,
        true,
    )?;
    validate_unique_plain_texts(
        &document.questions,
        AiContractField::Questions,
        MAX_OUTPUT_LIST_TEXT_BYTES,
    )?;
    validate_unique_plain_texts(
        &document.uncertainties,
        AiContractField::Uncertainties,
        MAX_OUTPUT_LIST_TEXT_BYTES,
    )?;
    validate_unique_plain_texts(
        &document.research_suggestions,
        AiContractField::ResearchSuggestions,
        MAX_OUTPUT_LIST_TEXT_BYTES,
    )?;

    let allowed_node_ids = input
        .metadata
        .children
        .iter()
        .map(|child| child.input_node_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut first_group_by_node = std::collections::BTreeMap::<String, usize>::new();
    let mut groups = Vec::with_capacity(document.groups.len());
    for (group_index, group) in document.groups.into_iter().enumerate() {
        validate_text(
            &group.title,
            AiContractField::GroupTitle,
            MAX_OUTPUT_GROUP_TITLE_BYTES,
            AiTextPolicy::ProviderOutput,
        )
        .map_err(map_output_text_error)?;
        validate_text(
            &group.reason,
            AiContractField::GroupReason,
            MAX_OUTPUT_GROUP_REASON_BYTES,
            AiTextPolicy::ProviderOutput,
        )
        .map_err(map_output_text_error)?;
        if group.input_node_ids.is_empty()
            || group.input_node_ids.len() > MAX_AI_OUTPUT_GROUP_NODE_IDS
        {
            return Err(AiOutputContractError::CollectionLimitExceeded {
                field: AiContractField::GroupNodeIds,
                count: group.input_node_ids.len(),
                maximum: MAX_AI_OUTPUT_GROUP_NODE_IDS,
            });
        }
        let mut group_ids = BTreeSet::new();
        for (reference_index, node_id) in group.input_node_ids.iter().enumerate() {
            if !is_node_id(node_id) {
                return Err(AiOutputContractError::InvalidNodeId { group_index });
            }
            if !group_ids.insert(node_id.as_str()) {
                let first_index = group
                    .input_node_ids
                    .iter()
                    .position(|candidate| candidate == node_id)
                    .unwrap_or(reference_index);
                return Err(AiOutputContractError::DuplicateValue {
                    field: AiContractField::GroupNodeIds,
                    first_index,
                    duplicate_index: reference_index,
                });
            }
            if !allowed_node_ids.contains(node_id.as_str()) {
                return Err(AiOutputContractError::UnknownNodeReference {
                    group_index,
                    reference_index,
                });
            }
            if let Some(first_group_index) =
                first_group_by_node.insert(node_id.clone(), group_index)
            {
                return Err(AiOutputContractError::OverlappingGroups {
                    first_group_index,
                    duplicate_group_index: group_index,
                });
            }
        }
        groups.push(AiExplanationGroupV1 {
            title: group.title,
            input_node_ids: group.input_node_ids,
            reason: group.reason,
        });
    }

    Ok(AiExplanationOutputV1 {
        input_digest_sha256: document.input_digest_sha256,
        summary: document.summary,
        labels: document.labels,
        groups,
        questions: document.questions,
        uncertainties: document.uncertainties,
        research_suggestions: document.research_suggestions,
    })
}

fn validate_input_metadata(metadata: &AiInputMetadataV1) -> Result<(), AiInputContractError> {
    if metadata.content_included {
        return Err(AiInputContractError::ContentIncluded);
    }
    if metadata.protected || metadata.children.iter().any(|child| child.protected) {
        return Err(AiInputContractError::ProtectedMetadata);
    }
    validate_text(
        &metadata.root_label,
        AiContractField::RootLabel,
        MAX_INPUT_TEXT_BYTES,
        AiTextPolicy::PrivacyShapedInput,
    )
    .map_err(map_input_text_error)?;
    validate_safe_integer(
        metadata.total_logical_bytes,
        AiContractField::TotalLogicalBytes,
    )?;
    validate_safe_integer(
        metadata.omitted_child_count,
        AiContractField::OmittedChildCount,
    )?;
    validate_safe_integer(
        metadata.omitted_logical_bytes,
        AiContractField::OmittedLogicalBytes,
    )?;
    validate_age_summary(
        &metadata.age_summary,
        metadata.total_logical_bytes,
        AiContractField::AgeSummary,
    )?;
    validate_age_summary(
        &metadata.omitted_age_summary,
        metadata.omitted_logical_bytes,
        AiContractField::OmittedAgeSummary,
    )?;
    let total_child_count = u64::try_from(metadata.children.len())
        .ok()
        .and_then(|count| count.checked_add(metadata.omitted_child_count))
        .ok_or(AiInputContractError::InvalidOmissionAccounting)?;
    if total_child_count > MAX_SAFE_JSON_INTEGER {
        return Err(AiInputContractError::InvalidOmissionAccounting);
    }
    if metadata.children_complete {
        if metadata.omitted_child_count != 0
            || metadata.omitted_logical_bytes != 0
            || !metadata.omitted_age_summary.is_zero()
        {
            return Err(AiInputContractError::InvalidOmissionAccounting);
        }
    } else if metadata.omitted_child_count == 0 {
        return Err(AiInputContractError::InvalidOmissionAccounting);
    }
    if metadata.children.len() > MAX_AI_INPUT_CHILDREN {
        return Err(AiInputContractError::CollectionLimitExceeded {
            field: AiContractField::Children,
            count: metadata.children.len(),
            maximum: MAX_AI_INPUT_CHILDREN,
        });
    }
    if metadata.known_classifications.len() > MAX_AI_INPUT_CLASSIFICATIONS {
        return Err(AiInputContractError::CollectionLimitExceeded {
            field: AiContractField::KnownClassifications,
            count: metadata.known_classifications.len(),
            maximum: MAX_AI_INPUT_CLASSIFICATIONS,
        });
    }

    let mut node_ids = std::collections::BTreeMap::<&str, usize>::new();
    let mut child_logical_bytes = 0u64;
    let mut child_age_bytes = [0u64; 5];
    for (index, child) in metadata.children.iter().enumerate() {
        if !is_node_id(&child.input_node_id) {
            return Err(AiInputContractError::InvalidNodeId { index });
        }
        if let Some(first_index) = node_ids.insert(&child.input_node_id, index) {
            return Err(AiInputContractError::DuplicateNodeId {
                first_index,
                duplicate_index: index,
            });
        }
        validate_text(
            &child.label,
            AiContractField::ChildLabel,
            MAX_INPUT_TEXT_BYTES,
            AiTextPolicy::PrivacyShapedInput,
        )
        .map_err(map_input_text_error)?;
        validate_safe_integer(child.logical_bytes, AiContractField::ChildLogicalBytes)?;
        validate_age_summary(
            &child.age_summary,
            child.logical_bytes,
            AiContractField::AgeSummary,
        )?;
        child_logical_bytes = child_logical_bytes
            .checked_add(child.logical_bytes)
            .ok_or(AiInputContractError::InvalidChildAccounting)?;
        for (accounted, observed) in child_age_bytes.iter_mut().zip(child.age_summary.values()) {
            *accounted = accounted
                .checked_add(observed)
                .ok_or(AiInputContractError::InvalidChildAccounting)?;
        }
    }
    if child_logical_bytes.checked_add(metadata.omitted_logical_bytes)
        != Some(metadata.total_logical_bytes)
    {
        return Err(AiInputContractError::InvalidChildAccounting);
    }
    for ((accounted, omitted), root) in child_age_bytes
        .into_iter()
        .zip(metadata.omitted_age_summary.values())
        .zip(metadata.age_summary.values())
    {
        if accounted.checked_add(omitted) != Some(root) {
            return Err(AiInputContractError::InvalidChildAccounting);
        }
    }

    let mut classifications = std::collections::BTreeMap::<(&str, &str), usize>::new();
    for (index, classification) in metadata.known_classifications.iter().enumerate() {
        if !node_ids.contains_key(classification.input_node_id.as_str()) {
            return Err(AiInputContractError::UnknownClassificationNode { index });
        }
        validate_stable_identifier(
            &classification.classification_id,
            AiContractField::ClassificationId,
            MAX_CLASSIFICATION_ID_BYTES,
            true,
            AiTextPolicy::Identifier,
        )
        .map_err(map_input_text_error)?;
        validate_text(
            &classification.label,
            AiContractField::ClassificationLabel,
            MAX_INPUT_TEXT_BYTES,
            AiTextPolicy::PrivacyShapedInput,
        )
        .map_err(map_input_text_error)?;
        let key = (
            classification.input_node_id.as_str(),
            classification.classification_id.as_str(),
        );
        if let Some(first_index) = classifications.insert(key, index) {
            return Err(AiInputContractError::DuplicateClassification {
                first_index,
                duplicate_index: index,
            });
        }
    }
    Ok(())
}

fn validate_age_summary(
    summary: &AiAgeSummaryV1,
    expected_logical_bytes: u64,
    field: AiContractField,
) -> Result<(), AiInputContractError> {
    for value in [
        summary.within_7_days_logical_bytes,
        summary.days_8_to_30_logical_bytes,
        summary.days_31_to_90_logical_bytes,
        summary.older_than_90_days_logical_bytes,
        summary.unknown_age_logical_bytes,
    ] {
        validate_safe_integer(value, field)?;
    }
    if summary.checked_total() != Some(expected_logical_bytes) {
        return Err(AiInputContractError::InvalidAgeSummary);
    }
    Ok(())
}

fn validate_safe_integer(value: u64, field: AiContractField) -> Result<(), AiInputContractError> {
    if value > MAX_SAFE_JSON_INTEGER {
        return Err(AiInputContractError::IntegerLimitExceeded { field });
    }
    Ok(())
}

fn validate_collection_limit(
    field: AiContractField,
    count: usize,
    maximum: usize,
) -> Result<(), AiOutputContractError> {
    if count > maximum {
        return Err(AiOutputContractError::CollectionLimitExceeded {
            field,
            count,
            maximum,
        });
    }
    Ok(())
}

fn validate_unique_stable_texts(
    values: &[String],
    field: AiContractField,
    maximum_bytes: usize,
    output_label: bool,
) -> Result<(), AiOutputContractError> {
    let mut seen = std::collections::BTreeMap::<&str, usize>::new();
    for (index, value) in values.iter().enumerate() {
        let result = if output_label {
            validate_stable_identifier(
                value,
                field,
                maximum_bytes,
                false,
                AiTextPolicy::ProviderOutput,
            )
        } else {
            validate_text(value, field, maximum_bytes, AiTextPolicy::ProviderOutput)
        };
        result.map_err(map_output_text_error)?;
        if let Some(first_index) = seen.insert(value, index) {
            return Err(AiOutputContractError::DuplicateValue {
                field,
                first_index,
                duplicate_index: index,
            });
        }
    }
    Ok(())
}

fn validate_unique_plain_texts(
    values: &[String],
    field: AiContractField,
    maximum_bytes: usize,
) -> Result<(), AiOutputContractError> {
    validate_unique_stable_texts(values, field, maximum_bytes, false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AiTextPolicy {
    Identifier,
    PrivacyShapedInput,
    ProviderOutput,
}

fn validate_text(
    value: &str,
    field: AiContractField,
    maximum_bytes: usize,
    policy: AiTextPolicy,
) -> Result<(), (AiContractField, AiTextErrorReason)> {
    if value.is_empty() {
        return Err((field, AiTextErrorReason::Empty));
    }
    if value.len() > maximum_bytes {
        return Err((field, AiTextErrorReason::TooLong));
    }
    if value.trim() != value {
        return Err((field, AiTextErrorReason::SurroundingWhitespace));
    }
    if value.chars().any(char::is_control) {
        return Err((field, AiTextErrorReason::ControlCharacter));
    }
    if value.chars().any(is_invisible_formatting) {
        return Err((field, AiTextErrorReason::InvisibleFormatting));
    }
    if policy != AiTextPolicy::Identifier && contains_path_like_text(value) {
        return Err((field, AiTextErrorReason::PathLikeText));
    }
    if policy == AiTextPolicy::ProviderOutput && contains_action_language(value) {
        return Err((field, AiTextErrorReason::ActionLanguage));
    }
    Ok(())
}

fn validate_stable_identifier(
    value: &str,
    field: AiContractField,
    maximum_bytes: usize,
    allow_dot: bool,
    policy: AiTextPolicy,
) -> Result<(), (AiContractField, AiTextErrorReason)> {
    validate_text(value, field, maximum_bytes, policy)?;
    let bytes = value.as_bytes();
    let boundary_valid = bytes.first().is_some_and(u8::is_ascii_lowercase)
        || bytes.first().is_some_and(u8::is_ascii_digit);
    let final_valid = bytes.last().is_some_and(u8::is_ascii_lowercase)
        || bytes.last().is_some_and(u8::is_ascii_digit);
    let characters_valid = bytes.iter().all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'_' | b'-')
            || (allow_dot && *byte == b'.')
    });
    if !boundary_valid || !final_valid || !characters_valid {
        return Err((field, AiTextErrorReason::InvalidStableIdentifier));
    }
    Ok(())
}

fn contains_action_language(value: &str) -> bool {
    const ACTION_PREFIXES: &[&str] = &[
        "approv",
        "clean",
        "clear",
        "command",
        "delet",
        "destr",
        "discard",
        "dispos",
        "empt",
        "eras",
        "evict",
        "execut",
        "install",
        "kill",
        "purg",
        "reclaim",
        "remov",
        "schedul",
        "shell",
        "trash",
        "uninstall",
        "unlink",
        "wip",
    ];
    const ACTION_WORDS: &[&str] = &[
        "chmod", "chown", "mv", "ran", "rm", "rmdir", "run", "running", "sudo", "xargs",
    ];
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .any(|word| {
            ACTION_WORDS
                .iter()
                .any(|action| word.eq_ignore_ascii_case(action))
                || ACTION_PREFIXES.iter().any(|prefix| {
                    word.get(..prefix.len())
                        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
                })
        })
}

fn contains_path_like_text(value: &str) -> bool {
    if value.contains("..")
        || contains_percent_encoded_path_marker(value)
        || value.chars().any(|character| {
            matches!(
                character,
                '/' | '\\' | '~' | '\u{2044}' | '\u{2215}' | '\u{29f5}' | '\u{ff0f}' | '\u{ff3c}'
            )
        })
    {
        return true;
    }

    value.char_indices().any(|(index, character)| {
        if character != ':' {
            return false;
        }
        let prefix = &value.as_bytes()[..index];
        let mut token_start = prefix.len();
        while token_start > 0 && is_uri_scheme_continuation(prefix[token_start - 1]) {
            token_start -= 1;
        }
        let scheme_or_drive = &prefix[token_start..];
        scheme_or_drive
            .first()
            .is_some_and(|first| first.is_ascii_alphabetic())
            && scheme_or_drive
                .iter()
                .copied()
                .all(is_uri_scheme_continuation)
    })
}

fn is_uri_scheme_continuation(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
}

fn contains_percent_encoded_path_marker(value: &str) -> bool {
    value.as_bytes().windows(3).any(|window| {
        window[0] == b'%'
            && matches!(
                (
                    window[1].to_ascii_lowercase(),
                    window[2].to_ascii_lowercase()
                ),
                (b'2', b'e' | b'f') | (b'5', b'c') | (b'7', b'e')
            )
    })
}

fn is_node_id(value: &str) -> bool {
    let Some(suffix) = value.strip_prefix("n-") else {
        return false;
    };
    value.len() <= MAX_NODE_ID_BYTES
        && !suffix.is_empty()
        && suffix
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && suffix.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_invisible_formatting(value: char) -> bool {
    matches!(
        value,
        '\u{00ad}'
            | '\u{034f}'
            | '\u{061c}'
            | '\u{180b}'..='\u{180f}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
    )
}

pub(super) fn canonical_input_digest(metadata: &AiInputMetadataV1) -> String {
    let mut hasher = Sha256::new();
    hasher.update(AI_INPUT_DIGEST_DOMAIN);
    hasher.update([0x01]); // metadata record
    hash_string(&mut hasher, 0x01, &metadata.root_label);
    hash_u64(&mut hasher, 0x02, metadata.total_logical_bytes);
    hash_age_summary(&mut hasher, 0x03, &metadata.age_summary);
    hash_enum(
        &mut hasher,
        0x04,
        observation_coverage_tag(metadata.coverage),
    );
    hash_bool(&mut hasher, 0x05, metadata.children_complete);
    hash_u64(&mut hasher, 0x06, metadata.omitted_child_count);
    hash_u64(&mut hasher, 0x07, metadata.omitted_logical_bytes);
    hash_age_summary(&mut hasher, 0x08, &metadata.omitted_age_summary);
    hash_vector_header(&mut hasher, 0x09, metadata.children.len());
    for child in &metadata.children {
        hasher.update([0xa1]); // child record
        hash_string(&mut hasher, 0x01, &child.input_node_id);
        hash_string(&mut hasher, 0x02, &child.label);
        hash_enum(&mut hasher, 0x03, input_node_kind_tag(child.kind));
        hash_u64(&mut hasher, 0x04, child.logical_bytes);
        hash_age_summary(&mut hasher, 0x05, &child.age_summary);
        hash_enum(&mut hasher, 0x06, observation_coverage_tag(child.coverage));
        hash_bool(&mut hasher, 0x07, child.protected);
    }
    hash_vector_header(&mut hasher, 0x0a, metadata.known_classifications.len());
    for classification in &metadata.known_classifications {
        hasher.update([0xa2]); // classification record
        hash_string(&mut hasher, 0x01, &classification.input_node_id);
        hash_string(&mut hasher, 0x02, &classification.classification_id);
        hash_string(&mut hasher, 0x03, &classification.label);
    }
    hash_bool(&mut hasher, 0x0b, metadata.protected);
    hash_bool(&mut hasher, 0x0c, metadata.content_included);
    lower_hex(&hasher.finalize())
}

fn hash_string(hasher: &mut Sha256, field_tag: u8, value: &str) {
    hasher.update([field_tag]);
    hash_length(hasher, value.len());
    hasher.update(value.as_bytes());
}

fn hash_u64(hasher: &mut Sha256, field_tag: u8, value: u64) {
    hasher.update([field_tag]);
    hasher.update(value.to_le_bytes());
}

fn hash_bool(hasher: &mut Sha256, field_tag: u8, value: bool) {
    hasher.update([field_tag, u8::from(value)]);
}

fn hash_enum(hasher: &mut Sha256, field_tag: u8, value_tag: u8) {
    hasher.update([field_tag, value_tag]);
}

fn hash_age_summary(hasher: &mut Sha256, field_tag: u8, summary: &AiAgeSummaryV1) {
    hasher.update([field_tag]);
    for value in summary.values() {
        hasher.update(value.to_le_bytes());
    }
}

fn hash_vector_header(hasher: &mut Sha256, field_tag: u8, length: usize) {
    hasher.update([field_tag]);
    hash_length(hasher, length);
}

fn hash_length(hasher: &mut Sha256, length: usize) {
    let length = u64::try_from(length).expect("AI contract lengths fit in u64");
    hasher.update(length.to_le_bytes());
}

fn input_node_kind_tag(kind: AiInputNodeKindV1) -> u8 {
    match kind {
        AiInputNodeKindV1::Directory => 0x01,
        AiInputNodeKindV1::File => 0x02,
        AiInputNodeKindV1::Symlink => 0x03,
        AiInputNodeKindV1::Other => 0x04,
        AiInputNodeKindV1::Unavailable => 0x05,
    }
}

fn observation_coverage_tag(coverage: AiObservationCoverageV1) -> u8 {
    match coverage {
        AiObservationCoverageV1::Complete => 0x01,
        AiObservationCoverageV1::Partial => 0x02,
        AiObservationCoverageV1::Unknown => 0x03,
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    value
}

fn map_input_json_error(error: serde_json::Error) -> AiInputContractError {
    AiInputContractError::MalformedJson {
        line: error.line(),
        column: error.column(),
        kind: error.classify().into(),
    }
}

fn map_output_json_error(error: serde_json::Error) -> AiOutputContractError {
    AiOutputContractError::MalformedJson {
        line: error.line(),
        column: error.column(),
        kind: error.classify().into(),
    }
}

fn map_input_text_error(
    (field, reason): (AiContractField, AiTextErrorReason),
) -> AiInputContractError {
    AiInputContractError::InvalidText { field, reason }
}

fn map_output_text_error(
    (field, reason): (AiContractField, AiTextErrorReason),
) -> AiOutputContractError {
    AiOutputContractError::InvalidText { field, reason }
}
