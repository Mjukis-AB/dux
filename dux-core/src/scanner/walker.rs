use std::collections::HashMap;
use std::fs::Metadata;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use crossbeam_channel::{Receiver, Sender};
use jwalk::WalkDir;

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

/// Patterns that indicate potentially slow/problematic paths
const SLOW_PATTERNS: &[&str] = &[
    "/Volumes/",                // Mounted volumes (might be network/external)
    "/.Spotlight-V100",         // Spotlight index
    "/.fseventsd",              // FSEvents
    "/.DocumentRevisions-V100", // Document versions
    "/System/Volumes/Data/.Spotlight-V100",
    "CoreSimulator/Volumes",    // iOS Simulator disk images
    "/.MobileBackups",          // Mobile backups
    ".timemachine",             // Time Machine
    "/dev/",                    // Device files
    "/proc/",                   // Linux proc filesystem
    "/sys/",                    // Linux sys filesystem
    "/private/var/folders",     // macOS temp folders (can hang)
    "/private/var/db/dyld",     // dyld cache (permission issues)
    "/private/var/db/uuidtext", // UUID text (slow)
];

/// Check if a path looks like a virtual/problematic filesystem path
/// Only returns true if the path contains a slow pattern AND is not under the root path
fn is_virtual_or_slow_path(path: &std::path::Path, root_path: &std::path::Path) -> bool {
    // If this path is the root or an ancestor of root, don't skip it
    if path == root_path || root_path.starts_with(path) {
        return false;
    }

    let path_str = path.to_string_lossy();
    let root_str = root_path.to_string_lossy();

    // Check for known virtual filesystem patterns
    for pattern in SLOW_PATTERNS {
        // Only skip if the pattern appears in the path but NOT in the root path
        // This allows scanning within /private/var/folders if that's where we started
        if path_str.contains(pattern) && !root_str.contains(pattern) {
            return true;
        }
    }

    false
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
        let walker = WalkDir::new(&root_path)
            .skip_hidden(false)
            .follow_links(self.config.follow_symlinks)
            .sort(false) // We'll sort by size later
            .process_read_dir(move |_depth, path, _read_dir_state, children| {
                // Skip children in virtual/slow directories
                if is_virtual_or_slow_path(path, &root_for_filter) {
                    children.clear();
                    return;
                }

                // Filter out children that are on different filesystems or are virtual
                if same_fs {
                    children.retain(|entry| {
                        if let Ok(e) = entry {
                            // Check if child is on same filesystem
                            if let Ok(meta) = e.metadata()
                                && get_device_id(&meta) != root_dev
                            {
                                return false;
                            }
                            // Check if child path is virtual/slow
                            if is_virtual_or_slow_path(&e.path(), &root_for_filter) {
                                return false;
                            }
                        }
                        true
                    });
                }
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
            } else if file_type.is_symlink() {
                NodeKind::Symlink
            } else {
                NodeKind::File
            };

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
}
