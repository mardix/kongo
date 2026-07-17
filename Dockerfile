# syntax=docker/dockerfile:1

# 1. Start with cargo-chef
FROM lukemathwalker/cargo-chef:latest-rust-1.85-bookworm AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# 2. Build dependencies (Cached layer)
FROM chef AS builder 
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# 3. Application stage: Copy source and build
COPY . . 
RUN cargo build --release --bin kongo

# Build the browser admin independently so frontend changes do not invalidate
# the cached Rust dependency layer.
FROM node:22-bookworm-slim AS admin-ui-builder
WORKDIR /app/admin-ui
COPY admin-ui/package.json admin-ui/package-lock.json ./
RUN npm ci
COPY admin-ui/ ./
RUN npm run build

# 4. Runtime stage: The final image
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 10001 kongo \
    && mkdir -p /data \
    && chown -R kongo:kongo /data

WORKDIR /app

# --- ASSET COPIES ---
# Copy the binary
COPY --from=builder /app/target/release/kongo /usr/local/bin/kongo
# Copy the README specifically so the app can access it
COPY --from=builder /app/DOCUMENTATION.md /app/DOCUMENTATION.md
COPY --from=admin-ui-builder /app/admin-ui/dist /app/admin-ui/dist
# Copy scripts and env data
COPY scripts/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
COPY kongodb.env /app/kongodb.env

ENV KONGODB_DATA_DIR=/data
ENV KONGODB_BACKUP_PATH=/data/backups
ENV KONGODB_EXPORT_PATH=/data/exports
ENV KONGODB_DOCS_FILE=/app/DOCUMENTATION.md
VOLUME ["/data"]

USER kongo
EXPOSE 8080

CMD ["/usr/local/bin/docker-entrypoint.sh"]
