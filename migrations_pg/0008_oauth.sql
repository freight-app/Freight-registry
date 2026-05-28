-- OAuth provider accounts (Postgres version).
CREATE TABLE IF NOT EXISTS oauth_accounts (
    id          BIGSERIAL PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider    TEXT   NOT NULL DEFAULT 'github',
    provider_id TEXT   NOT NULL,
    login       TEXT   NOT NULL DEFAULT '',
    created_at  BIGINT NOT NULL DEFAULT EXTRACT(EPOCH FROM NOW())::BIGINT,
    UNIQUE(provider, provider_id)
);
