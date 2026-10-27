#!/usr/bin/env bash
# From the dev machine: send the COMMITTED tree to the server and run one step there (infra/deploy.sh lists them).
#   infra/ship.sh test | base | google-idp | keycloak | keycloak-ingress | build api|web | rollout api|web
#                 | dbload [--check] | ingress | status | backup-secret | backup | backup-now | backup-list
#   backup-secret sends the R2_* lines of ~/.punchable/r2.env (a token scoped to the backup bucket) over ssh STDIN into
#   the Secret punchable-r2-backup (deploy.sh backup-secret). Never in argv, git, logs or the screen.
#   legal-rules sends ~/.punchable/legal_rules.json (gsync/rules.py's private list) over ssh STDIN into the Secret
#   punchable-legal-rules, which the API's ingest routes apply to everything the home worker posts.
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
if [ "$STEP" = legal-rules ]; then
  F="${PUNCHABLE_LEGAL_RULES:-$HOME/.punchable/legal_rules.json}"
  [ -f "$F" ] || { echo "!! $F not found"; exit 1; }
  python -c 'import json,sys; json.load(open(sys.argv[1], encoding="utf-8"))' "$F" || { echo "!! $F is not JSON"; exit 1; }
  ssh -i "$KEY" "$HOST" "cd ~/$DIR && SHA=$SHA bash infra/deploy.sh legal-rules" < "$F"
  exit $?
fi
if [ "$STEP" = backup-secret ]; then
  F="${R2_BACKUP_ENV:-$HOME/.punchable/r2.env}"
  [ -f "$F" ] || { echo "!! $F not found (R2_ENDPOINT, R2_BUCKET, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY)"; exit 1; }
  grep -qE '^R2_ENDPOINT=https://[0-9a-f]{32}\.r2\.cloudflarestorage\.com/?\s*$' "$F" \
    || { echo "!! R2_ENDPOINT missing or not https://<account id>.r2.cloudflarestorage.com in $F"; exit 1; }
  grep -qE '^R2_BUCKET=[a-z0-9][a-z0-9-]{1,61}[a-z0-9]\s*$' "$F" || { echo "!! R2_BUCKET missing or not a bucket name in $F"; exit 1; }
  grep -qE '^R2_ACCESS_KEY_ID=[0-9a-f]{32}\s*$' "$F" || { echo "!! R2_ACCESS_KEY_ID missing or not 32 hex in $F"; exit 1; }
  grep -qE '^R2_SECRET_ACCESS_KEY=[0-9a-f]{64}\s*$' "$F" || { echo "!! R2_SECRET_ACCESS_KEY missing or not 64 hex in $F"; exit 1; }
  grep -E '^R2_(ENDPOINT|BUCKET|ACCESS_KEY_ID|SECRET_ACCESS_KEY)=' "$F" | tr -d '\r' \
    | ssh -i "$KEY" "$HOST" "cd ~/$DIR && SHA=$SHA bash infra/deploy.sh backup-secret"
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
