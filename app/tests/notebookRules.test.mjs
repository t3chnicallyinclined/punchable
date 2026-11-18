// Unit test for rules/notebookRules.ts (the one list: SPEC 2026-11-17 one notebook, screen 1 and §3). Every title and
// line is invented.
//   node --experimental-strip-types --no-warnings tests/notebookRules.test.mjs               the suite
//   node --experimental-strip-types --no-warnings tests/notebookRules.test.mjs --red         every red arm
//   node --experimental-strip-types --no-warnings tests/notebookRules.test.mjs --red=<arm>   one of: order, pinned, status, his, roots
// A red arm plants a plausible wrong rule and must FAIL the same check the real rule passes:
//   order   a list sorted by title (the page he just touched would not be on top)
//   pinned  a list that leaves a pinned row in its day too (the same page twice on one screen)
//   status  a status that counts marks before a waiting draft (the one thing that waits for him would be hidden)
//   his     a row that shows any summary as his line (the app's own wording passed off as his words)
//   roots   an old tab URL that lands on the notebook with no filter (a link to Sets would show everything)
import * as N from "../src/rules/notebookRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const NOW = new Date(2026, 10, 17, 21, 0, 0); // Tuesday 17 November 2026, local
const iso = (y, m, d, h = 12) => new Date(y, m - 1, d, h, 0, 0).toISOString();
const BITS = [
  { id: "joke:checkout", title: "Self-checkout", punch: "looks at me like I tried to steal a lime.", touched: iso(2026, 11, 17, 9), yours: true, summary: "the machine" },
  { id: "joke:voicemail", title: "My dad's voicemails (phone)", punch: null, yours: true, summary: "He starts every one with his own name", touched: iso(2026, 11, 16, 9) },
  { id: "joke:fivea", title: "Gym at 5 a.m.", punch: null, yours: false, summary: "A bit about a gym, in the app's words", last_seen: "2026-11-12" },
  { id: "joke:old", title: "Parking at the airport", punch: "I left the car there for good.", first_seen: "2025-06-02" },
];
const rowsOf = () => [
  N.bitRow(BITS[0]),
  N.bitRow(BITS[1], { marks: 2, tapes: 1 }),
  N.bitRow(BITS[2]),
  N.bitRow(BITS[3]),
  N.setRow({ id: "sb:friday", title: "Friday, late show", n: 5, runtime_s: 420, updated: iso(2026, 11, 15) }, "Sunday"),
  N.tapeRow({ from: "phone", id: "rec1", title: "Voice memo", at: iso(2026, 11, 10, 23), kind: "Memo" }),
  N.tapeRow({ from: "server", id: "set:x", title: "Late show", at: "2026-10-30", line: "Set · 9:12" }),
];

// ---- the checks, each taking the rule under test so a red arm can hand in a wrong one

/** Newest first, under its day; a draft he asked for lifts its page to when he asked. */
function ordered(notebook) {
  const rows = rowsOf();
  rows[3] = N.bitRow(BITS[3], { round: "ready", sentAt: iso(2026, 11, 17, 20) });
  const n = notebook(rows, { filter: "all", now: NOW });
  const flat = n.groups.flatMap((g) => g.rows.map((r) => r.key));
  return eq(n.groups.map((g) => g.title), ["Today", "Yesterday", "This week", "This month", "October"])
    && eq(flat.slice(0, 3), ["bit:joke:old", "bit:joke:checkout", "bit:joke:voicemail"])
    && n.count === 7;
}
/** A pinned row is in Pinned, and not again under its day. */
function pinnedOnce(notebook) {
  const n = notebook(rowsOf(), { filter: "all", pins: ["set:sb:friday", "bit:joke:fivea"], now: NOW });
  const below = n.groups.flatMap((g) => g.rows.map((r) => r.key));
  return eq(n.pinned.map((r) => r.key), ["set:sb:friday", "bit:joke:fivea"])
    && !below.includes("set:sb:friday") && !below.includes("bit:joke:fivea")
    && below.length === 5 && n.count === 7;
}
/** The status line: a waiting draft first; else marks and tapes; nothing when there is nothing. */
function statuses(bitStatus) {
  return eq(bitStatus({ round: "ready", marks: 2, tapes: 1 }), { text: "New draft ready", hot: true })
    && eq(bitStatus({ marks: 2, tapes: 1 }), { text: "2 marks · 1 tape", hot: false })
    && eq(bitStatus({ marks: 1 }), { text: "1 mark", hot: false })
    && bitStatus({}) === null
    && bitStatus({ round: "waiting", marks: 3 })?.text === "Writing a new draft…";
}
/** The line under a title is his words or nothing: a sentence of the page, or the idea he typed himself. */
function hisWords(bitRow) {
  return bitRow(BITS[0]).line === "looks at me like I tried to steal a lime."
    && bitRow(BITS[1]).line === "He starts every one with his own name"
    && bitRow(BITS[2]).line === ""
    && bitRow(BITS[1]).title === "My dad's voicemails";
}
/** Every old tab root lands on the notebook with its filter on. */
function rootsLand(roots, showOf) {
  const filterOf = (url) => showOf(new URL(url, "https://x.test").searchParams.get("show"));
  return eq(Object.keys(roots).sort(), ["/bits", "/filmed", "/sets", "/tapes"])
    && Object.values(roots).every((u) => u.startsWith("/?show="))
    && filterOf(roots["/bits"]) === "bits" && filterOf(roots["/sets"]) === "sets"
    && filterOf(roots["/tapes"]) === "tapes" && filterOf(roots["/filmed"]) === "tapes";
}

// ---- the red arms: each a plausible wrong rule
const RED = {
  order: () => ordered((rows, o) => {
    const n = N.notebook(rows, o);
    const all = n.groups.flatMap((g) => g.rows).sort((a, b) => a.title.localeCompare(b.title));
    return { ...n, groups: [{ title: "Today", rows: all }] };
  }),
  pinned: () => pinnedOnce((rows, o) => ({ ...N.notebook(rows, { ...o, pins: [] }), pinned: rows.filter((r) => (o.pins || []).includes(r.key)) })),
  status: () => statuses((o) => (o.marks || o.tapes ? N.bitStatus({ marks: o.marks, tapes: o.tapes }) : N.bitStatus(o))),
  his: () => hisWords((j, o) => ({ ...N.bitRow(j, o), line: j.punch || j.summary || "" })),
  roots: () => rootsLand({ "/bits": "/", "/sets": "/", "/tapes": "/", "/filmed": "/" }, N.showOf),
};

const red = process.argv.find((a) => a.startsWith("--red"));
if (red) {
  const which = red.includes("=") ? [red.split("=")[1]] : Object.keys(RED);
  for (const k of which) {
    if (!RED[k]) { console.log(`no red arm "${k}"`); process.exit(2); }
    const passed = RED[k]();
    console.log(`${passed ? "NOT RED" : "RED "} ${k}`);
    if (passed) fails.push(`the red arm "${k}" passed: its check cannot fail`);
  }
} else {
  check(ordered(N.notebook), "newest first under its day; a draft he asked for lifts its page");
  check(pinnedOnce(N.notebook), "a pinned row is listed once, in Pinned");
  check(statuses(N.bitStatus), "the status line: a waiting draft first, else marks and tapes, else nothing");
  check(hisWords(N.bitRow), "the line under a title is his words or nothing");
  check(rootsLand(N.OLD_ROOTS, N.showOf), "every old tab root lands on the notebook with its filter");

  // the edges
  const rows = rowsOf();
  check(eq(N.notebook(rows, { filter: "bits", now: NOW }).count, 4) && N.notebook(rows, { filter: "sets", now: NOW }).count === 1 && N.notebook(rows, { filter: "tapes", now: NOW }).count === 2, "the filter keeps its kind");
  const withTopic = [...rows, N.topicRow({ id: "topic:1", text: "Airports", touched: iso(2026, 11, 1) })];
  check(N.notebook(withTopic, { filter: "sets", now: NOW }).count === 2 && N.notebook(withTopic, { filter: "bits", now: NOW }).count === 4, "a topic not yet a set is listed with the sets");
  const found = N.notebook(rows, { filter: "all", q: "  LIME ", pins: ["bit:joke:checkout"], now: NOW });
  check(found.count === 1 && found.pinned.length === 0 && found.groups[0].rows[0].key === "bit:joke:checkout", "a search reads titles and lines, and lists a pinned row with the rest");
  check(N.notebook(rows, { filter: "all", q: "friday", now: NOW }).count === 1, "a search finds a set by its name");
  const hits = N.found([rows[3], rows[0]], rows.slice(4), "late show", "all");
  check(eq(hits.map((r) => r.key), ["bit:joke:old", "bit:joke:checkout", "set:sb:friday", "tape:set:x"]), "a search: the pages in the server's order, then the sets and tapes that say the words");
  check(eq(N.found([rows[3]], rows.slice(4), "late show", "sets").map((r) => r.key), ["set:sb:friday"]) && N.found([], rows.slice(4), "", "all").length === 0, "a search under a filter keeps its kind; no words find no other rows");
  check(N.dayGroup(iso(2026, 11, 17, 1), NOW) === "Today" && N.dayGroup("2026-11-16", NOW) === "Yesterday" && N.dayGroup("2026-11-11", NOW) === "This week" && N.dayGroup("2026-11-02", NOW) === "This month" && N.dayGroup("2026-03-09", NOW) === "March" && N.dayGroup("2025-06-02", NOW) === "June 2025" && N.dayGroup("", NOW) === "Earlier" && N.dayGroup("not a date", NOW) === "Earlier", "the day a row is under");
  check(N.dayGroup(iso(2026, 11, 18, 1), NOW) === "Today", "a clock a little ahead still reads Today");
  const s = N.setRow({ id: "sb:a", title: "Friday, late show", n: 5, runtime_s: 420, updated: "t" }, "Sunday");
  check(s.line === "5 bits · about 7 min · last taped Sunday" && s.href === "/list/sb%3Aa" && s.tag === "Set", "a set's line (the mockup's words)");
  check(N.setRow({ id: "sb:b", title: "", n: 1 }).line === "1 bit" && N.setRow({ id: "sb:b", title: "", n: 1 }).title === "Untitled set", "a set with nothing known says only what it knows");
  check(N.aboutMin(420) === "about 7 min" && N.aboutMin(20) === "under a minute" && N.aboutMin(0) === null && N.aboutMin(null) === null, "about N min");
  check(N.bitRow(BITS[0], { round: "ready" }).href === "/joke/joke%3Acheckout" && N.bitRow(BITS[0]).href === "/joke/joke%3Acheckout", "a row opens its page, also when a draft waits (the page's button opens the draft over it)");
  check(N.bitRow({ id: "joke:n", title: "Untitled bit 2026-11-17 21:04" }).title === "New page", "a page the pencil made, not named yet");
  check(N.bitRow(BITS[0], { round: "failed", marks: 1 }).status === "No draft came · 1 mark", "a round that made no draft says so");
  check(N.bitAt({ id: "x", title: "x", touched: "2026-11-01T10:00:00.123+00:00", last_seen: "2026-11-03" }) === "2026-11-03", "a bit's time is the newest thing known about it");
  check(N.countLabel(42, "all") === "42 pages" && N.countLabel(1, "all") === "1 page" && N.countLabel(3, "sets") === "3 sets" && N.countLabel(1, "tapes") === "1 tape" && N.countLabel(2, "bits") === "2 bits", "the count under the list");
  check(eq(N.togglePin(["a"], "b"), ["b", "a"]) && eq(N.togglePin(["b", "a"], "b"), ["a"]), "pin and unpin");
  check(N.showOf(undefined) === "all" && N.showOf("nope") === "all" && N.showOf(["sets"]) === "sets", "an unknown filter is All");
  check(N.pencilMakes("sets") === "set" && N.pencilMakes("all") === "page" && N.pencilMakes("tapes") === "page", "the pencil makes a set on Sets, else a page");
  const phone = N.tapeRow({ from: "phone", id: "rec1", title: "Voice memo", at: "t", kind: "Memo" });
  check(phone.href === null && phone.rec === "rec1" && phone.key === "rec:rec1", "a recording on this phone draws itself");
}

if (fails.length) {
  console.log(`\n${fails.length} FAILED`);
  process.exit(1);
}
console.log("\nall passed");
