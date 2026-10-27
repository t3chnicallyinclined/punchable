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
| `build api\|web` / `rollout api\|web` | build the images on the server, then roll out pinned to the digest |
| `dbload [--check]` | pack your text/JSON data here (`db/push_remote.py`), load it into SurrealDB from a pod. Refused after the online-first cutover |
| `legal-rules` | your private `~/.punchable/legal_rules.json`, over ssh stdin, into the Secret the API's ingest routes read |
| `ingress` | the `<domain>` Ingress and certificate |
| `status` | what is live |
| `backup-secret` | the R2 token for the backup bucket, from `~/.punchable/r2.env`, over ssh stdin, into a Secret |
| `backup` / `backup-now` / `backup-list` | schedule the nightly SurrealDB backup; run one now; list what is in the bucket |

`infra/ship.sh dns plan|apply|undo` runs locally against the Cloudflare API (`CF_API_TOKEN` in
`~/.punchable/cloudflare.env`) and only touches the records it created.

## Online-first
The database on the server is the source of truth once you cut over; your PC becomes a compute worker that posts
results through `/api/worker/*` (`api-dev/ingest_api.py`, `worker/online.py`). The order (one last `dbload`, then
`db/push_remote.py cutover` in the dbload pod's image, then `PK_ONLINE_FIRST=1` for the worker and gsync) and the
rollback are in your private notes. `legal-rules` must run before the first worker post, so court material is
refused on the server too. Uploads from the app wait in the API pod's `/tmp` (2 Gi) until the worker has processed
them, then they are deleted; no recording is kept on the server.

## Files
| Path | What |
|---|---|
| `config.env.example` | the only place the hostnames, namespaces and IP are set |
| `render.sh` | fills `__PK_*__` placeholders in a manifest from `config.env`; refuses to output a leftover placeholder |
| `k8s/` | namespace, default-deny NetworkPolicies, SurrealDB StatefulSet, API and web Deployments, Ingress |
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
