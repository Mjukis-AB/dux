use super::*;

pub(in crate::persistence) fn load_candidate_record(
    connection: &Connection,
    id: &CandidateId,
) -> Result<Option<StoredCandidateRecord>, HistoryError> {
    // Product-facing exact loads own a fresh query envelope and preflight the
    // decoded graph before copying payloads. Cleanup-history callers use the
    // `_within_budget` form only after their enclosing aggregate query has
    // already bounded the same data; repeating these scans there would reject
    // the maximum legal cleanup/journal contracts under the shared VM limit.
    run_bounded_query(connection, || {
        ensure_stored_candidate_budget(connection, id)?;
        load_candidate_record_within_budget(connection, id)
    })
}

pub(in crate::persistence) fn load_candidate_record_within_budget(
    connection: &Connection,
    id: &CandidateId,
) -> Result<Option<StoredCandidateRecord>, HistoryError> {
    load_candidate_record_within_budget_and_hook(connection, id, |_| Ok(()))
}

/// Load one evaluator-owned scan batch without issuing one parent/child query
/// per candidate. The caller owns the shared query progress budget, so every
/// statement is set-based and carries a SQL-side upper bound before rows are
/// materialized.
pub(in crate::persistence) fn load_complete_candidate_batch_within_budget(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
) -> Result<Vec<CompleteCandidateRecord>, HistoryError> {
    ensure_stored_candidate_batch_budget(connection, scan_id, maximum)?;
    let row_limit =
        i64::try_from(maximum.checked_add(1).ok_or_else(corrupt)?).map_err(|_| corrupt())?;
    let mut statement = connection
        .prepare(
            "SELECT record_format_version,
                    typeof(candidate_id), length(CAST(candidate_id AS BLOB)), candidate_id,
                    typeof(scan_id), length(CAST(scan_id AS BLOB)), scan_id,
                    typeof(rule_id), length(CAST(rule_id AS BLOB)), rule_id,
                    rule_revision,
                    typeof(safety_tier), length(CAST(safety_tier AS BLOB)), safety_tier,
                    estimated_bytes, created_at_unix_ms,
                    typeof(status), length(CAST(status AS BLOB)), status,
                    typeof(category), length(CAST(category AS BLOB)), category,
                    typeof(proposed_action), length(CAST(proposed_action AS BLOB)), proposed_action,
                    rule_schedule_eligible,
                    newest_mtime_unix_seconds, newest_mtime_nanoseconds
             FROM candidates INDEXED BY candidates_by_scan_time
             WHERE scan_id = ?1
             ORDER BY created_at_unix_ms DESC, candidate_id
             LIMIT ?2",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![scan_id.as_str(), row_limit])
        .map_err(map_query_sql_error)?;
    let mut batch = Vec::with_capacity(maximum.min(256));
    let mut indices = HashMap::with_capacity(maximum.min(256));
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if batch.len() >= maximum {
            return Err(corrupt());
        }
        let raw = raw_candidate_row(row).map_err(map_query_sql_error)?;
        if raw.record_format_version != 2 {
            return Err(corrupt());
        }
        let common = decode_common(&raw)?;
        if common.source_scan_id != *scan_id || indices.contains_key(&common.id) {
            return Err(corrupt());
        }
        let category = category_from_stored(raw.category.as_deref().ok_or_else(corrupt)?)?;
        let action = action_from_stored(raw.action.as_deref().ok_or_else(corrupt)?)?;
        let rule_schedule_eligible = stored_bool(raw.rule_schedule_eligible.ok_or_else(corrupt)?)?;
        validate_policy(common.safety, action, rule_schedule_eligible, corrupt)?;
        let newest_mtime =
            decode_optional_time(raw.newest_mtime_seconds, raw.newest_mtime_nanoseconds)?;
        let index = batch.len();
        indices.insert(common.id.clone(), index);
        batch.push(BatchCandidate {
            common,
            category,
            action,
            rule_schedule_eligible,
            newest_mtime,
            paths: Vec::new(),
            evidence: Vec::new(),
            blockers: Vec::new(),
        });
    }
    drop(rows);
    drop(statement);
    if batch.is_empty() {
        return Ok(Vec::new());
    }

    load_batch_paths(connection, scan_id, maximum, &indices, &mut batch)?;
    load_batch_evidence(connection, scan_id, maximum, &indices, &mut batch)?;
    load_batch_blockers(connection, scan_id, maximum, &indices, &mut batch)?;
    validate_batch_plan_claims(connection, scan_id, maximum, &indices, &batch)?;

    batch
        .into_iter()
        .map(|candidate| {
            if candidate.paths.is_empty() || candidate.evidence.is_empty() {
                return Err(corrupt());
            }
            validate_complete_children(
                candidate.common.safety,
                candidate.action,
                &candidate.paths,
                &candidate.evidence,
            )?;
            Ok(CompleteCandidateRecord {
                id: candidate.common.id,
                source_scan_id: candidate.common.source_scan_id,
                rule: candidate.common.rule,
                category: candidate.category,
                paths: candidate.paths,
                estimated_bytes: candidate.common.estimated_bytes,
                newest_mtime: candidate.newest_mtime,
                evidence: candidate.evidence,
                safety: candidate.common.safety,
                action: candidate.action,
                rule_schedule_eligible: candidate.rule_schedule_eligible,
                blockers: candidate.blockers,
                created_at: candidate.common.created_at,
                status: candidate.common.status,
            })
        })
        .collect()
}

pub(super) fn ensure_stored_candidate_batch_budget(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
) -> Result<(), HistoryError> {
    let parent_limit = batch_parent_limit(maximum)?;
    let (candidates, invalid_candidates) = bounded_batch_validation(
        connection,
        "SELECT count(*), COALESCE(sum(invalid), 0) FROM (
             SELECT CASE WHEN
                        typeof(record_format_version) != 'integer' OR
                        record_format_version != 2 OR
                        typeof(candidate_id) != 'text' OR
                        length(CAST(candidate_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                        typeof(scan_id) != 'text' OR
                        length(CAST(scan_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                        typeof(rule_id) != 'text' OR
                        length(CAST(rule_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                        typeof(rule_revision) != 'integer' OR
                        typeof(safety_tier) != 'text' OR
                        length(CAST(safety_tier AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(estimated_bytes) != 'integer' OR
                        typeof(created_at_unix_ms) != 'integer' OR
                        typeof(status) != 'text' OR
                        length(CAST(status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(category) != 'text' OR
                        length(CAST(category AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(proposed_action) != 'text' OR
                        length(CAST(proposed_action AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(rule_schedule_eligible) != 'integer' OR
                        typeof(newest_mtime_unix_seconds) NOT IN ('null', 'integer') OR
                        typeof(newest_mtime_nanoseconds) NOT IN ('null', 'integer')
                    THEN 1 ELSE 0 END AS invalid
             FROM candidates INDEXED BY candidates_by_scan_time
             WHERE scan_id = ?1 LIMIT ?2
         )",
        scan_id,
        parent_limit,
    )?;
    if invalid_candidates != 0 || candidates > u64::try_from(maximum).map_err(|_| corrupt())? {
        return Err(corrupt());
    }

    let (paths, path_payload_bytes, invalid_paths) = bounded_batch_payload_usage(
        connection,
        "SELECT count(*), COALESCE(sum(payload_bytes), 0),
                COALESCE(sum(invalid), 0)
         FROM (
             SELECT CASE WHEN typeof(path.observed_path) = 'blob'
                         THEN length(path.observed_path) ELSE 0 END AS payload_bytes,
                    CASE WHEN typeof(path.path_ordinal) != 'integer' OR
                                   typeof(path.observed_path) != 'blob' OR
                                   length(path.observed_path) NOT BETWEEN 1 AND 65536
                                   OR typeof(path.observed_path_encoding) != 'integer'
                         THEN 1 ELSE 0 END AS invalid
             FROM candidates AS candidate INDEXED BY candidates_by_scan_time
             JOIN candidate_paths AS path USING (candidate_id)
             WHERE candidate.scan_id = ?1 LIMIT ?2
         )",
        scan_id,
        batch_child_limit(maximum, MAX_PATHS)?,
    )?;
    let (evidence, evidence_payload_bytes, invalid_evidence) = bounded_batch_payload_usage(
        connection,
        "SELECT count(*), COALESCE(sum(payload_bytes), 0),
                COALESCE(sum(invalid), 0)
         FROM (
             SELECT CASE WHEN typeof(evidence.path_value) = 'blob'
                         THEN length(evidence.path_value) ELSE 0 END +
                    CASE WHEN typeof(evidence.text_value) = 'text'
                         THEN length(CAST(evidence.text_value AS BLOB)) ELSE 0 END
                        AS payload_bytes,
                    CASE WHEN
                        typeof(evidence.evidence_kind) != 'text' OR
                        length(CAST(evidence.evidence_kind AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(evidence.evidence_ordinal) != 'integer' OR
                        NOT (
                            (typeof(evidence.path_value) = 'null' AND
                             length(evidence.path_value) IS NULL) OR
                            (typeof(evidence.path_value) = 'blob' AND
                             length(evidence.path_value) BETWEEN 1 AND 65536)
                        ) OR
                        NOT (
                            (typeof(evidence.text_value) = 'null' AND
                             length(CAST(evidence.text_value AS BLOB)) IS NULL) OR
                            (typeof(evidence.text_value) = 'text' AND
                             length(CAST(evidence.text_value AS BLOB)) BETWEEN 1 AND 4096)
                        ) OR
                        typeof(evidence.path_value_encoding) NOT IN ('null', 'integer') OR
                        typeof(evidence.observed_unix_seconds) NOT IN ('null', 'integer') OR
                        typeof(evidence.observed_nanoseconds) NOT IN ('null', 'integer') OR
                        typeof(evidence.duration_seconds) NOT IN ('null', 'integer') OR
                        typeof(evidence.duration_nanoseconds) NOT IN ('null', 'integer') OR
                        typeof(evidence.observed_bytes) NOT IN ('null', 'integer') OR
                        typeof(evidence.minimum_bytes) NOT IN ('null', 'integer')
                    THEN 1 ELSE 0 END AS invalid
             FROM candidates AS candidate INDEXED BY candidates_by_scan_time
             JOIN candidate_evidence AS evidence USING (candidate_id)
             WHERE candidate.scan_id = ?1 LIMIT ?2
         )",
        scan_id,
        batch_child_limit(maximum, MAX_EVIDENCE)?,
    )?;
    let (blockers, invalid_blockers) = bounded_batch_validation(
        connection,
        "SELECT count(*), COALESCE(sum(invalid), 0) FROM (
             SELECT CASE WHEN typeof(blocker.blocker_ordinal) != 'integer' OR
                                   typeof(blocker.blocker_kind) != 'text' OR
                                   length(CAST(blocker.blocker_kind AS BLOB)) NOT BETWEEN 1 AND 64
                         THEN 1 ELSE 0 END AS invalid
             FROM candidates AS candidate INDEXED BY candidates_by_scan_time
             JOIN candidate_blockers AS blocker USING (candidate_id)
             WHERE candidate.scan_id = ?1 LIMIT ?2
         )",
        scan_id,
        batch_child_limit(maximum, MAX_BLOCKERS)?,
    )?;
    let (_, invalid_claims) = bounded_batch_validation(
        connection,
        "SELECT count(*), COALESCE(sum(invalid), 0) FROM (
             SELECT CASE WHEN
                        typeof(claim.prior_review_status) != 'text' OR
                        length(CAST(claim.prior_review_status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(session.candidate_status_coupling_version) != 'integer' OR
                        typeof(session.status) != 'text' OR
                        length(CAST(session.status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(session.record_format_version) != 'integer' OR
                        typeof(item.candidate_id) != 'text' OR
                        length(CAST(item.candidate_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                        typeof(item.record_format_version) != 'integer'
                    THEN 1 ELSE 0 END AS invalid
             FROM candidates AS candidate INDEXED BY candidates_by_scan_time
             JOIN candidate_plan_claims AS claim USING (candidate_id)
             LEFT JOIN cleanup_sessions AS session
               ON session.session_id = claim.session_id
             LEFT JOIN cleanup_items AS item
               ON item.session_id = claim.session_id
              AND item.item_ordinal = claim.item_ordinal
             WHERE candidate.scan_id = ?1 LIMIT ?2
         )",
        scan_id,
        parent_limit,
    )?;
    if invalid_paths != 0
        || invalid_evidence != 0
        || invalid_blockers != 0
        || invalid_claims != 0
        || paths > u64::try_from(maximum.saturating_mul(MAX_PATHS)).map_err(|_| corrupt())?
        || evidence > u64::try_from(maximum.saturating_mul(MAX_EVIDENCE)).map_err(|_| corrupt())?
        || blockers > u64::try_from(maximum.saturating_mul(MAX_BLOCKERS)).map_err(|_| corrupt())?
    {
        return Err(corrupt());
    }
    CandidateBatchUsage {
        candidates,
        paths,
        path_payload_bytes,
        evidence,
        evidence_payload_bytes,
        blockers,
    }
    .ensure_within_budget(HistoryErrorKind::QueryLimitExceeded)
}

fn bounded_batch_validation(
    connection: &Connection,
    sql: &str,
    scan_id: &ScanId,
    limit: i64,
) -> Result<(u64, u64), HistoryError> {
    let (count, invalid) = connection
        .query_row(sql, params![scan_id.as_str(), limit], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(map_query_sql_error)?;
    Ok((
        u64::try_from(count).map_err(|_| corrupt())?,
        u64::try_from(invalid).map_err(|_| corrupt())?,
    ))
}

fn bounded_batch_payload_usage(
    connection: &Connection,
    sql: &str,
    scan_id: &ScanId,
    limit: i64,
) -> Result<(u64, u64, u64), HistoryError> {
    let (count, bytes, invalid) = connection
        .query_row(sql, params![scan_id.as_str(), limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(map_query_sql_error)?;
    Ok((
        u64::try_from(count).map_err(|_| corrupt())?,
        u64::try_from(bytes).map_err(|_| corrupt())?,
        u64::try_from(invalid).map_err(|_| corrupt())?,
    ))
}

/// Preflight one exact candidate before any payload-bearing child query. The
/// scan-wide loader has the same aggregate guard, but review/detail operations
/// also call this exact loader directly and must not be able to materialize a
/// schema-shaped graph above the shared decoded-memory budget.
pub(super) fn ensure_stored_candidate_budget(
    connection: &Connection,
    id: &CandidateId,
) -> Result<(), HistoryError> {
    let (count, invalid, format): (i64, i64, i64) = connection
        .query_row(
            "SELECT count(*), COALESCE(sum(invalid), 0), COALESCE(max(format), 0)
             FROM (
                 SELECT CASE WHEN typeof(record_format_version) = 'integer'
                                  THEN record_format_version ELSE -1 END AS format,
                        CASE WHEN
                            typeof(record_format_version) != 'integer' OR
                            record_format_version NOT IN (1, 2) OR
                            typeof(candidate_id) != 'text' OR
                            length(CAST(candidate_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                            typeof(scan_id) != 'text' OR
                            length(CAST(scan_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                            typeof(rule_id) != 'text' OR
                            length(CAST(rule_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                            typeof(rule_revision) != 'integer' OR
                            typeof(safety_tier) != 'text' OR
                            length(CAST(safety_tier AS BLOB)) NOT BETWEEN 1 AND 64 OR
                            typeof(estimated_bytes) != 'integer' OR
                            typeof(created_at_unix_ms) != 'integer' OR
                            typeof(status) != 'text' OR
                            length(CAST(status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                            (record_format_version = 1 AND (
                                typeof(category) != 'null' OR
                                typeof(proposed_action) != 'null' OR
                                typeof(rule_schedule_eligible) != 'null' OR
                                typeof(newest_mtime_unix_seconds) != 'null' OR
                                typeof(newest_mtime_nanoseconds) != 'null'
                            )) OR
                            (record_format_version = 2 AND (
                                typeof(category) != 'text' OR
                                length(CAST(category AS BLOB)) NOT BETWEEN 1 AND 64 OR
                                typeof(proposed_action) != 'text' OR
                                length(CAST(proposed_action AS BLOB)) NOT BETWEEN 1 AND 64 OR
                                typeof(rule_schedule_eligible) != 'integer' OR
                                typeof(newest_mtime_unix_seconds) NOT IN ('null', 'integer') OR
                                typeof(newest_mtime_nanoseconds) NOT IN ('null', 'integer')
                            ))
                        THEN 1 ELSE 0 END AS invalid
                 FROM candidates WHERE candidate_id = ?1 LIMIT 2
             )",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(map_query_sql_error)?;
    if count == 0 {
        return Ok(());
    }
    if count != 1 || invalid != 0 || !matches!(format, 1 | 2) {
        return Err(corrupt());
    }
    if format == 1 {
        return Ok(());
    }

    let (paths, path_payload_bytes, invalid_paths) = exact_payload_usage(
        connection,
        "SELECT count(*), COALESCE(sum(payload_bytes), 0),
                COALESCE(sum(invalid), 0)
         FROM (
             SELECT CASE WHEN typeof(observed_path) = 'blob'
                         THEN length(observed_path) ELSE 0 END AS payload_bytes,
                    CASE WHEN typeof(path_ordinal) != 'integer' OR
                                   typeof(observed_path) != 'blob' OR
                                   length(observed_path) NOT BETWEEN 1 AND 65536 OR
                                   typeof(observed_path_encoding) != 'integer'
                         THEN 1 ELSE 0 END AS invalid
             FROM candidate_paths WHERE candidate_id = ?1 LIMIT ?2
         )",
        id,
        MAX_PATHS + 1,
    )?;
    let (evidence, evidence_payload_bytes, invalid_evidence) = exact_payload_usage(
        connection,
        "SELECT count(*), COALESCE(sum(payload_bytes), 0),
                COALESCE(sum(invalid), 0)
         FROM (
             SELECT CASE WHEN typeof(path_value) = 'blob'
                         THEN length(path_value) ELSE 0 END +
                    CASE WHEN typeof(text_value) = 'text'
                         THEN length(CAST(text_value AS BLOB)) ELSE 0 END
                        AS payload_bytes,
                    CASE WHEN
                        typeof(evidence_kind) != 'text' OR
                        length(CAST(evidence_kind AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(evidence_ordinal) != 'integer' OR
                        NOT (
                            (typeof(path_value) = 'null' AND length(path_value) IS NULL) OR
                            (typeof(path_value) = 'blob' AND
                             length(path_value) BETWEEN 1 AND 65536)
                        ) OR
                        NOT (
                            (typeof(text_value) = 'null' AND
                             length(CAST(text_value AS BLOB)) IS NULL) OR
                            (typeof(text_value) = 'text' AND
                             length(CAST(text_value AS BLOB)) BETWEEN 1 AND 4096)
                        ) OR
                        typeof(path_value_encoding) NOT IN ('null', 'integer') OR
                        typeof(observed_unix_seconds) NOT IN ('null', 'integer') OR
                        typeof(observed_nanoseconds) NOT IN ('null', 'integer') OR
                        typeof(duration_seconds) NOT IN ('null', 'integer') OR
                        typeof(duration_nanoseconds) NOT IN ('null', 'integer') OR
                        typeof(observed_bytes) NOT IN ('null', 'integer') OR
                        typeof(minimum_bytes) NOT IN ('null', 'integer')
                    THEN 1 ELSE 0 END AS invalid
             FROM candidate_evidence WHERE candidate_id = ?1 LIMIT ?2
         )",
        id,
        MAX_EVIDENCE + 1,
    )?;
    let (blockers, invalid_blockers) = exact_validation_usage(
        connection,
        "SELECT count(*), COALESCE(sum(invalid), 0) FROM (
             SELECT CASE WHEN typeof(blocker_ordinal) != 'integer' OR
                                   typeof(blocker_kind) != 'text' OR
                                   length(CAST(blocker_kind AS BLOB)) NOT BETWEEN 1 AND 64
                         THEN 1 ELSE 0 END AS invalid
             FROM candidate_blockers WHERE candidate_id = ?1 LIMIT ?2
         )",
        id,
        MAX_BLOCKERS + 1,
    )?;
    let (claims, invalid_claims) = exact_validation_usage(
        connection,
        "SELECT count(*), COALESCE(sum(invalid), 0) FROM (
             SELECT CASE WHEN
                        typeof(claim.prior_review_status) != 'text' OR
                        length(CAST(claim.prior_review_status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(session.candidate_status_coupling_version) != 'integer' OR
                        typeof(session.status) != 'text' OR
                        length(CAST(session.status AS BLOB)) NOT BETWEEN 1 AND 64 OR
                        typeof(session.record_format_version) != 'integer' OR
                        typeof(item.candidate_id) != 'text' OR
                        length(CAST(item.candidate_id AS BLOB)) NOT BETWEEN 1 AND 128 OR
                        typeof(item.record_format_version) != 'integer'
                    THEN 1 ELSE 0 END AS invalid
             FROM candidate_plan_claims AS claim
             LEFT JOIN cleanup_sessions AS session
               ON session.session_id = claim.session_id
             LEFT JOIN cleanup_items AS item
               ON item.session_id = claim.session_id
              AND item.item_ordinal = claim.item_ordinal
             WHERE claim.candidate_id = ?1 LIMIT ?2
         )",
        id,
        2,
    )?;
    if invalid_paths != 0
        || invalid_evidence != 0
        || invalid_blockers != 0
        || invalid_claims != 0
        || paths > MAX_PATHS as u64
        || evidence > MAX_EVIDENCE as u64
        || blockers > MAX_BLOCKERS as u64
        || claims > 1
    {
        return Err(corrupt());
    }
    CandidateBatchUsage {
        candidates: 1,
        paths,
        path_payload_bytes,
        evidence,
        evidence_payload_bytes,
        blockers,
    }
    .ensure_within_budget(HistoryErrorKind::QueryLimitExceeded)
}

fn exact_validation_usage(
    connection: &Connection,
    sql: &str,
    id: &CandidateId,
    limit: usize,
) -> Result<(u64, u64), HistoryError> {
    let limit = i64::try_from(limit).map_err(|_| corrupt())?;
    let (count, invalid) = connection
        .query_row(sql, params![id.as_str(), limit], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(map_query_sql_error)?;
    Ok((
        u64::try_from(count).map_err(|_| corrupt())?,
        u64::try_from(invalid).map_err(|_| corrupt())?,
    ))
}

fn exact_payload_usage(
    connection: &Connection,
    sql: &str,
    id: &CandidateId,
    limit: usize,
) -> Result<(u64, u64, u64), HistoryError> {
    let limit = i64::try_from(limit).map_err(|_| corrupt())?;
    let (count, bytes, invalid) = connection
        .query_row(sql, params![id.as_str(), limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(map_query_sql_error)?;
    Ok((
        u64::try_from(count).map_err(|_| corrupt())?,
        u64::try_from(bytes).map_err(|_| corrupt())?,
        u64::try_from(invalid).map_err(|_| corrupt())?,
    ))
}

pub(super) fn load_candidate_record_within_budget_and_hook(
    connection: &Connection,
    id: &CandidateId,
    after_source_scan: impl FnOnce(&Connection) -> Result<(), HistoryError>,
) -> Result<Option<StoredCandidateRecord>, HistoryError> {
    let raw = connection
            .query_row(
                "SELECT record_format_version,
                        typeof(candidate_id), length(CAST(candidate_id AS BLOB)), candidate_id,
                        typeof(scan_id), length(CAST(scan_id AS BLOB)), scan_id,
                        typeof(rule_id), length(CAST(rule_id AS BLOB)), rule_id,
                        rule_revision,
                        typeof(safety_tier), length(CAST(safety_tier AS BLOB)), safety_tier,
                        estimated_bytes, created_at_unix_ms,
                        typeof(status), length(CAST(status AS BLOB)), status,
                        typeof(category), length(CAST(category AS BLOB)), category,
                        typeof(proposed_action), length(CAST(proposed_action AS BLOB)), proposed_action,
                        rule_schedule_eligible,
                        newest_mtime_unix_seconds, newest_mtime_nanoseconds
                 FROM candidates WHERE candidate_id = ?1",
                [id.as_str()],
                raw_candidate_row,
            )
            .optional()
            .map_err(map_query_sql_error)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let common = decode_common(&raw)?;
    match raw.record_format_version {
        1 => Ok(Some(StoredCandidateRecord::LegacySummary({
            ensure_source_scan_exists(connection, &common.source_scan_id)?;
            if raw.category.is_some()
                || raw.action.is_some()
                || raw.rule_schedule_eligible.is_some()
                || raw.newest_mtime_seconds.is_some()
                || raw.newest_mtime_nanoseconds.is_some()
            {
                return Err(corrupt());
            }
            ensure_no_candidate_children(connection, id)?;
            LegacyCandidateSummary {
                id: common.id,
                source_scan_id: common.source_scan_id,
                rule: common.rule,
                safety: common.safety,
                estimated_bytes: common.estimated_bytes,
                created_at: common.created_at,
                status: common.status,
            }
        }))),
        2 => {
            ensure_source_scan_succeeded(connection, &common.source_scan_id)?;
            after_source_scan(connection)?;
            let paths = load_paths(connection, id)?;
            let evidence = load_evidence(connection, id)?;
            let blockers = load_blockers(connection, id)?;
            let category = category_from_stored(raw.category.as_deref().ok_or_else(corrupt)?)?;
            let action = action_from_stored(raw.action.as_deref().ok_or_else(corrupt)?)?;
            let schedule = raw.rule_schedule_eligible.ok_or_else(corrupt)?;
            let rule_schedule_eligible = stored_bool(schedule)?;
            validate_policy(common.safety, action, rule_schedule_eligible, corrupt)?;
            let newest_mtime =
                decode_optional_time(raw.newest_mtime_seconds, raw.newest_mtime_nanoseconds)?;
            validate_complete_children(common.safety, action, &paths, &evidence)?;
            validate_candidate_plan_claim_state(connection, &common.id, common.status)?;
            Ok(Some(StoredCandidateRecord::Complete(
                CompleteCandidateRecord {
                    id: common.id,
                    source_scan_id: common.source_scan_id,
                    rule: common.rule,
                    category,
                    paths,
                    estimated_bytes: common.estimated_bytes,
                    newest_mtime,
                    evidence,
                    safety: common.safety,
                    action,
                    rule_schedule_eligible,
                    blockers,
                    created_at: common.created_at,
                    status: common.status,
                },
            )))
        }
        _ => Err(corrupt()),
    }
}

fn validate_candidate_plan_claim_state(
    connection: &Connection,
    id: &CandidateId,
    status: CandidateHistoryStatus,
) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT typeof(claim.prior_review_status),
                    length(CAST(claim.prior_review_status AS BLOB)),
                    claim.prior_review_status,
                    session.candidate_status_coupling_version,
                    typeof(session.status), length(CAST(session.status AS BLOB)),
                    session.status, session.record_format_version,
                    typeof(item.candidate_id), length(CAST(item.candidate_id AS BLOB)),
                    item.candidate_id, item.record_format_version
             FROM candidate_plan_claims AS claim
             LEFT JOIN cleanup_sessions AS session
               ON session.session_id = claim.session_id
             LEFT JOIN cleanup_items AS item
               ON item.session_id = claim.session_id
              AND item.item_ordinal = claim.item_ordinal
             WHERE claim.candidate_id = ?1
             LIMIT 2",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([id.as_str()])
        .map_err(map_query_sql_error)?;
    let first = rows.next().map_err(map_query_sql_error)?;
    if status != CandidateHistoryStatus::Planned {
        return if first.is_none() {
            Ok(())
        } else {
            Err(corrupt())
        };
    }
    let row = first.ok_or_else(corrupt)?;
    validate_required_value(row, 0, 1, "text", MAX_STORED_POLICY_BYTES)
        .map_err(map_query_sql_error)?;
    validate_required_value(row, 4, 5, "text", MAX_STORED_POLICY_BYTES)
        .map_err(map_query_sql_error)?;
    validate_required_value(row, 8, 9, "text", MAX_STORED_ID_BYTES).map_err(map_query_sql_error)?;
    let prior: String = row.get(2).map_err(map_query_sql_error)?;
    let coupling: Option<i64> = row.get(3).map_err(map_query_sql_error)?;
    let session_status: String = row.get(6).map_err(map_query_sql_error)?;
    let session_version: Option<i64> = row.get(7).map_err(map_query_sql_error)?;
    let item_candidate_id: String = row.get(10).map_err(map_query_sql_error)?;
    let item_version: Option<i64> = row.get(11).map_err(map_query_sql_error)?;
    if CandidatePriorReviewStatus::from_stored(&prior).is_err()
        || coupling != Some(2)
        || !matches!(
            session_status.as_str(),
            "planned" | "running" | "recovering"
        )
        || session_version != Some(2)
        || item_candidate_id != id.as_str()
        || item_version != Some(2)
        || rows.next().map_err(map_query_sql_error)?.is_some()
    {
        return Err(corrupt());
    }
    Ok(())
}

struct RawCandidateRow {
    record_format_version: i64,
    id: String,
    source_scan_id: String,
    rule_id: String,
    rule_revision: i64,
    safety: String,
    estimated_bytes: i64,
    created_at_unix_ms: i64,
    status: String,
    category: Option<String>,
    action: Option<String>,
    rule_schedule_eligible: Option<i64>,
    newest_mtime_seconds: Option<i64>,
    newest_mtime_nanoseconds: Option<i64>,
}

fn raw_candidate_row(row: &Row<'_>) -> rusqlite::Result<RawCandidateRow> {
    validate_required_value(row, 1, 2, "text", MAX_STORED_ID_BYTES)?;
    validate_required_value(row, 4, 5, "text", MAX_STORED_ID_BYTES)?;
    validate_required_value(row, 7, 8, "text", MAX_STORED_ID_BYTES)?;
    validate_required_value(row, 11, 12, "text", MAX_STORED_POLICY_BYTES)?;
    validate_required_value(row, 16, 17, "text", MAX_STORED_POLICY_BYTES)?;
    let record_format_version: i64 = row.get(0)?;
    match record_format_version {
        1 => {
            validate_null_value(row, 19, 20)?;
            validate_null_value(row, 22, 23)?;
        }
        2 => {
            validate_required_value(row, 19, 20, "text", MAX_STORED_POLICY_BYTES)?;
            validate_required_value(row, 22, 23, "text", MAX_STORED_POLICY_BYTES)?;
        }
        _ => return Err(rusqlite::Error::InvalidQuery),
    }
    Ok(RawCandidateRow {
        record_format_version,
        id: row.get(3)?,
        source_scan_id: row.get(6)?,
        rule_id: row.get(9)?,
        rule_revision: row.get(10)?,
        safety: row.get(13)?,
        estimated_bytes: row.get(14)?,
        created_at_unix_ms: row.get(15)?,
        status: row.get(18)?,
        category: row.get(21)?,
        action: row.get(24)?,
        rule_schedule_eligible: row.get(25)?,
        newest_mtime_seconds: row.get(26)?,
        newest_mtime_nanoseconds: row.get(27)?,
    })
}

struct DecodedCommon {
    id: CandidateId,
    source_scan_id: ScanId,
    rule: RuleRef,
    safety: SafetyTier,
    estimated_bytes: u64,
    created_at: SystemTime,
    status: CandidateHistoryStatus,
}

struct BatchCandidate {
    common: DecodedCommon,
    category: CandidateCategory,
    action: CandidateAction,
    rule_schedule_eligible: bool,
    newest_mtime: Option<SystemTime>,
    paths: Vec<PathBuf>,
    evidence: Vec<Evidence>,
    blockers: Vec<BlockReason>,
}

fn decode_common(raw: &RawCandidateRow) -> Result<DecodedCommon, HistoryError> {
    let id = CandidateId::new(raw.id.clone()).map_err(|_| corrupt())?;
    let source_scan_id = ScanId::new(raw.source_scan_id.clone()).map_err(|_| corrupt())?;
    let rule_id = RuleId::new(raw.rule_id.clone()).map_err(|_| corrupt())?;
    let revision = u32::try_from(raw.rule_revision).map_err(|_| corrupt())?;
    let rule_revision = RuleRevision::new(revision).map_err(|_| corrupt())?;
    Ok(DecodedCommon {
        id,
        source_scan_id,
        rule: RuleRef::new(rule_id, rule_revision),
        safety: safety_from_stored(&raw.safety)?,
        estimated_bytes: from_i64(raw.estimated_bytes)?,
        created_at: unix_ms_to_system_time(raw.created_at_unix_ms)?,
        status: CandidateHistoryStatus::from_stored(&raw.status)?,
    })
}

fn batch_child_limit(maximum: usize, per_candidate: usize) -> Result<i64, HistoryError> {
    i64::try_from(
        maximum
            .checked_mul(per_candidate)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(corrupt)?,
    )
    .map_err(|_| corrupt())
}

fn batch_parent_limit(maximum: usize) -> Result<i64, HistoryError> {
    i64::try_from(maximum.checked_add(1).ok_or_else(corrupt)?).map_err(|_| corrupt())
}

fn load_batch_paths(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
    indices: &HashMap<CandidateId, usize>,
    batch: &mut [BatchCandidate],
) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "WITH batch(candidate_id) AS MATERIALIZED (
                 SELECT candidate_id
                 FROM candidates INDEXED BY candidates_by_scan_time
                 WHERE scan_id = ?1
                 ORDER BY created_at_unix_ms DESC, candidate_id
                 LIMIT ?2
             ), bounded_path AS MATERIALIZED (
                 SELECT path.candidate_id, path.path_ordinal,
                        path.observed_path, path.observed_path_encoding
                 FROM batch
                 JOIN candidate_paths AS path USING (candidate_id)
                 LIMIT ?3
             )
             SELECT typeof(candidate_id),
                    length(CAST(candidate_id AS BLOB)), candidate_id,
                    path_ordinal, typeof(observed_path),
                    length(observed_path), observed_path,
                    observed_path_encoding
             FROM bounded_path
             ORDER BY candidate_id, path_ordinal",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![
            scan_id.as_str(),
            batch_parent_limit(maximum)?,
            batch_child_limit(maximum, MAX_PATHS)?,
        ])
        .map_err(map_query_sql_error)?;
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        validate_required_value(row, 0, 1, "text", MAX_STORED_ID_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 4, 5, "blob", MAX_STORED_PATH_BYTES)
            .map_err(map_query_sql_error)?;
        let id = CandidateId::new(row.get::<_, String>(2).map_err(map_query_sql_error)?)
            .map_err(|_| corrupt())?;
        let index = *indices.get(&id).ok_or_else(corrupt)?;
        let paths = &mut batch[index].paths;
        if paths.len() >= MAX_PATHS
            || row.get::<_, i64>(3).map_err(map_query_sql_error)? != paths.len() as i64
        {
            return Err(corrupt());
        }
        let path = decode_absolute_path(
            row.get(6).map_err(map_query_sql_error)?,
            row.get(7).map_err(map_query_sql_error)?,
        )?;
        if paths.contains(&path) {
            return Err(corrupt());
        }
        paths.push(path);
    }
    Ok(())
}

fn load_batch_evidence(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
    indices: &HashMap<CandidateId, usize>,
    batch: &mut [BatchCandidate],
) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "WITH batch(candidate_id) AS MATERIALIZED (
                 SELECT candidate_id
                 FROM candidates INDEXED BY candidates_by_scan_time
                 WHERE scan_id = ?1
                 ORDER BY created_at_unix_ms DESC, candidate_id
                 LIMIT ?2
             ), bounded_evidence AS MATERIALIZED (
                 SELECT evidence.candidate_id, evidence.evidence_ordinal,
                        evidence.evidence_kind, evidence.path_value,
                        evidence.path_value_encoding, evidence.text_value,
                        evidence.observed_unix_seconds, evidence.observed_nanoseconds,
                        evidence.duration_seconds, evidence.duration_nanoseconds,
                        evidence.observed_bytes, evidence.minimum_bytes
                 FROM batch
                 JOIN candidate_evidence AS evidence USING (candidate_id)
                 LIMIT ?3
             )
             SELECT typeof(candidate_id),
                    length(CAST(candidate_id AS BLOB)), candidate_id,
                    evidence_ordinal, typeof(evidence_kind),
                    length(CAST(evidence_kind AS BLOB)), evidence_kind,
                    typeof(path_value), length(path_value), path_value,
                    path_value_encoding, typeof(text_value),
                    length(CAST(text_value AS BLOB)), text_value,
                    observed_unix_seconds, observed_nanoseconds,
                    duration_seconds, duration_nanoseconds,
                    observed_bytes, minimum_bytes
             FROM bounded_evidence
             ORDER BY candidate_id, evidence_ordinal",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![
            scan_id.as_str(),
            batch_parent_limit(maximum)?,
            batch_child_limit(maximum, MAX_EVIDENCE)?,
        ])
        .map_err(map_query_sql_error)?;
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        validate_required_value(row, 0, 1, "text", MAX_STORED_ID_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 4, 5, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 7, 8, "blob", MAX_STORED_PATH_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 11, 12, "text", MAX_STORED_TEXT_BYTES)
            .map_err(map_query_sql_error)?;
        let id = CandidateId::new(row.get::<_, String>(2).map_err(map_query_sql_error)?)
            .map_err(|_| corrupt())?;
        let index = *indices.get(&id).ok_or_else(corrupt)?;
        let evidence = &mut batch[index].evidence;
        if evidence.len() >= MAX_EVIDENCE
            || row.get::<_, i64>(3).map_err(map_query_sql_error)? != evidence.len() as i64
        {
            return Err(corrupt());
        }
        evidence.push(decode_evidence(RawEvidence {
            kind: row.get(6).map_err(map_query_sql_error)?,
            path: row.get(9).map_err(map_query_sql_error)?,
            path_encoding: row.get(10).map_err(map_query_sql_error)?,
            text: row.get(13).map_err(map_query_sql_error)?,
            observed_seconds: row.get(14).map_err(map_query_sql_error)?,
            observed_nanoseconds: row.get(15).map_err(map_query_sql_error)?,
            duration_seconds: row.get(16).map_err(map_query_sql_error)?,
            duration_nanoseconds: row.get(17).map_err(map_query_sql_error)?,
            observed_bytes: row.get(18).map_err(map_query_sql_error)?,
            minimum_bytes: row.get(19).map_err(map_query_sql_error)?,
        })?);
    }
    Ok(())
}

fn load_batch_blockers(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
    indices: &HashMap<CandidateId, usize>,
    batch: &mut [BatchCandidate],
) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "WITH batch(candidate_id) AS MATERIALIZED (
                 SELECT candidate_id
                 FROM candidates INDEXED BY candidates_by_scan_time
                 WHERE scan_id = ?1
                 ORDER BY created_at_unix_ms DESC, candidate_id
                 LIMIT ?2
             ), bounded_blocker AS MATERIALIZED (
                 SELECT blocker.candidate_id, blocker.blocker_ordinal,
                        blocker.blocker_kind
                 FROM batch
                 JOIN candidate_blockers AS blocker USING (candidate_id)
                 LIMIT ?3
             )
             SELECT typeof(candidate_id),
                    length(CAST(candidate_id AS BLOB)), candidate_id,
                    blocker_ordinal, typeof(blocker_kind),
                    length(CAST(blocker_kind AS BLOB)), blocker_kind
             FROM bounded_blocker
             ORDER BY candidate_id, blocker_ordinal",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![
            scan_id.as_str(),
            batch_parent_limit(maximum)?,
            batch_child_limit(maximum, MAX_BLOCKERS)?,
        ])
        .map_err(map_query_sql_error)?;
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        validate_required_value(row, 0, 1, "text", MAX_STORED_ID_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 4, 5, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        let id = CandidateId::new(row.get::<_, String>(2).map_err(map_query_sql_error)?)
            .map_err(|_| corrupt())?;
        let index = *indices.get(&id).ok_or_else(corrupt)?;
        let blockers = &mut batch[index].blockers;
        if blockers.len() >= MAX_BLOCKERS
            || row.get::<_, i64>(3).map_err(map_query_sql_error)? != blockers.len() as i64
        {
            return Err(corrupt());
        }
        blockers.push(blocker_from_stored(
            &row.get::<_, String>(6).map_err(map_query_sql_error)?,
        )?);
    }
    Ok(())
}

fn validate_batch_plan_claims(
    connection: &Connection,
    scan_id: &ScanId,
    maximum: usize,
    indices: &HashMap<CandidateId, usize>,
    batch: &[BatchCandidate],
) -> Result<(), HistoryError> {
    let mut statement = connection
        .prepare(
            "WITH batch(candidate_id) AS MATERIALIZED (
                 SELECT candidate_id
                 FROM candidates INDEXED BY candidates_by_scan_time
                 WHERE scan_id = ?1
                 ORDER BY created_at_unix_ms DESC, candidate_id
                 LIMIT ?2
             ), bounded_claim AS MATERIALIZED (
                 SELECT claim.candidate_id, claim.prior_review_status,
                        session.candidate_status_coupling_version,
                        session.status AS session_status,
                        session.record_format_version AS session_record_format_version,
                        item.candidate_id AS item_candidate_id,
                        item.record_format_version AS item_record_format_version
                 FROM batch
                 JOIN candidate_plan_claims AS claim USING (candidate_id)
                 LEFT JOIN cleanup_sessions AS session
                   ON session.session_id = claim.session_id
                 LEFT JOIN cleanup_items AS item
                   ON item.session_id = claim.session_id
                  AND item.item_ordinal = claim.item_ordinal
                 LIMIT ?3
             )
             SELECT typeof(candidate_id),
                    length(CAST(candidate_id AS BLOB)), candidate_id,
                    typeof(prior_review_status),
                    length(CAST(prior_review_status AS BLOB)), prior_review_status,
                    candidate_status_coupling_version,
                    typeof(session_status), length(CAST(session_status AS BLOB)),
                    session_status, session_record_format_version,
                    typeof(item_candidate_id), length(CAST(item_candidate_id AS BLOB)),
                    item_candidate_id, item_record_format_version
             FROM bounded_claim
             ORDER BY candidate_id",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query(params![
            scan_id.as_str(),
            batch_parent_limit(maximum)?,
            batch_parent_limit(maximum)?,
        ])
        .map_err(map_query_sql_error)?;
    let mut claimed = HashSet::with_capacity(batch.len().min(256));
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        validate_required_value(row, 0, 1, "text", MAX_STORED_ID_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 3, 4, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 7, 8, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_required_value(row, 11, 12, "text", MAX_STORED_ID_BYTES)
            .map_err(map_query_sql_error)?;
        let id = CandidateId::new(row.get::<_, String>(2).map_err(map_query_sql_error)?)
            .map_err(|_| corrupt())?;
        let index = *indices.get(&id).ok_or_else(corrupt)?;
        let prior: String = row.get(5).map_err(map_query_sql_error)?;
        let coupling: Option<i64> = row.get(6).map_err(map_query_sql_error)?;
        let session_status: String = row.get(9).map_err(map_query_sql_error)?;
        let session_version: Option<i64> = row.get(10).map_err(map_query_sql_error)?;
        let item_candidate_id: String = row.get(13).map_err(map_query_sql_error)?;
        let item_version: Option<i64> = row.get(14).map_err(map_query_sql_error)?;
        if batch[index].common.status != CandidateHistoryStatus::Planned
            || !claimed.insert(id.clone())
            || CandidatePriorReviewStatus::from_stored(&prior).is_err()
            || coupling != Some(2)
            || !matches!(
                session_status.as_str(),
                "planned" | "running" | "recovering"
            )
            || session_version != Some(2)
            || item_candidate_id != id.as_str()
            || item_version != Some(2)
        {
            return Err(corrupt());
        }
    }
    if batch.iter().any(|candidate| {
        (candidate.common.status == CandidateHistoryStatus::Planned)
            != claimed.contains(&candidate.common.id)
    }) {
        return Err(corrupt());
    }
    Ok(())
}

fn load_paths(connection: &Connection, id: &CandidateId) -> Result<Vec<PathBuf>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT path_ordinal, typeof(observed_path), length(observed_path),
                    observed_path, observed_path_encoding
             FROM candidate_paths WHERE candidate_id = ?1
             ORDER BY path_ordinal LIMIT 257",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut paths = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if paths.len() >= MAX_PATHS {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != paths.len() as i64 {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "blob", MAX_STORED_PATH_BYTES)
            .map_err(map_query_sql_error)?;
        let bytes: Vec<u8> = row.get(3).map_err(map_query_sql_error)?;
        let encoding: i64 = row.get(4).map_err(map_query_sql_error)?;
        let path = decode_absolute_path(bytes, encoding)?;
        if paths.contains(&path) {
            return Err(corrupt());
        }
        paths.push(path);
    }
    if paths.is_empty() {
        return Err(corrupt());
    }
    Ok(paths)
}

fn load_evidence(connection: &Connection, id: &CandidateId) -> Result<Vec<Evidence>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT evidence_ordinal,
                    typeof(evidence_kind), length(CAST(evidence_kind AS BLOB)), evidence_kind,
                    typeof(path_value), length(path_value), path_value, path_value_encoding,
                    typeof(text_value), length(CAST(text_value AS BLOB)), text_value,
                    observed_unix_seconds, observed_nanoseconds,
                    duration_seconds, duration_nanoseconds, observed_bytes, minimum_bytes
             FROM candidate_evidence WHERE candidate_id = ?1
             ORDER BY evidence_ordinal LIMIT 513",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut evidence = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if evidence.len() >= MAX_EVIDENCE {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != evidence.len() as i64 {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 4, 5, "blob", MAX_STORED_PATH_BYTES)
            .map_err(map_query_sql_error)?;
        validate_optional_value(row, 8, 9, "text", MAX_STORED_TEXT_BYTES)
            .map_err(map_query_sql_error)?;
        let raw = RawEvidence {
            kind: row.get(3).map_err(map_query_sql_error)?,
            path: row.get(6).map_err(map_query_sql_error)?,
            path_encoding: row.get(7).map_err(map_query_sql_error)?,
            text: row.get(10).map_err(map_query_sql_error)?,
            observed_seconds: row.get(11).map_err(map_query_sql_error)?,
            observed_nanoseconds: row.get(12).map_err(map_query_sql_error)?,
            duration_seconds: row.get(13).map_err(map_query_sql_error)?,
            duration_nanoseconds: row.get(14).map_err(map_query_sql_error)?,
            observed_bytes: row.get(15).map_err(map_query_sql_error)?,
            minimum_bytes: row.get(16).map_err(map_query_sql_error)?,
        };
        evidence.push(decode_evidence(raw)?);
    }
    if evidence.is_empty() {
        return Err(corrupt());
    }
    Ok(evidence)
}

pub(in crate::persistence) struct RawEvidence {
    pub(in crate::persistence) kind: String,
    pub(in crate::persistence) path: Option<Vec<u8>>,
    pub(in crate::persistence) path_encoding: Option<i64>,
    pub(in crate::persistence) text: Option<String>,
    pub(in crate::persistence) observed_seconds: Option<i64>,
    pub(in crate::persistence) observed_nanoseconds: Option<i64>,
    pub(in crate::persistence) duration_seconds: Option<i64>,
    pub(in crate::persistence) duration_nanoseconds: Option<i64>,
    pub(in crate::persistence) observed_bytes: Option<i64>,
    pub(in crate::persistence) minimum_bytes: Option<i64>,
}

pub(in crate::persistence) fn decode_evidence(raw: RawEvidence) -> Result<Evidence, HistoryError> {
    let no_path = raw.path.is_none() && raw.path_encoding.is_none();
    let no_text = raw.text.is_none();
    let no_observed_time = raw.observed_seconds.is_none() && raw.observed_nanoseconds.is_none();
    let no_duration = raw.duration_seconds.is_none() && raw.duration_nanoseconds.is_none();
    let no_bytes = raw.observed_bytes.is_none() && raw.minimum_bytes.is_none();
    match raw.kind.as_str() {
        "matched_path" if no_text && no_observed_time && no_duration && no_bytes => {
            Ok(Evidence::MatchedPath {
                path: decode_required_evidence_path(raw.path, raw.path_encoding)?,
            })
        }
        "required_marker" if no_text && no_observed_time && no_duration && no_bytes => {
            Ok(Evidence::RequiredMarker {
                path: decode_required_evidence_path(raw.path, raw.path_encoding)?,
            })
        }
        "forbidden_marker_absent" if no_text && no_observed_time && no_duration && no_bytes => {
            Ok(Evidence::ForbiddenMarkerAbsent {
                path: decode_required_evidence_path(raw.path, raw.path_encoding)?,
            })
        }
        "cloud_upload_complete" if no_text && no_observed_time && no_duration && no_bytes => {
            Ok(Evidence::CloudUploadComplete {
                path: decode_required_evidence_path(raw.path, raw.path_encoding)?,
            })
        }
        "bundle_identifier" if no_observed_time && no_duration && no_bytes => {
            let identifier = raw.text.ok_or_else(corrupt)?;
            validate_text(&identifier, HistoryErrorKind::CorruptData)?;
            Ok(Evidence::BundleIdentifier {
                path: decode_required_evidence_path(raw.path, raw.path_encoding)?,
                identifier,
            })
        }
        "minimum_age" if no_path && no_text && no_bytes => Ok(Evidence::MinimumAge {
            newest_mtime: decode_required_time(raw.observed_seconds, raw.observed_nanoseconds)?,
            minimum_age: decode_required_duration(raw.duration_seconds, raw.duration_nanoseconds)?,
        }),
        "minimum_size" if no_path && no_text && no_observed_time && no_duration => {
            Ok(Evidence::MinimumSize {
                observed_bytes: from_i64(raw.observed_bytes.ok_or_else(corrupt)?)?,
                minimum_bytes: from_i64(raw.minimum_bytes.ok_or_else(corrupt)?)?,
            })
        }
        "inactive_process" if no_path && no_observed_time && no_duration && no_bytes => {
            let identifier = raw.text.ok_or_else(corrupt)?;
            validate_text(&identifier, HistoryErrorKind::CorruptData)?;
            Ok(Evidence::InactiveProcess { identifier })
        }
        _ => Err(corrupt()),
    }
}

fn load_blockers(
    connection: &Connection,
    id: &CandidateId,
) -> Result<Vec<BlockReason>, HistoryError> {
    let mut statement = connection
        .prepare(
            "SELECT blocker_ordinal, typeof(blocker_kind),
                    length(CAST(blocker_kind AS BLOB)), blocker_kind
             FROM candidate_blockers WHERE candidate_id = ?1
             ORDER BY blocker_ordinal LIMIT 65",
        )
        .map_err(map_query_sql_error)?;
    let mut rows = statement
        .query([id.as_str()])
        .map_err(map_query_sql_error)?;
    let mut blockers = Vec::new();
    while let Some(row) = rows.next().map_err(map_query_sql_error)? {
        if blockers.len() >= MAX_BLOCKERS {
            return Err(corrupt());
        }
        let ordinal: i64 = row.get(0).map_err(map_query_sql_error)?;
        if ordinal != blockers.len() as i64 {
            return Err(corrupt());
        }
        validate_required_value(row, 1, 2, "text", MAX_STORED_POLICY_BYTES)
            .map_err(map_query_sql_error)?;
        let value: String = row.get(3).map_err(map_query_sql_error)?;
        blockers.push(blocker_from_stored(&value)?);
    }
    Ok(blockers)
}

fn ensure_no_candidate_children(
    connection: &Connection,
    id: &CandidateId,
) -> Result<(), HistoryError> {
    let child_exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM candidate_paths WHERE candidate_id = ?1
             UNION ALL
             SELECT 1 FROM candidate_evidence WHERE candidate_id = ?1
             UNION ALL
             SELECT 1 FROM candidate_blockers WHERE candidate_id = ?1
             LIMIT 1",
            [id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    if child_exists.is_some() {
        return Err(corrupt());
    }
    Ok(())
}

fn ensure_source_scan_exists(connection: &Connection, id: &ScanId) -> Result<(), HistoryError> {
    let exists: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM scans WHERE scan_id = ?1",
            [id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_query_sql_error)?;
    if exists.is_none() {
        return Err(corrupt());
    }
    Ok(())
}

fn ensure_source_scan_succeeded(connection: &Connection, id: &ScanId) -> Result<(), HistoryError> {
    let Some(scan) = load_scan_record_within_budget(connection, id)? else {
        return Err(corrupt());
    };
    if scan.status() != ScanStatus::Succeeded {
        return Err(corrupt());
    }
    Ok(())
}

pub(super) fn validate_candidate(
    candidate: &Candidate,
    kind: HistoryErrorKind,
) -> Result<(), HistoryError> {
    if candidate.paths().is_empty()
        || candidate.paths().len() > MAX_PATHS
        || candidate.evidence().is_empty()
        || candidate.evidence().len() > MAX_EVIDENCE
        || candidate.blockers().len() > MAX_BLOCKERS
    {
        return Err(HistoryError::new(kind));
    }
    let mut paths = HashSet::with_capacity(candidate.paths().len());
    for path in candidate.paths() {
        if !path.is_absolute() || !paths.insert(path) || encode_host_path(path).is_err() {
            return Err(HistoryError::new(kind));
        }
    }
    to_i64(candidate.estimated_bytes(), kind)?;
    if let Some(value) = candidate.newest_mtime() {
        time_parts(value, kind)?;
    }
    for evidence in candidate.evidence() {
        PreparedEvidence::prepare(evidence)?;
    }
    validate_policy(
        candidate.safety(),
        candidate.action(),
        candidate.rule_marks_schedule_eligible(),
        || HistoryError::new(kind),
    )?;
    validate_complete_children(
        candidate.safety(),
        candidate.action(),
        candidate.paths(),
        candidate.evidence(),
    )
    .map_err(|_| HistoryError::new(kind))
}

pub(in crate::persistence) fn validate_complete_children(
    safety: SafetyTier,
    action: CandidateAction,
    paths: &[PathBuf],
    evidence: &[Evidence],
) -> Result<(), HistoryError> {
    if paths.is_empty() || evidence.is_empty() {
        return Err(corrupt());
    }
    if safety == SafetyTier::SafeEvictable || action == CandidateAction::EvictLocalCopy {
        let confirmed = evidence
            .iter()
            .filter_map(|fact| match fact {
                Evidence::CloudUploadComplete { path } => Some(path),
                _ => None,
            })
            .collect::<Vec<_>>();
        let unique = confirmed.iter().copied().collect::<HashSet<_>>();
        let expected = paths.iter().collect::<HashSet<_>>();
        if confirmed.len() != paths.len() || unique != expected {
            return Err(corrupt());
        }
    }
    Ok(())
}

pub(in crate::persistence) fn validate_policy(
    safety: SafetyTier,
    action: CandidateAction,
    schedule_eligible: bool,
    error: impl Fn() -> HistoryError,
) -> Result<(), HistoryError> {
    if !action.is_compatible_with(safety)
        || (schedule_eligible && !safety.is_schedule_policy_pair(action))
    {
        return Err(error());
    }
    Ok(())
}

pub(in crate::persistence) fn validate_required_value(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
    expected_type: &str,
    maximum_length: i64,
) -> rusqlite::Result<()> {
    let storage_type: String = row.get(type_column)?;
    let length: i64 = row.get(length_column)?;
    if storage_type != expected_type || !(1..=maximum_length).contains(&length) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

pub(in crate::persistence) fn validate_optional_value(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
    expected_type: &str,
    maximum_length: i64,
) -> rusqlite::Result<()> {
    let storage_type: String = row.get(type_column)?;
    let length: Option<i64> = row.get(length_column)?;
    if storage_type == "null" && length.is_none() {
        return Ok(());
    }
    if storage_type != expected_type
        || !length.is_some_and(|value| (1..=maximum_length).contains(&value))
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

pub(in crate::persistence) fn validate_null_value(
    row: &Row<'_>,
    type_column: usize,
    length_column: usize,
) -> rusqlite::Result<()> {
    let storage_type: String = row.get(type_column)?;
    let length: Option<i64> = row.get(length_column)?;
    if storage_type != "null" || length.is_some() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

pub(super) fn prepare_absolute_path(path: &Path) -> Result<EncodedBytes, HistoryError> {
    if !path.is_absolute() {
        return Err(HistoryError::new(HistoryErrorKind::InvalidInput));
    }
    encode_host_path(path).map_err(|_| HistoryError::new(HistoryErrorKind::InvalidInput))
}

fn decode_required_evidence_path(
    bytes: Option<Vec<u8>>,
    encoding: Option<i64>,
) -> Result<PathBuf, HistoryError> {
    decode_absolute_path(bytes.ok_or_else(corrupt)?, encoding.ok_or_else(corrupt)?)
}

pub(in crate::persistence) fn decode_absolute_path(
    bytes: Vec<u8>,
    encoding: i64,
) -> Result<PathBuf, HistoryError> {
    let encoding = StoredEncoding::host_path_from_stored(encoding).map_err(|_| corrupt())?;
    let path = decode_host_path(&EncodedBytes { bytes, encoding }).map_err(|_| corrupt())?;
    if !path.is_absolute() {
        return Err(corrupt());
    }
    Ok(path)
}

pub(in crate::persistence) fn validate_text(
    value: &str,
    kind: HistoryErrorKind,
) -> Result<(), HistoryError> {
    if value.is_empty() || value.len() > MAX_TEXT_BYTES || value.chars().any(char::is_control) {
        return Err(HistoryError::new(kind));
    }
    Ok(())
}

pub(in crate::persistence) fn time_parts(
    value: SystemTime,
    kind: HistoryErrorKind,
) -> Result<TimeParts, HistoryError> {
    let duration = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HistoryError::new(kind))?;
    duration_parts(duration, kind)
}

pub(super) fn duration_parts(
    value: Duration,
    kind: HistoryErrorKind,
) -> Result<TimeParts, HistoryError> {
    Ok(TimeParts {
        seconds: i64::try_from(value.as_secs()).map_err(|_| HistoryError::new(kind))?,
        nanoseconds: i64::from(value.subsec_nanos()),
    })
}

fn decode_required_time(
    seconds: Option<i64>,
    nanoseconds: Option<i64>,
) -> Result<SystemTime, HistoryError> {
    decode_time(
        seconds.ok_or_else(corrupt)?,
        nanoseconds.ok_or_else(corrupt)?,
    )
}

pub(in crate::persistence) fn decode_optional_time(
    seconds: Option<i64>,
    nanoseconds: Option<i64>,
) -> Result<Option<SystemTime>, HistoryError> {
    match (seconds, nanoseconds) {
        (None, None) => Ok(None),
        (Some(seconds), Some(nanoseconds)) => decode_time(seconds, nanoseconds).map(Some),
        _ => Err(corrupt()),
    }
}

fn decode_time(seconds: i64, nanoseconds: i64) -> Result<SystemTime, HistoryError> {
    let seconds = u64::try_from(seconds).map_err(|_| corrupt())?;
    let nanoseconds = u32::try_from(nanoseconds).map_err(|_| corrupt())?;
    if nanoseconds >= 1_000_000_000 {
        return Err(corrupt());
    }
    UNIX_EPOCH
        .checked_add(Duration::new(seconds, nanoseconds))
        .ok_or_else(corrupt)
}

fn decode_required_duration(
    seconds: Option<i64>,
    nanoseconds: Option<i64>,
) -> Result<Duration, HistoryError> {
    let seconds = u64::try_from(seconds.ok_or_else(corrupt)?).map_err(|_| corrupt())?;
    let nanoseconds = u32::try_from(nanoseconds.ok_or_else(corrupt)?).map_err(|_| corrupt())?;
    if nanoseconds >= 1_000_000_000 {
        return Err(corrupt());
    }
    Ok(Duration::new(seconds, nanoseconds))
}

pub(in crate::persistence) fn category_as_stored(value: CandidateCategory) -> &'static str {
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

pub(in crate::persistence) fn category_from_stored(
    value: &str,
) -> Result<CandidateCategory, HistoryError> {
    match value {
        "developer_artifact" => Ok(CandidateCategory::DeveloperArtifact),
        "application_cache" => Ok(CandidateCategory::ApplicationCache),
        "browser_cache" => Ok(CandidateCategory::BrowserCache),
        "log_and_diagnostic" => Ok(CandidateCategory::LogAndDiagnostic),
        "installer_and_download" => Ok(CandidateCategory::InstallerAndDownload),
        "device_and_simulator_data" => Ok(CandidateCategory::DeviceAndSimulatorData),
        "cloud_file" => Ok(CandidateCategory::CloudFile),
        "large_review_item" => Ok(CandidateCategory::LargeReviewItem),
        "protected_system_data" => Ok(CandidateCategory::ProtectedSystemData),
        "unknown_storage" => Ok(CandidateCategory::UnknownStorage),
        _ => Err(corrupt()),
    }
}

pub(in crate::persistence) fn safety_as_stored(value: SafetyTier) -> &'static str {
    match value {
        SafetyTier::SafeRegenerable => "safe_regenerable",
        SafetyTier::SafeEvictable => "safe_evictable",
        SafetyTier::ReviewRequired => "review_required",
        SafetyTier::Informational => "informational",
        SafetyTier::Protected => "protected",
    }
}

pub(in crate::persistence) fn safety_from_stored(value: &str) -> Result<SafetyTier, HistoryError> {
    match value {
        "safe_regenerable" => Ok(SafetyTier::SafeRegenerable),
        "safe_evictable" => Ok(SafetyTier::SafeEvictable),
        "review_required" => Ok(SafetyTier::ReviewRequired),
        "informational" => Ok(SafetyTier::Informational),
        "protected" => Ok(SafetyTier::Protected),
        _ => Err(corrupt()),
    }
}

pub(in crate::persistence) fn action_as_stored(value: CandidateAction) -> &'static str {
    match value {
        CandidateAction::RemoveKnownRegenerableContents => "remove_known_regenerable_contents",
        CandidateAction::EvictLocalCopy => "evict_local_copy",
        CandidateAction::MoveToTrash => "move_to_trash",
        CandidateAction::RevealOnly => "reveal_only",
        CandidateAction::NoAction => "no_action",
    }
}

pub(in crate::persistence) fn action_from_stored(
    value: &str,
) -> Result<CandidateAction, HistoryError> {
    match value {
        "remove_known_regenerable_contents" => Ok(CandidateAction::RemoveKnownRegenerableContents),
        "evict_local_copy" => Ok(CandidateAction::EvictLocalCopy),
        "move_to_trash" => Ok(CandidateAction::MoveToTrash),
        "reveal_only" => Ok(CandidateAction::RevealOnly),
        "no_action" => Ok(CandidateAction::NoAction),
        _ => Err(corrupt()),
    }
}

pub(super) fn evidence_kind_as_stored(value: &Evidence) -> &'static str {
    match value {
        Evidence::MatchedPath { .. } => "matched_path",
        Evidence::RequiredMarker { .. } => "required_marker",
        Evidence::ForbiddenMarkerAbsent { .. } => "forbidden_marker_absent",
        Evidence::BundleIdentifier { .. } => "bundle_identifier",
        Evidence::MinimumAge { .. } => "minimum_age",
        Evidence::MinimumSize { .. } => "minimum_size",
        Evidence::InactiveProcess { .. } => "inactive_process",
        Evidence::CloudUploadComplete { .. } => "cloud_upload_complete",
    }
}

pub(super) fn blocker_as_stored(value: &BlockReason) -> &'static str {
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

pub(super) fn blocker_from_stored(value: &str) -> Result<BlockReason, HistoryError> {
    match value {
        "missing_or_incomplete_evidence" => Ok(BlockReason::MissingOrIncompleteEvidence),
        "missing_modification_time" => Ok(BlockReason::MissingModificationTime),
        "partial_scan_coverage" => Ok(BlockReason::PartialScanCoverage),
        "recent_activity" => Ok(BlockReason::RecentActivity),
        "below_minimum_bytes" => Ok(BlockReason::BelowMinimumBytes),
        "active_use" => Ok(BlockReason::ActiveUse),
        "access_denied" => Ok(BlockReason::AccessDenied),
        "protected_path" => Ok(BlockReason::ProtectedPath),
        "protected_descendant" => Ok(BlockReason::ProtectedDescendant),
        "symlink_boundary" => Ok(BlockReason::SymlinkBoundary),
        "volume_boundary" => Ok(BlockReason::VolumeBoundary),
        "changed_since_scan" => Ok(BlockReason::ChangedSinceScan),
        "unsupported_platform" => Ok(BlockReason::UnsupportedPlatform),
        "cloud_upload_unconfirmed" => Ok(BlockReason::CloudUploadUnconfirmed),
        _ => Err(corrupt()),
    }
}

pub(super) fn corrupt() -> HistoryError {
    HistoryError::new(HistoryErrorKind::CorruptData)
}
