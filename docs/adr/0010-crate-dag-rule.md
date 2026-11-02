# ADR 0010: the crate dependency rule

Date: 2026-10-29. Status: accepted.

**Decision.** Tiers: `pk-core`, `pk-wire` (0); `pk-kb`, `pk-llm`, `pk-embed`, `pk-obs` (1); `pk-creative`, `pk-store`
(2); `pk-api`, `pk-worker` (3). An edge only points to a lower tier. Pure crates never depend on an IO crate; IO crates
never depend on each other; the binaries never depend on each other; the API never depends on the model client.
`cargo deny check bans` enforces it with `wrappers`. Library crates never call `std::env`; a binary builds
`pk_core::config::Env` once and passes typed config down.

**Amended 2026-11-02 (ADR 0013).** Tier 1 gains `pk-engine` (pure, the upstream fork); `pk-creative` and
`pk-store` depend on it.

**Amended 2026-11-02 (T8, the one-shot Ask).** Tier 1 gains `pk-fts`: an in-memory SQLite FTS5 table (porter
unicode61, `bm25()`) through `rusqlite` with `bundled`, so the Ask ranks exactly as the Python's `sqlite3` does. It
counts as pure: no file (`temp_store = memory`), no environment, no network; it is the one crate in the workspace that
compiles C of our choosing (SQLite's public-domain amalgamation; ONNX Runtime arrives prebuilt through `pk-embed`). Only
`pk-worker` depends on it; `pk-creative` stays C-free and takes the search as a function. `deny.toml` bans `rusqlite`
outside `pk-fts` and `libsqlite3-sys` outside `rusqlite`. Red arm, run 2026-11-02: `rusqlite` in `pk-creative`'s
dependencies fails `cargo deny check bans` ("crate 'rusqlite = 0.41.1' is explicitly banned").

**Red arm.** Adding `pk-store` to `pk-creative`'s dependencies must fail the gate.
