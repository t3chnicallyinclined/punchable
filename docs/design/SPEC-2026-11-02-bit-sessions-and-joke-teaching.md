# SPEC 2026-11-02: sessions on a bit, and teaching Claude his misdirect method

> **Superseded in part (2026-11-03).** The screens (§3) and the build order (§11) are replaced by
> `SPEC-2026-11-03-bit-page-3c.md` (design option 3c, approved by the owner, with the build slices). **§1 his
> method, §5 the data model, §6 learning without weights, §7 server, §8 cost, §9 measurement and §10 privacy still
> hold** and are referenced from there, not repeated.

Status: **draft for the owner** (design only, no code). The iPhone sections come from ios-design-expert (HIG, read
live on developer.apple.com 2026-11-02) and ios-dev-expert (native feasibility). Server, data and worker are the
lead's. Tags: **CONFIRMED** = a file:line, a doc page or a guard run; **INFERRED** = design reasoning, not proven.
Nothing here is built.

**The claim in one paragraph.** Claude can learn his method **without training any weights**:
- he marks Claude's breakdown of each joke right or wrong, part by part, and says why;
- the cards he has corrected come back into later prompts. They are picked by the **shape of the joke** (what the
  audience pictures, what is true, what is left out), not by its words;
- reasons that keep recurring become standing rules that ride in every prompt.

The same screens hold all AI work on a bit: a **session** belongs to the bit, Claude's edits arrive as **tracked
changes** he keeps line by line, and the session ends in a saved version. We can tell whether it is learning in three
ways: a held-out set of his own labeled jokes, a "hide the punch word, can Claude find it" test, and the share of parts
Claude gets right before he corrects them.

**The smallest first slice is offline (Phase 0).** Run the method prompt over 20-30 of his jokes and have him mark the
cards in a private file. That gives the baseline with zero lessons. It needs no app build, no schema and no deploy.
Phase 1 is "Break it down" in the app.

---

## 1. His method (the theory the prompts encode)

The owner's theory, in his terms (2026-11-02):

| Part | What it is |
|---|---|
| **Scene A** | The picture the setup paints in the audience's head. |
| **Scene B** | What is actually true. The setup **never lies**: every line is true in B as well as in A. |
| **Left out** | The one fact withheld, so the audience fills it in wrong. |
| **Supporting lines** | Every line backs up the wrong picture (A) while staying true in B. |
| **Reveal word** | The last word, ideally a single word, gives away the missing fact and forces the audience to throw out the whole picture. |

**Method.** Picture two scenes that can be described with the same words. Find the one word that tells them apart.
Hide it until the end.

**His two example jokes** are in his private notes (the worked examples, with all five parts filled in, are in the
`docs/punchable/` folder of his private material repo). **The commit guard blocks one of them as his material**
(CONFIRMED: `tools/guard/check.py --files` on 2026-11-02 reported a 7-word run from his data folder), so neither is
quoted here. In general terms:
- One is about **who someone is**. A family scene with a child's cartoon and tickling reads as one relationship, and
  the reveal word, an age, makes it a different one.
- The other is about **where someone is**. A request and a rant read as one kind of exchange, and the reveal, a
  workplace, makes it a service exchange.

**Invented examples for this spec and for the public prompt** (not his material; written for illustration only):

| Joke (invented) | Scene A | Scene B | Left out | Reveal |
|---|---|---|---|---|
| "We've been together eleven years. He still waits by the door when I get home. Still gets excited when I call him. Still drinks out of the toilet." | a devoted partner | a dog | what he is (species) | toilet |
| "Same woman every night. She brings me dinner, asks about my day, says I look tired. Last night she said she's seeing someone. I said 'You can't do this to me.' She said 'Sir, I'm your waitress.'" | a wife or partner | a waitress at a diner he eats at every night | who she is to him (role) | waitress |

**Known failures of AI attempts** (his list). These become seed rules (§6.4):

| Failure | Example (invented) |
|---|---|
| Comparison punchline | "That's not a marriage, that's a hostage situation." |
| Stock targets | wife, mother-in-law |
| Revealing too early | the withheld fact shows up in line two |
| No concrete scene | abstract observation, nothing to picture |

---

## 2. Sessions (the agent window, on a bit)

| Rule | Detail |
|---|---|
| A session belongs to **one bit** | Sessions on a **set list** come later (§11 phase 8, Q7). |
| A session has **one mode** | **Break it down** · **Find the turn** · **Tighten** · **Tag it**, plus **Talk** (free questions about the bit; today's bit chat, §4). |
| Claude never changes a bit without his acceptance | Every edit is a **tracked change** that he keeps or skips **per line**. |
| A session ends tied to **a version** | If he kept changes, it ends in the new version he saved. Otherwise it ends on the version it ran against. The bit's existing versions are the store (CONFIRMED: `POST /api/jokes/<id>/versions` writes a `unit` of kind `tag` plus a `part_of` edge, and old versions stay; `api-dev/server.py:1658-1683`). |
| Nothing needs review | His marks are optional. Every tap saves. Claude's output shows as the current state with the AI badge. |

### Modes

| Mode | Input | Claude returns | He does | Saved as |
|---|---|---|---|---|
| **Break it down** | a span of the bit (lines he picks, or the whole bit) | a card: Scene A, Scene B, Left out, Supporting lines (verbatim), Reveal word, a technique tag | marks each part Right or Wrong with one tap; can fix it and say why (typed, or a voice memo) | a `breakdown` (the lesson) |
| **Find the turn** | the bit (a new one, or a setup without its ending) | 3-5 reveal words, each with the scene it flips to | thumbs up or down on each; **Write it in** turns one into a tracked change | `turn_rating` rows (preference data) |
| **Tighten** | the bit | line edits: cut, trim, reorder | Keep or Skip per line, then **Save version** | `suggestion` rows, then a version |
| **Tag it** | the bit | tag lines after the punch | Keep or Skip per line, then **Save version** | `suggestion` rows, then a version |
| **Talk** | his typed question | an answer with **Keep as a line** | as today | `session_turn` rows |

---

## 3. Screens

### 3.1 Phone (ios-design-expert; HIG pages read live 2026-11-02)

Keep the tab bar (Tonight · Bits · Sets). Everything below lives on the bit's page (`app/src/app/(tabs)/(bits)/joke/[id].tsx`,
the workbench; CONFIRMED by both experts).

| # | Screen / element | Spec | Evidence |
|---|---|---|---|
| S1 | **Sessions button** on the bit page | Bottom toolbar, one prominent full-width button above the tab bar. Label: **"Work on it"**, or **"Resume · Tighten"** while a session is open. Owner asked for "a Sessions button"; the label is Q9. | HIG/toolbars ("Use the .prominent style for key actions"); bottom placement CONFIRMED in `docs/design/HIG.md` §3.4 |
| S1 | **Sessions section** on the bit page | Sits above Voice memos. Up to 3 rows, then "All sessions ›". Each row reads mode · date, with a second line like "v7 · 3 of 5 kept" or "Lesson · 4 of 5 right". A tap opens the session read-only. Hidden when there are none. | HIG/lists-and-tables |
| S1 | Version chips | A version a session made reads `v7 · Tighten · Nov 2`. An open session's draft reads **"Draft · 3 kept"** with an amber outline. | CONFIRMED chip row `joke/[id].tsx:209-218` |
| S2 | **Session sheet** | A new root route `app/session/[bit].tsx`, modelled on `bitnotes` (`presentation: 'formSheet'`, detents `[0.5, 1]`, grabber on, `sheetLargestUndimmedDetentIndex: 0` so the bit above stays live, `sheetExpandsWhenScrolledToEdge: false`). The sheet covers the tab bar while it is open; that is the HIG's own exception. | CONFIRMED `app/_layout.tsx:65-67,166`, `bitnotes.tsx:77-80`; HIG/sheets ("consider supporting the medium detent", nonmodal sheets); HIG/tab-bars (modal exception); Apple: the undimmed area "responds to user interaction" ([UISheetPresentationController](https://developer.apple.com/documentation/uikit/uisheetpresentationcontroller/largestundimmeddetentidentifier)) |
| S2 | Sheet bar | Leading: **Close**, which keeps the session open (same as swiping down, so nothing is lost and there is no confirm). Title: the mode. Trailing: **Save version**, disabled until he has kept a change. | HIG/sheets pairs Done with Cancel; this is a deliberate deviation, risk R1 |
| S2 | Sheet root | "New session": four rows (a list, not a segmented control, because each row starts something), then Past sessions, then a one-line score. | HIG/segmented-controls ("Don't assign actions to segments") |
| S2 | Composer | One line pinned with `unstable_sheetFooter`: "Ask Claude about this bit". Typed only. | CONFIRMED footer pattern `bitnotes.tsx:77-80` |
| S3 | **Tracked changes** | The unit is a **whole line**: old struck through, new under it; an inserted line; a deleted line. On the bit, a deleted line is grey with strikethrough and a "−" in the gutter. An inserted line has an underline, a tint background and a "+" in the gutter. **The text itself is never coloured** (light green measures about 4.1:1, under the 4.5:1 floor). New adaptive tokens `insBg` / `delBg` go in DESIGN.md §3. | HIG/color, HIG/accessibility, HIG/dark-mode (4.5:1); ratio arithmetic INFERRED; no red tint token today, CONFIRMED `theme.ts:34-41,100,107-110` |
| S3 | Keep / Skip | On the phone these sit **in the sheet**, where the thumb reaches: one row per change ("Was …" / "Now …") with 44-pt **Keep** and **Skip** buttons. A full swipe also works (Mail pattern), but the buttons are always shown. Tapping a marked line on the bit selects its row. Every decision can be undone. **Keep all** lives only in the ⋯ menu, with a one-snack undo (Q2). | HIG/generative-ai ("revert or retry"); HIG/undo-and-redo ("revert multiple changes at once") |
| S3 | VoiceOver | One element per change: "Change 2 of 4. Removed: …. Added: …", with actions Keep and Skip. Strikethrough is not spoken, so the label says it. | HIG/accessibility; INFERRED |
| S4 | **Picking the joke** (Break it down) | Phase 1: **he taps lines.** The bit enters select mode, each line becomes a row with a circle, and tapping the first and last line fills the range between. Or he takes **the whole bit** (the default when the bit is one joke). Later: Claude proposes the jokes in the bit and he taps one (Q5). Word-level selection is **wide screens first**. | ios-dev: RN 0.88 has **no API for a custom edit-menu item** (CONFIRMED `RCTUITextView.mm:305-325`; `<Text selectable>` raises no selection event, `TextProps.js:102,225`); HIG/lists-and-tables (edit-mode selection) |
| S5 | **Breakdown card** | An inset-grouped list headed **"Claude's read"** with the AI badge. Five rows: Scene A · Scene B · Left out · Supporting lines (each quote in quotation marks; a tap highlights it on the bit) · Reveal word. Each row has two 44-pt toggle buttons, **Right** and **Wrong**, filled when on (not colour alone). **Wrong** opens the row in place: "Fix it" (prefilled with Claude's words), "Why? (optional)", and a mic. Each tap saves, and the status reads "Saved as a lesson · 3 of 5 right". Leaving everything unmarked is allowed. | HIG/generative-ai ("simple thumbs-up and thumbs-down", "Always make providing feedback voluntary"); HIG/toggles |
| S6 | **Reason by voice** | Recorded inline in the row, Messages-style (clock, level bar, stop square), using `lib/recorder`. It **cannot open `/record` over the sheet**, because only one sheet layer is allowed. Transcribed on the phone; only the words are sent. | HIG/modality; CONFIRMED one sheet layer `SPEC-2026-10-29-set-lists-and-stage-notes.md:37`; on-device only, CONFIRMED `lib/transcribe.native.ts:110-114,162` |
| S7 | **Find the turn** | 3-5 rows: the reveal word (Headline), then "flips to: …". Trailing thumbs up / thumbs down; stars are not supported on iOS. **Write it in** per row. Footer: **Show others**. | HIG/rating-indicators ("Not supported in iOS"); HIG/generative-ai ("offering alternate versions") |
| S8 | Past sessions | Read-only replay: changes with their Keep / Skip outcome, or the card with its marks. **"Open v7"** opens the bit at that version. | INFERRED |
| S9 | **The score** | One line on the sheet root, e.g. "Claude got 3 of 5 right last time · 68% over 20 lessons" (Q4). A card in Settings › What I've learned: a line chart (`@expo/ui` Chart on iOS, the existing SVG on web) and a per-part table. | CONFIRMED `@expo/ui` Chart `type="line"`; `learned.tsx` exists |

**Copy** (plain words, sentence case), from ios-design-expert:

| Element | Copy |
|---|---|
| Mode rows | **Break it down** "Pick a joke; Claude says how it works; you mark it." · **Find the turn** "Claude offers reveal words; you rate them." · **Tighten** "Claude cuts and trims; you keep what you like." · **Tag it** "Claude adds tags after the punchline; you keep what you like." |
| Changes and saving | **Keep** · **Skip** · "Skipped · Undo" · **Save version** · snack "Saved as v8 · Undo" |
| Empty states | "No sessions yet. Pick one above to start." · "This bit is one line. Break down the whole thing?" |
| Offline | "Claude is offline. Your marks are saved; suggestions come back when it is." |
| Budget | "Claude is paused for today. This session picks up after midnight UTC." |
| Nothing to suggest | "Claude has nothing to cut here." |

### 3.2 Wide: web, iPad, desktop (three panes)

This follows the owner's desktop agent window, which itself follows VS Code's Agents window: a grouped list on the
left, the work in the middle, the session beside it. The reference spec is private and outside this repo; only the
layout pattern is used.

| Width | Layout | Evidence |
|---|---|---|
| < 900 px (web) or compact (iPad) | The phone design, with the sheet | HIG/split-views ("regular — not compact") |
| 900-1279 px | App sidebar (232) · **bit** · **session (360)**. The bits list is behind a "Bits" toggle. | CONFIRMED breakpoint `lib/theme.ts:198`, sidebar `lib/Screen.tsx:65`; HIG/split-views ("intermediate window widths") |
| ≥ 1280 px | App sidebar · **bits list (300, grouped)** · **bit (flex, max 760)** · **session (380)** | HIG/sidebars ("a content list between the sidebar items and detail view") |

- **On wide screens, Keep / Skip also sit on the line itself** (a pointer is precise), as well as in the session pane.
- Word-level selection works with a pointer: drag-select, then "Break down selection".
- The session pane hides with a bar button and remembers the choice. It is not hidden by default (HIG/sidebars).
- Build order: **web first.** For iPad later, `expo-router/unstable-split-view` `SplitView.Inspector` is iOS 26+ and
  marked unstable, and the app has never been designed for iPad (CONFIRMED `app.json:58` `supportsTablet: true`;
  HIG.md B9) (Q6).
- Grouping of the left pane is Q8.

### 3.3 Native feasibility (ios-dev-expert)

| Item | Route | Risk |
|---|---|---|
| Picking a span | Phase 1: tap lines, a plain RN row model. Later, "Select words…": a `TextInput` with `editable={false}` and `onSelectionChange`, plus our own button (CONFIRMED `RCTTextInputComponentView.mm:215-216,515-534`). It must live outside today's tap-to-edit `Pressable` (`joke/[id].tsx:231-233`). | A custom edit-menu item needs our own Swift module, on an API Apple marks deprecated in iOS 27. **Not planned.** |
| Half-height sheet | Reuse the `bitnotes` options. | The keyboard resizes a medium sheet and can bring the dimming back (CONFIRMED `react-native-screens` does no keyboard work here, `RNSScreen.mm:1174`). **Device check:** the owner's open build-18 notes-sheet check is the cheapest proof. |
| Tracked changes | Hunk blocks. Plain paragraphs are one `Text`; each change is a `View` with its buttons. **Diffing happens on the server** (no diff library in the app, CONFIRMED by a `package-lock.json` grep). | Offsets drift: see §5.3. |
| Getting Claude's answer | **The same job-plus-poll as Ask** (CONFIRMED `lib/ask.tsx:28-34,63-97`), plus a reload when `/api/live` reports a change. A proposal only makes sense whole, so streaming gains nothing. | none |
| Voice reason | `lib/recorder` and the recordings queue with a session/part target. The memo route over 4,000 characters must not divert it to the set-transcript path (CONFIRMED `SPEC-2026-10-31-voice-memos.md:73`). | It goes through the transcription queue, so the row shows "Transcribing your reason…". |
| No signal backstage | Marks and reasons queue (file-backed, idempotency key: the memo queue pattern). Keep / Skip queue too. **A proposal is never shown unless it exists.** | A cold start offline shows no bit text: the response cache is in memory only on native (CONFIRMED `lib/api.ts:130-153`). Separate item. |
| App Store | TestFlight internal only today. | Guideline 5.1.2(i) requires **explicit permission before sharing personal data with third-party AI**. Settings only states it (CONFIRMED `settings.tsx:171`). Needed before any external tester (Q10). |

**No new native module is needed for Phases 1-5.**

---

## 4. What happens to Ask under Tonight

**Decision proposed (ios-design-expert, agreed by the lead).** "Ask your material" stays on Tonight for questions about
all his material. A conversation **pinned to a bit** becomes a **Talk** session on that bit.

| Today (CONFIRMED) | Change |
|---|---|
| Tonight's search field sends to `/say`, which makes an ask chat (`(tonight)/index.tsx:49,54`). On the phone the past-questions list `/ask` is **unreachable**: `AskButton` only renders on web branches, and Tonight passes `ask={false}` (`index.tsx:111`, `bits.tsx:173-178`). | Add one row under Tonight's three questions, **"Past questions ›"**, opening `/ask`. No new tab or button. |
| A bit's chat (`lib/BitTalk`) is a creative `chats` item with a `joke` field (`pk-creative/src/reducers/chats.rs:24-28`). The app keeps it in a module map, lost when the app quits (`lib/BitTalk.tsx:12-15`). | Chats with `joke` set are **listed as Talk sessions** on that bit (read from the chats store; no data migration). New Talk turns are `session_turn` rows (Phase 5). |
| The bit page has its own input bar (`joke/[id].tsx:148,181`). | On the phone the bar goes away. Asking moves into the session; adding a line already lives in Edit (Q1). |
| An answer from Ask that cites one bit | Gets **"Work on <bit>"**, which opens the bit with the sheet up. |

---

## 5. Data model (SurrealDB, hosted, the single source of truth)

### 5.1 What exists and is reused (CONFIRMED)

| Thing | Where | Used for |
|---|---|---|
| `joke` (the bit) | `db/schema.surql:18` | `session.bit` |
| `unit` (a version: kind `tag`, `part_of` the joke) | `db/schema.surql:55`; written by `api-dev/server.py:1677-1683` | `session.base_unit` / `saved_unit`; the text the spans point into |
| `unit.embedding` (bge-small 384-d) with the HNSW index | `db/schema.surql:86` | the same model and dimension for the plan vector |
| `decision` (append-only, undo by seq) | `db/schema.surql:322` | the version write and its undo (unchanged) |
| `creative_event` store `chats` | `db/schema_extra.surql:8`, `chats.rs` | old bit chats, listed as Talk sessions |
| `profile:voice.learned` (kept 25, dropped 25, laugh 15) | `creative/learned.py:25-27` | kept session lines feed it through `draft: true` (`server.py:1693-1695`) |
| The engine's `lesson` table | `rust/crates/pk-engine/schema/engine.surql:25` | **Not used.** The name is taken, so ours is `breakdown` (§5.4). |

### 5.2 New tables

All new tables go in a new file, `db/schema_teach.surql`: SCHEMAFULL, `IF NOT EXISTS` on tables. It is applied
through the ADR 0009 path, with his go, on the hosted database. Every row carries `at`; append-only where marked.

| Table | Key fields | Notes |
|---|---|---|
| `session` | `bit: record<joke>`, `mode` ∈ {breakdown, turn, tighten, tag, talk}, `status` ∈ {open, closed}, `base_unit: record<unit>`, `saved_unit: option<record<unit>>`, `span: option<{start, end, quote, unit: "utf16"}>`, `by`, `opened_at`, `closed_at` | One mode per session. `target` for set lists is added later (Q7). |
| `session_turn` | `session`, `n`, `role` ∈ {me, claude}, `text`, `job: {kind, status ∈ {queued, claimed, done, failed, paused}, claimed_by, claimed_at, tries}`, `usage: {model, in, out, cache_r, cache_w, usd}`, `error` | **Also the queue** for session work (§7.1). |
| `suggestion` | `session`, `turn`, `base_unit`, `hunk: {line_from, line_to, removed: [str], added: [str]}`, `state` ∈ {proposed, kept, skipped}, `decided_at`, `checks: {rule_hits: [...]}` | Tighten, Tag it, and Find the turn's "Write it in". The state changes; history goes in `session_event` below. |
| `breakdown` (**the lesson**) | `bit`, `unit`, `span`, `claude: {scene_a, scene_b, left_out, supporting: [str], reveal, technique}`, `marks: {part → right / wrong / none}`, `fixed: {part → text}`, `final` (Claude's card with his fixes applied), `plan_text`, `plan_vec: array<float, 384>`, `right_before` (count), `marked` (count), `held_out: bool`, `status` ∈ {live, retired}, `model`, `prompt_rev` | HNSW index on `plan_vec`. **Held-out rows are never retrieved** (§6). |
| `correction` | `breakdown`, `part`, `from`, `to`, `reason`, `via` ∈ {typed, memo}, `reason_vec: array<float, 384>`, `at` | Append-only. One row per Wrong-with-a-fix or a reason. |
| `technique` | `key` (e.g. `role_left_out`), `label`, `by` ∈ {seed, ai, owner} | Seeds: who-someone-is (age, role, relationship), where (place), when (time), what (object or species), how many, who is speaking. "Other" lets Claude name a new one, AI badge. |
| `reveal` | id = the reveal word, lower-cased and normalised | Lets "which reveal words recur" be a graph query. |
| `rule` | `text`, `status` ∈ {standing, retired}, `origin` ∈ {owner, promoted}, `from: [record<correction>]`, `check: option<string>` (the name of a programmatic check, §6.4), `at`, `retired_at` | Every standing rule rides in every session prompt. |
| `turn_rating` | `session`, `turn`, `candidate: {reveal, flips_to}`, `rank_shown`, `rating` ∈ {up, down}, `written_in: bool`, `at` | Preference data: shown order plus his rating. |
| `teach_eval` | `run_id`, `kind` ∈ {breakdown, masked_turn}, `arm` ∈ {no_lessons, lessons, lessons_rules}, `model`, `prompt_rev`, `n`, `per_part: {...}`, `hit_at_1`, `hit_at_5`, `usd`, `at` | One row per held-out run (§9). |
| `session_event` | every Keep / Skip / mark / undo as an append-only event | Undo, plus an exact replay for the future training set. Its own table, not `creative_event`, whose `seq` the Python API owns in memory (§7.1). |

**Graph edges (new relation tables).**
- `uses` (breakdown → technique)
- `reveals` (breakdown → reveal)
- `corrected_by` (breakdown → correction)
- `promoted_from` (rule → correction)
- `session_of` is not needed: it is the `session.bit` field.

That gives his chain **joke → technique → reveal word → corrections** as one traversal.

### 5.3 Spans and offsets (ios-dev-expert)

- The phone sends `{unit, start, end, quote, unit_kind: "utf16"}`. iOS `NSRange` and JS count **UTF-16 code units**;
  Rust indexes UTF-8, so pk-api converts (INFERRED).
- The server checks that `text[start..end] == quote` on that exact `unit`. If not, it answers 409, then re-finds the
  quote in the latest version.
- The client must never send a span against a synthetic id. The client falls back to `${date}-n` / `'idea'` keys
  (CONFIRMED `joke/[id].tsx:43,47`), so `unit` must be a real unit id.

### 5.4 Where the upstream engine fits (pk-engine, ADR 0013) and where it does not

| Engine piece | Use here | Why |
|---|---|---|
| `pk_engine::verbatim::verbatim(quote, source)` (CONFIRMED `verbatim.rs:84`), already the one gate (ADR 0013) | **Yes.** Every supporting line on a card must pass against the span's unit text. The reveal word must pass against the span. A Tighten "trim" that claims to keep his words must pass against the base line. | "Every AI line claiming to be his is verbatim-checked." Paraphrase alignment stays **off** (ADR 0013). |
| `card.rs` + `frame::data` (a card goes into a prompt only through `render`: quote first, facets labelled unverified, every field escaped; CONFIRMED `card.rs:1-10`) | **Yes, as the pattern** for putting lessons back into prompts (§7.3). His reasons and Claude's old wrong facets are text a model will read, so they are framed as data. | Injection defence, already reviewed upstream. |
| `learning.rs`: `Kpis`, `replay(reads, candidate, held_out)`, `Guard` / `fence` (CONFIRMED `learning.rs:143,312,356-371`) | **Yes, the pure functions.** A rule promotion or a prompt revision is a *candidate*; it is replayed on the held-out set before it goes live, and fenced (rolled back) if the score drops. | The same promote/rollback discipline, without its ranking tables. |
| The engine tables (`lesson`, `rank_config`, `rank_switch`, `packet`, …) | **No.** Punchable's own tables above. | The engine's offline proof **did not pass** (ADR 0013 runs 1-3; `RUST_WORKSTREAM.md` T19). Its `lesson` is a fact / decision store with grades, and grades never reach the app (ADR 0013). A breakdown is domain data with a different shape. |
| **Per-role database logins** (the engine review's open item: "per-role logins with table PERMISSIONS come before any service writes these tables", CONFIRMED `FORK.md:90,97`; ADR 0013 last paragraph) | **Applies here before anything is wired.** pk-api gets a login that may write only the §5.2 tables and read `joke` / `unit`. The worker still never opens the database (ADR 0011). | Today every service shares one login, so a table's guards stop bugs, not a holder of that login. |

---

## 6. Learning without weights

### 6.1 The loop

```
he marks a card ──► breakdown.final + corrections ──► plan_vec (bge-small) ──► retrievable lesson
                                     │
                                     └─► reason_vec ──► cluster ──► standing rule (every prompt)
new request ──► draft plan (cheap) ──► nearest lessons by plan_vec (+ technique diversity) ──► the real call
```

### 6.2 Retrieval by plan, not by words

- **Plan text** = `Picture: <scene_a>\nTruth: <scene_b>\nLeft out: <left_out>`, from `final` (his fixes win). It is
  embedded with the model the API already ships (bge-small, 384-d; CONFIRMED HANDOVER "Search", `unit_vec`
  `schema.surql:86`).
- A new bit has no plan yet, so each request runs a **draft plan** first (Sonnet, small, §8), embeds it, and takes
  the k = 3 nearest live, non-held-out breakdowns.
- **MMR over technique** (INFERRED): no more than two lessons share a technique.
- **Corrected lessons rank ahead of all-right ones.** The correction is the teaching signal.
- The **reveal word and the joke text are never embedded for retrieval**, so the structure matches across topics: an
  age-left-out joke about a hospital finds an age-left-out joke about a cartoon.
- **Each lesson goes into the prompt as a contrast**: "Claude read Left out as X. He fixed it to Y, because Z." His
  quote comes first, and the whole lesson is framed as data (§5.4).

### 6.3 The graph

**joke → technique → reveal word → corrections** are edges (§5.2). Two queries the app can show in Settings › What
I've learned:
- "your most-used technique";
- "reveal words Claude keeps missing".

### 6.4 Standing rules

| Source | When it becomes standing | Shown |
|---|---|---|
| **His stated failure modes** (§1): comparison punchlines, stock targets (wife, mother-in-law), revealing too early, no concrete scene | **Seeded on day one** with `origin: owner` (his statement of 2026-11-02) | Settings › What I've learned › Rules; he can retire one or add his own |
| **A recurring correction**: ≥ 3 corrections on ≥ 3 different bits whose reasons are near each other (`reason_vec` cosine ≥ a threshold fixed before the first run, e.g. 0.80), or the same part wrong with the same technique | **Automatically** (no review step): Haiku drafts one sentence quoting his reasons. Before it goes live it is **replayed on the held-out set** (§9). If it scores worse it stays proposed and unused, and says why. | The same list, AI badge, one-tap Retire |

**Program checks, not only prompt text** (INFERRED):
- "no comparison punchline" is a pattern check ("that's not X, that's Y", "isn't X, it's Y");
- "no stock targets" is a word list.

Both run on every suggestion and Find-the-turn candidate before it is shown. A hit is dropped and counted
(`suggestion.checks`). The count is a learning metric (§9).

### 6.5 The future training set (not built in this spec)

`breakdown` (Claude's card, his marks, his fixes, his reasons), `turn_rating` (shown order and rating) and the
Keep / Skip log are exactly the supervised and preference pairs an own open-weight model would need. They stay in his
database. Training one, and a blind test against Claude to decide whether it replaces anything, is a later, separate
decision.

---

## 7. Server: routes, worker, prompts

### 7.1 Where the code goes (the Rust port in progress)

| Fact (CONFIRMED) | Consequence |
|---|---|
| New routes go into **pk-api** (Rust). It answers its own paths and forwards everything else to the Python (`proxy.rs`); the Ingress lists exactly `OWN_PATHS` (`rust/crates/pk-api/src/lib.rs:47`; `RUST_WORKSTREAM.md` T9). | The session routes are **new paths** with no Python twin, so there is nothing to diff against. They are added to `OWN_PATHS` and proven by their own tests on a scratch 3.3.4. |
| pk-api writes are T10 ("all at once"), still open (`RUST_WORKSTREAM.md:57`). | The new tables are written by pk-api from day one; they are not in the T10 set. **The version write stays Python's until T10.** "Save version" in pk-api builds the text, then calls the existing `POST /api/jokes/<id>/versions {text, source: "session", draft: true, reason}` through its proxy with his token (`source` must match `^[a-z_]{1,24}$`, CONFIRMED `server.py:1672`). It records the returned unit as `saved_unit`. INFERRED design. |
| The Python API holds the creative `jobs` queue **in memory**, replayed from `creative_event` (`api-dev/creative_api.py:336-339,532-537`). | Session jobs **must not** be appended to that store by a second writer. They get their own queue in `session_turn`, owned by pk-api. |
| ADR 0011: the worker speaks HTTP only, and the cutover is "one image swap, never a per-kind split (one worker, one spend ledger)". | New kinds run **only in pk-worker**. They wait for **the T8 swap** (built, on the owner's go; `RUST_WORKSTREAM.md:55`). pk-worker claims session work from pk-api at `/api/worker/sessions/claim` when its normal claim is empty: one process, one ledger, one budget pause. |

### 7.2 Routes (pk-api, owner-only actor rule on every one)

| Route | Does |
|---|---|
| `GET /api/jokes/{id}/sessions` | Sessions on the bit (plus old bit chats as Talk), newest first. |
| `POST /api/sessions` `{bit, mode, base_unit, span?}` | Opens a session and queues its first Claude turn. The span is checked (§5.3). One open session per bit and mode: a second POST returns the open one. |
| `GET /api/sessions/{id}` | Turns, suggestions, card, ratings. The app polls this. |
| `POST /api/sessions/{id}/turns` `{text}` | His message; queues Claude's turn. |
| `POST /api/sessions/{id}/suggestions/{sid}` `{state}` | kept / skipped / proposed (undo). Idempotency key. |
| `POST /api/sessions/{id}/save` | Applies kept hunks to `base_unit` and writes the version (§7.1). If the bit has a newer version than `base_unit`, the hunks are re-found in the latest text; any hunk that cannot be found is reported and left out, never forced. |
| `POST /api/sessions/{id}/close` | Closes without saving. A breakdown session stays tied to its `base_unit`. |
| `PATCH /api/breakdowns/{id}` `{part, mark?, fix?}` | One tap = one call. Recomputes `final`, `plan_vec`, `right_before`. |
| `POST /api/breakdowns/{id}/reasons` `{part, text, via}` | A reason, typed or from a memo's words. Legal rules run first (as `server.py:1667-1670` does). |
| `POST /api/sessions/{id}/ratings` `{candidate, rating}` | Thumbs up or down on a Find-the-turn candidate. |
| `GET /api/teach/score` | Per-lesson series, per-part rates, the latest held-out run. |
| `GET` / `POST /api/teach/rules`, `PATCH /api/teach/rules/{id}` | List, add his own, retire. |
| `POST /api/worker/sessions/claim`, `GET /api/worker/sessions/{turn}/context?plan=`, `POST /api/worker/sessions/{turn}/result` | Worker only (the `pk-worker` client-credentials role). `context` embeds the draft plan and returns the framed lesson packet plus the standing rules. |

**On a worker result**, pk-api validates before anything is visible:
- the JSON schema;
- every supporting line passes `verbatim` against the unit text (a failing line is **dropped** and the card says so);
- the reveal word is inside the span, and its position is recorded ("last word" or "N words from the end");
- every hunk's `removed` lines equal the base text at those lines;
- the rule checks (§6.4);
- the legal rules.

### 7.3 Worker kinds and tasks

New `[tasks.*]` entries go in `engine/config.hosted.toml`, which pk-llm's router reads (CONFIRMED T5). Structured
output is validated with `jsonschema` plus one retry (T5). Thinking is adaptive (CONFIRMED `config.hosted.toml:42`).

| Kind | Task (model) | Prompt (system blocks, in cache order) | Output |
|---|---|---|---|
| `session_plan` (step 1 of every mode except Talk) | `session_plan` (Sonnet 5.5) | method block | `{scene_a, scene_b?, left_out?}` |
| `session_breakdown` | `session_breakdown` (Opus 5.5, effort high) | 1. **method** (static: §1, invented examples only) · 2. **profile + learned** (existing, cached; `creative/voice.py`) · 3. **standing rules** · user: framed lessons, then the bit, the span | the card + `technique` |
| `session_turn` | `session_turn` (Opus) | same 1-3 · user: lessons, the bit (or the setup without its last line) | 3-5 `{reveal, flips_to, line}` |
| `session_tighten` | `session_tighten` (Opus) | same 1-3 · user: the bit with numbered lines | hunks by line number |
| `session_tag` | `session_tag` (Opus) | same 1-3 · user: the bit, its punch line | added lines after a given line |
| `session_talk` | `session_talk` (Sonnet, like `ask_chat`, `config.hosted.toml:99-101`) | the existing Ask prompt, with the bit's sources first | answer |
| `rule_draft` | `rule_draft` (Haiku 5.5) | the reasons | one sentence |
| `teach_eval` | Opus / Sonnet as run | as the mode under test | `teach_eval` row |

**The method block is public** (this repo), so it carries **no material**: only §1's theory and the invented
examples. His own examples reach the prompt **only as retrieved lessons from his database** at run time.

---

## 8. Cost per call (INFERRED; the token counts are assumptions until Phase 0 measures them)

Prices are per million tokens: input / output / cache write / cache read.
- Opus 5.5: 4.00 / 20.00 / 5.00 / 0.20
- Sonnet 5.5: 2.00 / 10.00 / 2.50 / 0.20
- Haiku 5.5: 0.10 / 0.50 / 0.125 / 0.01

(CONFIRMED `rust/crates/pk-llm/src/budget.rs:43-45`.)

Assumptions:
- cached system ≈ 6k tokens (method 1.5k, profile ≈ 3k, learned ≈ 1k, rules 0.5k);
- uncached input ≈ 2.5k (bit ≤ 1k, 3 lessons ≈ 1.2k, the ask);
- adaptive thinking ≈ 2k output tokens. **UNKNOWN; Phase 0 measures it.**

| Call | Model | Estimate per call (warm cache) | First call in a cache window |
|---|---|---|---|
| Draft plan | Sonnet | ≈ $0.006 | — |
| Break it down | Opus | ≈ $0.06 (output ≈ 2.4k → $0.048; input $0.010; cache read $0.001) | + ≈ $0.03 cache write |
| Find the turn | Opus | ≈ $0.06-0.07 | + ≈ $0.03 |
| Tighten / Tag it | Opus | ≈ $0.06-0.08 | + ≈ $0.03 |
| Talk turn | Sonnet | ≈ $0.02-0.04 (as Ask) | — |
| Rule draft | Haiku | < $0.001 | — |
| **One session (plan + one Opus call)** | | **≈ $0.07** | ≈ $0.10 |
| **One held-out run** (25 jokes × breakdown + masked turn) | | **≈ $3.50** | |

**The daily cap is $2.00 by default** (CONFIRMED `engine/budget.py:21`, `PK_AI_DAILY_USD`). That is about 25-30
sessions a day, and **a held-out run alone exceeds it** (Q3).

---

## 9. How we know it is learning

| Measure | How | Shown to him |
|---|---|---|
| **Parts right before correction** | Per breakdown: `right_before / marked`; unmarked parts are excluded, never counted right. Rolling over the last 20, plus a per-part rate (Scene A, Scene B, Left out, Supporting, Reveal). | Yes: the sheet line and the Settings chart |
| **Held-out set** | 20-30 of his jokes he labels fully (his own card, or Claude's fully corrected), flagged `held_out`, **never retrieved**. A `teach_eval` run re-does the breakdown with the current lessons and rules. The reveal word is scored exact (normalised). Scene / Left out are scored by a Sonnet judge against his label, and a sample of 5 per run he checks himself, so the judge's agreement with him is known. | Yes: "On your test jokes: 14 of 25 reveal words right (was 9)" |
| **Masked turn** (objective, cheap) | Each held-out joke with its last line hidden: does his reveal word appear in Claude's 3-5? hit@1, hit@5. | Yes |
| **Ablation arms** | Every run scores three arms on the same jokes: no lessons, lessons, lessons + rules. The model id and prompt revision are recorded per row. | In the run's detail |
| **Preference** | The share of thumbs-up among rated candidates; the rank of the first thumbs-up. | Settings |
| **Rule hits** | Suggestions dropped by a rule check, and corrections that repeat an existing standing rule. Both should fall. | Settings |

**Gates (pre-registered before Phase 2's first run).**
- A rule or prompt revision goes live only if the held-out score does not drop (`learning.rs` replay + fence).
- "It is learning" means the lessons arm beats the no-lessons arm on the held-out set, with the model held fixed.
- A gain that also shows in the no-lessons arm is the model, not the lessons.

---

## 10. Privacy

| Rule | How |
|---|---|
| His material never leaves his account | Every new table is in his database. Every route is owner-only (the actor rule). The worker sees one bit plus its retrieved lessons, over the API (ADR 0011). |
| Nothing trains anyone else's model | No export, no shared lessons, no cross-account retrieval. The calls go to Anthropic's API: under its commercial terms, API inputs and outputs are not used to train models by default (**INFERRED; verify against the current terms before Phase 1 ships**). A future own model is trained only on his rows, and its weights are his and private (never in this repo). |
| Voice reasons | Audio stays on the phone and is transcribed on the device only; only the words are sent (CONFIRMED `lib/transcribe.native.ts:110-114,162`). |
| Legal rules | They run on every reason, fix and Talk turn before it is stored, as on every ingest path today. |
| The public repo | Prompts and tests use invented jokes only; the worked examples of his are in his private notes; the commit guard runs (§1 proves it catches his material). |
| App Store 5.1.2(i) | An explicit "send my bits to Claude" consent is needed before any external tester (Q10). |

---

## 11. Build order (smallest first)

| Phase | What | Needs | Proof |
|---|---|---|---|
| **0. Offline baseline (first slice)** | The method block and the breakdown + masked-turn prompts, run by a tool on his desktop against the frontier API (like the T19 proof), over 20-30 of his jokes from his private folder. He marks the cards in a private file. Those marks become the **held-out set**. | Nothing deployed. About $3.50 per run. | Per-part rates with zero lessons; hit@1 / hit@5; measured tokens and cost per call (replaces §8's assumptions) |
| 1. Break it down in the app | `schema_teach.surql` (session, session_turn, breakdown, correction, technique, reveal, rule seeds); the pk-api routes for sessions, breakdowns and reasons (typed); pk-worker `session_plan` + `session_breakdown`; the sheet (S2), line select (S4), card (S5), the score line. **No retrieval yet**: lessons are only collected. | **T8 swap** (pk-worker live) · **pk-api deployed** with the new paths in `OWN_PATHS` · **per-role DB login** for pk-api · his go on the schema (ADR 0009) | Scratch-DB route tests; verbatim drop test (red arm: a paraphrased supporting line must be refused); the owner's on-device check |
| 2. Lessons and rules in the prompt | `plan_vec` / `reason_vec`, the context route, framed lessons, seed rules plus program checks, rule promotion with replay + fence, the Settings score card | Phase 0's held-out set | Held-out run: the lessons arm vs the no-lessons arm |
| 3. Find the turn | `session_turn`, ratings, Write it in → tracked change | Phase 4's suggestion rendering, or Write it in later | Masked-turn hit@5 over time |
| 4. Tighten and Tag it | Server-side line diff (`similar`, a popular crate), suggestions, Keep / Skip, Save version through the proxy, past sessions tied to versions | Phase 1 | Scratch test: kept hunks → the exact version text; a stale base → hunks re-found or reported |
| 5. Talk sessions, Ask cleanup | Old bit chats listed as Talk; "Past questions" on Tonight; the phone input bar retired (Q1) | — | — |
| 6. Wide three panes (web) | `lib/Screen.tsx` at 900 / 1280 | Phases 1-4 | Web checks |
| 7. Voice reasons | A memo target for the session reason | The memo build (item 4) on a device | On-device check |
| 8. iPad, set-list sessions, Claude proposing the jokes in a bit | — | Owner calls Q5 / Q6 / Q7 | — |
| Later (not in scope) | Own open-weight model trained on his cards; blind test vs Claude | Enough rows; his go | Blind test |

---

## 12. Risks

| # | Risk | Mitigation |
|---|---|---|
| R1 | HIG pairs Done with Cancel; here Cancel would throw away kept lines | **Close** keeps everything (the same as swiping down) |
| R2 | The keyboard in a medium sheet covers the bit and can dim it again | Sessions are mostly taps; if it is bad on a device, go to the large detent while typing |
| R3 | Offsets drift (UTF-16 vs UTF-8, synthetic version ids) | The quote check, a 409, real unit ids only |
| R4 | Phase 1 depends on T8 + pk-api deployed + per-role logins | Phase 0 needs none of them and is worth doing first anyway |
| R5 | The LLM judge in the held-out score is biased | Reveal and masked-turn are exact-match; the judge's agreement with him is sampled every run |
| R6 | Lessons fit his last 20 jokes, not his method | The held-out set never feeds retrieval; the ablation arms; technique diversity in retrieval |
| R7 | The daily cap starves sessions or evals | Q3 |
| R8 | Lessons are model-written text put back into prompts (injection) | `frame::data` framing, his quote first, fields escaped (§5.4) |

---

## 13. Questions for the owner (each carries its own context)

1. **The bit page's input bar.** Today a bit's page has its own input bar: a question starts a chat about the bit, and
   a typed line becomes a new version. With sessions, asking moves into the session sheet and adding a line is
   already in Edit. Should the bar go away on the phone, so there are not two chats about one bit on one screen?
2. **"Keep all".** Claude never changes a bit unless you accept it. Is a "Keep all 4" item, tucked in the session's ⋯
   menu and undone with one tap, acceptance enough? Or must every line be kept one at a time?
3. **The daily AI budget.** The cap is $2.00 a day today. One session costs about 7 cents (Opus), so that is roughly
   25-30 a day. One full test run on your 25 test jokes costs about $3.50, more than a whole day. Raise the cap, give
   test runs their own budget, or run tests on Sonnet (about half the price, but then they test a different model)?
4. **Where the score shows.** "Claude got 3 of 5 parts right before you corrected it" can show on the session sheet
   after every breakdown (plus a running %), or only in Settings › What I've learned. Which?
5. **Picking the joke.** Phase 1: you tap the lines of the joke (or take the whole bit). Later, Claude could split the
   bit into its jokes and you tap one, which costs one more small call per bit. Do you want that later step, or is
   tapping lines enough?
6. **Wide screens.** Three panes come first on the web (punchable.io on a laptop). The iPad needs an Apple API that
   is still marked unstable, and the app was never designed for iPad. Web first and iPad later, OK?
7. **Sessions on a set list.** You said sessions belong to a bit "or set". The four modes are about one joke. What
   would you want from a session on a set list: order, bridges, timing, or something else? It is left out of the
   first phases until you say.
8. **The left pane on wide screens.** Group bits by the cloud map's named groups, or by Recent / Performed like the
   Bits tab?
9. **The button's name.** You asked for a "Sessions" button on a bit. The design review suggests **"Work on it"**
   (it says what happens, and reads **"Resume · Tighten"** when one is open). Which label?
10. **Asking you before sending to Claude.** Apple's rule 5.1.2(i) wants an explicit "yes, send my bits to Claude"
    before any third-party AI sees personal data. Today Settings only says it. It does not matter while you are the
    only tester. Add the one-time consent screen now, or only before anyone else tests?
11. **Your test jokes.** Phase 0 needs 20-30 of your jokes that you will label fully and that Claude never sees as
    lessons, so they stay a fair test. Do you want to pick them, or should we pick a spread (different techniques,
    old and new) for you to approve?
12. **Seed rules.** Your four known failures become standing rules on day one: no comparison punchlines, no stock
    targets like wife or mother-in-law, no early reveal, always a concrete scene. Any others to add before the first
    run?
