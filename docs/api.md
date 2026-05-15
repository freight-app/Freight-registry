# freight-registry — HTTP API Reference

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
  "name": "mylib",
  "description": "A useful library",
  "versions": [
    { "version": "1.0.1", "checksum": "def456...", "yanked": false },
    { "version": "1.0.0", "checksum": "abc123...", "yanked": false }
  ]
}
```

**404** — package not found.

---

### `GET /api/v1/packages/:name/:version/download`

Streams the source tarball for the requested version.

**Response headers**
```
Content-Type: application/octet-stream
X-Checksum-SHA256: <hex>
```

**404** — package or version not found.  
**410 Gone** — version is yanked.

---

### `GET /api/v1/search?q=<query>[&limit=<n>]`

Searches package names and descriptions (case-insensitive substring). Default `limit` is 20.

**Response 200**
```json
{
  "results": [
    { "name": "mylib", "description": "A useful library", "latest": "1.0.1" }
  ],
  "total": 1
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

#### Username / password rules

| Field | Constraints |
|---|---|
| `username` | 1–64 chars, `[a-zA-Z0-9_-]` |
| `password` | min 8 chars |

---

### `POST /api/v1/users/login`

Verifies username + password and returns a new API token. Rate-limited (write limiter, 10 req/min/IP).

**Request body**
```json
{
  "username": "alice",
  "password": "hunter2",
  "token_name": "laptop-2026",
  "expires_days": 30
}
```

`token_name` defaults to `login-<unix-timestamp>`. `expires_days` defaults to `90`; clamped to `[1, 365]`.

**Response 200**
```json
{
  "token": "a3f7c2…",
  "expires_days": 30
}
```

The token is shown once and never retrievable again.

**401** — unknown username or wrong password (intentionally ambiguous).  
**429** — rate limit exceeded.

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
