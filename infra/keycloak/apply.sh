#!/usr/bin/env bash
# Put Punchable's realm into the shared Keycloak. Run ON the server from the unpacked repo (infra/ship.sh keycloak).
#   infra/keycloak/apply.sh            realm ConfigMap + import job (works before DNS; in-cluster only)
#   infra/keycloak/apply.sh ingress    the auth.<domain> Ingress (ONLY after DNS resolves to the server)
#
# Touches ONLY punchable objects in the Keycloak namespace (ConfigMap keycloak-realm-punchable, Job
# keycloak-config-punchable, Ingress punchable-auth). It does NOT touch the keycloak Deployment (the login theme is a
# separate step, mounted by hand), the shared keycloak-config job, or any other realm.
# Rollback: microk8s kubectl -n "$PK_KC_NS" delete ingress punchable-auth; delete job keycloak-config-punchable;
#   delete configmap keycloak-realm-punchable; then delete the realm (README "Rollback").
set -euo pipefail
cd "$(dirname "$0")"
. ../config.env
K="microk8s kubectl"
NS="$PK_KC_NS"
R=../render.sh

if [ "${1:-}" = ingress ]; then
  getent hosts "$PK_AUTH_HOST" | grep -q "$PK_ORIGIN_IP" || { echo "!! $PK_AUTH_HOST does not resolve to $PK_ORIGIN_IP yet; not applying"; exit 1; }
  bash $R auth-ingress.yaml | $K apply -f -
  echo "==> waiting for the certificate"; $K -n $NS wait --for=condition=Ready certificate/punchable-auth-tls --timeout=5m
  curl -fsS "https://$PK_AUTH_HOST/realms/$PK_REALM/.well-known/openid-configuration" | python3 -c 'import sys,json;print("issuer:",json.load(sys.stdin)["issuer"])'
  exit 0
fi

echo "==> refusing to run if the realm file would touch another realm"
[ "$(grep -c '^realm: ' realm-punchable.yaml)" = 1 ] && grep -q "^realm: $PK_REALM$" realm-punchable.yaml || { echo "!! realm file is not exactly realm $PK_REALM"; exit 1; }
$K -n $NS get secret keycloak-smtp-cloudflare >/dev/null || { echo "!! keycloak-smtp-cloudflare missing"; exit 1; }
$K -n $NS get secret punchable-google-idp >/dev/null || { echo "!! punchable-google-idp missing: run infra/ship.sh google-idp first (gsync/SETUP.md)"; exit 1; }

REALM=realm-punchable.yaml
if grep -q '^loginTheme: punchable$' $REALM && ! $K -n $NS get deploy keycloak -o jsonpath='{.spec.template.spec.volumes[*].name}' | grep -qw punchable-theme; then
  # The theme isn't mounted on the shared Keycloak yet : import without loginTheme so the
  # realm keeps Keycloak's default look instead of naming a theme that doesn't exist.
  echo "==> punchable theme not mounted yet: importing the realm without loginTheme"
  mkdir -p /tmp/pk-realm && sed '/^loginTheme: punchable$/d' $REALM > /tmp/pk-realm/realm-punchable.yaml && REALM=/tmp/pk-realm/realm-punchable.yaml
fi
echo "==> realm ConfigMap (server-side apply: no last-applied annotation)"
$K -n $NS create configmap keycloak-realm-punchable --from-file=realm-punchable.yaml=$REALM --dry-run=client -o yaml \
  | $K apply --server-side --field-manager=punchable -f -
echo "==> realm import job (its own; never the shared keycloak-config job)"
$K -n $NS delete job keycloak-config-punchable --ignore-not-found
bash $R keycloak-punchable.yaml | $K apply --server-side --field-manager=punchable -f -
$K -n $NS wait --for=condition=complete job/keycloak-config-punchable --timeout=5m
$K -n $NS logs job/keycloak-config-punchable --tail=3
echo "==> in-cluster check"
# The host cannot reach ClusterIPs (verified 2026-10-26), so ask from a throwaway pod in this namespace.
ISS=$($K -n $NS run pk-kc-check --rm -i --quiet --restart=Never --labels=app.kubernetes.io/part-of=punchable   --image=curlimages/curl@sha256:d9b4541e214bcd85196d6e92e2753ac6d0ea699f0af5741f8c6cccbfcf00ef4b --command --   curl -fsS "http://keycloak/realms/$PK_REALM/.well-known/openid-configuration"   | python3 -c 'import sys,json;print(json.load(sys.stdin)["issuer"])')
echo "issuer: $ISS"
[ "$ISS" = "https://$PK_AUTH_HOST/realms/$PK_REALM" ] || { echo "!! issuer is not https://$PK_AUTH_HOST/realms/$PK_REALM"; exit 1; }
echo "==> done. Public discovery works after DNS + 'apply.sh ingress'."
