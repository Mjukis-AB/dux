//! Pure, path-free scheduling decisions for future cleanup automation.
//!
//! This module only decides whether one exact schedule revision is ready for a
//! fresh eligibility assessment. It cannot establish eligibility, create a
//! cleanup plan, enqueue work, or grant cleanup authority. Time and runtime
//! facts are injected so the decision is deterministic and side-effect free.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use super::{AutomationScheduleCadence, AutomationScheduleId, MAX_AUTOMATION_SCHEDULE_DRAFTS};

pub const AUTOMATION_SCHEDULER_DECISION_POLICY_REVISION: u32 = 1;
pub const MAX_AUTOMATION_SCHEDULER_OBSERVATIONS: usize = MAX_AUTOMATION_SCHEDULE_DRAFTS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationSchedulerSignal {
    Startup,
    Timer,
    Wake,
    SignificantTimeChange,
    LowDiskPressureChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationGlobalSwitchEvidence {
    Disabled,
    Enabled,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationScheduleActivationEvidence {
    DisabledDraft,
    Paused,
    Enabled,
    Unavailable,
}

/// Runtime facts observed by a trusted core-owned adapter.
///
/// `Unavailable` is distinct from an observed all-clear state and always
/// defers a due assessment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationSchedulerRuntimeEvidence {
    Unavailable,
    Observed {
        engine_open: bool,
        manual_scan_active: bool,
        cleanup_active: bool,
        thermal_restricted: bool,
        battery_restricted: bool,
    },
}

/// Durable clock facts for one exact schedule revision.
///
/// Monthly calendar and timezone semantics remain outside this kernel: both
/// periodic cadences provide an already-materialized wall-clock deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationScheduleClockObservation {
    Periodic {
        next_check_at: SystemTime,
    },
    LowDiskOnly {
        current_episode_started_at: Option<SystemTime>,
        last_assessed_episode_started_at: Option<SystemTime>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationSchedulerScheduleObservation {
    pub schedule_id: AutomationScheduleId,
    pub schedule_revision: u64,
    pub activation: AutomationScheduleActivationEvidence,
    pub cadence: AutomationScheduleCadence,
    pub clock: AutomationScheduleClockObservation,
}

pub struct AutomationSchedulerDecisionInput<'a> {
    pub observed_at: SystemTime,
    pub signal: AutomationSchedulerSignal,
    pub global_switch: AutomationGlobalSwitchEvidence,
    pub runtime: AutomationSchedulerRuntimeEvidence,
    pub schedules: &'a [AutomationSchedulerScheduleObservation],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationSchedulerTrigger {
    Scheduled { scheduled_for: SystemTime },
    LowDisk { episode_started_at: SystemTime },
}

impl AutomationSchedulerTrigger {
    const fn priority(self) -> u8 {
        match self {
            Self::LowDisk { .. } => 0,
            Self::Scheduled { .. } => 1,
        }
    }

    const fn occurred_at(self) -> SystemTime {
        match self {
            Self::Scheduled { scheduled_for } => scheduled_for,
            Self::LowDisk { episode_started_at } => episode_started_at,
        }
    }
}

/// A request to collect fresh eligibility evidence for one exact revision.
///
/// This is an observation, not an execution capability. No cleanup API accepts
/// it, and it deliberately contains no path, candidate, plan, or task data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationSchedulerRevalidationObservation {
    schedule_id: AutomationScheduleId,
    schedule_revision: u64,
    trigger: AutomationSchedulerTrigger,
    observed_at: SystemTime,
    wake_catch_up: bool,
}

impl AutomationSchedulerRevalidationObservation {
    pub fn schedule_id(&self) -> &AutomationScheduleId {
        &self.schedule_id
    }

    pub const fn schedule_revision(&self) -> u64 {
        self.schedule_revision
    }

    pub const fn trigger(&self) -> AutomationSchedulerTrigger {
        self.trigger
    }

    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }

    pub const fn wake_catch_up(&self) -> bool {
        self.wake_catch_up
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationSchedulerDormantReason {
    GlobalSwitchDisabled,
    NoEnabledSchedules,
    NoPendingTrigger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationSchedulerDeferredReason {
    GlobalSwitchUnavailable,
    ScheduleActivationUnavailable,
    RuntimeUnavailable,
    EngineClosed,
    CleanupActive,
    ManualScanActive,
    ThermalRestricted,
    BatteryRestricted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutomationSchedulerDecision {
    Dormant {
        reason: AutomationSchedulerDormantReason,
    },
    WaitUntil {
        next_check_at: SystemTime,
    },
    Deferred {
        reason: AutomationSchedulerDeferredReason,
    },
    NeedsEligibilityAssessment(AutomationSchedulerRevalidationObservation),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationSchedulerAssessment {
    policy_revision: u32,
    decision: AutomationSchedulerDecision,
}

impl AutomationSchedulerAssessment {
    pub const fn policy_revision(&self) -> u32 {
        self.policy_revision
    }

    pub const fn decision(&self) -> &AutomationSchedulerDecision {
        &self.decision
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum AutomationSchedulerInputError {
    #[error("the scheduler observation exceeds the supported schedule limit")]
    TooManySchedules,
    #[error("the scheduler observation time predates the Unix epoch")]
    ObservedAtBeforeUnixEpoch,
    #[error("a schedule revision must be greater than zero")]
    ZeroScheduleRevision,
    #[error("the scheduler observation contains a duplicate schedule ID")]
    DuplicateScheduleId,
    #[error("a schedule cadence does not match its clock observation")]
    CadenceClockMismatch,
    #[error("a schedule clock timestamp predates the Unix epoch")]
    ScheduleTimeBeforeUnixEpoch,
    #[error("a low-disk episode timestamp is later than the observation time")]
    LowDiskEpisodeFutureDated,
    #[error("the last-assessed low-disk episode is later than the current episode")]
    LowDiskEpisodeOrderInvalid,
}

struct PendingAssessment<'a> {
    schedule: &'a AutomationSchedulerScheduleObservation,
    trigger: AutomationSchedulerTrigger,
}

/// Assesses injected scheduler facts without performing I/O or granting
/// execution authority.
pub fn assess_automation_scheduler(
    input: AutomationSchedulerDecisionInput<'_>,
) -> Result<AutomationSchedulerAssessment, AutomationSchedulerInputError> {
    validate_input(&input)?;

    let decision = match input.global_switch {
        AutomationGlobalSwitchEvidence::Disabled => AutomationSchedulerDecision::Dormant {
            reason: AutomationSchedulerDormantReason::GlobalSwitchDisabled,
        },
        AutomationGlobalSwitchEvidence::Unavailable => AutomationSchedulerDecision::Deferred {
            reason: AutomationSchedulerDeferredReason::GlobalSwitchUnavailable,
        },
        AutomationGlobalSwitchEvidence::Enabled => assess_enabled_schedules(&input),
    };

    Ok(AutomationSchedulerAssessment {
        policy_revision: AUTOMATION_SCHEDULER_DECISION_POLICY_REVISION,
        decision,
    })
}

fn validate_input(
    input: &AutomationSchedulerDecisionInput<'_>,
) -> Result<(), AutomationSchedulerInputError> {
    if input.schedules.len() > MAX_AUTOMATION_SCHEDULER_OBSERVATIONS {
        return Err(AutomationSchedulerInputError::TooManySchedules);
    }
    validate_epoch(input.observed_at)
        .map_err(|_| AutomationSchedulerInputError::ObservedAtBeforeUnixEpoch)?;

    let mut schedule_ids = BTreeSet::new();
    for schedule in input.schedules {
        if schedule.schedule_revision == 0 {
            return Err(AutomationSchedulerInputError::ZeroScheduleRevision);
        }
        if !schedule_ids.insert(&schedule.schedule_id) {
            return Err(AutomationSchedulerInputError::DuplicateScheduleId);
        }
        match (schedule.cadence, schedule.clock) {
            (
                AutomationScheduleCadence::Weekly | AutomationScheduleCadence::Monthly,
                AutomationScheduleClockObservation::Periodic { next_check_at },
            ) => validate_epoch(next_check_at)?,
            (
                AutomationScheduleCadence::LowDiskOnly,
                AutomationScheduleClockObservation::LowDiskOnly {
                    current_episode_started_at,
                    last_assessed_episode_started_at,
                },
            ) => validate_low_disk_clock(
                current_episode_started_at,
                last_assessed_episode_started_at,
                input.observed_at,
            )?,
            _ => return Err(AutomationSchedulerInputError::CadenceClockMismatch),
        }
    }
    Ok(())
}

fn validate_epoch(timestamp: SystemTime) -> Result<(), AutomationSchedulerInputError> {
    timestamp
        .duration_since(UNIX_EPOCH)
        .map(|_| ())
        .map_err(|_| AutomationSchedulerInputError::ScheduleTimeBeforeUnixEpoch)
}

fn validate_low_disk_clock(
    current: Option<SystemTime>,
    last_assessed: Option<SystemTime>,
    observed_at: SystemTime,
) -> Result<(), AutomationSchedulerInputError> {
    for timestamp in [current, last_assessed].into_iter().flatten() {
        validate_epoch(timestamp)?;
        if timestamp > observed_at {
            return Err(AutomationSchedulerInputError::LowDiskEpisodeFutureDated);
        }
    }
    if current
        .zip(last_assessed)
        .is_some_and(|(current, last_assessed)| last_assessed > current)
    {
        return Err(AutomationSchedulerInputError::LowDiskEpisodeOrderInvalid);
    }
    Ok(())
}

fn assess_enabled_schedules(
    input: &AutomationSchedulerDecisionInput<'_>,
) -> AutomationSchedulerDecision {
    // One unknown activation could belong to a canonically earlier due row, so
    // no known row may be selected until every activation is known.
    if input.schedules.iter().any(|schedule| {
        matches!(
            schedule.activation,
            AutomationScheduleActivationEvidence::Unavailable
        )
    }) {
        return AutomationSchedulerDecision::Deferred {
            reason: AutomationSchedulerDeferredReason::ScheduleActivationUnavailable,
        };
    }

    let mut enabled_count = 0usize;
    let mut earliest_future = None;
    let mut selected: Option<PendingAssessment<'_>> = None;

    for schedule in input.schedules {
        match schedule.activation {
            AutomationScheduleActivationEvidence::Enabled => enabled_count += 1,
            AutomationScheduleActivationEvidence::Unavailable => {
                return AutomationSchedulerDecision::Deferred {
                    reason: AutomationSchedulerDeferredReason::ScheduleActivationUnavailable,
                };
            }
            AutomationScheduleActivationEvidence::DisabledDraft
            | AutomationScheduleActivationEvidence::Paused => continue,
        }

        let trigger = match schedule.clock {
            AutomationScheduleClockObservation::Periodic { next_check_at }
                if next_check_at <= input.observed_at =>
            {
                Some(AutomationSchedulerTrigger::Scheduled {
                    scheduled_for: next_check_at,
                })
            }
            AutomationScheduleClockObservation::Periodic { next_check_at } => {
                earliest_future = Some(earliest_future.map_or(next_check_at, |earliest| {
                    std::cmp::min(earliest, next_check_at)
                }));
                None
            }
            AutomationScheduleClockObservation::LowDiskOnly {
                current_episode_started_at: Some(current),
                last_assessed_episode_started_at,
            } if last_assessed_episode_started_at != Some(current) => {
                Some(AutomationSchedulerTrigger::LowDisk {
                    episode_started_at: current,
                })
            }
            AutomationScheduleClockObservation::LowDiskOnly { .. } => None,
        };

        if let Some(trigger) = trigger {
            let candidate = PendingAssessment { schedule, trigger };
            if selected
                .as_ref()
                .is_none_or(|selected| pending_precedes(&candidate, selected))
            {
                selected = Some(candidate);
            }
        }
    }

    if enabled_count == 0 {
        return AutomationSchedulerDecision::Dormant {
            reason: AutomationSchedulerDormantReason::NoEnabledSchedules,
        };
    }

    let Some(selected) = selected else {
        return earliest_future.map_or(
            AutomationSchedulerDecision::Dormant {
                reason: AutomationSchedulerDormantReason::NoPendingTrigger,
            },
            |next_check_at| AutomationSchedulerDecision::WaitUntil { next_check_at },
        );
    };

    if let Some(reason) = runtime_deferred_reason(input.runtime) {
        return AutomationSchedulerDecision::Deferred { reason };
    }

    let occurred_at = selected.trigger.occurred_at();
    AutomationSchedulerDecision::NeedsEligibilityAssessment(
        AutomationSchedulerRevalidationObservation {
            schedule_id: selected.schedule.schedule_id.clone(),
            schedule_revision: selected.schedule.schedule_revision,
            trigger: selected.trigger,
            observed_at: input.observed_at,
            wake_catch_up: matches!(input.signal, AutomationSchedulerSignal::Wake)
                && occurred_at < input.observed_at,
        },
    )
}

fn pending_precedes(candidate: &PendingAssessment<'_>, selected: &PendingAssessment<'_>) -> bool {
    (
        candidate.trigger.priority(),
        candidate.trigger.occurred_at(),
        &candidate.schedule.schedule_id,
    ) < (
        selected.trigger.priority(),
        selected.trigger.occurred_at(),
        &selected.schedule.schedule_id,
    )
}

fn runtime_deferred_reason(
    runtime: AutomationSchedulerRuntimeEvidence,
) -> Option<AutomationSchedulerDeferredReason> {
    let AutomationSchedulerRuntimeEvidence::Observed {
        engine_open,
        manual_scan_active,
        cleanup_active,
        thermal_restricted,
        battery_restricted,
    } = runtime
    else {
        return Some(AutomationSchedulerDeferredReason::RuntimeUnavailable);
    };
    if !engine_open {
        Some(AutomationSchedulerDeferredReason::EngineClosed)
    } else if cleanup_active {
        Some(AutomationSchedulerDeferredReason::CleanupActive)
    } else if manual_scan_active {
        Some(AutomationSchedulerDeferredReason::ManualScanActive)
    } else if thermal_restricted {
        Some(AutomationSchedulerDeferredReason::ThermalRestricted)
    } else if battery_restricted {
        Some(AutomationSchedulerDeferredReason::BatteryRestricted)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const CLEAR_RUNTIME: AutomationSchedulerRuntimeEvidence =
        AutomationSchedulerRuntimeEvidence::Observed {
            engine_open: true,
            manual_scan_active: false,
            cleanup_active: false,
            thermal_restricted: false,
            battery_restricted: false,
        };

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn periodic(
        id: &str,
        revision: u64,
        activation: AutomationScheduleActivationEvidence,
        cadence: AutomationScheduleCadence,
        next_check_seconds: u64,
    ) -> AutomationSchedulerScheduleObservation {
        AutomationSchedulerScheduleObservation {
            schedule_id: AutomationScheduleId::new(id).expect("valid schedule ID"),
            schedule_revision: revision,
            activation,
            cadence,
            clock: AutomationScheduleClockObservation::Periodic {
                next_check_at: at(next_check_seconds),
            },
        }
    }

    fn low_disk(
        id: &str,
        current: Option<u64>,
        last_assessed: Option<u64>,
    ) -> AutomationSchedulerScheduleObservation {
        AutomationSchedulerScheduleObservation {
            schedule_id: AutomationScheduleId::new(id).expect("valid schedule ID"),
            schedule_revision: 1,
            activation: AutomationScheduleActivationEvidence::Enabled,
            cadence: AutomationScheduleCadence::LowDiskOnly,
            clock: AutomationScheduleClockObservation::LowDiskOnly {
                current_episode_started_at: current.map(at),
                last_assessed_episode_started_at: last_assessed.map(at),
            },
        }
    }

    fn assess(
        schedules: &[AutomationSchedulerScheduleObservation],
        observed_at: SystemTime,
        signal: AutomationSchedulerSignal,
        global_switch: AutomationGlobalSwitchEvidence,
        runtime: AutomationSchedulerRuntimeEvidence,
    ) -> Result<AutomationSchedulerAssessment, AutomationSchedulerInputError> {
        assess_automation_scheduler(AutomationSchedulerDecisionInput {
            observed_at,
            signal,
            global_switch,
            runtime,
            schedules,
        })
    }

    #[test]
    fn global_switch_is_default_off_and_unavailable_defers() {
        let schedules = [periodic(
            "due",
            1,
            AutomationScheduleActivationEvidence::Enabled,
            AutomationScheduleCadence::Weekly,
            10,
        )];
        assert_eq!(
            assess(
                &schedules,
                at(20),
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Disabled,
                CLEAR_RUNTIME,
            )
            .unwrap()
            .decision(),
            &AutomationSchedulerDecision::Dormant {
                reason: AutomationSchedulerDormantReason::GlobalSwitchDisabled
            }
        );
        assert_eq!(
            assess(
                &schedules,
                at(20),
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Unavailable,
                CLEAR_RUNTIME,
            )
            .unwrap()
            .decision(),
            &AutomationSchedulerDecision::Deferred {
                reason: AutomationSchedulerDeferredReason::GlobalSwitchUnavailable
            }
        );
    }

    #[test]
    fn disabled_and_paused_activation_are_dormant() {
        for (activation, reason) in [
            (
                AutomationScheduleActivationEvidence::DisabledDraft,
                AutomationSchedulerDormantReason::NoEnabledSchedules,
            ),
            (
                AutomationScheduleActivationEvidence::Paused,
                AutomationSchedulerDormantReason::NoEnabledSchedules,
            ),
        ] {
            let schedules = [periodic(
                "inactive",
                1,
                activation,
                AutomationScheduleCadence::Monthly,
                10,
            )];
            assert_eq!(
                assess(
                    &schedules,
                    at(20),
                    AutomationSchedulerSignal::Timer,
                    AutomationGlobalSwitchEvidence::Enabled,
                    CLEAR_RUNTIME,
                )
                .unwrap()
                .decision(),
                &AutomationSchedulerDecision::Dormant { reason }
            );
        }
    }

    #[test]
    fn unavailable_activation_defers_even_with_an_enabled_due_schedule() {
        let schedules = [
            periodic(
                "known-due",
                1,
                AutomationScheduleActivationEvidence::Enabled,
                AutomationScheduleCadence::Weekly,
                10,
            ),
            periodic(
                "unknown-earlier",
                1,
                AutomationScheduleActivationEvidence::Unavailable,
                AutomationScheduleCadence::Weekly,
                5,
            ),
        ];
        assert_eq!(
            assess(
                &schedules,
                at(20),
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Enabled,
                CLEAR_RUNTIME,
            )
            .unwrap()
            .decision(),
            &AutomationSchedulerDecision::Deferred {
                reason: AutomationSchedulerDeferredReason::ScheduleActivationUnavailable
            }
        );
    }

    #[test]
    fn wake_and_time_change_do_not_make_a_future_deadline_due() {
        let schedules = [periodic(
            "future",
            1,
            AutomationScheduleActivationEvidence::Enabled,
            AutomationScheduleCadence::Monthly,
            30,
        )];
        for signal in [
            AutomationSchedulerSignal::Wake,
            AutomationSchedulerSignal::SignificantTimeChange,
        ] {
            assert_eq!(
                assess(
                    &schedules,
                    at(20),
                    signal,
                    AutomationGlobalSwitchEvidence::Enabled,
                    CLEAR_RUNTIME,
                )
                .unwrap()
                .decision(),
                &AutomationSchedulerDecision::WaitUntil {
                    next_check_at: at(30)
                }
            );
        }
    }

    #[test]
    fn exact_deadline_revalidates_without_marking_a_catch_up() {
        let schedules = [periodic(
            "exact",
            7,
            AutomationScheduleActivationEvidence::Enabled,
            AutomationScheduleCadence::Weekly,
            20,
        )];
        let assessment = assess(
            &schedules,
            at(20),
            AutomationSchedulerSignal::Wake,
            AutomationGlobalSwitchEvidence::Enabled,
            CLEAR_RUNTIME,
        )
        .unwrap();
        let AutomationSchedulerDecision::NeedsEligibilityAssessment(observation) =
            assessment.decision()
        else {
            panic!("deadline must request revalidation");
        };
        assert_eq!(observation.schedule_id().as_str(), "exact");
        assert_eq!(observation.schedule_revision(), 7);
        assert_eq!(
            observation.trigger(),
            AutomationSchedulerTrigger::Scheduled {
                scheduled_for: at(20)
            }
        );
        assert!(!observation.wake_catch_up());
    }

    #[test]
    fn overdue_work_coalesces_to_one_canonical_observation() {
        let first_order = [
            periodic(
                "z-later",
                1,
                AutomationScheduleActivationEvidence::Enabled,
                AutomationScheduleCadence::Weekly,
                12,
            ),
            periodic(
                "b-tie",
                2,
                AutomationScheduleActivationEvidence::Enabled,
                AutomationScheduleCadence::Monthly,
                10,
            ),
            periodic(
                "a-tie",
                3,
                AutomationScheduleActivationEvidence::Enabled,
                AutomationScheduleCadence::Weekly,
                10,
            ),
        ];
        let second_order = [
            first_order[2].clone(),
            first_order[0].clone(),
            first_order[1].clone(),
        ];
        let decide = |schedules: &[AutomationSchedulerScheduleObservation]| {
            assess(
                schedules,
                at(50),
                AutomationSchedulerSignal::Wake,
                AutomationGlobalSwitchEvidence::Enabled,
                CLEAR_RUNTIME,
            )
            .unwrap()
        };
        let first = decide(&first_order);
        let second = decide(&second_order);
        assert_eq!(first, second);
        let AutomationSchedulerDecision::NeedsEligibilityAssessment(observation) = first.decision()
        else {
            panic!("one overdue schedule must be selected");
        };
        assert_eq!(observation.schedule_id().as_str(), "a-tie");
        assert!(observation.wake_catch_up());
    }

    #[test]
    fn a_new_low_disk_episode_revalidates_once_and_only_changes_priority() {
        let schedules = [
            periodic(
                "old-periodic",
                1,
                AutomationScheduleActivationEvidence::Enabled,
                AutomationScheduleCadence::Weekly,
                5,
            ),
            low_disk("pressure", Some(15), Some(10)),
        ];
        let assessment = assess(
            &schedules,
            at(20),
            AutomationSchedulerSignal::LowDiskPressureChanged,
            AutomationGlobalSwitchEvidence::Enabled,
            CLEAR_RUNTIME,
        )
        .unwrap();
        let AutomationSchedulerDecision::NeedsEligibilityAssessment(observation) =
            assessment.decision()
        else {
            panic!("new low-disk episode must request revalidation");
        };
        assert_eq!(observation.schedule_id().as_str(), "pressure");
        assert_eq!(
            observation.trigger(),
            AutomationSchedulerTrigger::LowDisk {
                episode_started_at: at(15)
            }
        );
    }

    #[test]
    fn absent_or_already_assessed_low_disk_episode_is_dormant() {
        for schedule in [
            low_disk("absent", None, Some(10)),
            low_disk("same", Some(10), Some(10)),
        ] {
            assert_eq!(
                assess(
                    &[schedule],
                    at(20),
                    AutomationSchedulerSignal::LowDiskPressureChanged,
                    AutomationGlobalSwitchEvidence::Enabled,
                    CLEAR_RUNTIME,
                )
                .unwrap()
                .decision(),
                &AutomationSchedulerDecision::Dormant {
                    reason: AutomationSchedulerDormantReason::NoPendingTrigger
                }
            );
        }
    }

    #[test]
    fn due_assessment_defers_for_every_unsafe_runtime_state() {
        let schedules = [periodic(
            "due",
            1,
            AutomationScheduleActivationEvidence::Enabled,
            AutomationScheduleCadence::Weekly,
            10,
        )];
        let observed = |engine_open, manual_scan_active, cleanup_active, thermal, battery| {
            AutomationSchedulerRuntimeEvidence::Observed {
                engine_open,
                manual_scan_active,
                cleanup_active,
                thermal_restricted: thermal,
                battery_restricted: battery,
            }
        };
        for (runtime, reason) in [
            (
                AutomationSchedulerRuntimeEvidence::Unavailable,
                AutomationSchedulerDeferredReason::RuntimeUnavailable,
            ),
            (
                observed(false, false, false, false, false),
                AutomationSchedulerDeferredReason::EngineClosed,
            ),
            (
                observed(true, false, true, false, false),
                AutomationSchedulerDeferredReason::CleanupActive,
            ),
            (
                observed(true, true, false, false, false),
                AutomationSchedulerDeferredReason::ManualScanActive,
            ),
            (
                observed(true, false, false, true, false),
                AutomationSchedulerDeferredReason::ThermalRestricted,
            ),
            (
                observed(true, false, false, false, true),
                AutomationSchedulerDeferredReason::BatteryRestricted,
            ),
        ] {
            assert_eq!(
                assess(
                    &schedules,
                    at(20),
                    AutomationSchedulerSignal::Timer,
                    AutomationGlobalSwitchEvidence::Enabled,
                    runtime,
                )
                .unwrap()
                .decision(),
                &AutomationSchedulerDecision::Deferred { reason }
            );
        }
    }

    #[test]
    fn validates_limit_revision_duplicates_and_cadence_clock_pairing() {
        let mut schedules = (0..MAX_AUTOMATION_SCHEDULER_OBSERVATIONS)
            .map(|index| {
                periodic(
                    &format!("schedule-{index}"),
                    1,
                    AutomationScheduleActivationEvidence::Paused,
                    AutomationScheduleCadence::Weekly,
                    10,
                )
            })
            .collect::<Vec<_>>();
        assert!(
            assess(
                &schedules,
                at(20),
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Enabled,
                CLEAR_RUNTIME,
            )
            .is_ok()
        );
        schedules.push(periodic(
            "one-too-many",
            1,
            AutomationScheduleActivationEvidence::Paused,
            AutomationScheduleCadence::Weekly,
            10,
        ));
        assert_eq!(
            assess(
                &schedules,
                at(20),
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Enabled,
                CLEAR_RUNTIME,
            ),
            Err(AutomationSchedulerInputError::TooManySchedules)
        );

        let zero_revision = [periodic(
            "zero",
            0,
            AutomationScheduleActivationEvidence::Enabled,
            AutomationScheduleCadence::Weekly,
            10,
        )];
        assert_eq!(
            assess(
                &zero_revision,
                at(20),
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Enabled,
                CLEAR_RUNTIME,
            ),
            Err(AutomationSchedulerInputError::ZeroScheduleRevision)
        );

        let duplicate = [
            periodic(
                "same",
                1,
                AutomationScheduleActivationEvidence::Enabled,
                AutomationScheduleCadence::Weekly,
                10,
            ),
            periodic(
                "same",
                2,
                AutomationScheduleActivationEvidence::Enabled,
                AutomationScheduleCadence::Weekly,
                11,
            ),
        ];
        assert_eq!(
            assess(
                &duplicate,
                at(20),
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Enabled,
                CLEAR_RUNTIME,
            ),
            Err(AutomationSchedulerInputError::DuplicateScheduleId)
        );

        let mismatch = [AutomationSchedulerScheduleObservation {
            schedule_id: AutomationScheduleId::new("mismatch").unwrap(),
            schedule_revision: 1,
            activation: AutomationScheduleActivationEvidence::Enabled,
            cadence: AutomationScheduleCadence::Weekly,
            clock: AutomationScheduleClockObservation::LowDiskOnly {
                current_episode_started_at: None,
                last_assessed_episode_started_at: None,
            },
        }];
        assert_eq!(
            assess(
                &mismatch,
                at(20),
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Enabled,
                CLEAR_RUNTIME,
            ),
            Err(AutomationSchedulerInputError::CadenceClockMismatch)
        );
    }

    #[test]
    fn rejects_pre_epoch_and_invalid_low_disk_episode_times() {
        let pre_epoch = UNIX_EPOCH - Duration::from_secs(1);
        assert_eq!(
            assess(
                &[],
                pre_epoch,
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Disabled,
                CLEAR_RUNTIME,
            ),
            Err(AutomationSchedulerInputError::ObservedAtBeforeUnixEpoch)
        );

        let schedule = AutomationSchedulerScheduleObservation {
            schedule_id: AutomationScheduleId::new("pre-epoch").unwrap(),
            schedule_revision: 1,
            activation: AutomationScheduleActivationEvidence::Enabled,
            cadence: AutomationScheduleCadence::Weekly,
            clock: AutomationScheduleClockObservation::Periodic {
                next_check_at: pre_epoch,
            },
        };
        assert_eq!(
            assess(
                &[schedule],
                at(20),
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Enabled,
                CLEAR_RUNTIME,
            ),
            Err(AutomationSchedulerInputError::ScheduleTimeBeforeUnixEpoch)
        );

        assert_eq!(
            assess(
                &[low_disk("future", Some(21), None)],
                at(20),
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Enabled,
                CLEAR_RUNTIME,
            ),
            Err(AutomationSchedulerInputError::LowDiskEpisodeFutureDated)
        );
        assert_eq!(
            assess(
                &[low_disk("reversed", Some(10), Some(11))],
                at(20),
                AutomationSchedulerSignal::Timer,
                AutomationGlobalSwitchEvidence::Enabled,
                CLEAR_RUNTIME,
            ),
            Err(AutomationSchedulerInputError::LowDiskEpisodeOrderInvalid)
        );
    }
}
