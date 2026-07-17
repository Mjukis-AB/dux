//! Deterministic volume-capacity and disk-pressure policy.
//!
//! Capacity is observed outside the core. This module validates those raw
//! observations, selects the headline value, and applies configurable entry
//! thresholds and exit hysteresis without floating-point arithmetic.

use thiserror::Error;

const BASIS_POINTS_PER_WHOLE: u16 = 10_000;
const GIB: u64 = 1_024 * 1_024 * 1_024;

/// Observed disk-pressure classification for a volume.
///
/// This is presentation and scheduling policy, not cleanup authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiskPressure {
    Healthy,
    Warning,
    Critical,
    Unknown,
}

/// Which validated capacity observation supplies the user-facing free-space
/// value and pressure input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AvailableCapacitySource {
    /// Capacity available for important usage, when the platform supplies it.
    ImportantUsage,
    /// Ordinary filesystem availability used as the portable fallback.
    Ordinary,
}

/// Invalid raw capacity values for one volume observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum VolumeCapacityError {
    #[error("volume total capacity must be positive")]
    TotalBytesZero,
    #[error("ordinary available capacity exceeds total capacity")]
    AvailableBytesExceedTotal,
    #[error("important-usage available capacity exceeds total capacity")]
    ImportantAvailableBytesExceedTotal,
    #[error("at least one available-capacity observation is required")]
    AvailableCapacityMissing,
}

/// One validated capacity observation.
///
/// Scanned node totals must never be used to construct this value. An
/// important-only value is valid for ephemeral status, but durable history
/// deliberately requires ordinary filesystem availability as well.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VolumeCapacity {
    total_bytes: u64,
    available_bytes: Option<u64>,
    important_available_bytes: Option<u64>,
}

impl VolumeCapacity {
    /// Validates a raw capacity observation.
    pub fn new(
        total_bytes: u64,
        available_bytes: Option<u64>,
        important_available_bytes: Option<u64>,
    ) -> Result<Self, VolumeCapacityError> {
        if total_bytes == 0 {
            return Err(VolumeCapacityError::TotalBytesZero);
        }
        if available_bytes.is_some_and(|value| value > total_bytes) {
            return Err(VolumeCapacityError::AvailableBytesExceedTotal);
        }
        if important_available_bytes.is_some_and(|value| value > total_bytes) {
            return Err(VolumeCapacityError::ImportantAvailableBytesExceedTotal);
        }
        if available_bytes.is_none() && important_available_bytes.is_none() {
            return Err(VolumeCapacityError::AvailableCapacityMissing);
        }

        Ok(Self {
            total_bytes,
            available_bytes,
            important_available_bytes,
        })
    }

    pub const fn total_bytes(self) -> u64 {
        self.total_bytes
    }

    /// Ordinary filesystem availability, retained for honest usage
    /// accounting even when it is not the headline value.
    pub const fn available_bytes(self) -> Option<u64> {
        self.available_bytes
    }

    pub const fn important_available_bytes(self) -> Option<u64> {
        self.important_available_bytes
    }

    /// The capacity value used for the pressure label and headline display.
    pub fn headline_available_bytes(self) -> u64 {
        self.important_available_bytes
            .or(self.available_bytes)
            .expect("validated capacity always has one available value")
    }

    pub const fn headline_source(self) -> AvailableCapacitySource {
        if self.important_available_bytes.is_some() {
            AvailableCapacitySource::ImportantUsage
        } else {
            AvailableCapacitySource::Ordinary
        }
    }
}

/// One pressure-entry threshold.
///
/// Its effective byte boundary is the smaller of `maximum_available_bytes`
/// and `maximum_available_basis_points` of the observed total capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiskPressureThreshold {
    maximum_available_bytes: u64,
    maximum_available_basis_points: u16,
}

impl DiskPressureThreshold {
    pub fn new(
        maximum_available_bytes: u64,
        maximum_available_basis_points: u16,
    ) -> Result<Self, DiskPressureConfigError> {
        if maximum_available_bytes == 0 {
            return Err(DiskPressureConfigError::ThresholdBytesZero);
        }
        if !(1..=BASIS_POINTS_PER_WHOLE).contains(&maximum_available_basis_points) {
            return Err(DiskPressureConfigError::ThresholdBasisPointsOutOfRange);
        }
        Ok(Self {
            maximum_available_bytes,
            maximum_available_basis_points,
        })
    }

    pub const fn maximum_available_bytes(self) -> u64 {
        self.maximum_available_bytes
    }

    /// Maximum available percentage in basis points, where 100 basis points
    /// equal one percentage point.
    pub const fn maximum_available_basis_points(self) -> u16 {
        self.maximum_available_basis_points
    }

    fn effective_boundary_bytes(self, total_bytes: u64) -> u64 {
        self.maximum_available_bytes.min(bytes_at_basis_points(
            total_bytes,
            self.maximum_available_basis_points,
        ))
    }
}

/// Extra capacity required above an entry boundary before pressure can
/// de-escalate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiskPressureRecoveryMargin {
    bytes: u64,
    basis_points: u16,
}

impl DiskPressureRecoveryMargin {
    pub fn new(bytes: u64, basis_points: u16) -> Result<Self, DiskPressureConfigError> {
        if bytes == 0 {
            return Err(DiskPressureConfigError::RecoveryBytesZero);
        }
        if !(1..=BASIS_POINTS_PER_WHOLE).contains(&basis_points) {
            return Err(DiskPressureConfigError::RecoveryBasisPointsOutOfRange);
        }
        Ok(Self {
            bytes,
            basis_points,
        })
    }

    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    /// Recovery percentage in basis points, where 100 basis points equal one
    /// percentage point.
    pub const fn basis_points(self) -> u16 {
        self.basis_points
    }
}

/// Invalid configurable pressure policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum DiskPressureConfigError {
    #[error("a pressure threshold byte value must be positive")]
    ThresholdBytesZero,
    #[error("pressure threshold basis points must be between 1 and 10000")]
    ThresholdBasisPointsOutOfRange,
    #[error("the warning byte threshold must not be below the critical threshold")]
    WarningBytesBelowCritical,
    #[error("the warning percentage threshold must not be below the critical threshold")]
    WarningBasisPointsBelowCritical,
    #[error("warning and critical thresholds must not be identical")]
    WarningThresholdMatchesCritical,
    #[error("the pressure recovery byte margin must be positive")]
    RecoveryBytesZero,
    #[error("pressure recovery basis points must be between 1 and 10000")]
    RecoveryBasisPointsOutOfRange,
}

/// User-configurable deterministic pressure thresholds and hysteresis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiskPressureConfig {
    critical: DiskPressureThreshold,
    warning: DiskPressureThreshold,
    recovery: DiskPressureRecoveryMargin,
}

impl DiskPressureConfig {
    /// Default startup-volume policy: Critical at `min(10 GiB, 5%)`, Warning
    /// at `min(30 GiB, 10%)`, and recovery only after clearing a boundary by
    /// both 2 GiB and one percentage point.
    pub const DEFAULT: Self = Self {
        critical: DiskPressureThreshold {
            maximum_available_bytes: 10 * GIB,
            maximum_available_basis_points: 500,
        },
        warning: DiskPressureThreshold {
            maximum_available_bytes: 30 * GIB,
            maximum_available_basis_points: 1_000,
        },
        recovery: DiskPressureRecoveryMargin {
            bytes: 2 * GIB,
            basis_points: 100,
        },
    };

    pub fn new(
        critical: DiskPressureThreshold,
        warning: DiskPressureThreshold,
        recovery: DiskPressureRecoveryMargin,
    ) -> Result<Self, DiskPressureConfigError> {
        if warning.maximum_available_bytes < critical.maximum_available_bytes {
            return Err(DiskPressureConfigError::WarningBytesBelowCritical);
        }
        if warning.maximum_available_basis_points < critical.maximum_available_basis_points {
            return Err(DiskPressureConfigError::WarningBasisPointsBelowCritical);
        }
        if warning == critical {
            return Err(DiskPressureConfigError::WarningThresholdMatchesCritical);
        }
        Ok(Self {
            critical,
            warning,
            recovery,
        })
    }

    pub const fn critical_threshold(self) -> DiskPressureThreshold {
        self.critical
    }

    pub const fn warning_threshold(self) -> DiskPressureThreshold {
        self.warning
    }

    pub const fn recovery_margin(self) -> DiskPressureRecoveryMargin {
        self.recovery
    }

    /// Evaluates one capacity observation against a prior pressure state.
    ///
    /// Escalation is immediate. `Unknown` initializes directly from the
    /// measurement. De-escalation requires clearing the effective boundary by
    /// both configured recovery margins. The result never grants cleanup
    /// authority.
    pub fn evaluate(
        self,
        capacity: VolumeCapacity,
        previous: DiskPressure,
    ) -> DiskPressureEvaluation {
        let available_bytes = capacity.headline_available_bytes();
        let total_bytes = capacity.total_bytes();
        let critical_boundary_bytes = self.critical.effective_boundary_bytes(total_bytes);
        let warning_boundary_bytes = self.warning.effective_boundary_bytes(total_bytes);

        let pressure = match previous {
            DiskPressure::Unknown | DiskPressure::Healthy => classify_without_hysteresis(
                available_bytes,
                critical_boundary_bytes,
                warning_boundary_bytes,
            ),
            DiskPressure::Warning => {
                if available_bytes <= critical_boundary_bytes {
                    DiskPressure::Critical
                } else if clears_recovery_margin(
                    available_bytes,
                    warning_boundary_bytes,
                    total_bytes,
                    self.recovery,
                ) {
                    DiskPressure::Healthy
                } else {
                    DiskPressure::Warning
                }
            }
            DiskPressure::Critical => {
                if !clears_recovery_margin(
                    available_bytes,
                    critical_boundary_bytes,
                    total_bytes,
                    self.recovery,
                ) {
                    DiskPressure::Critical
                } else if clears_recovery_margin(
                    available_bytes,
                    warning_boundary_bytes,
                    total_bytes,
                    self.recovery,
                ) {
                    DiskPressure::Healthy
                } else {
                    DiskPressure::Warning
                }
            }
        };

        DiskPressureEvaluation {
            pressure,
            headline_available_bytes: available_bytes,
            headline_source: capacity.headline_source(),
            critical_boundary_bytes,
            warning_boundary_bytes,
        }
    }
}

impl Default for DiskPressureConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Deterministic result plus the effective entry boundaries used to derive it.
///
/// Exposing the boundaries lets clients explain the label without duplicating
/// policy or percentage arithmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiskPressureEvaluation {
    pressure: DiskPressure,
    headline_available_bytes: u64,
    headline_source: AvailableCapacitySource,
    critical_boundary_bytes: u64,
    warning_boundary_bytes: u64,
}

impl DiskPressureEvaluation {
    pub const fn pressure(self) -> DiskPressure {
        self.pressure
    }

    pub const fn headline_available_bytes(self) -> u64 {
        self.headline_available_bytes
    }

    pub const fn headline_source(self) -> AvailableCapacitySource {
        self.headline_source
    }

    pub const fn critical_boundary_bytes(self) -> u64 {
        self.critical_boundary_bytes
    }

    pub const fn warning_boundary_bytes(self) -> u64 {
        self.warning_boundary_bytes
    }
}

fn classify_without_hysteresis(
    available_bytes: u64,
    critical_boundary_bytes: u64,
    warning_boundary_bytes: u64,
) -> DiskPressure {
    if available_bytes <= critical_boundary_bytes {
        DiskPressure::Critical
    } else if available_bytes <= warning_boundary_bytes {
        DiskPressure::Warning
    } else {
        DiskPressure::Healthy
    }
}

fn clears_recovery_margin(
    available_bytes: u64,
    boundary_bytes: u64,
    total_bytes: u64,
    recovery: DiskPressureRecoveryMargin,
) -> bool {
    let Some(bytes_above_boundary) = available_bytes.checked_sub(boundary_bytes) else {
        return false;
    };
    if bytes_above_boundary < recovery.bytes {
        return false;
    }

    ratio_is_at_least(bytes_above_boundary, total_bytes, recovery.basis_points)
}

fn ratio_is_at_least(part: u64, whole: u64, basis_points: u16) -> bool {
    let Some(scaled_part) = u128::from(part).checked_mul(u128::from(BASIS_POINTS_PER_WHOLE)) else {
        return false;
    };
    let Some(scaled_required) = u128::from(whole).checked_mul(u128::from(basis_points)) else {
        return false;
    };
    scaled_part >= scaled_required
}

fn bytes_at_basis_points(total_bytes: u64, basis_points: u16) -> u64 {
    let scaled = u128::from(total_bytes)
        .checked_mul(u128::from(basis_points))
        .expect("a u64 capacity times a u16 percentage fits in u128");
    u64::try_from(scaled / u128::from(BASIS_POINTS_PER_WHOLE))
        .expect("a percentage no greater than 100% fits in u64")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIB: u64 = 1_024 * GIB;

    fn capacity(total_bytes: u64, headline_available_bytes: u64) -> VolumeCapacity {
        VolumeCapacity::new(total_bytes, Some(headline_available_bytes), None).unwrap()
    }

    fn evaluate(
        config: DiskPressureConfig,
        total_bytes: u64,
        available_bytes: u64,
        previous: DiskPressure,
    ) -> DiskPressure {
        config
            .evaluate(capacity(total_bytes, available_bytes), previous)
            .pressure()
    }

    fn custom_config(
        critical_bytes: u64,
        critical_basis_points: u16,
        warning_bytes: u64,
        warning_basis_points: u16,
        recovery_bytes: u64,
        recovery_basis_points: u16,
    ) -> DiskPressureConfig {
        DiskPressureConfig::new(
            DiskPressureThreshold::new(critical_bytes, critical_basis_points).unwrap(),
            DiskPressureThreshold::new(warning_bytes, warning_basis_points).unwrap(),
            DiskPressureRecoveryMargin::new(recovery_bytes, recovery_basis_points).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn validates_capacity_and_preserves_both_observations() {
        let value = VolumeCapacity::new(100, Some(40), Some(30)).unwrap();
        assert_eq!(value.total_bytes(), 100);
        assert_eq!(value.available_bytes(), Some(40));
        assert_eq!(value.important_available_bytes(), Some(30));

        assert_eq!(
            VolumeCapacity::new(0, Some(0), None),
            Err(VolumeCapacityError::TotalBytesZero)
        );
        assert_eq!(
            VolumeCapacity::new(100, Some(101), Some(20)),
            Err(VolumeCapacityError::AvailableBytesExceedTotal)
        );
        assert_eq!(
            VolumeCapacity::new(100, Some(20), Some(101)),
            Err(VolumeCapacityError::ImportantAvailableBytesExceedTotal)
        );
        assert_eq!(
            VolumeCapacity::new(100, None, None),
            Err(VolumeCapacityError::AvailableCapacityMissing)
        );
    }

    #[test]
    fn important_usage_is_preferred_and_ordinary_is_the_explicit_fallback() {
        let important = VolumeCapacity::new(100, Some(70), Some(5)).unwrap();
        let result = DiskPressureConfig::default().evaluate(important, DiskPressure::Unknown);
        assert_eq!(result.headline_available_bytes(), 5);
        assert_eq!(
            result.headline_source(),
            AvailableCapacitySource::ImportantUsage
        );
        assert_eq!(result.pressure(), DiskPressure::Critical);
        assert_eq!(important.available_bytes(), Some(70));

        let ordinary = VolumeCapacity::new(100, Some(70), None).unwrap();
        let result = DiskPressureConfig::default().evaluate(ordinary, DiskPressure::Unknown);
        assert_eq!(result.headline_available_bytes(), 70);
        assert_eq!(result.headline_source(), AvailableCapacitySource::Ordinary);
        assert_eq!(result.pressure(), DiskPressure::Healthy);

        let important_only = VolumeCapacity::new(100, None, Some(5)).unwrap();
        let result = DiskPressureConfig::default().evaluate(important_only, DiskPressure::Unknown);
        assert_eq!(result.headline_available_bytes(), 5);
        assert_eq!(
            result.headline_source(),
            AvailableCapacitySource::ImportantUsage
        );
        assert_eq!(result.pressure(), DiskPressure::Critical);
    }

    #[test]
    fn default_configuration_is_exact_and_accessible() {
        let config = DiskPressureConfig::default();
        assert_eq!(config, DiskPressureConfig::DEFAULT);
        assert_eq!(
            config.critical_threshold(),
            DiskPressureThreshold::new(10 * GIB, 500).unwrap()
        );
        assert_eq!(
            config.warning_threshold(),
            DiskPressureThreshold::new(30 * GIB, 1_000).unwrap()
        );
        assert_eq!(
            config.recovery_margin(),
            DiskPressureRecoveryMargin::new(2 * GIB, 100).unwrap()
        );
    }

    #[test]
    fn rejects_every_invalid_threshold_and_recovery_component() {
        assert_eq!(
            DiskPressureThreshold::new(0, 500),
            Err(DiskPressureConfigError::ThresholdBytesZero)
        );
        for value in [0, 10_001, u16::MAX] {
            assert_eq!(
                DiskPressureThreshold::new(1, value),
                Err(DiskPressureConfigError::ThresholdBasisPointsOutOfRange)
            );
        }
        assert_eq!(
            DiskPressureRecoveryMargin::new(0, 100),
            Err(DiskPressureConfigError::RecoveryBytesZero)
        );
        for value in [0, 10_001, u16::MAX] {
            assert_eq!(
                DiskPressureRecoveryMargin::new(1, value),
                Err(DiskPressureConfigError::RecoveryBasisPointsOutOfRange)
            );
        }
    }

    #[test]
    fn rejects_inverted_or_indistinguishable_level_configuration() {
        let critical = DiskPressureThreshold::new(20, 500).unwrap();
        let warning = DiskPressureThreshold::new(10, 1_000).unwrap();
        let recovery = DiskPressureRecoveryMargin::new(1, 100).unwrap();
        assert_eq!(
            DiskPressureConfig::new(critical, warning, recovery),
            Err(DiskPressureConfigError::WarningBytesBelowCritical)
        );

        let warning = DiskPressureThreshold::new(30, 400).unwrap();
        assert_eq!(
            DiskPressureConfig::new(critical, warning, recovery),
            Err(DiskPressureConfigError::WarningBasisPointsBelowCritical)
        );

        assert_eq!(
            DiskPressureConfig::new(critical, critical, recovery),
            Err(DiskPressureConfigError::WarningThresholdMatchesCritical)
        );
    }

    #[test]
    fn default_uses_the_smaller_byte_or_percentage_boundary() {
        let config = DiskPressureConfig::default();

        let large = config.evaluate(capacity(4 * TIB, 1), DiskPressure::Unknown);
        assert_eq!(large.critical_boundary_bytes(), 10 * GIB);
        assert_eq!(large.warning_boundary_bytes(), 30 * GIB);

        let small = config.evaluate(capacity(64 * GIB, 1), DiskPressure::Unknown);
        assert_eq!(small.critical_boundary_bytes(), (64 * GIB) / 20);
        assert_eq!(small.warning_boundary_bytes(), (64 * GIB) / 10);
    }

    #[test]
    fn entry_boundaries_are_inclusive_and_one_byte_above_advances() {
        let config = DiskPressureConfig::default();
        let total = 4 * TIB;

        assert_eq!(
            evaluate(config, total, 10 * GIB, DiskPressure::Unknown),
            DiskPressure::Critical
        );
        assert_eq!(
            evaluate(config, total, 10 * GIB + 1, DiskPressure::Unknown),
            DiskPressure::Warning
        );
        assert_eq!(
            evaluate(config, total, 30 * GIB, DiskPressure::Unknown),
            DiskPressure::Warning
        );
        assert_eq!(
            evaluate(config, total, 30 * GIB + 1, DiskPressure::Unknown),
            DiskPressure::Healthy
        );
    }

    #[test]
    fn percentage_boundary_rounding_is_exact_for_integer_bytes() {
        let config = custom_config(1_000, 333, 2_000, 667, 1, 1);
        let total = 101;
        let result = config.evaluate(capacity(total, 3), DiskPressure::Unknown);
        assert_eq!(result.critical_boundary_bytes(), 3);
        assert_eq!(result.warning_boundary_bytes(), 6);
        assert_eq!(result.pressure(), DiskPressure::Critical);
        assert_eq!(
            evaluate(config, total, 4, DiskPressure::Unknown),
            DiskPressure::Warning
        );
        assert_eq!(
            evaluate(config, total, 7, DiskPressure::Unknown),
            DiskPressure::Healthy
        );
    }

    #[test]
    fn unknown_initializes_from_each_measured_level_without_hysteresis() {
        let config = DiskPressureConfig::default();
        let total = 4 * TIB;
        for (available, expected) in [
            (5 * GIB, DiskPressure::Critical),
            (20 * GIB, DiskPressure::Warning),
            (40 * GIB, DiskPressure::Healthy),
        ] {
            assert_eq!(
                evaluate(config, total, available, DiskPressure::Unknown),
                expected
            );
        }
    }

    #[test]
    fn escalation_is_immediate_even_across_a_level() {
        let config = DiskPressureConfig::default();
        let total = 4 * TIB;
        assert_eq!(
            evaluate(config, total, 5 * GIB, DiskPressure::Healthy),
            DiskPressure::Critical
        );
        assert_eq!(
            evaluate(config, total, 5 * GIB, DiskPressure::Warning),
            DiskPressure::Critical
        );
        assert_eq!(
            evaluate(config, total, 20 * GIB, DiskPressure::Healthy),
            DiskPressure::Warning
        );
    }

    #[test]
    fn healthy_does_not_apply_exit_hysteresis_to_a_fresh_measurement() {
        let config = DiskPressureConfig::default();
        let total = 4 * TIB;
        assert_eq!(
            evaluate(config, total, 30 * GIB + 1, DiskPressure::Healthy),
            DiskPressure::Healthy
        );
    }

    #[test]
    fn warning_recovery_requires_byte_and_percentage_margins() {
        let config = DiskPressureConfig::default();

        // On a large volume, the percentage-point margin is larger.
        let large_total = TIB;
        let boundary = 30 * GIB;
        assert_eq!(
            evaluate(
                config,
                large_total,
                boundary + 2 * GIB,
                DiskPressure::Warning
            ),
            DiskPressure::Warning
        );
        let one_percent = large_total.div_ceil(100);
        assert_eq!(
            evaluate(
                config,
                large_total,
                boundary + one_percent,
                DiskPressure::Warning
            ),
            DiskPressure::Healthy
        );

        // On a small volume, the byte margin is larger.
        let small_total = 64 * GIB;
        let boundary = small_total / 10;
        assert_eq!(
            evaluate(
                config,
                small_total,
                boundary + small_total / 100,
                DiskPressure::Warning
            ),
            DiskPressure::Warning
        );
        assert_eq!(
            evaluate(
                config,
                small_total,
                boundary + 2 * GIB,
                DiskPressure::Warning
            ),
            DiskPressure::Healthy
        );
    }

    #[test]
    fn recovery_percentage_uses_ceiling_semantics_at_fractional_bytes() {
        let config = custom_config(10, 100, 20, 200, 1, 100);
        let total = 101;
        let warning_boundary = 2;

        // One byte is less than one percentage point of 101 bytes.
        assert_eq!(
            evaluate(config, total, warning_boundary + 1, DiskPressure::Warning),
            DiskPressure::Warning
        );
        assert_eq!(
            evaluate(config, total, warning_boundary + 2, DiskPressure::Warning),
            DiskPressure::Healthy
        );
    }

    #[test]
    fn critical_deescalation_obeys_both_level_recovery_boundaries() {
        let config = DiskPressureConfig::default();
        let total = 4 * TIB;

        assert_eq!(
            evaluate(config, total, 11 * GIB, DiskPressure::Critical),
            DiskPressure::Critical
        );
        assert_eq!(
            evaluate(config, total, 60 * GIB, DiskPressure::Critical),
            DiskPressure::Warning
        );
        assert_eq!(
            evaluate(config, total, 80 * GIB, DiskPressure::Critical),
            DiskPressure::Healthy
        );
    }

    #[test]
    fn checked_arithmetic_handles_maximum_capacity() {
        let config = DiskPressureConfig::default();
        let result = config.evaluate(capacity(u64::MAX, u64::MAX), DiskPressure::Unknown);
        assert_eq!(result.critical_boundary_bytes(), 10 * GIB);
        assert_eq!(result.warning_boundary_bytes(), 30 * GIB);
        assert_eq!(result.pressure(), DiskPressure::Healthy);

        let percent_limited = custom_config(u64::MAX, 500, u64::MAX, 1_000, u64::MAX, 10_000);
        let result = percent_limited.evaluate(capacity(u64::MAX, 0), DiskPressure::Unknown);
        assert_eq!(
            result.critical_boundary_bytes(),
            u64::try_from((u128::from(u64::MAX) * 500) / 10_000).unwrap()
        );
        assert_eq!(
            result.warning_boundary_bytes(),
            u64::try_from((u128::from(u64::MAX) * 1_000) / 10_000).unwrap()
        );
    }

    #[test]
    fn pressure_never_worsens_as_available_capacity_increases() {
        let config = custom_config(10_000, 500, 20_000, 1_000, 500, 100);
        let total = 50_000;

        for previous in [
            DiskPressure::Unknown,
            DiskPressure::Healthy,
            DiskPressure::Warning,
            DiskPressure::Critical,
        ] {
            let mut prior_severity = u8::MAX;
            for available in 0..=total {
                let pressure = evaluate(config, total, available, previous);
                let severity = match pressure {
                    DiskPressure::Healthy => 0,
                    DiskPressure::Warning => 1,
                    DiskPressure::Critical => 2,
                    DiskPressure::Unknown => unreachable!(),
                };
                assert!(
                    severity <= prior_severity,
                    "pressure worsened for {previous:?} at {available} bytes"
                );
                prior_severity = severity;
            }
        }
    }

    #[test]
    fn default_boundaries_are_monotonic_across_representative_volume_sizes() {
        let config = DiskPressureConfig::default();
        let mut previous_critical = 0;
        let mut previous_warning = 0;
        for total in [1, 101, GIB, 32 * GIB, 64 * GIB, TIB, 4 * TIB, u64::MAX] {
            let result = config.evaluate(capacity(total, 0), DiskPressure::Unknown);
            assert!(result.critical_boundary_bytes() <= result.warning_boundary_bytes());
            assert!(result.critical_boundary_bytes() >= previous_critical);
            assert!(result.warning_boundary_bytes() >= previous_warning);
            previous_critical = result.critical_boundary_bytes();
            previous_warning = result.warning_boundary_bytes();
        }
    }
}
