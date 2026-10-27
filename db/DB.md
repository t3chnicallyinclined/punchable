# Punchable on SurrealDB 3.3.4

Punchable's data now lives in a local **SurrealDB 3.3.4** server (decision: `docs/CONTEXT.md`, "Storage decision, revised 2026-10-26"). The dev API (`api-dev/server.py`) serves the **same endpoints with the same response shapes** from it. On top of that, SurrealDB adds three things:
- **Working** is recomputed by `DEFINE EVENT` + `fn::working` inside the write that changed it.
- **LIVE SELECT** pushes changes to the app (`/api/live`) instead of the app polling every 15 s.
- **Hybrid search**: stemmed BM25 + HNSW vectors, fused by `search::rrf` (`/api/jokes?q=…&mode=hybrid`).

`decisions.jsonl` stays the canonical history. The database can always be rebuilt from the files.

## Where things are

| | |
|---|---|
| Binary | `%USERPROFILE%\.punchable\bin\surreal.exe`, the official 3.3.4 Windows build. `start.ps1` refuses anything older than 3.3.1. |
| Server | `ws://127.0.0.1:8797/rpc` (loopback only, scripting denied) |
| Store | SurrealKV at `~/punchable-data\db\punchable\data`. `comedy\db\.gitignore` ignores everything there (data, logs, backups, caches). |
| Credentials | `%USERPROFILE%\.punchable\secrets.env` (`SURREAL_USER`, `SURREAL_PASS`, `SURREAL_PORT`). Never committed. They are passed by environment, never on a command line. |
| Database | namespace `punchable`, database `main` (STRICT). Tests use namespace `punchable_test` on their own in-memory server. |
| API | `http://127.0.0.1:8796`, SurrealDB backend by default. Logs go to `comedy\db\punchable\logs\api.log`. |
| Graph loaded | recorded in `comedy\db\punchable\graph_choice.json`. The API's files backend reads the same graph. |

## Start, stop

```powershell
cd <repo>
powershell -ExecutionPolicy Bypass -File db\start.ps1     # idempotent; prints pid + URL
powershell -ExecutionPolicy Bypass -File db\stop.ps1
python api-dev\server.py --port 8796                       # API on SurrealDB (default --backend surreal)
python api-dev\server.py --port 8796 --backend files       # API on the files only (SurrealDB not needed)
```

The API refuses to start on SurrealDB if:
- the server is older than 3.3.1;
- the database was never loaded;
- **a change feed is missing** (trap T6). Fix that by running `python db/load.py`.

Any request can take `?backend=files` to read the same graph through the old file replay (the fallback switch).

## Load

```powershell
python db\load.py                 # full rebuild (about 8-17 s); graph "auto"
python db\load.py --graph v0      # or v1, or a path to a graph.json
python db\load.py --sync          # incremental: writes only what changed (what the API's watcher runs)
```

The load order follows KB_SCHEMA's precedence (the comic's decisions > inferred > proposed):
1. **graph.json.**
   - v1 is used if `kb\v1\graph.json` passes `tools\kb\kb_validate.py` **and** every line of `decisions.jsonl` replays over it with no orphans. Otherwise v0 is used.
   - `kb_validate --decisions` itself is stricter than the replay. It flags edits on items that an earlier `add` created, and confirms of AI-inferred edges.
2. **inferred.json**, then every **`corpus\sets\*\analysis.json`**.
   - These are laid over the graph by `api-dev\ai_overlay.py`, the same code the files backend uses.
   - v0 ids are resolved through `kb\v1\id_map.json`. Any that don't resolve are logged in `load_info.id_map_log` and on stderr.
3. **`laughs_auto.json`**, every set. Laughs are measurements, with ids `laugh:[set, start_ms, n]`.
4. **decisions.jsonl.**
   - Each line becomes a `decision` record (`seq` = its index).
   - The `apply_decision` **event** applies it in the same transaction, mirroring `kb.View._apply`: add, delete, confirm, reject and edit, a target brought back from its before-image, and `_edited` and undo.
5. **Unit embeddings** come from `tools\kb\cache\vectors.*` (bge-small). Any missing ones are embedded locally by `db\embed.mjs`, with no downloads.
6. **Working** is backfilled with `fn::recompute_working`, and the change feeds are checked again.

Every write goes through `sdb.Rpc.write`, which raises `EmptyWrite` on an empty result (trap T5). The client speaks CBOR only (trap T1).

## Reload after new pipeline output

It's automatic. The API watches every input once a second: graph.json, inferred.json, analysis.json, laughs_auto.json, meta.json, id_map.json, and decision lines appended by another writer. When one changes, it runs the **incremental sync**.
- The sync replays, in Python, what the database should hold, diffs it against the database, and rewrites only the records that differ. New decision lines go through the event.
- It takes about 1–2.5 s. Requests wait for it rather than see half of it. The app then gets a live push.
- `/api/backend` shows `file_syncs` and `sync_error`.

By hand:
- `python db\load.py --sync` runs the incremental sync.
- `python db\load.py` runs a full rebuild.

After re-running the infer pipeline (`python -m infer.sets --all`), nothing else is needed. If the **graph version changes** (for example, v1 rebuilt into a new folder), run `python db\load.py --graph <v>` and restart the API.

## Backup, restore, compaction

```powershell
powershell -ExecutionPolicy Bypass -File db\backup.ps1                       # -> comedy\db\punchable\backups\punchable-punchable-main-<UTC>.surql.gz (~10 MB)
powershell -ExecutionPolicy Bypass -File db\restore.ps1 -File <backup.gz>    # REMOVES punchable/main, imports the backup
powershell -ExecutionPolicy Bypass -File db\restore.ps1 -File <backup.gz> -Namespace punchable_test -Database restore_check   # restore elsewhere to inspect
```

Restore was verified into a scratch database: the same counts, the change feeds present, KNN and full-text working.
- Stop the API before restoring `main`, and start it afterwards.
- Decision lines that the backup lacks are appended by the watcher on start.
- An export is a snapshot of derived data. The files are the real backup: `python db\load.py` rebuilds everything.

**Hosted.** The hosted database is backed up nightly to R2 by a CronJob (`infra/backup/backup.py`; schedule, retention,
the deploy steps and the restore drill are in `infra/README.md` "Backups"). It exports over HTTP as the database user
`api`, not root: a database user's export is byte-identical to root's. `backup.py export <out.gz>` and
`restore-check <file.gz>` also work against this local server (`SURREAL_URL=http://127.0.0.1:8797`,
`SURREAL_AUTH_LEVEL=root` with the `secrets.env` login). Restore drill, 2026-10-26: the local database and the hosted
one, each exported, restored into a scratch server and compared table by table: every count, table definition and
change feed equal, full-text and KNN answering.

**Compaction.** SurrealKV keeps an append-only log, so the store grows with churn. It reached 1.2 GB after test runs on the real store; tests no longer use it.
- To shrink it: `db\stop.ps1`, delete `comedy\db\punchable\data`, `db\start.ps1`, `python db\load.py`, then restart the API.
- This is safe whenever `/api/health` `decision_lines` equals the line count of `decisions.jsonl`.
- A fresh v1 load is **59 MB**.

## Schema (`db/schema.surql`)

- **Strictness.** The database is STRICT and every table is SCHEMAFULL. An unknown table or field is an error.
- **Tables** are defined `IF NOT EXISTS`, never `OVERWRITE`, because `OVERWRITE` drops a change feed (T6). A table-level change needs a migration.
- **Nodes:** `joke`, `unit`, `set`, `performance`, `label`, `laugh`.
  - Extra tables: `list` (`list:tonight`), `laugh_run` (laughs_auto.json header), `analysis` (the AI index), `meta` (load state and `gen`), `decision`.
- **Edges:** one `TYPE RELATION` table per KB kind (`part_of` … `custom`).
  - Fields: `status`, `confidence`, `reason`, `evidence`, `source`, `run_id` (from `source`), `custom_kind`.
  - `status` must be one of `proposed`, `inferred`, `confirmed`, `ruled_out`, `superseded` **or `rejected`**. `rejected` has to be allowed because it is what kb.py writes when the comic rejects something.
  - `has_label` must point at a label. The other kinds accept any node table, because kb.py lets the comic connect any two nodes.
- **Exactness.** Values keep the files' types, so responses stay byte-for-byte equivalent.
  - Dates stay ISO strings.
  - Fields accept NULL, and NONE and NULL are kept distinct.
  - Private replay markers are stored as `x_*`.
  - `ord` keeps the files backend's insertion order; it is a number, so a sync can slot new records in between.
- **Working.**
  - `fn::working($joke)`: confirmed matches to performances that are not crowd work or banter and that the comic rated killed or worked, in 3 or more distinct sets. This is exactly `AppIndex.joke_rooms`.
  - Events `working_on_match` (on `performed_as`) and `working_on_perf` (a rating or kind change) write `joke.working` and `working_sets` in the same transaction.
  - The API reads `rooms.hit_sets` and `ready_for_working` from these fields.
- **Search indexes.**
  - FULLTEXT `punch_en`: class tokenizer, lowercase, ascii, snowball(english), on `unit.text` and `joke.title`.
  - HNSW 384-d cosine on `unit.embedding`. The KNN `WHERE` (for example `hidden != true`) runs **inside** the index scan (`EXPLAIN`: `KnnScan … predicate`).
- **Change feeds** (`30d INCLUDE ORIGINAL`) on `decision`, `joke`, `performance`, `set`, `label`, `list`, `performed_as`.
  - Those are the tables a phone sync would pull. Bulk tables (units, has_label) are left out to save disk.
  - The loader checks them, and repairs a dropped one loudly. The API refuses to serve without them.

## API additions (existing shapes unchanged)

| | |
|---|---|
| `?backend=files` / `?backend=surreal` | Picks the backend for one request. Reads compare like for like. A POST with `?backend=files` writes the jsonl only, and the watcher then syncs it into SurrealDB. |
| `?include_hidden=1` | Shows `hidden: true` jokes and units. These are kb v1's `not_a_joke` (no text) and `pasted_not_his` units. They are left out of every list, search, count and joke page otherwise, on both backends. |
| `GET /api/jokes?q=…&mode=hybrid` | The same JokeSummary items, ranked by `search::rrf` over BM25 (unit text + joke titles) and HNSW KNN (the query is embedded by `db\embed.mjs`). Without a query vector it fuses full-text only. The default `q` search is unchanged. |
| `GET /api/live` (WebSocket) | Pushes `{type:"change", rev, changes:[{table, action, id, working?, hit_sets?}]}` from LIVE SELECT on `meta`, `joke`, `decision`, coalesced every 150 ms. The app (`app/src/lib/api.ts`) subscribes and re-fetches. It polls `/api/health` only while the socket is down. |
| `GET /api/backend` | Backend status: version, ns/db, gen, graph, change-feed check, live clients, file syncs. |

**Writes.**
- A write goes to SurrealDB first, as a `decision` record. The event applies it, and the read-back happens in the same transaction. Then the same line is appended to `decisions.jsonl` (fsync).
- A write is refused (409) if the database and the jsonl disagree on the line count, and (503) while a reload runs.
- Write latency is 65–120 ms; the files backend takes about 50 ms.

## Tests (`db/tests/`, all on a throwaway in-memory SurrealDB with TEMP copies; never the real decisions)

```
python db\tests\run_all.py           # guards, hidden, mutation + live, baseline diff
python db\tests\run_all.py --e2e     # ... + app/tests/e2e.mjs on the SurrealDB backend (Expo web on :8798)
```

- **`test_baseline.py`.** Every GET endpoint and variant, all 1,432 joke pages, every set's review and analysis, audio with ranges, 404s.
  - First compared SurrealDB against `?backend=files` inside one API.
  - Then every POST (all ops, undo, errors) is sent to a SurrealDB API and a files-only API, and the responses compared.
  - Then every GET again: after the writes, across the two APIs, and after the watcher's incremental sync (new analysis.json plus a files-only writer).
  - **6,056 comparisons, 0 different.**
- **`test_mutation_live.py`.** Three match + rate decisions flip Working false → false → **true** at the third room, read from the database the moment the POST returns.
  - The `/api/live` push arrived in about 106 ms. Undo flips it back and is pushed again.
  - The files backend agrees at every step, and on all 1,428 jokes.
- **`test_guards.py`.**
  - The empty-OK trap is reproduced: for a missing target, and for a permission-**denied** write under a record session. `write()` raises in both cases.
  - The CBOR text round-trip holds.
  - A change feed dropped by `OVERWRITE` is detected, the API refuses to serve, and the loader repairs it.
  - The version is at least 3.3.1.
- **`test_hidden.py`.** Hidden jokes and units are absent from lists, pages and search (word and hybrid) on both backends, and come back with `include_hidden=1`.
- **`run_e2e.py`.** Runs the app suite with every spawned API on its own temp SurrealDB database, then deletes the run's ~150 MB Chrome profile.
  - 104/104 on the v0 test graph in 3 of 4 runs. One run failed a UI wait ("Review: pick a set"), and the next two passed.
  - The suite hard-codes v0 data ("1501" jokes), so with `REAL_KB=…\kb\v1` it stops at check 15 **on both backends alike**.

### Documented differences

These are the only allowed ones:
- `/api/health` `rev`: a DB generation (`db:<gen>`) against a file signature.
- `/api/backend`.
- `?mode=hybrid`, which is SurrealDB only.
- Floats compared to 1e-9.
- Ordering ties. Equal-confidence labels and `top_labels` ties follow Python's per-process string hashing (`kb.View.joke_scope` is a `set`), so they can differ between two processes **on either backend**. The cross-process test pins `PYTHONHASHSEED=0`. Inside one process the backends always agree.

## Hosting additions (2026-10-26)
- `db/schema_extra.surql` (applied by load.py and by the API at start; additive): `creative_event` (the comic's
  storyboard / working-set / venue-pin events and the job queue; exact jsonl lines, unique store+seq, change feed),
  `srcfile` (the text files the hosted API serves from).
- `sdb.Rpc` honours `SURREAL_URL` and a database-level user (`SURREAL_API_USER/PASS`, EDITOR on one ns/db); load.py
  then skips DEFINE NAMESPACE/DATABASE. Verified: a full load as EDITOR on a scratch 3.3.4.
- `db/push_remote.py pack|apply`: the hosted load (infra/README.md "Data load"). Idempotent; refuses when the database
  holds decisions or creative events the bundle lacks.
- `db/tests/test_creative.py`: the creative routes, SurrealDB vs files, writes on both, live push, /api/live auth.

## Traps met here (in addition to the audit's T1–T8)

| Trap | What happens | What we do |
|---|---|---|
| `UPDATE edge SET out = …` | **Silently ignored** (`in`/`out` can't change; status OK) | Changing an edge's ends or kind deletes it and re-creates it with the same id and ord |
| `RETURN` inside `BEGIN … COMMIT` | Ends the transaction block early | Read-backs inside a transaction are bare expressions |
| `USE NS` in one query call | Doesn't carry to the next call | `USE` and the statement go in the same call |
| Field named `info` | Parse error (keyword) | `meta.load_info` |
| `TYPE array` holding objects | Nested fields rejected on a SCHEMAFULL table | `array<object> FLEXIBLE` |
| Test databases in the real store | SurrealKV grew to 1.2 GB (log not compacted after `REMOVE DATABASE`) | Tests run on their own in-memory server; compaction = rebuild |

## Known issues

- **Transcripts and audio** are still read from `corpus\` and `tapes\` files. They are not graph data.
- The **Desk screen** still calls the default word search. Hybrid search is available with `mode=hybrid`, but switching Desk to it is an app change that wasn't made, because the e2e suite asserts word-search results.
- **LIVE is WebSocket-only, and dies when the phone backgrounds the app.** The app retries every 5 s and polls in the meantime. Phone sync should still pull from the change feeds.
- A full reload (`db\load.py`) churns the change feeds; the incremental sync doesn't. After a full reload, a future phone sync should resync from scratch.
- The e2e suite's data assumptions are v0's.
