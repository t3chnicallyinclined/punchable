# ADR 0012: the transcriber kinds stay off the server

Date: 2026-10-29. Status: accepted; the owner still owes the product decision.

**Decision.** `process_set` and `import_youtube` are never claimed by the hosted worker and are not ported; the API
keeps them `waiting_for: transcriber`. Which path replaces them (on-device transcription in the production build, or a
cloud speech API from the worker) is the owner's open decision; until then the Record screen must say a taped set is
saved but not transcribed.
