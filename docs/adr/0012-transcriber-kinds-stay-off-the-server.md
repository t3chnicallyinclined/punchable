# ADR 0012: the transcriber kinds stay off the server

Date: 2026-10-29. Status: accepted. **Decided 2026-10-31: on-device.** The owner chose voice memos recorded in
Punchable with the audio kept on the phone; the iPhone app now transcribes memos and its own taped sets on the phone
(expo-speech-recognition over Apple's on-device recognizer) and posts only the words (`POST /api/ideas`, `POST
/api/sets/transcript`): `docs/design/SPEC-2026-10-31-voice-memos.md`. The kinds below stay off the server; what still
waits on them is YouTube links, older uploads and sets taped in the web app.

**Decision.** `process_set` and `import_youtube` are never claimed by the hosted worker and are not ported; the API
keeps them `waiting_for: transcriber`. Which path replaces them (on-device transcription in the production build, or a
cloud speech API from the worker) is the owner's open decision; until then the Record screen must say a taped set is
saved but not transcribed.
