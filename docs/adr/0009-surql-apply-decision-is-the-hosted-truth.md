# ADR 0009: SurQL fn::apply_decision is the hosted truth

Date: 2026-10-29. Status: accepted.

**Decision.** Decision replay stays in two places on purpose: `db/schema.surql` `fn::apply_decision` applies a decision
row inside SurrealDB (the source of truth), and `pk-kb::view` replays in memory. The three-way test (SurQL vs `pk-kb`
vs the Python `kb.py` while it exists) stays in CI permanently.
