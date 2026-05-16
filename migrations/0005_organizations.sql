CREATE TABLE organizations (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    description TEXT,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Maps users to orgs: role is 'owner' or 'member'
CREATE TABLE org_members (
    org_id  INTEGER NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role    TEXT    NOT NULL DEFAULT 'member',
    PRIMARY KEY (org_id, user_id)
);

-- Packages can be owned by a user OR an org (only one of these will be non-null)
ALTER TABLE packages ADD COLUMN org_id INTEGER REFERENCES organizations(id) ON DELETE SET NULL;

CREATE INDEX idx_org_members_user ON org_members(user_id);
CREATE INDEX idx_packages_org     ON packages(org_id);
