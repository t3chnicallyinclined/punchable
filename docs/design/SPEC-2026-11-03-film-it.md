# SPEC 2026-11-03: Film it (front-camera rant videos, a prompter that follows his voice, captions in his words)

Status: **approved feature, build spec** (owner 2026-11-03: "lets get this in"). It hangs off the bit page of
`SPEC-2026-11-03-bit-page-3c.md` (the lines, the versions stepper, the sessions sheet with Keep / Skip, the More
menu, the On tape row, the clutter rules) and reuses the recording patterns of `app/src/app/stage/[id].tsx` and
`record.tsx`. The owner rules of `docs/HANDOVER.md` hold: voice is recognised on the phone only; audio and video stay
on the phone and only words go to the server; a cloud backup of recordings comes later as a paid add-on; one-handed,
dark, backstage; TestFlight-only.

Tags: **CONFIRMED** = a `file:line` in this repo or under `app/node_modules`, a HIG or Apple page read live
2026-11-03, or a gate run; **INFERRED** = design reasoning or a figure not yet measured; **UNKNOWN** = the device or
the spike decides. Every joke in this file is invented; nothing here is his material. Nothing here is a commit, a
deploy or a build. No time estimates.

**The feasibility spike** (`docs/design/SPIKE-2026-11-03-film-it-feasibility.md`, ios-dev-expert) **did not exist
when this was written.** So: **v1 needs nothing from it** (a steady prompter with tap-to-advance while the camera
records; captions after the take, from the recording); **v2 (the prompter follows his voice) depends on it**; §4 lists
the exact questions it must answer, and the builder reads the spike before v2.

**The feature in one paragraph.** From a bit's More menu, **Film it…** opens the front camera full-screen, 9:16, with
his lines in a narrow band directly under the camera so his eyes stay near the lens. A countdown, a running clock, the
punch marked. He records, the prompter advances (v1: a steady pace he can nudge with **Next line**; v2: by his voice),
he stops, reviews, and **Keep** saves the take to Photos in a **Punchable** album with captions burned in from the
word-timed transcript; **Redo** starts again. A kept take's words and timings become a camera night on the bit's **On
tape** row ("Filmed · Nov 3 · 0:48"), so a later session can compare what he said with what he wrote. A bit can also
get a **rant cut**: the session **Make it a rant** reshapes the lines for direct address and escalation (Keep / Skip
per line, saved as a cut beside the stage cut, "rant" in the versions stepper), and **Pick a hook** offers three
openers, he picks one (the Find the turn pattern). Nothing else: no editor, no effects, no upload, no sharing beyond
the Photos save (the share sheet is Photos' own).

---

## 1. Decisions taken (lead calls, owner may flip)

| # | Question | Decision | Flip cost |
|---|---|---|---|
| D1 | Where the prompter sits | **Directly under the bar, above the preview**: the band is the thing nearest the front lens, so his eye line stays within a few degrees of it. The preview sits under the band. | Swap the two `View`s. |
| D2 | What the band holds | **Three lines**: the one before (dim), the current (bright, big), the next (dim). At font scale ≥ 1.3: the current and the next only. Never a pixel-scrolling column of text (motion under his eyes, and nothing for Reduce Motion to fall back to). | `LINES_IN_BAND` constant. |
| D3 | How v1 advances | **Line by line on a timer at his pace** (words ÷ his words-per-second from the bit's taped run time, `duration_s`, CONFIRMED `lib/api.ts:343-344`; else 2.4 words/s, INFERRED), plus **Next line** (60-pt dock control) and **Hold** (pauses the advance, not the recording). A tap on the band is Next line too; the preview is tap-free. | The rate constant; the gestures are extras over the buttons (HIG/accessibility "Offer alternatives to gestures"). |
| D4 | Orientation, size | **Portrait only, 9:16, 1080 × 1920, front camera, `avc1`**. No landscape, no rear camera, no zoom. | `facing`, `codec` props. |
| D5 | Mirroring | Preview mirrored (the front camera's native preview; he sees himself as in a mirror); **the file is not mirrored** (as the iOS Camera app saves selfies: a sign in the room reads the right way). | `mirror` prop on `CameraView`; open question Q2. |
| D6 | The countdown | **3 · 2 · 1 over the preview**, `selection` haptic per tick, then `medium` on the first frame and the clock starts. Tap the countdown to skip it. **No haptics while recording** (HIG/playing-haptics: "Ensure that haptic vibrations don't disrupt experiences involving device features like the camera, gyroscope, or microphone", CONFIRMED). | `COUNTDOWN_S`. |
| D7 | Stop, Redo, Close | The square **stops and goes to review** (nothing is lost: the take is a file in the cache until he decides). **Redo** on the review discards that take and returns to the countdown without a sheet (HIG/feedback: "don't warn people when data loss is the expected result of their action", CONFIRMED). **Close** with a take recorded and not kept: an action sheet "Leave the take?" · Discard the take (destructive) · Keep the take · Keep going (HIG/modality: "present an action sheet that includes a save option", CONFIRMED). | None. |
| D8 | Where the video lives | **Photos only** (the captioned file). Punchable keeps no copy of the video; it keeps the words, the timings and the Photos asset id. Deleting it in Photos leaves the words on the bit ("not in Photos" on the row). The paid backup later syncs words first; video is Q1. | A copy under `Documents/takes/` (doubles his storage). |
| D9 | Captions | **One fixed look, burned in on Keep**: his words, Inter 600, white with a dark edge and a soft band, lower third, at most four words or 1.6 s per caption, broken at line ends where the words match a line. No caption editor. The review shows the captions as a live overlay before the burn, so Keep never surprises. | A "Captions" switch on the review (Q3). |
| D10 | The rant cut is a version | A rant cut is a **version row** (`unit`) whose `line_meta.cut = 'rant'` (3c §5 `line_meta`; `cut` defaults to `stage`). The page body always shows the newest **stage** version; the stepper walks every version and says the cut; a rant version shows a "Rant cut" word above the lines. Write on a rant version writes a rant version. **Film it opens on the newest rant cut if one exists, else the stage cut**, with a cut picker in the bar. | `cut` on `line_meta` only; no new table. |
| D11 | Pick a hook | Works on the cut being filmed (rant if there is one). Three openers; **he picks one or none**; the pick arrives as a tracked change inserting line 0 (Keep / Skip, Save version), and his pick order is preference data. **Show others** asks for three more. The same screen as Find the turn (old spec S7). | None. |
| D12 | Off script (v2) | When six heard words in a row match nothing in the next two lines the prompter **holds** and the status word reads **Riffing**; three matched words in a row inside the next three lines snap it there. Next line always works. Captions are what he said, never the script. | Thresholds in `rules/prompter.ts`. |
| D13 | The take's audio (v1) | The camera records with sound; after the take the **audio track is extracted on the phone** (the same local Swift module that burns captions; AVFoundation) and goes through today's on-device file transcription (`lib/transcribe.native.ts`, CONFIRMED on-device only, `:110-114`) with **per-word timings kept** (the recognizer's `segments`, CONFIRMED `expo-speech-recognition/build/ExpoSpeechRecognitionModule.types.d.ts:37-47`; today `addFinal` collapses them to utterances, `rules/memoRules.ts:61-67`). | The spike may prefer a muted camera plus `expo-audio` for the m4a (§4). |
| D14 | Offline | Film it works **entirely offline** (camera, prompter, transcription, burn-in, Photos). Only the words wait for signal (the memo queue's pattern). The two sessions need Claude and grey "offline" like every session (3c §8.5). | None. |

Standing rules, unchanged: HIG first on the iPhone; plain words; no control that leads nowhere; a greyed control shows
its reason in one word; nothing on screen explains itself in a sentence; his words first; 44 pt; never colour alone;
every write has Undo; AI work carries the badge; Claude never changes a bit without his per-line say.

---

## 2. Screens

Sizes are points on a 390 × 844 iPhone; every control is 44 pt (HIG/buttons; HIG/accessibility iOS default 44,
minimum 28). Dark is the default; every token has a light twin (DESIGN.md §3). Film it is a full-screen modal because
it is a camera view (HIG/modality: "A modal experience that fills a window or the device display minimizes
distractions, so it can work well for presenting videos, photos, or camera views", CONFIRMED).

### 2.1 Entry: the More menu

**Reorder lines · Record a memo · Film it… · — · Pick a set list… · Merge with… · Share… · — · Rename short name… ·
Stage › · — · Retire.** Film it sits with the two other "make something from this bit" items (frequency order, HIG/menus),
icon `video` (CONFIRMED present in `sf-symbols-typescript@2.2.0 dist/index.d.ts`, as is `video.fill`; outline in a menu, HIG/sf-symbols), the
ellipsis because a screen follows. Today's menu is CONFIRMED `joke/[id].tsx:258-276` (`Stack.Toolbar.Menu`, the web
`MenuSheet` twin `:232-242`). **Disabled with a reason** when the bit has no lines (`subtitle` "nothing to read" on
the native item; VoiceOver hint on the web). Never disabled offline (D14). Pushes `/film/[id]` with `?cut=<unit>` when
he chose a cut.

### 2.2 The Film it screen (`app/film/[id].tsx`, full-screen modal)

Route: the root stack with the `fullScreen` options Stage uses (CONFIRMED `app/_layout.tsx:63` `fullScreen`:
`presentation: 'fullScreenModal'`, `autoHideHomeIndicator: true`), `gestureEnabled: false` and `useKeepAwake` as
`stage/[id].tsx:37` does. Header: the task `TopBar` (Close top-leading, title "Film it", trailing the **cut picker**:
"Rant ▾" / "Stage ▾", a `TextBtn` that opens an action sheet listing the newest version of each cut; **as built
(build 24): shown whenever the camera is idle, also with only the stage cut, because its "Make a rant cut…" item
(§2.5) must be reachable from here**; the cut shown carries a ✓). Status bar hidden while recording is not done: the clock is ours and the system's time is one
glance away.

```
┌────────────────────────────────────────┐
│ Close             Film it       Rant ▾ │  the task bar (custom, as Stage)
├────────────────────────────────────────┤
│   Self-checkout is the store hiring    │  band: previous line, label.3
│ ► you for free, and you said yes.      │  current line, type.prompter (§2.3), punch = bold + amber dot
│   Nobody at my job claps when I scan.  │  next line, label.3
├────────────────────────────────────────┤
│ ┌────────────────────────────────────┐ │
│ │ ● 0:12  Recording                  │ │  the clock and status word, inside the preview's top-left
│ │                                    │ │
│ │          (front camera)            │ │  9:16, `contentFit` contain, radius.4, the preview mirrored
│ │                                    │ │
│ │                                    │ │
│ └────────────────────────────────────┘ │
│ ┌──────────┐   ╭────╮   ┌───────────┐  │
│ │   Hold   │   │ ■  │   │ Next line │  │  the dock: 60 · 92 · 60 (lib/RecordControl, RecordDock)
│ └──────────┘   ╰────╯   └───────────┘  │
└────────────────────────────────────────┘
```

| State | Band | Preview | Clock / status | Dock (left · centre · right) |
|---|---|---|---|---|
| **Idle** | the first three lines of the cut; ► on line 1 | live | "0:00 · Ready" | **cut picker is in the bar**; left: *(empty)* · **red disc** (80 in a 92 ring, `systemRed`, CONFIRMED `RecordControl.tsx:13-16`) · **Next line** (steps the start line; the clock does not run) |
| **Countdown** | unchanged | **3 · 2 · 1** centred, `type.clock` (84 pt tabular, cap 1.2), one tick a second, `selection` haptic each; tap = skip | "0:00 · Starting…" | the disc dims and ignores taps (`busy`, CONFIRMED `RecordControl.tsx:28-29`) |
| **Recording** | advances (v1 timer, v2 voice, §2.3); the punch line bold with the dot | live; a 1-pt `record` ring around the preview (shape + the word, never colour alone) | "● 0:12 · Recording" in `destructive`, the same words as Record (`statusWord`, CONFIRMED `rules/recorderLogic.ts:123`) | **Hold / Go** (pauses the prompter only; `tone="amber"` while held) · **red square** (30 pt in the 80 disc: stop → review) · **Next line** (prominent, as Stage's Next bit) |
| **Review** | §2.4 | the take plays | "0:48 · 112 words" (one status line, no sentence) | **Redo** (plain) · *(no centre control)* · **Keep** (prominent) |
| **Saving** | — | the frame he stopped on, dimmed | "Saving…" with a determinate bar under it (burn-in progress; HIG/feedback) | all disabled |

Rules in play: **nothing records until the red disc** (HIG.md §3.9, kept: opening Film it never starts a take); the
centre control never moves ("his thumb learns one place", `RecordControl.tsx` header); the clock ticks every 250 ms
(`stage/[id].tsx:59-65`); the screen stays awake. The clock sits inside the preview because the band must hold only
his words (the clutter rule) and the status belongs "near the items it describes" (HIG/feedback). No target bar and no
level meter: a rant is as long as it is, and he is looking at the lens, not a meter (Q8 keeps a 60-s light open).

VoiceOver: the band is one element, label "Line 2 of 7, punch: …", `accessibilityLiveRegion="polite"` on the
current line only (a line change is announced; the clock is not a live region, as on Record, SPEC 2026-10-31 §9);
the disc and the dock buttons carry the Record control's labels ("Start recording", "Stop and review the take",
"Resume" is not needed: there is no pause of the take, only of the prompter). The countdown numbers are announced.

### 2.3 The prompter band

| Element | Spec | Evidence |
|---|---|---|
| Height | 3 lines of `type.prompter` + `space.5` top and bottom ≈ 3 × 32 + 24 = **120 pt**; at font scale ≥ 1.3 two lines; the band never grows past 40 % of the height (the preview must stay a face). | HIG/typography: "If you need to display three or more lines of text, avoid tight leading even in areas where height is limited." |
| Type (PROPOSED token) | **`type.prompter` 26 / 32, Inter 600, cap 1.6**, `label.1`; the previous and next lines `label.3` at the same size (a dim step, not a smaller size, so the eye never re-focuses). Why 26: at arm's length (≈ 60 cm, INFERRED) 26 pt subtends about what 17 pt does at reading distance (≈ 40 cm): 26/60 ≈ 17/40. Why 600: "In general, avoid light font weights"; the punch is **700 with the amber dot** in a 14-pt gutter (the page's mark, 3c §3.1), so the punch is weight + dot + its place, never colour alone. | HIG/typography: "People need to be able to read your content at various viewing distances and under a variety of conditions"; "Keep in mind that font weight can also impact how easy text is to read." (CONFIRMED) |
| The current line mark | ► in the gutter (`label.1`) on the current line; the line itself bright. Under Reduce Motion the swap is a cut; otherwise a 120-ms fade (`useReduceMotion`, CONFIRMED `ui.tsx:69`). | HIG/accessibility: "Replacing transitions in x-, y-, and z-axes with fades to avoid motion" (CONFIRMED). |
| v1 advance | `rules/prompter.ts` `nextAt(line, wps)`: the current line's word count ÷ his words-per-second, min 1.2 s; **Hold** freezes the timer; **Next line** advances at once and re-arms. The last line holds until he stops. The timer never runs while idle. | D3. |
| v2 advance (the voice) | The live recognizer's interim transcripts (`continuous: true, interimResults: true, requiresOnDeviceRecognition: true`, CONFIRMED options `ExpoSpeechRecognitionModule.types.d.ts:183,204`) feed `rules/prompter.ts` `follow(state, heardWords)`: the last three heard words are matched against a window of the current line and the next two (lower-cased, punctuation stripped, one edit allowed per word); a match moves the **current word**; a match at a later line advances the line. The **current word** carries a 2-pt amber underline and the words before it dim to `label.3` (two cues: colour and the dim step). The off-script rule is D12. Falls back to v1 silently if the recognizer fails mid-take (the band's header shows a `mic.slash` glyph with the one word "voice off"). | HIG/color "Convey information with more than color alone". |
| Words he adds | Nothing happens on the band (D12); the captions carry them. | |
| The last line | Stays on the band with ► until he stops; the next slot reads nothing (no "end" sentence). | The clutter rule. |

### 2.4 Review

The take plays in `expo-video`'s `VideoView` with `nativeControls` (HIG/playing-video: "Use the system video player
to give people a familiar and convenient experience", CONFIRMED), `contentFit="contain"`, `loop`, sound on. **The
captions are drawn over it as a `Text` overlay from the word timings** (the player's `timeUpdate` at 0.1 s,
CONFIRMED `timeUpdateEventInterval` in the SDK 58 docs) in the exact look the burn will use, so what he sees is what
Keep saves. Under the player one status line: "0:48 · 112 words", or "0:48 · no words" (§2.9), or "0:31 · cut short
by a call". The dock: **Redo** · **Keep**.

**Keep** does, in order, with the progress bar: (1) burn the captions into a new file (the local Swift module, §4);
(2) `createAssetAsync` → the **Punchable** album (`getAlbumAsync('Punchable')` or `createAlbumAsync('Punchable',
asset)`, CONFIRMED `expo-media-library/build/legacy/MediaLibrary.d.ts:385,456,472`); under **limited** Photos access
no album is possible ("You can't create or fetch user albums", Apple, Delivering an Enhanced Privacy Experience in
Your Photos App, CONFIRMED read live) so the take goes to the library and the snack says "Saved to Photos"; (3) write
the take row on the phone (`Documents/takes/index.json`, the recordings index's pattern, `lib/recordings.native.ts:1-16`)
and queue the words (§5, §6); (4) `success` haptic; (5) close to the bit page with the snack **"Saved to Photos ·
Punchable · Undo"** (Undo within the snack deletes the asset and the row; after that the row's own Delete is the
path, non-timed, HIG.md §7). The cache file is deleted after the save.

### 2.5 The sessions: Make it a rant, Pick a hook (in the sessions sheet, `/session/[bit]`)

The sheet's whole-bit rows (3c §3.6; old spec S2) become **Gather · Break it down · Find the turn · Make it a rant ·
Pick a hook · Talk**. Each row is one verb phrase; the one-line description under it is the old spec's pattern.

| Mode | What Claude returns | What he does | Saved as |
|---|---|---|---|
| **Make it a rant** | A full set of hunks over the cut (old spec S3: whole lines; old struck, new under; the text never coloured): second person, present tense, escalation line to line, the punch kept as the last beat, tags folded after it. The AI badge on the sheet. | **Keep / Skip per line**; **Save rant cut** (the button's label says the cut; not rendered until a change is kept, 3c §3.6). | A new version with `line_meta.cut = 'rant'` (D10); the snack "Saved as v9 · rant · Undo". The stepper reads "v9 of 9 · Nov 3 · rant". |
| **Pick a hook** | **Three openers** for the cut being filmed, each one line, each with the AI badge; the pattern of Find the turn (old spec S7: rows, thumbs up / down, **Write it in**, footer **Show others**). "Use this one" is the row's Write it in. | Picks one (or none; "Show others" asks for three more, once). | The pick = a tracked change inserting line 0; Keep → Save version (same cut). `hook_rating` rows (§5) hold the three shown, their order and his pick. |

Both grey with the one word "offline" or "paused" (3c §3.13). Both run only after Slice 4 (sessions, T8) exists;
until then the rows are **not rendered** (a control that leads nowhere is not shown; 3c Slice 1's rule). From the Film
it screen, the cut picker's last item reads **"Make a rant cut…"** when none exists: it closes Film it and opens the
sheet on the bit page (one modal at a time, HIG/modality "Let people dismiss a modal view before presenting another
one", CONFIRMED).

HIG/generative-ai (CONFIRMED): "Clearly identify when and where you use AI" (the badge on every hunk and opener);
"present a single result or multiple meaningfully different results from which people can choose" (three openers);
"Give them the ability to dismiss new content they don't want, and revert or retry" (Skip, Show others, Undo on the
save); "Always make providing feedback voluntary" (thumbs are optional).

### 2.6 The versions stepper with cuts (3c §3.5, extended)

Collapsed: "Versions · v9 · Nov 3 · 1:40" (unchanged: the newest **stage** version). Open: the label gains the cut
word when it is not stage: "v9 of 9 · Nov 3 · rant · Make it a rant". The page's lines re-render from the stepped
version; on a rant version a 44-pt row above the lines reads **"Rant cut"** with the chip (a word, not a colour), and
Write on it writes a rant version. "Restore v9 as a new version" restores into its own cut. VoiceOver: "Version 9 of
9, November 3, rant cut, from Make it a rant."

### 2.7 The On tape row and screen (3c §3.7, extended)

Row meta: **"On tape · 3 nights · killed twice · filmed 2"**; at font scale ≥ 1.3 the meta wraps under the label as
every row does. "filmed N" counts kept takes. A bit never performed but filmed reads "On tape · filmed 1". The push
screen `/joke/[id]/tape` gains a **Filmed** section under the performed rows:

| Row | Reads | Tap | Long press / actions |
|---|---|---|---|
| A take | "Filmed · Nov 3 · 0:48" · second line "rant cut v9 · 112 words"; a 44-pt leading **play** glyph (plays the Photos asset full-screen in `expo-video`; "not in Photos" greyed on the glyph when the asset is gone, the words stay) | **Said vs written**: the cut's lines with the take's words diffed against them in the S3 look (a changed word underlined, a skipped line struck with "−", added words with "+"; the text never coloured) from `GET /api/bits/{id}/takes/{take}/diff` (§6); offline, the phone's own word list (no diff). | Play · Open in Photos · Pull the words into the pile (Q6) · — · **Delete the words** (destructive, last; the video in Photos is his to delete there; the sheet says which). |

### 2.8 Photos, the album, permissions (HIG/privacy, CONFIRMED read live)

| Permission | When asked | Purpose string (the plugin key) | Denied |
|---|---|---|---|
| Camera | The first time Film it opens (the preview needs it); never at launch. | `expo-camera` `cameraPermission`: "Punchable films you reading a bit so you can post it. The video stays on your phone." | The preview area shows one line "Punchable can't use the camera." and one button **Open Settings** (`Linking.openSettings`, the pattern of `record.tsx:187-192`); the disc greys with "no camera". |
| Microphone | Already granted for sets and memos (`app.json:22`); if not, asked with the camera. | the existing string | The disc greys "no mic"; the preview stays. |
| Speech recognition | Already asked on the first transcription (SPEC 2026-10-31 §8); in v2 asked on the first Film it with the voice. | the existing string (`app.json`, `expo-speech-recognition`) | v1: Keep saves **without captions** and the status reads "0:48 · no words"; v2: the prompter falls back to v1 with "voice off". |
| Photos (add + read) | On the first **Keep** (not when Film it opens: "wait to request permission until people actually use an app feature that requires access", HIG/privacy). Today's Capture badge already asks read access for videos (CONFIRMED `lib/photos.native.ts:9-17`). | `expo-media-library` `photosPermission`: "Punchable saves the videos you film to a Punchable album in Photos and finds set-length videos you taped elsewhere." `savePhotosPermission`: "Punchable saves the videos you film to Photos." **The plugin is not in `app.json`'s `plugins` today (CONFIRMED `app.json:83-119`) and the only usage string there is the microphone's: the builder adds the plugin entry with both strings and verifies the keys land in the built Info.plist** (UNKNOWN whether autolinking applied a default string in builds 9-20). | **Denied**: Keep is greyed "no Photos" and the reason row has **Open Settings**; the take is not lost until he leaves. **Limited**: saved to the library, no album (§2.4). |

HIG/privacy on the strings: "Aim for a brief, complete sentence that's straightforward, specific, and easy to
understand. Use sentence case, avoid passive voice, and include a period at the end." (CONFIRMED). No custom
pre-permission screen: the context (a camera screen) is the explanation.

### 2.9 Error and edge states (the one-word reason rule; no sentence explains itself)

| State | What shows | What happens |
|---|---|---|
| Camera or mic denied | §2.8: one status line and **Open Settings**; the disc greyed with "no camera" / "no mic". | Nothing records. |
| Low storage (< 500 MB free, `expo-file-system` legacy `getFreeDiskStorageAsync`, CONFIRMED `build/legacy/FileSystem.d.ts:93`) | The disc greyed with "no room"; status "Ready · low storage". | He frees space; the screen re-checks on return to the foreground. |
| **A call while recording** | The capture session is interrupted and the take **ends** (the camera cannot resume a file; INFERRED from AVFoundation, the spike confirms): review opens with "0:31 · cut short by a call". The clock's word is "Paused by a call" for the second it takes to land there. | Keep or Redo as usual. There is no 60-pt Resume here (nothing can resume), so the review is the honest state, the same reasoning as `recorderLogic.ts` "failed → idle". |
| Lock, background, app switch | The take ends the same way (a camera stops in the background); review on return. | |
| No speech in the take (v1 transcription `no_speech`) | Review status "0:48 · no words"; Keep saves the video uncaptioned; the row reads "Filmed · Nov 3 · 0:48 · no words". | No sentence; the state is the fact. |
| Recognition unavailable on this phone (`unavailable`) | As above, plus Settings › Connections › Transcription already says "Off · …" (SPEC 2026-10-31 §8). | |
| **The one-minute recognition limit** | On-device recognition has no one-minute limit (WWDC19 session 256, INFERRED, SPEC 2026-10-31 §1 row "Long sets"); the v1 file route transcribes the whole take. **v2 live**: if the spike finds the live session ends at ~60 s, `lib/prompter.native.ts` restarts it at 50 s and stitches the word clock (the restart's `start` event marks the new zero; `recordingOptions.persist` "marks the start of the file to within tens of milliseconds", CONFIRMED `types.d.ts:139`). The band shows nothing during the restart; a missed word is caught by the next match. | UNKNOWN until the spike. |
| A take longer than 10 min | `maxDuration: 600` ends it; review opens with "10:00". | A rant is a minute; the cap is a guard, not a feature. |
| Burn-in fails | The review stays with an `ErrorCard` "Couldn't add the captions: <reason>." and two buttons **Keep without captions** · **Try again**. | The take is still in the cache. |
| Photos save fails (full, denied mid-way) | `ErrorCard` "Couldn't save to Photos: <reason>. The take is still here." · **Try again**. | |
| The album was deleted by him | Recreated on the next Keep. | |
| The asset deleted in Photos | The On tape row's play glyph greys "not in Photos"; the words stay; Delete the words is the only destructive action. | |
| Offline | Everything works but the words wait: the row reads "Filmed · Nov 3 · 0:48 · waiting for a connection" (the memo queue's words, `rules/memoRules.ts:219`). The two sessions grey "offline". | |
| Reduce Motion | Line swaps cut instead of fade; the countdown still counts (numbers, not motion); the burn progress is a bar. | |
| Dynamic Type at the caps | The band drops to two lines at ≥ 1.3; the preview shrinks (it is `flex: 1` under the band); the dock words wrap to two lines before they truncate; order never changes. | HIG/typography. |

---

## 3. Copy (plain words, sentence case; nothing explains itself)

| Where | Copy |
|---|---|
| More menu | **Film it…** · disabled subtitle "nothing to read" |
| Bar | title **Film it** · trailing **Rant ▾** / **Stage ▾** · Close |
| Cut picker (action sheet) | title the short name · "Rant cut · v9 · Nov 3" · "Stage cut · v8 · Nov 2" · "Make a rant cut…" (when none) · Cancel |
| Status words | Ready · Starting… · Recording · Riffing (v2) · Paused by a call · Saving… |
| Dock | **Hold** / **Go** · **Next line** · **Redo** · **Keep** |
| Disc labels (VoiceOver) | "Start recording" · "Stop and review the take" |
| Review status | "0:48 · 112 words" · "0:48 · no words" · "0:31 · cut short by a call" |
| Leave sheet | title "Leave the take?" · message "0:48 recorded." · **Discard the take** (destructive) · **Keep the take** · **Keep going** (cancel) |
| Snacks | "Saved to Photos · Punchable · Undo" · "Saved to Photos · Undo" (limited access) · "Take discarded" |
| Reasons (greyed controls) | no camera · no mic · no room · no Photos · offline · paused · nothing to read · voice off · not in Photos |
| Errors | "Punchable can't use the camera." + Open Settings · "Couldn't add the captions: <reason>." + Keep without captions · Try again · "Couldn't save to Photos: <reason>. The take is still here." + Try again |
| Session rows | **Make it a rant** "Claude turns it to face the camera and build; you keep what you like." · **Pick a hook** "Three openers; pick one or none." |
| Session buttons | Keep · Skip · **Save rant cut** · **Use this one** · **Show others** |
| Stepper | "v9 of 9 · Nov 3 · rant · Make it a rant" · chip **Rant cut** |
| On tape | "On tape · 3 nights · killed twice · filmed 2" · "Filmed · Nov 3 · 0:48" · "rant cut v9 · 112 words" · **Said vs written** · "not in Photos" |
| Purpose strings | §2.8 |

Invented sample bit used in the frames ("Self-checkout", stage cut → rant cut):

| Stage cut | Rant cut (Claude's hunks, each kept or skipped) |
|---|---|
| Self-checkout is the store hiring you for free. | You know what self-checkout is? It's the store hiring you. For free. |
| I scanned forty things and nobody clapped. | You scan forty things. Nobody claps. |
| At my job I at least get a chair. | At your actual job you get a chair. |
| (punch) Unexpected item in the bagging area: my dignity. | (punch) "Unexpected item in the bagging area." Yeah. It's my dignity. |

A hook Claude might offer for it (three shown, one picked): "Stop scanning your own groceries." · "I want to talk to
whoever invented self-checkout." · "I got fired today. From a store I don't work at."

---

## 4. Native feasibility and the spike's questions (ios-dev-expert owns the answers)

**What is installed** (CONFIRMED `app/package.json`): `expo-media-library` 58.0.13 (the Photos save and album,
`legacy` API in use at `lib/photos.native.ts:3`), `expo-speech-recognition` 58.0.1 (on-device file and live
recognition), `expo-audio` 58.0.11 (the recorder), `expo-keep-awake`, `expo-haptics`, `expo-file-system`.

**What is not, and what that means** (CONFIRMED absent from `package.json` and `app/node_modules`):

| Need | Module | Binary |
|---|---|---|
| The front camera, 9:16 video | **`expo-camera`** (SDK 58: `CameraView` with `facing="front"`, `mode="video"`, `mirror`, `mute`, `active`, `onCameraReady`; `recordAsync({maxDuration, codec: 'avc1'})` resolves on `stopRecording()`, on `maxDuration`, or when the preview stops; the file lands in the cache; the config plugin sets `NSCameraUsageDescription` and `NSMicrophoneUsageDescription`; CONFIRMED read in the SDK 58 docs) | **A new native module: a new TestFlight binary** (the `expo-speech-recognition` precedent, HANDOVER item 4: an older dev client treats it as unavailable, so the import is lazy as in `lib/transcribe.native.ts:16-21`). |
| Playing the take (review, the On tape row) | **`expo-video`** (`VideoView`, `useVideoPlayer`, `nativeControls`, `contentFit`, `loop`, `currentTime`, `timeUpdateEventInterval`; CONFIRMED SDK 58 docs) | Same binary. |
| **Caption burn-in** and **audio-track extraction** | **No Expo module does either.** A **local Expo Module** in Swift (`app/modules/punchable-takes/`, `expo-modules-core`): `burnCaptions(inputUri, captions: [{text, s, e}], style) → outputUri` with `AVMutableComposition` + `AVVideoCompositionCoreAnimationTool` (`CATextLayer` per caption, the look of D9) + `AVAssetExportSession` (1080p preset), progress events; `extractAudio(inputUri) → m4aUri` with `AVAssetExportSession` (`AVAssetExportPresetAppleM4A`), the TODO already written at `lib/photos.native.ts:43`. Pure Swift, no third-party pod. | Same binary. **The one piece of native code this feature adds**; it is also what the paid backup and the set-video import (the audio-only upload) have been waiting for. |
| Photos album | `expo-media-library` legacy `createAlbumAsync` / `getAlbumAsync` / `addAssetsToAlbumAsync` (CONFIRMED `legacy/MediaLibrary.d.ts:385,408,456,472`); the plugin keys `photosPermission`, `savePhotosPermission` (CONFIRMED `plugin/build/withMediaLibrary.js:55-61`) | No new module; **the plugin entry is new in `app.json`** (§2.8). |

**The spike must answer** (each with a `file:line` in `node_modules` or a device result; v2 waits on 1-4, v1 on 5-7):

1. **Can `SFSpeechRecognizer` (live, on-device) run while `expo-camera` records with sound?** Both want the
   microphone through `AVAudioSession`; expo-camera configures its own session (UNKNOWN), expo-speech-recognition sets
   a category (`iosCategory`, CONFIRMED `types.d.ts:280`). If they fight: (a) the camera records **muted**
   (`mute`) and the recognizer's `recordingOptions.persist` writes the audio file (CONFIRMED `types.d.ts:242,314-326`),
   muxed into the video by the Swift module; or (b) the camera keeps the sound and the recognizer taps the session's
   input. The answer picks the v2 audio path.
2. **Word alignment**: do the live `segments` carry timings from the session start, and how far does the camera's
   first frame sit from the recognizer's `start` (the offset for captions)? Tens of milliseconds is fine; hundreds
   need the persisted-file route (a) and a cross-correlation we do not want to write.
3. **The one-minute limit** on the live on-device session (§2.9 row): real or not on iOS 26.
4. **Eye line**: on the iPhone 17 the front lens sits in the Dynamic Island's left half; is a 120-pt band under the
   bar close enough that a reader's gaze reads as "at the lens" in the saved file? A device photo decides the band's
   height and whether the bar should be thinner (the system status bar hidden, the title dropped).
5. **File transcription of the extracted m4a**: `AVAudioFile` opens it (the file route reads with `AVAudioFile`,
   CONFIRMED `expo-speech-recognition/ios/ExpoSpeechRecognizer.swift:311`); a `.mov` would not, hence the extraction.
6. **A call during `recordAsync`**: the promise resolves with the partial file (hoped) or rejects (then the cache
   file is still there and the screen re-opens it): which, on the device.
7. **Burn-in time** for a 60-s 1080p take on the iPhone 17 (the progress bar is honest either way).

---

## 5. Data model deltas (SurrealDB, hosted; pk-api's own tables, the 3c §5 rules)

| Delta | Shape | Why |
|---|---|---|
| **`line_meta.cut`** (on the 3c row) | `cut: string` ∈ {`stage`, `rant`}, default `stage`; written by `POST /api/bits/{id}/versions` (`cut` in the body; carried over from `base_unit` when omitted). | D10. The page's "newest version" becomes "newest with `cut = stage`" in `versionsOf` (CONFIRMED today's split, `joke/[id].tsx:39-50`), one rule in `rules/bitLines.ts`, tested. |
| **`take`** (new table, `db/schema_film.surql`) | `bit: record<joke>`, `cut_unit: record<unit>`, `cut: string`, `recorded_at`, `duration_s: float`, `words: array<{w: string, s: float, e: float}>` (seconds from the first frame), `word_count: int`, `captions: bool`, `engine: string`, `asset: {kind: 'photos', id: string, device: string}` (the Photos local identifier and which phone; never a file), `cut_short: option<string>` ('call'), `by`, `at`. One row per kept take; **no audio, no video, ever**. | The On tape entry and the said-vs-written diff. |
| **`hook_rating`** (new) | `session`, `turn`, `shown: array<{text, rank}>`, `picked: option<int>`, `written_in: bool`, `at`. | Preference data, the `turn_rating` twin (old spec §5.2). |
| **`session.mode`** | gains `rant` and `hook`. | §2.5. |
| **The take index on the phone** (not the database) | `Documents/takes/index.json`: `{id, bit, cut_unit, cut, recorded_at, duration_s, words, asset_id, state ∈ waiting · sending · sent · failed, attempts, next_at, error}`; the recordings index's pattern (`lib/recordings.native.ts`, writes serialised, file names only). The On tape row reads the phone's rows **and** the server's, deduped by `id` (a uuid minted on the phone, the idempotency key). | D14: the row shows the take the moment he keeps it, signal or not. |

What does not change: `unit`, the versions route's contract, the sessions tables beyond the two mode values.

---

## 6. API routes (pk-api, Rust; owner-only; `OWN_PATHS`; scratch-DB tests; the 3c §6 rule)

| Route | Does | Idempotent |
|---|---|---|
| `POST /api/bits/{id}/takes` | Writes a `take` row from the phone's record (**words and timings only**; the body is refused if it carries a `uri`, a `file` or more than 4,000 words); runs the legal rules over the words; returns `{ok, take, indexes}`. | **`X-Idempotency-Key` = the take's uuid** (the 3c `write_receipt`). |
| `GET /api/bits/{id}/takes` | The bit's takes, newest first, with `word_count`, `cut`, `asset.device`. | — |
| `GET /api/bits/{id}/takes/{take}/diff` | The take's words against its `cut_unit`'s lines: `{lines: [{i, text, said: 'same' \| 'changed' \| 'skipped', words: [[from, to]]}], added: [{after_line, text}]}` from the `similar` crate (the 3c line diff's sibling), words normalised as the prompter does. | — |
| `DELETE /api/bits/{id}/takes/{take}` | "Delete the words": the row goes; the video is his in Photos. | key |
| `GET /api/bits/{id}/tape` | Gains `filmed: int` and the take rows (3c §6.1 row 2 extended). | — |
| `POST /api/bits/{id}/versions` | Gains `cut?` in the body (§5). | key |
| `POST /api/sessions {bit, mode: 'rant' \| 'hook', base_unit}` and the session routes | Old spec §7.2 unchanged; `hook` writes `hook_rating` on `POST /api/sessions/{id}/ratings {shown, picked}`. | key |

The phone's queue sends `POST …/takes` with the memo queue's backoff (`rules/uploadsCore`), a 401 through
`handle401`, a refusal → `failed` with the words kept on the phone (the row says so, Try again).

---

## 7. Worker jobs (pk-worker only; after the T8 swap, ADR 0011; `[tasks.*]` in `engine/config.hosted.toml`)

| Kind | Task (model) | Prompt (system blocks in cache order) | Output | Checks in pk-api |
|---|---|---|---|---|
| **`session_rant`** | `session_rant` (Opus 5.5) | 1 method (public, invented examples: direct address, present tense, one escalation per line, the punch stays last, tags after) · 2 profile + learned (cached) · 3 standing rules · user: the cut with numbered lines, the punch index | hunks by line number (the `session_tighten` shape, old spec §7.3) | every `removed` line equals the base; the punch line's reveal word survives (the 3c verbatim rule on the reveal, not the whole line); rule checks; legal rules |
| **`session_hook`** | `session_hook` (Opus 5.5) | the same 1-3 · user: the cut, three lines max each, "an opener that earns the first line" | `[{text, why}]` × 3 (the Find the turn shape) | ≤ 24 words each; none equal to a line of the cut; legal rules |
| Takes, the diff | **none** | — | — | $0: no model touches a take |

---

## 8. Cost per call (prices CONFIRMED `rust/crates/pk-llm/src/budget.rs:43-45`; token counts INFERRED until measured)

| Call | Model | Per call (warm cache) |
|---|---|---|
| Film it, Keep, the burn, the Photos save, the words | none | **$0** |
| Said vs written | none (`similar`) | **$0** |
| Make it a rant (a whole cut, ≈ 6k cached read + ≈ 1.5k uncached in, ≈ 1.5k out with thinking) | Opus | ≈ **$0.06-0.08** (the whole-bit Tighten figure, 3c §9) |
| Pick a hook (≈ 1k in uncached, ≈ 300 out) · Show others | Opus | ≈ **$0.04-0.05** each |
| First call in a cache window | + ≈ $0.03 | |

An evening of one rant cut and two rounds of hooks is about twenty cents; filming is free. The daily cap ($2.00,
CONFIRMED `engine/budget.py:21`) is untouched by takes.

---

## 9. Privacy

- **The video never leaves the phone.** Punchable writes it to Photos and keeps no copy (D8); the share sheet is
  Photos' own, his choice, outside the app. The clean take sits in the app cache only between stop and Keep / Redo,
  then is deleted.
- **The audio never leaves the phone.** Extracted and transcribed on the phone (`requiresOnDeviceRecognition`, the
  JS refuses to start otherwise, CONFIRMED `lib/transcribe.native.ts:110-114`).
- **The server receives**: the take's words with per-word seconds, its duration, the cut it was read from, a Photos
  asset id (a string that is meaningless off that phone) and the device name; for a session, the cut's lines (his
  words, already on the server) and nothing from the camera. `POST …/takes` refuses a body with a file. The legal
  rules run over the words as over every memo (`server.py:1667-1670` pattern).
- **Purpose strings** say exactly this (§2.8). HIG/generative-ai (CONFIRMED): "making sure people know their
  information may be sent to a server, showing them what's shared": the session sheet's existing offline/paused copy
  already names Claude; the Privacy row in Settings (SPEC 2026-10-31 §8) gains "Videos you film · Stay in Photos on
  this iPhone. Only the words go to Punchable."
- The public repo holds invented jokes only; the commit guard runs. App Store 5.1.2(i) consent before any external
  tester (unchanged).

---

## 10. Gates and the owner's phone checks

Every build: `cd app && npx tsc --noEmit && npm run test:unit && npx expo-doctor`; `python tools/guard/check.py
--staged` before every commit; a failing-capable check for every rule; nothing private; LF. Rust: `cd rust &&
PK_REQUIRE_EXTERNAL=1 cargo xtask gate` plus the route's scratch-DB test. **No time estimates.**

| Gate | Red arm it must be able to fail on |
|---|---|
| `tests/prompter.test.mjs` (v1 pacing) | a 12-word line at 2.4 w/s must arm for 5.0 s; Hold must stop the timer; Next line past the last line must stay on the last line |
| `tests/prompter.test.mjs` (v2 follow) | six off-script words must **not** advance; a repeated word ("the the") must not skip a line; a match two lines ahead must land there; three matched words after a riff must re-sync |
| `tests/takeWords.test.mjs` | no caption over 4 words or 1.6 s; a caption never crosses a matched line end; per-word timings monotonic; a take with no finals → `no_words` |
| `tests/takeRules.test.mjs` | the state machine (idle → countdown → recording → review → saving → done; a call → review with `cut_short`); a second Keep while saving is a no-op |
| pk-api `tests/takes.rs` | a body with `uri` must 400; a repeated idempotency key returns the first response and writes nothing; the diff marks a changed word, a skipped line, an added phrase |
| pk-worker scratch test (`session_rant`) | a hunk whose `removed` line is not in the base is dropped; an output that loses the reveal word is refused |
| The static gates | `static.values` (new literals become tokens: `type.prompter`, the band height, the caption style live in `src/ui`), `static.worklets`, `selfImport` |

**Owner checks on the phone** (per build; each a pass / fail he can say in one line):

- v1: open a bit › More › Film it: the camera asks once, the preview is his face, the band shows three lines with
  the punch bold and dotted; the disc counts 3-2-1 with a tick he feels, then records; the lines advance at about his
  pace, Next line jumps, Hold holds; the square opens the review with the captions playing over his words; Keep →
  "Saved to Photos · Punchable", the album exists in Photos, the video has the captions burned in and is not
  mirrored; the bit's On tape row reads "filmed 1" and the row plays it; in airplane mode the whole thing still works
  and the row says "waiting for a connection"; a call during a take lands on the review with "cut short by a call";
  Reduce Motion on: the lines cut, no fade; text size at AX1: the band is two lines, the dock words wrap.
- v2: read a take while riffing one line: the band holds and says Riffing, then catches up; skip a line on purpose:
  the band follows; the current word's underline keeps up at his natural pace; captions match what he said, not the
  script.
- v3: Make it a rant → the hunks per line with the badge, Keep three, Skip one, Save rant cut → "v9 · rant" in the
  stepper and the page still shows the stage cut; Film it opens on the rant cut with "Rant ▾"; Pick a hook → three
  openers, Use this one → line 0 as a draft, Save version; On tape › a take › Said vs written shows the skipped line
  struck.

---

## 11. Build order (v1 / v2 / v3; v2 and v3 are independent and ship in whichever order unblocks first)

### v1: Film it with the steady prompter (one new binary; no server change required to film)

| | |
|---|---|
| What | `app/film/[id].tsx` (§2.2 idle · countdown · recording · review · saving; v1 advance; the cut picker shows only "Stage" until v3); `lib/Prompter.tsx` (the band) with `rules/prompter.ts` (pacing) + tests; `lib/takes.native.ts` (the take index, the Keep pipeline: extract → transcribe with per-word segments → captions → burn → Photos → album → queue) with `rules/takeWords.ts` and `rules/takeRules.ts` + tests; the Swift module `app/modules/punchable-takes/` (`burnCaptions`, `extractAudio`, progress); `expo-camera` + `expo-video` added; `app.json`: the camera plugin, the media-library plugin with both strings; the More menu item; the On tape row's "filmed N" and the Filmed rows from the phone's index (play via `expo-video`); the Privacy row line. `type.prompter` as a token (DESIGN.md §4 amended, §14 here). |
| Needs | The new binary (owner's go to build); the spike's answers 5-7 (or the device check does them: Film it is its own spike). Nothing from the server: takes sit on the phone as `waiting` until `POST …/takes` exists, and the row reads from the phone. |
| Routes | None required. If pk-api's `takes` routes land in the same window, the queue starts sending. |
| Risks | expo-camera's audio session vs expo-audio's `setAudioModeAsync` (CONFIRMED `lib/recorder.native.ts:35`: `doNotMix`, background recording): the recorder's mode must not be active when the camera opens (the memo auto-start is the precedent for ordering); the Swift module is new native code (small, pure AVFoundation, no pod); `createAlbumAsync` under limited access (handled, §2.4); the cache file surviving a crash between stop and Keep (re-opened from the cache on the next launch with "a take from Nov 3 is still here" as a row on the bit page, or deleted after 24 h: builder's call, say which). |
| Gate | §10 v1 rows; **owner checks v1**. |
| **Built** (2026-11-03, branch `app/film-it-v1`, not pushed, not built; reviewed by ios-dev-expert and ios-design-expert, must-fix items fixed) | **On the spike's v1 path (the lead's calls on §9a of the spike), with this spec's screens and copy.** What is in: `expo-camera` 58.0.14 + `expo-video` 58.0.13 (both loaded lazily: an older binary reads "Not in this build yet."), the camera and media-library plugins with the strings of §2.8 (the Info.plist keys verified by `expo config --type introspect` on a scratch copy: Windows cannot generate the iOS project); `app/film/[id].tsx` (idle · countdown · recording · review · saving; `rules/filmRules.ts` + `tests/filmRules.test.mjs` with a red arm); the band `lib/Prompter.tsx` on `rules/prompterRules.ts` (+ tests, red arm); "Film it…" in More (phone only, greyed "nothing to read"); the On tape row's "filmed N" and the **Filmed** rows on `/joke/[id]/tape` (`RecRows` kind `film`; play from `/take/[rec]`); the recordings queue's kind `film` with per-word timings (`memoRules addFinal` keeps the segments' words); the `.srt` sidecar; the words up through today's routes (an idea filed on the bit, `routeOf`); the Privacy row line; `type.prompter` and `size.band` in DESIGN.md; the HIG.md §3.9 sibling. **Deviations from this spec, each the spike's side or a v1 cut:** (1) **`hvc1`**, not `avc1` (D4; §9a). (2) **No burn-in and no Swift module in v1**: the captions are an `.srt` beside the take (Share captions… on the row); the review player shows no caption overlay because the words arrive after Keep from the queue (§9a; burn-in is v1.5). (3) **No audio extraction**: the `.mov` goes through today's file transcriber (D13; the owner's D3 decides; if `AVAudioFile` refuses it the row fails with the file kept and the 30-line Swift export is the fix). (4) **Documents while in flight, then one copy in Photos**: the `.mov` moves to `Documents/recordings` the moment the take stops, Keep adds it to Photos, the Documents copy is deleted once the words exist (the `.srt` stays); Redo deletes it (D8; §9a). (5) **Add-only Photos access, no album** (§2.4, §2.8; the spike's §6): the snack reads "Saved to Photos · 0:48"; under add-only `createAssetAsync` cannot read the asset back, so `saveToLibraryAsync` is used and the row has no asset id unless full access was already granted; therefore **no Undo on the save snack** and **no "not in Photos" greying** (both need read access: v1.5). (6) **The band**: the current line at the TOP, at most four lines (the spike's §5 eye-line table), not the previous · current · next of D2; stepped, never a pixel scroll (D2 kept); the ► marker is dropped (the top position is the mark; the punch dot keeps the gutter). (7) **The review's status is "0:48" / "0:31 · cut short by a call"**; "112 words" / "no words" appear on the On tape row once the words exist. (8) **Lock / background reads "cut short"** (a call reads "cut short by a call"); the take lands on the review on return. (9) The More item pushes `/film/[id]` with no `?cut=` (v3). (10) A take's words are filed on the bit as an idea (today's route) and that note is kept OUT of the pile (§12 Q6's default: the words live on the On tape row). Not built (v1.5+): burn-in, the Photos album, Undo on the snack, the caption overlay on the review, the speed control, the cut picker, the sessions. **From the two reviews (must-fix items fixed; these are the accepted calls):** the camera view mounts only after the grant (a session started under the prompt fails its permission check once and never recovers); a denied mic keeps the preview and runs the view muted; the review's audio mode becomes playback before the player is made (the capture leaves `playAndRecord` behind; expo-video keeps it as is); the lock's own stop can land before ours, so a file back while the app is not in front reads "cut short", and the ten-minute cap is not cut short at all; a `.mov` the recognizer cannot open reads "no words" (not "Couldn't transcribe") and the Documents copy stays beside the Photos one until the Swift export exists; the clock pill over the preview keeps the dark tokens in both appearances (over video, which has no scheme); the idle dock's left slot is empty (a greyed Hold would owe a reason); the countdown scrim is 50 %; "Opening…" until the camera is ready; a mount error reads "Couldn't open the camera." with Try again (a remount), not Open Settings; the offline strip is off on the camera (it would sit on the eye line and say something untrue; `offlineAt="none"`); the dock words wrap to two lines; the status bar is hidden for the modal's life (the Camera app's pattern; reclaims nothing under the island, flip if D6 says the battery read matters); the bar stays 52 pt (the current line's centre is ~18 mm under the lens, ~3° at 35 cm; D7 moves the type size, not the bar); the `medium` haptic on the first frame stays until D1 says it is audible in the file; `barcodeScannerEnabled: false` (nothing scans; a smaller binary). |

### v2: the prompter follows his voice (depends on the spike)

| | |
|---|---|
| What | `lib/prompter.native.ts`: the live recognizer (`continuous`, `interimResults`, on-device; the restart rule if the limit is real) feeding `rules/prompter.ts` `follow`; the current-word underline; Riffing; "voice off" fallback; live captions from the finals when the spike says the timings are good enough (else v1's after-the-take captions stay and only the following is live). |
| Needs | Spike answers 1-4; speech permission already granted (else v1 behaviour). No new binary if the answer is route (b); route (a) (muted camera + persisted audio, muxed) needs `muxAudio` added to the Swift module: a new binary. |
| Gate | §10 v2 rows; **owner checks v2**. |

### v3: the rant cut, the hook, said vs written (needs Slice 4 of 3c: sessions, the T8 swap, pk-api deployed)

| | |
|---|---|
| What | `db/schema_film.surql` (`take`, `hook_rating`; `line_meta.cut`) with his go (ADR 0009); pk-api `routes/takes.rs` (§6) + `cut` on the versions route + the diff; pk-worker `session_rant`, `session_hook`; the sessions sheet's two rows; the stepper's cut word and the "Rant cut" row; the cut picker in Film it; **Said vs written** on the take row; `versionsOf` by cut. |
| Needs | 3c Slice 3 (the receipt route and `line_meta`) and Slice 4 (the session sheet); T8; the pk-api deploy order (the Python with `GET /api/ai/worker` first, then pk-api's Ingress). |
| Gate | §10 v3 rows; the Rust gate; the API differential unchanged (new routes have no twin); **owner checks v3**. |
| **Built, the rant half** (2026-11-06, branch `app/film-it-rant`, TestFlight build 24; not merged, not pushed, not deployed; reviewed by ios-design-expert, ios-dev-expert and rust-application-architect, must-fix items fixed) | **The smallest slice that makes the rant usable on the phone, on today's servers (no Slice 3 / Slice 4, no new table).** What is in: two hosted Claude job kinds, **`rant`** and **`hook`** (`creative/rant.py`; tasks `creative_rant` / `creative_hook`, Opus, the punch-up's prompt pattern: voice profile, learned block, the task line with the public method and invented examples, the numbered lines), in the **Python worker** and ported as-is to **pk-worker** (`kinds/rant.rs`, 5 recorded differential scenarios); the routes **`POST /api/jokes/<id>/rant {unit, punch?, fresh?}`** and **`/hook {unit, exclude?, punch?, fresh?}`** on the **Python API** (`creative_api.py`) and **pk-api** (`writes/film.rs`, the write differential's film phase); the app's sessions sheet `/session/[bit]` (the sparkle), Keep / Skip per hunk, Save rant cut, Pick a hook with Use this one / Show others (once), the cut in the stepper and on the page, the cut picker in Film it. **Deviations:** (1) **the cut rides on the version's `source`** (`rant`, `rant_session`, `rant_hook` = the rant cut; anything else stage), not `line_meta.cut`: `line_meta` does not exist yet (3c Slice 3), and `source` is the one field both servers already write through the versions route and read back on the page, so neither server nor the differential changed for it (`cut_of` in `creative_api.py` and pk-api, `cutOf` in `rules/bitLines.ts`, all three tested). (2) **No sessions tables**: the two kinds run on the existing job queue (`/api/jobs/<id>` polled by the sheet), not `POST /api/sessions` / `session.mode` / `hook_rating`; nothing is stored about which hooks he saw or picked, and there are no thumbs (S7) yet. (3) **The worker kinds are `rant` / `hook` in both workers** (not pk-worker-only `session_rant` / `session_hook`): the live worker is the Python one until the T8 swap. (4) **The hunk shape is `{op: change · cut · add, line_no, was, now, why}`** (an add goes after `line_no`, 0 = before line 1); `was` must be his line word for word or the hunk is dropped; the reveal-word rule applies only when a punch index is sent (no punch marks exist until Slice 3, so the app sends none). (5) **The method block is the third system block** (after the profile and the learned block), as the punch-up's task line, not the first (§7): the profile stays the cached prefix. (6) **The sparkle opens the sheet** (3c's place for Ask Claude; HIG/pull-down-buttons: a primary action is not hidden under More); the sheet has only these two rows (no greyed rows for the rest). (7) **The cut picker is shown also with only the stage cut** (§2.2 amended above) and is an action sheet with a ✓ on the cut shown (a pop-up button would need a native menu in the custom bar). (8) **The save is `POST /api/jokes/<id>/versions` with `draft: true`** (the profile learns the kept text) and the hunks he explicitly skipped are sent as lines he passed over; Close or a swipe with a kept change asks "Leave without saving?". (9) **"paused" is not a separate word on the rows**: the worker line under "Asking Claude…" says offline / paused for today. (10) Same-day versions are now ordered by the decision that added them (the read's `history`), which they were not (ids are hashes). (11) Said vs written, `take` rows, `POST …/takes` are not built (they need pk-api deployed). **Deploy order for the phone:** the Python API image with this commit first (a worker claim naming `rant`/`hook` is a 400 on an older API), then the Python worker image; pk-api and pk-worker carry the same kinds for their own later cutover. |

---

## 12. Open questions for the owner (each with its context)

1. **Where the video lives.** Default (D8): Photos only, Punchable keeps the words and the asset id. If you want the
   paid backup later to carry the videos too, Punchable must keep its own copy under Documents (doubling the space a
   take uses on the phone) or read it back from Photos at backup time (fine as long as you have not deleted it there).
   Say which, or leave it for the backup's own spec.
2. **Mirroring.** iOS Camera saves selfies un-mirrored (text in the room reads right; your face looks "wrong" to you
   for a second). Default (D5): the same. If you would rather the file match what you see in the preview, it is one
   switch in Settings; say so.
3. **Captions always on.** Default (D9): burned on every Keep, one fixed look (white Inter 600, lower third, four
   words at a time). If you sometimes want a clean file (to caption elsewhere), a "Captions" switch on the review is
   one line; the words still land on the bit either way.
4. **The rant cut elsewhere.** Default (D10): the page, the Bits list, set lists and the stage all keep using the
   stage cut; the rant cut exists for the camera and the stepper. If a rant cut should ever be what a set list shows,
   that is a per-list choice to design.
5. **Opus for hooks.** Default: Opus for both sessions (voice fidelity; ≈ 5 cents a round). Sonnet would halve it
   with a plainer voice.
6. **The words he said, into the pile?** Default: no. A kept take's words sit on the On tape row and in Said vs
   written; "Pull the words into the pile" is a row action if you want what you actually said as a scrap (the same
   question 3c §13 leaves open for set clips).
7. **Filming a bit with no punch marked.** Default: allowed; the band shows no bold line. The alternative greys Film
   it with "no punch".
8. **A length light.** Default: no target bar on the camera (a clock only). If you want the Stage's "light" at 60 s
   (the clock turns amber), it is the existing `RecordClock` prop.
9. **Where Film it also starts from.** Default: the bit's More menu only. Tonight's ⋯ ("Film a bit…" → pick a bit)
   and the Record screen are possible second doors; not in v1 unless you ask.

---

## 13. HIG pages relied on (read live 2026-11-03)

| Page | Rule used here |
|---|---|
| HIG/modality (`https://developer.apple.com/design/human-interface-guidelines/modality`) | "A modal experience that fills a window or the device display minimizes distractions, so it can work well for presenting videos, photos, or camera views"; "Always give people an obvious way to dismiss a modal view"; "present an action sheet that includes a save option"; "Let people dismiss a modal view before presenting another one." → the full-screen route, Close, the Leave sheet, the cut picker's "Make a rant cut…" closing first. |
| HIG/going-full-screen | "Continue to provide access to essential features and controls so people can complete their task without exiting full-screen mode"; "Be sure to keep controls visible when they're essential for navigation or performing tasks"; "a game or a slideshow needs to pause automatically when people leave the experience". → the dock always visible, the take ending on background. |
| HIG/playing-video | "Use the system video player"; "Always display video content at its original aspect ratio"; "In general, restrict this content so that it doesn't obscure media playback." → `nativeControls`, `contain`, the caption overlay kept to the lower third. |
| HIG/privacy | "Request permission only when your app clearly needs access"; "Avoid requesting permission at launch"; the purpose-string sentence rules; "Include only one button" on a custom pre-permission screen (we have none). → §2.8. |
| HIG/accessibility | "Offer alternatives to gestures"; "Convey information with more than color alone"; "Replacing transitions … with fades"; "Support text-based ways to enjoy audio and video … Captions give people the textual equivalent"; "Let people control audio and video playback"; "Minimize use of time-boxed interface elements." → buttons for every gesture, the punch mark, the band's fade, the captions themselves, the player's controls, the snack's non-timed Undo path. |
| HIG/typography | viewing distance and weight; "avoid tight leading" for three lines; the 17 / 11 pt table. → `type.prompter`. |
| HIG/playing-haptics | "Ensure that haptic vibrations don't disrupt experiences involving device features like the camera, gyroscope, or microphone"; "Make haptics optional"; selection vs impact vs notification. → no haptics while recording; `selection` on the countdown, `medium` on start, `success` after the save. |
| HIG/feedback | "status feedback … near the items it describes"; "don't warn people when data loss is the expected result"; "Show people when a command can't be carried out and help them understand why." → the clock inside the preview, Redo without a sheet, the one-word reasons. |
| HIG/generative-ai | identify AI; multiple results to choose from; dismiss, revert, retry; voluntary feedback; server-side processing disclosed. → the badge, three hooks, Keep / Skip / Show others, the Privacy row. |
| HIG/camera-control | "people appreciate a large preview image with as few visual distractions as possible"; "Keep names of controls short." → the clutter rules on the camera; no Camera Control overlay is adopted (the hardware control is out of scope; the page is cited for its preview guidance only). |
| HIG/live-photos | "Never include a playback button that a viewer can interpret as a video playback button." → the On tape row's play glyph is a real play control on a real video; no still-with-badge confusion. |
| HIG/menus, HIG/toolbars, HIG/buttons, HIG/writing, HIG/undo-and-redo | as 3c §3.14: frequency order, the ellipsis, 44 pt, one prominent button per view (Keep; Next line while recording is the Stage's accepted second), plain verbs, Undo on every write. |
| Apple, PhotoKit "Delivering an Enhanced Privacy Experience in Your Photos App" (`https://developer.apple.com/documentation/photokit/delivering-an-enhanced-privacy-experience-in-your-photos-app`) | "You can't create or fetch user albums" under limited access; "Request authorization only for the level of access required." → §2.4, §2.8. |

---

## 14. Amendments this spec makes elsewhere (done by the builder in the same commit as the code that needs them)

- `docs/design/DESIGN.md` §4: a new type token **`type.prompter` 26 / 32, Inter 600, cap 1.6** ("a line read at arm's
  length from a camera; the punch 700 with the dot"); §5.3 `size.band` 120 (the prompter band) and `size.caption`
  (the burned caption's point size at 1080 wide, set by the Swift module, recorded here for the record).
- `docs/design/HIG.md` §3.9: a sibling line for Film it (`/film/{id}`, full-screen modal; nothing records until the
  disc; the square goes to review; Discard behind Close; no haptics while recording).
- `docs/design/SPEC-2026-11-03-bit-page-3c.md`: §3.6 the two session rows; §3.7 the Filmed rows and "filmed N"; §3.8
  "Film it…" in the More menu; §5 `line_meta.cut`.
- `docs/design/SPEC-2026-11-02-bit-sessions-and-joke-teaching.md` §2: the modes `rant` and `hook`.
- `docs/HANDOVER.md`: one open-list pointer to this spec and its three versions; the owner rules line gains "video
  stays in Photos on the phone; words go to the server".
