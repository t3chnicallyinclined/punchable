# ADR 0014: uploads in the database

Date: 2026-11-06. Status: **Accepted (lead defaults, owner may flip)** on 2026-11-06 (T10 round 5, group D): the three
defaults below are implemented and proven. `PK_UPLOAD_DB` is still off by default on both servers; the deploy of the
Rust API turns it on, on both Deployments, once the Python's `emptyDir` inbox goes away with the Python.

**Context.** `POST /api/upload` (the phone's recording of a set) put the file in the Python pod's inbox folder until a
transcriber had processed it, which only the Python could read. T10 group 6a moved it into SurrealDB as rows
(`upload`, `upload_part` and a small `upload_stage` row per upload in flight; `db/schema_extra.surql`) so `pk-api` can answer
`/api/upload`, `/api/inbox` and `/api/worker/jobs/<id>/file` and the Python can still read it. That is a design
change, not a port as-is:
- The folder was a 2 Gi `emptyDir` wiped on every restart (`infra/k8s/30-api.yaml`), so an upload was bounded and
  temporary. The production SurrealDB is a 512 Mi Guaranteed pod on a 5 Gi PVC, SurrealKV versioned, and its WAL has
  filled that disk once (`db/DB.md`, "Compaction": 853 MB in one day).
- Rows do not expire: a dropped job (`job_drop`) and a failed result keep the upload, nothing deletes one that no
  transcriber ever claims, and the nightly export to R2 (`infra/k8s/60-backup.yaml`, `GET /export`, 14 daily + 8
  weekly) would carry every waiting recording into every backup.
- The blob queries are plain index lookups (checked with `EXPLAIN FULL` on SurrealDB 3.3.4, plans in
  `docs/RUST_WORKSTREAM.md` T10 round 4), but a megabyte row is still a megabyte of WAL and of memtable.

**Decision (accepted on the lead's defaults).**
1. **Off unless asked.** `PK_UPLOAD_DB=1` turns the database home on, on the Python and on `pk-api` alike. Off (the
   default), a hosted upload is the file in the pod's inbox folder exactly as before, `pk-api` answers
   `POST /api/upload` with 503 and a loud log line (a moved path can never silently diverge), and its inbox and fetch
   routes are forwarded to the Python. The deploy of the Rust API turns it on, on both Deployments.
2. **Lead defaults, implemented and proven; the owner may flip any of them:**
   - *Size.* At most **64 MiB per upload** when the database is the home (audio of a set is tens of MB; a Film-it
     video never leaves the phone). Over it: **413** `{"error": "upload over 64 MiB"}` on both servers (an empty
     one: 400 `upload missing`), decided from `Content-Length` before a byte is read (`api-dev/server.py` `upload`,
     `writes/uploads.rs`). The phone's recorder should treat a 413 as final and not retry.
   - *Retention.* The bytes are deleted **when the job is dropped, when it fails for good (a result with `retry`
     that goes back to the queue keeps them), when it is done, and 7 days after the upload if no transcriber has
     claimed it**. The first three are in both servers (`creative_api.job_drop` and `w_result`,
     `writes::uploads::release_for_job`); the 7 days is pk-api's expiry pass (`expiry.rs`: every hour under
     `lease:uploads`, `PK_UPLOAD_KEEP_S` and `PK_UPLOAD_SWEEP_S`), which also fails the waiting job with the reason so
     the app shows it is gone, leaves a claimed recording alone and takes a stray one that has no job. The Python
     runs no such pass (it leaves at T11); until then pk-api's pass covers the one database.
   - *Backups.* Uploads are **excluded from the nightly export**: `infra/backup/backup.py` lists the tables
     (`INFO FOR DB`), asks `POST /export` for every table but `upload`, `upload_part` and `upload_stage` (rows and
     definitions), then for those three alone with `records: false` (definitions only, its second `OPTION IMPORT;`
     dropped: an import refuses to see two). Restore drill on a scratch SurrealDB 3.3.4, exported as a database
     EDITOR and imported into an empty database: the other rows came back, the three tables were defined again with
     their index, their rows were gone (`docs/RUST_WORKSTREAM.md`, T10 round 5 group D). The Job's ConfigMap is
     made from the committed file by `ship.sh backup`.
3. **Never half an upload.** A recording is listed only once its `upload:<name>` row exists, which is made last and
   only when the staging row counts exactly the parts written; an upload whose job could not be queued is deleted (the
   phone sends it again); the row is deleted before its bytes; and the orphan sweep reads the small staging rows
   (newest part older than an hour), never the megabyte rows.

**Consequences.** The cap, the deletions and the export exclusion are implemented and proven (Rust tests, differential
cases, the restore drill). Still the owner's: a size check of the 5 Gi PVC against the cap times the longest realistic
queue (64 MiB times a week of recordings is the bound), and the flag on both Deployments at deploy. Flipping a default
is one constant (`MAX_UPLOAD`, `upload_keep`, `EXCLUDED_TABLES`). Rejecting uploads in the database means
`/api/upload` stays on the Python until T11, where uploads move to an object store (R2) with a signed URL and the
database holds only a pointer.
