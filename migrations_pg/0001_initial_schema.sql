-- freight-registry complete schema — PostgreSQL dialect.
-- Uses lower() functional indexes for case-insensitive uniqueness so that
-- sqlx's AnyPool can decode all columns as plain TEXT/BIGINT.
-- Timestamps are stored as BIGINT (Unix seconds) to stay compatible with the
-- SQLite schema so the same Rust code works against both backends.

-- ── Users ──────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS users (
    id             BIGSERIAL PRIMARY KEY,
    username       TEXT      NOT NULL,
    email          TEXT,
    password_hash  TEXT      NOT NULL,
    is_admin       INTEGER   NOT NULL DEFAULT 0,
    email_verified INTEGER   NOT NULL DEFAULT 0,
    totp_secret    TEXT,
    totp_enabled   INTEGER   NOT NULL DEFAULT 0,
    created_at     BIGINT    NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_username_ci ON users(lower(username));
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_email_ci    ON users(lower(email)) WHERE email IS NOT NULL;

-- ── Organisations ──────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS organizations (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT      NOT NULL,
    description TEXT,
    created_at  BIGINT    NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_organizations_name_ci ON organizations(lower(name));

CREATE TABLE IF NOT EXISTS org_members (
    org_id  BIGINT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id)         ON DELETE CASCADE,
    role    TEXT   NOT NULL DEFAULT 'member',  -- 'owner' | 'member'
    PRIMARY KEY (org_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_org_members_user ON org_members(user_id);

-- ── Packages & versions ────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS packages (
    id          BIGSERIAL PRIMARY KEY,
    name        TEXT      NOT NULL,
    channel     TEXT      NOT NULL DEFAULT 'stable',
    description TEXT,
    license     TEXT,
    keywords    TEXT,                          -- comma-separated, e.g. "math,geometry"
    org_id      BIGINT    REFERENCES organizations(id) ON DELETE SET NULL,
    created_at  BIGINT    NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_packages_name_channel_ci ON packages(lower(name), lower(channel));
CREATE INDEX IF NOT EXISTS idx_packages_org ON packages(org_id);

CREATE TABLE IF NOT EXISTS versions (
    id           BIGSERIAL PRIMARY KEY,
    package_id   BIGINT    NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    version      TEXT      NOT NULL,
    checksum     TEXT      NOT NULL,
    yanked       INTEGER   NOT NULL DEFAULT 0,
    downloads    INTEGER   NOT NULL DEFAULT 0,
    dependencies TEXT      NOT NULL DEFAULT '{}', -- JSON object: {"name":"version", …}
    upstream_url TEXT,                             -- redirect target for metadata-only packages
    build_system TEXT,                             -- "cmake" | "make" | "meson" | …
    created_at   BIGINT    NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT),
    UNIQUE(package_id, version)
);

CREATE TABLE IF NOT EXISTS package_owners (
    package_id BIGINT NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    user_id    BIGINT NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    PRIMARY KEY (package_id, user_id)
);

-- ── Prebuilt binary tarballs ───────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS prebuilts (
    id         BIGSERIAL PRIMARY KEY,
    name       TEXT      NOT NULL,
    channel    TEXT      NOT NULL DEFAULT 'stable',
    version    TEXT      NOT NULL,
    triple     TEXT      NOT NULL,             -- e.g. "x86_64-linux-gnu"
    checksum   TEXT      NOT NULL,
    created_at BIGINT    NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT),
    UNIQUE(name, channel, version, triple)
);

-- ── Tokens ─────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS tokens (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name       TEXT      NOT NULL,
    kind       TEXT      NOT NULL DEFAULT 'access',   -- 'access' | 'refresh'
    scope      TEXT      NOT NULL DEFAULT 'publish',  -- 'publish' | 'read' | 'admin'
    token_hash TEXT      NOT NULL UNIQUE,
    expires_at BIGINT,
    last_used  BIGINT,
    created_at BIGINT    NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT),
    UNIQUE(user_id, name)
);

-- ── Audit log ──────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS audit_log (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT    REFERENCES users(id) ON DELETE SET NULL,
    action     TEXT      NOT NULL,
    package    TEXT,
    version    TEXT,
    ip_addr    TEXT,
    created_at BIGINT    NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT)
);

-- ── Email verification & password reset ───────────────────────────────────────

CREATE TABLE IF NOT EXISTS email_tokens (
    id         BIGSERIAL PRIMARY KEY,
    user_id    BIGINT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind       TEXT      NOT NULL,             -- 'verify' | 'reset'
    token_hash TEXT      NOT NULL UNIQUE,
    expires_at BIGINT    NOT NULL,
    created_at BIGINT    NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT),
    UNIQUE(user_id, kind)
);

-- ── OAuth / OIDC linked accounts ───────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS oauth_accounts (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider    TEXT      NOT NULL,            -- slug: "github", "gitlab", "okta", …
    provider_id TEXT      NOT NULL,            -- stable ID issued by the provider
    login       TEXT      NOT NULL DEFAULT '', -- display name at the provider
    created_at  BIGINT    NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT),
    UNIQUE(provider, provider_id)
);
