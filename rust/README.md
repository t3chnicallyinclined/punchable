# Punchable services in Rust

The port of the Python API, worker and engine. Why, what, in which order and how each step is proved:
**[`../docs/RUST.md`](../docs/RUST.md)**. Nothing here ships until the worker swap (step 3 there).

```
crates/
  pk-kb        the graph with no IO: ids, decisions, the replay, the read models, the decision builder   (step 1, in progress)
  pk-creative  the rules with no IO: stores, the verbatim gate, askchat, writing, merge                 (step 1)
  pk-store     SurrealDB through the official crate: rows, the decision transaction, the mirror, LIVE   (step 1)
  pk-llm       the Anthropic client, the task router, schema validation, the budget                     (step 2)
  pk-embed     bge-small over ort + tokenizers                                                          (step 2)
  pk-obs       tracing, Prometheus metrics, sentry with the scrubber                                    (step 2)
  pk-api       axum: every /api route the app calls                                                     (steps 4-5)
  pk-worker    the claim / run / result loop                                                            (step 3)
xtask/         the gates and the differential runners (cargo xtask ...)
tools/         CPython scripts that generate the goldens the Rust tests replay against
```

Gates run per crate: `cargo test -p pk-kb`. Lints: `cargo clippy -p <crate> --all-targets -- -D warnings`, `cargo fmt --check`.

Goldens: `python tools/gen_kb_goldens.py` (needs the repo's Python; writes `crates/pk-kb/tests/goldens/`). The committed
goldens come from synthetic fixtures only; real data is never in this repo.
