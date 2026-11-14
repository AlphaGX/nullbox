# Two images from one file: `api` (the Rust server) and `web` (Caddy serving the SPA and proxying /api).

FROM rust:1.99.1-slim-bookworm AS api-build
WORKDIR /src
ENV SQLX_OFFLINE=true
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p nullbox-api && cp target/release/nullbox-api /nullbox-api

FROM debian:bookworm-20261104-slim AS api
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* && useradd -r -u 10001 nullbox && mkdir /data && chown nullbox /data
COPY --from=api-build /nullbox-api /usr/local/bin/nullbox-api
USER nullbox
ENV BIND_ADDR=0.0.0.0:3000 DATABASE_URL=sqlite:///data/nullbox.db RUST_LOG=info
EXPOSE 3000
HEALTHCHECK --interval=10s --timeout=3s CMD curl -fsS http://127.0.0.1:3000/api/healthz || exit 1
CMD ["nullbox-api"]

FROM node:22.23.5-bookworm-slim AS web-build
WORKDIR /src
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps/web/package.json apps/web/
RUN --mount=type=cache,target=/root/.local/share/pnpm/store pnpm install --frozen-lockfile
COPY . .
# Empty: the web app calls /api on its own origin, Caddy proxies it.
ENV VITE_API_BASE_URL=
RUN pnpm build

FROM caddy:2.11.9-alpine AS web
COPY Caddyfile /etc/caddy/Caddyfile
COPY --from=web-build /src/apps/web/dist /srv
