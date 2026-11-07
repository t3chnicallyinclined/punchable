// Unit test for rules/rantRules.ts and rules/hookRules.ts (the Rant tab, SPEC 2026-11-06 §2, §3, §5): the tab's lists
// (Drafts vs Finished, a topic's row, Gather's ticks), Film a rant's target, the Keep / Skip assembly of a drafted
// rant and the hook check on the script. Every joke here is invented.
//   node --experimental-strip-types --no-warnings tests/rantRules.test.mjs          the suite
//   node --experimental-strip-types --no-warnings tests/rantRules.test.mjs --red    the red arm: lenient rules (a filmed
//                                                                                  rant still a draft, an undecided new
//                                                                                  line kept, every hit counted, a hook
//                                                                                  never late) must FAIL
import * as R from "../src/rules/rantRules.ts";
import * as H from "../src/rules/hookRules.ts";

const red = process.argv.includes("--red");
const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// ---- the rules under test, or the planted lenient ones (--red)
const L = red ? {
  ...R,
  splitRants: (rants, takes) => ({ drafts: rants, finished: R.splitRants(rants, takes).finished }),
  isIn: () => true,
  assemble: (lines) => lines.map((l) => l.text.trim()),
  matchCount: (r) => (r?.bits || []).length,
  gatherHits: (r, a) => R.gatherHits(r, a).map((h) => ({ ...h, ticked: true })),
} : R;
const HK = red ? { ...H, hookCheck: (lines, his, wpm) => ({ ...H.hookCheck(lines, his, wpm), late: false }) } : H;

const RANTS = [
  { id: "joke:rant-b", title: "Self-checkout machines", short_name: "Self-checkout", versions: 2, v: 2, unit: "unit:b2", at: "2026-11-06", hook: true },
  { id: "joke:rant-a", title: "Airport bins", short_name: "Airport bins", versions: 1, v: 1, unit: "unit:a1", at: "2026-11-05" },
];
const TAKES = [
  { id: "rec-1", kind: "film", recorded_at: "2026-11-06T20:00:00Z", duration_s: 58, bit: { id: "joke:rant-a", title: "Airport bins" } },
  { id: "rec-2", kind: "memo", recorded_at: "2026-11-06T21:00:00Z", duration_s: 9, bit: { id: "joke:rant-b", title: "x" } },
  { id: "rec-3", kind: "film", recorded_at: "2026-11-06T22:00:00Z", duration_s: 40, bit: { id: "joke:not-a-rant", title: "x" } },
];

// ---- Drafts and Finished
const s = L.splitRants(RANTS, TAKES);
check(eq(s.drafts.map((r) => r.id), ["joke:rant-b"]), "a filmed rant leaves Drafts; a memo on a rant does not film it");
check(eq(s.finished.map((f) => f.take.id), ["rec-1"]), "Finished is the filmed takes of rants only (a take on another bit is not a rant)");
check(R.draftSub(RANTS[0]) === "v2 · Nov 6 · hook", "a draft reads v2 · Nov 6 · hook");
check(R.finishedSub(TAKES[0]) === "Nov 6 · 0:58 · in Photos", "a finished video reads Nov 6 · 0:58 · in Photos");
check(eq(R.filmTarget([]), { k: "none" }) && eq(R.filmTarget([RANTS[0]]), { k: "one", id: "joke:rant-b" }) && R.filmTarget(RANTS).k === "pick",
  "Film a rant: nothing without a draft, the one draft, a pick among several");

// ---- My topics
const SEARCH = { bits: [{ id: "joke:a", title: "A", distance: 0.2 }, { id: "joke:b", title: "B", distance: null }, { id: "joke:c", title: "C", distance: 0.55 }, { id: "joke:d", title: "D", distance: 0.39 }],
  moments: [{ id: "performance:p1", text: "  unexpected item\n in the bagging area ", venue: "The Cellar", date: "2025-03-04" }] };
check(L.matchCount(SEARCH) === 3, "a topic counts the close bits only (word matches and distance under 0.4)");
check(R.topicSub({ id: "topic:a", text: "x" }, 3) === "3 bits" && R.topicSub({ id: "topic:a", text: "x" }, 1) === "1 bit", "the row says how many bits");
check(R.topicSub({ id: "topic:a", text: "x", ranted_at: "2026-11-06T10:00:00Z" }, 3) === "ranted Nov 6", "a topic used for a rant reads ranted Nov 6");

// ---- Gather
const hits = L.gatherHits(SEARCH);
check(eq(hits.map((h) => [h.id, h.ticked]), [["joke:a", true], ["joke:b", true], ["joke:c", false], ["joke:d", true], ["performance:p1", false]]),
  "Gather ticks the close bits and never a far one or a taped moment");
check(hits[4].title === "unexpected item in the bagging area" && hits[4].sub === "On tape · The Cellar · Mar 4", "a moment row: its words, On tape and where");
const more = L.gatherHits({ bits: [{ id: "joke:a", title: "A", distance: 0.1 }, { id: "joke:e", title: "E", distance: 0.1 }] }, hits);
check(more.length === 6 && more[5].id === "joke:e" && more[5].ticked === false, "Find more adds new hits unticked, never twice");
check(R.tickedIds(hits.map((h) => ({ ...h, ticked: true }))).length === 5 && R.tickedIds(Array.from({ length: 20 }, (_, i) => ({ id: `joke:${i}`, ticked: true }))).length === 12,
  "at most 12 sources go to the draft");

// ---- a drafted rant: Keep / Skip
const LINES = [
  { role: "hook", text: "Stop scanning your own groceries.", his: false },
  { role: "setup", text: "Self-checkout is the store hiring you for free.", his: true, source: "joke:self-checkout", source_label: "Self-checkout v8" },
  { role: "beat", text: "You bag. You weigh. You apologise to a machine.", his: false },
  { role: "turn", text: "my dignity", his: true, source: "performance:p1", source_label: "On tape" },
  { role: "button", text: "I quit. I never worked there.", his: false },
];
check(eq(R.assemble(LINES, []), LINES.map((l) => l.text)), "no review step: every line is in until he takes it out");
const marks = [undefined, undefined, false, false, undefined];
check(eq(L.assemble(LINES, marks), [LINES[0].text, LINES[1].text, LINES[4].text]), "Skip one new line, cut his moment: the rant is the rest");
check(R.lineAction(LINES[1], undefined) === "Cut" && R.lineAction(LINES[0], undefined) === "Skip" && R.lineAction(LINES[0], false) === "Put back", "the button: Cut his, Skip a new one, Put back");
check(eq(R.skippedNew(LINES, marks), [LINES[2].text]), "the skipped new line teaches the profile; his cut line never does");
check(eq(R.fromLinks(LINES, marks), ["joke:self-checkout"]), "the saved rant links the bits whose lines he kept");
check(R.canSaveRant(LINES, []) && !R.canSaveRant([LINES[0]], [false]), "Save is live while the rant has a line");
check(R.rantTitle("  self-checkout   machines ") === "Self-checkout machines", "the new bit is named for the topic");
check(R.lineLabel(LINES[1], 1, 5, undefined) === "Line 2 of 5, the setup. Yours, from Self-checkout v8: Self-checkout is the store hiring you for free.", "VoiceOver says whose line it is");

// ---- Hook check on the script
const c1 = HK.hookCheck(["Nobody asked for self-checkout.", "x"], [false, true]);
check(c1 && c1.seconds === 1.7 && !c1.late && c1.shape === "claim" && c1.wordsBeforeHis === 4, "a 4-word claim lands at 1.7 s, 4 words before his");
check(H.hookWord(H.hookCheck(["Five words make this hook."], [true])) === "Hook lands at 2.1 s", "five words at 144 wpm: Hook lands at 2.1 s");
const late = HK.hookCheck(["So yesterday I went to the store and I tried to buy one single thing"], [false], 144);
check(late && late.late && H.hookWord(late) === "Hook lands late: 6.3 s", "a long hook is flagged late");
check(late && late.shape === "scene" && late.wordsBeforeHis === null, "it is a scene, and none of it is his");
check(H.shapeOf("Have you ever been fired by a machine?”") === "question", "a question, closing quote and all");
check(H.hookDetail(c1) === "a claim · 4 words before yours", "the detail line");
check(H.hookCheck([], []) === null, "no lines, no check");

if (fails.length) {
  console.log(`\n${fails.length} FAILED${red ? " (the red arm: lenient rules must fail)" : ""}`);
  process.exit(red ? 0 : 1);
}
if (red) { console.log("\nthe red arm PASSED with lenient rules: the suite cannot fail"); process.exit(1); }
console.log("\nall passed");
