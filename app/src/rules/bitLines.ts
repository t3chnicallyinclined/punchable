// The bit page's pure rules (docs/design/SPEC-2026-11-03-bit-page-3c.md §3, §14 Slice 1): a version's text as lines,
// the punch marks, the tags fold, the row labels and meta strings, the one-word reason under Write, and the AX
// re-stack rule. No React, no React Native: tests/bitLines.test.mjs runs it under node.
import { MONTHS, MONTHS_LONG, mmss } from './format.ts';
import { STACK_AT } from './layoutRules.ts';

/** The Dynamic Type caps (DESIGN.md §4, owner-signed): his lines are prose (2.0×); rows and chips are controls (1.6×). */
export const PROSE_CAP = 2.0;
export const ROW_CAP = 1.6;
/** The newest versions the pile shows inline before "N more". */
export const PILE_INLINE = 5;

const isInt = (x: unknown): x is number => typeof x === 'number' && Number.isInteger(x);

/** A version's text as lines: split on newlines, trailing space dropped, blank lines left out. */
export function linesOf(text?: string | null): string[] {
  return (text || '').split(/\r?\n/).map((l) => l.replace(/\s+$/, '')).filter((l) => l.trim().length > 0);
}
/** Lines back to the text a version stores (one line per newline). */
export function joinLines(lines: string[]): string {
  return lines.map((l) => l.trim()).filter(Boolean).join('\n');
}

/** Where the i-th line (as `linesOf` counts them) starts in the raw text: the caret position for "open Write at this line". */
export function lineOffset(text: string | null | undefined, i: number): number {
  const raw = text || '';
  let n = 0;
  let pos = 0;
  for (const seg of raw.split(/(\r?\n)/)) {
    if (/^\r?\n$/.test(seg)) { pos += seg.length; continue; }
    if (seg.trim().length > 0) {
      if (n === i) return pos + (seg.length - seg.replace(/^\s+/, '').length);
      n++;
    }
    pos += seg.length;
  }
  return raw.length;
}

/** The punch marks a read carries, cleaned for display: whole numbers inside the bit, sorted, no repeats. Reads are lenient. */
export function cleanPunch(punch: unknown, count: number): number[] {
  if (!Array.isArray(punch)) return [];
  const out = new Set<number>();
  for (const x of punch) if (isInt(x) && x >= 0 && x < count) out.add(x);
  return [...out].sort((a, b) => a - b);
}

/** Toggle the punch mark on line `i` of a bit with `count` lines. A write is strict: a line that is not there is refused. */
export function markPunch(punch: number[], i: number, count: number): number[] {
  if (!isInt(i) || i < 0 || i >= count) throw new RangeError(`no line ${i} in a bit of ${count} lines`);
  const cur = cleanPunch(punch, count);
  return cur.includes(i) ? cur.filter((x) => x !== i) : [...cur, i].sort((a, b) => a - b);
}

export const isPunch = (punch: number[], i: number) => punch.includes(i);

/** The index where the tags start (the lines after the last punch); `count` when nothing folds (no punch, or the punch is last). */
export function foldAt(count: number, punch: number[]): number {
  const p = cleanPunch(punch, count);
  if (!p.length) return count;
  const last = p[p.length - 1];
  return last >= count - 1 ? count : last + 1;
}
export const tagsFolded = (count: number, punch: number[]) => count - foldAt(count, punch);
export const foldLabel = (n: number) => `+ ${n} ${n === 1 ? 'tag' : 'tags'}`;

/** VoiceOver for a line: "Line 5 of 8, punch: Still drinks out of the toilet." */
export function lineLabel(i: number, count: number, punch: number[], text: string): string {
  return `Line ${i + 1} of ${count}${isPunch(punch, i) ? ', punch' : ''}: ${text}`;
}

/** The bit without line `i`; null when that would leave nothing (a version is never empty). */
export function cutLine(lines: string[], i: number): string[] | null {
  if (i < 0 || i >= lines.length || lines.length < 2) return null;
  return lines.filter((_, k) => k !== i);
}
export function moveLine(lines: string[], from: number, to: number): string[] {
  if (from === to || from < 0 || from >= lines.length || to < 0 || to >= lines.length) return lines.slice();
  const a = lines.slice();
  const [x] = a.splice(from, 1);
  a.splice(to, 0, x);
  return a;
}
/** A scrap's words put into the bit after line `at` (null = at the end); the words may hold lines of their own. */
export function insertAfter(lines: string[], at: number | null, words: string): string[] {
  const add = linesOf(words);
  if (!add.length) return lines.slice();
  const k = at == null || at >= lines.length ? lines.length : at < 0 ? 0 : at + 1;
  return [...lines.slice(0, k), ...add, ...lines.slice(k)];
}

const fold = (s: string) => s.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
/** A scrap is already in the bit when its words (or every line of them) appear in it. */
export function inBit(lines: string[], words?: string | null): boolean {
  const add = linesOf(words);
  if (!add.length) return false;
  const bit = fold(lines.join('\n'));
  return add.every((l) => fold(l).length > 0 && bit.includes(fold(l)));
}

/** "Nov 2" from an ISO date; '' when there is none. */
export function monthDay(iso?: string | null): string {
  if (!iso) return '';
  const m = Number(iso.slice(5, 7));
  const d = Number(iso.slice(8, 10));
  return m >= 1 && m <= 12 && d >= 1 ? `${MONTHS[m - 1]} ${d}` : iso.slice(0, 4);
}
/** "November 2" for VoiceOver. */
export function monthDayLong(iso?: string | null): string {
  if (!iso) return '';
  const m = Number(iso.slice(5, 7));
  const d = Number(iso.slice(8, 10));
  return m >= 1 && m <= 12 && d >= 1 ? `${MONTHS_LONG[m - 1]} ${d}` : iso.slice(0, 4);
}

/** The run time as it reads in a row: "1:40", "~1:40" when estimated, '' when unknown. */
export function runTime(durationS?: number | null, estimated?: boolean): string {
  if (durationS == null || !Number.isFinite(durationS) || durationS <= 0) return '';
  return `${estimated ? '~' : ''}${mmss(durationS)}`;
}

export type VersionsMetaInput = { count: number; newestDate?: string | null; durationS?: number | null; estimated?: boolean; justWrote?: boolean };
/** The Versions row collapsed: "v8 · Nov 2 · 1:40"; right after a write "v9 · just now". */
export function versionsMeta(x: VersionsMetaInput): string {
  const parts = [`v${x.count}`];
  if (x.justWrote) parts.push('just now');
  else {
    const d = monthDay(x.newestDate);
    if (d) parts.push(d);
  }
  const t = runTime(x.durationS, x.estimated);
  if (t) parts.push(t);
  return parts.join(' · ');
}
/** VoiceOver for the Versions row: "Versions, 8, newest November 2, run time 1:40". */
export function versionsLabel(x: VersionsMetaInput): string {
  const parts = ['Versions', String(x.count)];
  if (x.justWrote) parts.push('newest just now');
  else {
    const d = monthDayLong(x.newestDate);
    if (d) parts.push(`newest ${d}`);
  }
  const t = runTime(x.durationS, x.estimated);
  if (t) parts.push(`run time ${t}`);
  return parts.join(', ');
}

/** The stepper's label: "v5 of 8 · Oct 20". */
export function stepperLabel(vi: number, count: number, date?: string | null): string {
  const n = count - vi;
  const d = monthDay(date);
  return `v${n} of ${count}${d ? ` · ${d}` : ''}`;
}
/** The stepper's second line: the source doc and that version's run time, what there is of them. */
export function stepperSub(doc?: string | null, durationS?: number | null, estimated?: boolean): string {
  return [doc ? `from ${doc}` : '', runTime(durationS, estimated)].filter(Boolean).join(' · ');
}
/** VoiceOver for the stepper's value (the element is labelled "Version"): "5 of 8, October 20". */
export function stepperValue(vi: number, count: number, date?: string | null): string {
  const d = monthDayLong(date);
  return `${count - vi} of ${count}${d ? `, ${d}` : ''}`;
}
export const restoreLabel = (vi: number, count: number) => `Restore v${count - vi} as a new version`;

export const countWord = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
/** "killed once", "killed twice", "killed 3 times". */
export function killedWord(n: number): string {
  if (n <= 0) return '';
  if (n === 1) return 'killed once';
  if (n === 2) return 'killed twice';
  return `killed ${n} times`;
}
/** The On tape row's meta; null hides the row (never taped, never filmed, nothing sounds like it). `filmed` = kept
 *  takes on this phone (Film it, SPEC 2026-11-03 §2.7): "3 nights · killed twice · filmed 2"; never performed but
 *  filmed reads "filmed 1". */
export function tapeMeta(nights: number, killed: number, echoes = 0, filmed = 0): string | null {
  const f = filmed > 0 ? `filmed ${filmed}` : '';
  if (nights > 0) return [countWord(nights, 'night', 'nights'), killedWord(killed), f].filter(Boolean).join(' · ');
  if (f) return f;
  if (echoes > 0) return `not yet · ${countWord(echoes, 'sounds like it', 'sound like it')}`;
  return null;
}

/** A version of the bit as the page and the camera read it from GET /api/jokes/<id>: every unit with text, newest
 *  first, labelled vN from the newest; the typed idea stands in when a bit of his has no unit yet. The punch marks
 *  ride on the row once the read carries them (line_meta, 3c Slice 3). */
export type BitVersion = { key: string; label: string; date: string | null; text: string; doc?: string | null; punch?: number[] };
export function versionsOf(d: any): BitVersion[] {
  const out: BitVersion[] = [];
  for (const g of d?.versions || []) {
    for (const u of g.units || []) {
      if (!u.text) continue;
      out.push({ key: u.id || `${g.date}-${out.length}`, label: '', date: u.date || g.date || null, text: u.text, doc: u.doc_title, punch: u.line_meta?.punch ?? u.punch });
    }
  }
  if (!out.length && d?.joke?.summary && d?.yours) out.push({ key: 'idea', label: '', date: d.joke.first_seen || null, text: d.joke.summary });
  out.sort((a, b) => (b.date || '').localeCompare(a.date || ''));
  return out.map((v, i) => ({ ...v, label: `v${out.length - i}` }));
}
/** How many performed rows he rated "killed" (the rows that carry a rating; unrated rows count nothing). */
export function killedCount(rows: { rating?: string | null }[]): number {
  return rows.filter((r) => r.rating === 'killed').length;
}

/** The one reason under Write, or null when it is live: the offline line wins, then "Reading v5". */
export function writeReason(x: { vi: number; count: number; online: boolean }): string | null {
  if (!x.online) return 'Offline · reading only';
  if (x.vi > 0) return `Reading v${x.count - x.vi}`;
  return null;
}
/** The sparkle and To a set are off until the bit has a line or an idea. */
export const barEnabled = (lines: string[], idea?: string | null) => lines.length > 0 || !!(idea || '').trim();

/** The idea line: one line at the default size, two from the re-stack point. */
export const ideaLines = (fontScale: number) => (fontScale >= STACK_AT ? 2 : 1);
/** A row's meta goes under its label from the re-stack point (HIG/typography: a stacked layout at large sizes). */
export const stackMeta = (fontScale: number) => fontScale >= STACK_AT;
/** A control's height at the OS text size, capped at the control class (1.6×). */
export function rowHeight(base: number, fontScale: number): number {
  const fs = Number.isFinite(fontScale) && fontScale > 0 ? fontScale : 1;
  return Math.round(base * Math.min(ROW_CAP, Math.max(1, fs)));
}

/** A scrap in the pile, as the page shows it in Slice 1: a note attached to the bit, or a memo on this phone. */
export type Scrap = { id: string; kind: 'jot' | 'memo' | 'doc'; words: string; date?: string | null; recId?: string | null; ai?: boolean };
/** The pile's rows: memos on this phone first (newest first), then the server's attached notes, minus the notes that
 *  ARE those memos (a filed memo's attach id is the note's id), newest first. */
export function pileScraps(notes: { id: string; text?: string | null; date?: string | null; reasons?: unknown[] | null; score?: number | null }[], memos: { id: string; text?: string | null; recorded_at: string; merge_id?: string | null }[]): Scrap[] {
  const memoAttach = new Set(memos.map((m) => m.merge_id).filter(Boolean) as string[]);
  const fromMemos: Scrap[] = [...memos].sort((a, b) => b.recorded_at.localeCompare(a.recorded_at))
    .map((m) => ({ id: `memo:${m.id}`, kind: 'memo', words: (m.text || '').trim(), date: m.recorded_at, recId: m.id }));
  // a judge's reasons or score on the attach record = Claude filed it; his own attach carries neither
  const fromNotes: Scrap[] = notes.filter((n) => !memoAttach.has(n.id)).sort((a, b) => (b.date || '').localeCompare(a.date || ''))
    .map((n) => ({ id: n.id, kind: 'jot', words: (n.text || '').trim(), date: n.date || null, ai: !!(n.reasons && n.reasons.length) || n.score != null }));
  return [...fromMemos, ...fromNotes];
}
