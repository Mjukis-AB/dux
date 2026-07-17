CREATE TABLE scan_process_claims (
    scan_id TEXT PRIMARY KEY CHECK (length(scan_id) BETWEEN 1 AND 128)
        REFERENCES scans(scan_id) ON DELETE RESTRICT,
    record_format_version INTEGER NOT NULL CHECK (record_format_version = 1),
    owner_process_instance TEXT NOT NULL CHECK (
        length(CAST(owner_process_instance AS BLOB)) BETWEEN 1 AND 128
    ),
    recovery_scope TEXT CHECK (
        recovery_scope IS NULL OR (
            length(recovery_scope) = 66 AND
            substr(recovery_scope, 1, 2) IN ('l:', 'm:') AND
            substr(recovery_scope, 3) NOT GLOB '*[^0-9a-f]*'
        )
    ),
    claimed_at_unix_ms INTEGER NOT NULL CHECK (claimed_at_unix_ms >= 0)
) STRICT;

CREATE INDEX scan_process_claims_by_time
ON scan_process_claims(recovery_scope, claimed_at_unix_ms, scan_id);

CREATE INDEX scan_process_claims_by_owner
ON scan_process_claims(owner_process_instance, scan_id);

CREATE TRIGGER scan_process_claims_insert_guard
BEFORE INSERT ON scan_process_claims
BEGIN
    SELECT CASE
        WHEN (
            SELECT count(*) FROM scan_process_claims
            WHERE owner_process_instance = NEW.owner_process_instance
        ) >= 64
        THEN RAISE(ABORT, 'scan process claim limit exceeded')
    END;
    SELECT CASE
        WHEN NOT EXISTS (
            SELECT 1 FROM scans
            WHERE
                scan_id = NEW.scan_id AND
                status = 'running' AND
                completed_at_unix_ms IS NULL AND
                directory_count = 0 AND file_count = 0 AND logical_bytes = 0 AND
                allocated_bytes IS NULL AND
                snapshot_version IS NULL AND
                snapshot_relative_path IS NULL AND
                snapshot_relative_path_encoding IS NULL AND
                snapshot_checksum_sha256 IS NULL AND
                coverage_status = 'unknown' AND coverage_permille IS NULL AND
                issue_count = 0 AND
                started_at_unix_ms = NEW.claimed_at_unix_ms AND
                NOT EXISTS (
                    SELECT 1 FROM scan_issues
                    WHERE scan_issues.scan_id = scans.scan_id
                )
        )
        THEN RAISE(ABORT, 'scan process claim parent is not pristine running')
    END;
END;

CREATE TRIGGER scan_process_claims_update_guard
BEFORE UPDATE ON scan_process_claims
BEGIN
    SELECT RAISE(ABORT, 'scan process claim rows are immutable');
END;

CREATE TRIGGER scans_terminal_with_process_claim_guard
BEFORE UPDATE OF status ON scans
WHEN
    OLD.status = 'running' AND NEW.status != 'running' AND
    EXISTS (
        SELECT 1 FROM scan_process_claims
        WHERE scan_id = OLD.scan_id
    )
BEGIN
    SELECT RAISE(ABORT, 'terminal scan cannot retain a process claim');
END;
