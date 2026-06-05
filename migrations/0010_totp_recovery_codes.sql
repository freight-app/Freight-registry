-- One-use TOTP recovery codes (SHA-256 hashes stored, never plaintext).
CREATE TABLE IF NOT EXISTS totp_recovery_codes (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id  INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT    NOT NULL,
    used     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_recovery_codes_user ON totp_recovery_codes(user_id);
