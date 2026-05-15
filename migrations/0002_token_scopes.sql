-- Add scope column to tokens.
-- Existing tokens keep full publish capability by default.
ALTER TABLE tokens ADD COLUMN scope TEXT NOT NULL DEFAULT 'publish';
