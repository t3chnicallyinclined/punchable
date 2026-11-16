"""The edits made on purpose to the recorded gsync tapes (T11 round A0, "gsync fails closed").

The tapes in rust/crates/pk-gsync/tests/tapes are what the PYTHON gsync did. Round A0 changed pk-gsync where the Python
failed open in the legal check, so a few places in the tapes now differ ON PURPOSE. This file is the list of those places,
each with its reason, and `apply(tape)` makes them. `gen_gsync_tapes.py` calls it after recording, so `--check` still
proves that the tapes are the Python's recording plus exactly these edits and nothing else. Run this file by itself to
apply the edits to the tapes on disk (it changes nothing the second time).

Edits (what, and why):

E1  A legal doc's reason in `kb/gsync/map.json` is the KIND only (`"excluded": {"<doc>": "kw"}`), not `"kw:<words>"`.
    Why: the words that matched include his own private keywords; the map is copied to the database and to backups
    (gw-security must-fix 8). The posted state's `sha256` and every later listing of that file change with the text.

E2  Before a voice memo's bytes are downloaded, pk-gsync reads that file's metadata again (`GET /drive/v3/files/<id>`) and
    the legal rules look at it; the Python downloaded a listed memo without looking (gw-security must-fix 2, a clearance
    at every download). One new request per downloaded memo, answered with the metadata the tape listed for that file.
    The run record's `drive.calls` counts those requests, so it is raised by the same number (in the posted
    `runs.jsonl`, the files left behind, and the `sha256` of each).
"""
from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path

TAPES = Path(__file__).resolve().parents[1] / "crates" / "pk-gsync" / "tests" / "tapes"

# the kind only: `"kw:<words>"` / `"filing:<words>"` as a value in the map's text
_REASON = re.compile(r'("(?:kw|filing)):[^"\n]*"')
_CALLS = re.compile(r'("drive":\{"calls":)(\d+)')

META_FIELDS = "id,name,mimeType,trashed,parents,owners(emailAddress),createdTime,modifiedTime,version,appProperties"
META_KEYS = ["id", "name", "mimeType", "trashed", "parents", "owners", "createdTime", "modifiedTime", "version", "appProperties"]


def _sha(text: str) -> str:
    """The hash of the text as it is on disk when a test runs (`{GDOCS_HOST}` is the real host there; `{BASE}` is
    hashed again by the replay, which knows the port)."""
    return hashlib.sha256(text.replace("{GDOCS_HOST}", "docs.google.com").encode("utf-8")).hexdigest()


def _retext(tape: dict, fn) -> None:
    """Replace the text of every state file `fn(path, text)` changes: in each posted body (with its `sha256`), in each
    later listing of that file (the `sha256` it reports) and in the files left behind after each step."""
    named: dict[str, str] = {}
    for step in tape["steps"]:
        for ex in step["exchanges"]:
            if ex["path"] != "/api/worker/files":
                continue
            if ex["method"] == "POST":
                for f in ex["body"].get("files", []):
                    new = fn(f["path"], f["text"])
                    if new != f["text"]:
                        f["text"] = new
                        f["sha256"] = _sha(new)
                        named[f["path"]] = f["sha256"]
                    else:
                        named.pop(f["path"], None)
            else:
                for f in ex["resp"].get("json", {}).get("files", []):
                    if f["path"] in named:
                        f["sha256"] = named[f["path"]]
        for rel, text in list(step["after"].items()):
            step["after"][rel] = fn(rel, text)


def _e1(tape: dict) -> None:
    def cut(path: str, text: str) -> str:
        if not path.endswith("kb/gsync/map.json"):
            return text
        return _REASON.sub(lambda m: m.group(1) + '"', text)

    _retext(tape, cut)


def _listed_meta(tape: dict, step_index: int, fid: str) -> dict | None:
    """The metadata the tape listed for `fid`, in this step or an earlier one (the newest first)."""
    for si in range(step_index, -1, -1):
        for ex in reversed(tape["steps"][si]["exchanges"]):
            if ex["svc"] != "drive" or ex["method"] != "GET" or ex["path"] != "/drive/v3/files":
                continue
            for f in (ex["resp"].get("json") or {}).get("files", []):
                if f.get("id") == fid:
                    return {k: f[k] for k in META_KEYS if k in f}
    return None


def _e2(tape: dict) -> None:
    added: dict[int, int] = {}  # the step -> how many requests were added to its run
    for si, step in enumerate(tape["steps"]):
        out: list[dict] = []
        seen: set[str] = set()
        for ex in step["exchanges"]:
            if ex["svc"] == "drive" and ex["method"] == "GET" and ex.get("query", {}).get("alt") == ["media"]:
                fid = ex["path"].rsplit("/", 1)[1]
                first = fid not in seen
                seen.add(fid)
                before = out[-1] if out else {}
                made = before.get("path") == f"/drive/v3/files/{fid}" and before.get("query", {}).get("fields") == [META_FIELDS]
                if first and not made:
                    meta = _listed_meta(tape, si, fid)
                    if meta is None:
                        raise SystemExit(f"E2: no listed metadata for {fid} in {tape['name']} step {si + 1}")
                    out.append(
                        {"svc": "drive", "method": "GET", "path": f"/drive/v3/files/{fid}",
                         "query": {"alt": ["json"], "fields": [META_FIELDS]}, "auth": ex["auth"], "ctype": None,
                         "range": None, "body": None,
                         "resp": {"status": 200, "ctype": "application/json", "extra": {}, "json": meta}}
                    )
                    added[si] = added.get(si, 0) + 1
            out.append(ex)
        step["exchanges"] = out
    if not added:
        return
    # the run record of a step is the last line its `runs.jsonl` gained
    line_of: dict[int, int] = {}
    seen_lines = 0
    for si, step in enumerate(tape["steps"]):
        n = 0
        for rel, text in step["after"].items():
            if rel.endswith("kb/gsync/runs.jsonl"):
                n = len(text.splitlines())
        if n > seen_lines:
            line_of[si] = n - 1
            seen_lines = n
    by_line = {line_of[si]: k for si, k in added.items() if si in line_of}

    def raise_calls(path: str, text: str) -> str:
        if not path.endswith("kb/gsync/runs.jsonl"):
            return text
        lines = text.split("\n")
        for i, k in by_line.items():
            if i < len(lines):
                lines[i] = _CALLS.sub(lambda m: m.group(1) + str(int(m.group(2)) + k), lines[i], count=1)
        return "\n".join(lines)

    _retext(tape, raise_calls)


def apply(tape: dict) -> dict:
    """The recorded `tape` with the round A0 edits made."""
    _e1(tape)
    _e2(tape)
    return tape


def main() -> int:
    for p in sorted(TAPES.glob("*.json")):
        tape = json.loads(p.read_text(encoding="utf-8"))
        text = json.dumps(apply(tape), ensure_ascii=False, indent=1) + "\n"
        if text != p.read_text(encoding="utf-8"):
            p.write_text(text, encoding="utf-8", newline="\n")
            print("edited", p.name)
    return 0


if __name__ == "__main__":
    sys.exit(main())
