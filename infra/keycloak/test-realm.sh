#!/usr/bin/env bash
# Prove realm-punchable.yaml BEFORE production: import it with the same keycloak-config-cli into a THROWAWAY
# Keycloak (the production image, by digest) in plain Docker on the server, run test_realm.py, tear everything down.
# Touches no k8s object and not the shared Keycloak. Run ON the server from the unpacked repo: infra/keycloak/test-realm.sh
# The punchable login theme is mounted into the throwaway  and its login page is fetched
# and checked, so this also proves the FTL renders in the production Keycloak image (THEME_HTML=<file> keeps the page).
# ⚠ Plain Docker next to MicroK8s can disturb the cluster on some hosts: run it on a box where that is safe.
#   --red   mutate the realm (lifespan 301, PKCE off) and REQUIRE the tests to fail (the check can fail).
set -euo pipefail
cd "$(dirname "$0")"
. ../config.env
KC_IMG=${PK_KC_IMAGE:?set PK_KC_IMAGE in infra/config.env}
CLI_IMG=adorsys/keycloak-config-cli@sha256:44fcacaba522c159f4bb23a6bef0dd9e82291c5c84be3ed0ada77f6ff4663fd4
RED=${1:-}
T=pkkc-$$; NET=$T-net; W=$(mktemp -d /tmp/$T.XXXX)
cleanup() { docker rm -f $T-kc >/dev/null 2>&1 || true; docker network rm $NET >/dev/null 2>&1 || true; rm -rf "$W"; }
trap cleanup EXIT
cp realm-punchable.yaml "$W/"; chmod 755 "$W"; chmod 644 "$W/realm-punchable.yaml"
# theme, laid out exactly as the setup-theme init container does it (apply-theme.sh)
TH="$W/theme/login"; mkdir -p "$TH/resources/css" "$TH/messages"
cp theme/login/*.ftl theme/login/theme.properties "$TH/"; cp theme/login/login.css "$TH/resources/css/"; cp theme/login/messages_en.properties "$TH/messages/"
chmod -R a+rX "$W/theme"
if [ "$RED" = --red ]; then
  sed -i -e 's/^accessTokenLifespan: 300/accessTokenLifespan: 301/' -e 's/^    storeToken: true/    storeToken: false/' -e '0,/pkce.code.challenge.method: S256/s//pkce.code.challenge.method: ""/' "$W/realm-punchable.yaml"
fi
export PK_DOMAIN PK_AUTH_HOST PK_MAIL_FROM PK_REALM PK_OWNER_EMAIL
export KCA_USER=admin KCA_PASS=$(openssl rand -hex 16)
docker network create $NET >/dev/null
PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
docker run -d --name $T-kc --network $NET --network-alias kc -p 127.0.0.1:$PORT:8080 -m 1200m \
  -e KC_BOOTSTRAP_ADMIN_USERNAME=$KCA_USER -e KC_BOOTSTRAP_ADMIN_PASSWORD=$KCA_PASS -e KC_DB=dev-file \
  -e KC_HTTP_ENABLED=true -e KC_HOSTNAME_STRICT=false -v "$W/theme":/opt/keycloak/themes/punchable:ro "$KC_IMG" start-dev >/dev/null
for i in $(seq 120); do curl -sf "http://127.0.0.1:$PORT/realms/master" >/dev/null && break; sleep 2; done
curl -sf "http://127.0.0.1:$PORT/realms/master" >/dev/null || { docker logs --tail 30 $T-kc; echo "!! throwaway keycloak never came up"; exit 2; }
docker run --rm --network $NET -v "$W":/config:ro \
  -e KEYCLOAK_URL=http://kc:8080 -e KEYCLOAK_USER=$KCA_USER -e KEYCLOAK_PASSWORD=$KCA_PASS \
  -e KEYCLOAK_AVAILABILITYCHECK_ENABLED=true -e KEYCLOAK_AVAILABILITYCHECK_TIMEOUT=120s \
  -e IMPORT_FILES_LOCATIONS='/config/*' -e IMPORT_VARSUBSTITUTION_ENABLED=true \
  -e PK_DOMAIN -e PK_AUTH_HOST -e PK_MAIL_FROM -e CF_EMAIL_TOKEN=throwaway-not-a-token \
  -e GOOGLE_CLIENT_ID=throwaway.apps.googleusercontent.com -e GOOGLE_CLIENT_SECRET=throwaway-not-a-secret \
  "$CLI_IMG" 2>&1 | grep -E "ERROR|Exception|ran in" | head -20
set +e
python3 test_realm.py "http://127.0.0.1:$PORT" KCA_USER KCA_PASS "https://$PK_AUTH_HOST" "$PK_DOMAIN"
rc=$?
# first-login.sh logic against the throwaway: twice (idempotent), SEND=0. Must create exactly one user with
# zero credentials and both required actions.
cp kc-first-login.sh "$W/"; chmod 644 "$W/kc-first-login.sh"
for run in 1 2; do
  docker run --rm --network $NET -v "$W":/w:ro --entrypoint /bin/bash     -e KC_URL=http://kc:8080 -e KEYCLOAK_ADMIN=$KCA_USER -e KEYCLOAK_ADMIN_PASSWORD=$KCA_PASS     -e PK_REALM -e PK_OWNER_EMAIL -e PK_DOMAIN -e SEND=0 "$KC_IMG" /w/kc-first-login.sh > "$W/fl$run.log" 2>&1 || true
  sed 's/^/  first-login run '$run': /' "$W/fl$run.log" | grep -vE 'id=[0-9a-f-]{36}$' ; grep -oE '(created|exists)' "$W/fl$run.log" | sed 's/^/  -> /'
done
grep -q "^created user" "$W/fl1.log" && grep -q "^user exists" "$W/fl2.log" && grep -q "^credentials on the user: 0$" "$W/fl2.log"   && grep -q "UPDATE_PASSWORD" "$W/fl2.log" && grep -q "VERIFY_EMAIL" "$W/fl2.log"   && echo "PASS first-login: created once, idempotent, 0 credentials, both required actions"   || { echo "FAIL first-login"; rc=1; }
# The login page, rendered by Keycloak with the punchable theme (a public pk-web authorization request).
LOGIN=$(curl -s "http://127.0.0.1:$PORT/realms/$PK_REALM/protocol/openid-connect/auth?client_id=pk-web&response_type=code&scope=openid&redirect_uri=https%3A%2F%2F$PK_DOMAIN%2Fauth%2Fcallback&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM&code_challenge_method=S256")
[ -n "${THEME_HTML:-}" ] && printf '%s' "$LOGIN" > "$THEME_HTML"
if grep -q 'Continue with Google' <<<"$LOGIN" && grep -q 'id="kc-form-login"' <<<"$LOGIN" && grep -q 'pk-google' <<<"$LOGIN"    && ! grep -qi 'registrations' <<<"$LOGIN" && ! grep -qi 'FreeMarker template error' <<<"$LOGIN"; then
  echo "PASS theme: login page renders with the punchable theme (Continue with Google, email form, no sign-up link)"
else
  echo "FAIL theme: login page did not render with the punchable theme"; printf '%s
' "$LOGIN" | head -20; docker logs --tail 20 $T-kc 2>&1 | grep -i -E "freemarker|template|theme" ; rc=1
fi
set -e
if [ "$RED" = --red ]; then
  [ $rc -ne 0 ] && { echo "RED OK: the mutated realm failed the tests, as it must"; exit 0; }
  echo "!! RED run PASSED a mutated realm: the tests are not checking anything"; exit 1
fi
exit $rc
