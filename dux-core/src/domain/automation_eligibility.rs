//! Deterministic, path-free policy evaluation for cleanup automation.
//!
//! This module returns an observation only. Even an `Eligible` result carries
//! no candidate, path, plan, approval, journal lease, task, or effect
//! capability. A scheduler must obtain sealed current facts inside the core,
//! create a fresh plan, and independently revalidate it before every run.

use std::time::{Duration, SystemTime};

use super::{
    AutomationScheduleDraftConfig, AutomationScheduleScope, CLEANUP_PLAN_VALIDITY, CandidateAction,
    Rule, RuleRef, SafetyTier,
};

pub const AUTOMATION_ELIGIBILITY_POLICY_REVISION: u32 = 1;
pub const AUTOMATION_REQUIRED_MANUAL_SUCCESSES: u16 = 2;
pub const AUTOMATION_REQUIRED_RECENT_RUNS: usize = 2;
pub const AUTOMATION_CURRENT_EVIDENCE_MAX_AGE: Duration = CLEANUP_PLAN_VALIDITY;
pub const MAX_AUTOMATION_DRAFT_POLICY_REASONS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AutomationDraftPolicyReason {
    ScopeRuleNotShipped,
    ScopeRuleRevisionNotCurrent,
    ScopeRuleNotSafeRegenerable,
    ScopeRuleActionNotPermanentSafe,
    ScopeRuleNotMarkedScheduleEligible,
    CategoryHasNoScheduleEligibleRules,
    AllScheduleEligibleRulesExcluded,
    ExclusionRuleNotShipped,
    ExclusionRuleRevisionNotCurrent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AutomationDraftPolicyPreflight {
    pub(crate) included_rule_count: u16,
    pub(crate) reasons: Vec<AutomationDraftPolicyReason>,
}

/// Evaluate only immutable shipped policy and exact exclusions for a draft.
/// Runtime evidence is deliberately outside this preflight.
pub(crate) fn assess_automation_draft_policy<'a>(
    config: &AutomationScheduleDraftConfig,
    rules: impl Iterator<Item = &'a Rule>,
) -> AutomationDraftPolicyPreflight {
    let rules = rules.collect::<Vec<_>>();
    let mut reasons = Vec::new();
    let included_rule_count = match config.scope() {
        AutomationScheduleScope::Rule(reference) => {
            let current = rules
                .iter()
                .copied()
                .find(|rule| rule.reference().id() == reference.id());
            let Some(rule) = current else {
                reasons.push(AutomationDraftPolicyReason::ScopeRuleNotShipped);
                return AutomationDraftPolicyPreflight {
                    included_rule_count: 0,
                    reasons,
                };
            };
            if rule.reference() != reference {
                reasons.push(AutomationDraftPolicyReason::ScopeRuleRevisionNotCurrent);
            } else {
                if rule.safety() != SafetyTier::SafeRegenerable {
                    reasons.push(AutomationDraftPolicyReason::ScopeRuleNotSafeRegenerable);
                }
                if rule.action() != CandidateAction::RemoveKnownRegenerableContents {
                    reasons.push(AutomationDraftPolicyReason::ScopeRuleActionNotPermanentSafe);
                }
                if !rule.schedule_eligible() {
                    reasons.push(AutomationDraftPolicyReason::ScopeRuleNotMarkedScheduleEligible);
                }
            }
            u16::from(reasons.is_empty())
        }
        AutomationScheduleScope::Category(category) => {
            for exclusion in config.excluded_rules() {
                match rules
                    .iter()
                    .copied()
                    .find(|rule| rule.reference().id() == exclusion.id())
                {
                    None => reasons.push(AutomationDraftPolicyReason::ExclusionRuleNotShipped),
                    Some(rule) if rule.reference() != exclusion => {
                        reasons.push(AutomationDraftPolicyReason::ExclusionRuleRevisionNotCurrent)
                    }
                    Some(_) => {}
                }
            }
            let eligible = rules
                .iter()
                .copied()
                .filter(|rule| {
                    rule.category() == *category
                        && rule.safety() == SafetyTier::SafeRegenerable
                        && rule.action() == CandidateAction::RemoveKnownRegenerableContents
                        && rule.schedule_eligible()
                })
                .collect::<Vec<_>>();
            if eligible.is_empty() {
                reasons.push(AutomationDraftPolicyReason::CategoryHasNoScheduleEligibleRules);
                0
            } else {
                let included = eligible
                    .iter()
                    .filter(|rule| !config.excluded_rules().contains(rule.reference()))
                    .count();
                if included == 0 {
                    reasons.push(AutomationDraftPolicyReason::AllScheduleEligibleRulesExcluded);
                }
                u16::try_from(included).unwrap_or(0)
            }
        }
    };
    reasons.sort_unstable();
    reasons.dedup();
    debug_assert!(reasons.len() <= MAX_AUTOMATION_DRAFT_POLICY_REASONS);
    AutomationDraftPolicyPreflight {
        included_rule_count,
        reasons,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AutomationEligibilityGate {
    ShippedPolicy,
    ManualHistory,
    RecentRunSafety,
    CurrentCandidateAge,
    CurrentCandidateSize,
    ActivityGuard,
    EvidenceFreshness,
    ExecutionPrivilege,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationEligibilityGateStatus {
    Passed,
    NotApplicable,
    Blocked,
    Unproven,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AutomationEligibilityReason {
    RuleNotShipped,
    RuleRevisionNotCurrent,
    RuleNotSafeRegenerable,
    RuleActionNotPermanentSafe,
    RuleNotMarkedScheduleEligible,
    RuleNotInScheduleScope,
    RuleExcludedBySchedule,
    ManualHistoryUnavailable,
    InsufficientManualSuccesses,
    RecentRunHistoryUnavailable,
    RecentRunFailed,
    RecentRunProtectedDescendant,
    CurrentCandidateUnavailable,
    CurrentCandidateRuleMismatch,
    CurrentCandidateAgeUnavailable,
    CurrentCandidateBelowMinimumAge,
    CurrentCandidateBelowMinimumSize,
    ActivityEvidenceUnavailable,
    ActivityDetected,
    ActivityEvidenceFutureDated,
    ActivityEvidenceStale,
    ScanCoverageIncomplete,
    EvaluationNotCurrent,
    CurrentProtectedDescendant,
    CurrentValidationUnavailable,
    CurrentValidationBlocked,
    EvidenceTimestampInconsistent,
    EvidenceFutureDated,
    EvidenceStale,
    RuntimeIdentityUnavailable,
    RuntimePrivileged,
    RuntimePlatformUnsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutomationEligibilityGateAssessment {
    gate: AutomationEligibilityGate,
    status: AutomationEligibilityGateStatus,
    reason: Option<AutomationEligibilityReason>,
}

impl AutomationEligibilityGateAssessment {
    const fn passed(gate: AutomationEligibilityGate) -> Self {
        Self {
            gate,
            status: AutomationEligibilityGateStatus::Passed,
            reason: None,
        }
    }

    const fn not_applicable(gate: AutomationEligibilityGate) -> Self {
        Self {
            gate,
            status: AutomationEligibilityGateStatus::NotApplicable,
            reason: None,
        }
    }

    const fn blocked(gate: AutomationEligibilityGate, reason: AutomationEligibilityReason) -> Self {
        Self {
            gate,
            status: AutomationEligibilityGateStatus::Blocked,
            reason: Some(reason),
        }
    }

    const fn unproven(
        gate: AutomationEligibilityGate,
        reason: AutomationEligibilityReason,
    ) -> Self {
        Self {
            gate,
            status: AutomationEligibilityGateStatus::Unproven,
            reason: Some(reason),
        }
    }

    pub const fn gate(self) -> AutomationEligibilityGate {
        self.gate
    }

    pub const fn status(self) -> AutomationEligibilityGateStatus {
        self.status
    }

    pub const fn reason(self) -> Option<AutomationEligibilityReason> {
        self.reason
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationEligibilityDecision {
    Eligible,
    Ineligible,
    Indeterminate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationEligibilityAssessment {
    policy_revision: u32,
    decision: AutomationEligibilityDecision,
    gates: [AutomationEligibilityGateAssessment; 8],
}

impl AutomationEligibilityAssessment {
    pub const fn policy_revision(&self) -> u32 {
        self.policy_revision
    }

    pub const fn decision(&self) -> AutomationEligibilityDecision {
        self.decision
    }

    pub fn gates(&self) -> &[AutomationEligibilityGateAssessment; 8] {
        &self.gates
    }

    pub const fn is_eligible(&self) -> bool {
        matches!(self.decision, AutomationEligibilityDecision::Eligible)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationRecentRunEvidence {
    Succeeded,
    Failed,
    ProtectedDescendant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutomationManualHistoryEvidence {
    Unavailable,
    Observed {
        successful_manual_runs: u16,
        /// Newest matching manual attempts first. Only the first two are
        /// relevant; retaining more makes test and persistence adapters simple.
        recent_runs: Vec<AutomationRecentRunEvidence>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationCurrentValidationEvidence {
    Clear,
    Blocked,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationCurrentCandidateEvidence {
    pub rule: RuleRef,
    pub estimated_bytes: u64,
    pub newest_mtime: Option<SystemTime>,
    pub scan_completed_at: SystemTime,
    pub evaluated_at: SystemTime,
    pub scan_coverage_complete: bool,
    pub evaluation_matches_current_policy: bool,
    pub protected_descendant_observed: bool,
    pub current_validation: AutomationCurrentValidationEvidence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationActivityEvidence {
    NotRequired,
    Unavailable,
    Active { observed_at: SystemTime },
    Inactive { observed_at: SystemTime },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomationRuntimeIdentityEvidence {
    CurrentUser,
    Unavailable,
    Privileged,
    UnsupportedPlatform,
}

/// Complete path-free facts for the eight eligibility gates.
///
/// Callers may construct facts for diagnostics and tests, but the result is
/// deliberately non-authoritative. No cleanup API accepts this type.
pub struct AutomationEligibilityInput<'a> {
    pub requested_rule: &'a RuleRef,
    pub shipped_rule: Option<&'a Rule>,
    pub schedule: &'a AutomationScheduleDraftConfig,
    pub history: &'a AutomationManualHistoryEvidence,
    pub current_candidate: Option<&'a AutomationCurrentCandidateEvidence>,
    pub activity: AutomationActivityEvidence,
    pub runtime_identity: AutomationRuntimeIdentityEvidence,
    pub assessed_at: SystemTime,
}

pub fn assess_automation_eligibility(
    input: AutomationEligibilityInput<'_>,
) -> AutomationEligibilityAssessment {
    let gates = [
        assess_policy(input.requested_rule, input.shipped_rule, input.schedule),
        assess_manual_history(input.history),
        assess_recent_runs(input.history),
        assess_candidate_age(
            input.requested_rule,
            input.shipped_rule,
            input.schedule,
            input.current_candidate,
            input.assessed_at,
        ),
        assess_candidate_size(
            input.requested_rule,
            input.shipped_rule,
            input.schedule,
            input.current_candidate,
        ),
        assess_activity(
            input.requested_rule,
            input.shipped_rule,
            input.activity,
            input.assessed_at,
        ),
        assess_freshness(
            input.requested_rule,
            input.current_candidate,
            input.assessed_at,
        ),
        assess_runtime(input.runtime_identity),
    ];
    debug_assert!(
        gates
            .iter()
            .zip([
                AutomationEligibilityGate::ShippedPolicy,
                AutomationEligibilityGate::ManualHistory,
                AutomationEligibilityGate::RecentRunSafety,
                AutomationEligibilityGate::CurrentCandidateAge,
                AutomationEligibilityGate::CurrentCandidateSize,
                AutomationEligibilityGate::ActivityGuard,
                AutomationEligibilityGate::EvidenceFreshness,
                AutomationEligibilityGate::ExecutionPrivilege,
            ])
            .all(|(assessment, gate)| assessment.gate == gate)
    );
    let decision = if gates
        .iter()
        .any(|gate| gate.status == AutomationEligibilityGateStatus::Blocked)
    {
        AutomationEligibilityDecision::Ineligible
    } else if gates
        .iter()
        .any(|gate| gate.status == AutomationEligibilityGateStatus::Unproven)
    {
        AutomationEligibilityDecision::Indeterminate
    } else {
        AutomationEligibilityDecision::Eligible
    };
    AutomationEligibilityAssessment {
        policy_revision: AUTOMATION_ELIGIBILITY_POLICY_REVISION,
        decision,
        gates,
    }
}

fn assess_policy(
    requested: &RuleRef,
    shipped: Option<&Rule>,
    schedule: &AutomationScheduleDraftConfig,
) -> AutomationEligibilityGateAssessment {
    let gate = AutomationEligibilityGate::ShippedPolicy;
    let Some(rule) = shipped else {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::RuleNotShipped,
        );
    };
    if rule.reference() != requested {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::RuleRevisionNotCurrent,
        );
    }
    match schedule.scope() {
        AutomationScheduleScope::Rule(scope_rule) if scope_rule != requested => {
            return AutomationEligibilityGateAssessment::blocked(
                gate,
                AutomationEligibilityReason::RuleNotInScheduleScope,
            );
        }
        AutomationScheduleScope::Category(category) if rule.category() != *category => {
            return AutomationEligibilityGateAssessment::blocked(
                gate,
                AutomationEligibilityReason::RuleNotInScheduleScope,
            );
        }
        AutomationScheduleScope::Category(_) if schedule.excluded_rules().contains(requested) => {
            return AutomationEligibilityGateAssessment::blocked(
                gate,
                AutomationEligibilityReason::RuleExcludedBySchedule,
            );
        }
        AutomationScheduleScope::Rule(_) | AutomationScheduleScope::Category(_) => {}
    }
    if rule.safety() != SafetyTier::SafeRegenerable {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::RuleNotSafeRegenerable,
        );
    }
    if rule.action() != CandidateAction::RemoveKnownRegenerableContents {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::RuleActionNotPermanentSafe,
        );
    }
    if !rule.schedule_eligible() {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::RuleNotMarkedScheduleEligible,
        );
    }
    AutomationEligibilityGateAssessment::passed(gate)
}

fn assess_manual_history(
    history: &AutomationManualHistoryEvidence,
) -> AutomationEligibilityGateAssessment {
    let gate = AutomationEligibilityGate::ManualHistory;
    match history {
        AutomationManualHistoryEvidence::Unavailable => {
            AutomationEligibilityGateAssessment::unproven(
                gate,
                AutomationEligibilityReason::ManualHistoryUnavailable,
            )
        }
        AutomationManualHistoryEvidence::Observed {
            successful_manual_runs,
            ..
        } if *successful_manual_runs < AUTOMATION_REQUIRED_MANUAL_SUCCESSES => {
            AutomationEligibilityGateAssessment::blocked(
                gate,
                AutomationEligibilityReason::InsufficientManualSuccesses,
            )
        }
        AutomationManualHistoryEvidence::Observed { .. } => {
            AutomationEligibilityGateAssessment::passed(gate)
        }
    }
}

fn assess_recent_runs(
    history: &AutomationManualHistoryEvidence,
) -> AutomationEligibilityGateAssessment {
    let gate = AutomationEligibilityGate::RecentRunSafety;
    let AutomationManualHistoryEvidence::Observed { recent_runs, .. } = history else {
        return AutomationEligibilityGateAssessment::unproven(
            gate,
            AutomationEligibilityReason::RecentRunHistoryUnavailable,
        );
    };
    if recent_runs.len() < AUTOMATION_REQUIRED_RECENT_RUNS {
        return AutomationEligibilityGateAssessment::unproven(
            gate,
            AutomationEligibilityReason::RecentRunHistoryUnavailable,
        );
    }
    if recent_runs
        .iter()
        .take(AUTOMATION_REQUIRED_RECENT_RUNS)
        .any(|run| matches!(run, AutomationRecentRunEvidence::ProtectedDescendant))
    {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::RecentRunProtectedDescendant,
        );
    }
    if recent_runs
        .iter()
        .take(AUTOMATION_REQUIRED_RECENT_RUNS)
        .any(|run| matches!(run, AutomationRecentRunEvidence::Failed))
    {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::RecentRunFailed,
        );
    }
    AutomationEligibilityGateAssessment::passed(gate)
}

fn assess_candidate_age(
    requested: &RuleRef,
    shipped: Option<&Rule>,
    schedule: &AutomationScheduleDraftConfig,
    candidate: Option<&AutomationCurrentCandidateEvidence>,
    assessed_at: SystemTime,
) -> AutomationEligibilityGateAssessment {
    let gate = AutomationEligibilityGate::CurrentCandidateAge;
    let Some(candidate) = candidate else {
        return AutomationEligibilityGateAssessment::unproven(
            gate,
            AutomationEligibilityReason::CurrentCandidateUnavailable,
        );
    };
    if &candidate.rule != requested {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::CurrentCandidateRuleMismatch,
        );
    }
    let Some(newest_mtime) = candidate.newest_mtime else {
        return AutomationEligibilityGateAssessment::unproven(
            gate,
            AutomationEligibilityReason::CurrentCandidateAgeUnavailable,
        );
    };
    let rule_minimum = shipped
        .and_then(|rule| (rule.reference() == requested).then_some(rule))
        .and_then(|rule| rule.guards().minimum_age())
        .unwrap_or(Duration::ZERO);
    let minimum_age = schedule.minimum_age().max(rule_minimum);
    if !assessed_at
        .duration_since(newest_mtime)
        .is_ok_and(|age| age >= minimum_age)
    {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::CurrentCandidateBelowMinimumAge,
        );
    }
    AutomationEligibilityGateAssessment::passed(gate)
}

fn assess_candidate_size(
    requested: &RuleRef,
    shipped: Option<&Rule>,
    schedule: &AutomationScheduleDraftConfig,
    candidate: Option<&AutomationCurrentCandidateEvidence>,
) -> AutomationEligibilityGateAssessment {
    let gate = AutomationEligibilityGate::CurrentCandidateSize;
    let Some(candidate) = candidate else {
        return AutomationEligibilityGateAssessment::unproven(
            gate,
            AutomationEligibilityReason::CurrentCandidateUnavailable,
        );
    };
    if &candidate.rule != requested {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::CurrentCandidateRuleMismatch,
        );
    }
    let rule_minimum = shipped
        .and_then(|rule| (rule.reference() == requested).then_some(rule))
        .map_or(0, |rule| rule.guards().minimum_bytes());
    let minimum_bytes = schedule.minimum_reclaimable_bytes().max(rule_minimum);
    if candidate.estimated_bytes < minimum_bytes {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::CurrentCandidateBelowMinimumSize,
        );
    }
    AutomationEligibilityGateAssessment::passed(gate)
}

fn assess_activity(
    requested: &RuleRef,
    shipped: Option<&Rule>,
    activity: AutomationActivityEvidence,
    assessed_at: SystemTime,
) -> AutomationEligibilityGateAssessment {
    let gate = AutomationEligibilityGate::ActivityGuard;
    let Some(rule) = shipped.filter(|rule| rule.reference() == requested) else {
        return AutomationEligibilityGateAssessment::unproven(
            gate,
            AutomationEligibilityReason::ActivityEvidenceUnavailable,
        );
    };
    if rule.guards().inactive_processes().is_empty() {
        return AutomationEligibilityGateAssessment::not_applicable(gate);
    }
    let observed_at = match activity {
        AutomationActivityEvidence::NotRequired | AutomationActivityEvidence::Unavailable => {
            return AutomationEligibilityGateAssessment::unproven(
                gate,
                AutomationEligibilityReason::ActivityEvidenceUnavailable,
            );
        }
        AutomationActivityEvidence::Active { observed_at } => {
            if observed_at > assessed_at {
                return AutomationEligibilityGateAssessment::blocked(
                    gate,
                    AutomationEligibilityReason::ActivityEvidenceFutureDated,
                );
            }
            return AutomationEligibilityGateAssessment::blocked(
                gate,
                AutomationEligibilityReason::ActivityDetected,
            );
        }
        AutomationActivityEvidence::Inactive { observed_at } => observed_at,
    };
    match assessed_at.duration_since(observed_at) {
        Ok(age) if age <= AUTOMATION_CURRENT_EVIDENCE_MAX_AGE => {
            AutomationEligibilityGateAssessment::passed(gate)
        }
        Ok(_) => AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::ActivityEvidenceStale,
        ),
        Err(_) => AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::ActivityEvidenceFutureDated,
        ),
    }
}

fn assess_freshness(
    requested: &RuleRef,
    candidate: Option<&AutomationCurrentCandidateEvidence>,
    assessed_at: SystemTime,
) -> AutomationEligibilityGateAssessment {
    let gate = AutomationEligibilityGate::EvidenceFreshness;
    let Some(candidate) = candidate else {
        return AutomationEligibilityGateAssessment::unproven(
            gate,
            AutomationEligibilityReason::CurrentCandidateUnavailable,
        );
    };
    if &candidate.rule != requested {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::CurrentCandidateRuleMismatch,
        );
    }
    if !candidate.scan_coverage_complete {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::ScanCoverageIncomplete,
        );
    }
    if !candidate.evaluation_matches_current_policy {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::EvaluationNotCurrent,
        );
    }
    if candidate.protected_descendant_observed {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::CurrentProtectedDescendant,
        );
    }
    match candidate.current_validation {
        AutomationCurrentValidationEvidence::Unavailable => {
            return AutomationEligibilityGateAssessment::unproven(
                gate,
                AutomationEligibilityReason::CurrentValidationUnavailable,
            );
        }
        AutomationCurrentValidationEvidence::Blocked => {
            return AutomationEligibilityGateAssessment::blocked(
                gate,
                AutomationEligibilityReason::CurrentValidationBlocked,
            );
        }
        AutomationCurrentValidationEvidence::Clear => {}
    }
    if candidate.scan_completed_at > candidate.evaluated_at {
        return AutomationEligibilityGateAssessment::blocked(
            gate,
            AutomationEligibilityReason::EvidenceTimestampInconsistent,
        );
    }
    for timestamp in [candidate.scan_completed_at, candidate.evaluated_at] {
        match assessed_at.duration_since(timestamp) {
            Ok(age) if age <= AUTOMATION_CURRENT_EVIDENCE_MAX_AGE => {}
            Ok(_) => {
                return AutomationEligibilityGateAssessment::blocked(
                    gate,
                    AutomationEligibilityReason::EvidenceStale,
                );
            }
            Err(_) => {
                return AutomationEligibilityGateAssessment::blocked(
                    gate,
                    AutomationEligibilityReason::EvidenceFutureDated,
                );
            }
        }
    }
    AutomationEligibilityGateAssessment::passed(gate)
}

fn assess_runtime(
    runtime: AutomationRuntimeIdentityEvidence,
) -> AutomationEligibilityGateAssessment {
    let gate = AutomationEligibilityGate::ExecutionPrivilege;
    match runtime {
        AutomationRuntimeIdentityEvidence::CurrentUser => {
            AutomationEligibilityGateAssessment::passed(gate)
        }
        AutomationRuntimeIdentityEvidence::Unavailable => {
            AutomationEligibilityGateAssessment::unproven(
                gate,
                AutomationEligibilityReason::RuntimeIdentityUnavailable,
            )
        }
        AutomationRuntimeIdentityEvidence::Privileged => {
            AutomationEligibilityGateAssessment::blocked(
                gate,
                AutomationEligibilityReason::RuntimePrivileged,
            )
        }
        AutomationRuntimeIdentityEvidence::UnsupportedPlatform => {
            AutomationEligibilityGateAssessment::blocked(
                gate,
                AutomationEligibilityReason::RuntimePlatformUnsupported,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        ActivityGuard, AutomationConfirmationMode, AutomationScheduleCadence,
        AutomationScheduleScope, CandidateCategory, LocalizedTextKey, ProvenanceUrl,
        RuleDefinition, RuleGuards, RuleId, RuleMatcher, RuleMatcherDefinition, RuleRevision,
        RuleScope,
    };

    fn rule(schedule_eligible: bool, minimum_age: Duration, minimum_bytes: u64) -> Rule {
        rule_with_activity_guard(schedule_eligible, minimum_age, minimum_bytes, true)
    }

    fn rule_with_activity_guard(
        schedule_eligible: bool,
        minimum_age: Duration,
        minimum_bytes: u64,
        activity_guard: bool,
    ) -> Rule {
        Rule::try_new(RuleDefinition {
            reference: RuleRef::new(
                RuleId::new("developer.fixture.cache").unwrap(),
                RuleRevision::new(1).unwrap(),
            ),
            title_key: LocalizedTextKey::new("rule.developer.fixture.cache.title").unwrap(),
            category: CandidateCategory::DeveloperArtifact,
            scope: RuleScope::SelectedScanRoot,
            matcher: RuleMatcher::try_new(RuleMatcherDefinition {
                path_component: Some("cache".to_owned()),
                required_ancestor_markers_any: Vec::new(),
                required_markers_all: Vec::new(),
                forbidden_markers_any: Vec::new(),
                exact_bundle_identifiers: Vec::new(),
                excluded_descendants: Vec::new(),
                protected_descendants: Vec::new(),
            })
            .unwrap(),
            guards: RuleGuards::try_new(
                (!minimum_age.is_zero()).then_some(minimum_age),
                minimum_bytes,
                activity_guard
                    .then(|| ActivityGuard::ProcessName("fixture".to_owned()))
                    .into_iter()
                    .collect(),
                false,
            )
            .unwrap(),
            safety: SafetyTier::SafeRegenerable,
            action: CandidateAction::RemoveKnownRegenerableContents,
            schedule_eligible,
            explanation_key: LocalizedTextKey::new("rule.developer.fixture.cache.explanation")
                .unwrap(),
            provenance: vec![ProvenanceUrl::new("https://example.com/cache").unwrap()],
        })
        .unwrap()
    }

    fn config(rule: &Rule, age: Duration, bytes: u64) -> AutomationScheduleDraftConfig {
        config_for_scope(
            AutomationScheduleScope::Rule(rule.reference().clone()),
            age,
            bytes,
            Vec::new(),
        )
    }

    fn config_for_scope(
        scope: AutomationScheduleScope,
        age: Duration,
        bytes: u64,
        excluded_rules: Vec<RuleRef>,
    ) -> AutomationScheduleDraftConfig {
        AutomationScheduleDraftConfig::try_new(
            scope,
            AutomationScheduleCadence::Monthly,
            age,
            bytes,
            10_000,
            excluded_rules,
            true,
            AutomationConfirmationMode::RequireConfirmation,
        )
        .unwrap()
    }

    fn candidate(rule: &Rule, now: SystemTime) -> AutomationCurrentCandidateEvidence {
        AutomationCurrentCandidateEvidence {
            rule: rule.reference().clone(),
            estimated_bytes: 1_000,
            newest_mtime: Some(now - Duration::from_secs(100)),
            scan_completed_at: now - Duration::from_secs(2),
            evaluated_at: now - Duration::from_secs(1),
            scan_coverage_complete: true,
            evaluation_matches_current_policy: true,
            protected_descendant_observed: false,
            current_validation: AutomationCurrentValidationEvidence::Clear,
        }
    }

    fn history() -> AutomationManualHistoryEvidence {
        AutomationManualHistoryEvidence::Observed {
            successful_manual_runs: 2,
            recent_runs: vec![
                AutomationRecentRunEvidence::Succeeded,
                AutomationRecentRunEvidence::Succeeded,
            ],
        }
    }

    #[test]
    fn draft_policy_preflight_requires_exact_current_schedule_safe_rule() {
        let eligible_rule = rule(true, Duration::ZERO, 0);
        let eligible_config = config(&eligible_rule, Duration::ZERO, 0);
        let eligible =
            assess_automation_draft_policy(&eligible_config, [&eligible_rule].into_iter());
        assert_eq!(eligible.included_rule_count, 1);
        assert!(eligible.reasons.is_empty());

        let ineligible_rule = rule(false, Duration::ZERO, 0);
        let ineligible_config = config(&ineligible_rule, Duration::ZERO, 0);
        let ineligible =
            assess_automation_draft_policy(&ineligible_config, [&ineligible_rule].into_iter());
        assert_eq!(ineligible.included_rule_count, 0);
        assert_eq!(
            ineligible.reasons,
            vec![AutomationDraftPolicyReason::ScopeRuleNotMarkedScheduleEligible]
        );

        let stale_reference = RuleRef::new(
            eligible_rule.reference().id().clone(),
            RuleRevision::new(2).unwrap(),
        );
        let stale_config = config_for_scope(
            AutomationScheduleScope::Rule(stale_reference),
            Duration::ZERO,
            0,
            Vec::new(),
        );
        let stale = assess_automation_draft_policy(&stale_config, [&eligible_rule].into_iter());
        assert_eq!(stale.included_rule_count, 0);
        assert_eq!(
            stale.reasons,
            vec![AutomationDraftPolicyReason::ScopeRuleRevisionNotCurrent]
        );
    }

    #[test]
    fn category_preflight_counts_only_included_rules_and_validates_exclusions() {
        let eligible_rule = rule(true, Duration::ZERO, 0);
        let included_config = config_for_scope(
            AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
            Duration::ZERO,
            0,
            Vec::new(),
        );
        let included =
            assess_automation_draft_policy(&included_config, [&eligible_rule].into_iter());
        assert_eq!(included.included_rule_count, 1);
        assert!(included.reasons.is_empty());

        let excluded_config = config_for_scope(
            AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
            Duration::ZERO,
            0,
            vec![eligible_rule.reference().clone()],
        );
        let excluded =
            assess_automation_draft_policy(&excluded_config, [&eligible_rule].into_iter());
        assert_eq!(excluded.included_rule_count, 0);
        assert_eq!(
            excluded.reasons,
            vec![AutomationDraftPolicyReason::AllScheduleEligibleRulesExcluded]
        );

        let stale_exclusion = RuleRef::new(
            eligible_rule.reference().id().clone(),
            RuleRevision::new(2).unwrap(),
        );
        let stale_config = config_for_scope(
            AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
            Duration::ZERO,
            0,
            vec![stale_exclusion],
        );
        let stale = assess_automation_draft_policy(&stale_config, [&eligible_rule].into_iter());
        assert_eq!(stale.included_rule_count, 1);
        assert_eq!(
            stale.reasons,
            vec![AutomationDraftPolicyReason::ExclusionRuleRevisionNotCurrent]
        );
    }

    #[test]
    fn all_eight_gates_are_required_for_eligibility() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let rule = rule(true, Duration::from_secs(50), 500);
        let config = config(&rule, Duration::from_secs(80), 750);
        let history = history();
        let candidate = candidate(&rule, now);
        let assessment = assess_automation_eligibility(AutomationEligibilityInput {
            requested_rule: rule.reference(),
            shipped_rule: Some(&rule),
            schedule: &config,
            history: &history,
            current_candidate: Some(&candidate),
            activity: AutomationActivityEvidence::Inactive {
                observed_at: now - Duration::from_secs(1),
            },
            runtime_identity: AutomationRuntimeIdentityEvidence::CurrentUser,
            assessed_at: now,
        });

        assert_eq!(assessment.policy_revision(), 1);
        assert_eq!(
            assessment.decision(),
            AutomationEligibilityDecision::Eligible
        );
        assert!(assessment.is_eligible());
        assert_eq!(assessment.gates().len(), 8);
        assert!(assessment.gates().iter().all(|gate| matches!(
            gate.status(),
            AutomationEligibilityGateStatus::Passed
                | AutomationEligibilityGateStatus::NotApplicable
        )));
    }

    #[test]
    fn absence_is_indeterminate_and_never_eligible() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let rule = rule(true, Duration::ZERO, 0);
        let config = config(&rule, Duration::ZERO, 0);
        let assessment = assess_automation_eligibility(AutomationEligibilityInput {
            requested_rule: rule.reference(),
            shipped_rule: Some(&rule),
            schedule: &config,
            history: &AutomationManualHistoryEvidence::Unavailable,
            current_candidate: None,
            activity: AutomationActivityEvidence::Unavailable,
            runtime_identity: AutomationRuntimeIdentityEvidence::Unavailable,
            assessed_at: now,
        });

        assert_eq!(
            assessment.decision(),
            AutomationEligibilityDecision::Indeterminate
        );
        assert!(!assessment.is_eligible());
        assert!(
            assessment
                .gates()
                .iter()
                .any(|gate| gate.status() == AutomationEligibilityGateStatus::Unproven)
        );
    }

    #[test]
    fn shipped_rule_determines_whether_activity_evidence_is_required() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let guarded_rule = rule(true, Duration::ZERO, 0);
        let guarded_config = config(&guarded_rule, Duration::ZERO, 0);
        let history = history();
        let guarded_candidate = candidate(&guarded_rule, now);
        let guarded = assess_automation_eligibility(AutomationEligibilityInput {
            requested_rule: guarded_rule.reference(),
            shipped_rule: Some(&guarded_rule),
            schedule: &guarded_config,
            history: &history,
            current_candidate: Some(&guarded_candidate),
            activity: AutomationActivityEvidence::NotRequired,
            runtime_identity: AutomationRuntimeIdentityEvidence::CurrentUser,
            assessed_at: now,
        });
        assert_eq!(
            guarded.decision(),
            AutomationEligibilityDecision::Indeterminate
        );
        assert_eq!(
            guarded.gates()[5].reason(),
            Some(AutomationEligibilityReason::ActivityEvidenceUnavailable)
        );

        let unguarded_rule = rule_with_activity_guard(true, Duration::ZERO, 0, false);
        let unguarded_config = config(&unguarded_rule, Duration::ZERO, 0);
        let unguarded_candidate = candidate(&unguarded_rule, now);
        let unguarded = assess_automation_eligibility(AutomationEligibilityInput {
            requested_rule: unguarded_rule.reference(),
            shipped_rule: Some(&unguarded_rule),
            schedule: &unguarded_config,
            history: &history,
            current_candidate: Some(&unguarded_candidate),
            activity: AutomationActivityEvidence::Unavailable,
            runtime_identity: AutomationRuntimeIdentityEvidence::CurrentUser,
            assessed_at: now,
        });
        assert_eq!(
            unguarded.decision(),
            AutomationEligibilityDecision::Eligible
        );
        assert_eq!(
            unguarded.gates()[5].status(),
            AutomationEligibilityGateStatus::NotApplicable
        );
    }

    #[test]
    fn requested_rule_must_be_inside_the_exact_unexcluded_schedule_scope() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let rule = rule(true, Duration::ZERO, 0);
        let history = history();
        let candidate = candidate(&rule, now);
        let other_reference = RuleRef::new(
            RuleId::new("developer.other.cache").unwrap(),
            RuleRevision::new(1).unwrap(),
        );
        let configs = [
            config_for_scope(
                AutomationScheduleScope::Rule(other_reference),
                Duration::ZERO,
                0,
                Vec::new(),
            ),
            config_for_scope(
                AutomationScheduleScope::Category(CandidateCategory::ApplicationCache),
                Duration::ZERO,
                0,
                Vec::new(),
            ),
            config_for_scope(
                AutomationScheduleScope::Category(CandidateCategory::DeveloperArtifact),
                Duration::ZERO,
                0,
                vec![rule.reference().clone()],
            ),
        ];
        let expected = [
            AutomationEligibilityReason::RuleNotInScheduleScope,
            AutomationEligibilityReason::RuleNotInScheduleScope,
            AutomationEligibilityReason::RuleExcludedBySchedule,
        ];

        for (schedule, reason) in configs.iter().zip(expected) {
            let assessment = assess_automation_eligibility(AutomationEligibilityInput {
                requested_rule: rule.reference(),
                shipped_rule: Some(&rule),
                schedule,
                history: &history,
                current_candidate: Some(&candidate),
                activity: AutomationActivityEvidence::Inactive { observed_at: now },
                runtime_identity: AutomationRuntimeIdentityEvidence::CurrentUser,
                assessed_at: now,
            });
            assert_eq!(
                assessment.decision(),
                AutomationEligibilityDecision::Ineligible
            );
            assert_eq!(assessment.gates()[0].reason(), Some(reason));
        }
    }

    #[test]
    fn every_definitive_safety_failure_blocks() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let rule = rule(true, Duration::ZERO, 0);
        let config = config(&rule, Duration::ZERO, 0);
        let mut candidate = candidate(&rule, now);
        candidate.protected_descendant_observed = true;
        let history = AutomationManualHistoryEvidence::Observed {
            successful_manual_runs: 2,
            recent_runs: vec![
                AutomationRecentRunEvidence::Succeeded,
                AutomationRecentRunEvidence::Failed,
            ],
        };
        let assessment = assess_automation_eligibility(AutomationEligibilityInput {
            requested_rule: rule.reference(),
            shipped_rule: Some(&rule),
            schedule: &config,
            history: &history,
            current_candidate: Some(&candidate),
            activity: AutomationActivityEvidence::Active { observed_at: now },
            runtime_identity: AutomationRuntimeIdentityEvidence::Privileged,
            assessed_at: now,
        });

        assert_eq!(
            assessment.decision(),
            AutomationEligibilityDecision::Ineligible
        );
        let reasons = assessment
            .gates()
            .iter()
            .filter_map(|gate| gate.reason())
            .collect::<Vec<_>>();
        assert!(reasons.contains(&AutomationEligibilityReason::RecentRunFailed));
        assert!(reasons.contains(&AutomationEligibilityReason::CurrentProtectedDescendant));
        assert!(reasons.contains(&AutomationEligibilityReason::ActivityDetected));
        assert!(reasons.contains(&AutomationEligibilityReason::RuntimePrivileged));
    }

    #[test]
    fn schedule_thresholds_strengthen_rule_thresholds_at_inclusive_boundary() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let rule = rule(true, Duration::from_secs(40), 600);
        let config = config(&rule, Duration::from_secs(80), 750);
        let history = history();
        let mut candidate = candidate(&rule, now);
        candidate.newest_mtime = Some(now - Duration::from_secs(80));
        candidate.estimated_bytes = 750;
        let eligible = assess_automation_eligibility(AutomationEligibilityInput {
            requested_rule: rule.reference(),
            shipped_rule: Some(&rule),
            schedule: &config,
            history: &history,
            current_candidate: Some(&candidate),
            activity: AutomationActivityEvidence::Inactive { observed_at: now },
            runtime_identity: AutomationRuntimeIdentityEvidence::CurrentUser,
            assessed_at: now,
        });
        assert!(eligible.is_eligible());

        candidate.estimated_bytes = 749;
        let below = assess_automation_eligibility(AutomationEligibilityInput {
            requested_rule: rule.reference(),
            shipped_rule: Some(&rule),
            schedule: &config,
            history: &history,
            current_candidate: Some(&candidate),
            activity: AutomationActivityEvidence::Inactive { observed_at: now },
            runtime_identity: AutomationRuntimeIdentityEvidence::CurrentUser,
            assessed_at: now,
        });
        assert_eq!(below.decision(), AutomationEligibilityDecision::Ineligible);
        assert_eq!(
            below.gates()[4].reason(),
            Some(AutomationEligibilityReason::CurrentCandidateBelowMinimumSize)
        );

        candidate.estimated_bytes = 750;
        candidate.newest_mtime = Some(now - Duration::from_secs(79));
        let too_young = assess_automation_eligibility(AutomationEligibilityInput {
            requested_rule: rule.reference(),
            shipped_rule: Some(&rule),
            schedule: &config,
            history: &history,
            current_candidate: Some(&candidate),
            activity: AutomationActivityEvidence::Inactive { observed_at: now },
            runtime_identity: AutomationRuntimeIdentityEvidence::CurrentUser,
            assessed_at: now,
        });
        assert_eq!(
            too_young.gates()[3].reason(),
            Some(AutomationEligibilityReason::CurrentCandidateBelowMinimumAge)
        );
    }

    #[test]
    fn rule_thresholds_strengthen_schedule_thresholds_at_inclusive_boundary() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let rule = rule(true, Duration::from_secs(80), 750);
        let config = config(&rule, Duration::from_secs(40), 600);
        let history = history();
        let mut candidate = candidate(&rule, now);
        candidate.newest_mtime = Some(now - Duration::from_secs(80));
        candidate.estimated_bytes = 750;
        let eligible = assess_automation_eligibility(AutomationEligibilityInput {
            requested_rule: rule.reference(),
            shipped_rule: Some(&rule),
            schedule: &config,
            history: &history,
            current_candidate: Some(&candidate),
            activity: AutomationActivityEvidence::Inactive { observed_at: now },
            runtime_identity: AutomationRuntimeIdentityEvidence::CurrentUser,
            assessed_at: now,
        });
        assert!(eligible.is_eligible());

        candidate.estimated_bytes = 749;
        let below_size = assess_automation_eligibility(AutomationEligibilityInput {
            requested_rule: rule.reference(),
            shipped_rule: Some(&rule),
            schedule: &config,
            history: &history,
            current_candidate: Some(&candidate),
            activity: AutomationActivityEvidence::Inactive { observed_at: now },
            runtime_identity: AutomationRuntimeIdentityEvidence::CurrentUser,
            assessed_at: now,
        });
        assert_eq!(
            below_size.gates()[4].reason(),
            Some(AutomationEligibilityReason::CurrentCandidateBelowMinimumSize)
        );

        candidate.estimated_bytes = 750;
        candidate.newest_mtime = Some(now - Duration::from_secs(79));
        let too_young = assess_automation_eligibility(AutomationEligibilityInput {
            requested_rule: rule.reference(),
            shipped_rule: Some(&rule),
            schedule: &config,
            history: &history,
            current_candidate: Some(&candidate),
            activity: AutomationActivityEvidence::Inactive { observed_at: now },
            runtime_identity: AutomationRuntimeIdentityEvidence::CurrentUser,
            assessed_at: now,
        });
        assert_eq!(
            too_young.gates()[3].reason(),
            Some(AutomationEligibilityReason::CurrentCandidateBelowMinimumAge)
        );
    }

    #[test]
    fn freshness_rejects_stale_future_and_inconsistent_evidence() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let rule = rule(true, Duration::ZERO, 0);
        let config = config(&rule, Duration::ZERO, 0);
        let history = history();
        let cases = [
            (
                now - AUTOMATION_CURRENT_EVIDENCE_MAX_AGE - Duration::from_secs(1),
                now - AUTOMATION_CURRENT_EVIDENCE_MAX_AGE - Duration::from_secs(1),
                AutomationEligibilityReason::EvidenceStale,
            ),
            (
                now + Duration::from_secs(1),
                now + Duration::from_secs(1),
                AutomationEligibilityReason::EvidenceFutureDated,
            ),
            (
                now,
                now - Duration::from_secs(1),
                AutomationEligibilityReason::EvidenceTimestampInconsistent,
            ),
        ];
        for (scan_completed_at, evaluated_at, reason) in cases {
            let mut candidate = candidate(&rule, now);
            candidate.scan_completed_at = scan_completed_at;
            candidate.evaluated_at = evaluated_at;
            let assessment = assess_automation_eligibility(AutomationEligibilityInput {
                requested_rule: rule.reference(),
                shipped_rule: Some(&rule),
                schedule: &config,
                history: &history,
                current_candidate: Some(&candidate),
                activity: AutomationActivityEvidence::Inactive { observed_at: now },
                runtime_identity: AutomationRuntimeIdentityEvidence::CurrentUser,
                assessed_at: now,
            });
            assert_eq!(assessment.gates()[6].reason(), Some(reason));
            assert!(!assessment.is_eligible());
        }
    }
}
