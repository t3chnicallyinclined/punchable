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
  **Film it (2026-11-03, item 0d): the video he films stays in Photos on the phone (one copy, add-only access); only
  its words go to the server, through the same routes as a memo's.**
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

0d. **Film it, v1 (owner: "lets get this in", 2026-11-03): `docs/design/SPEC-2026-11-03-film-it.md` (the screens, the
   copy) built on `docs/design/SPIKE-2026-11-03-film-it-feasibility.md` (the technical path; the lead took the spike's
   side on every §9a point).** Branch `app/film-it-v1` (from `7147fda`), **not pushed, not built**; the spec's §11 v1
   "Built" row lists what is in and the ten deviations. The owner rules hold: the video and the audio never leave the
   phone (one copy in Photos, add-only access, no album); the words go to the server through today's routes (an idea
   filed on the bit); the set and memo recorders are untouched. **Two new native modules (`expo-camera`, `expo-video`),
   so the next binary is required**; both are loaded lazily and an older build reads "Not in this build yet.".
   **`app/package-lock.json` changed: run `npm install` in `app/` on main after the merge** (the shared `node_modules`
   has neither module until then). Gates on the branch: tsc, `test:unit` (two new suites, `prompterRules` and
   `filmRules`, each with a `--red` arm shown failing a planted lenient rule), the six static checks, expo-doctor 20/20,
   eslint 54 / 38 (the baseline), guard. **Owner device checks on that build, in the spike's §7.1 order, one line each:**
   - **D1** a 20-second take; open it in Photos: portrait 1080 × 1920, plays, has sound at a normal level with the phone
     at arm's length (the preset, the orientation and the audio input). Two more one-liners from the same take: does
     the review (before Keep) play through the speaker at a normal level? Is the start thud (the `medium` haptic on
     the first frame) audible in the first half-second of the file? (if yes, it goes: the count's last tick is the cue).
   - **D2** a shirt with text: the Photos copy reads correctly (unmirrored); the preview read mirrored (the mirror
     default; whether a "Mirror the take" switch is wanted).
   - **D3** after Keep, the bit's On tape row › the Filmed row: "Transcribing…" then "N words"; `Share captions…` hands
     over an `.srt` with per-word cues of 1-4 words (whether `AVAudioFile` opens a `.mov`: **if a take with clear
     speech reads "no words"**, it did not (the package ends an empty request, which reads as no speech; the console
     shows OSStatus 1954115647 'typ?'), and the 30-line Swift export to `.m4a` is the next step; also whether the
     cues are one word each, which says the segments are per word).
   - **D4** lock the phone at 0:15 of a take; unlock: the review is there with "0:15 · cut short" and the file is whole.
   - **D5** a phone call (or an alarm) at 0:15: the review reads "0:15 · cut short by a call", the preview comes back
     after Redo (what the promise returns; what the status word should say).
   - **D6** a 3-minute take: the file size in Photos (expect about 180 MB), the phone's warmth, the battery drop (the
     1080p / HEVC choice; whether 4K is ever offered).
   - **D7** read a 30-second bit off the band at arm's length, then watch it: does he look at the camera? (the band's
     height, 26 pt vs 34-40, and the "current line at the top" rule).
   - **D8** VoiceOver on: the disc ("Start recording"), Next line, Hold and the band's current line are announced, the
     countdown counts aloud; Reduce Motion on: the lines cut instead of fading.
   Also from the spec's v1 list: the More menu greys "Film it…" with "nothing to read" on an empty bit; Keep with
   Photos denied greys with "no Photos" and Open Settings; in airplane mode the whole thing works and the row says
   "Waiting for a connection"; text size at AX1: the band is two lines.
0c. **The bit page, design option 3c (owner: "lets get it in", 2026-11-03): `docs/design/SPEC-2026-11-03-bit-page-3c.md`.**
   Five slices in its §14, smallest first, each its own TestFlight build: 1 the page app-only + the three device
   checks as a spike screen (bottom toolbar vs the minimising tabs; the glass bar above the keyboard; dictation into
   per-line inputs) → 2 focus mode + the offline outbox → 3 the pile, its pk-api routes (`db/schema_bit.surql`, per-role
   login first), the deck, the capture proposal → 4 line actions and sessions with Claude (needs T8 + pk-api deployed)
   → 5 Gather, overnight filing, the Bits list, desktop focus. The lead's calls on the design's questions are its §1
   (owner may flip); what stays open is its §13.
   **Slice 1 is built (2026-11-03, branch `app/slice1-bit-page`, not pushed, not built; the spec's §14 "Built" row
   lists the twelve calls recorded against the spec):** the 3c page on today's data (`joke/[id].tsx`; new
   `lib/SectionRow`, `BitLines`, `VersionStepper`, `IdeaLine`, `Pile`, `KeyboardBar*`; `rules/bitLines.ts` with
   `tests/bitLines.test.mjs` and its `--red` arm), the On tape screen (`joke/[id]/tape`), the offline/reason footnote
   slot in both `Screen` twins (`offlineAt`, `footnote`), `prefs.tips.pile` / `prefs.focus`, Settings › About (version,
   build) › **Device checks**: the spike (`(bits)/spike`, `spike-write`) for the three §4.1 checks, removed in Slice 2.
   No server change. Gates: tsc, `test:unit`, `static.worklets`, guard green; eslint 54 / 38 (the ceiling, warnings
   down from 42); **expo-doctor 19/20 only because Expo published new SDK 58 patches mid-session** (it was 20/20 with
   the same `package.json` an hour earlier): `npx expo install --fix` is the usual pre-build bump and needs the go
   (lockfile + the shared `node_modules`). Reviewed by ios-design-expert and ios-dev-expert; every must-fix landed.
   **Owner checks on build 22** (the Slice 1 build), in this order:
   - A bit's page, dark and light: the bar is the short name · a greyed sparkle · To a set · ⋯; one muted idea line
     (tap: edit); the lines at 44 pt; the rows Pile · N, Versions · v8 · Nov 2 (· 1:40 when the bit has a run time),
     On tape · 3 nights (· killed twice when rated); **Write** alone at the bottom. Nothing explains itself.
   - Tap a line: it highlights with Move · Cut under it; tap it again: the editor opens with the caret on that line.
     Cut → "Line cut" with Undo, and the Versions row reads "v9 · just now · Undo" (tap Undo there: it comes back).
   - Versions row › the stepper: ◀ steps to v5 (hold to repeat), the lines above re-render, Write greys with
     "Reading v5" above it, "Restore v5 as a new version" appears; ▶ back to v8 wakes Write.
   - More › Reorder lines (or Move under a line): drag by the grip, Done → "Reordered" with Undo. More › Stage ›
     Testing: the snack says "Now testing" and the Bits list's dot follows.
   - To a set from the bar: the snack "Added to <list>" (or the picker when no list is open).
   - Pile · N opens inline: this phone's memos (play glyph) and attached notes; + puts the words after the selected
     line ("Added", Undo); the scrap then reads "in the bit". Pile · 0 opens on "Gather from your notes ›" (the inbox).
   - Airplane mode: "Offline · reading only" sits above Write; Write and To a set are greyed; reading still works.
   - **Settings › About › Device checks**, then tell us which way each went (SPEC §4.1 says what each outcome decides):
     1 the bottom toolbar: scroll down and up on the spike page, tap Write (a snack), note whether the footnote line
     sits just above Write or under it, Back to Bits: is an empty bar strip left behind? 2 the glass bar: open the
     focus mock, tap a line, hide and bring back the keyboard: flush on the keyboard with no gap or overlap, and the
     bar says "Liquid Glass" or "flat". 3 dictation: mic, "line one, new line, line two", stop; then dictate and tap
     another line mid-sentence: read the log's last lines (which path "new line" took, whether dictation died).
   Slice 2 needs from those: check 1 → keep `Stack.Toolbar placement="bottom"` or build the in-page glass bar; check
   2 → KAV + GlassView as drawn, or a `keyboardVerticalOffset`; check 3 → per-line inputs (the settle rule) or the
   one-multiline fallback. Next step: the owner's go to bump the SDK patches, push and build 22.
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
   read-after-write gate met; the Ingress change is prepared, not applied (`infra/k8s/prepared/t10-api-rs.yaml` (renamed from `t9-api-rs.yaml` at T10)).
   Trap found: never clone `pk_store::Store` per request (a clone is a new session, ~17 ms a query). Second group
   (same day): `/api/map/cloud` in Rust (67/67 equal, numpy's random stream ported bit for bit) and an incremental
   snapshot patch after writes; under a write-heavy load the Rust reads still rebuild ~130 ms per write, so
   `/api/health` is slower than the Python's there (RUST_WORKSTREAM T9 row has the numbers and the next fixes).
   **Round 3 (branch `rust/t9-round3`, merged into main locally, not deployed):** fixed: rebuilds coalesced with tickets, views and
   the index built lazily per route, `pk_kb::View::from_records`, records shared between generations; under the same
   write-heavy load `/api/health` is now p50 2.4 / p95 20.0 ms vs the Python's 17.6 / 39.1 (was 144.9 / 211.6), 67/67
   still equal. Trap written down: never serve an older generation to "callers who did not write": the app refetches
   once per `change` event, so a stale answer hides the change.
   **Round 4 (branch `rust/t9-round4`, merged into main locally, not deployed):** pk-api also answers `/api/sets`,
   `/api/sets/<id>/analysis`, `/api/sets/<id>/review` and `/api/jokes/<id>` (98/98 equal, empty allow-list, a
   worker transcript post read back on both servers). Under the write-heavy load on the owner's PC, p95 Python vs
   pk-api: `/api/sets` 4,182 vs 158 ms, `/api/jokes/<id>` 122 vs 2.5 ms, `/api/sets/<id>/analysis` 74 vs 29 ms;
   memory peak 550 vs 105 MiB. Settled: laughs are the database's `laugh` rows, transcripts are
   `srcfile` rows, and the hosted pod has no recordings (`audio` is always `null`, as the Python's). The id pages need a
   regex Ingress (prepared; check ingress-nginx's host-wide `use-regex` first). Not done this round: `/api/jokes` and
   `/api/today` (need the creative Corpus built from `srcfile` rows inside pk-api; `/api/today`'s `inbox` count is the
   Python pod's own upload folder, T10), `/api/search` + `/similar`. Exact next steps: RUST_WORKSTREAM T9 row.
   **Round 5 (branch `rust/t9-round5`, not merged, not deployed):** pk-api also answers `/api/jokes` (the Bits list:
   filters, the attached-note filter over the merges store, the five sorts, each row's duration from the creative
   notebook, which pk-api now builds from the `srcfile` rows once per generation) and `/api/today` (the older feed plus
   `today.build`, with the working-set reads ported into `pk-creative/src/today.rs`). `?q=&mode=hybrid` is still
   forwarded to the Python (it needs the model). 147/147 equal; the only allow-list entry is Today's `inbox` (the
   Python pod's upload folder until T10; the app does not read it). Under the write-heavy load on the owner's PC (another build running, compare within a run), p50 / p95 Python vs pk-api: `/api/today` 3,937 / 4,261 vs 75 / 92 ms, `/api/jokes` 58 / 135 vs 11 / 17 ms; memory peak 519 vs 146 MiB. The still-forwarded creative and search routes were slower behind pk-api in the same run (e.g. `/api/settings` p95 256 vs 522 ms; causes INFERRED, in the T9 row). Not done: `/api/search`, `/similar`,
   `/api/jokes?mode=hybrid` (pk-embed in pk-api); exact next steps in the RUST_WORKSTREAM T9 row.
   **Round 6 (branch `rust/t9-round6`, not merged, not deployed):** (1) **the forward is not what is slow.** Asked the
   same request at the same rate, interleaved (`tools/load/forward_probe.py`), the Python direct and through pk-api
   are within about 2 ms at 1, 2, 4 and 8 in flight, with and without writes (owner's PC). The forwarded routes look
   slower behind pk-api because the Python itself slows with the number of calls it runs at once (`/api/settings`
   p50 3 ms alone, 76 ms at 8 at once; writes add more) and pk-api, answering its own routes in milliseconds, keeps
   the Python's slots full of exactly those calls. The cure is porting them. pk-api now turns Nagle off on its
   sockets (hardening; it changed nothing measurable here). (2) pk-api answers **`/api/search`,
   `/api/jokes/<id>/similar` and `/api/jokes?q=&mode=hybrid`** with bge-small loaded on the first search; the fused
   search starts right after the request's state read and runs while the data is made ready. 260/260 equal against
   the Python, both with the model and with no model on either side. Two findings: SurrealDB's order among equal
   scores is not stable (the Python disagrees with itself on repeated searches; compared up to that order), and
   distances from the two servers' query vectors agree to 1e-5, not exactly. Under the write-heavy load, p50 / p95
   Python vs pk-api: search 114 / 553 vs 49 / 137 ms, hybrid 209 / 303 vs 148 / 229 ms; **`/similar` 71 / 129 vs
   110 / 142 ms: not at or below the Python in that run** (pk-api's run made twice the calls per second); asked at
   the same rate with writes, pk-api is several times faster (`/similar` p50 45-56 vs 294-410 ms). Memory peak with
   the model: 332 MiB vs the Python's 530.
   **Round 7 (branch `rust/t9-round7`, not merged, not deployed):** pk-api also answers **`/api/settings`,
   `/api/boards`, `/api/mysets` and `/api/tonight/screen`** (286/286 equal against the Python, allow-list unchanged;
   equal at production scale too). The gate is now "at or below the Python at the same request rate"
   (`tools/load/forward_probe.py`): p50 one at a time 0.6-2.5 ms vs the Python's 3-42 ms, and at 8 at once with writes
   0.6-2.8 ms vs 93-215 ms. In pk-load (context only) `/similar` is now below the Python too (46 / 72 vs 95 / 197 ms).
   The Tonight screen's worker status lives in the Python process's memory until T10, so pk-api asks a new tiny Python
   route, `GET /api/ai/worker`: **deploy the Python with that route before pk-api moves `/api/tonight/screen`**.
   Also: `rust/Dockerfile.api` (model included, pinned bases; not built here: Docker's engine was not running), and
   pk-embed no longer goes unusable after a panic. A text-keyed vector cache was tried and dropped (no measurable gain).
   Slowest still-forwarded route: `/api/boards/{id}` (next). Details and the deploy prerequisites: the RUST_WORKSTREAM
   T9 row.
   **Round 8 (branch `rust/t9-round8`, not merged, not deployed):** pk-api also answers the rest of the app's
   forwarded GETs: one set list (`/api/boards/{id}`, `/build`), one of his sets (`/api/mysets/{id}`, `/trim`), one bit's
   connections and map, `/api/notebook/related`, a set's clips, his voice profile, the "not a joke?" list, one job,
   and the working sets (list, page, nights, one bit). 432/432 equal against the Python, allow-list unchanged; three
   red arms fail it (28, 41 and 5 requests). The notebook now carries the vectors (parsed once per change). The
   set-list rules moved into pk-creative (`setlist.rs`, `mysets.rs`, `connections.rs`), the round-7 follow-up.
   CI builds `rust/Dockerfile.api` (build only, no push; Docker is never started on the PC or the node). Still
   forwarded: `/api/ask/chats` (a GET that writes), `/api/merge/candidates`, `/api/memos`, `/api/inbox`,
   `/api/venues`, `/api/live`: T10 with the writes. **Deploy order stands: the Python with `GET /api/ai/worker`
   first, then pk-api's Ingress.** Numbers, the review and the prerequisites: the RUST_WORKSTREAM T9 row.
   **T10 round 1 (branch `rust/t10-round1`, not merged, not deployed): the first writes.** pk-api answers **`POST
   /api/ideas`, `POST /api/sets/transcript`** (group 1: what the app does most) and **`POST /api/jokes/<id>/versions`,
   `/api/decision`, `/api/undo`, `/api/merge`, `/api/merge/attach`, `/api/merge/dismiss`, `/api/merge/undo`** (group 2:
   one bit's writes). A decision goes through the same one-transaction append the Python uses, so the database's own
   event bumps the generation both servers' reads and the Python's `/api/live` follow; a creative write creates the
   next `creative_event` row, which the Python's next request finds (its file is extended, never diverged); the job
   queue is rows on both sides, never process memory. Two processes write until T11: each serialises its own writes
   (the Python's `kb.lock` / `cr.lock` order), the unique indexes refuse the second of two appends racing for one
   place, and the loser retries from scratch (a decision rebuilt from a fresh view, an undo's inverse rebuilt, a
   route's own check run again); the Python got the same bounded retry, because before it a lost race was a 500.
   Proof: `rust/tools/diff_api_writes.py` (CI): 720 checks, 0 differences, through pk-api and through the Python,
   the rows, 8 writers at once, no model on either side; two red arms fail it (7 and 6 checks); `tests/writes.rs`
   on a scratch SurrealDB (the raw race, eight racers, four undos of one line). Same rate (`forward_probe.py`, which
   now takes `POST` routes): pk-api at or below the Python on every write route in every phase. Found and written
   down: a failed SurrealDB transaction marks every statement "not executed" and only one carries the cause
   (`pk_store::Store::query` and `db/sdb.py` now report it); under two writers the hosted Python's own
   decisions-count check could 409 (skipped hosted: `decisions_in_db`). The prepared manifest is now
   `infra/k8s/prepared/t10-api-rs.yaml` (the write paths added; deploy order and the rollback in it). Deploy
   prerequisites unchanged plus one: **the Python image with this round's `api-dev` changes goes out before any
   write path moves.** Numbers, the review, the documented differences and the next groups: the RUST_WORKSTREAM T10
   row.
   **`pk-worker` (2026-11-02; all nine kinds merged into main locally, `ask` from `rust/t8-ask`; not deployed)**: the loop and **all nine hosted kinds** over
   a `pk-creative` port of the notebook `Corpus`, creative/merge.py and engine/ask.py (`pk-fts`: SQLite FTS5 + bm25,
   the only C we compile), proven equal to the Python worker over 41 recorded scenarios. On the PC (mock model):
   later Asks 0.29 s vs 1.88 s; idle 13 vs 37 MiB working set; the first Ask after a NEW pod embeds every source
   (5,085 on a production-size synthetic notebook: about 2 minutes on one CPU, measured for the Rust one, INFERRED for the
   Python one on the pod's 1-CPU limit); a container
   restart reads them back from the emptyDir. **Ready for the swap, which is the owner's call**: image (bases pinned
   by digest) and manifest prepared (`rust/Dockerfile.worker`, `infra/worker/rust/35-worker.yaml`), not applied;
   the runbook (build off the node, swap commit, `pk-worker crash-test` into Bugsink, the spend-ledger reset note,
   rollback) is in the T8 row of `docs/RUST_WORKSTREAM.md`. Gate: `cd rust && PK_REQUIRE_EXTERNAL=1 PK_BGE_DIR=<model
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
