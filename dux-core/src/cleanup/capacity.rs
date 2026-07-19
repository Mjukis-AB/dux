//! Bounded pre/post capacity verification for cleanup history.
//!
//! Capacity is an observation, not proof that a filesystem effect succeeded.
//! This boundary only produces a journal-safe signed delta when both samples
//! are for the same stable volume, bracket the effect, and remain close enough
//! to the effect window to be useful. Estimates are never converted into a
//! verified delta, and an unavailable verification is represented by `None`
//! at the journal boundary.

use std::time::{Duration, SystemTime};

use thiserror::Error;

use crate::domain::{AvailableCapacitySource, VolumeCapacity, VolumeId};
use crate::engine::VolumeCapacityObservation;
use crate::persistence::{CleanupJournalClaim, HistoryError};

const MAX_SAMPLE_SKEW: Duration = Duration::from_secs(15 * 60);

/// One stable-identity capacity sample admitted to cleanup verification.
///
/// The source observation has already passed `VolumeCapacity` validation; the
/// stable volume identity is required here because a mount can be replaced
/// while a cleanup is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CleanupCapacityObservation {
    volume_id: VolumeId,
    sampled_at: SystemTime,
    capacity: VolumeCapacity,
}

impl CleanupCapacityObservation {
    pub(crate) fn try_from_observation(
        observation: &VolumeCapacityObservation,
    ) -> Result<Self, CleanupCapacityVerificationError> {
        let volume_id = observation
            .volume_id()
            .cloned()
            .ok_or(CleanupCapacityVerificationError::MissingVolumeIdentity)?;
        Ok(Self {
            volume_id,
            sampled_at: observation.sampled_at(),
            capacity: observation.capacity(),
        })
    }

    #[cfg(test)]
    fn new(volume_id: VolumeId, sampled_at: SystemTime, capacity: VolumeCapacity) -> Self {
        Self {
            volume_id,
            sampled_at,
            capacity,
        }
    }

    pub(crate) fn volume_id(&self) -> &VolumeId {
        &self.volume_id
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
    volume_id: VolumeId,
    pre_sampled_at: SystemTime,
    post_sampled_at: SystemTime,
    source: AvailableCapacitySource,
    delta_bytes: i64,
}

impl VerifiedCleanupCapacity {
    pub(crate) fn volume_id(&self) -> &VolumeId {
        &self.volume_id
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
pub(crate) fn verify_cleanup_capacity(
    expected_volume_id: &VolumeId,
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
    if pre.volume_id != *expected_volume_id || post.volume_id != *expected_volume_id {
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
        volume_id: expected_volume_id.clone(),
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
}
