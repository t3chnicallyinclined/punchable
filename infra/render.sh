#!/usr/bin/env bash
# Render a manifest from infra/config.env: replaces __PK_DOMAIN__, __PK_AUTH_HOST__, __PK_MAIL_FROM__, __PK_NS__,
# __PK_KC_NS__, __PK_REALM__, __PK_ORIGIN_IP__, __PK_OWNER_ID__ (default owner), __PK_COMIC_NAME__ and
# __PK_COMIC_FIRST__ (how the AI prompts name the comic; default "the comic", the first name defaults to the name),
# __PK_WORKER_API__ (the worker's API base URL; default http://punchable-api, the Python Service; the Rust cutover sets
# PK_WORKER_API=http://punchable-api-rs in config.env, the one place it changes) and (optionally) __IMAGE_REF__ and
# __IMAGE_REF_RS__. Refuses to output anything with ANY __NAME__ placeholder left (upper-case only, so a Python
# __init__ in a ConfigMap is not one).
#   infra/render.sh <file> [image_ref [image_ref_rs]]      (image_ref_rs may also come from $PK_IMAGE_REF_RS)
# A document carrying the line `# render-flag: servicemonitor` is dropped unless PK_SERVICEMONITOR=1: it needs the
# Prometheus Operator's ServiceMonitor CRD, which not every cluster has.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
. "$HERE/config.env"
F="${1:?usage: render.sh <file> [image_ref [image_ref_rs]]}"; IMG="${2:-}"; IMGRS="${3:-${PK_IMAGE_REF_RS:-}}"
out="$(sed -e "s|__PK_DOMAIN__|$PK_DOMAIN|g" -e "s|__PK_AUTH_HOST__|$PK_AUTH_HOST|g" -e "s|__PK_MAIL_FROM__|$PK_MAIL_FROM|g" \
           -e "s|__PK_NS__|$PK_NS|g" -e "s|__PK_KC_NS__|$PK_KC_NS|g" -e "s|__PK_REALM__|$PK_REALM|g" -e "s|__PK_ORIGIN_IP__|$PK_ORIGIN_IP|g" \
           -e "s|__PK_OWNER_ID__|${PK_OWNER_ID:-owner}|g"            -e "s|__PK_COMIC_NAME__|${PK_COMIC_NAME:-the comic}|g" -e "s|__PK_COMIC_FIRST__|${PK_COMIC_FIRST:-${PK_COMIC_NAME:-the comic}}|g" \
           -e "s|__PK_WORKER_API__|${PK_WORKER_API:-http://punchable-api}|g" \
           ${IMG:+-e "s|__IMAGE_REF__|$IMG|g"} ${IMGRS:+-e "s|__IMAGE_REF_RS__|$IMGRS|g"} "$F")"
if [ "${PK_SERVICEMONITOR:-0}" != 1 ]; then
  out="$(awk 'function flush() { if (n && !drop) printf "%s", buf; buf = ""; n = 0; drop = 0 }
              /^---[ \t]*$/ { flush() }
              { buf = buf $0 "\n"; n = 1; if ($0 ~ /^# render-flag: servicemonitor[ \t]*$/) drop = 1 }
              END { flush() }' <<<"$out")"
fi
if grep -qE '__[A-Z][A-Z0-9_]*__' <<<"$out"; then
  echo "!! $F still has placeholders after rendering: $(grep -oE '__[A-Z][A-Z0-9_]*__' <<<"$out" | sort -u | tr '\n' ' ')" >&2
  exit 1
fi
printf '%s\n' "$out"
