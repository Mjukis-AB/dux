//! Read-only rule outcome and regrowth derivation from durable history.
//!
//! Outcomes are recomputed from schema-v2 cleanup journals plus the exact
//! schema-v13 scan/evaluation/candidate graph. The legacy `rule_outcomes`
//! table is deliberately outside this module's inputs.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use rusqlite::{Connection, params};

use crate::domain::{
    CandidateAction, KNOWN_USER_CACHE_SCAN_ID_PREFIX, RuleRef, SafetyTier, ScanCoverageStatus,
    ScanId,
};

use super::candidate_evaluation_history::{
    CandidateEvaluationRecord, load_candidate_evaluation_within_budget,
};
use super::cleanup_history::CleanupSessionId;
use super::cleanup_journal::{
    CleanupJournal, JournalItem, JournalLifecycle, PathStatus, TerminalSessionStatus,
    load_cleanup_journal_within_budget,
};
use super::codec::encode_host_path;
use super::history::{
    HistoryError, HistoryErrorKind, ScanRecord, ScanStatus, load_scan_record_within_budget,
    map_query_sql_error, system_time_to_unix_ms,
};
use super::store::StoreCoordinator;

const MAX_FOLLOWUP_SCANS: usize = 256;
const MAX_INTERVENING_JOURNALS: usize = 256;
pub(super) const MAX_SOURCE_ITEMS: usize = 64;
const QUERY_PROGRESS_INTERVAL: i32 = 100;
const QUERY_MAX_CALLBACKS: u64 = 40_000;
const QUERY_MAX_ELAPSED: Duration = Duration::from_secs(5);
const RUST_MAX_COMPARISON_WORK: u64 = 2_000_000;
const MAX_STORED_ID_BYTES: i64 = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredRuleOutcomeBatch {
    pub(crate) session_id: CleanupSessionId,
    pub(crate) outcomes: Vec<StoredRuleOutcome>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredRuleOutcome {
    pub(crate) item_ordinal: usize,
    pub(crate) rule: RuleRef,
    pub(crate) state: StoredRuleOutcomeState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StoredRuleOutcomeState {
    NotEligible {
        reason: StoredRuleOutcomeNotEligibleReason,
    },
    AwaitingComparableScan {
        cleaned_at: SystemTime,
    },
    Superseded {
        cleaned_at: SystemTime,
        superseded_at: SystemTime,
    },
    LaterSizeObserved {
        cleaned_at: SystemTime,
        observed_at: SystemTime,
        observed_bytes: u64,
    },
    ZeroBaselineObserved {
        cleaned_at: SystemTime,
        observed_at: SystemTime,
    },
    Regrown {
        cleaned_at: SystemTime,
        zero_observed_at: SystemTime,
        observed_at: SystemTime,
        observed_bytes: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StoredRuleOutcomeNotEligibleReason {
    SourceCleanupIncomplete,
    ItemNotSuccessfulPermanentRegenerable,
    SourceScanNotComparable,
    SourceEvaluationNotComparable,
    SourceEvaluationAfterPlan,
    SourceCandidateMismatch,
}

struct FollowupObservation {
    scan_id: ScanId,
    started_at: SystemTime,
    completed_at: Option<SystemTime>,
    comparable: bool,
    observed_bytes_by_item: Vec<Option<u64>>,
}

struct OutcomeSource<'a> {
    root: &'a Path,
    scan: &'a ScanRecord,
    evaluation: &'a CandidateEvaluationRecord,
    items: &'a [JournalItem],
    eligibility: &'a [Result<SystemTime, StoredRuleOutcomeNotEligibleReason>],
}

impl StoreCoordinator {
    /// Derive outcomes for every item in one exact cleanup session.
    ///
    /// The result is observation-only. All paths, candidate identities,
    /// evaluator digests, snapshots, and execution fences remain sealed.
    pub(crate) fn rule_outcomes_for_cleanup_session(
        &self,
        session_id: &CleanupSessionId,
    ) -> Result<Option<StoredRuleOutcomeBatch>, HistoryError> {
        let guard = self.lock_current_history_connection()?;
        run_bounded_outcome_query(&guard.connection, || {
            let mut work = OutcomeWorkBudget::new();
            derive_rule_outcomes(&guard.connection, session_id, &mut work)
        })
    }
}

fn derive_rule_outcomes(
    connection: &Connection,
    session_id: &CleanupSessionId,
    work: &mut OutcomeWorkBudget,
) -> Result<Option<StoredRuleOutcomeBatch>, HistoryError> {
    // The complete journal decoder admits only record_format_version = 2 and
    // validates the full frozen plan plus dynamic graph before returning.
    let Some(journal) = load_cleanup_journal_within_budget(connection, session_id)? else {
        return Ok(None);
    };
    derive_rule_outcomes_from_journal(connection, journal, work).map(Some)
}

pub(super) fn derive_rule_outcomes_from_journal(
    connection: &Connection,
    journal: CleanupJournal,
    work: &mut OutcomeWorkBudget,
) -> Result<StoredRuleOutcomeBatch, HistoryError> {
    let session_id = journal.session_id.clone();
    let eligibility = journal
        .items
        .iter()
        .map(|item| eligibility_anchor(&journal, item))
        .collect::<Vec<_>>();
    let Some(first_anchor) = eligibility
        .iter()
        .filter_map(|result| result.as_ref().ok().copied())
        .min()
    else {
        return Ok(StoredRuleOutcomeBatch {
            session_id,
            outcomes: journal
                .items
                .iter()
                .zip(eligibility)
                .map(|(item, eligibility)| StoredRuleOutcome {
                    item_ordinal: item.frozen.ordinal,
                    rule: item.frozen.rule.clone(),
                    state: StoredRuleOutcomeState::NotEligible {
                        reason: eligibility.expect_err("no eligible anchor"),
                    },
                })
                .collect(),
        });
    };

    let source_scan =
        load_scan_record_within_budget(connection, &journal.source_scan_id)?.ok_or_else(corrupt)?;
    let source_evaluation =
        load_candidate_evaluation_within_budget(connection, &journal.source_scan_id)?;
    let common_source_failure = if !scan_is_complete_snapshot_observation(&source_scan) {
        Some(StoredRuleOutcomeNotEligibleReason::SourceScanNotComparable)
    } else {
        match source_evaluation.as_ref() {
            None => Some(StoredRuleOutcomeNotEligibleReason::SourceEvaluationNotComparable),
            Some(evaluation)
                if !evaluation.is_succeeded()
                    || !evaluation.matches_current_scan_observation(&source_scan) =>
            {
                Some(StoredRuleOutcomeNotEligibleReason::SourceEvaluationNotComparable)
            }
            Some(evaluation)
                if evaluation
                    .completed_at()
                    .is_none_or(|completed_at| completed_at > journal.plan_created_at) =>
            {
                Some(StoredRuleOutcomeNotEligibleReason::SourceEvaluationAfterPlan)
            }
            Some(_) => None,
        }
    };
    let mut source_eligibility = Vec::with_capacity(journal.items.len());
    for (item, eligibility) in journal.items.iter().zip(&eligibility) {
        source_eligibility.push(match eligibility {
            Err(reason) => Err(*reason),
            Ok(anchor) => {
                if let Some(reason) = common_source_failure {
                    Err(reason)
                } else if !exact_source_candidate(
                    source_evaluation
                        .as_ref()
                        .expect("source evaluation checked"),
                    item,
                    work,
                )? {
                    Err(StoredRuleOutcomeNotEligibleReason::SourceCandidateMismatch)
                } else {
                    Ok(*anchor)
                }
            }
        });
    }
    if source_eligibility.iter().all(Result::is_err) {
        return Ok(StoredRuleOutcomeBatch {
            session_id,
            outcomes: journal
                .items
                .iter()
                .zip(source_eligibility)
                .map(|(item, eligibility)| StoredRuleOutcome {
                    item_ordinal: item.frozen.ordinal,
                    rule: item.frozen.rule.clone(),
                    state: StoredRuleOutcomeState::NotEligible {
                        reason: eligibility.expect_err("no eligible source item"),
                    },
                })
                .collect(),
        });
    }
    let root = source_scan.root().to_path_buf();
    let source_evaluation = source_evaluation.as_ref().expect("eligible source exists");
    let source = OutcomeSource {
        root: &root,
        scan: &source_scan,
        evaluation: source_evaluation,
        items: &journal.items,
        eligibility: &source_eligibility,
    };
    let followups = load_followups(connection, first_anchor, &source, work)?;
    let superseding_by_item =
        load_superseding_effects(connection, &session_id, first_anchor, &source, work)?;

    let outcomes = journal
        .items
        .iter()
        .zip(source_eligibility)
        .enumerate()
        .map(|(item_index, (item, eligibility))| {
            let state = match eligibility {
                Err(reason) => StoredRuleOutcomeState::NotEligible { reason },
                Ok(anchor) => derive_item_outcome(
                    item_index,
                    anchor,
                    &followups,
                    superseding_by_item[item_index],
                    work,
                )?,
            };
            Ok(StoredRuleOutcome {
                item_ordinal: item.frozen.ordinal,
                rule: item.frozen.rule.clone(),
                state,
            })
        })
        .collect::<Result<Vec<_>, HistoryError>>()?;

    Ok(StoredRuleOutcomeBatch {
        session_id,
        outcomes,
    })
}

pub(super) fn eligibility_anchor(
    journal: &CleanupJournal,
    item: &JournalItem,
) -> Result<SystemTime, StoredRuleOutcomeNotEligibleReason> {
    if !matches!(
        journal.lifecycle,
        JournalLifecycle::Terminal {
            status: TerminalSessionStatus::Completed | TerminalSessionStatus::PartiallyCompleted,
            ..
        }
    ) {
        return Err(StoredRuleOutcomeNotEligibleReason::SourceCleanupIncomplete);
    }
    if journal.mode != crate::domain::CleanupMode::PermanentSafe
        || item.frozen.safety != SafetyTier::SafeRegenerable
        || item.frozen.proposed_action != CandidateAction::RemoveKnownRegenerableContents
        || item.status != PathStatus::Removed
        || item.error_category.is_some()
        || item.paths.is_empty()
        || item
            .paths
            .iter()
            .any(|path| path.status != PathStatus::Removed || path.error_category.is_some())
    {
        return Err(StoredRuleOutcomeNotEligibleReason::ItemNotSuccessfulPermanentRegenerable);
    }
    item.paths
        .iter()
        .map(|path| path.completed_at)
        .max()
        .flatten()
        .ok_or(StoredRuleOutcomeNotEligibleReason::ItemNotSuccessfulPermanentRegenerable)
}

fn scan_is_complete_snapshot_observation(scan: &ScanRecord) -> bool {
    scan.status() == ScanStatus::Succeeded
        && scan.completed_at().is_some()
        && scan.snapshot().is_some()
        && scan.coverage().status() == ScanCoverageStatus::Complete
        && scan.root_identity_v1_sha256().is_some()
}

fn exact_source_candidate(
    evaluation: &CandidateEvaluationRecord,
    item: &JournalItem,
    work: &mut OutcomeWorkBudget,
) -> Result<bool, HistoryError> {
    let mut found = None;
    for candidate in evaluation.candidates() {
        work.charge(1)?;
        if candidate.id() == &item.frozen.candidate_id && found.replace(candidate).is_some() {
            return Err(corrupt());
        }
    }
    let Some(candidate) = found else {
        return Ok(false);
    };
    Ok(candidate.source_scan_id() == evaluation.scan_id()
        && candidate.rule() == &item.frozen.rule
        && candidate.category() == item.frozen.category
        && candidate.paths() == item.frozen.paths.as_slice()
        && candidate.estimated_bytes() == item.frozen.estimated_bytes
        && candidate.newest_mtime() == item.frozen.newest_mtime
        && candidate.evidence() == item.frozen.evidence.as_slice()
        && candidate.safety() == item.frozen.safety
        && candidate.action() == item.frozen.proposed_action
        && candidate.rule_schedule_eligible() == item.frozen.rule_schedule_eligible)
}

fn derive_item_outcome(
    item_index: usize,
    anchor: SystemTime,
    followups: &[FollowupObservation],
    superseding: Option<SystemTime>,
    work: &mut OutcomeWorkBudget,
) -> Result<StoredRuleOutcomeState, HistoryError> {
    let cutoff = superseding;
    let mut zero_observed_at = None;
    let comparable = followups
        .iter()
        .map(|followup| {
            followup.comparable
                && followup.started_at > anchor
                && followup
                    .completed_at
                    .is_some_and(|completed_at| cutoff.is_none_or(|cutoff| completed_at < cutoff))
        })
        .collect::<Vec<_>>();
    let mut overlaps = vec![false; followups.len()];
    for left in 0..followups.len() {
        if !comparable[left] {
            continue;
        }
        for right in left + 1..followups.len() {
            if comparable[right]
                && scan_intervals_overlap(&followups[left], &followups[right], work)?
            {
                overlaps[left] = true;
                overlaps[right] = true;
            }
        }
    }

    for (index, followup) in followups.iter().enumerate() {
        if !comparable[index] || overlaps[index] {
            continue;
        }
        let completed_at = followup.completed_at.ok_or_else(corrupt)?;
        let Some(observed_bytes) = followup.observed_bytes_by_item[item_index] else {
            // Candidate absence is not an explicit zero observation.
            continue;
        };
        if observed_bytes == 0 {
            zero_observed_at.get_or_insert(completed_at);
            continue;
        }
        return Ok(match zero_observed_at {
            Some(zero_observed_at) => StoredRuleOutcomeState::Regrown {
                cleaned_at: anchor,
                zero_observed_at,
                observed_at: completed_at,
                observed_bytes,
            },
            None => StoredRuleOutcomeState::LaterSizeObserved {
                cleaned_at: anchor,
                observed_at: completed_at,
                observed_bytes,
            },
        });
    }

    if let Some(superseded_at) = superseding {
        return Ok(StoredRuleOutcomeState::Superseded {
            cleaned_at: anchor,
            superseded_at: superseded_at.max(anchor),
        });
    }
    Ok(match zero_observed_at {
        Some(observed_at) => StoredRuleOutcomeState::ZeroBaselineObserved {
            cleaned_at: anchor,
            observed_at,
        },
        None => StoredRuleOutcomeState::AwaitingComparableScan { cleaned_at: anchor },
    })
}

fn scan_intervals_overlap(
    left: &FollowupObservation,
    right: &FollowupObservation,
    work: &mut OutcomeWorkBudget,
) -> Result<bool, HistoryError> {
    work.charge(1)?;
    let left_completed = left.completed_at.ok_or_else(corrupt)?;
    let right_completed = right.completed_at.ok_or_else(corrupt)?;
    Ok(left.started_at <= right_completed && right.started_at <= left_completed)
}

fn load_followups(
    connection: &Connection,
    started_after: SystemTime,
    source: &OutcomeSource<'_>,
    work: &mut OutcomeWorkBudget,
) -> Result<Vec<FollowupObservation>, HistoryError> {
    // The complete journal decoder caps source items at 64. Compacting each
    // evaluation immediately therefore retains at most 256 × 64 optional u64
    // slots, never candidate/path/evidence graphs from multiple scans.
    if source.items.len() > MAX_SOURCE_ITEMS || source.eligibility.len() != source.items.len() {
        return Err(corrupt());
    }
    let encoded_root = encode_host_path(source.root).map_err(|_| corrupt())?;
    let started_after = system_time_to_unix_ms(started_after, HistoryErrorKind::CorruptData)?;
    let row_limit = i64::try_from(MAX_FOLLOWUP_SCANS + 1).map_err(|_| corrupt())?;
    let mut statement = connection
        .prepare(
            "SELECT typeof(scan_id), length(CAST(scan_id AS BLOB)), scan_id
             FROM scans
             WHERE root_path = ?1 AND root_path_encoding = ?2
               AND started_at_unix_ms > ?3
             ORDER BY started_at_unix_ms ASC, scan_id ASC
             LIMIT ?4",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![
            encoded_root.bytes,
            encoded_root.encoding as i64,
            started_after,
            row_limit
        ])
        .map_err(map_query_sql_error)?;
    let mut ids = Vec::with_capacity(MAX_FOLLOWUP_SCANS + 1);
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        validate_id_row(row, 0, 1)?;
        ids.push(
            ScanId::new(row.get::<_, String>(2).map_err(map_query_sql_error)?)
                .map_err(|_| corrupt())?,
        );
    }
    drop(rows);
    drop(statement);
    if ids.len() > MAX_FOLLOWUP_SCANS {
        return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
    }

    let mut followups = Vec::with_capacity(ids.len());
    for id in ids {
        let scan = load_scan_record_within_budget(connection, &id)?.ok_or_else(corrupt)?;
        if scan.root() != source.root || scan.started_at() <= started_after_system(started_after)? {
            return Err(corrupt());
        }
        let mut comparable = scan_is_complete_snapshot_observation(&scan)
            && scan.root_identity_v1_sha256() == source.scan.root_identity_v1_sha256()
            && same_evaluation_scope(scan.id(), source.scan.id());
        let mut observed_bytes_by_item = vec![None; source.items.len()];
        if comparable {
            let evaluation = load_candidate_evaluation_within_budget(connection, &id)?;
            comparable = evaluation.as_ref().is_some_and(|evaluation| {
                evaluation.is_succeeded()
                    && evaluation.matches_current_scan_observation(&scan)
                    && evaluation.has_compatible_outcome_identity(source.evaluation)
            });
            if comparable {
                let evaluation = evaluation.as_ref().expect("comparable evaluation");
                for (item_index, item) in source.items.iter().enumerate() {
                    if source.eligibility[item_index].is_err() {
                        continue;
                    }
                    let mut observed = None;
                    for candidate in evaluation.candidates() {
                        work.charge(1)?;
                        if candidate.rule() == &item.frozen.rule
                            && candidate.paths() == item.frozen.paths.as_slice()
                            && observed.replace(candidate.estimated_bytes()).is_some()
                        {
                            return Err(corrupt());
                        }
                    }
                    observed_bytes_by_item[item_index] = observed;
                }
            }
            // The full evaluation and every candidate/path payload drop here.
        }
        followups.push(FollowupObservation {
            scan_id: scan.id().clone(),
            started_at: scan.started_at(),
            completed_at: scan.completed_at(),
            comparable,
            observed_bytes_by_item,
        });
    }
    followups.sort_by(
        |left, right| match (left.completed_at, right.completed_at) {
            (Some(left_completed), Some(right_completed)) => left_completed
                .cmp(&right_completed)
                .then_with(|| left.started_at.cmp(&right.started_at))
                .then_with(|| left.scan_id.as_str().cmp(right.scan_id.as_str())),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => left
                .started_at
                .cmp(&right.started_at)
                .then_with(|| left.scan_id.as_str().cmp(right.scan_id.as_str())),
        },
    );
    Ok(followups)
}

fn started_after_system(unix_ms: i64) -> Result<SystemTime, HistoryError> {
    super::history::unix_ms_to_system_time(unix_ms)
}

fn load_superseding_effects(
    connection: &Connection,
    source_session_id: &CleanupSessionId,
    completed_at_or_after: SystemTime,
    source: &OutcomeSource<'_>,
    work: &mut OutcomeWorkBudget,
) -> Result<Vec<Option<SystemTime>>, HistoryError> {
    // Only one scalar timestamp per bounded source item survives each decoded
    // journal; target paths from different journals are never accumulated.
    if source.items.len() > MAX_SOURCE_ITEMS || source.eligibility.len() != source.items.len() {
        return Err(corrupt());
    }
    let completed_at =
        system_time_to_unix_ms(completed_at_or_after, HistoryErrorKind::CorruptData)?;
    let row_limit = i64::try_from(MAX_INTERVENING_JOURNALS + 1).map_err(|_| corrupt())?;
    let mut statement = connection
        .prepare(
            "SELECT typeof(session_id), length(CAST(session_id AS BLOB)), session_id
             FROM cleanup_sessions
             WHERE session_id != ?1 AND record_format_version = 2
               AND (
                   completed_at_unix_ms >= ?2 OR
                   completed_at_unix_ms IS NULL
               )
             ORDER BY started_at_unix_ms ASC, session_id ASC
             LIMIT ?3",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![source_session_id.as_str(), completed_at, row_limit])
        .map_err(map_query_sql_error)?;
    let mut ids = Vec::with_capacity(MAX_INTERVENING_JOURNALS + 1);
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        validate_id_row(row, 0, 1)?;
        ids.push(
            CleanupSessionId::new(row.get::<_, String>(2).map_err(map_query_sql_error)?)
                .map_err(|_| corrupt())?,
        );
    }
    drop(rows);
    drop(statement);
    if ids.len() > MAX_INTERVENING_JOURNALS {
        return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
    }

    let mut superseding_by_item: Vec<Option<SystemTime>> = vec![None; source.items.len()];
    for id in ids {
        let journal = load_cleanup_journal_within_budget(connection, &id)?.ok_or_else(corrupt)?;
        if journal.mode != crate::domain::CleanupMode::PermanentSafe {
            continue;
        }
        for item in &journal.items {
            if item.frozen.proposed_action != CandidateAction::RemoveKnownRegenerableContents
                || item.status != PathStatus::Removed
                || item.error_category.is_some()
                || item
                    .paths
                    .iter()
                    .any(|path| path.status != PathStatus::Removed || path.error_category.is_some())
            {
                continue;
            }
            for path in &item.paths {
                let effect_started = path.effect_started_at.ok_or_else(corrupt)?;
                let effect_completed = path.completed_at.ok_or_else(corrupt)?;
                for (source_index, source_item) in source.items.iter().enumerate() {
                    let Ok(anchor) = source.eligibility[source_index] else {
                        continue;
                    };
                    let mut overlaps = false;
                    for source in &source_item.frozen.paths {
                        work.charge(1)?;
                        if paths_overlap(source, &path.target) {
                            overlaps = true;
                            break;
                        }
                    }
                    if effect_completed >= anchor && overlaps {
                        let retained = &mut superseding_by_item[source_index];
                        *retained = Some(
                            retained.map_or(effect_started, |current| current.min(effect_started)),
                        );
                    }
                }
            }
        }
        // The complete journal and all path payloads drop before the next ID.
    }
    Ok(superseding_by_item)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn same_evaluation_scope(left: &ScanId, right: &ScanId) -> bool {
    left.as_str().starts_with(KNOWN_USER_CACHE_SCAN_ID_PREFIX)
        == right.as_str().starts_with(KNOWN_USER_CACHE_SCAN_ID_PREFIX)
}

fn validate_id_row(
    row: &rusqlite::Row<'_>,
    type_index: usize,
    length_index: usize,
) -> Result<(), HistoryError> {
    let storage_type: String = row.get(type_index).map_err(map_query_sql_error)?;
    let length: i64 = row.get(length_index).map_err(map_query_sql_error)?;
    if storage_type != "text" || !(1..=MAX_STORED_ID_BYTES).contains(&length) {
        return Err(corrupt());
    }
    Ok(())
}

pub(super) struct OutcomeWorkBudget {
    started_at: Instant,
    remaining: u64,
}

impl OutcomeWorkBudget {
    pub(super) fn new() -> Self {
        Self {
            started_at: Instant::now(),
            remaining: RUST_MAX_COMPARISON_WORK,
        }
    }

    #[cfg(test)]
    fn with_limit(limit: u64) -> Self {
        Self {
            started_at: Instant::now(),
            remaining: limit,
        }
    }

    fn charge(&mut self, amount: u64) -> Result<(), HistoryError> {
        if self.started_at.elapsed() >= QUERY_MAX_ELAPSED || self.remaining < amount {
            return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
        }
        self.remaining -= amount;
        Ok(())
    }
}

pub(super) fn run_bounded_outcome_query<T>(
    connection: &Connection,
    query: impl FnOnce() -> Result<T, HistoryError>,
) -> Result<T, HistoryError> {
    let started_at = Instant::now();
    let mut callbacks = 0_u64;
    connection
        .progress_handler(
            QUERY_PROGRESS_INTERVAL,
            Some(move || {
                callbacks = callbacks.saturating_add(1);
                callbacks >= QUERY_MAX_CALLBACKS || started_at.elapsed() >= QUERY_MAX_ELAPSED
            }),
        )
        .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
    let mut guard = OutcomeProgressGuard {
        connection,
        installed: true,
    };
    let result = query();
    guard.remove()?;
    let value = result.map_err(|error| {
        if error.kind == HistoryErrorKind::DatabaseUnavailable
            && started_at.elapsed() >= QUERY_MAX_ELAPSED
        {
            HistoryError::new(HistoryErrorKind::QueryLimitExceeded)
        } else {
            error
        }
    })?;
    if started_at.elapsed() >= QUERY_MAX_ELAPSED {
        return Err(HistoryError::new(HistoryErrorKind::QueryLimitExceeded));
    }
    Ok(value)
}

struct OutcomeProgressGuard<'a> {
    connection: &'a Connection,
    installed: bool,
}

impl OutcomeProgressGuard<'_> {
    fn remove(&mut self) -> Result<(), HistoryError> {
        self.connection
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(|_| HistoryError::new(HistoryErrorKind::DatabaseUnavailable))?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for OutcomeProgressGuard<'_> {
    fn drop(&mut self) {
        if self.installed {
            let _ = self.connection.progress_handler(0, None::<fn() -> bool>);
        }
    }
}

const fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_work_budget_exhaustion_is_typed_and_never_goes_negative() {
        let mut budget = OutcomeWorkBudget::with_limit(1);
        budget.charge(1).unwrap();
        assert_eq!(
            budget.charge(1).unwrap_err().kind,
            HistoryErrorKind::QueryLimitExceeded
        );
    }
}
