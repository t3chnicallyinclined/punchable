"""Goldens for pk-core's legal rules: `gsync/rules.py` loaded under several synthetic rules files, and what it makes of
them: the canonical text its `fingerprint()` hashes (`GET /api/worker/state` shows the first twelve hex of that hash)
and `content_is_legal` over a few texts. Each case is a fresh interpreter, because the module reads the file at import.
Synthetic names only.

    python rust/tools/gen_legal_goldens.py        (from the repo root)
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "rust" / "crates" / "pk-core" / "tests" / "goldens" / "legal.json"

PROBE = r"""
import importlib.util, json, hashlib, sys
spec = importlib.util.spec_from_file_location("r", sys.argv[1])
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
canon = json.dumps({'parents': sorted(m.LEGAL_PARENTS), 'extra': sorted(m.LEGAL_FOLDERS_EXTRA), 'kw': m.KW,
                    'owner': m.OWNER, 'ids': sorted(m.LEGAL_IDS), 'markers': m.CONTENT_MARKERS,
                    'own_kw_only': sorted(m.OWN_KW_ONLY_IDS)}, sort_keys=True)
assert hashlib.sha256(canon.encode()).hexdigest() == m.fingerprint()
texts = json.loads(sys.argv[2])
print(json.dumps({"canonical": canon, "legal": [m.content_is_legal(t) for t in texts]}))
"""

TEXTS = ["The petitioner walked in.", "The PETITIONER and the respondent walked in.", "golden county, plaintiff",
         "golden county golden county", "silver county and the silver county clerk", "nothing here", ""]

CASES = [
    ("none", None, None),
    ("owner_from_env", None, "me@example.com"),
    ("full", {"owner": "boss@example.com", "legal_parents": ["folderB", "folderA"], "legal_folders_extra": ["fx"],
              "keywords": ["Moot", "legal", "ZETA"], "content_markers": ["Golden County", "petitioner", "Silver County"],
              "legal_ids": ["d2", "d1"], "own_keywords_only_ids": ["d9"]}, "ignored@example.com"),
    ("duplicate_own_markers", {"content_markers": ["Silver County", "silver county"]}, None),
    ("garbage", "this is not json", None),
]


def main():
    cases = []
    with tempfile.TemporaryDirectory() as tmp:
        probe = Path(tmp) / "probe.py"
        probe.write_text(PROBE, encoding="utf-8", newline=chr(10))
        for name, rules, owner_env in CASES:
            rf = Path(tmp) / f"{name}.json"
            text = rules if isinstance(rules, str) else (json.dumps(rules) if rules is not None else None)
            env = {k: v for k, v in os.environ.items() if not k.startswith(("PUNCHABLE_", "PK_"))}
            env["PUNCHABLE_HOME"] = str(Path(tmp) / "nohome")
            if text is not None:
                rf.write_text(text, encoding="utf-8", newline=chr(10))
                env["PUNCHABLE_LEGAL_RULES"] = str(rf)
            if owner_env:
                env["PUNCHABLE_OWNER_EMAIL"] = owner_env
            r = subprocess.run([sys.executable, "-I", str(probe), str(ROOT / "gsync" / "rules.py"), json.dumps(TEXTS)],
                               env=env, capture_output=True, text=True, check=False)
            if r.returncode:
                raise SystemExit(f"{name}: {r.stderr[-800:]}")
            got = json.loads(r.stdout)
            cases.append({"name": name, "rules_text": text if text is not None else "", "owner_env": owner_env,
                          "texts": TEXTS, **got})
    OUT.write_text(json.dumps(cases, indent=1, ensure_ascii=False) + "\n", encoding="utf-8", newline=chr(10))
    print(f"wrote {OUT} ({len(cases)} cases)")


if __name__ == "__main__":
    main()
