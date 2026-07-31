CREATE INDEX cleanup_items_by_unresolved_effect
ON cleanup_items(final_status, session_id, item_ordinal)
WHERE final_status IN ('effect_started', 'outcome_unknown');

CREATE INDEX cleanup_item_paths_by_unresolved_effect
ON cleanup_item_paths(status, session_id, item_ordinal, path_ordinal)
WHERE status IN ('effect_started', 'outcome_unknown');
