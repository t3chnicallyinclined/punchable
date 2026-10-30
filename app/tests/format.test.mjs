// Unit test for src/rules/format.ts: the one m:ss and the three date formatters. The expected strings were captured
// from the OLD functions at 32a4af3 (theme.ts fmtMs / fmtDate, laughs.ts fmtDur / fmtShortDate, searchRows.ts and
// set/[id].tsx mmss, simple.ts monthYear) before they were replaced, so every caller keeps its exact output.
//   node --experimental-strip-types --no-warnings tests/format.test.mjs
import { MONTHS, MONTHS_LONG, clockMs, fmtDate, fmtShortDate, mmss, monthYear } from "../src/rules/format.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const table = (name, f, rows) => {
  for (const [input, want] of rows) {
    const got = f(input);
    check(got === want, `${name}(${input === undefined ? "undefined" : JSON.stringify(input)}) = ${JSON.stringify(got)}${got === want ? "" : `, want ${JSON.stringify(want)}`}`);
  }
};

// theme.ts fmtMs(ms): floor, never "–" (the record clock, the lengths the recorder wrote)
table("clockMs", clockMs, [[0, "0:00"], [999, "0:00"], [1000, "0:01"], [1500, "0:01"], [59999, "0:59"], [60000, "1:00"], [61000, "1:01"], [3599000, "59:59"], [3600000, "60:00"], [3661500, "61:01"], [-5, "0:00"], [null, "0:00"], [undefined, "0:00"], [NaN, "0:00"]]);
// laughs.ts fmtDur(s): round, "–" for no number (a bit's length, a set's runtime)
table("mmss", mmss, [[0, "0:00"], [0.4, "0:00"], [0.5, "0:01"], [1.5, "0:02"], [59.4, "0:59"], [59.5, "1:00"], [103.2, "1:43"], [600, "10:00"], [3599.6, "60:00"], [3661, "61:01"], [-5, "0:00"], [null, "–"], [undefined, "–"], [NaN, "–"], [Infinity, "–"]]);
// searchRows.ts mmss(ms) and set/[id].tsx mmss(ms) = fmtDur(ms / 1000): round
table("mmss(ms / 1000)", (ms) => mmss(ms / 1000), [[0, "0:00"], [499, "0:00"], [500, "0:01"], [1500, "0:02"], [60000, "1:00"], [125000, "2:05"], [3661500, "61:02"], [-5, "0:00"]]);
// the clock's floor through the option (what clockMs uses)
check(mmss(59.9, { floor: true }) === "0:59" && mmss(59.9) === "1:00", "floor is the option; round is the default");
// theme.ts fmtDate
table("fmtDate", fmtDate, [["2025-03-04", "Mar 4, 2025"], ["2025-03-04T10:00:00Z", "Mar 4, 2025"], ["2024-12-31", "Dec 31, 2024"], ["2025-01-05", "Jan 5, 2025"], ["", ""], [null, ""], [undefined, ""]]);
// laughs.ts fmtShortDate
table("fmtShortDate", fmtShortDate, [["2025-03-04", "Mar 4 '25"], ["2025-03-04T10:00:00Z", "Mar 4 '25"], ["2024-12-31", "Dec 31 '24"], ["2017-08-16", "Aug 16 '17"], ["", ""], [null, ""], [undefined, ""]]);
// simple.ts / searchRows.ts monthYear
table("monthYear", monthYear, [["2025-03-04", "Mar 2025"], ["2025-03", "Mar 2025"], ["2025", "2025"], ["2025-13-01", "2025"], ["2025-00-01", "2025"], ["", ""], [null, ""], [undefined, ""]]);
check(MONTHS.length === 12 && MONTHS[0] === "Jan" && MONTHS[11] === "Dec" && MONTHS_LONG.length === 12 && MONTHS_LONG[9] === "October", "the two month arrays");

console.log(fails.length ? `${fails.length} FAILED` : "ALL PASS");
process.exit(fails.length ? 1 : 0);
