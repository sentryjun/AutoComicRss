# syntax=docker/dockerfile:1
# SPDX-License-Identifier: GPL-2.0-only
FROM rust:1.93.1-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY web ./web
RUN cargo build --release --locked --bin autocomic-rss

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 autocomic \
    && useradd --uid 10001 --gid 10001 --no-create-home --home-dir /app autocomic \
    && install -d -o 10001 -g 10001 /app/data
WORKDIR /app
COPY --from=build /build/target/release/autocomic-rss /usr/local/bin/autocomic-rss
COPY LICENSE /usr/share/doc/autocomic-rss/LICENSE
ENV APP_HOST=0.0.0.0 APP_PORT=8080 DATABASE_URL=sqlite:///app/data/ani.db RUST_LOG=info
USER 10001:10001
EXPOSE 8080
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl --noproxy '*' --fail --silent --show-error --max-time 4 "http://127.0.0.1:${APP_PORT}/ready" || exit 1
ENTRYPOINT ["/usr/local/bin/autocomic-rss"]
