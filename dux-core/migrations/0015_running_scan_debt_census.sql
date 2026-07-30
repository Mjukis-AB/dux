CREATE INDEX scans_running_by_started
ON scans(started_at_unix_ms, scan_id)
WHERE status = 'running';
