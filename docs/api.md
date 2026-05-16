# freight-registry — HTTP API Reference

## Health check

### `GET /health`

No authentication required. Returns `200` when the server is healthy, `503` when the database is unreachable.

**Response**
```json
{ "status": "ok", "db": "ok", "time": 1700000000 }
```

---

## Metrics

### `GET /metrics`

Returns a Prometheus-compatible text exposition. No authentication required.

Exposed metrics include package/version/user/token counts (gauges) and publish, download, and login outcome counters.

---

Base path: `/api/v1`

All responses are JSON unless the endpoint streams a binary file. Errors always have the shape:

```json
{ "errors": [{ "detail": "human-readable message" }] }
```

---

## Channels

Most package endpoints accept an optional `?channel=<name>` query parameter. Channels allow a single server to host multiple independent release streams (e.g. `stable`, `experimental`). The default channel is `stable`.

---

## Authentication

Write endpoints and `/api/v1/me` require a valid API token:

```
Authorization: Bearer <token>
```

Tokens are issued via `POST /api/v1/users/register`, `POST /api/v1/users/login`, or `POST /api/v1/me/tokens`.

### Token scopes

| Scope | Permissions |
|---|---|
| `read` | Read-only: GET endpoints only |
| `publish` | Read + publish, yank, owner management |
| `admin` | Full access including admin endpoints (requires `is_admin = true` on the account) |

---

## Read endpoints (no auth)

### `GET /api/v1/packages/:name[?channel=stable]`

Returns metadata and all version records for a package.

**Response 200**
```json
{
  "name":    "mylib",
  "description": "A useful library",
  "latest":  "1.0.1",
  "versions": [
    {
      "version": "1.0.1",
      "checksum": "def456...",
      "yanked": false,
      "downloads": 42,
      "download_url": "…",
      "prebuilt_triples": ["x86_64-linux-gnu", "aarch64-linux-gnu"]
    },
    {
      "version": "1.0.0",
      "checksum": "abc123...",
      "yanked": false,
      "downloads": 17,
      "download_url": "…",
      "prebuilt_triples": []
    }
  ]
}
```

When a mirror upstream is configured and the package is not found locally, the response is proxied from the upstream registry.

**404** — package not found in the local registry and upstream (if configured).

---

### `GET /api/v1/packages/:name/:version/download[?channel=stable]`

Streams the source tarball for the requested version. The SHA-256 checksum is re-verified against the stored value before streaming; a mismatch returns 500.

**Response headers**
```
Content-Type: application/gzip
X-Checksum-SHA256: <hex>
```

When a mirror upstream is configured and the version is not found locally, the tarball is proxied from the upstream registry.

**404** — package or version not found.  
**410 Gone** — version is yanked.

---

### `GET /api/v1/search?q=<query>[&limit=<n>][&offset=<n>][&channel=stable]`

Searches package names (case-insensitive substring). Default `limit` is 20, max 100. `offset` defaults to 0.

When a mirror upstream is configured, upstream results are appended for packages not present locally.

**Response 200**
```json
{
  "packages": [
    { "name": "mylib", "description": "A useful library", "latest": "1.0.1", "downloads": 42 }
  ],
  "total":  1,
  "limit":  20,
  "offset": 0
}
```

---

### `GET /api/v1/packages/:name/:version/prebuilts[?channel=stable]`

Lists available prebuilt binary tarballs for the requested version.

**Response 200**
```json
{
  "name": "mylib",
  "version": "1.0.1",
  "channel": "stable",
  "prebuilts": [
    { "triple": "x86_64-linux-gnu",   "checksum": "abc123…" },
    { "triple": "aarch64-linux-gnu",  "checksum": "def456…" }
  ]
}
```

**404** — package or version not found.

---

### `GET /api/v1/packages/:name/:version/prebuilt/:triple/download[?channel=stable]`

Streams the prebuilt tarball for the given target triple. The checksum is verified before streaming.

**Response headers**
```
Content-Type: application/gzip
Content-Disposition: attachment; filename="mylib-1.0.1-x86_64-linux-gnu.tar.gz"
X-Checksum-SHA256: <hex>
```

**404** — no prebuilt for this triple.

---

### `GET /api/v1/packages/:name/owners`

Lists all owners of a package. No auth required.

**Response 200**
```json
{
  "users": [
    { "login": "alice", "id": 1 },
    { "login": "bob",   "id": 2 }
  ]
}
```

**404** — package not found.

---

### `GET /api/v1/orgs/:name`

Returns org metadata and its member list. No auth required.

**Response 200**
```json
{
  "id": 1,
  "name": "acme",
  "description": "Acme Corp packages",
  "members": [
    { "username": "alice", "role": "owner" },
    { "username": "bob",   "role": "member" }
  ]
}
```

**404** — org not found.

---

### `GET /api/v1/orgs/:name/members`

Lists members of an org. No auth required.

**Response 200**
```json
{ "members": [{ "username": "alice", "role": "owner" }] }
```

---

## Auth-required endpoints

### `GET /api/v1/me`

Returns the authenticated user's identity.

**Response 200**
```json
{ "login": "alice", "id": 42 }
```

---

### `GET /api/v1/me/tokens`

Lists all API tokens for the authenticated user.

**Response 200**
```json
{
  "tokens": [
    {
      "id": 1,
      "name": "laptop",
      "kind": "api",
      "scope": "publish",
      "expires_at": 1720000000,
      "last_used": 1719000000
    }
  ]
}
```

---

### `POST /api/v1/me/tokens`

Creates a new API token. Requires a token with `publish` scope or higher.

**Request body**
```json
{
  "name": "ci-token",
  "expires_days": 30,
  "scope": "publish"
}
```

`expires_days` and `scope` are optional. `scope` defaults to `"publish"`. Valid values: `"read"`, `"publish"`, `"admin"`.

**Response 200**
```json
{ "token": "a3f7c2…", "name": "ci-token", "scope": "publish" }
```

The token value is shown once and never retrievable again.

**409** — a token with that name already exists.

---

### `DELETE /api/v1/me/tokens/:name`

Revokes a token by name. Requires a token with `publish` scope or higher.

**Response 200** — `{ "ok": true }`  
**404** — token not found.

---

### `PUT /api/v1/packages` — Publish

Upload a new package version. The request body uses the cargo binary wire format:

```
[u32 LE: JSON metadata length]
[JSON metadata bytes]
[u32 LE: tarball length]
[tarball bytes]
```

**JSON metadata fields**

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Package name (1–64 chars, `[a-zA-Z0-9_-]`) |
| `vers` | string | yes | Semver version string |
| `description` | string | no | Short description |
| `license` | string | no | SPDX license identifier |
| `channel` | string | no | Channel to publish to (default: `stable`) |

**Response 200**
```json
{
  "ok": true,
  "warnings": { "invalid_categories": [], "invalid_badges": [], "other": [] }
}
```

**400** — invalid name, version, or malformed body.  
**403** — authenticated user is not an owner of an existing package.  
**409** — version already exists in that channel.  
**429** — write rate limit exceeded.

---

### `PUT /api/v1/packages/:name/:version/prebuilt/:triple[?channel=stable]`

Upload a prebuilt binary tarball for a target triple. The source version must already be published. Requires package ownership.

**Body** — raw `.tar.gz` bytes.

**Response 200**
```json
{ "ok": true, "triple": "x86_64-linux-gnu", "checksum": "abc123…" }
```

**400** — body is not a valid gzip archive, or triple is invalid.  
**403** — not a package owner.  
**404** — source version not published yet.

---

### `DELETE /api/v1/packages/:name/:version/yank[?channel=stable]`

Marks a version as yanked. Yanked versions are excluded from `freight add` resolution but remain downloadable for locked projects.

**Response 200** — `{ "ok": true }`  
**403** — not an owner.  
**404** — package or version not found.

---

### `PUT /api/v1/packages/:name/:version/yank[?channel=stable]`

Removes yank status (unyank).

**Response 200** — `{ "ok": true }`  
**403** — not an owner.  
**404** — package or version not found.

---

### `PUT /api/v1/packages/:name/owners`

Adds one or more owners. Caller must already be an owner.

**Request body**
```json
{ "users": ["bob", "carol"] }
```

**Response 200**
```json
{ "ok": true, "msg": "added 2 owner(s)" }
```

If some usernames are not found:
```json
{ "ok": true, "msg": "added alice; not found: unknown-user" }
```

**403** — caller is not an owner.

---

### `DELETE /api/v1/packages/:name/owners`

Removes one or more owners. Caller must already be an owner.

**Request body**
```json
{ "users": ["bob"] }
```

**Response 200**
```json
{ "ok": true, "msg": "removed 1 owner(s)" }
```

**400** — request would remove the last owner.  
**403** — caller is not an owner.

---

## Organizations

### `POST /api/v1/orgs`

Creates a new organization. The caller becomes the first owner.

**Request body**
```json
{ "name": "acme", "description": "Acme Corp packages" }
```

`description` is optional. `name` follows the same rules as package names.

**Response 200**
```json
{ "id": 1, "name": "acme" }
```

**409** — org name already taken.

---

### `DELETE /api/v1/orgs/:name`

Deletes an org. Only org owners (or admins) can delete it.

**Response 200** — `{ "deleted": true }`  
**403** — not an org owner.  
**404** — org not found.

---

### `PUT /api/v1/orgs/:name/members`

Adds a member to an org. Only org owners (or admins) can call this.

**Request body**
```json
{ "username": "bob", "role": "member" }
```

`role` must be `"owner"` or `"member"` (default `"member"`).

**Response 200**
```json
{ "added": "bob", "role": "member" }
```

**403** — not an org owner.  
**404** — org or user not found.

---

### `DELETE /api/v1/orgs/:name/members/:username`

Removes a member from an org. Org owners may remove anyone; members may remove themselves.

**Response 200** — `{ "removed": "bob" }`  
**403** — insufficient permission.  
**404** — member not found in org.

---

### `PUT /api/v1/packages/:name/:channel/org`

Assigns (or clears) org ownership for a package. The caller must own the package; if setting an org, the caller must also be a member of that org.

**Request body**
```json
{ "org": "acme" }
```

Pass `null` (or omit `org`) to clear org ownership.

**Response 200** — `{ "org": "acme" }`  
**403** — not a package owner or not an org member.  
**404** — package or org not found.

---

## Registration and login

### `POST /api/v1/users/register`

Creates a new user account and returns an initial API token. Rate-limited (write limiter).

**Request body**
```json
{
  "username":   "alice",
  "password":   "<sha256-hex>",
  "email":      "alice@example.com",
  "token_name": "laptop"
}
```

`email` and `token_name` are optional. `token_name` defaults to `init`.

**Response 201**
```json
{
  "ok":           true,
  "login":        "alice",
  "id":           1,
  "token":        "a3f7c2…",
  "expires_days": 90
}
```

The token is shown once and never retrievable again.

**400** — invalid username or password.  
**409** — username already taken.  
**429** — rate limit exceeded.

#### Password encoding

The `password` field must be the **lowercase hex SHA-256 digest** of the plaintext password, not the plaintext itself.

```sh
echo -n 'mysecretpassword' | sha256sum | cut -d' ' -f1
# → 5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8
```

| Field | Constraints |
|---|---|
| `username` | 2–32 chars, `[a-zA-Z0-9_-]`, must start with a letter |
| `password` (plaintext before hashing) | min 8 chars |

---

### `POST /api/v1/users/login`

Verifies credentials and returns an access token + refresh token. Rate-limited (write limiter).

**Request body**
```json
{
  "username":    "alice",
  "password":    "<sha256-hex>",
  "token_name":  "laptop-2026",
  "expires_days": 30,
  "totp_code":   "123456"
}
```

`token_name` defaults to `login-<unix-timestamp>`. `expires_days` defaults to `90`; clamped to `[1, 365]`.  
`totp_code` is **required** when the account has TOTP enabled.

**Response 200**
```json
{
  "token":         "a3f7c2…",
  "refresh_token": "b8e91f…",
  "expires_days":   30
}
```

Both tokens are shown once. The refresh token can be exchanged at `POST /api/v1/auth/refresh` and is valid for 30 days.

**400** — TOTP code required but missing, or TOTP code invalid.  
**401** — unknown username or wrong password.  
**429** — rate limit exceeded or account locked out.

---

### `POST /api/v1/auth/refresh`

Exchange a valid refresh token for a new 90-day access token.

**Headers**
```
Authorization: Bearer <refresh_token>
```

**Response 200**
```json
{ "token": "c2d4e6…", "expires_days": 90 }
```

**401** — token missing, expired, or not a refresh token.

---

## Email verification

When a user registers with an email address the server logs a verification link to stdout:

```
WARN EMAIL VERIFICATION LINK (expires in 24 h): https://registry.example.com/api/v1/users/verify-email?token=…
```

### `GET /api/v1/users/verify-email?token=<token>`

**Response 200** — `{ "ok": true, "message": "email verified" }`  
**400** — invalid or expired token.

---

## Password reset

Reset links are logged to the server console (no SMTP required).

### `POST /api/v1/users/reset-password/request`

**Request body** — `{ "username": "alice" }`

Always returns 200 (prevents user enumeration). A link is logged to the console if the account exists:

```
WARN PASSWORD RESET LINK (expires in 1 h): https://registry.example.com/api/v1/users/reset-password/confirm?token=…
```

**Response 200** — `{ "ok": true, "message": "if that account exists, a reset link has been logged to the server console" }`

---

### `POST /api/v1/users/reset-password/confirm`

**Request body**
```json
{
  "token":        "<reset-token>",
  "new_password": "<sha256-hex-of-new-plaintext-password>"
}
```

**Response 200** — `{ "ok": true, "message": "password updated" }`  
**400** — invalid or expired token.

---

## TOTP / 2FA

All TOTP endpoints require a valid API token.

### `POST /api/v1/me/totp/enroll`

Generates a TOTP secret (not yet active). Returns the base32 secret and an `otpauth://` URI for authenticator apps.

**Response 200**
```json
{
  "secret": "JBSWY3DPEHPK3PXP",
  "uri":    "otpauth://totp/freight-registry:alice?secret=JBSWY3DPEHPK3PXP&issuer=freight-registry"
}
```

---

### `POST /api/v1/me/totp/confirm`

Activates TOTP by verifying the first code from the authenticator.

**Request body** — `{ "code": "123456" }`

**Response 200** — `{ "ok": true }`  
**400** — not enrolled or invalid code.

---

### `DELETE /api/v1/me/totp`

Disables TOTP after verifying the current code.

**Request body** — `{ "code": "123456" }`

**Response 200** — `{ "ok": true }`  
**400** — TOTP not enabled or invalid code.

---

## Admin endpoints

All admin endpoints require a token whose owner has `is_admin = true`. Returns `403` otherwise.

### `GET /api/v1/admin/users`

Lists all registered user accounts.

**Response 200**
```json
{
  "users": [
    { "id": 1, "username": "alice", "email": "alice@example.com", "is_admin": true },
    { "id": 2, "username": "bob",   "email": null,                 "is_admin": false }
  ]
}
```

---

### `POST /api/v1/admin/users/:name/promote`

Grants the admin role to a user.

**Response 200** — `{ "ok": true }`  
**404** — user not found.

---

### `POST /api/v1/admin/users/:name/demote`

Revokes the admin role from a user.

**Response 200** — `{ "ok": true }`  
**404** — user not found.

---

### `DELETE /api/v1/admin/users/:name`

Removes a user account and all their tokens.

**Response 200** — `{ "ok": true }`  
**404** — user not found.

---

### `DELETE /api/v1/admin/packages/:name`

Hard-deletes a package and all its versions (DB rows + tarball files). **Irreversible** — use `yank` to hide without destroying.

**Response 200** — `{ "ok": true }`  
**404** — package not found.

---

### `GET /api/v1/audit`

Returns audit log entries (admin only).

| Param | Description |
|---|---|
| `user` | Filter by username |
| `action` | Filter by action (`login`, `publish`, `yank`, `unyank`, `register`, `publish_prebuilt`) |
| `since` | Unix timestamp lower bound |
| `until` | Unix timestamp upper bound |
| `limit` | Max rows (default 100, max 500) |

**Response 200**
```json
{
  "entries": [
    {
      "id": 1, "user_id": 1, "username": "alice",
      "action": "publish", "package": "mylib", "version": "1.0.0",
      "ip_addr": "127.0.0.1", "created_at": 1700000000
    }
  ],
  "count": 1
}
```

---

## Login lockout

After **5 consecutive failed login attempts** for the same username within a 10-minute window, that username is locked for **15 minutes**. The lockout state is in-memory and resets on server restart. Successful login clears the counter immediately.

Locked accounts return `429 Too Many Requests` on login attempts.

---

## HTTP status code summary

| Code | Meaning |
|---|---|
| 200 | Success |
| 201 | Created (register) |
| 400 | Bad request — invalid input |
| 401 | Missing or invalid token |
| 403 | Token valid but insufficient permission |
| 404 | Resource not found |
| 409 | Conflict — e.g. version already exists |
| 410 | Gone — yanked version |
| 429 | Too many requests — rate limit or lockout |
| 500 | Internal server error |
| 503 | Service unavailable — database unreachable |
