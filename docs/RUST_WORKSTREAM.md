# The Rust port: workstream (2026-10-29)

The plan is `RUST.md`; this is the board. One row per ticket, in lanes that run in parallel because they touch
different crates. Status is edited here as tickets land; the gate for every ticket is `cd rust && cargo xtask gate`
plus the ticket's own proof. Nothing ships until T8 (the worker swap).

**Rules every lane follows**
- Popular crates, no hand-rolling (the owner's rule, 2026-10-29). Pin versions in `rust/Cargo.toml`
  `[workspace.dependencies]`; a crate says `{ workspace = true }`.
- A lane owns its crate directory and its generator under `rust/tools/`. It does not edit another lane's crate.
  `rust/Cargo.toml` and `Cargo.lock` are shared: add your lines, never reorder or remove others'.
- Edition 2024, `unsafe` forbidden, `thiserror` in libraries, `anyhow` in binaries, clippy pedantic clean, rustfmt 120,
  `//!` module docs that say what the module is for (not how), identifiers in backticks in docs.
- Proof against the Python it replaces: a CPython generator writes goldens from synthetic data (never real material)
  into `rust/crates/<crate>/tests/goldens/`; the Rust test compares as parsed JSON (`pk-kb/tests/goldens.rs` is the
  pattern). Property tests (`proptest`) where equality cannot hold.
- Nothing private in the public repo: no hostnames, no material, no other project's names. The commit guard runs.
- Edition 2024 traps met so far: `gen` is a reserved word (write `r#gen` or rename); `ndarray`'s `s!` macro expands
  with `#[allow(unsafe_code)]` and is refused under `unsafe_code = "forbid"` (index axes instead).
- A Python behaviour that the design said to relax is still ported exactly when it costs nothing (the gate's
  `str.isspace` set, the SDK's cache breakpoints): the allow-list is for things that cannot hold, not a licence.

## Lanes

| Lane | Crate | Replaces | Can start | Blocks |
|---|---|---|---|---|
| A | `pk-store` | `db/sdb.py`, `db/kbmap.py`, `api-dev/surreal_kb.py` (write path, LIVE), `creative_api` LineStore/Mirror | now (pk-kb is in) | T9 API, T6 three-way replay |
| B | `pk-creative` | `creative/store.py`, the `*_apply` reducers, `engine/verify.py` + `askchat.verbatim`, `askchat` retrieve/prompt/finish, `textdiff.ntok` | now | T8 worker, T9 API |
| C | `pk-llm` | `engine/providers.py` (Anthropic), `engine/config.py` + `config.hosted.toml`, `engine/budget.py` | now | T8 worker |
| D | `pk-embed` | `engine/embed.py`, `db/embedder.py`, `tools/kb/fetch_bge.py` | now | T8 (embed kinds), T9 (search) |
| E | `pk-obs` | `engine/prom.py`, `api-dev/metrics.py` + `worker/metrics.py` (names), `api-dev/crash.py` + `scrub.py` | now | T8, T9 |
| F | step 0 | measurement on the server | now (read-only) | the memory targets in RUST.md |
| G | `pk-worker` | `worker/__main__.py`, `worker/online.py`, `worker/execute.py` | after B, C, D, E | T8 |
| H | `pk-api` | `api-dev/*` routes | after A, B, D, E | T9, T10 |

## Tickets

| # | Lane | Ticket | Proof | Status |
|---|---|---|---|---|
| T1 | — | Workspace, `pk-kb`, gate, CI | 5 golden suites vs `kb.py` pass | **done** `c44eb31` |
| T2 | A | `pk-store`: connect (official crate) to a scratch `surreal start memory` 3.3.4; apply `db/schema.surql` + `schema_extra.surql` via `include_str!`; typed rows for the six node tables, the ten edge tables, `decision`, `creative_event`, `srcfile`; `kbmap` both ways; NONE vs NULL round-trip; the decision-append transaction (`CREATE decision` → the `apply_decision` event → read back); the `creative_event` mirror with the seq-unique guard; one LIVE subscription | tests against the scratch DB: a record-looking string stays a string; NONE ≠ NULL survives; a 3-line decision batch applies and reads back equal to `pk-kb`'s replay of the same lines; a denied write is an error, not silence | **done** 2026-10-29: 7 scratch tests + 13 unit; the three-way replay (SurrealDB `fn::apply_decision` vs `pk-kb` vs `kb.py`) agrees over the fixture's 15 hosted-shaped lines, the 2 the hosted path cannot make are schema errors; trap: select the database only after `DEFINE DATABASE … STRICT` or it is created lax for good |
| T3 | B | `pk-creative` part 1: `Store` replay over event lines + the reducers (`job_apply`, storyboards, workingsets, venues, account, merges, chats, intents, suggestions, profile); `verbatim` (one copy) with proptest + a corpus golden from both Python copies; `ntok` | goldens from `creative/store.py` and each `*_apply` over synthetic event logs; verbatim corpus incl. U+001C–U+001F, NBSP, `…`, astral | **done** 2026-10-29: 10 reducers, 230 events / 112 refusals equal; verbatim corpus 55 pairs + proptest; the two Python copies differ on one ellipsis case (Rust follows `askchat`) |
| T4 | B | `pk-creative` part 2: `askchat` sources / retrieve (BM25) / prompt / finish, with the gate | goldens: the same fake corpus → the same source ids, the same prompt messages, the same finish over canned answers | **done** 2026-10-29: `pk_creative::ask` (`Notebook` over the flattened sources + rooms + titles; `retrieve`, `prompt`, `finish`, `refusal`, `header`, `chip`, `tokens`, `material_owner`). Goldens from a synthetic notebook (27 sources, 5 rooms): 29 retrieve cases equal in ids **and in the summed scores** (the generator asserts no two scores within 1e-9 unless exactly tied); 7 prompt cases (named / unnamed owner, history, scope, long-text snippets, an unknown id = `KeyError`); 23 finish cases over canned answers (22 outputs equal, the `citations: 5` `TypeError` equal) + 3 refusals; 19 `tokens`, 15 `mmss` (ties to even), 13 snippet, 12 `month_year`, 7 owner cases; every header and chip. Red arm: four mutations (the `2.2`, the gate text, the rounding, `PER_JOKE`) each fail. The BM25 scoring is written out in `ask/bm25.rs` on purpose: the proof is the same order, so it must be the Python's own formula (twenty lines, not a search engine; hosted retrieval moves to SurrealDB's BM25 at T9). `gate_text` on a `Source` carries `Corpus.text_of` where it differs from the shown text (a moment's unstripped transcript, a typed idea whose node has its own `text`): observable in the goldens. Known differences, none in the goldens: `pinned` returns source order (Python a set); a truthy non-string `quote` faults with one message where Python's depends on the path; `YEAR_RE` takes ASCII digits. The Python `sources(c)` builder and `Corpus` stay Python until T9; `run_job` / `ask_local` are the worker's and the API's. |
| T5 | C | `pk-llm`: typed Messages API models, SSE streaming, `cache_control` on system blocks, `transform_schema`, the TOML router (tasks, models, fallbacks), `jsonschema` validation + one retry, `Budget` (UTC-day USD ledger), plain-words errors copied from `providers.py:56-77` | request bodies per task equal (parsed JSON) to those the pinned Python SDK builds; wiremock: normal, refusal, max_tokens, 429, cache usage; budget ledger golden | **done** 2026-10-29: 50 request bodies equal, `transform_schema` branch for branch, retry + fallback, ledger over 19 days, 7 refusal texts; `eventsource-stream` (not `reqwest-eventsource`, which reconnects) |
| T6 | D | `pk-embed`: bge-small over `ort` + `tokenizers`, CLS pooling, L2, query prefix; the hash embedder for tests; the model fetched by the existing `fetch_bge.py` (hash-checked) | token ids equal to Python's for a corpus; vectors cosine ≥ 0.99999 vs `engine/embed.py` on the same texts; the `fetch_bge.py` fingerprint | **done** 2026-10-29: 31 texts, token ids equal, cosine floor 1 − 6e-13 (ort 2.0.0-rc.15 = ONNX Runtime 1.31; `tokenizers` 0.22.1 exactly), hash embedder bit-equal |
| T7 | E | `pk-obs`: `tracing` init (env filter, stderr), the `prometheus` registry with the exact metric names / labels / types of `api-dev/metrics.py` and `worker/metrics.py`, `sentry` init with the `scrub.py` rules as `before_send` | the `/metrics` text has the same name/label/type set as Python after the same script; a scrubber property test (no sensitive key survives; red arm: bypass) | **done** 2026-10-29: 21 metrics equal (type, help, labels, buckets, values); scrubber goldens + proptest + red arms; sentry `before_send` over the typed event; no per-metric series cap (the label functions bound cardinality) |
| T0 | F | Baseline: Python API / worker image sizes and idle memory on the server; `db/tests/test_baseline.py` on the PC that holds the data | numbers in `RUST.md` "Build, CI, images" | **measured** 2026-10-29 (images 650 / 679 / 97 MB; API 371 MiB idle, worker 130 MiB); the baseline test is still to re-run where the data is |
| T8 | G (running: the loop, the API client, heartbeat, metrics listener, the kind trait with embed + ask_chat real; the writing kinds wait for pk-creative's prompt halves) | `pk-worker`: claim / run / result over `/api/worker/*`, heartbeat file, `:9100/metrics`, the hosted kinds, the budget pause, crash reports; the offline differential against the Python worker on a fixture API + wiremock Anthropic; image, k8s swap | both workers post equal results (ts/ids normalised); a week of `pk_worker` metrics in prod | open |
| T9 | H | `pk-api` reads: axum, JWKS auth (`jsonwebtoken`), actor rules, CORS, the GET routes group by group behind more specific Ingress paths | the baseline GET generator over both base URLs, equal as values with the allow-list | open |
| T10 | H | `pk-api` writes + `/api/worker/*` + `/api/live`, all at once | the POST sequence on scratch DBs: responses, reads, `decision` rows, `creative_event.line` equal as values | open |
| T11 | — | Retire Python: `dbload` on its own image; delete `api-dev/`, `worker/`, `engine/`, `creative/` from the images | the same gates on the shipped images | open |

## Working in parallel

One agent per lane, each with its crate directory and its generator, none editing another's; the lead merges,
runs the gate and commits. When two lanes need the same new workspace dependency, both add the same line; the lead
keeps one. `cargo build -p <crate>` and `cargo test -p <crate>` are the per-lane commands; `cargo xtask gate` is the
lead's.
