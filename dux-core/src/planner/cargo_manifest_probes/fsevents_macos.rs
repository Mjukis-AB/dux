//! Bounded, file-granular FSEvents replay for absent ancestor `Cargo.toml`
//! probes.
//!
//! Kqueue reports that a directory changed but not which name changed. This
//! companion fence anchors an ephemeral replay before the observation, watches
//! only the farthest retained ancestor, and compares event paths against the
//! bounded candidate set. FSEvents is evidence for a revalidation boundary,
//! not an audit log; any discontinuity is unavailable and the caller still
//! captures the filesystem state again.

use std::ffi::{CStr, c_char, c_void};
use std::path::Path;
use std::ptr;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
};

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use nix::libc;

use super::{AncestorManifestProbe, CargoManifestProbeError};

const OUTCOME_CLEAN: u8 = 0;
const OUTCOME_CHANGED: u8 = 1;
const OUTCOME_UNAVAILABLE: u8 = 2;
const MAX_CALLBACK_EVENTS: usize = 65_536;

// The CoreServices stream service is process-global. Serializing the short
// replay windows avoids exhausting the daemon or interleaving history cursors
// when Cargo tests (or multiple planner requests) run concurrently.
static FSEVENTS_REPLAY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

const CREATE_FLAG_NO_DEFER: u32 = 0x0000_0002;
const CREATE_FLAG_WATCH_ROOT: u32 = 0x0000_0004;
const CREATE_FLAG_FILE_EVENTS: u32 = 0x0000_0010;

const EVENT_FLAG_MUST_SCAN_SUBDIRS: u32 = 0x0000_0001;
const EVENT_FLAG_USER_DROPPED: u32 = 0x0000_0002;
const EVENT_FLAG_KERNEL_DROPPED: u32 = 0x0000_0004;
const EVENT_FLAG_IDS_WRAPPED: u32 = 0x0000_0008;
const EVENT_FLAG_HISTORY_DONE: u32 = 0x0000_0010;
const EVENT_FLAG_ROOT_CHANGED: u32 = 0x0000_0020;
const EVENT_FLAG_MOUNT: u32 = 0x0000_0040;
const EVENT_FLAG_UNMOUNT: u32 = 0x0000_0080;
const EVENT_FLAG_ITEM_CREATED: u32 = 0x0000_0100;
const EVENT_FLAG_ITEM_REMOVED: u32 = 0x0000_0200;
const EVENT_FLAG_ITEM_INODE_META_MOD: u32 = 0x0000_0400;
const EVENT_FLAG_ITEM_RENAMED: u32 = 0x0000_0800;
const EVENT_FLAG_ITEM_MODIFIED: u32 = 0x0000_1000;
const EVENT_FLAG_ITEM_FINDER_INFO_MOD: u32 = 0x0000_2000;
const EVENT_FLAG_ITEM_CHANGE_OWNER: u32 = 0x0000_4000;
const EVENT_FLAG_ITEM_XATTR_MOD: u32 = 0x0000_8000;
const EVENT_FLAG_ITEM_IS_FILE: u32 = 0x0001_0000;
const EVENT_FLAG_ITEM_IS_DIR: u32 = 0x0002_0000;
const EVENT_FLAG_ITEM_IS_SYMLINK: u32 = 0x0004_0000;
const EVENT_FLAG_OWN_EVENT: u32 = 0x0008_0000;
const EVENT_FLAG_ITEM_IS_HARDLINK: u32 = 0x0010_0000;
const EVENT_FLAG_ITEM_IS_LAST_HARDLINK: u32 = 0x0020_0000;
const EVENT_FLAG_ITEM_CLONED: u32 = 0x0040_0000;
const KNOWN_EVENT_FLAGS: u32 = 0x007f_ffff;

const DROPPED_FLAGS: u32 =
    EVENT_FLAG_MUST_SCAN_SUBDIRS | EVENT_FLAG_USER_DROPPED | EVENT_FLAG_KERNEL_DROPPED;
const VOLUME_FLAGS: u32 = EVENT_FLAG_ROOT_CHANGED | EVENT_FLAG_MOUNT | EVENT_FLAG_UNMOUNT;

type FSEventStreamRef = *mut c_void;
type ConstFSEventStreamRef = *const c_void;
type DispatchQueueRef = *mut c_void;
type StreamCallback =
    extern "C" fn(ConstFSEventStreamRef, *mut c_void, usize, *mut c_void, *const u32, *const u64);

#[repr(C)]
struct FSEventStreamContext {
    version: isize,
    info: *mut c_void,
    retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<extern "C" fn(*const c_void)>,
    copy_description: Option<extern "C" fn(*const c_void) -> *const c_void>,
}

#[repr(C)]
struct CFUUIDBytes {
    byte0: u8,
    byte1: u8,
    byte2: u8,
    byte3: u8,
    byte4: u8,
    byte5: u8,
    byte6: u8,
    byte7: u8,
    byte8: u8,
    byte9: u8,
    byte10: u8,
    byte11: u8,
    byte12: u8,
    byte13: u8,
    byte14: u8,
    byte15: u8,
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn FSEventsGetCurrentEventId() -> u64;
    fn FSEventsCopyUUIDForDevice(dev: libc::dev_t) -> *mut c_void;
    fn FSEventStreamCreate(
        allocator: *const c_void,
        callback: StreamCallback,
        context: *mut FSEventStreamContext,
        paths_to_watch: CFArrayRef,
        since_when: u64,
        latency: f64,
        flags: u32,
    ) -> FSEventStreamRef;
    fn FSEventStreamSetDispatchQueue(stream: FSEventStreamRef, queue: DispatchQueueRef);
    fn FSEventStreamStart(stream: FSEventStreamRef) -> u8;
    fn FSEventStreamFlushSync(stream: FSEventStreamRef);
    fn FSEventStreamGetLatestEventId(stream: ConstFSEventStreamRef) -> u64;
    fn FSEventStreamStop(stream: FSEventStreamRef);
    fn FSEventStreamInvalidate(stream: FSEventStreamRef);
    fn FSEventStreamRelease(stream: FSEventStreamRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFUUIDGetUUIDBytes(uuid: *const c_void) -> CFUUIDBytes;
}

#[link(name = "System")]
unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attributes: *const c_void) -> DispatchQueueRef;
    fn dispatch_release(object: DispatchQueueRef);
}

pub(super) struct ExactManifestEventCursor {
    event_id: u64,
    volume: u64,
    volume_uuid: [u8; 16],
}

impl ExactManifestEventCursor {
    pub(super) fn capture(volume: u64) -> Result<Self, CargoManifestProbeError> {
        let event_id = unsafe { FSEventsGetCurrentEventId() };
        let volume_uuid = copy_volume_uuid(volume)?;
        if event_id == u64::MAX {
            return Err(CargoManifestProbeError::Unavailable);
        }
        Ok(Self {
            event_id,
            volume,
            volume_uuid,
        })
    }
}

pub(super) struct ExactManifestEventFence {
    cursor: Mutex<ExactManifestEventCursor>,
    candidates: Vec<CandidatePath>,
    watch_root: Vec<u8>,
    terminal: AtomicU8,
}

impl ExactManifestEventFence {
    pub(super) fn disabled() -> Self {
        Self {
            cursor: Mutex::new(ExactManifestEventCursor {
                event_id: 0,
                volume: 0,
                volume_uuid: [0; 16],
            }),
            candidates: Vec::new(),
            watch_root: Vec::new(),
            terminal: AtomicU8::new(OUTCOME_CLEAN),
        }
    }

    pub(super) fn new(
        probes: &[AncestorManifestProbe],
        cursor: ExactManifestEventCursor,
    ) -> Result<Self, CargoManifestProbeError> {
        let mut candidates = Vec::new();
        for probe in probes.iter().filter(|probe| probe.manifest.is_none()) {
            if probe.directory.identity().volume() != cursor.volume {
                return Err(CargoManifestProbeError::Changed);
            }
            let directory = probe.directory.canonical_path();
            let parent = directory.as_os_str().as_encoded_bytes().to_vec();
            let path = probe.path.as_os_str().as_encoded_bytes().to_vec();
            let case_sensitive = case_sensitive_directory(directory)?;
            candidates.push(CandidatePath {
                full: path,
                parent,
                case_sensitive,
            });
        }
        if candidates.is_empty() {
            return Ok(Self::disabled());
        }
        let watch_root = candidates
            .last()
            .map(|candidate| candidate.parent.clone())
            .ok_or(CargoManifestProbeError::Unsupported)?;
        Ok(Self {
            cursor: Mutex::new(cursor),
            candidates,
            watch_root,
            terminal: AtomicU8::new(OUTCOME_CLEAN),
        })
    }

    pub(super) fn poll(&self) -> Result<(), CargoManifestProbeError> {
        match self.terminal.load(Ordering::Acquire) {
            OUTCOME_CLEAN => Ok(()),
            OUTCOME_CHANGED => Err(CargoManifestProbeError::Changed),
            _ => Err(CargoManifestProbeError::Unavailable),
        }
    }

    pub(super) fn flush(&self) -> Result<(), CargoManifestProbeError> {
        self.poll()?;
        if self.candidates.is_empty() {
            return Ok(());
        }
        let _replay_lock = FSEVENTS_REPLAY_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| CargoManifestProbeError::Unavailable)?;
        let mut cursor = self
            .cursor
            .lock()
            .map_err(|_| CargoManifestProbeError::Unavailable)?;
        if copy_volume_uuid(cursor.volume)? != cursor.volume_uuid {
            self.mark_unavailable();
            return self.poll();
        }

        let root = std::str::from_utf8(&self.watch_root)
            .map_err(|_| CargoManifestProbeError::Unsupported)?;
        let watch_string = CFString::new(root);
        let watch_array = CFArray::from_CFTypes(&[watch_string]);
        let state = Arc::new(EventState::new(
            self.candidates.clone(),
            self.watch_root.clone(),
        ));
        let mut context = FSEventStreamContext {
            version: 0,
            info: Arc::as_ptr(&state).cast_mut().cast(),
            retain: None,
            release: None,
            copy_description: None,
        };
        let stream = unsafe {
            FSEventStreamCreate(
                ptr::null(),
                event_callback,
                &mut context,
                watch_array.as_concrete_TypeRef(),
                cursor.event_id,
                0.01,
                CREATE_FLAG_NO_DEFER | CREATE_FLAG_WATCH_ROOT | CREATE_FLAG_FILE_EVENTS,
            )
        };
        if stream.is_null() {
            self.mark_unavailable();
            return self.poll();
        }
        let queue = unsafe {
            dispatch_queue_create(c"se.mjukis.dux.cargo-manifest-replay".as_ptr(), ptr::null())
        };
        if queue.is_null() {
            unsafe { FSEventStreamRelease(stream) };
            self.mark_unavailable();
            return self.poll();
        }
        unsafe { FSEventStreamSetDispatchQueue(stream, queue) };
        let started = unsafe { FSEventStreamStart(stream) } != 0;
        if started {
            unsafe { FSEventStreamFlushSync(stream) };
        }
        let latest = if started {
            unsafe { FSEventStreamGetLatestEventId(stream) }
        } else {
            cursor.event_id
        };
        unsafe {
            if started {
                FSEventStreamStop(stream);
            }
            FSEventStreamInvalidate(stream);
            FSEventStreamRelease(stream);
            dispatch_release(queue);
        }
        if !started || !state.history_done.load(Ordering::Acquire) || latest < cursor.event_id {
            self.mark_unavailable();
            return self.poll();
        }
        if copy_volume_uuid(cursor.volume)? != cursor.volume_uuid {
            self.mark_unavailable();
            return self.poll();
        }
        cursor.event_id = latest;
        match state.outcome.load(Ordering::Acquire) {
            OUTCOME_CLEAN => Ok(()),
            OUTCOME_CHANGED => {
                self.terminal.store(OUTCOME_CHANGED, Ordering::Release);
                Err(CargoManifestProbeError::Changed)
            }
            _ => {
                self.mark_unavailable();
                self.poll()
            }
        }
    }

    fn mark_unavailable(&self) {
        self.terminal.store(OUTCOME_UNAVAILABLE, Ordering::Release);
    }
}

#[derive(Clone)]
struct CandidatePath {
    full: Vec<u8>,
    parent: Vec<u8>,
    case_sensitive: bool,
}

struct EventState {
    candidates: Vec<CandidatePath>,
    watch_root: Vec<u8>,
    outcome: AtomicU8,
    history_done: AtomicBool,
    callback_events: AtomicUsize,
}

impl EventState {
    fn new(candidates: Vec<CandidatePath>, watch_root: Vec<u8>) -> Self {
        Self {
            candidates,
            watch_root,
            outcome: AtomicU8::new(OUTCOME_CLEAN),
            history_done: AtomicBool::new(false),
            callback_events: AtomicUsize::new(0),
        }
    }

    fn mark_changed(&self) {
        self.outcome.fetch_max(OUTCOME_CHANGED, Ordering::AcqRel);
    }

    fn mark_unavailable(&self) {
        self.outcome.store(OUTCOME_UNAVAILABLE, Ordering::Release);
    }

    fn observe(&self, path: &[u8], flags: u32) {
        if flags & !KNOWN_EVENT_FLAGS != 0 {
            self.mark_unavailable();
            return;
        }
        if flags & EVENT_FLAG_HISTORY_DONE != 0 {
            self.history_done.store(true, Ordering::Release);
            if flags & !EVENT_FLAG_HISTORY_DONE != 0 {
                self.mark_unavailable();
            }
            return;
        }
        if flags & EVENT_FLAG_IDS_WRAPPED != 0 {
            self.mark_unavailable();
            return;
        }
        if flags & DROPPED_FLAGS != 0 {
            if self.path_affects_candidate(path) {
                self.mark_unavailable();
            }
            return;
        }
        if flags & VOLUME_FLAGS != 0 {
            if self.path_affects_candidate(path)
                || is_path_prefix(path, &self.watch_root)
                || is_path_prefix(&self.watch_root, path)
            {
                self.mark_changed();
            }
            return;
        }
        if self.matches_candidate(path) {
            self.mark_changed();
        }
    }

    fn matches_candidate(&self, path: &[u8]) -> bool {
        self.candidates.iter().any(|candidate| {
            path == candidate.full
                || (!candidate.case_sensitive
                    && same_case_insensitive_cargo_name(&candidate.parent, path))
        })
    }

    fn path_affects_candidate(&self, path: &[u8]) -> bool {
        self.candidates.iter().any(|candidate| {
            path == candidate.full
                || is_path_prefix(path, &candidate.full)
                || is_path_prefix(path, &candidate.parent)
        })
    }
}

extern "C" fn event_callback(
    _stream: ConstFSEventStreamRef,
    info: *mut c_void,
    event_count: usize,
    event_paths: *mut c_void,
    event_flags: *const u32,
    _event_ids: *const u64,
) {
    if info.is_null() {
        return;
    }
    let state = unsafe { &*info.cast::<EventState>() };
    if event_count > MAX_CALLBACK_EVENTS
        || (event_count != 0 && (event_paths.is_null() || event_flags.is_null()))
    {
        state.mark_unavailable();
        return;
    }
    let previous = state
        .callback_events
        .fetch_add(event_count, Ordering::AcqRel);
    if previous.saturating_add(event_count) > MAX_CALLBACK_EVENTS {
        state.mark_unavailable();
        return;
    }
    let paths = event_paths.cast::<*const c_char>();
    for index in 0..event_count {
        let flags = unsafe { *event_flags.add(index) };
        if flags & EVENT_FLAG_HISTORY_DONE != 0 {
            state.observe(b"", flags);
            continue;
        }
        let path_pointer = unsafe { *paths.add(index) };
        if path_pointer.is_null() {
            state.mark_unavailable();
            continue;
        }
        let path = unsafe { CStr::from_ptr(path_pointer) }.to_bytes();
        state.observe(path, flags);
    }
}

fn copy_volume_uuid(volume: u64) -> Result<[u8; 16], CargoManifestProbeError> {
    let uuid = unsafe { FSEventsCopyUUIDForDevice(volume as libc::dev_t) };
    if uuid.is_null() {
        return Err(CargoManifestProbeError::Unavailable);
    }
    let bytes = unsafe { CFUUIDGetUUIDBytes(uuid.cast()) };
    unsafe { CFRelease(uuid.cast()) };
    Ok([
        bytes.byte0,
        bytes.byte1,
        bytes.byte2,
        bytes.byte3,
        bytes.byte4,
        bytes.byte5,
        bytes.byte6,
        bytes.byte7,
        bytes.byte8,
        bytes.byte9,
        bytes.byte10,
        bytes.byte11,
        bytes.byte12,
        bytes.byte13,
        bytes.byte14,
        bytes.byte15,
    ])
}

fn case_sensitive_directory(path: &Path) -> Result<bool, CargoManifestProbeError> {
    let bytes = path.as_os_str().as_encoded_bytes();
    let c_path = std::ffi::CString::new(bytes).map_err(|_| CargoManifestProbeError::Unsupported)?;
    let result = unsafe { libc::pathconf(c_path.as_ptr(), libc::_PC_CASE_SENSITIVE) };
    match result {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CargoManifestProbeError::Unavailable),
    }
}

fn same_case_insensitive_cargo_name(parent: &[u8], path: &[u8]) -> bool {
    let Some(name) = path
        .strip_prefix(parent)
        .and_then(|suffix| suffix.strip_prefix(b"/"))
    else {
        return false;
    };
    name.eq_ignore_ascii_case(b"Cargo.toml") && !name.contains(&b'/')
}

fn is_path_prefix(prefix: &[u8], path: &[u8]) -> bool {
    if prefix == b"/" {
        return path.starts_with(prefix);
    }
    path.get(..prefix.len()) == Some(prefix)
        && path
            .get(prefix.len())
            .is_some_and(|separator| *separator == b'/')
}

#[cfg(test)]
mod tests {
    use super::{
        CandidatePath, EVENT_FLAG_HISTORY_DONE, EVENT_FLAG_MUST_SCAN_SUBDIRS,
        EVENT_FLAG_USER_DROPPED, EventState, OUTCOME_CHANGED, OUTCOME_CLEAN, OUTCOME_UNAVAILABLE,
        is_path_prefix, same_case_insensitive_cargo_name,
    };

    fn state(case_sensitive: bool) -> EventState {
        EventState::new(
            vec![CandidatePath {
                full: b"/tmp/project/Cargo.toml".to_vec(),
                parent: b"/tmp/project".to_vec(),
                case_sensitive,
            }],
            b"/tmp".to_vec(),
        )
    }

    #[test]
    fn path_prefix_is_component_aware() {
        assert!(is_path_prefix(b"/", b"/tmp/Cargo.toml"));
        assert!(is_path_prefix(b"/tmp", b"/tmp/Cargo.toml"));
        assert!(!is_path_prefix(b"/tmp", b"/tmp-other/Cargo.toml"));
        assert!(!is_path_prefix(b"/tmp/Cargo.toml", b"/tmp/Cargo.toml"));
    }

    #[test]
    fn case_insensitive_volume_matches_only_fixed_manifest_name() {
        assert!(same_case_insensitive_cargo_name(
            b"/tmp/project",
            b"/tmp/project/cargo.toml"
        ));
        assert!(!same_case_insensitive_cargo_name(
            b"/tmp/project",
            b"/tmp/project/Cargo.toml/child"
        ));
        assert!(!same_case_insensitive_cargo_name(
            b"/tmp/project",
            b"/tmp/project-other/Cargo.toml"
        ));
    }

    #[test]
    fn exact_and_case_variant_events_change_insensitive_candidates() {
        let insensitive = state(false);
        insensitive.observe(b"/tmp/project/cargo.toml", 0);
        assert_eq!(
            insensitive
                .outcome
                .load(std::sync::atomic::Ordering::Acquire),
            OUTCOME_CHANGED
        );

        let sensitive = state(true);
        sensitive.observe(b"/tmp/project/cargo.toml", 0);
        assert_eq!(
            sensitive.outcome.load(std::sync::atomic::Ordering::Acquire),
            OUTCOME_CLEAN
        );
    }

    #[test]
    fn dropped_events_fail_closed_only_when_they_cover_a_candidate() {
        let unrelated = state(false);
        unrelated.observe(
            b"/tmp/other",
            EVENT_FLAG_MUST_SCAN_SUBDIRS | EVENT_FLAG_USER_DROPPED,
        );
        assert_eq!(
            unrelated.outcome.load(std::sync::atomic::Ordering::Acquire),
            OUTCOME_CLEAN
        );

        let affected = state(false);
        affected.observe(b"/tmp/project", EVENT_FLAG_MUST_SCAN_SUBDIRS);
        assert_eq!(
            affected.outcome.load(std::sync::atomic::Ordering::Acquire),
            OUTCOME_UNAVAILABLE
        );
    }

    #[test]
    fn history_done_is_required_and_does_not_change_state() {
        let state = state(false);
        state.observe(b"", EVENT_FLAG_HISTORY_DONE);
        assert!(
            state
                .history_done
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert_eq!(
            state.outcome.load(std::sync::atomic::Ordering::Acquire),
            OUTCOME_CLEAN
        );
    }

    #[test]
    fn unknown_event_flags_fail_closed() {
        let state = state(false);
        state.observe(b"/tmp/other", 0x8000_0000);
        assert_eq!(
            state.outcome.load(std::sync::atomic::Ordering::Acquire),
            OUTCOME_UNAVAILABLE
        );
    }
}
