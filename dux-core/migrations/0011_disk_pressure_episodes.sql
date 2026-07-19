CREATE TABLE disk_pressure_episodes (
    episode_id INTEGER PRIMARY KEY,
    volume_id TEXT NOT NULL REFERENCES volumes(volume_id) ON DELETE RESTRICT,
    pressure TEXT NOT NULL CHECK (pressure IN ('warning', 'critical')),
    entered_at_unix_ms INTEGER NOT NULL CHECK (entered_at_unix_ms >= 0),
    exited_at_unix_ms INTEGER CHECK (
        exited_at_unix_ms IS NULL OR exited_at_unix_ms >= entered_at_unix_ms
    ),
    policy_revision INTEGER NOT NULL CHECK (policy_revision >= 0),
    UNIQUE (volume_id, pressure, entered_at_unix_ms)
) STRICT;

CREATE UNIQUE INDEX disk_pressure_episodes_open_by_volume
    ON disk_pressure_episodes (volume_id)
    WHERE exited_at_unix_ms IS NULL;

CREATE INDEX disk_pressure_episodes_by_volume_time
    ON disk_pressure_episodes (volume_id, entered_at_unix_ms DESC);
