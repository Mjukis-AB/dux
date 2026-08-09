SELECT 'DUX-DESTRUCTIVE: allow=migration-ai-insight-cache-v1-rebuild -- discard only never-admitted legacy AI cache rows while preserving every non-AI table';
DROP INDEX ai_insights_by_identity;
DROP INDEX ai_insights_by_expiration;
DROP TABLE ai_insights;

CREATE TABLE ai_insights (
    insight_id TEXT PRIMARY KEY CHECK (length(CAST(insight_id AS BLOB)) BETWEEN 1 AND 128),
    input_digest BLOB NOT NULL CHECK (length(input_digest) = 32),
    privacy_policy_revision INTEGER NOT NULL CHECK (privacy_policy_revision > 0),
    input_schema_version INTEGER NOT NULL CHECK (input_schema_version > 0),
    input_digest_revision INTEGER NOT NULL CHECK (input_digest_revision > 0),
    output_schema_version INTEGER NOT NULL CHECK (output_schema_version > 0),
    provider TEXT NOT NULL CHECK (length(CAST(provider AS BLOB)) BETWEEN 1 AND 128),
    adapter_id TEXT NOT NULL CHECK (length(CAST(adapter_id AS BLOB)) BETWEEN 1 AND 128),
    adapter_revision INTEGER NOT NULL CHECK (adapter_revision > 0),
    model_revision TEXT NOT NULL CHECK (length(CAST(model_revision AS BLOB)) BETWEEN 1 AND 256),
    output_payload BLOB NOT NULL CHECK (length(output_payload) BETWEEN 1 AND 65536),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    expires_at_unix_ms INTEGER NOT NULL CHECK (
        expires_at_unix_ms = created_at_unix_ms + 2592000000
    )
) STRICT;

CREATE INDEX ai_insights_by_expiration
    ON ai_insights (expires_at_unix_ms);
CREATE UNIQUE INDEX ai_insights_by_identity
    ON ai_insights (
        input_digest,
        privacy_policy_revision,
        input_schema_version,
        input_digest_revision,
        output_schema_version,
        provider,
        adapter_id,
        adapter_revision,
        model_revision
    );
