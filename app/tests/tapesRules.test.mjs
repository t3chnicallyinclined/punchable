// Unit test for src/rules/tapesRules.ts (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §3.6, Slice 6): a topic is a
// set once the migration made one (before it, the topic still opens); Tapes is one list, newest first, each tape once.
//   node --experimental-strip-types --no-warnings tests/tapesRules.test.mjs
//   node --experimental-strip-types --no-warnings tests/tapesRules.test.mjs --red    the red arm: a planted rule that
//       drops every phone set that has been sent (a tape whose server set has another title, or is not listed yet,
//       would vanish from Tapes)
import * as R from "../src/rules/tapesRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const keys = (rows) => rows.map((r) => r.key);

const phone = [
  { id: "m1", kind: "memo", recorded_at: "2026-11-15T10:00:00Z", state: "new_idea" },
  { id: "s1", kind: "set", recorded_at: "2026-11-14T21:00:00Z", board: { id: "sb:fri", title: "Friday late" }, title: "Friday late", state: "sent" },
  { id: "s2", kind: "set", recorded_at: "2026-11-16T21:00:00Z", board: { id: "sb:sat", title: "Saturday" }, title: "Saturday", state: "sent", mode: "video" },
  { id: "f1", kind: "film", recorded_at: "2026-11-13T09:00:00Z", state: "filed" },
];
const server = [
  { id: "set:a", title: "Friday late", date: "2026-11-14" },
  { id: "set:b", title: "Open mic", date: "2026-11-01" },
];

/** The rule under test: a tape is shown exactly once, and a phone set is never hidden unless its server set is there. */
function eachTapeOnce(rows) {
  const k = keys(rows);
  return !k.includes("phone:s1") && k.includes("server:set:a") && k.includes("phone:s2") && k.length === 5;
}

if (process.argv.includes("--red")) {
  const lenient = (p, s) => R.tapeRows(p.filter((r) => !(r.kind === "set" && r.state === "sent")), s);
  const caught = !eachTapeOnce(lenient(phone, server));
  console.log(`${caught ? "PASS" : "FAIL"} red arm: dropping every sent phone set is caught (Saturday's tape vanishes)`);
  const real = eachTapeOnce(R.tapeRows(phone, server));
  console.log(`${real ? "PASS" : "FAIL"} the real tapeRows passes the same check`);
  process.exit(caught && real ? 0 : 1);
}

// ---- topics and the sets made from them
{
  const lists = [{ id: "sb:1", title: "Self-checkout", seed: { topic: "topic:a" } }, { id: "sb:2", title: "Bus", seed: { rant: "joke:x" } }, { id: "sb:3", title: "Mine", seed: null }];
  const topics = [{ id: "topic:a", text: "Self-checkout", touched: "2026-11-10" }, { id: "topic:b", text: "Landlords", touched: "2026-11-12" }, { id: "topic:c", text: "Gyms", created: "2026-11-11" }];
  check(R.boardOfTopic(lists, "topic:a")?.id === "sb:1" && R.boardOfTopic(lists, "topic:b") === null, "a topic's set is the list seeded with it");
  check(eq(R.topicsNotYetSets(topics, lists).map((t) => t.id), ["topic:b", "topic:c"]), "before the migration a topic is listed (newest first); after it, only its set is");
  check(R.topicsNotYetSets(topics, []).length === 3, "no lists made yet: every topic is still reachable");
  check(R.boardOfTopic([{ id: "x", title: "x", seed: "insights" }], "topic:a") === null, "an app draft's seed is never a topic");
}
// ---- Tapes: one list, newest first, each tape once
{
  const rows = R.tapeRows(phone, server);
  check(eqOrder(rows), "newest first across the phone and the server");
  check(eachTapeOnce(rows), "a sent phone set with its server set is shown once; the rest all stay");
  check(R.tapeRows([{ ...phone[1], state: "words" }], server).length === 3, "a phone set not yet sent stays beside the server's");
  check(R.tapeRows([phone[1]], [{ ...server[0], title: "Something else" }]).length === 2, "a different title on the server: both stay (never hidden by a guess)");
  check(eq(rows.map(R.tapeKind), ["Filmed set", "Memo", "Set", "Video", "Set"]), "each row says what it is in a word");
}
function eqOrder(rows) {
  return eq(keys(rows), ["phone:s2", "phone:m1", "server:set:a", "phone:f1", "server:set:b"]);
}

console.log(fails.length ? `\n${fails.length} FAILED` : "\nall passed");
process.exit(fails.length ? 1 : 0);
