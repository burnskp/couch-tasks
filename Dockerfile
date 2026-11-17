# syntax=docker/dockerfile:1

FROM rust:1.99-trixie AS builder
WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked --bin ct-sync-server \
    && install -Dm755 target/release/ct-sync-server /out/ct-sync-server \
    && mkdir -p /out/data

FROM gcr.io/distroless/cc-debian13:nonroot

LABEL org.opencontainers.image.title="couch-tasks-sync-server" \
      org.opencontainers.image.description="Couch Tasks mTLS sync server" \
      org.opencontainers.image.source="https://github.com/burnskp/couch-tasks" \
      org.opencontainers.image.licenses="MIT"

COPY --from=builder /out/ct-sync-server /usr/local/bin/ct-sync-server
COPY --from=builder --chown=65532:65532 /out/data /data

ENV CT_SYNC_BIND=0.0.0.0:51401 \
    CT_SYNC_DB=/data/tasks.db \
    CT_SYNC_SERVER_CERT=/certs/server.crt \
    CT_SYNC_SERVER_KEY=/certs/server.key \
    CT_SYNC_CLIENT_CA=/certs/ca.crt

VOLUME ["/data"]
EXPOSE 51401
USER nonroot

ENTRYPOINT ["/usr/local/bin/ct-sync-server"]
