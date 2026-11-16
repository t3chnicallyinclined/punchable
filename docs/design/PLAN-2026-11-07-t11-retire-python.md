# T11: retire the Python (plan, 2026-11-07)

Status: **plan; round 1 in progress** (branch `rust/t11-round1`). Written by the architect on main `435d652` (T10
round 6b live), revised the same day with the owner's answers (section 7). The owner's decision (2026-11-07): port
everything to Rust, deploy it in one go, then retire the Python. Two things ship ahead of the one-time cutover, both
by the owner's or the lead's answer: round 1 (the worker's legal rules) and the Rust backup job's three parallel
nights. This document says what is left, the engineer rounds that port it, how the proof survives once the Python
is gone, and how the cutover and the retirement run.

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
| R3 | `DELETE /api/me` (Settings: "delete my material"; the app calls it at `app/src/app/(tabs)/(rant)/settings.tsx:309`) | `api-dev/server.py:1858`, `inventory` at `:1868`, `delete_me` at `:1896` | Not ported. The inventory names the Python pod's folders, so this needs a small redesign over the database tables, not a straight port. |
| R4 | Any path pk-api does not route, and any method a moved path does not take | `server.py` `do_*` → `404 not found` | pk-api's fallback forwards them (`lib.rs:553`, `proxy.rs:54`). With `PK_PYTHON_URL` unset it answers the same 404 itself (`proxy.rs:55-58`). Whether every such answer is equal has not been tested yet (INFERRED). |
| R5 | `/metrics` of the Python | `server.py:1289` | Prometheus scrapes only `app=punchable-api` (`infra/observability/apply_prometheus.py:39-44`, an anchored `keep` regex). **pk-api is not scraped at all**, and the "API down" alert and the Grafana tile watch the Python only (`infra/k8s/60-observability.yaml:60`, `infra/grafana/punchable.json:25`). INFERRED: the cluster has no ServiceMonitor CRD (journal, 2026-11-07), so `apply_prometheus.py` is the scrape config in force. |

### 1.2 Work the Python pod does in the background

| # | Duty | Code | After T11 |
|---|---|---|---|
| B1 | Applies `db/schema_extra.surql` at start | `api-dev/surreal_kb.py:150` | pk-api with `PK_SCHEMA_OWNER=1` (`rust/crates/pk-api/src/state.rs:208-240`); live is `0` (CONFIRMED). |
| B2 | Holds `lease:embedder` and runs the vector sweep | `api-dev/search_api.py:210`; releases it on SIGTERM (`api/entry.py:258-268`) | pk-api's sweeper (`lib.rs` `serve`, `sweep.rs`) takes the lease when the Python stops. Memory with the model loaded was measured under the 1 GiB limit (M4b, peak 807 MiB under synthetic search load). |
| B3 | Live hub, file watcher, the files cache under `PK_DATA` (`materialize` / `refresh_files`) | `surreal_kb.py:66-67,168`; `api/entry.py:51,102,253` | Serve only the Python's own routes; they go with it. |
| B4 | Holds the upload inbox (`emptyDir`, wiped on restart) | `30-api.yaml:109-114` | Recordings in object storage (R2), round 2. |
| B5 | Mounts Secrets pk-api does not need today: `punchable-r2`, `punchable-ai` | `30-api.yaml:73,75` | `punchable-ai` goes with the Python. Round 2 decides whether the recordings use `punchable-r2` (the audio bucket's token) or a new bucket-scoped Secret. INFERRED: no other pk-api route needs either. |

### 1.3 Schema, admin and data duties

| # | Duty | Code | Runs as |
|---|---|---|---|
| S1 | `db/schema.surql`: tables, change feeds, the change-feed repair, the `meta:state` row | `db/load.py:163-200` (`apply_schema`) | Only through dbload (`db/push_remote.py:284`). pk-store already has the same function (`rust/crates/pk-store/src/schema.rs:106`), but only tests call it. |
| S2 | dbload: `pack` on the PC, `apply` in a one-off pod | `infra/ship.sh:96`; `infra/deploy.sh:114-145` (`pk-dbload`, the Python API image); `db/load.py:320` (`load`) | `apply` is refused since the online-first cutover (2026-10-27, `db/push_remote.py:286`). `sync` moved to pk-api in T10 round 6. **Owner, 2026-11-07: dropped**; the nightly backups are the way to rebuild the database. |
| S3 | `cutover [--undo]`: the online-first flag in `meta:state` | `db/push_remote.py:332-356`; `infra/deploy.sh:146-165` (`pk-cutover`) | One-off pod from the Python image. |
| S4 | Nightly backup to object storage, plus `backup-list` | `infra/k8s/60-backup.yaml:47-48` (a `python:3.13-slim` CronJob running `infra/backup/backup.py`, 677 lines, standard library only, its own bucket and token, `infra/README.md` "The bucket"); `infra/deploy.sh:249-263` | The ConfigMap in the cluster predates the export that leaves the upload tables out (`upload_part` appears 0 times in the live script, once in the repo's; CONFIRMED). With recordings in R2 (round 2) this no longer matters: recordings never enter the database. See round 7. |

### 1.4 Python on the build, CI and operations path

- Both Rust images fetch the search model in a Python stage (`rust/Dockerfile.api:39-46`,
  `rust/Dockerfile.worker:35-42`, `tools/kb/fetch_bge.py`).
- CI: `.github/workflows/api.yml:84` runs the Python suites (436 test functions across `api-dev`, `worker`,
  `creative`, `engine`, `infer` and `gsync`). `rust.yml:171-203` (`goldens-current`) regenerates every golden
  from the Python. `rust.yml:208-282` (`api-differential`) runs both differentials against the hosted Python.
- On the PC: `ship.sh` runs `push_remote.py pack`, `infra/dns/cf_dns.py` and `infra/observability/apply_prometheus.py`
  (`infra/ship.sh:38,96`; `deploy.sh:281`); `infra/test_k8s.py` tests the manifests; the commit guard is
  `tools/guard/check.py`.
- **gsync** (`gsync/`, 2,889 lines, a scheduled task on the PC, `gsync/install_task.ps1`): Google Docs and Drive to
  Punchable and back, posting through the worker routes pk-api already answers (`gsync/punchable.py:167-235`).
  Still in use (owner, 2026-11-07).
- Rollback copy of the Python worker: `infra/worker/python/35-worker.yaml` plus `worker/`.

**Scope (lead's default, 2026-11-07).** "No Python" means the server and CI. gsync is in T11 by the owner's word.
Other tools that only run on the PC (the commit guard, the DNS and dashboard scripts, the local analysis tools)
come later and stay Python for now. They are named, not forgotten.

### 1.5 The worker's legal rules (live gap; round 1, in progress)

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

After round 9 the first two become the recorded conformance run (section 3). Each round ends with the engineer
removing their worktree's `rust/target` once the branch is merged (journal, 2026-11-07).

The order follows risk:
- The live gap comes first.
- The new design (recordings in R2) comes next, because it is the one thing with no Python to copy and it changes
  what the standalone check must cover.
- Then the rounds that answer "does anything still need the Python?", then gsync, which writes into his Google Docs.
- The safety net (backups) follows; its parallel nights must start before the cutover.
- The reference is frozen last, because it must be recorded from the final Python.

| Round | What | Size | Its own proof |
|---|---|---|---|
| **1** (in progress, `rust/t11-round1`; ships alone, owner Q5) | **The worker's legal rules and pk-api monitoring.** (a) Mount `punchable-legal-rules` read-only in both worker manifests and set `PUNCHABLE_LEGAL_RULES`. Change `pk-worker/src/main.rs:174` to refuse to start when the variable is set and the file cannot be read or parsed (port pk-api's `load_legal`). (b) Have Prometheus scrape pk-api: a `punchable-api-rs` job in `apply_prometheus.py`, and the `up` / 5xx alerts and the Grafana tile for both jobs. | small | Worker unit test: variable set and file unreadable means no start, variable unset means generic markers (red arm: the old `map_or_else`). Render both worker manifests and check the mount (`infra/test_k8s.py` case). Deployed: the worker logs `legal_markers=18`, and Prometheus shows `punchable-api-rs` UP. |
| **2** | **Recordings in R2** (owner Q2b; design in section 2.1). pk-api streams `POST /api/upload` into an R2 bucket and keeps only a pointer row. It streams the object back on `GET /api/worker/jobs/<id>/file`. The 7-day expiry deletes the object, and R2 lifecycle rules back that up. ADR 0015 records this and supersedes ADR 0014's storage choice (the "never half an upload" and release rules carry over). pk-api's database home for uploads (`upload_part`, `upload_stage` writes) is removed. | medium | The writes differential's upload cases: pk-api in R2 mode against the Python in its file mode. The app-facing answers must be equal (the upload's answer, `/api/inbox`, `/api/notes`, `/api/today` `inbox`, the job, the worker's fetch byte for byte). The tests run a local S3-compatible server (dev-dependency only). Red arms: kill the stream mid-upload, which must leave no listed upload, no pointer row and an aborted multipart; skip the delete in `release_for_job` and the test sees the object remain. pk-api memory during a 500 MiB upload stays within one part buffer per upload (measured, machine named). |
| **3** | **pk-api stands alone.** (a) Port `DELETE /api/me` from `server.py:1858` and `:1868-1914` (`inventory`, `delete_me`): the same confirm phrase, the same "never deletes here" behaviour, and the deletion request recorded in the `account` log. The inventory lists database tables and counts rather than pod folders, plus the recordings waiting in R2. That change of answer must be stated in the round's report. (b) The fallback with no Python: unknown paths and unsupported methods answer what the Python answers (404 `not found`; check what HEAD and OPTIONS do). (c) Every forward is logged and counted (`pk_api_forwarded_total{route}`), so production can show that zero requests reach the Python. (d) Both differentials gain a `--standalone` pass: the same matrix against a pk-api with `PK_PYTHON_URL` unset and recordings in R2. Every answer must equal the Python's, and the forward counter must stay at 0. | medium | The `--standalone` pass is the proof. Red arm: drop one route from the router and the pass fails on it. Differential case for `DELETE /api/me` (a wrong phrase is refused with the Python's status and text, the right phrase answers 202 `requested` and records one `account` row, and nothing is deleted). |
| **4** | **gsync in Rust** (owner Q3, before the cutover). A `pk-gsync` crate and binary, ported as-is from `gsync/`: the same Google Docs and Drive calls, the same OAuth refresh-token file, the same legal rules, the same worker routes, the same state files. It keeps running where it runs today, as a scheduled task on the PC (`install_task.ps1` points at the binary). Where it should run later is a separate question (section 7, note N1). | medium-large | Goldens: `gsync/tests/fakes.py` drives the Python over fake Google and fake Punchable APIs, and the generator records every request and the resulting doc edits. The Rust replays the same scenarios and must make the same requests and edits. Red arm: change one rule in the legal file and a blocked line must pass through. A read-only dry run against a **throwaway copy** of one Google Doc, never his real Docs (team principle 16), plans the same changes on both. |
| **5** | **Schema owner and the admin command.** A `pk-api admin` subcommand (the image is distroless, so it is a subcommand of the same binary): `schema --check / --apply` (`pk_store::apply_schema`: the whole `schema.surql`, the change-feed repair, `meta:state`, both shas), `state` (prints `meta:state`: `online_first`, `loading`, `loading_by`, the shas), `cutover [--undo]` (a port of `push_remote.py:332-356`) and `recover-sync` (clears a dead sync's flag under the lease rules). `deploy.sh` `cutover` and a new `schema` step run it in a one-off pod from the pk-api image with the same Secret and NetworkPolicy label as `pk-dbload`. | small-medium | Differential: the Python `apply_schema` and `pk-api admin schema --apply` on two empty scratch databases give equal `INFO FOR DB` and equal `meta:state` shas. Red arm: drop one change feed and check that both repair it. A test that `PK_SCHEMA_OWNER=1` re-applies a changed `schema_extra` (one exists: `tests/worker.rs:1509`). The cutover and undo pair gives the Python's rows. |
| **6** | **A scratch database without the Python** (owner Q1: the PC loader is dropped; only this test part stays). `pk-api admin seed <folder>` fills an empty database from the synthetic data folder the differentials and tests use, over `pk_store::kbmap` / `load_graph`. It runs only against a database whose `meta:state` is empty or marked scratch, and refuses anything else. No `pack`, no `apply`, no PC bundle. `deploy.sh dbload` and `ship.sh dbload` are removed. | small-medium | Differential: the same synthetic folder loaded by `push_remote.py pack + apply` and by `pk-api admin seed` into two scratch databases. Every table is dumped and must be equal as values (the `ord`, the ids, `meta:state` minus times). Red arm: perturb one unit's text. `seed` against a database with real `meta:state` refuses. |
| **7** | **Backups in Rust.** `pk-backup run / list` (or `pk-api admin backup`): a port of `infra/backup/backup.py` as-is: export by table list with the upload tables as definitions only, drop the second `OPTION IMPORT`, signed upload to the backup bucket with its own bucket-scoped token, and the same retention (14 daily, 8 weekly). It ships on its own distroless image. A new `60-backup.yaml` runs it beside the Python CronJob for **3 nights** (lead's default), writing under a separate prefix. **What round 2 changes here:** recordings never enter SurrealDB, so the stale live ConfigMap no longer puts recordings into backups, and remaking it stops being a precondition for the cutover. The upload tables stay excluded in the Rust job because they still exist and the exclusion costs nothing. The recordings bucket is never read by the backup job, and its token cannot read the backup bucket. | medium | The restore drill from ADR 0014 against the Rust export: a scratch database, import, rows back, upload tables defined and empty. Compare against the Python export of the same scratch database: equal after the header. Retention test over a list of fake object names, against the Python's answer. Red arm: include `upload_part` and the drill fails. In production: three nights of both jobs, sizes within a few bytes, one restore drill of a Rust backup. |
| **8** | **Python off the build and operations path.** The model fetch in the image builds: a verified download (the pinned hashes CI already checks, `BGE_MODEL_SHA256` / `BGE_TOKENIZER_SHA256`) in place of `fetch_bge.py`, with no Python stage. `infra/test_k8s.py` becomes an `xtask` check. The server-side steps of `ship.sh` / `deploy.sh` that call Python (`deploy.sh:179` JSON check, `:269` dashboard JSON check, `:281` the Prometheus apply) become `xtask` subcommands or shell. The PC-only scripts (`cf_dns.py`, the guard) stay Python for now (scope, 1.4). | small-medium | Both images build in CI with no `python` stage. The image ids change, but the model files' hashes inside them are the same as before. The `xtask` manifest check fails on a manifest with a removed mount (red arm). |
| **9** | **Freeze the reference** (section 3). Record the conformance tapes from the final Python. Add the Rust replayer, turn the goldens into fixtures with `MANIFEST.sha256`, and tag `python-oracle-final` at the recorded commit. gsync's recorded scenarios from round 4 join the tapes. CI gains `conformance` beside `api-differential` (both run until the Python is deleted). | medium | The replayer passes on the recorded tapes. Red arms: edit one recorded answer (fails on that request); break one pk-api field (fails); change a fixture byte without updating `MANIFEST.sha256` (gate fails). |
| **10** | **The cutover manifests.** `infra/k8s/30-api.yaml` becomes pk-api (from the prepared file) with `PK_SCHEMA_OWNER=1`, the recordings bucket settings, and no `PK_PYTHON_URL` (or with it, for step 4a of the cutover). The Python's manifest moves to `infra/api/python/30-api.yaml` for the rollback window, as the worker's did. `50-ingress.yaml` sends `/api` to `punchable-api-rs`. The `-ids`, `-files` and `-upload` Ingresses are deleted, which also removes the `use-regex` hazard. The pk-api NetworkPolicies move into `10-networkpolicy.yaml`, with egress to TCP 443 for R2, and the `app=punchable-api` rules go to the rollback file. `ship.sh` / `deploy.sh` / `render.sh` roll out pk-api as `api`, and the one-off pods use Rust images. `PK_WORKER_API` defaults to `http://punchable-api-rs`. | medium | `render.sh` of every manifest with test values, then the `xtask` manifest check: no `__NAME__` left, no Ingress path pointing at `punchable-api`, and the NetworkPolicy lets the worker, Prometheus and the ingress host reach pk-api and lets pk-api reach R2. A dry-run apply (`kubectl apply --dry-run=server`) of the rendered files is part of the SRE's pre-flight, not the engineer's proof. |
| **11** | **Delete the Python** (only after the rollback window closes, section 5). Delete `api/`, `api-dev/`, `worker/`, `engine/`, `creative/`, `infer/`, `gsync/` (its Rust replacement is live), `db/*.py`, `infra/worker/python/`, `infra/api/python/`, `infra/backup/backup.py`, `rust/tools/gen_*.py`, `rust/tools/diff_api_*.py`, `api/Dockerfile`, `infra/worker/Dockerfile`, `.github/workflows/api.yml` and the `goldens-current` and `api-differential` jobs. pk-api's unused upload tables (`upload_part`, `upload_stage`) are dropped from `schema_extra.surql` here, not earlier: a schema change inside the window would break the rollback. Update `docs/RUST.md`, `RUST_WORKSTREAM.md` (T11 row), `ARCHITECTURE.md`, `DEVELOPING.md` and `HANDOVER.md`. | medium (mostly deletion) | The gate and `conformance` stay green with the Python gone (they must not need it). `rg -l "python" .github rust infra` lists only the PC-only tools named in 1.4. |

### 2.1 Recordings in R2 (round 2's design)

**Problem.** A set recording must wait somewhere until a transcriber claims it, survive a pod restart, never land
in a backup, and be gone within a week. Today it sits in the Python pod's `emptyDir`, which is lost on every restart.
ADR 0014 moved it into SurrealDB rows, but the owner chose separate file storage instead (Q2b): the database disk is
5 GiB, and its WAL has filled it once (`db/DB.md`, "Compaction").

**Options.**
1. **Streamed through pk-api** (recommended). The phone keeps calling `POST /api/upload` exactly as today. pk-api
   streams the body to R2 as a multipart upload, about 8 MiB per part, so memory is bounded by one part per upload.
   It writes the pointer row `upload:<name>` (key, size, kind, time) only after the multipart completes, then queues
   the job. The worker's `GET /api/worker/jobs/<id>/file` is streamed back from R2 by pk-api. This is the same
   approach the backup job already uses: path-style SigV4 requests to one bucket with a bucket-scoped token
   (`infra/backup/backup.py:140-170`). Cost: the bytes cross pk-api twice, which is fine for one comic's few uploads
   a week. No app change and no new TestFlight build; the worker keeps speaking HTTP to the API only (ADR 0011); no
   credentials or bucket URLs ever leave the server; no bucket CORS.
2. **Presigned PUT from the phone.** pk-api hands out a short-lived signed URL with `Content-Length` signed, the phone
   PUTs straight to R2, then tells pk-api it is done. The bytes skip the server. Cost: a new app build and TestFlight
   round, a two-step protocol with a "started but never finished" state, bucket CORS for the web app, and an upload
   that bypasses the API's legal and size checks until the "done" call. Rejected for T11. Revisit only if streaming
   through pk-api shows up in its memory or CPU.
3. **Database rows (ADR 0014).** Rejected by the owner.

**Decisions inside option 1.**
- **Crate:** `rusty-s3` (sans-IO signing for PUT, multipart, GET, HEAD and DELETE) over the `reqwest` client pk-api
  already has. `aws-sdk-s3` would also work but pulls in a second HTTP and TLS stack. The engineer may choose
  `aws-sdk-s3` with a stated reason.
- **Bucket:** a dedicated recordings bucket with its own bucket-scoped token in a new Secret, unversioned, not the
  backup bucket and not the existing audio bucket. Least privilege: the recordings token cannot read backups. The SRE
  confirms whether an audio bucket and token already exist before anything new is made.
- **Size cap:** the Python's current limit (`MAX_UPLOAD`, 1 GiB, `api-dev/server.py:76`), ported as-is. ADR 0014's
  64 MiB existed only because of the database disk. 413 from `Content-Length` before a byte is read, as today.
- **Retention (7 days), three layers.** (1) `release_for_job` deletes the object when the job is dropped, fails for
  good or is done (the ADR 0014 rule, ported). (2) pk-api's hourly expiry pass (`expiry.rs`, under `lease:uploads`)
  fails a job whose recording was never claimed after 7 days, deletes the object, and shows the reason in the app.
  (3) An R2 lifecycle rule on the bucket deletes any object older than 8 days and aborts any multipart upload left
  incomplete after 1 day. This backstop works even if pk-api is down or the row was lost.
- **Never half an upload:** the pointer row is written last; a failed stream aborts the multipart; the orphan sweep
  reads pointer rows and the bucket listing, never the bytes.
- **Out of backups:** the bytes are never in SurrealDB, and the backup job never reads the recordings bucket. The
  pointer rows are small and the backup may carry them. A restored pointer whose object has gone is treated like an
  expired recording.
- **Rollback:** the Python's file mode cannot see recordings waiting in R2 (section 5).

**How it fails and how the owner finds out.** If R2 is unreachable, `POST /api/upload` answers 503 with a JSON
reason, the app shows it, and the phone still has the recording. pk-api logs the error and the 5xx alert from
round 1 fires. If the bucket token is wrong, pk-api's start-up check (a `HEAD` on the bucket) refuses readiness, so
the rollout stops before traffic moves. If an object goes missing, the worker's fetch answers 404 and the job fails
with "the recording is gone", which shows in the app.

**Security.** The recordings token, the bucket policy and the streaming path go to `gw-security` before round 2 is
final.

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
  replayer (`cargo xtask conformance`) seeds a scratch SurrealDB with `pk-api admin seed` (round 6), starts pk-api
  alone with a local S3-compatible server for recordings, plays the tapes in order and compares each answer. The
  two-writers and rows checks (phases D and F) do not need a Python: they run against pk-api's own rows. Phase B
  (writes straight to the Python) is dropped, because no second writer exists any more. gsync's recorded scenarios
  (round 4) replay against `pk-gsync` the same way.
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
- `goldens-current` and `api-differential` (`rust.yml`) stay until round 9 merges. From then `conformance` runs
  beside them, and both must stay green until the cutover.
- From the cutover, the Python code is frozen: no change to `api-dev/`, `db/`, `creative/`, `engine/`, `worker/` or
  `gsync/`. The rollback image is a digest that already exists, not a rebuild. `api.yml` keeps running in that window
  only to show the frozen tree still passes. A failure in the window means the rollback image's source went bad,
  which is worth knowing.
- Round 11, when the rollback window closes, deletes `api.yml`, `goldens-current`, `api-differential` and the code
  together. The PC-only tools keep their own tests until they are ported (scope, 1.4).

## 4. The cutover (one deploy, the owner's go at each production step)

**Ahead of the cutover:** round 1 (shipping alone now), the Rust backup job beside the Python one for 3 nights
(round 7), and gsync switched to the Rust binary on the PC (round 4) once its dry run matches. Each of these is its
own step with the owner's go and a rollback line.

Preconditions for the cutover: rounds 1-10 merged on main and CI green on the merge commit. `conformance`, both
differentials and the `--standalone` pass all pass. The 3 parallel backup nights matched and one Rust backup
restored. The recordings bucket, its token Secret and its lifecycle rules exist, and `gw-security` has signed off
round 2. The pk-api, pk-worker and backup images come from that CI run, loaded by digest (`docs/RUST_WORKSTREAM.md`
"Deploying the images"). Use `infra/ship.sh rollout <name> <ref>`, which passes the ref since `435d652` but has not
yet been used in a live rollout, or `deploy.sh` on the node as the journal describes. The private deploy log gets
one line per step with its rollback.

1. **Backup, verified** (the usual pre-deploy step, now by the Rust job), plus `pk-api admin state`, which must show
   `loading=false`.
2. **Recordings.** Check that the Python's inbox is empty (`GET /api/inbox` = `[]`), because anything in the
   `emptyDir` is lost when the pod is replaced. A recording uploaded and not yet collected would otherwise wait for
   the phone to send it again.
3. **pk-api takes the whole API.**
   0. Roll out the Python API image built from the cutover commit, so its copy of `schema_extra.surql` equals
      pk-api's. This is the image the rollback will use.
   a. Roll out pk-api (round 10 image) with `PK_SCHEMA_OWNER=1`, the recordings bucket on, and `PK_PYTHON_URL`
      **still set**: a safety net while the Ingress moves. `pk-api admin schema --check` must report no difference,
      and readiness confirms the bucket check.
   b. Point `/api` in `50-ingress.yaml` at `punchable-api-rs` and delete the `-ids`, `-files` and `-upload`
      Ingresses. Watch `pk_api_forwarded_total`. Any forward means a request still needs the Python: stop and look.
   c. Once forwards have stayed at zero through one use of the app by the owner (Bits, a set, a jot, an upload,
      Settings), roll out pk-api without `PK_PYTHON_URL`.
   d. Scale the Python Deployment to 0. Its SIGTERM handler releases `lease:embedder` (`api/entry.py:258-268`), and
      pk-api's sweeper takes it. Watch pk-api's memory against the 850 MiB line in the journal.
4. **Check** with the owner's real use (one recording uploaded and collected end to end, its object deleted after the
   job is done), and in Grafana (`punchable-api-rs` UP, 5xx at 0, p95 per route against the pre-cutover numbers).
   Write the deploy-log line.

The worker needs no step. It already talks to pk-api (`PK_WORKER_API=http://punchable-api-rs`, live).

## 5. Rollback window and retirement

**The window: two weeks that include a gig night** (lead's default). The Python Deployment stays at `replicas: 0`.
Its Service, Secrets, NetworkPolicies and last image digest also stay. The Python gsync stays installable on the PC
(its task can be pointed back).

**Rules while the window is open.** These keep the rollback valid:
- No change to `db/schema.surql` or `db/schema_extra.surql`. A rolled-back Python re-applies **its** copy of
  `schema_extra` at start (`surreal_kb.py:150`), and pk-api with `PK_SCHEMA_OWNER=1` re-applies its own whenever the
  stored sha differs. Two different copies would fight. Step 3.0 of the cutover deploys a Python image with the same
  copy, so any schema change round 2 makes is already in the rollback image.
- No new job kind and no new table the Python does not know. The Python's job replay refuses an unknown kind and
  stops every creative route (journal gotcha 2026-11-07, `api-dev/creative_api.py:113`).
- New pk-api features that need either of these wait until the window closes.

**Rollback** (minutes, every step reversible):
1. Scale the Python to 1 and wait until it is Ready (it materialises from the database, as it does at every start).
2. Roll out pk-api with `PK_PYTHON_URL` set and `PK_SCHEMA_OWNER=0`.
3. Point `/api` back at `punchable-api` and re-apply the pk-api Ingresses from the rollback manifests. New uploads
   go back to the Python's inbox folder. Recordings already waiting in R2 are invisible to the Python. They stay in
   the bucket (the lifecycle rule still removes them after 8 days), the phone still holds each one, and pk-api
   serves them again on roll-forward. The deploy-log line lists them (`pk-api admin state` prints the waiting
   pointers).
4. Deploy-log line. Then find out what went wrong before trying again.

**Retirement** (after the window, the owner's go):
1. Last check: `pk_api_forwarded_total` was 0 for the whole window (it should be, since no Python URL is set), and
   every nightly backup in the window passed.
2. Delete the Python Deployment, its Service and the `app=punchable-api` NetworkPolicies, and the Python backup
   CronJob. Delete the Secrets only the Python mounted (`punchable-ai`, and `punchable-r2` unless round 2 reused it)
   after a check that nothing else mounts them. Remove the Python images from the cluster registry. Uninstall the
   Python gsync task on the PC.
3. Round 11 merges: the code, CI and docs go. Tag `python-oracle-final` before the deletion commit (round 9 tagged
   the recording commit; this tag is the same commit or a later one with only frozen Python in between).
4. Close T11 in `docs/RUST_WORKSTREAM.md`.

## 6. How each part fails, and how the owner would find out

| Part | Fails how | Seen how |
|---|---|---|
| Worker legal rules | Secret unreadable | Round 1: the worker does not start (CrashLoop, alert), never quietly runs on 14 markers. |
| pk-api alone | A path the app uses was never ported | `pk_api_forwarded_total` > 0 at step 3b. After 3c, the app shows an error on that screen and pk-api logs the 404. The standalone differential is meant to catch this before deploy. |
| Recordings in R2 | R2 unreachable, token wrong, object gone | Section 2.1: 503 with a reason in the app, readiness refused on a bad token, a failed job naming the missing recording. The lifecycle rule bounds the bucket even if pk-api stops cleaning. |
| gsync in Rust | A Doc edit differs from the Python's | The dry run on a throwaway Doc copy before the switch; afterwards, the Python task stays installable for the window. |
| Backups | The Rust export misses a table | The restore drill in round 7, the 3 parallel nights, and a restore drill before the cutover. |
| Schema owner | Two writers of `schema_extra` | Only possible while the Python runs with a different copy, which the freeze rule prevents. pk-api logs "the database holds another build's tables". |
| pk-api memory with the embedder always on it | OOM at 1 GiB | The journal's 850 MiB line; `OOMKilled` in the pod status; the new `punchable-api-rs` alert from round 1. Uploads add at most one part buffer each. |
| The reference after T11 | A tape edited to hide a regression | Tape edits show in review like golden edits; `MANIFEST.sha256` stops silent fixture edits. |

## 7. Answers (2026-11-07) and what is still open

Owner:
- **Q1, the PC loader:** dropped. The nightly backups rebuild the database. Only the scratch-database part stays
  (round 6).
- **Q2b, where recordings wait:** separate file storage (R2), not the database disk (round 2, section 2.1).
- **Q3, gsync:** still used; moves to Rust now, inside T11, before the cutover (round 4).
- **Q5, the worker's legal rules:** ships on its own, now (round 1, in progress).

Lead's defaults:
- **Q2a:** the Rust backup job runs 3 nights beside the Python one.
- **Q4:** "no Python" means the server and CI. PC-only tools come later.
- **Q6:** the rollback window is two weeks and includes a gig night.

Still open, for later and not blocking T11:
- **N1.** gsync keeps running on the PC after round 4, because the port is as-is. The standing direction is
  phone plus server, with no home PC. Moving gsync onto the server means keeping his Google sign-in token on the
  server, which needs a security review (`gw-security`) first. **Picked up on 2026-11-16: section 8.**

## 8. Nothing on the PC (plan, 2026-11-16)

Status: **plan only; nothing built.** Owner, 2026-11-16: nothing of Punchable should rely on his PC, as its own
rounds of the Rust port, starting with a security review for moving the Google Docs sync to the server. Written by
the architect on main `7e3f623`; part A's design for keeping the Google sign-in was reviewed by `gw-security` the same day (8.4). New code
is Rust (`CLAUDE.md`).

Tags: CONFIRMED = read in the code at `file:line` on main `7e3f623`, or checked on the owner's PC on 2026-11-16;
INFERRED = follows from what was read, not run; UNKNOWN = nobody has checked. Nothing here was run against
production.

### 8.1 What still runs on the PC

| # | Piece | Today | Tag |
|---|---|---|---|
| L1 | **Google Docs sync** (`pk-gsync`, Rust) | A scheduled task on the PC, one pass a minute. It holds his Google sign-in and a Keycloak offline token as files in his home folder (`rust/crates/pk-gsync/src/config.rs:194-206`), keeps its state as files (`store.rs:66-76`: `map.json`, `units.json`, the logs, `mirror/`), and reads his private legal rules file (`config.rs:119-121`). | CONFIRMED (lead's check of the task; code) |
| L2 | **Drive voice memos** | `pk-gsync` lists and downloads audio from a Drive folder (`memos.rs:252-280`), then starts a Python processor that needs ffmpeg and faster-whisper on the PC's GPU (`gsync/memos.py:1-19`; `pk-gsync/README.md` "What is different"). | CONFIRMED |
| L3 | **Laugh detection** | Ran on the PC (`tools/laughs/detect_laughs.py` through `engine/laughs_runner.py`, a torch model). The PC worker task is disabled; the laugh map of every set is frozen (`docs/HANDOVER.md` open item 6). | CONFIRMED (lead's check; handover) |

Facts that shape the design:

- **Where the Drive access really lives.** The refresh token with the full `drive` scope is only in the PC's token
  file. Sign-in through Keycloak has asked Google for basic scopes only since 2026-10-26, so the token Keycloak
  stores no longer carries Drive (`infra/keycloak/realm-punchable.yaml:126-132`). The app's "Reconnect Google"
  comment still says it renews Drive access (`app/src/app/(tabs)/(rant)/settings.tsx:117-119`). INFERRED: today's
  Reconnect button does not renew Drive access; if Google ever refused the PC's token, the only repair is a
  command on the PC.
- **gsync borrows the worker's identity.** It calls `/api/worker/*` with the worker's own client secret
  (`punchable.rs:61-80`), and the API's rule is one role plus a path prefix (`pk-api/src/auth/mod.rs:72-80`).
  CONFIRMED.
- **Most of the state is already copied to the database.** Five state files and the run log are posted as `srcfile`
  rows (`punchable.rs:28,507-539`), and pk-api reads them for Settings, memos and merges (`routes/status.rs:213-215`).
  `mirror/` stays on the PC. CONFIRMED.
- **The legal rules are already on the server.** The same private file is the Secret `punchable-legal-rules`, mounted
  in pk-api and pk-worker, which refuse to start when it is set and unreadable (`pk-api/src/lib.rs:600-617`;
  `infra/ship.sh:58-62`). CONFIRMED.
- **gsync's own legal check is weaker than the servers'.** Four faults, all live on the PC today:
  1. A missing, unreadable or broken rules file silently becomes the generic rules (`rules.rs:120-128`). CONFIRMED.
  2. With no owner address configured the "not his doc" net is off (`rules.rs:159-167`). CONFIRMED.
  3. A parent folder Drive does not return is recorded as having no parents, so the folder net passes the doc, and
     that answer is cached (`sync.rs:470-479`). CONFIRMED by reading; not run.
  4. The check order is metadata first, with nothing opened (`sync.rs:494-507`), then export and a second check on the
     text (`:592-600`). A legal doc the metadata net misses **is opened**, then dropped. Eight more places export or
     download without their own check (`sync.rs:533, 869, 1060, 1168, 1222, 1401, 1445`; `memos.rs:453`), and two of
     them store the text with no content check (`:869`, `:1060`). CONFIRMED by `gw-security`.
- **Voice memos from Drive have gone quiet.** On the PC every listed memo is done, none is waiting, and the state
  file last changed on 2026-10-27, four days before memos moved onto the phone (ADR 0012, decided 2026-10-31).
  CONFIRMED (the PC's `memos.json`, counts only).

### 8.2 Part A: the Docs sync on the server

**Problem.** Run the sync where the rest of Punchable runs, without making the server a way into his court and legal
documents, and without him needing a PC to set it up, repair it or reconnect Google.

**Where it runs.**

| Option | For | Against |
|---|---|---|
| **Its own Deployment** `punchable-gsync`, one replica, `Recreate`, running `pk-gsync serve` (recommended) | Its secrets are in one pod that does nothing else. Nothing public reaches it. It can finish the Google sign-in. The loop and its token cache stay warm. | One more pod (small: no database client, no model). |
| A CronJob | No standing process. A failed run is a failed Job. | A pod a minute. Nowhere to finish a Google sign-in, so a second component is needed anyway. |
| A loop inside pk-worker | No new pod. | The worker pod holds the model provider's key and runs prompts over imported text; it would then also hold the Drive token. A sync fault would restart the AI worker. Against "one worker, one spend ledger" (ADR 0011). Rejected. |

The pod follows the worker's manifest: non-root, read-only root, no service-account token, default-deny
NetworkPolicy (`infra/k8s/35-worker.yaml`, `10-networkpolicy.yaml`). In: pk-api (two calls, below) and Prometheus.
Out: pk-api, Keycloak's token endpoint, and TCP 443 to public ranges (Google).

**How the Google sign-in gets there, with no PC.**

1. A **new Google OAuth client used only for Drive**, in the Google Cloud project he already has (its consent
   screen is already "In production", `gsync/SETUP.md` step 3.3, so refresh tokens do not lapse weekly; INFERRED
   that a second client in the same project inherits that). Its one redirect address is a callback path on
   punchable.io. Login keeps its own client, so rotating one never breaks the other.
2. Settings on the web gets a real **Connect Google Docs** button (the phone shows the status and where to go, as
   now). pk-api `POST /api/google/connect` checks that the caller is the owner by a pinned subject, not merely
   signed in, then asks the gsync pod for a Google address to send the browser to. gsync makes a single-use `state`
   (10 minutes, bound to the owner's subject and to a short-lived `__Host-` cookie) and a PKCE verifier it keeps.
3. Google sends the browser back to `GET /api/google/callback` on pk-api, which only passes `code` and `state` to
   gsync and answers with a redirect to a fixed Settings address (`no-store`, `no-referrer`; neither value logged).
4. gsync redeems the code with the client secret and the verifier. It refuses, and revokes what it was given, unless
   Google itself says the account is his (verified address on first use, Google's stable subject id from then on)
   and the scopes granted are exactly the scopes asked for.
5. **Reconnect** when Google has expired or revoked the access is the same button. The previous token is revoked
   explicitly; a new consent alone leaves the old one valid.

pk-api never holds the client secret, the verifier or the token.

**Which Secret, who can read it.**

| Thing | Where it rests | Who can read it |
|---|---|---|
| Drive client id and secret, the sealing key (32 random bytes made on the server, never anywhere else), the state-signing key | Secret `punchable-gsync-google` | the gsync pod only |
| The Google refresh token | One row in the database, **sealed** (ChaCha20-Poly1305 from the crypto stack already in the tree; a version byte; the row id, key id, Google subject and scope bound in as associated data). A new row per reconnect. | Ciphertext: pk-api, the database, the nightly backups. Plaintext: the gsync pod only. |
| gsync's Punchable credential: a new Keycloak client `pk-gsync-svc` with **its own role** | Secret `punchable-gsync` | the gsync pod only |
| The legal rules | Secret `punchable-legal-rules` (exists) | pk-api, pk-worker, gsync |

Considered for the token: a Kubernetes Secret the pod can update (needs a service-account token and API-server
egress in a pod with internet egress, which no pod here has), and a file on a small volume (acceptable to
`gw-security`, adds nothing against root on the node). The sealed row keeps the pod diskless and the backups useless
to a thief without the key.

What goes away at the switch: the Keycloak offline token and device client `pk-gsync`, the `broker/read-token`
mapping and default role, `storeToken` on the Google provider (`realm-punchable.yaml:117-118,283-287`), and the
worker secret in gsync's hands.

**Scopes.** The read-only trial (below) uses a token with `drive.readonly` only, so it cannot write even with a
bug. At the switch: **narrow** (`drive.readonly` plus `drive.file`; the mode exists, `config.rs:17`). The server can
then read his Drive and change only the docs Punchable itself made. What narrow costs: a merge no longer moves his
old docs into the merged folder, and docs the Claude connector made are re-made by Punchable so it can keep
updating them (INFERRED from `drive.rs:1-9`, `sync.rs:284-302`; the engineer lists the exact writes). Full `drive`
would be a later, explicit owner decision (8.4, 8.7 Q1).

Also considered: a service identity that sees only a folder he shares with it. Court documents would be
unreachable by construction and nothing would ever expire. It changes how he works (a doc outside that folder is
never seen; today gsync finds comedy docs anywhere in his Drive, `sync.rs:345-349`), and whether such an identity
can create Docs in a personal Drive is UNKNOWN (`gw-security` believes it cannot). Not recommended now (8.7 Q2).

**Rotation.**

| Secret | When | How |
|---|---|---|
| Google refresh token | Reconnect; any suspicion | The button. The old token is revoked at Google. |
| Sealing key, state-signing key, `pk-gsync-svc` secret | Any suspicion, any exposure of the node or a backup, and once a year | `ship.sh` step that makes a new value on the server; gsync re-seals under the new key id on its next start. |
| Drive client secret | Any suspicion | New secret in the Google console, then reconnect: rotating it does not revoke tokens already issued. |

**Blast radius if the server is compromised.**

- *Root on the node, or code running in the gsync pod:* the refresh token and the client secret. Under narrow that
  is **every file in his Drive readable, court documents included**, and Punchable-made docs changeable, until he
  removes Punchable's access in his Google account. Under full `drive` it would also be change and deletion of any
  file. The legal rules file is exposed too (it already is on the server).
- *Code running in pk-api only:* ciphertext, and a `code` it cannot redeem. **This holds only after must-fix 1 in
  8.4**: as the code stands, whoever can write the state rows can name any Drive file as a Punchable-made doc, and
  gsync will export it and (under full `drive`) overwrite it.
- *A stolen backup:* ciphertext, doc ids and titles of his comedy docs. Not the token.
- *The worker's secret:* nothing of gsync's, once the worker role can no longer read or write gsync's rows.

**Where state lives.** The same JSON documents gsync writes today, as rows read and written through pk-api, not
files: no pod disk, the differential tapes keep proving the logic, and pk-api's readers do not change. `mirror/`
moves into rows too. Rules for the hosted store:

- A read that fails stops the pass. A missing `map.json` row is a refusal, not an empty map (`store.rs:80-87` starts
  empty today), unless the pod was started with an explicit bootstrap flag.
- Every write carries the hash of what it replaces, so two writers cannot interleave.
- Every state document carries a MAC under the state-signing key. A document that fails the check is refused. The
  state is an input the pod must not trust (must-fix 1).
- Proper tables instead of documents: later, only if a measurement asks for it. It would rewrite a proven path and
  pk-api's readers (principle: port as-is).

**The legal check: court and legal docs are never opened; fails closed.**

1. **Rules required.** With `PUNCHABLE_LEGAL_RULES` set (always, on the server and in the PC task): an unreadable or
   unparsable file, no owner address, or no private entry means the program does not start. pk-api's rule
   (`load_legal`), ported.
2. **Fingerprint.** Logged at start, exported as a metric, and pinned: a fingerprint other than the accepted one
   stops the loop until it is accepted. `fingerprint()` exists and nothing calls it (`rules.rs:214`).
3. **Unknown means excluded.** A parent folder that cannot be read excludes the doc and is not cached. An error
   inside the legal check excludes the doc; today it is retried next pass (`sync.rs:444-454`).
4. **No export without clearance.** `export` and `download` take a clearance value that only a passing metadata
   check on **fresh** metadata for that file id can make. It is a type, so no call site can skip it. Every site
   named in 8.1 goes through it. The write gate checks the doc's own "made by Punchable" tag on fresh metadata at
   every write and never trusts the map alone.
5. **The content net runs before anything else touches the text**: before it is hashed, stored, mirrored or sent,
   at every site, including the two that skip it today.
6. **A dropped doc leaves nothing behind.** Its text is never logged, stored or sent. The `excluded` set only grows.
   What is recorded is the reason's kind (`folder`, `kw`), never the keyword that hit (`sync.rs:499` stores the
   keywords today, and they reach the database and the backups). Run records carry a hash, not the id, of a
   skipped legal doc, and titles stay out of the pod's log.
7. **A content-net hit is an alert.** It means the metadata net missed a legal doc and the doc was opened in
   memory. He adds its folder to the rules so the next one is never opened.
8. **Second line.** pk-api's own legal check on everything gsync posts stays (`pk-api/src/lib.rs:600`). What it
   covers for `edits.jsonl` text is UNKNOWN.

What cannot be promised: "never opened" is exact for every doc the metadata rules name (folder, id, owner, title).
A legal doc that none of those rules name is opened once in the pod's memory and dropped by the content net. That
is true on the PC today; on the server the alert in item 7 makes each such case visible.

**How it is proven.**

1. *In the repo.* The recorded tapes replay against the hosted store and must give the same requests and the same
   documents. New tests, each with a red arm: no rules file means no start; an unreadable parent folder means the doc
   is skipped; a state document with a bad MAC is refused; a map row naming a doc without Punchable's tag cannot be
   exported or written; a stale hash on a write is refused; a callback with a wrong `state`, a second use of a
   `state`, or another Google account is refused.
2. *Read-only beside the PC.* The pod runs in shadow mode on the server with the `drive.readonly` token and clients
   that refuse every write (as a dry run does, `pk-gsync/README.md`). Each pass writes its plan to its own rows. The
   PC keeps doing the real work. The two plans are compared pass by pass over a period that includes a real edit, a
   new doc, a merge, and a legal-looking doc in a throwaway folder, which both must report as `skip_legal`. The
   rules fingerprints must be equal. A difference is a finding, not noise.
3. *`gw-security` reads the built code* (the callback, the sealing, the state MAC, the role) before the switch.
4. *The switch* (owner's go): disable the PC task first (one writer), reconnect with the narrow scopes, turn shadow
   mode off, watch one real edit go through.

**Rollback.** Scale `punchable-gsync` to 0 and enable the PC task again; the state documents are the same ones. The
PC's own token is left alone during the window, which has an end date set at the switch. When it closes: the PC's
token is revoked at Google, the task and the token files are removed, and the Keycloak pieces above are deleted.

**How it fails and how he finds out.**

| Failure | Seen how |
|---|---|
| Google refuses the token | Alert on refresh failure; Settings shows "Reconnect Google Docs" |
| The rules file is missing or changed | The pod does not start, or the loop stops on the fingerprint; alert |
| The pod is down or a pass has not finished lately | Alert on the last-pass age metric; Settings shows the last sync time |
| The state rows were tampered with or lost | The pass refuses (MAC or missing row); alert |
| A legal doc slipped the metadata net | Alert on the content-net counter |

### 8.3 Parts B and C

**B. Drive voice memos without the PC's GPU.**

| Option | Cost | Quality | Notes |
|---|---|---|---|
| A hosted speech API, called by a server job | Well under a cent a minute at list prices as recalled; INFERRED, check on the day. A new key and a new egress. | About today's (the same family of model). | His audio goes to another company. Against "the audio stays on his phone" (owner, 2026-10-31) in spirit, though these files already sit in Google Drive. |
| A CPU model on the server | No money. A large model wants several GiB of memory and roughly real time or slower on shared cores; the worker pod has 1 CPU and 1 GiB (`35-worker.yaml:100-102`). Spare capacity on the node is UNKNOWN. | Today's with the same model; clearly worse with one small enough to be cheap (INFERRED). | A new pod, a model in the image, an audio decoder in Rust. The most work of the three. |
| **Drop the Drive-folder path** (recommended) | Nothing to build; code to delete. | The phone's on-device recogniser, which he already uses for every new memo. | The phone records and transcribes its own memos, and "Import from Files" on the phone takes audio files, several at once (`app/src/lib/recordings.native.ts:324-328`). The Drive folder has had nothing waiting since 2026-10-27. |

**Recommendation: drop it.** The path existed to bring in a backlog, and the backlog is done. An old memo that turns
up later comes in through the phone. This also removes the last reason the Python `gsync/` package must stay
(`pk-gsync/README.md`), **which round 11 needs**: it deletes `gsync/` while `pk-gsync` still starts
`python -m gsync.memos process`. The memo list keeps showing what was already imported.

**C. Laugh detection on the server.**

Known:
- The detector is a pretrained audio-tagging network plus a pass that lines events up with the transcript
  (`engine/audio.py:111`). It has a CPU switch (`engine/laughs_runner.py:21`). CONFIRMED.
- Its output is one `laughs_auto.json` per set, and pk-api already takes that file from a worker and turns it into
  laugh rows (`pk-api/src/writes/srcpaths.rs:20`). No schema change is needed to receive laughs. CONFIRMED.
- A recording sent with `POST /api/upload` waits in R2 for at most 7 days and is deleted when its job ends
  (ADR 0015). Nothing on the server claims that job (ADR 0012).

UNKNOWN:
- Whether a set taped in the app has yet reached R2 from his phone (no real app upload was made at the round 2
  deploy, journal 2026-11-07).
- How well the detector does on phone recordings; it was run on other tapes. There is no answer key, only two
  detectors compared with each other (`tools/laughs/agreement.py`).
- Whether the transcript pass works with the phone's transcripts, which carry times per segment, not per word
  (`infer/laughs.py:36-47` reads word times).
- Whether the network gives the same events when run from Rust through ONNX Runtime (already in the tree for
  search), and its time and memory on the server's CPU.
- Whether he wants laughs on app recordings at all (owner, 2026-11-16: decided after he has used Tapes).

**Smallest first step (C0): a measurement, nothing deployed.** A `pk-audio` crate that decodes a recording, runs
the network and writes events. It is compared with the laugh maps already stored for a few existing sets, using
the measures `agreement.py` defines, and with invented audio in CI (no tape in the repo). It reports agreement,
seconds per minute of audio and peak memory on a named CPU-only machine. Red arm: shift the input features by one
band and agreement must collapse. C0 answers three of the unknowns and commits the server to nothing.

**C1, only after C0 and his decision:** a small audio pod that claims a new `laughs` job, fetches the recording
through the route the transcriber would use, and posts `laughs_auto.json`. It needs a new job kind, so it cannot
ship inside the T11 rollback window (section 5), and an ADR that amends ADR 0012 for laughs only.

### 8.4 `gw-security` on part A (2026-11-16)

**Verdict: approve with must-fixes.** The shape is sound: its own pod, a separate Drive client, a sealed token, a
read-only shadow. The review was of the design and of the code on main; nothing was run.

Must-fix, all before the switch:

1. **The state rows are authority over Drive.** The map can mark any doc id as Punchable-made and gsync then
   write-allows it without checking the doc's tag (`sync.rs:286-302`), exports it with no legal check
   (`:1168, 1222, 1401, 1445`) and stores the text (`:1401-1415`). Any worker-role token may write that row
   (`pk-api/src/writes/srcpaths.rs:23`). On his disk this is merely unvalidated; in the database it is exploitable.
   Fix: authenticate each state document with a key only gsync holds; legal check and tag check on fresh metadata at
   every export and write; take `kb/gsync/*` away from the worker role.
2. **Clearance at every site**, the memo download included, and the content net before any text is hashed, stored
   or sent.
3. **A new role, not the worker role**, with a route allow-list; the worker loses gsync's state and the sealed row.
4. **"Owner session" means a pinned subject**: any non-worker token is the comic today (`auth/mod.rs:4-6`).
5. **The callback** is a browser redirect and carries no bearer token: bind `state` to the subject and a `__Host-`
   cookie, answer only with a redirect to a fixed address, keep `code` and `state` out of logs.
6. **The account check** comes from Google itself, pins Google's subject id after the first connect, and refuses
   scopes other than those asked for.
7. **Fail-closed additions:** the unreadable parent folder, the rules loader, and an error in the legal check.
8. **Private keywords stay in the rules file**: store the reason's kind only.

Should-fix: the seal framing as in 8.2; **narrow scopes at the switch** ("full `drive` on an internet-facing node
turns a compromise from disclosure into destruction of court documents"); the rotation table in 8.2; hashed ids for
legal skips and no titles in the pod's log; alerts on refresh failure and fingerprint change; an end date on the
rollback window.

UNKNOWN, to settle in the build: whether a second client keeps the consent screen's production status; whether the
web app and `/api` share one origin (the cookie needs it); whether the ingress access log records the callback's
query string; what pk-api's ingest check covers for `edits.jsonl`.

All eight must-fixes and the should-fixes are folded into 8.2 and assigned to rounds below.

### 8.5 The rounds

Ordered by risk: the live gaps first, then the pieces that delete or only measure, then the build, then the trial,
and the switch last.

| Round | What | Size | Its own proof |
|---|---|---|---|
| **A0** | **gsync fails closed** (pk-gsync crate only; ships to the PC task, which is live). Rules required when the variable is set; owner address required; unreadable parent folder and legal-check errors exclude; clearance type on every export and download site; content net first everywhere; tag check on fresh metadata at every write; reason kind only in the map; fingerprint logged. Must-fixes 2, 7, 8 and the code half of 1. | medium | The tape differential stays green for every scenario whose behaviour is unchanged; each changed behaviour has a Rust test with a red arm (the old code path turns it red). Any tape that must change is listed in the round's report with the reason. |
| **B1** | **Drop the Drive-memo path** (owner Q3). `pk-gsync` stops listing, downloading and starting the Python processor; the "PC is low on disk" wording goes (`pk-api/src/routes/memos.rs:141`); the memo list keeps its rows. Must land before round 11. | small | A tape of a pass with memos enabled shows no Drive memo request and no process start (red arm: the old call). The memo list answers the same rows as before. |
| **C0** | **Laugh detector measurement** (8.3). New crate, nothing deployed. | medium | The numbers, on a named machine, with the red arm. |
| **A1** | **gsync's own identity and authority in pk-api**, behind a setting that is off by default (as `PK_UPLOAD_R2` was): role and client `pk-gsync-svc`, route allow-list, the worker loses `kb/gsync/*`, the pinned owner subject, the opaque sealed-row route, write-with-expected-hash. Must-fixes 3, 4 and the API half of 1. | medium | With the setting off both differentials are unchanged. With it on: the worker's token is refused on gsync's rows, gsync's token is refused off its list, a stale hash is refused (red arm each). |
| **A2** | **Hosted mode of `pk-gsync`**: `serve`, the store over pk-api rows with MACs, `mirror/` in rows, shadow mode, metrics, the image, the manifest and NetworkPolicy (prepared, not applied). | medium-large | The tapes replay against the hosted store: same requests, same documents. Shadow mode makes no write request (tape check, as the dry run's). Bad MAC and missing map row refuse (red arms). |
| **A3** | **Connect Google Docs**: the Drive client, the two pk-api routes, the exchange, the account and scope checks, sealing, reconnect with revocation, the Settings row. Must-fixes 5, 6. Then `gw-security` reads A1 to A3 as built. | medium | Tests against a stand-in Google: wrong `state`, reused `state`, missing cookie, other account, extra scope are each refused; the sealed row opens only with the key and the right associated data (red arm: change one bound field). |
| **A4** | **Shadow on the server** (SRE; owner's go). `drive.readonly`. Plans compared with the PC's. | small build, real waiting | 8.2 "How it is proven", step 2: zero unexplained differences, equal fingerprints, the throwaway legal doc skipped on both. |
| **A5** | **The switch and the clean-up** (owner's go at each step): PC task off, narrow reconnect, shadow off; after the window, revoke the PC's token, remove the task and files, delete the Keycloak device client, `read-token` and `storeToken`, drop file mode from `pk-gsync`, update the Settings wording ("your PC", `routes/status.rs:127`). | small-medium | One real edit end to end. The realm test shows no client may read a broker token. Nothing named "Punchable" is left in the PC's scheduled tasks. |
| **C1** | **Laughs on the server**, only if C0 and the owner say yes (8.3). | medium-large | Decided after C0. |

**What can run beside the remaining T11 rounds (5, 6, 8, 9, 10, 11).**

- **Now, in parallel:** A0 (its own crate), B1 (pk-gsync and one pk-api wording line), C0 (a new crate), A2 (its own
  crate), and the building of A1 and A3 on branches. They share only `rust/Cargo.toml` and `Cargo.lock` (add lines,
  never reorder).
- **Watch round 9:** it freezes the tapes, gsync's included. A0 and B1 change gsync's behaviour on purpose, so they
  land **before** the tapes are frozen, or each edits the frozen tape in its own commit with the reason (section 3).
  A1 stays off by default so the frozen API tapes are unaffected.
- **Watch round 10:** it rewrites the manifests and NetworkPolicies. A2's manifest is a prepared file until then and
  joins `infra/k8s/` after round 10 merges.
- **Before round 11:** B1 must be merged, or round 11 keeps `gsync/memos.py` out of its deletion.
- **After the T11 rollback window closes (round 11):** turning A1 on, deploying A3's sealed row if it needs a table
  (no new table inside the window, section 5), A4 and A5. Until then the PC task keeps running, which the T11
  rollback needs anyway (section 5: the Python gsync stays installable on the PC for the window). So "nothing on
  the PC" is finished after T11, not during it.
- **C1** is outside T11 and waits for the owner.

### 8.6 What the owner does by hand

- In the Google Cloud console: add one OAuth client for Drive with the callback address (A3). The secret goes
  straight into the server's Secret through a `ship.sh` step that reads it without showing it, as the login
  client's did.
- Press **Connect Google Docs** on punchable.io: once for the read-only trial, once at the switch.
- After the window: remove the old "Punchable" access entry in his Google account if the revocation did not.

### 8.7 Questions for the owner

1. **How much of your Google Drive may the server change?** Recommended: *read everything, change only the docs
   Punchable made.* If the server were ever broken into, your other documents could be read but not altered or
   deleted. The cost: when you merge bits, the old Docs stay where they are, and a few Docs made earlier through
   Claude get re-made by Punchable.
2. **Would you keep all comedy Docs in one folder you share with Punchable?** Then the server could never see a
   court document at all. Recommended: *not now.* It changes how you work (a Doc outside that folder would not be
   picked up) and Google may not allow that kind of access to create new Docs in a personal Drive. Worth revisiting
   if you would rather file that way.
3. **Stop importing voice memos from the Drive folder?** Recommended: *yes.* Every memo from that folder is in, none
   has arrived since the phone started recording memos, and an old recording can still be brought in with "Import
   from Files" on the phone.
4. **May we measure laugh detection now, before you decide on laughs?** Recommended: *yes.* It deploys nothing, and
   it tells you whether laughs on app recordings would be good enough to want.
5. **The Docs sync stays on your PC until the Python is retired and its two-week fallback has passed.** Recommended:
   *accept this order.* The safety fixes (A0) reach the PC straight away; the move itself is the riskiest step and
   should not overlap the cutover.
6. **Connecting Google stays a button on the website, not in the phone app** (as Google sign-in is today).
   Recommended: *yes.*
