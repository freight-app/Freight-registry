-- freight-registry complete schema — SQLite dialect.

-- ── Users ──────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS users (
    id             INTEGER PRIMARY KEY,
    username       TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    email          TEXT    UNIQUE,
    password_hash  TEXT    NOT NULL,
    is_admin       INTEGER NOT NULL DEFAULT 0,
    email_verified INTEGER NOT NULL DEFAULT 0,
    totp_secret    TEXT,
    totp_enabled   INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL DEFAULT (unixepoch())
);

-- ── Organisations ──────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS organizations (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    description TEXT,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE IF NOT EXISTS org_members (
    org_id  INTEGER NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id)         ON DELETE CASCADE,
    role    TEXT    NOT NULL DEFAULT 'member',  -- 'owner' | 'member'
    PRIMARY KEY (org_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_org_members_user ON org_members(user_id);

-- ── Packages & versions ────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS packages (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL COLLATE NOCASE,
    channel     TEXT    NOT NULL DEFAULT 'stable',
    description TEXT,
    license     TEXT,
    keywords    TEXT,                          -- comma-separated, e.g. "math,geometry"
    org_id      INTEGER REFERENCES organizations(id) ON DELETE SET NULL,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(name, channel)
);

CREATE INDEX IF NOT EXISTS idx_packages_org ON packages(org_id);

CREATE TABLE IF NOT EXISTS versions (
    id           INTEGER PRIMARY KEY,
    package_id   INTEGER NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    version      TEXT    NOT NULL,
    checksum     TEXT    NOT NULL,
    yanked       INTEGER NOT NULL DEFAULT 0,
    downloads    INTEGER NOT NULL DEFAULT 0,
    dependencies TEXT    NOT NULL DEFAULT '{}', -- JSON object: {"name":"version", …}
    upstream_url TEXT,                           -- redirect target for metadata-only packages
    build_system TEXT,                           -- "cmake" | "make" | "meson" | …
    created_at   INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(package_id, version)
);

CREATE TABLE IF NOT EXISTS package_owners (
    package_id INTEGER NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL REFERENCES users(id)    ON DELETE CASCADE,
    PRIMARY KEY (package_id, user_id)
);

-- ── Prebuilt binary tarballs ───────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS prebuilts (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL COLLATE NOCASE,
    channel    TEXT    NOT NULL DEFAULT 'stable',
    version    TEXT    NOT NULL,
    triple     TEXT    NOT NULL,               -- e.g. "x86_64-linux-gnu"
    checksum   TEXT    NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(name, channel, version, triple)
);

-- ── Tokens ─────────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS tokens (
    id         INTEGER PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,
    kind       TEXT    NOT NULL DEFAULT 'access',   -- 'access' | 'refresh'
    scope      TEXT    NOT NULL DEFAULT 'publish',  -- 'publish' | 'read' | 'admin'
    token_hash TEXT    NOT NULL UNIQUE,
    expires_at INTEGER,
    last_used  INTEGER,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(user_id, name)
);

-- ── Audit log ──────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS audit_log (
    id         INTEGER PRIMARY KEY,
    user_id    INTEGER REFERENCES users(id) ON DELETE SET NULL,
    action     TEXT    NOT NULL,
    package    TEXT,
    version    TEXT,
    ip_addr    TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

-- ── Email verification & password reset ───────────────────────────────────────

CREATE TABLE IF NOT EXISTS email_tokens (
    id         INTEGER PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind       TEXT    NOT NULL,               -- 'verify' | 'reset'
    token_hash TEXT    NOT NULL UNIQUE,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(user_id, kind)
);

-- ── OAuth / OIDC linked accounts ───────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS oauth_accounts (
    id          INTEGER PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider    TEXT    NOT NULL,              -- slug: "github", "gitlab", "okta", …
    provider_id TEXT    NOT NULL,              -- stable ID issued by the provider
    login       TEXT    NOT NULL DEFAULT '',   -- display name at the provider
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(provider, provider_id)
);
