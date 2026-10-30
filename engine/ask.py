"""Ask your material (the "ask" pattern).

    python -m engine.ask "what have I written about my dad"
    from engine.ask import ask; r = ask("...")

1 plan        the LLM turns the question into search queries, label ids, a year range,
              performed/never-performed, named jokes, by-year ordering (validated)
2 retrieve    hybrid: SQLite FTS5 (bm25) + bge-small vectors, fused by reciprocal rank,
              plus graph routes (has_label edges, a named joke's units and performances),
              over written units, performances, set transcripts and voice notes;
              decisions.jsonl is replayed so rejected links are not used
3 answer      the LLM answers ONLY from the retrieved sources, citing unit/perf ids
4 verify      every quote must be found verbatim in the source it cites; anything that
              fails is removed and flagged; uncited "double-quoted" prose is checked too
5 not found   if the sources do not contain the answer, it says so
"""
from __future__ import annotations

import os
import argparse
import json
import re
import sqlite3
import sys
import time

import numpy as np

from .config import Config
from .kbstore import KB, REJECTED
from .providers import Router
from .verify import find_verbatim, normalize_ids, strip_unverified_inline_quotes

COMIC = os.environ.get("PK_COMIC_NAME", "the comic")  # how the AI prompts name you; PK_COMIC_NAME
COMIC_FIRST = os.environ.get("PK_COMIC_FIRST", COMIC)

MAX_SOURCES = 32
PER_JOKE = 3
SNIPPET = 900
TX_WINDOW = 8


def _obj(props, required=None):
    return {"type": "object", "properties": props, "required": required or list(props), "additionalProperties": False}


S_PLAN = _obj({
    "search_queries": {"type": "array", "items": {"type": "string"}},
    "label_ids": {"type": "array", "items": {"type": "string"}},
    "year_min": {"type": "integer"}, "year_max": {"type": "integer"},
    "performed": {"type": "string", "enum": ["any", "performed", "never"]},
    "joke_titles": {"type": "array", "items": {"type": "string"}},
    "by_year": {"type": "boolean"}})
S_ANSWER = _obj({
    "found": {"type": "boolean"}, "answer": {"type": "string"},
    "citations": {"type": "array", "items": _obj({"source_id": {"type": "string"}, "quote": {"type": "string"},
                                                  "note": {"type": "string"}})},
    "not_found": {"type": "string"}})

SYS = ("You answer questions about stand-up comic " + COMIC + "'s own material (his written joke docs, recorded sets and "
       "voice notes). Use ONLY the numbered SOURCES given. Never use outside knowledge and never invent material. "
       "Quotes must be copied character for character from the source they cite.")


class Corpus:
    def __init__(self, cfg: Config, kb: KB, embedder):
        self.cfg, self.kb = cfg, kb
        v = kb.view
        self.docs: dict[str, dict] = {}
        perf_spans: dict[str, list] = {}
        sets = {n["id"]: n for n in v.nodes.values() if n.get("type") == "set"}
        joke_of_perf = {}
        for e in v.edges.values():
            if e.get("kind") == "performed_as" and e.get("status") not in REJECTED:
                joke_of_perf.setdefault(e["src"], e["dst"])
        for nid, n in v.nodes.items():
            t = n.get("type")
            if t == "unit" and n.get("text"):
                self.docs[nid] = {"id": nid, "kind": "memo" if n.get("source") == "memo" else "written",
                                  "text": n["text"], "date": (n.get("date") or "")[:10],
                                  "title": n.get("doc_title") or "", "joke": v.unit_joke.get(nid)}
            elif t == "performance" and n.get("text"):
                s = sets.get(n.get("set"), {})
                self.docs[nid] = {"id": nid, "kind": "performed", "text": n["text"], "date": s.get("date", ""),
                                  "title": s.get("venue") or s.get("title", ""), "joke": joke_of_perf.get(nid),
                                  "start_ms": n.get("start_ms")}
                perf_spans.setdefault(n.get("set"), []).append((n.get("start_ms") or 0, n.get("end_ms") or 0))
            elif t == "joke" and nid not in kb.rejected_nodes:
                self.docs[nid] = {"id": nid, "kind": "joke", "text": f"{n.get('title', '')}. {n.get('summary', '')}",
                                  "date": n.get("first_seen", ""), "title": n.get("title", ""), "joke": nid,
                                  "quotable": False}
        # set transcripts: windows not already covered by a performance span
        for root in {cfg.p("corpus_dir") / "sets", cfg.p("out_corpus_dir") / "sets"}:
            if not root.exists():
                continue
            for d in sorted(root.iterdir()):
                tp = d / "transcript.json"
                if not tp.exists():
                    continue
                m = re.match(r"(\d{4})-?(\d{2})-?(\d{2})_(.+)$", d.name)
                date = f"{m.group(1)}-{m.group(2)}-{m.group(3)}" if m else ""
                set_id = f"set:{date}_{m.group(4)}" if m else f"set:{d.name}"
                venue = (sets.get(set_id) or {}).get("venue") or ""
                if not venue and (d / "meta.json").exists():
                    try:
                        venue = json.loads((d / "meta.json").read_text(encoding="utf-8")).get("venue_guess", "")
                    except ValueError:
                        pass
                segs = json.loads(tp.read_text(encoding="utf-8")).get("segments", [])
                spans = perf_spans.get(set_id, [])
                for i in range(0, len(segs), TX_WINDOW):
                    w = segs[i:i + TX_WINDOW]
                    a, b = int(w[0]["start"] * 1000), int(w[-1]["end"] * 1000)
                    covered = sum(max(0, min(b, e) - max(a, s)) for s, e in spans)
                    if covered > 0.5 * (b - a):
                        continue
                    tid = f"tx:{d.name}@{a}"
                    self.docs[tid] = {"id": tid, "kind": "transcript", "text": " ".join(s["text"].strip() for s in w),
                                      "date": date, "title": venue or d.name, "joke": None, "start_ms": a}
        self.ids = list(self.docs)
        # full text
        self.db = sqlite3.connect(":memory:")
        self.db.execute("create virtual table f using fts5(id unindexed, title, text, tokenize='porter unicode61')")
        self.db.executemany("insert into f values (?,?,?)",
                            [(d["id"], d["title"], d["text"]) for d in self.docs.values()])
        # vectors (units reuse tools/kb's cache; everything else is embedded once and cached)
        from .embed import VectorIndex
        self.embedder = embedder
        vi = VectorIndex(cfg.p("vectors_dir"), cfg.p("kb_dir") / "engine_cache", embedder)
        got = vi.ensure({i: d["text"][:2000] for i, d in self.docs.items() if i not in vi.row})
        self.M = np.stack([vi.m[vi.row[i]] if i in vi.row else got[i] for i in self.ids]).astype(np.float32)
        self.performed = kb.performed_jokes

    # ---- retrieval primitives -------------------------------------------
    def fts(self, q: str, k=50) -> list[str]:
        terms = [t for t in re.findall(r"[A-Za-z0-9']+", q.lower()) if len(t) > 2 and t not in STOP]
        if not terms:
            return []
        expr = " OR ".join('"' + t.replace('"', "") + '"' for t in terms)
        try:
            return [r[0] for r in self.db.execute("select id from f where f match ? order by bm25(f) limit ?", (expr, k))]
        except sqlite3.OperationalError:
            return []

    def vec(self, q: str, k=50) -> list[str]:
        s = self.M @ self.embedder.query(q)
        return [self.ids[i] for i in np.argsort(-s)[:k]]

    def sim(self, q: str) -> dict[str, float]:
        s = self.M @ self.embedder.query(q)
        return {self.ids[i]: float(s[i]) for i in range(len(self.ids))}


STOP = set("the and for with that this what have has had about all any are was were you your my mine our from into "
           "jokes joke bits bit wrote written write ever every which where when who how did does not never".split())


def ask(question: str, cfg: Config | None = None, router: Router | None = None, embedder=None, kb: KB | None = None,
        corpus: Corpus | None = None) -> dict:
    t_all = time.perf_counter()
    cfg = cfg or Config()
    router = router or Router(cfg)
    if embedder is None:
        from .embed import Embedder
        embedder = Embedder(cfg.p("bge_dir"))
    kb = kb or KB(cfg)
    t0 = time.perf_counter()
    corpus = corpus or Corpus(cfg, kb, embedder)
    timings = {"index_s": round(time.perf_counter() - t0, 2)}
    n0 = len(router.calls)
    run_id = "ask-" + time.strftime("%Y%m%d-%H%M%S")

    # 1 plan --------------------------------------------------------------
    labels = {n["id"]: n.get("name", "") for n in kb.view.labels}
    t0 = time.perf_counter()
    plan = {"search_queries": [question], "label_ids": [], "year_min": 0, "year_max": 0, "performed": "any",
            "joke_titles": [], "by_year": False}
    try:
        c = router.complete("ask_plan", [
            {"role": "system", "content": "You turn a comic's question about his own joke archive into a search plan."},
            {"role": "user", "content":
                f"QUESTION: {question}\n\n"
                "search_queries: 1-4 short searches with the words his jokes would actually contain (include synonyms, "
                "spelled the way he would say it). label_ids: only if the question asks for a device, structure, form or "
                "topic from this list, else []. year_min/year_max: 0 when not asked. performed: 'never' if he asks for "
                "material never performed, 'performed' if only performed, else 'any'. joke_titles: names of specific "
                "bits he mentions (e.g. 'airport bit'), else []. by_year: true if he wants it ordered or grouped by year.\n\n"
                "LABEL IDS: " + ", ".join(f"{k} ({v})" for k, v in labels.items())}],
            S_PLAN, run_id=run_id, request_path=True)
        plan.update(c.data)
    except Exception as e:  # planning is an aid; retrieval still runs on the raw question
        plan["plan_error"] = str(e)[:200]
    plan["label_ids"] = [x for x in plan["label_ids"] if x in labels]
    ql = question.lower()
    for lid, name in labels.items():  # a label named in the question is always used
        n = (name or "").lower().split("(")[0].split("/")[0].strip()
        if len(n) > 3 and re.search(r"\b" + re.escape(n) + r"\b", ql) and lid not in plan["label_ids"]:
            plan["label_ids"].append(lid)
    years = [int(y) for y in re.findall(r"\b(19[89]\d|20[0-4]\d)\b", question)]
    if years and not plan["year_min"] and not plan["year_max"]:
        plan["year_min"], plan["year_max"] = min(years), max(years)
    if plan["year_min"] and not re.search(r"\b(19|20)\d\d\b", question):
        plan["year_min"] = plan["year_max"] = 0  # a year the question never named is the planner's guess
    if question not in plan["search_queries"]:
        plan["search_queries"] = plan["search_queries"][:4] + [question]
    timings["plan_s"] = round(time.perf_counter() - t0, 2)

    # 2 retrieve ----------------------------------------------------------
    t0 = time.perf_counter()
    score: dict[str, float] = {}
    route: dict[str, set] = {}

    def add(i, s, why):
        score[i] = score.get(i, 0.0) + s
        route.setdefault(i, set()).add(why)

    for q in plan["search_queries"]:
        for r, i in enumerate(corpus.fts(q)):
            add(i, 1 / (60 + r), "fts")
        for r, i in enumerate(corpus.vec(q)):
            add(i, 1 / (60 + r), "vector")
    qsim = corpus.sim(question)
    v = kb.view
    focus = set()
    for lid in plan["label_ids"]:
        members = set()
        for e in v.edges.values():
            if e.get("kind") == "has_label" and e.get("dst") == lid and not kb.is_rejected_edge(e):
                members.add(e["src"])
        docs = [d for d in corpus.docs.values() if d["id"] in members or (d.get("joke") in members)]
        docs.sort(key=lambda d: -qsim.get(d["id"], 0))
        for r, d in enumerate(docs):
            add(d["id"], 0.5 + 0.1 * qsim.get(d["id"], 0), "label:" + lid)  # membership dominates
    for title in plan["joke_titles"]:
        jd = [d for d in corpus.docs.values() if d["kind"] == "joke"]
        js = corpus.sim(title)
        tw = set(re.findall(r"[a-z]+", title.lower())) - STOP
        jd.sort(key=lambda d: -(js.get(d["id"], 0) + 0.2 * len(tw & set(re.findall(r"[a-z]+", d["title"].lower())))))
        for d in jd[:1]:
            focus.add(d["id"])
            for x in corpus.docs.values():
                if x.get("joke") == d["id"]:
                    add(x["id"], 1.0, "named joke")

    def keep(d):
        y = int(d["date"][:4]) if d.get("date", "")[:4].isdigit() else None
        if plan["year_min"] and (y is None or y < plan["year_min"]):
            return False
        if plan["year_max"] and (y is None or y > plan["year_max"]):
            return False
        if plan["performed"] == "never":
            if d["kind"] not in ("written", "memo", "joke"):
                return False
            if d.get("joke") and d["joke"] in corpus.performed:
                return False
        if plan["performed"] == "performed":
            if d["kind"] in ("written", "memo") and not (d.get("joke") in corpus.performed):
                return False
        return True

    ranked = [i for i in sorted(score, key=lambda i: -score[i]) if keep(corpus.docs[i])]
    picked, per = [], {}
    for i in ranked:
        d = corpus.docs[i]
        j = d.get("joke") or i
        cap = 40 if j in focus else PER_JOKE
        if per.get(j, 0) >= cap:
            continue
        per[j] = per.get(j, 0) + 1
        picked.append(i)
        if len(picked) >= (48 if focus else MAX_SOURCES):
            break
    if plan["by_year"]:
        picked.sort(key=lambda i: corpus.docs[i].get("date") or "9999")
    timings["retrieve_s"] = round(time.perf_counter() - t0, 2)

    # 3 answer ------------------------------------------------------------
    t0 = time.perf_counter()
    lines = []
    for i in picked:
        d = corpus.docs[i]
        jt = kb.title(d["joke"]) if d.get("joke") and d["joke"] != i else ""
        head = {"written": "written doc", "memo": "voice note", "performed": "performed live",
                "transcript": "set transcript", "joke": "joke card (title + summary written by the system)"}[d["kind"]]
        txt = d["text"] if len(d["text"]) <= SNIPPET else d["text"][:SNIPPET] + " [...]"
        lines.append(f"[{i}] {head} | {d.get('date') or 'undated'} | {d.get('title', '')}"
                     + (f" | joke: {jt}" if jt else "") + f"\n{txt}")
    result = {"question": question, "plan": plan, "run_id": run_id, "sources": picked}
    if not picked:
        result.update({"found": False, "answer": "Nothing in your material matches this.", "citations": [],
                       "removed": [], "timings": timings})
        return result
    c = router.complete("ask_answer", [
        {"role": "system", "content": SYS},
        {"role": "user", "content":
            f"QUESTION: {question}\n\nSOURCES:\n\n" + "\n\n".join(lines) + "\n\n"
            "Answer from these sources only. Be complete: list every source that is relevant (not just a sample), "
            "grouped sensibly" + (" and ordered by year" if plan["by_year"] else "") + ". Cite source ids in square "
            "brackets in the answer, one short line per item (plain text, no tables, no quotes in the answer: quotes go in "
            "citations only). Read the question literally (e.g. 'my dad' means his father, not him being a dad). "
            "citations: one per relevant source, quote = a short exact line from it. "
            "Sources that are off-topic must be ignored. If the sources do not answer the question, set found=false, "
            "say so plainly in answer, and say what is missing in not_found."}],
        S_ANSWER, run_id=run_id, request_path=True)
    data = c.data
    timings["answer_s"] = round(time.perf_counter() - t0, 2)

    # 4 verify ------------------------------------------------------------
    kept, removed = [], []
    for cit in data["citations"]:
        sid = normalize_ids(cit["source_id"]).strip().strip("[]")
        d = corpus.docs.get(sid)
        if sid not in picked or d is None:
            removed.append(dict(cit, why="cites a source that was not retrieved"))
            continue
        exact = find_verbatim(cit["quote"], d["text"])
        if exact is None:
            removed.append(dict(cit, why="quote not found verbatim in " + sid))
            kept.append({"source_id": sid, "quote": "", "note": cit["note"], "kind": d["kind"], "date": d["date"],
                         "title": d["title"], "flag": "quote removed"})
        else:
            kept.append({"source_id": sid, "quote": exact, "note": cit["note"], "kind": d["kind"], "date": d["date"],
                         "title": d["title"], "joke": kb.title(d["joke"]) if d.get("joke") else None})
    ans = data["answer"]
    if not kept:  # the model put everything in the prose: take the ids it cited, without quotes
        for sid in dict.fromkeys(normalize_ids(x) for x in re.findall(r"(?:unit|perf|tx|joke):[^\]\s,;)'\"]+", ans)):
            d = corpus.docs.get(sid)
            if d and sid in picked:
                kept.append({"source_id": sid, "quote": "", "note": "cited in the answer", "kind": d["kind"],
                             "date": d["date"], "title": d["title"]})
    for sid in set(re.findall(r"(?:unit|perf|tx|joke):[^\]\s,;)]+", ans)):
        if normalize_ids(sid) in corpus.docs:
            ans = ans.replace(sid, normalize_ids(sid))
    answer, inline = strip_unverified_inline_quotes(ans, [corpus.docs[i]["text"] for i in picked])
    removed += [{"source_id": None, "quote": q, "why": "quoted in the answer but not verbatim in any source"} for q in inline]
    bad_ids = sorted({x for x in (y.rstrip(":.") for y in re.findall(r"\[((?:unit|perf|tx|joke):[^\]\s,;]+)", answer))
                      if x not in picked})
    for x in bad_ids:
        answer = answer.replace(x, x + " (not a retrieved source)")
    if bad_ids:
        removed.append({"source_id": None, "quote": "", "why": f"answer cited ids that were not retrieved: {bad_ids}"})
    found = bool(data["found"]) and bool(kept)
    if not found and data["found"]:
        answer = "No verified support: " + answer
    calls = router.calls[n0:]
    timings["total_s"] = round(time.perf_counter() - t_all, 2)
    result.update({"found": found, "answer": answer, "not_found": data.get("not_found", ""), "citations": kept,
                   "removed": removed, "timings": timings,
                   "tokens_in": sum(x["tokens_in"] for x in calls), "tokens_out": sum(x["tokens_out"] for x in calls),
                   "provider": f"{c.provider}:{c.model}"})
    return result


def render(r: dict) -> str:
    out = [f"Q: {r['question']}", ""]
    if not r.get("found"):
        out.append("NOT IN YOUR MATERIAL. " + (r.get("not_found") or ""))
    out.append(r.get("answer", ""))
    if r.get("citations"):
        out.append("\nSources:")
        for c in r["citations"]:
            q = f' "{c["quote"]}"' if c.get("quote") else ""
            out.append(f"  [{c['source_id']}] {c['kind']} {c.get('date') or ''} {c.get('title') or ''}{q}"
                       + (f"  <- {c['flag']}" if c.get("flag") else ""))
    if r.get("removed"):
        out.append("\nRemoved (failed the verbatim check):")
        for x in r["removed"]:
            out.append(f"  - {x.get('source_id')}: {x.get('quote', '')[:120]!r} ({x['why']})")
    t = r.get("timings", {})
    out.append(f"\n[{r.get('provider', '')}; {len(r.get('sources', []))} sources; {t.get('total_s', '?')}s; "
               f"tokens in/out {r.get('tokens_in', 0)}/{r.get('tokens_out', 0)}]")
    return "\n".join(out)


def main(argv=None):
    ap = argparse.ArgumentParser(description="Ask your material.")
    ap.add_argument("question", nargs="+")
    ap.add_argument("--config")
    ap.add_argument("--root")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args(argv)
    cfg = Config(a.config, root=a.root)
    r = ask(" ".join(a.question), cfg)
    print(json.dumps(r, indent=1, ensure_ascii=False) if a.json else render(r))


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
