// Unit test for rules/prompterRules.ts (the Film it band's pacing, SPEC 2026-11-03 §2.3 D3, §10 "v1 pacing").
//   node --experimental-strip-types --no-warnings tests/prompterRules.test.mjs          the suite
//   node --experimental-strip-types --no-warnings tests/prompterRules.test.mjs --red    the red arm: the pacing checks run
//                                                                                     against a lenient prompter must FAIL
import * as P from "../src/rules/prompterRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const LINES = ["Self-checkout is the store hiring you for free, and you said yes.", "I scanned forty things.", "Nobody at my job claps when I scan.", "Unexpected item in the bagging area: my dignity."];
const twelve = "one two three four five six seven eight nine ten eleven twelve";

/** The rules under test (SPEC §10): a 12-word line at 2.4 w/s arms for 5.0 s; a short line never under 1.2 s; Hold
 *  stops the timer; Next line past the last line stays on the last line. */
function pacingHolds(R) {
  const wpm = 144;
  const lines = [twelve, "short.", "last line."];
  if (R.dwellMs(twelve, wpm) !== 5000) return false;
  if (R.dwellMs("short.", wpm) !== 1200) return false;
  let p = R.startPrompter(R.prompterIdle(), 1000, lines, wpm);
  if (p.dueAt !== 6000) return false;
  p = R.holdPrompter(p, 3000);
  p = R.tickPrompter(p, 9000, lines, wpm);
  if (p.i !== 0 || !p.held) return false;              // held: the timer does not run
  p = R.goPrompter(p, 10000, lines, wpm);
  if (p.dueAt !== 13000) return false;                 // the 3 s that were left run out from now
  p = R.nextLine(p, 11000, lines, wpm);
  p = R.nextLine(p, 11000, lines, wpm);
  p = R.nextLine(p, 11000, lines, wpm);
  return p.i === 2 && p.dueAt === null;                // past the end stays on the last line, nothing armed
}

if (process.argv.includes("--red")) {
  // a planted lenient prompter: no minimum dwell, and a hold the tick ignores
  const lenient = {
    ...P,
    dwellMs: (line, wpm) => Math.round((P.wordCount(line) / wpm) * 60000),
    tickPrompter: (p, now, lines, wpm) => (p.running && p.dueAt != null && now >= p.dueAt ? { ...p, i: Math.min(p.i + 1, lines.length - 1), dueAt: now + 1000 } : p),
  };
  const caught = !pacingHolds(lenient);
  console.log(`${caught ? "PASS" : "FAIL"} red arm: a lenient prompter (no minimum dwell, a hold the tick ignores) is caught by the pacing check`);
  console.log(`${pacingHolds(P) ? "PASS" : "FAIL"} the real prompter passes the same check`);
  process.exit(caught && pacingHolds(P) ? 0 : 1);
}

// the pace
check(P.wordCount("  a  b c ") === 3 && P.wordCount("") === 0, "wordCount");
check(P.paceWpm(LINES, null) === P.DEFAULT_WPM && P.paceWpm(LINES, 0) === P.DEFAULT_WPM && P.paceWpm([], 30) === P.DEFAULT_WPM, "no run time (or no words): the default pace");
check(P.paceWpm(["a b c d e f g h i j"], 5) === 120, "ten words in five seconds is 120 words a minute");
check(P.paceWpm(["a b"], 60) === P.DEFAULT_WPM && P.paceWpm([twelve, twelve, twelve, twelve, twelve], 2) === P.DEFAULT_WPM, "a pace no reader has (2 or 1800 wpm) falls back to the default");
check(pacingHolds(P), "a 12-word line at 2.4 w/s arms for 5.0 s; a short line holds 1.2 s; Hold stops the timer; Go resumes what was left; Next line past the end stays on the last line");

// the timer
const wpm = 144;
let p = P.prompterIdle();
check(p.i === 0 && !p.running && p.dueAt === null, "idle: nothing armed");
p = P.nextLine(p, 500, LINES, wpm);
check(p.i === 1 && !p.running && p.dueAt === null, "Next line before the disc moves the start line; the clock does not run");
p = P.startPrompter(p, 1000, LINES, wpm);
check(p.running && p.i === 1 && p.dueAt === 1000 + P.dwellMs(LINES[1], wpm), "start: runs from the line he is on");
p = P.tickPrompter(p, p.dueAt - 1, LINES, wpm);
check(p.i === 1, "a tick before the dwell is up moves nothing");
const due1 = p.dueAt;
p = P.tickPrompter(p, due1 + 50, LINES, wpm);
check(p.i === 2 && p.dueAt === due1 + P.dwellMs(LINES[2], wpm), "a tick past the dwell advances and re-arms from when it was due (no drift)");
p = P.tickPrompter(p, p.dueAt + 100000, LINES, wpm);
check(p.i === 3 && p.dueAt === null, "a long stall lands every advance that was due, then the last line holds (nothing armed)");
p = P.nextLine(p, 0, LINES, wpm);
check(p.i === 3 && p.dueAt === null, "Next line on the last line stays there");
p = P.holdPrompter(p, 0);
check(p.held && p.leftMs === null, "Hold on the last line: held, nothing left to run");
p = P.goPrompter(p, 0, LINES, wpm);
check(!p.held && p.dueAt === null, "Go on the last line: nothing arms");
let q = P.startPrompter(P.prompterIdle(), 0, LINES, wpm);
q = P.holdPrompter(q, 1000);
q = P.nextLine(q, 2000, LINES, wpm);
check(q.i === 1 && q.held && q.leftMs === P.dwellMs(LINES[1], wpm) && q.dueAt === null, "Next line while held: the new line waits, held, with its whole dwell");
q = P.toggleHold(q, 3000, LINES, wpm);
check(!q.held && q.dueAt === 3000 + P.dwellMs(LINES[1], wpm), "toggleHold from held is Go");
q = P.toggleHold(q, 3500, LINES, wpm);
check(q.held && q.leftMs === P.dwellMs(LINES[1], wpm) - 500, "toggleHold from going is Hold, with what was left");
check(P.holdPrompter(P.prompterIdle(), 0).held === false && P.goPrompter(P.prompterIdle(), 0, LINES, wpm).held === false, "Hold / Go while idle do nothing");
const s = P.stopPrompter(q);
check(!s.running && !s.held && s.dueAt === null && s.i === 1, "stop: the band keeps the line he stopped on, nothing runs");
check(P.startPrompter(P.prompterIdle(9), 0, LINES, wpm).i === 3, "a start line past the end is the last line");
check(P.startPrompter(P.prompterIdle(), 0, [], wpm).dueAt === null, "no lines: nothing arms");

// the band
check(JSON.stringify(P.bandWindow(LINES, 1, 4)) === JSON.stringify(LINES.slice(1)) && P.bandWindow(LINES, 3, 4).length === 1 && P.bandWindow(LINES, 0, 2).length === 2 && P.bandWindow(LINES, 9, 4)[0] === LINES[3], "the window: the current line first, at most n, past the end = the last");
check(P.bandLines(1) === 4 && P.bandLines(1.29) === 4 && P.bandLines(1.3) === 2 && P.bandLines(NaN) === 4, "four lines, two from 1.3×");
check(P.bandLabel(1, 7, [1], "my dignity.") === "Line 2 of 7, punch: my dignity." && P.bandLabel(0, 7, [1], "x") === "Line 1 of 7: x", "VoiceOver says the position and the word punch");
check(P.holdWord(false) === "Hold" && P.holdWord(true) === "Go", "the left control's word");

if (fails.length) { console.log(`${fails.length} FAILED`); process.exit(1); }
console.log("ALL PASS");
