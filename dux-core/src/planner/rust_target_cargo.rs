use std::ffi::OsStr;
use std::io::{self, Read};
use std::mem::MaybeUninit;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::libc;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::rust_target::{RustTargetLiveValidationError, RustTargetLiveWitness};
use super::rust_target_source::RustTargetSourceError;
use crate::path_validation::{
    CanonicalFileDigestError, CanonicalFileDigestSnapshot, CanonicalPathError, CanonicalScanRoot,
    FilesystemEntryKind, LexicalPathError, capture_regular_file_sha256, capture_scan_root,
    validate_cleanup_path, validate_scan_root,
};

const SUPPORTED_CARGO_MAJOR: u32 = 1;
const SUPPORTED_CARGO_MINOR: u32 = 96;
const SUPPORTED_CARGO_PATCH: u32 = 0;
const VERSION_STDOUT_LIMIT: usize = 16 * 1024;
const VERSION_STDERR_LIMIT: usize = 16 * 1024;
const METADATA_STDOUT_LIMIT: usize = 8 * 1024 * 1024;
const METADATA_STDERR_LIMIT: usize = 64 * 1024;
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const METADATA_TIMEOUT: Duration = Duration::from_secs(10);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(2);
const MAX_CARGO_EXECUTABLE_BYTES: usize = 256 * 1024 * 1024;
const CARGO_RESOLUTION_POLICY_REVISION: u32 = 1;

/// A canonical Cargo-named executable observation.
///
/// Observation is deliberately separate from target validation. It rejects
/// symlink launchers and binds regular-file bytes plus verbose-version output,
/// but it cannot authenticate Cargo or exclude an exec swap/restore race. A
/// future signed settings grant must add that provenance before authority use.
pub(crate) struct CargoExecutableObservation {
    executable: PathBuf,
    parent: CanonicalScanRoot,
    file: CanonicalFileDigestSnapshot,
    release: CargoRelease,
    version_sha256: [u8; 32],
    environment: CargoResolutionEnvironment,
}

struct CargoResolutionEnvironment {
    home: CanonicalScanRoot,
    cargo_home: CanonicalScanRoot,
    temporary_directory: CanonicalScanRoot,
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
    resolution_policy_revision: u32,
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

pub(crate) fn observe_cargo_executable(
    executable: &Path,
) -> Result<CargoExecutableObservation, CargoMetadataValidationError> {
    let (parent, file) = capture_exact_cargo_executable(executable)?;
    let environment = capture_resolution_environment()?;
    let output = run_cargo(
        executable,
        parent.canonical_path(),
        &[OsStr::new("--version"), OsStr::new("--verbose")],
        ProcessLimits::version(),
        &environment,
    )?;
    require_success(&output)?;
    let release = parse_cargo_release(&output.stdout)?;
    let version_sha256 = sha256(&output.stdout);
    let (after_parent, after_file) = capture_exact_cargo_executable(executable)?;
    if after_parent != parent || after_file != file {
        return Err(CargoMetadataValidationError::ExecutableChanged);
    }
    Ok(CargoExecutableObservation {
        executable: executable.to_path_buf(),
        parent,
        file,
        release,
        version_sha256,
        environment,
    })
}

pub(crate) fn validate_cargo_metadata(
    live: RustTargetLiveWitness,
    cargo: &CargoExecutableObservation,
) -> Result<RustTargetCargoMetadataWitness, CargoMetadataValidationError> {
    validate_cargo_metadata_with_limits(live, cargo, RunnerLimits::production())
}

fn validate_cargo_metadata_with_limits(
    live: RustTargetLiveWitness,
    cargo: &CargoExecutableObservation,
    limits: RunnerLimits,
) -> Result<RustTargetCargoMetadataWitness, CargoMetadataValidationError> {
    live.revalidate_current()?;
    cargo.revalidate()?;
    let before_version = cargo.read_version(limits.version)?;

    let output = run_cargo(
        &cargo.executable,
        live.project_root(),
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
    )?;
    require_success(&output)?;
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
    live.revalidate_current()?;

    Ok(RustTargetCargoMetadataWitness {
        live,
        cargo: cargo.evidence(),
        metadata_sha256: sha256(&output.stdout),
        resolution_policy_revision: CARGO_RESOLUTION_POLICY_REVISION,
    })
}

impl CargoExecutableObservation {
    fn evidence(&self) -> CargoExecutableEvidence {
        CargoExecutableEvidence {
            executable: self.executable.clone(),
            parent: self.parent.clone(),
            file: self.file.clone(),
            release: self.release,
            version_sha256: self.version_sha256,
            environment: self.environment.evidence(),
        }
    }

    fn revalidate(&self) -> Result<(), CargoMetadataValidationError> {
        let (parent, file) = capture_exact_cargo_executable(&self.executable)?;
        if parent != self.parent || file != self.file {
            return Err(CargoMetadataValidationError::ExecutableChanged);
        }
        self.environment.revalidate()?;
        Ok(())
    }

    fn read_version(
        &self,
        limits: ProcessLimits,
    ) -> Result<[u8; 32], CargoMetadataValidationError> {
        let output = run_cargo(
            &self.executable,
            self.parent.canonical_path(),
            &[OsStr::new("--version"), OsStr::new("--verbose")],
            limits,
            &self.environment,
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
        || first_release.major != SUPPORTED_CARGO_MAJOR
        || first_release.minor != SUPPORTED_CARGO_MINOR
        || first_release.patch != SUPPORTED_CARGO_PATCH
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
    current_dir: &Path,
    arguments: &[&OsStr],
    limits: ProcessLimits,
    environment: &CargoResolutionEnvironment,
) -> Result<BoundedOutput, CargoMetadataValidationError> {
    // DUX-DESTRUCTIVE: allow=cargo-metadata-observer-spawn -- launch only the exactly observed canonical executable with fixed observer-owned arguments, bounded nonblocking pipes, offline mode, and process-group termination; this observation does not authenticate Cargo or grant cleanup authority
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(current_dir)
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
    let mut child = command
        .spawn()
        .map_err(|source| CargoMetadataValidationError::Spawn {
            kind: source.kind(),
            source,
        })?;
    collect_bounded_output(&mut child, limits)
}

fn collect_bounded_output(
    child: &mut Child,
    limits: ProcessLimits,
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

    pub(super) fn resolution_policy_revision(&self) -> u32 {
        self.resolution_policy_revision
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
    )
}
