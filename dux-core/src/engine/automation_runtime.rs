//! Sealed, path-free runtime blockers for future automation assessment.
//!
//! This is an observation only. Even four passing gates do not mean that a
//! schedule is enabled, due, eligible, runnable, or safe. No scheduler or
//! cleanup API accepts this type, and platform energy/current-candidate facts
//! remain deliberately absent.

use std::time::SystemTime;

use crate::persistence::StoredAutomationRuntimeObservation;

use super::EngineLifecycle;

pub(crate) const AUTOMATION_CORE_RUNTIME_POLICY_REVISION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomationCoreRuntimeGate {
    EngineLifecycle,
    RuntimeIdentity,
    ScanWork,
    CleanupWork,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomationCoreRuntimeGateStatus {
    Passed,
    Blocked,
    Unproven,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomationCoreRuntimeReason {
    EngineNotOpen,
    RuntimeIdentityUnavailable,
    RuntimePrivileged,
    RuntimePlatformUnsupported,
    LocalObservationUnavailable,
    ScanWorkActive,
    ScanWorkUnresolved,
    CleanupWorkActive,
    CleanupWorkUnavailable,
    ObservationBudgetExceeded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AutomationCoreRuntimeGateAssessment {
    gate: AutomationCoreRuntimeGate,
    status: AutomationCoreRuntimeGateStatus,
    reason: Option<AutomationCoreRuntimeReason>,
}

impl AutomationCoreRuntimeGateAssessment {
    const fn passed(gate: AutomationCoreRuntimeGate) -> Self {
        Self {
            gate,
            status: AutomationCoreRuntimeGateStatus::Passed,
            reason: None,
        }
    }

    const fn blocked(gate: AutomationCoreRuntimeGate, reason: AutomationCoreRuntimeReason) -> Self {
        Self {
            gate,
            status: AutomationCoreRuntimeGateStatus::Blocked,
            reason: Some(reason),
        }
    }

    const fn unproven(
        gate: AutomationCoreRuntimeGate,
        reason: AutomationCoreRuntimeReason,
    ) -> Self {
        Self {
            gate,
            status: AutomationCoreRuntimeGateStatus::Unproven,
            reason: Some(reason),
        }
    }

    #[cfg(test)]
    pub(crate) const fn gate(self) -> AutomationCoreRuntimeGate {
        self.gate
    }

    #[cfg(test)]
    pub(crate) const fn status(self) -> AutomationCoreRuntimeGateStatus {
        self.status
    }

    #[cfg(test)]
    pub(crate) const fn reason(self) -> Option<AutomationCoreRuntimeReason> {
        self.reason
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AutomationCoreRuntimeAssessment {
    policy_revision: u32,
    assessed_at: SystemTime,
    gates: [AutomationCoreRuntimeGateAssessment; 4],
}

impl AutomationCoreRuntimeAssessment {
    #[cfg(test)]
    pub(crate) const fn policy_revision(&self) -> u32 {
        self.policy_revision
    }

    #[cfg(test)]
    pub(crate) const fn assessed_at(&self) -> SystemTime {
        self.assessed_at
    }

    #[cfg(test)]
    pub(crate) fn gates(&self) -> &[AutomationCoreRuntimeGateAssessment; 4] {
        &self.gates
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AutomationCoreRuntimeLocalObservation {
    pub(super) lifecycle: EngineLifecycle,
    pub(super) scan_work_active: bool,
    pub(super) cleanup_work_active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "unavailable and cross-platform identity states remain explicit fail-closed facts"
    )
)]
pub(super) enum AutomationCoreRuntimeIdentityObservation {
    CurrentUser,
    Unavailable,
    Privileged,
    UnsupportedPlatform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AutomationCoreRuntimeStoreObservation {
    Observed(StoredAutomationRuntimeObservation),
    BudgetExceeded,
    Unavailable,
}

pub(super) fn observe_runtime_identity() -> AutomationCoreRuntimeIdentityObservation {
    #[cfg(target_os = "macos")]
    {
        let real = nix::unistd::getuid();
        let effective = nix::unistd::geteuid();
        let real_group = nix::unistd::getgid();
        let effective_group = nix::unistd::getegid();
        classify_macos_runtime_identity(
            macos_process_is_set_id_tainted(),
            real == effective,
            effective.is_root(),
            real_group == effective_group,
            effective_group.as_raw() == 0,
            macos_supplementary_root_group(),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        AutomationCoreRuntimeIdentityObservation::UnsupportedPlatform
    }
}

#[cfg(target_os = "macos")]
pub(super) fn macos_process_is_set_id_tainted() -> bool {
    // SAFETY: `issetugid` takes no pointers and only reads the kernel's
    // immutable process taint flag for the current process.
    unsafe { nix::libc::issetugid() != 0 }
}

#[cfg(target_os = "macos")]
pub(super) fn macos_supplementary_root_group() -> Option<bool> {
    const MAX_SUPPLEMENTARY_GROUPS: usize = 1_024;
    // SAFETY: a zero-sized `getgroups` call accepts a null output pointer and
    // returns only the required element count.
    let count = unsafe { nix::libc::getgroups(0, std::ptr::null_mut()) };
    let count = usize::try_from(count).ok()?;
    if count > MAX_SUPPLEMENTARY_GROUPS {
        return None;
    }
    let mut groups: Vec<nix::libc::gid_t> = vec![0; count];
    let output = if count == 0 {
        std::ptr::null_mut()
    } else {
        groups.as_mut_ptr()
    };
    // SAFETY: `output` is null for zero elements or points to `count`
    // initialized `gid_t` slots retained for the duration of the call.
    let count_arg = i32::try_from(count).ok()?;
    let observed = unsafe { nix::libc::getgroups(count_arg, output) };
    let observed = usize::try_from(observed).ok()?;
    if observed > count {
        return None;
    }
    Some(groups[..observed].contains(&0))
}

#[cfg(any(test, target_os = "macos"))]
fn classify_macos_runtime_identity(
    set_id_tainted: bool,
    user_ids_match: bool,
    effective_user_is_root: bool,
    group_ids_match: bool,
    effective_group_is_root: bool,
    supplementary_root: Option<bool>,
) -> AutomationCoreRuntimeIdentityObservation {
    let Some(supplementary_root) = supplementary_root else {
        return AutomationCoreRuntimeIdentityObservation::Unavailable;
    };
    if set_id_tainted
        || !user_ids_match
        || effective_user_is_root
        || !group_ids_match
        || effective_group_is_root
        || supplementary_root
    {
        AutomationCoreRuntimeIdentityObservation::Privileged
    } else {
        AutomationCoreRuntimeIdentityObservation::CurrentUser
    }
}

pub(super) fn assess_automation_core_runtime(
    assessed_at: SystemTime,
    before: Option<AutomationCoreRuntimeLocalObservation>,
    after: Option<AutomationCoreRuntimeLocalObservation>,
    identity: AutomationCoreRuntimeIdentityObservation,
    retained_scan_admission_witness: bool,
    store: AutomationCoreRuntimeStoreObservation,
) -> AutomationCoreRuntimeAssessment {
    let lifecycle = if before.zip(after).is_some_and(|(before, after)| {
        before.lifecycle == EngineLifecycle::Open && after.lifecycle == EngineLifecycle::Open
    }) {
        AutomationCoreRuntimeGateAssessment::passed(AutomationCoreRuntimeGate::EngineLifecycle)
    } else if before.is_some_and(|observation| observation.lifecycle != EngineLifecycle::Open)
        || after.is_some_and(|observation| observation.lifecycle != EngineLifecycle::Open)
    {
        AutomationCoreRuntimeGateAssessment::blocked(
            AutomationCoreRuntimeGate::EngineLifecycle,
            AutomationCoreRuntimeReason::EngineNotOpen,
        )
    } else {
        AutomationCoreRuntimeGateAssessment::unproven(
            AutomationCoreRuntimeGate::EngineLifecycle,
            AutomationCoreRuntimeReason::LocalObservationUnavailable,
        )
    };
    let identity = match identity {
        AutomationCoreRuntimeIdentityObservation::CurrentUser => {
            AutomationCoreRuntimeGateAssessment::passed(AutomationCoreRuntimeGate::RuntimeIdentity)
        }
        AutomationCoreRuntimeIdentityObservation::Unavailable => {
            AutomationCoreRuntimeGateAssessment::unproven(
                AutomationCoreRuntimeGate::RuntimeIdentity,
                AutomationCoreRuntimeReason::RuntimeIdentityUnavailable,
            )
        }
        AutomationCoreRuntimeIdentityObservation::Privileged => {
            AutomationCoreRuntimeGateAssessment::blocked(
                AutomationCoreRuntimeGate::RuntimeIdentity,
                AutomationCoreRuntimeReason::RuntimePrivileged,
            )
        }
        AutomationCoreRuntimeIdentityObservation::UnsupportedPlatform => {
            AutomationCoreRuntimeGateAssessment::blocked(
                AutomationCoreRuntimeGate::RuntimeIdentity,
                AutomationCoreRuntimeReason::RuntimePlatformUnsupported,
            )
        }
    };
    let local_observation_complete = before.is_some() && after.is_some();
    let local_scan_active = before.is_some_and(|observation| observation.scan_work_active)
        || after.is_some_and(|observation| observation.scan_work_active);
    let local_cleanup_active = before.is_some_and(|observation| observation.cleanup_work_active)
        || after.is_some_and(|observation| observation.cleanup_work_active);
    let (scan, cleanup) = match store {
        AutomationCoreRuntimeStoreObservation::Observed(observation) => {
            let scan = if local_scan_active {
                AutomationCoreRuntimeGateAssessment::blocked(
                    AutomationCoreRuntimeGate::ScanWork,
                    AutomationCoreRuntimeReason::ScanWorkActive,
                )
            } else if !local_observation_complete {
                AutomationCoreRuntimeGateAssessment::unproven(
                    AutomationCoreRuntimeGate::ScanWork,
                    AutomationCoreRuntimeReason::LocalObservationUnavailable,
                )
            } else if !retained_scan_admission_witness || observation.scan_work_unresolved() {
                AutomationCoreRuntimeGateAssessment::unproven(
                    AutomationCoreRuntimeGate::ScanWork,
                    AutomationCoreRuntimeReason::ScanWorkUnresolved,
                )
            } else {
                AutomationCoreRuntimeGateAssessment::passed(AutomationCoreRuntimeGate::ScanWork)
            };
            let cleanup = if local_cleanup_active || observation.cleanup_work_active() {
                AutomationCoreRuntimeGateAssessment::blocked(
                    AutomationCoreRuntimeGate::CleanupWork,
                    AutomationCoreRuntimeReason::CleanupWorkActive,
                )
            } else if !local_observation_complete {
                AutomationCoreRuntimeGateAssessment::unproven(
                    AutomationCoreRuntimeGate::CleanupWork,
                    AutomationCoreRuntimeReason::LocalObservationUnavailable,
                )
            } else if observation.cleanup_work_unresolved() {
                AutomationCoreRuntimeGateAssessment::unproven(
                    AutomationCoreRuntimeGate::CleanupWork,
                    AutomationCoreRuntimeReason::CleanupWorkUnavailable,
                )
            } else {
                AutomationCoreRuntimeGateAssessment::passed(AutomationCoreRuntimeGate::CleanupWork)
            };
            (scan, cleanup)
        }
        AutomationCoreRuntimeStoreObservation::BudgetExceeded => (
            if local_scan_active {
                AutomationCoreRuntimeGateAssessment::blocked(
                    AutomationCoreRuntimeGate::ScanWork,
                    AutomationCoreRuntimeReason::ScanWorkActive,
                )
            } else {
                AutomationCoreRuntimeGateAssessment::unproven(
                    AutomationCoreRuntimeGate::ScanWork,
                    AutomationCoreRuntimeReason::ObservationBudgetExceeded,
                )
            },
            if local_cleanup_active {
                AutomationCoreRuntimeGateAssessment::blocked(
                    AutomationCoreRuntimeGate::CleanupWork,
                    AutomationCoreRuntimeReason::CleanupWorkActive,
                )
            } else {
                AutomationCoreRuntimeGateAssessment::unproven(
                    AutomationCoreRuntimeGate::CleanupWork,
                    AutomationCoreRuntimeReason::ObservationBudgetExceeded,
                )
            },
        ),
        AutomationCoreRuntimeStoreObservation::Unavailable => (
            if local_scan_active {
                AutomationCoreRuntimeGateAssessment::blocked(
                    AutomationCoreRuntimeGate::ScanWork,
                    AutomationCoreRuntimeReason::ScanWorkActive,
                )
            } else {
                AutomationCoreRuntimeGateAssessment::unproven(
                    AutomationCoreRuntimeGate::ScanWork,
                    AutomationCoreRuntimeReason::ScanWorkUnresolved,
                )
            },
            if local_cleanup_active {
                AutomationCoreRuntimeGateAssessment::blocked(
                    AutomationCoreRuntimeGate::CleanupWork,
                    AutomationCoreRuntimeReason::CleanupWorkActive,
                )
            } else {
                AutomationCoreRuntimeGateAssessment::unproven(
                    AutomationCoreRuntimeGate::CleanupWork,
                    AutomationCoreRuntimeReason::CleanupWorkUnavailable,
                )
            },
        ),
    };

    AutomationCoreRuntimeAssessment {
        policy_revision: AUTOMATION_CORE_RUNTIME_POLICY_REVISION,
        assessed_at,
        gates: [lifecycle, identity, scan, cleanup],
    }
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use super::*;

    fn local() -> AutomationCoreRuntimeLocalObservation {
        AutomationCoreRuntimeLocalObservation {
            lifecycle: EngineLifecycle::Open,
            scan_work_active: false,
            cleanup_work_active: false,
        }
    }

    fn statuses(
        assessment: &AutomationCoreRuntimeAssessment,
    ) -> [AutomationCoreRuntimeGateStatus; 4] {
        assessment
            .gates
            .map(AutomationCoreRuntimeGateAssessment::status)
    }

    #[test]
    fn clear_runtime_is_path_free_and_passes_exactly_four_ordered_gates() {
        let assessment = assess_automation_core_runtime(
            UNIX_EPOCH,
            Some(local()),
            Some(local()),
            AutomationCoreRuntimeIdentityObservation::CurrentUser,
            true,
            AutomationCoreRuntimeStoreObservation::Observed(
                StoredAutomationRuntimeObservation::clear_for_test(),
            ),
        );
        assert_eq!(assessment.policy_revision(), 1);
        assert_eq!(assessment.assessed_at(), UNIX_EPOCH);
        assert_eq!(
            statuses(&assessment),
            [AutomationCoreRuntimeGateStatus::Passed; 4]
        );
        assert_eq!(
            assessment
                .gates
                .map(AutomationCoreRuntimeGateAssessment::gate),
            [
                AutomationCoreRuntimeGate::EngineLifecycle,
                AutomationCoreRuntimeGate::RuntimeIdentity,
                AutomationCoreRuntimeGate::ScanWork,
                AutomationCoreRuntimeGate::CleanupWork,
            ]
        );
        assert!(assessment.gates.iter().all(|gate| gate.reason().is_none()));
    }

    #[test]
    fn lifecycle_and_local_work_block_without_softening_missing_store_facts() {
        let before = AutomationCoreRuntimeLocalObservation {
            lifecycle: EngineLifecycle::Closing,
            scan_work_active: true,
            cleanup_work_active: true,
        };
        let assessment = assess_automation_core_runtime(
            UNIX_EPOCH,
            Some(before),
            Some(local()),
            AutomationCoreRuntimeIdentityObservation::CurrentUser,
            false,
            AutomationCoreRuntimeStoreObservation::Unavailable,
        );
        assert_eq!(
            assessment
                .gates
                .map(AutomationCoreRuntimeGateAssessment::reason),
            [
                Some(AutomationCoreRuntimeReason::EngineNotOpen),
                None,
                Some(AutomationCoreRuntimeReason::ScanWorkActive),
                Some(AutomationCoreRuntimeReason::CleanupWorkActive),
            ]
        );
    }

    #[test]
    fn unavailable_privileged_and_unsupported_identity_never_pass() {
        for (identity, status, reason) in [
            (
                AutomationCoreRuntimeIdentityObservation::Unavailable,
                AutomationCoreRuntimeGateStatus::Unproven,
                AutomationCoreRuntimeReason::RuntimeIdentityUnavailable,
            ),
            (
                AutomationCoreRuntimeIdentityObservation::Privileged,
                AutomationCoreRuntimeGateStatus::Blocked,
                AutomationCoreRuntimeReason::RuntimePrivileged,
            ),
            (
                AutomationCoreRuntimeIdentityObservation::UnsupportedPlatform,
                AutomationCoreRuntimeGateStatus::Blocked,
                AutomationCoreRuntimeReason::RuntimePlatformUnsupported,
            ),
        ] {
            let assessment = assess_automation_core_runtime(
                UNIX_EPOCH,
                Some(local()),
                Some(local()),
                identity,
                true,
                AutomationCoreRuntimeStoreObservation::Observed(
                    StoredAutomationRuntimeObservation::clear_for_test(),
                ),
            );
            assert_eq!(assessment.gates[1].status(), status);
            assert_eq!(assessment.gates[1].reason(), Some(reason));
        }
    }

    #[test]
    fn macos_identity_facts_reject_set_id_and_supplementary_root_authority() {
        assert_eq!(
            classify_macos_runtime_identity(true, true, false, true, false, Some(false)),
            AutomationCoreRuntimeIdentityObservation::Privileged
        );
        assert_eq!(
            classify_macos_runtime_identity(false, true, false, true, false, Some(true)),
            AutomationCoreRuntimeIdentityObservation::Privileged
        );
        assert_eq!(
            classify_macos_runtime_identity(false, true, false, true, false, None),
            AutomationCoreRuntimeIdentityObservation::Unavailable
        );
        assert_eq!(
            classify_macos_runtime_identity(false, true, false, true, false, Some(false)),
            AutomationCoreRuntimeIdentityObservation::CurrentUser
        );
    }

    #[test]
    fn durable_uncertainty_and_budget_exhaustion_are_never_clear() {
        let unresolved = assess_automation_core_runtime(
            UNIX_EPOCH,
            Some(local()),
            Some(local()),
            AutomationCoreRuntimeIdentityObservation::CurrentUser,
            true,
            AutomationCoreRuntimeStoreObservation::Observed(
                StoredAutomationRuntimeObservation::unresolved_for_test(),
            ),
        );
        assert_eq!(
            unresolved.gates[2].status(),
            AutomationCoreRuntimeGateStatus::Unproven
        );
        assert_eq!(
            unresolved.gates[3].status(),
            AutomationCoreRuntimeGateStatus::Unproven
        );

        let budget = assess_automation_core_runtime(
            UNIX_EPOCH,
            Some(local()),
            Some(local()),
            AutomationCoreRuntimeIdentityObservation::CurrentUser,
            false,
            AutomationCoreRuntimeStoreObservation::BudgetExceeded,
        );
        assert_eq!(
            budget.gates[2].reason(),
            Some(AutomationCoreRuntimeReason::ObservationBudgetExceeded)
        );
        assert_eq!(
            budget.gates[3].reason(),
            Some(AutomationCoreRuntimeReason::ObservationBudgetExceeded)
        );
    }
}
