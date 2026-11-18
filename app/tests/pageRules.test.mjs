// Unit test for rules/pageRules.ts (a bit is a page, marking is selecting: SPEC 2026-11-17 one notebook, screens 2
// and 3) and the `intent` a mark carries (rules/markRules.ts sendBody). Every line is invented.
//   node --experimental-strip-types --no-warnings tests/pageRules.test.mjs               the suite
//   node --experimental-strip-types --no-warnings tests/pageRules.test.mjs --red         every red arm
//   node --experimental-strip-types --no-warnings tests/pageRules.test.mjs --red=<arm>   one of: switch, exact, sent, note, punch
// A red arm plants a plausible wrong rule and must FAIL the same check the real rule passes:
//   switch  a button that only ever adds (a second tap would stack a second mark on the same words)
//   exact   a button that lights for any mark that overlaps the selection (it would switch off a mark on other words)
//   sent    a send that leaves out how he asked (Shorter would reach the model as a plain "make it land harder")
//   note    a note saved empty that leaves an empty mark behind (a mark that asks for nothing would be sent)
//   punch   a main button that sends while a draft is still being written (two rounds, the first thrown away)
import * as P from "../src/rules/pageRules.ts";
import * as M from "../src/rules/markRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const DOC = [
  "The machine says unexpected item like I built it wrong.",
  "",
  "I work here now. No training, no vest, and somehow I'm already in trouble with my manager, who is a screen.",
  "",
  "Then a real employee walks over and looks at me like I tried to steal a lime.",
].join("\n");
const at = (t) => ({ text: t, at: DOC.indexOf(t) });
const SEL = at("somehow I'm already in trouble with my manager, who is a screen.");
const OTHER = at("in trouble with my manager");

// ---- the checks, each taking the rule under test so a red arm can hand in a wrong one

/** A button is a switch: one tap marks the words, the same tap again takes the mark off, another button changes how
 *  he asked without a second mark. */
function isSwitch(toggleAsk) {
  const a = toggleAsk(DOC, [], SEL, "funnier", "m1");
  const b = toggleAsk(DOC, a.marks, SEL, "shorter", "m2");
  const c = toggleAsk(DOC, b.marks, SEL, "shorter", "m3");
  return a.marks.length === 1 && a.marks[0].intent === "funnier" && a.marks[0].type === "change" && a.said === "Marked: Funnier"
    && b.marks.length === 1 && b.marks[0].intent === "shorter" && b.marks[0].id === "m1"
    && c.marks.length === 0 && c.said === "Mark removed";
}
/** The buttons are about exactly the selected words: a mark on other words, even inside them, lights nothing here and
 *  is not touched by a tap here. */
function isExact(markOn) {
  const set = P.toggleAsk(DOC, [], OTHER, "funnier", "m1").marks;
  return markOn(DOC, set, SEL) === null
    && markOn(DOC, set, OTHER)?.id === "m1"
    && markOn(DOC, set, null) === null
    && eq(P.lit(markOn(DOC, set, OTHER)), { funnier: true, shorter: false, options: false, note: false });
}
/** Send carries how he asked on each Change, and nothing on a mark that has none. */
function sendsIntent(sendBody) {
  let set = P.toggleAsk(DOC, [], SEL, "shorter", "m1").marks;
  const n = P.markForNote(DOC, set, at("looks at me like I tried to steal a lime."), "m2");
  set = P.saveNote(n.marks, "m2", "this is the laugh, build to it");
  const s = sendBody(DOC, set);
  return !s.error && s.marks.length === 2
    && s.marks[0].intent === "shorter" && s.marks[0].note === undefined && s.marks[0].type === "change"
    && s.marks[1].intent === undefined && s.marks[1].note === "this is the laugh, build to it"
    && !("intent" in s.marks[1]);
}
/** A note typed and then emptied on a mark that asks for nothing else takes the mark with it; on a Funnier mark the
 *  mark stays. */
function emptyNoteGoes(saveNote) {
  const n = P.markForNote(DOC, [], SEL, "m1");
  const plain = saveNote(saveNote(n.marks, "m1", "tighter"), "m1", "   ");
  const withAsk = saveNote(P.toggleAsk(DOC, [], SEL, "funnier", "m2").marks, "m2", "");
  return n.mark?.id === "m1" && plain.length === 0 && withAsk.length === 1 && withAsk[0].intent === "funnier";
}
/** The main button: sends his marks; never while a round is being written; opens a draft that waits; asks what to
 *  punch up when nothing is marked; off with a reason offline. */
function punches(punch) {
  const base = { marks: 0, round: "none", online: true, sending: false, empty: false };
  return punch({ ...base, marks: 2 }).does === "send" && punch({ ...base, marks: 2 }).label === "Get a new draft"
    && punch({ ...base, marks: 2, round: "waiting" }).does === "open"
    && punch({ ...base, round: "waiting" }).label === "Writing…"
    && punch({ ...base, round: "ready" }).does === "open" && punch({ ...base, round: "ready" }).label === "New draft ready"
    && punch({ ...base, marks: 1, round: "ready" }).does === "send"
    && punch(base).does === "choose" && punch(base).label === "Punch it up"
    && punch({ ...base, online: false }).does === "none"
    && punch({ ...base, marks: 2, online: false }).does === "none"
    && punch({ ...base, marks: 2, sending: true }).does === "none"
    && punch({ ...base, empty: true }).does === "none";
}

// ---- the red arms: each a plausible wrong rule
const RED = {
  switch: () => isSwitch((text, set, target, ask, id) => {
    const r = M.addMark(text, set, { id, type: "change", text: target.text, at: target.at, note: "", intent: ask });
    return { marks: r.error ? [...set, { id, type: "change", text: target.text, at: target.at, note: "", intent: ask }] : r.marks, said: "Marked" };
  }),
  exact: () => isExact((text, set, target) => {
    if (!target) return null;
    return set.find((m) => { const s = M.locate(text, m); return s != null && s < target.at + target.text.length && s + m.text.length > target.at; }) || null;
  }),
  sent: () => sendsIntent((text, set) => M.sendBody(text, set.map((m) => ({ ...m, intent: undefined })))),
  note: () => emptyNoteGoes((set, id, note) => M.noteMark(set, id, note.trim())),
  punch: () => punches((o) => (o.marks > 0 && o.online && !o.sending ? { label: "Get a new draft", does: "send", hint: "" } : P.punch(o))),
};

const red = process.argv.find((a) => a.startsWith("--red"));
if (red) {
  const which = red.includes("=") ? [red.split("=")[1]] : Object.keys(RED);
  for (const k of which) {
    if (!RED[k]) { console.log(`no red arm "${k}"`); process.exit(2); }
    const passed = RED[k]();
    console.log(`${passed ? "NOT RED" : "RED "} ${k}`);
    if (passed) fails.push(`the red arm "${k}" passed: its check cannot fail`);
  }
} else {
  check(isSwitch(P.toggleAsk), "a button is a switch: on, another way, off");
  check(isExact(P.markOn), "the buttons are about exactly the selected words");
  check(sendsIntent(M.sendBody), "Send carries how he asked");
  check(emptyNoteGoes(P.saveNote), "an emptied note takes a bare mark with it");
  check(punches(P.punch), "the main button: send, wait, open, choose, off");

  // the edges
  const noted = P.saveNote(P.toggleAsk(DOC, [], SEL, "funnier", "m1").marks, "m1", "build to it");
  const off = P.toggleAsk(DOC, noted, SEL, "funnier", "m2");
  check(off.marks.length === 1 && off.marks[0].intent === undefined && off.marks[0].note === "build to it" && off.said === "Funnier off. The note stays", "switching the ask off keeps a mark that has a note");
  check(eq(P.lit(off.marks[0]), { funnier: false, shorter: false, options: false, note: true }), "Note is lit while the mark has one");
  check(P.toggleAsk(DOC, [], { text: "not on the page", at: 0 }, "funnier", "m1").error === "Those words are not in the bit", "words that are not on the page are not marked");
  check(P.askWord({ type: "change", intent: "options" }) === "3 options" && P.askWord({ type: "change" }) === "Note" && P.askWord({ type: "keep" }) === "Keep" && P.askWord({ type: "ask" }) === "Question" && P.askWord({ type: "change", intent: "again" }) === "Another take", "a mark's name");
  check(P.marksLine(2) === "2 marks on this page. Anything you didn't mark stays word for word." && P.marksLine(1).startsWith("1 mark on"), "the sentence over Get a new draft");
  const t = P.punchTargets(DOC, DOC.indexOf("No training"));
  check(t.paragraph?.text.startsWith("I work here now.") && t.paragraph.text.endsWith("who is a screen.") && t.paragraph.at === DOC.indexOf("I work"), "Punch it up, this paragraph: the lines around the caret up to a blank line");
  check(t.whole?.text === DOC && t.whole.at === 0, "Punch it up, the whole page");
  check(P.punchTargets(DOC, null).paragraph?.text.startsWith("Then a real employee"), "no caret: the last paragraph");
  check(P.punchTargets(DOC, 0).paragraph?.text === "The machine says unexpected item like I built it wrong.", "a caret at the very start: the first paragraph");
  check(P.punchTargets("   \n\n  ", 1).paragraph === null && P.punchTargets("", 0).whole === null, "an empty page has nothing to punch up");
  check(P.punchTargets("x".repeat(M.MAX_QUOTE + 1), 0).whole === null, "a page longer than one mark may quote is not offered whole");
  check(P.savedWord({ pending: false, note: null, typed: false, versions: 3 }) === "Saved" && P.savedWord({ pending: true, note: null, typed: false, versions: 3 }) === "Saving…" && P.savedWord({ pending: true, note: null, typed: true, versions: 3 }) === "" && P.savedWord({ pending: false, note: "Offline · kept on this iPhone", typed: false, versions: 3 }) === "" && P.savedWord({ pending: false, note: null, typed: false, versions: 0 }) === "", "the quiet word in the bar");
  check(eq(P.ASKS.map((a) => a.label), ["Funnier", "Shorter", "3 options", "Note"]), "the four buttons, in the mockup's order");
  const raw = JSON.stringify({ v: 1, bit: "joke:a", on: null, at: "t", marks: [{ id: "1", type: "change", text: "x", at: 3, note: "", intent: "shorter" }, { id: "2", type: "change", text: "y", at: 0, note: "", intent: "louder" }] });
  const back = M.parseMarkSet(raw);
  check(back?.marks[0].intent === "shorter" && back.marks[1].intent === undefined, "the marks read back from the phone keep how he asked; an unknown word is dropped");
}

if (fails.length) {
  console.log(`\n${fails.length} FAILED`);
  process.exit(1);
}
console.log("\nall passed");
