CREATE TABLE snapshot_temp_leases (
    lease_id TEXT PRIMARY KEY CHECK (
        length(lease_id) = 32 AND lease_id NOT GLOB '*[^0-9a-f]*'
    ),
    record_format_version INTEGER NOT NULL CHECK (record_format_version = 1),
    scan_id TEXT NOT NULL UNIQUE CHECK (length(scan_id) BETWEEN 1 AND 128)
        REFERENCES scans(scan_id) ON DELETE RESTRICT,
    scan_status TEXT NOT NULL CHECK (scan_status = 'running'),
    final_relative_name TEXT NOT NULL UNIQUE CHECK (
        length(final_relative_name) = 85 AND
        substr(final_relative_name, 1, 9) = 'snapshot-' AND
        substr(final_relative_name, 10, 64) NOT GLOB '*[^0-9a-f]*' AND
        substr(final_relative_name, 74) = '.duxsnapshot'
    ),
    temp_relative_name TEXT NOT NULL UNIQUE CHECK (
        length(temp_relative_name) BETWEEN 113 AND 122 AND
        substr(temp_relative_name, 1, 10) = '.snapshot-' AND
        substr(temp_relative_name, 11, 64) NOT GLOB '*[^0-9a-f]*' AND
        substr(temp_relative_name, 75, 1) = '.' AND
        substr(temp_relative_name, 76, length(temp_relative_name) - 112)
            NOT GLOB '*[^0-9]*' AND
        substr(temp_relative_name, 76, 1) BETWEEN '1' AND '9' AND
        substr(temp_relative_name, length(temp_relative_name) - 36, 1) = '.' AND
        substr(temp_relative_name, length(temp_relative_name) - 35, 32)
            NOT GLOB '*[^0-9a-f]*' AND
        substr(temp_relative_name, -4) = '.tmp'
    ),
    owner_process_instance TEXT NOT NULL CHECK (
        length(CAST(owner_process_instance AS BLOB)) BETWEEN 1 AND 128
    ),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    CHECK (
        substr(temp_relative_name, 11, 64) =
        substr(final_relative_name, 10, 64)
    )
) STRICT;

CREATE TRIGGER snapshot_temp_leases_insert_guard
BEFORE INSERT ON snapshot_temp_leases
BEGIN
    SELECT CASE
        WHEN (SELECT count(*) FROM snapshot_temp_leases) >= 64
        THEN RAISE(ABORT, 'snapshot temp lease limit exceeded')
    END;
    SELECT CASE
        WHEN NOT EXISTS (
            SELECT 1 FROM scans
            WHERE
                scan_id = NEW.scan_id AND
                status = 'running' AND
                snapshot_version IS NULL AND
                snapshot_relative_path IS NULL AND
                snapshot_relative_path_encoding IS NULL AND
                snapshot_checksum_sha256 IS NULL
        )
        THEN RAISE(ABORT, 'snapshot temp lease parent is not running')
    END;
END;

CREATE TRIGGER snapshot_temp_leases_update_guard
BEFORE UPDATE ON snapshot_temp_leases
BEGIN
    SELECT RAISE(ABORT, 'snapshot temp lease rows are immutable');
END;

CREATE TRIGGER scans_succeeded_without_temp_lease_guard
BEFORE UPDATE OF status ON scans
WHEN
    NEW.status = 'succeeded' AND
    EXISTS (
        SELECT 1 FROM snapshot_temp_leases
        WHERE scan_id = NEW.scan_id
    )
BEGIN
    SELECT RAISE(ABORT, 'succeeded scan cannot retain a snapshot temp lease');
END;
