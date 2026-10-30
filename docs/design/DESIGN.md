# Punchable design system: the values (DESIGN.md)

Status: design-system reference, 2026-10-29 (lane A0), written after the owner signed off `docs/ARCHITECTURE.md`
(§7 item 4: the five token scales, the twelve-component kit, the gate) including its four visual value changes: the
content heading 30 → 28 pt (Title 1); prose growing to 2.0× under Dynamic Type; the paper card's shadow measured in
light before it becomes a token; 24 spacing values → 9 steps and 18 radii → 5.

What this file is: the roles and the numbers, and the HIG rule behind each. What it is not: the iPhone screen spec
(`docs/design/HIG.md`), the product direction (`docs/UX_2026-10.md`) or the code layout (`docs/ARCHITECTURE.md` §7).

Tags. **CONFIRMED** = read in a file at `file:line` or computed from values read there (contrast ratios are WCAG
relative luminance over the hex values in `theme.ts`, rounded to one decimal). **PROPOSED** = a value this file sets
that the code does not hold yet. **INFERRED** = a device or library figure not read on a page. HIG pages are cited by
URL under `https://developer.apple.com/design/human-interface-guidelines/` (written `HIG/<page>`); when the HTML
renders empty, `https://developer.apple.com/tutorials/data/design/human-interface-guidelines/<page>.json` holds the
text. Sample copy is invented. Nothing here is a commit, a push or a deploy.

---

## 1. Principles

From `docs/design/HIG.md` §1, which holds the eight rules with their quotes:

1. One comic, often one-handed, often in the dark: backstage five minutes before a set, or on stage with the phone in
   the other hand.
2. Standard, familiar, intuitive: "Place UI in expected locations, use standard symbols to represent common actions,
   and rely on established conventions for navigation and modality." (HIG/branding)
3. Every rule is the HIG's; the order is ours. Where the owner has decided otherwise it is written down
   (`docs/UX_2026-10.md`, HIG.md §10) and this file says so at the value.

What that means for values: the system draws the chrome (tab bar, headers, sheets, menus) in SF with its own colours;
our tokens colour the content layer and the one accent. Dark is the default and light must work (owner decision).

---

## 2. How values reach code

| What | Where it lives | Rule |
|---|---|---|
| The numbers (colour, type, space, radius, size) | **`app/src/ui/tokens.ts`**, the source of truth once the app lane lands (`docs/ARCHITECTURE.md` §7 item 4). **Until then: `app/src/lib/theme.ts`** (`DARK`, `LIGHT_TOKENS`, `C`, `F`; CONFIRMED `theme.ts:10-176`) and the type bag in `app/src/lib/ui.tsx` (`buildTv`, `T`; CONFIRMED `ui.tsx:20-37`). | This file explains the roles and **must match that file**; a value changes in both in one commit, or not at all. |
| Reading a token | `C.x` at render time, never at module load (`theme.ts:140-141`: one mutable object swapped by `setScheme`); `useTheme()` once the React Compiler lands (ARCHITECTURE.md §7 item 6). | `RATING` captures `C.*` at load and goes stale after a scheme switch (CONFIRMED `theme.ts:191-196`): a known bug the token move fixes. |
| The web | The same tokens through react-native-web; nothing is copied. | — |
| Copies kept by hand | `app/scripts/brand/brand.mjs:21` (the brand palette, §7) and `infra/keycloak/theme/login/template.ftl:31-54` (the login page, §9). | A colour change here is a change there too; both are listed in §7 and §9 so none is forgotten. The Keycloak copy ships only with the owner's go (a shared-Keycloak outage, `docs/HANDOVER.md` traps). |
| The gate | `app/tests/static.values.test.mjs` (§10): number literals for style keys outside `src/ui` can only fall. | A new value is a token first, then a use. |

---

## 3. Colour

HIG rules (CONFIRMED on the page):
- "At a minimum, make sure the contrast ratio between colors is no lower than 4.5:1. For custom foreground and background colors, strive for a contrast ratio of 7:1, especially in small text." HIG/dark-mode
- "Avoid relying solely on color to differentiate between objects, indicate interactivity, or communicate essential information." HIG/color
- "Even if your app ships in a single appearance mode, provide both light and dark colors to support Liquid Glass adaptivity in these contexts." HIG/color
- "Apply color sparingly to the Liquid Glass material, and to symbols or text on the material." / "To emphasize primary actions, apply color to the background rather than to symbols or text." / "Refrain from adding color to the background of multiple controls." HIG/color
- "Prefer the system background colors." and base vs elevated: "The base colors are dimmer, making background interfaces appear to recede, and the elevated colors are brighter, making foreground interfaces appear to advance." HIG/dark-mode
- "Avoid hard-coding system color values in your app." HIG/color (so the record red is `systemRed` on iOS with `C.rec` as the fallback, HIG.md §3.9)

### 3.1 Roles (values CONFIRMED `theme.ts:10-132`; ratios computed)

Ratios are **on the ground / on a card**: dark on `#0B0B0D` / `#1C1C1F`, light on `#f6f6f4` / `#ffffff`. Text roles
must clear 4.5:1 on both; marks (dots, rings, bars) are shapes with a glyph or a word beside them (HIG/color) and are
held to 3:1 (WCAG 2.1 SC 1.4.11, `https://www.w3.org/TR/WCAG21/#non-text-contrast`, a non-HIG rule).

| Role (tokens.ts) | Token today | Dark | Light | HIG counterpart | Ratio dark | Ratio light | Where used |
|---|---|---|---|---|---|---|---|
| `bg.base` | `bg` | `#0B0B0D` | `#f6f6f4` | systemBackground (base) | — | — | every screen's ground, `contentStyle` |
| `bg.deep` | `bgDeep` | `#060607` | `#ebebe8` | — (a step under base) | — | — | the stage, wells |
| `bg.elevated` | `surface` | `#161618` | `#ffffff` | secondarySystemBackground / elevated | — | — | `s.card`, panels |
| `bg.card` | `card` | `#1C1C1F` | `#ffffff` | tertiarySystemBackground | — | — | cards, the search pill, the record dock |
| `bg.card2` | `surface2` = `card2` | `#1C1C1F` / `#161618` | `#eeeeeb` / `#f0f0ec` | — | — | — | inner boxes, skeleton bars, chips off |
| `label.1` | `text` | `#f3f4f6` | `#111214` | label | 17.9 / 15.4 | 17.3 / 18.7 | prose, titles |
| `label.2` | `text2` | `#d1d5db` | `#3d4048` | secondaryLabel | 13.3 / 11.5 | 9.6 / 10.4 | chips, snack-adjacent text |
| `label.3` | `mut` and `mut2` (**two tokens, one role**) | `#9ca3af` / `#A1A1A8` | `#5d6169` (both) | tertiaryLabel | 7.7 / 6.7 and 7.7 / 6.6 | 5.7 / 6.2 | row subs, help lines, "mut" text. PROPOSED: one token `label.3 = mut2` (the artboards' value); `mut` becomes an alias and goes. |
| `label.4` | `dim` | `#878d97` | `#6b7079` | quaternaryLabel / placeholder | 5.9 / 5.1 | 4.6 / 5.0 | placeholders, excluded points. Light sits at the floor: never smaller than 13 pt. |
| `label.tabOff` | `tabOff` | `#8E8E93` | `#6b6f78` | systemGray (unselected tab) | 6.0 / 5.2 | 4.7 / 5.0 | the web tab bar only (the phone's tabs are the system's) |
| `tint` | `amber` (**alias kept**) | `#FFB340` | `#b47a0e` | tintColor / accentColor | 11.0 / 9.5 | **3.4 / 3.7** | `headerTintColor`, `NativeTabs tintColor`, the one prominent button's background, the focus ring, the record ring. Light: marks only (§3.2). |
| `tint.ink` | `amberDeep` | `#7f5700` | `#8a5a00` | — | 3.1 (not text on dark) | 5.5 / 5.9 | amber-coloured TEXT and links on light; the snack's action on light paper |
| `tint.soft`, `tint.pill` | `amberSoft`, `amberPill` | `rgba(245,181,68,.15)`, `.22` | `rgba(180,122,14,.12)`, `.2` | — | fills | fills | selected chips, pills, the draft fill |
| `tint.mid` | `amberMid` | `#c98f2c` | `#c98f2c` | — | 7.0 / 6.0 | **2.6 / 2.8** | the middle step where a ramp has three (map pins, heat dot): a mark, never text |
| `destructive` | `red` | `#FF453A` | `#b91c1c` | systemRed (dark 255,66,69 on the page; ours is the iOS 18 value, INFERRED) | 5.8 / 5.0 | 6.0 / 6.5 | Remove, Delete, the gap segment's text, "Recording" |
| `destructive.soft` | `redSoft` | `#fca5a5` | `#b91c1c` | — | 10.4 / 9.0 | 6.0 / 6.5 | "Died" rating, error text |
| `gap`, `gap.text` | `gap`, `gapText` | `#FF6B5A`, `#FF8A7D` | `#d9482f`, `#b63a22` | — | 8.6 / 7.4 (text) | — | the set bar's gap segment; the clock in the light |
| `record` | `rec` | `#FF453A` | `#d9382c` | systemRed (`PlatformColor` first) | 5.8 | — | the record disc and square; white glyph on it is 3.4:1 dark: a 30 pt shape, not text |
| `separator`, `separator.strong`, `hairline` | `border`, `borderStrong`, `hairline` | `#2d3139`, `#4b5059`, `#1F1F22` | `#d9d9d4`, `#8f8b82`, `#e4e4df` | separator / opaqueSeparator | 1.5 (not text) | 1.3 | 1-pt card borders, row rules, the paper card's edge |
| `success` set | `green`, `greenDeep`, `greenSoft`, `greenBorder` | `#22c55e`, `#052e14`, `.15`, `.45` | `#15803d`, same, `.12`, `.45` | systemGreen | 8.6 / 7.5 | 4.6 / 5.0 | "Worked"/"Working", "proven on stage" |
| `info` set | `blue`, `blueSoft`, `aiBorder`, `aiBg` | `#38bdf8`, `.15`, `#1e3a52`, `.08` | `#0369a1`, `.12`, `#b9dcf0`, `.06` | systemBlue / systemCyan | 9.2 / 7.9 | 5.5 / 5.9 | the AI badge, "Ideas" stage, the AI dot |
| `status.kills/working/fresh/resting` | `statusKills`, `statusWorking`, `statusFresh`, `statusResting` | `#30D158`, `#FFB340`, `#64D2FF`, `#8E8E93` | `#248a3d`, `#b47a0e`, `#0a6e9e`, `#6b6f78` | systemGreen / tint / systemCyan / systemGray | 9.7, 11.0, 11.4, 6.0 | **4.1**, 3.4, 5.2, 4.7 | a bit's status dot, always beside its glyph ★ ● ○ – (never alone). Marks: 3:1 holds for all. |
| `cluster.1-6` | `cluster1…6` | `#40CBE0 #8E8CFF #FF6482 #A8E05F #D48CFF #9BB0C8` | `#0A7A8C #4641C9 #B8204A #4E7A0C #7B2FB3 #4F6378` | — | 10.1, 6.9, 6.9, 12.6, 8.4, 8.8 | 4.6, 6.8, 5.8, 4.7, 6.7, 5.7 | the galaxy's cluster colours; label pills take `label.1`, the colour is the dot |
| `heat[0..4]` | `heat` | amber at .12 .35 .6 .85 then `#f5b544` | `#b47a0e` at .12 .3 .5 .75 then solid | — | fills | fills | laugh density, the laugh bars, the mark's four bars (§7) |
| `scrim` | `scrim` | `rgba(9,10,11,.8)` | `rgba(17,18,20,.45)` | the system's sheet dimming | — | — | under the web `Sheet` only; native sheets dim themselves |
| `stage.*` | `STAGE[k].fg/bg` (`theme.ts:180-186`) | ideas blue, to_try `#e5e7eb` on `#22252a`, testing `#f5b544`, working green, retired `#9ca3af` | **none** | — | ≥ 7:1 on dark | **no light twin** (CONFIRMED) | `StageBadge`, `StageWord`. PROPOSED: `stage` maps onto `info`, `label.2`, `tint`, `success`, `label.3` so it inherits the light twin and the alias dies. |
| `draft` | `draftBorder`, `draftBg` | `#a7781f`, amber .05 | `#b47a0e`, .05 | — | border | border | the dashed draft box (the "draft rule": an AI line he has not kept is dashed and amber-edged, `s.draft`, `ui.tsx:48`) |
| `add` | `addBorder`, `addBg` | `#5c4a1f`, `#241e12` | `#e2cfa3`, `#fbf3df` | — | — | — | the "+" button in Add a joke, the warn banner's family (`warnBg` `#3b2a0a` / `#fff2d6`) |
| `snack` | `snackBg`, `snackText`, `snackAction` | `#f3f4f6`, `#0B0B0D`, `#7f5700` | `#1b1b1f`, `#f3f4f6`, `#ffb340` | an inverted bar | 17.9, action 5.8 | 15.6, action 9.6 | the one snackbar; the action is `tint.ink` on light paper and `tint` on dark ink |
| `fixture` | `fixture`, `fixtureBg`, `fixtureText` | violet `#7c3aed` | `#5b21b6` text | — | dev only | dev only | the dev fixture banner; a colour nothing real uses (on purpose) |
| `panel` | `panel` | `#16171a` | `#ffffff` | — | — | — | the web sidebar |

Three findings from the computation (CONFIRMED by the numbers, all PROPOSED fixes):
1. **`theme.ts:95` says light `amber` `#b47a0e` is "4.6:1 on white"; it is 3.7:1** (3.4:1 on the ground). It is a
   mark colour, not a text colour, which is exactly what §3.2 says; the comment is wrong and the few places that set
   11-13 pt text in `C.amber` on light (`Yours`, `ui.tsx:132`; the AI word; `RATING.killed`) must use `tint.ink`.
2. Light `statusKills` `#248a3d` is 4.1:1 on the ground: fine for a dot beside ★, under 4.5:1 if it ever colours a
   word. Keep it a mark, or darken to the system's increased-contrast green (`#008932`, HIG/color table).
3. `mut` and `mut2` are the same role at 7.7:1 (dark) and the same hex on light. One token.

### 3.2 The light variant, and the rule

`LIGHT_TOKENS` keeps the structure of `DARK` and swaps hues (`theme.ts:75-132`); `resolveScheme(pref, system)` picks
dark unless the OS or the owner's Settings override says light (`theme.ts:162-165`; HIG/settings lists "interface
style" as a legitimate in-app setting, HIG.md §3.11). The light ground is `#f6f6f4`, not pure white, so the white
cards read as elevated on it (the HIG's base/elevated idea, inverted for light).

**Amber never carries text on light.** `#b47a0e` is 3.7:1 on white and `#FFB340` is unreadable there; so on light:
- text and links in the accent use `tint.ink` `#8a5a00` (5.5:1 on the ground, 5.9:1 on white; CONFIRMED computed);
- marks (the selected tab, the focus ring, dots, the record ring, the mark in §7) use `tint` `#b47a0e` (3:1 non-text
  rule, met at 3.4:1);
- the one prominent button keeps an amber background with ink text: `LIGHT.amber` `#f5b544` under `#111214` is
  10.3:1 (CONFIRMED `theme.ts:137`, computed). This is the HIG's "apply color to the background rather than to
  symbols or text" (HIG/color).
- `tint.mid` and the heat ramp's lower steps are fills, never text (2.6:1).

Scheme-dependent rules that are the same in both modes: 1-pt borders are `separator` (never a lighter card on a card
to imply an edge); status is shape + colour; the AI badge is blue in both and never amber (amber means "yours").

### 3.3 Fixed colours (scheme-independent, CONFIRMED)

| Name | Value | Where | Why fixed |
|---|---|---|---|
| `paper` | `#F2EEE4` (`sets.tsx:21`) with ink `#1B1B1F`, marker `#2A2A2E` and the faint `#6B6660` (`sets.tsx:22-23,36-60`) | `PaperCard` only, the set lists | A paper set list is paper in the dark too (owner's artboards). Ink on paper 16.2:1, the faint 4.9:1 (computed). HIG/dark-mode "Soften the color of white backgrounds": it is off-white. |
| `paper.shadow` | today `shadowColor '#000', opacity .5, radius 10, offset 0/6` (`sets.tsx:36`) in **both** schemes | `PaperCard` | **Owner decision (ARCHITECTURE.md decision 5): measure in light first.** A 50 % black shadow on a `#f6f6f4` ground is a hard drop; the dark value stays. PROPOSED token `shadow.paper = { dark: {.5, 10, 0/6}, light: measured }`; nothing else in the app casts a shadow (CONFIRMED: the only `shadow*` literals are this line). |
| `white` | `#fff` | glyphs on the record disc, the REC label, "Resume" on the disc (`SayBar.tsx:129`, `RecordControl.tsx:36`) | White on red/amber shapes; 3.4:1 on the dark red (a 30-pt shape), 1.8:1 on amber (so no white text on the amber button, ever: ink only). |
| `record` | `systemRed` on iOS, `C.rec` elsewhere | the disc, the square, "Recording" | HIG/color: never hard-code the system's red; Voice Memos' red is the one he knows. |

---

## 4. The type scale

HIG rules (CONFIRMED, HIG/typography): "Follow the recommended default and minimum text sizes for each platform — for
both custom and system fonts"; iOS default **17 pt**, minimum **11 pt** (the platform table); "System fonts
automatically support Dynamic Type (where available) and respond when people turn on accessibility features, such as
Bold Text. If you use a custom font, make sure it implements the same behaviors."; "Make sure your app's layout
adapts to all font sizes."; "Keep text truncation to a minimum as font size increases."; "In general, avoid light
font weights." HIG/branding: "It can work well to use a custom font for headlines and subheadings while using system
fonts for body copy and captions, because the system fonts are designed for optimal legibility at small sizes."

Families (CONFIRMED `theme.ts:167-176`): **SF** wherever the system draws (bar titles, tabs, menus, sheets, alerts,
search fields: no token, no override); **Inter** for prose and controls (400/500/600/700/800); **Space Grotesk** for
headings, numbers and tracked caption labels (500/600/700). Inter for body is the owner's artboards and an accepted
deviation from the HIG/branding sentence above (HIG.md §6); the cost is Bold Text, which custom fonts ignore (§8).

| Token | HIG style | Size / leading at Large | Family, weight | Cap (`maxFontSizeMultiplier`) | Today (`ui.tsx:23-33` unless noted) | Change |
|---|---|---|---|---|---|---|
| (system) | Large Title | 34 / 41 | SF, bold | system | `headerLargeTitle` (`Screen.native.tsx:69`); `F.black` 34 in the web `TopBar` | none: the system owns it on the phone |
| `type.title1` | Title 1 | **28 / 34** | Space Grotesk 700, ls −0.6 | **1.8** | `h1` 30 / 35 | **30 → 28 (owner signed)**; the bit's full title, Settings' h1 on the web |
| `type.title2` | Title 2 | 22 / 28 | Space Grotesk 600 | 1.8 | none ("Ask your material" is 20 bold, HIG.md §6) | new |
| `type.title3` | Title 3 | 20 / 25 | Space Grotesk 600 | 1.8 | `h2` 20 / 26 | leading 26 → 25 |
| `type.headline` | Headline | 17 / 22 | Inter 600 | **2.0** (it sits in rows with body) | `semi` 17 / 24 | leading 24 → 22 |
| `type.body` | Body | 17 / 22 (web 16 / 21 = Callout) | Inter 400 | **2.0** | `body` 17 / 24, `BODY = 17` on iOS (`ui.tsx:22`) | cap 1.6 → 2.0 (owner signed); leading 24 → 22 |
| `type.callout` | Callout | 16 / 21 | Inter 400; Space Grotesk 600 tabular for `num` | 2.0 as prose, 1.6 as a control label | `num` 16; the input field 16 (`SayBar.tsx:116`) | names the 52 literal 16s |
| `type.subhead` | Subhead | 15 / 20 | Inter 400; Inter 600 / Space Grotesk 700 for button labels | 1.6 (controls) | row subs at 15; `Btn` 15 (`ui.tsx:171`); `TextBtn` 15 | names the 37 literal 15s; `mut` 14 / 20 moves up to 15 where it is a row sub |
| `type.footnote` | Footnote | 13 / 18 | Inter 400 | 1.6 | `small` 13 / 18; `mut` 14 / 20 where it is help text | 14 has no HIG style: 40 literals become 13 or 15 by role |
| `type.caption1` | Caption 1 | 12 / 16 | Space Grotesk 700, uppercase, ls 0.96 | 1.6 | `eb` 12 | the eyebrow; map labels 12 keep cap 1.2 (HIG.md §3.10) as `caption1` with an explicit lower cap |
| `type.caption2` | Caption 2 | **11 / 13** (the floor) | Space Grotesk 700, ls 0.66 | 1.6 (today 1.3-1.4) | `Badge` 11, `AIBadge` 11, `Yours` 11 (`ui.tsx:97,122,132`) | nothing below 11, ever |
| `type.display` | — (ours) | 40 / 44 | Space Grotesk 700 tabular, ls −1 | 1.6 (a figure beside its label; PROPOSED) | `big` 40 / 44 | the set time, "6:40" |
| `type.clock` | — (ours) | 84 / 90 | Inter 800 tabular, ls −2 | **1.2** | `RecordControl.tsx:76` | the one clock; the status word beside it is `subhead` at 1.4 |

The Dynamic Type caps, by class (owner signed; ARCHITECTURE.md §7 item 4): **prose 2.0×, titles 1.8×, captions and
controls 1.6×, the clock 1.2×.** Today every `T` caps at 1.6× (`ui.tsx:36`) and the inline caps are spread 14 × 1.2,
20 × 1.3, 48 × 1.4, 3 × 1.5, 10 × 1.6 (CONFIRMED tally). The honest note: the HIG asks layouts to adapt to **all**
sizes and the AX sizes run to about 3.1× for Body (AX5 = 53 pt, INFERRED from the AX table; the fetched page text did
not carry the iOS AX rows); a cap is a deviation. 2.0× is the WCAG 2.1 SC 1.4.4 figure ("resize text up to 200
percent", `https://www.w3.org/TR/WCAG21/#resize-text`), chosen so prose keeps growing past the old 1.6× while rows
stay rows. Each token carries its cap, so no screen writes `maxFontSizeMultiplier` again (the gate counts it).

Row heights scale with `PixelRatio.getFontScale()` (CONFIRMED `bits.tsx:22`, `ListRow` `minHeight: 56`,
`ui.tsx:228`); rows re-stack at font scale ≥ 1.3 (`layoutRules.ts`, `STACK_AT`); titles get `numberOfLines={1}` and
trailing items `flexShrink: 0` (HIG.md §6).

**The 11 pt floor: two known violations (CONFIRMED) and their fix.**

| Where | Today | Fix |
|---|---|---|
| `app/src/lib/SayBar.tsx:129` | "● REC" at **9 pt** in the accessory and inline variants (`variant === 'dock' ? 11 : 9`) | `caption2` (11) in every variant; at 11 pt the word no longer fits the 36-pt inner ring, so the inline variant shows the glyph alone with `accessibilityLabel="Recording"` (the label is already there). The record button is leaving the input bar anyway (jots are typed). |
| `app/src/lib/metrics.tsx:201` | the chart's axis labels at **10 pt** / 13 | `caption2` 11 / 13; the 68-pt label box becomes 76 or the labels wrap to two lines (`numberOfLines={2}`). |

Mapping from today's nine `TV` names (`ui.tsx:20`): `body → type.body`, `semi → type.headline`, `mut → type.subhead`
(row subs) or `type.footnote` (help), `small → type.footnote`, `eb → type.caption1`, `h1 → type.title1`,
`h2 → type.title3`, `num → type.callout` (tabular), `big → type.display`. The colour leaves the type token: `mut` and
`small` carried `C.mut`; the `Text` component takes `color="label.3"` separately so a footnote can be `label.1`.

---

## 5. Spacing, radius, size, layer

HIG rules (CONFIRMED): "As a general rule, a button needs a hit region of at least 44x44 pt" (HIG/buttons);
"Controls that are too small are hard for many people to interact with and select. Strive to meet the recommended
minimum control size for each platform" with the iOS table **default 44 × 44 pt, minimum 28 × 28 pt**
(HIG/accessibility); "Keep the number of prominent buttons to one or two per view" (HIG/buttons); "Respecting the safe
area is essential" and "horizontally adjacent views may need to stack vertically to provide more space for text"
(HIG/layout). The HIG gives no spacing grid or radius list for iOS; those are ours.

### 5.1 Space: 9 steps (PROPOSED) from today's 24 distinct values (CONFIRMED tally of `padding*`, `margin*`, `gap*`)

| Step | pt | Use | Absorbs today's |
|---|---|---|---|
| `space.1` | 2 | between a glyph and its word; the dot-to-text gap | 2, 3 (49 uses) |
| `space.2` | 4 | inside pills and badges; between a title and its sub | 4, 5 (98) |
| `space.3` | 6 | chip padding; the snack's inner gap | 6, 7 (143) |
| `space.4` | 8 | the base unit: row gaps, card inner gaps, icon-to-label | 8 (237) |
| `space.5` | 12 | card inner padding (small), list row gap, the inner box | 10, 12, 14 (228; 10 goes to 8 or 12 by its neighbour, 14 to 12 or 16) |
| `space.6` | 16 | card padding, screen side padding, the control's horizontal pad | 16, 18 (62) |
| `space.7` | 20 | section gaps; the large-title side margin the system uses | 20 (6) |
| `space.8` | 24 | between sections; sheet top padding under the grabber | 24 (7) |
| `space.9` | 32 | the empty state's breathing room; the stage's vertical rhythm | 32 (2) |

Not spacing, so not steps: `0` (allowed as a literal), and 40, 52, 72, 74, 84, 120, 140, 150 (the reserved heights
under floating bars and the stage's layout; CONFIRMED 11 uses). Those become `size.*` (the bar `120`, the dock `96`,
`Screen.native.tsx:35-36`) or are computed from `useSafeAreaInsets()` (HIG.md §12: no fixed device number).
Snapping moves a value by at most 2 pt (ARCHITECTURE.md decision 5): 3 → 2 or 4, 5 → 4 or 6, 7 → 6 or 8, 10 → 8 or
12, 14 → 12 or 16, 18 → 16 or 20, each decided by what sits beside it, never by the file.

### 5.2 Radius: 5 steps (PROPOSED) from today's 18 distinct values (CONFIRMED tally, 174 literals)

| Step | pt | Use | Absorbs today's |
|---|---|---|---|
| `radius.1` | 4 | badges, the `rv` mark, the paper card (6 → 4: paper is crisp), tiny swatches | 1-7 (50) |
| `radius.2` | 8 | inner boxes, chips off, the draft box (10 → 8), the eye button | 8, 9, 10 (52) |
| `radius.3` | 12 | cards, buttons, fields, sheet rows (`s.card`, `Btn`) | 11, 12, 14 (36) |
| `radius.4` | 16 | sheets, the 60-pt side controls, the big tiles, cards with 18-20 | 16, 18, 20 (22) |
| `radius.pill` | 9999 | everything round: the search pill (24), 44-pt circles (22), the disc and ring, progress bars (a 4-pt bar at `pill` is 2 pt, so the 1-3 pt radii on bars go here too) | 22, 24 and the circles (14 + the computed `size / 2`s, which the gate does not count) |

### 5.3 Size (CONFIRMED where cited; the token names PROPOSED)

| Token | pt | Rule | Today |
|---|---|---|---|
| `size.control` | 44 | the default control (HIG/accessibility table; HIG/buttons) | `IconBtn`, `AddBtn`, `Btn small`, tab triggers |
| `size.minTarget` | 28 | the minimum, **only with `hitSlop`** to 44 (HIG/accessibility minimum 28) | four targets under 44 with no slop: `bit/[id].tsx:344` (28), `shows.tsx:40`, `write.tsx:264` (32), `joke/[id].tsx:202` (36, the version tabs): each gets `hitSlop` or grows |
| `size.row` | 44 min, 56 default | a list row (HIG/lists-and-tables; HIG.md §3.2 "≥ 44 pt scaled") | `ListRow` 56 (`ui.tsx:228`); Bits rows 44 × font scale |
| `size.button` | 48 / 44 small | `Btn` | `ui.tsx:167` |
| `size.field` | 52 dock / 44 accessory / 40 inline | the input bar's field by placement | `SayBar.tsx:45-49` (`BAR_SIZES`); 40 is under 44 and allowed only because the inline bar is the system's minimised accessory (INFERRED: the system sizes the inline slot) |
| `size.stageDisc`, `size.stageRing` | 80 / 92 | the red disc in its ring; "92 pt hit region on the centre control" | `RecordControl.tsx:13-14` |
| `size.stageSide` | 60 | Pause / Next bit / Mark, one-handed in the dark | `RecordControl.tsx:47`; HIG.md §1 |
| `size.stageStop` | 30 | the square while taping (a shape inside the 80-pt target) | HIG.md §3.9 |
| `size.avatar` | 34 (header), 44 (tap) | the face | `Face` (`ui.tsx:308`) |
| `size.badge` | 24 min | `Badge` | `ui.tsx:95` |
| `size.bar`, `size.dock` | 120 / 96 | reserved height under the input bar and a task dock | `Screen.native.tsx:35-36` |

### 5.4 Layer, elevation, material

HIG rules (CONFIRMED, HIG/materials): "Liquid Glass forms a distinct functional layer for controls and navigation
elements — like tab bars and sidebars — that floats above the content layer"; "Don't use Liquid Glass in the content
layer"; "Use Liquid Glass effects sparingly"; "iOS and iPadOS continue to provide four standard materials — ultra-thin,
thin, regular (default), and thick — which you can use in the content layer". HIG/layout: "Instead of applying a solid
or semi-opaque background color beneath controls, use a scroll edge effect to visually elevate controls above content."

| Layer | What draws it | Our values |
|---|---|---|
| Navigation (glass) | the system: `NativeTabs` (`TabsShell.native.tsx`), the stack header (`Screen.native.tsx:69`), form sheets (`_layout.tsx:55`), menus, action sheets, the bottom accessory (`accessory.native.ts`, gated by `isLiquidGlassAvailable()`) | `tint` only; no background colour (HIG/toolbars "Reduce the use of toolbar backgrounds"); `headerTransparent: false` where a screen brings its own body (HANDOVER trap). **No `GlassView` anywhere today (CONFIRMED: no import)**; if one is ever used it is the floating input bar and nothing else (HIG.md §5). |
| Content, flat | our tokens | ground → `bg.elevated` → `bg.card` → `bg.card2`: each step brighter in dark (the base/elevated idea), each step whiter-on-grey in light; a 1-pt `separator` border, no shadow, no blur. `elevation` is a colour step, not a `shadow*`. |
| Content, the one shadow | `PaperCard` | §3.3; measured in light before it is a token. |
| Scrim | the web `Sheet` | `scrim`; native sheets dim themselves. |
| Z order inside a screen | `Screen.native.tsx:38-41` | the snackbar sits above whatever floats at the bottom (`pinned = bar + dock + inset`); nothing else is absolute. |

---

## 6. The kit index

Twelve components (ARCHITECTURE.md §7 item 4), each in `src/ui/`. Every one: height from the font scale (Dynamic
Type), a label or a text child and actions mirroring its gestures (VoiceOver: "Offer alternatives to gestures",
HIG/accessibility), a fade where it would slide (Reduce Motion: "reducing automatic and repetitive animations"),
and a 44-pt target reachable with a thumb (one hand).

| # | Component | Use | Don't | Web twin | Dynamic Type · VoiceOver · Reduce Motion · one hand | Today |
|---|---|---|---|---|---|---|
| 1 | `Text` | every string; `variant` from §4, `color` from §3 | inline `fontSize`, `maxFontSizeMultiplier` or a hex | same file | cap from the token · reads as text · — · — | `T` (`ui.tsx:35`) |
| 2 | `Input` | every text field: label (required), placeholder in `label.4`, `clearButtonMode="while-editing"` (HIG/text-fields), `keyboardType`/`returnKeyType` by content | a bare `TextInput` (19 copies today) | same file; the web keeps `clearButtonMode` as a × button | field grows with the scale · the label is the accessibility label · — · 44 pt min, 52 in the dock | `SayBar.tsx:116`, the rename forms |
| 3 | `RenameSheet` | short name, list title, voice profile: a native form sheet, Cancel leading, Done trailing (HIG/sheets) | an inline rename link in content | the RN `Modal` `Sheet` with `SheetHead` | the sheet scrolls · the title is announced · the system fades · Cancel is top-left, Done top-right, the field focused | `RenameForm` ×3, `ShortNameForm`, `TitleForm` |
| 4 | `ListRow` | title + sub + trailing + `chevron.right` for a push (HIG/lists-and-tables: "A disclosure indicator reveals the next level") | a chevron on a row that opens a sheet | same | `minHeight` × font scale, sub wraps · one label "title, sub" · — · 56 pt | `ListRow` (`ui.tsx:223`) |
| 5 | `SwipeRow` / `DragList` | set-list rows: trailing swipe (Remove full-swipe + Undo, Swap), drag by the grip after a long press, `light` on lift, `selection` on drop (HIG/playing-haptics) | a swipe that deletes without Undo | `SwipeRow.tsx` / `DragList.tsx` (buttons, no gesture) | row height scales · `accessibilityActions` Move up / Move down / Remove / Swap · no spring on drop · 24-pt back-swipe edge kept (`SwipeRow.native.tsx:49`) | `SwipeRow.native.tsx`, `DragList.native.tsx` |
| 6 | `Card` / `PaperCard` | a content block on `bg.card`; `PaperCard` is the **only** user of `paper` (§3.3) | glass, a shadow (except paper), a card on a card for an edge | same | padding is `space.6`; text inside scales · a card with one action is one `accessible` node · — · the whole card is the target | `Card` (`ui.tsx:89`), `PaperCard` (`sets.tsx:29`) |
| 7 | `Badge` (`kind`) | status, stage, AI, "yours", proven, counts: one component, a `kind` picks colour and glyph | colour alone (every kind has a glyph or a word; HIG/color) | same | `caption2` at 1.6, `minHeight` scales · the label is the text · — · not a target | `Badge`, `StageBadge`, `AIBadge`, `Yours`, `Proven` (`ui.tsx:93-145`) |
| 8 | `Button` (`variant`) | `prominent` (one per view, amber background, ink text), `plain`, `destructive`, `icon` (a symbol, labelled), `text` | two prominent buttons; white text on amber; a 9-11 pt label | same | 44/48 pt × font scale, label wraps to 2 lines before it truncates · `accessibilityRole="button"` + label · — · 44 pt | `Btn`, `IconBtn`, `TextBtn`, `SmallAct`, `AddBtn` (`ui.tsx:146-183, 354-371`) |
| 9 | `Chip` / `Seg` | `Chip` filters (on = `tint.pill`); `Seg` switches **views** (≤ 5 nouns; HIG/segmented-controls: never an action) | a segment that navigates (Map was one; it is a toolbar button now) | same (`@expo/ui` Picker is an option on iOS, HIG.md §4) | 44 pt scaled · `selected` state announced · — · thumb row | `Chip`, `Chips`, `Seg` (`ui.tsx:185-221`) |
| 10 | `ProfileButton` | the face: initials or the system `person.crop.circle`, label "Your profile" | the mark (§7) as an avatar | same | 34 pt face in a 44 pt target · labelled · — · top-right of the bar | `Face`, `Avatar` (`ui.tsx:308-352`) |
| 11 | the sheets (web only once native form sheets land everywhere) | `Sheet` + `SheetHead` + `MenuSheet` for the web and Android | on iOS, where the route is a `formSheet` (`create`, `add/[board]`, `voice`) and the long press is `RowMenu.native` | — | KAV around the body · `label` announced · fade · grabber + Cancel | `ui.tsx:393-507` |
| 12 | `Skeleton` / `Empty` / `ErrorCard` / `Snack` | loading = skeleton lines (HIG/loading), empty = one sentence + one action, error = "Try again" with no blame (HIG/writing), snack = one action, 8 s with Undo, above the pinned bars | a centred spinner; two snack actions; a snack that is the only Undo | same | text scales · `progressbar` label "Loading"; the snack is announced · — · the snack's action is 44 pt | `ui.tsx:79-87, 261-280, 509-640` |

Gone with the kit: the static style bag `s` (`ui.tsx:42-51`; `card`, `inner`, `punch`, `rv`, `draft` become
`Card` props and `Badge` kinds), `GoogleG` moves to `src/ui/brand`, and the SVG `Icon` stays the web twin of
`Icon.native.tsx` (SF Symbols, `SymbolView`, filled in tab bars, outline in toolbars and lists: HIG/sf-symbols).

---

## 7. The mark and the wordmark (what exists; CONFIRMED)

The comic's pick (2026-10-26) is **concept C, "Laugh meter"**: the app's own four-bar reception meter, peaking
(`app/src/lib/Brand.tsx:1`). The options page `docs/brand/pick-a-logo.html` marked concept A ("Mic P") as the
recommendation; the pick overrode it. Concepts A and B stay in `docs/brand/` as history.

- **Geometry** (`app/scripts/brand/brand.mjs:54-61`, mirrored in `Brand.tsx:8-13`): a 512 grid; four bars 64 wide at
  x = 96, 192, 288, 384, heights 120, 200, 280, 360, foot at y = 436, round top (radius 22) and square foot; bar
  opacities 0.35, 0.6, 0.85, 1 on the amber heat ramp (`heat`, §3.1). Mono: one colour, no opacities,
  `currentColor`.
- **Tile** (the app icon): a graphite square, `rx` 112 of 512, the mark in amber. **Drift (CONFIRMED):** the brand
  script's palette is the older one (`brand.mjs:21`: tile `#111214`, amber `#f5b544`) while the app's ground is
  `#0B0B0D` and its amber `#FFB340` (`theme.ts:11,29`). The icon's tile is therefore a shade lighter than the screen
  behind the splash (`app.json:10` `backgroundColor #0B0B0D`). PROPOSED: `brand.mjs` reads `theme.ts` or carries the
  same four values, then regenerate (`node scripts/brand/brand.mjs --png`).
- **Appearances**: `app.json:12-15` ships `ios.icon { light, dark, tinted }` PNGs (CONFIRMED). HIG/app-icons: "You can
  design app icon variants for every appearance variant, and the system automatically generates variants you don't
  provide"; "Keep your icon's features consistent across appearances"; "Include text only when it's essential to your
  experience or brand" (the mark has none); "For iOS, iPadOS, and macOS icons, provide square layers so the system can
  apply rounded corners"; "Embrace simplicity in your icon design." An Icon Composer `.icon` bundle (HIG.md §8) is
  still open; the PNG trio is the fallback it names.
- **Wordmark**: "punchable", lowercase, Space Grotesk Bold, **outlined to paths** (`glyphs.json` via `outline.py`) so
  no SVG or the login page depends on a font. Lockup: the mark scaled to the wordmark's full height (ascender −70 to
  descender +20 at size 100), 22 units of space, then the word (`brand.mjs:72-83`). In the app the `Lockup` draws the
  word live in `F.g7` at 0.54 × size with letter-spacing −0.016 × size, cap 1.2, one accessible node
  `accessibilityRole="header"` `accessibilityLabel="Punchable"` (`Brand.tsx:28-33`).
- **Files**: `docs/brand/<concept>-{icon,favicon,mark,mark-light,mono,lockup-dark,lockup-light}.svg` ×3 concepts and
  `logo-options.html` (generated, do not hand-edit); `app/public/{favicon.ico,favicon.svg,apple-touch-icon.png,
  icon-192.png,icon-512.png,og-image.png}` (the web, cited from `app/public/index.html:8`); `app/assets/images/icon*.png`
  (the phone). The favicon cut at 16-48 px is the same four bars (concept C's `small` = `mark`).
- **Where it may appear**: the app icon, the launch screen's absence (HIG/launching: no logos; HIG.md §8), the web
  favicon and share image, the login page header (§9), Settings › About. **Never in a navigation bar or a tab**
  (HIG/branding: "Resist the temptation to display your logo throughout your app or game unless it's essential for
  providing context"). The accent does the branding: "use it intentionally for primary actions or status indicators,
  like … an icon for the selected tab in a tab bar" (HIG/branding).
- On light the mark is `tint` `#b47a0e` and the word is ink `#111214` (`mark-light`, `lockup-light`); on dark the
  mark is `#FFB340` and the word `#f3f4f6`.

---

## 8. Accessibility defaults

What every component ships with, so a screen never has to remember (rules CONFIRMED, HIG/accessibility unless noted):

| Default | Rule | Value / mechanism | Today |
|---|---|---|---|
| Targets | 44 × 44 default, 28 minimum (the iOS table) | `size.control`; `size.minTarget` only with `hitSlop` | 11 `hitSlop` uses; four under-44 targets without (§5.3) |
| Dynamic Type | "Your interface can support font size enlargement either through custom UI, or by adopting Dynamic Type." | the cap per token (§4); `PixelRatio.getFontScale()` for heights; rows re-stack at ≥ 1.3 | caps spread inline; `T` at 1.6 |
| Bold Text | "If you use a custom font, make sure it implements the same behaviors." (HIG/typography) | INFERRED: `AccessibilityInfo.isBoldTextEnabled()` at start, `Inter_400 → 600`, `600 → 700`, `SpaceGrotesk_600 → 700` (HIG.md §6) | not handled |
| VoiceOver | "Describe your app's interface and content for VoiceOver." | a label or a text child on every control; `accessibilityRole`; rows carry `accessibilityActions` that mirror swipe and drag | rows, grips, swipes, cards labelled; native tabs and headers come labelled |
| Gestures | "Offer alternatives to gestures. Make sure your UI's core functionality is accessible through more than one type of physical interaction." | every swipe has a menu item, every drag has Move up/down, every pinch has buttons (the map's turn buttons), every long press has a visible path | `RowMenuBase.rowActions`, the stage's Next button |
| Reduce Motion | "ensure your app or game responds by reducing automatic and repetitive animations, including zooming, scaling, and peripheral motion." | `useReduceMotion()` (`ui.tsx:63-77`): fades not slides, no spring, the cloud's slow turn off | done where touched |
| Colour | "Convey information with more than color alone." / "Offer visual indicators, like distinct shapes or icons, in addition to color" | status = shape + colour; links = colour + dash; the AI badge = a word | done |
| Contrast | 4.5:1 minimum, 7:1 for small custom text (HIG/dark-mode); "ensure it at least provides a higher contrast color scheme when the system setting Increase Contrast is turned on" | §3.1 ratios; label.1-3 at ≥ 5.7:1 in both modes. Increase Contrast: UNKNOWN whether RN 0.88 exposes the iOS setting (read `AccessibilityInfo`'s typings in `node_modules` before claiming); if it does, `label.3 → label.2`, `label.4 → label.3`. | not handled |
| Haptics | "Make haptics optional" (HIG/playing-haptics) | the Settings switch; `selection` for values changing, `light`/`medium` impact for lift and snap, `success`/`warning`/`error` for outcomes only, after the write | `lib/haptics.ts` |
| Timed UI | "Minimize use of time-boxed interface elements … Prefer dismissing views with an explicit action." | the snack is 4-8 s with one action; every Undo also has a non-timed path (the inbox's "Filed for you", the set-list menu's "Undo last change") | partly (HIG.md §7) |
| Keyboard | "Use the keyboard layout guide" (HIG/virtual-keyboards) | `KeyboardAvoidingView` around stack content; native form sheets handle their own | done |
| Type floor | 11 pt (HIG/typography table) | `caption2` is the smallest token | two violations (§4) |

---

## 9. The Keycloak login theme (CONFIRMED `infra/keycloak/theme/login/template.ftl`)

The sign-in page is the first Punchable screen a browser or the iOS in-app sheet shows, and it is a hand-kept copy of
the tokens (no build step; Keycloak caches theme resources hard, so the CSS is inline in `template.ftl:3-5`). Its
static test `infra/keycloak/test_theme_static.py` pins every text/background pair at ≥ 4.5:1 in both schemes, 44 pt
targets, 16 px inputs (no iOS zoom) and no external JS or CSS; `theme_mockups.mjs` renders it without Keycloak.
Shipping a change is `infra/ship.sh keycloak-theme` on the shared Keycloak (a short outage for every realm: the
owner's go, `docs/HANDOVER.md` traps); `realm-punchable.yaml:14` names the theme.

| CSS variable (`template.ftl:31-54`) | Dark | Light | Token here |
|---|---|---|---|
| `--bg`, `--surface`, `--card`, `--hair`, `--border`, `--border-strong` | `#0B0B0D`, `#161618`, `#1C1C1F`, `#1F1F22`, `#2d3139`, `#4b5059` | `#f6f6f4`, `#ffffff`, `#ffffff`, `#e4e4df`, `#d9d9d4`, `#8f8b82` | `bg.base`, `bg.elevated`, `bg.card`, `hairline`, `separator`, `separator.strong` (§3.1, exact) |
| `--text`, `--text2`, `--mut`, `--dim` | `#f3f4f6`, `#d1d5db`, `#A1A1A8`, `#878d97` | `#111214`, `#3d4048`, `#5d6169`, `#6b7079` | `label.1-4` (exact; `--mut` is `mut2`) |
| `--mark`, `--focus` | `#FFB340` | `#b47a0e` | `tint` (marks: the lockup, the focus ring) |
| `--link` | `#FFB340` | `#8a5a00` | `tint` on dark, `tint.ink` on light: the §3.2 rule, kept |
| `--btn-bg` / `--btn-fg` | `#FFB340` / `#0B0B0D` | `#f5b544` / `#111214` | the prominent button (`LIGHT.amber` under ink, 10.3:1) |
| `--err*`, `--warn*`, `--ok*`, `--info*` | `#fca5a5` on `#2a1210`, amber on `#241e12`, `#86efac` on `#0f2418`, `#bae6fd` on `#10222e` | `#b91c1c` on `#fdecec`, `#8a5a00` on `#fbf3df`, `#15803d` on `#eaf7ee`, `#0369a1` on `#eaf4fa` | `destructive.soft`, `add`/`warnBg`, `success`, `info`: the fills are the theme's own tints of them (no app token for the three light pastels; PROPOSED `*.bg` tokens if the app ever shows the same banners) |
| `--g-bg`, `--g-fg`, `--g-border` | `#131314`, `#E3E3E3`, `#8E918F` | `#ffffff`, `#1F1F1F`, `#747775` | Google's own button colours (their brand rules), not ours |
| `--shadow` | `0 24px 64px rgba(0,0,0,.45)` | `… rgba(17,18,20,.10)` | the card's one shadow: note the light value is **0.10**, a measured-looking figure the paper card (§3.3) can start from |
| `--font` | the system stack (SF on the iPhone sheet), Inter when installed; no web font | same | §4 families: the sign-in page never fetches a font |

Type and size on the page (`template.ftl:69-142`): h1 28 px / 1.15 (= `title1`), sub 15 (`subhead`), buttons 17 /
600 (`headline`) at 52 px, inputs 16 px at 48 px (`callout`; 16 so iOS does not zoom), labels 14 / 600, hints and
errors 14, footer 13 (`footnote`), the "or" rule 13 uppercase tracked (`caption1` at 13); the card radius 20 and the
controls 12 (`radius.4`, `radius.3`); the eye button 44 × 44 (`size.control`).

---

## 10. The gate, and how to add a token

**The gate** (`app/tests/static.values.test.mjs`, ARCHITECTURE.md §7 item 4): a **per-file count map, not a set**, of
number literals for `borderRadius*`, `padding*`, `margin*`, `gap*`, `fontSize`, `lineHeight`, `letterSpacing` and
`maxFontSizeMultiplier` in every file outside `src/ui`; the test fails **above and below** each entry, so a count can
only fall, and a fall is recorded in the map in the same commit. `0` and expressions (`size / 2`, `ins.top + 4`) are
not matched; `width` and `height` stay out (geometry); Skia and Reanimated props are not style keys and need no
exemption (CONFIRMED in ARCHITECTURE.md). Baseline at the time of writing: 174 radius, 691 spacing, 272 font-size
literals (ARCHITECTURE.md); this file's own tally of distinct values is 18 radii and 24 spacings (§5), the figures the
owner signed.

**To add a token** (one commit, in this order):
1. Say the role in this file first: which section, the HIG rule or the owner decision behind it, dark and light values
   with their ratios (compute them; the §3.1 method), the cap if it is type.
2. Add it to `src/ui/tokens.ts` (today `theme.ts` + `ui.tsx`), dark **and** light; a colour with one scheme is a
   type error there.
3. If the Keycloak page or the brand script shows the same thing, change the copy (§2, §7, §9) and say so in the
   commit message; the Keycloak copy waits for the owner's go to ship.
4. Use it. If a screen had a literal for it, the count map entry for that file goes down in the same commit.
5. `cd app && npx tsc --noEmit && npm run test:unit` (the gate is in `test:unit`), then the dev-client smoke if a
   `.native` file changed (ENGINEERING.md §6).

**To retire a literal without a new token**: find the step it snaps to (§5.1, §5.2: at most 2 pt), replace, lower
the count. A value that fits no step and no role is a design question, not an exemption: ask here before adding a
sixth radius.

**Never**: a hex in a screen file; `maxFontSizeMultiplier` inline; a `shadow*` outside `PaperCard`; a `GlassView`
outside the input bar; a font size under 11; white text on amber; amber text on light.
