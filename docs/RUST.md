# Punchable services in Rust: the port plan (2026-10-29, revised the same day)

Standing owner decision (2026-10-29): **the services are Rust from here.** The Python API, worker and engine are
what the project was built in; nothing recorded chose Python over Rust. New services go in Rust, the existing ones
are ported in the order below, and the phone/web app stays TypeScript (React Native). Public-safe: no hostnames, no
material, no other project's names.

Owner's rule for the port (2026-10-29): **do not reinvent the wheel; use the popular, maintained crates.** The first
draft proposed a hand-rolled SurrealDB client, a crate reproducing Python's `json.dumps`, and ports of the Python
metrics registry and JWT check. All of that is gone from this version; where the Python has a quirk, the plan
relaxes the compatibility requirement rather than reproducing the quirk.

Tags: **[C]** read in a file or run; **[I]** inferred; **[U]** unknown until measured.

## Why, honestly

The API serves one person and waits on Claude and on SurrealDB, so raw speed is not the reason. The reasons are:
one toolchain across the owner's projects, one binary per service, memory that can be reasoned about (the API pod hit
824 MiB of a 1 GiB limit on 2026-10-28 because of onnxruntime's arena in Python), and a crate graph that cannot have
the import cycles the Python has. Rust does not go on the phone: iOS apps are Swift or React Native; a Rust core
(UniFFI) earns its place only when a feature needs it (on-device transcription, laugh detection).

## What exists today (Python, hosted runtime only)

| Area | Lines | What it is |
|---|---|---|
| `api-dev/` | ~8,200 | a standard-library HTTP handler (`server.py`, 2,265 lines) plus route modules that import each other through `creative_api.py` as a hub; a hand-written WebSocket for `/api/live`; RS256 JWT against Keycloak JWKS; Prometheus text metrics; crash reports with a scrubber |
| `creative/` | ~6,200 | the domain: a read-only Corpus over the graph + decision replay, event-log stores, the askchat retrieve → prompt → verbatim-gate pipeline, writing (riff, punch-up, bridge, facts), merge judge, memolink |
| `engine/` | ~3,000 | the Anthropic provider router (TOML config, JSON-schema validation, retry, fallback, prompt caching, daily USD budget), the bge-small ONNX embedder, the metrics registry, the verbatim check |
| `worker/` | ~1,400 | claim → run → post a result, over the API only (never the database); hosted kinds bridge, punchup, facts, riff, ask_chat, set_text, embed, memolink, ask |
| `db/` | ~1,650 | CBOR-over-websocket SurrealDB RPC, the exact KB ↔ row mapping, the schema (`fn::apply_decision` applies a decision row to the tables) |
| `tools/explorer/kb.py` | 853 | the decisions.jsonl replay (View + `_apply`: add / delete / confirm / reject / edit, `undo_of`) |

Sources of truth, hosted: the SurrealDB `decision` table (the jsonl is a cache), the `creative_event{store, seq,
line}` rows (each store's jsonl is a cache), `srcfile` rows for text files. ~70 HTTP routes; owner token everywhere,
worker token only on `/api/worker/*`. About a quarter of the Python is dead or PC-only (below) and is not ported.

Facts that shape the port [C]:

1. The quote gate exists twice (`engine/verify.py:46` and `creative/askchat.py:86`) because the API image ships
   `creative/` but not `engine/`. Rust gets one.
2. Decision replay exists twice: Python `kb.py:140 _apply` and SurQL `fn::apply_decision`. `db/tests/test_baseline.py`
   is the existing differential. The SurQL function stays the hosted truth; the Rust replay is gated against both.
3. Today the Python compares creative event lines **byte for byte** and hashes them (`creative_api.py:394-404`,
   `worker/__main__.py:130-133,209-232`), and Python's float repr differs from serde's. **Decision: the Rust side does
   not reproduce `json.dumps`.** Equality becomes equality of the parsed JSON value; the line hash is recomputed over
   a canonical form that Rust defines (serde_json compact, keys sorted); a one-time migration re-hashes the existing
   rows. The byte compare was an implementation detail, not a product requirement.
4. Python's `str.isspace` includes U+001C–U+001F; Rust's `char::is_whitespace` does not (`verify.py:34` branches on
   it). Decision: the Rust gate defines whitespace as Unicode `White_Space`; the differential allows this difference.
5. SurrealDB `NONE` and `NULL` are distinct here (`schema.surql:13-14`, `kbmap.py strip_none`). The official
   `surrealdb` crate's value model keeps them distinct (`Value::None` vs `Value::Null`) [I: verify in step 1].
6. API writes cannot be split between two processes: after a Rust write the Python pod's jsonl cache lags and every
   Python write 409s (`surreal_kb.py:322-324`). All write routes cut over in one step.

## Crates we stand on (popular, maintained; the owner already uses most)

| Need | Crate | Replaces in Python |
|---|---|---|
| HTTP server | `axum` 0.8, `tower-http` (cors, trace, compression) | the std-lib handler in `server.py` |
| WebSocket for `/api/live` | `axum::extract::ws` | the hand-written upgrade in `server.py:1457` |
| HTTP client | `reqwest` (rustls, json, stream) | urllib / httpx |
| Async runtime | `tokio` (named features) | threads |
| SurrealDB | **the official `surrealdb` crate** (ws + cbor, `LIVE SELECT` support) | `db/sdb.py`, `surreal_kb.py` LiveHub |
| JWT + JWKS | `jsonwebtoken` (RS256, `jwk` module) | `api-dev/auth.py` |
| Metrics | `prometheus` (text encoder) or `metrics` + `metrics-exporter-prometheus` | `engine/prom.py` |
| Crash reports | `sentry` + `sentry-tower` (`before_send` scrubber) | `crash.py` / `scrub.py` |
| Logs | `tracing`, `tracing-subscriber` (env-filter) | print |
| JSON, schema | `serde`, `serde_json` (`preserve_order`), `jsonschema` | json, jsonschema |
| Config | `toml` + serde | tomllib |
| Embeddings | `ort` (onnxruntime), `tokenizers` | onnxruntime, tokenizers |
| Linear algebra, PCA, k-means | `ndarray`, `linfa-reduction`, `linfa-clustering` | numpy (`cloud_api.py`) |
| Text retrieval | SurrealDB's own BM25 (`@@` with the `punch_en` analyzer, `search::score`) for both the Bits search and the askchat retrieval; `bm25` crate only if a query needs to run off-DB | `creative/askchat.py:_bm25` |
| Anthropic | no official Rust SDK. `reqwest` + typed serde models for the Messages API, SSE via `reqwest-eventsource` | the `anthropic` SDK |
| Errors | `thiserror` 2 (libs), `anyhow` (bins) | exceptions |
| Hashing, time, ids | `sha2`, `hex`, `chrono`, `uuid` | hashlib, datetime |
| Tests | `wiremock` (Anthropic, JWKS), `proptest` (the gate), `insta` (goldens), `cargo-mutants` | pytest |

## Workspace (`rust/` at the repo root)

| Crate | Hides | Depends on | Python cycle it breaks |
|---|---|---|---|
| `pk-kb` | the graph with no IO: typed ids, `Decision`, replay, merge resolution, the read index, decision validation (`kb.py:677`), the legal rules (`gsync/rules.py`) | serde | engine.kbstore → kb.py; db/load → ai_overlay |
| `pk-creative` | the rules with no IO: store reducers (`job_apply` and friends), Store replay over lines, the verbatim gate, askchat prompt / finish, story / workingset / write / merge / memolink split into a prompt half and a finish half; job wire types | pk-kb | creative → engine.providers (the binaries run prompt, then the model, then finish) |
| `pk-store` | SurrealDB through the official crate: typed row models (the `kbmap.py` contract), the decision-append transaction (`surreal_kb.py:330-337`), the creative_event mirror, srcfile, LIVE. The schema is `include_str!("../../db/schema.surql")` | pk-kb, surrealdb | |
| `pk-llm` | the Anthropic Messages client (reqwest), the TOML router with fallbacks (`config.hosted.toml`), `jsonschema` validation, prompt caching on the system blocks, the daily budget | jsonschema, reqwest | |
| `pk-embed` | bge-small, CLS pooling, `ort` + `tokenizers`; a hash embedder for tests | ort, tokenizers | |
| `pk-obs` | tracing init, the metrics registry, sentry init with the scrubber | prometheus, sentry | |
| `pk-api` (lib + bin) | axum: routes, JWKS auth, actor rules, CORS, `/api/live`; routes depend on `AppState`, never on each other | kb, creative, store, embed, obs | the api-dev hub |
| `pk-worker` (bin) | the claim / run / result loop over HTTP to the API only, as today | kb, creative, llm, embed, obs | worker ↔ gsync |
| `xtask` | the gates and the differential runners | | |

A DAG. `pk-api` never depends on `pk-llm` (the hosted API holds no model). The binaries never depend on each other.

**Embeddings stay in-process** in the API: search embeds the query before the KNN (`search_api.py:197-223`) and a
trip through the job queue is too slow for search.

## Errors and wire shapes (the owner's conventions)

- Libraries: one `thiserror` 2 enum each. Binaries: `anyhow::Result` with `.context()`.
  `pk-kb::KbError::Refused { kind: BadRequest | Conflict | Unavailable, msg }` mirrors `DecisionError(msg, status)`;
  `pk-llm::LlmError` has `Refusal`, `CutShort`, `RateLimited`, `Budget`, `NoProvider`, `Schema`, with the plain-words
  texts copied from `providers.py:56-77`; `pk-api::ApiError` is `IntoResponse` as `{"error": msg}`, and a 401 carries
  the exact `WWW-Authenticate` header (`server.py:1098`).
- **The app's contract is the JSON shape, not the bytes.** The React Native app's 48 exported API types
  (`app/src/lib/api.ts`, `creative.ts`) are the acceptance test; refusal strings the app shows are kept.
- Typed ids: `NodeId` parsed from `joke: | unit: | set: | perf: | label: | list:` (`perf` → table `performance`,
  colons kept in the key); `EdgeId` = `"e:"` + 12 hex of sha1 (`kb.py:46-47`); `Seq`, `JobId`;
  `Actor::{Owner, Import(s), Ai(s)}` validated by `ACTOR_RE` (`server.py:61`).
- KB nodes and edges stay `serde_json::Map` newtypes with typed accessors (the replay is a dict merge, `kb.py:172`;
  a fully typed struct would drop fields). Closed shapes become structs: the decision line, the creative event
  `{ts, op, id, by, data}`, the job record, the claim body, health. `Option` serialises as `null` where Python
  writes `null`.

## Async vs blocking

- `pk-api`: axum + tokio (the long-lived `/api/live` sockets and the LIVE subscription). Requests read an immutable
  `Arc` snapshot swapped when `meta:state.gen` changes; the rebuild runs in `spawn_blocking`. ONNX query embedding
  runs in `spawn_blocking` behind a one-permit semaphore, one intra-op thread, no arena. PCA and k-means run in
  `spawn_blocking`, cached per gen. **No rayon**: the pod limit is 1 CPU (`30-api.yaml`).
- `pk-worker`: one tokio current-thread runtime (reqwest is async), one job at a time as today.

## Migration order, and the proof at each step

| # | Step | Gate | Red arm (the gate must be able to fail) |
|---|---|---|---|
| 0 | Baseline | `db/tests/test_baseline.py` green and recorded; Python RSS and image sizes measured on the server under the load in `infra/README.md` "Memory" | mutate one branch of `fn::apply_decision` on a scratch DB: baseline fails |
| 1 | `pk-kb`, `pk-creative`, `pk-store` | a three-way replay (Python View, SurQL on a scratch SurrealDB, Rust) compared as parsed JSON (`insta` snapshots, synthetic fixtures per `_apply` branch; real data by env path, never committed); NONE/NULL round-trip through the official crate | change one replay branch: snapshots fail; `cargo-mutants` on the replay kills its mutants |
| 2 | `pk-llm`, `pk-embed` | request bodies per task compared as parsed JSON against those dumped from the pinned Python SDK; token ids exactly equal; vectors at cosine ≥ 0.99999 plus the `fetch_bge.py` fingerprint | drop one `cache_control`, or pool by mean instead of CLS: fails |
| 3 | **Worker** | offline: both workers against the same fixture API and a `wiremock` Anthropic; outgoing requests and posted results equal as values with ts/ids normalised. Prod: an image swap, then a week of `pk_worker` done/failed metrics | a corrupt canned answer fails the same way in both |
| 4 | API reads, route group by route group | the baseline's GET generator over two base URLs on temp copies of real data, compared as values with an allow-list of known differences; more specific Ingress paths send each group to `pk-api` | rename one field in one Rust route: fails |
| 5 | **All writes, `/api/worker/*` and `/api/live` together** | the same POST sequence to both on scratch DBs: responses, post-write reads, `decision` rows and `creative_event.line` equal as values; the line-hash migration applied | drop a field from one write: fails |
| 6 | Retire Python | the same gates re-run against the shipped images; `dbload` gets its own image first (it runs on the API image today) | |

Constraints: the worker cutover is a swap, not a per-kind split (`35-worker.yaml` forbids two workers; the day's spend
ledger is per pod). Route groups must not split in-memory state (`worker_seen` in `creative_api.py:362` is why the
worker routes move in step 5).

**What is tested by property, not equality**: model output (the request is pinned; the post-processing is golden
over canned answers); the verbatim gate (`proptest`: anything returned is a substring of the source, a 3+ char
substring returns itself, typographic perturbation maps back, a changed word returns None); embeddings (the `ort`
runtime version may differ [U]: tolerance plus top-k neighbour overlap); the cloud map (numpy's RNG is not
reproducible: shape properties and the owner's sign-off); label tie order (depends on PYTHONHASHSEED today: Rust
defines an order, the harness compares as multisets).

## Build, CI, images

Edition 2024, pinned `rust-version`, `unsafe_code = "forbid"`, rustfmt `max_width = 120`, `clippy -D warnings`,
`cargo deny`; release `lto = "thin"`, `codegen-units = 1`, `strip = true`. Gates per crate (`cargo test -p`). CI:
`.github/workflows/rust.yml` next to `api.yml`; the differential jobs need both languages until step 6. Images:
`gcr.io/distroless/cc-debian12` (onnxruntime needs glibc and libstdc++ [I]); keep the hash-checked model stage from
`tools/kb/fetch_bge.py`; the k8s securityContext unchanged. Targets to measure on the server [I]: API image ≤ 250 MB;
RSS warm ≤ 200 MiB with the model loaded.

**Step 0 baseline, measured on the server 2026-10-29 (the Python images at `51bfdf1` / `3d4dce9`)** [C]:

| | API | Worker | Web |
|---|---|---|---|
| image (`docker image ls`, uncompressed) | 650 MB | 679 MB | 97 MB |
| anonymous memory, idle (cgroup `anon`) | 371 MiB (450 MiB on 2026-10-28 before a restart; ~560 MiB during an embed sweep) | 130 MiB | |

`db/tests/test_baseline.py` was not re-run on the server: the hosted pods carry no pytest and the files-backend oracle
needs the knowledge base on disk; it runs on the PC that holds the data (its last green run is in the repo's history).

## Not ported; stays as is

Not ported: `tools/audio` (whisper, VAD, laugh detection); `gsync` except `rules.py` (→ `pk-kb`); the transcriber
kinds `process_set` / `import_youtube`; the ClaudeCode and OpenAI-compatible backends (the worker image asserts
`providers == {'anthropic'}`); `tools/explorer/server.py`; `infer/*` except `ntok`; the files backend (it stays as the
Python oracle in the harness only); `db/load.py` and `push_remote.py` stay Python until a later ticket. Whatever
`set_text`'s split step needs from `engine/inbox.py` is [U] and must be traced before step 3.

Stays TypeScript: `app/` (Expo React Native) and `infra/web`.

## Progress

| Date | What landed |
|---|---|
| 2026-10-29 | `rust/` workspace (nine crates as above, `cargo xtask gate`, `.github/workflows/rust.yml`). **`pk-kb` built**: typed ids, `Decision`, the replay (`_apply`, merge resolution, the index), the read models (brief, edge view, summaries, search, joke detail, label index, actors) and the decision builder with undo. Proven against the Python by goldens: `rust/tools/gen_kb_goldens.py` replays a synthetic graph with every `_apply` branch, 52 builder requests (14 accepted, 38 refused with the exact message and status) and every undo through `kb.py`; the Rust tests compare as parsed JSON and pass. CI regenerates the goldens and fails on drift. Not yet in `pk-kb`: the Desk explorer's `queue`, `neighborhood`, `stats` (PC-only screens), the legal rules. |

## Next ticket

Step 1 continued: `pk-creative` (the store reducers over the creative event logs, the verbatim gate with property
tests, askchat prompt/finish) and `pk-store` (the official crate against a scratch SurrealDB: NONE/NULL round-trip,
the decision-append transaction, the `creative_event` mirror). Then step 0's server measurement and step 2.
