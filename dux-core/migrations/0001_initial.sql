CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version > 0),
    name TEXT NOT NULL UNIQUE CHECK (length(name) BETWEEN 1 AND 128),
    checksum_sha256 BLOB NOT NULL CHECK (length(checksum_sha256) = 32),
    applied_at_unix_ms INTEGER NOT NULL CHECK (applied_at_unix_ms >= 0)
) STRICT;

CREATE TABLE volumes (
    volume_id TEXT PRIMARY KEY CHECK (length(volume_id) BETWEEN 1 AND 128),
    mount_path BLOB NOT NULL,
    mount_path_encoding INTEGER NOT NULL,
    display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 512),
    filesystem TEXT NOT NULL CHECK (length(filesystem) BETWEEN 1 AND 128),
    is_internal INTEGER NOT NULL CHECK (is_internal IN (0, 1)),
    is_removable INTEGER NOT NULL CHECK (is_removable IN (0, 1)),
    first_seen_unix_ms INTEGER NOT NULL CHECK (first_seen_unix_ms >= 0),
    last_seen_unix_ms INTEGER NOT NULL CHECK (last_seen_unix_ms >= first_seen_unix_ms),
    CHECK (
        (mount_path_encoding = 1 AND length(mount_path) BETWEEN 1 AND 32768) OR
        (
            mount_path_encoding = 2 AND
            length(mount_path) BETWEEN 2 AND 65536 AND
            length(mount_path) % 2 = 0
        )
    )
) STRICT;

CREATE TABLE disk_samples (
    sample_id INTEGER PRIMARY KEY,
    volume_id TEXT NOT NULL REFERENCES volumes(volume_id) ON DELETE RESTRICT,
    sample_kind TEXT NOT NULL CHECK (sample_kind IN ('raw', 'daily_rollup')),
    sampled_at_unix_ms INTEGER NOT NULL CHECK (sampled_at_unix_ms >= 0),
    total_bytes INTEGER NOT NULL CHECK (total_bytes >= 0),
    available_bytes INTEGER NOT NULL CHECK (available_bytes >= 0 AND available_bytes <= total_bytes),
    important_available_bytes INTEGER CHECK (
        important_available_bytes IS NULL OR
        (important_available_bytes >= 0 AND important_available_bytes <= total_bytes)
    ),
    pressure TEXT NOT NULL CHECK (pressure IN ('healthy', 'warning', 'critical', 'unknown')),
    UNIQUE (volume_id, sample_kind, sampled_at_unix_ms)
) STRICT;

CREATE TABLE scans (
    scan_id TEXT PRIMARY KEY CHECK (length(scan_id) BETWEEN 1 AND 128),
    volume_id TEXT REFERENCES volumes(volume_id) ON DELETE RESTRICT,
    root_path BLOB NOT NULL,
    root_path_encoding INTEGER NOT NULL,
    started_at_unix_ms INTEGER NOT NULL CHECK (started_at_unix_ms >= 0),
    completed_at_unix_ms INTEGER CHECK (
        completed_at_unix_ms IS NULL OR completed_at_unix_ms >= started_at_unix_ms
    ),
    status TEXT NOT NULL CHECK (
        status IN ('queued', 'running', 'succeeded', 'failed', 'cancelled', 'interrupted')
    ),
    snapshot_version INTEGER,
    snapshot_relative_path BLOB,
    snapshot_relative_path_encoding INTEGER,
    snapshot_checksum_sha256 BLOB,
    directory_count INTEGER NOT NULL DEFAULT 0 CHECK (directory_count >= 0),
    file_count INTEGER NOT NULL DEFAULT 0 CHECK (file_count >= 0),
    logical_bytes INTEGER NOT NULL DEFAULT 0 CHECK (logical_bytes >= 0),
    allocated_bytes INTEGER CHECK (allocated_bytes IS NULL OR allocated_bytes >= 0),
    coverage_status TEXT NOT NULL DEFAULT 'unknown' CHECK (
        coverage_status IN ('unknown', 'complete', 'limited_access', 'partial')
    ),
    coverage_permille INTEGER CHECK (coverage_permille BETWEEN 0 AND 1000),
    issue_count INTEGER NOT NULL DEFAULT 0 CHECK (issue_count >= 0),
    CHECK (
        (root_path_encoding = 1 AND length(root_path) BETWEEN 1 AND 32768) OR
        (
            root_path_encoding = 2 AND
            length(root_path) BETWEEN 2 AND 65536 AND
            length(root_path) % 2 = 0
        )
    ),
    CHECK (
        (coverage_status = 'unknown' AND coverage_permille IS NULL) OR
        (coverage_status = 'complete' AND coverage_permille = 1000) OR
        (
            coverage_status IN ('limited_access', 'partial') AND
            (coverage_permille IS NULL OR coverage_permille < 1000)
        )
    ),
    CHECK (
        (
            snapshot_version IS NULL AND
            snapshot_relative_path IS NULL AND
            snapshot_relative_path_encoding IS NULL AND
            snapshot_checksum_sha256 IS NULL
        ) OR (
            snapshot_version IS NOT NULL AND
            snapshot_version > 0 AND
            snapshot_relative_path IS NOT NULL AND
            snapshot_relative_path_encoding IS NOT NULL AND
            (
                (
                    snapshot_relative_path_encoding = 1 AND
                    length(snapshot_relative_path) BETWEEN 1 AND 32768
                ) OR (
                    snapshot_relative_path_encoding = 2 AND
                    length(snapshot_relative_path) BETWEEN 2 AND 65536 AND
                    length(snapshot_relative_path) % 2 = 0
                )
            ) AND
            snapshot_checksum_sha256 IS NOT NULL AND
            length(snapshot_checksum_sha256) = 32
        )
    )
) STRICT;

CREATE TABLE scan_aggregates (
    scan_id TEXT NOT NULL REFERENCES scans(scan_id) ON DELETE CASCADE,
    aggregate_kind TEXT NOT NULL CHECK (
        aggregate_kind IN ('category', 'rule', 'top_level_path')
    ),
    aggregate_key BLOB NOT NULL,
    aggregate_key_encoding INTEGER NOT NULL,
    bytes INTEGER NOT NULL CHECK (bytes >= 0),
    file_count INTEGER NOT NULL CHECK (file_count >= 0),
    CHECK (
        (aggregate_key_encoding = 0 AND length(aggregate_key) BETWEEN 1 AND 16384) OR
        (aggregate_key_encoding = 1 AND length(aggregate_key) BETWEEN 1 AND 32768) OR
        (
            aggregate_key_encoding = 2 AND
            length(aggregate_key) BETWEEN 2 AND 65536 AND
            length(aggregate_key) % 2 = 0
        )
    ),
    PRIMARY KEY (scan_id, aggregate_kind, aggregate_key_encoding, aggregate_key)
) STRICT;

CREATE TABLE scan_issues (
    issue_id INTEGER PRIMARY KEY,
    scan_id TEXT NOT NULL REFERENCES scans(scan_id) ON DELETE CASCADE,
    shortened_path BLOB,
    shortened_path_encoding INTEGER,
    issue_kind TEXT NOT NULL CHECK (length(issue_kind) BETWEEN 1 AND 128),
    occurrence_count INTEGER NOT NULL DEFAULT 1 CHECK (occurrence_count BETWEEN 1 AND 1000000000),
    message_key TEXT NOT NULL CHECK (length(message_key) BETWEEN 1 AND 128),
    CHECK (
        (shortened_path IS NULL AND shortened_path_encoding IS NULL) OR
        (
            shortened_path IS NOT NULL AND
            shortened_path_encoding IS NOT NULL AND
            (
                (
                    shortened_path_encoding = 1 AND
                    length(shortened_path) BETWEEN 1 AND 32768
                ) OR (
                    shortened_path_encoding = 2 AND
                    length(shortened_path) BETWEEN 2 AND 65536 AND
                    length(shortened_path) % 2 = 0
                )
            )
        )
    )
) STRICT;

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
    )
) STRICT;

CREATE TABLE cleanup_sessions (
    session_id TEXT PRIMARY KEY CHECK (length(session_id) BETWEEN 1 AND 128),
    plan_id TEXT NOT NULL CHECK (length(plan_id) BETWEEN 1 AND 128),
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
            'planned', 'running', 'completed', 'partially_completed', 'failed',
            'cancelled', 'interrupted', 'rejected', 'dry_run'
        )
    ),
    UNIQUE (plan_id)
) STRICT;

CREATE TABLE cleanup_items (
    item_id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES cleanup_sessions(session_id) ON DELETE RESTRICT,
    item_ordinal INTEGER NOT NULL CHECK (item_ordinal >= 0),
    rule_id TEXT NOT NULL CHECK (length(rule_id) BETWEEN 1 AND 128),
    rule_revision INTEGER NOT NULL CHECK (rule_revision > 0),
    target_path BLOB NOT NULL,
    target_path_encoding INTEGER NOT NULL,
    estimated_bytes INTEGER NOT NULL CHECK (estimated_bytes >= 0),
    final_status TEXT NOT NULL CHECK (
        final_status IN (
            'planned', 'dry_run', 'trashed', 'removed', 'evicted', 'skipped',
            'rejected', 'failed', 'changed_since_plan', 'interrupted', 'unavailable'
        )
    ),
    error_category TEXT CHECK (
        error_category IS NULL OR length(error_category) BETWEEN 1 AND 128
    ),
    UNIQUE (session_id, item_ordinal),
    CHECK (
        (target_path_encoding = 1 AND length(target_path) BETWEEN 1 AND 32768) OR
        (
            target_path_encoding = 2 AND
            length(target_path) BETWEEN 2 AND 65536 AND
            length(target_path) % 2 = 0
        )
    )
) STRICT;

CREATE TABLE rule_outcomes (
    outcome_id INTEGER PRIMARY KEY,
    rule_id TEXT NOT NULL CHECK (length(rule_id) BETWEEN 1 AND 128),
    rule_revision INTEGER NOT NULL CHECK (rule_revision > 0),
    cleaned_at_unix_ms INTEGER NOT NULL CHECK (cleaned_at_unix_ms >= 0),
    cleaned_bytes INTEGER NOT NULL CHECK (cleaned_bytes >= 0),
    next_observed_bytes INTEGER CHECK (next_observed_bytes IS NULL OR next_observed_bytes >= 0),
    regrowth_duration_ms INTEGER CHECK (regrowth_duration_ms IS NULL OR regrowth_duration_ms >= 0)
) STRICT;

CREATE TABLE ai_insights (
    insight_id TEXT PRIMARY KEY CHECK (length(insight_id) BETWEEN 1 AND 128),
    input_digest BLOB NOT NULL CHECK (length(input_digest) = 32),
    provider TEXT NOT NULL CHECK (length(provider) BETWEEN 1 AND 128),
    adapter_version TEXT NOT NULL CHECK (length(adapter_version) BETWEEN 1 AND 128),
    model_label TEXT CHECK (model_label IS NULL OR length(model_label) BETWEEN 1 AND 256),
    output_schema_version INTEGER NOT NULL CHECK (output_schema_version > 0),
    output_payload BLOB NOT NULL CHECK (length(output_payload) BETWEEN 1 AND 16777216),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    expires_at_unix_ms INTEGER NOT NULL CHECK (expires_at_unix_ms >= created_at_unix_ms)
) STRICT;

CREATE TABLE schedules (
    schedule_id TEXT PRIMARY KEY CHECK (length(schedule_id) BETWEEN 1 AND 128),
    rule_id TEXT CHECK (rule_id IS NULL OR length(rule_id) BETWEEN 1 AND 128),
    category TEXT CHECK (
        category IS NULL OR category IN (
            'developer_artifact', 'application_cache', 'browser_cache',
            'log_and_diagnostic', 'installer_and_download', 'device_and_simulator_data',
            'cloud_file', 'large_review_item', 'protected_system_data', 'unknown_storage'
        )
    ),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    cadence_seconds INTEGER NOT NULL CHECK (cadence_seconds BETWEEN 3600 AND 31557600),
    minimum_age_seconds INTEGER NOT NULL CHECK (minimum_age_seconds >= 0),
    size_cap_bytes INTEGER NOT NULL CHECK (size_cap_bytes >= 0),
    last_run_unix_ms INTEGER CHECK (last_run_unix_ms IS NULL OR last_run_unix_ms >= 0),
    next_run_unix_ms INTEGER CHECK (next_run_unix_ms IS NULL OR next_run_unix_ms >= 0),
    CHECK ((rule_id IS NULL) != (category IS NULL))
) STRICT;

CREATE TABLE settings (
    setting_key TEXT PRIMARY KEY CHECK (length(setting_key) BETWEEN 1 AND 128),
    value_json TEXT NOT NULL CHECK (length(value_json) BETWEEN 1 AND 1048576),
    value_schema_version INTEGER NOT NULL CHECK (value_schema_version > 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0)
) STRICT;

CREATE INDEX disk_samples_by_kind_time
    ON disk_samples (sample_kind, sampled_at_unix_ms);
CREATE INDEX disk_samples_by_volume_kind_time
    ON disk_samples (volume_id, sample_kind, sampled_at_unix_ms DESC);
CREATE INDEX scans_by_volume_time
    ON scans (volume_id, started_at_unix_ms DESC);
CREATE INDEX scan_issues_by_scan_kind
    ON scan_issues (scan_id, issue_kind);
CREATE INDEX candidates_by_scan_status
    ON candidates (scan_id, status);
CREATE INDEX cleanup_sessions_by_time
    ON cleanup_sessions (started_at_unix_ms DESC);
CREATE INDEX cleanup_items_by_session
    ON cleanup_items (session_id, item_ordinal);
CREATE INDEX rule_outcomes_by_rule_time
    ON rule_outcomes (rule_id, rule_revision, cleaned_at_unix_ms DESC);
CREATE INDEX ai_insights_by_expiration
    ON ai_insights (expires_at_unix_ms);
CREATE UNIQUE INDEX ai_insights_by_identity
    ON ai_insights (input_digest, provider, adapter_version, ifnull(model_label, ''));
CREATE INDEX schedules_by_next_run
    ON schedules (enabled, next_run_unix_ms);
