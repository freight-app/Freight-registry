-- Optional org binding on tokens.  When set, the token may only publish to
-- packages that belong to this org; all other operations are unaffected.
ALTER TABLE tokens ADD COLUMN org_id BIGINT REFERENCES organizations(id) ON DELETE SET NULL;
