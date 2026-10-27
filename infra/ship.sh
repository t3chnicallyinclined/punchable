#!/usr/bin/env bash
# From the dev machine: send the COMMITTED tree to the server and run one step there (infra/deploy.sh lists them).
#   infra/ship.sh test | base | google-idp | keycloak | keycloak-ingress | build api|web | rollout api|web
#                 | dbload [--check] | ingress | status
#   dbload packs Punchable's text/JSON data HERE (python db/push_remote.pack, no audio), copies the tarball to the server
#   over the same ssh, and loads it from a pod there (infra/deploy.sh dbload). The tarball is deleted after a load.
#   infra/ship.sh dns plan|apply|undo     runs HERE (Cloudflare API, token from ~/.punchable/cloudflare.env)
# Ships `git archive HEAD` (only committed work) plus your config file as infra/config.env. The config is
# $PK_INFRA_ENV, else infra/config.env, else ~/.punchable/infra.env (keep it there: it is never in git).
# Set PK_SSH_HOST (user@host) and PK_SSH_KEY in it.
set -euo pipefail
CFG="${PK_INFRA_ENV:-}"
if [ -z "$CFG" ]; then
  if [ -f "$(dirname "$0")/config.env" ]; then CFG="$(dirname "$0")/config.env"; else CFG="$HOME/.punchable/infra.env"; fi
fi
[ -f "$CFG" ] || { echo "!! no infra config: copy infra/config.env.example to ~/.punchable/infra.env and fill it in"; exit 1; }
. "$CFG"
STEP="${1:?usage: infra/ship.sh <step> [arg]}"; ARG="${2:-}"
HOST="${PK_SSH_HOST:?set PK_SSH_HOST (user@host) in $CFG}"
KEY="${PK_SSH_KEY:-$HOME/.ssh/id_ed25519}"
cd "$(git rev-parse --show-toplevel)"
if [ "$STEP" = dns ]; then PK_INFRA_ENV="$CFG" exec python -P infra/dns/cf_dns.py "${ARG:-plan}"; fi
SHA="$(git rev-parse HEAD)"
git diff --quiet HEAD -- infra || echo "!! infra/ has uncommitted changes: they are NOT shipped"
DIR="projects/punchable-hosted/src-${SHA:0:12}"
git archive --format=tar HEAD | ssh -i "$KEY" "$HOST" "mkdir -p ~/$DIR && tar -x -C ~/$DIR"
# the config is not in git: send it next to the tree (deploy.sh, render.sh and keycloak/*.sh source infra/config.env)
tr -d '\r' < "$CFG" | ssh -i "$KEY" "$HOST" "umask 077 && cat > ~/$DIR/infra/config.env"
if [ "$STEP" = google-idp ]; then
  # "Sign in with Google": the OAuth client id + secret from ~/.punchable/google-oauth.env go over ssh STDIN into the
  # k8s Secret the Keycloak namespace/punchable-google-idp (deploy.sh google-idp). Never in argv, git, logs or the screen.
  F="${GOOGLE_OAUTH_ENV:-$HOME/.punchable/google-oauth.env}"
  [ -f "$F" ] || { echo "!! $F not found (gsync/SETUP.md step 5)"; exit 1; }
  grep -qE '^GOOGLE_CLIENT_ID=.+\.apps\.googleusercontent\.com\s*$' "$F" || { echo "!! GOOGLE_CLIENT_ID missing or not a Google client id in $F"; exit 1; }
  grep -qE '^GOOGLE_CLIENT_SECRET=.+' "$F" || { echo "!! GOOGLE_CLIENT_SECRET missing in $F"; exit 1; }
  grep -E '^GOOGLE_CLIENT_(ID|SECRET)=' "$F" | tr -d '\r' | ssh -i "$KEY" "$HOST" "cd ~/$DIR && SHA=$SHA bash infra/deploy.sh google-idp"
  exit $?
fi
if [ "$STEP" = dbload ]; then
  [ -z "$ARG" ] || [ "$ARG" = --check ] || { echo "usage: infra/ship.sh dbload [--check]"; exit 1; }
  TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
  python db/push_remote.py pack --tar "$TMP/bundle.tgz"
  B="projects/punchable-hosted/dbload-${SHA:0:12}-$(date -u +%Y%m%d%H%M%S).tgz"
  ssh -i "$KEY" "$HOST" "umask 077 && cat > ~/$B" < "$TMP/bundle.tgz"
  ARG="\$HOME/$B $ARG"
fi
ssh -i "$KEY" "$HOST" "cd ~/$DIR && SHA=$SHA bash infra/deploy.sh $STEP $ARG"
