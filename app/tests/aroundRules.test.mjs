// Unit test for rules/aroundRules.ts (Around this bit, In these sets, the Add to a set picker; SPEC 2026-11-16 §3.1,
// §4.5, Slice 1). Every bit here is invented.
//   node --experimental-strip-types --no-warnings tests/aroundRules.test.mjs          the suite
//   node --experimental-strip-types --no-warnings tests/aroundRules.test.mjs --red    the red arm: a planted rule that
//       sends every "Not connected" to the links store (so a stored edge would stay standing and come straight back)
//       must FAIL the same check the real rule passes
//   node --experimental-strip-types --no-warnings tests/aroundRules.test.mjs --red=inline   the red arm for the cards
//       drawn on the page: a planted rule that draws every card of every group (headers for empty groups too, no cap)
//       must FAIL the check the real rule passes
import * as A from "../src/rules/aroundRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const BIT = "joke:fridge";
const LINES = [
  "My fridge has wifi now.",
  "It sends me a notification when the door is open.",
  "I know the door is open. I am standing in it.",
  "That is not a smart fridge, that is a roommate.",
];
const item = (o) => ({ id: `${o.g || "same_idea"}:${o.ref}`, kind: "bit", title: "Smart doorbell", words: ["My doorbell shows me who is at the door.", "It is always me."], meta: "never on stage", why: "Sounds like this bit", source: "ai", edge: null, line: null, does: ["merge", "add", "dismiss"], ...o });
const G = { same: { key: "same_idea", title: "Same idea" }, cb: { key: "callbacks", title: "Callbacks" }, stage: { key: "on_stage", title: "On stage" }, loose: { key: "loose", title: "Loose scraps" }, shape: { key: "same_shape", title: "Same shape" } };

const computed = item({ ref: "joke:doorbell" });
const edged = item({ g: "callbacks", ref: "joke:roommate", title: "Roommate", edge: "e:cb1", source: "ai", does: ["connect", "add", "dismiss"] });
const note = item({ g: "loose", ref: "joke:camera", kind: "note", attach: "at:1", title: "Note, 2026-10-30", words: ["The fridge has a camera inside."], does: ["add", "dismiss"] });
const moment = item({ g: "on_stage", ref: "perf:p9", kind: "moment", title: "Tuesday, Mar 4", words: ["my fridge has wifi now and it tells me the door is open"], connect: { kind: "performed_as", src: "perf:p9", dst: BIT }, does: ["link", "add", "dismiss"] });
const tag = item({ g: "callbacks", ref: "unit:t1", kind: "line", words: ["It labeled the salt."], connect: { kind: "tag_of", src: "unit:t1", dst: BIT }, does: ["connect", "add", "dismiss"] });
const said = item({ g: "on_stage", ref: "set:20260313_late", kind: "wording", title: "What you said, Friday", words: ["I know the door is open. I am IN it."], source: "his", line: { i: 2, text: LINES[2] }, does: ["use", "dismiss"] });

/** The rule under test: "Not connected" reaches the place that can make it true. An edge is rejected (the decision
 *  log), a filed note is taken off (the merges store), and only a card with neither is remembered as a pair. */
function dismissGoesToTheRightPlace(planFor) {
  const p = (it, g) => planFor(BIT, g, it, "dismiss", LINES);
  const e = p(edged, G.cb), n = p(note, G.loose), c = p(computed, G.same);
  return e.kind === "call" && e.call.path === "/api/decision" && eq(e.call.body, { target: "edge", id: "e:cb1", action: "reject" })
    && n.kind === "call" && n.call.path === "/api/merge/undo" && eq(n.call.body, { merge_id: "at:1" })
    && c.kind === "call" && c.call.path === "/api/links/dismiss" && eq(c.call.body, { a: BIT, b: "joke:doorbell", group: "same_idea" });
}

/** The rule under test for the page itself: each group that holds something, in the server's order, its first two
 *  cards in the server's order, "See all N" only when there are more, and no header for an empty group. */
function pageShowsTheFirstTwo(inlineGroups) {
  const many = [1, 2, 3, 4, 5].map((n) => item({ g: "callbacks", ref: `joke:cb${n}`, title: `Callback ${n}` }));
  const a = { joke: BIT, unit: null, sets: [], groups: [
    { ...G.same, items: [] }, { ...G.cb, items: many }, { ...G.stage, items: [moment, said] }, { ...G.loose, items: [note] }, { ...G.shape, items: [] },
  ] };
  const g = inlineGroups(a);
  return eq(g.map((x) => x.key), ["callbacks", "on_stage", "loose"])
    && eq(g[0].cards.map((c) => c.title), ["Callback 1", "Callback 2"]) && g[0].total === 5 && g[0].more === 5
    && eq(g[1].cards.map((c) => c.id), [moment.id, said.id]) && g[1].more === 0
    && g[2].cards.length === 1 && g[2].more === 0
    && inlineGroups(null).length === 0 && inlineGroups({ joke: BIT, unit: null, sets: [], groups: [{ ...G.same, items: [] }] }).length === 0;
}

if (process.argv.includes("--red=inline")) {
  // a planted rule: every group gets a header and every card is drawn. The page would be a wall between him and the rows below.
  const everything = (a) => (a?.groups || []).map((g) => ({ key: g.key, title: g.title, cards: g.items, total: g.items.length, more: 0 }));
  const caught = !pageShowsTheFirstTwo(everything);
  console.log(`${caught ? "PASS" : "FAIL"} red arm: a rule that draws every card of every group is caught (two cards a group, no empty headers)`);
  console.log(`${pageShowsTheFirstTwo(A.inlineGroups) ? "PASS" : "FAIL"} the real inlineGroups passes the same check`);
  process.exit(caught && pageShowsTheFirstTwo(A.inlineGroups) ? 0 : 1);
}

if (process.argv.includes("--red")) {
  // a planted rule: every dismissal is "remember the pair". The edge would still be in the graph and the card back.
  const lenient = (bit, group, it) => ({ kind: "call", call: { method: "POST", path: "/api/links/dismiss", body: { a: bit, b: it.ref, group: group.key } } });
  const caught = !dismissGoesToTheRightPlace(lenient);
  console.log(`${caught ? "PASS" : "FAIL"} red arm: a rule that only remembers pairs is caught (an edge card must reject its edge)`);
  console.log(`${dismissGoesToTheRightPlace(A.planFor) ? "PASS" : "FAIL"} the real planFor passes the same check`);
  process.exit(caught && dismissGoesToTheRightPlace(A.planFor) ? 0 : 1);
}

// ---- the page: grouping and counts
const around = { joke: BIT, unit: "unit:f3", groups: [
  { ...G.same, items: [computed] }, { ...G.cb, items: [edged, tag] }, { ...G.stage, items: [moment, said] }, { ...G.loose, items: [note] }, { ...G.shape, items: [] },
], sets: [{ board: "sb:1", title: "Friday late show", position: 2, count: 6, notes: [] }] };
check(eq(A.shownGroups(around).map((g) => g.key), ["same_idea", "callbacks", "on_stage", "loose"]), "an empty group has no header; the rest keep the server's order");
check(A.aroundCount(around) === 6 && A.aroundCount(null) === 0 && A.countMeta(6) === "6" && A.countMeta(0) === "0", "the row counts every card");
check(A.findItem(around, "callbacks:unit:t1")?.group.key === "callbacks" && A.findItem(around, "nope") === null && A.findItem(null, "x") === null, "a card is found by its id with its group");
check(A.setPlace(around.sets[0]) === "bit 2 of 6", "a set row says where the bit sits");
check(A.setNotesLine({ notes: [" wifi ", "", "standing in it"] }) === "wifi · standing in it" && A.setNotesLine({ notes: [] }) === "" && A.setNotesLine({}) === "", "his notes on one line; none is nothing (never 'no notes yet')");

// ---- the page: what is drawn inline under the document
check(pageShowsTheFirstTwo(A.inlineGroups), "on the page: groups in the server's order, the first two cards of each, See all only when there are more, no header for an empty group");
check(A.INLINE_CAP === 2 && A.inlineGroups(around, 1)[1].cards.length === 1 && A.inlineGroups(around, 1)[1].more === 2 && A.inlineGroups(around, 0)[1].cards.length === 1, "the cap is two; a smaller cap still shows one card, never none");
check(A.seeAllLabel(5) === "See all 5", "See all names the count");
check(A.findGroup(around, "callbacks")?.items.length === 2 && A.findGroup(around, "same_shape") === null && A.findGroup(around, "nope") === null && A.findGroup(null, "callbacks") === null, "the sheet behind See all finds a group by its key; an empty or unknown one is none");
check(eq(A.inlineAction(edged), { key: "connect", label: "Connect", direct: true }) && eq(A.inlineAction(moment), { key: "link", label: "This tape is this bit", direct: true }), "a card's one button on the page is its first action");
check(eq(A.inlineAction(computed), { key: "merge", label: "Merge into this bit", direct: true }) && eq(A.inlineAction(note), { key: "add", label: "Add this line", direct: true }), "a one-line note adds its line in one tap; a merge is the first action of a same-idea bit (the page still asks once)");
check(eq(A.inlineAction(item({ ref: "joke:two", does: ["add", "dismiss"] })), { key: "add", label: "Add this line", direct: false }), "a card with two lines to choose from is not a one-tap add: the sheet has the picker");
check(A.inlineAction(item({ ref: "joke:q", does: ["dismiss"] })) === null && A.inlineAction(item({ ref: "joke:q2", does: [] })) === null, "Not connected is never the button on a card; a card with nothing else has none");

// ---- the action per kind
check(dismissGoesToTheRightPlace(A.planFor), "Not connected: an edge is rejected, a filed note is taken off, a computed card is remembered as a pair");
check(eq(A.actionsFor(computed).map((a) => a.label), ["Merge into this bit", "Add this line", "Not connected"]), "a same-idea bit: merge, add a line, not connected");
check(eq(A.actionsFor(edged).map((a) => a.label), ["Connect", "Add this line", "Not connected"]) && A.actionsFor(edged)[2].quiet && !A.actionsFor(edged)[0].quiet, "an AI tie: Connect (never Yes), and Not connected is the quiet one");
check(eq(A.actionsFor(note).map((a) => a.label), ["Add this line", "Not this bit"]), "a filed note says Not this bit");
check(eq(A.actionsFor(moment).map((a) => a.label), ["This tape is this bit", "Add this line", "Not connected"]), "a taped moment is said to be this bit");
check(eq(A.actionsFor(said).map((a) => a.label), ["Use this wording", "Not connected"]), "what he said on stage offers its wording");
check(eq(A.actionsFor(item({ ref: "joke:x", does: ["teleport", "dismiss"] })).map((a) => a.key), ["dismiss"]), "an action this build does not know is not a button");
check(A.writesLine("add") && A.writesLine("use") && !A.writesLine("merge") && !A.writesLine("dismiss"), "only Add this line and Use this wording write a line");

const call = (it, g, action, pick) => A.planFor(BIT, g, it, action, LINES, pick);
check(eq(call(computed, G.same, "merge").call, { method: "POST", path: "/api/merge", body: { main: BIT, others: ["joke:doorbell"], reason: "merged from Around this bit (Same idea)" } }), "Merge into this bit: this bit is the main");
check(eq(call(edged, G.cb, "connect").call.body, { target: "edge", id: "e:cb1", action: "confirm" }), "Connect on a stored edge confirms it");
check(eq(call(tag, G.cb, "connect").call.body, { target: "edge", action: "add", after: { kind: "tag_of", src: "unit:t1", dst: BIT }, reason: "Callbacks" }), "Connect on a computed card adds the edge, with the group's name as the reason");
check(eq(call(moment, G.stage, "link").call.body, { target: "edge", action: "add", after: { kind: "performed_as", src: "perf:p9", dst: BIT }, reason: "On stage" }), "Link adds the performed_as edge");
check(call(item({ ref: "joke:x", does: ["connect"] }), G.shape, "connect").kind === "none", "nothing to connect is no call");
check(eq(call(item({ g: "loose", ref: "joke:icemaker", kind: "idea" }), G.loose, "attach").call, { method: "POST", path: "/api/merge/attach", body: { note: "joke:icemaker", into: BIT } }), "File under this bit attaches the note");
check(eq(call(moment, G.stage, "dismiss").call.body, { a: BIT, b: "perf:p9", group: "on_stage" }) && eq(call(said, G.stage, "dismiss").call.body, { a: BIT, b: "set:20260313_late", group: "on_stage" }), "a moment and a night's wording are remembered as pairs");

// ---- pulling a line in, using a wording (lines have no ids: the anchor is found by its words)
check(A.anchorIndex(LINES, said) === 2 && A.anchorIndex(LINES, computed) === null, "the anchor is the server's line while it still says the same");
check(A.anchorIndex(["A new first line.", ...LINES], said) === 3, "after an edit above it, the anchor is found by its words");
check(A.anchorIndex(LINES, { line: { i: 2, text: "A line that is gone." } }) === null, "a line that changed is no anchor");
const pulled = call(computed, G.same, "add", 1);
check(pulled.kind === "version" && eq(pulled.lines, [...LINES, "It is always me."]) && pulled.reason === "pulled in from Same idea", "Add this line: the picked line goes at the end when the card has no line");
check(eq(pulled.edit, { op: "insert", at: null, words: "It is always me." }), "the same add as one edit of the document (so the gaps between his ideas stay)");
const anchored = A.pullLine(LINES, { ...computed, line: { i: 0, text: LINES[0] } }, 0);
check(eq(anchored, [LINES[0], "My doorbell shows me who is at the door.", ...LINES.slice(1)]), "with an anchor the line goes right after it");
check(call(item({ ref: "joke:same", words: ["my fridge has WIFI now"] }), G.same, "add", 0).kind === "none", "a line the bit already says is not added twice");
const swapped = call(said, G.stage, "use", 0);
check(swapped.kind === "version" && eq(swapped.lines, [LINES[0], LINES[1], "I know the door is open. I am IN it.", LINES[3]]) && swapped.lines.length === LINES.length, "Use this wording replaces exactly the line it belongs to");
check(eq(swapped.edit, { op: "replace", at: 2, words: "I know the door is open. I am IN it." }) && eq(call({ ...said, line: { i: 2, text: "gone" } }, G.stage, "use", 0).edit, { op: "insert", at: null, words: "I know the door is open. I am IN it." }), "the same swap as one edit; when its line changed, an insert at the end");
check(eq(A.swapWording(LINES, { ...said, line: { i: 2, text: "gone" } }, 0), [...LINES, "I know the door is open. I am IN it."]), "when that line changed, the wording is added at the end, nothing is replaced");
check(A.pullLine(LINES, computed, 9)?.length === LINES.length + 1 && A.pullLine(LINES, item({ ref: "joke:e", words: [] }), 0) === null, "a pick past the end falls back to the first line; no words is nothing");

// ---- every action has its Undo
const u = (c, answer, it = computed) => A.undoFor(BIT, it, c, answer);
check(eq(u(call(edged, G.cb, "dismiss").call, { ok: true, indexes: [41] }), { method: "POST", path: "/api/undo", body: { indexes: [41] } }), "a rejected edge is undone through the decisions");
check(eq(u(call(computed, G.same, "dismiss").call, { ok: true, id: "lk:abc" }), { method: "POST", path: "/api/links/undismiss", body: { id: "lk:abc" } }), "a remembered pair is undismissed by its id");
check(eq(u(call(computed, G.same, "merge").call, { merge_id: "mg:1" }), { method: "POST", path: "/api/merge/undo", body: { merge_id: "mg:1" } }), "a merge is undone by its id");
check(eq(u(call(note, G.loose, "dismiss").call, { ok: true }, note), { method: "POST", path: "/api/merge/attach", body: { note: "joke:camera", into: BIT } }), "a note taken off goes back under the bit");
check(u(call(edged, G.cb, "connect").call, { ok: true }) === null && u({ method: "POST", path: "/api/other", body: {} }, {}) === null, "no handle, no Undo offered");
check(A.doneWord(computed, "merge") === "Smart doorbell is now part of this bit" && A.doneWord(computed, "dismiss") === "Not connected" && A.doneWord(note, "dismiss") === "Taken off this bit" && A.doneWord(said, "use") === "Line changed", "the snack says what happened");

// ---- Add to a set: ticks staged, written on Done
const boards = [{ id: "sb:1", title: "Friday late show", n: 6, runtime: "9:40" }, { id: "sb:2", title: "Smart home stuff", n: 1 }, { id: "sb:3", title: "Landlords" }];
const was = new Set(["sb:1"]);
check(eq(A.pickerRows(boards, was), [
  { id: "sb:1", title: "Friday late show", sub: "6 bits · 9:40", on: true },
  { id: "sb:2", title: "Smart home stuff", sub: "1 bit", on: false },
  { id: "sb:3", title: "Landlords", sub: "", on: false },
]), "the rows show a tick on the sets the bit is already in");
let now = A.toggled(A.toggled(was, "sb:2"), "sb:3");
check(eq(A.stagedDiff(was, now), { add: ["sb:2", "sb:3"], remove: [] }), "two ticks: two adds");
now = A.toggled(now, "sb:1");
check(eq(A.stagedDiff(was, now), { add: ["sb:2", "sb:3"], remove: ["sb:1"] }), "an untick is a remove");
check(eq(A.stagedDiff(was, A.toggled(A.toggled(was, "sb:2"), "sb:2")), { add: [], remove: [] }) && was.size === 1, "a tick and an untick is nothing; the starting set is never changed");
check(!A.hasChanges({ add: [], remove: [] }, "   ") && A.hasChanges({ add: [], remove: [] }, " Late set ") && A.hasChanges({ add: ["sb:2"], remove: [] }, ""), "Done is live only with something to write");
check(eq(A.pickerCalls(BIT, { add: ["sb:2", "sb:3"], remove: ["sb:1"] }, " Late set "), [
  { method: "POST", path: "/api/boards", body: { title: "Late set", joke_ids: [BIT] } },
  { method: "POST", path: "/api/boards/sb%3A2/items", body: { joke_id: BIT } },
  { method: "POST", path: "/api/boards/sb%3A3/items", body: { joke_id: BIT } },
  { method: "PATCH", path: "/api/boards/sb%3A1", body: { remove: { joke_id: BIT } } },
]), "Done: the new set first (made with the bit in it), then each add, then each remove");
check(A.pickerCalls(BIT, { add: [], remove: [] }, "").length === 0, "nothing staged is no call");
check(eq(A.pickerCalls(BIT, { add: ["sb:2"], remove: ["sb:1"] }, "", " the wifi line "), [
  { method: "POST", path: "/api/boards/sb%3A2/items", body: { joke_id: BIT, note: "the wifi line" } },
  { method: "PATCH", path: "/api/boards/sb%3A1", body: { remove: { joke_id: BIT } } },
]), "words taken elsewhere (Slice 3) go as the note on each add, never on a remove");
check(A.newSetTitle("  x  ") === "x" && A.newSetTitle("y".repeat(300)).length === 200, "a new set's name is trimmed to the route's limit");
check(eq(A.pickerUndo(BIT, { added: ["sb:2"], removed: ["sb:1"], created: ["sb:9"] }, { "sb:1": 2 }), [
  { method: "PATCH", path: "/api/boards/sb%3A2", body: { remove: { joke_id: BIT } } },
  { method: "PATCH", path: "/api/boards/sb%3A9", body: { delete: true } },
  { method: "PATCH", path: "/api/boards/sb%3A1", body: { add: { joke_id: BIT, at: 1 } } },
]), "Undo: off what it was added to, the set made here deleted, back where it was at the place it sat");
check(eq(A.pickerUndo(BIT, { added: [], removed: ["sb:1"], created: [] }), [{ method: "PATCH", path: "/api/boards/sb%3A1", body: { add: { joke_id: BIT } } }]), "with no known place it goes back at the end");
const titles = { "sb:1": "Friday late show", "sb:2": "Smart home stuff" };
check(A.pickerSnack({ added: ["sb:2", "sb:3"], removed: [], created: [] }, titles) === "Added to 2 sets", "Added to 2 sets");
check(A.pickerSnack({ added: ["sb:2"], removed: [], created: [] }, titles) === "Added to Smart home stuff", "one set is named");
check(A.pickerSnack({ added: [], removed: ["sb:1"], created: [] }, titles) === "Removed from Friday late show" && A.pickerSnack({ added: [], removed: ["sb:1", "sb:2"], created: [] }, titles) === "Removed from 2 sets", "removes say so");
check(A.pickerSnack({ added: ["sb:2"], removed: ["sb:1"], created: ["sb:9"] }, titles) === "Added to 2 sets · removed from 1 set", "both in one snack; a set made here counts as an add");
check(A.pickerSnack({ added: [], removed: [], created: [] }) === "" && A.pickerSnack({ added: ["sb:7"], removed: [], created: [] }) === "Added to 1 set", "nothing written is no snack; an unnamed set is counted");

if (fails.length) { console.log(`\n${fails.length} FAILED`); process.exit(1); }
console.log("\nall aroundRules checks passed");
