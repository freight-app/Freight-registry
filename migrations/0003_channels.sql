-- Add channel support to packages.
--
-- Packages are now namespaced by channel (e.g. "stable", "experimental").
-- The same package name can exist in multiple channels independently.
-- The default channel for all existing packages is "stable".
--
-- SQLite cannot ALTER a UNIQUE constraint, so the table is recreated.

PRAGMA foreign_keys = OFF;

CREATE TABLE packages_new (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL COLLATE NOCASE,
    channel     TEXT    NOT NULL DEFAULT 'stable',
    description TEXT,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(name, channel)
);

INSERT INTO packages_new (id, name, channel, description, created_at)
SELECT id, name, 'stable', description, created_at FROM packages;

DROP TABLE packages;
ALTER TABLE packages_new RENAME TO packages;

PRAGMA foreign_keys = ON;
