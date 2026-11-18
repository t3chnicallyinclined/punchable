// Unit test for src/rules/runRules.ts (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §3.4, §4.4, Slice 5): the
// walk-through's splits per bit, what goes to the server, the transcript cut by bit, the After rows.
//   node --experimental-strip-types --no-warnings tests/runRules.test.mjs
//   node --experimental-strip-types --no-warnings tests/runRules.test.mjs --red    the red arm: a planted rule that takes
//       the split positions from the wall clock (Date.now() since the start) instead of the recorder's position, so a
//       phone call's pause lands inside a bit's span and every later span sits late by the call
import * as R from "../src/rules/runRules.ts";
import { clockElapsed, clockObserve, clockResume, clockStart } from "../src/rules/recorderLogic.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const IDS = ["joke:fridge", "joke:roommate", "joke:bus"];
const BITS = [{ id: IDS[0], name: "Fridge" }, { id: IDS[1], name: "Roommate" }, { id: IDS[2], name: "Bus" }];

/** A take with a 30-second call while on the second bit, positions from `posOf(clock, now)`. The rule under test: no
 *  split covers the call, and the spans add up to the time on tape. */
function callIsInNoSplit(posOf) {
  let c = clockStart(1_000_000);
  let r = R.runStart(R.runIdle(), posOf(c, 1_000_000));
  r = R.runGo(r, 1, posOf(c, 1_060_000), IDS);          // 60 s on Fridge, Next
  c = clockObserve(c, false, 1_090_000);                  // 30 s later a call takes the microphone
  c = clockObserve(c, true, 1_120_000);                   // 30 s call, it comes back
  r = R.runGo(r, 2, posOf(c, 1_150_000), IDS);          // 30 s more on Roommate (60 s on tape), Next
  const end = 1_200_000;
  const splits = R.runEnd(r, posOf(c, end), IDS);
  const onTape = clockElapsed(c, end);                    // 170 s
  const total = splits.reduce((a, s) => a + s.end_ms - s.start_ms, 0);
  const roommate = splits.find((s) => s.joke_id === IDS[1]);
  return total === onTape && !!roommate && roommate.end_ms - roommate.start_ms === 60_000 && splits.every((s) => s.end_ms <= onTape);
}
const recorderPos = (c, now) => clockElapsed(c, now);
const wallPos = (c, now) => now - c.startedAt;

if (process.argv.includes("--red")) {
  const caught = !callIsInNoSplit(wallPos);
  console.log(`${caught ? "PASS" : "FAIL"} red arm: splits from the wall clock are caught (the call sits inside Roommate)`);
  const real = callIsInNoSplit(recorderPos);
  console.log(`${real ? "PASS" : "FAIL"} the recorder's position passes the same check`);
  process.exit(caught && real ? 0 : 1);
}

// ---- on stage: the line beside the clock and the line over Next
{
  check(R.stageHeader(1, 5, 480) === "2 of 5 · 8 min slot" && R.stageHeader(0, 3, 0) === "1 of 3" && R.stageHeader(9, 3, 420) === "3 of 3 · 7 min slot" && R.stageHeader(0, 0, 420) === "7 min slot", "the line beside the clock: where he is, and the slot when there is one");
  const names = ["Gym at 5 a.m.", "Self-checkout", "My dad's voicemails"];
  check(R.nextLine(names, 1) === "Next: My dad's voicemails" && R.nextLine(names, 0) === "Next: Self-checkout", "the line over Next names the bit that comes after this one");
  check(R.nextLine(names, 2) === "Last bit" && R.nextLine([], 0) === "", "on the last bit it says so; an empty set says nothing");
  // the red arm, run here so it cannot be skipped: a line that names the bit he is ON would pass none of the above
  const wrong = (n, i) => (n[i] ? `Next: ${n[i]}` : "");
  check(wrong(names, 1) !== R.nextLine(names, 1) && wrong(names, 2) !== R.nextLine(names, 2), "red arm: a line that names the current bit is caught");
  check(R.HOLD_MS >= 500 && R.HOLD_MS <= 1000, "Hold to end is a real hold, and a short one");
}
// ---- Next then Back then Next: three spans, the second visit to a bit is its own span
{
  let r = R.runStart(R.runIdle(), 0);
  r = R.runGo(r, 1, 40_000, IDS);   // Fridge 0-40
  r = R.runGo(r, 0, 55_000, IDS);   // Roommate 40-55, Back
  r = R.runGo(r, 1, 70_000, IDS);   // Fridge 55-70, Next
  const s = R.runEnd(r, 100_000, IDS); // Roommate 70-100
  check(s.length === 4 && eq(s.map((x) => x.joke_id), [IDS[0], IDS[1], IDS[0], IDS[1]]), "Next, Back, Next, End: four spans, a bit visited twice has two");
  check(s.every((x, k) => k === 0 || x.start_ms === s[k - 1].end_ms), "the spans meet end to start: no gap, no overlap");
}
// ---- before the disc nothing is recorded; a tap through a bit is not time on it
{
  let r = R.runIdle();
  r = R.runGo(r, 2, 5_000, IDS);
  check(r.i === 2 && r.at === null && r.splits.length === 0, "Next before the disc moves the highlight and records nothing");
  r = R.runStart(R.runIdle(), 0);
  r = R.runGo(r, 1, 30_000, IDS);
  r = R.runGo(r, 2, 30_100, IDS);   // 100 ms on Roommate: passing through
  const s = R.runEnd(r, 60_000, IDS);
  check(s.length === 2 && !s.some((x) => x.joke_id === IDS[1]), "a bit passed in under a quarter second gets no span");
  check(R.runGo(r, 9, 61_000, IDS).i === 2 && R.runGo(r, -3, 61_000, IDS).i === 0, "Next and Back stay inside the set");
}
// ---- a call's pause is in no split (the recorder's clock), and the planted wall clock is caught
check(callIsInNoSplit(recorderPos), "a 30 s call on the second bit: no split covers it, the spans add up to the time on tape");
check(!callIsInNoSplit(wallPos), "the same take with wall-clock positions fails the check (the red arm's plant)");

// ---- what goes to the server
{
  const raw = [
    { joke_id: IDS[1], start_ms: 50_000.4, end_ms: 120_000 },
    { joke_id: IDS[0], start_ms: -5, end_ms: 50_600 },          // overlaps the next, starts below zero
    { joke_id: "joke:gone", start_ms: 120_000, end_ms: 130_000 }, // not on the set
    { joke_id: IDS[2], start_ms: 130_000, end_ms: 999_000 },      // past the end
    { joke_id: IDS[2], start_ms: 200_000, end_ms: 200_000 },      // nothing
  ];
  const s = R.splitsForSend(raw, 180_000.7, IDS);
  check(eq(s, [
    { joke_id: IDS[0], start_ms: 0, end_ms: 50_600 },
    { joke_id: IDS[1], start_ms: 50_600, end_ms: 120_000 },
    { joke_id: IDS[2], start_ms: 130_000, end_ms: 180_001 },
  ]), "sent splits are whole ms, in order, never overlapping, inside the take, on the set only");
  const many = Array.from({ length: 700 }, (_, k) => ({ joke_id: IDS[k % 3], start_ms: k * 1000, end_ms: k * 1000 + 900 }));
  check(R.splitsForSend(many, 800_000, IDS).length === R.SPLITS_MAX, "at most 500 splits go up");
}
// ---- the transcript cut by bit
{
  const segs = [
    { text: "hello room", start: 0, end: 2 },                   // before the first span (2 s in)
    { text: "my fridge has wifi now", start: 3, end: 6 },
    { text: "it tells me the door is open", start: 6, end: 9 },
    { text: "anyway my roommate", start: 10, end: 13 },
    { text: "the fridge again", start: 21, end: 23 },
  ];
  const splits = [
    { joke_id: IDS[0], start_ms: 2_500, end_ms: 9_500 },
    { joke_id: IDS[1], start_ms: 9_500, end_ms: 20_000 },
    { joke_id: IDS[0], start_ms: 20_000, end_ms: 30_000 },
  ];
  const w = R.wordsBySplit(segs, splits);
  check(w[IDS[0]] === "my fridge has wifi now it tells me the door is open the fridge again", "a bit's words are its spans' segments in time order");
  check(w[IDS[1]] === "anyway my roommate" && !Object.values(w).join(" ").includes("hello room"), "words before the first span are no bit's");
  const rows = R.afterRows(BITS, splits, w, { [IDS[0]]: "My fridge has wifi now.\nIt tells me the door is open.", [IDS[1]]: "Roommate bit." });
  check(rows[0].ms === 17_000 && rows[0].spans === 2 && !rows[0].skipped && !rows[0].changed, "After: Fridge 17 s over two visits, said as written");
  check(rows[2].skipped && rows[2].ms === 0 && rows[2].words === "", "After: a bit he never reached says skipped");
  check(R.afterRows(BITS, splits, null)[0].words === null, "After: no words until the phone has them");
  check(R.saidDifferently("purple umbrellas collapse during every windstorm outside honestly", "My fridge has wifi now.") === true, "said differently: most of what he said is not in the bit");
  check(R.saidDifferently("my fridge has wifi now honestly", "My fridge has wifi now.") === false, "said as written: one new word is not a change");
}
// ---- the dock's words (§4.4: Next never becomes Finish under the thumb)
check(eq(R.nextControl(0, 3), { label: "Next bit", disabled: false }) && eq(R.nextControl(2, 3), { label: "Last bit", disabled: true }), "Next greys as Last bit on the last bit");
check(R.backControl(0).disabled && !R.backControl(1).disabled, "Back is off on the first bit");

// ---- a filmed set needs room for a long take
{
  const GB = 1024 * 1024 * 1024;
  check(R.setRoom(null) === null, "unknown free space stays unknown");
  check(R.setFileCap(3 * GB) > 1 * GB && R.setFileCap(3 * GB) < 1.5 * GB && R.setFileCap(null) === null, "the file cap leaves room for the Photos copy");
  check(R.setRoom(1.5 * GB) < 500 * 1024 * 1024 && R.setRoom(2.5 * GB) > 500 * 1024 * 1024, "1.5 GB free is no room for a filmed set; 2.5 GB is");
}
console.log(fails.length ? `\n${fails.length} FAILED` : "\nall passed");
process.exit(fails.length ? 1 : 0);
