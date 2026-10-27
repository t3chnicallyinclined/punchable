"""Cross-set rollup -> kb/v0/inferred.json (an overlay on graph.json; the app
shows it as current state with an AI badge; anything the comic does wins).

Stage:   working  = AI-rated hit (any laugh) in >= 3 distinct sets at >= 3 distinct
                    venues (an unknown venue counts as its own room)
         testing  = performed, not yet working, last performed within 3 years of the newest tape
         to_try   = performed, not yet working, not on any tape in the last 3 years
         (jokes never performed keep their graph stage: idea / retired)
performed_as -> status "inferred" when the judged match confidence >= 0.75 AND an
         independent check agrees: the stage transcript shares a verbatim run of
         >= 3 words with one of the joke's written units, or the bge-small cosine
         between the stage span and the version unit is >= 0.78; else "proposed". A stage-only joke's
         defining performance is inferred at 0.9 (the joke was created from it).
has_label -> status "inferred" when confidence >= 0.70, where confidence =
         max(v0 ingest confidence, v1 support) + 0.10 if both passes carry the label.
         No gold set exists yet (decisions.jsonl: 4 confirms, 0 rejects), so the bar
         is "one strong pass or two agreeing passes"; learn/ re-tunes it per label
         once the comic's confirms and rejects accumulate."""
import datetime as dt
import hashlib
import json
import os
from difflib import SequenceMatcher

from .metrics import trend
from .paths import INFERRED, KB_V0, SETS, set_dirs
from .scriptdiff import ntok, unit_lines, unit_tokens
from .transcript import Transcript

PERF_MIN = 0.75
RUN_MIN = 3
COS_MIN = 0.78
LABEL_MIN = 0.70
WORKING_SETS = 3
RECENT_YEARS = 3


def edge_id(src, kind, dst):
    return "e:" + hashlib.sha1(f"{src}|{kind}|{dst}".encode("utf-8")).hexdigest()[:12]


def shared_run(unit_text, tr, a, b):
    """Longest verbatim word run shared by the written unit and stage words [a, b)."""
    ut = unit_tokens(unit_text)
    st = [ntok(tr.words[i]["w"]) for i in range(a, b)]
    m = SequenceMatcher(None, [t for t, *_ in ut], st, autojunk=False).find_longest_match(0, len(ut), 0, len(st))
    if m.size == 0:
        return 0, None, None, None
    uq = unit_text[ut[m.a][1]:ut[m.a + m.size - 1][2]]
    sq = tr.span(a + m.b, a + m.b + m.size)
    return m.size, uq, sq, tr.words[a + m.b]["s_ms"]


def word_range(tr, start_ms, end_ms):
    a = next(i for i, w in enumerate(tr.words) if w["s_ms"] >= start_ms)
    b = max(i for i, w in enumerate(tr.words) if w["e_ms"] <= end_ms) + 1
    return a, b


def rejected_edges():
    """Edge ids the comic rejected or deleted (read-only; never re-inferred)."""
    p = os.path.join(KB_V0, "decisions.jsonl")
    out = set()
    if os.path.exists(p):
        for line in open(p, encoding="utf-8"):
            if line.strip():
                d = json.loads(line)
                if d.get("target") == "edge" and d.get("action") in ("reject", "delete"):
                    out.add(d.get("id"))
    return out


def _label_edges(cat, jid, run_id, rejected):
    v0 = cat.joke_labels.get(jid, {})
    us = cat.joke_units.get(jid, [])
    sup, evid = {}, {}
    for uid in us:
        u = cat.units[uid]
        for lid, c, r, e in cat.unit_v1_labels(uid):
            sup[lid] = sup.get(lid, 0) + c
            q = (e or "").strip()
            if q and lid not in evid:
                k = u["text"].lower().find(q.lower())
                if k >= 0:
                    evid[lid] = (uid, u["text"][k:k + len(q)], r)
    out = []
    for lid in sorted(set(v0) | set(sup)):
        s1 = sup.get(lid, 0) / max(1, len(us))
        s0 = v0.get(lid, (0, ""))[0]
        both = s0 > 0.3 and s1 > 0.3
        conf = max(s0, s1) + (0.1 if both else 0)
        if conf < LABEL_MIN or cat.labels.get(lid, {}).get("kind") in (None, "theory"):
            continue
        eid = edge_id(jid, "has_label", lid)
        if eid in rejected:
            continue
        if lid in evid:
            uid, q, r = evid[lid]
            ev, why = [{"node": f"unit:{uid}", "quote": q, "start_ms": None}], (r or f"Carried on unit {uid}.")
        else:
            ge = next((e for e in cat.graph["edges"] if e["id"] == eid), None)
            if not ge or not ge.get("evidence"):
                continue
            ev, why = ge["evidence"][:1], ge.get("reason", "")
        why = why.rstrip(".") + "." + (" Both catalog passes carry it." if both else "")
        out.append({"id": eid, "src": jid, "dst": lid, "kind": "has_label", "status": "inferred",
                    "confidence": round(min(conf, 0.95), 2), "reason": why, "evidence": ev, "source": f"auto:{run_id}"})
    return out


def laugh_metrics(ps):
    """Cross-set laugh-time view of one joke's inferred performances.
    avg = mean of laugh_s_per_min over performances; best = max (with its set);
    pct_laughing_by_set = one row per performance, in date order;
    trend = metrics.trend over (date, laugh_s_per_min)."""
    pts = [p for p in ps if p.get("laugh_s_per_min") is not None]
    if not pts:
        return None
    vals = [p["laugh_s_per_min"] for p in pts]
    best = max(pts, key=lambda p: (p["laugh_s_per_min"], p["date"]))
    tr, rel = trend([(p["date"], p["laugh_s_per_min"]) for p in pts])
    return {"avg_laugh_s_per_min": round(sum(vals) / len(vals), 2),
            "best_laugh_s_per_min": best["laugh_s_per_min"], "best_set": best["set"],
            "pct_laughing_by_set": [{"set": p["set"], "date": p["date"], "pct_laughing": p["pct_laughing"]} for p in pts],
            "trend": tr, "trend_rel_change": rel, "trend_dates": len({p["date"] for p in pts})}


def show_meta(set_dir):
    p = os.path.join(SETS, set_dir, "meta.json")
    return json.load(open(p, encoding="utf-8")) if os.path.exists(p) else {}


def rollup(cat, emb, run_id, model):
    rejected = rejected_edges()
    sets = {n["id"]: n for n in cat.graph["nodes"] if n["type"] == "set"}
    perf, nodes, edges, unlinked = {}, [], [], []
    for d in set_dirs():
        p = os.path.join(SETS, d, "analysis.json")
        if not os.path.exists(p):
            continue
        an = json.load(open(p, encoding="utf-8"))
        sid = "set:" + an["set"]
        meta = show_meta(d)
        # The show's own venue and date (parsed from the title into meta.json, marked inferred)
        # beat the graph node, which has the YouTube upload date and no node at all for newer sets.
        venue = meta.get("show_venue") or (sets.get(sid) or {}).get("venue") or "unknown"
        show_date = meta.get("show_date") or an["set"][:10]
        if venue.lower().startswith("unknown"):
            venue = f"unknown room ({an['set'][:10]})"
        tr = Transcript(d)
        for j in an["jokes"]:
            if j["status"] == "banter":
                continue
            if not j["joke_id"]:
                unlinked.append({"set": an["set"], "idx": j["idx"], "title": j["title"], "status": j["status"],
                                 "ai_rating": j["ai_rating"], "laughs": j["laughs"]["count"],
                                 "similar": [s["joke_id"] for s in j["similar"]]})
                continue
            pid = f"perf:{an['set']}@{j['start_ms']}"
            nodes.append({"id": pid, "type": "performance", "set": sid, "start_ms": j["start_ms"], "end_ms": j["end_ms"],
                          "text": j["transcript"], "kind": "crowd_work" if j["status"] == "crowd_work" else "bit",
                          "name": j["title"], "status": "inferred", "source": f"auto:{run_id}",
                          "ai_rating": j["ai_rating"], "rating_basis": j["rating_basis"], "laughs": j["laughs"]})
            joke = cat.jokes[j["joke_id"]]
            conf, ev, why = j["match_confidence"], [], j["match_reason"]
            stage_only = bool(joke.get("first_seen_on_stage")) and not cat.joke_units.get(j["joke_id"])
            if j["version_unit_id"]:
                uid = j["version_unit_id"].split(":", 1)[1]
                a, b = word_range(tr, j["start_ms"], j["end_ms"])
                best = (0, None, None, None, None)
                for u2 in cat.joke_units.get(j["joke_id"], []):
                    r = shared_run(cat.units[u2]["text"], tr, a, b)
                    if r[0] > best[0]:
                        best = r + (u2,)
                n, uq, sq, sms, u_ev = best
                cos = float(emb.texts([j["transcript"]])[0] @ emb.unit(uid, cat.units[uid]["text"]))
                if n >= RUN_MIN:
                    ev = [{"node": pid, "quote": sq, "start_ms": sms}, {"node": f"unit:{u_ev}", "quote": uq, "start_ms": None}]
                else:
                    top = max(j["punchlines"], key=lambda p_: p_["bars"]) if j["punchlines"] else None
                    lines = unit_lines(cat.units[uid]["text"])
                    ev = ([{"node": pid, "quote": top["text"], "start_ms": top["at_ms"]}] if top else []) +                          ([{"node": j["version_unit_id"], "quote": lines[-1][0], "start_ms": None}] if lines else [])
                ok = conf >= PERF_MIN and (n >= RUN_MIN or cos >= COS_MIN)
                why = why.rstrip(".") + f". Check: shared wording run {n} words, wording similarity {cos:.2f}."
            else:
                if stage_only:
                    conf = max(conf, 0.9)
                    why = "First seen on stage: this joke was created from this performance. " + why
                if j["punchlines"]:
                    p0 = j["punchlines"][0]
                    ev = [{"node": pid, "quote": p0["text"], "start_ms": p0["at_ms"]}]
                else:
                    ev = [{"node": pid, "quote": j["transcript"][:200].rsplit(" ", 1)[0], "start_ms": j["start_ms"]}]
                ok = stage_only and conf >= PERF_MIN
            e = {"id": edge_id(pid, "performed_as", j["joke_id"]), "src": pid, "dst": j["joke_id"], "kind": "performed_as",
                 "status": "inferred" if ok else "proposed", "confidence": round(conf, 2), "reason": why,
                 "evidence": ev, "source": f"auto:{run_id}"}
            if e["id"] in rejected:
                continue
            edges.append(e)
            if ok:
                perf.setdefault(j["joke_id"], []).append({
                    "set": an["set"], "set_id": sid, "date": show_date, "upload_date": an["set"][:10],
                    "date_inferred": bool(meta.get("show_date_inferred")), "venue": venue, "perf": pid,
                    "idx": j["idx"], "title": j["title"], "ai_rating": j["ai_rating"], "rating_basis": j["rating_basis"], "laughs": j["laughs"]["count"],
                    "max_bars": j["laughs"]["max_bars"], "laugh_s": j["laughs"]["total_s"],
                    "laugh_s_per_min": j.get("laugh_s_per_min"), "pct_laughing": j.get("pct_laughing"),
                    "vs_room": j.get("vs_room")})

    newest = max((p["date"] for ps in perf.values() for p in ps), default="1970-01-01")
    cutoff = f"{int(newest[:4]) - RECENT_YEARS}{newest[4:]}"
    jokes = []
    for jid, ps in sorted(perf.items()):
        ps.sort(key=lambda p: (p["date"], p["idx"]))
        hits = [p for p in ps if p["laughs"] > 0]
        hit_sets = sorted({p["set"] for p in hits})
        hit_venues = sorted({p["venue"] for p in hits})
        n_sets = len({p["set"] for p in ps})
        last = ps[-1]["date"]
        if len(hit_sets) >= WORKING_SETS and len(hit_venues) >= WORKING_SETS:
            stage, conf = "working", min(0.95, 0.6 + 0.08 * len(hit_sets))
            reason = f"AI-rated a hit in {len(hit_sets)} sets at {len(hit_venues)} rooms: " + "; ".join(hit_venues) + "."
        else:
            stage = "testing" if last >= cutoff else "to_try"
            conf = 0.7
            reason = (f"Performed in {n_sets} set{'s' if n_sets > 1 else ''}, a hit in {len(hit_sets)}; Working needs "
                      f"{WORKING_SETS} sets at {WORKING_SETS} rooms. Last on tape {last}"
                      + ("." if stage == "testing" else f", over {RECENT_YEARS} years before the newest tape."))
        jokes.append({"id": jid, "title": cat.jokes[jid]["title"], "graph_stage": cat.jokes[jid].get("stage"),
                      "stage": stage, "status": "inferred", "confidence": round(conf, 2), "stage_reason": reason,
                      "hit_sets": len(hit_sets), "hit_venues": len(hit_venues),
                      "first_performed": ps[0]["date"], "last_performed": last,
                      "laugh_metrics": laugh_metrics(ps), "performances": ps})

    for jid in sorted(perf):
        edges.extend(_label_edges(cat, jid, run_id, rejected))

    # every evidence quote must be verbatim in its node's text
    texts = {n["id"]: n.get("text", "") for n in cat.graph["nodes"] if n["type"] in ("unit", "performance")}
    texts.update({f"unit:{k}": u["text"] for k, u in cat.units.items()})
    texts.update({n["id"]: n["text"] for n in nodes})
    checked = failed = 0
    kept, fails = [], []
    for e in edges:
        good = []
        for q in e["evidence"]:
            checked += 1
            if q.get("quote") and q["quote"] in texts.get(q["node"], ""):
                good.append(q)
            else:
                failed += 1
                fails.append(f"{e['id']} {e['kind']} {q.get('node')}: {str(q.get('quote'))[:60]!r}")
        e["evidence"] = good
        if good:
            kept.append(e)
    edges = kept
    validation = {"evidence_checked": checked, "evidence_failed": failed, "edges_kept": len(edges), "dropped": fails}

    out = {
        "version": 0, "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "run_id": run_id,
        "notes": "AI inferences from the per-set analyses (punchable/infer). Shown as current state; the comic's edits override.",
        "runs": [{"run_id": run_id, "pipeline": "infer", "model": model,
                  "notes": "Per-set analysis (punchable/infer) rolled up across sets. Overlay on graph.json; never written to decisions.jsonl."}],
        "thresholds": {
            "working": f"AI-rated hit (any laugh) in >= {WORKING_SETS} distinct sets at >= {WORKING_SETS} distinct venues",
            "performed_as_inferred": f"judged match confidence >= {PERF_MIN} and a verbatim shared run of >= {RUN_MIN} words with the written version",
            "has_label_inferred": f"confidence >= {LABEL_MIN}: max(v0 ingest, v1 support) + 0.10 when both passes agree",
            "why": "No gold set yet (4 confirms, 0 rejects in decisions.jsonl), so each inference needs one strong signal or two independent agreeing ones; learn/ re-tunes per label from the comic's confirms and rejects.",
        },
        "validation": validation,
        "jokes": jokes, "nodes": nodes, "edges": edges, "unlinked_new": unlinked,
    }
    from .sets import write_json
    write_json(INFERRED, out)
    return out
