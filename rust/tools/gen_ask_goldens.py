"""Goldens for the one-shot Ask's halves with no model call (engine/ask.py `Corpus`, engine/verify.py), run in-process
over the synthetic notebook (rust/tools/notebook_fixture.py `ask_files`, `ask_decisions`): every source the Corpus
builds (id, kind, text, date, title, joke) in its order, the FTS5 query and the ids SQLite's bm25 returns for a list of
searches, and `strip_unverified_inline_quotes` / `normalize_ids` over a list of cases. The Rust test is
crates/pk-worker/tests/ask_goldens.rs (pk-creative's `oneshot` + pk-fts). Synthetic material only.

    python rust/tools/gen_ask_goldens.py        (from the repo root)
"""
from __future__ import annotations

import json
import os
import sqlite3
import sys
import tempfile
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(ROOT))
import notebook_fixture as F  # noqa: E402

OUT = ROOT / "rust" / "crates" / "pk-worker" / "tests" / "goldens" / "ask_oneshot.json"

SEARCHES = [
    "airport security bin belt", "What have I written about airport security and travel?", "parrot lawyer disclaimers",
    "my parrot's rights", "landlord group chat rent", "running jokes about the gym", "dad father", "a an of",
    "pigeons", "Laundromat dating socks fireplace", "scanner shoes", 'quote "inside" it', "don't stop believing",
]
INLINE = [
    ('He said "started a group chat for the building" and "a thing he never said at all" there.', 4),
    ("“Every message is a threat with a thumbs up” and “a threat of rain later today”.", 4),
    ('"two words" "three words here" "four words right here"', 4),
    ('A "quote\nwith a newline in it ok" and "ok"', 4),
    ('Nested "a "b" c" quotes', 1),
    ('Long "' + "word " * 200 + '" end', 4),
    ("", 4),
]
IDS = ["unit:bins‐a", "perf:2024‑0301‒x–y−z", "joke:a—b", "plain-id"]


class Zeros:
    def embed(self, texts, batch=8):
        return np.zeros((len(texts), 384), np.float32)

    def query(self, text):
        return np.zeros(384, np.float32)


def main():
    with tempfile.TemporaryDirectory(prefix="pk-ask-golden-") as td:
        comedy = Path(td) / "comedy"
        for rel, text in F.ask_files().items():
            p = comedy / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text, encoding="utf-8", newline="\n")
        (comedy / "kb" / "v0" / "decisions.jsonl").write_text(
            "".join(json.dumps(d) + "\n" for d in F.ask_decisions()), encoding="utf-8", newline="\n")
        os.environ["COMEDY_DIR"] = str(comedy)
        os.environ["ENGINE_CONFIG"] = str(ROOT / "engine" / "config.hosted.toml")
        os.environ.pop("PK_OWNER_ID", None)
        from engine.ask import Corpus
        from engine.config import Config
        from engine.kbstore import KB
        from engine.verify import normalize_ids, strip_unverified_inline_quotes
        cfg = Config()
        assert not (cfg.p("vectors_dir") / "vectors_ids.json").exists(), "the hosted image has no tools/kb/cache"
        kb = KB(cfg)
        c = Corpus(cfg, kb, Zeros())
        docs = [{"id": d["id"], "kind": d["kind"], "text": d["text"], "date": d["date"], "title": d["title"],
                 "joke": d["joke"]} for d in c.docs.values()]
        searches = [{"q": q, "hits": c.fts(q)} for q in SEARCHES]
        out = {
            "sqlite": sqlite3.sqlite_version,
            "files": F.ask_files(), "decisions": F.ask_decisions(), "owner": "owner",
            "labels": [[n["id"], n.get("name", "")] for n in kb.view.labels],
            "performed": sorted(kb.performed_jokes),
            "docs": docs, "searches": searches,
            "inline": [{"text": t, "min_words": w, "out": list(strip_unverified_inline_quotes(t, [d["text"] for d in docs], w))} for t, w in INLINE],
            "ids": [[i, normalize_ids(i)] for i in IDS],
        }
    OUT.write_text(json.dumps(out, indent=1, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(docs)} sources, {len(searches)} searches (sqlite {sqlite3.sqlite_version}), {len(INLINE)} inline cases")


if __name__ == "__main__":
    main()
