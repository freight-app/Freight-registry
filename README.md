# freight-registry

Self-hosted registry server for the [freight](https://github.com/TiniTinyTerminator/freight) package manager. Implements the same HTTP wire protocol as `freight.dev` so any `freight` client can point at it with a single config entry.

## Features

- **Package publish / download / search / yank** — full cargo-compatible wire protocol
- **User accounts** — Argon2id password hashing; create and manage users via the CLI
- **API tokens** — SHA-256 stored in DB; optional expiry; last-used tracking
- **Package ownership** — first publisher claims a package; multi-owner with transfer support
- **Rate limiting** — per-IP via `governor` (120 req/min read, 10 req/min write)
- **Audit log** — every login, publish, yank, and unyank recorded asynchronously
- **SQLite storage** — single-file database, WAL mode; no external DB required
- **Zero runtime deps** — reverse-proxy for TLS; everything else is self-contained

## Quick start

```sh
# Build
cargo build --release
# The binary is at target/release/freight-registry

# 1. Create the first user
freight-registry --data /var/lib/freight-registry user add alice

# 2. Issue an API token for that user
freight-registry --data /var/lib/freight-registry token add deploy --user alice

# 3. Start the server
freight-registry --data /var/lib/freight-registry serve \
    --base-url https://registry.example.com
```

The server listens on `0.0.0.0:7878` by default. Point a reverse proxy (nginx, Caddy) at it for TLS.

## Installation

**Prerequisites:** Rust stable toolchain.

```sh
git clone https://github.com/yourorg/freight-registry.git
cd freight-registry
cargo install --path .
```

## CLI reference

### `serve`

```
freight-registry [--data <dir>] serve [OPTIONS]

Options:
  --bind <addr>           Address and port to listen on  [env: FREIGHT_BIND]
                          [default: 0.0.0.0:7878]
  --base-url <url>        Publicly reachable base URL embedded in download links
                          [env: FREIGHT_BASE_URL] [default: http://localhost:7878]
  --max-upload-mb <n>     Maximum publish upload size in MB  [env: FREIGHT_MAX_UPLOAD_MB]
                          [default: 50]
```

### `user`

```
freight-registry [--data <dir>] user add <username> [--email <email>] [--password <pw>]
freight-registry [--data <dir>] user list
freight-registry [--data <dir>] user remove <username>
```

Password is prompted interactively when `--password` is omitted.

### `token`

```
freight-registry [--data <dir>] token add <name> --user <username> [--expires <days>]
freight-registry [--data <dir>] token list [--user <username>]
freight-registry [--data <dir>] token revoke <name> --user <username>
```

The raw token is printed once on `add` and never stored in plain text. Tokens expire after `--expires` days; omit for no expiry.

## Global options

| Flag | Env | Default | Description |
|---|---|---|---|
| `--data <dir>` | `FREIGHT_DATA_DIR` | `/var/lib/freight-registry` | Directory for `registry.db` and tarball storage |

## Connecting a freight client

Add a `[[registry]]` entry to `~/.freight/config.toml` (or your project's `.freight/config.toml`):

```toml
[[registry]]
name  = "myregistry"
url   = "https://registry.example.com"
token = "your-api-token-here"
```

Then use it in `freight.toml` or on the command line:

```sh
freight add mylib --repo myregistry
```

Registries are tried in declaration order when no `--repo` is given — add your private registry first to give it priority over `freight.dev`.

## Data layout

```
<data-dir>/
  registry.db          # SQLite database (users, tokens, packages, versions, owners, audit)
  tarballs/
    <name>/
      <version>.tar.gz # published source archives
```

## Logging

Set `RUST_LOG` to control log verbosity:

```sh
RUST_LOG=freight_registry=debug,tower_http=info freight-registry serve
```

Default level: `info` for both `freight_registry` and `tower_http`.

## Documentation

| Document | Contents |
|---|---|
| [docs/api.md](docs/api.md) | Complete HTTP API reference (all endpoints, request/response shapes) |
| [docs/architecture.md](docs/architecture.md) | Module layout, DB schema, request lifecycle, design notes |
| [TODO.md](TODO.md) | Roadmap — planned features and known gaps |
