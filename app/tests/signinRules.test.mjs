// Unit test for rules/signinRules.ts (SPEC 2026-11-17 Slice 1): the phone gets the Google button with Google's account
// chooser, the email way sends no hint, and signing out (or the trip to sign-in when the session ends) waits while a
// tape runs and while he is still on a take screen.
//   node --experimental-strip-types --no-warnings tests/signinRules.test.mjs          the suite
//   node --experimental-strip-types --no-warnings tests/signinRules.test.mjs --red    the red arms: the old web-only
//                                                                                     Google gate, a Google request
//                                                                                     without prompt=select_account, and
//                                                                                     a sign-out that ignores a running
//                                                                                     tape, and a trip to sign-in that
//                                                                                     looks only at the tape (not the
//                                                                                     take screen) must each FAIL
import * as R from "../src/rules/signinRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/** Google is offered on the phone (iOS and Android) and the web, first, unless the build turns it off. */
function phoneGetsGoogle(ways) {
  return ["ios", "android", "web"].every((p) => eq(ways(p, false), ["google", "email"])) && eq(ways("ios", true), ["email"]);
}
/** The Google request carries the hint AND the chooser; the email request carries neither. */
function googleAsksChooser(params) {
  const g = params("google") || {};
  const e = params("email");
  return g.kc_idp_hint === "google" && g.prompt === "select_account" && Object.keys(g).length === 2 && (e === undefined || Object.keys(e).length === 0);
}
/** A running tape refuses sign-out with a reason; idle lets it go. */
function tapeBlocksSignOut(gate) {
  const busy = gate({ recording: true });
  const idle = gate({ recording: false });
  return busy.ok === false && typeof busy.reason === "string" && busy.reason.length > 0 && idle.ok === true;
}
/** The trip to sign-in when a session ends: held while a tape runs anywhere, and on a take screen (the tape just
 *  stopped, the screen is saving it) after a session that ended under him; never held for a signed-out visitor who
 *  opened a take screen by link, nor anywhere else once the tape is off. */
function takeHoldsLogin(holds) {
  const s = (recording, path, hadSession) => holds({ recording, path, hadSession });
  return s(true, "/", true) === true && s(true, "/joke/j1", false) === true
    && s(false, "/record", true) === true && s(false, "/stage/set:1", true) === true && s(false, "/film/joke:2", true) === true && s(false, "/spike-run", true) === true
    && s(false, "/record", false) === false
    && s(false, "/", true) === false && s(false, "/recordings", true) === false && s(false, "/joke/j1", true) === false;
}

if (process.argv.includes("--red")) {
  // the gate this slice removes (login.tsx before 2026-11-17: `Platform.OS === 'web' && ...`)
  const webOnly = (p, off) => (p === "web" && !off ? ["google", "email"] : ["email"]);
  // the Google request as it was (kc_idp_hint only: a sign-out did not make Google ask again)
  const noChooser = (w) => (w === "google" ? { kc_idp_hint: "google" } : undefined);
  // a sign-out that never looks at the recorder (auth.signOut before 2026-11-17)
  const blind = () => ({ ok: true });
  // the root layout's AuthGate before 2026-11-17: never holds
  const neverHolds = () => false;
  // the first cut of this slice: held only while the tape runs, so the take screen's save and Back raced the login
  const tapeOnly = (h) => h.recording;
  // a hold that ignores who was signed in: a signed-out visitor opening /record by link would be let in
  const anyone = (h) => h.recording || R.isTakePath(h.path);
  const caught = [
    ["the web-only Google gate", !phoneGetsGoogle(webOnly)],
    ["a Google request without prompt=select_account", !googleAsksChooser(noChooser)],
    ["a sign-out that ignores a running tape", !tapeBlocksSignOut(blind)],
    ["a session end that sends him to sign-in mid-take", !takeHoldsLogin(neverHolds)],
    ["a hold that lets go when the tape stops, before the take screen has saved and left", !takeHoldsLogin(tapeOnly)],
    ["a hold that keeps a signed-out visitor on a take screen", !takeHoldsLogin(anyone)],
  ];
  for (const [what, ok] of caught) console.log(`${ok ? "PASS" : "FAIL"} red arm: ${what} is caught`);
  const real = phoneGetsGoogle(R.signInWays) && googleAsksChooser(R.paramsFor) && tapeBlocksSignOut(R.signOutGate) && takeHoldsLogin(R.loginHolds);
  console.log(`${real ? "PASS" : "FAIL"} the real rules pass the same checks`);
  process.exit(caught.every(([, ok]) => ok) && real ? 0 : 1);
}

// ---- the buttons
check(phoneGetsGoogle(R.signInWays), "Google and email on iOS, Android and the web; EXPO_PUBLIC_OIDC_GOOGLE=0 leaves email only");
check(R.emailLabel(["google", "email"]) === "Sign in with email" && R.emailLabel(["email"]) === "Sign in", "the email button says 'Sign in' only when it is the only way");
check(R.FOOTER.includes("Invite only for now") && !/no sign-up/i.test(R.FOOTER), "the footer says invite only, not 'There's no sign-up'");

// ---- the parameters
check(googleAsksChooser(R.paramsFor), "Google: kc_idp_hint=google + prompt=select_account; email: none");
const a = R.paramsFor("google"); a.prompt = "none";
check(R.paramsFor("google").prompt === "select_account" && R.GOOGLE_PARAMS.prompt === "select_account", "a caller changing its copy cannot change the next request");
check(eq(R.promptOptions("google", false), {}) && eq(R.promptOptions("google", true), { preferEphemeralSession: true }) && eq(R.promptOptions("email", true), {}), "Private sign-in: an ephemeral sheet for Google only, and only when switched on");

// ---- leaving
check(tapeBlocksSignOut(R.signOutGate), "a running tape blocks sign-out, with a reason; idle does not");
check(takeHoldsLogin(R.loginHolds), "a session ending mid-take holds the trip to sign-in while the tape runs and while he is on the take screen; a signed-out visitor is not held");
check(R.isTakePath("/record") && R.isTakePath("/stage/x") && !R.isTakePath("/recordings") && !R.isTakePath("/stagecraft"), "take screens are matched by whole path segments");
check(/stop the take/i.test(R.signOutGate({ recording: true }).reason), "the reason says what to do");

if (fails.length) { console.log(`\n${fails.length} FAILED`); process.exit(1); }
console.log("\nall passed");
