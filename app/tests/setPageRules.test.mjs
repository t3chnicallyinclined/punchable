// Unit test for rules/setPageRules.ts (a set is a page: SPEC 2026-11-17 one notebook, screen 5). Every title is
// invented.
//   node --experimental-strip-types --no-warnings tests/setPageRules.test.mjs               the suite
//   node --experimental-strip-types --no-warnings tests/setPageRules.test.mjs --red         every red arm
//   node --experimental-strip-types --no-warnings tests/setPageRules.test.mjs --red=<arm>   one of: twice, order, twin, at
// A red arm plants a plausible wrong rule and must FAIL the same check the real rule passes:
//   twice  an offer list that includes bits already in the set (a tap would add a bit that is there: a 409)
//   order  an offer list in the list's own order (the bit whose name starts with his letters would not lead)
//   twin   a "New bit called" that is offered even when a bit has exactly that name (two bits, one name)
//   at     a search that keeps the @ (nothing would ever match)
import * as S from "../src/rules/setPageRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const BITS = [
  { id: "j:group", title: "Parent group chat" },
  { id: "j:gym", title: "Gym at 5 a.m." },
  { id: "j:spar", title: "My sparring partner (long version)", short_name: "Sparring" },
  { id: "j:park", title: "Parking at the airport" },
  { id: "j:checkout", title: "Self-checkout", short_name: "Checkout" },
  { id: "j:dad", title: "My dad's voicemails" },
];
const IN_SET = ["j:gym", "j:checkout", "j:dad"];

// ---- the checks, each taking the rule under test so a red arm can hand in a wrong one

/** A bit already in the set is never offered. */
function notTwice(offers) {
  return offers(BITS, IN_SET, "@gym").length === 0
    && eq(S.offers(BITS, [], "@gym").map((b) => b.id), ["j:gym"])
    && offers(BITS, new Set(IN_SET), "check").length === 0;
}
/** His letters at the start of a name lead; then at the start of a word; then anywhere; ties in the list's order. */
function ordered(offers) {
  return eq(offers(BITS, IN_SET, "@par").map((b) => b.id), ["j:group", "j:park", "j:spar"])
    && eq(offers(BITS, [], "a").map((b) => b.id).slice(0, 2), ["j:gym", "j:park"])
    && offers(BITS, [], "@").length === 0
    && offers(BITS, [], "   ").length === 0
    && offers(BITS, [], "p", 1).length === 1;
}
/** "New bit called …" is offered for a new name, never for a name a bit already has. */
function noTwin(newBitName) {
  return newBitName(BITS, "@par") === "par"
    && newBitName(BITS, "@Parking at the airport") === null
    && newBitName(BITS, " @sparring ") === null
    && newBitName(BITS, "@") === null
    && newBitName(BITS, "@ Airport   security ") === "Airport security";
}
/** The @ is optional and never part of what is searched for. */
function atIsOptional(asked) {
  return asked("@par") === "par" && asked("par") === "par" && asked("  @  par k ") === "par k" && asked("@") === "" && asked("") === "";
}

// ---- the red arms: each a plausible wrong rule
const RED = {
  twice: () => notTwice((bits, _inSet, text, max) => S.offers(bits, [], text, max)),
  order: () => ordered((bits, inSet, text, max = S.MAX_OFFERS) => {
    const q = S.asked(text).toLowerCase();
    const taken = new Set(inSet);
    return q ? bits.filter((b) => !taken.has(b.id) && (b.title.toLowerCase().includes(q) || (b.short_name || "").toLowerCase().includes(q))).slice(0, max) : [];
  }),
  twin: () => noTwin((_bits, text) => S.asked(text) || null),
  at: () => atIsOptional((text) => (text || "").trim()),
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
  check(notTwice(S.offers), "a bit already in the set is never offered");
  check(ordered(S.offers), "his letters at the start of a name lead, then a word's start, then anywhere");
  check(noTwin(S.newBitName), "New bit called … only for a name no bit has");
  check(atIsOptional(S.asked), "the @ is optional and never searched for");

  // the edges
  check(S.nameOf(BITS[2]) === "Sparring" && S.nameOf(BITS[3]) === "Parking at the airport" && S.nameOf({ id: "x", title: "" }) === "Untitled", "a bit's name on a set");
  check(eq(S.offers(BITS, [], "CHECK").map((b) => b.id), ["j:checkout"]) && eq(S.offers(BITS, [], "voicemail").map((b) => b.id), ["j:dad"]), "letters match whatever their case, by short name or title");
  check(eq(S.offers([{ id: "a", title: "Café at night" }], [], "cafe").map((b) => b.id), ["a"]), "an accent does not hide a bit");
  check(S.newBitLabel("par") === "New bit called “par”", "the button's words");
  check(S.newBitName(BITS, "@" + "x".repeat(200))?.length === S.MAX_NAME, "a new bit's name is cut to a title's length");
  check(S.summaryLine(5, 420, 480) === "5 bits · about 7 min · you have 8", "the line under the title (the mockup's words)");
  check(S.summaryLine(1, 70, null) === "1 bit · about 1 min" && S.summaryLine(0, 0, 420) === "0 bits · you have 7", "no slot, or nothing on it yet");
  check(S.summaryLine(6, 600, 420) === "6 bits · about 10 min · 3 min over your 7", "a set that runs over says by how much");
  check(S.HINT.startsWith("Type a line to add a bit."), "the hint under the list");
}

if (fails.length) {
  console.log(`\n${fails.length} FAILED`);
  process.exit(1);
}
console.log("\nall passed");
