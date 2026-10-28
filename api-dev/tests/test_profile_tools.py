"""The two desk tools around the voice profile, dry: tools/profile/put_voice.py (reads a private file, refuses court
text and anything inside the repo, sends nothing on --dry-run) and tools/prompts/eval_riff.py (BEFORE/AFTER riff
prompts for a bit, written to a private folder, no model on --dry-run). Invented corpus and profile only; no network.

    python -m pytest api-dev/tests/test_profile_tools.py -q
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

DESK = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(DESK))
from creative.tests.conftest import make_analyses, make_graph  # noqa: E402

PROFILE = "# Voice (invented)\n\n" + "\n\n".join(
    f"Section {i}: he sits on the punch word, polls the room, repeats the word. Never explains." for i in range(40))
LEGAL = "The petitioner and the respondent appeared; index no 12 was read, sworn to before me."


def run(*args, env=None):
    return subprocess.run([sys.executable, *args], cwd=str(DESK), capture_output=True, text=True, encoding="utf-8",
                          env=env)


def test_put_voice_dry_run_checks_and_sends_nothing(tmp_path):
    p = tmp_path / "profile.md"
    p.write_text(PROFILE, encoding="utf-8")
    r = run("tools/profile/put_voice.py", "--file", str(p), "--dry-run")
    assert r.returncode == 0, r.stderr
    assert "legal rules: clear" in r.stdout and "dry run: nothing sent" in r.stdout
    assert "polls the room" not in r.stdout and "Section 1" not in r.stdout, "the text is never printed"
    (tmp_path / "legal.md").write_text(PROFILE + "\n" + LEGAL, encoding="utf-8")
    r = run("tools/profile/put_voice.py", "--file", str(tmp_path / "legal.md"), "--dry-run")
    assert r.returncode != 0 and "legal rules match" in (r.stdout + r.stderr)
    r = run("tools/profile/put_voice.py", "--file", "README.md", "--dry-run")
    assert r.returncode != 0 and "inside the public repo" in (r.stdout + r.stderr)
    # the owner path without a token says what it needs, never reads argv for one
    r = run("tools/profile/put_voice.py", "--file", str(p), "--as", "owner", env={**os.environ, "PK_OWNER_TOKEN": ""})
    assert r.returncode != 0 and "PK_OWNER_TOKEN" in (r.stdout + r.stderr)


def test_eval_riff_writes_before_and_after_prompts_to_a_private_folder(tmp_path):
    comedy = tmp_path / "comedy"
    (comedy / "kb" / "v1").mkdir(parents=True)
    (comedy / "kb" / "v1" / "graph.json").write_text(json.dumps(make_graph()), encoding="utf-8")
    for key, a in make_analyses().items():
        d = comedy / "corpus" / "sets" / key
        d.mkdir(parents=True)
        (d / "analysis.json").write_text(json.dumps(a), encoding="utf-8")
    prof = tmp_path / "profile.md"
    prof.write_text(PROFILE, encoding="utf-8")
    out = tmp_path / "evals"
    r = run("tools/prompts/eval_riff.py", "--bits", "joke:b,joke:d", "--comedy", str(comedy), "--profile", str(prof),
            "--out", str(out), "--dry-run")
    assert r.returncode == 0, r.stderr
    files = sorted(out.glob("riff_*_b.md")) + sorted(out.glob("riff_*_d.md"))
    assert len(files) == 2
    md = files[0].read_text(encoding="utf-8")
    assert "## BEFORE" in md and "## AFTER" in md and "| BEFORE | | |" in md
    before, after = md.split("## AFTER")
    assert "Section 1" not in before and PROFILE in after, "the profile is only in the AFTER prompt"
    assert 'B1 B bit: "The purple monkey dishwasher line here."' in before, "one sentence per bit, as it was"
    assert "with how they did on stage" in after and "name the move in `move`" in after
    assert "_no drafts: dry run_" in md
    # never into the repo
    r = run("tools/prompts/eval_riff.py", "--bits", "joke:b", "--comedy", str(comedy), "--out", str(DESK / "docs"), "--dry-run")
    assert r.returncode != 0 and "inside the public repo" in (r.stdout + r.stderr)
    r = run("tools/prompts/eval_riff.py", "--bits", "joke:zz", "--comedy", str(comedy), "--out", str(out), "--dry-run")
    assert r.returncode != 0 and "not in the corpus" in (r.stdout + r.stderr)
