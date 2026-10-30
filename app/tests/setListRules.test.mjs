// Unit test for src/rules/setListRules.ts (docs/design/SPEC-2026-10-29-set-lists-and-stage-notes.md §4.1): his vs
// drafts vs archived, the drafts row's words, the "To a set" target, the picker's rows, the notes sheet's fallbacks.
//   node --experimental-strip-types --no-warnings tests/setListRules.test.mjs
import { addTarget, draftsLabel, notesFor, notesFooter, pickerLists, seedWord, splitLists, stageRowLabel } from "../src/rules/setListRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

const ins = (id, extra = {}) => ({ id, title: id, source: "inferred", by: "ai:seed", seed: { from: "insights.chunks", chunk: 3, minutes_est: 7 }, archived: false, ...extra });
const his = (id, extra = {}) => ({ id, title: id, source: "his", by: "owner", seed: null, archived: false, ...extra });

// splitLists: archived wins, then inferred, then his; a missing source is his
const all = [his("a"), ins("d1"), his("b", { archived: true }), ins("d2", { archived: true }), { id: "c" }, his("e", { source: undefined })];
const sp = splitLists(all);
check(sp.archived.map((x) => x.id).join() === "b,d2", "archived first, whatever the source");
check(sp.drafts.map((x) => x.id).join() === "d1", "inferred (and not archived) is a draft");
check(sp.his.map((x) => x.id).join() === "a,c,e", "his, and a list with no source is his (his wins when unsure)");
check(splitLists([]).his.length === 0 && splitLists([]).drafts.length === 0, "empty in, empty out");

// draftsLabel
const fifteen = Array.from({ length: 15 }, (_, i) => ins(`d${i}`));
check(draftsLabel(fifteen) === "15 drafts from your Insights", `15 same-seed drafts: "${draftsLabel(fifteen)}"`);
check(draftsLabel([ins("x")]) === "1 draft from your Insights", `one draft: "${draftsLabel([ins("x")])}"`);
check(draftsLabel([ins("x"), ins("y", { seed: null })]) === "2 drafts the app made", "mixed seeds: the app made");
check(draftsLabel([ins("x", { seed: null }), ins("y", { seed: {} })]) === "2 drafts the app made", "no seed at all: the app made");
check(draftsLabel([ins("x", { seed: { from: "notes" } }), ins("y", { seed: { from: "Notes.v2" } })]) === "2 drafts from your Notes", "the seed word is the first word, capitalised");
check(draftsLabel([ins("demo", { by: "ai:demo", seed: null })]) === "A sample list", "the demo list alone is a sample list");
check(draftsLabel([ins("demo", { by: "ai:demo", seed: null }), ins("x")]) === "2 drafts the app made", "the demo list among others counts as a draft");
check(draftsLabel([]) === "", "no drafts: no label");
for (const l of [draftsLabel(fifteen), draftsLabel([ins("x")]), draftsLabel([ins("x", { seed: null })]), draftsLabel([ins("demo", { by: "ai:demo" })])]) {
  check(!/inferred|seed|\bAI\b/i.test(l), `never "inferred", "seed" or "AI" on screen: "${l}"`);
}
check(seedWord("insights.chunks") === "Insights" && seedWord("") === null && seedWord(null) === null && seedWord("  ") === null, "seedWord");

// addTarget: null for no current, a draft current, an archived current, an unknown current; the list when it is his and live
const boards = [his("mine"), ins("draft"), his("gone", { archived: true })];
check(addTarget(null, boards) === null && addTarget(undefined, boards) === null, "no current -> the picker");
check(addTarget({ id: "draft" }, boards) === null, "a draft current -> the picker (never a draft)");
check(addTarget({ id: "gone" }, boards) === null, "an archived current -> the picker");
check(addTarget({ id: "other" }, boards) === null, "a current that is no longer served -> the picker");
check(addTarget({ id: "mine" }, boards)?.id === "mine", "his live current -> itself");
check(addTarget({ id: "mine" }, []) === null, "no boards served (offline) -> the picker, never a guess");

// pickerLists: his live lists only, in the served order
check(pickerLists(boards).map((x) => x.id).join() === "mine", "the picker lists his live lists only");
check(pickerLists([his("z"), his("y"), ins("d")]).map((x) => x.id).join() === "z,y", "the served order is kept");

// notesFor: written text -> idea -> quote -> the unattributed split -> empty
const from = { unit: "u1", date: "2026-10-01", doc_title: "Set 3" };
check(JSON.stringify(notesFor({ text: { setup: "Kale is a scam.", reveal: "It is lettuce with a publicist.", from }, idea: null, quote: "x" })) === JSON.stringify({ kind: "written", setup: "Kale is a scam.", reveal: "It is lettuce with a publicist.", idea: null }), "a written version wins");
check(notesFor({ text: { setup: " Kale ", reveal: null, from } }).setup === "Kale" && notesFor({ text: { setup: " Kale ", reveal: null, from } }).reveal === null, "trimmed; a missing reveal stays null");
const typed = notesFor({ text: { setup: "Kale", reveal: "lettuce", from: null }, idea: "Kale, lettuce with a publicist", quote: "taped line" });
check(typed.kind === "idea" && typed.idea === "Kale, lettuce with a publicist" && typed.setup === null, "no written version: his typed idea, whole (the split is the server's guess)");
const taped = notesFor({ text: null, idea: null, quote: "the taped line", cut_start: true });
check(taped.kind === "quote" && taped.setup === "…the taped line" && taped.reveal === null, "no text, no idea: the taped line is the body, cut marks kept");
const split = notesFor({ text: { setup: "a", reveal: "b", from: null }, idea: null, quote: null });
check(split.kind === "split" && split.setup === "a" && split.reveal === "b", "a split with nothing else still shows");
check(notesFor({ text: { setup: "", reveal: "  ", from }, idea: " ", quote: "" }).kind === "empty", "blank everywhere is empty");
check(notesFor(null).kind === "empty" && notesFor(undefined).kind === "empty" && notesFor({}).kind === "empty", "nothing is empty");

// the footer and the row label
check(JSON.stringify(notesFooter(true, 0, 7)) === JSON.stringify({ label: "Next bit", disabled: false }), "stage, mid-set: Next bit");
check(JSON.stringify(notesFooter(true, 6, 7)) === JSON.stringify({ label: "Last bit", disabled: true }), "stage, the last bit: Last bit, disabled");
check(JSON.stringify(notesFooter(true, 0, 0)) === JSON.stringify({ label: "Last bit", disabled: true }), "stage, no bits: disabled");
check(JSON.stringify(notesFooter(false, 2, 7)) === JSON.stringify({ label: "Open the bit", disabled: false }), "from the set list: Open the bit");
check(stageRowLabel(2, 7, "Kale", 2) === "3 of 7, Kale, current. Notes", "the current row");
check(stageRowLabel(0, 7, "Kale", 2) === "1 of 7, Kale, done. Notes", "a crossed-out row");
check(stageRowLabel(5, 7, "Kale", 2) === "6 of 7, Kale. Notes", "a row still to come");

console.log(fails.length ? `${fails.length} FAILED` : "ALL PASS");
process.exit(fails.length ? 1 : 0);
