# ── Build stage ───────────────────────────────────────────────────────────────
FROM rust:1-slim AS builder

WORKDIR /build

# Cache dependencies separately from application code.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
 && cargo build --release \
 && rm -rf src

COPY src ./src
# Touch main.rs so Cargo rebuilds the binary (not just the deps).
RUN touch src/main.rs && cargo build --release

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
