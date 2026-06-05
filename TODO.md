# freight-registry — TODO

Size labels: **S** = small (hours), **B** = big (days / multiple files).

## Open

- [ ] **A6 · S** **CSP header** — Content-Security-Policy once any HTML pages are served directly
- [x] **E2 · S** **TOTP recovery codes** — Generated on TOTP confirm (8 codes, SHA-256 hashes stored); returned once in the confirm response; consumed atomically on login as an alternative to a live TOTP code.
- [x] **E3 · S** **Org role enforcement** — `delete_org`, `add_member`, `remove_member` already required owner; fixed `set_package_org` to require org owner instead of just org member.
- [ ] **E4 · S** **Org-scoped tokens** — Tokens are user-scoped. Add optional `org_id` binding so CI tokens can't publish outside their org.
- [x] **E5 · S** **Prebuilt blob GC** — `freight-registry gc` subcommand; dry-run by default, `--execute` to delete. Removes source tarballs, prebuilts, README, and docs for yanked versions. DB rows preserved.
- [ ] **T1 · B** **Integration test suite** — Full publish → download → yank flow against an in-memory SQLite DB (`Db::open(":memory:")`); TOTP enforcement; org role gating.
- [ ] **P8 · B** **Server-side prebuilt builds** — on source publish, queue Docker-based build jobs for each configured triple (`--build-triples x86_64-linux-gnu,aarch64-linux-gnu,x86_64-windows-gnu`); use **[Bollard](https://github.com/fussybeaver/bollard)** (Rust Docker API library) to drive the daemon directly — pull build image, create container with source bind-mount + output volume, stream logs, wait for exit; ClamAV scans output in a second container on the same volume before storing; results and live log streaming exposed via `GET /api/v1/packages/:name/:version/builds` and `GET /api/v1/packages/:name/:version/builds/:id/logs`; Linux/cross targets use `cross-rs` images; Windows targets use Windows Server Core Docker images; macOS cannot run in Docker legally so client-uploaded prebuilts remain the escape hatch for Apple targets

## Done

- [x] Token scopes (`read` / `publish` / `admin`)
- [x] Registry channels (`stable`, `experimental`, …)
- [x] Bulk search with pagination
- [x] Package deletion (admin hard-delete)
- [x] Checksum verification on download
- [x] Email verification
- [x] Password reset flow
- [x] Optional SMTP delivery — real email when `[serve.smtp]` / `--smtp-*` / `FREIGHT_SMTP_*` is configured; stdout link logging otherwise
- [x] TOTP / 2FA
- [x] CORS & security headers
- [x] Refresh tokens
- [x] Admin role + promote/demote/remove
- [x] Audit log pruning (TTL background task)
- [x] Login attempt lockout
- [x] S3-compatible storage backend
- [x] PostgreSQL support
- [x] Versioned SQL migrations
- [x] Tarball integrity check on publish
- [x] Health endpoint
- [x] Prometheus metrics endpoint
- [x] Structured audit log API
- [x] Package browser TUI — lives in the `freight` CLI (`freight add`); see `crates/freight/src/tui/TODO.md`
- [x] Dockerfile + Docker Compose
- [x] Systemd unit file
- [x] Mirror / proxy mode
- [x] Org / team accounts
- [x] Download count tracking
- [x] Rate-limit config flags
- [x] Dependencies stored and served per version
- [x] README stored on disk and served via `/api/v1/packages/:name/readme`
- [x] Prebuilt filter by arch/os/backend query params
- [x] Web documentation viewer (`docs.html`) — full symbol browser matching `freight doc` TUI grouping and palette; syntax highlighting via highlight.js; sidebar shows source link + owner chips; all docify tag types rendered (tparams, params, returns, retvals, throws, notes, warnings, examples, deprecated, since, see-also)
