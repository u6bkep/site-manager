FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static libgit2-dev zlib-dev zlib-static
WORKDIR /app
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    cp target/release/site-manager /site-manager

FROM caddy:2-alpine AS caddy

FROM alpine:3.20
RUN apk add --no-cache ca-certificates tini libgit2

COPY --from=builder /site-manager /usr/local/bin/site-manager
COPY --from=caddy /usr/bin/caddy /usr/bin/caddy
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

ENV DATA_DIR=/var/lib/site-manager \
    CADDY_ROOT=/etc/caddy \
    BIND_ADDR=0.0.0.0:8080 \
    RUST_LOG=info

EXPOSE 80 443 8080
VOLUME ["/var/lib/site-manager", "/etc/caddy"]

ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/entrypoint.sh"]
