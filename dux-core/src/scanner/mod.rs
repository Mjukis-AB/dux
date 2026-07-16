mod accounting;
mod facts;
mod filesystem;
mod issues;
mod outcome;
mod probe_pool;
mod progress;
mod walker;

pub use outcome::{ScanOutcome, ScanTermination};
pub use progress::{ScanMessage, ScanProgress};
pub use walker::{CancellationToken, ScanConfig, Scanner};

pub(crate) use facts::{FreshScanFacts, ScanNodeFlags, ScanObjectIdentity};
pub(crate) use outcome::CompletedScanArtifact;
