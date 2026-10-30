# ADR 0010: the crate dependency rule

Date: 2026-10-29. Status: accepted.

**Decision.** Tiers: `pk-core`, `pk-wire` (0); `pk-kb`, `pk-llm`, `pk-embed`, `pk-obs` (1); `pk-creative`, `pk-store`
(2); `pk-api`, `pk-worker` (3). An edge only points to a lower tier. Pure crates never depend on an IO crate; IO crates
never depend on each other; the binaries never depend on each other; the API never depends on the model client.
`cargo deny check bans` enforces it with `wrappers`. Library crates never call `std::env`; a binary builds
`pk_core::config::Env` once and passes typed config down.

**Red arm.** Adding `pk-store` to `pk-creative`'s dependencies must fail the gate.
