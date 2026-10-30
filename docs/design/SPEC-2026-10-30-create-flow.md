# SPEC 2026-10-30: the create flow ("+" on Tonight, Bits and Sets)

Status: design spec against `main` 79e143c (TestFlight 18). Owner, on build 18: "i dont like the buttons for when we click
new bit or new set, we need a better UX for this". Tags: **CONFIRMED** = read at `file:line` or on the HIG page cited;
**PROPOSED** = what this spec sets. HIG pages are `https://developer.apple.com/design/human-interface-guidelines/<page>`
(written `HIG/<page>`). Nothing here is a commit, push or deploy; no code under `app/` is touched by this spec.

**Claim: the "+" should do the one obvious thing at once, the way Notes and Reminders do, and the sheet of buttons
goes.** Bits already does this (build 17); Tonight and Sets still open a form sheet of rows, length chips and a Start
button, which is what he is calling "buttons". Target: **1 tap from "+" to typing** on Tonight and Bits, **1 tap to an
open set list** on Sets (its first bit one tap further), secondary creates in the toolbar's existing `⋯` menu.

## 1. Today (CONFIRMED)

| Tab | What "+" does | Taps to the first keystroke | Cite |
|---|---|---|---|
| Tonight | pushes `/create`, a form sheet at detent 0.5 titled "New": four 64-pt card rows (Bit · Set list · Record a set · Voice memos), a YouTube field + "Import" `Btn`, "Or just say it…" | **2** to a bit, and it lands in `/say` (the conversation), not a document; 3-4 to a set list (+ → Set list → Start, or name first) | `(tonight)/index.tsx:58,86`; `create.tsx:14-45`; `CreateSheet.tsx:19-29,91-112`; `_layout.tsx:151` |
| Bits | creates the bit (`decide add`, title `Untitled bit <ISO minute>`), pushes `/joke/<id>?edit=1`; the editor is focused; the first Save names it from the first seven words | **1** | `bits.tsx:139-150`; `joke/[id].tsx:64-68,99-102` |
| Sets | pushes `/create?start=set`: a name field, four length `Btn`s (5·7·10·15) + a "custom" number field, a prominent `Btn` "Start the set list" **and** a toolbar "Start" (two commits for one write) | **2** to an empty list (+ → Start); **3** to its first bit (→ Add a joke) | `sets.tsx:100,186`; `create.tsx:30-33`; `CreateSheet.tsx:114-126` |

What breaks "standard, familiar" (docs/UX_2026-10.md; HIG.md §1):

- **A sheet used as a menu.** "If you need to give people important information or present a set of choices, consider
  using an alert or action sheet" and "Avoid using a sheet to help people navigate your app's content" (HIG/sheets). Three
  of the four rows only navigate (`/say`, `/record`, `/notes`): `CreateSheet.tsx:95-98`.
- **Choices drawn as custom buttons, not a system menu.** "Prefer simple, recognizable symbols … rely on established
  conventions for navigation and modality" (HIG/toolbars, HIG/branding). The rows are `Pressable` cards with our icons,
  so he reads them as buttons, and nothing else on the phone looks like them.
- **Two primary actions in one view.** "Only specify one primary action, and put it on the trailing side of the toolbar"
  (HIG/toolbars); "Keep the number of prominent buttons to one or two per view" (HIG/buttons). The set form has the
  amber `Btn` plus the bar's Start (`create.tsx:33`, `CreateSheet.tsx:125`), plus four length chips, one of them amber.
- **Asking for data the app can default.** "You can also prefill fields with reasonable default values, which can
  minimize decision making and speed data entry" (HIG/entering-data). Name and length are both asked before the list
  exists; the command path already defaults both ("Friday · 7", UX_2026-10 §Commands) and `POST /api/boards` only
  requires a title (`api-dev/creative_api.py:1099-1106`; `target_s` defaults to 900).
- **Two ways to make a bit that land in two places.** Tonight "+ → Bit" goes to the conversation; Bits "+" opens a
  document. "Consistency builds familiarity" (HIG/writing).

## 2. Target (PROPOSED)

The pattern a comic already knows: Notes' compose glyph opens a blank note with the keyboard up and names it from its
first line; Reminders' "New List" exists before you name it. HIG/pull-down-buttons allows an Add menu ("An Add button
could present a menu that lets people specify the item they want to add") but also warns "A view's primary actions need
to be easily discoverable, so you don't want to hide them in a pull-down button". The primary create per tab is
unambiguous, so **"+" acts; the `⋯` menu (`ellipsis.circle`, already on Sets, `sets.tsx:183`) carries the rest.**
`Stack.Toolbar.Button` has `onPress` only, no long press (`StackToolbarButton/types.d.ts:125`), so the secondary
choices live in the toolbar menu, not under a hold.

| Tab | "+" (tap) | `⋯` menu items, in order | Taps |
|---|---|---|---|
| Tonight | the same `newBit` as Bits: create, push `/joke/<id>?edit=1`, keyboard up | New set list · Record a set · Import a recording… (`/import`) | 1 to typing; 2 to a list; 2 to the recorder |
| Bits | unchanged (build 17) | none added: the bar already holds Map · sort · + ("aim for a maximum of three" groups, HIG/toolbars) | 1 |
| Sets | `createBoard({ title: defaultListTitle(now, titles), joke_ids: [], target_s: 420 })`, `boards.setCurrent`, push `/list/<id>` | Record a set · Archived (existing) | 1 to the list; 2 to Add a joke; 2 to Rename… |
| Set list `⋯` | — | Rename… · **Length…** (new: 5 · 7 · 10 · 15 · custom, `PATCH {target_s}`, `creative_api.py:1124-1125`) · Duplicate · Share… · Delete | the length he used to pick up front, now one menu away |

Rules: a menu item that needs more input ends in an ellipsis; no articles; items sentence-cased (HIG.md §7 accepted
deviation) (HIG/menus). The record and import items keep `ui.withMic` / the existing routes. Nothing new is modal.

**What goes.** `app/src/lib/CreateSheet.tsx` and `app/src/app/create.tsx` are deleted on every platform; the web gets
the same immediate "+" and its secondary items through the existing `MenuSheet` behind a "…" `IconBtn` (web parity,
same labels). The YouTube field already exists on `/import` (web paste, `import.tsx:2-3`), so nothing is lost. Remove
`'/create'` from `REQUIRED_URLS` (`src/rules/routeTable.ts:11`), the `/create` clause in `tests/routes.test.mjs:41`,
the `Stack.Screen name="create"` line (`_layout.tsx:151`) and its comment. `docs/design/HIG.md` §2 (the rows
"create.tsx" and "Create ("+")") and §3.12 are superseded by this spec; say so there in the next docs pass.

**Conflicts.** (a) HIG/pull-down-buttons permits an Add menu; the owner's "one-handed, fewer taps" and the Notes
precedent win: the "+" acts. (b) The set length was a first-class choice in the sheet; HIG/entering-data says default
it. Default 7 min (the sheet's own default, `CreateSheet.tsx:44`), changed from the list's menu. (c) Offline, `decide`
fails and "+" shows a snack (`bits.tsx:148`): a known gap, out of this spec; the snack copy below says what to do.

## 3. Copy (PROPOSED; sentence case per HIG.md §7)

| Where | Text |
|---|---|
| "+" (Tonight, Bits) | symbol `plus`, label "New bit", hint "Opens a blank bit with the keyboard up" |
| "+" (Sets) | symbol `plus`, label "New set list", hint "Starts an empty list named for today" |
| `⋯` (Tonight) | items: "New set list" (`list.bullet.rectangle`), "Record a set" (`record.circle`), "Import a recording…" (`square.and.arrow.down`) |
| `⋯` (Sets) | "Record a set" (`record.circle`), "Archived" (`archivebox`) |
| Set list `⋯` | "Rename…", "Length…" (`timer`), "Duplicate", "Share…", "Delete" (destructive, last) |
| Length sheet | title "How long is the set?", chips "5 min · 7 min · 10 min · 15 min", field placeholder "custom", Cancel / Done |
| New bit, default title | `Untitled bit YYYY-MM-DD HH:MM` (kept: the marker the first Save keys on, `joke/[id].tsx:99`); the bar shows "New bit" until it is named |
| New bit, named | the first seven words of the first saved line, trailing punctuation cut, ≤ 120 chars, else "New bit" (`joke/[id].tsx:101`, moved to a rule) |
| New set list, default title | `<Weekday> · <d> <Mon>`, e.g. "Thursday · 30 Oct"; a second one that day gets " (2)", then " (3)" |
| Fresh bit editor placeholder | "Write the bit. The first line names it." |
| Bits empty | "No bits yet. Tap + to write the first one." |
| Sets empty | "No set list yet. Tap + and it starts one for tonight." (the in-content `Btn` at `sets.tsx:199` goes: the "+" is the button) |
| Fresh set list empty | "Nothing on it yet. Add a joke below." + a 44-pt text link "Name it" → the rename sheet |
| Snack after a new list | "Started Thursday · 30 Oct" with one action "Rename" |
| Snack when "+" fails offline | "Can't start a bit offline. Ask it in the search field and it saves when you're back." |

## 4. Targets, Dynamic Type, VoiceOver, haptics

- Toolbar items are the system's: 44-pt targets and SF scaling come free (HIG/buttons "at least 44x44 pt"). The web
  `IconBtn` stays 44 (`ui.tsx:359`). "Name it" is a `TextBtn` with `minHeight: 44`.
- Dynamic Type: the editor keeps 17/26 with font scaling on; menu text is SF and scales itself; the empty-state `T`
  scales through `buildTv`. Nothing under 11 pt (DESIGN.md §4).
- VoiceOver: "+" `accessibilityLabel` "New bit" / "New set list" with the hints above (`accessibilityHint` is on
  `StackToolbarButtonProps`); `⋯` is "More"; menu items are their own text. The fresh bit's editor keeps
  `accessibilityLabel="The bit's text"` (`joke/[id].tsx:211`) and focus lands in it (`autoFocus`).
- Haptics: `haptics.success()` once the server has the new bit or list (the existing "success on save", `haptics.ts:1`),
  never on the tap itself; the menu plays the system's own. Off when Settings › Haptics is off (HIG/playing-haptics
  "Make haptics optional"). Reduce Motion: pushes and menus are system-animated; nothing custom.

## 5. Acceptance

Node (pure rules, added to `test:unit`): `src/rules/setListRules.ts` gains `defaultListTitle(now: Date, taken:
readonly string[])` (English weekday/month arrays, no locale); `src/rules/bitWorkRules.ts` gains `freshBitTitle(now)`,
`isFreshBit(title)` and `nameFromFirstLine(text)`; `bits.tsx` and `joke/[id].tsx` import them instead of the inline
strings. Tests: `defaultListTitle(2026-10-30T20:00, [])` = "Thursday · 30 Oct"; with it taken → "Thursday · 30 Oct
(2)"; `nameFromFirstLine("so my landlord, right, he texts me at 3am...")` = "so my landlord, right, he texts me"
(seven words, trailing punctuation cut); a 121-char first line → 120; empty → "New bit"; `isFreshBit` true only for
the marker prefix. Static gates: `routes.test.mjs` no longer resolves `/create`; `static.rules` stays green (rules
import nothing from lib); `npx tsc --noEmit`; the ESLint ceiling does not rise; grep finds no `CreateSheet` import.

Owner's phone check (build 19), taps counted from the tab root: Tonight "+" → keyboard up on a blank bit (**1**); type
a line, Save → the bar shows its first words. Bits "+" → the same (**1**). Sets "+" → an empty list titled for today
(**1**), the bottom bar reads "Add a joke · 7:00 left"; tap it (**2**) → the picker. `⋯` → Length… → 10 min → the bar
reads "10:00 left". `⋯` → Rename… works; "Name it" in the empty list opens the same sheet. Tonight `⋯` shows three
items; "Record a set" asks for the mic once and opens the recorder. A second Sets "+" the same day makes "(2)". No
screen anywhere shows the old "New" sheet. Dark and light; VoiceOver reads "New bit, button" and the hint.

## 6. Builder brief (another agent, in this order)

1. Rules + tests (§5), then switch `bits.tsx:139-150` and `joke/[id].tsx:99-101` to them; export `newBit` from a
   small `lib/newBit.ts` so Tonight and Bits share one function.
2. Tonight: "+" → `newBit`; add `Stack.Toolbar.Menu icon="ellipsis.circle"` with the three items; web: the same via
   `MenuSheet`. Drop `openCreate`, `CreateSheet` and the `create` state (`index.tsx:37,58,136`).
3. Sets: "+" → create + push; `⋯` gains "Record a set"; the empty state loses its `Btn`. Set list: the "Length…" item
   and a `fitToContents` sheet (reuse the `Sheet`/`SheetHead` kit until A5 lands), the empty-state "Name it".
4. Delete `create.tsx` and `CreateSheet.tsx`; fix `routeTable.ts`, `routes.test.mjs:41`, `_layout.tsx:151`; run the
   gates in `docs/HANDOVER.md`; one TestFlight build; the owner runs §5.
