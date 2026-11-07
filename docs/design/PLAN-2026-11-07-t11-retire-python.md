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
  server, which needs a security review (`gw-security`) first.
