# ADR 0001: the services are Rust

Date: 2026-10-29. Status: accepted (owner).

**Context.** The API, worker and engine were built in Python; nothing recorded chose Python over Rust. The owner wants
one toolchain across his projects, one binary per service, memory that can be reasoned about (the API pod hit 824 MiB
of 1 GiB on 2026-10-28), and a crate graph without the import cycles the Python has.

**Decision.** New services are Rust. The Python API, worker and engine are ported in the order in `docs/RUST.md`
(worker first, API reads by route group, every write at once, retire Python), each step gated by a differential against
the Python it replaces.

**Consequences.** Two languages until T11; the Python is the oracle and its tests run in CI until then. The app stays
TypeScript (ADR 0003).
