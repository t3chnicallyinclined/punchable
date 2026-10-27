# Home worker

The hosted API (punchable.io) has no models in it. Anything that needs one (bridge and punch-up drafts, Ask, new
set processing) is queued there, and this process on the comic's PC runs it through `engine.providers`
(claude_code on his subscription, else ollama, per `engine/config.toml`) and posts the result back.
A YouTube link from the app (`POST /api/import/youtube`) is an `import_youtube` job: the worker runs
`tools/ingest/fetch_youtube.py` (yt-dlp: info.json, captions, audio into `comedy/tapes/raw/<date>_<id>/`),
`tools/ingest/show_dates.py`, then `tools/audio/run_all.py --sets <set>` in the laughs venv (`PK_LAUGHS_PY`, default
`~/.venvs/laughs/Scripts/python.exe`). Downloads happen only here, never in the API process.
It is **outbound only**: it calls Keycloak for a token and punchable.io for jobs. Nothing listens.

Each pass it also pulls what he did on the phone back to the PC:
- **decisions** into `comedy/kb/v0/decisions.jsonl`: append-only (fsync), deduplicated by the line's content hash,
  `undo_of` remapped to local line numbers;
- **creative edits** (storyboards, working sets, venue pins) into `comedy/kb/creative/*.jsonl`, appended when they
  still replay after the local ones; otherwise reported and left alone.
That is what lets `infra/ship.sh dbload` run later without losing phone work (it refuses until this has run).

## Online-first mode (`PK_ONLINE_FIRST=1` or `--online`)
After the cutover the database on the server is the source of truth and this PC keeps nothing as the truth
(`worker/online.py`):
- each job runs on a disposable **workspace** (`<PK_CACHE_DIR>/worker`, or `PK_WORKSPACE`) rebuilt from the API
  (`/api/worker/files`, decisions, creative logs), never on your data folder;
- results go back through `/api/worker/files` (pipeline output, transcripts), `/api/worker/ideas` and
  `/api/worker/events`, each with its actor (`import:youtube`, `import:upload`, `ai:attach`, ...); court material
  is checked with the legal rules here and again on the server;
- every recording is deleted from the workspace after the job (YouTube audio, uploads, decoded wavs), also when the
  job fails; what stays online is the transcript, the jokes and laughs, and a reference (the video id, the Drive
  file, the file's name and date);
- two more job kinds run: `embed` (vectors for new notes) and `memolink` (memo <-> bit linking, offline Claude);
- nothing is pulled into a data folder. `python -m worker.online pull --into DIR` writes everything the database
  holds into DIR (the rollback path).
gsync and the voice-memo processor follow the same switch (`PK_ONLINE_FIRST=1` in `~/.punchable/punchable.env`,
then re-run both `install_task.ps1`): they write through the worker routes with the worker's token, post their own
state, and keep no memo audio.

## One-time setup
1. After `infra/ship.sh keycloak` has created client `pk-worker`: Keycloak admin console (https://auth.punchable.io,
   realm **punchable**) > Clients > `pk-worker` > Credentials > copy the client secret.
2. Put it in `%USERPROFILE%\.punchable\secrets.env` (the file that already holds the SurrealDB credentials; never
   committed, never printed):
   ```
   PK_WORKER_SECRET=<paste>
   ```
   Optional overrides: `PK_API` (default https://punchable.io), `PK_TOKEN_URL`, `PK_WORKER_CLIENT_ID` (pk-worker).
3. Check it once by hand:
   ```powershell
   cd <repo>
   python -m worker --once
   ```

## Run it
```powershell
python -m worker                    # forever: pulls every 60 s, polls for jobs every 15 s when idle
python -m worker --once             # one pass (kinds: bridge,punchup,ask,process_set,import_youtube,facts,riff,ask_chat)
python -m worker --api http://127.0.0.1:8796 --no-auth   # the local dev API instead (loopback only)
```
**At logon, as a scheduled task** (restarts on failure, runs only while he is logged on, no admin rights needed):
```powershell
powershell -ExecutionPolicy Bypass -File worker\install_task.ps1            # register "Punchable worker"
powershell -ExecutionPolicy Bypass -File worker\install_task.ps1 -Remove    # unregister
Get-ScheduledTask "Punchable worker" | Get-ScheduledTaskInfo                # last run / result
```
Output goes to `comedy\db\punchable\logs\worker.log`.

Local dev alternative: `python api-dev/server.py --jobs inproc` runs the same executor in a thread of the local API.

## Tests
```
python -m pytest worker/tests -q
```
