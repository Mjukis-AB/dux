//! Bounded configuration and path-free activation state for cleanup automation.
//!
//! A schedule records user preferences, consent state, and a recurrence cursor.
//! It has no path, candidate, plan, journal, or effect capability and therefore
//! cannot grant cleanup authority. Every run must still prove current
//! eligibility and create a fresh deterministic plan.

use std::time::{Duration, SystemTime};

use thiserror::Error;

use super::{AutomationScheduleId, CandidateCategory, RuleRef};

pub const MAX_AUTOMATION_SCHEDULE_DRAFTS: usize = 64;
pub const MAX_AUTOMATION_SCHEDULE_EXCLUSIONS: usize = 32;
pub const DEFAULT_AUTOMATION_MINIMUM_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
pub const DEFAULT_AUTOMATION_MINIMUM_RECLAIMABLE_BYTES: u64 = 0;
pub const DEFAULT_AUTOMATION_MAXIMUM_BYTES_PER_RUN: u64 = 25 * 1024 * 1024 * 1024;
pub const DEFAULT_AUTOMATION_PRE_RUN_NOTIFICATIONS: u8 = 3;
pub const AUTOMATION_RECURRENCE_POLICY_REVISION: u32 = 1;

pub(crate) const MAX_AUTOMATION_UNIX_MS: i64 = 253_402_300_799_999;
const MILLIS_PER_DAY: i64 = 86_400_000;
const MILLIS_PER_WEEK: i64 = 7 * MILLIS_PER_DAY;

const MAX_AUTOMATION_MINIMUM_AGE: Duration = Duration::from_secs(36_500 * 24 * 60 * 60);
const MAX_AUTOMATION_STORED_BYTES: u64 = i64::MAX as u64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutomationScheduleScope {
    Rule(RuleRef),
    Category(CandidateCategory),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationScheduleCadence {
    Weekly,
    Monthly,
    LowDiskOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationConfirmationMode {
    RequireConfirmation,
    FullyAutomatic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationSchedulePauseReason {
    User,
    Failure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationPeriodicCursor {
    cursor_revision: u64,
    recurrence_policy_revision: u32,
    recurrence_anchor: SystemTime,
    next_occurrence_ordinal: u64,
    next_run: SystemTime,
}

impl AutomationPeriodicCursor {
    pub(crate) const fn from_stored_parts(
        cursor_revision: u64,
        recurrence_policy_revision: u32,
        recurrence_anchor: SystemTime,
        next_occurrence_ordinal: u64,
        next_run: SystemTime,
    ) -> Self {
        Self {
            cursor_revision,
            recurrence_policy_revision,
            recurrence_anchor,
            next_occurrence_ordinal,
            next_run,
        }
    }

    pub const fn cursor_revision(self) -> u64 {
        self.cursor_revision
    }

    pub const fn recurrence_policy_revision(self) -> u32 {
        self.recurrence_policy_revision
    }

    pub const fn recurrence_anchor(self) -> SystemTime {
        self.recurrence_anchor
    }

    pub const fn next_occurrence_ordinal(self) -> u64 {
        self.next_occurrence_ordinal
    }

    pub const fn next_run(self) -> SystemTime {
        self.next_run
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationScheduleCursor {
    Periodic(AutomationPeriodicCursor),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationScheduleState {
    Disabled,
    Enabled,
    Paused(AutomationSchedulePauseReason),
}

/// Validated user-editable fields for one schedule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationScheduleDraftConfig {
    scope: AutomationScheduleScope,
    cadence: AutomationScheduleCadence,
    minimum_age: Duration,
    minimum_reclaimable_bytes: u64,
    maximum_bytes_per_run: u64,
    excluded_rules: Vec<RuleRef>,
    notify_before_run: bool,
    confirmation_mode: AutomationConfirmationMode,
}

impl AutomationScheduleDraftConfig {
    pub fn default_for_scope(scope: AutomationScheduleScope) -> Self {
        Self {
            scope,
            cadence: AutomationScheduleCadence::Monthly,
            minimum_age: DEFAULT_AUTOMATION_MINIMUM_AGE,
            minimum_reclaimable_bytes: DEFAULT_AUTOMATION_MINIMUM_RECLAIMABLE_BYTES,
            maximum_bytes_per_run: DEFAULT_AUTOMATION_MAXIMUM_BYTES_PER_RUN,
            excluded_rules: Vec::new(),
            notify_before_run: true,
            confirmation_mode: AutomationConfirmationMode::RequireConfirmation,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        scope: AutomationScheduleScope,
        cadence: AutomationScheduleCadence,
        minimum_age: Duration,
        minimum_reclaimable_bytes: u64,
        maximum_bytes_per_run: u64,
        mut excluded_rules: Vec<RuleRef>,
        notify_before_run: bool,
        confirmation_mode: AutomationConfirmationMode,
    ) -> Result<Self, AutomationScheduleConfigError> {
        if minimum_age.subsec_nanos() != 0 || minimum_age > MAX_AUTOMATION_MINIMUM_AGE {
            return Err(AutomationScheduleConfigError::InvalidMinimumAge);
        }
        if minimum_reclaimable_bytes > MAX_AUTOMATION_STORED_BYTES {
            return Err(AutomationScheduleConfigError::InvalidMinimumReclaimableBytes);
        }
        if maximum_bytes_per_run == 0 || maximum_bytes_per_run > MAX_AUTOMATION_STORED_BYTES {
            return Err(AutomationScheduleConfigError::InvalidMaximumBytesPerRun);
        }
        if excluded_rules.len() > MAX_AUTOMATION_SCHEDULE_EXCLUSIONS {
            return Err(AutomationScheduleConfigError::TooManyExclusions);
        }
        if matches!(scope, AutomationScheduleScope::Rule(_)) && !excluded_rules.is_empty() {
            return Err(AutomationScheduleConfigError::ExclusionsRequireCategoryScope);
        }
        excluded_rules.sort();
        if excluded_rules.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(AutomationScheduleConfigError::DuplicateExclusion);
        }
        Ok(Self {
            scope,
            cadence,
            minimum_age,
            minimum_reclaimable_bytes,
            maximum_bytes_per_run,
            excluded_rules,
            notify_before_run,
            confirmation_mode,
        })
    }

    pub fn scope(&self) -> &AutomationScheduleScope {
        &self.scope
    }

    pub const fn cadence(&self) -> AutomationScheduleCadence {
        self.cadence
    }

    pub const fn minimum_age(&self) -> Duration {
        self.minimum_age
    }

    pub const fn minimum_reclaimable_bytes(&self) -> u64 {
        self.minimum_reclaimable_bytes
    }

    pub const fn maximum_bytes_per_run(&self) -> u64 {
        self.maximum_bytes_per_run
    }

    pub fn excluded_rules(&self) -> &[RuleRef] {
        &self.excluded_rules
    }

    pub const fn notify_before_run(&self) -> bool {
        self.notify_before_run
    }

    pub const fn confirmation_mode(&self) -> AutomationConfirmationMode {
        self.confirmation_mode
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationSchedule {
    id: AutomationScheduleId,
    config: AutomationScheduleDraftConfig,
    state: AutomationScheduleState,
    cursor: Option<AutomationScheduleCursor>,
    revision: u64,
    created_at: SystemTime,
    updated_at: SystemTime,
    pre_run_notifications_remaining: u8,
}

impl AutomationSchedule {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_stored_parts(
        id: AutomationScheduleId,
        config: AutomationScheduleDraftConfig,
        state: AutomationScheduleState,
        cursor: Option<AutomationScheduleCursor>,
        revision: u64,
        created_at: SystemTime,
        updated_at: SystemTime,
        pre_run_notifications_remaining: u8,
    ) -> Self {
        Self {
            id,
            config,
            state,
            cursor,
            revision,
            created_at,
            updated_at,
            pre_run_notifications_remaining,
        }
    }

    pub fn id(&self) -> &AutomationScheduleId {
        &self.id
    }

    pub fn config(&self) -> &AutomationScheduleDraftConfig {
        &self.config
    }

    pub const fn state(&self) -> AutomationScheduleState {
        self.state
    }

    pub const fn cursor(&self) -> Option<AutomationScheduleCursor> {
        self.cursor
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn created_at(&self) -> SystemTime {
        self.created_at
    }

    pub const fn updated_at(&self) -> SystemTime {
        self.updated_at
    }

    pub const fn pre_run_notifications_remaining(&self) -> u8 {
        self.pre_run_notifications_remaining
    }
}

/// Temporary source-compatibility alias while engine and FFI projections move
/// from disabled-only terminology to the activation-aware record.
pub type AutomationScheduleDraft = AutomationSchedule;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum AutomationRecurrenceError {
    #[error("the recurrence timestamp is outside the supported UTC range")]
    TimestampOutOfRange,
    #[error("low-disk cadence has no periodic recurrence")]
    NonPeriodicCadence,
    #[error("the recurrence ordinal is invalid or cannot be represented")]
    InvalidOrdinal,
}

pub(crate) fn materialize_automation_occurrence_unix_ms(
    cadence: AutomationScheduleCadence,
    anchor_unix_ms: i64,
    ordinal: u64,
) -> Result<i64, AutomationRecurrenceError> {
    if !(0..=MAX_AUTOMATION_UNIX_MS).contains(&anchor_unix_ms) {
        return Err(AutomationRecurrenceError::TimestampOutOfRange);
    }
    if ordinal == 0 || ordinal > i64::MAX as u64 {
        return Err(AutomationRecurrenceError::InvalidOrdinal);
    }
    match cadence {
        AutomationScheduleCadence::Weekly => {
            let next =
                i128::from(anchor_unix_ms) + i128::from(MILLIS_PER_WEEK) * i128::from(ordinal);
            i64::try_from(next)
                .ok()
                .filter(|value| *value <= MAX_AUTOMATION_UNIX_MS)
                .ok_or(AutomationRecurrenceError::TimestampOutOfRange)
        }
        AutomationScheduleCadence::Monthly => {
            materialize_monthly_occurrence(anchor_unix_ms, ordinal)
        }
        AutomationScheduleCadence::LowDiskOnly => {
            Err(AutomationRecurrenceError::NonPeriodicCadence)
        }
    }
}

pub(crate) fn first_automation_occurrence_after_unix_ms(
    cadence: AutomationScheduleCadence,
    anchor_unix_ms: i64,
    strictly_after_unix_ms: i64,
) -> Result<(u64, i64), AutomationRecurrenceError> {
    if !(0..=MAX_AUTOMATION_UNIX_MS).contains(&strictly_after_unix_ms) {
        return Err(AutomationRecurrenceError::TimestampOutOfRange);
    }
    match cadence {
        AutomationScheduleCadence::Weekly => {
            let elapsed = strictly_after_unix_ms.saturating_sub(anchor_unix_ms);
            let ordinal = if strictly_after_unix_ms < anchor_unix_ms {
                1
            } else {
                u64::try_from(elapsed / MILLIS_PER_WEEK)
                    .ok()
                    .and_then(|value| value.checked_add(1))
                    .ok_or(AutomationRecurrenceError::InvalidOrdinal)?
            };
            Ok((
                ordinal,
                materialize_automation_occurrence_unix_ms(cadence, anchor_unix_ms, ordinal)?,
            ))
        }
        AutomationScheduleCadence::Monthly => {
            let (anchor_year, anchor_month, _, _) = split_unix_ms(anchor_unix_ms)?;
            let (after_year, after_month, _, _) = split_unix_ms(strictly_after_unix_ms)?;
            let anchor_month_index = i64::from(anchor_year) * 12 + i64::from(anchor_month - 1);
            let after_month_index = i64::from(after_year) * 12 + i64::from(after_month - 1);
            let mut ordinal = u64::try_from((after_month_index - anchor_month_index).max(1))
                .map_err(|_| AutomationRecurrenceError::InvalidOrdinal)?;
            let mut next =
                materialize_automation_occurrence_unix_ms(cadence, anchor_unix_ms, ordinal)?;
            while next <= strictly_after_unix_ms {
                ordinal = ordinal
                    .checked_add(1)
                    .ok_or(AutomationRecurrenceError::InvalidOrdinal)?;
                next = materialize_automation_occurrence_unix_ms(cadence, anchor_unix_ms, ordinal)?;
            }
            while ordinal > 1 {
                let previous = materialize_automation_occurrence_unix_ms(
                    cadence,
                    anchor_unix_ms,
                    ordinal - 1,
                )?;
                if previous <= strictly_after_unix_ms {
                    break;
                }
                ordinal -= 1;
                next = previous;
            }
            Ok((ordinal, next))
        }
        AutomationScheduleCadence::LowDiskOnly => {
            Err(AutomationRecurrenceError::NonPeriodicCadence)
        }
    }
}

fn materialize_monthly_occurrence(
    anchor_unix_ms: i64,
    ordinal: u64,
) -> Result<i64, AutomationRecurrenceError> {
    let (year, month, day, millis_of_day) = split_unix_ms(anchor_unix_ms)?;
    let anchor_month_index = i128::from(year) * 12 + i128::from(month - 1);
    let target_month_index = anchor_month_index + i128::from(ordinal);
    let target_year = i32::try_from(target_month_index.div_euclid(12))
        .map_err(|_| AutomationRecurrenceError::TimestampOutOfRange)?;
    let target_month = u8::try_from(target_month_index.rem_euclid(12) + 1)
        .map_err(|_| AutomationRecurrenceError::TimestampOutOfRange)?;
    if !(1970..=9999).contains(&target_year) {
        return Err(AutomationRecurrenceError::TimestampOutOfRange);
    }
    let target_day = day.min(days_in_month(target_year, target_month));
    let target_days = days_from_civil(target_year, target_month, target_day);
    target_days
        .checked_mul(MILLIS_PER_DAY)
        .and_then(|value| value.checked_add(millis_of_day))
        .filter(|value| *value <= MAX_AUTOMATION_UNIX_MS)
        .ok_or(AutomationRecurrenceError::TimestampOutOfRange)
}

fn split_unix_ms(timestamp: i64) -> Result<(i32, u8, u8, i64), AutomationRecurrenceError> {
    if !(0..=MAX_AUTOMATION_UNIX_MS).contains(&timestamp) {
        return Err(AutomationRecurrenceError::TimestampOutOfRange);
    }
    let days = timestamp / MILLIS_PER_DAY;
    let millis_of_day = timestamp % MILLIS_PER_DAY;
    let (year, month, day) = civil_from_days(days);
    if !(1970..=9999).contains(&year) {
        return Err(AutomationRecurrenceError::TimestampOutOfRange);
    }
    Ok((year, month, day, millis_of_day))
}

const fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let adjusted_year = year - i32::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = i32::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + i32::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    i64::from(era * 146_097 + day_of_era - 719_468)
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u8, u8) {
    let shifted = days_since_epoch + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = i32::try_from(year_of_era + era * 400).expect("supported UTC year fits i32");
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i32::from(month <= 2);
    (
        year,
        u8::try_from(month).expect("civil month is valid"),
        u8::try_from(day).expect("civil day is valid"),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum AutomationScheduleConfigError {
    #[error("minimum age must be a whole number of seconds within the supported range")]
    InvalidMinimumAge,
    #[error("minimum reclaimable bytes exceed the supported storage range")]
    InvalidMinimumReclaimableBytes,
    #[error("maximum bytes per run must be nonzero and within the supported storage range")]
    InvalidMaximumBytesPerRun,
    #[error("the schedule contains too many rule exclusions")]
    TooManyExclusions,
    #[error("rule exclusions are valid only for a category-scoped schedule")]
    ExclusionsRequireCategoryScope,
    #[error("the schedule contains a duplicate rule exclusion")]
    DuplicateExclusion,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{RuleId, RuleRevision};

    fn rule(id: &str, revision: u32) -> RuleRef {
        RuleRef::new(
            RuleId::new(id).unwrap(),
            RuleRevision::new(revision).unwrap(),
        )
    }

    fn utc_ms(year: i32, month: u8, day: u8, millis_of_day: i64) -> i64 {
        days_from_civil(year, month, day) * MILLIS_PER_DAY + millis_of_day
    }

    #[test]
    fn monthly_recurrence_clamps_from_original_anchor_without_drift() {
        let noon = 12 * 60 * 60 * 1_000;
        let anchor = utc_ms(2023, 1, 31, noon);
        assert_eq!(
            materialize_automation_occurrence_unix_ms(
                AutomationScheduleCadence::Monthly,
                anchor,
                1,
            ),
            Ok(utc_ms(2023, 2, 28, noon))
        );
        assert_eq!(
            materialize_automation_occurrence_unix_ms(
                AutomationScheduleCadence::Monthly,
                anchor,
                2,
            ),
            Ok(utc_ms(2023, 3, 31, noon))
        );
        assert_eq!(
            materialize_automation_occurrence_unix_ms(
                AutomationScheduleCadence::Monthly,
                anchor,
                3,
            ),
            Ok(utc_ms(2023, 4, 30, noon))
        );
    }

    #[test]
    fn monthly_recurrence_handles_leap_year_and_strict_future_selection() {
        let anchor = utc_ms(2024, 1, 31, 123);
        let february = utc_ms(2024, 2, 29, 123);
        assert_eq!(
            materialize_automation_occurrence_unix_ms(
                AutomationScheduleCadence::Monthly,
                anchor,
                1,
            ),
            Ok(february)
        );
        assert_eq!(
            first_automation_occurrence_after_unix_ms(
                AutomationScheduleCadence::Monthly,
                anchor,
                february,
            ),
            Ok((2, utc_ms(2024, 3, 31, 123)))
        );
    }

    #[test]
    fn weekly_recurrence_is_exact_utc_arithmetic_and_bounded() {
        let anchor = utc_ms(2026, 8, 10, 42);
        assert_eq!(
            materialize_automation_occurrence_unix_ms(AutomationScheduleCadence::Weekly, anchor, 3,),
            Ok(anchor + 3 * MILLIS_PER_WEEK)
        );
        assert_eq!(
            first_automation_occurrence_after_unix_ms(
                AutomationScheduleCadence::Weekly,
                anchor,
                anchor + 2 * MILLIS_PER_WEEK,
            ),
            Ok((3, anchor + 3 * MILLIS_PER_WEEK))
        );
        assert_eq!(
            materialize_automation_occurrence_unix_ms(
                AutomationScheduleCadence::Weekly,
                MAX_AUTOMATION_UNIX_MS,
                1,
            ),
            Err(AutomationRecurrenceError::TimestampOutOfRange)
        );
        assert_eq!(
            materialize_automation_occurrence_unix_ms(
                AutomationScheduleCadence::LowDiskOnly,
                anchor,
                1,
            ),
            Err(AutomationRecurrenceError::NonPeriodicCadence)
        );
    }

    #[test]
    fn defaults_are_disabled_draft_policy_values() {
        let config = AutomationScheduleDraftConfig::default_for_scope(
            AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
        );
        assert_eq!(config.cadence(), AutomationScheduleCadence::Monthly);
        assert_eq!(config.minimum_age(), DEFAULT_AUTOMATION_MINIMUM_AGE);
        assert_eq!(config.minimum_reclaimable_bytes(), 0);
        assert_eq!(
            config.maximum_bytes_per_run(),
            DEFAULT_AUTOMATION_MAXIMUM_BYTES_PER_RUN
        );
        assert!(config.excluded_rules().is_empty());
        assert!(config.notify_before_run());
        assert_eq!(
            config.confirmation_mode(),
            AutomationConfirmationMode::RequireConfirmation
        );
    }

    #[test]
    fn category_exclusions_are_bounded_unique_and_canonical() {
        let config = AutomationScheduleDraftConfig::try_new(
            AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
            AutomationScheduleCadence::Weekly,
            Duration::ZERO,
            1,
            2,
            vec![rule("developer.z", 2), rule("developer.a", 1)],
            false,
            AutomationConfirmationMode::FullyAutomatic,
        )
        .unwrap();
        assert_eq!(
            config.excluded_rules(),
            [rule("developer.a", 1), rule("developer.z", 2)]
        );

        let duplicate = AutomationScheduleDraftConfig::try_new(
            AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
            AutomationScheduleCadence::LowDiskOnly,
            Duration::ZERO,
            0,
            1,
            vec![rule("developer.a", 1), rule("developer.a", 1)],
            true,
            AutomationConfirmationMode::RequireConfirmation,
        );
        assert_eq!(
            duplicate,
            Err(AutomationScheduleConfigError::DuplicateExclusion)
        );
    }

    #[test]
    fn invalid_numeric_or_scope_combinations_fail_closed() {
        let scope = AutomationScheduleScope::Rule(rule("developer.rust.target", 3));
        assert_eq!(
            AutomationScheduleDraftConfig::try_new(
                scope.clone(),
                AutomationScheduleCadence::Monthly,
                Duration::from_nanos(1),
                0,
                1,
                Vec::new(),
                true,
                AutomationConfirmationMode::RequireConfirmation,
            ),
            Err(AutomationScheduleConfigError::InvalidMinimumAge)
        );
        assert_eq!(
            AutomationScheduleDraftConfig::try_new(
                scope.clone(),
                AutomationScheduleCadence::Monthly,
                Duration::ZERO,
                0,
                0,
                Vec::new(),
                true,
                AutomationConfirmationMode::RequireConfirmation,
            ),
            Err(AutomationScheduleConfigError::InvalidMaximumBytesPerRun)
        );
        assert_eq!(
            AutomationScheduleDraftConfig::try_new(
                scope,
                AutomationScheduleCadence::Monthly,
                Duration::ZERO,
                0,
                1,
                vec![rule("developer.rust.target", 3)],
                true,
                AutomationConfirmationMode::RequireConfirmation,
            ),
            Err(AutomationScheduleConfigError::ExclusionsRequireCategoryScope)
        );

        let category = AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact);
        assert_eq!(
            AutomationScheduleDraftConfig::try_new(
                category.clone(),
                AutomationScheduleCadence::Monthly,
                MAX_AUTOMATION_MINIMUM_AGE + Duration::from_secs(1),
                0,
                1,
                Vec::new(),
                true,
                AutomationConfirmationMode::RequireConfirmation,
            ),
            Err(AutomationScheduleConfigError::InvalidMinimumAge)
        );
        assert_eq!(
            AutomationScheduleDraftConfig::try_new(
                category.clone(),
                AutomationScheduleCadence::Monthly,
                Duration::ZERO,
                MAX_AUTOMATION_STORED_BYTES + 1,
                1,
                Vec::new(),
                true,
                AutomationConfirmationMode::RequireConfirmation,
            ),
            Err(AutomationScheduleConfigError::InvalidMinimumReclaimableBytes)
        );
        assert_eq!(
            AutomationScheduleDraftConfig::try_new(
                category.clone(),
                AutomationScheduleCadence::Monthly,
                Duration::ZERO,
                0,
                MAX_AUTOMATION_STORED_BYTES + 1,
                Vec::new(),
                true,
                AutomationConfirmationMode::RequireConfirmation,
            ),
            Err(AutomationScheduleConfigError::InvalidMaximumBytesPerRun)
        );
        assert_eq!(
            AutomationScheduleDraftConfig::try_new(
                category,
                AutomationScheduleCadence::Monthly,
                Duration::ZERO,
                0,
                1,
                (0..=MAX_AUTOMATION_SCHEDULE_EXCLUSIONS)
                    .map(|index| rule(&format!("developer.rule{index}"), 1))
                    .collect(),
                true,
                AutomationConfirmationMode::RequireConfirmation,
            ),
            Err(AutomationScheduleConfigError::TooManyExclusions)
        );
    }
}
