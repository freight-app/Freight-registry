-- Abuse / problem reports filed against packages, triaged by admins.
CREATE TABLE IF NOT EXISTS reports (
    id           BIGSERIAL PRIMARY KEY,
    package      TEXT   NOT NULL,
    version      TEXT,
    reporter_id  BIGINT REFERENCES users(id) ON DELETE SET NULL,
    reason       TEXT   NOT NULL,
    details      TEXT   NOT NULL DEFAULT '',
    status       TEXT   NOT NULL DEFAULT 'open',
    created_at   BIGINT NOT NULL,
    resolved_by  BIGINT REFERENCES users(id) ON DELETE SET NULL,
    resolved_at  BIGINT,
    resolution   TEXT
);
CREATE INDEX IF NOT EXISTS idx_reports_status  ON reports(status);
CREATE INDEX IF NOT EXISTS idx_reports_package ON reports(package);
