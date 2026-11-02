# ADR 0013: fork the upstream engine into the Rust workspace as `pk-engine`

Date: 2026-11-02. Status: accepted (owner, 2026-11-02: "fork [upstream] into punchable and edit/customize it for
punchable in the way it makes most sense"; "[upstream] is going public", so its code may sit in this public repo; the
name in brackets is his project engine's, kept in his private notes until he publishes it). Wiring it into `pk-api` /
`pk-worker` is a separate decision that waits on the offline proof and his go.

**Context.** Upstream (the owner's project engine, Rust, `rust-successor` 9dfedad) already has what Punchable's next
steps need: a fact store with an evidence rule, an ask whose every quote a program checks, a ranking config that is
versioned, replayed offline, proposed, promoted and rolled back, and cards and bridges between ideas. Punchable has its
own verbatim gate (`pk-creative::verbatim`), its own Ask retrieval (in-memory BM25 + comedy rules) and its own search
(SurrealDB full-text + bge vectors). Upstream's retrieval is full-text only, its store client is blocking, it
migrates its schema on open, and much of it is dev tooling that runs processes and reads secrets.

**Decision.**
- A new crate `pk-engine`, **tier 1, pure** (deps: `pk-core` and plain crates; no IO, no environment, no processes,
  pinned by `tests/boundary.rs`). Its tables live in `schema/engine.surql`, applied only by
  `pk_store::Store::apply_engine_schema` on an explicit call, never on connect; on the hosted database by his go
  through the ADR 0009 path. `FORK.md` records the source commit per file, what was dropped and what changed.
- **One verbatim gate**: `pk-creative::verbatim` moves to `pk_engine::verbatim`; `pk-creative` re-exports it, so Ask,
  its goldens and its property tests and the engine's answer check run one function. Upstream's paraphrase
  alignment is kept as a locator and is **off** under Punchable's rules: a line that claims to be his is his, word for
  word.
- The store seam is an **async trait** (`EngineStore`) implemented in `pk-store/src/engine.rs` over the official
  `surrealdb` crate and the indexes `db/schema*.surql` already define; the ranking fuses the full-text leg, the bge
  vector leg (`pk-embed` makes the query vector) and BM25 over the read.
- No dev tooling, shell, secret path or MCP/desktop code comes across. The app sees lessons and links in plain words;
  no CONFIRMED / INFERRED grade ever reaches it (`edge.rs`, tested).
- Multi-user: not built, not designed out (every read goes through the trait a per-comic store can implement).

**ADR 0010 amendment.** Tier 1 gains `pk-engine`; `pk-creative` and `pk-store` (tier 2) depend on it. `surrealdb` stays
banned outside `pk-store` (`deny.toml` unchanged).

**Proof before wiring.** An offline comparison on a scratch database loaded from the local data folder: the fork's
packets against Ask's and Search's on about 20 questions he would ask his notebook, recall of the right bit and
latency, on the owner's desktop. Pass bar: no worse than today's retrieval. The numbers are in `docs/RUST_WORKSTREAM.md`
(T19); the questions and gold quote his material and stay in his private folder.

**Result (2026-11-02): not passed.** The fork's default order found the right bit within 24 places on 18 of 24
questions against Search's 24 and Ask's 19, at 175 ms p50 against Search's 34 ms. The engine is kept on its branch,
unwired; the next attempt is a held-out question set and a config that keeps SurrealDB's own fusion as the store leg.
