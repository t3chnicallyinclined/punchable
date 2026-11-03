# SPIKE 2026-11-03: "Film it" feasibility (front-camera rant, voice-following prompter, word-timed captions)

Status: feasibility spike, 2026-11-03, no app code changed (another lane is building in `app/`). Tags on every
claim: **CONFIRMED** = read at file:line in this checkout or `node_modules`, on an Apple reference page (URL given),
or in a probe that ran; **INFERRED** = follows from those but was not run; **UNKNOWN** = only the phone can answer
(each one has a device check in §7). Throwaway probe code lives outside the repo (§3.3 names it).

Owner rules this spike holds to (`docs/HANDOVER.md` "Standing owner decisions"): on-device recognition only, never
Apple's servers; the audio (and now the video) stays on the phone; only the words go to the server; HIG first,
one-handed in the dark; TestFlight only; nothing private in this public repo.

## 0. The answer in one screen

**Claim: v1 (a steady-scroll prompter over the front camera, captions after the take) is buildable with no native
code of our own: `expo-camera` 58.0.14 records front-camera video with audio, portrait 1080p, HEVC, and the existing
on-device transcription path (`lib/transcribe.native.ts`) can give word timings from the finished file.** Live
voice-following (v2) is the one real unknown: `expo-camera` owns the microphone through an `AVCaptureSession` and
`expo-speech-recognition` opens its own `AVAudioEngine` tap on the same shared audio session; whether iOS lets both
read the mic at once is UNKNOWN until a 20-minute dev-client probe. If it does not, v2 needs a small Swift module (one
capture session feeding both the movie file and the recognizer), and on iOS 26 that module should use the new
`SpeechAnalyzer` / `SpeechTranscriber` (on-device, long-form, volatile results, per-run time codes) rather than
`SFSpeechRecognizer`.

**The recommended v1 path, five lines:**

1. `npx expo install expo-camera` (58.0.14) and add its plugin plus `expo-media-library`'s (`savePhotosPermission`)
   to `app.json`; one dev-client rebuild, one TestFlight build (both are Info.plist changes).
2. A new full-screen modal route `film/[id]` built like `stage/[id]`: `CameraView facing="front" mode="video"
   videoQuality="1080p"` as the backdrop, the Voice Memos record control in the dock (nothing records until the
   disc; the square stops AND saves; Discard behind Close), keep-awake on, `audioBusy.setRecording(true)` while it runs.
3. The prompter band pinned under the Dynamic Island (lens side), 3-4 lines at 34-40 pt bold on a dark scrim, the
   current line at the top (nearest the lens); steady scroll at a words-per-minute he sets, tap = advance a line,
   long-press = hold. The scroll maths is a pure rule in `src/rules/` with node tests.
4. On stop: move the `.mov` out of the cache into `Documents/recordings`, save it to Photos (add-only permission),
   queue it as kind `film` in the existing recordings index; the phone transcribes it (per-word segments kept this
   time) and writes an `.srt` beside it; the words go up through today's idea / set-transcript routes. **No server
   change.** Burn-in is a v1.5 native module if he wants it in-app; the `.srt` plus his editor needs none.
5. Device checks before anything else (§7.1 D1-D8): the saved file is portrait and has audio; the transcriber opens a
   `.mov`; lock the phone and take a call mid-take; the eye line on a real 30-second take.

**Ranked risks** (the full table is §7.4):

| # | Risk | Bites | Tag |
|---|---|---|---|
| 1 | Two microphone clients at once (camera capture session + the recognizer's audio engine) for live following | v2 | UNKNOWN |
| 2 | Any Swift we write compiles only on EAS (no Mac): each iteration is a build | v2, captions burn-in | CONFIRMED precedent (`SPEC-2026-10-31-voice-memos.md`: "Uncompiled until the EAS build") |
| 3 | `AVAudioFile` may refuse a QuickTime `.mov` on the post-hoc transcription path | v1 captions | UNKNOWN (30-line Swift fallback) |
| 4 | Word-level timing: `SFTranscriptionSegment` is "a word or group of words"; per-word is usual but not promised | v1 captions | INFERRED |
| 5 | Storage: about 60 MB a minute at 1080p30 HEVC, twice that while a take sits in both Documents and Photos | v1 | INFERRED |
| 6 | Heat and battery: preview + HEVC encode + recognition on one phone; 4K doubles the encode | v2 | INFERRED |

## 1. Q1: can `expo-camera` record front-camera video with audio at 9:16 1080p on iOS 26 in SDK 58?

**Yes.** Not installed today (`app/node_modules/expo-camera` is MISSING; `app/package.json` has no `expo-camera`,
CONFIRMED). SDK 58's recommended version is `~58.0.14` (CONFIRMED, docs.expo.dev/versions/v58.0.0/sdk/camera). The
iOS sources read for this spike are the `sdk-58` branch of expo/expo, `packages/expo-camera/ios/Current/`
(CONFIRMED, fetched 2026-11-03; line numbers below are those files).

| Question | Answer | Evidence |
|---|---|---|
| Front camera, video mode | `facing="front"`, `mode="video"`. The audio input and the `AVCaptureMovieFileOutput` are added **only** in video mode and only when not muted; the default mode is `picture`, so `mode="video"` must be set before `recordAsync`. | CONFIRMED `CameraSessionManager.swift:142-178` (`updateSessionAudioIsMuted`: adds `AVCaptureDeviceInput(device: AVCaptureDevice.default(for: .audio))` when `!isMuted && mode == .video`), `CameraView.swift:105` (`mode = .picture` default) |
| 1080p / 4K | `videoQuality="1080p"` → `AVCaptureSession.Preset.hd1920x1080`; `"2160p"` → `.hd4K3840x2160`; an unsupported preset falls back to `.high`. Default is 1080p. The iPhone 17 front camera records 4K and 1080p (Apple specs: "4K Dolby Vision video recording at 24 fps, 25 fps, 30 fps, or 60 fps", "1080p Dolby Vision video recording at 25 fps, 30 fps, or 60 fps"). | CONFIRMED `CameraRecordingOptions.swift:11-31`, `CameraSessionManager.swift:55-80`, `CameraView.swift:51`; apple.com/iphone-17/specs |
| 9:16 (portrait) | The preset is a 1920x1080 sensor frame; the file's orientation comes from `connection.videoOrientation`, set at record time from the interface orientation (the app is `orientation: "portrait"` in `app.json`, and `responsiveOrientationWhenOrientationLocked` defaults false), so the saved movie is a portrait 1080x1920 track. "Videos are rotated to match device's orientation" (docs). | CONFIRMED `CameraVideoRecording.swift:38-42`, `app.json` `orientation`; INFERRED that the rotation is a QuickTime track matrix (Apple: a movie-file connection "applies the mirror effect with a QuickTime track matrix", same mechanism) |
| Frame rate | Not exposed by `expo-camera` (no prop); the preset's default, 30 fps. | CONFIRMED by absence (`CameraViewModule.swift` prop list :85-244) |
| Codec, bit rate | `recordAsync({ codec: 'hvc1' })` picks HEVC when `availableVideoCodecTypes` has it; `videoBitrate` (prop) applies only when `codec` is given. With no `codec` the movie file output's own default is used (INFERRED: HEVC on this hardware; pass `codec: 'hvc1'` to be sure). `getAvailableVideoCodecsAsync()` lists them. | CONFIRMED `CameraVideoRecording.swift:105-116`, `CameraViewModule.swift:397-399, 473-506` |
| Max duration, file size | `maxDuration` (s) → `maxRecordedDuration`; `maxFileSize` (bytes) → `maxRecordedFileSize`. Apple: the default is "`CMTime.invalid`, which indicates no limit"; recording stops at the limit "with an appropriate error" and the file is still delivered (`AVErrorRecordingSuccessfullyFinishedKey` makes the promise resolve). | CONFIRMED `CameraVideoRecording.swift:92-97, 183-190`; developer.apple.com/documentation/avfoundation/avcapturefileoutput/maxrecordedduration |
| Mirroring: preview vs file | The preview is an `AVCaptureVideoPreviewLayer`, whose connection mirrors a front camera by itself (`automaticallyAdjustsVideoMirroring` defaults true: "For some session configurations, the connection mirrors the video data by default"). The **file** is mirrored only when the `mirror` prop is true (`connection.isVideoMirrored = true` on the movie-file connection); default false = the file reads the way an audience sees him (text on a shirt reads correctly), the preview reads like a mirror. The docs' "iOS flips front-camera videos by default" refers to this preview-vs-file difference. | CONFIRMED `CameraView.swift:20-27, 140`, `CameraVideoRecording.swift:50-52`; developer.apple.com/documentation/avfoundation/avcaptureconnection/automaticallyadjustsvideomirroring, `.../isvideomirrored` |
| Stabilization | `videoStabilizationMode` prop (`off`, `standard`, `cinematic`, `auto`; default `auto`) → `preferredVideoStabilizationMode` when supported. Apple lists "Ultra-stabilized video" and "Cinematic video stabilization (4K, 1080p, and 720p)" for the 17's front camera. | CONFIRMED `CameraVideoRecording.swift:100-103`, `CameraEnums.swift:55-72`; apple.com/iphone-17/specs |
| Pause / resume | `toggleRecordingAsync()` on iOS 18+ (`pauseRecording` / `resumeRecording` on the movie file output). | CONFIRMED `CameraVideoRecording.swift:76-83` |
| Progress | `onRecordingProgress` every 0.5 s (min 0.1): `duration`, `fileSize`, `maxDuration`; drives the clock. | CONFIRMED `CameraVideoRecording.swift:118-147` |
| Where the file lands | `<cache>/Camera/<uuid>.mov`: the cache can be purged, so move it to `Documents/recordings` at once (the recorder already does this for audio). | CONFIRMED `CameraVideoRecording.swift:62`; `lib/recorder.native.ts` `stop()` |
| Background | Camera use is "prohibited while in the background": the capture session is interrupted with `videoDeviceNotAvailableInBackground`; a locked phone ends the take. Unlike audio, `UIBackgroundModes: audio` does not help. The screen must stop-and-save on `AppState` → `background`. | CONFIRMED developer.apple.com/documentation/avfoundation/avcapturesession/interruptionreason/videodevicenotavailableinbackground |
| A phone call | `audioDeviceInUseByAnotherClient` interrupts the session; the movie file output finishes with an error that still carries the file (the promise resolves). What the preview does after the call is a device check. | CONFIRMED the reason exists (`.../interruptionreason`); INFERRED the resolve path (`CameraVideoRecording.swift:183-190`) |
| The audio session | `expo-camera` never touches `usesApplicationAudioSession` / `automaticallyConfiguresApplicationAudioSession` (both default true), so the capture session "automatically configure[s] the app's shared AVAudioSession instance for optimal recording ... The audio session's original state isn't restored after capture finishes." Harmless here: the set recorder sets its mode on **every** start and the memo player sets playback before playing. | CONFIRMED `CameraSessionManager.swift:29` (`let session = AVCaptureSession()`, nothing else set); developer.apple.com/documentation/avfoundation/avcapturesession/automaticallyconfiguresapplicationaudiosession; `lib/recorder.native.ts` `prepareSession` ("Set on EVERY start, never cached"), `lib/localPlay.native.ts` header |
| One preview at a time | "Only one Camera preview can be active at any given time." The `active` prop (iOS) stops the session without unmounting; tie it to the screen's focus. | CONFIRMED docs; `CameraViewModule.swift:231-239` |
| Permissions | The plugin writes `NSCameraUsageDescription` and `NSMicrophoneUsageDescription` (the latter already set by `expo-audio`'s plugin; keep one copy of the sentence). `CameraView` refuses to start without the camera permission (`onMountError`). Ask at the moment of the first Film it, never at launch (HIG/privacy). | CONFIRMED `plugin/src/withCamera.ts:12-13, 50-56`, `CameraSessionManager.swift:401-404`; `app.json` plugins |

Limits worth stating: no frame-rate control; no HDR / Dolby Vision control (the file is whatever the preset gives;
INFERRED SDR 8-bit unless Apple's default for HEVC on this device is HDR, a device check); no access to the audio
sample buffers from JS (the session, inputs and outputs are private to `CameraSessionManager`), which is what §2 is
about.

## 2. Q2: can the on-device recognizer run LIVE while `expo-camera` records with audio?

What each side does with the microphone today (CONFIRMED from source):

- `expo-camera`: an `AVCaptureDeviceInput` for audio inside its `AVCaptureSession`, on the app's shared
  `AVAudioSession` (defaults above), writing straight to an `AVCaptureMovieFileOutput`. No tap, no buffers.
- `expo-speech-recognition` 58.0.1 (mic route): `setupAudioSession` sets `.playAndRecord`, mode `.measurement`,
  options `[.defaultToSpeaker, .allowBluetooth]` (or the caller's `iosCategory`), then `setActive(true)`; builds an
  `AVAudioEngine`, connects `inputNode` → mixer, installs a 1024-frame tap and appends each `AVAudioPCMBuffer` to an
  `SFSpeechAudioBufferRecognitionRequest`; refuses to start if the input format reads 0 Hz / 0 channels
  (`audioInputIsBusy`); tears down on an interruption `.began`; restarts its engine on a route change
  (`node_modules/expo-speech-recognition/ios/ExpoSpeechRecognizer.swift`: `prepareMicrophoneRecognition`,
  `setupAudioSession`, `audioInputIsBusy`, `prepareEngine`, `registerAudioSessionObservers`).
- The file route (`audioSource.uri`) never touches the audio session: it reads the file with `AVAudioFile` and feeds
  4096-frame chunks every 15 ms (on-device) into the same request type (`prepareFileRecognition`). This is the path
  `lib/transcribe.native.ts` uses (`continuous: false, interimResults: false, audioSource: { uri }`).

Apple's own words on the limits: "Plan for a one-minute limit on audio duration ... the framework stops speech
recognition tasks that last longer than one minute" (SFSpeechRecognizer overview, CONFIRMED
developer.apple.com/documentation/speech/sfspeechrecognizer), and for on-device: "A server-based recognition support
has limits on number of requests and audio duration. With on-device recognition, these limits do not apply" (WWDC19
session 256 transcript, CONFIRMED developer.apple.com/videos/play/wwdc2019/256/). Also: "on-device requests won't be
as accurate" (`requiresOnDeviceRecognition`, CONFIRMED). The package honours `requiresOnDeviceRecognition` only when
`supportsOnDeviceRecognition` is true (`ExpoSpeechRecognizer.swift` `prepareRequest`), which is why
`transcribe.native.ts` checks both before `start()` (CONFIRMED, its trap 1).

### 2.1 The options

| Option | What it is | Does it work? | What it takes |
|---|---|---|---|
| **(a) both at once** | Start `recordAsync`, then `ExpoSpeechRecognitionModule.start({ continuous: true, interimResults: true, requiresOnDeviceRecognition: true, iosCategory: { category: 'playAndRecord', mode: 'videoRecording', categoryOptions: ['defaultToSpeaker'] } })`. Two readers (the capture session's audio input and the engine's `inputNode`) on one shared audio session. | **UNKNOWN.** Nothing in Apple's pages forbids two clients of the app's own session, and the package's `audioInputIsBusy` guard exists because the input can read as busy. The hazards: the package's `setCategory` + `setActive(true)` lands while the capture session is running (the capture session may reconfigure or be interrupted); its mode change (`measurement` by default) also changes the processing on the movie's audio (set `iosCategory` mode `videoRecording` to match what the capture session wants); an interruption is delivered to both and the package tears itself down. | No native code, no rebuild beyond v1's. A 20-minute dev-client probe (§7.2 V1) decides it. |
| **(b) a native module that taps the capture session's audio** | Our own Expo module: one `AVCaptureSession` with the front camera input, the audio input, an `AVCaptureMovieFileOutput` for the file and an `AVCaptureAudioDataOutput` whose delegate calls `request.appendAudioSampleBuffer(_:)` on an `SFSpeechAudioBufferRecognitionRequest` (Apple designed that method for exactly this: "Appends audio to the end of the recognition request", takes a `CMSampleBuffer`). The view is an `AVCaptureVideoPreviewLayer`-backed `ExpoView`. | **INFERRED yes.** The `CMSampleBuffer` append API is CONFIRMED (developer.apple.com/documentation/speech/sfspeechaudiobufferrecognitionrequest). Whether `AVCaptureMovieFileOutput` and `AVCaptureAudioDataOutput` may sit in one session on iOS 26 is INFERRED (`canAddOutput` answers at runtime; the fallback is the "RosyWriter" shape: `AVCaptureVideoDataOutput` + `AVCaptureAudioDataOutput` into an `AVAssetWriter`, which also makes record-time caption burn-in and the mirror transform ours). | A local Expo module under `app/modules/` (Swift view + module, `expo-module.config.json`), roughly 400-700 lines; `app.json` keys as in v1; a dev-client rebuild per iteration; **compiles only on EAS** (no Mac here). It replaces `expo-camera` on this screen (two capture sessions cannot share the camera). |
| **(c) record first, recognize the file after** | After the take, hand the `.mov` to the existing `transcribeFile` (the file route) and keep the per-word segments for captions. | **INFERRED yes, one UNKNOWN**: `AVAudioFile(forReading:)` on a QuickTime `.mov` with an AAC track (CoreAudio opens the MPEG-4 family; `.mov` is usually accepted but not documented). If it refuses, a 30-line Swift function exports the audio track (`AVAssetExportSession`, preset `AppleM4A`) or uses `SFSpeechURLRecognitionRequest` (AVAsset-based, opens movies) with `requiresOnDeviceRecognition`. No live following. | Nothing native if `.mov` opens; otherwise the smallest possible module. Reuses the recordings queue (`lib/recordings.native.ts` `pump`, one at a time, memos first, never while a recording runs). |
| **(d) iOS 26 `SpeechAnalyzer` / `SpeechTranscriber`** | Apple's new on-device engine (iOS 26.0+): "good for long-form and distant audio", `reportingOptions: [.volatileResults]` ("tentative results for an audio range in addition to the finalized result"), `attributeOptions: [.audioTimeRange]` ("time-code attributes in a transcription's attributed string", each run a `CMTimeRange`, "precise down to an individual audio sample"), models fetched by `AssetInventory` and kept in system storage, no one-minute design limit. Fed by the same buffers as (b) through `AnalyzerInputConverter` / `AnalyzerInput`. | **CONFIRMED the API** (developer.apple.com/documentation/speech/speechanalyzer, `.../speechtranscriber`, `.../speechtranscriber/reportingoption`, `.../resultattributeoption`; WWDC25 session 277). Caveat from a developer-forum report: time ranges may arrive only on **final** results, not volatile ones (forum thread 794510, INFERRED): the prompter follows volatile text (no timing needed), captions take the final runs. Not in `expo-speech-recognition` 58.0.1 (grep: no `SpeechAnalyzer` in its `ios/` or `src/`, CONFIRMED). | Inside the same module as (b): if `SpeechTranscriber.isAvailable`, use it; else the `SFSpeechAudioBufferRecognitionRequest` path. The owner's phone is iOS 26 / iPhone 17, so this is the path that will actually run. |

### 2.2 Recommendation

- **v1: (c).** Record with `expo-camera`; captions from the file afterwards through today's transcription path with
  per-word segments kept. No native code, one rebuild for the two plugins.
- **v2: probe (a) first (one dev-client session), ship it if it holds; otherwise (b)+(d) as one small module, with
  (d) preferred on iOS 26 and the SFSpeech path as the fallback.** Either way the prompter logic (§3) is the same
  pure rule fed with partial transcripts, so v1's tests carry over.
- Never: mute the camera and let the recognizer record the audio (`recordingOptions.persist`) to mux later. That
  needs native code anyway and desyncs the file.

## 3. Q3: word tracking against a known script

### 3.1 What the recognizer gives us

Per `result` event (mic route, `interimResults: true`): `isFinal`, `results[0].transcript` (the utterance so far,
re-sent as it grows), `segments[] { startTimeMillis, endTimeMillis, segment, confidence }` where confidence "will be 0
on partial results" (CONFIRMED `node_modules/expo-speech-recognition/src/ExpoSpeechRecognitionModule.types.ts`
`ExpoSpeechRecognitionResult`; the mapping from `SFTranscriptionSegment.timestamp` / `duration` is
`ExpoSpeechRecognitionModule.swift` `handleRecognitionResult`). On iOS 18+ a final is detected through
`speechRecognitionMetadata.speechDuration` and the next transcript starts with a space (the package's iOS 18
workaround, CONFIRMED same file). With `continuous: false` recognition ends at the first final; with
`continuous: true` the package runs no timers and the task runs until `stop()` / `abort()` (CONFIRMED
`ExpoSpeechRecognizer.swift` `startRecognitionTask`, `shouldRunTimers = !continuous && !isSourcedFromFile`).

So the stream the prompter sees is: growing partial transcripts, each utterance restarting from empty after a final,
occasional restarts of the whole session (an interruption, a route change the engine could not survive), and a
one-minute cut only on the server path (not ours). The tracker must therefore key on **the last few spoken words**,
not on absolute position in the transcript.

### 3.2 The alignment rule (anchored window match)

- Script = the bit's lines tokenised (lowercase, punctuation stripped); `cursor` = index of the next script word.
- Each result → the last K = 4 spoken tokens.
- Search the script in `[cursor - 3, cursor + 12]` for the best place those 4 tokens match in order, allowing one
  skipped script word and one inserted spoken word (a 4-gram with gaps); a match needs 3 of 4 hits; ties go to the
  place nearest the cursor.
- A match moves the cursor to the end of the match (never back by more than 3); the band scrolls so the cursor's
  line sits at the top.
- No match leaves the cursor alone; after 3 consecutive misses the band shows a quiet "off script" state (the text
  dims, nothing jumps) and keeps waiting at the cursor; the first match brings it back. A callback to an earlier
  line never rewinds (the window is bounded).
- `contextualStrings` gets the script's unusual words (names, invented words) so the recognizer prefers them
  (CONFIRMED option, `SFSpeechRecognitionRequest.contextualStrings`).
- Tap-to-advance and the steady scroll stay as manual overrides in v2 (one-handed in the dark: a thumb on the
  screen beats a model).

### 3.3 Probe (ran 2026-11-03)

A 100-line node probe of that rule, with an invented 58-word script, in the agent session's scratchpad outside the
repo (`film-it/probe/align.mjs`; throwaway, not kept; the shipped version is a `src/rules/prompterRules.ts` with
`tests/prompterRules.test.mjs`). Eight
scenarios, all PASS (CONFIRMED, `node align.mjs` → `ALL PASS`):

| Scenario | Result |
|---|---|
| Verbatim, results as growing partials | cursor reaches the end |
| One script word in seven dropped | reaches the end |
| A filler ("um") inserted every five words | reaches the end |
| One word in six misrecognised | reaches the end |
| The session restarts at word 30 (transcript from empty) | reaches the end |
| Twelve off-script words, then back at the right place | re-anchors, not off |
| Off script and never returns | reported off, cursor holds at 23 |
| A callback repeats an earlier phrase at the end | no rewind |

What the probe cannot show: recogniser latency (word spoken → partial delivered), which sets how far behind the band
runs (a device check, §7.2 V2); and how often on-device partials revise earlier words (the rule tolerates one
substitution per 4-gram).

## 4. Q4: captions, word timestamps, burn-in

### 4.1 Timestamps

`SFTranscriptionSegment`: "represents an utterance, which is a vocalized word or group of words"; `timestamp` = "The
start time of the segment in the processed audio stream", `duration` = "The number of seconds it took for the user to
speak the utterance", `confidence` 0-1 (CONFIRMED developer.apple.com/documentation/speech/sftranscriptionsegment).
In practice on-device dictation returns one segment per word (INFERRED; widely observed, not promised). The package
already carries them to JS as `startTimeMillis` / `endTimeMillis` per segment (CONFIRMED §3.1). On the file route the
whole file is one request, so timestamps are continuous from 0 (CONFIRMED `prepareFileRecognition` feeds one stream);
an old report of per-minute timestamp resets concerned the server path (INFERRED, a 2020 Stack Overflow thread).

Today's rules drop per-word data: `rules/memoRules.ts` `addFinal` folds each final into one `Seg {text, start, end}`
(CONFIRMED `memoRules.ts:37, 69-89`). A `film` kind keeps `words: [{w, s, e, c}]` beside the segments. With (d) in
v2, `audioTimeRange` runs give the same thing with sample precision.

### 4.2 Burn-in: three ways

| Path | What it is | Native cost | Verdict |
|---|---|---|---|
| **Sidecar `.srt` (+ the raw `.mov`)** | Write SubRip from the word list (cues of 2-4 words or a line each, 1-2 s), share both from the share sheet; his editor (CapCut, Final Cut, Premiere, Descript) imports SRT and burns it with its own style. | None. `expo-file-system` writes it; React Native's `Share.share({ url })` hands a file URL to the sheet. | **v1.** Zero native, zero risk, and the owner keeps the typographic control editors give. |
| **`AVMutableVideoComposition` + `CATextLayer`** | Native: `AVMutableComposition` from the `.mov`, a video composition with `renderSize` 1080x1920 and the track's `preferredTransform`, `AVVideoCompositionCoreAnimationTool(postProcessingAsVideoLayer:in:)`, one `CATextLayer` per cue animated on the video timeline (Apple: "Set animations' `beginTime` property to `AVCoreAnimationBeginTimeAtZero`", "`isRemovedOnCompletion` to false", "Avoid using layers that are associated with `UIView` objects"), `AVAssetExportSession` to a new HEVC `.mov`. | About 200-300 lines of Swift in a local module; export time roughly a third to a half of the take's length (INFERRED); re-encodes the whole file (another 60 MB/min). Compiles only on EAS. | **v1.5 if he wants it in-app.** CONFIRMED API (developer.apple.com/documentation/avfoundation/avvideocompositioncoreanimationtool). |
| **JS-side** | There is no JS way to write a video file in this stack: `react-native-skia` draws but does not encode, `expo-video` plays but does not export, `expo-camera` does not overlay. The only JS "captions" are an overlay at playback time inside Punchable (the player reads the word list). | None, but nothing leaves the app burned. | Fine for the in-app review player; not a burn-in. |

## 5. Q5: the eye line on an iPhone 17

Figures (apple.com/iphone-17/specs, CONFIRMED): 6.3-inch display, 2622 x 1206 px at 460 ppi = **874 x 402 pt**
at 3x; 1 pt = 0.166 mm; the display is 144.8 mm tall; the phone 149.6 x 71.5 mm. Front camera: "18MP Center Stage
camera", f/1.9, a square sensor (press coverage, INFERRED) so portrait and landscape framings crop the same sensor.

Where the lens is (INFERRED from the Dynamic Island layout since iPhone 14 Pro, reported unchanged on the 17: the
island splits into "a small pill on the left and a circle on the right" and "the camera is positioned slightly to the
right of center"): the lens is the right-hand circle of the island, its centre about **30 pt (5 mm) below the top
edge and about 40 pt (7 mm) right of the display's centre line**. The top safe-area inset is about 62 pt (INFERRED;
read `useSafeAreaInsets().top` at runtime, never hard-code).

The band: pin it to the safe-area top, directly under the island, full width, text left-aligned with a 20-pt inset
(the horizontal offset to the lens is under 1 degree at any distance, so centring is fine too). Height is the
lever: with the current line at the top of the band the gaze error is the distance from the lens to that line:

| Band (lines x size) | Band height | Current line centre below the lens | Gaze error at 35 cm (hand-held) | at 50 cm (propped) |
|---|---|---|---|---|
| 3 x 40/44 (`type.display`) | 132 pt | ~54 pt = 9 mm | 1.5° | 1.0° |
| 4 x 40/44 | 176 pt | same (top line) | 1.5° | 1.0° |
| last line of a 4-line band | | ~186 pt = 31 mm | 5.1° | 3.5° |
| 6 x 34/41 (Large Title) | 246 pt | last line ~216 pt = 36 mm | 5.9° | 4.1° |

Teleprompter practice treats up to about 5 degrees off-axis as eye contact (INFERRED, industry rule of thumb; no
Apple page), so: **a band of at most four lines (about 180-240 pt), the current line kept at the top, and the text
scrolling up so his eyes stay within ~10 mm of the lens.** Below the band the preview stays clear (he needs to see his
framing); the record control is the dock at the bottom as on Stage. Size: 40/44 Space Grotesk 700 (`type.display`,
`docs/design/DESIGN.md:164`) or the stage rows' 40/30 marker text (`stage/[id].tsx`), cap 1.2 like the clock; white on
a 60% black scrim (the preview behind is bright and moving).

HIG pages, cited:

- HIG/typography (CONFIRMED developer.apple.com/design/human-interface-guidelines/typography): "People need to be
  able to read your content at various viewing distances and under a variety of conditions"; iOS default 17 pt,
  minimum 11 pt; "Keep in mind that font weight can also impact how easy text is to read."
- HIG/camera-control (CONFIRMED `.../camera-control`; the iPhone 17 has the Camera Control button): "Minimize
  distractions in the viewfinder. When capturing a photo or video, people appreciate a large preview image with as
  few visual distractions as possible"; "Make space for the overlay in the viewfinder ... place your UI outside of
  the overlay areas." We do not adopt Camera Control in v1; if v3 does, the right edge in portrait stays clear.
- HIG/privacy (CONFIRMED `.../privacy`): "Request permission only when your app clearly needs access to the data or
  resource"; "Avoid requesting permission at launch"; purpose strings "Aim for a brief, complete sentence that's
  straightforward, specific, and easy to understand." Proposed camera string: "Punchable films your takes on this
  iPhone. The video stays on your phone." Photos (add-only): "Punchable saves your takes to Photos."
- HIG/playing-video (CONFIRMED `.../playing-video`): "Always display video content at its original aspect ratio":
  the in-app review player (`expo-video` 58.0.13, installable, PiP needs its plugin) shows the 9:16 take in aspect
  mode, never cropped.
- `docs/design/HIG.md` §2 and §3.9 (CONFIRMED): Stage and Record are `fullScreenModal`, `gestureEnabled: false`,
  `autoHideHomeIndicator: true` (`app/src/app/_layout.tsx:63, 159-160`); nothing records until the disc; the square
  stops and saves; Discard behind Close; 44-pt targets, 60-pt dock controls. Film it is the third such modal.
- HIG/modality (in `HIG.md` §2): one full-screen task, no sheet over it but the notes sheet precedent
  (`SPEC-2026-10-29`): the prompter's speed slider lives in a `Stack.Toolbar` menu or on the screen, not a sheet.

## 6. Q6: saving, size, what stays on the phone, what the server gets

| Item | Answer | Evidence |
|---|---|---|
| Save to Photos | `expo-media-library` 58.0.13 is installed (used for the Capture badge and Import, `docs/IOS.md` §7). `saveToLibraryAsync(uri)` writes a video through `UISaveVideoAtPathToSavedPhotosAlbum` after `UIVideoAtPathIsCompatibleWithSavedPhotosAlbum`; it **throws `MissingPListKeyException` unless `NSPhotoLibraryAddUsageDescription` is in Info.plist**, and that key is not there today (`app.json` has no media-library plugin). Add the plugin with `savePhotosPermission`; request `requestPermissionsAsync(true)` (write-only). | CONFIRMED `node_modules/expo-media-library/ios/MediaLibraryModule.swift:106-146, 60-66`; `plugin/src/withMediaLibrary.ts:82-86` (sdk-58); `app.json` plugins |
| A "Punchable" album | `createAlbumAsync` / `addAssetsToAlbumAsync` run under `runIfAllPermissionsWereGranted`, i.e. **full** library access (read + write, `NSPhotoLibraryUsageDescription`), and the album is a second permission prompt with a much bigger grant. | CONFIRMED `MediaLibraryModule.swift` (`addAssetsToAlbumAsync` and album functions wrapped in `runIfAllPermissionsWereGranted`) |
| Recommendation | v1: add-only save to the camera roll, no album (the smaller grant, HIG/privacy "Request access only to data that you actually need"). An album later, only if he asks, behind its own prompt. | INFERRED |
| Size per minute | 1080p30 HEVC about **60 MB/min**; 4K30 HEVC about **170 MB/min**; H.264 roughly double (130 / 350). Apple shows these in Settings › Camera › Record Video; third-party write-ups agree. A 3-minute rant at 1080p ≈ 180 MB; ten takes a week ≈ 1.8 GB. `videoBitrate` can pin it (8 Mbit/s ≈ 60 MB/min). | INFERRED (phonearena, videoproc, macrumors write-ups; not an Apple reference page) |
| Where the take lives | Owner rule: on the phone. The `.mov` moves from the cache to `Documents/recordings` (backed up with the phone like the memos), is saved to Photos, and the Documents copy is **deleted once the words exist and the Photos save succeeded** (the index row keeps the Photos `localIdentifier`, not a path). One copy, in his Photos, under his iCloud Photos settings, never ours. | INFERRED design; `lib/recordings.native.ts` header (file names only, Documents) |
| What the server receives | Only words, as today: a `film` row routes like a memo (`routeOf`: an idea when ≤ 4000 characters, else a set transcript with `{segments: [{text, start, end}]}`), `from: {note: "Film it, <date>"}`. Word timings and the `.srt` stay on the phone in v1; v3 sends `words` with the session work (§7.3). The video never goes up; there is no upload route for it and none is proposed. | CONFIRMED routes `api-dev/creative_api.py:1391-1423` (`POST /api/sets/transcript`), `lib/recordings.native.ts` `sendOne`, `rules/memoRules.ts:166-167` |
| Privacy manifest | Nothing new: camera, microphone and Photos are permission-gated APIs, not "required reason" APIs. | CONFIRMED `app.json` `privacyManifests` (the four reasons listed are the only required-reason categories in use) |

## 7. Q7: the build plan

### 7.1 v1: steady-scroll + tap-to-advance prompter while recording; captions post-hoc

| Piece | What | Native module | Rebuild |
|---|---|---|---|
| Dependencies | `npx expo install expo-camera` (58.0.14); `app.json` plugins: `["expo-camera", {cameraPermission}]`, `["expo-media-library", {savePhotosPermission}]`; `npx expo-doctor` N/N | no (Expo's own) | **yes**: dev client (`dev-device`) and TestFlight |
| Route | `app/src/app/film/[id].tsx` (a bit's id; `?version=` optional), registered in `_layout.tsx` with the `fullScreen` options and `gestureEnabled: false`; entry from the bit page's ⋯ ("Film it", `video`) and the Tonight ⋯ | no | no |
| Screen | `CameraView` (`facing="front" mode="video" videoQuality="1080p" mirror={false} videoStabilizationMode="auto" active={focused}`), `lib/RecordControl` in the dock (disc / square / Pause left / "Next line" right, prominent), `useKeepAwake`, `audioBusy.setRecording(true)` while taping so the transcription queue waits and no memo plays (CONFIRMED `recordings.native.ts` `pump`, `localPlay.native.ts`), AppState `background` → stop and save, `onMountError` → the mic-off style card with Open Settings | no | no |
| Prompter | `src/rules/prompterRules.ts` (pure): lines from the version text (or `rules/bitLines.ts` once Slice 1 lands), words-per-minute → pt/s given the band's line height and average words per line, tap = next line, long-press = hold, a 3-position speed (slow / his pace / fast, default 150 wpm) in the toolbar; `lib/Prompter.native.tsx` draws it with Reanimated (one `'worklet'` per top-level function, declare-before-use: the two traps in `HANDOVER.md`) | no | no |
| Save | `recordAsync({ codec: 'hvc1', maxDuration: 600 })` → move to `Documents/recordings/<iso>.mov` → `saveToLibraryAsync` → `addRecording(..., { kind: 'film', bit })` → the queue transcribes (per-word segments kept in a `words` field) → `<name>.srt` written → words sent as today → delete the Documents copy | no | no |
| Share | Row actions in Bits › Memos / the bit's page (`lib/RecRows`): Play (in-app, `expo-video` in aspect mode), Share take + captions (share sheet with the Photos asset and the `.srt`), Delete | no (`expo-video` is Expo's) | yes if `expo-video` is added (another plugin) |
| Server | none | | |
| Gates | `tsc`, `test:unit` (new `prompterRules.test.mjs`, `memoRules` cases for the `film` kind), `static.*`, `expo-doctor`, guard | | |

Device checks for v1 (dev client, in this order; each is one short take):

| # | Check | Decides |
|---|---|---|
| D1 | A 20-second take; open it in Photos: portrait 1080x1920, plays, has audio at a normal level with the phone at arm's length | the preset/orientation path and the audio input |
| D2 | A shirt with text: the Photos copy reads correctly (unmirrored), the preview reads mirrored; then `mirror={true}` once | the mirror default and whether a "Mirror the take" switch is wanted |
| D3 | Bits › Memos: "Transcribing…" then words for the take; the `.srt` has per-word cues | whether `AVAudioFile` opens a `.mov` (else the 30-line Swift export, §2.1 c) and whether segments are per word (§4.1) |
| D4 | Lock the phone at 0:15 of a take | the stop-and-save on `background` lands before the interruption, and the file is whole |
| D5 | A phone call at 0:15 (or an alarm) | what the promise returns, whether the preview comes back, what the status word should say |
| D6 | A 3-minute take: file size in Photos (expect ~180 MB), the phone's warmth, the battery drop | the 1080p / HEVC choice; whether 4K is ever offered |
| D7 | Read a 30-second bit off the band at arm's length, then watch it: does he look at the camera? | the band height and the "current line at the top" rule (§5) |
| D8 | VoiceOver on: the disc, Next line and the band's current line are announced; Reduce Motion: the scroll becomes line steps | accessibility of the band |

### 7.2 v2: live voice-following

| Step | What | Native module | Rebuild |
|---|---|---|---|
| V1 probe (first) | In the dev client, with v1 recording, call `ExpoSpeechRecognitionModule.start({ continuous: true, interimResults: true, requiresOnDeviceRecognition: true, contextualStrings, iosCategory: { category: 'playAndRecord', mode: 'videoRecording', categoryOptions: ['defaultToSpeaker'] } })`; try both orders (recognizer first, camera first). Pass = partial results arrive for 2 minutes AND the saved file has audio AND `recordAsync` did not resolve early. | no | no |
| If V1 passes | Wire the tracker (§3.2) to the partials; the manual controls stay; "off script" dims the band | no | no |
| If V1 fails | Local module `app/modules/punchable-film/`: one `AVCaptureSession` (front camera, audio), `AVCaptureMovieFileOutput` + `AVCaptureAudioDataOutput` (fallback: `AVAssetWriter` from data outputs), preview view, events `partial {text}`, `final {runs: [{text, start, end}]}`, `progress`, `interrupted {reason}`; iOS 26: `SpeechTranscriber(locale, reportingOptions: [.volatileResults], attributeOptions: [.audioTimeRange])` via `AnalyzerInputConverter`; else `SFSpeechAudioBufferRecognitionRequest.appendAudioSampleBuffer`. `expo-camera` leaves this screen. | **yes** (Swift, compiled on EAS only) | yes, per iteration |
| Checks | V2 latency word → cursor (want under ~400 ms); V3 a 3-minute take without a restart (on-device: no one-minute limit, CONFIRMED WWDC19); V4 his names and invented words with `contextualStrings`; V5 the `AssetInventory` download prompt and its size on first use (d); V6 heat over 3 minutes with recognition on | | |

### 7.3 v3: hooks / rant cut with the sessions machinery

`docs/design/SPEC-2026-11-02-bit-sessions-and-joke-teaching.md` defines sessions on a bit (`session`, `session_turn`,
`suggestion` rows; modes Break it down · Find the turn · Tighten · Tag it · Talk; routes in pk-api). v3 adds:

- a take's `words` (with timings) as a session input (`source: "film"`), so Find the turn / Tighten work on what he
  actually said, and the kept version becomes the next take's script (the loop the feature is for);
- the **rant cut**: the server (or the phone's rule) picks a 30-60 s window by the words (the reveal and the laugh
  marks when audio processing exists, item 6 in HANDOVER); the phone cuts the video with `AVAssetExportSession`
  `timeRange` (native, ~60 lines, same module as §4.2 burn-in) or writes a cut list the owner's editor applies;
- a `POST` carrying `words` (text, start, end, confidence) alongside the transcript: the first server change, in
  pk-api (`OWN_PATHS`), proven on a scratch database as the sessions spec does.

Native: yes (cut/burn in the same module); rebuild: yes; server: yes (words with timings). Checks: the cut lands on
the words it claims; a 60-second cut exports in well under a minute.

### 7.4 Risks, ranked

| # | Risk | Tag | Confirm / rule out | Mitigation |
|---|---|---|---|---|
| 1 | Two microphone clients at once (v2 option a) | UNKNOWN | §7.2 V1, 20 minutes on the dev client | the module path (b)+(d) uses one capture session |
| 2 | Swift compiles only on EAS: a typo is a 15-minute build; the `SPEC-2026-10-31` module shipped uncompiled the same way | CONFIRMED precedent | the first EAS build log | keep v1 JS-only; the module small and in one PR; `npx expo prebuild --platform ios --no-install` locally catches plugin/podspec wiring (no compile) |
| 3 | `AVAudioFile` refuses `.mov` | UNKNOWN | D3 | 30 lines of Swift (`AVAssetExportSession` AppleM4A), or the URL request |
| 4 | Segments not per word; timing drift on long takes | INFERRED | D3 (inspect the `.srt`) | cues per 2-4 words tolerate it; v2 (d) gives sample-exact runs |
| 5 | Storage: two copies while a take is in flight | INFERRED | D6 | delete the Documents copy after the words exist; `maxDuration: 600`; 1080p only |
| 6 | Heat / battery: preview + HEVC + recognition | INFERRED | D6, V6 | 1080p, `videoStabilizationMode="standard"` if `auto` runs hot, stop recognition when off script for 30 s |
| 7 | `background` kills the take before our stop lands (the capture session is interrupted at once) | INFERRED | D4 | stop on `inactive` as well as `background` for this screen only (a permission prompt also sends `inactive`, so re-arm on `active`) |
| 8 | A call mid-take: `expo-camera` has no resume; the promise resolves with the partial file | INFERRED | D5 | treat as stop-and-save with "Saved · interrupted by a call" |
| 9 | `expo-camera`'s automatic audio-session configuration leaves the session in a recording state after the take | CONFIRMED (Apple: "original state isn't restored") | play a memo right after a take | already handled: the player and the recorder set their modes on every start |
| 10 | The prompter band covers the framing; he cannot see whether he is in shot | INFERRED | D7 | the band is at most four lines; a one-tap "peek" hides it for 2 s |
| 11 | Accessibility: a scrolling band is motion; VoiceOver reading a moving line | INFERRED | D8 | Reduce Motion → stepped lines; the current line is the one accessibility element |

## 8. What the owner must do

Nothing until a build exists. No Apple 2FA step: EAS credentials cover a new native dependency. When build v1 is
ready: run D1-D8 in §7.1 in order and send the eight one-line answers.

## 9a. Where this spike and `SPEC-2026-11-03-film-it.md` (design lane, commit `0c6a9fe`) differ

The spec landed while this spike was being written; its §4 names this spike as the owner of the native answers.
Both agree on: `expo-camera` front / video / portrait / preview mirrored, file not; `expo-video` for review;
`expo-media-library` for Photos with the plugin added to `app.json`; the words through today's queue; the prompter
as a pure rule; the full-screen modal built like Stage. The differences, for the lead to settle:

| Spec says | Spike finds | Suggest |
|---|---|---|
| D4: `codec: 'avc1'` (H.264) | HEVC (`hvc1`) is about half the size at the same quality (§6: ~60 vs ~130 MB/min) and is what the Camera app writes on this phone; Photos, Messages and every editor he would use read HEVC. `getAvailableVideoCodecsAsync()` confirms it on the device. | `hvc1`, with `avc1` only if a device check shows a player he needs cannot open it |
| D9 + §4: captions **burned in on Keep** through a local Swift module, in v1 | Burn-in is the one piece of Swift in the feature and it compiles only on EAS (risk 2). An `.srt` sidecar needs no native code and keeps v1 free of blind Swift. | v1 ships the `.srt` (and the in-app overlay at review); the burn module is v1.5, built once the camera path is proven on the phone |
| D13: the audio track is **extracted** by the Swift module before transcription | Whether `AVAudioFile` opens the `.mov` directly is UNKNOWN (D3); if it does, no extraction is needed at all. | run D3 first; write `extractAudio` only if D3 fails |
| D8: Photos only, "the take is a file in the cache until he decides" | The cache can be purged under pressure (the same reason the recorder writes to Documents); a take in flight (transcribing, not yet saved) should sit in `Documents/recordings` and be deleted after the words exist and the Photos save succeeded (§6). The end state is the same: one copy, in Photos. | Documents while in flight, then delete |
| §2.3: band type 26/32 Inter 600 (reasoned for 60 cm) | Consistent with §5: a smaller line makes a shorter band, which keeps the gaze nearer the lens; the trade is legibility of a moving line at arm's length. D7 decides between 26 and 34-40. | keep the token proposal; D7 picks the size |
| §2.9: the one-minute live limit is "UNKNOWN until the spike" | Answered: on-device has no such limit (WWDC19 transcript, CONFIRMED) and the package runs no timers in `continuous` mode (§3.1); the restart logic in the spec's row is still right for interruptions. | treat 60-s restarts as an interruption case, not the normal path |
| §4: lazy import of the new module, as `lib/transcribe.native.ts` does | Agreed and worth stating in the plan: a route file that imports `expo-camera` at module scope would fail at launch on a binary without it (every native route is required at startup; `asyncRoutes` is web-only in `app.json`). | `const { CameraView } = await import('expo-camera')` inside the screen, the way `transcribe.native.ts` loads `expo-speech-recognition`; the same for `expo-video` |

## 9. Evidence index

In this checkout: `app/package.json` (no `expo-camera`, `expo-video`; `expo-speech-recognition` 58.0.1,
`expo-media-library` ~58.0.13, `expo-audio` ~58.0.11); `app/app.json` (plugins, `orientation`, `infoPlist`);
`app/src/lib/transcribe.native.ts` (the file route and its traps); `app/src/lib/recordings.native.ts` (the queue,
`pump`, `interrupt`, `sendOne`); `app/src/lib/recorder.native.ts` (`prepareSession`, `stop`); `app/src/lib/audioBusy.ts`;
`app/src/lib/localPlay.native.ts`; `app/src/rules/memoRules.ts:8, 37, 69-89, 166-167`; `app/src/app/record.tsx`;
`app/src/app/stage/[id].tsx`; `app/src/app/_layout.tsx:63, 159-160`; `docs/design/HIG.md` §2, §3.9;
`docs/design/DESIGN.md:164, 166`; `docs/design/SPEC-2026-10-31-voice-memos.md`;
`docs/design/SPEC-2026-11-02-bit-sessions-and-joke-teaching.md`; `api-dev/creative_api.py:1391-1423`;
`app/node_modules/expo-speech-recognition/ios/ExpoSpeechRecognizer.swift`, `.../ExpoSpeechRecognitionModule.swift`,
`.../SpeechRecognitionOptions.swift`, `.../src/ExpoSpeechRecognitionModule.types.ts`, `.../README.md` (audio session
notes); `app/node_modules/expo-media-library/ios/MediaLibraryModule.swift:60-66, 106-146`.

Fetched 2026-11-03: expo/expo `sdk-58` `packages/expo-camera/ios/Current/{CameraView,CameraSessionManager,
CameraVideoRecording,CameraRecordingOptions,CameraEnums}.swift`, `ios/CameraViewModule.swift`,
`plugin/src/withCamera.ts`; `packages/expo-media-library/plugin/src/withMediaLibrary.ts`;
docs.expo.dev/versions/v58.0.0/sdk/{camera,media-library,video}; Apple reference pages:
`avfoundation/avcapturesession/{automaticallyconfiguresapplicationaudiosession,usesapplicationaudiosession,preset,
interruptionreason}`, `avfoundation/avcapturefileoutput/maxrecordedduration`,
`avfoundation/avcaptureconnection/{isvideomirrored,automaticallyadjustsvideomirroring}`,
`avfoundation/avcaptureaudiodataoutput`, `avfoundation/avvideocompositioncoreanimationtool`,
`speech/{sfspeechrecognizer,sfspeechaudiobufferrecognitionrequest,sfspeechurlrecognitionrequest,
sftranscriptionsegment,speechanalyzer,speechtranscriber}`, `speech/sfspeechrecognitionrequest/requiresondevicerecognition`,
`speech/sfspeechrecognizer/supportsondevicerecognition`, `speech/speechtranscriber/{reportingoption,resultattributeoption}`;
HIG `typography`, `camera-control`, `playing-video`, `privacy`; WWDC19 session 256 and WWDC25 session 277 transcripts;
apple.com/iphone-17/specs. Press for the island layout and the per-minute sizes: MacRumors (2022, the island's
"circle on the right"), 9to5Mac, phonearena, videoproc (INFERRED figures only).
