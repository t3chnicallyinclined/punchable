#!/usr/bin/env bash
# Mount the punchable login theme on the SHARED Keycloak, the way the other products' themes are mounted there:
#   ConfigMap keycloak-theme-punchable (the flat theme/login/ dir)  ->  init container `setup-theme` copies it into an
#   emptyDir `punchable-theme`  ->  mounted at /opt/keycloak/themes/punchable in the keycloak container.
# Patching the Deployment RESTARTS Keycloak (every realm's sign-in pauses for the restart): run only on the owner's go.
# Idempotent: a second run changes nothing and does not restart. `--dry-run` applies nothing and prints the patch.
# Then infra/keycloak/apply.sh imports the realm WITH loginTheme (it checks the `punchable-theme` volume by name).
# Rollback: microk8s kubectl -n "$PK_KC_NS" rollout undo deploy/keycloak; then infra/keycloak/apply.sh (it imports
#           without loginTheme when the volume is gone), so the realm shows Keycloak's default look again.
set -euo pipefail
cd "$(dirname "$0")/../.."
. infra/config.env
K="microk8s kubectl"; NS="$PK_KC_NS"; DIR=infra/keycloak/theme/login
[ -f "$DIR/template.ftl" ] && [ -f "$DIR/theme.properties" ] || { echo "!! $DIR is not a theme"; exit 1; }
DRY="${1:-}"

FILES=()
for f in "$DIR"/*.ftl "$DIR"/theme.properties "$DIR"/messages_en.properties "$DIR"/login.css; do [ -f "$f" ] && FILES+=(--from-file="$f"); done
echo "==> ConfigMap keycloak-theme-punchable: ${#FILES[@]} files from $DIR"
if [ "$DRY" != --dry-run ]; then
  $K -n "$NS" create cm keycloak-theme-punchable "${FILES[@]}" --dry-run=client -o yaml | $K -n "$NS" apply -f -
fi

echo "==> deploy/keycloak: volumes, setup-theme copy lines, theme mount"
$K -n "$NS" get deploy keycloak -o json > /tmp/kc-deploy.json
set +e; python3 infra/keycloak/theme_patch.py /tmp/kc-deploy.json /tmp/kc-patch.json; RC=$?; set -e
if [ "$RC" = 3 ]; then :; elif [ "$RC" != 0 ]; then exit "$RC"; elif [ "$DRY" = --dry-run ]; then
  echo "(dry run) patch:"; python3 -c 'import json; print(json.dumps(json.load(open("/tmp/kc-patch.json")), indent=1)[:3000])'
  rm -f /tmp/kc-patch.json; exit 0
else
  $K -n "$NS" patch deploy keycloak --type strategic --patch-file /tmp/kc-patch.json
  rm -f /tmp/kc-patch.json
  echo "==> Keycloak restarting (shared: every realm's sign-in pauses until it is back)"
  $K -n "$NS" rollout status deploy/keycloak --timeout=5m
  echo -n "themes/punchable/login: "; $K -n "$NS" exec deploy/keycloak -- ls /opt/keycloak/themes/punchable/login | tr '\n' ' '; echo
fi
[ "$DRY" = --dry-run ] && exit 0
echo "==> realm import with loginTheme (infra/keycloak/apply.sh)"
bash infra/keycloak/apply.sh
