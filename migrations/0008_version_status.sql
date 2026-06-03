-- Add verification status to versions.
-- Existing rows default to 'published' so the registry stays fully operational
-- after the upgrade.  New publishes start as 'pending' until the CI container
-- job completes.
ALTER TABLE versions ADD COLUMN status        TEXT NOT NULL DEFAULT 'published';
ALTER TABLE versions ADD COLUMN status_reason TEXT;
