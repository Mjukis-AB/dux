//! Deterministic policy for non-destructive cloud local-copy eviction.
//!
//! Platform adapters may report only these bounded facts. They do not decide
//! eligibility and cannot create a candidate, plan, approval, or effect.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Providers with a reviewed, supported local-copy eviction API.
///
/// The first policy revision intentionally admits only Foundation's iCloud
/// ubiquitous-item API. Adding another provider requires a new reviewed enum
/// case and adapter; a caller cannot supply a provider name or command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudEvictionProvider {
    ICloudDrive,
}

/// A raw Boolean platform fact. Missing resource values remain `Unknown`;
/// clients must never turn them into a favorable default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudBooleanState {
    True,
    False,
    Unknown,
}

impl CloudBooleanState {
    pub const fn from_optional(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::True,
            Some(false) => Self::False,
            None => Self::Unknown,
        }
    }
}

/// Whether a requested Foundation error value was present.
///
/// `Unknown` is distinct from a successfully read `nil` value so a failed or
/// unsupported lookup cannot become favorable evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudErrorState {
    Absent,
    Present,
    Unknown,
}

/// No-follow item kind observed by the platform adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudEvictionItemKind {
    RegularFile,
    Directory,
    Symlink,
    Other,
    Unknown,
}

/// Foundation's local iCloud copy state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudLocalCopyState {
    Current,
    Stale,
    NotDownloaded,
    Unknown,
}

/// Whether one provider/account/container/item identity prerequisite stayed
/// stable across a bracketed platform read.
///
/// A stable value is still only capability evidence. It cannot be persisted as
/// cleanup authority or reused in place of a fresh eligibility probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudIdentityFactState {
    Stable,
    Unavailable,
    ChangedDuringRead,
    Unsupported,
}

/// Provider identity prerequisites observed around one metadata read.
///
/// `container` is the File Provider domain identity and `provider_item` is the
/// provider-owned item identity. Keeping them separate prevents a display
/// name, path component, or item-generation token from silently becoming
/// container or item authority later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloudEvictionIdentityFacts {
    pub account: CloudIdentityFactState,
    pub container: CloudIdentityFactState,
    pub provider_item: CloudIdentityFactState,
    pub item_generation: CloudIdentityFactState,
    pub file_version: CloudIdentityFactState,
    pub shared: CloudBooleanState,
    pub sync_paused: CloudBooleanState,
}

impl CloudEvictionIdentityFacts {
    pub const fn unsupported() -> Self {
        Self {
            account: CloudIdentityFactState::Unavailable,
            container: CloudIdentityFactState::Unsupported,
            provider_item: CloudIdentityFactState::Unsupported,
            item_generation: CloudIdentityFactState::Unavailable,
            file_version: CloudIdentityFactState::Unavailable,
            shared: CloudBooleanState::Unknown,
            sync_paused: CloudBooleanState::Unknown,
        }
    }
}

/// The complete set of facts a platform metadata adapter may report.
///
/// Provider, item kind, allocation, target identity, path, and observation
/// time are intentionally absent; the engine owns those facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloudEvictionPlatformFacts {
    pub ubiquitous: CloudBooleanState,
    pub uploaded: CloudBooleanState,
    pub uploading: CloudBooleanState,
    pub upload_error: CloudErrorState,
    pub unresolved_conflicts: CloudBooleanState,
    pub local_copy_state: CloudLocalCopyState,
    pub download_requested: CloudBooleanState,
    pub downloading: CloudBooleanState,
    pub download_error: CloudErrorState,
    pub excluded_from_sync: CloudBooleanState,
    pub identity: CloudEvictionIdentityFacts,
}

/// Raw, non-authoritative facts captured together for one core-selected item.
///
/// This value deliberately contains no path. A later one-shot engine boundary
/// must bind it to the exact retained no-follow target that requested the
/// platform probe, and must re-probe before any effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloudEvictionObservation {
    provider: CloudEvictionProvider,
    item_kind: CloudEvictionItemKind,
    ubiquitous: CloudBooleanState,
    uploaded: CloudBooleanState,
    uploading: CloudBooleanState,
    upload_error: CloudErrorState,
    unresolved_conflicts: CloudBooleanState,
    local_copy_state: CloudLocalCopyState,
    download_requested: CloudBooleanState,
    downloading: CloudBooleanState,
    download_error: CloudErrorState,
    excluded_from_sync: CloudBooleanState,
    identity: CloudEvictionIdentityFacts,
    local_allocated_bytes: Option<u64>,
    observed_at: SystemTime,
}

/// Named input for one raw observation. Explicit field names keep opposite
/// favorable Boolean polarities from becoming an unsafe positional call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloudEvictionObservationInput {
    pub provider: CloudEvictionProvider,
    pub item_kind: CloudEvictionItemKind,
    pub ubiquitous: CloudBooleanState,
    pub uploaded: CloudBooleanState,
    pub uploading: CloudBooleanState,
    pub upload_error: CloudErrorState,
    pub unresolved_conflicts: CloudBooleanState,
    pub local_copy_state: CloudLocalCopyState,
    pub download_requested: CloudBooleanState,
    pub downloading: CloudBooleanState,
    pub download_error: CloudErrorState,
    pub excluded_from_sync: CloudBooleanState,
    pub identity: CloudEvictionIdentityFacts,
    pub local_allocated_bytes: Option<u64>,
    pub observed_at: SystemTime,
}

impl CloudEvictionObservation {
    pub const fn new(input: CloudEvictionObservationInput) -> Self {
        Self {
            provider: input.provider,
            item_kind: input.item_kind,
            ubiquitous: input.ubiquitous,
            uploaded: input.uploaded,
            uploading: input.uploading,
            upload_error: input.upload_error,
            unresolved_conflicts: input.unresolved_conflicts,
            local_copy_state: input.local_copy_state,
            download_requested: input.download_requested,
            downloading: input.downloading,
            download_error: input.download_error,
            excluded_from_sync: input.excluded_from_sync,
            identity: input.identity,
            local_allocated_bytes: input.local_allocated_bytes,
            observed_at: input.observed_at,
        }
    }

    pub const fn provider(&self) -> CloudEvictionProvider {
        self.provider
    }

    pub const fn item_kind(&self) -> CloudEvictionItemKind {
        self.item_kind
    }

    pub const fn ubiquitous(&self) -> CloudBooleanState {
        self.ubiquitous
    }

    pub const fn uploaded(&self) -> CloudBooleanState {
        self.uploaded
    }

    pub const fn uploading(&self) -> CloudBooleanState {
        self.uploading
    }

    pub const fn upload_error(&self) -> CloudErrorState {
        self.upload_error
    }

    pub const fn unresolved_conflicts(&self) -> CloudBooleanState {
        self.unresolved_conflicts
    }

    pub const fn local_copy_state(&self) -> CloudLocalCopyState {
        self.local_copy_state
    }

    pub const fn download_requested(&self) -> CloudBooleanState {
        self.download_requested
    }

    pub const fn downloading(&self) -> CloudBooleanState {
        self.downloading
    }

    pub const fn download_error(&self) -> CloudErrorState {
        self.download_error
    }

    pub const fn excluded_from_sync(&self) -> CloudBooleanState {
        self.excluded_from_sync
    }

    pub const fn identity(&self) -> CloudEvictionIdentityFacts {
        self.identity
    }

    pub const fn local_allocated_bytes(&self) -> Option<u64> {
        self.local_allocated_bytes
    }

    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }
}

/// Fixed, independently presentable reasons an observation cannot support a
/// local-copy eviction candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudEvictionBlockReason {
    UnsupportedItemKind,
    UbiquityUnknown,
    NotUbiquitous,
    UploadStateUnknown,
    UploadIncomplete,
    UploadActivityUnknown,
    UploadInProgress,
    UploadErrorUnknown,
    UploadErrorPresent,
    ConflictStateUnknown,
    UnresolvedConflicts,
    LocalCopyStateUnknown,
    StaleLocalCopy,
    NoLocalCopy,
    DownloadRequestUnknown,
    DownloadRequested,
    DownloadActivityUnknown,
    DownloadInProgress,
    DownloadErrorUnknown,
    DownloadErrorPresent,
    SyncExclusionUnknown,
    ExcludedFromSync,
    AllocationUnknown,
    NoLocalAllocation,
    InvalidObservationTime,
}

/// Fixed reasons the current platform capture cannot become durable identity
/// evidence for candidate admission.
///
/// These are deliberately separate from [`CloudEvictionBlockReason`]: a file
/// may have favorable point-in-time sync metadata while still lacking the
/// independently versioned account/container/item binding required by ADR
/// 0006.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudEvictionIdentityBlockReason {
    AccountIdentityUnavailable,
    AccountIdentityChanged,
    AccountIdentityUnsupported,
    ContainerIdentityUnavailable,
    ContainerIdentityChanged,
    ContainerIdentityUnsupported,
    ProviderItemIdentityUnavailable,
    ProviderItemIdentityChanged,
    ProviderItemIdentityUnsupported,
    ItemGenerationUnavailable,
    ItemGenerationChanged,
    ItemGenerationUnsupported,
    FileVersionUnavailable,
    FileVersionChanged,
    FileVersionUnsupported,
    SharedStateUnknown,
    SharedItem,
    SyncPausedStateUnknown,
    SyncPaused,
}

/// The complete deterministic assessment of one raw platform observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudEvictionAssessment {
    observation: CloudEvictionObservation,
    blockers: Arc<[CloudEvictionBlockReason]>,
    identity_blockers: Arc<[CloudEvictionIdentityBlockReason]>,
    discovery_evidence: Option<CloudEvictionDiscoveryEvidence>,
}

impl CloudEvictionAssessment {
    pub const fn observation(&self) -> &CloudEvictionObservation {
        &self.observation
    }

    pub fn blockers(&self) -> &[CloudEvictionBlockReason] {
        &self.blockers
    }

    pub fn identity_blockers(&self) -> &[CloudEvictionIdentityBlockReason] {
        &self.identity_blockers
    }

    pub const fn discovery_evidence(&self) -> Option<&CloudEvictionDiscoveryEvidence> {
        self.discovery_evidence.as_ref()
    }

    pub const fn is_eligible_observation(&self) -> bool {
        self.discovery_evidence.is_some()
    }

    /// Whether every separately required durable-identity prerequisite was
    /// stable during this read.
    ///
    /// Even `true` is not a candidate or reusable proof.
    pub fn is_identity_ready(&self) -> bool {
        self.identity_blockers.is_empty()
    }
}

/// Path-free evidence that all currently required platform facts were present.
///
/// This remains an observation, not a candidate or effect capability. It is
/// intentionally constructible only by [`assess_cloud_eviction`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloudEvictionDiscoveryEvidence {
    provider: CloudEvictionProvider,
    local_allocated_bytes: u64,
    observed_at: SystemTime,
}

impl CloudEvictionDiscoveryEvidence {
    pub const fn provider(&self) -> CloudEvictionProvider {
        self.provider
    }

    pub const fn local_allocated_bytes(&self) -> u64 {
        self.local_allocated_bytes
    }

    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }
}

/// Evaluate one platform observation with a fixed fail-closed order.
///
/// The complete point-in-time metadata predicate is required. A passing result
/// is useful only for discovery: Apple describes `Current` as the newest copy
/// known to this device, so this does not prove absence of unflushed or
/// concurrent local changes and cannot authorize an effect.
pub fn assess_cloud_eviction(observation: CloudEvictionObservation) -> CloudEvictionAssessment {
    let mut blockers = Vec::new();
    match observation.provider {
        CloudEvictionProvider::ICloudDrive => {}
    }
    if observation.item_kind != CloudEvictionItemKind::RegularFile {
        blockers.push(CloudEvictionBlockReason::UnsupportedItemKind);
    }
    match observation.ubiquitous {
        CloudBooleanState::True => {}
        CloudBooleanState::False => blockers.push(CloudEvictionBlockReason::NotUbiquitous),
        CloudBooleanState::Unknown => blockers.push(CloudEvictionBlockReason::UbiquityUnknown),
    }
    match observation.uploaded {
        CloudBooleanState::True => {}
        CloudBooleanState::False => blockers.push(CloudEvictionBlockReason::UploadIncomplete),
        CloudBooleanState::Unknown => blockers.push(CloudEvictionBlockReason::UploadStateUnknown),
    }
    match observation.uploading {
        CloudBooleanState::False => {}
        CloudBooleanState::True => blockers.push(CloudEvictionBlockReason::UploadInProgress),
        CloudBooleanState::Unknown => {
            blockers.push(CloudEvictionBlockReason::UploadActivityUnknown);
        }
    }
    match observation.upload_error {
        CloudErrorState::Absent => {}
        CloudErrorState::Present => blockers.push(CloudEvictionBlockReason::UploadErrorPresent),
        CloudErrorState::Unknown => blockers.push(CloudEvictionBlockReason::UploadErrorUnknown),
    }
    match observation.unresolved_conflicts {
        CloudBooleanState::False => {}
        CloudBooleanState::True => blockers.push(CloudEvictionBlockReason::UnresolvedConflicts),
        CloudBooleanState::Unknown => {
            blockers.push(CloudEvictionBlockReason::ConflictStateUnknown);
        }
    }
    match observation.local_copy_state {
        CloudLocalCopyState::Current => {}
        CloudLocalCopyState::Stale => blockers.push(CloudEvictionBlockReason::StaleLocalCopy),
        CloudLocalCopyState::NotDownloaded => blockers.push(CloudEvictionBlockReason::NoLocalCopy),
        CloudLocalCopyState::Unknown => {
            blockers.push(CloudEvictionBlockReason::LocalCopyStateUnknown);
        }
    }
    match observation.download_requested {
        CloudBooleanState::False => {}
        CloudBooleanState::True => blockers.push(CloudEvictionBlockReason::DownloadRequested),
        CloudBooleanState::Unknown => {
            blockers.push(CloudEvictionBlockReason::DownloadRequestUnknown);
        }
    }
    match observation.downloading {
        CloudBooleanState::False => {}
        CloudBooleanState::True => blockers.push(CloudEvictionBlockReason::DownloadInProgress),
        CloudBooleanState::Unknown => {
            blockers.push(CloudEvictionBlockReason::DownloadActivityUnknown);
        }
    }
    match observation.download_error {
        CloudErrorState::Absent => {}
        CloudErrorState::Present => blockers.push(CloudEvictionBlockReason::DownloadErrorPresent),
        CloudErrorState::Unknown => blockers.push(CloudEvictionBlockReason::DownloadErrorUnknown),
    }
    match observation.excluded_from_sync {
        CloudBooleanState::False => {}
        CloudBooleanState::True => blockers.push(CloudEvictionBlockReason::ExcludedFromSync),
        CloudBooleanState::Unknown => {
            blockers.push(CloudEvictionBlockReason::SyncExclusionUnknown);
        }
    }
    match observation.local_allocated_bytes {
        Some(0) => blockers.push(CloudEvictionBlockReason::NoLocalAllocation),
        Some(_) => {}
        None => blockers.push(CloudEvictionBlockReason::AllocationUnknown),
    }
    if observation.observed_at.duration_since(UNIX_EPOCH).is_err() {
        blockers.push(CloudEvictionBlockReason::InvalidObservationTime);
    }

    let discovery_evidence = blockers.is_empty().then(|| CloudEvictionDiscoveryEvidence {
        provider: observation.provider,
        local_allocated_bytes: observation
            .local_allocated_bytes
            .expect("checked nonzero allocation"),
        observed_at: observation.observed_at,
    });
    let identity_blockers = assess_identity_facts(observation.identity);
    CloudEvictionAssessment {
        observation,
        blockers: blockers.into(),
        identity_blockers: identity_blockers.into(),
        discovery_evidence,
    }
}

fn assess_identity_facts(
    identity: CloudEvictionIdentityFacts,
) -> Vec<CloudEvictionIdentityBlockReason> {
    let mut blockers = Vec::new();
    assess_identity_fact(
        identity.account,
        CloudEvictionIdentityBlockReason::AccountIdentityUnavailable,
        CloudEvictionIdentityBlockReason::AccountIdentityChanged,
        CloudEvictionIdentityBlockReason::AccountIdentityUnsupported,
        &mut blockers,
    );
    assess_identity_fact(
        identity.container,
        CloudEvictionIdentityBlockReason::ContainerIdentityUnavailable,
        CloudEvictionIdentityBlockReason::ContainerIdentityChanged,
        CloudEvictionIdentityBlockReason::ContainerIdentityUnsupported,
        &mut blockers,
    );
    assess_identity_fact(
        identity.provider_item,
        CloudEvictionIdentityBlockReason::ProviderItemIdentityUnavailable,
        CloudEvictionIdentityBlockReason::ProviderItemIdentityChanged,
        CloudEvictionIdentityBlockReason::ProviderItemIdentityUnsupported,
        &mut blockers,
    );
    assess_identity_fact(
        identity.item_generation,
        CloudEvictionIdentityBlockReason::ItemGenerationUnavailable,
        CloudEvictionIdentityBlockReason::ItemGenerationChanged,
        CloudEvictionIdentityBlockReason::ItemGenerationUnsupported,
        &mut blockers,
    );
    assess_identity_fact(
        identity.file_version,
        CloudEvictionIdentityBlockReason::FileVersionUnavailable,
        CloudEvictionIdentityBlockReason::FileVersionChanged,
        CloudEvictionIdentityBlockReason::FileVersionUnsupported,
        &mut blockers,
    );
    match identity.shared {
        CloudBooleanState::False => {}
        CloudBooleanState::True => blockers.push(CloudEvictionIdentityBlockReason::SharedItem),
        CloudBooleanState::Unknown => {
            blockers.push(CloudEvictionIdentityBlockReason::SharedStateUnknown);
        }
    }
    match identity.sync_paused {
        CloudBooleanState::False => {}
        CloudBooleanState::True => blockers.push(CloudEvictionIdentityBlockReason::SyncPaused),
        CloudBooleanState::Unknown => {
            blockers.push(CloudEvictionIdentityBlockReason::SyncPausedStateUnknown);
        }
    }
    blockers
}

fn assess_identity_fact(
    state: CloudIdentityFactState,
    unavailable: CloudEvictionIdentityBlockReason,
    changed: CloudEvictionIdentityBlockReason,
    unsupported: CloudEvictionIdentityBlockReason,
    blockers: &mut Vec<CloudEvictionIdentityBlockReason>,
) {
    match state {
        CloudIdentityFactState::Stable => {}
        CloudIdentityFactState::Unavailable => blockers.push(unavailable),
        CloudIdentityFactState::ChangedDuringRead => blockers.push(changed),
        CloudIdentityFactState::Unsupported => blockers.push(unsupported),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn eligible_observation() -> CloudEvictionObservation {
        CloudEvictionObservation::new(CloudEvictionObservationInput {
            provider: CloudEvictionProvider::ICloudDrive,
            item_kind: CloudEvictionItemKind::RegularFile,
            ubiquitous: CloudBooleanState::True,
            uploaded: CloudBooleanState::True,
            uploading: CloudBooleanState::False,
            upload_error: CloudErrorState::Absent,
            unresolved_conflicts: CloudBooleanState::False,
            local_copy_state: CloudLocalCopyState::Current,
            download_requested: CloudBooleanState::False,
            downloading: CloudBooleanState::False,
            download_error: CloudErrorState::Absent,
            excluded_from_sync: CloudBooleanState::False,
            identity: CloudEvictionIdentityFacts {
                account: CloudIdentityFactState::Stable,
                container: CloudIdentityFactState::Stable,
                provider_item: CloudIdentityFactState::Stable,
                item_generation: CloudIdentityFactState::Stable,
                file_version: CloudIdentityFactState::Stable,
                shared: CloudBooleanState::False,
                sync_paused: CloudBooleanState::False,
            },
            local_allocated_bytes: Some(4096),
            observed_at: UNIX_EPOCH + Duration::from_secs(10),
        })
    }

    #[test]
    fn complete_current_regular_file_observation_produces_discovery_evidence_only() {
        let assessment = assess_cloud_eviction(eligible_observation());
        assert!(assessment.is_eligible_observation());
        assert!(assessment.blockers().is_empty());
        let evidence = assessment.discovery_evidence().unwrap();
        assert_eq!(evidence.provider(), CloudEvictionProvider::ICloudDrive);
        assert_eq!(evidence.local_allocated_bytes(), 4096);
        assert_eq!(evidence.observed_at(), UNIX_EPOCH + Duration::from_secs(10));
    }

    #[test]
    fn every_required_platform_fact_fails_closed_independently() {
        let cases = vec![
            (
                CloudEvictionObservation {
                    item_kind: CloudEvictionItemKind::Directory,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UnsupportedItemKind,
            ),
            (
                CloudEvictionObservation {
                    item_kind: CloudEvictionItemKind::Symlink,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UnsupportedItemKind,
            ),
            (
                CloudEvictionObservation {
                    item_kind: CloudEvictionItemKind::Other,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UnsupportedItemKind,
            ),
            (
                CloudEvictionObservation {
                    item_kind: CloudEvictionItemKind::Unknown,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UnsupportedItemKind,
            ),
            (
                CloudEvictionObservation {
                    ubiquitous: CloudBooleanState::Unknown,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UbiquityUnknown,
            ),
            (
                CloudEvictionObservation {
                    ubiquitous: CloudBooleanState::False,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::NotUbiquitous,
            ),
            (
                CloudEvictionObservation {
                    uploaded: CloudBooleanState::Unknown,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UploadStateUnknown,
            ),
            (
                CloudEvictionObservation {
                    uploaded: CloudBooleanState::False,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UploadIncomplete,
            ),
            (
                CloudEvictionObservation {
                    uploading: CloudBooleanState::Unknown,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UploadActivityUnknown,
            ),
            (
                CloudEvictionObservation {
                    uploading: CloudBooleanState::True,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UploadInProgress,
            ),
            (
                CloudEvictionObservation {
                    upload_error: CloudErrorState::Unknown,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UploadErrorUnknown,
            ),
            (
                CloudEvictionObservation {
                    upload_error: CloudErrorState::Present,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UploadErrorPresent,
            ),
            (
                CloudEvictionObservation {
                    unresolved_conflicts: CloudBooleanState::Unknown,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::ConflictStateUnknown,
            ),
            (
                CloudEvictionObservation {
                    unresolved_conflicts: CloudBooleanState::True,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::UnresolvedConflicts,
            ),
            (
                CloudEvictionObservation {
                    local_copy_state: CloudLocalCopyState::Unknown,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::LocalCopyStateUnknown,
            ),
            (
                CloudEvictionObservation {
                    local_copy_state: CloudLocalCopyState::Stale,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::StaleLocalCopy,
            ),
            (
                CloudEvictionObservation {
                    local_copy_state: CloudLocalCopyState::NotDownloaded,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::NoLocalCopy,
            ),
            (
                CloudEvictionObservation {
                    download_requested: CloudBooleanState::Unknown,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::DownloadRequestUnknown,
            ),
            (
                CloudEvictionObservation {
                    download_requested: CloudBooleanState::True,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::DownloadRequested,
            ),
            (
                CloudEvictionObservation {
                    downloading: CloudBooleanState::Unknown,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::DownloadActivityUnknown,
            ),
            (
                CloudEvictionObservation {
                    downloading: CloudBooleanState::True,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::DownloadInProgress,
            ),
            (
                CloudEvictionObservation {
                    download_error: CloudErrorState::Unknown,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::DownloadErrorUnknown,
            ),
            (
                CloudEvictionObservation {
                    download_error: CloudErrorState::Present,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::DownloadErrorPresent,
            ),
            (
                CloudEvictionObservation {
                    excluded_from_sync: CloudBooleanState::Unknown,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::SyncExclusionUnknown,
            ),
            (
                CloudEvictionObservation {
                    excluded_from_sync: CloudBooleanState::True,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::ExcludedFromSync,
            ),
            (
                CloudEvictionObservation {
                    local_allocated_bytes: None,
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::AllocationUnknown,
            ),
            (
                CloudEvictionObservation {
                    local_allocated_bytes: Some(0),
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::NoLocalAllocation,
            ),
            (
                CloudEvictionObservation {
                    observed_at: UNIX_EPOCH - Duration::from_nanos(1),
                    ..eligible_observation()
                },
                CloudEvictionBlockReason::InvalidObservationTime,
            ),
        ];

        for (observation, expected) in cases {
            let assessment = assess_cloud_eviction(observation);
            assert!(!assessment.is_eligible_observation());
            assert_eq!(assessment.blockers(), &[expected]);
            assert_eq!(assessment.discovery_evidence(), None);
        }
    }

    #[test]
    fn multiple_failures_remain_separate_in_fixed_order() {
        let assessment = assess_cloud_eviction(CloudEvictionObservation::new(
            CloudEvictionObservationInput {
                provider: CloudEvictionProvider::ICloudDrive,
                item_kind: CloudEvictionItemKind::Symlink,
                ubiquitous: CloudBooleanState::Unknown,
                uploaded: CloudBooleanState::False,
                uploading: CloudBooleanState::True,
                upload_error: CloudErrorState::Present,
                unresolved_conflicts: CloudBooleanState::True,
                local_copy_state: CloudLocalCopyState::NotDownloaded,
                download_requested: CloudBooleanState::True,
                downloading: CloudBooleanState::True,
                download_error: CloudErrorState::Present,
                excluded_from_sync: CloudBooleanState::True,
                identity: CloudEvictionIdentityFacts::unsupported(),
                local_allocated_bytes: Some(0),
                observed_at: UNIX_EPOCH,
            },
        ));
        assert_eq!(
            assessment.blockers(),
            &[
                CloudEvictionBlockReason::UnsupportedItemKind,
                CloudEvictionBlockReason::UbiquityUnknown,
                CloudEvictionBlockReason::UploadIncomplete,
                CloudEvictionBlockReason::UploadInProgress,
                CloudEvictionBlockReason::UploadErrorPresent,
                CloudEvictionBlockReason::UnresolvedConflicts,
                CloudEvictionBlockReason::NoLocalCopy,
                CloudEvictionBlockReason::DownloadRequested,
                CloudEvictionBlockReason::DownloadInProgress,
                CloudEvictionBlockReason::DownloadErrorPresent,
                CloudEvictionBlockReason::ExcludedFromSync,
                CloudEvictionBlockReason::NoLocalAllocation,
            ]
        );
    }

    #[test]
    fn optional_boolean_values_preserve_polarity_and_unknown() {
        assert_eq!(
            CloudBooleanState::from_optional(Some(true)),
            CloudBooleanState::True
        );
        assert_eq!(
            CloudBooleanState::from_optional(Some(false)),
            CloudBooleanState::False
        );
        assert_eq!(
            CloudBooleanState::from_optional(None),
            CloudBooleanState::Unknown
        );
    }

    #[test]
    fn smallest_nonzero_allocation_and_epoch_are_discovery_eligible() {
        let assessment = assess_cloud_eviction(CloudEvictionObservation {
            local_allocated_bytes: Some(1),
            observed_at: UNIX_EPOCH,
            ..eligible_observation()
        });
        assert!(assessment.is_eligible_observation());
        assert_eq!(
            assessment
                .discovery_evidence()
                .unwrap()
                .local_allocated_bytes(),
            1
        );
    }

    #[test]
    fn identity_readiness_is_independent_from_favorable_sync_metadata() {
        let assessment = assess_cloud_eviction(CloudEvictionObservation {
            identity: CloudEvictionIdentityFacts::unsupported(),
            ..eligible_observation()
        });

        assert!(assessment.is_eligible_observation());
        assert!(!assessment.is_identity_ready());
        assert_eq!(
            assessment.identity_blockers(),
            &[
                CloudEvictionIdentityBlockReason::AccountIdentityUnavailable,
                CloudEvictionIdentityBlockReason::ContainerIdentityUnsupported,
                CloudEvictionIdentityBlockReason::ProviderItemIdentityUnsupported,
                CloudEvictionIdentityBlockReason::ItemGenerationUnavailable,
                CloudEvictionIdentityBlockReason::FileVersionUnavailable,
                CloudEvictionIdentityBlockReason::SharedStateUnknown,
                CloudEvictionIdentityBlockReason::SyncPausedStateUnknown,
            ]
        );
    }

    #[test]
    fn every_identity_fact_fails_closed_in_fixed_order() {
        let cases = [
            (
                CloudIdentityFactState::Unavailable,
                CloudEvictionIdentityBlockReason::AccountIdentityUnavailable,
            ),
            (
                CloudIdentityFactState::ChangedDuringRead,
                CloudEvictionIdentityBlockReason::AccountIdentityChanged,
            ),
            (
                CloudIdentityFactState::Unsupported,
                CloudEvictionIdentityBlockReason::AccountIdentityUnsupported,
            ),
        ];
        for (state, expected) in cases {
            let assessment = assess_cloud_eviction(CloudEvictionObservation {
                identity: CloudEvictionIdentityFacts {
                    account: state,
                    ..eligible_observation().identity()
                },
                ..eligible_observation()
            });
            assert_eq!(assessment.identity_blockers(), &[expected]);
            assert!(!assessment.is_identity_ready());
        }

        for (state, expected) in [
            (
                CloudIdentityFactState::Unavailable,
                CloudEvictionIdentityBlockReason::ProviderItemIdentityUnavailable,
            ),
            (
                CloudIdentityFactState::ChangedDuringRead,
                CloudEvictionIdentityBlockReason::ProviderItemIdentityChanged,
            ),
            (
                CloudIdentityFactState::Unsupported,
                CloudEvictionIdentityBlockReason::ProviderItemIdentityUnsupported,
            ),
        ] {
            let assessment = assess_cloud_eviction(CloudEvictionObservation {
                identity: CloudEvictionIdentityFacts {
                    provider_item: state,
                    ..eligible_observation().identity()
                },
                ..eligible_observation()
            });
            assert_eq!(assessment.identity_blockers(), &[expected]);
            assert!(!assessment.is_identity_ready());
        }

        let assessment = assess_cloud_eviction(CloudEvictionObservation {
            identity: CloudEvictionIdentityFacts {
                account: CloudIdentityFactState::Unavailable,
                container: CloudIdentityFactState::ChangedDuringRead,
                provider_item: CloudIdentityFactState::Unsupported,
                item_generation: CloudIdentityFactState::Unsupported,
                file_version: CloudIdentityFactState::Unavailable,
                shared: CloudBooleanState::True,
                sync_paused: CloudBooleanState::True,
            },
            ..eligible_observation()
        });
        assert_eq!(
            assessment.identity_blockers(),
            &[
                CloudEvictionIdentityBlockReason::AccountIdentityUnavailable,
                CloudEvictionIdentityBlockReason::ContainerIdentityChanged,
                CloudEvictionIdentityBlockReason::ProviderItemIdentityUnsupported,
                CloudEvictionIdentityBlockReason::ItemGenerationUnsupported,
                CloudEvictionIdentityBlockReason::FileVersionUnavailable,
                CloudEvictionIdentityBlockReason::SharedItem,
                CloudEvictionIdentityBlockReason::SyncPaused,
            ]
        );
    }
}
