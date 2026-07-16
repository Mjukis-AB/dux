CREATE INDEX scans_by_snapshot_path
    ON scans(
        snapshot_relative_path_encoding,
        snapshot_relative_path,
        scan_id
    )
    WHERE snapshot_relative_path IS NOT NULL;
