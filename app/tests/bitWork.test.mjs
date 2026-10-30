// Unit test for rules/bitWorkRules.ts (the workbench's pure rules). node --experimental-strip-types tests/bitWork.test.mjs
import * as B from "../src/rules/bitWorkRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

check(B.pickMain([]) === null, "nothing picked from nothing");
check(B.pickMain([{ id: "a" }]).id === "a", "one is the main");
check(B.pickMain([{ id: "a", versions: 1 }, { id: "b", versions: 3, sets: 0 }, { id: "c", versions: 3, sets: 2 }]).id === "c", "most versions, then most nights");
check(B.pickMain([{ id: "a", versions: 2, sets: 5 }, { id: "b", versions: 2, sets: 5 }]).id === "a", "a tie keeps the first he picked");
check(B.answerLine("The meter bit [1] is the same as the quarter one [2].") === "The meter bit is the same as the quarter one.", "source marks come out of a kept answer");
check(B.answerLine("  two   spaces \n here [3]") === "two spaces here", "whitespace folded");
check(B.answerLine(null) === "" && B.answerLine(undefined) === "", "no answer is empty");

// a fresh bit (SPEC 2026-10-30 §3, §5): the marker title, the test for it, the name from the first line
check(B.freshBitTitle(new Date(Date.UTC(2026, 9, 30, 20, 5, 59))) === "Untitled bit 2026-10-30 20:05", `the marker title is the UTC minute: "${B.freshBitTitle(new Date(Date.UTC(2026, 9, 30, 20, 5, 59)))}"`);
check(B.isFreshBit(B.freshBitTitle(new Date())), "a title the app made is fresh");
check(B.isFreshBit("Untitled bit") && B.isFreshBit("  Untitled bit 2026-10-30 20:05 "), "the bare marker and a padded one are fresh");
check(!B.isFreshBit("Untitled bits of wisdom") && !B.isFreshBit("untitled bit 2026") && !B.isFreshBit("so my landlord") && !B.isFreshBit("") && !B.isFreshBit(null) && !B.isFreshBit(undefined), "only the marker prefix counts: not a word that starts with it, not another case, not empty");
check(B.nameFromFirstLine("so my landlord, right, he texts me at 3am...") === "so my landlord, right, he texts me", `seven words, trailing punctuation cut: "${B.nameFromFirstLine("so my landlord, right, he texts me at 3am...")}"`);
check(B.nameFromFirstLine("Kale is a scam.") === "Kale is a scam", "a short line loses its full stop");
check(B.nameFromFirstLine("one two three four five six seven eight") === "one two three four five six seven", "exactly seven words");
check(B.nameFromFirstLine("  \n\n  second line is the first with words\nthird") === "second line is the first with words", "the first NON-EMPTY line names it; later lines never do");
check(B.nameFromFirstLine("a".repeat(121)).length === 120, "a 121-character first line is cut to 120");
check(B.nameFromFirstLine("a".repeat(120)) === "a".repeat(120), "120 stays 120");
check(B.nameFromFirstLine("") === "New bit" && B.nameFromFirstLine("   \n ") === "New bit" && B.nameFromFirstLine(null) === "New bit" && B.nameFromFirstLine(undefined) === "New bit", "nothing to name it from: New bit");
check(B.nameFromFirstLine("?!...") === "New bit", "punctuation alone is nothing");
check(B.nameFromFirstLine("wait, what? no, really, listen, ok, so, yes, go") === "wait, what? no, really, listen, ok, so", "punctuation inside the seven words stays; only the tail is cut");

if (fails.length) { console.log(`${fails.length} FAILED`); process.exit(1); }
console.log("ALL PASS");
