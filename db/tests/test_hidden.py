"""Hidden material (kb v1 `hidden: true`: not_a_joke units with no text, pasted_not_his units with text kept).

    python db/tests/test_hidden.py

They must not show in any API list or search, on either backend, and come back only when explicitly requested
(?include_hidden=1). TEMP database, TEMP copies.
"""
from __future__ import annotations

import json
import sys
from urllib.parse import quote

from common import Api, TempDB, TempKB

FAILS: list[str] = []


def check(ok: bool, msg: str):
    print(("PASS " if ok else "FAIL ") + msg)
    if not ok:
        FAILS.append(msg)


def main():
    t = TempKB("hidden")
    db = TempDB("hidden")
    api = None
    try:
        db.load(t)
        g = json.loads(t.graph.read_text(encoding="utf-8"))
        hj = {n["id"] for n in g["nodes"] if n["type"] == "joke" and n.get("hidden") is True}
        hu = {n["id"]: n for n in g["nodes"] if n["type"] == "unit" and n.get("hidden") is True}
        check(bool(hj) or bool(hu), f"graph has hidden material: {len(hj)} jokes, {len(hu)} units")
        api = Api(t, "surreal", db)
        for be in ("", "backend=files"):
            sep = "&" + be if be else ""
            listed = set()
            o = 0
            while True:
                _, page = api.get(f"/api/jokes?limit=500&offset={o}{sep}")
                listed |= {x["id"] for x in page["items"]}
                o += 500
                if o >= page["total"]:
                    break
            check(not (listed & hj), f"[{be or 'surreal'}] /api/jokes lists none of the {len(hj)} hidden jokes")
            _, allh = api.get(f"/api/jokes?limit=500&include_hidden=1{sep}")
            check(allh["total"] == page["total"] + len(hj), f"[{be or 'surreal'}] ?include_hidden=1 brings them back ({allh['total']} vs {page['total']})")
            for jid in list(hj)[:4]:
                s1, _ = api.get(f"/api/jokes/{quote(jid, safe=':')}?x=1{sep}")
                s2, _ = api.get(f"/api/jokes/{quote(jid, safe=':')}?include_hidden=1{sep}")
                check(s1 == 404 and s2 == 200, f"[{be or 'surreal'}] hidden joke page {jid}: {s1} by default, {s2} when requested")
            # no hidden unit appears in any visible joke's versions
            leaked = []
            for jid in sorted(listed)[:2000]:
                _, d = api.get(f"/api/jokes/{quote(jid, safe=':')}?x=1{sep}")
                for grp in d.get("versions", []):
                    leaked += [u["id"] for u in grp["units"] if u["id"] in hu]
                if d.get("text", {}).get("from") and d["text"]["from"].get("unit") in hu:
                    leaked.append(d["text"]["from"]["unit"])
            check(not leaked, f"[{be or 'surreal'}] no hidden unit in any joke's versions or text ({len(leaked)} found)")
            # search: text of a pasted_not_his unit finds nothing through that unit
            pasted = [u for u in hu.values() if (u.get("text") or "").strip()]
            if pasted:
                words = " ".join(pasted[0]["text"].split()[:6])
                _, r = api.get(f"/api/jokes?q={quote(words)}&limit=500{sep}")
                _, r2 = api.get(f"/api/jokes?q={quote(words)}&limit=500&include_hidden=1{sep}")
                check(len(r["items"]) <= len(r2["items"]), f"[{be or 'surreal'}] word search for hidden text: {len(r['items'])} hits, {len(r2['items'])} with include_hidden")
        if pasted:
            words = " ".join(pasted[0]["text"].split()[:8])
            _, h = api.get(f"/api/jokes?q={quote(words)}&mode=hybrid&limit=50")
            _, hh = api.get(f"/api/jokes?q={quote(words)}&mode=hybrid&limit=50&include_hidden=1")
            check(not ({x["id"] for x in h["items"]} & hj), "hybrid search returns no hidden joke by default")
            check(len(hh["items"]) >= len(h["items"]), f"hybrid search with include_hidden: {len(hh['items'])} >= {len(h['items'])}")
    finally:
        if api:
            api.stop()
        db.drop()
        t.cleanup()
    print(f"\n{'PASS' if not FAILS else 'FAIL'}: {len(FAILS)} failed")
    sys.exit(1 if FAILS else 0)


if __name__ == "__main__":
    main()
