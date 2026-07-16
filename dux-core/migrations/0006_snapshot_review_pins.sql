CREATE TABLE snapshot_review_pins (
    pin_id TEXT PRIMARY KEY CHECK (
        length(pin_id) = 32 AND pin_id NOT GLOB '*[^0-9a-f]*'
    ),
    record_format_version INTEGER NOT NULL CHECK (record_format_version = 1),
    scan_id TEXT NOT NULL CHECK (length(scan_id) BETWEEN 1 AND 128),
    scan_status TEXT NOT NULL CHECK (scan_status = 'succeeded'),
    completed_at_unix_ms INTEGER NOT NULL CHECK (completed_at_unix_ms >= 0),
    snapshot_version INTEGER NOT NULL CHECK (snapshot_version > 0),
    snapshot_relative_path BLOB NOT NULL,
    snapshot_relative_path_encoding INTEGER NOT NULL,
    snapshot_checksum_sha256 BLOB NOT NULL CHECK (length(snapshot_checksum_sha256) = 32),
    owner_process_instance TEXT NOT NULL CHECK (
        length(CAST(owner_process_instance AS BLOB)) BETWEEN 1 AND 128
    ),
    purpose TEXT NOT NULL CHECK (purpose IN ('explorer', 'cleanup_review')),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    renewed_at_unix_ms INTEGER NOT NULL CHECK (
        renewed_at_unix_ms >= created_at_unix_ms
    ),
    expires_at_unix_ms INTEGER NOT NULL CHECK (
        expires_at_unix_ms - renewed_at_unix_ms = 600000
    ),
    CHECK (
        (
            snapshot_relative_path_encoding = 1 AND
            length(snapshot_relative_path) BETWEEN 1 AND 32768
        ) OR (
            snapshot_relative_path_encoding = 2 AND
            length(snapshot_relative_path) BETWEEN 2 AND 65536 AND
            length(snapshot_relative_path) % 2 = 0
        )
    ),
    FOREIGN KEY (
        scan_id,
        scan_status,
        completed_at_unix_ms,
        snapshot_version,
        snapshot_relative_path_encoding,
        snapshot_relative_path,
        snapshot_checksum_sha256
    ) REFERENCES scans(
        scan_id,
        status,
        completed_at_unix_ms,
        snapshot_version,
        snapshot_relative_path_encoding,
        snapshot_relative_path,
        snapshot_checksum_sha256
    ) ON DELETE RESTRICT
) STRICT;

CREATE INDEX snapshot_review_pins_by_scan_expiration
    ON snapshot_review_pins(scan_id, expires_at_unix_ms, pin_id);

CREATE INDEX snapshot_review_pins_by_expiration
    ON snapshot_review_pins(expires_at_unix_ms, pin_id);

CREATE TRIGGER snapshot_review_pins_update_guard
BEFORE UPDATE ON snapshot_review_pins
WHEN
    NEW.pin_id IS NOT OLD.pin_id OR
    NEW.record_format_version IS NOT OLD.record_format_version OR
    NEW.scan_id IS NOT OLD.scan_id OR
    NEW.scan_status IS NOT OLD.scan_status OR
    NEW.completed_at_unix_ms IS NOT OLD.completed_at_unix_ms OR
    NEW.snapshot_version IS NOT OLD.snapshot_version OR
    NEW.snapshot_relative_path IS NOT OLD.snapshot_relative_path OR
    NEW.snapshot_relative_path_encoding IS NOT OLD.snapshot_relative_path_encoding OR
    NEW.snapshot_checksum_sha256 IS NOT OLD.snapshot_checksum_sha256 OR
    NEW.owner_process_instance IS NOT OLD.owner_process_instance OR
    NEW.purpose IS NOT OLD.purpose OR
    NEW.created_at_unix_ms IS NOT OLD.created_at_unix_ms OR
    NEW.renewed_at_unix_ms < OLD.renewed_at_unix_ms OR
    NEW.expires_at_unix_ms < OLD.expires_at_unix_ms
BEGIN
    SELECT RAISE(ABORT, 'snapshot review pin update is invalid');
END;
