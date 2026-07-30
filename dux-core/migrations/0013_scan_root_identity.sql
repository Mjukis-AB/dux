ALTER TABLE scans
ADD COLUMN root_identity_v1_sha256 BLOB
CHECK (
    root_identity_v1_sha256 IS NULL OR
    length(root_identity_v1_sha256) = 32
);

CREATE INDEX scans_by_exact_root_start
ON scans(root_path, root_path_encoding, started_at_unix_ms DESC, scan_id ASC);
