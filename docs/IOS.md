# Punchable on iOS

The phone app is Expo SDK 58 / React Native 0.88 / expo-router, built and signed on EAS, distributed through
TestFlight. This page is the operating manual: how sign-in works on the phone and what it depends on, how to run a
dev client against Metro, how to read a crash, how to cut a build, and which native APIs the app leans on.

Public-safe: hostnames here are `punchable.io` and `auth.punchable.io` only; no secrets, no credentials.

## 1. Sign-in on the phone, step by step

Code: `app/src/lib/auth.ts`, `app/src/app/login.tsx`, `app/src/app/auth/callback.tsx`, `app/src/app/auth/logout.tsx`,
`app/src/app/_layout.tsx` (AuthGate), `app/src/lib/api.ts` (bearer, 401, the live socket).

| Step | What happens | Where |
|---|---|---|
| 1 | The app launches with `EXPO_PUBLIC_AUTH=1` (every EAS profile but the simulator one), so `AUTH_ENABLED` is true. `auth.ts` installs the bearer provider on `api.ts`, wraps `fetch` (any API 401 clears the tokens and goes to `/login`) and wraps `WebSocket` (the token is the first message on `/api/live`, never in the URL). | `auth.ts` `install()` |
| 2 | `AuthGate` covers every non-public route until the stored refresh token has been tried once (`getAccessToken()` -> `refreshAsync`). Success = straight in; failure = `/login`. | `_layout.tsx` `AuthGate`, `auth.ts` `checkOnce` |
| 3 | `/login` builds an authorization-code + PKCE (S256) request with `useAuthRequest` against the realm's discovery document (`https://auth.punchable.io/realms/punchable`). One button on the phone: **Sign in**. | `login.tsx`, `auth.ts` `authRequestConfig` |
| 4 | Tapping it calls `promptAsync`, which calls `WebBrowser.openAuthSessionAsync(authUrl, 'punchable://auth/callback')`. On iOS that is an `ASWebAuthenticationSession` with `callbackURLScheme: "punchable"` (expo-web-browser `ios/WebAuthSession.swift`). iOS first shows "Punchable wants to use auth.punchable.io to sign in"; then the Keycloak page (our `punchable` login theme) opens in the sheet. | expo-auth-session `AuthRequest.promptAsync` |
| 5 | He signs in on the Keycloak page (email + password; Google is web-only by owner decision). Keycloak answers with a 302 to `punchable://auth/callback?code=...&state=...`. |  |
| 6 | **The sheet intercepts that redirect itself.** The URL goes to the session's completion handler and comes back as `{ type: 'success', url }` to the waiting `promptAsync`; it is *not* delivered to the app's URL handler, so expo-router never routes it and the `auth/callback` screen never mounts during a normal sign-in (the screen is a no-op on the phone for the rare case it does). | `WebAuthSession.swift` lines 27-57 |
| 7 | `promptAsync` checks `state` against the request and returns `{ type: 'success', params: { code } }`. The login screen's effect calls `completeSignIn(code, request.codeVerifier)` -> `exchangeCodeAsync` with `code_verifier` at the token endpoint. | `login.tsx` effect, `auth.ts` `completeSignIn` |
| 8 | `setTokens` keeps the access token and ID token in memory, stores the refresh token in the keychain (`expo-secure-store`, `AFTER_FIRST_UNLOCK`, retried on a transient failure, logged on a lasting one, never thrown), and notifies `useSession` listeners. `AuthGate` lifts; `router.replace(next)` lands him on Tonight. | `auth.ts` `setTokens`, `saveRefresh` |
| 9 | Every API call goes through `api.ts` -> `headers()` -> `getAccessToken()`, which refreshes 60 s before expiry (`refreshAsync`; the realm rotates refresh tokens). The refresh is a tested state machine (`lib/tokens.ts`): the new refresh token is **stored before** the new set is used or announced; **one refresh in flight** for headers, the live socket, uploads and a 401; **only Keycloak's `invalid_grant` signs him out** (a network failure retries 0.5 s / 1 s, then keeps the session and sends the stale token; an API 401 refreshes through the same path instead of going to `/login`); on `invalid_grant` the stored token is tried once if it differs from the one in memory (rotation grace; the realm's `refreshTokenMaxReuse` is the server-side grace). | `auth.ts` `getAccessToken`, `lib/tokens.ts` |
| 10 | Sign out: `signOut()` clears tokens, then opens Keycloak's end-session URL (`id_token_hint`, `post_logout_redirect_uri=punchable://auth/logout`, `client_id`) in another auth session; the sheet closes on that URL and the app goes to `/login`. | `auth.ts` `signOut`, `session.ts` `endSessionUrl` |

### The Keycloak client this depends on (`infra/keycloak/realm-punchable.yaml`, client `pk-mobile`)

| Setting | Value | Why it matters |
|---|---|---|
| `publicClient` | `true` | No client secret on the phone. |
| `standardFlowEnabled` | `true` | Authorization code flow. |
| `redirectUris` | `punchable://auth/callback` (exact) | Keycloak matches the redirect exactly; `REDIRECT_URI` in `auth.ts` must be byte-identical. |
| `attributes.pkce.code.challenge.method` | `S256` | The token endpoint rejects a code exchange without the matching `code_verifier`. |
| `attributes.post.logout.redirect.uris` | `punchable://auth/logout` | Must match `POST_LOGOUT_URI` or sign-out ends on a Keycloak error page. |
| `defaultClientScopes` | `web-origins acr basic profile email roles pk-api-audience` | `pk-api-audience` puts `aud=pk-api` and the SurrealDB claims on the access token; `profile`/`email` fill the ID token the profile page reads. |
| `webOrigins` | none | Not needed for a native client: the sheet does a top-level navigation, no CORS. |
| realm `accessTokenLifespan` | 300 s | The app refreshes at 240 s. |
| realm `revokeRefreshToken` / `refreshTokenMaxReuse` | `true` / `0` | Rotation: a reused refresh token is `invalid_grant`; the app stores every new one and signs in again on failure. |
| realm `ssoSessionIdleTimeout` / `MaxLifespan` | 14 d / 30 d | How long a phone stays signed in without a fresh sign-in. |
| realm `loginTheme` | `punchable` | The page in the sheet. It is one POST form plus an inline script for the password eye and once-only submit; no redirects, no external JS (`infra/keycloak/theme/login/template.ftl`). |

App side: `app/app.json` `scheme: "punchable"` registers the URL scheme (`CFBundleURLSchemes`); `associatedDomains:
applinks:punchable.io` is for Universal Links into the web routes, not for sign-in (the sheet uses the custom scheme).

## 2. The dev client (see errors on the phone)

A release build has no red box: an uncaught JS error ends the process. The dev client shows it.

```sh
cd app
npx eas-cli@latest build --profile dev-device --platform ios     # an ad-hoc dev client for the registered iPhone
# install it from the build page / QR on the phone, then:
npx expo start --dev-client --tunnel
```

- `--tunnel` works from any network (the phone does not have to be on the PC's Wi-Fi); the QR in the terminal opens in
  the dev client. The dev client's launcher lists recent servers too.
- The dev-client build carries the same env as production (`EXPO_PUBLIC_API_URL=https://punchable.io/api`,
  `EXPO_PUBLIC_AUTH=1`), so sign-in runs for real; only the JS comes from Metro.
- Where errors show: a thrown render error = the red box on the phone and the same stack in the Metro terminal;
  `console.warn`/`console.error` = LogBox at the bottom of the phone and the terminal; native module exceptions
  surface as a JS error with the Swift message (e.g. `AudioRecordingException`, `KeyChainException`).
- Shake the phone (or three-finger long press) for the dev menu: reload, element inspector, performance monitor.
- A new native dependency (anything with `ios/` code or a config plugin) needs a new dev-client build; a JS-only change
  does not.

## 3. The in-app crash record

`app/src/lib/crashReport.ts` wraps React Native's global error handler. A fatal error is written to
`<documents>/last-crash.json` (name, message, stack, path, platform; bearer tokens and JWTs redacted) **before** it is
handed on to React Native, waiting at most 1.5 s for the write; the process then ends as before. The next launch shows
the record on `/login` under **Last crash** with *Copy or share* (the share sheet's first row is Copy) and *Dismiss*.

A render error is caught earlier, by the root `ErrorBoundary` (`app/src/app/_layout.tsx` -> `lib/CrashScreen.tsx`):
a readable screen with the error, the stack, *Copy or share* and *Try again*, and the same record on disk.

What to ask the owner to read off the phone: the first line of the Last crash card (`Name: message`) and the
`at /path` line; *Copy or share* -> Messages/Mail gets the full stack.

### The crash that TestFlight builds 4-6 had

Six `*.native.tsx|ts` files imported `'./<their own basename>'` meaning the web twin. On the phone Metro resolves that
to the `.native` file itself: a require cycle, infinite recursion at module init, "missing the required default
export" for every route. In the dev client that is a red box at launch; in a release build it is the hard crash,
seen right after sign-in only because `AuthGate` kept the screens unmounted until then (web was fine: there the
import resolves to the `.tsx` twin). Fixed by splitting each pair into `<Name>Base` + a re-export shim;
`app/tests/selfImport.test.mjs` fails on any new self-import. Rule: a `.native` file never imports its own basename.

## 4. Reading an .ips crash log

On the phone: Settings > Privacy & Security > Analytics & Improvements > Analytics Data; the files are named
`Punchable-<date>.ips`. Share one (the share button top right). The first line is a JSON header; the rest is a JSON
body. The fields that matter:

| Field | What it tells you |
|---|---|
| `bundleID` | `com.tristanrega.punchable` - it is ours. |
| `bug_type` / `exception.type` / `exception.signal` | `309` + `EXC_CRASH (SIGABRT)` = an uncaught exception (ObjC or Swift) aborted the process. `EXC_BAD_ACCESS` = a memory fault in native code. |
| `termination.reason` / `exception.codes` | Watchdog kills say `0x8badf00d`; a Jetsam (memory) kill is a different file type (`JetsamEvent-`). |
| `asi` / `lastExceptionBacktrace` | For an uncaught NSException: the exception's name and reason. **A JS crash reads `RCTFatalException: Unhandled JS Exception: <message>`** - that name comes from `RCTAssert.m` (`RCTFatal`) and the message is the JS error, truncated to 175 characters; the JS stack is in the untruncated `userInfo`, which the .ips does not carry, so pair it with the in-app record (section 3). A Swift `fatalError` / precondition reads `Fatal error: ...` with the Swift file. |
| `faultingThread` + `threads[n].frames` | The thread that crashed. Frames show `imageIndex` + `imageOffset`; `usedImages[imageIndex].name` names the binary (`Punchable`, `hermes`, `ExpoModulesCore`, `RNScreens`, `UIKitCore`). A crash with `UIKitCore` / `RNScreens` on top of the faulting thread and no `RCTFatalException` is native UI (tabs, sheets, headers); one with `hermes` under `RCTFatalException` is JS. |
| `osVersion` / `modelCode` | iOS version and device (iPhone 17 = `iPhone18,x`). |

Symbolication: EAS keeps the dSYM for every build (the build page > *Download dSYMs*); without it our own frames show
as `Punchable + 0x...`. System frames (UIKitCore, Foundation) are already named.

## 5. Cutting a TestFlight build

```sh
cd app
npx eas-cli@latest build --profile production --platform ios --non-interactive --no-wait
#  -> prints the build id and URL; autoIncrement bumps the build number (appVersionSource: remote)
npx eas-cli@latest build:list --platform ios --limit 3            # status; wait for "finished"
npx eas-cli@latest submit --platform ios --id <build-id> --non-interactive   # to App Store Connect (ascAppId in eas.json)
```

TestFlight processing takes a few minutes after submit; the tester gets the update in the TestFlight app. Credentials
(distribution certificate, provisioning profile, App Store Connect API key) live on EAS, nothing local.

## 6. EAS profiles (`app/eas.json`) and env

| Profile | Builds | Env | Use |
|---|---|---|---|
| `production` | App Store / TestFlight, `autoIncrement` | `EXPO_PUBLIC_API_URL=https://punchable.io/api`, `EXPO_PUBLIC_AUTH=1` | The build testers get. |
| `preview` | internal distribution (ad hoc), release JS | same | A release-mode build to a registered device without TestFlight. |
| `dev-device` | internal, `developmentClient: true` | same | The dev client (section 2). |
| `development` | simulator, dev client | none (API at loopback, auth off) | Mac only; not used in this project. |

| Variable | Read by | Meaning |
|---|---|---|
| `EXPO_PUBLIC_API_URL` | `api.ts` | The API base; a trailing `/api` is dropped. Unset = `http://127.0.0.1:8796` (local dev, auth off). |
| `EXPO_PUBLIC_AUTH` | `auth.ts` | `1` forces sign-in on; `0` forces it off; unset = on unless the API is loopback. |
| `EXPO_PUBLIC_OIDC_ISSUER` | `auth.ts` | Override the realm URL (default `https://auth.punchable.io/realms/punchable`). |
| `EXPO_PUBLIC_OIDC_CLIENT_ID` / `_REDIRECT` / `_POST_LOGOUT` | `auth.ts` | Web only; the phone always uses `pk-mobile` / `punchable://...`. |
| `EXPO_PUBLIC_OIDC_GOOGLE` | `login.tsx` | Web only; `0` hides the Google button. |
| `EXPO_PUBLIC_GLASS_ACCESSORY` | `flags.ts` | `0` turns the Liquid Glass input-bar accessory off at build time. |
| `EAS_BUILD_NO_EXPO_GO_WARNING` | EAS | Quiet the Expo Go note in the build log. |

Remote switches without a build: the API's `/api/health` carries `flags` from the `PK_APP_FLAGS` env of the API
Deployment (a JSON object, e.g. `{"glass_accessory": false}`); the app reads it once per launch
(`app/src/lib/useFlags.ts`). A build-time `0` is final; the server can only turn a feature off, not back on.

## 7. Native APIs the app uses and their minimum iOS

| API | Package | Minimum iOS | Where | Notes |
|---|---|---|---|---|
| `ASWebAuthenticationSession` (custom scheme) | expo-web-browser | 12 (the app's deployment target is SDK 58's, 15.1) | sign-in, sign-out | The https-callback variant (17.4+) is not used. |
| Keychain (`SecItemAdd`, `kSecAttrAccessibleAfterFirstUnlock`) | expo-secure-store | any | the refresh token | No size limit enforced by Expo; native errors are caught. |
| `UITabBarController` native tabs | expo-router `NativeTabs` over react-native-screens 4.30 | 15.1; Liquid Glass look on 26 | the tab bar | `minimizeBehavior="onScrollDown"` is a 26-only feature, ignored below. |
| Tab bar bottom accessory (`UITabAccessory`) | expo-router `NativeTabs.BottomAccessory` over RNS | **26** (`API_AVAILABLE(ios(26.0))` in `RNSTabsBottomAccessoryComponentView.mm`) | the input bar on iOS 26 | Gated by `isLiquidGlassAvailable()` (expo-glass-effect) AND the remote flag. |
| `UIGlassEffect` | expo-glass-effect | 26 (Xcode 26 to compile) | `isLiquidGlassAvailable()` only; no `GlassView` is drawn | |
| Native stack headers, large titles, `UISearchController`, form sheets with detents, link previews / context menus | react-native-screens | 15.1 (detents 15+, `Link.Menu` previews 13+) | every screen | |
| SF Symbols | expo-symbols `SymbolView`, `NativeTabs.Trigger.Icon sf=` | 13 (names used exist by 15) | icons | An unknown name draws nothing (native `guard let image ... else { return }`), never throws. |
| `UIImpactFeedbackGenerator` etc. | expo-haptics | 10 | haptics | No-op without an engine. |
| `AVAudioRecorder` / `AVAudioSession` (playAndRecord, background) | expo-audio | 15.1 | hold-to-talk, recording | A recorder object is created at the first mount of a screen with the input bar (`useAudioRecorder`); `setAudioModeAsync` runs on the first record, not at mount. `UIBackgroundModes: audio` in app.json. |
| File system (documents, cache), background upload | expo-file-system (legacy API) | any | prefs, crash record, recordings, upload queue | |
| Photos library (videos) | expo-media-library (legacy API) | any | the Capture badge, Import | Limited access handled. |
| `Appearance.setColorScheme` | react-native | 13 | the per-device appearance override | |

New architecture (Fabric / bridgeless) is on, as SDK 58 defaults; `NativeTabs` requires it.
