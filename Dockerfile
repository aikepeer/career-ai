# syntax=docker/dockerfile:1

# ---- builder: compile the workspace in release on Rust 1.78 (MSRV) ----
FROM rust:1.78-bookworm AS builder
WORKDIR /app

COPY . .

# The repo pins `channel = "stable"` in rust-toolchain.toml, which would
# make rustup fetch latest stable and bypass this image's 1.78 toolchain.
# Drop it so the build uses the pinned 1.78.0 (MSRV) from rust:1.78.
RUN rm -f rust-toolchain.toml

RUN cargo build --release --workspace

# ---- runtime: slim Debian + pandoc + weasyprint for rendering -------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        pandoc \
        weasyprint \
        fonts-dejavu-core \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/careerai /usr/local/bin/careerai
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh && mkdir -p /data

# All app state (SQLite DB, LLM cache, logs, render artifacts, config,
# profile) lives under CAREERAI_ROOT. Bind-mount /data from the host.
ENV CAREERAI_ROOT=/data
# The dashboard binds to loopback only by default. In a container the
# published port must bind 0.0.0.0 to be reachable from the host, so
# allow the non-loopback bind. Set CAREERAI_DASHBOARD_TOKEN to require
# a bearer token on every request before exposing it to a network.
ENV CAREERAI_DASHBOARD_ALLOW_NON_LOOPBACK=1

EXPOSE 3000
ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["daemon"]
