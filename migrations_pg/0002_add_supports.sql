-- Add platform support expression column to versions table.
-- e.g. "!uwp & !arm", "linux", "x64"
ALTER TABLE versions ADD COLUMN supports TEXT;
