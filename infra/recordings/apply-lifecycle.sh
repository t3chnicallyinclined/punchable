#!/usr/bin/env bash
# The recordings bucket's lifecycle rule (docs/adr/0015, layer 3 of the retention): lifecycle.json, applied to the bucket.
# It is the backstop under pk-api's own deletes: any object older than 8 days goes, and a multipart upload left incomplete
# for a day is aborted, even when pk-api is down or a pointer row was lost.
#
# DRY RUN BY DEFAULT: prints the rule and the bucket it would be put on and changes nothing. `--apply` puts it. The SRE
# runs it once, after reading lifecycle.json, with an ADMIN token for this bucket (the pk-api token cannot edit a
# lifecycle, on purpose). The credentials come from the environment, never from argv:
#   PK_REC_ADMIN_ENDPOINT, PK_REC_ADMIN_BUCKET, PK_REC_ADMIN_ACCESS_KEY_ID, PK_REC_ADMIN_SECRET_ACCESS_KEY
# Needs the AWS CLI (any v2) on the machine that runs it. `--show` reads back what the bucket has now.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
MODE="${1:-}"
case "$MODE" in ""|--apply|--show) ;; *) echo "usage: $0 [--apply|--show]" >&2; exit 2 ;; esac
for v in PK_REC_ADMIN_ENDPOINT PK_REC_ADMIN_BUCKET PK_REC_ADMIN_ACCESS_KEY_ID PK_REC_ADMIN_SECRET_ACCESS_KEY; do
  [ -n "${!v:-}" ] || { echo "!! $v is not set" >&2; exit 1; }
done
case "$PK_REC_ADMIN_ENDPOINT" in https://*) ;; *) echo "!! the endpoint must be https://" >&2; exit 1 ;; esac
command -v aws >/dev/null || { echo "!! the AWS CLI is not installed" >&2; exit 1; }
export AWS_ACCESS_KEY_ID="$PK_REC_ADMIN_ACCESS_KEY_ID" AWS_SECRET_ACCESS_KEY="$PK_REC_ADMIN_SECRET_ACCESS_KEY" AWS_DEFAULT_REGION=auto
s3api() { aws --endpoint-url "$PK_REC_ADMIN_ENDPOINT" s3api "$@" --bucket "$PK_REC_ADMIN_BUCKET"; }
if [ "$MODE" = --show ]; then
  s3api get-bucket-lifecycle-configuration
  exit 0
fi
echo "rule (infra/recordings/lifecycle.json):"
cat "$HERE/lifecycle.json"
if [ "$MODE" != --apply ]; then
  echo "dry run: nothing was changed. Re-run with --apply to put this rule on the bucket named in PK_REC_ADMIN_BUCKET."
  exit 0
fi
s3api put-bucket-lifecycle-configuration --lifecycle-configuration "file://$HERE/lifecycle.json"
echo "applied. Read it back: $0 --show"
