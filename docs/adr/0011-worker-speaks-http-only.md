# ADR 0011: the worker speaks HTTP to the API only

Date: 2026-10-27. Status: accepted.

**Decision.** The worker claims, runs and posts results over `/api/worker/*` with a `pk-worker` client-credentials
token, and never opens the database. The API applies the actor rules on every worker write. The worker cutover is one
image swap, never a per-kind split (one worker, one spend ledger).
