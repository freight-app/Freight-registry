# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo build                  # compile (dev)
cargo build --release        # release build
cargo check                  # type-check without linking (faster)
cargo clippy                 # lint
cargo test                   # run tests
```

No test suite exists yet — correctness is verified by running the server and exercising endpoints manually.

## Architecture

Single Rust binary (`src/main.rs`) with two modes: CLI admin commands and an Axum HTTP server.

**Module map**

| File | Responsibility |
|---|---|
| `main.rs` | CLI (clap): `serve` / `user` / `token` subcommands; `AppState` init; audit pruning task |
| `db.rs` | `Db` handle — all SQL; row types (`UserRow`, `TokenRow`, …); migrations |
| `auth.rs` | `AuthToken`, `AdminToken`, `RefreshTokenAuth` extractors; `hash_password` (Argon2id) |
| `totp.rs` | TOTP helpers: generate secret (base32), build `TOTP` instance, verify code |
| `rate_limit.rs` | `Limiters` — two `governor` keyed rate limiters + per-username `LoginLimiter` |
| `storage.rs` | Tarball read/write under `<data>/tarballs/<name>/<version>.tar.gz` |
| `validate.rs` | Input validation: package names, semver, usernames, passwords |
| `api/mod.rs` | `router()`, `ApiError` type, CORS layer, security-headers middleware |
| `api/*.rs` | One file per endpoint group (see `docs/api.md` for the full HTTP reference) |

**DB schema** — see `docs/architecture.md` for the full schema. Key points:
- Passwords stored as Argon2id PHC; tokens stored as SHA-256(raw) hex
- Tokens have a `kind` column: `api` (CLI-issued), `access` (login session), `refresh`
- Users have `is_admin`, `email_verified`, `totp_secret`, `totp_enabled`
- `email_tokens` table for verify/reset tokens (one pending per user per kind)
- All schema changes are additive `ALTER TABLE` migrations in `db.rs::migrate()`

**Auth flow**: `AuthToken` extractor hashes the bearer token, looks it up in `tokens`, rejects expired or `kind=refresh` tokens, then fetches the associated `UserRow`.

**Password wire protocol**: clients send `SHA-256(plaintext)` hex; server wraps that with Argon2id. Plaintext never transmitted.

## Development conventions

- New endpoints go in their own file under `src/api/`, registered in `api/mod.rs::router()`
- DB schema changes: always additive `ALTER TABLE` via `add_column_if_missing()`, never drop/recreate — existing deployed databases must upgrade transparently
- `db.audit()` is fire-and-forget (`tokio::spawn`); never block a request on audit writes
- `ApiError` in `api/mod.rs` is the standard error type; add new constructors there if needed

## Maintenance rule

**Always update `TODO.md`** when implementing a feature: mark the item `[x]` and expand the description to reflect what was actually built. Do this in the same commit as the implementation.
