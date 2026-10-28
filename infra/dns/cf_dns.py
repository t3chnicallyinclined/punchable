#!/usr/bin/env python3
"""Punchable DNS on Cloudflare. Touches ONE zone: PK_DOMAIN from infra/config.env. Stdlib only.

    python -P infra/dns/cf_dns.py plan     # default: read-only diff, changes nothing
    python -P infra/dns/cf_dns.py apply    # create the missing records; never modifies or deletes an existing one
    python -P infra/dns/cf_dns.py undo     # delete ONLY records this tool created (comment tag + ledger match)

Records (grey-cloud: both names resolve straight to the origin;
cert-manager's HTTP-01 issues the certificate, so Cloudflare's SSL mode does not apply):
    A  <domain>       -> PK_ORIGIN_IP   proxied=false  ttl=300
    A  auth.<domain>  -> PK_ORIGIN_IP   proxied=false  ttl=300
    A  bugs.<domain>  -> PK_ORIGIN_IP   proxied=false  ttl=300   (the crash-report store, infra/k8s/70-bugsink.yaml)
Token: CF_API_TOKEN from ~/.punchable/cloudflare.env, read in-process, never printed. The token can see other zones;
this tool resolves the zone BY NAME, asserts the answer is exactly PK_DOMAIN, and refuses any write whose URL is not
under that zone id. Ledger of created record ids: ~/.punchable/dns-created.json (ids only, no secrets).
"""
import json, os, sys, urllib.error, urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
CFG = {}
_CFG_FILE = Path(os.environ.get("PK_INFRA_ENV") or (HERE.parent / "config.env" if (HERE.parent / "config.env").exists()
                 else Path.home() / ".punchable" / "infra.env"))
for line in _CFG_FILE.read_text().splitlines():
    if "=" in line and not line.lstrip().startswith("#"):
        k, v = line.split("=", 1); CFG[k.strip()] = v.strip().strip("\"'")
DOMAIN, ORIGIN = CFG["PK_DOMAIN"], CFG["PK_ORIGIN_IP"]
AUTH = CFG["PK_AUTH_HOST"]
assert AUTH.endswith("." + DOMAIN), "auth host must be inside the zone"
# The comment that marks records this tool made (undo deletes only those). PK_DNS_TAG keeps an older tag working.
TAG = CFG.get("PK_DNS_TAG") or "punchable-infra (punchable/infra/dns/cf_dns.py)"
WANT = [("A", DOMAIN, ORIGIN), ("A", AUTH, ORIGIN)]
SECRETS = Path.home() / ".punchable"
LEDGER = SECRETS / "dns-created.json"

env = {}
for line in (SECRETS / "cloudflare.env").read_text(encoding="utf-8").splitlines():
    line = line.strip()
    if line and not line.startswith("#") and "=" in line:
        k, v = line.split("=", 1); env[k] = v.strip().strip('"')
TOKEN = env["CF_API_TOKEN"]
API = "https://api.cloudflare.com/client/v4"
ZONE_ID = None


def cf(method, path, body=None):
    if method != "GET":
        assert ZONE_ID and path.startswith(f"/zones/{ZONE_ID}/dns_records"), f"refusing a write outside the {DOMAIN} zone"
    req = urllib.request.Request(API + path, method=method, data=json.dumps(body).encode() if body else None,
                                 headers={"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            out = json.load(r)
    except urllib.error.HTTPError as e:
        out = json.load(e)
    if not out.get("success"):
        sys.exit(f"Cloudflare {method} {path.split('?')[0]} failed: {out.get('errors')}")
    return out["result"]


zones = cf("GET", f"/zones?name={DOMAIN}")
if len(zones) != 1 or zones[0]["name"] != DOMAIN:
    sys.exit(f"zone {DOMAIN} not found exactly once (got {[z['name'] for z in zones]})")
ZONE_ID = zones[0]["id"]
print(f"zone {DOMAIN} id {ZONE_ID[:8]}... status {zones[0]['status']}")
have = cf("GET", f"/zones/{ZONE_ID}/dns_records?per_page=100")
by_key = {}
for r in have:
    by_key.setdefault((r["type"], r["name"]), []).append(r)
print(f"existing records: {len(have)} {sorted((r['type'], r['name'], r['content'], r['proxied']) for r in have)}")
mode = (sys.argv[1:] or ["plan"])[0]

if mode in ("plan", "apply"):
    todo = []
    for t, name, content in WANT:
        cur = by_key.get((t, name), []) + by_key.get(("CNAME", name), [])
        if not cur:
            todo.append((t, name, content)); print(f"  + create {t} {name} -> {content} (grey, ttl 300)")
        elif any(r["type"] == t and r["content"] == content and not r["proxied"] for r in cur):
            print(f"  = ok     {t} {name} -> {content}")
        else:
            print(f"  ! CONFLICT {name}: {[(r['type'], r['content'], r['proxied']) for r in cur]} -- left untouched")
    if mode == "plan":
        print("plan only: nothing changed"); sys.exit(0)
    ledger = json.loads(LEDGER.read_text()) if LEDGER.exists() else []
    for t, name, content in todo:
        r = cf("POST", f"/zones/{ZONE_ID}/dns_records",
               {"type": t, "name": name, "content": content, "proxied": False, "ttl": 300, "comment": TAG})
        ledger.append({"id": r["id"], "zone": ZONE_ID, "type": t, "name": name, "content": content})
        LEDGER.write_text(json.dumps(ledger, indent=1))
        print(f"  created {t} {name} -> {content} id {r['id'][:8]}...")
elif mode == "undo":
    ledger = json.loads(LEDGER.read_text()) if LEDGER.exists() else []
    live = {r["id"]: r for r in have}
    keep = []
    for e in ledger:
        r = live.get(e["id"])
        if r and r.get("comment") == TAG and r["name"] == e["name"] and e["zone"] == ZONE_ID:
            cf("DELETE", f"/zones/{ZONE_ID}/dns_records/{e['id']}")
            print(f"  deleted {e['type']} {e['name']}")
        elif r:
            print(f"  ! {e['name']} changed since we created it -- left in place"); keep.append(e)
    LEDGER.write_text(json.dumps(keep, indent=1))
else:
    sys.exit(__doc__)
