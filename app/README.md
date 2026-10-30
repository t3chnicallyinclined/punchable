# Punchable v0 (app)

Punchable's app: a comic's joke-writing app over their own knowledge base. It is React Native + Expo (Expo Router, TypeScript). For now it runs on the web through react-native-web; the iPhone build comes next.

- **UX spec:** the approved v2 prototype ("Backstage Amber": `#111214` background, `#f5b544` amber, Space Grotesk + Inter).
- **Data contract:** `docs/KB_SCHEMA.md`.
- **API shapes:** `api-dev/API.md`.

## Start (two terminals)
```
python <repo>\api-dev\server.py
cd <repo>\app && npm run web
```
| What | URL |
|---|---|
| **App** | **http://localhost:8798/** (narrow the window for the phone layout; at 900 px and wider a sidebar replaces the tabs) |
| API | http://127.0.0.1:8796/api/health |

- The API reads `comedy\kb\v0\graph.json` (it never writes it) and appends every decision to `comedy\kb\v0\decisions.jsonl`.
- **AI-first:** the API lays the AI's `inferred.json` and each set's `analysis.json` over the graph. Precedence is the comic's decisions > inferred > proposed. Inferred things show as fact with a small **AI** badge, and nothing has to be reviewed.
  - Until the infer pipeline writes those files, `api-dev/fixtures/` stands in.
  - Real files are picked up on the next request. Today and the Set screen poll every 15 s.
  - See `api-dev/API.md`, "The AI layer".
- Uploads and recordings land in `comedy\tapes\inbox\` as "queued". No pipeline runs yet.
- To point the app at another API:
  - `EXPO_PUBLIC_API_URL=http://127.0.0.1:NNNN npm run web`, or
  - on the web only, open `http://localhost:8798/?api=http://127.0.0.1:NNNN`. This is remembered for the tab.
- If the app was started with `CI=1`, Metro doesn't reload when files change. Start it without `CI=1` while developing.
- Punchable uses 8796 (API) and 8798 (app); the explorer uses 8795.

## Screens (UX_SPEC.md, 2026-10-26 rebuild)
Tabs: **Today · Desk · ● Capture · Shows · Boards**; Settings and Learned sit behind the avatar. At ≥ 900 px a sidebar (Import, the tabs, Notes, the pinned Working set) replaces the tabs.

| Screen | What it does |
|---|---|
| **Today** (`/`) | Slots reorder by mode: after a show (new-video banner → last set → Working set → one insight), show day (tonight's Working set + 3 to try → last set), other days (Working set → write next → one insight → last set). `?mode=` overrides. Renders from the phone cache offline. |
| **Desk** | Search or ask (a question runs Ask; words run hybrid search), Jokes / Notes / Pipeline, facets Stage · Topic · Persona · Craft · Attitude · On stage · Year, four sorts, virtualized rows (short title, strength dots, nights, stage word, ＋). |
| **Joke** | Best stage version with the reveal highlighted, On stage sparkline (one dot per night, fragments merged, unreliable rooms hollow), Connect by creative intent (≤ 3 / 5 per group, proven first, Show N more, Long shots), Brainstorm drafts (dashed, “Draft”), Versions by year, Notes. |
| **Set** | laugh-s/min + room heat in words, % laughing, rolling laughs/min, longest laugh (bit), longest quiet; density timeline; vs Working set; New tonight; best clips; rows with rating words derived from × room; “This was…” and rating sheets. |
| **Shows** | List / Calendar / Map (Leaflet + OSM on web, react-native-maps natively), venue pages. |
| **Boards** | Storyboards (drag or ↑↓, transitions, Story health, Best next, bridge drafts incl. 202 queued, export, promote) and the **Working set** (versions, last nights, per-bit strips, try tonight, lock bit/line, Perfected, Set list that keeps the screen awake). |
| **Capture** | Import from Photos (native; web = files + YouTube link, same-night grouping), voice note, record a set, upload. |
| **Settings** | Where your material goes, provider, toggles, export, typed-confirm delete, the AI audit view. |

AI presentation: one `[AI]` badge on the claim, confidence in words at rest, one-line why, and the **Why sheet** (evidence → Open the doc paragraph or ▶ from 4 s before). “This is right” / “Not this” exist only there. Every decision has Undo.

Laugh rules live in `src/lib/laughs.ts` (unit-tested over every real `analysis.json`).

## Contract routes, auth, dev fixtures
- `src/lib/creative.ts` is the typed client for `docs/API_CONTRACT.md`. While a route isn't live on the API you point at, a **dev build only** answers it from `public/dev-fixtures/` (the creative layer's demo output; `npm run dev-fixtures` copies it; gitignored, never shipped) and shows a purple **DEV FIXTURE** banner. Production builds never fall back.
- Fields and routes the app still needs are listed under “Requested by app” in `docs/API_CONTRACT.md`.
- `src/lib/auth.ts` (the server agent's) is imported once in `src/app/_layout.tsx`: bearer on every call, token on `/api/live`, any 401 → `/login`. Off against a local API.

## Test
```
npm run test:unit      # laugh rules (+ every analysis.json under REAL_CORPUS when you have one)
npx tsc --noEmit
```
The browser end-to-end suites ran against the author's own material and are not part of the public repo.

Web bundle (`npx expo export --platform web`, gzip): 332 → 362 KB first load (entry + common + layout + Today), 357 → 464 KB all JS; Leaflet is its own lazy chunk; gesture-handler/Reanimated are native-only.

## Pinned versions (from `create-expo-app` 5.1.5, default template, 2026-10-26)
| Package | Version |
|---|---|
| expo | 58.0.19 |
| expo-router | 58.0.26 |
| react / react-dom | 19.3.0 |
| react-native | 0.88.0 |
| react-native-web | 0.21.4 |
| typescript | 6.0.3 |
| react-native-svg | 15.16.0 |
| react-native-maps / leaflet | 1.29.11 / 1.9.4 |
| expo-media-library / expo-document-picker / expo-keep-awake | 58.0.12 / 58.0.4 / 58.0.3 |
| expo-audio | 58.0.10 |
| @expo-google-fonts/inter / space-grotesk | 0.4.2 / 0.4.1 |

`package.json` pins every dependency exactly, and `package-lock.json` is committed.

The template's typed routes and React Compiler experiments are switched off. `web.output` is `single` (an SPA).
