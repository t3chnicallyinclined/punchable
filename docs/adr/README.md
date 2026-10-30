# Architecture decision records

One file per decision, a few lines each: context, decision, consequences. A new decision gets the next number; a
reversed one is marked superseded and points at its replacement. The standing decisions were seeded on 2026-10-29
from `docs/HANDOVER.md`, `docs/RUST.md` and `docs/design/HIG.md`.

| # | Decision |
|---|---|
| [0001](0001-services-are-rust.md) | the services are Rust |
| [0002](0002-popular-crates-only.md) | popular, maintained crates only |
| [0003](0003-app-stays-typescript.md) | the app stays TypeScript |
| [0004](0004-phone-and-frontier-models-only.md) | phone + frontier models only |
| [0005](0005-testflight-only.md) | TestFlight only; email sign-in on the phone |
| [0006](0006-nothing-private-in-the-public-repo.md) | nothing private in the public repo |
| [0007](0007-equality-is-parsed-json.md) | equality is of parsed JSON values |
| [0008](0008-whitespace-is-unicode-white-space.md) | whitespace means Unicode White_Space |
| [0009](0009-surql-apply-decision-is-the-hosted-truth.md) | SurQL `fn::apply_decision` is the hosted truth |
| [0010](0010-crate-dag-rule.md) | the crate dependency rule |
| [0011](0011-worker-speaks-http-only.md) | the worker speaks HTTP only |
| [0012](0012-transcriber-kinds-stay-off-the-server.md) | the transcriber kinds stay off the server |
