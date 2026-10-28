# Engineering rules for the app

The rules that keep a TestFlight build from failing in a way the desk never saw. Written after 2026-10-28, when six
`*.native.tsx` files imported `./<their own basename>` (the web twin, on the desk; the file itself, on the phone), Metro
recursed until the stack overflowed, every route failed, and TestFlight builds 4-6 crashed after sign-in. Nothing had
run `tsc`, the unit tests, `expo-doctor` or a bundle before the build. Now something does (`.github/workflows/app.yml`),
and the static tests under `app/tests/static.*.test.mjs` pin the rules below.

The product direction is `docs/UX_2026-10.md`; the iPhone design rules are `docs/design/HIG.md`; the code-organisation
and performance plan is `docs/design/ARCHITECTURE.md`.

## 1. Platform splits

Metro picks a file per platform by extension, most specific first:

| Platform | Resolution order for `import './X'` |
|---|---|
| iOS | `X.ios.tsx` → `X.native.tsx` → `X.tsx` |
| Android | `X.android.tsx` → `X.native.tsx` → `X.tsx` |
| Web | `X.web.tsx` → `X.tsx` |

(`.ts` after `.tsx` at each step; `index.*` inside a directory last.)

**The convention: `<Name>Base.tsx` + `<Name>.tsx` shim + `<Name>.native.tsx`.**

| File | Holds | Imports |
|---|---|---|
| `<Name>Base.tsx` | the shared implementation, props type, pure helpers; the web/default behaviour | never a platform-only module (no gesture-handler, Reanimated, expo-symbols, react-native-maps) |
| `<Name>.tsx` | **a shim only**: `export * from './<Name>Base';` (plus `export { X as default }` if needed) | nothing else |
| `<Name>.native.tsx` | the phone implementation | `./<Name>Base` for the shared parts; **never `./<Name>`** |

Examples on disk: `src/lib/RowMenu*.tsx`, `src/lib/SwipeRow*.tsx`, `src/lib/PanZoom*.tsx`, `src/lib/recorder*.ts`,
`src/lib/holdToTalk*.ts`. The screen shell (`Screen.tsx` / `Screen.native.tsx`) and the icons (`Icon.tsx` → `IconSvg.tsx`
/ `Icon.native.tsx`) are the two pairs still to be brought to the pattern (ARCHITECTURE.md, S1).

When to use the other extensions instead:

| Use | When | The plain file serves |
|---|---|---|
| `X.ios.tsx` | iOS differs from Android **and** the web (SF Symbols, Liquid Glass, the system context menu) | Android and the web |
| `X.web.tsx` | only the web differs (Leaflet, the browser's speech recognition, `sessionStorage`) | both phones |
| `X.native.tsx` | both phones differ from the web (gesture-handler, Reanimated, expo-audio, the Photos library) | the web |

Never have `X.native.tsx` **and** `X.ios.tsx` without `X.android.tsx`: iOS would take `.ios`, Android `.native`, and two
different components would share one name. `tests/static.twins.test.mjs` fails on that, on a `.native` file with no web
twin, and on a `.web` file with no phone twin.

Inside a split file, **a `Platform.OS === 'ios'` branch is a second mechanism for the same decision**: in the web twin it
is dead code; in the `.native` twin it is an iOS/Android fork that belongs in `X.ios.tsx`. `tests/static.platform.test.mjs`
keeps a shrinking list of the files that still do it.

## 2. Never import your own basename

From `X.native.tsx` (or `X.ios.tsx`, `X.web.tsx`), `import ... from './X'` **is a self-import on the platform that
resolves to you**. Metro does not warn; the module evaluates itself until the stack overflows, and because every route
imports the UI kit, every route dies. Import `./XBase` (or the concrete module, e.g. `./IconSvg`) directly.

Pinned by `tests/selfImport.test.mjs` (the `.native` case) and `tests/static.twins.test.mjs` (every variant). A plain
`X.tsx` must not `export * from './X'` either (that is the same loop on the web).

## 3. Import cycles

A cycle (`a` imports `b` imports `a`) is read-before-initialised: a component that is `undefined` at render, or a
function that is not a function, on the phone only and only on a cold start. Metro prints "Require cycle" to the
console nobody reads on the device. `tests/static.cycles.test.mjs` resolves the graph the way Metro does (once for iOS,
once for the web; value imports only) and fails on any cycle. Today there are none; `KNOWN_CYCLES` stays empty.

Barrel files (`index.ts` that re-exports a directory) are the usual way cycles arrive: two siblings importing each
other through the barrel. The policy is in ARCHITECTURE.md §1: a barrel per **layer** (`src/ui`, `src/api`), never per
feature, and a layer never imports a feature.

## 4. Routes

`src/app/**` holds route files only. Each one:

- has a **default export** (the screen); a file without one is a blank screen with a warning on the Metro console;
- exports **no other component** (a second component export turns a route into a shared module, and expo-router
  complains about non-route exports). `tests/static.routes.test.mjs` keeps the short list of the ones still doing it;
- uses `unstable_settings = { anchor: '<file>' }`, not `initialRouteName`, which expo-router has deprecated
  (docs.expo.dev/router/advanced/router-settings: "`initialRouteName` is deprecated", use `anchor`). The three tab
  layouts still on `initialRouteName` are listed in the test and come off it with ARCHITECTURE.md S3.

Keep the screen thin: the route file composes feature components and hooks; logic lives under `src/lib` (today) /
`src/features` (ARCHITECTURE.md §1).

## 5. Dependencies

- **`npx expo install <package>`**, never bare `npm install`, for anything Expo or React Native: it resolves the
  version the SDK was tested with (the pinned-versions table in `app/README.md`).
- After adding or upgrading: **`npx expo-doctor` must pass every check** (today "20/20 checks passed"; the count moves
  with expo-doctor's version, so the rule is N/N). It is expo-doctor that catches a missing native peer (expo-asset,
  required by expo-audio, was absent on 2026-10-28 and TestFlight build 4 crashed at launch). `npx expo install --check`
  / `--fix` aligns versions.
- Pin exact versions in `package.json`; commit `package-lock.json`; `npm ci`, never `npm install`, on a clean checkout.
- A config plugin added by `expo install` goes into `app.json`'s `plugins` (the commit `b9f008d` is the shape).
- No `console.log` in app code outside `if (__DEV__)`: in a bundled app every console call stalls the JS thread
  (reactnative.dev/docs/performance, "Using console.log statements"). `console.warn` / `console.error` are for the crash
  reporter. Pinned by `tests/static.platform.test.mjs`.

## 6. The build checklist

Everything below must be green **before `eas build`**, and `.github/workflows/app.yml` runs the same list on every push
and pull request that touches `app/`:

| Step | Command (from `app/`) | Catches |
|---|---|---|
| 1 | `npm ci` | a lockfile out of step with `package.json` |
| 2 | `npx tsc --noEmit` | types |
| 3 | `npm run test:unit` | the laugh rules, routes, session, say, uploads, the self-import rule |
| 4 | `node --experimental-strip-types --no-warnings tests/static.<cycles,twins,routes,platform>.test.mjs` | cycles, missing twins, route shape, console/Platform forks (until `test:unit` lists them, §7) |
| 5 | `npx expo-doctor` → N/N | dependency and config issues, missing native peers |
| 6 | `npx expo export --platform web` | every route bundles on the web; a bad import fails here |
| 7 | `npx expo export --platform ios` | **the iOS JS bundle + Hermes bytecode, with the `.ios`/`.native` resolution, no Xcode** (no `ios/` directory needed; proven on a machine without Xcode, 2026-10-28: 2011 modules, one `.hbc`) |
| 8 | the manual smoke on the dev client: sign in, open each tab, push one screen per tab, pull to refresh | what only a device shows |

Then `eas build --profile production` (`app/eas.json`), and the TestFlight notes name the commit.

What the iOS export does **not** catch: a runtime recursion like the 2026-10-28 one (the bundle was fine; the module
evaluated itself at runtime). That is what §2's tests are for, and why step 8 exists.

## 7. The pre-commit plan

The commit guard lives in `tools/guard/hooks/pre-commit` (installed by `tools/guard/install.ps1`; its logic is
`tools/guard/check.py`). The static tests take under a second together (874 ms for the five on 2026-10-28), so they
belong in the hook, but only when something under `app/` is staged. Proposed lines, inserted **before** the final
`exec` in the hook (the builder edits the hook; this file only proposes):

```sh
# App static guards (docs/ENGINEERING.md §7): under 1 s; only when something under app/ is staged.
if git diff --cached --name-only --diff-filter=ACMR | grep -q '^app/'; then
  ROOT="$(git rev-parse --show-toplevel)"
  for t in "$ROOT"/app/tests/selfImport.test.mjs "$ROOT"/app/tests/static.*.test.mjs; do
    node --experimental-strip-types --no-warnings "$t" > /dev/null 2>&1 \
      || { echo "pre-commit: $(basename "$t") failed; run it from app/ for the lines"; exit 1; }
  done
fi
```

Each test resolves `src/` relative to its own file, so the hook's working directory does not matter. `node` 22.6+ must
be on the hook's PATH (`--experimental-strip-types`); Node 22.16 is what the desk and CI use.

Adding the static tests to `npm run test:unit` (the builder edits `app/package.json`; the string today is one long
`&&` chain): append

```
&& node --experimental-strip-types --no-warnings tests/static.cycles.test.mjs
&& node --experimental-strip-types --no-warnings tests/static.twins.test.mjs
&& node --experimental-strip-types --no-warnings tests/static.routes.test.mjs
&& node --experimental-strip-types --no-warnings tests/static.platform.test.mjs
```

or, better, replace the chain with a two-line runner (`tests/run.mjs`: glob `tests/*.test.mjs`, run each with
`node --experimental-strip-types`, exit non-zero on the first failure) so a new test is picked up by being added to the
folder. Until then CI runs the static tests as their own step.

## 8. Adding a static test

- Name it `tests/static.<rule>.test.mjs`; one rule per file; Node only (`node:fs`, `node:path`), no dependencies.
- Resolve `src/` from `import.meta.url`, walk it, print `FAIL <file>:<line>: <what and the fix>` per hit, and
  `ALL PASS` / `N FAILED` last; exit 1 on failure.
- If the rule is not yet true on `main`, add a **baseline** (a `KNOWN_*` set of the current offenders) that fails on a
  new offender **and** on a stale entry, so the list can only shrink.
- Prove it can fail: plant the mistake in a scratch copy and watch it go red (the four shipped on 2026-10-28 were each
  checked that way), or give it a `--self-test` mode like `static.cycles.test.mjs`.
- Keep the whole set under 5 s; these run on every commit.
