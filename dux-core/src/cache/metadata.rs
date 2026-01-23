use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Current cache format version - increment when format changes
pub const CACHE_VERSION: u32 = 1;

/// Magic bytes identifying a DUX cache file
pub const CACHE_MAGIC: [u8; 4] = *b"DUXC";

/// Metadata stored with cached tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    /// Cache format version
    pub version: u32,
    /// Root path that was scanned
    pub root_path: PathBuf,
    /// When the scan was performed
    pub scan_time: SystemTime,
    /// Modification time of root directory at scan time (for quick invalidation)
    pub root_mtime: SystemTime,
    /// Total size of scanned tree
    pub total_size: u64,
    /// Number of nodes in tree
    pub node_count: usize,
    /// Scan configuration used
    pub config: CachedScanConfig,
}

/// Scan configuration that affects cache validity
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CachedScanConfig {
    /// Whether symlinks were followed during scan
    pub follow_symlinks: bool,
    /// Whether scan stayed on same filesystem
    pub same_filesystem: bool,
    /// Maximum depth that was scanned
    pub max_depth: Option<usize>,
}
