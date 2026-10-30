# Punchable: the target architecture (2026-10-29)

The one document that says how Punchable is laid out and why, for the whole system: the app (TypeScript), the services
(Rust), the database, sign-in, and the conventions a contributor follows. `docs/RUST.md` is the port plan under it,
`docs/RUST_WORKSTREAM.md` the board, `docs/adr/` the decisions one by one, `docs/design/ARCHITECTURE.md` the app-side
audit this builds on. Public-safe: no hostnames beyond punchable.io, no material, no other project's names.

Tags: **C** read in a file or run; **I** inferred; **U** unknown until measured. Evidence for the duplicates and gaps is
the 2026-10-29 repo audit (summarised in §6).

## 0. The system in one picture

```
iPhone (Expo RN, TestFlight)  ─┐                          ┌─ Keycloak (realm punchable; pk-web, pk-mobile, pk-api, pk-worker, pk-gsync)
Web (react-native-web)        ─┼─ HTTPS ─► pk-api (axum) ─┤
                               │     ▲        │  LIVE      └─ SurrealDB (the source of truth; fn::apply_decision; creative_event; srcfile)
pk-worker (claim/run/result) ──┘     │        │
        │                            │        └─ in-process: bge-small query embedding (ort), PCA + k-means for the map
        └─ Anthropic Messages API    └─ /metrics, crash reports → Prometheus / Grafana / Bugsink (self-hosted)
```

Three deployables, one cluster namespace, one owner. The worker speaks HTTP to the API only, never to the database
(ADR 0011). The transcriber kinds (`process_set`, `import_youtube`) stay off the server (ADR 0012).

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
| `pk-wire` | the typed HTTP contract: the worker protocol (claim, job, result, files, decisions, creative lines, events, ideas) and the 58 app-facing response types now hand-written in `app/src/lib/{api,creative,simple}.ts`. Open KB records stay `#[serde(flatten)] extra`. After T9 it generates the app's TypeScript types with `ts-rs` | behaviour, errors, internal deps |
| `pk-kb` | the graph with no IO: typed ids (`edge_id`, `Actor`), `Decision`, replay, merge resolution, the read index, the decision builder, `Rating` | |
| `pk-creative` | rules with no IO: store reducers, the verbatim gate (one copy), `ask` (retrieve / prompt / finish), `words` (one tokeniser, named stop-word sets), `display` (`mmss`, `month_year`, `short_name_of`, `venue_short`), `voice`, the writing and merge prompt halves | |
| `pk-store` | SurrealDB through the official crate: typed rows, `kbmap`, the decision-append transaction, the `creative_event` mirror, LIVE | |
| `pk-llm` | the Anthropic Messages client, the TOML router, schema validation, prompt caching, the USD budget | |
| `pk-embed` | bge-small over `ort` + `tokenizers`, CLS pooling, the hash embedder for tests | |
| `pk-obs` | tracing init, the metrics registry (names identical to Python's), sentry with the one scrubber | |
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
             auth/{verifier,principal,actor}.rs   snapshot.rs  embed.rs  live.rs
             routes/{health,tonight,bits,sets,search,creative,ask,merge,say,ingest,memos,profile,suggest,cloud,worker,metrics}.rs
```

- **One route module per Python route module**, so T9's Ingress split is one path prefix per module. Routes depend on
  `AppState`, never on each other.
- **Auth cannot be forgotten:** the worker router is nested under `/api/worker` with `require_worker` as a route layer,
  the app router carries `require_owner`; `/api/health` and `/api/live` sit outside both. `Owner` / `Worker` extractors
  hand handlers a typed principal; `Actor::for_write` is a function (the body is consumed once). The five refusal
  texts and statuses are kept exactly (`server.py:1123-1147`, C). Auth off is refused unless the bind address is
  loopback.
- **Errors:** `ApiError` renders `{"error": msg}` with `Cache-Control: no-store`; a 401 carries exactly
  `WWW-Authenticate: Bearer realm="punchable", error="invalid_token"`; axum's body rejections are mapped to the Python
  texts (`body too large`, `body is not JSON`, `body must be an object`); `KbError::Refused` maps to 400/409/503;
  anything else is a fixed 500 and goes to sentry through the scrubber.
- **The snapshot:** `tokio::sync::watch<Arc<Snapshot>>`. One task owns the LIVE SELECT on `meta:state` and the watched
  tables; on a `gen` change it debounces, rebuilds in `spawn_blocking`, sends. The same channel feeds `/api/live`
  (`{"type":"change","rev":"db:<gen>"}`, the Python shape) and `rev` is the `ETag` (304 on `If-None-Match`): swap,
  push and cache check are one mechanism.
- **Embeddings in-process, nothing else:** the query embedding runs behind `Semaphore(1)` + `spawn_blocking`; KNN,
  BM25 and RRF run inside SurrealDB; PCA and k-means use the same permit, cached per `gen`. No LLM in the API.
- **Sized for 1 CPU:** tokio `worker_threads(2)`, `max_blocking_threads(4)`; ort `intra_threads(1)`,
  `inter_threads(1)`, arena off, memory pattern off; `tokenizers` parallelism off (rayon is in the tree through
  `tokenizers 0.22.1`, C). Gate: thread count after the first search ≤ 2 + 4 + the runtime's own.
- **Edges:** `CorsLayer` reproducing `origin_ok`; `CompressionLayer` on JSON only (never audio ranges or uploads);
  uploads streamed to disk, never buffered; `/metrics` open on the main router with the `process` feature so RSS is
  scrapable without a shell; SIGTERM flips readiness to 503, waits 10 s, then drains (this replaces the manifest's
  `preStop: sleep 10`, which the distroless image cannot run).

**Memory budget against 200 MiB warm (all I, to be measured):** binary + runtime + clients 25 MiB; bge-small fp32 in
ORT + tokenizer 135 MiB (the model file is 126.9 MiB, C); snapshot ×2 during a swap 30 MiB (U); buffers and caches
10 MiB. No headroom. The three likeliest overruns and how each is measured: ORT's own allocations (a `pk-embed` example
printing `Anonymous:` from `/proc/self/smaps_rollup` after one and after 1,000 embeds, run in the same image on the
server); the double snapshot plus allocator fragmentation under a write burst (cgroup `memory.stat` `anon` before,
during, after 100 decision writes); buffered bodies or an uncapped blocking pool (upload 200 MB, `anon` moves by MiB
not hundreds). The target is restated in cgroup `anon` (the baseline's unit) and marked at risk until measured; if the
model number lands near 250 MiB the honest target is 256 MiB with the request raised, still far under Python's
371 MiB idle.

## 3. pk-worker

One current-thread runtime; one job at a time (`Recreate` guarantees one worker). The loop is the Python's
(`worker/__main__.py:446-472`): heartbeat → budget check (pause with a ≤ 300-char reason) → `POST /api/worker/claim`
with a `pk_wire::Claim` → idle 15 s on nothing → run → post a `pk_wire::JobResult` → heartbeat; on a loop error back
off 5 s to 300 s doubling and report the crash. `api::Client` is the one factory (client credentials as `pk-worker`,
token cached until 30 s before expiry, dropped on 401, HTTP only).

Kinds are a trait with static dispatch (no `dyn`): one `dispatch(kind, …)` match names every hosted kind, so a new kind
is a compile error until handled. Hosted: bridge, punchup, facts, riff, ask_chat, set_text; embed, memolink, ask when
bge is in the image. Not ported: process_set, import_youtube (ADR 0012). `set_text`'s split step from
`engine/inbox.py` is U and must be traced before that kind is written.

The workspace is typed and in memory (a `pk_kb::View`, a `pk_creative::Store`, a sha-keyed srcfile listing), not a
file tree copied to disk per job (I: lane G confirms kind by kind). New behaviour: `pk-worker probe --max-age 20m`
replaces the manifest's `sh -c find` liveness probe (distroless has no shell, C for the manifest); SIGTERM mid-job
posts `{ok:false, retry:true, error:"the worker was stopped; the job goes back to the queue"}` within the 60 s grace;
budget hit mid-job posts a retry and no crash report; `:9100` is a two-route axum listener.

## 4. One home per rule

How a rule that must exist in both Rust and TypeScript stays one rule: a `contracts/` folder at the repo root holds
JSON case files (input → expected). `cargo xtask contracts` writes them from the Rust home, CI fails on drift (the
`goldens-current` pattern), and the app's node tests assert the TS implementation over the same cases.

| Rule | Single home | Python copy dies at | App side |
|---|---|---|---|
| Verbatim gate, inline-quote stripping | `pk-creative::verbatim` (askchat's variant) | `engine/verify.py` at T8; `creative/askchat.py` at T11 | — |
| Decision replay | SurQL `fn::apply_decision` is the hosted truth; `pk-kb::view` in memory; both kept, the three-way test stays in CI permanently (ADR 0009) | `surreal_kb.py` overrides at T10; `kb.py` stays (PC tool, oracle) | — |
| Metrics, `build_info`, label sanitisers | `pk-obs::{metrics,labels}` | `worker/metrics.py` T8; `engine/prom.py`, `api-dev/metrics.py` T11 | — |
| Crash scrubber | `pk-obs::scrub`, the one `before_send`; adopt the app's four extra keys | `worker/crashreport.py` T8; `api-dev/{scrub,crash}.py` T11 | `crash.ts` stays, pinned by `contracts/sensitive_keys.json`; fold `crashlog.ts` into it |
| `edge_id`, `Actor`/`ACTOR_RE`, `Rating` | `pk-kb::ids`, `pk-kb::model::Rating` (a test parses the `schema.surql` ASSERT list against the enum) | engine/infer/api copies T8/T11; `server.py` actor rules T10 | the server sends `by_kind: owner\|import\|ai`; delete the prefix test in `voiceProfile.ts`; `laughs.ts` keeps its button list, pinned by `contracts/ratings.json` |
| `now_iso`, `OWNER_ID`, `COMIC`, bge dir, `secrets.env` | `pk-core::{time,config}`; `pk-creative::voice`; `pk_embed::bge_dir_in` | worker copies T8; API copies T11; PC tools keep theirs | — |
| `_send`, `HttpError`, `ID_RE`, `_ts` ×3, `store(cr,kb)` ×4 | `pk-api::error`, `NodeId` path extractor, `pk-core::time`, `AppState` | T10 | — |
| Tokenisers ×6, stop-word lists ×7 | `pk-creative::words`: one tokeniser, named sets per purpose, merged only where goldens agree | T8 / T11 | `sayRules.ts` FILLERS stays, pinned by `contracts/say_fillers.json` |
| `mmss`, `month_year`, `short_name_of`, `venue_short` | `pk-creative::display` (T4's `ask/words.rs` copies move there after T4) | T11 | **the server sends `short_name`, `venue_short` (+ `venue_guessed`), `when`, `at` on list rows**; the app keeps one `format.ts` for clocks it runs itself |
| `venue_short` (the two rules differ) | the owner picks; default the server's (`creative/data.py:56-70`) | | |
| `isQuestion` (two rules in the app) | app only: `sayRules.ts` | | delete `simple.ts:76` |
| `QUERY_PREFIX`, bge dir | `pk_embed` | `db/embedder.py` T8; `engine/embed.py`, `search_api.py` T11 | — |
| Worker `Client(...)` ×4 | `pk-worker::api::Client` | T8 | — |
| Line identity / hash | `pk-core::canon` + `pk-store::kbmap::line_hash`, one-time re-hash (RUST.md fact 3) | worker `h()` T8; `creative_api.py` T10 | — |

**Stays in TypeScript** (pure UI rules tested under node): `layoutRules`, `routeTable`, `recorderLogic`, `uploadsCore`,
`retry`, `flags`, `session`, the auth token machine, `galaxyLayout` / `cloudMath`, `searchRows` (row assembly once the
server sends `at`), `sayRules`, `crash.ts`'s `routePattern`.

**Sequencing:** the Python API gains `short_name`, `venue_short`, `when`, `at`, `by_kind` first (small, the rules'
home today is `creative/`), so the app stops re-implementing server rules now and T9 carries the fields without an
allow-list entry.

## 5. Conventions

- **Errors:** one `thiserror` enum per library (`KbError`, `CreativeError`, `StoreError`, `LlmError`, `EmbedError`,
  `ObsError`, `ConfigError`); `anyhow` only in `main.rs` and xtask. Every `#[error]` text the app can show is part of
  the contract and changes only with its golden or contract file in the same commit.
- **Config:** only binaries read the environment; each crate exposes `XConfig::from_env(&Env)` and a plain constructor
  for tests.
- **Tracing:** one request span with `method`, `route` (axum's `MatchedPath`, never the raw path: ids are slugs of his
  titles), `status`, `ms`; worker spans carry `job_id`, `kind`; never log bodies, query strings, titles or ids. Gate: a
  capture-layer test sends a sentinel in path, body and query and asserts it appears in no event.
- **Metrics:** `pk_` prefix, `_total` / `_seconds`; label values only from `pk_obs::labels`; names identical to
  Python's until T11 so dashboards and alerts do not move.
- **Testing per crate until T11:** goldens from the Python oracle (compared as parsed JSON), proptest where equality
  cannot hold, `wiremock` for Anthropic and JWKS, the scratch SurrealDB for pk-store and pk-api, `cargo-mutants` on
  the replay, the gate and auth (zero surviving mutants or each listed with a reason). **A skipped test is a failed
  test:** CI sets `PK_REQUIRE_EXTERNAL=1` with `surreal` and the bge model installed, so the pk-store and pk-embed
  suites that return early without them (C, `pk-store/tests/scratch.rs`, `pk-embed/tests/goldens.rs`) count as run.
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

## 7. The app (TypeScript stays, ADR 0003)

Adopt the layout in `docs/design/ARCHITECTURE.md` §1 (`src/app` → `src/features/*` → `src/ui`, `src/api`,
`src/auth`, `src/platform`; no imports between features; a `static.layers` test) with five amendments, structure only
(visual values are the design expert's lane):

1. **`src/rules/` as the bottom layer:** pure modules tested under node, no React imports, imports with explicit `.ts`
   extensions (`allowImportingTsExtensions` in tsconfig; the node runner cannot resolve extension-less imports, which
   is why `searchRows.ts` copies `mmss` today, C). Holds `format.ts` (one `mmss`, `monthYear`, `fmtDate`, `MONTHS`,
   replacing four formatters and six month arrays), `sayRules`, the rules half of `laughs`, `layoutRules`,
   `routeTable`, `retry`, `flags`, `searchRows`, `cloudMath`, `bitWorkRules`, `recorderLogic`, `uploadsCore`.
2. **One API client:** `src/api/client.ts` exports `authHeaders()`; `uploads.native.ts` uses it instead of its own
   Bearer header (C); the two poll loops (`ask.tsx`, `simple.draft`) become one `poll()`; after T9 `src/api/wire.gen.ts`
   from `pk-wire` replaces the hand-written shapes (and ends the `Step` name collision).
3. **Auth:** `lib/tokens.ts` (the token state machine) becomes `src/auth/tokenMachine.ts` so `src/ui/tokens.ts` can
   mean design tokens; the code-exchange effect duplicated in `login.tsx` and `settings.tsx` becomes one
   `useCodeExchange`.
4. **Kit:** `src/ui/tokens.ts` with colour, spacing, radius and type scales (today 18 distinct radii, 24 spacings, the
   type scale only inside `ui.tsx`); `Input` replaces the 19 copied input-style blocks; one `RenameSheet` replaces
   three `RenameForm`s plus `ShortNameForm`/`TitleForm`; `GoogleG` moves to `src/ui/brand`. Gate: a static test fails
   on a number literal for `borderRadius`, `padding*`, `fontSize` outside `src/ui`, with a baseline list that only
   shrinks.
5. **Twins and storage:** `Screen` and `DragList` get the `XBase` + shim + `.native` pattern (ENGINEERING.md §1); the
   eleven hand-rolled `localStorage` sites go behind `src/platform/kv.ts`.

Order: rules layer (with `format.ts` and the one `isQuestion`); the upload header; the auth rename; `Input` and the
token structure; the folder move last. Each step's gate: `typecheck`, `test:unit`, `lint`, the web and iOS exports.

## 8. Order from here, and what "optimized and smooth" means

| # | Step | Gate | Ships? |
|---|---|---|---|
| 1 | T4 ask (done 2026-10-29, `52df9e3`) | the `ask_*` goldens | no |
| 2 | Honest CI (§6) | green with the scratch-DB and model tests counted as run | CI only |
| 3 | T12 `pk-core`; delete the copies in kb, creative, llm, obs, store; libraries stop reading env | every golden unchanged + a new float-in-refusal case | no |
| 4 | T13 `pk-wire`, the worker protocol first | the Python API's worker-route responses round-trip equal as values | no |
| 5 | Measure `pk-embed` memory in the distroless base on the server | a number in RUST.md with the machine named | decides the target |
| 6 | T8 worker + `probe` + the liveness change + the SIGTERM handback, one commit | offline differential vs the Python worker; in prod the failed-jobs rate no worse than Python's week before | **the first shipping artefact**; rollback = previous image digest |
| 7 | The Python API sends `short_name`, `venue_short`, `when`, `at`, `by_kind`; the app stops computing them | app tests over `contracts/*.json`; pytest | app + Python API |
| 8 | T9 reads, one route group at a time behind Ingress paths; `/metrics`; ETag = `rev` | the baseline GET generator over both base URLs, equal as values with the allow-list in the repo | per group; rollback = remove the path |
| 9 | T10 writes + `/api/worker/*` + `/api/live` together, with the canonical line-hash migration | the POST sequence on scratch DBs equal as values | cannot be split |
| 10 | T11 retire Python; `dbload` on its own image; the oracle tag; goldens → fixtures with a manifest | the same gates on the shipped images | after 9 has run in prod |

The app lanes (§7) run alongside; they only touch `app/`.

Three measurements, each taken on Python first the same way (T7 made the metric names identical):

1. **Memory:** cgroup `anon` of the API pod warm and idle, and after the load script in `infra/README.md` "Memory"
   (`memory.stat` on the node, or `process_resident_memory_bytes{job="punchable-api"}` once the `process` feature is
   on). Target ≤ 200 MiB warm, or what step 5 justifies. Python: 371 MiB idle.
2. **Latency:** p95 per route group under the same script,
   `histogram_quantile(0.95, sum by (le, route) (rate(pk_http_request_duration_seconds_bucket[5m])))`. Target: Rust
   p95 ≤ Python p95 on every route; proposed absolutes (I) ≤ 50 ms for snapshot reads, ≤ 300 ms for `/api/search`;
   CFS throttling flat.
3. **Write-to-push:** from a decision `POST` to the `/api/live` `change` on a second socket, p95 over 100 writes, via
   `cargo xtask live-probe`. Target (I) ≤ 250 ms; the Python figure is U, so measure it first.
