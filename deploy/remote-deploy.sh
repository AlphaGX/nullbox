#!/usr/bin/env bash
# Runs on the server, fed over SSH by the deploy job. The first stdin line is a GHCR token
# (the job's own, short-lived); API_IMAGE, WEB_IMAGE and ACTOR come from the command line.
set -euo pipefail
cd ~/nullbox
# Record the running version in .env, so a plain `docker compose up -d` keeps it.
sed -i '/^API_IMAGE=/d;/^WEB_IMAGE=/d' .env
printf 'API_IMAGE=%s\nWEB_IMAGE=%s\n' "$API_IMAGE" "$WEB_IMAGE" >> .env
printf '%s' "$GHCR_TOKEN" | docker login ghcr.io -u "$ACTOR" --password-stdin >/dev/null
trap 'docker logout ghcr.io >/dev/null' EXIT
docker compose pull -q
docker compose up -d --no-build --wait --wait-timeout 90
# Fail the job unless both services run this commit's images.
for svc in api web; do
  want=$([ "$svc" = api ] && echo "$API_IMAGE" || echo "$WEB_IMAGE")
  got=$(docker inspect --format '{{.Config.Image}}' "$(docker compose ps -q "$svc")")
  [ "$got" = "$want" ] || { echo "$svc runs $got, expected $want" >&2; exit 1; }
done
echo "deployed $API_IMAGE and $WEB_IMAGE"
docker image prune -af --filter until=24h >/dev/null || true
