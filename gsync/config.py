"""Paths and constants. Env overrides: COMEDY_DIR, PUNCHABLE_API, PUNCHABLE_HOME, GSYNC_SCOPES (full|narrow)."""
from __future__ import annotations

import os
from dataclasses import dataclass, field
from pathlib import Path

import json as _json


def _local_docs() -> dict:
    """Your own Drive ids live in a private file, never in the repo:
    $PUNCHABLE_HOME/gsync_docs.json (default ~/.punchable/gsync_docs.json), e.g.
      {"folder_id": "<the Punchable folder id>", "board_docs": {"sb:...": "<doc id>"}, "ai_draft_docs": {"<doc id>": "title"},
       "mixed_docs": {"<doc id>": "joke:..."}, "adopted_joke_docs": {"<doc id>": "joke:..."}}
    Without it the sample ids below are used (they match nothing in a real Drive; the tests use them)."""
    p = Path(os.environ.get("PUNCHABLE_HOME", Path.home() / ".punchable")) / "gsync_docs.json"
    try:
        return _json.loads(p.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


_LOCAL = _local_docs()
FOLDER_ID = _LOCAL.get("folder_id", "sample-folder-punchable-0000000000")  # the "Punchable" folder in your Drive
BOARD_DOCS = _LOCAL.get("board_docs", {"sb:robot-vacuum-104b6cc0": "sample-board-doc-robot-vacuum-000000000000"})
AI_DRAFT_DOCS = _LOCAL.get("ai_draft_docs", {"sample-ai-draft-doc-0000000000000000000000":
                                              "AI explained step by step (DRAFT walk-through for the AI chunk)"})
# Your words above the first horizontal rule, "Punchable notes (not your words)" (ai_draft) below it. Never written.
MIXED_DOCS = _LOCAL.get("mixed_docs", {"sample-mixed-doc-00000000000000000000000000": "joke:good-data-the-soup-label"})
# Per-joke docs made by hand through a connector (your verbatim text): adopted, never pushed again.
ADOPTED_JOKE_DOCS = _LOCAL.get("adopted_joke_docs", {
    "sample-adopted-doc-1-000000000000000000000": "joke:robot-guesses-the-next-word",
    "sample-adopted-doc-2-000000000000000000000": "joke:the-room-finishes-the-sentence",
})
DOC_MIME = "application/vnd.google-apps.document"
FOLDER_MIME = "application/vnd.google-apps.folder"
SCOPES_FULL = ["https://www.googleapis.com/auth/drive"]
SCOPES_NARROW = ["https://www.googleapis.com/auth/drive.readonly", "https://www.googleapis.com/auth/drive.file"]
SETTLE_S = 120          # a doc modified this recently is parked and retried (he may still be typing)
IDEA_MAX = 4000         # POST /api/ideas text limit
# Voice memos (gsync/memos.py): his iPhone Voice Memos, saved to Drive > Punchable > "Voice memos"
MEMO_FOLDER_NAME = "Voice memos"
MEMO_RESERVE_MB = 600   # never download or transcode when C: would drop below this much free space
MEMO_STAGE_MB = 300     # at most this much downloaded-but-not-yet-transcribed audio on disk at once
MEMO_BATCH = 25         # downloads per gsync pass


@dataclass
class Config:
    comedy: Path = field(default_factory=lambda: Path(os.environ.get("COMEDY_DIR", Path.home() / "punchable-data")))
    api: str = field(default_factory=lambda: os.environ.get("PUNCHABLE_API", "http://127.0.0.1:8796"))
    home: Path = field(default_factory=lambda: Path(os.environ.get("PUNCHABLE_HOME", Path.home() / ".punchable")))
    scopes_mode: str = field(default_factory=lambda: os.environ.get("GSYNC_SCOPES", "full"))
    folder_id: str = FOLDER_ID
    board_docs: dict = field(default_factory=lambda: dict(BOARD_DOCS))
    ai_draft_docs: dict = field(default_factory=lambda: dict(AI_DRAFT_DOCS))
    mixed_docs: dict = field(default_factory=lambda: dict(MIXED_DOCS))
    adopted_joke_docs: dict = field(default_factory=lambda: dict(ADOPTED_JOKE_DOCS))
    settle_s: int = SETTLE_S
    memos_enabled: bool = field(default_factory=lambda: os.environ.get("PK_GSYNC_MEMOS", "1") != "0")
    memo_reserve_mb: int = field(default_factory=lambda: int(os.environ.get("PK_MEMO_RESERVE_MB", MEMO_RESERVE_MB)))
    memo_stage_mb: int = MEMO_STAGE_MB
    memo_batch: int = MEMO_BATCH
    # the memo transcriber runs in a Python that has faster-whisper (the gsync venv does not)
    whisper_python: str = field(default_factory=lambda: os.environ.get("PK_WHISPER_PY", "python"))

    @property
    def scopes(self) -> list[str]:
        return SCOPES_NARROW if self.scopes_mode == "narrow" else SCOPES_FULL

    @property
    def state(self) -> Path:
        return self.comedy / "kb" / "gsync"

    auth_url: str = field(default_factory=lambda: os.environ.get("PK_AUTH_URL", "https://auth.punchable.io/realms/punchable"))
    kc_client: str = "pk-gsync"

    @property
    def google_env(self) -> Path:  # GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET (the same Web client Keycloak uses)
        return self.home / "google-oauth.env"

    @property
    def kc_token_file(self) -> Path:  # pk-gsync's Keycloak offline refresh token
        return self.home / "keycloak_gsync_token.json"

    @property
    def client_file(self) -> Path:  # fallback only: a Desktop-app client for `auth --desktop`
        return self.home / "google_client.json"

    @property
    def token_file(self) -> Path:
        return self.home / "google_token.json"

    @property
    def corpus_gdocs(self) -> Path:
        return self.comedy / "corpus" / "gdocs"

    @property
    def graph(self) -> Path:
        return self.comedy / "kb" / "v1" / "graph.json"

    @property
    def decisions(self) -> Path:
        return self.comedy / "kb" / "v0" / "decisions.jsonl"

    @property
    def storyboards(self) -> Path:
        return self.comedy / "kb" / "creative" / "storyboards.json"

    @property
    def merges(self) -> Path:  # the API's merge store snapshot (api-dev/merge_api.py); read-only here
        return self.comedy / "kb" / "creative" / "merges.json"

    @property
    def memos_state(self) -> Path:  # comedy/kb/gsync/memos.json: every voice memo, its status and where it went
        return self.state / "memos.json"

    @property
    def notes_inbox(self) -> Path:  # downloads land here; he can also drop audio files here by hand
        return self.comedy / "notes" / "inbox"

    @property
    def memo_audio(self) -> Path:  # tapes/processed/<date>_<sha8>/audio.opus (the engine's place for recordings)
        return self.comedy / "tapes" / "processed"

    @property
    def memo_corpus(self) -> Path:  # corpus/memos/<date>_<sha8>/transcript.json|md, meta.json (the engine's layout)
        return self.comedy / "corpus" / "memos"
