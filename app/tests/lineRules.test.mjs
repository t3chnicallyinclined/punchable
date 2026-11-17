// Unit test for rules/lineRules.ts (a bit is a document: SPEC 2026-11-16 B11, §4.3, Slice 2). Every line is invented.
//   node --experimental-strip-types --no-warnings tests/lineRules.test.mjs               the suite
//   node --experimental-strip-types --no-warnings tests/lineRules.test.mjs --red         every red arm
//   node --experimental-strip-types --no-warnings tests/lineRules.test.mjs --red=<arm>   one of: save, ideas, edit, draft, caret
// A red arm plants a plausible wrong rule and must FAIL the same check the real rule passes:
//   save    a rule that saves whenever the strings differ (a trailing space or an empty doc would become a version)
//   ideas   a rule that drops blank lines (the ideas of a bit run together after a reorder)
//   edit    a rule that puts a line in by joining the lines with words (every gap between ideas is lost)
//   draft   a rule that opens on the saved version whenever there is one (words typed offline vanish on reopen)
//   caret   a rule that always scrolls the caret to the keyboard's edge (the page jumps under his thumb as he types)
import * as L from "../src/rules/lineRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const DOC = [
  "My fridge has wifi now.",
  "It tells me when the door is open.",
  "",
  "I know the door is open. I am standing in it.",
  "",
  "",
  "That is not a smart fridge, that is a roommate.",
].join("\n");

// ---- the checks, each taking the rule under test so a red arm can hand in a wrong one

/** A save is a new version only when the words changed; never for an empty document; never above the limit. */
function savesOnlyWords(saveDecision) {
  return saveDecision(DOC, DOC) === "none"
    && saveDecision(DOC, DOC + "  \n\n") === "none"
    && saveDecision(DOC, DOC.replace("now.", "now.   ")) === "none"
    && saveDecision(DOC, DOC.replace(/\n/g, "\r\n")) === "none"
    && saveDecision(DOC, "   \n \n") === "empty"
    && saveDecision("", "") === "none"
    && saveDecision(null, "A first line.") === "version"
    && saveDecision(DOC, DOC + "\nIt eats my food.") === "version"
    && saveDecision(DOC, DOC.replace("wifi", "bluetooth")) === "version"
    && saveDecision(DOC, DOC.replace("open.\n\nI know", "open.\nI know")) === "version"
    && saveDecision(DOC, "x".repeat(L.MAX_DOC + 1)) === "too_long"
    && saveDecision(DOC, "x".repeat(L.MAX_DOC)) === "version";
}
/** A blank line separates ideas, a run of them is one gap, and rows -> text -> rows is the same document. */
function keepsIdeas(rowsOf, textOf) {
  const rows = rowsOf(DOC);
  return eq(rows.map((r) => r.kind), ["line", "line", "break", "line", "break", "line"])
    && textOf(rows) === DOC.replace("\n\n\n", "\n\n")
    && eq(rowsOf(textOf(rows)), rows)
    && eq(rowsOf("\n\n  \nOne.\n\n"), [{ kind: "line", text: "One." }])
    && textOf([{ kind: "break" }, { kind: "line", text: "One." }, { kind: "break" }, { kind: "break" }, { kind: "line", text: "Two." }, { kind: "break" }]) === "One.\n\nTwo.";
}
/** A line put in from a card or the pile lands after its line and every gap between ideas stays where it was. */
function editKeepsGaps(applyEdit) {
  const put = applyEdit(DOC, { op: "insert", at: 1, words: "It has opinions." });
  const end = applyEdit(DOC, { op: "insert", at: null, words: "Rent is due." });
  const swap = applyEdit(DOC, { op: "replace", at: 2, words: "I know. I am IN it." });
  return put === DOC.replace("is open.\n\nI know", "is open.\nIt has opinions.\n\nI know")
    && end === DOC + "\nRent is due."
    && swap === DOC.replace("I know the door is open. I am standing in it.", "I know. I am IN it.")
    && L.ideasOf(put).length === 3 && L.ideasOf(swap).length === 3;
}
/** The page opens on his draft when it holds words no version has; otherwise on the saved version. */
function opensOnHisWords(openWith) {
  const d = L.bitDraft("joke:fridge", "stage", "unit:f3", DOC + "\nTyped on the train.", "2026-11-16T10:00:00Z");
  const same = L.bitDraft("joke:fridge", "stage", "unit:f3", DOC + "  \n", "2026-11-16T10:00:00Z");
  const empty = L.bitDraft("joke:fridge", "stage", "unit:f3", "  \n", "2026-11-16T10:00:00Z");
  const a = openWith(DOC, d), b = openWith(DOC, same), c = openWith(DOC, null), e = openWith(DOC, empty);
  // a save reached the server but its answer was lost, then he wrote more on another device: the draft is an OLD version
  const old = openWith(DOC + "\nTyped on the train.\nAnd more, on the laptop.", d, [DOC + "\nTyped on the train.", DOC]);
  return a.pending && a.text.endsWith("Typed on the train.")
    && !b.pending && b.text === DOC
    && !c.pending && c.text === DOC
    && !e.pending && e.text === DOC
    && !old.pending && old.text.endsWith("on the laptop.");
}
/** The caret is brought above the keyboard when it is under it, back under the bar when above it, and the page does
 *  not move at all while the caret is in view. */
function caretStaysPut(caretScroll) {
  const at = (top, offset) => caretScroll({ top, bottom: top + 26, offset, height: 400, guardTop: 100, margin: 24 });
  return at(300, 0) === null                    // in view: no jump
    && at(110, 0) === null && at(349, 0) === null
    && at(500, 0) === 150                       // under the keyboard: its line sits `margin` above the edge
    && at(40, 200) === -60                      // above the bar: back under it
    && caretScroll({ top: 0, bottom: 26, offset: 0, height: 0, guardTop: 100, margin: 24 }) === null;
}

const ARMS = {
  save: () => [!savesOnlyWords((saved, typed) => (saved === typed ? "none" : "version")), savesOnlyWords(L.saveDecision), "a rule that saves whenever the strings differ is caught (a trailing space, an empty document)"],
  ideas: () => [!keepsIdeas((t) => t.split("\n").filter((l) => l.trim()).map((text) => ({ kind: "line", text })), L.textOf), keepsIdeas(L.rowsOf, L.textOf), "a rule that drops blank lines is caught (the ideas run together)"],
  edit: () => [!editKeepsGaps((text, e) => { const ls = text.split("\n").filter((l) => l.trim()); const k = e.at == null ? ls.length : e.at + 1; return e.op === "insert" ? [...ls.slice(0, k), e.words, ...ls.slice(k)].join("\n") : ls.map((l, i) => (i === e.at ? e.words : l)).join("\n"); }), editKeepsGaps(L.applyEdit), "a rule that joins the lines with words is caught (the gaps between ideas are lost)"],
  draft: () => [!opensOnHisWords((saved) => ({ text: saved || "", pending: false })), opensOnHisWords(L.openWith), "a rule that always opens on the saved version is caught (words typed offline vanish)"],
  caret: () => [!caretStaysPut((x) => Math.round(x.bottom - x.height + x.margin)), caretStaysPut(L.caretScroll), "a rule that always scrolls to the caret is caught (the page jumps while he types)"],
};

const red = process.argv.find((a) => a === "--red" || a.startsWith("--red="));
if (red) {
  const names = red === "--red" ? Object.keys(ARMS) : [red.slice(6)];
  let ok = true;
  for (const n of names) {
    if (!ARMS[n]) { console.log(`FAIL no red arm named ${n}`); ok = false; continue; }
    const [caught, real, what] = ARMS[n]();
    console.log(`${caught ? "PASS" : "FAIL"} red arm ${n}: ${what}`);
    console.log(`${real ? "PASS" : "FAIL"} the real rule passes the same check (${n})`);
    ok = ok && caught && real;
  }
  process.exit(ok ? 0 : 1);
}

// ---- what a save is
check(savesOnlyWords(L.saveDecision), "a save is a new version only when the words changed: not for trailing spaces, blank ends, line-end style or an empty document; a removed gap between ideas IS a change");
check(L.docText("\n\n  My fridge.  \r\nIt has wifi.\n\n\n") === "  My fridge.  \nIt has wifi." && L.docText(null) === "", "the text sent keeps his spacing and blank lines; only the blank ends and the line-end style go");
check(L.sameDoc("a \nb", "a\nb\n") && !L.sameDoc("a\nb", "a\n\nb") && !L.sameDoc("a b", "a  b"), "two texts are the same when only trailing space differs; a blank line or a space inside a line is a difference");
check(L.MAX_DOC === 40000 && L.tooLongNote("x".repeat(40120)) === "Too long to save · 40,120 of 40,000", "the one reason a long document is not saved says both numbers (the limit is the server’s: 40,000 characters)");

// ---- structure read out of the text
check(keepsIdeas(L.rowsOf, L.textOf), "a blank line separates ideas, a run of blank lines is one gap, and rows -> text -> rows is the same document");
check(eq(L.ideasOf(DOC), [["My fridge has wifi now.", "It tells me when the door is open."], ["I know the door is open. I am standing in it."], ["That is not a smart fridge, that is a roommate."]]), "a bit's ideas are its runs of lines between blank lines");
check(eq(L.ideasOf(""), []) && eq(L.ideasOf("One.\nTwo."), [["One.", "Two."]]), "no text is no idea; no blank line is one idea");
check(L.docMeta(DOC) === "3 ideas · 4 lines" && L.docMeta("One.") === "1 line" && L.docMeta("One.\nTwo.") === "2 lines", "the meta counts ideas only when there is more than one");
const rows = L.rowsOf(DOC);
check(L.textOf(L.moveRow(rows, 5, 0)) === "That is not a smart fridge, that is a roommate.\nMy fridge has wifi now.\nIt tells me when the door is open.\n\nI know the door is open. I am standing in it.", "Reorder: the last line moved to the top joins the first idea; the gap it left at the end is dropped");
check(L.textOf(L.moveRow(rows, 2, 1)) === "My fridge has wifi now.\n\nIt tells me when the door is open.\nI know the door is open. I am standing in it.\n\nThat is not a smart fridge, that is a roommate.", "Reorder: a gap moved up splits the idea there");
check(eq(L.moveRow(rows, 1, 1), rows) && eq(L.moveRow(rows, -1, 2), rows) && eq(L.moveRow(rows, 0, 99), rows) && L.moveRow(rows, 0, 0) !== rows, "a move that goes nowhere changes nothing (and never hands back the same array)");
check(L.moveRow(rows, 0, 3).filter((r) => r.kind === "line").map((r) => r.text).sort().join("|") === rows.filter((r) => r.kind === "line").map((r) => r.text).sort().join("|"), "a reorder keeps every line's words");

// ---- the caret's line, and an edit that keeps the gaps
check(L.caretLine(DOC, 0) === 0 && L.caretLine(DOC, 23) === 0 && L.caretLine(DOC, 24) === 1, "the caret's line counts lines with words; the end of a line is still that line");
check(L.caretLine(DOC, DOC.indexOf("\n\nI know") + 1) === 1 && L.caretLine(DOC, DOC.length) === 3 && L.caretLine("\n\nOne.", 0) === null && L.caretLine("", 0) === null, "on a blank line it is the line above; with no line above there is none");
check(editKeepsGaps(L.applyEdit), "a line put in after line 2 stays in the first idea, one at the end goes last, a swapped wording replaces exactly its line; three ideas stay three");
check(L.applyEdit(DOC, { op: "insert", at: -1, words: "First." }).startsWith("First.\nMy fridge") && L.applyEdit(DOC, { op: "insert", at: 99, words: "Last." }).endsWith("roommate.\nLast."), "an insert before the first line or past the last lands at that end");
check(L.applyEdit(DOC, { op: "replace", at: 99, words: "Gone line." }).endsWith("roommate.\nGone line.") && L.applyEdit(DOC, { op: "insert", at: 0, words: "  \n " }) === DOC, "a replace of a line that is not there adds at the end; no words is no change");
check(L.applyEdit("", { op: "insert", at: null, words: "One.\n\nTwo." }) === "One.\nTwo." && L.applyEdit(DOC, { op: "insert", at: 0, words: "A.\nB." }).startsWith("My fridge has wifi now.\nA.\nB.\nIt tells"), "words with lines of their own go in as lines");

// ---- a place in the document, found by its words (what a card is about today; what a mark hangs on in Slice 3)
const anchors = L.anchorsOf(DOC);
check(anchors.length === 4 && anchors.map((a) => a.idea).join("") === "0012" && anchors.every((a) => DOC.slice(a.start, a.end) === a.text), "every line with words is a place: its idea, and exactly where its words sit in the text");
check(L.findAnchor(DOC, "I know the door is open. I am standing in it.", 2)?.line === 2 && L.findAnchor("A new first line.\n" + DOC, "I know the door is open. I am standing in it.", 2)?.line === 3, "a place is found by the line's words: after a line is added above it, it is still found");
check(L.findAnchor(DOC, "A line he rewrote.", 2) === null && L.findAnchor(DOC, "   ", 0) === null, "a line he rewrote is no place any more (never the line that happens to sit at that number)");
check(L.findAnchor("Same.\nOther.\nSame.", "Same.", 2)?.line === 2 && L.findAnchor("Same.\nOther.\nSame.", "Same.", null)?.line === 0 && L.findAnchor("Other.\nSame.\nx\ny\nSame.", "Same.", 3)?.line === 4, "when two lines say the same, the hinted one, else the nearest to the hint, else the first");
check(L.findAnchor("  Indented line.  \nNext.", "Indented line.", 0)?.start === 2 && L.anchorsOf("").length === 0 && L.anchorsOf("\n\nOne.\n\n\nTwo.").map((a) => a.idea).join("") === "01", "leading space is not part of the place; blank lines before the first line start no idea");

// ---- the local draft
check(opensOnHisWords(L.openWith), "the page opens on his draft when it holds words no version has; a draft that says the same, an empty one or none opens the saved version");
const d = L.bitDraft("joke:fridge", "rant", null, "A line.\r\nAnother.", "2026-11-16T10:00:00Z");
check(eq(L.parseBitDraft(JSON.stringify(d)), { v: 1, bit: "joke:fridge", cut: "rant", base: null, text: "A line.\nAnother.", at: "2026-11-16T10:00:00Z" }), "a draft reads back as written, line ends plain");
check(L.parseBitDraft('{"v":1,"bit":"joke:x","cut":"stage","text":') === null && L.parseBitDraft("") === null && L.parseBitDraft('{"v":2,"bit":"joke:x","cut":"stage","text":"a"}') === null && L.parseBitDraft('{"v":1,"bit":"joke:x","cut":"web","text":"a"}') === null && L.parseBitDraft('{"v":1,"bit":"","cut":"stage","text":"a"}') === null, "a half-written file, another format, an unknown cut or no bit is no draft");
check(L.draftName("joke:fridge", "stage") === "joke:fridge" && L.draftName("joke:fridge", "rant") === "joke:fridge~rant", "one draft per bit and cut");
const dr = (text) => L.bitDraft("joke:fridge", "stage", "unit:f3", text, "");
check(L.flushPlan(dr(DOC + "\nTyped offline."), DOC) === "save" && L.flushPlan(dr(DOC + " "), DOC) === "drop" && L.flushPlan(dr(""), DOC) === "drop" && L.flushPlan(dr("x".repeat(L.MAX_DOC + 1)), DOC) === "keep", "a draft found at launch is saved when it says something new, dropped when the bit says it or it is empty, kept when too long");
check(L.flushPlan(dr("Typed offline."), "The bit moved on elsewhere.") === "save", "his draft is saved even when the bit moved on elsewhere: it becomes the newest version, nothing is dropped");
check(L.flushPlan(dr("An older wording."), "A newer wording, from the laptop.", ["A newer wording, from the laptop.", "An older wording. "]) === "drop", "a draft that an OLDER version already says is dropped: its save landed and only the answer was lost");
check(L.pauseSaves(1000, null) && !L.pauseSaves(1000 + L.PAUSE_GAP_MS - 1, 1000) && L.pauseSaves(1000 + L.PAUSE_GAP_MS, 1000) && L.PAUSE_GAP_MS > L.PAUSE_MS, "a pause saves a version at most once per gap; the first pause always may");
check(L.mirrorText("My fridge has wifi now.", 5) === "My fridge" && L.mirrorText("My fridge has wifi now.", 9) === "My fridge" && L.mirrorText("My fridge\nhas", 10) === "My fridge\nhas" && L.mirrorText("abc", 99) === "abc" && L.mirrorText("", 0) === "", "the caret is measured to the end of its word (a word wraps whole); at a word's end or past the text nothing is added");

// ---- the caret stays above the keyboard
check(caretStaysPut(L.caretScroll), "the caret is brought above the keyboard or back under the bar, and the page does not move while it is in view");
check(L.PAUSE_MS >= 10000 && L.DRAFT_SETTLE_MS <= 1000, "the local draft settles in under a second; a pause that saves a version is a real pause");

if (fails.length) { console.log(`\n${fails.length} FAILED`); process.exit(1); }
console.log("\nall lineRules checks passed");
