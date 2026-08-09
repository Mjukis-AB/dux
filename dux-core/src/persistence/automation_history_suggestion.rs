//! Bounded, read-only automation suggestions from exact manual history.
//!
//! The private grouping key includes the exact source root and durable root
//! identity. Neither value is exposed, and observations from distinct roots
//! are never combined.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::SystemTime;

use rusqlite::{Connection, params};

use crate::domain::{
    CandidateAction, CleanupMode, KNOWN_USER_CACHE_SCAN_ID_PREFIX, Rule, RuleRef, RuleScope,
    SafetyTier, ScanCoverageStatus,
};

use super::candidate_evaluation_history::load_candidate_evaluation_within_budget;
use super::cleanup_history::{CleanupSessionId, CleanupTrigger};
use super::cleanup_journal::{CleanupJournal, load_cleanup_journal_within_budget};
use super::history::{
    HistoryError, HistoryErrorKind, ScanStatus, load_scan_record_within_budget, map_query_sql_error,
};
use super::rule_outcome::{
    MAX_SOURCE_ITEMS, OutcomeWorkBudget, StoredRuleOutcomeState, derive_rule_outcomes_from_journal,
    eligibility_anchor, run_bounded_outcome_query,
};
use super::store::StoreCoordinator;

pub(crate) const MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS: usize = 32;
pub(crate) const MAX_AUTOMATION_HISTORY_SUGGESTIONS: usize = 12;

const MAX_STORED_ID_BYTES: i64 = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredAutomationScheduleSuggestionFeed {
    pub(crate) source_session_count: usize,
    pub(crate) qualifying_rule_count: usize,
    pub(crate) has_older_source_sessions: bool,
    pub(crate) suggestions: Vec<StoredAutomationScheduleSuggestion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredAutomationScheduleSuggestion {
    pub(crate) rule: RuleRef,
    pub(crate) successful_manual_run_count: usize,
    pub(crate) manual_regrowth_cycle_count: usize,
    pub(crate) latest_manual_attempt_at: SystemTime,
    pub(crate) latest_regrowth_at: SystemTime,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PrivateGroupKey {
    rule: RuleRef,
    root: PathBuf,
    root_identity_v1_sha256: [u8; 32],
}

#[derive(Default)]
struct GroupAggregate {
    attempts_newest_first: Vec<bool>,
    successful_manual_run_count: usize,
    manual_regrowth_cycle_count: usize,
    latest_manual_attempt_at: Option<SystemTime>,
    latest_regrowth_at: Option<SystemTime>,
}

impl StoreCoordinator {
    pub(crate) fn automation_schedule_suggestions(
        &self,
        current_rules: &[Rule],
    ) -> Result<StoredAutomationScheduleSuggestionFeed, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        run_bounded_outcome_query(&guard.connection, || {
            derive_suggestions(&guard.connection, current_rules)
        })
    }
}

fn derive_suggestions(
    connection: &Connection,
    current_rules: &[Rule],
) -> Result<StoredAutomationScheduleSuggestionFeed, HistoryError> {
    let current_rules = validate_current_rules(current_rules)?;
    let (session_ids, has_older_source_sessions) = load_source_session_ids(connection)?;
    let source_session_count = session_ids.len();
    if current_rules.is_empty() {
        return Ok(StoredAutomationScheduleSuggestionFeed {
            source_session_count,
            qualifying_rule_count: 0,
            has_older_source_sessions,
            suggestions: Vec::new(),
        });
    }
    let mut groups = BTreeMap::<PrivateGroupKey, GroupAggregate>::new();
    let mut blocked_rules = BTreeSet::<RuleRef>::new();
    let mut work = OutcomeWorkBudget::new();

    for session_id in session_ids {
        let journal =
            load_cleanup_journal_within_budget(connection, &session_id)?.ok_or_else(corrupt)?;
        if journal.session_id != session_id
            || journal.mode != CleanupMode::PermanentSafe
            || journal.trigger != CleanupTrigger::Manual
            || journal.items.len() > MAX_SOURCE_ITEMS
        {
            return Err(corrupt());
        }

        let indexes_by_rule = matching_item_indexes(&journal, &current_rules);
        if indexes_by_rule.is_empty() {
            continue;
        }

        let Some((root, root_identity, source_is_comparable, exact_items)) = validate_source(
            connection,
            &journal,
            &indexes_by_rule,
            &current_rules,
            &mut work,
        )?
        else {
            blocked_rules.extend(indexes_by_rule.into_keys());
            continue;
        };

        let anchors_are_valid = indexes_by_rule.values().any(|indexes| {
            indexes
                .iter()
                .all(|&index| eligibility_anchor(&journal, &journal.items[index]).is_ok())
        });
        let outcomes = if source_is_comparable && anchors_are_valid {
            Some(derive_rule_outcomes_from_journal(
                connection,
                journal.clone(),
                &mut work,
            )?)
        } else {
            None
        };

        for (rule, indexes) in indexes_by_rule {
            let exact_source = source_is_comparable
                && indexes
                    .iter()
                    .all(|index| exact_items.contains(&journal.items[*index].frozen.ordinal));
            let successful = exact_source
                && indexes
                    .iter()
                    .all(|&index| eligibility_anchor(&journal, &journal.items[index]).is_ok())
                && outcomes.as_ref().is_some_and(|batch| {
                    indexes.iter().all(|index| {
                        batch
                            .outcomes
                            .iter()
                            .find(|outcome| {
                                outcome.item_ordinal == journal.items[*index].frozen.ordinal
                            })
                            .is_some_and(|outcome| {
                                !matches!(outcome.state, StoredRuleOutcomeState::NotEligible { .. })
                            })
                    })
                });
            let regrowth_at = successful
                .then(|| {
                    outcomes.as_ref().and_then(|batch| {
                        indexes
                            .iter()
                            .filter_map(|index| {
                                batch.outcomes.iter().find_map(|outcome| {
                                    if outcome.item_ordinal != journal.items[*index].frozen.ordinal
                                    {
                                        return None;
                                    }
                                    match outcome.state {
                                        StoredRuleOutcomeState::Regrown { observed_at, .. } => {
                                            Some(observed_at)
                                        }
                                        _ => None,
                                    }
                                })
                            })
                            .max()
                    })
                })
                .flatten();
            let key = PrivateGroupKey {
                rule,
                root: root.clone(),
                root_identity_v1_sha256: root_identity,
            };
            groups
                .entry(key)
                .or_default()
                .observe(journal.started_at, successful, regrowth_at)?;
        }
    }

    let mut suggestions = groups
        .into_iter()
        .filter(|(key, aggregate)| !blocked_rules.contains(&key.rule) && aggregate.qualifies())
        .map(|(key, aggregate)| aggregate.finish(key.rule))
        .collect::<Result<Vec<_>, _>>()?;
    suggestions.sort_by(|left, right| {
        right
            .latest_regrowth_at
            .cmp(&left.latest_regrowth_at)
            .then_with(|| {
                right
                    .successful_manual_run_count
                    .cmp(&left.successful_manual_run_count)
            })
            .then_with(|| {
                right
                    .manual_regrowth_cycle_count
                    .cmp(&left.manual_regrowth_cycle_count)
            })
            .then_with(|| left.rule.cmp(&right.rule))
    });
    let mut retained_rules = BTreeSet::new();
    suggestions.retain(|suggestion| retained_rules.insert(suggestion.rule.clone()));
    let qualifying_rule_count = suggestions.len();
    suggestions.truncate(MAX_AUTOMATION_HISTORY_SUGGESTIONS);

    Ok(StoredAutomationScheduleSuggestionFeed {
        source_session_count,
        qualifying_rule_count,
        has_older_source_sessions,
        suggestions,
    })
}

fn validate_current_rules(
    current_rules: &[Rule],
) -> Result<BTreeMap<RuleRef, &Rule>, HistoryError> {
    let mut validated = BTreeMap::new();
    for rule in current_rules {
        if !rule.schedule_eligible()
            || rule.scope() != RuleScope::UserCacheDirectory
            || rule.safety() != SafetyTier::SafeRegenerable
            || rule.action() != CandidateAction::RemoveKnownRegenerableContents
            || !rule.matcher().protected_descendants().is_empty()
            || validated.insert(rule.reference().clone(), rule).is_some()
        {
            return Err(HistoryError::new(HistoryErrorKind::InternalState));
        }
    }
    Ok(validated)
}

fn matching_item_indexes(
    journal: &CleanupJournal,
    current_rules: &BTreeMap<RuleRef, &Rule>,
) -> BTreeMap<RuleRef, Vec<usize>> {
    let mut matches = BTreeMap::<RuleRef, Vec<usize>>::new();
    for (index, item) in journal.items.iter().enumerate() {
        if current_rules.contains_key(&item.frozen.rule) {
            matches
                .entry(item.frozen.rule.clone())
                .or_default()
                .push(index);
        }
    }
    matches
}

#[allow(clippy::type_complexity)]
fn validate_source(
    connection: &Connection,
    journal: &CleanupJournal,
    indexes_by_rule: &BTreeMap<RuleRef, Vec<usize>>,
    current_rules: &BTreeMap<RuleRef, &Rule>,
    work: &mut OutcomeWorkBudget,
) -> Result<Option<(PathBuf, [u8; 32], bool, BTreeSet<usize>)>, HistoryError> {
    let Some(scan) = load_scan_record_within_budget(connection, &journal.source_scan_id)? else {
        return Ok(None);
    };
    if scan.id() != &journal.source_scan_id
        || !scan
            .id()
            .as_str()
            .starts_with(KNOWN_USER_CACHE_SCAN_ID_PREFIX)
    {
        return Ok(None);
    }
    let Some(root_identity) = scan.root_identity_v1_sha256() else {
        return Ok(None);
    };
    let evaluation = load_candidate_evaluation_within_budget(connection, scan.id())?;
    let source_is_comparable = scan.status() == ScanStatus::Succeeded
        && scan.completed_at().is_some()
        && scan.snapshot().is_some()
        && scan.coverage().status() == ScanCoverageStatus::Complete
        && evaluation.as_ref().is_some_and(|evaluation| {
            evaluation.is_succeeded()
                && evaluation.matches_current_scan_observation(&scan)
                && evaluation
                    .completed_at()
                    .is_some_and(|completed_at| completed_at <= journal.plan_created_at)
        });
    let mut exact_items = BTreeSet::new();
    if let Some(evaluation) = evaluation.as_ref().filter(|_| source_is_comparable) {
        for (rule_ref, indexes) in indexes_by_rule {
            let current_rule = current_rules.get(rule_ref).ok_or_else(corrupt)?;
            for &index in indexes {
                let item = &journal.items[index];
                let mut found = None;
                for candidate in evaluation.candidates() {
                    work.charge(1)?;
                    if candidate.id() == &item.frozen.candidate_id
                        && found.replace(candidate).is_some()
                    {
                        return Err(corrupt());
                    }
                }
                if found.is_some_and(|candidate| {
                    candidate.source_scan_id() == evaluation.scan_id()
                        && candidate.rule() == current_rule.reference()
                        && candidate.category() == current_rule.category()
                        && candidate.category() == item.frozen.category
                        && candidate.paths() == item.frozen.paths.as_slice()
                        && candidate.estimated_bytes() == item.frozen.estimated_bytes
                        && candidate.newest_mtime() == item.frozen.newest_mtime
                        && candidate.evidence() == item.frozen.evidence.as_slice()
                        && candidate.safety() == SafetyTier::SafeRegenerable
                        && candidate.safety() == item.frozen.safety
                        && candidate.action() == CandidateAction::RemoveKnownRegenerableContents
                        && candidate.action() == item.frozen.proposed_action
                        && candidate.rule_schedule_eligible()
                        && item.frozen.rule_schedule_eligible
                        && candidate.blockers().is_empty()
                }) {
                    exact_items.insert(item.frozen.ordinal);
                }
            }
        }
    }
    Ok(Some((
        scan.root().to_path_buf(),
        root_identity,
        source_is_comparable,
        exact_items,
    )))
}

fn load_source_session_ids(
    connection: &Connection,
) -> Result<(Vec<CleanupSessionId>, bool), HistoryError> {
    let row_limit =
        i64::try_from(MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS + 1).map_err(|_| corrupt())?;
    let mut statement = connection
        .prepare(
            "SELECT typeof(session_id), length(CAST(session_id AS BLOB)), session_id
             FROM cleanup_sessions
             WHERE record_format_version = 2
               AND mode = 'permanent_safe'
               AND trigger_source = 'manual'
             ORDER BY started_at_unix_ms DESC, session_id ASC
             LIMIT ?1",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![row_limit])
        .map_err(map_query_sql_error)?;
    let mut session_ids = Vec::with_capacity(MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS + 1);
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
    let has_older = session_ids.len() > MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS;
    session_ids.truncate(MAX_AUTOMATION_HISTORY_SOURCE_SESSIONS);
    Ok((session_ids, has_older))
}

impl GroupAggregate {
    fn observe(
        &mut self,
        attempted_at: SystemTime,
        successful: bool,
        regrowth_at: Option<SystemTime>,
    ) -> Result<(), HistoryError> {
        self.attempts_newest_first.push(successful);
        self.latest_manual_attempt_at = Some(
            self.latest_manual_attempt_at
                .map_or(attempted_at, |current| current.max(attempted_at)),
        );
        if successful {
            self.successful_manual_run_count = self
                .successful_manual_run_count
                .checked_add(1)
                .ok_or_else(limit_exceeded)?;
        }
        if let Some(regrowth_at) = regrowth_at {
            self.manual_regrowth_cycle_count = self
                .manual_regrowth_cycle_count
                .checked_add(1)
                .ok_or_else(limit_exceeded)?;
            self.latest_regrowth_at = Some(
                self.latest_regrowth_at
                    .map_or(regrowth_at, |current| current.max(regrowth_at)),
            );
        }
        Ok(())
    }

    fn qualifies(&self) -> bool {
        self.attempts_newest_first.len() >= 2
            && self.attempts_newest_first[0]
            && self.attempts_newest_first[1]
            && self.manual_regrowth_cycle_count >= 1
    }

    fn finish(self, rule: RuleRef) -> Result<StoredAutomationScheduleSuggestion, HistoryError> {
        Ok(StoredAutomationScheduleSuggestion {
            rule,
            successful_manual_run_count: self.successful_manual_run_count,
            manual_regrowth_cycle_count: self.manual_regrowth_cycle_count,
            latest_manual_attempt_at: self.latest_manual_attempt_at.ok_or_else(corrupt)?,
            latest_regrowth_at: self.latest_regrowth_at.ok_or_else(corrupt)?,
        })
    }
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

const fn limit_exceeded() -> HistoryError {
    HistoryError::new(HistoryErrorKind::QueryLimitExceeded)
}
