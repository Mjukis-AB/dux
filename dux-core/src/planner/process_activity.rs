//! Bounded, observation-only process activity evidence.
//!
//! Rule documents can describe process guards, but the discovery evaluator
//! deliberately rejects them until a live provider exists.  This module is
//! the first provider seam: it captures a bounded macOS process table without
//! invoking a shell, keeps PID/start-time/executable identity private, and
//! requires a fresh enumeration for every revalidation.  It is not an
//! authorization or a proof that a process cannot retain an already-open file
//! descriptor; the future planner and executor must keep those limits visible.

use std::cmp::Ordering;
use std::mem::MaybeUninit;
use std::path::PathBuf;

use thiserror::Error;

use crate::domain::ActivityGuard;

const PROCESS_ACTIVITY_PROOF_REVISION: u32 = 1;
const MAX_ACTIVITY_GUARDS: usize = 32;
const MAX_PROCESS_RECORDS: usize = 16_384;
const MAX_PROCESS_NAME_BYTES: usize = 255;
const MAX_EXECUTABLE_PATH_BYTES: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct ProcessStartToken {
    seconds: u64,
    microseconds: u32,
}

impl ProcessStartToken {
    fn is_valid(self) -> bool {
        self.seconds != 0 && self.microseconds < 1_000_000
    }
}

/// A process identity is intentionally private and never enters durable or
/// FFI records.  The executable bytes make PID reuse and same-name image
/// replacement visible to the next live check.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcessRecord {
    pid: u32,
    start: ProcessStartToken,
    name: String,
    executable: PathBuf,
}

impl Ord for ProcessRecord {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.pid, self.start, &self.executable, &self.name).cmp(&(
            other.pid,
            other.start,
            &other.executable,
            &other.name,
        ))
    }
}

impl PartialOrd for ProcessRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

trait ProcessProvider {
    fn enumerate(&self) -> Result<Vec<ProcessRecord>, ProcessActivityError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(crate) enum ProcessActivityError {
    #[error("process activity observation is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("process activity observation requires at least one guard")]
    MissingGuards,
    #[error("process activity observation received too many guards")]
    TooManyGuards,
    #[error("process activity observation received duplicate guards")]
    DuplicateGuard,
    #[error("bundle-identifier activity guards require a signed bundle provider")]
    UnsupportedBundleIdentifier,
    #[error("the process table could not be enumerated completely")]
    EnumerationUnavailable,
    #[error("the process table exceeded the bounded observation budget")]
    ProcessLimitExceeded,
    #[error("the process table contained malformed identity data")]
    MalformedRecord,
    #[error("a guarded process is active")]
    Active,
    #[error("process activity changed after the witness was captured")]
    Changed,
}

/// Non-cloneable evidence that every requested process-name guard was absent
/// in one bounded observation.  Holding it does not authorize a plan or an
/// effect; callers must revalidate immediately before any future mutation.
#[must_use = "process activity evidence must be revalidated at the effect boundary"]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ProcessActivityWitness {
    proof_revision: u32,
    guards: Vec<ActivityGuard>,
    observed: Vec<ProcessRecord>,
}

impl ProcessActivityWitness {
    /// Capture process activity using the platform provider.  Bundle
    /// identifiers are rejected until a provider can bind a signed bundle
    /// identity rather than guessing from a process name.
    pub(crate) fn capture(guards: &[ActivityGuard]) -> Result<Self, ProcessActivityError> {
        validate_guards(guards)?;
        let provider = NativeProcessProvider;
        Self::capture_with_provider(guards, &provider)
    }

    fn capture_with_provider(
        guards: &[ActivityGuard],
        provider: &impl ProcessProvider,
    ) -> Result<Self, ProcessActivityError> {
        validate_guards(guards)?;
        let observed = normalized_records(provider.enumerate()?)?;
        ensure_guards_inactive(guards, &observed)?;
        Ok(Self {
            proof_revision: PROCESS_ACTIVITY_PROOF_REVISION,
            guards: guards.to_vec(),
            observed,
        })
    }

    /// Re-enumerate the process table and require the same conservative
    /// inactive result.  The old process list is retained only to make the
    /// witness non-trivial and to ensure the captured policy is not replaced;
    /// the fresh result, not the old list, is the safety decision.
    pub(crate) fn revalidate(&self) -> Result<(), ProcessActivityError> {
        if self.proof_revision != PROCESS_ACTIVITY_PROOF_REVISION {
            return Err(ProcessActivityError::Changed);
        }
        let provider = NativeProcessProvider;
        self.revalidate_with_provider(&provider)
    }

    fn revalidate_with_provider(
        &self,
        provider: &impl ProcessProvider,
    ) -> Result<(), ProcessActivityError> {
        let current = normalized_records(provider.enumerate()?)?;
        ensure_guards_inactive(&self.guards, &current)?;
        // A process that was present in the retained observation must not be
        // silently changed into a different identity while the witness lives.
        // For an inactive guard this is normally empty, but retaining the
        // comparison prevents future provider changes from weakening the
        // boundary without a revision bump.
        if self
            .observed
            .iter()
            .any(|old| current.iter().any(|new| new.pid == old.pid && new != old))
        {
            return Err(ProcessActivityError::Changed);
        }
        Ok(())
    }
}

fn validate_guards(guards: &[ActivityGuard]) -> Result<(), ProcessActivityError> {
    if guards.is_empty() {
        return Err(ProcessActivityError::MissingGuards);
    }
    if guards.len() > MAX_ACTIVITY_GUARDS {
        return Err(ProcessActivityError::TooManyGuards);
    }
    for (index, guard) in guards.iter().enumerate() {
        match guard {
            ActivityGuard::ProcessName(name)
                if name.is_empty()
                    || name.len() > MAX_PROCESS_NAME_BYTES
                    || name.chars().any(char::is_control) =>
            {
                return Err(ProcessActivityError::MalformedRecord);
            }
            ActivityGuard::ProcessName(_) => {}
            ActivityGuard::BundleIdentifier(_) => {
                return Err(ProcessActivityError::UnsupportedBundleIdentifier);
            }
        }
        if guards[..index].iter().any(|previous| previous == guard) {
            return Err(ProcessActivityError::DuplicateGuard);
        }
    }
    Ok(())
}

fn normalized_records(
    mut records: Vec<ProcessRecord>,
) -> Result<Vec<ProcessRecord>, ProcessActivityError> {
    if records.len() > MAX_PROCESS_RECORDS {
        return Err(ProcessActivityError::ProcessLimitExceeded);
    }
    for record in &records {
        if record.pid == 0
            || !record.start.is_valid()
            || record.name.is_empty()
            || record.name.len() > MAX_PROCESS_NAME_BYTES
            || record.executable.as_os_str().is_empty()
            || record.executable.as_os_str().len() > MAX_EXECUTABLE_PATH_BYTES
            || !record.executable.is_absolute()
        {
            return Err(ProcessActivityError::MalformedRecord);
        }
    }
    records.sort_unstable();
    if records.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ProcessActivityError::MalformedRecord);
    }
    Ok(records)
}

fn ensure_guards_inactive(
    guards: &[ActivityGuard],
    records: &[ProcessRecord],
) -> Result<(), ProcessActivityError> {
    for guard in guards {
        match guard {
            ActivityGuard::ProcessName(name)
                if records.iter().any(|record| record.name == *name) =>
            {
                return Err(ProcessActivityError::Active);
            }
            ActivityGuard::ProcessName(_) => {}
            ActivityGuard::BundleIdentifier(_) => {
                return Err(ProcessActivityError::UnsupportedBundleIdentifier);
            }
        }
    }
    Ok(())
}

struct NativeProcessProvider;

impl ProcessProvider for NativeProcessProvider {
    fn enumerate(&self) -> Result<Vec<ProcessRecord>, ProcessActivityError> {
        #[cfg(target_os = "macos")]
        {
            enumerate_macos_processes()
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(ProcessActivityError::UnsupportedPlatform)
        }
    }
}

#[cfg(target_os = "macos")]
fn enumerate_macos_processes() -> Result<Vec<ProcessRecord>, ProcessActivityError> {
    use std::os::raw::c_void;

    let reported = unsafe { nix::libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if reported <= 0 {
        return Err(ProcessActivityError::EnumerationUnavailable);
    }
    let reported =
        usize::try_from(reported).map_err(|_| ProcessActivityError::EnumerationUnavailable)?;
    if reported > MAX_PROCESS_RECORDS {
        return Err(ProcessActivityError::ProcessLimitExceeded);
    }
    let capacity = reported
        .checked_add(256)
        .ok_or(ProcessActivityError::ProcessLimitExceeded)?;
    let byte_len = capacity
        .checked_mul(std::mem::size_of::<nix::libc::pid_t>())
        .ok_or(ProcessActivityError::ProcessLimitExceeded)?;
    let byte_len =
        i32::try_from(byte_len).map_err(|_| ProcessActivityError::ProcessLimitExceeded)?;
    let mut pids = vec![0 as nix::libc::pid_t; capacity];
    let returned =
        unsafe { nix::libc::proc_listallpids(pids.as_mut_ptr().cast::<c_void>(), byte_len) };
    if returned < 0 {
        return Err(ProcessActivityError::EnumerationUnavailable);
    }
    let returned =
        usize::try_from(returned).map_err(|_| ProcessActivityError::EnumerationUnavailable)?;
    if returned > capacity {
        return Err(ProcessActivityError::EnumerationUnavailable);
    }

    let mut records = Vec::with_capacity(returned);
    for pid in pids.into_iter().take(returned) {
        if pid <= 0 {
            continue;
        }
        let mut info = MaybeUninit::<nix::libc::proc_bsdinfo>::zeroed();
        let size = i32::try_from(std::mem::size_of::<nix::libc::proc_bsdinfo>())
            .map_err(|_| ProcessActivityError::MalformedRecord)?;
        let read = unsafe {
            nix::libc::proc_pidinfo(
                pid,
                nix::libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast::<c_void>(),
                size,
            )
        };
        // A process can exit between proc_listallpids and proc_pidinfo.  It
        // contributes no active evidence and will be checked again on use.
        if read == 0 {
            continue;
        }
        if read != size {
            return Err(ProcessActivityError::EnumerationUnavailable);
        }
        let info = unsafe { info.assume_init() };
        if info.pbi_pid != pid as u32 || info.pbi_start_tvusec >= 1_000_000 {
            return Err(ProcessActivityError::MalformedRecord);
        }
        let name_end = info
            .pbi_name
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(ProcessActivityError::MalformedRecord)?;
        if name_end == 0 {
            return Err(ProcessActivityError::MalformedRecord);
        }
        let name_bytes = info.pbi_name[..name_end]
            .iter()
            .map(|byte| *byte as u8)
            .collect::<Vec<_>>();
        let name =
            String::from_utf8(name_bytes).map_err(|_| ProcessActivityError::MalformedRecord)?;
        if name.len() > MAX_PROCESS_NAME_BYTES {
            return Err(ProcessActivityError::MalformedRecord);
        }
        let mut path = vec![0_u8; MAX_EXECUTABLE_PATH_BYTES];
        let path_len = unsafe {
            nix::libc::proc_pidpath(
                pid,
                path.as_mut_ptr().cast::<c_void>(),
                u32::try_from(path.len()).expect("bounded path length fits u32"),
            )
        };
        if path_len <= 0 {
            return Err(ProcessActivityError::EnumerationUnavailable);
        }
        let path_len =
            usize::try_from(path_len).map_err(|_| ProcessActivityError::MalformedRecord)?;
        if path_len >= path.len() {
            return Err(ProcessActivityError::MalformedRecord);
        }
        path.truncate(path_len);
        let executable = PathBuf::from(
            String::from_utf8(path).map_err(|_| ProcessActivityError::MalformedRecord)?,
        );
        if !executable.is_absolute() {
            return Err(ProcessActivityError::MalformedRecord);
        }
        records.push(ProcessRecord {
            pid: pid as u32,
            start: ProcessStartToken {
                seconds: info.pbi_start_tvsec,
                microseconds: info.pbi_start_tvusec as u32,
            },
            name,
            executable,
        });
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeProvider {
        records: Vec<ProcessRecord>,
    }

    impl ProcessProvider for FakeProvider {
        fn enumerate(&self) -> Result<Vec<ProcessRecord>, ProcessActivityError> {
            Ok(self.records.clone())
        }
    }

    fn record(pid: u32, name: &str) -> ProcessRecord {
        ProcessRecord {
            pid,
            start: ProcessStartToken {
                seconds: 10,
                microseconds: pid,
            },
            name: name.to_owned(),
            executable: PathBuf::from(format!("/usr/bin/{name}")),
        }
    }

    #[test]
    fn inactive_process_name_produces_revalidatable_witness() {
        let provider = FakeProvider {
            records: vec![record(1, "unrelated")],
        };
        let witness = ProcessActivityWitness::capture_with_provider(
            &[ActivityGuard::ProcessName("cargo".to_owned())],
            &provider,
        )
        .unwrap();
        assert!(witness.revalidate_with_provider(&provider).is_ok());
    }

    #[test]
    fn active_process_fails_closed() {
        let provider = FakeProvider {
            records: vec![record(7, "cargo")],
        };
        assert_eq!(
            ProcessActivityWitness::capture_with_provider(
                &[ActivityGuard::ProcessName("cargo".to_owned())],
                &provider,
            ),
            Err(ProcessActivityError::Active)
        );
    }

    #[test]
    fn bundle_guards_do_not_downgrade_to_process_names() {
        let provider = FakeProvider {
            records: Vec::new(),
        };
        assert_eq!(
            ProcessActivityWitness::capture_with_provider(
                &[ActivityGuard::BundleIdentifier(
                    "com.example.App".to_owned()
                )],
                &provider,
            ),
            Err(ProcessActivityError::UnsupportedBundleIdentifier)
        );
    }

    #[test]
    fn pid_reuse_or_image_change_invalidates_retained_observation() {
        let first = FakeProvider {
            records: vec![record(12, "other")],
        };
        let witness = ProcessActivityWitness::capture_with_provider(
            &[ActivityGuard::ProcessName("cargo".to_owned())],
            &first,
        )
        .unwrap();
        let replacement = FakeProvider {
            records: vec![record(12, "replacement")],
        };
        assert_eq!(
            witness.revalidate_with_provider(&replacement),
            Err(ProcessActivityError::Changed)
        );
    }

    #[test]
    fn duplicate_and_malformed_records_fail_closed() {
        let duplicate = record(1, "one");
        let provider = FakeProvider {
            records: vec![duplicate.clone(), duplicate],
        };
        assert_eq!(
            ProcessActivityWitness::capture_with_provider(
                &[ActivityGuard::ProcessName("cargo".to_owned())],
                &provider,
            ),
            Err(ProcessActivityError::MalformedRecord)
        );
        let malformed = FakeProvider {
            records: vec![ProcessRecord {
                pid: 0,
                ..record(2, "bad")
            }],
        };
        assert_eq!(
            ProcessActivityWitness::capture_with_provider(
                &[ActivityGuard::ProcessName("cargo".to_owned())],
                &malformed,
            ),
            Err(ProcessActivityError::MalformedRecord)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_process_provider_is_bounded_and_returns_absolute_images() {
        match NativeProcessProvider.enumerate() {
            Ok(records) => {
                assert!(records.len() <= MAX_PROCESS_RECORDS);
                assert!(records.iter().all(|record| record.executable.is_absolute()));
            }
            // A restricted CI/test host is expected to fail closed rather
            // than turn incomplete process visibility into inactive evidence.
            Err(ProcessActivityError::EnumerationUnavailable) => {}
            Err(error) => panic!("unexpected native process result: {error:?}"),
        }
    }
}
