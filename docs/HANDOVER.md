# Punchable: start here (handover, 2026-10-28, updated 2026-10-31: voice memos and on-device transcription; 2026-11-02: the cloud map as a real 3D view, committed for the build after 20)

Read this first in a new session. It says what is live, what learns, what is open, and the traps that cost time
today. It is public-safe: no secrets, no server names, no material. Private operating state (deploy log with image
digests and rollbacks, token notes) lives in the owner's private operations repo (its `infra/` deploy log); secrets and private
settings live in `~/.punchable/` and are never printed or committed.

## What Punchable is

One comic's joke-writing app. It learns **his** voice from **his** material only (notes, Google Docs, taped sets) and
every AI line that claims to be his is checked word for word against his notes first. Personal use first; the hosted
copy at punchable.io is his. Product direction: `docs/UX_2026-10.md`. iPhone design rules: `docs/design/HIG.md`.
Engineering rules (platform splits, CI gates): `docs/ENGINEERING.md`. Phone operating manual: `docs/IOS.md`.
The Rust port of the services (the standing direction, the workspace, the migration gates): `docs/RUST.md`.

## Live right now

| Surface | State |
|---|---|
| Web | punchable.io, API and web images from `51bfdf1` (the workbench), rolled out 2026-10-29. Tabs Tonight · Bits · Sets. The cloud map computes in ~4 s on first request (1,576 bits, 14 groups), then from a 5-minute cache. |
| iPhone, next build | **2026-11-02: build 20 was building on EAS from `ce0b0dc` (the memo work below); the 3D cloud map (local commit of 2026-11-02, not pushed) is NOT in 20 and goes in the next build (owner checks: open item 0a).** Before that: **not built yet (EAS free iOS quota until Nov 1; push and build need the owner's go).** On `main`, local commits of 2026-10-31: voice memos recorded in Punchable and **on-device transcription** of memos and taped sets (`docs/design/SPEC-2026-10-31-voice-memos.md`; the new native module `expo-speech-recognition` 58.0.1, so the next binary is required: an older dev client will not have it and treats transcription as unavailable). Plus six SDK 58 patch bumps for expo-doctor 20/20. Owner checks for that build: open item 4. |
| iPhone | TestFlight **0.1.0 (18)**, built from `617ae81` and auto-submitted 2026-10-30 (Set lists are his with the drafts folded and archive/undo, notes on stage, the rules layer, the upload 401 fix, Reconnect Google web-only); 17 had the Skia cloud + pinch fix; 12 had the workbench. **Owner checks on 18** are the three in open item 0b. Older: TestFlight **0.1.0 (12)**, built from `51bfdf1` (the workbench), auto-submitted 2026-10-29; 11 had the record control, Set lists · Recorded, Bits Recent / + / sort and the cloud map; 10 had search over everything. Builds 4-6 crashed (self-import, see ENGINEERING.md §1); 7-11 are good. |
| API + hosted worker | Python API behind Keycloak; the worker runs Claude (Opus 5.5 riff / punch-up / set analysis, Sonnet 5.5 ask / bridge, Haiku 5.5 merge / triage) with a daily budget. No home PC in the loop. |
| Database | Hosted SurrealDB is the single source of truth (online-first since 2026-10-27). Nightly backup Job to R2 with a bucket-scoped token; the Bugsink database rides along. |
| Search | `GET /api/search?q=` (bits + taped set moments) and `GET /api/jokes/<id>/similar` ("did I ever do this, is it like anything"), `api-dev/search_api.py`. Full-text + bge-small vectors over unit text, joke titles, **typed ideas** (`joke.summary`) and **set moments** (`performance.text`). The API image ships the model (`PK_BGE_DIR`); a sweeper embeds anything new within a minute. Before 2026-10-28 the hosted API had no model (word search only), the 162 typed jots were not indexed at all and the 675 taped moments were invisible. **Live since 2026-10-28 20:42 UTC**: 159 typed ideas (3 have no text) and all 675 moments embedded; a fused query takes ~0.1 s on the pod; API memory 450 MiB idle, ~560 MiB during a sweep (limit 1 Gi). |
| Crash reporting | Self-hosted Bugsink at bugs.punchable.io. API wired and proven end to end. App DSN is an EAS environment variable (`EXPO_PUBLIC_SENTRY_DSN`, pulled by each `eas.json` profile's `environment`). Worker wired 2026-10-28 (`worker/crashreport.py`, the same Secret, release `punchable-worker@…`, tags kind/retry; a budget pause is never reported) and live since 2026-10-28 20:35 UTC (`crash reporting on: punchable-worker@…` in its log). |
| Metrics | Prometheus scrape of API + worker, Grafana dashboard, 13 alert rules. Alerts for `service=punchable` have their own Alertmanager route and receiver since 2026-10-29 (`[Punchable …]` subject, runbook in the body); the sender becomes `alerts@punchable.io` once the owner verifies punchable.io in Resend. **Two alerts can never fire** (backup missed, certificate expiring): the cluster has no kube-state-metrics and cert-manager is not scraped. Fix needs the owner's go (shared namespace). |
| DNS | `infra/dns/cf_dns.py plan` / `apply` with a token scoped to this one zone (DNS Write + Zone Read). Records: apex, auth, bugs. |

## What the app does today (phone + web)

- **Tonight**: tonight's set list as one bar, "N to sort" inbox, three questions to ask the material, a system search
  field ("Ask your material", typed only). "+" makes a new bit at once (the same `lib/newBit` as Bits); the ⋯ menu
  holds New set list · Record a set · Import a recording… (the create sheet is gone, SPEC 2026-10-30). No input bar in the tab bar (owner decision).
- **Bits**: opens on what he touched last (Recent; Performed / Strongest in the toolbar's sort menu), "+" opens a fresh bit as a document (created at once, the editor focused; the first Save names it from its first line); a search shows the matching bits and, under "In your sets", the taped moments that say it (tap = the set at that second, playing); a bit's page is the **workbench** (2026-10-29): tap the text to edit (Save = a NEW version, the old ones stay, undoable; `POST /api/jokes/<id>/versions`), the input bar talks to a conversation pinned to this bit (`lib/BitTalk`; `POST /api/ask/chats {question, joke}`: the bit's versions, idea and tapes lead every turn's sources) with **Keep as a line** and **Merge “X” into this** under each answer, or adds a typed line as a version; **Elsewhere** lists the tapes it is on, taped moments that sound like it (**link it**) and bits that sound like it (**merge it in**); a Bits search has **Select → Make these one bit** (`pickMain`: most versions, then nights); **Map** is the **cloud** by default (`/api/map/cloud`; every bit placed by its vector, 3-axis PCA, named k-means groups). **Since 2026-11-02 a real 3D view** (`docs/design/SPEC-2026-11-02-cloud-3d.md`): an orbit camera in `rules/cloudMath.ts` (yaw + pitch, a target, zoom 0.6-12×, real perspective, a near fade); on the phone `lib/Cloud.native` draws it with **react-native-skia** at device resolution (zoom moves the camera, it no longer scales the view, which is what blurred), the camera in Reanimated shared values, gestures inline worklets (one finger orbits with momentum, pinch zooms about the fingers and pans, double tap zooms in, double-tap-and-drag zooms one-handed, two-finger tap zooms out, tap selects, a group's name or chip flies there); from 2.5× the nearest dots show their names; + · − · Whole map bottom-trailing; VoiceOver actions; the web's `lib/Cloud` is SVG with the same camera (drag, wheel / trackpad pinch, two-pointer pinch) at ≤ 30 fps; `lib/PanZoom` is no longer used by the cloud (the galaxy keeps it) with the older links **galaxy map** (`/map`: clusters by likeness, edge kinds by
  colour and dash, status by shape, shuffle / centre / find / legend isolate); write + riff; merge same-idea notes.
- **Sets**: segments **Set lists · Recorded** (a set exists once it is taped); set lists (boards) with timing, gaps, bridges; **Record** (`/stage/[id]`, the list's prominent action) and the free recorder (`/record`) share the Voice Memos control (`lib/RecordControl`, 2026-10-29): nothing tapes until the red disc, the square stops and saves, Discard behind Close.
- **Voice memos and on-device transcription (2026-10-31, committed, not yet in a build)**: Tonight ⋯, a bit's ⋯ and
  Bits › Memos' mic open `/record?kind=memo`, which starts recording as it opens (a memo started on a bit is filed on
  that bit). The audio stays in the phone's Documents (`lib/recordings.native`: one index, file names only); the phone
  transcribes it (`lib/transcribe.native`, Apple's on-device recognizer only, never its servers) and sends only the
  words: a memo → `POST /api/ideas` (auto-filed, or `merge/undo` + `merge/attach` onto the bit it came from), a taped
  set or a memo over 4000 characters → `POST /api/sets/transcript`. Record and Stage now keep the set on the phone
  instead of uploading it. Rows (`lib/RecRows`) in Bits › Memos, on a bit's page and in Sets › Recorded › On this
  iPhone show the state, play the audio, Delete (confirms), Not this bit, Try again. All rules in
  `rules/memoRules.ts` (tests/memoRules.test.mjs). **No server change.** Laughs still need audio processing (item 6).
- **Recorder** (sets and memos; jots and questions are text): 48 kHz stereo AAC .m4a, keep-awake while taping, real dBFS
  meter on Record and Stage, a phone call pauses the clock ("Paused by a call", 60-pt Resume), a media-services reset
  keeps the partial file, built-in mic preferred over a Bluetooth headset mic. Logic in `app/src/lib/recorderLogic.ts`
  (32 unit checks). Measurement mode / AGC off needs a config plugin and is not done.
- **Settings › Voice profile**: his hand-written profile (edit, save, undo) and **"What it has learned"**.

## What learns (built 2026-10-28)

Before today the voice profile was a static document and nothing he did with drafts reached a prompt. Now:

| Signal | Where it is recorded | How it reaches the model |
|---|---|---|
| Riff / punch-up line he keeps ("Use as mine", including one he edited first), an answer kept as a line on the bit page (`draft: true` on `POST /api/jokes/<id>/versions`) | `profile:voice.learned.kept_lines` (cap 25) via the `draft: true` flag on the save | second cached system block after the profile, writing tasks only |
| Line he dismisses | `.dropped_lines` (cap 25) via `POST /api/profile/voice/learn` | same block ("lines he passed over") |
| Laugh marks from tapes | `.laugh_lines` (cap 15) | same block; **empty until audio processing runs on the hosted worker** |
| Locks on a set | board state | already in the set-analysis prompt |

Code: `creative/learned.py`, `creative/voice.py` (`learned_block`, `cacheable_learned`), `api-dev/profile_api.py`,
`app/src/lib/simple.ts` (`saveIdea`, `dismissDraft`), `app/src/app/(tabs)/(tonight)/learned.tsx`. The block is only
cached as a prefix together with the profile (alone it is under the 512-token cache minimum).

## Where this is going (owner, 2026-10-28): organizing ideas

The unit he thinks in is the **premise** (a handle like "raccoons"), not the note. Built so far: search over
everything, the workbench on a bit (edit as versions, a conversation about the bit, merge and link from its page),
"make these one bit" off a search list, his own attach (`POST /api/merge/attach {note, into}`, undoable; not yet in
the UI). Still to build: a note under several premises (attach is one-to-one today), "add to this bit" from the
search panel, the quiet "goes with X?" suggestion after a jot.

Found 2026-10-29 while building it: the decision builder refused `type: unit` adds, so Brainstorm's "Use as mine"
(`bit/[id].tsx`) had been failing with "you can add a label or a joke/idea"; fixed (units can be added, with a
`part_of` edge as the next decision). Also a merge used to lose a typed idea's words (no unit to re-point); it now
writes them as a version of the main first.

## Standing owner decisions (do not re-open)

- **The services are Rust from here (2026-10-29).** New services in Rust; the Python API, worker and engine are
  ported in the order in `docs/RUST.md` (worker first, then API reads by route group, then every write at once,
  then retire Python), each step gated by a differential against the Python it replaces (parsed JSON, not bytes). Owner's rule: popular crates (the official `surrealdb` crate, `jsonwebtoken`, `prometheus`, `sentry`, `jsonschema`, `ort`), never hand-rolled clients or codecs. The app stays
  TypeScript (React Native); Rust goes on the phone only as a library when a feature needs it.

- Phone + frontier models only. The home PC / GPU worker is retired, never proposed.
- TestFlight-only distribution; email-only sign-in on the phone; Google sign-in on the web only.
- Jots and questions are typed. Voice records sets AND voice memos (owner 2026-10-31, replaces "voice is for sets only"):
  memos are recorded in Punchable, the audio stays on his phone, the words go to the server. A cloud backup of the
  audio comes later as a paid add-on, so the same recordings play on the phone and the web/desktop (the words already
  show everywhere). The memo recorder and on-device transcription are built (2026-10-31, item 4); the paid backup is not.
- HIG first on iPhone: standard, familiar, one-handed in the dark. Dark default, light must work.
- No review step is ever required; AI inferences show as the current state with an AI badge and he edits.
- No Sentry SaaS: crash reports go to the self-hosted Bugsink.
- Nothing private in this public repo (material, hostnames beyond punchable.io / auth / bugs, secrets, other projects).
  The commit guard blocks it; fix the content, never bypass.
- Court documents are never ingested; the legal rules run on every ingest and request path.

## How to work (the short version)

```bash
# gates before any commit (all must be clean)
python -m pytest creative engine gsync worker infer/tests tools/audio/tests api-dev/tests -q
cd app && npx tsc --noEmit && npm run test:unit && npx expo-doctor
# ship (from the repo root; each step logs to the private deploy log with a rollback line)
bash infra/ship.sh build api && bash infra/ship.sh rollout api      # same for worker, web
# phone
cd app && EAS_NO_VCS=1 npx eas-cli build --profile production --platform ios --non-interactive --auto-submit
EXPO_PUBLIC_API_URL=https://punchable.io/api EXPO_PUBLIC_AUTH=1 npx expo start --dev-client --tunnel
```

Commit with the owner's git identity (the one on the existing commits, passed with `git -c user.name=… -c user.email=… -c core.autocrlf=false commit`),
stage paths explicitly (never `git add -A`), message ending with a `Co-Authored-By:` line for the agent that helped.

## Traps that cost time (each happened at least once)

| Trap | Rule |
|---|---|
| A Python `set` on the path to a golden | Its order follows `PYTHONHASHSEED`, so the regenerated golden differs run to run and CI's `goldens-current` goes red while the gate stays green (`kb.py joke_scope`, 2026-10-29). Sort every set in the generator; where Python's order is the behaviour, normalise on both sides and say so. |
| Saving a file on this Windows checkout (`core.autocrlf=true`) and committing with `autocrlf=false` | Stores CRLF and turns a 100-line change into a whole-file diff (three lanes did it on 2026-10-29). Write every file with LF (`newline="
"`); the root `.gitattributes` (`* text=auto eol=lf`, 2026-10-29) now normalises on add. In a shared checkout commit by pathspec (`git commit -- <paths>`), never a bare commit over others' staged files. |
| `python -I` on the owner's PC | Cannot see the user site (numpy lives there): run the golden generators with plain `python`. |
| SurrealDB: selecting a database before `DEFINE DATABASE … STRICT` | Creates it lax, and `IF NOT EXISTS` then leaves it lax for good: an unknown table becomes a silent create instead of an error. `db/load.py` and `pk-store`'s `Store::connect` define first, then select. |
| EAS packs the repository root under `EAS_NO_VCS=1` | `rust/target` (GBs) went into the tarball and filled the disk (ENOSPC); builds 13-15 died uploading. `.easignore` at the root keeps everything but `app/` out. |
| A nested `'worklet'` function writing an outer `let` | The worklets plugin hands a nested worklet a COPY of the outer variables: `out[k++]` in a helper never advanced the caller's `k`, so the cloud's name picker never stopped at 30 (iOS dev review, 2026-11-02; node tests cannot see it, they run without the plugin). One directive per top-level function in `rules/`; inner helpers are plain closures or inlined. `tests/cloudMath.test.mjs` pins both. |
| A worklet naming a function declared further down its file | The plugin builds each worklet's closure when the module loads, so the later name is captured as `undefined`: the cloud's `resolveTap` would have thrown on the first tap off a dot (iOS dev review, 2026-11-02). Declare before use. `tests/static.worklets.test.mjs` compiles `src/rules` with the real plugin and fails on any `undefined` capture. |
| `useFrameCallback(cb, flag)` with a changing flag | The flag is read only when the callback first registers (`react-native-reanimated/src/hook/useFrameCallback.ts`): the cloud passed `turning`, false on the first render, so on the phone the slow turn most likely never ran (builds 17-18, INFERRED). Pass `false` and call `setActive(flag)` in an effect; memoise the callback (a new one re-registers each render and pauses a frame). |
| A plain function called inside a Reanimated / gesture worklet | Crashes the app on the UI thread ("[Worklets] Tried to synchronously call a Remote Function"): the map's crash on build 16 was `clampScale` in the pinch gesture (Bugsink). Every function a worklet calls carries the `'worklet'` directive, or is inlined. |
| Two uploads of one recording in the same second | Shared `<dest>.part` and the loser's `os.replace` raised FileNotFoundError (Bugsink 2026-10-29). The upload route reserves the name with an exclusive create and writes to a private temp. |
| iOS 26 header is transparent by default (expo-router ≥ 58.0.20 with Liquid Glass) | A screen that brings its own body (`scroll={false}`: Bits, the maps) sets `headerTransparent: false` or the list runs under the bar and the search field floats over the rows (`lib/Screen.native.tsx`, 2026-10-29). Search fields: `placement: 'stacked'`. |
| A `requestAnimationFrame` loop re-rendering 1,600 SVG dots every frame | Starves the JS thread: taps and toolbar buttons stop landing, the screen reads as dead. Throttle to ~30 fps, round path numbers, stop under a pushed screen (`lib/Cloud.tsx`). |
| `*.native.tsx` importing its own basename | Base + shim + `.native`; never import `./<own name>` (ENGINEERING.md §1; static tests pin it). |
| Worktree with an `app/node_modules` junction | Delete the junction FIRST (PowerShell `(Get-Item <path> -Force).Delete()`), then `git worktree remove`, then `git branch -d`. Both `Remove-Item -Recurse` and `git worktree remove --force` follow the junction into the real `node_modules` (restore: `npm ci`). |
| `ship.sh rollout <svc>` on the same image digest | Does not restart pods. After a Secret change run `kubectl rollout restart deploy/<name>`. |
| A Job that opens Bugsink's SQLite as another uid | Creates `-shm`/`-wal` files Bugsink cannot write ("readonly database"). The backup Job runs as Bugsink's uid; `infra/test_k8s.py` pins it. |
| `npx eas …` | "could not determine executable": use `npx eas-cli …` with `EAS_NO_VCS=1`. |
| The DNS tool's plan vs its docstring | A record must be in `WANT` in `cf_dns.py`, not only in the comment. |
| Hand-listed modules in `api/Dockerfile` | Glob the directory and keep the build-time import-all check. |
| Shared Keycloak rolling update | Cannot surge on this node: a theme change is a short outage for every realm. Needs the owner's go. |
| Never on the server | `docker run` (restarts the whole cluster), `ship.sh test`, touching other namespaces beyond the agreed Keycloak theme mount and the observability ConfigMaps. |

## Open, in order

0a. **Owner checks on the build after 20: the 3D cloud map.** Bits › Map:
   - It turns slowly by itself (on 17-18 it most likely never did) and stops when you touch it.
   - Drag one finger: it turns any way you push it and keeps coasting after a flick.
   - Pinch into a crowded spot: dots and names stay **sharp**, the spot opens up, and from about 3× the nearest dots
     show their names.
   - Double-tap zooms in where you tap. Double-tap and keep your thumb down, then slide up or down: one-thumb zoom.
     A two-finger tap zooms out.
   - + and − (bottom right) zoom too; + greys out at the closest zoom and − at the widest. A double-tap at the closest
     zoom jumps back to the whole map.
   - Tap a group's chip or its name: it flies there. **Whole map** (bottom right) brings it all back.
   - Tap a dot: its card. Tap it again, or Open: the bit.
   - **Tell us**: smooth or stuttering while you pinch (it draws every bit live; this is the part we're least sure of);
     whether a single tap feels slow to select (it waits a moment in case it is a double tap); whether the names are
     readable in light mode. On the web: drag, the scroll wheel or a trackpad pinch, Whole map.
0b. **Owner checks on build 18** (2026-10-30): (1) Sets tab: only his lists as paper cards; one row "N drafts from your
   Insights" at the bottom opens and remembers its state; toolbar ⋯ → Archived. (2) A draft: swipe → Archive, snack, Undo
   brings it back; "Use this" makes it his (snack offers Open, there is no server op back to draft yet); "Archive all
   drafts", Undo, Archived → Unarchive; on a bit page "To a set" with no list open shows the picker (his lists only).
   (3) Notes on stage: open a list → Record → disc; tap a bit row → a half-height sheet with setup, reveal (or "Idea"),
   the stage line, the clock still running above; the sheet's "Next bit" advances behind it ("Last bit" at the end);
   swipe down; the dock's Next still works; the square saves. Server side is live: `PATCH /api/boards/<id>`
   `{"archive": true}` / `{"unarchive": true}`, `GET /api/boards` hides archived (`?archived=1`), each build step
   carries `text` + `idea` (commit `9ee66d0`, API image rolled out 2026-10-30; web image too).
0. **Owner checks on build 17** (2026-10-29: the Skia cloud, the pinch crash fix, "+" opens a document; 16 had the build-12 fixes and crashed on a pinch in the map): Bits › scroll (the search field stays put, nothing
   passes under it); Bits › Map (dots turn, a tap selects within a second, Links from the skeleton); Sets › Record ›
   the disc ("Starting…" then "Recording", the meter moves on a clap, the square saves). Still from 12: edit a bit's
   text › Save; ask "what else do I have like this?" in a bit's bar; search › Select › Make them one bit. From 10:
   which search results feel wrong. **Two answers wanted**: on 12, did the map show a spinner, an error, or dead dots?
   Did recording's clock run after the disc, or did a snack say it couldn't save?

1. **Owner checks on build 9**: tape a short set from Record, clap near the phone and watch the meter; riff a bit, keep
   one line and dismiss another, then Settings › Voice profile › "What it has learned" should list both; Bits › Map.
2. **"go ksm"**: deploy kube-state-metrics and a cert-manager scrape in the shared observability namespace so the two
   dead alerts work (config reload, no app downtime).
3. Worker crash reporting is live; still unproven end to end: run the worker command under "Verifying a crash lands" in
   `infra/OBSERVABILITY.md` once and see the event in Bugsink.
4. **Transcription is on the phone now (built 2026-10-31, committed locally, NOT pushed, NOT built; was: recorded
   sets were a dead end since 2026-10-27).** Spec agreed by the iOS dev and design experts, both reviewed the diff and
   their findings are fixed (`docs/design/SPEC-2026-10-31-voice-memos.md`). Route: `expo-speech-recognition` 58.0.1
   (SFSpeechRecognizer, `requiresOnDeviceRecognition`; the JS refuses to start unless on-device is supported, because
   the package would otherwise fall back to Apple's servers silently). **Uncompiled until the EAS build** (no Mac).
   Owner checks on that build, in this order: (1) Tonight ⋯ › Record a memo: it records at once; the square saves;
   Bits › Memos shows "Transcribing…" then "On <bit>" or "New idea"; the play button plays it through the speaker;
   (2) the first transcription asks for Speech Recognition once; (3) a bit's ⋯ › Record a memo files it on that bit;
   (4) tape a 1-2 minute set from a set list: Sets › Recorded › On this iPhone shows the percent, then "Sent · finding
   the bits", and the set appears; (5) lock the phone mid-transcription of a set and come back: "Paused", then it
   restarts; (6) Settings › Connections › Transcription reads "On this iPhone". Unverified until then: the pod
   compiles; a 48 kHz stereo file transcribes; timestamps across long files; a 20-45 min set's time and memory
   (a jetsam kill shows in Settings › Privacy › Analytics, not Bugsink). Still waiting on the server and saying so:
   YouTube links, older uploads, sets taped in the web app. Follow-ups: spec §10 (locale patch, idea de-dupe key,
   Voice Memos share extension, paid audio backup, quick action / Control Center, Recently deleted, background
   processing task, marks on the transcript route, linking a phone tape to its server set for playback).
5. **The architecture is signed off (owner, 2026-10-29 evening: "keep it going and all these fixes in") and the first
   six lanes are in: `docs/ARCHITECTURE.md`** (reviewed by the Punchable, iOS dev and iOS design experts, corrections
   marked [review]), the decisions in `docs/adr/`, the index `docs/README.md`, `CLAUDE.md` for agents, a PR template.
   Defaults taken on his five calls: `venue_short` keeps the app's exact semantics (no visible change; the server's
   `venue_label` reproduces `laughs.ts venueShort`, pinned by `contracts/venue_short.json`), "Reconnect Google" is
   web only, the Google Docs sync stays on the PC until its own ticket, the design values go in through
   `docs/design/DESIGN.md`. Landed that night, all on `main`: A0 `DESIGN.md`; T16 the API sends `venue_short`,
   `venue_guessed`, `when_label`, `at_label`, `short_name`, `by_kind` on list rows + `contracts/` (**the API image is
   NOT yet rebuilt or rolled out with it**: `ship.sh build api && rollout api` is the next production step); A1 the
   app's `src/rules/` layer with one `format.ts` and one `isQuestion`; A2 the upload fix (a 401 no longer deletes a
   recording; `X-Idempotency-Key` on every upload); A3 `tokenMachine`, one `useCodeExchange`, Reconnect Google on the
   web; T14 honest CI (`PK_REQUIRE_EXTERNAL=1`, pinned sha-checked `surreal` + cached bge in `rust.yml`, `cargo deny`
   with the ADR 0010 bans, the oracle's Python suites, ruff E9/F, an ESLint ceiling of 58/49); T12 `pk-core`
   (24 copies deleted, 14 env reads out of libraries, a CPython golden); T13 `pk-wire` (198 types, 99 goldens off the
   Python API, the T16 fields). CI green on every push; the full gate (`PK_REQUIRE_EXTERNAL=1 cargo xtask gate`)
   green on the merged tree with the database and model tests run. Next, in the ARCHITECTURE.md §8 order: T17 the
   load script + the Python baseline numbers, T15 the `pk-embed` memory measurement on the server, A4 `kv` + fonts
   (one TestFlight build), A5 `Input` + tokens + the `static.values` gate, A6 `ScreenBase`/`DragListBase`, A7 the
   folder move, then **T8 the Rust worker** (the first thing that ships), T9, T10, T11.
   **Rust port** (`docs/RUST.md`, the board `docs/RUST_WORKSTREAM.md`): nine crates are in and golden against the
   Python: `pk-core`, `pk-wire`, `pk-kb`, `pk-creative` (store + reducers + verbatim + `ask`), `pk-llm`, `pk-embed`,
   `pk-obs`, `pk-store`. **`pk-api` T9 first group (2026-11-02, branch `rust/t9-api-reads`, merged into main locally,
   not deployed):** six GETs (health, labels, decisions, meta, pipeline, learned) answered from the database snapshot
   behind the Python's exact auth, CORS and errors, everything else forwarded to the Python; 57 of 57 requests equal
   against the hosted Python over one scratch database (`rust/tools/diff_api_reads.py`, empty allow-list), the
   read-after-write gate met; the Ingress change is prepared, not applied (`infra/k8s/prepared/t9-api-rs.yaml`).
   Trap found: never clone `pk_store::Store` per request (a clone is a new session, ~17 ms a query). Second group
   (same day): `/api/map/cloud` in Rust (67/67 equal, numpy's random stream ported bit for bit) and an incremental
   snapshot patch after writes; under a write-heavy load the Rust reads still rebuild ~130 ms per write, so
   `/api/health` is slower than the Python's there (RUST_WORKSTREAM T9 row has the numbers and the next fixes).
   **`pk-worker` (2026-11-02, branch `rust/t8-worker`, merged into main locally, not deployed)**: the loop and six of the nine hosted kinds (ask_chat, facts, set_text, riff, punchup, bridge) over
   a `pk-creative` port of the notebook `Corpus`, proven equal to the Python worker over 29 recorded scenarios (the learned block second
   on every writing request); per job 3.6 ms vs 580 ms and 35 vs 95 MiB idle on the PC (mock model). **It cannot replace the Python
   worker yet**: `embed`, `memolink`, `ask` remain (T8 row). Image and manifest prepared (`rust/Dockerfile.worker`, `infra/worker/rust/35-worker.yaml`), not applied. Gate: `cd rust && PK_REQUIRE_EXTERNAL=1 PK_BGE_DIR=<model
   dir> cargo xtask gate` (the model: `python tools/kb/fetch_bge.py --dest <dir>`; `surreal` 3.3.4 at
   `~/.punchable/bin/`). Nothing ships until the worker swap (T8).
6. Laugh detection on the hosted worker (audio processing returns `laugh_lines`; today the set's laugh map is frozen).
7. Measurement mode / AGC off (expo-audio config plugin), only if a real tape sounds pumped against a plain Voice Memo.
8. Source maps for app crashes (EAS secret `SENTRY_AUTH_TOKEN`, drop `SENTRY_DISABLE_AUTO_UPLOAD`).
9. Move the Google Docs sync off the PC (last PC dependency). Its tests (`gsync/tests/test_auth.py`) need the
   google-auth library, which this PC's Python no longer has: four of them fail locally for that reason alone.
10. `docs/design/ARCHITECTURE.md` performance items (fonts, cold start, storage on iOS); prompt-audit items 6 / 8 / 9.
11. Pin the Bugsink image by digest.
12. Riff eval: five BEFORE/AFTER pairs await the owner's judgment (private folder).
13. **The upstream engine fork (2026-11-02, branch `rust/engine-fork`, not pushed, not wired)**: `pk-engine` (ADR
   0013, `rust/crates/pk-engine/FORK.md`), the one verbatim gate, the engine's store seam in `pk-store`, its schema
   (applied only on an explicit call; never on the hosted database without his go). **Its offline proof did not pass**:
   right bit within 24 places on 18/24 questions vs Search 24/24 and Ask 19/24; 175 ms vs Search's 34 ms
   (`docs/RUST_WORKSTREAM.md` T19). **Run 3 did not pass either**: on a held-out set of 22 questions written and
   pre-registered before any change, the candidate (SurrealDB's own fusion as the order) found the right bit within
   5 / 24 places on 18 / 20 questions vs Search 20 / 21, at 262 ms vs 33 ms. Ranking changes stopped there. The security
   re-review's schema findings are fixed and proven on a scratch server; what the database cannot prove with one shared
   login (`signed`, snapshots, promotes) needs per-role logins before any wiring. The private questions, gold and
   results are in his private data folder.

Done 2026-11-02 (local commit, not pushed): the cloud map as a real 3D view (orbit, camera zoom, fly to a group, bit
names when zoomed, Whole map, VoiceOver actions), and two latent phone bugs it found: `wrapAngle` called from the turn's
worklet without `'worklet'` (the build-16 crash class) and the slow turn that never started. Done 2026-10-31 (local commits, not pushed): voice memos recorded in Punchable, on-device transcription of memos and
taped sets, the copy that said "transcription happens on your phone for now" or sent him to Drive and the PC. Done 2026-10-29: the bit workbench (edit as versions, the bit's conversation, merge/link from its page, Select → one bit), the Voice Memos record control, Set lists · Recorded, Bits Recent/+/sort, the cloud map, Punchable's own alert route, the drop route for dead jobs. Done 2026-10-28: worker crash reporting wired and rolled out; `holdToTalk.native.ts` removed (the phone's input button
now only focuses the field, jots are typed); search over everything shipped (API, web, worker, TestFlight 10).
