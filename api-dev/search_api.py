"""Search over everything, and "is this like anything?" (2026-10-28).

Routes (server.py):
    GET /api/search?q=<words>[&limit=20]    bits and taped set moments that match, best first. Words match a bit's
                                            title, its typed idea (joke.summary), every version (unit.text) and every
                                            transcribed moment of every set (performance.text); with the model,
                                            meaning matches too (bge-small vectors over the same four texts).
    GET /api/jokes/<id>/similar[&limit=8]   where he did this bit (performed_as edges, confirmed or proposed) and what
                                            else sounds like it: other bits, and taped moments not linked to it (a
                                            version he forgot he did, a line that belongs with it).

Vectors. Units were embedded at the load (db/load.py). Typed ideas (jokes with no unit) and set moments were not, and
the hosted API image had no model, so its search ran on words alone ("hybrid search: no query vector" in the log,
16 times the week of 2026-10-28). Now:
  * api/Dockerfile ships the bge-small ONNX model (PK_BGE_DIR) and onnxruntime; `make_embedder()` finds it.
  * `Sweeper` fills `embedding` + `embedded_sha` on every joke without units and every performance with text, and
    `embedding` on any unit that lacks one: at start and every PK_VEC_SWEEP_S seconds (60). A new jot or an edited
    one is embedded within a minute with no write-path hook to forget. Idempotent: a row is embedded again only when
    the sha256 of its text differs from the one recorded next to the vector.
  * db/schema_extra.surql defines the fields, the full-text indexes (joke.summary, performance.text) and the HNSW
    indexes; the API applies it at start (db/load.py apply_extra_schema), no reload.

PK_EMBEDDER=hash: a deterministic word-hash embedder for the tests and a PC without the model. Never the server.
Nothing here is a verdict: candidates for his eyes, ranked.
"""
from __future__ import annotations

import hashlib
import math
import os
import re
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
DESK = HERE.parent
for p in (DESK / "db", DESK / "tools" / "explorer", DESK):
    if str(p) not in sys.path:
        sys.path.insert(0, str(p))
import sdb  # noqa: E402
from kb import bucket  # noqa: E402

DIM = 384
QUERY_PREFIX = "Represent this sentence for searching relevant passages: "
BATCH = 16
MAX_LIMIT = 50
SNIPPET = 240


# ------------------------------------------------------------------------------------------------- embedders
class HashEmbedder:
    """Deterministic 384-d vectors from the words (tests, dev): texts that share words are close. Not the model."""
    name = "hash"

    def embed(self, texts: list[str], query: bool = False) -> list[list[float]]:
        out = []
        for t in texts:
            v = [0.0] * DIM
            for w in re.findall(r"[a-z0-9']+", (t or "").lower()):
                h = int(hashlib.sha1(w.encode("utf-8")).hexdigest()[:8], 16)
                v[h % DIM] += 1.0
                v[(h >> 9) % DIM] += 0.5
            n = math.sqrt(sum(x * x for x in v)) or 1.0
            out.append([x / n for x in v])
        return out


class ModelEmbedder:
    """engine/embed.py (onnxruntime + tokenizers) over the bge-small folder: the same vectors db/load.py stored for
    the units (CLS pooling, L2-normalised), so a query vector lands in the same space."""
    name = "bge-small-en-v1.5"

    def __init__(self, bge_dir: Path):
        from engine.embed import Embedder
        self._e = Embedder(Path(bge_dir))

    def embed(self, texts: list[str], query: bool = False) -> list[list[float]]:
        xs = [(QUERY_PREFIX if query else "") + (t or " ") for t in texts]
        return [[float(x) for x in row] for row in self._e.embed(xs)]


def bge_dir() -> Path | None:
    """PK_BGE_DIR, else <PK_CACHE_DIR>/models/bge-small-en-v1.5 (tools/kb/fetch_bge.py); only a folder with the model."""
    cands = [os.environ.get("PK_BGE_DIR")]
    if os.environ.get("PK_CACHE_DIR"):
        cands.append(os.path.join(os.environ["PK_CACHE_DIR"], "models", "bge-small-en-v1.5"))
    for c in cands:
        if c and (Path(c) / "onnx" / "model.onnx").is_file() and (Path(c) / "tokenizer.json").is_file():
            return Path(c)
    return None


def make_embedder(log=print):
    """The embedder this process can run, or None (search then runs on words alone, and says so once)."""
    if os.environ.get("PK_EMBEDDER") == "hash":
        log("vectors: PK_EMBEDDER=hash (word-hash vectors: tests and dev only)")
        return HashEmbedder()
    d = bge_dir()
    if d is None:
        log("vectors: no bge-small model here (PK_BGE_DIR); search runs on words alone")
        return None
    try:
        e = ModelEmbedder(d)
        e.embed(["warm up"])
    except Exception as ex:  # noqa: BLE001  a broken model must not stop the API
        log(f"vectors: the model at {d} did not load ({type(ex).__name__}: {ex}); search runs on words alone")
        return None
    log(f"vectors: {e.name} from {d}")
    return e


def sha(text: str) -> str:
    """sha256 hex of the UTF-8 text: the same value SurrealDB's crypto::sha256() gives, so the pending query can
    compare without a round trip."""
    return hashlib.sha256((text or "").encode("utf-8")).hexdigest()


# --------------------------------------------------------------------------------------------------- sweeper
def pending(r: sdb.Rpc, v) -> dict[str, list[dict]]:
    """Rows whose text has no vector yet, or a vector made from an older text. `v` (the view) says which jokes have
    no unit: only those carry their text in `summary` (a joke with units is searched through them)."""
    out: dict[str, list[dict]] = {"joke": [], "performance": [], "unit": []}
    st = r.last("SELECT loading FROM ONLY meta:state;") if r.last("RETURN record::exists(meta:state);") else None
    if st and sdb.strip_none(st).get("loading"):
        return out
    jids = [sdb.node_rid(j["id"]) for j in v.jokes
            if j["id"] not in v.joke_units and (j.get("summary") or "").strip()]
    if jids:
        out["joke"] = r.last("SELECT id, summary AS text FROM joke WHERE id IN $ids AND summary != NONE "
                             "AND (embedded_sha = NONE OR embedded_sha != crypto::sha256(summary));", {"ids": jids})
    out["performance"] = r.last("SELECT id, text FROM performance WHERE text != NONE AND string::len(text) > 0 "
                                "AND (embedded_sha = NONE OR embedded_sha != crypto::sha256(text));")
    out["unit"] = r.last("SELECT id, text FROM unit WHERE embedding = NONE AND text != NONE AND string::len(text) > 0;")
    return {k: [x for x in rows if (x.get("text") or "").strip()] for k, rows in out.items()}


def sweep(r: sdb.Rpc, emb, v) -> dict[str, int]:
    """Embed and store every pending row; returns how many per table (all zero on a quiet pass)."""
    done: dict[str, int] = {}
    for table, rows in pending(r, v).items():
        n = 0
        for i in range(0, len(rows), BATCH):
            chunk = rows[i:i + BATCH]
            vecs = emb.embed([x["text"] for x in chunk])
            if table == "unit":
                r.query("FOR $x IN $rows { LET $id = $x.id; UPDATE $id SET embedding = $x.v; };",
                        {"rows": [{"id": x["id"], "v": vec} for x, vec in zip(chunk, vecs)]})
            else:
                r.query("FOR $x IN $rows { LET $id = $x.id; UPDATE $id SET embedding = $x.v, embedded_sha = $x.s; };",
                        {"rows": [{"id": x["id"], "v": vec, "s": sha(x["text"])} for x, vec in zip(chunk, vecs)]})
            n += len(chunk)
        done[table] = n
    return done


class Sweeper:
    """Keeps the vectors complete on its own connection: a pass at start, then every PK_VEC_SWEEP_S seconds."""

    def __init__(self, kb, embedder, log=print, every: float | None = None):
        self.kb, self.emb, self.log = kb, embedder, log
        self.every = float(every if every is not None else os.environ.get("PK_VEC_SWEEP_S", "60"))
        self.runs, self.last, self.error, self.last_at = 0, None, None, None

    def start(self):
        threading.Thread(target=self._loop, daemon=True, name="vector-sweep").start()
        return self

    def once(self) -> dict[str, int]:
        r = sdb.Rpc(self.kb.port, self.kb.ns, self.kb.db)
        try:
            return sweep(r, self.emb, self.kb.view())
        finally:
            r.close()

    def _loop(self):
        while True:
            try:
                self.last = self.once()
                self.error = None
                if sum(self.last.values()):
                    self.log(f"vectors: embedded {self.last}")
            except Exception as e:  # noqa: BLE001  a failed pass is retried on the next one
                self.error = f"{type(e).__name__}: {str(e)[:200]}"
                self.log(f"vectors: sweep failed ({self.error})")
            self.runs += 1
            self.last_at = time.time()
            time.sleep(self.every)

    def status(self) -> dict:
        return {"embedder": getattr(self.emb, "name", None), "runs": self.runs, "last": self.last, "error": self.error,
                "every_s": self.every}


# ---------------------------------------------------------------------------------------------------- search
def _qvec(emb, q: str):
    if emb is None:
        return None
    try:
        return emb.embed([q], query=True)[0]
    except Exception as e:  # noqa: BLE001
        sys.stderr.write(f"search: no query vector ({type(e).__name__}: {e}); words only\n")
        return None


def search_all(r: sdb.Rpc, emb, q: str, limit: int = 20, include_hidden: bool = False) -> dict:
    """Fused search over unit text, joke title, joke summary and performance text (BM25, snowball-stemmed), plus the
    three vector indexes when a query vector exists; search::rrf fuses each family. Returns raw hits:
    {"hits": [(unit or joke id, score)], "moments": [(performance id, score)], "vectors": bool}. The caller folds
    unit ids into their jokes with the view (fold_jokes)."""
    qvec = _qvec(emb, q)
    hid = "" if include_hidden else " AND hidden != true"
    k = max(20, min(200, limit * 3))
    legs_j, legs_p = ["$ft", "$jt", "$js"], ["$pt"]
    sql = (f"LET $ft = SELECT id, search::score(1) AS score FROM unit WHERE text @1@ $q{hid} ORDER BY score DESC LIMIT {k};"
           f"LET $jt = SELECT id, search::score(1) AS score FROM joke WHERE title @1@ $q{hid} ORDER BY score DESC LIMIT {k};"
           f"LET $js = SELECT id, search::score(1) AS score FROM joke WHERE summary @1@ $q{hid} ORDER BY score DESC LIMIT {k};"
           f"LET $pt = SELECT id, search::score(1) AS score FROM performance WHERE text @1@ $q ORDER BY score DESC LIMIT {k};")
    if qvec is not None:
        sql += (f"LET $vs = SELECT id, vector::distance::knn() AS distance FROM unit WHERE embedding <|{k},150|> $qvec{hid} ORDER BY distance;"
                f"LET $jv = SELECT id, vector::distance::knn() AS distance FROM joke WHERE embedding <|{k},150|> $qvec{hid} ORDER BY distance;"
                f"LET $pv = SELECT id, vector::distance::knn() AS distance FROM performance WHERE embedding <|{k},150|> $qvec ORDER BY distance;")
        legs_j += ["$vs", "$jv"]
        legs_p += ["$pv"]
    sql += (f"RETURN search::rrf([{', '.join(legs_j)}], {k}, 60);"
            f"RETURN search::rrf([{', '.join(legs_p)}], {k}, 60);")
    res = r.query(sql, {"q": q, "qvec": qvec})
    rows_j, rows_p = res[-2]["result"] or [], res[-1]["result"] or []
    return {"hits": [(sdb.rid_str(x["id"]), x.get("score")) for x in rows_j if isinstance(x, dict) and x.get("id") is not None],
            "moments": [(sdb.rid_str(x["id"]), x.get("score")) for x in rows_p if isinstance(x, dict) and x.get("id") is not None],
            "vectors": qvec is not None}


def fold_jokes(hits: list[tuple[str, float | None]], v) -> list[str]:
    """Unit and joke hits, best first -> joke ids, each once, only jokes the view knows (hidden ones are already out)."""
    out: list[str] = []
    for nid, _ in hits:
        jid = nid if nid.startswith("joke:") else v.unit_joke.get(nid)
        if jid and jid in v.nodes and jid not in out:
            out.append(jid)
    return out


def newest_text(v, jid: str) -> str:
    """What the bit says now: its newest version, else the typed idea."""
    units = []
    for u, _ in v.joke_units.get(jid, []):
        n = v.nodes.get(u) or {}
        if (n.get("text") or "").strip() and not n.get("hidden"):
            units.append((n.get("date") or "", n["text"]))
    if units:
        units.sort(key=lambda x: x[0], reverse=True)
        return units[0][1]
    return ((v.nodes.get(jid) or {}).get("summary") or "").strip()


def similar(r: sdb.Rpc, emb, v, jid: str, limit: int = 8, include_hidden: bool = False) -> dict:
    """Vectors near this bit's newest text (a passage vector, symmetric with the stored ones): other bits, through
    their units or their typed idea, and taped moments. Raw: {"bits": [(jid, distance)], "moments": [(pid, distance)],
    "vectors": bool, "text": str}. Without a model: the word search over the bit's first words."""
    text = newest_text(v, jid)
    own = {u for u, _ in v.joke_units.get(jid, [])}
    if not text:
        return {"bits": [], "moments": [], "vectors": False, "text": ""}
    if emb is None:
        res = search_all(r, None, " ".join(text.split()[:12]), limit * 2, include_hidden)
        bits = [(j, None) for j in fold_jokes(res["hits"], v) if j != jid][:limit]
        return {"bits": bits, "moments": res["moments"][:limit], "vectors": False, "text": text}
    vec = emb.embed([text])[0]
    hid = "" if include_hidden else " AND hidden != true"
    k = max(20, min(200, limit * 4))
    res = r.query(
        f"SELECT id, vector::distance::knn() AS d FROM unit WHERE embedding <|{k},150|> $v{hid} ORDER BY d;"
        f"SELECT id, vector::distance::knn() AS d FROM joke WHERE embedding <|{k},150|> $v{hid} ORDER BY d;"
        f"SELECT id, vector::distance::knn() AS d FROM performance WHERE embedding <|{k},150|> $v ORDER BY d;",
        {"v": vec})
    best: dict[str, float] = {}
    for x in (res[0]["result"] or []) + (res[1]["result"] or []):
        nid = sdb.rid_str(x["id"])
        if nid in own:
            continue
        j = nid if nid.startswith("joke:") else v.unit_joke.get(nid)
        if not j or j == jid or j not in v.nodes:
            continue
        d = float(x.get("d") or 0.0)
        if j not in best or d < best[j]:
            best[j] = d
    bits = sorted(best.items(), key=lambda kv: kv[1])[:limit]
    moments = [(sdb.rid_str(x["id"]), float(x.get("d") or 0.0)) for x in (res[2]["result"] or [])]
    return {"bits": bits, "moments": moments, "vectors": True, "text": text}


# -------------------------------------------------------------------------------------- payloads for the app
def _words(q: str) -> list[str]:
    return [w for w in re.split(r"\s+", (q or "").lower().strip()) if w]


def file_search(v, q: str) -> dict:
    """The files backend (dev, tests): every word in the joke's text (kb.View.search) or in a moment's text."""
    words = _words(q)
    moments = []
    if words:
        for nid, n in v.nodes.items():
            if n.get("type") == "performance":
                hay = (n.get("text") or "").lower()
                if all(w in hay for w in words):
                    moments.append((nid, None))
    return {"hits": [(j, None) for j in v.search(q)], "moments": moments, "vectors": False}


def linked_joke(v, pid: str) -> str | None:
    for eid in v.adj.get(pid, []):
        e = v.edges.get(eid) or {}
        if e.get("kind") == "performed_as" and bucket(e.get("status")) != "rejected":
            other = e["src"] if e.get("dst") == pid else e.get("dst")
            if v.ntype(other) == "joke":
                return other
    return None


def moment_item(v, pid: str, score=None) -> dict | None:
    n = v.nodes.get(pid)
    if not n or n.get("type") != "performance":
        return None
    s = v.nodes.get(n.get("set")) or {}
    j = linked_joke(v, pid)
    return {"id": pid, "set": n.get("set"), "set_title": s.get("title"), "venue": s.get("venue"),
            "date": s.get("show_date") or s.get("date"), "start_ms": n.get("start_ms"), "end_ms": n.get("end_ms"),
            "text": (n.get("text") or "").strip()[:SNIPPET], "joke": j,
            "joke_title": (v.nodes.get(j) or {}).get("title") if j else None, "score": score}


def _snippet(text: str, words: list[str]) -> str | None:
    t = (text or "").strip()
    if not t:
        return None
    low = t.lower()
    pos = min((low.find(w) for w in words if w in low), default=-1)
    if pos < 0:
        return None
    start = max(0, low.rfind(" ", 0, max(0, pos - 60)) + 1)
    return ("…" if start else "") + t[start:start + SNIPPET].strip()


def bit_why(v, jid: str, words: list[str]) -> str | None:
    """The line that matched, for the row under the bit: a version or the typed idea that holds a query word."""
    n = v.nodes.get(jid) or {}
    cands = []
    for uid, _ in v.joke_units.get(jid, []):
        u = v.nodes.get(uid) or {}
        if not u.get("hidden"):
            cands.append((u.get("date") or "", u.get("text") or ""))
    cands.sort(key=lambda x: x[0], reverse=True)
    for _, t in cands + [("", n.get("summary") or "")]:
        s = _snippet(t, words)
        if s:
            return s
    return None


def _summaries_by_id(a) -> dict[str, dict]:
    return {x["id"]: {k: x[k] for k in x if k != "label_ids"} for x in a.summaries()}


def search_payload(kb, a, q: str, limit: int, include_hidden: bool) -> dict:
    """GET /api/search."""
    v = a.v
    words = _words(q)
    if getattr(kb, "backend", None) == "surreal" and hasattr(kb, "search_all"):
        res = kb.search_all(q, limit, include_hidden)
    else:
        res = file_search(v, q)
    by_id = _summaries_by_id(a)
    bits = []
    for jid in fold_jokes(res["hits"], v):
        it = by_id.get(jid)
        if it is None:
            continue
        bits.append({**it, "why": bit_why(v, jid, words)})
        if len(bits) >= limit:
            break
    moments = [m for m in (moment_item(v, pid, sc) for pid, sc in res["moments"]) if m][:limit]
    return {"query": q, "bits": bits, "moments": moments, "vectors": bool(res.get("vectors"))}


def similar_payload(kb, a, jid: str, limit: int, include_hidden: bool) -> tuple[int, dict]:
    """GET /api/jokes/<id>/similar: {"performed": [moments linked to it], "like": {"bits": [...], "moments": [...]},
    "vectors": bool}. A moment already linked to this bit is never also "like" it."""
    v = a.v
    if v.ntype(jid) != "joke":
        return 404, {"error": "no such joke"}
    performed = []
    for e in v.joke_edges(jid):
        if e.get("kind") != "performed_as" or bucket(e.get("status")) == "rejected":
            continue
        pid = e["src"] if v.ntype(e.get("src")) == "performance" else e.get("dst")
        m = moment_item(v, pid)
        if m:
            m["status"] = bucket(e.get("status"))
            performed.append(m)
    performed.sort(key=lambda m: (m.get("date") or ""), reverse=True)
    linked = {m["id"] for m in performed}
    if getattr(kb, "backend", None) == "surreal" and hasattr(kb, "similar_vectors"):
        raw = kb.similar_vectors(v, jid, limit, include_hidden)
    else:
        text = newest_text(v, jid)
        res = file_search(v, " ".join(text.split()[:6])) if text else {"hits": [], "moments": [], "vectors": False}
        raw = {"bits": [(j, None) for j in fold_jokes(res["hits"], v) if j != jid][:limit],
               "moments": res["moments"], "vectors": False, "text": text}
    by_id = _summaries_by_id(a)
    bits = []
    for j, d in raw["bits"]:
        it = by_id.get(j)
        if it is not None:
            bits.append({**it, "distance": d, "why": (newest_text(v, j) or it.get("summary") or "")[:SNIPPET] or None})
    moments = [m for m in (moment_item(v, pid, d) for pid, d in raw["moments"] if pid not in linked) if m][:limit]
    return 200, {"performed": performed, "like": {"bits": bits[:limit], "moments": moments}, "vectors": bool(raw.get("vectors")),
                 "text": (raw.get("text") or "")[:SNIPPET]}
