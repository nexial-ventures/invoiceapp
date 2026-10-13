# syntax=docker/dockerfile:1.7

# ---------- Frontend ----------
FROM node:22-bookworm-slim AS frontend
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# ---------- Backend ----------
# Keep the tag equal to rust-toolchain.toml's channel.
FROM rust:1.97.0-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src/ ./src/
# rust-embed bakes the default PDF design into the binary.
COPY design/ ./design/
# rust-embed reads frontend/dist at compile time (release embeds the files).
COPY --from=frontend /app/frontend/dist ./frontend/dist
RUN cargo build --release --locked --bin invoiceapp

# ---------- Runtime ----------
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin invoiceapp \
 && mkdir -p /data \
 && chown invoiceapp:invoiceapp /data
COPY --from=builder /app/target/release/invoiceapp /usr/local/bin/invoiceapp
USER invoiceapp
ENV INVOICEAPP__BIND=0.0.0.0:3000
# fs storage: archived PDFs, originals, design overrides (keep on a persistent
# volume). With INVOICEAPP__STORAGE_KIND=s3 the volume stays unused.
ENV INVOICEAPP__STORAGE_DIR=/data
VOLUME /data
EXPOSE 3000
ENTRYPOINT ["invoiceapp"]
CMD ["serve"]
