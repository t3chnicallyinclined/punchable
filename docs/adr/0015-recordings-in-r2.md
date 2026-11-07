# ADR 0015: recordings in R2, not in the database

Date: 2026-11-07. Status: **Accepted** (owner decision 2026-11-07, T11 plan question Q2b: "separate file storage, not the
database disk"). **Supersedes ADR 0014's choice of where a recording waits** (rows in SurrealDB). The rest of ADR 0014
carries over: retention, "never half an upload", the exclusion from backups. gw-security reviews the token scope, the
bucket policy and the streaming path before round 2 is final (section "Security").

**Context.** `POST /api/upload` (the phone's recording of a set) must park the file somewhere until a transcriber claims
it, survive a pod restart, never land in a backup and be gone within a week. The Python kept it in the pod's `emptyDir`,
lost on every restart. ADR 0014 moved it into SurrealDB rows so both servers could see it, and capped it at 64 MiB
because the production database is a 5 Gi volume whose write-ahead log has filled it once (`db/DB.md`, "Compaction").
Retiring the Python (T11) removes the reason for the shared database home, and the owner chose separate file storage.

**Decision.**
1. **Streamed through pk-api into one R2 bucket.** The phone keeps calling `POST /api/upload` with the same headers and
   body, and gets the same answer (no app change, no TestFlight build). pk-api streams the body into the recordings bucket:
   a recording of at most one part (8 MiB) is a single PUT, a longer one a multipart upload of 8 MiB parts, so memory is one
   part per upload however long the recording is (measured: `tests/recordings.rs`, 384 MiB streamed, the process grew by
   a few parts, not by the recording). The worker keeps speaking HTTP to the API only (ADR 0011): `GET
   /api/worker/jobs/<id>/file` streams the object back with the Python's headers. No credential, bucket name or signed URL
   ever leaves the server.
2. **A pointer row, written last.** `upload:<file name>` holds the object key (in its `token` field, the name ADR 0014
   gave it), the size, the part count, the meta.json and the time. It is made after the object is complete, so a
   half-sent file is never listed. A name already taken gets the next suffix, as before. **The schema is unchanged**
   (`db/schema_extra.surql` still defines `upload`, `upload_part` and `upload_stage`; pk-api writes only `upload`), so the
   Python image stays a valid rollback during the window. The two unused tables are dropped when the Python is deleted
   (plan round 11), not before.
3. **Size cap: the Python's, as it is now.** At most 1 GiB (`MAX_UPLOAD`), decided from `Content-Length` before a byte is
   read; an empty one or one over it is the Python's file-mode `400 {"error": "upload missing or over 1 GB"}`. ADR 0014's
   64 MiB and its 413 existed only because of the database disk and are gone. A body that stalls for 60 s or ends before
   its `Content-Length` is `400 upload was cut short`, as the Python's.
4. **Retention, three layers** (7 days; the first two are ADR 0014's rules, now deleting an object as well as a row):
   - *on the job's end*: a `process_set` job that is dropped, fails for good or is done deletes its recording (the pointer
     row first, then the object; a delete the bucket refuses is logged and left to the next two layers). A retried
     failure keeps it.
   - *the hourly pass* (`expiry.rs`, under `lease:uploads`): a recording still unclaimed after `PK_UPLOAD_KEEP_S` (7 days) is
     deleted and its waiting job ends `failed` with the reason, so the app shows it; a claimed one is left alone. The same
     pass sweeps the bucket: an object under `rec/` older than an hour that no pointer row names (an upload whose row never
     got written, a delete that failed) is deleted. It reads rows and the bucket listing, never the bytes.
   - *the bucket's lifecycle rule* (`infra/recordings/lifecycle.json`, applied by the SRE, never by pk-api): delete any
     object older than 8 days and abort any multipart upload left incomplete after 1 day. This holds even when pk-api is down
     or a row was lost.
5. **Out of backups.** The bytes are never in SurrealDB, and the backup job never reads this bucket (its token cannot: it is
   scoped to the backup bucket). The pointer rows are small; a restored pointer whose object has gone is handled like an
   expired recording (the fetch answers 404 `the recording is gone ...`, the hourly pass removes the row).
6. **Failure is loud and never leaves half an upload.** R2 unreachable or refusing: `POST /api/upload` answers 503 with a
   reason (a status, never a URL, key or bucket) and the phone still has the recording; a failed or abandoned stream aborts
   its multipart upload and writes no row; a row that cannot be written deletes the object. At start the pod `HEAD`s the
   bucket and stays not ready until that answers, so a wrong token or bucket stops the rollout instead of surfacing at the
   first upload.
7. **Off unless asked, like ADR 0014.** `PK_UPLOAD_R2=1` turns it on and then `PK_REC_ENDPOINT`, `PK_REC_BUCKET`,
   `PK_REC_ACCESS_KEY_ID` and `PK_REC_SECRET_ACCESS_KEY` are required (`PK_REC_REGION` defaults to `auto`); an endpoint must
   be `https` (plain `http` only to a loopback host, for the test stand-in). Off, `POST /api/upload` answers 503 and the
   inbox and the fetch are forwarded to the Python, so a moved path never silently diverges.

**How it is built.** `rusty-s3` (sans-IO signing and parsing, BSD-2-Clause, its `aws-lc-rs` backend: the crypto stack the
tree already has) over the `reqwest` client pk-api already uses, path-style SigV4 to one bucket with a bucket-scoped token,
the shape `infra/backup/backup.py` already runs in production. Every request is a presigned URL good for five minutes and
used at once; a transport error, 5xx, 408 or 429 is tried three times, anything else is final. `aws-sdk-s3` would also work
and pulls in a second HTTP and TLS stack. **Not chosen:** presigned PUTs from the phone (a new app build, bucket CORS, a
two-step protocol with a "started, never finished" state, and an upload that skips the API's checks until a "done" call);
revisit only if streaming through pk-api shows up in its memory or CPU.

**Security (for gw-security).**
- *Token scope.* One R2 API token, object read and write, restricted to the one recordings bucket; no other bucket, no
  bucket administration, no lifecycle edit. It lives in the Secret `punchable-r2-recordings`, referenced by `envFrom`, and
  is mounted only in the pk-api Deployment. The backup token and bucket are separate, so neither can read the other.
- *Bucket policy.* Private, no public access, no custom domain, no CORS, unversioned (a deleted recording is gone), the
  lifecycle rule above. Nothing reads it but pk-api.
- *Streaming path.* The route needs a signed-in caller (not the worker role). Memory is bounded by one part per upload; an
  upload that stalls is cut after 60 s; the size is checked before the body is read; the object key is made by pk-api
  (`rec/<random>`), never from the phone's file name; a pointer row's key is used only if it starts with `rec/`; the
  presigned URLs hold the signature in their query, so no error or log line is built from a URL (checked by a test that
  looks for the endpoint, bucket, key id and `X-Amz` in the error text); `Debug` of the settings redacts the credentials;
  egress is TCP 443 to the public internet only, as the backup job's policy.
- *What is not covered here:* real R2 (the tests run against a stand-in that verifies the SigV4 signature of every
  request and R2's multipart rules); the SRE's scratch run against a real bucket is the proof for that.

**Consequences.** The recording leaves the database disk and the pod's disk entirely; a pod restart loses nothing; nothing
about recordings enters a backup. The bytes cross pk-api twice (phone to R2, R2 to the transcriber), which is fine for one
comic's few uploads a week. Rollback: the Python's file mode cannot see recordings waiting in R2; they stay in the bucket
(the lifecycle rule still removes them after 8 days), the phone still holds each one, and pk-api serves them again on
roll-forward. Flipping a default is one constant (`MAX_UPLOAD`, `PART`, `upload_keep`, `ORPHAN_AGE_S`). The SRE creates
one bucket, one token and one Secret (plan round 10 and `infra/recordings/README.md`).
