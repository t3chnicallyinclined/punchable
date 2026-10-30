// Unit test for the upload retry queue rules (src/rules/sayRules.ts: backoffMs, dueUploads, afterFailure, isPermanent) and
// the queue driver (src/lib/uploads.ts runQueue) with the recorder and the uploader stubbed.
//   node --experimental-strip-types --no-warnings tests/uploadQueue.test.mjs
import { parseCommand, guessKind } from "../src/rules/sayRules.ts";
import { afterFailure, backoffMs, dueUploads, isPermanent, runQueue } from "../src/rules/uploadsCore.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

// ---- backoff
check(backoffMs(0) === 5000 && backoffMs(1) === 30000 && backoffMs(3) === 600000 && backoffMs(9) === 1800000, "backoff: 5 s, 30 s, 2 min, 10 min, then 30 min forever");
const u = { id: "a", uri: "file:///set.m4a", name: "set.m4a", type: "audio/mp4", meta: { kind: "set" }, attempts: 0, nextAt: 0 };
const f1 = afterFailure(u, 1000, "Network request failed");
check(f1.attempts === 1 && f1.nextAt === 6000 && f1.lastError === "Network request failed", "a failure schedules the next try and keeps why");
check(dueUploads([f1, { ...u, id: "b", nextAt: 0 }], 2000).map((x) => x.id).join() === "b", "only due items run, oldest first");
check(isPermanent(400) && isPermanent(413) && !isPermanent(429) && !isPermanent(408) && !isPermanent(500) && !isPermanent(null), "4xx is permanent except 408/429; 5xx and no status retry");

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
