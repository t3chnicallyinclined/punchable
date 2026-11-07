# pk-gsync: gsync in Rust

`pk-gsync` is the program on the owner's PC that pulls his Google Docs into Punchable (and writes the docs Punchable makes
back to Drive). It is a port **as-is** of `gsync/` (Python): the same command line, the same requests in the same order,
the same state files, the same token files, the same words in the log. T11 round 4. The Python stays installed, and its
scheduled task stays registered (disabled), until the T11 rollback window closes.

```
pk-gsync auth                      sign in once (Keycloak device flow, "Sign in with Google")
pk-gsync status                    what is set up and what is missing
pk-gsync run [--dry-run]           one sync pass; --dry-run reads and plans, writes nothing
pk-gsync run --watch [--interval 60]   keep running (the scheduled task)
pk-gsync undo <run_id>             undo one run's Punchable changes
```

Build: `cd rust && cargo build --release -p pk-gsync` (it is the lightest binary in the workspace: no SurrealDB, no model).

## What is the same

| | |
|---|---|
| Google | Drive v3 through REST: the same calls, query parameters, bodies, retry rule (8 tries, `min(64, 2^n)` plus jitter on 429/5xx and rate-limit 403), 401 refresh-and-retry, 8 MiB ranged downloads |
| Sign-in | Keycloak offline token (`keycloak_gsync_token.json`, rotated on every use), the broker's Google token, the cached Google token (`google_token.json`), `google-oauth.env`; all by reference, owner-only (`icacls` on Windows), never printed |
| Punchable | the hosted API's `/api/worker/*` routes with the worker's Keycloak client-credentials token (`PK_WORKER_SECRET` from `secrets.env`), actor `import:gsync` |
| State | `<comedy>/kb/gsync/{map.json,units.json,memos.json,runs.jsonl,conflicts.jsonl,edits.jsonl,mirror/}`, byte for byte what `json.dumps` printed; either program can read what the other wrote |
| Rules | the legal rules (`legal_rules.json`), triage, settle time, loop guards, conflicts, merges, boards, adoption (`gsync_docs.json`) |
| Memos | the Drive half (list, skip legal, dedupe by md5, disk guard, download a batch) and the start of the processor |

## What is different (on purpose)

- **Local-API mode is gone.** `PK_ONLINE_FIRST=1` is required for `run` (the PC task runs that way). Without it `run` stops
  with the reason. `status` and `undo` still talk to the local API at `PUNCHABLE_API`, exactly as the Python's do.
- **`auth --desktop` is not ported.** The fallback client flow (a separate Desktop-app client with a local redirect) is not
  in Rust. A Desktop token file (`google_token.json` with a `client_id`) is still used by `run` and `status`. To make a
  new one run the Python `python -m gsync auth --desktop` once.
- **The voice-memo transcriber stays Python.** `python -m gsync.memos process` (ffmpeg and faster-whisper) is started by
  `pk-gsync` exactly as the Python started it (same arguments, same working folder, same log), reads and writes the same
  `memos.json` under the same lock file. It needs the `gsync/` package, so `gsync/` cannot be deleted from the PC until
  that processor has a Rust or server replacement (named in the T11 plan, section 1.4: PC-only tools come later).
- **A dry run is read-only in code.** The Drive and Punchable clients of a `--dry-run` refuse every write before a request is
  made (`WriteBlocked`). In the Python a dry run avoids writes by care; here it also cannot write.
- **Wording that cannot match.** A transport failure (connection refused, TLS) reads differently (`reqwest`'s words, not
  `urllib`'s). The Python traceback in a `run` error record (`trace`) is a fixed sentence. Google's own token endpoint is
  not retried on 5xx (google-auth retries with backoff); the next pass tries again in 60 s.

## The proof

`tests/differential.rs` replays tapes the Python recorded. `rust/tools/gen_gsync_tapes.py` runs the real Python gsync
(the gsync venv: google-api-python-client, google-auth) against fake Google, Keycloak and Punchable servers on
127.0.0.1 and records, for every CLI call, the files to change first, every HTTP request in order with the answer, the exit
code, the printed text and every file left behind. The Rust replay starts from the same files and answers each request
from the tape; it fails on the first request that is not the next recorded one (method, path, query, bearer token,
content type, range, JSON/form/multipart body) and on any difference in exit code, printed text or any file.

Regenerate (the Python is frozen, so only when a scenario changes):
`~/.venvs/gsync/Scripts/python.exe rust/tools/gen_gsync_tapes.py` (add `--check` to see that the tapes are current).
Everything in the tapes is invented: tokens, Doc ids and titles, texts. The Doc host name is stored as `{GDOCS_HOST}` so
no Doc URL is in the repo.

Red arms (each changes one thing and must make the replay fail): a changed recorded POST body, a weaker legal rule (a
blocked doc is exported), a changed recorded file, a request the tape lacks, a request pk-gsync does not make.
`tests/behaviour.rs` covers what a tape cannot: a dry run cannot write, the command line's refusals, chunked downloads,
no token value printed.

## Switch the scheduled task over (the lead does this with the owner)

The Python task is `Punchable gsync` (`gsync/install_task.ps1`). The Rust task is a **different name**,
`Punchable gsync (Rust)`, so the Python task is the way back. **Only one may be enabled**: both write the same state
files and rotate the same Keycloak token.

1. Build and install the binary:
   `cd rust; cargo build --release -p pk-gsync` then copy `rust\target\release\pk-gsync.exe` to `%USERPROFILE%\.punchable\bin\`.
2. Do the dry run below and compare. Switch only when the two outputs are equal.
3. Stop and disable the Python task, and check that no `python -m gsync run` is left:
   `Stop-ScheduledTask 'Punchable gsync'; Disable-ScheduledTask 'Punchable gsync'`, then
   `Get-CimInstance Win32_Process | Where-Object CommandLine -match 'gsync run'` lists nothing.
4. Register and start the Rust task (`-WhatIf` first prints the command line and registers nothing):
   `powershell -ExecutionPolicy Bypass -File rust\crates\pk-gsync\install_task.ps1 -WhatIf`, then without `-WhatIf`.
   It runs `pk-gsync run --watch --interval 60`, log `<data folder>\db\punchable\logs\gsync.log` (the same file).
5. Check: the log's next line reads `gs-... [pull] ...` with no `error(s)`; `pk-gsync status` ends with `Ready.`;
   `kb/gsync/runs.jsonl` has a new line. The voice-memo line in `status` still counts.
6. Roll back at any time (the state files are the same): `Stop-ScheduledTask 'Punchable gsync (Rust)'; Disable-ScheduledTask
   'Punchable gsync (Rust)'; Enable-ScheduledTask 'Punchable gsync'; Start-ScheduledTask 'Punchable gsync'`.

## The dry run against a throwaway Doc (prepared, not run)

What a `pk-gsync run --dry-run` does: **reads only.** On Drive: `changes.getStartPageToken`, `changes.list`,
`files.list`, `files.get`, `files.export`, all GET. On Punchable: GETs of `/api/health`, `/api/worker/decisions`,
`/jokes/<id>`, `/creative`, `/files`. Keycloak and Google token POSTs (every run needs them). It writes, locally, only
`memos.json` (the Voice memos folder id) and `memos.processor.lock` in the data folder it is given. It never touches his
real Docs and never writes to production, and that is checked three ways: the code refuses writes
(`GoogleDrive`/`Hosted` built read-only for a dry run; `tests/behaviour.rs`), the recorded dry-run tapes contain no
request but GET and token forms (`a_dry_run_makes_no_write_request_to_google_or_the_hosted_api`), and the checks in step 6.

Two things the dry run does do to his real files, as any run does: the Keycloak refresh token rotates, and
`google_token.json` is rewritten (so **always run with the real `PUNCHABLE_HOME`, never a copy**: a copied token file would
be one rotation behind and would break the real task).

Nothing below prints a token. Doc ids and titles stay in the terminal; do not paste them anywhere public.

1. **Disable the Python task first** (`Stop-ScheduledTask 'Punchable gsync'; Disable-ScheduledTask 'Punchable gsync'`).
   Otherwise the running task would ingest the throwaway Doc as a real idea within three minutes.
2. **Make the throwaway Doc** in Google Docs: a new blank Doc titled `gsync dry run`, with two invented lines (or File >
   Make a copy of one comedy Doc, renamed `gsync dry run`). Leave it in My Drive. Wait three minutes (gsync waits two
   minutes for a Doc to settle; a Doc newer than that is reported as `settling`, not `new`).
3. **Load his settings and find the data folder:**
   ```powershell
   cd <repo>
   . .\tools\local_env.ps1                    # sets COMEDY_DIR, PK_CACHE_DIR, PK_ONLINE_FIRST, ... in this session
   $Real = if ($env:PK_WORKSPACE) { Join-Path $env:PK_WORKSPACE 'comedy' } else { Join-Path $env:PK_CACHE_DIR 'worker\comedy' }
   $Scratch = Join-Path $env:LOCALAPPDATA 'Temp\gsync-dryrun'
   $before = (Get-FileHash "$Real\kb\gsync\map.json").Hash
   ```
4. **Two scratch copies of the state** (only `map.json` and `memos.json`; the changes token in `map.json` is where the
   real task stopped, so the dry run sees exactly the changes the real task would see next):
   ```powershell
   foreach ($side in 'py','rs') {
     New-Item -ItemType Directory -Force "$Scratch-$side\comedy\kb\gsync" | Out-Null
     Copy-Item "$Real\kb\gsync\map.json","$Real\kb\gsync\memos.json" "$Scratch-$side\comedy\kb\gsync\"
   }
   ```
5. **Run both dry runs** (`PK_WORKSPACE` moves the data folder; everything else, including the tokens, is the real one):
   ```powershell
   $env:PK_WORKSPACE = "$Scratch-py"; & "$env:USERPROFILE\.venvs\gsync\Scripts\python.exe" -m gsync run --dry-run | Out-File "$Scratch-py.txt" -Encoding utf8
   $env:PK_WORKSPACE = "$Scratch-rs"; & "$env:USERPROFILE\.punchable\bin\pk-gsync.exe" run --dry-run | Out-File "$Scratch-rs.txt" -Encoding utf8
   $strip = { (Get-Content $args[0]) -replace '^gs-\d{8}-\d{6}', 'gs-RUN' }
   Compare-Object (& $strip "$Scratch-py.txt") (& $strip "$Scratch-rs.txt")      # no output = the same plan
   ```
   The first line is the summary (`[pull DRY RUN] new 1, ...`), then one line per planned action. Expect `new 1` for
   the throwaway Doc (and whatever else changed in his Drive since the real task last ran: both sides see the same).
6. **Prove nothing was written:**
   - `(Get-FileHash "$Real\kb\gsync\map.json").Hash -eq $before` is `True` (the real state is untouched);
   - in each scratch folder, only `kb\gsync\memos.json` (and `memos.processor.lock`) changed: `Get-ChildItem -Recurse "$Scratch-rs" -File | Select FullName, LastWriteTime`;
   - in Google Docs, the throwaway Doc's version history shows no version newer than the one you typed, and its name is unchanged;
   - in the Punchable app the Bits count did not move.
7. **Clean up:** trash the throwaway Doc **before** re-enabling the task (an untrashed Doc would be ingested as an idea),
   remove the scratch folders by exact path (`Remove-Item -Recurse -Force "$Scratch-py","$Scratch-rs","$Scratch-py.txt","$Scratch-rs.txt"`),
   then `Enable-ScheduledTask 'Punchable gsync'; Start-ScheduledTask 'Punchable gsync'` (or go on to the switch-over above).
