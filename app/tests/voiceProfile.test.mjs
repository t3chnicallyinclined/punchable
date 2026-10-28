// Unit test for src/lib/voiceProfile.ts: what the Voice profile screen does with the API's document (Save on/off,
// Undo = the previous version, the status line, the normalisation the API applies). Invented text only.
//   node --experimental-strip-types --no-warnings tests/voiceProfile.test.mjs
import * as V from "../src/lib/voiceProfile.ts";

const fails = [];
const check = (c, m) => { console.log((c ? "PASS " : "FAIL ") + m); if (!c) fails.push(m); };

const v1 = "# Voice\n\nHe sits on the punch word. The parrot bit closes.";
const doc = { kind: "voice", text: v1 + "\n", updated_at: "2026-10-28T10:00:00+00:00", by: "owner", chars: v1.length + 1, previous: null };

check(V.normalize("\r\n# A\r\nb\n\n") === "# A\nb", "normalize: CRLF -> LF, blank lines at both ends dropped");
check(V.wordCount(v1) === 12 && V.wordCount("") === 0 && V.wordCount(null) === 0, `word count (${V.wordCount(v1)})`);

check(V.canSave("", null).ok === false && V.canSave("   \n", null).why === "Write something first.", "empty: Save off, says why");
check(V.canSave(v1, null).ok === true, "new text, nothing saved: Save on");
check(V.canSave(v1 + "\n\n", doc.text).ok === false && V.canSave(v1, doc.text).why === "Nothing changed.", "the same text (modulo trailing newlines): Save off");
check(V.canSave(v1 + " More.", doc.text).ok === true, "a change: Save on");
const long = "x".repeat(V.MAX_CHARS + 5);
check(V.canSave(long, null).ok === false && V.canSave(long, null).why === "Too long by 5 characters.", "over the API's limit: Save off, says by how much");

check(V.undoVersion(doc) === null && V.undoVersion(null) === null, "a first save has nothing to undo");
const doc2 = { ...doc, text: v1 + " More.\n", updated_at: "2026-10-29T10:00:00+00:00", previous: { text: doc.text, updated_at: doc.updated_at, by: "owner" } };
check(V.undoVersion(doc2)?.text === doc.text, "after a second save, Undo restores the previous version");
check(V.undoVersion({ ...doc2, previous: { text: "  \n", updated_at: "x", by: "owner" } }) === null, "a blank previous is no undo");

check(V.statusLine(null) === "Not written yet. The AI drafts without it." && V.statusLine({ ...doc, text: null }) === "Not written yet. The AI drafts without it.", "no profile: the honest line");
const s = V.statusLine(doc);
check(/^Saved Oct 2[78], 2026 · 12 words · by you$/.test(s), `saved by him: "${s}"`);
check(V.statusLine({ ...doc, by: "import:profile" }).endsWith("· loaded from your desk"), "loaded by the desk tool: says so");
check(V.savedWords(doc) === "Saved · 12 words", "the snackbar words");

// the learned section (read-only; the server fills it from his keeps)
check(V.learnedWords(null) === "Nothing yet. It learns when you take one of its drafts as yours." && V.learnedWords({ kept: [], dropped: [], updated_at: null }) === V.learnedWords(undefined), "nothing learned: the honest line");
const learned = { kept: [{ text: "The parrot audits the vet.", bit: "joke:p", date: "2026-10-28" }], dropped: [{ text: "Parrot, IRS, same bird.", bit: "joke:p", date: "2026-10-28" }, { text: "A parrot that files its own taxes.", bit: null, date: null }], updated_at: "2026-10-28T11:00:00+00:00" };
const lw = V.learnedWords(learned);
check(/^1 line kept · 2 passed over · last Oct 2[78], 2026$/.test(lw), `counts and the last day: "${lw}"`);
check(V.learnedWords({ ...learned, kept: [...learned.kept, learned.kept[0]], updated_at: null }) === "2 lines kept · 2 passed over", "plural, no date when the server has none");
const rows = V.learnedRows(learned.dropped);
check(rows.length === 2 && rows[0].text === "Parrot, IRS, same bird." && /^Oct 2[78], 2026$/.test(rows[0].when) && rows[1].when === "", "rows keep the server's order, a missing date shows nothing");
check(V.learnedRows(null).length === 0 && V.learnedRows(learned.dropped, 1).length === 1, "no lines, or a cap");

if (fails.length) { console.log(`\n${fails.length} failed`); process.exit(1); }
console.log("\nall passed");
