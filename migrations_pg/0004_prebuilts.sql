-- Prebuilt binary tarballs — PostgreSQL dialect.

CREATE TABLE IF NOT EXISTS prebuilts (
    id         BIGSERIAL PRIMARY KEY,
    name       CITEXT    NOT NULL,
    channel    CITEXT    NOT NULL DEFAULT 'stable',
    version    TEXT      NOT NULL,
    triple     TEXT      NOT NULL,
    checksum   TEXT      NOT NULL,
    created_at BIGINT    NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT),
    UNIQUE(name, channel, version, triple)
);
