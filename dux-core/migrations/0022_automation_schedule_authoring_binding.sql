SELECT 'DUX-DESTRUCTIVE: allow=migration-automation-schedule-authoring-binding-v1-rebuild -- preserve every admitted v21 schedule, activation cursor, and exclusion while adding no fabricated authoring binding';

CREATE TABLE schedules_v22 (
    schedule_id TEXT PRIMARY KEY
        CHECK (length(CAST(schedule_id AS BLOB)) BETWEEN 1 AND 128),
    state TEXT NOT NULL DEFAULT 'disabled'
        CHECK (state IN ('disabled', 'enabled', 'paused')),
    pause_reason TEXT
        CHECK (pause_reason IS NULL OR pause_reason IN ('user', 'failure')),
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
    authoring_policy_revision INTEGER
        CHECK (authoring_policy_revision IS NULL OR authoring_policy_revision > 0),
    authoring_membership_sha256 BLOB
        CHECK (authoring_membership_sha256 IS NULL OR length(authoring_membership_sha256) = 32),
    pre_run_notifications_remaining INTEGER NOT NULL
        CHECK (pre_run_notifications_remaining BETWEEN 0 AND 3),
    revision INTEGER NOT NULL CHECK (revision > 0),
    cursor_revision INTEGER NOT NULL DEFAULT 0 CHECK (cursor_revision >= 0),
    recurrence_policy_revision INTEGER NOT NULL DEFAULT 0
        CHECK (recurrence_policy_revision IN (0, 1)),
    recurrence_anchor_unix_ms INTEGER CHECK (
        recurrence_anchor_unix_ms IS NULL
        OR recurrence_anchor_unix_ms BETWEEN 0 AND 253402300799999
    ),
    next_occurrence_ordinal INTEGER CHECK (
        next_occurrence_ordinal IS NULL OR next_occurrence_ordinal > 0
    ),
    next_run_unix_ms INTEGER CHECK (
        next_run_unix_ms IS NULL OR next_run_unix_ms BETWEEN 0 AND 253402300799999
    ),
    created_at_unix_ms INTEGER NOT NULL CHECK (
        created_at_unix_ms BETWEEN 0 AND 253402300799999
    ),
    updated_at_unix_ms INTEGER NOT NULL CHECK (
        updated_at_unix_ms BETWEEN created_at_unix_ms AND 253402300799999
    ),
    CHECK (
        (scope_kind = 'rule' AND rule_id IS NOT NULL AND rule_revision IS NOT NULL AND category IS NULL)
        OR
        (scope_kind = 'category' AND rule_id IS NULL AND rule_revision IS NULL AND category IS NOT NULL)
    ),
    CHECK (
        (authoring_policy_revision IS NULL AND authoring_membership_sha256 IS NULL)
        OR
        (scope_kind = 'category' AND authoring_policy_revision IS NOT NULL
            AND authoring_membership_sha256 IS NOT NULL)
    ),
    CHECK (notify_before_run = 1 OR pre_run_notifications_remaining = 0),
    CHECK (
        (state = 'disabled'
            AND pause_reason IS NULL
            AND cursor_revision = 0
            AND recurrence_policy_revision = 0
            AND recurrence_anchor_unix_ms IS NULL
            AND next_occurrence_ordinal IS NULL
            AND next_run_unix_ms IS NULL)
        OR
        (state IN ('enabled', 'paused')
            AND ((state = 'enabled' AND pause_reason IS NULL)
                OR (state = 'paused' AND pause_reason IS NOT NULL))
            AND cursor_revision > 0
            AND recurrence_policy_revision = 1
            AND cadence IN ('weekly', 'monthly')
            AND recurrence_anchor_unix_ms IS NOT NULL
            AND next_occurrence_ordinal IS NOT NULL
            AND next_run_unix_ms > recurrence_anchor_unix_ms)
    )
) STRICT;

INSERT INTO schedules_v22 (
    schedule_id, state, pause_reason, scope_kind, rule_id, rule_revision, category,
    cadence, minimum_age_seconds, minimum_reclaimable_bytes, maximum_bytes_per_run,
    notify_before_run, confirmation_mode,
    authoring_policy_revision, authoring_membership_sha256,
    pre_run_notifications_remaining, revision,
    cursor_revision, recurrence_policy_revision, recurrence_anchor_unix_ms,
    next_occurrence_ordinal, next_run_unix_ms,
    created_at_unix_ms, updated_at_unix_ms
)
SELECT
    schedule_id, state, pause_reason, scope_kind, rule_id, rule_revision, category,
    cadence, minimum_age_seconds, minimum_reclaimable_bytes, maximum_bytes_per_run,
    notify_before_run, confirmation_mode, NULL, NULL,
    pre_run_notifications_remaining, revision,
    cursor_revision, recurrence_policy_revision, recurrence_anchor_unix_ms,
    next_occurrence_ordinal, next_run_unix_ms,
    created_at_unix_ms, updated_at_unix_ms
FROM schedules;

CREATE TABLE schedule_rule_exclusions_v22_copy (
    schedule_id TEXT NOT NULL,
    exclusion_ordinal INTEGER NOT NULL CHECK (exclusion_ordinal BETWEEN 0 AND 31),
    rule_id TEXT NOT NULL
        CHECK (length(CAST(rule_id AS BLOB)) BETWEEN 1 AND 128),
    rule_revision INTEGER NOT NULL CHECK (rule_revision > 0),
    PRIMARY KEY (schedule_id, exclusion_ordinal),
    UNIQUE (schedule_id, rule_id, rule_revision)
) STRICT;

INSERT INTO schedule_rule_exclusions_v22_copy (
    schedule_id, exclusion_ordinal, rule_id, rule_revision
)
SELECT schedule_id, exclusion_ordinal, rule_id, rule_revision
FROM schedule_rule_exclusions;

DROP TABLE schedule_rule_exclusions;
DROP TABLE schedules;
ALTER TABLE schedules_v22 RENAME TO schedules;

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

INSERT INTO schedule_rule_exclusions (
    schedule_id, exclusion_ordinal, rule_id, rule_revision
)
SELECT schedule_id, exclusion_ordinal, rule_id, rule_revision
FROM schedule_rule_exclusions_v22_copy;

DROP TABLE schedule_rule_exclusions_v22_copy;

CREATE INDEX schedules_by_updated
    ON schedules (updated_at_unix_ms DESC, schedule_id);

CREATE INDEX schedules_enabled_by_next_run
    ON schedules (next_run_unix_ms, schedule_id)
    WHERE state = 'enabled' AND next_run_unix_ms IS NOT NULL;
