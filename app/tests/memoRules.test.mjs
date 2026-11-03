// Unit test for src/rules/memoRules.ts: voice memos and taped sets transcribed on the phone (SPEC 2026-10-31).
//   node --experimental-strip-types --no-warnings tests/memoRules.test.mjs
import * as M from "../src/rules/memoRules.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };
const fin = (transcript, s, e) => ({ isFinal: true, results: [{ transcript, segments: [{ startTimeMillis: s, endTimeMillis: e }] }] });
const rec = (o = {}) => ({ id: "r1", kind: "memo", file: "2026-10-31T01-02-03-000Z.m4a", recorded_at: "2026-10-31T01:02:03.000Z", duration_s: 42, state: "queued", attempts: 0, next_at: 0, ...o });

// ---- the transcript from the recognizer's finals
let a = M.emptyAcc();
a = M.addFinal(a, { isFinal: false, results: [{ transcript: "partial", segments: [] }] });
check(a.segs.length === 0, "a partial result is ignored (interimResults off, only finals count)");
a = M.addFinal(a, fin("So my landlord", 500, 2100));
a = M.addFinal(a, fin(" texts me at 3am", 2400, 4200));
check(a.segs.length === 2 && a.segs[1].text === "texts me at 3am", "the second final's leading space is trimmed");
check(a.segs[0].start === 0.5 && a.segs[0].end === 2.1 && a.segs[1].start === 2.4, "milliseconds become seconds");
a = M.addFinal(a, fin(" texts me at 3am", 2400, 4200));
check(a.segs.length === 2, "a repeated last final is kept once");
a = M.addFinal(a, { isFinal: true, results: [] });
a = M.addFinal(a, fin("   ", 0, 0));
check(a.segs.length === 2, "an empty last final adds nothing");
a = M.addFinal(a, fin("and I said", 300, 1500));
check(a.segs[2].start >= a.segs[1].end - 0.0001 && a.segs[2].end > a.segs[2].start, "a final whose clock restarted is moved to follow the one before");
check(M.textOf(a.segs) === "So my landlord texts me at 3am and I said", "the text is the finals joined by one space");
check(Math.abs(M.progressOf(a, 42) - a.lastEnd / 42) < 1e-9 && M.progressOf(a, 0) === 0, "progress = the last end over the duration; no duration = 0");

const many = Array.from({ length: 12001 }, (_, i) => ({ text: `w${i}`, start: i, end: i + 0.5 }));
const capped = M.capSegments(many);
check(capped.length <= M.SEGMENTS_MAX && capped.length > 0, `12001 segments fit the API's ${M.SEGMENTS_MAX} (got ${capped.length})`);
check(capped[0].start === 0 && capped[capped.length - 1].end === 12000.5 && M.textOf(capped) === M.textOf(many), "merging keeps every word and the span");
check(M.capSegments(many.slice(0, 10)).length === 10, "under the cap nothing is merged");

// ---- how a run ended
check(M.outcomeOf(null, 3, false) === "done" && M.outcomeOf(null, 0, false) === "no_speech", "end with finals = done; end with none = no speech");
check(M.outcomeOf("aborted", 0, true) === "requeue" && M.outcomeOf(null, 0, true) === "requeue", "our own abort (background, watchdog) goes back in the queue, never 'no speech'");
check(M.outcomeOf("aborted", 0, false) === "failed", "an abort we did not cause fails (no retry loop)");
check(M.outcomeOf("not-allowed", 0, false) === "needs_permission", "permission off -> needs permission");
check(M.outcomeOf("service-not-allowed", 0, false) === "unavailable" && M.outcomeOf("on-device-unavailable", 0, false) === "unavailable", "no on-device recognizer -> unavailable (never a server fallback)");
check(M.outcomeOf("busy", 0, false) === "requeue" && M.outcomeOf("network", 0, false) === "requeue", "busy / speech daemon lost -> retried");
check(["audio-capture", "interrupted", "language-not-supported", "bad-grammar", "whatever"].every((e) => M.outcomeOf(e, 0, false) === "failed"), "audio-capture, interrupted, language, unknown -> failed");
check(M.outcomeOf("no-speech", 0, false) === "no_speech" && M.outcomeOf("nomatch", 2, false) === "done", "no-speech with no finals = no speech; with finals the words win");

// ---- the queue
const q = [rec({ id: "s", kind: "set", recorded_at: "2026-10-30T00:00:00Z" }), rec({ id: "m2", recorded_at: "2026-10-31T02:00:00Z" }), rec({ id: "m1", recorded_at: "2026-10-31T01:00:00Z" }), rec({ id: "done", state: "filed" })];
check(M.nextToTranscribe(q)?.id === "m1", "memos before sets, oldest first");
check(M.nextToTranscribe([...q, rec({ id: "x", state: "transcribing" })]) === null, "one transcription at a time");
check(M.nextToTranscribe([rec({ state: "words" })]) === null, "nothing queued, nothing picked");
const w = [rec({ id: "a", state: "words", next_at: 50 }), rec({ id: "b", state: "words", next_at: 10 }), rec({ id: "c", state: "words", next_at: 999 })];
check(M.dueToSend(w, 100).map((r) => r.id).join() === "b,a", "words due to send, by when they are due");
const f1 = M.afterSendFailure(rec({ state: "sending", text: "hi" }), 1000, "No connection");
check(f1.state === "words" && f1.attempts === 1 && f1.next_at === 6000 && f1.text === "hi", "a failed send keeps the words and backs off (5 s first)");
check(M.afterRefusal(rec({ state: "sending" }), "400 bad").state === "failed", "a refusal fails the row (audio kept)");
const rc = M.recover(rec({ state: "transcribing", progress: 0.6 }));
check(rc.state === "queued" && rc.paused === true && rc.peak === 0.6 && rc.progress === 0, "a run killed mid-way is queued again, paused, remembering how far it got");
check(M.recover(rec({ state: "sending" })).state === "words", "a send killed mid-way is sent again");
check(M.retry(rec({ state: "failed", text: "words here" })).state === "words", "Try again with the words here sends them");
check(M.retry(rec({ state: "unavailable" })).state === "queued", "Try again without words transcribes again");

check(M.nextToTranscribe([rec({ id: "later", next_at: 5000 })], 1000) === null && M.nextToTranscribe([rec({ id: "later", next_at: 5000 })], 6000)?.id === "later", "a row backing off waits for its next_at");
const rq = M.afterRequeue(rec({ state: "transcribing", progress: 0.4, attempts: 0 }), 1000, null, "busy");
check(rq.state === "queued" && rq.attempts === 1 && rq.next_at === 6000 && rq.peak === 0.4 && !rq.paused, "busy: back off 5 s and count a try");
check(M.afterRequeue(rec({ state: "transcribing", attempts: M.REQUEUE_MAX - 1 }), 0, null, "busy").state === "failed", `after ${M.REQUEUE_MAX} tries it fails (no endless loop)`);
const bg = M.afterRequeue(rec({ state: "transcribing", progress: 0.7, attempts: 3 }), 0, "bg", "aborted");
check(bg.state === "queued" && bg.paused === true && bg.attempts === 3 && bg.peak === 0.7 && bg.next_at === 0, "left the foreground: Paused, no try counted, runs again at once");
check(M.afterRequeue(rec({ state: "transcribing" }), 0, "rec", "aborted").paused === false, "a recording started: it waits, not 'Paused'");
check(M.watchdogMs(42) === M.WATCHDOG_MS && M.watchdogMs(45 * 60) === 900000 && M.watchdogMs(0) === M.WATCHDOG_MS, "the watchdog: 2 min, or a third of a long file");

// ---- where the words go
check(M.routeOf({ kind: "memo" }, "x".repeat(4000)) === "idea" && M.routeOf({ kind: "memo" }, "x".repeat(4001)) === "set", "a memo over 4000 characters goes as a set transcript");
check(M.routeOf({ kind: "set" }, "short") === "set", "a set always goes as a set transcript");
const ib = M.ideaBody(rec({ bit: { id: "j1", title: "Raccoons" } }), "words");
check(ib.text === "words" && ib.from.note === "Voice memo, 2026-10-31" && ib.reason.includes("Raccoons") && ib.reason.startsWith("[memo]") && ib.reason.includes("42 s"), "the idea body carries the words, the date, the length and the bit");
check(!M.ideaBody(rec({ duration_s: 0 }), "w").reason.includes(" 0 s"), "an import with no known length says no '0 s'");
const sb = M.setBody(rec({ kind: "set", board: { id: "b1", title: "Tuesday open mic" }, duration_s: 721.6 }), many, M.ENGINE);
check(sb.segments.length <= 5000 && sb.title === "Tuesday open mic" && sb.duration_s === 722 && sb.engine === M.ENGINE && sb.recorded_at === "2026-10-31T01:02:03.000Z", "the set body: capped segments, the list's title, whole seconds, the engine");
check(M.setBody(rec({ kind: "set" }), [], "e").title === "Set 2026-10-31" && M.setBody(rec(), [], "e").title === "Voice memo 2026-10-31", "a default title by kind");
check(JSON.stringify(M.filingSteps(null, { decision: "attach", into: "x", id: "at:1" })) === "{}", "no bit: the server's auto-attach stands");
check(JSON.stringify(M.filingSteps("j1", { decision: "attach", into: "j1", id: "at:1" })) === "{}", "auto-attached to the same bit: nothing more");
check(JSON.stringify(M.filingSteps("j1", { decision: "attach", into: "j2", id: "at:1" })) === JSON.stringify({ undo: "at:1", attach: true }), "auto-attached elsewhere: undo it, then attach to his bit");
check(JSON.stringify(M.filingSteps("j1", { decision: "new" })) === JSON.stringify({ attach: true }), "a new idea: attach to his bit");

// ---- what the row says
check(M.statusWords(rec()) === "Waiting to transcribe" && M.statusWords(rec({ paused: true })) === "Paused · finishes when Punchable is open", "queued and paused words");
check(M.statusWords(rec({ kind: "set", state: "transcribing", progress: 0.4 })) === "Transcribing · 40%", "a set shows its percent");
check(M.statusWords(rec({ kind: "set", state: "transcribing", progress: 0.1, peak: 0.6 })) === "Transcribing…", "after a restart the percent hides until it passes the old run");
check(M.statusWords(rec({ state: "transcribing", progress: 0.5 })) === "Transcribing…", "a memo shows no percent");
check(M.statusWords(rec({ state: "words", attempts: 2 })) === "Waiting for a connection", "words that could not go up yet");
check(M.statusWords(rec({ state: "filed", filed_on: { id: "j", title: "Raccoons" } })) === "On Raccoons", "filed on a bit");
check(M.statusWords(rec({ state: "sent" })) === "Sent · finding the bits" && M.statusWords(rec({ state: "no_speech" })) === "No speech heard", "sent / no speech words");
check(M.statusWords(rec({ state: "failed", error: "language-not-supported" })).includes("language"), "the language failure says so");
check(M.deleteMessage(rec({ state: "filed", filed_on: { id: "j", title: "Raccoons" } })) === "The words stay on Raccoons.", "delete: filed keeps the words on the bit");
check(M.deleteMessage(rec({ state: "new_idea" })) === "The words stay in Punchable." && M.deleteMessage(rec({ state: "sent" })) === "The words stay in Punchable.", "delete: a new idea or a sent set keeps the words");
check(M.deleteMessage(rec({ state: "failed" })).includes("hasn’t been transcribed"), "delete: nothing transcribed says it can't be undone");
check(M.wordsTitle("one two three four five six seven eight nine", "Memo") === "one two three four five six seven eight…" && M.wordsTitle("", "Memo") === "Memo", "a memo is titled by its first eight words");
check(M.shownPercent({ progress: 0.333, peak: 0 }) === 33 && M.shownPercent({ progress: 0.5, paused: true }) === null, "percent floors; paused shows none");

// ---- files
const n = M.safeFileName("2026-10-31T01:02:03.000Z", "ab9z", "M4A");
check(M.isSafeFileName(n) && n.endsWith(".m4a") && !n.includes(":"), `a generated name is URL-safe (${n})`);
check(!M.isSafeFileName("My memo (1).m4a") && !M.isSafeFileName("../x.m4a") && !M.isSafeFileName(".hidden"), "names with spaces, slashes or a leading dot are refused");
check(M.isCleanFileUri("file:///var/mobile/Containers/Data/Application/ABC-1/Documents/recordings/x.m4a"), "a clean Documents URI passes");
check(!M.isCleanFileUri("file:///var/x/My memo.m4a") && !M.isCleanFileUri("https://x/y.m4a") && !M.isCleanFileUri("file:///a/%20b.m4a"), "a URI with a space, a percent or another scheme is refused");
const idx = M.parseIndex([rec(), rec({ id: "bad", file: "../../etc" }), null, { id: 3 }, rec({ id: "t", state: "transcribing" })]);
check(idx.length === 2 && idx[1].state === "queued", "the index drops malformed rows and recovers a killed run");
check(M.parseIndex("nope").length === 0, "a broken index reads as empty");

// ---- a filmed take (kind film, Film it SPEC 2026-11-03): per-word timings, the queue order, where the words go
const fw = (transcript, words) => ({ isFinal: true, results: [{ transcript, segments: words.map(([segment, s, e]) => ({ startTimeMillis: s, endTimeMillis: e, segment })) }] });
let fa = M.emptyAcc();
fa = M.addFinal(fa, fw("Self checkout is", [["Self", 100, 400], ["checkout", 450, 900], ["is", 950, 1100]]));
fa = M.addFinal(fa, fw(" the store", [["the", 200, 300], ["store", 350, 800]]));
check(fa.words.length === 5 && fa.words[0].w === "Self" && fa.words[0].s === 0.1 && fa.words[0].e === 0.4, "per-segment words keep their seconds");
check(fa.words[3].w === "the" && fa.words[3].s >= fa.words[2].e && fa.words[4].s >= fa.words[3].e, "a final whose clock restarted shifts its words after the ones before (monotonic)");
check(M.addFinal(M.emptyAcc(), fin("no segment text", 0, 500)).words.length === 0 && M.addFinal(M.emptyAcc(), fin("no segment text", 0, 500)).segs.length === 1, "a segment with no text is no word (the utterance still counts)");
const fq = [rec({ id: "s", kind: "set", recorded_at: "2026-10-30T00:00:00Z" }), rec({ id: "f", kind: "film", recorded_at: "2026-10-29T00:00:00Z" }), rec({ id: "m", recorded_at: "2026-10-31T00:00:00Z" })];
check(M.nextToTranscribe(fq)?.id === "m" && M.nextToTranscribe(fq.filter((r) => r.id !== "m"))?.id === "f", "memos, then filmed takes, then sets");
check(M.routeOf({ kind: "film" }, "short") === "idea" && M.routeOf({ kind: "film" }, "x".repeat(4001)) === "set", "a take's words go as an idea (a long one as a set transcript)");
const fb = M.ideaBody(rec({ kind: "film", bit: { id: "j1", title: "Self-checkout" } }), "words");
check(fb.from.note === "Film it, 2026-10-31" && fb.reason.startsWith("[film]") && fb.reason.includes("Self-checkout"), "a take's idea body says it was filmed, and on which bit");
check(M.setBody(rec({ kind: "film", script: "line one\nline two" }), [], "e").title === "Film it 2026-10-31", "a long take's set title is not its script");
check(M.recTitle(rec({ kind: "film", text: "" })) === "Take" && M.recTitle(rec({ kind: "film", text: "what I said" })) === "what I said", "a take's title");
check(M.deleteMessage(rec({ kind: "film", state: "filed", filed_on: { id: "j", title: "Raccoons" } })) === "The words stay on Raccoons. The video stays in Photos.", "deleting a take's words says the video stays in Photos");
check(M.parseIndex([rec({ kind: "film" }), rec({ id: "x", kind: "video" })]).length === 1, "the index reads film rows and drops an unknown kind");
check(M.parseIndex([rec({ kind: "constructor" })]).length === 0, "a kind named after an Object property is not a kind");

if (fails.length) { console.error(`\n${fails.length} failed`); process.exit(1); }
console.log("\nmemoRules: all passed");
