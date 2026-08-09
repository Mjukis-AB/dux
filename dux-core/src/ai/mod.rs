//! Provider-neutral AI explanation contracts.
//!
//! This module is deliberately crate-private. Its provider-neutral contract
//! imports no DUX type; the sealed privacy child imports only complete scan
//! coverage and a validated immutable snapshot observation. Neither surface
//! reaches engine, planner, live filesystem, FFI, provider, or cleanup types.
//! Parsing input proves only wire shape and cannot mint the privacy proof.

mod contract;

#[cfg(test)]
mod contract_tests;

use std::fmt;
use std::sync::Arc;

use crate::domain::ScanCoverage;
use crate::persistence::snapshot::SnapshotReviewDocument;

use contract::{
    AI_EXPLANATION_INPUT_DIGEST_REVISION, AI_EXPLANATION_INPUT_SCHEMA_VERSION,
    AI_EXPLANATION_OUTPUT_SCHEMA_VERSION, AiExplanationOutputV1, AiInputNodeKindV1,
    AiOutputContractError, PrivacyShapedAiInputV1, PrivacyShapingError,
    canonical_ai_explanation_output_v1, parse_ai_explanation_output_v1,
    shape_ai_explanation_input_v1,
};

pub(crate) const AI_METADATA_INPUT_SCHEMA_VERSION: u64 = AI_EXPLANATION_INPUT_SCHEMA_VERSION;
pub(crate) const AI_METADATA_INPUT_DIGEST_REVISION: u64 = AI_EXPLANATION_INPUT_DIGEST_REVISION;
pub(crate) const AI_METADATA_OUTPUT_SCHEMA_VERSION: u64 = AI_EXPLANATION_OUTPUT_SCHEMA_VERSION;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AiMetadataShapeError {
    IncompleteCoverage,
    SelectionUnavailable,
    SelectionNotDirectory,
    SensitiveSelection,
    UnsupportedObservation,
    InspectionLimitExceeded,
    InvalidAccounting,
    ContractRejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AiMetadataAgeSummaryV1 {
    pub(crate) within_7_days_logical_bytes: u64,
    pub(crate) days_8_to_30_logical_bytes: u64,
    pub(crate) days_31_to_90_logical_bytes: u64,
    pub(crate) older_than_90_days_logical_bytes: u64,
    pub(crate) unknown_age_logical_bytes: u64,
}

impl AiMetadataAgeSummaryV1 {
    fn from_values(values: [u64; 5]) -> Self {
        Self {
            within_7_days_logical_bytes: values[0],
            days_8_to_30_logical_bytes: values[1],
            days_31_to_90_logical_bytes: values[2],
            older_than_90_days_logical_bytes: values[3],
            unknown_age_logical_bytes: values[4],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AiMetadataNodeKindV1 {
    Directory,
    File,
    Symlink,
    Other,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AiMetadataChildV1 {
    pub(crate) input_node_id: String,
    pub(crate) label: String,
    pub(crate) kind: AiMetadataNodeKindV1,
    pub(crate) logical_bytes: u64,
    pub(crate) age_summary: AiMetadataAgeSummaryV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AiMetadataProjectionV1 {
    pub(crate) root_label: String,
    pub(crate) total_logical_bytes: u64,
    pub(crate) age_summary: AiMetadataAgeSummaryV1,
    pub(crate) children_complete: bool,
    pub(crate) omitted_child_count: u64,
    pub(crate) omitted_logical_bytes: u64,
    pub(crate) omitted_age_summary: AiMetadataAgeSummaryV1,
    pub(crate) children: Vec<AiMetadataChildV1>,
}

/// Sealed metadata proof plus the exact path-free projection that may be shown
/// before any provider is selected. It is deliberately non-cloneable.
pub(crate) struct AiMetadataPreviewV1 {
    shaped: PrivacyShapedAiInputV1,
    projection: AiMetadataProjectionV1,
}

/// Bounded, path-free reasons an untrusted provider output was rejected.
///
/// Contract field names, JSON locations, request-local IDs, and source values
/// deliberately do not cross the private AI boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AiMetadataOutputError {
    DocumentTooLarge,
    MalformedJson,
    UnsupportedSchemaVersion,
    UnsupportedTask,
    InvalidInputDigest,
    InputDigestMismatch,
    CollectionLimitExceeded,
    InvalidText,
    DuplicateValue,
    InvalidNodeReference,
    OverlappingGroups,
    RequestLocalIdIncluded,
    InvalidMapping,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AiMetadataExplanationGroupV1 {
    pub(crate) title: String,
    pub(crate) snapshot_node_ids: Vec<u64>,
    pub(crate) reason: String,
}

impl fmt::Debug for AiMetadataExplanationGroupV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiMetadataExplanationGroupV1")
            .field("snapshot_node_count", &self.snapshot_node_ids.len())
            .finish_non_exhaustive()
    }
}

/// Sealed validation result. Request-local IDs have already been replaced by
/// exact snapshot node IDs from the privacy proof and cannot escape this type.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AiMetadataExplanationV1 {
    pub(crate) input_digest_sha256: String,
    pub(crate) summary: String,
    pub(crate) labels: Vec<String>,
    pub(crate) groups: Vec<AiMetadataExplanationGroupV1>,
    pub(crate) questions: Vec<String>,
    pub(crate) uncertainties: Vec<String>,
    pub(crate) research_suggestions: Vec<String>,
}

/// A provider output which passed the complete v1 contract together with its
/// canonical, request-local-ID representation. The canonical bytes remain
/// inert and are eligible only for the sealed cache bridge; presentation uses
/// the separately projected snapshot IDs.
pub(crate) struct CacheableAiMetadataExplanationV1 {
    pub(crate) explanation: AiMetadataExplanationV1,
    pub(crate) canonical_output_json_utf8: Box<[u8]>,
}

impl fmt::Debug for AiMetadataExplanationV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiMetadataExplanationV1")
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

impl fmt::Debug for AiMetadataPreviewV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiMetadataPreviewV1")
            .field("input_schema_version", &AI_METADATA_INPUT_SCHEMA_VERSION)
            .field("encoded_bytes", &self.shaped.encoded_json().len())
            .field("child_count", &self.projection.children.len())
            .finish_non_exhaustive()
    }
}

impl AiMetadataPreviewV1 {
    pub(crate) fn input_digest_sha256(&self) -> &str {
        self.shaped.checked_input().input_digest_sha256()
    }

    pub(crate) fn share_encoded_input_json(&self) -> Arc<[u8]> {
        self.shaped.share_encoded_json()
    }

    pub(crate) fn disclosure_values(&self) -> [u64; 5] {
        self.shaped.disclosure().values()
    }

    pub(crate) fn projection(&self) -> &AiMetadataProjectionV1 {
        &self.projection
    }

    /// Validate exactly one untrusted response against this checked input and
    /// replace its request-local group IDs with the corresponding immutable
    /// snapshot node IDs without ever publishing the complete mapping.
    pub(crate) fn validate_explanation_output_v1(
        &self,
        output_json_utf8: &[u8],
    ) -> Result<AiMetadataExplanationV1, AiMetadataOutputError> {
        self.validate_cacheable_explanation_output_v1(output_json_utf8)
            .map(|validated| validated.explanation)
    }

    /// Validate and canonicalize exactly one provider output. Callers cannot
    /// supply a cache key or binding; those remain fixed by the retained proof
    /// and the engine's reviewed adapter.
    pub(crate) fn validate_cacheable_explanation_output_v1(
        &self,
        output_json_utf8: &[u8],
    ) -> Result<CacheableAiMetadataExplanationV1, AiMetadataOutputError> {
        let output = parse_ai_explanation_output_v1(self.shaped.checked_input(), output_json_utf8)
            .map_err(map_output_error)?;
        let canonical_output_json_utf8 = canonical_ai_explanation_output_v1(&output)
            .ok_or(AiMetadataOutputError::InvalidMapping)?;
        let explanation = project_validated_output(&self.shaped, output)?;
        Ok(CacheableAiMetadataExplanationV1 {
            explanation,
            canonical_output_json_utf8,
        })
    }
}

fn project_validated_output(
    shaped: &PrivacyShapedAiInputV1,
    output: AiExplanationOutputV1,
) -> Result<AiMetadataExplanationV1, AiMetadataOutputError> {
    let input_children = shaped.checked_input().children();
    let snapshot_node_ids = shaped.included_snapshot_node_ids();
    if input_children.len() != snapshot_node_ids.len() {
        return Err(AiMetadataOutputError::InvalidMapping);
    }
    let request_local_id_is_exposed = |text: &str| {
        input_children
            .iter()
            .any(|child| text.contains(child.input_node_id()))
    };
    if request_local_id_is_exposed(output.summary())
        || output
            .labels()
            .iter()
            .any(|value| request_local_id_is_exposed(value))
        || output
            .questions()
            .iter()
            .any(|value| request_local_id_is_exposed(value))
        || output
            .uncertainties()
            .iter()
            .any(|value| request_local_id_is_exposed(value))
        || output
            .research_suggestions()
            .iter()
            .any(|value| request_local_id_is_exposed(value))
        || output.groups().iter().any(|group| {
            request_local_id_is_exposed(group.title())
                || request_local_id_is_exposed(group.reason())
        })
    {
        return Err(AiMetadataOutputError::RequestLocalIdIncluded);
    }

    let mut groups = Vec::new();
    groups
        .try_reserve_exact(output.groups().len())
        .map_err(|_| AiMetadataOutputError::CollectionLimitExceeded)?;
    for group in output.groups() {
        let mut mapped_ids = Vec::new();
        mapped_ids
            .try_reserve_exact(group.input_node_ids().len())
            .map_err(|_| AiMetadataOutputError::CollectionLimitExceeded)?;
        for request_local_id in group.input_node_ids() {
            let index = input_children
                .iter()
                .position(|child| child.input_node_id() == request_local_id)
                .ok_or(AiMetadataOutputError::InvalidMapping)?;
            mapped_ids.push(
                *snapshot_node_ids
                    .get(index)
                    .ok_or(AiMetadataOutputError::InvalidMapping)?,
            );
        }
        groups.push(AiMetadataExplanationGroupV1 {
            title: group.title().to_owned(),
            snapshot_node_ids: mapped_ids,
            reason: group.reason().to_owned(),
        });
    }

    Ok(AiMetadataExplanationV1 {
        input_digest_sha256: output.input_digest_sha256().to_owned(),
        summary: output.summary().to_owned(),
        labels: output.labels().to_vec(),
        groups,
        questions: output.questions().to_vec(),
        uncertainties: output.uncertainties().to_vec(),
        research_suggestions: output.research_suggestions().to_vec(),
    })
}

const fn map_output_error(error: AiOutputContractError) -> AiMetadataOutputError {
    match error {
        AiOutputContractError::DocumentTooLarge { .. } => AiMetadataOutputError::DocumentTooLarge,
        AiOutputContractError::MalformedJson { .. } => AiMetadataOutputError::MalformedJson,
        AiOutputContractError::UnsupportedSchemaVersion { .. } => {
            AiMetadataOutputError::UnsupportedSchemaVersion
        }
        AiOutputContractError::UnsupportedTask => AiMetadataOutputError::UnsupportedTask,
        AiOutputContractError::InvalidInputDigest => AiMetadataOutputError::InvalidInputDigest,
        AiOutputContractError::InputDigestMismatch => AiMetadataOutputError::InputDigestMismatch,
        AiOutputContractError::CollectionLimitExceeded { .. } => {
            AiMetadataOutputError::CollectionLimitExceeded
        }
        AiOutputContractError::InvalidText { .. } => AiMetadataOutputError::InvalidText,
        AiOutputContractError::DuplicateValue { .. } => AiMetadataOutputError::DuplicateValue,
        AiOutputContractError::InvalidNodeId { .. }
        | AiOutputContractError::UnknownNodeReference { .. } => {
            AiMetadataOutputError::InvalidNodeReference
        }
        AiOutputContractError::OverlappingGroups { .. } => AiMetadataOutputError::OverlappingGroups,
    }
}

pub(crate) fn shape_ai_metadata_preview_v1(
    document: &SnapshotReviewDocument,
    coverage: &ScanCoverage,
    selected_node_id: u64,
) -> Result<AiMetadataPreviewV1, AiMetadataShapeError> {
    let shaped = shape_ai_explanation_input_v1(document, coverage, selected_node_id)
        .map_err(map_privacy_error)?;
    let input = shaped.checked_input();
    let children = input
        .children()
        .iter()
        .map(|child| AiMetadataChildV1 {
            input_node_id: child.input_node_id().to_owned(),
            label: child.label().to_owned(),
            kind: map_node_kind(child.kind()),
            logical_bytes: child.logical_bytes(),
            age_summary: AiMetadataAgeSummaryV1::from_values(child.age_summary().values()),
        })
        .collect::<Vec<_>>();
    if children.len() != shaped.included_snapshot_node_ids().len() {
        return Err(AiMetadataShapeError::InvalidAccounting);
    }
    let projection = AiMetadataProjectionV1 {
        root_label: input.root_label().to_owned(),
        total_logical_bytes: input.total_logical_bytes(),
        age_summary: AiMetadataAgeSummaryV1::from_values(input.age_summary().values()),
        children_complete: input.children_complete(),
        omitted_child_count: input.omitted_child_count(),
        omitted_logical_bytes: input.omitted_logical_bytes(),
        omitted_age_summary: AiMetadataAgeSummaryV1::from_values(
            input.omitted_age_summary().values(),
        ),
        children,
    };
    Ok(AiMetadataPreviewV1 { shaped, projection })
}

fn map_node_kind(kind: AiInputNodeKindV1) -> AiMetadataNodeKindV1 {
    match kind {
        AiInputNodeKindV1::Directory => AiMetadataNodeKindV1::Directory,
        AiInputNodeKindV1::File => AiMetadataNodeKindV1::File,
        AiInputNodeKindV1::Symlink => AiMetadataNodeKindV1::Symlink,
        AiInputNodeKindV1::Other => AiMetadataNodeKindV1::Other,
        AiInputNodeKindV1::Unavailable => AiMetadataNodeKindV1::Unavailable,
    }
}

fn map_privacy_error(error: PrivacyShapingError) -> AiMetadataShapeError {
    match error {
        PrivacyShapingError::IncompleteCoverage => AiMetadataShapeError::IncompleteCoverage,
        PrivacyShapingError::SelectionUnavailable => AiMetadataShapeError::SelectionUnavailable,
        PrivacyShapingError::SelectionNotDirectory => AiMetadataShapeError::SelectionNotDirectory,
        PrivacyShapingError::SensitiveSelection => AiMetadataShapeError::SensitiveSelection,
        PrivacyShapingError::UnsupportedPathObservation
        | PrivacyShapingError::IncompleteObservation => {
            AiMetadataShapeError::UnsupportedObservation
        }
        PrivacyShapingError::InspectionLimitExceeded => {
            AiMetadataShapeError::InspectionLimitExceeded
        }
        PrivacyShapingError::InvalidAccounting => AiMetadataShapeError::InvalidAccounting,
        PrivacyShapingError::Contract(_) => AiMetadataShapeError::ContractRejected,
    }
}
