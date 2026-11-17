// Unit test for rules/usageRules.ts (SPEC 2026-11-17 §13, Settings › Usage): spend is never shown as nothing, what
// cost most comes first, the month and the "what to charge" lines read right and say they are a guess, a refusal he
// can act on says so, the Settings row reaches only someone the page answers, and the export carries counts and
// money only.
//   node --experimental-strip-types --no-warnings tests/usageRules.test.mjs          the suite
//   node --experimental-strip-types --no-warnings tests/usageRules.test.mjs --red    the red arms: a money format that
//                                                                                    rounds a small spend to $0.00,
//                                                                                    kinds in the server's order, every
//                                                                                    refusal read as "can't reach", and
//                                                                                    a Settings row shown to every
//                                                                                    account must each FAIL
import * as R from "../src/rules/usageRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

const report = {
  from: "2026-11-01", to: "2026-11-10",
  days: [
    { day: "2026-11-01", ai_usd: 0.5, ai_calls: 3, requests: 40, writes: 9, bits: 4, redlines: 1, drafts: 3, sessions: 2, active: true },
    { day: "2026-11-02", ai_usd: 0, ai_calls: 0, requests: 0, writes: 0, bits: 0, redlines: 0, drafts: 0, sessions: 0, active: false },
  ],
  totals: {
    ai_usd: 2.004, ai_calls: 12,
    by_kind: [{ kind: "ask_chat", usd: 0.004, calls: 1 }, { kind: "riff", usd: 1.5, calls: 7 }, { kind: "redline", usd: 0.5, calls: 3 }],
    active_days: 1, requests: 40, writes: 9, bits: 4, redlines: 1, drafts: 3, sessions: 2,
    media: [{ kind: "transcript", count: 2, seconds: 1500, bytes: 9000 }],
  },
  month: {
    month: "2026-11", ai_usd: 2.004, days_elapsed: 10, days_in_month: 30, projected_usd: 6.012, active_days: 6,
    usd_per_active_day: 0.334, price_to_cover: { margin: 0.7, usd: 20.04, assumption: "the lead's" },
  },
  prices: { read: "2026-11-17", source: "https://platform.claude.com/docs/en/about-claude/pricing" },
};

/** A spend that happened never reads as nothing; round numbers read plainly. */
function moneyHonest(money) {
  return money(0.004) !== "$0.00" && money(0.004) !== "$0" && money(0.004).length > 0
    && money(0) === "$0" && money(1.5) === "$1.50" && money(20.04) === "$20.04" && money(1234.4) === "$1,234";
}
/** What cost most comes first, with its plain name. */
function costliestFirst(view) {
  const v = view(report);
  return v.kinds.map((k) => k.key).join(",") === "riff,redline,ask_chat" && v.kinds[0].name === "Riffs"
    && v.kinds[0].usd === "$1.50" && v.kinds[0].detail === "asked 7 times" && v.kinds[2].usd === "under 1¢"
    && v.kinds[2].detail === "asked once";
}
/** A refusal he can act on says what it is; a network failure says that. */
function refusalsSayWhy(err) {
  return err("the owner is not pinned on this server (PK_OWNER_SUB)") === "Usage isn’t switched on yet. It needs a one-time setting on the server."
    && err("this page is the owner's only") === "Only the owner can see usage."
    && err("Network request failed") === "Can’t reach Punchable right now."
    && err("HTTP 404") === "Usage comes with the next server update.";
}
/** The row reaches the owner, and everyone only while nothing is pinned (the page then says it is not on); never a
 *  tester, never when the server cannot say. */
function rowOnlyWhereItAnswers(show) {
  return show({ owner: true, pinned: true }) === true && show({ owner: false, pinned: false }) === true
    && show({ owner: false, pinned: true }) === false && show(null) === false && show(undefined) === false;
}

const v = R.usageView(report);
check(moneyHonest(R.money), "money: a small spend is never shown as nothing");
check(costliestFirst(R.usageView), "the kinds: most expensive first, with plain names");
check(v.heading === "AI cost, November so far" && v.spent === "$2.00" && v.spentLabel === "$2.00 spent on AI in November so far", "the heading says it is AI money, and VoiceOver hears it");
check(v.pace === "About $6 by the end of November at this pace", "the month at this pace, in whole dollars");
check(v.active === "Used on 6 of 10 days" && v.perDay === "$0.33 a day you used it", "active days, per active day");
check(v.cover.line === "To keep 70¢ of every dollar, charge about $20 a month" && v.cover.note.startsWith("An assumption, not a price"), "what to charge: plain words, whole dollars, labelled an assumption");
check(v.activityHeading === "What you did in November", "the activity says over what period");
check(v.activity.find((a) => a.key === "sessions").label === "Times the app connected" && !v.activity.some((a) => a.label === "App opens"), "a socket count is not called app opens");
check(v.activity.find((a) => a.key === "recordings").value === "2 · 25 min", "recordings: how many and how long");
check(v.footnote.includes("read Nov 17, 2026") && v.footnote.includes("nothing you wrote"), "the footnote: the prices' date and the privacy line");
check(refusalsSayWhy(R.usageError), "the refusals say why");
check(rowOnlyWhereItAnswers(R.showUsageRow), "the Settings row only where the page answers");
const early = R.usageView({ ...report, month: { ...report.month, days_elapsed: 2 } });
check(early.pace === "Too early in November to guess the month", "no guess on the first two days");
const empty = R.usageView({ ...report, totals: { ...report.totals, by_kind: [], requests: 0 }, month: { ...report.month, ai_usd: 0 } });
check(empty.empty === true && empty.pace === "Nothing spent on AI this month yet" && empty.perDay === null && empty.cover === null, "a month with nothing yet says so, and shows no price");
check(R.whole(0.4) === "$0.40" && R.whole(6.6) === "$7" && R.whole(1234.5) === "$1,235", "forecasts: whole dollars from $1");
const csv = R.usageCsv(report);
check(csv.split("\n")[0] === "day,ai_usd,ai_calls,requests,writes,bits,redlines,drafts,sessions,active"
  && csv.split("\n")[1] === "2026-11-01,0.500000,3,40,9,4,1,3,2,true" && csv.endsWith("\n"), "the export: counts and money, one line a day");
check(R.csvName(report) === "punchable-usage-2026-11-01-to-2026-11-10.csv", "the export's file name");
check(R.dayLabel("2026-01-05") === "Jan 5, 2026" && R.monthName("2026-12") === "December", "dates read plainly");

if (process.argv.includes("--red")) {
  // a money format that rounds to cents: a small spend reads $0.00 (the meter hiding money)
  const cents = (usd) => `$${(usd || 0).toFixed(2)}`;
  // the kinds as the server listed them (no sort): the costliest may sit last
  const unsorted = (r) => ({ kinds: r.totals.by_kind.map((k) => ({ key: k.kind, name: R.kindName(k.kind), usd: R.money(k.usd), detail: `asked ${k.calls} times` })) });
  // every failure read as a network failure: the not-pinned server tells him nothing he can act on
  const blind = () => "Can’t reach Punchable right now.";
  // the row for every signed-in account: a tester taps it and meets only a refusal
  const everyone = () => true;
  const caught = [
    ["a money format that shows a small spend as $0.00", !moneyHonest(cents)],
    ["kinds in the server's order", !costliestFirst(unsorted)],
    ["every refusal read as can't reach", !refusalsSayWhy(blind)],
    ["a Settings row shown to every account", !rowOnlyWhereItAnswers(everyone)],
  ];
  for (const [what, ok] of caught) console.log(`${ok ? "PASS" : "FAIL"} red arm: ${what} is caught`);
  process.exit(caught.every(([, ok]) => ok) && fails.length === 0 ? 0 : 1);
}

if (fails.length) { console.log(`\n${fails.length} FAILED`); process.exit(1); }
console.log("\nall passed");
