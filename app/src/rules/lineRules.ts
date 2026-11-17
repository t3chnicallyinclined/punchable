// A bit is a document (owner 2026-11-16: "a bit is a document, it just has a bunch of ideas for the same topic ... it
// should be as easy as editing a google doc on your phone"; docs/design/SPEC-2026-11-16-bits-sets-tapes.md B11, §4.3,
// Slice 2). The page edits ONE text, in one multiline input, so selection, paste, dictation and Backspace behave as
// they do in Notes. Nothing here splits the text into inputs. These rules read structure OUT of the text and decide
// what a save is:
//   - a newline is a line; a blank line separates two ideas (a bit holds several ideas on one topic);
//   - a save is a new version only when the words changed (trailing spaces and blank ends are not words), never when
//     the document is empty, never above the route's limit;
//   - a line put in from a card or the pile, a wording swapped and a reorder keep the blank lines where they were;
//   - the local draft (nothing typed is lost) and which text the page opens on;
//   - where the page must scroll so the caret is never under the keyboard.
// No React, no React Native: tests/lineRules.test.mjs runs it under node, with a --red arm per rule.

/** The longest text a version takes, in characters (POST /api/jokes/<id>/versions refuses more: pk-api
 *  MAX_VERSION_CHARS). A bit is a document of several ideas on one topic, so this is a few pages, not a few lines. */
export const MAX_DOC = 40000;
/** The local draft is written this long after the last keystroke. */
export const DRAFT_SETTLE_MS = 400;
/** A pause: this long without a keystroke while the words differ from the saved version, and a version is saved. */
export const PAUSE_MS = 30000;
/** A pause saves a version at most this often: thinking between sentences must not fill the Versions list with
 *  near-copies. Leaving the text, the page or the app always saves; the words are on the phone meanwhile. */
export const PAUSE_GAP_MS = 180000;
/** Whether a pause may save a version now: never sooner than PAUSE_GAP_MS after the last version this page saved. */
export function pauseSaves(now: number, lastSavedAt: number | null): boolean {
  return lastSavedAt == null || now - lastSavedAt >= PAUSE_GAP_MS;
}

const unix = (s?: string | null): string => (s || '').replace(/\r\n?/g, '\n');
const blank = (l: string): boolean => l.trim().length === 0;

/** The document as it is sent: the line ends made plain and the blank ends dropped (the route strips them too).
 *  Nothing inside is touched: his blank lines and his spacing are his. */
export function docText(raw?: string | null): string {
  return unix(raw).replace(/^\s*\n/, '').replace(/\s+$/, '');
}
/** What two texts are compared by: each line without its trailing space, the blank ends dropped. */
const wordsOf = (raw?: string | null): string => unix(raw).split('\n').map((l) => l.replace(/\s+$/, '')).join('\n').trim();
/** Two texts say the same when only trailing spaces or blank ends differ. */
export const sameDoc = (a?: string | null, b?: string | null): boolean => wordsOf(a) === wordsOf(b);

/** What leaving (or a pause) does with the typed text against the saved version: nothing when the words are the same,
 *  nothing when it is empty (a version is never empty), a reason when it is too long, else one new version. */
export type SaveDecision = 'none' | 'empty' | 'too_long' | 'version';
export function saveDecision(saved: string | null | undefined, typed: string | null | undefined): SaveDecision {
  if (sameDoc(saved, typed)) return 'none';
  const t = docText(typed);
  if (!t) return 'empty';
  if ([...t].length > MAX_DOC) return 'too_long';
  return 'version';
}
/** "4,120 of 4,000": the one reason a long document is not saved as a version (it stays on the phone). */
export function tooLongNote(typed: string): string {
  return `Too long to save · ${[...docText(typed)].length.toLocaleString('en-US')} of ${MAX_DOC.toLocaleString('en-US')}`;
}

// ------------------------------------------------------------------ structure read out of the text

/** A row of the document: a line of his, or the gap between two ideas (any run of blank lines is one gap). */
export type Row = { kind: 'line'; text: string } | { kind: 'break' };

/** The text as rows: every line with words, and one break where blank lines sit between two of them. */
export function rowsOf(text?: string | null): Row[] {
  const out: Row[] = [];
  for (const l of unix(text).split('\n')) {
    if (blank(l)) {
      if (out.length && out[out.length - 1].kind === 'line') out.push({ kind: 'break' });
    } else {
      out.push({ kind: 'line', text: l.replace(/\s+$/, '') });
    }
  }
  while (out.length && out[out.length - 1].kind === 'break') out.pop();
  return out;
}
/** Rows back to text: a line per newline, one blank line per break; a break at an end or beside another is dropped. */
export function textOf(rows: Row[]): string {
  const parts: string[] = [];
  let gap = false;
  for (const r of rows) {
    if (r.kind === 'break') { gap = parts.length > 0; continue; }
    if (gap) parts.push('');
    gap = false;
    parts.push(r.text);
  }
  return parts.join('\n');
}
/** The ideas of a bit: each run of lines between blank lines. */
export function ideasOf(text?: string | null): string[][] {
  const out: string[][] = [];
  let cur: string[] = [];
  for (const r of rowsOf(text)) {
    if (r.kind === 'line') cur.push(r.text);
    else if (cur.length) { out.push(cur); cur = []; }
  }
  if (cur.length) out.push(cur);
  return out;
}
/** "3 ideas · 12 lines" for a row's meta and VoiceOver; one idea is not counted ("12 lines"). */
export function docMeta(text?: string | null): string {
  const ideas = ideasOf(text);
  const lines = ideas.reduce((n, i) => n + i.length, 0);
  const l = `${lines} ${lines === 1 ? 'line' : 'lines'}`;
  return ideas.length > 1 ? `${ideas.length} ideas · ${l}` : l;
}
/** Reorder mode: one row moved (a line, or the gap between two ideas). */
export function moveRow(rows: Row[], from: number, to: number): Row[] {
  if (from === to || from < 0 || from >= rows.length || to < 0 || to >= rows.length) return rows.slice();
  const a = rows.slice();
  const [x] = a.splice(from, 1);
  a.splice(to, 0, x);
  return a;
}

/** Where each line with words sits in the raw text: its index among the raw lines. */
function lineSlots(raw: string[]): number[] {
  const out: number[] = [];
  raw.forEach((l, k) => { if (!blank(l)) out.push(k); });
  return out;
}
/** The line (counted as the page counts them: lines with words only) the caret is on; on a blank line, the line
 *  above it; null when no line is at or above the caret. */
export function caretLine(text: string | null | undefined, caret: number): number | null {
  const raw = unix(text).split('\n');
  let pos = 0;
  let n = -1;
  for (const l of raw) {
    if (!blank(l)) n++;
    if (caret <= pos + l.length) break;
    pos += l.length + 1;
  }
  return n >= 0 ? n : null;
}

/** A place in the document that something is about (a card today; a mark he puts on a line or on words in Slice 3,
 *  and the tracked change that answers it): the
 *  line, counted among lines with words, the idea it is in, and where its words sit in the raw text. Lines have no
 *  ids (B1), so a place is found by the line's own WORDS: it survives a new version, an edit above it and a reorder,
 *  and is simply gone when he rewrote that line. */
export type Anchor = { line: number; idea: number; start: number; end: number; text: string };
/** Every line with words as an anchor, in order. */
export function anchorsOf(text?: string | null): Anchor[] {
  const out: Anchor[] = [];
  let pos = 0;
  let idea = 0;
  let gap = false;
  for (const l of unix(text).split('\n')) {
    if (blank(l)) { gap = out.length > 0; }
    else {
      if (gap) idea++;
      gap = false;
      const lead = l.length - l.replace(/^\s+/, '').length;
      const words = l.trim();
      out.push({ line: out.length, idea, start: pos + lead, end: pos + lead + words.length, text: words });
    }
    pos += l.length + 1;
  }
  return out;
}
/** The place a quoted line is at now: the hinted line when it still says the same, else the nearest line to the
 *  hint that says it, else null (the line changed: whatever was tied to it is about the whole bit now). */
export function findAnchor(text: string | null | undefined, quoted: string, hint?: number | null): Anchor | null {
  const q = quoted.trim();
  if (!q) return null;
  const all = anchorsOf(text);
  if (hint != null && all[hint]?.text === q) return all[hint];
  const same = all.filter((a) => a.text === q);
  if (!same.length) return null;
  if (hint == null) return same[0];
  return same.reduce((best, a) => (Math.abs(a.line - hint) < Math.abs(best.line - hint) ? a : best));
}

/** One change to the document that keeps every blank line where it was: `words` put in after line `at` (null = at
 *  the end), or line `at` replaced by them. `at` counts lines with words, as the cards and the pile do. */
export type DocEdit = { op: 'insert'; at: number | null; words: string } | { op: 'replace'; at: number; words: string };
export function applyEdit(text: string | null | undefined, edit: DocEdit): string {
  const add = unix(edit.words).split('\n').map((l) => l.trim()).filter(Boolean);
  const doc = docText(text);
  if (!add.length) return doc;
  const raw = doc ? doc.split('\n') : [];
  const slots = lineSlots(raw);
  if (edit.op === 'replace') {
    const k = slots[edit.at];
    if (k == null) return [...raw, ...add].join('\n');
    return [...raw.slice(0, k), ...add, ...raw.slice(k + 1)].join('\n');
  }
  const k = edit.at == null || edit.at >= slots.length ? raw.length : edit.at < 0 ? 0 : slots[edit.at] + 1;
  return [...raw.slice(0, k), ...add, ...raw.slice(k)].join('\n');
}

// ------------------------------------------------------------------ the local draft (nothing typed is lost)

/** What the phone keeps while the words differ from the saved version: one per bit and cut. */
export type BitDraft = {
  v: 1; bit: string; cut: 'stage' | 'rant';
  /** the version it was typed over (its unit), or null on a bit with none */ base: string | null;
  text: string; at: string;
};
export function bitDraft(bit: string, cut: 'stage' | 'rant', base: string | null, text: string, at: string): BitDraft {
  return { v: 1, bit, cut, base, text: unix(text), at };
}
/** A draft read back; anything that is not one (a half-written file) is no draft. */
export function parseBitDraft(raw: string | null | undefined): BitDraft | null {
  if (!raw) return null;
  try {
    const d = JSON.parse(raw);
    if (!d || d.v !== 1 || typeof d.bit !== 'string' || !d.bit || typeof d.text !== 'string') return null;
    if (d.cut !== 'stage' && d.cut !== 'rant') return null;
    return { v: 1, bit: d.bit, cut: d.cut, base: typeof d.base === 'string' ? d.base : null, text: d.text, at: typeof d.at === 'string' ? d.at : '' };
  } catch {
    return null;
  }
}
/** The draft's name on the phone: the bit, and its cut when that is not the stage cut. */
export const draftName = (bit: string, cut: 'stage' | 'rant'): string => (cut === 'rant' ? `${bit}~rant` : bit);

/** Whether some version of the bit already says what the draft says. A save can reach the server and its answer be
 *  lost (the app was suspended): the draft is then a copy of an OLDER version, and saving it again would bury
 *  whatever he wrote since on another device. `said` = the texts of every version of that cut. */
const alreadySaid = (draft: BitDraft, said: readonly (string | null | undefined)[]): boolean => said.some((t) => sameDoc(t, draft.text));

/** The text the page opens on: his draft when it says something NO version does, else the saved version.
 *  `pending` = there are words on the phone that no version holds yet. */
export function openWith(saved: string | null | undefined, draft: BitDraft | null, said: readonly (string | null | undefined)[] = []): { text: string; pending: boolean } {
  if (draft && docText(draft.text) && !alreadySaid(draft, [saved, ...said])) return { text: draft.text, pending: true };
  return { text: unix(saved), pending: false };
}
/** A draft found at launch, against the bit's versions of that cut: dropped when some version already says it or it
 *  is empty, kept waiting when it is too long, else saved as a new version. His words always win a place: when the bit
 *  moved on elsewhere a draft that says something new is still saved, as the newest version, and the other stays
 *  under it. */
export function flushPlan(draft: BitDraft, saved: string | null | undefined, said: readonly (string | null | undefined)[] = []): 'drop' | 'keep' | 'save' {
  const d = saveDecision(saved, draft.text);
  if (d === 'none' || d === 'empty' || alreadySaid(draft, said)) return 'drop';
  return d === 'too_long' ? 'keep' : 'save';
}

// ------------------------------------------------------------------ the caret stays above the keyboard

/** The text whose laid-out height is the caret's height in the document: everything up to the caret, and on to the
 *  end of the word the caret is in. A word wraps whole, so cutting it at the caret would measure it on the line above. */
export function mirrorText(text: string, caret: number): string {
  const at = Math.max(0, Math.min(caret, text.length));
  const rest = text.slice(at).match(/^\S*/);
  return text.slice(0, at + (rest ? rest[0].length : 0));
}

/** Where the page scrolls to so the caret is in view, or null when it already is. Content coordinates: `top` and
 *  `bottom` are the caret's line, `offset` the scroll position, `height` the scroll view's height (already short by the
 *  keyboard), `guardTop` what the bar covers at the top, `margin` the room kept under the caret. Never a jump when
 *  the caret is visible: scrolling under his thumb while he types is worse than a caret one line low. */
export function caretScroll(x: { top: number; bottom: number; offset: number; height: number; guardTop: number; margin: number }): number | null {
  if (!(x.height > 0)) return null;
  const low = x.offset + x.height - x.margin;
  if (x.bottom > low) return Math.round(x.bottom - x.height + x.margin);
  const high = x.offset + x.guardTop;
  if (x.top < high) return Math.round(x.top - x.guardTop);
  return null;
}
