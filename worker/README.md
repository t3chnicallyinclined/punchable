# The worker

The hosted API has no models in it. Anything that needs one (Ask, riffs, punch-ups, bridge drafts, fact checks,
memo linking, the text-only analysis of a set transcribed on the phone) is queued there as a job, and **the worker**
claims it, runs it through `engine.providers`, and posts the result back through `/api/worker/*` with its actor
(`ai:<task>`). The worker is outbound only: it calls Keycloak for a token and the API for jobs. Nothing listens.

Since **2026-10-27 the worker runs on the server** (owner: "we won't use the 3090 for anything; this is going to be
all phone / frontier models"). The PC is retired. Transcription of audio recorded on the phone happens on the phone
(iOS 26 SpeechTranscriber; a separate build); nothing on the server hears audio.

## Hosted (the default)

`infra/k8s/35-worker.yaml`: Deployment `punchable-worker`, image `infra/worker/Dockerfile`, entrypoint
`python -m worker --hosted`. What that mode means (`worker/__main__.py`):

- **online-first**: each job runs on a disposable workspace rebuilt from the API (`worker/online.py`); nothing on the
  pod is the truth; media never lands there;
- **the Anthropic API** is the only provider (`engine/config.hosted.toml`, selected with `ENGINE_CONFIG`): Opus 5.5
  for riffs, punch-ups and the text-only set analysis, Sonnet 5.5 for Ask, bridges, fact checks and clip ranking,
  Haiku 5.5 for the merge judge and memo triage; adaptive thinking on Opus/Sonnet, streamed, the stable system prefix
  cached (`engine/providers.py` `AnthropicBackend`). The key is `ANTHROPIC_API_KEY` from the Secret
  `punchable-anthropic` (`infra/ship.sh ai-secret`); the worker's own credentials are the `pk-worker` client secret
  from the Secret `punchable-worker` (`infra/ship.sh worker-secret`). Neither is ever printed or logged;
- **kinds**: `bridge, punchup, facts, riff, ask_chat, set_text`, plus `embed, memolink, ask` when the bge-small
  model is in the image (it is, unless built with `PK_WITH_BGE=0`). It **never** claims `process_set` or
  `import_youtube` (`TRANSCRIBER_KINDS`): those need whisper, the laugh detector and yt-dlp. The API keeps them queued
  as `waiting_for: "transcriber"` and the app says "transcription happens on your phone for now; this file is waiting";
  nothing is dropped. A job of that kind handed over by mistake is given straight back;
- **a set transcribed on the phone** (`POST /api/sets/transcript`) runs as `set_text`: the engine's split step over
  the transcript (device timestamps, or synthetic ones marked as such), `corpus/sets/<date>_ph<sha9>/{transcript,
  meta, analysis}.json` with every audio-derived field absent and `audio.status: "unavailable"`, and each funny unit
  posted as an idea (`ai:set_text`). Laughs, room heat and timing arrive only when a recording goes through the full
  pipeline;
- **the voice profile** (`creative/voice.py`): fetched from the API before every job (`GET /api/worker/profile/voice`,
  replaced only when `updated_at` moved) and put first in every riff, punch-up, bridge and Ask prompt with the craft
  rules in the same block, where `engine/providers.py` sets the cache breakpoint; the log says its length, sha and
  `updated_at`, never its text. No profile on the server: the log says so and the drafts run without it;
- **refusals** are never silent: a model refusal is one more try on the task's fallback model (`fallbacks` in
  `engine/config.hosted.toml`), then the plain words "Claude declined this one: ..." in the job's `why`;
- **the cost guard** (`engine/budget.py`): `PK_AI_DAILY_USD` (default 2.00) per UTC day, counted from the SDK's
  usage numbers per job kind (`runs.jsonl` records `usd`, `cache_read`, `cache_write`). Over the cap the worker keeps
  polling but takes no job; its claim carries the reason, which the app shows ("AI paused for today: ...") and the
  jobs stay queued. A cap hit mid-job sends the job back to the queue with the reason;
- **liveness**: a heartbeat file (`PK_HEARTBEAT_FILE`) touched every poll and around every job; the Deployment's
  probe restarts the pod when it goes stale;
- **the claim** says who it is (`PK_WORKER_NAME` = the pod), that it is hosted, and which models it writes with;
  `/api/connections` and the app's worker line show that instead of "your PC";
- legal rules apply before any text leaves (the same `gsync/rules.py` check as before, and again on the server).

Deploy steps, verification and rollback: `infra/README.md` ("The hosted worker").

## Local development

The PC mode still exists for development and for a laptop with Claude Code; it is not the plan for anything else.

```powershell
python -m worker --api http://127.0.0.1:8796 --no-auth   # the local dev API (loopback only; auth off)
python -m worker --once                                   # one pass against the hosted API, then exit
python -m worker --online                                 # online-first on this machine (workspace, nothing kept)
```

- Without `--hosted` it uses `engine/config.toml` (claude_code on the comic's subscription, else ollama) and asks for
  the PC kinds, including the audio ones (`BASE_KINDS`): that is the only way to run whisper / the laugh detector /
  a YouTube import by hand now.
- `PK_WORKER_SECRET` (the `pk-worker` client secret) is read from `%USERPROFILE%\.punchable\secrets.env`; optional
  `PK_API`, `PK_TOKEN_URL`, `PK_WORKER_CLIENT_ID`.
- Local dev alternative: `python api-dev/server.py --jobs inproc` runs the same executor in a thread of the local API.
- `worker\install_task.ps1` (register the PC worker as a scheduled task at logon) and `gsync\install_task.ps1` stay
  in the repo as **development-only** installers; nothing hosted depends on them.
- The non-online PC mode also pulls what he did on the phone back into the data folder (decisions, creative edits),
  which `infra/ship.sh dbload` needed before the online-first cutover. Kept for the rollback path
  (`python -m worker.online pull --into DIR` writes everything the database holds into DIR).

## Tests
```
python -m pytest worker/tests -q      # claim policy, hosted mode, budget pause, set_text, the k8s manifest (PyYAML)
python -m pytest engine/tests/test_providers.py -q   # the Anthropic provider with the SDK mocked, the budget guard
```
