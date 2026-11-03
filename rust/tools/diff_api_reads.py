"""T9's gate: the same GET requests against the hosted Python API (api/entry.py) and pk-api, over ONE scratch
SurrealDB loaded the way production is (db/push_remote.py pack + apply), with Keycloak replaced by a local JWKS
(api-dev/tests/jwt_kit.py). Every answer must be equal: the status, the JSON body as a parsed value (ADR 0007) and
the headers the app reads (content type, no-store, the 401 challenge, CORS). Known differences are in ALLOW below,
each with its reason; there are none on the happy path.

    python rust/tools/diff_api_reads.py --pk-api rust/target/release/pk-api.exe [--bench 300] [--report out.json]

What it does, in order: build a SYNTHETIC data folder (invented bits, nights and labels; nothing of the comic's),
start `surreal start memory` with a throwaway password, pack and apply the bundle, start both APIs on loopback with
auth ON, write through pk-api (every write is forwarded to the Python: the T9 proxy) and check that the very next
GET from pk-api sees it (the read-after-write gate, docs/ARCHITECTURE.md section 2), run the request matrix, and
optionally time each route group on both (p50 / p95, sequential, warm) and read both processes' memory.
Exit 1 on any difference outside ALLOW. Needs: surreal 3.3.4 (PUNCH_SURREAL or ~/.punchable/bin/surreal[.exe]), the
repo's Python deps (the API's), the bge-small model folder in PK_BGE_DIR (both servers embed search queries and
passages with it; tools/kb/fetch_bge.py), and a built pk-api.
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import secrets
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "api-dev" / "tests"))
import jwt_kit as JK  # noqa: E402  (test-only RSA key, JWKS and RS256 tokens)

NS, DB = "punchable_test", "t9"
ORIGIN = "https://punchable.io"
FLAGS = '{"glass_accessory": false}'

# ---------------------------------------------------------------- known differences (path in the response -> why)
# Keys are (request label, JSON path or "status" / "header:<name>"). Empty on purpose: add one only with a reason the
# owner would accept (the board's rule: the allow-list is for things that cannot hold, not a licence).
ALLOW: dict[tuple[str, str], str] = {}
# /api/today's `inbox` is the length of the Python pod's own upload folder (kb.inbox, written by POST /api/upload), which
# pk-api cannot see until T10 moves uploads; pk-api says 0. The app does not read the field (grep app/src: today.tsx
# reads next_show and pipeline, bit/[id].tsx reads tonight.items). Owner's decision, 2026-11-02.
TODAY = ["/api/today", "/api/today?include_hidden=1", "/api/today?backend=files", "after his tonight list: /api/today"]
ALLOW |= {(p, "$.inbox"): "the Python pod's upload folder, not in the database until T10; the app does not read it"
          for p in TODAY}
# Round 6: Python's int() has no bound and the Bits list echoes `offset` back; pk-api reads integers as i64 and saturates
# one past it (every other use clamps it, so only this echo can differ). The app sends offsets in the hundreds.
BIG_OFFSET = "/api/jokes?offset=99999999999999999999"
ALLOW |= {(lbl, "$.offset"): "an offset past 2^63 echoes 9223372036854775807 (i64); the Python echoes the number"
          for lbl in (BIG_OFFSET, "no model: " + BIG_OFFSET)}
# Numbers that cannot be bit-equal, compared within a tolerance instead: the cloud's coordinates come from numpy's
# float32 LAPACK SVD on the Python side and a float64 eigen-decomposition in Rust (crates/pk-api/src/cloud.rs); they
# agree to ~1e-6 before the 4-decimal rounding. Everything else in the cloud (ids, clusters, labels, counts) is exact.
TOLERANT_KEYS = {"x", "y", "z", "cx", "cy", "cz"}
TOLERANCE = 1e-3
# With 2 or 3 bits there is no spread along the last axis: numpy's float32 SVD returns a noise direction (~1e-8) that
# the 1e-9 cut lets through, so the Python stretches noise to +/-1; the float64 eigen-decomposition gives ~1e-17 and
# Rust leaves the axis at 0. The coordinates of these two requests are not compared (everything else is).
DEGENERATE = {"/api/map/cloud?cap=2", "/api/map/cloud?cap=3"}
# Round 6. /similar's `distance` (a bit's) and `score` (a moment's: its distance) come from a passage vector each server
# makes itself: onnxruntime in the Python, `ort` in pk-api, equal to cosine >= 0.99999 (pk-embed's goldens), so the
# distances SurrealDB returns agree to ~1e-7, not bit for bit. Compared within VECTOR_TOLERANCE; the order and every
# other field are exact. Search's own `score` is a reciprocal-rank sum and stays exact.
VECTOR_TOLERANCE = 1e-5
# Round 6. SurrealDB's order among equal scores (equal BM25 scores of near-identical lines, then equal RRF sums) is not
# stable: the Python asked the same search twice answers in two orders (CONFIRMED 2026-11-02, /api/search?q=pilot).
# When pk-api's answer differs, the Python is asked 11 more times; if it disagrees with itself, the ranked lists are
# compared up to that tie order (compare_ranked), everything else exactly.
RANKED_LISTS = ("bits", "moments", "items")

# ---------------------------------------------------------------- the synthetic data folder
TEXT = {
    "a": "Alpha opener line one. The alpha punch lands here every time.",
    "b": "Bravo setup about the dog. The purple monkey dishwasher line here. Bravo closer words.",
    "c": "Charlie premise about cars and traffic and more traffic today.",
    "d": "Delta bit about my niece and the laundromat place.",
    "e": "Echo bit about my niece and her soccer schedule.",
    "f": "Foxtrot never performed idea about planes and pilots up there.",
    "g": "Golf hidden note that is not a joke at all, just a grocery list.",
}
TITLE = {"a": "Alpha bit", "b": "Bravo bit (the dog one)", "c": "Charlie bit", "d": "Delta bit", "e": "Echo bit",
         "f": "Foxtrot bit about the very long title that goes on", "g": "Golf list"}
STAGE = {"a": "testing", "b": "working", "c": "testing", "d": "to_try", "e": "idea", "f": "idea", "g": "idea"}
UNIT_DATE = {"a": "2024-01-10", "b": "2023-05-01", "c": "2024-04-04", "d": "2015-03-03", "e": "2024-08-08",
             "f": "2016-07-07", "g": "2024-09-09"}
SX, SY, SZ = "2024-01-01_x", "2024-02-01_y", "2024-03-01_z"
N_EXTRA = 46
EXTRA = [("dog", ["leash", "park", "bark"]), ("car", ["traffic", "parking", "engine"]),
         ("plane", ["pilot", "luggage", "airport"]), ("family", ["niece", "uncle", "dinner"])]
TOPIC_OF = {"a": 0, "b": 0, "c": 1, "d": 3, "e": 3, "f": 2, "g": 3}


def set_dir(key: str) -> str:
    return key[:4] + key[5:7] + key[8:10] + key[10:]


def make_graph() -> dict:
    nodes, edges = [], []
    for k in "abcdefg":
        j = {"id": f"joke:{k}", "type": "joke", "title": TITLE[k], "stage": STAGE[k], "summary": f"A bit about {k}.",
             "first_seen": "2023-01-01", "last_seen": UNIT_DATE[k], "lead_unit": f"unit:{k}1"}
        u = {"id": f"unit:{k}1", "type": "unit", "kind": "setup+punch", "text": TEXT[k], "date": UNIT_DATE[k],
             "source": "gdoc", "doc_title": "Notes"}
        if k == "g":
            j["hidden"] = u["hidden"] = True
        if k == "f":
            j["from"] = {"set": f"set:{SY}", "start_ms": 150000}
        nodes += [j, u]
        edges.append({"id": f"e:p{k}", "src": f"unit:{k}1", "dst": f"joke:{k}", "kind": "part_of", "status": "inferred",
                      "confidence": 0.9, "evidence": []})
    # 46 more bits on four invented topics, so the cloud has real clusters to find, and (round 8) more than 50 bits
    # with vectors, so the story layer's similarity band (`story._cos_band`, (1, 1) under 50) is really computed
    for i in range(N_EXTRA):
        topic, words = EXTRA[i % 4]
        k = f"x{i:02d}"
        nodes.append({"id": f"joke:{k}", "type": "joke", "title": f"{topic.title()} {words[i % 3]} number {i}", "stage": "idea",
                      "summary": f"An idea about {topic}.", "first_seen": "2023-01-01", "last_seen": f"2024-{1 + i % 12:02d}-{1 + i % 27:02d}",
                      "lead_unit": f"unit:{k}1"})
        nodes.append({"id": f"unit:{k}1", "type": "unit", "kind": "setup+punch", "date": f"2024-{1 + i % 12:02d}-{1 + i % 27:02d}",
                      "text": f"The {topic} and the {words[i % 3]}. Then the {words[(i + 1) % 3]} again, number {i}.", "source": "gdoc"})
        edges.append({"id": f"e:p{k}", "src": f"unit:{k}1", "dst": f"joke:{k}", "kind": "part_of", "status": "inferred",
                      "confidence": 0.9, "evidence": []})
    # round 6: an older version of another bit that repeats Charlie's whole line, so /similar with no model (a words
    # search over the bit's first twelve words, every word required) finds another bit
    nodes.append({"id": "unit:x03b", "type": "unit", "kind": "setup+punch", "date": "2022-01-01", "source": "gdoc",
                  "text": "Old draft: Charlie premise about cars and traffic and more traffic today, said twice."})
    edges.append({"id": "e:px03b", "src": "unit:x03b", "dst": "joke:x03", "kind": "part_of", "status": "inferred",
                  "confidence": 0.9, "evidence": []})
    nodes.append({"id": "unit:a0", "type": "unit", "kind": "setup+punch", "text": "Alpha opener, first draft.", "date": "2023-02-02"})
    edges.append({"id": "e:pa0", "src": "unit:a0", "dst": "joke:a", "kind": "part_of", "status": "proposed", "confidence": 0.6,
                  "evidence": []})
    for lid, kind, name in [("label:persona:dad", "persona", "Dad"), ("label:topic:kids", "topic", "Kids"),
                            ("label:topic:cars", "topic", "Cars"), ("label:device:d1", "device", "D-one"),
                            ("label:device:d2", "device", "D-two"), ("label:structure:s1", "structure", "S-one")]:
        nodes.append({"id": lid, "type": "label", "kind": kind, "name": name, "definition": f"{name}, as a label"})

    def lab(j, lbl, w, q):
        edges.append({"id": f"e:l{j}-{lbl.split(':')[-1]}", "src": f"joke:{j}", "dst": lbl, "kind": "has_label",
                      "status": "proposed", "confidence": w, "reason": f"{q} reads as {lbl.split(':')[-1]}",
                      "evidence": [{"node": f"unit:{j}1", "quote": q}]})
    lab("a", "label:device:d1", 0.9, "Alpha opener")
    lab("b", "label:device:d1", 0.9, "purple monkey")
    lab("c", "label:device:d2", 0.9, "Charlie premise")
    lab("d", "label:persona:dad", 0.8, "my niece")
    lab("e", "label:persona:dad", 0.8, "my niece")
    lab("d", "label:topic:kids", 0.8, "laundromat")
    lab("e", "label:topic:kids", 0.8, "soccer schedule")
    lab("c", "label:topic:cars", 0.8, "cars and traffic")
    lab("g", "label:topic:kids", 0.5, "grocery list")
    edges.append({"id": "e:seg_bc", "src": "joke:b", "dst": "joke:c", "kind": "segue", "status": "proposed", "confidence": 0.7,
                  "reason": "dog leads to cars", "evidence": [{"node": "unit:b1", "quote": "Bravo closer words."}]})
    # round 8: what one bit's connections and its map read: a line of another bit tagging it, a callback, a
    # shared premise, the same shape (two shared moves), attitudes (another angle), the reveal of a bit and its note
    for lid, kind, name in [("label:structure:s2", "structure", "S-two"), ("label:attitude:mad", "attitude", "Mad"),
                            ("label:attitude:calm", "attitude", "Calm"), ("label:topic:dogs", "topic", "Dogs")]:
        nodes.append({"id": lid, "type": "label", "kind": kind, "name": name, "definition": f"{name}, as a label"})
    lab("a", "label:structure:s1", 0.8, "Alpha opener")
    lab("b", "label:structure:s1", 0.8, "Bravo setup")
    lab("x01", "label:device:d1", 0.7, "The car and the")
    lab("x01", "label:structure:s2", 0.7, "The car and the")
    lab("b", "label:topic:dogs", 0.8, "about the dog")
    lab("x00", "label:topic:dogs", 0.8, "The dog and the")
    lab("x04", "label:topic:dogs", 0.8, "The dog and the")
    lab("b", "label:attitude:calm", 0.7, "Bravo setup")
    lab("x00", "label:attitude:mad", 0.7, "The dog and the")
    lab("c", "label:topic:kids", 0.6, "Charlie premise")
    lab("c", "label:persona:dad", 0.6, "Charlie premise")
    edges.append({"id": "e:tag_x01a", "src": "unit:x011", "dst": "joke:a", "kind": "tag_of", "status": "confirmed",
                  "confidence": 0.8, "reason": "the car line tags the alpha punch",
                  "evidence": [{"node": "unit:x011", "quote": "The car and the parking."}]})
    edges.append({"id": "e:cb_ed", "src": "joke:e", "dst": "joke:d", "kind": "callback_to", "status": "inferred",
                  "confidence": 0.6, "reason": "the niece again", "evidence": [{"node": "unit:e1", "quote": "my niece"}]})
    edges.append({"id": "e:prem_dx03", "src": "joke:d", "dst": "joke:x03", "kind": "same_premise", "status": "proposed",
                  "confidence": 0.5, "reason": "errands", "evidence": []})
    edges.append({"id": "e:shape_ab", "src": "joke:a", "dst": "joke:b", "kind": "same_shape", "status": "proposed",
                  "confidence": 0.55, "reason": "both list then flip", "evidence": []})
    for n in nodes:
        if n["id"] == "joke:a":
            n["anatomy"] = {"reveal": "The alpha punch lands here every time."}
        if n["id"] == "joke:b":
            n["quality"] = {"notes": "the dog part runs long"}
        if n["id"] == "unit:x001":
            n["anatomy"] = {"reveal": "The dog and the leash."}
    for key, title, venue, dur in ((SX, "Corner Room, Tuesday", "The Corner Room", 300),
                                   (SY, "Seaside open mic", "Seaside Comedy Club", 240),
                                   (SZ, "Mabel's, March", "Mabel's (upstairs)", 120)):
        n = {"id": f"set:{key}", "type": "set", "title": title, "venue": venue, "date": key[:10], "duration_s": dur}
        if key == SZ:
            n["audio_unreliable"] = True
        nodes.append(n)
    for start, end, text, kind, name, joke in ((0, 60000, TEXT["a"], "bit", "Alpha", "a"),
                                                (60000, 120000, TEXT["b"], "bit", "Bravo", "b"),
                                                (120000, 140000, "So who here has a car?", "crowd_work", "Crowd", None),
                                                (140000, 200000, TEXT["c"], "bit", "Charlie", "c")):
        pid = f"perf:{SX}@{start}"
        nodes.append({"id": pid, "type": "performance", "set": f"set:{SX}", "start_ms": start, "end_ms": end, "text": text,
                      "kind": kind, "name": name})
        if joke:
            edges.append({"id": f"e:pa-{joke}", "src": pid, "dst": f"joke:{joke}", "kind": "performed_as",
                          "status": "proposed", "confidence": 0.8, "reason": f"the {name.lower()} words",
                          "evidence": [{"node": pid, "quote": text.split(".")[0] + ".", "start_ms": start + 1000}]})
    return {"version": "t9-test", "generated_at": "2026-10-01", "runs": ["r1"], "nodes": nodes, "edges": edges}


def row(idx, joke, start_s, dur_s, text, laugh_s, set_rate, status="known", **extra):
    window = float(dur_s)
    lspm = round(laugh_s / (window / 60), 2) if window else 0.0
    return {"idx": idx, "start_ms": int(start_s * 1000), "end_ms": int((start_s + dur_s) * 1000), "title": f"T{idx}",
            "status": status, "joke_id": joke, "version_unit_id": f"unit:{joke[5:]}1" if joke else None,
            "match_confidence": 0.9 if joke else None, "match_reason": "same words" if joke else "", "transcript": text,
            "punchlines": [], "laughs": {"count": 1 if laugh_s else 0, "max_bars": 2 if laugh_s else 0, "total_s": laugh_s,
                                         "merged_count": 1 if laugh_s else 0, "laugh_s": laugh_s, "window_s": window},
            "laugh_s_per_min": lspm, "pct_laughing": round(laugh_s / window, 3), "time_to_first_laugh_s": 5.0 if laugh_s else None,
            "dead_air_max_s": 5.0, "vs_room": round(lspm / set_rate, 2), "ai_rating": "worked" if laugh_s else "died",
            "rating_basis": "the laugh" if laugh_s else "silence", "similar": [], "hint_labels": [],
            "script_diff": {"on_script_pct": 50, "dropped": [], "added": []}, **extra}


ROW1_EXTRA = {
    "punchlines": [{"text": "The alpha punch lands here every time.", "at_ms": 5000, "bars": 2, "laugh_id": "L1"},
                   {"text": "Yes.", "at_ms": 54000, "bars": 3, "laugh_id": "L2"}, {"text": ""}],
    "similar": [{"joke_id": "joke:c", "why": "both open the night", "confidence": 0.4}, {"joke_id": "joke:a", "why": "itself"},
                {"joke_id": "joke:nope"}, {"joke_id": "label:device:d1", "why": "not a joke"}],
    "hint_labels": [{"label_id": "label:device:d1", "why": "a callback"}, {"label_id": "label:topic:kids", "why": "family"},
                    {"label_id": "label:missing"}],
}


def analysis(key, rate, rows):
    return {"set": key, "run_id": "synthetic-1", "generated_at": "2026-10-01T00:00:00+00:00",
            "summary": {"laugh_s_per_min": rate, "room_heat": 1.0, "room_heat_basis": "set", "material_s": 300.0,
                        "laugh_s_total": 24.0, "pct_laughing": 0.08, "laughs_merged": 4, "laughs_per_min_merged": 0.8,
                        "applause_s": 0.0, "longest_laugh_s": 3.0, "longest_laugh_joke": rows[0]["joke_id"],
                        "dead_air_max_s": 9.0, "dead_air_max_at_ms": 100000, "duration_s": 300, "best": None,
                        "weakest": None, "jokes_counted": sum(1 for r in rows if r["status"] in ("known", "new")),
                        "new_material": sum(1 for r in rows if r["status"] == "new")},
            "jokes": rows}


SETS = {
    SX: {"analysis": analysis(SX, 6.0, [row(1, "joke:a", 0, 60, TEXT["a"], 9.0, 6.0, **ROW1_EXTRA),
                                        row(2, "joke:b", 60, 60, TEXT["b"], 6.0, 6.0,
                                            punchlines=[{"text": "Bravo closer words.", "at_ms": 89500, "bars": 1, "laugh_id": "LX"}]),
                                        row(3, None, 120, 20, "So who here has a car?", 0.0, 6.0, status="crowd_work"),
                                        row(4, "joke:c", 140, 60, TEXT["c"], 6.1, 6.0)]),
         "meta": {"title": "Corner Room, Tuesday", "venue_guess": "The Corner Room", "show_date": "2024-01-05",
                  "show_date_source": "the folder date", "duration": 300.0}},
    SY: {"analysis": analysis(SY, 7.3, [row(1, "joke:a", 0, 60, TEXT["a"], 8.4, 7.3),
                                        row(2, "joke:b", 60, 90, "Bravo setup about the dog. Bravo closer words.", 2.0, 7.3),
                                        row(3, None, 150, 60, TEXT["d"], 3.0, 7.3, status="new")]),
         "meta": {"source": "app_upload", "title": "Seaside open mic", "show_venue": "Seaside Comedy Club", "duration_s": 240.0}},
    SZ: {"analysis": analysis(SZ, 5.0, [row(1, "joke:b", 0, 40, "Bravo setup about the dog.", 1.5, 5.0),
                                        row(2, "joke:a", 40, 30, "Alpha opener line one.", 4.0, 5.0,
                                            similar=[{"joke_id": "joke:c", "title": "Charlie bit", "why": "both short"}],
                                            hint_labels=[{"label_id": "label:topic:kids", "why": "family again"}])]),
         "meta": {"title": "Mabel's, March", "venue_guess": "Mabel's (upstairs)", "show_date": "2024-03-01",
                  "drive_id": "drv-synthetic-1", "original_name": "march.m4a"}},
}
# What the tape pages read (T9 round 4): laughs (laughs_auto.json -> laugh_run + laugh rows) and the transcript
# (a srcfile row). Laughs inside and outside the performances, of every bar count, in and out of the set segment, so
# punch_of, review_steps' "new line?" steps, the ticks and the summaries all have something to choose between.
TAPES = {
    SX: {"laughs": [{"id": "L1", "start_ms": 20000, "end_ms": 23000, "bars": 2, "strength": 0.6, "segment": "set"},
                    {"id": "L2", "start_ms": 55000, "end_ms": 59000, "bars": 3, "strength": 0.9},
                    {"id": "L3", "start_ms": 90000, "end_ms": 91500, "bars": 1, "strength": 0.3, "segment": "set"},
                    {"id": "L3b", "start_ms": 95000, "end_ms": 96000, "bars": 1, "strength": 0.8, "segment": "set"},
                    {"id": "L4", "start_ms": 130000, "end_ms": 132000, "bars": 2, "strength": 0.5, "segment": "crowd"},
                    {"id": "L5", "start_ms": 230000, "end_ms": 233000, "bars": 2, "strength": 0.7},
                    {"id": "L6", "start_ms": 260000, "end_ms": 261000, "bars": 1},
                    {"id": "L7", "start_ms": 280500, "end_ms": 282500, "bars": 3, "strength": 0.2, "segment": "set"}],
         "segments": [(0, 5, "Alpha opener line one."), (5, 18, "The alpha punch lands here every time."),
                      (18, 40, "Short bit."), (40, 54, "Then a longer line with many words to say here."),
                      (54, 58, "Yes."), (60, 80, "Bravo setup about the dog."),
                      (80, 89.5, "  The purple monkey dishwasher line here. "), (89.5, 100, "Bravo closer words."),
                      (120, 129, "So who here has a car?"),
                      (140, 170, "Charlie premise about cars and traffic and more traffic today."),
                      (210, 222, "A new thing I tried tonight about the bus driver."), (222, 229.9996, "Big finish."),
                      (270, 279, "Ok."), (279, 280.0004, "Then the real closer that got them all laughing.")]},
    SY: {"laughs": [{"id": "Y1", "start_ms": 30000, "end_ms": 31000, "bars": 2},
                    {"id": "Y2", "start_ms": 170000, "end_ms": 172000, "bars": 3, "strength": 1.0}],
         "segments": [(0, 30, "Alpha opener line one."), (160, 171, "Delta bit about my niece and the laundromat place.")]},
}
INFERRED = {"run_id": "inf-1", "generated_at": "2026-10-01T00:00:00+00:00", "notes": "synthetic",
            "jokes": [{"id": "joke:a", "stage": "working", "stage_confidence": 0.8, "stage_reason": "three nights"},
                      {"id": "joke:f", "stage": "idea", "stage_confidence": 0.6, "stage_reason": "never taped"}]}
LEARNING = {"generated_at": "2026-10-01T00:00:00+00:00", "lessons": [{"what": "openers land", "n": 3}]}


def write_json(p: Path, obj) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(obj, ensure_ascii=False, indent=1), encoding="utf-8", newline="\n")


def build_comedy(comedy: Path) -> None:
    # kb/v0 (no kb/v1): db/load.py's v1 path runs kb_validate, whose id and evidence rules a small fixture need not meet
    v0 = comedy / "kb" / "v0"
    write_json(v0 / "graph.json", make_graph())
    (v0 / "decisions.jsonl").write_text("", encoding="utf-8")
    write_json(v0 / "inferred.json", INFERRED)
    write_json(v0 / "learning.json", LEARNING)
    (v0 / "KB_REPORT.md").write_text("# Report\n\n## first 50\n\n- e:pa0\n", encoding="utf-8", newline="\n")
    for key, s in SETS.items():
        d = comedy / "corpus" / "sets" / set_dir(key)
        write_json(d / "analysis.json", s["analysis"])
        write_json(d / "meta.json", s["meta"])
        tape = TAPES.get(key, {})
        segs = [{"start": a, "end": b, "text": t} for a, b, t in tape.get("segments", [(0, 5, "Hello.")])]
        write_json(d / "transcript.json", {"segments": segs})
        write_json(d / "laughs_auto.json", {"detector": "synthetic", "run_id": "laughs-1", "laughs": tape.get("laughs", [])})
        if key == SX:  # round 8: a set's clips file, served as it is
            write_json(d / "clips.json", {"set": f"set:{SX}", "source": "clipper-1",
                                          "clips": [{"start_ms": 0, "end_ms": 20000, "title": "Alpha", "score": 0.8}]})
    (comedy / "kb" / "creative").mkdir(parents=True)
    # the Today feed's inputs: gems and orphans (serendipity), and a working set with a locked bit and a locked line
    write_json(v0 / "insights.json", INSIGHTS)
    (comedy / "kb" / "creative" / "workingsets.jsonl").write_text(
        "".join(json.dumps(e) + "\n" for e in WORKINGSETS), encoding="utf-8", newline="\n")
    # round 7: set lists (a seeded draft, one of his with a bit that is not in the notebook, an empty one of his, a
    # deleted one) and the triage judge's "not a joke?" verdicts
    (comedy / "kb" / "creative" / "storyboards.jsonl").write_text(
        "".join(json.dumps(e) + "\n" for e in STORYBOARDS), encoding="utf-8", newline="\n")
    write_json(comedy / "kb" / "creative" / "suggest_judge.json", SUGGEST_JUDGE)


INSIGHTS = {"gems": {"gems": [
    {"joke": "joke:f", "like": "joke:a", "like_title": "Alpha bit", "shared_shape": ["device:d1", "structure:s1", "x", "y"],
     "shared_topics": ["planes"], "score": 0.71},
    {"joke": "joke:x03", "like": "joke:b", "like_title": "Bravo bit", "shared_shape": ["device:d2"], "shared_topics": [],
     "score": 0.52},
    {"joke": "joke:x04", "like": "joke:x04", "like_title": "itself", "shared_shape": [], "score": 0.4},
    {"joke": "joke:a", "like": "joke:b", "like_title": "Bravo bit", "shared_shape": ["d"], "score": 0.9}]},
    "orphans": {"active_orphans": ["joke:x07", "joke:x08", "joke:nope", "joke:f", "joke:x11"]},
    "chunks": {"chunks": [{"name": "Family errands", "jokes": ["joke:d", "joke:e", "joke:x03", "joke:nope"],
                           "fits_5_15": True, "minutes_est": 6}]}}
WS_BITS = [{"joke_id": "joke:a", "title": "Alpha bit", "target_s": 40}, {"joke_id": "joke:b", "title": "Bravo bit", "target_s": 60},
           {"joke_id": "joke:x01", "title": "Car one", "target_s": 30}, {"joke_id": "joke:c", "title": "Charlie bit", "target_s": 50},
           {"joke_id": "joke:d", "title": "Delta bit", "target_s": 45}]
WORKINGSETS = [
    {"ts": "2024-03-20T10:00:00+00:00", "op": "promote", "id": "ws:main", "by": "owner",
     "data": {"title": "Main set", "storyboard_id": None, "target_s": 225, "bits": WS_BITS}},
    {"ts": "2024-03-21T10:00:00+00:00", "op": "lock_bit", "id": "ws:main", "by": "owner", "data": {"joke_id": "joke:d"}},
    {"ts": "2024-03-21T11:00:00+00:00", "op": "lock_line", "id": "ws:main", "by": "owner",
     "data": {"joke_id": "joke:b", "text": "The purple monkey dishwasher line here.", "node": "unit:b1"}},
    {"ts": "2024-03-22T10:00:00+00:00", "op": "new_version", "id": "ws:main", "by": "owner",
     "data": {"note": "v2: Charlie before Bravo", "bits": [WS_BITS[0], WS_BITS[3], WS_BITS[1], WS_BITS[2], WS_BITS[4]]}},
    {"ts": "2024-03-01T10:00:00+00:00", "op": "promote", "id": "ws:old", "by": "owner",
     "data": {"title": "Old set", "target_s": 60, "bits": WS_BITS[:1]}}]

STORYBOARDS = [
    {"ts": "2024-06-01T10:00:00+00:00", "op": "create", "id": "sb:seed1", "by": "ai:seed",
     "data": {"title": "Draft from your Insights", "joke_ids": ["joke:c", "joke:d"], "target_s": 300,
              "seed": {"from": "insights.chunks", "chunk": 1, "minutes_est": 5}}},
    {"ts": "2024-06-02T10:00:00+00:00", "op": "create", "id": "sb:old", "by": "owner",
     "data": {"title": "Old seven", "joke_ids": ["joke:a", "joke:nope", "joke:b", "joke:x02"], "target_s": 420}},
    {"ts": "2024-06-10T10:00:00+00:00", "op": "create", "id": "sb:empty", "by": "owner",
     "data": {"title": "Nothing yet", "joke_ids": [], "target_s": 600}},
    {"ts": "2024-06-11T10:00:00+00:00", "op": "create", "id": "sb:gone", "by": "owner",
     "data": {"title": "Gone", "joke_ids": ["joke:a"], "target_s": 300}},
    {"ts": "2024-06-11T11:00:00+00:00", "op": "delete", "id": "sb:gone", "by": "owner", "data": {}}]
# memo key -> the triage verdict: flagged (a conversation, a note, an empty unclear one, each confident), not flagged
# (unsure, or a joke), on a hidden bit (counted only in the hidden view), on a bit that does not exist, one he keeps
SUGGEST_JUDGE = {"version": 1, "items": {
    "memo:1": {"kind": "conversation", "confidence": 0.9, "joke_id": "joke:x06"},
    "memo:2": {"kind": "note", "confidence": 0.5, "joke_id": "joke:x07"},
    "memo:3": {"kind": "unclear", "confidence": 0.8, "chars": 20, "joke_id": "joke:x08"},
    "memo:4": {"kind": "note", "confidence": 0.95, "joke_id": "joke:g"},
    "memo:5": {"kind": "note", "confidence": 0.9, "joke_id": "joke:nope"},
    "memo:6": {"kind": "conversation", "confidence": 0.8, "joke_id": "joke:x09"},
    "memo:7": {"kind": "joke", "confidence": 0.99, "joke_id": "joke:x10"},
    "memo:8": {"kind": "unclear", "confidence": 0.9, "chars": 400, "joke_id": "joke:x11"}}}
# round 8: one set list (each of the bundle's, archived and deleted ones, a miss), one of his sets and its trim, one
# bit's connections and map (every kind of bit, the caps, the misses and the refusals)
CREATIVE_IDS = ([f"/api/boards/{b}" for b in ("sb:seed1", "sb:old", "sb:empty", "sb:gone", "sb:nope")]
                + [f"/api/boards/{b}/build" for b in ("sb:seed1", "sb:old", "sb:empty", "sb:gone", "sb:nope")]
                + ["/api/boards/sb:old?include_hidden=1", "/api/boards/sb:old/build?include_hidden=1",
                   "/api/boards/sb:old?backend=files", "/api/boards/sb%3Aold"]
                + ["/api/mysets/ws:main", "/api/mysets/ws:old", "/api/mysets/ws:nope", "/api/mysets/ws:main?backend=x",
                   "/api/mysets/ws:main/trim?target_s=120", "/api/mysets/ws:main/trim?target_s=300",
                   "/api/mysets/ws:main/trim?target_s=60", "/api/mysets/ws:main/trim?target_s=59",
                   "/api/mysets/ws:main/trim?target_s=14401", "/api/mysets/ws:main/trim?target_s=abc",
                   "/api/mysets/ws:main/trim", "/api/mysets/ws:nope/trim?target_s=300",
                   "/api/mysets/ws:nope/trim?target_s=1", "/api/mysets/ws:main/trim?target_s=99999999999999999999999",
                   f"/api/mysets/match?set={SX}", f"/api/mysets/match?set=set:{SY}", "/api/mysets/match?set=nope"]
                + [f"/api/jokes/joke:{k}/connections" for k in ("a", "b", "c", "d", "e", "f", "x00", "x01", "x03", "x04")]
                + [f"/api/jokes/joke:{k}/map" for k in ("a", "b", "c", "d", "e", "f", "x00", "x01", "x03", "x04")]
                + ["/api/jokes/joke:a/connections?cap=1", "/api/jokes/joke:a/connections?cap=20",
                   "/api/jokes/joke:a/connections?cap=0", "/api/jokes/joke:a/connections?cap=21",
                   "/api/jokes/joke:a/connections?cap=abc", "/api/jokes/joke:a/connections?cap=",
                   "/api/jokes/joke:nope/connections", "/api/jokes/unit:a1/connections",
                   "/api/jokes/joke%20a/connections", "/api/jokes/joke:g/connections?include_hidden=1",
                   "/api/jokes/joke:nope/connections?cap=0", "/api/jokes/joke:a/map?cap=1", "/api/jokes/joke:b/map?cap=20",
                   "/api/jokes/joke:a/map?cap=0", "/api/jokes/joke:a/map?cap=x", "/api/jokes/joke:nope/map",
                   "/api/jokes/joke:g/map", "/api/jokes/joke%3Ab/map", "/api/jokes/joke:a/map?backend=files"])
# round 8: the working sets, his voice profile, the "not a joke?" list, one job, "From your notebook"
LONG_Q = "dog " * 1001
STORE_READS = ["/api/workingsets", "/api/workingsets?backend=files", "/api/workingsets/ws:main", "/api/workingsets/ws:old",
               "/api/workingsets/ws:nope", "/api/workingsets/ws:main/nights", "/api/workingsets/ws:nope/nights",
               "/api/workingsets/ws:main/bits/joke:a", "/api/workingsets/ws:main/bits/joke:b",
               "/api/workingsets/ws:main/bits/joke:d", "/api/workingsets/ws:main/bits/joke:f",
               "/api/workingsets/ws:main/bits/joke:nope", "/api/workingsets/ws:main/bits/joke%20a",
               "/api/workingsets/ws:nope/bits/joke:a", "/api/workingsets/ws:main/bits/joke:g",
               "/api/profile/voice", "/api/profile/voice?backend=files", "/api/suggestions/not-a-joke",
               "/api/suggestions/not-a-joke?include_hidden=1", "/api/suggestions/not-a-joke?backend=x",
               "/api/jobs/job:nope", "/api/jobs/x?backend=files",
               "/api/notebook/related?q=niece", "/api/notebook/related?q=dog%20leash%20park&cap=3",
               "/api/notebook/related?q=traffic&cap=30", "/api/notebook/related?q=traffic&cap=0",
               "/api/notebook/related?q=traffic&cap=31", "/api/notebook/related?q=traffic&cap=x",
               "/api/notebook/related", "/api/notebook/related?q=%20%20", "/api/notebook/related?q=" + LONG_Q.replace(" ", "%20"),
               "/api/notebook/related?q=niece&exclude=joke:d&exclude=joke:e",
               "/api/notebook/related?q=the%20car%0Aparking%20engine%20traffic",
               "/api/notebook/related?q=alpha%20punch&include_hidden=1", "/api/notebook/related?q=zzqqxx",
               "/api/notebook/related?q=niece&backend=files"]
CREATIVE_LISTS = ["/api/settings", "/api/settings?backend=files", "/api/boards", "/api/boards?archived=1",
                  "/api/boards?archived=true", "/api/boards?archived=yes", "/api/boards?include_hidden=1",
                  "/api/boards?backend=files", "/api/mysets", "/api/mysets?include_hidden=1", "/api/mysets?backend=x",
                  "/api/tonight/screen", "/api/tonight/screen?include_hidden=1", "/api/tonight/screen?backend=files"]


# the merges store (creative_event rows; the bundle carries no merges log, the API writes them): a note auto-attached
# under a bit, which the Desk does not list on its own (merge_api.desk_filter), one attached then detached (listed),
# and a pair he dismissed (no effect on the list)
MERGES = [{"ts": "2024-05-01T10:00:00+00:00", "op": "attach", "id": "at:x05", "by": "ai:attach",
               "data": {"joke": "joke:x05", "into": "joke:x01", "text": "x05", "date": "2024-05-01", "score": 0.9,
                        "reasons": ["same words"], "source": "app"}},
              {"ts": "2024-05-01T11:00:00+00:00", "op": "attach", "id": "at:x09", "by": "ai:attach",
               "data": {"joke": "joke:x09", "into": "joke:x01", "text": "x09", "date": "2024-05-01"}},
              {"ts": "2024-05-01T12:00:00+00:00", "op": "detach", "id": "at:x09", "by": "owner", "data": {}},
              {"ts": "2024-05-02T10:00:00+00:00", "op": "dismiss", "id": "dm:x02", "by": "owner",
               "data": {"jokes": ["joke:x02", "joke:x06"], "reason": "kept as its own bit"}}]


ATTACH_LATE = {"ts": "2024-05-03T10:00:00+00:00", "op": "attach", "id": "at:x10", "by": "ai:attach",
               "data": {"joke": "joke:x10", "into": "joke:x02", "text": "x10", "date": "2024-05-03"}}


def surreal_sql(port: int, pw: str, sql: str, vars_: dict | None = None) -> list:
    """One SurrealQL request to the scratch database (root, over HTTP)."""
    import base64
    q = "".join(f"LET ${k} = {json.dumps(v)};" for k, v in (vars_ or {}).items()) + sql
    req = Request(f"http://127.0.0.1:{port}/sql", data=q.encode(), method="POST",
                  headers={"Accept": "application/json", "surreal-ns": NS, "surreal-db": DB,
                           "Authorization": "Basic " + base64.b64encode(f"root:{pw}".encode()).decode()})
    with urlopen(req, timeout=30) as r:
        out = json.loads(r.read())
    bad = [x for x in out if x.get("status") != "OK"]
    if bad:
        raise SystemExit(f"surreal: {bad[0]}")
    return out


def write_merges(port: int, pw: str) -> None:
    import hashlib
    for seq, ev in enumerate(MERGES):
        line = json.dumps(ev)
        row = {"store": "merges", "seq": seq, "line": line, "h": hashlib.sha256(line.encode()).hexdigest(),
               "op": ev["op"], "item": ev["id"], "at": ev["ts"]}
        surreal_sql(port, pw, "CREATE creative_event CONTENT $row RETURN NONE;", {"row": row})


def write_vectors(cache_kb: Path, graph: dict) -> None:
    """Synthetic 384-d unit vectors (four topic centres plus noise, seeded) in the cache layout db/load.py reads, so
    the bundle carries unit embeddings and the cloud has something to place."""
    import numpy as np
    rng = np.random.default_rng(3)
    centres = rng.normal(size=(4, 384))
    ids, rows = [], []
    for n in graph["nodes"]:
        if n["type"] != "unit":
            continue
        key = n["id"].split(":", 1)[1]
        topic = int(key[1:3]) % 4 if key.startswith("x") else TOPIC_OF.get(key[0], 0)
        v = centres[topic] + 0.8 * rng.normal(size=384)
        rows.append(v / np.linalg.norm(v))
        ids.append(n["id"])
    # round 8: reveal vectors (`reveal:<unit>`), what one bit's "could tag it" query compares; x00's lands on a's note
    reveal = {}
    for uid, v in zip(list(ids), list(rows)):
        r = v + 0.3 * rng.normal(size=384)
        reveal[uid] = r / np.linalg.norm(r)
    reveal["unit:x001"] = reveal["unit:a1"] + 0.05 * rng.normal(size=384)
    reveal["unit:x001"] /= np.linalg.norm(reveal["unit:x001"])
    for uid, r in reveal.items():
        ids.append("reveal:" + uid.split(":", 1)[1])
        rows.append(r)
    cache_kb.mkdir(parents=True, exist_ok=True)
    (cache_kb / "vectors_ids.json").write_text(json.dumps(ids), encoding="utf-8")
    np.asarray(rows, dtype=np.float32).tofile(cache_kb / "vectors.f32")


# ---------------------------------------------------------------- processes
def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def surreal_exe() -> str:
    p = os.environ.get("PUNCH_SURREAL") or str(Path.home() / ".punchable" / "bin" / ("surreal.exe" if os.name == "nt" else "surreal"))
    if not Path(p).is_file():
        p = shutil.which("surreal") or p
    if not Path(p).is_file():
        sys.exit(f"no surreal binary (PUNCH_SURREAL or {p})")
    return p


def wait_http(url: str, proc: subprocess.Popen, what: str, secs: float = 120) -> None:
    t = time.time()
    while time.time() - t < secs:
        if proc.poll() is not None:
            raise SystemExit(f"{what} exited {proc.returncode}")
        try:
            with urlopen(url, timeout=2) as r:
                if r.status == 200:
                    return
        except (OSError, HTTPError):
            pass
        time.sleep(0.2)
    raise SystemExit(f"{what} did not become ready at {url}")


class JwksServer:
    def __init__(self, keys: list):
        doc = json.dumps({"keys": [k.jwk() for k in keys]}).encode()

        class H(BaseHTTPRequestHandler):
            def do_GET(self):  # noqa: N802
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(doc)))
                self.end_headers()
                self.wfile.write(doc)

            def log_message(self, *a):
                pass

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.srv.server_address[1]}/certs"


def rss_mib(pid: int) -> float | None:
    try:
        import psutil
        return round(psutil.Process(pid).memory_info().rss / 2**20, 1)
    except Exception:  # noqa: BLE001
        return None


# ---------------------------------------------------------------- HTTP
def call(base: str, method: str, path: str, headers: dict | None = None, body=None):
    data = json.dumps(body).encode() if body is not None else None
    h = dict(headers or {})
    if data is not None:
        h["Content-Type"] = "application/json"
    req = Request(base + path, data=data, method=method, headers=h)
    try:
        with urlopen(req, timeout=60) as r:
            return r.status, {k.lower(): v for k, v in r.headers.items()}, r.read()
    except HTTPError as e:
        return e.code, {k.lower(): v for k, v in e.headers.items()}, e.read()


HEADERS = ("content-type", "cache-control", "www-authenticate", "access-control-allow-origin", "vary",
           "access-control-expose-headers", "access-control-allow-methods", "access-control-allow-headers",
           "access-control-max-age")


def diff(a, b, path="$", out=None):
    out = [] if out is None else out
    if isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b), key=str):
            if k not in a or k not in b:
                out.append((f"{path}.{k}", a.get(k, "<absent>"), b.get(k, "<absent>")))
            else:
                diff(a[k], b[k], f"{path}.{k}", out)
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            out.append((f"{path}.len", len(a), len(b)))
        for i, (x, y) in enumerate(zip(a, b)):
            diff(x, y, f"{path}[{i}]", out)
    elif isinstance(a, bool) or isinstance(b, bool):
        if a is not b:
            out.append((path, a, b))
    elif (path.startswith("$.nodes[") or path.startswith("$.clusters[")) and path.rsplit(".", 1)[-1] in TOLERANT_KEYS \
            and isinstance(a, (int, float)) and isinstance(b, (int, float)):
        if abs(a - b) > TOLERANCE:
            out.append((path, a, b))
    elif a != b:
        out.append((path, a, b))
    return out


def compare(label: str, py: tuple, rs: tuple) -> list[str]:
    bad = []
    if py[0] != rs[0] and (label, "status") not in ALLOW:
        bad.append(f"{label}: status python {py[0]} rust {rs[0]}")
    for h in HEADERS:
        if py[1].get(h) != rs[1].get(h) and (label, f"header:{h}") not in ALLOW:
            bad.append(f"{label}: header {h}: python {py[1].get(h)!r} rust {rs[1].get(h)!r}")
    if py[2] or rs[2]:
        try:
            a, b = json.loads(py[2] or b"null"), json.loads(rs[2] or b"null")
        except ValueError:
            if py[2] != rs[2]:
                bad.append(f"{label}: bodies differ and are not both JSON")
            return bad
        for p, x, y in diff(a, b):
            if label in DEGENERATE and p.rsplit(".", 1)[-1] in TOLERANT_KEYS:
                continue
            if ("/similar" in label and p.rsplit(".", 1)[-1] in ("distance", "score")
                    and isinstance(x, (int, float)) and isinstance(y, (int, float)) and abs(x - y) <= VECTOR_TOLERANCE):
                continue
            if (label, p) not in ALLOW:
                bad.append(f"{label}: {p}: python {json.dumps(x)[:120]} rust {json.dumps(y)[:120]}")
    return bad


def compare_ranked(label: str, pys: list, rs) -> list[str]:
    """pk-api's answer against several of the Python's own answers to the same request, which differ only in the
    database's tie order. Outside the ranked lists the answer must equal one of the Python's exactly. Each ranked list
    must hold exactly the rows (as a multiset) of one of the Python's lists; a place where every Python answer has the
    same row must hold that row; any other place must hold a row some Python answer put at that same place."""
    # /similar nests its two ranked lists under `like`
    flat = lambda d: {**{k: v for k, v in d.items() if k != "like"},  # noqa: E731
                      **{f"like.{k}": v for k, v in (d.get("like") or {}).items()}}
    pys, rs = [flat(p) for p in pys], flat(rs)
    rest = lambda d: {k: v for k, v in d.items() if k.rsplit(".", 1)[-1] not in RANKED_LISTS}  # noqa: E731
    rows_of = lambda xs: sorted(json.dumps(x, sort_keys=True) for x in xs)  # noqa: E731
    bad = []
    if all(diff(rest(p), rest(rs)) for p in pys):
        bad.append(f"{label}: outside the ranked lists pk-api equals none of the Python's {len(pys)} answers")
    for key in [k for k in rs if k.rsplit(".", 1)[-1] in RANKED_LISTS]:
        lists = [p.get(key) or [] for p in pys]
        mine = rs[key]
        if rows_of(mine) not in [rows_of(x) for x in lists]:
            bad.append(f"{label}: $.{key} is not a reordering of any of the Python's {len(lists)} lists")
            continue
        for i, row in enumerate(mine):
            here = [y[i] for y in lists if i < len(y)]
            stable = len(here) == len(lists) and all(r == here[0] for r in here)
            if stable and row != here[0]:
                bad.append(f"{label}: $.{key}[{i}] differs where the Python never varies: "
                           f"python {json.dumps(here[0])[:120]} rust {json.dumps(row)[:120]}")
            elif not stable and row not in here:
                bad.append(f"{label}: $.{key}[{i}] is a row the Python never put at that place: {json.dumps(row)[:160]}")
    return bad


# ---------------------------------------------------------------- the run
def coverage(py: str, app: dict) -> list[str]:
    """Equal answers only prove something when they carry the cases: the Python's own pages must show each rule the
    synthetic tapes were written to reach (a gate that compares two empty lists passes without doing work)."""
    get = lambda p: json.loads(call(py, "GET", p, app)[2])  # noqa: E731
    out = []
    rv = get(f"/api/sets/set:{SX}/review")["steps"]
    checks = {
        "review: a 'new line?' step from an uncovered laugh": any(s["key"].startswith("line:") for s in rv),
        "review: a punch of two short segments": any(sum(x["punch"] for x in s["punch"]["segments"]) == 2 for s in rv),
        "review: a step with laughs and a match": any(s["laughs"] and s["match"] for s in rv),
    }
    an = get(f"/api/sets/set:{SX}/analysis")
    r1 = an["rows"][0]
    checks |= {"analysis: ticks, hints with an edge, similar, punchline play": bool(
        r1["ticks"] and any(h["edge"] for h in r1["hints"]) and r1["similar"] and r1["play"]["end_ms"] > 0),
        "analysis: summary best and weakest": bool(an["summary"]["best"] and an["summary"]["weakest"])}
    checks["analysis: a saved new moment"] = any(r["saved_as"] for r in get(f"/api/sets/set:{SY}/analysis")["rows"])
    cards = {c["id"]: c for c in get("/api/sets")["sets"]}
    checks["sets: a drive reference, a file reference, laughs counted"] = (
        (cards[f"set:{SZ}"]["reference"] or {}).get("kind") == "drive"
        and (cards[f"set:{SY}"]["reference"] or {}).get("kind") == "file" and cards[f"set:{SX}"]["laughs"] > 0)
    ja = get("/api/jokes/joke:a")
    checks["joke: a performance with laughs, a punch and a labelled edge"] = bool(
        ja["performances"] and ja["performances"][0]["laughs"] and ja["performances"][0]["punch"]["text"] and ja["labels"])
    js = get("/api/jokes?limit=5000")["items"]
    ids = {x["id"] for x in js}
    checks["jokes: the attached note out, the detached one in, both kinds of duration"] = (
        "joke:x05" not in ids and "joke:x09" in ids and any(not x["duration_estimated"] for x in js)
        and any(x["duration_estimated"] for x in js) and len({x["duration_s"] for x in js}) > 3)
    td = get("/api/today")
    ws_ = td.get("working_set") or {}
    checks |= {"today: a working set with a last night": bool(ws_.get("last_night")),
               "today: notes to try tonight": bool(td.get("try_tonight")),
               "today: the latest analysis and the card's moments": bool(
                   td.get("latest_analysis") and (td.get("latest_set") or {}).get("moments") is not None),
               "today: gems and orphans": {p["kind"] for p in td.get("serendipity") or []} == {"gem", "orphan"},
               "today: glance hints": bool(td["glance"]["hints"]),
               "today: minutes in testing": bool(td["minutes"]["n_testing"])}
    checks["jokes: the hidden view keeps the attached note"] = any(
        x["id"] == "joke:x05" for x in get("/api/jokes?limit=5000&include_hidden=1")["items"])
    # round 8: one bit's connections and map, one set list, one of his sets
    con = {g["key"]: g["items"] for g in get("/api/jokes/joke:a/connections?cap=20")["groups"]}
    checks["connections: a confirmed tag, a tag by the reveal, next by tape, same shape"] = bool(
        any(x.get("source") == "inferred" for x in con["tags"]) and any("reveal_query" in x["basis"] for x in con["tags"])
        and any("back_to_back" in x["basis"] for x in con["next"]) and con["same_shape"])
    cd = get("/api/jokes/joke:d/connections")
    cdg = {g["key"]: g["items"] for g in cd["groups"]}
    checks["connections: a callback, a chunk, a persona and the stage trend"] = bool(
        cdg["callbacks"] and any("chunk" in x["basis"] for x in cdg["same_world"])
        and any(x.get("personas") for x in cdg["same_world"]))
    cb = {g["key"]: g["items"] for g in get("/api/jokes/joke:b/connections")["groups"]}
    checks["connections: another angle, his note, a trend"] = bool(cb["contrasts"]) and bool(
        get("/api/jokes/joke:b/connections")["land_harder"]) and get("/api/jokes/joke:a/connections")["stage_trend"]["trend"]
    mp = {g["key"]: g["items"] for g in get("/api/jokes/joke:a/map")["groups"]}
    checks["map: next, a tag with its line, the same world, an older version"] = bool(
        mp["next"] and mp["tags"] and mp["tags"][0]["quote"] and mp["same_world"] and mp["versions"])
    subj = [x for k in ("a", "b", "c", "d", "x00", "x01", "x03") for x in
            {g["key"]: g["items"] for g in get(f"/api/jokes/joke:{k}/map")["groups"]}["same_world"] if "subject" in x]
    checks["map: a same-subject bit (the similarity band computed past 50 bits)"] = bool(subj)
    bv = get("/api/boards/sb:old")
    checks["board: a jump, a closer note, best next with a reason"] = bool(
        any(t["label"] == "jump" for t in bv["health"]["transitions"])
        and any(c["rule"] == "closer" for c in bv["health"]["checks"]) and bv["best_next"])
    bb = get("/api/boards/sb:seed1/build")
    checks["build: steps with text, next cards"] = bool(bb["steps"] and bb["steps"][0]["text"] and bb["next"])
    ms_ = get("/api/mysets/ws:main")
    checks["myset: tough spots with actions, a tape that lines up, versions"] = bool(
        any(sp["actions"] for b_ in ms_["bits"] for sp in b_["tough"]) and ms_["tapes"]["n"] and len(ms_["versions"]) > 1)
    wsd = get("/api/workingsets/ws:main")
    checks["workingset: try tonight with ids, a last night, a perfected score with rows"] = bool(
        wsd["try_tonight"] and all("id" in t for t in wsd["try_tonight"])
        and any(b_["last_night"] for b_ in wsd["bits"]) and wsd["perfected"]["bits"])
    wb = get("/api/workingsets/ws:main/bits/joke:a")
    checks["workingset bit: wording effects and night-to-night changes"] = bool(
        wb["wording"]["effects"] and wb["wording"]["changes"] and wb["best_night"])
    rl = get("/api/notebook/related?q=niece")
    checks["related: cards with a line and a reason"] = bool(rl["items"] and rl["items"][0]["quote"] and rl["items"][0]["why"])
    sr = get("/api/search?q=niece")
    checks["search: vectors, bits with the matched line, moments"] = bool(
        sr["vectors"] and sr["bits"] and any(b["why"] for b in sr["bits"]) and get("/api/search?q=traffic")["moments"])
    checks["search: a moment performed as a bit, with its room and place on the tape"] = any(
        m["joke"] and m["venue_short"] and m["at_label"] for m in get("/api/search?q=traffic")["moments"])
    checks["search: the hidden bit only in the hidden view"] = (
        all(b["id"] != "joke:g" for b in get("/api/search?q=grocery")["bits"])
        and any(b["id"] == "joke:g" for b in get("/api/search?q=grocery&include_hidden=1")["bits"]))
    sm = get("/api/jokes/joke:a/similar")
    checks["similar: performed moments, other bits with distances, unlinked moments"] = bool(
        sm["vectors"] and sm["performed"] and sm["like"]["bits"] and sm["like"]["bits"][0]["distance"] is not None
        and sm["like"]["moments"] and sm["text"])
    checks["hybrid: a ranked list"] = len(get("/api/jokes?q=niece&mode=hybrid")["items"]) >= 2
    # round 7 (the bundle's creative stores, before any write of his)
    tn = get("/api/tonight/screen")
    bds = {b["id"]: b for b in get("/api/boards")["boards"]}
    checks["tonight: his newest list with bits, a bit not in the notebook left out, a gap named"] = bool(
        (tn["board"] or {}).get("id") == "sb:old" and "joke:nope" not in [s["joke_id"] for s in tn["board"]["segments"]]
        and tn["board"]["first_gap"] and tn["latest_set"] and len(tn["questions"]) == 3)
    checks["tonight: flagged memos counted, the hidden bit only in the hidden view"] = (
        tn["to_sort"] >= 2 and get("/api/tonight/screen?include_hidden=1")["to_sort"] == tn["to_sort"] + 1)
    checks["boards: a seeded draft, his own, the deleted one gone, a bit not in the notebook named nowhere"] = (
        set(bds) == {"sb:seed1", "sb:old", "sb:empty"} and bds["sb:seed1"]["seed"] and bds["sb:seed1"]["source"] == "inferred"
        and bds["sb:old"]["n"] == 4 and len(bds["sb:old"]["names"]) == 3)
    checks["settings: the defaults before he changes one"] = get("/api/settings")["source"] == "default"
    for what, ok_ in checks.items():
        if not ok_:
            out.append(f"coverage: the synthetic data no longer reaches '{what}'")
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pk-api", required=True, type=Path)
    ap.add_argument("--bench", type=int, default=0, help="requests per route group per server for the timing (0 = none)")
    ap.add_argument("--report", type=Path, default=None)
    ap.add_argument("--keep", action="store_true", help="keep the temp folder (logs)")
    a = ap.parse_args()

    tmp = Path(tempfile.mkdtemp(prefix="pk-t9-"))
    comedy, data, home = tmp / "comedy", tmp / "data", tmp / "home"
    for d in (data, home, tmp / "cache"):
        d.mkdir()
    build_comedy(comedy)
    write_vectors(tmp / "cache" / "kb", make_graph())
    port = free_port()
    pw = secrets.token_hex(16)
    env = {**os.environ, "PUNCHABLE_HOME": str(home), "COMEDY_DIR": str(comedy), "PUNCH_COMEDY": str(comedy),
           "PK_CACHE_DIR": str(tmp / "cache"), "PK_DBLOAD_DIR": str(tmp / "dbload"), "SURREAL_PORT": str(port),
           "SURREAL_USER": "root", "SURREAL_PASS": pw, "SURREAL_NS": NS, "SURREAL_DB": DB, "PYTHONHASHSEED": "0",
           "PUNCH_API_QUIET": "1", "PYTHONIOENCODING": "utf-8",
           # pk-api reads PUNCH_SECRETS (default ~/.punchable/secrets.env) under the process env: point it at nothing, so
           # no real DSN, database user or token can reach this run
           "PUNCH_SECRETS": str(home / "secrets.env")}
    for k in ("SURREAL_URL", "SURREAL_API_USER", "SURREAL_API_PASS", "SENTRY_DSN", "PUNCHABLE_LEGAL_RULES",
              "PK_EMBEDDER"):
        env.pop(k, None)
    # round 6: search embeds on both sides, with the same bge-small folder (tools/kb/fetch_bge.py); the Python's
    # sweeper makes one pass at start (the stored vectors) and no second one while the matrix runs
    bge = env.get("PK_BGE_DIR", "")
    if not (Path(bge) / "onnx" / "model.onnx").is_file() or not (Path(bge) / "tokenizer.json").is_file():
        raise SystemExit("set PK_BGE_DIR to the bge-small folder (python tools/kb/fetch_bge.py --dest <dir>): "
                         "search, /similar and the hybrid list embed on both servers")
    env["PK_VEC_SWEEP_S"] = "86400"
    procs: list[subprocess.Popen] = []
    logs = []

    def start(args, extra, name):
        log = open(tmp / f"{name}.log", "w", encoding="utf-8")
        logs.append(log)
        p = subprocess.Popen(args, env={**env, **extra}, stdout=log, stderr=subprocess.STDOUT, cwd=str(ROOT))
        procs.append(p)
        return p

    problems: list[str] = []
    report: dict = {"machine": f"{platform.node()} ({platform.system()} {platform.release()}, {platform.processor()})",
                    "python": platform.python_version()}
    try:
        sp = start([surreal_exe(), "start", "--user", "root", "--pass", pw, "--bind", f"127.0.0.1:{port}", "memory"], {}, "surreal")
        wait_http(f"http://127.0.0.1:{port}/health", sp, "surreal")
        tgz = tmp / "bundle.tgz"
        for step in (["pack", "--comedy", str(comedy), "--tar", str(tgz)], ["apply", "--tar", str(tgz)]):
            r = subprocess.run([sys.executable, str(ROOT / "db" / "push_remote.py"), *step], env=env, cwd=str(ROOT),
                               capture_output=True, text=True, encoding="utf-8")
            if r.returncode != 0:
                raise SystemExit(f"push_remote {step[0]} failed:\n{r.stdout[-2000:]}\n{r.stderr[-3000:]}")
        write_merges(port, pw)

        key, other = JK.Key("t9", seed=9), JK.Key("t9-other", seed=10)
        jwks = JwksServer([key])
        auth_env = {"PK_AUTH": "1", "OIDC_ISSUER": JK.ISS, "OIDC_AUDIENCE": JK.AUD, "OIDC_JWKS_URL": jwks.url,
                    "PUBLIC_URL": ORIGIN + "/", "PK_APP_FLAGS": FLAGS, "PK_DATA": str(data), "HOST": "127.0.0.1",
                    "PK_BOOT_POLL_S": "1"}
        py_port, rs_port = free_port(), free_port()
        py = f"http://127.0.0.1:{py_port}"
        rs = f"http://127.0.0.1:{rs_port}"
        pp = start([sys.executable, str(ROOT / "api" / "entry.py")], {**auth_env, "PORT": str(py_port)}, "python-api")
        wait_http(py + "/api/health?ready=1", pp, "python api")
        rp = start([str(a.pk_api)], {**auth_env, "PORT": str(rs_port), "PK_PYTHON_URL": py, "RUST_LOG": os.environ.get("RUST_LOG", "info")}, "pk-api")
        wait_http(rs + "/api/health?ready=1", rp, "pk-api")

        now = time.time()
        app = {"Authorization": "Bearer " + JK.token(key, now=now)}
        tok = lambda **c: {"Authorization": "Bearer " + JK.token(key, now=now, **c)}  # noqa: E731
        # the Python's first vector sweep (typed ideas and moments embedded with the model) before anything is compared
        deadline = time.time() + 300
        while True:
            vec = ((json.loads(call(py, "GET", "/api/backend", app)[2]).get("backends") or {}).get("surreal") or {}).get("vectors") or {}
            if vec.get("runs", 0) >= 1:
                break
            if time.time() > deadline:
                raise SystemExit(f"the Python's vector sweep did not finish: {vec}")
            time.sleep(0.5)
        if vec.get("embedder") != "bge-small-en-v1.5" or vec.get("error"):
            raise SystemExit(f"the Python is not searching with the model: {vec}")
        report["python_sweep"] = vec.get("last")

        # -- writes through pk-api (forwarded to the Python), each followed at once by a pk-api read
        raw_ms = []
        writes = [("POST", "/api/ideas", {"text": "Trains are late every morning here. The conductor waves anyway."}),
                  ("POST", "/api/decision", {"target": "node", "id": "joke:c", "action": "edit", "after": {"stage": "working"}}),
                  ("POST", "/api/decision", {"target": "edge", "id": "e:la-d1", "action": "confirm"}),
                  ("POST", "/api/review/rate", {"perf": f"perf:{SX}@0", "rating": "killed"}),
                  ("POST", "/api/review/rate", {"perf": f"perf:{SX}@60000", "rating": "meh"}),
                  ("POST", "/api/decision", {"target": "node", "id": "joke:e", "action": "edit", "after": {"stage": "testing"}}),
                  # his room edit: the card's venue changes, its venue_id keeps the room it was (set_info)
                  ("POST", "/api/decision", {"target": "node", "id": f"set:{SY}", "action": "edit",
                                             "after": {"venue": "Comix at Riverside (show)"}})]
        want = json.loads(call(rs, "GET", "/api/decisions?limit=1", app)[2])["total"]
        for i, (m, p, b) in enumerate(writes):
            st, _, body = call(rs, m, p, app, b)
            if st != 200:
                raise SystemExit(f"write {p} through pk-api: HTTP {st} {body[:300]!r}")
            want += len(json.loads(body)["indexes"])
            t = time.perf_counter()
            st, _, body = call(rs, "GET", "/api/decisions?limit=1", app)
            raw_ms.append((time.perf_counter() - t) * 1000)
            total = json.loads(body)["total"]
            if total != want:
                problems.append(f"read-after-write: after write {i + 1} pk-api shows {total} decisions, wanted {want}")
        st, _, body = call(rs, "POST", "/api/undo", app, {"index": want - 1})
        if st != 200:
            raise SystemExit(f"undo through pk-api: HTTP {st} {body[:300]!r}")
        report["read_after_write_ms"] = [round(x, 1) for x in raw_ms]
        report["read_after_write_max_ms"] = round(max(raw_ms), 1)
        if max(raw_ms) > 100:
            problems.append(f"read-after-write: a pk-api read took {max(raw_ms):.0f} ms after a write (gate 100 ms)")

        # -- the matrix
        reqs: list[tuple[str, str, str, dict]] = []
        for p in ("/api/labels", "/api/labels?include_hidden=1", "/api/decisions", "/api/decisions?limit=2",
                  "/api/decisions?limit=0", "/api/decisions?limit=-2", "/api/decisions?limit=abc", "/api/decisions?limit=9999",
                  "/api/decisions?limit=%203%20", "/api/meta", "/api/meta?labels=2", "/api/meta?labels=-1",
                  "/api/meta?include_hidden=1", "/api/meta?labels=x", "/api/pipeline", "/api/pipeline?limit=1",
                  "/api/pipeline?include_hidden=true", "/api/pipeline?limit=-1", "/api/learned", "/api/health",
                  "/api/health?ready=1", "/api/labels?backend=files", "/api/labels?backend=surreal",
                  "/api/labels?backend=", "/api/health?backend=x",
                  "/api/map/cloud", "/api/map/cloud?cap=5", "/api/map/cloud?cap=0", "/api/map/cloud?cap=abc",
                  "/api/map/cloud?cap=2001", "/api/map/cloud?cap=2000", "/api/map/cloud?cap=1",
                  "/api/map/cloud?cap=2", "/api/map/cloud?cap=3", "/api/map/cloud?cap=12"):
            reqs.append((p, "GET", p, app))
        # round 4: the tape pages and one bit's page, both views, every set and bit, the misses
        pages = ["/api/sets", "/api/sets?include_hidden=1", "/api/sets/set:nope/analysis", "/api/sets/joke:a/analysis",
                 "/api/sets/set:nope/review", "/api/jokes/joke:nope", "/api/jokes/unit:a1", "/api/jokes/joke%3Aa",
                 "/api/jokes/joke:g", "/api/jokes/joke:g?include_hidden=1", "/api/jokes/joke:a?backend=files",
                 "/api/sets/set:" + SX + "/review?include_hidden=1"]
        for sk in (SX, SY, SZ):
            pages += [f"/api/sets/set:{sk}/analysis", f"/api/sets/set:{sk}/review"]
        pages += [f"/api/jokes/joke:{k}" for k in "abcdef"] + ["/api/jokes/joke:x00", "/api/jokes/joke:x07"]
        reqs += [(p, "GET", p, app) for p in pages]
        # round 5: the Bits list (filters, every sort, the page bounds, both views, the attached note taken out)
        jokes = ["/api/jokes", "/api/jokes?limit=5000", "/api/jokes?limit=5000&include_hidden=1", "/api/jokes?limit=3",
                 "/api/jokes?offset=2&limit=2", "/api/jokes?offset=-5", "/api/jokes?offset=999", "/api/jokes?offset=abc",
                 "/api/jokes?limit=0", "/api/jokes?limit=-3", "/api/jokes?limit=99999", "/api/jokes?limit=x",
                 "/api/jokes?sort=bad", "/api/jokes?sort=bad&offset=abc", "/api/jokes?q=niece", "/api/jokes?q=%20%20",
                 "/api/jokes?q=NIECE%20laundromat", "/api/jokes?q=nothingmatches", "/api/jokes?stage=idea",
                 "/api/jokes?stage=testing&sort=strongest", "/api/jokes?label=label:persona:dad",
                 "/api/jokes?label=label:persona:dad&label=label:topic:kids", "/api/jokes?label=label:nope",
                 "/api/jokes?who=edited", "/api/jokes?who=confirmed", "/api/jokes?who=ai&limit=5", "/api/jokes?who=nope",
                 "/api/jokes?mode=hybrid", "/api/jokes?q=%20&mode=hybrid", "/api/jokes?backend=files",
                 "/api/jokes?include_hidden=1&q=grocery", "/api/jokes?q=x05", "/api/jokes?q=x09"]
        jokes += TODAY[:3]
        jokes += [f"/api/jokes?sort={s}&limit=5000" + h for s in ("recent", "performed", "strongest", "written", "untouched")
                  for h in ("", "&include_hidden=1")]
        reqs += [(p, "GET", p, app) for p in jokes]
        # round 6: search (words and vectors fused), one bit's neighbours, the hybrid Bits list; both views, the misses,
        # the refusals
        search = ["/api/search", "/api/search?q=%20", "/api/search?q=niece", "/api/search?q=NIECE%20laundromat",
                  "/api/search?q=niece&limit=1", "/api/search?q=niece&limit=0", "/api/search?q=dog&limit=99",
                  "/api/search?q=dog&limit=abc", "/api/search?q=traffic", "/api/search?q=car", "/api/search?q=alpha%20punch",
                  "/api/search?q=grocery", "/api/search?q=grocery&include_hidden=1", "/api/search?q=nothingmatcheszz",
                  "/api/search?q=dog&backend=files", "/api/search?q=pilot%20luggage", "/api/search?q=the",
                  "/api/search?q=opener", "/api/search?q=crowd&include_hidden=1",
                  "/api/search?q=dog&limit=99999999999999999999", "/api/search?q=dog&limit=-99999999999999999999"]
        similar = [f"/api/jokes/joke:{k}/similar" for k in "abcdef"] + [
            "/api/jokes/joke:a/similar?limit=1", "/api/jokes/joke:a/similar?limit=0", "/api/jokes/joke:a/similar?limit=abc",
            "/api/jokes/joke:b/similar?limit=50", "/api/jokes/joke:g/similar", "/api/jokes/joke:g/similar?include_hidden=1",
            "/api/jokes/joke:a/similar?include_hidden=1", "/api/jokes/joke:nope/similar", "/api/jokes/unit:a1/similar",
            "/api/jokes/joke%3Ac/similar", "/api/jokes/joke:x05/similar", "/api/jokes/joke:x10/similar?limit=3",
            "/api/jokes/a/b/similar", "/api/jokes/joke:a/similar?limit=99999999999999999999"]
        hybrid = ["/api/jokes?q=niece&mode=hybrid", "/api/jokes?q=dog&mode=hybrid&limit=3",
                  "/api/jokes?q=traffic&mode=hybrid&sort=recent", "/api/jokes?q=grocery&mode=hybrid",
                  "/api/jokes?q=grocery&mode=hybrid&include_hidden=1", "/api/jokes?q=dog&mode=hybrid&limit=abc",
                  "/api/jokes?q=car&mode=hybrid&stage=idea", "/api/jokes?q=x05&mode=hybrid",
                  "/api/jokes?q=dog&mode=hybrid&offset=2&limit=2", "/api/jokes?q=pilot&mode=hybrid&limit=-4",
                  "/api/jokes?q=pilot&mode=hybrid&limit=7", "/api/search?q=pilot&limit=7",
                  "/api/jokes?q=dog&mode=hybrid&limit=99999999999999999999", "/api/jokes?offset=99999999999999999999",
                  "/api/decisions?limit=99999999999999999999"]
        reqs += [(p, "GET", p, app) for p in search + similar + hybrid]
        # round 7: the creative lists over the bundle's stores (no write of his yet), and their refusals
        reqs += [(p, "GET", p, app) for p in CREATIVE_LISTS]
        reqs += [(p, "GET", p, app) for p in CREATIVE_IDS]
        reqs += [(p, "GET", p, app) for p in STORE_READS]
        reqs += [("related signed out", "GET", "/api/notebook/related?q=niece", {}),
                 ("workingsets signed out", "GET", "/api/workingsets", {})]
        reqs += [("board signed out", "GET", "/api/boards/sb:old", {}),
                 ("connections signed out", "GET", "/api/jokes/joke:a/connections", {})]
        reqs += [("tonight signed out", "GET", "/api/tonight/screen", {}), ("settings signed out", "GET", "/api/settings", {})]
        # round 8: a set's clips (one with a clips file, one without, a miss)
        reqs += [(p, "GET", p, app) for p in (f"/api/sets/set:{SX}/clips", f"/api/sets/set:{SY}/clips",
                                              "/api/sets/set:nope/clips", "/api/sets/joke:a/clips",
                                              f"/api/sets/set:{SX}/clips?include_hidden=1")]
        reqs += [("HEAD meta", "HEAD", "/api/meta", app),
                 ("health signed out", "GET", "/api/health", {}),
                 ("health bad token", "GET", "/api/health", {"Authorization": "Bearer x.y.z"}),
                 ("OPTIONS labels", "OPTIONS", "/api/labels", {"Origin": ORIGIN}),
                 ("OPTIONS labels evil", "OPTIONS", "/api/labels", {"Origin": "https://evil.example"}),
                 ("CORS public", "GET", "/api/labels", {**app, "Origin": ORIGIN}),
                 ("CORS loopback", "GET", "/api/meta", {**app, "Origin": "http://localhost:8081"}),
                 ("CORS evil", "GET", "/api/meta", {**app, "Origin": "http://localhost.evil.example"}),
                 ("CORS 401", "GET", "/api/meta", {"Origin": ORIGIN}),
                 ("no token", "GET", "/api/labels", {}),
                 ("not bearer", "GET", "/api/labels", {"Authorization": "Basic abc"}),
                 ("garbage", "GET", "/api/labels", {"Authorization": "Bearer garbage"}),
                 ("bad chars", "GET", "/api/labels", {"Authorization": "Bearer a$b.c.d"}),
                 ("expired", "GET", "/api/labels", tok(exp=int(now) - 3600)),
                 ("in skew", "GET", "/api/labels", tok(exp=int(now) - 30)),
                 ("no exp", "GET", "/api/labels", tok(exp=None)),
                 ("wrong aud", "GET", "/api/labels", tok(aud="account")),
                 ("aud string", "GET", "/api/labels", tok(aud=JK.AUD)),
                 ("wrong iss", "GET", "/api/labels", tok(iss="https://elsewhere.example/realms/x")),
                 ("nbf ahead", "GET", "/api/labels", tok(nbf=int(now) + 3600)),
                 ("iat ahead", "GET", "/api/labels", tok(iat=int(now) + 3600)),
                 ("refresh typ", "GET", "/api/labels", tok(typ="Refresh")),
                 ("no typ", "GET", "/api/labels", tok(typ=None)),
                 ("worker on app", "GET", "/api/labels", tok(realm_access={"roles": ["worker"]})),
                 ("unknown kid", "GET", "/api/labels", {"Authorization": "Bearer " + JK.token(other, now=now)}),
                 ("HS256", "GET", "/api/labels", {"Authorization": "Bearer " + JK.token(key, now=now, header={"alg": "HS256"})}),
                 ("no kid", "GET", "/api/labels", {"Authorization": "Bearer " + JK.token(key, now=now, header={"kid": None})}),
                 ("bad sig", "GET", "/api/labels", {"Authorization": "Bearer " + JK.token(key, now=now)[:-6] + "AAAAAA"}),
                 ("proxied 404", "GET", "/api/nothing-here", app),
                 ("search no token", "GET", "/api/search?q=dog", {}),
                 ("similar no token", "GET", "/api/jokes/joke:a/similar", {})]
        ranked = set(search + similar + hybrid)
        unstable: dict[str, int] = {}

        def matrix(py: str, rs: str, reqs: list) -> tuple[list[str], int]:
            bad, n = [], 0
            for label, m, p, h in reqs:
                pyr, rsr = call(py, m, p, h), call(rs, m, p, h)
                got = compare(label, pyr, rsr)
                if got and p in ranked and pyr[0] == rsr[0] == 200:
                    # the database's tie order: ask the Python again; if it disagrees with itself, compare up to the ties
                    seen = [pyr] + [call(py, m, p, h) for _ in range(11)]
                    if len({x[2] for x in seen}) > 1:
                        heads = [g for g in got if g.startswith(f"{label}: header ")]
                        ranked_bad = compare_ranked(label, [json.loads(x[2]) for x in seen], json.loads(rsr[2]))
                        if ranked_bad:
                            # round 7: twelve answers do not always show every place a tied row can take (one run in
                            # eight put a tied row first where the Python's twelve never did): ask 36 more, same rule
                            seen += [call(py, m, p, h) for _ in range(36)]
                            ranked_bad = compare_ranked(label, [json.loads(x[2]) for x in seen], json.loads(rsr[2]))
                        unstable[label] = len({x[2] for x in seen})
                        got = heads + ranked_bad
                bad += got
                n += not got
            return bad, n

        got, n_ok = matrix(py, rs, reqs)
        problems += got
        report["tie_order_requests"] = unstable
        problems += coverage(py, app)
        # a transcript the worker rewrites (POST /api/worker/files through pk-api): files_rev moves, the Python pod
        # refreshes its disk copy within a poll, pk-api's transcript cache misses on the new hash; both pages again
        worker = tok(realm_access={"roles": ["worker"]})
        tpath = f"corpus/sets/{set_dir(SX)}/transcript.json"
        new_tx = {"segments": [{"start": 0, "end": 30, "text": "A rewritten opener for the worker test."},
                               {"start": 50, "end": 56, "text": "Tiny."}, {"start": 56, "end": 59.5, "text": "Ha."}]}
        before = call(py, "GET", f"/api/sets/set:{SX}/review", app)[2]
        st, _, body = call(rs, "POST", "/api/worker/files", worker,
                           {"files": [{"path": tpath, "text": json.dumps(new_tx)}], "actor": "import:upload"})
        if st != 200:
            raise SystemExit(f"worker files through pk-api: HTTP {st} {body[:300]!r}")
        deadline = time.time() + 30
        while call(py, "GET", f"/api/sets/set:{SX}/review", app)[2] == before and time.time() < deadline:
            time.sleep(0.5)
        for p in (f"/api/sets/set:{SX}/review", "/api/jokes/joke:a", "/api/jokes/joke:b"):
            label = "after a transcript post: " + p
            got = compare(label, call(py, "GET", p, app), call(rs, "GET", p, app))
            problems += got
            n_ok += not got
            reqs.append((label, "GET", p, app))
        if call(rs, "GET", f"/api/sets/set:{SX}/review", app)[2] == before:
            problems.append("after a transcript post: pk-api still shows the old transcript")
        # his own list for tonight (a write through pk-api): Today shows it saved, and the creative half lists it too
        st, _, body = call(rs, "POST", "/api/tonight", app, {"items": [{"joke": "joke:c", "done": True},
                                                                       {"joke": "joke:x02", "done": False}]})
        if st != 200:
            raise SystemExit(f"tonight through pk-api: HTTP {st} {body[:300]!r}")
        label = TODAY[3]
        got = compare(label, call(py, "GET", "/api/today", app), call(rs, "GET", "/api/today", app))
        problems += got
        n_ok += not got
        reqs.append((label, "GET", "/api/today", app))
        # a note attached after pk-api has served the list (creative writes do not move the generation): the very next
        # list on both servers leaves it out
        seen = call(rs, "GET", "/api/jokes?q=x10", app)[2]
        surreal_sql(port, pw, "CREATE creative_event CONTENT $row RETURN NONE;", {"row": {
            "store": "merges", "seq": len(MERGES), "op": "attach", "item": "at:x10", "at": "2024-05-03T10:00:00+00:00",
            "line": json.dumps(ATTACH_LATE), "h": __import__("hashlib").sha256(json.dumps(ATTACH_LATE).encode()).hexdigest()}})
        for p in ("/api/jokes?q=x10", "/api/jokes?limit=5000"):
            label = "after a late attach: " + p
            got = compare(label, call(py, "GET", p, app), call(rs, "GET", p, app))
            problems += got
            n_ok += not got
            reqs.append((label, "GET", p, app))
        if "joke:x10" not in seen.decode() or "joke:x10" in call(rs, "GET", "/api/jokes?q=x10", app)[2].decode():
            problems.append("after a late attach: pk-api did not take the newly attached note out of the list")
        # round 7: the creative lists again after writes through pk-api (each forwarded to the Python), every list
        # read by both servers right after the write it must show: set lists made (tonight's list moves to the newest
        # of his), one archived, his settings, a set made and one deleted, a "not a joke?" he keeps, the worker's claim
        def cwrite(m: str, p: str, b: dict, h: dict | None = None) -> dict:
            st_, _, body_ = call(rs, m, p, h or app, b)
            if st_ not in (200, 201, 202, 204):
                raise SystemExit(f"{m} {p} through pk-api: HTTP {st_} {body_[:300]!r}")
            return json.loads(body_) if body_ else {}

        def again(label: str, paths: list[str]) -> None:
            nonlocal n_ok
            for p in paths:
                lab = f"{label}: {p}"
                got = compare(lab, call(py, "GET", p, app), call(rs, "GET", p, app))
                problems.extend(got)
                n_ok += not got
                reqs.append((lab, "GET", p, app))

        from urllib.parse import quote as q_
        b1 = cwrite("POST", "/api/boards", {"title": "Friday ten", "target_s": 600,
                                            "joke_ids": ["joke:b", "joke:c", "joke:x01", "joke:d", "joke:e"]})["board"]["id"]
        again("after a set list", ["/api/boards", "/api/tonight/screen"])
        b2 = cwrite("POST", "/api/boards", {"title": "Put away", "joke_ids": ["joke:a"], "target_s": 300})["board"]["id"]
        cwrite("PATCH", f"/api/boards/{q_(b2, safe='')}", {"archive": True})
        cwrite("PATCH", f"/api/boards/{q_(b1, safe='')}", {"move": {"from": 4, "to": 1}})
        again("after archive and move", ["/api/boards", "/api/boards?archived=1", "/api/tonight/screen"])
        cwrite("PATCH", "/api/settings", {"provider": "key", "training": True})
        again("after his settings", ["/api/settings"])
        cwrite("POST", "/api/mysets", {"title": "A tight five", "target_s": 300, "joke_ids": ["joke:a", "joke:c", "joke:e"]})
        cwrite("POST", "/api/mysets", {"title": "Too long", "target_s": 60, "joke_ids": ["joke:b", "joke:c", "joke:x01"]})
        cwrite("POST", "/api/mysets", {"title": "Nothing in it", "target_s": 300, "joke_ids": []})
        cwrite("POST", "/api/mysets", {"title": "One minute", "target_s": 60, "joke_ids": ["joke:a"]})
        cwrite("PATCH", "/api/mysets/ws:old", {"delete": True})
        again("after his sets", ["/api/mysets"])
        mine = [x["id"] for x in json.loads(call(py, "GET", "/api/mysets", app)[2])["sets"]]
        again("round 8, after the writes", [f"/api/boards/{q_(b1, safe='')}", f"/api/boards/{q_(b1, safe='')}/build",
                                            f"/api/boards/{q_(b2, safe='')}", f"/api/boards/{q_(b2, safe='')}/build",
                                            "/api/mysets/ws:old"]
              + [f"/api/mysets/{q_(m_, safe='')}" for m_ in mine]
              + [f"/api/mysets/{q_(m_, safe='')}/trim?target_s={t_}" for m_ in mine for t_ in (60, 300)])
        cwrite("POST", "/api/suggestions/not-a-joke/keep", {"joke_id": "joke:x09"})
        again("after a keep", ["/api/tonight/screen", "/api/tonight/screen?include_hidden=1",
                               "/api/suggestions/not-a-joke", "/api/suggestions/not-a-joke?include_hidden=1"])
        # round 8: his voice profile saved twice (the previous rides along), a draft dismissed, a job queued
        cwrite("PUT", "/api/profile/voice", {"text": "I talk fast and land on the last word."})
        cwrite("PUT", "/api/profile/voice", {"text": "I talk fast, then I wait.\nShort lines."})
        cwrite("POST", "/api/profile/voice/learn", {"dropped": [{"text": "A drafted line he passed on.", "bit": "joke:a"}]})
        job = cwrite("POST", "/api/riff", {"idea": "parking meters that talk back", "lines": ["a line of his"]})
        again("round 8, after his profile and a job", ["/api/profile/voice", f"/api/jobs/{q_(job['job']['id'], safe='')}",
                                                       "/api/workingsets", "/api/workingsets/ws:main"]
              + [f"/api/workingsets/{q_(m_, safe='')}" for m_ in mine]
              + [f"/api/workingsets/{q_(m_, safe='')}/nights" for m_ in mine[:2]])
        cwrite("POST", "/api/worker/claim", {"kinds": ["riff"], "worker": "t9-worker", "hosted": True},
               tok(realm_access={"roles": ["worker"]}))
        again("after a claim", ["/api/tonight/screen"])
        tn = json.loads(call(rs, "GET", "/api/tonight/screen", app)[2])
        if not tn["worker"]["online"] or (tn["board"] or {}).get("id") != b1 or not tn["to_sort"]:
            problems.append(f"coverage: tonight after the writes: worker {tn['worker']}, board "
                            f"{(tn['board'] or {}).get('id')}, to_sort {tn['to_sort']}")
        ms = json.loads(call(py, "GET", "/api/mysets", app)[2])["sets"]
        if len({s["health"].split(" ")[-1] for s in ms}) < 3 or len({s["status"] for s in ms}) < 3:
            problems.append(f"coverage: his sets show one health and one status only: {[(s['health'], s['status']) for s in ms]}")
        # the whole Bits-list row (AppIndex.summaries) before /api/jokes moves: the Python's page without the creative
        # layer's duration, against pk-api's diagnostics route (a build with --features diagnostics)
        for hid in ("", "&include_hidden=1"):
            label = "summaries" + hid
            s_py = call(py, "GET", "/api/jokes?limit=5000" + hid, app)
            s_rs = call(rs, "GET", "/api/_t9/summaries?x=1" + hid, app)
            if s_rs[0] != 200:
                problems.append(f"{label}: pk-api has no diagnostics route (build it with --features diagnostics)")
                continue
            items = json.loads(s_py[2])["items"]
            for x in items:
                x.pop("duration_s", None)
                x.pop("duration_estimated", None)
            # the Python's page has the attached note taken out (desk_filter) unless include_hidden; the raw rows have not
            rs_items = [x for x in json.loads(s_rs[2])["items"] if hid or x["id"] not in ("joke:x05", "joke:x10")]
            got = compare(label, (200, {}, json.dumps({"items": items}).encode()),
                          (200, {}, json.dumps({"items": rs_items}).encode()))
            problems += got
            n_ok += not got
            reqs.append((label, "GET", "", {}))
        report["requests"] = len(reqs)
        report["equal"] = n_ok
        # proof the answers came from pk-api and not through its fallback: its own counters saw every moved route
        mtext = call(rs, "GET", "/metrics")[2].decode()
        for route in ("/api/health", "/api/labels", "/api/decisions", "/api/meta", "/api/pipeline", "/api/learned",
                      "/api/map/cloud", "/api/sets", "/api/sets/{id}/analysis", "/api/sets/{id}/review", "/api/jokes/{id}",
                      "/api/jokes", "/api/today", "/api/search", "/api/jokes/{id}/similar", "/api/settings",
                      "/api/boards", "/api/mysets", "/api/tonight/screen", "/api/boards/{id}", "/api/boards/{id}/build",
                      "/api/mysets/{id}", "/api/mysets/{id}/trim", "/api/jokes/{id}/connections", "/api/jokes/{id}/map",
                      "/api/profile/voice", "/api/suggestions/not-a-joke", "/api/jobs/{id}", "/api/workingsets",
                      "/api/workingsets/{id}", "/api/workingsets/{id}/nights", "/api/workingsets/{id}/bits/{id}",
                      "/api/notebook/related", "/api/sets/{id}/clips"):
            if f'route="{route}",status="200"' not in mtext:
                problems.append(f"pk-api did not answer {route} itself (no 200 in its own /metrics)")
        # a forwarded request is counted by nobody here (the fallback adds nothing to what it forwards)
        def counted() -> float:
            return sum(float(ln.rsplit(" ", 1)[1]) for ln in call(rs, "GET", "/metrics")[2].decode().splitlines()
                       if ln.startswith("pk_http_requests_total{"))
        n0 = counted()
        call(rs, "GET", "/api/merge/candidates", app)
        if counted() != n0:
            problems.append("pk-api counted a forwarded request (/api/merge/candidates): the fallback must count nothing")
        # and the model was used here: pk-api's own search answered with vectors, not words only
        if not json.loads(call(rs, "GET", "/api/search?q=dog", app)[2]).get("vectors"):
            problems.append("pk-api searched on words only: the model did not load (PK_BGE_DIR)")

        # -- timing and memory
        if a.bench:
            # (path, signed in?): /metrics and the signed-out health split the HTTP, auth and database costs apart
            groups = {"metrics (no auth, no database)": ("/metrics", False), "health signed out (database)": ("/api/health", False),
                      "health": ("/api/health", True), "labels": ("/api/labels", True),
                      "decisions": ("/api/decisions?limit=100", True), "meta": ("/api/meta", True),
                      "pipeline": ("/api/pipeline", True), "learned": ("/api/learned", True),
                      "map/cloud (cached)": ("/api/map/cloud", True)}
            timing = {}
            for g, (p, signed) in groups.items():
                row_ = {}
                h = app if signed else {}
                for name, base in (("python", py), ("rust", rs)):
                    for _ in range(10):
                        call(base, "GET", p, h)
                    xs = []
                    for _ in range(a.bench):
                        t = time.perf_counter()
                        call(base, "GET", p, h)
                        xs.append((time.perf_counter() - t) * 1000)
                    xs.sort()
                    row_[name] = {"p50_ms": round(statistics.median(xs), 2), "p95_ms": round(xs[int(len(xs) * 0.95) - 1], 2)}
                timing[g] = row_
            report["timing"] = timing
            report["rss_mib"] = {"python": rss_mib(pp.pid), "rust": rss_mib(rp.pid)}

        # -- round 6: no model on either side (search_api.make_embedder finds none): words-only search, /similar over
        # the bit's first twelve words, the hybrid list on words; the same database
        for p_ in (rp, pp):
            p_.terminate()
            p_.wait(15)
        env.pop("PK_BGE_DIR", None)
        py2, rs2 = f"http://127.0.0.1:{free_port()}", f"http://127.0.0.1:{free_port()}"
        pp2 = start([sys.executable, str(ROOT / "api" / "entry.py")], {**auth_env, "PORT": py2.rsplit(":", 1)[1]}, "python-api-words")
        wait_http(py2 + "/api/health?ready=1", pp2, "python api (no model)")
        rp2 = start([str(a.pk_api)], {**auth_env, "PORT": rs2.rsplit(":", 1)[1], "PK_PYTHON_URL": py2,
                                      "RUST_LOG": os.environ.get("RUST_LOG", "info")}, "pk-api-words")
        wait_http(rs2 + "/api/health?ready=1", rp2, "pk-api (no model)")
        words = [("no model: " + p, "GET", p, app) for p in search + similar + hybrid]
        got, n_words = matrix(py2, rs2, words)
        problems += got
        for base, who in ((py2, "the Python"), (rs2, "pk-api")):
            if json.loads(call(base, "GET", "/api/search?q=dog", app)[2]).get("vectors") is not False:
                problems.append(f"no model: {who} still searched with vectors")
        likes = [json.loads(call(py2, "GET", p, app)[2]).get("like") or {} for p in similar]
        if not any(x_.get("bits") for x_ in likes) or not any(x_.get("moments") for x_ in likes):
            problems.append("coverage: no model: /similar over the first words found no other bit or no moment")
        report["requests"] += len(words)
        report["equal"] += n_words
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
    report["problems"] = problems
    out = json.dumps(report, indent=1)
    print(out)
    if a.report:
        a.report.write_text(out + "\n", encoding="utf-8", newline="\n")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
