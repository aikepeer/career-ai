# Multi-stage Dockerfile for careerai-hosted beta API.
#
# Build stage: compile the hosted API binary in release mode.
# Runtime stage: minimal image with the binary + pandoc for rendering.

FROM rust:1.78-bookworm AS builder

WORKDIR /build

# Install mold linker for faster linking
RUN apt-get update && apt-get install -y --no-install-recommends \
    clang \
    mold \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace
COPY . .

# The build image is the toolchain pin; do not let the repository's
# development channel override it.
RUN rm -f rust-toolchain.toml

# Build the hosted API server binary
RUN SCCACHE_DIRECT=true cargo build --release -p careerai-hosted --bin careerai-hosted || \
    cargo build --release -p careerai-hosted

# ── Runtime stage ────────────────────────────────────────────────────

FROM debian:bookworm-slim AS runtime

# Install pandoc for resume rendering + ca-certificates for TLS
RUN apt-get update && apt-get install -y --no-install-recommends \
    pandoc \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy the built binary
COPY --from=builder /build/target/release/careerai-hosted /usr/local/bin/careerai-hosted

# Non-root user
RUN useradd --create-home --shell /bin/bash careerai
USER careerai

ENV RUST_LOG=info
ENV CAREERAI_HOSTED_BIND=0.0.0.0:3000

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://localhost:3000/v1/health || exit 1

ENTRYPOINT ["careerai-hosted"]
