# syntax=docker/dockerfile:1.7

# Multi-stage build targeting linux/amd64 (x86_64)
FROM rust:1.92-trixie AS builder

WORKDIR /app

# System deps for native-tls / OpenSSL
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    ca-certificates \
  && rm -rf /var/lib/apt/lists/*

# Copy manifests first for better caching
COPY Cargo.toml Cargo.lock ./
COPY entity/Cargo.toml entity/Cargo.toml
COPY migration/Cargo.toml migration/Cargo.toml

# Copy minimal src so workspace members are valid
COPY entity/src entity/src
COPY migration/src migration/src

# Pre-fetch dependencies (cached across builds)
RUN cargo fetch --locked

# Copy source
COPY src src

# Create a permanent directory for the output
RUN mkdir -p /app/dist

# Cache only Cargo registries/git; keep build artifacts in the image layer
# so the runtime stage can COPY the binaries.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build -p backvonia --release --locked \
    && cp /app/target/release/backvonia /app/dist/backvonia

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build -p migration --release --locked \
    && cp /app/target/release/migration /app/dist/backvonia-migrate

FROM debian:trixie-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/dist/backvonia /usr/local/bin/backvonia
COPY --from=builder /app/dist/backvonia-migrate /usr/local/bin/backvonia-migrate

RUN useradd -m -u 10001 appuser
USER appuser

ENV HOST=0.0.0.0 \
    PORT=8080 \
    RUST_LOG=info,backvonia=debug

ENTRYPOINT ["backvonia"]
