# Punchable

A personal joke-writing app for stand-up comics. It learns **one comic's voice from that comic's own material
only**: their written jokes, their voice memos and the tapes of their sets. Nothing is trained on anyone else's jokes,
and every line the AI shows you that claims to be yours is checked word for word against your own notes first.

The hosted version runs at [punchable.io](https://punchable.io). This repository is the code, released under the MIT
license. It holds no one's material: the examples and test data are invented.

**Working on it next?** Start at [docs/HANDOVER.md](docs/HANDOVER.md): what is live, what learns, what is open, the traps.

## What it does

- **Write and riff.** Start from a premise or a line; get related bits from your own notebook and riff ideas built on
  them, each marked with what it was built from.
- **Notebook.** Every joke, note and memo, searchable by words and by meaning, with each joke's versions over the
  years and how it did on stage.
- **Joke map.** Jokes linked by topic, persona, device and callback, so you can see what connects to what.
- **Set builder.** Put bits in order with timing, transitions and bridge drafts between them.
- **Your sets.** For each taped set: what's working, tough spots, laughs per minute, the longest laugh and the
  longest quiet, with swap / tag / bridge / move suggestions you can lock or ignore.
- **Merge same-idea notes.** Finds notes that are the same bit written twice and merges them, keeping every original
  word (and undo).
- **Ask.** A chat over your own notebook ("what have I written about airports?"). Answers cite your notes, and any
  quote that is not verbatim in a source is removed before you see it.
- **Google Docs sync (two-way).** Your joke docs become notes; jokes made in the app become docs. Court and legal
  documents are never opened (metadata check before any download, plus a second check on the text).
- **Voice memo import.** Memos saved to a Drive folder are downloaded, transcribed and filed as notes.
- **Local Whisper + laugh detection.** Set audio is transcribed on your own GPU (faster-whisper) and laughs are
  detected with an audio classifier, so laughs per minute and "what's working" come from the room, not a guess.

## How it is built

| Part | What it is |
|---|---|
| `app/` | The app: React Native + Expo (Expo Router, TypeScript). Runs on the web today via react-native-web. |
| `api-dev/` | The Python API (standard library HTTP server). Same code runs locally and hosted (`api/entry.py`, `api/Dockerfile`). |
| `db/` | SurrealDB schema, loader and tools. The database is rebuilt from append-only files (`decisions.jsonl` and friends). |
| `creative/` | Notebook search, related bits, riffs, set builder, working sets, merges and the Ask chat. |
| `engine/` | Model providers (Anthropic API, Claude Code CLI, or a local OpenAI-compatible server such as Ollama), the inbox pipeline and the verbatim quote checker. |
| `worker/` | The "home worker": a process on your own computer that runs AI jobs and audio imports for the hosted API, so the server holds no models. |
| `gsync/` | Google Docs and Drive sync, voice memo import, legal-document exclusion. |
| `infer/`, `tools/audio/`, `tools/laughs/` | Set analysis: VAD, transcription, laugh detection, per-bit laugh metrics and clip ranking. |
| `tools/explorer/` | A local web tool to browse and correct the joke graph. Its `fixtures/` folder is the invented sample knowledge base. |
| `infra/` | Hosting on one MicroK8s server with Keycloak login (Google sign-in) and Cloudflare DNS. See `infra/README.md`. |

Data lives in a folder **outside the repo**: `~/punchable-data` by default (`COMEDY_DIR` / `PUNCH_COMEDY` to move it).
Secrets live in `~/.punchable/` (`secrets.env`, Google OAuth files). Neither is ever committed.

## Run it locally with the sample data

Needs Python 3.13 and Node 22.

```bash
# 1. the API on the invented sample knowledge base (files backend, no database needed)
mkdir -p /tmp/pk/kb /tmp/pk/tapes /tmp/pk/creative
python api-dev/server.py --backend files \
  --graph tools/explorer/fixtures/graph.sample.json --corpus tools/explorer/fixtures/corpus \
  --kb /tmp/pk/kb --tapes /tmp/pk/tapes --creative-dir /tmp/pk/creative --no-fixtures
#    -> http://127.0.0.1:8796/api/health

# 2. the app
cd app && npm ci && npm run web
#    -> http://localhost:8798/
```

To browse the sample graph on its own: `python tools/explorer/server.py --graph tools/explorer/fixtures/graph.sample.json --decisions /tmp/pk/dx.jsonl --corpus tools/explorer/fixtures/corpus`.

For your own material, put it under `~/punchable-data` (the layout is `kb/`, `corpus/`, `tapes/`; see `db/DB.md` and
`api-dev/API.md`), start SurrealDB with `db/start.ps1`, load it with `python db/load.py`, and run the API without the
`--graph/--kb` flags. AI features need a provider configured in `engine/config.toml` (an `ANTHROPIC_API_KEY` in
`~/.punchable/secrets.env`, the Claude Code CLI, or a local Ollama).

Your own private settings go in files the repo ignores:
- `~/.punchable/gsync_docs.json`: your Drive folder and doc ids for the Google Docs sync (`gsync/config.py`).
- `~/.punchable/legal_rules.json`: extra folders, docs and title keywords to never open (`gsync/rules.py`).
- `~/.punchable/infra.env`: your domain, server and namespaces (copy `infra/config.env.example`).
- `~/.punchable/punchable.env`: your data folder and local settings (`tools/local_env.ps1` loads it).

The full list, and the commit guard that keeps your material out of git: [docs/DEVELOPING.md](docs/DEVELOPING.md).

## Tests

```bash
python -m pytest creative engine gsync worker infer/tests tools/audio/tests api-dev/tests -q
python tools/explorer/test_explorer.py
cd app && npm run test:unit && npx tsc --noEmit
```

Tests that need a real knowledge base (most of `api-dev/tests`, the SurrealDB suites in `db/tests`) skip themselves
when there is none. The Google auth tests need `google-auth` and `google-api-python-client` installed.

## License

MIT. Copyright (c) 2026 Tristan Rega. Contact: technicallyinclined@proton.me
