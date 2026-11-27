# syntax=docker/dockerfile:1

# Build stage. Trixie matches the Debian 13 runtime glibc.
FROM rust:1.100-slim-trixie AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Copy the workspace. .dockerignore keeps the context small.
COPY . .

RUN cargo build --release --locked -p storkv-server -p storkv-cli

# Stage the writable directories with the distroless nonroot owner.
RUN install -d -o 65532 -g 65532 \
    /staging/var/lib/storkv \
    /staging/var/lib/storkv/backups \
    /staging/etc/storkv

# Runtime stage. Distroless has no shell and runs as UID/GID 65532.
FROM gcr.io/distroless/cc-debian13:nonroot

COPY --from=builder --chown=65532:65532 \
    /src/target/release/storkvd /usr/local/bin/storkvd
COPY --from=builder --chown=65532:65532 \
    /src/target/release/storkvctl /usr/local/bin/storkvctl
COPY --from=builder --chown=65532:65532 \
    /staging/var/lib/storkv /var/lib/storkv
COPY --from=builder --chown=65532:65532 \
    /staging/etc/storkv /etc/storkv
COPY --from=builder --chown=65532:65532 \
    /src/storkv.example.toml /etc/storkv/storkv.example.toml

USER 65532:65532
WORKDIR /var/lib/storkv

# Native, admin, HTTP, WebSocket, gRPC, Redis, and Memcached listeners.
EXPOSE 7878 7879 8080 8081 9090 6379 11211

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD ["storkvctl", "health", "--sock", "127.0.0.1:7878"]

ENTRYPOINT ["storkvd"]
CMD ["--data-dir", "/var/lib/storkv", "--listen", "0.0.0.0:7878"]
