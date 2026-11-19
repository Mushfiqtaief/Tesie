# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Stage 1 — Dashboard SPA
# Built on BUILDPLATFORM (native host arch) because the output is static files,
# architecture-independent, and building Bun under QEMU is painfully slow.
# ---------------------------------------------------------------------------
FROM --platform=$BUILDPLATFORM oven/bun:1-alpine AS web
WORKDIR /src/frontend
COPY frontend/package.json frontend/bun.lock ./
RUN --mount=type=cache,target=/root/.bun/install/cache \
    bun install --frozen-lockfile
COPY frontend/ ./
RUN bun run build

# ---------------------------------------------------------------------------
# Stage 2 — TesieCore (the mihomo-based proxy engine)
# Cross-compiled with Go's native cross-compiler, CGO disabled, so this also
# runs on BUILDPLATFORM rather than under emulation.
# ---------------------------------------------------------------------------
FROM --platform=$BUILDPLATFORM golang:1.24-alpine AS core
ARG TARGETOS
ARG TARGETARCH
WORKDIR /src/mihomo
COPY mihomo/go.mod mihomo/go.sum ./
RUN --mount=type=cache,target=/go/pkg/mod go mod download
COPY mihomo/ ./
RUN --mount=type=cache,target=/go/pkg/mod \
    --mount=type=cache,target=/root/.cache/go-build \
    CGO_ENABLED=0 GOOS=${TARGETOS:-linux} GOARCH=${TARGETARCH:-amd64} \
    go build -trimpath -ldflags="-s -w" -o /out/tesie-core .

# ---------------------------------------------------------------------------
# Stage 3 — Rust gateway
# Runs for the real TARGETPLATFORM. rustls is used for all TLS, so no OpenSSL
# headers are needed in the builder or the runtime.
# ---------------------------------------------------------------------------
FROM rust:1-slim-bookworm AS gateway
WORKDIR /src
COPY backend/ ./backend/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/backend/target \
    cargo build --release --locked --bin tesie --manifest-path backend/Cargo.toml \
 && mkdir -p /out \
 && cp backend/target/release/tesie /out/tesie

# ---------------------------------------------------------------------------
# Stage 4 — Runtime
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tzdata \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --system --gid 10001 tesie \
 && useradd --system --uid 10001 --gid tesie --home-dir /data tesie

COPY --from=gateway /out/tesie /app/bin/tesie
COPY --from=core /out/tesie-core /app/bin/tesie-core
COPY --from=web /src/frontend/dist /app/www

# TesieCore is resolved from the binary's own directory, so both binaries sit
# in /app/bin and the SPA is passed explicitly.
ENV HOST=0.0.0.0 \
    PORT=20129 \
    TESIE_DATA_DIR=/data \
    FRONTEND_DIR=/app/www \
    TESIE_NO_OPEN=1 \
    TESIE_REQUIRE_ADMIN_AUTH=1 \
    RUST_LOG=info

RUN mkdir -p /data && chown -R tesie:tesie /data /app
USER tesie
WORKDIR /data
VOLUME ["/data"]

EXPOSE 20129

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/app/bin/tesie", "--port", "20129", "status"]

ENTRYPOINT ["/app/bin/tesie", "--serve"]
