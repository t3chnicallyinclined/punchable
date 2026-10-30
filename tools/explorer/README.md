# Desk explorer: browse, confirm, change and extend the joke graph

A local web tool for the comic. It shows the automatically built knowledge graph (`comedy\kb\v0\graph.json`) with every decision he has made (`decisions.jsonl`) replayed on top. Every tap appends exactly one decision line. The data contract is `docs/KB_SCHEMA.md`.

## Start
```
python <repo>\tools\explorer\server.py
```
Then open **http://127.0.0.1:8795/**. Stop it with Ctrl+C.

- It needs only Python 3 (standard library). There are no installs, no CDN and no network calls.
- It binds to `127.0.0.1` only. Use a narrow browser window to get the phone layout.
- If `graph.json` doesn't exist yet, the page says so and checks again every 10 seconds. When a pipeline rewrites the graph, the next request picks it up, with no restart and no code change.
- Options:
  - `--graph <file>`: the graph to read. Default `~/punchable-data\kb\v0\graph.json`.
  - `--decisions <file.jsonl>`: default `decisions.jsonl` next to `--graph`.
  - `--report <KB_REPORT.md>`: default `KB_REPORT.md` next to `--graph`.
  - `--corpus <dir>`: for `sets\<set>\laughs_auto.json`. Default `~/punchable-data\corpus`.
  - `--port N`: default 8795. On 2026-10-25, 8765, 8766, 8780 and 8791 were taken.
- To try it on the fixture without touching the real kb:
  `python tools\explorer\server.py --graph tools\explorer\fixtures\graph.sample.json --decisions %TEMP%\dx_decisions.jsonl --corpus tools\explorer\fixtures\corpus --report tools\explorer\fixtures\KB_REPORT.sample.md`

## Pages
| Page | What |
|---|---|
| **Jokes** `#/` | Search (unit text, titles, labels), filter by stage and label, counts of to-review / confirmed / rejected links per joke, and sorting by most to review |
| **Joke** `#/joke/<id>` | Verbatim units grouped by date, with each doc title linking to its Google Doc. Performances with their transcript span and auto laughs as 0–4 bars. Every label and connection with status colour, confidence, reason and evidence quotes. ✓ / ✕ / ✎ on each. Add label (existing or new custom, with an optional definition), add connection (joke, label or a new idea; any kind or a custom one; a reason is required), change the stage, and undo your recent changes |
| **Graph** `#/graph/<id>` | A home-made force layout (SVG) of the joke's neighbours within 1 or 2 hops, with edge-kind filters and optional units, performances and rejected links. Tap a node to open it |
| **Labels** `#/labels` | Every label with its definition and the jokes carrying it (green confirmed, blue proposed, red rejected). Rename or merge a label; each is one decision. Unmerge or undo at any time |
| **Review** `#/review` | One proposal at a time, sorted for learning value. Keys: **Y** yes, **N** no, **E** edit, **S** skip (skips are not saved), **U** undo. A "Rejected" tab lists rejected items |
| **Learned** `#/learned` | Confirm and reject rates per edge kind and per label, your custom labels, connections and new ideas, and recent decisions with Undo |

## Data rules
- **graph.json is opened read-only, always.** The server refuses to start if `--decisions` is the graph file or isn't a `.jsonl` file.
- Each action appends one line to `decisions.jsonl`: `ts, target, id, action, before, after, reason, by:"owner"`. The file is opened in append mode and flushed + fsynced. If the last line is torn, the next line starts on a fresh line, and unparseable lines are skipped and counted (`/api/health`).
  - `before` is the full record as it was. That lets a confirmed edge survive a pipeline re-run that drops it.
  - Confirm sets `after = {status: confirmed}` and reject sets `after = {status: rejected}`.
  - Edit sets `after` = the changed fields (`kind`, `custom_kind`, `src`/`dst`, `stage`, `name`, `definition`, `merged_into`) and marks the edge confirmed.
  - Add creates a new edge (ID `e:sha1(src|kind|dst)[:12]`, using the custom kind name for custom edges), a new label `label:<kind>:<slug>`, or a new joke `joke:<slug>` at stage idea.
  - Delete is only allowed for things the comic added.
- **Undo appends an inverse line** with an extra field `undo_of: <line index>`. Add ↔ delete; confirm, reject and edit become an edit back to the previous values. Undone lines don't count in "Learned".
- **Merge** = one node edit `{merged_into: <label>}`. In the view, links to the merged label point at the target, and a duplicate link is shown once.
- Rejected items leave the queue and the default views but stay under "show rejected" and Review → Rejected.

## Review order
1. Edge IDs listed under the "first 50" heading of `KB_REPORT.md`, in that order (only that section).
2. Everything else by `(0.55 × uncertainty + 0.45 × impact) × kind weight`.
   - uncertainty = `1 − |confidence − 0.5| × 2`.
   - impact = log of how many links the two ends have.
   - Kind weight favours `same_shape`, `callback_to`, `variant_of`, `has_label` and `same_premise`.

## API (all JSON)
`GET /api/health · /api/meta · /api/jokes?q=&stage=&label= · /api/joke/<id> · /api/labels · /api/queue?status=proposed|rejected · /api/graph?id=&hops=1|2&kinds=a,b&all=1&rejected=1 · /api/stats · /api/decisions?limit=`
`POST /api/decision {target, id?, action, after?, reason?}` and `POST /api/undo {index?}` (no index = the latest decision that is still in effect).

## Tests (temp copies only; never the real kb)
```
python tools\explorer\test_explorer.py      # replay, undo, merge, torn lines, missing/half-written graph, every endpoint
node   tools\explorer\test_browser.mjs      # headless Chrome: every page, all actions, reload, 375 px, no JS errors
```
`SHOTS=<dir>` makes the browser test save phone screenshots. `fixtures/make_fixture.py` regenerates the fixture (invented jokes, schema-exact, deterministic edge IDs).

## Limits
- One person, one machine. There is no auth and no locking beyond one server process. Don't run two servers on the same `decisions.jsonl`.
- The view is rebuilt from scratch whenever an input file changes, which is fine for thousands of edges. The graph view caps at 120 nodes.
- Undoing an older decision after a later one changed the same item restores that older decision's "before" values.
- Laughs are matched to a performance by time window only (from the start of the span to 3 s after its end). Laugh decisions (`target: "laugh"`) aren't editable here yet.
