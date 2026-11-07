# Recordings in R2 (docs/adr/0015)

Where a set's recording waits for a transcriber. pk-api streams `POST /api/upload` into one R2 bucket, keeps only a small
pointer row (`upload:<name>`) in the database, and streams the object back on `GET /api/worker/jobs/<id>/file`. Nothing
here is applied by any script in this repo except the two the SRE runs by hand (`ship.sh recordings-secret` and
`apply-lifecycle.sh --apply`). The manifest changes are prepared, not applied, in `infra/k8s/prepared/t10-api-rs.yaml`.

## What the SRE creates (exactly this, nothing more)

1. **One R2 bucket, dedicated to recordings.** Private (no public access, no custom domain, no CORS), unversioned (R2 buckets
   are, by default), in the account the backup bucket is in. Not the backup bucket and not the audio bucket. Name it as you
   like; the name lives only in the Secret below, never in git. Nothing else is stored in it: every object pk-api makes is
   under `rec/`.
2. **One R2 API token, scoped to that bucket alone, with Object Read & Write.** Not "Admin", no other bucket, no account
   permissions. It cannot read the backup bucket and the backup token cannot read this one. (R2 shows the Access Key ID and
   the Secret Access Key once; they go straight into the file in step 3.)
3. **One Secret, `punchable-r2-recordings`, in the `punchable` namespace**, with exactly four keys:

   | key | value |
   |---|---|
   | `PK_REC_ENDPOINT` | `https://<account id>.r2.cloudflarestorage.com` (the S3 API endpoint, no bucket in it) |
   | `PK_REC_BUCKET` | the bucket's name |
   | `PK_REC_ACCESS_KEY_ID` | the token's Access Key ID |
   | `PK_REC_SECRET_ACCESS_KEY` | the token's Secret Access Key |

   Make it the way the backup Secret is made: put the four lines in `~/.punchable/r2-recordings.env` on the PC (never in
   git), then `infra/ship.sh recordings-secret`. The values go over ssh stdin into `kubectl`, never argv, a file on the host
   or the output. `infra/ship.sh recordings-secret` checks the shapes first (https endpoint on `r2.cloudflarestorage.com`,
   a 32-hex key id, a 64-hex secret).
4. **The lifecycle rule** in `lifecycle.json`, put on the bucket once with `infra/recordings/apply-lifecycle.sh` (a dry run
   unless `--apply`). Delete any object older than 8 days; abort any multipart upload left incomplete after 1 day. This needs
   a bucket **Admin** token, a different one from the pk-api token and thrown away afterwards: the app's token cannot edit a
   lifecycle on purpose. Alternatively set the same two rules in the Cloudflare dashboard (bucket, Settings, Object lifecycle
   rules, prefix `rec/`).

Then, in this order (the cutover steps, plan round 10): Secret, lifecycle rule, `infra/ship.sh rollout api <ref>` of the pk-api
image built from this change (readiness waits for a `HEAD` on the bucket, so a wrong token or bucket stops the rollout with
the old pod still serving), apply the `punchable-api-rs-upload` and `punchable-api-rs-file` Ingresses and the
`punchable-api-rs-r2` NetworkPolicy from `infra/k8s/prepared/t10-api-rs.yaml`.

## Proving it on a real bucket before it matters (the SRE's scratch run)

The tests run against a stand-in that verifies the SigV4 signature and R2's multipart rules, not against R2. Before the
phone uses it: with the Secret in a scratch namespace, upload a file of about 20 MiB (three parts) and one of a few KB
through pk-api, fetch both back (`GET /api/worker/jobs/<id>/file` with a worker token), compare checksums, drop the jobs and
check the bucket listing is empty, then upload one more and confirm the lifecycle rule shows in `apply-lifecycle.sh --show`.

## Settings of pk-api (all optional but the first)

| variable | default | meaning |
|---|---|---|
| `PK_UPLOAD_R2` | off | `1` turns recordings in R2 on; then the four `PK_REC_*` above are required |
| `PK_REC_REGION` | `auto` | what R2 signs with |
| `PK_UPLOAD_KEEP_S` | 7 days | how long a recording waits for a transcriber before the hourly pass deletes it |
| `PK_UPLOAD_SWEEP_S` | 1 hour | how often the pass (and the bucket sweep) runs; `0` = never |

## What to watch

- pk-api's log: `recordings:` lines (a refused or retried request names an operation and a status, never a URL).
- A `503` from `POST /api/upload` with "the recording could not be stored right now" is R2 (or the token) failing; the phone
  keeps the recording and tries again. The `punchable-api-rs` 5xx alert covers it.
- The bucket should hold at most the recordings waiting for a transcriber (a few) and nothing older than 8 days.
