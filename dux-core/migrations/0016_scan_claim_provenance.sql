ALTER TABLE scan_process_claims
ADD COLUMN execution_host_identity_v1_sha256 BLOB
CHECK (
    execution_host_identity_v1_sha256 IS NULL OR
    (
        typeof(execution_host_identity_v1_sha256) = 'blob' AND
        length(execution_host_identity_v1_sha256) = 32
    )
);

ALTER TABLE scan_process_claims
ADD COLUMN execution_boot_scope_v1_sha256 BLOB
CHECK (
    execution_boot_scope_v1_sha256 IS NULL OR
    (
        typeof(execution_boot_scope_v1_sha256) = 'blob' AND
        length(execution_boot_scope_v1_sha256) = 32
    )
);

ALTER TABLE scan_process_claims
ADD COLUMN execution_recovery_policy TEXT
CHECK (
    execution_recovery_policy IS NULL OR
    (
        typeof(execution_recovery_policy) = 'text' AND
        execution_recovery_policy = 'interrupt_only'
    )
);

CREATE TRIGGER scan_process_claims_provenance_insert_guard
BEFORE INSERT ON scan_process_claims
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
        typeof(NEW.execution_recovery_policy) = 'text' AND
        NEW.execution_recovery_policy IS NOT NULL AND
        NEW.execution_recovery_policy = 'interrupt_only' AND
        NEW.recovery_scope IS NOT NULL
    )
)
BEGIN
    SELECT RAISE(ABORT, 'scan process claim provenance is incomplete');
END;

CREATE INDEX scan_process_claims_by_recovery_time
ON scan_process_claims(claimed_at_unix_ms, scan_id);
