# Hosting Punchable (MicroK8s)

How the hosted version runs: one server with MicroK8s, a Keycloak you already run (in its own namespace), Cloudflare
DNS, and images built on the server into the MicroK8s registry (`localhost:32000`). Nothing here is needed to run
Punchable on your own computer; see the top-level README for that.

## Setup
1. `cp infra/config.env.example ~/.punchable/infra.env` and fill it in (domain, auth host, namespaces, server IP, ssh
   target). `infra/config.env` (gitignored) also works; `PK_INFRA_ENV=<file>` picks another one.
2. `infra/ship.sh <step>` sends the committed tree and that config file to the server over ssh and runs
   `infra/deploy.sh <step>` there.

| Step | What it does |
|---|---|
| `test` | imports the realm into a throwaway Keycloak (plain Docker) and checks it, then checks it fails on a broken realm |
| `base` | namespace, Secrets (generated on the server, never in git), NetworkPolicies, SurrealDB, db-init, a network check |
| `google-idp` | "Sign in with Google": the OAuth client id/secret from `~/.punchable/google-oauth.env`, over ssh stdin, into a Secret |
| `keycloak` | the realm ConfigMap and its own import job (other realms are not touched) |
| `keycloak-ingress` | the `auth.<domain>` Ingress and certificate |
| `build api\|web\|worker` / `rollout api\|web\|worker` | build the images on the server, then roll out pinned to the digest |
| `ai-secret` | the `ANTHROPIC_API_KEY=` line of `~/.punchable/secrets.env`, over ssh stdin, into the Secret `punchable-anthropic` (the hosted worker's key) |
| `worker-secret` | the `PK_WORKER_SECRET=` line of the same file (the `pk-worker` client secret), over ssh stdin, into the Secret `punchable-worker` |
| `dbload [--check]` | pack your text/JSON data here (`db/push_remote.py`), load it into SurrealDB from a pod. Refused after the online-first cutover |
| `legal-rules` | your private `~/.punchable/legal_rules.json`, over ssh stdin, into the Secret the API's ingest routes read |
| `ingress` | the `<domain>` Ingress and certificate |
| `status` | what is live |
| `backup-secret` | the R2 token for the backup bucket, from `~/.punchable/r2.env`, over ssh stdin, into a Secret |
| `backup` / `backup-now` / `backup-list` | schedule the nightly SurrealDB backup (and the Bugsink SQLite copy); run one now; list what is in the bucket |
| `observability` | ServiceMonitors, alert rules and the Grafana dashboard (`OBSERVABILITY.md`); falls back to the plain Prometheus's ConfigMap without the operator |
| `bugsink-secret` / `bugsink` | the self-hosted crash-report store at `bugs.<domain>` (`k8s/70-bugsink.yaml`): its key (generated on the server) and first user, then the Deployment and Ingress |
| `sentry-secret` | the `SENTRY_DSN=` line of `~/.punchable/secrets.env` (the API's Bugsink DSN), over ssh stdin, into the Secret `punchable-sentry` |

`infra/ship.sh dns plan|apply|undo` runs locally against the Cloudflare API (`CF_API_TOKEN` in
`~/.punchable/cloudflare.env`) and only touches the records it created.

### The login theme

`keycloak/theme/login/` is the realm's login theme (`loginTheme: punchable`): a flat directory (template.ftl, the
page .ftl files, theme.properties, messages_en.properties; the CSS is inline in template.ftl). It is not a `ship.sh`
step: the shared Keycloak's Deployment mounts it from a ConfigMap (`keycloak-theme-punchable`, in the Keycloak
namespace) through its `setup-theme` init container into `/opt/keycloak/themes/punchable/login/` (messages go to
`login/messages/`). Updating it means recreating that ConfigMap from the directory and restarting the shared Keycloak,
which signs every realm out for the restart: an explicit go from the owner each time. Before shipping:
`python infra/keycloak/test_theme_static.py` (and `--red`), `python -m pytest infra/keycloak/test_realm_static.py -q`,
and `node infra/keycloak/theme_mockups.mjs` (PNGs of every page, phone and desktop, dark and light, no Keycloak needed).
`apply.sh` imports the realm without `loginTheme` until the Deployment has the `punchable-theme` volume.

## Online-first
The database on the server is the source of truth once you cut over; a worker posts results through `/api/worker/*`
(`api-dev/ingest_api.py`, `worker/online.py`). The order (one last `dbload`, then `db/push_remote.py cutover` in the
dbload pod's image) and the rollback are in your private notes. `legal-rules` must run before the first worker post,
so court material is refused on the server too. Uploads from the app wait in the API pod's `/tmp` (2 Gi); no
recording is kept on the server.

## The hosted worker (since 2026-10-27)

Every AI job runs on the server: `k8s/35-worker.yaml` (Deployment `punchable-worker`, image `infra/worker/Dockerfile`,
`python -m worker --hosted`) runs the same worker code the PC ran, online-first, against the in-cluster API Service
(`http://punchable-api`) with the `pk-worker` client credentials, and calls the Anthropic API per
`engine/config.hosted.toml` (Opus 5.5 for the writing, Sonnet 5.5 for conversation and grounding, Haiku 5.5 for
classification; adaptive thinking, streamed, the system prefix cached). It has no GPU and no audio pipeline: it never
claims `process_set` or `import_youtube`; those jobs stay queued as `waiting_for: "transcriber"` and the app says
"transcription happens on your phone for now; this file is waiting". A set transcribed on the phone comes in through
`POST /api/sets/transcript` and runs the text-only analysis (`set_text`). The PC worker is retired: `worker/README.md`
keeps it as the development mode only.

**Cost guard.** `PK_AI_DAILY_USD` (default `2.00`, set in `35-worker.yaml`) caps what the worker spends per UTC day,
counted from the SDK's usage numbers per job kind (`engine/budget.py`; the ledger is in the pod's `/tmp`, so a pod
restart starts the day's count again). Over the cap the worker keeps polling but takes no job; the reason shows in
Settings > Connections and under anything queued. Raise it with a new value in the manifest and `rollout worker`.

**Network.** `k8s/10-networkpolicy.yaml` `punchable-worker`: nothing in; out to the API pods on 8080 and to the public
internet on 443 (api.anthropic.com and the auth host for its token; Calico here has no name-based rule, so the rule
is "public, never a private range", as for the API and the backup). The API's ingress rule admits the worker's pods.
The worker never reaches SurrealDB.

**Liveness.** The worker touches `/tmp/pk-worker/heartbeat` on every poll and around every job; the probe restarts the
pod when the file is older than 20 minutes three checks in a row (a job is at most ~15 minutes, the API's stale-claim
window).

Deploy, in this order (the orchestrator runs these; the first two read `~/.punchable/secrets.env` and never show
the values):

```bash
infra/ship.sh worker-secret   # PK_WORKER_SECRET=  -> Secret punchable-worker   (the pk-worker client secret)
infra/ship.sh ai-secret       # ANTHROPIC_API_KEY= -> Secret punchable-anthropic
infra/ship.sh build api       # the API routes changed (waiting_for, claims, /api/sets/transcript)
infra/ship.sh rollout api
infra/ship.sh build worker    # downloads and checks the bge-small model at build (PK_WITH_BGE=0 to skip)
infra/ship.sh rollout worker  # applies the NetworkPolicies, then the Deployment pinned to the digest
infra/ship.sh build web && infra/ship.sh rollout web   # the app's copy ("the writer", the transcriber line)
```

Verify:

```bash
infra/ship.sh status                                   # punchable-worker 1/1, no restarts
microk8s kubectl -n punchable logs deploy/punchable-worker --tail=20
#   "hosted: Anthropic API · claude-opus-5-5, ... ; budget $2.00/day (spent $0.00); bge yes"
#   "worker up: http://punchable-api (auth), kinds [...]"   -- never process_set / import_youtube
curl -fsS -H "Authorization: Bearer $TOKEN" https://<domain>/api/connections | python -m json.tool
#   ai.worker: online true, where "the server", name "punchable-worker-...", model "Anthropic API · ..."
```
Then ask a question or a riff in the app: it answers from the server within a minute, and the worker's log shows the
job with its spend (`runs.jsonl` records `usd`, `cache_read`). The worker pod's own log never prints a key.

Rollback: `infra/ship.sh rollout worker <previous digest>` (from `~/projects/punchable-hosted/images.log`) or scale it
away (`microk8s kubectl -n punchable scale deploy/punchable-worker --replicas=0`): jobs then stay queued with the
"writer is offline" line, nothing is lost. The PC worker (`python -m worker --online`) still runs against the same API
if ever needed; it is not the plan.

## Files
| Path | What |
|---|---|
| `config.env.example` | the only place the hostnames, namespaces and IP are set |
| `render.sh` | fills `__PK_*__` placeholders in a manifest from `config.env`; refuses to output a leftover placeholder |
| `k8s/` | namespace, default-deny NetworkPolicies, SurrealDB StatefulSet, API, worker and web Deployments, Ingress |
| `worker/` | the hosted worker's image (`Dockerfile`, wheels only, the bge model baked in) |
| `keycloak/` | the `punchable` realm (Google login, PKCE, short tokens), its import job, the login theme, realm tests |
| `web/` | nginx image for the exported web app |
| `dns/cf_dns.py` | Cloudflare A records for the domain and the auth host |
| `backup/` | the nightly SurrealDB backup to R2 (`backup.py`, standard library only) and its tests |
| `secrets.sh`, `db-init.sh` | generate the in-cluster Secrets; create the SurrealDB namespace, database and users |

## Backups

A CronJob (`k8s/60-backup.yaml`, script `backup/backup.py`, Python standard library only) backs up SurrealDB every
night at 04:17 UTC:

1. streams `GET /export` for `punchable/main` from `surrealdb:8000`, gzips it to the pod's `/tmp`;
2. checks it: the whole gzip reads back (CRC and length), it starts with `OPTION IMPORT;`, has `DEFINE TABLE`, ends
   with a complete statement, and is not tiny;
3. uploads it to R2 with a signed `PUT` (SigV4), then checks the stored size with `HEAD`;
4. on Sundays (UTC) copies it to the weekly folder;
5. deletes old backups of the same database: it keeps the newest 14 daily and 8 weekly. Nothing outside the
   `surreal/` prefix, and nothing that is not exactly a backup name, is ever deleted. If any step before fails, nothing
   is deleted.

Keys: `surreal/daily/punchable-main-<yyyymmddTHHMMSSZ>.surql.gz` and `surreal/weekly/<same>`. A run exits non-zero with
one line on any failure; the failed Job stays visible (`kubectl -n punchable get jobs`).

**Who exports.** The database user `api` (EDITOR, Secret `punchable-api`), not root. A database user's export is
byte-identical to root's (checked 2026-10-26: root, EDITOR and VIEWER exports of the same database compared with
`cmp`), including the `DEFINE USER` lines. A dedicated VIEWER user would be tighter still; it needs `db-init.sh` to
create it (that script currently insists `api` is the only database user). SurrealDB 3.x only checks Basic credentials
against a database user when the request carries `Surreal-Auth-NS`/`Surreal-Auth-DB`; `backup.py` sends them
(`SURREAL_AUTH_LEVEL=database`).

**The bucket.** `R2_ENDPOINT`, `R2_BUCKET`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` in `~/.punchable/r2.env`: a token
that can only touch the backup bucket. `infra/ship.sh backup-secret` sends those four lines over ssh stdin into the
Secret `punchable-r2-backup`; they are never in git, argv or the output. The pod may reach SurrealDB and TCP 443 on the
public internet, nothing else (`k8s/10-networkpolicy.yaml`, `punchable-backup`).

Turn it on (after `base`):

```bash
infra/ship.sh backup-secret   # the R2 token -> Secret punchable-r2-backup
infra/ship.sh backup          # ConfigMap from backup.py, NetworkPolicies, the CronJob
infra/ship.sh backup-now      # one run now; prints its log
infra/ship.sh backup-list     # what is in the bucket
```

`backup.py` changes reach the cluster only through `infra/ship.sh backup` (the ConfigMap is made from the committed
file). Tests: `python -m pytest infra/backup -q`.

### Restore

Never restore straight into `main`. Prove the backup first:

1. **Download and check** (on your PC; the R2 settings come from the environment):
   ```bash
   set -a; . ~/.punchable/r2.env; set +a
   python infra/backup/backup.py list
   python infra/backup/backup.py fetch surreal/daily/punchable-main-<stamp>.surql.gz ./restore.surql.gz
   ```
   `fetch` compares the byte count with `HEAD` and reads the whole gzip; a cut or corrupted download is refused.
2. **Load it into a scratch SurrealDB** (3.3.4, in memory, a free port, your own throwaway root password):
   ```bash
   export SURREAL_USER=root SURREAL_PASS="$(openssl rand -hex 16)"   # the CLI reads both from the environment
   surreal start --bind 127.0.0.1:18797 memory &
   gunzip -k restore.surql.gz
   echo 'DEFINE NAMESPACE punchable; USE NS punchable; DEFINE DATABASE main STRICT;' \
     | surreal sql -e http://127.0.0.1:18797 --hide-welcome
   surreal import -e http://127.0.0.1:18797 --ns punchable --db main restore.surql
   ```
3. **Compare.** `INFO FOR DB;` on the scratch copy must list every table; for each table compare
   `SELECT count() FROM <table> GROUP ALL;` with the live database (or the counts the API reports). Check the
   `CHANGEFEED 30d INCLUDE ORIGINAL` clauses are on `decision`, `joke`, `performance`, `set`, `label`, `list` and
   `performed_as`, and that a full-text query (`unit.text @@ '<word>'`) and a KNN query (`embedding <|5,64|> $v`) answer.
4. **Then restore for real**, with the API stopped (`kubectl -n punchable scale deploy/punchable-api --replicas=0`) and
   the CronJob suspended (`kubectl -n punchable patch cronjob punchable-backup -p '{"spec":{"suspend":true}}'`).
   Load it into a check database inside the pod first (`DB=restore_check`), compare counts as in step 3, remove that
   database, then repeat with `DB=main`:
   ```bash
   K="microk8s kubectl -n punchable exec -i surrealdb-0 --"; DB=restore_check
   printf 'USE NS punchable; REMOVE DATABASE IF EXISTS %s; DEFINE DATABASE %s STRICT;\n' $DB $DB \
     | $K /surreal sql --endpoint http://127.0.0.1:8000 --hide-welcome
   $K /surreal import --endpoint http://127.0.0.1:8000 --ns punchable --db $DB /dev/stdin < restore.surql
   ```
   (the root login comes from the pod's own environment). The dump carries the `api` user; `infra/ship.sh base`
   re-sets its password from the Secret if they ever differ. Then scale the API back to 1 and unsuspend the CronJob.
   The import from `/dev/stdin` inside the pod has not been run on the server yet: do it on the check database first.

`surreal export ... -` run inside the pod is **not** a clean dump: the pod sets `SURREAL_LOG=info`, and the CLI writes
two log lines into stdout with the SurrealQL (first and last line). `backup.py` uses the HTTP endpoint and is not
affected; its end-of-statement check refuses such a file.

Keep the audio bucket versioned; it is not part of this backup.
