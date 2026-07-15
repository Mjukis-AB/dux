use std::collections::HashMap;
use std::fs::Metadata;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use crossbeam_channel::{Receiver, Sender};
use jwalk::WalkDir;

use super::filesystem::{self, FilesystemDisposition};
use super::probe_pool::{ProbePool, ProbePoolError, directory_probe_pool};
use super::progress::{ScanMessage, ScanProgress};
use crate::tree::{DiskTree, NodeId, NodeKind};

/// Scanner configuration
#[derive(Debug, Clone)]
pub struct ScanConfig {
    /// Follow symbolic links
    pub follow_symlinks: bool,
    /// Maximum depth to scan (None = unlimited)
    pub max_depth: Option<usize>,
    /// Stay on same filesystem (don't cross mount points)
    pub same_filesystem: bool,
    /// Number of parallel threads (0 = auto)
    pub num_threads: usize,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            follow_symlinks: false,
            max_depth: None,
            same_filesystem: true,
            num_threads: 0, // auto
        }
    }
}

/// Cancellation token for stopping scans
#[derive(Debug, Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared progress state for heartbeat updates
struct SharedProgress {
    files_scanned: AtomicU64,
    dirs_scanned: AtomicU64,
    bytes_scanned: AtomicU64,
    errors: AtomicU64,
    current_path: Mutex<Option<PathBuf>>,
    done: AtomicBool,
}

impl SharedProgress {
    fn new() -> Self {
        Self {
            files_scanned: AtomicU64::new(0),
            dirs_scanned: AtomicU64::new(0),
            bytes_scanned: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            current_path: Mutex::new(None),
            done: AtomicBool::new(false),
        }
    }

    fn to_scan_progress(&self) -> ScanProgress {
        ScanProgress {
            files_scanned: self.files_scanned.load(Ordering::Relaxed),
            dirs_scanned: self.dirs_scanned.load(Ordering::Relaxed),
            bytes_scanned: self.bytes_scanned.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            current_path: self.current_path.lock().ok().and_then(|g| g.clone()),
        }
    }
}

/// Structured rules for paths that are potentially slow or virtual.
///
/// Matching path components instead of string fragments avoids treating a
/// normal directory such as `<scan root>/dev` as the system `/dev` tree.
#[derive(Debug, Clone, Copy)]
enum SlowPathRule {
    AbsolutePrefix(&'static str),
    Component(&'static str),
    MacUserHomePrefix(&'static str),
}

impl SlowPathRule {
    fn matches(self, path: &Path) -> bool {
        match self {
            Self::AbsolutePrefix(prefix) => path.starts_with(Path::new(prefix)),
            Self::Component(name) => path.components().any(|part| part.as_os_str() == name),
            Self::MacUserHomePrefix(prefix) => {
                let mut parts = path.components();
                matches!(parts.next(), Some(Component::RootDir))
                    && parts.next().is_some_and(|part| part.as_os_str() == "Users")
                    && matches!(parts.next(), Some(Component::Normal(_)))
                    && parts.as_path().starts_with(Path::new(prefix))
            }
        }
    }
}

const SLOW_PATH_RULES: &[SlowPathRule] = &[
    SlowPathRule::Component(".Spotlight-V100"),
    SlowPathRule::Component(".fseventsd"),
    SlowPathRule::Component(".DocumentRevisions-V100"),
    SlowPathRule::AbsolutePrefix("/Library/Developer/CoreSimulator/Volumes"),
    SlowPathRule::MacUserHomePrefix("Library/Developer/CoreSimulator/Volumes"),
    SlowPathRule::Component(".MobileBackups"),
    SlowPathRule::Component(".timemachine"),
    SlowPathRule::AbsolutePrefix("/dev"),
    SlowPathRule::AbsolutePrefix("/proc"),
    SlowPathRule::AbsolutePrefix("/sys"),
    SlowPathRule::AbsolutePrefix("/private/var/folders"),
    SlowPathRule::AbsolutePrefix("/private/var/db/dyld"),
    SlowPathRule::AbsolutePrefix("/private/var/db/uuidtext"),
    SlowPathRule::AbsolutePrefix("/Library/CloudStorage"),
    SlowPathRule::MacUserHomePrefix("Library/CloudStorage"),
    SlowPathRule::AbsolutePrefix("/Library/Mobile Documents"),
    SlowPathRule::MacUserHomePrefix("Library/Mobile Documents"),
];

/// Check if a path looks like a virtual/problematic filesystem path.
///
/// A rule is ignored when the scan root matches the same rule, allowing a user
/// to explicitly scan a location that DUX would skip while scanning its parent.
fn is_virtual_or_slow_path(path: &std::path::Path, root_path: &std::path::Path) -> bool {
    // If this path is the root or an ancestor of root, don't skip it
    if path == root_path || root_path.starts_with(path) {
        return false;
    }

    SLOW_PATH_RULES
        .iter()
        .copied()
        .any(|rule| rule.matches(path) && !rule.matches(root_path))
}

/// How long to wait for a metadata() call before assuming the path is on a slow/hung filesystem.
const METADATA_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct DirectoryProbe {
    metadata: Metadata,
    filesystem: FilesystemDisposition,
}

struct DirectoryProbeRequest {
    path: PathBuf,
    root_dev: u64,
    classify_crossed_filesystem: bool,
    filesystem_cache: Arc<Mutex<HashMap<u64, FilesystemDisposition>>>,
}

/// Fetch metadata and, when crossing a device boundary, classify its filesystem.
/// Pool failures are distinct from ordinary metadata errors so callers can
/// avoid treating a missing entry as a pool-wide timeout.
fn directory_probe_with_timeout(
    path: &Path,
    root_dev: u64,
    classify_crossed_filesystem: bool,
    filesystem_cache: Arc<Mutex<HashMap<u64, FilesystemDisposition>>>,
    cancellation: &CancellationToken,
) -> Result<Option<DirectoryProbe>, ProbePoolError> {
    directory_probe_with(
        directory_probe_pool(),
        DirectoryProbeRequest {
            path: path.to_path_buf(),
            root_dev,
            classify_crossed_filesystem,
            filesystem_cache,
        },
        METADATA_TIMEOUT,
        cancellation,
        filesystem::probe,
    )
}

fn directory_probe_with<F>(
    pool: &ProbePool,
    request: DirectoryProbeRequest,
    timeout: Duration,
    cancellation: &CancellationToken,
    probe_filesystem: F,
) -> Result<Option<DirectoryProbe>, ProbePoolError>
where
    F: FnOnce(&Path) -> FilesystemDisposition + Send + 'static,
{
    pool.run(timeout, cancellation, move || {
        let result = std::fs::metadata(&request.path).map(|metadata| {
            let device = get_device_id(&metadata);
            let filesystem = if request.classify_crossed_filesystem && device != request.root_dev {
                request
                    .filesystem_cache
                    .lock()
                    .ok()
                    .and_then(|cache| cache.get(&device).copied())
                    .unwrap_or_else(|| {
                        // Concurrent first visits may duplicate this probe. Do
                        // not hold the mutex across a syscall that can hang.
                        let disposition = probe_filesystem(&request.path);
                        if let Ok(mut cache) = request.filesystem_cache.lock() {
                            cache.insert(device, disposition);
                        }
                        disposition
                    })
            } else {
                FilesystemDisposition::Unknown
            };

            DirectoryProbe {
                metadata,
                filesystem,
            }
        });
        result.ok()
    })
}

fn should_skip_directory(
    same_filesystem: bool,
    root_dev: u64,
    directory_dev: u64,
    filesystem: FilesystemDisposition,
) -> bool {
    directory_dev != root_dev
        && (same_filesystem || filesystem == FilesystemDisposition::NetworkOrVirtual)
}

/// Filesystem scanner
pub struct Scanner {
    config: ScanConfig,
    cancel_token: CancellationToken,
}

impl Scanner {
    pub fn new(config: ScanConfig) -> Self {
        Self {
            config,
            cancel_token: CancellationToken::new(),
        }
    }

    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancel_token = token;
        self
    }

    /// Scan a directory and build a tree
    /// Returns a receiver for progress updates and spawns scanning in background
    pub fn scan(
        self,
        root_path: PathBuf,
    ) -> (Receiver<ScanMessage>, std::thread::JoinHandle<DiskTree>) {
        let (tx, rx) = crossbeam_channel::unbounded();

        let handle = std::thread::spawn(move || self.scan_sync(root_path, tx));

        (rx, handle)
    }

    /// Synchronous scan (runs in thread)
    fn scan_sync(self, root_path: PathBuf, tx: Sender<ScanMessage>) -> DiskTree {
        let root_path = root_path.canonicalize().unwrap_or(root_path);
        let mut tree = DiskTree::new(root_path.clone());

        // Set root mtime for cache invalidation
        if let Ok(root_meta) = std::fs::metadata(&root_path)
            && let Ok(mtime) = root_meta.modified()
            && let Some(root_node) = tree.get_mut(NodeId::ROOT)
        {
            root_node.mtime = Some(mtime);
        }

        // Map from path to node ID for parent lookups
        let mut path_to_id: HashMap<PathBuf, NodeId> = HashMap::new();
        path_to_id.insert(root_path.clone(), NodeId::ROOT);

        // Get root device for same-filesystem check
        let root_dev = std::fs::metadata(&root_path)
            .map(|m| get_device_id(&m))
            .unwrap_or(0);

        // Shared progress state
        let shared_progress = Arc::new(SharedProgress::new());
        let progress_for_heartbeat = Arc::clone(&shared_progress);
        let tx_for_heartbeat = tx.clone();
        let cancel_for_heartbeat = self.cancel_token.clone();

        // Spawn heartbeat thread that sends progress every 100ms
        let heartbeat_handle = std::thread::spawn(move || {
            while !progress_for_heartbeat.done.load(Ordering::Relaxed)
                && !cancel_for_heartbeat.is_cancelled()
            {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let progress = progress_for_heartbeat.to_scan_progress();
                let _ = tx_for_heartbeat.send(ScanMessage::Progress(progress));
            }
        });

        let _ = tx.send(ScanMessage::StartedDirectory(root_path.clone()));

        // Configure walker with process_read_dir to skip problematic directories
        let same_fs = self.config.same_filesystem;
        let root_for_filter = root_path.clone();
        let cancel_for_filter = self.cancel_token.clone();
        let filesystem_cache = Arc::new(Mutex::new(HashMap::new()));
        let walker = WalkDir::new(&root_path)
            .skip_hidden(false)
            .follow_links(self.config.follow_symlinks)
            .sort(false) // We'll sort by size later
            .process_read_dir(move |_depth, path, _read_dir_state, children| {
                if cancel_for_filter.is_cancelled() {
                    children.clear();
                    return;
                }

                // Skip children in virtual/slow directories
                if is_virtual_or_slow_path(path, &root_for_filter) {
                    children.clear();
                    return;
                }

                children.retain(|entry| {
                    if cancel_for_filter.is_cancelled() {
                        return false;
                    }

                    if let Ok(e) = entry {
                        // Check if child path is virtual/slow
                        if is_virtual_or_slow_path(&e.path(), &root_for_filter) {
                            return false;
                        }

                        // For directories, probe metadata with a timeout to detect
                        // slow FUSE/network mounts before jwalk descends into them.
                        // Metadata must stay inside this timeout: asking jwalk's
                        // entry for metadata first can itself block on the mount.
                        if e.file_type().is_dir() {
                            match directory_probe_with_timeout(
                                &e.path(),
                                root_dev,
                                !same_fs,
                                Arc::clone(&filesystem_cache),
                                &cancel_for_filter,
                            ) {
                                Ok(Some(probe)) => {
                                    return !should_skip_directory(
                                        same_fs,
                                        root_dev,
                                        get_device_id(&probe.metadata),
                                        probe.filesystem,
                                    );
                                }
                                Ok(None) => return false,
                                Err(_) => return false,
                            }
                        } else if same_fs {
                            // For files, use jwalk's cached metadata (already fetched)
                            if let Ok(meta) = e.metadata()
                                && get_device_id(&meta) != root_dev
                            {
                                return false;
                            }
                        }
                    }
                    true
                });
            });

        let walker = if let Some(depth) = self.config.max_depth {
            walker.max_depth(depth)
        } else {
            walker
        };

        let walker = if self.config.num_threads > 0 {
            walker.parallelism(jwalk::Parallelism::RayonNewPool(self.config.num_threads))
        } else {
            walker
        };

        for entry_result in walker {
            // Check for cancellation
            if self.cancel_token.is_cancelled() {
                shared_progress.done.store(true, Ordering::Relaxed);
                let _ = heartbeat_handle.join();
                let _ = tx.send(ScanMessage::Cancelled);
                return tree;
            }

            let entry = match entry_result {
                Ok(e) => e,
                Err(_e) => {
                    shared_progress.errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };

            let path = entry.path();

            // Skip root (already added)
            if path == root_path {
                continue;
            }

            // Get metadata
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => {
                    shared_progress.errors.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };

            // Check filesystem boundary
            if self.config.same_filesystem && get_device_id(&metadata) != root_dev {
                continue;
            }

            // Determine node kind
            let file_type = entry.file_type();
            let kind = if file_type.is_dir() {
                NodeKind::Directory
            } else if file_type.is_file() {
                NodeKind::File
            } else if file_type.is_symlink() {
                NodeKind::Symlink
            } else {
                NodeKind::Other
            };
            let path_is_symlink = entry.path_is_symlink();

            // Get parent path and node ID
            let parent_path = match path.parent() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };

            let parent_id = match path_to_id.get(&parent_path) {
                Some(&id) => id,
                None => continue, // Parent not in tree (skipped?)
            };

            // Get name
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());

            // Add node
            let node_id = tree.add_node(name, kind, path.clone(), parent_id);
            if let Some(node) = tree.get_mut(node_id) {
                node.path_is_symlink = path_is_symlink;
            }

            // Track path and mtime for directories
            if kind == NodeKind::Directory {
                path_to_id.insert(path.clone(), node_id);
                if let Ok(mtime) = metadata.modified()
                    && let Some(node) = tree.get_mut(node_id)
                {
                    node.mtime = Some(mtime);
                }
                shared_progress.dirs_scanned.fetch_add(1, Ordering::Relaxed);
            } else {
                shared_progress
                    .files_scanned
                    .fetch_add(1, Ordering::Relaxed);
            }

            // Set size for files
            let size = get_disk_usage(&metadata);
            tree.set_size(node_id, size);
            shared_progress
                .bytes_scanned
                .fetch_add(size, Ordering::Relaxed);

            // Update current path
            if let Ok(mut guard) = shared_progress.current_path.lock() {
                *guard = Some(path.clone());
            }
        }

        // Stop heartbeat thread
        shared_progress.done.store(true, Ordering::Relaxed);
        let _ = heartbeat_handle.join();

        // Send finalizing message (aggregation can take time on large trees)
        let _ = tx.send(ScanMessage::Finalizing);

        // Aggregate sizes from children to parents
        tree.aggregate_sizes();

        // Sort all children by size
        tree.sort_by_size();

        // Send final progress
        let progress = shared_progress.to_scan_progress();
        let _ = tx.send(ScanMessage::Progress(progress));
        let _ = tx.send(ScanMessage::Completed);

        tree
    }
}

/// Get actual disk usage for a file (accounts for sparse files and block size)
#[cfg(unix)]
fn get_disk_usage(metadata: &Metadata) -> u64 {
    // st_blocks is in 512-byte units
    metadata.blocks() * 512
}

/// Get actual disk usage for a file (Windows fallback - uses file size)
#[cfg(not(unix))]
fn get_disk_usage(metadata: &Metadata) -> u64 {
    metadata.len()
}

/// Get device ID for same-filesystem checks
#[cfg(unix)]
fn get_device_id(metadata: &Metadata) -> u64 {
    metadata.dev()
}

/// Get device ID (Windows - not supported, return 0)
#[cfg(not(unix))]
fn get_device_id(_metadata: &Metadata) -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_scan_empty_dir() {
        let temp = TempDir::new().unwrap();
        let scanner = Scanner::new(ScanConfig::default());
        let (rx, handle) = scanner.scan(temp.path().to_path_buf());

        // Drain messages
        for _ in rx {}

        let tree = handle.join().unwrap();
        assert_eq!(tree.len(), 1); // Just root
    }

    #[test]
    fn test_scan_with_files() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("file1.txt"), "hello").unwrap();
        fs::write(temp.path().join("file2.txt"), "world").unwrap();
        fs::create_dir(temp.path().join("subdir")).unwrap();
        fs::write(temp.path().join("subdir/file3.txt"), "test").unwrap();

        let scanner = Scanner::new(ScanConfig::default());
        let (rx, handle) = scanner.scan(temp.path().to_path_buf());

        for _ in rx {}

        let tree = handle.join().unwrap();
        assert!(tree.len() >= 4); // root + 2 files + subdir + 1 file
    }

    #[test]
    fn slow_path_rules_match_boundaries_not_substrings() {
        let root = Path::new("/tmp/dux-scan");

        assert!(is_virtual_or_slow_path(Path::new("/dev/null"), root));
        assert!(is_virtual_or_slow_path(
            Path::new("/Users/test/Library/CloudStorage/provider"),
            root
        ));
        assert!(is_virtual_or_slow_path(
            Path::new("/Users/test/Library/Developer/CoreSimulator/Volumes/device"),
            root
        ));

        assert!(!is_virtual_or_slow_path(
            Path::new("/tmp/dux-scan/dev/payload.bin"),
            root
        ));
        assert!(!is_virtual_or_slow_path(
            Path::new("/tmp/dux-scan/developer/payload.bin"),
            root
        ));
        assert!(!is_virtual_or_slow_path(
            Path::new("/tmp/dux-scan/.timemachine-backup/data"),
            root
        ));
        assert!(!is_virtual_or_slow_path(
            Path::new("/tmp/dux-scan/Library/CloudStorage-copy/data"),
            root
        ));
        assert!(!is_virtual_or_slow_path(
            Path::new("/tmp/dux-scan/project/Library/CloudStorage/data"),
            root
        ));
        assert!(!is_virtual_or_slow_path(
            Path::new("/tmp/dux-scan/fixture/CoreSimulator/Volumes/device"),
            root
        ));
    }

    #[test]
    fn slow_path_rules_allow_explicit_scan_roots() {
        assert!(!is_virtual_or_slow_path(
            Path::new("/dev/fd"),
            Path::new("/dev")
        ));
        assert!(!is_virtual_or_slow_path(
            Path::new("/tmp/root/.fseventsd/events"),
            Path::new("/tmp/root/.fseventsd")
        ));
    }

    #[test]
    fn volumes_are_classified_by_filesystem_instead_of_path() {
        assert!(!is_virtual_or_slow_path(
            Path::new("/Volumes/External/data"),
            Path::new("/")
        ));
    }

    #[test]
    fn mount_policy_preserves_roots_and_rejects_problematic_crossings() {
        assert!(!should_skip_directory(
            false,
            1,
            1,
            FilesystemDisposition::NetworkOrVirtual
        ));
        assert!(should_skip_directory(
            true,
            1,
            2,
            FilesystemDisposition::Local
        ));
        assert!(should_skip_directory(
            false,
            1,
            2,
            FilesystemDisposition::NetworkOrVirtual
        ));
        assert!(!should_skip_directory(
            false,
            1,
            2,
            FilesystemDisposition::Local
        ));
        assert!(!should_skip_directory(
            false,
            1,
            2,
            FilesystemDisposition::Unknown
        ));
    }

    #[test]
    fn directory_probe_timeout_rejects_the_subtree() {
        let temp = TempDir::new().unwrap();
        let device = get_device_id(&fs::metadata(temp.path()).unwrap());
        let cache = Arc::new(Mutex::new(HashMap::new()));
        let pool = ProbePool::new(1, 1);

        let result = directory_probe_with(
            &pool,
            DirectoryProbeRequest {
                path: temp.path().to_path_buf(),
                root_dev: device.wrapping_add(1),
                classify_crossed_filesystem: true,
                filesystem_cache: cache,
            },
            Duration::from_millis(1),
            &CancellationToken::new(),
            |_| {
                std::thread::sleep(Duration::from_millis(25));
                FilesystemDisposition::Local
            },
        );

        assert!(matches!(result, Err(ProbePoolError::DeadlineExceeded)));
    }

    #[test]
    fn metadata_error_does_not_report_pool_unavailability() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("missing");
        let pool = ProbePool::new(1, 1);

        let result = directory_probe_with(
            &pool,
            DirectoryProbeRequest {
                path: missing,
                root_dev: 0,
                classify_crossed_filesystem: true,
                filesystem_cache: Arc::new(Mutex::new(HashMap::new())),
            },
            Duration::from_secs(1),
            &CancellationToken::new(),
            |_| FilesystemDisposition::Local,
        );

        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn timed_out_probe_does_not_prevent_the_next_probe() {
        let temp = TempDir::new().unwrap();
        let device = get_device_id(&fs::metadata(temp.path()).unwrap());
        let pool = ProbePool::new(1, 1);
        let request = || DirectoryProbeRequest {
            path: temp.path().to_path_buf(),
            root_dev: device.wrapping_add(1),
            classify_crossed_filesystem: true,
            filesystem_cache: Arc::new(Mutex::new(HashMap::new())),
        };

        let timed_out = directory_probe_with(
            &pool,
            request(),
            Duration::from_millis(1),
            &CancellationToken::new(),
            |_| {
                std::thread::sleep(Duration::from_millis(20));
                FilesystemDisposition::Local
            },
        );
        assert!(matches!(timed_out, Err(ProbePoolError::DeadlineExceeded)));

        let next = directory_probe_with(
            &pool,
            request(),
            Duration::from_secs(1),
            &CancellationToken::new(),
            |_| FilesystemDisposition::Local,
        );
        assert!(matches!(next, Ok(Some(_))));
    }

    #[test]
    fn scanner_keeps_nested_directory_named_dev() {
        let temp = TempDir::new().unwrap();
        let nested_dev = temp.path().join("dev");
        let payload = nested_dev.join("payload.bin");
        fs::create_dir(&nested_dev).unwrap();
        fs::write(&payload, b"payload").unwrap();

        let scanner = Scanner::new(ScanConfig::default());
        let (rx, handle) = scanner.scan(temp.path().to_path_buf());
        for _ in rx {}

        let tree = handle.join().unwrap();
        assert!(
            tree.find_by_path(&nested_dev.canonicalize().unwrap())
                .is_some()
        );
        assert!(
            tree.find_by_path(&payload.canonicalize().unwrap())
                .is_some()
        );
    }

    #[cfg(unix)]
    #[test]
    fn followed_symlinks_preserve_path_provenance() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let targets = TempDir::new().unwrap();
        let real_dir = targets.path().join("real-dir");
        let linked_dir = temp.path().join("linked-dir");
        let real_manifest = targets.path().join("real-manifest");
        let linked_manifest = temp.path().join("Cargo.toml");
        fs::create_dir(&real_dir).unwrap();
        fs::write(real_dir.join("output"), b"payload").unwrap();
        fs::write(&real_manifest, b"[package]").unwrap();
        symlink(&real_dir, &linked_dir).unwrap();
        symlink(&real_manifest, &linked_manifest).unwrap();

        let scanner = Scanner::new(ScanConfig {
            follow_symlinks: true,
            ..ScanConfig::default()
        });
        let (rx, handle) = scanner.scan(temp.path().to_path_buf());
        for _ in rx {}
        let tree = handle.join().unwrap();

        let canonical_root = temp.path().canonicalize().unwrap();
        let linked_dir = canonical_root.join("linked-dir");
        let linked_manifest = canonical_root.join("Cargo.toml");
        let scanned_paths = || {
            tree.iter()
                .map(|node| node.path.clone())
                .collect::<Vec<_>>()
        };
        let linked_dir_id = tree
            .find_by_path(&linked_dir)
            .unwrap_or_else(|| panic!("linked directory missing from {:#?}", scanned_paths()));
        let linked_manifest_id = tree
            .find_by_path(&linked_manifest)
            .unwrap_or_else(|| panic!("linked marker missing from {:#?}", scanned_paths()));
        let linked_dir_node = tree.get(linked_dir_id).unwrap();
        let linked_manifest_node = tree.get(linked_manifest_id).unwrap();
        assert_eq!(linked_dir_node.kind, NodeKind::Directory);
        assert!(linked_dir_node.path_is_symlink);
        assert_eq!(linked_manifest_node.kind, NodeKind::File);
        assert!(linked_manifest_node.path_is_symlink);
    }
}
