# freight-registry — Architecture

Internal documentation for contributors.

---

## Module layout

```
freight-registry/
├── Cargo.toml
├── README.md
├── TODO.md
├── docs/
│   ├── architecture.md   # this file
│   └── api.md            # HTTP API reference
└── src/
    ├── main.rs           # CLI (clap): serve / user / token subcommands; AppState init
    ├── db.rs             # Db handle — all SQL; row types (UserRow, TokenRow, …)
    ├── auth.rs           # AuthToken + AdminToken extractors; hash_password (Argon2id PHC)
    ├── rate_limit.rs     # Limiters — two DefaultKeyedRateLimiter<IpAddr> via governor
    ├── storage.rs        # Storage — read/write tarballs under <data>/tarballs/<name>/
    ├── validate.rs       # Input validation — package names, semver, usernames, passwords
    └── api/
        ├── mod.rs        # router(), ApiError type, /api/v1/me handler
        ├── packages.rs   # GET /api/v1/packages/:name
        ├── download.rs   # GET /api/v1/packages/:name/:version/download
        ├── search.rs     # GET /api/v1/search
        ├── publish.rs    # PUT /api/v1/packages
        ├── yank.rs       # DELETE/PUT /api/v1/packages/:name/:version/yank
        ├── owners.rs     # GET/PUT/DELETE /api/v1/packages/:name/owners
        ├── login.rs      # POST /api/v1/users/login
        ├── register.rs   # POST /api/v1/users/register
        └── admin.rs      # GET  /api/v1/admin/users   (admin only)
```

---

## Database schema

Six tables in `registry.db` (SQLite, WAL mode):

```sql
users (
    id            INTEGER PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
    email         TEXT UNIQUE,
    password_hash TEXT NOT NULL,          -- Argon2id PHC string
    created_at    INTEGER DEFAULT (unixepoch())
)

tokens (
    id         INTEGER PRIMARY KEY,
    user_id    INTEGER REFERENCES users(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,             -- human label, unique per user
    token_hash TEXT NOT NULL UNIQUE,      -- SHA-256(raw_token) hex
    expires_at INTEGER,                   -- Unix timestamp, NULL = never
    last_used  INTEGER,                   -- updated async on each auth
    created_at INTEGER DEFAULT (unixepoch()),
    UNIQUE(user_id, name)
)

packages (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
    description TEXT,
    created_at  INTEGER DEFAULT (unixepoch())
)

versions (
    id         INTEGER PRIMARY KEY,
    package_id INTEGER REFERENCES packages(id) ON DELETE CASCADE,
    version    TEXT NOT NULL,
    checksum   TEXT NOT NULL,             -- SHA-256(tarball) hex
    yanked     INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch()),
    UNIQUE(package_id, version)
)

package_owners (
    package_id INTEGER REFERENCES packages(id) ON DELETE CASCADE,
    user_id    INTEGER REFERENCES users(id)    ON DELETE CASCADE,
    PRIMARY KEY (package_id, user_id)
)

audit_log (
    id         INTEGER PRIMARY KEY,
    user_id    INTEGER REFERENCES users(id) ON DELETE SET NULL,
    action     TEXT NOT NULL,             -- login | publish | yank | unyank
    package    TEXT,
    version    TEXT,
    ip_addr    TEXT,
    created_at INTEGER DEFAULT (unixepoch())
)
```

---

## Request lifecycle

### Authentication

Every auth-protected handler uses the `AuthToken` axum extractor (`auth.rs`):

```
Request header: Authorization: Bearer <raw_token>
   │
   ▼
SHA-256(raw_token) → token_hash
   │
   ▼
SELECT token WHERE token_hash = ? AND (expires_at IS NULL OR expires_at > now())
   │
   ├── not found → 401 Unauthorized
   └── found → UPDATE last_used (tokio::spawn, fire-and-forget)
               SELECT user WHERE id = token.user_id
               → AuthToken { user, token } injected into handler
```

Raw tokens are never stored — only their SHA-256 hash. This means a DB leak does not expose usable tokens.

### Publish flow

```
PUT /api/v1/packages
   │
   ├── 1. Write rate limit check (10 req/min/IP) → 429 if exceeded
   ├── 2. Parse binary body: [u32 JSON len][JSON meta][u32 tar len][tarball bytes]
   ├── 3. validate::package_name + validate::version → 400 on failure
   ├── 4. db.user_owns_package(user_id, name):
   │       None         → new package, proceed (first publisher claims it)
   │       Some(true)   → existing owner, proceed
   │       Some(false)  → 403 Forbidden
   ├── 5. Check for duplicate version in db → 409 Conflict
   ├── 6. SHA-256(tarball)
   ├── 7. storage.save(name, version, tarball) → write to tarballs/<name>/<version>.tar.gz
   ├── 8. db.publish_version(user_id, name, description, version, checksum)
   │       INSERT OR UPDATE packages
   │       INSERT versions
   │       if package_owners count == 0: INSERT package_owners (auto-claim)
   └── 9. db.audit("publish", …)  [fire-and-forget]
       → 200 { ok: true, warnings: { … } }
```

### Rate limiting

`rate_limit.rs` creates two `DefaultKeyedRateLimiter<IpAddr>` instances at startup via `governor`:

| Limiter | Burst | Refill | Used by |
|---|---|---|---|
| `limiters.api` | 120 req/min | 2 req/s | (reserved for general read endpoints) |
| `limiters.write` | 10 req/min | 1 req/6s | login, register, publish |
| `limiters.login` | 5 failures / 10 min → 15-min lockout | per username | login only |

---

## Key design decisions

**SQLite over PostgreSQL** — Single-file deployment. WAL mode gives concurrent reads with exclusive writes. Switching to PostgreSQL later is a compile-time feature flag (see TODO.md).

**SHA-256 token storage** — Tokens are 32-byte random hex strings. Only `SHA-256(token)` is stored. A full DB dump cannot be used to authenticate as any user.

**Argon2id passwords** — Industry-standard GPU-resistant KDF. PHC string format stored in `users.password_hash` includes algorithm, parameters, salt, and hash.

**Fire-and-forget audit + last_used** — Neither is on the critical path. Failures are silently dropped via `tokio::spawn`. This keeps request latency unaffected by audit write pressure.

**Cargo binary wire format for publish** — `[u32 LE JSON len][JSON][u32 LE tar len][tar]`. Matches Cargo's `PUT /api/v1/crates/new` format, making the endpoint compatible with tools that know how to publish to a Cargo registry.

**First-publisher ownership** — `publish_version` checks `package_owners` count after upsert; if zero, the publishing user is auto-granted ownership. Subsequent publishes require an existing owner.

**Last-owner removal guard** — `owners::remove` rejects a request that would leave `package_owners` empty for the package. The error message directs the user to add another owner first.

---

## AppState

```rust
pub struct AppState {
    pub db:       Db,        // cloneable pool handle
    pub storage:  Storage,   // tarball read/write
    pub base_url: String,    // embedded in download URLs
    pub limiters: Limiters,  // shared rate limit state
}
```

All handlers receive `State(Arc<AppState>)`. The `Arc` is cloned cheaply per request; `Db` and `Limiters` contain their own `Arc`-wrapped internals.
