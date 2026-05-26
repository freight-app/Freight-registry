-- Add license field to packages table.
ALTER TABLE packages ADD COLUMN IF NOT EXISTS license TEXT;
