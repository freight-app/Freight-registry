# freight-registry — TODO

Size labels: **S** = small (hours), **B** = big (days / multiple files).

## Core / Protocol

- [ ] **C1 · B** **Token scopes** — read-only vs. publish vs. admin scopes; enforce per-endpoint
- [ ] **C2 · B** **Scoped packages** — `@org/name` namespace support (affects validation, ownership, routes)
- [x] **C3 · S** **Bulk search pagination** — cursor- or page-based results for `/api/v1/search`
- [x] **C4 · S** **Package deletion** — hard-delete (tarball + DB row) for admins only; separate from yank
- [x] **C5 · S** **Checksum verification on download** — re-check SHA-256 against stored value before streaming

## Auth & Security

- [x] **A1 · S** **Email verification** — token logged to stdout on register; `GET /api/v1/users/verify-email?token=<t>`
- [x] **A2 · S** **Password reset flow** — reset link logged to stdout; `POST /api/v1/users/reset-password/request` + `/confirm`
- [x] **A3 · B** **TOTP / 2FA** — `totp_secret`/`totp_enabled` on users; enroll/confirm/disable endpoints; login checks code when enabled
- [x] **A4 · S** **CORS & security headers** — permissive `CorsLayer` + `X-Content-Type-Options`, `X-Frame-Options`, `Referrer-Policy`
- [x] **A5 · S** **Refresh tokens** — `kind` column on tokens; login returns `access` + `refresh` tokens; `POST /api/v1/auth/refresh`
- [ ] **A6 · S** **CSP header** — Content-Security-Policy for the web UI (once HTML pages exist)
- [x] **A7 · S** **Admin role** — `is_admin` column on users; `AdminToken` extractor; `GET /api/v1/admin/users`; `user promote/demote` CLI
- [x] **A8 · S** **Audit log pruning** — `--audit-log-ttl-days` flag; background task deletes old entries every 24 h
- [x] **A9 · S** **Login attempt lockout** — 5 failures / 10-min window → 15-min lockout; per-username in-memory; cleared on success

## Storage & Database

- [x] **D1 · B** **S3-compatible backend** — `Storage` enum dispatches between local `PathBuf` and `object_store` S3/MinIO; `--s3-bucket/endpoint/key-id/secret/region` CLI flags
- [ ] **D2 · B** **PostgreSQL support** — compile-time feature flag; share schema via sqlx migrations folder
- [x] **D3 · B** **Proper migrations** — versioned `.sql` files in `migrations/`; `sqlx::migrate!()` runs on startup; backward-compat via `CREATE TABLE IF NOT EXISTS`
- [x] **D4 · S** **Tarball integrity check on publish** — verify the uploaded file is a valid gzip/tar archive

## Observability

- [x] **O1 · S** **Health endpoint** — `GET /health` returning DB reachability and uptime
- [x] **O2 · B** **Metrics endpoint** — Prometheus-compatible `/metrics` via `prometheus-client 0.22`; gauges for package/version/user/token counts; counters for publishes, downloads served, login outcomes
- [x] **O3 · S** **Structured audit log API** — `GET /api/v1/audit` (admin only) with filters

## Terminal UI (`freight-registry-tui`)

- [x] **T1 · B** **Package browser** — searchable list, per-version downloads, yank/unyank actions
- [x] **T2 · B** **Publish form** — name/version/path form; reads `.tar.gz` and sends wire-format PUT
- [x] **T3 · B** **User management** — admin promote/demote/remove via HTTP (`/api/v1/admin/users/:name/*`)
- [x] **T4 · B** **Token management** — list/create/revoke via `/api/v1/me/tokens`
- [x] **T5 · B** **Audit log viewer** — scrollable table with `user:` / action filter (admin only)
- [x] **T6 · S** **Login persistence** — save token to `~/.config/freight-registry/tui.toml` after login
- [x] **T7 · S** **Download metrics graph** — sparkline of download counts per version in detail pane

## Web UI

- [ ] **U1 · B** **Package index page** — browsable HTML listing of packages and versions
- [ ] **U2 · B** **Package detail page** — README rendering, version history, owner list, yank status
- [ ] **U3 · B** **User profile / token management UI** — browser-based token creation and revocation
- [ ] **U4 · B** **Search UI** — front-end over the existing search API

## Operations

- [x] **P1 · S** **Dockerfile** — multi-stage build; non-root user; healthcheck instruction
- [x] **P2 · S** **Docker Compose example** — service + volume mounts for data dir
- [x] **P3 · S** **Systemd unit file** — for bare-metal installs
- [ ] **P4 · B** **Mirror / proxy mode** — transparent fallback to freight.dev for unknown packages
- [ ] **P5 · B** **Org / team accounts** — organization namespaces with member-based ownership delegation
- [x] **P6 · S** **Download count tracking** — per-version counter incremented on each download; exposed on `GET /api/v1/packages/:name`
- [x] **P7 · S** **Rate-limit config** — `--rate-limit-read` and `--rate-limit-write` CLI flags (req/min)
