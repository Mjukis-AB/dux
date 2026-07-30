//! Bounded, read-only ranking of recurring deterministic cleanup rules.
//!
//! Rankings are recomputed from complete cleanup journals and the same
//! compatible later-scan observations used by exact-session rule outcomes.
//! The legacy `rule_outcomes` table is deliberately not an input.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{Duration, SystemTime};

use rusqlite::{Connection, params};

use crate::domain::RuleRef;

use super::cleanup_history::{CleanupSessionId, CleanupTrigger};
use super::cleanup_journal::load_cleanup_journal_within_budget;
use super::history::{HistoryError, HistoryErrorKind, map_query_sql_error};
use super::rule_outcome::{
    MAX_SOURCE_ITEMS, OutcomeWorkBudget, StoredRuleOutcomeState, derive_rule_outcomes_from_journal,
    eligibility_anchor, run_bounded_outcome_query,
};
use super::store::StoreCoordinator;

pub(crate) const MAX_STORAGE_THIEF_SOURCE_SESSIONS: usize = 32;
pub(crate) const MAX_STORAGE_THIEF_GROUPS: usize = 12;

const MAX_STORED_ID_BYTES: i64 = 128;
const NANOS_PER_DAY: u128 = 86_400_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredStorageThiefRanking {
    pub(crate) permanent_safe_session_count: usize,
    pub(crate) manual_cleanup_session_count: usize,
    pub(crate) ranked_rule_count: usize,
    pub(crate) has_older_permanent_safe_sessions: bool,
    pub(crate) groups: Vec<StoredStorageThiefGroup>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredStorageThiefGroup {
    pub(crate) latest_rule: RuleRef,
    pub(crate) observed_revision_count: usize,
    pub(crate) successful_cleanup_count: usize,
    pub(crate) successful_manual_cleanup_count: usize,
    pub(crate) observed_regrowth_cycle_count: usize,
    pub(crate) manual_regrowth_cycle_count: usize,
    pub(crate) total_observed_regrown_bytes: u64,
    pub(crate) total_regrowth_duration: Duration,
    pub(crate) bytes_regrown_per_day: u64,
    pub(crate) rate_capped: bool,
    pub(crate) latest_cleanup_at: SystemTime,
    pub(crate) latest_regrowth_at: SystemTime,
    pub(crate) automation_history_threshold_met: bool,
}

impl StoreCoordinator {
    pub(crate) fn recurring_storage_thieves(
        &self,
    ) -> Result<StoredStorageThiefRanking, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        run_bounded_outcome_query(&guard.connection, || {
            derive_storage_thief_ranking(&guard.connection)
        })
    }
}

fn derive_storage_thief_ranking(
    connection: &Connection,
) -> Result<StoredStorageThiefRanking, HistoryError> {
    let (session_ids, has_older_permanent_safe_sessions) = load_source_session_ids(connection)?;
    let permanent_safe_session_count = session_ids.len();
    let mut manual_cleanup_session_count = 0_usize;
    let mut groups = BTreeMap::<String, GroupAggregate>::new();
    let mut work = OutcomeWorkBudget::new();

    for session_id in session_ids {
        let journal =
            load_cleanup_journal_within_budget(connection, &session_id)?.ok_or_else(corrupt)?;
        if journal.session_id != session_id {
            return Err(corrupt());
        }
        let is_manual = journal.trigger == CleanupTrigger::Manual;
        if is_manual {
            manual_cleanup_session_count = manual_cleanup_session_count
                .checked_add(1)
                .ok_or_else(limit_exceeded)?;
        }

        if journal.items.len() > MAX_SOURCE_ITEMS {
            return Err(corrupt());
        }
        let mut item_indexes_by_rule = BTreeMap::<String, Vec<usize>>::new();
        for (index, item) in journal.items.iter().enumerate() {
            item_indexes_by_rule
                .entry(item.frozen.rule.id().as_str().to_owned())
                .or_default()
                .push(index);
        }
        let mut successful_rules_in_session = HashSet::new();
        for (rule_id, indexes) in &item_indexes_by_rule {
            let mut observations = Vec::with_capacity(indexes.len());
            for &index in indexes {
                let item = &journal.items[index];
                let Ok(cleaned_at) = eligibility_anchor(&journal, item) else {
                    observations.clear();
                    break;
                };
                observations.push((&item.frozen.rule, cleaned_at));
            }
            if observations.len() != indexes.len() {
                continue;
            }
            let latest = observations
                .iter()
                .max_by(|(left_rule, left_time), (right_rule, right_time)| {
                    left_time
                        .cmp(right_time)
                        .then_with(|| left_rule.revision().get().cmp(&right_rule.revision().get()))
                })
                .ok_or_else(corrupt)?;
            let aggregate = groups
                .entry(rule_id.clone())
                .or_insert_with(|| GroupAggregate::new(latest.0, latest.1));
            aggregate.observe_successful_session(&observations, is_manual)?;
            successful_rules_in_session.insert(rule_id.clone());
        }
        if successful_rules_in_session.is_empty() {
            continue;
        }

        let outcomes = derive_rule_outcomes_from_journal(connection, journal, &mut work)?;
        for outcome in outcomes.outcomes {
            if !successful_rules_in_session.contains(outcome.rule.id().as_str()) {
                continue;
            }
            let StoredRuleOutcomeState::Regrown {
                zero_observed_at,
                observed_at,
                observed_bytes,
                ..
            } = outcome.state
            else {
                continue;
            };
            let duration = observed_at
                .duration_since(zero_observed_at)
                .map_err(|_| corrupt())?;
            if duration.is_zero() || observed_bytes == 0 {
                return Err(corrupt());
            }
            let aggregate = groups
                .get_mut(outcome.rule.id().as_str())
                .ok_or_else(corrupt)?;
            aggregate.observe_regrowth(
                outcome.rule.revision().get(),
                is_manual,
                observed_at,
                observed_bytes,
                duration,
            )?;
        }
    }

    let mut ranked = groups
        .into_values()
        .filter(|aggregate| aggregate.observed_regrowth_cycle_count > 0)
        .map(GroupAggregate::finish)
        .collect::<Result<Vec<_>, _>>()?;
    ranked.sort_by(|left, right| {
        compare_storage_thief_rates(
            right.total_observed_regrown_bytes,
            right.total_regrowth_duration,
            left.total_observed_regrown_bytes,
            left.total_regrowth_duration,
        )
        .then_with(|| {
            right
                .successful_cleanup_count
                .cmp(&left.successful_cleanup_count)
        })
        .then_with(|| {
            right
                .observed_regrowth_cycle_count
                .cmp(&left.observed_regrowth_cycle_count)
        })
        .then_with(|| right.latest_regrowth_at.cmp(&left.latest_regrowth_at))
        .then_with(|| {
            left.latest_rule
                .id()
                .as_str()
                .cmp(right.latest_rule.id().as_str())
        })
    });
    let ranked_rule_count = ranked.len();
    ranked.truncate(MAX_STORAGE_THIEF_GROUPS);

    Ok(StoredStorageThiefRanking {
        permanent_safe_session_count,
        manual_cleanup_session_count,
        ranked_rule_count,
        has_older_permanent_safe_sessions,
        groups: ranked,
    })
}

fn load_source_session_ids(
    connection: &Connection,
) -> Result<(Vec<CleanupSessionId>, bool), HistoryError> {
    let row_limit = i64::try_from(MAX_STORAGE_THIEF_SOURCE_SESSIONS + 1).map_err(|_| corrupt())?;
    let mut statement = connection
        .prepare(
            "SELECT typeof(session_id), length(CAST(session_id AS BLOB)), session_id
             FROM cleanup_sessions
             WHERE record_format_version = 2
               AND mode = 'permanent_safe'
               AND status IN ('completed', 'partially_completed')
             ORDER BY started_at_unix_ms DESC, session_id ASC
             LIMIT ?1",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![row_limit])
        .map_err(map_query_sql_error)?;
    let mut session_ids = Vec::with_capacity(MAX_STORAGE_THIEF_SOURCE_SESSIONS + 1);
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        let storage_type: String = row.get(0).map_err(map_query_sql_error)?;
        let length: i64 = row.get(1).map_err(map_query_sql_error)?;
        if storage_type != "text" || !(1..=MAX_STORED_ID_BYTES).contains(&length) {
            return Err(corrupt());
        }
        session_ids.push(
            CleanupSessionId::new(row.get::<_, String>(2).map_err(map_query_sql_error)?)
                .map_err(|_| corrupt())?,
        );
    }
    let has_older = session_ids.len() > MAX_STORAGE_THIEF_SOURCE_SESSIONS;
    session_ids.truncate(MAX_STORAGE_THIEF_SOURCE_SESSIONS);
    Ok((session_ids, has_older))
}

struct GroupAggregate {
    latest_rule: RuleRef,
    revisions: BTreeSet<u32>,
    revision_evidence: BTreeMap<u32, RevisionEvidence>,
    successful_cleanup_count: usize,
    successful_manual_cleanup_count: usize,
    observed_regrowth_cycle_count: usize,
    manual_regrowth_cycle_count: usize,
    total_observed_regrown_bytes: u128,
    total_regrowth_duration: Duration,
    latest_cleanup_at: SystemTime,
    latest_regrowth_at: Option<SystemTime>,
}

#[derive(Default)]
struct RevisionEvidence {
    successful_manual_cleanup_count: usize,
    manual_regrowth_cycle_count: usize,
}

impl GroupAggregate {
    fn new(rule: &RuleRef, cleaned_at: SystemTime) -> Self {
        let mut revisions = BTreeSet::new();
        revisions.insert(rule.revision().get());
        Self {
            latest_rule: rule.clone(),
            revisions,
            revision_evidence: BTreeMap::new(),
            successful_cleanup_count: 0,
            successful_manual_cleanup_count: 0,
            observed_regrowth_cycle_count: 0,
            manual_regrowth_cycle_count: 0,
            total_observed_regrown_bytes: 0,
            total_regrowth_duration: Duration::ZERO,
            latest_cleanup_at: cleaned_at,
            latest_regrowth_at: None,
        }
    }

    fn observe_successful_session(
        &mut self,
        observations: &[(&RuleRef, SystemTime)],
        is_manual: bool,
    ) -> Result<(), HistoryError> {
        let mut session_revisions = BTreeSet::new();
        let (rule, cleaned_at) = observations
            .iter()
            .max_by(|(left_rule, left_time), (right_rule, right_time)| {
                left_time
                    .cmp(right_time)
                    .then_with(|| left_rule.revision().get().cmp(&right_rule.revision().get()))
            })
            .copied()
            .ok_or_else(corrupt)?;
        for (rule, _) in observations {
            self.revisions.insert(rule.revision().get());
            session_revisions.insert(rule.revision().get());
        }
        self.successful_cleanup_count = self
            .successful_cleanup_count
            .checked_add(1)
            .ok_or_else(limit_exceeded)?;
        if is_manual {
            self.successful_manual_cleanup_count = self
                .successful_manual_cleanup_count
                .checked_add(1)
                .ok_or_else(limit_exceeded)?;
            if session_revisions.len() == 1 {
                let revision = *session_revisions.first().ok_or_else(corrupt)?;
                let evidence = self.revision_evidence.entry(revision).or_default();
                evidence.successful_manual_cleanup_count = evidence
                    .successful_manual_cleanup_count
                    .checked_add(1)
                    .ok_or_else(limit_exceeded)?;
            }
        }
        let replaces_latest = cleaned_at > self.latest_cleanup_at
            || (cleaned_at == self.latest_cleanup_at
                && rule.revision().get() > self.latest_rule.revision().get());
        if replaces_latest {
            self.latest_rule = rule.clone();
            self.latest_cleanup_at = cleaned_at;
        }
        Ok(())
    }

    fn observe_regrowth(
        &mut self,
        revision: u32,
        is_manual: bool,
        observed_at: SystemTime,
        observed_bytes: u64,
        duration: Duration,
    ) -> Result<(), HistoryError> {
        self.observed_regrowth_cycle_count = self
            .observed_regrowth_cycle_count
            .checked_add(1)
            .ok_or_else(limit_exceeded)?;
        if is_manual {
            self.manual_regrowth_cycle_count = self
                .manual_regrowth_cycle_count
                .checked_add(1)
                .ok_or_else(limit_exceeded)?;
            let evidence = self.revision_evidence.entry(revision).or_default();
            evidence.manual_regrowth_cycle_count = evidence
                .manual_regrowth_cycle_count
                .checked_add(1)
                .ok_or_else(limit_exceeded)?;
        }
        self.total_observed_regrown_bytes = self
            .total_observed_regrown_bytes
            .checked_add(u128::from(observed_bytes))
            .ok_or_else(limit_exceeded)?;
        self.total_regrowth_duration = self
            .total_regrowth_duration
            .checked_add(duration)
            .ok_or_else(limit_exceeded)?;
        self.latest_regrowth_at = Some(
            self.latest_regrowth_at
                .map_or(observed_at, |current| current.max(observed_at)),
        );
        Ok(())
    }

    fn finish(self) -> Result<StoredStorageThiefGroup, HistoryError> {
        let latest_regrowth_at = self.latest_regrowth_at.ok_or_else(corrupt)?;
        let duration_nanos = self.total_regrowth_duration.as_nanos();
        if duration_nanos == 0 {
            return Err(corrupt());
        }
        let total_observed_regrown_bytes =
            u64::try_from(self.total_observed_regrown_bytes).map_err(|_| limit_exceeded())?;
        let (bytes_regrown_per_day, rate_capped) =
            storage_thief_rate_per_day(self.total_observed_regrown_bytes, duration_nanos);
        let observed_revision_count = self.revisions.len();
        let latest_revision_evidence = self
            .revision_evidence
            .get(&self.latest_rule.revision().get());
        let automation_history_threshold_met = latest_revision_evidence.is_some_and(|evidence| {
            evidence.successful_manual_cleanup_count >= 2
                && evidence.manual_regrowth_cycle_count >= 1
        });

        Ok(StoredStorageThiefGroup {
            latest_rule: self.latest_rule,
            observed_revision_count,
            successful_cleanup_count: self.successful_cleanup_count,
            successful_manual_cleanup_count: self.successful_manual_cleanup_count,
            observed_regrowth_cycle_count: self.observed_regrowth_cycle_count,
            manual_regrowth_cycle_count: self.manual_regrowth_cycle_count,
            total_observed_regrown_bytes,
            total_regrowth_duration: self.total_regrowth_duration,
            bytes_regrown_per_day,
            rate_capped,
            latest_cleanup_at: self.latest_cleanup_at,
            latest_regrowth_at,
            automation_history_threshold_met,
        })
    }
}

pub(crate) fn compare_storage_thief_rates(
    left_bytes: u64,
    left_duration: Duration,
    right_bytes: u64,
    right_duration: Duration,
) -> std::cmp::Ordering {
    compare_positive_fractions(
        u128::from(left_bytes),
        left_duration.as_nanos(),
        u128::from(right_bytes),
        right_duration.as_nanos(),
    )
}

fn compare_positive_fractions(
    mut left_numerator: u128,
    mut left_denominator: u128,
    mut right_numerator: u128,
    mut right_denominator: u128,
) -> std::cmp::Ordering {
    debug_assert!(left_denominator > 0 && right_denominator > 0);
    let mut inverted = false;
    loop {
        let left_quotient = left_numerator / left_denominator;
        let right_quotient = right_numerator / right_denominator;
        if left_quotient != right_quotient {
            let ordering = left_quotient.cmp(&right_quotient);
            return if inverted {
                ordering.reverse()
            } else {
                ordering
            };
        }
        let left_remainder = left_numerator % left_denominator;
        let right_remainder = right_numerator % right_denominator;
        match (left_remainder == 0, right_remainder == 0) {
            (true, true) => return std::cmp::Ordering::Equal,
            (true, false) => {
                return if inverted {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Less
                };
            }
            (false, true) => {
                return if inverted {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            (false, false) => {
                left_numerator = left_denominator;
                left_denominator = left_remainder;
                right_numerator = right_denominator;
                right_denominator = right_remainder;
                inverted = !inverted;
            }
        }
    }
}

pub(crate) fn storage_thief_rate_per_day(total_bytes: u128, duration_nanos: u128) -> (u64, bool) {
    debug_assert!(duration_nanos > 0);
    let whole = total_bytes / duration_nanos;
    if whole > u128::from(u64::MAX) / NANOS_PER_DAY {
        return (u64::MAX, true);
    }
    let base = whole * NANOS_PER_DAY;
    let remainder = total_bytes % duration_nanos;
    let fractional = multiply_then_divide_below_one(remainder, NANOS_PER_DAY, duration_nanos);
    let rate = base + fractional;
    match u64::try_from(rate) {
        Ok(rate) => (rate, false),
        Err(_) => (u64::MAX, true),
    }
}

fn multiply_then_divide_below_one(numerator: u128, multiplier: u128, denominator: u128) -> u128 {
    debug_assert!(numerator < denominator);
    let highest_bit = 127_u32.saturating_sub(multiplier.leading_zeros());
    let mut quotient = 0_u128;
    let mut remainder = 0_u128;
    for bit in (0..=highest_bit).rev() {
        let doubled_carry = remainder >= denominator - remainder;
        remainder = if doubled_carry {
            remainder - (denominator - remainder)
        } else {
            remainder + remainder
        };
        quotient = quotient * 2 + u128::from(doubled_carry);
        if (multiplier >> bit) & 1 == 1 {
            let added_carry = remainder >= denominator - numerator;
            remainder = if added_carry {
                remainder - (denominator - numerator)
            } else {
                remainder + numerator
            };
            quotient += u128::from(added_carry);
        }
    }
    quotient
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

const fn limit_exceeded() -> HistoryError {
    HistoryError::new(HistoryErrorKind::QueryLimitExceeded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_comparison_is_exact_without_cross_multiplication() {
        assert_eq!(
            compare_positive_fractions(u128::MAX, u128::MAX - 1, u128::MAX - 1, u128::MAX),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_positive_fractions(2, 4, u128::MAX / 2, u128::MAX - 1),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            compare_positive_fractions(1, 3, 2, 5),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn daily_rate_is_floored_and_explicitly_capped() {
        assert_eq!(storage_thief_rate_per_day(1, NANOS_PER_DAY * 2), (0, false));
        assert_eq!(storage_thief_rate_per_day(3, NANOS_PER_DAY * 2), (1, false));
        assert_eq!(storage_thief_rate_per_day(10, NANOS_PER_DAY), (10, false));
        assert_eq!(
            storage_thief_rate_per_day(u128::from(u64::MAX), 1),
            (u64::MAX, true)
        );
    }

    #[test]
    fn overflow_free_fractional_multiply_matches_direct_products() {
        for numerator in [0_u128, 1, 2, 7, 99, 999] {
            for denominator in [1_000_u128, 1_001, 65_535] {
                assert_eq!(
                    multiply_then_divide_below_one(numerator, NANOS_PER_DAY, denominator),
                    numerator * NANOS_PER_DAY / denominator
                );
            }
        }
        let denominator = u128::MAX - 17;
        let numerator = denominator - 1;
        let quotient = multiply_then_divide_below_one(numerator, NANOS_PER_DAY, denominator);
        assert_eq!(quotient, NANOS_PER_DAY - 1);
    }
}
