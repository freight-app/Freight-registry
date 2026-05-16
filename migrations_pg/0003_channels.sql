-- Add channel support to packages — PostgreSQL dialect.
-- PostgreSQL supports adding a UNIQUE constraint directly so no table recreation needed.

ALTER TABLE packages ADD COLUMN IF NOT EXISTS channel CITEXT NOT NULL DEFAULT 'stable';

-- Drop the old single-column unique constraint and add the composite one.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'packages_name_key' AND conrelid = 'packages'::regclass
    ) THEN
        ALTER TABLE packages DROP CONSTRAINT packages_name_key;
    END IF;
END$$;

ALTER TABLE packages DROP CONSTRAINT IF EXISTS packages_name_channel_key;
ALTER TABLE packages ADD CONSTRAINT packages_name_channel_key UNIQUE (name, channel);
