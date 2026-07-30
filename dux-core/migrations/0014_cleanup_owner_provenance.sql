ALTER TABLE cleanup_sessions
ADD COLUMN execution_host_identity_v1_sha256 BLOB
CHECK (
    execution_host_identity_v1_sha256 IS NULL OR
    (
        typeof(execution_host_identity_v1_sha256) = 'blob' AND
        length(execution_host_identity_v1_sha256) = 32
    )
);

ALTER TABLE cleanup_sessions
ADD COLUMN execution_boot_scope_v1_sha256 BLOB
CHECK (
    execution_boot_scope_v1_sha256 IS NULL OR
    (
        typeof(execution_boot_scope_v1_sha256) = 'blob' AND
        length(execution_boot_scope_v1_sha256) = 32
    )
);

ALTER TABLE cleanup_sessions
ADD COLUMN execution_recovery_policy TEXT
CHECK (
    execution_recovery_policy IS NULL OR
    (
        typeof(execution_recovery_policy) = 'text' AND
        execution_recovery_policy = 'resumable'
    )
);
