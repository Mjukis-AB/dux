use std::io::{self, Read};
use std::mem::MaybeUninit;
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::Instant;

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::libc;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

use super::*;

pub(super) fn require_success(output: &BoundedOutput) -> Result<(), CargoMetadataValidationError> {
    let _ = &output.stderr;
    if output.status.success() {
        Ok(())
    } else {
        Err(CargoMetadataValidationError::ProcessFailed)
    }
}

#[derive(Clone, Copy)]
pub(super) struct RunnerLimits {
    pub(super) version: ProcessLimits,
    pub(super) metadata: ProcessLimits,
}

#[derive(Clone, Copy)]
pub(super) enum ConfigurationFenceMode {
    Armed,
    #[cfg(test)]
    UnarmedForParallelTest,
}

impl RunnerLimits {
    pub(super) const fn production() -> Self {
        Self {
            version: ProcessLimits::version(),
            metadata: ProcessLimits::metadata(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ProcessLimits {
    pub(super) timeout: Duration,
    pub(super) stdout_bytes: usize,
    pub(super) stderr_bytes: usize,
}

impl ProcessLimits {
    pub(super) const fn version() -> Self {
        Self {
            timeout: VERSION_TIMEOUT,
            stdout_bytes: VERSION_STDOUT_LIMIT,
            stderr_bytes: VERSION_STDERR_LIMIT,
        }
    }

    pub(super) const fn metadata() -> Self {
        Self {
            timeout: METADATA_TIMEOUT,
            stdout_bytes: METADATA_STDOUT_LIMIT,
            stderr_bytes: METADATA_STDERR_LIMIT,
        }
    }
}

pub(super) struct BoundedOutput {
    pub(super) status: ExitStatus,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) launch_policy_revision: u32,
    pub(super) running_code_directory_hash_sha256: [u8; 32],
}

pub(super) fn run_cargo(
    launch: CargoLaunchTarget<'_>,
    current_directory: &RetainedCargoDirectory,
    arguments: &[&OsStr],
    limits: ProcessLimits,
    environment: &CargoResolutionEnvironment,
    guards: CargoInputGuards<'_>,
) -> Result<BoundedOutput, CargoMetadataValidationError> {
    #[cfg(target_os = "macos")]
    if let Some(signature) = launch.code_signature {
        return run_cargo_suspended_macos(
            &launch,
            signature,
            current_directory,
            arguments,
            limits,
            environment,
            guards,
        );
    }
    run_cargo_portable(
        launch.executable,
        current_directory,
        arguments,
        limits,
        environment,
        guards,
    )
}

#[expect(
    clippy::disallowed_methods,
    reason = "test-only fake Cargo and non-macOS observation use one exact executable with fixed arguments, bounded pipes, a timeout, and process-group termination"
)]
fn run_cargo_portable(
    executable: &Path,
    current_directory: &RetainedCargoDirectory,
    arguments: &[&OsStr],
    limits: ProcessLimits,
    environment: &CargoResolutionEnvironment,
    guards: CargoInputGuards<'_>,
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
        .env("CARGO_LOG", "cargo::util::context=debug")
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
    guards.revalidate()?;
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
    if let Err(error) = guards.revalidate() {
        terminate_process_group(&mut child);
        return Err(error);
    }
    collect_bounded_output(&mut child, limits, guards)
}

fn collect_bounded_output(
    child: &mut Child,
    limits: ProcessLimits,
    guards: CargoInputGuards<'_>,
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
        if let Err(error) = guards.poll() {
            terminate_process_group(child);
            return Err(error);
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
            if let Err(error) = guards.revalidate() {
                terminate_process_group(child);
                return Err(error);
            }
            let status = finish_process_group(child)?;
            return Ok(BoundedOutput {
                status,
                stdout: stdout_bytes,
                stderr: stderr_bytes,
                launch_policy_revision: 0,
                running_code_directory_hash_sha256: [0; 32],
            });
        }
        if Instant::now() >= deadline {
            terminate_process_group(child);
            return Err(CargoMetadataValidationError::Timeout);
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    }
}

#[cfg(target_os = "macos")]
fn run_cargo_suspended_macos(
    launch: &CargoLaunchTarget<'_>,
    signature: &crate::persistence::CargoCodeSignatureRecord,
    current_directory: &RetainedCargoDirectory,
    arguments: &[&OsStr],
    limits: ProcessLimits,
    environment: &CargoResolutionEnvironment,
    guards: CargoInputGuards<'_>,
) -> Result<BoundedOutput, CargoMetadataValidationError> {
    let fence = ExecutableMutationFence::capture(launch.parent, launch.executable, launch.file)
        .map_err(map_cargo_spawn_error)?;
    revalidate_launch_target(launch)?;
    fence.poll().map_err(map_cargo_spawn_error)?;
    current_directory.revalidate()?;
    guards.revalidate()?;

    let mut child = SuspendedCargoChild::spawn(CargoSpawnRequest {
        executable: launch.executable,
        arguments,
        current_directory: current_directory.directory.as_fd(),
        expected_current_directory: current_directory.root(),
        home: environment.home.canonical_path(),
        cargo_home: environment.cargo_home.canonical_path(),
        temporary_directory: environment.temporary_directory.canonical_path(),
        expected_signature: signature,
    })
    .map_err(map_cargo_spawn_error)?;
    let pre_resume = (|| {
        fence.poll().map_err(map_cargo_spawn_error)?;
        revalidate_launch_target(launch)?;
        current_directory.revalidate()?;
        guards.revalidate()?;
        fence.poll().map_err(map_cargo_spawn_error)?;
        child.resume().map_err(map_cargo_spawn_error)
    })();
    if let Err(error) = pre_resume {
        child.terminate();
        return Err(error);
    }
    let output = collect_bounded_output_suspended_macos(&mut child, limits, guards, &fence)?;
    fence.poll().map_err(map_cargo_spawn_error)?;
    revalidate_launch_target(launch)?;
    current_directory.revalidate()?;
    guards.revalidate()?;
    fence.poll().map_err(map_cargo_spawn_error)?;
    Ok(output)
}

#[cfg(target_os = "macos")]
fn collect_bounded_output_suspended_macos(
    child: &mut SuspendedCargoChild,
    limits: ProcessLimits,
    guards: CargoInputGuards<'_>,
    fence: &ExecutableMutationFence,
) -> Result<BoundedOutput, CargoMetadataValidationError> {
    let Some(mut stdout) = child.take_stdout() else {
        child.terminate();
        return Err(CargoMetadataValidationError::PipeConfiguration);
    };
    let Some(mut stderr) = child.take_stderr() else {
        child.terminate();
        return Err(CargoMetadataValidationError::PipeConfiguration);
    };
    if set_nonblocking(&stdout).is_err() || set_nonblocking(&stderr).is_err() {
        child.terminate();
        return Err(CargoMetadataValidationError::PipeConfiguration);
    }
    let Some(deadline) = Instant::now().checked_add(limits.timeout) else {
        child.terminate();
        return Err(CargoMetadataValidationError::Timeout);
    };
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let mut child_exited = false;
    loop {
        if let Err(error) = fence.poll() {
            child.terminate();
            return Err(map_cargo_spawn_error(error));
        }
        if let Err(error) = guards.poll() {
            child.terminate();
            return Err(error);
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
                child.terminate();
                return Err(error);
            }
        }
        if !child_exited {
            child_exited = match child.exited_without_reaping() {
                Ok(exited) => exited,
                Err(source) => {
                    let error = CargoMetadataValidationError::OutputRead {
                        kind: source.kind(),
                        source,
                    };
                    child.terminate();
                    return Err(error);
                }
            };
        }
        if child_exited && stdout_eof && stderr_eof {
            if let Err(error) = guards.revalidate() {
                child.terminate();
                return Err(error);
            }
            fence.poll().map_err(map_cargo_spawn_error)?;
            let launch = child.evidence().clone();
            let status =
                child
                    .finish()
                    .map_err(|source| CargoMetadataValidationError::OutputRead {
                        kind: source.kind(),
                        source,
                    })?;
            return Ok(BoundedOutput {
                status,
                stdout: stdout_bytes,
                stderr: stderr_bytes,
                launch_policy_revision: launch.policy_revision,
                running_code_directory_hash_sha256: sha256(&launch.running_code_directory_hash),
            });
        }
        if Instant::now() >= deadline {
            child.terminate();
            return Err(CargoMetadataValidationError::Timeout);
        }
        thread::sleep(PROCESS_POLL_INTERVAL);
    }
}

#[cfg(target_os = "macos")]
fn revalidate_launch_target(
    launch: &CargoLaunchTarget<'_>,
) -> Result<(), CargoMetadataValidationError> {
    let (parent, file) = capture_exact_cargo_executable(launch.executable)?;
    if &parent != launch.parent || &file != launch.file {
        return Err(CargoMetadataValidationError::ExecutableChanged);
    }
    let _ = launch
        .code_signature
        .ok_or(CargoMetadataValidationError::InvalidCodeSignature)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn map_cargo_spawn_error(error: CargoSpawnError) -> CargoMetadataValidationError {
    match error {
        CargoSpawnError::ExecutableChanged | CargoSpawnError::RunningImageChanged => {
            CargoMetadataValidationError::ExecutableChanged
        }
        CargoSpawnError::WorkingDirectoryChanged => {
            CargoMetadataValidationError::CargoWorkingDirectoryChanged
        }
        CargoSpawnError::PipeConfiguration => CargoMetadataValidationError::PipeConfiguration,
        CargoSpawnError::Spawn(source) => CargoMetadataValidationError::Spawn {
            kind: source.kind(),
            source,
        },
        CargoSpawnError::Process(source) => CargoMetadataValidationError::OutputRead {
            kind: source.kind(),
            source,
        },
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
