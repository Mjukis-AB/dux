//! Private process-instance identity and conservative liveness evidence.
//!
//! A process ID, heartbeat age, or cleanup-lock observation is never enough to
//! prove that an execution owner is gone. Reliable platforms bind the PID and
//! start token to a boot/namespace scope; an unproven scope can report `Alive`
//! for an exact live match but never `DefinitelyGone`.

#[cfg(any(target_os = "linux", target_os = "macos"))]
use sha2::{Digest, Sha256};

const ENCODING_VERSION: &str = "1";
const MAX_ENCODED_BYTES: usize = 128;
const SCOPE_BYTES: usize = 32;
const IDENTITY_BYTES: usize = 32;
const NONCE_BYTES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Platform {
    Linux,
    Macos,
    Windows,
}

impl Platform {
    fn tag(self) -> &'static str {
        match self {
            Self::Linux => "l",
            Self::Macos => "m",
            Self::Windows => "w",
        }
    }

    fn from_tag(value: &str) -> Option<Self> {
        match value {
            "l" => Some(Self::Linux),
            "m" => Some(Self::Macos),
            "w" => Some(Self::Windows),
            _ => None,
        }
    }
}

/// A versioned, bounded identifier for one operating-system process instance.
///
/// The random nonce makes separately created ownership claims unique. It is
/// intentionally not consulted by the liveness classifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ProcessInstanceId {
    encoded: String,
    platform: Platform,
    pid: u32,
    start_token: u64,
    scope: Option<[u8; SCOPE_BYTES]>,
    _nonce: [u8; NONCE_BYTES],
}

impl ProcessInstanceId {
    /// Decode the exact representation reserved for cleanup-journal owners.
    pub(crate) fn from_stored(value: &str) -> Result<Self, ProcessIdentityError> {
        if value.is_empty() || value.len() > MAX_ENCODED_BYTES || !value.is_ascii() {
            return Err(ProcessIdentityError::InvalidEncoding);
        }
        let mut fields = value.split(':');
        let version = fields.next();
        let platform = fields.next().and_then(Platform::from_tag);
        let pid_text = fields.next();
        let start_text = fields.next();
        let scope_text = fields.next();
        let nonce_text = fields.next();
        if version != Some(ENCODING_VERSION) || fields.next().is_some() {
            return Err(ProcessIdentityError::InvalidEncoding);
        }
        let platform = platform.ok_or(ProcessIdentityError::InvalidEncoding)?;
        let pid = parse_canonical_hex_u32(pid_text.ok_or(ProcessIdentityError::InvalidEncoding)?)?;
        let start_token =
            parse_canonical_hex_u64(start_text.ok_or(ProcessIdentityError::InvalidEncoding)?)?;
        if pid == 0 || start_token == 0 {
            return Err(ProcessIdentityError::InvalidEncoding);
        }
        let scope_text = scope_text.ok_or(ProcessIdentityError::InvalidEncoding)?;
        let scope = match (platform, scope_text) {
            (Platform::Macos | Platform::Windows, "-") => None,
            (Platform::Linux | Platform::Macos, value) => Some(parse_fixed_hex(value)?),
            (Platform::Windows, _) => return Err(ProcessIdentityError::InvalidEncoding),
        };
        let nonce = parse_fixed_hex(nonce_text.ok_or(ProcessIdentityError::InvalidEncoding)?)?;
        Ok(Self {
            encoded: value.to_owned(),
            platform,
            pid,
            start_token,
            scope,
            _nonce: nonce,
        })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.encoded
    }

    /// Stable discovery partition for one reliable boot/namespace scope.
    /// This narrows bounded recovery queries but is never liveness authority.
    pub(crate) fn recovery_scope_key(&self) -> Option<String> {
        self.scope
            .as_ref()
            .map(|scope| format!("{}:{}", self.platform.tag(), hex_lower(scope)))
    }

    pub(super) const fn pid(&self) -> u32 {
        self.pid
    }

    fn accepts_provenance(&self, provenance: &ExecutionProvenance) -> bool {
        self.platform == provenance.platform && self.scope == Some(provenance.boot_scope)
    }

    fn from_snapshot(
        snapshot: ProcessSnapshot,
        nonce: [u8; NONCE_BYTES],
    ) -> Result<Self, ProcessIdentityError> {
        let scope = snapshot
            .scope
            .as_ref()
            .map(|value| hex_lower(value))
            .unwrap_or_else(|| "-".to_owned());
        let encoded = format!(
            "{}:{}:{:x}:{:x}:{}:{}",
            ENCODING_VERSION,
            snapshot.platform.tag(),
            snapshot.pid,
            snapshot.start_token,
            scope,
            hex_lower(&nonce)
        );
        Self::from_stored(&encoded)
    }
}

/// Separate stable-host and boot/namespace provenance for one execution owner.
///
/// These digests are recovery classification evidence only. They carry no
/// path, plan, approval, or filesystem-effect authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExecutionProvenance {
    platform: Platform,
    stable_host: [u8; IDENTITY_BYTES],
    boot_scope: [u8; SCOPE_BYTES],
}

impl ExecutionProvenance {
    pub(crate) fn from_stored(
        owner: &ProcessInstanceId,
        stable_host: &[u8],
        boot_scope: &[u8],
    ) -> Result<Self, ProcessIdentityError> {
        let stable_host: [u8; IDENTITY_BYTES] = stable_host
            .try_into()
            .map_err(|_| ProcessIdentityError::InvalidEncoding)?;
        let boot_scope: [u8; SCOPE_BYTES] = boot_scope
            .try_into()
            .map_err(|_| ProcessIdentityError::InvalidEncoding)?;
        let provenance = Self {
            platform: owner.platform,
            stable_host,
            boot_scope,
        };
        if !owner.accepts_provenance(&provenance) {
            return Err(ProcessIdentityError::InvalidEncoding);
        }
        Ok(provenance)
    }

    pub(crate) fn stable_host(&self) -> &[u8; IDENTITY_BYTES] {
        &self.stable_host
    }

    pub(crate) fn boot_scope(&self) -> &[u8; SCOPE_BYTES] {
        &self.boot_scope
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessExecutionIdentity {
    pub(crate) owner: ProcessInstanceId,
    pub(crate) provenance: Option<ExecutionProvenance>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProvenanceRelationship {
    SameBoot,
    PriorBoot,
    ForeignHost,
    Unproven,
}

pub(crate) fn compare_execution_provenance(
    stored: Option<&ExecutionProvenance>,
    current: Option<&ExecutionProvenance>,
) -> ProvenanceRelationship {
    let (Some(stored), Some(current)) = (stored, current) else {
        return ProvenanceRelationship::Unproven;
    };
    if stored.platform != current.platform || stored.stable_host != current.stable_host {
        ProvenanceRelationship::ForeignHost
    } else if stored.boot_scope != current.boot_scope {
        ProvenanceRelationship::PriorBoot
    } else {
        ProvenanceRelationship::SameBoot
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcessLiveness {
    Alive,
    DefinitelyGone,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcessIdentityError {
    InvalidEncoding,
    ObservationUnavailable,
    RandomUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcessSnapshot {
    platform: Platform,
    pid: u32,
    start_token: u64,
    scope: Option<[u8; SCOPE_BYTES]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessObservation {
    Running(ProcessSnapshot),
    Missing {
        platform: Platform,
        scope: Option<[u8; SCOPE_BYTES]>,
    },
    Unknown,
}

/// Create an identity for the calling process without granting journal or
/// cleanup authority.
pub(crate) fn current_process_instance() -> Result<ProcessInstanceId, ProcessIdentityError> {
    let snapshot = platform::current_process()?;
    let mut nonce = [0_u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).map_err(|_| ProcessIdentityError::RandomUnavailable)?;
    ProcessInstanceId::from_snapshot(snapshot, nonce)
}

/// Observe one current process identity together with independent host/boot
/// provenance where the platform can supply the complete pair.
pub(crate) fn current_process_execution_identity()
-> Result<ProcessExecutionIdentity, ProcessIdentityError> {
    let owner = current_process_instance()?;
    let provenance = platform::current_execution_provenance()
        .filter(|provenance| owner.accepts_provenance(provenance));
    Ok(ProcessExecutionIdentity { owner, provenance })
}

/// Observe only the calling process's stable-host and boot context.
///
/// This path intentionally creates no process owner or random nonce and never
/// probes another process. An unavailable or internally inconsistent
/// observation is represented as absent provenance for read-only diagnostics.
pub(crate) fn current_process_execution_provenance() -> Option<ExecutionProvenance> {
    let snapshot = platform::current_process().ok()?;
    let provenance = platform::current_execution_provenance()?;
    (snapshot.platform == provenance.platform && snapshot.scope == Some(provenance.boot_scope))
        .then_some(provenance)
}

/// Conservatively classify the exact process instance represented by `owner`.
///
/// An observation from another boot/namespace/host scope is ambiguous because
/// the database may be shared. Only a same-scope absence or start-token change
/// proves death. Platforms without a reliable scope never report death.
pub(crate) fn probe_process_instance(owner: &ProcessInstanceId) -> ProcessLiveness {
    classify(owner, platform::observe_process(owner.pid))
}

fn classify(owner: &ProcessInstanceId, observation: ProcessObservation) -> ProcessLiveness {
    match observation {
        ProcessObservation::Running(current)
            if current.platform == owner.platform
                && current.scope == owner.scope
                && current.start_token == owner.start_token =>
        {
            ProcessLiveness::Alive
        }
        ProcessObservation::Running(current)
            if current.platform == owner.platform
                && owner.scope.is_some()
                && current.scope == owner.scope =>
        {
            ProcessLiveness::DefinitelyGone
        }
        ProcessObservation::Missing { platform, scope }
            if platform == owner.platform && owner.scope.is_some() && scope == owner.scope =>
        {
            ProcessLiveness::DefinitelyGone
        }
        ProcessObservation::Running(_)
        | ProcessObservation::Missing { .. }
        | ProcessObservation::Unknown => ProcessLiveness::Unknown,
    }
}

fn parse_canonical_hex_u32(value: &str) -> Result<u32, ProcessIdentityError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(is_lower_hex)
    {
        return Err(ProcessIdentityError::InvalidEncoding);
    }
    u32::from_str_radix(value, 16).map_err(|_| ProcessIdentityError::InvalidEncoding)
}

fn parse_canonical_hex_u64(value: &str) -> Result<u64, ProcessIdentityError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(is_lower_hex)
    {
        return Err(ProcessIdentityError::InvalidEncoding);
    }
    u64::from_str_radix(value, 16).map_err(|_| ProcessIdentityError::InvalidEncoding)
}

fn parse_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], ProcessIdentityError> {
    if value.len() != N * 2 || !value.bytes().all(is_lower_hex) {
        return Err(ProcessIdentityError::InvalidEncoding);
    }
    let bytes = value.as_bytes();
    let mut decoded = [0_u8; N];
    for (index, output) in decoded.iter_mut().enumerate() {
        let high = hex_value(bytes[index * 2]);
        let low = hex_value(bytes[index * 2 + 1]);
        *output = (high << 4) | low;
    }
    Ok(decoded)
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("hex input was validated"),
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn scope_digest(label: &[u8], parts: &[&[u8]]) -> [u8; SCOPE_BYTES] {
    let mut hasher = Sha256::new();
    hasher.update(label);
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn parse_uuid(value: &[u8]) -> Option<[u8; 16]> {
    if value.len() != 36
        || value[8] != b'-'
        || value[13] != b'-'
        || value[18] != b'-'
        || value[23] != b'-'
    {
        return None;
    }
    let mut compact = [0_u8; 32];
    let mut length = 0;
    for byte in value {
        if *byte == b'-' {
            continue;
        }
        if !byte.is_ascii_hexdigit() || length == compact.len() {
            return None;
        }
        compact[length] = byte.to_ascii_lowercase();
        length += 1;
    }
    (length == compact.len()).then(|| {
        let text = std::str::from_utf8(&compact).expect("ASCII UUID");
        parse_fixed_hex(text).expect("validated UUID hex")
    })
}

#[cfg(target_os = "linux")]
mod platform {
    use std::fs::{self, File};
    use std::io::{ErrorKind, Read};
    use std::os::unix::fs::MetadataExt;

    use super::{
        ExecutionProvenance, Platform, ProcessIdentityError, ProcessObservation, ProcessSnapshot,
        parse_fixed_hex, parse_uuid, scope_digest,
    };

    const MAX_BOOT_ID_BYTES: u64 = 64;
    const MAX_MACHINE_ID_BYTES: u64 = 64;
    const MAX_PROC_STAT_BYTES: u64 = 8_192;

    pub(super) fn current_process() -> Result<ProcessSnapshot, ProcessIdentityError> {
        let pid = std::process::id();
        match observe_process(pid) {
            ProcessObservation::Running(snapshot) => Ok(snapshot),
            ProcessObservation::Missing { .. } | ProcessObservation::Unknown => {
                Err(ProcessIdentityError::ObservationUnavailable)
            }
        }
    }

    pub(super) fn observe_process(pid: u32) -> ProcessObservation {
        let Ok(before) = current_scope() else {
            return ProcessObservation::Unknown;
        };
        let observed = read_start_token(pid);
        let Ok(after) = current_scope() else {
            return ProcessObservation::Unknown;
        };
        if before != after {
            return ProcessObservation::Unknown;
        }
        match observed {
            StartObservation::Running(start_token) => {
                ProcessObservation::Running(ProcessSnapshot {
                    platform: Platform::Linux,
                    pid,
                    start_token,
                    scope: Some(before),
                })
            }
            StartObservation::Missing => ProcessObservation::Missing {
                platform: Platform::Linux,
                scope: Some(before),
            },
            StartObservation::Unknown => ProcessObservation::Unknown,
        }
    }

    pub(super) fn current_execution_provenance() -> Option<ExecutionProvenance> {
        let boot_scope = current_scope().ok()?;
        let file = File::open("/etc/machine-id").ok()?;
        let mut bytes = Vec::new();
        file.take(MAX_MACHINE_ID_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > MAX_MACHINE_ID_BYTES as usize {
            return None;
        }
        while matches!(bytes.last(), Some(b'\n' | b'\r')) {
            bytes.pop();
        }
        let text = std::str::from_utf8(&bytes).ok()?;
        let machine_id: [u8; 16] = parse_fixed_hex(text).ok()?;
        Some(ExecutionProvenance {
            platform: Platform::Linux,
            stable_host: scope_digest(b"dux-linux-stable-host-v1", &[&machine_id]),
            boot_scope,
        })
    }

    fn current_scope() -> Result<[u8; 32], ()> {
        let current_pid = std::process::id();
        if read_proc_stat_file("/proc/self/stat", current_pid).is_none() {
            return Err(());
        }
        let file = File::open("/proc/sys/kernel/random/boot_id").map_err(|_| ())?;
        let mut bytes = Vec::new();
        file.take(MAX_BOOT_ID_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ())?;
        if bytes.len() > MAX_BOOT_ID_BYTES as usize {
            return Err(());
        }
        while matches!(bytes.last(), Some(b'\n' | b'\r')) {
            bytes.pop();
        }
        let boot_id = parse_uuid(&bytes).ok_or(())?;
        let namespace = namespace_identity("/proc/self/ns/pid")?;
        if read_proc_stat_file("/proc/self/stat", current_pid).is_none() {
            return Err(());
        }
        Ok(scope_digest(
            b"dux-linux-process-scope-v1",
            &[
                &boot_id,
                &namespace.0.to_le_bytes(),
                &namespace.1.to_le_bytes(),
            ],
        ))
    }

    enum StartObservation {
        Running(u64),
        Missing,
        Unknown,
    }

    fn read_start_token(pid: u32) -> StartObservation {
        let path = format!("/proc/{pid}/stat");
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return corroborate_missing_process(pid);
            }
            Err(_) => return StartObservation::Unknown,
        };
        let mut bytes = Vec::new();
        if file
            .take(MAX_PROC_STAT_BYTES + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() > MAX_PROC_STAT_BYTES as usize
        {
            return StartObservation::Unknown;
        }
        let Some(start_token) = parse_proc_stat_start(pid, &bytes) else {
            return StartObservation::Unknown;
        };
        let target_namespace = namespace_identity(&format!("/proc/{pid}/ns/pid"));
        let current_namespace = namespace_identity("/proc/self/ns/pid");
        if target_namespace.is_err() || target_namespace != current_namespace {
            return StartObservation::Unknown;
        }
        StartObservation::Running(start_token)
    }

    fn read_proc_stat_file(path: &str, pid: u32) -> Option<u64> {
        let file = File::open(path).ok()?;
        let mut bytes = Vec::new();
        file.take(MAX_PROC_STAT_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > MAX_PROC_STAT_BYTES as usize {
            return None;
        }
        parse_proc_stat_start(pid, &bytes)
    }

    fn namespace_identity(path: &str) -> Result<(u64, u64), ()> {
        let metadata = fs::metadata(path).map_err(|_| ())?;
        Ok((metadata.dev(), metadata.ino()))
    }

    fn corroborate_missing_process(pid: u32) -> StartObservation {
        let Ok(pid) = i32::try_from(pid) else {
            return StartObservation::Unknown;
        };
        // SAFETY: signal 0 performs an existence/permission probe only and
        // sends no signal to the numeric PID.
        let result = unsafe { nix::libc::kill(pid, 0) };
        let error = (result != 0)
            .then(|| std::io::Error::last_os_error().raw_os_error())
            .flatten();
        classify_kill_probe(result, error)
    }

    fn classify_kill_probe(result: i32, error: Option<i32>) -> StartObservation {
        if result != 0 && error == Some(nix::libc::ESRCH) {
            StartObservation::Missing
        } else {
            // A successful or permission-denied probe proves that some process
            // owns the PID, but without its start token the exact owner remains
            // unknown. Every other error is ambiguous too.
            StartObservation::Unknown
        }
    }

    fn parse_proc_stat_start(pid: u32, bytes: &[u8]) -> Option<u64> {
        let prefix = format!("{pid} (");
        if !bytes.starts_with(prefix.as_bytes()) {
            return None;
        }
        let close = bytes.windows(2).rposition(|pair| pair == b") ")?;
        let fields = bytes.get(close + 2..)?;
        let start = fields
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty())
            .nth(19)?;
        let value = std::str::from_utf8(start).ok()?.parse::<u64>().ok()?;
        (value > 0).then_some(value)
    }

    #[cfg(test)]
    mod tests {
        use super::{StartObservation, classify_kill_probe, parse_proc_stat_start};

        #[test]
        fn proc_stat_parser_uses_the_last_comm_delimiter() {
            let stat =
                b"42 (name with ) delimiter) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 999 20";
            assert_eq!(parse_proc_stat_start(42, stat), Some(999));
            assert_eq!(parse_proc_stat_start(41, stat), None);
            assert_eq!(parse_proc_stat_start(42, b"42 (truncated) S 1"), None);
        }

        #[test]
        fn hidden_proc_entry_is_missing_only_after_esrch() {
            assert!(matches!(
                classify_kill_probe(-1, Some(nix::libc::ESRCH)),
                StartObservation::Missing
            ));
            for (result, error) in [
                (0, None),
                (-1, Some(nix::libc::EPERM)),
                (-1, Some(nix::libc::EIO)),
            ] {
                assert!(matches!(
                    classify_kill_probe(result, error),
                    StartObservation::Unknown
                ));
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{c_char, c_int, c_void};
    use std::mem::{MaybeUninit, size_of};

    use nix::libc;

    use super::{
        ExecutionProvenance, Platform, ProcessIdentityError, ProcessObservation, ProcessSnapshot,
        parse_uuid, scope_digest,
    };

    const PROC_PIDTBSDINFO: c_int = 3;

    #[repr(C)]
    struct ProcBsdInfo {
        pbi_flags: u32,
        pbi_status: u32,
        pbi_xstatus: u32,
        pbi_pid: u32,
        pbi_ppid: u32,
        pbi_uid: u32,
        pbi_gid: u32,
        pbi_ruid: u32,
        pbi_rgid: u32,
        pbi_svuid: u32,
        pbi_svgid: u32,
        rfu_1: u32,
        pbi_comm: [c_char; 16],
        pbi_name: [c_char; 32],
        pbi_nfiles: u32,
        pbi_pgid: u32,
        pbi_pjobc: u32,
        e_tdev: u32,
        e_tpgid: u32,
        pbi_nice: i32,
        pbi_start_tvsec: u64,
        pbi_start_tvusec: u64,
    }

    unsafe extern "C" {
        fn proc_pidinfo(
            pid: c_int,
            flavor: c_int,
            arg: u64,
            buffer: *mut c_void,
            buffersize: c_int,
        ) -> c_int;
    }

    pub(super) fn current_process() -> Result<ProcessSnapshot, ProcessIdentityError> {
        let pid = std::process::id();
        match observe_process(pid) {
            ProcessObservation::Running(snapshot) => Ok(snapshot),
            ProcessObservation::Missing { .. } | ProcessObservation::Unknown => {
                Err(ProcessIdentityError::ObservationUnavailable)
            }
        }
    }

    pub(super) fn observe_process(pid: u32) -> ProcessObservation {
        let before = current_scope().ok();
        let observed = read_start_token(pid);
        let after = current_scope().ok();
        // Some hardened runtimes deny the boot-session sysctl. Retain the
        // exact PID/start observation but omit scope; classification then
        // remains useful for a live exact match and can never prove death.
        let scope = match (before, after) {
            (Some(before), Some(after)) if before == after => Some(before),
            _ => None,
        };
        match observed {
            StartObservation::Running(start_token) => {
                ProcessObservation::Running(ProcessSnapshot {
                    platform: Platform::Macos,
                    pid,
                    start_token,
                    scope,
                })
            }
            StartObservation::Missing => ProcessObservation::Missing {
                platform: Platform::Macos,
                scope,
            },
            StartObservation::Unknown => ProcessObservation::Unknown,
        }
    }

    pub(super) fn current_execution_provenance() -> Option<ExecutionProvenance> {
        let stable_host_before = read_stable_host_uuid()?;
        let boot_uuid = read_uuid_sysctl(b"kern.bootsessionuuid\0").ok()?;
        let stable_host_after = read_stable_host_uuid()?;
        if stable_host_before != stable_host_after {
            return None;
        }
        Some(ExecutionProvenance {
            platform: Platform::Macos,
            stable_host: scope_digest(b"dux-macos-stable-host-v1", &[&stable_host_before]),
            boot_scope: scope_digest(b"dux-macos-process-scope-v1", &[&boot_uuid]),
        })
    }

    fn current_scope() -> Result<[u8; 32], ()> {
        let boot_id = read_uuid_sysctl(b"kern.bootsessionuuid\0")?;
        Ok(scope_digest(b"dux-macos-process-scope-v1", &[&boot_id]))
    }

    fn read_uuid_sysctl(name: &[u8]) -> Result<[u8; 16], ()> {
        if name.last() != Some(&0) {
            return Err(());
        }
        let mut buffer = [0_u8; 64];
        let mut length = buffer.len();
        // SAFETY: name and output pointers are valid for the supplied lengths;
        // this read-only sysctl has no new-value buffer.
        let result = unsafe {
            libc::sysctlbyname(
                name.as_ptr().cast(),
                buffer.as_mut_ptr().cast(),
                &raw mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if result != 0 || !(37..=buffer.len()).contains(&length) || buffer[length - 1] != 0 {
            return Err(());
        }
        parse_uuid(&buffer[..length - 1]).ok_or(())
    }

    fn read_stable_host_uuid() -> Option<[u8; 16]> {
        let mut uuid = [0_u8; 16];
        let timeout = libc::timespec {
            tv_sec: 1,
            tv_nsec: 0,
        };
        // SAFETY: `uuid` is a writable 16-byte UUID buffer and `timeout`
        // points to a valid bounded immutable timespec for the duration of the
        // read-only host observation.
        if unsafe { libc::gethostuuid(uuid.as_mut_ptr(), &raw const timeout) } != 0
            || uuid.iter().all(|byte| *byte == 0)
        {
            return None;
        }
        Some(uuid)
    }

    enum StartObservation {
        Running(u64),
        Missing,
        Unknown,
    }

    fn read_start_token(pid: u32) -> StartObservation {
        let Ok(pid) = c_int::try_from(pid) else {
            return StartObservation::Unknown;
        };
        let mut info = MaybeUninit::<ProcBsdInfo>::uninit();
        let expected = size_of::<ProcBsdInfo>();
        let Ok(buffer_size) = c_int::try_from(expected) else {
            return StartObservation::Unknown;
        };
        // SAFETY: `info` points to an aligned writable buffer of exactly the
        // declared proc_bsdinfo ABI size. It is read only after a full result.
        let result = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                buffer_size,
            )
        };
        if result == buffer_size {
            // SAFETY: proc_pidinfo reported that the full structure was filled.
            let info = unsafe { info.assume_init() };
            if info.pbi_pid != pid as u32 || info.pbi_start_tvusec >= 1_000_000 {
                return StartObservation::Unknown;
            }
            return info
                .pbi_start_tvsec
                .checked_mul(1_000_000)
                .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
                .filter(|value| *value > 0)
                .map(StartObservation::Running)
                .unwrap_or(StartObservation::Unknown);
        }
        if result == 0 {
            let code = std::io::Error::last_os_error().raw_os_error();
            if matches!(code, Some(libc::ESRCH | libc::ENOENT)) {
                return StartObservation::Missing;
            }
        }
        StartObservation::Unknown
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Foundation::{
        CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    use super::{
        ExecutionProvenance, Platform, ProcessIdentityError, ProcessObservation, ProcessSnapshot,
    };

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: OpenProcess returned this owned non-null handle exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub(super) fn current_process() -> Result<ProcessSnapshot, ProcessIdentityError> {
        // SAFETY: GetCurrentProcessId has no preconditions.
        let pid = unsafe { GetCurrentProcessId() };
        match observe_process(pid) {
            ProcessObservation::Running(snapshot) => Ok(snapshot),
            ProcessObservation::Missing { .. } | ProcessObservation::Unknown => {
                Err(ProcessIdentityError::ObservationUnavailable)
            }
        }
    }

    pub(super) fn observe_process(pid: u32) -> ProcessObservation {
        // SAFETY: OpenProcess receives a numeric PID and fixed query/synchronize rights.
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            // A local absence is not a death proof without a reliable host/boot
            // scope; access failures are equally ambiguous.
            return ProcessObservation::Unknown;
        }
        let handle = OwnedHandle(handle);
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        // SAFETY: all FILETIME pointers are valid and the retained handle has
        // process-query rights.
        if unsafe {
            GetProcessTimes(
                handle.0,
                &raw mut creation,
                &raw mut exit,
                &raw mut kernel,
                &raw mut user,
            )
        } == 0
        {
            return ProcessObservation::Unknown;
        }
        // SAFETY: the retained handle carries synchronize rights and timeout 0
        // is a nonblocking state observation.
        match unsafe { WaitForSingleObject(handle.0, 0) } {
            WAIT_TIMEOUT => {
                let start_token =
                    (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
                if start_token == 0 {
                    ProcessObservation::Unknown
                } else {
                    ProcessObservation::Running(ProcessSnapshot {
                        platform: Platform::Windows,
                        pid,
                        start_token,
                        scope: None,
                    })
                }
            }
            WAIT_OBJECT_0 => ProcessObservation::Missing {
                platform: Platform::Windows,
                scope: None,
            },
            _ => ProcessObservation::Unknown,
        }
    }

    pub(super) fn current_execution_provenance() -> Option<ExecutionProvenance> {
        None
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
    use super::{ExecutionProvenance, ProcessIdentityError, ProcessObservation, ProcessSnapshot};

    pub(super) fn current_process() -> Result<ProcessSnapshot, ProcessIdentityError> {
        Err(ProcessIdentityError::ObservationUnavailable)
    }

    pub(super) fn observe_process(_pid: u32) -> ProcessObservation {
        ProcessObservation::Unknown
    }

    pub(super) fn current_execution_provenance() -> Option<ExecutionProvenance> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(platform: Platform, scope: Option<[u8; 32]>) -> ProcessInstanceId {
        ProcessInstanceId::from_snapshot(
            ProcessSnapshot {
                platform,
                pid: 0x2a,
                start_token: 0x1234,
                scope,
            },
            [0x5a; NONCE_BYTES],
        )
        .unwrap()
    }

    #[test]
    fn identity_encoding_is_canonical_bounded_and_strict() {
        let scoped = fixture(Platform::Linux, Some([0xab; 32]));
        assert!(scoped.as_str().len() <= MAX_ENCODED_BYTES);
        assert_eq!(
            ProcessInstanceId::from_stored(scoped.as_str()).unwrap(),
            scoped
        );
        let windows = fixture(Platform::Windows, None);
        assert_eq!(
            ProcessInstanceId::from_stored(windows.as_str()).unwrap(),
            windows
        );
        let unscoped_macos = fixture(Platform::Macos, None);
        assert_eq!(
            ProcessInstanceId::from_stored(unscoped_macos.as_str()).unwrap(),
            unscoped_macos
        );
        let maximum = ProcessInstanceId::from_snapshot(
            ProcessSnapshot {
                platform: Platform::Linux,
                pid: u32::MAX,
                start_token: u64::MAX,
                scope: Some([u8::MAX; SCOPE_BYTES]),
            },
            [u8::MAX; NONCE_BYTES],
        )
        .unwrap();
        assert_eq!(maximum.as_str().len(), 127);
        assert_eq!(
            ProcessInstanceId::from_stored(maximum.as_str()).unwrap(),
            maximum
        );
        let oversized = format!("{}xx", maximum.as_str());
        assert_eq!(oversized.len(), 129);
        assert_eq!(
            ProcessInstanceId::from_stored(&oversized),
            Err(ProcessIdentityError::InvalidEncoding)
        );
        for malformed in [
            "",
            "2:l:2a:1234:abab:00",
            "1:x:2a:1234:-:00000000000000000000000000000000",
            "1:l:02a:1234:abababababababababababababababababababababababababababababababab:00000000000000000000000000000000",
            "1:l:2a:0:abababababababababababababababababababababababababababababababab:00000000000000000000000000000000",
            "1:w:2a:1234:abababababababababababababababababababababababababababababababab:00000000000000000000000000000000",
            "1:l:2a:1234:ABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABAB:00000000000000000000000000000000",
        ] {
            assert_eq!(
                ProcessInstanceId::from_stored(malformed),
                Err(ProcessIdentityError::InvalidEncoding),
                "accepted malformed identity {malformed:?}"
            );
        }
    }

    #[test]
    fn only_same_reliable_scope_can_prove_death() {
        let scope = [1_u8; 32];
        let owner = fixture(Platform::Linux, Some(scope));
        let running = |start_token, scope| {
            ProcessObservation::Running(ProcessSnapshot {
                platform: Platform::Linux,
                pid: owner.pid,
                start_token,
                scope,
            })
        };
        assert_eq!(
            classify(&owner, running(owner.start_token, Some(scope))),
            ProcessLiveness::Alive
        );
        assert_eq!(
            classify(&owner, running(owner.start_token + 1, Some(scope))),
            ProcessLiveness::DefinitelyGone
        );
        assert_eq!(
            classify(
                &owner,
                ProcessObservation::Missing {
                    platform: Platform::Linux,
                    scope: Some(scope),
                }
            ),
            ProcessLiveness::DefinitelyGone
        );
        assert_eq!(
            classify(&owner, running(owner.start_token, Some([2; 32]))),
            ProcessLiveness::Unknown
        );
        assert_eq!(
            classify(&owner, ProcessObservation::Unknown),
            ProcessLiveness::Unknown
        );
    }

    #[test]
    fn unscoped_observations_never_prove_death() {
        for platform in [Platform::Macos, Platform::Windows] {
            let owner = fixture(platform, None);
            let running = |start_token| {
                ProcessObservation::Running(ProcessSnapshot {
                    platform,
                    pid: owner.pid,
                    start_token,
                    scope: None,
                })
            };
            assert_eq!(
                classify(&owner, running(owner.start_token)),
                ProcessLiveness::Alive
            );
            assert_eq!(
                classify(&owner, running(owner.start_token + 1)),
                ProcessLiveness::Unknown
            );
            assert_eq!(
                classify(
                    &owner,
                    ProcessObservation::Missing {
                        platform,
                        scope: None,
                    },
                ),
                ProcessLiveness::Unknown
            );
        }
    }

    #[test]
    fn execution_provenance_is_strictly_bound_to_owner_boot_scope() {
        let owner = fixture(Platform::Linux, Some([1; 32]));
        let provenance = ExecutionProvenance::from_stored(&owner, &[2; 32], &[1; 32]).unwrap();
        assert_eq!(provenance.stable_host(), &[2; 32]);
        assert_eq!(provenance.boot_scope(), &[1; 32]);
        for (host, boot) in [
            (&[2_u8; 31][..], &[1_u8; 32][..]),
            (&[2_u8; 32][..], &[1_u8; 31][..]),
            (&[2_u8; 32][..], &[3_u8; 32][..]),
        ] {
            assert_eq!(
                ExecutionProvenance::from_stored(&owner, host, boot),
                Err(ProcessIdentityError::InvalidEncoding)
            );
        }
    }

    #[test]
    fn provenance_relationship_separates_boot_host_and_unproven_states() {
        let owner = fixture(Platform::Linux, Some([1; 32]));
        let stored = ExecutionProvenance::from_stored(&owner, &[2; 32], &[1; 32]).unwrap();
        let same = stored.clone();
        let prior_owner = fixture(Platform::Linux, Some([3; 32]));
        let prior = ExecutionProvenance::from_stored(&prior_owner, &[2; 32], &[3; 32]).unwrap();
        let foreign = ExecutionProvenance::from_stored(&owner, &[4; 32], &[1; 32]).unwrap();
        assert_eq!(
            compare_execution_provenance(Some(&stored), Some(&same)),
            ProvenanceRelationship::SameBoot
        );
        assert_eq!(
            compare_execution_provenance(Some(&stored), Some(&prior)),
            ProvenanceRelationship::PriorBoot
        );
        assert_eq!(
            compare_execution_provenance(Some(&stored), Some(&foreign)),
            ProvenanceRelationship::ForeignHost
        );
        assert_eq!(
            compare_execution_provenance(Some(&stored), None),
            ProvenanceRelationship::Unproven
        );
    }

    #[test]
    fn current_process_is_observed_alive() {
        let owner = current_process_instance().unwrap();
        assert_eq!(probe_process_instance(&owner), ProcessLiveness::Alive);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn current_execution_identity_is_internally_consistent() {
        let identity = current_process_execution_identity().unwrap();
        assert_eq!(
            probe_process_instance(&identity.owner),
            ProcessLiveness::Alive
        );
        if let Some(provenance) = identity.provenance {
            assert!(identity.owner.accepts_provenance(&provenance));
            assert!(provenance.stable_host().iter().any(|byte| *byte != 0));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires unsandboxed macOS host and boot identity access"]
    fn macos_execution_identity_has_complete_provenance() {
        let identity = current_process_execution_identity().unwrap();
        let provenance = identity
            .provenance
            .expect("macOS host or boot identity was unavailable");
        assert!(identity.owner.accepts_provenance(&provenance));
        assert!(provenance.stable_host().iter().any(|byte| *byte != 0));
    }

    #[test]
    fn uuid_parser_is_exact_and_case_insensitive_at_the_os_boundary() {
        assert_eq!(
            parse_uuid(b"550E8400-E29B-41D4-A716-446655440000"),
            Some([
                0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
                0x00, 0x00,
            ])
        );
        assert_eq!(parse_uuid(b"550e8400e29b41d4a716446655440000"), None);
    }
}
