// Haptic meaning and order (docs/design/HIG.md §3.6, §3.9, B8): success after the write, nothing on Stop, one impact
// per hold.   node --experimental-strip-types --no-warnings tests/feedback.test.mjs
import { afterWrite, holdHaptic, stopHaptic } from "../src/lib/feedback.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const recorder = () => { const log = []; return { log, h: { success: () => log.push("haptic:success") }, write: (result, fail) => () => new Promise((res, rej) => setTimeout(() => { log.push("write:done"); fail ? rej(new Error("no")) : res(result); }, 5)) }; };

// "To a set": the write, then success
{
  const r = recorder();
  const out = await afterWrite(r.write(true), (ok) => ok, r.h);
  check(out === true && r.log.join(",") === "write:done,haptic:success", "success plays after the write resolved, not before");
}
// the picker opened instead (nothing written): no success
{
  const r = recorder();
  await afterWrite(r.write(false), (ok) => ok, r.h);
  check(r.log.join(",") === "write:done", "no success when nothing was written");
}
// the write failed: the error passes through and no success plays
{
  const r = recorder();
  let threw = false;
  try { await afterWrite(r.write(true, true), (ok) => ok, r.h); } catch { threw = true; }
  check(threw && !r.log.includes("haptic:success"), "a failed write throws and plays no success");
}
// hold to talk: one impact per hold
check(holdHaptic("in") === "light" && holdHaptic("out") === null, "one light impact on press-in, none on release");
// Stop opens a confirmation, which is not a warning
check(stopHaptic() === null, "no warning haptic on tapping Stop");

console.log(fails.length ? `${fails.length} FAILED` : "ALL PASS");
process.exit(fails.length ? 1 : 0);
