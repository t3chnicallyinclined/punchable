# The app's code organisation and performance plan

An audit of `app/src/**` (2026-10-28, `main` at 2d703de; Expo SDK 58, React Native 0.88, New Architecture, Hermes) with
the work ordered by value ÷ effort. The builder executes it after the observability and iOS-auth agents land; the rules
it leans on are in `docs/ENGINEERING.md`. Effort: **S** < half a day, **M** one to two days, **L** more.

Every claim below names the file and line it comes from. Docs cited: docs.expo.dev (router settings, async routes,
fonts, React Compiler, splash screen, sqlite kv-store, analyzing bundles, debugging tools), reactnative.dev/docs/
performance, shopify.github.io/flash-list.

## 0. The ten findings that matter

| # | Finding | Evidence | Effort |
|---|---|---|---|
| 1 | **The iOS app bundles all 23 font files (≈6.6 MB) and still waits for 8 of them at runtime.** `@expo-google-fonts/inter`'s `index.js` requires every weight (18 Inter + 5 Space Grotesk files appear in the iOS export's asset list); `useFonts` then loads 8 of them before the first frame. | `app/src/app/_layout.tsx:102`; `npx expo export --platform ios` asset list, 2026-10-28; `node_modules/@expo-google-fonts/inter/index.js:9` | S |
| 2 | **"Renders from the phone cache offline" and the jot outbox are not true on the phone.** Both use `globalThis.localStorage`, which React Native does not provide (no match for `localStorage` under `node_modules/react-native`); on iOS the cache is memory-only and the outbox is lost when the app is killed. Eight sites do this. | `app/src/lib/api.ts:122-133`, `say.ts:54-66`, `prefs.ts` (web branch only), `settings.tsx:173,191`, `today.tsx:57-63`, `workingset/[id].tsx:16,71`, `boards.tsx:16` | S |
| 3 | **Cold start serialises four waits before any content**: fonts ∥ the prefs file → the first frame → OIDC discovery (network) → token refresh (network) → `GET /api/tonight/screen` (network, nothing cached on the phone, see 2) → paint. Plus a `GET /api/backend` probe before the live socket, and a `/api/health` tick. | `_layout.tsx:86-108`, `prefs.ts:33-58`, `_layout.tsx:59-80`, `auth.ts:182-194`, `auth.ts:108-109`, `auth.ts:133`, `(tonight)/index.tsx:36`, `api.ts:173-174`, `api.ts:285` | S+M |
| 4 | **The Photos library is scanned at launch** (`getAssetsAsync`, up to 300 videos) from the root layout, for a badge. | `_layout.tsx:34-52`, `photos.native.ts:23-25` | S |
| 5 | **Every `useApi` instance is its own cache and its own fetch**: 49 call sites, 12 polling; one `/api/live` change message calls `load()` in every mounted instance; no in-flight dedupe, no `rev` comparison on the push path; `fetch` has no timeout. | `api.ts:233-312`, `:279-281`, `:59` | M |
| 6 | **The `/api/live` socket reconnects every 5 s flat**, no backoff, no jitter; the hosted API sends no `ETag` and no gzip (`curl -I -H 'Accept-Encoding: gzip' https://punchable.io/api/health`: `Cache-Control: no-store`, no `Content-Encoding`). | `api.ts:195`; curl 2026-10-28 | S (app) + S (API/ingress) |
| 7 | **One UI context carries the input bar, the counts, the snackbar and the actions**; its value changes whenever any screen sets its bar on mount, and 32 route files plus the shell consume it. Theme tokens are a mutated object (`Object.assign(C, …)`) swapped during render, with the whole tree remounted on a scheme change; `RATING` captures `C.*` at module load and goes stale after a switch. | `ui.tsx:508-567`, `theme.ts:138,147-151,188-193`, `_layout.tsx:93,110`, `ui.tsx:52-55` | M |
| 8 | **Writes are confirm-then-refetch**: tap → `await decide()` → snack → `reload()` (two round trips before the row shows its new state); the pieces for optimistic per-item writes (`setData`, `undo(indexes)`) already exist. | `bit/[id].tsx:155,167,292,309`, `list/[id].tsx:128-130`, `api.ts:312` | M |
| 9 | **`src/lib` is 76 files of kit, client, features and platform splits in one folder**; `Screen.tsx` / `Screen.native.tsx` duplicate the refresh, pinned-height and bar/dock blocks instead of sharing a Base; `Icon.native.tsx` and `RowMenu.native.tsx` fork again on `Platform.OS` inside the `.native` file. | `ls app/src/lib`; `Screen.tsx:124-137` vs `Screen.native.tsx:39-52`; `Icon.native.tsx:26`, `RowMenu.native.tsx:15` | M |
| 10 | **Compiler and types left off**: `experiments.reactCompiler: false` (SDK 54+ needs no Babel setup), `typedRoutes: false` with 149 `router.push(... as any)` casts in routes, 11 function-valued `style={(st) => …}` props re-created per render, `initialRouteName` (deprecated for `anchor`) in all three tab layouts, `expo-image` installed but never imported. | `app/app.json` experiments; `grep -c "as any" src/app`; `(tabs)/*/_layout.tsx:9`; `package.json` | S each |

## 1. Structure

### Target layout

```
src/app/                 routes only: compose, no logic (docs/ENGINEERING.md §4)
src/ui/                  the kit: tokens, text, boxes, buttons, sheets, snackbar, icons, the screen shell, haptics
src/api/                 the HTTP client, the live socket, the cache/outbox store, the typed contract clients, the shapes
src/auth/                sign-in (auth.ts), session helpers (session.ts), the AuthGate
src/features/tonight/    the input bar and "say", the conversation, the worker status, the inbox, import, memos
src/features/bits/       the bit page pieces, the maps, why/evidence, laugh rules and meters, merge
src/features/sets/       set lists, boards, "your sets", add-a-joke, bridges
src/features/stage/      the recorder, media playback, uploads, photos
src/platform/            device preferences, the key-value store, action sheets, the tab-bar accessory probe, screen types
```

Rules: `app` → `features` → (`ui`, `api`, `auth`, `platform`); `ui` imports only `platform` and `api` types; `api`
imports nothing above it; a feature never imports another feature (shared things move down a layer). The cycle test
(`tests/static.cycles.test.mjs`) enforces the absence of loops; the layering is a review rule until a second static test
(`static.layers.test.mjs`: an import from a lower layer to a higher one fails) is added with the move (S).

### Move map (`src/lib/X` → target)

| Target | Files |
|---|---|
| `src/ui/` | `theme.ts` (tokens + `useTheme`), `ui.tsx` split into `text.tsx` (T, buildTv), `boxes.tsx` (s, Card, Sheet…), `buttons.tsx` (Btn, SignInBtn), `snack.tsx` (the provider), `skeleton.tsx`, `UIProvider.tsx`; `Icon.tsx` (SVG, plain) + `Icon.ios.tsx` (SF Symbols; delete `Icon.native.tsx`, `IconSvg.tsx` becomes `Icon.tsx`), `Brand.tsx`, `Screen*.tsx` → `ScreenBase.tsx` + shim + `Screen.native.tsx`, `TopBar.tsx`, `TabsShell*.tsx`, `RowMenu*` (`RowMenuBase.tsx`, shim, `RowMenu.ios.tsx`), `SwipeRow*`, `DragList*`, `PanZoom*`, `GestureRoot*`, `haptics.ts`, `feedback.ts`, `layoutRules.ts`, `metrics.tsx` (the meters), `MergeHint.tsx`, `AskButton.tsx` |
| `src/api/` | `api.ts` → `client.ts` (req, headers, ApiError, upload), `live.ts` (the socket), `store.ts` (the cache, in-flight dedupe, rev), `useApi.ts`, `types.ts` (the shapes at the bottom of `api.ts`); `creative.ts`, `simple.ts`, `mysets.ts`, `sets.ts` (the client half), `devFixtures.ts` |
| `src/auth/` | `auth.ts`, `session.ts`, the `AuthGate` component out of `_layout.tsx` |
| `src/features/tonight/` | `say.ts`, `sayRules.ts`, `SayBar.tsx`, `holdToTalk*.ts`, `ask.tsx`, `hear.tsx`, `worker.tsx`, `memos.tsx`, `importgroup.ts`, `newBit.ts`, `newSetList.ts`, `SetBar` (out of `(tonight)/index.tsx`) |
| `src/features/bits/` | `jokemap.tsx`, `galaxyLayout.ts` + `Galaxy.tsx` (the all-bits map; `allmap.ts` is its predecessor, kept until nothing imports it), `mapPins.ts`, `VenueMap*.tsx`, `leafletMount.ts`, `why.tsx`, `laughs.ts`, `BitRow` (out of `(bits)/write.tsx`) |
| `src/features/sets/` | `boards.tsx`, `AddJokeSheet.tsx`, `bridge.tsx`, `PaperCard` (out of `(sets)/sets.tsx`), the view half of `sets.ts` |
| `src/features/stage/` | `recorder*.ts`, `media.ts`, `uploads*.ts`, `uploadsCore.ts`, `photos*.ts` |
| `src/platform/` | `prefs.ts`, **`kv.ts` (new, §2)**, `actionSheet.ts`, `accessory*.ts`, `screenTypes.ts`, `routeTable.ts` |
| stays / verify | `crash.ts`, `crashReporting.ts` (the observability agent's; `src/platform/` once landed) |

The move is mechanical (`git mv` + import rewrites) and the tests keep their paths working through the aliases.
Do it **after** the two agents' branches merge, in one commit per layer, each commit green on the checklist.

### Barrels and aliases

- **One barrel per layer** (`src/ui/index.ts`, `src/api/index.ts`, `src/platform/index.ts`), re-exporting named
  modules; **no barrel per feature** and none under `src/app`. A barrel that re-exports a sibling which imports the
  barrel is how cycles start; the cycle test is the net.
- `tsconfig.json` paths: `@/ui/*`, `@/api/*`, `@/auth/*`, `@/features/*`, `@/platform/*` beside the existing `@/*`
  (the same mechanism the app already uses for `@/lib/...`, so Metro needs nothing new).
- Platform-split files follow ENGINEERING.md §1; the twins and platform tests keep them honest.

## 2. Startup latency

What runs at import time today (every module in the root layout's graph: 2516 modules on iOS at 2d703de, 2011 before @sentry/react-native):
`SplashScreen.preventAutoHideAsync()` (`_layout.tsx:22`, correct per the splash-screen docs); `WebBrowser.
maybeCompleteAuthSession()` and `AUTH_ENABLED` (`auth.ts:24,74`); `AccessibilityInfo.isReduceMotionEnabled()`
(`ui.tsx:60`); `require('expo-haptics')` (`haptics.ts:9`); the prefs web read (`prefs.ts:24-31`). None is heavy; the
cost is the chain in finding 3.

| Step | Today | Plan | Effort |
|---|---|---|---|
| Fonts | `useFonts` with 8 faces blocks the first frame (`_layout.tsx:102-108`); the whole Inter and Space Grotesk packages are bundled | **Embed with the `expo-font` config plugin** listing exactly the 8 files under `node_modules/@expo-google-fonts/...` ("Fonts are available immediately when the app starts"; recommended by the fonts guide). Keep `useFonts` for the web only (`fonts.web.ts`). Import per-weight subpaths (`@expo-google-fonts/inter/400Regular`) wherever a runtime import stays, so only used files ship. Acceptance: the iOS export lists 8 `.ttf`; no font wait on the phone. | S |
| Prefs | the phone awaits a dynamic import of `expo-file-system/legacy` and a file read before the first frame (`prefs.ts:33-58`, `_layout.tsx:86-91`) | `src/platform/kv.ts` over `expo-sqlite/kv-store` (`getItemSync` / `setItemSync`, SDK docs) read synchronously at import; `prefsReady` goes away | S |
| Splash | hidden the moment `ready` flips (`_layout.tsx:105-107`), no fade | `SplashScreen.setOptions({ fade: true, duration: 200 })` at module scope (iOS; docs: default 400 ms) so the hand-off is a fade, not a cut | S |
| Auth gate | discovery fetch + refresh on every cold start (`auth.ts:108-109,133`); the gate shows a spinner meanwhile | (a) persist the discovery document in `kv` with a 24 h TTL (or build it from `ISSUER`: Keycloak's endpoints are `${ISSUER}/protocol/openid-connect/{auth,token,logout}`); (b) keep `expiresAt` with the refresh token and **skip the refresh when the access token has > 60 s left** (store the access token in SecureStore too, the owner's call: it is a bearer, Keychain-protected, 5-minute life). Warm relaunch: 0 RTT before the gate opens. | S |
| First data | `GET /api/tonight/screen` after the gate, nothing cached on the phone (finding 2) | with `kv`, the cached screen paints before the request; the request runs in parallel with the token refresh when the token is still valid | S (falls out of `kv`) |
| Photos badge | `getAssetsAsync` at launch (`_layout.tsx:34-52`) | run it from the Capture/Create surface on focus, or after the first Tonight paint via `InteractionManager.runAfterInteractions`, and cache the count in `kv` | S |
| Hermes | bytecode is produced by the export (`entry-*.hbc`: 5.0 MB at 7aee815, 7.1 MB at 2d703de with @sentry/react-native; verified) | nothing to do; `--no-bytecode` only for source-map analysis | – |
| Lazy routes | `extra.router.asyncRoutes: { web: true }` (`app.json`; still read by `@expo/metro-config/build/rewriteRequestUrl.js:90`) | async routes are **default-on for web in SDK 58 and unsupported for native production** (async-routes docs), so: delete the `extra.router` entry (or move it to the `expo-router` plugin options as the docs now show). On the phone, shrink the root graph instead: the root layout imports `photos` (expo-media-library), `importgroup`, `boards`, `why` (`_layout.tsx:15-20`); move those behind the screens that use them | S |

## 3. Interaction latency

| Area | Today | Plan | Effort |
|---|---|---|---|
| Lists | `bits.tsx:148-150` and `desk.tsx:229-237` are `FlatList` with `getItemLayout`, a fixed row height, `windowSize`, `removeClippedSubviews` (good); the set page maps rows inside a `ScrollView` (`set/[id].tsx:144-185`); connections on the bit page are a memoised map (`bit/[id].tsx:319`) | measure first (§7). Where a list exceeds ~50 rows on his data (set rows, connections, inbox): **FlashList v2** (`npx expo install @shopify/flash-list`; v2 targets the New Architecture, which SDK 58 uses). Keep `FlatList` where `getItemLayout` already makes it cheap. | M |
| Re-render storms | one UI context (`ui.tsx:565-567`) whose value changes with `bar`, `toSort`, `workingSet`, `lastRoot`, `importCount`; every `useUI()` caller re-renders on each | split into `UIActions` (stable callbacks), `UIBar` (the input bar config; only the shell reads it), `UICounts` (badges), and `useSnack()`; or one tiny external store with `useSyncExternalStore` and selector hooks | M |
| Theme | `C` mutated in place and swapped **during render** (`_layout.tsx:93`, `theme.ts:147-151`); the remount on `key={scheme}` makes it work; `RATING` (`theme.ts:188-193`) is wrong after a switch | `useTheme()` returning the frozen `DARK` / `LIGHT_TOKENS`; `StyleSheet.create` per scheme cached in a `Map`; `C` stays as the dark default for non-React code. Fix `RATING` first (S) whatever else happens. The remount can then go. | M (L to touch every `C.` read; do it with the §1 move, file by file) |
| Memoisation | 10 `memo()` components, inline `style={(st) => …}` in 11 places, inline handlers everywhere | turn on **React Compiler** (`experiments.reactCompiler: true`; "Babel is automatically configured in Expo SDK 54 and later"); run `npx react-compiler-healthcheck` first; fix what it flags (the in-render `setScheme`, the mutated `C`/`s`/`tv` are the known ones); then delete manual memo where the profiler shows it redundant. `eslint-config-expo` 58 carries the compiler lint rules. | S to enable, M to clean |
| Optimistic writes | confirm-then-refetch (finding 8) | per item: `setData(prev => …)` first, `haptics.selection()` at press, then `decide()`; on success keep, on failure revert + snack; Undo via the existing `undo(indexes)` and the local inverse. Start with the bit page's stage change and the set list's rename/reorder. | M |
| Request dedupe | finding 5 | `src/api/store.ts`: `Map<path, {data, rev, inflight}>`; `useApi` subscribes to the entry; one fetch per path at a time; a live message with `rev` equal to the entry's skips the refetch; focus revalidation becomes free when `rev` is unchanged | M |
| Live socket | fixed 5 s retry (`api.ts:195`); `/api/backend` probe (`:173-174`) | exponential backoff 1 s → 30 s with jitter, reset on open; reconnect on AppState active (exists, `:206-216`) and on network change (`expo-network` `useNetworkState`, `npx expo install`); resend the auth message after every reconnect; drop the probe on the hosted build (`EXPO_PUBLIC_AUTH=1` means SurrealDB) | S |
| Keyboard / gestures | `KeyboardAvoidingView` on iOS in both shells (`Screen.native.tsx:64`, `Screen.tsx:138`); drag, swipe and pan run as Reanimated worklets on the UI thread (`DragList.native.tsx:6,20,71-73`, `SwipeRow.native.tsx`, `PanZoom.native.tsx`) | keep; move the KAV into `ScreenBase` once (removes two baseline entries in the platform test); check `runOnJS` is called only at gesture end | S |
| Images | RN `Image` for Photos thumbnails (`import.tsx:100`) and the avatar (`ui.tsx:313`); `expo-image` 58.0.17 installed, unused | use `expo-image` for both (disk + memory cache, `transition`, `placeholder`; "Transitioning between images when the source changes (no more flickering!)"), or `npm uninstall` it: an unused native module still ships in the binary | S |

## 4. Network on bad wifi

| Item | Today | Plan | Effort |
|---|---|---|---|
| Timeouts | none (`api.ts:59`): a stalled request leaves `loading` true forever | `AbortController` + 15 s for reads, 30 s for writes, 0 for uploads (background session handles those) | S |
| ETag / If-None-Match | the API sends `Cache-Control: no-store`, no `ETag` (curl, 2026-10-28) | API: `ETag` = the response's `rev` (the API already has one per `/api/health`); app: send `If-None-Match` from the store, treat 304 as "keep"; the live `rev` and the ETag become the same value | S (API) + S (app) |
| gzip | not negotiated at the ingress (`Accept-Encoding: gzip` gets no `Content-Encoding`) | turn on compression at the ingress for `application/json` (infra; RN's `fetch` decodes transparently); measure `/api/tonight/screen` and `/api/bits` bytes before/after | S (infra) |
| Payload trims | unmeasured | log sizes for the five launch requests in the baseline (§7); trim fields the screen does not read (the shapes in `api.ts` mark several as optional "requested by app") | S |
| Retry / backoff | uploads have it (`uploadsCore.ts:4-16`, with a background session on iOS, `uploads.native.ts:77`); reads and `decide` / `post` have none | reads: no retry (pull-to-refresh and the live socket cover it); writes: one retry on a network error **only with an idempotency key** (`X-Idempotency-Key`, a UUID per tap; the API dedupes for 24 h); without the key, no automatic retry | S (app) + M (API) |
| Offline outbox | jots (`say.ts:54-70`) in `localStorage`: memory-only on the phone | on `kv` (finding 2); the decisions (`decide`) join the same outbox behind the idempotency key, so a stage change on a train platform is kept and sent | S then M |

## 5. Perceived speed

- **Skeletons, not spinners**: `Skeleton` exists (`ui.tsx`, used on Tonight, `index.tsx:99`) and 8 route files use it;
  5 files still show `ActivityIndicator` mid-screen. The gate's spinner is right (nothing to shape yet); the lists are
  not. Acceptance: no `ActivityIndicator` under `src/app` except the gate and the upload progress.
- **Haptics timing**: fire `haptics.selection()` / `light()` at the press, never after the `await` (today
  `list/[id].tsx:128` fires `success()` after the write resolves, which is right for *success*; the selection tick at
  press is missing).
- **Transitions**: native stack and native tabs already (HIG.md B2/B3); keep `animation: 'none'` on the `(tabs)`
  screen; a form sheet's first paint should not wait on a request (render from the store, then revalidate).
- **Prefetch on tab focus**: after Tonight's first paint, `InteractionManager.runAfterInteractions(() =>
  store.prefetch(['/api/bits', '/api/sets']))` so the first tab switch paints from the store; `useFocusEffect` already
  revalidates (`api.ts:303-311`) and becomes a no-op when `rev` is unchanged.
- **The gate**: with the cached discovery and a still-valid access token, a warm relaunch shows Tonight from `kv`
  immediately and only *then* refreshes; the "Checking your sign-in" cover appears only on a true cold start.

## 6. Dependencies to add or drop

| Package | Why | Command |
|---|---|---|
| `@shopify/flash-list` (v2) | §3 lists, only after measuring | `npx expo install @shopify/flash-list` |
| `expo-sqlite` | `kv-store` (sync key-value) for `src/platform/kv.ts` | `npx expo install expo-sqlite` |
| `expo-network` | reconnect the socket on a network change | `npx expo install expo-network` |
| `expo-image` | already installed: use it or remove it | – |
| `@expo-google-fonts/*` | keep, but reference per-file paths in the `expo-font` plugin | – |

After each: `npx expo-doctor` N/N, the checklist, and (native modules) a new dev client (`eas build --profile dev-device`).

## 7. Measurement (no Xcode on the desk)

What works in SDK 58 without a Mac:

| Tool | For | How |
|---|---|---|
| `performance.now()` / `performance.mark()` marks | every number in the baseline table | a `src/platform/perf.ts` with `mark(name)` and `measure(a, b)`; marks: `js_start` (first line of `_layout.tsx`), `first_frame` (the root's first `useEffect`), `gate_open` (AuthGate unmounts), `tonight_data` (first non-null `/api/tonight/screen`), `tap:<route>` at `onPress`, `paint:<route>` in the screen's first effect after `requestAnimationFrame`. Print under `__DEV__`; in release, post to the observability agent's `/api/metrics` (`api-dev/metrics.py`) as `{name, ms, build}` once per launch |
| `@sentry/react-native` (the observability agent's) | **cold/warm app start, slow and frozen frames** on real TestFlight devices: the only native-side start timing available without Instruments | confirm its performance tracing is on in the agent's `_layout.tsx` changes; read App Start in Bugsink/Sentry |
| React Native DevTools (press `J` in the Metro terminal) | the Profiler (re-render counts per interaction) and the Network panel, debug builds only | docs.expo.dev/debugging/tools |
| Perf monitor (dev menu) | JS and UI FPS while scrolling Bits / dragging a set list | dev client |
| Expo Atlas | what is in the 7.1 MB `.hbc` and which import pulled it in | `EXPO_ATLAS=true npx expo export --platform ios` then `npx expo-atlas .expo/atlas.jsonl` (treat the file as source code) |
| CI step summary | bundle bytes per push | `.github/workflows/app.yml` "Bundle sizes" |

### Baseline (fill before the first change, then after each ranked item)

Device: the owner's iPhone, release build from TestFlight, wifi; three runs, median. "Cold" = app killed, phone awake.

| Metric | How | Before | After |
|---|---|---|---|
| Cold start → first frame (ms) | Sentry App Start cold; `js_start → first_frame` | | |
| Cold start → gate open (ms) | `js_start → gate_open` | | |
| Cold start → Tonight data painted (ms) | `js_start → tonight_data` | | |
| Warm relaunch → Tonight painted (ms) | same marks, app backgrounded 1 min | | |
| Requests before Tonight paints (count, bytes) | DevTools Network / API access log | | |
| Tap Bits tab → list painted (ms) | `tap:bits → paint:bits` | | |
| Tap a bit → page painted (ms) | `tap:joke → paint:joke` | | |
| Bits scroll, 300 rows (JS FPS min) | perf monitor | | |
| Set list drag (UI FPS min) | perf monitor | | |
| Requests per live change, 3 screens mounted | API access log | | |
| iOS `.hbc` bytes | CI summary | 7.1 MB (2d703de; 5.0 MB before Sentry) | |
| Font files in the iOS export | export asset list | 23 | 8 |
| TestFlight install size (MB) | App Store Connect | | |

## 8. Implementation order (value ÷ effort), with acceptance checks

| # | Item | Effort | Acceptance |
|---|---|---|---|
| 1 | Fonts: `expo-font` plugin with the 8 files; per-weight imports; `useFonts` web-only | S | iOS export lists 8 `.ttf`; no font wait before `first_frame`; app size down ≈ 6 MB |
| 2 | `src/platform/kv.ts` on `expo-sqlite/kv-store`; replace the 8 `localStorage` sites; prefs read sync | S | kill the app offline → Tonight paints from cache; a jot typed offline is there after relaunch; `prefsReady` deleted |
| 3 | `anchor` for `initialRouteName` in the three tab layouts; delete the three baseline entries | S | `static.routes` passes with an empty `KNOWN_INITIAL_ROUTE_NAME`; a cold deep link to `/joke/x` still has Back to Bits |
| 4 | Defer the Photos scan; drop `extra.router`; fix `RATING`; `SplashScreen.setOptions` fade | S | no MediaLibrary call before `tonight_data`; rating colours right after a scheme switch |
| 5 | Auth: cached discovery; skip refresh while the token is valid | S | warm relaunch: 0 requests before the gate opens; cold: 1 |
| 6 | Request layer: timeout, in-flight dedupe, `rev`-aware refetch, shared store; socket backoff + resend auth; `expo-network` reconnect | M | one request per path per change with 3 screens mounted; a stalled request errors at 15 s; reconnect gaps ≤ 30 s |
| 7 | UI context split; `useTheme()`; remove the remount | M | Profiler: setting a screen's bar re-renders the shell only; scheme switch without remount |
| 8 | Structure move (§1) with `ScreenBase`, `Icon.ios`, `RowMenu.ios`; `static.layers` test | M | all tests green; `KNOWN_PLATFORM_BRANCHES` empty; `src/lib` gone |
| 9 | React Compiler on; healthcheck clean; manual memo pruned | S/M | healthcheck passes; Bits scroll re-render count down in the Profiler |
| 10 | Optimistic per-item writes (bit page stage, set list rename/reorder) | M | tap → row updated < 50 ms; failure reverts with a snack; Undo works |
| 11 | FlashList where measured; skeletons for every list; prefetch after first paint | M | lists ≥ 55 JS FPS on 300 rows; no `ActivityIndicator` mid-list; first tab switch paints from the store |
| 12 | API/infra: `ETag` + `If-None-Match`, gzip at the ingress, idempotency key for writes, payload trims | S+M | 304s on unchanged screens; bytes for the five launch requests halved; a retried write is applied once |
| 13 | Typed routes on; the 149 casts removed | L | `typedRoutes: true`, `tsc` clean, zero `as any` on `router.*` |
| 14 | `expo-image` for the avatar and thumbnails, or uninstall | S | one or the other; expo-doctor N/N |

Items 1-5 are a day's work together and remove most of the cold-start chain; 6-7 are the performance core; 8-9
make the codebase findable and cheap to render; 10-14 are polish with measurable wins. Each lands as its own commit,
green on the checklist (ENGINEERING.md §6), with the baseline row it moves filled in.

## 9. What the owner decides

1. **Keep the access token in the Keychain** (item 5) so a warm relaunch skips the refresh, or accept one round trip
   per launch to keep the access token in memory only.
2. **React Compiler** (item 9): turn it on now (a build-time change, revertible, with the healthcheck as the gate) or
   after the structure move.
3. **FlashList** only where the baseline shows a list dropping frames, or everywhere for uniformity.
4. **`expo-image`**: adopt or remove.
5. The idempotency key (item 12) is an API change: the API team's queue, or the app skips write retries.
