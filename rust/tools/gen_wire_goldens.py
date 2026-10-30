"""Goldens for pk-wire: the Python API's `/api/worker/*` protocol and the app-facing response shapes, as the API
really writes them, over a SYNTHETIC knowledge base. The Rust tests (rust/crates/pk-wire/tests/goldens.rs) read each
golden into its struct, write it back and compare as parsed JSON, so a field the struct drops, renames or retypes
shows up there. Synthetic data only: six invented bits, three invented nights, no hostnames, nothing of the comic's.

    python rust/tools/gen_wire_goldens.py        (from the repo root; writes rust/crates/pk-wire/tests/goldens/)

How it runs the API: api-dev/server.py's Handler is served in this process on a loopback port (no subprocess, no
auth, the files backend, the creative routes, jobs queued for a worker), over a temp folder built here the way
db/tests/common.py's TempKB lays one out (graph.json, kb/, corpus/sets/<set>/..., an inbox, a creative dir), with
PUNCHABLE_HOME pointed at an empty temp folder (as conftest.py does for every test) so nothing private is read.
Determinism: every clock the routes stamp (kb.now_iso, creative.util.now_iso, today's date) is pinned to NOW, the
process re-runs itself with PYTHONHASHSEED=0, the embedder is off (no node, no model), and nothing reads the network
(the geocoder runs offline). Running it twice must produce no diff.

Each golden is one response body (or one request body the worker sends), named <module>_<what>.json. Two values are
normalised before writing, for the public repo's commit guard (tools/guard/check.py), never a shape: the geocoder's
`cache` (an absolute path on this machine) becomes "geocode_cache.json", and a decision line's `h` (bare 64-hex on
the wire, which the guard reads as a secret) is written as "sha256:<hex>". No YouTube set is in the fixture: the
guard refuses any watch URL, so `import_youtube` and the `youtube` reference have no golden (both are PC-only and
go at T11).
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
from datetime import date
from http.server import ThreadingHTTPServer
from pathlib import Path
from urllib.parse import quote
from urllib.request import Request, urlopen
from urllib.error import HTTPError

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "rust" / "crates" / "pk-wire" / "tests" / "goldens"
NOW = "2026-10-29T12:00:00+00:00"
TODAY = date(2026, 10, 29)
OWNER = "owner"

if os.environ.get("PYTHONHASHSEED") != "0":  # set-iteration order must not depend on the run
    sys.exit(subprocess.run([sys.executable, str(Path(__file__).resolve())], env={**os.environ, "PYTHONHASHSEED": "0"}).returncode)

# ---------------------------------------------------------------- the environment, before any module is imported
HOME = Path(tempfile.mkdtemp(prefix="pk-wire-home-"))
os.environ["PUNCHABLE_HOME"] = str(HOME)
for _k in ("PUNCHABLE_LEGAL_RULES", "PUNCHABLE_VENUES", "PK_COMIC_NAME", "PK_COMIC_FIRST", "PK_CACHE_DIR",
           "PK_INFER_JUDGE_DIR", "PK_AUTH", "PK_JOBS", "PK_GEOCODE", "SENTRY_DSN", "PK_APP_FLAGS", "PUNCHABLE_CREATIVE_DIR",
           "COMEDY_DIR", "PUNCH_COMEDY", "PK_GSYNC_DIR"):
    os.environ.pop(_k, None)
os.environ["PK_OWNER_ID"] = OWNER
os.environ["PUNCH_API_QUIET"] = "1"
for p in (ROOT, ROOT / "db", ROOT / "tools" / "explorer", ROOT / "api-dev"):  # api-dev first: the explorer has a server.py too
    sys.path.insert(0, str(p))

import creative.util as CU  # noqa: E402

CU.now_iso = lambda: NOW  # before anything does `from creative.util import now_iso`
import kb as K  # noqa: E402

K.now_iso = lambda: NOW
import creative.today as CT  # noqa: E402


class _Today:
    @staticmethod
    def today():
        return TODAY


CT._date = _Today
import server as SV  # noqa: E402
import creative_api as CR  # noqa: E402
import merge_api as MGA  # noqa: E402

MGA.embed = lambda text: None  # no local embedder here: an idea attaches by words alone
from creative.story import DRAFT_LABEL  # noqa: E402

# ---------------------------------------------------------------- the synthetic knowledge base
TEXT = {
    "a": "Alpha opener line one. The alpha punch lands here every time.",
    "b": "Bravo setup about the dog. The purple monkey dishwasher line here. Bravo closer words.",
    "c": "Charlie premise about cars and traffic and more traffic today.",
    "d": "Delta bit about my niece and the laundromat place.",
    "e": "Echo bit about my niece and her soccer schedule.",
    "f": "Foxtrot never performed idea about planes and pilots up there.",
}
TITLE = {"a": "Alpha bit", "b": "Bravo bit (the dog one)", "c": "Charlie bit", "d": "Delta bit", "e": "Echo bit", "f": "Foxtrot bit"}
STAGE = {"a": "testing", "b": "working", "c": "testing", "d": "to_try", "e": "idea", "f": "idea"}
UNIT_DATE = {"a": "2024-01-10", "b": "2023-05-01", "c": "2024-04-04", "d": "2015-03-03", "e": "2024-08-08", "f": "2016-07-07"}
SX, SY, SZ = "2024-01-01_x", "2024-02-01_y", "2024-03-01_z"


def set_dir(key: str) -> str:
    return key[:4] + key[5:7] + key[8:10] + key[10:]


def make_graph() -> dict:
    nodes, edges = [], []
    for k in "abcdef":
        nodes.append({"id": f"joke:{k}", "type": "joke", "title": TITLE[k], "stage": STAGE[k], "summary": f"A bit about {k}.",
                      "first_seen": "2023-01-01", "last_seen": UNIT_DATE[k], "lead_unit": f"unit:{k}1"})
        nodes.append({"id": f"unit:{k}1", "type": "unit", "kind": "setup+punch", "text": TEXT[k], "date": UNIT_DATE[k],
                      "source": "gdoc", "doc_title": "Notes"})
        edges.append({"id": f"e:p{k}", "src": f"unit:{k}1", "dst": f"joke:{k}", "kind": "part_of", "status": "inferred",
                      "confidence": 0.9, "evidence": []})
    nodes.append({"id": "unit:a0", "type": "unit", "kind": "setup+punch", "text": "Alpha opener, first draft.", "date": "2023-02-02"})
    edges.append({"id": "e:pa0", "src": "unit:a0", "dst": "joke:a", "kind": "part_of", "status": "proposed", "confidence": 0.6, "evidence": []})
    for lid, kind, name in [("label:persona:dad", "persona", "Dad"), ("label:topic:kids", "topic", "Kids"),
                            ("label:topic:cars", "topic", "Cars"), ("label:device:d1", "device", "D-one"),
                            ("label:device:d2", "device", "D-two"), ("label:structure:s1", "structure", "S-one")]:
        nodes.append({"id": lid, "type": "label", "kind": kind, "name": name, "definition": f"{name}, as a label"})

    def lab(j, l, w, q):
        edges.append({"id": f"e:l{j}-{l.split(':')[-1]}", "src": f"joke:{j}", "dst": l, "kind": "has_label", "status": "proposed",
                      "confidence": w, "reason": f"{q} reads as {l.split(':')[-1]}", "evidence": [{"node": f"unit:{j}1", "quote": q}]})
    lab("a", "label:device:d1", 0.9, "Alpha opener")
    lab("b", "label:device:d1", 0.9, "purple monkey")
    lab("c", "label:device:d2", 0.9, "Charlie premise")
    lab("d", "label:persona:dad", 0.8, "my niece")
    lab("e", "label:persona:dad", 0.8, "my niece")
    lab("d", "label:topic:kids", 0.8, "laundromat")
    lab("e", "label:topic:kids", 0.8, "soccer schedule")
    lab("c", "label:topic:cars", 0.8, "cars and traffic")
    edges.append({"id": "e:seg_bc", "src": "joke:b", "dst": "joke:c", "kind": "segue", "status": "proposed", "confidence": 0.7,
                  "reason": "dog leads to cars", "evidence": [{"node": "unit:b1", "quote": "Bravo closer words."},
                                                              {"node": "unit:c1", "quote": "Charlie premise"}]})
    edges.append({"id": "e:cb_ad", "src": "joke:a", "dst": "joke:d", "kind": "callback_to", "status": "proposed", "confidence": 0.5,
                  "reason": "the opener comes back", "evidence": [{"node": "unit:a1", "quote": "The alpha punch"}]})
    for key, title, venue, dur in ((SX, "Corner Room, Tuesday", "The Corner Room", 300),
                                   (SY, "Seaside open mic", "Seaside Comedy Club", 240),
                                   (SZ, "Mabel's, March", "Mabel's (upstairs)", 120)):
        nodes.append({"id": f"set:{key}", "type": "set", "title": title, "venue": venue, "date": key[:10], "duration_s": dur})
    # the first night's performances are graph nodes with proposed matches (the overlay confirms them from the analysis);
    # the other nights' come from analysis.json alone (the overlay adds them)
    for start, end, text, kind, name, joke in ((0, 60000, TEXT["a"], "bit", "Alpha", "a"), (60000, 120000, TEXT["b"], "bit", "Bravo", "b"),
                                                (120000, 140000, "So who here has a car?", "crowd_work", "Crowd", None),
                                                (140000, 200000, TEXT["c"], "bit", "Charlie", "c")):
        pid = f"perf:{SX}@{start}"
        nodes.append({"id": pid, "type": "performance", "set": f"set:{SX}", "start_ms": start, "end_ms": end, "text": text,
                      "kind": kind, "name": name})
        if joke:
            edges.append({"id": f"e:pa-{joke}", "src": pid, "dst": f"joke:{joke}", "kind": "performed_as", "status": "proposed",
                          "confidence": 0.8, "reason": f"the {name.lower()} words", "evidence": [{"node": pid, "quote": text.split(".")[0] + ".", "start_ms": start + 1000}]})
    return {"version": "wire-test", "generated_at": "2026-10-01", "runs": ["r1"], "nodes": nodes, "edges": edges}


def row(idx, joke, start_s, dur_s, text, laugh_s, set_rate, status="known", ttfl=5.0, punch=None, similar=None, hints=None):
    window = float(dur_s)
    lspm = round(laugh_s / (window / 60), 2) if window else 0.0
    return {"idx": idx, "start_ms": int(start_s * 1000), "end_ms": int((start_s + dur_s) * 1000), "title": f"T{idx}",
            "status": status, "joke_id": joke, "version_unit_id": f"unit:{joke[5:]}1" if joke else None,
            "match_confidence": 0.9 if joke else None, "match_reason": "same words" if joke else "", "transcript": text,
            "punchlines": punch or [], "laughs": {"count": 1 if laugh_s else 0, "max_bars": 2 if laugh_s else 0, "total_s": laugh_s,
                                                 "merged_count": 1 if laugh_s else 0, "laugh_s": laugh_s, "window_s": window},
            "laugh_s_per_min": lspm, "pct_laughing": round(laugh_s / window, 3), "time_to_first_laugh_s": ttfl if laugh_s else None,
            "dead_air_max_s": 5.0, "vs_room": round(lspm / set_rate, 2), "ai_rating": "worked" if laugh_s else "died",
            "rating_basis": "the laugh after the punch" if laugh_s else "silence", "similar": similar or [], "hint_labels": hints or [],
            "script_diff": {"on_script_pct": 50, "dropped": [], "added": []}}


def analysis(key, rate, rows):
    return {"set": key, "run_id": "synthetic-1", "generated_at": "2026-10-01T00:00:00+00:00",
            "summary": {"laugh_s_per_min": rate, "room_heat": 1.0, "room_heat_basis": "set", "material_s": 300.0, "laugh_s_total": 24.0,
                        "pct_laughing": 0.08, "laughs_merged": 4, "laughs_per_min_merged": 0.8, "applause_s": 0.0, "longest_laugh_s": 3.0,
                        "longest_laugh_joke": rows[0]["joke_id"], "dead_air_max_s": 9.0, "dead_air_max_at_ms": 100000, "duration_s": 300,
                        "best": None, "weakest": None, "jokes_counted": sum(1 for r in rows if r["status"] in ("known", "new")),
                        "new_material": sum(1 for r in rows if r["status"] == "new")},
            "jokes": rows}


def laugh(i, start_ms, bars, strength=0.8):
    return {"id": f"l{i}", "start_ms": start_ms, "end_ms": start_ms + 1000 * bars, "bars": bars, "strength": strength,
            "kind": "laugh", "confidence": 0.9, "segment": "set"}


SETS = {
    SX: {
        "analysis": analysis(SX, 6.0, [
            row(1, "joke:a", 0, 60, TEXT["a"], 9.0, 6.0, punch=[{"text": "The alpha punch lands here every time.", "at_ms": 8000, "laugh_id": "l1", "bars": 3}],
                similar=[{"joke_id": "joke:d", "why": "the same opener shape", "confidence": 0.6}], hints=[{"label_id": "label:device:d1", "why": "a hard turn"}]),
            row(2, "joke:b", 60, 60, TEXT["b"], 6.0, 6.0, punch=[{"text": "The purple monkey dishwasher line here.", "at_ms": 70000, "laugh_id": "l2", "bars": 3}]),
            row(3, None, 120, 20, "So who here has a car?", 0.0, 6.0, status="crowd_work"),
            row(4, "joke:c", 140, 60, TEXT["c"], 6.0, 6.0, punch=[{"text": "and more traffic today.", "at_ms": 150000, "laugh_id": "l3", "bars": 1}])]),
        "transcript": [(0, 8, "Alpha opener line one."), (8, 20, "The alpha punch lands here every time."), (60, 70, "Bravo setup about the dog."),
                       (70, 80, "The purple monkey dishwasher line here."), (80, 90, "Bravo closer words."), (120, 130, "So who here has a car?"),
                       (140, 150, "Charlie premise about cars and traffic and more traffic today."), (200, 206, "Thanks everybody, you were a dream.")],
        "laughs": [laugh(1, 20500, 3, 0.9), laugh(2, 80500, 3, 0.95), laugh(3, 150500, 1, 0.5), laugh(4, 206500, 2, 0.7)],
        "meta": {"title": "Corner Room, Tuesday", "venue_guess": "The Corner Room", "show_date": "2024-01-01",
                 "show_date_source": "the folder date", "duration": 300.0},
        "clips": {"set": set_dir(SX), "run_id": "clips-1", "generated_at": "2026-10-01T00:00:00+00:00", "show_date": "2024-01-01",
                  "venue": "The Corner Room", "signals_present": ["laugh", "vs_room"], "signals_missing": ["applause"],
                  "params": {"weights": {"laugh": 1.0, "vs_room": 0.5}, "scales": {"laugh": 3.0}, "min_s": 8, "max_s": 40, "max_overlap": 0.2,
                             "keep": 2, "candidates": 3},
                  "llm": None,
                  "clips": [{"rank": 1, "start_ms": 60000, "end_ms": 84000, "duration_s": 24.0, "joke_id": "joke:b", "title": TITLE["b"],
                             "status": "known", "ai_rating": "worked", "score": 0.91,
                             "score_breakdown": {"laugh": {"raw": 3.0, "norm": 1.0, "weight": 1.0, "contribution": 1.0},
                                                 "vs_room": {"raw": 1.0, "norm": 0.5, "weight": 0.5, "contribution": 0.25}},
                             "why": "the biggest laugh of the night", "punchline": "The purple monkey dishwasher line here.",
                             "punch_at_ms": 70000, "laugh_id": "l2", "transcript": TEXT["b"]},
                            {"rank": 2, "start_ms": 0, "end_ms": 24000, "duration_s": 24.0, "joke_id": "joke:a", "title": TITLE["a"],
                             "status": "known", "ai_rating": "worked", "score": 0.7,
                             "score_breakdown": {"laugh": {"raw": 3.0, "norm": 1.0, "weight": 1.0, "contribution": 1.0},
                                                 "vs_room": {"raw": None, "norm": 0.0, "weight": 0.5, "contribution": 0.0}},
                             "why": "a strong opener", "punchline": "The alpha punch lands here every time.", "punch_at_ms": 8000,
                             "laugh_id": "l1", "transcript": TEXT["a"]}]},
    },
    SY: {
        "analysis": analysis(SY, 6.0, [
            row(1, "joke:a", 0, 60, TEXT["a"], 8.4, 6.0),
            row(2, "joke:b", 60, 90, "Bravo setup about the dog. Bravo closer words.", 2.0, 6.0, ttfl=40.0),
            row(3, None, 150, 60, TEXT["d"], 3.0, 6.0, status="new", punch=[{"text": "the laundromat place.", "at_ms": 170000, "laugh_id": "l1", "bars": 2}])]),
        "transcript": [(0, 10, "Alpha opener line one."), (10, 20, "The alpha punch lands here every time."), (60, 90, "Bravo setup about the dog."),
                       (90, 120, "Bravo closer words."), (150, 170, "Delta bit about my niece and the laundromat place.")],
        "laughs": [laugh(1, 170500, 2), laugh(2, 20500, 2)],
        "meta": {"source": "app_upload", "original_name": "seaside.m4a", "recorded_at": "2024-02-01T21:00:00", "title": "Seaside open mic",
                 "show_venue": "Seaside Comedy Club", "duration_s": 240.0, "sha256": "0" * 64},
    },
    SZ: {
        "analysis": analysis(SZ, 5.0, [
            row(1, "joke:b", 0, 40, "Bravo setup about the dog. Bravo closer words about the cat today.", 1.5, 5.0, ttfl=30.0),
            row(2, "joke:a", 40, 30, "Alpha opener line one.", 4.0, 5.0),
            row(3, None, 70, 10, "Thanks for coming out tonight folks.", 0.0, 5.0, status="banter")]),
        "transcript": [(0, 20, "Bravo setup about the dog."), (20, 40, "Bravo closer words about the cat today."), (40, 70, "Alpha opener line one."),
                       (70, 80, "Thanks for coming out tonight folks.")],
        "laughs": [laugh(1, 70500, 2)],
        "meta": {"source": "voice_memo", "drive_id": "memo0001", "original_name": "mabels-march.m4a", "title": "Mabel's, March",
                 "venue_guess": "Mabel's (upstairs)", "show_date": "2024-03-01", "show_date_source": "the title", "duration": 120.0},
    },
}
INFERRED = {"run_id": "inf-1", "generated_at": "2026-10-01T00:00:00+00:00", "notes": "synthetic",
            "jokes": [{"id": "joke:a", "stage": "working", "stage_confidence": 0.8, "stage_reason": "three nights, two hits"},
                      {"id": "joke:f", "stage": "idea", "stage_confidence": 0.6, "stage_reason": "never taped"}]}
UPLOAD_NAME = "2024-02-01_punchable-set_210000_seaside.m4a"
UPLOAD_META = {"kind": "set", "original_name": "seaside.m4a", "title": "Seaside open mic", "received_at": NOW, "bytes": 12, "duration_s": 240.0,
               "marks_ms": [61000, 150000], "status": "queued", "recorded_at": "2024-02-01T21:00:00", "venue": "Seaside Comedy Club",
               "gps": {"lat": 51.5, "lon": -0.1}, "show_group": None, "align_working_set": None}


def write_json(p: Path, obj) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(obj, ensure_ascii=False, indent=1), encoding="utf-8", newline="\n")


def build_fixture(root: Path) -> None:
    write_json(root / "graph.json", make_graph())
    (root / "kb").mkdir()
    (root / "kb" / "decisions.jsonl").write_text("", encoding="utf-8")
    write_json(root / "kb" / "inferred.json", INFERRED)
    for key, s in SETS.items():
        d = root / "corpus" / "sets" / set_dir(key)
        write_json(d / "analysis.json", s["analysis"])
        write_json(d / "transcript.json", {"segments": [{"start": a, "end": b, "text": t} for a, b, t in s["transcript"]]})
        write_json(d / "laughs_auto.json", {"detector": "synthetic", "run_id": "laughs-1", "laughs": s["laughs"]})
        write_json(d / "meta.json", s["meta"])
        if "clips" in s:
            write_json(d / "clips.json", s["clips"])
    (root / "inbox").mkdir()
    (root / "inbox" / UPLOAD_NAME).write_bytes(b"not really audio")
    write_json(root / "inbox" / (UPLOAD_NAME + ".meta.json"), UPLOAD_META)
    (root / "tapes").mkdir()
    (root / "creative").mkdir()


# ---------------------------------------------------------------- the API, in this process
class Api:
    def __init__(self, root: Path):
        kb = SV.AppKB(root / "kb", root / "corpus", root / "tapes", root / "inbox", None, graph=root / "graph.json", id_map=None)
        SV.Handler.kbs = {"files": kb}
        SV.Handler.primary = "files"
        SV.Handler.verifier = None
        SV.Handler.origins = ()
        SV.Handler.jobs_on_upload = False
        SV.Handler.files_mode = "disk"
        SV.Handler.creative = CR.Creative(root / "creative", None, geocode=False, jobs_mode="worker")
        self.kb, self.cr = kb, SV.Handler.creative
        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), SV.Handler)
        self.srv.daemon_threads = True
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.srv.server_address[1]}"

    def call(self, method: str, path: str, body=None, raw: bool = False):
        data = json.dumps(body).encode("utf-8") if body is not None else None
        h = {"Accept": "application/json", "User-Agent": "punchable-worker/1"}
        if data is not None:
            h["Content-Type"] = "application/json"
        req = Request(self.url + path, data=data, method=method, headers=h)
        try:
            with urlopen(req, timeout=60) as r:
                b = r.read()
                if r.status == 204:
                    return 204, None
                return r.status, (b if raw else json.loads(b))
        except HTTPError as e:
            b = e.read()
            return e.code, (b if raw else json.loads(b or b"{}"))

    def stop(self):
        self.srv.shutdown()
        self.srv.server_close()


# ---------------------------------------------------------------- goldens
def gold(name: str, obj) -> None:
    write_json(OUT / f"{name}.json", obj)


def expect(status: int, want: int, what: str, body=None) -> None:
    if status != want:
        raise SystemExit(f"{what}: HTTP {status}, wanted {want}: {str(body)[:300]}")


def q(s: str) -> str:
    return quote(s, safe=":")


def sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


# what the hosted worker would post back per kind (worker/execute.py run -> the API's `result`): the real shapes of
# creative/write.py riff_ideas / check_facts, engine/ask.py ask, creative/story.py bridge_draft, creative/workingset.py
# punch_up, with invented words
def result_for(job: dict) -> dict:
    kind, inp = job["kind"], job.get("input") or {}
    if kind == "riff":
        return {"idea": inp["idea"], "draft": True, "label": DRAFT_LABEL, "status": "ok",
                "ideas": [{"text": "The meter wants a tip now.", "built_on": ["riff", "joke:a"], "move": "escalation", "draft": True, "label": DRAFT_LABEL},
                          {"text": "It only takes quarters and apologies.", "built_on": ["riff"], "move": None, "draft": True, "label": DRAFT_LABEL}],
                "grounded_in": [{"id": "B1", "joke_id": "joke:a", "title": TITLE["a"], "quote": "The alpha punch lands here every time.",
                                 "text": TEXT["a"], "stage_line": "Worked at Mabel's, Mar 2024 · 3 nights",
                                 "last_night": "last night at Mabel's, Mar 2024: worked, 4 s of laughs"}],
                "provider": "anthropic", "model": "claude-synthetic"}
    if kind == "facts":
        return {"idea": inp["text"], "draft": True, "label": "Checked by the AI against its sources · read the source", "status": "ok",
                "facts": [{"text": "A parking meter was first installed in 1935.", "source_title": "Parking meter", "source_url": "https://example.invalid/parking-meter"}],
                "dropped": 1, "provider": "anthropic", "model": "claude-synthetic"}
    if kind == "ask":
        return {"question": inp["question"], "plan": {"by_year": False, "kinds": ["written", "performed"]}, "run_id": "ask-1",
                "sources": ["unit:a1", "perf:2024-01-01_x@0"], "found": True,
                "answer": "You did the alpha bit at the Corner Room [unit:a1] and it worked [perf:2024-01-01_x@0].",
                "not_found": "", "citations": [{"source_id": "unit:a1", "quote": "Alpha opener line one."}], "removed": [],
                "timings": {"retrieve_s": 0.1, "answer_s": 1.2, "total_s": 1.3}, "tokens_in": 1200, "tokens_out": 80,
                "provider": "anthropic:claude-synthetic"}
    if kind == "bridge":
        return {"from": inp["from"], "to": inp["to"], "from_title": TITLE[inp["from"][5:]], "to_title": TITLE[inp["to"][5:]], "transition": "link",
                "draft": True, "label": DRAFT_LABEL, "status": "ok", "text": "Speaking of things that come back, my niece.",
                "grounded_in": [{"id": "L1", "kind": "callback_to", "edge_id": "e:cb_ad", "name": "the opener comes back",
                                 "evidence": [{"node": "unit:a1", "quote": "The alpha punch"}]}],
                "all_links": ["L1"], "provider": "anthropic", "model": "claude-synthetic", "call_id": "c-1", "task": "creative_bridge"}
    if kind == "punchup":
        return {"joke_id": inp["joke_id"], "title": TITLE[inp["joke_id"][5:]], "draft": True, "label": DRAFT_LABEL,
                "drafts": [{"target_line": "Alpha opener line one.", "target_evidence": {"node": "perf:2024-01-01_x@0", "quote": "Alpha opener line one."},
                            "alt": "Alpha opener, line one, take two.", "move": "rule of three", "draft": True, "label": DRAFT_LABEL}],
                "status": "ok", "blocked_locked": 0, "based_on": f"set:{SX}", "provider": "anthropic", "model": "claude-synthetic",
                "call_id": "c-2", "task": "creative_punchup"}
    if kind == "process_set":
        return {"status": "processed", "report": {"status": "ok", "kind": "set", "set": set_dir(SY), "set_id": f"set:{SY}",
                                                  "stats": {"segments": 5, "laughs": 2}, "totals": {"duration_s": 240.0}, "date": "2024-02-01",
                                                  "venue": "Seaside Comedy Club"}}
    if kind == "set_text":
        audio = {"status": "unavailable", "why": "transcribed on the phone; laughs, room heat and timing need the recording", "timestamps": "device"}
        return {"status": "analyzed_text", "kind": "set", "set_dir": "20240501_ph0123456ab", "units": 2, "funny_units": 1, "audio": audio,
                "run_id": "set_text-1", "ideas": [{"unit": "u1", "joke_id": "joke:the-meter-wants-a-tip-now", "attach": "attach"}]}
    if kind == "ask_chat":
        return {"status": "answered", "data": {"found": True, "answer": "At the Corner Room, and it worked [S1].",
                                               "citations": [{"source": "S1", "quote": "Alpha opener line one."}]},
                "provider": "anthropic", "model": "claude-synthetic"}
    if kind == "embed":
        return {"status": "embedded", "summary_only": 2, "embedded": 2, "removed": 0, "have": 9, "file": "vectors/extra_summary.json"}
    if kind == "memolink":
        return {"status": "linked", "embed": {"summary_only": 0, "embedded": 0, "removed": 0, "have": 9, "file": "vectors/extra_summary.json"},
                "judge": {"judged": 1, "cached": 0, "same": 1}, "triage": {"triaged": 1, "cached": 0}, "attached": ["at:f00dbabe12"],
                "same_idea_groups": 0}
    raise SystemExit(f"no synthetic result for kind {kind}")


HOSTED_KINDS = ["bridge", "punchup", "facts", "riff", "ask_chat", "set_text", "embed", "memolink", "ask"]


def main() -> int:
    root = Path(tempfile.mkdtemp(prefix="pk-wire-"))
    OUT.mkdir(parents=True, exist_ok=True)
    for p in OUT.glob("*.json"):
        p.unlink()
    build_fixture(root)
    api = Api(root)
    c = api.call
    try:
        # ---------------------------------------------------------- the app writes first (so every read has state)
        s, written = c("POST", "/api/decision", {"target": "edge", "action": "confirm", "id": "e:la-d1", "reason": "yes, that is the move"})
        expect(s, 200, "decision confirm", written)
        gold("api_written", written)
        s, w2 = c("POST", "/api/decision", {"target": "node", "action": "edit", "id": "joke:c", "after": {"stage": "working"}})
        expect(s, 200, "decision edit", w2)
        s, rated = c("POST", "/api/set-row", {"perf": f"perf:{SX}@60000", "op": "rate", "rating": "killed"})
        expect(s, 200, "set-row rate", rated)
        s, tonight = c("POST", "/api/tonight", {"items": [{"joke": "joke:a", "done": False}, {"joke": "joke:d", "done": True}]})
        expect(s, 200, "tonight", tonight)
        s, board = c("POST", "/api/boards", {"title": "Tuesday run", "joke_ids": ["joke:a", "joke:b", "joke:c", "joke:d"], "target_s": 300,
                                             "notes": "the four taped bits"})
        expect(s, 201, "board create", board)
        gold("creative_board_view", board)
        bid = board["board"]["id"]
        s, ws = c("POST", "/api/workingsets", {"board_id": bid, "target_s": 300})
        expect(s, 201, "workingset create", ws)
        wid = ws["working_set"]["id"]
        s, locked = c("POST", f"/api/workingsets/{q(wid)}/lock", {"joke_id": "joke:b", "line": "Bravo closer words."})
        expect(s, 200, "lock a line", locked)
        s, note = c("POST", f"/api/workingsets/{q(wid)}/try", {"joke_id": "joke:a", "text": "slow the opener down"})
        expect(s, 201, "try note", note)
        s, ws = c("GET", f"/api/workingsets/{q(wid)}")
        expect(s, 200, "workingset view", ws)
        gold("creative_ws_view", ws)
        s, kept = c("POST", "/api/ideas", {"text": "The meter wants a tip now.", "reason": "riff on: parking meters", "draft": True})
        expect(s, 200, "idea kept as his (learned)", kept)

        # ---------------------------------------------------------- drafts queued for the worker (202), then claimed
        queued = {}
        for name, path, body in (("riff", "/api/riff", {"idea": "parking meters", "lines": ["The meter asked for my pronouns before my card."]}),
                                 ("facts", "/api/facts", {"text": "parking meters"}),
                                 ("ask", "/api/ask", {"question": "where did I do the alpha bit?"}),
                                 ("bridge", f"/api/boards/{q(bid)}/bridge", {"from": "joke:a", "to": "joke:d"}),
                                 ("punchup", f"/api/workingsets/{q(wid)}/bits/joke:a/punchup", {})):
            s, r = c("POST", path, body)
            expect(s, 202, f"queue {name}", r)
            queued[name] = (path, body, r)
        gold("creative_queued", queued["ask"][2])
        gold("creative_queued_bridge", queued["bridge"][2])
        # the staged upload: a process_set job as POST /api/upload queues one hosted (creative_api.job_apply input shape)
        with api.cr.lock:
            st = api.cr.stores(api.kb)
            st["jobs"].append("enqueue", "job:process_set-fixture0", {"kind": "process_set", "key": f"upload|{UPLOAD_NAME}",
                                                                      "input": {"file": UPLOAD_NAME, "meta": UPLOAD_META}})
        s, waiting = c("GET", "/api/jobs/job:process_set-fixture0")
        expect(s, 200, "job waiting", waiting)
        gold("creative_job_waiting", waiting)

        claim = {"kinds": HOSTED_KINDS, "worker": "api-worker-7f9c", "hosted": True, "model": "Anthropic API · claude-synthetic"}
        gold("worker_claim", claim)
        gold("worker_claim_paused", {**claim, "paused": "the day's AI budget ($4.00) is spent; back at midnight UTC"})
        s, r = c("POST", "/api/worker/claim", {**claim, "paused": "the day's AI budget ($4.00) is spent; back at midnight UTC"})
        expect(s, 204, "paused claim", r)
        s, r = c("POST", "/api/worker/claim", {**claim, "kinds": HOSTED_KINDS + ["process_set"]})
        expect(s, 400, "hosted claim with a transcriber kind", r)
        gold("worker_error", r)
        done_jobs: dict[str, str] = {}
        retried = {"facts": False}

        def drain(body: dict, want: set[str]) -> None:
            """Claim until 204, posting the kind's synthetic result (the facts job fails once first: a retry)."""
            for _ in range(12):
                s, r = c("POST", "/api/worker/claim", body)
                if s == 204:
                    break
                expect(s, 200, "claim", r)
                job = r["job"]
                if job["kind"] not in done_jobs:
                    gold(f"worker_claimed_{job['kind']}", r)
                if job["kind"] == "facts" and not retried["facts"]:  # one retry: the provider timed out
                    retried["facts"] = True
                    res = {"ok": False, "error": "TimeoutError: the provider did not answer in 120 s", "retry": True}
                    s, r = c("POST", f"/api/worker/jobs/{q(job['id'])}/result", res)
                    expect(s, 200, "result (retry)", r)
                    gold("worker_result_retry", res)
                    gold("worker_result_retry_response", r)
                    continue
                if job["kind"] == "process_set":
                    s, raw = c("GET", f"/api/worker/jobs/{q(job['id'])}/file", raw=True)
                    expect(s, 200, "job file")
                    if raw != b"not really audio":
                        raise SystemExit("the staged upload came back changed")
                res = {"ok": True, "result": result_for(job)}
                s, r = c("POST", f"/api/worker/jobs/{q(job['id'])}/result", res)
                expect(s, 200, "result", r)
                done_jobs[job["kind"]] = job["id"]
                gold(f"worker_result_{job['kind']}", res)
                if job["kind"] == "riff":
                    gold("worker_result_response", r)
            else:
                raise SystemExit("the claim loop did not drain")
            if not want <= set(done_jobs):
                raise SystemExit(f"claimed {sorted(done_jobs)}, wanted {sorted(want)}")

        drain(claim, {"riff", "facts", "ask", "bridge", "punchup"})
        # the PC worker's claim (process_set is a transcriber kind: never hosted)
        pc_claim = {"kinds": ["process_set", "import_youtube"], "worker": "home", "hosted": False}
        drain(pc_claim, {"process_set"})
        s, r = c("GET", f"/api/worker/jobs/{q(done_jobs['riff'])}/file")
        expect(s, 404, "job file of a riff job", r)
        gold("worker_error_job_file", r)
        s, r = c("POST", f"/api/worker/jobs/{q(done_jobs['process_set'])}/result", {"ok": True, "result": {}})
        expect(s, 409, "result twice", r)
        gold("worker_error_result_conflict", r)

        # ---------------------------------------------------------- the app reads the finished drafts
        for name in ("riff", "facts", "ask", "bridge", "punchup"):
            s, r = c("GET", f"/api/jobs/{q(done_jobs[name])}")
            expect(s, 200, f"job {name}", r)
            gold(f"creative_job_{name}", r)
            path, body, _ = queued[name]
            s, r = c("POST", path, body)
            expect(s, 200, f"cached {name}", r)
            gold(f"{'simple' if name in ('riff', 'facts') else 'creative'}_cached_{name}", r)

        # ---------------------------------------------------------- the worker's other routes
        s, r = c("GET", "/api/worker/state")
        expect(s, 200, "state", r)
        gold("worker_state", r)
        s, r = c("GET", "/api/worker/files")
        expect(s, 200, "files", r)
        gold("worker_files", r)
        s, r = c("GET", "/api/worker/files?prefix=corpus/sets/20240101_x/")
        expect(s, 200, "files prefix", r)
        first = r["files"][0]["path"]
        s, r = c("GET", "/api/worker/files/content?path=" + quote(first))
        expect(s, 200, "file content", r)
        gold("worker_file_content", r)
        memo_tx = json.dumps({"segments": [{"start": 0.0, "end": 4.0, "text": "A note about the meter and the quarters."}]})
        memo_meta = json.dumps({"kind": "voice_note", "title": "meter note", "recorded_at": "2024-01-05T09:00:00", "duration_s": 4.0})
        files_post = {"files": [{"path": "corpus/memos/2024-01-05_a1b2c3d4/transcript.json", "text": memo_tx, "sha256": sha(memo_tx)},
                                {"path": "corpus/memos/2024-01-05_a1b2c3d4/meta.json", "text": memo_meta, "sha256": sha(memo_meta)},
                                {"path": first, "text": r["text"], "sha256": r["sha256"]}],
                      "remove": ["kb/gsync/runs.jsonl"], "actor": "ai:memolink"}
        gold("worker_files_post", files_post)
        s, r = c("POST", "/api/worker/files", files_post)
        expect(s, 200, "files post", r)
        gold("worker_files_post_response", r)
        s, r = c("POST", "/api/worker/files", {"files": [{"path": "tapes/raw/x/audio.opus", "text": "no"}], "actor": "ai:memolink"})
        expect(s, 403, "files post refused", r)
        gold("worker_error_files_refused", r)

        idea = {"text": "A note about the meter and the quarters.", "title": "meter note", "actor": "import:upload",
                "from": {"note": "Voice note 'meter note', recorded 2024-01-05"},
                "reason": "[upload] voice note 'meter note' (upload a1b2c3d4, recorded 2024-01-05)"}
        gold("worker_ideas", idea)
        s, r = c("POST", "/api/worker/ideas", idea)
        expect(s, 200, "worker idea", r)
        gold("worker_ideas_response", r)
        new_joke = r["id"]
        idea2 = {"text": "Alpha opener line one. The alpha punch lands here every time.", "title": "", "actor": "ai:set_text",
                 "from": {"set": set_dir(SY), "unit": "u3"}, "reason": "[set_text] bit from 'Seaside open mic' (2024-02-01), transcribed on the phone; no audio"}
        s, r = c("POST", "/api/worker/ideas", idea2)
        expect(s, 200, "worker idea (a bit it already has)", r)
        gold("worker_ideas_response_attached", r)
        dec = {"target": "node", "action": "edit", "id": new_joke, "after": {"summary": "A note about the meter and the quarters, edited in the doc."},
               "reason": "[gdocs] edited", "actor": "import:gsync"}
        gold("worker_decision", dec)
        s, r = c("POST", "/api/worker/decision", dec)
        expect(s, 200, "worker decision", r)
        gold("worker_decision_response", r)
        undo = {"indexes": [r["indexes"][0]], "actor": "import:gsync"}
        gold("worker_undo", undo)
        s, r = c("POST", "/api/worker/undo", undo)
        expect(s, 200, "worker undo", r)
        gold("worker_undo_response", r)
        s, r = c("POST", "/api/worker/decision", {"target": "node", "action": "edit", "id": new_joke, "after": {"summary": "x"}})
        expect(s, 400, "worker write without an actor", r)
        gold("worker_error_actor", r)
        s, r = c("GET", "/api/worker/jokes/" + q("joke:a"))
        expect(s, 200, "worker joke", r)
        gold("worker_joke", r)
        s, r = c("GET", "/api/worker/jokes/" + q("joke:nope"))
        expect(s, 404, "worker joke 404", r)
        for name, after in (("worker_decisions", 0), ("worker_decisions_after", 3)):
            s, r = c("GET", f"/api/worker/decisions?after={after}")
            expect(s, 200, name, r)
            for line in r["lines"]:  # the guard reads a bare 64-hex as a secret: label the content hash as what it is
                line["h"] = "sha256:" + line["h"]
            gold(name, r)
        s, r = c("GET", "/api/worker/creative?store=storyboards")
        expect(s, 200, "worker creative", r)
        gold("worker_creative", r)
        s, r = c("GET", "/api/worker/creative?store=storyboards&snapshot=1")
        expect(s, 200, "worker creative snapshot", r)
        gold("worker_creative_snapshot", r)
        s, r = c("GET", "/api/worker/creative?store=merges&snapshot=1")
        expect(s, 200, "worker creative merges", r)
        gold("worker_creative_merges", r)
        s, r = c("GET", "/api/worker/creative?store=profile")
        expect(s, 400, "worker creative bad store", r)
        ev = {"store": "merges", "op": "attach", "id": "at:f00dbabe12", "actor": "ai:attach",
              "data": {"joke": "joke:f", "into": "joke:a", "text": TEXT["f"], "date": NOW[:10], "score": 0.91, "source": "memo",
                       "reasons": ["Claude: the same bit (0.91): the same opener", "shares 4 rare words"]}}
        gold("worker_events", ev)
        s, r = c("POST", "/api/worker/events", ev)
        expect(s, 200, "worker event", r)
        gold("worker_events_response", r)
        s, r = c("POST", "/api/worker/events", ev)
        expect(s, 200, "worker event again", r)
        gold("worker_events_response_already", r)
        s, r = c("GET", "/api/worker/profile/voice")
        expect(s, 200, "voice (empty)", r)
        gold("worker_voice_empty", r)
        put = {"text": "# Voice\n\nShort sentences. Sits on the punch word. Never explains the joke.\n", "actor": "import:profile"}
        gold("worker_voice_put", put)
        s, r = c("PUT", "/api/worker/profile/voice", put)
        expect(s, 200, "voice put", r)
        gold("worker_voice_put_response", r)
        s, r = c("PUT", "/api/worker/profile/voice", {**put, "text": "# Voice v2\n\nEven shorter.\n"})
        expect(s, 200, "voice put again", r)
        gold("worker_voice_put_response_previous", r)
        s, r = c("GET", "/api/worker/profile/voice")
        expect(s, 200, "voice", r)
        gold("worker_voice", r)
        s, r = c("GET", "/api/profile/voice")
        expect(s, 200, "voice (app)", r)
        gold("api_voice", r)
        # the kinds the other routes queue: a phone transcript (set_text), a conversation (ask_chat), a YouTube link
        # (import_youtube), and the follow-ups the imported idea queued above (embed, memolink)
        s, r = c("POST", "/api/sets/transcript", {"segments": [{"text": "The meter wants a tip now.", "start": 0.0, "end": 3.0},
                                                               {"text": "It only takes quarters and apologies.", "start": 3.0, "end": 6.5}],
                                                  "title": "Phone night", "recorded_at": "2024-05-01T21:00:00", "venue": "The Corner Room",
                                                  "duration_s": 7.0, "engine": "speech"})
        expect(s, 202, "set transcript", r)
        s, r = c("POST", "/api/ask/chats", {"question": "what did I write about the purple monkey dishwasher?"})
        expect(s, 201, "ask chat", r)
        if not r["turns"] or r["turns"][-1].get("status") != "thinking":  # thinking = an ask_chat job is queued
            raise SystemExit(f"the conversation queued no ask_chat job: {json.dumps(r)[:400]}")
        drain(claim, {"set_text", "ask_chat", "embed", "memolink"})

        # ---------------------------------------------------------- the app-facing reads
        s, r = c("GET", "/api/jokes?limit=50&sort=recent&include_hidden=1")
        expect(s, 200, "jokes", r)
        gold("api_jokes", r)
        s, r = c("GET", "/api/jokes/" + q("joke:a"))
        expect(s, 200, "joke detail", r)
        gold("api_joke_detail", r)
        gold("api_edge_views", r["labels"] + r["links"])
        s, r = c("GET", "/api/sets")
        expect(s, 200, "sets", r)
        gold("api_sets", r)
        s, r = c("GET", f"/api/sets/{q('set:' + SX)}/review")
        expect(s, 200, "review", r)
        gold("api_review", r)
        s, r = c("GET", "/api/jokes/" + q("joke:b") + "/connections?cap=5")
        expect(s, 200, "connections", r)
        gold("creative_connections", r)
        s, r = c("GET", "/api/boards")
        expect(s, 200, "boards", r)
        gold("creative_boards", r)
        s, r = c("GET", f"/api/boards/{q(bid)}")
        expect(s, 200, "board", r)
        gold("creative_board_view_read", r)
        s, r = c("PATCH", f"/api/boards/{q(bid)}", {"title": "Tuesday run v2", "move": {"from": 0, "to": 1}})
        expect(s, 200, "board patch", r)
        s, r = c("GET", "/api/workingsets")
        expect(s, 200, "workingsets", r)
        gold("creative_workingsets", r)
        s, r = c("GET", f"/api/workingsets/{q(wid)}/nights")
        expect(s, 200, "nights", r)
        gold("creative_nights", r)
        s, r = c("GET", f"/api/workingsets/{q(wid)}/bits/joke:a")
        expect(s, 200, "bit history", r)
        gold("creative_bit_history", r)
        s, r = c("GET", f"/api/workingsets/{q(wid)}/bits/joke:f")
        expect(s, 200, "bit history (never taped)", r)
        gold("creative_bit_history_untaped", r)
        s, r = c("GET", "/api/venues")
        expect(s, 200, "venues", r)
        r["geocoder"]["cache"] = "geocode_cache.json"  # the one normalised value: the cache's absolute path on this machine
        gold("creative_venues", r)
        s, r = c("GET", "/api/venues.geojson")
        expect(s, 200, "geojson", r)
        gold("creative_geojson", r)
        s, r = c("GET", "/api/today")
        expect(s, 200, "today", r)
        gold("creative_today", r)
        s, r = c("GET", f"/api/sets/{q('set:' + SX)}/clips")
        expect(s, 200, "clips", r)
        gold("creative_clips", r)
        s, r = c("GET", f"/api/sets/{q('set:' + SY)}/clips")
        expect(s, 200, "clips (none)", r)
        gold("creative_clips_none", r)
        s, r = c("GET", "/api/connections")
        expect(s, 200, "connections status", r)
        gold("creative_connections_status", r)
        s, r = c("GET", "/api/notebook/related?q=" + quote("my niece and the laundromat") + "&cap=10")
        expect(s, 200, "related", r)
        gold("simple_related", r)
        s, r = c("GET", "/api/jokes/" + q("joke:a") + "/map?cap=8")
        expect(s, 200, "joke map", r)
        gold("simple_joke_map", r)
        s, r = c("GET", f"/api/boards/{q(bid)}/build")
        expect(s, 200, "build", r)
        gold("simple_build", r)
        s, r = c("POST", "/api/ideas", {"text": "An idea about airport security lines.", "reason": "riff on: airports"})
        expect(s, 200, "save idea", r)
        gold("simple_save_idea", r)
        s, r = c("POST", "/api/profile/voice/learn", {"dropped": [{"text": "It only takes quarters and apologies.", "bit": "joke:a"}]})
        expect(s, 200, "dismiss", r)
        gold("simple_dismiss_response", r)
        # client-side shapes the app never receives from a route: written here as the app builds them
        gold("api_api_event", {"method": "POST", "path": "/api/ideas", "status": 0, "reason": "TypeError"})
        gold("api_upload_meta", {"kind": "set", "filename": "seaside.m4a", "title": "Seaside open mic", "durationS": 240.4, "marksMs": [61000, 150000],
                                 "recordedAt": "2024-02-01T21:00:00", "venue": "Seaside Comedy Club", "lat": 51.5, "lng": -0.1,
                                 "showGroup": "open mic", "alignWorkingSet": wid})
        s, r = c("GET", "/api/jokes/" + q("joke:nope"))
        expect(s, 404, "app 404", r)
        gold("api_error", r)
    finally:
        api.stop()
        shutil.rmtree(root, ignore_errors=True)
        shutil.rmtree(HOME, ignore_errors=True)
    n = len(list(OUT.glob("*.json")))
    print(f"wrote {n} goldens to {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
