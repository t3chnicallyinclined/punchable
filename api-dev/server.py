"""Punchable dev API: the app's JSON API over the real joke knowledge base.

    python api-dev/server.py                          # http://127.0.0.1:8796/api/health
    python api-dev/server.py --kb %TEMP%/kbcopy --inbox %TEMP%/inbox --port 8899

Standard library only. Binds to 127.0.0.1. Reuses tools/explorer/kb.py for the
replay of decisions.jsonl over graph.json (graph.json is only ever read).
Every write appends to decisions.jsonl through kb.py. Uploads are saved to the
tapes inbox and reported as "queued"; no pipeline runs from here.
Shapes are documented in api-dev/API.md (the Rust API on the server keeps them).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import mimetypes
import os
import re
import sys
import threading
import time
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "tools" / "explorer"))
import kb as K  # noqa: E402  (the explorer's replay core, reused as-is)
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))  # the creative package (short names, the say route's store)
import ai_overlay as AI  # noqa: E402  (inferred.json + analysis.json over graph.json)
import auth as AUTH  # noqa: E402  (Keycloak bearer tokens when PK_AUTH=1)
import metrics as M  # noqa: E402  (GET /metrics: Prometheus text; infra/OBSERVABILITY.md)
import crash as CRASH  # noqa: E402  (sentry-sdk -> the self-hosted Bugsink; off without SENTRY_DSN)
from creative.data import by_kind, venue_label  # noqa: E402  (the formatted values list rows carry, T16)
from creative.notebook import month_year  # noqa: E402
from creative.util import mmss  # noqa: E402

OWNER_ID = os.environ.get("PK_OWNER_ID", "owner")  # the actor id on your own edits (`by`); PK_OWNER_ID


# ---- the formatted values a list row carries (docs/ARCHITECTURE.md section 4, T16) ----------------------------------
# Additive only: no existing key changes type or meaning; the shipped app ignores keys it does not know.
def venue_fields(venue) -> dict:
    """`venue_short` + `venue_guessed`: the room name the Sets list shows (creative/data.py venue_label)."""
    lab = venue_label(venue)
    return {"venue_short": lab["name"], "venue_guessed": lab["guessed"]}


def when_label(date) -> str:
    """'2015-06-26' -> 'Jun 2015'; '' when there is no date."""
    return month_year(date) if isinstance(date, str) and date else ""


def at_label(start_ms) -> str | None:
    """A moment's place in the tape, 'm:ss' (187000 -> '3:07'); None when the row has no start."""
    return mmss(start_ms / 1000) if isinstance(start_ms, (int, float)) and not isinstance(start_ms, bool) else None


def _app_flags() -> dict:
    """PK_APP_FLAGS: a JSON object of switches the phone reads off /api/health without a new build
    (app/src/lib/flags.ts), e.g. {"glass_accessory": false}. Unset or unreadable = no flags (the app's defaults)."""
    try:
        v = json.loads(os.environ.get("PK_APP_FLAGS") or "{}")
        return v if isinstance(v, dict) else {}
    except ValueError:
        return {}


APP_FLAGS = _app_flags()

COMEDY = Path(os.path.expanduser(os.environ.get("COMEDY_DIR", "~/punchable-data")))  # your data folder
DEFAULT_PORT = 8796
MAX_BODY = 256 * 1024
MAX_UPLOAD = 1024 * 1024 * 1024  # 1 GB
ID_RE = re.compile(r"^[A-Za-z0-9:_@.+\-]{1,300}$")
# Who made a write (`by` on the decision line). His own actions: OWNER_ID, always (an app token can never claim
# another actor). Imports: `import:<source>` (gsync, memo, youtube, upload). Machine judgments: `ai:<what>`.
# Only the home worker (a worker token) or a loopback dev API without auth may name an import or ai actor.
ACTOR_RE = re.compile(r"^(import|ai):[a-z][a-z0-9_]{1,30}$")
STAGE_ORDER = ["working", "testing", "to_try", "idea", "retired"]
RATINGS = ["killed", "worked", "meh", "died"]
HIT_RATINGS = {"killed", "worked"}
TONIGHT_ID = "list:tonight"
AUDIO_EXT = {".webm", ".m4a", ".mp3", ".wav", ".ogg", ".opus", ".mp4", ".mov", ".aac", ".flac", ".caf", ".3gp"}
MAX_JOKES_PAGE = 5000      # the Desk pages 500 at a time; a whole-catalogue page is allowed
JOKE_SORTS = ("recent", "performed", "strongest", "written", "untouched")
VS_ROOM_CLIP = 3.0         # a near-silent room inflates vs_room (a casino room '17: 7.47x); creative/ clips it the same way
# analysis.json summary fields passed through to /api/sets/{id}/analysis as they are (docs/KB_SCHEMA.md "Set analysis")
SUMMARY_PASS = ("material_s", "laugh_s_total", "laugh_s_per_min", "pct_laughing", "laughs_merged", "laughs_per_min_merged",
                "applause_s", "longest_laugh_s", "longest_laugh_joke", "dead_air_max_s", "dead_air_max_at_ms",
                "room_heat", "room_heat_basis")
ROW_PASS = ("laugh_s_per_min", "pct_laughing", "vs_room", "time_to_first_laugh_s", "dead_air_max_s")
DELETE_CONFIRM = "delete my material"
_SENT_END = re.compile(r"[.!?…][\"'”’)\]]*\s+(?=\S)")
_LAST_WORDS = re.compile(r"(?:\S+\s+){0,13}\S+\s*$")


def suggest_rating(max_bars: int) -> str:
    return "killed" if max_bars >= 3 else "worked" if max_bars == 2 else "meh" if max_bars == 1 else "died"


def last_sentence_split(text: str) -> tuple[str, str]:
    """(setup, reveal): the reveal is the last sentence. A display heuristic only."""
    t = re.sub(r"\s+", " ", (text or "").strip())
    parts = re.split(r"(?<=[.!?])\s+(?=\S)", t)
    parts = [p for p in parts if p]
    if len(parts) <= 1:
        words = t.split(" ")
        if len(words) > 8:
            return " ".join(words[:-4]), " ".join(words[-4:])
        return "", t
    return " ".join(parts[:-1]), parts[-1]


def punch_line(text: str) -> str | None:
    """The punch of a written joke: its last sentence (the last 14 words of a long one). Always an exact substring
    of `text`; None when there is nothing to show."""
    t = (text or "").rstrip()
    if not t.strip():
        return None
    ends = list(_SENT_END.finditer(t))
    p = t[ends[-1].end():] if ends else t
    if len(p.split()) > 16 or len(p) > 140:
        m = _LAST_WORDS.search(p)
        p = m.group(0) if m else p
    p = p.strip()
    return p if p and p in (text or "") else None


def short_name_of(title: str, custom: str | None = None) -> str:
    """What he writes on a paper set list (docs/UX_2026-10.md): his own short name, else the title cut at a word
    break inside 18 characters. The one definition is creative/notebook.py; the creative package is always here."""
    from creative.notebook import short_name_of as _sn
    return _sn(title, custom)


def short_title(title: str) -> str:
    """The Desk row title: the catalogue title cut at its first parenthesis, at most 28 characters."""
    t = (title or "").split(" (")[0].strip() or (title or "").strip()
    return t if len(t) <= 28 else t[:27].rstrip() + "…"


def r2(x):
    return None if x is None else round(x, 2)


def visible_view(v: K.View) -> K.View:
    """A copy of the view without hidden jokes/units and the edges that touch them (indexes rebuilt)."""
    hid = {nid for nid, n in v.nodes.items() if n.get("hidden") is True}
    if not hid:
        return v
    w = object.__new__(type(v))
    w.__dict__.update(v.__dict__)
    w.nodes = {k: n for k, n in v.nodes.items() if k not in hid}
    w.edges = {k: e for k, e in v.edges.items() if e.get("src") not in hid and e.get("dst") not in hid}
    w.alias = {k: x for k, x in v.alias.items() if k not in hid}
    w.hidden_ids = hid
    w._index()
    return w


class AppKB(K.KB):
    """kb.KB plus the corpus layout of the real data and the app's own decision lines."""

    backend = "files"

    def __init__(self, kbdir: Path, corpus: Path, tapes: Path, inbox: Path, fixtures: Path | None = None,
                 graph: Path | None = None, id_map: Path | None = None):
        super().__init__(graph or (kbdir / "graph.json"), kbdir / "decisions.jsonl", corpus, kbdir / "KB_REPORT.md")
        self.kbdir = kbdir
        self.tapes = tapes
        self.inbox = inbox
        self._tx_cache: dict[str, tuple] = {}
        self._desc_cache: dict[str, tuple] = {}
        self._app_cache: dict = {}
        self._vis_cache: tuple | None = None
        self.tl = threading.local()  # per request: include_hidden
        self.ai = AI.AIFiles(kbdir, corpus, fixtures, id_map)
        self._ov_key = None
        self._merged = None
        self.ai_index = {"rows": {}, "analysis": {}, "inferred": None}

    # ---- hidden material (kb v1: not_a_joke / pasted_not_his) ---------------
    def view(self, include_hidden: bool | None = None) -> K.View:
        """The replayed state with `hidden: true` jokes and units (and their edges) left out, unless this request
        asked for them (?include_hidden=1). Writes validate against the same view, so a hidden item can only be
        changed when it was explicitly requested."""
        if include_hidden is None:
            include_hidden = bool(getattr(self.tl, "include_hidden", False))
        with self.lock:
            full = self.full_view()
            if include_hidden:
                return full
            if self._vis_cache is None or self._vis_cache[0] is not full:
                self._vis_cache = (full, visible_view(full))
            return self._vis_cache[1]

    def full_view(self) -> K.View:  # overrides kb.KB: graph.json -> AI layer -> decisions replayed on top
        with self.lock:
            self._load_graph()
            aisig = self.ai.signature()
            sig = (self._graph_sig, self._sig(self.decisions_path), self._sig(self.report_path), self.graph_state, aisig)
            if self._view is None or sig != self._view_sig:
                okey = (self._graph_sig, self.graph_state, aisig)
                if self._ov_key != okey or self._merged is None:
                    self._merged, self.ai_index = AI.build_overlay(self._graph, self.ai)
                    self._ov_key = okey
                report = K.parse_report(self.report_path) if self.report_path else []
                v = K.View(self._merged, self._read_decisions(), report, self.laughs_for)
                v.ai = self.ai_index
                self._view = v
                self._view_sig = sig
                self.rev = hashlib.sha1(repr(sig).encode()).hexdigest()[:12]
            return self._view

    # set ids are set:YYYY-MM-DD_<ytid>; folders are YYYYMMDD_<ytid>
    def set_dir(self, set_id: str | None) -> str | None:
        if not set_id:
            return None
        sid = set_id[4:] if set_id.startswith("set:") else set_id
        m = re.match(r"^(\d{4})-?(\d{2})-?(\d{2})_([A-Za-z0-9_-]+)$", sid)
        if not m:
            return None
        return f"{m.group(1)}{m.group(2)}{m.group(3)}_{m.group(4)}"

    def laughs_doc(self, set_id):
        d = self.set_dir(set_id)
        if not d or not self.corpus:
            return None
        p = self.corpus / "sets" / d / "laughs_auto.json"
        sig = self._sig(p)
        if sig is None:
            return None
        hit = self._laughs_cache.get(d)
        if not hit or hit[0] != sig:
            try:
                hit = (sig, json.loads(p.read_text(encoding="utf-8")))
            except (ValueError, OSError):
                return None
            self._laughs_cache[d] = hit
        return hit[1]

    def laughs_for(self, set_id, start_ms, end_ms):  # overrides kb.KB (folder naming differs)
        doc = self.laughs_doc(set_id)
        if not doc or start_ms is None:
            return None
        end = (end_ms if end_ms is not None else start_ms) + 3000
        ls = [l for l in doc.get("laughs", []) if isinstance(l, dict) and l.get("start_ms") is not None
              and start_ms <= l["start_ms"] <= end]
        return {"detector": doc.get("detector"), "run_id": doc.get("run_id"), "laughs": ls,
                "max_bars": max((l.get("bars") or 0 for l in ls), default=0), "hits": len(ls)}

    def transcript(self, set_id) -> list[dict]:
        d = self.set_dir(set_id)
        if not d or not self.corpus:
            return []
        p = self.corpus / "sets" / d / "transcript.json"
        sig = self._sig(p)
        if sig is None:
            return []
        hit = self._tx_cache.get(d)
        if not hit or hit[0] != sig:
            try:
                doc = json.loads(p.read_text(encoding="utf-8"))
            except (ValueError, OSError):
                return []
            segs = [{"start_ms": int(round(s.get("start", 0) * 1000)), "end_ms": int(round(s.get("end", 0) * 1000)),
                     "text": (s.get("text") or "").strip()} for s in doc.get("segments", []) if isinstance(s, dict)]
            hit = (sig, segs)
            self._tx_cache[d] = hit
        return hit[1]

    def video_description(self, d: str | None) -> str | None:
        """The YouTube description (tapes/raw/<set>/info.json, fetch_youtube.py), where show dates often are."""
        if not d:
            return None
        p = self.tapes / "raw" / d / "info.json"
        sig = self._sig(p)
        if sig is None:
            return None
        hit = self._desc_cache.get(d)
        if not hit or hit[0] != sig:
            try:
                desc = json.loads(p.read_text(encoding="utf-8")).get("description")
            except (ValueError, OSError, AttributeError):
                desc = None
            hit = (sig, (desc or "").strip()[:4000] or None)
            self._desc_cache[d] = hit
        return hit[1]

    def audio_path(self, set_id, fmt="opus") -> Path | None:
        d = self.set_dir(set_id)
        if not d:
            return None
        p = self.tapes / "raw" / d / ("audio16k.wav" if fmt == "wav" else "audio.opus")
        return p if p.is_file() else None

    # ---- derived per-view index (rebuilt when the view is rebuilt) --------
    def app(self) -> "AppIndex":
        v = self.view()
        hit = self._app_cache.get(id(v))
        if hit is None or hit[0] is not v:
            if len(self._app_cache) >= 4:  # old views: one visible + one full per data revision is all we need
                self._app_cache = {}
            hit = (v, self.index_class(self, v))
            self._app_cache[id(v)] = hit
        return hit[1]

    @property
    def index_class(self):
        return AppIndex

    def hybrid_search(self, q: str, limit: int, include_hidden: bool):
        """Fused full-text + vector search (SurrealDB backend). The files backend has none: None = not available."""
        return None

    # ---- the app's own lines (shapes documented in API.md) ---------------
    def append_lines(self, lines: list[dict], by: str = OWNER_ID) -> list[int]:
        out = []
        with self.lock:
            for d in lines:
                rec = {"ts": K.now_iso(), "target": d["target"], "id": d["id"], "action": d["action"],
                       "before": d.get("before"), "after": d.get("after"), "reason": (d.get("reason") or "").strip(),
                       "by": by}
                out.append(self._append(rec))
        return out


class AppIndex:
    """Everything the app needs that kb.View doesn't index: performances per joke, ratings, sets."""

    def __init__(self, kb: AppKB, v: K.View):
        self.kb, self.v = kb, v
        self.perf_edge: dict[str, dict] = {}       # perf id -> performed_as edge (best, not rejected first)
        self.joke_perfs: dict[str, list[str]] = {}  # joke id -> perf ids (edge not rejected)
        for e in v.edges.values():
            if e.get("kind") != "performed_as":
                continue
            p, j = e.get("src"), e.get("dst")
            if v.ntype(p) != "performance":
                p, j = j, p
            if v.ntype(p) != "performance" or v.ntype(j) != "joke":
                continue
            cur = self.perf_edge.get(p)
            if cur is None or (K.bucket(cur.get("status")) == "rejected" and K.bucket(e.get("status")) != "rejected"):
                self.perf_edge[p] = e
        for p, e in self.perf_edge.items():
            if K.bucket(e.get("status")) != "rejected":
                self.joke_perfs.setdefault(e["dst"] if v.ntype(e["dst"]) == "joke" else e["src"], []).append(p)
        self.sets = sorted([n for n in v.nodes.values() if n.get("type") == "set"], key=lambda s: s.get("date") or "")
        self.set_perfs: dict[str, list[dict]] = {}
        for n in v.nodes.values():
            if n.get("type") == "performance":
                self.set_perfs.setdefault(n.get("set"), []).append(n)
        for ps in self.set_perfs.values():
            ps.sort(key=lambda p: p.get("start_ms") or 0)
        self._summaries = None
        self._set_info: dict[str, dict] = {}
        self.ai = getattr(v, "ai", None) or {"rows": {}, "analysis": {}, "inferred": None}
        # What the comic has touched (live lines only; undone lines and undo lines don't count).
        self.touch: dict[str, dict] = {}
        for i, d in enumerate(v.decisions):
            if i in v.undone or "undo_of" in d or not d.get("id"):
                continue
            t = self.touch.setdefault(d["id"], {"actions": set(), "fields": set()})
            act = d.get("action")
            after = d.get("after") if isinstance(d.get("after"), dict) else {}
            before = d.get("before") if isinstance(d.get("before"), dict) else {}
            if act == "edit" and set(after) <= {"rating", "rating_auto", "max_bars"} and after.get("rating") \
                    and after.get("rating") == before.get("ai_rating"):
                act = "confirm"  # agreeing with the AI's rating is a confirm, not an edit
            t["actions"].add(act)
            t["fields"].update(after.keys())
            t.setdefault("ends", set()).update(x for x in (before.get("src"), before.get("dst")) if x)
        self.who: dict[str, str] = {}
        for tid, t in self.touch.items():
            js = self._jokes_of(tid)
            for end in t.get("ends", ()):
                js += [j for j in self._jokes_of(end) if j not in js] if v.ntype(end) != "label" else []
            for jid in js:
                cur = self.who.get(jid)
                w = "edited" if t["actions"] - {"confirm"} else "confirmed"
                if cur != "edited":
                    self.who[jid] = w

    def _jokes_of(self, nid) -> list[str]:
        v = self.v
        t = v.ntype(nid)
        if t == "joke":
            return [nid]
        if t == "unit":
            return [v.unit_joke[nid]] if nid in v.unit_joke else []
        if t == "performance":
            j = self.joke_of(nid)
            return [j] if j else []
        e = v.edges.get(nid)
        if e:
            out = []
            for end in (e.get("src"), e.get("dst")):
                out += [j for j in self._jokes_of(end) if j not in out] if v.ntype(end) != "label" else []
            return out
        return []

    # ---- AI state ---------------------------------------------------------
    def edge_state(self, e: dict) -> str:
        """ai (inferred, untouched) | suggested (proposed) | confirmed | edited | yours | rejected (by you) | dismissed (by the AI)."""
        b = K.bucket(e.get("status"))
        if e.get("_by_tristan"):
            return "yours"
        t = self.touch.get(e.get("id"))
        if t:
            if b == "rejected":
                return "rejected"
            if "edit" in t["actions"]:
                return "edited"
            if b == "confirmed":
                return "confirmed"
        if e.get("status") == "inferred":
            return "ai"
        return {"rejected": "dismissed", "confirmed": "confirmed"}.get(b, "suggested")

    def deco(self, ev: dict) -> dict:
        """An explorer EdgeView plus its AI state."""
        if not ev:
            return ev
        e = self.v.edges.get(ev.get("id")) or ev
        ev["state"] = self.edge_state(e)
        ev["ai"] = ev["state"] == "ai"
        return ev

    def stage_ai(self, jid: str) -> dict:
        n = self.v.nodes.get(jid) or {}
        ai = n.get("_ai") or {}
        t = self.touch.get(jid) or {"actions": set(), "fields": set()}
        on = bool(ai) and n.get("stage") == ai.get("stage") and "stage" not in t["fields"] and "confirm" not in t["actions"]
        return {"stage_ai": on, "stage_confidence": ai.get("stage_confidence") if on else None,
                "stage_reason_ai": ai.get("stage_reason") if on else None, "from_stage": ai.get("from_stage") if on else None,
                "stage_confirmed": "confirm" in t["actions"] and "stage" not in t["fields"] and bool(ai)}

    def perf_rating(self, p: dict) -> tuple[str | None, str | None]:
        """(effective rating, by): the comic's rating wins over the AI's."""
        if p.get("rating"):
            return p["rating"], "you"
        if p.get("ai_rating"):
            return p["ai_rating"], "ai"
        return None, None

    def joke_of(self, perf_id):
        e = self.perf_edge.get(perf_id)
        if not e:
            return None
        return e["dst"] if self.v.ntype(e["dst"]) == "joke" else e["src"]

    def rating(self, perf_id):
        return (self.v.nodes.get(perf_id) or {}).get("rating")

    def perf_summary(self, p: dict) -> dict:
        laughs = self.kb.laughs_for(p.get("set"), p.get("start_ms"), p.get("end_ms")) or {"laughs": [], "max_bars": 0, "hits": 0}
        return {"max_bars": laughs["max_bars"], "hits": laughs["hits"], "rating": p.get("rating"),
                "suggested": suggest_rating(laughs["max_bars"])}

    def joke_rooms(self, jid: str) -> dict:
        """Distinct sets the joke was performed in, and sets where the comic confirmed it and it hit."""
        sets, hit_sets, ai_hits = set(), set(), set()
        for pid in self.joke_perfs.get(jid, []):
            p = self.v.nodes.get(pid) or {}
            if p.get("kind") in AI.NOT_COUNTED:
                continue
            sets.add(p.get("set"))
            e = self.perf_edge.get(pid)
            if K.bucket(e.get("status")) == "confirmed":
                r = p.get("rating")
                if r in HIT_RATINGS:
                    hit_sets.add(p.get("set"))
            if self.perf_rating(p)[0] in HIT_RATINGS:
                ai_hits.add(p.get("set"))
        return {"sets": len(sets), "hit_sets": len(hit_sets), "ready_for_working": len(hit_sets) >= 3, "ai_hit_sets": len(ai_hits)}

    # ---- per set: show date, venue, laugh rate, his "audio is bad" -------------
    def set_info(self, sid: str) -> dict:
        """Everything a SetCard adds over the graph's set node (docs/API_CONTRACT.md "Requested by app"). Sources:
        his set edits (decisions) > corpus/sets/<set>/meta.json (show_dates.py, inferred) > the set node."""
        hit = self._set_info.get(sid)
        if hit is not None:
            return hit
        s = self.v.nodes.get(sid) or {}
        d = self.kb.set_dir(sid)
        meta = (self.kb.ai.set_meta(d) if d else None) or {}
        summ = (self.ai["analysis"].get(sid) or {}).get("summary") or {}
        his_date = s.get("show_date")
        orig_venue = s.get("venue")
        for i, dd in enumerate(self.v.decisions):  # the room before any venue edit of his: /api/venues keys rooms by it
            if dd.get("id") == sid and i not in self.v.undone and "undo_of" not in dd and "venue" in (dd.get("after") or {}):
                orig_venue = (dd.get("before") or {}).get("venue", orig_venue)
                break
        out = {
            "show_date": his_date or meta.get("show_date") or None,
            "show_date_source": "yours" if his_date else (meta.get("show_date_source") if meta.get("show_date") else None),
            "description": self.kb.video_description(d),
            "laugh_s_per_min": summ.get("laugh_s_per_min"),
            "room_heat": summ.get("room_heat"),
            "audio_unreliable": s.get("audio_unreliable") if isinstance(s.get("audio_unreliable"), bool) else None,
            "venue_id": venue_id_of(orig_venue or meta.get("venue_guess") or "Unknown"),
            "source": set_source(meta, d),
            "reference": set_reference(meta, d),
        }
        self._set_info[sid] = out
        return out

    def joke_stats(self) -> dict[str, dict]:
        """Per joke, over its analysed nights (not crowd work or banter, not a set he marked "audio is bad"):
        nights, avg laugh-s/min, avg vs_room (clipped at 3.0) and the last show date. A joke in two segments of one
        set is one night: laugh_s and window_s are summed, as creative/ does."""
        if getattr(self, "_jstats", None) is not None:
            return self._jstats
        rowmap = {pid: (sid, row) for sid, rows in self.ai["rows"].items() for row, pid in rows}
        out = {}
        for jid, pids in self.joke_perfs.items():
            nights, last = {}, None
            for pid in pids:
                p = self.v.nodes.get(pid) or {}
                if p.get("kind") in AI.NOT_COUNTED:
                    continue
                sid = p.get("set")
                info = self.set_info(sid) if sid else {}
                day = info.get("show_date") or (self.v.nodes.get(sid) or {}).get("date")
                if day and (last is None or day > last):
                    last = day
                if info.get("audio_unreliable"):
                    continue
                hit = rowmap.get(pid)
                if not hit:
                    continue
                row = hit[1]
                lg = row.get("laughs") if isinstance(row.get("laughs"), dict) else {}
                n = nights.setdefault(sid, {"laugh_s": 0.0, "window_s": 0.0, "lspm": [], "ok": True})
                if isinstance(lg.get("laugh_s"), (int, float)) and isinstance(lg.get("window_s"), (int, float)):
                    n["laugh_s"] += lg["laugh_s"]
                    n["window_s"] += lg["window_s"]
                elif isinstance(row.get("laugh_s_per_min"), (int, float)):
                    n["lspm"].append(row["laugh_s_per_min"])
                else:
                    n["ok"] = False
            rates, rooms = [], []
            for sid, n in sorted(nights.items()):
                if n["window_s"] > 0:
                    rate = n["laugh_s"] / (n["window_s"] / 60)
                elif n["lspm"]:
                    rate = sum(n["lspm"]) / len(n["lspm"])
                else:
                    continue
                rates.append(rate)
                set_rate = ((self.ai["analysis"].get(sid) or {}).get("summary") or {}).get("laugh_s_per_min")
                if isinstance(set_rate, (int, float)) and set_rate > 0:
                    rooms.append(min(VS_ROOM_CLIP, rate / set_rate))
            out[jid] = {"avg_laugh_s_per_min": r2(sum(rates) / len(rates)) if rates else None,
                        "avg_vs_room": r2(sum(rooms) / len(rooms)) if rooms else None, "last_performed": last}
        self._jstats = out
        return out

    def joke_punch(self, jid: str) -> str | None:
        """The punch line of the joke's newest written version (verbatim), or of his own idea text."""
        units = [(self.v.nodes.get(u) or {}) for u, _ in self.v.joke_units.get(jid, [])]
        units = [u for u in units if u.get("text")]
        if units:
            units.sort(key=lambda u: (u.get("date") or "", len(u.get("text") or "")))
            return punch_line(units[-1]["text"])
        j = self.v.nodes.get(jid) or {}
        return punch_line(j.get("summary") or "") if j.get("_by_tristan") else None

    def summaries(self) -> list[dict]:
        if self._summaries is None:
            out = []
            stats = self.joke_stats()
            for s in self.v.joke_summaries():
                rooms = self.joke_rooms(s["id"])
                labels = sorted(s["labels"], key=lambda l: (l["bucket"] != "confirmed", l["name"] or ""))
                st = self.stage_ai(s["id"])
                js = stats.get(s["id"]) or {}
                out.append({"id": s["id"], "title": s["title"], "stage": s["stage"], "summary": s.get("summary") or "",
                            "first_seen": s.get("first_seen"), "last_seen": s.get("last_seen"), "touched": s.get("touched"),
                            "versions": s["units"], "sets": rooms["sets"], "hit_sets": rooms["hit_sets"], "ai_hit_sets": rooms["ai_hit_sets"],
                            "to_review": s["counts"]["proposed"], "labels": labels[:6], "label_ids": [l["id"] for l in s["labels"]],
                            "yours": s.get("yours", False), "stage_ai": st["stage_ai"], "stage_confidence": st["stage_confidence"],
                            "who": "edited" if s.get("yours") else self.who.get(s["id"], "ai"),
                            "short_title": short_title(s["title"]), "punch": self.joke_punch(s["id"]),
                            "short_name": short_name_of(s["title"], (self.v.nodes.get(s["id"]) or {}).get("short_name")),
                            "nights": rooms["sets"], "avg_vs_room": js.get("avg_vs_room"),
                            "avg_laugh_s_per_min": js.get("avg_laugh_s_per_min"), "last_performed": js.get("last_performed")})
            out.sort(key=lambda x: (STAGE_ORDER.index(x["stage"]) if x["stage"] in STAGE_ORDER else 9,
                                    -(x["sets"]), "" if not x.get("last_seen") else "".join(chr(255 - ord(c)) for c in x["last_seen"]),
                                    x["title"].lower()))
            self._summaries = out
        return self._summaries


class DBAppIndex(AppIndex):
    """SurrealDB backend: Working comes from the database (joke.working / joke.working_sets, kept by DEFINE EVENT
    and fn::working in the same transaction as the match or rating that changed it), not from Python."""

    def joke_rooms(self, jid: str) -> dict:
        r = super().joke_rooms(jid)
        w = (getattr(self.v, "db_working", None) or {}).get(jid)
        if w is not None:
            r["hit_sets"] = len(w[1])
            r["ready_for_working"] = bool(w[0])
        return r


# ---------------------------------------------------------------------------
# views
# ---------------------------------------------------------------------------

def brief_joke(a: AppIndex, jid: str) -> dict:
    j = a.v.nodes.get(jid) or {}
    rooms = a.joke_rooms(jid)
    return {"id": jid, "title": j.get("title") or jid, "stage": j.get("stage"), "summary": j.get("summary") or "",
            "short_name": short_name_of(j.get("title") or jid, j.get("short_name")),
            "sets": rooms["sets"], "hit_sets": rooms["hit_sets"], "ai_hit_sets": rooms["ai_hit_sets"],
            "stage_ai": a.stage_ai(jid)["stage_ai"]}


def joke_text(a: AppIndex, jid: str) -> dict:
    """The text shown on the joke page: the newest version, split so the reveal can be highlighted."""
    units = [(a.v.nodes.get(u) or {}) for u, _ in a.v.joke_units.get(jid, [])]
    units = [u for u in units if u.get("text")]
    if not units:
        j = a.v.nodes.get(jid) or {}
        setup, reveal = last_sentence_split(j.get("summary") or j.get("title") or "")
        return {"setup": setup, "reveal": reveal, "from": None}
    units.sort(key=lambda u: (u.get("date") or "", len(u.get("text") or "")))
    u = units[-1]
    text = u["text"]
    if len(text) > 900:
        text = "…" + text[-900:].split(" ", 1)[-1]
    setup, reveal = last_sentence_split(text)
    return {"setup": setup, "reveal": reveal, "from": {"unit": u.get("id"), "date": u.get("date"), "doc_title": u.get("doc_title")}}


def punch_of(kb: AppKB, perf: dict) -> dict:
    """Where the punch is: the line said just before the strongest laugh in the span."""
    laughs = kb.laughs_for(perf.get("set"), perf.get("start_ms"), perf.get("end_ms")) or {"laughs": [], "max_bars": 0, "hits": 0}
    ls = laughs["laughs"]
    s0, s1 = perf.get("start_ms") or 0, perf.get("end_ms") or 0
    segs = [s for s in kb.transcript(perf.get("set")) if s["end_ms"] > s0 and s["start_ms"] < s1 + 500]
    best = max(ls, key=lambda l: ((l.get("bars") or 0), l.get("strength") or 0), default=None)
    if best and segs:
        before = [s for s in segs if s["start_ms"] <= best["start_ms"] + 300]
        chosen = before[-1:] if before else segs[-1:]
        if chosen and len(chosen[0]["text"].split()) < 6 and len(before) >= 2:
            chosen = before[-2:]
    else:
        chosen = segs[-1:]
    if not chosen:
        setup, reveal = last_sentence_split(perf.get("text") or "")
        return {"text": reveal, "start_ms": s0, "end_ms": s1, "laugh": best, "segments": []}
    return {"text": " ".join(c["text"] for c in chosen), "start_ms": chosen[0]["start_ms"], "end_ms": chosen[-1]["end_ms"],
            "laugh": best, "segments": [{"start_ms": s["start_ms"], "end_ms": s["end_ms"], "text": s["text"],
                                         "punch": s in chosen} for s in segs]}


def review_steps(kb: AppKB, a: AppIndex, set_id: str) -> list[dict]:
    steps = []
    perfs = a.set_perfs.get(set_id, [])
    covered = [(p.get("start_ms") or 0, (p.get("end_ms") or 0) + 3000) for p in perfs]
    for p in perfs:
        e = a.perf_edge.get(p["id"])
        jid = a.joke_of(p["id"])
        laughs = kb.laughs_for(p.get("set"), p.get("start_ms"), p.get("end_ms")) or {"laughs": [], "max_bars": 0, "hits": 0}
        pu = punch_of(kb, p)
        lbest = pu.get("laugh")
        kind = "crowd_work" if p.get("kind") in AI.NOT_COUNTED else ("joke" if jid and K.bucket(e.get("status")) != "rejected" else "new_line")
        play_from = max(p.get("start_ms") or 0, (pu["start_ms"] or 0) - 4000)
        play_to = max(play_from + 3000, ((lbest or {}).get("end_ms") or pu["end_ms"] or 0) + 1200)
        steps.append({
            "key": p["id"], "kind": kind, "perf": K.public(p), "name": p.get("name"),
            "joke": brief_joke(a, jid) if jid else None,
            "match": a.deco(a.v.edge_view(e)) if e else None,
            "text": p.get("text") or "", "punch": pu,
            "laughs": laughs["laughs"], "max_bars": laughs["max_bars"], "hits": laughs["hits"],
            "suggested": suggest_rating(laughs["max_bars"]), "rating": p.get("rating"), "ai_rating": p.get("ai_rating"),
            "play": {"start_ms": play_from, "end_ms": play_to},
        })
    # laughs nobody's joke explains: "New line?"
    doc = kb.laughs_doc(set_id) or {}
    segs = kb.transcript(set_id)
    for l in doc.get("laughs", []):
        if (l.get("bars") or 0) < 2 or l.get("segment") not in (None, "set"):
            continue
        if any(a0 <= l["start_ms"] <= a1 for a0, a1 in covered):
            continue
        before = [s for s in segs if s["end_ms"] <= l["start_ms"] + 500 and s["end_ms"] >= l["start_ms"] - 8000]
        if not before:
            continue
        chosen = before[-2:] if len(before[-1]["text"].split()) < 6 and len(before) >= 2 else before[-1:]
        text = " ".join(s["text"] for s in chosen)
        start, end = chosen[0]["start_ms"], chosen[-1]["end_ms"]
        covered.append((start, l["end_ms"]))
        steps.append({
            "key": f"line:{set_id}@{start}", "kind": "new_line", "perf": None, "name": None, "joke": None, "match": None,
            "text": text, "punch": {"text": text, "start_ms": start, "end_ms": end, "laugh": l,
                                    "segments": [{"start_ms": s["start_ms"], "end_ms": s["end_ms"], "text": s["text"], "punch": True} for s in chosen]},
            "laughs": [l], "max_bars": l.get("bars") or 0, "hits": 1, "suggested": suggest_rating(l.get("bars") or 0), "rating": None,
            "play": {"start_ms": max(0, start - 2000), "end_ms": (l.get("end_ms") or end) + 1200},
        })
    steps.sort(key=lambda s: s["punch"]["start_ms"] if s["kind"] == "new_line" else (s["perf"].get("start_ms") or 0))
    return steps


# ---------------------------------------------------------------------------
# set analysis (the AI's reading of a set, with the comic's decisions on top)
# ---------------------------------------------------------------------------
RANK = {"killed": 3, "worked": 2, "meh": 1, "died": 0}


def _title_from(text: str) -> str:
    words = re.sub(r"[^\w' ]+", " ", text or "").split()
    return " ".join(words[:6]).strip() or "New line"


def analysis_rows(kb: AppKB, a: AppIndex, sid: str) -> list[dict] | None:
    rows = a.ai["rows"].get(sid)
    if rows is None:
        return None
    v = a.v
    laughs = [l for l in (kb.laughs_doc(sid) or {}).get("laughs", []) if l.get("segment") in (None, "set")]
    saved = {}
    for j in v.jokes:
        f = j.get("from")
        if isinstance(f, dict) and f.get("set") == sid:
            saved[f.get("perf") or f"@{f.get('start_ms')}"] = j
    out = []
    for row, pid in rows:
        p = v.nodes.get(pid) or {}
        e = a.perf_edge.get(pid)
        live = e and K.bucket(e.get("status")) != "rejected"
        jid = a.joke_of(pid) if live else None
        t = a.touch.get(pid) or {"actions": set(), "fields": set()}
        kind = p.get("kind")
        status = kind if kind in AI.NOT_COUNTED else ("known" if jid else "new")
        rating, by = a.perf_rating(p)
        s0, s1 = p.get("start_ms", row.get("start_ms")) or 0, p.get("end_ms", row.get("end_ms")) or 0
        pls = [x for x in (row.get("punchlines") or []) if isinstance(x, dict) and x.get("text")]
        ticks = [{"start_ms": l["start_ms"], "end_ms": l.get("end_ms"), "bars": l.get("bars") or 0}
                 for l in laughs if s0 <= l["start_ms"] <= s1 + 3000]
        best = max(pls, key=lambda x: x.get("bars") or 0, default=None)
        if best:
            lend = next((l.get("end_ms") for l in laughs if l.get("id") == best.get("laugh_id")), None) or (best.get("at_ms") or s0) + 4000
            pf = max(s0, (best.get("at_ms") or s0) - 4000)
            play = {"start_ms": pf, "end_ms": max(pf + 3000, lend + 1200)}
        else:
            play = {"start_ms": s0, "end_ms": min(max(s1, s0 + 3000), s0 + 20000)}
        joke = brief_joke(a, jid) if jid else None
        unit = v.nodes.get(row.get("version_unit_id") or "") if jid and jid == row.get("joke_id") else None
        sim = []
        for x in row.get("similar") or []:
            o = v.nodes.get((x or {}).get("joke_id"))
            if o and o.get("type") == "joke" and o["id"] != jid:
                sim.append({"joke_id": o["id"], "title": o.get("title"), "stage": o.get("stage"), "why": x.get("why") or "",
                            "confidence": x.get("confidence")})
        hints = []
        scope = v.joke_scope(jid) if jid else set()
        for x in row.get("hint_labels") or []:
            lid = v.alias.get((x or {}).get("label_id"), (x or {}).get("label_id"))
            ln = v.nodes.get(lid)
            if not ln or ln.get("type") != "label":
                continue
            he = next((v.edges[k] for k in v.adj.get(lid, []) if v.edges[k].get("kind") == "has_label" and v.edges[k].get("src") in scope), None)
            hints.append({"label_id": lid, "name": ln.get("name"), "kind": ln.get("kind"), "definition": ln.get("definition") or "",
                          "why": x.get("why") or "", "edge": {"id": he["id"], "state": a.edge_state(he)} if he else None})
        sv = saved.get(pid) or saved.get(f"@{s0}")
        out.append({
            "idx": row.get("idx"), "perf": pid, "start_ms": s0, "end_ms": s1, "at_label": at_label(s0),
            "status": status, "ai_status": row.get("status"), "kind_by": "you" if "kind" in t["fields"] else "ai",
            "title": joke["title"] if joke else (p.get("name") if status in AI.NOT_COUNTED and p.get("name") else "New material"),
            "suggested_title": None if jid or status != "new" else (
                row.get("title") if row.get("status") == "new" and row.get("title") else _title_from(pls[-1]["text"] if pls else (row.get("transcript") or "")[:80])),
            "joke": joke,
            "match": {"edge": e["id"], "state": a.edge_state(e), "confidence": e.get("confidence"), "reason": e.get("reason") or ""} if live else None,
            "match_reason": row.get("match_reason") or "", "match_confidence": row.get("match_confidence"),
            "version": {"id": unit["id"], "date": unit.get("date"), "doc_title": unit.get("doc_title"), "text": (unit.get("text") or "")[:400]}
            if unit and unit.get("type") == "unit" else None,
            "rating": rating, "rating_by": by, "ai_rating": p.get("ai_rating") or row.get("ai_rating"), "rating_basis": row.get("rating_basis") or "",
            "laughs": row.get("laughs") or {"count": len(ticks), "max_bars": max((x["bars"] for x in ticks), default=0), "total_s": 0},
            "ticks": ticks, "punchlines": pls, "play": play, "similar": sim, "hints": hints,
            "script_diff": row.get("script_diff") if jid and jid == row.get("joke_id") else None,
            "transcript": row.get("transcript") or p.get("text") or "",
            "saved_as": {"id": sv["id"], "title": sv.get("title")} if sv else None,
            **{k: row.get(k) for k in ROW_PASS},
        })
    return out


def _summary(rows: list[dict], meta: dict, sid: str, kb: AppKB, a: AppIndex) -> dict:
    ms = (meta or {}).get("summary") or {}
    known = [r for r in rows if r["status"] == "known"]
    key = lambda r: (RANK.get(r["rating"] or "", -1), r["laughs"].get("max_bars") or 0, r["laughs"].get("total_s") or 0)  # noqa: E731
    best = max(known, key=key) if known else None
    worst = min(known, key=key) if known else None
    s = a.v.nodes.get(sid) or {}
    dur = ms.get("duration_s") or s.get("duration_s") or 0
    nl = ms.get("laughs")
    if nl is None:
        nl = len([l for l in (kb.laughs_doc(sid) or {}).get("laughs", []) if l.get("segment") in (None, "set")])
    pick = lambda r: {"id": r["joke"]["id"], "title": r["joke"]["title"], "idx": r["idx"], "rating": r["rating"]} if r else None  # noqa: E731
    return {"duration_s": dur, "jokes_counted": sum(1 for r in rows if r["status"] in ("known", "new")),
            "known_jokes": len(known), "new_material": sum(1 for r in rows if r["status"] == "new"),
            "crowd_work": sum(1 for r in rows if r["status"] == "crowd_work"), "banter": sum(1 for r in rows if r["status"] == "banter"),
            "laughs": nl, "laughs_per_min": ms.get("laughs_per_min") if ms.get("laughs_per_min") is not None else round(nl / max(1, dur / 60), 2),
            "best": pick(best), "weakest": pick(worst if worst is not best else None),
            **{k: ms.get(k) for k in SUMMARY_PASS}}


def analysis_brief(kb: AppKB, a: AppIndex, sid: str) -> dict | None:
    meta = a.ai["analysis"].get(sid)
    rows = analysis_rows(kb, a, sid)
    if rows is None:
        return None
    sm = _summary(rows, meta, sid, kb, a)
    return {"source": meta["source"], "run_id": meta.get("run_id"), "generated_at": meta.get("generated_at"),
            "jokes_counted": sm["jokes_counted"], "new_material": sm["new_material"], "crowd_work": sm["crowd_work"],
            "best": sm["best"], "laughs_per_min": sm["laughs_per_min"]}


def set_analysis(kb: AppKB, a: AppIndex, sid: str) -> dict | None:
    s = a.v.nodes.get(sid)
    rows = analysis_rows(kb, a, sid)
    if not s or rows is None:
        return None
    meta = a.ai["analysis"].get(sid) or {}
    laughs = [{"start_ms": l["start_ms"], "end_ms": l.get("end_ms"), "bars": l.get("bars") or 0}
              for l in (kb.laughs_doc(sid) or {}).get("laughs", []) if l.get("segment") in (None, "set")]
    return {"set": {k: s.get(k) for k in ("id", "title", "date", "venue", "duration_s")},
            "source": meta.get("source"), "run_id": meta.get("run_id"), "generated_at": meta.get("generated_at"),
            "summary": _summary(rows, meta, sid, kb, a), "rows": rows, "laughs": laughs,
            "audio": {"opus": f"/api/audio/{sid}", "wav": f"/api/audio/{sid}?fmt=wav"} if kb.audio_path(sid) else None,
            "reference": a.set_info(sid).get("reference")}


def latest_analyzed(a: AppIndex) -> dict | None:
    xs = [s for s in a.sets if s["id"] in a.ai["rows"]]
    return xs[-1] if xs else (a.sets[-1] if a.sets else None)


def glance(kb: AppKB, a: AppIndex) -> dict:
    counts = {st: 0 for st in K.STAGES}
    ai_working = 0
    for j in a.v.jokes:
        counts[j.get("stage")] = counts.get(j.get("stage"), 0) + 1
        if j.get("stage") == "working" and a.stage_ai(j["id"])["stage_ai"]:
            ai_working += 1
    s = latest_analyzed(a)
    rows = analysis_rows(kb, a, s["id"]) if s else None
    new, hints = [], []
    if rows:
        new = [{"set": s["id"], "idx": r["idx"], "perf": r["perf"], "start_ms": r["start_ms"], "end_ms": r["end_ms"],
                "at_label": at_label(r["start_ms"]),
                "title": r["suggested_title"], "punch": (r["punchlines"][-1]["text"] if r["punchlines"] else r["transcript"][:160]),
                "rating": r["rating"], "saved_as": r["saved_as"]} for r in rows if r["status"] == "new"]
        ranked = sorted([r for r in rows if r["joke"]], key=lambda r: (-RANK.get(r["rating"] or "", -1), -(r["laughs"].get("total_s") or 0)))
        # one hint per joke, best jokes first, alternating "similar joke" and "label"
        seen = set()
        for k, r in enumerate(ranked):
            if len(hints) >= 3:
                break
            order = ("similar", "label") if k % 2 == 0 else ("label", "similar")
            for kind in order:
                xs = r["similar"] if kind == "similar" else r["hints"]
                x = next((x for x in xs if (x.get("joke_id") or x.get("label_id")) not in seen and x.get("why")), None)
                if not x:
                    continue
                seen.add(x.get("joke_id") or x.get("label_id"))
                tgt = {"id": x["joke_id"], "title": x["title"]} if kind == "similar" else {"id": x["label_id"], "title": x["name"]}
                hints.append({"kind": kind, "idx": r["idx"], "joke": {"id": r["joke"]["id"], "title": r["joke"]["title"]}, "target": tgt, "why": x["why"]})
                break
    return {"stages": counts, "ai_working": ai_working, "set": {"id": s["id"], "title": s.get("title")} if s else None,
            "new_material": new, "hints": hints[:3]}


def recent_key(x: dict) -> str:
    return max((x.get("touched") or "")[:19], (x.get("last_seen") or "")[:10], (x.get("last_performed") or "")[:10],
               (x.get("first_seen") or "")[:10])


def sort_jokes(items: list[dict], how: str) -> list[dict]:
    """Desk sorts (UX_SPEC 4.2). Missing values go last; ties by title then id, so pages are stable."""
    if how not in JOKE_SORTS:
        raise ValueError(f"sort must be one of {', '.join(JOKE_SORTS)}")
    tie = lambda x: (x["title"].lower(), x["id"])  # noqa: E731
    if how == "recent":      # what he touched last, first (the Bits tab's default since 2026-10-29, like a Docs list):
        # the newest of: his last edit/add/confirm on it (a decision timestamp), the newest version, the last night it
        # was on stage, the day it first appeared. Date-only values compare correctly against same-day timestamps.
        return sorted(sorted(items, key=tie), key=recent_key, reverse=True)
    if how == "performed":   # most recently on stage first (by show date), then most recently written
        return sorted(sorted(items, key=tie), key=lambda x: (x.get("last_performed") or "", x.get("last_seen") or ""), reverse=True)
    if how == "strongest":   # best average against the room, then most nights
        return sorted(items, key=lambda x: (x.get("avg_vs_room") is None, -(x.get("avg_vs_room") or 0), -(x.get("nights") or 0), *tie(x)))
    if how == "written":     # most recently written first
        return sorted(sorted(items, key=tie), key=lambda x: x.get("last_seen") or "", reverse=True)
    return sorted(items, key=lambda x: (not x.get("last_seen"), x.get("last_seen") or "", *tie(x)))  # untouched: oldest first


def venue_id_of(raw: str) -> str:
    """The same room id creative/venues.py gives /api/venues."""
    try:
        from creative.util import slug
        from creative.venues import parse_venue
    except ImportError:  # creative/ not importable: still deterministic
        return "venue:" + (re.sub(r"[^a-z0-9]+", "-", (raw or "").lower()).strip("-")[:40] or "untitled")
    return f"venue:{slug(parse_venue(raw)['name'], 40)}"


YT_ID = re.compile(r"^[A-Za-z0-9_-]{11}$")


def set_source(meta: dict, d: str | None) -> str:
    """youtube | voice_memo | app_upload | tape: where a set's recording came from (corpus/sets/<set>/meta.json)."""
    if meta.get("source") in ("voice_memo", "app_upload"):
        return meta["source"]
    vid = meta.get("id")
    if vid and d and d.endswith("_" + str(vid)) and YT_ID.match(str(vid)) and meta.get("upload_date"):
        return "youtube"
    return "tape"


def set_reference(meta: dict, d: str | None) -> dict | None:
    """Where the recording itself lives, since Punchable keeps no media library (online-first, 2026-10-27): a
    YouTube video (play = its url with &t=<seconds>s), his Drive file (a voice memo), or the file on his phone (an
    upload: name and date). None for the old tapes that are only on his PC."""
    src = set_source(meta, d)
    if src == "youtube":
        return {"kind": "youtube", "video_id": meta["id"], "url": f"https://www.youtube.com/watch?v={meta['id']}",
                "title": meta.get("title")}
    if meta.get("drive_id"):
        return {"kind": "drive", "url": f"https://drive.google.com/file/d/{meta['drive_id']}/view",
                "name": meta.get("original_name")}
    if src in ("app_upload", "voice_memo"):  # a memo dropped in from the phone: its file name and recording date
        return {"kind": "file", "name": meta.get("original_name"), "recorded_at": meta.get("recorded_at")}
    return None


def set_card(kb: AppKB, a: AppIndex, s: dict) -> dict:
    perfs = a.set_perfs.get(s["id"], [])
    rateable = [p for p in perfs if p.get("kind") not in AI.NOT_COUNTED and a.joke_of(p["id"])]
    rated = [p for p in rateable if p.get("rating")]
    doc = kb.laughs_doc(s["id"])
    an = analysis_brief(kb, a, s["id"])

    def live_joke(p):
        e = a.perf_edge.get(p["id"])
        return a.joke_of(p["id"]) if e and K.bucket(e.get("status")) != "rejected" else None

    def row_title(p):
        return (a.v.nodes.get(live_joke(p)) or {}).get("title") if live_joke(p) else (p.get("name") or "New material")

    def row_short_name(p):
        j = a.v.nodes.get(live_joke(p)) if live_joke(p) else None
        return short_name_of(j.get("title") or j["id"], j.get("short_name")) if j else short_name_of(row_title(p))
    info = a.set_info(s["id"])
    return {"id": s["id"], "title": s.get("title"), "date": s.get("date"), "venue": s.get("venue"),
            **info,
            # T16: the formatted values the list row shows (additive; the app's rule, no year; the show date when known)
            **venue_fields(s.get("venue")), "when_label": when_label(info.get("show_date") or s.get("date")),
            "duration_s": s.get("duration_s"), "performances": len(perfs), "rateable": len(rateable), "rated": len(rated),
            "reviewed": bool(rateable) and len(rated) == len(rateable), "laughs": len((doc or {}).get("laughs", [])),
            "has_audio": kb.audio_path(s["id"]) is not None, "analysis": an,
            "jokes": [{"perf": p["id"], "title": row_title(p), "short_name": row_short_name(p),
                       "joke": live_joke(p), "kind": p.get("kind"), "rating": a.perf_rating(p)[0], "rating_by": a.perf_rating(p)[1],
                       "start_ms": p.get("start_ms"), "at_label": at_label(p.get("start_ms"))} for p in perfs]}


def tonight(a: AppIndex) -> dict:
    node = a.v.nodes.get(TONIGHT_ID)
    if node and isinstance(node.get("items"), list):
        items, source = node["items"], "saved"
    else:
        cands = [s for s in a.summaries() if s["stage"] in ("to_try", "testing")]
        cands.sort(key=lambda s: (0 if s["stage"] == "to_try" else 1, s["last_seen"] or ""), reverse=False)
        to_try = [c for c in cands if c["stage"] == "to_try"]
        testing = sorted([c for c in cands if c["stage"] == "testing"], key=lambda c: c["last_seen"] or "", reverse=True)
        items = [{"joke": c["id"], "done": False} for c in (testing[:4] + to_try[:4])]
        source = "default"
    out = []
    for it in items:
        jid = it.get("joke")
        if a.v.ntype(jid) != "joke":
            continue
        out.append({**brief_joke(a, jid), "done": bool(it.get("done"))})
    return {"items": out, "source": source, "candidates": [{"id": s["id"], "title": s["title"], "stage": s["stage"]}
                                                           for s in a.summaries() if s["stage"] in ("to_try", "testing")]}


def ai_status(kb: AppKB) -> dict:
    ix = kb.ai_index
    an = ix.get("analysis") or {}
    return {"inferred": ix.get("inferred"),
            "analysis": {"sets": len(an), "real": sum(1 for m in an.values() if m["source"] == "real"),
                         "fixture": sum(1 for m in an.values() if m["source"] == "fixture"),
                         "by_set": {k: {"source": m["source"], "run_id": m.get("run_id")} for k, m in an.items()}}}


def inbox_list(kb: AppKB) -> list[dict]:
    out = []
    if not kb.inbox.is_dir():
        return out
    for p in sorted(kb.inbox.iterdir(), key=lambda p: p.stat().st_mtime, reverse=True):
        if not p.is_file() or p.name.startswith(".") or p.name.endswith(".meta.json"):
            continue
        meta = {}
        mp = p.with_name(p.name + ".meta.json")
        if mp.is_file():
            try:
                meta = json.loads(mp.read_text(encoding="utf-8"))
            except ValueError:
                meta = {}
        row = {"file": p.name, "bytes": p.stat().st_size, "kind": meta.get("kind") or "upload",
               "title": meta.get("title") or p.name, "received_at": meta.get("received_at"),
               "duration_s": meta.get("duration_s"), "marks_ms": meta.get("marks_ms") or [], "status": "queued"}
        if Handler.jobs_on_upload:  # hosted: nothing transcribes it here; the phone will (api-dev/creative_api.py)
            import creative_api as CR
            row["waiting_for"], row["note"] = "transcriber", CR.WAITING_NOTE["transcriber"]
        out.append(row)
    return out


# ---------------------------------------------------------------------------
# HTTP
# ---------------------------------------------------------------------------

class Handler(BaseHTTPRequestHandler):
    kb: AppKB = None
    server_version = "punchable-api-dev/0"
    protocol_version = "HTTP/1.1"
    kbs: dict = {}             # backend name -> KB (main() fills it; hosted: api/entry.py once the data is loaded)
    primary = "surreal"
    creative = None            # creative_api.Creative: the routes in docs/API_CONTRACT.md
    verifier = None            # auth.Verifier when PK_AUTH=1 (hosted); None = open, loopback only
    origins: tuple = ()        # browser origins allowed besides loopback (PUBLIC_URL when hosted)
    boot = None                # hosted: callable -> status dict while the data is not loaded yet
    jobs_on_upload = False     # hosted: an upload queues a process_set job for the home worker
    files_mode = "disk"        # worker file posts (api-dev/ingest_api.py): "db" hosted (srcfile rows), "disk" local
    principal = None

    def handle(self):
        try:
            super().handle()
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass

    def send_response(self, code, message=None):
        self._pk_status = code   # metrics.instrument reads the status this handler sent
        super().send_response(code, message)

    def metrics(self):
        """GET /metrics: open, in-cluster only (the Ingress never forwards it; the NetworkPolicy admits the
        observability namespace). Answers while booting too, so `up` and pk_build_info exist from the first second."""
        b = M.render()
        self.send_response(200)
        self.send_header("Content-Type", M.CONTENT_TYPE)
        self.send_header("Content-Length", str(len(b)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(b)

    def log_message(self, fmt, *args):
        if os.environ.get("PUNCH_API_QUIET"):
            return
        sys.stderr.write("%s %s\n" % (self.log_date_time_string(), fmt % args))

    # CORS: only pages served from this machine (any port) may call the API (hosted: also PUBLIC_URL, same origin).
    def origin_ok(self, o: str) -> bool:
        return bool(re.match(r"^https?://(127\.0\.0\.1|localhost)(:\d+)?$", o)) or o in self.origins

    def cors(self):
        o = self.headers.get("Origin") or ""
        if self.origin_ok(o):
            self.send_header("Access-Control-Allow-Origin", o)
            self.send_header("Vary", "Origin")
            self.send_header("Access-Control-Expose-Headers", "Content-Range, Accept-Ranges, Content-Length")

    def send_json(self, obj, status=200):
        b = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.cors()
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(b)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(b)

    def err(self, status, msg):
        self.send_json({"error": msg}, status)

    def body_json(self, limit: int = MAX_BODY):
        n = int(self.headers.get("Content-Length") or 0)
        if n <= 0:
            return {}
        if n > limit:
            raise K.DecisionError("body too large")
        try:
            b = json.loads(self.rfile.read(n).decode("utf-8"))
        except ValueError:
            raise K.DecisionError("body is not JSON")
        if not isinstance(b, dict):
            raise K.DecisionError("body must be an object")
        return b

    def do_OPTIONS(self):
        self.send_response(204)
        self.cors()
        self.send_header("Access-Control-Allow-Methods", "GET, POST, PATCH, DELETE, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Authorization, Content-Type, Range, X-Filename, X-Kind, X-Title, "
                         "X-Duration-S, X-Marks-Ms, X-Recorded-At, X-Venue, X-Lat, X-Lng, X-Show-Group, X-Align-Working-Set")
        self.send_header("Access-Control-Max-Age", "600")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_HEAD(self):
        self.do_GET()

    # ---- auth ---------------------------------------------------------------
    def deny(self, status: int, msg: str):
        b = json.dumps({"error": msg}).encode("utf-8")
        self.send_response(status)
        self.cors()
        if status == 401:
            self.send_header("WWW-Authenticate", 'Bearer realm="punchable", error="invalid_token"')
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(b)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(b)

    def authorize(self, path: str) -> bool:
        """PK_AUTH=1: a valid Keycloak access token on every route except /api/health (and /api/live, which takes
        the token in its first WebSocket message). A `worker` token may use only /api/worker/*, and only it may."""
        self.principal = None
        v = self.verifier
        if v is None:
            return True
        tok = AUTH.bearer(self.headers.get("Authorization"))
        if path in ("/api/health", "/api/live"):
            if tok:
                try:
                    self.principal = v.verify(tok)
                except AUTH.AuthError:
                    pass
            return True
        if not tok:
            self.deny(401, "sign in required")
            return False
        try:
            self.principal = v.verify(tok)
        except AUTH.AuthError as e:
            self.deny(401, str(e))
            return False
        if AUTH.is_worker(self.principal) != path.startswith("/api/worker/"):
            self.deny(403, "this route is for the home worker only" if path.startswith("/api/worker/")
                      else "a worker token cannot use the app's routes")
            return False
        return True

    def actor(self, b: dict | None = None) -> str:
        """Who this write is by. An app token (or the local app) is the comic: OWNER_ID. A worker token must name an
        import or ai actor in the body (`actor`); a loopback dev API without auth (the PC's own importers, before
        the online-first cutover) may name one too. Anything else is refused."""
        want = (b or {}).get("actor") if isinstance(b, dict) else None
        if self.verifier is not None and AUTH.is_worker(getattr(self, "principal", None) or {}):
            if not isinstance(want, str) or not ACTOR_RE.match(want):
                raise K.DecisionError("a worker write must name its actor: import:<source> or ai:<what>", 400)
            return want
        if want is not None and self.verifier is None:
            if not isinstance(want, str) or not ACTOR_RE.match(want):
                raise K.DecisionError("actor must be import:<source> or ai:<what>", 400)
            return want
        return OWNER_ID

    def booting(self, path: str):
        """Hosted, before the data is loaded: /api/health answers (SurrealDB reachable or not); the rest is 503."""
        st = self.boot() if self.boot else {"ok": False, "error": "not started"}
        if path == "/api/health":
            # ?ready=1 is the readiness probe: 200 only once this pod serves data (infra/k8s/30-api.yaml)
            ready = "ready=1" in (urlparse(self.path).query or "").split("&")
            return self.send_json(st, 200 if (st.get("ready") if ready else st.get("surreal")) else 503)
        return self.err(503, st.get("error") or "the data is not loaded yet (infra/ship.sh dbload)")

    def send_file(self, p: Path):
        size = p.stat().st_size
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(size))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        with open(p, "rb") as f:
            while True:
                chunk = f.read(256 * 1024)
                if not chunk:
                    break
                self.wfile.write(chunk)

    def creative_route(self, method: str, path: str, q: dict) -> bool:
        if self.creative is None:
            return False
        import creative_api as CR
        import merge_api as MG  # /api/merge/* (docs/API_CONTRACT.md "Merge")
        import ask_api as AK  # /api/ask/chats* (docs/API_CONTRACT.md "Ask")
        import mysets_api as MS  # /api/mysets* (docs/API_CONTRACT.md "Your sets")
        import suggest_api as SG  # /api/suggestions/not-a-joke ("not a joke?" voice memos; hide is his tap)
        import say_api as SY  # /api/say + /api/tonight/screen (docs/UX_2026-10.md)
        import profile_api as PF  # /api/profile/voice + /api/worker/profile/voice (his voice profile, as data)
        limit = 8 * 1024 * 1024 if path.startswith("/api/worker/") else MAX_BODY
        return (PF.handle(self.creative, self, method, path, q, lambda: self.body_json(limit))
                or SY.handle(self.creative, self, method, path, q, lambda: self.body_json(limit))
                or SG.handle(self.creative, self, method, path, q, lambda: self.body_json(limit))
                or MS.handle(self.creative, self, method, path, q, lambda: self.body_json(limit))
                or AK.handle(self.creative, self, method, path, q, lambda: self.body_json(limit))
                or MG.handle(self.creative, self, method, path, q, lambda: self.body_json(limit))
                or CR.handle(self.creative, self, method, path, q, lambda: self.body_json(limit)))

    # ---- GET --------------------------------------------------------------
    def pick(self, q: dict):
        """?backend=files|surreal picks the backend for this request (default: the one the server was started with);
        ?include_hidden=1 shows hidden (not_a_joke / pasted_not_his) jokes and units."""
        b = (q.get("backend") or [None])[0]
        kb = self.kbs.get(b) if b else None
        if b and kb is None:
            raise K.DecisionError(f"backend {b!r} is not available (have: {', '.join(sorted(self.kbs))})", 400)
        kb = kb or self.kbs[self.primary]
        kb.tl.include_hidden = (q.get("include_hidden") or ["0"])[0] in ("1", "true", "yes")
        self.kb = kb
        return kb

    def do_GET(self):
        u = urlparse(self.path)
        path, q = u.path, parse_qs(u.query)
        arg = lambda k, d=None: q.get(k, [d])[0]  # noqa: E731
        if path == "/metrics":
            return self.metrics()
        if not self.authorize(path):
            return
        if not self.kbs:
            return self.booting(path)
        try:
            kb = self.pick(q)
        except K.DecisionError as e:
            return self.err(e.status, str(e))
        if path == "/api/live":
            return self.live_socket()
        try:
            import ingest_api as IG  # /api/worker/* ingest routes (online-first): imports land in the database
            if IG.handle(self, "GET", path, q):
                return
            if self.creative_route("GET", path, q):
                return
            if path.startswith("/api/memos"):  # iPhone voice memos (api-dev/memos_api.py; gsync/memos.py fills it)
                import memos_api as MM
                return MM.handle(self, kb, path, q)
            if path == "/api/backend":
                return self.send_json({"primary": self.primary, "this_request": kb.backend,
                                       "backends": {k: (x.status() if hasattr(x, "status") else
                                                        {"backend": "files", "graph": str(x.graph_path), "decisions": str(x.decisions_path)})
                                                    for k, x in self.kbs.items()}})
            if path == "/api/health":
                v = kb.view()
                if self.verifier is not None and self.principal is None:  # signed out: liveness only, no paths
                    return self.send_json({"ok": True, "rev": kb.rev, "auth": "required", "flags": APP_FLAGS})
                return self.send_json({"ok": True, "flags": APP_FLAGS, "graph": str(kb.graph_path), "graph_state": kb.graph_state,
                                       "decisions": str(kb.decisions_path), "decision_lines": len(v.decisions),
                                       "bad_lines": kb.bad_lines, "inbox": str(kb.inbox), "corpus": str(kb.corpus),
                                       "tapes": str(kb.tapes), "rev": kb.rev, "ai": ai_status(kb)})
            m = re.match(r"^/api/audio/([^/]+)$", path)
            if m:
                return self.send_audio(unquote(m.group(1)), arg("fmt", "opus"))
            a = kb.app()
            v = a.v
            if path == "/api/meta":
                counts = {}
                for jj in v.jokes:
                    counts[jj.get("stage")] = counts.get(jj.get("stage"), 0) + 1
                lab = {}
                for s in a.summaries():
                    for lid in s["label_ids"]:
                        lab[lid] = lab.get(lid, 0) + 1
                top = sorted(lab.items(), key=lambda x: -x[1])[: int(arg("labels", "12"))]
                ai_counts = {}
                for s_ in a.summaries():
                    if s_["stage_ai"]:
                        ai_counts[s_["stage"]] = ai_counts.get(s_["stage"], 0) + 1
                who = {w: sum(1 for s_ in a.summaries() if s_["who"] == w) for w in ("ai", "confirmed", "edited")}
                return self.send_json({"stages": K.STAGES, "stage_counts": counts, "ai_stage_counts": ai_counts, "who_counts": who, "ratings": RATINGS,
                                       "top_labels": [{"id": k, "name": v.nodes[k].get("name"), "kind": v.nodes[k].get("kind"),
                                                       "jokes": n} for k, n in top],
                                       "counts": {"jokes": len(v.jokes), "labels": len(v.labels), "edges": len(v.edges),
                                                  "sets": len(a.sets), "performances": sum(len(x) for x in a.set_perfs.values()),
                                                  "decisions": len(v.decisions)}})
            if path == "/api/today":
                latest = a.sets[-1] if a.sets else None
                card = set_card(kb, a, latest) if latest else None
                if card:
                    steps = review_steps(kb, a, latest["id"])
                    card["moments"] = len(steps)
                    card["review_minutes"] = max(1, round(len(steps) * 25 / 60))
                la = latest_analyzed(a)
                latest_an = None
                if la:
                    sa = set_analysis(kb, a, la["id"])
                    if sa:
                        latest_an = {"id": la["id"], "title": la.get("title"), "date": la.get("date"), "source": sa["source"],
                                     **{k: sa["summary"][k] for k in ("jokes_counted", "known_jokes", "new_material", "crowd_work",
                                                                       "laughs_per_min", "best", "weakest")}}
                counts = {s: 0 for s in K.STAGES}
                for jj in v.jokes:
                    counts[jj.get("stage")] = counts.get(jj.get("stage"), 0) + 1
                unreviewed = [set_card(kb, a, s) for s in a.sets]
                unreviewed = [c for c in unreviewed if not c["reviewed"]]
                old = {"latest_set": card, "unreviewed_sets": len(unreviewed),
                       "latest_analysis": latest_an, "glance": glance(kb, a),
                       "tonight": tonight(a), "pipeline": counts, "next_show": None,
                       "inbox": len(inbox_list(kb))}
                if self.creative is not None:
                    import creative_api as CR
                    try:
                        old = CR.merged_today(old, self.creative.corpus(kb), CR.latest_ws(self.creative.stores(kb)))
                    except Exception as e:  # noqa: BLE001  the creative half never takes the old feed down
                        self.log_message("today: creative part failed: %r", e)
                        old["creative_error"] = f"{type(e).__name__}: {e}"
                return self.send_json(old)
            if path == "/api/map/cloud":   # every bit placed by meaning (api-dev/cloud_api.py); 501 on the files backend
                import cloud_api as CL
                cap = arg("cap", str(CL.CAP_DEFAULT))
                if not str(cap).isdigit() or not 1 <= int(cap) <= CL.CAP_MAX:
                    return self.err(400, f"cap must be 1-{CL.CAP_MAX}")
                st, out = CL.cloud_payload(kb, a, int(cap))
                return self.send_json(out, st)
            if path == "/api/search":   # bits + taped set moments, best first (api-dev/search_api.py)
                import search_api as SR
                qs = (arg("q") or "").strip()
                if not qs:
                    return self.err(400, "q is required")
                lim = max(1, min(int(arg("limit", "20")), SR.MAX_LIMIT))
                return self.send_json(SR.search_payload(kb, a, qs, lim, bool(getattr(kb.tl, "include_hidden", False))))
            if path == "/api/jokes":
                items = a.summaries()
                qs = (arg("q") or "").strip()
                if qs and arg("mode") == "hybrid":
                    # fused full-text + vector search (SurrealDB); same item shape, ranked by relevance
                    ranked = kb.hybrid_search(qs, int(arg("limit", "40")), bool(getattr(kb.tl, "include_hidden", False)))
                    if ranked is None:
                        hits = set(v.search(qs))
                        items = [x for x in items if x["id"] in hits]
                    else:
                        rank = {j: i for i, j in enumerate(ranked)}
                        items = sorted([x for x in items if x["id"] in rank], key=lambda x: rank[x["id"]])
                elif qs:
                    hits = set(v.search(qs))
                    items = [x for x in items if x["id"] in hits]
                if arg("stage"):
                    items = [x for x in items if x["stage"] == arg("stage")]
                for lab in [x for x in q.get("label", []) if x]:  # repeated label= : every one must be on the joke
                    items = [x for x in items if lab in x["label_ids"]]
                if arg("who") in ("ai", "confirmed", "edited"):
                    items = [x for x in items if x["who"] == arg("who")]
                if self.creative is not None:  # an auto-attached note shows under its joke (merge_api)
                    import merge_api as MG
                    items = MG.desk_filter(self.creative, kb, items, bool(getattr(kb.tl, "include_hidden", False)))
                if arg("sort"):
                    items = sort_jokes(items, arg("sort"))
                off = max(0, int(arg("offset", "0")))
                lim = max(1, min(int(arg("limit", "40")), MAX_JOKES_PAGE))
                page = [{k: x[k] for k in x if k != "label_ids"} for x in items[off:off + lim]]
                if self.creative is not None:  # the Bits tab's row: how long it runs (from his tapes, else estimated)
                    c = self.creative.corpus(kb)
                    for x in page:
                        d, basis = c.duration(x["id"])
                        x["duration_s"], x["duration_estimated"] = d, not basis.startswith("tape")
                return self.send_json({"items": page, "total": len(items), "offset": off, "limit": lim})
            if path == "/api/pipeline":
                out = {}
                for st in K.STAGES:
                    xs = [x for x in a.summaries() if x["stage"] == st]
                    out[st] = {"total": len(xs), "items": [{k: x[k] for k in ("id", "title", "short_name", "stage", "sets", "hit_sets", "ai_hit_sets", "versions", "stage_ai")}
                                                          for x in xs[: int(arg("limit", "60"))]]}
                return self.send_json({"stages": out})
            m = re.match(r"^/api/jokes/(.+)/similar$", path)
            if m:   # where he did this bit, and what sounds like it (api-dev/search_api.py)
                import search_api as SR
                lim = max(1, min(int(arg("limit", "8")), SR.MAX_LIMIT))
                st, out = SR.similar_payload(kb, a, unquote(m.group(1)), lim, bool(getattr(kb.tl, "include_hidden", False)))
                return self.send_json(out, st)
            m = re.match(r"^/api/jokes/(.+)$", path)
            if m:
                return self.joke(a, unquote(m.group(1)))
            if path == "/api/sets":
                return self.send_json({"sets": [set_card(kb, a, s) for s in reversed(a.sets)]})
            m = re.match(r"^/api/sets/([^/]+)/analysis$", path)
            if m:
                sid = unquote(m.group(1))
                s = v.nodes.get(sid)
                if not s or s.get("type") != "set":
                    return self.err(404, "no such set")
                sa = set_analysis(kb, a, sid)
                if not sa:
                    return self.send_json({"set": {k: s.get(k) for k in ("id", "title", "date", "venue", "duration_s")}, "source": None,
                                           "rows": [], "summary": None, "laughs": [], "audio": None})
                return self.send_json(sa)
            m = re.match(r"^/api/sets/([^/]+)/review$", path)
            if m:
                sid = unquote(m.group(1))
                s = v.nodes.get(sid)
                if not s or s.get("type") != "set":
                    return self.err(404, "no such set")
                steps = review_steps(kb, a, sid)
                return self.send_json({"set": set_card(kb, a, s), "steps": steps,
                                       "audio": {"opus": f"/api/audio/{sid}", "wav": f"/api/audio/{sid}?fmt=wav"}
                                       if kb.audio_path(sid) else None})
            if path == "/api/labels":
                return self.send_json({"labels": [{"id": l["id"], "name": l.get("name"), "kind": l.get("kind"),
                                                   "definition": l.get("definition")} for l in v.labels]})
            if path == "/api/inbox":
                return self.send_json({"items": inbox_list(kb)})
            if path == "/api/notes":
                return self.send_json({"items": [x for x in inbox_list(kb) if x["kind"] == "note"]})
            if path == "/api/learned":
                lp = Path(kb.kbdir) / "learning.json"
                if not lp.is_file():
                    return self.err(404, "nothing learned yet (no learning.json)")
                return self.send_json(json.loads(lp.read_text(encoding="utf-8")))
            if path == "/api/export":
                return self.export(kb, a)
            if path == "/api/decisions":
                n = min(int(arg("limit", "30")), 500)
                ds = [dict(d, index=i, undone=i in v.undone, actor=K.actor_of(d), by_kind=by_kind(K.actor_of(d)))
                      for i, d in enumerate(v.decisions)][-n:][::-1]
                return self.send_json({"decisions": ds, "total": len(v.decisions)})
            return self.err(404, "not found")
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass
        except ValueError as e:
            return self.err(400, str(e))
        except Exception as e:  # noqa: BLE001
            self.log_message("ERROR %r", e)
            CRASH.report(e, route=M.route_pattern(self.path), status=500)
            return self.err(500, f"{type(e).__name__}: {e}")

    def joke(self, a: AppIndex, jid: str):
        v, kb = a.v, self.kb
        d = v.joke_detail(jid)
        if not d:
            return self.err(404, "no such joke")
        perfs = []
        for p in d["performances"]:
            pid = p["perf"]["id"]
            node = v.nodes.get(pid) or {}
            a.deco(p["edge"])
            laughs = p.get("laughs") or {"laughs": [], "max_bars": 0, "hits": 0}
            pu = punch_of(kb, node)
            lb = pu.get("laugh")
            play_from = max(node.get("start_ms") or 0, (pu["start_ms"] or 0) - 4000)
            eff, by = a.perf_rating(node)
            perfs.append({"perf": p["perf"], "set": p["set"], "edge": p["edge"], "laughs": laughs["laughs"],
                          "max_bars": laughs["max_bars"], "hits": laughs["hits"], "rating": node.get("rating"),
                          "ai_rating": node.get("ai_rating"), "rating_basis": node.get("rating_basis") or "",
                          "effective_rating": eff, "rating_by": by, "kind": node.get("kind"),
                          "suggested": suggest_rating(laughs["max_bars"]), "punch": {k: pu[k] for k in ("text", "start_ms", "end_ms")},
                          "play": {"start_ms": play_from, "end_ms": max(play_from + 3000, ((lb or {}).get("end_ms") or pu["end_ms"] or 0) + 1200)},
                          "audio": f"/api/audio/{p['set'].get('id')}" if kb.audio_path(p["set"].get("id")) else None,
                          "reference": a.set_info(p["set"].get("id")).get("reference") if p["set"].get("id") else None})
        for e in d["edges"]:
            a.deco(e)
        for g in d["unit_groups"]:
            for u in g["units"]:
                a.deco(u["edge"])
        order = {"ai": 0, "edited": 1, "yours": 1, "confirmed": 1, "suggested": 2, "rejected": 3, "dismissed": 3}
        labels = sorted([e for e in d["edges"] if e.get("kind") == "has_label"], key=lambda e: (order.get(e["state"], 2), -(e.get("confidence") or 0)))
        links = sorted([e for e in d["edges"] if e.get("kind") != "has_label"], key=lambda e: (order.get(e["state"], 2), -(e.get("confidence") or 0)))
        live = [p for p in perfs if p["edge"]["bucket"] != "rejected" and p.get("kind") not in AI.NOT_COUNTED]
        ai_ratings = [{"set": p["set"].get("id"), "date": p["set"].get("date"), "title": p["set"].get("title"), "perf": p["perf"]["id"],
                       "when_label": when_label(p["set"].get("date")), **venue_fields(p["set"].get("venue")),
                       "rating": p["effective_rating"], "rating_by": p["rating_by"], "ai_rating": p["ai_rating"], "max_bars": p["max_bars"]}
                      for p in live]
        return self.send_json({"joke": d["joke"], "yours": d["yours"], "text": joke_text(a, jid), "rooms": a.joke_rooms(jid),
                               "ai": a.stage_ai(jid), "who": "edited" if d["yours"] else a.who.get(jid, "ai"), "ai_ratings": ai_ratings,
                               "versions": d["unit_groups"], "performances": perfs, "labels": labels, "links": links,
                               "history": d["history"]})

    def live_socket(self):
        """GET /api/live (WebSocket): the SurrealDB backend pushes {type:"change", rev, changes:[...]} whenever
        LIVE SELECT reports a change (a decision, Working flipping, a reload). Replaces the app's 15 s polling."""
        kb = self.kbs.get("surreal")
        hub = getattr(kb, "live", None)
        key = self.headers.get("Sec-WebSocket-Key")
        if (self.headers.get("Upgrade") or "").lower() != "websocket" or not key:
            return self.err(400, "this is a WebSocket endpoint")
        if hub is None:
            return self.err(503, "live updates need the SurrealDB backend")
        origin = self.headers.get("Origin") or ""
        if origin and not self.origin_ok(origin):
            return self.err(403, "origin not allowed")
        import base64
        import queue
        import struct
        accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        self.send_response(101, "Switching Protocols")
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept)
        self.end_headers()
        self.close_connection = True
        sock = self.connection
        wlock = threading.Lock()
        closed = threading.Event()

        def send(op: int, data: bytes):
            n = len(data)
            if n < 126:
                hdr = bytes([0x80 | op, n])
            elif n < 65536:
                hdr = bytes([0x80 | op, 126]) + struct.pack(">H", n)
            else:
                hdr = bytes([0x80 | op, 127]) + struct.pack(">Q", n)
            with wlock:
                sock.sendall(hdr + data)

        def frame(limit: int = 1 << 20):
            h = self.rfile.read(2)
            if len(h) < 2:
                return None, b""
            op, n = h[0] & 0x0F, h[1] & 0x7F
            if n == 126:
                n = struct.unpack(">H", self.rfile.read(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self.rfile.read(8))[0]
            if n > limit:
                raise ValueError("frame too large")
            mask = self.rfile.read(4) if h[1] & 0x80 else b"\0\0\0\0"
            return op, bytes(b ^ mask[i % 4] for i, b in enumerate(self.rfile.read(n)))

        def reader():  # client frames: answer pings, notice close; payloads are ignored
            try:
                while not closed.is_set():
                    op, data = frame()
                    if op is None or op == 0x8:
                        break
                    if op == 0x9:
                        send(0xA, data)
            except (OSError, ValueError):
                pass
            closed.set()

        if self.verifier is not None:
            # PK_AUTH=1: the first message must be {"type":"auth","token":"<access token>"} within 10 s. The token is
            # never in the URL (URLs end up in access logs).
            try:
                sock.settimeout(10)
                op, data = frame(32768)
                msg = json.loads(data.decode("utf-8")) if op == 0x1 else {}
                claims = self.verifier.verify(msg.get("token") if isinstance(msg, dict) and msg.get("type") == "auth" else "")
                if AUTH.is_worker(claims):
                    raise AUTH.AuthError("a worker token cannot subscribe")
            except (AUTH.AuthError, OSError, ValueError) as e:
                try:
                    send(0x1, json.dumps({"type": "error", "error": "unauthorized", "detail": str(e)[:80]}).encode())
                    send(0x8, struct.pack(">H", 4401))
                except OSError:
                    pass
                return
            finally:
                try:
                    sock.settimeout(None)
                except OSError:
                    pass

        q = queue.Queue()
        hub.subscribe(q)
        threading.Thread(target=reader, daemon=True).start()
        M.live_clients.inc()
        try:
            send(0x1, json.dumps({"type": "hello", "rev": getattr(kb, "rev", None), "live": hub.connected}).encode())
            while not closed.is_set():
                try:
                    ev = q.get(timeout=20)
                    send(0x1, json.dumps(ev, ensure_ascii=False, default=str).encode("utf-8"))
                except queue.Empty:
                    send(0x9, b"")  # keep-alive ping
        except OSError:
            pass
        finally:
            M.live_clients.dec()
            hub.unsubscribe(q)
            closed.set()
            try:
                send(0x8, b"")
            except OSError:
                pass

    def send_audio(self, set_id: str, fmt: str):
        if not ID_RE.match(set_id):
            return self.err(400, "bad set id")
        p = self.kb.audio_path(set_id, "wav" if fmt == "wav" else "opus")
        if not p:
            return self.err(404, "no audio for that set")
        size = p.stat().st_size
        ctype = "audio/wav" if p.suffix == ".wav" else "audio/ogg"
        rng = self.headers.get("Range")
        start, end = 0, size - 1
        status = 200
        if rng:
            m = re.match(r"^bytes=(\d*)-(\d*)$", rng.strip())
            if not m or (m.group(1) == "" and m.group(2) == ""):
                self.send_response(416)
                self.cors()
                self.send_header("Content-Range", f"bytes */{size}")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            if m.group(1) == "":
                start, end = max(0, size - int(m.group(2))), size - 1
            else:
                start = int(m.group(1))
                end = min(int(m.group(2)), size - 1) if m.group(2) else size - 1
            if start >= size or start > end:
                self.send_response(416)
                self.cors()
                self.send_header("Content-Range", f"bytes */{size}")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            status = 206
        n = end - start + 1
        self.send_response(status)
        self.cors()
        self.send_header("Content-Type", ctype)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(n))
        self.send_header("Cache-Control", "private, max-age=3600")
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.end_headers()
        if self.command == "HEAD":
            return
        with open(p, "rb") as f:
            f.seek(start)
            left = n
            while left > 0:
                chunk = f.read(min(256 * 1024, left))
                if not chunk:
                    break
                self.wfile.write(chunk)
                left -= len(chunk)

    # ---- POST -------------------------------------------------------------
    def add_version(self, kb, jid: str, b: dict):
        """POST /api/jokes/<id>/versions {text, reason?, source?, draft?}: his new version of a bit: a unit (kind
        tag, source app) + its part_of edge, two decisions by him, both undone together ({indexes}). The old versions
        stay (the bit page shows them as v1, v2, …). `draft: true` = the text came from an AI answer or draft he kept:
        the voice profile learns it (profile_api.learn_kept). The legal rules run on the text first."""
        by = self.actor(b)
        text = (b.get("text") or "").strip() if isinstance(b.get("text"), str) else ""
        if not text or len(text) > 4000:
            raise K.DecisionError("text is required (up to 4000 characters)")
        import creative_api as CA
        try:
            CA.refuse_legal(text)
        except CA.HttpError as e:
            return self.err(e.status, str(e))
        reason = (b.get("reason") or "").strip()[:300] if isinstance(b.get("reason"), str) else ""
        reason = reason or "a new version written in the app"
        source = b.get("source") if isinstance(b.get("source"), str) and re.match(r"^[a-z_]{1,24}$", b.get("source")) else "app"
        with kb.lock:
            v = kb.view()
            if v.ntype(jid) != "joke":
                raise K.DecisionError("no such joke", 404)
            r1 = kb.decide({"target": "node", "action": "add", "reason": reason,
                            "after": {"type": "unit", "text": text, "kind": "tag", "source": source, "date": K.now_iso()[:10]}}, by=by)
            uid = r1["decision"]["id"]
            try:
                r2 = kb.decide({"target": "edge", "action": "add", "reason": reason,
                                "after": {"src": uid, "dst": jid, "kind": "part_of"}}, by=by)
            except Exception:
                kb.undo(r1["index"], by=by)   # never a line that belongs to nothing
                raise
        out = {"ok": True, "unit": uid, "joke": jid, "indexes": [r1["index"], r2["index"]]}
        if self.creative is not None and by == OWNER_ID and b.get("draft"):
            import profile_api as PF
            out["learned"] = PF.learn_kept(self.creative, kb, text, jid, by, draft=True)
        return self.send_json(out)

    def do_POST(self):
        u = urlparse(self.path)
        path = u.path
        if not self.authorize(path):
            return
        if not self.kbs:
            return self.booting(path)
        try:
            kb = self.pick(parse_qs(u.query))
            if path == "/api/upload":
                return self.upload()
            import ingest_api as IG
            if IG.handle(self, "POST", path, parse_qs(u.query)):
                return
            if self.creative_route("POST", path, parse_qs(u.query)):
                return
            b = self.body_json()
            m = re.match(r"^/api/jokes/(.+)/versions$", path)
            if m:   # a new version of a bit, written in the app (the workbench's edit, a kept answer, a typed line)
                return self.add_version(kb, unquote(m.group(1)), b)
            if path == "/api/decision":
                if b.get("id") is not None and (not isinstance(b["id"], str) or not ID_RE.match(b["id"])):
                    raise K.DecisionError("bad id")
                by = self.actor(b)
                r = kb.decide(b, by=by)
                out = {"ok": True, "indexes": [r["index"]], "decisions": [r["decision"]]}
                after = b.get("after") if isinstance(b.get("after"), dict) else {}
                if (self.creative is not None and by == OWNER_ID and b.get("target") == "node" and b.get("action") == "add"
                        and after.get("type") == "unit" and after.get("source") == "draft" and isinstance(after.get("text"), str)):
                    # Brainstorm's "Use as mine": a punch-up draft kept as his version of the bit (the variant_of edge
                    # follows in the app's next call; the unit's own id is the bit reference until then)
                    import profile_api as PF
                    out["learned"] = PF.learn_kept(self.creative, kb, after["text"], after.get("id") or r["decision"].get("id"), by)
                return self.send_json(out)
            if path == "/api/review/rate":
                return self.rate(b)
            if path == "/api/set-row":
                return self.set_row(b)
            if path == "/api/review/reject-match":
                pid = b.get("perf")
                a = kb.app()
                e = a.perf_edge.get(pid)
                if not e:
                    raise K.DecisionError("no match to reject for that performance", 404)
                r = kb.decide({"target": "edge", "id": e["id"], "action": "reject", "reason": b.get("reason") or ""})
                return self.send_json({"ok": True, "indexes": [r["index"]], "decisions": [r["decision"]]})
            if path == "/api/ideas":
                return self.idea(b)
            if path == "/api/tonight":
                return self.save_tonight(b)
            if path == "/api/undo":
                idxs = b.get("indexes")
                if idxs is None:
                    idxs = [b.get("index")]
                if not isinstance(idxs, list) or not all(i is None or isinstance(i, int) for i in idxs):
                    raise K.DecisionError("indexes must be a list of integers")
                out = []
                by = self.actor(b)
                with kb.lock:
                    for i in sorted(idxs, key=lambda x: -1 if x is None else x, reverse=True):
                        out.append(kb.undo(i, by=by))
                return self.send_json({"ok": True, "indexes": [r["index"] for r in out], "decisions": [r["decision"] for r in out]})
            return self.err(404, "not found")
        except K.DecisionError as e:
            return self.err(e.status, str(e))
        except Exception as e:  # noqa: BLE001
            self.log_message("ERROR %r", e)
            CRASH.report(e, route=M.route_pattern(self.path), status=500)
            return self.err(500, f"{type(e).__name__}: {e}")

    def do_PATCH(self):
        u = urlparse(self.path)
        path = u.path
        if not self.authorize(path):
            return
        if not self.kbs:
            return self.booting(path)
        try:
            self.pick(parse_qs(u.query))
            if self.creative_route("PATCH", path, parse_qs(u.query)):
                return
            return self.err(404, "not found")
        except K.DecisionError as e:
            return self.err(e.status, str(e))
        except Exception as e:  # noqa: BLE001
            self.log_message("ERROR %r", e)
            CRASH.report(e, route=M.route_pattern(self.path), status=500)
            return self.err(500, f"{type(e).__name__}: {e}")

    def do_PUT(self):
        """PUT replaces one whole document (today: his voice profile, api-dev/profile_api.py)."""
        u = urlparse(self.path)
        path = u.path
        if not self.authorize(path):
            return
        if not self.kbs:
            return self.booting(path)
        try:
            self.pick(parse_qs(u.query))
            if self.creative_route("PUT", path, parse_qs(u.query)):
                return
            return self.err(404, "not found")
        except K.DecisionError as e:
            return self.err(e.status, str(e))
        except Exception as e:  # noqa: BLE001
            self.log_message("ERROR %r", e)
            CRASH.report(e, route=M.route_pattern(self.path), status=500)
            return self.err(500, f"{type(e).__name__}: {e}")

    def do_DELETE(self):
        u = urlparse(self.path)
        path = u.path
        if not self.authorize(path):
            return
        if not self.kbs:
            return self.booting(path)
        try:
            kb = self.pick(parse_qs(u.query))
            if path == "/api/me":
                return self.delete_me(kb, self.body_json())
            return self.err(404, "not found")
        except K.DecisionError as e:
            return self.err(e.status, str(e))
        except Exception as e:  # noqa: BLE001
            self.log_message("ERROR %r", e)
            CRASH.report(e, route=M.route_pattern(self.path), status=500)
            return self.err(500, f"{type(e).__name__}: {e}")

    def inventory(self, kb: AppKB) -> list[dict]:
        """What "delete all my data" covers, with sizes. Read-only."""
        def lines(p: Path) -> int | None:
            try:
                with open(p, "rb") as f:
                    return sum(1 for ln in f if ln.strip())
            except OSError:
                return None
        v = kb.full_view()
        out = [{"what": "your decisions (every confirm, edit, rating, idea)", "where": str(kb.decisions_path),
                "count": len(v.decisions)},
               {"what": "your jokes, versions and labels (the knowledge graph)", "where": str(kb.graph_path),
                "count": len(v.jokes)}]
        if getattr(kb, "backend", "files") == "surreal":
            out.append({"what": "the database copy of all of it", "where": f"surrealdb {kb.ns}/{kb.db}", "count": None})
        if self.creative is not None:
            for n in ("storyboards", "workingsets", "venues", "account", "jobs"):
                pth = self.creative.dir / f"{n}.jsonl"
                out.append({"what": f"{n} (append-only log + snapshot)", "where": str(pth), "count": lines(pth) or 0})
        sets = Path(kb.corpus) / "sets"
        out.append({"what": "your sets: transcripts, laughs, analyses, clips", "where": str(sets),
                    "count": sum(1 for d in sets.iterdir() if d.is_dir()) if sets.is_dir() else 0})
        raw = Path(kb.tapes) / "raw"
        out.append({"what": "your tapes (audio)", "where": str(raw),
                    "count": sum(1 for d in raw.iterdir() if d.is_dir()) if raw.is_dir() else 0})
        out.append({"what": "uploads waiting in the inbox", "where": str(kb.inbox), "count": len(inbox_list(kb))})
        return out

    def delete_me(self, kb: AppKB, b: dict):
        """DELETE /api/me {"confirm": "delete my material"}. Never deletes anything here.
        Local dev: answers what WOULD be deleted (dry run). Hosted (PK_AUTH=1): records a deletion request (his
        `account` log) and answers the same list. Real deletion is a manual step the owner runs (api-dev/API.md)."""
        if (b.get("confirm") or "").strip().lower() != DELETE_CONFIRM:
            raise K.DecisionError(f'type "{DELETE_CONFIRM}" as {{"confirm": "..."}} to ask for deletion')
        would = self.inventory(kb)
        if self.verifier is None:
            return self.send_json({"status": "dry_run", "deleted": False, "would_delete": would,
                                   "note": "Local dev: nothing is ever deleted by the API. This is what a deletion would cover."})
        if self.creative is None:
            raise K.DecisionError("deletion requests need the creative store (started with --no-creative)", 503)
        with self.creative.lock:
            st = self.creative.stores(kb)
            st["account"].append("request_deletion", "deletion", {"by_sub": (self.principal or {}).get("sub"),
                                                                  "would_delete": would})
        return self.send_json({"status": "requested", "deleted": False, "would_delete": would,
                               "note": "Your request is recorded. The owner deletes it by hand and tells you when it is done."}, 202)

    def export(self, kb: AppKB, a: "AppIndex"):
        """GET /api/export: everything that is his, as one JSON download (decisions, sets, creative logs, settings).
        Audio is not included (it is on his phone and in tapes/)."""
        v = a.v
        out = {"format": "punchable-export/1", "exported_at": K.now_iso(),
               "decisions": v.decisions, "sets": [set_card(kb, a, s) for s in reversed(a.sets)],
               "ideas": [{k: j.get(k) for k in ("id", "title", "stage", "summary", "from", "first_seen")}
                         for j in v.jokes if j.get("_by_tristan")],
               "inbox": inbox_list(kb)}
        if self.creative is not None:
            import creative_api as CR
            st = self.creative.stores(kb)
            out["creative"] = {n: {"items": st[n].items, "events": [json.loads(x) for x in (
                st[n]._lines if hasattr(st[n], "_lines") else CR.file_lines(self.creative.dir / f"{n}.jsonl"))]}
                for n in ("storyboards", "workingsets", "venues", "account")}
        b = json.dumps(out, ensure_ascii=False, indent=1).encode("utf-8")
        self.send_response(200)
        self.cors()
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Disposition", f'attachment; filename="punchable-export-{datetime.now():%Y%m%d}.json"')
        self.send_header("Content-Length", str(len(b)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(b)

    def rate(self, b: dict):
        kb = self.kb
        pid, rating = b.get("perf"), b.get("rating")
        if rating not in RATINGS:
            raise K.DecisionError(f"rating must be one of {RATINGS}")
        with kb.lock:
            a = kb.app()
            p = a.v.nodes.get(pid)
            if not p or p.get("type") != "performance":
                raise K.DecisionError("unknown performance", 404)
            lines = []
            e = a.perf_edge.get(pid)
            if e and K.bucket(e.get("status")) == "proposed" and b.get("confirm_match", True) is not False:
                lines.append({"target": "edge", "id": e["id"], "action": "confirm", "before": K.public(e),
                              "after": {"status": "confirmed"}, "reason": "review: this is the joke"})
            summ = a.perf_summary(p)
            after = {"rating": rating, "rating_auto": summ["suggested"], "max_bars": summ["max_bars"]}
            before = K.public(p)
            if all(before.get(k) == x for k, x in after.items()):
                raise K.DecisionError("already rated that way", 409)
            lines.append({"target": "node", "id": pid, "action": "edit", "before": before, "after": after,
                          "reason": b.get("reason") or ""})
            idx = kb.append_lines(lines)
            a2 = kb.app()
            jid = a2.joke_of(pid)
            out = {"ok": True, "indexes": idx, "decisions": [kb.view().decisions[i] for i in idx]}
            if jid:
                out["joke"] = {**brief_joke(a2, jid), **a2.joke_rooms(jid)}
            return self.send_json(out)

    def set_row(self, b: dict):
        """In-place edits on the Set page. Each op appends one decision line (two for "new" on crowd work).

        {perf, op:"rate", rating}       node edit on the performance (the AI's match is left as it is)
        {perf, op:"match", joke}        confirm / edit / add the performed_as edge so it points at that joke
        {perf, op:"new"}                reject the match (and un-mark crowd work)
        {perf, op:"crowd"|"bit"}        node edit kind on the performance
        {perf, op:"confirm"}            confirm the AI's match
        """
        kb = self.kb
        pid, op = b.get("perf"), b.get("op")
        if op == "rate":
            return self.rate({**b, "confirm_match": False})
        with kb.lock:
            a = kb.app()
            p = a.v.nodes.get(pid)
            if not p or p.get("type") != "performance":
                raise K.DecisionError("unknown performance", 404)
            e = a.perf_edge.get(pid)
            live = e and K.bucket(e.get("status")) != "rejected"
            reason = (b.get("reason") or "").strip()
            if op == "confirm":
                if not live:
                    raise K.DecisionError("nothing matched to confirm", 409)
                r = kb.decide({"target": "edge", "id": e["id"], "action": "confirm", "reason": reason})
                return self.send_json({"ok": True, "indexes": [r["index"]], "decisions": [r["decision"]]})
            if op == "match":
                jid = b.get("joke")
                if a.v.ntype(jid) != "joke":
                    raise K.DecisionError("pick one of your jokes", 404)
                if e:
                    end = "dst" if a.v.ntype(e.get("dst")) == "joke" else "src"
                    if e.get(end) == jid:
                        if live and K.bucket(e.get("status")) == "confirmed":
                            raise K.DecisionError("already matched to that joke", 409)
                        req = {"target": "edge", "id": e["id"], "action": "confirm", "reason": reason}
                    else:
                        req = {"target": "edge", "id": e["id"], "action": "edit", "after": {end: jid}, "reason": reason}
                else:
                    req = {"target": "edge", "action": "add", "after": {"src": pid, "dst": jid, "kind": "performed_as"},
                           "reason": reason or "matched on the set page"}
                r = kb.decide(req)
                return self.send_json({"ok": True, "indexes": [r["index"]], "decisions": [r["decision"]]})
            if op in ("crowd", "bit", "banter"):
                kind = {"crowd": "crowd_work", "bit": "bit", "banter": "banter"}[op]
                if p.get("kind") == kind:
                    raise K.DecisionError("already marked that way", 409)
                idx = kb.append_lines([{"target": "node", "id": pid, "action": "edit", "before": K.public(p), "after": {"kind": kind}, "reason": reason}])
                return self.send_json({"ok": True, "indexes": idx, "decisions": [kb.view().decisions[i] for i in idx]})
            if op == "new":
                lines = []
                if p.get("kind") in AI.NOT_COUNTED:
                    lines.append({"target": "node", "id": pid, "action": "edit", "before": K.public(p), "after": {"kind": "bit"}, "reason": reason})
                if live:
                    lines.append({"target": "edge", "id": e["id"], "action": "reject", "before": K.public(e), "after": {"status": "rejected"},
                                  "reason": reason or "new material, not this joke"})
                if not lines:
                    raise K.DecisionError("already new material", 409)
                idx = kb.append_lines(lines)
                return self.send_json({"ok": True, "indexes": idx, "decisions": [kb.view().decisions[i] for i in idx]})
            raise K.DecisionError("op must be rate|match|new|crowd|bit|banter|confirm")

    def idea(self, b: dict, by: str | None = None):
        """POST /api/ideas: a new idea (a joke node at stage idea). `by` = who: the comic (OWNER_ID) or an import."""
        return self.send_json(self.idea_result(b, by))

    def idea_result(self, b: dict, by: str | None = None) -> dict:
        """The idea flow as a value (POST /api/say reuses it): the node add line, the auto-attach, the embed job."""
        kb = self.kb
        by = by or self.actor(b)
        text = (b.get("text") or "").strip()
        if not text or len(text) > 4000:
            raise K.DecisionError("an idea needs its text (up to 4000 characters)")
        title = (b.get("title") or "").strip() or " ".join(text.split()[:7]).strip(" ,.;:!?\"'")
        title = title[:120] or "New line"
        with kb.lock:
            v = kb.view()
            d = kb._build(v, {"target": "node", "action": "add", "after": {"type": "joke", "title": title, "stage": "idea",
                                                                            "summary": text}})
            frm = b.get("from") if isinstance(b.get("from"), dict) else None
            if frm:
                d["after"]["from"] = {k: frm.get(k) for k in ("set", "perf", "start_ms", "end_ms", "note") if frm.get(k) is not None}
            idx = kb.append_lines([{**d, "reason": b.get("reason") or ("saved from review" if frm else "")}], by=by)
            out = {"ok": True, "indexes": idx, "id": d["id"], "decisions": [kb.view().decisions[i] for i in idx]}
        if self.creative is not None:  # a jot that is clearly an existing bit attaches to it (merge_api; undoable)
            import merge_api as MG
            src = by.split(":", 1)[1] if by.startswith("import:") else (
                "gdocs" if (b.get("reason") or "").startswith("[gdocs]") else "app")
            out["attach"] = MG.after_idea(self.creative, kb, d["id"], text, src)
            if by == OWNER_ID:
                # a riff draft he took as his own ("Use as mine": the app says `draft: true`, or the text is a
                # finished riff job's draft word for word): the voice profile's learned section
                # (profile_api.learn_kept) remembers it, and the drafts he passed over beside it; his own typed
                # line is no draft and writes nothing there
                import profile_api as PF
                into = (out["attach"] or {}).get("into") if isinstance(out.get("attach"), dict) else None
                out["learned"] = PF.learn_kept(self.creative, kb, text, into or d["id"], by, draft=b.get("draft") is True)
            if self.jobs_on_upload and by == OWNER_ID:
                # hosted: no model in this process, so his typed idea gets its vector from the home worker (job
                # `embed`, one pending at a time); imports queue theirs in ingest_api.enqueue_followups
                try:
                    with self.creative.lock:
                        self.creative.enqueue_followup(self.creative.stores(kb), "embed", OWNER_ID)
                except Exception as e:  # noqa: BLE001  the idea is saved either way
                    self.log_message("embed job not queued: %r", e)
        return out

    def save_tonight(self, b: dict):
        kb = self.kb
        items = b.get("items")
        if not isinstance(items, list) or len(items) > 60:
            raise K.DecisionError("items must be a list (up to 60)")
        with kb.lock:
            v = kb.view()
            clean = []
            for it in items:
                if not isinstance(it, dict) or v.ntype(it.get("joke")) != "joke":
                    raise K.DecisionError(f"unknown joke in list: {it!r}"[:200])
                clean.append({"joke": it["joke"], "done": bool(it.get("done"))})
            node = v.nodes.get(TONIGHT_ID)
            if node is None:
                line = {"target": "node", "id": TONIGHT_ID, "action": "add", "before": None,
                        "after": {"type": "list", "name": "Tonight", "items": clean}}
            else:
                if node.get("items") == clean:
                    raise K.DecisionError("nothing changed", 409)
                line = {"target": "node", "id": TONIGHT_ID, "action": "edit", "before": K.public(node), "after": {"items": clean}}
            idx = kb.append_lines([{**line, "reason": b.get("reason") or ""}])
            return self.send_json({"ok": True, "indexes": idx, "tonight": tonight(kb.app())})

    def upload(self):
        kb = self.kb
        n = int(self.headers.get("Content-Length") or 0)
        if n <= 0 or n > MAX_UPLOAD:
            raise K.DecisionError("upload missing or over 1 GB")
        kind = (self.headers.get("X-Kind") or "upload").strip()
        if kind not in ("set", "note", "upload"):
            raise K.DecisionError("X-Kind must be set|note|upload")
        orig = unquote(self.headers.get("X-Filename") or "")
        ext = Path(orig).suffix.lower()
        if ext not in AUDIO_EXT:
            ct = (self.headers.get("Content-Type") or "").split(";")[0].strip()
            ext = {"audio/webm": ".webm", "video/webm": ".webm", "audio/mp4": ".m4a", "audio/mpeg": ".mp3", "audio/wav": ".wav",
                   "audio/ogg": ".ogg", "video/mp4": ".mp4", "video/quicktime": ".mov"}.get(ct)
            if not ext:
                raise K.DecisionError("only audio or video files can be uploaded")
        now = datetime.now()
        stem = re.sub(r"[^a-z0-9]+", "-", Path(orig).stem.lower()).strip("-")[:40] or "recording"
        name = f"{now:%Y-%m-%d}_punchable-{kind}_{now:%H%M%S}_{stem}{ext}"
        kb.inbox.mkdir(parents=True, exist_ok=True)
        # the name is reserved with an exclusive create, so two uploads in the same second (the phone posting one
        # recording twice, seen 2026-10-29) never share a destination; the body goes to a private temp first
        dest = kb.inbox / name
        k = 2
        while True:
            try:
                os.close(os.open(dest, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o644))
                break
            except FileExistsError:
                dest = kb.inbox / f"{Path(name).stem}-{k}{ext}"
                k += 1
        tmp = dest.with_name(f"{dest.name}.{os.getpid()}-{threading.get_ident()}.part")
        left = n
        with open(tmp, "wb") as f:
            while left > 0:
                chunk = self.rfile.read(min(1024 * 1024, left))
                if not chunk:
                    break
                f.write(chunk)
                left -= len(chunk)
        if left:
            tmp.unlink(missing_ok=True)
            raise K.DecisionError("upload was cut short")
        os.replace(tmp, dest)
        try:
            marks = json.loads(self.headers.get("X-Marks-Ms") or "[]")
            marks = [int(x) for x in marks][:500] if isinstance(marks, list) else []
        except (ValueError, TypeError):
            marks = []
        try:
            dur = float(self.headers.get("X-Duration-S") or 0) or None
        except ValueError:
            dur = None
        meta = {"kind": kind, "original_name": orig or None, "title": unquote(self.headers.get("X-Title") or "")[:200] or None,
                "received_at": K.now_iso(), "bytes": n, "duration_s": dur, "marks_ms": marks, "status": "queued",
                "by": "punchable-api-dev"}
        meta.update(upload_extras(self.headers))
        dest.with_name(dest.name + ".meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=1), encoding="utf-8")
        out = {"status": "queued", "file": dest.name, "kind": kind, "bytes": n}
        if self.jobs_on_upload and self.creative is not None:
            # hosted: queued as process_set. No worker transcribes any more (the PC is retired; transcription
            # happens on the phone), so the job says so: waiting_for "transcriber" and the note, never a silent drop.
            st = self.creative.stores(kb)
            import creative_api as CA
            dup = CA.recent_duplicate(st["jobs"].items, meta, time.time())
            if dup is not None:   # the same recording posted twice within two minutes: one job, the first file
                dest.unlink(missing_ok=True)
                dest.with_name(dest.name + ".meta.json").unlink(missing_ok=True)
                out.update(file=(dup.get("input") or {}).get("file"), job=CA.job_brief(dup), duplicate=True)
                if dup.get("waiting_for"):
                    out["waiting_for"], out["note"] = dup["waiting_for"], CA.WAITING_NOTE.get(dup["waiting_for"])
                return self.send_json(out)
            _, q = self.creative.enqueue(st, "process_set", f"process|{dest.name}", {"file": dest.name, "meta": meta})
            out["job"] = q["job"]
            if q.get("waiting_for"):
                out["waiting_for"], out["note"] = q["waiting_for"], q.get("note")
        return self.send_json(out)


def upload_extras(h) -> dict:
    """Optional capture metadata the app sends with an upload (X-Recorded-At, X-Venue, X-Lat/X-Lng, X-Show-Group,
    X-Align-Working-Set). Kept in the inbox meta.json for the pipeline; anything malformed is dropped."""
    out = {}
    ra = (h.get("X-Recorded-At") or "").strip()
    if ra and re.match(r"^\d{4}-\d{2}-\d{2}([T ][0-9:.]+)?(Z|[+-]\d{2}:?\d{2})?$", ra):
        out["recorded_at"] = ra
    for hdr, key in (("X-Venue", "venue"), ("X-Show-Group", "show_group"), ("X-Align-Working-Set", "align_working_set")):
        v = unquote(h.get(hdr) or "").strip()[:200]
        if v:
            out[key] = v
    try:
        lat, lng = float(h.get("X-Lat")), float(h.get("X-Lng"))
        if -90 <= lat <= 90 and -180 <= lng <= 180:
            out["gps"] = {"lat": lat, "lon": lng}
    except (TypeError, ValueError):
        pass
    return out


def default_graph(kb_given: bool, kbdir: Path) -> tuple[Path, Path | None]:
    """(graph.json, id_map). With --kb, that folder's graph.json. Otherwise what db/load.py last loaded
    (comedy/db/punchable/graph_choice.json), else kb/v1 if it exists, else kb/v0: both backends read the same graph."""
    v1 = COMEDY / "kb" / "v1"
    if kb_given:
        return kbdir / "graph.json", None
    choice = COMEDY / "db" / "punchable" / "graph_choice.json"
    try:
        c = json.loads(choice.read_text(encoding="utf-8"))
        return Path(c["graph"]), (Path(c["id_map"]) if c.get("id_map") else None)
    except (OSError, ValueError, KeyError):
        pass
    if (v1 / "graph.json").is_file():
        return v1 / "graph.json", (v1 / "id_map.json") if (v1 / "id_map.json").is_file() else None
    return kbdir / "graph.json", None


# Every do_* is timed and counted by route pattern (api-dev/metrics.py); /api/live is left to pk_live_clients.
for _m in ("do_GET", "do_HEAD", "do_POST", "do_PATCH", "do_DELETE", "do_OPTIONS"):
    setattr(Handler, _m, M.instrument(getattr(Handler, _m)))
# pk_jobs_queued{kind} etc. are read at scrape time from the job store of the primary backend.
M.set_jobs_source(lambda: (lambda cr, kb: cr.stores(kb)["jobs"].items.values() if cr and kb else [])(
    Handler.creative, Handler.kbs.get(Handler.primary)))

def surreal_kb_class():
    import surreal_kb as SK

    class SurrealAppKB(SK.SurrealKB, AppKB):
        @property
        def index_class(self):
            return DBAppIndex

    return SurrealAppKB, SK


def temp_surreal_db(SK, ns: str) -> str:
    """e2e / tests: a fresh database in the temp namespace; stale ones from dead processes are removed."""
    import sdb
    r = sdb.Rpc(None, ns=None, db=None)
    try:
        dbs = (r.last(f"DEFINE NAMESPACE IF NOT EXISTS {ns}; USE NS {ns}; INFO FOR NS;") or {}).get("databases", {})
        for name in dbs:
            m = re.match(r"^api_(\d+)_\d+$", name)
            if m and not _pid_alive(int(m.group(1))):
                r.query(f"USE NS {ns}; REMOVE DATABASE IF EXISTS {name};")
    finally:
        r.close()
    return f"api_{os.getpid()}_{int(time.time())}"


def _pid_alive(pid: int) -> bool:
    try:
        import ctypes
        h = ctypes.windll.kernel32.OpenProcess(0x1000, False, pid)
        if not h:
            return False
        code = ctypes.c_ulong()
        ctypes.windll.kernel32.GetExitCodeProcess(h, ctypes.byref(code))
        ctypes.windll.kernel32.CloseHandle(h)
        return code.value == 259
    except Exception:  # noqa: BLE001
        return True


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--port", type=int, default=DEFAULT_PORT)
    ap.add_argument("--kb", type=Path, default=None, help="folder with decisions.jsonl, inferred.json, KB_REPORT.md (default kb/v0)")
    ap.add_argument("--graph", type=Path, default=None, help="graph.json (default: what db/load.py loaded; else kb/v1; else <kb>/graph.json)")
    ap.add_argument("--id-map", type=Path, default=None, help="kb/v1/id_map.json for v0 ids left in the infer files")
    ap.add_argument("--corpus", type=Path, default=COMEDY / "corpus")
    ap.add_argument("--tapes", type=Path, default=COMEDY / "tapes")
    ap.add_argument("--inbox", type=Path, default=None, help="default: <tapes>/inbox")
    ap.add_argument("--fixtures", type=Path, default=HERE / "fixtures",
                    help="stand-in inferred.json + sets/<set>/analysis.json, used only where the real file is missing")
    ap.add_argument("--no-fixtures", action="store_true", help="show only real AI files")
    ap.add_argument("--backend", choices=["surreal", "files"], default=os.environ.get("PUNCH_BACKEND", "surreal"),
                    help="which backend serves requests by default (?backend=files|surreal overrides per request)")
    ap.add_argument("--ns", default=os.environ.get("PUNCH_SURREAL_NS", "punchable"))
    ap.add_argument("--db", default=os.environ.get("PUNCH_SURREAL_DB", "main"))
    ap.add_argument("--surreal-port", type=int, default=None)
    ap.add_argument("--surreal-temp", action="store_true", default=os.environ.get("PUNCH_SURREAL_TEMP") == "1",
                    help="tests: load this server's kb/corpus into a fresh temp database (namespace punchable_test)")
    ap.add_argument("--host", default="127.0.0.1",
                    help="bind address; anything but loopback needs PK_AUTH=1 (Keycloak tokens)")
    ap.add_argument("--creative-dir", type=Path, default=Path(os.environ.get("PUNCHABLE_CREATIVE_DIR") or COMEDY / "kb" / "creative"),
                    help="storyboards / working sets / venue pins / jobs (append-only jsonl + snapshot)")
    ap.add_argument("--vectors", type=Path, default=Path(os.environ["PK_CACHE_DIR"]) / "kb" if os.environ.get("PK_CACHE_DIR")
                    else HERE.parent / "tools" / "kb" / "cache",
                    help="bge-small vector cache creative/ reads (read-only)")
    ap.add_argument("--jobs", choices=["worker", "inproc"], default=os.environ.get("PK_JOBS", "worker"),
                    help="drafts and ask: queue for the home worker (default), or run in a thread here (local dev)")
    ap.add_argument("--no-creative", action="store_true", help="serve only the original endpoints")
    a = ap.parse_args()
    verifier = AUTH.from_env()
    if a.host not in ("127.0.0.1", "localhost", "::1") and verifier is None:
        raise SystemExit("refusing to listen on a non-loopback address without PK_AUTH=1 (Keycloak tokens)")
    Handler.verifier = verifier
    Handler.origins = tuple(x for x in [os.environ.get("PUBLIC_URL", "").rstrip("/")] if x)
    kbdir = (a.kb or COMEDY / "kb" / "v0").resolve()
    g, idm = default_graph(a.kb is not None, kbdir)
    graph = (a.graph or g).resolve()
    id_map = (a.id_map or idm)
    inbox = (a.inbox or a.tapes / "inbox").resolve()
    fixtures = None if a.no_fixtures else a.fixtures.resolve()
    files_kb = AppKB(kbdir, a.corpus.resolve(), a.tapes.resolve(), inbox, fixtures, graph=graph, id_map=id_map)
    Handler.kbs = {"files": files_kb}
    Handler.primary = a.backend
    t = time.time()
    if a.backend == "surreal":
        cls, SK = surreal_kb_class()
        ns, db = a.ns, a.db
        if a.surreal_temp:
            ns = "punchable_test"
            db = temp_surreal_db(SK, ns)
            sys.path.insert(0, str(HERE.parent / "db"))
            import load as L
            import sdb
            r = sdb.Rpc(a.surreal_port, ns=None, db=None)
            try:
                L.load(r, ns=ns, db=db, graph=graph, kbdir=kbdir, corpus=a.corpus.resolve(), fixtures=fixtures,
                       id_map=id_map, version="temp", reason="--surreal-temp")
            finally:
                r.close()
        skb = cls(kbdir, a.corpus.resolve(), a.tapes.resolve(), inbox, fixtures, graph=graph, id_map=id_map)
        skb.init_surreal(ns, db, a.surreal_port)
        Handler.kbs["surreal"] = skb
        loaded = Path((skb._state().get("load_info") or {}).get("graph") or "")
        if loaded.resolve() != graph:
            print(f"WARNING: SurrealDB {ns}/{db} was loaded from {loaded}, but the files backend reads {graph}. "
                  "?backend=files will differ until both use the same graph (python db/load.py).", flush=True)
    primary = Handler.kbs[Handler.primary]
    if not a.no_creative:
        import creative_api as CR
        Handler.creative = CR.Creative(a.creative_dir.resolve(), a.vectors.resolve() if a.vectors else None,
                                       geocode=os.environ.get("PK_GEOCODE") == "1", jobs_mode=a.jobs)
    idx = primary.app()
    idx.summaries()
    srv = ThreadingHTTPServer((a.host, a.port), Handler)
    srv.daemon_threads = True
    extra = f"\n  db    surreal {primary.ns}/{primary.db} on 127.0.0.1:{primary.port} (gen {primary._snap_gen})" if a.backend == "surreal" else ""
    print(f"punchable api-dev on http://{a.host}:{a.port}/api/health  auth {'on' if verifier else 'off'}  backend {a.backend} ({len(idx.v.jokes)} jokes, {len(idx.sets)} sets, "
          f"{len(idx.v.decisions)} decisions, warmed in {time.time() - t:.1f}s)\n  graph {graph}\n  kb    {kbdir}\n  inbox {inbox}{extra}\n"
          f"  ai    analysis {ai_status(primary)['analysis']['real']} real / {ai_status(primary)['analysis']['fixture']} fixture sets; inferred: "
          f"{(primary.ai_index.get('inferred') or {}).get('source')}", flush=True)
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
