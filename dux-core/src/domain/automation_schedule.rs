//! Inert, bounded configuration for future cleanup automation.
//!
//! A draft records user preferences only. It has no enabled state, path,
//! candidate, plan, journal, or effect capability, and therefore cannot grant
//! cleanup authority. A later scheduler must separately prove eligibility and
//! create a fresh deterministic plan for every run.

use std::time::{Duration, SystemTime};

use thiserror::Error;

use super::{AutomationScheduleId, CandidateCategory, RuleRef};

pub const MAX_AUTOMATION_SCHEDULE_DRAFTS: usize = 64;
pub const MAX_AUTOMATION_SCHEDULE_EXCLUSIONS: usize = 32;
pub const DEFAULT_AUTOMATION_MINIMUM_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
pub const DEFAULT_AUTOMATION_MINIMUM_RECLAIMABLE_BYTES: u64 = 0;
pub const DEFAULT_AUTOMATION_MAXIMUM_BYTES_PER_RUN: u64 = 25 * 1024 * 1024 * 1024;
pub const DEFAULT_AUTOMATION_PRE_RUN_NOTIFICATIONS: u8 = 3;

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

/// Validated user-editable fields for one disabled schedule draft.
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
pub struct AutomationScheduleDraft {
    id: AutomationScheduleId,
    config: AutomationScheduleDraftConfig,
    revision: u64,
    created_at: SystemTime,
    updated_at: SystemTime,
    pre_run_notifications_remaining: u8,
}

impl AutomationScheduleDraft {
    pub(crate) fn from_stored_parts(
        id: AutomationScheduleId,
        config: AutomationScheduleDraftConfig,
        revision: u64,
        created_at: SystemTime,
        updated_at: SystemTime,
        pre_run_notifications_remaining: u8,
    ) -> Self {
        Self {
            id,
            config,
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
