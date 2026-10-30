// Unit test for src/rules/layoutRules.ts: the Tonight header's two-row shape at every text size (docs/design/HIG.md §0).
//   node --experimental-strip-types --no-warnings tests/layoutRules.test.mjs
import { LARGE_TITLE_PT, STACK_AT, headerTopPadding, rowFits, tonightTitleRow } from "../src/rules/layoutRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

// the pill sits beside the title up to Large; it stacks under it from 1.3x through the AX sizes
for (const fs of [0.8, 1, 1.1, 1.2, 1.29]) check(tonightTitleRow(fs).direction === "row", `font scale ${fs}: pill beside the title`);
for (const fs of [1.3, 1.4, 1.6, 2.0, 3.1]) check(tonightTitleRow(fs).direction === "column", `font scale ${fs}: pill stacks under the title`);
check(tonightTitleRow(NaN).direction === "row" && tonightTitleRow(0).direction === "row", "a bad font scale reads as 1");
check(tonightTitleRow(2).titleScale === 1.3 && tonightTitleRow(1.1).titleScale === 1.1, "the title caps at 1.3x (34 pt Large Title)");
check(STACK_AT === 1.3 && LARGE_TITLE_PT === 34, "the spec's numbers");

// build 3's one-row header over-committed on a 402 pt phone (20 pt side padding -> 362 inner): title 125 + pill 100 + plus 44 + Ask 70 + face 44
check(!rowFits(362, 125, [100, 44, 70, 44]), "build 3's one row did not fit (the title wrapped)");
// the bar row (date + plus + face) and the title row (title + pill) each fit, at 1x and with the pill scaled to 1.4x
check(rowFits(362, 120, [44, 44]), "bar row: date, +, face fit");
check(rowFits(362, 125, [100]), "title row: Tonight + pill fit at 1x");
check(rowFits(362, 125 * 1.3, [140]), "title row: Tonight at its 1.3x cap + a 1.4x pill still fit");
check(rowFits(362, 125, [100, 44, 70]) === false, "putting Ask back in the title row breaks it again");

// the inset is the system's; a missing value is 0, never a guessed device number
check(headerTopPadding(62) === 62 && headerTopPadding(0) === 0 && headerTopPadding(undefined) === 0 && headerTopPadding(-3) === 0, "header top padding = the safe-area inset");

console.log(fails.length ? `${fails.length} FAILED` : "ALL PASS");
process.exit(fails.length ? 1 : 0);
