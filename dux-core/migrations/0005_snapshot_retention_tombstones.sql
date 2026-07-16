CREATE UNIQUE INDEX scans_snapshot_identity
    ON scans(
        scan_id,
        status,
        completed_at_unix_ms,
        snapshot_version,
        snapshot_relative_path_encoding,
        snapshot_relative_path,
        snapshot_checksum_sha256
    );

CREATE TABLE snapshot_retention_tombstones (
    scan_id TEXT PRIMARY KEY CHECK (length(scan_id) BETWEEN 1 AND 128),
    record_format_version INTEGER NOT NULL CHECK (record_format_version = 1),
    scan_status TEXT NOT NULL CHECK (scan_status = 'succeeded'),
    completed_at_unix_ms INTEGER NOT NULL CHECK (completed_at_unix_ms >= 0),
    snapshot_version INTEGER NOT NULL CHECK (snapshot_version > 0),
    snapshot_relative_path BLOB NOT NULL,
    snapshot_relative_path_encoding INTEGER NOT NULL,
    snapshot_checksum_sha256 BLOB NOT NULL CHECK (length(snapshot_checksum_sha256) = 32),
    committed_at_unix_ms INTEGER NOT NULL CHECK (
        committed_at_unix_ms >= completed_at_unix_ms
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

CREATE INDEX snapshot_retention_tombstones_by_commit
    ON snapshot_retention_tombstones(committed_at_unix_ms, scan_id);

CREATE TRIGGER snapshot_retention_tombstones_update_guard
BEFORE UPDATE ON snapshot_retention_tombstones
BEGIN
    SELECT RAISE(ABORT, 'snapshot retention tombstones are immutable');
END;

CREATE TRIGGER snapshot_retention_tombstones_delete_guard
BEFORE DELETE ON snapshot_retention_tombstones
BEGIN
    SELECT RAISE(ABORT, 'snapshot retention tombstones cannot be deleted');
END;
