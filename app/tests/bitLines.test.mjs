// Unit test for rules/bitLines.ts (the bit page's pure rules, SPEC 2026-11-03 §3, §14 Slice 1).
//   node --experimental-strip-types --no-warnings tests/bitLines.test.mjs          the suite
//   node --experimental-strip-types --no-warnings tests/bitLines.test.mjs --red    the red arm: the same punch check run
//                                                                                 against a lenient markPunch must FAIL
import * as B from "../src/rules/bitLines.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const throws = (f) => { try { f(); return false; } catch { return true; } };

/** The rule under test: a punch mark on a line that is not there must be refused (the write is strict). */
function punchIsStrict(markPunch) {
  return throws(() => markPunch([], 8, 8)) && throws(() => markPunch([], -1, 8)) && throws(() => markPunch([], 2.5, 8)) && throws(() => markPunch([], 0, 0));
}

if (process.argv.includes("--red")) {
  // a planted lenient rule: it silently clamps instead of refusing. The check must catch it.
  const lenient = (punch, i, count) => [...new Set([...punch, Math.min(Math.max(0, Math.round(i)), Math.max(0, count - 1))])];
  const caught = !punchIsStrict(lenient);
  console.log(`${caught ? "PASS" : "FAIL"} red arm: a lenient markPunch (clamps a bad index) is caught by the strictness check`);
  console.log(`${punchIsStrict(B.markPunch) ? "PASS" : "FAIL"} the real markPunch passes the same check`);
  process.exit(caught && punchIsStrict(B.markPunch) ? 0 : 1);
}

// lines
check(JSON.stringify(B.linesOf("a\n\nb  \r\n  c\n")) === JSON.stringify(["a", "b", "  c"]), "linesOf: newlines split, blanks dropped, trailing space trimmed, leading kept");
check(B.linesOf("") .length === 0 && B.linesOf(null).length === 0 && B.linesOf(undefined).length === 0, "linesOf: nothing is no lines");
check(B.joinLines(["a", " b ", "", "c"]) === "a\nb\nc", "joinLines: one line per newline, trimmed, blanks dropped");
check(B.lineOffset("one\n\n  two\nthree", 1) === 7 && B.lineOffset("one\ntwo", 0) === 0 && B.lineOffset("one\ntwo", 5) === 7, "lineOffset: the caret lands on the i-th non-blank line's first word; past the end = the end");

// the punch (the write is strict, the read is lenient)
check(punchIsStrict(B.markPunch), "markPunch refuses a line past the end, a negative one, a fraction, and any line of an empty bit (RangeError)");
check(JSON.stringify(B.markPunch([], 4, 8)) === "[4]" && JSON.stringify(B.markPunch([4], 4, 8)) === "[]" && JSON.stringify(B.markPunch([4], 1, 8)) === "[1,4]", "markPunch toggles and keeps the marks sorted (several allowed, D13)");
check(JSON.stringify(B.cleanPunch([9, 2, 2, -1, 1.5, "3", 0], 5)) === "[0,2]", "cleanPunch (a read) drops what is out of range or not a whole number, no repeats, sorted");
check(JSON.stringify(B.cleanPunch(null, 5)) === "[]" && JSON.stringify(B.cleanPunch("x", 5)) === "[]", "cleanPunch: not an array is no marks");

// the tags fold: the lines after the last punch
check(B.foldAt(8, [4]) === 5 && B.tagsFolded(8, [4]) === 3, "three tags fold after a punch on line 5 of 8");
check(B.foldAt(8, []) === 8 && B.tagsFolded(8, []) === 0, "no punch: nothing folds (Slice 1 shows no fold)");
check(B.foldAt(8, [7]) === 8 && B.tagsFolded(8, [7]) === 0, "the punch last: nothing folds");
check(B.foldAt(8, [2, 5]) === 6 && B.tagsFolded(8, [2, 5]) === 2, "two punches: the fold follows the last one");
check(B.foldLabel(3) === "+ 3 tags" && B.foldLabel(1) === "+ 1 tag", "the fold's label");
check(B.lineLabel(4, 8, [4], "Still drinks out of the toilet.") === "Line 5 of 8, punch: Still drinks out of the toilet." && B.lineLabel(0, 8, [4], "We've been together eleven years.") === "Line 1 of 8: We've been together eleven years.", "VoiceOver says the position and the word punch (never colour alone)");

// cut, move, insert, in the bit
check(JSON.stringify(B.cutLine(["a", "b", "c"], 1)) === JSON.stringify(["a", "c"]) && B.cutLine(["a"], 0) === null && B.cutLine(["a", "b"], 5) === null, "cutLine removes one line; the only line and a missing line are refused (null)");
check(JSON.stringify(B.moveLine(["a", "b", "c", "d"], 0, 2)) === JSON.stringify(["b", "c", "a", "d"]) && JSON.stringify(B.moveLine(["a", "b"], 1, 1)) === JSON.stringify(["a", "b"]) && JSON.stringify(B.moveLine(["a", "b"], 0, 9)) === JSON.stringify(["a", "b"]), "moveLine moves one line; a no-op or a bad index leaves the order");
check(JSON.stringify(B.insertAfter(["a", "b", "c"], 0, "x\ny")) === JSON.stringify(["a", "x", "y", "b", "c"]) && JSON.stringify(B.insertAfter(["a", "b"], null, "z")) === JSON.stringify(["a", "b", "z"]) && JSON.stringify(B.insertAfter(["a", "b"], 7, "z")) === JSON.stringify(["a", "b", "z"]) && JSON.stringify(B.insertAfter(["a"], 0, "  ")) === JSON.stringify(["a"]), "insertAfter: after the line he is on, at the end with none, past the end = the end, nothing from blank words");
check(B.inBit(["He waits by the door.", "Not sad."], "he waits by the door") && !B.inBit(["He waits by the door."], "supervising") && !B.inBit(["a"], "") && B.inBit(["one", "two"], "one\ntwo") && !B.inBit(["one"], "one\ntwo"), "inBit: the scrap's words are in the bit, case and punctuation aside; every line of a multi-line scrap must be");

// dates and run time
check(B.monthDay("2026-11-02T10:00:00Z") === "Nov 2" && B.monthDay("2026-11-02") === "Nov 2" && B.monthDay("") === "" && B.monthDay(null) === "", "monthDay");
check(B.monthDayLong("2026-11-02") === "November 2", "monthDayLong for VoiceOver");
check(B.runTime(100) === "1:40" && B.runTime(100, true) === "~1:40" && B.runTime(0) === "" && B.runTime(null) === "" && B.runTime(NaN) === "", "runTime: ~ when estimated, nothing when unknown");

// the Versions row
check(B.versionsMeta({ count: 8, newestDate: "2026-11-02", durationS: 100 }) === "v8 · Nov 2 · 1:40", "collapsed: v8 · Nov 2 · 1:40");
check(B.versionsMeta({ count: 9, newestDate: "2026-11-02", durationS: 100, justWrote: true }) === "v9 · just now · 1:40", "right after a write: v9 · just now");
check(B.versionsMeta({ count: 3 }) === "v3" && B.versionsMeta({ count: 3, newestDate: "2026-10-20" }) === "v3 · Oct 20", "no run time, no date: only what there is");
check(B.versionsLabel({ count: 8, newestDate: "2026-11-02", durationS: 100 }) === "Versions, 8, newest November 2, run time 1:40", "VoiceOver for the row");
check(B.stepperLabel(3, 8, "2026-10-20") === "v5 of 8 · Oct 20" && B.stepperLabel(0, 8, null) === "v8 of 8", "the stepper's label counts from the newest");
check(B.stepperSub("Pets notes 2024", 88) === "from Pets notes 2024 · 1:28" && B.stepperSub(null, null) === "" && B.stepperSub(null, 88, true) === "~1:28", "the stepper's second line: the source doc and the run time, what there is");
check(B.stepperValue(3, 8, "2026-10-20") === "5 of 8, October 20", "the adjustable's value (its label says Version)");
check(B.restoreLabel(3, 8) === "Restore v5 as a new version", "the restore row");

// the On tape row
check(B.killedWord(0) === "" && B.killedWord(1) === "killed once" && B.killedWord(2) === "killed twice" && B.killedWord(3) === "killed 3 times", "killed in words");
check(B.tapeMeta(3, 2) === "3 nights · killed twice" && B.tapeMeta(1, 0) === "1 night" && B.tapeMeta(0, 0) === null && B.tapeMeta(0, 0, 2) === "not yet · 2 sound like it" && B.tapeMeta(0, 0, 1) === "not yet · 1 sounds like it", "the row's meta: nights and his ratings; hidden at nothing; echoes when never taped");
check(B.killedCount([{ rating: "killed" }, { rating: "worked" }, {}, { rating: "killed" }]) === 2 && B.killedCount([]) === 0, "killed = his rating on the performed rows");
check(B.tapeMeta(3, 2, 0, 2) === "3 nights · killed twice · filmed 2" && B.tapeMeta(0, 0, 0, 1) === "filmed 1" && B.tapeMeta(0, 0, 2, 1) === "filmed 1", "filmed takes on the row (Film it): after the nights; alone when never performed; before the echoes");

// the versions as the page and the camera read them
const vs = B.versionsOf({ versions: [{ date: "2026-10-01", units: [{ id: "u1", text: "old" }, { id: "skip", text: "" }] }, { date: "2026-11-02", units: [{ id: "u2", text: "new\nline", line_meta: { punch: [1] } }] }] });
check(vs.length === 2 && vs[0].key === "u2" && vs[0].label === "v2" && vs[1].label === "v1" && JSON.stringify(vs[0].punch) === "[1]", "versionsOf: newest first, vN from the newest, empty units skipped, the punch from line_meta");
check(B.versionsOf({ joke: { summary: "an idea", first_seen: "2026-09-01" }, yours: true })[0].text === "an idea" && B.versionsOf({ joke: { summary: "an idea" } }).length === 0 && B.versionsOf(null).length === 0, "the typed idea stands in for a bit of his with no unit; nothing for nothing");

// the reason under Write, the bar, the AX rules
check(B.writeReason({ vi: 0, count: 8, online: true }) === null && B.writeReason({ vi: 3, count: 8, online: true }) === "Reading v5" && B.writeReason({ vi: 3, count: 8, online: false }) === "Offline · reading only", "one reason: offline wins, then Reading vN, else none");
check(B.barEnabled([], "") === false && B.barEnabled([], " an idea ") === true && B.barEnabled(["a line"], null) === true, "the sparkle and To a set wake with a line or an idea");
check(B.ideaLines(1) === 1 && B.ideaLines(1.29) === 1 && B.ideaLines(1.3) === 2 && B.ideaLines(2) === 2, "the idea line gets two lines from the re-stack point");
check(B.stackMeta(1) === false && B.stackMeta(1.3) === true, "a row's meta re-stacks at the same point");
check(B.rowHeight(48, 1) === 48 && B.rowHeight(48, 1.3) === 62 && B.rowHeight(48, 3) === 77 && B.rowHeight(48, 0.8) === 48 && B.rowHeight(48, NaN) === 48, "row heights scale with the OS text size, capped at 1.6×, never under the base");

// the pile in Slice 1: memos on this phone first, then the attached notes, a filed memo counted once
const scraps = B.pileScraps(
  [{ id: "at:1", text: "a jot", date: "2026-10-01", reasons: ["same dog"], score: 0.9 }, { id: "at:2", text: "the memo's words", date: "2026-10-30" }, { id: "at:3", text: "his own attach", date: "2026-10-20" }],
  [{ id: "m1", text: "the memo's words", recorded_at: "2026-10-30T20:00:00Z", merge_id: "at:2" }, { id: "m0", text: "", recorded_at: "2026-10-29T20:00:00Z" }],
);
check(scraps.length === 4 && scraps[0].id === "memo:m1" && scraps[1].id === "memo:m0" && scraps[2].id === "at:3" && scraps[3].id === "at:1", `memos first (newest first), then notes newest first, the filed memo's note dropped: ${scraps.map((s) => s.id).join(" ")}`);
check(scraps[3].ai === true && scraps[2].ai === false && scraps[0].kind === "memo" && scraps[0].recId === "m1", "a note the judge filed carries the AI badge; his own does not; a memo scrap keeps its recording id");

if (fails.length) { console.log(`${fails.length} FAILED`); process.exit(1); }
console.log("ALL PASS");
