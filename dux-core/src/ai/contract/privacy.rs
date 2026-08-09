//! Core-owned privacy shaping for one explicit AI explanation observation.
//!
//! This module consumes only a validated immutable snapshot plus its complete
//! coverage fact. It emits code-owned generic labels and never exposes source
//! paths, source names, snapshot IDs, file content, or cleanup authority.

use std::fmt;
use std::sync::Arc;

use thiserror::Error;

use crate::domain::{ScanCoverage, ScanCoverageStatus};
use crate::persistence::snapshot::{
    HostEncoding, HostValue, SnapshotNode, SnapshotNodeKind, SnapshotReviewDocument,
    SnapshotScanFlags, SnapshotTimestamp,
};

use super::{
    AI_EXPLANATION_INPUT_SCHEMA_VERSION, AiAgeSummaryV1, AiExplanationInputV1, AiInputChildV1,
    AiInputContractError, AiInputMetadataV1, AiInputNodeKindV1, AiObservationCoverageV1,
    MAX_AI_INPUT_CHILDREN, encode_privacy_shaped_input_v1,
};

const AI_SENSITIVE_PATH_POLICY_REVISION: u64 = 1;
const MAX_AI_PRIVACY_INSPECTED_NODES: usize = 200_000;
const SECONDS_PER_DAY: u64 = 86_400;

/// Path-free failure taxonomy. No variant retains or formats source names.
#[derive(Debug, Error, PartialEq, Eq)]
pub(in crate::ai) enum PrivacyShapingError {
    #[error("AI privacy shaping requires complete scan coverage")]
    IncompleteCoverage,
    #[error("the selected snapshot observation is unavailable")]
    SelectionUnavailable,
    #[error("AI privacy shaping requires a directory selection")]
    SelectionNotDirectory,
    #[error("the selected observation belongs to a sensitive category")]
    SensitiveSelection,
    #[error("the snapshot contains an unsupported path observation")]
    UnsupportedPathObservation,
    #[error("the snapshot contains an incomplete observation")]
    IncompleteObservation,
    #[error("AI privacy shaping exceeded its inspection limit")]
    InspectionLimitExceeded,
    #[error("AI privacy shaping could not reconcile bounded metadata")]
    InvalidAccounting,
    #[error("AI privacy-shaped metadata did not satisfy the v1 contract")]
    Contract(#[source] AiInputContractError),
}

impl From<AiInputContractError> for PrivacyShapingError {
    fn from(value: AiInputContractError) -> Self {
        Self::Contract(value)
    }
}

/// Local-only disclosure facts for the pre-provider review UI.
///
/// It deliberately records no sensitive byte count, path, source name,
/// snapshot ID, or durable request identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ai) struct AiPrivacyDisclosureV1 {
    policy_revision: u64,
    inspected_node_count: u64,
    included_direct_child_count: u64,
    excluded_sensitive_direct_child_count: u64,
    omitted_eligible_direct_child_count: u64,
}

/// Non-cloneable proof that the encoded input came from this privacy shaper.
/// Parsing schema-valid JSON cannot construct this type.
pub(in crate::ai) struct PrivacyShapedAiInputV1 {
    checked_input: AiExplanationInputV1,
    encoded_json: Arc<[u8]>,
    disclosure: AiPrivacyDisclosureV1,
    included_snapshot_node_ids: Box<[u64]>,
}

impl fmt::Debug for PrivacyShapedAiInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivacyShapedAiInputV1")
            .field("schema_version", &AI_EXPLANATION_INPUT_SCHEMA_VERSION)
            .field("encoded_bytes", &self.encoded_json.len())
            .field("disclosure", &self.disclosure)
            .finish_non_exhaustive()
    }
}

impl PrivacyShapedAiInputV1 {
    pub(in crate::ai) fn checked_input(&self) -> &AiExplanationInputV1 {
        &self.checked_input
    }

    pub(in crate::ai) fn encoded_json(&self) -> &[u8] {
        &self.encoded_json
    }

    pub(in crate::ai) fn share_encoded_json(&self) -> Arc<[u8]> {
        Arc::clone(&self.encoded_json)
    }

    pub(in crate::ai) fn disclosure(&self) -> AiPrivacyDisclosureV1 {
        self.disclosure
    }

    #[allow(
        dead_code,
        reason = "the private mapping is consumed when validated AI groups reach Explorer overlays"
    )]
    pub(in crate::ai) fn included_snapshot_node_ids(&self) -> &[u64] {
        &self.included_snapshot_node_ids
    }
}

impl AiPrivacyDisclosureV1 {
    pub(in crate::ai) fn values(self) -> [u64; 5] {
        [
            self.policy_revision,
            self.inspected_node_count,
            self.included_direct_child_count,
            self.excluded_sensitive_direct_child_count,
            self.omitted_eligible_direct_child_count,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SensitiveCategory {
    ProtectedSystem,
    CredentialOrToken,
    Keychain,
    BrowserProfile,
    PrivateCommunication,
    PasswordManager,
    SecurityOrManagement,
    VirtualMachineOrContainer,
    CloudDocument,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AgeAccumulator {
    values: [u64; 5],
}

impl AgeAccumulator {
    fn add_timestamped(
        &mut self,
        logical_bytes: u64,
        modified_at: Option<SnapshotTimestamp>,
        observed_at: SnapshotTimestamp,
    ) -> Result<(), PrivacyShapingError> {
        let index = age_bucket(modified_at, observed_at);
        self.values[index] = self.values[index]
            .checked_add(logical_bytes)
            .ok_or(PrivacyShapingError::InvalidAccounting)?;
        Ok(())
    }

    fn checked_add(self, other: Self) -> Result<Self, PrivacyShapingError> {
        let mut combined = Self::default();
        for (target, (left, right)) in combined
            .values
            .iter_mut()
            .zip(self.values.into_iter().zip(other.values))
        {
            *target = left
                .checked_add(right)
                .ok_or(PrivacyShapingError::InvalidAccounting)?;
        }
        Ok(combined)
    }

    fn total(self) -> Option<u64> {
        self.values.into_iter().try_fold(0_u64, u64::checked_add)
    }

    fn into_contract(self) -> AiAgeSummaryV1 {
        AiAgeSummaryV1 {
            within_7_days_logical_bytes: self.values[0],
            days_8_to_30_logical_bytes: self.values[1],
            days_31_to_90_logical_bytes: self.values[2],
            older_than_90_days_logical_bytes: self.values[3],
            unknown_age_logical_bytes: self.values[4],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EligibleDirectChild {
    snapshot_id: u64,
    kind: AiInputNodeKindV1,
    logical_bytes: u64,
    age: AgeAccumulator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectChildInspection {
    Eligible(EligibleDirectChild),
    Sensitive,
}

/// Shape one selected immutable directory without making it provider-visible.
pub(in crate::ai) fn shape_ai_explanation_input_v1(
    document: &SnapshotReviewDocument,
    coverage: &ScanCoverage,
    selected_node_id: u64,
) -> Result<PrivacyShapedAiInputV1, PrivacyShapingError> {
    if coverage.status() != ScanCoverageStatus::Complete
        || coverage.measured_permille().is_none()
        || !coverage.issues().is_empty()
    {
        return Err(PrivacyShapingError::IncompleteCoverage);
    }

    let selected_index =
        usize::try_from(selected_node_id).map_err(|_| PrivacyShapingError::SelectionUnavailable)?;
    let selected = document
        .nodes
        .get(selected_index)
        .ok_or(PrivacyShapingError::SelectionUnavailable)?;
    if selected.id != selected_node_id {
        return Err(PrivacyShapingError::SelectionUnavailable);
    }
    if selected.kind != SnapshotNodeKind::Directory {
        return Err(PrivacyShapingError::SelectionNotDirectory);
    }

    let selected_components = selected_component_chain(document, selected_index)?;
    if selected_components.is_empty() || classify_normalized_path(&selected_components).is_some() {
        return Err(PrivacyShapingError::SensitiveSelection);
    }
    ensure_complete_observation(selected)?;

    let mut inspected_node_count = 1_usize;
    let child_indices = document
        .direct_child_indices(selected_index)
        .map_err(|_| PrivacyShapingError::SelectionUnavailable)?;
    let mut eligible = Vec::new();
    eligible
        .try_reserve(child_indices.len().min(MAX_AI_PRIVACY_INSPECTED_NODES))
        .map_err(|_| PrivacyShapingError::InspectionLimitExceeded)?;
    let mut excluded_sensitive = 0_u64;
    let mut traversal_components = selected_components;
    let selected_component_count = traversal_components.len();

    for child_index in child_indices {
        traversal_components.truncate(selected_component_count);
        match inspect_direct_child(
            document,
            usize::try_from(*child_index).map_err(|_| PrivacyShapingError::SelectionUnavailable)?,
            &mut traversal_components,
            &mut inspected_node_count,
        )? {
            DirectChildInspection::Eligible(child) => eligible.push(child),
            DirectChildInspection::Sensitive => {
                excluded_sensitive = excluded_sensitive
                    .checked_add(1)
                    .ok_or(PrivacyShapingError::InvalidAccounting)?;
            }
        }
    }

    eligible.sort_by(|left, right| {
        right
            .logical_bytes
            .cmp(&left.logical_bytes)
            .then_with(|| left.snapshot_id.cmp(&right.snapshot_id))
    });

    let included_count = eligible.len().min(MAX_AI_INPUT_CHILDREN);
    let (included, omitted) = eligible.split_at(included_count);
    let mut root_logical_bytes = 0_u64;
    let mut root_age = AgeAccumulator::default();
    for child in &eligible {
        root_logical_bytes = root_logical_bytes
            .checked_add(child.logical_bytes)
            .ok_or(PrivacyShapingError::InvalidAccounting)?;
        root_age = root_age.checked_add(child.age)?;
    }

    let mut children = Vec::new();
    children
        .try_reserve_exact(included.len())
        .map_err(|_| PrivacyShapingError::InspectionLimitExceeded)?;
    for (index, child) in included.iter().enumerate() {
        let ordinal = index
            .checked_add(1)
            .ok_or(PrivacyShapingError::InvalidAccounting)?;
        children.push(AiInputChildV1 {
            input_node_id: format!("n-{ordinal}"),
            label: generic_child_label(child.kind, ordinal),
            kind: child.kind,
            logical_bytes: child.logical_bytes,
            age_summary: child.age.into_contract(),
            coverage: AiObservationCoverageV1::Complete,
            protected: false,
        });
    }

    let mut omitted_logical_bytes = 0_u64;
    let mut omitted_age = AgeAccumulator::default();
    for child in omitted {
        omitted_logical_bytes = omitted_logical_bytes
            .checked_add(child.logical_bytes)
            .ok_or(PrivacyShapingError::InvalidAccounting)?;
        omitted_age = omitted_age.checked_add(child.age)?;
    }
    let omitted_child_count =
        u64::try_from(omitted.len()).map_err(|_| PrivacyShapingError::InvalidAccounting)?;

    let metadata = AiInputMetadataV1 {
        root_label: "Selected folder".to_owned(),
        total_logical_bytes: root_logical_bytes,
        age_summary: root_age.into_contract(),
        coverage: AiObservationCoverageV1::Complete,
        children_complete: omitted.is_empty(),
        omitted_child_count,
        omitted_logical_bytes,
        omitted_age_summary: omitted_age.into_contract(),
        children,
        known_classifications: Vec::new(),
        protected: false,
        content_included: false,
    };
    let (checked_input, encoded_json) = encode_privacy_shaped_input_v1(metadata)?;
    let disclosure = AiPrivacyDisclosureV1 {
        policy_revision: AI_SENSITIVE_PATH_POLICY_REVISION,
        inspected_node_count: u64::try_from(inspected_node_count)
            .map_err(|_| PrivacyShapingError::InvalidAccounting)?,
        included_direct_child_count: u64::try_from(included.len())
            .map_err(|_| PrivacyShapingError::InvalidAccounting)?,
        excluded_sensitive_direct_child_count: excluded_sensitive,
        omitted_eligible_direct_child_count: omitted_child_count,
    };
    Ok(PrivacyShapedAiInputV1 {
        checked_input,
        encoded_json: Arc::from(encoded_json),
        disclosure,
        included_snapshot_node_ids: included
            .iter()
            .map(|child| child.snapshot_id)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    })
}

fn inspect_direct_child(
    document: &SnapshotReviewDocument,
    child_index: usize,
    components: &mut Vec<String>,
    inspected_node_count: &mut usize,
) -> Result<DirectChildInspection, PrivacyShapingError> {
    let child = document
        .nodes
        .get(child_index)
        .ok_or(PrivacyShapingError::SelectionUnavailable)?;
    let child_depth = child.depth;
    let selected_component_count = components.len();
    let mut age = AgeAccumulator::default();
    let mut sensitive = false;
    let mut index = child_index;

    while let Some(node) = document.nodes.get(index) {
        if index != child_index && node.depth <= child_depth {
            break;
        }
        charge_inspected_node(inspected_node_count)?;
        let relative_depth = node
            .depth
            .checked_sub(child_depth)
            .ok_or(PrivacyShapingError::InvalidAccounting)?;
        let parent_component_count = selected_component_count
            .checked_add(
                usize::try_from(relative_depth)
                    .map_err(|_| PrivacyShapingError::InvalidAccounting)?,
            )
            .ok_or(PrivacyShapingError::InvalidAccounting)?;
        if components.len() < parent_component_count {
            return Err(PrivacyShapingError::InvalidAccounting);
        }
        components.truncate(parent_component_count);
        let name = node
            .name
            .as_ref()
            .ok_or(PrivacyShapingError::UnsupportedPathObservation)?;
        let component = normalize_component(&decode_host_value(name)?)?;
        let category =
            classify_sensitive_component(components.last().map(String::as_str), &component);
        components.push(component);
        ensure_complete_observation(node)?;
        if category.is_some() {
            sensitive = true;
        }
        if node.kind != SnapshotNodeKind::Directory {
            age.add_timestamped(
                node.logical_bytes,
                node.modified_at,
                document.metadata.captured_at,
            )?;
        }
        index = index
            .checked_add(1)
            .ok_or(PrivacyShapingError::InspectionLimitExceeded)?;
    }

    if sensitive {
        return Ok(DirectChildInspection::Sensitive);
    }

    if age.total() != Some(child.logical_bytes) {
        return Err(PrivacyShapingError::InvalidAccounting);
    }
    Ok(DirectChildInspection::Eligible(EligibleDirectChild {
        snapshot_id: child.id,
        kind: contract_kind(child.kind)?,
        logical_bytes: child.logical_bytes,
        age,
    }))
}

fn charge_inspected_node(count: &mut usize) -> Result<(), PrivacyShapingError> {
    *count = count
        .checked_add(1)
        .ok_or(PrivacyShapingError::InspectionLimitExceeded)?;
    if *count > MAX_AI_PRIVACY_INSPECTED_NODES {
        return Err(PrivacyShapingError::InspectionLimitExceeded);
    }
    Ok(())
}

fn selected_component_chain(
    document: &SnapshotReviewDocument,
    selected_index: usize,
) -> Result<Vec<String>, PrivacyShapingError> {
    let mut components = root_components(&document.metadata.root)?;
    let mut lineage = Vec::new();
    let mut current = selected_index;
    while current != 0 {
        if lineage.len() >= document.nodes.len() {
            return Err(PrivacyShapingError::InvalidAccounting);
        }
        let node = document
            .nodes
            .get(current)
            .ok_or(PrivacyShapingError::SelectionUnavailable)?;
        let name = node
            .name
            .as_ref()
            .ok_or(PrivacyShapingError::UnsupportedPathObservation)?;
        lineage.push(normalize_component(&decode_host_value(name)?)?);
        current = usize::try_from(node.parent.ok_or(PrivacyShapingError::InvalidAccounting)?)
            .map_err(|_| PrivacyShapingError::InvalidAccounting)?;
    }
    components.extend(lineage.into_iter().rev());
    Ok(components)
}

fn root_components(root: &HostValue) -> Result<Vec<String>, PrivacyShapingError> {
    let decoded = decode_host_value(root)?;
    let mut components = Vec::new();
    match root.encoding() {
        HostEncoding::UnixBytes => {
            for value in decoded.split('/').filter(|component| !component.is_empty()) {
                components.push(normalize_component(value)?);
            }
            if components.is_empty() && decoded != "/" {
                return Err(PrivacyShapingError::UnsupportedPathObservation);
            }
        }
        HostEncoding::WindowsUtf16Le => {
            if decoded.starts_with("\\\\") || decoded.starts_with("//") {
                return Err(PrivacyShapingError::UnsupportedPathObservation);
            }
            let mut values = decoded
                .split(['/', '\\'])
                .filter(|component| !component.is_empty());
            let drive = values
                .next()
                .ok_or(PrivacyShapingError::UnsupportedPathObservation)?;
            if !is_windows_drive_prefix(drive) {
                return Err(PrivacyShapingError::UnsupportedPathObservation);
            }
            for value in values {
                components.push(normalize_component(value)?);
            }
            if components.is_empty() && !is_windows_drive_root(&decoded) {
                return Err(PrivacyShapingError::UnsupportedPathObservation);
            }
        }
    }
    Ok(components)
}

fn decode_host_value(value: &HostValue) -> Result<String, PrivacyShapingError> {
    match value.encoding() {
        HostEncoding::UnixBytes => String::from_utf8(value.bytes().to_vec())
            .map_err(|_| PrivacyShapingError::UnsupportedPathObservation),
        HostEncoding::WindowsUtf16Le => {
            let chunks = value.bytes().chunks_exact(2);
            if !chunks.remainder().is_empty() {
                return Err(PrivacyShapingError::UnsupportedPathObservation);
            }
            char::decode_utf16(chunks.map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]])))
                .collect::<Result<String, _>>()
                .map_err(|_| PrivacyShapingError::UnsupportedPathObservation)
        }
    }
}

fn normalize_component(value: &str) -> Result<String, PrivacyShapingError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.starts_with('~')
        || value.starts_with(char::is_whitespace)
        || value.ends_with(char::is_whitespace)
        || value.ends_with('.')
        || value.contains(['/', '\\', '\0'])
        || value.contains(['\u{2044}', '\u{2215}', '\u{29f8}', '\u{ff0f}', '\u{ff3c}'])
        || value.chars().any(is_ambiguous_format_character)
    {
        return Err(PrivacyShapingError::UnsupportedPathObservation);
    }
    let mut normalized = value.to_owned();
    normalized.make_ascii_lowercase();
    if normalized.contains("%2f")
        || normalized.contains("%5c")
        || normalized.contains("%252f")
        || normalized.contains("%255c")
        || normalized.contains("://")
        || normalized.contains(':')
    {
        return Err(PrivacyShapingError::UnsupportedPathObservation);
    }
    Ok(normalized)
}

fn is_ambiguous_format_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{feff}'
        )
}

fn is_windows_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn is_windows_drive_root(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn classify_normalized_path(components: &[String]) -> Option<SensitiveCategory> {
    let first = components.first().map(String::as_str);
    if first.is_some_and(|value| {
        matches!(
            value,
            "system"
                | "bin"
                | "sbin"
                | "usr"
                | "private"
                | "dev"
                | "proc"
                | "etc"
                | "var"
                | "boot"
                | "root"
                | "run"
                | "sys"
                | "lib"
                | "lib64"
                | "lib32"
                | "libx32"
                | "opt"
                | "srv"
                | "nix"
                | "snap"
                | "lost+found"
                | "cores"
                | "network"
                | "net"
                | "windows"
                | "program files"
                | "program files (x86)"
                | "programdata"
                | "$recycle.bin"
                | "system volume information"
                | "recovery"
                | "perflogs"
                | "library"
                | "applications"
        )
    }) || components.len() == 1
        && matches!(
            components[0].as_str(),
            "users" | "volumes" | "home" | "mnt" | "media" | "tmp"
        )
    {
        return Some(SensitiveCategory::ProtectedSystem);
    }
    if is_guarded_user_data_root(components) {
        return Some(SensitiveCategory::ProtectedSystem);
    }
    let mut previous = None;
    for value in components {
        if let Some(category) = classify_sensitive_component(previous, value) {
            return Some(category);
        }
        previous = Some(value.as_str());
    }
    None
}

fn is_guarded_user_data_root(components: &[String]) -> bool {
    components
        .last()
        .is_some_and(|value| matches!(value.as_str(), "library" | "appdata"))
}

fn classify_sensitive_component(previous: Option<&str>, value: &str) -> Option<SensitiveCategory> {
    if credential_component(value) {
        return Some(SensitiveCategory::CredentialOrToken);
    }
    if matches!(value, "keychain" | "keychains") || value.ends_with(".keychain-db") {
        return Some(SensitiveCategory::Keychain);
    }
    if matches!(
        value,
        "safari" | "chrome" | "chromium" | "firefox" | "bravesoftware" | "microsoft edge" | "arc"
    ) || value.starts_with("com.apple.safari")
        || value.starts_with("com.google.chrome")
        || value.starts_with("org.mozilla.firefox")
        || value.starts_with("company.thebrowser.browser")
        || matches!(
            (previous, value),
            (Some("google"), "chrome") | (Some("mozilla"), "firefox")
        )
    {
        return Some(SensitiveCategory::BrowserProfile);
    }
    if matches!(value, "messages" | "mail" | "notes")
        || value.starts_with("com.apple.mobilemail")
        || value.starts_with("com.apple.mail")
        || value.starts_with("group.com.apple.mail")
        || value.starts_with("com.apple.notes")
        || value.starts_with("com.apple.mobilenotes")
        || value.starts_with("group.com.apple.notes")
        || value.starts_with("com.apple.messages")
        || value.starts_with("com.apple.mobilesms")
    {
        return Some(SensitiveCategory::PrivateCommunication);
    }
    if [
        "1password",
        "bitwarden",
        "keepass",
        "lastpass",
        "dashlane",
        "enpass",
    ]
    .iter()
    .any(|product| value.contains(product))
    {
        return Some(SensitiveCategory::PasswordManager);
    }
    if [
        "little snitch",
        "crowdstrike",
        "sentinelone",
        "carbon black",
        "jamf",
        "microsoft defender",
        "endpoint security",
    ]
    .iter()
    .any(|product| value.contains(product))
    {
        return Some(SensitiveCategory::SecurityOrManagement);
    }
    if matches!(
        value,
        "parallels"
            | "utm"
            | "virtualbox vms"
            | "virtual machines.localized"
            | "vmware"
            | "colima"
            | "lima"
            | "podman"
            | ".minikube"
            | "docker.raw"
            | "docker.qcow2"
    ) || [
        ".vmdk", ".vdi", ".vhd", ".vhdx", ".avhdx", ".qcow", ".qcow2", ".hdd", ".pvm", ".utm",
    ]
    .iter()
    .any(|extension| value.ends_with(extension))
    {
        return Some(SensitiveCategory::VirtualMachineOrContainer);
    }
    if cloud_document_component(previous, value) {
        return Some(SensitiveCategory::CloudDocument);
    }
    None
}

fn cloud_document_component(previous: Option<&str>, value: &str) -> bool {
    matches!(
        value,
        "mobile documents"
            | "cloudstorage"
            | "clouddocs"
            | "icloud"
            | "icloud drive"
            | "dropbox"
            | "google drive"
            | "box"
            | "box drive"
            | "box sync"
            | "pcloud drive"
            | "megasync"
            | "nextcloud"
            | "owncloud"
            | "proton drive"
            | "synology drive"
            | "tresorit"
            | "sync.com"
            | "idrive"
            | "creative cloud files"
            | ".icloud"
    ) || value.starts_with("com~apple~clouddocs")
        || value.starts_with("icloud")
        || value.starts_with("onedrive")
        || value.ends_with(".icloud")
        || matches!((previous, value), (Some("google"), "drivefs"))
}

fn credential_component(value: &str) -> bool {
    if matches!(
        value,
        ".ssh"
            | ".gnupg"
            | ".aws"
            | ".azure"
            | ".kube"
            | ".docker"
            | ".password-store"
            | ".netrc"
            | ".npmrc"
            | ".pypirc"
            | ".env"
            | ".envrc"
            | ".config"
            | "credentials"
            | "credentials.json"
            | "secrets"
            | "secrets.json"
            | "tokens"
            | "tokens.json"
            | "auth.json"
            | "id_rsa"
            | "id_dsa"
            | "id_ecdsa"
            | "id_ed25519"
    ) || value.starts_with(".env.")
        || [".pem", ".key", ".p12", ".pfx"]
            .iter()
            .any(|extension| value.ends_with(extension))
    {
        return true;
    }
    let mut previous = None;
    for word in value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        if matches!(
            word,
            "auth"
                | "apikey"
                | "accesstoken"
                | "authorization"
                | "bearer"
                | "cookie"
                | "cookies"
                | "credential"
                | "credentials"
                | "clientsecret"
                | "oauth"
                | "passwd"
                | "password"
                | "privatekey"
                | "refreshtoken"
                | "secret"
                | "secrets"
                | "session"
                | "token"
                | "tokens"
        ) || matches!(
            (previous, word),
            (Some("api"), "key")
                | (Some("access"), "token")
                | (Some("refresh"), "token")
                | (Some("client"), "secret")
                | (Some("private"), "key")
        ) {
            return true;
        }
        previous = Some(word);
    }
    false
}

fn ensure_complete_observation(node: &SnapshotNode) -> Result<(), PrivacyShapingError> {
    if node.kind == SnapshotNodeKind::Error
        || node.scan_flags.contains(SnapshotScanFlags::INACCESSIBLE)
        || node.scan_flags.contains(SnapshotScanFlags::TIMED_OUT)
        || node.scan_flags.contains(SnapshotScanFlags::MOUNT_BOUNDARY)
    {
        return Err(PrivacyShapingError::IncompleteObservation);
    }
    Ok(())
}

fn contract_kind(kind: SnapshotNodeKind) -> Result<AiInputNodeKindV1, PrivacyShapingError> {
    match kind {
        SnapshotNodeKind::Directory => Ok(AiInputNodeKindV1::Directory),
        SnapshotNodeKind::File => Ok(AiInputNodeKindV1::File),
        SnapshotNodeKind::Symlink => Ok(AiInputNodeKindV1::Symlink),
        SnapshotNodeKind::Other => Ok(AiInputNodeKindV1::Other),
        SnapshotNodeKind::Error => Err(PrivacyShapingError::IncompleteObservation),
    }
}

fn generic_child_label(kind: AiInputNodeKindV1, ordinal: usize) -> String {
    let kind = match kind {
        AiInputNodeKindV1::Directory => "Directory",
        AiInputNodeKindV1::File => "File",
        AiInputNodeKindV1::Symlink => "Symbolic link",
        AiInputNodeKindV1::Other => "Other item",
        AiInputNodeKindV1::Unavailable => "Unavailable item",
    };
    format!("{kind} {ordinal}")
}

fn age_bucket(modified_at: Option<SnapshotTimestamp>, observed_at: SnapshotTimestamp) -> usize {
    let Some(modified_at) = modified_at else {
        return 4;
    };
    let observed = (
        observed_at.seconds_since_unix_epoch(),
        observed_at.nanoseconds(),
    );
    let modified = (
        modified_at.seconds_since_unix_epoch(),
        modified_at.nanoseconds(),
    );
    if modified > observed {
        return 4;
    }
    let mut seconds = observed.0 - modified.0;
    if modified.1 > observed.1 {
        seconds = seconds.saturating_sub(1);
    }
    if seconds <= 7 * SECONDS_PER_DAY {
        0
    } else if seconds <= 30 * SECONDS_PER_DAY {
        1
    } else if seconds <= 90 * SECONDS_PER_DAY {
        2
    } else {
        3
    }
}

#[cfg(test)]
#[path = "privacy_tests.rs"]
mod tests;
