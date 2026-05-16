CREATE TABLE organizations (
    id          BIGSERIAL PRIMARY KEY,
    name        CITEXT    NOT NULL UNIQUE,
    description TEXT,
    created_at  BIGINT    NOT NULL DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT)
);

CREATE TABLE org_members (
    org_id  BIGINT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role    TEXT   NOT NULL DEFAULT 'member',
    PRIMARY KEY (org_id, user_id)
);

ALTER TABLE packages ADD COLUMN IF NOT EXISTS org_id BIGINT REFERENCES organizations(id) ON DELETE SET NULL;

CREATE INDEX idx_org_members_user ON org_members(user_id);
CREATE INDEX idx_packages_org     ON packages(org_id);
