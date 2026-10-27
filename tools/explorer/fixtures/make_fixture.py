"""Writes fixtures/graph.sample.json (+ fixture laughs) following docs/KB_SCHEMA.md.

Invented sample material only (not the comic's jokes). Re-run to regenerate:
    python tools/explorer/fixtures/make_fixture.py
"""
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def eid(src, kind, dst):
    return "e:" + hashlib.sha1(f"{src}|{kind}|{dst}".encode()).hexdigest()[:12]


RUN_BITS = "auto-20261025-2200-bits"
RUN_LINKS = "auto-20261025-2210-links"
RUN_MATCH = "auto-20261025-2220-match"

jokes = [
    ("joke:airport-belt", "Airport belt", "testing", "TSA agent holds my belt like evidence.", "2019-03-02", "2025-10-11"),
    ("joke:gps-voice", "GPS voice", "working", "My GPS sounds disappointed in me.", "2016-05-10", "2025-10-11"),
    ("joke:plant-dad", "Plant dad", "to_try", "I keep a cactus alive and call it parenting.", "2023-01-20", "2023-04-28"),
    ("joke:gym-mirror", "Gym mirror", "idea", "The gym mirror is the only one that lies in my favor.", "2024-08-01", "2024-08-01"),
    ("joke:dating-40", "Dating at 40", "testing", "Dating apps at 40 feel like a used-car lot.", "2022-02-14", "2025-10-11"),
    ("joke:old-phone", "Old phone", "retired", "My phone is so old it asks me to charge it emotionally.", "2015-06-26", "2017-10-10"),
]
units = [
    ("u-belt-1", "joke:airport-belt", "The TSA guy holds my belt up like it's evidence. Sir, that's not a weapon, that's a cry for help.", "setup+punch", "gdoc", "1AbCdocBelt", "Airport stuff REV 2", "2019-03-02"),
    ("u-belt-2", "joke:airport-belt", "He sniffed it. Twice.", "tag", "gdoc", "1AbCdocBelt", "Airport stuff REV 2", "2019-03-02"),
    ("u-belt-3", "joke:airport-belt", "Now I fly with suspenders. Nobody searches a man in suspenders. They just feel sorry for him.", "topper", "gdoc", "1AbCdocBelt2", "Belt new tags", "2024-11-09"),
    ("u-gps-1", "joke:gps-voice", "My GPS says 'recalculating' the way my mom says 'interesting choice'.", "setup+punch", "gdoc", "1GpsDoc", "GPS", "2016-05-10"),
    ("u-gps-2", "joke:gps-voice", "Recalculating. Recalculating. Okay she's not recalculating anymore, she's just sighing.", "tag", "gdoc", "1GpsDoc", "GPS", "2016-05-10"),
    ("u-plant-1", "joke:plant-dad", "I have a cactus. It needs water once a month and I still forget. That's my kid now.", "premise", "gdoc", "1PlantDoc", "Plant dad idea", "2023-01-20"),
    ("u-mirror-1", "joke:gym-mirror", "Gym mirrors are tilted. That's the only reason anyone goes back.", "note", "gdoc", "1GymDoc", "Gym notes", "2024-08-01"),
    ("u-dating-1", "joke:dating-40", "Dating at 40 is a used-car lot. Everybody's got miles, nobody's telling you about the transmission.", "setup+punch", "gdoc", "1DateDoc", "Dating 40", "2022-02-14"),
    ("u-dating-2", "joke:dating-40", "Her profile said 'no drama'. That's the drama. That's the whole show.", "tag", "gdoc", "1DateDoc", "Dating 40", "2022-02-14"),
    ("u-phone-1", "joke:old-phone", "My phone is so old it asks me to charge it emotionally.", "setup+punch", "gdoc", "1PhoneDoc", "Phone", "2015-06-26"),
]
sets = [
    ("set:20230428_AAAAAAAAAAA", "Open mic, Brooklyn", "2023-04-28", "Brooklyn open mic", 420),
    ("set:20251011_BBBBBBBBBBB", "Showcase, Manhattan", "2025-10-11", "Manhattan showcase", 900),
]
perfs = [
    ("set:20230428_AAAAAAAAAAA", 15000, 32000, "I have a cactus. It needs water once a month and I still forget. That's my kid now.", "joke:plant-dad", 0.81),
    ("set:20251011_BBBBBBBBBBB", 30000, 61000, "The TSA guy holds my belt up like evidence. He sniffed it. Twice.", "joke:airport-belt", 0.93),
    ("set:20251011_BBBBBBBBBBB", 90000, 118000, "My GPS says recalculating the way my mom says interesting choice.", "joke:gps-voice", 0.88),
    ("set:20251011_BBBBBBBBBBB", 200000, 240000, "Dating at 40 is a used car lot, everybody's got miles.", "joke:dating-40", 0.52),
]
labels = [
    ("device", "literal-interpretation", "literal interpretation", "Takes an expression at face value."),
    ("device", "specificity", "specificity", "A concrete detail beats a generic one."),
    ("device", "repetition", "repetition", "Repeats for rhythm or emphasis."),
    ("device", "analogy", "analogy", "Maps one world onto another."),
    ("structure", "tag-topper-chain", "tag / topper chain", "Stacks punches on one setup."),
    ("structure", "misdirect", "misdirect", "Leads one way, then pulls the rug."),
    ("form", "observational", "observational", ""),
    ("attitude", "hard", "hard", "Carter attitude: this is hard."),
    ("theory", "incongruity-resolution", "incongruity-resolution", "Setup builds one picture; the punch reveals a second that also fits."),
    ("topic", "travel", "travel", ""),
    ("topic", "dating", "dating", ""),
    ("persona", "plant-parent", "plant parent", ""),
]

nodes, edges = [], []
for jid, title, stage, summary, fs, ls in jokes:
    nodes.append({"id": jid, "type": "joke", "title": title, "stage": stage, "summary": summary, "first_seen": fs, "last_seen": ls})
for uid, jid, text, kind, src, doc, dtitle, date in units:
    nodes.append({"id": f"unit:{uid}", "type": "unit", "text": text, "kind": kind, "source": src, "doc_id": doc,
                  "doc_title": dtitle, "date": date, "locator": {"para_start": 0, "para_end": 0}})
for sid, title, date, venue, dur in sets:
    nodes.append({"id": sid, "type": "set", "title": title, "date": date, "venue": venue, "duration_s": dur})
for sid, s, e, text, _, _ in perfs:
    nodes.append({"id": f"perf:{sid[4:]}@{s}", "type": "performance", "set": sid, "start_ms": s, "end_ms": e, "text": text})
for kind, slug, name, d in labels:
    nodes.append({"id": f"label:{kind}:{slug}", "type": "label", "kind": kind, "name": name, "definition": d})


def edge(src, kind, dst, conf, reason, ev, run):
    edges.append({"id": eid(src, kind, dst), "src": src, "dst": dst, "kind": kind, "status": "proposed",
                  "confidence": conf, "reason": reason,
                  "evidence": [{"node": n, "quote": q, "start_ms": sm} for n, q, sm in ev], "source": f"auto:{run}"})


for uid, jid, text, *_ in units:
    weak = uid == "u-belt-3"
    edge(f"unit:{uid}", "part_of", jid, 0.62 if weak else 0.95,
         "Same belt premise five years later; might be a new bit." if weak else "This passage is the same bit.",
         [(f"unit:{uid}", text[:60], None)], RUN_BITS)
for sid, s, e, text, jid, conf in perfs:
    pid = f"perf:{sid[4:]}@{s}"
    edge(jid, "performed_as", pid, conf, "The transcript matches the written bit.", [(pid, text[:50], s)], RUN_MATCH)
edge("unit:u-belt-2", "tag_of", "unit:u-belt-1", 0.9, "An extra punch on the same setup.", [("unit:u-belt-2", "He sniffed it. Twice.", None)], RUN_LINKS)
edge("unit:u-gps-2", "tag_of", "unit:u-gps-1", 0.86, "Keeps hitting the 'recalculating' setup.", [("unit:u-gps-2", "Recalculating. Recalculating.", None)], RUN_LINKS)
edge("joke:gps-voice", "same_shape", "joke:dating-40", 0.48, "Both map a machine or market onto a judgmental relative.",
     [("unit:u-gps-1", "the way my mom says 'interesting choice'", None), ("unit:u-dating-1", "Dating at 40 is a used-car lot", None)], RUN_LINKS)
edge("joke:airport-belt", "same_shape", "joke:old-phone", 0.35, "Both treat an object as if it had feelings.", [("unit:u-phone-1", "charge it emotionally", None)], RUN_LINKS)
edge("joke:plant-dad", "same_premise", "joke:gym-mirror", 0.22, "Both about keeping up appearances at home.", [("unit:u-mirror-1", "Gym mirrors are tilted", None)], RUN_LINKS)
edge("joke:dating-40", "callback_to", "joke:old-phone", 0.41, "'Miles' could call back the old-phone battery line.", [("unit:u-dating-1", "Everybody's got miles", None)], RUN_LINKS)
edge("joke:gps-voice", "segue", "joke:airport-belt", 0.57, "Driving to flying is a natural segue.", [("perf:20251011_BBBBBBBBBBB@90000", "recalculating", 90000)], RUN_LINKS)
edge("joke:old-phone", "variant_of", "joke:gps-voice", 0.3, "Both are 'my device judges me' bits.", [("unit:u-phone-1", "asks me to charge it emotionally", None)], RUN_LINKS)


def lab(j, k, conf, why, q):
    edge(j, "has_label", k, conf, why, [(q[0], q[1], None)], RUN_LINKS)


lab("joke:airport-belt", "label:device:specificity", 0.84, "The belt held 'like evidence' is a concrete picture.", ("unit:u-belt-1", "holds my belt up like it's evidence"))
lab("joke:airport-belt", "label:device:repetition", 0.71, "'Twice.' lands as a one-word repeat.", ("unit:u-belt-2", "He sniffed it. Twice."))
lab("joke:airport-belt", "label:structure:tag-topper-chain", 0.66, "Belt, sniff, suspenders stack on one setup.", ("unit:u-belt-3", "Now I fly with suspenders."))
lab("joke:airport-belt", "label:topic:travel", 0.97, "It's about airport security.", ("unit:u-belt-1", "The TSA guy"))
lab("joke:gps-voice", "label:device:analogy", 0.78, "GPS mapped onto a disappointed mom.", ("unit:u-gps-1", "the way my mom says"))
lab("joke:gps-voice", "label:device:repetition", 0.55, "'Recalculating' repeated.", ("unit:u-gps-2", "Recalculating. Recalculating."))
lab("joke:gps-voice", "label:theory:incongruity-resolution", 0.5, "A machine word turns into a parent's judgement.", ("unit:u-gps-1", "interesting choice"))
lab("joke:dating-40", "label:attitude:hard", 0.62, "Dating at 40 is presented as hard.", ("unit:u-dating-1", "Dating at 40"))
lab("joke:dating-40", "label:topic:dating", 0.98, "It's about dating.", ("unit:u-dating-1", "Dating at 40"))
lab("joke:dating-40", "label:device:analogy", 0.74, "Dating mapped onto a used-car lot.", ("unit:u-dating-1", "a used-car lot"))
lab("joke:dating-40", "label:structure:misdirect", 0.45, "'No drama' turns into the drama.", ("unit:u-dating-2", "That's the drama."))
lab("joke:plant-dad", "label:persona:plant-parent", 0.58, "Talks about the plant as his kid.", ("unit:u-plant-1", "That's my kid now."))
lab("joke:old-phone", "label:device:literal-interpretation", 0.6, "Takes 'charge' literally and emotionally.", ("unit:u-phone-1", "charge it emotionally"))
lab("joke:gym-mirror", "label:form:observational", 0.69, "An everyday observation about gyms.", ("unit:u-mirror-1", "Gym mirrors are tilted."))
lab("unit:u-belt-2", "label:device:repetition", 0.8, "The tag is the repeat.", ("unit:u-belt-2", "Twice."))

graph = {"version": 0, "generated_at": "2026-10-25T22:30:00+00:00",
         "runs": [{"run_id": RUN_BITS, "pipeline": "bits", "model": "fixture", "notes": "fixture"},
                  {"run_id": RUN_LINKS, "pipeline": "links", "model": "fixture", "notes": "fixture"},
                  {"run_id": RUN_MATCH, "pipeline": "match", "model": "fixture", "notes": "fixture"}],
         "nodes": nodes, "edges": edges}
assert len({e["id"] for e in edges}) == len(edges)
(HERE / "graph.sample.json").write_text(json.dumps(graph, indent=1, ensure_ascii=False), encoding="utf-8", newline="\n")

laughs = {
    "20251011_BBBBBBBBBBB": [(41000, 43000, 0.7, 3, "laugh"), (52000, 53500, 0.3, 1, "laugh"), (60000, 64000, 0.95, 4, "applause"),
                             (101000, 102500, 0.5, 2, "laugh"), (230000, 231000, 0.2, 1, "laugh")],
}
for sid, ls in laughs.items():
    doc = {"run_id": "auto-20261025-2215-laughs", "detector": "fixture@0", "room_baseline": {"rms_db": -38.0},
           "laughs": [{"id": f"l{s}", "start_ms": s, "end_ms": e, "strength": st, "bars": b, "kind": k, "confidence": 0.8}
                      for s, e, st, b, k in ls]}
    p = HERE / "corpus" / "sets" / sid / "laughs_auto.json"
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(doc, indent=1), encoding="utf-8", newline="\n")
first = [eid("joke:dating-40", "has_label", "label:structure:misdirect"), eid("joke:gps-voice", "same_shape", "joke:dating-40")]
(HERE / "KB_REPORT.sample.md").write_text(
    "# KB report (fixture)\n\n## First 50 confirmations\nConfirm these first; each one teaches the most.\n\n"
    + "".join(f"{i}. `{x}`\n" for i, x in enumerate(first, 1))
    + "\n## Other notes\nIDs below this heading are not part of the list: `e:000000000000`\n",
    encoding="utf-8", newline="\n")
print(f"{len(nodes)} nodes, {len(edges)} edges")
