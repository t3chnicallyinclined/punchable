"""Automatic inbox pipeline: tapes/inbox -> transcript, laughs, units, proposals.

    python -m engine.inbox                 # process everything in the inbox
    python -m engine.inbox --root TMP      # kb, inbox, processed, corpus output all under TMP

Per new file (audio or video; set or voice note, told apart by a sidecar
<file>.json {"kind": "set"|"voice_note", "date", "venue", "title"} or by length):
  1 normalize   2 transcribe   3 laughs (sets only)   4 split into units
  5 match to jokes/versions (bge-small candidates -> LLM judge: yes/no, confidence, reason)
  6 propose new jokes/ideas for unmatched funny units
  7 propose labels (devices, structures, topics: JOKE_SCIENCE vocabulary)
  8 propose links (segues from set order; same_premise / tag_of / callback_to for new jokes)

Rules: output is a new run merged into graph.json; decisions.jsonl is only read.
Rejected items are never re-proposed, decided items are kept, the comic's confirmed
decisions (never auto-confirmed ones) are the few-shot examples. Idempotent on
file sha256 + model/prompt versions. Every evidence quote is checked verbatim and
dropped if it fails. Finished files move to tapes/processed/<date>_<sha8>/; audio is
never deleted.
"""
from __future__ import annotations

import os
import argparse
import hashlib
import json
import re
import shutil
import sys
import time
import uuid
from datetime import datetime
from pathlib import Path

import numpy as np

from .config import Config
from .kbstore import KB, edge_id, now_iso
from .providers import Router, sha
from .verify import check_quotes, find_verbatim

COMIC = os.environ.get("PK_COMIC_NAME", "the comic")  # how the AI prompts name you; PK_COMIC_NAME
COMIC_FIRST = os.environ.get("PK_COMIC_FIRST", COMIC)

PROMPT_VERSION = "inbox-v1"
LLM_TASKS = ["split", "match_judge", "propose_joke", "label", "link_judge"]
MATCH_KINDS = {"bit", "idea", "joke"}
SPLIT_WINDOW = 220


def file_sha256(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def slugify(s, maxlen=48):
    s = (s or "").lower().replace("’", "'").replace("&", " and ")
    s = re.sub(r"[^a-z0-9]+", "-", s).strip("-")
    return (s[:maxlen].rstrip("-")) or "untitled"


def last_sentence(text, n=160):
    parts = [p.strip() for p in re.split(r"(?<=[.!?])\s+", text.strip()) if p.strip()]
    s = parts[-1] if parts else text.strip()
    return s if len(s) <= n else s[-n:].split(" ", 1)[-1]


def first_sentence(text, n=160):
    parts = [p.strip() for p in re.split(r"(?<=[.!?])\s+", text.strip()) if p.strip()]
    s = parts[0] if parts else text.strip()
    return s if len(s) <= n else s[:n].rsplit(" ", 1)[0]


def excerpt(text, n=200):
    t = text.strip()
    return t if len(t) <= n else t[:n].rsplit(" ", 1)[0]


# ---- schemas --------------------------------------------------------------
def _obj(props: dict, required=None):
    return {"type": "object", "properties": props, "required": required or list(props), "additionalProperties": False}


S_SPLIT = _obj({"units": {"type": "array", "items": _obj({
    "first": {"type": "integer"}, "last": {"type": "integer"},
    "kind": {"type": "string", "enum": ["bit", "idea", "crowd_work", "banter", "intro", "outro", "note"]},
    "name": {"type": "string"}, "funny": {"type": "boolean"}})}})
S_JUDGE = _obj({"judgments": {"type": "array", "items": _obj({
    "joke_id": {"type": "string"}, "same_joke": {"type": "boolean"}, "confidence": {"type": "number"},
    "reason": {"type": "string"}, "version_unit": {"type": "string"},
    "new_quote": {"type": "string"}, "old_quote": {"type": "string"}})}})
S_PROPOSE = _obj({"is_material": {"type": "boolean"}, "title": {"type": "string"}, "summary": {"type": "string"},
                  "quote": {"type": "string"}, "reason": {"type": "string"}})
S_LABEL = _obj({"labels": {"type": "array", "items": _obj({
    "label_id": {"type": "string"}, "quote": {"type": "string"}, "reason": {"type": "string"},
    "confidence": {"type": "number"}})}})
S_LINK = _obj({"links": {"type": "array", "items": _obj({
    "joke_id": {"type": "string"}, "kind": {"type": "string", "enum": ["same_premise", "tag_of", "callback_to", "none"]},
    "confidence": {"type": "number"}, "reason": {"type": "string"},
    "new_quote": {"type": "string"}, "old_quote": {"type": "string"}})}})

SYS_VOICE = ("You work on " + (f"stand-up comic {COMIC}'s" if COMIC != "the comic" else "a stand-up comic's")
             + " own material. Be literal and exact. "
             "Every quote you give must be copied character for character from the text it names; never paraphrase a quote. "
             "Reasons are one plain sentence a comic would understand.")


def fewshot_block(pos, neg, label=COMIC_FIRST + " confirmed"):
    if not pos and not neg:
        return "", None
    lines = []
    for r in pos:
        lines.append(f"- {label}: {r['src_title']} --{r['kind']}--> {r['dst_title']}"
                     + (f" | evidence: \"{r['evidence'][0]}\"" if r["evidence"] else "")
                     + (f" | his reason: {r['reason']}" if r.get("reason") else ""))
    for r in neg:
        lines.append(f"- {COMIC_FIRST} RULED OUT: {r['src_title']} --{r['kind']}--> {r['dst_title']}"
                     + (f" | evidence: \"{r['evidence'][0]}\"" if r["evidence"] else ""))
    block = "Examples of " + COMIC_FIRST + "'s own decisions (follow his judgment):\n" + "\n".join(lines)
    return block, sha(block)


class StageClock:
    def __init__(self, router: Router):
        self.router = router
        self.stages: dict[str, dict] = {}

    def run(self, name, fn, *a, **kw):
        n0 = len(self.router.calls)
        t0 = time.perf_counter()
        out = fn(*a, **kw)
        calls = [c for c in self.router.calls[n0:]]
        self.stages[name] = {"s": round(time.perf_counter() - t0, 2), "llm_calls": len(calls),
                             "tokens_in": sum(c["tokens_in"] for c in calls),
                             "tokens_out": sum(c["tokens_out"] for c in calls),
                             "failed_calls": sum(1 for c in calls if not c["ok"])}
        return out


class Inbox:
    def __init__(self, cfg: Config | None = None, router: Router | None = None, embedder=None, transcriber=None,
                 laughs=None, normalizer=None, prober=None, log=print):
        from . import audio
        self.cfg = cfg or Config()
        self.router = router or Router(self.cfg)
        if embedder is None:
            from .embed import Embedder
            embedder = Embedder(self.cfg.p("bge_dir"))
        self.embedder = embedder
        self.transcriber = transcriber or audio.Transcriber(self.cfg)
        self.laughs = laughs or audio.LaughDetector(self.cfg)
        self.normalizer = normalizer or audio.normalize
        from . import media
        self.prober = prober or media.probe
        self.log = log
        self.icfg = self.cfg.inbox
        self.ledger_path = self.cfg.p("kb_dir") / "inbox_ledger.jsonl"

    # ---- discovery / idempotency -------------------------------------------
    def files(self) -> list[Path]:
        inbox = self.cfg.p("inbox_dir")
        exts = set(self.icfg.get("audio_ext", []))
        return sorted(p for p in inbox.iterdir() if p.is_file() and p.suffix.lower() in exts) if inbox.exists() else []

    def versions(self) -> dict:
        return {"prompts": PROMPT_VERSION, "whisper": getattr(self.transcriber, "version", "?"),
                "laughs": getattr(self.laughs, "version", "?"), "embed": getattr(self.embedder, "name", "?"),
                "tasks": {t: self.router.describe(t) for t in LLM_TASKS}}

    def ledger(self) -> dict[str, dict]:
        out = {}
        if self.ledger_path.exists():
            for line in self.ledger_path.read_text(encoding="utf-8").splitlines():
                try:
                    d = json.loads(line)
                    out[d["key"]] = d
                except (ValueError, KeyError):
                    pass
        return out

    def run(self) -> list[dict]:
        return [self.process(p) for p in self.files()]

    def _sidecar(self, p: Path) -> tuple[dict, Path | None]:
        for c in (p.with_suffix(".json"), p.with_name(p.name + ".json")):
            if c.exists():
                try:
                    return json.loads(c.read_text(encoding="utf-8")), c
                except ValueError:
                    return {}, c
        return {}, None

    def _move_done(self, p: Path, side: Path | None, dest: Path) -> Path:
        dest.mkdir(parents=True, exist_ok=True)
        moved = None
        for f in [p] + ([side] if side else []):
            target = dest / f.name
            n = 1
            while target.exists():
                target = dest / f"{f.stem}.dup{n}{f.suffix}"
                n += 1
            shutil.move(str(f), str(target))  # a move, never a delete
            moved = moved or target
        return moved

    # ---- one file -------------------------------------------------------------
    def process(self, path: Path) -> dict:
        path = Path(path)
        self._dropped = 0
        file_sha = file_sha256(path)
        sha8 = file_sha[:8]
        side, side_path = self._sidecar(path)
        versions = self.versions()
        key = file_sha + "|" + sha(versions)
        meta = self.prober(path)
        if not isinstance(meta, dict):  # a bare duration (tests, plain audio probers)
            meta = {"duration_s": float(meta)}
        m = re.match(r"(\d{4}-\d{2}-\d{2})", path.name)
        date = side.get("date") or meta.get("date") or (m.group(1) if m else None) or \
            datetime.fromtimestamp(path.stat().st_mtime).strftime("%Y-%m-%d")
        meta["date"] = date
        meta["date_source"] = "sidecar" if side.get("date") else meta.get("date_source") or \
            ("filename" if m else "file modified time")
        name = f"{date}_{sha8}"
        done_dir = self.cfg.p("processed_dir") / name
        prior = self.ledger().get(key)
        if prior:
            moved = self._move_done(path, side_path, done_dir)
            self.log(f"{path.name}: already processed with these versions (run {prior['run_id']}); moved to {moved}")
            return {"file": path.name, "status": "skipped", "key": key, "run_id": prior["run_id"], "moved_to": str(moved)}

        run_id = f"auto-{datetime.now().strftime('%Y%m%d-%H%M%S')}-inbox-{sha8}-{uuid.uuid4().hex[:4]}"
        clock = StageClock(self.router)
        duration = float(meta.get("duration_s") or 0)
        origin = {"engine": "inbox", "sha256": file_sha, "run_id": run_id, "file": path.name}
        self.log(f"{path.name}: {duration:.0f}s{' video ' + str(meta.get('video_codec')) if meta.get('has_video') else ''}, "
                 f"run {run_id}")

        # work happens in processed/<name>/ (audio16k.wav + audio.opus, transcript, laughs); the original
        # file is moved there at the end. A video is never copied: only its audio is extracted.
        wav = done_dir / "audio16k.wav"
        clock.run("1_normalize", self.normalizer, path, wav)
        tx_path = done_dir / "transcript.json"
        tver = getattr(self.transcriber, "version", "?")
        cached = None
        if tx_path.exists():
            try:
                cached = json.loads(tx_path.read_text(encoding="utf-8"))
                if cached.get("engine_version") != tver:
                    cached = None
            except ValueError:
                cached = None
        if cached:
            transcript = clock.run("2_transcribe", lambda: cached)
            clock.stages["2_transcribe"]["cached"] = True
        else:
            transcript = clock.run("2_transcribe", self.transcriber, wav, done_dir)
            transcript["engine_version"] = tver
            tx_path.write_text(json.dumps(transcript, indent=1, ensure_ascii=False), encoding="utf-8")
        # set or voice note: the sidecar decides; else long = set; else short recordings are run through the
        # laugh detector and count as a clip of a set only if the room laughed
        set_min = float(self.icfg.get("set_min_seconds", 240))
        laughs = {"laughs": []}
        side_kind = side.get("kind")
        if side_kind in ("voice_note", "memo", "note"):
            kind, kind_basis = "voice_note", "sidecar"
        elif side_kind in ("set", "clip"):
            kind, kind_basis = side_kind, "sidecar"
        elif duration >= set_min:
            kind, kind_basis = "set", f"{duration:.0f}s >= {set_min:.0f}s"
        else:
            kind, kind_basis = None, None
        if kind in (None, "set", "clip"):
            laughs = clock.run("3_laughs", self.laughs, name, self.cfg.p("processed_dir"), self.cfg.p("processed_dir"),
                               run_id)
        else:
            clock.stages["3_laughs"] = {"s": 0, "skipped": "voice note"}
        if kind is None:
            n_l = len(laughs.get("laughs", []))
            kind = "clip" if n_l >= 1 else "voice_note"
            kind_basis = f"{duration:.0f}s < {set_min:.0f}s and {n_l} crowd laugh(s)"
            if kind == "voice_note":
                laughs = {"laughs": []}
        is_set = kind in ("set", "clip")
        meta.update({"kind": kind, "kind_basis": kind_basis, "source_file": path.name, "sha256": file_sha})
        corpus_out = self.cfg.p("out_corpus_dir") / ("sets" if is_set else "memos") / name
        corpus_out.mkdir(parents=True, exist_ok=True)
        for f in ("transcript.json", "transcript.md") + (("laughs_auto.json",) if is_set else ()):
            if (done_dir / f).exists():
                shutil.copyfile(done_dir / f, corpus_out / f)
        show = self.place_and_group(meta, side, name, is_set)
        (corpus_out / "meta.json").write_text(json.dumps(meta, indent=1, ensure_ascii=False), encoding="utf-8")

        kb = KB(self.cfg)
        units = clock.run("4_split", self.split, transcript, laughs, is_set, run_id, name)
        nodes, edges = [], []
        matches = clock.run("5_match", self.match, kb, units, is_set, run_id)
        units, matches = self.merge_adjacent(units, matches, laughs, is_set, name)
        texts = {u["id"]: u["text"] for u in units}
        self.match_edges(kb, units, matches, is_set, run_id, origin, texts, edges)
        set_id = f"set:{name}"
        nodes.append({"id": set_id, "type": "set", "title": side.get("title") or path.stem, "date": date,
                      "venue": meta.get("venue", ""), "venue_status": meta.get("venue_status"),
                      "venue_basis": meta.get("venue_basis"), "duration_s": int(round(duration)), "recording": kind,
                      "kind_basis": kind_basis, "recorded_utc": meta.get("created_utc"), "gps": meta.get("gps"),
                      "device": meta.get("device"), "show": meta.get("show"), "segment": meta.get("segment"),
                      "source_file": path.name, "origin": origin})
        if show:
            nodes.append(show["node"])
            edges.extend(show["edges"])
        for u in units:
            if is_set:
                nodes.append({"id": u["id"], "type": "performance", "set": set_id, "start_ms": u["start_ms"],
                              "end_ms": u["end_ms"], "text": u["text"], "kind": u["kind"], "name": u["name"],
                              "laughs": u["laughs"], "origin": origin})
            else:
                nodes.append({"id": u["id"], "type": "unit", "text": u["text"], "kind": u["kind"], "source": "memo",
                              "doc_title": u["name"], "date": date,
                              "locator": {"recording": set_id, "start_ms": u["start_ms"], "end_ms": u["end_ms"]},
                              "origin": origin})
        new_jokes = clock.run("6_propose", self.propose, kb, units, matches, is_set, run_id, origin, texts, date,
                              nodes, edges)
        clock.run("7_label", self.label, kb, units, matches, new_jokes, run_id, origin, texts, edges)
        clock.run("8_link", self.link, kb, units, matches, new_jokes, is_set, run_id, origin, texts, side, date,
                  path.stem, nodes, edges)

        # a final verbatim gate on every evidence quote of every edge
        all_texts = dict(texts)
        for e in edges:
            for ev in e.get("evidence", []):
                n = kb.view.nodes.get(ev["node"])
                if n and ev["node"] not in all_texts:
                    all_texts[ev["node"]] = n.get("text") or ""
        gated, dropped_total = [], 0
        for e in edges:
            kept, dropped = check_quotes(e.get("evidence", []), all_texts)
            dropped_total += len(dropped)
            if kept or not e.get("evidence"):
                e["evidence"] = kept
                gated.append(e)
        edges = gated

        run = {"run_id": run_id, "pipeline": "inbox", "model": json.dumps(versions["tasks"]),
               "notes": f"{path.name} ({kind}, {duration:.0f}s); sha256 {file_sha}; versions {sha(versions)}",
               "source_sha256": file_sha, "versions": versions}
        t0 = time.perf_counter()
        stats = kb.merge_run(run, nodes, edges, file_sha)
        clock.stages["9_merge"] = {"s": round(time.perf_counter() - t0, 2)}
        stats.update({"units": len(units), "matched": sum(1 for v in matches.values() if v),
                      "new_jokes": len(new_jokes), "quotes_dropped_final_gate": dropped_total,
                      "quotes_dropped_in_stages": getattr(self, "_dropped", 0)})
        moved = self._move_done(path, side_path, done_dir)
        report = {"file": path.name, "status": "done", "key": key, "run_id": run_id, "kind": kind, "name": name,
                  "duration_s": round(duration, 1), "stats": stats, "stages": clock.stages,
                  "tokens_in": sum(s.get("tokens_in", 0) for s in clock.stages.values()),
                  "tokens_out": sum(s.get("tokens_out", 0) for s in clock.stages.values()),
                  "moved_to": str(moved), "versions": versions}
        (done_dir / "run.json").write_text(json.dumps(report, indent=1), encoding="utf-8")
        self.ledger_path.parent.mkdir(parents=True, exist_ok=True)
        with open(self.ledger_path, "a", encoding="utf-8") as f:
            f.write(json.dumps({"key": key, "sha256": file_sha, "file": path.name, "name": name, "run_id": run_id,
                                "ts": now_iso(), "versions": versions, "stats": stats}) + "\n")
        self._dropped = 0
        return report

    # ---- venue + show grouping -------------------------------------------------
    def place_and_group(self, meta, side, name, is_set):
        """Venue: the sidecar's, else inferred from GPS against earlier set metas. Clips of one night and
        place (3 h, 200 m) become segments of one show node."""
        from . import media
        roots = [self.cfg.p("out_corpus_dir") / "sets", self.cfg.p("corpus_dir") / "sets"]
        if side.get("venue"):
            meta.update({"venue": side["venue"], "venue_status": "given", "venue_basis": "sidecar"})
        else:
            meta.update(media.infer_venue(meta.get("gps"), media.known_venues(roots)))
        if not is_set:
            return None
        members = [dict(meta, name=name)]
        for mp in sorted((self.cfg.p("out_corpus_dir") / "sets").glob("*/meta.json")):
            if mp.parent.name == name:
                continue
            try:
                other = json.loads(mp.read_text(encoding="utf-8"))
            except ValueError:
                continue
            if media.same_show(meta, other):
                members.append(dict(other, name=mp.parent.name, _path=str(mp)))
        if len(members) < 2:
            return None
        members.sort(key=lambda x: x.get("created_utc") or "")
        show_id = "show:" + (next((x["show"].split(":", 1)[1] for x in members if x.get("show")), None)
                             or members[0]["name"])
        seg_ids = []
        for i, x in enumerate(members):
            seg_ids.append(f"set:{x['name']}")
            if x["name"] == name:
                meta["show"], meta["segment"] = show_id, i + 1
            elif x.get("_path"):
                x2 = {k: v for k, v in x.items() if k not in ("_path", "name")}
                x2["show"], x2["segment"] = show_id, i + 1
                Path(x["_path"]).write_text(json.dumps(x2, indent=1, ensure_ascii=False), encoding="utf-8")
        first = members[0]
        node = {"id": show_id, "type": "show", "title": f"Show {first.get('date')} ({meta.get('venue') or 'unknown venue'})",
                "date": first.get("date"), "venue": meta.get("venue"), "venue_status": meta.get("venue_status"),
                "gps": first.get("gps"), "start_utc": first.get("created_utc"), "segments": seg_ids,
                "origin": {"engine": "inbox-show"}}
        edges = []
        for i, sid in enumerate(seg_ids):
            edges.append({"id": edge_id(sid, "part_of", show_id), "src": sid, "dst": show_id, "kind": "part_of",
                          "status": "proposed", "confidence": 0.8, "segment": i + 1,
                          "reason": "Recorded within 3 hours and 200 m of the other clips of this show.",
                          "evidence": [], "source": "auto:inbox-show", "origin": {"engine": "inbox-show"}})
        return {"node": node, "edges": edges}

    # ---- 4 split ----------------------------------------------------------------
    @staticmethod
    def lines(transcript):
        """Sentence-sized lines from word timestamps (whisper segments often straddle two bits)."""
        out = []
        for s in transcript.get("segments", []):
            words = [w for w in s.get("words") or [] if (w.get("w") or "").strip()]
            if not words:
                if (s.get("text") or "").strip():
                    out.append({"start": s["start"], "end": s["end"], "text": s["text"].strip()})
                continue
            cur = []
            for i, w in enumerate(words):
                cur.append(w)
                nxt = words[i + 1] if i + 1 < len(words) else None
                if nxt is None or w["w"].rstrip("\"')").endswith((".", "?", "!")) or len(cur) >= 30 \
                        or (nxt["start"] - w["end"]) > 0.8:
                    out.append({"start": cur[0]["start"], "end": cur[-1]["end"],
                                "text": " ".join(x["w"].strip() for x in cur)})
                    cur = []
        return out

    def split(self, transcript, laughs, is_set, run_id, name):
        segs = self.lines(transcript)
        units = []
        for w0 in range(0, len(segs), SPLIT_WINDOW):
            win = segs[w0:w0 + SPLIT_WINDOW]
            listing = "\n".join(f"[{w0 + i}] ({int(s['start'] // 60):02d}:{int(s['start'] % 60):02d}) {s['text'].strip()}"
                                for i, s in enumerate(win))
            what = ("a recorded stand-up SET. Units are: bit (a joke or bit he performs), crowd_work, banter "
                    "(filler, transitions), intro (host/walk-on), outro") if is_set else \
                ("a VOICE NOTE where the comic records joke ideas. Units are: idea (one joke idea or premise), "
                 "bit (a worked joke), note (logistics, reminders, not material)")
            msgs = [{"role": "system", "content": SYS_VOICE},
                    {"role": "user", "content":
                        f"This is the transcript of {what}, one numbered line each.\n"
                        f"Split it into consecutive units. Each unit covers lines first..last (inclusive) and units "
                        f"must not overlap. A bit is ONE unit including its tags, act-outs and asides; start a new unit "
                        f"only when he moves to a different premise (watch for the turn: 'anyway', 'so', a new subject). "
                        f"intro = only the host introducing him / walk-on applause: his own first lines are material, "
                        f"even quick openers. outro = only his sign-off ('that's my time', socials). "
                        f"name = a short title for the unit (3-7 words). funny = true if it is comedic material.\n\n"
                        f"{listing}"}]
            c = self.router.complete("split", msgs, S_SPLIT, run_id=run_id)
            spans = sorted(c.data["units"], key=lambda u: u["first"])
            last_end = w0 - 1
            for u in spans:
                a, b = max(int(u["first"]), last_end + 1, w0), min(int(u["last"]), w0 + len(win) - 1)
                if b < a:
                    continue
                last_end = b
                if u["kind"] in ("intro", "outro"):
                    continue
                piece = segs[a:b + 1]
                text = " ".join(s["text"].strip() for s in piece)
                start_ms, end_ms = int(piece[0]["start"] * 1000), int(piece[-1]["end"] * 1000)
                ls = [l for l in laughs.get("laughs", []) if start_ms <= l.get("start_ms", -1) <= end_ms + 3000]
                kind = u["kind"]
                if not is_set and kind == "bit":
                    kind = "joke"
                uid = f"perf:{name}@{start_ms}" if is_set else f"unit:{name}@{start_ms}"
                units.append({"id": uid, "start_ms": start_ms, "end_ms": end_ms, "text": text, "kind": kind,
                              "name": u["name"].strip()[:80], "funny": bool(u["funny"]),
                              "laughs": {"hits": len(ls), "max_bars": max((l.get("bars", 0) for l in ls), default=0)},
                              "_segs": piece})
        return units

    @staticmethod
    def quote_ms(unit, quote):
        """Start time of a quote inside a unit (segment-level)."""
        k = unit["text"].find(quote)
        if k < 0:
            return None
        pos = 0
        for s in unit["_segs"]:
            pos += len(s["text"].strip()) + 1
            if k < pos:
                return int(s["start"] * 1000)
        return unit["start_ms"]

    # ---- 5 match --------------------------------------------------------------------
    def _candidate_pool(self, kb: KB):
        from .embed import VectorIndex
        if not hasattr(self, "_vindex"):
            self._vindex = VectorIndex(self.cfg.p("vectors_dir"), self.cfg.p("kb_dir") / "engine_cache", self.embedder)
        vi = self._vindex
        v = kb.view
        rows, owners, kinds = [], [], []
        mats = []
        unit_keys = [u for u in v.unit_joke if v.unit_joke[u] not in kb.rejected_nodes]
        texts = {}
        for u in unit_keys:
            text = (v.nodes.get(u) or {}).get("text") or ""
            if u in vi.row:
                mats.append(vi.m[vi.row[u]])
            else:
                texts[u] = text[:2000]
                mats.append(None)
            rows.append(u)
            owners.append(v.unit_joke[u])
            kinds.append("unit")
            rk = "reveal:" + u.split(":", 1)[1]
            if rk in vi.row:
                rows.append(u)
                owners.append(v.unit_joke[u])
                kinds.append("reveal")
                mats.append(vi.m[vi.row[rk]])
        for eid, e in v.edges.items():
            if e.get("kind") == "performed_as" and not kb.is_rejected_edge(e) and e.get("dst") in v.nodes \
                    and e["src"] in v.nodes:
                texts[e["src"]] = (v.nodes[e["src"]].get("text") or "")[:2000]
                rows.append(e["src"])
                owners.append(e["dst"])
                kinds.append("perf")
                mats.append(None)
        got = vi.ensure(texts) if texts else {}
        M = np.stack([m if m is not None else got[r] for m, r in zip(mats, rows)]) if rows else np.zeros((0, 384))
        return rows, owners, kinds, M

    def candidates(self, kb, unit, pool, k):
        rows, owners, kinds, M = pool
        if not rows:
            return []
        q = self.embedder.embed([unit["text"][:2000], unit["text"][-220:]])
        s_text = M @ q[0]
        s_rev = M @ q[1]
        score = np.where(np.array(kinds) == "reveal", s_rev, s_text)
        best: dict[str, list] = {}
        for i in np.argsort(-score)[:400]:
            best.setdefault(owners[i], []).append((float(score[i]), rows[i]))
        ranked = sorted(best.items(), key=lambda kv: -kv[1][0][0])[:k]
        out = []
        for jid, hits in ranked:
            seen, units = set(), []
            for s, r in hits:
                if r not in seen:
                    seen.add(r)
                    units.append(r)
                if len(units) >= 2:
                    break
            out.append({"joke_id": jid, "score": round(hits[0][0], 3), "units": units})
        return out

    def match(self, kb: KB, units, is_set, run_id):
        self._pool = self._candidate_pool(kb)
        pos, neg = kb.examples({"performed_as", "part_of", "variant_of"})
        fs, fs_sha = fewshot_block(pos, neg)
        k = int(self.icfg.get("match_candidates", 5))
        min_conf = float(self.icfg.get("match_min_confidence", 0.5))
        out = {}
        self._cands = {}
        for u in units:
            if u["kind"] not in MATCH_KINDS:
                out[u["id"]] = None
                continue
            cands = self.candidates(kb, u, self._pool, k)
            self._cands[u["id"]] = cands
            if not cands:
                out[u["id"]] = None
                continue
            blocks = []
            for c in cands:
                j = kb.view.nodes.get(c["joke_id"], {})
                ub = "\n".join(f"    [{x}] {excerpt((kb.view.nodes.get(x) or {}).get('text', ''), 700)}" for x in c["units"])
                blocks.append(f"- joke_id {c['joke_id']}: \"{j.get('title', '')}\" ({j.get('first_seen', '?')}..{j.get('last_seen', '?')})"
                              f"\n  summary: {j.get('summary', '')}\n  closest written/performed versions:\n{ub}")
            msgs = [{"role": "system", "content": SYS_VOICE + ("\n\n" + fs if fs else "")},
                    {"role": "user", "content":
                        f"NEW {'performance (live transcript)' if is_set else 'voice-note unit'}:\n\"\"\"{u['text']}\"\"\"\n\n"
                        f"CANDIDATE existing jokes:\n" + "\n".join(blocks) + "\n\n"
                        "For EACH candidate decide: is the new text the SAME joke (same premise and punch, possibly reworded, "
                        "shortened or extended)? Topic overlap alone is NOT the same joke. confidence 0..1. "
                        "version_unit = the id in brackets of the closest version, or \"\". "
                        "new_quote = a short exact line from the NEW text that shows it; old_quote = an exact line from "
                        "that version. Copy quotes exactly."}]
            c = self.router.complete("match_judge", msgs, S_JUDGE, run_id=run_id, fewshot_sha=fs_sha)
            valid = {x["joke_id"]: x for x in cands}
            js = [j for j in c.data["judgments"] if j["joke_id"] in valid and j["same_joke"]
                  and float(j["confidence"]) >= min_conf]
            if not js:
                out[u["id"]] = None
                continue
            j = max(js, key=lambda j: float(j["confidence"]))
            ver = j["version_unit"] if j["version_unit"] in valid[j["joke_id"]]["units"] else valid[j["joke_id"]]["units"][0]
            out[u["id"]] = {"joke_id": j["joke_id"], "version": ver, "confidence": float(j["confidence"]),
                            "reason": j["reason"].strip(), "new_quote": j["new_quote"], "old_quote": j["old_quote"]}
        return out

    def merge_adjacent(self, units, matches, laughs, is_set, name):
        """Neighbouring units matched to the same joke are one performance of it (one bit split in two)."""
        out, mout = [], {}
        for u in units:
            m = matches.get(u["id"])
            prev = out[-1] if out else None
            pm = mout.get(prev["id"]) if prev else None
            if prev and m and pm and m["joke_id"] == pm["joke_id"]:
                prev["text"] = prev["text"] + " " + u["text"]
                prev["end_ms"] = u["end_ms"]
                prev["_segs"] = prev["_segs"] + u["_segs"]
                ls = [l for l in laughs.get("laughs", []) if prev["start_ms"] <= l.get("start_ms", -1) <= prev["end_ms"] + 3000]
                prev["laughs"] = {"hits": len(ls), "max_bars": max((l.get("bars", 0) for l in ls), default=0)}
                prev["merged"] = prev.get("merged", 1) + 1
                if m["confidence"] > pm["confidence"]:
                    mout[prev["id"]] = dict(m)
                self._cands[prev["id"]] = self._cands.get(prev["id"]) or self._cands.get(u["id"])
                continue
            out.append(u)
            mout[u["id"]] = m
        return out, mout

    def match_edges(self, kb, units, matches, is_set, run_id, origin, texts, edges):
        kind = "performed_as" if is_set else "part_of"
        for u in units:
            m = matches.get(u["id"])
            if not m:
                continue
            evidence = self._evidence(kb, u, m["new_quote"], m["version"], m["old_quote"], texts)
            edges.append({"id": edge_id(u["id"], kind, m["joke_id"]), "src": u["id"], "dst": m["joke_id"],
                          "kind": kind, "status": "proposed",
                          "confidence": round(min(1.0, max(0.0, m["confidence"])), 2), "reason": m["reason"],
                          "version": m["version"], "evidence": evidence, "source": f"auto:{run_id}", "origin": origin})

    def _evidence(self, kb, u, new_q, other_node, other_q, texts):
        srcs = dict(texts)
        if other_node:
            srcs[other_node] = (kb.view.nodes.get(other_node) or {}).get("text", "")
        items = [{"node": u["id"], "quote": new_q}]
        if other_node and other_q:
            items.append({"node": other_node, "quote": other_q})
        kept, dropped = check_quotes(items, srcs)
        self._dropped = getattr(self, "_dropped", 0) + len(dropped)
        if not any(k["node"] == u["id"] for k in kept):
            kept.insert(0, {"node": u["id"], "quote": excerpt(u["text"], 160), "quote_by": "engine"})
        for k in kept:
            k["start_ms"] = self.quote_ms(u, k["quote"]) if k["node"] == u["id"] and "_segs" in u else None
        return kept

    # ---- 6 propose ------------------------------------------------------------------
    def propose(self, kb, units, matches, is_set, run_id, origin, texts, date, nodes, edges):
        new = {}
        existing = set(kb.view.nodes)
        for u in units:
            if u["kind"] not in MATCH_KINDS or matches.get(u["id"]):
                continue
            if is_set and not (u["funny"] or u["laughs"]["hits"]):
                continue
            if u["id"] in kb.rejected_origins:
                continue
            laugh = f"Room reaction: {u['laughs']['hits']} laugh(s), loudest {u['laughs']['max_bars']}/4 bars.\n" if is_set else ""
            msgs = [{"role": "system", "content": SYS_VOICE},
                    {"role": "user", "content":
                        f"This {'was performed live' if is_set else 'is from a voice note'} and matches none of his existing jokes.\n"
                        f"{laugh}\"\"\"{u['text']}\"\"\"\n\n"
                        "Is it comedy material worth tracking as its own joke or idea (not filler, not crowd work)? "
                        "If so give a short title (3-6 words), a one-sentence summary of the premise and punch, and "
                        "quote = the funniest line, copied exactly."}]
            c = self.router.complete("propose_joke", msgs, S_PROPOSE, run_id=run_id)
            d = c.data
            if not d["is_material"]:
                continue
            jid = "joke:" + slugify(d["title"])
            if jid in existing or jid in new.values():
                jid += "-" + hashlib.sha1(u["id"].encode()).hexdigest()[:6]
            if jid in kb.rejected_nodes:
                continue
            nodes.append({"id": jid, "type": "joke", "title": d["title"].strip()[:80], "summary": d["summary"].strip(),
                          "stage": "testing" if is_set else "idea", "first_seen": date, "last_seen": date,
                          "stage_reason": f"Proposed by the inbox from {origin['file']}: {'performed once' if is_set else 'voice-note idea'}; confirm or reject.",
                          "performances": 1 if is_set else 0, "unit_count": 0 if is_set else 1,
                          "proposed": True, "origin": origin, "origin_span": u["id"]})
            kind = "performed_as" if is_set else "part_of"
            ev = self._evidence(kb, u, d["quote"], None, None, texts)
            edges.append({"id": edge_id(u["id"], kind, jid), "src": u["id"], "dst": jid, "kind": kind,
                          "status": "proposed", "confidence": 0.6, "reason": d["reason"].strip() or "New material.",
                          "evidence": ev, "source": f"auto:{run_id}", "origin": origin})
            new[u["id"]] = jid
        return new

    # ---- 7 label --------------------------------------------------------------------
    def label(self, kb, units, matches, new_jokes, run_id, origin, texts, edges):
        vocab = kb.label_vocab()
        if not vocab:
            return
        pos, neg = kb.examples({"has_label"}, dst_prefix="label:")
        fs, fs_sha = fewshot_block(pos, neg)
        dev = "\n".join(f"  {k}: {v['name']} - {v['definition']}" for k, v in vocab.items() if v["kind"] in ("device", "structure"))
        top = ", ".join(k for k, v in vocab.items() if v["kind"] in ("topic", "custom"))
        for u in units:
            if u["kind"] not in MATCH_KINDS or not (matches.get(u["id"]) or u["id"] in new_jokes):
                continue
            target = new_jokes.get(u["id"]) or u["id"]
            msgs = [{"role": "system", "content": SYS_VOICE + ("\n\n" + fs if fs else "")},
                    {"role": "user", "content":
                        f"Label this joke with the comedy devices, structures and topics it really uses.\n"
                        f"\"\"\"{u['text']}\"\"\"\n\nDEVICES AND STRUCTURES (id: name - definition):\n{dev}\n\n"
                        f"TOPICS (ids): {top}\n\n"
                        "Give 2-6 labels, only ids from these lists. For each, quote = the exact line that shows it, "
                        "reason = one sentence, confidence 0..1. Prefer fewer, surer labels."}]
            c = self.router.complete("label", msgs, S_LABEL, run_id=run_id, fewshot_sha=fs_sha)
            seen = set()
            for l in c.data["labels"][:8]:
                lid = l["label_id"].strip()
                if lid not in vocab or lid in seen:
                    continue
                seen.add(lid)
                q = find_verbatim(l["quote"], u["text"])
                if q is None:
                    self._dropped = getattr(self, "_dropped", 0) + 1
                    continue
                edges.append({"id": edge_id(target, "has_label", lid), "src": target, "dst": lid, "kind": "has_label",
                              "status": "proposed", "confidence": round(min(1.0, max(0.0, float(l["confidence"]))), 2),
                              "reason": l["reason"].strip(),
                              "evidence": [{"node": u["id"], "quote": q, "start_ms": self.quote_ms(u, q)}],
                              "source": f"auto:{run_id}", "origin": origin})

    # ---- 8 link ---------------------------------------------------------------------
    def link(self, kb, units, matches, new_jokes, is_set, run_id, origin, texts, side, date, stem, nodes, edges):
        def joke_of(u):
            m = matches.get(u["id"])
            return (m or {}).get("joke_id") or new_jokes.get(u["id"])

        if is_set:  # segues: consecutive bits in the order he did them
            bits = [u for u in units if joke_of(u)]
            where = side.get("venue") or side.get("title") or stem
            for a, b in zip(bits, bits[1:]):
                ja, jb = joke_of(a), joke_of(b)
                if ja == jb:
                    continue
                qa, qb = last_sentence(a["text"]), first_sentence(b["text"])
                edges.append({"id": edge_id(ja, "segue", jb), "src": ja, "dst": jb, "kind": "segue",
                              "status": "proposed", "confidence": 0.5,
                              "reason": f"Performed back to back in {where} ({date}).",
                              "evidence": [{"node": a["id"], "quote": qa, "start_ms": self.quote_ms(a, qa)},
                                           {"node": b["id"], "quote": qb, "start_ms": self.quote_ms(b, qb)}],
                              "source": f"auto:{run_id}", "origin": origin})
        pos, neg = kb.examples({"same_premise", "tag_of", "callback_to"})
        fs, fs_sha = fewshot_block(pos, neg)
        k = int(self.icfg.get("link_candidates", 3))
        for u in units:
            jid = new_jokes.get(u["id"])
            if not jid:
                continue
            cands = (self._cands.get(u["id"]) or [])[:k]
            if not cands:
                continue
            blocks = []
            for c in cands:
                j = kb.view.nodes.get(c["joke_id"], {})
                ub = "\n".join(f"    [{x}] {excerpt((kb.view.nodes.get(x) or {}).get('text', ''), 600)}" for x in c["units"][:1])
                blocks.append(f"- joke_id {c['joke_id']}: \"{j.get('title', '')}\": {j.get('summary', '')}\n{ub}")
            msgs = [{"role": "system", "content": SYS_VOICE + ("\n\n" + fs if fs else "")},
                    {"role": "user", "content":
                        f"NEW joke:\n\"\"\"{u['text']}\"\"\"\n\nEXISTING jokes:\n" + "\n".join(blocks) + "\n\n"
                        "For each existing joke, is there a link? same_premise = same underlying premise, different "
                        "joke; tag_of = the new one is a tag/topper that could follow the existing one; callback_to = "
                        "the new one calls back to it; none = no real link (the usual answer). Quote exact lines from "
                        "each side as evidence (new_quote from NEW, old_quote from the existing text)."}]
            c = self.router.complete("link_judge", msgs, S_LINK, run_id=run_id, fewshot_sha=fs_sha)
            valid = {x["joke_id"]: x for x in cands}
            for l in c.data["links"]:
                if l["kind"] == "none" or l["joke_id"] not in valid or float(l["confidence"]) < 0.5:
                    continue
                other = valid[l["joke_id"]]["units"][0]
                srcs = dict(texts)
                srcs[other] = (kb.view.nodes.get(other) or {}).get("text", "")
                kept, dropped = check_quotes([{"node": u["id"], "quote": l["new_quote"]},
                                              {"node": other, "quote": l["old_quote"]}], srcs)
                self._dropped = getattr(self, "_dropped", 0) + len(dropped)
                if not kept:
                    continue
                for x in kept:
                    x["start_ms"] = self.quote_ms(u, x["quote"]) if x["node"] == u["id"] else None
                edges.append({"id": edge_id(jid, l["kind"], l["joke_id"]), "src": jid, "dst": l["joke_id"],
                              "kind": l["kind"], "status": "proposed",
                              "confidence": round(min(1.0, float(l["confidence"])), 2), "reason": l["reason"].strip(),
                              "evidence": kept, "source": f"auto:{run_id}", "origin": origin})


def main(argv=None):
    ap = argparse.ArgumentParser(description="Process new recordings in tapes/inbox.")
    ap.add_argument("--config")
    ap.add_argument("--root", help="put kb/, inbox/, processed/, corpus/ (outputs) under this folder")
    ap.add_argument("--file", help="process just this inbox file")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args(argv)
    cfg = Config(a.config, root=a.root)
    ib = Inbox(cfg, log=lambda s: print(s, file=sys.stderr, flush=True))
    reports = [ib.process(Path(a.file))] if a.file else ib.run()
    if a.json:
        print(json.dumps(reports, indent=1))
    else:
        for r in reports:
            print(f"{r['file']}: {r['status']} ({r['run_id']})")
            if r["status"] == "done":
                print("  stats:", json.dumps(r["stats"]))
                for k, s in r["stages"].items():
                    print(f"  {k:12s} {s.get('s', 0):8.1f}s  calls={s.get('llm_calls', 0):3d}  "
                          f"in={s.get('tokens_in', 0):7d}  out={s.get('tokens_out', 0):6d}"
                          + ("  (cached)" if s.get("cached") else "") + (f"  ({s['skipped']})" if s.get("skipped") else ""))
        if not reports:
            print("inbox empty")


if __name__ == "__main__":
    main()
