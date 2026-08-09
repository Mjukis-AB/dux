SELECT 'DUX-DESTRUCTIVE: allow=schema-migration-drop-never-admitted-schedule-reservation -- v1 schedule rows had no reviewed producer or consumer and cannot prove automation consent';
DROP INDEX schedules_by_next_run;
DROP TABLE schedules;

CREATE TABLE schedules (
    schedule_id TEXT PRIMARY KEY
        CHECK (length(CAST(schedule_id AS BLOB)) BETWEEN 1 AND 128),
    state TEXT NOT NULL DEFAULT 'disabled_draft' CHECK (state = 'disabled_draft'),
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('rule', 'category')),
    rule_id TEXT CHECK (
        rule_id IS NULL OR length(CAST(rule_id AS BLOB)) BETWEEN 1 AND 128
    ),
    rule_revision INTEGER CHECK (rule_revision IS NULL OR rule_revision > 0),
    category TEXT CHECK (
        category IS NULL OR category IN (
            'developer_artifact', 'application_cache', 'browser_cache',
            'log_and_diagnostic', 'installer_and_download', 'device_and_simulator_data',
            'cloud_file', 'large_review_item', 'protected_system_data', 'unknown_storage'
        )
    ),
    cadence TEXT NOT NULL CHECK (cadence IN ('weekly', 'monthly', 'low_disk_only')),
    minimum_age_seconds INTEGER NOT NULL
        CHECK (minimum_age_seconds BETWEEN 0 AND 3153600000),
    minimum_reclaimable_bytes INTEGER NOT NULL CHECK (minimum_reclaimable_bytes >= 0),
    maximum_bytes_per_run INTEGER NOT NULL CHECK (maximum_bytes_per_run > 0),
    notify_before_run INTEGER NOT NULL CHECK (notify_before_run IN (0, 1)),
    confirmation_mode TEXT NOT NULL
        CHECK (confirmation_mode IN ('require_confirmation', 'fully_automatic')),
    pre_run_notifications_remaining INTEGER NOT NULL
        CHECK (pre_run_notifications_remaining BETWEEN 0 AND 3),
    revision INTEGER NOT NULL CHECK (revision > 0),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= created_at_unix_ms),
    CHECK (
        (scope_kind = 'rule' AND rule_id IS NOT NULL AND rule_revision IS NOT NULL AND category IS NULL)
        OR
        (scope_kind = 'category' AND rule_id IS NULL AND rule_revision IS NULL AND category IS NOT NULL)
    ),
    CHECK (notify_before_run = 1 OR pre_run_notifications_remaining = 0)
) STRICT;

CREATE TABLE schedule_rule_exclusions (
    schedule_id TEXT NOT NULL,
    exclusion_ordinal INTEGER NOT NULL CHECK (exclusion_ordinal BETWEEN 0 AND 31),
    rule_id TEXT NOT NULL
        CHECK (length(CAST(rule_id AS BLOB)) BETWEEN 1 AND 128),
    rule_revision INTEGER NOT NULL CHECK (rule_revision > 0),
    PRIMARY KEY (schedule_id, exclusion_ordinal),
    UNIQUE (schedule_id, rule_id, rule_revision),
    FOREIGN KEY (schedule_id) REFERENCES schedules(schedule_id) ON DELETE CASCADE
) STRICT;

CREATE INDEX schedules_by_updated
    ON schedules (updated_at_unix_ms DESC, schedule_id);
