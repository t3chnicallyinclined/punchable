# Developing Punchable

Punchable is built for one comic's own material, and this repository is public. The rule that keeps both true:
**code goes in the repo; your material, your settings and your secrets never do.** Those stay in two folders outside
the clone.

## Where things go

| What | Where | Notes |
|---|---|---|
| Code, tests, invented sample data | this repo | samples live under `tools/explorer/fixtures/` and in the tests |
| Your material (jokes, notes, transcripts, tapes, the joke graph, decisions) | your data folder: `COMEDY_DIR` (default `~/punchable-data`) | layout in `db/DB.md` and `api-dev/API.md`; `PUNCH_COMEDY` is the same setting for `db/` |
| Caches built from your material (vectors, embeddings, audio stages) | `PK_CACHE_DIR` (`<dir>/kb`, `/infer`, `/audio`) | without it they go in gitignored folders inside the clone |
| Judgments on your own sets (`infer`) | `PK_INFER_JUDGE_DIR` (default `<data folder>/kb/infer_judge`) | |
| Secrets | `~/.punchable/` (`$PUNCHABLE_HOME`) | `secrets.env`, `google-oauth.env`, `cloudflare.env`, token files |
| Private settings | `~/.punchable/` | see the next table |

Private settings in `~/.punchable/` (all optional; each one only adds to the public defaults):

| File | Read by | What |
|---|---|---|
| `punchable.env` | `tools/local_env.ps1`, the scheduled-task installers, `tools/guard` | `KEY=VALUE` lines: `COMEDY_DIR`, `PUNCH_COMEDY`, `PK_CACHE_DIR`, `PK_INFER_JUDGE_DIR`, `PK_WHISPER_PY`, `PK_WORKER_PY`, `PK_LAUGHS_PY`, `PK_OWNER_ID`, `PK_COMIC_NAME`, `PK_COMIC_FIRST`. No secrets. |
| `legal_rules.json` | `gsync/rules.py` | your legal folders, documents, title keywords and text markers that gsync must never open |
| `gsync_docs.json` | `gsync/config.py` | your Drive folder and document ids |
| `venues.json` | `tools/ingest/show_dates.py` | your rooms (pattern -> name) and any fields for a new set's `meta.json` |
| `persona_synonyms.json` | `infer/catalog.py` | spellings of your persona labels -> label slug |
| `infra.env` | `infra/ship.sh`, `infra/dns/cf_dns.py` | your domain, server, ssh target and namespaces (copy `infra/config.env.example`) |
| `guard_terms.json` | `tools/guard/check.py` | private words the commit guard must block (names, hosts, other projects) and its allowlists |

`PK_OWNER_ID` is the actor id written on your own edits (`by` in `decisions.jsonl` and the stores). Keep it equal to
the id already in your data, and set the same value in `infra.env` so the hosted API writes it too. `PK_COMIC_NAME`
(and `PK_COMIC_FIRST`) is how the AI prompts refer to you; the default is "the comic".

Load the settings into a PowerShell session before running things by hand:

```powershell
. .\tools\local_env.ps1
python api-dev\server.py --port 8796
```

`gsync\install_task.ps1` and `worker\install_task.ps1` read the same file and put the values on the scheduled task's
command line, so the tasks run with your settings and log under `<data folder>\db\punchable\logs`.

## The commit guard

Install it once per clone:

```powershell
powershell -ExecutionPolicy Bypass -File tools\guard\install.ps1
```

It sets `core.hooksPath` to `tools/guard/hooks`. Before every commit it checks the lines being added, and before every
push it checks each commit being pushed, for:

- secrets (keys, tokens, JWTs, long hex, public IP addresses);
- the private words in `~/.punchable/guard_terms.json`;
- your own material: titles, ids and every 7-word run of words from your data folder, built at runtime and cached in
  `~/.punchable/guard_cache` (rebuilt when the data changes);
- file names that are never committed (`*.env`, token files, the private settings above).

Any hit blocks the commit and names `file:line`. If the guard settings exist but the data folder cannot be found, it
refuses rather than skipping the material check. `python tools/guard/check.py --files <paths>` checks files by hand;
`--range A..B` checks commits.

Do not use `git add -A` or `git add .` in this repo: stage the paths you mean to commit.

## Tests

```bash
python -m pytest creative engine gsync worker infer/tests tools/audio/tests api-dev/tests -q
python tools/explorer/test_explorer.py
cd app && npm run test:unit && npx tsc --noEmit
```

The root `conftest.py` points `PUNCHABLE_HOME` at an empty temporary folder and clears the identity settings, so the
tests never read your private settings and always see the public defaults. Tests that need a real knowledge base skip
themselves without one. The gsync auth tests need `google-auth` and `google-api-python-client` (use the gsync venv).
