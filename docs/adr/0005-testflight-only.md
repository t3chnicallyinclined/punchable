# ADR 0005: TestFlight-only distribution; email-only sign-in on the phone

Date: 2026-10-28. Status: accepted (owner). **Amended 2026-11-17** (below): the sign-in half is superseded; the
distribution half stands.

**Decision.** The iPhone build ships through TestFlight only. Sign-in on the phone is email through Keycloak; Google
sign-in is web only.

## Amendment 2026-11-17: social sign-in on the phone, invite-only testers first

The owner, word for word: "should we make the social log in too" and "yes lets open sign up and invite only testers
first".

- **Distribution is unchanged: TestFlight only.**
- **Sign-in on the phone and the web: email, Google and (from Slice 3) Apple**, all through the existing Keycloak realm
  `punchable`. Google is a button on the phone's login screen as well as the web's
  (`docs/design/SPEC-2026-11-17-social-sign-in-and-docs-import.md` Slice 1); it asks Google for its account chooser
  every time (`prompt=select_account`), so signing out really signs out.
- **Internal TestFlight only until Sign in with Apple is in the build.** App Store guideline 4.8 requires an
  equivalent login service (Sign in with Apple) once a third-party login authenticates the account, and the first
  external TestFlight build goes through Beta App Review (spec §3.1). No external group gets a build with Google and
  without Apple.
- **Sign-up opens invite-only**, and no one but the owner is invited until per-user data exists (spec §4.1, Slice 4):
  today every signed-in account is treated as him.
- Unchanged: connecting **Docs sync** stays a button on the website for him (T11 plan §8.7 Q6).
