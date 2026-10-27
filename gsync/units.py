"""Corpus-format mirror files and units, the way comedy/corpus was built.

File: <created-date>_<slug>__<docid8>.md, front matter source/doc_id/title/created/modified/url, body = the export
unedited. Units: "paragraph indices count the non-blank lines of the doc body from 0, inclusive"; an empty body is a
title unit (locator {"source": "title"}). The ingest grouped paragraphs into jokes by reading them; gsync does not
guess: one unit per doc, cut at markdown headings, kind 'note' + needs_look for the labelling pipeline.
"""
from __future__ import annotations

import json
import re

PARA_RULE = "non-blank lines of fileContent, 0-based, inclusive"


def slug(t: str) -> str:  # triage.py's slug, verbatim
    s = re.sub(r'[^a-z0-9]+', '-', t.lower()).strip('-')
    return (s[:50].rstrip('-') or 'untitled')


def mirror_name(meta: dict) -> str:
    return f"{(meta.get('createdTime') or '')[:10]}_{slug(meta.get('name') or '')}__{meta['id'][:8]}.md"


def mirror_text(meta: dict, body: str) -> str:
    fm = ["---", "source: gdoc", f"doc_id: {meta['id']}", f"title: {json.dumps(meta.get('name') or '', ensure_ascii=False)}",
          f"created: {meta.get('createdTime')}", f"modified: {meta.get('modifiedTime')}",
          f"url: https://docs.google.com/document/d/{meta['id']}/edit", "---"]
    return "\n".join(fm) + "\n" + body


def split_units(meta: dict, body: str, file: str) -> list[dict]:
    lines = [ln for ln in (body or "").split("\n") if ln.strip()]
    base = {"doc_id": meta["id"], "doc_title": meta.get("name") or "", "created": meta.get("createdTime"),
            "modified": meta.get("modifiedTime"), "file": file, "kind": "note", "parent_hint": meta.get("name") or "",
            "topics": [], "devices": [], "structures": [], "form": None, "attitude": None, "confidence": 0.4,
            "needs_look": True, "source": "gsync"}
    if not lines:
        return [{**base, "unit_id": f"{meta['id'][:8]}-00", "locator": {"source": "title"}, "text": meta.get("name") or ""}]
    cuts = [0] + [i for i, ln in enumerate(lines) if i > 0 and re.match(r"^\s{0,3}#{1,6}\s", ln)]
    out = []
    for n, s in enumerate(cuts):
        e = (cuts[n + 1] if n + 1 < len(cuts) else len(lines)) - 1
        out.append({**base, "unit_id": f"{meta['id'][:8]}-{n:02d}",
                    "locator": {"para_start": s, "para_end": e, "para_rule": PARA_RULE},
                    "text": "\n".join(lines[s:e + 1])})
    return out


def norm(t: str | None) -> str:
    """Comparison form of a text: no BOM, \\n line ends, no trailing spaces, trimmed."""
    t = (t or "").replace("\ufeff", "").replace("\r\n", "\n").replace("\r", "\n")
    return "\n".join(ln.rstrip() for ln in t.split("\n")).strip()


def h(t: str | None) -> str:
    import hashlib
    return hashlib.sha256(norm(t).encode("utf-8")).hexdigest()[:16]
