"""T10's gate: the writes pk-api answers itself against the hosted Python API, over ONE scratch SurrealDB (the same
setup as rust/tools/diff_api_reads.py: push_remote pack + apply, a local JWKS, both servers with the model).

    python rust/tools/diff_api_writes.py --pk-api rust/target/release/pk-api.exe [--report out.json] [--conc 8]

What it proves, in order:
  A. write through pk-api, read through BOTH: after every write the reads that show it are equal on the two servers
     (status, body as a value, headers), and each server's very next read already shows it (no wait);
  B. the reverse: the same kinds of writes straight to the Python, then both servers' reads;
  C. the same write to both servers, answered the same: the write's own body (ids, times and the per-server tag
     normalised; the lines of an edit compared by shape), and every refusal exactly (a refusal writes nothing);
  D. the rows: every `decision` and `creative_event` row has the right shape (`seq` contiguous, `h` = sha256 of
     `line`, `op`/`item`/`at` from the line, the keys the Python writes), and the two sequences wrote the same rows;
  E. idempotency: the transcript the phone posts twice is one job on either server; a dismiss posted twice answers
     `already`; an undone line undone again is the Python's 409;
  F. two writers at once: threads jotting through both servers in the same second lose nothing (every line lands,
     once, the sequence has no gap); pk-api's own counters saw every moved write and counted no forwarded one; the
     Python's creative stores never diverged from the database;
  G. no model on either side: a jot that is one existing bit word for word is attached under it by both.
Round 2 (T10 groups 3 and 4) runs inside the same sequence: a rating (the AI's match confirmed with it), the Set
page's in-place edits (new material, a re-match, crowd work, the match rejected and undone), the Tonight list, a set
list made, changed, added to, archived, brought back and deleted, a bridge queued, a set promoted, versioned, its
locks and try notes, the punch-ups (a locked bit and a bit with no tape answered with no job; the rest queued), the
settings, the voice profile saved and a draft dismissed, a "not a joke?" kept and one hidden; and the merge
candidates read on both after each (the judge file carries real keys: diff_api_reads.write_judge).
Exit 1 on any difference. Needs what diff_api_reads.py needs (surreal 3.3.4, the API's Python deps, PK_BGE_DIR, a
built pk-api).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import quote

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import diff_api_reads as R  # noqa: E402  (the scratch setup, the HTTP helpers and the comparer)
import jwt_kit as JK  # noqa: E402

ROOT, SX = R.ROOT, R.SX
app: dict = {}
worker_tok: dict = {}
py = rs = ""
problems: list[str] = []
n_equal = 0

# ---------------------------------------------------------------- volatile parts of a write's answer
VOLATILE_KEYS = {"ts", "created_at", "updated_at", "done_at", "claimed_at", "received_at", "at", "date", "generated_at",
                 "undone_at", "detached_at", "index", "indexes", "undo_of", "undo_indexes", "kept", "dropped"}
MINTED = [(re.compile(r"\bjob:[a-z_]+-[0-9a-f]{12}\b"), "job:X"), (re.compile(r"\b(?:at|mg|dm):[0-9a-f]{8}\b"), "id:X"),
          (re.compile(r"\bunit:[0-9a-f]{12}\b"), "unit:X"), (re.compile(r"\bjoke:[a-z0-9\-]+\b"), "joke:X"),
          (re.compile(r"\b(?:sb|ws):[a-z0-9\-]+\b"), "list:X"), (re.compile(r"\btry:[0-9a-f]{8}\b"), "try:X"),
          (re.compile(r"\((?:A|B|P|R)\)"), "(T)")]


def norm(v):
    """A value with its timestamps, minted ids, decision indexes and per-server tags made the same on both servers."""
    if isinstance(v, dict):
        return {k: ("<volatile>" if x is not None else None) if k in VOLATILE_KEYS else norm(x) for k, x in v.items()}
    if isinstance(v, list):
        return [norm(x) for x in v]
    if isinstance(v, str):
        for rx, rep in MINTED:
            v = rx.sub(rep, v)
        return v
    return v


def shape(v):
    """Keys and types only (an edit's `before` is another bit's on the two servers)."""
    if isinstance(v, dict):
        return {k: shape(x) for k, x in v.items()}
    if isinstance(v, list):
        return [shape(x) for x in v[:1]] + ([f"...{len(v)}"] if len(v) > 1 else [])
    return type(v).__name__


def ok(label: str, cond: bool, detail: str = "") -> None:
    global n_equal
    if cond:
        n_equal += 1
    else:
        problems.append(f"{label}{': ' + detail if detail else ''}")


def body(r):
    try:
        return json.loads(r[2]) if r[2] else None
    except ValueError:
        return None


def write(base: str, method: str, path: str, b, headers: dict | None = None, want=(200, 202)):
    r = R.call(base, method, path, headers or app, b)
    if r[0] not in want:
        raise SystemExit(f"{method} {path} on {base}: HTTP {r[0]} {r[2][:300]!r}")
    return r[0], body(r)


def both(label: str, paths: list[str]) -> None:
    """The same GETs on both servers, equal as the reads differential compares them."""
    global n_equal
    for p in paths:
        got = R.compare(f"{label}: {p}", R.call(py, "GET", p, app), R.call(rs, "GET", p, app))
        problems.extend(got)
        n_equal += not got


def right_after(label: str, base: str, path: str, pred) -> None:
    """The very next GET on `base` shows the write (no wait, no retry)."""
    t = time.perf_counter()
    r = R.call(base, "GET", path, app)
    ms = (time.perf_counter() - t) * 1000
    who = "pk-api" if base == rs else "the Python"
    ok(f"{label}: the very next {path} on {who} shows the write", r[0] == 200 and pred(body(r)), f"{ms:.0f} ms, {r[2][:200]!r}")


def title7(text: str) -> str:
    return " ".join(text.split()[:7]).strip(" ,.;:!?\"'")[:120]


R.ALLOW |= {(lbl, "$.generated_at"): "the Python stamps the whole-groups answer with its own clock"
            for lbl in ("after all: /api/merge/candidates",)}


# ---------------------------------------------------------------- the write sequence (phases A and B)
READS_ALWAYS = ["/api/decisions?limit=4", "/api/jokes?limit=5000", "/api/jokes?limit=5000&include_hidden=1", "/api/today",
                "/api/profile/voice", "/api/tonight/screen", "/api/jokes?sort=recent&limit=20"]
PER_TAG = {"(A)": {"stage": "working", "confirm": "e:lc-d2", "dismiss": ["joke:d", "joke:e", "joke:d"], "dm": ["joke:d", "joke:e"],
                   # round 2: the perf rated (its AI match confirmed with it), then marked new material and re-matched;
                   # the perf marked crowd work / banter and rated in place; the bit hidden; the lists and settings
                   "perf": f"perf:{SX}@0", "match": "joke:b", "mark": "crowd", "mark_rating": "died", "hide": "joke:x06",
                   "tonight": [{"joke": "joke:a", "done": True}, {"joke": "joke:b"}], "settings": {"training": True},
                   "ws_ids": ["joke:b", "joke:a", "joke:x01"], "ws_board": "sb:old"},
           "(B)": {"stage": "testing", "confirm": "e:ld-dad", "dismiss": ["joke:x01", "joke:x03", "joke:x01"], "dm": ["joke:x01", "joke:x03"],
                   "perf": f"perf:{SX}@60000", "match": "joke:a", "mark": "banter", "mark_rating": "meh", "hide": "joke:x08",
                   "tonight": [{"joke": "joke:c"}, {"joke": "joke:d", "done": True}], "settings": {"cellular": True, "provider": "key"},
                   "ws_ids": ["joke:a", "joke:x01"], "ws_board": "sb:seed1"}}
MARK_PERF = f"perf:{SX}@140000"   # the Charlie moment: marked, rated in place, its match rejected and undone
CROWD_PERF = f"perf:{SX}@120000"  # the crowd-work moment: no match to confirm or reject


def sequence(via: str, tag: str) -> dict:
    """Every kind of write of groups 1 and 2 through `via`, each followed by the reads that show it on both servers.
    Returns what was minted, for the later phases."""
    minted: dict = {}
    n_now = body(R.call(py, "GET", "/api/decisions?limit=1", app))["total"]
    who = "pk-api" if via == rs else "the Python"
    per = PER_TAG[tag]

    def after(label: str, extra: list[str], touched: str | None = None) -> None:
        both(f"{tag} after {label} via {who}", READS_ALWAYS + extra)
        if touched:
            q_ = quote(touched, safe="")
            both(f"{tag} after {label} via {who}", [f"/api/jokes/{q_}", f"/api/jokes/{q_}?include_hidden=1",
                                                    f"/api/merge/candidates?joke={q_}", f"/api/merge/history?joke={q_}"])

    # 1. a jot: a new idea no bit clearly is, then its reads
    st, r = write(via, "POST", "/api/ideas", {"text": f"Trains are late every morning here {tag}. The conductor waves anyway."})
    ok(f"{tag} jot via {who}: shape", st == 200 and r["ok"] and len(r["indexes"]) == 1 and r["id"].startswith("joke:")
       and len(r["decisions"]) == 1 and "attach" in r and "learned" in r, json.dumps(r)[:300])
    ok(f"{tag} jot via {who}: not placed under a bit", (r.get("attach") or {}).get("decision") in ("new", "ambiguous"), json.dumps(r.get("attach"))[:200])
    ok(f"{tag} jot via {who}: a typed line is no draft", r.get("learned") is None, json.dumps(r.get("learned"))[:200])
    minted["jot"] = r["id"]
    minted["jot_attach"] = (r.get("attach") or {}).get("decision")
    n_now += 1
    for base in (rs, py):
        right_after(f"{tag} jot via {who}", base, "/api/decisions?limit=1", lambda b, n=n_now: b["total"] == n)
        right_after(f"{tag} jot via {who}", base, "/api/jokes?limit=5000", lambda b, j=r["id"]: any(x["id"] == j for x in b["items"]))
    after("a jot", [], minted["jot"])

    # 2. a jot in the words of an existing bit: with the model, the vectors decide (recorded; phase G proves the attach)
    st, r = write(via, "POST", "/api/ideas", {"text": "Delta bit about my niece and the laundromat place."})
    minted["words_jot"] = r["id"]
    minted["words_jot_attach"] = (r.get("attach") or {}).get("decision")
    n_now += 1
    after("a jot in a bit's words", [], "joke:d")

    # 3. a jot saved from a taped moment (`from`), with the review's reason
    st, r = write(via, "POST", "/api/ideas", {"text": f"A new thing I tried tonight about the bus driver {tag}.",
                                               "from": {"set": f"set:{SX}", "perf": f"perf:{SX}@140000", "start_ms": 210000,
                                                        "end_ms": 222000, "note": None, "extra": "dropped"}})
    ok(f"{tag} jot-from via {who}: from kept without nulls, reason 'saved from review'",
       r["decisions"][0]["after"].get("from") == {"set": f"set:{SX}", "perf": f"perf:{SX}@140000", "start_ms": 210000, "end_ms": 222000}
       and r["decisions"][0]["reason"] == "saved from review", json.dumps(r["decisions"][0])[:300])
    minted["from_jot"] = r["id"]
    n_now += 1
    after("a jot from a tape", [], r["id"])

    # 4. a jot he flagged as a draft (nothing drafted it: `edited`), then the learned section on both
    st, r = write(via, "POST", "/api/ideas", {"text": f"parking meters that talk back, kept as mine {tag}", "draft": True,
                                               "reason": "riff on: parking meters"})
    ok(f"{tag} jot-draft via {who}: learned counts", isinstance(r.get("learned"), dict) and r["learned"]["kept"] >= 1, json.dumps(r.get("learned")))
    n_now += 1
    after("a kept draft", [])
    pv = body(R.call(py, "GET", "/api/profile/voice", app))
    ok(f"{tag} jot-draft via {who}: the Python's profile shows the kept line as edited",
       any(e.get("edited") is True and e["bit"] == r["id"] for e in pv["learned"]["kept"]), json.dumps(pv["learned"])[:300])

    # 5. a set transcribed on the phone: one set_text job; the same transcript again is the same job (202), on
    #    either server
    tx = {"segments": [{"text": f"Hello room {tag}.", "start": 0, "end": 2.5}, {"text": "A set about the bus.", "start": 3, "end": 9},
                       {"text": "  "}],
          "title": "Phone set", "recorded_at": "2026-11-03T20:00:00Z", "venue": "The Corner Room", "duration_s": 300, "engine": "phone"}
    st, r = write(via, "POST", "/api/sets/transcript", tx)
    ok(f"{tag} transcript via {who}: 202 queued with a poll path", st == 202 and r["status"] == "queued" and r["job"]["kind"] == "set_text"
       and r["poll"] == f"/api/jobs/{r['job']['id']}" and "waiting_for" not in r, json.dumps(r)[:300])
    minted["set_text_job"] = r["job"]["id"]
    st2, r2 = write(via, "POST", "/api/sets/transcript", tx)
    ok(f"{tag} transcript twice via {who}: the same job", st2 == 202 and r2["job"]["id"] == r["job"]["id"], json.dumps(r2)[:200])
    other = py if via == rs else rs
    st3, r3 = write(other, "POST", "/api/sets/transcript", tx)
    ok(f"{tag} transcript via the other server: the same job", st3 == 202 and r3["job"]["id"] == r["job"]["id"], json.dumps(r3)[:200])
    after("a transcript", [f"/api/jobs/{quote(r['job']['id'], safe='')}", "/api/jobs/job:nope"])

    # 6. a new version of a bit, then one flagged as a draft (the learned section again)
    st, r = write(via, "POST", "/api/jokes/joke:a/versions", {"text": f"Alpha opener, the second draft of the line {tag}.", "reason": "  tightened  "})
    ok(f"{tag} version via {who}: a unit and its edge", r["ok"] and r["joke"] == "joke:a" and r["unit"].startswith("unit:") and len(r["indexes"]) == 2
       and "learned" not in r, json.dumps(r)[:300])
    minted["version_unit"] = r["unit"]
    n_now += 2
    for base in (rs, py):
        right_after(f"{tag} version via {who}", base, "/api/jokes/joke:a",
                    lambda b, u=r["unit"]: u in json.dumps(b))
    after("a version", [], "joke:a")
    st, r = write(via, "POST", "/api/jokes/joke:b/versions", {"text": f"Bravo with a draft flag {tag}.", "draft": True, "source": "draft"})
    ok(f"{tag} version-draft via {who}: learned", isinstance(r.get("learned"), dict), json.dumps(r)[:300])
    n_now += 2
    after("a kept version", [], "joke:b")

    # 7. one bit's decisions: a stage, a title with a reason, a retire, a confirm, Brainstorm's kept line
    for label, d in [("stage", {"target": "node", "id": "joke:c", "action": "edit", "after": {"stage": per["stage"]}}),
                     ("title", {"target": "node", "id": "joke:d", "action": "edit", "after": {"title": f"Delta, renamed {tag}"}, "reason": "renamed in the app"}),
                     ("retire", {"target": "node", "id": "joke:e", "action": "edit", "after": {"stage": "retired"}}),
                     ("confirm", {"target": "edge", "id": per["confirm"], "action": "confirm"}),
                     ("use as mine", {"target": "node", "action": "add", "reason": "kept from Brainstorm",
                                      "after": {"type": "unit", "text": f"A brainstorm line kept as mine {tag}.", "kind": "tag", "source": "draft"}})]:
        st, r = write(via, "POST", "/api/decision", d)
        ok(f"{tag} decision {label} via {who}: shape", r["ok"] and len(r["indexes"]) == 1 and r["decisions"][0]["action"] == d["action"]
           and r["decisions"][0]["by"] == "owner", json.dumps(r)[:300])
        if label == "use as mine":
            ok(f"{tag} decision {label} via {who}: learned is answered (null: no finished punch-up drafted it)",
               "learned" in r and r["learned"] is None, json.dumps(r.get("learned")))
        n_now += 1
        after(f"decision {label}", [], d.get("id") if d["target"] == "node" else "joke:c")
    for base in (rs, py):
        right_after(f"{tag} decisions via {who}", base, "/api/decisions?limit=1", lambda b, n=n_now: b["total"] == n)

    # 8. undo: the newest line (the Brainstorm unit), then by index (the retire)
    st, r = write(via, "POST", "/api/undo", {"index": None})
    ok(f"{tag} undo newest via {who}", r["ok"] and r["decisions"][0]["action"] == "delete" and r["decisions"][0]["undo_of"] == n_now - 1, json.dumps(r)[:300])
    n_now += 1
    retire_index = n_now - 4
    st, r = write(via, "POST", "/api/undo", {"indexes": [retire_index]})
    ok(f"{tag} undo by index via {who}", r["decisions"][0]["undo_of"] == retire_index and r["decisions"][0]["after"] == {"stage": "idea"}, json.dumps(r)[:300])
    minted["undone"] = retire_index
    n_now += 1
    after("two undos", [], "joke:e")

    # 9. merges: his attach, a dismiss (twice), a merge (with a typed note of his), both undone
    st, r = write(via, "POST", "/api/merge/attach", {"note": "joke:f", "into": "joke:a", "reason": " belongs here "})
    ok(f"{tag} attach via {who}", r["ok"] and r["merge_id"].startswith("at:") and r["merge"]["reasons"] == ["belongs here"]
       and r["merge"]["text"] == "A bit about f." and r["merge"]["source"] == "owner"
       and r["undo"] == {"POST": "/api/merge/undo", "merge_id": r["merge_id"]}, json.dumps(r)[:400])
    minted["his_attach"] = r["merge_id"]
    after("his attach", [f"/api/merge/{quote(r['merge_id'], safe='')}"], "joke:f")
    st, r = write(via, "POST", "/api/merge/dismiss", {"jokes": per["dismiss"], "reason": "not the same"})
    want_id = "dm:" + hashlib.sha1(json.dumps(sorted(set(per["dm"])), separators=(",", ":")).encode()).hexdigest()[:8]
    ok(f"{tag} dismiss via {who}", r == {"ok": True, "id": want_id}, json.dumps(r))
    minted["dismiss"] = r["id"]
    st, r2 = write(via, "POST", "/api/merge/dismiss", {"jokes": list(reversed(per["dm"]))})
    ok(f"{tag} dismiss again via {who}: already", r2 == {"ok": True, "id": want_id, "already": True}, json.dumps(r2))
    after("a dismiss", [f"/api/merge/{quote(want_id, safe='')}"], per["dm"][0])
    # the merge: a bit with a unit (x04), plus his typed jot from step 3 (a note with no version: kept as one first)
    st, r = write(via, "POST", "/api/merge", {"main": "joke:x00", "others": ["joke:x04", minted["from_jot"], "joke:x04"], "reason": "same dog bit", "group": "grp:test"})
    ok(f"{tag} merge via {who}: lines written, the typed note kept as a version",
       r["ok"] and r["merge_id"].startswith("mg:") and r["drive"]["state"] == "queued" and len(r["indexes"]) == 5
       and r["merge"]["others"] == ["joke:x04", minted["from_jot"]] and "snapshot" not in r["merge"], json.dumps(r)[:400])
    minted["merge"] = r["merge_id"]
    n_now += len(r["indexes"])
    after("a merge", [f"/api/merge/{quote(minted['merge'], safe='')}"], "joke:x00")
    both(f"{tag} after a merge via {who}", ["/api/jokes/joke:x04", "/api/jokes/joke:x04?include_hidden=1",
                                           f"/api/jokes/{quote(minted['from_jot'], safe='')}?include_hidden=1"])
    st, r = write(via, "POST", "/api/merge/undo", {"merge_id": minted["merge"]})
    ok(f"{tag} merge undo via {who}", r["ok"] and r["merge"]["status"] == "undone" and len(r["indexes"]) == 5 and r["drive"]["state"] == "undo_queued", json.dumps(r)[:300])
    n_now += len(r["indexes"])
    after("a merge undone", [f"/api/merge/{quote(minted['merge'], safe='')}"], "joke:x00")
    st, r = write(via, "POST", "/api/merge/undo", {"merge_id": minted["his_attach"]})
    ok(f"{tag} attach undo via {who}", r["ok"] and r["merge"]["status"] == "detached" and r["indexes"] == [], json.dumps(r)[:300])
    after("a note detached", [], "joke:f")
    # ---- round 2: groups 3 and 4 --------------------------------------------------------------------------
    # 10. a rating: the AI's match confirmed with it (two lines), the page's bit in the answer
    st, r = write(via, "POST", "/api/review/rate", {"perf": per["perf"], "rating": "killed", "reason": "big laugh"})
    ok(f"{tag} rate via {who}: the match confirmed and the rating written, the bit in the answer",
       r["ok"] and len(r["indexes"]) == 2 and r["decisions"][0]["action"] == "confirm"
       and r["decisions"][1]["after"]["rating"] == "killed" and "rating_auto" in r["decisions"][1]["after"]
       and r.get("joke", {}).get("id") and "hit_sets" in r["joke"] and "ready_for_working" in r["joke"], json.dumps(r)[:400])
    n_now += 2
    perf_joke = r["joke"]["id"]
    for base in (rs, py):
        right_after(f"{tag} rate via {who}", base, f"/api/jokes/{quote(perf_joke, safe='')}",
                    lambda b, j=perf_joke: b.get("id") == j and any(p.get("rating") == "killed" for p in b.get("performances", b.get("perfs", []))) or True)
    after("a rating", [f"/api/sets/{SX}/review", f"/api/sets/{SX}/analysis", "/api/sets"], perf_joke)
    # 11. the Set page's in-place edits: new material (the match rejected), re-matched to another bit, a moment marked
    st, r = write(via, "POST", "/api/set-row", {"perf": per["perf"], "op": "new"})
    ok(f"{tag} set-row new via {who}: the match rejected", r["ok"] and len(r["indexes"]) == 1 and r["decisions"][0]["action"] == "reject"
       and r["decisions"][0]["reason"] == "new material, not this joke", json.dumps(r)[:300])
    n_now += 1
    st, r = write(via, "POST", "/api/set-row", {"perf": per["perf"], "op": "match", "joke": per["match"], "reason": " it is this one "})
    ok(f"{tag} set-row match via {who}: the edge re-pointed", r["ok"] and r["decisions"][0]["action"] == "edit"
       and r["decisions"][0]["after"].get("dst") == per["match"] and r["decisions"][0]["reason"] == "it is this one", json.dumps(r)[:300])
    n_now += 1
    st, r = write(via, "POST", "/api/set-row", {"perf": MARK_PERF, "op": per["mark"]})
    ok(f"{tag} set-row {per['mark']} via {who}", r["ok"] and r["decisions"][0]["after"] == {"kind": {"crowd": "crowd_work"}.get(per["mark"], per["mark"])},
       json.dumps(r)[:300])
    n_now += 1
    st, r = write(via, "POST", "/api/set-row", {"perf": MARK_PERF, "op": "rate", "rating": per["mark_rating"]})
    ok(f"{tag} set-row rate via {who}: the rating alone (the match left as it is)", r["ok"] and len(r["indexes"]) == 1
       and r["decisions"][0]["after"]["rating"] == per["mark_rating"] and "joke" in r, json.dumps(r)[:300])
    n_now += 1
    after("the set page's edits", [f"/api/sets/{SX}/review", f"/api/sets/{SX}/analysis"], per["match"])
    # 12. the AI's match rejected from the review, then undone (so the other sequence can reject it again)
    st, r = write(via, "POST", "/api/review/reject-match", {"perf": MARK_PERF, "reason": "not that bit"})
    ok(f"{tag} reject-match via {who}", r["ok"] and r["decisions"][0]["action"] == "reject" and r["decisions"][0]["reason"] == "not that bit", json.dumps(r)[:300])
    n_now += 1
    st, r2 = write(via, "POST", "/api/undo", {"indexes": r["indexes"]})
    ok(f"{tag} reject-match undone via {who}", r2["ok"] and r2["decisions"][0]["undo_of"] == r["indexes"][0], json.dumps(r2)[:300])
    n_now += 1
    after("a match rejected and undone", [f"/api/sets/{SX}/review"], "joke:c")
    # 13. the Tonight list, then the same list again (nothing changed)
    st, r = write(via, "POST", "/api/tonight", {"items": per["tonight"], "reason": "set for tonight"})
    ok(f"{tag} tonight via {who}: saved, the list answered", r["ok"] and len(r["indexes"]) == 1 and r["tonight"]["source"] == "saved"
       and [x["id"] for x in r["tonight"]["items"]] == [x["joke"] for x in per["tonight"]], json.dumps(r)[:400])
    n_now += 1
    st_again = R.call(via, "POST", "/api/tonight", app, {"items": per["tonight"]})
    ok(f"{tag} tonight again via {who}: nothing changed", st_again[0] == 409 and body(st_again) == {"error": "nothing changed"}, st_again[2][:200])
    after("the tonight list", ["/api/tonight/screen", "/api/today"])
    # 14. a set list: made, changed in one body, a bit added after another, archived, brought back; a second one deleted
    st, r = write(via, "POST", "/api/boards", {"title": f"  New seven {tag}  ", "joke_ids": ["joke:a", "joke:b"], "target_s": 300, "notes": "n"}, want=(201,))
    bid = r["board"]["id"]
    ok(f"{tag} board made via {who}", bid.startswith("sb:new-seven") and r["board"]["title"] == f"New seven {tag}" and r["board"]["joke_ids"] == ["joke:a", "joke:b"]
       and r["board"]["source"] == "his" and "health" in r and "setlist" in r and "best_next" in r, json.dumps(r)[:400])
    minted["board"] = bid
    q_b = quote(bid, safe="")
    after("a set list made", ["/api/boards", f"/api/boards/{q_b}", f"/api/boards/{q_b}/build"])
    st, r = write(via, "PATCH", f"/api/boards/{q_b}", {"title": f"Seven {tag}", "add": {"joke_id": "joke:c", "at": 0}, "move": {"from": 0, "to": 2},
                                                   "target_s": 420, "notes": "changed"})
    ok(f"{tag} board patched via {who}: every op applied in order", r["board"]["title"] == f"Seven {tag}" and r["board"]["joke_ids"] == ["joke:a", "joke:b", "joke:c"]
       and r["board"]["target_s"] == 420 and r["board"]["notes"] == "changed", json.dumps(r["board"])[:300])
    st, r = write(via, "POST", f"/api/boards/{q_b}/items", {"joke_id": "joke:d", "after": "joke:a"})
    ok(f"{tag} board item via {who}: added after a bit", r["board"]["joke_ids"] == ["joke:a", "joke:d", "joke:b", "joke:c"], json.dumps(r["board"])[:300])
    st, r = write(via, "POST", f"/api/boards/{q_b}/items", {"joke_id": "joke:x01", "after": 0})
    ok(f"{tag} board item via {who}: added at a place", r["board"]["joke_ids"] == ["joke:a", "joke:x01", "joke:d", "joke:b", "joke:c"], json.dumps(r["board"])[:300])
    st, r = write(via, "PATCH", f"/api/boards/{q_b}", {"archive": True})
    ok(f"{tag} board archived via {who}", r["board"]["archived"] is True, json.dumps(r["board"])[:200])
    after("a set list archived", ["/api/boards", "/api/boards?archived=1", f"/api/boards/{q_b}", "/api/tonight/screen"])
    st, r = write(via, "PATCH", f"/api/boards/{q_b}", {"unarchive": True, "remove": "joke:x01"})
    ok(f"{tag} board back via {who}: brought back and edited in one go", r["board"]["archived"] is False and "joke:x01" not in r["board"]["joke_ids"], json.dumps(r["board"])[:200])
    st, r = write(via, "POST", f"/api/boards/{q_b}/bridge", {"from": "joke:a", "to": "joke:d"})
    ok(f"{tag} bridge via {who}: queued", st == 202 and r["status"] == "queued" and r["job"]["kind"] == "bridge", json.dumps(r)[:300])
    minted["bridge_job"] = r["job"]["id"]
    st, r = write(via, "POST", "/api/boards", {"title": f"Gone soon {tag}"}, want=(201,))
    st, r = write(via, "PATCH", f"/api/boards/{quote(r['board']['id'], safe='')}", {"delete": True, "title": "renamed first"})
    ok(f"{tag} board deleted via {who}: the title as it was", r == {"ok": True, "board": {"id": r["board"]["id"], "title": f"Gone soon {tag}", "deleted": True}}, json.dumps(r)[:200])
    after("the set lists", ["/api/boards", "/api/boards?archived=1", f"/api/boards/{q_b}", f"/api/boards/{q_b}/build",
                            f"/api/jobs/{quote(minted['bridge_job'], safe='')}"])
    # 15. his set: promoted from the list, a new version, a bit locked and unlocked, a line locked and unlocked, a try
    #     note added and ticked, one of the AI's ticked, the punch-ups
    st, r = write(via, "POST", "/api/workingsets", {"board_id": bid, "target_s": 240}, want=(201,))
    ws = r["working_set"]
    wid = ws["id"]
    ok(f"{tag} set promoted via {who}", wid.startswith("ws:seven") and ws["storyboard_id"] == bid and ws["current"] == 1
       and [b["joke_id"] for b in ws["versions"][0]["bits"]] == ["joke:a", "joke:d", "joke:b", "joke:c"]
       and ws["versions"][0]["target_s"] == 240 and all(b["target_s"] % 5 == 0 for b in ws["versions"][0]["bits"]), json.dumps(ws)[:400])
    minted["ws"] = wid
    q_w = quote(wid, safe="")
    st, r = write(via, "POST", f"/api/workingsets/{q_w}/versions", {"note": " v2: fewer ", "joke_ids": per["ws_ids"]}, want=(201,))
    ok(f"{tag} set version via {who}: the old bits kept, a new one built", r["working_set"]["current"] == 2
       and [b["joke_id"] for b in r["working_set"]["versions"][1]["bits"]] == per["ws_ids"] and r["working_set"]["versions"][1]["note"] == "v2: fewer", json.dumps(r["working_set"])[:400])
    st, r = write(via, "POST", f"/api/workingsets/{q_w}/lock", {"joke_id": "joke:a"})
    ok(f"{tag} bit locked via {who}", r["working_set"]["locks"]["joke:a"]["bit"] is True, json.dumps(r["working_set"].get("locks"))[:200])
    st, r = write(via, "POST", f"/api/workingsets/{q_w}/bits/joke:a/punchup", {})
    ok(f"{tag} punchup of a locked bit via {who}: answered here, no job", st == 200 and r["status"] == "locked" and r["drafts"] == [] and r["draft"] is True, json.dumps(r)[:300])
    st, r = write(via, "POST", f"/api/workingsets/{q_w}/lock", {"joke_id": "joke:a", "unlock": True})
    ok(f"{tag} bit unlocked via {who}", r["working_set"]["locks"]["joke:a"]["bit"] is False, json.dumps(r["working_set"].get("locks"))[:200])
    line = "The purple monkey dishwasher line here."
    st, r = write(via, "POST", f"/api/workingsets/{q_w}/lock", {"joke_id": "joke:b" if "joke:b" in per["ws_ids"] else "joke:a", "line": line}
                  if "joke:b" in per["ws_ids"] else {"joke_id": "joke:a", "line": "The alpha punch lands here every time."})
    locked_j = "joke:b" if "joke:b" in per["ws_ids"] else "joke:a"
    lk = r["working_set"]["locks"][locked_j]["lines"]
    ok(f"{tag} line locked via {who}: verbatim in the newest night's segment", len(lk) == 1 and lk[0]["node"].startswith("seg:"), json.dumps(lk)[:300])
    st, r = write(via, "POST", f"/api/workingsets/{q_w}/lock", {"joke_id": locked_j, "line": lk[0]["text"], "unlock": True})
    ok(f"{tag} line unlocked via {who}", r["working_set"]["locks"][locked_j]["lines"] == [], json.dumps(r["working_set"]["locks"])[:200])
    st, r = write(via, "POST", f"/api/workingsets/{q_w}/try", {"joke_id": "joke:a", "text": f" slower on the opener {tag} "}, want=(201,))
    nid = r["note"]["id"]
    ok(f"{tag} try note via {who}", nid.startswith("try:") and r["note"]["text"] == f"slower on the opener {tag}" and r["note"]["title"] == R.TITLE["a"]
       and r["note"]["source"] == "his" and r["working_set"] == wid, json.dumps(r)[:300])
    st, r = write(via, "PATCH", f"/api/workingsets/{q_w}/try/{quote(nid, safe='')}", {"done": True})
    ok(f"{tag} try note ticked via {who}", r["changed"] is True and r["note"]["done"] is True and r["note"]["source"] == "his", json.dumps(r)[:300])
    st, r = write(via, "PATCH", f"/api/workingsets/{q_w}/try/{quote(nid, safe='')}", {"done": True})
    ok(f"{tag} try note ticked again via {who}: unchanged", r["changed"] is False, json.dumps(r)[:300])
    st, r = write(via, "PATCH", f"/api/workingsets/{q_w}/try/joke:a%7Cput_back", {"done": True})
    ok(f"{tag} the AI's try note ticked via {who}", r["changed"] is True and r["note"] == {"id": "joke:a|put_back", "done": True, "source": "inferred"}, json.dumps(r)[:300])
    st, r = write(via, "POST", f"/api/workingsets/{q_w}/bits/joke:a/punchup", {})
    ok(f"{tag} punchup via {who}: queued", st == 202 and r["status"] == "queued" and r["job"]["kind"] == "punchup", json.dumps(r)[:300])
    minted["punchup_job"] = r["job"]["id"]
    st, r = write(via, "POST", "/api/workingsets/ws:main/bits/joke:d/punchup", {})
    ok(f"{tag} punchup of the bundle's locked bit via {who}", st == 200 and r["status"] == "locked", json.dumps(r)[:300])
    st, r = write(via, "POST", "/api/jokes/joke:f/punchup", {})
    ok(f"{tag} brainstorm with no tape via {who}", st == 200 and r["status"] == "no_tape" and r["why"] == "No taped night to work from.", json.dumps(r)[:300])
    st, r = write(via, "POST", "/api/jokes/joke:d/punchup", {})
    ok(f"{tag} brainstorm of a bit locked in any set via {who}", st == 200 and r["status"] == "locked", json.dumps(r)[:300])
    st, r = write(via, "POST", "/api/jokes/joke:a/punchup", {})
    ok(f"{tag} brainstorm via {who}: queued (the same key on both: one job)", st == 202 and r["job"]["kind"] == "punchup", json.dumps(r)[:300])
    after("his set", ["/api/workingsets", f"/api/workingsets/{q_w}", f"/api/workingsets/{q_w}/nights", "/api/mysets", f"/api/mysets/{q_w}",
                      f"/api/jobs/{quote(minted['punchup_job'], safe='')}", f"/api/jobs/{quote(r['job']['id'], safe='')}", "/api/today"])
    # 16. the settings, twice (the second changes nothing)
    st, r = write(via, "PATCH", "/api/settings", per["settings"])
    ok(f"{tag} settings via {who}", all(r[k] == v for k, v in per["settings"].items()) and r["source"] == "his", json.dumps(r))
    st, r2 = write(via, "PATCH", "/api/settings", per["settings"])
    ok(f"{tag} settings again via {who}: nothing written", r2 == r, json.dumps(r2))
    after("the settings", ["/api/settings"])
    # 17. the voice profile saved (the previous rides along), a draft dismissed
    st, r = write(via, "PUT", "/api/profile/voice", {"text": f"\nI talk fast {tag}.\r\nShort lines.\n\n"})
    ok(f"{tag} profile via {who}", r["text"] == f"I talk fast {tag}.\nShort lines.\n" and r["by"] == "owner" and r["by_kind"] == "owner"
       and r["chars"] == len(r["text"]) and "learned" in r and "previous" in r, json.dumps(r)[:300])
    st, r = write(via, "POST", "/api/profile/voice/learn", {"dropped": [f"A drafted line he passed on {tag}.", {"text": "another", "bit": "joke:a"}], "bit": "joke:b"})
    ok(f"{tag} learn dropped via {who}", r["ok"] and r["learned"]["dropped"] >= 2, json.dumps(r))
    after("his profile", ["/api/profile/voice"])
    # 18. "not a joke?": one kept (the second sequence keeps it again: already kept), one hidden
    st, r = write(via, "POST", "/api/suggestions/not-a-joke/keep", {"joke_id": "joke:x09"})
    ok(f"{tag} keep via {who}", r == {"ok": True, "joke_id": "joke:x09", "kept": True}, json.dumps(r))
    st, r = write(via, "POST", "/api/suggestions/not-a-joke/hide", {"joke_id": per["hide"]})
    ok(f"{tag} hide via {who}: a decision by him, undoable", r["ok"] and r["joke_id"] == per["hide"] and r["decisions"][0]["after"] == {"hidden": True}
       and r["decisions"][0]["by"] == "owner" and r["undo"] == {"POST": "/api/undo", "indexes": r["indexes"]}, json.dumps(r)[:300])
    n_now += 1
    after("the not-a-joke answers", ["/api/suggestions/not-a-joke", "/api/suggestions/not-a-joke?include_hidden=1"], per["hide"])
    # the merge candidates on both, after everything (a hidden bit, a kept one, the attaches of round 1)
    both(f"{tag} candidates after the writes via {who}", ["/api/merge/candidates?joke=joke:x00", "/api/merge/candidates?joke=joke:a",
                                                           "/api/merge/candidates?jokes=joke:x00,joke:x04,joke:a,joke:x05"])
    minted["n_decisions"] = n_now
    ok(f"{tag} via {who}: the decision count is what the sequence wrote",
       body(R.call(py, "GET", "/api/decisions?limit=1", app))["total"] == n_now)
    return minted


# ---------------------------------------------------------------- phase C: the same write to both servers
def raw_call(base: str, method: str, path: str, data: bytes, headers: dict):
    req = urllib.request.Request(base + path, data=data, method=method, headers={**headers, "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r_:
            return r_.status, {k.lower(): v for k, v in r_.headers.items()}, r_.read()
    except urllib.error.HTTPError as e:
        try:
            b = e.read()
        except OSError:  # the server answered and closed before the body was all sent (a refusal from Content-Length)
            b = b""
        return e.code, {k.lower(): v for k, v in e.headers.items()}, b
    except OSError:
        return 0, {}, b""


def same_write(label: str, method: str, path: str, b_py, b_rs=None, by_shape: bool = False) -> None:
    """The same request to both; the answers equal after normalisation (status, body, the headers the app reads)."""
    global n_equal
    a = R.call(py, method, path, app, b_py)
    b = R.call(rs, method, path, app, b_rs if b_rs is not None else b_py)
    bad = []
    if a[0] != b[0]:
        bad.append(f"{label}: status python {a[0]} rust {b[0]}: {a[2][:200]!r} / {b[2][:200]!r}")
    for h in R.HEADERS:
        if a[1].get(h) != b[1].get(h):
            bad.append(f"{label}: header {h}: python {a[1].get(h)!r} rust {b[1].get(h)!r}")
    f = shape if by_shape else norm
    for p, u, v in R.diff(f(body(a)), f(body(b))):
        bad.append(f"{label}: {p}: python {json.dumps(u)[:160]} rust {json.dumps(v)[:160]}")
    problems.extend(bad)
    n_equal += not bad


def refusals() -> None:
    """Requests both servers refuse, equal to the letter (nothing is written by a refusal)."""
    global n_equal
    long = "x" * 4001
    cases = [
        ("ideas: no body", "POST", "/api/ideas", None, None),
        ("ideas: empty text", "POST", "/api/ideas", {"text": "   "}, None),
        ("ideas: too long", "POST", "/api/ideas", {"text": long}, None),
        ("ideas: a list", "POST", "/api/ideas", [1], None),
        ("ideas: backend", "POST", "/api/ideas?backend=files", {"text": "x y z"}, None),
        ("ideas: no token", "POST", "/api/ideas", {"text": "x y z"}, {}),
        ("ideas: worker token", "POST", "/api/ideas", {"text": "x y z"}, worker_tok),
        ("transcript: no segments", "POST", "/api/sets/transcript", {}, None),
        ("transcript: a bool start", "POST", "/api/sets/transcript", {"segments": [{"text": "a", "start": True}]}, None),
        ("transcript: empty", "POST", "/api/sets/transcript", {"segments": [{"text": " "}]}, None),
        ("transcript: bad title", "POST", "/api/sets/transcript", {"segments": [{"text": "a"}], "title": 3}, None),
        ("transcript: bad date", "POST", "/api/sets/transcript", {"segments": [{"text": "a"}], "recorded_at": "soon"}, None),
        ("transcript: bad venue", "POST", "/api/sets/transcript", {"segments": [{"text": "a"}], "venue": "v" * 121}, None),
        ("transcript: bad duration", "POST", "/api/sets/transcript", {"segments": [{"text": "a"}], "duration_s": -3}, None),
        ("versions: no text", "POST", "/api/jokes/joke:a/versions", {}, None),
        ("versions: too long", "POST", "/api/jokes/joke:a/versions", {"text": long}, None),
        ("versions: no such joke", "POST", "/api/jokes/joke:nope/versions", {"text": "a line"}, None),
        ("versions: a unit", "POST", "/api/jokes/unit:a1/versions", {"text": "a line"}, None),
        ("versions: hidden bit", "POST", "/api/jokes/joke:g/versions", {"text": "a line"}, None),
        ("versions: court text", "POST", "/api/jokes/joke:a/versions", {"text": "The petitioner and the respondent walked in."}, None),
        ("decision: bad id", "POST", "/api/decision", {"target": "node", "id": "joke a", "action": "edit", "after": {"stage": "idea"}}, None),
        ("decision: bad target", "POST", "/api/decision", {"target": "x", "action": "edit"}, None),
        ("decision: unknown node", "POST", "/api/decision", {"target": "node", "id": "joke:nope", "action": "edit", "after": {"stage": "idea"}}, None),
        ("decision: bad stage", "POST", "/api/decision", {"target": "node", "id": "joke:a", "action": "edit", "after": {"stage": "flying"}}, None),
        ("decision: edge add without a reason", "POST", "/api/decision", {"target": "edge", "action": "add", "after": {"src": "joke:a", "dst": "joke:b", "kind": "callback_to"}}, None),
        ("decision: hidden node", "POST", "/api/decision", {"target": "node", "id": "joke:g", "action": "edit", "after": {"stage": "idea"}}, None),
        ("decision: reason too long", "POST", "/api/decision", {"target": "node", "id": "joke:a", "action": "edit", "after": {"stage": "idea"}, "reason": long}, None),
        ("undo: not a list", "POST", "/api/undo", {"indexes": 3}, None),
        ("undo: a float", "POST", "/api/undo", {"indexes": [1.5]}, None),
        ("undo: no such decision", "POST", "/api/undo", {"index": 999999}, None),
        ("undo: negative", "POST", "/api/undo", {"index": -1}, None),
        ("attach: bad ids", "POST", "/api/merge/attach", {"note": "joke a", "into": "joke:a"}, None),
        ("attach: itself", "POST", "/api/merge/attach", {"note": "joke:a", "into": "joke:a"}, None),
        ("attach: unknown", "POST", "/api/merge/attach", {"note": "joke:nope", "into": "joke:a"}, None),
        ("attach: already attached", "POST", "/api/merge/attach", {"note": "joke:x05", "into": "joke:a"}, None),
        ("dismiss: one joke", "POST", "/api/merge/dismiss", {"jokes": ["joke:a"]}, None),
        ("dismiss: not ids", "POST", "/api/merge/dismiss", {"jokes": ["joke:a", 3]}, None),
        ("merge: no main", "POST", "/api/merge", {"others": ["joke:a"]}, None),
        ("merge: no others", "POST", "/api/merge", {"main": "joke:a", "others": []}, None),
        ("merge: main in others", "POST", "/api/merge", {"main": "joke:a", "others": ["joke:b", "joke:a"]}, None),
        ("merge: unknown", "POST", "/api/merge", {"main": "joke:a", "others": ["joke:nope"]}, None),
        ("merge: hidden", "POST", "/api/merge", {"main": "joke:a", "others": ["joke:g"]}, None),
        ("merge undo: missing", "POST", "/api/merge/undo", {}, None),
        ("merge undo: no such", "POST", "/api/merge/undo", {"merge_id": "mg:nope1234"}, None),
        ("merge undo: detached already", "POST", "/api/merge/undo", {"merge_id": "at:x09"}, None),
        ("merge undo: a dismiss", "POST", "/api/merge/undo", {"merge_id": "dm:x02"}, None),
        # round 2
        ("rate: bad rating", "POST", "/api/review/rate", {"perf": f"perf:{SX}@0", "rating": "great"}, None),
        ("rate: unknown perf", "POST", "/api/review/rate", {"perf": "perf:nope", "rating": "meh"}, None),
        ("rate: already", "POST", "/api/review/rate", {"perf": MARK_PERF, "rating": "killed"}, None),
        ("set-row: bad op", "POST", "/api/set-row", {"perf": f"perf:{SX}@0", "op": "zap"}, None),
        ("set-row: unknown perf", "POST", "/api/set-row", {"perf": "perf:nope", "op": "crowd"}, None),
        ("set-row: match unknown joke", "POST", "/api/set-row", {"perf": f"perf:{SX}@0", "op": "match", "joke": "joke:nope"}, None),
        ("set-row: nothing to confirm", "POST", "/api/set-row", {"perf": CROWD_PERF, "op": "confirm"}, None),
        ("set-row: already marked", "POST", "/api/set-row", {"perf": MARK_PERF, "op": PER_TAG["(B)"]["mark"]}, None),
        ("reject-match: no match", "POST", "/api/review/reject-match", {"perf": CROWD_PERF}, None),
        ("tonight: not a list", "POST", "/api/tonight", {"items": 3}, None),
        ("tonight: too many", "POST", "/api/tonight", {"items": [{"joke": "joke:a"}] * 61}, None),
        ("tonight: unknown joke", "POST", "/api/tonight", {"items": [{"joke": "joke:nope", "done": 1}]}, None),
        ("tonight: nothing changed", "POST", "/api/tonight", {"items": [{"joke": "joke:x02"}]}, None),
        ("boards: no title", "POST", "/api/boards", {"joke_ids": []}, None),
        ("boards: bad notes", "POST", "/api/boards", {"title": "x", "notes": 5}, None),
        ("boards: bad joke id", "POST", "/api/boards", {"title": "x", "joke_ids": ["joke a"]}, None),
        ("boards: unknown joke", "POST", "/api/boards", {"title": "x", "joke_ids": ["joke:nope"]}, None),
        ("boards: duplicates", "POST", "/api/boards", {"title": "x", "joke_ids": ["joke:a", "joke:a"]}, None),
        ("boards: bad target", "POST", "/api/boards", {"title": "x", "target_s": 5}, None),
        ("boards: too many", "POST", "/api/boards", {"title": "x", "joke_ids": ["joke:a"] * 201}, None),
        ("board patch: nothing", "PATCH", "/api/boards/sb:old", {}, None),
        ("board patch: unknown", "PATCH", "/api/boards/sb:nope", {"title": "x"}, None),
        ("board patch: deleted", "PATCH", "/api/boards/sb:gone", {"title": "x"}, None),
        ("board patch: bad title", "PATCH", "/api/boards/sb:old", {"title": "  "}, None),
        ("board patch: move out of range", "PATCH", "/api/boards/sb:old", {"move": {"from": 0, "to": 99}}, None),
        ("board patch: remove missing", "PATCH", "/api/boards/sb:old", {"remove": "joke:c"}, None),
        ("board patch: add twice", "PATCH", "/api/boards/sb:old", {"add": "joke:a"}, None),
        ("board items: already", "POST", "/api/boards/sb:old/items", {"joke_id": "joke:a"}, None),
        ("board items: not on board", "POST", "/api/boards/sb:old/items", {"joke_id": "joke:c", "after": "joke:d"}, None),
        ("board items: bad after", "POST", "/api/boards/sb:old/items", {"joke_id": "joke:c", "after": 999}, None),
        ("bridge: unknown board", "POST", "/api/boards/sb:nope/bridge", {"from": "joke:a", "to": "joke:b"}, None),
        ("bridge: unknown joke", "POST", "/api/boards/sb:old/bridge", {"from": "joke:a", "to": "joke:nope"}, None),
        ("ws create: no board", "POST", "/api/workingsets", {"board_id": "sb:nope"}, None),
        ("ws create: empty board", "POST", "/api/workingsets", {"board_id": "sb:empty"}, None),
        ("ws create: no id", "POST", "/api/workingsets", {}, None),
        ("ws version: no note", "POST", "/api/workingsets/ws:main/versions", {}, None),
        ("ws version: empty ids", "POST", "/api/workingsets/ws:main/versions", {"note": "n", "joke_ids": []}, None),
        ("ws version: unknown set", "POST", "/api/workingsets/ws:nope/versions", {"note": "n"}, None),
        ("ws lock: not in set", "POST", "/api/workingsets/ws:main/lock", {"joke_id": "joke:e"}, None),
        ("ws lock: bad line", "POST", "/api/workingsets/ws:main/lock", {"joke_id": "joke:a", "line": 3}, None),
        ("ws lock: empty line", "POST", "/api/workingsets/ws:main/lock", {"joke_id": "joke:a", "line": "  "}, None),
        ("ws lock: line not verbatim", "POST", "/api/workingsets/ws:main/lock", {"joke_id": "joke:a", "line": "not a line anyone said tonight"}, None),
        ("ws lock: node not verbatim", "POST", "/api/workingsets/ws:main/lock", {"joke_id": "joke:a", "line": "Alpha opener line one.", "node": "unit:b1"}, None),
        ("ws lock: unknown set", "POST", "/api/workingsets/ws:nope/lock", {"joke_id": "joke:a"}, None),
        ("ws try: no text", "POST", "/api/workingsets/ws:main/try", {"joke_id": "joke:a"}, None),
        ("ws try: not in set", "POST", "/api/workingsets/ws:main/try", {"joke_id": "joke:e", "text": "x"}, None),
        ("ws try: too long", "POST", "/api/workingsets/ws:main/try", {"text": "x" * 501}, None),
        ("ws try patch: not bool", "PATCH", "/api/workingsets/ws:main/try/try:nope", {"done": "yes"}, None),
        ("ws try patch: no such", "PATCH", "/api/workingsets/ws:main/try/try:nope", {"done": True}, None),
        ("punchup: unknown joke", "POST", "/api/workingsets/ws:main/bits/joke:nope/punchup", {}, None),
        ("punchup: unknown set", "POST", "/api/workingsets/ws:nope/bits/joke:a/punchup", {}, None),
        ("joke punchup: unknown", "POST", "/api/jokes/joke:nope/punchup", {}, None),
        ("joke punchup: bad id", "POST", "/api/jokes/joke%20a/punchup", {}, None),
        ("settings: empty", "PATCH", "/api/settings", {}, None),
        ("settings: unknown", "PATCH", "/api/settings", {"foo": 1, "bar": 2}, None),
        ("settings: bad provider", "PATCH", "/api/settings", {"provider": "x"}, None),
        ("settings: not bool", "PATCH", "/api/settings", {"training": "yes"}, None),
        ("profile: no text", "PUT", "/api/profile/voice", {}, None),
        ("profile: blank", "PUT", "/api/profile/voice", {"text": " \n "}, None),
        ("profile: too long", "PUT", "/api/profile/voice", {"text": "x" * 60001}, None),
        ("profile: court", "PUT", "/api/profile/voice", {"text": "The petitioner and the respondent walked in."}, None),
        ("profile: worker token", "PUT", "/api/profile/voice", {"text": "x"}, worker_tok),
        ("learn: empty", "POST", "/api/profile/voice/learn", {"dropped": []}, None),
        ("learn: too many", "POST", "/api/profile/voice/learn", {"dropped": ["x"] * 11}, None),
        ("learn: no text", "POST", "/api/profile/voice/learn", {"dropped": [{"bit": "joke:a"}]}, None),
        ("learn: too long", "POST", "/api/profile/voice/learn", {"dropped": ["x" * 2001]}, None),
        ("learn: court", "POST", "/api/profile/voice/learn", {"dropped": ["The petitioner and the respondent walked in."]}, None),
        ("keep: bad id", "POST", "/api/suggestions/not-a-joke/keep", {"joke_id": "unit:a1"}, None),
        ("keep: no id", "POST", "/api/suggestions/not-a-joke/keep", {}, None),
        ("hide: unknown", "POST", "/api/suggestions/not-a-joke/hide", {"joke_id": "joke:nope"}, None),
        ("hide: bad id", "POST", "/api/suggestions/not-a-joke/hide", {"joke_id": "joke a"}, None),
        ("candidates: bad joke", "GET", "/api/merge/candidates?joke=joke%20a", None, None),
        ("candidates: bad jokes", "GET", "/api/merge/candidates?jokes=joke:a,joke%20b", None, None),
        ("candidates: too many", "GET", "/api/merge/candidates?jokes=" + ",".join(f"joke:x{i:02d}" for i in range(51)), None, None),
    ]
    for label, m, p, b, h in cases:
        a, c = R.call(py, m, p, h if h is not None else app, b), R.call(rs, m, p, h if h is not None else app, b)
        got = R.compare(label, a, c)
        problems.extend(got)
        n_equal += not got
        ok(f"{label}: a refusal", a[0] >= 400, f"the Python answered {a[0]}")
    for label, p, data in [("ideas: not JSON", "/api/ideas", b"{not json"), ("decision: not JSON", "/api/decision", b"[")]:
        got = R.compare(label, raw_call(py, "POST", p, data, app), raw_call(rs, "POST", p, data, app))
        problems.extend(got)
        n_equal += not got
    # a body over MAX_BODY (256 KiB): both refuse it from `Content-Length` alone, before reading it (the Python's
    # `body_json`), so the client may lose the connection while it is still sending: the status is what is compared
    big = json.dumps({"segments": [{"text": "x" * 300_000}]}).encode()
    codes = [raw_call(base, "POST", "/api/sets/transcript", big, app)[0] for base in (py, rs)]
    ok("transcript: a body over 256 KiB is refused unread by both", codes == [400, 400], str(codes))


# ---------------------------------------------------------------- phase D: the rows
def check_rows(port: int, pw: str, a_rows: tuple[int, int], b_rows: tuple[int, int]) -> None:
    d = R.surreal_sql(port, pw, "SELECT * FROM decision ORDER BY seq;")[-1]["result"]
    c = R.surreal_sql(port, pw, "SELECT * FROM creative_event ORDER BY store, seq;")[-1]["result"]
    ok("rows: decision seq is 0..n with no gap", [x["seq"] for x in d] == list(range(len(d))), f"{len(d)} rows")
    by_store: dict = {}
    for x in c:
        by_store.setdefault(x["store"], []).append(x)
    for store, xs in by_store.items():
        ok(f"rows: creative_event {store} seq is 0..n with no gap", [x["seq"] for x in xs] == list(range(len(xs))), f"{len(xs)} rows")
        for x in xs:
            ev = json.loads(x["line"])
            ok(f"rows: creative_event {store}/{x['seq']} h, op, item, at come from the line",
               x["h"] == hashlib.sha256(x["line"].encode("utf-8")).hexdigest() and x.get("op") == ev.get("op")
               and x.get("item") == ev.get("id") and x.get("at") == ev.get("ts"))
            ok(f"rows: creative_event {store}/{x['seq']} line keys as creative/store.py writes them",
               list(ev.keys()) == ["ts", "op", "id", "by", "data"], ",".join(ev.keys()))
    a = [x for x in d if a_rows[0] <= x["seq"] < a_rows[1]]
    b = [x for x in d if b_rows[0] <= x["seq"] < b_rows[1]]
    ok("rows: the two sequences wrote the same number of decision rows", len(a) == len(b), f"pk-api {len(a)} vs Python {len(b)}")
    for x, y in zip(a, b):
        kx = {k: v for k, v in x.items() if k not in ("id", "seq")}
        ky = {k: v for k, v in y.items() if k not in ("id", "seq")}
        drop_null = lambda v: {k: drop_null(x) for k, x in v.items() if x is not None} if isinstance(v, dict) else v  # noqa: E731
        dd = R.diff(shape(drop_null(norm(kx))), shape(drop_null(norm(ky))))
        stable = all(norm(kx.get(k)) == norm(ky.get(k)) for k in ("target", "action", "by", "reason"))
        ok(f"rows: decision {x['seq']} (pk-api) and {y['seq']} (Python) have the same shape and stable values", not dd and stable,
           "; ".join(f"{p}: {u!r} vs {v!r}" for p, u, v in dd[:4]) or f"{norm(kx)} vs {norm(ky)}"[:300])
    ok("rows: the jobs store carries the set_text jobs both servers queued",
       sum(1 for x in by_store.get("jobs", []) if '"set_text"' in x["line"]) >= 2)
    ok("rows: the profile store carries learn events from both sequences", len(by_store.get("profile", [])) >= 6)
    ok("rows: the storyboards, workingsets, account and suggestions stores carry both sequences' events",
       all(len(by_store.get(n, [])) >= 2 for n in ("storyboards", "workingsets", "account", "suggestions")),
       str({n: len(by_store.get(n, [])) for n in ("storyboards", "workingsets", "account", "suggestions")}))
    ok("rows: the merges store carries both sequences' attach, dismiss, merge, undo and detach",
       sorted({json.loads(x["line"])["op"] for x in by_store.get("merges", [])}) >= ["attach", "detach", "dismiss", "merge", "undo"])


# ---------------------------------------------------------------- phase F: two writers at once
def hammer(conc: int, each: int) -> None:
    n0 = body(R.call(py, "GET", "/api/decisions?limit=1", app))["total"]
    errors: list[str] = []
    texts: list[str] = []
    lock = threading.Lock()

    def worker(w: int) -> None:
        for i in range(each):
            base = rs if (w + i) % 2 else py
            text = f"Hammer jot {w}-{i}: the {['meter', 'gate', 'floss', 'toast'][w % 4]} that argues back at dawn number {w * 100 + i}."
            r = R.call(base, "POST", "/api/ideas", app, {"text": text})
            with lock:
                if r[0] != 200:
                    errors.append(f"worker {w} write {i} via {'pk-api' if base == rs else 'python'}: HTTP {r[0]} {r[2][:200]!r}")
                else:
                    texts.append(text)

    ts = [threading.Thread(target=worker, args=(w,)) for w in range(conc)]
    t0 = time.perf_counter()
    for t in ts:
        t.start()
    for t in ts:
        t.join()
    wall = time.perf_counter() - t0
    problems.extend(errors[:10])
    n = body(R.call(py, "GET", "/api/decisions?limit=1", app))["total"]
    ok(f"hammer: {conc} writers x {each} jots through both servers at once ({wall:.1f} s), every line landed once",
       not errors and n == n0 + conc * each, f"{n - n0} new decisions for {conc * each} writes, {len(errors)} errors: {errors[:3]}")
    items_py = body(R.call(py, "GET", "/api/jokes?limit=5000", app))["items"]
    items_rs = body(R.call(rs, "GET", "/api/jokes?limit=5000", app))["items"]
    want = {title7(t) for t in texts}
    seen_py = [x["title"] for x in items_py if x["title"] in want]
    seen_rs = [x["title"] for x in items_rs if x["title"] in want]
    ok("hammer: both servers list every jot exactly once",
       sorted(seen_py) == sorted(want) and sorted(seen_rs) == sorted(want), f"python {len(seen_py)} rust {len(seen_rs)} of {len(want)}")
    both("hammer", ["/api/decisions?limit=100", "/api/jokes?limit=5000", "/api/today"])
    errors.clear()

    def tworker(w: int) -> None:
        for i in range(2):
            base = rs if (w + i) % 2 else py
            r = R.call(base, "POST", "/api/sets/transcript", app, {"segments": [{"text": f"Hammer set {w}-{i} about the bus.", "start": 0, "end": 5}]})
            with lock:
                if r[0] != 202:
                    errors.append(f"transcript worker {w} {i}: HTTP {r[0]} {r[2][:200]!r}")

    ts = [threading.Thread(target=tworker, args=(w,)) for w in range(conc)]
    for t in ts:
        t.start()
    for t in ts:
        t.join()
    problems.extend(errors[:10])
    ok("hammer: transcripts at once all queued", not errors, f"{len(errors)} errors")


# ---------------------------------------------------------------- the run
def main() -> int:
    global app, py, rs, worker_tok
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pk-api", required=True, type=Path)
    ap.add_argument("--report", type=Path, default=None)
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("--conc", type=int, default=8)
    a = ap.parse_args()

    tmp = Path(tempfile.mkdtemp(prefix="pk-t10-"))
    comedy, data, home = tmp / "comedy", tmp / "data", tmp / "home"
    for d in (data, home, tmp / "cache"):
        d.mkdir()
    R.build_comedy(comedy)
    R.write_vectors(tmp / "cache" / "kb", R.make_graph())
    judge = R.write_judge(comedy, tmp / "cache" / "kb")
    port = R.free_port()
    pw = secrets.token_hex(16)
    env = {**os.environ, "PUNCHABLE_HOME": str(home), "COMEDY_DIR": str(comedy), "PUNCH_COMEDY": str(comedy),
           "PK_CACHE_DIR": str(tmp / "cache"), "PK_DBLOAD_DIR": str(tmp / "dbload"), "SURREAL_PORT": str(port),
           "SURREAL_USER": "root", "SURREAL_PASS": pw, "SURREAL_NS": R.NS, "SURREAL_DB": R.DB, "PYTHONHASHSEED": "0",
           "PUNCH_API_QUIET": "1", "PYTHONIOENCODING": "utf-8", "PUNCH_SECRETS": str(home / "secrets.env"),
           # neither server reads the owner's private rules file: the generic markers alone, as on the hosted pod
           "PUNCHABLE_LEGAL_RULES": str(home / "no-rules.json")}
    for k in ("SURREAL_URL", "SURREAL_API_USER", "SURREAL_API_PASS", "SENTRY_DSN", "PK_EMBEDDER"):
        env.pop(k, None)
    bge = env.get("PK_BGE_DIR", "")
    if not (Path(bge) / "onnx" / "model.onnx").is_file():
        raise SystemExit("set PK_BGE_DIR to the bge-small folder (python tools/kb/fetch_bge.py --dest <dir>)")
    env["PK_VEC_SWEEP_S"] = "86400"
    procs: list[subprocess.Popen] = []
    logs = []

    def start(args, extra, name):
        log = open(tmp / f"{name}.log", "w", encoding="utf-8")
        logs.append(log)
        p = subprocess.Popen(args, env={**env, **extra}, stdout=log, stderr=subprocess.STDOUT, cwd=str(ROOT))
        procs.append(p)
        return p

    report: dict = {}
    try:
        sp = start([R.surreal_exe(), "start", "--user", "root", "--pass", pw, "--bind", f"127.0.0.1:{port}", "memory"], {}, "surreal")
        R.wait_http(f"http://127.0.0.1:{port}/health", sp, "surreal")
        tgz = tmp / "bundle.tgz"
        for step in (["pack", "--comedy", str(comedy), "--tar", str(tgz)], ["apply", "--tar", str(tgz)]):
            r = subprocess.run([sys.executable, str(ROOT / "db" / "push_remote.py"), *step], env=env, cwd=str(ROOT),
                               capture_output=True, text=True, encoding="utf-8")
            if r.returncode != 0:
                raise SystemExit(f"push_remote {step[0]} failed:\n{r.stdout[-2000:]}\n{r.stderr[-3000:]}")
        R.write_merges(port, pw)
        key = JK.Key("t10", seed=9)
        jwks = R.JwksServer([key])
        auth_env = {"PK_AUTH": "1", "OIDC_ISSUER": JK.ISS, "OIDC_AUDIENCE": JK.AUD, "OIDC_JWKS_URL": jwks.url,
                    "PUBLIC_URL": R.ORIGIN + "/", "PK_APP_FLAGS": R.FLAGS, "PK_DATA": str(data), "HOST": "127.0.0.1",
                    "PK_BOOT_POLL_S": "1"}
        now = time.time()
        app = {"Authorization": "Bearer " + JK.token(key, now=now)}
        worker_tok = {"Authorization": "Bearer " + JK.token(key, now=now, realm_access={"roles": ["worker"]})}

        def pair(name: str, with_model: bool) -> tuple[subprocess.Popen, subprocess.Popen]:
            global py, rs
            e = dict(auth_env)
            if not with_model:
                env.pop("PK_BGE_DIR", None)
            py_port, rs_port = R.free_port(), R.free_port()
            py, rs = f"http://127.0.0.1:{py_port}", f"http://127.0.0.1:{rs_port}"
            pp_ = start([sys.executable, str(ROOT / "api" / "entry.py")], {**e, "PORT": str(py_port)}, f"python-api-{name}")
            R.wait_http(py + "/api/health?ready=1", pp_, "python api")
            rp_ = start([str(a.pk_api)], {**e, "PORT": str(rs_port), "PK_PYTHON_URL": py, "RUST_LOG": os.environ.get("RUST_LOG", "info")}, f"pk-api-{name}")
            R.wait_http(rs + "/api/health?ready=1", rp_, "pk-api")
            return pp_, rp_

        pp, rp = pair("model", True)
        deadline = time.time() + 300
        while True:
            vec = ((body(R.call(py, "GET", "/api/backend", app)).get("backends") or {}).get("surreal") or {}).get("vectors") or {}
            if vec.get("runs", 0) >= 1:
                break
            if time.time() > deadline:
                raise SystemExit(f"the Python's vector sweep did not finish: {vec}")
            time.sleep(0.5)
        # pk-api's model loads on its first use: a search warms it, so the jots' vectors are there from the first one
        R.call(rs, "GET", "/api/search?q=dog", app)

        # A. through pk-api; B. through the Python
        write(py, "POST", "/api/tonight", {"items": [{"joke": "joke:d"}, {"joke": "joke:e"}]})
        write(py, "POST", "/api/set-row", {"perf": MARK_PERF, "op": "banter"})
        write(py, "POST", "/api/set-row", {"perf": MARK_PERF, "op": "rate", "rating": "worked"})
        n_a0 = body(R.call(py, "GET", "/api/decisions?limit=1", app))["total"]
        minted_a = sequence(rs, "(A)")
        minted_b = sequence(py, "(B)")
        report["minted"] = {"pk_api": minted_a, "python": minted_b}
        report["judged_pairs"] = len(judge)
        ok("the judge file carries verdicts for the candidates to read", len(judge) >= 1, str(len(judge)))
        ok("the two servers placed the jots alike (with the model)",
           minted_a["jot_attach"] == minted_b["jot_attach"] and minted_a["words_jot_attach"] == minted_b["words_jot_attach"],
           f"{minted_a['jot_attach']}/{minted_a['words_jot_attach']} vs {minted_b['jot_attach']}/{minted_b['words_jot_attach']}")
        # C. the same write to both, then every refusal
        same_write("same: a jot", "POST", "/api/ideas", {"text": "Same jot, the dentist drill that hums (P)."}, {"text": "Same jot, the dentist drill that hums (R)."})
        same_write("same: a jot from a tape", "POST", "/api/ideas",
                   {"text": "Same tape jot about the bus (P).", "from": {"set": f"set:{SX}", "start_ms": 1000}},
                   {"text": "Same tape jot about the bus (R).", "from": {"set": f"set:{SX}", "start_ms": 1000}})
        same_write("same: a kept draft", "POST", "/api/ideas", {"text": "Same kept draft (P).", "draft": True}, {"text": "Same kept draft (R).", "draft": True})
        same_write("same: a transcript", "POST", "/api/sets/transcript", {"segments": [{"text": "Same set (P)."}]}, {"segments": [{"text": "Same set (R)."}]})
        same_write("same: a version", "POST", "/api/jokes/joke:c/versions", {"text": "Same version (P).", "reason": "r"}, {"text": "Same version (R).", "reason": "r"})
        same_write("same: a kept version", "POST", "/api/jokes/joke:c/versions", {"text": "Same kept version (P).", "draft": True}, {"text": "Same kept version (R).", "draft": True})
        same_write("same: a stage", "POST", "/api/decision", {"target": "node", "id": "joke:x30", "action": "edit", "after": {"stage": "testing"}},
                   {"target": "node", "id": "joke:x31", "action": "edit", "after": {"stage": "testing"}}, by_shape=True)
        same_write("same: a title", "POST", "/api/decision", {"target": "node", "id": "joke:x30", "action": "edit", "after": {"title": "Same (P)"}, "reason": "renamed"},
                   {"target": "node", "id": "joke:x31", "action": "edit", "after": {"title": "Same (R)"}, "reason": "renamed"}, by_shape=True)
        same_write("same: brainstorm kept", "POST", "/api/decision",
                   {"target": "node", "action": "add", "after": {"type": "unit", "text": "Same brainstorm (P).", "kind": "tag", "source": "draft"}},
                   {"target": "node", "action": "add", "after": {"type": "unit", "text": "Same brainstorm (R).", "kind": "tag", "source": "draft"}})
        same_write("same: undo newest", "POST", "/api/undo", {"index": None})
        same_write("same: attach", "POST", "/api/merge/attach", {"note": "joke:x11", "into": "joke:x03"}, {"note": "joke:x15", "into": "joke:x03"})
        same_write("same: dismiss", "POST", "/api/merge/dismiss", {"jokes": ["joke:x13", "joke:x14"]}, {"jokes": ["joke:x16", "joke:x20"]})
        same_write("same: merge", "POST", "/api/merge", {"main": "joke:x17", "others": ["joke:x21"], "reason": "r"}, {"main": "joke:x18", "others": ["joke:x22"], "reason": "r"})
        same_write("same: a rating in place", "POST", "/api/set-row", {"perf": MARK_PERF, "op": "rate", "rating": "worked"},
                   {"perf": MARK_PERF, "op": "rate", "rating": "killed"}, by_shape=True)
        same_write("same: the tonight list", "POST", "/api/tonight", {"items": [{"joke": "joke:x01"}]}, {"items": [{"joke": "joke:x02"}]}, by_shape=True)
        same_write("same: a set list", "POST", "/api/boards", {"title": "Same list (P)", "joke_ids": ["joke:a", "joke:c"]},
                   {"title": "Same list (R)", "joke_ids": ["joke:a", "joke:c"]})
        same_write("same: a bridge", "POST", "/api/boards/sb:old/bridge", {"from": "joke:a", "to": "joke:b"}, {"from": "joke:b", "to": "joke:x02"})
        _, bp = write(py, "POST", "/api/boards", {"title": "Same bits (P)", "joke_ids": ["joke:a", "joke:c"], "target_s": 300}, want=(201,))
        _, br = write(rs, "POST", "/api/boards", {"title": "Same bits (R)", "joke_ids": ["joke:a", "joke:c"], "target_s": 300}, want=(201,))
        same_write("same: a set promoted", "POST", "/api/workingsets", {"board_id": bp["board"]["id"]}, {"board_id": br["board"]["id"]})
        same_write("same: the settings", "PATCH", "/api/settings", {"training": False})
        same_write("same: the profile", "PUT", "/api/profile/voice", {"text": "Same profile (P)."}, {"text": "Same profile (R)."}, by_shape=True)
        same_write("same: a dismissed draft", "POST", "/api/profile/voice/learn", {"dropped": ["Same dropped (P)."]}, {"dropped": ["Same dropped (R)."]}, by_shape=True)
        same_write("same: a keep", "POST", "/api/suggestions/not-a-joke/keep", {"joke_id": "joke:x10"})
        same_write("same: a hide", "POST", "/api/suggestions/not-a-joke/hide", {"joke_id": "joke:x11"}, {"joke_id": "joke:x12"}, by_shape=True)
        same_write("same: a brainstorm with no tape", "POST", "/api/jokes/joke:f/punchup", {})
        both("after all", ["/api/merge/candidates", "/api/merge/candidates?joke=joke:x11", "/api/merge/candidates?jokes=joke:x10,joke:x11,joke:x12"])
        refusals()
        same_write("same: undo an undone line", "POST", "/api/undo", {"index": minted_a["undone"]})
        # D. the rows
        check_rows(port, pw, (n_a0, minted_a["n_decisions"]), (minted_a["n_decisions"], minted_b["n_decisions"]))
        # F. two writers at once, then the counters and the Python's stores
        hammer(a.conc, 5)
        mtext = R.call(rs, "GET", "/metrics")[2].decode()
        for method, route, status in (("POST", "/api/ideas", 200), ("POST", "/api/sets/transcript", 202), ("POST", "/api/jokes/{id}/versions", 200),
                                      ("POST", "/api/decision", 200), ("POST", "/api/undo", 200), ("POST", "/api/merge", 200),
                                      ("POST", "/api/merge/attach", 200), ("POST", "/api/merge/dismiss", 200), ("POST", "/api/merge/undo", 200),
                                      # round 2
                                      ("POST", "/api/review/rate", 200), ("POST", "/api/set-row", 200), ("POST", "/api/review/reject-match", 200),
                                      ("POST", "/api/tonight", 200), ("POST", "/api/tonight", 409), ("POST", "/api/boards", 201),
                                      ("PATCH", "/api/boards/{id}", 200), ("POST", "/api/boards/{id}/items", 200), ("POST", "/api/boards/{id}/bridge", 202),
                                      ("POST", "/api/workingsets", 201), ("POST", "/api/workingsets/{id}/versions", 201),
                                      ("POST", "/api/workingsets/{id}/lock", 200), ("POST", "/api/workingsets/{id}/try", 201),
                                      ("PATCH", "/api/workingsets/{id}/try/{id}", 200), ("POST", "/api/workingsets/{id}/bits/{id}/punchup", 202),
                                      ("POST", "/api/workingsets/{id}/bits/{id}/punchup", 200), ("POST", "/api/jokes/{id}/punchup", 200),
                                      ("POST", "/api/jokes/{id}/punchup", 202), ("PATCH", "/api/settings", 200), ("OTHER", "/api/profile/voice", 200),
                                      ("POST", "/api/profile/voice/learn", 200), ("POST", "/api/suggestions/not-a-joke/keep", 200),
                                      ("POST", "/api/suggestions/not-a-joke/hide", 200), ("GET", "/api/merge/candidates", 200),
                                      ("GET", "/api/merge/candidates", 400)):
            ok(f"metrics: pk-api answered {method} {route} ({status}) itself", f'method="{method}",route="{route}",status="{status}"' in mtext)
        n0 = sum(float(ln.rsplit(" ", 1)[1]) for ln in mtext.splitlines() if ln.startswith("pk_http_requests_total{"))
        R.call(rs, "POST", "/api/jobs/job:nope/drop", app, {})
        mtext2 = R.call(rs, "GET", "/metrics")[2].decode()
        n1 = sum(float(ln.rsplit(" ", 1)[1]) for ln in mtext2.splitlines() if ln.startswith("pk_http_requests_total{"))
        ok("metrics: a forwarded write (POST /api/jobs/<id>/drop) is counted by nobody here", n1 == n0)
        plog = (tmp / "python-api-model.log").read_text(encoding="utf-8", errors="replace")
        ok("the Python's creative stores never diverged from the database", "differ from event" not in plog, plog[-600:])
        report["rss_mib"] = {"python": R.rss_mib(pp.pid), "rust": R.rss_mib(rp.pid)}
        # G. no model on either side: a jot in a bit's exact words is attached under it by both
        for p_ in (rp, pp):
            p_.terminate()
            p_.wait(15)
        pair("words", False)
        got = {}
        for base, who, text, into, title in ((rs, "pk-api", R.TEXT["f"], "joke:f", R.TITLE["f"]), (py, "python", R.TEXT["b"], "joke:b", R.TITLE["b"])):
            st, r = write(base, "POST", "/api/ideas", {"text": text})
            got[who] = r.get("attach") or {}
            ok(f"no model: a jot in a bit's exact words is attached under it by {who}",
               got[who].get("decision") == "attach" and got[who].get("into") == into and got[who].get("record", {}).get("source") == "app"
               and got[who].get("reasons", [None])[0] == f'reads like your bit "{title}"', json.dumps(got[who])[:400])
            both(f"no model: after {who}'s attached jot", ["/api/jokes?limit=5000", "/api/jokes?limit=5000&include_hidden=1",
                                                          f"/api/jokes/{quote(r['id'], safe='')}?include_hidden=1",
                                                          f"/api/merge/history?joke={into}", f"/api/merge/{quote(got[who].get('id', 'at:none'), safe='')}"])
        ok("no model: both servers gave the same shape of reasons", [len(got[w].get("reasons", [])) for w in ("pk-api", "python")] == [2, 2],
           f"{got['pk-api'].get('reasons')} vs {got['python'].get('reasons')}")
        same_write("no model: a jot nobody wrote", "POST", "/api/ideas", {"text": "Nobody wrote this before, a new bit (P)."}, {"text": "Nobody wrote this before, a new bit (R)."})
    finally:
        for p in reversed(procs):
            p.terminate()
            try:
                p.wait(10)
            except subprocess.TimeoutExpired:
                p.kill()
        for lg in logs:
            lg.close()
        if not a.keep:
            shutil.rmtree(tmp, ignore_errors=True)
        else:
            print(f"kept {tmp}", file=sys.stderr)
    report["checks_equal"] = n_equal
    report["problems"] = problems
    out = json.dumps(report, indent=1)
    print(out)
    if a.report:
        a.report.write_text(out + "\n", encoding="utf-8", newline="\n")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
