# Punchable: the target architecture (2026-10-29, revised the same day after the expert reviews)

The one document that says how Punchable is laid out and why, for the whole system: the app (TypeScript), the services
(Rust), the database, sign-in, operations, and the conventions a contributor follows. `docs/RUST.md` is the port plan
under it, `docs/RUST_WORKSTREAM.md` the board, `docs/adr/` the decisions one by one, `docs/design/ARCHITECTURE.md` the
app-side audit this builds on, `docs/design/HIG.md` the iPhone rules, `docs/design/DESIGN.md` (to be written, lane A0)
the design values. Public-safe: no hostnames beyond punchable.io, no material, no other project's names.

Reviewed before sign-off by the Punchable expert (product and stack), the iOS dev expert (§7 native) and the iOS design
expert (§7 tokens and kit); their corrections are folded in and marked **[review]** where they overturned the first
draft. Tags: **C** read in a file or run; **I** inferred; **U** unknown until measured.

## Owner decisions this document needs

1. Sign off on the whole, or say what to change.
2. **`venue_short`** **[review]**: the one rule becomes the app's semantics (`{name, guessed}`, no year; prose adds the
   year itself), because that is what the Sets list he uses most shows today. The one real call left: "Comix at X"
   becomes "X" (server style) or "Comix X" (app style).
3. **"Reconnect Google" in Settings is reachable on the phone today** **[review]**, which opens the Google path he
   excluded on native: gate it to web, or accept it.
4. **Where the Google Docs sync lands** (a worker kind, a CronJob, or Python on the PC for now; ADR 0004 says its own
   ticket).
5. Four visual values the design expert's token work would change on screen: the content heading from 30 to 28 pt
   (Title 1); prose growing to 2.0× under Dynamic Type (today capped at 1.6×); the paper card's shadow in light mode
   (measure first); spacing and radius snapping by up to 2 pt as 24 spacings collapse to 9 and 18 radii to 5.

## 0. The system in one picture

```
iPhone (Expo RN, TestFlight)  ─┐                          ┌─ Keycloak (realm punchable; pk-web, pk-mobile, pk-api, pk-worker, pk-gsync)
Web (react-native-web)        ─┼─ HTTPS ─► pk-api (axum) ─┤
                               │     ▲        │  LIVE      └─ SurrealDB StatefulSet (the source of truth; fn::apply_decision; creative_event; srcfile)
pk-worker (claim/run/result) ──┘     │        │
        │                            │        └─ in-process: bge-small query embedding (ort), PCA + k-means for the map
        └─ Anthropic Messages API    └─ /metrics, crash reports → Prometheus / Grafana / Bugsink (self-hosted)
```

Deployables, all in one namespace **[review]**: the API, the worker, the web image, SurrealDB (a StatefulSet), the
nightly backup CronJob (`infra/backup/backup.py`, Python on a stock image: it is a Job, not a service, and stays Python
under ADR 0001 until an `xtask backup` earns its place), the `dbload` Job (on the API image until T11), Bugsink. The
worker speaks HTTP to the API only, never to the database (ADR 0011). The transcriber kinds (`process_set`,
`import_youtube`) stay off the server (ADR 0012). The legal rules run on the API on every ingest and request path from
a mounted Secret; the worker's copy has no Secret and runs the generic nets only, which is defence in depth because the
API re-checks every worker post (C, `ingest_api.py`, `35-worker.yaml`).

## 1. The Rust workspace: crates and the one dependency rule

Eleven crates: the nine in `docs/RUST.md` plus two small leaves, `pk-core` and `pk-wire`.

```
tier 0  pure, no internal deps     pk-core                     pk-wire
tier 1  pure domain                pk-kb -> core
tier 1  IO leaves                  pk-llm -> core    pk-embed -> core    pk-obs -> core
tier 2  domain + IO                pk-creative -> kb, core     pk-store -> kb, core
tier 3  binaries                   pk-api    -> wire, core, kb, creative, store, embed, obs
                                   pk-worker -> wire, core, kb, creative, llm,   embed, obs
tooling                            xtask (no internal deps)
```

**The rule (ADR 0010):** an edge only points to a lower tier. Pure crates (core, wire, kb, creative) never depend on an
IO crate (store, llm, embed, obs); IO crates never depend on each other; the binaries never depend on each other; the
API never depends on the model client. `cargo deny check bans` enforces it with `wrappers` (which crates may use
`surrealdb`, `ort`/`tokenizers`, `pk-store`, `pk-llm`, `axum` directly). Today's graph already obeys it (C, `cargo
metadata`).

| Crate | Owns | Must not hold |
|---|---|---|
| `pk-core` | `py` (the Python-format helpers: `repr`, `str`, `quote`, `float_repr`, `dumps`, `isspace`, `truthy`), `text::head`, `time::now_iso` + a `Clock` for tests, `config::Env` (one parse of `secrets.env` with the process env over it, typed getters, `ConfigError`), `canon` (the canonical JSON form for line hashes, lands with T10). Deps: `serde_json`, `chrono` only | domain rules, regexes, any service's key names |
| `pk-wire` | the typed HTTP contract: **the whole `/api/worker/*` surface** **[review]** (claim with `worker`, `hosted`, `model`, `paused`; job; result; files, `files/content`; decisions; creative lines; events; ideas; `profile/voice`; `jokes/<id>`; `state`; `decision`; `undo`; `jobs/<id>/file`; the PC-only ones marked for deletion at T11) and the 58 app-facing response types now hand-written in `app/src/lib/{api,creative,simple}.ts`. Open KB records stay `#[serde(flatten)] extra`. After T9 it generates the app's TypeScript types with `ts-rs` | behaviour, errors, internal deps |
| `pk-kb` | the graph with no IO: typed ids (`edge_id`, `Actor`), `Decision`, replay, merge resolution, the read index, the decision builder, `Rating` | |
| `pk-creative` | rules with no IO: store reducers, the verbatim gate (one copy), `ask` (retrieve / prompt / finish), `words` (one tokeniser, named stop-word sets), `display` (`mmss`, `month_year`, `short_name_of`, `venue_short`), `voice` (the profile and learned blocks), **`legal`** (rules JSON in, `content_is_legal` out) **[review]**, **`settext`** (the `Inbox.split` prompt half and finish half, `engine/inbox.py:440-483`, `worker/execute.py:124-192`) **[review]**, the writing and merge prompt halves | |
| `pk-store` | SurrealDB through the official crate: typed rows, `kbmap`, the decision-append transaction, the `creative_event` mirror, LIVE | |
| `pk-llm` | the Anthropic Messages client, the TOML router, schema validation, prompt caching, the USD budget | |
| `pk-embed` | bge-small over `ort` + `tokenizers`, CLS pooling, the hash embedder for tests | |
| `pk-obs` | tracing init, the metrics registry (names and label values identical to Python's), sentry with the one scrubber | |
| `pk-api` | lib + thin bin: axum router, JWKS auth, the snapshot, `/api/live`, routes one module per route group | the model client |
| `pk-worker` | bin: the claim/run/result loop, the kinds (static dispatch), `probe`, the SIGTERM handback | the database |

Why `pk-core` now: the Python-format helpers exist three times (`pk-kb/src/model.rs`, `pk-creative/src/pyish.rs`,
`pk-llm/src/pyfmt.rs`) and already disagree on numbers (serde's `to_string` vs Python's float repr, C), and env reading
is spread over five crates. The rule that comes with it: **library crates never call `std::env`**; a binary builds
`Env` once and passes typed config down.

Why `pk-wire`: the contract has no Rust home. pk-kb's read models return `serde_json::Map`, and the worker protocol is
untyped on both sides (C, `worker/__main__.py`, `worker/online.py`). Without it pk-api and pk-worker would each grow
their own copy.

Dropped from the plan: `insta` (one way to do goldens: Python writes JSON, Rust compares parsed values), `clap` until a
binary needs it, `reqwest-eventsource` (T5 chose `eventsource-stream`).

## 2. pk-api

A library (`fn app(AppState) -> Router`, `async fn serve(ApiConfig)`) plus a `main.rs` that builds `Env`, starts
`pk-obs` and calls `serve`. Route tests are `tower::ServiceExt::oneshot` against `app()`, no socket.

```
pk-api/src/  lib.rs main.rs config.rs state.rs error.rs
             auth/{verifier,principal,actor}.rs   snapshot.rs  embed.rs  live.rs  proxy.rs (T9 only)
             routes/{health,tonight,bits,sets,search,creative,ask,merge,say,ingest,memos,profile,suggest,cloud,worker,metrics}.rs
```

- **One route module per Python route module**, so T9's Ingress split is one path prefix per module. Routes depend on
  `AppState`, never on each other.
- **The T9 split is by prefix, and an Ingress prefix routes every method** **[review]**: `/api/jokes`, `/api/sets` and
  `/api/map` all carry POSTs beside their GETs (C), and a write on the wrong side 409s every Python write (RUST.md
  fact 6). So `pk-api` at T9 forwards any non-GET under a moved prefix to the Python Service through a `tower`
  reverse-proxy fallback (`proxy.rs`, deleted at T10), and the first prefixes moved are the GET-only ones
  (`/api/search`, `/api/today`, `/api/labels`, `/api/learned`, `/api/export`, `/api/meta`, `/api/backend`,
  `/api/audio`). During T9 the two API deployments carry an `impl` label so no Grafana panel mixes them.
- **Read-after-write across the two processes** **[review]**: the app refetches on reload, focus and `change` (C), and
  the Python answers a GET from the process that just wrote. A debounced Rust snapshot could answer that reload from
  the previous generation and his edit would vanish until the next change (I). Gate at T9: "POST to Python, GET from
  Rust within 100 ms returns the new row", met by reading `meta:state.gen` per request and waiting (bounded) for the
  snapshot to catch up.
- **Auth cannot be forgotten:** the worker router is nested under `/api/worker` with `require_worker` as a route layer,
  the app router carries `require_owner`; `/api/health` and `/api/live` sit outside both. `Owner` / `Worker` extractors
  hand handlers a typed principal; `Actor::for_write` is a function (the body is consumed once). The five refusal
  texts and statuses are kept exactly (`server.py:1123-1147`, C). Auth off is refused unless the bind address is
  loopback. `/api/health` keeps `flags` and `auth` exactly: `flags` is the only way to turn a phone feature off without
  a build (C, `app/src/lib/flags.ts`).
- **Errors:** `ApiError` renders `{"error": msg}` with `Cache-Control: no-store`; a 401 carries exactly
  `WWW-Authenticate: Bearer realm="punchable", error="invalid_token"`; axum's body rejections are mapped to the Python
  texts (`body too large`, `body is not JSON`, `body must be an object`); `KbError::Refused` maps to 400/409/503;
  anything else is a fixed 500 and goes to sentry through the scrubber.
- **The snapshot:** `tokio::sync::watch<Arc<Snapshot>>`. One task owns the LIVE SELECT on `meta:state` and the watched
  tables; on a `gen` change it debounces, rebuilds in `spawn_blocking`, sends. The same channel feeds `/api/live`
  (a `hello` on connect, 20 s pings, then `{"type":"change","rev":"db:<gen>"}`, the Python shape, C). The ETag / 304
  leg of the first draft is dropped **[review]**: the Python sends `no-store` on every JSON and the app never sends
  `If-None-Match`; `rev` stays the thing the app compares on `/api/health`.
- **Embeddings in-process, nothing else:** the search query embedding runs behind `Semaphore(1)` + `spawn_blocking`;
  the Bits search's KNN, BM25 and RRF run inside SurrealDB; PCA and k-means use the same permit, cached per `gen`. No
  LLM in the API. **askchat keeps its in-process scorer** **[review]**: the 29 retrieve goldens prove the written-out
  formula over an in-memory notebook, and SurrealDB's `search::score` is a different scorer that would change which
  sources the model sees; only the Bits search uses the database's BM25.
- **Sized for 1 CPU:** tokio `worker_threads(2)`, `max_blocking_threads(4)`; ort `intra_threads(1)`,
  `inter_threads(1)`, arena off, memory pattern off; `tokenizers` parallelism off (rayon is in the tree through
  `tokenizers 0.22.1`, C). Gate: thread count after the first search ≤ 2 + 4 + the runtime's own.
- **Edges:** `CorsLayer` reproducing `origin_ok`; `CompressionLayer` on JSON only (never audio ranges or uploads);
  uploads streamed to disk, never buffered, deduplicated on an `X-Idempotency-Key` the app sends **[review]**;
  `/metrics` open on the main router with the `process` feature; SIGTERM flips readiness to 503, waits 10 s, then
  drains (this replaces the manifest's `preStop: sleep 10`, which the distroless image cannot run).

**Memory budget against 200 MiB warm (all I, to be measured):** binary + runtime + clients 25 MiB; bge-small fp32 in
ORT + tokenizer 135 MiB (the model file is 126.9 MiB, C); snapshot ×2 during a swap 30 MiB (U); buffers and caches
10 MiB. No headroom. The three likeliest overruns and how each is measured: ORT's own allocations (a `pk-embed` example
printing `Anonymous:` from `/proc/self/smaps_rollup` after one and after 1,000 embeds, run in the same image on the
server); the double snapshot plus allocator fragmentation under a write burst (cgroup `memory.stat` `anon` before,
during, after 100 decision writes); buffered bodies or an uncapped blocking pool (upload 200 MB, `anon` moves by MiB
not hundreds). The target is restated in cgroup `anon` and marked at risk until measured; if the model number lands
near 250 MiB the honest target is 256 MiB with the request raised, still far under Python's 371 MiB idle.

## 3. pk-worker

One current-thread runtime; one job at a time (`Recreate` guarantees one worker). The loop is the Python's
(`worker/__main__.py:446-472`): heartbeat → budget check (pause with a ≤ 300-char reason) → `POST /api/worker/claim`
with a `pk_wire::Claim` carrying `worker`, `hosted`, `model` (what Settings shows) and `paused` → idle 15 s on nothing
→ run → post a `pk_wire::JobResult` → heartbeat; on a loop error back off 5 s to 300 s doubling and report the crash.
`api::Client` is the one factory (client credentials as `pk-worker`, token cached until 30 s before expiry, dropped on
401, HTTP only).

**Every writing job fetches the voice profile and the learned block first** **[review]**: the Python calls
`GET /api/worker/profile/voice` per job and never fails a job on it (C, `creative/voice.py`), and those two cached
system blocks are the whole "what it has learned" loop. A Rust worker that skipped the call would write without his
voice and nothing would error. The T8 differential asserts both blocks, in order, with their `cache_control`, in every
writing request body.

Kinds are a trait with static dispatch (no `dyn`): one `dispatch(kind, …)` match names every hosted kind, so a new kind
is a compile error until handled. Hosted: bridge, punchup, facts, riff, ask_chat, set_text; embed, memolink, ask when
bge is in the image. Not ported: process_set, import_youtube (ADR 0012). `set_text` is known, not U **[review]**: one
`Inbox.split` prompt (windows, `SYS_VOICE`, task `split` on Opus, span clipping, intro/outro dropped) then ideas as
`ai:set_text` via `POST /api/worker/ideas` and three files via the files route; the in-memory workspace must emit the
exact srcfile paths `db/srcpaths.py` whitelists, because the API applies the legal rules to them.

The workspace is typed and in memory (a `pk_kb::View`, a `pk_creative::Store`, a sha-keyed srcfile listing), not a
file tree copied to disk per job (I: lane G confirms kind by kind). New behaviour: `pk-worker probe --max-age 20m`
replaces the manifest's `sh -c find` liveness probe (distroless has no shell, C for the manifest); SIGTERM mid-job
posts `{ok:false, retry:true, error:"the worker was stopped; the job goes back to the queue"}` within the 60 s grace;
budget hit mid-job posts a retry and no crash report; `:9100` is a two-route axum listener. Crash reports go to
Bugsink through the `sentry` crate with the same release string `punchable-worker@<version>+<sha>` and the crate off
without a DSN; the T8 gate runs "Verifying a crash lands" once with the Rust worker, since even the Python path is
unproven end to end (C, HANDOVER open 3). **The spend ledger lives on an `emptyDir` and resets on the swap**
**[review]** (C, `35-worker.yaml`): say so in the deploy-log entry, and move the ledger into the API at T10 so it
survives pods.

## 4. One home per rule

How a rule that must exist in both Rust and TypeScript stays one rule: a `contracts/` folder at the repo root holds
JSON case files (input → expected). `cargo xtask contracts` writes them from the Rust home, CI fails on drift (the
`goldens-current` pattern), and the app's node tests assert the TS implementation over the same cases.

| Rule | Single home | Python copy dies at | App side |
|---|---|---|---|
| Verbatim gate, inline-quote stripping | `pk-creative::verbatim` (askchat's variant) | `engine/verify.py` at T8; `creative/askchat.py` at T11 | — |
| Decision replay | SurQL `fn::apply_decision` is the hosted truth; `pk-kb::view` in memory; both kept, the three-way test stays in CI permanently (ADR 0009) | `surreal_kb.py` overrides at T10; `kb.py` ships in both images until T11, then stays as the PC tool and oracle **[review]** | — |
| Metrics, `build_info`, label sanitisers | `pk-obs::{metrics,labels}` | `worker/metrics.py` T8; `engine/prom.py`, `api-dev/metrics.py` T11 | — |
| Crash scrubber | `pk-obs::scrub`, the one `before_send`; adopt the app's four extra keys | `worker/crashreport.py` T8; `api-dev/{scrub,crash}.py` T11 | `crash.ts` stays, pinned by `contracts/sensitive_keys.json`; fold `crashlog.ts` into it |
| `edge_id`, `Actor`/`ACTOR_RE`, `his_material` (the real `by_kind` rule, `creative/data.py:26`) **[review]**, `Rating` | `pk-kb::ids`, `pk-kb::model::Rating` (a test parses the `schema.surql` ASSERT list against the enum) | engine/infer/api copies T8/T11; `server.py` actor rules T10 | the server sends `by_kind: owner\|import\|ai`; delete the prefix test in `voiceProfile.ts`; `laughs.ts` keeps its button list, pinned by `contracts/ratings.json` |
| Legal rules | `pk-creative::legal` (pure); the API holds the private list via the existing Secret; the worker keeps the generic nets **[review]** | `gsync/rules.py` copies in the images at T11 (`gsync/rules.py` itself stays for the PC sync) | — |
| `now_iso`, `OWNER_ID`, `COMIC`, bge dir, `secrets.env` | `pk-core::{time,config}`; `pk-creative::voice`; `pk_embed::bge_dir_in` | worker copies T8; API copies T11; PC tools keep theirs | — |
| `_send`, `HttpError`, `ID_RE`, `_ts` ×3, `store(cr,kb)` ×4 | `pk-api::error`, `NodeId` path extractor, `pk-core::time`, `AppState` | T10 | — |
| Tokenisers ×6, stop-word lists ×7 | `pk-creative::words`: one tokeniser, named sets per purpose, merged only where goldens agree | T8 / T11 | `sayRules.ts` FILLERS stays, pinned by `contracts/say_fillers.json` |
| `mmss`, `month_year`, `short_name_of` | `pk-creative::display` (T4's `ask/words.rs` copies move there after T4) | T11 | **the server sends `short_name`, `when_label` ("Jun 2015"), `at_label` ("3:07") on list rows** (`when` / `at` are taken: `at` is a board position elsewhere, C **[review]**); the app keeps one `format.ts` for clocks it runs itself |
| `venue_short` (three rules today: server, app, and `mysets._where` re-adding a year **[review]**) | `pk-creative::display::venue_short(venue) -> {name, guessed}`, no year, the app's semantics (the Sets list shows it); prose callers add `'YY` themselves | `creative/data.py:56-70`, `mysets.py:61-62` T11 | the server sends `venue_short` + `venue_guessed`; owner call: "Comix at X" |
| `isQuestion` (two rules in the app) | app only: `sayRules.ts` | — | delete `simple.ts:76` |
| `QUERY_PREFIX`, bge dir | `pk_embed` | `db/embedder.py` T8; `engine/embed.py`, `search_api.py` T11 | — |
| Worker `Client(...)` ×4 | `pk-worker::api::Client` | T8 | — |
| Line identity / hash | `pk-core::canon` + `pk-store::kbmap::line_hash`, one-time re-hash (RUST.md fact 3) | worker `h()` T8; `creative_api.py` T10 | — |

**Stays in TypeScript** (pure UI rules tested under node): `layoutRules`, `routeTable`, `recorderLogic`, `uploadsCore`,
`retry`, `flags`, `session`, the auth token machine, `galaxyLayout` / `cloudMath`, `searchRows` (row assembly once the
server sends `at_label`), `sayRules`, `crash.ts`'s `routePattern`.

**Sequencing (T16):** the Python API gains `short_name` (already sent), `venue_short` + `venue_guessed`, `when_label`,
`at_label`, `by_kind` first. The rule for the builds already on TestFlight **[review]**: no existing key changes type;
the shipped build's read set (the types in `api.ts`) is the acceptance test. The app's hand-written types ignore unknown
keys (C), so the additions are safe.

## 5. Conventions

- **Errors:** one `thiserror` enum per library (`KbError`, `CreativeError`, `StoreError`, `LlmError`, `EmbedError`,
  `ObsError`, `ConfigError`); `anyhow` only in `main.rs` and xtask. Every `#[error]` text the app can show is part of
  the contract and changes only with its golden or contract file in the same commit.
- **Config:** only binaries read the environment; each crate exposes `XConfig::from_env(&Env)` and a plain constructor
  for tests. The Rust binaries read the same Secret names as the Python (`punchable-api`, `-r2`, `-ai`, `-sentry`,
  `-legal-rules`, `-worker`, `-anthropic`); a rename is a rollout restart, so none is planned.
- **Tracing:** one request span with `method`, `route`, `status`, `ms`; worker spans carry `job_id`, `kind`; never log
  bodies, query strings, titles or ids. Gate: a capture-layer test sends a sentinel in path, body and query and asserts
  it appears in no event.
- **Metrics, and what the dashboard needs** **[review]**: `pk_` prefix, `_total` / `_seconds`; names identical to
  Python's until T11 (T7 proved 21 equal). **Label values too:** the `route` label is the Python's `route_pattern`
  output (every id-like segment is literally `{id}`, depth-capped), so axum's `MatchedPath` goes through the same
  sanitiser and a 404 gets `{unmatched}`; the latency alert excludes `/api/upload` and `/api/audio/.*` by that value
  (C, `60-observability.yaml`). The 13 rules key on `pk_worker_heartbeat_age_seconds`, `pk_http_requests_total{status}`,
  `pk_http_request_duration_seconds_bucket{route}`, `pk_ai_paused`, `pk_jobs_waiting_transcriber_oldest_seconds`;
  `pk_jobs_queued` is read from the job store at scrape, so only the process that owns the queue (Python until T10)
  reports it. `up{job=…}` comes from the Service's `app` label and the port names `http` / `metrics`, unchanged. The
  Python `test_metrics.py` cases become a golden for `pk_obs::labels`.
- **Testing per crate until T11:** goldens from the Python oracle (compared as parsed JSON), proptest where equality
  cannot hold, `wiremock` for Anthropic and JWKS, the scratch SurrealDB for pk-store and pk-api, `cargo-mutants` on
  the replay, the gate and auth (zero surviving mutants or each listed with a reason). **A skipped test is a failed
  test:** CI sets `PK_REQUIRE_EXTERNAL=1` with `surreal` and the bge model installed, so the pk-store and pk-embed
  suites that return early without them (C) count as run.
- **After T11:** tag the last two-language commit `python-oracle-final`; the generators stay runnable on demand
  (`cargo xtask oracle`); goldens become fixtures with `MANIFEST.sha256` per crate (a golden changes only with its
  manifest in the same commit); mutation runs stay; the three-way replay against SurrealDB stays forever.
- **`pk-testkit`** (golden loader, value diff, scratch-DB lifecycle) is introduced when pk-api needs the scratch DB
  (T10), not before.

## 6. The repo as a team would keep it

In CI from now: `PK_REQUIRE_EXTERNAL=1`; `rust.yml` also triggers on `db/schema*.surql` and
`engine/config.hosted.toml` (they are `include_str!`-compiled, C); `cargo deny` (advisories, licenses, the DAG bans) in
`cargo xtask gate`; the oracle's Python tests (`creative`, `engine`, `infer`, `gsync`) in `api.yml` until T11; `ruff
check --select E9,F` over shipped Python only; `npm run lint` in `app.yml` with today's count as a ceiling that only
falls. Repo files: `docs/adr/` (seeded with the twelve standing decisions), `CLAUDE.md` at the root and in `rust/`,
`docs/README.md` as the one index (the eleven dangling references in the audit get fixed or deleted), a PR template
(what changed, the gate and its result, the red arm run, which goldens or contracts changed and why, the private-material
check). Deliberately not done: CODEOWNERS (one owner, branch protection does the work), issue templates, a CHANGELOG,
Dependabot (the pins are deliberate; `cargo deny advisories` covers security), a repo-wide Prettier pass, the
`pre-commit` framework (extend `tools/guard`), `.editorconfig`.

**Where the Rust images are built** **[review]**: not on the production node. `deploy.sh build` today runs
`docker build` on the server from the shipped tree (C); a cargo build with thin LTO and `codegen-units = 1` of eleven
crates, with `ort` downloading its runtime at build time, is the heavy run the owner forbids on a box doing real work.
The images are built in GitHub Actions (or on the PC), `docker save`d, `docker load`ed over ssh and pushed to the
cluster's registry; `deploy.sh build` learns a `rust/Dockerfile.<svc>` path and `PK_BUILD_SHA` keeps feeding
`pk_*_build_info{sha}` and the Bugsink release.

## 7. The app (TypeScript stays, ADR 0003)

Adopt the layout in `docs/design/ARCHITECTURE.md` §1 (`src/app` → `src/features/*` → `src/ui`, `src/api`,
`src/auth`, `src/platform`; no imports between features; a `static.layers` test) with the amendments below. The iOS dev
expert verified each against Metro, Node 22 and the SDK 58 docs; the iOS design expert set the token and kit
structure; visual values beyond the HIG's numbers are in `docs/design/DESIGN.md` (lane A0).

1. **`src/rules/` as the bottom layer:** pure modules tested under node, no React imports, imports with explicit `.ts`
   extensions (`allowImportingTsExtensions`; expo's base tsconfig already sets `noEmit`). Verified **[review]**: Node 22
   needs the extension, Metro resolves the exact path first, and no lighter fix exists (a loader or `tsx` adds a
   dependency). Two guards: `src/rules/` forbids platform twins (an explicit extension skips `.ios`/`.native`
   resolution), and `tests/static.cycles.test.mjs` must try the bare specifier before appending extensions or edges
   inside `src/rules` vanish from the graph. Holds `format.ts` (one `mmss`, `monthYear`, `fmtDate`, `MONTHS`, replacing
   four formatters and six month arrays), `sayRules`, the rules half of `laughs`, `layoutRules`, `routeTable`, `retry`,
   `flags`, `searchRows`, `cloudMath`, `bitWorkRules`, `recorderLogic`, `uploadsCore`.
2. **One API client, and the upload path's real bug** **[review]**: `src/api/client.ts` exports `authHeaders()` and
   `uploadHeaders()`; `uploads.native.ts` uses them instead of its own Bearer header (C). Today a 401 from the
   background upload is treated as permanent and **deletes the recording from the queue** (the file stays on disk,
   unreferenced; C, `uploadsCore.ts`), and the stale-token path after a transient refresh failure is exactly the
   "offline at the venue, back online" case. So `send()` treats 401 as transient: refresh, retry once with fresh
   headers, keep the item queued; every upload carries `X-Idempotency-Key: item.id` so a task killed mid-flight cannot
   duplicate on relaunch; `tests/uploadQueue.test.mjs` gains the 401 case. The two poll loops (`ask.tsx`, `simple.draft`)
   become one `poll()`; after T9 `src/api/wire.gen.ts` from `pk-wire` replaces the hand-written shapes.
3. **Auth:** `lib/tokens.ts` (the token state machine) becomes `src/auth/tokenMachine.ts` so `src/ui/tokens.ts` can
   mean design tokens (two importers, no runtime risk, C). The code-exchange effect duplicated in `login.tsx` and
   `settings.tsx` becomes one `useCodeExchange(result, request, …)` that keys on `result` only and guards with a ref of
   the last handled result **[review]**: today the effect also keys on `request`, which is rebuilt when discovery
   lands, so a late rebuild re-exchanges the same code and Keycloak answers `invalid_grant` after a successful sign-in
   (C). "Reconnect Google" is owner decision 3.
4. **Tokens and the kit (structure here, values in DESIGN.md)** **[review]**: `src/ui/tokens.ts` holds five semantic
   scales, dark default with a light twin for every colour role: `color` (roles: backgrounds base/elevated/card, labels
   in four levels, separators, `tint` with ink and soft variants, status, stage, rating, cluster and heat sets, scrim,
   the fixed scheme-independent set for the paper card, white and record red), `type` (the eleven iOS Dynamic Type
   styles plus `display` and `clock`, each `{family, size, lineHeight, cap}` with the 11 pt floor and **the Dynamic Type
   cap per style**: prose 2.0×, titles 1.8×, captions and controls 1.6×, the clock 1.2×; today every `T` caps at 1.6×
   against the HIG's 200 % rule, C), `space` (9 steps from 24 values), `radius` (5 steps from 18), `size` (44 pt
   control, 28 pt minimum with hit slop, the stage disc and ring, the 44 pt row). The kit is twelve components, each
   handling Dynamic Type (height from the font scale), VoiceOver (a label or a text child, actions mirroring gestures),
   Reduce Motion and one hand: `Text`, `Input` (replaces the 19 copied blocks; clear button, required label),
   `RenameSheet` (a native form sheet, Cancel leading, Done trailing; replaces three `RenameForm`s plus
   `ShortNameForm`/`TitleForm`), `ListRow`, `SwipeRow`/`DragList`, `Card`/`PaperCard` (the only user of the paper
   colours), one `Badge` with a `kind`, one `Button` with variants, `Chip`/`Seg`, `ProfileButton`, the sheets (web only
   once native form sheets land), `Skeleton`/`Empty`/`ErrorCard`/`Snack`. The static style bag `s` goes; `GoogleG` moves
   to `src/ui/brand`. HIG violations to fix with the tokens: `SayBar.tsx:129` 9 pt and `metrics.tsx:201` 10 pt text
   (floor 11), four tap targets under 44 pt without hit slop (`bit/[id].tsx:344`, `shows.tsx:40`, `write.tsx:264`,
   `joke/[id].tsx:202`), `STAGE` with no light twin and `RATING` captured at module load (`theme.ts`).
   **The gate**: `tests/static.values.test.mjs`, a per-file count map (not a set) of number literals for
   `borderRadius*`, `padding*`, `margin*`, `gap*`, `fontSize`, `lineHeight`, `letterSpacing` and
   `maxFontSizeMultiplier` outside `src/ui`, failing above and below each entry so the count only falls; `0` and
   expressions are not matched; Skia and Reanimated code needs no exemption (their props are not style keys, C);
   `width`/`height` stay out (geometry). Current counts: 174 radius, 691 spacing, 272 font-size literals.
5. **Twins and storage** **[review]**: `Screen.tsx` is the web shell and `Screen.native.tsx` the phone shell, so the
   pattern is `ScreenBase.tsx` (the shared hooks and blocks) imported by both, never a pure shim, and never `./Screen`
   from the native file (the self-import tests pin it). `DragListBase.tsx` holds props, `move` and the grip's
   accessibility actions; the `'worklet'` `targetIndex` stays in `.native` (babel adds the worklets plugin
   unconditionally and the web file exists to keep Reanimated out of that bundle). **ENGINEERING.md §1's resolution
   table is backwards** (Metro loops extensions outside platforms and `.ts` before `.tsx`: `X.ios.ts, X.native.ts,
   X.ts, X.ios.tsx, …`, C): fix the table, add "a split pair shares one extension" to `static.twins`, fix
   `static.cycles`' resolver. Storage: `src/platform/kv.ts` is a platform split, not one file: `expo-sqlite/kv-store`
   (`SQLiteStorage('punchable')`) on phones, `localStorage` with today's keys unchanged on web (web SQLite would need
   wasm plus COOP/COEP headers, C), so there is no web migration; `prefs.json` is read once and deleted; SecureStore
   keeps the refresh token; **sign-out clears the cache prefix** (today the phone cache is memory-only, so nothing
   clears it; once persisted his material would survive a sign-out). `expo-sqlite` is a native module: it lands in one
   commit with the `expo-font` file list (design/ARCHITECTURE.md §0 finding 1), one dev-device build, one TestFlight
   build, `expo-doctor` in that step's gate.
6. **After the move:** typed routes on (144 `as any` today; `expo-env.d.ts` generated in CI before `tsc`); the React
   Compiler only after `useTheme()` replaces the in-render `Object.assign(C, …)` (the compiler would memoise over that
   mutation and ship stale UI silently), with the healthcheck clean.

Order **[review]**: rules layer with `format.ts` and the one `isQuestion` → the upload fix → the auth rename and
`useCodeExchange` → `kv` + the fonts plugin (one native build) → `Input`, tokens and DESIGN.md → `ScreenBase` /
`DragListBase` → the folder move last (mechanical; 33 `Screen` importers rewrite in one commit per layer). Each step's
gate: `typecheck`, `test:unit`, `lint`, the web and iOS exports, and the dev-client smoke (ENGINEERING.md §6 step 8)
on every step that touches a `.native` file or a native module.

## 8. Order from here, and what "optimized and smooth" means

| # | Step | Gate | Ships? |
|---|---|---|---|
| 1 | T4 ask (done 2026-10-29, `52df9e3`) | the `ask_*` goldens | no |
| 2 | T14 honest CI (§6) | green with the scratch-DB and model tests counted as run | CI only |
| 3 | T12 `pk-core`; delete the copies in kb, creative, llm, obs, store; libraries stop reading env | every golden unchanged + a new float-in-refusal case | no |
| 4 | T13 `pk-wire`, the whole worker surface first | the Python API's worker-route responses round-trip equal as values | no |
| 5 | T15 measure `pk-embed` memory in the distroless base on the server | a number in RUST.md with the machine named | decides the target |
| 6 | T8 worker + `probe` + the liveness change + the SIGTERM handback + the voice/learned blocks + the Bugsink proof, one commit; built off the node (§6) | offline differential vs the Python worker (request bodies incl. the two system blocks, results, the claim fields, the srcfile paths); in prod the failed-jobs rate no worse than Python's week before; the ledger reset noted in the deploy log | **the first shipping artefact**; rollback = previous image digest |
| 7 | T16 the Python API sends `short_name`, `venue_short` + `venue_guessed`, `when_label`, `at_label`, `by_kind`; the app stops computing them | app tests over `contracts/*.json`; pytest; no existing key changes type | app + Python API |
| 8 | T9 reads: GET-only prefixes first, then the mixed ones with the non-GET proxy to Python; `/metrics` with the sanitised `route`; the `impl` label | the baseline GET generator over both base URLs, equal as values with the allow-list in the repo; the read-after-write gate (§2) | per group; rollback = remove the path |
| 9 | T10 writes + `/api/worker/*` + `/api/live` together, with the canonical line-hash migration and the ledger moved into the API | the POST sequence on scratch DBs equal as values; **the restore drill run once on a scratch SurrealDB first; `ship.sh backup-now` immediately before; rollback = that backup restored + the previous API digest**, because after the re-hash a Python redeploy 409s every creative write (C) **[review]** | cannot be split |
| 10 | T11 retire Python; `dbload` on its own image; the oracle tag; goldens → fixtures with a manifest | the same gates on the shipped images | after 9 has run in prod |

The app lanes (§7) run alongside; they only touch `app/`. The Google Docs sync's landing is owner decision 4.

**Four measurements** **[review]**, each taken on Python first the same way. The load script RUST.md and the
manifests cite as `infra/README.md` "Memory" does not exist (C): writing it is the first task of T15, and the common
memory unit for both languages is cAdvisor's `container_memory_working_set_bytes` (the Python `/metrics` has no process
collector).

1. **Memory:** the API pod warm and idle, and after the load script. Target ≤ 200 MiB, or what step 5 justifies.
   Python: 371 MiB idle after a restart, 450 MiB before it, ~560 MiB during an embed sweep, 824 MiB of 1 GiB on
   2026-10-28 (all recorded).
2. **Latency:** p95 per route group under the same script,
   `histogram_quantile(0.95, sum by (le, route) (rate(pk_http_request_duration_seconds_bucket[5m])))`. Target: Rust
   p95 ≤ Python p95 on every route; proposed absolutes (I) ≤ 50 ms for snapshot reads, ≤ 300 ms for `/api/search`;
   CFS throttling flat. Python per route: not recorded; read it off the Grafana "p95 by route" panel first (the alert
   fires at 2 s; the fused search is ~0.1 s on the pod).
3. **The cold cloud map:** `/api/map/cloud` with the cache empty (PCA + k-means over 1,576 bits). Python: ~4 s, then
   a 5-minute cache. That screen is what "smooth" means on the phone and the absolutes above exclude it. Target (I):
   under 1 s cold, served from the per-`gen` cache after. The embed sweeper's "anything new within a minute" is the
   second SLA carried over.
4. **Write-to-push:** from a decision `POST` to the `/api/live` `change` on a second socket, p95 over 100 writes, via
   `cargo xtask live-probe`. Python has a 150 ms flush floor in its LiveHub (C), so measure it first; target (I)
   ≤ 250 ms.

## 9. Operations the port must not disturb **[review]**

- No Keycloak realm change: the five clients, the in-cluster JWKS URL and the audience scope stay as they are (a realm
  change is a shared-Keycloak outage needing the owner's go).
- Secret names unchanged; `/api/health` keeps `flags` and `auth`; the Service labels and port names that give
  Prometheus its `up{job}` stay.
- Every production step is logged in the private operations repo's deploy log with the image digest and the rollback
  line (ADR 0006), the swap-day ledger reset included.
- The nightly backup (SurrealDB + the Bugsink SQLite to R2) keeps running through every step, and T10 adds the restore
  drill before the cutover.
