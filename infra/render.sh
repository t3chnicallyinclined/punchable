#!/usr/bin/env bash
# Render a manifest from infra/config.env: replaces __PK_DOMAIN__, __PK_AUTH_HOST__, __PK_MAIL_FROM__, __PK_NS__,
# __PK_KC_NS__, __PK_REALM__, __PK_ORIGIN_IP__, __PK_OWNER_ID__ (default owner) and (optionally) __IMAGE_REF__. Refuses to output anything with a placeholder left.
#   infra/render.sh <file> [image_ref]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
. "$HERE/config.env"
F="${1:?usage: render.sh <file> [image_ref]}"; IMG="${2:-}"
out="$(sed -e "s|__PK_DOMAIN__|$PK_DOMAIN|g" -e "s|__PK_AUTH_HOST__|$PK_AUTH_HOST|g" -e "s|__PK_MAIL_FROM__|$PK_MAIL_FROM|g" \
           -e "s|__PK_NS__|$PK_NS|g" -e "s|__PK_KC_NS__|$PK_KC_NS|g" -e "s|__PK_REALM__|$PK_REALM|g" -e "s|__PK_ORIGIN_IP__|$PK_ORIGIN_IP|g" \
           -e "s|__PK_OWNER_ID__|${PK_OWNER_ID:-owner}|g" \
           ${IMG:+-e "s|__IMAGE_REF__|$IMG|g"} "$F")"
if grep -qE '__(PK_[A-Z_]+|IMAGE_REF)__' <<<"$out"; then
  echo "!! $F still has placeholders after rendering: $(grep -oE '__(PK_[A-Z_]+|IMAGE_REF)__' <<<"$out" | sort -u | tr '\n' ' ')" >&2
  exit 1
fi
printf '%s\n' "$out"
