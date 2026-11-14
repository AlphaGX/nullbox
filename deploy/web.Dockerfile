# CI image: Caddy with the web bundle built by the workflow.
FROM caddy:2.11.9-alpine
COPY Caddyfile /etc/caddy/Caddyfile
COPY dist /srv
