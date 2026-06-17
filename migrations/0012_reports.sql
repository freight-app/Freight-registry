-- Abuse / problem reports filed against packages, triaged by admins.
CREATE TABLE IF NOT EXISTS reports (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    package      TEXT    NOT NULL,
    version      TEXT,
    reporter_id  INTEGER REFERENCES users(id) ON DELETE SET NULL,
    reason       TEXT    NOT NULL,
    details      TEXT    NOT NULL DEFAULT '',
    status       TEXT    NOT NULL DEFAULT 'open',
    created_at   INTEGER NOT NULL,
    resolved_by  INTEGER REFERENCES users(id) ON DELETE SET NULL,
    resolved_at  INTEGER,
    resolution   TEXT
);
CREATE INDEX IF NOT EXISTS idx_reports_status  ON reports(status);
CREATE INDEX IF NOT EXISTS idx_reports_package ON reports(package);
