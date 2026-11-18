// The Redline loop on a bit (owner 2026-11-16: "when i say redline, i mean our repo redline"; 2026-11-17: "lets make it a
// whole new draft"; SPEC 2026-11-16 B12, §3.2 and Slice 3 as built). He selects words in his document (or puts the
// caret on a line with nothing selected) and marks them Change (an optional note), Ask (a question) or Keep; "Take it
// elsewhere" (Redline's Fork) sends the words to a set or a new bit and is never a mark. The marks collect in a tray on
// the phone and survive leaving the page; Send sends them once, in document order, each anchored to the words it
// quotes; what comes back is a whole new draft and the answers to his questions. These rules are pure: what a mark
// is on, where it sits in the text now (by its own words: lines have no ids, B1), what Send sends, which lines of the
// draft differ from his, what a round's state is. No React: tests/markRules.test.mjs runs them under node, with a
// --red arm per rule.

export type MarkType = 'change' | 'ask' | 'keep';
/** How he asked for a Change from the selection's buttons (SPEC 2026-11-17 one notebook §4; pk-creative redline
 *  `Intent`): absent on a plain Change with a note. `again` = another take on a range of a draft. */
export type Intent = 'funnier' | 'shorter' | 'options' | 'again';
export const INTENTS: readonly Intent[] = ['funnier', 'shorter', 'options', 'again'];
/** One mark. `at` is where its words started when he made it (a UTF-16 index, the phone's own count): a hint, never
 *  the truth; the words are. */
export type Mark = { id: string; type: MarkType; text: string; at: number; note: string; carried?: boolean; intent?: Intent };
/** The marks on one text: his bit (`on` null) or the draft of a round (`on` = that round's job). */
export type MarkSet = { v: 1; bit: string; on: string | null; marks: Mark[]; at: string };

export const MAX_MARKS = 20;
export const MAX_NOTE = 500;
export const MAX_QUOTE = 4000;
export const TYPE_WORD: Record<MarkType, string> = { change: 'Change', ask: 'Ask', keep: 'Keep' };

const unix = (s?: string | null): string => (s || '').replace(/\r\n?/g, '\n');

/** What a mark made now would be on: the selected words with the spaces at their ends left out, or, with nothing
 *  selected, the whole line the caret is on (its own spaces left out). Null on a blank line or nothing at all. */
export function targetOf(text: string, sel: { start: number; end: number } | null): { text: string; at: number } | null {
  if (!sel) return null;
  const t = unix(text);
  let a = Math.max(0, Math.min(sel.start, sel.end, t.length));
  let b = Math.max(0, Math.min(Math.max(sel.start, sel.end), t.length));
  if (a === b) {
    a = t.lastIndexOf('\n', a - 1) + 1;
    const nl = t.indexOf('\n', b);
    b = nl < 0 ? t.length : nl;
  }
  while (a < b && /\s/.test(t[a])) a++;
  while (b > a && /\s/.test(t[b - 1])) b--;
  if (a >= b) return null;
  return { text: t.slice(a, b), at: a };
}

/** Where a mark's words are in the text now: at its hint when they still stand there, else the occurrence nearest the
 *  hint (the server's rule, pk_creative::redline::locate), else null (he rewrote them: the mark is gone). */
export function locate(text: string, m: { text: string; at: number }): number | null {
  const t = unix(text);
  if (!m.text) return null;
  if (t.startsWith(m.text, m.at)) return m.at;
  let best: number | null = null;
  for (let i = t.indexOf(m.text); i >= 0; i = t.indexOf(m.text, i + 1)) {
    if (best == null || Math.abs(i - m.at) < Math.abs(best - m.at)) best = i;
  }
  return best;
}

const overlap = (a: { s: number; e: number }, b: { s: number; e: number }) => a.s < b.e && b.s < a.e;

/** A new mark, or why not: at most MAX_MARKS; the same words with the same type once; a Keep and a Change on the same
 *  words are a contradiction (an Ask on kept words is fine). */
export function addMark(text: string, set: Mark[], m: Mark): { marks: Mark[]; error?: string } {
  if (set.length >= MAX_MARKS) return { marks: set, error: `At most ${MAX_MARKS} marks in one send` };
  if ([...m.text].length > MAX_QUOTE) return { marks: set, error: 'Too much in one mark: mark less' };
  const at = locate(text, m);
  if (at == null) return { marks: set, error: 'Those words are not in the bit' };
  const me = { s: at, e: at + m.text.length };
  for (const x of set) {
    const xa = locate(text, x);
    if (xa == null) continue;
    const it = { s: xa, e: xa + x.text.length };
    if (x.type === m.type && x.text === m.text && xa === at) return { marks: set, error: `Already marked ${TYPE_WORD[m.type]}` };
    const clash = (x.type === 'keep' && m.type === 'change') || (x.type === 'change' && m.type === 'keep');
    if (clash && overlap(me, it)) return { marks: set, error: 'Those words are marked Keep and Change: remove one first' };
  }
  return { marks: [...set, { ...m, at, note: m.type === 'keep' ? '' : m.note.slice(0, MAX_NOTE) }] };
}

export const removeMark = (set: Mark[], id: string): Mark[] => set.filter((m) => m.id !== id);
export const noteMark = (set: Mark[], id: string, note: string): Mark[] =>
  set.map((m) => (m.id === id && m.type !== 'keep' ? { ...m, note: note.slice(0, MAX_NOTE) } : m));

/** The marks as they sit in the text now, in document order; the ones whose words are gone at the end, with `gone`. */
export function placed(text: string, set: Mark[]): { mark: Mark; start: number; end: number; gone: boolean }[] {
  const out = set.map((mark) => {
    const s = locate(text, mark);
    return s == null ? { mark, start: Infinity, end: Infinity, gone: true } : { mark, start: s, end: s + mark.text.length, gone: false };
  });
  return out.sort((a, b) => a.start - b.start || a.end - b.end);
}

/** The text cut into runs for drawing the marks as quiet highlights: each run has the type of the mark it is under
 *  (Keep wins over Change, Change over Ask, where two overlap), else `tint` on a line `tint` says changed (the draft's
 *  lines that are not his), else null. */
export function highlights(text: string, set: readonly Mark[], tint?: readonly boolean[]): { text: string; type: MarkType | 'tint' | null }[] {
  const t = unix(text);
  const rank: Record<MarkType, number> = { keep: 3, change: 2, ask: 1 };
  const at: (MarkType | 'tint' | null)[] = new Array(t.length).fill(null);
  if (tint) {
    let line = 0;
    for (let i = 0; i < t.length; i++) {
      if (t[i] === '\n') line++;
      else if (tint[line]) at[i] = 'tint';
    }
  }
  for (const p of placed(t, [...set])) {
    if (p.gone) continue;
    for (let i = p.start; i < p.end; i++) {
      const was = at[i];
      if (!was || was === 'tint' || rank[p.mark.type] > rank[was]) at[i] = p.mark.type;
    }
  }
  const out: { text: string; type: MarkType | 'tint' | null }[] = [];
  for (let i = 0; i < t.length; i++) {
    const last = out[out.length - 1];
    if (last && last.type === at[i]) last.text += t[i];
    else out.push({ text: t[i], type: at[i] });
  }
  return out;
}

/** What Send sends: the marks still in the text, in document order, with where they are now; the ones that are gone
 *  are left out and named. `error` when nothing can be sent (at least one Change or Ask; an Ask needs its question). */
export function sendBody(text: string, set: Mark[]): { marks: { type: MarkType; text: string; at: number; note?: string; intent?: Intent }[]; gone: Mark[]; error?: string } {
  const p = placed(text, set);
  const live = p.filter((x) => !x.gone);
  const gone = p.filter((x) => x.gone).map((x) => x.mark);
  const marks = live.map((x) => ({
    type: x.mark.type, text: x.mark.text, at: x.start,
    ...(x.mark.type !== 'keep' && x.mark.note.trim() ? { note: x.mark.note.trim() } : {}),
    // how he asked rides only on a Change (the server drops it anywhere else, and refuses a word it does not know)
    ...(x.mark.type === 'change' && x.mark.intent && INTENTS.includes(x.mark.intent) ? { intent: x.mark.intent } : {}),
  }));
  if (live.some((x) => x.mark.type === 'ask' && !x.mark.note.trim())) return { marks, gone, error: 'An Ask needs its question' };
  if (!live.some((x) => x.mark.type !== 'keep')) return { marks, gone, error: 'Mark something to change or ask about' };
  return { marks, gone };
}

/** "3 marks", "1 mark". */
export const markCount = (n: number) => `${n} ${n === 1 ? 'mark' : 'marks'}`;
/** The Send button's words. */
export const sendLabel = (n: number) => (n ? `Send ${markCount(n)}` : 'Send');

// ------------------------------------------------------------------ the round, as the server keeps it

/** GET /api/jokes/<id>/redline `round` (pk-api writes/redline.rs). */
export type RoundResult = {
  status?: string; why?: string | null; draft?: string | null; mode?: string;
  answers?: { item: number; text: string; question?: string; quote?: string }[];
  notes?: { item: number; was: string | null; why: string | null }[];
  marks?: { item: number; type: MarkType; text: string; at: number; note: string; carried?: boolean; intent?: Intent }[];
  /** The takes of each item that asked for 3 options (the first stands in the draft); only in such a round. */
  options?: { item: number; quote?: string; texts: string[]; at?: number }[];
  dropped?: { item: number | null; reason: string }[]; refused?: string[]; tries?: number;
};
export type Round = {
  job: string; status: string; unit: string; on: string | null; n: number; sent_at: string;
  marks: { type: MarkType; text: string; at: number; note: string; carried?: boolean; intent?: Intent }[];
  stale: boolean; newest: string | null; result: RoundResult | null; error?: string | null;
};
export type RoundState = 'none' | 'waiting' | 'ready' | 'refused' | 'failed';

/** What the page shows for a round: waiting on the worker, a draft ready, a draft the checks refused (or nothing to
 *  draft), or the job failed. */
export function roundState(r: Round | null | undefined): RoundState {
  if (!r) return 'none';
  if (r.status === 'failed') return 'failed';
  if (r.status !== 'done') return 'waiting';
  return r.result?.status === 'ok' && r.result.draft ? 'ready' : 'refused';
}

/** The one line the bit page shows for a round under his words. */
export function roundLine(r: Round | null | undefined): string | null {
  switch (roundState(r)) {
    case 'waiting': return 'Reading your marks…';
    case 'ready': return r?.stale ? 'A new draft is ready (made before your last edit)' : 'A new draft is ready';
    case 'refused': return r?.result?.why || 'No draft this time';
    case 'failed': return 'Couldn’t make a draft';
    default: return null;
  }
}

/** What "Use this draft" does: nothing without a draft; asks first when he saved a newer version after sending (the
 *  draft was made from older words and would sit on top of the newer ones: offered honestly, never applied by
 *  itself); else uses it at once (a new version, with Undo). */
export function planForUse(r: Round | null | undefined): 'none' | 'confirm' | 'use' {
  if (roundState(r) !== 'ready') return 'none';
  return r?.stale ? 'confirm' : 'use';
}

/** For "flip between the draft and your words": which lines of the draft are not lines of his (changed or new), by
 *  a longest common run of lines. Blank lines are never tinted. */
export function changedLines(mine: string, draft: string): boolean[] {
  const a = unix(mine).split('\n').map((l) => l.trimEnd());
  const b = unix(draft).split('\n').map((l) => l.trimEnd());
  const n = a.length;
  const m = b.length;
  // the classic table; bits are a few hundred lines at most
  const L: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) L[i][j] = a[i] === b[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
  const same = new Array(m).fill(false);
  for (let i = 0, j = 0; i < n && j < m;) {
    if (a[i] === b[j]) { same[j] = true; i++; j++; } else if (L[i + 1][j] >= L[i][j + 1]) i++; else j++;
  }
  return b.map((l, j) => !same[j] && l.trim().length > 0);
}

/** The Keeps of a round, put on its draft so he sees what was kept (round two carries them on). */
export function keepsOn(draft: string, r: Round | null | undefined): Mark[] {
  const out: Mark[] = [];
  for (const k of r?.marks || []) {
    if (k.type !== 'keep') continue;
    const at = locate(draft, { text: k.text, at: 0 });
    if (at != null) out.push({ id: `keep-${out.length}`, type: 'keep', text: k.text, at, note: '', carried: true });
  }
  return out;
}

/** "Put my marks on what I saved": the round's marks put back on his newer text, the ones he has since rewritten left
 *  out and counted. */
export function marksAgain(text: string, r: Round, mkId: (i: number) => string): { marks: Mark[]; lost: number } {
  const marks: Mark[] = [];
  let lost = 0;
  (r.marks || []).forEach((m, i) => {
    const at = locate(text, m);
    if (at == null) lost++;
    else marks.push({ id: mkId(i), type: m.type, text: m.text, at, note: m.note || '', ...(m.intent && m.intent !== 'again' ? { intent: m.intent } : {}) });
  });
  return { marks, lost };
}

// ------------------------------------------------------------------ the tray kept on the phone

/** The tray's file name: the marker FIRST, so a long bit id cut to the file-name limit can never make the two trays one. */
export const markSetName = (bit: string, on: string | null): string => (on ? `draft~${bit}` : bit);
export function parseMarkSet(raw: string | null | undefined): MarkSet | null {
  if (!raw) return null;
  try {
    const d = JSON.parse(raw);
    if (!d || d.v !== 1 || typeof d.bit !== 'string' || !Array.isArray(d.marks)) return null;
    const marks = d.marks.filter((m: any) => m && ['change', 'ask', 'keep'].includes(m.type) && typeof m.text === 'string' && m.text && typeof m.id === 'string')
      .map((m: any) => ({ id: m.id, type: m.type, text: m.text, at: Number.isFinite(m.at) ? m.at : 0, note: typeof m.note === 'string' ? m.note : '', ...(m.carried ? { carried: true } : {}), ...(INTENTS.includes(m.intent) ? { intent: m.intent } : {}) }));
    return { v: 1, bit: d.bit, on: typeof d.on === 'string' ? d.on : null, marks, at: typeof d.at === 'string' ? d.at : '' };
  } catch {
    return null;
  }
}
