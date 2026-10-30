// Unit test for src/rules/retry.ts.   node --experimental-strip-types --no-warnings tests/retry.test.mjs
import { retry } from "../src/rules/retry.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const noSleep = { sleep: async () => {} };

let n = 0;
check((await retry(async () => ++n, noSleep)) === 1 && n === 1, "a success returns at once");

n = 0;
const v = await retry(async () => { if (++n < 3) throw new Error("not yet"); return "ok"; }, noSleep);
check(v === "ok" && n === 3, "two failures then a success");

n = 0;
let err = null;
try { await retry(async () => { n++; throw new Error("still no"); }, { ...noSleep, tries: 4 }); } catch (e) { err = e; }
check(err && err.message === "still no" && n === 4, "every try fails -> the last error, after `tries` tries");

const waits = [];
n = 0;
await retry(async () => { if (++n < 3) throw new Error("x"); }, { delayMs: 100, sleep: async (ms) => { waits.push(ms); } });
check(waits.join(",") === "100,200", "the wait doubles between tries");

n = 0;
err = null;
try { await retry(async () => { n++; throw new Error("once"); }, { ...noSleep, tries: 0 }); } catch (e) { err = e; }
check(n === 1 && err, "tries below 1 still runs once");

if (fails.length) { console.log(`\n${fails.length} failed`); process.exit(1); }
console.log("\nall passed");
