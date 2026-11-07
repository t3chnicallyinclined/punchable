// Unit test for rules/scriptRules.ts (the script, SPEC 2026-11-06 §12.4, §12.5): the line assembly a save writes, how
// a line's mark moves when he edits, splits, joins, cuts, moves, pulls a line of his, uses one of Claude's, or Claude
// drafts the rest around his lines (a draft only adds), the local draft that keeps what he typed, and the hook line.
// Every joke here is invented.
//   node --experimental-strip-types --no-warnings tests/scriptRules.test.mjs          the suite
//   node --experimental-strip-types --no-warnings tests/scriptRules.test.mjs --red    the red arm: lenient rules (an
//                                                                                    edited Claude line still badged
//                                                                                    AI, a draft that replaces his
//                                                                                    lines, blank lines saved as lines,
//                                                                                    an older local draft winning)
//                                                                                    must FAIL
import * as R from "../src/rules/scriptRules.ts";

const red = process.argv.includes("--red");
const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const L = red ? {
  ...R,
  edited: (l, text) => ({ ...l, text }),
  editLine: (lines, i, text) => lines.map((l, k) => (k === i ? { ...l, text } : l)),
  mergeDraft: (cur, sent, result) => ({ lines: result.map((l) => ({ key: R.newKey(), text: l.text, by: l.fixed ? "him" : "ai" })), added: result.length }),
  toSave: (lines) => ({ text: lines.map((l) => l.text).join("\n"), marks: lines.map(() => ({})), lines: lines.map((l) => l.text) }),
  pickOpen: (local) => (local ? "local" : "server"),
} : R;

const DOC = [
  { text: "Nobody asked me if I wanted a second job.", by: "him" },
  { text: "I scan, I bag, I check my own ID.", by: "him", from: "joke:free", label: "Free employee v3" },
  { text: "And it still treats me like the new guy.", by: "ai" },
  { text: "Unexpected item in the bagging area.", by: "him", from: "joke:item", label: "Unexpected item v8", punch: true },
];
const lines = () => R.fromDoc(DOC);

// ---- the marks move with the edits
let s = L.editLine(lines(), 2, "And it treats me like the new guy on day one.");
check(s[2].by === "yours" && R.provOf(s[2]).k === "yours", "Claude's line he edits is his: yours, the AI badge gone");
s = L.editLine(lines(), 1, "I scan, I bag, I check my own ID, twice.");
check(s[1].by === "him" && s[1].edited === true && R.provWords(R.provOf(s[1])) === "from Free employee v3 · edited", "a quoted line he changes: from Free employee v3 · edited");
check(L.editLine(lines(), 1, DOC[1].text)[1].edited !== true, "the same words are no edit");
check(R.provWords(R.provOf(DOC[1])) === "from Free employee v3" && R.provOf(DOC[0]).k === "none" && R.provOf({ text: "x" }).k === "none", "his quote shows its source; a typed or unknown line shows nothing");

// ---- Return, Backspace, the next line, cut, move, punch
const sp = R.splitAt(lines(), 0, "Nobody asked me ", "if I wanted a second job.");
check(sp.lines.length === 5 && sp.lines[0].text === "Nobody asked me" && sp.lines[1].text === "if I wanted a second job." && sp.focus === 1 && sp.lines[1].by === "him", "Return in a line: the rest is a new line under it, the caret there");
const ju = R.joinUp([...lines().slice(0, 2), { key: "e", text: " ", by: "him" }], 2);
check(ju && ju.lines.length === 2 && ju.focus === 1, "Backspace on an empty line joins up, the caret to the line above");
check(R.joinUp(lines(), 1) === null && R.joinUp([{ key: "a", text: "" }], 0) === null, "a line with words, or the first line, stays");
const nx = R.addLine(lines(), 3);
check(nx.lines.length === 5 && nx.focus === 4 && nx.lines[4].text === "", "Next line: a new line at the end, keyboard up");
const c = R.cutLine(lines(), 1);
check(c.lines.length === 3 && eq(c.undo(c.lines).map((l) => l.text), DOC.map((l) => l.text)), "Cut, then Undo: the line is back in its place with its mark");
check(R.moveLine(lines(), 0, 1)[1].text === DOC[0].text && R.moveLine(lines(), 0, -1)[0].text === DOC[0].text, "Move down; Move up at the top does nothing");
check(R.togglePunch(lines(), 0)[0].punch === true && R.togglePunch(lines(), 3)[3].punch === false, "Punch on and off");

// ---- Undo touches only what it undoes; empty lines go when he stops writing
const pulled = R.pullLine(lines(), 0, "Pulled.", "joke:x", "X v1");
const afterTyping = R.editLine(pulled.lines, 0, "Typed after the pull.");
const back = R.removeKeys(afterTyping, [pulled.lines[pulled.focus].key]);
check(back.length === 4 && back[0].text === "Typed after the pull.", "Undo of Added removes that line and keeps what he typed since");
const base = lines();
const used = R.applyOption(base, 2, "Claude's new one.");
const reverted = R.restoreLine(R.editLine(used.lines, 0, "Typed after."), base[2]);
check(reverted[2].by === "ai" && reverted[2].text === DOC[2].text && reverted[0].text === "Typed after.", "Undo of Line changed puts that line back, nothing else");
const withEmpty = [...lines(), { key: "e1", text: "  ", by: "him" }, { key: "e2", text: "", by: "him" }];
check(R.dropEmpty(withEmpty).length === 4 && R.dropEmpty(withEmpty, "e2").length === 5, "empty lines go when he stops writing; the one he is moving to stays");

// ---- Material, To the rant, Claude's options
const pl = R.pullLine(lines(), 0, "  It's a bag. In the bagging area. ", "joke:item", "Unexpected item v8");
check(pl.focus === 1 && pl.lines[1].text === "It's a bag. In the bagging area." && pl.lines[1].from === "joke:item" && pl.lines[1].by === "him", "Material +: one line of his after the line he is on, with its source");
check(R.inRant(lines(), "unexpected item in the  bagging area.") && !R.inRant(lines(), "It's a bag."), "a line already there reads \"in the rant\"");
const tr = R.appendMaterial([...lines(), { key: "z", text: "" }], ["One.", "Two."], "joke:x", "X v1");
check(tr.length === 6 && tr[4].from === "joke:x" && tr[5].text === "Two.", "To the rant: all its lines at the end, each with its source");
const u = R.applyOption(lines(), 3, "Unexpected item. It's me. I'm the item.");
check(u.lines.length === 4 && u.lines[3].by === "ai" && u.lines[3].punch === true && u.lines[3].text === "Unexpected item. It's me. I'm the item.", "Use this: Claude's version replaces the line, badged, the punch kept");
check(R.applyOption(lines(), 3, "Tag.", true).lines.length === 5 && R.applyOption(lines(), 3, "Tag.", true).lines[4].by === "ai", "Add it after: under the line, badged");

// ---- Draft the rest only adds; his lines are never replaced
const cur = lines();
const sent = R.keepOf(cur);
check(eq(sent.texts, DOC.map((l) => l.text)), "every line with words goes as fixed");
const merged = L.mergeDraft(cur, sent.keys, [
  { text: "Stop scanning your own groceries.", his: false },
  { text: DOC[0].text, fixed: true },
  { text: DOC[1].text.toUpperCase(), fixed: true },
  { text: "Self-checkout is the store hiring you for free.", his: true, source: "joke:sc", source_label: "Self-checkout v2" },
  { text: DOC[2].text, fixed: true },
  { text: DOC[3].text, fixed: true },
  { text: "I want my fifteen.", his: false },
]);
check(merged && merged.added === 3 && merged.lines.length === 7 && merged.keys.length === 3, "three lines added around his four");
check(merged && R.removeKeys(merged.lines, merged.keys).every((l, i) => l === cur[i]), "Undo of the draft takes out exactly what it added");
check(merged && merged.lines[0].by === "ai" && merged.lines[0].text === "Stop scanning your own groceries.", "a new first line, badged");
check(merged && merged.lines[2] === cur[1], "his line is his own object, never the draft's copy (the case the draft changed is ignored)");
check(merged && merged.lines[3].from === "joke:sc" && merged.lines[3].by === "him", "a quoted line of his from the draft: from Self-checkout v2");
check(merged && merged.lines[6].text === "I want my fifteen.", "the button at the end");
const cutMeanwhile = R.mergeDraft(cur.filter((l) => l.key !== cur[0].key), sent.keys, [{ text: DOC[0].text, fixed: true }, { text: "After the cut one.", his: false }, { text: DOC[1].text, fixed: true }, { text: DOC[2].text, fixed: true }, { text: DOC[3].text, fixed: true }]);
check(cutMeanwhile && cutMeanwhile.lines[cutMeanwhile.lines.length - 1].text === "After the cut one.", "a line he cut while it drafted: what came after it lands at the end");
check(R.mergeDraft(cur, sent.keys, [{ text: DOC[0].text, fixed: true }]) === null, "a result missing his lines is refused again here");

// ---- what a save writes
const sv = L.toSave([...lines().slice(0, 2), { key: "b", text: "   ", by: "him" }, ...lines().slice(2)]);
check(sv.lines.length === 4 && sv.text.split("\n").length === 4, "a blank line is not a line: the text and the marks line up");
check(eq(sv.marks[1], { by: "him", from: "joke:free", label: "Free employee v3" }) && eq(sv.marks[3], { by: "him", from: "joke:item", label: "Unexpected item v8", punch: true }), "each line's mark is saved beside it");
check(eq(R.toSave([{ key: "a", text: "Old line." }]).marks, [{}]), "a line with no record saves none (never a guess)");
check(R.sourceOf(lines()) === "rant_draft" && R.sourceOf(lines().filter((l) => l.by !== "ai")) === "rant_write", "a version with Claude's lines keeps rant_draft; his own is rant_write");
check(!R.changed(lines(), DOC) && R.changed(R.togglePunch(lines(), 0), DOC), "a punch mark is a change; nothing else is");
check(eq(R.newAiLines(R.applyOption(lines(), 0, "New AI.").lines, DOC), ["New AI."]), "the learning loop hears only Claude's new lines");
const carried = R.carryMarks(lines(), ["Hook from Claude.", ...DOC.map((l) => l.text)], "ai");
check(carried[0].by === "ai" && carried[2].from === "joke:free" && carried[4].punch === true, "Pick a hook's version: the new first line badged, his marks carried");

// ---- the local draft
const local = R.localDraft("topic:a", "joke:r", "unit:3", R.editLine(lines(), 0, "Changed offline."), true);
check(L.pickOpen(local, "joke:r", "unit:3", DOC) === "local", "reopened with the same version on the server: his unsaved words come back");
check(L.pickOpen(local, "joke:r", "unit:4", DOC) === "server", "a newer version saved since wins over an older local draft");
check(L.pickOpen({ ...local, pending: false }, "joke:r", "unit:3", DOC) === "server", "a draft already saved is not restored");
check(R.pickOpen(R.localDraft("topic:a", null, null, R.fromDoc([{ text: "First line typed offline.", by: "him" }]), true), null, null, null) === "local", "a first rant typed offline comes back");
check(eq(R.parseLocal(JSON.stringify(local)), local) && R.parseLocal("{") === null && R.parseLocal(JSON.stringify({ v: 2 })) === null, "a stored draft reads back; anything else is none");
check(R.draftFile("topic:ab/../c") === "topic_ab____c.json", "a draft's file name is the id's safe characters");

// ---- the hook line and VoiceOver
check(R.hookLine("Hook lands at 2.1 s", ["one two three four five six seven eight nine ten", "eleven twelve"]) === "Hook lands at 2.1 s · runs 0:05", "Hook lands at 2.1 s · runs 0:05");
check(R.lineA11y(DOC[3], 3, 4) === "Line 4 of 4, punch. From Unexpected item v8. Unexpected item in the bagging area.", "VoiceOver: place, punch, source, words");

if (fails.length) {
  console.log(`\n${fails.length} FAILED${red ? " (the red arm: lenient rules must fail)" : ""}`);
  process.exit(red ? 0 : 1);
}
if (red) { console.log("\nthe red arm PASSED with lenient rules: the suite cannot fail"); process.exit(1); }
console.log("\nall passed");
