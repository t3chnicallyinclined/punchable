# pk-gsync: gsync in Rust

`pk-gsync` is the program on the owner's PC that pulls his Google Docs into Punchable (and writes the docs Punchable makes
back to Drive). It is a port **as-is** of `gsync/` (Python), except where the Python's legal check failed open (round A0, below): the same command line, the same requests in the same order,
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

### Court and legal documents: the check fails closed (T11 round A0)

The Python's legal check had places where a problem switched a net off and the doc went through. They are closed here,
and each is a departure from the "port as-is" rule, on purpose (`gw-security` must-fixes 2, 7 and 8 of the T11 plan,
section 8). In every case the doc is excluded or left closed, and the log and the state say the **kind** of reason
(`denylist`, `folder`, `owner`, `kw`, `filing`, `content`, `parent`, `check_error`), never a word from his private rules.

| The Python | pk-gsync | Test (`tests/legal.rs`) |
|---|---|---|
| A rules file that is missing, unreadable or not JSON became the generic rules only. | **`run` refuses to start** and says which file and why (nothing printed from inside it); no request is made. The file is always required (`PUNCHABLE_LEGAL_RULES`, else `legal_rules.json` in the home folder). `status` and `undo` do not read it. In `--watch` the file is read again before **every** pass, so an edit takes effect at once and a file that goes missing or broken stops the loop (exit 1; the task restarts it each minute and it refuses until the file is back). | `a_missing_rules_file_stops_run_before_any_request`, `an_unusable_rules_file_stops_run_...`, `a_rules_file_that_goes_missing_between_passes_stops_the_loop` |
| With no owner address the "not his doc" net was off. | No owner address (in the file or `PUNCHABLE_OWNER_EMAIL`) is a refusal to start; and in the check itself, with no owner known every doc is `owner`, so it is closed. A rules file with none of his own folders, doc ids or keywords is also refused. | `an_unusable_rules_file_stops_run_...`, `a_doc_by_anyone_is_closed_when_no_owner_is_known`, `the_owner_address_may_come_from_the_environment` |
| A parent folder Drive did not return was cached as "no parents", so the folder net passed the doc. | The doc is excluded (`parent`) and nothing is cached for the missing folder. | `a_parent_folder_drive_does_not_return_closes_the_doc_and_is_not_cached` |
| An error inside the legal check was retried next pass, with Drive's error text in the run record. | The doc stays closed that pass, is looked at again next pass (the failure may be a dropped connection), and the record says only `legal check failed: <error kind>`. A folder Drive answers 404 for is final (`parent`); a transient error is not. | `an_error_inside_the_legal_check_opens_nothing_and_records_only_its_kind` |
| The reason stored in `map.json` (copied to the database and to the backups) included the words that matched, `kw:<his keyword>`. | Only the kind is stored (`kw`). A map written by the Python or an older pk-gsync is cut to kinds the next time a pass opens it. | `the_keyword_that_matched_never_reaches_the_map_or_the_state_sent_to_the_database` |
| Export and download had no clearance; a doc was opened first and looked at after, and some sites never checked the text. | `GoogleDrive::export` and `download` take a `Clearance`, which only `Rules::clear` makes, from the file's metadata read just now. No call site can skip it. Every site goes through it: the pull, the three kinds of doc gsync tracks (AI drafts, mixed, merged), the baseline of an adopted doc, a doc gsync just made, an app edit, a storyboard doc, and every voice-memo download (its metadata is read again before the bytes are fetched). | the tests below, and the type |
| The content net (two or more court markers in the text) ran at one site, after export. | It runs on every export, **before** the text is hashed, stored in `edits.jsonl` or the mirror, or sent: pull, baseline, AI drafts, merged docs, docs gsync made, board docs. A doc it drops is recorded as `content`; its text is kept nowhere. | `an_ai_draft_doc_...`, `a_merged_doc_...`, `an_adopted_doc_...` |
| gsync read back and rewrote its own docs (a joke's doc, a storyboard doc) without a legal check. | The check runs on fresh metadata first. A doc the rules close (for example one moved into a legal folder) is neither read nor written, and is recorded as excluded. | `a_doc_gsync_just_made_is_not_read_back_...`, `an_app_edit_is_not_written_...`, `a_board_doc_is_neither_read_nor_rewritten_...`, `a_listed_voice_memo_in_a_legal_folder_is_not_downloaded` |

What an owner sees when the rules file is missing or broken: the scheduled task's log (`gsync.log`) and `pk-gsync run`'s
stderr say, for example, `the legal rules file <path> cannot be read (...); pk-gsync will not run without it`, and the
program exits 1 before it asks Google or Punchable for anything. The task stays registered and tries again at its next
start; nothing is synced until the file is back.

Not in round A0 (the plan's later rounds): the write gate still trusts the map's list of docs gsync made (`adopt` in
`sync.rs`; A1 and A2 authenticate the state), and the rules fingerprint is not yet logged or pinned.

### The recorded tapes were edited, on purpose

The tapes are the Python's recording. Two places differ now, listed with their reasons in
`rust/tools/gsync_tape_edits.py` (the generator applies them after recording, so `--check` holds the tapes to "the
Python's recording plus exactly these edits"): (E1) a legal doc's `excluded` reason in `map.json` is the kind only; (E2) a
voice memo's metadata is read once more before its bytes are downloaded, and the run record's `drive.calls` grows by that
count. Every other recorded request, answer and file is unchanged.

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

`tests/legal.rs` covers what the Python got wrong and the tapes therefore cannot show (the table above), on small invented
servers. Red arms of the tapes (each changes one thing and must make the replay fail): a changed recorded POST body, a weaker legal rule (a
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
