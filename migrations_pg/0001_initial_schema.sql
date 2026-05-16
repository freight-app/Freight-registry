-- Initial schema — PostgreSQL dialect.
-- Uses CITEXT for case-insensitive text columns (equivalent to COLLATE NOCASE).
-- Timestamps are stored as INTEGER (unix seconds) for compatibility with the SQLite schema.

CREATE EXTENSION IF NOT EXISTS citext;

CREATE TABLE IF NOT EXISTS users (
    id             BIGSERIAL    PRIMARY KEY,
    username       CITEXT       NOT NULL UNIQUE,
    email          CITEXT       UNIQUE,
    password_hash  TEXT         NOT NULL,
    is_admin       INTEGER      NOT NULL DEFAULT 0,
    email_verified INTEGER      NOT NULL DEFAULT 0,
    totp_secret    TEXT,
    totp_enabled   INTEGER      NOT NULL DEFAULT 0,
    created_at     BIGINT       NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT)
);

CREATE TABLE IF NOT EXISTS packages (
    id          BIGSERIAL    PRIMARY KEY,
    name        CITEXT       NOT NULL UNIQUE,
    description TEXT,
    created_at  BIGINT       NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT)
);

CREATE TABLE IF NOT EXISTS versions (
    id         BIGSERIAL    PRIMARY KEY,
    package_id BIGINT       NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    version    TEXT         NOT NULL,
    checksum   TEXT         NOT NULL,
    yanked     INTEGER      NOT NULL DEFAULT 0,
    downloads  INTEGER      NOT NULL DEFAULT 0,
    created_at BIGINT       NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT),
    UNIQUE(package_id, version)
);

CREATE TABLE IF NOT EXISTS package_owners (
    package_id BIGINT NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    user_id    BIGINT NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    PRIMARY KEY (package_id, user_id)
);

CREATE TABLE IF NOT EXISTS tokens (
    id         BIGSERIAL    PRIMARY KEY,
    user_id    BIGINT       NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name       TEXT         NOT NULL,
    kind       TEXT         NOT NULL DEFAULT 'access',
    scope      TEXT         NOT NULL DEFAULT 'publish',
    token_hash TEXT         NOT NULL UNIQUE,
    expires_at BIGINT,
    last_used  BIGINT,
    created_at BIGINT       NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT),
    UNIQUE(user_id, name)
);

CREATE TABLE IF NOT EXISTS audit_log (
    id         BIGSERIAL    PRIMARY KEY,
    user_id    BIGINT       REFERENCES users(id) ON DELETE SET NULL,
    action     TEXT         NOT NULL,
    package    TEXT,
    version    TEXT,
    ip_addr    TEXT,
    created_at BIGINT       NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT)
);

CREATE TABLE IF NOT EXISTS email_tokens (
    id         BIGSERIAL    PRIMARY KEY,
    user_id    BIGINT       NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind       TEXT         NOT NULL,
    token_hash TEXT         NOT NULL UNIQUE,
    expires_at BIGINT       NOT NULL,
    created_at BIGINT       NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT),
    UNIQUE(user_id, kind)
);
