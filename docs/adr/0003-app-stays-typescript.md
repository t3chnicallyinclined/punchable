# ADR 0003: the app stays TypeScript

Date: 2026-10-29. Status: accepted (owner).

**Context.** iOS apps are Swift or React Native; the app is Expo React Native and ships to TestFlight and the web from
one codebase.

**Decision.** `app/` and `infra/web` stay TypeScript. Rust goes on the phone only as a library (UniFFI) when a feature
needs it (on-device transcription, laugh detection), never for the UI.

**Consequences.** Rules that must exist on both sides are pinned by shared `contracts/` test cases; after T9 the app's
wire types are generated from `pk-wire`.
