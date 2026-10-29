# Punchable on iPhone: the Human Interface Guidelines spec

Status: design spec, 2026-10-27. Written against the live HIG pages (cited per rule) and the code at the commit
that shipped TestFlight build 3. Tags: **CONFIRMED** = read on the HIG page or in the code at file:line;
**INFERRED** = a measurement, device figure or library behaviour not read on a page. Sample copy in this document is
invented. Nothing here is a commit, a push or a deploy; the builder brief at the end is the order of work.

HIG base URL: `https://developer.apple.com/design/human-interface-guidelines/` (pages cited as `HIG/<page>`).

---

## 0. The header overflow on the iPhone 17 (fix first)

**Claim: the Tonight header draws under the status bar and its title row is ~45 pt too narrow; both are layout
bugs in our custom header, not a font problem.** The web never showed either: the web's top inset is 0 and the window
is wide.

| # | Cause | Evidence | Tag |
|---|---|---|---|
| 1 | **No safe-area inset.** `Screen` renders `{header \|\| <TopBar …/>}`; only `TopBar` adds `paddingTop: ins.top + 4`. The Tonight header is a plain `View` with `paddingTop: 12`, so the eyebrow date and "Tonight" start 12 pt from the top edge of the screen, under the status bar and the Dynamic Island. | `app/src/lib/ui.tsx` `Screen` (the `header \|\|` branch) and `TopBar` (`paddingTop: ins.top + 4`); `app/src/app/index.tsx:45`. Same defect: `app/src/app/today.tsx:70-71`, `app/src/app/write.tsx:172-173`. `review.tsx`, `list/[id].tsx`, `joke/[id].tsx`, `stage/[id].tsx` pass a `TopBar` and are fine. | CONFIRMED (code) |
| 2 | **Title row over-committed.** One row holds "Tonight" (34 pt `Inter_800ExtraBold`, `flex: 1`, **no `numberOfLines`**) + the "N to sort" pill (≈100 pt at 14 pt text, scaled up to 1.4×) + the 44 pt "+" + `Avatar` = `AskButton` (44 pt min plus the word "Ask", ≈70 pt) + the 44 pt face, with three 8 pt gaps and 20 pt side padding. On a 402 pt wide phone that leaves ≈80 pt for a title that needs ≈125 pt, so it wraps ("Tonig / ht"). Above the Large text size the title is up to 44 pt and the pill 20 pt: worse. | `index.tsx:47-58`; `ui.tsx` `Avatar`; `app/src/lib/AskButton.tsx:12-16`. The width arithmetic is INFERRED (iPhone 17 logical width 402 pt; "Tonight" in Inter ExtraBold 34 pt ≈125 pt). | CONFIRMED (code) / INFERRED (numbers) |
| 3 | **Font fallback is possible but secondary.** `ready = (loaded \|\| !!error) && prefsReady`: on a font-load error the app renders anyway with family names iOS doesn't know, and iOS substitutes the system font silently in a release build. That changes metrics (SF is narrower than Inter ExtraBold) and would make the problem *less* visible, not more. The fonts ship inside `@expo-google-fonts/*`, so a load failure is unlikely. | `app/src/app/_layout.tsx:88-90`; `app/package.json` (`@expo-google-fonts/inter` 0.4.2, `space-grotesk` 0.4.1). | CONFIRMED (code) / INFERRED (iOS fallback) |

HIG rules this breaks (all CONFIRMED on the page):
- "Respecting the safe area is essential to make sure system UI and hardware features like the Dynamic Island don't obstruct content and controls." HIG/layout
- "Support Dynamic Type by adjusting your layout to accommodate text at larger sizes. For example, horizontally adjacent views may need to stack vertically to provide more space for text." HIG/layout
- "When font size increases in a horizontally constrained context, inline items (like glyphs and timestamps) and container boundaries can crowd text and cause truncation or overlapping." HIG/typography
- "Use a large title to help people stay oriented as they navigate and scroll. By default, a large title transitions to a standard title as people begin scrolling the content." HIG/toolbars

**The fix, in two steps.**

*Step A, the patch (build 4, about an hour):*
1. `ui.tsx` `Screen`: when `header` is given, wrap it: `<View style={{ paddingTop: ins.top }}>{header}</View>`, and
   make `TopBar` stop adding `ins.top` itself (it keeps the `+4`). One change fixes Tonight, Today and Write.
2. `index.tsx` header: two rows, the HIG large-title shape. Row 1 (the bar, 44 pt): the eyebrow date on the left,
   `+` and the face on the right. **Drop `AskButton` from this header**: the input bar below *is* Ask (standing
   decision: Ask lives in the input bar, not a button). Row 2: `Tonight` as one `Text` with `numberOfLines={1}` and
   `flexShrink: 1`, then the "12 to sort" pill with `flexShrink: 0`; when `PixelRatio.getFontScale() >= 1.3` the pill
   stacks under the title (HIG/layout: stack vertically at large sizes).
3. Keep `maxFontSizeMultiplier={1.3}` on the title (HIG/typography: Large Title is 34 pt; our 34 pt matches).

*Step B, the real fix (part of the native-navigation rewrite, §11 B3):* the Tonight screen gets a native header with
`headerLargeTitle: true`; the system owns the inset, the collapse on scroll, Dynamic Type and Bold Text. The custom
header becomes content rows.

Acceptance: on the iPhone 17 at text size Large and at xxxLarge, Bold Text on, the date line sits below the status
bar, "Tonight" is one line, nothing is clipped; pull-to-refresh still works; the web is unchanged.

---

## 1. Principles for this app

One comic, often one-handed, often in the dark: backstage five minutes before a set, or on stage with the phone in the
other hand. Standard, familiar, intuitive. Every rule below is the HIG's; the order is ours.

| Principle | HIG rule (verbatim) | Page | In Punchable |
|---|---|---|---|
| Reach | "Place search at the bottom if there's room … since it keeps the search experience easy to reach." | HIG/search-fields | The input bar stays pinned at the bottom. Primary actions go in a bottom toolbar, not the top corners. |
| Targets | "As a general rule, a button needs a hit region of at least 44x44 pt." | HIG/buttons | 44 pt everywhere (first audit); the Stage buttons are 60 pt (`stage/[id].tsx:88-89`), right for a hand that can't look. |
| Legible in the dark | "At a minimum, make sure the contrast ratio between colors is no lower than 4.5:1. For custom foreground and background colors, strive for a contrast ratio of 7:1, especially in small text." | HIG/dark-mode | `theme.ts` documents ratios (`mut2` 6.6:1, `dim` ≥4.6:1). Body text at 7:1. |
| Not colour alone | "Avoid relying solely on color to differentiate between objects, indicate interactivity, or communicate essential information." | HIG/color | Status glyphs ★ ● ○ – beside the dots (`bits.tsx:20`) stay. |
| Familiar | "Place UI in expected locations, use standard symbols to represent common actions, and rely on established conventions for navigation and modality." | HIG/branding | Native tab bar, native headers, SF Symbols, system sheets. Amber is the accent, not the chrome. |
| Branding defers | "Resist the temptation to display your logo throughout your app." / "Minimize its use on controls and instead use it intentionally for primary actions or status indicators, like … an icon for the selected tab in a tab bar." | HIG/branding | No wordmark in headers. Amber on the selected tab, the one primary button, the record ring. |
| Motion is optional | "Make motion optional … supplement visual feedback by also using alternatives like haptics and audio." | HIG/motion | Reduce Motion is honoured (`ui.tsx` `useReduceMotion`). |
| Feedback matches weight | "Warn people when they initiate a task that can cause data loss that's unexpected and irreversible. In contrast, don't warn people when data loss is the expected result of their action." | HIG/feedback | Stop-recording asks (`stage/[id].tsx:112-119`); Remove-from-list doesn't ask, it offers Undo. Keep both. |

---

## 2. Navigation model

**Claim: three native tabs, each with its own native stack; modals are form sheets (short tasks) or full-screen
modals (stage, record); nothing else is modal.**

Rules (CONFIRMED):
- "Use a tab bar to support navigation, not to provide actions." HIG/tab-bars
- "Make sure the tab bar is visible when people navigate to different sections of your app. If you hide the tab bar, people can forget which area of the app they're in. The exception is when a modal view covers the tab bar." HIG/tab-bars. Today the custom bar is hidden on any screen with a `dock` (`ui.tsx` `showNav = !wide && !task && !dock`, so the set list hides the tabs on a push). The rewrite fixes this by construction.
- "They also let people quickly switch between sections of the view while preserving the current navigation state within each section." HIG/tab-bars: one stack per tab.
- "A tab bar floats above content at the bottom of the screen. Its items rest on a Liquid Glass background that allows content beneath to peek through." HIG/tab-bars (iOS)
- "Present content modally only when there's a clear benefit." / "Always give people an obvious way to dismiss a modal view … in iOS … a button in the top toolbar or swipe down." / "Let people dismiss a modal view before presenting another one." HIG/modality
- "Consider using a full-screen modal style for in-depth content or a complex task." HIG/modality: Stage and Record.
- "In an iPhone app, consider supporting the medium detent to allow progressive disclosure of the sheet's content." / "Include a grabber in a resizable sheet." / "Support swiping to dismiss a sheet." / "In iOS and iPadOS, for sheets with a single view, the Cancel button belongs on the leading edge of the top toolbar. When present, the Done button belongs on the trailing edge." HIG/sheets

### 2.1 Route tree (iOS and web share the files; groups keep the URLs)

```
app/src/app/
  _layout.tsx                     root Stack: (tabs) + the modals + login/auth
  (tabs)/_layout.tsx              iOS: NativeTabs; web: the existing shell (Slot)
  (tabs)/(tonight)/_layout.tsx    Stack
  (tabs)/(tonight)/index.tsx      /            Tonight   (large title)
  (tabs)/(tonight)/inbox.tsx      /inbox       push
  (tabs)/(tonight)/say.tsx        /say         push
  (tabs)/(tonight)/settings.tsx   /settings    push
  (tabs)/(tonight)/profile.tsx    /profile     push
  (tabs)/(tonight)/today.tsx, import.tsx, learned.tsx          (kept routes)
  (tabs)/(bits)/_layout.tsx       Stack
  (tabs)/(bits)/bits.tsx          /bits        Bits      (large title + search bar)
  (tabs)/(bits)/joke/[id].tsx     /joke/:id    push (inline title = short name)
  (tabs)/(bits)/merge.tsx         /merge       push
  (tabs)/(bits)/map/index.tsx     /map         push
  (tabs)/(bits)/map/[id].tsx, bit/[id].tsx, notes.tsx, notebook.tsx, desk.tsx, shows.tsx, venue/[id].tsx, calendar.tsx
  (tabs)/(sets)/_layout.tsx       Stack
  (tabs)/(sets)/sets.tsx          /sets        Sets      (large title)
  (tabs)/(sets)/list/[id].tsx     /list/:id    push (inline title = list title, bottom toolbar)
  (tabs)/(sets)/set/[id].tsx, board/[id].tsx, builder/[id].tsx, boards.tsx, myset/[id].tsx, workingset/[id].tsx, setlist/[id].tsx
  stage/[id].tsx                  /stage/:id   fullScreenModal (root)
  record.tsx                      /record      fullScreenModal (root)
  review.tsx                      /review      fullScreenModal (root, kept)
  create.tsx                      /create      formSheet [0.5, 1]   (the "+" sheet)
  add/[board].tsx                 /add/:board  formSheet [1]        (Add a joke / Swap)
  login.tsx, auth/callback.tsx, auth/logout.tsx                  (root, animation none)
```

Route groups `(tabs)`, `(tonight)`, `(bits)`, `(sets)` do not appear in the URL, so every existing deep link
(`/joke/{id}`, `/list/{id}`, `/set/{id}`, `/inbox`, `/map`) keeps working on the web and as a universal link.
CONFIRMED: `expo-router` 58 ships `NativeTabs` (`node_modules/expo-router/build/native-tabs/NativeTabs.d.ts`; import
from `expo-router/native-tabs`, listed in the package `exports`). INFERRED: each tab stack needs
`unstable_settings = { initialRouteName }` so `/joke/x` opened cold has a Back to Bits.

### 2.2 Push vs present

| Screen | iOS presentation | Rule |
|---|---|---|
| Tonight, Bits, Sets | tab roots, `headerLargeTitle: true` | HIG/toolbars large title |
| Bit page, set list, inbox, say, merge, map, settings, profile, set review | `push` with a native header and the system Back | HIG/toolbars: "Use the standard Back and Close buttons … Prefer the standard symbols for each, and don't use a text label that says Back or Close." |
| Create ("+") | `formSheet`, `sheetAllowedDetents: [0.5, 1]`, `sheetGrabberVisible: true`, `sheetInitialDetentIndex: 0` | HIG/sheets medium detent; the menu fits in half, "New set list" expands |
| Add a joke / Swap | `formSheet`, `sheetAllowedDetents: [1]`, grabber on, `sheetExpandsWhenScrolledToEdge: true` | HIG/sheets: "you might not want to support the medium detent if a sheet's content is more useful when it displays at full height" |
| Rename short name, Rename set list, Why (map), Bridge | `formSheet`, `sheetAllowedDetents: 'fitToContents'`, grabber on | HIG/sheets: "requesting specific information from people" |
| Stop recording? | **action sheet** (`ActionSheetIOS` or `@expo/ui` ConfirmationDialog), not a form sheet | HIG/action-sheets: "Use an action sheet — not an alert — to offer choices related to an intentional action." Destructive first, Cancel ("Keep going") last |
| Row and card actions (long press) | **context menu** (`@expo/ui/swift-ui` ContextMenu), not a sheet | HIG/context-menus: "list them at the end of the menu and identify them as destructive" |
| Stage, Record | `fullScreenModal`, `gestureEnabled: false`, `autoHideHomeIndicator: true` | HIG/modality full-screen for "a distinct, multistep task"; nothing swipes you off stage |
| Delete my account | stays a form sheet with the typed confirmation | HIG/alerts confirms destructive; the typed phrase is stronger than the HIG asks and is the owner's call |

Option names `presentation`, `sheetAllowedDetents`, `sheetGrabberVisible`, `sheetInitialDetentIndex`,
`sheetExpandsWhenScrolledToEdge`, `sheetLargestUndimmedDetentIndex`, `sheetCornerRadius`, `unstable_sheetFooter`,
`autoHideHomeIndicator`, `keyboardHandlingEnabled` CONFIRMED in
`node_modules/expo-router/build/react-navigation/native-stack/types.d.ts` (lines 432-822). Presentation values
`'formSheet' | 'pageSheet' | 'fullScreenModal' | 'modal' | 'transparentModal' | …` CONFIRMED in
`node_modules/react-native-screens/lib/typescript/types.d.ts:22`.

---

## 3. Per-screen spec

Common to every screen on iOS: a native header (`headerShown: true`; `headerTintColor: C.amber` colours the buttons
only; `headerTitleStyle` and `headerLargeTitleStyle` left at the system default so SF Pro and Dynamic Type apply);
`contentStyle: { backgroundColor: C.bg }`; the `ScrollView`/`FlatList` gets `contentInsetAdjustmentBehavior="automatic"`
so the large title collapses and content scrolls under the Liquid Glass bar (HIG/layout: "use a scroll edge effect to
visually elevate controls above content"; "extend it underneath sidebars, toolbars, and tab bars"). Pull to refresh via
`RefreshControl` (HIG/progress-indicators: "A refresh control lets people immediately reload content"; "Perform automatic
content updates", which we do by polling). Empty states are one sentence and one action (HIG/tab-bars: "If a section is
empty, explain why its content is unavailable"). Loading shows skeletons, never a centred spinner (HIG/loading: "consider
showing placeholder text, graphics, or animations as content loads"). Errors are a card with "Try again" and no blame
(HIG/writing: "avoid blame, and be clear about what someone can do to fix it").

### 3.1 Tonight (`/`)
- Header: `headerLargeTitle: true`, title "Tonight"; `headerRight` = `+` (`plus`) then the face. The eyebrow date
  ("Friday · set tonight") becomes the first content row (HIG/toolbars: "keep the title under 15 characters long so you
  leave enough room for other controls").
- "12 to sort" becomes a tappable row under the title with a `chevron.right`, not a pill in the bar
  (HIG/lists-and-tables: "A disclosure indicator reveals the next level in a hierarchy").
- The set bar card, the "Ask your material" rows and the "Last set" row are unchanged.
- Input bar: on iOS 26 the tab's `NativeTabs.BottomAccessory` (the MiniPlayer pattern: "For tab bars with an attached
  accessory, like the MiniPlayer in Music, you can choose to minimize the tab bar and move the accessory inline with it
  when a person scrolls down." HIG/tab-bars). `minimizeBehavior: 'onScrollDown'` CONFIRMED in
  `expo-router/build/native-tabs/types.js:57-60`. Fallback below iOS 26: the current absolute dock.
- Haptics: none on load; `selection` when a segment changes.
- Copy (sample): empty set card "No set list yet. Tap + or say "new 7 for Friday"."

### 3.2 Bits (`/bits`)
- Header: `headerLargeTitle: true`; `headerSearchBarOptions: { placeholder: 'Search your bits, or ask', placement:
  'automatic', hideWhenScrolling: false, autoCapitalize: 'none', onChangeText, onSearchButtonPress, onCancelButtonPress,
  tintColor: C.amber }`. Prop names CONFIRMED `react-native-screens/lib/typescript/types.d.ts:846-1020`; `placement`
  values `'automatic' | 'inline' | 'stacked' | 'integrated' | 'integratedButton' | 'integratedCentered'` CONFIRMED line
  43. The system supplies Clear and Cancel (HIG/search-fields: "If possible, start search immediately when a person
  types."). The "Ask your material: …" row appears under the bar when the text is a question (`bits.tsx:67`, unchanged).
- Segmented control Bits · Notes · Map: Map is a navigation, not a view (HIG/segmented-controls: "Don't assign actions to
  segments in a control that otherwise represents selection state"). Make it Bits · Notes and put Map as a `headerRight`
  button (`map`). **Conflict with the approved artboard (three segments); HIG wins.**
- Rows: ≥44 pt scaled, status glyph + dot, short name, one-line sub, tabular time (unchanged). No swipe on this list (a
  bit isn't removable here). Context menu: "Open", "Add to tonight's set", "Rename short name…", "Retire" (destructive,
  last).
- Legend footer stays (colour is never alone).

### 3.3 Sets (`/sets`)
- Header: large title "Sets"; `headerRight` = `+` ("New set list") then the face.
- Segmented Set lists · Performed is fine (HIG/segmented-controls: "Consider a segmented control to switch between closely
  related subviews").
- Paper cards unchanged: content-layer branding is where the HIG wants it ("consider moving it into the content layer,
  where it scrolls beneath Liquid Glass controls", HIG/branding). Context menu on a card: "Open", "Go on stage",
  "Rename…", "Duplicate", "Delete" (destructive; confirm with an action sheet because it isn't undoable:
  HIG/pull-down-buttons "When people choose a destructive action, the system displays an action sheet").
- "Found for you" rows with disclosure indicators (unchanged).

### 3.4 Set list (`/list/{id}`)
- Header: inline `headerTitle` = the list title; `headerBackButtonDisplayMode: 'default'` shows "Sets" (values
  `'default' | 'generic' | 'minimal'` CONFIRMED `react-native-screens … types.d.ts:21`). `headerRight`: a
  `Stack.Toolbar.Menu` (`ellipsis.circle`) with "Rename…", "Duplicate", "Share…" (`square.and.arrow.up`, the system
  share sheet with the plain-text list: HIG/activity-views "Use the Share button to display an activity view"), "Delete"
  (destructive, last); and a `Stack.Toolbar.Button` "Stage" (`play.fill`) with `variant: 'prominent'` ("Use the
  .prominent style for key actions … Only specify one primary action, and put it on the trailing side of the toolbar",
  HIG/toolbars). `variant` values `'plain' | 'done' | 'prominent'` CONFIRMED (native-stack `types.d.ts:878`).
- The in-content "Rename" link beside the title goes (the title is in the bar; the menu renames). The runtime line
  "6:40 / 7:00 · 20 s left" and the progress bar stay as the first content row.
- Bottom toolbar: `Stack.Toolbar placement="bottom"` with one prominent button "Add a joke · 20 s left" (`plus`).
  `placement` values `'left' | 'right' | 'bottom'` CONFIRMED (`layouts/stack-utils/toolbar/StackToolbarClient.d.ts:18-22`).
  The tab bar stays visible beneath (HIG/tab-bars).
- Rows: tap opens the bit; drag by the grip after a long press (`DragList.native.tsx`, unchanged; haptic `light` on
  lift, `selection` on drop: "Selection haptics provide feedback while the values of a UI element are changing",
  HIG/playing-haptics). Trailing swipe: Remove (red, full swipe runs it, Undo offered) + Swap (`SwipeRow.native.tsx`,
  unchanged). Context menu: "Open the bit", "Swap…", "Rename short name…", "Move to top", "Move to bottom", separator,
  "Remove from this list" (destructive). Ellipses where more input follows (HIG/menus: "Append an ellipsis to a menu
  item's label when the action requires more information before it can complete").
- The per-row "…" button goes once the context menu exists (HIG/context-menus: "Provide either a context menu or an edit
  menu for an item, but not both"); keep the row's `accessibilityActions`, the non-gesture path ("Offer alternatives to
  gestures", HIG/accessibility).
- Gap rows ("Gap here · add a bridge") unchanged.

### 3.5 Add a joke / Swap (`/add/{board}`, form sheet)
- Native sheet at the large detent, grabber visible; the sheet's own bar: Cancel leading, the title ("Add a joke · 20 s
  left"), no Done (a tap on + is the write). HIG/sheets: "If you provide a Done button, always pair it with a Cancel
  button"; Cancel alone is allowed.
- A plain `TextInput` search at the top of the sheet (`headerSearchBarOptions` does not apply inside a form sheet,
  INFERRED). Filter chips unchanged. Rows with a trailing 44 pt `+`.
- A route can't take `onPick` as a param, so the sheet writes the board op itself and the set list re-reads on focus
  (`useFocusEffect` + `b.reload()`); a `swap` param carries `at` and the old joke id.

### 3.6 Bit page (`/joke/{id}`)
- Header: inline title = the short name; Back shows the previous screen's title ("Bits" or the list title) with no
  custom label. `headerRight`: "Edit" as a text toolbar button ("except for actions like edit that aren't
  well-represented by symbols", HIG/toolbars) and a menu (`ellipsis.circle`): "Rename short name…", "Add to tonight's
  set", "Pick a set list…", "Merge with…", "Share…", "Retire" (destructive).
- Content: the full title as the first row (Title 1, 28 pt), the stage word, versions as a horizontal chip row (keep:
  HIG/segmented-controls caps iPhone segments at about five and versions can exceed that), the text card, Punch up ·
  Riff · To a set, the map.
- Its input bar: the bottom accessory, placeholder "Ask about Kale, or add a line".
- Haptic: `success` **after** "To a set" completes, not before (today `joke/[id].tsx:102` fires it before the write;
  HIG/playing-haptics Success "Indicates that a task or action has completed").

### 3.7 Merge review (`/merge`)
- Push, inline title "Merge"; "2 of 7" as the first row; the two pieces as cards with a radio each (unchanged). One
  prominent button "Merge into one bit" (HIG/buttons: "Keep the number of prominent buttons to one or two per view");
  "Not the same" and "Make it a tag" plain.
- Undo in the toast after a merge (kept; see §10).

### 3.8 Inbox (`/inbox`)
- Push, inline title "Inbox"; "Since last night · 3 filed · 2 need a call" as the first row.
- One card at a time with the four buttons ("Add as a new version" prominent, the rest plain), labelled leading and
  trailing swipes (kept), a context menu on long press replacing the sheet.
- "Filed for you" rows with an Undo text button on the right (kept).
- Audio: the memo player plays under the playback category; recording a set uses `doNotMix` (HIG/playing-audio: "don't
  make people stop listening to music from another app if you don't need to" applies to playback, not to recording).

### 3.9 Stage (`/stage/{id}`, full-screen modal) and Record (`/record`): the record control (2026-10-29)
- `autoHideHomeIndicator: true`, `gestureEnabled: false`, `useKeepAwake` (kept). Title "Record"; Close top-leading.
- **Nothing records until he taps the red disc** (`lib/RecordControl`): on open the clock reads 0:00, status "Ready",
  the set list in marker text. Next bit before the disc runs the clock alone ("Clock only").
- The control: an 80 pt red disc (systemRed, `C.rec` fallback) in a 92 pt ring, centred in the dock, above the home
  indicator (HIG/buttons: 44 pt minimum; one-handed in the dark). Tap → recording, `haptics.medium()` (impact).
  While recording the disc is a 30 pt red square: **tap = stop AND save**, no sheet (HIG/feedback: do not warn when
  the result is expected; Voice Memos does the same). Pause is the 60 pt plain control on the left ("Paused" while
  paused or during a call; the disc then reads Resume); Next bit (Stage) / Mark (Record) is the 60 pt control on the
  right, prominent on Stage. `success` after the upload is queued.
- Discard is the hard path: Close while taping → action sheet "Leave the stage?" · "Discard the recording"
  (destructive, first) · "Save the set" · "Keep going" (cancel, last) (HIG/modality: closing a view that could lose
  user content presents an action sheet with a save option). On the web the same as a sheet.
- One clock (`RecordClock`): 84 pt tabular, `maxFontSizeMultiplier 1.2`, the status word beside it ("Recording" in
  red, "Paused by a call" during an interruption), the target bar with the light mark, the room's level while taping.
- The state machine is pure (`recorderLogic.nextRecordState`, tested): idle → recording → paused ⇄ recording → saving.
- Sets tab: segments **Set lists · Recorded** (equal-width nouns, HIG/segmented-controls; the count in a header row);
  the list screen's prominent action is **Record** with `record.circle`, never `play.fill` (play means playback).
- Audio session: record category, `doNotMix`, background recording (kept, `lib/recorder.native`).

### 3.10 All-bits map (`/map`): the galaxy (built 2026-10-28, `lib/Galaxy.tsx`)
- **The cloud (default since 2026-10-29, `lib/Cloud.tsx`, `GET /api/map/cloud`)**: every bit placed by what it SAYS: a
  3-axis PCA of its bge-small vector (the mean of its versions', or the typed idea's own), k-means groups named by
  their titles' words. A slow turn about the vertical axis until he touches (off under Reduce Motion; HIG/motion:
  motion is never the only signal and stops on request), depth as size and fade (near = bigger, brighter), pinch and
  pan (lib/PanZoom), a tap selects the nearest dot (a card: Open · To a set), group names float at their centres, the
  legend's chips turn a group to the front. Turn left/right buttons (44 pt) for a no-gesture path. "Links" switches to
  the galaxy below, where nearness means a stored link; the choice is remembered (prefs.mapMode).
- Push, inline title "Map"; `Stack.Toolbar` right: Shuffle (`shuffle`), Centre (`scope`, "Centre on the selected
  bit"), List (`list.bullet`, the map as a SectionList grouped by cluster for VoiceOver); the header search field
  ("Find a bit") on the phone, a field above the map on the web. Pan + pinch (`lib/PanZoom`), double-tap zooms.
- Colour = which bits are alike (cluster tokens `cluster1…6` in `theme.ts`, light and dark, all ≥ 4.5:1); status =
  shape only (★ ● ○ –); links keep `GROUP_COLOR` plus a dash per kind so colour is never alone (HIG/color). Amber is
  the focus ring only. Labels 12 pt in a pill (`maxFontSizeMultiplier` 1.2), never overlapping; the card and the
  legend scale fully; the legend goes to one column at font scale ≥ 1.3.
- Touch-and-hold a labelled bit = the system context menu (`RowMenu`/`Link.Menu`: Open · To a set… · Merge with… ·
  Rename short name…); a dot with no label = the action sheet; the web and Android = `MenuSheet`. The card keeps a
  visible path (Open · To a set · Merge · Done) and swipes down to dismiss. Haptics: selection on focus only. Reduce
  Motion: no tween on Shuffle, Centre or Find.

### 3.11 Settings (`/settings`)
- Push, large title "Settings" (the in-content "Settings & privacy" h1 goes).
- Grouped list: This device (Appearance System / Dark / Light as a segmented control; Haptics as a switch in a row:
  HIG/toggles "Use the switch toggle style only in a list row"), Connections, Where your material goes, AI provider
  (radio rows), Controls (switches), the links, Export / Delete.
- The in-app appearance override is allowed ("a custom settings area … to offer general settings that affect your
  overall experience, like interface style", HIG/settings) and a standing owner decision. The haptics toggle is required
  ("Make haptics optional", HIG/playing-haptics). Account deletion is required and present ("If you help people create
  an account … you must also help them delete it", HIG/managing-accounts).

### 3.12 Create ("+", form sheet at [0.5, 1])
- Rows Bit · Set list · Record a set · Voice memos; the YouTube field; "Or just say it". Cancel leading. "Set list"
  expands to the large detent (programmatic detent change is INFERRED to need a re-mount with
  `sheetInitialDetentIndex: 1`; otherwise the grabber lets him drag).
- Keyboard: `keyboardType="url"` and `"number-pad"` kept (HIG/virtual-keyboards: "Choose a keyboard that matches the
  type of content people are editing"); `returnKeyType="go"` on the link field ("Consider customizing the Return key
  type if it helps clarify the text-entry experience").

### 3.13 The conversation (`/say`) and New bit (`/new-bit`)
- `/say`: push, inline title "Asking"; the thread as now; the input bar as the bottom accessory. The spinner with
  "Reading your material…" is an acceptable indeterminate indicator for a short wait (HIG/progress-indicators); the
  "The writer is offline" card stays (HIG/feedback: status near the item it describes).
- Bits "+" (2026-10-29) opens **`/new-bit`**, a form sheet at `fitToContents`: "Premise" (the short name he'd write on
  a set list, autoFocus) and an optional "First line"; Cancel leading, **Add** trailing (`variant: 'done'`; HIG/sheets:
  a Done button is always paired with Cancel). Not the conversation: the agreed unit is a named premise
  (docs/HANDOVER.md "organizing ideas"). Tonight's "+ Bit" still opens the composer.
- Bits sort (Recent · Performed · Strongest, default Recent) is a toolbar pop-up labelled with the current choice
  (HIG/pop-up-buttons), not a segmented control (segments are for views: Bits · Notes).

---

## 4. Component kit mapping

| HIG component | Page | Punchable today | Build with | Exact options (CONFIRMED in node_modules unless noted) |
|---|---|---|---|---|
| Tab bar | HIG/tab-bars | custom `BottomNav` (`ui.tsx`), SVG icons, 64 pt | `NativeTabs` from `expo-router/native-tabs` | `<NativeTabs tintColor={C.amber} minimizeBehavior="onScrollDown" backBehavior="history">`; `<NativeTabs.Trigger name="(tonight)"><NativeTabs.Trigger.Label>Tonight</NativeTabs.Trigger.Label><NativeTabs.Trigger.Icon sf="moon.stars.fill" /></NativeTabs.Trigger>`; props `tintColor`, `iconColor {default, selected}`, `labelStyle`, `minimizeBehavior 'automatic' \| 'never' \| 'onScrollDown'`, `blurEffect`, `disableTransparentOnScrollEdge`, per-trigger `badgeValue`, `hidden`, `role: 'search'` (`native-tabs/types.d.ts`) |
| Toolbar / navigation bar | HIG/toolbars | custom `TopBar` | `Stack.Screen` options + `Stack.Toolbar` | `headerLargeTitle`, `headerLargeTitleShadowVisible`, `headerTransparent`, `headerBlurEffect`, `headerTintColor`, `headerTitle`, `headerBackTitle`, `headerBackButtonDisplayMode`, `headerSearchBarOptions`, `headerRight`, `unstable_headerRightItems`; `<Stack.Toolbar placement="right"><Stack.Toolbar.Button icon="plus" onPress variant="plain" /><Stack.Toolbar.Menu icon="ellipsis.circle"><Stack.Toolbar.MenuAction title="Rename…" icon="pencil" onPress /><Stack.Toolbar.MenuAction title="Delete" destructive /></Stack.Toolbar.Menu></Stack.Toolbar>` (`layouts/stack-utils/toolbar/*.d.ts`) |
| Large title | HIG/toolbars | `largeTitle` prop in `Screen` (JS collapse) | `headerLargeTitle: true` + `contentInsetAdjustmentBehavior="automatic"` | as above |
| Search field | HIG/search-fields | custom pill `bits.tsx:87-96` | `headerSearchBarOptions` | `placeholder`, `placement`, `hideWhenScrolling`, `obscureBackground`, `autoFocus`, `cancelButtonText`, `onChangeText`, `onSearchButtonPress`, `onCancelButtonPress`, `tintColor`, `textColor`, `barTintColor` |
| Sheet | HIG/sheets | RN `Modal` + hand-rolled grabber (`ui.tsx` `Sheet`) | a route with `presentation: 'formSheet'` | `sheetAllowedDetents: number[] \| 'fitToContents'`, `sheetGrabberVisible`, `sheetInitialDetentIndex: number \| 'last'`, `sheetLargestUndimmedDetentIndex`, `sheetCornerRadius`, `sheetExpandsWhenScrolledToEdge`, `unstable_sheetFooter`, `keyboardHandlingEnabled` |
| Action sheet | HIG/action-sheets | `Sheet` with buttons (`stage/[id].tsx:112`) | `ActionSheetIOS.showActionSheetWithOptions` (RN core) or `@expo/ui/swift-ui` `ConfirmationDialog` | `options`, `destructiveButtonIndex`, `cancelButtonIndex`, `title`, `message` (standard RN API; INFERRED, not read here) |
| Alert | HIG/alerts | none needed | `Alert.alert` (RN core) | title ≤2 lines; buttons `style: 'cancel' \| 'destructive'` |
| Context menu | HIG/context-menus | `MenuSheet` on long press | `@expo/ui/swift-ui` `ContextMenu` (installed, `@expo/ui` 58.0.23; API INFERRED, read `node_modules/@expo/ui/build/swift-ui/ContextMenu` before use) | destructive item last; keep `accessibilityActions` on the row |
| Menu / pull-down | HIG/menus, HIG/pull-down-buttons | "…" buttons opening a sheet | `Stack.Toolbar.Menu` (header) | `title`, `icon` (SF name), `destructive`, `disabled`, `inline`, `subtitle` |
| Lists | HIG/lists-and-tables | `FlatList` + custom rows | keep `FlatList`; disclosure via `SymbolView chevron.right` | row ≥44 pt scaled (`bits.tsx:22`) |
| Swipe actions | HIG/accessibility (gestures need alternatives) | `SwipeRow.native.tsx` | keep (labelled, full swipe on the main action, back-swipe edge respected at 24 pt: `SwipeRow.native.tsx:49`) | — |
| Segmented control | HIG/segmented-controls | custom `Seg` (`ui.tsx`) | keep `Seg`, or `@expo/ui/swift-ui` `Picker` segmented (INFERRED name) | ≤5 segments, nouns, no actions |
| Text field | HIG/text-fields | RN `TextInput` | keep; add `clearButtonMode="while-editing"` ("Display a Clear button in the trailing end of a text field") | `keyboardType`, `returnKeyType`, `textContentType` |
| Button | HIG/buttons | `Btn` (`ui.tsx`) | keep for content; toolbar buttons native | one prominent per view |
| Toggle | HIG/toggles | RN `Switch` in rows | keep | only in list rows |
| Progress | HIG/progress-indicators | `Skeleton`, `RefreshControl`, `ActivityIndicator` | keep | a determinate bar for the upload queue (the count is known) |
| Activity view | HIG/activity-views | none | RN `Share.share` | from the list menu "Share…" |
| SF Symbols | HIG/sf-symbols | SVG path map `ui.tsx` `P` | `SymbolView` from `expo-symbols` (58.0.4) | `name`, `size`, `weight 'regular' \| 'semibold' \| 'bold' …`, `scale 'small' \| 'medium' \| 'large'`, `type 'monochrome' \| 'hierarchical' \| 'palette' \| 'multicolor'`, `tintColor`, `fallback` (`SymbolModule.types.d.ts`) |
| Liquid Glass (custom) | HIG/materials | `expo-glass-effect` installed, unused | only the floating input bar (navigation layer), never a card | `<GlassView glassEffectStyle="regular" isInteractive colorScheme="auto">`; `isLiquidGlassAvailable()` guard (`expo-glass-effect/build`) |
| Haptics | HIG/playing-haptics | `lib/haptics.ts` | keep | `selection` (values changing), impact `light`/`medium` (lift, snap), notification `success`/`warning`/`error` (outcomes only) |
| Pull to refresh | HIG/progress-indicators | `RefreshControl` amber | keep; consider dropping the custom `tintColor` under the glass bar (INFERRED preference) | — |

Symbol choices: tabs `moon.stars.fill` (Tonight), `square.grid.2x2.fill` (Bits), `list.bullet.rectangle.portrait.fill`
(Sets); actions `plus`, `ellipsis.circle`, `magnifyingglass`, `mic.fill`, `waveform`, `play.fill`, `stop.fill`,
`record.circle`, `pencil`, `arrow.triangle.swap`, `arrow.up.to.line`, `arrow.down.to.line`, `trash`,
`line.3.horizontal` (grip), `chevron.right`, `square.and.arrow.up`, `person.crop.circle`, `gearshape`, `map`,
`text.bubble`, `arrow.down.left.and.arrow.up.right`. Names are INFERRED to exist in SF Symbols 7; `sf-symbols-typescript`
(a dependency of expo-router's native tabs) type-checks them. "Prefer filled symbols or icons for consistency with the
platform" in tab bars; "The outline variant works well in toolbars, lists" (HIG/tab-bars, HIG/sf-symbols).

---

## 5. Appearance

Rules (CONFIRMED):
- "Ensure that your app looks good in both appearance modes." HIG/dark-mode
- "Prefer the system background colors. Dark Mode is dynamic, which means that the background color automatically changes from base to elevated when an interface is in the foreground, such as a popover or modal sheet." HIG/dark-mode
- "Even if your app ships in a single appearance mode, provide both light and dark colors to support Liquid Glass adaptivity in these contexts." HIG/color
- "Don't use Liquid Glass in the content layer … use standard materials for elements in the content layer, such as app backgrounds." / "Use Liquid Glass effects sparingly. Standard components from system frameworks pick up the appearance and behavior of this material automatically." HIG/materials
- "Reduce the use of toolbar backgrounds and tinted controls … use the content layer to inform the color and appearance of the toolbar." HIG/toolbars
- "Apply color sparingly to the Liquid Glass material, and to symbols or text on the material … To emphasize primary actions, apply color to the background rather than to symbols or text." HIG/color
- "Soften the color of white backgrounds." HIG/dark-mode (the paper cards are `#F2EEE4`, already off-white).

Token mapping (`app/src/lib/theme.ts`, values CONFIRMED):

| Role | Dark | Light | HIG counterpart | Note |
|---|---|---|---|---|
| ground `bg` | `#0B0B0D` | `#f6f6f4` | systemBackground (base) | the owner's approved ground; keep |
| `surface` / `card` | `#161618` / `#1C1C1F` | `#ffffff` | secondary / elevated background | native sheets get the system elevated background; our card tokens stay in content |
| text / `mut2` / `dim` | `#f3f4f6` / `#A1A1A8` / `#878d97` | `#111214` / `#5d6169` / `#6b7079` | label / secondaryLabel / tertiaryLabel | ratios documented in the file |
| accent `amber` | `#FFB340` | `#b47a0e` (mark); text uses `amberDeep` | tint | `headerTintColor` and `NativeTabs tintColor`; never a bar background (HIG/branding; HIG/tab-bars colour rule) |
| destructive `red` | `#FF453A` | `#b91c1c` | systemRed | matches systemRed dark (INFERRED) |
| separators `border` / `hairline` | `#2d3139` / `#1F1F22` | `#d9d9d4` / `#e4e4df` | separator | `bits.tsx:43` hardcodes `#1F1F22`; use `C.hairline` |
| status dots | `#30D158`, `#64D2FF` (hardcoded) | none | systemGreen / systemCyan | move into tokens with light variants (`bits.tsx:17`, `map/index.tsx:22`) |

Where glass applies: the native tab bar, native headers and native form sheets (automatic). The floating input bar is a
navigation-layer control, so it may use `GlassView regular` on iOS 26 with the amber record ring on top. Cards, the paper
set lists and the stage clock stay flat (content layer). Nothing else gets glass.

The System / Dark / Light override stays (owner decision; HIG/settings lists "interface style" as a legitimate in-app
setting). `GlassView colorScheme` must follow `C.scheme` so an in-app Light override doesn't leave dark glass.

---

## 6. Typography

Rules (CONFIRMED, HIG/typography): default body 17 pt, minimum 11 pt on iOS; "Consider using the built-in text styles";
"If you use a custom font, make sure it implements the same behaviors" (Dynamic Type, Bold Text); "Keep text truncation to
a minimum as font size increases"; "In general, avoid light font weights". HIG/branding: "It can work well to use a custom
font for headlines and subheadings while using system fonts for body copy and captions."

| SF text style (Large) | pt / leading | Punchable style | File | Verdict |
|---|---|---|---|---|
| Large Title | 34 / 41, Bold emphasized | `F.black` 34, ls −0.7 (`index.tsx:48`; `ui.tsx` `largeTitle`) | custom header | → the system large title via `headerLargeTitle`; Inter ExtraBold stays for in-content headings |
| Title 1 | 28 / 34 | `h1` Space Grotesk 700 30/35 | `ui.tsx` `tv.h1` | content headings; the bit title 30 → 28 |
| Title 2 | 22 / 28 | — | | "Ask your material" (now 20 bold) |
| Title 3 | 20 / 25 | `h2` SG 600 20/26 | | ok |
| Headline | 17 / 22 Semibold | `semi` Inter 600 17/24 | | ok |
| Body | 17 / 22 | `body` Inter 400 17/24 (`BODY = 17` on iOS, `ui.tsx:22`) | | ok |
| Subhead | 15 / 20 | row subs at 15 | | ok |
| Footnote | 13 / 18 | `small` 13/18, `mut` 14/20 | | ok |
| Caption 1 / 2 | 12 / 16, 11 / 13 | `eb` 12, badges 11 | | 11 is the floor; nothing below in the screens read |

Dynamic Type: `T` caps at 1.6× (`ui.tsx:35`); rows scale with `PixelRatio.getFontScale()` (`bits.tsx:22`); the stage
clock caps at 1.2× (a clock, not prose). The native header and tab bar scale themselves. Bold Text: custom fonts don't
respond; INFERRED that `AccessibilityInfo.isBoldTextEnabled()` can switch `F.body` → `Inter_600SemiBold` and `F.semi` →
`Inter_700Bold` at start, next to the Reduce Motion read in `ui.tsx`.

The header overflow fix is §0. Rule of thumb from it: **no row may contain a large-title `Text` and more than one
fixed-width control**; titles get `numberOfLines={1}` and trailing items `flexShrink: 0`; rows re-stack at font scale
≥1.3.

---

## 7. Accessibility checklist

| Item | Rule (HIG/accessibility unless noted) | In code | Action |
|---|---|---|---|
| 44 pt targets | "Strive to meet the recommended minimum control size" (44×44 pt default on iOS) | done (`Btn`, `IconBtn`, `AddBtn`, pills) | the map's "Fit" link at `minHeight: 24` (`map/index.tsx:125`) → 44 |
| VoiceOver labels | "Describe your app's interface and content for VoiceOver" | rows, grips, swipe actions, cards labelled | native tabs and headers come labelled; add a label to the face ("Your profile") |
| Gestures have alternatives | "Offer alternatives to gestures … also make a button available" | grip has Move up/down; swipe rows expose `accessibilityActions`; the stage has a Next button | context menus must keep a visible path (header menu or row actions) |
| Dynamic Type | "support font size enlargement … by adopting Dynamic Type" | capped scaling; rows grow | §6; test at xxxLarge and AX1 |
| Reduce Motion | "ensure your app … responds by reducing automatic and repetitive animations" | `useReduceMotion` fades | native sheets honour it themselves |
| Increase Contrast | "provide a higher contrast color scheme when … Increase Contrast is turned on" | not handled | INFERRED: no iOS Increase Contrast query in the installed RN; raise `mut2` to `text2` if an API exists (check `AccessibilityInfo`) |
| Colour not alone | "Convey information with more than color alone" | glyphs and words | keep the legend |
| Timed elements | "Minimize use of time-boxed interface elements … Prefer dismissing views with an explicit action." | the toast auto-dismisses at 4–8 s (`ui.tsx` `showSnack`) | add a non-timed Undo path: the inbox has it; the set-list menu gains "Undo last change" |
| Keyboard | "Use the keyboard layout guide" (HIG/virtual-keyboards) | `KeyboardAvoidingView` around screens and sheets | native form sheets handle it; keep KAV on stack content |
| Bold Text | HIG/typography custom-font rule | not handled | §6 |
| Haptics optional | "Make haptics optional" (HIG/playing-haptics) | Settings toggle | keep |

---

## 8. App icon and launch screen

- Icon: a layered Icon Composer `.icon` with default, dark, clear and tinted appearances ("You can design app icon
  variants for every appearance variant, and the system automatically generates variants you don't provide"; "Keep your
  icon's features consistent across appearances"; "Include text only when it's essential to your experience or brand";
  "provide square layers so the system can apply rounded corners", HIG/app-icons). The 4-bar meter mark
  (`app/src/lib/Brand.tsx`) is a simple shape with no text: right. Today `app/app.json` ships one `icon.png`. INFERRED:
  Expo SDK 58 accepts an Icon Composer bundle through `ios.icon` (verify in the SDK docs); the fallback is
  `ios.icon: { light, dark, tinted }` PNGs.
- Launch screen: "Design a launch screen that's nearly identical to the first screen of your app"; "If your app or game
  displays a solid color before transitioning to the first screen, create a launch screen that displays only that solid
  color"; "Avoid including text on your launch screen"; "Don't advertise … don't include logos or other branding elements"
  (HIG/launching). Today: `expo-splash-screen` with `splash-icon.png` at 160 px on `#111214` (`app/app.json` plugins),
  while the app's ground is `#0B0B0D`: a visible step at launch. → background `#0B0B0D`, no image, plus a dark/light pair
  so the splash "matches the device's current orientation and appearance mode" (`expo-splash-screen` supports a `dark`
  block; exact key INFERRED). **Conflict:** the mark on the splash is a branding wish; HIG says no. HIG wins; the mark
  lives on the icon.

---

## 9. Writing

Rules (CONFIRMED, HIG/writing): "Choose simple, plain language"; "When labeling buttons and links, it's almost always best
to use a verb"; "Adopt capitalization rules that align with your app's style, then apply them consistently"; errors
"avoid blame, and be clear about what someone can do to fix it"; "Interjections like 'oops!' or 'uh-oh' are typically
unnecessary". HIG/alerts: "Always use the title 'Cancel' to title a button that cancels an alert's action"; "Aim for a
one- or two-word title that describes the result of selecting the button"; no "OK" except informational, no "Yes"/"No".
HIG/menus: "Remove articles like a, an, and the from menu-item labels"; the ellipsis rule.

Punchable's voice is sentence case and plain ("Add a joke", "Not a joke", "Keep going"). Decision: **sentence case
everywhere, including native menus and action sheets**, because the HIG permits either if applied consistently and the
owner's plain-language rule is a standing decision. Hard HIG rules kept: the cancel role is "Cancel" or a verb that cancels
("Keep going"); destructive items last and red; no "OK"; no "Yes/No"; ellipsis when more input follows.

Sample copy (invented):

| Where | Copy |
|---|---|
| Tonight, empty set | "No set list yet. Tap + or say "new 7 for Friday"." |
| Bits, empty | "No bits yet. Say one." |
| Set list, full | "Full. Swap a joke or cut one." |
| Stop recording (action sheet) | title "Stop recording?" · message "6:12 on tape. Save it and it gets reviewed after the show." · "Discard the recording" (destructive) · "Save the set" · "Keep going" (cancel) |
| Remove toast | "Removed Kale" · Undo |
| Can't reach | "Can't reach Punchable. Your bits stay on this phone and go up when it's back." · Try again |
| Context menu, set-list row | Open the bit · Swap… · Rename short name… · Move to top · Move to bottom · — · Remove from this list |

---

## 10. Conflicts resolved

| Conflict | HIG says | Owner / artboard says | Resolution |
|---|---|---|---|
| Material snackbar with Undo | no such component; undo via shake or three-finger swipe; "Prefer dismissing views with an explicit action" | the toast + Undo is the established pattern | **Keep the toast**: there is no HIG transient confirmation, and alerts for undoable actions are discouraged ("Avoid displaying alerts for common, undoable actions, even when they're destructive", HIG/alerts). One action max, 8 s with Undo, plus a non-timed Undo path (§7). Accepted deviation. |
| Bits · Notes · Map segments | segments are selection, not actions | the artboard has three segments | Map becomes a header button; two segments remain. HIG wins. |
| "…" row button + long press | one of context menu or edit menu, not both | the audit added both for discoverability | native context menu + the row's VoiceOver actions; the "…" goes. HIG wins; keep the hint line under the list for one release. |
| Splash with the mark | no logos on the launch screen | brand wish | HIG wins (§8). |
| Appearance override in Settings | avoid redundant copies of system settings, but "interface style" is a listed in-app setting | standing decision | Keep. Not a conflict on a close read. |
| Sentence case in native menus | "To be consistent with platform experiences, use title-style capitalization" (HIG/menus) | plain language, sentence case | Keep sentence case for consistency ("apply them consistently", HIG/writing). Accepted deviation. |
| Custom fonts in the bar | the system draws the large title in SF | Inter ExtraBold headers in the artboards | Native large titles are SF; Inter and Space Grotesk stay for content headings (HIG/branding). The owner loses the ExtraBold bar title; gains Dynamic Type, Bold Text, collapse and the safe area for free. |
| Tabs hidden under the "Add a joke" dock | the tab bar stays visible on pushes | the dock design | a bottom toolbar inside the tab's stack; the tabs remain. HIG wins. |

---

## 11. Builder brief (ordered by value to the owner's first week on TestFlight)

Each item: files · exact options · what stays on web · acceptance · risk. The builder commits; this spec does not.
Target: iOS 26 on the iPhone 17. Nothing below needs a native module beyond what `app/package.json` lists (`expo-router`
58.0.26, `react-native-screens` 4.30.0, `expo-symbols` 58.0.4, `expo-glass-effect` 58.0.5, `@expo/ui` 58.0.23,
`expo-haptics`), so an EAS build, not Expo Go.

### B1. Header overflow patch (build 4, today)
- Files: `app/src/lib/ui.tsx` (`Screen`: wrap a custom `header` in `paddingTop: ins.top`; `TopBar`: drop its own
  `ins.top`), `app/src/app/index.tsx:44-61` (two-row header, no `AskButton`, `numberOfLines={1}`, `flexShrink`, re-stack
  at font scale ≥1.3). `today.tsx:70` and `write.tsx:172` inherit the wrapper; no change.
- Web: unchanged (inset 0).
- Acceptance: §0.
- Risk: low. One wrapper `View`; the e2e tests target testIDs, not layout.

### B2. Native tabs with a stack per tab
- Files: new `app/src/app/(tabs)/_layout.tsx` (`Platform.OS === 'web'` → `<Slot />` inside the existing shell; native →
  `NativeTabs`); new `(tonight)/_layout.tsx`, `(bits)/_layout.tsx`, `(sets)/_layout.tsx` (each a `Stack` with
  `screenOptions={{ headerTintColor: C.amber, contentStyle: { backgroundColor: C.bg } }}`); move the route files per §2.1
  (`git mv`, no content change); `app/src/app/_layout.tsx` (root `Stack`: `(tabs)`, `stage/[id]`, `record`, `review`,
  `create`, `add/[board]`, `login`, `auth/*`); `app/src/lib/ui.tsx` → split `Screen.tsx` (web, as is) and
  `Screen.native.tsx` (no `TopBar`, no `BottomNav`, `ScrollView contentInsetAdjustmentBehavior="automatic"`, bottom
  padding from the tab bar height; INFERRED there is no hook for the native tab bar height, so pad by the content inset
  the system applies). `rootOf` / `PUSHED_ROOT` are web-only after this.
- Options: `<NativeTabs tintColor={C.amber} minimizeBehavior="onScrollDown" backBehavior="history">`; triggers
  `name="(tonight)" | "(bits)" | "(sets)"` with `Label` + `Icon sf` (§4); `badgeValue` on Tonight = the to-sort count,
  only when >0 ("Reserve badges for critical information so you don't dilute their impact", HIG/tab-bars).
- Web: the three-tab `BottomNav` and `Sidebar` stay exactly as they are.
- Acceptance: switching tabs preserves each tab's stack; `/joke/x` from a universal link lands in Bits with Back; the tab
  bar is visible on the set list; scrolling Bits minimises the bar; VoiceOver reads "Tonight, tab, 1 of 3".
- Risk: medium. `Screen`'s contract (`header`, `dock`, `bar`, `task`) needs the native variants in B3 and B4; do B2
  and B3 on one branch.

### B3. Native stack headers (large titles, system Back, search bar, toolbar items)
- Files: each tab `_layout.tsx` (`Stack.Screen` options); `bits.tsx` (`headerSearchBarOptions`; the custom pill becomes
  web-only); `list/[id].tsx` (`Stack.Toolbar placement="right"` menu + prominent "Stage"; `placement="bottom"` prominent
  "Add a joke"; delete the in-content Rename link and the per-row "…"); `joke/[id].tsx` (`headerTitle` = short name,
  "Edit", the menu); `sets.tsx`; `index.tsx` (header content becomes rows); `settings.tsx` (drop the h1); `ui.tsx`
  (`TopBar` web-only).
- Options: `headerLargeTitle: true` on the three roots; `headerLargeTitleShadowVisible: false`;
  `headerBackButtonDisplayMode: 'default'`; `headerSearchBarOptions` (§3.2); `headerTransparent` left default (the
  system scroll-edge effect does the work: HIG/toolbars "Reduce the use of toolbar backgrounds").
- Web: `Screen.tsx` keeps `TopBar` and the custom search pill.
- Acceptance: the large title collapses on scroll; Back reads the previous title; the search bar shows Cancel and Clear;
  at xxxLarge nothing in the bar wraps; a long list title at AX1 truncates with an ellipsis instead of pushing "Stage"
  off (the system does this).
- Risk: medium. `Stack.Toolbar` is new API (expo-router 58, `layouts/stack-utils/toolbar`); if it misbehaves, fall back to
  `headerRight` rendering `Pressable` + `SymbolView`.

### B4. Native form sheets
- Files: new `app/src/app/create.tsx` (from `lib/CreateSheet.tsx`), new `app/src/app/add/[board].tsx` (from
  `lib/AddJokeSheet.tsx`; writes the op itself; `swap` params), root `_layout.tsx` options, `index.tsx` / `sets.tsx`
  (`router.push('/create')`), `list/[id].tsx` (`router.push('/add/<id>')`, `useFocusEffect` reload). The small sheets
  (rename ×2, why, bridge) become `fitToContents` form-sheet routes, or stay on the RN `Modal` `Sheet` for now.
- Options: `presentation: 'formSheet'`; `sheetAllowedDetents: [0.5, 1]` (create) / `[1]` (add) / `'fitToContents'`
  (small); `sheetGrabberVisible: true`; `sheetInitialDetentIndex: 0`; `sheetExpandsWhenScrolledToEdge: true`;
  `headerShown: true` with `headerLeft` Cancel (`variant: 'plain'`) so the bar carries Cancel leading (HIG/sheets).
- Web: the web `(tabs)` shell renders these routes with the existing `Sheet` chrome (keep `lib/CreateSheet.tsx` as the
  body; the route renders it with `visible` always true and `onClose = router.back`).
- Acceptance: swipe down anywhere on the sheet dismisses; the grabber cycles detents; the keyboard pushes the content,
  not Cancel; Reduce Motion crossfades; VoiceOver announces the sheet title.
- Risk: medium-high. Form sheets plus the keyboard had regressions in react-native-screens 4.x (INFERRED); keep
  `keyboardHandlingEnabled` default and test "New set list" typing first.

### B5. Context menus and action sheets
- Files: `list/[id].tsx` rows, `inbox.tsx` card, `sets.tsx` cards, `joke/[id].tsx` "To a set" long press
  (`@expo/ui/swift-ui` `ContextMenu`); `stage/[id].tsx` stop flow (`ActionSheetIOS`); `ui.tsx` `MenuSheet` web-only.
- Options: destructive item last with the destructive role; no preview (rows are small; the row's own text is the target).
  No haptic on open (the system plays its own).
- Web: `MenuSheet` stays.
- Acceptance: long press lifts the row with the system blur and the menu; "Remove from this list" is red and last;
  VoiceOver custom actions still list the same verbs.
- Risk: medium. The `@expo/ui` ContextMenu API must be read before use (not read for this spec).

### B6. SF Symbols on iOS
- Files: `ui.tsx` `Icon` → `Icon.native.tsx` mapping our names to `SymbolView` (`name`, `size`, `weight: 'semibold'`,
  `type: 'monochrome'`, `tintColor`); keep the SVG `Icon` for the web; `SayBar.tsx` `MicGlyph` → `mic.fill`;
  `DragList.native.tsx` grip → `line.3.horizontal`.
- Acceptance: every icon matches the adjacent text weight ("Each of the nine symbol weights … corresponds to a weight of
  the San Francisco system font", HIG/sf-symbols).
- Risk: low.

### B7. Input bar as the tab bar accessory (iOS 26)
- Files: `(tabs)/_layout.tsx` (`NativeTabs.BottomAccessory` hosting `SayBar`; `usePlacement()` returns `'regular' |
  'inline'`); `Screen.native.tsx` (drop the absolute dock when the accessory is in use); `SayBar.tsx` (inline variant:
  the field only, the record ring hidden when `inline`).
- Options: `GlassView glassEffectStyle="regular" isInteractive` around the bar; `isLiquidGlassAvailable()` guard.
- Web: unchanged.
- Acceptance: scrolling Bits minimises the tab bar and the bar goes inline; the keyboard lifts the bar; hold-to-talk
  works at both placements.
- Risk: medium. New API; if `BottomAccessory` fights the keyboard, keep the absolute dock and ship B7 later.

### B8. Haptics corrections, colour tokens, small HIG items
- `joke/[id].tsx:102`: success after the write. `stage/[id].tsx:55`: no warning on Stop. `SayBar.tsx:76-77`: one
  `light` on press-in only (two impacts per hold is "overusing haptics", HIG/playing-haptics).
- `bits.tsx:17,43`, `map/index.tsx:22` → tokens with light variants.
- `map/index.tsx:125` "Fit" to 44 pt.
- `TextInput`s get `clearButtonMode="while-editing"`; the YouTube field `returnKeyType="go"`.
- Splash: `#0B0B0D`, no image, dark/light pair (§8). Icon Composer `.icon` (§8).
- Risk: low.

### B9. Not in scope here
- Transcription on the phone (SpeechTranscriber) and the recorder's interruption handling: separate builds; this spec
  only flags the HIG/playing-audio rule for the interruption case.
- iPad (`supportsTablet: true` in `app.json`): native tabs adapt to a sidebar (`sidebarAdaptable`,
  `preferredPlacement`); not designed here.

---

## 12. Device figures used (INFERRED)

iPhone 17: 402 × 874 pt at 3×; top safe-area inset about 62 pt (Dynamic Island); bottom 34 pt. The HIG layout page's
device table did not come through the text endpoint, so these are from memory and should be read off the device with
`useSafeAreaInsets()` before any fixed number is written into code. No fixed number should be: use the insets.
