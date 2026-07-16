ALTER TABLE cleanup_sessions
    ADD COLUMN candidate_status_coupling_version INTEGER NOT NULL DEFAULT 1
        CHECK (candidate_status_coupling_version IN (1, 2));

CREATE TABLE candidate_plan_claims (
    candidate_id TEXT PRIMARY KEY
        REFERENCES candidates(candidate_id) ON DELETE RESTRICT,
    session_id TEXT NOT NULL,
    item_ordinal INTEGER NOT NULL CHECK (item_ordinal >= 0),
    prior_review_status TEXT NOT NULL CHECK (
        prior_review_status IN ('discovered', 'selected')
    ),
    UNIQUE (session_id, item_ordinal),
    FOREIGN KEY (session_id, item_ordinal)
        REFERENCES cleanup_items(session_id, item_ordinal) ON DELETE RESTRICT
) STRICT;
