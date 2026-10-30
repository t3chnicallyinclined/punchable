# rust/: the crate rules

Read `docs/ARCHITECTURE.md` sections 1 to 5 and `docs/adr/0010-crate-dag-rule.md` first; the board is
`docs/RUST_WORKSTREAM.md`.

- Gate: `cargo xtask gate` (fmt, clippy `-D warnings`, tests, per crate, never `--workspace`; `cargo deny`). Per lane:
  `cargo build -p <crate>`, `cargo test -p <crate>`. Build only in this `target/`; stop on os error 112 (disk).
- The DAG: `pk-core`, `pk-wire` -> `pk-kb`, `pk-llm`, `pk-embed`, `pk-obs` -> `pk-creative`, `pk-store` -> `pk-api`,
  `pk-worker`. Edges point down only. Libraries never call `std::env`.
- Anything named after a Python builtin (`repr`, `str`, `isspace`, ...) lives in `pk-core::py` and nowhere else.
- Goldens: a CPython generator in `rust/tools/gen_<crate>_goldens.py` writes synthetic JSON into
  `crates/<crate>/tests/goldens/`; the Rust test compares as parsed values; CI regenerates and fails on drift, so the
  generator must be deterministic (sort every set). Every golden suite has a red arm that is written down in the board.
- Edition 2024 traps met: `gen` is reserved (`r#gen`); `ndarray`'s `s!` expands with `#[allow(unsafe_code)]` and is
  refused (index axes instead); `python -I` cannot see the user site on the owner's PC (run the generators with plain
  `python`).
- Error texts the app can show are part of the contract: change them only with the golden or contract in the same
  commit.
