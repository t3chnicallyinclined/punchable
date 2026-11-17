# SPEC 2026-11-17: social sign-in on the phone, invite-only testers, and importing Google Docs

Status: **spec only. Nothing is built, no config is changed, nothing is deployed.** Written by the lead on main
`a313f47`, branch `docs/social-signin-import`. Reviewed by four experts on 2026-11-16: the iOS dev expert (phone
auth flow, guideline 4.8, the Docs picker on iPhone), the identity and access expert (Keycloak, Apple and Google
brokering, linking), `gw-security` (takeover, scopes, the import's legal check, dedupe), and the cloud architect
(the live Keycloak, read-only; no change was made on the server).

Tags: **CONFIRMED** = read in the code at `file:line` on `a313f47`, read live on the server (read-only), or read on
an official page fetched on the date given; **INFERRED** = follows from what was read, not run; **UNKNOWN** = nobody
has checked. Dates of web pages are the day they were fetched (2026-11-16, the day before the owner's ask; the
experts ran that evening). No time estimates.

## 0. The answer in short

1. **Extend the existing `punchable` realm; do not make a new one.** It is live and the phone signs in through it
   today (CONFIRMED, §2). The owner's "create a realm for punchable" is already done.
2. **Google sign-in on the phone is a small, app-only change** (Slice 1): show the Google button the web already
   has, and ask Google to show its account chooser every time so signing out really signs out. It goes to
   **internal TestFlight only**.
3. **Sign in with Apple is required before any external tester** (App Store guideline 4.8, quoted in §3.1). In
   practice it arguably applies today, because Keycloak's page in the phone's sign-in sheet already shows Google.
4. **Apple needs a community provider jar on the shared Keycloak.** Keycloak has no built-in Apple provider.
   Adding the jar restarts the one Keycloak that serves every product's sign-in, which means a short outage for
   all of them. That needs the owner's go.
5. **Fix the realm before Apple or any new person** (Slice 2). The Google login auto-links by email with
   `trustEmail: true`. The live Keycloak (26.5.2) does not have the fix for the matching account-takeover CVE. The
   comments and the realm tests disagree with the config.
6. **Sign-up opens invite-only (owner's decision), but only after per-user data exists** (Slice 4). Today every
   signed-in account is treated as him: a tester would read and write his whole notebook, and the AI would draft
   in his voice. This is the largest piece of work in the spec, and it gates every other person.
7. **Profiles:** Keycloak holds identity only. SurrealDB holds everything else, keyed by Keycloak's `sub`, never by
   email (§5).
8. **Import from Google Docs is server-side Rust inside `pk-gsync`.** It uses the same fail-closed legal check, and
   the app never holds a Google token. For him it needs no new Google scope. For testers it is Google's mobile
   Picker with `drive.file`, a scope that needs no verification (§7).
9. **Duplicates:** an import is keyed by the Drive file id that gsync itself reads from Google, never by an id the
   phone sends. Today's lookup by doc id inside a free-text reason can be forged and must be replaced (§8).
10. **He can test Google sign-in and sign-out on his own account first**, with no second account (§9). A second
    test account is a real tester tenant, made with the invite tool once Slice 4 exists. Before that, it would be
    a second key to his data.

## 1. What the owner said (word for word)

| When | What | What it decides |
|---|---|---|
| 2026-11-17 | "should we make the social log in too, and an import feature so we can import google docs i can test with my account logging in and out" | This spec |
| 2026-11-17 | "we can use our keycloak create a realm for [Punchable] check with the [server's] architect" (the server's name is left out of this public repo) | The punchable realm exists on the shared Keycloak; extend it (§2, §3.2) |
| 2026-11-17 | "yes lets open sign up and invite only testers first" | Sign-up opens, invite-only first; the "keep closed" option is gone (§4) |
| 2026-11-17 | "how should we save the profile/user data keycloak or surreal ?" | §5 (the lead's answer) |

What this spec proposes to amend (only when its slices land):

- **ADR 0005**, "email-only sign-in on the phone; Google sign-in web only" (`docs/adr/0005-testflight-only.md:5-6`).
  It becomes: email, Google and Apple on the phone and the web. Distribution stays TestFlight-only.
- **The T11 plan §8.7 Q6**, "Connecting Google stays a button on the website, not in the phone app"
  (`docs/design/PLAN-2026-11-07-t11-retire-python.md:662-663`). Connecting **Docs sync** for him stays on the web.
  Importing a Doc becomes an action in the app, and it never needs a Google consent screen on the phone for him
  (§7.3).
- The other approved T11 answers stand: the server may read all of his Drive but change only Punchable-made docs,
  and Docs sync stays on his PC until the Python is retired plus two weeks (`PLAN…:646-649, 659-661`).

## 2. What exists today

| Fact | Evidence | Tag |
|---|---|---|
| The realm `punchable` is live on the shared Keycloak. Its issuer `https://auth.punchable.io/realms/punchable` answers discovery. | Cloud architect, public discovery read | CONFIRMED |
| It is applied from the repo file. ConfigMap `keycloak-realm-punchable` has field manager `punchable` and no last-applied annotation, as `infra/keycloak/apply.sh:39-40` makes it. The live content equals `realm-punchable.yaml` at HEAD except one comment line. The import Job has been garbage-collected (`keycloak-punchable.yaml:13`, 24 h TTL), so its last run cannot be read. | Cloud architect, live read | CONFIRMED |
| **Keycloak 26.5.2**, one replica, 2 CPU / 2 Gi Guaranteed. It is a custom image built in another product's repository (upstream 26.5.2 plus one other social provider jar). It starts without `--optimized`, so it rebuilds itself on every start (about 13 s) and picks up new provider jars by itself. | Cloud architect: server start log, image label, the Dockerfile | CONFIRMED (version, start mode); INFERRED (which Dockerfile built the live image) |
| **Five product realms share it**, plus `master`. A restart stops sign-in for all of them until the pod is back (about 21 s on 2026-10-28). Sign-ins on the other realms happen daily. | Cloud architect, live read | CONFIRMED |
| Requests on the node are at 98% of memory, and a second Keycloak pod needs 2 Gi. A rolling update's new pod would sit Pending. The rollout needs `Recreate` or a deleted old pod, which means a planned outage. | Cloud architect, live read | CONFIRMED (numbers); INFERRED (the hang) |
| Registration is off (`realm-punchable.yaml:16`), `duplicateEmailsAllowed: false` (`:19`), `verifyEmail: true` (`:20`), password reset allowed (`:21`). | file | CONFIRMED |
| The Google IdP has **`trustEmail: true`** (`:116`), `linkOnly: false` (`:119`), basic scopes only since 2026-10-26 (`:126-132`), `storeToken: true` (`:117`). | file | CONFIRMED |
| The first Google login runs `idp-detect-existing-broker-user` then `idp-auto-link` (`:141-156`). It **auto-links to the existing user with the same email** and never creates one. | file | CONFIRMED |
| **The comments disagree with the config.** `:104-107` say the link is made only from the Account Console and that `trustEmail` is false. `:106` says "his Google is not his Punchable email", while `:139-140` say his email is his Gmail. | file | CONFIRMED (the disagreement); UNKNOWN (which email is true; read the live user) |
| **The realm tests disagree with the config.** `infra/keycloak/test_realm.py` expects `refreshTokenMaxReuse` 0 (`:60`, the yaml has 1 at `:40`), offline + `prompt=consent` (`:70-71`), `trustEmail` false (`:73`) and detect-existing then password re-auth (`:81`). `test_realm_static.py:52` asserts auto-link. Either the live test is not run or it is red. | file | CONFIRMED |
| The script that makes the owner's account is missing. `first-login.sh` / `kc-first-login.sh` are named at `realm-punchable.yaml:7,15` and `test-realm.sh:45-49`, and neither file exists. | `ls infra/keycloak` | CONFIRMED |
| The phone has one "Sign in" button that opens Keycloak's page in the system auth sheet. That page shows Google (`hideOnLogin: false`, `:120`). The native Google shortcut is web-only (`app/src/app/login.tsx:24-27`). | file | CONFIRMED |
| pk-mobile is a public client with PKCE S256, redirect `punchable://auth/callback` and post-logout `punchable://auth/logout` (`realm-punchable.yaml:224-240`, `app/src/lib/auth.ts:30-50`). Sign-out is RP-initiated logout in an auth session (`auth.ts:217-253`). | file | CONFIRMED |
| **pk-api is single-tenant.** Any valid non-worker token is the owner (`rust/crates/pk-api/src/auth/mod.rs:4-6, 72-81`). The verifier never reads `sub` (`auth/verifier.rs:366-373`). Every app write's actor is `PK_OWNER_ID` (`auth/actor.rs:50`), pinned by the test `an_app_token_is_always_the_owner` (`actor.rs:94-98`). | file | CONFIRMED |
| The app has no `expo-updates`, so every JS change ships in a new EAS build. | `app/package.json` (iOS dev expert) | CONFIRMED |

## 3. Social sign-in on the phone

### 3.1 When Apple is required (guideline 4.8) and what TestFlight changes

App Review Guideline 4.8, "Login Services", verbatim (https://developer.apple.com/app-store/review/guidelines/,
fetched 2026-11-16 by the lead and by the iOS dev expert; the page shows no last-updated date):

> Apps that use a third-party or social login service (such as Facebook Login, Google Sign-In, Log in with X, Sign
> In with LinkedIn, Login with Amazon, or WeChat Login) to set up or authenticate the user's primary account with
> the app must also offer as an equivalent option another login service with the following features:
> * the login service limits data collection to the user's name and email address;
> * the login service allows users to keep their email address private as part of setting up their account; and
> * the login service does not collect interactions with your app for advertising purposes without consent.
>
> [...] Another login service is not required if:
> * Your app exclusively uses your company's own account setup and sign-in systems.
> * [marketplace, education/enterprise, government ID and third-party-client exceptions]

Guideline 2.2, same page: "Any app submitted for beta distribution via TestFlight should be intended for public
distribution and should comply with the App Review Guidelines."

| Question | Answer | Tag |
|---|---|---|
| When is Apple (or an equal) required? | As soon as Google authenticates the primary account. The "own sign-in only" exception stops applying once Google is offered. | CONFIRMED (text) |
| Does Keycloak's page showing Google inside the auth sheet already count? | Almost certainly yes. 4.8 covers any third-party login that authenticates the account; it does not depend on the button being native. ADR 0005's "email-only on the phone" is not true in practice today. | INFERRED |
| Internal TestFlight | Apple ties review to **external** testers: "If you invite external testers, your beta build may require review" (TestFlight overview, help/app-store-connect, fetched 2026-11-16). No page says internal builds are reviewed. **He can test Google on the phone internally first.** Internal testers are members of his App Store Connect team ("Add internal testers" page). | CONFIRMED (quotes); INFERRED (that internal builds skip review: Apple does not say so outright) |
| External TestFlight | "The first build you submit requires a full review" ("Invite external testers", fetched 2026-11-16). 4.8 applies, so Apple sign-in must be in before the first external tester. Review also asks for a sign-in it can use (§9.4). | CONFIRMED |

### 3.2 Which realm: the existing one

The owner asked to "use our keycloak, create a realm for punchable". **The realm exists and is the one the phone
uses (§2).** Keep it. A second realm would mean a second issuer, so every existing session and refresh token would
end. pk-api pins one exact issuer (`verifier.rs:366`), and gsync's Keycloak token would have to move too. Nothing
is gained: the realm already isolates Punchable's users, clients and identity providers from the other products'
realms. CONFIRMED (the facts); INFERRED (the cost of a second realm).

A **separate throwaway realm** is still useful **for tests** (§9.3): its tokens are refused by production pk-api,
because the issuer differs. CONFIRMED (`verifier.rs:366`).

### 3.3 Google on the phone

- **The flow.** It is the same authorization-code + PKCE request the phone uses today, with `kc_idp_hint: 'google'`,
  so Keycloak hands straight to Google. The request is already built (`login.tsx:25`) and is hidden on the phone
  only by `Platform.OS === 'web'` (`:27`). `kc_idp_hint` is handled by Keycloak's Identity Provider Redirector
  (Keycloak server admin guide, fetched 2026-11-16). CONFIRMED.
- **The sheet.** Google refuses sign-in from embedded web views (`disallowed_useragent`) and recommends the system
  browser and AppAuth (https://developers.google.com/identity/protocols/oauth2/native-app, fetched 2026-11-16).
  AppAuth-iOS uses ASWebAuthenticationSession (its README). That `expo-web-browser`'s auth session is
  ASWebAuthenticationSession on iOS is INFERRED; sign-in works in it today, Google included, through Keycloak's
  page.
- **Signing out really signs out.** The sheet shares Safari's cookies by default
  (`prefersEphemeralWebBrowserSession` is false by default, Apple's docs, fetched 2026-11-16; Expo SDK 58 exposes
  it as `preferEphemeralSession`). `signOut()` ends the Keycloak session but not Google's own cookie, so the next
  Google tap can go straight in without asking. INFERRED. **The lead's call:** pass `prompt: 'select_account'` on
  the Google request. Keycloak forwards the app's `prompt` to Google when the IdP config sets none (Keycloak 26.0.0
  `AbstractOAuth2IdentityProvider.java:346-351`; the realm sets none at `:122-134`). CONFIRMED for 26.0.0 source;
  INFERRED for 26.5.2. **Prove it** in a browser before the build: the authorize URL with
  `kc_idp_hint=google&prompt=select_account` shows Google's chooser. The ephemeral session is the heavier option:
  a password and 2-step every time. It is offered as a device check, not as the default.
- **Google Cloud.** The existing Web OAuth client is enough. Google's client is Keycloak, a web server, and the
  phone only ever talks to Keycloak. No iOS client is needed for sign-in. INFERRED from Google's rule "choose the
  client type that best matches the platform" (https://developers.google.com/identity/protocols/oauth2/policies,
  fetched 2026-11-16).
- **His account.** If his Google identity is already linked, the next Google login finds it by Google's subject id
  and skips the first-login flow, whatever the email (admin guide: the first login flow runs only on the first
  login from an IdP). CONFIRMED (doc); INFERRED (that a link exists: the Account Console or the live user shows
  it).

### 3.4 Apple through Keycloak

| Option | What it needs | Verdict |
|---|---|---|
| **Built-in Apple provider** | None exists. The built-in list at 26.8.2 is Bitbucket, Facebook, GitHub, GitLab, Google, Instagram, LinkedIn, Microsoft, OpenShift, PayPal, Stack Overflow and Twitter. The upstream attempts were closed (PR #7323, issues #12468, #15132). | Not available. CONFIRMED (keycloak.org server admin guide 26.8.2 and the GitHub API, 2026-11-16) |
| **Community jar `klausbetz/apple-identity-provider-keycloak`, web broker (recommended)** | Release **1.17.0** (2026-01-15) for **Keycloak 26.5.0 and later**, which covers the live 26.5.2. Config: client id = the Services ID, client secret = the raw `.p8` content, plus Team ID and Key ID. It signs Apple's client-secret JWT itself (ES256, 180 days, `AppleIdentityProvider.java:228,241,273`), so nothing needs rotating by hand. It sends `response_mode=form_post` (`:45`), and Apple POSTs to `/realms/punchable/broker/apple/endpoint`. It reads the secret through `session.vault()` (`:219`). | CONFIRMED (README and source at tag 1.17.0, 2026-11-16) |
| Generic OIDC provider | Apple's client secret is a JWT that expires within six months, so it needs a rotation job. Whether the generic broker sends `form_post` is UNKNOWN for 26.5. | Rejected |
| Native Apple button + token exchange | External-to-internal token exchange is only in legacy V1: "Implemented as a preview", "deprecated and not enabled by default" (https://www.keycloak.org/securing-apps/token-exchange, 2026-11-16). It needs a server-wide feature flag, so a second shared restart, on top of the same jar. The JWT Authorization Grant (supported since 26.6) needs a confidential client and an already-linked user, and issues no refresh token (https://www.keycloak.org/securing-apps/jwt-authorization-grant). | Rejected |

**How the jar gets on the server** (cloud architect, live read, no change made):

- **Path A, the recommended one:** add the jar (pinned by sha256) to the Keycloak image's Dockerfile, build a new
  tag, then `set image`. It is reversible by setting the image back. The Dockerfile lives in **another product's
  repository**, so the change is made with that lane, and the image is shared.
- Path B: an init container that copies the jar into `/opt/keycloak/providers/`, the theme-mount precedent
  (`infra/keycloak/apply-theme.sh:30-34`). **Trap:** mounting a whole volume over `/opt/keycloak/providers` hides
  the jar already baked into the image and breaks another product's sign-in. A later client-side apply from the
  other repository could also drop the mount (the Deployment carries that repository's last-applied annotation).
  Not recommended.
- **Either path restarts the shared Keycloak:** a short sign-in outage for every product's realm, which needs the
  owner's explicit go (the repo's trap, `docs/HANDOVER.md:159`). Because of the node's memory (§2) the rollout must
  be `Recreate` or a deleted old pod, at a quiet hour.
- **The CVE in §3.7 is fixed in 26.4.17, 26.6.8 and 26.8.2**, not in 26.5.x (the issue's release labels). If an
  upgrade is wanted, do it in the same restart as the jar, with the jar version matched to it (26.6.x: 1.17.0
  still applies per the README table). This is an owner question (§11 Q1).

**Secrets, by reference only** (cloud architect; the pattern of the Google IdP, `keycloak-punchable.yaml:28-46`,
`apply.sh:31`): a new Secret `punchable-apple-idp` with keys `APPLE_SERVICES_ID`, `APPLE_TEAM_ID`, `APPLE_KEY_ID`
and `APPLE_P8`, written by a new `ship.sh apple-idp` step that reads the values over ssh stdin without showing
them. It adds four `secretKeyRef` env entries to the import Job, a guard in `apply.sh` like line 31, and an
`identityProviders` entry `alias: apple, providerId: apple`. **Caveat:** keycloak-config-cli substitutes env values
into the YAML text before parsing, so a multi-line PEM inside a quoted scalar may be mangled. Prove it on the
throwaway Keycloak first (`infra/keycloak/test-realm.sh`), or load the key through Keycloak's vault (the jar
supports it). INFERRED. Who holds the `.p8`, what a leak costs and how it is rotated go to the key
expert before Slice 3.

**Routing:** the existing Ingress `punchable-auth` already sends `/realms/punchable/*` to Keycloak, and a POST to
`/realms/punchable/broker/apple/endpoint` reaches it (404 today, as for any unknown alias). No Ingress change is
needed. CONFIRMED (live probe).

**The phone side:** an Apple button on `login.tsx` with `kc_idp_hint: 'apple'`, the same request as the others.
JS only, no entitlement. Apple's button rules allow a custom button: only Apple's logo artwork, the titles "Sign
in / Sign up / Continue with Apple", and "no smaller than other sign-in buttons" (Sign in with Apple HIG, fetched
2026-11-16). CONFIRMED. **Whether App Review accepts Apple through a web sheet rather than the native button is
UNKNOWN.** Apple's text allows Sign in with Apple "in every version of your app or website across all platforms".
The fallback, if Review objects, is `expo-apple-authentication` plus a server route, a later slice.

### 3.5 What the owner creates in the consoles (exact checklists)

**Apple Developer account** (https://developer.apple.com/help/account/capabilities/configure-sign-in-with-apple-for-the-web/
and …/configure-private-email-relay-service/, fetched 2026-11-16):

1. Identifiers › the app's App ID (the bundle id in `app/app.json:17`): enable **Sign in with Apple**, as a primary
   App ID. (The web-sheet flow does not use the entitlement in the app. Enabling it now costs nothing and keeps the
   native fallback open.)
2. Identifiers › **+ › Services IDs**: make one, for example `io.punchable.signin`. Tick Sign in with Apple ›
   Configure › primary App ID = the app's.
3. In that configuration, **Domains:** `auth.punchable.io`. **Return URLs:**
   `https://auth.punchable.io/realms/punchable/broker/apple/endpoint` (the alias in the path must be `apple`). No
   verification file is needed.
4. Keys › **+**: a key with Sign in with Apple enabled, tied to the primary App ID. **Download the `.p8`** (Apple
   lets you download it once; INFERRED) and note its **Key ID**.
5. Membership: note the **Team ID**. (INFERRED location.)
6. Services › **Sign in with Apple for Email Communication**: register the domain Keycloak sends from (the realm's
   `PK_MAIL_FROM`), so mail to Apple relay addresses (invites, verification) is delivered. It must pass SPF, or
   DKIM `d=` must equal the From domain. Which one Cloudflare's sending satisfies is UNKNOWN: read it off one real
   message's headers.
7. Hand the four values (Services ID, Team ID, Key ID, the `.p8` file) to the lead's `ship.sh apple-idp` step on
   his own machine. Never paste them into a chat, a ticket or the repo.

**Google Cloud console** (the project that already holds the login client):

1. **Sign-in: no new client.** Check that the existing Web client lists the redirect
   `https://auth.punchable.io/realms/punchable/broker/google/endpoint` (INFERRED: it must, since web sign-in works).
2. **Publishing status: read it and tell the lead** (Google Auth Platform › Audience). In "Testing", sign-in is
   limited to 100 test users, and any scope beyond openid/email/profile expires after **7 days**
   (https://support.google.com/cloud/answer/15549945, fetched 2026-11-16). The T11 plan believes the project is "In
   production" (`PLAN…:383-385`, INFERRED there too). Invited testers who use Google must be test users while it is
   in Testing. Sign-in alone, with basic scopes, has no 7-day expiry and no warning screen.
3. **For the testers' Doc import (Slice 7 only):** a **separate Google Cloud project** for the Picker client with
   `drive.file`. That keeps his own project, which holds the restricted Drive scope for his sync, under Google's
   personal-use case ("only a few users, all of whom are known personally to you", restricted-scope verification
   page, fetched 2026-11-16) and away from public users. INFERRED (the lead's and `gw-security`'s call).
4. **Branding:** the consent screen's app name, logo, privacy policy (`https://punchable.io/privacy`) and terms.
   Testers will see it.

### 3.6 Account linking: his account, his Google link, Apple

| Case | What happens | Tag |
|---|---|---|
| He taps Google, and his Google identity is already linked | Found by Google's subject id; the first-login flow does not run; signed in | CONFIRMED (doc), INFERRED (that the link exists) |
| He taps Google, not linked, and his Google email equals his account email | Today: auto-linked with no check (`:147-156`). After Slice 2: Keycloak asks him to prove the account (email link or password) once, then links | CONFIRMED (today), INFERRED (after) |
| He taps Apple with **Hide My Email** | The relay address matches nobody: "federated identity unavailable", event `USER_NOT_FOUND` (source at 26.8.2) | INFERRED |
| **How he links Apple** | Signed in, open Account › Linked accounts on `https://auth.punchable.io/realms/punchable/account` › Apple › Link. Later Apple logins go by Apple's `sub`. An in-app link through Keycloak's application-initiated action (`kc_action`) is UNKNOWN for 26.5.2; check it in Slice 3. | INFERRED |
| An Apple email that changes, or relay forwarding turned off | No effect once linked by `sub`. **Never re-link by email.** | INFERRED |

### 3.7 What stops someone taking over an account by email

- **Today, with sign-up closed and Google the only IdP: not exploitable by a stranger if his account email is
  a Google consumer address.** Google is authoritative only for its own consumer addresses, or for verified Workspace addresses with `hd` set
  (https://developers.google.com/identity/gsi/web/guides/verify-google-id-token, fetched 2026-11-16). **If his
  account email is on another domain, anyone who ever controls that inbox** (its mail host, a Workspace admin, a
  lapsed domain) can make a Google account with it and be auto-linked to him. Which case is his is UNKNOWN (§2).
  INFERRED (`gw-security`).
- **Keycloak 26.5.2 auto-links without checking that the IdP verified the email.** The fix is
  "[CVE-2026-97175] Unverified IdP email auto-linking enables account takeover", keycloak/keycloak#54707, closed
  2026-11-05, release labels 26.4.17, 26.6.8 and 26.8.2 (fetched by the lead 2026-11-16). Even in 26.8.2 the check
  is skipped when `trustEmail` is true (source, identity expert). The admin guide itself says the auto-link
  authenticator "is dangerous … Do not use this authenticator unless you are carefully curating user
  registration." CONFIRMED.
- **When sign-up opens, auto-link becomes the classic pre-hijack.** An attacker registers his address first, he
  later signs in with Google, and the two are merged. Exploitable. INFERRED (`gw-security`).

**The fix, Slice 2 (before Apple and before any tester):**

1. `trustEmail: false` on google and apple. Google sends `email_verified` itself, so this costs nothing.
2. The first-login flow becomes: `idp-detect-existing-broker-user` (REQUIRED; never creates a user), then a
   REQUIRED sub-flow with two ALTERNATIVEs, `idp-email-verification` and `idp-username-password-form`. **Remove
   `idp-auto-link`.** Never add `idp-create-user-if-unique` while any auto-link exists.
   - *Where the experts differ:* `gw-security` wants password (plus OTP) only, saying email proof is only as strong
     as a password reset. The identity expert keeps email verification so an invited tester with no password can
     link Google. **The lead's call:** keep both. The realm already allows a password reset by email (`:21`), so the
     email link adds no weaker way in, and invitees need it. If the owner turns on OTP for himself, the password
     form asks for it. INFERRED.
3. Make `test_realm.py` and `test_realm_static.py` match the intended realm, each with a red arm (auto-link back
   in, or `trustEmail: true`, must fail). Fix the comments at `:104-107, 137-140`.
4. Restore the account script as a Rust tool (`pk-admin`, §4.2); the realm file keeps declaring no users.
5. Should-fix: a Grafana alert on `IDENTITY_PROVIDER_LINK_ACCOUNT` events in the punchable realm (a stolen session
   can add a permanent link from the Account Console; `ssoSessionIdleTimeout` is 14 days, `:42`).
6. Note: admins of the shared Keycloak can create users or impersonate him. They are keys to his data. That is
   true today and does not change here. INFERRED.

## 4. Who can create an account: invite-only first (decided)

The owner chose "open sign up and invite only testers first". The options left are **invite-only now** and **open
later**.

### 4.1 The hard gate: nobody else signs in until per-user data exists

Under today's pk-api a second account is the owner (§2): it reads his bits, sets, tapes and Docs text, writes as
him, gets AI drafts written in his voice from his profile (`PK_COMIC_NAME` in `infra/k8s/35-worker.yaml:81-82`;
`profile:voice` at `pk-api/src/routes/stores.rs:17`), and can file "delete my account", which records a request
against his notebook (`writes/me.rs:150-166`). **Slice 4 (§6) must be live, and its isolation test green, before
any invite is sent.** CONFIRMED (code).

### 4.2 Invite-only: what it needs

| Layer | What | Tag |
|---|---|---|
| Keycloak | `registrationAllowed` stays **false** during invite-only. A tester exists only because an invite created the user. | INFERRED (design) |
| The invite | A Rust tool `pk-admin invite <email>` (it also restores the missing owner script, §2). Through the Keycloak admin API with a service account of the **punchable realm only** (`manage-users`, nothing else), it creates the user with no password and the required actions VERIFY_EMAIL and UPDATE_PASSWORD, adds realm role **`pk-user`**, and sends `execute-actions-email`. The link lives 24 h (`actionTokenGeneratedByAdminLifespan`, `:44-45`). | INFERRED (design); the admin API calls are standard Keycloak |
| The invitee | Opens the email, sets a password, **or** taps Google / Apple with the same address. Slice 2's flow finds the user and asks once for the email link, then links. With **Apple Hide My Email** the relay will not match: the invite email says "use Share My Email, or sign in with email first and link Apple after". | INFERRED |
| pk-api | A registry in SurrealDB (§5, §6.2): row `account:<sub>` with `status` (`invited`, `active`, `disabled`), `tenant`, `invited_by`, `invited_at`. **A token whose `sub` has no active row gets 403 "not invited" on every route.** A token without `pk-user` (other than the owner's pinned `sub`) gets 403 too: two independent locks. | INFERRED (design) |
| The app | A "You're not on the list yet" screen for a 403 "not invited", with Sign out. Settings › Account for the owner: **Invite a tester** (web first), the list of testers with Disable. | INFERRED (design) |
| Keycloak Organizations invitations (26.x) | They exist, but the invitee goes "through the realm's self-registration flow" (admin guide 26.8.2). Whether that works while registration is off is UNKNOWN. Not needed. | UNKNOWN |

### 4.3 Open sign-up, later

`registrationAllowed: true`, the Slice 2 flow plus `idp-create-user-if-unique` (only once no auto-link exists),
pk-api making an `account` row and a tenant on the first request of a new `sub`, per-user AI spend caps, rate
limits, abuse controls, and Google's verification for anything beyond `drive.file`. Not in this spec's slices.

## 5. Where profile and user data live: Keycloak or SurrealDB

**The lead's answer, which this spec builds on:**

| Keycloak holds identity only | SurrealDB holds everything else |
|---|---|
| Sign-in name and verified email, password, OTP, linked Google and Apple identities, sessions, the realm role `pk-user` (the invite gate) | The account registry (`sub` → tenant, status, invited by and when), display name and stage name, settings (`PATCH /api/settings`), the voice profile and what it learned (`profile:voice`, `profile:voice.learned`), every bit, set, tape and Doc, the invites' state, a plan or limits later |

Why:

- **Key everything by Keycloak's `sub`, never by email.** Emails change, Apple's relay address is not the person,
  and the realm can rename a user. `sub` is stable for the life of the user in the realm. INFERRED (standard OIDC;
  Keycloak's `sub` is the user's id).
- SurrealDB is already the single source of truth and is backed up nightly. Keycloak's database is shared with
  other products and is backed up on their schedule (UNKNOWN). Data in Keycloak attributes would miss Punchable's
  backups, export and delete. INFERRED.
- Keycloak attributes go into tokens through mappers, so app data there leaks into every token and every log of
  one. INFERRED.
- **Nothing app-level goes in Keycloak attributes** beyond what the invite gate needs (the role).
- The app keeps showing name and email from the ID token (`app/src/lib/session.ts`, `profileFrom`). A display name
  he edits in Punchable lives in SurrealDB and wins on screen.
- **Drop the hard-coded `db: main` claim** from the `pk-api-audience` scope (`realm-punchable.yaml:180-183`), or
  never read it. pk-api must choose the database from the registry by `sub`, never from a claim. INFERRED (design).
- Device preferences (appearance, haptics) stay on the device (`lib/prefs`), as now.

## 6. Per-user data: the first server slice before any other person (Slice 4)

### 6.1 Every place that assumes one owner (grep on `a313f47`)

| # | Where | What assumes one owner | Tag |
|---|---|---|---|
| 1 | `pk-api/src/auth/mod.rs:72-81`, `auth/verifier.rs:366-373` | Any non-worker token passes; `sub` is never read | CONFIRMED |
| 2 | `pk-api/src/auth/actor.rs:50`; test `:94-98` | Every app write's actor is `PK_OWNER_ID` | CONFIRMED |
| 3 | `pk-api/src/config.rs:191`; `pk-store` default db | One database, `punchable/main` | CONFIRMED |
| 4 | `pk-api/src/state.rs:671-678`, `snapshot.rs:192-208, 493, 609, 722-767` | One in-memory snapshot of the whole database, with one owner id | CONFIRMED |
| 5 | `pk-api/src/index.rs:649-666`, `routes/kb.rs:15-55`, `pk-kb/src/view.rs:47-124`, `pk-kb/src/lib.rs:33-34`, `pk-store/src/store.rs:106-107` | Views and "by owner" built with one owner id | CONFIRMED |
| 6 | `pk-api/src/writes/` (`workingsets.rs`, `mysets.rs`, `mod.rs`, `review.rs`, `merge.rs`, `boards.rs`, `chats.rs`, `rant.rs`, `bits.rs`, `account.rs:51,104`, `venues.rs`, `uploads.rs`, `transcript.rs`, `say.rs`, `ingest.rs`, `ideas.rs`, `film.rs`, `worker.rs`) | Writes carry the one owner id | CONFIRMED (grep: 40 files name it across `pk-api`, `pk-kb`, `pk-creative`, `pk-worker`, `pk-gsync`, `pk-store`) |
| 7 | `pk-creative/src/reducers/mod.rs`, `workingset.rs`, `story.rs`, `corpus/mod.rs` | Reducers replay with one owner id | CONFIRMED |
| 8 | `pk-api/src/live.rs:67` | One broadcast channel: every change event goes to every socket | CONFIRMED |
| 9 | `pk-api/src/sweep.rs:7-9`, `db/schema_extra.surql:81-86` | One vector sweeper and one `lease:embedder` for the database | CONFIRMED |
| 10 | `pk-api/src/expiry.rs:76`, `writes/ask.rs:213`, `creative.rs:335-357` | One `jobs` store; the worker claims from it | CONFIRMED |
| 11 | `pk-worker/src/config.rs:85,195,203`, `runner.rs:309,336`, `kinds/mod.rs:293`; `infra/k8s/35-worker.yaml:81-82` | One owner id, **his name in every prompt** (`PK_COMIC_NAME`, `PK_COMIC_FIRST`) | CONFIRMED |
| 12 | `pk-api/src/routes/stores.rs:17`, `writes/account.rs:100` | One `profile:voice` (his voice profile and learned block go into every writing prompt) | CONFIRMED |
| 13 | `pk-llm/src/budget.rs:16,104`; the worker's one spend ledger | One daily AI cap (`PK_AI_DAILY_USD`) for everyone | CONFIRMED |
| 14 | `pk-api/src/recordings.rs:62-63` | R2 keys are a prefix plus a uuid, with no owner in the key | CONFIRMED |
| 15 | `infra/k8s/60-backup.yaml:52-53`, `pk-backup/src/cli.rs:122`, `dump.rs:70` | The nightly backup exports `punchable/main` only | CONFIRMED |
| 16 | `pk-api/src/writes/me.rs:150-166` | "Delete my account" records a request against the one notebook | CONFIRMED |
| 17 | `pk-api/src/lib.rs:617-620`, `config.rs:47` | One legal rules file with one owner address (his private court-document rules) | CONFIRMED |
| 18 | `pk-gsync` (`config.rs`, `sync.rs`) | One Drive, one owner address; it posts as the worker | CONFIRMED (T11 §8.1) |
| 19 | `realm-punchable.yaml:176-187` | Every token carries `db: main` | CONFIRMED |
| 20 | `docs/HANDOVER.md:14`; the Python API | The Python still answers forwarded routes (`DELETE /api/me`, the file-mode uploads) and is single-tenant | CONFIRMED |
| 21 | Caches: the cloud map's 5-minute cache, the search index, the snapshot generations | Keyed by nothing but the database | INFERRED (from 4, 9) |
| 22 | `db/schema.surql:18-322`, `schema_extra.surql:8-110` | **No table has an owner field**; the relations join any record to any record | CONFIRMED |

**Size, honestly: large.** It touches auth, the snapshot, every write path, the worker loop and prompts, the
sweeper, live events, recordings, backups, delete and the budget. It is a round of its own, like a T-round, and is
split into four sub-rounds below.

### 6.2 The design: one database per person, his data stays where it is

- **One SurrealDB database per tenant** in namespace `punchable`. **His tenant is `main`, unchanged: no data
  moves.** His Keycloak `sub` is pinned to it (`PK_OWNER_SUB`, which is also the T11 §8.4 must-fix 4 "owner means a
  pinned subject"). A tester's tenant is `u_<first 16 hex of sha256(sub)>`, created on invite by `pk-admin`: `DEFINE
  DATABASE … STRICT` first, then the schema (the repo's trap: select-before-define makes it lax for good).
- **The registry** lives in its own database `punchable/control`: `account:<sub>` as in §4.2.
- **Why a database each, not an owner field on every row:** no table has one (row 22). Every read goes through one
  snapshot of the whole database (row 4), so a per-row filter would have to be right in hundreds of places, and one
  miss leaks his material. With a database each, a request's store session is bound to one database, and a query
  cannot reach another's rows. Isolation comes from the structure, and the code above it can stay as it is. INFERRED
  (design). The cost: pk-api keeps one store connection and one snapshot per active tenant. The trap "never clone
  `pk_store::Store` per request" (a new session per query costs about 17 ms, `docs/HANDOVER.md` round 1) means a
  small map, tenant → (store, snapshot), with an LRU cap. Memory: pk-api's limit is 1536 Mi (`a346574`); a tester's
  snapshot is small next to his (INFERRED; measure it in 4b).
- **Considered:** SurrealDB record permissions with the JWT access method (the realm already sends `ns`, `db`, `ac`
  claims, `:176-187`). Rejected for now. pk-api connects as a database user, record permissions do not bind system
  users, and every query would have to run as the record user. INFERRED.
- **The worker:** a claim carries its tenant. pk-api hands jobs from each active tenant's `jobs` store, and every
  `/api/worker/*` write takes the tenant from the **claim it issued** (a lease row in `control`), never from the
  body. The worker loads the voice profile, the name and the verbatim check's notes **of that tenant**; his
  `PK_COMIC_NAME` applies to `main` only. Spend: a cap per tenant plus the global cap.
- **Live events, the sweeper, caches:** one channel, one lease and one cache key per tenant.
- **Recordings in R2:** key `<tenant>/<uuid>` under the same bucket (ADR 0015); the upload row lives in the
  tenant's database.
- **Backups:** the backup Job exports every tenant database it finds in `control`, one file each, plus `control`.
- **Delete my account:** for a tester, a request on their tenant only (then drop the database, its R2 prefix and
  the Keycloak user). For him, unchanged.
- **Legal rules:** his private rules stay on `main` and on his Drive only. Testers get the generic court-document
  net (owner question, §11 Q3). His private rules are never applied in a way a tester can observe. Every refusal
  is the same "can't import this doc".
- **The Python:** pk-api **never forwards a non-owner token to the Python** (403). No tester request reaches it,
  and Slice 4 does not wait for T11.

### 6.3 What must hold, and the test that proves it

**Invariant:** a token whose `sub` is not his reads none of his material, writes none of it, triggers no job on
his tenant, and sees none of his change events. A token with no active `account` row reads and writes nothing.

`pk-api/tests/tenants.rs`, over two scratch databases:

1. `main` holds a bit with the text `OWNER-SENTINEL-7f3a`; tester T is active with its own tenant.
2. With T's token: `GET /api/jokes`, `/api/jokes/<his id>`, `/api/search?q=OWNER-SENTINEL-7f3a`, `/api/today`,
   `/api/sets`, `/api/boards`, `/api/map/cloud`, `/api/profile/voice`, `/api/connections`, and every other GET in
   pk-api's own route table (the test walks the table, so a new route is covered by default). None of the responses
   contains the sentinel, and the bit-id route answers 404.
3. T writes a bit. It appears in T's tenant and not in `main`. A `/api/live` socket held by the owner sees no event
   from it.
4. A job T queues is claimed with tenant T, and its result posts only to T. The worker's prompt for it contains
   neither his name nor a line of his voice profile.
5. An unknown `sub`, and a `sub` with `status: disabled`, get 403 on every route.
6. **Red arms**, each of which must turn the test red: (a) the tenant resolver returns `main` for every `sub`; (b)
   the worker's write route takes the tenant from the body; (c) the live channel is shared.

### 6.4 Sub-rounds

| Round | What | Its own proof |
|---|---|---|
| 4a | The registry, `PK_OWNER_SUB`, `pk-user`, 403 for everyone else. No tenant yet: still only he can sign in, and this closes T11 must-fix 4. | Route-table test: another `sub` gets 403 everywhere; red arm: the pin removed |
| 4b | A database per tenant in pk-api: the tenant map, snapshots, caches, live, the sweeper, writes, no forwarding to the Python for non-owners | §6.3 items 1-3, 5, red arms (a), (c); the existing differentials unchanged on `main` |
| 4c | The worker by claim, the per-tenant voice, name and budget, R2 keys, backups, delete | §6.3 item 4, red arm (b); a backup of two tenants restored into scratch |
| 4d | `gw-security` reads 4a-4c as built; the owner's go; deploy order: pk-api, then pk-worker, then the backup Job | The review; one real tester invited only after |

## 7. Import from Google Docs

### 7.1 How a person picks a Doc on an iPhone

| Option | What happens | Scope | Tag |
|---|---|---|---|
| **A. Google's Picker for mobile apps** | The app opens Google's authorize URL with `trigger_onepick=true` in the system sheet. Google returns `picked_file_ids` and a `code` to the redirect. It accepts only `drive.file`, which "can't be combined with any other scope", and needs a client per platform. | `drive.file`: **non-sensitive**, no verification, no unverified-app screen | CONFIRMED (https://developers.google.com/workspace/drive/picker/guides/overview-desktop and the Drive scopes page, 2026-11-16); INFERRED that it runs inside ASWebAuthenticationSession (Google says "system browser") |
| B. The JavaScript Picker in a web view | "can no longer be displayed within an embedded webview"; Google sign-in is blocked in WKWebView | | CONFIRMED (Picker overview; native-app OAuth page) |
| C. The server lists his Docs through the Drive API, and the app shows that list | A list screen in the app | `drive.readonly` / `drive.metadata.readonly` are **restricted**: verification, and a CASA security assessment for a server that handles the data, renewed "at least every 12 months", unless the personal-use case applies | CONFIRMED (restricted-scope verification page, 2026-11-16) |
| D. The Files app (Drive's file provider) via `expo-document-picker` (already in the binary) | What arrives for a native Google Doc is UNKNOWN (an old report says a `.gdoc` shortcut, not text) | none | UNKNOWN; a one-tap device check settles it |

### 7.2 Where the import runs: Rust, on the server, inside `pk-gsync`

- **Owner rule: new code is Rust, nothing on his PC** (`CLAUDE.md:24-26`). The import is a `pk-gsync` operation
  ("import this doc now"), so there is **one** code path that opens a Doc. pk-api never exports a Doc itself.
  CONFIRMED (rule); INFERRED (design, `gw-security` must-fix 4).
- **The legal check is the same, and it fails closed.** `Rules::clear` runs on **fresh** metadata for the file id,
  then the export (which takes a `Clearance` value only `clear` makes; `rust/crates/pk-gsync/README.md:60`), then
  the content net before anything is hashed, stored or sent (`README:61`). The reason recorded is its kind only
  (`README:59`). **An explicit pick never overrides the rules:** a Doc in a legal folder, a Doc owned by someone
  else (the "not his doc" net, `README:56`), or a Doc whose text trips the content net is refused. The phone sees
  only "Can't import this doc". Nothing of a refused Doc (title, text, matched word, raw id) is logged, stored or
  returned; skips carry a hashed id (T11 §8.2 item 6). A content-net hit raises the alert (§8.2 item 7).
- **The phone never holds a Google token**, and pk-api holds none in plaintext (T11 §8.2, `PLAN…:399, 406`).
  `gw-security` must-fix 6.
- **How a request reaches gsync:** the phone sends `POST /api/gdocs/import {handle}` with its app token. pk-api
  checks the pinned `sub` (or, for a tester, its active account), records a queued import row in that tenant, and
  gsync takes it through its own role `pk-gsync-svc` (T11 §8.4 must-fixes 3-4), never the worker's. A `handle` is a
  short-lived value the server issued for one listed Doc, bound to the `sub`. **A raw file id from the phone is
  refused** (`gw-security` must-fix 4).
- **Until Docs sync leaves his PC** (Python retired plus two weeks, approved), the queued import row is taken by
  the PC's gsync on its next pass, which runs every minute. After the move, by the gsync pod. The app's wording
  says "Importing…" until the row is done, never "imported" before it is. INFERRED (design).

### 7.3 For him, and for testers

- **For him, no new scope and no consent screen.** His sync already reads his Drive (today on the PC with `drive`,
  after the move with `drive.readonly` plus `drive.file`). The app's import lists **only the Docs gsync has already
  cleared** (its own state), with "In Punchable" or "Not yet". Picking one asks gsync to take it now. So for him
  "import" mostly means **"pull this Doc now"** (a Doc that is still settling, or one he wants in before the next
  pass). Honest note: every comedy Doc he has is already imported automatically. The import is worth more to
  testers than to him (§11 Q5). INFERRED.
- **For testers: option A, the mobile Picker with `drive.file`,** in a separate Google Cloud project (§3.5). Google
  calls back to a punchable.io route, pk-api passes the `code` and `state` to gsync (the T11 §8.2 callback rules:
  `state` bound to the `sub` and a `__Host-` cookie, no codes in logs, a redirect to a fixed address), gsync
  redeems the code, holds the per-tenant token sealed, exports the picked Docs through the same legal path, then
  redirects to `punchable://import/done`, which closes the sheet. INFERRED (design). The tester's refresh token is
  sealed per tenant (T11 §8.2's sealing, with the tenant bound in as associated data).
- **"Testing" status** for the Picker project: up to 100 test users, and the `drive.file` grant expires after 7
  days (support.google.com/cloud/answer/15549945). For a one-off import a 7-day grant is enough. Moving that project
  "In production" with only `drive.file` needs no restricted-scope verification (the restricted list does not
  include `drive.file`). CONFIRMED (pages); INFERRED (that this is enough).

### 7.4 How an imported Doc becomes bits

It goes through gsync's existing `new_doc` (`rust/crates/pk-gsync/src/sync.rs:715-760`): one idea per Doc, the
Doc's text as the body, its title as the title, `from: {gdoc_id, title, modified}`, then a `docs[<fileId>]` entry
in `map.json`. Splitting a long Doc into several bits is the redesign's planned **Doc sorting** (Slice 8 "A Google
Doc as written", `docs/design/SPEC-2026-11-16-bits-sets-tapes.md` §3.7); the import hands its result to that when
it exists and invents no second splitter. For testers the same code runs against their tenant. CONFIRMED (code);
INFERRED (the hand-off).

## 8. No duplicate bits on his account

**Today's two keys** (CONFIRMED, code):

- `map.json` `docs[<fileId>]` (`sync.rs:746`), copied to the database as a `srcfile` row (T11 §8.1).
- Before making a bit, `joke_for_doc` (`sync.rs:695-713`) scans the decision log for an un-undone `add` whose
  **reason text** starts with the gdocs prefix and contains `(<fileId>,`.

**The problem** (`gw-security`, CONFIRMED by reading): the reason is free text that any app token can write
(`pk-api/src/writes/ideas.rs:73-82, 108`, and `:129` treats a gdocs-prefixed reason as gsync's). A Doc **title**
is inside that reason (`sync.rs:737-739`), so a Doc named `a' (FID_B, ` matches FID_B. With one token holder (him)
this is unvalidated and harmless. A phone import that passes a file id, or a second account, makes it exploitable:
someone could map a Doc onto an existing bit.

**The rule:**

1. The key is **the `id` Google returns from gsync's own fresh `files.get`**, never the phone's string.
2. Look it up in `map.json` `docs` (signed, T11 must-fix 1). If it is mapped and the bit exists, the import
   **returns the existing bit** ("Already in Punchable: <bit>") and writes nothing.
3. Replace the reason scan with a structured `from.gdoc_id` that **only gsync's role may write**. pk-api refuses
   the gdocs reason prefix and that field from any other caller.
4. One Doc, one bit, per tenant.
5. In-flight races: an import and the PC's pass on the same Doc take the same per-file lock as gsync's pass (one
   writer per state file, `README:103-104`). Whichever is second finds the map entry.

**Tests** (`pk-gsync/tests/import.rs`), each with a red arm:

1. Import a Doc already in `map.json`: zero new decisions, and the answer names the existing bit. *Red arm:* skip
   the map lookup, and a second bit appears.
2. Import a new Doc, then run a normal pass with that Doc changed: one bit, with its text updated. *Red arm:* the
   pass ignores the import's map entry, and two bits appear.
3. An app-token idea with a forged `[gdocs] … (FID_A, …` reason, then gsync meets FID_A: a **new** bit, nothing
   attached to the forged one. *Red arm:* today's `joke_for_doc`.
4. A Doc titled with `(FID_B,`: FID_B still gets its own bit.
5. A forged, expired or other-`sub` handle is refused. *Red arm:* accept a raw id.
6. A Doc in a legal folder, picked explicitly, is refused with kind `folder` and nothing exported. *Red arm:* let
   an explicit pick skip `clear`.

## 9. Testing on his own account, logging in and out

### 9.1 What could go wrong, and why it is safe

| Risk | Fact | Tag |
|---|---|---|
| His recordings | They survive a sign-out. `signOut` clears tokens only (`auth.ts:217-253`); the audio and its index stay in `Documents/recordings/` (`recordings.native.ts:52-53`); a 401 upload is kept and retried (`:219-221`). | CONFIRMED |
| **Being sent to sign-in during a recording** | `goLogin()` replaces the screen with no check for a running recording (`auth.ts:259-277`), and the recorder stops on unmount (`recorder.native.ts:52-60`). A take may land on disk but not in the list. **Never test sign-out while recording.** Should-fix: `goLogin` waits while the recorder is running. | CONFIRMED (code); INFERRED (the lost take) |
| A Google address not tied to his account | Refused with "federated identity unavailable"; nothing is created. | CONFIRMED (`:141-156`) |
| Signing out on punchable.io in Safari | It may sign the phone out too: the sheet and Safari may share one Keycloak session. | INFERRED |
| **The gsync token** | Signing in and out on the phone does not touch it. gsync holds its own Keycloak offline token and its own Google token on the PC (`pk-gsync/README.md:23`). **Never run `pk-gsync` with a copied home folder:** the Keycloak token rotates on every use, so a copy is one rotation behind and breaks the real task (`README:130-132`). Do not press web Reconnect Google as part of these tests: today it renews nothing for Drive (T11 §8.1). | CONFIRMED |
| His data | Sign-in tests write nothing. Import tests write one bit per new Doc: use a throwaway Doc and trash it after (as the gsync dry run does, `README:136-173`). | CONFIRMED |

### 9.2 The order he tests in (Slice 1's checks)

1. Let every memo finish sending (Bits › Memos shows none waiting).
2. Settings › Sign out. The login screen shows **Sign in with Google** and **Sign in with email**.
3. Sign in with Google. Google's account chooser appears; pick his. He lands where he was, signed in.
4. Sign out again, then tap Google: **the chooser asks again** and does not go straight in.
5. Quit the app from the app switcher and reopen it: still signed in.
6. Sign in with email once (the old way still works).
7. Settings › Device checks › "Private sign-in": with this on, Google asks for the password. Tell us whether to keep
   it off.

### 9.3 A second test account

- **Before Slice 4: none.** Any second user would be a second full key to his data (§4.1). He tests on his own
  account only. CONFIRMED (code).
- **After Slice 4:** he invites a **second email of his own** with `pk-admin invite`. It becomes a tester tenant
  with nothing of his in it. That is the account to test "a new person signs in with Google / Apple / email", the
  invite email and import-as-a-tester, and **the demo login App Review asks for** (Beta App Review needs one; it must
  never be his). INFERRED (design).
- **For realm changes:** a throwaway realm on the throwaway Keycloak (`infra/keycloak/test-realm.sh`) proves the
  Slice 2 flow and the Apple jar before production. Its tokens are refused by production (`verifier.rs:366`).
  Note: `test-realm.sh` runs plain Docker and must never run on the production server (the repo's trap); run it on
  the build box. CONFIRMED (script, traps).

### 9.4 External testers and App Review

External TestFlight needs Beta App Review (§3.1): Apple sign-in in the build, and the demo tenant from §9.3. Until
Slices 3, 4 and 5 are live, testers stay off TestFlight. INFERRED.

## 10. Slices (smallest first; each its own TestFlight build where it has an app part, usable alone)

Every slice: `cd app && npx tsc --noEmit && npm run test:unit && npx expo-doctor`; a rules file in `src/rules/` with
a test and a `--red` arm for every new rule; Rust slices add `cd rust && PK_REQUIRE_EXTERNAL=1 cargo xtask gate` and
a scratch-database test with a red arm; realm slices run `infra/keycloak/test_realm_static.py` and the throwaway
realm test; the guard before each commit; the iOS dev and design experts review every app diff, and `gw-security`
every auth diff. Server before app. Every production step (realm re-import, Keycloak restart, rollout) is the
owner's go and gets a private deploy-log entry with a rollback line. No time estimates.

### Slice 1: Google sign-in and a real sign-out on the phone (internal TestFlight)

**After it he can:** sign in with Google on the phone, sign out, and be asked again.

- **Server:** none. First, in a browser: the authorize URL with `kc_idp_hint=google&prompt=select_account` shows
  Google's chooser (§3.3). If it does not, the slice adds `prompt` to the IdP config, a realm re-import (no
  Keycloak restart).
- **App:** `login.tsx` shows the Google button on every platform (drop the `Platform.OS === 'web'` gate at `:27`),
  `prompt: 'select_account'` on the Google request, the footer "There's no sign-up" (`:178`) becomes "Invite only
  for now", and the comment at `:3-7` matches. `settings.tsx:122-124`: fix the stale "offline + consent" comment.
  Device check "Private sign-in" (`preferEphemeralSession: true` on the Google prompt, off by default).
  `rules/signinRules.ts` (which buttons, which params per platform).
- **Tests:** `tests/signinRules.test.mjs` (the phone gets Google with `select_account`; email has no hint; *red
  arm:* the web-only gate back in fails it); the existing `useCodeExchange` and `tokenMachine` suites unchanged.
- **Owner's phone checks:** §9.2, steps 1-7.
- **Distribution:** internal TestFlight only, never an external group (4.8, §3.1).

### Slice 2: The realm, safe before anyone else (no phone build)

**After it:** nobody can be linked to his account by email alone; the realm tests tell the truth.

- **Realm:** `trustEmail: false`; the first-login flow without auto-link (§3.7); comments fixed; `pk-admin`
  (Rust) restores the owner-account tool and is ready for invites (`invite` stays unused until Slice 5).
- **Tests:** `test_realm_static.py` asserts no `idp-auto-link`, `trustEmail` false, and the two alternatives (*red
  arms:* each one undone); `test_realm.py` updated to the real config and run against the throwaway realm.
- **Production:** one realm re-import (`apply.sh`, the punchable Job only; no Keycloak restart). The owner's go.
- **Owner's checks:** sign out, sign in with Google: still straight in (already linked). If Keycloak asks to
  confirm the account once, use the email link. Tell us which happened.

### Slice 3: Sign in with Apple

**After it he can:** sign in with Apple. The build may go to external testers once Slices 4 and 5 are also live.

- **Owner first:** the Apple checklist (§3.5).
- **Server:** the jar in the shared Keycloak image (Path A, with the other lane), and a Keycloak upgrade in the same
  restart if §11 Q1 is yes. The `punchable-apple-idp` Secret via `ship.sh apple-idp`, the IdP in the realm, the
  `apply.sh` guard. Proven first on the throwaway Keycloak (jar, PEM substitution, form_post callback). Then **one
  planned restart of the shared Keycloak** (Recreate, at a quiet hour; all products' sign-in pauses briefly), then
  the realm re-import. Each step is the owner's go.
- **App:** an Apple button (Apple's artwork and title, the same size as Google's) with `kc_idp_hint: 'apple'`.
  Check: `kc_action` to link Apple from inside the app (UNKNOWN on 26.5.2; otherwise a row "Link Apple" that opens
  the Account Console).
- **Tests:** `signinRules` gains Apple (*red arm:* Apple smaller than Google, or missing on the phone); the realm
  static test pins the Apple IdP with `trustEmail` false and the vault/env reference, never a value.
- **Owner's phone checks:** sign out; Sign in with Apple with **Share My Email** → in; sign out; Apple with **Hide
  My Email** → "not linked" page; signed in, Link Apple; sign out; Apple → in.

### Slice 4: Per-user data, the gate for every other person (server; small phone build)

**After it:** a second account sees an empty Punchable of its own, or a "not on the list yet" screen.

- **Server:** §6.4 rounds 4a-4d (registry, pinned owner `sub`, database per tenant, worker by claim, per-tenant
  voice/name/budget, R2 keys, backups, delete, no forwarding to the Python for non-owners).
- **App:** the "You're not on the list yet" screen for a 403 "not invited", with Sign out (`rules/accessRules.ts`
  maps the API's 403 kind to the screen).
- **Tests:** §6.3 with its three red arms; the T10 differentials unchanged on `main`.
- **Owner's checks:** none on the phone beyond "everything still works". The proof is the test, and the review by
  `gw-security` before the deploy.

### Slice 5: Invite-only testers

**After it he can:** invite a tester from Settings on the web; the tester signs in with email, Google or Apple and
gets their own empty Punchable.

- **Server:** `pk-admin invite` behind `POST /api/admin/invites` (owner `sub` only), the `account` rows, Disable.
  The punchable-realm service account with `manage-users` only.
- **App:** Settings › Account › Invite a tester, the tester list with Disable; a tester's first-run screen.
- **Tests:** an invite creates exactly one Keycloak user with `pk-user` and one `invited` row; a disabled tester
  gets 403 (*red arm:* the status check removed); a tester cannot call the invite route.
- **Owner's checks:** invite his second email (§9.3); on a second phone or after signing out, accept with Google:
  an empty Bits list, Settings shows that email; sign back in as himself: everything of his is there and the
  tester is not in any list.

### Slice 6: Import a Google Doc, for him

**After it he can:** in Bits › + › Import from Google Docs, see his Docs that Punchable can read, each marked "In
Punchable" or "Not yet", and pull one in now.

- **Server (Rust):** `pk-gsync` gains `import` (the queued row, handles, the shared legal path, §8's dedupe and
  structured `from.gdoc_id`); pk-api `GET /api/gdocs` (the cleared list from gsync's state, owner only) and `POST
  /api/gdocs/import {handle}`; pk-api refuses the gdocs reason prefix from app tokens. It works with the PC's
  gsync now and the pod later.
- **App:** the import sheet (search, the two states, "Importing…" until done, "Already in Punchable" with Open).
- **Tests:** §8's six tests; the legal tests of `pk-gsync/tests/legal.rs` still green; a route test that a tester
  token gets 403 on both routes.
- **Owner's checks:** make a throwaway Doc; within two minutes it shows "Not yet"; Import → "Importing…" → a new bit;
  Import it again → "Already in Punchable"; edit the Doc → the same bit updates; trash the Doc. A Doc in a legal
  folder never appears in the list.

### Slice 7: Import a Google Doc, for testers

**After it:** a tester picks Docs with Google's own picker and they become bits in their own Punchable.

- **Owner first:** a separate Google Cloud project with a `drive.file` Picker client (§3.5 item 3).
- **Server:** the Picker callback route, the per-tenant sealed token, generic legal rules for testers (§11 Q3).
- **App:** "Choose from Google Drive" (the mobile Picker in the system sheet), back via `punchable://import/done`.
  Device check first: what the Files app hands over for a Doc (§7.1 D).
- **Tests:** the T11 callback tests (wrong, reused or missing `state`, another account, an extra scope), plus "a
  tester's import lands only in their tenant".
- **Owner's checks:** as his second email, pick a Doc → a bit; as himself, nothing new.

## 11. Questions for the owner (each re-explains itself; the lead's recommendation first)

1. **Upgrade the shared sign-in server while we add Apple?** Apple sign-in needs an add-on installed on the one
   Keycloak that signs in all your products, which means a short restart. The version running has a known
   account-takeover bug (fixed in later releases). Slice 2 avoids the risky setting anyway, so we are safe without
   the upgrade. *Recommended: yes, upgrade in the same restart* (one outage instead of two), done with the other
   product's lane, because its image is shared.
2. **When may we restart the shared sign-in server?** It pauses sign-in on every product briefly. *Recommended: a
   quiet hour you name, after the add-on is proven on a throwaway copy.*
3. **Which legal rules for testers' Docs?** Your private court-document rules are about your documents. *Recommended:
   testers get the generic court-document check only; your private rules stay on your account and are never
   shown.*
4. **Who may invite, and how much AI may each tester use?** Testers' drafts run on your Anthropic key. *Recommended:
   only you invite, from Settings on the website; each tester gets a small daily AI cap you set, inside your
   overall cap.*
5. **Do you want "Import from Google Docs" for yourself?** Your Docs already come in automatically every minute, so
   for you it is "pull this Doc in now". *Recommended: build it (Slice 6), because it proves the import path on
   your own Docs before any tester uses it. If you would rather not, skip to testers (Slice 7) after Slice 5.*
6. **Apple through the web sign-in sheet, not Apple's native button?** It needs no new app capability. Whether App
   Review accepts it is not certain. *Recommended: web sheet first; switch to the native button only if Review
   asks.*
7. **Retire "email-only sign-in on the phone" (ADR 0005)?** *Recommended: yes. The phone offers email, Google and
   Apple, distribution stays TestFlight-only, and Docs sync stays a website button for you.*
8. **One check only you can make:** in the Google Cloud console, is the sign-in project "In production" or
   "Testing" (Google Auth Platform › Audience)? In Testing, invited Google users must be listed by hand, and
   Drive access expires every 7 days.
9. **Your Punchable email: is it your Gmail?** The realm's notes say both yes and no. It decides how exposed the
   current Google auto-link is until Slice 2 lands. *Recommended: tell us, or let the lead read it from the account
   list (read-only).*

## 12. What is not known (UNKNOWN), and how each gets settled

| Unknown | Settled by |
|---|---|
| Whether App Review accepts Sign in with Apple through a web sheet | The first external TestFlight review (fallback: the native button) |
| Whether `prompt=select_account` reaches Google through Keycloak 26.5.2 | One browser check before Slice 1 |
| Whether his account email is his Gmail; whether his Google link exists | A read of the live user (owner's go, read-only) |
| The Google project's publishing status | Owner, Q8 |
| The PEM through keycloak-config-cli's substitution | The throwaway Keycloak in Slice 3 |
| In-app Apple linking (`kc_action`) on 26.5.2 | Slice 3 check |
| Cloudflare sending vs Apple relay (SPF or DKIM path) | One real message's headers |
| What the Files app returns for a Google Doc | Device check in Slice 7 |
| Whether the mobile Picker runs inside ASWebAuthenticationSession | Slice 7's first build |
| Keycloak Organizations invitations with registration off | Not needed; open |
| pk-api memory with several tenant snapshots | Measured in 4b |
| Whether Keycloak's database backups include Punchable's users (shared schedule) | Cloud architect, before Slice 5 |
| Whether the live `test_realm.py` is run or red | Slice 2 runs it |
