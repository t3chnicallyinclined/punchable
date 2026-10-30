# Docs index

| Doc | Area | State |
|---|---|---|
| [HANDOVER.md](HANDOVER.md) | start here: what is live, what learns, what is open, the traps | current; updated each session |
| [ARCHITECTURE.md](ARCHITECTURE.md) | the target architecture: crates, the DAG rule, pk-api / pk-worker shapes, one home per rule, conventions, the app layout, the order and the three measurements | current (2026-10-29) |
| [adr/](adr/README.md) | the standing decisions, one file each | current |
| [RUST.md](RUST.md) | the port plan: what exists, the wire-contract facts, migration steps and gates, images, progress | current; amended 2026-10-29 by ARCHITECTURE.md |
| [RUST_WORKSTREAM.md](RUST_WORKSTREAM.md) | the board: lanes, tickets, proofs, status | current |
| [ENGINEERING.md](ENGINEERING.md) | app build rules: platform splits, CI gates, the crash-class traps | current |
| [DEVELOPING.md](DEVELOPING.md) | dev setup, data layout, secrets, the test commands | current |
| [IOS.md](IOS.md) | phone operating manual: EAS, TestFlight, auth on the device | current; its 401 wording is older than the code (the token machine refreshes first) |
| [UX_2026-10.md](UX_2026-10.md) | product direction: Tonight, Bits, Sets, one input | current |
| [design/HIG.md](design/HIG.md) | the iPhone design spec: the rules, the per-screen spec, the kit mapping | current; §0 done, the "today" column and the file cites refreshed 2026-10-29 (A0); values in design/DESIGN.md |
| [design/DESIGN.md](design/DESIGN.md) | the design-system reference: colour roles with ratios in both schemes, the type scale and its caps, space / radius / size, the kit index, the mark, the login theme, the gate | current (2026-10-29, lane A0); its numbers must match `app/src/lib/theme.ts` until `src/ui/tokens.ts` lands |
| [design/ARCHITECTURE.md](design/ARCHITECTURE.md) | the app-side audit and layout proposal of 2026-10-28 | adopted by ARCHITECTURE.md section 7 with amendments; not yet executed |
| [brand/](brand/) | logo options | reference |
| `../api-dev/API.md`, `../db/DB.md`, `../engine/ENGINE.md`, `../worker/README.md`, `../infra/README.md`, `../infra/OBSERVABILITY.md`, `../rust/README.md`, `../gsync/SETUP.md`, `../tools/explorer/README.md`, `../app/README.md` | per-area docs | API.md, DB.md, ENGINE.md current for the Python until T11; `app/README.md` and the root README's worker paragraph are stale |

Referenced but never written (delete the reference or write the file): `docs/KB_SCHEMA.md`, `docs/API_CONTRACT.md`,
`docs/CONTEXT.md`, `gsync/GSYNC.md`, `creative/CREATIVE.md`, `UX_SPEC` / `SIMPLE.md`,
`tools/kb/kb_vocab.py`, `app/tests/e2e.mjs`, `tests/run.mjs`, `infra/DEPLOY_LOG.md` (now in the private operations
repo).
