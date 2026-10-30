# ADR 0002: popular, maintained crates; never hand-rolled clients or codecs

Date: 2026-10-29. Status: accepted (owner).

**Context.** The first port draft proposed a hand-rolled SurrealDB client, a crate reproducing Python's `json.dumps`,
and ports of the metrics registry and the JWT check.

**Decision.** The official `surrealdb` crate, `jsonwebtoken`, `prometheus`, `sentry`, `jsonschema`, `ort`, `axum`,
`reqwest`, `tokio`. Where the Python has a quirk, the compatibility requirement is relaxed rather than the quirk
reproduced. A short formula written out so a golden holds (the askchat BM25 scoring) is not a client or a codec and is
allowed when the module doc says why.

**Consequences.** Versions are pinned in `rust/Cargo.toml` `[workspace.dependencies]`; a crate says
`{ workspace = true }`.
