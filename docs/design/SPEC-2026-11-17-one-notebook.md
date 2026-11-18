# SPEC 2026-11-17: One notebook

Status: **built on branch `app/one-notebook` (§9 "Built"); not merged, not pushed, not deployed, no phone build.**

The owner, on build 33 (Bits · Sets · Tapes), word for word: "i feel like the ux is still clonky, i think people are
used to google docs/notes how can we simplify it but keep it as powerful". On the mockup that answered it: "ok lets
deploy this". His standing rules hold (`SPEC-2026-11-16-bits-sets-tapes.md` B11, B12): a bit is a document, as easy as
editing a Google Doc on the phone, tap and type, no edit mode; Punchable's power is the Redline loop, and a round
returns a whole new draft.

**The mockup** is six artboards kept in `docs/design/mockups/2026-11-17-one-notebook/` (`Main`, `Page`, `Select`,
`Draft`, `Set`, `Stage`; plain HTML, 390 × 844). It is the source for layout and wording. Every joke in it and in this
file is invented.

**The design in one paragraph.** No tabs. The app opens on one list, **Notebook**: every bit, set and tape, newest
first, grouped by day, with a search field over all of it, a filter (All · Bits · Sets · Tapes), a Pinned section, the
avatar for You and Settings, and two buttons at the bottom: talk out a new page, or start a blank one. A bit is a page
he types in; three buttons sit above the keyboard (Say it, Film it, Punch it up). Marking is selecting: with words
selected the same bar offers Funnier · Shorter · 3 options · Note. A round returns the whole new draft with every
change underlined; he flips Mine / New draft, puts any change back, asks for another take on one, and uses it or keeps
his. A set is a page too: a numbered list he types into (`@` finds a bit). On stage the bit's name is huge and his note
is under it.

This is a new front on the same data. Bits, versions, sets (boards), set notes, tapes, redline rounds and the marks on
the phone are stored exactly as before. One small server change (§4).

Tags: CONFIRMED = read at that `file:line` on this branch or in the installed modules; INFERRED = reasoned from code
that was read, not run; UNKNOWN = needs a phone. **Nothing here was run on a phone.**

---

## 1. Screens

| # | Screen | Route | What it is |
|---|---|---|---|
| 1 | Notebook | `/` | The one list. System large title and search field; a segmented filter All · Bits · Sets · Tapes; "New draft ready" rows first; Pinned; then day groups (Today, Yesterday, This week, Earlier this month, a month's name). A row is a title, one line of his words, and a status line only when there is something to say ("New draft ready", "2 marks · 1 tape"). Bottom: the microphone (records a memo: a new page from what he says), the page count, the pencil (a blank page, keyboard up). Hold a row: Pin or Unpin. With the filter on Sets the pencil makes a set. |
| 2 | A bit is a page | `/joke/<id>` | The title is its own field at the top (tap and type; Return goes to the text). Under it the document (`lib/BitDoc`, unchanged: one text field, autosave, a version on leaving or pausing). The bar shows a quiet "Saved", Undo (the last save), ⋯ (Versions, Add to a set, Share, and the rest of what More held). One bar above the keyboard, and at the bottom of the page when the keyboard is down: **Say it** (a memo filed on this bit) · **Film it** · **Punch it up**. Under the document, when he is not typing: his marks, then Around this bit, In these sets, the Pile and On tape as before. |
| 3 | Marking is selecting | the same page | With words selected the bar is **Funnier · Shorter · 3 options · Note**, for exactly those words; each is a switch (tap again and the mark is gone; another one changes how he asked). A marked range stays marked in the text (tinted and underlined) while he types and after. The marks are listed under the document with their notes (tap one to edit its note or remove it). With marks on the page the bar reads "N marks on this page. Anything you didn't mark stays word for word." and **Get a new draft**. |
| 4 | The new draft | `/joke/<id>/draft` | A sheet over the page: **Keep mine** leading, **Use it** trailing; a Mine / New draft switch; the whole draft as text with each changed range underlined on a tint. Tap a change: **Put mine back** / **Try again**, and the other takes when he asked for 3 options. A change outside what he marked is put back by itself and said so. His words are always kept in Versions. |
| 5 | A set is a page | `/list/<id>` | The title as a field; "5 bits · about 7 min · you have 8"; a numbered list, each bit with his note under it and its time; a last row he types in: `@` plus letters (or just letters) lists matching bits and "New bit called …"; a grip moves a row (hold and drag; VoiceOver: Move up / Move down). Bottom: **Record this set** and Film. |
| 6 | On stage | `/run/<id>` | The walk-through of Slice 5, restyled: the clock and "2 of 5 · 8 min slot" on top, the bit's name huge, his note in amber, "Show the full page", "Next: …", Back and Next at 84 pt, "Hold to end the set". The level bar, "Paused by a call" and Resume stay. |

## 2. What is removed

| Gone | Replaced by |
|---|---|
| The tab bar (Bits · Sets · Tapes), `lib/TabsShell*`, the web's bottom tabs and sidebar tabs, the device checks for a Record control by the tab bar (`(tabs)/recslot`, the accessory strip) | Notebook and its filter |
| The marks bar (Change · Ask · Keep · Copy to…: `lib/MarkBar`, `lib/DocMarkBar`) and the marks tray (`/marks`, `lib/MarkTray`) | The page bar (`lib/PageBar`) and the marks under the document (`lib/MarksList`) |
| The draft screen's layout (answers first, Draft · Yours, tinted lines, a marks bar for round two, Use this draft / Discard in a bottom toolbar) | Screen 4 |
| The Bits, Sets and Tapes roots as screens | Notebook rows; their URLs open Notebook with that filter |
| The bit page's idea line, its round row and its always-open Versions row | The idea is in ⋯ (Idea…); the round is the bar's button ("New draft ready"); Versions opens from ⋯ |
| A set page's seams between bits, drawn on every row | Off by default; ⋯ › Show how the bits connect |

Ask and Keep are no longer offered when marking (a Note covers a question; "anything you didn't mark stays" covers a
Keep). The server still takes both, and a round sent by build 33 still reads.

## 3. Routes and redirects

Every screen moved from `(tabs)/(bits)`, `(tabs)/(sets)`, `(tabs)/(tapes)` into one group, `(nb)`, with one stack. A
group's name never appears in a URL, so **every URL in `rules/routeTable.ts` `REQUIRED_URLS` still resolves**
(`tests/routes.test.mjs`). What changed behind a URL:

| URL | Now |
|---|---|
| `/` | Notebook (was: pop to Bits) |
| `/bits`, `/sets`, `/tapes` | Pop to Notebook with the filter set (`/?show=bits` …); never a second copy of the root (`lib/PopTo`, journal gotcha 2026-11-17) |
| `/filmed` | Notebook, filter Tapes |
| `/marks` | The tray is gone: opens the bit it names (`?bit=`), else Notebook |
| `/joke/<id>/draft` | The same URL, the new sheet |
| `/stage/<id>` | Still redirects to `/run/<id>` |
| `/topic/<id>` | Unchanged (a topic made into a set opens that set) |
| `/recslot` | Removed with the tab bar (a device-check page, never linked) |
| `/profile`, `/settings`, `/voice`, `/learned`, `/usage` | Unchanged (the stack behind the avatar) |
| Universal links (`/joke/x`, `/list/x`, `/set/x`) | Unchanged; a cold link gets Notebook under it (`anchor: 'index'`) |

Everything the old roots' toolbars held is still reachable: the joke map, Notes to sort (the inbox), Merge review,
Archived sets, Import a recording, Record a set and topics not yet in a set are rows of Notebook's ⋯ menu.

## 4. Server: how he asked rides on a mark

A mark today is `{type: change|ask|keep, text, at?, note?}` (CONFIRMED `rust/crates/pk-api/src/writes/redline.rs`
`marks_arg`). The selection's buttons need one more word.

- **`intent`**, optional, on a Change: `funnier` | `shorter` | `options` | `again`. Note is a plain Change with his
  note, as before. Anything else is a 400; on an Ask or a Keep it is dropped.
- **Rust only.** The route keeps it on the stored round and in the job's input **only when it is set**, so a mark
  without one is stored and keyed byte for byte as before. No new store, no new op, no new job kind, no schema change,
  no Ingress change.
- **Who tolerates what** (each CONFIRMED by reading): an older pk-worker ignores the field (`WireMark` has no
  `deny_unknown_fields`) and treats the mark as a plain Change; the `redline` store's reducer keeps `marks` as it gets
  them, so an older pk-api replays the rows; the Python API keeps a job's `input` whole and never reads it
  (`api-dev/creative_api.py:118`); an older pk-api drops the field from a newer phone's body (the round is then a plain
  Change). So: **pk-worker first, then pk-api**, and nothing breaks in between.
- **The prompt.** Under the quoted words the worker prints one line for the intent ("Make this funnier.", "Make this
  shorter: fewer words, the same joke.", "Give 3 options …", "He passed on this take. Write a different one."), then
  his note when he typed one. Golden: `pk-worker/tests/goldens/redline_intents_prompt.txt`, written by hand. A round
  with no intent is the old golden, word for word.
- **3 options** come back inside the one new draft: the model puts the take it would pick in the draft and lists all
  of them under `options: [{item, texts}]` (asked for in the answer's schema only when a mark asks). The worker keeps a
  row only for an item that asked, drops empty and repeated takes, puts the take that stands in the draft first, and
  drops the row when none of them is in the draft. The result gains `options: [{item, quote, texts, at}]` only in such
  a round. On the phone a tap on that change lists the takes; picking one swaps it in before Use it.
- **Try again** is round two of the existing loop: one Change on that range of the draft with `intent: again`.
- The checks on a draft (a Keep byte for byte, no unmarked idea dropped, refused twice is never shown) are unchanged.

## 5. "Anything you didn't mark stays word for word"

The server asks the model for it and refuses a draft that drops an unmarked idea, but it does not prove every
unmarked word. The phone makes the sentence true: the draft is compared with his words (`rules/draftRules.ts`: lines
first, then words inside changed lines), and **a change on a line he put no mark on is put back by itself**; the
screen says how many and lets him take any of them ("Use the new one"). What he uses is the text he sees.

## 6. Platform calls (iOS dev and design reviews, before the build)

| Question | Call |
|---|---|
| Items in the system's selection menu | **Not possible without native code** in React Native 0.88 (CONFIRMED `RCTUITextView.mm:305-325`: no hook; `@expo/ui`'s text field has none either). Built instead: the one bar above the keyboard, driven by the selection. The system's Copy / Look Up bubble is untouched. The native menu is on the roadmap: a local Swift module that adds a `UIMenu` in `buildMenuWithBuilder:`; high risk with no Mac. |
| A root with no tab bar | One stack; the system large title and search field; a segmented control for the filter (HIG: chips are not a system control; a scope bar is for search only). |
| Mic and pencil | A bottom toolbar on the phone (the system's), a bar on the web. The mic is drawn in the label colour, not red: red means "recording now". |
| The draft | A sheet: Keep mine / Use it are Cancel / Done. Swiping it down keeps his words and the draft waits. |
| Hold to move, hold to end | Never the only way: the grip has Move up / Move down for VoiceOver, and "Hold to end the set" opens the Leave sheet on a VoiceOver activate. |

## 7. Deferred

- Items inside the system selection bubble (native module).
- A note drawn between two paragraphs of the page: the page is one text field, so notes are listed under it.
- Search over sets' and tapes' words on the server (Notebook filters set titles and phone tapes on the phone; bits and
  taped moments use `GET /api/search`).
- A tape as its transcript (Slice 7), a Google Doc as written (Slice 8), the always-there Record control (Slice 9) of
  the earlier spec: unchanged, still to build.
- Pins are kept on this device only.
- Removing the leftover screens of the earlier spec's §7 Q7 (still waiting on the owner).

## 8. Owner's phone checks

Listed in §9 once built.

## 9. Built

(Filled in when the branch is done.)
