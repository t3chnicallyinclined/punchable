# Punchable engine, layer 1

An importable Python package (`engine/`) plus two CLIs. The API calls these
functions directly; nothing here serves HTTP. Python 3.13, run from the
`punchable` folder (`python -m engine.<module>`).

Data contract: `docs/KB_SCHEMA.md`. **`decisions.jsonl` is the comic's**: the engine only
ever opens it for reading. It writes `graph.json` (atomically, tmp + rename),
`runs.jsonl`, `inbox_ledger.jsonl` and `engine_cache/` in the kb folder, transcripts and
laughs under `corpus/sets|memos/<date>_<sha8>/`, and moves audio into
`tapes/processed/<date>_<sha8>/`. It never deletes audio.

## Configuration: `engine/config.toml`

- `[paths]`: comedy folder, kb, corpus, inbox, processed, tools, the bge-small model,
  the laughs venv, `secrets_env`.
  - `Config(root=DIR)` / `--root DIR` puts every **written** path (kb, inbox,
    processed, corpus output) under DIR. That is how tests and trial runs use temp copies.
- `[providers.<name>]`: `kind` = `openai_compatible` | `anthropic` | `claude_code`.
- `[tasks.<task>]`: `providers` (a preference list: first available wins, and an error
  falls through to the next), `models` per provider, `offline`, `reasoning_effort`.
- Tasks: `split`, `match_judge`, `propose_joke`, `label`, `link_judge` (offline, inbox);
  `ask_plan`, `ask_answer` (request path).

## 1. Providers: `engine/providers.py`

```python
from engine.providers import complete, Router
c = complete(task, messages, schema=None, run_id=None, fewshot_sha=None, request_path=False)
c.data          # parsed JSON, validated against `schema` (one retry on invalid JSON)
c.text, c.provider, c.model, c.tokens_in, c.tokens_out, c.ms, c.call_id, c.prompt_sha

r = Router(Config(...))      # a router bound to one config (the API should hold one)
r.resolve(task)              # [(provider, model)] that are available now, in order
r.calls                      # this router's call records (stage accounting)
```

- `messages` are `{"role": "system"|"user"|"assistant", "content": str}`.
- Every call, including a failed one, is appended to `<kb>/runs.jsonl`:
  `ts, call_id, run_id, task, provider, model, prompt_sha, fewshot_sha, tokens_in, tokens_out, ms, ok[, error]`.
  Cost per hour of audio = token sums per `run_id` over the audio length in the inbox
  `run.json`.

Backends:

| kind | used when | notes |
|---|---|---|
| `openai_compatible` | the endpoint answers `/models` | Ollama is the default (`gpt-oss:20b`). `response_format: json_schema`. Runs `ollama serve` if it is down. Also covers LM Studio, vLLM, or a `custom` endpoint (`ENGINE_CUSTOM_BASE_URL`, `ENGINE_CUSTOM_API_KEY`). |
| `anthropic` | `ANTHROPIC_API_KEY` is in the environment **or** in `~\.punchable\secrets.env` (`ANTHROPIC_API_KEY=...`) | Official SDK, model `claude-opus-5-5`, `messages.create(..., thinking={"type": "adaptive"}, output_config={"format": {"type": "json_schema", "schema": ...}})` (parameter names checked in SDK 0.104.1 source). The key is never logged and never written to the repo. With no key, routing falls back to ollama. |
| `claude_code` | `enabled = true` **and** the task is `offline = true` **and** not `request_path` | **Personal use on the comic's own machine via the official CLI; not for serving other users.** Runs `claude -p --output-format json --tools "" --no-session-persistence --strict-mcp-config --model <m>` with the prompt and schema instructions on stdin, then validates the JSON itself. Concurrency 1-2 (semaphore). Rate limits back off (`backoff_s`), then fall through to the next provider (ollama). Tokens come from the CLI's `usage`. Off by default; turn it on per task by putting `"claude_code"` first in that task's `providers` and setting `providers.claude_code.enabled = true`. |

Tests register a `mock` kind with `register_kind("mock", cls)`.

## 2. Inbox: `engine/inbox.py`

```
python -m engine.inbox [--root DIR] [--file PATH] [--json] [--config TOML]
```
```python
from engine.inbox import Inbox
reports = Inbox(cfg).run()          # every audio/video file in the inbox
report  = Inbox(cfg).process(path)  # one file
```

How it works:

- **Phone videos are the main way in** (`.mov/.mp4/.m4v`, H.264 or HEVC, iPhone and
  Android; `engine/media.py`).
  - **Audio only:** ffmpeg extracts a 16 kHz WAV plus a 32 kbps Opus copy. The original
    video is **moved**, never copied, from the inbox to `processed/<date>_<sha8>/`.
  - **ffprobe metadata** goes into `corpus/<sets|memos>/<name>/meta.json` and the `set` node:
    - `creation_time` (UTC);
    - `com.apple.quicktime.creationdate` (local time with the timezone);
    - duration and codecs;
    - GPS from `com.apple.quicktime.location.ISO6709` or Android `location`;
    - the device (`com.apple.quicktime.make/model`, `com.android.manufacturer/model`).
  - **Show date:** the iPhone's local creation date. Otherwise the UTC creation time in
    this PC's timezone, then the filename, then the mtime. `date_source` records which.
  - **Venue:** the sidecar's (`given`). Otherwise the nearest named venue within 200 m in
    earlier set metas (`inferred`, with `venue_basis`). Otherwise `unnamed venue @ lat,lon` (`inferred`).
  - **Shows:** recordings within 3 h and 200 m of each other (both need a time and GPS)
    become segments of one `show:` node. Each clip's set node gets `show` and `segment`,
    and a `part_of` set→show edge is proposed.
- **Set, clip or voice note:** a sidecar `<file>.json` decides it: `{"kind": "set"|"clip"|"voice_note", "date", "venue", "title"}`.
  - Without one, a recording of `set_min_seconds` (240 s) or more is a **set**.
  - Anything shorter goes through the laugh detector: crowd laughter makes it a **clip**
    (handled like a set), none makes it a **voice note**. `kind_basis` records why.
- **Idempotency key** = file sha256 + a hash of the versions: prompt version, whisper,
  laugh detector, embedder, and the provider:model each task resolves to.
  - A file whose key is in `inbox_ledger.jsonl` is not processed again; it is only moved.
  - New versions mean a real re-run. That run replaces only the untouched proposals
    from earlier runs of the same file.
  - A transcript made by the same whisper version is reused.

Stages, each timed with its LLM calls and tokens in `report["stages"]`:

1. **normalize**: ffmpeg to 16 kHz mono PCM (the same args as `tools/ingest`) plus Opus; for a video, audio only.
2. **transcribe**: `tools/ingest/transcribe.py` (faster-whisper large-v3, GPU, the same VAD fallback).
3. **laughs** (sets, and every short recording, to classify it): `tools/laughs/detect_laughs.py` in its venv, via `laughs_runner.py`.
   The runner redirects the tool's folders; the detector itself is unchanged.
4. **split**: sentence lines built from the word timestamps. The LLM returns line ranges
   only, so every unit's text is verbatim transcript by construction. Intro and outro are dropped.
5. **match**: bge-small candidates (the unit text against written units, the reveal against
   reveals, and existing performances), then an LLM judge per unit over the top 5 jokes:
   same joke yes/no, confidence, a one-line reason, the closest version.
   - Neighbouring units matched to the same joke merge into one performance.
   - Sets produce `performed_as` perf→joke edges; voice notes produce `part_of` unit→joke.
   - The edge carries `version` (the closest unit or performance).
6. **propose**: unmatched units that are funny or got a laugh. The LLM decides whether
   it is material and gives a title, summary and the funniest line.
   - The result is a new `joke:` node (`proposed: true`, `origin_span`).
   - Its stage is `testing` for a set and `idea` for a voice note.
7. **label**: devices, structures and topics. The vocabulary is `tools/kb/kb_vocab.py`
   (from `JOKE_SCIENCE.md`) plus the live topic labels and his custom labels in the graph.
   - Out-of-vocabulary ids are dropped.
   - Each label needs a verbatim quote.
   - Labels go on the new joke, or on the performance/unit when the joke already existed.
8. **link**: `segue` edges from set order (no LLM), and `same_premise` / `tag_of` /
   `callback_to` between a new joke and its nearest existing jokes (LLM).
9. **merge**: into `graph.json`, plus a `runs[]` entry. Engine-made nodes and edges carry `origin.sha256`.

Rules enforced in `kbstore.KB`:

- **Decisions are replayed** with the Desk explorer's own `View` (`tools/explorer/kb.py`).
- **Never re-proposed:**
  - a rejected or deleted edge id, or the same (src, kind, dst);
  - a rejected node;
  - a new joke from the same span (`origin_span`).
- **Anything the comic decided on stays exactly as it is** in graph.json, so his decision replays on it.
- **Few-shot examples** are the comic's own decisions (`by == "owner"`, still in that state
  after replay): up to 8 confirmed and 3 ruled out, per kind. They are hashed into `fewshot_sha`.
  - Auto-confirmed items (`status: confirmed_auto`, or decisions not by owner or marked
    `auto`) are never examples.
- **Verbatim evidence:** every evidence quote is checked against its node's text, in the
  stage and again at a final gate before the merge.
  - A quote that changed only typography (curly quotes, dashes, whitespace) is replaced by
    the exact source substring.
  - Anything else is dropped.
  - Match edges whose model quotes all fail keep a verbatim excerpt the engine chose, marked `quote_by: engine`.

Report: returned, and written to `processed/<name>/run.json`. It has `stats`, `stages`
(seconds, llm_calls, tokens in/out per stage), totals, and `versions`.

## 3. Ask: `engine/ask.py`

```
python -m engine.ask "every version of the airport bit by year" [--json] [--root DIR]
```
```python
from engine.ask import ask, Corpus, render
r = ask(question, cfg=None, router=None, embedder=None, kb=None, corpus=None)
# r: question, plan, sources[ids], found, answer, not_found, citations[{source_id, quote, note, kind, date, title, joke}],
#    removed[{source_id, quote, why}], timings, tokens_in, tokens_out, provider
```
The API should build `Corpus(cfg, KB(cfg), Embedder(...))` once and pass it in, rather
than rebuilding it on every request (about 1-3 s warm).

1. **plan** (`ask_plan`): search queries with synonyms, label ids from the graph's labels,
   a year range, performed / never performed, named bits, and by-year ordering.
   If planning fails, retrieval uses the raw question.
2. **retrieve**: SQLite FTS5 bm25 and bge-small cosine, fused by reciprocal rank.
   - It covers written units (vectors reused from `tools/kb/cache`), performances,
     voice-note units, and set-transcript windows not covered by a performance.
   - Graph routes: members of a label via non-rejected `has_label` edges, and every unit
     and performance of a named joke.
   - At most 3 sources per joke (40 for a named bit), 32 sources in total.
3. **answer** (`ask_answer`): only from the numbered sources, citing ids.
4. **verify**:
   - Each citation's quote must be verbatim in the cited source, which must be one that
     was retrieved. Otherwise the quote is removed and flagged in `removed`.
   - "Double-quoted" passages in the prose that are in no source are cut.
   - Ids cited in the prose that were never retrieved are flagged.
   - Joke cards (title + summary) can be cited but not quoted.
5. **not found**: `found=false` plus `not_found`, and the CLI prints "NOT IN YOUR MATERIAL".

## Tests

```
python -m pytest engine/tests -q
```
They cover:

- **providers:** the mock backend, runs.jsonl fields, the invalid-JSON retry, availability
  fall-through, the Anthropic key from env or the secrets file, the Anthropic request
  shape, and that the key is not logged.
- **claude_code:** off by default, offline-only, JSON + tokens, rate-limit back-off then fallback.
- **media:** synthetic HEVC/H.264 videos made by ffmpeg (no real recordings): ISO 6709
  parsing, ffprobe metadata (local date with the timezone, GPS, device), audio-only
  extraction, venue inference from an earlier meta, and two clips grouped into one show.
- **inbox:** end to end with fakes, a short recording with laughs becoming a clip, idempotency (same file → skipped, no model calls,
  graph byte-identical, both audio copies kept), and decision preservation (reject →
  re-run → not re-proposed, confirmed kept exactly, decisions.jsonl byte-identical,
  the comic's examples in prompts, auto-confirmed never).
- **the quote verifier and Ask** drop tampered quotes.

All of them use temp copies and never touch `comedy/`.

## Known limits (2026-10-26)

- **Splitting** is driven by an LLM over sentence lines.
  - On the 2023-05-10 test set (gpt-oss:20b) the last bit, "Rex", stayed inside the airport-bit unit.
  - Unit names can be off by one bit.
- **Matching** uses only the jokes in the graph. A fresh `tools/kb` rebuild of graph.json
  replaces the engine's merged proposals: run the inbox after a rebuild, or teach
  kb_build to keep `origin.engine` items.
- **Android metadata:** an ffmpeg-made MP4 keeps `creation_time` and `location`, but not
  `com.android.model`. Real phones vary, so the device can be missing.
  - With no iPhone `creationdate`, the show date uses this PC's timezone.
- **Show grouping** needs both a time and GPS.
- **Ask with gpt-oss:20b:** it pastes source text into its prose without quote marks,
  which the inline check cannot see. Only quoted passages and citations are verified.
- Ask with `reasoning_effort = low` answers in 10-60 s, but can list few items (the
  flying question found one).
