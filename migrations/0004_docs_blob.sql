-- Docify msgpack blobs — one per (package, version).
CREATE TABLE IF NOT EXISTS docs (
    id         INTEGER PRIMARY KEY,
    package_id INTEGER NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    version    TEXT    NOT NULL,
    data       BLOB    NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(package_id, version)
);
