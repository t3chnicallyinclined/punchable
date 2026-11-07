# T11: retire the Python (plan, 2026-11-07)

Status: **plan, not started.** Written by the architect on main `435d652` (T10 round 6b live). Nothing here is built
or deployed. The owner's decision (2026-11-07): port everything to Rust, deploy it in one go, then retire the Python.
This document says what is left, the engineer rounds that port it, how the proof survives once the Python is gone,
and how the cutover and the retirement run.

Tags: CONFIRMED = read in the code at `file:line` or seen in a read-only check of the live cluster on 2026-11-07;
INFERRED = follows from the code but has not been run. No host names, digests or material in this file (ADR 0006).

## 1. What still runs on Python in production

### 1.1 Requests the Python still answers

The public Ingress `punchable` (`infra/k8s/50-ingress.yaml:24-27`) sends every `/api` path to the Python Service
unless a more specific Ingress takes it. Live on 2026-11-07, three pk-api Ingresses take the paths pk-api owns
(`punchable-api-rs`, `-ids` and `-files`). `punchable-api-rs-upload` (`infra/k8s/prepared/t10-api-rs.yaml:363-383`)
is **not applied** (CONFIRMED, read-only `get ingress`). The prepared Ingresses list exactly pk-api's `OWN_PATHS`,
`OWN_WRITE_PATHS`, `OWN_PATTERNS` and `OWN_WRITE_PATTERNS` (`rust/crates/pk-api/src/lib.rs:54,105,136,179`). A script
compared the two lists and found no path missing from either side (CONFIRMED). Reaching the Python today:

| # | Request | Python code | Why it still goes there |
|---|---|---|---|
| R1 | `POST /api/upload` (the phone's set recording) | `api-dev/server.py:1749`, `upload` at `:2101` | Upload Ingress not applied; `PK_UPLOAD_DB` unset on both Deployments, so pk-api answers 503 (`writes/uploads.rs:266`). The controller log on 2026-11-07 showed 2 of these to the Python and nothing else (thin sample: the log only went back to 13:43Z). |
| R2 | `GET /api/inbox`, `GET /api/notes`, `GET /api/worker/jobs/<id>/file`, `GET /api/export` | the Python's inbox folder (`30-api.yaml:114`, `emptyDir`) | pk-api answers them only with `PK_UPLOAD_DB=1`; off, it **forwards** them (`writes/uploads.rs:433,446,489`, `routes/status.rs:264`). |
| R3 | `DELETE /api/me` (Settings: "delete my material"; the app calls it at `app/src/app/(tabs)/(rant)/settings.tsx:309`) | `api-dev/server.py:1858`, `delete_me` at `:1896`, `inventory` above it | Not ported. The inventory names the Python pod's folders, so this needs a small redesign over the database tables, not a straight port. |
| R4 | Any path pk-api does not route, and any method a moved path does not take | `server.py` `do_*` → `404 not found` | pk-api's fallback forwards them (`lib.rs:553`, `proxy.rs:54`). With `PK_PYTHON_URL` unset it answers the same 404 itself (`proxy.rs:55-58`). Whether every such answer is equal has not been tested yet (INFERRED). |
| R5 | `/metrics` of the Python | `server.py:1289` | Prometheus scrapes only `app=punchable-api` (`infra/observability/apply_prometheus.py:39-44`, an anchored `keep` regex). **pk-api is not scraped at all**, and the "API down" alert and the Grafana tile watch the Python only (`infra/k8s/60-observability.yaml:60`, `infra/grafana/punchable.json:25`). INFERRED: the cluster has no ServiceMonitor CRD (journal, 2026-11-07), so `apply_prometheus.py` is the scrape config in force. |

### 1.2 Work the Python pod does in the background

| # | Duty | Code | After T11 |
|---|---|---|---|
| B1 | Applies `db/schema_extra.surql` at start | `api-dev/surreal_kb.py:150` | pk-api with `PK_SCHEMA_OWNER=1` (`rust/crates/pk-api/src/state.rs:208-240`); live is `0` (CONFIRMED). |
| B2 | Holds `lease:embedder` and runs the vector sweep | `api-dev/search_api.py:210`; releases it on SIGTERM (`api/entry.py:258-268`) | pk-api's sweeper (`lib.rs` `serve`, `sweep.rs`) takes the lease when the Python stops. Memory with the model loaded was measured under the 1 GiB limit (M4b, peak 807 MiB under synthetic search load). |
| B3 | Live hub, file watcher, the files cache under `PK_DATA` (`materialize` / `refresh_files`) | `surreal_kb.py:66-67,168`; `api/entry.py:51,102,253` | Serve only the Python's own routes; they go with it. |
| B4 | Holds the upload inbox (`emptyDir`, wiped on restart) | `30-api.yaml:109-114` | Uploads in the database (ADR 0014), `PK_UPLOAD_DB=1`. |
| B5 | Mounts Secrets pk-api does not need: `punchable-r2`, `punchable-ai` | `30-api.yaml:73,75` | Gone with the Python: least privilege improves. INFERRED: no pk-api route needs either. |

### 1.3 Schema, admin and data duties

| # | Duty | Code | Runs as |
|---|---|---|---|
| S1 | `db/schema.surql`: tables, change feeds, the change-feed repair, the `meta:state` row | `db/load.py:163-200` (`apply_schema`) | Only through dbload (`db/push_remote.py:284`). pk-store already has the same function (`rust/crates/pk-store/src/schema.rs:106`), but only tests call it. |
| S2 | dbload: `pack` on the PC, `apply` in a one-off pod | `infra/ship.sh:96`; `infra/deploy.sh:114-145` (`pk-dbload`, the Python API image); `db/load.py:320` (`load`) | `apply` is refused since the online-first cutover (2026-10-27, `db/push_remote.py:286`). `sync` moved to pk-api in T10 round 6. |
| S3 | `cutover [--undo]`: the online-first flag in `meta:state` | `db/push_remote.py:332-356`; `infra/deploy.sh:146-165` (`pk-cutover`) | One-off pod from the Python image. |
| S4 | Nightly backup to object storage, plus `backup-list` | `infra/k8s/60-backup.yaml:47-48` (a `python:3.13-slim` CronJob running `infra/backup/backup.py`, 677 lines, standard library only); `infra/deploy.sh:249-263` | **Live gap:** the ConfigMap in the cluster predates the export that leaves uploads out. `upload_part` appears 0 times in the live script and once in the repo's (CONFIRMED). It must be remade before `PK_UPLOAD_DB=1`, or recordings land in every backup. |

### 1.4 Python on the build, CI and operations path (not a production process)

- Both Rust images fetch the search model in a Python stage (`rust/Dockerfile.api:39-46`,
  `rust/Dockerfile.worker:35-42`, `tools/kb/fetch_bge.py`).
- CI: `.github/workflows/api.yml:84` runs the Python suites (436 test functions across `api-dev`, `worker`,
  `creative`, `engine`, `infer` and `gsync`). `rust.yml:171-203` (`goldens-current`) regenerates every golden
  from the Python. `rust.yml:208-282` (`api-differential`) runs both differentials against the hosted Python.
- On the PC: `ship.sh` runs `push_remote.py pack`, `infra/dns/cf_dns.py` and `infra/observability/apply_prometheus.py`
  (`infra/ship.sh:38,96`; `deploy.sh:281`); `infra/test_k8s.py` tests the manifests. gsync (`gsync/`, a scheduled task on the PC)
  posts to the ingest routes, which pk-api already answers. The commit guard is Python too (`tools/guard/check.py`).
- Rollback copy of the Python worker: `infra/worker/python/35-worker.yaml` plus `worker/`.

### 1.5 The worker's legal rules (live gap)

`infra/k8s/35-worker.yaml` mounts no legal-rules Secret and sets no `PUNCHABLE_LEGAL_RULES` (env `:57-86`, volumes
`:105-110`). pk-worker then falls back to the generic markers **silently** (`rust/crates/pk-worker/src/main.rs:174`:
`map_or_else(|_| Legal::default(), ...)`), so it logs `legal_markers=14` where pk-api has 18. pk-api fixed the same
fault by refusing to start when the variable is set and the file cannot be read (`lib.rs` `load_legal`). The pod
already sets `fsGroup: 10001` (`35-worker.yaml:50`), so a 0440 Secret is readable once it is mounted.

## 2. The rounds

The proof for every round is the same three commands on a named machine, plus the round's own proof, which must
include a red arm (a deliberate break that turns it red):

```text
python rust/tools/diff_api_writes.py --pk-api <pk-api> --conc 4     (exit 0, 0 problems)
python rust/tools/diff_api_reads.py  --pk-api <pk-api>              (exit 0, outside ALLOW nothing)
cd rust && PK_REQUIRE_EXTERNAL=1 CARGO_BUILD_JOBS=2 cargo xtask gate
```

After round 7 the first two become the recorded conformance run (section 3). Each round ends with the engineer
removing their worktree's `rust/target` once the branch is merged (journal, 2026-11-07).

The order follows risk. Live gaps come first. Next are the rounds that answer the open question ("does anything
still need the Python?"). The data and safety-net rounds (dbload, backups) follow. The reference is frozen last,
because it must be recorded from the final Python. Nothing deploys before the cutover except round 1, which fixes
a live gap (owner question Q5).

| Round | What | Size | Its own proof |
|---|---|---|---|
| **1** | **Live gaps in the manifests and the worker.** (a) The worker's legal rules (section 1.5): mount `punchable-legal-rules` read-only in both worker manifests and set `PUNCHABLE_LEGAL_RULES`. Change `pk-worker/src/main.rs:174` to refuse to start when the variable is set and the file cannot be read or parsed (port pk-api's `load_legal`). (b) Have Prometheus scrape pk-api: a `punchable-api-rs` job in `apply_prometheus.py`, and the `up` / 5xx alerts and the Grafana tile for both jobs. | small | Worker unit test: variable set and file unreadable means no start, variable unset means generic markers (red arm: the old `map_or_else`). Render both worker manifests and check the mount (`infra/test_k8s.py` case). Deployed alone, the worker logs `legal_markers=18`, and Prometheus shows `punchable-api-rs` UP. |
| **2** | **pk-api stands alone.** (a) Port `DELETE /api/me` from `server.py:1858` and `:1868-1914` (`inventory`, `delete_me`): the same confirm phrase, the same "never deletes here" behaviour, and the deletion request recorded in the `account` log. The inventory lists database tables and counts rather than pod folders. That change of answer must be stated in the round's report. (b) The fallback with no Python: unknown paths and unsupported methods answer what the Python answers (404 `not found`; check what HEAD and OPTIONS do). (c) Every forward is logged and counted (`pk_api_forwarded_total{route}`), so production can show that zero requests reach the Python. (d) Both differentials gain a `--standalone` pass: the same matrix against a pk-api with `PK_PYTHON_URL` unset and `PK_UPLOAD_DB=1`. Every answer must equal the Python's, and the forward counter must stay at 0. | medium | The `--standalone` pass is the proof. Red arm: drop one route from the router and the pass fails on it. Differential case for `DELETE /api/me` (a wrong phrase is refused with the Python's status and text, the right phrase answers 202 `requested` and records one `account` row, and nothing is deleted). |
| **3** | **Schema owner and the admin command.** A `pk-api admin` subcommand (the image is distroless, so it is a subcommand of the same binary): `schema --check / --apply` (`pk_store::apply_schema`: the whole `schema.surql`, the change-feed repair, `meta:state`, both shas), `state` (prints `meta:state`: `online_first`, `loading`, `loading_by`, the shas), `cutover [--undo]` (a port of `push_remote.py:332-356`) and `recover-sync` (clears a dead sync's flag under the lease rules). `deploy.sh` `cutover` and a new `schema` step run it in a one-off pod from the pk-api image with the same Secret and NetworkPolicy label as `pk-dbload`. | small-medium | Differential: the Python `apply_schema` and `pk-api admin schema --apply` on two empty scratch databases give equal `INFO FOR DB` and equal `meta:state` shas. Red arm: drop one change feed and check that both repair it. A test that `PK_SCHEMA_OWNER=1` re-applies a changed `schema_extra` (one exists: `tests/worker.rs:1509`). The cutover and undo pair gives the Python's rows. |
| **4** | **dbload in Rust** (if the owner keeps it, Q1): `pk-api admin load <folder>` (a port of `db/load.py:320` `load` over `pk_store::kbmap` / `load_graph`), and `pack` / `apply` (a port of `push_remote.py` with the online-first refusal at `:286`, exit code 3). This round also covers what the Python-free conformance run needs: seeding a scratch database from the synthetic data folder, which today goes through `push_remote.py pack + apply`. | medium-large | Differential: the same synthetic folder loaded by the Python and by `pk-api admin load` into two scratch databases. Every table is dumped and must be equal as values (the `ord`, the ids, `meta:state` minus times). Red arm: perturb one unit's text. `apply` against an `online_first` database exits 3. |
| **5** | **Backups in Rust.** `pk-backup run / list / prune` (or `pk-api admin backup`): a port of `infra/backup/backup.py` as-is. It exports by table list with the three upload tables as definitions only and drops the second `OPTION IMPORT`, uploads with a bucket-scoped key, and keeps the same retention (14 daily, 8 weekly). It ships on its own distroless image. A new `60-backup.yaml` runs it beside the Python CronJob, writing under a separate prefix for the parallel nights (section 4, step 2). | medium | The restore drill from ADR 0014 against the Rust export: a scratch database, import, rows back, upload tables defined and empty. Compare against the Python export of the same scratch database: equal after the header. Retention test over a list of fake object names, against the Python's answer. Red arm: include `upload_part` and the drill fails. |
| **6** | **Python off the build and operations path.** The model fetch in the image builds: a verified download (the pinned hashes CI already checks, `BGE_MODEL_SHA256` / `BGE_TOKENIZER_SHA256`) in place of `fetch_bge.py`, with no Python stage. `infra/test_k8s.py` becomes an `xtask` check. `ship.sh`'s `dns`, the Prometheus apply and the JSON checks either become `xtask` subcommands or stay as named exceptions (Q4). | small-medium | Both images build in CI with no `python` stage. The image ids change, but the model files' hashes inside them are the same as before. The `xtask` manifest check fails on a manifest with a removed mount (red arm). |
| **7** | **Freeze the reference** (section 3). Record the conformance tapes from the final Python. Add the Rust replayer, turn the goldens into fixtures with `MANIFEST.sha256`, and tag `python-oracle-final` at the recorded commit. CI gains `conformance` beside `api-differential` (both run until the Python is deleted). | medium | The replayer passes on the recorded tapes. Red arms: edit one recorded answer (fails on that request); break one pk-api field (fails); change a fixture byte without updating `MANIFEST.sha256` (gate fails). |
| **8** | **The cutover manifests.** `infra/k8s/30-api.yaml` becomes pk-api (from the prepared file) with `PK_SCHEMA_OWNER=1` and `PK_UPLOAD_DB=1`, and without `PK_PYTHON_URL` (or with it, for step 4c of the cutover). The Python's manifest moves to `infra/api/python/30-api.yaml` for the rollback window, as the worker's did. `50-ingress.yaml` sends `/api` to `punchable-api-rs`. The `-ids`, `-files` and `-upload` Ingresses are deleted, which also removes the `use-regex` hazard. The pk-api NetworkPolicies move into `10-networkpolicy.yaml`, and the `app=punchable-api` rules go to the rollback file. `ship.sh` / `deploy.sh` / `render.sh` roll out pk-api as `api`, and the one-off pods use Rust images. `PK_WORKER_API` defaults to `http://punchable-api-rs`. | medium | `render.sh` of every manifest with test values, then `xtask` manifest check: no `__NAME__` left, no Ingress path pointing at `punchable-api`, and the NetworkPolicy lets the worker, Prometheus and the ingress host reach pk-api. A dry-run apply (`kubectl apply --dry-run=server`) of the rendered files is part of the SRE's pre-flight, not the engineer's proof. |
| **9** | **Delete the Python** (only after the rollback window closes, section 5). Delete `api/`, `api-dev/`, `worker/`, `engine/`, `creative/`, `infer/`, `db/*.py`, `infra/worker/python/`, `infra/api/python/`, `infra/backup/backup.py`, `rust/tools/gen_*.py`, `rust/tools/diff_api_*.py`, `api/Dockerfile`, `infra/worker/Dockerfile`, `.github/workflows/api.yml` and the `goldens-current` and `api-differential` jobs. gsync and the guard stay or go per Q3 / Q4. Update `docs/RUST.md`, `RUST_WORKSTREAM.md` (T11 row), `ARCHITECTURE.md`, `DEVELOPING.md` and `HANDOVER.md`. | medium (mostly deletion) | The gate and `conformance` stay green with the Python gone (they must not need it). `rg -l "python" .github rust infra` lists only what Q3 / Q4 kept. |

Conditional, per Q3: **round 10, gsync in Rust** (Google Docs and Drive, an OAuth refresh token, the same legal rules,
posting to the ingest routes it already uses). It can run after the cutover, because it talks to pk-api through
routes pk-api already answers.

## 3. The reference after the Python is gone

**Why the Python tests still exist (the owner's question).** There are three reasons, and each ends at a known point:
1. The Python still serves production (section 1): uploads, `DELETE /api/me`, the schema, the backups. Its tests
   guard code that is live today.
2. The Python is the reference the Rust is checked against. Every golden is written by running the Python
   (`rust.yml:187-203`), and both differentials run the real Python API beside pk-api. If the Python broke without a
   test noticing, the Rust would then be "proven equal" to a wrong answer. The Python tests keep the reference honest.
3. gsync and the backup job are Python, and they run.

**What replaces the live Python as the reference (decision).** The Python's answers are recorded once and frozen.
They are not kept alive:
- **Conformance tapes.** `diff_api_reads.py` and `diff_api_writes.py` gain `--record <dir>`. Over the synthetic data
  folder they save every request in order, the Python's answer, and the normalisation the differential already
  applies (ids, times, the per-server tag, the `ALLOW` list). Answers are stored as parsed JSON (ADR 0007). A Rust
  replayer (`cargo xtask conformance`) seeds a scratch SurrealDB with `pk-api admin load` (round 4), starts pk-api
  alone, plays the tapes in order and compares each answer. The two-writers and rows checks (phases D and F) do not
  need a Python: they run against pk-api's own rows. Phase B (writes straight to the Python) is dropped, because no
  second writer exists any more.
- **Goldens become fixtures.** `rust/crates/*/tests/goldens` stay as committed files, and `MANIFEST.sha256` lists every
  file's hash. The gate checks it, so a golden cannot change without the manifest changing in the same commit.
- **The tag.** `python-oracle-final` marks the commit the tapes were recorded from. Anyone can check out that tag to
  re-record or to settle a dispute. The tag costs nothing to keep.
- **When the answer should change.** After T11 a deliberate change of behaviour edits the tape in the same commit,
  with the reason in the commit message, as a golden change is reviewed today. The tape is the contract, and the
  Python no longer is.

Alternatives considered: keep a frozen Python image in CI as a permanent oracle. This keeps a second runtime, its
dependency advisories and 436 tests alive forever to guard code nobody changes, and any new route still has no
oracle. Rejected. Dropping the differentials and trusting the Rust unit tests was also considered. That loses the
only end-to-end check of the app-facing contract. Rejected.

**When the Python tests and CI go.**
- `goldens-current` and `api-differential` (`rust.yml`) stay until round 7 merges. From then `conformance` runs
  beside them, and both must stay green until the cutover.
- From the cutover, the Python code is frozen: no change to `api-dev/`, `db/`, `creative/`, `engine/` or `worker/`.
  The rollback image is a digest that already exists, not a rebuild. `api.yml` keeps running in that window only
  to show the frozen tree still passes. A failure in the window means the rollback image's source went bad, which
  is worth knowing.
- Round 9, when the rollback window closes, deletes `api.yml`, `goldens-current`, `api-differential` and the code
  together. gsync's and the guard's tests stay as long as those programs are Python (Q3, Q4).

## 4. The cutover (one deploy, the owner's go at each production step)

Preconditions: rounds 1-8 merged on main and CI green on the merge commit. `conformance`, both differentials and the
`--standalone` pass all pass. The pk-api, pk-worker and backup images come from that CI run, loaded by digest
(`docs/RUST_WORKSTREAM.md` "Deploying the images"). Use `infra/ship.sh rollout <name> <ref>`, which passes the ref
since `435d652` but has not yet been used in a live rollout, or `deploy.sh` on the node as the journal describes.
The private deploy log gets one line per step with its rollback.

1. **Backup, verified** (the usual pre-deploy step), plus `pk-api admin state`, which must show `loading=false`.
2. **Backups first.** Remake the backup ConfigMap from the repo (upload tables excluded), or apply the Rust
   backup CronJob beside the Python one. Run a backup at once and a restore drill on a scratch namespace before
   anything else changes. Q2 decides the parallel nights.
3. **Uploads.** Check that the Python's inbox is empty (`GET /api/inbox` = `[]`), because anything in the `emptyDir`
   is lost when the pod is replaced. Then roll out the Python with `PK_UPLOAD_DB=1` (the image already supports it),
   and pk-api with `PK_UPLOAD_DB=1`. Apply the upload route. Check a test upload end to end, from the phone (the
   owner) or with a scratch token, and confirm it shows in `/api/inbox` on both.
4. **pk-api takes the whole API.**
   a. Roll out pk-api (round 8 image) with `PK_SCHEMA_OWNER=1` and `PK_PYTHON_URL` **still set**: a safety net
      while the Ingress moves. `pk-api admin schema --check` must report no difference.
   b. Point `/api` in `50-ingress.yaml` at `punchable-api-rs` and delete the `-ids`, `-files` and `-upload`
      Ingresses. Watch `pk_api_forwarded_total`. Any forward means a request still needs the Python: stop and look.
   c. Once forwards have stayed at zero through one use of the app by the owner (Bits, a set, a jot, an upload,
      Settings), roll out pk-api without `PK_PYTHON_URL`.
   d. Scale the Python Deployment to 0. Its SIGTERM handler releases `lease:embedder` (`api/entry.py:258-268`), and
      pk-api's sweeper takes it. Watch pk-api's memory against the 850 MiB line in the journal.
5. **Check** with the owner's real use, and in Grafana (`punchable-api-rs` UP, 5xx at 0, p95 per route against the
   pre-cutover numbers). Write the deploy-log line.

The worker needs no step. It already talks to pk-api (`PK_WORKER_API=http://punchable-api-rs`, live).

## 5. Rollback window and retirement

**The window.** The Python Deployment stays at `replicas: 0`. Its Service, Secrets, NetworkPolicies and last image
digest also stay. Length: Q6 (recommended: 14 days of the owner's real use, including at least one gig night with
an upload).

**Rules while the window is open.** These keep the rollback valid:
- No change to `db/schema.surql` or `db/schema_extra.surql`. A rolled-back Python re-applies **its** copy of
  `schema_extra` at start (`surreal_kb.py:150`), and pk-api with `PK_SCHEMA_OWNER=1` re-applies its own whenever the
  stored sha differs. Two different copies would fight.
- No new job kind and no new table the Python does not know. The Python's job replay refuses an unknown kind and
  stops every creative route (journal gotcha 2026-11-07, `api-dev/creative_api.py:113`).
- New pk-api features that need either of these wait until the window closes.

**Rollback** (minutes, every step reversible):
1. Scale the Python to 1 and wait until it is Ready (it materialises from the database, as it does at every start).
2. Roll out pk-api with `PK_PYTHON_URL` set and `PK_SCHEMA_OWNER=0`.
3. Point `/api` back at `punchable-api` and re-apply the three pk-api Ingresses from the rollback manifests.
   Uploads stay in the database. Both servers read them there, so no recording is lost.
4. Deploy-log line. Then find out what went wrong before trying again.

**Retirement** (after the window, the owner's go):
1. Last check: `pk_api_forwarded_total` was 0 for the whole window (it should be, since no Python URL is set), and
   every nightly backup in the window passed.
2. Delete the Python Deployment, its Service and the `app=punchable-api` NetworkPolicies, and the Python backup
   CronJob if Q2 kept it running. Delete the Secrets only the Python mounted (`punchable-r2`, `punchable-ai`) after a
   check that nothing else mounts them. Remove the Python images from the cluster registry.
3. Round 9 merges: the code, CI and docs go. Tag `python-oracle-final` before the deletion commit (round 7 tagged
   the recording commit; this tag is the same commit or a later one with only frozen Python in between).
4. Close T11 in `docs/RUST_WORKSTREAM.md`.

## 6. How each part fails, and how the owner would find out

| Part | Fails how | Seen how |
|---|---|---|
| Worker legal rules | Secret unreadable | Round 1: the worker does not start (CrashLoop, alert), never quietly runs on 14 markers. |
| pk-api alone | A path the app uses was never ported | `pk_api_forwarded_total` > 0 at step 4b. After 4c, the app shows an error on that screen and pk-api logs the 404. The standalone differential is meant to catch this before deploy. |
| Uploads in the database | Disk fills (64 MiB per upload; the PVC is 5 GiB) | Q2b; SurrealDB disk alert (`db/DB.md` "Compaction"); pk-api's 7-day expiry keeps the queue bounded. |
| Backups | The Rust export misses a table | The restore drill in round 5 and at step 2; the parallel nights compare against the Python export. |
| Schema owner | Two writers of `schema_extra` | Only possible while the Python runs with a different copy, which the freeze rule prevents. pk-api logs "the database holds another build's tables" and uploads answer 503. |
| pk-api memory with the embedder always on it | OOM at 1 GiB | The journal's 850 MiB line; `OOMKilled` in the pod status; the new `punchable-api-rs` alert from round 1. |
| The reference after T11 | A tape edited to hide a regression | Tape edits show in review like golden edits; `MANIFEST.sha256` stops silent fixture edits. |

## 7. Open questions for the owner

Each answer changes the plan: Q1 sets round 4's size, Q2 round 5's parallel run and the upload home, Q3 round 10,
Q4 round 6's exceptions, Q5 whether round 1 ships alone, Q6 the window's length.

- **Q1.** The database is the source of truth since 2026-10-27, so the "load my PC files into the server" tool
  (dbload) has been switched off since then. Keep a Rust version of it (round 4 at full size), or drop it and rely
  on the nightly backups to rebuild the database? The tests need a small part of it either way.
- **Q2.** (a) For how many nights should the new backup job run beside the old one before the old one is
  removed? (b) Uploads will sit in the database for up to a week (64 MiB each, a 5 GiB disk): is that bound
  acceptable, or should they go to object storage instead?
- **Q3.** gsync, the program on your PC that pulls Google Docs into Punchable: is it still in use? If so, should it
  move to Rust as part of "everything", or later?
- **Q4.** Do "everything" and "no Python" cover the small PC-side tools (the commit guard, the DNS and dashboard
  scripts, the local analysis tools)? Or only what runs on the server and in CI?
- **Q5.** Round 1 fixes a gap that is live today: the server-side worker checks text against 14 generic court
  markers instead of your 18. May it ship on its own, ahead of the one-time cutover?
- **Q6.** How long should the old Python be kept, switched off but ready, after the cutover? Recommended: two weeks
  that include a gig night.
