use std::fmt::Write as _;
use std::process::ExitCode;
use std::time::{Duration, UNIX_EPOCH};

use dux_core::engine::{
    CandidateHistoryError, CandidateReviewCommand, CandidateReviewError, CleanupHistoryCursor,
    CleanupHistoryError, DurableCandidateEvaluation, DurableCandidateEvaluationStatus,
    DurableCandidateStatus, DurableCandidateSummary, DurableCleanupHistoryPage,
    DurableCleanupItemStatus, DurableCleanupMode, DurableCleanupRecordFormat,
    DurableCleanupSessionId, DurableCleanupSessionObservation, DurableCleanupSessionStatus,
    DurableCleanupSessionSummary, DurableCleanupStatusCounts, DurableCleanupTrigger,
    DurableCleanupWarning, DurableScanCoverageDetailsPage, DurableScanIssueKind,
    ScanCoverageDetailsError, SnapshotReviewError,
};
use dux_core::{
    BlockReason, CandidateAction, CandidateCategory, CandidateEvaluationTaskFailureKind,
    EvidenceKind, SafetyTier,
};
use serde::Serialize;

use crate::cli::{
    CandidatesArgs, CleanupHistoryArgs, CleanupHistoryCommand, CleanupHistoryListArgs,
    CleanupHistoryShowArgs, ReviewCommandArg, ReviewStateArgs, ScanDetailArgs,
};
use crate::noninteractive::{
    CommandError, ErrorCode, JSON_SCHEMA_VERSION, coverage_status, durable_status, emit,
    serialize_json, unix_ms, with_engine,
};

const PAGE_ORDER: &str = "evaluation_ordinal_asc";
const CLEANUP_ORDER: &str = "started_at_desc_session_id_asc";

pub(crate) fn run_scan_detail(args: ScanDetailArgs) -> ExitCode {
    let json = args.json;
    emit("scan-detail", json, build_scan_detail(args))
}

pub(crate) fn run_candidates(args: CandidatesArgs) -> ExitCode {
    let json = args.json;
    emit("candidates", json, build_candidates(args))
}

pub(crate) fn run_review_state(args: ReviewStateArgs) -> ExitCode {
    let json = args.json;
    emit("review-state", json, build_review_state(args))
}

pub(crate) fn run_cleanup_history(args: CleanupHistoryArgs) -> ExitCode {
    let json = match &args.command {
        CleanupHistoryCommand::List(args) => args.json,
        CleanupHistoryCommand::Show(args) => args.json,
    };
    emit(
        "cleanup-history",
        json,
        match args.command {
            CleanupHistoryCommand::List(args) => build_cleanup_history_list(args),
            CleanupHistoryCommand::Show(args) => build_cleanup_history_show(args),
        },
    )
}

fn build_scan_detail(args: ScanDetailArgs) -> Result<String, CommandError> {
    with_engine(|engine| {
        let page = engine
            .scan_coverage_details(&args.scan_id, args.offset, args.limit)
            .map_err(map_scan_detail_error)?;
        if args.json {
            serialize_json(&ScanDetailDocument::try_from_page(page, args.limit)?)
        } else {
            render_scan_detail(&page, args.limit)
        }
    })
}

fn build_candidates(args: CandidatesArgs) -> Result<String, CommandError> {
    with_engine(|engine| {
        let observation = engine
            .candidate_history_for_scan(&args.scan_id)
            .map_err(map_candidate_history_error)?;
        let page = CandidatePage::new(&observation, args.cursor, args.limit)?;
        if args.json {
            serialize_json(&CandidatesDocument::try_from_page(&observation, page)?)
        } else {
            render_candidates(&observation, &page)
        }
    })
}

fn build_review_state(args: ReviewStateArgs) -> Result<String, CommandError> {
    with_engine(|engine| {
        let mut review = engine
            .acquire_explorer_snapshot_review(&args.scan_id)
            .map_err(map_snapshot_review_error)?;
        if let Err(error) = review.validate_current() {
            let _ = review.release();
            return Err(map_snapshot_review_error(error));
        }

        let command = review_command(args.command);
        let result = match engine.review_candidate(&args.scan_id, &args.candidate_id, command) {
            Ok(result) => result,
            Err(error) => {
                let _ = review.release();
                return Err(map_candidate_review_error(error));
            }
        };

        // A failed post-write lease check or release makes the durable outcome
        // ambiguous to this process. Never retry the semantic transition.
        let post_write_validation = review.validate_current();
        let release = review.release();
        if post_write_validation.is_err() || release.is_err() {
            return Err(outcome_unknown());
        }
        let document = ReviewStateDocument {
            schema_version: JSON_SCHEMA_VERSION,
            command: "review-state",
            path_disclosure: "none",
            scan_id: result.scan_id().as_str(),
            candidate_id: result.candidate_id().as_str(),
            requested_command: review_command_name(args.command),
            status: candidate_status(result.status()),
            cleanup_performed: false,
        };
        if args.json {
            serialize_json(&document)
        } else {
            Ok(format!(
                "{} for {} is now {}. No cleanup was performed.",
                review_command_name(args.command),
                result.candidate_id().as_str(),
                candidate_status(result.status())
            ))
        }
    })
}

fn build_cleanup_history_list(args: CleanupHistoryListArgs) -> Result<String, CommandError> {
    let cursor = match (
        args.after_started_at_unix_ms,
        args.after_session_id.as_deref(),
    ) {
        (Some(started_at), Some(session_id)) => Some(CleanupHistoryCursor::new(
            UNIX_EPOCH
                .checked_add(Duration::from_millis(
                    u64::try_from(started_at).map_err(|_| CommandError::internal())?,
                ))
                .ok_or_else(CommandError::internal)?,
            cleanup_session_id(session_id)?,
        )),
        (None, None) => None,
        _ => return Err(CommandError::internal()),
    };
    with_engine(|engine| {
        let page = engine
            .recent_cleanup_history(cursor.as_ref(), args.limit)
            .map_err(map_cleanup_history_error)?;
        if args.json {
            serialize_json(&CleanupHistoryListDocument::try_from_page(
                &page, args.limit,
            )?)
        } else {
            render_cleanup_history_list(&page)
        }
    })
}

fn build_cleanup_history_show(args: CleanupHistoryShowArgs) -> Result<String, CommandError> {
    let session_id = cleanup_session_id(&args.session_id)?;
    with_engine(|engine| {
        let observation = engine
            .cleanup_session_history(&session_id)
            .map_err(map_cleanup_history_error)?;
        if args.json {
            serialize_json(&CleanupHistoryShowDocument::try_from_observation(
                &observation,
            )?)
        } else {
            render_cleanup_history_show(&observation)
        }
    })
}

fn cleanup_session_id(value: &str) -> Result<DurableCleanupSessionId, CommandError> {
    DurableCleanupSessionId::from_stable_str(value).ok_or_else(CommandError::internal)
}

struct CandidatePage<'a> {
    cursor: u16,
    limit: u16,
    next_cursor: Option<u16>,
    items: &'a [DurableCandidateSummary],
}

impl<'a> CandidatePage<'a> {
    fn new(
        observation: &'a DurableCandidateEvaluation,
        cursor: u16,
        limit: u16,
    ) -> Result<Self, CommandError> {
        let start = usize::from(cursor);
        let candidates = observation.candidates();
        if start > candidates.len() {
            return Err(cursor_out_of_range());
        }
        let end = start
            .saturating_add(usize::from(limit))
            .min(candidates.len());
        let next_cursor = if end < candidates.len() {
            Some(u16::try_from(end).map_err(|_| CommandError::internal())?)
        } else {
            None
        };
        Ok(Self {
            cursor,
            limit,
            next_cursor,
            items: &candidates[start..end],
        })
    }
}

#[derive(Serialize)]
struct ScanDetailDocument {
    schema_version: u32,
    command: &'static str,
    path_disclosure: &'static str,
    scan_id: String,
    page: OffsetPageDocument,
    coverage: CoverageDetailDocument,
    items: Vec<ScanIssueDocument>,
}

impl ScanDetailDocument {
    fn try_from_page(
        page: DurableScanCoverageDetailsPage,
        requested_limit: u16,
    ) -> Result<Self, CommandError> {
        let next_offset = if page.has_more() {
            Some(
                page.offset()
                    .checked_add(
                        u16::try_from(page.issues().len()).map_err(|_| CommandError::internal())?,
                    )
                    .ok_or_else(CommandError::internal)?,
            )
        } else {
            None
        };
        Ok(Self {
            schema_version: JSON_SCHEMA_VERSION,
            command: "scan-detail",
            path_disclosure: "none",
            scan_id: page.scan_id().as_str().to_owned(),
            page: OffsetPageDocument {
                schema_version: JSON_SCHEMA_VERSION,
                requested_offset: page.offset(),
                requested_limit,
                next_offset,
                has_more: page.has_more(),
            },
            coverage: CoverageDetailDocument {
                schema_version: JSON_SCHEMA_VERSION,
                status: coverage_status(page.status()),
                measured_permille: page.measured_permille().map(|value| value.get()),
                issue_record_count: page.total_issue_records(),
                issue_occurrence_count: page.total_issue_occurrences(),
            },
            items: page
                .issues()
                .iter()
                .map(|issue| {
                    let location = issue.location();
                    ScanIssueDocument {
                        schema_version: JSON_SCHEMA_VERSION,
                        ordinal: issue.ordinal(),
                        kind: scan_issue_kind(issue.kind()),
                        occurrence_count: issue.occurrence_count(),
                        location_scope: match location {
                            None => "global",
                            Some(location) if location.is_scan_root() => "scan_root",
                            Some(_) => "descendant",
                        },
                        location_recorded: location.is_some(),
                        location_context_truncated: location
                            .is_some_and(|value| value.context_truncated()),
                    }
                })
                .collect(),
        })
    }
}

#[derive(Serialize)]
struct OffsetPageDocument {
    schema_version: u32,
    requested_offset: u16,
    requested_limit: u16,
    next_offset: Option<u16>,
    has_more: bool,
}

#[derive(Serialize)]
struct CoverageDetailDocument {
    schema_version: u32,
    status: &'static str,
    measured_permille: Option<u16>,
    issue_record_count: u16,
    issue_occurrence_count: u64,
}

#[derive(Serialize)]
struct ScanIssueDocument {
    schema_version: u32,
    ordinal: u16,
    kind: &'static str,
    occurrence_count: u32,
    location_scope: &'static str,
    location_recorded: bool,
    location_context_truncated: bool,
}

#[derive(Serialize)]
struct CandidatesDocument {
    schema_version: u32,
    command: &'static str,
    path_disclosure: &'static str,
    order: &'static str,
    scan_id: String,
    source_scan_status: &'static str,
    scheduled_at_unix_ms: Option<i64>,
    completed_at_unix_ms: Option<i64>,
    evaluation: CandidateEvaluationDocument,
    total_candidates: u32,
    page: CursorPageDocument,
    items: Vec<CandidateDocument>,
}

impl CandidatesDocument {
    fn try_from_page(
        observation: &DurableCandidateEvaluation,
        page: CandidatePage<'_>,
    ) -> Result<Self, CommandError> {
        Ok(Self {
            schema_version: JSON_SCHEMA_VERSION,
            command: "candidates",
            path_disclosure: "none",
            order: PAGE_ORDER,
            scan_id: observation.scan_id().as_str().to_owned(),
            source_scan_status: durable_status(observation.source_scan_status()),
            scheduled_at_unix_ms: observation.scheduled_at().map(unix_ms).transpose()?,
            completed_at_unix_ms: observation.completed_at().map(unix_ms).transpose()?,
            evaluation: CandidateEvaluationDocument::from(observation.status()),
            total_candidates: u32::try_from(observation.candidates().len())
                .map_err(|_| CommandError::internal())?,
            page: CursorPageDocument {
                schema_version: JSON_SCHEMA_VERSION,
                requested_cursor: page.cursor,
                requested_limit: page.limit,
                next_cursor: page.next_cursor,
            },
            items: page
                .items
                .iter()
                .map(CandidateDocument::try_from)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Serialize)]
struct CandidateEvaluationDocument {
    schema_version: u32,
    status: &'static str,
    candidate_count: Option<u32>,
    failure: Option<&'static str>,
}

impl From<DurableCandidateEvaluationStatus> for CandidateEvaluationDocument {
    fn from(value: DurableCandidateEvaluationStatus) -> Self {
        let (status, candidate_count, failure) = match value {
            DurableCandidateEvaluationStatus::NotRun => ("not_run", None, None),
            DurableCandidateEvaluationStatus::Pending => ("pending", None, None),
            DurableCandidateEvaluationStatus::Succeeded { candidate_count } => {
                ("succeeded", Some(candidate_count), None)
            }
            DurableCandidateEvaluationStatus::Failed { kind } => {
                ("failed", None, Some(candidate_failure_kind(kind)))
            }
            _ => ("unknown", None, None),
        };
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            status,
            candidate_count,
            failure,
        }
    }
}

#[derive(Serialize)]
struct CursorPageDocument {
    schema_version: u32,
    requested_cursor: u16,
    requested_limit: u16,
    next_cursor: Option<u16>,
}

#[derive(Serialize)]
struct CandidateDocument {
    schema_version: u32,
    candidate_id: String,
    rule_id: String,
    rule_revision: u32,
    category: &'static str,
    estimated_bytes: u64,
    newest_mtime_unix_ms: Option<i64>,
    safety: &'static str,
    action: &'static str,
    rule_schedule_eligible: bool,
    path_count: u16,
    evidence_kinds: Vec<&'static str>,
    blockers: Vec<&'static str>,
    created_at_unix_ms: i64,
    status: &'static str,
}

impl TryFrom<&DurableCandidateSummary> for CandidateDocument {
    type Error = CommandError;

    fn try_from(value: &DurableCandidateSummary) -> Result<Self, Self::Error> {
        Ok(Self {
            schema_version: JSON_SCHEMA_VERSION,
            candidate_id: value.id().as_str().to_owned(),
            rule_id: value.rule().id().as_str().to_owned(),
            rule_revision: value.rule().revision().get(),
            category: candidate_category(value.category()),
            estimated_bytes: value.estimated_bytes(),
            newest_mtime_unix_ms: value.newest_mtime().map(unix_ms).transpose()?,
            safety: safety_tier(value.safety()),
            action: candidate_action(value.action()),
            rule_schedule_eligible: value.rule_schedule_eligible(),
            path_count: value.path_count(),
            evidence_kinds: value
                .evidence_kinds()
                .iter()
                .copied()
                .map(evidence_kind)
                .collect(),
            blockers: value.blockers().iter().cloned().map(block_reason).collect(),
            created_at_unix_ms: unix_ms(value.created_at())?,
            status: candidate_status(value.status()),
        })
    }
}

#[derive(Serialize)]
struct ReviewStateDocument<'a> {
    schema_version: u32,
    command: &'static str,
    path_disclosure: &'static str,
    scan_id: &'a str,
    candidate_id: &'a str,
    requested_command: &'static str,
    status: &'static str,
    cleanup_performed: bool,
}

#[derive(Serialize)]
struct CleanupHistoryListDocument {
    schema_version: u32,
    command: &'static str,
    operation: &'static str,
    path_disclosure: &'static str,
    order: &'static str,
    requested_limit: u16,
    has_more: bool,
    next_cursor: Option<CleanupCursorDocument>,
    items: Vec<CleanupSummaryDocument>,
}

impl CleanupHistoryListDocument {
    fn try_from_page(
        page: &DurableCleanupHistoryPage,
        requested_limit: u16,
    ) -> Result<Self, CommandError> {
        Ok(Self {
            schema_version: JSON_SCHEMA_VERSION,
            command: "cleanup-history",
            operation: "list",
            path_disclosure: "none",
            order: CLEANUP_ORDER,
            requested_limit,
            has_more: page.next_cursor().is_some(),
            next_cursor: page
                .next_cursor()
                .map(CleanupCursorDocument::try_from)
                .transpose()?,
            items: page
                .records()
                .iter()
                .map(CleanupSummaryDocument::try_from)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Serialize)]
struct CleanupCursorDocument {
    schema_version: u32,
    started_at_unix_ms: i64,
    session_id: String,
}

impl TryFrom<&CleanupHistoryCursor> for CleanupCursorDocument {
    type Error = CommandError;

    fn try_from(value: &CleanupHistoryCursor) -> Result<Self, Self::Error> {
        Ok(Self {
            schema_version: JSON_SCHEMA_VERSION,
            started_at_unix_ms: unix_ms(value.started_at())?,
            session_id: value.session_id().as_str().to_owned(),
        })
    }
}

#[derive(Serialize)]
struct CleanupHistoryShowDocument {
    schema_version: u32,
    command: &'static str,
    operation: &'static str,
    path_disclosure: &'static str,
    summary: CleanupSummaryDocument,
    items: Vec<CleanupItemDocument>,
    warnings: Vec<&'static str>,
}

impl CleanupHistoryShowDocument {
    fn try_from_observation(
        observation: &DurableCleanupSessionObservation,
    ) -> Result<Self, CommandError> {
        Ok(Self {
            schema_version: JSON_SCHEMA_VERSION,
            command: "cleanup-history",
            operation: "show",
            path_disclosure: "none",
            summary: CleanupSummaryDocument::try_from(observation.summary())?,
            items: observation
                .items()
                .iter()
                .map(CleanupItemDocument::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            warnings: observation
                .warnings()
                .iter()
                .copied()
                .map(cleanup_warning)
                .collect(),
        })
    }
}

#[derive(Serialize)]
struct CleanupSummaryDocument {
    schema_version: u32,
    session_id: String,
    plan_id: String,
    format: &'static str,
    source_scan_id: Option<String>,
    started_at_unix_ms: i64,
    completed_at_unix_ms: Option<i64>,
    plan_created_at_unix_ms: Option<i64>,
    plan_expires_at_unix_ms: Option<i64>,
    mode: &'static str,
    trigger: &'static str,
    status: &'static str,
    estimated_bytes: u64,
    verified_capacity_delta_bytes: Option<i64>,
    cancellation_requested: Option<bool>,
    item_total: u16,
    path_total: u16,
    evidence_total: u16,
    item_status_counts: CleanupStatusCountsDocument,
    path_status_counts: CleanupStatusCountsDocument,
}

impl TryFrom<&DurableCleanupSessionSummary> for CleanupSummaryDocument {
    type Error = CommandError;

    fn try_from(value: &DurableCleanupSessionSummary) -> Result<Self, Self::Error> {
        Ok(Self {
            schema_version: JSON_SCHEMA_VERSION,
            session_id: value.id().as_str().to_owned(),
            plan_id: value.plan_id().to_owned(),
            format: cleanup_record_format(value.format()),
            source_scan_id: value
                .source_scan_id()
                .map(|value| value.as_str().to_owned()),
            started_at_unix_ms: unix_ms(value.started_at())?,
            completed_at_unix_ms: value.completed_at().map(unix_ms).transpose()?,
            plan_created_at_unix_ms: value.plan_created_at().map(unix_ms).transpose()?,
            plan_expires_at_unix_ms: value.plan_expires_at().map(unix_ms).transpose()?,
            mode: cleanup_mode(value.mode()),
            trigger: cleanup_trigger(value.trigger()),
            status: cleanup_session_status(value.status()),
            estimated_bytes: value.estimated_bytes(),
            verified_capacity_delta_bytes: value.verified_capacity_delta_bytes(),
            cancellation_requested: value.cancellation_requested(),
            item_total: value.item_total(),
            path_total: value.path_total(),
            evidence_total: value.evidence_total(),
            item_status_counts: CleanupStatusCountsDocument::from(value.item_status_counts()),
            path_status_counts: CleanupStatusCountsDocument::from(value.path_status_counts()),
        })
    }
}

#[derive(Serialize)]
struct CleanupItemDocument {
    schema_version: u32,
    ordinal: u16,
    rule_id: String,
    rule_revision: u32,
    category: Option<&'static str>,
    safety: Option<&'static str>,
    action: Option<&'static str>,
    rule_schedule_eligible: Option<bool>,
    newest_mtime_unix_ms: Option<i64>,
    estimated_bytes: u64,
    status: &'static str,
    error_recorded: bool,
    error_category: Option<String>,
    path_count: u16,
    evidence_count: u16,
}

impl TryFrom<&dux_core::engine::DurableCleanupItemSummary> for CleanupItemDocument {
    type Error = CommandError;

    fn try_from(value: &dux_core::engine::DurableCleanupItemSummary) -> Result<Self, Self::Error> {
        Ok(Self {
            schema_version: JSON_SCHEMA_VERSION,
            ordinal: value.ordinal(),
            rule_id: value.rule().id().as_str().to_owned(),
            rule_revision: value.rule().revision().get(),
            category: value.category().map(candidate_category),
            safety: value.safety().map(safety_tier),
            action: value.action().map(candidate_action),
            rule_schedule_eligible: value.rule_schedule_eligible(),
            newest_mtime_unix_ms: value.newest_mtime().map(unix_ms).transpose()?,
            estimated_bytes: value.estimated_bytes(),
            status: cleanup_item_status(value.status()),
            error_recorded: value.error_recorded(),
            error_category: value
                .error_category()
                .map(|value| value.as_str().to_owned()),
            path_count: value.path_count(),
            evidence_count: value.evidence_count(),
        })
    }
}

#[derive(Serialize)]
struct CleanupStatusCountsDocument {
    schema_version: u32,
    planned: u16,
    validating: u16,
    dry_run: u16,
    effect_started: u16,
    trashed: u16,
    removed: u16,
    evicted: u16,
    skipped: u16,
    rejected: u16,
    failed: u16,
    changed_since_plan: u16,
    interrupted: u16,
    unavailable: u16,
    outcome_unknown: u16,
    total: u16,
}

impl From<&DurableCleanupStatusCounts> for CleanupStatusCountsDocument {
    fn from(value: &DurableCleanupStatusCounts) -> Self {
        Self {
            schema_version: JSON_SCHEMA_VERSION,
            planned: value.planned(),
            validating: value.validating(),
            dry_run: value.dry_run(),
            effect_started: value.effect_started(),
            trashed: value.trashed(),
            removed: value.removed(),
            evicted: value.evicted(),
            skipped: value.skipped(),
            rejected: value.rejected(),
            failed: value.failed(),
            changed_since_plan: value.changed_since_plan(),
            interrupted: value.interrupted(),
            unavailable: value.unavailable(),
            outcome_unknown: value.outcome_unknown(),
            total: value.total(),
        }
    }
}

fn render_scan_detail(
    page: &DurableScanCoverageDetailsPage,
    limit: u16,
) -> Result<String, CommandError> {
    let mut output = format!(
        "Scan {} coverage: {} ({} issue records, {} occurrences)\n",
        page.scan_id().as_str(),
        coverage_status(page.status()),
        page.total_issue_records(),
        page.total_issue_occurrences()
    );
    if page.issues().is_empty() {
        output.push_str("No coverage issues in this page.\n");
    } else {
        for issue in page.issues() {
            let scope = match issue.location() {
                None => "global",
                Some(location) if location.is_scan_root() => "scan root",
                Some(_) => "descendant",
            };
            let _ = writeln!(
                output,
                "{}  {}  occurrences={}  scope={}",
                issue.ordinal(),
                scan_issue_kind(issue.kind()),
                issue.occurrence_count(),
                scope
            );
        }
    }
    if page.has_more() {
        let next = page
            .offset()
            .checked_add(u16::try_from(page.issues().len()).map_err(|_| CommandError::internal())?)
            .ok_or_else(CommandError::internal)?;
        let _ = writeln!(
            output,
            "More issues exist; continue with --offset {next} --limit {limit}."
        );
    }
    output.pop();
    Ok(output)
}

fn render_candidates(
    observation: &DurableCandidateEvaluation,
    page: &CandidatePage<'_>,
) -> Result<String, CommandError> {
    let evaluation = CandidateEvaluationDocument::from(observation.status());
    let mut output = format!(
        "Candidates for {}: {}\n",
        observation.scan_id().as_str(),
        evaluation.status
    );
    if page.items.is_empty() {
        output.push_str("No candidates in this page.\n");
    } else {
        for candidate in page.items {
            let _ = writeln!(
                output,
                "{}  {}  {}  {}  {}",
                candidate.id().as_str(),
                candidate_status(candidate.status()),
                candidate_category(candidate.category()),
                dux_core::format_size(candidate.estimated_bytes()),
                candidate_action(candidate.action())
            );
        }
    }
    if let Some(next) = page.next_cursor {
        let _ = writeln!(
            output,
            "More candidates exist; continue with --cursor {next} --limit {}.",
            page.limit
        );
    }
    output.pop();
    Ok(output)
}

fn render_cleanup_history_list(page: &DurableCleanupHistoryPage) -> Result<String, CommandError> {
    let mut output = String::from("Cleanup history\n");
    if page.records().is_empty() {
        output.push_str("No cleanup sessions recorded.\n");
    } else {
        for record in page.records() {
            let _ = writeln!(
                output,
                "{}  {}  {}  {}  started {}",
                record.id().as_str(),
                cleanup_session_status(record.status()),
                cleanup_mode(record.mode()),
                dux_core::format_size(record.estimated_bytes()),
                unix_ms(record.started_at())?
            );
        }
    }
    if let Some(cursor) = page.next_cursor() {
        let _ = writeln!(
            output,
            "More sessions exist; continue with --after-started-at-unix-ms {} --after-session-id {}.",
            unix_ms(cursor.started_at())?,
            cursor.session_id().as_str()
        );
    }
    output.pop();
    Ok(output)
}

fn render_cleanup_history_show(
    observation: &DurableCleanupSessionObservation,
) -> Result<String, CommandError> {
    let summary = observation.summary();
    let mut output = format!(
        "Cleanup {}: {} ({}, {})\nEstimated: {}\nItems: {}  Paths: {}  Evidence: {}\n",
        summary.id().as_str(),
        cleanup_session_status(summary.status()),
        cleanup_mode(summary.mode()),
        cleanup_record_format(summary.format()),
        dux_core::format_size(summary.estimated_bytes()),
        summary.item_total(),
        summary.path_total(),
        summary.evidence_total()
    );
    for item in observation.items() {
        let _ = writeln!(
            output,
            "{}  {}  {}  {}",
            item.ordinal(),
            item.rule().id().as_str(),
            cleanup_item_status(item.status()),
            dux_core::format_size(item.estimated_bytes())
        );
    }
    output.pop();
    Ok(output)
}

fn map_scan_detail_error(error: ScanCoverageDetailsError) -> CommandError {
    match error {
        ScanCoverageDetailsError::ScanNotFound => scan_not_found(),
        ScanCoverageDetailsError::InvalidOffset => cursor_out_of_range(),
        ScanCoverageDetailsError::IncompatibleSchema => incompatible_schema(),
        ScanCoverageDetailsError::Busy => CommandError::busy(),
        ScanCoverageDetailsError::UnsafeStorage => CommandError::unsafe_storage(),
        ScanCoverageDetailsError::QueryLimitExceeded => query_limit(),
        ScanCoverageDetailsError::CorruptData => corrupt_data(),
        ScanCoverageDetailsError::Unavailable => CommandError::storage_unavailable(),
        ScanCoverageDetailsError::InvalidLimit { .. }
        | ScanCoverageDetailsError::Closed
        | ScanCoverageDetailsError::InternalState => CommandError::internal(),
        _ => CommandError::internal(),
    }
}

fn map_candidate_history_error(error: CandidateHistoryError) -> CommandError {
    match error {
        CandidateHistoryError::ScanNotFound => scan_not_found(),
        CandidateHistoryError::IncompatibleSchema => incompatible_schema(),
        CandidateHistoryError::Busy => CommandError::busy(),
        CandidateHistoryError::UnsafeStorage => CommandError::unsafe_storage(),
        CandidateHistoryError::QueryLimitExceeded => query_limit(),
        CandidateHistoryError::CorruptData => corrupt_data(),
        CandidateHistoryError::Unavailable => CommandError::storage_unavailable(),
        CandidateHistoryError::Closed | CandidateHistoryError::InternalState => {
            CommandError::internal()
        }
        _ => CommandError::internal(),
    }
}

fn map_candidate_review_error(error: CandidateReviewError) -> CommandError {
    match error {
        CandidateReviewError::CandidateNotFound => candidate_not_found(),
        CandidateReviewError::NotReviewable => CommandError::new(
            ErrorCode::CandidateNotReviewable,
            "The requested review transition is not valid for this candidate.",
        ),
        CandidateReviewError::IncompatibleSchema => incompatible_schema(),
        CandidateReviewError::Busy => CommandError::busy(),
        CandidateReviewError::UnsafeStorage => CommandError::unsafe_storage(),
        CandidateReviewError::QueryLimitExceeded => query_limit(),
        CandidateReviewError::CorruptData => corrupt_data(),
        CandidateReviewError::OutcomeUnknown => outcome_unknown(),
        CandidateReviewError::Unavailable => CommandError::storage_unavailable(),
        CandidateReviewError::Closed | CandidateReviewError::InternalState => {
            CommandError::internal()
        }
        _ => CommandError::internal(),
    }
}

fn map_cleanup_history_error(error: CleanupHistoryError) -> CommandError {
    match error {
        CleanupHistoryError::SessionNotFound => CommandError::new(
            ErrorCode::CleanupSessionNotFound,
            "The requested cleanup session does not exist.",
        ),
        CleanupHistoryError::IncompatibleSchema => incompatible_schema(),
        CleanupHistoryError::Busy => CommandError::busy(),
        CleanupHistoryError::UnsafeStorage => CommandError::unsafe_storage(),
        CleanupHistoryError::QueryLimitExceeded => query_limit(),
        CleanupHistoryError::CorruptData => corrupt_data(),
        CleanupHistoryError::Unavailable => CommandError::storage_unavailable(),
        CleanupHistoryError::InvalidLimit { .. }
        | CleanupHistoryError::Closed
        | CleanupHistoryError::InternalState => CommandError::internal(),
        _ => CommandError::internal(),
    }
}

fn map_snapshot_review_error(error: SnapshotReviewError) -> CommandError {
    match error {
        SnapshotReviewError::ScanNotFound => scan_not_found(),
        SnapshotReviewError::IncompatibleSchema | SnapshotReviewError::ReadOnlyStore => {
            incompatible_schema()
        }
        SnapshotReviewError::Busy => CommandError::busy(),
        SnapshotReviewError::UnsafeStorage => CommandError::unsafe_storage(),
        SnapshotReviewError::BudgetExceeded => query_limit(),
        SnapshotReviewError::CorruptData | SnapshotReviewError::IncompatibleSnapshot => {
            corrupt_data()
        }
        SnapshotReviewError::SnapshotUnavailable => CommandError::new(
            ErrorCode::CandidateNotReviewable,
            "The requested scan has no retained snapshot available for exact review.",
        ),
        SnapshotReviewError::Unavailable => CommandError::storage_unavailable(),
        SnapshotReviewError::OutcomeUnknown => outcome_unknown(),
        SnapshotReviewError::LeaseExpired
        | SnapshotReviewError::LivePathUnavailable
        | SnapshotReviewError::LivePathMissing
        | SnapshotReviewError::LivePathSymlink
        | SnapshotReviewError::LivePathCrossVolume
        | SnapshotReviewError::LivePathChanged
        | SnapshotReviewError::LivePathAccessDenied => CommandError::new(
            ErrorCode::CandidateNotReviewable,
            "The exact retained snapshot review is no longer current.",
        ),
        _ => CommandError::internal(),
    }
}

fn scan_not_found() -> CommandError {
    CommandError::new(
        ErrorCode::ScanNotFound,
        "The requested durable scan does not exist.",
    )
}

fn candidate_not_found() -> CommandError {
    CommandError::new(
        ErrorCode::CandidateNotFound,
        "The requested candidate does not belong to this scan.",
    )
}

fn cursor_out_of_range() -> CommandError {
    CommandError::new(
        ErrorCode::CursorOutOfRange,
        "The requested page cursor is outside the immutable observation.",
    )
}

fn incompatible_schema() -> CommandError {
    CommandError::new(
        ErrorCode::IncompatibleDatabaseSchema,
        "This history is unavailable because the database schema is incompatible.",
    )
}

fn query_limit() -> CommandError {
    CommandError::new(
        ErrorCode::QueryLimitExceeded,
        "The history query exceeded its safety limit.",
    )
}

fn corrupt_data() -> CommandError {
    CommandError::new(
        ErrorCode::CorruptDatabase,
        "The stored history is corrupt or invalid.",
    )
}

fn outcome_unknown() -> CommandError {
    CommandError::new(
        ErrorCode::OutcomeUnknown,
        "The candidate review outcome is unknown; read candidates again before taking further action.",
    )
}

fn review_command(value: ReviewCommandArg) -> CandidateReviewCommand {
    match value {
        ReviewCommandArg::Select => CandidateReviewCommand::Select,
        ReviewCommandArg::ClearSelection => CandidateReviewCommand::ClearSelection,
        ReviewCommandArg::Dismiss => CandidateReviewCommand::Dismiss,
        ReviewCommandArg::Restore => CandidateReviewCommand::Restore,
    }
}

fn review_command_name(value: ReviewCommandArg) -> &'static str {
    match value {
        ReviewCommandArg::Select => "select",
        ReviewCommandArg::ClearSelection => "clear_selection",
        ReviewCommandArg::Dismiss => "dismiss",
        ReviewCommandArg::Restore => "restore",
    }
}

fn scan_issue_kind(value: DurableScanIssueKind) -> &'static str {
    match value {
        DurableScanIssueKind::PermissionDenied => "permission_denied",
        DurableScanIssueKind::TimedOut => "timed_out",
        DurableScanIssueKind::DifferentFilesystem => "different_filesystem",
        DurableScanIssueKind::NetworkOrVirtualFilesystem => "network_or_virtual_filesystem",
        DurableScanIssueKind::SymlinkSkipped => "symlink_skipped",
        DurableScanIssueKind::FileChangedDuringScan => "file_changed_during_scan",
        DurableScanIssueKind::MetadataError => "metadata_error",
        DurableScanIssueKind::Cancelled => "cancelled",
        DurableScanIssueKind::PolicyExcluded => "policy_excluded",
        DurableScanIssueKind::DepthLimited => "depth_limited",
        DurableScanIssueKind::ProbePoolExhausted => "probe_pool_exhausted",
        DurableScanIssueKind::FilesystemBoundaryUnknown => "filesystem_boundary_unknown",
        DurableScanIssueKind::IssueLimitReached => "issue_limit_reached",
        _ => "unknown",
    }
}

fn candidate_failure_kind(value: CandidateEvaluationTaskFailureKind) -> &'static str {
    match value {
        CandidateEvaluationTaskFailureKind::Cancelled => "cancelled",
        CandidateEvaluationTaskFailureKind::CatalogInvalid => "catalog_invalid",
        CandidateEvaluationTaskFailureKind::ContextInvalid => "context_invalid",
        CandidateEvaluationTaskFailureKind::EvaluationFailed => "evaluation_failed",
        CandidateEvaluationTaskFailureKind::CandidateInvalid => "candidate_invalid",
        CandidateEvaluationTaskFailureKind::LimitExceeded => "limit_exceeded",
        _ => "unknown",
    }
}

fn candidate_status(value: DurableCandidateStatus) -> &'static str {
    match value {
        DurableCandidateStatus::Discovered => "discovered",
        DurableCandidateStatus::Selected => "selected",
        DurableCandidateStatus::Dismissed => "dismissed",
        DurableCandidateStatus::Stale => "stale",
        DurableCandidateStatus::Planned => "planned",
        DurableCandidateStatus::Completed => "completed",
        DurableCandidateStatus::Failed => "failed",
        DurableCandidateStatus::Unavailable => "unavailable",
        _ => "unknown",
    }
}

fn candidate_category(value: CandidateCategory) -> &'static str {
    match value {
        CandidateCategory::DeveloperArtifact => "developer_artifact",
        CandidateCategory::ApplicationCache => "application_cache",
        CandidateCategory::BrowserCache => "browser_cache",
        CandidateCategory::LogAndDiagnostic => "log_and_diagnostic",
        CandidateCategory::InstallerAndDownload => "installer_and_download",
        CandidateCategory::DeviceAndSimulatorData => "device_and_simulator_data",
        CandidateCategory::CloudFile => "cloud_file",
        CandidateCategory::LargeReviewItem => "large_review_item",
        CandidateCategory::ProtectedSystemData => "protected_system_data",
        CandidateCategory::UnknownStorage => "unknown_storage",
    }
}

fn safety_tier(value: SafetyTier) -> &'static str {
    match value {
        SafetyTier::SafeRegenerable => "safe_regenerable",
        SafetyTier::SafeEvictable => "safe_evictable",
        SafetyTier::ReviewRequired => "review_required",
        SafetyTier::Informational => "informational",
        SafetyTier::Protected => "protected",
    }
}

fn candidate_action(value: CandidateAction) -> &'static str {
    match value {
        CandidateAction::RemoveKnownRegenerableContents => "remove_known_regenerable_contents",
        CandidateAction::EvictLocalCopy => "evict_local_copy",
        CandidateAction::MoveToTrash => "move_to_trash",
        CandidateAction::RevealOnly => "reveal_only",
        CandidateAction::NoAction => "no_action",
    }
}

fn evidence_kind(value: EvidenceKind) -> &'static str {
    match value {
        EvidenceKind::MatchedPath => "matched_path",
        EvidenceKind::RequiredMarker => "required_marker",
        EvidenceKind::ForbiddenMarkerAbsent => "forbidden_marker_absent",
        EvidenceKind::BundleIdentifier => "bundle_identifier",
        EvidenceKind::MinimumAge => "minimum_age",
        EvidenceKind::MinimumSize => "minimum_size",
        EvidenceKind::InactiveProcess => "inactive_process",
        EvidenceKind::CloudUploadComplete => "cloud_upload_complete",
    }
}

fn block_reason(value: BlockReason) -> &'static str {
    match value {
        BlockReason::MissingOrIncompleteEvidence => "missing_or_incomplete_evidence",
        BlockReason::MissingModificationTime => "missing_modification_time",
        BlockReason::PartialScanCoverage => "partial_scan_coverage",
        BlockReason::RecentActivity => "recent_activity",
        BlockReason::BelowMinimumBytes => "below_minimum_bytes",
        BlockReason::ActiveUse => "active_use",
        BlockReason::AccessDenied => "access_denied",
        BlockReason::ProtectedPath => "protected_path",
        BlockReason::ProtectedDescendant => "protected_descendant",
        BlockReason::SymlinkBoundary => "symlink_boundary",
        BlockReason::VolumeBoundary => "volume_boundary",
        BlockReason::ChangedSinceScan => "changed_since_scan",
        BlockReason::UnsupportedPlatform => "unsupported_platform",
        BlockReason::CloudUploadUnconfirmed => "cloud_upload_unconfirmed",
    }
}

fn cleanup_record_format(value: DurableCleanupRecordFormat) -> &'static str {
    match value {
        DurableCleanupRecordFormat::LegacyIncomplete => "legacy_incomplete",
        DurableCleanupRecordFormat::Complete => "complete",
        _ => "unknown",
    }
}

fn cleanup_mode(value: DurableCleanupMode) -> &'static str {
    match value {
        DurableCleanupMode::DryRun => "dry_run",
        DurableCleanupMode::Trash => "trash",
        DurableCleanupMode::PermanentSafe => "permanent_safe",
        DurableCleanupMode::EvictLocalCopy => "evict_local_copy",
        _ => "unknown",
    }
}

fn cleanup_trigger(value: DurableCleanupTrigger) -> &'static str {
    match value {
        DurableCleanupTrigger::Manual => "manual",
        DurableCleanupTrigger::LowDisk => "low_disk",
        DurableCleanupTrigger::Scheduled => "scheduled",
        DurableCleanupTrigger::Cli => "cli",
        _ => "unknown",
    }
}

fn cleanup_session_status(value: DurableCleanupSessionStatus) -> &'static str {
    match value {
        DurableCleanupSessionStatus::Planned => "planned",
        DurableCleanupSessionStatus::Running => "running",
        DurableCleanupSessionStatus::Recovering => "recovering",
        DurableCleanupSessionStatus::Completed => "completed",
        DurableCleanupSessionStatus::PartiallyCompleted => "partially_completed",
        DurableCleanupSessionStatus::Failed => "failed",
        DurableCleanupSessionStatus::Cancelled => "cancelled",
        DurableCleanupSessionStatus::Interrupted => "interrupted",
        DurableCleanupSessionStatus::Rejected => "rejected",
        DurableCleanupSessionStatus::DryRun => "dry_run",
        _ => "unknown",
    }
}

fn cleanup_item_status(value: DurableCleanupItemStatus) -> &'static str {
    match value {
        DurableCleanupItemStatus::Planned => "planned",
        DurableCleanupItemStatus::Validating => "validating",
        DurableCleanupItemStatus::DryRun => "dry_run",
        DurableCleanupItemStatus::EffectStarted => "effect_started",
        DurableCleanupItemStatus::Trashed => "trashed",
        DurableCleanupItemStatus::Removed => "removed",
        DurableCleanupItemStatus::Evicted => "evicted",
        DurableCleanupItemStatus::Skipped => "skipped",
        DurableCleanupItemStatus::Rejected => "rejected",
        DurableCleanupItemStatus::Failed => "failed",
        DurableCleanupItemStatus::ChangedSincePlan => "changed_since_plan",
        DurableCleanupItemStatus::Interrupted => "interrupted",
        DurableCleanupItemStatus::Unavailable => "unavailable",
        DurableCleanupItemStatus::OutcomeUnknown => "outcome_unknown",
        _ => "unknown",
    }
}

fn cleanup_warning(value: DurableCleanupWarning) -> &'static str {
    match value {
        DurableCleanupWarning::EstimatedBytesUnverified => "estimated_bytes_unverified",
        DurableCleanupWarning::DryRunDoesNotMutate => "dry_run_does_not_mutate",
        DurableCleanupWarning::TrashDoesNotFreeSpaceImmediately => {
            "trash_does_not_free_space_immediately"
        }
        DurableCleanupWarning::PermanentRemovalCannotBeUndone => {
            "permanent_removal_cannot_be_undone"
        }
        DurableCleanupWarning::CloudEvictionRequiresNetworkToRedownload => {
            "cloud_eviction_requires_network_to_redownload"
        }
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_scan_detail_json_is_a_golden_path_free_shape() {
        let document = ScanDetailDocument {
            schema_version: JSON_SCHEMA_VERSION,
            command: "scan-detail",
            path_disclosure: "none",
            scan_id: "scan:one".to_owned(),
            page: OffsetPageDocument {
                schema_version: JSON_SCHEMA_VERSION,
                requested_offset: 0,
                requested_limit: 20,
                next_offset: None,
                has_more: false,
            },
            coverage: CoverageDetailDocument {
                schema_version: JSON_SCHEMA_VERSION,
                status: "complete",
                measured_permille: Some(1000),
                issue_record_count: 0,
                issue_occurrence_count: 0,
            },
            items: Vec::new(),
        };
        assert_eq!(
            serialize_json(&document).unwrap(),
            "{\n  \"schema_version\": 1,\n  \"command\": \"scan-detail\",\n  \"path_disclosure\": \"none\",\n  \"scan_id\": \"scan:one\",\n  \"page\": {\n    \"schema_version\": 1,\n    \"requested_offset\": 0,\n    \"requested_limit\": 20,\n    \"next_offset\": null,\n    \"has_more\": false\n  },\n  \"coverage\": {\n    \"schema_version\": 1,\n    \"status\": \"complete\",\n    \"measured_permille\": 1000,\n    \"issue_record_count\": 0,\n    \"issue_occurrence_count\": 0\n  },\n  \"items\": []\n}"
        );
    }

    #[test]
    fn candidate_evaluation_shapes_preserve_null_semantics() {
        let cases = [
            (
                DurableCandidateEvaluationStatus::NotRun,
                r#"{"schema_version":1,"status":"not_run","candidate_count":null,"failure":null}"#,
            ),
            (
                DurableCandidateEvaluationStatus::Pending,
                r#"{"schema_version":1,"status":"pending","candidate_count":null,"failure":null}"#,
            ),
            (
                DurableCandidateEvaluationStatus::Succeeded { candidate_count: 3 },
                r#"{"schema_version":1,"status":"succeeded","candidate_count":3,"failure":null}"#,
            ),
            (
                DurableCandidateEvaluationStatus::Failed {
                    kind: CandidateEvaluationTaskFailureKind::LimitExceeded,
                },
                r#"{"schema_version":1,"status":"failed","candidate_count":null,"failure":"limit_exceeded"}"#,
            ),
        ];
        for (status, expected) in cases {
            assert_eq!(
                serde_json::to_string(&CandidateEvaluationDocument::from(status)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn review_json_explicitly_denies_cleanup_authority() {
        let document = ReviewStateDocument {
            schema_version: JSON_SCHEMA_VERSION,
            command: "review-state",
            path_disclosure: "none",
            scan_id: "scan:one",
            candidate_id: "candidate:one",
            requested_command: "select",
            status: "selected",
            cleanup_performed: false,
        };
        assert_eq!(
            serde_json::to_string(&document).unwrap(),
            r#"{"schema_version":1,"command":"review-state","path_disclosure":"none","scan_id":"scan:one","candidate_id":"candidate:one","requested_command":"select","status":"selected","cleanup_performed":false}"#
        );
    }

    #[test]
    fn empty_cleanup_list_json_is_a_golden_keyset_shape() {
        let document = CleanupHistoryListDocument {
            schema_version: JSON_SCHEMA_VERSION,
            command: "cleanup-history",
            operation: "list",
            path_disclosure: "none",
            order: CLEANUP_ORDER,
            requested_limit: 20,
            has_more: false,
            next_cursor: None,
            items: Vec::new(),
        };
        assert_eq!(
            serde_json::to_string(&document).unwrap(),
            r#"{"schema_version":1,"command":"cleanup-history","operation":"list","path_disclosure":"none","order":"started_at_desc_session_id_asc","requested_limit":20,"has_more":false,"next_cursor":null,"items":[]}"#
        );
    }

    #[test]
    fn complete_candidate_json_is_a_golden_path_free_shape() {
        let document = CandidateDocument {
            schema_version: JSON_SCHEMA_VERSION,
            candidate_id: "candidate:one".to_owned(),
            rule_id: "developer.rust.target".to_owned(),
            rule_revision: 1,
            category: "developer_artifact",
            estimated_bytes: 42,
            newest_mtime_unix_ms: Some(1_750_000_000_000),
            safety: "safe_regenerable",
            action: "remove_known_regenerable_contents",
            rule_schedule_eligible: true,
            path_count: 1,
            evidence_kinds: vec!["matched_path", "minimum_age"],
            blockers: Vec::new(),
            created_at_unix_ms: 1_750_000_000_100,
            status: "discovered",
        };
        assert_eq!(
            serde_json::to_string(&document).unwrap(),
            r#"{"schema_version":1,"candidate_id":"candidate:one","rule_id":"developer.rust.target","rule_revision":1,"category":"developer_artifact","estimated_bytes":42,"newest_mtime_unix_ms":1750000000000,"safety":"safe_regenerable","action":"remove_known_regenerable_contents","rule_schedule_eligible":true,"path_count":1,"evidence_kinds":["matched_path","minimum_age"],"blockers":[],"created_at_unix_ms":1750000000100,"status":"discovered"}"#
        );
    }

    #[test]
    fn stable_error_maps_cover_not_found_unknown_and_paging() {
        let errors = [
            (scan_not_found(), "scan_not_found"),
            (candidate_not_found(), "candidate_not_found"),
            (cursor_out_of_range(), "cursor_out_of_range"),
            (outcome_unknown(), "outcome_unknown"),
            (
                map_cleanup_history_error(CleanupHistoryError::SessionNotFound),
                "cleanup_session_not_found",
            ),
        ];
        for (error, code) in errors {
            assert_eq!(error.code().as_str(), code);
            assert!(!error.code().retryable());
        }
    }

    #[test]
    fn mapping_tables_cover_all_frozen_policy_values() {
        assert_eq!(
            candidate_category(CandidateCategory::CloudFile),
            "cloud_file"
        );
        assert_eq!(
            candidate_action(CandidateAction::RemoveKnownRegenerableContents),
            "remove_known_regenerable_contents"
        );
        assert_eq!(
            block_reason(BlockReason::ChangedSinceScan),
            "changed_since_scan"
        );
        assert_eq!(
            cleanup_item_status(DurableCleanupItemStatus::OutcomeUnknown),
            "outcome_unknown"
        );
        assert_eq!(
            cleanup_warning(DurableCleanupWarning::TrashDoesNotFreeSpaceImmediately),
            "trash_does_not_free_space_immediately"
        );
        assert_eq!(
            coverage_status(dux_core::ScanCoverageStatus::Partial),
            "partial"
        );
    }
}
