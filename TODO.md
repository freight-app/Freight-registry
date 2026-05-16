# freight-registry — TODO

Size labels: **S** = small (hours), **B** = big (days / multiple files).

## Open

- [ ] **A6 · S** **CSP header** — Content-Security-Policy once any HTML pages are served directly
- [ ] **P8 · B** **Server-side prebuilt builds** — on source publish, queue Docker-based build jobs for each configured triple (`--build-triples x86_64-linux-gnu,aarch64-linux-gnu,x86_64-windows-gnu`); use **[Bollard](https://github.com/fussybeaver/bollard)** (Rust Docker API library) to drive the daemon directly — pull build image, create container with source bind-mount + output volume, stream logs, wait for exit; ClamAV scans output in a second container on the same volume before storing; results and live log streaming exposed via `GET /api/v1/packages/:name/:version/builds` and `GET /api/v1/packages/:name/:version/builds/:id/logs`; Linux/cross targets use `cross-rs` images; Windows targets use Windows Server Core Docker images; macOS cannot run in Docker legally so client-uploaded prebuilts remain the escape hatch for Apple targets

## Done

- [x] Token scopes (`read` / `publish` / `admin`)
- [x] Registry channels (`stable`, `experimental`, …)
- [x] Bulk search with pagination
- [x] Package deletion (admin hard-delete)
- [x] Checksum verification on download
- [x] Email verification
- [x] Password reset flow
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
