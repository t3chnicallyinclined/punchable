# ADR 0007: equality is of parsed JSON values, not bytes

Date: 2026-10-29. Status: accepted.

**Context.** The Python compares creative event lines byte for byte and hashes them; Python's float repr differs from
serde's (`docs/RUST.md` fact 3).

**Decision.** The Rust side does not reproduce `json.dumps`. Differentials compare parsed values. The line hash is
recomputed over a canonical form Rust defines (`pk-core::canon`: compact, keys sorted); a one-time migration re-hashes
existing rows at T10.

**Consequences.** Goldens are written by Python as JSON and compared as values; `insta` is not used.
