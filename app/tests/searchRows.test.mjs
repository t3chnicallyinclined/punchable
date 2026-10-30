// Unit test for rules/searchRows.ts (the rows under a search and on a bit's page: where-and-when, the deep link,
// closeness in words). node --experimental-strip-types tests/searchRows.test.mjs   (npm run test:unit)
import * as S from "../src/rules/searchRows.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

check(S.whenWhere({ venue: "The Cellar", date: "2025-03-04", start_ms: 60000 }) === "The Cellar · Mar 2025 · 1:00", "venue · month · minute");
check(S.whenWhere({ venue: null, set_title: "Tuesday open mic", date: "2025-03-04", start_ms: 0 }) === "Tuesday open mic · Mar 2025 · 0:00", "falls back to the set's title; 0:00 is a place too");
check(S.whenWhere({ venue: "", set_title: "", date: null, start_ms: null }) === "", "nothing known: empty, no stray dots");
check(S.whenWhere({ venue: "Room", date: "2025", start_ms: 125000 }) === "Room · 2025 · 2:05", "a year-only date still reads");
check(S.momentHref({ set: "set:abc def", start_ms: 60000 }) === "/set/set%3Aabc%20def?at=60000&play=1", "deep link into the set at the second, playing");
check(S.momentHref({ set: null, start_ms: 60000 }) === null, "no set, no link");
check(S.closeness(0.1) === "Very close" && S.closeness(0.3) === "Close" && S.closeness(0.7) === "A bit like it", "distance in words");
check(S.closeness(null) === null && S.closeness(undefined) === null && S.closeness(NaN) === null, "words-only match has no closeness");
check(S.oneLine("  so a   waterbug\nwalks in  ") === "so a waterbug walks in", "whitespace folded");
const long = "word ".repeat(60).trim();
const cut = S.oneLine(long, 50);
check(cut.endsWith("…") && cut.length <= 51 && !cut.includes("wor…"), "cut at a word with an ellipsis");
check(S.oneLine(null) === "", "null text is empty");

if (fails.length) { console.log(`${fails.length} FAILED`); process.exit(1); }
console.log("ALL PASS");
