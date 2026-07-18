//! Suspended macOS Cargo launch with pre-user-space running-image attestation.
//!
//! macOS has no supported `fexecve` or `execveat`. Production therefore arms
//! an APFS vnode fence around the exact enrolled executable and its ancestry,
//! launches that path suspended, and validates the kernel-owned process image
//! before permitting any user-space instruction to execute.

use std::collections::BTreeSet;
use std::ffi::{CStr, CString, OsStr, c_void};
use std::fs::{self, File};
use std::io;
use std::mem::{MaybeUninit, size_of};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::ptr;
use std::sync::Mutex;

use core_foundation::base::{CFGetTypeID, TCFType};
use core_foundation::data::{CFData, CFDataGetLength, CFDataGetTypeID, CFDataRef};
use core_foundation::dictionary::{CFDictionary, CFDictionaryGetValueIfPresent, CFDictionaryRef};
use security_framework::os::macos::code_signing::{Flags, GuestAttributes, SecCode};

use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl, open};
use nix::libc;
use nix::libc::{
    posix_spawn_file_actions_addclose as spawn_actions_add_close,
    posix_spawn_file_actions_adddup2 as spawn_actions_add_dup2,
    posix_spawn_file_actions_addopen as spawn_actions_add_open,
    posix_spawn_file_actions_destroy as spawn_actions_destroy,
    posix_spawn_file_actions_init as spawn_actions_init,
    posix_spawn_file_actions_t as RawSpawnFileActions,
    posix_spawnattr_destroy as spawn_attributes_destroy,
    posix_spawnattr_init as spawn_attributes_init,
    posix_spawnattr_setflags as spawn_attributes_set_flags,
    posix_spawnattr_setpgroup as spawn_attributes_set_process_group,
    posix_spawnattr_setsigdefault as spawn_attributes_set_default_signals,
    posix_spawnattr_setsigmask as spawn_attributes_set_signal_mask,
    posix_spawnattr_t as RawSpawnAttributes,
};
use nix::mount::MntFlags;
use nix::sys::event::{EvFlags, EventFilter, FilterFlag, KEvent, Kqueue};
use nix::sys::signal::{Signal, kill, killpg};
use nix::sys::stat::{Mode, fstat};
use nix::sys::statfs::fstatfs;
use nix::unistd::{Pid, pipe};

use crate::path_validation::{CanonicalFileDigestSnapshot, CanonicalScanRoot};
use crate::persistence::CargoCodeSignatureRecord;

const SPAWN_POLICY_REVISION: u32 = 1;
const MAX_EXECUTABLE_ANCESTORS: usize = 64;
const MAX_EXECUTABLE_PATH_BYTES: usize = 64 * 1024;
const SIGNING_INFORMATION: u32 = 1 << 1;
const POSIX_SPAWN_FLAGS: libc::c_short = (libc::POSIX_SPAWN_SETPGROUP
    | libc::POSIX_SPAWN_SETSIGDEF
    | libc::POSIX_SPAWN_SETSIGMASK
    | libc::POSIX_SPAWN_START_SUSPENDED
    | libc::POSIX_SPAWN_CLOEXEC_DEFAULT) as libc::c_short;

#[derive(Debug)]
pub(super) enum CargoSpawnError {
    ExecutableChanged,
    RunningImageChanged,
    WorkingDirectoryChanged,
    PipeConfiguration,
    Spawn(io::Error),
    Process(io::Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CargoSpawnEvidence {
    pub(super) policy_revision: u32,
    pub(super) running_code_directory_hash: Vec<u8>,
}

pub(super) struct ExecutableMutationFence {
    queue: Kqueue,
    _descriptors: Vec<OwnedFd>,
}

static SPAWN_DESCRIPTOR_LOCK: Mutex<()> = Mutex::new(());

pub(super) struct SuspendedCargoChild {
    pid: libc::pid_t,
    stdout: Option<File>,
    stderr: Option<File>,
    evidence: CargoSpawnEvidence,
    reaped: bool,
    resumed: bool,
}

pub(super) struct CargoSpawnRequest<'a> {
    pub(super) executable: &'a Path,
    pub(super) arguments: &'a [&'a OsStr],
    pub(super) current_directory: BorrowedFd<'a>,
    pub(super) expected_current_directory: &'a CanonicalScanRoot,
    pub(super) home: &'a Path,
    pub(super) cargo_home: &'a Path,
    pub(super) temporary_directory: &'a Path,
    pub(super) expected_signature: &'a CargoCodeSignatureRecord,
}

impl ExecutableMutationFence {
    pub(super) fn capture(
        parent: &CanonicalScanRoot,
        executable: &Path,
        file: &CanonicalFileDigestSnapshot,
    ) -> Result<Self, CargoSpawnError> {
        let mut descriptors = watched_ancestor_directories(parent)?;
        let executable_fd = open(
            executable,
            OFlag::from_bits_retain(libc::O_EVTONLY) | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| CargoSpawnError::ExecutableChanged)?;
        require_reviewed_filesystem(&executable_fd)?;
        let status = fstat(&executable_fd).map_err(|_| CargoSpawnError::ExecutableChanged)?;
        let expected = file.path();
        if status.st_dev as u64 != expected.target_identity().volume()
            || u128::from(status.st_ino) != expected.target_identity().object()
            || status.st_nlink as u64 != expected.hard_link_count()
            || status.st_size < 0
            || status.st_size as u64 != file.byte_length()
        {
            return Err(CargoSpawnError::ExecutableChanged);
        }

        descriptors.push(executable_fd);
        let queue = Kqueue::new().map_err(|_| CargoSpawnError::ExecutableChanged)?;
        fcntl(&queue, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
            .map_err(|_| CargoSpawnError::ExecutableChanged)?;
        let ancestor_flags =
            FilterFlag::NOTE_DELETE | FilterFlag::NOTE_RENAME | FilterFlag::NOTE_REVOKE;
        let executable_flags = FilterFlag::NOTE_DELETE
            | FilterFlag::NOTE_WRITE
            | FilterFlag::NOTE_EXTEND
            | FilterFlag::NOTE_LINK
            | FilterFlag::NOTE_RENAME
            | FilterFlag::NOTE_REVOKE;
        let changes: Vec<KEvent> = descriptors
            .iter()
            .enumerate()
            .map(|(index, descriptor)| {
                // Higher-directory NOTE_WRITE/NOTE_LINK events describe
                // unrelated child activity (especially at `/` and `/Users`),
                // not replacement of the retained ancestor itself. Rename,
                // delete, and revoke remain terminal on every ancestor. A
                // successful exec also updates executable access attributes,
                // so only its content/link/rename/delete/revoke events are
                // terminal.
                let flags = if index + 1 == descriptors.len() {
                    executable_flags
                } else {
                    ancestor_flags
                };
                KEvent::new(
                    descriptor.as_raw_fd() as usize,
                    EventFilter::EVFILT_VNODE,
                    EvFlags::EV_ADD | EvFlags::EV_ENABLE | EvFlags::EV_CLEAR,
                    flags,
                    0,
                    0,
                )
            })
            .collect();
        let mut events = vec![empty_event(); changes.len()];
        let count = queue
            .kevent(&changes, &mut events, Some(zero_timeout()))
            .map_err(|_| CargoSpawnError::ExecutableChanged)?;
        if count != 0 {
            return Err(CargoSpawnError::ExecutableChanged);
        }
        Ok(Self {
            queue,
            _descriptors: descriptors,
        })
    }

    pub(super) fn poll(&self) -> Result<(), CargoSpawnError> {
        let mut event = [empty_event()];
        let count = self
            .queue
            .kevent(&[], &mut event, Some(zero_timeout()))
            .map_err(|_| CargoSpawnError::ExecutableChanged)?;
        if count == 0 {
            Ok(())
        } else {
            Err(CargoSpawnError::ExecutableChanged)
        }
    }
}

impl SuspendedCargoChild {
    pub(super) fn spawn(request: CargoSpawnRequest<'_>) -> Result<Self, CargoSpawnError> {
        let CargoSpawnRequest {
            executable,
            arguments,
            current_directory,
            expected_current_directory,
            home,
            cargo_home,
            temporary_directory,
            expected_signature,
        } = request;
        let _spawn_descriptor_guard = SPAWN_DESCRIPTOR_LOCK
            .lock()
            .map_err(|_| CargoSpawnError::PipeConfiguration)?;
        let executable = path_cstring(executable).map_err(CargoSpawnError::Spawn)?;
        let arguments = argument_vector(&executable, arguments).map_err(CargoSpawnError::Spawn)?;
        let environment = environment_vector(home, cargo_home, temporary_directory)
            .map_err(CargoSpawnError::Spawn)?;
        let (stdout_read, stdout_write) = create_pipe()?;
        let (stderr_read, stderr_write) = create_pipe()?;
        let spawn_current_directory = duplicate_above_stdio(current_directory)?;
        let actions = SpawnFileActions::new()?;
        actions.add_open_stdin()?;
        actions.add_dup2(stdout_write.as_raw_fd(), libc::STDOUT_FILENO)?;
        actions.add_dup2(stderr_write.as_raw_fd(), libc::STDERR_FILENO)?;
        actions.add_inherit(spawn_current_directory.as_raw_fd())?;
        actions.add_fchdir(spawn_current_directory.as_raw_fd())?;
        actions.add_close(spawn_current_directory.as_raw_fd())?;
        actions.add_close(stdout_write.as_raw_fd())?;
        actions.add_close(stderr_write.as_raw_fd())?;

        let attributes = SpawnAttributes::new()?;
        let mut pid = 0;
        let argument_pointers = mutable_pointer_vector(&arguments);
        let environment_pointers = mutable_pointer_vector(&environment);
        // SAFETY: all C strings and pointer vectors remain alive through the
        // call; actions/attributes are initialized opaque Darwin objects; the
        // output PID points to writable storage.
        let result = unsafe {
            // DUX-DESTRUCTIVE: allow=cargo-suspended-observer-spawn -- direct suspended launch of only the exact enrolled Cargo path with fixed arguments/environment, dynamic running-code attestation before SIGCONT, bounded output, and process-group termination; this remains observation only
            libc::posix_spawn(
                &raw mut pid,
                executable.as_ptr(),
                actions.as_ptr(),
                attributes.as_ptr(),
                argument_pointers.as_ptr(),
                environment_pointers.as_ptr(),
            )
        };
        if result != 0 {
            return Err(CargoSpawnError::Spawn(io::Error::from_raw_os_error(result)));
        }
        drop(stdout_write);
        drop(stderr_write);

        let mut child = Self {
            pid,
            stdout: Some(File::from(stdout_read)),
            stderr: Some(File::from(stderr_read)),
            evidence: CargoSpawnEvidence {
                policy_revision: SPAWN_POLICY_REVISION,
                running_code_directory_hash: Vec::new(),
            },
            reaped: false,
            resumed: false,
        };
        let process_start = child.verify_kernel_process(expected_current_directory)?;
        child.evidence.running_code_directory_hash =
            inspect_running_code(pid, executable.as_c_str(), expected_signature)?;
        if child.verify_kernel_process(expected_current_directory)? != process_start {
            return Err(CargoSpawnError::RunningImageChanged);
        }
        Ok(child)
    }

    pub(super) fn pid(&self) -> libc::pid_t {
        self.pid
    }

    pub(super) fn take_stdout(&mut self) -> Option<File> {
        self.stdout.take()
    }

    pub(super) fn take_stderr(&mut self) -> Option<File> {
        self.stderr.take()
    }

    pub(super) fn evidence(&self) -> &CargoSpawnEvidence {
        &self.evidence
    }

    pub(super) fn resume(&mut self) -> Result<(), CargoSpawnError> {
        if self.reaped || self.resumed {
            return Err(CargoSpawnError::Process(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cargo child is not suspended",
            )));
        }
        kill(Pid::from_raw(self.pid), Signal::SIGCONT).map_err(|error| {
            CargoSpawnError::Process(io::Error::from_raw_os_error(error as i32))
        })?;
        self.resumed = true;
        Ok(())
    }

    pub(super) fn exited_without_reaping(&self) -> Result<bool, io::Error> {
        let process_id = libc::id_t::try_from(self.pid)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid child PID"))?;
        let mut information = MaybeUninit::<libc::siginfo_t>::zeroed();
        // SAFETY: the writable siginfo storage lives through the call. WNOWAIT
        // retains this direct child's PID until the later exact waitpid.
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
        // SAFETY: successful waitid initializes the object; no-event is zero.
        Ok(unsafe { information.assume_init().si_pid() != 0 })
    }

    pub(super) fn finish(&mut self) -> Result<ExitStatus, io::Error> {
        let _ = killpg(Pid::from_raw(self.pid), Signal::SIGKILL);
        self.wait()
    }

    pub(super) fn terminate(&mut self) {
        if self.reaped {
            return;
        }
        let _ = killpg(Pid::from_raw(self.pid), Signal::SIGKILL);
        let _ = kill(Pid::from_raw(self.pid), Signal::SIGKILL);
        let _ = self.wait();
    }

    fn wait(&mut self) -> Result<ExitStatus, io::Error> {
        if self.reaped {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cargo child was already reaped",
            ));
        }
        let mut status = 0;
        loop {
            // SAFETY: status points to writable storage and this object owns
            // the unreaped direct-child PID.
            let result = unsafe { libc::waitpid(self.pid, &raw mut status, 0) };
            if result == self.pid {
                self.reaped = true;
                return Ok(ExitStatus::from_raw(status));
            }
            if result == -1 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(if result == -1 {
                io::Error::last_os_error()
            } else {
                io::Error::other("waitpid returned an unexpected PID")
            });
        }
    }

    fn verify_kernel_process(
        &self,
        expected_current_directory: &CanonicalScanRoot,
    ) -> Result<(u64, u64), CargoSpawnError> {
        let bsd = process_info::<libc::proc_bsdinfo>(self.pid, libc::PROC_PIDTBSDINFO)?;
        let parent = unsafe { libc::getpid() };
        if bsd.pbi_pid != self.pid as u32
            || bsd.pbi_ppid != parent as u32
            || bsd.pbi_pgid != self.pid as u32
            || bsd.pbi_status != libc::SSTOP
            || bsd.pbi_uid != unsafe { libc::geteuid() }
            || bsd.pbi_gid != unsafe { libc::getegid() }
            || bsd.pbi_ruid != unsafe { libc::getuid() }
            || bsd.pbi_rgid != unsafe { libc::getgid() }
        {
            return Err(CargoSpawnError::RunningImageChanged);
        }
        let paths =
            process_info::<libc::proc_vnodepathinfo>(self.pid, libc::PROC_PIDVNODEPATHINFO)?;
        let current = paths.pvi_cdir.vip_vi.vi_stat;
        if current.vst_dev as u64 != expected_current_directory.identity().volume()
            || u128::from(current.vst_ino) != expected_current_directory.identity().object()
        {
            return Err(CargoSpawnError::WorkingDirectoryChanged);
        }
        Ok((bsd.pbi_start_tvsec, bsd.pbi_start_tvusec))
    }
}

impl Drop for SuspendedCargoChild {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn inspect_running_code(
    pid: libc::pid_t,
    expected_path: &CStr,
    expected_signature: &CargoCodeSignatureRecord,
) -> Result<Vec<u8>, CargoSpawnError> {
    let runtime_path = process_path(pid)?;
    if runtime_path.as_bytes() != expected_path.to_bytes() {
        return Err(CargoSpawnError::RunningImageChanged);
    }
    let mut attributes = GuestAttributes::new();
    attributes.set_pid(pid);
    let code = SecCode::copy_guest_with_attribues(None, &attributes, Flags::NONE)
        .map_err(|_| CargoSpawnError::RunningImageChanged)?;
    // A null requirement requests dynamic validity of this exact kernel guest.
    // SecCode's dynamic validity API requires default flags on the supported
    // deployment range; the strict/all-architecture/no-network flags used for
    // static enrollment return errSecCSInvalidFlags here. The enrolled static
    // record was already produced under that stricter no-network policy.
    let status = unsafe {
        SecCodeCheckValidity(
            code.as_CFTypeRef().cast_mut(),
            Flags::NONE.bits(),
            ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(CargoSpawnError::RunningImageChanged);
    }
    let mut raw_information: CFDictionaryRef = ptr::null();
    let status = unsafe {
        SecCodeCopySigningInformation(
            code.as_CFTypeRef().cast_mut(),
            SIGNING_INFORMATION,
            &raw mut raw_information,
        )
    };
    if status != 0 || raw_information.is_null() {
        return Err(CargoSpawnError::RunningImageChanged);
    }
    let information: CFDictionary = unsafe { TCFType::wrap_under_create_rule(raw_information) };
    let unique = dictionary_data(&information, unsafe { kSecCodeInfoUnique })?;
    if !expected_signature
        .code_directory_hashes
        .iter()
        .any(|expected| expected == &unique)
    {
        return Err(CargoSpawnError::RunningImageChanged);
    }
    Ok(unique)
}

fn process_path(pid: libc::pid_t) -> Result<CString, CargoSpawnError> {
    let mut bytes = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer is writable and its exact size is supplied.
    let count = unsafe {
        libc::proc_pidpath(
            pid,
            bytes.as_mut_ptr().cast(),
            libc::PROC_PIDPATHINFO_MAXSIZE as u32,
        )
    };
    if count <= 0 {
        return Err(CargoSpawnError::RunningImageChanged);
    }
    let nul = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(CargoSpawnError::RunningImageChanged)?;
    bytes.truncate(nul);
    CString::new(bytes).map_err(|_| CargoSpawnError::RunningImageChanged)
}

fn process_info<T>(pid: libc::pid_t, flavor: libc::c_int) -> Result<T, CargoSpawnError> {
    let mut information = MaybeUninit::<T>::zeroed();
    let size =
        libc::c_int::try_from(size_of::<T>()).map_err(|_| CargoSpawnError::RunningImageChanged)?;
    // SAFETY: the destination has exactly `size_of::<T>()` writable bytes and
    // the caller selects the matching fixed-size proc_pidinfo flavor.
    let count =
        unsafe { libc::proc_pidinfo(pid, flavor, 0, information.as_mut_ptr().cast(), size) };
    if count != size {
        return Err(CargoSpawnError::RunningImageChanged);
    }
    Ok(unsafe { information.assume_init() })
}

fn dictionary_data(
    dictionary: &CFDictionary,
    key: core_foundation::string::CFStringRef,
) -> Result<Vec<u8>, CargoSpawnError> {
    if key.is_null() {
        return Err(CargoSpawnError::RunningImageChanged);
    }
    let mut value: *const c_void = ptr::null();
    let present = unsafe {
        CFDictionaryGetValueIfPresent(dictionary.as_concrete_TypeRef(), key.cast(), &raw mut value)
    };
    if present == 0
        || value.is_null()
        || unsafe { CFGetTypeID(value.cast()) } != unsafe { CFDataGetTypeID() }
    {
        return Err(CargoSpawnError::RunningImageChanged);
    }
    let data_ref: CFDataRef = value.cast();
    let length = usize::try_from(unsafe { CFDataGetLength(data_ref) })
        .map_err(|_| CargoSpawnError::RunningImageChanged)?;
    if !(20..=64).contains(&length) {
        return Err(CargoSpawnError::RunningImageChanged);
    }
    Ok(unsafe { CFData::wrap_under_get_rule(data_ref) }
        .bytes()
        .to_vec())
}

fn create_pipe() -> Result<(OwnedFd, OwnedFd), CargoSpawnError> {
    let (read, write) = pipe().map_err(|_| CargoSpawnError::PipeConfiguration)?;
    fcntl(&read, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
        .map_err(|_| CargoSpawnError::PipeConfiguration)?;
    fcntl(&write, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
        .map_err(|_| CargoSpawnError::PipeConfiguration)?;
    Ok((move_above_stdio(read)?, move_above_stdio(write)?))
}

fn move_above_stdio(descriptor: OwnedFd) -> Result<OwnedFd, CargoSpawnError> {
    if descriptor.as_raw_fd() > libc::STDERR_FILENO {
        return Ok(descriptor);
    }
    let duplicated = fcntl(&descriptor, FcntlArg::F_DUPFD_CLOEXEC(3))
        .map_err(|_| CargoSpawnError::PipeConfiguration)?;
    // SAFETY: F_DUPFD_CLOEXEC returned a new owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(duplicated) })
}

fn duplicate_above_stdio(descriptor: BorrowedFd<'_>) -> Result<OwnedFd, CargoSpawnError> {
    let duplicated = fcntl(descriptor, FcntlArg::F_DUPFD_CLOEXEC(3))
        .map_err(|_| CargoSpawnError::PipeConfiguration)?;
    // SAFETY: F_DUPFD_CLOEXEC returned a new owned descriptor greater than
    // every standard descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(duplicated) })
}

fn argument_vector(executable: &CStr, arguments: &[&OsStr]) -> io::Result<Vec<CString>> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(arguments.len().saturating_add(1))
        .map_err(|_| io::Error::other("Cargo argument allocation failed"))?;
    result.push(executable.to_owned());
    for argument in arguments {
        result.push(CString::new(argument.as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "Cargo argument contains NUL")
        })?);
    }
    Ok(result)
}

fn environment_vector(
    home: &Path,
    cargo_home: &Path,
    temporary: &Path,
) -> io::Result<Vec<CString>> {
    let mut result = Vec::with_capacity(9);
    result.push(environment_path("HOME", home)?);
    result.push(environment_path("CARGO_HOME", cargo_home)?);
    result.push(environment_path("TMPDIR", temporary)?);
    for value in [
        b"PATH=/dev/null".as_slice(),
        b"LC_ALL=C",
        b"LANG=C",
        b"CARGO_NET_OFFLINE=true",
        b"CARGO_TERM_COLOR=never",
        b"CARGO_LOG=cargo::util::context=debug",
    ] {
        result.push(CString::new(value).expect("fixed Cargo environment contains no NUL"));
    }
    Ok(result)
}

fn environment_path(name: &str, path: &Path) -> io::Result<CString> {
    let mut bytes = Vec::with_capacity(name.len() + 1 + path.as_os_str().as_bytes().len());
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(b'=');
    bytes.extend_from_slice(path.as_os_str().as_bytes());
    CString::new(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "environment path contains NUL"))
}

fn mutable_pointer_vector(values: &[CString]) -> Vec<*mut libc::c_char> {
    values
        .iter()
        .map(|value| value.as_ptr().cast_mut())
        .chain(std::iter::once(ptr::null_mut()))
        .collect()
}

fn path_cstring(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
}

struct SpawnFileActions {
    raw: RawSpawnFileActions,
}

impl SpawnFileActions {
    fn new() -> Result<Self, CargoSpawnError> {
        let mut raw = ptr::null_mut();
        let result = unsafe { spawn_actions_init(&raw mut raw) };
        if result == 0 {
            Ok(Self { raw })
        } else {
            Err(CargoSpawnError::Spawn(io::Error::from_raw_os_error(result)))
        }
    }

    fn as_ptr(&self) -> *const RawSpawnFileActions {
        &raw const self.raw
    }

    fn add_open_stdin(&self) -> Result<(), CargoSpawnError> {
        let path = c"/dev/null";
        check_spawn_setup(unsafe {
            spawn_actions_add_open(
                self.raw_mut(),
                libc::STDIN_FILENO,
                path.as_ptr(),
                libc::O_RDONLY,
                0,
            )
        })
    }

    fn add_dup2(&self, descriptor: RawFd, target: RawFd) -> Result<(), CargoSpawnError> {
        check_spawn_setup(unsafe { spawn_actions_add_dup2(self.raw_mut(), descriptor, target) })
    }

    fn add_close(&self, descriptor: RawFd) -> Result<(), CargoSpawnError> {
        check_spawn_setup(unsafe { spawn_actions_add_close(self.raw_mut(), descriptor) })
    }

    fn add_inherit(&self, descriptor: RawFd) -> Result<(), CargoSpawnError> {
        check_spawn_setup(unsafe {
            posix_spawn_file_actions_addinherit_np(self.raw_mut(), descriptor)
        })
    }

    fn add_fchdir(&self, descriptor: RawFd) -> Result<(), CargoSpawnError> {
        check_spawn_setup(unsafe {
            posix_spawn_file_actions_addfchdir_np(self.raw_mut(), descriptor)
        })
    }

    fn raw_mut(&self) -> *mut RawSpawnFileActions {
        (&raw const self.raw).cast_mut()
    }
}

impl Drop for SpawnFileActions {
    fn drop(&mut self) {
        let _ = unsafe { spawn_actions_destroy(&raw mut self.raw) };
    }
}

struct SpawnAttributes {
    raw: RawSpawnAttributes,
}

impl SpawnAttributes {
    fn new() -> Result<Self, CargoSpawnError> {
        let mut raw = ptr::null_mut();
        let result = unsafe { spawn_attributes_init(&raw mut raw) };
        if result != 0 {
            return Err(CargoSpawnError::Spawn(io::Error::from_raw_os_error(result)));
        }
        let attributes = Self { raw };
        check_spawn_setup(unsafe {
            spawn_attributes_set_flags(attributes.raw_mut(), POSIX_SPAWN_FLAGS)
        })?;
        check_spawn_setup(unsafe { spawn_attributes_set_process_group(attributes.raw_mut(), 0) })?;
        let mut default_signals = MaybeUninit::<libc::sigset_t>::zeroed();
        let mut signal_mask = MaybeUninit::<libc::sigset_t>::zeroed();
        unsafe {
            libc::sigfillset(default_signals.as_mut_ptr());
            libc::sigdelset(default_signals.as_mut_ptr(), libc::SIGKILL);
            libc::sigdelset(default_signals.as_mut_ptr(), libc::SIGSTOP);
            libc::sigemptyset(signal_mask.as_mut_ptr());
            check_spawn_setup(spawn_attributes_set_default_signals(
                attributes.raw_mut(),
                default_signals.as_ptr(),
            ))?;
            check_spawn_setup(spawn_attributes_set_signal_mask(
                attributes.raw_mut(),
                signal_mask.as_ptr(),
            ))?;
        }
        Ok(attributes)
    }

    fn as_ptr(&self) -> *const RawSpawnAttributes {
        &raw const self.raw
    }

    fn raw_mut(&self) -> *mut RawSpawnAttributes {
        (&raw const self.raw).cast_mut()
    }
}

impl Drop for SpawnAttributes {
    fn drop(&mut self) {
        let _ = unsafe { spawn_attributes_destroy(&raw mut self.raw) };
    }
}

fn check_spawn_setup(result: libc::c_int) -> Result<(), CargoSpawnError> {
    if result == 0 {
        Ok(())
    } else {
        Err(CargoSpawnError::Spawn(io::Error::from_raw_os_error(result)))
    }
}

fn watched_ancestor_directories(
    expected_parent: &CanonicalScanRoot,
) -> Result<Vec<OwnedFd>, CargoSpawnError> {
    let mut descriptors = Vec::new();
    let mut identities = BTreeSet::new();
    let mut native_path_bytes = 0_usize;
    for path in expected_parent.canonical_path().ancestors() {
        if descriptors.len() == MAX_EXECUTABLE_ANCESTORS {
            return Err(CargoSpawnError::ExecutableChanged);
        }
        native_path_bytes = native_path_bytes
            .checked_add(path.as_os_str().as_bytes().len())
            .ok_or(CargoSpawnError::ExecutableChanged)?;
        if native_path_bytes > MAX_EXECUTABLE_PATH_BYTES {
            return Err(CargoSpawnError::ExecutableChanged);
        }
        let metadata =
            fs::symlink_metadata(path).map_err(|_| CargoSpawnError::ExecutableChanged)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(CargoSpawnError::ExecutableChanged);
        }
        let canonical = fs::canonicalize(path).map_err(|_| CargoSpawnError::ExecutableChanged)?;
        if canonical != path {
            return Err(CargoSpawnError::ExecutableChanged);
        }
        let descriptor = open(
            path,
            OFlag::from_bits_retain(libc::O_EVTONLY) | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| CargoSpawnError::ExecutableChanged)?;
        require_reviewed_filesystem(&descriptor)?;
        let status = fstat(&descriptor).map_err(|_| CargoSpawnError::ExecutableChanged)?;
        let identity = (status.st_dev as u64, u128::from(status.st_ino));
        if path == expected_parent.canonical_path()
            && (identity.0 != expected_parent.identity().volume()
                || identity.1 != expected_parent.identity().object())
        {
            return Err(CargoSpawnError::ExecutableChanged);
        }
        if identities.insert(identity) {
            descriptors.push(descriptor);
        }
    }
    if descriptors.is_empty() {
        return Err(CargoSpawnError::ExecutableChanged);
    }
    Ok(descriptors)
}

fn require_reviewed_filesystem(descriptor: &impl AsFd) -> Result<(), CargoSpawnError> {
    let status = fstatfs(descriptor).map_err(|_| CargoSpawnError::ExecutableChanged)?;
    if status.filesystem_type_name().eq_ignore_ascii_case("apfs")
        && status.flags().contains(MntFlags::MNT_LOCAL)
    {
        Ok(())
    } else {
        Err(CargoSpawnError::ExecutableChanged)
    }
}

fn empty_event() -> KEvent {
    KEvent::new(
        0,
        EventFilter::EVFILT_VNODE,
        EvFlags::empty(),
        FilterFlag::empty(),
        0,
        0,
    )
}

fn zero_timeout() -> libc::timespec {
    libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    }
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecCodeCheckValidity(code: *mut c_void, flags: u32, requirement: *mut c_void) -> i32;
    fn SecCodeCopySigningInformation(
        code: *mut c_void,
        flags: u32,
        information: *mut CFDictionaryRef,
    ) -> i32;
    static kSecCodeInfoUnique: core_foundation::string::CFStringRef;
}

unsafe extern "C" {
    fn posix_spawn_file_actions_addinherit_np(
        actions: *mut RawSpawnFileActions,
        descriptor: libc::c_int,
    ) -> libc::c_int;
    fn posix_spawn_file_actions_addfchdir_np(
        actions: *mut RawSpawnFileActions,
        descriptor: libc::c_int,
    ) -> libc::c_int;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Read;
    use std::os::fd::AsFd;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;
    use crate::path_validation::{
        capture_regular_file_sha256, capture_scan_root, validate_cleanup_path, validate_scan_root,
    };
    use crate::planner::cargo_code_signature_macos::inspect_cargo_code_signature;

    fn retained_directory(path: &Path) -> (CanonicalScanRoot, OwnedFd) {
        let root = capture_scan_root(validate_scan_root(path).unwrap()).unwrap();
        let descriptor = open(
            root.canonical_path(),
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        (root, descriptor)
    }

    fn wait_for_exit(child: &SuspendedCargoChild) {
        for _ in 0..1_000 {
            if child.exited_without_reaping().unwrap() {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!("signed helper did not exit");
    }

    #[test]
    fn suspended_child_cannot_run_before_attestation_and_uses_retained_cwd() {
        let temp = TempDir::new().unwrap();
        let cwd = fs::canonicalize(temp.path()).unwrap();
        let (root, directory) = retained_directory(&cwd);
        let sentinel = cwd.join("created");
        let executable = Path::new("/usr/bin/touch");
        let signature = inspect_cargo_code_signature(executable).unwrap();
        let mut child = SuspendedCargoChild::spawn(CargoSpawnRequest {
            executable,
            arguments: &[sentinel.as_os_str()],
            current_directory: directory.as_fd(),
            expected_current_directory: &root,
            home: &cwd,
            cargo_home: &cwd,
            temporary_directory: &cwd,
            expected_signature: &signature,
        })
        .unwrap();
        assert!(!sentinel.exists());
        assert_eq!(child.evidence().policy_revision, SPAWN_POLICY_REVISION);
        assert!(!child.evidence().running_code_directory_hash.is_empty());
        child.resume().unwrap();
        wait_for_exit(&child);
        assert!(child.finish().unwrap().success());
        assert!(sentinel.exists());
    }

    #[test]
    fn descriptor_selected_cwd_is_visible_to_child() {
        let temp = TempDir::new().unwrap();
        let cwd = fs::canonicalize(temp.path()).unwrap();
        let (root, directory) = retained_directory(&cwd);
        let executable = Path::new("/bin/pwd");
        let signature = inspect_cargo_code_signature(executable).unwrap();
        let mut child = SuspendedCargoChild::spawn(CargoSpawnRequest {
            executable,
            arguments: &[],
            current_directory: directory.as_fd(),
            expected_current_directory: &root,
            home: &cwd,
            cargo_home: &cwd,
            temporary_directory: &cwd,
            expected_signature: &signature,
        })
        .unwrap();
        let mut stdout = child.take_stdout().unwrap();
        child.resume().unwrap();
        wait_for_exit(&child);
        assert!(child.finish().unwrap().success());
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).unwrap();
        assert_eq!(output, format!("{}\n", cwd.display()).as_bytes());
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test launches only its exact current test binary to isolate deliberately closed standard descriptors"
    )]
    fn closed_standard_descriptors_cannot_alias_the_retained_cwd() {
        const HELPER_ENVIRONMENT: &str = "DUX_TEST_CARGO_CLOSED_STDIO_HELPER";
        if std::env::var_os(HELPER_ENVIRONMENT).is_some() {
            run_closed_standard_descriptor_helper();
            return;
        }
        // DUX-DESTRUCTIVE: allow=test-cargo-closed-stdio-helper-spawn -- launch only this exact test binary with one exact test filter to isolate process-global descriptor closure
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(
                "planner::cargo_spawn_macos::tests::closed_standard_descriptors_cannot_alias_the_retained_cwd",
            )
            .env(HELPER_ENVIRONMENT, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn run_closed_standard_descriptor_helper() {
        let temp = TempDir::new().unwrap();
        let cwd = fs::canonicalize(temp.path()).unwrap();
        let (root, directory) = retained_directory(&cwd);
        let executable = Path::new("/bin/pwd");
        let signature = inspect_cargo_code_signature(executable).unwrap();
        // SAFETY: this helper runs in a dedicated subprocess. It deliberately
        // places the retained cwd at descriptor 0 and closes 1/2 to reproduce
        // a GUI-style process with unavailable standard descriptors.
        unsafe {
            assert_eq!(libc::dup2(directory.as_raw_fd(), libc::STDIN_FILENO), 0);
            assert_eq!(libc::close(libc::STDOUT_FILENO), 0);
            assert_eq!(libc::close(libc::STDERR_FILENO), 0);
        }
        // SAFETY: descriptor 0 is the duplicate created immediately above and
        // remains open through the synchronous spawn call.
        let current_directory = unsafe { BorrowedFd::borrow_raw(libc::STDIN_FILENO) };
        let mut child = SuspendedCargoChild::spawn(CargoSpawnRequest {
            executable,
            arguments: &[],
            current_directory,
            expected_current_directory: &root,
            home: &cwd,
            cargo_home: &cwd,
            temporary_directory: &cwd,
            expected_signature: &signature,
        })
        .unwrap();
        let mut stdout = child.take_stdout().unwrap();
        child.resume().unwrap();
        wait_for_exit(&child);
        assert!(child.finish().unwrap().success());
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).unwrap();
        assert_eq!(output, format!("{}\n", cwd.display()).as_bytes());
    }

    #[test]
    fn mismatched_running_code_never_reaches_user_space() {
        let temp = TempDir::new().unwrap();
        let cwd = fs::canonicalize(temp.path()).unwrap();
        let (root, directory) = retained_directory(&cwd);
        let sentinel = cwd.join("must-not-exist");
        let expected_signature = inspect_cargo_code_signature(Path::new("/usr/bin/false")).unwrap();
        let result = SuspendedCargoChild::spawn(CargoSpawnRequest {
            executable: Path::new("/usr/bin/touch"),
            arguments: &[sentinel.as_os_str()],
            current_directory: directory.as_fd(),
            expected_current_directory: &root,
            home: &cwd,
            cargo_home: &cwd,
            temporary_directory: &cwd,
            expected_signature: &expected_signature,
        });
        assert!(matches!(result, Err(CargoSpawnError::RunningImageChanged)));
        assert!(!sentinel.exists());
    }

    #[test]
    fn cloexec_default_excludes_an_explicitly_inheritable_parent_descriptor() {
        let temp = TempDir::new().unwrap();
        let cwd = fs::canonicalize(temp.path()).unwrap();
        let (root, directory) = retained_directory(&cwd);
        let inherited = File::open("/dev/null").unwrap();
        fcntl(&inherited, FcntlArg::F_SETFD(FdFlag::empty())).unwrap();
        let descriptor = inherited.as_raw_fd();
        let cwd_descriptor = directory.as_raw_fd();
        let script =
            format!("test ! -e /dev/fd/{descriptor} && test ! -e /dev/fd/{cwd_descriptor}");
        let executable = Path::new("/bin/sh");
        let signature = inspect_cargo_code_signature(executable).unwrap();
        let mut child = SuspendedCargoChild::spawn(CargoSpawnRequest {
            executable,
            arguments: &[OsStr::new("-c"), OsStr::new(&script)],
            current_directory: directory.as_fd(),
            expected_current_directory: &root,
            home: &cwd,
            cargo_home: &cwd,
            temporary_directory: &cwd,
            expected_signature: &signature,
        })
        .unwrap();
        child.resume().unwrap();
        wait_for_exit(&child);
        assert!(child.finish().unwrap().success());
    }

    #[test]
    fn termination_reaps_the_exact_resumed_child() {
        let temp = TempDir::new().unwrap();
        let cwd = fs::canonicalize(temp.path()).unwrap();
        let (root, directory) = retained_directory(&cwd);
        let executable = Path::new("/bin/sleep");
        let signature = inspect_cargo_code_signature(executable).unwrap();
        let mut child = SuspendedCargoChild::spawn(CargoSpawnRequest {
            executable,
            arguments: &[OsStr::new("30")],
            current_directory: directory.as_fd(),
            expected_current_directory: &root,
            home: &cwd,
            cargo_home: &cwd,
            temporary_directory: &cwd,
            expected_signature: &signature,
        })
        .unwrap();
        let pid = child.pid();
        child.resume().unwrap();
        child.terminate();
        assert!(child.reaped);
        assert!(matches!(
            kill(Pid::from_raw(pid), None),
            Err(nix::errno::Errno::ESRCH)
        ));
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test renames only a TempDir-owned executable ancestor to prove the retained vnode fence is terminal"
    )]
    fn higher_executable_ancestor_rename_is_terminal() {
        let temp = TempDir::new().unwrap();
        let base = fs::canonicalize(temp.path()).unwrap();
        let toolchain = base.join("toolchain");
        let parent_path = toolchain.join("bin");
        fs::create_dir_all(&parent_path).unwrap();
        let executable = parent_path.join("cargo");
        fs::write(&executable, b"fixture").unwrap();
        let lexical_parent = validate_scan_root(&parent_path).unwrap();
        let parent = capture_scan_root(lexical_parent.clone()).unwrap();
        let lexical_file = validate_cleanup_path(&lexical_parent, &executable).unwrap();
        let file = capture_regular_file_sha256(&parent, lexical_file, 1024).unwrap();
        let fence = ExecutableMutationFence::capture(&parent, &executable, &file).unwrap();

        // DUX-DESTRUCTIVE: allow=test-cargo-executable-ancestor-rename -- rename only the TempDir-owned ancestor after arming its vnode fence
        fs::rename(&toolchain, base.join("moved-toolchain")).unwrap();
        assert!(matches!(
            fence.poll(),
            Err(CargoSpawnError::ExecutableChanged)
        ));
    }

    #[test]
    fn in_place_executable_write_is_terminal() {
        let temp = TempDir::new().unwrap();
        let parent_path = fs::canonicalize(temp.path()).unwrap();
        let executable = parent_path.join("cargo");
        fs::write(&executable, b"before").unwrap();
        let lexical_parent = validate_scan_root(&parent_path).unwrap();
        let parent = capture_scan_root(lexical_parent.clone()).unwrap();
        let lexical_file = validate_cleanup_path(&lexical_parent, &executable).unwrap();
        let file = capture_regular_file_sha256(&parent, lexical_file, 1024).unwrap();
        let fence = ExecutableMutationFence::capture(&parent, &executable, &file).unwrap();
        fs::write(&executable, b"after").unwrap();
        assert!(matches!(
            fence.poll(),
            Err(CargoSpawnError::ExecutableChanged)
        ));
    }
}
