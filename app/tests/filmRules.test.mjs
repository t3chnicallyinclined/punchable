// Unit test for rules/filmRules.ts (Film it, SPEC 2026-11-03 §10 "takeRules" and "takeWords").
//   node --experimental-strip-types --no-warnings tests/filmRules.test.mjs          the suite
//   node --experimental-strip-types --no-warnings tests/filmRules.test.mjs --red    the red arm: the same checks run against
//                                                                                 a lenient machine and cutter must FAIL
import * as F from "../src/rules/filmRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const W = (text, s, step = 0.3) => text.split(" ").map((w, i) => ({ w, s: s + i * step, e: s + i * step + step * 0.8 }));
const rec = (o = {}) => ({ id: "t1", kind: "film", file: "2026-11-03T20-00-00-000Z-ab12.mov", recorded_at: "2026-11-03T20:00:00.000Z", duration_s: 48, state: "filed", attempts: 0, next_at: 0, bit: { id: "j1", title: "Self-checkout" }, filed_on: { id: "j1", title: "Self-checkout" }, text: "one two three", ...o });

/** The rules under test: a second Keep while saving is a no-op (the machine), and no caption over 4 words or 1.6 s
 *  (the cutter). */
function machineIsStrict(next) {
  return next("saving", "keep") === "saving" && next("done", "keep") === "done" && next("idle", "stop") === "idle" && next("recording", "keep") === "recording";
}
function cutterIsStrict(cuesOf) {
  const many = W("a b c d e f g h i j", 0, 0.1);              // ten words in a second: the word cap must cut
  const slow = W("a b c", 0, 2);                               // three words two seconds apart: the time cap must cut
  return cuesOf(many, []).every((c) => c.text.split(" ").length <= F.CUE_WORDS) && cuesOf(slow, []).every((c) => c.e - c.s <= F.CUE_S + 1e-9 || c.text.split(" ").length === 1);
}

if (process.argv.includes("--red")) {
  const lenientNext = (s, a) => (a === "keep" ? "saving" : F.nextFilmState(s, a));
  const lenientCues = (words) => (words.length ? [{ text: words.map((w) => w.w).join(" "), s: words[0].s, e: words[words.length - 1].e }] : []);
  const caught = !machineIsStrict(lenientNext) && !cutterIsStrict(lenientCues);
  console.log(`${caught ? "PASS" : "FAIL"} red arm: a machine that keeps again while saving, and a cutter with no caps, are both caught`);
  const real = machineIsStrict(F.nextFilmState) && cutterIsStrict(F.cuesOf);
  console.log(`${real ? "PASS" : "FAIL"} the real machine and cutter pass the same checks`);
  process.exit(caught && real ? 0 : 1);
}

// ---- the state machine
const walk = (actions, from = "idle") => actions.reduce((s, a) => F.nextFilmState(s, a), from);
check(walk(["arm", "go", "stop", "keep", "kept"]) === "done", "idle → countdown → recording → review → saving → done");
check(walk(["arm", "cancel"]) === "idle" && walk(["arm", "go", "ended"]) === "review", "Close during the count goes back to idle; a take that ends on its own (a call) lands on the review");
check(walk(["arm", "go", "stop", "redo"]) === "idle" && walk(["arm", "go", "stop", "keep", "failed"]) === "review", "Redo is a fresh take; a failed save is the review again (the take is still here)");
check(machineIsStrict(F.nextFilmState), "a second Keep while saving (or after) is a no-op; stop while idle and keep while recording are no-ops");
check(walk(["go"]) === "idle" && walk(["stop"], "countdown") === "countdown", "go without a countdown, stop during the count: no-ops");

// ---- the disc's reason and the status words
check(F.discReason({ camera: true, mic: true, freeBytes: null }) === null && F.discReason({ camera: false, mic: false, freeBytes: 0 }) === "no camera" && F.discReason({ camera: true, mic: false, freeBytes: 0 }) === "no mic", "the first reason that applies: camera, mic, room");
check(F.discReason({ camera: true, mic: true, freeBytes: F.LOW_STORAGE_BYTES - 1 }) === "no room" && F.discReason({ camera: true, mic: true, freeBytes: F.LOW_STORAGE_BYTES }) === null, "under 500 MB free is no room");
check(F.readyStatus(null) === "Ready" && F.readyStatus("no room") === "Ready · low storage" && F.readyStatus(null, true) === "Starting…", "Ready, with the one qualifier he can fix; Starting… during the count");
check(F.cutShortOf(false, false) === "call" && F.cutShortOf(true, true) === "background" && F.cutShortOf(true, false) === null, "a file that came back without our stop = the session was taken (a call); our stop from the background = cut short");
check(F.cutShortOf(false, true) === "background" && F.cutShortOf(false, false, 600000) === null && F.cutShortOf(false, false, 598000) === "call" && F.cutShortOf(true, false, 600000) === null, "the lock's own stop landing first is still the background, not a call; the ten-minute cap is not cut short");
check(F.reviewStatus(48400, null) === "0:48" && F.reviewStatus(31000, "call") === "0:31 · cut short by a call" && F.reviewStatus(31000, "background") === "0:31 · cut short", "the review's one line");
check(F.reviewStatus(48000, null, 112) === "0:48 · 112 words" && F.reviewStatus(48000, null, 0) === "0:48 · no words" && F.reviewStatus(48000, null, 1) === "0:48 · 1 word", "with the words once they exist");
check(F.leaveMessage(48400) === "0:48 recorded.", "the Leave sheet's message");

// ---- the take's row
check(F.wordTotal(rec({ words: W("a b c d", 0) })) === 4 && F.wordTotal(rec({ words: null, text: "five words in a row" })) === 5 && F.wordTotal(rec({ state: "no_speech", text: "" })) === 0 && F.wordTotal(rec({ state: "queued", text: null })) === null, "the word count: the timed words, else the text, zero for no speech, unknown before");
let row = F.takeRow(rec({ words: W("one two three", 0) }), "On Self-checkout");
check(row.label === "Filmed · Nov 3 · 0:48" && row.sub === "3 words", `a kept take: ${row.label} / ${row.sub}`);
row = F.takeRow(rec({ state: "transcribing", text: null, cut_short: "call", duration_s: 31 }), "Transcribing…");
check(row.label === "Filmed · Nov 3 · 0:31" && row.sub === "cut short by a call · Transcribing…", `on its way: ${row.sub}`);
row = F.takeRow(rec({ state: "no_speech", text: "", words: [] }), "No speech heard");
check(row.sub === "no words", "no speech reads no words");
row = F.takeRow(rec({ state: "words", attempts: 2 }), "Waiting for a connection");
check(row.sub === "Waiting for a connection", "offline: the queue's words (the words are on the phone, not yet up)");
check(F.filmedCount([rec(), rec({ id: "t2" }), rec({ id: "m", kind: "memo" }), rec({ id: "t3", bit: { id: "j2", title: "x" } })], "j1") === 2, "filmed N counts this bit's takes on the phone");

// ---- the .srt sidecar
check(cutterIsStrict(F.cuesOf), "no caption over 4 words or 1.6 s");
const script = ["Self-checkout is the store hiring you for free.", "I scanned forty things and nobody clapped."];
const said = W("self checkout is the store hiring you for free i scanned forty things and nobody clapped", 0, 0.25);
const cues = F.cuesOf(said, script);
check(cues.some((c) => /for free$/.test(c.text)) && !cues.some((c) => /free i/.test(c.text)), "a caption breaks where his line ends (…for free | I scanned…), never across it");
check(cues.every((c, i) => i === 0 || c.s >= cues[i - 1].e - 1e-9) && cues.every((c) => c.e >= c.s), "captions never overlap and never run backwards");
const back = F.cuesOf([{ w: "a", s: 1, e: 1.5 }, { w: "b", s: 0.2, e: 0.4 }, { w: "c", s: 1.4, e: 1.9 }], []);
check(back.length === 1 && back[0].text === "a b c" && back[0].s === 1 && back[0].e === 1.9, "a word timed before the one before it is pushed after it, never dropped");
check(F.cuesOf([], []).length === 0 && F.srtOf([], []) === "", "no words: no captions, an empty sidecar");
check(F.srtTime(0) === "00:00:00,000" && F.srtTime(1.2) === "00:00:01,200" && F.srtTime(3661.005) === "01:01:01,005", "SubRip time stamps");
const srt = F.srtOf(W("one two three four five", 0, 0.4), []);
check(srt.startsWith("1\n00:00:00,000 --> 00:00:01,520\none two three four\n\n2\n00:00:01,600 --> 00:00:01,920\nfive\n") && srt.endsWith("five\n\n"), `the sidecar's shape:\n${srt}`);
check(F.cuesOf(W("a b c d e f g h i", 0, 0.1), []).map((c) => c.text.split(" ").length).join() === "3,3,3" && F.cuesOf(W("a b c d e", 0, 0.1), []).map((c) => c.text.split(" ").length).join() === "3,2", "a run over the cap splits into near-equal parts (3+3+3, 3+2), never 4+4+1");
check(F.cuesOf(W("the store hiring you for free", 0, 0.1), ["for free"]).map((c) => c.text).join("|") === "the store hiring|you for free", "the line end wins over the word balance");
check(/00:00:00,000 --> 00:00:00,300/.test(F.srtOf([{ w: "hi", s: 0, e: 0.05 }], [])), "a cue is shown at least 0.3 s");
check(F.srtNameOf("2026-11-03T20-00-00-000Z-ab12.mov") === "2026-11-03T20-00-00-000Z-ab12.srt", "the sidecar's name");
check(F.lineEnds(["Hello, world!", "a"]).has("hello world") && F.lineEnds(["a"]).has("a") && F.lineEnds([""]).size === 0, "line ends: the last two words, folded");

if (fails.length) { console.log(`${fails.length} FAILED`); process.exit(1); }
console.log("ALL PASS");
