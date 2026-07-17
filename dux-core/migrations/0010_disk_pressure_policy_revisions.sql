ALTER TABLE disk_samples
ADD COLUMN policy_revision INTEGER NOT NULL DEFAULT 0 CHECK (policy_revision >= 0);
