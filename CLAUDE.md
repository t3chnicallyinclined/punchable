# Punchable: how to work in this repo

Start with `docs/HANDOVER.md` (what is live, what is open), then `docs/ARCHITECTURE.md` (how it is laid out and why),
`docs/adr/` (the decisions; do not re-open them), `docs/RUST_WORKSTREAM.md` (the board). The index of every doc is
`docs/README.md`.

## Gates (all must be green before a commit)

```bash
cd rust && cargo xtask gate                                   # fmt, clippy -D warnings, tests, per crate; cargo deny
cd app && npm run typecheck && npm run test:unit && npm run lint && npx expo-doctor
python -m pytest creative engine gsync worker infer/tests tools/audio/tests api-dev/tests -q   # the Python oracle, until T11
```

A skipped test is a failed test: the pk-store and pk-embed suites need `~/.punchable/bin/surreal` and the bge model;
run with `PK_REQUIRE_EXTERNAL=1` to make a missing one fail instead of skip.

## Rules

- Nothing private in this public repo (ADR 0006): no material, secrets, hostnames beyond punchable.io / auth / bugs,
  other projects' names. The commit guard runs on commit and push; fix the content, never bypass it.
- One lane owns one crate directory and its generator under `rust/tools/`; never edit another lane's crate.
  `rust/Cargo.toml` and `Cargo.lock` are shared: add lines, never reorder or remove others'.
- Popular crates only (ADR 0002); the crate DAG rule (ADR 0010); library crates never read the environment.
- Equality in a differential is of parsed JSON values (ADR 0007); goldens are synthetic and written by the Python
  generator; the Rust test compares. Never regenerate a golden from the code under test.
- Edition 2024, `unsafe` forbidden, `thiserror` in libraries, `anyhow` in binaries, clippy pedantic clean, rustfmt 120,
  `//!` module docs that say what a module is for.
- The app: HIG first on iPhone (`docs/design/HIG.md`), the platform-split rules in `docs/ENGINEERING.md`, the layout in
  `docs/ARCHITECTURE.md` section 7. Jots and questions are typed; voice is for recording sets only.
- Commit with the owner's git identity (from `git log -1 --format='%an <%ae>'`), stage paths explicitly (never
  `git add -A`), end the message with a `Co-Authored-By:` line for the agent that helped. No time estimates in docs.
- Never on the server: `docker run`, `ship.sh test`, touching other namespaces beyond the agreed mounts.

## Which doc is authoritative

| For | Read |
|---|---|
| live state, open items, traps | `docs/HANDOVER.md` |
| the target layout, conventions, lane order | `docs/ARCHITECTURE.md` |
| the Rust port plan and migration gates | `docs/RUST.md`; the board `docs/RUST_WORKSTREAM.md` |
| app build rules, CI gates, platform splits | `docs/ENGINEERING.md`; phone operations `docs/IOS.md` |
| product direction | `docs/UX_2026-10.md`; iPhone design `docs/design/HIG.md` |
| data, secrets layout, dev setup | `docs/DEVELOPING.md`, `db/DB.md`, `infra/README.md`, `infra/OBSERVABILITY.md` |
