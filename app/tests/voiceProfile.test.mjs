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

if (fails.length) { console.log(`\n${fails.length} failed`); process.exit(1); }
console.log("\nall passed");
