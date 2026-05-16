-- Prebuilt binary tarballs per (package, channel, version, target triple).
-- A publisher can upload compiled artifacts for common triples alongside the
-- source tarball so consumers can skip compilation.

CREATE TABLE IF NOT EXISTS prebuilts (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL COLLATE NOCASE,
    channel    TEXT    NOT NULL DEFAULT 'stable',
    version    TEXT    NOT NULL,
    triple     TEXT    NOT NULL,
    checksum   TEXT    NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(name, channel, version, triple)
);
