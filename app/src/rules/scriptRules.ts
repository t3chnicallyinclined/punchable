// The script's pure rules (docs/design/SPEC-2026-11-06-rant-tab.md §12.4, §12.5): a rant as lines he edits like Notes,
// each line carrying where it came from, and how those marks move when he edits, splits, joins, cuts, moves, pulls a
// line from his material, uses one of Claude's versions, or Claude drafts the rest around his lines.
//
// The marks (shown under a line): `by: 'him'` with `from` = his words from a bit ("from Free employee v3", "· edited"
// once he changes it); `by: 'ai'` = Claude's line (the AI badge) until he edits it, then `by: 'yours'` (the `Yours`
// mark); `by: 'him'` with no `from` = a line he typed (nothing); no `by` = not known (an older rant): nothing shown,
// never a guess. No React, no network: tests/scriptRules.test.mjs runs it under node, with a --red arm.
import { DEFAULT_WPM, wordCount } from './prompterRules.ts';
import { mmss } from './format.ts';

export type By = 'him' | 'ai' | 'yours';
/** One line on the server (GET /api/topics/<id> rants[].lines). */
export type DocLine = { text: string; by?: By | null; from?: string | null; label?: string | null; edited?: boolean; punch?: boolean };
/** One line on the page: a stable key for React and for anchoring a draft. */
export type SLine = DocLine & { key: string };
/** What the `script` store keeps per line. */
export type Mark = { by?: By; from?: string; label?: string; edited?: true; punch?: true };

let seq = 0;
/** A key no other line on this phone has (stable across edits of the same line). */
export const newKey = (): string => `l${Date.now().toString(36)}${(seq++).toString(36)}`;

const clean = (t: string) => (t || '').replace(/\s+/g, ' ').trim();

/** The server's lines as page lines. */
export function fromDoc(lines: DocLine[] | null | undefined): SLine[] {
  return (lines || []).map((l) => ({ ...l, text: l.text || '', key: newKey() }));
}

/** What a changed line becomes: Claude's line he edits is his ("yours"); a quoted line of his is "edited"; a typed
 *  or unknown line stays as it is. The same text is no change. */
export function edited(l: SLine, text: string): SLine {
  if (text === l.text) return l;
  if (l.by === 'ai') return { ...l, text, by: 'yours' };
  if (l.by === 'him' && l.from) return { ...l, text, edited: true };
  return { ...l, text };
}

/** Edit line i. */
export function editLine(lines: SLine[], i: number, text: string): SLine[] {
  if (i < 0 || i >= lines.length) return lines;
  const out = lines.slice();
  out[i] = edited(lines[i], text);
  return out;
}

/** Return in the middle of line i: the text before the caret stays (an edit), the rest is a new typed line under it. */
export function splitAt(lines: SLine[], i: number, before: string, after: string): { lines: SLine[]; focus: number } {
  if (i < 0 || i >= lines.length) return { lines, focus: i };
  const out = editLine(lines, i, before.replace(/\s+$/, ''));
  out.splice(i + 1, 0, { key: newKey(), text: after.replace(/^\s+/, ''), by: 'him' });
  return { lines: out, focus: i + 1 };
}

/** Backspace on an empty line joins up: the line goes, the caret goes to the end of the line above. The first line, or
 *  a line with words, stays. */
export function joinUp(lines: SLine[], i: number): { lines: SLine[]; focus: number } | null {
  if (i <= 0 || i >= lines.length || lines[i].text.trim()) return null;
  const out = lines.slice();
  out.splice(i, 1);
  return { lines: out, focus: i - 1 };
}

/** A new typed line after line i (i = -1: at the top; i >= length: at the end). */
export function addLine(lines: SLine[], i: number, text = ''): { lines: SLine[]; focus: number } {
  const at = Math.max(0, Math.min(lines.length, i + 1));
  const out = lines.slice();
  out.splice(at, 0, { key: newKey(), text, by: 'him' });
  return { lines: out, focus: at };
}

/** Lines without these keys (an Undo of something added: whatever he typed since stays). */
export const removeKeys = (lines: SLine[], keys: string[]): SLine[] => {
  const k = new Set(keys);
  return lines.filter((l) => !k.has(l.key));
};

/** Put a line back as it was, by its key (an Undo of a change to one line: the other lines stay as they are now). */
export const restoreLine = (lines: SLine[], was: SLine): SLine[] => lines.map((l) => (l.key === was.key ? was : l));

/** Empty lines go when he stops writing (the iOS multiline field never reports Backspace on an empty line, so an
 *  empty line he leaves behind would stay); `keep` = the key of the line he is moving to. */
export const dropEmpty = (lines: SLine[], keep?: string | null): SLine[] => lines.filter((l) => l.text.trim() || l.key === keep);

/** Cut line i: the new list and what Undo puts back. */
export function cutLine(lines: SLine[], i: number): { lines: SLine[]; undo: (cur: SLine[]) => SLine[] } {
  const gone = lines[i];
  const out = lines.filter((_, k) => k !== i);
  return {
    lines: out,
    undo: (cur) => {
      if (!gone || cur.some((l) => l.key === gone.key)) return cur;
      const back = cur.slice();
      back.splice(Math.min(i, back.length), 0, gone);
      return back;
    },
  };
}

/** Move line i up (-1) or down (+1). */
export function moveLine(lines: SLine[], i: number, dir: -1 | 1): SLine[] {
  const j = i + dir;
  if (i < 0 || i >= lines.length || j < 0 || j >= lines.length) return lines;
  const out = lines.slice();
  [out[i], out[j]] = [out[j], out[i]];
  return out;
}

/** The punch mark on line i, on or off. */
export function togglePunch(lines: SLine[], i: number): SLine[] {
  if (i < 0 || i >= lines.length) return lines;
  const out = lines.slice();
  out[i] = { ...lines[i], punch: !lines[i].punch };
  return out;
}

/** Material's +: one of his lines after line `at` (or at the end), word for word, with its source. */
export function pullLine(lines: SLine[], at: number | null, text: string, from: string | null, label: string | null): { lines: SLine[]; focus: number } {
  const i = at == null ? lines.length - 1 : at;
  const r = addLine(lines, i, text.trim());
  const l = r.lines[r.focus];
  r.lines[r.focus] = { ...l, by: 'him', ...(from ? { from, label: label || null } : {}) };
  return r;
}

/** Whether these words are already a line of the rant (Material reads "in the rant"). */
export const inRant = (lines: SLine[], text: string): boolean => {
  const t = clean(text).toLowerCase();
  return !!t && lines.some((l) => clean(l.text).toLowerCase() === t);
};

/** To the rant (a card's action): all its lines at the end, each with its source. */
export function appendMaterial(lines: SLine[], texts: string[], from: string | null, label: string | null): SLine[] {
  const add = texts.map((t) => t.trim()).filter(Boolean)
    .map((text) => ({ key: newKey(), text, by: 'him' as By, ...(from ? { from, label: label || null } : {}) }));
  return [...lines.filter((l) => l.text.trim()), ...add];
}

/** Use this (applyOption): Claude's version replaces line i, badged. Add after / Add it after: it goes under line i. */
export function applyOption(lines: SLine[], i: number, text: string, after = false): { lines: SLine[]; focus: number } {
  if (i < 0 || i >= lines.length) return { lines, focus: i };
  const out = lines.slice();
  const line: SLine = { key: newKey(), text: text.trim(), by: 'ai' };
  if (after) {
    out.splice(i + 1, 0, line);
    return { lines: out, focus: i + 1 };
  }
  out[i] = { ...line, key: lines[i].key, punch: lines[i].punch };
  return { lines: out, focus: i };
}

/** A drafted line from the worker (pk-worker rant_draft): `fixed` = one of his lines sent as fixed (always his own text
 *  back), `his` = quoted from a source, else Claude's. */
export type Drafted = { text: string; his?: boolean; fixed?: boolean; source?: string | null; source_label?: string | null };

/** Fold a draft into the script (§12.5: a draft only adds lines). `sent` = the keys of the lines sent as fixed, in
 *  order; the result's fixed lines are those, in that order. Each new line lands right after the fixed line before it
 *  in the result (before the first one: at the top), wherever that line is now; a line he cut meanwhile anchors at
 *  the end. His lines are never replaced by the result's copy. Returns the lines and how many were added, or null when
 *  the result does not carry the fixed lines in order (the server refuses that; this refuses it again). */
export function mergeDraft(current: SLine[], sent: string[], result: Drafted[]): { lines: SLine[]; added: number; keys: string[] } | null {
  const fixed = result.filter((l) => l.fixed);
  if (fixed.length !== sent.length) return null;
  const groups = new Map<string | null, SLine[]>();
  let anchor: string | null = null;
  let k = 0;
  for (const l of result) {
    if (l.fixed) { anchor = sent[k++]; continue; }
    const text = (l.text || '').trim();
    if (!text) continue;
    const line: SLine = l.his && l.source
      ? { key: newKey(), text, by: 'him', from: l.source, label: l.source_label || null }
      : { key: newKey(), text, by: 'ai' };
    const g = groups.get(anchor) || [];
    g.push(line);
    groups.set(anchor, g);
  }
  const out: SLine[] = [...(groups.get(null) || [])];
  const placed = new Set<string | null>([null]);
  for (const l of current) {
    out.push(l);
    if (groups.has(l.key)) { out.push(...(groups.get(l.key) as SLine[])); placed.add(l.key); }
  }
  for (const [a, g] of groups) if (!placed.has(a)) out.push(...g);
  const keys = [...groups.values()].flat().map((l) => l.key);
  return { lines: out, added: keys.length, keys };
}

/** What a save writes: the lines with words, as text and as their marks, aligned (blank lines are not lines). */
export function toSave(lines: SLine[]): { text: string; marks: Mark[]; lines: string[] } {
  const kept = lines.filter((l) => l.text.trim());
  const marks = kept.map((l) => {
    const m: Mark = {};
    if (l.by) m.by = l.by;
    if (l.from) m.from = l.from;
    if (l.label) m.label = l.label;
    if (l.edited) m.edited = true;
    if (l.punch) m.punch = true;
    return m;
  });
  const texts = kept.map((l) => l.text.replace(/\s+/g, ' ').trim());
  return { text: texts.join('\n'), marks, lines: texts };
}

/** Whether the page differs from what was last saved (the words or any mark). */
export function changed(lines: SLine[], saved: DocLine[] | null | undefined): boolean {
  const a = toSave(lines);
  const b = toSave(fromDoc(saved || []));
  return a.text !== b.text || JSON.stringify(a.marks) !== JSON.stringify(b.marks);
}

/** The version's source: a version holding any of Claude's lines keeps `rant_draft`, else he wrote it (`rant_write`). */
export const sourceOf = (lines: SLine[]): 'rant_draft' | 'rant_write' =>
  lines.some((l) => l.by === 'ai' && l.text.trim()) ? 'rant_draft' : 'rant_write';

/** Claude's lines this save adds that the last save did not have (the learning loop: kept lines). */
export function newAiLines(lines: SLine[], saved: DocLine[] | null | undefined): string[] {
  const had = new Set((saved || []).filter((l) => l.by === 'ai').map((l) => clean(l.text)));
  return lines.filter((l) => l.by === 'ai' && l.text.trim() && !had.has(clean(l.text))).map((l) => l.text.trim());
}

/** Marks carried onto a version written elsewhere (Pick a hook, Make it a rant): a line whose words were there keeps its
 *  mark; a new line is `newBy` (Claude's, from a session) or not known. Duplicates take the marks in order. */
export function carryMarks(old: SLine[], texts: string[], newBy: By | null): SLine[] {
  const pool = new Map<string, SLine[]>();
  for (const l of old) {
    const k = clean(l.text);
    pool.set(k, [...(pool.get(k) || []), l]);
  }
  return texts.map((text) => {
    const hit = pool.get(clean(text))?.shift();
    if (hit) return { ...hit, text };
    return newBy ? { key: newKey(), text, by: newBy } : { key: newKey(), text };
  });
}

/** The lines a draft is sent as fixed: every line with words, in order (§12.5). */
export const keepOf = (lines: SLine[]): { texts: string[]; keys: string[] } => {
  const k = lines.filter((l) => l.text.trim());
  return { texts: k.map((l) => l.text.replace(/\s+/g, ' ').trim()), keys: k.map((l) => l.key) };
};

/** The run time at his pace: "0:48". */
export function runTime(lines: string[], wpm: number = DEFAULT_WPM): string {
  const words = lines.reduce((n, l) => n + wordCount(l), 0);
  return mmss((words / (wpm > 0 ? wpm : DEFAULT_WPM)) * 60);
}

/** The hook line (§12.4): "Hook lands at 2.1 s · runs 0:48" / "Hook lands late: 4.2 s · runs 0:48". */
export function hookLine(word: string, lines: string[], wpm: number = DEFAULT_WPM): string {
  return `${word} · runs ${runTime(lines, wpm)}`;
}

/** What shows under a line: a source, an AI badge, the Yours mark, or nothing. */
export type Prov = { k: 'from'; label: string; edited: boolean } | { k: 'ai' } | { k: 'yours' } | { k: 'none' };
export function provOf(l: DocLine): Prov {
  if (l.by === 'ai') return { k: 'ai' };
  if (l.by === 'yours') return { k: 'yours' };
  if (l.by === 'him' && l.from) return { k: 'from', label: l.label || 'your material', edited: !!l.edited };
  return { k: 'none' };
}
/** "from Free employee v3" / "from Free employee v3 · edited". */
export const provWords = (p: Prov): string => (p.k === 'from' ? `from ${p.label}${p.edited ? ' · edited' : ''}` : '');

/** VoiceOver for a line: "Line 3 of 8, punch. From Free employee v3: …" / "…, by Claude: …". */
export function lineA11y(l: DocLine, i: number, n: number): string {
  const p = provOf(l);
  const who = p.k === 'from' ? ` From ${p.label}${p.edited ? ', edited' : ''}.` : p.k === 'ai' ? ' By Claude.' : p.k === 'yours' ? ' Yours.' : '';
  return `Line ${i + 1} of ${n}${l.punch ? ', punch' : ''}.${who} ${l.text}`;
}

// ---------------------------------------------------------------- the local draft (nothing typed is lost)
/** What is kept on the phone per topic while he types (Documents/script/<topic>.json; localStorage on the web). */
export type LocalDraft = { v: 1; topic: string; rant: string | null; base: string | null; at: number; lines: DocLine[]; pending: boolean };

/** The draft to keep for this page state. */
export function localDraft(topic: string, rant: string | null, base: string | null, lines: SLine[], pending: boolean): LocalDraft {
  return { v: 1, topic, rant, base, at: Date.now(), pending, lines: lines.map(({ key: _k, ...l }) => l) };
}

/** On open: the phone's draft wins when it was made over the version the server has now and it differs; a version saved
 *  since (another device, a session) wins over an older local draft. A draft of a rant that has no version yet wins
 *  when the server has none either. */
export function pickOpen(local: LocalDraft | null, serverRant: string | null, serverUnit: string | null, serverLines: DocLine[] | null): 'local' | 'server' {
  if (!local || !local.pending || !local.lines.length) return 'server';
  if ((local.rant || null) !== (serverRant || null) && local.rant) return 'server';
  if ((local.base || null) !== (serverUnit || null)) return 'server';
  return changed(fromDoc(local.lines), serverLines) ? 'local' : 'server';
}

/** Parse a stored draft; anything else is none. */
export function parseLocal(raw: string | null | undefined): LocalDraft | null {
  if (!raw) return null;
  try {
    const d = JSON.parse(raw);
    if (!d || d.v !== 1 || typeof d.topic !== 'string' || !Array.isArray(d.lines)) return null;
    const lines = d.lines.filter((l: any) => l && typeof l.text === 'string');
    return { v: 1, topic: d.topic, rant: typeof d.rant === 'string' ? d.rant : null, base: typeof d.base === 'string' ? d.base : null, at: Number(d.at) || 0, pending: !!d.pending, lines };
  } catch {
    return null;
  }
}

/** A file name for a topic's draft: the id's safe characters only. */
export const draftFile = (topic: string): string => `${topic.replace(/[^a-z0-9]/gi, '_').slice(0, 80)}.json`;
