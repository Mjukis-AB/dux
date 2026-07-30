//! Read-only iCloud local-copy eligibility probing.
//!
//! The retained Explorer review chooses and revalidates the only path that may
//! cross the callback boundary. The callback can report bounded platform facts
//! for that path, but it cannot nominate a target or return a path. Rust stamps
//! the observation time and applies the deterministic eligibility policy.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::domain::{
    CloudEvictionAssessment, CloudEvictionItemKind, CloudEvictionObservation,
    CloudEvictionObservationInput, CloudEvictionPlatformFacts, CloudEvictionProvider,
    assess_cloud_eviction,
};

use super::{SnapshotReviewError, SnapshotReviewSession};

/// A core-issued, consume-once request for one exact reviewed file.
///
/// This is read-only evidence collection. It grants no candidate, plan,
/// journal, approval, or eviction capability.
#[must_use = "the probe request must be consumed by the platform callback"]
pub struct CloudEvictionProbeRequest {
    absolute_path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CloudEvictionProbeRequestError {
    #[error("the cloud probe path cannot be represented losslessly")]
    UnsupportedPathEncoding,
}

impl CloudEvictionProbeRequest {
    pub(crate) fn from_target(
        target: &super::snapshot_review::SnapshotReviewCloudEvictionTarget,
    ) -> Self {
        Self {
            absolute_path: target.snapshot.object_path().to_path_buf(),
        }
    }

    /// Consume the request into exact host path bytes. The path cannot be
    /// queried or reused afterward.
    pub fn into_path_bytes(self) -> Result<Vec<u8>, CloudEvictionProbeRequestError> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Ok(self.absolute_path.as_os_str().as_bytes().to_vec())
        }
        #[cfg(not(unix))]
        {
            let _ = self;
            Err(CloudEvictionProbeRequestError::UnsupportedPathEncoding)
        }
    }
}

/// Bounded failures from the trusted platform metadata adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudEvictionProbePlatformError {
    Unsupported,
    Failed,
}

/// Path-free failures from one read-only selected-file probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CloudEvictionProbeError {
    #[error("engine session is closed")]
    Closed,
    #[error("the snapshot review belongs to a different engine")]
    WrongReview,
    #[error("the selected snapshot node cannot be probed for cloud eviction")]
    InvalidTarget,
    #[error("the selected snapshot node changed since capture")]
    ChangedSinceSnapshot,
    #[error("the retained snapshot review is unavailable")]
    ReviewUnavailable,
    #[error("the platform does not support this cloud metadata probe")]
    PlatformUnsupported,
    #[error("the cloud metadata probe failed closed")]
    PlatformFailed,
}

pub(super) fn probe_selected_file<F>(
    review: &mut SnapshotReviewSession,
    node_id: u64,
    probe: F,
) -> Result<CloudEvictionAssessment, CloudEvictionProbeError>
where
    F: FnOnce(
        CloudEvictionProbeRequest,
    ) -> Result<CloudEvictionPlatformFacts, CloudEvictionProbePlatformError>,
{
    let target = review
        .cloud_eviction_target(node_id)
        .map_err(map_review_error)?;
    let snapshot_allocated_bytes = target.snapshot_allocated_bytes;
    let request = CloudEvictionProbeRequest::from_target(&target);
    let facts = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| probe(request)))
        .map_err(|_| CloudEvictionProbeError::PlatformFailed)?
        .map_err(|error| match error {
            CloudEvictionProbePlatformError::Unsupported => {
                CloudEvictionProbeError::PlatformUnsupported
            }
            CloudEvictionProbePlatformError::Failed => CloudEvictionProbeError::PlatformFailed,
        })?;
    let refreshed_target = review
        .cloud_eviction_target(node_id)
        .map_err(map_review_error)?;
    if refreshed_target.snapshot != target.snapshot
        || refreshed_target.snapshot_allocated_bytes != snapshot_allocated_bytes
    {
        return Err(CloudEvictionProbeError::ChangedSinceSnapshot);
    }

    // Join platform-only facts with core-owned provider, kind, allocation,
    // and clock before classification.
    let observation = CloudEvictionObservation::new(CloudEvictionObservationInput {
        provider: CloudEvictionProvider::ICloudDrive,
        item_kind: CloudEvictionItemKind::RegularFile,
        ubiquitous: facts.ubiquitous,
        uploaded: facts.uploaded,
        uploading: facts.uploading,
        upload_error: facts.upload_error,
        unresolved_conflicts: facts.unresolved_conflicts,
        local_copy_state: facts.local_copy_state,
        download_requested: facts.download_requested,
        downloading: facts.downloading,
        download_error: facts.download_error,
        excluded_from_sync: facts.excluded_from_sync,
        identity: facts.identity,
        local_allocated_bytes: Some(snapshot_allocated_bytes),
        observed_at: SystemTime::now(),
    });
    Ok(assess_cloud_eviction(observation))
}

fn map_review_error(error: SnapshotReviewError) -> CloudEvictionProbeError {
    match error {
        SnapshotReviewError::LivePathChanged => CloudEvictionProbeError::ChangedSinceSnapshot,
        SnapshotReviewError::NodeNotFound
        | SnapshotReviewError::NodeNotDirectory
        | SnapshotReviewError::LiveTargetUnsupported => CloudEvictionProbeError::InvalidTarget,
        _ => CloudEvictionProbeError::ReviewUnavailable,
    }
}
