#!/usr/bin/env bash
# Once SurrealDB is Ready: namespace `punchable`, database `main` (STRICT), and a DATABASE-level user `api` (EDITOR)
# for the API -- the API never holds root. Run ON the server (infra/ship.sh base). Idempotent.
#   Secret punchable-api: SURREAL_API_USER=api, SURREAL_API_PASS=<generated once>. DEFINE USER ... OVERWRITE keeps the
#   database in step with the Secret on every run.
# The SQL (with the password) goes to `surreal sql` on STDIN inside the pod; the root login comes from the
# container's own env. Nothing secret is in argv or printed.
# Test hook: SURREAL_EXEC / API_PASS_CMD let backup/selftest.sh run this against a Docker stand-in.
set -euo pipefail
K="microk8s kubectl"; NS=punchable; POD=surrealdb-0
EXEC="${SURREAL_EXEC:-$K -n $NS exec -i $POD --}"
if [ -z "${SURREAL_EXEC:-}" ]; then
  $K -n $NS wait --for=condition=Ready pod/$POD --timeout=3m
  if ! $K -n $NS get secret punchable-api >/dev/null 2>&1; then
    P="$(openssl rand -hex 32)"
    $K -n $NS create secret generic punchable-api --from-env-file=<(printf 'SURREAL_API_USER=api\nSURREAL_API_PASS=%s\n' "$P")
    unset P
    echo "created punchable-api"
  fi
  AP="$($K -n $NS get secret punchable-api -o jsonpath='{.data.SURREAL_API_PASS}' | base64 -d)"
else
  AP="$(${API_PASS_CMD:?})"
fi
[[ "$AP" =~ ^[0-9a-f]{64}$ ]] || { echo "!! SURREAL_API_PASS is not 64 hex"; exit 1; }
OUT=$(mktemp); trap 'rm -f "$OUT"' EXIT
printf 'DEFINE NAMESPACE IF NOT EXISTS punchable;\nUSE NS punchable;\nDEFINE DATABASE IF NOT EXISTS main STRICT;\nUSE NS punchable DB main;\nDEFINE USER OVERWRITE api ON DATABASE PASSWORD "%s" ROLES EDITOR;\n' "$AP" \
  | $EXEC /surreal sql --hide-welcome --json >"$OUT" 2>&1 || { sed -E 's/[0-9a-f]{64}/<redacted>/g' "$OUT"; exit 1; }
unset AP
if grep -qiE 'error|denied|failed' "$OUT"; then sed -E 's/[0-9a-f]{64}/<redacted>/g' "$OUT"; exit 1; fi
U=$(printf 'INFO FOR DB;\n' | $EXEC /surreal sql --ns punchable --db main --json --hide-welcome \
  | python3 -c 'import sys,json; d=json.loads(sys.stdin.read().strip().splitlines()[-1]); print(" ".join(sorted(d[-1]["users"])))')
echo "database users on punchable/main: $U"
[ "$U" = api ] || { echo "!! expected exactly the user api"; exit 1; }
