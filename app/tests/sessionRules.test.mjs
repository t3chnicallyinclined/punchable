// Unit test for rules/sessionRules.ts and the cut rules in rules/bitLines.ts (Film it, SPEC 2026-11-03 D10, D11, §2.5,
// §2.6): Keep / Skip assembly of whole-line hunks, the hook as line 0, and which version the page, the stepper and the
// camera use. Every joke here is invented.
//   node --experimental-strip-types --no-warnings tests/sessionRules.test.mjs          the suite
//   node --experimental-strip-types --no-warnings tests/sessionRules.test.mjs --red    the red arm: a lenient assembler (it
//                                                                                     applies skipped hunks) and a lenient
//                                                                                     cut pick (the newest of any cut) must FAIL
import * as S from "../src/rules/sessionRules.ts";
import * as B from "../src/rules/bitLines.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const BASE = ["Self-checkout is the store hiring you for free.", "I scanned forty things and nobody clapped.", "At my job I at least get a chair.", "Unexpected item in the bagging area: my dignity."];
const HUNKS = [
  { op: "change", line_no: 1, was: BASE[0], now: "You know what self-checkout is? It's the store hiring you. For free." },
  { op: "change", line_no: 2, was: BASE[1], now: "You scan forty things. Nobody claps." },
  { op: "cut", line_no: 3, was: BASE[2], now: "" },
  { op: "add", line_no: 4, was: "", now: "Yeah. It's my dignity." },
  { op: "add", line_no: 0, was: "", now: "Stop scanning your own groceries." },
];
// the version list as GET /api/jokes/<id> sends it: v1 stage, v2 rant (same day as v3, ordered by the history), v3 stage
const READ = {
  versions: [
    { date: "2026-11-01", units: [{ id: "unit:a", text: "stage one", source: "app" }] },
    { date: "2026-11-03", units: [{ id: "unit:c", text: "stage two", source: "app" }, { id: "unit:b", text: "rant one", source: "rant_session" }] },
  ],
  history: [{ index: 10, target: "node", action: "add", id: "unit:b" }, { index: 12, target: "node", action: "add", id: "unit:c" }],
};

/** The rules under test: a skipped hunk changes nothing, and the page opens on the newest STAGE version. */
function assemblerIsStrict(apply) {
  return eq(apply(BASE, HUNKS, [false, true, false, false, false]), [BASE[0], "You scan forty things. Nobody claps.", BASE[2], BASE[3]])
    && eq(apply(BASE, HUNKS, []), BASE);
}
function pickIsStrict(home) {
  const vs = [{ cut: "rant" }, { cut: "stage" }, { cut: "rant" }];
  return home(vs) === 1 && home([{ cut: "stage" }]) === 0;
}

if (process.argv.includes("--red")) {
  const lenientApply = (base, hunks) => S.applyHunks(base, hunks, hunks.map(() => true));
  const lenientHome = () => 0;
  const caught = !assemblerIsStrict(lenientApply) && !pickIsStrict(lenientHome);
  console.log(`${caught ? "PASS" : "FAIL"} red arm: an assembler that applies skipped hunks, and a page that opens on the newest of any cut, are both caught`);
  const real = assemblerIsStrict(S.applyHunks) && pickIsStrict(B.homeIndex);
  console.log(`${real ? "PASS" : "FAIL"} the real assembler and pick pass the same checks`);
  process.exit(caught && real ? 0 : 1);
}

// ---- Keep / Skip assembly
check(assemblerIsStrict(S.applyHunks), "a skipped or undecided hunk changes nothing; no marks = the base");
check(eq(S.applyHunks(BASE, HUNKS, [true, true, true, true, true]), [
  "Stop scanning your own groceries.",
  "You know what self-checkout is? It's the store hiring you. For free.",
  "You scan forty things. Nobody claps.",
  "Unexpected item in the bagging area: my dignity.",
  "Yeah. It's my dignity.",
]), "every hunk kept: the add at 0 first, changes in place, the cut gone, the tag after the punch");
check(eq(S.applyHunks(BASE, [{ op: "add", line_no: 2, now: "a" }, { op: "add", line_no: 2, now: "b" }], [true, true]), [BASE[0], BASE[1], "a", "b", BASE[2], BASE[3]]), "two adds after one line keep Claude's order");
check(eq(S.applyHunks(BASE, [{ op: "change", line_no: 9, now: "x" }, { op: "add", line_no: -1, now: "y" }, { op: "change", line_no: 1, now: "  " }, { op: "zap", line_no: 1, now: "z" }], [true, true, true, true]), BASE), "a hunk that does not fit the lines is left out");
check(eq(S.applyHunks(BASE, [{ op: "change", line_no: 2, now: "first" }, { op: "cut", line_no: 2 }], [true, true]), [BASE[0], "first", BASE[2], BASE[3]]), "two kept hunks on one line: the first wins");
check(!S.canSave(BASE, HUNKS, [false, undefined]) && S.canSave(BASE, HUNKS, [false, true]), "Save rant cut is not shown until a change is kept");
check(!S.canSave(["only"], [{ op: "cut", line_no: 1, was: "only", now: "" }], [true]), "a save that would leave no line is not offered");
check(S.hunkLabel(HUNKS[1], 1, 5) === "Change 2 of 5. Removed: I scanned forty things and nobody clapped.. Added: You scan forty things. Nobody claps."
  && S.hunkLabel(HUNKS[2], 2, 5).endsWith("Removed: At my job I at least get a chair.") && S.hunkLabel(HUNKS[4], 4, 5).includes("Added at the start"), "VoiceOver says what is removed and added");

// ---- the hook: one hunk through the same path
check(eq(S.applyHunks(BASE, [S.hookHunk("  I got fired today. From a store I don't work at. ")], [true]), ["I got fired today. From a store I don't work at.", ...BASE]), "Use this one = a line before line 1");
check(S.canShowOthers(1) && !S.canShowOthers(2), "Show others asks once");
check(S.savedWord(9, "rant") === "Saved as v9 · rant" && S.savedWord(9, "stage") === "Saved as v9", "the snack says the cut");

// ---- where a session stands
check(S.jobView({ status: "queued", job: { id: "job:rant-1" }, poll: "/api/jobs/job:rant-1" }).state === "waiting", "the 202 waits");
check(S.jobView({ id: "job:1", status: "claimed" }).job === "job:1", "a claimed job waits with its id");
check(eq(S.jobView({ id: "job:1", status: "done", result: { status: "ok", hunks: [] } }), { state: "done", result: { status: "ok", hunks: [] } }), "a finished poll carries the result");
check(S.jobView({ cached: true, status: "ok", hunks: [1] }).result.hunks.length === 1, "a cached 200 is the result");
check(S.jobView({ id: "j", status: "failed", error: "boom" }).error === "boom" && S.jobView(null).state === "failed", "a failed job says why");
check(S.emptyWord("rant", "none") === "Claude has nothing to change here." && S.emptyWord("hook", "ok") === null && S.emptyWord("rant", "provider_error", "busy").includes("busy"), "the one line when there is nothing to keep");

// ---- the cuts
check(B.cutOf("rant") === "rant" && B.cutOf("rant_session") === "rant" && B.cutOf("rant_hook") === "rant", "rant sources are the rant cut");
check(["app", "hook", "draft", "rants", "rant-x", "", null, undefined, 3].every((s) => B.cutOf(s) === "stage"), "anything else is the stage cut (the server's cut_of agrees: api-dev/tests/test_rant_api.py)");
check(B.sourceFor("stage", "write") === undefined && B.sourceFor("rant", "write") === "rant" && B.sourceFor("rant", "rant") === "rant_session"
  && B.sourceFor("stage", "rant") === "rant_session" && B.sourceFor("stage", "hook") === "hook" && B.sourceFor("rant", "hook") === "rant_hook", "a write keeps its cut; a session names itself");
check(["rant_session", "rant_hook", "hook", "rant"].every((s) => /^[a-z_]{1,24}$/.test(s)), "every source passes the versions route's rule");
const vs = B.versionsOf(READ);
check(eq(vs.map((v) => [v.label, v.key, v.cut]), [["v3", "unit:c", "stage"], ["v2", "unit:b", "rant"], ["v1", "unit:a", "stage"]]), "two versions on one day are ordered by the decision that added them, each with its cut");
check(pickIsStrict(B.homeIndex) && B.homeIndex(vs) === 0 && B.homeIndex([{ cut: "rant" }, { cut: "stage" }]) === 1, "the page shows the newest stage version");
check(B.isNewestOfCut([{ cut: "rant" }, { cut: "stage" }, { cut: "stage" }], 0) && B.isNewestOfCut([{ cut: "rant" }, { cut: "stage" }, { cut: "stage" }], 1)
  && !B.isNewestOfCut([{ cut: "rant" }, { cut: "stage" }, { cut: "stage" }], 2), "Write is live on the newest of each cut, not on an older one");
check(B.writeReason({ vi: 1, count: 3, online: true, newest: true }) === null && B.writeReason({ vi: 2, count: 3, online: true, newest: false }) === "Reading v1", "the reason under Write follows the cut");
const film = [{ key: "r2", cut: "rant" }, { key: "s2", cut: "stage" }, { key: "r1", cut: "rant" }];
check(B.filmIndex(film) === 0 && B.filmIndex(film, "s2") === 1 && B.filmIndex(film, "gone") === 0 && B.filmIndex([{ key: "s", cut: "stage" }]) === 0, "Film it opens on the newest rant cut, else stage; a chosen cut wins");
check(eq(B.cutChoices(vs).map((c) => c.label), ["Rant cut · v2 · Nov 3", "Stage cut · v3 · Nov 3"]) && B.cutChoices([{ key: "s", label: "v1", date: null, cut: "stage" }]).length === 1, "the cut picker lists the newest of each cut, rant first");
check(B.stepperLabel(1, 9, "2026-11-03", { cut: "rant", source: "rant_session" }) === "v8 of 9 · Nov 3 · rant · Make it a rant" && B.stepperLabel(0, 9, "2026-11-03", { cut: "stage", source: "app" }) === "v9 of 9 · Nov 3", "the stepper says the cut and the session");
check(B.stepperValue(0, 9, "2026-11-03", { cut: "rant", source: "rant_session" }) === "9 of 9, November 3, rant cut, from Make it a rant", "VoiceOver: Version 9 of 9, November 3, rant cut, from Make it a rant");

if (fails.length) { console.log(`\n${fails.length} FAILED`); process.exit(1); }
console.log("\nall sessionRules checks passed");
