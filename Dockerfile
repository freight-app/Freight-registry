# ── Build stage ───────────────────────────────────────────────────────────────
FROM rust:1-slim AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*

# Cache dependencies separately from application code.
# migrations/ must be present because sqlx::migrate! embeds SQL at compile time.
COPY Cargo.toml Cargo.lock ./
COPY migrations    ./migrations
COPY migrations_pg ./migrations_pg
# Stub all three targets (lib + two binaries) so `cargo build` can resolve them.
RUN mkdir -p src/tui \
 && echo '' > src/lib.rs \
 && echo 'fn main() {}' > src/main.rs \
 && echo 'fn main() {}' > src/tui/main.rs \
 && cargo build --release \
 && rm -rf src

COPY src ./src
# Touch entry points so Cargo knows to rebuild them.
RUN touch src/lib.rs src/main.rs src/tui/main.rs \
 && cargo build --release

# ── Runtime stage ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && adduser --disabled-password --gecos '' freight

COPY --from=builder /build/target/release/freight-registry /usr/local/bin/freight-registry

USER freight
WORKDIR /data
VOLUME ["/data"]
EXPOSE 7878

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD curl -fs http://localhost:7878/health || exit 1

ENTRYPOINT ["freight-registry"]
CMD ["--data", "/data", "serve"]
