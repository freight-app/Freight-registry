-- Store the [language.*] keys declared in freight.toml as a comma-separated
-- string, e.g. "c,cpp" or "fortran".  Empty/NULL means unknown (pre-migration
-- packages or metadata-only stubs).
ALTER TABLE versions ADD COLUMN languages TEXT;
