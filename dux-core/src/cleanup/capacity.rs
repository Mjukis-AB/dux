//! Bounded pre/post capacity verification for cleanup history.
//!
//! Capacity is an observation, not proof that a filesystem effect succeeded.
//! This boundary only produces a journal-safe signed delta when both samples
//! are for the same stable volume, bracket the effect, and remain close enough
//! to the effect window to be useful. Estimates are never converted into a
//! verified delta, and an unavailable verification is represented by `None`
//! at the journal boundary.

#![allow(
    dead_code,
    reason = "some capacity witness accessors remain private until trusted volume sampling is wired"
)]

use std::time::{Duration, SystemTime};

use thiserror::Error;

#[cfg(test)]
use crate::domain::VolumeId;
use crate::domain::{AvailableCapacitySource, VolumeCapacity};
#[cfg(test)]
use crate::engine::VolumeCapacityObservation;
use crate::path_validation::FilesystemCapacityScope;
use crate::persistence::{CleanupJournalClaim, HistoryError, TerminalSessionStatus};

const MAX_SAMPLE_SKEW: Duration = Duration::from_secs(15 * 60);

/// One stable-identity capacity sample admitted to cleanup verification.
///
/// The source observation has already passed `VolumeCapacity` validation; the
/// stable volume identity is required here because a mount can be replaced
/// while a cleanup is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CleanupCapacityObservation {
    identity: CleanupCapacityIdentity,
    sampled_at: SystemTime,
    capacity: VolumeCapacity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CleanupCapacityIdentity {
    #[cfg(test)]
    StableVolumeId(VolumeId),
    TrustedFilesystem(FilesystemCapacityScope),
}

/// Synchronous, private sampling seam used only while a cleanup journal claim
/// is held. Implementations return a bounded observation and must not retain
/// cleanup authority; a missing observation is represented by `None` and
/// prevents capacity from being reported as verified.
pub(crate) trait CleanupCapacitySampler {
    fn expected_identity(&self) -> CleanupCapacityIdentity;
    fn sample(&mut self) -> Option<CleanupCapacityObservation>;
}

impl CleanupCapacityObservation {
    #[cfg(test)]
    pub(crate) fn try_from_observation(
        observation: &VolumeCapacityObservation,
    ) -> Result<Self, CleanupCapacityVerificationError> {
        let volume_id = observation
            .volume_id()
            .cloned()
            .ok_or(CleanupCapacityVerificationError::MissingVolumeIdentity)?;
        Ok(Self {
            identity: CleanupCapacityIdentity::StableVolumeId(volume_id),
            sampled_at: observation.sampled_at(),
            capacity: observation.capacity(),
        })
    }

    #[cfg(target_os = "macos")]
    fn from_trusted_filesystem(
        scope: FilesystemCapacityScope,
        sampled_at: SystemTime,
        capacity: VolumeCapacity,
    ) -> Self {
        Self {
            identity: CleanupCapacityIdentity::TrustedFilesystem(scope),
            sampled_at,
            capacity,
        }
    }

    #[cfg(test)]
    fn new(volume_id: VolumeId, sampled_at: SystemTime, capacity: VolumeCapacity) -> Self {
        Self {
            identity: CleanupCapacityIdentity::StableVolumeId(volume_id),
            sampled_at,
            capacity,
        }
    }

    pub(crate) fn identity(&self) -> &CleanupCapacityIdentity {
        &self.identity
    }

    pub(crate) const fn sampled_at(&self) -> SystemTime {
        self.sampled_at
    }

    pub(crate) const fn capacity(&self) -> VolumeCapacity {
        self.capacity
    }
}

/// Why a cleanup capacity delta could not be verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(crate) enum CleanupCapacityVerificationError {
    #[error("a stable volume identity is required for capacity verification")]
    MissingVolumeIdentity,
    #[error("the cleanup capacity effect window is invalid")]
    InvalidEffectWindow,
    #[error("the pre-effect capacity sample is missing")]
    MissingPreSample,
    #[error("the post-effect capacity sample is missing")]
    MissingPostSample,
    #[error("the capacity sample belongs to a different volume")]
    VolumeChanged,
    #[error("the pre-effect capacity sample is outside the bounded window")]
    PreSampleOutsideWindow,
    #[error("the post-effect capacity sample is outside the bounded window")]
    PostSampleOutsideWindow,
    #[error("the total volume capacity changed during verification")]
    TotalCapacityChanged,
    #[error("the headline capacity source changed during verification")]
    AvailableSourceChanged,
    #[error("the available-capacity shape changed during verification")]
    AvailabilityShapeChanged,
    #[error("the capacity delta cannot be represented as a signed journal value")]
    DeltaOutOfRange,
}

/// A verified signed change in headline available capacity.
///
/// Positive values mean that headline available space increased. This value
/// has no cleanup authority and is intentionally the only value accepted by
/// the private journal-terminalization adapter below.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerifiedCleanupCapacity {
    identity: CleanupCapacityIdentity,
    pre_sampled_at: SystemTime,
    post_sampled_at: SystemTime,
    source: AvailableCapacitySource,
    delta_bytes: i64,
}

impl VerifiedCleanupCapacity {
    pub(crate) fn identity(&self) -> &CleanupCapacityIdentity {
        &self.identity
    }

    pub(crate) const fn pre_sampled_at(&self) -> SystemTime {
        self.pre_sampled_at
    }

    pub(crate) const fn post_sampled_at(&self) -> SystemTime {
        self.post_sampled_at
    }

    pub(crate) const fn source(&self) -> AvailableCapacitySource {
        self.source
    }

    pub(crate) const fn delta_bytes(&self) -> i64 {
        self.delta_bytes
    }
}

/// Verify a bounded pre/post capacity pair around one cleanup effect.
///
/// The pre sample must be no older than [`MAX_SAMPLE_SKEW`] at effect start;
/// the post sample must follow effect completion and arrive within the same
/// bound. Stable volume identity, total capacity, headline source, and
/// availability shape must remain equal. Any mismatch is a refusal to report
/// a verified delta, never a zero or estimated value.
#[cfg(test)]
pub(crate) fn verify_cleanup_capacity(
    expected_volume_id: &VolumeId,
    pre: Option<&CleanupCapacityObservation>,
    post: Option<&CleanupCapacityObservation>,
    effect_started_at: SystemTime,
    effect_completed_at: SystemTime,
) -> Result<VerifiedCleanupCapacity, CleanupCapacityVerificationError> {
    verify_cleanup_capacity_identity(
        &CleanupCapacityIdentity::StableVolumeId(expected_volume_id.clone()),
        pre,
        post,
        effect_started_at,
        effect_completed_at,
    )
}

pub(crate) fn verify_cleanup_capacity_identity(
    expected_identity: &CleanupCapacityIdentity,
    pre: Option<&CleanupCapacityObservation>,
    post: Option<&CleanupCapacityObservation>,
    effect_started_at: SystemTime,
    effect_completed_at: SystemTime,
) -> Result<VerifiedCleanupCapacity, CleanupCapacityVerificationError> {
    if effect_started_at > effect_completed_at {
        return Err(CleanupCapacityVerificationError::InvalidEffectWindow);
    }
    let pre = pre.ok_or(CleanupCapacityVerificationError::MissingPreSample)?;
    let post = post.ok_or(CleanupCapacityVerificationError::MissingPostSample)?;
    if pre.identity != *expected_identity || post.identity != *expected_identity {
        return Err(CleanupCapacityVerificationError::VolumeChanged);
    }

    let pre_age = effect_started_at
        .duration_since(pre.sampled_at)
        .map_err(|_| CleanupCapacityVerificationError::PreSampleOutsideWindow)?;
    if pre_age > MAX_SAMPLE_SKEW {
        return Err(CleanupCapacityVerificationError::PreSampleOutsideWindow);
    }
    let post_age = post
        .sampled_at
        .duration_since(effect_completed_at)
        .map_err(|_| CleanupCapacityVerificationError::PostSampleOutsideWindow)?;
    if post_age > MAX_SAMPLE_SKEW {
        return Err(CleanupCapacityVerificationError::PostSampleOutsideWindow);
    }

    let pre_capacity = pre.capacity;
    let post_capacity = post.capacity;
    if pre_capacity.total_bytes() != post_capacity.total_bytes() {
        return Err(CleanupCapacityVerificationError::TotalCapacityChanged);
    }
    if pre_capacity.headline_source() != post_capacity.headline_source() {
        return Err(CleanupCapacityVerificationError::AvailableSourceChanged);
    }
    if pre_capacity.available_bytes().is_some() != post_capacity.available_bytes().is_some()
        || pre_capacity.important_available_bytes().is_some()
            != post_capacity.important_available_bytes().is_some()
    {
        return Err(CleanupCapacityVerificationError::AvailabilityShapeChanged);
    }

    let pre_available = pre_capacity.headline_available_bytes();
    let post_available = post_capacity.headline_available_bytes();
    let delta_bytes = signed_delta(post_available, pre_available)
        .ok_or(CleanupCapacityVerificationError::DeltaOutOfRange)?;
    Ok(VerifiedCleanupCapacity {
        identity: expected_identity.clone(),
        pre_sampled_at: pre.sampled_at,
        post_sampled_at: post.sampled_at,
        source: pre_capacity.headline_source(),
        delta_bytes,
    })
}

fn signed_delta(after: u64, before: u64) -> Option<i64> {
    if after >= before {
        i64::try_from(after - before).ok()
    } else {
        i64::try_from(before - after).ok().map(i64::wrapping_neg)
    }
}

/// Core-owned macOS sampler bound to one exact trusted plan volume.
///
/// The mount path and kernel filesystem identity originate from the retained
/// rule-scope authorization; no caller can substitute a path, volume label, or
/// capacity value. Sampling failure only makes verification unknown.
#[cfg(target_os = "macos")]
pub(crate) struct MacOSCleanupCapacitySampler {
    scope: FilesystemCapacityScope,
}

#[cfg(target_os = "macos")]
impl MacOSCleanupCapacitySampler {
    pub(crate) fn new(scope: FilesystemCapacityScope) -> Option<Self> {
        if scope.filesystem_id() == [0, 0]
            || scope.mount_id() != 0
            || scope.filesystem_type() == 0
            || scope.mount_path().is_none_or(|path| !path.is_absolute())
        {
            return None;
        }
        Some(Self { scope })
    }

    fn sample_bound_volume(&self) -> Option<CleanupCapacityObservation> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let mount_path = self.scope.mount_path()?;
        let path = CString::new(mount_path.as_os_str().as_bytes()).ok()?;
        let mut stats = std::mem::MaybeUninit::<nix::libc::statfs>::uninit();
        // SAFETY: `path` is a NUL-terminated copy of the trusted mount path
        // and `stats` points to writable storage for the duration of statfs.
        if unsafe { nix::libc::statfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: statfs returned success above.
        let stats = unsafe { stats.assume_init() };
        if macos_fsid_values(stats.f_fsid)? != self.scope.filesystem_id()
            || stats.f_type as u64 != self.scope.filesystem_type()
            || macos_mount_path(&stats)? != mount_path
        {
            return None;
        }
        let block_size = u64::from(stats.f_bsize);
        let total_bytes = stats.f_blocks.checked_mul(block_size)?;
        let available_bytes = stats.f_bavail.checked_mul(block_size)?;
        let capacity = VolumeCapacity::new(total_bytes, Some(available_bytes), None).ok()?;
        Some(CleanupCapacityObservation::from_trusted_filesystem(
            self.scope.clone(),
            SystemTime::now(),
            capacity,
        ))
    }
}

#[cfg(target_os = "macos")]
impl CleanupCapacitySampler for MacOSCleanupCapacitySampler {
    fn expected_identity(&self) -> CleanupCapacityIdentity {
        CleanupCapacityIdentity::TrustedFilesystem(self.scope.clone())
    }

    fn sample(&mut self) -> Option<CleanupCapacityObservation> {
        self.sample_bound_volume()
    }
}

#[cfg(target_os = "macos")]
fn macos_fsid_values(value: nix::libc::fsid_t) -> Option<[u64; 2]> {
    // Darwin's fsid is two native-endian 32-bit words whose Rust fields are
    // private. Copy the fixed ABI bytes without naming those fields.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&value as *const nix::libc::fsid_t).cast::<u8>(),
            std::mem::size_of::<nix::libc::fsid_t>(),
        )
    };
    (bytes.len() >= 8).then(|| {
        [
            u32::from_ne_bytes(bytes[0..4].try_into().expect("length checked")) as u64,
            u32::from_ne_bytes(bytes[4..8].try_into().expect("length checked")) as u64,
        ]
    })
}

#[cfg(target_os = "macos")]
fn macos_mount_path(stats: &nix::libc::statfs) -> Option<&std::path::Path> {
    use std::os::unix::ffi::OsStrExt;

    let bytes = unsafe {
        std::slice::from_raw_parts(
            stats.f_mntonname.as_ptr().cast::<u8>(),
            stats.f_mntonname.len(),
        )
    };
    let end = bytes.iter().position(|byte| *byte == 0)?;
    Some(std::path::Path::new(std::ffi::OsStr::from_bytes(
        &bytes[..end],
    )))
}

/// Terminalize a cleanup session with only a verified capacity delta.
///
/// `None` is deliberately allowed for unknown verification, while callers
/// cannot pass an arbitrary signed integer through this adapter.
pub(crate) fn terminalize_with_capacity(
    claim: &mut CleanupJournalClaim,
    completed_at: SystemTime,
    verification: Option<&VerifiedCleanupCapacity>,
) -> Result<(), HistoryError> {
    claim.terminalize_for_capacity_verification(
        completed_at,
        verification.map(VerifiedCleanupCapacity::delta_bytes),
    )
}

/// Terminalize through the same capability-restricted adapter while retaining
/// the journal's bounded terminal status for the private session orchestrator.
pub(crate) fn terminalize_with_capacity_with_status(
    claim: &mut CleanupJournalClaim,
    completed_at: SystemTime,
    verification: Option<&VerifiedCleanupCapacity>,
) -> Result<TerminalSessionStatus, HistoryError> {
    claim.terminalize_for_capacity_verification_with_status(
        completed_at,
        verification.map(VerifiedCleanupCapacity::delta_bytes),
    )
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn volume_id() -> VolumeId {
        VolumeId::new("volume:internal").unwrap()
    }

    fn sample(
        volume_id: VolumeId,
        sampled_at_ms: u64,
        total: u64,
        available: Option<u64>,
        important: Option<u64>,
    ) -> CleanupCapacityObservation {
        CleanupCapacityObservation::new(
            volume_id,
            UNIX_EPOCH + Duration::from_millis(sampled_at_ms),
            VolumeCapacity::new(total, available, important).unwrap(),
        )
    }

    #[test]
    fn verifies_positive_negative_and_zero_headline_deltas() {
        let id = volume_id();
        let pre = sample(id.clone(), 1_000, 1_000, Some(400), None);
        let post = sample(id.clone(), 3_000, 1_000, Some(550), None);
        let verified = verify_cleanup_capacity(
            &id,
            Some(&pre),
            Some(&post),
            UNIX_EPOCH + Duration::from_millis(1_500),
            UNIX_EPOCH + Duration::from_millis(2_000),
        )
        .unwrap();
        assert_eq!(verified.delta_bytes(), 150);

        let post = sample(id.clone(), 3_000, 1_000, Some(250), None);
        let verified = verify_cleanup_capacity(
            &id,
            Some(&pre),
            Some(&post),
            UNIX_EPOCH + Duration::from_millis(1_500),
            UNIX_EPOCH + Duration::from_millis(2_000),
        )
        .unwrap();
        assert_eq!(verified.delta_bytes(), -150);

        let post = sample(id.clone(), 3_000, 1_000, Some(400), None);
        let verified = verify_cleanup_capacity(
            &id,
            Some(&pre),
            Some(&post),
            UNIX_EPOCH + Duration::from_millis(1_500),
            UNIX_EPOCH + Duration::from_millis(2_000),
        )
        .unwrap();
        assert_eq!(verified.delta_bytes(), 0);
    }

    #[test]
    fn important_usage_source_is_verified_without_mixing_ordinary_free_space() {
        let id = volume_id();
        let pre = sample(id.clone(), 1_000, 1_000, Some(400), Some(300));
        let post = sample(id.clone(), 3_000, 1_000, Some(700), Some(450));
        let verified = verify_cleanup_capacity(
            &id,
            Some(&pre),
            Some(&post),
            UNIX_EPOCH + Duration::from_millis(1_500),
            UNIX_EPOCH + Duration::from_millis(2_000),
        )
        .unwrap();
        assert_eq!(verified.delta_bytes(), 150);
        assert_eq!(verified.source(), AvailableCapacitySource::ImportantUsage);
    }

    #[test]
    fn refuses_missing_or_mismatched_evidence() {
        let id = volume_id();
        let pre = sample(id.clone(), 1_000, 1_000, Some(400), None);
        let post = sample(id.clone(), 3_000, 1_001, Some(550), None);
        let window_start = UNIX_EPOCH + Duration::from_millis(1_500);
        let window_end = UNIX_EPOCH + Duration::from_millis(2_000);
        for (pre_value, post_value, expected) in [
            (
                None,
                Some(&post),
                CleanupCapacityVerificationError::MissingPreSample,
            ),
            (
                Some(&pre),
                None,
                CleanupCapacityVerificationError::MissingPostSample,
            ),
            (
                Some(&pre),
                Some(&post),
                CleanupCapacityVerificationError::TotalCapacityChanged,
            ),
        ] {
            assert_eq!(
                verify_cleanup_capacity(&id, pre_value, post_value, window_start, window_end)
                    .unwrap_err(),
                expected
            );
        }

        let other = VolumeId::new("volume:external").unwrap();
        let post = sample(other, 3_000, 1_000, Some(550), None);
        assert_eq!(
            verify_cleanup_capacity(&id, Some(&pre), Some(&post), window_start, window_end)
                .unwrap_err(),
            CleanupCapacityVerificationError::VolumeChanged
        );
    }

    #[test]
    fn refuses_source_shape_and_window_mismatches() {
        let id = volume_id();
        let pre = sample(id.clone(), 1_000, 1_000, Some(400), None);
        let important_post = sample(id.clone(), 3_000, 1_000, Some(550), Some(500));
        let start = UNIX_EPOCH + Duration::from_millis(1_500);
        let end = UNIX_EPOCH + Duration::from_millis(2_000);
        assert_eq!(
            verify_cleanup_capacity(&id, Some(&pre), Some(&important_post), start, end)
                .unwrap_err(),
            CleanupCapacityVerificationError::AvailableSourceChanged
        );

        let late_pre = sample(id.clone(), 0, 1_000, Some(400), None);
        let old_start = UNIX_EPOCH + Duration::from_secs(16 * 60);
        let old_end = old_start + Duration::from_millis(500);
        let ordinary_post = sample(id.clone(), 16 * 60 * 1_000 + 1_000, 1_000, Some(550), None);
        assert_eq!(
            verify_cleanup_capacity(
                &id,
                Some(&late_pre),
                Some(&ordinary_post),
                old_start,
                old_end
            )
            .unwrap_err(),
            CleanupCapacityVerificationError::PreSampleOutsideWindow
        );

        let late_post = sample(id.clone(), 1_000_000, 1_000, Some(550), None);
        assert_eq!(
            verify_cleanup_capacity(&id, Some(&pre), Some(&late_post), start, end).unwrap_err(),
            CleanupCapacityVerificationError::PostSampleOutsideWindow
        );
        assert_eq!(
            verify_cleanup_capacity(&id, Some(&pre), Some(&pre), end, start,).unwrap_err(),
            CleanupCapacityVerificationError::InvalidEffectWindow
        );
    }

    #[test]
    fn clamps_signed_delta_to_journal_representation() {
        let id = volume_id();
        let pre = sample(id.clone(), 1_000, u64::MAX, Some(u64::MAX), None);
        let post = sample(id.clone(), 3_000, u64::MAX, Some(0), None);
        assert_eq!(
            verify_cleanup_capacity(
                &id,
                Some(&pre),
                Some(&post),
                UNIX_EPOCH + Duration::from_millis(1_500),
                UNIX_EPOCH + Duration::from_millis(2_000),
            )
            .unwrap_err(),
            CleanupCapacityVerificationError::DeltaOutOfRange
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_sampler_accepts_only_the_exact_kernel_volume_scope() {
        use crate::path_validation::{
            capture_filesystem_boundary, capture_scan_root, validate_scan_root,
        };

        let temp = tempfile::Builder::new()
            .prefix("dux-capacity-scope-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        let root_path = std::fs::canonicalize(temp.path()).unwrap();
        let root = capture_scan_root(validate_scan_root(&root_path).unwrap()).unwrap();
        let scope = capture_filesystem_boundary(&root)
            .unwrap()
            .capacity_scope()
            .unwrap();

        let mut sampler = MacOSCleanupCapacitySampler::new(scope.clone()).unwrap();
        assert_eq!(
            sampler.expected_identity(),
            CleanupCapacityIdentity::TrustedFilesystem(scope.clone())
        );
        let observation = sampler
            .sample()
            .expect("exact mounted volume should sample");
        assert_eq!(
            observation.identity(),
            &CleanupCapacityIdentity::TrustedFilesystem(scope.clone())
        );
        assert!(observation.capacity().total_bytes() > 0);

        let mut wrong_id = scope.filesystem_id();
        wrong_id[0] ^= 1;
        let forged = scope.with_filesystem_id_for_test(wrong_id);
        let mut forged_sampler = MacOSCleanupCapacitySampler::new(forged).unwrap();
        assert!(
            forged_sampler.sample().is_none(),
            "a self-consistent caller label cannot replace the kernel fsid"
        );
    }
}
