# The Rust backup job (pk-backup): steps for the SRE

T11 round 7. `pk-backup` is a port as-is of `backup.py` (same commands, settings, object names, checks, retention and
exit status; see `rust/crates/pk-backup/src/lib.rs`). It runs **beside** the Python CronJob for 3 nights under its own
prefixes, then one of its backups is restored. Nothing here is applied by `infra/ship.sh`; every production step is
the owner's go, with its own line in the private deploy log.

What each step changes, and its rollback, is stated with the step. No bucket, host or key name is in this repo: the
Secret `punchable-r2-backup` is used by reference only.

## 1. The image

CI (`.github/workflows/rust.yml`, job `backup-image`) builds `rust/Dockerfile.backup` from the merge commit, checks the
user (10001) and that a bad command exits 2, and keeps `punchable-backup-rs-image-<sha>` for 14 days. Then, as the
other Rust images (`docs/RUST_WORKSTREAM.md` "Deploying the images"):

```bash
gh run download <run-id> -n punchable-backup-rs-image-<sha>          # check the sha256 against the job log
gunzip -c punchable-backup-rs.tar.gz | docker load                    # the loaded id equals the job log's `image id:`
docker tag punchable-backup-rs:ci <registry>/punchable-backup-rs:<sha>
docker push <registry>/punchable-backup-rs:<sha>
docker inspect --format '{{index .RepoDigests 0}}' <registry>/punchable-backup-rs:<sha>      # the digest to roll out
```

Roll out by digest, never a tag. Never build it on the production node.

## 2. The CronJob beside the Python one

Prerequisites: the Python backup already runs (`kubectl -n punchable get cronjob punchable-backup`), the Secret
`punchable-r2-backup` and the PVC `punchable-bugsink-data` exist (the same ones it uses).

```bash
bash infra/render.sh infra/k8s/prepared/t11-backup-rs.yaml <registry>/punchable-backup-rs@sha256:<digest> > /tmp/t11-backup-rs.yaml
kubectl -n punchable apply --server-side -f /tmp/t11-backup-rs.yaml        # the cluster tooling the other prepared manifests use
kubectl -n punchable get cronjob punchable-backup-rs
```

It runs at 04:47 UTC (the Python at 04:17) and writes only under `surreal-rs/` and `bugsink-rs/`, so neither job's
retention can see the other's objects. The existing NetworkPolicy `punchable-backup` covers its pods (the label
`app=punchable-backup`). The alerts: `PunchableBackupFailed` matches its Jobs by the name prefix `punchable-backup`,
and since 2026-11-17 `PunchableBackupMissed` names `punchable-backup-rs` too; both read kube-state-metrics, which this
cluster did NOT have until 2026-11-17 (`infra/observability/kube-state-metrics.yaml`, infra/OBSERVABILITY.md). Before that
date neither alert could fire for either CronJob, whatever the rule text said. **Rollback:** `kubectl -n punchable delete cronjob punchable-backup-rs`; the Python CronJob is not touched.

One run now, before waiting for the night (a Job from the CronJob; prints its log):

```bash
J=punchable-backup-rs-now-$(date -u +%Y%m%d%H%M%S)
kubectl -n punchable create job "$J" --from=cronjob/punchable-backup-rs
kubectl -n punchable wait --for=condition=complete "job/$J" --timeout=30m
kubectl -n punchable logs -l job-name="$J" --prefix --tail=-1
```

A good log reads like the Python's, line for line: `export ok: punchable/main <n> bytes -> <m> gz, <k> tables, <s>s`,
`uploaded surreal-rs/daily/punchable-main-<stamp>.surql.gz (<m> bytes, verified)`, (on Sundays UTC `weekly copy ...`),
`retention: kept <n> (14 daily / 8 weekly max), deleted <d>`, then `bugsink ok: ...` and `bugsink retention: ...`.
On failure the Job fails with one `backup: FAIL ...` line on stderr (secrets redacted) and exit status 1.

## 3. The three nights

Each morning, from a machine with the backup bucket token in its environment (the `R2_*` lines of the private env file;
never typed into a command), list both prefixes with the image that made them:

```bash
set -a; . ~/.punchable/r2.env; set +a
BACKUP_PREFIX=surreal/    pk-backup list      # or: python infra/backup/backup.py list
BACKUP_PREFIX=surreal-rs/ pk-backup list
```

A night **matches** when, for the same date:
- both Jobs succeeded (`kubectl -n punchable get jobs`);
- both prefixes hold a `daily/punchable-main-<stamp>.surql.gz` (and on a Sunday a `weekly/` copy), and the Bugsink
  prefixes a `daily/bugsink-db-<stamp>.sqlite3.gz`;
- the SQL sizes are within a few hundred bytes of each other when nothing was written in between (measured on the first
  live run: the gz sizes differ by about 0.8%, because the two gzip implementations compress differently, so compare the
  SQL, not the gz) (different gzip implementations; the SurrealQL inside is identical
  when nothing was written between the two runs, 30 minutes apart, so a few KB of difference after a busy evening is
  the writes, not the job);
- the count of `daily/` keys is equal until 14 nights exist, then stays at 14 in each prefix.

If a night does not match: do not apply step 5; keep the Python as the backup of record and report the two logs.

## 4. The restore drill (once, on a Rust-made backup)

`pk-backup` below is the binary built on a development machine, never on the node:
`cd rust && cargo build --release -p pk-backup` gives `rust/target/release/pk-backup`. `python infra/backup/backup.py`
takes the same commands and arguments, so either does `list`, `fetch` and `restore-check` here.

Never restore into `main`. This takes a Rust backup into a scratch database on the **scratch** server (or a second
database in the pod, `restore_check`) and compares every table with the live one. It is the procedure of
`infra/README.md` "Restore" steps 1 to 3, with the Rust binary doing the download, and a per-table row count.

1. **Download and check** (the check is the one the job makes before an upload: whole gzip read, CRC, length,
   `OPTION IMPORT`, `DEFINE TABLE`, ends with a complete statement):
   ```bash
   set -a; . ~/.punchable/r2.env; set +a
   BACKUP_PREFIX=surreal-rs/ pk-backup list
   BACKUP_PREFIX=surreal-rs/ pk-backup fetch surreal-rs/daily/punchable-main-<stamp>.surql.gz ./restore.surql.gz
   pk-backup restore-check ./restore.surql.gz
   ```
2. **Load it into a scratch SurrealDB 3.3.4** (in memory, a free port, your own throwaway root password):
   ```bash
   export SURREAL_USER=root SURREAL_PASS="$(openssl rand -hex 16)"
   surreal start --bind 127.0.0.1:18797 memory &
   gunzip -k restore.surql.gz
   echo 'DEFINE NAMESPACE punchable; USE NS punchable; DEFINE DATABASE main STRICT;' | surreal sql -e http://127.0.0.1:18797 --hide-welcome
   surreal import -e http://127.0.0.1:18797 --ns punchable --db main restore.surql
   ```
3. **Row counts per table, restored against live.** Tables of the live database (from `INFO FOR DB`) and the counts of
   each; the same two statements on both, one table per line:
   ```bash
   cat > counts.surql <<'EOF'
   INFO FOR DB;
   EOF
   # restored copy (the scratch server above); the live side is the same through the pod: see below
   surreal sql -e http://127.0.0.1:18797 --ns punchable --db main --hide-welcome --json < counts.surql | jq -r '.[0].tables | keys[]' > tables.txt
   while read t; do
     printf '%s ' "$t"
     echo "SELECT count() AS n FROM \`$t\` GROUP ALL;" | surreal sql -e http://127.0.0.1:18797 --ns punchable --db main --hide-welcome --json | jq -r '.[0][0].n // 0'
   done < tables.txt > restored.counts
   ```
   For the live side run the same loop inside the SurrealDB pod (`kubectl -n punchable exec -i surrealdb-0 -- /surreal sql
   --endpoint http://127.0.0.1:8000 --ns punchable --db main ...`; the root login comes from the pod's own environment),
   into `live.counts`, and `diff live.counts restored.counts`.

   **Expected:** the two files are equal line for line, **except** the three recording tables `upload`, `upload_part`
   and `upload_stage`, which are **defined** in the restored database (they appear in `tables.txt`) and **empty**
   (count 0): a backup carries their definitions and never their rows (ADR 0014). Any other difference is a failed
   drill: a table the export missed. A table present live and absent in `tables.txt` is the same failure.
4. **Definitions and indexes.** `INFO FOR DB;` on both sides must list the same tables with the same definition text
   (including `CHANGEFEED 30d INCLUDE ORIGINAL` on `decision`, `joke`, `performance`, `set`, `label`, `list`, `performed_as`, `creative_event`; SurrealDB prints
   the 30 days as `4w2d`);
   a full-text query (`unit.text @@ '<word>'`) and a KNN query (`embedding <|5,64|> $v`) must answer.
5. Stop the scratch server and delete `restore.surql*`, `*.counts`, `tables.txt`. Record in the deploy log: the backup
   key, the date, "N tables, counts equal, upload tables defined and empty".

What `pk-backup` automates of this, on a scratch server and a stand-in bucket, is `rust/crates/pk-backup/tests/differential.rs`
(the restore drill section): the Rust export imported into a scratch database, every table's count and definition equal to
the source, the upload tables defined and empty. The SRE's drill proves the same on the real bucket and the real data.

## 5. After three matching nights and one good drill

Not part of round 7; the owner's call and the cutover's step list (`docs/design/PLAN-2026-11-07-t11-retire-python.md`
section 4): the Rust CronJob takes the Python's schedule and prefix (or the Python is deleted), `PunchableBackupMissed`
learns the new CronJob name, and `infra/ship.sh backup` / `backup-now` / `backup-list` point at the Rust image.

## Settings

Environment only, the same names and defaults as `backup.py` (`infra/backup/backup.py` header):
`SURREAL_URL SURREAL_NS SURREAL_DB SURREAL_USER SURREAL_PASS SURREAL_AUTH_LEVEL`, `R2_ENDPOINT R2_BUCKET R2_ACCESS_KEY_ID
R2_SECRET_ACCESS_KEY R2_REGION`, `BACKUP_PREFIX BUGSINK_DB_PATH BUGSINK_PREFIX BACKUP_KEEP_DAILY BACKUP_KEEP_WEEKLY
BACKUP_WEEKLY BACKUP_MIN_BYTES BACKUP_TMP`. One addition: `SSL_CERT_FILE`, when set, adds the certificates in that file
to the trusted roots (for a test CA; the pod does not set it).
