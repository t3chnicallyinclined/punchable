# Punchable services in Rust: the port plan (2026-10-29)

Standing owner decision (2026-10-29): **the services are Rust from here.** The Python API, worker and engine are
what the project was built in; nothing recorded chose Python over Rust. New services go in Rust, the existing ones
are ported in the order below, and the phone/web app stays TypeScript (React Native). Public-safe: no hostnames, no
material, no other project's names.

Tags: **[C]** read in a file or run; **[I]** inferred; **[U]** unknown until measured.

## Why, honestly

The API serves one person and waits on Claude and on SurrealDB, so raw speed is not the reason. The reasons are:
one toolchain across the owner's projects, a single static-ish binary, memory that can be reasoned about (the API
pod hit 824 MiB of a 1 GiB limit on 2026-10-28 because of onnxruntime's arena in Python), and the cycles in the
Python that a crate graph cannot have. Rust does not go on the phone: iOS apps are Swift or React Native; a Rust
core (UniFFI) earns its place only when a feature needs it (on-device transcription, laugh detection).

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
worker token only on `/api/worker/*`.

Six facts that shape the port [C]:

1. The quote gate exists twice (`engine/verify.py:46` and `creative/askchat.py:86`) because the API image ships
   `creative/` but not `engine/`. Rust gets one.
2. Decision replay exists twice: Python `kb.py:140 _apply` and SurQL `fn::apply_decision`. `db/tests/test_baseline.py`
   is the existing differential. The SurQL function stays the hosted truth; the Rust replay is gated against both.
3. **How Python writes JSON is part of the contract.** Event lines are compared byte for byte and hashed
   (`creative_api.py:394-404`, `worker/__main__.py:130-133,209-232`); Python's float repr (`1e-05`, `1e+16`) is not
   serde's. A `pk-py` crate reproduces `json.dumps` (default, compact, `sort_keys`, `ensure_ascii=False`).
4. Python's `str.isspace` includes U+001C–U+001F; Rust's `char::is_whitespace` does not. The verbatim normaliser
   branches on it (`verify.py:34`).
5. SurrealDB `NONE` and `NULL` are distinct here (`schema.surql:13-14`, `kbmap.py strip_none`); the CBOR codec must
   keep them apart.
6. API writes cannot be split between two processes: after a Rust write the Python pod's jsonl cache lags and every
   Python write 409s (`surreal_kb.py:322-324`). All write routes cut over in one step.

## Workspace (`rust/` at the repo root)

| Crate | Hides | Depends on | Python cycle it breaks |
|---|---|---|---|
| `pk-py` | Python semantics in one place: `json.dumps` forms, float repr, `round`, `isspace`/`strip`/`splitlines`, `now_iso` (duplicated today at `kb.py:42` and `util.py:15`) | serde_json | |
| `pk-kb` | the graph with no IO: typed ids, `Decision`, replay, merge resolution, the read index, decision validation (`kb.py:677`), the legal rules (`gsync/rules.py`) | pk-py | engine.kbstore → kb.py; db/load → ai_overlay |
| `pk-creative` | the rules with no IO: store reducers (`job_apply` and friends), Store replay over lines, verbatim, askchat retrieve / prompt / finish, story / workingset / write / merge / memolink split into a prompt half and a finish half, `ntok`; job wire types | pk-kb | creative → engine.providers (the binaries run prompt, then the model, then finish) |
| `pk-store` | SurrealDB: the CBOR codec (NONE ≠ NULL), HTTP `/rpc` over a reqwest pool, one websocket for LIVE, the guards (EmptyWrite, changefeeds, version), kbmap, the decision-append transaction (`surreal_kb.py:330-337`), the creative_event mirror, srcfile. The schema is `include_str!("../../db/schema.surql")` | pk-kb | |
| `pk-llm` | Anthropic Messages over raw HTTP (no official Rust SDK), blocking: request builder (`providers.py:274`), a port of `transform_schema`, the TOML router with fallbacks, JSON-schema validation, the budget | pk-py | |
| `pk-embed` | bge-small, CLS pooling, over `ort` + `tokenizers`; the hash embedder for tests; sync | | |
| `pk-obs` | tracing to stderr, the Prometheus text registry (port of `engine/prom.py`), sentry to Bugsink with the scrubber | | |
| `pk-api` (lib + bin) | axum 0.8: routes, JWKS auth, actor rules, CORS, `/api/live`; routes depend on `AppState`, never on each other | kb, creative, store, embed, obs | the api-dev hub |
| `pk-worker` (bin) | the claim / run / result loop over HTTP to the API only, as today | kb, creative, llm, embed, obs | worker ↔ gsync |
| `xtask` | the gates and the differential runners | | |

A DAG. `pk-api` never depends on `pk-llm` (the hosted API holds no model). The binaries never depend on each other.
Only store, llm, embed and obs do IO.

**SurrealDB client: hand-rolled CBOR over HTTP `/rpc`, not the official crate** [I]. The owner already measured the
JSON-RPC trap (a string that looks like a record id becomes a record) and built this path in another project; HTTP
`/rpc` is stateless so reqwest's pool is the pool; LIVE needs exactly one websocket. The official crate would put a
second value model between us and the types that are the contract. Multi-statement `BEGIN … COMMIT` over HTTP `/rpc`
is the first pk-store test.

**Embeddings stay in-process** in the API: search embeds the query before the KNN (`search_api.py:197-223`) and a
trip through the job queue is too slow for search.

## Errors and wire shapes (the owner's conventions)

- Libraries: one `thiserror` 2 enum each. Binaries: `anyhow::Result` with `.context()`.
  `pk-kb::KbError::Refused { kind: BadRequest | Conflict | Unavailable, msg }` mirrors `DecisionError(msg, status)`;
  `pk-llm::LlmError` has `Refusal`, `CutShort`, `RateLimited`, `Budget`, `NoProvider`, `Schema`, with the plain-words
  texts copied from `providers.py:56-77`; `pk-api::ApiError` is `IntoResponse` as `{"error": msg}`, and a 401 carries
  the exact `WWW-Authenticate` header (`server.py:1098`).
- **Every refusal string is contract**: 39 `str(e)` sites in api-dev and creative. Each becomes a fixed message or is
  reclassified. 500s are compared by status only (they leak Python class names today).
- Typed ids: `NodeId` parsed from `joke: | unit: | set: | perf: | label: | list:` (`perf` → table `performance`,
  colons kept in the key); `EdgeId` = `"e:"` + 12 hex of sha1 (`kb.py:46-47`); `Seq`, `JobId`;
  `Actor::{Owner, Import(s), Ai(s)}` validated by `ACTOR_RE` (`server.py:61`).
- Wire shapes: KB nodes and edges stay `serde_json::Map` newtypes with typed accessors (the replay is a dict merge,
  `kb.py:172`; a fully typed struct would drop fields). Closed shapes become structs: the decision line
  (`DECISION_KEYS` + `extra`), the creative event `{ts, op, id, by, data}`, the job record, the claim body, health.
  `Option` serialises as `null`, never skipped, unless Python omits the key. `serde_json` with `preserve_order`, never
  `arbitrary_precision`. The React Native app's 48 exported API types (`app/src/lib/api.ts`, `creative.ts`) are the
  acceptance test: the app changes nothing.

## Async vs blocking

- `pk-api`: axum + tokio, justified by the long-lived `/api/live` sockets and the LIVE websocket. Requests read an
  immutable `Arc` snapshot swapped when `meta:state.gen` changes; the rebuild runs in `spawn_blocking`. ONNX query
  embedding runs in `spawn_blocking` behind a one-permit semaphore, one intra-op thread, no arena. The cloud's SVD
  and k-means run in `spawn_blocking`, cached per gen. **No rayon**: the pod limit is 1 CPU (`30-api.yaml`).
- `pk-worker`: fully blocking (ureq, one thread). It runs one job at a time today; the Anthropic SSE stream is read
  from a blocking reader. So `pk-llm` and `pk-embed` are sync; only `pk-store` (API only) is async.

## Migration order, and the proof at each step

| # | Step | Gate | Red arm (the gate must be able to fail) |
|---|---|---|---|
| 0 | Baseline | `db/tests/test_baseline.py` green and recorded; Python RSS and image sizes measured on the server under the load in `infra/README.md` "Memory" | mutate one branch of `fn::apply_decision` on a scratch DB: baseline fails |
| 1 | `pk-py`, `pk-kb`, `pk-creative` | CPython-generated goldens; a three-way replay (Python View, SurQL on a scratch 3.3.4, Rust) compared byte for byte through `pk-py` | swap in `char::is_whitespace`: the goldens fail; cargo-mutants on the replay kills its mutants |
| 2 | `pk-llm`, `pk-embed` | request-body goldens per task dumped from the pinned Python SDK (`anthropic==0.104.1`); token ids exactly equal; vectors at cosine ≥ 0.99999 plus the `fetch_bge.py` fingerprint | drop one `cache_control`, or pool by mean instead of CLS: fails |
| 3 | **Worker** | offline: both workers against the same fixture API and a canned Anthropic (wiremock); outgoing request bodies and posted results equal with ts/ids normalised. Prod: an image swap, then a week of `pk_worker` done/failed metrics | a corrupt canned answer fails the same way in both |
| 4 | API reads, route group by route group | the baseline's GET generator over two base URLs on temp copies of real data; bytes compare through `pk-py`; more specific Ingress paths send each group to `pk-api` | rename one field in one Rust route: fails |
| 5 | **All writes, `/api/worker/*` and `/api/live` together** | the same POST sequence to both on scratch DBs: responses, post-write reads, `decision` rows and `creative_event.line` all equal | have Rust write compact JSON: the line compare fails |
| 6 | Retire Python | the same gates re-run against the shipped images; `dbload` gets its own image first (it runs on the API image today) | |

Constraints: the worker cutover is a swap, not a per-kind split (`35-worker.yaml` forbids two workers; the day's spend
ledger is per pod). Route groups must not split in-memory state (`worker_seen` in `creative_api.py:362` is why the
worker routes move in step 5).

**What cannot be byte-identical, and the test instead**: model output (pin the request bytes and the transformed
schema; golden the post-processing over canned answers); the verbatim gate (property tests: anything returned is a
substring of the source, a 3+ char substring returns itself, typographic perturbation maps back, a changed word
returns None; plus a 100k-case corpus run through both Python copies, covering U+001C–U+001F, NBSP, `…`, astral chars;
Python indexes by code point and Rust by byte); embeddings (the `ort` crate's bundled runtime may differ from 1.27 [U]:
tolerance plus top-k neighbour overlap); the cloud map (numpy's RNG is not reproducible: property gates and the
owner's sign-off on the layout change); label tie order (depends on PYTHONHASHSEED today: Rust defines an order, the
harness compares those fields as multisets).

## Testing

Pure crates: unit tests plus CPython-made goldens; real-data goldens stay outside the public repo (an env path);
committed fixtures are synthetic, one per `_apply` branch. `pk-store`: a scratch `surreal start memory` pinned to
3.3.4; traps T1–T6 (a record-looking string stays a string; an EDITOR's denied write raises EmptyWrite; changefeeds
survive). `pk-llm`: wiremock with recorded SSE for normal, refusal, max_tokens, 429 and cache-usage; the budget ledger
is golden. `pk-api`: `tower::ServiceExt::oneshot` over fixtures; the auth tests reuse `api-dev/tests/jwt_kit.py`'s
token set (red arms: `alg: none`, HS256 signed with the public key); response fixtures type-checked against the app's
TypeScript types. Observability: `/metrics` names, labels and types equal Python's after the same script; a scrubber
property test (red arm: bypass the scrubber).

## Build, CI, images

Edition 2024, pinned `rust-version`, `unsafe_code = "forbid"`, rustfmt `max_width = 120`, `clippy -D warnings`,
`cargo deny`; release `lto = "thin"`, `codegen-units = 1`, `strip = true`. Gates per crate (`cargo test -p`), never
`--workspace`. CI: `.github/workflows/rust.yml` next to `api.yml`; the differential jobs need both languages until
step 6. Images: `gcr.io/distroless/cc-debian12`, not static (onnxruntime needs glibc and libstdc++ [I]); keep the
hash-checked model stage from `tools/kb/fetch_bge.py`; the k8s securityContext unchanged. Targets to measure on the
server [I]: API image ≤ 250 MB; RSS warm ≤ 200 MiB with the model loaded (Python today: 450 MiB idle, ~560 MiB in a
sweep, measured 2026-10-28).

## Not ported; stays as is

Not ported: `tools/audio` (whisper, VAD, laugh detection); `gsync` except `rules.py` (→ `pk-kb`); the transcriber
kinds `process_set` / `import_youtube`; the ClaudeCode and OpenAI-compatible backends (the worker image asserts
`providers == {'anthropic'}`); `tools/explorer/server.py`; `infer/*` except `ntok`; the files backend (it stays as the
Python oracle in the harness only); `db/load.py` and `push_remote.py` stay Python until a later ticket. Whatever
`set_text`'s split step needs from `engine/inbox.py` is [U] and must be traced before step 3.

Stays TypeScript: `app/` (Expo React Native) and `infra/web`.

## First ticket

Step 0 and step 1: `rust/Cargo.toml`, `pk-py` with its `json.dumps` and `isspace` goldens generated by CPython,
`pk-kb` with the replay and a three-way differential against `kb.py` and `fn::apply_decision` on a scratch SurrealDB.
Nothing ships until step 3.
