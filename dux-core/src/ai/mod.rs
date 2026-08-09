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
    AI_EXPLANATION_INPUT_SCHEMA_VERSION, AiInputNodeKindV1, PrivacyShapedAiInputV1,
    PrivacyShapingError, shape_ai_explanation_input_v1,
};

pub(crate) const AI_METADATA_INPUT_SCHEMA_VERSION: u64 = AI_EXPLANATION_INPUT_SCHEMA_VERSION;

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

    #[allow(
        dead_code,
        reason = "the mapping remains sealed until validated provider groups reach Explorer overlays"
    )]
    pub(crate) fn included_snapshot_node_ids(&self) -> &[u64] {
        self.shaped.included_snapshot_node_ids()
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
