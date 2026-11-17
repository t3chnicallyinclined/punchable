// Unit test for src/rules/setNoteRules.ts (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §3.3, Slice 4): the bullet
// notes editor under a bit of a set, the save and its Undo, the seam between two bits, a Sets card's words.
//   node --experimental-strip-types --no-warnings tests/setNoteRules.test.mjs
//   node --experimental-strip-types --no-warnings tests/setNoteRules.test.mjs --red    the red arm: a planted rule that
//       opens the editor on whatever the server sent (so the words the app derived would be saved as notes of his)
import * as N from "../src/rules/setNoteRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const texts = (ds) => ds.map((d) => d.text);

const BOARD = "sb:friday-1a2b3c4d";
const BIT = "joke:fridge";
const his = { notes: ["open on the beep", "slow down"], notes_from: "his" };
const derived = { notes: ["My fridge has wifi now.", "It tells me the door is open."], notes_from: "derived" };

/** The rule under test: nothing the app derived is ever stored as a note of his. Opening the editor on a bit he has
 *  written nothing under and leaving again must plan no save; and typing one note must save that note alone. */
function derivedWordsAreNeverSaved(editorNotes) {
  const opened = editorNotes(derived);
  const untouched = N.planSave(BOARD, BIT, N.hisNotes(derived), N.onLeave(opened));
  const one = N.addNote(opened);
  const typed = N.onType(one.drafts, one.focus, "start quiet");
  const save = N.planSave(BOARD, BIT, N.hisNotes(derived), N.onLeave(typed));
  return untouched === null && !!save && eq(save.body, { joke_id: BIT, notes: ["start quiet"] });
}

if (process.argv.includes("--red")) {
  // a planted rule: the editor opens on the served list whoever it is from
  let k = 0;
  const lenient = (step) => (step.notes || []).map((text) => ({ key: `r${++k}`, text }));
  const caught = !derivedWordsAreNeverSaved(lenient);
  console.log(`${caught ? "PASS" : "FAIL"} red arm: an editor that opens on the derived words is caught (they would be saved as his)`);
  console.log(`${derivedWordsAreNeverSaved(N.editorNotes) ? "PASS" : "FAIL"} the real editorNotes passes the same check`);
  process.exit(caught && derivedWordsAreNeverSaved(N.editorNotes) ? 0 : 1);
}

// ---- whose notes
check(eq(N.hisNotes(his), his.notes) && eq(N.hisNotes(derived), []) && eq(N.hisNotes(null), []) && eq(N.hisNotes({}), []), "his notes are his only when the server says so");
check(eq(N.derivedNotes(derived), derived.notes) && eq(N.derivedNotes(his), []) && eq(N.derivedNotes({ notes: [" a ", ""], notes_from: null }), ["a"]), "the derived words show only while he has none of his own");
check(eq(texts(N.editorNotes(his)), his.notes) && eq(N.editorNotes(derived), []), "the editor opens on his notes, never on the derived ones");
check(derivedWordsAreNeverSaved(N.editorNotes), "derived words are never saved: opening and leaving plans nothing; one typed note saves that note alone");
const keys = N.editorNotes(his).map((d) => d.key);
check(new Set(keys).size === keys.length && keys.every((k) => typeof k === "string" && k), "every bullet has its own key");

// ---- cleaning
check(N.cleanNote("  slow\tdown\nhere \u0007") === "slow down here" && N.cleanNote(" \n ") === "" && N.cleanNote(null) === "", "a note is one line: control characters become spaces, the ends are trimmed");
check(N.cleanNote("one\u2028two\u2029three") === "one two three" && N.cleanNote("\u202edrawkcab\u202c") === "drawkcab" && N.cleanNote("ok \u{1F468}\u200d\u{1F469}") === "ok \u{1F468}\u200d\u{1F469}", "line separators and direction overrides go too (the server's own rule); an emoji's joiner stays");
check(Array.from(N.cleanNote("é".repeat(250))).length === N.MAX_NOTE_CHARS && N.MAX_NOTE_CHARS === 200, "a note is cut at 200 characters (code points)");
check(eq(N.cleanNotes([" a ", "", "  ", "b"]), ["a", "b"]), "blank bullets are no notes");
check(N.cleanNotes(Array.from({ length: 20 }, (_, i) => `n${i}`)).length === N.MAX_NOTES && N.MAX_NOTES === 12, "at most twelve notes are sent");

// ---- Return
const two = N.editorNotes(his);
const r = N.onReturn(two, two[0].key);
check(eq(texts(r.drafts), ["open on the beep", "", "slow down"]) && r.focus === r.drafts[1].key, "Return adds an empty bullet under the one he is in and moves the cursor there");
const r2 = N.onReturn(r.drafts, r.drafts[1].key);
check(eq(texts(r2.drafts), texts(r.drafts)) && r2.focus === null && r2.done === true, "Return on an empty bullet adds nothing and ends the writing");
check(!r.done && !N.onReturn(two, "nope").done, "Return in a bullet with words does not end the writing");
check(N.onReturn(two, "nope").focus === null && N.onReturn(two, "nope").drafts.length === 2, "an unknown bullet changes nothing");
const twelve = Array.from({ length: 12 }, (_, i) => ({ key: `k${i}`, text: `note ${i}` }));
check(N.onReturn(twelve, "k3").drafts.length === 12 && N.onReturn(twelve, "k3").focus === null && !N.canAddNote(twelve) && N.canAddNote(two), "at twelve Return adds nothing");
check(r.drafts[0].key === two[0].key && r.drafts[2].key === two[1].key, "the bullets that were there keep their keys");

// ---- Backspace
const b = N.onBackspace(r.drafts, r.drafts[1].key);
check(eq(texts(b.drafts), his.notes) && b.focus === two[0].key, "Backspace on an empty bullet removes it and the cursor goes to the one above");
const first = [{ key: "a", text: "" }, { key: "b", text: "kept" }];
check(eq(N.onBackspace(first, "a"), { drafts: [{ key: "b", text: "kept" }], focus: "b" }), "the first one gone: the cursor goes to the next");
check(eq(N.onBackspace([{ key: "a", text: "" }], "a"), { drafts: [], focus: null }), "the only one gone: nothing left to focus");
check(eq(texts(N.onBackspace(two, two[0].key).drafts), his.notes) && N.onBackspace(two, two[0].key).focus === null, "Backspace in a bullet with words removes nothing");
check(N.onBackspace([{ key: "a", text: " " }], "a").drafts.length === 1, "a bullet holding a space is not empty yet (Backspace deletes the space first)");

// ---- Add a note
const a1 = N.addNote([]);
check(eq(texts(a1.drafts), [""]) && a1.focus === a1.drafts[0].key, "Add a note on a bit with none: one empty bullet, focused");
const a2 = N.addNote(a1.drafts);
check(a2.drafts.length === 1 && a2.focus === a1.drafts[0].key, "Add a note twice does not pile up blanks: the cursor goes to the empty one");
const a3 = N.addNote(two);
check(eq(texts(a3.drafts), [...his.notes, ""]) && a3.focus === a3.drafts[2].key, "Add a note after his notes");
check(N.addNote(twelve).drafts.length === 12 && N.addNote(twelve).focus === "k11", "at twelve the last bullet takes the cursor");

// ---- typing and leaving
const t1 = N.onType(a3.drafts, a3.focus, "look left");
check(eq(texts(t1), [...his.notes, "look left"]) && eq(texts(N.onType(t1, "nope", "x")), texts(t1)), "typing changes the one bullet");
check(eq(texts(N.onLeave([{ key: "a", text: "" }, { key: "b", text: " x " }, { key: "c", text: "  " }])), [" x "]), "leaving drops the blank bullets and keeps his typing as typed");

// ---- the save and its Undo
check(N.planSave(BOARD, BIT, his.notes, two) === null, "nothing changed: no save");
check(N.planSave(BOARD, BIT, his.notes, [...two, { key: "z", text: "  " }]) === null, "a blank bullet is not a change");
const save = N.planSave(BOARD, BIT, his.notes, t1);
check(eq(save, { method: "POST", path: "/api/boards/sb%3Afriday-1a2b3c4d/notes", body: { joke_id: BIT, notes: ["open on the beep", "slow down", "look left"] } }), "one call with the whole list for that bit");
check(eq(N.planSave(BOARD, BIT, his.notes, []).body, { joke_id: BIT, notes: [] }), "clearing every note is a save too");
check(N.planSave(BOARD, BIT, [], []) === null, "no notes and none typed: no save");
check(eq(N.undoSave(BOARD, BIT, { notes: ["a"], was: ["a", "b"] }), { method: "POST", path: "/api/boards/sb%3Afriday-1a2b3c4d/notes", body: { joke_id: BIT, notes: ["a", "b"] } }), "Undo sends the list it replaced");
check(N.undoSave(BOARD, BIT, { notes: ["a"], was: ["a"] }) === null && N.undoSave(BOARD, BIT, { notes: ["a"] }) === null && N.undoSave(BOARD, BIT, null) === null, "nothing replaced, or no answer: no Undo");

// ---- the snack
check(N.saveSnack([], ["a"]) === "" && N.saveSnack(["a"], ["a", "b"]) === "" && N.saveSnack(["a", "c"], ["a", "b", "c"]) === "", "adding notes is quiet");
check(N.saveSnack(["a", "b"], ["a"]) === "Note removed" && N.saveSnack(["a", "b", "c"], ["b"]) === "2 notes removed" && N.saveSnack(["a"], []) === "Note removed", "a note that went says so");
check(N.saveSnack(["a", "b"], ["a", "c"]) === "Notes changed" && N.saveSnack(["a", "b"], ["b", "a"]) === "Notes changed", "a changed or moved note says so");
check(N.saveWords([], ["a"]) === "Note added" && N.saveWords(["a"], ["a", "b", "c"]) === "2 notes added" && N.saveWords(["a", "b"], ["a"]) === "Note removed" && N.saveWords(["a"], ["b"]) === "Notes changed", "every save has words for the menu's Undo, a quiet add too");
check(N.noteLabel(0, 3, "Smart fridge") === "Note 1 of 3 for Smart fridge", "VoiceOver names each bullet");

// ---- the seam
const tin = (extra) => ({ words: "No link yet", weak: true, kind: "jump", from: "joke:a", to: "joke:b", ...extra });
check(N.seamOf(null) === null && N.seamOf(undefined) === null, "the first bit has no seam");
const none = N.seamOf(tin({}));
check(none.tone === "none" && none.mark === "none" && none.label === "No bridge yet" && none.detail === "Nothing ties these two together yet." && none.bridge && !none.ai && none.key === "joke:a>joke:b", "nothing ties them: No bridge yet, a bridge to draft");
const world = N.seamOf(tin({ words: "Same world: Cars, Mom, needs a line between", kind: "link" }));
check(world.label === "No bridge yet" && world.detail === "Same world: Cars, Mom, needs a line between." && world.bridge, "a shared world with no line between is still a bridge to draft, and says what they share");
const proven = N.seamOf(tin({ words: "Done back to back on stage", weak: false, kind: "proven" }));
check(proven.tone === "good" && proven.mark === "check" && proven.label === "Usual order" && proven.detail === "You have done these two back to back on stage." && !proven.bridge, "the usual order");
const cb = N.seamOf(tin({ words: "Callback", weak: false, kind: "link" }));
check(cb.tone === "good" && cb.label === "Callback" && cb.detail === "One of these calls back to the other." && !cb.ai, "a callback");
const cbHis = N.seamOf(tin({ words: "Callback", weak: false, kind: "link", source: "his" }));
const cbAi = N.seamOf(tin({ words: "Callback", weak: false, kind: "link", source: "ai" }));
check(cbHis.tone === "good" && cbHis.mark === "check" && !cbHis.ai, "a callback he made or confirmed gets the check");
check(cbAi.tone === "none" && cbAi.mark === "none" && cbAi.ai && cbAi.label === "Callback" && !cbAi.bridge, "a callback that is only the app's reading keeps its name, carries the AI mark and gets no check");
check(N.seamOf(tin({ words: "Done back to back on stage", weak: false, kind: "proven", source: "ai" })).ai === false, "what he did on stage is never the AI's");
const lead = N.seamOf(tin({ words: "Could lead in", weak: false, kind: "link" }));
check(lead.ai && lead.label === "Could lead in" && lead.tone === "none" && lead.mark === "none" && N.seamOf(tin({ words: "Your segue", weak: false, kind: "link" })).ai === false, "a suggested segue carries the AI mark and no check; his own does not");
const bare = N.seamOf(tin({ words: "", weak: false, kind: "link" }));
check(bare.label === "Linked" && bare.detail === "These two are linked in your notebook.", "a link with no words is still named and still says something when opened");
const shape = N.seamOf(tin({ words: "Callback", weak: false, kind: "link", same_shape: { source: "ai", shared: ["Misdirect", "List of three"], edge: null } }));
check(shape.tone === "warn" && shape.mark === "warn" && shape.label === "Same shape twice" && shape.ai && !shape.bridge
  && shape.detail === "Both use Misdirect, List of three. Back to back, the second one usually gets a smaller laugh.", "same shape twice wins the label and says which moves");
const shapeHis = N.seamOf(tin({ same_shape: { source: "his", shared: [], edge: "e:1" } }));
check(!shapeHis.ai && shapeHis.bridge && shapeHis.detail.startsWith("You said these two have the same shape."), "his own same-shape tie is his; a weak seam under it still offers a bridge");
check(N.seamOf(tin({ same_shape: { source: "ai", shared: [], edge: "e:2" } })).detail.startsWith("These two turn the same way."), "a same-shape edge with no named moves");
check(N.seamOf(tin({ same_shape: null })).label === "No bridge yet", "no same-shape tie: the seam is what it was");
for (const s of [none, world, proven, cb, cbAi, lead, shape, shapeHis]) check(s.mark === (s.tone === "good" ? "check" : s.tone), `tone and mark agree, so colour is never the only sign: ${s.label}`);
for (const s of [cbAi, lead, shape]) check(s.ai && s.mark !== "check", `the app's reading never gets the check: ${s.label}`);
check(N.seamLabel(shape, "Smart fridge", "GPS") === "Between Smart fridge and GPS: Same shape twice, suggested by AI" && N.seamLabel(proven, "A", "B") === "Between A and B: Usual order", "VoiceOver names the seam, and says when it is the AI's reading");

// ---- a Sets card
check(N.cardMeta(4, "6:10") === "4 bits · 6:10" && N.cardMeta(1, "1:30") === "1 bit · 1:30" && N.cardMeta(0, "0:00") === "Empty", "a card says how many bits and how long");
check(N.cardNames(["Roommate", " Smart fridge ", "", "GPS"]) === "Roommate · Smart fridge · GPS" && N.cardNames([]) === "" && N.cardNames(null) === "", "the bits' names in order on one run");
check(N.cardFoot(true, "7:00", "Nov 16") === "Tonight · 7:00 slot · Edited Nov 16" && N.cardFoot(false, null, "Nov 16") === "Edited Nov 16" && N.cardFoot(false, null, null) === "", "the card's last line says the date is an edit, not a show");

// ---- Undo last change
check(N.undoLabel("Note removed") === "Undo: Note removed" && N.undoLabel("") === "Undo last change" && N.undoLabel(null) === "Undo last change", "the menu's Undo names the change it takes back");

if (fails.length) { console.log(`\n${fails.length} FAILED`); process.exit(1); }
console.log("\nall setNoteRules checks passed");
