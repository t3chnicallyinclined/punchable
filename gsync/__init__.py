"""gsync: Google Docs <-> Punchable sync (see GSYNC.md). `run_once()` is the entry point for worker/."""
from __future__ import annotations

from .config import Config


def run_once(dry_run: bool = False, cfg: Config | None = None) -> dict:
    """One sync pass. Returns the run record (also appended to comedy/kb/gsync/runs.jsonl unless dry_run).
    Never raises for Drive/API trouble: problems are in the record's `errors` / `mode`."""
    from .auth import load_creds
    from .drive import GoogleDrive
    from .punchable import make
    from .sync import Sync
    cfg = cfg or Config()
    try:
        creds = load_creds(cfg)
    except Exception as e:  # noqa: BLE001  a dead refresh token: `python -m gsync auth`
        return {"mode": "no_auth", "errors": [{"where": "auth", "error": f"{type(e).__name__}: {e}"}], "actions": []}
    if creds is None:
        return {"mode": "no_auth", "errors": [{"where": "auth", "error": "no token: run `python -m gsync auth`"}],
                "actions": []}
    pk = make(cfg)
    if not dry_run:
        try:
            pk.pull_state()  # online-first on a fresh PC: gsync's state comes back from the database
        except Exception:  # noqa: BLE001  the run reports api_down itself
            pass
    rec = Sync(cfg, GoogleDrive(creds), pk, dry_run=dry_run).run()
    if not dry_run:
        try:  # online-first: gsync's state (map, memos, runs) is posted to the database; local mode: nothing
            pk.push_state(force_runs=bool(rec.get("actions")) or bool(rec.get("errors")))
        except Exception as e:  # noqa: BLE001
            rec.setdefault("errors", []).append({"where": "push_state", "error": f"{type(e).__name__}: {e}"[:300]})
    return rec
