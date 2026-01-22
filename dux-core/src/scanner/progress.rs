use std::path::PathBuf;

/// Progress update during scanning
#[derive(Debug, Clone)]
pub enum ScanMessage {
    /// Started scanning a directory
    StartedDirectory(PathBuf),
    /// Progress update
    Progress(ScanProgress),
    /// Finalizing (aggregating sizes, sorting)
    Finalizing,
    /// Scan completed
    Completed,
    /// Scan was cancelled
    Cancelled,
    /// Error during scanning
    Error(String),
}

/// Scanning progress statistics
#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    /// Number of files scanned
    pub files_scanned: u64,
    /// Number of directories scanned
    pub dirs_scanned: u64,
    /// Total bytes scanned so far
    pub bytes_scanned: u64,
    /// Number of errors encountered
    pub errors: u64,
    /// Current directory being scanned
    pub current_path: Option<PathBuf>,
}

impl ScanProgress {
    pub fn total_entries(&self) -> u64 {
        self.files_scanned + self.dirs_scanned
    }
}
