# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this project is

`freight-registry` is the self-hosted registry server companion to the [`freight`](https://github.com/TiniTinyTerminator/freight) package manager — a Cargo-inspired build tool for C, C++, Fortran, CUDA, and other GCC/Clang languages. The registry implements the same HTTP wire protocol as `freight.dev` so any `freight` client can point at it with a single config entry.

**Two repos developed in tandem:**
- `/home/max/freight-registry` — this repo; the HTTP server (Axum + SQLite)
- `/home/max/Freight` — the freight CLI; the client that talks to this server

When the wire protocol or API shape changes, both repos usually need updating together. The client-side registry implementation is at `Freight/crates/freight-core/src/registry/freight_registry.rs`.

## Commands

```sh
cargo build                      # compile (dev)
cargo build --release            # release build
cargo check                      # type-check without linking (faster)
cargo clippy                     # lint
cargo test                       # run tests (no test suite yet)

# Run server locally for manual testing
cargo run -- --data /tmp/freight-dev serve --base-url http://localhost:7878

# Create a test user
cargo run -- --data /tmp/freight-dev user add alice --email alice@example.com

# Issue a token
cargo run -- --data /tmp/freight-dev token add dev --user alice
```

Default bind: `0.0.0.0:7878`. Default data dir: `/var/lib/freight-registry`.

## Architecture

Single Rust binary with two modes: CLI admin commands (`user`, `token`) and an Axum HTTP server (`serve`).

**Module map**

| File | Responsibility |
|---|---|
| `main.rs` | CLI (clap): `serve` / `user` / `token` subcommands; `AppState` construction; audit pruning background task |
| `db.rs` | `Db` pool handle — all SQL; `UserRow`, `TokenRow`, `PackageRow`, `VersionRow`; `migrate()` |
| `auth.rs` | `AuthToken`, `AdminToken`, `RefreshTokenAuth` extractors; `hash_password` (Argon2id PHC) |
| `totp.rs` | TOTP: `generate_secret_b32()`, `provisioning_uri()`, `verify()` — wraps `totp-rs` |
| `rate_limit.rs` | `Limiters`: two `governor` per-IP buckets + per-username `LoginLimiter` |
| `storage.rs` | Tarball read/write — `<data>/tarballs/<name>/<version>/<name>-<version>.tar.gz` |
| `validate.rs` | Input validation: `package_name`, `version`, `username`, `password` |
| `api/mod.rs` | `router()`, `ApiError` type, CORS layer, security-headers middleware |
| `api/publish.rs` | `PUT /api/v1/packages` — binary wire format parser + publish flow |
| `api/login.rs` | `POST /api/v1/users/login` — password verify, TOTP check, issue access+refresh tokens |
| `api/register.rs` | `POST /api/v1/users/register` — create account, issue initial token, log email verify link |
| `api/email.rs` | `GET /api/v1/users/verify-email` — consume verify token |
| `api/reset.rs` | `POST /api/v1/users/reset-password/request` + `/confirm` — stdout-logged reset links |
| `api/totp.rs` | `POST /api/v1/me/totp/enroll` + `/confirm`; `DELETE /api/v1/me/totp` |
| `api/refresh.rs` | `POST /api/v1/auth/refresh` — exchange refresh token for new access token |
| `api/admin.rs` | `GET /api/v1/admin/users` — admin-only user listing |
| `api/{packages,download,search,yank,owners}.rs` | Package read/download/search/yank/ownership endpoints |

## Database schema

SQLite, WAL mode, single file at `<data>/registry.db`. All migrations run at startup in `db.rs::migrate()`. Schema changes are always additive — never drop/recreate tables.

```
users           id, username, email, password_hash, is_admin, email_verified,
                totp_secret, totp_enabled, created_at

tokens          id, user_id, name, kind, token_hash, expires_at, last_used, created_at
                kind: "api" (CLI token) | "access" (login session) | "refresh"

email_tokens    id, user_id, kind, token_hash, expires_at, created_at
                kind: "verify" (24 h) | "reset" (1 h); one pending per (user, kind)

packages        id, name, description, created_at
versions        id, package_id, version, checksum, yanked, created_at
package_owners  package_id, user_id   (first publisher auto-claimed)
audit_log       id, user_id, action, package, version, ip_addr, created_at
```

**Key invariants:**
- `password_hash` is `Argon2id(SHA-256(plaintext))` — the SHA-256 is done client-side so plaintext never crosses the wire
- `token_hash` is `SHA-256(raw_token)` hex — raw tokens are never stored; a DB dump cannot authenticate
- `email_tokens` uses DELETE+INSERT (not UPSERT) to avoid hash collision edge cases

## Auth flow

```
Authorization: Bearer <raw_token>
  → SHA-256(raw_token) → look up in tokens
  → reject if expired or kind="refresh"
  → fetch UserRow by user_id
  → inject AuthToken { user, token } into handler
```

`AdminToken` wraps `AuthToken` and additionally checks `user.is_admin != 0`.  
`RefreshTokenAuth` is the mirror: accepts only `kind="refresh"` tokens, used only at `POST /api/v1/auth/refresh`.

## Publish wire format

Matches cargo's `PUT /api/v1/crates/new` binary format:
```
[u32 LE: JSON metadata length][JSON bytes][u32 LE: tarball length][tarball bytes]
```
JSON fields: `name` (required), `vers` (required), `description`, `license`.

## Input validation rules

| Field | Constraints |
|---|---|
| Package name | 1–64 chars, `[a-zA-Z0-9_-]`, no leading/trailing/consecutive separators, not reserved |
| Version | semver `major.minor[.patch][-pre][+build]` |
| Username | 2–32 chars, `[a-zA-Z0-9_-]`, must start with a letter |
| Password (plaintext) | min 8 chars (enforced client-side before SHA-256 hash) |

Reserved package names include `std`, `core`, `freight`, `registry`, `crate`, and a few others — see `validate.rs`.

## Rate limiting

| Limiter | Quota | Applied to |
|---|---|---|
| `limiters.write` | 10 req/min/IP | login, register, publish |
| `limiters.api` | 120 req/min/IP | declared but not yet wired to read endpoints |
| `limiters.login` | 5 failures/10 min → 15 min lockout | per username, in-memory |

Login lockout only records failures when the username exists in the DB (prevents DoS lockout of arbitrary usernames).

## Development conventions

- New endpoints: one file per handler group in `src/api/`, registered in `api/mod.rs::router()`
- DB schema changes: use `add_column_if_missing()` helper — never drop/recreate; deployed databases upgrade transparently on restart
- `db.audit()` is fire-and-forget (`tokio::spawn`); never await it on the request path
- `ApiError` in `api/mod.rs` is the standard error type; its `From<anyhow::Error>` impl logs at error level and returns 500
- TOTP `totp-rs` requires both `gen_secret` and `otpauth` features (the `otpauth` feature enables issuer/account fields and `get_url()`)

## Related documentation

| File | Contents |
|---|---|
| `docs/api.md` | Complete HTTP API reference — all endpoints, request/response shapes, status codes |
| `docs/architecture.md` | Deeper design notes, request lifecycle diagrams, key decisions |
| `TODO.md` | Roadmap — open items and completed features |

## Maintenance rules

**Always update `TODO.md`** when implementing a feature: mark the item `[x]` and expand the description to reflect what was actually built. Do this in the same commit as the implementation.

**Keep `docs/architecture.md` in sync** when the DB schema or request lifecycle changes significantly.
