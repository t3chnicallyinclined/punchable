"""Search over everything (api-dev/search_api.py): the word-hash embedder, the sweeper that fills the vectors, the
fused search and "similar" on a scratch SurrealDB with a four-node graph of its own, the files-backend fallback, and
the image/manifest pins that make the hosted API able to embed at all. No real data, no model, no network.

    python -m pytest api-dev/tests/test_search_api.py -q
"""
from __future__ import annotations

import os
import re
import sys
import time
from pathlib import Path

import pytest

DESK = Path(__file__).resolve().parents[2]
for p in (DESK / "db" / "tests", DESK / "db", DESK / "api-dev", DESK / "tools" / "explorer", DESK):
    sys.path.insert(0, str(p))
import sdb  # noqa: E402
import search_api as S  # noqa: E402
import kb as K  # noqa: E402

GRAPH = {
    "version": "test", "generated_at": "2026-10-28", "runs": [],
    "nodes": [
        {"id": "joke:j1", "type": "joke", "title": "Umbrella at the airport", "stage": "working",
         "summary": "Airports treat a umbrella like a carry-on"},
        {"id": "unit:u1", "type": "unit", "text": "They gate-check my umbrella like it is a stroller", "date": "2025-01-02"},
        {"id": "joke:j2", "type": "joke", "title": "Saw a raccoon in the green room", "stage": "idea",
         "summary": "Saw a raccoon in the green room the size of a shoe. It had its own mic.", "_by_tristan": True},
        {"id": "joke:j3", "type": "joke", "title": "Dating apps", "stage": "testing", "summary": "Dating apps are a job interview"},
        {"id": "unit:u3", "type": "unit", "text": "Dating apps are a job interview where everyone lies on the resume", "date": "2024-06-01"},
        {"id": "set:s1", "type": "set", "title": "Tuesday at the Cellar", "venue": "The Cellar", "date": "2025-03-04"},
        {"id": "perf:p1", "type": "performance", "set": "set:s1", "start_ms": 60000, "end_ms": 90000,
         "text": "so a raccoon walks into the green room and he is bigger than my shoe"},
        {"id": "perf:p2", "type": "performance", "set": "set:s1", "start_ms": 120000, "end_ms": 150000,
         "text": "they gate-checked my umbrella again, like a stroller"},
    ],
    "edges": [
        {"id": "e:a1", "kind": "part_of", "src": "unit:u1", "dst": "joke:j1", "status": "confirmed"},
        {"id": "e:a2", "kind": "part_of", "src": "unit:u3", "dst": "joke:j3", "status": "confirmed"},
        {"id": "e:a3", "kind": "performed_as", "src": "perf:p2", "dst": "joke:j1", "status": "confirmed"},
    ],
}


def view() -> K.View:
    return K.View(GRAPH, [], [], lambda *a: None)


class FakeApp:
    def __init__(self, v):
        self.v = v

    def summaries(self):
        return [{"id": j["id"], "title": j["title"], "stage": j.get("stage"), "summary": j.get("summary"), "label_ids": []}
                for j in self.v.jokes]


class FilesKB:
    backend = "files"


# ------------------------------------------------------------------------------------------ the hash embedder
def test_hash_embedder_is_deterministic_unit_length_and_word_sensitive():
    e = S.HashEmbedder()
    a, b, c = e.embed(["the raccoon in the green room", "a raccoon ran across the green room floor", "my umbrella bit"])
    dot = lambda x, y: sum(p * q for p, q in zip(x, y))  # noqa: E731
    assert len(a) == S.DIM and abs(dot(a, a) - 1.0) < 1e-9
    assert e.embed(["the raccoon in the green room"])[0] == a
    assert dot(a, b) > dot(a, c) > -1
    assert S.sha("abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"


def test_make_embedder_without_a_model_is_none_and_hash_on_request(monkeypatch):
    monkeypatch.delenv("PK_BGE_DIR", raising=False)
    monkeypatch.delenv("PK_CACHE_DIR", raising=False)
    monkeypatch.delenv("PK_EMBEDDER", raising=False)
    said = []
    assert S.make_embedder(log=said.append) is None and "words alone" in said[-1]
    monkeypatch.setenv("PK_EMBEDDER", "hash")
    assert isinstance(S.make_embedder(log=said.append), S.HashEmbedder)


# ---------------------------------------------------------------------------------------- the files fallback
def test_files_fallback_search_and_similar_use_words_over_jokes_and_moments():
    v = view()
    a = FakeApp(v)
    out = S.search_payload(FilesKB(), a, "raccoon", 10, False)
    assert [b["id"] for b in out["bits"]] == ["joke:j2"] and "raccoon" in out["bits"][0]["why"].lower()
    assert [m["id"] for m in out["moments"]] == ["perf:p1"]
    m = out["moments"][0]
    assert m["set"] == "set:s1" and m["venue"] == "The Cellar" and m["date"] == "2025-03-04" and m["start_ms"] == 60000
    assert m["joke"] is None and out["vectors"] is False
    st, sim = S.similar_payload(FilesKB(), a, "joke:j1", 8, False)
    assert st == 200
    assert [p["id"] for p in sim["performed"]] == ["perf:p2"] and sim["performed"][0]["status"] == "confirmed"
    assert sim["performed"][0]["set_title"] == "Tuesday at the Cellar"
    assert "perf:p2" not in [m["id"] for m in sim["like"]["moments"]]   # linked already: never also "like"
    assert "joke:j1" not in [b["id"] for b in sim["like"]["bits"]]
    assert S.similar_payload(FilesKB(), a, "joke:nope", 8, False)[0] == 404


def test_newest_text_prefers_the_newest_version_then_the_typed_idea():
    v = view()
    assert S.newest_text(v, "joke:j1").startswith("They gate-check")
    assert S.newest_text(v, "joke:j2").startswith("Saw a raccoon")


# --------------------------------------------------------------------------------------- scratch SurrealDB
def _scratch_ok() -> str | None:
    if not sdb.SURREAL_EXE.is_file():
        return f"no SurrealDB binary at {sdb.SURREAL_EXE}"
    if not sdb.secrets().get("SURREAL_PASS"):
        return "no SURREAL_PASS in ~/.punchable/secrets.env"
    return None


@pytest.fixture(scope="module")
def scratch():
    why = _scratch_ok()
    if why:
        pytest.skip(why)
    from common import scratch_server
    import kbmap as KM
    import load as L
    scratch_server()
    ns, db = "punchable_test", f"t_search_{os.getpid()}_{int(time.time() * 1000) % 10_000_000}"
    r = sdb.Rpc(None, ns=None, db=None)
    L.apply_schema(r, ns, db, force=True)
    r.use(ns, db)
    rows: dict[str, list] = {}
    for i, n in enumerate(GRAPH["nodes"]):
        tb, row = KM.node_to_row({k: v for k, v in n.items() if not k.startswith("_")}, i)
        rows.setdefault(tb, []).append(row)
    for tb, rs in rows.items():
        L.insert_rows(r, tb, rs)
    L.insert_rows(r, None, [KM.edge_to_row(e, i)[1] for i, e in enumerate(GRAPH["edges"])], relation=True)
    yield r
    r.query(f"USE NS {ns}; REMOVE DATABASE IF EXISTS {db};")
    r.close()


def test_sweeper_fills_vectors_once_and_again_only_when_the_text_changes(scratch):
    r, v, emb = scratch, view(), S.HashEmbedder()
    first = S.sweep(r, emb, v)
    # j2 is the only joke without a unit; both moments have text; both units were inserted without a vector
    assert first == {"joke": 1, "performance": 2, "unit": 2}
    assert S.sweep(r, emb, v) == {"joke": 0, "performance": 0, "unit": 0}
    row = r.last("SELECT embedded_sha, array::len(embedding) AS n FROM ONLY joke:j2;")
    assert row["n"] == S.DIM and row["embedded_sha"] == S.sha(GRAPH["nodes"][2]["summary"])
    assert r.last("RETURN count(SELECT id FROM unit WHERE embedding = NONE);") == 0
    r.query("UPDATE joke:j2 SET summary = 'Saw a raccoon in the green room. It asked for a tight five.';")
    assert S.sweep(r, emb, v) == {"joke": 1, "performance": 0, "unit": 0}
    r.query("UPDATE joke:j2 SET summary = $s;", {"s": GRAPH["nodes"][2]["summary"]})
    S.sweep(r, emb, v)


def test_fused_search_finds_a_typed_idea_and_a_taped_moment(scratch):
    r, v, emb = scratch, view(), S.HashEmbedder()
    S.sweep(r, emb, v)
    res = S.search_all(r, emb, "raccoon", 10, False)
    assert res["vectors"] is True
    assert S.fold_jokes(res["hits"], v)[0] == "joke:j2"            # the jot's body, not its title, is what matches
    assert [pid for pid, _ in res["moments"]][0] == "perf:p1"
    res = S.search_all(r, emb, "umbrella", 10, False)
    assert S.fold_jokes(res["hits"], v)[0] == "joke:j1"            # through its unit and its title
    assert [pid for pid, _ in res["moments"]][0] == "perf:p2"
    # words only (no embedder) still finds both through the full-text indexes
    res = S.search_all(r, None, "raccoon", 10, False)
    assert res["vectors"] is False and S.fold_jokes(res["hits"], v) == ["joke:j2"]
    assert [pid for pid, _ in res["moments"]] == ["perf:p1"]
    # hidden jokes stay out unless asked
    r.query("UPDATE joke:j2 SET hidden = true;")
    assert "joke:j2" not in S.fold_jokes(S.search_all(r, emb, "raccoon", 10, False)["hits"], v)
    assert "joke:j2" in S.fold_jokes(S.search_all(r, emb, "raccoon", 10, True)["hits"], v)
    r.query("UPDATE joke:j2 SET hidden = NONE;")


def test_similar_points_a_typed_idea_at_the_tape_it_echoes(scratch):
    r, v, emb = scratch, view(), S.HashEmbedder()
    S.sweep(r, emb, v)
    sim = S.similar(r, emb, v, "joke:j2", 8, False)
    assert sim["vectors"] is True and sim["text"].startswith("Saw a raccoon")
    assert sim["moments"][0][0] == "perf:p1"
    assert "joke:j2" not in [j for j, _ in sim["bits"]]
    sim = S.similar(r, emb, v, "joke:j1", 8, False)
    assert sim["moments"][0][0] == "perf:p2"            # its own tape comes first; the payload files it under "performed"
    assert "joke:j1" not in [j for j, _ in sim["bits"]]

    class KB:
        backend = "surreal"

        def search_all(self, q, limit, inc):
            return S.search_all(r, emb, q, limit, inc)

        def similar_vectors(self, v_, jid, limit, inc):
            return S.similar(r, emb, v_, jid, limit, inc)

    st, out = S.similar_payload(KB(), FakeApp(v), "joke:j1", 8, False)
    assert st == 200 and out["vectors"] is True
    assert [p["id"] for p in out["performed"]] == ["perf:p2"]
    assert "perf:p2" not in [m["id"] for m in out["like"]["moments"]]
    out = S.search_payload(KB(), FakeApp(v), "green room", 10, False)
    assert out["vectors"] is True and out["bits"][0]["id"] == "joke:j2" and out["moments"][0]["id"] == "perf:p1"


# ------------------------------------------------------------------------------- the image can embed at all
def test_api_image_ships_the_model_and_the_manifest_leaves_room_for_it():
    df = (DESK / "api" / "Dockerfile").read_text(encoding="utf-8")
    code = "\n".join(l for l in df.splitlines() if not l.lstrip().startswith("#"))
    assert "fetch_bge.py --dest /opt/punchable/models/bge-small-en-v1.5" in code
    assert "COPY --from=deps /opt/punchable/models /opt/punchable/models" in code
    assert "PK_BGE_DIR=/opt/punchable/models/bge-small-en-v1.5" in code and "engine/embed.py" in code
    ign = (DESK / "api" / "Dockerfile.dockerignore").read_text(encoding="utf-8").splitlines()
    allowed = [l[1:] for l in ign if l.startswith("!")]
    for m in re.finditer(r"^COPY (?!--from)(.+?) \S+$", df, re.M):
        for src in m.group(1).split():
            assert any(Path(src).match(a) or src == a for a in allowed), f"{src} is not in api/Dockerfile.dockerignore"
    req = (DESK / "api" / "requirements.txt").read_text(encoding="utf-8")
    wreq = (DESK / "infra" / "worker" / "requirements.txt").read_text(encoding="utf-8")
    for pin in re.findall(r"^(onnxruntime==\S+|tokenizers==\S+)", req, re.M):
        assert pin in wreq, pin   # one version of the runtime across both images
    import yaml
    s = (DESK / "infra" / "k8s" / "30-api.yaml").read_text(encoding="utf-8")
    s = re.sub(r"__[A-Z_]+__", "x", s)
    dep = next(d for d in yaml.safe_load_all(s) if d and d.get("kind") == "Deployment")
    c = dep["spec"]["template"]["spec"]["containers"][0]
    assert c["resources"]["limits"]["memory"] == "1Gi" and c["resources"]["requests"]["memory"] == "320Mi"
    extra = (DESK / "db" / "schema_extra.surql").read_text(encoding="utf-8")
    for want in ("embedding    ON joke", "embedded_sha ON performance", "joke_sum_ft", "perf_ft", "joke_vec", "perf_vec"):
        assert want in extra, want
