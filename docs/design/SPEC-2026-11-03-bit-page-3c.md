# SPEC 2026-11-03: the bit page (design option 3c) and the build slices

Status: **approved design, build spec** (owner chose option 3c on 2026-11-03 and said "lets get it in"). This file
supersedes the screens (§3) and the build order (§11) of `SPEC-2026-11-02-bit-sessions-and-joke-teaching.md`; that
spec's **§1 his method, §5 data model, §6 learning without weights, §7-§10 server, cost, measurement and privacy
still hold** and are referenced here, not repeated. The design pages themselves (options 3, 3b, 3c and the cold
critique of 3b) are working files outside the repo; everything a builder needs from them is written out below.

Tags: **CONFIRMED** = a `file:line` in this repo or under `app/node_modules`, a HIG page read live, or a gate run;
**INFERRED** = design reasoning or a measurement not yet made; **UNKNOWN** = a device check decides. Every joke in
this file is invented; nothing here is his material. Nothing here is a commit, a deploy or a build.

**The page in one paragraph.** A bit's page is mostly his words. Above them only the bar (his short name as the
title; the sparkle, To a set, More) and one muted idea line; his first line starts 44 pt under the bar. The bit is
lines at 44 pt each, the punch bold with one amber dot, the tags folded under "+ 3 tags". Under the bit, four
collapsed rows and one quiet Claude line: **Pile · 7**, **Versions · v8 · Nov 2 · 1:40**, **Sessions · 3**,
**On tape · 3 nights · killed twice**, **Claude has 3 thoughts**. One bottom button, **Write**, which opens focus mode
(big type, one input per line, a glass bar with Punch · Pile · hide keyboard above the keys, the system's own
dictation key). Tap a line and four actions appear under it (Punch · Tighten · Tag after · ⋯); tap it again and Write
opens at that line. Nothing on the page explains itself in a sentence, with one argued exception (the first-use tip
under the pile). Above the fold the page goes from 33 elements (3b) to 15.

---

## 1. Decisions taken (lead calls, owner may flip)

The design page asked nine questions and 3b left four open. These are the lead's calls; each is a one-line change if
the owner flips it, and the §13 open questions list says what stays genuinely open.

| # | Question | Decision | Flip cost |
|---|---|---|---|
| D1 | To a set: in the bar, or a second bottom button beside Write | **In the bar** (`text.badge.plus`), one tap adds to the open set list with the snack and Undo; "Pick a set list…" stays in More. The bottom holds Write alone. | A plain `Stack.Toolbar.Button` beside Write with a `Spacer`; HIG-legal (one prominent item). |
| D2 | The one tip under the pile, once | **Keep it.** One sentence, shown once the first time the pile has something, gone on ×, the first Add or leaving; `prefs.tips.pile`. | Delete one `View`. |
| D3 | The stage word off the page | **Off.** The Bits list's status dot and glyph carry it; More › Stage sets it; the On tape row carries the facts. | One `StageWord` row above the lines (one more element). |
| D4 | Offline writes | **Build the outbox with the page** (Slice 2). Until it lands (Slice 1), Write and To a set grey with the one word "offline". | None; the outbox is the standing offline rule applied to the bit's writes. |
| D5 | Versions: stepper, not a strip | **Stepper.** Two 44-pt arrows, hold to repeat, a drag across the label optional. | None. |
| D6 | "killed twice" from his ratings | **His ratings**: `rating === 'killed'` among the performed rows. Unrated reads "On tape · 3 nights". | Laughs per minute instead, once laugh lines exist (HANDOVER item 6). |
| D7 | Asking Claude about a line leaves focus mode | **Yes.** The keyboard bar holds Punch · Pile · N · hide keyboard. Tighten, Tag after, Find the turn and Why? live under a line on the page. | Add Tighten to the keyboard bar (it opens a sheet over the keyboard either way). |
| D8 | Pile and Versions inline; Sessions and On tape push | **As drawn.** Inline rows need the bit in view (Add puts a scrap after a line; stepping re-renders the bit); the push rows do not. | A `kind` prop on `SectionRow`. |
| D9 | Focus mode without the idea line | **Without.** Only his lines and the keyboard; the idea is one tap back. | One small `Text` above the first input. |
| D10 (3b) | Add puts a scrap straight into the bit | **Straight in, with Undo.** His words, his tap: no Keep / Skip for his own scraps. Claude's lines still arrive as tracked changes (the 2026-11-02 rule). | Render the inserted line dashed until Save. |
| D11 (3b) | Swipe a scrap away | **Back to Loose material** ("Not this bit"), never gone. "Not a joke" hides, and lives in the long-press menu only, destructive and last. | None. |
| D12 (3b) | Done saves a version every time the text changed | **Every time.** Versions are cheap and append-only; the stepper handles forty. | A "one version per day" collapse rule on the server. |
| D13 (3b) | More than one punch per bit | **Allowed.** Each punch line is bold with the dot; the tags fold under the last one. | None. |
| D14 (3b) | Claude files scraps into piles overnight | **Allowed, into any bit's pile**, confident matches only, each shown with the AI badge and a non-timed Undo. The pile is not the bit, so the per-line rule is not broken. Tonight's "N to sort" counts only what Claude was not sure about. | A threshold in the worker's config. |

Standing rules, unchanged: HIG first on the iPhone, standard and familiar, one-handed in the dark; plain words; no
control that leads nowhere; nothing private in the public repo; no time estimates; Claude never changes a bit
without his per-line say; AI work on screen carries the badge; every write has Undo.

---

## 2. What 3c keeps from the earlier spec, and what changes

| Area | SPEC 2026-11-02 | This spec |
|---|---|---|
| His method (§1), learning without weights (§6), privacy (§10) | holds | **unchanged**, referenced |
| Data model (§5) | `session`, `session_turn`, `suggestion`, `breakdown`, … in `db/schema_teach.surql` | **unchanged**, plus the deltas in §5 here (`line_meta`, `placement`, `write_receipt`; `summary_confirmed` derived) |
| Server (§7), cost (§8), measurement (§9) | pk-api routes for sessions; kinds in pk-worker after the T8 swap | **unchanged**, plus the page's own routes (§6), two worker kinds (§7) and the per-call costs they add (§9) |
| Screens (§3) | "Work on it" bottom button, the session sheet with a six-mode picker, line select for Break it down | **superseded**: the sparkle in the bar and the Claude line open the session sheet; line actions are sessions with a span (no picker for them); the picker keeps whole-bit jobs only (Gather · Break it down · Talk); the bit page's input bar goes (the old spec's Q1, answered by 3c) |
| Build order (§11) | Phase 0 offline baseline, then Break it down first | **superseded by §14**: the page ships first in app-only slices; Claude's work waits for T8 + pk-api writes; Phase 0 (the offline baseline) is still worth running on its own and blocks nothing here |

---

## 3. Screens

Every screen below is the phone unless marked. Sizes are points on a 390 × 844 iPhone; every control is 44 pt
(HIG/buttons: "a hit region of at least 44x44 pt"; HIG/accessibility: iOS default 44, minimum 28). Type: his lines
at `type.body` 17 / 22 on the page and `type.stageProse` 20 / 26 in focus mode, both capped at 2.0× (DESIGN.md §4,
owner-signed); rows and chips cap at 1.6×. Dark is the default; every token has a light twin (DESIGN.md §3).

### 3.1 The bit page at rest (`(tabs)/(bits)/joke/[id].tsx`, the route and its reads kept)

| Element | Spec | Evidence |
|---|---|---|
| Bar | Back (the previous screen's title) · title = **his short name** (`shortNameOf`, CONFIRMED `joke/[id].tsx:82`) · trailing: **sparkle** (`sparkles`, "Ask Claude") · **To a set** (`text.badge.plus`) · **More** (`ellipsis`, not `ellipsis.circle` as today at `:186`). The sparkle and To a set are `disabled` until the bit has a line or an idea. | HIG/toolbars: "Prefer simple, recognizable symbols … Borders (like outlined circle symbols) aren't necessary." Symbol names typed in `sf-symbols-typescript` (CONFIRMED by ios-dev-expert, §4). |
| Full title | Rendered as a Title 1 heading **only when it differs from the short name** (today's rule, CONFIRMED `:179-180,199`). | Critique A: the title three times. |
| Idea line | `joke.summary` (CONFIRMED `lib/api.ts:338`) as a 44-pt `Pressable`, muted, `numberOfLines={1}` (2 at font scale ≥ 1.3), a tiny AI badge while `summary_ai && !summary_confirmed` (§5). Tap = a `fitToContents` form sheet with one field; save = `decide({target:'node', action:'edit', id, after:{summary}})` as `saveShort` does (CONFIRMED `:162-171`). Placeholder "Idea" when empty. VoiceOver: "Idea: … Written by Claude. Edit". | HIG/text-fields; the badge rule (HANDOVER "AI inferences show as the current state with an AI badge"). |
| Lines | The newest version's text split on newlines (`versionsOf`, CONFIRMED `:39-50`: one text unit per version). Each line a 44-pt `Pressable`, `accessibilityRole="button"`, label "Line 5 of 8, punch: …", `accessibilityState={{selected}}`; the text wraps (`numberOfLines` unset). A **punch** line is Inter 700 with one amber dot in a 14-pt gutter (never colour alone: weight + dot + the spoken word). Lines after the last punch fold to **"+ 3 tags"**, a 44-pt row with `accessibilityState={{expanded}}`. His first line starts 44 pt under the bar (inside the critique's 100 pt). | HIG/accessibility "Convey information with more than color alone"; HIG/disclosure-controls. |
| Section rows | One component `SectionRow` (label · meta · chevron; `kind: 'inline' \| 'push'`). **Inline** (chevron turns down, content mounts under; `LayoutAnimation` off under Reduce Motion): **Pile · 7**, **Versions · v8 · Nov 2 · 1:40**. **Push** (chevron stays right): **Sessions · 3**, **On tape · 3 nights · killed twice**, **Claude has 3 thoughts** (quiet, with the AI badge). Heights from `PixelRatio.getFontScale()`, cap 1.6; at ≥ 1.3× the meta wraps under the label. | HIG/lists-and-tables: "A disclosure indicator reveals the next level in a hierarchy; it doesn't show details about the item" → the right chevron only on rows that push. |
| Hidden at zero | Versions (one version), Sessions (none), On tape (never taped), the Claude line (0 thoughts, or the writer offline) hide. **Pile stays at "Pile · 0"** and opens on one row, "Gather from your notes ›", so an empty bit keeps its one way to fill. | HIG/disclosure-controls: "hide details until they're relevant"; no control that leads nowhere. |
| Write | The **native bottom toolbar's one prominent item**: `Stack.Toolbar placement="bottom"` with a `Spacer` and `Button variant="prominent"` "Write" (`pencil`). | CONFIRMED API (§4 row 1); UNKNOWN with the minimising tab bar (device check 1). |
| Offline strip | Moves from under the bar (CONFIRMED `lib/Screen.native.tsx:89`, `lib/TopBar.tsx:48`) to **one footnote line above Write** on this screen ("Offline · saving on this phone"): `OfflineStrip` gains a `placement` prop; `Screen` takes `offlineAt: 'top' \| 'bottom'`. | HIG/feedback: status "near the items it describes" (Write and the sparkle). |
| Loading, error | Today's `Skeleton` and `ErrorCard` (CONFIRMED `:173-174`). | HIG/loading. |
| Gone from the page | The per-bit map (`:312-330`; the cloud map has it), the input bar (`useSayBar`, `:148`), the stage word, "On the list: …", the set count, the version chips (`:209-218`), Punch up · Riff · To a set as buttons (`:242-254`), `MergeHint` and `BitTalk` (`:255-257`; the conversation becomes a Talk session, old spec §4), the Elsewhere block (`:266-310`; it becomes the On tape screen), the voice memo rows (`:259-264`; memos are pile scraps with a play glyph). | Critique A and B. |

**Counts above the fold** (one per tappable control or separately styled text block; the lines count once): 3b (a)
33 → 3c (a) **15**: Back · bar title · sparkle · To a set · More · idea line · the lines · punch dot · "+ 3 tags" ·
Pile · Versions · Sessions · On tape · the Claude line · Write.

### 3.2 Line selection and the line actions

- Tap a line: it highlights (`card2` fill, a 3-pt amber inset edge, `accessibilityState selected`, haptic
  `selection`) and a row of 44-pt actions appears under it: **Punch · Tighten · Tag after · ⋯**. The ⋯ is a
  `RowMenu` with Find the turn · Why? · Move · Cut. Tap the same line again: Write opens at that line.
- **Punch** toggles the line's mark (no new version; `line_meta`, §5). **Move** enters Reorder (§3.8). **Cut**
  writes a new version without that line, Undo in the snack and in the Versions row ("v9 · just now · Undo").
- **Tighten · Tag after · Find the turn · Why?** each open a session with a span for that line (old spec §2, §5.3:
  `{unit, start, end, quote, unit_kind: 'utf16'}` against a real unit id, never a synthetic key) and present
  `/session/[bit]` at the medium detent with Claude's proposal as tracked changes, Keep / Skip per line (old spec S3).
  Offline, or with the writer offline (`lib/worker.tsx`), they grey with the one word "offline".
- Why not the system edit menu: RN 0.88 has no custom edit-menu item (CONFIRMED old spec §3.3, `RCTUITextView.mm:305-325`),
  and HIG/edit-menus says a custom copy of it would be redundant. Our actions are our own row.
- VoiceOver: the actions are `accessibilityActions` on the line too, so nothing depends on the second tap.

### 3.3 Focus mode (`app/joke/[id]/write.tsx`, full-screen modal)

| Element | Spec | Evidence |
|---|---|---|
| Route | `presentation: 'fullScreenModal'` with the `fullScreen` options (CONFIRMED `app/_layout.tsx:61`), header **Cancel · short name · Done**. Cancel with changes asks (action sheet: Discard · Keep writing). | HIG/modality (a distinct task, like Stage and Record, HIG.md §2.2); HIG/sheets "If you provide a Done button, always pair it with a Cancel button." |
| Entry | Write (the bottom toolbar), the second tap on a selected line (opens at that line), Bits "+" (a fresh bit opens here at once, HIG.md §3.13; the first line names it on Done, `nameFromFirstLine`, CONFIRMED `:108`). | |
| Lines | **One `TextInput` per line** at `stageProse` (CONFIRMED `ui.tsx:29,40`), each in a 44-pt row; `submitBehavior="submit"` (not the deprecated `blurOnSubmit`), `onSubmitEditing` inserts a line after and focuses it; `onKeyPress` Backspace on an empty line joins up (CONFIRMED in RN 0.88 source, §4 rows 5a-5c). The punch keeps its weight in focus mode. No idea line, no counter, no hint on screen; `accessibilityHint="Return starts a new line"` on the input (VoiceOver only). | HIG/text-fields "To let people input larger amounts of text, use a text view". |
| Done | `addVersion` when the joined text changed (CONFIRMED `lib/bitWork.ts:14`), plus the punch marks (`line_meta`) in the same write once `POST /api/bits/{id}/versions` exists (§6); until then two calls. Snack "Saved as v9 · Undo"; the Versions row reads "v9 · just now · Undo" (non-timed). | HIG.md §7 (a non-timed Undo path). |
| The bar above the keyboard | **Punch · Pile · N · hide keyboard** (`keyboard.chevron.compact.down`), three 44-pt items on **Liquid Glass**: a `GlassView glassEffectStyle="regular"` behind `isLiquidGlassAvailable()` (CONFIRMED installed, `expo-glass-effect` 58.0.5, `package.json:22`; §4 row 2), the `C.glass` token below iOS 26, `colorScheme` following `C.scheme`. **Not** `InputAccessoryView` (deprecated in RN 0.88, CONFIRMED `react-native/index.js:79-86`): a `View` at the bottom of the `KeyboardAvoidingView behavior="padding"` the shell already wraps screens in (CONFIRMED `lib/Screen.native.tsx:77`). Fixed space between the text item and the symbols. The Claude asks are not here (D7). | HIG/virtual-keyboards: "If other views in your app use Liquid Glass … apply Liquid Glass to the view that contains your controls"; "make sure they're relevant to the current task." DESIGN.md §6 amended for this one bar (§15). |
| Dictation (the rule) | The system's Dictation key appears under the keyboard on its own and is **never repeated** by us. Dictated text arrives through `onChangeText`; a spoken "new line" may arrive as `\n` **inside** the text, or UIKit may hand it to the same path a Return press takes (`onSubmitEditing`): the source cannot settle which (§4 row 6a), so **the input handles both the same way**: split at `\n` in `onChangeText`, and treat `onSubmitEditing` as a split at the caret. The split runs **only after the text has settled** (no change for ~700 ms, or blur, or Done): moving focus while dictation streams may end the session (UNKNOWN, device check 3). Paste splits at once. | HIG/virtual-keyboards: "the Emoji/Globe key and Dictation key automatically appear beneath the keyboard … it's likely to be confusing if you repeat them." |
| Fallback | If focus hand-off flashes the keyboard or dictation dies on the hand-off: **one multiline `TextInput`** for the whole bit, the caret line from `onSelectionChange` (CONFIRMED available, old spec §3.3), lines split on `\n` at Done; the punch loses its weight in focus mode (the dot alone marks it there; weight returns on the page). | §4, device check 3. |

### 3.4 The pile

**What the pile is.** Every scrap gathered for this bit but not placed in its text: typed jots and memos attached to
the bit (today's attach edge: `POST /api/merge/attach {note, into}`, CONFIRMED `lib/bitWork.ts:22`; a memo's words
reach the server as an idea and are attached the same way, CONFIRMED HANDOVER "Voice memos"), later set clips pulled
in from the deck (§3.9, Slice 5). Membership = the attach record; **placed** = a `placement` row for that attach
(§5). A doc stays a doc: doc paragraphs are versions (the stepper's second line names the source doc), never scraps.

| Element | Spec | Evidence |
|---|---|---|
| Inline (the row open) | The five newest scraps, then **"All 7 ›"** (pushes `/pile/[id]`, virtualized, no cap). A scrap row: a 44-pt leading **glyph** for the kind (jot · doc · clip; **for a memo the glyph is the 44-pt play button**, `playLocal`, CONFIRMED `lib/RecRows.tsx:12,45`), the words at `numberOfLines={2}`, a trailing 44-pt **+** (`accessibilityLabel="Add to the bit"`, `accessibilityHint="Puts it after the line you are on"`). A scrap Claude filed carries the tiny AI badge only; "Filed for you", date and source live in its detail. Filed scraps sit first. | Rule 6; HIG/lists-and-tables "Keep item text succinct … letting people choose an item to reveal its content in a detail view." |
| Add | **Straight into the bit** (D10): a new version with the scrap's text inserted after the selected line (or at the end), the `placement` written, the scrap leaves the pile, snack "Added · Undo" and "v9 · just now · Undo" in the Versions row. Haptic `success` after the write (`afterWrite`, CONFIRMED `lib/feedback.ts:12`). | HIG/undo-and-redo. |
| Tap the words | The scrap's detail, `/scrap/[id]`, a `fitToContents` form sheet: the full words, source · date (in words: "Memo · Oct 30 · 0:41", "Jot · Aug 3"), Play, **Add**, **Not this bit**, Undo (if Claude filed it). | HIG/sheets. |
| Long press | `RowMenu.native` (the system context menu, CONFIRMED `lib/RowMenu.native.tsx:14-20`): Open the source · Play · Add · Not this bit · Undo · **Not a joke** (destructive, last). Every item has a visible path too (the row's + and detail). | HIG/context-menus: "Always make context menu items available in the main interface, too." |
| Swipes | Leading = Add, trailing = Not this bit (labelled, `lib/SwipeRow.native.tsx`, Undo in the snack). Optional; VoiceOver `accessibilityActions` mirror them. | HIG/accessibility "Offer alternatives to gestures". |
| Not this bit | `POST /api/merge/undo {merge_id}` (the attach is a merge, CONFIRMED `bitWork.ts:24`) plus `merge/dismiss` for the pair; the scrap returns to Loose material with its next-best home (D11). | |
| Over focus mode (the sheet) | `app/pile/[id].tsx` with `bitnotes`'s options (CONFIRMED `_layout.tsx:63,166`: `formSheet`, `sheetAllowedDetents [0.5, 1]`, grabber, `sheetLargestUndimmedDetentIndex 0`) and `?after=<line>`. **Nonmodal for real**: no Close button, no footer sentence, the grabber and a swipe dismiss it, the keyboard may come back over it (INFERRED, §4 row 7b), the focus route's Cancel · Done stay the only bar. Add inserts after the line he was on and the caret returns there. The session sheet and the pile sheet are **never up together**: a line action on the page dismisses the pile sheet first; in focus mode there is no line action. | HIG/sheets "Use a nonmodal view when you want to present supplementary items that affect the main task"; HIG/modality "Let people dismiss a modal view before presenting another one." A form sheet over a full-screen modal is already Notes over Stage (CONFIRMED `RNSScreenStack.mm:552-580` chains modals; `_layout.tsx:161,166`). |
| The first-use tip (D2) | A custom inline card (no TipKit binding anywhere in the installed modules, CONFIRMED by grep, §4 row 8): "Scraps wait here until you add one to the bit." with a 44-pt ×. Shown once when `prefs.tips.pile` is unset and the count goes above 0; the row opens by itself that once. Set the pref on ×, the first Add or unmount; never again. | HIG/onboarding "display these instructions near that area"; HIG/offering-help "Keep your tips to one or two sentences." |
| Empty | "Pile · 0" opens on "Gather from your notes ›" (opens the session sheet in Gather, Slice 5; until then it opens Loose material filtered to this bit's candidates). | |

### 3.5 Versions: the stepper, reading an old version

| Element | Spec | Evidence |
|---|---|---|
| Collapsed | "Versions · v8 · Nov 2 · 1:40": the newest version's short date (`rules/format`) and the bit's run time (`duration_s` as `clockMs`; "~1:40" when `duration_estimated`, CONFIRMED `lib/api.ts:344`). Right after a write: "v9 · just now · **Undo**" (the `indexes` the write returned, kept until the page unmounts or the next write; `undo(indexes)` as the snack does, CONFIRMED `:92-93,115`). Offline: "v9 · waiting · Undo" (§8). VoiceOver: "Versions, 8, newest November 2, run time one forty." Hidden at one version. | Rule 11; HIG/writing "Check each word to be sure it needs to be there" ("run time" dropped). |
| Open | A **stepper row**: two 44-pt `IconBtn`s (`chevron.left` older, `chevron.right` newer; hold to repeat, haptic `selection` per step, none under Reduce Motion), the label "v5 of 8 · Oct 20 · Tighten", a second line with the source doc (`v.doc`, CONFIRMED `:235`) and that version's run time. `accessibilityRole="adjustable"` with `increment` / `decrement` (CONFIRMED typings, §4 row 10). An optional `PanGesture` across the label steps too. Under it: **"Restore v5 as a new version"** (`addVersion`). | Critique D: 8-pt ticks fail 44 pt; HIG/sliders "supplement a slider with … a stepper." |
| Reading an old version | Stepping sets `vi` (CONFIRMED `:62`) and the lines re-render from that version. A line the newest version cut is grey, struck, "−" in the gutter, its actions **Bring this line back · ⋯ (Why was it cut?)**; a line that comes in later is dashed with "v8" (both from the server's line diff, §5; until it lands, the older text renders with no marks). **Write and "Reorder lines" are off** while `vi > 0`, with one reason under Write: "Reading v5". | HIG/menus "Show people when a menu item is unavailable"; the one-word-reason rule. |

### 3.6 Sessions (push, `/joke/[id]/sessions`)

A plain list: mode · date, second line "v7 · 3 of 3 kept" or "Lesson · 4 of 5 right"; each row opens the session
sheet read-only (old spec S8). Count = `GET /api/jokes/{id}/sessions` (old spec §7.2; old bit chats listed as Talk).
In the session sheet, **"Save version" is not rendered until a change has been kept** (no greyed button with a
sentence). The sheet itself, its modes, tracked changes, Keep / Skip, the breakdown card and the score line are the
old spec's S2-S9, unchanged.

### 3.7 On tape (push, `/joke/[id]/tape`)

Today's Elsewhere block in full (CONFIRMED `:266-310`): the performed rows (`MomentRow`, each a deep link into the
set at that second), then "Sounds like it, on tape" with **link it** (a `performed_as` edge, undoable, `linkMoment`
`:137-146`), then "Sounds like these bits" with **merge it in** (`mergeLike`, the action sheet on the phone). No
map. The row's meta: nights = `performed.length`; "killed twice" = the count of `rating === 'killed'` among them
(the set rows carry `rating`, CONFIRMED `lib/api.ts:346,352`; **INFERRED that `/similar`'s moments do not carry it
today**, so the new `GET /api/bits/{id}/tape` route sends it, §6). Offline: rows whose audio is not in
`Documents/recordings` dim with "audio on the server". **Film it (2026-11-03, SPEC-2026-11-03-film-it.md §2.7, built
v1):** the row's meta gains "filmed N" (kept takes on this phone, `tapeMeta`'s fourth argument; "filmed 1" alone when
never performed) and the screen gains the **Filmed** rows under the performed ones ("Filmed · Nov 3 · 0:48" · "112
words"; play while the .mov is on the phone, Photos once the one copy lives there; Share captions…; Delete the words).

### 3.8 The More menu (in frequency order, the destructive item last)

**Reorder lines · Record a memo · Film it… (2026-11-03; phone only, greyed "nothing to read" on an empty bit) · — ·
Pick a set list… · Merge with… · Share… · — · Rename short name… · Stage ›
(Idea · To try · Testing · Working; the AI's guess carries the badge until he picks) · — · Retire.** Today's items
keep their handlers (CONFIRMED `:187-193`: `recordMemo`, `setRename`, `boards.pick`, `/merge`, `Share.share`,
`retire`). "Add to tonight's set" leaves the menu (it is the bar's To a set). "Reorder lines" is unavailable while an
old version is shown. **Reorder** is an edit mode on the page: lines become `DragList.native` rows (long-press-drag
the grip; ↑ ↓ as the VoiceOver fallback, CONFIRMED `lib/DragList.native.tsx:47-49`), Done writes a version, Undo in
the snack. HIG/menus: "Prefer listing important or frequently used menu items first"; HIG/context-menus: destructive
items "at the end of the menu".

### 3.9 The deck (`/loose`, Loose material)

One card, full width: eyebrow "Jot · Oct 31", the words at `stageProse`, Claude's reason under the AI badge ("Looks
like **Eleven years**: the dog as a partner."). **Two** buttons in the bottom toolbar: **Pull in to Eleven years**
(prominent) · **Not this bit** (plain). The bar: title "Loose material", a filter glyph (`line.3.horizontal.decrease`)
that is a pop-up over four kinds (All · Jots · Docs · Memos; labelled with the current choice for VoiceOver), More
with "Same idea" as a toggle (today's merge review, card for card, logic unchanged). "3 of 14" is the only count on
the screen: **as a two-line custom `headerTitle`**, because react-native-screens 4.30 has no header subtitle for
expo-router's stack (CONFIRMED absent, §4 row 3). Long press: Open · New bit · Ask · **Not a joke** (destructive,
last). The swipes stay (leading Pull in, trailing Not this bit, labelled). Pull in = `merge/attach` **into the pile**
(not `POST /api/merge`, which "Add as a new version" uses today, CONFIRMED `inbox.tsx:46`); Not this bit =
`merge/dismiss` for the pair; the piece stays loose with its next-best home offered next time. This is today's inbox
card made bigger (CONFIRMED `inbox.tsx:38-99`); `/inbox` and `/merge` keep their URLs; Tonight's "N to sort" routes
here. HIG/segmented-controls (about five on iPhone; a kind filter at AX sizes needs fewer) → the pop-up.

### 3.10 The capture proposal (on `/say`)

The jot path does not change until the save has happened: `BitEntry` shows "That's a bit, not a question. Saved as a
new bit. Undo · Open" exactly as today (CONFIRMED `say.tsx:128-146`). Then, and only if the answer exists, a second
bubble: the AI badge, "Looks like **Eleven years** (the dog as a partner). Put it in the pile?" with **Put it in the
pile** and **Not this**. The proposal is the inbox's existing batched call (`GET /api/merge/candidates?jokes=<id>`,
CONFIRMED `inbox.tsx:116-125`) for the one new id, asked after `state === 'done'`; a queued (offline) jot never shows
it until it is sent. A memo gets the same line on the record screen's saved state once its words are ready, and on
its deck card. Never before the save; nothing waits on Claude; no composer of our own anywhere (the system search
field, HIG.md §3.1/B7, the standing decision). HIG/generative-ai: "ask for confirmation before performing a
significant action on someone's behalf"; "Give them the ability to dismiss new content they don't want."

### 3.11 Bits list rows and Loose material (from option 3; Slice 5)

- **Bits rows** (`bits.tsx` `Row`, CONFIRMED `:58-70`): the sub line becomes what the bit is built from
  ("2 docs · 4 jots · 1 set clip") plus a trailing "3 to sort" badge when Claude found loose pieces for it; the status
  glyph and dot stay (never colour alone). `ROW_H` 64 → 72 scaled by `PixelRatio.getFontScale()`; at ≥ 1.3× the badge
  stacks under the sub (`layoutRules.ts` `STACK_AT`). Needs `built_from` and `pile` on the list row (§5).
- **Loose material** pinned above the list as a card: "Loose material · 14 ready · Claude has a home for 14 · Sort
  them ›" (the actionable count leads, not the total). The **Memos** segment on Bits goes: memos are loose material
  until filed, then scraps in a pile.
- **Desk / Notebook** retire as screens (`desk.tsx`, `notebook.tsx` become `<Redirect href="/bits" />`); the Filter
  sheet's facets move into Bits' sort menu as "Filter…"; Shows already live under Sets › Recorded. `notes.tsx`
  redirects to `/loose?view=memos`.
- **The map** stays; hollow dots for loose material near the bit they look like is an open question (§13).

### 3.12 Desktop (web, ≥ 1280): focus with edge handles

`lib/Screen.tsx` wide layout (CONFIRMED the 900 breakpoint and sidebar, `Screen.tsx:1,65,106`): three panes by default
(list 300 · the bit, max 760 · session 400, old spec §3.2) until he chooses **focus** once; then `prefs.focus` keeps
the last choice. Collapsed panes become **28-pt edge handles** with their labels ("Bits · 1,576", "Claude · 3"); the
focus toggle in the page header is lit; Escape and the toggle restore. The rows, the stepper and the pile are the
same components; a scrap can be dragged onto a line (a drop gap, Undo after). HIG/toolbars: "offer ways to reliably
restore hidden interface elements"; HIG/sidebars: not hidden by default.

### 3.13 The other states

| State | What shows |
|---|---|
| A brand-new empty bit (after backing out of focus mode without writing) | Bar "New bit" (the sparkle and To a set off), the idea placeholder, one status line "Nothing written yet." (a status, not an instruction; DESIGN.md §6 row 12: one sentence + one action), the Pile row open on "Gather from your notes ›", Write. Versions, Sessions, On tape and the Claude line hidden. |
| Offline backstage (frame f) | The footnote line above Write; the sparkle and the two Claude line actions grey "offline"; the Claude line hidden. Working: reading, the stepper (cached), Punch and Write (the outbox), memo play (local files), To a set (queued, the snack says so), On tape (server-only audio dims inside). |
| Dynamic Type at the app's caps | Everything wraps, nothing truncates; the rows re-stack (label above, meta below); the punch dot grows with the line; the order never changes (HIG/typography). The bar does not scale (the system's). |
| Reading an old version | §3.5. |
| The writer paused for the day | The Claude line hidden; line actions grey with "paused"; the old spec's budget copy in the session sheet. |

### 3.14 HIG pages relied on (read live on developer.apple.com 2026-11-03 for the design pages; carried over)

| Page | Rule used here |
|---|---|
| HIG/toolbars | "Prioritize only the most important items … Create a More menu." "Use the .prominent style for key actions … Only specify one primary action, and put it on the trailing side." "Prefer simple, recognizable symbols." "Borders (like outlined circle symbols) aren't necessary." "Keep actions with text labels separate … insert fixed space." "Consider temporarily hiding toolbars … offer ways to reliably restore hidden interface elements." → three symbols in the bar, Write alone, `ellipsis`, the keyboard bar's spacing, the desktop handles. (`/navigation-bars` now serves this page.) |
| HIG/buttons, HIG/sliders | 44 × 44 pt; "Keep the number of prominent buttons to one or two per view." "Consider supplementing a slider with … a stepper." → every target, one prominent Write, the stepper. |
| HIG/lists-and-tables, HIG/disclosure-controls | The disclosure indicator rule; "Keep item text succinct … reveal its content in a detail view." "Use a disclosure control to hide details until they're relevant." "People must enter an edit mode before they can select table items." → push vs inline rows, scrap rows, Reorder as a mode. |
| HIG/typography | "Make sure your app's layout adapts to all font sizes." "Keep text truncation to a minimum." "a stacked layout where text appears above secondary items." "Maintain a consistent information hierarchy." "Increase the size of meaningful interface icons as font size increases." → §3.13. |
| HIG/sheets, HIG/modality | Nonmodal sheets, the grabber, swipe to dismiss, one sheet at a time, Done pairs with Cancel, dismiss one modal before another. → the pile sheet, focus mode's bar. |
| HIG/onboarding, HIG/offering-help | Context-specific tips near the area, one or two sentences, never shown again. → the one tip. |
| HIG/loading, HIG/feedback | "Show something as soon as possible." "status feedback … near the items it describes." "Show people when a command can't be carried out and help them understand why." → skeletons, the offline line by Write, the one-word reasons. |
| HIG/virtual-keyboards | No repeated Dictation key; relevant custom controls; Liquid Glass on the bar; the keyboard layout guide. → §3.3. |
| HIG/menus, HIG/context-menus, HIG/segmented-controls | Frequent items first; show unavailable items; destructive last; context-menu items also in the main interface; about five segments on iPhone. → the More menu, the long presses, the deck's pop-up. |
| HIG/tab-bars, HIG/accessibility | "Use a tab bar to support navigation, not to provide actions." "Convey information with more than color alone." Reduce Motion. → no docked composer; the punch mark; no row animation under Reduce Motion. |
| HIG/text-fields, HIG/edit-menus, HIG/writing, HIG/generative-ai, HIG/undo-and-redo, HIG/drag-and-drop | Text views for larger input; the system edit menu untouched; "If you can use fewer words, do so."; confirmation before acting on his behalf, dismiss and revert; Undo on every write and every drop; alternatives to drag. |

---

## 4. Native feasibility (ios-dev-expert, read against `app/node_modules` 2026-11-03)

| # | Claim | Evidence | Verdict | Fallback |
|---|---|---|---|---|
| 1 | `Stack.Toolbar placement="bottom"` (default), `Spacer`, `Button variant="prominent"`, `disabled` | `expo-router/build/layouts/stack-utils/toolbar/context.d.ts:2` (`'left' \| 'right' \| 'bottom'`); `StackToolbarClient.d.ts:23` (`@default 'bottom'`), `.js:58`; `shared.d.ts:40` (variants), `:13` (disabled); `StackToolbarClient.js:155` + `StackToolbarSpacer/index.d.ts:7-10` (flexible in bottom placement); native `expo-router/ios/Toolbar/RouterToolbarItemView.swift:32,172` | **CONFIRMED** | — |
| 1e | How it is drawn, and a floor | `RouterToolbarHostView.swift:115-119`: the enclosing `RNSScreen`'s `UINavigationController` toolbar (`setToolbarItems`, `setToolbarHidden(false)`); **guarded by `#available(iOS 18.0, *)` at `:62`**; page components only (`StackToolbarClient.d.ts:84-85`) | **CONFIRMED**: iOS 18 is the floor | Gate on `Platform.Version >= 18`, else the in-page bar |
| 1f | Trap: never re-hidden | `RouterToolbarHostView.swift:170-178` only clears the items on unmount; the only `setToolbarHidden` call is the `false` at `:116`, and `toolbarHidden` is navigation-controller-wide, so an empty toolbar may linger on Bits after a pop | **INFERRED** | Device check 1; if it lingers, the in-page glass bar |
| 1g | A bottom toolbar with `NativeTabs minimizeBehavior="onScrollDown"` (`TabsShell.native.tsx:15`) | react-native-screens 4.30.1 has no code relating `UIToolbar` to the tab bar; `tabBarMinimizeBehavior` is a pass-through (`ios/conversion/RNSConversions-Tabs.mm:144-157`); Apple's material covers only the tab accessory | **UNKNOWN** | Device check 1 → the in-page `GlassView` bar |
| 2 | `GlassView glassEffectStyle="regular"`, `isLiquidGlassAvailable()`, `colorScheme`, a plain container | `expo-glass-effect/build/GlassView.types.d.ts:3,27,42,44`; `build/index.d.ts:5`; `ios/GlassView.swift:287-291,330-331,341-342` | **CONFIRMED** | A plain `View` with `C.glass` when unavailable |
| 3 | A header subtitle | None for expo-router's stack: `react-native-screens/src/types.tsx` `subtitle` hits are menu items only (`:1294,1338`); nothing in `ScreenStackHeaderConfigNativeComponent.ts` or `RNSScreenStackHeaderConfig.mm`; `subtitle` exists only in the experimental gamma stack | **CONFIRMED absent** | A two-line custom `headerTitle` (§3.9) |
| 4 | `InputAccessoryView` deprecated; a `View` under `KeyboardAvoidingView behavior="padding"` is right | `react-native/index.js:79-86` (`warnOnce` deprecation); KAV math `Libraries/Components/Keyboard/KeyboardAvoidingView.js:89-119,134-138,211-214` pads from its own frame to the keyboard's `screenY`, which `Screen.native.tsx:79-82` satisfies; no keyboard-layout-guide option in RNS (`RNSScreen.mm:1174` TODO) or expo-router; `react-native-keyboard-controller` **not installed** | **INFERRED** (works), device check 2 | `keyboardVerticalOffset` once; `behavior="position"` if padding jitters; keyboard-controller only as a new native dependency |
| 5a | `submitBehavior` is current, `blurOnSubmit` deprecated | `types_generated/…/TextInput.flow.d.ts:692-697,706-716,113`; `Libraries/Components/TextInput/TextInput.js:449-470` | **CONFIRMED** | Write `submitBehavior="submit"` |
| 5b | `onSubmitEditing` with `multiline` | The typing says invalid (`TextInput.flow.d.ts:607-610`) **but the iOS code honours it**: `RCTBackedTextInputDelegateAdapter.mm:283-294` intercepts a `"\n"` replacement and emits `onSubmitEditing` for `submit` / `blurAndSubmit` (`RCTTextInputComponentView.mm:429-443`), no newline inserted | **CONFIRMED (code)** | Rely on the code, not the comment; pin it in the device check |
| 5c | `onKeyPress` delivers Backspace, even in an empty field | `RCTTextInputComponentView.mm:455-466`; `RCTBackedTextInputDelegateAdapter.mm:148-153`; `TextInputEventEmitter.cpp:119-123` (`""` → `"Backspace"`) | **CONFIRMED** | — |
| 5d | `.focus()` on the next input inside `onSubmitEditing` keeps the keyboard up, no flash | With `submit` the field is not blurred (`RCTTextInputComponentView.mm:445-448`, the adapter never calls `endEditing`); whether the return-key row re-animates is not in the source | **INFERRED** keep-up / **UNKNOWN** flash | One multiline input, caret from `onSelectionChange` |
| 6a | Dictation: `\n` inside `onChangeText`, never `onSubmitEditing` | RN has no dictation path of its own (`RCTUITextView.mm:329-338`, `RCTTextInputComponentView.mm:875-898` only guard the session); if UIKit inserts a spoken "new line" as a lone `"\n"` replacement, `RCTBackedTextInputDelegateAdapter.mm:285` treats it like Return → `onSubmitEditing` | **UNKNOWN** | **Handle both the same way** (§3.3) |
| 6b | Moving focus mid-dictation ends the session | Not in RN source; UIKit behaviour | **UNKNOWN** | Only advance focus on an explicit submit / a settled `\n` |
| 6c | `onChangeText` batching | None: every `textViewDidChange` emits (`RCTTextInputComponentView.mm:496-511`); the dictation placeholder phase is marked text, events not suppressed | **CONFIRMED** (no batching) | The ~700 ms settle in JS |
| 7a | The undimmed detent keeps the parent interactive | `RNSScreen.mm:1043-1066,894-901,987-999`; index 0 with `[0.5, 1]` → `largestUndimmedDetentIdentifier` (`:1063`) | **CONFIRMED** | — |
| 7b | The keyboard over a medium sheet keeps the detent | No keyboard code in RNS (`:1174`); UIKit raises the sheet and keeps the detent | **INFERRED** | `sheetInitialDetentIndex: 1` for the pile sheet |
| 7c | A form sheet over a full-screen modal | `RNSScreenStack.mm:552-580` chains presentations; Notes over Stage already does it (`_layout.tsx:161,166`) | **CONFIRMED** | — |
| 8 | No TipKit binding | `grep -rl TipKit` over `node_modules`: zero; `@expo/ui` has only an Android Compose `Tooltip` | **CONFIRMED absent** | The custom tip card |
| 9 | The symbol names exist | `sf-symbols-typescript@2.2.0 dist/index.d.ts`: `sparkles`, `text.badge.plus`, `ellipsis`, `keyboard.chevron.compact.down`, `line.3.horizontal.decrease`, `chevron.left`, `chevron.right` each present; expo-router's `icon` is that `SFSymbol` type (`shared.d.ts:3,15`) | **CONFIRMED** | — |
| 10 | `accessibilityRole="adjustable"` + `increment` / `decrement` | `types_generated/Libraries/Components/View/ViewAccessibility.d.ts:14,22-26`; `ViewAccessibility.js:26` | **CONFIRMED** | — |

**No new native module for any slice** (`expo-glass-effect`, `@expo/ui`, `expo-symbols`, `react-native-screens` 4.30
are in `package.json:8,22,33,44`); the build 20 binary carries everything. The bottom toolbar's iOS 18 floor is
below the owner's phone (iOS 26) and is gated in code anyway.

### 4.1 The three device checks (Slice 1 ships them as a spike screen)

| Check | What the owner does | Pass | Fail | What it decides |
|---|---|---|---|---|
| **1. Bottom toolbar vs the minimising tab bar** | Open a bit, scroll down (the tab bar minimises), scroll up; tap Write; go Back to Bits. | Write floats above the tab bar in both states and stays tappable; **no empty toolbar strip remains on Bits** after Back. | Write overlaps or clips the minimised bar, moves under it, or an empty bar lingers on Bits (row 1f). | Pass → `Stack.Toolbar placement="bottom"` is the page's bottom. Fail → a 50-pt in-page `GlassView` bar above the tab bar in the content layer (no native toolbar), and the design frames are redrawn to match. `minimizeBehavior="never"` on the Bits tab only with the owner's say. |
| **2. The glass bar above the keyboard** | In the spike's focus mode, tap a line (keyboard up), move through lines, hide the keyboard, bring it back. | A 56-pt glass bar sits flush on the keyboard's top edge with no gap or overlap, moves with the keyboard frame, returns to the tab-bar edge on dismiss; it blurs what scrolls under it. | A gap or an overlap of about the tab-bar height, or jitter while the keyboard animates. | Pass → `KeyboardAvoidingView` + `GlassView` as drawn. Fail → `keyboardVerticalOffset` tuned once; `behavior="position"` if padding jitters; `react-native-keyboard-controller` only as a last resort (a new native dependency, a new binary). |
| **3. Dictation into per-line inputs** | In the spike's focus mode: press the keyboard's mic, say "line one, new line, line two", stop; then start dictating and tap another line mid-sentence. | The words land in the focused line; "new line" arrives as `\n` in `onChangeText` **or** as `onSubmitEditing` and both become a new line after the settle; Return between typed lines moves the caret without a keyboard flash. | "new line" is swallowed; the dictation session dies on the hand-off; text lands in the wrong line; the keyboard flashes on Return. | Pass → one `TextInput` per line (§3.3). Flash only → keep per-line inputs, accept it or drop `autoFocus` on the inserted line. Dictation dies → advance focus only on an explicit submit or a settled `\n` (the rule already); if it still dies → the one-multiline-input fallback (§3.3), punch weight lost in focus mode. |

---

## 5. Data model deltas (SurrealDB, hosted; everything from the old spec §5 stands)

New tables go in a new file, `db/schema_bit.surql` (SCHEMAFULL, `IF NOT EXISTS` on tables; applied through the
ADR 0009 path with his go). They are **pk-api's** from day one (written by pk-api only, so nothing here waits for
T10), which means the old spec's prerequisite applies: **per-role database logins before pk-api writes any table**
(old spec §5.4, `FORK.md:90,97`).

| Delta | Shape | Why here, not elsewhere |
|---|---|---|
| **`line_meta`** (the punch and line metadata on a version row) | `unit: record<unit>` (the version), `punch: array<int>` (0-based line indexes; several allowed, D13), `by`, `at`; one row per version, upserted. | A version is one text unit today (CONFIRMED `versionsOf`, `joke/[id].tsx:39-50`; `unit` fields `db/schema.surql:55-78`). `unit.bit` is a FLEXIBLE object with no reader today (CONFIRMED by grep), but the decision builder's editable fields are not proven to accept it and the version write stays Python's until T10 (§6); a pk-api-owned row avoids both. Lines are derived by splitting `unit.text` on `\n`; tags = the lines after the last punch. |
| **`placement`** (a scrap placed into the bit) | `attach: string` (the merge store's attach id, `at:…`), `bit: record<joke>`, `unit: record<unit>` (the version the Add wrote), `line: int`, `by`, `at`. | Membership of the pile is the creative store's attach record (CONFIRMED `pk-creative/src/reducers/merges.rs:74-100`: `{id, kind:'attach', status:'attached', joke, into}`), which pk-api already reads for the Bits list (T9 round 5). **The pile read** = attach records with `into = bit` and `status = attached` **minus** those with a `placement` row, joined with the note's words (its idea or first unit) and kind (jot · memo · clip from how the idea arrived). Undo of an Add = `undo(indexes)` for the version plus deleting the placement. |
| **`write_receipt`** (idempotency) | `key: string` (the client's `X-Idempotency-Key`), `route`, `bit`, `status: int`, `body: object` (the exact response), `at`; kept 7 days. | The outbox replays writes (§8); a repeated key returns the stored response, so the Undo `indexes` are the same and nothing lands twice. pk-api checks the receipt **before** proxying a Python write (the version write until T10), so the Python needs nothing. |
| **`summary_confirmed`** (derived, no table) | pk-api's `/api/jokes/{id}` (answered since T9 round 4) adds `ai.summary_ai` and `ai.summary_confirmed`, computed like `stage_confirmed` (CONFIRMED `api-dev/server.py:433-440`: from the `_ai` overlay and the touch log: `summary` in the fields he edited → confirmed). | A read-side flag; the write is today's `decide node edit {summary}`. Until the Python twin sends the same keys the differential allow-lists them (ADR 0007). |
| **The line diff** (`gone` / `later`) | `GET /api/bits/{id}/versions/{unit}/diff` → `{against: newest, gone: [i], later: [i], moved: [[from,to]], changed: [[old,new]]}` from the `similar` crate on the server (old spec phase 4), with a small-edit-distance rule so a line he changed by a word reads as "changed", not "one gone, one new". | No diff library in the app (CONFIRMED old spec §3.3). |
| **On tape counts** | `GET /api/bits/{id}/tape` → the performed moments **with `rating`** (from the set rows, `lib/api.ts:346,352`), `nights`, `killed` (= `rating === 'killed'`), plus `like` as `/similar` has it. | `/similar` is proven equal to the Python (T9 round 6); a new key there would need the allow-list, a new route needs nothing. |
| **The session count, the thoughts count** | `GET /api/jokes/{id}/sessions` (old spec §7.2) and `GET /api/bits/{id}/thoughts` → `{count, items: [{kind: 'same_bit' \| 'tag_material' \| 'clip', …}]}` = merge candidates for this bit + pile scraps the judge tagged as tag material + unlinked clips from `/similar`. | All existing reads composed once on the server, so the page makes one call for the row. |
| **List-row fields** (Slice 5) | `built_from: {docs, jots, clips, memos}` and `pile: int` on `JokeSummary` (CONFIRMED the type, `lib/api.ts:337-345`). | pk-api answers `/api/jokes` (T9 round 5); new keys go on both sides (the lane L pattern) or wait for T10. |
| **Outbox record** (the phone, not the database) | §8. | |

What does **not** change: `unit`, `joke`, `decision`, the attach record's shape, the versions route's contract
(`{text, reason?, source?, draft?}` → `{ok, unit, joke, indexes}`, CONFIRMED `server.py:1658-1700`).

---

## 6. API routes

Rule (old spec §7.1, CONFIRMED `rust/crates/pk-api/src/lib.rs:47` `OWN_PATHS`; `RUST_WORKSTREAM.md` T9/T10): **new
routes are pk-api's** (Rust), new paths with no Python twin, added to `OWN_PATHS` and proven by their own tests on a
scratch SurrealDB 3.3.4; everything pk-api does not own is forwarded to the Python unchanged. pk-api writes **are**
T10 ("all at once", open), so until T10: the tables in §5 are pk-api's own from day one; **the version write and the
merge writes stay Python's**, called by pk-api through its proxy with his token where a pk-api route needs them.
Every route is owner-only (the actor rule). The deploy prerequisite stands: **the Python with `GET /api/ai/worker`
first, then pk-api's Ingress** (T9 round 7).

### 6.1 New, in pk-api

| Route | Does | Idempotent |
|---|---|---|
| `GET /api/bits/{id}/pile?limit=5` | The pile read (§5): scraps newest first, `filed_by_ai`, `kind`, `words`, `source`, `date`, `memo` (the local recording id when the phone has it), `total`. | — |
| `GET /api/bits/{id}/tape` | §5: performed rows with `rating`, `nights`, `killed`, the like lists. | — |
| `GET /api/bits/{id}/thoughts` | §5: the count and the items behind "Claude has N thoughts". | — |
| `GET /api/bits/{id}/versions/{unit}/diff` | §5: the line diff against the newest version. | — |
| `GET /api/jokes/{id}/sessions` and the session routes | Old spec §7.2, unchanged. Line actions = `POST /api/sessions {bit, mode, base_unit, span}`. | key |
| `POST /api/bits/{id}/versions` `{text, punch?, source, reason?, base_unit?}` | **One write for Done, Add, Cut, Reorder, Restore**: checks the receipt; proxies `POST /api/jokes/{id}/versions` to the Python (until T10; `source` must match `^[a-z_]{1,24}$`, CONFIRMED `server.py:1676`), then upserts `line_meta` for the new unit (`punch` carried over from the base version when omitted, indexes shifted by the edit), writes the receipt. Returns `{ok, unit, joke, indexes, newer_than_base: bool}` (the conflict flag, §8). | **`X-Idempotency-Key` required** |
| `PUT /api/bits/{id}/lines/{unit}` `{punch: [int]}` | The punch mark on an existing version (no new version). Returns the previous value for Undo. | key |
| `POST /api/bits/{id}/pile/{attach}/place` `{line, text}` | Add from the pile: one call that writes the version (as above, the scrap's words inserted after `line`) and the `placement`. Undo = `POST …/unplace` + `undo(indexes)`. | key |
| `POST /api/bits/{id}/pile/{attach}/unplace` | Removes the placement (the scrap is back in the pile; the version stays unless undone). | key |
| `GET /api/bits/{id}/gather` (Slice 5) | Loose pieces that look like this bit: a fused search by the idea line and the lines (`/api/search` logic, in pk-api since T9 round 6) over unattached ideas, memos and taped moments, with the judge's reason where a merge candidate already exists. | — |
| `POST /api/worker/pile/claim`, `POST /api/worker/pile/{job}/result` (Slice 5) | The overnight filing job, worker-only (the `pk-worker` client-credentials role). | key |

### 6.2 Existing Python routes reused until T10 (forwarded by pk-api; the app may call them directly before pk-api is deployed)

| Route | Used for | Note |
|---|---|---|
| `POST /api/jokes/{id}/versions` | Done, Add, Cut, Reorder, Restore (Slices 1-2, direct; from Slice 3 behind `POST /api/bits/{id}/versions`) | **No idempotency on the Python side** (CONFIRMED `server.py:1658-1700`): the outbox dedupes by content until the receipt route lands (§8). |
| `POST /api/decide` (`decide`, `undo`) | The idea line, the stage, rename, retire; every Undo by `indexes` | CONFIRMED `lib/api.ts` `decide` / `undo`. |
| `POST /api/merge/attach`, `/undo`, `/dismiss` | Pull in, Not this bit, Not this | CONFIRMED `api-dev/merge_api.py:8-10,481-485`. |
| `GET /api/merge/candidates?jokes=` / `?joke=` | The capture proposal, the deck's reasons, the thoughts count | CONFIRMED `merge_api.py:3-4,54,445`. |
| `GET /api/jokes/{id}`, `GET /api/jokes/{id}/similar` | The page's two reads (pk-api answers both since T9) | The map read (`/api/jokes/{id}/map`) goes. |
| `PATCH /api/boards/{id}` (`boards.add`) | To a set | CONFIRMED `lib/boards.tsx:92-101`; the add reports `already` (INFERRED idempotent on membership). |
| `POST /api/ideas`, `POST /api/say` | Memos and jots as today | Unchanged. |

---

## 7. Worker jobs (pk-worker only; every new kind waits for the T8 swap, ADR 0011)

| Job | Kind / task | What it does | Model, cost (§9) |
|---|---|---|---|
| **Capture proposal** | none new: the existing `merge` judge behind `GET /api/merge/candidates` | After a jot or memo is saved, the inbox's batched call for that one id returns the judge's best bit and reason; the app shows the bubble (§3.10). Offline, the proposal comes with the sync. | Haiku, ≈ $0.0002 per pair |
| **Overnight filing** (Slice 5, D14) | `pile_file` (new; `[tasks.pile_file]` in `engine/config.hosted.toml`, the T5 router) | A nightly sweep over unattached ideas and memos: for each, the top candidates by vector (`pk-embed`), then the judge; a match at or above the threshold (`confidence ≥ 0.85`, tuned on the undo rate, §10) is attached **into the pile** with `by: 'ai'` and the reason; shown with the badge and a non-timed Undo; counted out of Tonight's "N to sort". Performed bits are never filed into automatically unless the owner says otherwise (§13). Today's `memolink` kind (CONFIRMED T8 row) already does this for memos; `pile_file` generalises it to jots on a schedule. | Haiku, ≈ $0.0002 per candidate pair; a few cents on the first night, well under a cent a night after |
| **Line actions** | `session_tighten`, `session_tag`, `session_turn`, `session_talk` with a **span** (old spec §7.3) | The session's first turn carries the line's span; the prompt sends the bit with numbered lines and names the line; the result is hunks for that line only (Tighten, Tag after), 3-5 reveal words (Find the turn), or an answer (Why? = `session_talk` with the fixed question "Why does this line work?"). pk-api validates as the old spec says (verbatim, hunks equal the base, rule checks, legal rules). | Opus (Sonnet for Why?), ≈ $0.03-0.07 |
| **Break it down, Gather, Talk** | `session_breakdown`, `session_talk` (old spec); Gather is a **read** (§6.1), no kind | The whole-bit jobs behind the sparkle and the Claude line. | Opus ≈ $0.06; Gather $0 (plus the judge's reasons where asked) |

---

## 8. The offline outbox (the standing rule applied to the bit's writes; built in Slice 2)

Today Done, Add and the line actions all go over the network and fail with a snack (CONFIRMED `joke/[id].tsx:116-117,
94-96`); the offline rule covers capture only (CONFIRMED `lib/say.ts:6-8`, `flushOutbox` `:85-100`); the memo queue
is file-backed with a backoff (CONFIRMED `lib/recordings.native.ts:1-11`, `rules/memoRules.ts`, `rules/uploadsCore`).
**The bit outbox is modelled on both**: a local record first, shown as the current state, sent when there is signal.

### 8.1 The record

```
type BitWrite = {
  key: string;                 // uuid v4 from expo-crypto; the X-Idempotency-Key
  bit: string;
  kind: 'version' | 'punch' | 'place' | 'unplace' | 'to_set' | 'attach' | 'dismiss';
  body: object;                // the exact request body, as it will be sent
  route: string;               // the path, so the replay is dumb
  base_unit: string | null;    // the newest version id when he wrote (the conflict rule)
  local_unit: string | null;   // 'local:<key>' shown as v9 until the server's id comes back
  at: string;                  // ISO
  state: 'waiting' | 'sending' | 'failed';
  attempts: number; next_at: number; error?: string;
};
```

Storage: the phone, `Documents/outbox/bit.json` (expo-file-system legacy, the `recordings` pattern: writes run one
after another so an older snapshot never lands after a newer one); the web, `localStorage` (the `say.ts` pattern).
Pure rules in `rules/bitOutbox.ts` (tested): which item is next, the backoff (`rules/uploadsCore` `isPermanent` and
the schedule), the patch to apply to the page's cached read, the conflict decision.

### 8.2 The queue and the replay

- **Local first.** Every write goes to the outbox first, then is sent at once if online. The page's cached read
  (`cacheRead` / `cacheWrite`, CONFIRMED `lib/api.ts:130-153`; in memory only on native, so a cold start offline shows
  the outbox's items on top of the last cached page) is **patched** so the new line, the punch or the Add shows at
  once: a waiting version renders as v9 with the meta "v9 · waiting · Undo".
- **Order.** Oldest first, **per bit in order** (a punch on `local:<key>` must follow its version); a failed item blocks
  only its own bit.
- **When it runs.** On launch, on `online` (the shared `useOnline`), on return to the foreground, after each save.
- **Idempotency.** `X-Idempotency-Key: key` on every send. pk-api's `write_receipt` returns the stored response for a
  repeated key (§5). **Until `POST /api/bits/{id}/versions` exists** (Slice 3) the app sends the Python versions
  route directly, which has no receipt: before each replay the app refetches `/api/jokes/{id}` and **dedupes by
  content** (a newest version whose text equals the waiting item's text and whose date is not before `at` means it
  landed; the item is marked sent with that unit as its id). A `to_set` replay checks `already`.
- **Permanent failures.** A 4xx that is not 401/408/429 (the legal rules, 4,000 characters, "no such joke") marks the
  item `failed` with the server's words in the row's meta ("v9 · couldn't save: …" with Undo); it never blocks the
  bit's later items. A 401 goes through `handle401` and waits.
- **`local_unit` resolution.** When the server's `unit` id comes back, every later item in the queue that named
  `local:<key>` (a punch, a place) is rewritten to the real id before it is sent.

### 8.3 The conflict rule (the server has a newer version than `base_unit`)

Versions are append-only, so **nothing is ever overwritten or dropped**: the waiting version still lands as the next
version. The response's `newer_than_base: true` (or, before Slice 3, the refetch showing a newer unit than
`base_unit`) makes the app say so once, with a way to look: snack "Saved as v10 · v9 was written elsewhere while you
were offline · **Compare**", where Compare opens the Versions row stepped to v9. A `punch` against a version that is
no longer the newest still applies to that version (the mark belongs to the row). A `place` names its unit, so it is
valid whatever came after. A `to_set` whose list is no longer live is dropped with "That list was archived · Pick a
set list" (the `boards.add` rule, CONFIRMED `boards.tsx:92-101`: no list is ever chosen for him).

### 8.4 Undo on a waiting item

Drops it from the queue and reverts the local patch; nothing was sent, so there is nothing to undo on the server. The
row's meta carries it ("v9 · waiting · Undo", non-timed, HIG.md §7); the snack's Undo is the timed one. Once sent,
Undo is the server's (`undo(indexes)`, the receipt's response).

### 8.5 What is not queued

Line actions (Tighten, Tag after, Find the turn, Why?) and the sparkle: they need an answer, so they grey with the
one word "offline" (or "paused"). The session sheet's marks and Keep / Skip queue by the old spec's rule (§3.3 there).

---

## 9. Cost per call (prices CONFIRMED `rust/crates/pk-llm/src/budget.rs:43-45`; the token counts are INFERRED until Phase 0 measures them)

Per million tokens, input / output / cache write / cache read: Opus 5.5 4.00 / 20.00 / 5.00 / 0.20; Sonnet 5.5
2.00 / 10.00 / 2.50 / 0.20; Haiku 5.5 0.10 / 0.50 / 0.125 / 0.01. The daily cap is $2.00 (CONFIRMED `engine/budget.py:21`).

| Call | Model | Per call (warm cache) | Note |
|---|---|---|---|
| Every write on the page (version, punch, place, reorder, To a set, the idea line) | none | **$0** | No model touches his words on a write. |
| The capture proposal (one jot or memo) | Haiku judge, ≈ 1.2k in / 80 out | ≈ **$0.0002** per candidate pair, ≈ $0.001 per jot (top 3-5) | The inbox's existing call. |
| Overnight filing | Haiku judge | ≈ $0.0002 per pair; first night over 212 loose jots × 3 ≈ **$0.13**; after that only new pieces, < $0.01 a night | Vector pre-filter is local (`pk-embed`). |
| A line action: Tighten / Tag after | Opus; cached system ≈ 6k read, uncached ≈ 1.2k (the bit, the line, 3 lessons), output ≈ 1.2k with thinking | ≈ **$0.03-0.04** | Cheaper than a whole-bit Tighten (≈ $0.06-0.08, old spec §8): the line, not the bit, is the target. |
| Find the turn (one line or the setup) | Opus | ≈ $0.06-0.07 | As the old spec. |
| Why? | Sonnet (`session_talk`) | ≈ $0.01-0.02 | |
| Break it down, Talk, Gather | Opus / Sonnet / none | ≈ $0.06 / ≈ $0.02-0.04 / $0 | Old spec §8. |
| First call in a cache window | + ≈ $0.03 (Opus cache write) | | |

A backstage evening of twenty line actions is under a dollar; the overnight sweep is noise. The held-out run (old
spec Q3) is still the one thing that exceeds a day's cap.

---

## 10. How we know it is learning (old spec §9 carried over, plus the page's own signals)

The old spec's measures stand: parts right before correction, the held-out set, the masked turn, the ablation arms,
the preference share, rule hits, and the pre-registered gates. The page adds three cheap signals, all from rows that
already exist:

| Signal | How | Shown |
|---|---|---|
| **Line-action keep share** | Among Tighten / Tag after hunks shown, the share kept (per mode, rolling 50). | Settings › What I've learned |
| **Filing precision** | 1 − (Undo or Not this bit within 7 days) / (scraps Claude filed). The `pile_file` threshold is tuned on it; below 0.8 the sweep pauses and says so. | The same card; "Claude filed 12 this week, you kept 11" |
| **Proposal acceptance** | Put it in the pile vs Not this on the capture bubble, and the share ignored (the piece waited in Loose material). | The same card |

---

## 11. Privacy (old spec §10 carried over)

His material never leaves his account; every new table is in his database and every route is owner-only; the worker
sees one bit plus its retrieved lessons over the API (ADR 0011); nothing trains anyone else's model (verify
Anthropic's commercial terms before the first external tester); voice stays on the phone, only the words are sent;
the legal rules run on every reason, fix, Talk turn and scrap; the public repo holds invented jokes only and the
commit guard runs; App Store 5.1.2(i) consent is needed before any external tester. New here: the outbox file on
the phone holds his words until sent (it is in the app's Documents, backed up with the phone, like the recordings
index); the device prefs (`tips`, `focus`) hold no material.

---

## 12. Trade-offs accepted (from the design page, for the record)

1. Every section is one tap further away; the counts in the rows are what make them worth opening, and a row with
   nothing in it hides. 2. To a set in the bar is two-handed (D1). 3. Four line actions, none on the keyboard bar
   (D7). 4. A stepper is slower than a strip for forty versions; hold-to-repeat and the optional drag mitigate.
   5. The idea line loses its label ("One idea:"); VoiceOver still says "Idea". 6. Dictation depends on a ~700 ms
   settle. 7. The outbox is new code the bit's routes never had (D4). 8. One tip is one sentence (D2).

---

## 13. Open questions (genuinely open; each with its context)

1. **The bottom toolbar vs the tab bar** is a device check (§4.1), not a question; but if it fails, does the owner
   accept `minimizeBehavior="never"` on the Bits tab (the tab bar stops shrinking on scroll there) rather than an
   in-page bar? Lead's default: the in-page glass bar, the tab bar keeps minimising.
2. **Which bits may Claude file into overnight?** D14 allows any bit's pile. Should a performed bit (one with
   nights on tape) be left alone, so a jot never lands under something he already does on stage? Lead's default:
   file everywhere, badge and Undo on each.
3. **Loose material on the map.** Hollow dots for loose pieces beside the bit they look like, drawn from 2.5×
   (where names appear), so he can pull in from the cloud. It makes the whole-map zoom busier. Lead's default: not
   in these slices.
4. **Docs not yet split** (option 3's Q4). 38 docs hold several jokes each. Claude proposes the split (one call per
   doc, each proposal confirmed by him), or he selects the lines himself (no AI cost, more taps)? Not in these slices
   either way.
5. **A set clip pulled in: words into the pile, or a link only?** The deck's Pull in on a taped moment can put its
   transcribed words into the pile as a scrap (so the bit holds what he actually said) and link the tape, or link
   only. Lead's default for Slice 5: words as a scrap plus the `performed_as` link.
6. **Where the score shows, the budget, consent, the test jokes, seed rules** (old spec Q3, Q4, Q10, Q11, Q12) are
   still the owner's; none blocks Slices 1-3.
7. **The Memos segment on Bits** goes in Slice 5 (memos are loose until filed, then scraps). If he reaches for it
   backstage to play a memo, the pile's play glyph and Loose material › Memos are the two paths; say if a third is
   wanted.

---

## 14. Build slices (smallest first; each ships to TestFlight on its own)

Every slice: `python -m pytest creative engine gsync worker infer/tests tools/audio/tests api-dev/tests -q`
(the four `gsync/tests/test_auth.py` failures are the known local-only ones, HANDOVER item 9); `cd app && npx tsc
--noEmit && npm run test:unit && npx expo-doctor` (`test:unit` includes `tests/static.worklets.test.mjs`, which
compiles `src/rules` with the real worklets plugin, and the `static.*` suites); `python tools/guard/check.py
--staged` before every commit; a failing-capable check for every rule; nothing private; LF. Rust slices add
`cd rust && PK_REQUIRE_EXTERNAL=1 cargo xtask gate` and the route's own scratch-DB test. **No time estimates.**

### Slice 1: the page, app-only (no server change)

| | |
|---|---|
| What | The 3c page on today's data: the bar (sparkle disabled until Slice 4 with a reason, To a set, More with the new order and Stage ›); the idea line editable (today's `decide`, the badge from `stage_ai`'s twin once the server sends it, else no badge); lines at 44 pt with selection and **Punch · ⋯ (Move · Cut)** (Tighten · Tag after greyed "later" is a control that leads nowhere, so they are **not rendered** until Slice 4); "+ N tags" fold; `SectionRow` with Pile (the attached notes the page already can read through `/api/merge/candidates?joke=` and the memo rows; Add = `addVersion` with the words inserted, no placement yet, so the scrap stays listed with "in the bit" until Slice 3), Versions (the stepper over `versionsOf`, Restore, no diff marks), Sessions hidden (none exist), On tape (`/similar`'s performed rows; "killed" shown only if the rows carry `rating`), the Claude line hidden; **Write = today's inline editor** (`setEditing`, Cancel · Save) opened from the new bottom button; the offline line above Write; Write and To a set greyed "offline" when offline (D4 until Slice 2). The **punch mark** is the one thing with no home yet: Slice 1 shows the punch from `line_meta` when the read carries it and the Punch action writes nothing (hidden) until Slice 3; the fold therefore appears only from Slice 3. **Plus the spike screen** `app/spike.tsx` for the three device checks (a bottom toolbar on a scrolling page in the Bits stack, a focus-mode mock with three per-line inputs and the glass bar, dictation into them), reached from Settings › About › "Device checks"; removed in Slice 2. |
| Files | `(tabs)/(bits)/joke/[id].tsx` (restructure); new `lib/SectionRow.tsx`, `lib/BitLines.tsx`, `lib/VersionStepper.tsx`, `lib/IdeaLine.tsx`; new `rules/bitLines.ts` (pure: split, punch, tags fold, labels, meta strings, the AX re-stack rule) + `tests/bitLines.test.mjs` (added to `test:unit`); `lib/TopBar.tsx` (`OfflineStrip placement`), `lib/Screen.native.tsx` / `Screen.tsx` / `screenTypes.ts` (`offlineAt`); `app/spike.tsx` + a `Stack.Screen` in `app/_layout.tsx`; `lib/prefs.ts` (`tips.pile`, `focus`); `(tonight)/settings.tsx` (the About link); the `/joke/[id]/tape` and `/joke/[id]/sessions` routes in `(bits)/_layout.tsx` (tape built now from `/similar`; sessions a placeholder is a control that leads nowhere, so the row stays hidden). |
| Routes | None new. Reads: `/api/jokes/{id}`, `/similar`, `/api/merge/candidates?joke=`. Writes: today's versions route, `decide`, `boards.add`. |
| Risks | The bottom toolbar on iOS 26 with the minimising tabs (device check 1; the fallback is drawn and coded behind a flag so the owner can flip without a build if both are shipped); `headerTransparent` with `scroll` (HANDOVER trap: this page scrolls, so the default holds); the `static.values` count map (new literals go to tokens or the map entry falls, DESIGN.md §10). |
| Gate | tsc, `test:unit` (the new `bitLines` suite with a red arm: a punch index past the last line is refused), `static.worklets`, expo-doctor 20/20, guard. **Owner checks on the phone**: the page reads as drawn in dark and light; a line selects and Cut writes a version with Undo in the row; the stepper reads v5 and Write greys "Reading v5"; To a set from the bar adds with the snack; More › Stage sets the word and the Bits dot follows; the offline line sits above Write with no signal; **the three device checks (§4.1) and which way each went**. |
| **Built** (2026-11-03, branch `app/slice1-bit-page`; reviewed by ios-design-expert and ios-dev-expert, must-fix items fixed) | **As specified**, with these calls recorded. (1) Under a selected line the actions are two visible chips **Move · Cut**, not a lone ⋯ holding them (Punch is hidden per this row; HIG/menus "use submenus sparingly", HIG/context-menus "make items available in the main interface"): the ⋯ returns in Slice 4 when it holds Find the turn · Why?. Move is not rendered on a one-line bit. (2) The idea editor and Rename use the page's RN-Modal `Sheet` + `SheetHead` (Cancel leading, Done trailing) until the kit's native `RenameSheet` lands (DESIGN.md §6 row 3). (3) The On tape row reads "not yet · 2 sound like it" when the bit was never taped but `/similar` has echoes, so link it / merge it in stay reachable; hidden only when there is nothing behind it. (4) The bottom "Write" is **label-only**: UIKit draws a bar item that has an image as the image alone (INFERRED), and the word is the HIG-standard prominent action; the spike's button is the same, so the owner sees it. (5) The footnote slot above Write reads "Offline · reading only" with no signal (the native cache is memory-only, so "showing what this phone saved" would be untrue on a cold start) and "Reading v5" on an old version; its height is measured (`onLayout`), never fixed. (6) The sparkle is rendered disabled with the reason in its VoiceOver hint only (a native bar button carries no text); the owner may hide it instead with one `&&`. (7) The stepper's haptic plays under Reduce Motion too (HIG/motion: haptics are the alternative to motion; the haptics switch is the owner's), overriding §3.5's "none under Reduce Motion". (8) Inline rows' chevron points down closed and up open (HIG/disclosure-controls); push rows keep the right indicator. (9) The spike lives at `(tabs)/(bits)/spike.tsx` (check 1 needs the tab bar beneath) and `app/spike-write.tsx` (a full-screen modal with the shipped custom task header inside the KeyboardAvoidingView: with a native header the KAV is short by the header and check 2 would fail for the wrong reason). (10) No `/joke/[id]/sessions` route (nothing points at it). (11) The web puts Write and More in the header; To a set is in the More sheet there. (12) `linkColor()` in `theme.ts` is the accent as text (amber on dark, the ink step on light). Open from the native review for Slice 2: the real focus route needs the same custom header or a `keyboardVerticalOffset`; check 1 also answers whether the safe area counts the bottom toolbar (the footnote line sits above Write, or under it). |

### Slice 2: focus mode and the outbox

| | |
|---|---|
| What | `app/joke/[id]/write.tsx` as §3.3 (per-line inputs or the fallback, decided by Slice 1's check 3; the glass bar as decided by check 2; the dictation rule with the settle; Cancel asks; Done writes a version); Bits "+" opens it; the second tap on a line opens it at that line. **The outbox** (§8) for version, to_set (punch, place, attach, dismiss are wired as their routes land): `lib/bitOutbox.native.ts` / `.ts`, `rules/bitOutbox.ts` + `tests/bitOutbox.test.mjs` (next item, backoff, the content dedupe, the conflict decision, `local_unit` rewriting, Undo on waiting); the Versions row's "v9 · waiting · Undo"; Write and To a set work offline. The spike screen goes. |
| Files | the new route + `_layout.tsx`; `lib/KeyboardBar.native.tsx` (`GlassView` behind `isLiquidGlassAvailable`, `colorScheme` from `C.scheme`) + web twin; `lib/bitOutbox.*`, `rules/bitOutbox.ts`; `joke/[id].tsx` (Write → the route; the row meta); `lib/api.ts` (`X-Idempotency-Key` on `post` when given); `docs/design/DESIGN.md` §6 (done with this spec); `app/_layout.tsx` (`startBitOutbox` beside `RecordingsQueue`). |
| Routes | None new; the Python versions route directly, deduped by content (§8.2). |
| Risks | Keyboard flash on Return (check 3); the KAV offset (check 2); a version written twice if the refetch dedupe misses (mitigated: equal text + date ≥ `at`; the receipt route in Slice 3 closes it); the 4,000-character limit on a long bit (the editor shows the count only when within 200 of it, as a one-word reason on Done). |
| Gate | tsc, `test:unit` (the `bitOutbox` suite with red arms: a replay after a newer server version must flag the conflict, never drop; an Undo on a waiting item must leave the queue without the item and the cache without the patch), `static.worklets`, expo-doctor, guard. **Owner checks**: write three lines in focus mode with Return between them and Done → v9 with the snack; dictate two lines with "new line" between; Pile · N on the bar opens the sheet over the keyboard, Add lands after the line he was on, the keyboard comes back; airplane mode → type a punchline, Done → "v9 · waiting · Undo", signal back → "v9 · just now"; To a set offline → the snack says when. |

### Slice 3: the pile, its server routes, the deck and the capture proposal

| | |
|---|---|
| What | `db/schema_bit.surql` (`line_meta`, `placement`, `write_receipt`) applied with his go (ADR 0009) after the per-role login for pk-api; pk-api: `GET /api/bits/{id}/pile`, `POST /api/bits/{id}/versions` (receipts; proxied Python write + `line_meta`), `PUT …/lines/{unit}`, `POST …/pile/{attach}/place` + `/unplace`, `GET …/versions/{unit}/diff`, `GET …/tape`, `summary_ai` / `summary_confirmed` on `/api/jokes/{id}`; the app: the Punch action live, the "+ N tags" fold, the pile inline as §3.4 (five + All, the play glyph, +, detail sheet `/scrap/[id]`, long press, swipes, Not this bit, the first-use tip), `/pile/[id]` over focus mode, the outbox moved onto the receipt route (punch, place, attach, dismiss queued), the stepper's cut / later marks, the On tape row's "killed twice"; **the deck** (`/loose` as §3.9; `/inbox` and `/merge` keep their URLs; Tonight's "N to sort" routes to `/loose`); **the capture proposal** on `/say` and on the record screen's saved state. |
| Files | `db/schema_bit.surql`; `rust/crates/pk-api/src/routes/bits.rs` (+ `OWN_PATHS`, the Ingress prepared under `infra/k8s/prepared/`), `pk-store` typed rows for the three tables, `pk-wire` types (+ `ts-rs` output); `rust/crates/pk-api/tests/bits.rs` (scratch DB); app: `lib/Pile.tsx`, `app/pile/[id].tsx`, `app/scrap/[id].tsx`, `(tabs)/(bits)/loose.tsx` (from `inbox.tsx`), `(tonight)/say.tsx` (`BitEntry`'s second bubble), `record.tsx` (saved state), `lib/bitWork.ts` (the new calls), `rules/bitOutbox.ts` (the new kinds), `lib/api.ts` (`JokeSummary.ai`). |
| Routes | §6.1 rows 1-2, 4, 6-9; reused: `merge/attach`, `/undo`, `/dismiss`, `candidates`. |
| Risks | pk-api not yet deployed (the deploy order: the Python with `GET /api/ai/worker`, then pk-api's Ingress; this slice's app code must work against the Python alone for the reads it still has and hide the pile's placement until pk-api answers); the per-role login prerequisite; the receipt table's growth (7-day sweep); the `similar` crate's diff on a line he edited by a word (the edit-distance rule, tested). |
| Gate | Rust: `cargo xtask gate` + `tests/bits.rs` with red arms (a repeated idempotency key must return the first response and write nothing; a `place` on an attach that is not this bit's must 404; a paraphrased `quote` in a span must 409); app: tsc, `test:unit`, `static.worklets`, expo-doctor, guard; the API differential still 432/432 with the new keys allow-listed and named. **Owner checks**: Pile · N opens inline with the five newest; + puts a scrap after the selected line with Undo in the row; the tip shows once and never again; swipe a scrap away → it is in Loose material with Undo; the deck: Pull in puts a jot in that bit's pile; after a jot on Tonight, "Looks like X · Put it in the pile" appears under the reply, and Not this leaves it loose; the stepper shows a cut line struck with "Bring this line back". |

### Slice 4: line actions and sessions with Claude (needs the T8 worker swap and pk-api deployed)

| | |
|---|---|
| What | The old spec's Phase 1 and Phase 4 through 3c's surfaces: `db/schema_teach.surql`; the session routes; pk-worker `session_plan`, `session_breakdown`, `session_tighten`, `session_tag`, `session_turn`, `session_talk` with spans; `/session/[bit]` (S2, with Save version hidden until a change is kept); **Tighten · Tag after · Find the turn · Why?** under a selected line; the sparkle and the Claude line (`GET /api/bits/{id}/thoughts`, `/jokes/{id}/sessions`); the Sessions row and screen; **Break it down** and **Find the turn** as whole-bit jobs from the sheet; the worker-offline and paused states greying the asks with one word. |
| Files | as the old spec §7 and §11 phases 1, 3, 4, plus `lib/BitLines.tsx` (the four actions), `app/session/[bit].tsx`, `app/joke/[id]/sessions.tsx`, `engine/config.hosted.toml` (the tasks), `rust/crates/pk-worker/src/kinds/session.rs`, `rust/crates/pk-api/src/routes/sessions.rs`. |
| Routes | Old spec §7.2 in full; `GET /api/bits/{id}/thoughts`. |
| Risks | T8 and the pk-api deploy are the owner's calls and come first (R4); the daily cap (Q3); offsets (R3, the quote check and 409); injection through lessons (R8, `frame::data`). |
| Gate | Rust: the gate + the old spec's scratch tests (a paraphrased supporting line refused; kept hunks → the exact version text; a stale base → hunks re-found or reported); the worker differential unchanged for the nine existing kinds; app: tsc, `test:unit`, `static.worklets`, expo-doctor, guard. **Owner checks**: select a line › Tighten → the sheet at the medium detent with the hunk, Keep → the line changes on the page as a draft, Save version → v10; Why? answers about that line; Break it down from the sparkle → the card, mark one part Wrong; the sparkle greys "offline" in airplane mode. |

### Slice 5: Gather, overnight filing, the Bits list, On tape counts everywhere, desktop focus

| | |
|---|---|
| What | `GET /api/bits/{id}/gather` and Gather as the first row of the sheet's whole-bit jobs (and "Gather from your notes ›" under an empty pile); pk-worker `pile_file` on a nightly schedule with the threshold, the badge, the Undo, and Tonight's "N to sort" counting only the unsure; the Bits list rows (`built_from`, the "N to sort" badge, 72-pt rows), the Loose material card above the list, the Memos segment retired, Desk / Notebook redirects, "Filter…" in the sort menu; set clips as scraps (open question 5's default); the filing-precision and keep-share cards in Settings › What I've learned; the desktop focus toggle with edge handles (`lib/Screen.tsx`, `prefs.focus`). |
| Files | `pk-api` `routes/bits.rs` (gather, the worker pile routes), `pk-worker` `kinds/pile_file.rs`, `engine/config.hosted.toml`; app: `bits.tsx`, `desk.tsx`, `notebook.tsx`, `notes.tsx`, `lib/Screen.tsx`, `(tonight)/learned.tsx`, `lib/api.ts` (`built_from`, `pile`). |
| Routes | §6.1 rows 10-11; list-row keys on both servers (lane L) or after T10. |
| Risks | The list-row keys vs the API differential (allow-list or the Python twin); the sweep's precision on his real jots (the pause rule at 0.8); row height on the Bits list at AX sizes. |
| Gate | Rust gate + a `pile_file` scratch test with a red arm (a pair under the threshold must not attach; a dismissed pair must never attach); app gates; guard. **Owner checks**: the morning after a jot, it is in the right bit's pile with the badge, Undo takes it out and Tonight's count does not include it; Bits rows read "2 docs · 4 jots" and the Loose material card leads with the actionable count; on the laptop, focus collapses both panes to handles and Escape brings them back. |

---

## 15. Amendments this spec makes elsewhere

- `docs/design/DESIGN.md` §5.4 and §6 "Never": a `GlassView` is allowed for the floating input bar **and the
  keyboard bar in focus mode** (both navigation-layer controls; HIG/virtual-keyboards); nothing else. HIG.md §5's
  sentence "Nothing else gets glass" reads with this one exception.
- `docs/design/SPEC-2026-11-02-bit-sessions-and-joke-teaching.md`: marked superseded for §3 and §11 at its top.
- `docs/HANDOVER.md`: one open-list pointer to this spec and its slices.
