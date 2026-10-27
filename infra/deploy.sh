#!/usr/bin/env bash
# Punchable steps ON the server, from the unpacked repo (infra/ship.sh sends `git archive HEAD` and calls this).
#   test                     realm test (+red) on a throwaway Keycloak; no k8s
#   base                     namespace, Secrets (by hand, generated here), NetworkPolicies, SurrealDB, db-init, netcheck
#   netcheck                 API-labelled pod reaches surrealdb:8000; a pod in another namespace TIMES OUT
#   google-idp               Secret the Keycloak namespace/punchable-google-idp from STDIN (ship.sh google-idp; before keycloak)
#   keycloak                 realm ConfigMap + its own import job (no Keycloak restart, other realms untouched)
#   keycloak-ingress         auth.<domain> Ingress + certificate (after DNS)
#   build api|web            docker build, push to localhost:32000, record the digest (no rollout)
#   rollout api|web [ref]    apply the Deployment pinned to a digest (default: last built)
#   dbload <bundle.tgz> [--check]   load Punchable's text/JSON data into SurrealDB from a pod (db/push_remote.py)
#   ingress                  punchable.io Ingress + certificate (after DNS and both rollouts)
#   backup-secret            Secret punchable-r2-backup from STDIN (ship.sh backup-secret; R2_* lines, never shown)
#   legal-rules              Secret punchable-legal-rules from STDIN (ship.sh legal-rules; the private legal_rules.json)
#   backup                   ConfigMap from infra/backup/backup.py + NetworkPolicies + the nightly CronJob
#   backup-now               one backup now (a Job from the CronJob), waits, prints its log
#   backup-list              the backups in the bucket (a one-off pod with the same image, Secret and policy)
#   status                   what is live
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
. infra/config.env
K="microk8s kubectl"; NS=$PK_NS
STATE="${STATE_DIR:-$HOME/projects/punchable-hosted}"; mkdir -p "$STATE"
R="bash infra/render.sh"
PYIMG=python:3.13-slim-bookworm@sha256:e32be0e55814a08977501ca863271d9bd02443710db8f2afaf8372e2c4508530  # = 60-backup.yaml
CURL=curlimages/curl@sha256:d9b4541e214bcd85196d6e92e2753ac6d0ea699f0af5741f8c6cccbfcf00ef4b
apply() { $R "$1" ${2:-} | $K apply --server-side --field-manager=punchable -f -; }
STEP="${1:?step}"; shift || true

netcheck() {
  echo "==> positive: an app=punchable-api pod reaches surrealdb:8000"
  $K -n $NS run pk-netcheck-ok --rm -i --quiet --restart=Never --labels=app=punchable-api --image=$CURL \
    --overrides='{"spec":{"securityContext":{"runAsNonRoot":true,"runAsUser":100,"seccompProfile":{"type":"RuntimeDefault"}},"containers":[{"name":"c","image":"'$CURL'","args":["-sS","-m","5","-o","/dev/null","-w","%{http_code}","http://surrealdb:8000/health"],"securityContext":{"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]}}}]}}' \
    | grep -q 200 && echo "PASS reach" || { echo "FAIL api pod cannot reach surrealdb"; return 1; }
  echo "==> negative: a pod in namespace default must TIME OUT on surrealdb.punchable:8000"
  OUT=$($K -n default run pk-netcheck-deny --rm -i --quiet --restart=Never --image=$CURL --command --        sh -c "curl -sS -m 5 -o /dev/null http://surrealdb.$NS.svc.cluster.local:8000/health 2>/dev/null; echo rc=\$?" || true)
  # 28 = curl's own timeout: the packets were dropped. Anything else (0 = reached, 6 = DNS, no rc = pod never ran)
  # is not proof of isolation.
  grep -q '^rc=28$' <<<"$OUT" || { echo "FAIL expected a timeout (rc=28), got: $OUT"; return 1; }
  echo "PASS denied"
}

case "$STEP" in
  test)
    bash infra/keycloak/test-realm.sh && bash infra/keycloak/test-realm.sh --red ;;
  base)
    apply infra/k8s/00-namespace.yaml
    bash infra/secrets.sh
    apply infra/k8s/10-networkpolicy.yaml
    apply infra/k8s/20-surrealdb.yaml
    $K -n $NS rollout status sts/surrealdb --timeout=5m
    bash infra/db-init.sh
    netcheck ;;
  netcheck) netcheck ;;
  google-idp)
    # STDIN = GOOGLE_CLIENT_ID=... / GOOGLE_CLIENT_SECRET=... lines (from ship.sh). Held in a variable and handed to
    # kubectl through printf (a builtin) and a pipe: never in argv, ps, a file on the host, or the output.
    ENVF="$(cat)"
    grep -qE '^GOOGLE_CLIENT_ID=.+' <<<"$ENVF" && grep -qE '^GOOGLE_CLIENT_SECRET=.+' <<<"$ENVF" \
      || { echo "!! expected GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET on stdin"; exit 1; }
    printf '%s\n' "$ENVF" | $K -n "$PK_KC_NS" create secret generic punchable-google-idp --from-env-file=/dev/stdin \
      --dry-run=client -o yaml | $K label --local -f - app.kubernetes.io/part-of=punchable -o yaml \
      | $K apply --server-side --field-manager=punchable -f - >/dev/null
    unset ENVF
    echo "punchable-google-idp applied (values not shown). Next: infra/ship.sh keycloak" ;;
  keycloak) bash infra/keycloak/apply.sh ;;
  keycloak-ingress) bash infra/keycloak/apply.sh ingress ;;
  build)
    WHAT="${1:?build api|web}"; SHA="${SHA:?SHA}"; TAG="$(date -u +%Y%m%d-%H%M%S)-${SHA:0:12}"
    REPO="localhost:32000/punchable-$WHAT"
    case "$WHAT" in
      web) DF=infra/web/Dockerfile; ARGS=(--build-arg "PK_DOMAIN=$PK_DOMAIN" --build-arg "PK_AUTH_HOST=$PK_AUTH_HOST") ;;
      api) DF="${API_DOCKERFILE:-api/Dockerfile}"; ARGS=()
           [ -f "$DF" ] || { echo "!! $DF not in this commit: the API image cannot be built yet"; exit 1; } ;;
      *) echo "build api|web"; exit 1 ;;
    esac
    nice -n 10 docker build -f "$DF" "${ARGS[@]}" -t "$REPO:$TAG" .
    docker push "$REPO:$TAG"
    REF=$(docker inspect --format '{{range .RepoDigests}}{{println .}}{{end}}' "$REPO:$TAG" | grep -E "^$REPO@sha256:[0-9a-f]{64}$" | head -1)
    [ -n "$REF" ] || { echo "!! no digest after push"; exit 1; }
    echo "$(date -u +%FT%TZ) $WHAT $REF sha=$SHA" >> "$STATE/images.log"
    echo "built $REF (not rolled out)" ;;
  rollout)
    WHAT="${1:?rollout api|web}"; REF="${2:-$(grep " $WHAT " "$STATE/images.log" | tail -1 | awk '{print $3}')}"
    [[ "$REF" =~ ^localhost:32000/punchable-$WHAT@sha256:[0-9a-f]{64}$ ]] || { echo "!! no pinned digest for $WHAT"; exit 1; }
    F=infra/k8s/30-api.yaml; [ "$WHAT" = web ] && F=infra/k8s/40-web.yaml
    [ "$WHAT" = api ] && { $K -n $NS get secret punchable-api >/dev/null || { echo "!! run base first"; exit 1; }; }
    apply "$F" "$REF"
    $K -n $NS rollout status deploy/punchable-$WHAT --timeout=5m
    echo "$(date -u +%FT%TZ) rollout $WHAT $REF" >> "$STATE/deploys.log"
    echo "rolled out $REF. Previous: $(grep "rollout $WHAT " "$STATE/deploys.log" | tail -2 | head -1)" ;;
  dbload)
    # The bundle was packed on the PC and copied here by infra/ship.sh dbload. The host cannot reach ClusterIPs, so
    # a pod (the API image, label app=punchable-dbload, NetworkPolicy: out to surrealdb:8000 only) reads it on stdin
    # and runs db/push_remote.py apply as the database user `api` (EDITOR). Idempotent; refuses (exit 3) when the
    # database has phone decisions the bundle lacks.
    B="${1:?dbload <bundle.tgz> [--check]}"; CHECK="${2:-}"
    [ -f "$B" ] || { echo "!! no bundle at $B"; exit 1; }
    [ -z "$CHECK" ] || [ "$CHECK" = --check ] || { echo "!! dbload <bundle.tgz> [--check]"; exit 1; }
    REF=$(grep " api " "$STATE/images.log" 2>/dev/null | tail -1 | awk '{print $3}')
    [[ "$REF" =~ ^localhost:32000/punchable-api@sha256:[0-9a-f]{64}$ ]] || { echo "!! no api image: infra/ship.sh build api first"; exit 1; }
    $K -n $NS get secret punchable-api >/dev/null || { echo "!! run base first"; exit 1; }
    apply infra/k8s/10-networkpolicy.yaml
    $K -n $NS delete pod pk-dbload --ignore-not-found --wait=true >/dev/null
    CMDJ='["python","/app/db/push_remote.py","apply","--tar","-"'"${CHECK:+,\"--check\"}"']'
    OV='{"spec":{"automountServiceAccountToken":false,"enableServiceLinks":false,
      "securityContext":{"runAsNonRoot":true,"runAsUser":10001,"runAsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},
      "containers":[{"name":"pk-dbload","image":"'$REF'","command":'$CMDJ',"stdin":true,"stdinOnce":true,"tty":false,
        "env":[{"name":"SURREAL_URL","value":"ws://surrealdb:8000/rpc"},{"name":"SURREAL_NS","value":"punchable"},
               {"name":"SURREAL_DB","value":"main"}],
        "envFrom":[{"secretRef":{"name":"punchable-api"}}],
        "resources":{"requests":{"cpu":"100m","memory":"256Mi"},"limits":{"cpu":"1","memory":"512Mi"}},
        "securityContext":{"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}},
        "volumeMounts":[{"name":"tmp","mountPath":"/tmp"}]}],
      "volumes":[{"name":"tmp","emptyDir":{"sizeLimit":"1Gi"}}]}}'
    set +e
    $K -n $NS run pk-dbload --rm -i --quiet --restart=Never --labels=app=punchable-dbload --image="$REF"       --overrides="$OV" < "$B"
    rc=$?
    set -e
    rm -f "$B"   # private material: never left on the host (ship.sh packs a fresh one each time)
    [ -z "$CHECK" ] && [ $rc -eq 0 ] && echo "$(date -u +%FT%TZ) dbload ok" >> "$STATE/deploys.log"
    [ $rc -eq 0 ] || echo "!! dbload exited $rc (3 = refused: run the home worker once, then infra/ship.sh dbload again)"
    exit $rc ;;
  ingress)
    getent hosts "$PK_DOMAIN" | grep -q "$PK_ORIGIN_IP" || { echo "!! $PK_DOMAIN does not resolve to $PK_ORIGIN_IP"; exit 1; }
    for d in punchable-web punchable-api; do
      [ "$($K -n $NS get deploy $d -o jsonpath='{.status.availableReplicas}' 2>/dev/null)" = 1 ] || { echo "!! $d not available"; exit 1; }
    done
    apply infra/k8s/50-ingress.yaml
    $K -n $NS wait --for=condition=Ready certificate/punchable-tls --timeout=5m
    curl -fsS -o /dev/null -w "https://$PK_DOMAIN/ %{http_code}\n" "https://$PK_DOMAIN/"
    curl -fsS -o /dev/null -w "https://$PK_DOMAIN/api/health %{http_code}\n" "https://$PK_DOMAIN/api/health" ;;
  legal-rules)
    # STDIN = ~/.punchable/legal_rules.json (from ship.sh legal-rules): the court-material rules every ingest route
    # applies. Private terms: a variable, printf, a pipe into the Secret; never argv, a file on the host, or the output.
    RULES="$(cat)"
    python3 -c 'import json,sys; json.loads(sys.stdin.read())' <<<"$RULES" || { echo "!! legal rules on stdin are not JSON"; exit 1; }
    printf '%s' "$RULES" | $K -n $NS create secret generic punchable-legal-rules --from-file=legal_rules.json=/dev/stdin \
      --dry-run=client -o yaml | $K label --local -f - app.kubernetes.io/part-of=punchable -o yaml \
      | $K apply --server-side --field-manager=punchable -f - >/dev/null
    unset RULES
    echo "punchable-legal-rules applied (not shown). The API reads it on its next start (infra/ship.sh rollout api)." ;;
  backup-secret)
    # STDIN = R2_ENDPOINT / R2_BUCKET / R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY lines (from ship.sh backup-secret): a
    # token scoped to the backup bucket only. Same path as google-idp: a variable, printf, a pipe; never argv or a file.
    ENVF="$(cat)"
    for k in R2_ENDPOINT R2_BUCKET R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY; do
      grep -qE "^$k=.+" <<<"$ENVF" || { echo "!! expected $k on stdin"; exit 1; }
    done
    printf '%s\n' "$ENVF" | $K -n $NS create secret generic punchable-r2-backup --from-env-file=/dev/stdin \
      --dry-run=client -o yaml | $K label --local -f - app.kubernetes.io/part-of=punchable -o yaml \
      | $K apply --server-side --field-manager=punchable -f - >/dev/null
    unset ENVF
    echo "punchable-r2-backup applied (values not shown). Next: infra/ship.sh backup" ;;
  backup)
    $K -n $NS get secret punchable-api >/dev/null || { echo "!! run base first"; exit 1; }
    $K -n $NS get secret punchable-r2-backup >/dev/null || { echo "!! run infra/ship.sh backup-secret first"; exit 1; }
    $K -n $NS create configmap punchable-backup-script --from-file=backup.py=infra/backup/backup.py \
      --dry-run=client -o yaml | $K label --local -f - app=punchable-backup app.kubernetes.io/part-of=punchable -o yaml \
      | $K apply --server-side --field-manager=punchable -f - >/dev/null
    apply infra/k8s/10-networkpolicy.yaml
    apply infra/k8s/60-backup.yaml
    $K -n $NS get cronjob punchable-backup
    echo "$(date -u +%FT%TZ) backup cronjob sha=${SHA:-?}" >> "$STATE/deploys.log"
    echo "nightly backup scheduled. Run one now: infra/ship.sh backup-now" ;;
  backup-now)
    $K -n $NS get cronjob punchable-backup >/dev/null || { echo "!! run infra/ship.sh backup first"; exit 1; }
    J="punchable-backup-now-$(date -u +%Y%m%d%H%M%S)"
    $K -n $NS create job "$J" --from=cronjob/punchable-backup >/dev/null
    echo "==> $J (waits up to 30 min)"
    ok=""
    for _ in $(seq 1 360); do
      S="$($K -n $NS get job "$J" -o jsonpath='{.status.succeeded}/{.status.failed}')"
      [ "${S%%/*}" = 1 ] && { ok=1; break; }
      [ "${S##*/}" -ge 3 ] 2>/dev/null && break   # backoffLimit 2 = three failed pods
      $K -n $NS get job "$J" -o jsonpath='{.status.conditions[?(@.type=="Failed")].status}' | grep -q True && break
      sleep 5
    done
    $K -n $NS logs -l job-name="$J" --prefix --tail=-1 || true
    [ -n "$ok" ] || { echo "!! $J did not succeed"; exit 1; }
    echo "$(date -u +%FT%TZ) backup-now ok $J" >> "$STATE/deploys.log" ;;
  backup-list)
    $K -n $NS get secret punchable-r2-backup >/dev/null || { echo "!! run infra/ship.sh backup-secret first"; exit 1; }
    $K -n $NS get configmap punchable-backup-script >/dev/null || { echo "!! run infra/ship.sh backup first"; exit 1; }
    $K -n $NS delete pod pk-backup-list --ignore-not-found --wait=true >/dev/null
    OV='{"spec":{"automountServiceAccountToken":false,"enableServiceLinks":false,
      "securityContext":{"runAsNonRoot":true,"runAsUser":10001,"runAsGroup":10001,"seccompProfile":{"type":"RuntimeDefault"}},
      "containers":[{"name":"pk-backup-list","image":"'$PYIMG'","command":["python","-I","/opt/backup/backup.py","list"],
        "stdin":true,"stdinOnce":true,"tty":false,
        "env":[{"name":"BACKUP_PREFIX","value":"surreal/"},{"name":"PYTHONDONTWRITEBYTECODE","value":"1"}],
        "envFrom":[{"secretRef":{"name":"punchable-r2-backup"}}],
        "resources":{"requests":{"cpu":"50m","memory":"64Mi"},"limits":{"cpu":"500m","memory":"128Mi"}},
        "securityContext":{"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}},
        "volumeMounts":[{"name":"script","mountPath":"/opt/backup","readOnly":true}]}],
      "volumes":[{"name":"script","configMap":{"name":"punchable-backup-script"}}]}}'
    $K -n $NS run pk-backup-list --rm -i --quiet --restart=Never --labels=app=punchable-backup --image="$PYIMG" \
      --overrides="$OV" </dev/null ;;
  status)
    $K -n $NS get all,netpol,pvc,secret,cronjob -o wide
    $K -n "$PK_KC_NS" get cm,job,ingress -l app.kubernetes.io/part-of=punchable 2>/dev/null
    $K -n "$PK_KC_NS" get cm keycloak-realm-punchable 2>/dev/null || true
    $K describe node | sed -n '/Allocated resources/,/Events/p' | head -6 ;;
  *) echo "unknown step $STEP"; exit 1 ;;
esac
