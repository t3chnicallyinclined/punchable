"""python -m gsync auth | status | run [--dry-run] [--watch [--interval 60]] | undo <run_id>"""
from __future__ import annotations

import argparse
import json
import sys
import time
from collections import Counter

from . import run_once
from .config import Config
from .store import Store, read_jsonl


def _summary(rec: dict) -> str:
    c = ", ".join(f"{k} {v}" for k, v in sorted((rec.get("counts") or {}).items())) or "nothing to do"
    errs = len(rec.get("errors") or [])
    return f"{rec.get('run_id', '-')} [{rec.get('mode')}{' DRY RUN' if rec.get('dry_run') else ''}] {c}" + \
        (f"; {errs} error(s): " + "; ".join(e.get("error", "")[:160] for e in rec["errors"][:3]) if errs else "")


def cmd_status(cfg: Config) -> int:
    from .auth import token_info
    missing = []
    print("gsync status")
    try:
        import google_auth_oauthlib  # noqa: F401
        import googleapiclient  # noqa: F401
        print("  libraries          ok")
    except ImportError:
        print("  libraries          MISSING")
        missing.append(r"use the venv: ~\.venvs\gsync\Scripts\python -m gsync ...")
    t = token_info(cfg)
    if t["desktop_fallback"]:
        print(f"  auth mode          desktop fallback ({cfg.token_file})")
    else:
        print(f"  google-oauth.env   {'ok' if t['google_env_keys'] else ('INCOMPLETE' if t['google_env'] else 'MISSING')}  {cfg.google_env}")
        if not t["google_env_keys"]:
            missing.append("SETUP.md steps 1-5: Google Cloud project + Web client; put GOOGLE_CLIENT_ID / "
                           "GOOGLE_CLIENT_SECRET in that file")
        print(f"  Keycloak sign-in   {'ok' if t['keycloak_signed_in'] else 'MISSING'}  {cfg.kc_token_file}")
        if not t["keycloak_signed_in"]:
            missing.append("SETUP.md steps 6-9 (realm on the server, link Google), then: python -m gsync auth")
    if t["keycloak_signed_in"] or t["desktop_fallback"]:
        try:
            from .auth import load_creds
            load_creds(cfg)
            print("  Google token       ok (refreshed just now)")
        except Exception as e:  # noqa: BLE001  messages carry the fix, never a token
            print(f"  Google token       FAILED: {e}")
            missing.append("if this fails about every 7 days, the consent screen is still 'Testing': SETUP.md step 3c")
    from .punchable import Punchable
    up = Punchable(cfg).health()
    print(f"  Punchable API      {'ok' if up else 'NOT RUNNING'}  {cfg.api}")
    if not up:
        missing.append("start the local API: python api-dev/server.py")
    st = Store(cfg.state)
    m = st.m
    origins = Counter(e.get("origin") for e in m["docs"].values())
    print(f"  map                {cfg.state / 'map.json'}")
    print(f"    bootstrapped     {m.get('bootstrapped_at') or 'not yet (the first run bootstraps)'}")
    print(f"    docs             {dict(origins)}; hidden {sum(1 for e in m['docs'].values() if e.get('hidden'))}")
    print(f"    legal excluded   {len(m.get('excluded', {}))} (IDs only; never exported)")
    print(f"    not comedy       {len(m.get('skipped', {}))}; pending {len(m.get('pending', {}))}")
    print(f"    boards           {len(m.get('boards', {}))}")
    runs = read_jsonl(cfg.state / "runs.jsonl")
    print(f"  last run           {_summary(runs[-1]) if runs else 'never'}")
    from .memos import MemoStore, processor_running, progress
    print(f"  voice memos        {progress(MemoStore(cfg.memos_state).load())['line']}"
          f"{' (transcribing now)' if processor_running(cfg) else ''}")
    conf = read_jsonl(cfg.state / "conflicts.jsonl")
    if conf:
        print(f"  conflicts          {len(conf)} (comedy/kb/gsync/conflicts.jsonl); latest:")
        for c in conf[-5:]:
            print(f"    {c.get('ts')}  {c.get('kind')}  {c.get('title')}")
    if missing:
        print("\nTo do:")
        for n, x in enumerate(missing, 1):
            print(f"  {n}. {x}")
        return 1
    print("\nReady.")
    return 0


def cmd_undo(cfg: Config, run_id: str) -> int:
    from .punchable import ApiError, Punchable
    runs = [r for r in read_jsonl(cfg.state / "runs.jsonl") if r.get("run_id") == run_id]
    if not runs:
        print(f"no run {run_id} in runs.jsonl")
        return 1
    idx = sorted({i for a in runs[-1].get("actions", []) for i in (a.get("indexes") or [])}, reverse=True)
    pk = Punchable(cfg)
    done = 0
    for i in idx:
        try:
            pk.undo([i])
            done += 1
        except ApiError as e:
            print(f"  line {i}: {e}")
    print(f"undid {done} of {len(idx)} Punchable change(s) from {run_id}. Google Docs were not changed.")
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="python -m gsync")
    sub = ap.add_subparsers(dest="cmd", required=True)
    au = sub.add_parser("auth", help="sign in (once): Punchable's 'Sign in with Google' via Keycloak")
    au.add_argument("--desktop", action="store_true", help="fallback: a separate Desktop-app Google client")
    sub.add_parser("status", help="what is set up and what is missing")
    r = sub.add_parser("run", help="one sync pass")
    r.add_argument("--dry-run", action="store_true")
    r.add_argument("--watch", action="store_true", help="keep running")
    r.add_argument("--interval", type=int, default=60)
    u = sub.add_parser("undo", help="undo one run's Punchable changes")
    u.add_argument("run_id")
    a = ap.parse_args(argv)
    cfg = Config()
    if a.cmd == "auth":
        from .auth import run_auth
        print("signed in; token saved (owner-only):", run_auth(cfg, desktop=a.desktop))
        return 0
    if a.cmd == "status":
        return cmd_status(cfg)
    if a.cmd == "undo":
        return cmd_undo(cfg, a.run_id)
    while True:
        rec = run_once(dry_run=a.dry_run, cfg=cfg)
        print(_summary(rec), flush=True)
        if a.dry_run:
            for x in rec.get("actions", [])[:200]:
                print("  ", json.dumps(x, ensure_ascii=False))
        if not a.watch:
            return 0 if not rec.get("errors") else 2
        time.sleep(max(15, a.interval))


if __name__ == "__main__":
    sys.exit(main())
