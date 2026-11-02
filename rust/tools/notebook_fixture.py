"""A synthetic notebook (invented bits, sets, labels, links, decisions, vectors) in the shape the hosted worker mirrors
from the API's `srcfile` rows: `kb/v1/graph.json`, `kb/v1/id_map.json`, `kb/v0/inferred.json`, the set files under
`corpus/sets/`, the bge vector parts under `vectors/`; plus the decision lines `GET /api/worker/decisions` serves.
Nothing here is anyone's material. Used by gen_corpus_goldens.py (the Corpus port) and gen_worker_goldens.py (the
worker differential for the notebook kinds).
"""
from __future__ import annotations

import json
import math

OWNER = "owner"

JOKES = [
    # id, title, lead unit text, extra
    ("joke:airport-bins", "Airport bins (security)", "So airport security wants my belt in a bin. The bin gets my belt, "
     "my shoes and most of my confidence. I walk through the scanner holding my pants up like a man surrendering to "
     "his own laundry.", {"stage": "working", "anatomy": {"reveal": "most of my confidence"}}),
    ("joke:parrot-lawyer", "Parrot lawyer", "I bought a parrot that only says legal disclaimers. Past results are no "
     "guarantee of future crackers. He read me my rights when I touched his perch.", {"stage": "polished"}),
    ("joke:landlord-chat", "Landlord group chat", "My landlord started a group chat for the building. Every message is "
     "a threat with a thumbs up. He reacted to my rent with the crying laughing face.", {"stage": "draft"}),
    ("joke:laundromat", "Laundromat dating", "The laundromat is the only place I meet women. We stare at our socks "
     "spinning like it is a fireplace. Nothing says romance like a dryer sheet stuck to my back.", {}),
    ("joke:gym-mirror", "Gym mirror", "At the gym I stare at the mirror the way my parrot does. Mostly to check if "
     "anyone saw me drop the small weights.", {"short_name": "Gym"}),
    ("joke:old-bins", "Old bins version", "Security took my belt.", {}),
    ("joke:tsa-poem", "TSA poem (written)", "Roses are red, the scanner is blue, the agent patted my personality "
     "through.", {}),
]
UNITS = [
    # unit, joke, text, date, part_of confidence
    ("unit:bins-a", "joke:airport-bins", JOKES[0][2], "2024-03-01", 0.95),
    ("unit:bins-b", "joke:airport-bins", "The bin gets my belt, my shoes and my dignity. Airports are a spa where "
     "strangers touch you without asking.", "2023-11-12", 0.9),
    ("unit:parrot-a", "joke:parrot-lawyer", JOKES[1][2], "2024-01-05", 0.92),
    ("unit:landlord-a", "joke:landlord-chat", JOKES[2][2], "2024-05-20", 0.88),
    ("unit:laundry-a", "joke:laundromat", JOKES[3][2], "2023-08-14", 0.8),
    ("unit:gym-a", "joke:gym-mirror", JOKES[4][2], "2024-06-02", 0.7),
    ("unit:gym-orphan", "joke:gym-mirror", "Short.", "2024-06-03", 0.2),
    ("unit:poem-a", "joke:tsa-poem", JOKES[6][2], "2024-02-02", 0.75),
    ("unit:hidden", "joke:laundromat", "A unit the comic deleted.", "2023-01-01", 0.99),
]
LABELS = [
    ("label:travel", "topic", "Travel"),
    ("label:pets", "topic", "Pets"),
    ("label:money", "topic", "Money"),
    ("label:landlord", "persona", "The landlord"),
    ("label:callback", "device", "Callback"),
    ("label:rule3", "structure", "Rule of three"),
]
HAS_LABEL = [
    # joke, label, confidence, status, evidence unit, quote
    ("joke:airport-bins", "label:travel", 0.9, "inferred", "unit:bins-a", "airport security"),
    ("joke:airport-bins", "label:rule3", 0.8, "inferred", "unit:bins-a", "my belt, my shoes and most of my confidence"),
    ("joke:tsa-poem", "label:travel", 0.6, "proposed", "unit:poem-a", "the scanner is blue"),
    ("joke:parrot-lawyer", "label:pets", 0.85, "inferred", "unit:parrot-a", "I bought a parrot"),
    ("joke:gym-mirror", "label:pets", 0.45, "proposed", "unit:gym-a", "the way my parrot does"),
    ("joke:landlord-chat", "label:landlord", 0.9, "confirmed", "unit:landlord-a", "My landlord started a group chat"),
    ("joke:laundromat", "label:money", 0.5, "inferred", "unit:laundry-a", "not in the text"),
    ("joke:parrot-lawyer", "label:callback", 0.7, "inferred", "unit:parrot-a", "He read me my rights"),
    ("joke:landlord-chat", "label:money", 0.55, "inferred", "unit:landlord-a", "my rent"),
]
LINKS = [
    # id, kind, src, dst, confidence, status, reason, evidence [(node, quote)]
    ("e:segue1", "segue", "joke:airport-bins", "joke:parrot-lawyer", 0.8, "inferred", "both about being searched",
     [("unit:bins-a", "holding my pants up"), ("unit:parrot-a", "I bought a parrot")]),
    ("e:prem1", "same_premise", "unit:gym-a", "joke:parrot-lawyer", 0.6, "proposed", "the parrot again",
     [("unit:gym-a", "the way my parrot does")]),
    ("e:call1", "callback_to", "joke:laundromat", "joke:landlord-chat", 0.5, "inferred", "the building",
     [("unit:laundry-a", "not verbatim")]),
]
SETS = [
    # dir, set key, date, venue (graph set node), venue_guess (meta), segments
    ("20240301_FakeSet0001", "2024-03-01_FakeSet0001", "The Corner Room Comedy Club, Springfield", None, [
        ("joke:airport-bins", "known", 0, 61000, "So airport security wants my belt in a bin. The bin gets my belt, "
         "my shoes and most of my confidence. Okay.", 5.2, 60.0, 1.4, "killed",
         [{"text": "most of my confidence", "bars": 3, "laugh_id": "L1"}]),
        (None, "banter", 61000, 70000, "How is everybody doing tonight, good, good.", 0, 9.0, None, None, []),
        ("joke:parrot-lawyer", "known", 70000, 110000, "I bought a parrot that only says legal disclaimers. Past "
         "results are no guarantee of future crackers.", 3.0, 40.0, 1.1, "worked",
         [{"text": "Past results are no guarantee of future crackers.", "bars": 2, "laugh_id": "L2"},
          {"text": "legal disclaimers", "bars": 1}]),
        ("joke:landlord-chat", "new", 110000, 150000, "My landlord started a group chat for the building. Every "
         "message is a threat with a thumbs up.", 0.4, 40.0, 0.2, "died", []),
    ]),
    ("20240412_FakeSet0002", None, None, "maybe The Loft", [
        ("joke:parrot-lawyer", "known", 0, 30000, "I got a parrot who only says legal disclaimers.", 1.0, 30.0, 0.6,
         "meh", [{"text": "legal disclaimers", "bars": 1, "laugh_id": "L9"}]),
        ("joke:parrot-lawyer", "known", 30000, 45000, "He read me my rights when I touched his perch.", 2.5, 15.0,
         None, "worked", [{"text": "He read me my rights", "bars": 3, "laugh_id": "L3"}]),
        ("joke:airport-bins", "known", 45000, 90000, "The bin gets my belt, my shoes and most of my confidence.",
         4.0, 45.0, 1.8, "killed", [{"text": "The bin gets my belt", "bars": 2, "laugh_id": "L4"}]),
        (None, "crowd_work", 90000, 100000, "Where are you from, sir?", 0, 10.0, None, None, []),
        ("joke:gym-mirror", "known", 100000, 130000, "At the gym I stare at the mirror the way my parrot does.",
         2.0, 30.0, 0.9, "worked", []),
    ]),
    ("20230814_FakeSet0003", "2023-08-14_FakeSet0003", "Unknown (title 'Laundry night')", None, [
        ("joke:laundromat", "known", 0, 50000, "The laundromat is the only place I meet women. We stare at our socks "
         "spinning like it is a fireplace.", 6.0, 50.0, 2.4, "killed",
         [{"text": "like it is a fireplace", "bars": 4, "laugh_id": "L5"}]),
        ("joke:airport-bins", "known", 50000, 95000, "Airports are a spa where strangers touch you without asking.",
         1.5, 45.0, 0.5, "meh", []),
        ("joke:parrot-lawyer", "known", 95000, 125000, "Past results are no guarantee of future crackers.", 3.5, 30.0,
         1.6, "worked", [{"text": "Past results are no guarantee of future crackers.", "bars": 3}]),
    ]),
]
LAUGHS = {"20240301_FakeSet0001": [{"id": "L1", "start_ms": 30000, "end_ms": 32150, "bars": 3},
                                   {"id": "L2", "start_ms": 100000, "end_ms": 101000}],
          "20240412_FakeSet0002": [{"id": "L3", "start_ms": 40000, "end_ms": 39000},
                                   {"id": "L4", "start_ms": 60000, "end_ms": 63500}]}


def decisions() -> list[dict]:
    """Decision lines as GET /api/worker/decisions serves them (oldest first)."""
    return [
        {"ts": "2024-07-01T10:00:00Z", "by": OWNER, "target": "node", "id": "joke:landlord-chat", "action": "edit",
         "after": {"title": "Landlord group chat (the building)", "stage": "working"}},
        {"ts": "2024-07-01T10:01:00Z", "by": "ai:labeler", "target": "node", "id": "joke:gym-mirror", "action": "edit",
         "after": {"title": "AI renamed it"}},
        {"ts": "2024-07-01T10:02:00Z", "by": OWNER, "auto": True, "target": "node", "id": "joke:gym-mirror",
         "action": "edit", "after": {"title": "auto renamed it"}},
        {"ts": "2024-07-02T09:00:00Z", "by": "import:app", "target": "node", "id": "joke:carousel-idea",
         "action": "add", "after": {"type": "joke", "title": "Baggage carousel", "summary": "The baggage carousel is a "
                                    "slot machine where every suitcase is black and none of them are mine.",
                                    "date": "2024-07-02"}},
        {"ts": "2024-07-02T09:05:00Z", "by": OWNER, "target": "edge", "id": "e:new-segue", "action": "add",
         "after": {"kind": "segue", "src": "joke:parrot-lawyer", "dst": "joke:gym-mirror",
                   "reason": "parrot to parrot", "evidence": [{"node": "unit:gym-a", "quote": "my parrot does"},
                                                              {"node": "unit:parrot-a", "quote": "touched his perch"}]}},
        {"ts": "2024-07-02T09:06:00Z", "by": OWNER, "target": "edge", "id": "e:call1", "action": "reject", "after": {}},
        {"ts": "2024-07-02T09:07:00Z", "by": OWNER, "target": "node", "id": "unit:hidden", "action": "delete",
         "after": {}},
        {"ts": "2024-07-02T09:08:00Z", "by": OWNER, "target": "node", "id": "perf:2024-03-01_FakeSet0001@110000",
         "action": "edit", "after": {"rating": "worked"}},
        {"ts": "2024-07-02T09:09:00Z", "by": OWNER, "target": "node", "id": "joke:old-v0-id", "action": "edit",
         "after": {"stage": "polished"}},
        {"ts": "2024-07-02T09:10:00Z", "by": OWNER, "target": "edge", "id": "e:segue1", "action": "confirm",
         "after": {"confidence": 0.99}},
    ]


def graph() -> dict:
    nodes = []
    for jid, title, _, extra in JOKES:
        nodes.append({"id": jid, "type": "joke", "title": title, "first_seen": "2023-05-01T00:00:00Z", **extra,
                      **({"lead_unit": "unit:bins-a"} if jid == "joke:airport-bins" else {})})
    nodes.append({"id": "joke:secret", "type": "joke", "title": "Hidden joke", "hidden": True})
    for uid, _, text, date, _ in UNITS:
        nodes.append({"id": uid, "type": "unit", "text": text, "date": date})
    for lid, kind, name in LABELS:
        nodes.append({"id": lid, "type": "label", "kind": kind, "name": name, "definition": f"About {name.lower()}."})
    for d, key, venue, _, _ in SETS:
        if key and venue:
            nodes.append({"id": f"set:{key}", "type": "set", "venue": venue, "title": f"Night {d[:8]}"})
    edges = []
    for uid, jid, _, _, conf in UNITS:
        edges.append({"id": f"e:part-{uid[5:]}", "kind": "part_of", "src": uid, "dst": jid, "confidence": conf,
                      "status": "inferred"})
    edges.append({"id": "e:part-bins-b-old", "kind": "part_of", "src": "unit:bins-b", "dst": "joke:tsa-poem",
                  "confidence": 0.3, "status": "proposed"})
    for k, (jid, lid, conf, status, node, quote) in enumerate(HAS_LABEL):
        edges.append({"id": f"e:lab{k}", "kind": "has_label", "src": jid, "dst": lid, "confidence": conf,
                      "status": status, "evidence": [{"node": node, "quote": quote}]})
    for eid, kind, src, dst, conf, status, reason, ev in LINKS:
        edges.append({"id": eid, "kind": kind, "src": src, "dst": dst, "confidence": conf, "status": status,
                      "reason": reason, "evidence": [{"node": n, "quote": q} for n, q in ev]})
    return {"nodes": nodes, "edges": edges}


def id_map() -> dict:
    return {"jokes": {"joke:old-v0-id": "joke:tsa-poem"}, "units": {"unit:old-bins": "unit:bins-b"}, "edges": {}}


def inferred() -> dict:
    return {"jokes": [{"id": "joke:old-v0-id", "stage": "draft"}, {"id": "joke:laundromat", "stage": "working"}]}


def analysis(d: str, key: str | None, segs: list) -> dict:
    jokes = []
    for i, (jid, status, a, b, text, laugh_s, window_s, vs_room, rating, pls) in enumerate(segs):
        jokes.append({"idx": i, "joke_id": jid, "status": status, "start_ms": a, "end_ms": b, "transcript": text,
                      "laughs": {"laugh_s": laugh_s, "window_s": window_s, "max_bars": max([p["bars"] for p in pls] or [0])},
                      "laugh_s_per_min": round(laugh_s / (window_s / 60), 2) if window_s else 0.0, "vs_room": vs_room,
                      "ai_rating": rating, "punchlines": pls, "time_to_first_laugh_s": 2.5 if pls else None,
                      "dead_air_max_s": 1.5 + i, "version_unit_id": None})
    out = {"jokes": jokes, "summary": {"laugh_s_per_min": 4.2 if d.endswith("1") else 3.1, "room_heat": 0.8}}
    if key:
        out["set"] = key
    return out


def vector(seed: str) -> list[float]:
    """A deterministic 384-d unit vector (no randomness), written the way pack() writes them (%.9g)."""
    xs = [math.sin((i + 1) * (sum(map(ord, seed)) % 97 + 1) * 0.37) + 0.3 * math.cos(i * len(seed)) for i in range(384)]
    n = math.sqrt(sum(x * x for x in xs))
    return [float(f"{x / n:.9g}") for x in xs]


MEMOS = [
    # key, transcript, joke it was filed as (None: a long memo kept as a note)
    ("memo-bins", "the bin got my belt and my shoes at airport security again", "joke:memo-bins"),
    ("memo-chat", "hey can you pick up milk on the way home and call the landlord", "joke:memo-chat"),
    ("memo-long", " ".join(["Thank you guys, give it up, here is the bit about the carousel and the luggage."] * 60), None),
]


def memo_decisions() -> list[dict]:
    """The memo ideas, filed as imports of his own memos."""
    return [{"ts": "2024-08-01T10:00:00Z", "by": "import:memo", "target": "node", "id": j, "action": "add",
             "after": {"type": "joke", "title": f"Memo {k[5:]}", "summary": text, "date": "2024-08-01"}}
            for k, text, j in MEMOS if j]


def memo_files() -> dict[str, str]:
    """kb/gsync/memos.json and each memo's transcript, plus a same-premise edge from the bins memo to the bins bit."""
    items = {k: {"status": "done", "transcript": f"corpus/memos/{k}/transcript.json", "joke_id": j,
                 "title": f"Memo {k[5:]}", "duration_s": 30 + 10 * n, "date": "2024-08-01"}
             for n, (k, _, j) in enumerate(MEMOS)}
    out = {"kb/gsync/memos.json": json.dumps({"items": items})}
    for k, text, _ in MEMOS:
        out[f"corpus/memos/{k}/transcript.json"] = json.dumps({"segments": [{"text": text}]})
    g = graph()
    g["edges"].append({"id": "e:memo-prem", "kind": "same_premise", "src": "joke:memo-bins", "dst": "joke:airport-bins",
                       "status": "inferred", "confidence": 0.8, "reason": "the bins again"})
    out["kb/v1/graph.json"] = json.dumps(g, indent=1)
    return out


def files() -> dict[str, str]:
    """path -> text, as the srcfile rows the API lists."""
    out = {"kb/v1/graph.json": json.dumps(graph(), indent=1), "kb/v1/id_map.json": json.dumps(id_map()),
           "kb/v0/inferred.json": json.dumps(inferred())}
    for d, key, _venue, guess, segs in SETS:
        out[f"corpus/sets/{d}/analysis.json"] = json.dumps(analysis(d, key, segs), indent=1)
        meta = {"title": f"Taped {d[:8]}"}
        if guess:
            meta["venue_guess"] = guess
        out[f"corpus/sets/{d}/meta.json"] = json.dumps(meta)
        if d in LAUGHS:
            out[f"corpus/sets/{d}/laughs_auto.json"] = json.dumps({"laughs": LAUGHS[d] + ["not a dict", {"no": "id"}]})
    ids = [u for u, *_ in UNITS] + ["reveal:bins-a"]
    out["vectors/vectors.part000.json"] = json.dumps({"cache": "vectors.f32", "ids_file": "vectors_ids.json", "dim": 384,
                                                      "ids": ids, "rows": [vector(i) for i in ids]},
                                                     separators=(",", ":"))
    out["vectors/extra_summary.json"] = json.dumps({"dim": 384, "model": "bge-small-en-v1.5",
                                                    "rows": {"summary:joke:carousel-idea": vector("carousel")},
                                                    "text_sha": {}}, separators=(",", ":"))
    return out
