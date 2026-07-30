CREATE TABLE scan_scope_leases (
    lease_id BLOB PRIMARY KEY CHECK (
        typeof(lease_id) = 'blob' AND length(lease_id) = 16
    ),
    record_format_version INTEGER NOT NULL CHECK (record_format_version = 1),
    root_path BLOB NOT NULL,
    root_path_encoding INTEGER NOT NULL,
    owner_process_instance TEXT NOT NULL CHECK (
        typeof(owner_process_instance) = 'text' AND
        length(CAST(owner_process_instance AS BLOB)) BETWEEN 1 AND 128
    ),
    recovery_scope TEXT CHECK (
        recovery_scope IS NULL OR (
            typeof(recovery_scope) = 'text' AND
            length(recovery_scope) = 66 AND
            substr(recovery_scope, 1, 2) IN ('l:', 'm:') AND
            substr(recovery_scope, 3) NOT GLOB '*[^0-9a-f]*'
        )
    ),
    acquired_at_unix_ms INTEGER NOT NULL CHECK (
        typeof(acquired_at_unix_ms) = 'integer' AND acquired_at_unix_ms >= 0
    ),
    execution_host_identity_v1_sha256 BLOB CHECK (
        execution_host_identity_v1_sha256 IS NULL OR (
            typeof(execution_host_identity_v1_sha256) = 'blob' AND
            length(execution_host_identity_v1_sha256) = 32
        )
    ),
    execution_boot_scope_v1_sha256 BLOB CHECK (
        execution_boot_scope_v1_sha256 IS NULL OR (
            typeof(execution_boot_scope_v1_sha256) = 'blob' AND
            length(execution_boot_scope_v1_sha256) = 32
        )
    ),
    execution_recovery_policy TEXT CHECK (
        execution_recovery_policy IS NULL OR (
            typeof(execution_recovery_policy) = 'text' AND
            execution_recovery_policy = 'release_only'
        )
    ),
    CHECK (
        typeof(root_path) = 'blob' AND (
            (
                root_path_encoding = 1 AND
                length(root_path) BETWEEN 1 AND 32768
            ) OR (
                root_path_encoding = 2 AND
                length(root_path) BETWEEN 2 AND 65536 AND
                length(root_path) % 2 = 0
            )
        )
    )
) STRICT;

CREATE UNIQUE INDEX scan_scope_leases_by_exact_root
ON scan_scope_leases(root_path_encoding, root_path);

CREATE INDEX scan_scope_leases_by_owner
ON scan_scope_leases(owner_process_instance, lease_id);

CREATE INDEX scan_scope_leases_by_recovery_time
ON scan_scope_leases(acquired_at_unix_ms, lease_id);

CREATE TRIGGER scan_scope_leases_insert_guard
BEFORE INSERT ON scan_scope_leases
BEGIN
    SELECT CASE
        WHEN (SELECT count(*) FROM scan_scope_leases) >= 64
        THEN RAISE(ABORT, 'scan scope lease limit exceeded')
    END;
    SELECT CASE
        WHEN NOT (
            (
                NEW.execution_host_identity_v1_sha256 IS NULL AND
                NEW.execution_boot_scope_v1_sha256 IS NULL AND
                NEW.execution_recovery_policy IS NULL
            )
            OR
            (
                typeof(NEW.execution_host_identity_v1_sha256) = 'blob' AND
                length(NEW.execution_host_identity_v1_sha256) = 32 AND
                typeof(NEW.execution_boot_scope_v1_sha256) = 'blob' AND
                length(NEW.execution_boot_scope_v1_sha256) = 32 AND
                NEW.execution_recovery_policy = 'release_only' AND
                NEW.recovery_scope IS NOT NULL
            )
        )
        THEN RAISE(ABORT, 'scan scope lease provenance is incomplete')
    END;
END;

CREATE TRIGGER scan_scope_leases_update_guard
BEFORE UPDATE ON scan_scope_leases
BEGIN
    SELECT RAISE(ABORT, 'scan scope lease rows are immutable');
END;
