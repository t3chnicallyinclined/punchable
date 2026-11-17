# SPEC 2026-11-16: Bits, Sets, Tapes (the redesign) and the build slices

Status: **Slices 5 and 6 are built on one branch, 2026-11-17 (§8, each "Built"). Slices 1, 2 and 4 are merged (§8, each "Built"); Slice 2 was rewritten by an owner statement of 2026-11-16
(B11: the bit is a document); Slice 3 is built on a branch (§8 Slice 3 "Built"), rewritten by the owner's rulings of
2026-11-16 and 2026-11-17 (B12, a whole new draft); the rest is spec only.** Owner, 2026-11-16, on the working mockup: "ok lets get all this in, maps on
the roadmap to figure out". This file turns that mockup into an inventory, the data and API changes, the platform
verdicts, the risks and nine slices. The mockup is an HTML sketch kept outside this repo; every joke in it and in
this file is invented.

**The design in one paragraph.** Three tabs, **Bits · Sets · Tapes**, plus a Record control that is always there
and starts recording at once. A rant is a set you film, so the Rant tab goes. A set is bits with plain bullet
notes; one walk-through screen records sound or films video, with a Notes / Full lines switch and a tap per bit
that marks where the bit ends on the recording. The bit page puts connections on the line they belong to, with
**Around this bit** and **In these sets** under the lines. Marks (Change / Ask / Keep, on highlighted words or a
whole line) are sent in one round and return as a whole new draft, his questions answered (owner 2026-11-17; §3.2). The bit's page is the document: he
types straight into it and it saves by itself (B11).
"Add to a set" is a multi-tick picker. A Google Doc is shown as written, each blurb sorted into a bit or left
loose. A tape is its transcript shown the same way, cut up by bit. The joke map stays where it is and goes on the
roadmap (§9).

**How to read the tags.** CONFIRMED = read at that `file:line` on `main` (`03b83b4`; `33f8c1e` differs only in
`CLAUDE.md`) during the inventory of 2026-11-16, or quoted from the live HIG page that day. A dagger (†) means the
spec's author re-read that line after the inventory. INFERRED = reasoned from code that was read, not run.
UNKNOWN = not read or not provable without a device or a scratch run. **Nothing here was run on a phone**: no
device check from builds 22-26 has a recorded answer in `docs/HANDOVER.md`, so "shipped" never means "the owner
confirmed it".

Supersedes, where they differ: `SPEC-2026-11-03-bit-page-3c.md` §3.1-3.3 and §14 Slices 2-5 (the page's sections,
the line actions, focus mode), `SPEC-2026-11-06-rant-tab.md` R1, R3, R4 and §12 (the Rant tab), and
`docs/design/HIG.md` §3.9's "nothing tapes until the red disc" for the one new entry point in Slice 9. What those
specs decided and this one does not mention still stands.

---

## 1. Decisions (lead calls from the inventory; the owner may flip any)

| # | Decision | Why | Flip cost |
|---|---|---|---|
| B1 | **Lines get no ids. A mark, a redline and a line anchor each carry the line's own words** (plus the version id and an index as a hint). | A version is one text blob and lines exist only as a split on `\n` at read time (CONFIRMED † `rust/crates/pk-api/src/writes/bits.rs:73-82`). Quoted text survives a new version; an index does not. The worker already verifies `was` word for word (`pk-worker/src/kinds/rant.rs:253-314`). | Line ids on units: a schema change and a rewrite of every version write. |
| B2 | **The page keeps 3c's skeleton**: section rows, the Versions stepper, the Pile, On tape, the tab bar visible, one prominent bottom item. The mockup's groups become the content of those rows. | The mockup drops the stepper and hides the tab bar; both are decisions already taken (3c §3.5, `HIG.md` §10). The owner approved the mockup's ideas, not its HTML chrome. | Per item in §4.6. |
| B3 | **No filter chips on the bit page.** The five groups are headers under "Around this bit"; a line's pip shows its own. | The chips repeat the headers below them (3c §3.1: no filter above the fold). | One `Chip` row. |
| B4 | **Each Keep on a redline saves a version at once**, with Undo. | Nothing waits unsaved; 3c D12 (versions are cheap); matches `session/[bit].tsx`. | One version when he leaves the page. |
| B5 | **New server data goes in new creative stores** (`links`, `redline`, `setnotes`, `docsort`), not new tables and not new ops on old stores. | A store name is a plain string in `creative_event`, so no schema change (`db/schema_extra.surql:8-17`). The Python still replays `storyboards`, `workingsets`, `venues`, `jobs`, `account` (CONFIRMED † `api-dev/creative_api.py:264-266`), so a new op on those could break it. | Fold `setnotes` into `storyboards` after T11. |
| B6 | **A set is a board.** Working sets, "my sets" and the builder are not part of this design and their screens are already unreachable leftovers. | "Set lists" are boards (CONFIRMED `app/src/app/(tabs)/(sets)/sets.tsx:3`). | None. |
| B7 | **A topic becomes a set; a written rant stays a bit** and sits in that set. Nothing is deleted: the topic store stays as the record. | A rant is already a bit whose versions have a `rant*` source (CONFIRMED `pk-api/src/writes/film.rs:24-35`); a topic's kept material is the nearest thing to "the bits in a rant". Details §3.6. | A different mapping is a re-run of the migration tool. |
| B8 | **A Next tap is his statement**: the stretch is filed under that bit as his, not as an AI guess. He can still say "Not this bit". | He tapped it. | File as AI with the badge. |
| B9 | **Record lives in the tab bar's bottom accessory**, not in the tab bar itself (§4.1). | HIG: "Use a tab bar to support navigation, not to provide actions." | The trailing-slot button (against the HIG's letter, closest to the drawing). |
| B10 | **Order of slices changed from the lead's suggestion in two places** (§8): the editor (Slice 2, since B11 the document) ships before marks; Sets is three slices. | The editor is app-only and proves the keyboard questions the marks slice also needs; "Sets" as one slice was five features. | None. |
| B11 | **The bit is a document (owner, 2026-11-16, on build 28's Write-behind-a-button plan): "remember a bit is a document, it just has a bunch of ideas for the same topic, the bit is a topic basically it should be as easy as editing a google doc on your phone, with the power of punchable".** The bit's page IS the document: a tap anywhere in the text puts the caret there; there is no Write screen and no Save; one text field over the whole bit, so selection, paste, dictation and Backspace are the system's; a blank line separates two ideas; the words are kept on the phone as he types and a version is saved when he leaves or pauses. Around this bit and In these sets are drawn on the page under the document (owner on build 28: "when i open a bit i dont automatically see other connections") and are out of the way while he types. | The owner's statement. **Supersedes** §4.3 as first written (per-line inputs, the Join up key), Slice 2 as first written (`joke/[id]/write.tsx`, `lib/LineEditor.tsx`, drag grips inside the editor), the §4.6 rows "Around this bit and In these sets as open cards → collapsed section rows" and "Write in place …", and B2's "one prominent bottom item" for this page (the document needs no button). | The older Write button and one-box editor are kept behind a preference (Slice 2 "Built", call 12). |
| B12 | **Punchable is Google-Doc-easy editing of the bit plus the Redline loop on it (owner, 2026-11-16, after B11): "maybe like punchable is the redline portion its a mix", and, on what he means by it: "when i say redline, i mean our repo redline, our app we integrating into [another of his projects, named in the original] as well that idea".** Redline is the owner's own review app (its README, "What it does" and "What the agent receives"): he selects any text and attaches a typed comment (Edit, Question, Keep or Fork); the comments collect in a tray; Send review batches them into one turn, in document order, each anchored to the text it quotes. That is the loop Slice 3's marks already come from (Change · Ask · Keep, sent once, §3.2); Fork maps to taking a line somewhere else (a set, a new bit). **It does not mean Punchable draws suggestions he did not ask for inside his text**, the way a shared Google Doc shows other people's suggestions: a first reading of the sentence said that, and it was wrong. | The owner's two statements. Nothing in the slices changes: Slice 2 is the easy editing, Slice 3 is the loop. What Slice 2 adds for Slice 3 is the anchor: a place in the document is found by the line's own words (`rules/lineRules.ts` `anchorsOf`, `findAnchor`; B1), so a mark and the tracked change that answers it survive a new version. | None. |

---

## 2. Inventory: what exists, what is new

App paths are under `app/src/`; Rust paths under `rust/crates/`. "pk-api owns" = the route is answered in Rust
today (`pk-api/src/lib.rs:55-200`, router `:286-555`).

### 2.1 Tabs and shell

| Mockup | Exists | New |
|---|---|---|
| Tabs Bits · Sets · Tapes | Rant · Bits · Sets as `NativeTabs`, `minimizeBehavior="onScrollDown"`, one stack per tab, badge on Bits (CONFIRMED † `lib/TabsShell.native.tsx:16-31`). Web shell: `lib/Screen.tsx:33-80`. | A `(tapes)` group; the `(rant)` group removed (Slice 6). |
| Record button, always there | No control outside a screen's content. `NativeTabs.BottomAccessory` is exposed by the installed expo-router (CONFIRMED `expo-router/build/native-tabs/NativeTabs.d.ts:26-28`) and unused; `lib/accessory.native.ts:8-14` already gates on Liquid Glass. `/record?kind=memo` starts on open (`app/record.tsx:95-99`). | The accessory with Record (Slice 9). |
| Settings, You, Voice profile, What it has learned | Live inside the Rant stack (`(tabs)/(rant)/_layout.tsx:16-28`): `settings`, `profile`, `voice`, `learned`. The Rant root also mounts `useTonight` so the Bits badge is right at launch (`(rant)/index.tsx:70`) and flushes pending script drafts (`:44-65`). | A root-level sheet stack behind the avatar on all three tab roots; the two start-up jobs move to the root layout (Slice 6). |
| Every route still opens | `rules/routeTable.ts:7-14` lists 52 required URLs checked by `tests/routes.test.mjs`. | Each removed screen leaves a redirect or comes off the list, in the same commit. |

### 2.2 The bit page

`(tabs)/(bits)/joke/[id].tsx` is the 3c page (Slice 1 of that spec is on `main`).

| Mockup | Exists | New |
|---|---|---|
| The lines, idea line, Write at the bottom | Yes: `IdeaLine`, `BitLines`, `Stack.Toolbar placement="bottom"` with Write (`joke/[id].tsx:318-328, 360-368`). Reads `GET /api/jokes/<id>`, `/similar?limit=5`, `/api/merge/candidates?joke=` (`:55-57`). | Nothing. |
| Tap a line: marks and that line's connections open under it | Tap selects; the actions are **Move** and **Cut** only (`joke/[id].tsx:257-260`); a second tap opens the editor at that line (`lib/BitLines.tsx:40`). | The marks row; the pip; the inline connection minis. |
| Highlight words inside a line | **Nothing on the page is selectable** (`selectable` appears only on crash text and the old builder). No selection or context-menu library is installed. | §4.2. |
| Connections on a line | **None, and the server has no line anchor.** Edges end on a joke or a whole version (`via_unit`), never a line (CONFIRMED `pk-kb/src/read.rs:273-283`). Only `tags` items carry a quoted `line`, and it is the other bit's (`pk-creative/src/connections.rs:231-295`). | A computed anchor (§3.1). |
| Around this bit, grouped | **The page shows no connections.** What was "Elsewhere" moved to `joke/[id]/tape.tsx:72-111` (nights, filmed takes, "sounds like it, on tape · link it", "sounds like these bits · merge it in"). Grouped data exists server-side and only the leftover `bit/[id].tsx:77` calls it: `GET /api/jokes/<id>/connections` with groups `next`, `tags`, `same_world`, `callbacks`, `same_shape`, `contrasts` (CONFIRMED † `lib/creative.ts:20`; `pk-creative/src/connections.rs:24-31`). | One read that joins five sources (§3.1). |
| Yes, connected / Not connected | An edge is confirmed or rejected with `POST /api/decision {target:"edge", id, action}` (`pk-kb/src/build.rs:182-255`). "Not the same bit" is `POST /api/merge/dismiss`. **There is no way to say "not connected" about a suggestion that is computed and has no edge.** | The `links` store (§3.1). |
| Same bit, merge / Pull this line in / Use this wording | `POST /api/merge`, `/merge/attach`, `/merge/undo` (`pk-api/src/writes/merge.rs:233-576`); a version is `POST /api/jokes/<id>/versions`. A note attaches to **one** bit only (409 on a second; CONFIRMED † `merge.rs:293-296`). | Client wiring only. |
| In these sets | **No field or call returns the sets a bit is in** (`pk-api/src/pages.rs:1189-1195` builds none). | In the new read (§3.1). |
| Add to a set, multi-tick | The picker is single-choice and closes on tap (CONFIRMED † `lib/boards.tsx:112-117`), fed by `GET /api/boards`, writing `POST /api/boards/<id>/items {joke_id, after?}`; 409 = already there. One bit can sit on many boards, once per board (`pk-creative/src/reducers/story.rs:104-109`). | Ticks, toggle off, staged Done (§4.5). No batch route needed. |
| Marks, send once, redlines | The nearest thing is the session sheet: `session/[bit].tsx:239-274` shows "Was / Now" hunks with Keep / Skip, saving one version with `draft: true` and sending skipped lines to `POST /api/profile/voice/learn`. Jobs: `POST /api/jokes/<id>/rant` and `/hook`. Per-line asks exist only for the Rant script (`rant-line.tsx`, `POST /api/rant/line`). **No stored marks, no multi-mark round, no Ask answer on a line, no Keep that protects a bit's line.** | §3.2. |
| Versions, Pile, On tape rows | Yes (`joke/[id].tsx:375-389`). The mockup omits them; they stay (B2). | Nothing. |

Unused today and removable once this lands: `lib/BitTalk`, `lib/MergeHint` (imported by no screen), `(bits)/bit/[id].tsx`.

### 2.3 Write

| Mockup | Exists | New |
|---|---|---|
| One field per line; Return splits; Backspace at the start joins; drag to reorder | The bit page edits **one multiline input over the whole version** (`joke/[id].tsx:333-336`) and saves with `POST /api/jokes/<id>/versions {text, reason, source?}` (`lib/bitWork.ts:15-18`). A per-line editor exists in the Rant script: only the edited line is an input, Return splits at the caret, a pasted or dictated newline splits, **Backspace joins only an empty line**, reorder is Move up / Move down in a sheet (`(rant)/topic/[topic]/script.tsx:181-189, 240-267, 330-343`; rules † `rules/scriptRules.ts:56-63`). `spike-write.tsx:59-109` keeps every line mounted. | The bit's Write as a line editor built from those two; join at the start of a non-empty line; drag. |
| Drag ≡ | `lib/DragList(.native).tsx`: props `{items, keyOf, render(t, i, handle), onReorder(from, to)}`; native is a long-press pan on the grip with Reanimated and VoiceOver Move up / Move down (`DragList.native.tsx:48-57, 91-108`). Used for the bit's Reorder mode, set lists and three leftovers. **Never combined with focused inputs; no auto-scroll while dragging.** | Rows that hold an input (UNKNOWN how it behaves: Slice 2 proves it). |
| Nothing lost | A local draft pattern exists for the script: saved 400 ms after each edit to the phone, one version on leaving (`lib/scriptStore.ts:27-38`, `lib/scriptDraft.native.ts:8`, `script.tsx:118-133`). **The bit page has no outbox**: Write is greyed offline (`joke/[id].tsx:8, 104`). 3c's Slice 2 outbox was never built. | The same local draft for a bit. |

`(bits)/write.tsx` is an older "Write" home with no way in; it is not the bit editor.

### 2.4 Recording and performing

| Screen | What it does today | Sends |
|---|---|---|
| `stage/[id]` | The set's short names with the current one lit, clock, dock Pause / disc / Next; a row opens the notes sheet (`bitnotes.tsx`, fed by `GET /api/boards/<id>/build`: setup, reveal or the typed idea, all server-derived). **Next writes no timestamp**: it moves the highlight and nothing else (CONFIRMED † `stage/[id].tsx:69-73`). No Back. | After on-device transcription, `POST /api/sets/transcript` with `{segments, title, recorded_at, duration_s, engine}` and nothing else: **no marks, no bit boundaries, not even the board's id** (CONFIRMED † `rules/memoRules.ts:222-226`). |
| `record` (untitled set) | Clock, level, a **Mark** button kept on the local row (`record.tsx:115`, `lib/recordings.native.ts:280`). | Same call; the marks never leave the phone. |
| `record?kind=memo` | Starts as it opens. | `POST /api/ideas`, then attach. |
| `film/[id]` (one bit) | `PromptBand` (one bit's lines, auto-advancing; `lib/Prompter.tsx:27-28`), front camera, 3-2-1, dock Hold / disc / Next line; review Redo / Keep; the video goes to Photos. Ten-minute cap (CONFIRMED † `rules/filmRules.ts:34`); locking the phone ends the take (`film/[id].tsx:284-292`). | Words as an idea filed on the bit. The video never uploads. |
| `take/[rec]` | Plays a kept take until its file leaves the app's folder. | Nothing. |

Two recorders, no shared screen or state machine: sound is `expo-audio` (`lib/recorder.native.ts:18, 44`; keeps
running when the phone locks, pauses on a call), video is `expo-camera` (`film/[id].tsx:155, 367`). Shared parts
are the controls only (`lib/RecordControl.tsx`). The Prompter knows lines, not bits or notes.

**New:** one walk-through for sound and video; Back; Notes / Full lines; a split per bit; the After screen; following
a set chosen after recording started; turning the camera on mid-take (§4.4).

### 2.5 Sets

| Mockup | Exists | New |
|---|---|---|
| The list of sets | `(sets)/sets.tsx` (Set lists · Recorded) and `list/[id]` are live. `boards`, `builder`, `myset`, `workingset`, `setlist`, `review.tsx` and `(rant)/today.tsx` are leftovers reachable only through each other. | Cards that show the bits' names and the last recording. |
| Bullet notes per bit | **None.** A board is `{id, title, joke_ids, target_s, notes (one string for the whole board), source, seed, archived}`; an item is a bare joke id (CONFIRMED † `pk-creative/src/reducers/story.rs:44-51`). The app's `BoardPatch` has no per-item field (CONFIRMED † `lib/creative.ts:63-66`). | The `setnotes` store (§3.3). |
| Add a bit, remove, drag, length | All exist with Undo (`list/[id].tsx:132-147`, `lib/AddJokeSheet.tsx`, `target_s`). | Nothing. |
| Seams between two bits | Each build step has `transition_in {words, weak, kind: proven|link|jump}`; `list/[id].tsx:271-278` draws only the weak ones ("Gap here · add a bridge"). Bridges: `POST /api/boards/<id>/bridge`, kept as a `segue` edge (`lib/bridge.tsx:30`). | Draw every seam. "Same shape twice" has no field: a new check in the build read (INFERRED small: both bits' `same_shape` edges are already in the graph). |
| Recordings of this set | **Nowhere.** The server is never told which board a tape followed; only the local row knows (`rules/memoRules.ts:25`). | `board` on the transcript (§3.4). |
| Record / Film on a set | Record opens `stage/[id]`. Film exists per bit only. | The walk-through. |

### 2.6 Rant

A **topic** is a record in the `topics` creative store: `{id, text, created, touched, ranted_at, rant, rants[],
from_trend, by, deleted_at}` plus, per piece of material (`joke:` or `perf:` ref), `{state: kept|dropped, at, jot}`
(`pk-creative/src/reducers/topics.rs:117-204`). A **script**'s provenance marks live in the `script` store, one
entry per version id, aligned by line index (`reducers/script.rs:20-83`). A **rant** is a joke node whose versions
carry `source` `rant`, `rant_session`, `rant_hook`, `rant_draft` or `rant_write`. A **local draft** sits only on the
phone (`Documents/script/<topic>.json`). "Finished" is decided on the phone from its own filmed takes. Routes, all
Rust only: `GET/POST /api/topics`, `GET/PATCH /api/topics/<id>`, `GET /api/topics/<id>/material`,
`POST /api/rant/draft`, `POST /api/rant/line`, `GET /api/rants`, `POST /api/jokes/<id>/rant`, `/hook`.

**New:** the mapping into sets (§3.6). A topic's kept material has **no order** today.

### 2.7 Tapes

| Mockup | Exists | New |
|---|---|---|
| One list of everything recorded | Three places: Sets › Recorded (local rows via `lib/RecRows` + `GET /api/sets`), Bits › Memos, Rant › Filmed. | One list. |
| The transcript in order, cut by bit | **Not shown anywhere.** `set/[id].tsx` shows analysis rows (time, matched bit or "New material", rating, script diff, punchlines); a non-bit row shows its first seven words (`:257-262`). The full transcript **is** stored in order with times: `corpus/sets/<dir>/transcript.json`, read back as `[{start_ms, end_ms, text}]` (`pk-api/src/tape.rs:74-96`); per-word times are not kept. | A read that cuts it by bit (§3.5). |
| Which bit a stretch is | `performance` nodes (`set, start_ms, end_ms, text, kind`) and `performed_as` edges, AI ones written `inferred` from analysis rows that carry a `joke_id` (`pk-kb/src/overlay.rs:451-598`). His corrections: `POST /api/set-row {perf, op: rate|match|new|crowd|bit|banter|confirm}`, `POST /api/review/reject-match`. | Stretches from his own Next taps (§3.4). |
| Not in any bit → new material | Partly: rows with `status: new`; "Save" in New tonight (`POST /api/ideas` with `from {set, perf, start_ms, end_ms}`). No general "transcript minus matched spans". | In the read. |
| Laugh after each stretch | `laugh` rows exist for sets processed from audio. **A phone-transcribed set has none**: `set_text` writes every audio field as null (`pk-worker/src/kinds/set_text.rs:274-285`), and the audio never leaves the phone (standing decision). | Not in these slices (§5 R4). |
| Play | Server audio does not play in the phone app (`lib/media.ts:58-66` throws unless the source is a link). Local files play whole, with no seek call in use (`lib/localPlay.native.ts:35-62`). | Seek to a stretch in the local file. |

**Two probable defects found by reading, not run (INFERRED). Slice 7 starts by proving or killing them:**

1. **A set transcribed on the phone may never become a set.** `set_text` writes its analysis under the key `rows`
   (CONFIRMED † `set_text.rs:311`; the Python it was ported from does the same, † `worker/execute.py:179`). The graph
   sync only takes in an `analysis.json` that has a `jokes` array (CONFIRMED † `pk-api/src/writes/ingest.rs:425`),
   and the `set` node and its performances are created inside that loop (`pk-kb/src/overlay.rs:407-451`). If so, the
   night is stored as three files and N new ideas but has no row under `GET /api/sets`. `HANDOVER.md` item 4 says
   "the set appears"; whether it does in production is UNKNOWN.
2. **A bit made from a phone set loses which stretch it came from.** `set_text` posts `from: {set, unit}`; the ideas
   route keeps only `set, perf, start_ms, end_ms, note` (`pk-api/src/writes/ideas.rs:91-99`).

### 2.8 Google Docs

| Mockup | Exists | New |
|---|---|---|
| The doc as written, blurb by blurb | **No screen, no list of docs, no route.** | All of it. |
| A doc is a pile of blurbs | **One new doc becomes one bit**: the whole export goes into `joke.summary`, the doc's name is the title (CONFIRMED `pk-gsync/src/sync.rs:652-685`; the Python `gsync/` is the one running, same behaviour). The splitter cuts **only at heading lines** over the non-blank lines; a doc with no headings is one unit (CONFIRMED † `pk-gsync/src/units.rs:79-135`). | A blank-line blurb splitter. |
| `locator` | Exactly `{para_start, para_end, para_rule: "non-blank lines of fileContent, 0-based, inclusive"}` or `{source: "title"}`. No character offsets, no heading path, and **blank lines are already gone**, so "as written" cannot be rebuilt from it. These units go only to the file `kb/gsync/units.json`; nothing reads them into the graph (INFERRED: no reader found). What `locator` holds on the older corpus units already in the hosted database is UNKNOWN (his data was not read). | The doc's raw text kept on the server. |
| Sorted into bits | A new idea is auto-attached when the match is clear (`pk-api/src/writes/ideas.rs:190-257`), but only for docs of ≤ 4000 characters; a longer doc is a raw node add that **skips the auto-attach and the embed follow-up** and has no size cap on `summary` (`pk-gsync/src/config.rs:24`, `pk-kb/src/build.rs:338-343`). The Haiku judge (`memolink`) is queued for memos and uploads, **not for docs** (`pk-api/src/writes/ingest.rs:526-542`). | A sort job for docs. |
| An edited doc | Identity is the doc id → one joke id; an edit replaces the summary; unit ids are doc id + position, so they shift when a heading is inserted (`sync.rs:706-847`). | Blurb identity that survives edits. |
| Not a joke | `GET /api/suggestions/not-a-joke`, `/keep`, `/hide`: built for voice memos. | Per blurb. |

No test with a long multi-blurb doc exists (the one found is a comment, `rust/tools/gen_gsync_tapes.py:651`).

### 2.9 The map

`(bits)/map/index.tsx` (Cloud and Links), reached from the Bits toolbar's Map button (`bits.tsx:165, 173`).
`map/[id].tsx` has no way in. **Unchanged by this spec** (§9).

---

## 3. Data and API deltas

All new routes are pk-api, Rust only, and follow the rant routes' rule: no Python twin, no differential, unit tests
plus `pk-api/tests/writes.rs` on a scratch database, each with a red arm. Every new route is added to the `OWN_*`
arrays, to `infra/k8s/prepared/t10-api-rs.yaml` (`tests/routes.rs:225-258` compares the two) and to the method
table at `tests/routes.rs:262-274`. A new store is added to `STORES` and `for_store`
(`pk-creative/src/reducers/mod.rs:31-64`) and the `Stores` struct (`pk-api/src/creative.rs:299-321`).

**The deploy-order trap still applies to job kinds.** The Python's job reducer raises on a kind it does not know
(CONFIRMED † `api-dev/creative_api.py:113-114`). Until T11 retires it, every new kind goes into `JOB_KINDS` there
first and that image rolls out first. New *stores* do not have this problem (INFERRED: the Python replays only the
five in its `APPLY` map).

### 3.1 The bit's surroundings: one read, one small store

`GET /api/jokes/<id>/around?cap=6` (Slice 1)

```json
{ "joke": "joke:…", "unit": "unit:…",
  "groups": [ { "key": "same_idea", "title": "Same idea", "items": [
      { "id": "…", "kind": "bit", "ref": "joke:…", "title": "Smart doorbell", "words": ["first line", "second"],
        "meta": "new, never on stage", "why": "Same idea, written in March", "source": "ai",
        "edge": null, "line": { "i": 2, "text": "the line of THIS bit it belongs to" }, "does": ["merge", "dismiss"] } ] } ],
  "sets": [ { "board": "sb:…", "title": "Friday late show", "position": 2, "count": 6, "notes": ["…"] } ] }
```

| Group (key) | Built from (all exist) | Actions (`does`) |
|---|---|---|
| Same idea (`same_idea`) | the merge judge's group for this bit (`pk-creative/src/merge.rs:1388-1434`, "same" ≥ 0.80) and `/similar`'s `like.bits` | merge → `POST /api/merge`; dismiss → `POST /api/merge/dismiss` |
| Callbacks (`callbacks`) | the `callbacks` and `tags` groups of `connections.rs` (`callback_to`, `tag_of` edges, with direction) | connect → edge `confirm`; dismiss → edge `reject` |
| On stage (`on_stage`) | the `next` group ("done right after it N×"), `/similar`'s unlinked `performed` moments, and the newest night's wording where it differs from the written line (the set page's `script_diff`) | link → `decide` edge add `performed_as`; use → a new version; dismiss |
| Loose scraps (`loose`) | notes attached in the `merges` store, and unattached ideas the judge puts near this bit | add → version; attach → `/merge/attach`; dismiss → `/merge/dismiss` |
| Same shape (`same_shape`) | the `same_shape` group | connect / dismiss as Callbacks |

`same_world` and `contrasts` are computed today and are not in the mockup: left out, one line to add back.

**`source`** is `his` or `ai`, from the edge state the page already computes (`pk-api/src/pages.rs:258-296`); a
computed suggestion with no edge is `ai`.

**`line`** (the anchor, Slice 1 sends it, Slice 3 draws the pip): the line of the newest version that shares the most
content words with the item's words, and the cosine of the two when the model is loaded, above a threshold that the
slice sets from a private eval; below it the item is bit-level and has no `line`. This is a guess by construction
(INFERRED quality, §5 R6) and never moves his words: it only decides where a card opens.

**`sets`**: a scan of the `storyboards` store for boards whose `joke_ids` hold the bit, archived ones left out, with
`setnotes` joined in (empty until Slice 4).

**"Not connected", remembered**: new store **`links`**, keyed by the sorted pair of refs; ops `dismiss {a, b, group}`
and `undismiss`. `POST /api/links/dismiss {a, b, group, reason?}` → `{ok, id}`; `POST /api/links/undismiss {id}`.
`/around` drops any computed item whose pair is dismissed. Stored edges keep using `POST /api/decision`.

**"Link something…"** is the existing search plus `POST /api/decision` edge add (a `reason` is required there; the
app sends the group's name).

### 3.2 Marks and the redline round (Slice 3; rewritten 2026-11-17 to what was built)

**What changed from the first §3.2.** The owner ruled on 2026-11-17 that a round returns **"a whole new draft"**, not
per-line tracked changes to keep or skip, and that marking up the draft and sending again is wanted (**"yes
possibly"**). So there are no `changes`/`tags`/`kept` hunks, no Keep/Skip per hunk and no version per Keep (B4 is
superseded for this loop: one version when he uses a draft). The loop is the owner's Redline (B12).

A **mark** (on the phone) is `{id, type: "change"|"ask"|"keep", text, at, note}`: `text` is his words exactly as they
stand (the selected words with their outer spaces left out, or, with nothing selected, the whole line the caret is
on), `at` where they started (a UTF-16 index, the phone's own count) as a hint only, `note` his note on a Change
(optional) or his question on an Ask (required). A mark is found again by its words (B1): at its hint when the words
still stand there, else the occurrence nearest the hint, else it is gone (shown in the tray, never sent).

| Piece | Shape (as built) |
|---|---|
| Unsent marks | On the phone only: `Documents/marks/<bit>.json` for his bit, `draft~<bit>.json` for the draft he is reading (the web: `localStorage`), one shared copy in memory (`lib/marks`). Marks live on the newest **stage** document only (not the rant cut, not an older version, not the older editor). At most 20. A Keep and a Change on overlapping words are refused on the phone and by the route. |
| Send | `POST /api/jokes/<id>/redline {unit, marks: [{type, text, at?, note?}]}` (round one: `unit` = the version he marked, which must be the newest of its cut) or `{on: <job>, marks}` (round two: the round whose draft he marked). The text marked is read on the server, never sent by the phone: the version from the KB, the draft from that job's result. 1 to 20 marks, each quote 1 to 4,000 characters and in the text, each note at most 500, an Ask needs its question, at least one Change or Ask. In round two the Keeps of the round he marked ride along (`carried`, at most 40 marks in all). Legal rules on the text, his bit, every note and the title, each alone and read together (422). Always a new job (`fresh`); the same marks twice are one round. → `202 {status: "queued", job, poll, round}`. **409** when the bit has a newer version of that cut, a quote is not in the text, the draft is not the open round's, or the bit changed since the draft was made: nothing is written, the marks stay on the phone. |
| Job kind `redline` | `pk-worker/src/kinds/redline.rs`, task `creative_redline` (Opus 5.5, effort high, `max_tokens` 32,000). Input `{joke, title, unit, on, n, base, bit, marks: [{type, text, start, at, note, carried}]}`; `bit` (his bit, for context) only in round two and only when it is at most 12,000 characters. Prompt: his voice profile, the learned block, the method, then the marks in Redline's own shape ("Review of his bit "…". 3 items, in document order. Please apply the CHANGEs, answer the QUESTIONs before revising, leave the KEEPs exactly as they are. Return the full revised bit …", each item `[n] CHANGE · idea 2` with its quote and note) and the bit between `<bit>` markers (`<draft>` in round two). A bit over 12,000 characters (`pk_creative::redline::WHOLE_CAP`) is still sent whole, but only the ideas a Change or an Ask is in, with one idea either side, are rewritten (`parts`) and put back by the server: every word outside them is his, byte for byte. |
| Result | `{status: ok|refused|empty|provider_error, draft, answers: [{item, text, question, quote}], notes: [{item, was, why}], marks, mode: whole|parts, tries, dropped, refused?, tokens, model, …}`. |
| Checks (each with a red arm) | A draft is **refused** when a Keep's words are not in it byte for byte, or an idea he did not mark (no Change or Ask in it) is gone: fewer than **50 %** of its content words (four letters or more; ideas with fewer than two are not judged) are anywhere in the draft (`KEPT_SHARE`, the lead's number, not measured on his material), or it is empty or over 40,000 characters. A refused draft is asked for once more with the reasons, then the result is `refused` with no draft: never shown. A change's `was` is kept only when it is his words exactly (the verbatim gate; in round two only his bit counts, never the model's own draft); a quotation of two words or more in an answer that is not his words exactly loses its quotation marks. |
| Rounds | New store **`redline`**, keyed by joke: `send {job, unit, on, n, marks}` (replaces the open round) and `resolve {job, verdict: used|discarded|open, unit?}` (`open` = Undo of a close). The reducer checks shape only; a resolve that names no round is replayed as nothing. The draft is the job's row: a waiting draft survives any restart. |
| Read | `GET /api/jokes/<id>/redline` → `{joke, newest, round: {job, status, unit, on, n, sent_at, marks, stale, newest, result, error} | null, last}`; `stale` = he saved a newer version of that cut since sending. `GET /api/redlines` → `{open: [{joke, title, short_name, job, status, state: waiting|ready|refused|failed, stale, sent_at}]}`. |
| Use / Discard | Use this draft = `POST /api/jokes/<id>/versions {text: draft, source: "redline", draft: true}` (the learning loop's kept text) then `resolve used` with that unit; Undo = undo the version and `resolve open`. A stale draft asks first ("Your newer words stay in Versions"). Discard = `resolve discarded`, Undo = `resolve open`; it teaches the voice profile nothing. |

### 3.3 Set bullet notes (Slice 4)

New store **`setnotes`**, keyed `<board>|<joke>`; one op `set {notes: [≤ 12 lines, ≤ 200 characters each]}`, newest
wins, Undo = the previous list.

- `POST /api/boards/<id>/notes {joke_id, notes}` → `{ok, notes, was}`.
- `GET /api/boards/<id>/build`: each step gains `notes: [...]` and `notes_from: "his" | "derived"`. With none of
  his, the step shows what the stage notes sheet shows today (setup and reveal, or the idea;
  `rules/setListRules.ts:60-72`) as `derived`; his first note replaces them.
- `POST /api/boards/<id>/items` gains an optional `note` (the highlighted words from "Add to a set").
- Removing a bit from a set leaves its notes in the store; adding it back brings them back.
- `GET /api/jokes/<id>/around` `sets[].notes` reads the same store.

### 3.4 Per-bit splits on a recording (Slice 5 sends and keeps them; Slice 7 uses them)

The walk-through records a **split** each time he leaves a bit: `{joke_id, start_ms, end_ms}`, from the recorder's
own position (`recorder.currentTime`, which already leaves out a call's pause), not the wall clock. Going Back and
forward again adds a second span for the same bit.

- Local row (`lib/recordings.native.ts`) gains `splits`, `board`, `mode: "sound" | "video"`.
- `POST /api/sets/transcript` gains optional `board` (a board id that must exist) and `splits` (≤ 500, ordered,
  inside `duration_s`, each `joke_id` on that board). Today it drops every key it does not know
  (CONFIRMED † `pk-api/src/writes/transcript.rs:89-99`). Both pass through the `set_text` input (`pk-wire`
  `SetTextInput`) into the set's `meta.json`. No new job kind.
- Slice 7 turns each split into a `performance` with a `performed_as` edge to that bit, marked his (B8), and runs
  the model split only over what no split covers.

For video there is no recorder clock: the start time is taken before the camera has begun writing
(`film/[id].tsx:147-155`), so video splits sit early by the camera's start-up time (size UNKNOWN; a device check).

### 3.5 A transcript cut by bit (Slice 7)

`GET /api/sets/<id>/transcript` →

```json
{ "set": {"id": "set:…", "title": "…", "date": "…", "duration_s": 552, "board": "sb:…", "mode": "sound"},
  "stretches": [ { "i": 0, "start_ms": 0, "end_ms": 95000, "text": "…", "perf": "perf:…",
                   "joke": "joke:…", "title": "Roommate", "state": "his" | "ai" | null,
                   "kind": "bit" | "crowd_work" | "banter" | "new", "laughs": null } ],
  "bits": [ {"joke": "joke:…", "title": "Roommate", "stretches": 1} ] }
```

Built from `transcript.json`'s segments and the set's performances; a run of segments no performance covers is a
stretch with `joke: null`. `laughs` is `{count, longest_ms}` from `laugh` rows where they exist (the rule in
`pk-api/src/tape.rs:208-237`) and `null` for phone tapes. Writes are the existing ones: Not this bit →
`POST /api/review/reject-match`; Add to a bit → `POST /api/set-row {op: "match"}`; Crowd work, skip →
`{op: "crowd"}`; New bit from this → `POST /api/ideas` with `from`; Use this wording in the bit → a new version
with `source: "tape"`.

What Slice 7 must add under it: phone sets entering the graph (§2.7 defect 1), performances from splits, a match
step for stretches he did not tap (§5 R2), and an end to `set_text` posting a new idea for a stretch that was
matched to a bit he already has.

Memos and filmed takes are tapes too. Their audio and video stay on the phone; a memo's transcript is its one
stretch. The Tapes list joins the phone's index with `GET /api/sets`, and a phone row learns its server set id from
the job's result so the page can play the local file at a stretch.

### 3.6 Rants and topics become sets, nothing lost (Slice 6)

A Rust tool (`cargo xtask rants-to-sets`, dry run by default, the write needs the owner's go and a backup first)
adds events; it changes and deletes nothing. It is safe to run twice (a board that already carries the topic in
`seed` is skipped).

| Today | Becomes | Kept how |
|---|---|---|
| Topic `{id, text}` | A board titled with the topic's text, `seed: {topic: <id>}`, `source: his` | The topic record stays in its store, readable at `GET /api/topics/<id>`. |
| The topic's rants (`rants[]`, oldest first) | Items of that board, first | They are bits already; every version and its `source` is untouched. |
| Kept bits (`joke:` refs, `state: kept`), including jots | Items after the rants, in the order he kept them (`at`) | The kept mark stays on the topic. |
| Kept taped moments (`perf:` refs) | Not items (a moment is not a bit): listed on the set page under "Also kept" until he makes one a bit | Read through the topic by `seed.topic`. |
| Dropped marks, `found` | Stay on the topic (they stop a dropped piece being suggested again) | Untouched. |
| Script provenance (`script` store) | Stays keyed by version id; the bit's line editor reads it for the AI badge on a line | Untouched. |
| A rant with no topic (`GET /api/rants` minus the above) | A one-bit board named with the bit's short name | The bit is untouched. |
| A deleted topic (tombstone) | No board | The tombstone stays, so its Undo still works. |
| A local draft on the phone | Flushed as a version at launch by the code that does it today, moved to the root layout, **before** the Rant screens are removed | The file is deleted only after the save answers. |
| Filmed takes, "Finished" | Phone-only already; they show under the set's recordings and in Tapes | Untouched. |

Writing tools: **Make it a rant** and **Pick a hook** stay on the bit page's sparkle. Per-line asks (`rant_line`) are
replaced by marks. **Draft a rant from a topic** (`rant_draft`) has no place in the mockup: it stays in the set's
More menu as "Draft it as one piece…" until the owner says otherwise (§7 Q4). The topic routes stay for one
release as the read behind "Also kept", then go with their own note.

### 3.7 Docs (Slice 8)

1. **Keep the doc's text.** `pk-gsync` posts each doc's export as a file (`kb/gsync/docs/<docid>.md`, through the
   worker file route it already uses for its state), so "as written" has a source. The legal rules run on it as on
   every ingest path.
2. **Blurbs.** A blurb is a block between blank lines (a heading starts one). Its id is a hash of its normalised
   words, so an edit elsewhere in the doc does not move it and a re-sync keeps every correction.
3. **Job kind `doc_sort`** (Haiku judge, as `memolink`): for each blurb the nearest bits by vector, then the judge:
   a bit, loose, or not a joke. Output `kb/docs/<docid>.json`. Python-first rollout of the kind.
4. **His corrections** in new store **`docsort`**, keyed `<doc>|<blurb>`: `assign {joke}`, `loose`, `new_bit`,
   `not_joke`; they always win over the job.
5. **Routes.** `GET /api/docs` → `[{id, title, modified, blurbs, bits, loose, first}]`;
   `GET /api/docs/<id>` → `{doc, blurbs: [{id, i, text, joke, title, state: his|ai, n, of, why}]}`;
   `POST /api/docs/<id>/blurbs/<blurb> {assign | loose | new_bit | not_joke}`.
6. **Versions.** A blurb assigned to a bit becomes a version of it (`source: "doc"`, the doc's date) unless a
   version with the same words exists. Which one the bit page shows as newest is §7 Q5.
7. **Today's one-doc-one-bit rows** stay; the doc's old container bit is hidden only when every blurb has a home, as
   an undoable decision.

**Slice 8 starts with a measurement, not a screen** (§5 R1).

---

## 4. Platform checks

Checked by the iOS design expert (live HIG pages, read 2026-11-16) and the iOS dev expert (the installed modules:
expo 58.0.23, react-native 0.88.1, expo-router 58.0.30, react-native-screens 4.30.1, expo-audio 58.0.11,
expo-camera 58.0.14; new architecture on). There is no Mac: every native claim below is from source, and the
device checks ride in Slice 1's build so the answers arrive before the slice that needs them.

| # | Question | Verdict | Build it as | Fallback | Device check (Slice 1's "Device checks" screen) |
|---|---|---|---|---|---|
| 4.1 | Record button in or beside the tab bar | **Against the HIG as drawn; works beside it** | The tab bar's bottom accessory | (a) trailing-slot button; (b) a toolbar button on each tab root | The accessory shows on all three roots and on pushed screens, survives the minimised bar, can be hidden on the bit page |
| 4.2 | Actions on selected words | **Works with changes** | Each line a read-only input; our own action row for the selection | Whole-line marks plus a note | Long-press selects words; no system menu; a tap still opens the line |
| 4.3 | The bit as one document he types straight into (B11; was: per-line inputs, Return splits, Backspace joins) | **Works; the scrolling is unproven on a phone** | One multiline input in the page's scroll view; our own caret follow | The older Write button and one-box editor, behind a preference | The Slice 2 build's checks (§8): tap deep in a long bit, type at the bottom, drag the keyboard down |
| 4.4 | Recording while the walk-through shows | **Works with changes** | One screen, one recorder at a time | Sound keeps running, camera films muted | The camera mounted over a running sound take, muted and unmuted |
| 4.5 | Add to a set as a multi-tick sheet | **Follows the HIG with changes** | Form sheet, checkmarks, Cancel / Done | Immediate writes, one Close | None |
| 4.6 | The bit page as drawn | **Follows with changes** | 3c's skeleton, the mockup's content | — | 3c check 1 (still unanswered) |

### 4.1 The Record button

- **The lead's recollection is right (CONFIRMED).** "Use a tab bar to support navigation, not to provide actions…
  If you need to provide controls that act on elements in the current view, use a toolbar instead." (HIG, Tab
  bars). The repo already wrote the same rule down (`lib/TabsShell.native.tsx:17`, the Rant spec's R4).
- **The trailing separated slot is the search slot** ("A tab bar can include a dedicated search tab at the trailing
  end"). In expo-router it is `role="search"` (a `UISearchTab`).
- **Build:** `NativeTabs.BottomAccessory` holding one red Record control. One tap presents the full-screen recorder
  already recording; while taping, the accessory shows the clock and Stop. The HIG's only sentence about the
  accessory is Music's mini-player ("you can choose to minimize the tab bar and move the accessory inline"); it
  sets no rule on what it holds (INFERRED reading). It stays on pushed screens, because each tab owns its stack,
  and is covered by full-screen modals (the walk-through, Write), which the HIG allows.
- **Risks (UNKNOWN until the device):** three bottom layers on the bit page (Write, the accessory, the tab bar);
  whether the accessory can be hidden per screen (developer-forum threads report visibility problems on iOS 26.0
  and 26.1).
- **Fallback (a), the closest to the drawing:** a fourth trigger with `role="search"` and `disabled`. A disabled
  trigger blocks the native selection and still emits `tabPress` (CONFIRMED
  `expo-router/build/native-tabs/types.d.ts:13-25, 547-562`); there is no `preventDefault`
  (`canPreventDefault: false`, `:24`). Whether a red icon renders there, whether it looks dimmed and whether the
  search animation starts first are UNKNOWN. It goes against the HIG sentence above: the owner's call (§7 Q1).
- **Fallback (b):** `record.circle` in the top bar of each tab root. To the letter of the HIG, not under the thumb.
- **Later:** the `prominent` tab style is exactly the drawing and is iOS 27+ (`types.d.ts:622-629`). The Action
  Button, a Control Center control and a Home-screen quick action are HIG-endorsed and each needs a new native
  target or an alpha module: roadmap.
- **This flips two standing notes** for this one control: "no input bar in the tab bar" (2026-10-28) and "the tab
  bar navigates; recording is a tab's content" (R4). The owner's approval of the mockup is taken as that flip.

### 4.2 Actions on selected words

- **`<Text selectable>` cannot do it**: on iOS it offers Copy for the whole text, with no range and no event
  (CONFIRMED `react-native/React/Fabric/Mounting/ComponentViews/Text/RCTParagraphComponentView.mm:335-395`).
- **React Native has no custom edit-menu items**; the input has only `contextMenuHidden`
  (CONFIRMED `…/TextInput/TextInput.flow.d.ts:460`).
- **Build:** each read line is a multiline input with `editable={false}`; `onSelectionChange` reports the range
  (it comes from the text view's selection delegate, `RCTTextInputComponentView.mm:514-534`; INFERRED that it
  fires when read-only) and our own row (Change · Ask · Keep · Add to set… · Make bit) opens under that line for
  exactly those words. Whether to hide the system menu is a design fork: the HIG says "let people select and copy
  noneditable text", so the default is to **leave the system menu alone** and show our row under the line.
- **HIG on the real thing (CONFIRMED):** custom edit-menu commands are sanctioned ("List custom commands near
  relevant system-provided ones… Avoid overwhelming people with too many custom commands"). That needs a small
  Swift text view; self-sizing one under the new architecture is the hard part, and with no Mac each compile error
  is one cloud build. Roadmap, not a slice.
- **Libraries (read 2026-11-16):** none fits. The maintained ones either give offsets but no menu, or a menu but no
  offsets; the older ones were last published in 2022.
- **Fallback, and what Slice 3 ships if the device check fails:** whole-line marks with a note field ("which
  words"). It is also the most standard: tap a row, inline actions. Word chips ("pick words") were rated weakest:
  a custom interaction for a standard task.

### 4.3 The bit as a document (rewritten 2026-11-16 for B11)

**What this replaces.** As first written this section specified a line editor: one multiline input per line, all
mounted, Return splitting at the caret, Backspace at the start of a line joining (UNKNOWN on iOS, so a "Join up" key
in a bar above the keyboard), grips to drag a line, Cancel and Done in the bar of a full-screen Write. The owner's
statement (B11) retires all of it: per-line inputs cannot select across lines, paste or dictate several lines the way
a document does, and the unknown about Backspace only exists because of them. None of it was built.

- **One multiline input over the whole text**, uncontrolled (the text view owns the text; React never writes it
  back while he types, so the caret, autocorrect and dictation are never disturbed), not scrolling by itself
  (`scrollEnabled={false}`: it grows with the text inside the page's scroll view). Return, Backspace at the start of
  a line, selection across lines, paste and dictation are the text view's own. CONFIRMED by reading
  (`RCTTextInputComponentView.mm`): a `defaultValue` is never re-sent to the native text. INFERRED: a tap on the
  unfocused field focuses it and places the caret at the tapped point in one tap (stock `UITextView`).
- **Structure is read out of the text, never typed into it** (`rules/lineRules.ts`): a newline is a line; a blank
  line separates two ideas; a run of blank lines is one gap. A reorder, a line put in from a card or the pile and a
  swapped wording all keep the blank lines where they were.
- **Saving.** A local draft a moment after each keystroke (the phone's Documents; nothing typed is lost offline or
  if the app dies). A version (`POST /api/jokes/<id>/versions`) when he leaves the text (Done, which puts the
  keyboard away), leaves the page, sends the app to the background, pauses (30 seconds without a keystroke, at most
  once every three minutes), or gets his connection back; only when the words changed (trailing spaces and blank
  ends are not words), never an empty one, one request at a time. `inactive` (Control Center, a system alert) does
  not save. Offline the draft waits and is sent when the connection returns, from the page or at the next launch.
- **The caret and the keyboard.** The shell's `KeyboardAvoidingView` shortens the scroll view. React Native's own
  "scroll the focused input into view" runs only with `automaticallyAdjustKeyboardInsets` and only when the keyboard
  frame changes (CONFIRMED `RCTScrollViewComponentView.mm` `_keyboardWillChangeFrame`), and falls back to the whole
  field's frame when the selection has no rectangle (CONFIRMED `RCTTextInputComponentView.mm`
  `reactUpdateResponderOffsetForScrollView`; INFERRED that a bare caret has none, which would scroll a long bit to
  its end), so the page does not use it. Instead a hidden copy of the text up to the caret (to the end of the caret's
  word, since a word wraps whole), laid out at the field's width and type, gives the caret's height, and
  `caretScroll` moves the page only when the caret is under the keyboard or above the bar. UIKit may also scroll the
  enclosing scroll view for a non-scrolling text view; nothing in React Native blocks that (CONFIRMED by the iOS dev
  review), and whether the two disagree is UNKNOWN until a phone.
- **HIG.** While the keyboard is up the bar's trailing side holds Done alone, prominent ("Use the .prominent style
  for key actions such as Done"); nothing above the document changes when the keyboard comes up, so a tap lands
  where he aimed it; everything under the document is unmounted. A save is not announced while he is on the page
  ("people typically expect their action or task to succeed, they only need to know when it doesn't"): the footnote
  line reads "Saved as v9" for a moment and the Versions row, directly under the document, carries the non-timed
  Undo. A save made as he leaves the page is a snack with Undo, because that row is no longer in front of him. The
  footnote line has the screen's ground, so it is never drawn over his words, and the caret is kept clear of it.
- **Reorder** is its own mode from More (the existing drag list): the rows are the document's lines and one "Blank
  line" row per gap between ideas, which can be dragged too.
- **Limit.** A version takes **40,000 characters** (raised from 4,000 on 2026-11-16, owner: "a bit is a document,
  it just has a bunch of ideas for the same topic"): `pk_kb::MAX_UNIT_CHARS` / `kb.MAX_UNIT_CHARS` (a unit),
  `pk-api/src/writes/bits.rs` and `api-dev/server.py` (the route, both servers, the same words), `rules/lineRules.ts`
  `MAX_DOC` (the app). A longer document stays on the phone with "Too long to save · 40,120 of 40,000" in the
  footnote. An idea jotted and the input bar keep 4,000. What a 40,000-character version meets (read in code
  2026-11-16, not measured):
  - **The write:** at most 160 KB of text in a 256 KiB body; one decision line and one unit hold it whole.
  - **Search by meaning and "sounds like it":** the vector is made from the first 2,000 characters only
    (`pk-embed` `MAX_CHARS`, then 512 tokens), so the lower part of a long document is found by its words (the
    full-text index covers all of it) and not by its meaning. Cut, never refused.
  - **The worker:** Make it a rant, Pick a hook and punch-up are handed the version's lines whole (INFERRED: no cap
    on the way in was found); a full document is about ten times the tokens of an old one, inside the daily budget.
    Draft a rant from a topic reads the first 4,000 characters of each source.
  - **Lists and search rows** show a cut line, and the decisions list is paged; backups dump whole tables.
  - **What grows:** every save is a whole new version. `GET /api/jokes/<id>` sends every version's text, and each
    one is a row in the decision log and a node in pk-api's memory, which already runs close to its limit under
    load (journal 2026-11-16). Twenty-five saves of a full document are 1 MB of text for one bit. Not a problem at
    one write; the thing to watch as documents get long. Follow-ups, none done: versions sent without their text
    until asked for, and the memory limit.
  - **On the phone:** one multiline field holds the whole text and the local draft is rewritten after each pause in
    typing; how that feels at several thousand words is a device check.
- **Fallback:** the older page (lines as 44-pt rows, the bottom Write button, the one-box editor with Save), behind
  Settings › This device › "Write button on a bit".

### 4.4 Recording during the walk-through

- **One screen can hold both recorders, one at a time.** Re-rendering the notes or tapping Next does not touch the
  file: the take is written natively, and `film/[id].tsx:178` already advances its band while recording (INFERRED
  sound; no device answer).
- **A running sound take cannot be handed to the camera.** "Camera on" after Record has started is one of: stop the
  sound file and start a video with sound (two files, one take record: the default), or mount the camera muted
  while the sound continues (one sound file and a silent video). Whether mounting the camera disturbs a running
  sound take is UNKNOWN: its capture session reconfigures the shared audio session (`lib/film.native.ts:19-24`).
- **Sound keeps recording when the phone locks and pauses on a call; video ends on both** and is capped at ten
  minutes. A filmed set longer than that needs the cap raised and a device check for heat and file size (Film it's
  D6, unanswered).
- **Film it's eight device checks (D1-D8) are all unanswered.** D3 decides whether a video's words can be
  transcribed on the phone at all (`lib/transcribe.native.ts:27-30`).
- **HIG changes to the drawing:** keep today's dock (Back 60 pt · stop square 92 pt · Next 60 pt). **Next never
  turns into Finish under the same thumb** (a double tap on the last bit would end the take); it greys as "Last
  bit", as it does today (`stage/[id].tsx:164`). End saves without asking; Close asks (today's behaviour). Notes /
  Full lines is a tap on the notes block or a top-bar toggle. **No haptic on Next while a microphone is live**
  (HIG, Playing haptics; `stage/[id].tsx:70` does it today and Film it forbids it: one rule for both).
- **"Starts at once"**: the HIG has no view. Only the Record control does it; a set's Record and Film still open at
  ready with the disc, and Film keeps its 3-2-1. Make it safe with the existing "Starting" state, one haptic at the
  real start, the 92-pt square to stop and save, Discard behind Close. The first use shows the microphone prompt
  once.

### 4.5 Add to a set

`presentation: 'formSheet'`, detents `[0.5, 1]`, grabber; rows with a trailing checkmark and
`accessibilityRole="checkbox"`; Cancel leading, Done trailing; ticks are staged and written on Done with one snack
("Added to 2 sets · Undo"); "New set…" is an inline field row, not a second sheet ("Display only one sheet at a
time"). The bit page's bar button (3c D1) opens it.

### 4.6 Where the mockup and earlier decisions differ, and which wins

| Mockup | Already decided | Keep |
|---|---|---|
| The tab bar replaced by foot buttons on pushed screens | Tabs stay on pushes (`HIG.md` §10) | Earlier decision |
| Six filter chips | No filter above the fold (3c §3.1) | Earlier decision (B3) |
| Tap a line → Change / Ask / Keep + its connections | Punch · Tighten · Tag after; second tap opens Write (3c §3.2) | **Mockup**, on 3c's mechanics. Punch, Tighten and Tag after become Change's chips; "Edit" joins the row to replace the second tap. |
| Redlines inline with Keep / Skip | Tracked changes in the session sheet | **Neither, since 2026-11-17:** a whole new draft on its own screen, Use this draft or Discard (owner; §3.2, Slice 3 "Built"). |
| "Around this bit" and "In these sets" as open cards | Collapsed section rows | **Mockup, since B11** (Slice 1 shipped the collapsed rows; the owner on build 28: "when i open a bit i dont automatically see other connections"): drawn on the page under the document, two cards a group, "See all N" |
| No Pile, Versions, On tape rows | 3c §3.4-3.7 | Earlier decision: Loose scraps opens the Pile's content, On stage links to On tape, the stepper stays |
| "Send 3 marks" swapping places with Write | The bottom holds Write alone | Write always trailing; Send leading |
| Write in place with a bottom Done, grips and ✕ | Focus mode, Done in the bar | **Neither, since B11:** the page is the document; Done in the bar only puts the keyboard away (§4.3) |
| Snack with Undo only | Snack plus a non-timed Undo (`HIG.md` §7, §10) | Earlier decision |
| "Yes, connected" | No Yes / No labels (`HIG.md` §9) | "Connect" / "Not connected" |
| Search drawn as a pill in the content | The system search field (`HIG.md` §3.2) | Earlier decision |
| Five edge colours for bits in a doc or tape | "Avoid relying solely on color" (HIG, Color) | **Mockup**, because every block also carries the bit's name; use the `cluster1…6` tokens (`lib/theme.ts:50, 117`) |

Tabs: Bits `square.grid.2x2.fill`, Sets `list.bullet.rectangle.portrait.fill` (both in use), Tapes `waveform`
(new to `lib/Icon.native.tsx`).

---

## 5. Open risks, stated plainly

| # | Risk | What is known | What the slice does about it |
|---|---|---|---|
| R1 | **Today's doc splitting does not handle long messy docs at all.** | It cuts only at headings; a doc without them is one unit; a doc over 4000 characters lands as one giant idea with no matching (§2.8). The 3c spec counted 38 docs that hold several jokes each. No test uses a long doc. How well a blank-line splitter plus a judge sorts his real docs is **UNKNOWN**. | Slice 8 begins by running the new splitter and judge over his docs **privately** and reporting counts (blurbs per doc, share sorted, share he would call wrong on a sample). The screen is built only if that is good enough; the numbers go in the private eval folder, never here. |
| R2 | **Matching a recording to bits when he never taps Next.** | The model split exists (`set_text`, Opus) and cuts a transcript into units, but **nothing matches those units to his bits for a phone set** (the `match_judge` task is in the config; what calls it is UNKNOWN), and each funny unit becomes a new idea today, which duplicates bits he already has. Quality of a vector-plus-judge match on spoken, paraphrased bits is **UNKNOWN**. | Slice 7 builds the match with a private eval on tapes he has already corrected, shows every guessed stretch with the AI badge and "Not this bit", and falls back to "Not in any bit" when unsure. Taps (Slice 5) are the reliable path and ship first. |
| R3 | **Phone-taped sets may not be reaching the Sets list today** (§2.7 defect 1). | Read in code on both the Rust and the Python side; not run. | First task of Slice 7: one scratch-database run settles it; the fix is that slice's first commit. Worth checking sooner if he has taped a set on build 20 or later and cannot find it. |
| R4 | **No laughs on phone tapes.** | The audio stays on the phone and nothing there finds laughs; hosted audio processing does not exist. The mockup's "laugh 5s" after each stretch and "4 laughs" on the After screen cannot be shown for anything he records in the app. | The transcript and After screens show times and words only, and laughs where older sets have them. §7 Q3. |
| R5 | **Keyboard and selection behaviours are unproven on a phone** (§4.2; §4.3: since B11 the per-line questions are gone and the open one is whether the caret stays above the keyboard in a long bit), and three earlier device checks were never answered (3c checks 1 and 3; build 26's C, D, E; Film it D1-D8). | Each has a fallback that needs no native code. | Slice 1's build carries one Device checks screen for the four new questions; Slices 2, 3, 5 and 9 each read its answer before they start. |
| R6 | **Which line a connection belongs to is a guess.** | No edge points at a line; the anchor is word overlap and a cosine. A wrong anchor puts a card under the wrong line, never changes his words. | Bit-level first (Slice 1), pips second (Slice 3), a threshold from a private eval, and no pip below it. |
| R7 | **Marks on a version that has moved on.** | Lines have no ids (B1). | The round is refused with 409 when the bit has a newer version; a redline is applied by its quoted `was`, so one that no longer matches a line is shown as "this line changed" and cannot be kept. |
| R8 | **A new job kind can break the Python while it still runs.** | Its reducer raises on an unknown kind. | `redline` and `doc_sort` ship Python image first, as `rant_draft` and `rant_line` did; or after T11, when the rule goes. |
| R9 | **The Rant tools lose their front door.** | Trending was specified and not built; "Draft a rant from a topic" is built and has no place in the mockup. | §3.6 keeps the draft in the set's More menu; §7 Q4. |
| R10 | **The redline round's cost per send.** | Measured on synthetic bits (Slice 3 "Built", the cost table): about $0.06 to $0.12 at 4,000 characters, $0.08 to $0.15 at 40,000 (a long bit is rewritten around its marks only); thinking tokens and his voice profile are on top. | The daily budget caps spend; a refused draft is retried once at most. Not measured on his material: the private eval pairs were not run. |
| R11 | **Possibly broken today, seen in passing (INFERRED):** the conversation's input bar may not be drawn on iOS 26 (`lib/SayBar.tsx:55-77` returns nothing when the accessory flag is on, and nothing puts the bar in the tab bar); several stores use `localStorage`, which may not persist on the phone. | Not part of the redesign. | One line each on Slice 1's Device checks screen. |

---

## 6. What each screen reads and writes (summary)

| Screen | Reads | Writes |
|---|---|---|
| Bits (home) | `GET /api/jokes` (rows gain `first` = the bit's first line, Slice 8; whether a row carries any of his words today is UNKNOWN), `GET /api/tonight/screen`, `GET /api/redlines` (S3), `GET /api/docs` (S8) | as today |
| Bit page | `GET /api/jokes/<id>`, **`/around`** (S1), **`/redline`** (S3) | `/versions`, `/api/decision`, `/api/merge*`, **`/api/links/dismiss`**, **`/redline`**, `/api/boards/<id>/items` |
| Write | the page's data; a local draft | `/versions` |
| Add to a set | `GET /api/boards`, the page's `sets` | `/api/boards/<id>/items` (add), `PATCH /api/boards/<id> {remove}`, `POST /api/boards` |
| Sets, a set | `GET /api/boards`, `/api/boards/<id>/build` (+ `notes`) | **`/api/boards/<id>/notes`**, existing board writes, `/bridge` |
| Walk-through, After | `/api/boards/<id>/build` | local row with `splits`; `POST /api/sets/transcript` (+ `board`, `splits`) |
| Tapes, a tape | the phone's index, `GET /api/sets`, **`/api/sets/<id>/transcript`** (S7) | `/api/set-row`, `/api/review/reject-match`, `/api/ideas`, `/versions` |
| A doc | **`GET /api/docs/<id>`** (S8) | **`POST /api/docs/<id>/blurbs/<blurb>`** |
| Record | none | the local row; the memo or set routes of today |

---

## 7. Open questions for the owner (each with the lead's recommendation)

1. **Where the Record button sits.** Apple's guideline says the tab bar is for moving between sections, not for
   buttons that do something. The closest we can get while following it is a Record strip that sits just above the
   tab bar on every screen (like the mini-player in Music). The alternative is a round red button at the right end
   of the tab bar as drawn, which bends the guideline and may look dimmed or animate oddly; we can only see that on
   your phone. **Recommendation: the strip above the tab bar; the next build includes a test page so you can see
   both and choose.**
2. **Highlighting words.** The iPhone does not let our kind of app add buttons to the Copy / Look Up bubble without
   custom Apple code that we can only test by sending you builds. The plan is: you press and hold to highlight
   words as usual, and Change / Ask / Keep appear under the line instead of in the bubble. If that does not work
   on your phone, marks apply to a whole line and you type which words you mean. **Recommendation: accept that
   plan; put "buttons inside the bubble" on the roadmap.**
3. **Laughs on what you record in the app.** The app cannot count laughs on a recording made on the phone, because
   the sound never leaves the phone and nothing on the phone listens for laughs. So the new Tapes screen can show
   what you said per bit and how long it ran, but not "laugh 5s". Getting laughs means either sending the sound to
   the server (changes your rule that audio stays on the phone) or building laugh detection on the phone (a
   sizeable native piece). **Recommendation: ship Tapes without laughs now; decide between the two once Tapes has
   been in use for a while.**
4. **"Draft a rant from a topic" and Trending.** The new design has no Rant tab, and the mockup shows no place for
   the tool that drafts a whole rant from a topic and the bits you tick, or for the Trending list that was planned.
   **Recommendation: keep the draft tool inside a set's More menu so nothing you have is lost, leave Trending on
   the roadmap, and tell us if you want either more visible.**
5. **Which version a bit shows after a doc is sorted.** When four blurbs in a doc are the same joke, they become
   four versions of one bit. The page shows the newest, but in a doc "newest" can only mean "lowest on the page",
   which is not always your best wording. **Recommendation: the bit keeps showing whatever it showed before; doc
   blurbs are added as older versions you can step to and restore.**
6. **Old rants.** Each topic becomes a set containing the rants written for it and the bits you kept for it. A rant
   that has no topic becomes its own one-bit set, so it still has Record and Film. **Recommendation: yes; they can
   be archived in one swipe if that is clutter.**
7. **About a dozen old screens nobody can reach** (the old Notebook, the builder, working sets, "After a show", the review
   stepper and others). **Recommendation: remove them during this work, leaving redirects so old links open the
   nearest new screen.**

---

## 8. Slices (smallest first; each its own TestFlight build, usable alone)

Every slice: `cd app && npx tsc --noEmit && npm run test:unit && npx expo-doctor`; a rules file in `src/rules/`
with a test and a `--red` arm for every new rule; Rust slices add `cd rust && PK_REQUIRE_EXTERNAL=1 cargo xtask
gate`, a scratch-database test in `pk-api/tests/writes.rs` with a red arm, the route lists in §3; the guard before
each commit; the iOS design and dev experts review each diff. Server before app: each build's routes are live
before the build goes out. No time estimates.

**The order against the lead's suggestion.** (1) The editor moves ahead of marks: it is app-only and it proves the
keyboard behaviour that the marks slice's inline note field and the set notes both lean on (since B11 it is the
document, not a line editor); marks need a new worker kind, a Python-first rollout and an eval. (2) "Sets with bullet notes,
the walk-through and the rant migration" is three slices (4, 5, 6): notes are useful on today's stage screen the
day they ship; the walk-through must exist with Film before rants can move into sets, or rants would lose filming;
the tab swap rides with the migration so the bar never shows four tabs or an empty one. (3) The multi-tick picker
moves into Slice 1: it needs the same new read as "In these sets" and nothing else. (4) The Record button stays
last, but its device check ships in Slice 1.

### Slice 1: Around this bit, In these sets, Add to a set

**After it he can:** open a bit and see everything tied to it in five groups with his own words on each card, act
on each (merge, pull a line in, link, connect, not connected), see which sets use the bit, and tick it into several
sets at once.

- **Server:** `GET /api/jokes/<id>/around`; store `links`; `POST /api/links/dismiss`, `/undismiss`
  (`pk-api/src/routes/`, `writes/`, `pk-creative/src/reducers/links.rs`, the join in `pk-creative/src/connections.rs`).
- **App:** `joke/[id].tsx` (two section rows under the lines), new `lib/Around.tsx`, `lib/SetPicker.tsx` (replaces
  the picker in `lib/boards.tsx`), `rules/aroundRules.ts`; `joke/[id]/tape.tsx` keeps nights and takes and loses
  the two "sounds like" lists (they are groups now); Settings › About › Device checks gains the four checks of §4
  and the two of R11; remove `(bits)/bit/[id].tsx` with a redirect.
- **Tests:** `tests/aroundRules.test.mjs` (grouping, the action per kind, the picker's staged diff → the calls);
  `tests/routes.test.mjs`; Rust: `/around` over a scratch graph (each group from its source, a dismissed pair
  gone, sets membership, an archived board left out), red arms for the dismissal and the membership.
- **Owner's phone checks:** a bit shows "Around this bit · N" and "In these sets · N"; each card shows a line of his
  and AI or Yours; "Not connected" removes it and it stays gone after a reload, Undo brings it back; "Same bit,
  merge" keeps every word; To a set shows ticks on the sets it is already in, tick two more, Done → "Added to 2
  sets · Undo"; untick removes; the four device checks, one line each on which way they went. (As built, the merge
  button reads "Merge into this bit": call 13 below.)
- **Built 2026-11-16 (branch `app/redesign-slice1`; not merged, not pushed, not deployed, no phone build).** Server:
  `pk-creative/src/reducers/links.rs`, `pk-creative/src/around.rs`, `pk-api/src/routes/around.rs`,
  `pk-api/src/writes/links.rs`; one regex Ingress path and two exact ones in `infra/k8s/prepared/t10-api-rs.yaml`.
  App: `rules/aroundRules.ts`, `lib/Around.tsx`, `lib/SetPicker.tsx`, the routes `/around` and `/setpick` (form
  sheets), `lib/deviceChecks.ts`, three check pages. The calls made against this spec:
  1. **"Not connected" on a card with no stored edge always goes to `links`, the Same idea cards too** (§3.1's table
     said `POST /api/merge/dismiss`). That route has no undo and the check above asks for one. `/around` also leaves
     out a pair he dismissed on the merge screen. **Not done:** `GET /api/merge/candidates` and the same-idea banner
     do not read `links`, so a pair the judge calls the same bit and he dismissed here is still counted there
     (the fix: the merge state gains the `links` pairs and the groups' cache key the store's length).
  2. **A card carries two fields more than §3.1's example:** `connect {kind, src, dst}` (the edge Connect or Link
     adds when the card has none) and `attach` (the record of a note filed under the bit; "Not this bit" is
     `POST /api/merge/undo`). `kind` is `bit | line | moment | wording | note | idea`; `does` is drawn from
     `merge | add | use | connect | link | attach | dismiss`.
  3. **`line` (the anchor) is word overlap only:** two shared content words and half of the smaller side. The cosine
     leg and the threshold from a private eval are **not done** and move to Slice 3, which draws the pip: one
     embedding per line and per card on every read was not worth it for a field nothing shows yet.
  4. **`sets` leaves out the app's drafts** as well as archived and deleted lists (the picker lists the same ones).
  5. **"Link something…" (§3.1) is not built:** it is not in this slice's list and needs its own search-and-pick
     sheet. A tie he makes by hand is still Merge with… or the map.
  6. **The join is its own module** (`around.rs`), reading `connections::joke_connections`, not a part of
     `connections.rs` (a golden port of the Python).
  7. **The half sheet and the picker are routes**, so the phone draws them as native form sheets (medium detent,
     grabber); their screens are `lib/Around.tsx` and `lib/SetPicker.tsx` as listed. **To a set in the bit's bar now
     opens the picker**; the one-tap add to the current list stays on the other surfaces.
  8. **"Use this wording" replaces the line the card belongs to only while that line still says the same** (B1);
     otherwise the wording is added at the end and nothing is replaced. The sheet says which before he taps.
  9. **Every write from a card has two Undos:** the snack, and a non-timed one on the row (Versions for a new
     version, Around this bit for the rest).
  10. **Nine device checks**, not six: the three of 3c (unanswered) stay on the screen. The switches for where Record
      sits are held in memory only; the trailing-slot one needs a route file, `(tabs)/recslot`.
  11. **No differential:** the route has no Python twin. Proof is hand-written cases in `around.rs` and
      `links.rs`, and `pk-api/tests/writes.rs` on a scratch database, with two red arms run once each (the join
      ignoring `links`; `sets` counting an archived list): unit and scratch test both failed, exit 101, then passed.
  12. **On tape no longer opens for "sounds like it" alone:** those two lists are groups of Around this bit.
  13. **From the two reviews (iOS design, iOS dev; both "ship with fixes", every must-fix landed):** the buttons
      read **Merge into this bit**, **Add this line**, **Use this wording**, **Connect**, **This tape is this bit**,
      **File under this bit**, **Not connected** (a filed note: **Not this bit**), so the mockup's "Same bit, merge"
      and "Pull this line in" are not on the phone; Merge is never the filled button (it asks once more in an action
      sheet); the picker does not swipe away while a tick or a name is staged, and Cancel then asks Discard changes
      or Keep editing; a sheet that was swiped away while its request ran pops nothing; the two rows are drawn
      while the read is on its way; on an empty bit the Pile leads.

### Slice 2: The bit is a document (rewritten 2026-11-16 for B11; was "Write as a line editor")

**After it he can:** open a bit and type straight into it, the way Notes or a Google Doc works on a phone: tap
anywhere and the caret is there; Return makes a line, a blank line starts another idea, Backspace joins two lines;
nothing typed is lost if the app closes or the phone is offline; and what is tied to the bit is on the page under
his words, without a tap.

**Superseded (not built):** `joke/[id]/write.tsx` as a full-screen modal with Cancel · name · Done,
`lib/LineEditor.tsx`, `lib/DragList` with input rows, the keyboard bar's Join up key, "an unchanged Done saves
nothing" as a button's rule (it is the autosave's rule now).

- **Server:** none.
- **App:** `joke/[id].tsx` (the page is the document), new `lib/BitDoc.tsx` (the one input, the autosave, the caret
  follow), `rules/lineRules.ts` (what a save is; lines, ideas and rows read out of the text; an edit that keeps the
  blank lines; the local draft and which text the page opens on; where the page scrolls for the caret),
  `lib/bitDraft.ts` on `lib/draftFiles(.native).ts` (`lib/scriptDraft*` generalised), `lib/Around.tsx`
  (`AroundInline`, `SetsInline`, the sheet's group list), `rules/aroundRules.ts` (`inlineGroups`, `inlineAction`),
  Reorder as a mode, the older editor behind `prefs.bitEditor`; remove `spike-write.tsx` and `(bits)/write.tsx`.
- **Tests:** `tests/lineRules.test.mjs` with `--red` (five arms: a save only when the words changed; blank lines
  kept through rows; an edit that keeps the gaps; the page opens on his unsaved words; the page does not scroll
  while the caret is in view); `tests/aroundRules.test.mjs --red=inline` (which cards the page draws).
- **Owner's phone checks:**
  1. Open a bit: his words, then Versions, then Around this bit with up to two cards a group and In these sets by
     name, with no tap.
  2. Tap a word in the middle of a long bit: the caret is on that word and the keyboard comes up; the page does not
     jump. Tap after the last word of a long bit: the same.
  3. Type at the bottom of a long bit, ten new lines: the line being typed stays above the keyboard and the page
     does not jiggle.
  4. Return mid-line splits it; Backspace at the start of a line joins it to the one above; select across three
     lines and cut; paste them elsewhere; dictate two sentences with "new line" between them.
  5. Tap Done: the keyboard goes, the line above the bottom says "Saved as v9" for a moment, and the Versions row
     under the text reads "v9 · just now" with Undo; Undo puts the old words back. Tap Done without changing
     anything: no new version.
  6. Type and swipe Back mid-word: Bits shows "Saved as v…" with Undo, and the bit has the words when reopened.
  7. Type, close the app completely within a second, reopen the bit: the words are there, and they become a version.
  8. Airplane mode: type; the line says "Offline · kept on this iPhone"; turn the connection back on: a version
     appears without a tap. Both the line he is typing and that offline line are readable.
  9. Drag down on the text with the keyboard up: does the keyboard follow the finger, and is there a blank band
     above it?
  10. More › Reorder lines: drag a line across a "Blank line" row; Done → "Reordered" with Undo, and the blank line
      is still there.
  11. On a card under the text, tap its button (Connect, Add this line…): it happens, with Undo; "Add this line"
      keeps the blank lines of the bit. "See all 5" opens that group; a card there opens in the same sheet with Back.
  12. Settings › This device › "Write button on a bit" on: the older page with Write and Save is back.
- **Built 2026-11-16 (branch `app/redesign-slice2`; not merged, not pushed, no phone build; app only, no server
  change).** Nothing was run on a phone. The calls made against this spec:
  1. **One multiline input, not one per line** (B11). Nothing of the first Slice 2's editor was built, so nothing
     was switched.
  2. **A version on leaving the text, the page or the app, on a pause and on reconnect; never on `inactive`.** The
     pause is 30 seconds without a keystroke and **at most one pause-save every three minutes**
     (`lineRules PAUSE_MS`, `PAUSE_GAP_MS`): both reviewers found a version per pause would fill the Versions list
     with near-copies. The owner may tune both numbers.
  3. **Saving is quiet on the page** (design review, must-fix): no snack and no haptic on Done; "Saved as v9" in the
     footnote line for four seconds and the non-timed Undo on the Versions row. Only a save made while leaving the
     page is a snack. This differs from the first Slice 2's check "Done → Saved as v9 and Undo" in where it shows.
  4. **The Versions row sits directly under the document** (above Around this bit) and is shown from the first save
     on, because its Undo is the save's Undo.
  5. **Around this bit on the page:** each group that holds something, in the server's order, **its first two
     cards**, "See all N" when there are more; nothing at all when nothing is tied to the bit; skeleton rows while
     the read is on its way. The design review suggested two cards for the first group and one for the rest, and no
     header until the read lands; the brief's two cards and skeleton rows were kept, for the owner's phone to judge
     (check 1).
  6. **A card carries one button on the page**, its first action that is not "Not connected": bordered, never
     filled, in the link colour. "Merge into this bit" still asks once. A card whose button would write a line and
     that has several lines to choose from opens the sheet instead (the picker is there). The card's body opens the
     Slice 1 sheet with every action. **"See all N" opens that same sheet route on a group** (`/around?group=`): the
     group's cards, then the tapped card in the same sheet with a Back chevron (one sheet at a time).
  7. **A button that writes a line edits the document** (`lineRules applyEdit`; a `Plan` of kind `version` carries
     `edit`), so the blank lines between ideas survive. Slice 1 joined the lines with words and lost them.
  8. **Cards, the sparkle and the page's own writes are off while typed words are not a version yet** (typed and
     unsaved, or a save on its way), on the page and in the sheet ("Saving what you typed first"), so nothing is
     written from the older text.
  9. **One local draft per bit and cut** (`Documents/bit/<bit>.json`; the web: `localStorage`). The storage is
     `lib/draftFiles(.native).ts`; `lib/scriptDraft.native.ts` is gone and `lib/scriptDraft.ts` sits on the shared
     storage with the same file path and web key as before (CONFIRMED by the iOS dev review).
  10. **A draft found later is saved as the newest version even when the bit moved on elsewhere** (the script's
      rule is the opposite: the server wins), **unless some version of that cut already says it**: then its save
      had landed and only the answer was lost, and it is dropped (iOS dev review, must-fix).
  11. **Offline he can type**: the draft waits, the line reads "Offline · kept on this iPhone", and it is sent when
      the connection returns (the page, or `flushBitDrafts` from the root layout at launch).
  12. **The fallback** is `prefs.bitEditor: 'classic'`, Settings › This device › "Write button on a bit": the
      Slice 1 page's lines, bottom Write button and one-box editor. One of the two is drawn, never both.
  13. **`(bits)/spike.tsx` is kept** (the first Slice 2 listed it for removal): it is the Device checks screen and
      six of its checks are still unanswered and needed by later slices. Checks 2, 3 and 6 (the glass bar above the
      keyboard, dictation into per-line inputs, Backspace between inputs) are retired with `spike-write.tsx`; the
      other numbers are kept so an answer given on build 28 names the same check. `(bits)/write.tsx` is a redirect
      to Bits (`/write` stays in the route table).
  14. **No bottom button on the document.** The bottom toolbar's Write is drawn only in the fallback, so 3c's
      device check 1 no longer gates this page (it still decides where Slice 3's Send sits).
  15. **`keyboardDismissMode` is `interactive` on this page** (the shell's default `on-drag` would put the keyboard
      away whenever he scrolls his own document), and while typing the field is at least as tall as the page, so a
      tap under a short bit moves the caret to its end instead of landing on nothing.
  16. **The shell (`lib/Screen*`) gained** `scrollWatch` (scroll position, height, the dismiss mode), `offlineNote`,
      and a ground behind the bottom footnote line.
  17. **Done after the slice (2026-11-16):** a version's limit is 40,000 characters (§4.3 Limit). **Not done, by
      design:** the punch mark and the folded tags are not drawn in the document (no read carries `line_meta` yet,
      3c Slice 3); they are still drawn in the fallback.
  18. **`lineRules` gained `anchorsOf` and `findAnchor`** (a place in the document found by the line's own words,
      with its idea and where its words sit in the text), so Slice 3 can hang marks on them (B12). Nothing of
      Slice 3 is built, and nothing on the page uses them yet.
  19. **From the two reviews (iOS dev, iOS design; both "ship with fixes"; every must-fix landed):** the caret is
      taken to be at the END of the text until the field reports otherwise (a tap after the last word reported
      nothing, and the page would have scrolled to the top); a save is single-flight from before its first await
      (blur then unmount could post twice); a stale draft cannot bury a newer version (call 10); the offline line
      is not drawn over his words; a card cannot write over words still saving (call 8); no snack on Done (call 3).
      Also landed: `background` only (not `inactive`), the pause gap, the caret measured to the end of its word, a
      newer server version that arrives while the keyboard is up on untouched text is taken on blur instead of
      being overwritten, a failed save is tried again by itself, Done is prominent, VoiceOver hears which card a
      button acts on, the gap row is "Blank line", a new bit has one placeholder.

### Slice 3: The Redline loop on a bit (rewritten 2026-11-17 to what was built; was "Marks and redlines (and the pips)")

**The owner's words this slice is built on, word for word:** "remember a bit is a document, it just has a bunch of
ideas for the same topic ... it should be as easy as editing a google doc on your phone, with the power of
punchable"; "when i say redline, i mean our repo redline, our app we integrating into [another of his projects,
named in the original] as well that idea"; asked whether a round returns changes in place with keep/skip or a whole
new draft: "lets make it a whole new draft"; on marking up the AI's answer and sending again: "yes possibly".

**After it he can:** select words in his bit (or put the cursor on a line) and mark them Change, Ask or Keep from a bar
on the keyboard, or Copy them to a set or a new bit; see the marks wait in a tray and as quiet highlights in his
words, even after leaving the page; send them once; get back a whole new draft with his questions answered; flip
between the draft and his words, use it (a new version, with Undo) or discard it; mark up the draft and send again.
The draft waits if he closes the app, and Bits says "Waiting for you".

- **Server:** `pk-creative/src/redline.rs` (the shared rules: locate by words, ideas, parts, the checks),
  `pk-creative/src/reducers/redline.rs` (store `redline`), `JOB_KINDS` + `redline` (Rust and
  `api-dev/creative_api.py`), `pk-worker/src/kinds/redline.rs` (kind `redline`, Rust only), `pk-api/src/writes/redline.rs`
  (`POST`/`GET /api/jokes/<id>/redline`, `POST /api/jokes/<id>/redline/resolve`, `GET /api/redlines`),
  `engine/config.hosted.toml` task `creative_redline`; two regex Ingress paths (`/api/jokes/[^/]+/redline$`,
  `/api/jokes/[^/]+/redline/resolve$`) and one exact (`/api/redlines`) in `infra/k8s/prepared/t10-api-rs.yaml`.
- **App:** `rules/markRules.ts`; `lib/marks.ts` (the tray on the phone), `lib/redline.ts` (the calls, the polled round),
  `lib/redlineActions.ts` (Send, Copy to…), `lib/MarkBar.tsx` + `lib/DocMarkBar.tsx` + `lib/BarGround(.native).tsx` (the
  bar), `lib/MarkTray.tsx` + `app/marks.tsx` (the tray, a form sheet), `lib/MarkedText.tsx` (the highlights, the
  read-only document); `lib/BitDoc.tsx` (the selection, the highlights, room for the bar), `joke/[id].tsx` (the bar on
  the keyboard, Send in the bottom bar, the round's row under the document), `joke/[id]/draft.tsx` (the draft),
  `(bits)/bits.tsx` (Waiting for you), `lib/SetPicker.tsx` + `rules/aroundRules.ts` (a note on each add).
- **Tests:** `tests/markRules.test.mjs` with `--red` (six arms: anchor, keep, stale, tint, target, gone);
  `tests/aroundRules.test.mjs` (the note on an add); Rust: `pk-creative` redline and reducer tests (a red arm on the
  drop share), `pk-worker` unit tests and `tests/redline.rs` (the golden prompt `tests/goldens/redline_prompt.txt`, a
  Keep broken then fixed, refused twice never shown, a provider failure), `pk-api` route tests and
  `tests/writes.rs` `the_redline_round_queues_waits_and_closes` on a scratch database; `store_jobs.json` and the
  pk-llm `requests.json` regenerated by their Python generators.
- **Owner's phone checks:**
  1. Open a bit, tap once on a line: the bar is on the keyboard and its buttons are on at once; it reads "On “…” " with
     that line.
  2. Select three words, tap Keep: the keyboard stays, the bar says "Kept" and the Keep button reads Undo for a moment;
     Undo takes it back.
  3. Select other words, tap Change: the keyboard goes and the Marks sheet opens with the note field ready; type
     "sharper". Tap Ask on another line and type a question. Close the sheet: the words show a highlight with an
     underline (Change solid, Ask dotted, Keep double); "3 marks" sits in the bottom bar.
  4. Leave the bit and come back, or close the app: the marks are still there.
  5. Send 3 marks: "Reading your marks…" under the text; leave the app; reopen: Bits shows "Waiting for you · <bit>".
  6. Open it: your question answered (with the AI badge), Draft · Yours flips, the changed lines are tinted, the kept
     words are exactly yours. Use this draft: "Saved as v10" with Undo; Undo puts the old words back.
  7. Send another round, then type in the bit before it comes back: the draft says it was made from the earlier words
     and Use asks first.
  8. On a draft, select words and mark them; Send: the next draft replaces it, and the earlier Keep is still kept.
  9. Copy to… on a line: Add to a set shows the words as the note; the set shows them under the bit after Done. New
     bit from these words makes a bit with Undo.
  10. Discard a draft: Undo brings it back.
- **Built 2026-11-17 (branch `app/redesign-slice3`; not merged, not pushed, not deployed, no phone build).** Nothing
  was run on a phone. The calls made against this spec:
  1. **A whole new draft, not tracked changes** (owner). No hunks, no Keep/Skip per line, no `lib/Redline.tsx` from
     the session sheet's `HunkRow`, no version per Keep (B4). One version when he uses a draft, with Undo.
  2. **The pips and the connections on a line are out** (owner, this slice's brief); connections stay under the
     document. The cosine leg of the anchor (Slice 1 call 3) stays not done.
  3. **A mark is `{type, text, at, note}`**, not `{i, line, span, chip}`: no line index, no Punch/Tighten/Tag chips.
     With nothing selected a mark is on the caret's whole line.
  4. **The bar is driven by the selection** (§4.2: no custom edit-menu items in RN 0.88). On the phone it is a plain
     view on the keyboard's top, measured from the keyboard's frame (the `lib/KeyboardBar` pattern), not an
     `InputAccessoryView`: that binds to its input once and never after the input is rebuilt, and it is deprecated in
     RN 0.88 (iOS dev review, must-fix). On the web it is in the shell's bottom bar whenever the document is open
     (a click on it blurs the text first). While it is on the keys it also carries the page's footnote line.
  5. **"Fork" is "Copy to…"** (design review: a verb, and the bit is not changed): an action sheet with Add to a set,
     as a note (the existing multi-tick picker; the words, cut to one line of 200 characters, go under the bit in each
     set it is added to; a set made inside the picker gets the bit without the note) and New bit from these words.
     **Not built:** Redline's "whole session" fork and a question asked about the words elsewhere (an Ask here covers it).
  6. **Keep is made at once; Change and Ask open the tray at that mark's note.** The Keep button is its Undo for four
     seconds (until his next mark while VoiceOver is on); the tray is the untimed path. Remove in the tray leaves a
     "Removed … · Undo" row in place (a snack would sit behind the sheet).
  7. **Marks live on the phone and on the newest stage document only**: not on the rant cut, an older version or the
     older editor. The tray's file names put the marker first (`draft~<bit>`) so a long id cut to the file-name limit
     cannot make two trays one.
  8. **Send saves his typing first** (the marks are on the words he sees, which must be a version), one send at a time
     from the first tap; a draft already waiting asks before it is replaced.
  9. **The store's ops are `send` and `resolve`** (verdicts used, discarded, open), not `keep`/`unkeep`/`round`/
     `resolve {n}`. **Keeps do not persist into later `rant` and `punchup` jobs** (not built): they ride along only into
     the next round on the draft, at most 40 marks in all.
  10. **The text marked is read on the server** in both rounds (the version from the KB, the draft from its job); a
      quote not in it is a 409, as is a draft that is no longer the open round's. A double tap of Send is one round.
  11. **A new task, `creative_redline`** (Opus 5.5, effort high, `max_tokens` 32,000), where `rant_draft` and
      `rant_line` share `creative_rant`: a whole draft needs the room. The Python generator recorded its three routing
      cases (59 in `requests.json`). Unlike the spec's R8, the Python image does change: **only `JOB_KINDS` gains the
      name**, because the Python API still replays the shared `jobs` store and stops every creative route on a kind it
      does not know. Everything else is Rust.
  12. **A long bit is rewritten around its marks** (over 12,000 characters, `WHOLE_CAP`): the whole bit goes as context,
      the model returns only the ideas with a Change or an Ask and one idea either side, and the server splices them in.
      In that mode a Change cannot move words to an idea far away. In round two his bit goes along for context only up
      to 12,000 characters (the draft carries the rest).
  13. **The checks** are those of §3.2: Keep byte for byte, no unmarked idea dropped (the 50 % share of content words is
      the lead's number), `was` his words (round two: his bit only, never the model's own draft; security review), a
      misquote in an answer loses its quotation marks; refused, retried once, then never shown. Not built as a check:
      "a Change on a span leaves the words outside the span" (a whole draft rewrites around a mark; the drop check is
      the guard) and the tag checks (no tags).
  14. **Stale is honest:** a draft made before his last save says so, Use asks first, and "Put my marks on what I
      saved" puts the round's marks back on his bit (the round closes; Undo reopens it).
  15. **Discard teaches the voice profile nothing** (the first spec's Skip did). Use teaches it the kept text, as every
      kept AI draft does.
  16. **Waiting for you on Bits lists ready drafts only**; `GET /api/redlines` also says waiting, refused and failed.
  17. **`rant-line.tsx` is not removed**: the Rant script still opens it.
  18. **From the reviews** (iOS design, iOS dev, security: each "ship with fixes"; every must-fix landed and went back
      for a second look, whose must-fixes landed too): VoiceOver hears each mark and each change of the round
      (`announceForAccessibilityWithOptions`, queued; never the first read); marks are told apart by more than colour
      (Change underlined, Ask dotted, Keep double, the swatches as a legend, a themed `violetSoft`); the bar is one line
      and one 44-pt row on Liquid Glass, labels capped at 1.3; the bar starts from the selection the focusing tap made;
      offline is said in the bar; the tray's Send is in its trailing bar, off offline; an Ask's error shows only after
      he leaves its field or tries to send; the draft hides its bottom bar while words are selected and makes Send
      prominent when round-two marks wait; the court-material check reads the bit, the notes and the title together.
  19. **Not done, each a follow-up:** pruning of finished jobs (each round's row holds the text marked, his bit and the
      draft; tens of KB at a typical size); `/api/export` and the delete-me inventory do not list the `redline` store
      (nor `links`, `setnotes`); marks on the rant cut.
- **What a round costs** (measured 2026-11-17 with the Anthropic token counter on synthetic bits, three marks; Opus 5.5
  at $4 in and $20 out per million tokens; his voice profile and the learned block come on top, cached, and their size
  is his; thinking tokens are UNKNOWN until a real call, assumed 1,000 to 4,000):

  | Bit | Mode | Tokens in | Draft tokens out | A round |
  |---|---|---|---|---|
  | 4,000 characters | whole | 2,096 | 1,265 | about $0.06 to $0.12 |
  | 40,000 characters | parts (3 ideas rewritten) | 13,817 | 316 | about $0.08 to $0.15 |
  | 40,000 characters, if it were rewritten whole | (not built) | 13,817 | about 12,700 | about $0.30 to $0.40, and past the default 16,000 `max_tokens` with thinking |

  A refused draft is asked for once more: up to twice the cost. With the daily cap of $2.00 that is about 15 to 30
  rounds a day.

### Slice 4: Sets with bullet notes

**After it he can:** write his own notes under each bit of a set, see why two neighbours sit together, and read
those notes on stage on today's Record screen.

- **Server:** store `setnotes`; `POST /api/boards/<id>/notes`; `notes` and `notes_from` on the build steps; `note`
  on `POST /api/boards/<id>/items`; the "same shape twice" seam in the build.
- **App:** `list/[id].tsx` (notes under each row, every seam drawn, "+ Add a bit" as today), `lib/BulletNotes.tsx`,
  `rules/setNoteRules.ts`, `bitnotes.tsx` and `lib/stageNotes.ts` read his notes first, the picker of Slice 1 sends
  highlighted words as a note, Sets cards show the bits' names.
- **Tests:** `tests/setNoteRules.test.mjs` with `--red`; Rust: notes survive remove and re-add, Undo, an archived
  board refuses, red arm on the join.
- **Owner's phone checks:** open a set, type a note under a bit, Return adds another, Backspace on an empty one
  removes it; reorder the bits: the notes travel with their bit; Record → tap the bit: his notes, not the app's;
  tap the words between two bits: a callback, the usual order, a bridge to draft, or "same shape twice".
- **Built 2026-11-16 (branch `app/redesign-slice4`; not merged, not pushed, not deployed, no phone build).** Server:
  `pk-creative/src/reducers/setnotes.rs` (the store), `pk-creative/src/setnotes.rs` (derived notes, the same-shape
  seam, whose tie a seam names), `pk-api/src/writes/setnotes.rs` (`POST /api/boards/<id>/notes`), `note` in
  `writes/boards.rs`, the join in `routes/sets_detail.rs` and `routes/around.rs`; one regex Ingress path
  (`/api/boards/[^/]+/notes$`) in `infra/k8s/prepared/t10-api-rs.yaml`. App: `rules/setNoteRules.ts`,
  `lib/BulletNotes.tsx`, `lib/SetCard.tsx`, `list/[id].tsx`, `bitnotes.tsx`, `lib/stageNotes.ts`. The calls made
  against this spec:
  1. **The caps are the route's, not the reducer's** (§3.3 put them on the op). The reducer checks only the shape (a
     list of texts for one board and one bit); 12 notes, 200 characters and "one clean line" are checked by the route.
     A cap in the reducer that was ever tightened would refuse the rows already written and the store could not be
     replayed (security review).
  2. **The answer carries more than `{ok, notes, was}`:** `board`, `joke_id`, and `already: true` when the list sent
     is the list stored (nothing is written). Notes need the bit to be on the list (400); an archived list refuses in
     the words every other change to it is refused with; a list the app made (a draft) takes notes and stays a draft.
  3. **A note is cleaned, not refused:** control characters, line separators and direction overrides become spaces,
     the ends are trimmed, blank bullets are dropped. Each note, and the notes read together, pass the court-material
     check (422, nothing written), as every stored text does.
  4. **`note` on `POST /api/boards/<id>/items`** goes after the notes the bit kept from an earlier stay on the list
     and is not written twice; the answer adds `notes` and `note_added`. The add and the note are two rows in two
     stores: when the note fails after the add landed the answer is still 200 with `note_error: true`. **The app
     sends no note yet:** nothing on the bit page can be highlighted until Slice 3, so the picker is unchanged
     (`creative.addItem` takes the argument).
  5. **"Same shape twice"** (`transition_in.same_shape: {source, shared, edge} | null`) is a stored `same_shape` edge
     between the two neighbours, or two shared device or structure labels above the bar Around this bit lists Same
     shape by; a pair he called Not connected is silent unless the edge is stored. It wins the seam's label.
  6. **`transition_in.source` (`his` | `ai` | null) is new:** a callback or a shared premise that is only the app's
     reading keeps its name, carries the AI badge on the row and gets no check; the check is for what he did on
     stage, wrote, made or confirmed (design review).
  7. **The notes and the dismissed pairs are extras on the two reads:** a `setnotes` or `links` store that cannot be
     replayed costs the notes or the seam, not the page (security review).
  8. **Every seam is one row, opened in place** (a mark, the words, the AI badge where it applies, a chevron):
     "Usual order", "Callback", "Your segue", "Could lead in", "Same premise", "Same shape twice", "No bridge yet".
     "Gap here · add a bridge" is now "No bridge yet" with **Draft a bridge** inside. No sheet (a bridge sheet would
     open over it).
  9. **Reading and writing are two looks.** Notes read as wrapped bullets; a tap turns that bit's notes into
     single-line fields (iOS reports Backspace in an empty field only for a single-line one). Return adds a note,
     **Return on an empty note puts the keyboard away**, Backspace on an empty note removes it, "Add a note" sits
     under every bit. Not done: only the focused note as a field (a long note is one line while it is edited).
  10. **Saved when he leaves a bit's notes**, when the app leaves the foreground and when the screen goes; never per
      Return. Adding is quiet; a removed or changed note gets the snack with Undo; **the list's menu gains
      "Undo: <the last change>" with no clock on it**, fed by the notes and by move, remove, add, swap, rename and
      length (`HIG.md` §7). A save that fails keeps his words with "Not saved · Try again", for as long as the app
      runs. **Offline he cannot write notes** ("Add a note · offline"); there is no queue.
  11. **The derived words are shown under "From the bit", muted, and are never saved:** the editor opens empty and
      they stay in sight until his first note is saved (`tests/setNoteRules.test.mjs --red` pins it). Each derived
      note is cut at 300 characters (a version can be a 40,000-character document; the step's `text` has it whole).
  12. **The row lost its quote line** (the notes are under it), the footer hint line is gone, and "Add a joke" reads
      **"Add a bit"** on the list and on the add sheet.
  13. **The Sets card is a system card** (name, "4 bits · 6:10", the bits' names on one run, "Tonight · 7:00 slot ·
      Edited Nov 16"); the paper card with its fixed colours is gone. **No "last recorded" line:** the server is not
      told which set a tape followed until Slice 5.
  14. **The notes sheet shows his notes first at stage size;** opened from the stage, the bit's words are folded
      behind a "The bit" row; opened from the list they follow open.
  15. **No differential:** the route has no Python twin. Proof is the unit tests in the three Rust files and one
      scratch-database test in `pk-api/tests/writes.rs`, with four red arms run once each (the build ignoring the
      store; a remove that clears the notes; `was` holding the new list; the archived check gone): each failed with
      exit 101, then passed with the plant out.
  16. **Not done, each a follow-up:** `/api/export` and the delete-me inventory do not list `setnotes` (nor `links`
      from Slice 1); `pk-wire`'s `Step` does not carry the new fields (pk-api answers with values); the snack can sit
      behind the keyboard when he moves straight into another bit's notes; whether holding Delete eats into the note
      above is a device check.

### Slice 5: The walk-through and After

**After it he can:** Record or Film a set from one screen that shows one bit at a time with his notes or the full
lines, go Next and Back, and get the recording back cut up by bit with what he actually said.

- **Server:** `board` and `splits` on `POST /api/sets/transcript`, `SetTextInput` and `meta.json`
  (`pk-api/src/writes/transcript.rs`, `pk-wire`, `pk-worker/src/kinds/set_text.rs`).
- **App:** new `run/[id].tsx` (full-screen modal, `?mode=sound|video`) on `lib/RecordControl`, `rules/runRules.ts`
  (the state machine; splits from the recorder's position; Back; pause on a call), `after/[rec].tsx` (per bit: time,
  his words cut by the splits from the phone's own segments, "you changed a line" → the bit), `lib/recordings.native.ts`
  (`splits`, `board`, `mode`), `rules/memoRules.ts` `setBody`; `stage/[id]` redirects to `run`; `film/[id]` stays
  for filming one bit; the ten-minute cap decision for a filmed set.
- **Tests:** `tests/runRules.test.mjs` with `--red` (Next then Back then Next makes three spans; a call's pause is
  not inside any split; splits never overlap or pass the duration); `tests/memoRules.test.mjs` for the body; Rust:
  the route keeps valid splits, refuses a bit not on the board, red arm; a `set_text` golden with splits.
- **Owner's phone checks:** a set → Record: the disc, then the first bit with his notes; Full lines and back; Next
  through three bits, Back once; lock the phone and unlock: still recording; End → After shows each bit's time and
  words, a skipped bit says so; Film: the camera on top, same screen, 3-2-1, Keep puts it in Photos; no buzz on
  Next while recording; a call mid-take: "Paused by a call", Resume, and the bit times are still right.
- **Built 2026-11-17 (branch `app/redesign-slice5-6`, with Slice 6; not merged, not pushed, not deployed, no phone
  build).** Nothing was run on a phone. Server: `board`, `splits`, `mode` on `POST /api/sets/transcript`
  (`pk-api/src/writes/transcript.rs` `check_walk`, `check_board` under the creative lock), `pk-wire` `SetTextInput`
  and `Split`, `pk-worker/src/kinds/set_text.rs` and `worker/execute.py` (`meta.json`). App: `run/[id].tsx`,
  `after/[rec].tsx`, `rules/runRules.ts`, `rules/memoRules.ts` (`setBody`, `walkFieldsRefused`),
  `lib/recordings(.native).ts`, `lib/film(.native).ts` `keepSetTake`, `lib/RecRows.tsx`, `stage/[id].tsx` (a redirect).
  The calls made against this spec:
  1. **One screen, two recorders, never at once.** `?mode=sound` (expo-audio, keeps taping locked, a call pauses it)
     and `?mode=video` (the camera on top, the bit under it, 3 · 2 · 1, the review's Redo / Keep). Camera on in the
     middle of a sound take (§4.4) is not built: it is Slice 9's.
  2. **Splits come from the recording's own position:** the recorder's clock for sound (it holds still during a call,
     so a call is in no split: `tests/runRules.test.mjs --red` plants the wall clock and is caught); for video the
     time since the start, moved once to the file's own clock by the camera's first progress report
     (`onRecordingProgress`, iOS dev review), so most of the camera's start-up bias of §3.4 is gone. A tap through a
     bit in under a quarter second is no span. Next, Back, Next makes a second span for the bit visited twice.
  3. **The dock is Back · disc · Next** (§4.4); Pause moved to the top bar while recording; Next greys as "Last bit";
     no haptic on Next or Back while a microphone is live; the start haptic fires on the press, before the
     microphone opens (design review, HIG/playing-haptics; the video take keeps Film it's).
  4. **Notes / Full lines:** his notes for the bit (or the app's, muted, while he has written none: Slice 4), or the
     bit's setup and reveal; a 44-pt "Show full lines" / "Show my notes" button under the words, which are plain
     text VoiceOver reads (design review must-fix).
  5. **A filmed set is a recording of kind `set` with `mode: video`:** the one copy goes to Photos on Keep, its words
     go up as a set transcript with the splits, the .mov leaves Documents once the words exist (as a filmed take's),
     and its row plays it or opens Photos. The take is capped at 20 minutes (`SET_TAKE_S`, this slice's cap decision)
     and at a file size from the free space; under 2 GB free the disc greys "no room" (iOS dev review). Heat and size
     at that length are a phone check.
  6. **The route refuses** a list he does not have, a bit not on it, splits out of order, overlapping, past the take
     or more than 500, a `mode` other than sound or video (400). **The phone never loses the words to it:** a 400 on
     a body with the list is sent again with the words alone (`walkFieldsRefused`). The job key gains a digest of the
     list, the splits and the mode only when they are sent, so the differentials (no such fields) see the Python's
     key, and the same words sent with corrected taps are a new job (security review).
  7. **After** reads the phone's row: each bit's time (visits added up), "Skipped", and once transcribed what he said
     there (`wordsBySplit`: a segment goes to the span its middle falls in; kept on the row as `bit_words`, because
     the segments leave the row once sent). "You said it differently · Open the bit" when at least four of his
     spoken content words, and 30 % of them, are in no line of the bit (`saidDifferently`, the lead's numbers). No
     laughs (§5 R4). The web has no After (it keeps no recordings).
  8. **The `set_text` golden with splits** is a new scenario of the Python generator (`set_text_splits`): the Python
     worker writes the same three keys, so the worker differential covers them.
  9. **Not done, each a follow-up:** the set cards' "last recording" (needs a phone tape's server set id: Slice 7);
     using the splits on the server (performances, `performed_as`: Slice 7); the splits of a sound take are in memory
     until the row is written, so a crash between Stop and the row keeps the tape but not its splits.

### Slice 6: Rants become sets; the tabs become Bits · Sets · Tapes

**After it he can:** find every topic and rant as a set with the same bits and words, film it from there, and use
one Tapes tab for every set, video and memo he has recorded.

- **Server:** `cargo xtask rants-to-sets` (dry run, then the owner's go, backup first; logged in the private deploy
  log with the rollback: the added boards are archived by id); "Also kept" read on the set by `seed.topic`.
- **App:** remove `(tabs)/(rant)`; new `(tabs)/(tapes)/tapes.tsx` (the phone's index + `GET /api/sets`, rows open
  today's pages); `settings`, `profile`, `voice`, `learned` to a root sheet stack behind the avatar on all three
  roots; `useTonight` and the draft flush to `app/_layout.tsx`; `lib/TabsShell(.native).tsx`, `lib/Screen.tsx`,
  `lib/Icon(.native).tsx` (`tapes`), `rules/routeTable.ts`; redirects for `/topic/*` (to the set), `/filmed` (to
  Tapes) and the unreachable leftovers of §7 Q7; the set's More menu gains "Draft it as one piece…".
- **Tests:** the migration over a scratch store (a topic with two rants, kept bits, a kept moment, a jot, a dropped
  piece, a tombstone; run twice = one board; nothing in `topics` or `script` changed), red arm on the idempotence;
  `tests/routes.test.mjs` (every old URL lands somewhere); `tests/rantTab.test.mjs` retired with its rules.
- **Owner's phone checks:** before the build, the dry run's list of sets to be made, for his yes; after: three tabs;
  each old topic is a set with its rants first and kept bits after; a rant opens as a bit with every version and
  its "rant" label; Film from the set; Tapes lists sets, videos and memos, newest first; the avatar opens Settings
  from every tab; a draft he was mid-way through is saved as a version.
- **Built 2026-11-17 (branch `app/redesign-slice5-6`, after Slice 5; not merged, not pushed, not deployed, no phone
  build; the migration ran on scratch databases only).** Server: `pk-api/src/migrate.rs` (`plan`, `mint_ids`,
  `rants_to_sets`), the binary `pk-rants-to-sets` (in the pk-api image), `cargo xtask rants-to-sets`. App: the tab
  bar, `(tabs)/(tapes)/` (Tapes, the performed set moved there, `/filmed`), `(me)/` (You, Settings, Voice, Learned,
  Usage), `(tabs)/(bits)/index.tsx` (`/`), the topic pages moved to `(tabs)/(sets)/topic/`, `lib/scriptFlush.ts`,
  `lib/PopTo.tsx`, `lib/AlsoKept.tsx`, `rules/tapesRules.ts`, `lib/TopBar.tsx` and `lib/Screen.tsx` (the web's roots).
  The calls made against this spec:
  1. **The migration** adds `create` events to `storyboards` in one transaction and changes nothing else: per live
     topic a list titled with its words (`seed: {topic}`), its rants first (oldest first), then the bits he kept in
     the order he kept them, jots included, each bit once; a one-bit list (`seed: {rant}`) for each rant no topic
     holds; nothing for a deleted topic, whose rants count as held by it. A topic or rant that already has a list
     seeded from it (live, archived or deleted) is skipped, so a second run makes nothing (scratch test, red arm).
     A bit that is no longer a bit is left out; a list holds at most 200 bits (the route's cap, security review); a
     title the court-material rules stop becomes "Untitled topic" / "Untitled rant" (security review).
  2. **Dry run by default; it prints counts only.** `--list` adds each list's title and size (his words: for his eyes,
     never a log). `--write` refuses unless `--i-have-a-backup` is also given, and prints how many lists it made;
     their ids (which start with the title) only with `--list`. Rollback: archive the lists whose `seed` names a
     topic or a rant. Run it once, by one person (two runs at once are not guarded across processes).
  3. **The app works before and after the migration.** A topic no list was made from yet is listed on Sets under
     "Topics not yet in a set" and opens its old page (now in the Sets stack); once its list exists, only the list
     shows and `/topic/<id>` opens it (the page waits for both board lists, so nothing typed is lost to the redirect).
     Rants with no topic are bits in Bits either way. `useTonight` was not moved: Bits, now the first tab, mounts it at
     launch.
  4. **"Also kept"** (the taped moments he kept for the topic) is read through the existing `GET /api/topics/<id>`, not
     a new route; **"Draft it as one piece…"** in a topic-made set's menu opens the topic's rant script.
  5. **Tapes** is one list, newest first: the phone's recordings (memos, filmed takes, sets taped here, with their
     state, play, Delete, Try again, and "By bit" for a walk-through set) and the server's sets; a phone set already
     sent is shown once when the server holds a set of the same day and title (`tapeRows`, red arm). The Recorded
     segment left Sets; `/set/<id>` lives in the Tapes stack, so opening a night from Bits or Sets switches to Tapes.
  6. **The avatar is on all three roots** and opens You, the first page of a root modal stack `(me)` with a standard
     Close (design review); Settings and the rest push inside it; its links into the tabs pop the modal
     (`router.dismissTo`, iOS dev review).
  7. **`/` and `/filmed` pop back to their tab's root** (`lib/PopTo`, `dismissTo`, no animation): a `<Redirect>` on a
     tab's first screen leaves two copies of it and a Back at launch (iOS dev review). `/stage/<id>` redirects to the
     walk-through.
  8. **Not done:** the other leftovers of §7 Q7 (the builder, working sets, Notebook, After a show…) still open as
     they did; removing them waits for the owner's answer to Q7. `tests/rantTab.test.mjs` is retired; its rules
     (`rantRules`, `scriptRules`) stay because the topic pages still use them; `tests/tabsShell.test.mjs` replaces it
     (red arm: main before the slice fails seven checks).

### Slice 7: A tape as its transcript, cut by bit

**After it he can:** open any recording and read what he said in order, see which bit each stretch is, fix a wrong
one, turn a riff into a bit, and put what he said on stage into the written bit.

- **Server:** first, the scratch run for §2.7's two defects and the fix; splits → performances and `performed_as`
  (his); the match step for untapped stretches (vector, then the judge; private eval); no duplicate idea for a
  matched stretch; `GET /api/sets/<id>/transcript`; the job's result carries the set id.
- **App:** new `(tapes)/tape/[id].tsx`, `lib/Blocks.tsx` (the coloured-edge block list, shared with Slice 8),
  `rules/tapeRules.ts`, seek-and-play in `lib/localPlay.native.ts`, the set page's "Recordings of this set", the
  bit page's On stage group gets "what you said" on its line; `set/[id].tsx` stays reachable from the tape as
  "Numbers" where a set has laughs.
- **Tests:** `tests/tapeRules.test.mjs` with `--red` (stretches cover the transcript once, in order; "use this
  wording" replaces exactly the anchored line); Rust: a phone set appears under `GET /api/sets` with performances
  (the defect's failing-capable test), splits win over the model's cut, an uncovered run is `joke: null`, red arms.
- **Owner's phone checks:** record a short set with Next taps → Tapes → the transcript with each bit's name on its
  stretch; record one without tapping → the guessed stretches carry the AI badge; Not this bit; a "Not in any
  bit" stretch → New bit from this → the bit exists; Use this wording in the bit → "Saved as v…" with Undo; tap a
  stretch and it plays from there (recordings made on this phone).

### Slice 8: A Google Doc as written

**After it he can:** open a doc inside the app exactly as he typed it, see which bit each blurb went to, fix any of
them, and find many wordings of one idea as one bit's versions; Bits shows a line of his on every row.

- **Server:** the private measurement first (R1); `pk-gsync` posts the doc text (needs the gsync switch-over to
  Rust, which is waiting on the owner); kind `doc_sort` (Python image first); store `docsort`; `GET /api/docs`,
  `GET /api/docs/<id>`, `POST /api/docs/<id>/blurbs/<blurb>`; `first` on the Bits rows.
- **App:** `(bits)/doc/[id].tsx` on `lib/Blocks.tsx`, `rules/docRules.ts`, "Your docs" and "Waiting for you" on
  `bits.tsx` (the inbox row and the same-idea banner become rows of it).
- **Tests:** `tests/docRules.test.mjs` with `--red`; Rust: the blurb splitter over invented long docs (no headings,
  mixed headings, lists, one-line jots, a 30,000-character doc), blurb ids stable across an edit elsewhere, his
  correction survives a re-sort, the legal rules stop a flagged doc, red arms.
- **Owner's phone checks:** Bits › a doc: his text top to bottom; a sorted blurb names its bit; a loose one offers
  Add to a bit / New bit / Not a joke; "Not this bit" makes it loose; edit the doc in Google Docs, wait for the
  sync: his corrections are still there; a bit with four blurbs shows "4 versions".

### Slice 9: The Record button

**After it he can:** start recording with one tap from anywhere, then follow a set or turn the camera on, or
neither; the recording lands in Tapes.

- **Server:** none.
- **App:** `lib/TabsShell.native.tsx` (the accessory, or the fallback the owner chose in §7 Q1), `run/[id].tsx`
  opened with no set ("Go ahead" + "Or follow a set"), Camera on per §4.4, a recording under 4000 characters filed
  as a memo and a longer one as a set (today's rule, `HANDOVER.md` "Voice memos"), the web keeps Record in its
  header.
- **Tests:** `tests/runRules.test.mjs` (following a set after the start begins the first split at that moment;
  Camera on makes a second file under the same take), `--red`.
- **Owner's phone checks:** from each tab and from a bit's page, one tap: "Starting…", a tick, the clock runs; Stop
  saves and Tapes shows it; start, then pick a set: the walk-through continues on the same recording; Camera on;
  Close offers Discard; the bar still minimises on scroll and nothing covers Write on the bit page.

---

## 9. Roadmap (build nothing)

- **Decide the map's job (List/Map switch on Bits?).** The current map stays reachable from the Bits toolbar
  exactly as today; no slice above touches `(bits)/map/`.
- Laughs for tapes recorded on the phone (§7 Q3).
- Custom buttons inside the selection bubble (a native text view), §4.2.
- The Action Button, a Control Center control, a Home-screen quick action for Record; the iOS 27 `prominent` tab.
- Trending and posting (carried from the Rant spec §9).
- A note under several bits (attach is one-to-one today).

---

## 10. Amendments this spec makes elsewhere (when its slices land, not now)

`HIG.md` §2.1 route tree and §3.9 (the walk-through; "starts at once" for Record only); `HIG.md` §7's row saying
there is no Increase Contrast API (`AccessibilityInfo.isDarkerSystemColorsEnabled` exists); `UX_2026-10.md` tabs;
`SPEC-2026-11-03-bit-page-3c.md` §14 (Slices 2-5 replaced by Slices 1-3 here; its Slice 3 pile routes and Slice 5
overnight filing are not in this spec and stay proposed); `SPEC-2026-11-06-rant-tab.md` status line (the tab is
retired by Slice 6; §6 Finish and §9 stay proposed); `rust/crates/pk-gsync/README.md` (the doc text file).
