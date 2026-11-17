// Sign-in and sign-out rules (docs/design/SPEC-2026-11-17-social-sign-in-and-docs-import.md Slice 1; ADR 0005 as
// amended 2026-11-17): which buttons the login screen shows, which parameters each one sends to Keycloak, and when
// signing out must wait. Pure: no React, no network; tests/signinRules.test.mjs runs it under node (with --red arms).
//
//   Buttons: Google and email on EVERY platform (the phone too, since 2026-11-17). EXPO_PUBLIC_OIDC_GOOGLE=0 hides
//            Google (a build whose realm has no Google IdP).
//   Google:  kc_idp_hint=google (Keycloak hands straight to Google) and prompt=select_account, so Google shows its
//            account chooser whenever the request reaches Google. Keycloak 26.5.2 forwards the app's prompt to Google
//            (read on auth.punchable.io 2026-11-16: the broker's Google URL carries prompt=select_account;
//            AbstractOAuth2IdentityProvider) and acts on none but `login` itself, so a live Keycloak session still
//            signs straight in: the chooser shows after a sign-out only because signOut() ends the Keycloak session
//            (end_session with id_token_hint, then the refresh token revoked, which ends it too: lib/auth).
//   Email:   no hint, no prompt: Keycloak's own page.
//   Leaving: never while a tape runs. Signing out, or being sent to sign-in when the session ends (the root
//            layout's AuthGate, lib/auth goLogin), replaces the screens, and the recorder stops when its screen goes
//            (lib/recorder.native.ts), so a take could land on disk but not in the list. So sign-out is refused while
//            a tape runs, and the trip to sign-in holds while a tape runs AND while he is still on a take screen
//            (the screen saves the take after the tape stops, then leaves; the trip goes once he has left). Words
//            typed into a bit are written to the phone before any save (lib/BitDoc keepLocal) and sent after he signs
//            back in (the root layout's BitDrafts), so they do not block a sign-out.

export type Way = 'google' | 'email';

/** What the Google button adds to the authorization request. */
export const GOOGLE_PARAMS: Readonly<Record<string, string>> = Object.freeze({ kc_idp_hint: 'google', prompt: 'select_account' });

/** The buttons, in order, for this platform. `googleOff` = the build said EXPO_PUBLIC_OIDC_GOOGLE=0. */
export function signInWays(_platform: string, googleOff: boolean): Way[] {
  return googleOff ? ['email'] : ['google', 'email'];
}

/** The extra authorization parameters for a button (undefined = none). */
export function paramsFor(way: Way): Record<string, string> | undefined {
  return way === 'google' ? { ...GOOGLE_PARAMS } : undefined;
}

/** The options for the auth sheet. "Private sign-in" (a device check, off by default) asks iOS for an ephemeral
 *  session: no cookies shared with Safari, so Google asks for the password every time. Google only: the email way
 *  keeps the shared session, so Keycloak's "remember me" still works there. */
export function promptOptions(way: Way, privateSignIn: boolean): { preferEphemeralSession?: true } {
  return way === 'google' && privateSignIn ? { preferEphemeralSession: true } : {};
}

/** The email button's words: "Sign in" when it is the only way, "Sign in with email" beside Google. */
export const emailLabel = (ways: Way[]): string => (ways.includes('google') ? 'Sign in with email' : 'Sign in');

/** The line under the buttons. */
export const FOOTER = 'Punchable is one comic’s notebook. Invite only for now.';

/** What the phone is doing when he taps Sign out. */
export type Leaving = { recording: boolean };
export type SignOutGate = { ok: true } | { ok: false; reason: string };

/** Sign out now, or not yet and why (the reason is shown under the greyed-out button). */
export function signOutGate(s: Leaving): SignOutGate {
  if (s.recording) return { ok: false, reason: 'Recording now. Stop the take first: signing out would end it.' };
  return { ok: true };
}

/** The screens that record (a tape, a set on stage, a film, the walk-through check): app/record, stage/[id],
 *  film/[id], spike-run. */
export const TAKE_PATHS: readonly string[] = Object.freeze(['/record', '/stage', '/film', '/spike-run']);
export const isTakePath = (p: string): boolean => TAKE_PATHS.some((x) => p === x || p.startsWith(x + '/'));

/** Where he is when the session ends. `hadSession` = he was signed in earlier in this launch (a session that ended
 *  under him, not a signed-out visitor opening a take screen by link). */
export type Holding = Leaving & { path: string; hadSession: boolean };

/** The trip to sign-in (and the "You're signed out" cover over the page) waits: while a tape runs anywhere, and while
 *  he is still on a take screen after a session ended under him, so its own save and Back finish first. */
export function loginHolds(s: Holding): boolean {
  return s.recording || (s.hadSession && isTakePath(s.path));
}
