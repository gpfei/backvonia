# syntax=docker/dockerfile:1.7

# Multi-stage build targeting linux/amd64 (x86_64)
FROM --platform=linux/amd64 rust:1.84-bookworm AS builder

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

# Pre-fetch dependencies (cached across builds)
RUN cargo fetch --locked

# Copy source
COPY src src
COPY entity/src entity/src
COPY migration/src migration/src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked

FROM --platform=linux/amd64 debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/backvonia /usr/local/bin/backvonia

RUN useradd -m -u 10001 appuser
USER appuser

EXPOSE 8080

ENV HOST=0.0.0.0 \
    PORT=8080 \
    RUST_LOG=info,backvonia=debug

ENTRYPOINT ["backvonia"]
