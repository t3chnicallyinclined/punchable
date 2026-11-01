# SPEC 2026-10-31: voice memos, and on-device transcription for memos and sets

Status: build spec, agreed by the Punchable, iOS dev and iOS design experts before the build (both iOS experts:
"AGREED" with corrections, folded in below and marked [review]). Owner, 2026-10-31:
voice memos are recorded in Punchable; the audio stays on his phone; only the words go to the server; a paid cloud
backup of the audio comes later (not built, not designed out). This also closes `docs/HANDOVER.md` open item 4: nothing
transcribed a taped set. Tags: **CONFIRMED** = read at `file:line`, in the package source, or on the Apple page cited;
**INFERRED** = judgement; **UNVERIFIED** = only a device build can settle it. HIG pages are
`https://developer.apple.com/design/human-interface-guidelines/<page>` (written `HIG/<page>`).

**Claim: one on-device transcriber (a maintained Expo module over Apple's on-device recognizer) turns every memo and
every taped set into words on the phone; memos become ideas filed on a bit through routes that are already live, sets
go to the already-live `POST /api/sets/transcript`. No server change. The audio never leaves the phone.**

## 1. The transcription route (iOS dev expert)

| Decision | Why | Tag |
|---|---|---|
| `expo-speech-recognition` pinned to **exactly `58.0.1`** | Versioned for SDK 58 (devDeps `expo ~58.0.11`, `react-native ^0.88.0`); its `ios/` and `app.plugin.js` are byte-identical to 57.1.1, which ran on SDK 57. Not in `bundledNativeModules.json`, so pin by hand. | CONFIRMED (`npm view`, `diff -r`) |
| File transcription: `start({ audioSource: { uri }, requiresOnDeviceRecognition: true, addsPunctuation: true, maxAlternatives: 1, continuous: false, interimResults: false, lang: <device locale> })` | Events `result {isFinal, results[0].transcript, results[0].segments[{startTimeMillis, endTimeMillis}]}`, `error {error, message}`, `nomatch`, `end`. | CONFIRMED (`src/ExpoSpeechRecognitionModule.types.ts`, `ios/ExpoSpeechRecognitionModule.swift:459-518`) |
| Not SpeechAnalyzer (iOS 26) | No maintained package with timestamps (one is a week old with one maintainer); a local Swift module would be uncompiled until the EAS build. Revisit only if the device test shows SFSpeech failing on long or noisy tapes. | CONFIRMED (package survey); decision INFERRED |
| Long sets | WWDC19 session 256: the one-minute limit is the server's; "with on-device recognition, these limits do not apply". The package sets no limit and feeds 4096-frame chunks with a 15 ms pause. | INFERRED (not in the current reference docs); UNVERIFIED for 45 min |

**Four traps in the package, each guarded in JS** (`app/src/lib/transcribe.native.ts`):

0. **[review] The speech prompt never fires by itself**: with `requiresOnDeviceRecognition: true`, `start` skips the
   speech authorization check (`ExpoSpeechRecognitionModule.swift:207-214`). We call
   `requestSpeechRecognizerPermissionsAsync()` (`:164`) and then the microphone permission first; denied →
   `needs_permission`, and `start` is never called.
1. **Silent server fallback** (`ExpoSpeechRecognizer.swift:588-590`: the on-device flag is applied only if the
   recognizer supports it). We call `supportsOnDeviceRecognition() && isRecognitionAvailable()` first and never `start`
   when either is false: the row says "Turn on Dictation to transcribe". **[review]** Both build the recognizer from
   the device's locale (`:352-364`), not our `lang`, so `lang` is the device's own locale (`Intl` resolved locale,
   `en-US` if none): the recognizer checked is the one used. (`getSupportedLocales` has no on-device list on iOS: both
   its fields are `SFSpeechRecognizer.supportedLocales()`, `:393-402`.) A locale the recognizer does not know fails
   closed (`language-not-supported`). The audio never goes to Apple's servers.
2. **No background handling.** File recognition is neither playing nor recording, so iOS suspends it in the background.
   While a transcription runs the screen is kept awake (`expo-keep-awake`, its own tag); on `AppState` → background we
   `abort()` and put the job back in the queue; on → active it starts again from the beginning (the package cannot
   start at an offset). A 2-minute watchdog aborts a stalled run.
3. **An unreadable file raises no error.** We check the file exists before `start`, and pass only a clean `file://` URI
   (the Swift force-unwraps `URL(string:)`, `:234`). **[review]** An imported file is copied into the app under a
   generated name (its own name may hold spaces or characters that crash that unwrap); recorder names are already safe.
4. **Several finals per long file.** Each utterance arrives as its own `isFinal` result (a leading space from the
   second on; a possible empty or repeated last final). The pure rule `rules/transcriptRules.ts` keeps each non-empty
   final once, as one segment `{text, start, end}` in seconds; whether timestamps restart per chunk is UNVERIFIED, so
   a chunk that starts before the previous end is shifted to follow it. More than 5000 segments (the API's cap) are
   merged pairwise until they fit.

Error map (CONFIRMED `Module.swift:531-607`) → row state: `not-allowed` → needs permission; `service-not-allowed` /
on-device unsupported → unavailable (Dictation off or no on-device model); `language-not-supported` → failed with its
own words ("Speech in this iPhone's language isn't available") [review]; `no-speech` / `nomatch` / zero finals →
no speech; `busy` / `network` (the speech daemon lost) / our own abort (background, watchdog) → back in the queue;
`audio-capture` (a bad source, code 203, unknown), `interrupted`, an `aborted` we did not cause, anything else →
failed (audio kept, Try again), never a retry loop [review]. A cancel (code 301) sends only `end`, so zero finals
after our own abort is never counted as no speech [review]. The file path never touches `AVAudioSession`
(`:337-342`), so the recorder's audio mode is untouched. Web: `transcribe.ts` throws "iPhone only" and never imports
the package (platform split, `docs/ENGINEERING.md` §1).

Info.plist (the package's config plugin): `NSSpeechRecognitionUsageDescription` = "Punchable turns your memos and sets
into text on this iPhone so it can file each idea on the right bit. Only the words leave your phone."
`NSMicrophoneUsageDescription` (both `ios.infoPlist` and the expo-audio plugin, the old string said "Audio goes only to
your own server", now false) = "Punchable records your sets and voice memos on this iPhone. The audio stays on your
phone." The package also asks for the microphone when it transcribes a file (`:217-223`); it is already granted by
then.

## 2. Where the words go (Punchable expert; no server change)

| Recording | Words | Route (all live today) |
|---|---|---|
| Memo, ≤ 4000 characters | an idea, auto-filed on the bit it clearly belongs to (`merge_api.after_idea`, undoable) | `POST /api/ideas {text, from: {note: "Voice memo, <date>"}, reason: "[memo] …"}` (`api-dev/server.py:1987`) |
| Memo started on a bit's page | the same idea, filed on **that** bit: if the auto-attach chose another bit it is undone first (`POST /api/merge/undo {merge_id}`), then `POST /api/merge/attach {note, into}` (his call, `merge_api.py:210`) | the same, plus attach |
| Memo over 4000 characters | sent as a set transcript (the split step finds the bits in it) | `POST /api/sets/transcript` |
| Taped set (Record, Stage) | the text-only set analysis on the hosted worker (`set_text`; bits and ideas; laughs and timing marked unavailable) | `POST /api/sets/transcript {segments, title, recorded_at, duration_s, engine}` (`creative_api.py:1360`) |
| "Not this bit" on a filed memo | detached; it stays a new idea | `POST /api/merge/undo {merge_id}` |

Set audio is **no longer uploaded**: the upload queued a `process_set` that nothing runs (ADR 0012) and fired the
alert. The tape stays in Documents next to the memos. The web recorder keeps uploading (it cannot transcribe a file);
its copy says the tape waits. A repeated set post is idempotent on the server (`set_text|<digest>`); a repeated idea
post after a lost answer could duplicate the idea (INFERRED, rare; the local row stores the idea id once it has it).

## 3. On the phone: one local store

- Files: `Documents/recordings/<iso>.m4a` (where the recorder already writes, `lib/recorder.native.ts`). The index is
  `Documents/recordings/index.json`: one row per recording `{id, kind: memo|set, file (name only), recorded_at,
  duration_s, bit?, board?, title?, state, progress?, text?, idea_id?, merge_id?, filed_on?, error?, attempts,
  next_at}`. **File names only, never absolute paths**: the app container's path changes across updates (INFERRED,
  standard iOS behaviour); the absolute URI is rebuilt from `documentDirectory` at use.
- States (pure, `rules/memoRules.ts`, tested): `queued` → `transcribing` → (`no_speech` | `needs_permission` |
  `unavailable` | `failed`) or `words` → `sending` → `filed` / `new_idea` / `sent` (sets, long memos); a send with no
  signal goes back to `words` with a backoff (`rules/uploadsCore.backoffMs`). One transcription at a time; memos before
  sets; oldest first. The queue runs on launch, on return to the foreground and after each save.
- Cloud backup later: each row already names its file and carries the server ids, which is what a sync needs; nothing
  here assumes the audio is phone-only forever.

## 4. Start points (iOS design expert)

| Where | Control | Cite |
|---|---|---|
| Tonight toolbar `⋯` | **"Record a memo"** (`mic`), first item, above New set list · Record a set · Import a recording… | HIG/toolbars: a More menu for additional actions, don't crowd the bar (CONFIRMED) |
| Bit page `⋯` | **"Record a memo"** (`mic`), first item; opens `/record?kind=memo&bit=<id>&title=<short name>`; filed on this bit | HIG/pull-down-buttons |
| Bits › **Memos** segment | toolbar `mic` button "Record a memo" in place of the sort menu (sorting is for bits) | HIG/toolbars |

"+" stays New bit everywhere (SPEC 2026-10-30). Phone only: the web has no memo recorder (it cannot transcribe).

## 5. The memo recorder

- The same `/record` screen and control (`lib/RecordControl`), full-screen modal (HIG/modality, CONFIRMED: a sheet
  could be swiped away by a pocket), with `kind=memo`.
- **It starts recording when it opens** (the tap on "Record a memo" is the decision; Voice Memos starts on its red
  button). The set recorder still waits for the disc (HIG.md §3.9): he opens it before he walks on. `haptics.medium()`
  when recording starts; VoiceOver hears "Recording".
- Differences from the set recorder: title "Memo"; "For <bit short name>" under the clock when started on a bit; no
  target chips, no target bar; no Mark (an empty slot keeps the disc centred); Pause stays (a call pauses a memo too).
  Close while recording → action sheet "Leave the memo?" · "Discard the memo" (destructive) · "Save the memo" · "Keep
  going" (cancel) (HIG/modality: a save option when content could be lost). Footer: "Lock the screen if you like: it
  keeps recording." Disc label while recording: "Stop and save the memo".
- Mic denied: nothing records; "Punchable can't use the microphone." and **Open Settings**.
- Square = stop and save: the file is on disk, `haptics.success()`, the recorder closes, snack **"Memo saved · 0:42"**.

## 6. Where memos are listed

- **Bits › Memos** (the segment was "Notes"; key `notes` kept). Newest first. Row: title = his first eight words
  (`memoTitle`) or "Memo" until there are words; sub line `30 Oct · 0:42 · <status>`; a 44-pt play/pause on the
  trailing side (`play.fill` / `pause.fill`, its own VoiceOver element "Play memo" / "Pause"). Tap a filed row → the
  bit. **Delete** and, on a filed memo, **Not this bit** are on swipe, on a long-press context menu (Play · Open bit ·
  Not this bit · Delete; HIG.md: context menus keep a visible path) [review] and as VoiceOver actions. Delete confirms
  with an action sheet because it cannot be undone (HIG/action-sheets: destructive first) [review]: title "Delete the
  recording?", message by state: filed "The words stay on <bit>."; new idea or sent "The words stay in Punchable.";
  not yet transcribed (waiting, transcribing, failed, no speech) "It hasn't been transcribed. This can't be undone.";
  buttons "Delete the recording" (destructive, first) · "Cancel". A failed row's action is **Try again**. Empty: "No voice memos yet. Tap the mic to record one." A last row **"Import from Files…"** copies
  the picked recordings into the app and transcribes them here.
- Memos from before (the old Drive/PC route, `GET /api/memos`) stay listed under **"Earlier memos"** with their words;
  the Drive/PC steps and the "transcription happens on your phone for now" lines go.
- **Bit page**: a **"Voice memos"** section above Elsewhere with this bit's memos on this phone (the same rows), hidden
  when there are none.
- Playback: `expo-audio` `createAudioPlayer(fileUri)`, released (`remove()`) when it stops or the row unmounts
  [review]; the audio mode switches to playback (`allowsRecording: false`, so it plays through the speaker, not the
  earpiece; no background play) and the recorder sets the recording mode again on every start (today it is cached
  once). Play is disabled while a recording runs [review]. HIG/playing-audio: a note-taking app with a recording mode
  "might switch its category to playback if it lets people play the recorded notes" (CONFIRMED).

Row status copy (HIG/writing: plain, no blame, say what fixes it):

| State | Copy | Row action |
|---|---|---|
| queued | Waiting to transcribe [review] | — |
| transcribing | Transcribing… (memo) · Transcribing · 40% (set) | — |
| words (no signal) | Waiting for a connection [review] | — |
| sending | Sending… | — |
| filed | On <bit> | open the bit |
| new_idea | New idea | open the idea |
| sent (set, long memo) | Sent · finding the bits [review] | — |
| no_speech | No speech heard | play |
| failed | Couldn't transcribe · Try again | Try again |
| needs_permission | Needs speech recognition · Open Settings | Settings |
| unavailable | Turn on Dictation to transcribe · Try again | Try again |

## 7. Sets

- Record and Stage save the tape to `Documents/recordings/`, queue its transcription and close. The queue does not run
  while a recording is running (no transcribing on stage) [review]; keep-awake uses its own tag and is released on
  finish, abort, watchdog and background. Snack: **"Set saved ·
  12:04 · transcribing on this phone"**.
- Sets › **Recorded** shows the tapes on this phone at the top ("On this iPhone") with the same status words and a
  determinate percent (HIG/progress-indicators: determinate when possible), progress = the last final's end ÷ the
  duration. "Paused · finishes when Punchable is open" (no percent) when the app left the foreground mid-way; after
  the restart from zero, "Transcribing…" with no percent until the new run passes the old one, so the number never
  goes backwards [review]. Done rows keep play.
- Marks from the free recorder stay in the local row (the transcript route has no field for them; follow-up).

## 8. Permissions and copy

- Microphone: the system asks on the first recording, as today. Speech recognition: asked on the **first
  transcription**, right after the first memo or set is saved; never at launch (HIG/privacy, CONFIRMED).
- Settings › Connections › Transcription: "On this iPhone · nothing waiting" / "On this iPhone · 2 waiting" / "Off ·
  turn on Speech Recognition for Punchable in Settings". Privacy row: "Audio of your sets and memos" · "Stays on this
  iPhone. It is transcribed here; only the words go to Punchable."
- Replaced: `memos.tsx` STEPS, "How to send them", the "transcription happens on your phone for now" lines;
  `inbox.tsx:173`; `settings.tsx:147`, `:164`; `record.tsx` snacks; `import.tsx:90` fallback.

## 9. Accessibility

VoiceOver: announce "Recording" on auto-start; the clock is not a live region; rows read "Memo, <words>, <date>,
<duration>, <status>"; Delete / Not this bit / Try again exist as accessibility actions (HIG/accessibility:
alternatives to gestures). Dynamic Type: rows wrap; play stays 44 pt; the clock keeps its 1.2 cap. Reduce Motion: the
scrolling level strip hides, the single level bar stays. Haptics: `medium` on start, `success` on save, none when
transcription or filing finishes (he didn't cause it).

## 10. Out of scope (follow-ups)

From the diff review (2026-10-31), deliberately not in this build: **the residual locale gap** (§1 trap 1: if the
runtime's Intl locale and iOS's `Locale.current` ever differ, the on-device check can answer for the other recognizer;
the close is a 3-line Swift patch in `ExpoSpeechRecognizer.startRecognizer` via patch-package, refusing to start when
`requiresOnDeviceRecognition` is set and the recognizer cannot do it; not added before the first build because it
would be uncompiled Swift); a server-side de-duplication key on `POST /api/ideas` (a lost answer after the server saved
the idea makes a second one on retry); the package's file feeder keeps reading after an abort (CPU only, upstream);
`TextBtn` busy state for Import from Files….

Voice Memos share extension; the paid cloud backup of the audio; Control Center / Action button / Home Screen quick
action "Record a memo" (needs a native module); a "Recently deleted" bin for memos (Voice Memos' 30 days; today Delete
confirms instead); `BGContinuedProcessingTask` so a long set keeps transcribing in the background (native code); laugh
detection (needs the audio somewhere that can process it); marks on the transcript route; linking a local tape to the
server's set so the set page plays it; SpeechAnalyzer if SFSpeech falls short on long or noisy tapes; the server's
waiting note for YouTube links still says "on your phone for now" (`creative_api.py` `WAITING_NOTE`).

## 11. Unverified until a device build

The pod compiles on EAS's SDK 58 image; an iPhone on iOS 26 transcribes a 48 kHz stereo AAC file on-device; whether
timestamps run across the file or restart per final; the iOS 18+ one-final-per-utterance behaviour on iOS 26; a
45-minute tape's time and memory (a jetsam kill shows in Settings › Privacy › Analytics, not in Bugsink; the fix is a
larger `chunkDelayMillis`); what happens on lock and return; accuracy on a club recording; playback through
`createAudioPlayer` from a `file://` URI; the auto-start and the haptics on a device.
