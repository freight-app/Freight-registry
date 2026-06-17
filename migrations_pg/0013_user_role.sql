-- Replace the is_admin boolean with a role tier (user / moderator / admin).
-- is_admin is kept in sync for backward compatibility.
ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'user';
UPDATE users SET role = 'admin' WHERE is_admin <> 0;
