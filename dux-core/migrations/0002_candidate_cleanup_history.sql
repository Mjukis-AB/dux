DROP INDEX candidates_by_scan_status;

ALTER TABLE candidates RENAME TO candidates_v1;

CREATE TABLE candidates (
    candidate_id TEXT PRIMARY KEY CHECK (length(candidate_id) BETWEEN 1 AND 128),
    scan_id TEXT NOT NULL REFERENCES scans(scan_id) ON DELETE CASCADE,
    rule_id TEXT NOT NULL CHECK (length(rule_id) BETWEEN 1 AND 128),
    rule_revision INTEGER NOT NULL CHECK (rule_revision > 0),
    safety_tier TEXT NOT NULL CHECK (
        safety_tier IN (
            'safe_regenerable', 'safe_evictable', 'review_required', 'informational', 'protected'
        )
    ),
    estimated_bytes INTEGER NOT NULL CHECK (estimated_bytes >= 0),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    status TEXT NOT NULL CHECK (
        status IN (
            'discovered', 'selected', 'dismissed', 'stale',
            'planned', 'completed', 'failed', 'unavailable'
        )
    ),
    record_format_version INTEGER NOT NULL CHECK (record_format_version IN (1, 2)),
    category TEXT CHECK (
        category IS NULL OR category IN (
            'developer_artifact', 'application_cache', 'browser_cache',
            'log_and_diagnostic', 'installer_and_download', 'device_and_simulator_data',
            'cloud_file', 'large_review_item', 'protected_system_data', 'unknown_storage'
        )
    ),
    proposed_action TEXT CHECK (
        proposed_action IS NULL OR proposed_action IN (
            'remove_known_regenerable_contents', 'evict_local_copy', 'move_to_trash',
            'reveal_only', 'no_action'
        )
    ),
    rule_schedule_eligible INTEGER CHECK (
        rule_schedule_eligible IS NULL OR rule_schedule_eligible IN (0, 1)
    ),
    newest_mtime_unix_seconds INTEGER CHECK (
        newest_mtime_unix_seconds IS NULL OR newest_mtime_unix_seconds >= 0
    ),
    newest_mtime_nanoseconds INTEGER CHECK (
        newest_mtime_nanoseconds IS NULL OR newest_mtime_nanoseconds BETWEEN 0 AND 999999999
    ),
    CHECK (
        (record_format_version = 1 AND category IS NULL AND proposed_action IS NULL AND
            rule_schedule_eligible IS NULL AND newest_mtime_unix_seconds IS NULL AND
            newest_mtime_nanoseconds IS NULL) OR
        (record_format_version = 2 AND category IS NOT NULL AND proposed_action IS NOT NULL AND
            rule_schedule_eligible IS NOT NULL AND
            ((newest_mtime_unix_seconds IS NULL AND newest_mtime_nanoseconds IS NULL) OR
             (newest_mtime_unix_seconds IS NOT NULL AND newest_mtime_nanoseconds IS NOT NULL)))
    ),
    CHECK (
        record_format_version = 1 OR
        (safety_tier = 'safe_regenerable' AND
            proposed_action = 'remove_known_regenerable_contents') OR
        (safety_tier = 'safe_evictable' AND proposed_action = 'evict_local_copy') OR
        (safety_tier = 'review_required' AND proposed_action = 'move_to_trash') OR
        (safety_tier IN ('informational', 'protected') AND
            proposed_action IN ('reveal_only', 'no_action'))
    ),
    CHECK (
        rule_schedule_eligible IS NULL OR rule_schedule_eligible = 0 OR
        (safety_tier = 'safe_regenerable' AND
            proposed_action = 'remove_known_regenerable_contents')
    )
) STRICT;

INSERT INTO candidates (
    candidate_id, scan_id, rule_id, rule_revision, safety_tier, estimated_bytes,
    created_at_unix_ms, status, record_format_version
)
SELECT candidate_id, scan_id, rule_id, rule_revision, safety_tier, estimated_bytes,
       created_at_unix_ms, status, 1
FROM candidates_v1;

DROP TABLE candidates_v1;

CREATE INDEX candidates_by_scan_status ON candidates(scan_id, status, estimated_bytes DESC);
CREATE INDEX candidates_by_scan_time
    ON candidates(scan_id, created_at_unix_ms DESC, candidate_id);

CREATE TABLE candidate_paths (
    candidate_id TEXT NOT NULL REFERENCES candidates(candidate_id) ON DELETE CASCADE,
    path_ordinal INTEGER NOT NULL CHECK (path_ordinal >= 0),
    observed_path BLOB NOT NULL,
    observed_path_encoding INTEGER NOT NULL,
    PRIMARY KEY (candidate_id, path_ordinal),
    UNIQUE (candidate_id, observed_path_encoding, observed_path),
    CHECK (
        (observed_path_encoding = 1 AND length(observed_path) BETWEEN 1 AND 32768) OR
        (observed_path_encoding = 2 AND length(observed_path) BETWEEN 2 AND 65536 AND
            length(observed_path) % 2 = 0)
    )
) STRICT;

CREATE TABLE candidate_evidence (
    candidate_id TEXT NOT NULL REFERENCES candidates(candidate_id) ON DELETE CASCADE,
    evidence_ordinal INTEGER NOT NULL CHECK (evidence_ordinal >= 0),
    evidence_kind TEXT NOT NULL CHECK (
        evidence_kind IN (
            'matched_path', 'required_marker', 'forbidden_marker_absent',
            'bundle_identifier', 'minimum_age', 'minimum_size',
            'inactive_process', 'cloud_upload_complete'
        )
    ),
    path_value BLOB,
    path_value_encoding INTEGER,
    text_value TEXT CHECK (
        text_value IS NULL OR length(CAST(text_value AS BLOB)) BETWEEN 1 AND 4096
    ),
    observed_unix_seconds INTEGER CHECK (
        observed_unix_seconds IS NULL OR observed_unix_seconds >= 0
    ),
    observed_nanoseconds INTEGER CHECK (
        observed_nanoseconds IS NULL OR observed_nanoseconds BETWEEN 0 AND 999999999
    ),
    duration_seconds INTEGER CHECK (duration_seconds IS NULL OR duration_seconds >= 0),
    duration_nanoseconds INTEGER CHECK (
        duration_nanoseconds IS NULL OR duration_nanoseconds BETWEEN 0 AND 999999999
    ),
    observed_bytes INTEGER CHECK (observed_bytes IS NULL OR observed_bytes >= 0),
    minimum_bytes INTEGER CHECK (minimum_bytes IS NULL OR minimum_bytes >= 0),
    PRIMARY KEY (candidate_id, evidence_ordinal),
    CHECK (
        (path_value IS NULL AND path_value_encoding IS NULL) OR
        (path_value IS NOT NULL AND path_value_encoding IS NOT NULL AND
            ((path_value_encoding = 1 AND length(path_value) BETWEEN 1 AND 32768) OR
             (path_value_encoding = 2 AND length(path_value) BETWEEN 2 AND 65536 AND
                length(path_value) % 2 = 0)))
    ),
    CHECK (
        (evidence_kind IN (
                'matched_path', 'required_marker', 'forbidden_marker_absent',
                'cloud_upload_complete'
            ) AND path_value IS NOT NULL AND text_value IS NULL AND
            observed_unix_seconds IS NULL AND observed_nanoseconds IS NULL AND
            duration_seconds IS NULL AND duration_nanoseconds IS NULL AND
            observed_bytes IS NULL AND minimum_bytes IS NULL) OR
        (evidence_kind = 'bundle_identifier' AND path_value IS NOT NULL AND
            text_value IS NOT NULL AND observed_unix_seconds IS NULL AND
            observed_nanoseconds IS NULL AND duration_seconds IS NULL AND
            duration_nanoseconds IS NULL AND observed_bytes IS NULL AND minimum_bytes IS NULL) OR
        (evidence_kind = 'minimum_age' AND path_value IS NULL AND text_value IS NULL AND
            observed_unix_seconds IS NOT NULL AND observed_nanoseconds IS NOT NULL AND
            duration_seconds IS NOT NULL AND duration_nanoseconds IS NOT NULL AND
            observed_bytes IS NULL AND minimum_bytes IS NULL) OR
        (evidence_kind = 'minimum_size' AND path_value IS NULL AND text_value IS NULL AND
            observed_unix_seconds IS NULL AND observed_nanoseconds IS NULL AND
            duration_seconds IS NULL AND duration_nanoseconds IS NULL AND
            observed_bytes IS NOT NULL AND minimum_bytes IS NOT NULL) OR
        (evidence_kind = 'inactive_process' AND path_value IS NULL AND text_value IS NOT NULL AND
            observed_unix_seconds IS NULL AND observed_nanoseconds IS NULL AND
            duration_seconds IS NULL AND duration_nanoseconds IS NULL AND
            observed_bytes IS NULL AND minimum_bytes IS NULL)
    )
) STRICT;

CREATE TABLE candidate_blockers (
    candidate_id TEXT NOT NULL REFERENCES candidates(candidate_id) ON DELETE CASCADE,
    blocker_ordinal INTEGER NOT NULL CHECK (blocker_ordinal >= 0),
    blocker_kind TEXT NOT NULL CHECK (
        blocker_kind IN (
            'missing_or_incomplete_evidence', 'missing_modification_time',
            'partial_scan_coverage', 'recent_activity', 'below_minimum_bytes',
            'active_use', 'access_denied', 'protected_path', 'protected_descendant',
            'symlink_boundary', 'volume_boundary', 'changed_since_scan',
            'unsupported_platform', 'cloud_upload_unconfirmed'
        )
    ),
    PRIMARY KEY (candidate_id, blocker_ordinal)
) STRICT;

DROP INDEX cleanup_items_by_session;
DROP INDEX cleanup_sessions_by_time;

ALTER TABLE cleanup_items RENAME TO cleanup_items_v1;
ALTER TABLE cleanup_sessions RENAME TO cleanup_sessions_v1;

CREATE TABLE cleanup_sessions (
    session_id TEXT PRIMARY KEY CHECK (length(session_id) BETWEEN 1 AND 128),
    plan_id TEXT NOT NULL UNIQUE CHECK (length(plan_id) BETWEEN 1 AND 128),
    started_at_unix_ms INTEGER NOT NULL CHECK (started_at_unix_ms >= 0),
    completed_at_unix_ms INTEGER CHECK (
        completed_at_unix_ms IS NULL OR completed_at_unix_ms >= started_at_unix_ms
    ),
    mode TEXT NOT NULL CHECK (mode IN ('dry_run', 'trash', 'permanent_safe', 'evict_local_copy')),
    estimated_bytes INTEGER NOT NULL CHECK (estimated_bytes >= 0),
    verified_capacity_delta_bytes INTEGER,
    trigger_source TEXT NOT NULL CHECK (trigger_source IN ('manual', 'low_disk', 'scheduled', 'cli')),
    status TEXT NOT NULL CHECK (
        status IN (
            'planned', 'running', 'recovering', 'completed', 'partially_completed', 'failed',
            'cancelled', 'interrupted', 'rejected', 'dry_run'
        )
    ),
    record_format_version INTEGER NOT NULL CHECK (record_format_version IN (1, 2)),
    source_scan_id TEXT REFERENCES scans(scan_id) ON DELETE RESTRICT,
    plan_created_at_unix_seconds INTEGER CHECK (
        plan_created_at_unix_seconds IS NULL OR plan_created_at_unix_seconds >= 0
    ),
    plan_created_at_nanoseconds INTEGER CHECK (
        plan_created_at_nanoseconds IS NULL OR plan_created_at_nanoseconds BETWEEN 0 AND 999999999
    ),
    plan_expires_at_unix_seconds INTEGER CHECK (
        plan_expires_at_unix_seconds IS NULL OR plan_expires_at_unix_seconds >= 0
    ),
    plan_expires_at_nanoseconds INTEGER CHECK (
        plan_expires_at_nanoseconds IS NULL OR plan_expires_at_nanoseconds BETWEEN 0 AND 999999999
    ),
    execution_owner_id TEXT CHECK (
        execution_owner_id IS NULL OR length(CAST(execution_owner_id AS BLOB)) BETWEEN 1 AND 128
    ),
    execution_generation INTEGER CHECK (
        execution_generation IS NULL OR execution_generation > 0
    ),
    last_heartbeat_at_unix_ms INTEGER CHECK (
        last_heartbeat_at_unix_ms IS NULL OR last_heartbeat_at_unix_ms >= started_at_unix_ms
    ),
    cancellation_requested INTEGER CHECK (
        cancellation_requested IS NULL OR cancellation_requested IN (0, 1)
    ),
    CHECK (
        (record_format_version = 1 AND source_scan_id IS NULL AND
            plan_created_at_unix_seconds IS NULL AND plan_created_at_nanoseconds IS NULL AND
            plan_expires_at_unix_seconds IS NULL AND plan_expires_at_nanoseconds IS NULL AND
            execution_owner_id IS NULL AND execution_generation IS NULL AND
            last_heartbeat_at_unix_ms IS NULL AND cancellation_requested IS NULL) OR
        (record_format_version = 2 AND source_scan_id IS NOT NULL AND
            plan_created_at_unix_seconds IS NOT NULL AND plan_created_at_nanoseconds IS NOT NULL AND
            plan_expires_at_unix_seconds IS NOT NULL AND plan_expires_at_nanoseconds IS NOT NULL AND
            cancellation_requested IS NOT NULL AND
            ((execution_owner_id IS NULL AND execution_generation IS NULL AND
                last_heartbeat_at_unix_ms IS NULL) OR
             (execution_owner_id IS NOT NULL AND execution_generation IS NOT NULL AND
                last_heartbeat_at_unix_ms IS NOT NULL)))
    ),
    CHECK (
        plan_created_at_unix_seconds IS NULL OR
        plan_expires_at_unix_seconds > plan_created_at_unix_seconds OR
        (plan_expires_at_unix_seconds = plan_created_at_unix_seconds AND
            plan_expires_at_nanoseconds >= plan_created_at_nanoseconds)
    ),
    CHECK (
        record_format_version = 1 OR status NOT IN ('running', 'recovering') OR
        execution_owner_id IS NOT NULL
    )
) STRICT;

INSERT INTO cleanup_sessions (
    session_id, plan_id, started_at_unix_ms, completed_at_unix_ms, mode,
    estimated_bytes, verified_capacity_delta_bytes, trigger_source, status,
    record_format_version
)
SELECT session_id, plan_id, started_at_unix_ms, completed_at_unix_ms, mode,
       estimated_bytes, verified_capacity_delta_bytes, trigger_source, status, 1
FROM cleanup_sessions_v1;

CREATE TABLE cleanup_items (
    item_id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES cleanup_sessions(session_id) ON DELETE RESTRICT,
    item_ordinal INTEGER NOT NULL CHECK (item_ordinal >= 0),
    rule_id TEXT NOT NULL CHECK (length(rule_id) BETWEEN 1 AND 128),
    rule_revision INTEGER NOT NULL CHECK (rule_revision > 0),
    estimated_bytes INTEGER NOT NULL CHECK (estimated_bytes >= 0),
    final_status TEXT NOT NULL CHECK (
        final_status IN (
            'planned', 'validating', 'dry_run', 'effect_started', 'trashed', 'removed',
            'evicted', 'skipped', 'rejected', 'failed', 'changed_since_plan',
            'interrupted', 'unavailable', 'outcome_unknown'
        )
    ),
    error_category TEXT,
    record_format_version INTEGER NOT NULL CHECK (record_format_version IN (1, 2)),
    legacy_target_path BLOB,
    legacy_target_path_encoding INTEGER,
    candidate_id TEXT CHECK (
        candidate_id IS NULL OR length(candidate_id) BETWEEN 1 AND 128
    ),
    category TEXT CHECK (
        category IS NULL OR category IN (
            'developer_artifact', 'application_cache', 'browser_cache',
            'log_and_diagnostic', 'installer_and_download', 'device_and_simulator_data',
            'cloud_file', 'large_review_item', 'protected_system_data', 'unknown_storage'
        )
    ),
    safety_tier TEXT CHECK (
        safety_tier IS NULL OR safety_tier IN (
            'safe_regenerable', 'safe_evictable', 'review_required', 'informational', 'protected'
        )
    ),
    proposed_action TEXT CHECK (
        proposed_action IS NULL OR proposed_action IN (
            'remove_known_regenerable_contents', 'evict_local_copy', 'move_to_trash'
        )
    ),
    rule_schedule_eligible INTEGER CHECK (
        rule_schedule_eligible IS NULL OR rule_schedule_eligible IN (0, 1)
    ),
    newest_mtime_unix_seconds INTEGER CHECK (
        newest_mtime_unix_seconds IS NULL OR newest_mtime_unix_seconds >= 0
    ),
    newest_mtime_nanoseconds INTEGER CHECK (
        newest_mtime_nanoseconds IS NULL OR newest_mtime_nanoseconds BETWEEN 0 AND 999999999
    ),
    UNIQUE (session_id, item_ordinal),
    CHECK (
        (legacy_target_path IS NULL AND legacy_target_path_encoding IS NULL) OR
        (legacy_target_path IS NOT NULL AND legacy_target_path_encoding IS NOT NULL AND
            ((legacy_target_path_encoding = 1 AND length(legacy_target_path) BETWEEN 1 AND 32768) OR
             (legacy_target_path_encoding = 2 AND length(legacy_target_path) BETWEEN 2 AND 65536 AND
                length(legacy_target_path) % 2 = 0)))
    ),
    CHECK (
        (record_format_version = 1 AND legacy_target_path IS NOT NULL AND
            candidate_id IS NULL AND category IS NULL AND safety_tier IS NULL AND
            proposed_action IS NULL AND rule_schedule_eligible IS NULL AND
            newest_mtime_unix_seconds IS NULL AND newest_mtime_nanoseconds IS NULL) OR
        (record_format_version = 2 AND legacy_target_path IS NULL AND
            candidate_id IS NOT NULL AND category IS NOT NULL AND safety_tier IS NOT NULL AND
            proposed_action IS NOT NULL AND rule_schedule_eligible IS NOT NULL AND
            ((newest_mtime_unix_seconds IS NULL AND newest_mtime_nanoseconds IS NULL) OR
             (newest_mtime_unix_seconds IS NOT NULL AND newest_mtime_nanoseconds IS NOT NULL)))
    ),
    CHECK (
        error_category IS NULL OR
        (record_format_version = 1 AND length(error_category) BETWEEN 1 AND 128) OR
        (record_format_version = 2 AND
            length(CAST(error_category AS BLOB)) BETWEEN 1 AND 128)
    ),
    CHECK (
        record_format_version = 1 OR
        (safety_tier = 'safe_regenerable' AND
            proposed_action = 'remove_known_regenerable_contents') OR
        (safety_tier = 'safe_evictable' AND proposed_action = 'evict_local_copy') OR
        (safety_tier = 'review_required' AND proposed_action = 'move_to_trash')
    ),
    CHECK (
        rule_schedule_eligible IS NULL OR rule_schedule_eligible = 0 OR
        (safety_tier = 'safe_regenerable' AND
            proposed_action = 'remove_known_regenerable_contents')
    )
) STRICT;

INSERT INTO cleanup_items (
    item_id, session_id, item_ordinal, rule_id, rule_revision, estimated_bytes,
    final_status, error_category, record_format_version,
    legacy_target_path, legacy_target_path_encoding
)
SELECT item_id, session_id, item_ordinal, rule_id, rule_revision, estimated_bytes,
       final_status, error_category, 1, target_path, target_path_encoding
FROM cleanup_items_v1;

DROP TABLE cleanup_items_v1;
DROP TABLE cleanup_sessions_v1;

CREATE TABLE cleanup_item_paths (
    session_id TEXT NOT NULL,
    item_ordinal INTEGER NOT NULL,
    path_ordinal INTEGER NOT NULL CHECK (path_ordinal >= 0),
    target_path BLOB NOT NULL,
    target_path_encoding INTEGER NOT NULL,
    attempt_generation INTEGER CHECK (attempt_generation IS NULL OR attempt_generation > 0),
    status TEXT NOT NULL CHECK (
        status IN (
            'planned', 'validating', 'dry_run', 'effect_started', 'trashed', 'removed',
            'evicted', 'skipped', 'rejected', 'failed', 'changed_since_plan',
            'interrupted', 'unavailable', 'outcome_unknown'
        )
    ),
    error_category TEXT CHECK (
        error_category IS NULL OR length(CAST(error_category AS BLOB)) BETWEEN 1 AND 128
    ),
    effect_started_at_unix_ms INTEGER CHECK (
        effect_started_at_unix_ms IS NULL OR effect_started_at_unix_ms >= 0
    ),
    completed_at_unix_ms INTEGER CHECK (
        completed_at_unix_ms IS NULL OR completed_at_unix_ms >= 0
    ),
    PRIMARY KEY (session_id, item_ordinal, path_ordinal),
    UNIQUE (session_id, target_path_encoding, target_path),
    FOREIGN KEY (session_id, item_ordinal)
        REFERENCES cleanup_items(session_id, item_ordinal) ON DELETE RESTRICT,
    CHECK (
        (target_path_encoding = 1 AND length(target_path) BETWEEN 1 AND 32768) OR
        (target_path_encoding = 2 AND length(target_path) BETWEEN 2 AND 65536 AND
            length(target_path) % 2 = 0)
    ),
    CHECK (
        effect_started_at_unix_ms IS NULL OR completed_at_unix_ms IS NULL OR
        completed_at_unix_ms >= effect_started_at_unix_ms
    ),
    CHECK (
        status NOT IN ('effect_started', 'trashed', 'removed', 'evicted', 'outcome_unknown') OR
        effect_started_at_unix_ms IS NOT NULL
    ),
    CHECK (
        status NOT IN ('trashed', 'removed', 'evicted') OR completed_at_unix_ms IS NOT NULL
    ),
    CHECK (status != 'effect_started' OR completed_at_unix_ms IS NULL),
    CHECK (status = 'planned' OR attempt_generation IS NOT NULL),
    CHECK (
        status IN ('planned', 'validating', 'effect_started') OR
        completed_at_unix_ms IS NOT NULL
    )
) STRICT;

CREATE TABLE cleanup_item_evidence (
    session_id TEXT NOT NULL,
    item_ordinal INTEGER NOT NULL,
    evidence_ordinal INTEGER NOT NULL CHECK (evidence_ordinal >= 0),
    evidence_kind TEXT NOT NULL CHECK (
        evidence_kind IN (
            'matched_path', 'required_marker', 'forbidden_marker_absent',
            'bundle_identifier', 'minimum_age', 'minimum_size',
            'inactive_process', 'cloud_upload_complete'
        )
    ),
    path_value BLOB,
    path_value_encoding INTEGER,
    text_value TEXT CHECK (
        text_value IS NULL OR length(CAST(text_value AS BLOB)) BETWEEN 1 AND 4096
    ),
    observed_unix_seconds INTEGER CHECK (
        observed_unix_seconds IS NULL OR observed_unix_seconds >= 0
    ),
    observed_nanoseconds INTEGER CHECK (
        observed_nanoseconds IS NULL OR observed_nanoseconds BETWEEN 0 AND 999999999
    ),
    duration_seconds INTEGER CHECK (duration_seconds IS NULL OR duration_seconds >= 0),
    duration_nanoseconds INTEGER CHECK (
        duration_nanoseconds IS NULL OR duration_nanoseconds BETWEEN 0 AND 999999999
    ),
    observed_bytes INTEGER CHECK (observed_bytes IS NULL OR observed_bytes >= 0),
    minimum_bytes INTEGER CHECK (minimum_bytes IS NULL OR minimum_bytes >= 0),
    PRIMARY KEY (session_id, item_ordinal, evidence_ordinal),
    FOREIGN KEY (session_id, item_ordinal)
        REFERENCES cleanup_items(session_id, item_ordinal) ON DELETE RESTRICT,
    CHECK (
        (path_value IS NULL AND path_value_encoding IS NULL) OR
        (path_value IS NOT NULL AND path_value_encoding IS NOT NULL AND
            ((path_value_encoding = 1 AND length(path_value) BETWEEN 1 AND 32768) OR
             (path_value_encoding = 2 AND length(path_value) BETWEEN 2 AND 65536 AND
                length(path_value) % 2 = 0)))
    ),
    CHECK (
        (evidence_kind IN (
                'matched_path', 'required_marker', 'forbidden_marker_absent',
                'cloud_upload_complete'
            ) AND path_value IS NOT NULL AND text_value IS NULL AND
            observed_unix_seconds IS NULL AND observed_nanoseconds IS NULL AND
            duration_seconds IS NULL AND duration_nanoseconds IS NULL AND
            observed_bytes IS NULL AND minimum_bytes IS NULL) OR
        (evidence_kind = 'bundle_identifier' AND path_value IS NOT NULL AND
            text_value IS NOT NULL AND observed_unix_seconds IS NULL AND
            observed_nanoseconds IS NULL AND duration_seconds IS NULL AND
            duration_nanoseconds IS NULL AND observed_bytes IS NULL AND minimum_bytes IS NULL) OR
        (evidence_kind = 'minimum_age' AND path_value IS NULL AND text_value IS NULL AND
            observed_unix_seconds IS NOT NULL AND observed_nanoseconds IS NOT NULL AND
            duration_seconds IS NOT NULL AND duration_nanoseconds IS NOT NULL AND
            observed_bytes IS NULL AND minimum_bytes IS NULL) OR
        (evidence_kind = 'minimum_size' AND path_value IS NULL AND text_value IS NULL AND
            observed_unix_seconds IS NULL AND observed_nanoseconds IS NULL AND
            duration_seconds IS NULL AND duration_nanoseconds IS NULL AND
            observed_bytes IS NOT NULL AND minimum_bytes IS NOT NULL) OR
        (evidence_kind = 'inactive_process' AND path_value IS NULL AND text_value IS NOT NULL AND
            observed_unix_seconds IS NULL AND observed_nanoseconds IS NULL AND
            duration_seconds IS NULL AND duration_nanoseconds IS NULL AND
            observed_bytes IS NULL AND minimum_bytes IS NULL)
    )
) STRICT;

CREATE TABLE cleanup_plan_warnings (
    session_id TEXT NOT NULL REFERENCES cleanup_sessions(session_id) ON DELETE RESTRICT,
    warning_ordinal INTEGER NOT NULL CHECK (warning_ordinal >= 0),
    warning_kind TEXT NOT NULL CHECK (
        warning_kind IN (
            'estimated_bytes_unverified', 'dry_run_does_not_mutate',
            'trash_does_not_free_space_immediately', 'permanent_removal_cannot_be_undone',
            'cloud_eviction_requires_network_to_redownload'
        )
    ),
    PRIMARY KEY (session_id, warning_ordinal),
    UNIQUE (session_id, warning_kind)
) STRICT;

CREATE INDEX cleanup_sessions_by_time
    ON cleanup_sessions(started_at_unix_ms DESC, session_id);
CREATE INDEX cleanup_sessions_by_recovery
    ON cleanup_sessions(status, last_heartbeat_at_unix_ms, session_id)
    WHERE status IN ('running', 'recovering');
CREATE INDEX cleanup_items_by_session ON cleanup_items(session_id, item_ordinal);
CREATE INDEX scans_by_started ON scans(started_at_unix_ms DESC, scan_id);
