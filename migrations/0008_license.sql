-- Add license field to packages table.
-- Stores the SPDX license identifier (e.g. "MIT", "Apache-2.0") as supplied
-- at publish time.  NULL for packages published before this migration.
ALTER TABLE packages ADD COLUMN license TEXT;
