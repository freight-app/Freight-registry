-- OAuth provider accounts.
-- Stores the link between a freight user and their external OAuth identity.
-- Currently only GitHub is supported; `provider` column makes adding more trivial.
CREATE TABLE IF NOT EXISTS oauth_accounts (
    id          INTEGER PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider    TEXT    NOT NULL DEFAULT 'github',
    provider_id TEXT    NOT NULL,          -- numeric ID from the provider
    login       TEXT    NOT NULL DEFAULT '', -- display name / username at the provider
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(provider, provider_id)
);
