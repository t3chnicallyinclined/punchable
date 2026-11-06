# ADR 0014: uploads in the database

Date: 2026-11-06. Status: **Proposed** (not accepted; the lead's defaults below are for the owner to flip). Nothing
in this ADR is switched on: `PK_UPLOAD_DB` is off by default on both servers.

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

**Decision (proposed).**
1. **Off unless asked.** `PK_UPLOAD_DB=1` turns the database home on, on the Python and on `pk-api` alike. Off (the
   default), a hosted upload is the file in the pod's inbox folder exactly as before, `pk-api` answers
   `POST /api/upload` with 503 and a loud log line (a moved path can never silently diverge), and its inbox and fetch
   routes are forwarded to the Python. Nothing is deployed in a way that turns it on until this ADR is accepted.
2. **Lead defaults, for the owner to flip:**
   - *Size.* At most **64 MiB per upload** when the database is the home (audio of a set is tens of MB; a Film-it
     video never leaves the phone). Over it: the 400 the Python gives for a missing or oversized upload.
   - *Retention.* The bytes are deleted **when the job is dropped, when it fails, and 7 days after the upload if no
     transcriber has claimed it**. Today only an `ok` result deletes them (done).
   - *Backups.* Uploads are **excluded from the nightly export**: the `api` user's `GET /export` is asked for every
     table but `upload`, `upload_part` and `upload_stage` (SurrealDB's export takes a table list; to be proven with
     `infra/backup/backup.py restore-check` before this is accepted), or the blobs live in a separate namespace the
     backup job never exports. A restore drill must show the listed jobs survive with their uploads gone.
3. **Never half an upload.** A recording is listed only once its `upload:<name>` row exists, which is made last and
   only when the staging row counts exactly the parts written; an upload whose job could not be queued is deleted (the
   phone sends it again); the row is deleted before its bytes; and the orphan sweep reads the small staging rows
   (newest part older than an hour), never the megabyte rows.

**Consequences.** Until accepted the T10 uploads work is dormant behind the flag and the hosted behaviour is
unchanged. Accepting it needs the cap, the three deletions and the export exclusion implemented and proven (differential
cases for each, the restore drill), and a size check of the 5 Gi PVC against the cap times the longest realistic queue.
Rejecting it means `/api/upload` stays on the Python until T11, where uploads move to an object store (R2) with a
signed URL and the database holds only a pointer.
