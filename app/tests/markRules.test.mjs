// Unit test for rules/markRules.ts (the Redline loop on a bit: SPEC 2026-11-16 B12, §3.2, Slice 3). Every line is
// invented.
//   node --experimental-strip-types --no-warnings tests/markRules.test.mjs               the suite
//   node --experimental-strip-types --no-warnings tests/markRules.test.mjs --red         every red arm
//   node --experimental-strip-types --no-warnings tests/markRules.test.mjs --red=<arm>   one of: anchor, keep, stale, tint, target, gone
// A red arm plants a plausible wrong rule and must FAIL the same check the real rule passes:
//   anchor  a rule that trusts where the words were (a line added above moves every mark onto the wrong words)
//   keep    a send that lets a Keep through as a Change (a Keep must reach the model as a Keep, never as changeable)
//   stale   a Use that ignores that he saved newer words after sending (the draft would sit on them unasked)
//   tint    a flip that tints every line of the draft (his own unchanged lines would look rewritten)
//   target  a mark with nothing selected that is on nothing (a tap on a line could not be marked)
//   gone    a send that keeps marks whose words he rewrote (the server would refuse the whole round)
import * as M from "../src/rules/markRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const DOC = [
  "My fridge has an app now.",
  "It texts me when the milk is sad.",
  "",
  "The doorbell judges my outfits.",
  "",
  "My parrot only says legal disclaimers.",
].join("\n");

const mk = (id, type, text, at, note = "") => ({ id, type, text, at, note });

// ---- the checks, each taking the rule under test so a red arm can hand in a wrong one

/** A mark is found by its words: still there after a line is added above it; the nearer of two copies; gone when he
 *  rewrote them. */
function anchors(locate) {
  const m = { text: "the milk is sad", at: DOC.indexOf("the milk is sad") };
  const above = "A new first line.\n" + DOC;
  const twice = DOC + "\nAgain: the milk is sad.";
  return locate(DOC, m) === DOC.indexOf("the milk is sad")
    && locate(above, m) === above.indexOf("the milk is sad")
    && locate(twice, { text: "the milk is sad", at: twice.lastIndexOf("the milk is sad") - 3 }) === twice.lastIndexOf("the milk is sad")
    && locate(DOC.replace("the milk is sad", "the milk is gone"), m) === null;
}
/** Send: document order, Keeps as Keeps with no note, the place where the words are now. */
function sendsKeeps(sendBody) {
  const set = [
    mk("a", "keep", "My parrot only says legal disclaimers.", 90, "ignored"),
    mk("b", "change", "the milk is sad", 40, " sharper "),
    mk("c", "ask", "judges my outfits", 70, "Too close to the mirror bit?"),
  ];
  const s = sendBody(DOC, set);
  const keep = s.marks.find((x) => x.text === "My parrot only says legal disclaimers.");
  return !s.error
    && eq(s.marks.map((x) => x.type), ["change", "ask", "keep"])
    && keep && keep.type === "keep" && keep.note === undefined && keep.at === DOC.indexOf("My parrot")
    && s.marks[0].note === "sharper"
    && sendBody(DOC, [set[0]]).error === "Mark something to change or ask about"
    && sendBody(DOC, [mk("x", "ask", "the milk", 0, "  ")]).error === "An Ask needs its question";
}
/** Use: at once when the draft was made from his newest words; asks first when he saved since. */
function staleAsks(planForUse) {
  const r = (over) => ({ job: "job:x", status: "done", unit: "u1", on: null, n: 1, sent_at: "", marks: [], stale: false, newest: "u1", result: { status: "ok", draft: "A fresh draft." }, ...over });
  return planForUse(r({})) === "use"
    && planForUse(r({ stale: true, newest: "u2" })) === "confirm"
    && planForUse(r({ status: "queued", result: null })) === "none"
    && planForUse(r({ result: { status: "refused", draft: null } })) === "none"
    && planForUse(null) === "none";
}
/** The flip: only the draft's changed or new lines are tinted; his own lines and blank lines never are. */
function tintsChanges(changedLines) {
  const draft = DOC.replace("It texts me when the milk is sad.", "It texts me. The milk is sad. We're all sad.") + "\n\nA new idea.";
  const t = changedLines(DOC, draft);
  return eq(t, [false, true, false, false, false, false, false, true])
    && changedLines(DOC, DOC).every((x) => x === false)
    && eq(changedLines("a\nb", "b\na"), [false, true]);
}
/** What a mark is on: the selected words without their spaces; with nothing selected, the caret's whole line; nothing
 *  on a blank line. */
function targets(targetOf) {
  const i = DOC.indexOf("milk");
  const line = DOC.indexOf("It texts me");
  return eq(targetOf(DOC, { start: i - 1, end: i + 11 }), { text: "milk is sad", at: i })
    && eq(targetOf(DOC, { start: i, end: i }), { text: "It texts me when the milk is sad.", at: line })
    && targetOf(DOC, { start: DOC.indexOf("\n\n") + 1, end: DOC.indexOf("\n\n") + 1 }) === null
    && targetOf(DOC, null) === null;
}
/** A mark whose words he rewrote is not sent (the others are); the tray names it. */
function dropsGone(sendBody) {
  const set = [mk("b", "change", "the milk is sad", 40, "sharper"), mk("g", "change", "the cheese is sad", 40)];
  const s = sendBody(DOC, set);
  return eq(s.marks.map((x) => x.text), ["the milk is sad"]) && eq(s.gone.map((x) => x.id), ["g"]) && !s.error;
}

// ---- the red arms: each a plausible wrong rule
const RED = {
  anchor: () => anchors((text, m) => (m.at <= text.length ? m.at : null)),
  keep: () => sendsKeeps((text, set) => M.sendBody(text, set.map((m) => (m.type === "keep" ? { ...m, type: "change" } : m)))),
  stale: () => staleAsks((r) => (M.roundState(r) === "ready" ? "use" : "none")),
  tint: () => tintsChanges((mine, draft) => draft.split("\n").map((l) => l.trim().length > 0)),
  target: () => targets((text, sel) => (sel && sel.start !== sel.end ? M.targetOf(text, sel) : null)),
  gone: () => dropsGone((text, set) => ({ marks: set.map((m) => ({ type: m.type, text: m.text, at: m.at })), gone: [] })),
};

const red = process.argv.find((a) => a.startsWith("--red"));
if (red) {
  const which = red.includes("=") ? [red.split("=")[1]] : Object.keys(RED);
  let ok = true;
  for (const k of which) {
    const caught = !RED[k]();
    console.log((caught ? "RED CAUGHT " : "RED MISSED ") + k);
    if (!caught) ok = false;
  }
  process.exit(ok ? 0 : 1);
}

check(anchors(M.locate), "a mark is found by its words: after an edit above, the nearer copy, gone when rewritten");
check(sendsKeeps(M.sendBody), "send: document order, a Keep reaches the server as a Keep with no note, Asks need a question");
check(staleAsks(M.planForUse), "use: at once on his newest words, asked first when he saved since sending");
check(tintsChanges(M.changedLines), "the flip tints only the draft's changed or new lines");
check(targets(M.targetOf), "a mark is on the selected words, else the caret's line, never a blank line");
check(dropsGone(M.sendBody), "a mark whose words he rewrote is not sent, and is named");

// the rest of the rules, plainly
{
  let set = [];
  const add = (m) => { const r = M.addMark(DOC, set, m); set = r.marks; return r.error; };
  check(add(mk("1", "change", "the milk is sad", 40)) === undefined, "a mark is added");
  check(add(mk("2", "change", "the milk is sad", 40)) === "Already marked Change", "the same mark twice is refused");
  check(add(mk("3", "keep", "milk", 0)) === "Those words are marked Keep and Change: remove one first", "Keep on changed words is refused");
  check(add(mk("4", "ask", "milk", 0, "why milk?")) === undefined, "an Ask on changed words is fine");
  check(add(mk("5", "keep", "the doorbell", 0)) === "Those words are not in the bit", "words that are not there cannot be marked");
  check(set[0].at === DOC.indexOf("the milk is sad"), "a mark keeps where its words are");
  set = M.noteMark(set, "1", "sharper");
  check(set[0].note === "sharper", "a note is edited");
  set = M.removeMark(set, "4");
  check(set.length === 1, "a mark is removed");
  const full = Array.from({ length: M.MAX_MARKS }, (_, i) => mk(`f${i}`, "keep", "x", 0));
  check(M.addMark(DOC, full, mk("z", "change", "the milk", 0)).error === "At most 20 marks in one send", "twenty marks at most");
  const h = M.highlights(DOC, [mk("k", "keep", "My parrot", 0), mk("c", "change", "parrot only", 0)]);
  check(eq(h.filter((x) => x.type).map((x) => [x.text, x.type]), [["My parrot", "keep"], [" only", "change"]]), "a Keep wins where it overlaps a Change");
  check(h.map((x) => x.text).join("") === DOC, "the highlights are the whole text");
  const NL = String.fromCharCode(10);
  const ht = M.highlights(["a", "bb", "cc"].join(NL), [mk("k", "keep", "c", 0)], [false, true, true]);
  check(eq(ht.map((x) => [x.text, x.type]), [["a" + NL, null], ["bb", "tint"], [NL, null], ["c", "keep"], ["c", "tint"]]), "a changed line is tinted, a mark wins over the tint, a newline is never tinted");
  check(M.sendLabel(3) === "Send 3 marks" && M.sendLabel(1) === "Send 1 mark", "Send says how many");
  const round = { job: "job:x", status: "done", unit: "u", on: null, n: 1, sent_at: "", stale: false, newest: "u", result: { status: "ok", draft: DOC },
    marks: [{ type: "keep", text: "My parrot only says legal disclaimers.", at: 0, note: "" }, { type: "change", text: "the milk is sad", at: 40, note: "x" }] };
  check(M.keepsOn(DOC, round).length === 1 && M.keepsOn(DOC, round)[0].carried === true, "the Keeps of a round are shown on its draft");
  const again = M.marksAgain(DOC.replace("the milk is sad", "the milk is gone"), round, (i) => `r${i}`);
  check(again.marks.length === 1 && again.lost === 1, "send again: the marks whose words are still there come back");
  check(M.roundState({ ...round, status: "queued" }) === "waiting" && M.roundState({ ...round, result: { status: "refused" } }) === "refused", "a round's state");
  check(M.roundLine({ ...round, stale: true }) === "A new draft is ready (made before your last edit)", "a stale draft says so");
  const raw = JSON.stringify({ v: 1, bit: "joke:a", on: null, at: "t", marks: [{ id: "1", type: "keep", text: "x", at: 3, note: "" }, { id: "2", type: "fork", text: "y" }, null] });
  check(M.parseMarkSet(raw)?.marks.length === 1 && M.parseMarkSet("{bad") === null, "the tray read back: only marks");
  check(M.markSetName("joke:a", null) === "joke:a" && M.markSetName("joke:a", "job:1") === "draft~joke:a", "one tray for the bit, one for its draft, the marker first");
}

if (fails.length) {
  console.log(`\n${fails.length} FAILED`);
  process.exit(1);
}
console.log("\nall passed");
