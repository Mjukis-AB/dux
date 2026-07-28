CREATE TABLE trusted_rust_target_plan_claims (
    candidate_id TEXT PRIMARY KEY
        REFERENCES candidate_plan_claims(candidate_id) ON DELETE CASCADE,
    session_id TEXT NOT NULL CHECK (
        length(CAST(session_id AS BLOB)) BETWEEN 1 AND 128
    ),
    item_ordinal INTEGER NOT NULL CHECK (item_ordinal >= 0),
    coupling_revision INTEGER NOT NULL CHECK (coupling_revision = 1),
    UNIQUE (session_id, item_ordinal)
) STRICT;
