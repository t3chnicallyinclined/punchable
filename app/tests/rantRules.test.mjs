// Unit test for rules/rantRules.ts and rules/hookRules.ts (the Rant tab as his creative space, SPEC 2026-11-06 §12;
// the hook check §5): a topic's row on the tab (every number has something behind it), where he left off, the topic
// page's lists with his taps applied (Kept, Found, Dropped: each card in one list), a card's head and lines, what a
// draft draws on, the filmed takes, the rants no topic names, and the hook check on the script. Every joke here is
// invented.
//   node --experimental-strip-types --no-warnings tests/rantRules.test.mjs          the suite
//   node --experimental-strip-types --no-warnings tests/rantRules.test.mjs --red    the red arm: lenient rules (a
//                                                                                  count with nothing behind it, a kept
//                                                                                  card still in Found, a fourth line on
//                                                                                  the card, a hook never late) must FAIL
import * as R from "../src/rules/rantRules.ts";
import * as H from "../src/rules/hookRules.ts";

const red = process.argv.includes("--red");
const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// ---- the rules under test, or the planted lenient ones (--red)
const L = red ? {
  ...R,
  // the build-25 bug: a number shown before the server looked, and the search's cap as a count
  topicSub: (t, f) => (t.found == null ? "12 bits" : R.topicSub(t, f)),
  // a tap that adds the card to Kept but leaves it in Found too
  materialRows: (p, marks, extra) => { const r = R.materialRows(p, marks, extra); return { ...r, found: [...(p?.found || []), ...(extra || [])] }; },
  cardLines: (m, open) => ({ ...R.cardLines(m, true), more: 0 }),
} : R;
const HK = red ? { ...H, hookCheck: (lines, his, wpm) => ({ ...H.hookCheck(lines, his, wpm), late: false }) } : H;

// ---- a topic's row (§12.2)
check(L.topicSub({ id: "t", text: "x", rant_id: "joke:r", v: 3 }, "2026-11-06T20:00:00Z") === "Rant v3 · filmed Nov 6", "a filmed rant: Rant v3 · filmed Nov 6");
check(L.topicSub({ id: "t", text: "x", rant_id: "joke:r", v: 3, kept: 2 }) === "Rant v3", "a rant leads, whatever else is there");
check(L.topicSub({ id: "t", text: "x", kept: 3, found: 9, jots: 1 }) === "3 kept · 9 found · 1 jot", "3 kept · 9 found · 1 jot");
check(L.topicSub({ id: "t", text: "x", kept: 0, found: 30, found_more: true, jots: 2 }) === "30+ found · 2 jots", "zero parts left out; 30+ at the cap");
check(L.topicSub({ id: "t", text: "x", kept: 0, found: 0, jots: 0 }) === "Nothing yet", "Nothing yet");
check(L.topicSub({ id: "t", text: "x", kept: 0, found: null, jots: 0 }) === "", "before the server looked: no number at all");
check(L.topicSub({ id: "t", text: "x", kept: 1, found: null }) === "1 kept", "what is known is said, what is not is left out");

// ---- where he left off; rants no topic names
const TOPICS = [
  { id: "topic:a", text: "Airport bins", kept: 1, found: 4 },
  { id: "topic:b", text: "Self-checkout", rant_id: "joke:r1", v: 3, rants: ["joke:r0", "joke:r1"] },
  { id: "topic:c", text: "Phone trees", rant_id: "joke:r2", v: 1, rants: ["joke:r2"] },
];
check(R.leftOff(TOPICS)?.id === "topic:b", "where he left off: the most recently touched topic with a rant");
check(R.leftOff([TOPICS[0]]) === null, "no rant yet: no card");
const RANTS = [{ id: "joke:r1" }, { id: "joke:r9" }, { id: "joke:r0" }];
check(eq(R.rantsWithoutTopic(RANTS, TOPICS).map((r) => r.id), ["joke:r9"]), "a rant made from a bit's page, named by no topic, is listed on its own");

// ---- the topic page's lists (§12.3): each card in one list, his taps at once
const m = (ref, kind = "bit", extra = {}) => ({ ref, kind, label: `${ref} v2`, short_name: ref, v: 2, at: "2026-10-29", words: "One.\nTwo.\nThree.\nFour.\nFive.", ...extra });
const PAGE = { kept: [m("joke:k1"), m("joke:j1", "jot")], found: [m("joke:f1"), m("joke:f2"), m("perf:p1", "moment")], dropped: [m("joke:d1")] };
let rows = L.materialRows(PAGE);
check(eq(rows.kept.map((x) => x.ref), ["joke:j1", "joke:k1"]), "Kept: his jots first, then the bits he kept");
rows = L.materialRows(PAGE, { "joke:f2": "kept" });
check(rows.kept[0].ref !== "joke:f2" && rows.kept.some((x) => x.ref === "joke:f2"), "+ on a Found card: it is kept (below his jots)");
check(!rows.found.some((x) => x.ref === "joke:f2"), "and it leaves Found: one card, one list");
rows = L.materialRows(PAGE, { "joke:f1": "dropped", "joke:d1": null });
check(eq(rows.dropped.map((x) => x.ref), ["joke:f1"]), "Drop: it goes to Dropped");
check(rows.found.some((x) => x.ref === "joke:d1") && !rows.found.some((x) => x.ref === "joke:f1"), "Put back: it is in Found again, and only there");
rows = L.materialRows(PAGE, {}, [m("joke:x1"), m("joke:f1")]);
check(eq(rows.found.map((x) => x.ref), ["joke:f1", "joke:f2", "perf:p1", "joke:x1"]), "Find more appends, never repeats a card");
check(eq(L.materialRows(null).found, []), "no page yet: empty lists");

// ---- what a draft draws on (C4)
check(eq(R.draftFrom(R.materialRows(PAGE)), { n: 2, word: "2 kept" }), "Draft it from 2 kept");
check(R.draftFrom({ kept: [], found: [m("a"), m("b")] }, true)?.word === "2+ found", "nothing kept: from the found");
check(R.draftFrom({ kept: [], found: [] }) === null, "nothing to draw on");

// ---- a card (his words on screen)
const card = L.cardLines(m("joke:k1", "bit", { punch: [2, 7] }));
check(eq(card.lines, ["One.", "Two.", "Three."]) && card.more === 2 && eq(card.punch, [2]), "a card shows three lines of the newest version, the punch within them");
check(R.moreWord(card.more) === "+ 2 more lines" && R.moreWord(1) === "+ 1 more line", "+ 2 more lines unfolds the rest");
check(R.cardLines(m("joke:k1"), true).lines.length === 5, "unfolded: every line");
check(R.cardHead(m("joke:k1")) === "joke:k1 · v2 · Oct 29", "a bit: short name · v2 · Oct 29");
check(R.cardHead({ ref: "perf:p", kind: "moment", label: "On tape", at: "2026-11-02", venue: "The Back Room", start_ms: 41000, words: "x" }) === "On tape · Nov 2 · The Back Room · 0:41", "a moment: On tape · Nov 2 · The Back Room · 0:41");
check(R.cardHead({ ref: "joke:j", kind: "jot", label: "x", at: "2026-11-07", words: "x" }) === "Jot · Nov 7", "a jot: Jot · Nov 7");
check(R.cardA11y(m("joke:k1"), "kept").startsWith("joke:k1, version 2, kept: One. Two."), "VoiceOver: one element, whose and its words");
check(R.sourceLabel(m("joke:k1")) === "joke:k1 v2", "a pulled line's source: the bit and its version");

// ---- filmed
const TAKES = [
  { id: "rec-1", kind: "film", recorded_at: "2026-11-06T20:00:00Z", duration_s: 58, bit: { id: "joke:r1", title: "x" } },
  { id: "rec-2", kind: "memo", recorded_at: "2026-11-06T21:00:00Z", duration_s: 9, bit: { id: "joke:r1", title: "x" } },
  { id: "rec-3", kind: "film", recorded_at: "2026-11-07T22:00:00Z", duration_s: 40, bit: { id: "joke:not-a-rant", title: "x" } },
];
const f = R.filmedList(TAKES, [{ id: "joke:r1", title: "S", short_name: "S", versions: 3, v: 3, unit: "u" }], TOPICS);
check(f.length === 1 && f[0].take.id === "rec-1" && f[0].topic?.id === "topic:b", "Filmed: takes of rants only (never a memo), with the topic it came from");
check(R.filmedSub(TAKES[0]) === "Nov 6 · 0:58 · in Photos", "Nov 6 · 0:58 · in Photos");
check(R.takesOf(TAKES, "joke:r1").length === 1, "a rant's takes: films only");
check(R.runOf(144) === "1:00" && R.runOf(null) === "0:00", "the run time at his pace");

// ---- Hook check on the script
const c1 = HK.hookCheck(["Nobody asked for self-checkout.", "x"], [false, true]);
check(c1 && c1.seconds === 1.7 && !c1.late && c1.shape === "claim", "a 4-word claim lands at 1.7 s");
check(H.hookWord(H.hookCheck(["Five words make this hook."], [true])) === "Hook lands at 2.1 s", "five words at 144 wpm: Hook lands at 2.1 s");
const late = HK.hookCheck(["So yesterday I went to the store and I tried to buy one single thing"], [false], 144);
check(late && late.late && H.hookWord(late) === "Hook lands late: 6.3 s", "a long hook is flagged late");
check(H.hookCheck([], []) === null, "no lines, no check");

if (fails.length) {
  console.log(`\n${fails.length} FAILED${red ? " (the red arm: lenient rules must fail)" : ""}`);
  process.exit(red ? 0 : 1);
}
if (red) { console.log("\nthe red arm PASSED with lenient rules: the suite cannot fail"); process.exit(1); }
console.log("\nall passed");
