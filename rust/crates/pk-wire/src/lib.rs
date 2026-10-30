//! pk-wire: Punchable's typed HTTP contract, with no behaviour.
//!
//! Two halves, both proven by `tests/goldens.rs` against JSON the Python API really wrote over a synthetic
//! knowledge base (`rust/tools/gen_wire_goldens.py`):
//!
//! - [`worker`]: the whole `/api/worker/*` protocol the worker speaks (`worker/__main__.py`, `worker/online.py`,
//!   `gsync/punchable.py`) and the API answers (`api-dev/ingest_api.py`, `creative_api.py`, `profile_api.py`):
//!   request bodies and response bodies as closed structs; ids are strings; `Option` is `null` where Python writes
//!   `null`, and a field Python leaves out is `skip_serializing_if` + `default`.
//! - [`api`], [`creative`], [`simple`]: the app-facing response shapes, one module per hand-written TypeScript file
//!   in `app/src/lib/`, named as there. They are what the Python sends today (the acceptance test, `docs/RUST.md`
//!   "Errors and wire shapes"), so where the TypeScript and the wire disagree the struct follows the wire and the
//!   doc comment says so.
//!
//! Open knowledge-base records (a node, an edge, a decision line, a merge record) keep `#[serde(flatten)] extra`:
//! the replay is a dict merge and a fully typed struct would drop fields. Everything else is closed.
//!
//! Feature `ts` (off by default) derives `ts_rs::TS` on every type, for the app's generated TypeScript after T9.
//! No internal crates, no error type, no `std::env`: the DAG's tier 0 (ADR 0010).

#![forbid(unsafe_code)]

pub mod api;
pub mod creative;
pub mod nullable;
pub mod simple;
pub mod worker;

#[cfg(test)]
mod shape {
    //! Every wire type is `Serialize + Deserialize`: a compile-time check that none of the derives were dropped.
    use serde::{Deserialize, Serialize};

    fn assert_wire<T: Serialize + for<'de> Deserialize<'de>>() {}

    #[test]
    fn every_type_is_a_wire_type() {
        assert_wire::<crate::worker::Claim>();
        assert_wire::<crate::worker::Job>();
        assert_wire::<crate::api::JokeSummary>();
        assert_wire::<crate::creative::TodayFeed>();
        assert_wire::<crate::simple::Build>();
    }
}
