# Punchable dev API (v0)

`api-dev/server.py` is a Python stdlib server bound to `127.0.0.1`, default port **8796**. It is the stand-in for the Rust API that will run on the server, so these shapes are the contract: keep them when porting.

- **Reads:** `comedy\kb\v0\graph.json` (read-only, always), with `decisions.jsonl` replayed on top by `tools/explorer/kb.py`, which is reused unchanged. It also reads `corpus\sets\<YYYYMMDD_ytid>\{transcript,laughs_auto}.json` and `tapes\raw\<dir>\audio.opus|audio16k.wav`.
- **Writes:** only two things.
  - Appends to `decisions.jsonl`, through `kb.py`'s append (append mode, fsync, torn-line safe).
  - Saves uploaded files into `tapes\inbox\`.
- **CORS:** any `http://127.0.0.1:*` or `http://localhost:*` origin (the Expo web dev server). Nothing else.
- **Errors:** `{ "error": "message" }` with 400 / 404 / 409 / 500.

```
python api-dev/server.py [--port 8796] [--backend surreal|files] [--kb DIR] [--graph FILE] [--id-map FILE] [--corpus DIR] [--tapes DIR] [--inbox DIR] [--fixtures DIR | --no-fixtures] [--ns NS --db DB]
```

## Backends (2026-10-26, `db/DB.md`)
- **Default: SurrealDB 3.3.4** (`--backend surreal`). It serves the same endpoints and identical shapes; `db/tests/test_baseline.py` proves it endpoint by endpoint.
  - Writes create a `decision` record (the DB event applies it), then append the same line to `decisions.jsonl`, which stays canonical.
  - Working (`rooms.hit_sets`, `ready_for_working`) comes from the DB's `fn::working` events.
  - New AI or pipeline files are picked up by a file watcher (an incremental sync, about 1–2 s).
- **`?backend=files`** on any request (or `--backend files`) uses the file replay described below. It is the fallback switch.
- **Graph:** without `--kb`, the graph `db/load.py` loaded (`comedy/db/punchable/graph_choice.json`, now kb **v1**), with v0 ids in the infer files resolved through `kb/v1/id_map.json`. `kb/v0/` still holds decisions.jsonl, inferred.json and KB_REPORT.md.
- **Hidden material:** jokes and units with `hidden: true` (v1's not_a_joke / pasted_not_his) are left out of every list, search, count and page. `?include_hidden=1` shows them.
- **New:**
  - `GET /api/jokes?q=…&mode=hybrid`: fused BM25 + vector search, same items.
  - `GET /api/live`: a WebSocket fed by LIVE SELECT; the app subscribes to it instead of polling.
  - `GET /api/backend`: status.

## The AI layer (AI-first display, docs/KB_SCHEMA.md)
What the app shows is **graph.json → inferred.json → each set's analysis.json → decisions.jsonl replayed on top**, so the precedence is **the comic's decisions > inferred > proposed**. `ai_overlay.py` does the overlay and writes nothing.
- **Files:** `kb\v0\inferred.json` and `corpus\sets\<set>\analysis.json`. They are re-read whenever their mtime or size changes, so new or rewritten files show up on the next request (no restart, no code change). `GET /api/health` returns `rev`, which changes with any input; the app polls it every 15 s on Today and the Set screen.
- **Fixtures:** until the real files exist, `api-dev/fixtures/inferred.json` and `api-dev/fixtures/sets/<set>/analysis.json` stand in. They are built from the real kb by `fixtures/make_fixture.py`. A real file always wins over the fixture for the same set, and `--no-fixtures` turns them off. `health.ai` says which source each set uses.
- **inferred.json** (read tolerantly):
  - `jokes[]` (`id` or `joke_id`): `stage`, `stage_confidence` or `confidence`, `stage_reason`, and `performances[]` (`perf`, `ai_rating`, `rating_basis`). The stage is set on the joke as AI state.
  - `nodes[]` adds new nodes. An inferred performance that overlaps an existing graph performance in the same set (start within 3 s, or 50 % overlap) is treated as that performance, and its fields are laid over it.
  - `edges[]` overlay an existing edge by `id` (`status`, `confidence`, `reason`, `evidence`), or are added if `src`, `dst` and `kind` exist. Status defaults to `inferred`. An inferred `performed_as` retires the other proposed matches for that performance (`superseded`).
- **analysis.json rows:**
  - Each row maps to the performance it overlaps. If none overlaps, a `perf:<set>@<start_ms>` node is created.
  - The row's `ai_rating` and `rating_basis` go on that node, and `status` crowd_work or banter goes on as its `kind`.
  - A `known` row's match becomes an `inferred` `performed_as` edge. A `new` row retires the proposed match.
- **AI state in responses:**
  - EdgeView gains `state`: `ai` (inferred and untouched, shown as fact), `suggested` (proposed), or one of the comic's own: `confirmed`, `edited`, `yours`, `rejected`. `dismissed` means the AI retired it. EdgeView also gains `ai: bool`.
  - Jokes gain `stage_ai` and `stage_confidence`, plus `ai_hit_sets`: distinct sets where the comic's rating, or else the AI's, is killed or worked.
  - Performances keep the comic's `rating` separate from `ai_rating`.
  - `who` is `ai`, `confirmed` or `edited`, from the comic's live decision lines. Agreeing with the AI's rating counts as a confirm.

## Reading

| Endpoint | Returns |
|---|---|
| `GET /api/health` | `{ok, graph, graph_state, decisions, decision_lines, bad_lines, inbox, corpus, tapes}` |
| `GET /api/meta?labels=12` | `{stages, stage_counts, ai_stage_counts, who_counts, ratings, top_labels:[{id,name,kind,jokes}], counts:{jokes,labels,edges,sets,performances,decisions}}`. `top_labels` are ranked by how many jokes carry a non-rejected label; they are the Desk filter chips |
| `GET /api/today` | `{latest_set: SetCard + {moments, review_minutes}, latest_analysis, glance, unreviewed_sets, tonight: Tonight, pipeline:{stage:n}, next_show:null, inbox:n}`.<br>`latest_analysis` = `{id, title, date, source, jokes_counted, known_jokes, new_material, crowd_work, laughs_per_min, best, weakest}` for the newest analysed set.<br>`glance` = `{stages, ai_working, set, new_material:[{set, idx, perf, start_ms, end_ms, title, punch, rating, saved_as}], hints:[3 × {kind: similar or label, idx, joke, target, why}]}` |
| `GET /api/jokes?q=&stage=&label=&who=&offset=0&limit=40` | `{items:[JokeSummary], total, offset, limit}`. `q` searches titles, summaries, unit text and label names; every word must match. Sorted by stage (working, testing, to_try, idea, retired), then sets performed, then last seen |
| `GET /api/jokes/<joke id>` | `JokeDetail` (below) |
| `GET /api/pipeline?limit=60` | `{stages:{<stage>:{total, items:[{id,title,stage,sets,hit_sets,versions}]}}}` |
| `GET /api/sets` | `{sets:[SetCard]}`, newest first |
| `GET /api/sets/<set id>/review` | `{set: SetCard, steps:[Step], audio:{opus,wav} or null}` (the optional step-through) |
| `GET /api/sets/<set id>/analysis` | `SetAnalysis` (below): the Set screen. `{source:null, rows:[]}` if the AI hasn't read the set |
| `GET /api/labels` | `{labels:[{id,name,kind,definition}]}` (merged labels are left out) |
| `GET /api/inbox` / `GET /api/notes` | `{items:[{file,bytes,kind:set/note/upload,title,received_at,duration_s,marks_ms,status:"queued"}]}`. `/api/notes` returns only kind `note` |
| `GET /api/decisions?limit=30` | the latest decision lines, newest first, with `index` and `undone` |
| `GET /api/audio/<set id>[?fmt=wav]` | the set's audio: `audio/ogg` (Opus) by default, or `audio/wav`. Supports `Range: bytes=a-b` (206 with `Content-Range`), so the player can seek |

### Shapes
- **JokeSummary:** `{id, title, stage, summary, first_seen, last_seen, versions, sets, hit_sets, to_review, labels:[{id,name,bucket}], yours}`
  - `versions` is the number of units filed under the joke.
  - `sets` is the number of distinct sets with a non-rejected `performed_as` edge.
  - `hit_sets` is the number of distinct sets where the comic **confirmed** the match **and** rated the performance `killed` or `worked`. Working needs `hit_sets >= 3` (`ready_for_working`).
- **JokeDetail:** `{joke, yours, who, ai:{stage_ai, stage_confidence, stage_reason_ai, from_stage, stage_confirmed}, ai_ratings:[{set, date, title, perf, rating, rating_by, ai_rating, max_bars}], text:{setup, reveal, from:{unit,date,doc_title}}, rooms:{sets,hit_sets,ai_hit_sets,ready_for_working}, versions:[{date, units:[unit + edge]}], performances:[Perf], labels:[EdgeView], links:[EdgeView], history}`. AI-state edges sort first.
  - `text` is the newest unit, with its last sentence as the reveal. This is a display heuristic.
  - **EdgeView** is the explorer's edge view: the edge fields plus `bucket` (proposed/confirmed/rejected), `display_kind`, `other`, `direction`, `via_unit`, `yours` and `edited`. Every edge carries `reason`, `confidence` and verbatim `evidence[]`.
- **Perf:** `{perf, set, edge, laughs:[Laugh], max_bars, hits, rating, ai_rating, rating_basis, effective_rating, rating_by:"you"|"ai", kind, suggested, punch:{text,start_ms,end_ms}, play:{start_ms,end_ms}, audio}`
  - `punch` is the transcript segment(s) just before the strongest laugh in the span.
  - `play` runs from 4 s before the punch to 1.2 s after that laugh ends.
- **SetCard:** `{id, title, date, venue, duration_s, performances, rateable, rated, reviewed, laughs, has_audio, analysis:{source, jokes_counted, new_material, crowd_work, best, laughs_per_min} or null, jokes:[{perf,title,joke,kind,rating,rating_by,start_ms}]}`. `rating` is the comic's rating, else the AI's.
- **SetAnalysis:** `{set, source:"real"|"fixture", run_id, generated_at, summary, rows:[Row], laughs:[{start_ms,end_ms,bars}], audio}`.
  - `summary` = `{duration_s, jokes_counted, known_jokes, new_material, crowd_work, banter, laughs, laughs_per_min, best:{id,title,idx,rating}, weakest}`. It is recomputed from the rows' current state, so the comic's edits move the counts and best/weakest.
  - **Row** = `{idx, perf, start_ms, end_ms, status, ai_status, kind_by, title, suggested_title, joke, match:{edge,state,confidence,reason}, match_reason, match_confidence, version:{id,date,doc_title,text}, rating, rating_by, ai_rating, rating_basis, laughs, ticks, punchlines, play, similar:[{joke_id,title,stage,why,confidence}], hints:[{label_id,name,kind,definition,why,edge:{id,state}}], script_diff, transcript, saved_as}`.
  - `status` is the current state: `known`, `new`, `crowd_work` or `banter`. `ai_status` is what the analysis said.
- **Step:** `{key, kind, perf, name, joke, match, text, punch:{text,start_ms,end_ms,laugh,segments[]}, laughs, max_bars, hits, suggested, rating, play}`
  - `kind` is `joke`, `new_line` or `crowd_work`.
  - Steps are ordered by time.
  - A performance whose match was rejected becomes a `new_line`. So does a laugh of 2 bars or more inside the set that no performance covers. In the current data every such laugh is covered, so `new_line` comes from rejected matches.
  - `suggested` comes from the laugh meter: 3–4 bars → killed, 2 → worked, 1 → meh, 0 → died.
- **Tonight:** `{items:[{id,title,stage,summary,sets,hit_sets,done}], source:"default"|"saved", candidates:[{id,title,stage}]}`
  - With no saved list, the default is the 4 most recently seen Testing jokes plus 4 To Try jokes.

## Writing
Every write returns `{ok, indexes:[line numbers], decisions:[lines]}`. Pass the `indexes` to `/api/undo` to undo exactly that action.

| Endpoint | Body | Lines appended |
|---|---|---|
| `POST /api/decision` | the explorer's request: `{target:"edge"/"node", action:"confirm"/"reject"/"edit"/"add"/"delete", id?, after?, reason?}` | 1, validated by `kb.py` (`add` on an edge needs a `reason`) |
| `POST /api/review/rate` | `{perf, rating:"killed"/"worked"/"meh"/"died", reason?, confirm_match?:true}` | 1 or 2: a `confirm` of the `performed_as` edge if it is still proposed or inferred (unless `confirm_match:false`), then a node `edit` on the performance with `after:{rating, rating_auto, max_bars}`. Returns `joke:{…, stage_ai, hit_sets, ready_for_working}` |
| `POST /api/set-row` | `{perf, op, rating?, joke?, reason?}`: in-place edits on the Set screen | One line per edit:<br>`rate`: node edit of the rating; the match is left alone.<br>`match`: the `performed_as` edge is confirmed if it already points at that joke, edited to point at it, or added (reason defaults to "matched on the set page").<br>`confirm`: confirms the match.<br>`crowd` / `bit`: node edit `{kind}`.<br>`new`: rejects the match. On crowd work it also appends a node edit `{kind:"bit"}`, so 2 lines. |
| `POST /api/review/reject-match` | `{perf, reason?}` | 1: `reject` of the performance's `performed_as` edge |
| `POST /api/ideas` | `{text, title?, from?:{set,perf,start_ms,end_ms,note}}` | 1: node `add` of a `joke:<slug>` at stage `idea`, with `summary` = text and `from` = source |
| `POST /api/tonight` | `{items:[{joke, done}]}` | 1: node `add` of `list:tonight` the first time, then node `edit` `{items}` |
| `POST /api/undo` | `{indexes:[…]}` or `{index}` or `{}` (the latest) | one inverse line per index, newest first, each with `undo_of` (the explorer's undo) |
| `POST /api/upload` | raw file body. Headers: `X-Kind: set/note/upload`, `X-Filename`, `Content-Type`, optional `X-Title`, `X-Duration-S` and `X-Marks-Ms` (JSON array) | none. Saves `tapes\inbox\YYYY-MM-DD_punchable-<kind>_HHMMSS_<name>.<ext>` plus `<file>.meta.json`, and returns `{status:"queued", file, kind, bytes}`. No pipeline runs; processing is wired later. Limit 1 GB; audio and video types only |

### Decision lines the app adds (all within KB_SCHEMA; no new action types)
- **Rating:** `{"target":"node","id":"perf:…","action":"edit","before":{…perf},"after":{"rating":"killed","rating_auto":"killed","max_bars":3}}`. This is a measurement *judgement* by the comic; the laughs themselves stay measurements.
- **Tonight's list:** a `list` node, `list:tonight`, with `items:[{joke,done}]`, created by `add` and changed by `edit`.
- **Idea from review / "Save as joke" on the Set screen:** a joke node `add` with an extra `from:{set,perf,start_ms,end_ms,note}`. The Set row then shows "Saved as …".
- **Crowd work / joke (Set screen):** `{"target":"node","id":"perf:…","action":"edit","after":{"kind":"crowd_work"}}` (or `"bit"`).
- **Confirming the AI's stage:** a node `confirm` on the joke (`after:{status:"confirmed"}`). The stage stays and is no longer marked AI.

`kb.py` replays all of these unchanged (node edits copy `after` fields onto the node), and its undo inverts them. The explorer's "Learned" page will list `list:tonight` among added jokes and ratings among node edits. That is cosmetic.

## Creative routes (docs/API_CONTRACT.md, `creative_api.py`)
Every route in the contract, serving creative/'s JSON as-is. Additions the contract implies:
- `GET /api/today` = the existing Today payload **plus** `today.build`'s keys (`date`, `working_set`, `try_tonight`,
  `serendipity`, `new_material`, `minutes`, `his_tonight_list`). `latest_set` is the SetCard plus today.build's set
  fields (SetCard keys win: title, date, venue). The working set is the most recently changed one.
- Lists: `GET /api/boards` -> `{boards:[{id,title,n,runtime_s,runtime,target_s,updated,source}]}`,
  `GET /api/workingsets` -> `{workingsets:[...]}`.
- `PATCH /api/boards/{id}` takes any of `title, notes, target_s, joke_ids (reorder/replace), add:{joke_id,at?},
  remove, move:{from,to}, delete: true` (the inverse of creating a list; answers `{ok, board: {id, title, deleted}}`);
  all are checked before any is written.
- `GET /api/map?cap=400` (1-2000): every bit as a node (the top `cap` by nights, laughs, recency; names only) and the
  links between them grouped like the per-joke map (`next, tags, same_world, callbacks, versions`; at most 6 per bit,
  4×nodes overall): `{rev, cap, nodes:[{id, short_name, title, stage, nights, heat, last_seen, degree}], edges:[{a, b,
  kind, w, why}], counts, groups}`. Cached by `rev` + the merge store (`creative/notebook.all_map`).
- `POST /api/say` also takes `kind: "command"` and `choice` (docs/UX_2026-10.md "Commands"). `POST .../items {joke_id, after?}`: `after` is a
  joke id on the board (or an index).
- `POST /api/workingsets/{id}/lock {joke_id, line?, node?, unlock?}`: a line must be verbatim in a taped night or
  written version (the node is found when not given).
- `PATCH /api/venues/{id} {lat, lon} | {name} | {reset: true}`: stored as an event in `venues.jsonl` and laid over the
  geocoder's pins (`status: "his"`).
- `GET /api/sets/{id}/clips`: the set's `clips.json` (infer/clips.py), or `{set, clips: [], source: null}`.
- **Drafts never block.** `POST /api/boards/{id}/bridge`, `POST /api/workingsets/{id}/bits/{joke_id}/punchup` and
  `POST /api/ask {question}` return the cached draft (200, `cached: true`) or `202 {status:"queued", job, poll}`;
  `GET /api/jobs/{id}` polls. `{fresh: true}` asks again. A locked bit answers `status: "locked"` with no job.
  Jobs run on the hosted worker (the server, the Anthropic API; worker/README.md); `--jobs inproc` runs them in this
  process (local dev).
- **Nothing transcribes on the server (2026-10-27).** The PC worker is retired; whisper, the laugh detector and yt-dlp
  have no worker, and transcription happens on the phone (a separate build). A job of a transcriber kind
  (`process_set`, `import_youtube`) is still queued, never dropped, and says so: the job carries `waiting_for:
  "transcriber"` (persisted: derived from the kind on every replay, cleared by a claim) and the 202 body / the job
  brief add `waiting_for` and `note` ("Transcription happens on your phone for now; this file is waiting."). The
  app shows it under the worker line. `GET /api/inbox` rows carry the same two fields hosted.
- **`POST /api/sets/transcript`** `{segments: [{text, start?, end?}], title?, recorded_at?, venue?, duration_s?,
  engine?}`: a set transcribed on the phone. 202 `{status:"queued", job, poll}` with a `set_text` job for the hosted
  worker: the text-only analysis (the split step; `corpus/sets/<date>_ph<sha9>/{transcript,meta,analysis}.json` with
  every audio-derived field null and `audio.status: "unavailable"`, timestamps `device` or `synthetic`), and every
  funny unit posted as an idea (actor `ai:set_text`). 400 for a bad body, 413 over 400,000 characters. Same text =
  same job key (no duplicates).
- Writes: append-only `<store>.jsonl` + snapshot under `--creative-dir` (default comedy/kb/creative); on SurrealDB
  also a `creative_event` row (written first), pushed on `/api/live` as `{table:"creative_event", store, op, item}`.
  `?backend=files` reads the jsonl; db/tests/test_creative.py diffs the two.
- Worker only (`worker` role when auth is on): `POST /api/worker/claim`, `POST /api/worker/jobs/{id}/result`,
  `GET /api/worker/jobs/{id}/file`, `GET /api/worker/decisions?after=N`, `GET /api/worker/creative?store=`,
  `GET|PUT /api/worker/profile/voice` ("Voice profile" below).
  The claim body is `{kinds, worker, hosted, model?, paused?}`: a hosted worker must list its kinds and never a
  transcriber kind (400 otherwise); `paused` (the day's AI budget is spent, with the reason) is recorded as the
  worker's status and answered 204 so the jobs stay queued with the reason visible.
- Uploads also keep `X-Recorded-At`, `X-Venue`, `X-Lat`/`X-Lng`, `X-Show-Group`, `X-Align-Working-Set` in the meta;
  hosted, an upload queues a `process_set` job.

## Requested by app (2026-10-26, docs/API_CONTRACT.md)
Every row of the contract's "Requested by app" table. Both backends serve them; `db/tests/test_baseline.py` and
`db/tests/test_creative.py` diff them, `api-dev/tests/test_app_requests.py` checks each on temp copies, and
`api-dev/tests/test_auth.py` checks that hosted, every one needs a bearer token (a worker token gets 403).
- **Set analysis summary**: `summary` also passes through analysis.json's `material_s, laugh_s_total, laugh_s_per_min,
  pct_laughing, laughs_merged, laughs_per_min_merged, applause_s, longest_laugh_s, longest_laugh_joke, dead_air_max_s,
  dead_air_max_at_ms, room_heat, room_heat_basis` as they are (null when the file has none). Each row adds
  `laugh_s_per_min, pct_laughing, vs_room, time_to_first_laugh_s, dead_air_max_s`.
- **SetCard** adds `show_date, show_date_source, description, laugh_s_per_min, room_heat, audio_unreliable, venue_id`.
  - `show_date`: his edit (`show_date_source: "yours"`), else `meta.json` `show_date` (tools/ingest/show_dates.py: from
    the title or the video description; the source text says which), else null.
  - `description`: the YouTube description from `tapes/raw/<set>/info.json` (null when the tape is not here).
  - `laugh_s_per_min`, `room_heat`: the analysis summary's.
  - `audio_unreliable`: **his mark only** (true/false), else null. The app applies its own "room too quiet" rule on null.
  - `venue_id`: the room id `/api/venues` uses (creative/venues.py), from the room before any rename of his.
- **Set and performance edits** (`POST /api/decision`, one line each, undoable): kb.py `NODE_EDIT_FIELDS["set"] =
  {show_date (YYYY-MM-DD or null), venue (1-120 chars), audio_unreliable (bool)}` and `["performance"] = {kind:
  bit|crowd_work|banter}`. `POST /api/set-row` also takes `op: "banter"`. SurrealDB: `set.show_date` and
  `set.audio_unreliable` are in db/schema_extra.surql (applied at API start).
- **JokeSummary** adds `short_title` (the title cut at its first parenthesis, at most 28 characters), `punch` (the last
  sentence of the newest written version, or of his own idea's text; the last 14 words of a long one; always an exact
  substring of that text, else null), `nights` (= `sets`), `avg_laugh_s_per_min` and `avg_vs_room` (mean over his
  analysed nights; two segments of one set are one night, laugh_s and window_s summed; vs_room clipped at 3.0; crowd
  work, banter and sets he marked `audio_unreliable` left out), `last_performed` (latest show date on tape).
- **`GET /api/jokes`**: `sort=performed|strongest|written|untouched` (performed: `last_performed` then `last_seen`,
  newest first; strongest: `avg_vs_room` high to low, then nights; written: `last_seen` newest first; untouched:
  `last_seen` oldest first; missing values last; ties by title then id). `label=` may repeat: every label must be on
  the joke (AND). `limit` up to 5000. Without `sort` the order is unchanged.
- **`POST /api/jokes/{id}/punchup`** `{fresh?}`: brainstorm drafts for any bit. Never blocks: `200` cached (or answered
  here without a model: `status: "locked"` when the bit is locked in any working set, `"no_tape"` when it was never
  taped), else `202 {status:"queued", job, poll}`. Line locks from every working set are honoured.
- **Try notes**: `POST /api/workingsets/{id}/try {joke_id|null, text}` -> `201 {working_set, note}`, note =
  `{id:"try:<8 hex>", joke_id, text, done, created_at, done_at, by, title, source:"his"}`.
  `PATCH /api/workingsets/{id}/try/{note_id} {done}` ticks his note or one of the AI's (id `"<joke_id>|<kind>"`, the
  key the app already uses) -> `{working_set, changed, note}`. Stored as `try_add` / `try_done` events in
  `workingsets.jsonl`. `GET /api/workingsets/{id}` `try_tonight` now lists his notes first, then the AI's, each with
  `id` and `done`.
- **Uploads** keep `X-Recorded-At` (`recorded_at`), `X-Venue` (`venue`), `X-Lat`/`X-Lng` (`gps:{lat,lon}`),
  `X-Show-Group` (`show_group`), `X-Align-Working-Set` (`align_working_set`) in the inbox `meta.json`
  (URL-decoded; malformed values are dropped).
- **`POST /api/import/youtube {url}`**: youtube.com/watch, youtu.be, /shorts, /live, /embed links. `202 {status:"queued",
  job, poll}` (a job `import_youtube` for the home worker), `200 {status:"exists", set}` when that video is already a
  set, `400` for anything else. The API never downloads: not even `--jobs inproc` runs this kind (`WORKER_ONLY`).
  The worker (worker/execute.py) runs `tools/ingest/fetch_youtube.py` (yt-dlp into `tapes/raw/<date>_<id>/`),
  `tools/ingest/show_dates.py` (the set's meta.json) and `tools/audio/run_all.py --sets <set>` in the laughs venv
  (transcribe, laughs, then infer.sets + infer.clips). The API's file watcher picks the new set up.
- **`GET /api/settings`** -> `{provider:"hosted"|"chatgpt"|"key"|"custom", training, upload_video, cellular,
  updated_at, source:"default"|"his"}` (defaults hosted/false/false/false). **`PATCH /api/settings`** takes any of
  the four (unknown keys and bad values are 400) and answers the same shape. Stored as `settings` events in the
  creative `account` store (`account.jsonl` + snapshot; SurrealDB `creative_event` store `account`).
- **`GET /api/export`**: one JSON download (`Content-Disposition: attachment`): `{format:"punchable-export/1",
  exported_at, decisions (every line), sets (SetCards), ideas (his), inbox, creative:{storyboards, workingsets, venues,
  account: {items, events}}}`. No audio.
- **`GET /api/learned`**: `kb/v0/learning.json` as it is (404 until it exists). Hosted: packed by db/push_remote.py.
- **`/api/workingsets`** answers `{workingsets: [...], working_sets: [...]}`: `workingsets` is the key, `working_sets`
  an alias of the same list.
- **`DELETE /api/me {"confirm": "delete my material"}`**: anything else is 400. It **never deletes anything**.
  - Local dev: `200 {status:"dry_run", deleted:false, would_delete:[{what, where, count}]}`; nothing on disk changes.
  - Hosted (PK_AUTH=1): appends a `request_deletion` event (his token's `sub`, the list) to the `account` store and
    answers `202 {status:"requested", deleted:false, would_delete}`.
  - **Real deletion is a manual step the owner runs**, never the API: (1) `infra/ship.sh backup` or a fresh export,
    kept offline as long as he wants it; (2) on the server, scale the API to 0 and `REMOVE DATABASE main` in namespace
    `punchable` (SurrealDB root, `infra/db-init.sh` credentials); (3) delete his Keycloak user in realm `punchable`;
    (4) on the PC, if he asks for that too: `comedy/kb/v0/decisions.jsonl`, `comedy/kb/creative/`, `comedy/corpus/`,
    `comedy/tapes/`, `comedy/db/punchable/`; (5) record the date in infra/DEPLOY_LOG.md and tell him.

## Settings > Connections (2026-10-26)
- **`GET /api/connections`** (read-only, auth required hosted, both backends): one plain status per outside source.
  `{google_docs: {status, last_sync, last_ok, docs_linked, joke_docs, boards, conflicts, errors_last_run, last_mode,
  where: "your PC"}, youtube: {sets, import: "link"}, ai: {provider, worker: {online, last_seen, name, where,
  hosted, model, paused, online_window_s}}, transcriber: {where: "your phone", waiting, note}, audio_storage:
  {status: "not_set_up", provider: "Cloudflare R2"}}`.
  - `google_docs` is read from gsync's own files (`PK_GSYNC_DIR`, default `<creative-dir>/../gsync` =
    `comedy/kb/gsync`): `map.json` (docs not hidden; joke docs = origin gdocs/app/mixed), the tail of `runs.jsonl`
    (dry runs ignored), `conflicts.jsonl` (count). **Counts only**: no titles, text, doc IDs or legal exclusions.
    `status`: `not_set_up` (no files; `not_reported` when hosted), `not_synced_yet`, `connected`, `stale` (no run in
    24 h), `error` (the last run had errors), `api_down` (gsync couldn't reach the local API).
  - `ai.worker`: the last `POST /api/worker/claim` (in memory); `online` = within 120 s. Hosted that is the server's
    worker: `where: "the server"`, `name` (the pod), `model` (the provider and models it writes with), `paused` (the
    budget reason while it is online and paused, else null). The same object is the `worker` on the ask/say
    responses (`ask_api.worker_status`).
  - `transcriber`: how many queued jobs wait for a transcript from the phone (`waiting`), and the note shown.
  - Hosted, gsync runs on his PC, so the pod has no gsync files until the worker reports them (not built).

## Voice profile (2026-10-28, `profile_api.py`)
His voice profile is **data in the database**, the single source of truth for every writing prompt (riff, punch-up,
bridge, Ask): never a file in the repo or in an image. Store `profile` (`comedy/kb/creative/profile.jsonl` +
snapshot; on SurrealDB also `creative_event` rows, reconciled like every creative store), one document `profile:voice`,
one op `put {kind, text}`; the document keeps its `previous` version.
- **`GET /api/profile/voice`** (app token): `{kind: "voice", text, updated_at, by, chars, previous: {text, updated_at, by} | null}`
  (`text: null` until it is written).
- **`PUT /api/profile/voice {text}`** (app token; `by` = the owner): at most 60,000 characters; **legal-checked like
  every ingest** (`gsync/rules.py content_is_legal`): a hit is `422 That reads like court material; ...` and nothing
  is kept. Undo in the app = PUT the `previous.text` again (his own write; the document moves back, with the undone
  version as its `previous`).
- **`GET /api/worker/profile/voice`** (worker token): `{kind, text, updated_at, by}` (no history). The hosted worker
  fetches it per job and caches it by `updated_at` (`creative/voice.py`); the text is never logged (length, sha,
  `updated_at` only).
- **`PUT /api/worker/profile/voice {text, actor}`** (worker token; the one-off loader `tools/profile/put_voice.py`):
  `actor` must be `import:<what>` (`import:profile`); the same legal check and 422.
- Local dev with `--jobs inproc`: the API loads its own copy into `creative.voice` before running the job.
- **The learned section** (2026-10-28, `creative/learned.py`): a second op `learn {kept[], dropped[]}` on the same
  document, written by the API itself, never by a PUT (`put` keeps `learned` as it was: the loader and Settings
  cannot clobber it). It fires on writes he already makes: `POST /api/ideas` with `draft: true` ("Use as mine" on
  Write, edited or not) or whose text is a finished riff job's draft word for word, and the `POST /api/decision` that
  adds a `unit` with `source: "draft"` (Brainstorm's "Use as mine"). The kept line goes to `learned.kept` (cap 25,
  newest first, `{text, bit, date, edited?}`: `edited` when the flag was set but no draft matched, his edit of a
  draft); the same job's other drafts, the ones he neither kept nor dismissed, go to `learned.dropped` (cap 25).
- **`POST /api/profile/voice/learn {dropped: [text | {text, bit?}], bit?}`** (app token only; 1-10 lines, each up to
  2,000 characters): he dismissed a draft ("Dismiss" on Write or Brainstorm; the app fires it in the background and
  never blocks on it). One `learn {dropped}` event by him; the same legal rule (422). `{ok, learned: {kept, dropped,
  updated_at}}` (counts).
- Both GETs return `learned`; the worker's GET also returns `laugh_lines` (cap 15, computed from the corpus: the
  punchlines with the longest detector laughs; `[]` without a laugh map). The worker sends it as the SECOND system
  block of a writing prompt, behind the profile (never for Ask). The app shows it read-only twice: Settings › Voice
  profile (under the editor) and "What I've learned" (`(tonight)/learned.tsx`, the same rows).

## Legal rules on the request path (2026-10-28)
`creative_api.refuse_legal`: the same `content_is_legal` runs on the texts a request would send to a model, **before
anything is queued**: `POST /api/riff` (idea + lines), `POST /api/facts` (text), the two punch-up routes (the bit's
latest-night lines and its written setup, `creative.workingset.punch_text`), and an Ask question (`ask_api.ask`,
so `POST /api/say` too). A hit is `422 That reads like court material; Punchable won't send it to the AI.`, the same
shape as an ingest refusal; no job, no event.

## Merge (2026-10-26, docs/API_CONTRACT.md "Merge", `merge_api.py`)
`GET /api/merge/candidates[?joke=]` (`?jokes=a,b,…`, up to 50: `{items: {joke_id: {group, others (the closest one), attached_to}}}` in ONE pass, the inbox's one call), `GET /api/merge/history?joke=`, `GET /api/merge/{id}`, `POST /api/merge`, `POST /api/merge/undo`, `POST /api/merge/dismiss`. Shapes are in the contract. Registered from `Handler.creative_route` (needs the creative layer; `--no-creative` turns them off).
- A merge is ordinary decision lines (`kb.py` now allows `hidden` on a joke edit), so both backends replay it and `/api/undo` can reverse any one line; `POST /api/merge/undo` reverses all of them, newest first.
- The merge store is `merges` (`comedy/kb/creative/merges.jsonl` + `merges.json`; on SurrealDB also `creative_event` rows with `store = "merges"`, the same reconcile as the other creative stores).
- Same-idea groups need the offline judge cache `comedy/kb/creative/merge_judge.json` (`python -m creative merge judge`). Without it there are no groups (never a guess).
- `POST /api/ideas` adds `attach` (see the contract). The jot's vector comes from `db/embedder.py` (local node); if that can't start, only an unmistakable word match can attach. The attach never fails the idea.
- `GET /api/jokes` leaves attached notes out unless `?include_hidden=1`.

## Auth (`auth.py`)
Off locally (loopback only; `--host` other than loopback refuses to start without it). `PK_AUTH=1` (hosted):
every route except `/api/health` needs `Authorization: Bearer <Keycloak access token>`: RS256 via the realm's JWKS
(cached, refetched on an unknown kid at most every 30 s), `iss` exact, `aud` contains `pk-api`, 60 s skew, refresh
and ID tokens refused. 401 carries `WWW-Authenticate: Bearer`. A `worker` token may use only `/api/worker/*`.
`/api/live` takes the token as its first message `{"type":"auth","token":"..."}`, never in the URL.
Signed out, `/api/health` answers only `{ok, rev, auth}`. Tests: `python -m pytest api-dev/tests -q`.

## Hosted (`api/entry.py`, `api/Dockerfile`)
Same code, configured from env (infra/k8s/30-api.yaml). SurrealDB is the source; db/push_remote.py loads it and the
API writes its text files under `/tmp` (infra/README.md "Data load"). Hybrid search falls back to BM25 (no model).

## Not here yet
- Booking future shows: `next_show` is always `null`.
- Running the pipelines on uploads.
- Live laugh matching while recording.
