"""gsync: Google Docs <-> Punchable sync (see GSYNC.md). `run_once()` is the entry point for worker/."""
from __future__ import annotations

from .config import Config


def run_once(dry_run: bool = False, cfg: Config | None = None) -> dict:
    """One sync pass. Returns the run record (also appended to comedy/kb/gsync/runs.jsonl unless dry_run).
    Never raises for Drive/API trouble: problems are in the record's `errors` / `mode`."""
    from .auth import load_creds
    from .drive import GoogleDrive
    from .punchable import Punchable
    from .sync import Sync
    cfg = cfg or Config()
    try:
        creds = load_creds(cfg)
    except Exception as e:  # noqa: BLE001  a dead refresh token: `python -m gsync auth`
        return {"mode": "no_auth", "errors": [{"where": "auth", "error": f"{type(e).__name__}: {e}"}], "actions": []}
    if creds is None:
        return {"mode": "no_auth", "errors": [{"where": "auth", "error": "no token: run `python -m gsync auth`"}],
                "actions": []}
    return Sync(cfg, GoogleDrive(creds), Punchable(cfg), dry_run=dry_run).run()
