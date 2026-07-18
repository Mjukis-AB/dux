use std::ffi::OsStr;
use std::io::{self, Read};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use nix::fcntl::{FcntlArg, OFlag, fcntl, open};
use nix::libc;
use nix::sys::signal::{Signal, killpg};
use nix::sys::stat::{Mode, fstat};
use nix::unistd::Pid;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(target_os = "macos")]
use super::cargo_code_signature_macos::inspect_cargo_code_signature;
use super::cargo_config::{
    CargoConfigurationError, CargoConfigurationEvidence, CargoConfigurationGuard,
};
use super::rust_target::{RustTargetLiveValidationError, RustTargetLiveWitness};
use super::rust_target_source::RustTargetSourceError;
use crate::path_validation::{
    CanonicalFileDigestError, CanonicalFileDigestSnapshot, CanonicalPathError, CanonicalScanRoot,
    FilesystemEntryKind, LexicalPathError, capture_regular_file_sha256, capture_scan_root,
    validate_cleanup_path, validate_scan_root,
};
use crate::persistence::{
    CARGO_ENROLLMENT_SUPPORTED_RELEASE, CargoEnrollmentSetting, CargoEnrollmentSettingUpdate,
    CargoEnrollmentState, CargoExecutableEnrollmentIdentity, HistoryErrorKind, StoreCoordinator,
};

const VERSION_STDOUT_LIMIT: usize = 16 * 1024;
const VERSION_STDERR_LIMIT: usize = 16 * 1024;
const METADATA_STDOUT_LIMIT: usize = 8 * 1024 * 1024;
const METADATA_STDERR_LIMIT: usize = 64 * 1024;
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const METADATA_TIMEOUT: Duration = Duration::from_secs(10);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(2);
const MAX_CARGO_EXECUTABLE_BYTES: usize = 256 * 1024 * 1024;
const CARGO_RESOLUTION_POLICY_REVISION: u32 = 2;

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
    metadata_sha256: [u8; 32],
    configuration: CargoConfigurationEvidence,
    resolution_policy_revision: u32,
    enrollment_revision: u64,
}

impl RustTargetCargoMetadataWitness {
    pub(crate) fn release(self) -> Result<(), RustTargetSourceError> {
        self.live.release()
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
    #[error("Cargo configuration is present and cannot yet be directly attested")]
    CargoConfigurationPresent,
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
            CargoConfigurationError::Present => Self::CargoConfigurationPresent,
            CargoConfigurationError::Changed => Self::CargoConfigurationChanged,
            CargoConfigurationError::Unavailable => Self::CargoConfigurationUnavailable,
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

    let output = run_cargo(
        &cargo.executable,
        &project_directory,
        &[
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
        ],
        limits.metadata,
        &cargo.environment,
        Some(&configuration),
    )?;
    require_success(&output)?;
    after_metadata();
    project_directory.revalidate()?;
    let configuration_evidence = configuration.evidence()?;
    let metadata: CargoMetadataDocument = serde_json::from_slice(&output.stdout)
        .map_err(|_| CargoMetadataValidationError::InvalidMetadata)?;
    if metadata.version != 1 {
        return Err(CargoMetadataValidationError::InvalidMetadata);
    }
    if !metadata.resolve.is_null() {
        return Err(CargoMetadataValidationError::InvalidMetadata);
    }
    let metadata_root = validate_scan_root(Path::new(&metadata.workspace_root))?;
    let _metadata_target =
        validate_cleanup_path(&metadata_root, Path::new(&metadata.target_directory))?;
    if Path::new(&metadata.workspace_root) != live.project_root() {
        return Err(CargoMetadataValidationError::WorkspaceMismatch);
    }
    if Path::new(&metadata.target_directory) != live.target_path() {
        return Err(CargoMetadataValidationError::TargetDirectoryMismatch);
    }

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
    live.revalidate_current()?;

    Ok(RustTargetCargoMetadataWitness {
        live,
        cargo: cargo.evidence(
            enrollment_guard
                .as_ref()
                .map_or(0, EnrollmentGuard::revision),
        ),
        metadata_sha256: sha256(&output.stdout),
        configuration: configuration_evidence,
        resolution_policy_revision: CARGO_RESOLUTION_POLICY_REVISION,
        enrollment_revision: enrollment_guard
            .as_ref()
            .map_or(0, EnrollmentGuard::revision),
    })
}

impl CargoExecutableStaticObservation {
    fn execute_version(self) -> Result<CargoExecutableObservation, CargoMetadataValidationError> {
        self.revalidate()?;
        let current_directory = RetainedCargoDirectory::capture(self.parent.canonical_path())?;
        let output = run_cargo(
            &self.executable,
            &current_directory,
            &[OsStr::new("--version"), OsStr::new("--verbose")],
            ProcessLimits::version(),
            &self.environment,
            None,
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
            &self.executable,
            &current_directory,
            &[OsStr::new("--version"), OsStr::new("--verbose")],
            limits,
            &self.environment,
            None,
        )?;
        require_success(&output)?;
        let release = parse_cargo_release(&output.stdout)?;
        let digest = sha256(&output.stdout);
        if release != self.release || digest != self.version_sha256 {
            return Err(CargoMetadataValidationError::CargoVersionChanged);
        }
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

#[derive(Deserialize)]
struct CargoMetadataDocument {
    version: u32,
    workspace_root: String,
    target_directory: String,
    resolve: serde_json::Value,
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
        || commit_hash.len() != 40
        || !commit_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
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

fn require_success(output: &BoundedOutput) -> Result<(), CargoMetadataValidationError> {
    let _ = &output.stderr;
    if output.status.success() {
        Ok(())
    } else {
        Err(CargoMetadataValidationError::ProcessFailed)
    }
}

#[derive(Clone, Copy)]
struct RunnerLimits {
    version: ProcessLimits,
    metadata: ProcessLimits,
}

#[derive(Clone, Copy)]
enum ConfigurationFenceMode {
    Armed,
    #[cfg(test)]
    UnarmedForParallelTest,
}

impl RunnerLimits {
    const fn production() -> Self {
        Self {
            version: ProcessLimits::version(),
            metadata: ProcessLimits::metadata(),
        }
    }
}

#[derive(Clone, Copy)]
struct ProcessLimits {
    timeout: Duration,
    stdout_bytes: usize,
    stderr_bytes: usize,
}

impl ProcessLimits {
    const fn version() -> Self {
        Self {
            timeout: VERSION_TIMEOUT,
            stdout_bytes: VERSION_STDOUT_LIMIT,
            stderr_bytes: VERSION_STDERR_LIMIT,
        }
    }

    const fn metadata() -> Self {
        Self {
            timeout: METADATA_TIMEOUT,
            stdout_bytes: METADATA_STDOUT_LIMIT,
            stderr_bytes: METADATA_STDERR_LIMIT,
        }
    }
}

struct BoundedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[expect(
    clippy::disallowed_methods,
    reason = "the sealed Cargo observer launches one exactly observed executable with fixed arguments, bounded pipes, a timeout, and process-group termination"
)]
fn run_cargo(
    executable: &Path,
    current_directory: &RetainedCargoDirectory,
    arguments: &[&OsStr],
    limits: ProcessLimits,
    environment: &CargoResolutionEnvironment,
    configuration: Option<&CargoConfigurationGuard>,
) -> Result<BoundedOutput, CargoMetadataValidationError> {
    // DUX-DESTRUCTIVE: allow=cargo-metadata-observer-spawn -- launch only the exactly observed canonical executable with fixed observer-owned arguments, bounded nonblocking pipes, offline mode, and process-group termination; this observation does not authenticate Cargo or grant cleanup authority
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("HOME", environment.home.canonical_path())
        .env("CARGO_HOME", environment.cargo_home.canonical_path())
        .env("TMPDIR", environment.temporary_directory.canonical_path())
        // A zero-length PATH component searches the current directory. Point
        // at this fixed non-directory system object so helper lookup cannot
        // fall back to project-controlled executables.
        .env("PATH", "/dev/null")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TERM_COLOR", "never")
        .process_group(0);
    let directory_fd = current_directory.raw_fd();
    // SAFETY: `fchdir` is async-signal-safe and the descriptor remains owned
    // by `current_directory` until `spawn` has completed. The closure performs
    // no allocation or other non-signal-safe work before exec.
    unsafe {
        command.pre_exec(move || {
            if libc::fchdir(directory_fd) == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    current_directory.revalidate()?;
    if let Some(configuration) = configuration {
        configuration.revalidate()?;
    }
    let mut child = command
        .spawn()
        .map_err(|source| CargoMetadataValidationError::Spawn {
            kind: source.kind(),
            source,
        })?;
    if let Err(error) = current_directory.revalidate() {
        terminate_process_group(&mut child);
        return Err(error);
    }
    if let Some(configuration) = configuration
        && let Err(error) = configuration.revalidate()
    {
        terminate_process_group(&mut child);
        return Err(error.into());
    }
    collect_bounded_output(&mut child, limits, configuration)
}

fn collect_bounded_output(
    child: &mut Child,
    limits: ProcessLimits,
    configuration: Option<&CargoConfigurationGuard>,
) -> Result<BoundedOutput, CargoMetadataValidationError> {
    let Some(mut stdout) = child.stdout.take() else {
        terminate_process_group(child);
        return Err(CargoMetadataValidationError::PipeConfiguration);
    };
    let Some(mut stderr) = child.stderr.take() else {
        terminate_process_group(child);
        return Err(CargoMetadataValidationError::PipeConfiguration);
    };
    if set_nonblocking(&stdout).is_err() || set_nonblocking(&stderr).is_err() {
        terminate_process_group(child);
        return Err(CargoMetadataValidationError::PipeConfiguration);
    }

    let Some(deadline) = Instant::now().checked_add(limits.timeout) else {
        terminate_process_group(child);
        return Err(CargoMetadataValidationError::Timeout);
    };
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let mut child_exited = false;

    loop {
        if let Some(configuration) = configuration
            && let Err(error) = configuration.poll()
        {
            terminate_process_group(child);
            return Err(error.into());
        }
        let stdout_result = drain_pipe(
            &mut stdout,
            &mut stdout_bytes,
            limits.stdout_bytes,
            CargoOutputStream::Stdout,
        );
        let stderr_result = drain_pipe(
            &mut stderr,
            &mut stderr_bytes,
            limits.stderr_bytes,
            CargoOutputStream::Stderr,
        );
        match (stdout_result, stderr_result) {
            (Ok(stdout_done), Ok(stderr_done)) => {
                stdout_eof |= stdout_done;
                stderr_eof |= stderr_done;
            }
            (Err(error), _) | (_, Err(error)) => {
                terminate_process_group(child);
                return Err(error);
            }
        }
        if !child_exited {
            child_exited = match child_exited_without_reaping(child) {
                Ok(exited) => exited,
                Err(source) => {
                    let error = CargoMetadataValidationError::OutputRead {
                        kind: source.kind(),
                        source,
                    };
                    terminate_process_group(child);
                    return Err(error);
                }
            };
        }
        if child_exited && stdout_eof && stderr_eof {
            if let Some(configuration) = configuration
                && let Err(error) = configuration.revalidate()
            {
                terminate_process_group(child);
                return Err(error.into());
            }
            let status = finish_process_group(child)?;
            return Ok(BoundedOutput {
                status,
                stdout: stdout_bytes,
                stderr: stderr_bytes,
            });
        }
        if Instant::now() >= deadline {
            terminate_process_group(child);
            return Err(CargoMetadataValidationError::Timeout);
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    }
}

fn child_exited_without_reaping(child: &Child) -> io::Result<bool> {
    let raw_pid = i32::try_from(child.id())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "child PID is out of range"))?;
    let process_id = libc::id_t::try_from(raw_pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "child PID is out of range"))?;
    let mut information = MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: `information` points to writable storage for the duration of the
    // call. P_PID selects only this direct child, while WNOWAIT keeps its PID
    // reserved until the process group is signalled and `Child::wait` reaps it.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            process_id,
            information.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful waitid initialized the supplied siginfo object. The
    // zeroed no-event result has si_pid == 0 by the waitid contract.
    let exited = unsafe { information.assume_init().si_pid() != 0 };
    Ok(exited)
}

fn finish_process_group(child: &mut Child) -> Result<ExitStatus, CargoMetadataValidationError> {
    if let Ok(raw_pid) = i32::try_from(child.id()) {
        let _ = killpg(Pid::from_raw(raw_pid), Signal::SIGKILL);
    }
    child
        .wait()
        .map_err(|source| CargoMetadataValidationError::OutputRead {
            kind: source.kind(),
            source,
        })
}

fn set_nonblocking(
    pipe: &(impl std::os::fd::AsFd + ?Sized),
) -> Result<(), CargoMetadataValidationError> {
    let raw = fcntl(pipe, FcntlArg::F_GETFL)
        .map_err(|_| CargoMetadataValidationError::PipeConfiguration)?;
    let flags = OFlag::from_bits_truncate(raw).union(OFlag::O_NONBLOCK);
    fcntl(pipe, FcntlArg::F_SETFL(flags))
        .map_err(|_| CargoMetadataValidationError::PipeConfiguration)?;
    Ok(())
}

fn drain_pipe<R: Read>(
    pipe: &mut R,
    output: &mut Vec<u8>,
    limit: usize,
    stream: CargoOutputStream,
) -> Result<bool, CargoMetadataValidationError> {
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if count > limit.saturating_sub(output.len()) {
                    return Err(CargoMetadataValidationError::OutputLimit { stream });
                }
                output.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(source) => {
                return Err(CargoMetadataValidationError::OutputRead {
                    kind: source.kind(),
                    source,
                });
            }
        }
    }
}

fn terminate_process_group(child: &mut Child) {
    if let Ok(raw_pid) = i32::try_from(child.id()) {
        let _ = killpg(Pid::from_raw(raw_pid), Signal::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
impl RustTargetCargoMetadataWitness {
    pub(super) fn cargo_release(&self) -> (u32, u32, u32) {
        (
            self.cargo.release.major,
            self.cargo.release.minor,
            self.cargo.release.patch,
        )
    }

    pub(super) fn cargo_version_sha256(&self) -> [u8; 32] {
        self.cargo.version_sha256
    }

    pub(super) fn cargo_executable_sha256(&self) -> [u8; 32] {
        self.cargo.file.sha256()
    }

    pub(super) fn cargo_executable_identity(&self) -> crate::path_validation::FilesystemIdentity {
        self.cargo.file.path().target_identity()
    }

    pub(super) fn cargo_environment_identities(
        &self,
    ) -> [crate::path_validation::FilesystemIdentity; 3] {
        [
            self.cargo.environment.home.identity(),
            self.cargo.environment.cargo_home.identity(),
            self.cargo.environment.temporary_directory.identity(),
        ]
    }

    pub(super) fn cargo_executable_path(&self) -> &Path {
        &self.cargo.executable
    }

    pub(super) fn cargo_executable_parent_identity(
        &self,
    ) -> crate::path_validation::FilesystemIdentity {
        self.cargo.parent.identity()
    }

    pub(super) fn live(&self) -> &RustTargetLiveWitness {
        &self.live
    }

    pub(super) fn metadata_sha256(&self) -> [u8; 32] {
        self.metadata_sha256
    }

    pub(super) fn configuration_policy_revision(&self) -> u32 {
        self.configuration.policy_revision
    }

    pub(super) fn configuration_lookup_count(&self) -> u32 {
        self.configuration.lookup_count
    }

    pub(super) fn configuration_closure_sha256(&self) -> [u8; 32] {
        self.configuration.closure_sha256
    }

    pub(super) fn resolution_policy_revision(&self) -> u32 {
        self.resolution_policy_revision
    }

    pub(super) fn enrollment_revision(&self) -> u64 {
        self.enrollment_revision
    }
}

#[cfg(test)]
impl CargoExecutableObservation {
    pub(super) fn executable_sha256(&self) -> [u8; 32] {
        self.file.sha256()
    }

    pub(super) fn executable_identity(&self) -> crate::path_validation::FilesystemIdentity {
        self.file.path().target_identity()
    }

    pub(super) fn environment_identities(&self) -> [crate::path_validation::FilesystemIdentity; 3] {
        [
            self.environment.home.identity(),
            self.environment.cargo_home.identity(),
            self.environment.temporary_directory.identity(),
        ]
    }
}

#[cfg(test)]
pub(super) fn validate_cargo_metadata_for_test(
    live: RustTargetLiveWitness,
    cargo: &CargoExecutableObservation,
    timeout: Duration,
    stdout_bytes: usize,
    stderr_bytes: usize,
) -> Result<RustTargetCargoMetadataWitness, CargoMetadataValidationError> {
    let limits = ProcessLimits {
        timeout,
        stdout_bytes,
        stderr_bytes,
    };
    validate_cargo_metadata_with_limits(
        live,
        cargo,
        RunnerLimits {
            version: ProcessLimits::version(),
            metadata: limits,
        },
        None,
        ConfigurationFenceMode::UnarmedForParallelTest,
        || {},
    )
}

#[cfg(test)]
pub(super) fn validate_cargo_metadata_with_configuration_fence_for_test(
    live: RustTargetLiveWitness,
    cargo: &CargoExecutableObservation,
) -> Result<RustTargetCargoMetadataWitness, CargoMetadataValidationError> {
    validate_cargo_metadata_with_limits(
        live,
        cargo,
        RunnerLimits::production(),
        None,
        ConfigurationFenceMode::Armed,
        || {},
    )
}

#[cfg(test)]
pub(super) fn revalidate_retained_cargo_directory_after_hook_for_test(
    path: &Path,
    hook: impl FnOnce(),
) -> Result<(), CargoMetadataValidationError> {
    let directory = RetainedCargoDirectory::capture(path)?;
    hook();
    directory.revalidate()
}

#[cfg(test)]
pub(super) fn validate_cargo_metadata_with_enrollment_hook_for_test(
    live: RustTargetLiveWitness,
    cargo: &CargoExecutableObservation,
    store: Arc<StoreCoordinator>,
    enrollment: CargoEnrollmentSetting,
    after_metadata: impl FnOnce(),
) -> Result<RustTargetCargoMetadataWitness, CargoMetadataValidationError> {
    validate_cargo_metadata_with_limits(
        live,
        cargo,
        RunnerLimits::production(),
        Some(EnrollmentGuard { store, enrollment }),
        ConfigurationFenceMode::UnarmedForParallelTest,
        after_metadata,
    )
}
