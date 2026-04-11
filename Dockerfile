# syntax=docker/dockerfile:1.7

FROM rust:1-bookworm AS builder

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked \
    && strip /app/target/release/tesla-superchargers \
    && install -Dm755 /app/target/release/tesla-superchargers /out/tesla-superchargers

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
        tini \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --home-dir /home/appuser --shell /usr/sbin/nologin appuser \
    && mkdir -p /app \
    && chown -R appuser:appuser /app /home/appuser

WORKDIR /app

ENV PORT=8080

COPY --from=builder /out/tesla-superchargers /usr/local/bin/tesla-superchargers

USER appuser

EXPOSE 8080

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/tesla-superchargers", "host"]
