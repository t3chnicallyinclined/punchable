# pk-engine: what was forked from upstream, and how

Decision: [docs/adr/0013-fork-the-upstream-engine.md](../../../docs/adr/0013-fork-the-upstream-engine.md). Owner,
2026-11-02: fork the upstream engine into Punchable and customise it; upstream is going public, so its code may
sit in this public repository.

**Upstream** is the owner's project engine (Rust; crates `upstream-ops`, `upstream-store`). Its repository is not public yet, so
this repository calls it "upstream" and its name stays in his private notes until he publishes it.

**Source.** Upstream's Rust branch branch at commit **`9dfedad`** (2026-11-02; the full hash is in his private
deploy log), the Rust workspace under `rust/crates/`. Every file below was read from that checkout's working tree on 2026-11-02; the
commit is the branch head at that moment, and the working tree was clean (see "Provenance check" at the end).

**Rule followed.** Proven code is ported as is first; each change is listed here with its reason. Nothing that runs a
process, reads the environment, touches files or the network, or reaches for a secret came across
(`tests/boundary.rs` fails if any does, with a red arm).

## Forked: file by file

| pk-engine | from (gw, 9dfedad) | ported as is | changed, and why |
|---|---|---|---|
| `src/verbatim.rs` | `pk-creative/src/verbatim.rs` (Punchable's own gate) + `upstream-ops/src/ask.rs` (`normalize`, `strip_numbers`, `align`, `lcs`, `tokens` → `words`, `nearest_line`, `QUOTE_MIN_WORDS`, `ALIGN_MIN`) | Punchable's `verbatim` unchanged (moved, not edited); upstream's helpers unchanged | **One gate**: Punchable's `verbatim` is the word-for-word test for both callers. Upstream's `align` is kept as a *locator* only (see `check.rs`). `pk-creative::verbatim` is now a re-export, so Ask and its goldens/proptests run the same function. |
| `src/check.rs` | `upstream-ops/src/ask.rs` `check` + its tests | every rule (read once, read all, cited in packet, lines inside, cut at the end, too short, changed since read, no longer present, a note may cite a gone record) and every upstream test, under `Rules::UPSTREAM` | (1) word for word = the one `verbatim` gate (accepts what `normalize(hay).contains(q)` accepted, whitespace aside, plus typography-only changes); (2) alignment is a switch, **off in `Rules::PUNCHABLE`** (a line that claims to be his must be his, word for word; a Punchable test pins it); (3) `Candidate` drops `project` / `commit` / `confirms()` (file-kind grading has no meaning for records of his material); (4) `Now` is a `BTreeMap` of a named struct instead of a tuple map; (5) `verdict` is an enum; (6) a neighbouring candidate is "the same file" only when both locators have a path (upstream compared `None == None`, which for records would treat every other candidate as the same file); (7) the loop body is `check_quote`. |
| `src/locator.rs` | `upstream-ops/src/store.rs` `line_range`, `first_line`, `numbered`; `ask.rs` `span_text` | all four | none (records without `#L` count their own lines from 1, which the code already did) |
| `src/rerank.rs` | `upstream-ops/src/rerank.rs` | `words`, `path_words`, `bm25*`, `bm25_window*`, `order_desc`, `rrf`, `rrf_positive`, the constants, the tests | the pool carries each candidate's rank in the store's **full-text leg and vector leg** instead of its project and in-project rank; `own_first` and its two ladder arms are gone (one notebook); arms `text`, `vector`, `hybrid` added; `path_words` also splits on `:` (record ids); `(k1, b)` passed as one pair; two names changed for clippy pedantic (`step`→`stride`, `scored`→`in_any`) |
| `src/rank_config.rs` | `upstream-ops/src/rank_config.rs` | `Bm25Spec`, `Order`, `LadderKnobs`, `validate` (every problem at once), canonical JSON + sha256 naming, `signal_order`, `order_pool*`, `ladder_arms`, the tests | `Signal::OwnFirst` gone; `Signal::Text` / `Signal::Vector` added; `per_bit` (Ask's `PER_JOKE`) added with `#[serde(default)]` so a upstream config still reads; **default = Punchable's hybrid** (RRF of text leg, vector leg, stemmed BM25; 24 places, 3 per bit), `RankConfig::upstream()` = the shipped upstream default for its tests; `load(path)` gone (no IO); errors are `EngineError`, not `anyhow` |
| `src/pack.rs` | `upstream-ops/src/ask.rs` `pack`, `order_traced`, `Trace`, `Packed`; `store.rs` `Hit` | facts first, deduplicated, cut by bytes and places, the trace | `Hit` is a record of his material (key, kind, title, text, bit, date, the two leg ranks, distance, lesson); hits keep the **store's given order** (upstream re-sorted by a score spanning projects); the `per_bit` cap is new; the trace drops the project index |
| `src/ask.rs` | `upstream-ops/src/ask.rs` `fan_out_with`, `open_traced` | the shape: read + facts, then `pack`, then numbered candidates | **async** over `EngineStore` (the read and the lessons run concurrently with `futures::join`, nothing blocks); no fan-out over projects, no reach or walls; the packet is returned, not written; the secret-shaped-question refusal and the `UPSTREAM_NO_RECORD` environment marker are gone |
| `src/store.rs` | `upstream-ops/src/store.rs` `retrieve`, `retrieve_facts` (their contract) | the contract: legs of records with scores, live lessons as their evidence | an **async trait** (`EngineStore`) instead of methods on a blocking `ureq` client; implemented in `pk-store/src/engine.rs` over the official `surrealdb` crate |
| `src/fact.rs` | `upstream-ops/src/store.rs` `needs_evidence`, `confirm_cap`, `Quote::verify`, `FactIn`; `journal.rs` kinds and grades | CONFIRMED needs evidence; the evidence is checked, not trusted; an owner statement confirms only when his own session signs it | evidence = a quote of his material through the one `verbatim` gate (the record's exact slice is kept) or his own statement; **run receipts dropped** (no `run`/`gate` here); kinds `gotcha` and `status` dropped (dev-journal kinds); `from_learned` turns the "What it has learned" lines (`profile:voice.learned`, cap 25) into lessons with a source |
| `src/learning.rs` | `upstream-ops/src/learning.rs` | `Mode`, `Reason`, `Switch`, `Source`, `Live` resolution (pointer → parse → hash check → default/fallback), `Kpis` + `pct`, `ReplayItem`/`Replay` + `kpis()`, the replay rule (reproduce the logged packet under its own config before scoring a candidate), `Guard` + the fence's three trips, `readback_holds`, the mode decision (his newest row, lowered by later member rows) | **decisions separated from IO**: the store fetches rows and hands them in (`Logged`, `ModeRow`, `Outcome`); gone with the IO: the per-process pointer cache, the `UPSTREAM_RANK_CONFIG` override, owner-password logins, the journal write on a trip, the export-folder read, `code_sha` (build env) |
| `src/proposer.rs` | `upstream-ops/src/proposer.rs` | `Fold`/`fold`, `Cand`, `Item`, `Outcome`, `score_item`, `Scored`, `Bounds`/`BOUNDS`, `in_search_space`, `Diff` (unknown fields refused) + `apply`, `parse_reply`, `front`, `batch` | pool entries carry the two leg ranks; `per_bit` in the bounds and the diff; `fold` reads the sha256 byte directly; errors typed |
| `src/card.rs` | `upstream-ops/src/card.rs` | `CardKind`, `Card` facets / whole text, `content_words`, `facet_problem` (empty, over `FACET_MAX`, no shared meaning word: **refused, not flagged**), `unservable` (re-checked at every read), the `STOP` list | a card belongs to a **bit**, not a project; `render` frames it (`frame::data`, quote first, facets labelled unverified) and renders an unservable card as nothing; quote resolution and the write are the store's |
| `src/bridge.rs` | `upstream-ops/src/bridge.rs` | `ARMS`, `Ranked`, `rank` (random / flat / facet with the ABC chain), `VETO_JACCARD`, `Z_MIN`, `Z_MIN_N`, `screen`, `below`, `jaccard` | bits instead of projects; `why` became `link` → `edge::Link` (plain words, no status) |
| `src/frame.rs` | `upstream-ops/src/frame.rs` | all of it | `write!` instead of `push_str(&format!)` (clippy); test tags renamed |
| `src/edge.rs` | new | — | the engine in the app's words: `Lesson` (text, where it came from, his words, the record; **no grade field exists**) and `Link`; a test pins the JSON keys and that no grade word appears |
| `src/error.rs` | new | — | `thiserror` enum; upstream used `anyhow` in a library |
| `schema/engine.surql` | `upstream-store/schema/0005`, `0006`, `0022` (the ideas) | — | written for Punchable's namespace and tables: `lesson` (evidence = a record of his material + the exact quote; an event refuses a CONFIRMED row without evidence), `rank_config`, `rank_switch`, `learning_mode`, `packet`. Applied **only** by `pk_store::Store::apply_engine_schema`, never on connect; upstream's migrate-on-open is not forked |
| `pk-store/src/engine.rs` | `upstream-ops/src/store.rs` `retrieve`, `retrieve_facts`, `any_word`, `query_words` | the any-word full-text query (each word its own `@n@`, scores summed), the 16-word cap, lessons shown as their evidence record | async; four full-text legs (`unit.text`, `joke.title`, `joke.summary`, `performance.text`) and three vector legs (bge HNSW, the indexes `db/schema*.surql` already define) in one round trip; a record's text rank is its best rank in any leg (never two indexes' scores compared); a lesson resting on his statement is its own candidate |

## Not forked (and why)

| Upstream | why not |
|---|---|
| `exec.rs`, `secrets.rs`, `secret_path.rs`, `census*.rs`, `hook.rs`, `agent_setup.rs`, `claude_setup.rs`, `agents_audit.rs`, `scaffold.rs`, `disk_guard.rs`, `supervise.rs`, `transcript_tail.rs`, `meter.rs`, `pulse*.rs`, `home.rs`, `registry.rs`, `survey.rs`, `scan.rs`, `intake.rs`, `project_settings.rs`, the `upstream's MCP server` server, the `gw` binary, the desktop window | dev tooling: runs shell commands, reads secrets or the owner's machine, or serves agents. **Nothing that runs a process or touches a secret enters Punchable** (owner rule; `tests/boundary.rs`) |
| `refusal.rs` (secret-shaped text refusal) | a guard for a dev journal; his material holds no secrets, logs go through `pk-obs`'s scrubber |
| `proposer.rs` `ClaudeCli`, the `search` loop, `StoreSink`, `proposer_run.rs` | `ClaudeCli` spawns the `claude` command line. A later lane makes the one tool-less call through `pk-llm` in the worker; the pure scoring it drives is forked |
| `eval*.rs`, `held_out.rs`, `parity.rs`, `attack.rs`, `lens.rs`, `panel.rs`, `refusal` metrics, `funnel.rs`, `progress.rs`, `knowledge.rs`, `journal.rs` (beyond its kinds), `store_migrate.rs`, `store_status.rs`, `bridge_eval.rs` | Upstream's evaluation, multi-project and agent-journal machinery; Punchable's proof is `rust/tools/engine_proof.py` + `pk-store/examples/engine_proof.rs` |
| `upstream-store` (migrations, scopes, member roles, `connect_root`) | Punchable has its own schema, namespaces and `pk-store`; migrate-on-open would touch prod. Roles stay Punchable's (one comic); a per-comic namespace is **not** designed out: every engine read goes through `EngineStore`, which a per-comic store implements |
| `upstream-core` blocking `ureq` client | replaced by the async official `surrealdb` crate in `pk-store` |

## Tests carried over

Upstream's tests for `check` (8), `rerank` (6), `rank_config` (4, two adapted to the hybrid default), `pack`'s order
test, `frame` (1) and the `verbatim` helpers run here; Punchable's verbatim unit tests moved with the gate, its
goldens (`pk-creative/tests/goldens.rs`, against both Python copies) and property tests
(`pk-creative/tests/verbatim_props.rs`) still run in `pk-creative` through the re-export.

**Red arms.** (1) `tests/boundary.rs` `the_scan_can_fail`: one `std::process::Command::new("sh")` line in a copy of the
sources makes the scan report two violations. (2) `check::tests::under_punchables_rules_a_line_that_claims_to_be_his_must_be_word_for_word`:
switching `Rules::PUNCHABLE.align` to `true` fails it. (3) `edge::tests::no_grade_word_reaches_the_app`: adding a
`grade` field to `Lesson` fails it.

## Changes made after review (2026-11-02; the engine's architecture reviewer, rust-application-architect, the security reviewer)

| Change | Why |
|---|---|
| `check`: a lesson's own text is quotable only under `Rules::UPSTREAM` (`lesson_text`); the lesson and its record are searched apart, never as one joined text | a lesson's text is the engine's or a model's words, not his; a quote could run from the end of the lesson into the record (both proven by a probe, now tests) |
| `fact::Checked` has private fields and no serde; only `check_fact` makes one; `pk-store::add_lesson` takes the proposed `FactIn`, fetches the record's words itself and refuses a hidden record | anyone could hand the store a CONFIRMED lesson on words he never said |
| evidence = at least 3 whole words (`fact::whole_words`) | "he" inside "the" passed the 3-character gate |
| `edge::Lesson`: "You …" only for a statement his own session signed | an unsigned statement was put in his mouth |
| `pack`: lessons are cut to `facts_k` *before* their evidence is hidden from the hits; the trace keeps the read as it came in | **a upstream bug**: a smaller `facts_k` lost the cut lessons' records, biasing replay toward the live config |
| `ask::open`: a failed lessons read opens the packet without lessons and says so (`lessons_skipped`, `trace.notes`); candidates cut to `max_hits`; `ask::render` frames the packet (`frame::data`, lessons labelled "not his words") | the ask failed whole without the engine tables; `facts_k == max_hits` gave `max_hits + 1`; upstream frames candidates and the port had dropped it |
| `learning::replay` leaves out held-out questions (`question_hash`, upstream's `eval::question_hash`) | Upstream's F3 rule had been lost |
| `proposer::score_item` applies `per_bit`; `Cand.bit` | scores must match what a live packet holds |
| `rerank::question_words` (Ask's `QSTOP`, moved here as `ASK_QSTOP` so `pk-creative` reads the one list, + English function words): the store's legs and the local BM25 read only what a question is about | measured: "the" cost ~60 ms per full-text leg against 1.8 ms for a content word, and drowned the content words |
| `rerank`: document frequency with a `HashSet` (was `Vec::contains`, quadratic on a set transcript); `words` no double allocation | performance |
| `error`: inner errors printed once (no `#[source]` beside `{0}`) | double print in chains |
| `pk-store::engine`: legs return ids only, each record fetched once; the four full-text legs fused by RRF with ties broken by the vector leg; the corpus count moved to `Store::corpus_size` (3 full scans, ~110 ms, measured); `Read.corpus_total` is 0 from this store | ties went to `joke:` before `perf:` before `unit:` by key; the count cost a third of a read |
| `schema/engine.surql`: upstream 0022's guards, adapted: READONLY fields, the config id = sha256 of its JSON, `rank_snapshot`, a rollback bound to a snapshot's own config, his statement on every pointer move but an auto-rollback, the learning mode raised only by a `signed` row with his statement, every learning row append-only (events, which bind root and database users where PERMISSIONS do not), a CONFIRMED lesson needs non-hidden evidence or a signed statement | the first port had dropped every guard; proven on a real 3.3.4 server by `pk-store/tests/engine_scratch.rs` |
| `tests/boundary.rs`: whitespace-free and cross-line scans, `std::{`, `pk_core::config` / `Env::` / `from_process`, `include_bytes!`, `#[path`, `std::io`, `tokio::`; every manifest dependency table (renames, `target.*`, build) via `toml`; no `build.rs` | the first scan could be fooled several ways (security review) |

Also not forked, found in review: `AnswerIn.record` (keep a passed answer as a fact by default; recording is the caller's) and `bounds_text` (the proposer prompt line).

## Run 3 and the security re-review (2026-11-02)

| Change | Why |
|---|---|
| `Signal::Fused` + `Hit.fused_rank` + the ladder arm `fused`: the store's own fusion of its legs (`search::rrf`, grouped as today's Search groups them: bits, then moments; merged by `rrf_score`) | the pre-registered run-3 candidate; **it did not pass** (docs/RUST_WORKSTREAM.md T19). The default order is unchanged |
| `schema/engine.surql`: a lesson's quoted record must exist, not be hidden, and hold the quote (checked on create, so a lesson can still be retired after its unit is hidden); every `at` within 5 s of the server's clock; the header says plainly that the guards stop bugs, not a holder of a root or EDITOR login, and that per-role logins with table PERMISSIONS come before any service writes these tables | re-review: a quote he never said, hidden evidence beside a forged `signed`, and a future-dated row that stays "newest" were all accepted |
| `tests/boundary.rs`: whole-line comments only (a `//` in a string hid the rest of the line), `include!(`, `use std::*`, `use std as`, `extern crate std`, renames in `[workspace.dependencies]` | re-review bypasses |
| `pk-store/tests/engine_scratch.rs`: a throwaway server password (never the secrets file's); each refusal checked by its message; the new refusals tested | re-review |
| `rust/tools/engine_proof.py`: refuses when `SURREAL_*` is set (it overrides the scratch file); Search's two lists merged by `rrf_score` through a capturing wrapper (the run-1/2 scorer merged by the full-text `score`, an instrument bug; set 1's Search numbers were re-scored) | re-review; measurement fix |
| Upstream's crate, branch and reviewer names and store receipt ids genericized in the public files | re-review: not public yet |

Not fixed here, and why: `signed`, snapshots, promotes and pins cannot be proven in the database while every service
shares one login (the re-review's D and E); that is the wiring step's first task. Quotes under three words in the
answer check still match as substrings (low impact).

## Provenance check

**CONFIRMED** by the upstream architect: `git status --short rust/crates` was empty and HEAD = Rust branch = `9dfedad` when the
files were read; the forked files were last modified before the fork's files were written.
