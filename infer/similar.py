"""Similar jokes and hint labels. These are hints that may be relative.

Similar = 0.45 * IDF-weighted overlap of craft labels (structure, device, form,
attitude, persona, topic) + 0.35 * bge-small cosine of joke centroids
+ 0.20 * an existing reviewed link (same_shape / same_premise / variant_of /
tag_of / callback_to). Spike 01 showed "same shape" by embedding alone fails,
so labels carry the most weight."""
import numpy as np

KIND_W = {"structure": 1.0, "device": 1.0, "form": 0.6, "attitude": 0.5, "persona": 0.5, "topic": 0.7, "theory": 0.2}
NICE = {"same_shape": "same shape", "same_premise": "same premise", "variant_of": "a variant",
        "tag_of": "a tag", "callback_to": "a callback"}


def _labels_of(cat, joke_id=None, extra=None):
    d = dict(cat.craft_labels(joke_id)) if joke_id else {}
    for lid, c in (extra or {}).items():
        d[lid] = max(d.get(lid, 0), c)
    return {l: c for l, c in d.items() if c >= 0.35}


def _overlap(cat, A, B):
    idf = cat.idf()
    def w(l):
        return idf.get(l, 1.0) * KIND_W.get(cat.labels[l]["kind"], 0.5) if l in cat.labels else 0
    inter = sum(w(l) for l in A.keys() & B.keys())
    union = sum(w(l) for l in A.keys() | B.keys()) or 1
    return inter / union, sorted(A.keys() & B.keys(), key=lambda l: -w(l))


def similar(cat, emb, joke_id=None, text=None, extra_labels=None, k=3, exclude=(), unit_id=None):
    ids, M = cat.joke_vectors(emb)
    if joke_id and joke_id in ids:
        q = M[ids.index(joke_id)]
    else:
        q = emb.texts([text])[0]
    cos = M @ q
    extra = dict(extra_labels or {})
    for lid, c, _r, _e in (cat.unit_v1_labels(unit_id) if unit_id else []):
        extra[lid] = max(extra.get(lid, 0), c)
    A = _labels_of(cat, joke_id, extra)
    rel = {o: (kind, c, r) for o, kind, c, r in cat.rel.get(joke_id, [])} if joke_id else {}
    pool = np.argsort(-cos)[:150].tolist()
    pool += [ids.index(o) for o in rel if o in ids]
    scored = []
    for j in set(pool):
        o = ids[j]
        if o == joke_id or o in exclude:
            continue
        ov, shared = _overlap(cat, A, _labels_of(cat, o))
        link = rel.get(o)
        s = 0.45 * ov + 0.35 * max(0.0, float(cos[j]) - 0.6) / 0.4 + (0.2 * link[1] if link else 0)
        scored.append((s, o, shared, link, float(cos[j])))
    scored.sort(key=lambda x: -x[0])
    out = []
    for s, o, shared, link, c in scored[:k]:
        names = [cat.labels[l]["name"] for l in shared if cat.labels[l]["kind"] != "theory"][:3]
        if link:
            why = f"Linked in the catalog as {NICE.get(link[0], link[0])}: {link[2]}".rstrip()
        elif names:
            why = "Shares " + " + ".join(names) + (f"; close in wording ({c:.2f})" if c >= 0.8 else "")
        else:
            why = f"Close in wording and premise (bge-small {c:.2f})"
        out.append({"joke_id": o, "why": why, "confidence": round(min(0.9, max(0.3, s + 0.2)), 2)})
    return out


def hint_labels(cat, joke_id=None, unit_id=None, k=5):
    """Craft labels for the segment, with one-line reasons: the version unit's
    v1 labels first (they carry a reason), then the joke's v0 labels."""
    cand = {}
    if unit_id:
        for lid, c, r, e in cat.unit_v1_labels(unit_id):
            if c >= 0.5 and (lid not in cand or c > cand[lid][0]):
                cand[lid] = (c + 0.1, r or f"On the written version: “{e}”")
    if joke_id:
        for lid, (c, r) in cat.joke_labels.get(joke_id, {}).items():
            if c >= 0.5 and lid not in cand:
                if r.startswith("Proposed at ingest on "):
                    r = "Tagged on " + r[len("Proposed at ingest on "):].split(" (")[0].rstrip(".") + " by the catalog pass."
                cand[lid] = (c, r)
    caps = {"device": 2, "structure": 2, "topic": 1, "form": 1, "attitude": 1, "persona": 1}
    used, out = {}, []
    for lid, (c, r) in sorted(cand.items(), key=lambda x: -x[1][0]):
        kind = cat.labels[lid]["kind"]
        if kind == "theory" or used.get(kind, 0) >= caps.get(kind, 1):
            continue
        used[kind] = used.get(kind, 0) + 1
        out.append({"label_id": lid, "why": r})
        if len(out) >= k:
            break
    return out
