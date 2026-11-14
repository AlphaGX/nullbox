# CI image: the release binary built (and cached) by the workflow. Trixie, so its glibc is
# at least as new as the ubuntu-24.04 runner that compiled the binary.
FROM debian:trixie-20261104-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* && useradd -r -u 10001 nullbox && mkdir /data && chown nullbox /data
COPY nullbox-api /usr/local/bin/nullbox-api
USER nullbox
ENV BIND_ADDR=0.0.0.0:3000 DATABASE_URL=sqlite:///data/nullbox.db RUST_LOG=info
EXPOSE 3000
HEALTHCHECK --interval=10s --timeout=3s CMD curl -fsS http://127.0.0.1:3000/api/healthz || exit 1
CMD ["nullbox-api"]
