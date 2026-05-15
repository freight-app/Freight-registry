# freight-registry — HTTP API Reference

## Health check

### `GET /health`

No authentication required. Returns `200` when the server is healthy, `503` when the database is unreachable.

**Response**
```json
{ "status": "ok", "db": "ok", "time": 1700000000 }
```

---

Base path: `/api/v1`

All responses are JSON unless the endpoint streams a binary file. Errors always have the shape:

```json
{ "errors": [{ "detail": "human-readable message" }] }
```

---

## Authentication

Write endpoints and `/api/v1/me` require a valid API token:

```
Authorization: Bearer <token>
```

Tokens are issued via `freight-registry token add`, `POST /api/v1/users/register`, or `POST /api/v1/users/login`.

---

## Read endpoints (no auth)

### `GET /api/v1/packages/:name`

Returns metadata and all version records for a package.

**Response 200**
```json
{
  "name":    "mylib",
  "description": "A useful library",
  "latest":  "1.0.1",
  "versions": [
    { "version": "1.0.1", "checksum": "def456...", "yanked": false, "downloads": 42, "download_url": "…" },
    { "version": "1.0.0", "checksum": "abc123...", "yanked": false, "downloads": 17, "download_url": "…" }
  ]
}
```

**404** — package not found.

---

### `GET /api/v1/packages/:name/:version/download`

Streams the source tarball for the requested version. The SHA-256 checksum is re-verified against the stored value before the response is sent; a mismatch returns 500.

**Response headers**
```
Content-Type: application/gzip
X-Checksum-SHA256: <hex>
```

**404** — package or version not found.  
**410 Gone** — version is yanked.

---

### `GET /api/v1/search?q=<query>[&limit=<n>][&offset=<n>]`

Searches package names (case-insensitive substring). Default `limit` is 20, max 100. `offset` defaults to 0.

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

## Auth-required endpoints

### `GET /api/v1/me`

Returns the authenticated user's identity.

**Response 200**
```json
{ "login": "alice", "id": 42 }
```

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
| `license` | string | no | SPDX license identifier (stored but not enforced) |

**Response 200**
```json
{
  "ok": true,
  "warnings": { "invalid_categories": [], "invalid_badges": [], "other": [] }
}
```

**400** — invalid name, version, or malformed body.  
**403** — authenticated user is not an owner of an existing package.  
**409** — version already exists.  
**429** — write rate limit exceeded (10 req/min/IP).

---

### `DELETE /api/v1/packages/:name/:version/yank`

Marks a version as yanked. Yanked versions are excluded from `freight add` resolution but remain downloadable for locked projects.

**Response 200**
```json
{ "ok": true }
```

**403** — not an owner.  
**404** — package or version not found.

---

### `PUT /api/v1/packages/:name/:version/yank`

Removes yank status (unyank).

**Response 200**
```json
{ "ok": true }
```

**403** — not an owner.  
**404** — package or version not found.

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

## Registration and login

### `POST /api/v1/users/register`

Creates a new user account and returns an initial API token. Rate-limited (write limiter, 10 req/min/IP).

**Request body**
```json
{
  "username":   "alice",
  "password":   "hunter2",
  "email":      "alice@example.com",
  "token_name": "laptop"
}
```

`email` and `token_name` are optional. `token_name` defaults to `init`.

**Response 201**
```json
{
  "ok":          true,
  "login":       "alice",
  "id":          1,
  "token":       "a3f7c2…",
  "expires_days": 90
}
```

The token is shown once and never retrievable again. It expires after 90 days by default.

**400** — invalid username or password (see validation rules below).  
**409** — username already taken.  
**429** — rate limit exceeded.

#### Password encoding

The `password` field must be the **lowercase hex SHA-256 digest** of the plaintext password, not the plaintext itself. The client hashes before sending so the plaintext never leaves the machine.

```sh
# curl example
echo -n 'mysecretpassword' | sha256sum | cut -d' ' -f1
# → 5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8
```

Validation (enforced client-side before hashing):

| Field | Constraints |
|---|---|
| `username` | 2–32 chars, `[a-zA-Z0-9_-]`, must start with a letter |
| `password` (plaintext) | min 8 chars |

---

### `POST /api/v1/users/login`

Verifies username + password and returns an access token + a refresh token. Rate-limited (write limiter, 10 req/min/IP).

**Request body**
```json
{
  "username":    "alice",
  "password":    "hunter2",
  "token_name":  "laptop-2026",
  "expires_days": 30,
  "totp_code":   "123456"
}
```

`token_name` defaults to `login-<unix-timestamp>`. `expires_days` defaults to `90`; clamped to `[1, 365]`.  
`totp_code` is **required** when the account has TOTP enabled.

The `password` field must be the SHA-256 hex digest of the plaintext password (see registration above).

**Response 200**
```json
{
  "token":         "a3f7c2…",
  "refresh_token": "b8e91f…",
  "expires_days":   30
}
```

`token` is an access token (kind `access`). `refresh_token` is a 30-day refresh token (kind `refresh`) that can be exchanged for a fresh access token at `POST /api/v1/auth/refresh`. Both are shown once and never retrievable again.

**400** — TOTP code required but missing, or TOTP code invalid.  
**401** — unknown username or wrong password (intentionally ambiguous).  
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
{
  "token":       "c2d4e6…",
  "expires_days": 90
}
```

**401** — token missing, expired, or not a refresh token.

---

## Email verification

When a user registers with an email address the server logs an email verification link to stdout:

```
WARN EMAIL VERIFICATION LINK (expires in 24 h): https://registry.example.com/api/v1/users/verify-email?token=…
```

### `GET /api/v1/users/verify-email?token=<token>`

Marks the user's email as verified.

**Response 200**
```json
{ "ok": true, "message": "email verified" }
```

**400** — invalid or expired token.

---

## Password reset

No SMTP dependency — reset links are logged to the server console.

### `POST /api/v1/users/reset-password/request`

**Request body**
```json
{ "username": "alice" }
```

Always returns 200 regardless of whether the username exists (prevents user enumeration). If the account exists, a link is logged:

```
WARN PASSWORD RESET LINK (expires in 1 h): https://registry.example.com/api/v1/users/reset-password/confirm?token=…
```

**Response 200**
```json
{ "ok": true, "message": "if that account exists, a reset link has been logged to the server console" }
```

---

### `POST /api/v1/users/reset-password/confirm`

**Request body**
```json
{
  "token":        "<reset-token>",
  "new_password": "<sha256-hex-of-new-plaintext-password>"
}
```

`new_password` uses the same SHA-256 pre-hash encoding as registration.

**Response 200**
```json
{ "ok": true, "message": "password updated" }
```

**400** — invalid or expired token.

---

## TOTP / 2FA

All TOTP endpoints require a valid API token (`Authorization: Bearer <token>`).

### `POST /api/v1/me/totp/enroll`

Generates a new TOTP secret and stores it (not yet active). Returns the base32 secret and an `otpauth://` provisioning URI for scanning with an authenticator app.

**Response 200**
```json
{
  "secret": "JBSWY3DPEHPK3PXP",
  "uri":    "otpauth://totp/freight-registry:alice?secret=JBSWY3DPEHPK3PXP&issuer=freight-registry"
}
```

---

### `POST /api/v1/me/totp/confirm`

Verify the first code from the authenticator to activate TOTP.

**Request body**
```json
{ "code": "123456" }
```

**Response 200** — `{ "ok": true }`  
**400** — TOTP not enrolled or invalid code.

---

### `DELETE /api/v1/me/totp`

Disable TOTP after verifying the current code.

**Request body**
```json
{ "code": "123456" }
```

**Response 200** — `{ "ok": true }`  
**400** — TOTP not enabled or invalid code.

---

## Admin endpoints

All admin endpoints require a valid token whose owner has `is_admin = true`. Returns `403` otherwise.

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

### `DELETE /api/v1/admin/packages/:name`

Hard-deletes a package and all its versions. Removes the DB row (cascades to versions and owners) and the tarball directory from storage. **Irreversible** — use `yank` to hide versions without destroying them.

**Response 200** — `{ "ok": true }`  
**404** — package not found.

---

### `GET /api/v1/audit`

Returns audit log entries. Supports query filters:

| Param | Description |
|---|---|
| `user` | Filter by username |
| `action` | Filter by action (`login`, `publish`, `yank`, `unyank`, `register`) |
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
| 429 | Too many requests — rate limit |
| 500 | Internal server error |
