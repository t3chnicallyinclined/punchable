// Unit test for the upload retry queue rules (src/rules/uploadsCore.ts: backoffMs, dueUploads, afterFailure, isPermanent),
// the queue driver (runQueue) with the recorder and the uploader stubbed, and sendOnce's 401 rule (refresh, retry
// once with fresh headers, keep the recording), plus the say rules (src/rules/sayRules.ts).
//   node --experimental-strip-types --no-warnings tests/uploadQueue.test.mjs
import { parseCommand, guessKind } from "../src/rules/sayRules.ts";
import { IDEMPOTENCY_HEADER, afterFailure, backoffMs, dueUploads, isPermanent, runQueue, sendOnce } from "../src/rules/uploadsCore.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

// ---- backoff
check(backoffMs(0) === 5000 && backoffMs(1) === 30000 && backoffMs(3) === 600000 && backoffMs(9) === 1800000, "backoff: 5 s, 30 s, 2 min, 10 min, then 30 min forever");
const u = { id: "a", uri: "file:///set.m4a", name: "set.m4a", type: "audio/mp4", meta: { kind: "set" }, attempts: 0, nextAt: 0 };
const f1 = afterFailure(u, 1000, "Network request failed");
check(f1.attempts === 1 && f1.nextAt === 6000 && f1.lastError === "Network request failed", "a failure schedules the next try and keeps why");
check(dueUploads([f1, { ...u, id: "b", nextAt: 0 }], 2000).map((x) => x.id).join() === "b", "only due items run, oldest first");
check(isPermanent(400) && isPermanent(413) && !isPermanent(401) && !isPermanent(429) && !isPermanent(408) && !isPermanent(500) && !isPermanent(null), "4xx is permanent except 401/408/429; 5xx and no status retry");

// ---- the driver: stubbed uploader; the queue state is a plain array the driver returns
const now = 10_000;
const q = [
  { ...u, id: "ok" },
  { ...u, id: "flaky" },
  { ...u, id: "refused" },
  { ...u, id: "later", nextAt: 99_999 },
];
const calls = [];
const uploader = async (item) => {
  calls.push(item.id);
  if (item.id === "flaky") throw Object.assign(new Error("Network request failed"), { status: null });
  if (item.id === "refused") throw Object.assign(new Error("only audio or video files can be uploaded"), { status: 400 });
  return { status: "queued", file: item.name };
};
const r = await runQueue(q, uploader, now);
check(calls.join() === "ok,flaky,refused", "due items are tried in order; the one scheduled later is not");
check(r.queue.map((x) => x.id).join() === "flaky,later", "sent and refused items leave the queue; the flaky one stays");
check(r.queue[0].attempts === 1 && r.queue[0].nextAt === now + 5000, "the flaky one is backed off");
check(r.sent.length === 1 && r.sent[0].id === "ok" && r.dropped.length === 1 && r.dropped[0].id === "refused", "the driver reports what was sent and what was refused");
check(r.dropped[0].error.includes("audio or video"), "a refusal keeps the server's words");
// ---- sendOnce: the 401 rule (docs/ARCHITECTURE.md §7.2). A fake token machine hands out a new bearer on refresh.
function harness(statuses, { signedIn = true } = {}) {
  const log = { puts: [], headers: [], refreshes: 0 };
  let token = "stale";
  const deps = {
    headers: async () => { log.headers.push(token); return { Authorization: `Bearer ${token}`, "X-Kind": "set" }; },
    put: async (item, headers) => { log.puts.push(headers); const st = statuses[log.puts.length - 1]; return { status: st, body: st === 400 ? JSON.stringify({ error: "only audio or video files can be uploaded" }) : JSON.stringify({ status: "queued", file: item.name }) }; },
    refresh: async () => { log.refreshes++; if (signedIn) token = "fresh"; return signedIn; },
  };
  return { deps, log };
}
{
  // 401 then success: one refresh, the retry carries the fresh bearer, the recording goes up once and leaves the queue
  const { deps, log } = harness([401, 200]);
  const r = await runQueue([{ ...u, id: "tape-1" }], (item) => sendOnce(item, deps), now);
  check(r.sent.length === 1 && r.queue.length === 0 && r.dropped.length === 0, "401 then success: uploaded once, nothing dropped, nothing left");
  check(log.refreshes === 1 && log.puts.length === 2 && log.puts[0].Authorization === "Bearer stale" && log.puts[1].Authorization === "Bearer fresh", "the retry reads fresh headers after the refresh");
  check(log.puts.every((h) => h[IDEMPOTENCY_HEADER] === "tape-1" && h["X-Kind"] === "set"), "every attempt carries X-Idempotency-Key = the item's id and the upload headers");
}
{
  // 401 twice: one refresh, one retry, then the recording stays queued (backed off), never dropped
  const { deps, log } = harness([401, 401]);
  const r = await runQueue([{ ...u, id: "tape-2" }], (item) => sendOnce(item, deps), now);
  check(r.queue.length === 1 && r.queue[0].id === "tape-2" && r.queue[0].attempts === 1 && r.queue[0].nextAt === now + 5000 && r.dropped.length === 0 && r.sent.length === 0, "401 twice: kept in the queue and backed off, not dropped");
  check(log.refreshes === 1 && log.puts.length === 2, "exactly one refresh and one retry");
  check(/signed in/i.test(r.queue[0].lastError || ""), "the queue says why (not signed in)");
}
{
  // the refresh says signed out (invalid_grant): no retry, the recording stays for after he signs in again
  const { deps, log } = harness([401, 200], { signedIn: false });
  const r = await runQueue([{ ...u, id: "tape-3" }], (item) => sendOnce(item, deps), now);
  check(r.queue.length === 1 && r.dropped.length === 0 && log.puts.length === 1 && log.refreshes === 1, "signed out: one attempt, no retry, kept");
}
{
  // 400 still drops, with the server's words; no refresh is tried
  const { deps, log } = harness([400]);
  const r = await runQueue([{ ...u, id: "tape-4" }], (item) => sendOnce(item, deps), now);
  check(r.dropped.length === 1 && r.dropped[0].error.includes("audio or video") && r.queue.length === 0 && log.refreshes === 0, "400 still drops and says why");
}
{
  // a network failure in put is passed through (status null): kept and backed off
  const deps = { headers: async () => ({}), put: async () => { throw Object.assign(new Error("Network request failed"), { status: null }); }, refresh: async () => true };
  const r = await runQueue([{ ...u, id: "tape-5" }], (item) => sendOnce(item, deps), now);
  check(r.queue.length === 1 && r.queue[0].lastError === "Network request failed", "no connection: kept");
}

// a recorder stub: the record screen only needs start/stop/elapsed; nothing here touches a microphone
const rec = { state: "idle", start: async function () { this.state = "recording"; }, stop: async function () { this.state = "stopped"; return { uri: "file:///x.m4a" }; } };
await rec.start(); const out = await rec.stop();
check(rec.state === "stopped" && out.uri.endsWith(".m4a"), "recorder stub shape (start → stop → uri)");

// ---- commands (client mirror of say_api.parse_command; the server is the one that acts)
check(parseCommand("new 7 for friday")?.op === "new_set" && parseCommand("new 7 for friday").minutes === 7, "new 7 for friday → new_set 7");
check(parseCommand("make a 10 for the club")?.title === "Club · 10", "make a 10 for the club → Club · 10");
check(parseCommand("add kale trophy to tonight")?.op === "add_to_tonight" && parseCommand("add kale trophy to tonight").query === "kale trophy", "add … to tonight → add_to_tonight");
check(parseCommand("what's new for friday") === null && guessKind("what's new for friday") === "question", "a question about the new set is a question");
check(parseCommand("the new 7 killed") === null && guessKind("the new 7 killed") === "bit", "a bit that mentions the new 7 is a bit");
check(guessKind("new 5") === "command" && guessKind("put the kale bit in tonight's set") === "command", "guessKind routes commands to the conversation");
check(parseCommand("new 500 for friday") === null, "a length out of range is not a command");

console.log(fails.length ? `${fails.length} FAILED` : "ALL PASS");
process.exit(fails.length ? 1 : 0);
