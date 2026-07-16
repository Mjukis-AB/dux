CREATE TABLE candidate_evaluations (
    scan_id TEXT PRIMARY KEY
        REFERENCES scans(scan_id) ON DELETE RESTRICT,
    record_format_version INTEGER NOT NULL CHECK (record_format_version = 1),
    evaluator_revision INTEGER NOT NULL CHECK (evaluator_revision > 0),
    rule_catalog_schema_version INTEGER NOT NULL CHECK (rule_catalog_schema_version > 0),
    rule_catalog_sha256 BLOB NOT NULL CHECK (length(rule_catalog_sha256) = 32),
    context_format_version INTEGER NOT NULL CHECK (context_format_version > 0),
    context_sha256 BLOB NOT NULL CHECK (length(context_sha256) = 32),
    snapshot_version INTEGER NOT NULL CHECK (snapshot_version > 0),
    snapshot_sha256 BLOB NOT NULL CHECK (length(snapshot_sha256) = 32),
    scheduled_at_unix_ms INTEGER NOT NULL CHECK (scheduled_at_unix_ms >= 0),
    completed_at_unix_ms INTEGER CHECK (
        completed_at_unix_ms IS NULL OR
        completed_at_unix_ms >= scheduled_at_unix_ms
    ),
    status TEXT NOT NULL CHECK (status IN ('pending', 'succeeded', 'failed')),
    candidate_count INTEGER CHECK (candidate_count IS NULL OR candidate_count >= 0),
    failure_kind TEXT CHECK (
        failure_kind IS NULL OR failure_kind IN (
            'cancelled', 'catalog_invalid', 'context_invalid', 'evaluation_failed',
            'candidate_invalid', 'limit_exceeded'
        )
    ),
    CHECK (
        (status = 'pending' AND completed_at_unix_ms IS NULL AND
            candidate_count IS NULL AND failure_kind IS NULL) OR
        (status = 'succeeded' AND completed_at_unix_ms IS NOT NULL AND
            candidate_count IS NOT NULL AND failure_kind IS NULL) OR
        (status = 'failed' AND completed_at_unix_ms IS NOT NULL AND
            candidate_count IS NULL AND failure_kind IS NOT NULL)
    )
) STRICT;

CREATE INDEX candidate_evaluations_by_status
    ON candidate_evaluations(status, scheduled_at_unix_ms, scan_id);
