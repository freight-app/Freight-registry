# freight-registry — TODO

## Core / Protocol

- [ ] **Token scopes** — read-only vs. publish vs. admin scopes; enforce per-endpoint
- [ ] **Scoped packages** — `@org/name` namespace support (affects validation, ownership, routes)
- [ ] **Bulk search pagination** — cursor- or page-based results for `/api/v1/search`
- [ ] **Package deletion** — hard-delete (tarball + DB row) for admins only; separate from yank
- [ ] **Checksum verification on download** — re-check SHA-256 against stored value before streaming

## Auth & Security

- [ ] **Email verification** — require verified email before first publish
- [ ] **Password reset flow** — token-based reset via email
- [ ] **TOTP / 2FA** — time-based one-time password as optional second factor on login
- [ ] **CORS & CSP headers** — configure allowed origins; Content-Security-Policy for the web UI
- [ ] **Refresh tokens** — short-lived access tokens with long-lived refresh tokens (OAuth2-style)
- [x] **Admin role** — `is_admin` column on users; `AdminToken` extractor; `GET /api/v1/admin/users`; `user promote/demote` CLI
- [ ] **Audit log pruning** — TTL or max-rows policy so the table doesn't grow unbounded
- [x] **Login attempt lockout** — 5 failures / 10-min window → 15-min lockout; per-username in-memory; cleared on success

## Storage & Database

- [ ] **S3-compatible backend** — abstract `Storage` trait so tarballs can live in S3/MinIO
- [ ] **PostgreSQL support** — compile-time feature flag; share schema via sqlx migrations folder
- [ ] **Proper migrations** — replace the startup `PRAGMA`-based schema migration with versioned `.sql` files (sqlx-cli `migrate run`)
- [ ] **Tarball integrity check on publish** — verify the uploaded file is a valid gzip/tar archive

## Observability

- [ ] **Health endpoint** — `GET /health` returning DB/storage reachability
- [ ] **Metrics endpoint** — Prometheus-compatible `/metrics` (download counts, publish rate, active tokens)
- [ ] **Structured audit log API** — `GET /api/v1/audit` (admin only) with filters

## Web UI

- [ ] **Package index page** — browsable HTML listing of packages and versions
- [ ] **Package detail page** — README rendering, version history, owner list, yank status
- [ ] **User profile / token management UI** — browser-based token creation and revocation
- [ ] **Search UI** — front-end over the existing search API

## Operations

- [ ] **Dockerfile** — multi-stage build; non-root user; healthcheck instruction
- [ ] **Docker Compose example** — service + volume mounts for data dir
- [ ] **Systemd unit file** — for bare-metal installs
- [ ] **Mirror / proxy mode** — transparent fallback to freight.dev for unknown packages
- [ ] **Org / team accounts** — organization namespaces with member-based ownership delegation
- [ ] **Download count tracking** — increment a counter per version on each download; expose via API
- [ ] **Rate-limit config** — make burst/refill rates configurable via CLI flags or config file
