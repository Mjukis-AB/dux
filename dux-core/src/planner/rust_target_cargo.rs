use std::ffi::OsStr;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nix::fcntl::{OFlag, open};
use nix::libc;
use nix::sys::stat::{Mode, fstat};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(target_os = "macos")]
use super::cargo_code_signature_macos::inspect_cargo_code_signature;
use super::cargo_config::{
    CargoConfigurationError, CargoConfigurationEvidence, CargoConfigurationGuard,
};
use super::cargo_manifest_probes::{
    CargoManifestProbeError, CargoManifestProbeEvidence, CargoManifestProbeGuard,
};
use super::cargo_package_metadata::{
    CargoPackageMetadataDeclaration, CargoPackageMetadataError, CargoPackageMetadataEvidence,
    CargoPackageMetadataGuard,
};
#[cfg(target_os = "macos")]
use super::cargo_spawn_macos::{
    CargoSpawnError, CargoSpawnRequest, ExecutableMutationFence, SuspendedCargoChild,
};
use super::cargo_target_namespace::{
    CargoTargetDeclaration, CargoTargetNamespaceDeclaration, CargoTargetNamespaceError,
    CargoTargetNamespaceEvidence, CargoTargetNamespaceGuard,
};
use super::cargo_workspace::{
    CargoDependencyManifestEvidence, CargoReportedPathDependency,
    CargoWorkspaceManifestDeclaration, CargoWorkspaceManifestError, CargoWorkspaceManifestEvidence,
    CargoWorkspaceManifestGuard, MAX_WORKSPACE_MEMBERS,
};
use super::cargo_workspace_glob::{
    CargoWorkspaceGlobError, CargoWorkspaceGlobEvidence, CargoWorkspaceGlobExpansion,
    CargoWorkspaceGlobGuard,
};
use super::descendant_policy::{DescendantPolicyError, DescendantPolicyWitness};
use super::process_activity::{ProcessActivityError, ProcessActivityWitness};
use super::rust_target::{
    RUST_TARGET_WITNESS_REVISION, RustTargetLiveValidationError, RustTargetLiveWitness,
};
use super::rust_target_source::RustTargetSourceError;
use crate::domain::{Candidate, CandidateId, ScanId};
use crate::path_validation::{
    CanonicalFileDigestError, CanonicalFileDigestSnapshot, CanonicalPathError,
    CanonicalPathSnapshot, CanonicalScanRoot, FilesystemBoundarySnapshot, FilesystemEntryKind,
    LexicalPathError, TrustedHomeMountError, TrustedHomeMountWitness, capture_filesystem_boundary,
    capture_regular_file_sha256, capture_scan_root, validate_cleanup_path, validate_scan_root,
};
use crate::persistence::{
    CARGO_ENROLLMENT_SUPPORTED_RELEASE, CargoEnrollmentSetting, CargoEnrollmentSettingUpdate,
    CargoEnrollmentState, CargoExecutableEnrollmentIdentity, CleanupSessionId, HistoryErrorKind,
    StoreCoordinator,
};

#[path = "rust_target_cargo/metadata.rs"]
mod metadata;
use metadata::{
    CargoPathDependencyEvidence, CargoWorkspaceMembershipConsistencyEvidence, ParsedCargoMetadata,
    parse_metadata_document, validate_workspace_membership_consistency,
};
#[path = "rust_target_cargo/runner.rs"]
mod runner;
use runner::{ConfigurationFenceMode, ProcessLimits, RunnerLimits, require_success, run_cargo};
#[cfg(test)]
#[path = "rust_target_cargo/test_support.rs"]
mod test_support;
#[cfg(test)]
pub(super) use test_support::{
    revalidate_retained_cargo_directory_after_hook_for_test, validate_cargo_metadata_for_test,
    validate_cargo_metadata_with_input_fences_for_test,
};
#[cfg(all(test, target_os = "macos"))]
pub(super) use test_support::{
    signed_cargo_output_limit_for_test, validate_cargo_metadata_with_enrollment_hook_for_test,
};

const VERSION_STDOUT_LIMIT: usize = 16 * 1024;
const VERSION_STDERR_LIMIT: usize = 16 * 1024;
const METADATA_STDOUT_LIMIT: usize = 8 * 1024 * 1024;
const METADATA_STDERR_LIMIT: usize = 512 * 1024;
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const METADATA_TIMEOUT: Duration = Duration::from_secs(10);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(2);
const MAX_CARGO_EXECUTABLE_BYTES: usize = 256 * 1024 * 1024;
const CARGO_RESOLUTION_POLICY_REVISION: u32 = 12;
pub(crate) const RUST_TARGET_RULE_BOUNDARY_REVISION: u32 = 1;
const MAX_PACKAGE_ID_BYTES: usize = 4 * 1024;
const MAX_WORKSPACE_DEFAULT_MEMBER_ROWS: usize = MAX_WORKSPACE_MEMBERS * MAX_WORKSPACE_MEMBERS;
const MAX_PACKAGE_DEPENDENCY_DECLARATIONS: usize = 4 * 1024;
const MAX_LOCAL_DEPENDENCY_PATH_BYTES: usize = 256 * 1024;
const CARGO_PATH_DEPENDENCY_POLICY_REVISION: u32 = 1;
const CARGO_WORKSPACE_MEMBERSHIP_CONSISTENCY_POLICY_REVISION: u32 = 1;
const CARGO_ENROLLMENT_SUPPORTED_COMMIT: &str = "30a34c6821b57de0aaec83a901aca39f88f6778c";

/// Static executable evidence captured without executing untrusted bytes.
struct CargoExecutableStaticObservation {
    executable: PathBuf,
    parent: CanonicalScanRoot,
    file: CanonicalFileDigestSnapshot,
    environment: CargoResolutionEnvironment,
    #[cfg(target_os = "macos")]
    code_signature: Option<crate::persistence::CargoCodeSignatureRecord>,
}

/// A canonical Cargo-named executable observation after verbose-version
/// execution. Production creates this only for an enrolled executable or after
/// the consuming enrollment commit explicitly authorizes execution.
pub(crate) struct CargoExecutableObservation {
    executable: PathBuf,
    parent: CanonicalScanRoot,
    file: CanonicalFileDigestSnapshot,
    release: CargoRelease,
    version_sha256: [u8; 32],
    environment: CargoResolutionEnvironment,
    #[cfg(target_os = "macos")]
    code_signature: Option<crate::persistence::CargoCodeSignatureRecord>,
}

/// Exact inspection result that can be committed only to its originating
/// store. It cannot clear a blocker, create a plan, or perform an effect.
pub(crate) struct DirectCargoEnrollmentPreview {
    store: Arc<StoreCoordinator>,
    expected_enrollment: CargoEnrollmentSetting,
    observation: CargoExecutableStaticObservation,
}

#[derive(Debug)]
pub(crate) enum DirectCargoEnrollmentCommitError {
    Validation(CargoMetadataValidationError),
    History(HistoryErrorKind),
}

struct EnrollmentGuard {
    store: Arc<StoreCoordinator>,
    enrollment: CargoEnrollmentSetting,
}

struct CargoResolutionEnvironment {
    home: CanonicalScanRoot,
    cargo_home: CanonicalScanRoot,
    temporary_directory: CanonicalScanRoot,
}

/// An exact directory kept open across process creation so the child changes
/// directory by descriptor rather than resolving a mutable pathname.
struct RetainedCargoDirectory {
    root: CanonicalScanRoot,
    directory: OwnedFd,
}

struct CargoLaunchTarget<'a> {
    executable: &'a Path,
    parent: &'a CanonicalScanRoot,
    file: &'a CanonicalFileDigestSnapshot,
    #[cfg(target_os = "macos")]
    code_signature: Option<&'a crate::persistence::CargoCodeSignatureRecord>,
}

#[derive(Clone, Copy, Default)]
struct CargoInputGuards<'a> {
    configuration: Option<&'a CargoConfigurationGuard>,
    manifest_probes: Option<&'a CargoManifestProbeGuard>,
    workspace_glob: Option<&'a CargoWorkspaceGlobGuard>,
    workspace: Option<&'a CargoWorkspaceManifestGuard>,
    package_metadata: Option<&'a CargoPackageMetadataGuard>,
    target_namespace: Option<&'a CargoTargetNamespaceGuard>,
}

impl CargoInputGuards<'_> {
    fn poll(self) -> Result<(), CargoMetadataValidationError> {
        if let Some(configuration) = self.configuration {
            configuration.poll()?;
        }
        if let Some(manifest_probes) = self.manifest_probes {
            manifest_probes.poll()?;
        }
        if let Some(workspace_glob) = self.workspace_glob {
            workspace_glob.poll()?;
        }
        if let Some(workspace) = self.workspace {
            workspace.poll()?;
        }
        if let Some(package_metadata) = self.package_metadata {
            package_metadata.poll()?;
        }
        if let Some(target_namespace) = self.target_namespace {
            target_namespace.poll()?;
        }
        Ok(())
    }

    fn revalidate(self) -> Result<(), CargoMetadataValidationError> {
        if let Some(configuration) = self.configuration {
            configuration.revalidate()?;
        }
        if let Some(manifest_probes) = self.manifest_probes {
            manifest_probes.revalidate()?;
        }
        if let Some(workspace_glob) = self.workspace_glob {
            workspace_glob.revalidate()?;
        }
        if let Some(workspace) = self.workspace {
            workspace.revalidate()?;
        }
        if let Some(package_metadata) = self.package_metadata {
            package_metadata.revalidate()?;
        }
        if let Some(target_namespace) = self.target_namespace {
            target_namespace.revalidate()?;
        }
        Ok(())
    }
}

/// Owned read-set fences retained after Cargo metadata publication.
///
/// The metadata parser emits evidence for presentation, but that evidence is
/// not enough to support a future planning boundary once the kqueue/FSEvents
/// guards are dropped. This capsule owns every guard and exposes only a
/// private revalidation operation; it cannot be cloned, serialized, or
/// converted into cleanup authority.
struct CargoInputGuardsOwned {
    configuration: CargoConfigurationGuard,
    manifest_probes: CargoManifestProbeGuard,
    workspace_glob: CargoWorkspaceGlobGuard,
    workspace: CargoWorkspaceManifestGuard,
    package_metadata: CargoPackageMetadataGuard,
    target_namespace: CargoTargetNamespaceGuard,
}

impl CargoInputGuardsOwned {
    fn borrowed(&self) -> CargoInputGuards<'_> {
        CargoInputGuards {
            configuration: Some(&self.configuration),
            manifest_probes: Some(&self.manifest_probes),
            workspace_glob: Some(&self.workspace_glob),
            workspace: Some(&self.workspace),
            package_metadata: Some(&self.package_metadata),
            target_namespace: Some(&self.target_namespace),
        }
    }

    fn revalidate(&self) -> Result<(), CargoMetadataValidationError> {
        self.borrowed().revalidate()
    }
}

/// Exact executable and environment evidence used for one metadata result.
///
/// This intentionally has no clone or serialization implementation. It is a
/// momentary observation and does not authenticate the executable as Cargo.
struct CargoExecutableEvidence {
    executable: PathBuf,
    parent: CanonicalScanRoot,
    file: CanonicalFileDigestSnapshot,
    release: CargoRelease,
    version_sha256: [u8; 32],
    environment: CargoResolutionEnvironment,
    enrollment_revision: u64,
    #[cfg(target_os = "macos")]
    code_signature: Option<crate::persistence::CargoCodeSignatureRecord>,
}

/// Current Cargo workspace/config evidence layered over the live layout check.
///
/// This type has no clone, serialization, blocker-removal, plan, FFI, or effect
/// operation. In particular, Cargo output is never cleanup authority.
#[must_use = "Cargo metadata is observational and does not authorize cleanup"]
pub(crate) struct RustTargetCargoMetadataWitness {
    live: RustTargetLiveWitness,
    cargo: CargoExecutableEvidence,
    cargo_observation: CargoExecutableObservation,
    enrollment_guard: Option<EnrollmentGuard>,
    project_directory: RetainedCargoDirectory,
    input_guards: CargoInputGuardsOwned,
    boundary: FilesystemBoundarySnapshot,
    metadata_sha256: [u8; 32],
    configuration: CargoConfigurationEvidence,
    manifest_probes: CargoManifestProbeEvidence,
    workspace_glob: CargoWorkspaceGlobEvidence,
    workspace_membership_consistency: CargoWorkspaceMembershipConsistencyEvidence,
    workspace: CargoWorkspaceManifestEvidence,
    path_dependencies: CargoPathDependencyEvidence,
    dependency_manifests: CargoDependencyManifestEvidence,
    package_metadata: CargoPackageMetadataEvidence,
    target_namespace: CargoTargetNamespaceEvidence,
    launch_policy_revision: u32,
    running_code_directory_hash_sha256: [u8; 32],
    resolution_policy_revision: u32,
    enrollment_revision: u64,
}

/// Consumed, path-private provenance for a future trusted planner boundary.
///
/// The token owns the fully fenced metadata witness, but intentionally has no
/// candidate paths, plan conversion, blocker-removal, approval, FFI, schedule,
/// or platform-effect operation. It exists to make the next authority join
/// consume and revalidate the exact observation rather than copy its digests.
#[must_use = "planning provenance must be consumed by a reviewed planner boundary"]
pub(crate) struct RustTargetCargoPlanningProvenance {
    witness: RustTargetCargoMetadataWitness,
    source_scan_id: ScanId,
    candidate_id: CandidateId,
    witness_revision: u32,
    resolution_policy_revision: u32,
    protected_path_still_unresolved: CargoProtectedPathStillUnresolved,
}

struct CargoProtectedPathStillUnresolved;

#[derive(Debug, Error)]
pub(crate) enum RustTargetCargoPlanningProvenanceError {
    #[error("Cargo metadata provenance could not be revalidated: {0}")]
    Revalidation(#[from] CargoMetadataValidationError),
    #[error("the Rust-target live witness revision is unsupported")]
    UnsupportedWitnessRevision,
    #[error("the Cargo resolution policy revision is unsupported")]
    UnsupportedResolutionPolicy,
    #[error("the protected-path blocker is not retained by the provenance witness")]
    ProtectedPathBlockerMissing,
    #[error("the provenance binding changed while the token was retained")]
    BindingChanged,
}

/// A consumed, path-private join between the exact Rust-target/Cargo
/// provenance and the macOS current-account home mount observation.
///
/// This is deliberately still not a grant that can clear `ProtectedPath`.
/// It carries the unresolved marker forward and exposes only revalidation and
/// lease release; plan, approval, FFI, scheduling, and effect conversion are
/// intentionally absent.
#[must_use = "rule-boundary evidence must remain attached to its provenance"]
pub(crate) struct RustTargetRuleBoundaryEvidence {
    provenance: RustTargetCargoPlanningProvenance,
    location: TrustedHomeMountWitness,
    process_activity: ProcessActivityWitness,
    descendant_policy: DescendantPolicyWitness,
    boundary_revision: u32,
    protected_path_still_unresolved: CargoProtectedPathStillUnresolved,
}

#[derive(Debug, Error)]
pub(crate) enum RustTargetRuleBoundaryError {
    #[error("Rust-target Cargo provenance could not be revalidated: {0}")]
    Provenance(#[source] RustTargetCargoPlanningProvenanceError),
    #[error("current-account home mount evidence could not be revalidated: {0}")]
    Location(#[source] TrustedHomeMountError),
    #[error("process activity evidence could not be revalidated: {0}")]
    ProcessActivity(#[source] ProcessActivityError),
    #[error("descendant policy evidence could not be revalidated: {0}")]
    DescendantPolicy(#[source] DescendantPolicyError),
    #[error("Rust-target Cargo provenance and home-mount evidence bind different boundaries")]
    BoundaryMismatch,
    #[error("Rust-target rule-boundary revision is unsupported")]
    UnsupportedRevision,
    #[error("Rust-target rule-boundary evidence lost the ProtectedPath blocker")]
    ProtectedPathBlockerMissing,
}

impl RustTargetCargoMetadataWitness {
    /// Revalidate every retained Cargo input fence and the live Rust-target
    /// evidence. This remains an internal provenance check: it exposes no
    /// paths, plan, approval, blocker-removal, FFI, scheduling, or effect
    /// capability.
    pub(crate) fn revalidate(&self) -> Result<(), CargoMetadataValidationError> {
        self.input_guards.revalidate()?;
        self.project_directory.revalidate()?;
        self.boundary
            .revalidate()
            .map_err(CargoMetadataValidationError::from)?;
        self.live.revalidate_current()?;
        self.cargo_observation.revalidate()?;
        if let Some(enrollment) = self.enrollment_guard.as_ref() {
            enrollment.revalidate(&self.cargo_observation)?;
        }
        self.cargo_observation
            .read_version(ProcessLimits::version())?;
        if let Some(enrollment) = self.enrollment_guard.as_ref() {
            enrollment.revalidate(&self.cargo_observation)?;
        }
        self.input_guards.revalidate()?;
        self.project_directory.revalidate()?;
        self.boundary
            .revalidate()
            .map_err(CargoMetadataValidationError::from)?;
        self.live.revalidate_current()?;
        Ok(())
    }

    pub(crate) fn bind_trusted_claim(
        &mut self,
        session_id: CleanupSessionId,
        item_ordinal: usize,
    ) -> Result<(), CargoMetadataValidationError> {
        self.live.bind_trusted_claim(session_id, item_ordinal)?;
        self.revalidate()
    }

    /// Consume this witness into path-private provenance after a complete
    /// revalidation. The unresolved protected-path marker is deliberately
    /// carried forward; this operation cannot make the result actionable.
    pub(crate) fn into_planning_provenance(
        self,
    ) -> Result<RustTargetCargoPlanningProvenance, RustTargetCargoPlanningProvenanceError> {
        self.revalidate()?;
        if self.live.witness_revision() != RUST_TARGET_WITNESS_REVISION {
            return Err(RustTargetCargoPlanningProvenanceError::UnsupportedWitnessRevision);
        }
        if self.resolution_policy_revision != CARGO_RESOLUTION_POLICY_REVISION {
            return Err(RustTargetCargoPlanningProvenanceError::UnsupportedResolutionPolicy);
        }
        if !self.live.protected_path_is_still_unresolved() {
            return Err(RustTargetCargoPlanningProvenanceError::ProtectedPathBlockerMissing);
        }
        Ok(RustTargetCargoPlanningProvenance {
            source_scan_id: self.live.source_scan_id().clone(),
            candidate_id: self.live.candidate_id().clone(),
            witness_revision: self.live.witness_revision(),
            resolution_policy_revision: self.resolution_policy_revision,
            protected_path_still_unresolved: CargoProtectedPathStillUnresolved,
            witness: self,
        })
    }

    pub(crate) fn release(self) -> Result<(), RustTargetSourceError> {
        self.live.release()
    }
}

impl RustTargetCargoPlanningProvenance {
    /// Repeat every retained source, Cargo, and filesystem fence. No path or
    /// cleanup capability is returned.
    pub(crate) fn revalidate(&self) -> Result<(), RustTargetCargoPlanningProvenanceError> {
        self.witness
            .revalidate()
            .map_err(RustTargetCargoPlanningProvenanceError::Revalidation)?;
        if self.witness.live.source_scan_id() != &self.source_scan_id
            || self.witness.live.candidate_id() != &self.candidate_id
            || self.witness.live.witness_revision() != self.witness_revision
            || self.witness_revision != RUST_TARGET_WITNESS_REVISION
            || self.resolution_policy_revision != CARGO_RESOLUTION_POLICY_REVISION
            || !self.witness.live.protected_path_is_still_unresolved()
        {
            return Err(RustTargetCargoPlanningProvenanceError::BindingChanged);
        }
        let _ = &self.protected_path_still_unresolved;
        Ok(())
    }

    pub(crate) fn bind_trusted_claim(
        &mut self,
        session_id: CleanupSessionId,
        item_ordinal: usize,
    ) -> Result<(), RustTargetCargoPlanningProvenanceError> {
        self.witness
            .bind_trusted_claim(session_id, item_ordinal)
            .map_err(RustTargetCargoPlanningProvenanceError::Revalidation)?;
        self.revalidate()
    }

    pub(crate) fn release(self) -> Result<(), RustTargetSourceError> {
        self.witness.release()
    }

    /// Capture exact rule-relative descendant selectors against the retained
    /// code-owned scan root. This creates observation-only evidence; it does
    /// not make the unresolved rule actionable.
    pub(crate) fn capture_descendant_policy(
        &self,
        protected: &[String],
        excluded: &[String],
    ) -> Result<DescendantPolicyWitness, DescendantPolicyError> {
        DescendantPolicyWitness::capture(self.witness.live.scan_root(), protected, excluded)
    }

    /// Consume provenance into a location-bound rule observation. The exact
    /// scan-root boundary must match the Cargo witness; the unresolved
    /// protected-path marker is retained rather than removed.
    pub(crate) fn into_rule_boundary_evidence(
        self,
        location: TrustedHomeMountWitness,
    ) -> Result<RustTargetRuleBoundaryEvidence, RustTargetRuleBoundaryError> {
        self.into_rule_boundary_evidence_with_process_activity_and_descendant_policy(
            location, None, None,
        )
    }

    /// Consume provenance into the same boundary while retaining the
    /// code-owned inactive Cargo/rustc witness. A caller-supplied witness is
    /// accepted only when it contains exactly those guards; it remains
    /// private evidence rather than blocker-removal or executor capability.
    pub(crate) fn into_rule_boundary_evidence_with_process_activity(
        self,
        location: TrustedHomeMountWitness,
        process_activity: Option<ProcessActivityWitness>,
    ) -> Result<RustTargetRuleBoundaryEvidence, RustTargetRuleBoundaryError> {
        self.into_rule_boundary_evidence_with_process_activity_and_descendant_policy(
            location,
            process_activity,
            None,
        )
    }

    /// Consume provenance while retaining exact protected/excluded descendant
    /// observations. Current Cargo rules have no selectors, so this remains an
    /// opt-in infrastructure seam and does not clear the ProtectedPath block.
    pub(crate) fn into_rule_boundary_evidence_with_process_activity_and_descendant_policy(
        self,
        location: TrustedHomeMountWitness,
        process_activity: Option<ProcessActivityWitness>,
        descendant_policy: Option<DescendantPolicyWitness>,
    ) -> Result<RustTargetRuleBoundaryEvidence, RustTargetRuleBoundaryError> {
        self.revalidate()
            .map_err(RustTargetRuleBoundaryError::Provenance)?;
        location
            .revalidate()
            .map_err(RustTargetRuleBoundaryError::Location)?;
        let process_activity = match process_activity {
            Some(process_activity) => {
                process_activity
                    .ensure_cargo_quiescence()
                    .map_err(RustTargetRuleBoundaryError::ProcessActivity)?;
                process_activity
            }
            None => ProcessActivityWitness::capture_cargo_quiescence()
                .map_err(RustTargetRuleBoundaryError::ProcessActivity)?,
        };
        process_activity
            .revalidate_cargo_quiescence()
            .map_err(RustTargetRuleBoundaryError::ProcessActivity)?;
        let descendant_policy = match descendant_policy {
            Some(descendant_policy) => descendant_policy,
            None => DescendantPolicyWitness::capture(self.witness.live.scan_root(), &[], &[])
                .map_err(RustTargetRuleBoundaryError::DescendantPolicy)?,
        };
        descendant_policy
            .revalidate()
            .map_err(RustTargetRuleBoundaryError::DescendantPolicy)?;
        if !descendant_policy.is_empty() {
            return Err(RustTargetRuleBoundaryError::DescendantPolicy(
                DescendantPolicyError::UnexpectedSelectors,
            ));
        }
        if !location.matches_scan_boundary(&self.witness.boundary) {
            return Err(RustTargetRuleBoundaryError::BoundaryMismatch);
        }
        if !self.witness.live.protected_path_is_still_unresolved() {
            return Err(RustTargetRuleBoundaryError::ProtectedPathBlockerMissing);
        }
        Ok(RustTargetRuleBoundaryEvidence {
            provenance: self,
            location,
            process_activity,
            descendant_policy,
            boundary_revision: RUST_TARGET_RULE_BOUNDARY_REVISION,
            protected_path_still_unresolved: CargoProtectedPathStillUnresolved,
        })
    }
}

impl RustTargetRuleBoundaryEvidence {
    pub(crate) fn protected_path_is_still_unresolved(&self) -> bool {
        let _ = &self.protected_path_still_unresolved;
        true
    }

    /// Check that this consumed Cargo boundary still names the exact durable
    /// candidate and live target selected by the planner. The path-bearing
    /// values are accepted only at this private join and are never exposed by
    /// the resulting grant.
    pub(crate) fn matches_target_binding(
        &self,
        source_scan_id: &ScanId,
        candidate_id: &CandidateId,
        scan_root: &CanonicalScanRoot,
        target: &CanonicalPathSnapshot,
    ) -> bool {
        let live = &self.provenance.witness.live;
        live.source_scan_id() == source_scan_id
            && live.candidate_id() == candidate_id
            && live.scan_root() == scan_root
            && live.target() == target
            && self
                .location
                .matches_scan_boundary(&self.provenance.witness.boundary)
    }

    pub(crate) fn matches_durable_candidate(&self, candidate: &Candidate) -> bool {
        self.provenance
            .witness
            .live
            .matches_durable_candidate(candidate)
    }

    pub(crate) fn matches_bound_target(
        &self,
        scan_root: &CanonicalScanRoot,
        target: &CanonicalPathSnapshot,
    ) -> bool {
        let live = &self.provenance.witness.live;
        live.scan_root() == scan_root
            && live.target() == target
            && self
                .location
                .matches_scan_boundary(&self.provenance.witness.boundary)
    }

    pub(crate) fn revalidate(&self) -> Result<(), RustTargetRuleBoundaryError> {
        if self.boundary_revision != RUST_TARGET_RULE_BOUNDARY_REVISION {
            return Err(RustTargetRuleBoundaryError::UnsupportedRevision);
        }
        self.provenance
            .revalidate()
            .map_err(RustTargetRuleBoundaryError::Provenance)?;
        self.location
            .revalidate()
            .map_err(RustTargetRuleBoundaryError::Location)?;
        self.process_activity
            .ensure_cargo_quiescence()
            .map_err(RustTargetRuleBoundaryError::ProcessActivity)?;
        self.process_activity
            .revalidate_cargo_quiescence()
            .map_err(RustTargetRuleBoundaryError::ProcessActivity)?;
        self.descendant_policy
            .revalidate()
            .map_err(RustTargetRuleBoundaryError::DescendantPolicy)?;
        if !self.descendant_policy.is_empty() {
            return Err(RustTargetRuleBoundaryError::DescendantPolicy(
                DescendantPolicyError::UnexpectedSelectors,
            ));
        }
        if !self
            .location
            .matches_scan_boundary(&self.provenance.witness.boundary)
        {
            return Err(RustTargetRuleBoundaryError::BoundaryMismatch);
        }
        if !self
            .provenance
            .witness
            .live
            .protected_path_is_still_unresolved()
        {
            return Err(RustTargetRuleBoundaryError::ProtectedPathBlockerMissing);
        }
        if !self.protected_path_is_still_unresolved() {
            return Err(RustTargetRuleBoundaryError::ProtectedPathBlockerMissing);
        }
        Ok(())
    }

    pub(crate) fn bind_trusted_claim(
        &mut self,
        session_id: CleanupSessionId,
        item_ordinal: usize,
    ) -> Result<(), RustTargetRuleBoundaryError> {
        self.provenance
            .bind_trusted_claim(session_id, item_ordinal)
            .map_err(RustTargetRuleBoundaryError::Provenance)?;
        self.revalidate()
    }

    pub(crate) fn release(self) -> Result<(), RustTargetSourceError> {
        self.provenance.release()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CargoRelease {
    major: u32,
    minor: u32,
    patch: u32,
}

#[derive(Debug, Error)]
pub(crate) enum CargoMetadataValidationError {
    #[error("Cargo executable locator is not an exact canonical absolute file named cargo")]
    InvalidExecutableLocator,
    #[error("Cargo executable is not a regular file")]
    ExecutableNotRegular,
    #[error("Cargo executable identity changed")]
    ExecutableChanged,
    #[error("Cargo resolution environment is not canonical and directory-backed")]
    InvalidResolutionEnvironment,
    #[error("Cargo verbose version is invalid or unsupported")]
    InvalidCargoVersion,
    #[error("Cargo verbose version changed")]
    CargoVersionChanged,
    #[error("Cargo does not have valid bounded macOS code-signing evidence")]
    InvalidCodeSignature,
    #[error("no current direct Cargo executable is explicitly enrolled")]
    CargoNotEnrolled,
    #[error("the direct Cargo enrollment is corrupt, unavailable, or changed")]
    CargoEnrollmentChanged,
    #[error("the direct Cargo enrollment store operation failed: {kind:?}")]
    CargoEnrollmentStore { kind: HistoryErrorKind },
    #[error("Cargo subprocess could not be started: {kind:?}")]
    Spawn {
        kind: io::ErrorKind,
        #[source]
        source: io::Error,
    },
    #[error("Cargo subprocess pipe could not be configured")]
    PipeConfiguration,
    #[error("Cargo subprocess output could not be read: {kind:?}")]
    OutputRead {
        kind: io::ErrorKind,
        #[source]
        source: io::Error,
    },
    #[error("Cargo subprocess exceeded its time limit")]
    Timeout,
    #[error("Cargo subprocess exceeded its {stream:?} byte limit")]
    OutputLimit { stream: CargoOutputStream },
    #[error("Cargo subprocess failed")]
    ProcessFailed,
    #[error("Cargo metadata was not one exact bounded format-version-1 JSON document")]
    InvalidMetadata,
    #[error("Cargo metadata does not resolve the candidate manifest as the workspace root")]
    WorkspaceMismatch,
    #[error("Cargo metadata does not resolve the candidate as its target directory")]
    TargetDirectoryMismatch,
    #[error("Cargo metadata has an invalid or ambiguous workspace-member declaration")]
    InvalidWorkspaceMembers,
    #[error("Cargo workspace glob namespace is outside the bounded provenance profile")]
    CargoWorkspaceGlobUnsupported,
    #[error("Cargo workspace glob namespace changed during metadata resolution")]
    CargoWorkspaceGlobChanged,
    #[error("Cargo workspace glob namespace could not be bounded")]
    CargoWorkspaceGlobUnavailable,
    #[error("Cargo metadata has malformed or out-of-bounds dependency declarations")]
    InvalidPathDependencies,
    #[error("Cargo metadata references a local dependency not reported as a workspace package")]
    CargoPathDependenciesUnsupported,
    #[error("Cargo local path declarations do not match the exact workspace manifests")]
    CargoDependencyManifestUnsupported,
    #[error("Cargo package README/license metadata is outside the bounded provenance profile")]
    CargoPackageMetadataUnsupported,
    #[error("Cargo package README/license metadata changed during resolution")]
    CargoPackageMetadataChanged,
    #[error("Cargo package README/license metadata could not be bounded")]
    CargoPackageMetadataUnavailable,
    #[error("Cargo workspace manifests could not be captured within the reviewed bounds")]
    WorkspaceManifestUnavailable,
    #[error("Cargo workspace manifests changed during metadata resolution")]
    WorkspaceManifestChanged,
    #[error(
        "Cargo package target/source/build namespace is outside the bounded provenance profile"
    )]
    CargoTargetNamespaceUnsupported,
    #[error("Cargo package target/source/build namespace changed during metadata resolution")]
    CargoTargetNamespaceChanged,
    #[error("Cargo package target/source/build namespace could not be bounded")]
    CargoTargetNamespaceUnavailable,
    #[error("Cargo ancestor manifest namespace is outside the bounded provenance profile")]
    CargoManifestProbesUnsupported,
    #[error("Cargo ancestor manifest namespace changed during metadata resolution")]
    CargoManifestProbesChanged,
    #[error("Cargo ancestor manifest namespace could not be bounded")]
    CargoManifestProbesUnavailable,
    #[error("Cargo configuration is outside the bounded positive provenance profile")]
    CargoConfigurationUnsupported,
    #[error("Cargo configuration discovery state changed during resolution")]
    CargoConfigurationChanged,
    #[error("Cargo configuration discovery state could not be bounded")]
    CargoConfigurationUnavailable,
    #[error("Cargo working-directory continuity changed during process creation")]
    CargoWorkingDirectoryChanged,
    #[error("live Rust-target evidence changed during Cargo resolution")]
    LiveEvidenceChanged,
    #[error(transparent)]
    Lexical(#[from] LexicalPathError),
    #[error(transparent)]
    Filesystem(#[from] CanonicalPathError),
    #[error(transparent)]
    FileDigest(#[from] CanonicalFileDigestError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CargoOutputStream {
    Stdout,
    Stderr,
}

impl From<RustTargetLiveValidationError> for CargoMetadataValidationError {
    fn from(_error: RustTargetLiveValidationError) -> Self {
        Self::LiveEvidenceChanged
    }
}

impl From<CargoConfigurationError> for CargoMetadataValidationError {
    fn from(error: CargoConfigurationError) -> Self {
        match error {
            CargoConfigurationError::Unsupported => Self::CargoConfigurationUnsupported,
            CargoConfigurationError::Changed => Self::CargoConfigurationChanged,
            CargoConfigurationError::Unavailable => Self::CargoConfigurationUnavailable,
        }
    }
}

impl From<CargoWorkspaceManifestError> for CargoMetadataValidationError {
    fn from(error: CargoWorkspaceManifestError) -> Self {
        match error {
            CargoWorkspaceManifestError::Invalid => Self::InvalidWorkspaceMembers,
            CargoWorkspaceManifestError::Changed => Self::WorkspaceManifestChanged,
            CargoWorkspaceManifestError::Unavailable => Self::WorkspaceManifestUnavailable,
        }
    }
}

impl From<CargoWorkspaceGlobError> for CargoMetadataValidationError {
    fn from(error: CargoWorkspaceGlobError) -> Self {
        match error {
            CargoWorkspaceGlobError::Invalid => Self::CargoWorkspaceGlobUnsupported,
            CargoWorkspaceGlobError::Changed => Self::CargoWorkspaceGlobChanged,
            CargoWorkspaceGlobError::Unavailable => Self::CargoWorkspaceGlobUnavailable,
        }
    }
}

impl From<CargoTargetNamespaceError> for CargoMetadataValidationError {
    fn from(error: CargoTargetNamespaceError) -> Self {
        match error {
            CargoTargetNamespaceError::Invalid => Self::CargoTargetNamespaceUnsupported,
            CargoTargetNamespaceError::Changed => Self::CargoTargetNamespaceChanged,
            CargoTargetNamespaceError::Unavailable => Self::CargoTargetNamespaceUnavailable,
        }
    }
}

impl From<CargoPackageMetadataError> for CargoMetadataValidationError {
    fn from(error: CargoPackageMetadataError) -> Self {
        match error {
            CargoPackageMetadataError::Invalid => Self::CargoPackageMetadataUnsupported,
            CargoPackageMetadataError::Changed => Self::CargoPackageMetadataChanged,
            CargoPackageMetadataError::Unavailable => Self::CargoPackageMetadataUnavailable,
        }
    }
}

impl From<CargoManifestProbeError> for CargoMetadataValidationError {
    fn from(error: CargoManifestProbeError) -> Self {
        match error {
            CargoManifestProbeError::Unsupported => Self::CargoManifestProbesUnsupported,
            CargoManifestProbeError::Changed => Self::CargoManifestProbesChanged,
            CargoManifestProbeError::Unavailable => Self::CargoManifestProbesUnavailable,
        }
    }
}

pub(crate) fn observe_cargo_executable(
    executable: &Path,
) -> Result<CargoExecutableObservation, CargoMetadataValidationError> {
    observe_cargo_executable_inner(executable, true)
}

fn observe_cargo_executable_inner(
    executable: &Path,
    require_platform_signature: bool,
) -> Result<CargoExecutableObservation, CargoMetadataValidationError> {
    inspect_cargo_executable_static(executable, require_platform_signature)?.execute_version()
}

fn inspect_cargo_executable_static(
    executable: &Path,
    require_platform_signature: bool,
) -> Result<CargoExecutableStaticObservation, CargoMetadataValidationError> {
    CargoExecutableEnrollmentIdentity::validate_path(executable)
        .map_err(|_| CargoMetadataValidationError::InvalidExecutableLocator)?;
    let (parent, file) = capture_exact_cargo_executable(executable)?;
    #[cfg(target_os = "macos")]
    let code_signature = if require_platform_signature {
        Some(
            inspect_cargo_code_signature(executable)
                .map_err(|_| CargoMetadataValidationError::InvalidCodeSignature)?,
        )
    } else {
        None
    };
    #[cfg(not(target_os = "macos"))]
    let _ = require_platform_signature;
    let environment = capture_resolution_environment()?;
    let (after_parent, after_file) = capture_exact_cargo_executable(executable)?;
    #[cfg(target_os = "macos")]
    let after_code_signature = if require_platform_signature {
        Some(
            inspect_cargo_code_signature(executable)
                .map_err(|_| CargoMetadataValidationError::InvalidCodeSignature)?,
        )
    } else {
        None
    };
    if after_parent != parent || after_file != file || {
        #[cfg(target_os = "macos")]
        {
            after_code_signature != code_signature
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    } {
        return Err(CargoMetadataValidationError::ExecutableChanged);
    }
    Ok(CargoExecutableStaticObservation {
        executable: executable.to_path_buf(),
        parent,
        file,
        environment,
        #[cfg(target_os = "macos")]
        code_signature,
    })
}

#[cfg(test)]
pub(super) fn observe_cargo_executable_for_test(
    executable: &Path,
) -> Result<CargoExecutableObservation, CargoMetadataValidationError> {
    observe_cargo_executable_inner(executable, false)
}

#[cfg(target_os = "macos")]
pub(crate) fn inspect_direct_cargo_enrollment(
    store: Arc<StoreCoordinator>,
    executable: &Path,
) -> Result<DirectCargoEnrollmentPreview, CargoMetadataValidationError> {
    let expected_enrollment = store
        .load_cargo_enrollment()
        .map_err(|error| CargoMetadataValidationError::CargoEnrollmentStore { kind: error.kind })?;
    let observation = inspect_cargo_executable_static(executable, true)?;
    observation.revalidate()?;
    if store
        .load_cargo_enrollment()
        .map_err(|error| CargoMetadataValidationError::CargoEnrollmentStore { kind: error.kind })?
        != expected_enrollment
    {
        return Err(CargoMetadataValidationError::CargoEnrollmentChanged);
    }
    Ok(DirectCargoEnrollmentPreview {
        store,
        expected_enrollment,
        observation,
    })
}

#[cfg(target_os = "macos")]
impl DirectCargoEnrollmentPreview {
    pub(crate) fn belongs_to(&self, store: &Arc<StoreCoordinator>) -> bool {
        Arc::ptr_eq(&self.store, store)
    }

    pub(crate) fn executable_path(&self) -> &Path {
        &self.observation.executable
    }

    pub(crate) fn executable_sha256(&self) -> [u8; 32] {
        self.observation.file.sha256()
    }

    pub(crate) fn code_signature(&self) -> &crate::persistence::CargoCodeSignatureRecord {
        self.observation
            .code_signature
            .as_ref()
            .expect("direct macOS enrollment inspection always requires a signature")
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn commit_direct_cargo_enrollment(
    preview: DirectCargoEnrollmentPreview,
) -> Result<CargoEnrollmentSettingUpdate, DirectCargoEnrollmentCommitError> {
    if preview
        .store
        .load_cargo_enrollment()
        .map_err(|error| DirectCargoEnrollmentCommitError::History(error.kind))?
        != preview.expected_enrollment
    {
        return Err(DirectCargoEnrollmentCommitError::Validation(
            CargoMetadataValidationError::CargoEnrollmentChanged,
        ));
    }
    preview
        .observation
        .revalidate()
        .map_err(DirectCargoEnrollmentCommitError::Validation)?;
    let identity = preview
        .observation
        .execute_version()
        .and_then(|observation| observation.enrollment_identity())
        .map_err(DirectCargoEnrollmentCommitError::Validation)?;
    preview
        .store
        .enroll_cargo_executable_if_current(identity, &preview.expected_enrollment)
        .map_err(|error| {
            if error.kind == HistoryErrorKind::InvalidTransition
                && preview.expected_enrollment.revision < i64::MAX as u64
            {
                DirectCargoEnrollmentCommitError::Validation(
                    CargoMetadataValidationError::CargoEnrollmentChanged,
                )
            } else {
                DirectCargoEnrollmentCommitError::History(error.kind)
            }
        })
}

pub(crate) fn revoke_direct_cargo_enrollment(
    store: &StoreCoordinator,
) -> Result<CargoEnrollmentSettingUpdate, CargoMetadataValidationError> {
    store
        .revoke_cargo_executable()
        .map_err(map_enrollment_history)
}

pub(crate) fn load_direct_cargo_enrollment(
    store: &StoreCoordinator,
) -> Result<CargoEnrollmentSetting, CargoMetadataValidationError> {
    store
        .load_cargo_enrollment()
        .map_err(map_enrollment_history)
}

fn map_enrollment_history<T>(_error: T) -> CargoMetadataValidationError {
    CargoMetadataValidationError::CargoEnrollmentChanged
}

impl EnrollmentGuard {
    fn revision(&self) -> u64 {
        self.enrollment.revision
    }

    fn revalidate(
        &self,
        cargo: &CargoExecutableObservation,
    ) -> Result<(), CargoMetadataValidationError> {
        let current = load_direct_cargo_enrollment(&self.store)?;
        if current != self.enrollment {
            return Err(CargoMetadataValidationError::CargoEnrollmentChanged);
        }
        require_enrollment_match(cargo, &current)
    }
}

fn require_enrollment_match(
    cargo: &CargoExecutableObservation,
    enrollment: &CargoEnrollmentSetting,
) -> Result<(), CargoMetadataValidationError> {
    let CargoEnrollmentState::Enrolled(identity) = &enrollment.state else {
        return Err(CargoMetadataValidationError::CargoNotEnrolled);
    };
    if identity.path() != cargo.executable
        || identity.executable_sha256 != cargo.file.sha256()
        || identity.version_sha256 != cargo.version_sha256
        || identity.cargo_release
            != [
                cargo.release.major,
                cargo.release.minor,
                cargo.release.patch,
            ]
    {
        return Err(CargoMetadataValidationError::CargoEnrollmentChanged);
    }
    #[cfg(target_os = "macos")]
    if cargo.code_signature.as_ref() != Some(&identity.signature) {
        return Err(CargoMetadataValidationError::CargoEnrollmentChanged);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = &identity.signature;
        return Err(CargoMetadataValidationError::CargoEnrollmentChanged);
    }
    #[cfg(target_os = "macos")]
    Ok(())
}

#[cfg(target_os = "macos")]
fn require_static_enrollment_match(
    cargo: &CargoExecutableStaticObservation,
    enrollment: &CargoEnrollmentSetting,
) -> Result<(), CargoMetadataValidationError> {
    let CargoEnrollmentState::Enrolled(identity) = &enrollment.state else {
        return Err(CargoMetadataValidationError::CargoNotEnrolled);
    };
    if identity.path() != cargo.executable
        || identity.executable_sha256 != cargo.file.sha256()
        || cargo.code_signature.as_ref() != Some(&identity.signature)
    {
        return Err(CargoMetadataValidationError::CargoEnrollmentChanged);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn observe_enrolled_cargo_executable(
    store: &StoreCoordinator,
    enrollment: &CargoEnrollmentSetting,
) -> Result<CargoExecutableObservation, CargoMetadataValidationError> {
    let executable = match &enrollment.state {
        CargoEnrollmentState::Enrolled(identity) => identity.path(),
        CargoEnrollmentState::NotEnrolled | CargoEnrollmentState::Revoked => {
            return Err(CargoMetadataValidationError::CargoNotEnrolled);
        }
    };
    let cargo = inspect_cargo_executable_static(executable, true)?;
    require_static_enrollment_match(&cargo, enrollment)?;
    if load_direct_cargo_enrollment(store)? != *enrollment {
        return Err(CargoMetadataValidationError::CargoEnrollmentChanged);
    }
    let cargo = cargo.execute_version()?;
    require_enrollment_match(&cargo, enrollment)?;
    Ok(cargo)
}

#[cfg(target_os = "macos")]
pub(crate) fn validate_enrolled_cargo_metadata(
    live: RustTargetLiveWitness,
) -> Result<RustTargetCargoMetadataWitness, CargoMetadataValidationError> {
    let store = Arc::clone(
        live.store()
            .ok_or(CargoMetadataValidationError::CargoEnrollmentChanged)?,
    );
    let enrollment = load_direct_cargo_enrollment(&store)?;
    let cargo = observe_enrolled_cargo_executable(&store, &enrollment)?;
    validate_cargo_metadata_with_limits(
        live,
        &cargo,
        RunnerLimits::production(),
        Some(EnrollmentGuard { store, enrollment }),
        ConfigurationFenceMode::Armed,
        || {},
    )
}

#[cfg(all(test, target_os = "macos"))]
pub(super) fn observe_enrolled_cargo_executable_for_test(
    store: &StoreCoordinator,
    enrollment: &CargoEnrollmentSetting,
) -> Result<CargoExecutableObservation, CargoMetadataValidationError> {
    observe_enrolled_cargo_executable(store, enrollment)
}

#[cfg(test)]
pub(crate) fn validate_cargo_metadata(
    live: RustTargetLiveWitness,
    cargo: &CargoExecutableObservation,
) -> Result<RustTargetCargoMetadataWitness, CargoMetadataValidationError> {
    validate_cargo_metadata_with_limits(
        live,
        cargo,
        RunnerLimits::production(),
        None,
        ConfigurationFenceMode::UnarmedForParallelTest,
        || {},
    )
}

fn validate_cargo_metadata_with_limits(
    live: RustTargetLiveWitness,
    cargo: &CargoExecutableObservation,
    limits: RunnerLimits,
    enrollment_guard: Option<EnrollmentGuard>,
    configuration_fence: ConfigurationFenceMode,
    after_metadata: impl FnOnce(),
) -> Result<RustTargetCargoMetadataWitness, CargoMetadataValidationError> {
    live.revalidate_current()?;
    if let Some(guard) = enrollment_guard.as_ref() {
        guard.revalidate(cargo)?;
    }
    cargo.revalidate()?;
    let before_version = cargo.read_version(limits.version)?;

    let project_directory = RetainedCargoDirectory::capture(live.project_root())?;
    live.revalidate_current()?;
    let boundary = capture_filesystem_boundary(live.scan_root())?;
    let configuration = match configuration_fence {
        ConfigurationFenceMode::Armed => CargoConfigurationGuard::capture(
            project_directory.root(),
            &cargo.environment.cargo_home,
        )?,
        #[cfg(test)]
        ConfigurationFenceMode::UnarmedForParallelTest => {
            CargoConfigurationGuard::capture_unfenced_for_test(
                project_directory.root(),
                &cargo.environment.cargo_home,
            )?
        }
    };
    let manifest_probes = match configuration_fence {
        ConfigurationFenceMode::Armed => CargoManifestProbeGuard::capture(
            project_directory.root(),
            &cargo.environment.cargo_home,
        )?,
        #[cfg(test)]
        ConfigurationFenceMode::UnarmedForParallelTest => {
            CargoManifestProbeGuard::capture_unfenced_for_test(
                project_directory.root(),
                &cargo.environment.cargo_home,
            )?
        }
    };
    let workspace_glob = match configuration_fence {
        ConfigurationFenceMode::Armed => {
            CargoWorkspaceGlobGuard::capture(project_directory.root(), live.manifest_path())?
        }
        #[cfg(test)]
        ConfigurationFenceMode::UnarmedForParallelTest => {
            CargoWorkspaceGlobGuard::capture_unfenced_for_test(
                project_directory.root(),
                live.manifest_path(),
            )?
        }
    };
    let workspace_glob_expansion = workspace_glob.expansion()?;

    let metadata_arguments = [
        OsStr::new("metadata"),
        OsStr::new("--format-version"),
        OsStr::new("1"),
        OsStr::new("--no-deps"),
        OsStr::new("--locked"),
        OsStr::new("--offline"),
        OsStr::new("--quiet"),
        OsStr::new("--color=never"),
        OsStr::new("--manifest-path"),
        live.manifest_path().as_os_str(),
    ];
    let discovery_output = run_cargo(
        cargo.launch_target(),
        &project_directory,
        &metadata_arguments,
        limits.metadata,
        &cargo.environment,
        CargoInputGuards {
            configuration: Some(&configuration),
            manifest_probes: Some(&manifest_probes),
            workspace_glob: Some(&workspace_glob),
            workspace: None,
            package_metadata: None,
            target_namespace: None,
        },
    )?;
    require_success(&discovery_output)?;
    configuration.verify_read_intent(&discovery_output.stderr)?;
    let ParsedCargoMetadata {
        document: discovery_metadata,
        workspace_manifests: discovery_workspace,
        path_dependencies: discovery_path_dependencies,
        package_metadata: discovery_package_metadata,
        target_namespace: discovery_target_namespace,
    } = parse_metadata_document(&discovery_output.stdout, &live)?;
    let discovery_workspace_membership_consistency =
        validate_workspace_membership_consistency(&workspace_glob_expansion, &discovery_metadata)?;
    let workspace = match configuration_fence {
        ConfigurationFenceMode::Armed => {
            CargoWorkspaceManifestGuard::capture(project_directory.root(), &discovery_workspace)?
        }
        #[cfg(test)]
        ConfigurationFenceMode::UnarmedForParallelTest => {
            CargoWorkspaceManifestGuard::capture_unfenced_for_test(
                project_directory.root(),
                &discovery_workspace,
            )?
        }
    };
    let discovery_dependency_manifests = workspace
        .dependency_evidence(&discovery_path_dependencies.reported_edges)
        .map_err(map_dependency_manifest_error)?;
    workspace.revalidate()?;
    let package_metadata = match configuration_fence {
        ConfigurationFenceMode::Armed => CargoPackageMetadataGuard::capture(
            project_directory.root(),
            &discovery_package_metadata,
        )?,
        #[cfg(test)]
        ConfigurationFenceMode::UnarmedForParallelTest => {
            CargoPackageMetadataGuard::capture_unfenced_for_test(
                project_directory.root(),
                &discovery_package_metadata,
            )?
        }
    };
    workspace.revalidate()?;
    let target_namespace = match configuration_fence {
        ConfigurationFenceMode::Armed => CargoTargetNamespaceGuard::capture(
            project_directory.root(),
            &discovery_target_namespace,
        )?,
        #[cfg(test)]
        ConfigurationFenceMode::UnarmedForParallelTest => {
            CargoTargetNamespaceGuard::capture_unfenced_for_test(
                project_directory.root(),
                &discovery_target_namespace,
            )?
        }
    };

    let output = run_cargo(
        cargo.launch_target(),
        &project_directory,
        &metadata_arguments,
        limits.metadata,
        &cargo.environment,
        CargoInputGuards {
            configuration: Some(&configuration),
            manifest_probes: Some(&manifest_probes),
            workspace_glob: Some(&workspace_glob),
            workspace: Some(&workspace),
            package_metadata: Some(&package_metadata),
            target_namespace: Some(&target_namespace),
        },
    )?;
    require_success(&output)?;
    configuration.verify_read_intent(&output.stderr)?;
    let ParsedCargoMetadata {
        document: metadata,
        workspace_manifests: accepted_workspace,
        path_dependencies,
        package_metadata: accepted_package_metadata,
        target_namespace: accepted_target_namespace,
    } = parse_metadata_document(&output.stdout, &live)?;
    let workspace_membership_consistency =
        validate_workspace_membership_consistency(&workspace_glob_expansion, &metadata)?;
    if accepted_target_namespace != discovery_target_namespace {
        return Err(CargoMetadataValidationError::CargoTargetNamespaceChanged);
    }
    if accepted_package_metadata != discovery_package_metadata {
        return Err(CargoMetadataValidationError::CargoPackageMetadataChanged);
    }
    if output.stdout != discovery_output.stdout
        || metadata != discovery_metadata
        || accepted_workspace != discovery_workspace
        || path_dependencies != discovery_path_dependencies
        || workspace_membership_consistency != discovery_workspace_membership_consistency
    {
        return Err(CargoMetadataValidationError::WorkspaceManifestChanged);
    }
    after_metadata();
    project_directory.revalidate()?;
    let configuration_evidence = configuration.evidence()?;
    let manifest_probe_evidence = manifest_probes.evidence()?;
    let workspace_glob_evidence = workspace_glob.evidence()?;
    let workspace_evidence = workspace.evidence()?;
    let dependency_manifest_evidence = workspace
        .dependency_evidence(&path_dependencies.reported_edges)
        .map_err(map_dependency_manifest_error)?;
    if dependency_manifest_evidence != discovery_dependency_manifests {
        return Err(CargoMetadataValidationError::WorkspaceManifestChanged);
    }
    let package_metadata_evidence = package_metadata.evidence()?;
    let target_namespace_evidence = target_namespace.evidence()?;

    let after_version = cargo.read_version(limits.version)?;
    if before_version != after_version {
        return Err(CargoMetadataValidationError::CargoVersionChanged);
    }
    cargo.revalidate()?;
    if let Some(guard) = enrollment_guard.as_ref() {
        guard.revalidate(cargo)?;
    }
    project_directory.revalidate()?;
    configuration.revalidate()?;
    manifest_probes.revalidate()?;
    workspace_glob.revalidate()?;
    workspace.revalidate()?;
    package_metadata.revalidate()?;
    target_namespace.revalidate()?;
    live.revalidate_current()?;

    boundary
        .revalidate()
        .map_err(CargoMetadataValidationError::from)?;

    let enrollment_revision = enrollment_guard
        .as_ref()
        .map_or(0, EnrollmentGuard::revision);
    let cargo_observation = cargo.retained();
    let cargo_evidence = cargo.evidence(enrollment_revision);
    let input_guards = CargoInputGuardsOwned {
        configuration,
        manifest_probes,
        workspace_glob,
        workspace,
        package_metadata,
        target_namespace,
    };

    Ok(RustTargetCargoMetadataWitness {
        live,
        cargo: cargo_evidence,
        cargo_observation,
        enrollment_guard,
        project_directory,
        input_guards,
        boundary,
        metadata_sha256: sha256(&output.stdout),
        configuration: configuration_evidence,
        manifest_probes: manifest_probe_evidence,
        workspace_glob: workspace_glob_evidence,
        workspace_membership_consistency,
        workspace: workspace_evidence,
        path_dependencies,
        dependency_manifests: dependency_manifest_evidence,
        package_metadata: package_metadata_evidence,
        target_namespace: target_namespace_evidence,
        launch_policy_revision: output.launch_policy_revision,
        running_code_directory_hash_sha256: output.running_code_directory_hash_sha256,
        resolution_policy_revision: CARGO_RESOLUTION_POLICY_REVISION,
        enrollment_revision,
    })
}

fn map_dependency_manifest_error(
    error: CargoWorkspaceManifestError,
) -> CargoMetadataValidationError {
    match error {
        CargoWorkspaceManifestError::Invalid => {
            CargoMetadataValidationError::CargoDependencyManifestUnsupported
        }
        CargoWorkspaceManifestError::Changed => {
            CargoMetadataValidationError::WorkspaceManifestChanged
        }
        CargoWorkspaceManifestError::Unavailable => {
            CargoMetadataValidationError::WorkspaceManifestUnavailable
        }
    }
}

impl CargoExecutableStaticObservation {
    fn launch_target(&self) -> CargoLaunchTarget<'_> {
        CargoLaunchTarget {
            executable: &self.executable,
            parent: &self.parent,
            file: &self.file,
            #[cfg(target_os = "macos")]
            code_signature: self.code_signature.as_ref(),
        }
    }

    fn execute_version(self) -> Result<CargoExecutableObservation, CargoMetadataValidationError> {
        self.revalidate()?;
        let current_directory = RetainedCargoDirectory::capture(self.parent.canonical_path())?;
        let output = run_cargo(
            self.launch_target(),
            &current_directory,
            &[OsStr::new("--version"), OsStr::new("--verbose")],
            ProcessLimits::version(),
            &self.environment,
            CargoInputGuards::default(),
        )?;
        require_success(&output)?;
        let release = parse_cargo_release(&output.stdout)?;
        let version_sha256 = sha256(&output.stdout);
        self.revalidate()?;
        Ok(CargoExecutableObservation {
            executable: self.executable,
            parent: self.parent,
            file: self.file,
            release,
            version_sha256,
            environment: self.environment,
            #[cfg(target_os = "macos")]
            code_signature: self.code_signature,
        })
    }

    fn revalidate(&self) -> Result<(), CargoMetadataValidationError> {
        let (parent, file) = capture_exact_cargo_executable(&self.executable)?;
        if parent != self.parent || file != self.file {
            return Err(CargoMetadataValidationError::ExecutableChanged);
        }
        #[cfg(target_os = "macos")]
        if self.code_signature.is_some()
            && inspect_cargo_code_signature(&self.executable)
                .map_err(|_| CargoMetadataValidationError::InvalidCodeSignature)?
                != *self
                    .code_signature
                    .as_ref()
                    .expect("checked as present above")
        {
            return Err(CargoMetadataValidationError::ExecutableChanged);
        }
        self.environment.revalidate()
    }
}

impl CargoExecutableObservation {
    fn launch_target(&self) -> CargoLaunchTarget<'_> {
        CargoLaunchTarget {
            executable: &self.executable,
            parent: &self.parent,
            file: &self.file,
            #[cfg(target_os = "macos")]
            code_signature: self.code_signature.as_ref(),
        }
    }

    #[cfg(target_os = "macos")]
    fn enrollment_identity(
        &self,
    ) -> Result<CargoExecutableEnrollmentIdentity, CargoMetadataValidationError> {
        CargoExecutableEnrollmentIdentity::new(
            self.executable.clone(),
            self.file.sha256(),
            self.version_sha256,
            [self.release.major, self.release.minor, self.release.patch],
            self.code_signature
                .clone()
                .ok_or(CargoMetadataValidationError::InvalidCodeSignature)?,
        )
        .map_err(map_enrollment_history)
    }

    fn evidence(&self, enrollment_revision: u64) -> CargoExecutableEvidence {
        CargoExecutableEvidence {
            executable: self.executable.clone(),
            parent: self.parent.clone(),
            file: self.file.clone(),
            release: self.release,
            version_sha256: self.version_sha256,
            environment: self.environment.evidence(),
            enrollment_revision,
            #[cfg(target_os = "macos")]
            code_signature: self.code_signature.clone(),
        }
    }

    fn retained(&self) -> Self {
        Self {
            executable: self.executable.clone(),
            parent: self.parent.clone(),
            file: self.file.clone(),
            release: self.release,
            version_sha256: self.version_sha256,
            environment: self.environment.evidence(),
            #[cfg(target_os = "macos")]
            code_signature: self.code_signature.clone(),
        }
    }

    fn revalidate(&self) -> Result<(), CargoMetadataValidationError> {
        let (parent, file) = capture_exact_cargo_executable(&self.executable)?;
        if parent != self.parent || file != self.file {
            return Err(CargoMetadataValidationError::ExecutableChanged);
        }
        #[cfg(target_os = "macos")]
        if self.code_signature.is_some()
            && inspect_cargo_code_signature(&self.executable)
                .map_err(|_| CargoMetadataValidationError::InvalidCodeSignature)?
                != *self
                    .code_signature
                    .as_ref()
                    .expect("checked as present above")
        {
            return Err(CargoMetadataValidationError::ExecutableChanged);
        }
        self.environment.revalidate()?;
        Ok(())
    }

    fn read_version(
        &self,
        limits: ProcessLimits,
    ) -> Result<[u8; 32], CargoMetadataValidationError> {
        let current_directory = RetainedCargoDirectory::capture(self.parent.canonical_path())?;
        let output = run_cargo(
            self.launch_target(),
            &current_directory,
            &[OsStr::new("--version"), OsStr::new("--verbose")],
            limits,
            &self.environment,
            CargoInputGuards::default(),
        )?;
        require_success(&output)?;
        let release = parse_cargo_release(&output.stdout)?;
        let digest = sha256(&output.stdout);
        if release != self.release || digest != self.version_sha256 {
            return Err(CargoMetadataValidationError::CargoVersionChanged);
        }
        self.revalidate()?;
        Ok(digest)
    }
}

impl CargoResolutionEnvironment {
    fn evidence(&self) -> Self {
        Self {
            home: self.home.clone(),
            cargo_home: self.cargo_home.clone(),
            temporary_directory: self.temporary_directory.clone(),
        }
    }

    fn revalidate(&self) -> Result<(), CargoMetadataValidationError> {
        for expected in [&self.home, &self.cargo_home, &self.temporary_directory] {
            let current = capture_exact_directory(expected.canonical_path())?;
            if &current != expected {
                return Err(CargoMetadataValidationError::InvalidResolutionEnvironment);
            }
        }
        Ok(())
    }
}

impl RetainedCargoDirectory {
    fn capture(path: &Path) -> Result<Self, CargoMetadataValidationError> {
        let root = capture_exact_directory(path)?;
        let directory = open(
            root.canonical_path(),
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| CargoMetadataValidationError::CargoWorkingDirectoryChanged)?;
        let retained = Self { root, directory };
        retained.revalidate()?;
        Ok(retained)
    }

    fn root(&self) -> &CanonicalScanRoot {
        &self.root
    }

    fn revalidate(&self) -> Result<(), CargoMetadataValidationError> {
        let status = fstat(&self.directory)
            .map_err(|_| CargoMetadataValidationError::CargoWorkingDirectoryChanged)?;
        if status.st_dev as u64 != self.root.identity().volume()
            || u128::from(status.st_ino) != self.root.identity().object()
        {
            return Err(CargoMetadataValidationError::CargoWorkingDirectoryChanged);
        }
        let current = capture_exact_directory(self.root.canonical_path())
            .map_err(|_| CargoMetadataValidationError::CargoWorkingDirectoryChanged)?;
        if current != self.root {
            return Err(CargoMetadataValidationError::CargoWorkingDirectoryChanged);
        }
        Ok(())
    }

    fn raw_fd(&self) -> libc::c_int {
        self.directory.as_raw_fd()
    }
}

fn capture_exact_cargo_executable(
    executable: &Path,
) -> Result<(CanonicalScanRoot, CanonicalFileDigestSnapshot), CargoMetadataValidationError> {
    if !executable.is_absolute() || executable.file_name() != Some(OsStr::new("cargo")) {
        return Err(CargoMetadataValidationError::InvalidExecutableLocator);
    }
    let canonical = std::fs::canonicalize(executable)
        .map_err(|_| CargoMetadataValidationError::InvalidExecutableLocator)?;
    if canonical != executable {
        return Err(CargoMetadataValidationError::InvalidExecutableLocator);
    }
    let parent_path = executable
        .parent()
        .ok_or(CargoMetadataValidationError::InvalidExecutableLocator)?;
    let lexical_parent = validate_scan_root(parent_path)?;
    let parent = capture_scan_root(lexical_parent.clone())?;
    let lexical_file = validate_cleanup_path(&lexical_parent, executable)?;
    let file = capture_regular_file_sha256(&parent, lexical_file, MAX_CARGO_EXECUTABLE_BYTES)?;
    if file.path().target_kind() != FilesystemEntryKind::RegularFile {
        return Err(CargoMetadataValidationError::ExecutableNotRegular);
    }
    if file.path().hard_link_count() != 1 {
        return Err(CargoMetadataValidationError::InvalidExecutableLocator);
    }
    Ok((parent, file))
}

fn capture_resolution_environment()
-> Result<CargoResolutionEnvironment, CargoMetadataValidationError> {
    let home_path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(CargoMetadataValidationError::InvalidResolutionEnvironment)?;
    let cargo_home_path = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_path.join(".cargo"));
    let temporary_path = std::env::temp_dir();
    Ok(CargoResolutionEnvironment {
        home: capture_exact_directory(&home_path)?,
        cargo_home: capture_exact_directory(&cargo_home_path)?,
        temporary_directory: capture_exact_directory(&temporary_path)?,
    })
}

fn capture_exact_directory(path: &Path) -> Result<CanonicalScanRoot, CargoMetadataValidationError> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|_| CargoMetadataValidationError::InvalidResolutionEnvironment)?;
    let lexical = validate_scan_root(&canonical)?;
    capture_scan_root(lexical).map_err(CargoMetadataValidationError::from)
}

fn parse_cargo_release(bytes: &[u8]) -> Result<CargoRelease, CargoMetadataValidationError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| CargoMetadataValidationError::InvalidCargoVersion)?;
    let mut lines = text.lines();
    let first = lines
        .next()
        .ok_or(CargoMetadataValidationError::InvalidCargoVersion)?;
    let first_release = first
        .strip_prefix("cargo ")
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(parse_release)
        .ok_or(CargoMetadataValidationError::InvalidCargoVersion)?;
    let verbose_release = lines
        .clone()
        .find_map(|line| line.strip_prefix("release: "))
        .and_then(parse_release)
        .ok_or(CargoMetadataValidationError::InvalidCargoVersion)?;
    let commit_hash = lines
        .clone()
        .find_map(|line| line.strip_prefix("commit-hash: "))
        .ok_or(CargoMetadataValidationError::InvalidCargoVersion)?;
    let host = lines
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or(CargoMetadataValidationError::InvalidCargoVersion)?;
    if first_release != verbose_release
        || [
            first_release.major,
            first_release.minor,
            first_release.patch,
        ] != CARGO_ENROLLMENT_SUPPORTED_RELEASE
        || commit_hash != CARGO_ENROLLMENT_SUPPORTED_COMMIT
        || !supported_cargo_host(host)
    {
        return Err(CargoMetadataValidationError::InvalidCargoVersion);
    }
    Ok(first_release)
}

fn supported_cargo_host(host: &str) -> bool {
    [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-musl",
        "x86_64-unknown-linux-musl",
    ]
    .contains(&host)
}

fn parse_release(value: &str) -> Option<CargoRelease> {
    let mut parts = value.split('.');
    let release = CargoRelease {
        major: parts.next()?.parse().ok()?,
        minor: parts.next()?.parse().ok()?,
        patch: parts.next()?.parse().ok()?,
    };
    if parts.next().is_some() {
        return None;
    }
    Some(release)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
