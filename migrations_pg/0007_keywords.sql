-- Add keywords column to packages table for `freight add` TUI display.
ALTER TABLE packages ADD COLUMN keywords TEXT;
