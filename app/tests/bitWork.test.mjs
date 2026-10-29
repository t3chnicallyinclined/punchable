// Unit test for lib/bitWorkRules.ts (the workbench's pure rules). node --experimental-strip-types tests/bitWork.test.mjs
import * as B from "../src/lib/bitWorkRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

check(B.pickMain([]) === null, "nothing picked from nothing");
check(B.pickMain([{ id: "a" }]).id === "a", "one is the main");
check(B.pickMain([{ id: "a", versions: 1 }, { id: "b", versions: 3, sets: 0 }, { id: "c", versions: 3, sets: 2 }]).id === "c", "most versions, then most nights");
check(B.pickMain([{ id: "a", versions: 2, sets: 5 }, { id: "b", versions: 2, sets: 5 }]).id === "a", "a tie keeps the first he picked");
check(B.answerLine("The meter bit [1] is the same as the quarter one [2].") === "The meter bit is the same as the quarter one.", "source marks come out of a kept answer");
check(B.answerLine("  two   spaces \n here [3]") === "two spaces here", "whitespace folded");
check(B.answerLine(null) === "" && B.answerLine(undefined) === "", "no answer is empty");

if (fails.length) { console.log(`${fails.length} FAILED`); process.exit(1); }
console.log("ALL PASS");
