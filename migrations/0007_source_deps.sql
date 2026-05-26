-- Add upstream source URL and build system for "metadata-only" package entries.
-- These allow the registry to store pointers to upstream source archives
-- (e.g. GitHub releases) without hosting the tarball itself.
-- When upstream_url is set the /download endpoint issues a 302 redirect to it.
ALTER TABLE versions ADD COLUMN upstream_url  TEXT;
ALTER TABLE versions ADD COLUMN build_system  TEXT;
