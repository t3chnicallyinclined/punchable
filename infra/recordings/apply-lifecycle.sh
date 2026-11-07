#!/usr/bin/env bash
# PRINTS the recordings bucket's lifecycle rule (docs/adr/0015, layer 3 of the retention); it changes nothing and needs no
# credentials. The rule is set by the owner by hand, in the Cloudflare dashboard under the owner's own login: R2, the
# recordings bucket, Settings, Object lifecycle rules:
#   prefix rec/ ; delete objects 8 days after they are created ; abort incomplete multipart uploads 1 day after they start.
# No API token is made to set it: R2 cannot scope an admin token to one bucket, so a token that could edit this lifecycle
# could also reach the backup bucket. (If a token is ever used anyway, make it short-lived and revoke it straight after.)
set -euo pipefail
cat "$(cd "$(dirname "$0")" && pwd)/lifecycle.json"
