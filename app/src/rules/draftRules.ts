// The new draft, compared with his words (docs/design/SPEC-2026-11-17-one-notebook.md screen 4 and §5). A round
// returns a whole new draft; the screen shows it with every changed range underlined, lets him put any change back,
// pick another of 3 options, and uses the text he sees. These rules are pure: which ranges differ (lines first, then
// words inside the lines that differ), which of them sit on a line he marked, what the page shows for a set of
// choices, and the text that "Use it" saves. No React: tests/draftRules.test.mjs runs them under node, with a --red
// arm per rule.
//
// "Anything you didn't mark stays word for word" is made true here, not hoped for: a change on a line he put no mark
// on starts put back (`initialChoices`), and the screen says how many.

import { locate } from './markRules.ts';

/** One changed range: `mine[a0, a1)` became `draft[b0, b1)`. With `options`, the whole takes that can stand in
 *  `draft[b0, b1)` (the first is what the draft holds). */
export type Hunk = { a0: number; a1: number; b0: number; b1: number; options?: string[] };
/** What a hunk shows: his words, or the draft's (0), or another of its options (1, 2). */
export type Choice = 'mine' | number;
/** The 3 options of one item of a round, as the worker sends them (`result.options`). */
export type RoundOption = { item?: number; quote?: string; texts: string[]; at?: number };

const unix = (s?: string | null): string => (s || '').replace(/\r\n?/g, '\n');
/** A table bigger than this is not built: the block is one change (a bit is a few hundred lines; this is the guard). */
const MAX_CELLS = 4_000_000;
/** Two changes with at most this many unchanged words between them, on one line, read as one. */
const JOIN_WORDS = 2;

type Tok = { t: string; s: number };

function lineToks(text: string): Tok[] {
  const out: Tok[] = [];
  let s = 0;
  for (;;) {
    const nl = text.indexOf('\n', s);
    if (nl < 0) { out.push({ t: text.slice(s), s }); break; }
    out.push({ t: text.slice(s, nl + 1), s });
    s = nl + 1;
  }
  return out;
}

function wordToks(text: string, base: number): Tok[] {
  const out: Tok[] = [];
  const re = /\s+|[^\s]+/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) out.push({ t: m[0], s: base + m.index });
  return out;
}

/** The pairs of token runs that differ, by a longest common run of equal tokens. Each pair is `[i0, i1, j0, j1]`
 *  (token indexes). Lines are compared with their trailing spaces left out, so a stripped space is not a change. */
function diffToks(a: Tok[], b: Tok[], same: (x: string, y: string) => boolean): [number, number, number, number][] {
  let pre = 0;
  while (pre < a.length && pre < b.length && same(a[pre].t, b[pre].t)) pre++;
  let suf = 0;
  while (suf < a.length - pre && suf < b.length - pre && same(a[a.length - 1 - suf].t, b[b.length - 1 - suf].t)) suf++;
  const n = a.length - pre - suf;
  const m = b.length - pre - suf;
  if (!n && !m) return [];
  if (!n || !m || (n + 1) * (m + 1) > MAX_CELLS) return [[pre, pre + n, pre, pre + m]];
  const w = m + 1;
  const L = new Int32Array((n + 1) * w);
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      L[i * w + j] = same(a[pre + i].t, b[pre + j].t) ? L[(i + 1) * w + j + 1] + 1 : Math.max(L[(i + 1) * w + j], L[i * w + j + 1]);
    }
  }
  const out: [number, number, number, number][] = [];
  let i = 0;
  let j = 0;
  let open: [number, number] | null = null;
  const close = () => { if (open) { out.push([pre + open[0], pre + i, pre + open[1], pre + j]); open = null; } };
  while (i < n || j < m) {
    if (i < n && j < m && same(a[pre + i].t, b[pre + j].t)) { close(); i++; j++; continue; }
    if (!open) open = [i, j];
    if (j >= m || (i < n && L[(i + 1) * w + j] >= L[i * w + j + 1])) i++; else j++;
  }
  close();
  return out;
}

const startOf = (toks: Tok[], k: number, len: number) => (k < toks.length ? toks[k].s : len);
const sameLine = (x: string, y: string) => x.replace(/[ \t]+(\n?)$/, '$1') === y.replace(/[ \t]+(\n?)$/, '$1');

/** Where a place in the draft's unchanged text is in his (the hunks before it shift it). */
function toMine(hs: Hunk[], pos: number): number {
  let shift = 0;
  for (const h of hs) {
    if (h.b1 <= pos) shift = h.a1 - h.b1; else break;
  }
  return pos + shift;
}

/** Every range of the draft that is not his words, in order. Lines first (a line he did not touch is never cut up),
 *  then words inside the lines that differ; two changes a word or two apart on one line are one; a change that only
 *  took words away is widened to the word before it, so there is something to tap. `options` (the worker's
 *  `result.options`) are put on the change that holds them. */
export function hunks(mineRaw: string, draftRaw: string, options?: readonly RoundOption[] | null): Hunk[] {
  // the blank end of either text is never a change (a version is saved without it)
  const mine = unix(mineRaw).replace(/\s+$/, '');
  const draft = unix(draftRaw).replace(/\s+$/, '');
  const la = lineToks(mine);
  const lb = lineToks(draft);
  let out: Hunk[] = [];
  for (const [i0, i1, j0, j1] of diffToks(la, lb, sameLine)) {
    const a0 = startOf(la, i0, mine.length);
    const a1 = startOf(la, i1, mine.length);
    const b0 = startOf(lb, j0, draft.length);
    const b1 = startOf(lb, j1, draft.length);
    const wa = wordToks(mine.slice(a0, a1), a0);
    const wb = wordToks(draft.slice(b0, b1), b0);
    const inner = diffToks(wa, wb, (x, y) => x === y).map(([p0, p1, q0, q1]) => ({
      a0: startOf(wa, p0, a1), a1: startOf(wa, p1, a1), b0: startOf(wb, q0, b1), b1: startOf(wb, q1, b1),
    }));
    // join two changes a word or two apart on the same line
    const joined: Hunk[] = [];
    for (const h of inner) {
      const last = joined[joined.length - 1];
      if (last) {
        const gap = draft.slice(last.b1, h.b0);
        const words = gap.trim() ? gap.trim().split(/\s+/).length : 0;
        if (!gap.includes('\n') && words <= JOIN_WORDS) { last.a1 = h.a1; last.b1 = h.b1; continue; }
      }
      joined.push({ ...h });
    }
    out.push(...joined);
  }
  // whitespace alone is not a change he needs to see (a trailing space, a doubled blank line at the very end)
  out = out.filter((h) => mine.slice(h.a0, h.a1).trim() !== draft.slice(h.b0, h.b1).trim() || /\n/.test(mine.slice(h.a0, h.a1)) !== /\n/.test(draft.slice(h.b0, h.b1)));
  // a change that only took words away: widen it to the word before (or after) so the page has words to underline
  out = out.map((h) => {
    if (draft.slice(h.b0, h.b1).trim()) return h;
    const before = draft.slice(0, h.b0);
    const back = /[^\s]+[^\S\n]*$/.exec(before);
    if (back && !before.slice(back.index).includes('\n')) {
      const d = h.b0 - back.index;
      return { ...h, a0: h.a0 - d, b0: h.b0 - d };
    }
    const fwd = /^[^\S\n]*[^\s]+/.exec(draft.slice(h.b1));
    if (fwd) return { ...h, a1: h.a1 + fwd[0].length, b1: h.b1 + fwd[0].length };
    return h;
  });
  // keep them apart after widening
  const clean: Hunk[] = [];
  for (const h of out) {
    const last = clean[clean.length - 1];
    if (last && h.b0 < last.b1) { last.a1 = Math.max(last.a1, h.a1); last.b1 = Math.max(last.b1, h.b1); } else clean.push(h);
  }
  return withOptions(mine, draft, clean, options || []);
}

/** The options of a round put on the changes that hold them: the take in the draft is found (nearest its hint), the
 *  changes it overlaps become one change that covers it, and that change's `options` are whole replacements for its
 *  range. An option whose first take is not in the draft is left out (the worker drops those; an older one may not). */
function withOptions(mine: string, draft: string, hs: Hunk[], options: readonly RoundOption[]): Hunk[] {
  let out = hs.map((h) => ({ ...h }));
  for (const o of options) {
    const texts = (o.texts || []).filter((t) => typeof t === 'string' && t.trim());
    if (texts.length < 2) continue;
    const s = locate(draft, { text: texts[0], at: o.at || 0 });
    if (s == null) continue;
    const e = s + texts[0].length;
    const hit = out.filter((h) => h.b0 < e && h.b1 > s);
    if (hit.some((h) => h.options)) continue;
    const b0 = Math.min(s, ...hit.map((h) => h.b0));
    const b1 = Math.max(e, ...hit.map((h) => h.b1));
    const rest = out.filter((h) => !hit.includes(h));
    const before = rest.filter((h) => h.b1 <= b0);
    const a0 = hit.length && hit[0].b0 <= b0 ? hit[0].a0 : toMine(before, b0);
    const last = hit[hit.length - 1];
    const a1 = last && last.b1 >= b1 ? last.a1 : last ? last.a1 + (b1 - last.b1) : a0 + (b1 - b0);
    const merged: Hunk = { a0, a1, b0, b1, options: texts.map((t) => draft.slice(b0, s) + t + draft.slice(e, b1)) };
    out = [...rest, merged].sort((x, y) => x.b0 - y.b0);
  }
  return out;
}

/** The text for a set of choices: the draft, with each change he put back as his words and each picked option in
 *  place. This is what "Use it" saves. */
export function compose(mineRaw: string, draftRaw: string, hs: readonly Hunk[], choices: readonly Choice[]): string {
  const mine = unix(mineRaw);
  const draft = unix(draftRaw);
  let out = '';
  let pos = 0;
  hs.forEach((h, i) => {
    out += draft.slice(pos, h.b0) + shown(mine, draft, h, choices[i] ?? 0);
    pos = h.b1;
  });
  return out + draft.slice(pos);
}

/** What one change shows for a choice. */
export function shown(mine: string, draft: string, h: Hunk, c: Choice): string {
  if (c === 'mine') return mine.slice(h.a0, h.a1);
  return (h.options && h.options[c]) ?? draft.slice(h.b0, h.b1);
}

/** A change's name across rounds: where his words were and what they said (his text does not move between rounds). */
export const hunkKey = (mineRaw: string, h: Hunk): string => `${h.a0}:${unix(mineRaw).slice(h.a0, h.a1)}`;

/** Which changes sit on a line he marked. A mark's line is every line its words touch. `marks` are the round's (found
 *  again in his words); when one cannot be found, or the draft was made from older words (`loose`), every change
 *  counts as marked: the phone cannot tell, so it puts nothing back by itself. */
export function onMarkedLines(mineRaw: string, hs: readonly Hunk[], marks: readonly { type: string; text: string; at: number }[], loose = false): boolean[] {
  const mine = unix(mineRaw);
  if (loose) return hs.map(() => true);
  const spans: [number, number][] = [];
  for (const m of marks) {
    if (m.type === 'keep') continue;
    const s = locate(mine, m);
    if (s == null) return hs.map(() => true);
    const ls = mine.lastIndexOf('\n', s - 1) + 1;
    const nl = mine.indexOf('\n', s + m.text.length - 1);
    spans.push([ls, nl < 0 ? mine.length : nl]);
  }
  return hs.map((h) => spans.some(([ls, le]) => (h.a0 === h.a1 ? h.a0 >= ls && h.a0 <= le + 1 : h.a0 <= le && h.a1 > ls)));
}

/** How the page opens on a draft: a change he put back before stays back (`putBack`: the keys of `hunkKey`); a change
 *  he took after all on this same draft stays taken (`used`); a change on a line he did not mark is put back by
 *  itself; the rest show the draft. `auto` = the changes that are outside his marks. */
export function initialChoices(mineRaw: string, hs: readonly Hunk[], marked: readonly boolean[], putBack: ReadonlySet<string>, used?: ReadonlySet<string>): { choices: Choice[]; auto: boolean[] } {
  const choices = hs.map((h, i): Choice => {
    const k = hunkKey(mineRaw, h);
    if (putBack.has(k)) return 'mine';
    if (used?.has(k)) return 0;
    return marked[i] ? 0 : 'mine';
  });
  return { choices, auto: hs.map((_, i) => !marked[i]) };
}

/** The two lines under the draft (the mockup's words). `auto` = the changes outside his marks; the ones still put
 *  back are counted on the second line. */
export function changesLines(choices: readonly Choice[], auto: readonly boolean[]): { main: string; also: string | null } {
  const n = choices.filter((c) => c !== 'mine').length;
  const outside = auto.filter((a, i) => a && choices[i] === 'mine').length;
  // "where you marked" only while every change shown is on a line he marked
  const allMarked = choices.every((c, i) => c === 'mine' || !auto[i]);
  const where = !allMarked ? '' : n === 1 ? ', where you marked' : n === 2 ? ', both where you marked' : ', all where you marked';
  const main = n === 0
    ? (choices.length ? 'Every change is put back: this is your page as you wrote it.' : 'Nothing changed in this draft.')
    : `${n} ${n === 1 ? 'change' : 'changes'}${where}. Tap ${n === 1 ? 'it' : 'one'} to put yours back.`;
  const also = outside ? `${outside} ${outside === 1 ? 'change' : 'changes'} outside your marks ${outside === 1 ? 'was' : 'were'} put back.` : null;
  return { main, also };
}

/** One run of a paragraph: unchanged words, or (part of) change `hunk` showing the draft's words (`new`) or his (`mine`). */
export type Run = { text: string; hunk: number | null; kind: 'same' | 'new' | 'mine' };
/** One line of the page. `ends` = the changes whose last words are on this line (their buttons go under it). */
export type Para = { runs: Run[]; ends: number[] };

/** The page for a set of choices, line by line, each change a run of its own (a change over two lines is a run on
 *  each). The lines joined with a newline are `compose`'s text. */
export function paragraphs(mineRaw: string, draftRaw: string, hs: readonly Hunk[], choices: readonly Choice[]): Para[] {
  const mine = unix(mineRaw);
  const draft = unix(draftRaw);
  const paras: Para[] = [{ runs: [], ends: [] }];
  const push = (text: string, hunk: number | null, kind: Run['kind']) => {
    const parts = text.split('\n');
    parts.forEach((part, k) => {
      if (k > 0) paras.push({ runs: [], ends: [] });
      if (part) paras[paras.length - 1].runs.push({ text: part, hunk, kind });
    });
    if (hunk != null) {
      // the line that holds the change's last words (a change that ends in a newline ends on the line before)
      let at = paras.length - 1;
      while (at > 0 && !paras[at].runs.some((r) => r.hunk === hunk)) at--;
      if (!paras[at].ends.includes(hunk)) paras[at].ends.push(hunk);
    }
  };
  let pos = 0;
  hs.forEach((h, i) => {
    push(draft.slice(pos, h.b0), null, 'same');
    const c = choices[i] ?? 0;
    push(shown(mine, draft, h, c), i, c === 'mine' ? 'mine' : 'new');
    pos = h.b1;
  });
  push(draft.slice(pos), null, 'same');
  return paras;
}

/** "Try again" on a change: the words of the draft another take is asked for (the change as it stands in the draft,
 *  its outer spaces left out). Null when the change holds no words in the draft. */
export function againTarget(draftRaw: string, h: Hunk): { text: string; at: number } | null {
  const draft = unix(draftRaw);
  let a = h.b0;
  let b = h.b1;
  while (a < b && /\s/.test(draft[a])) a++;
  while (b > a && /\s/.test(draft[b - 1])) b--;
  return a < b ? { text: draft.slice(a, b), at: a } : null;
}

/** A change in a few words, for a button's label and VoiceOver ("Put mine back: already in trouble with…"). */
export function brief(text: string, max = 40): string {
  const one = text.replace(/\s+/g, ' ').trim();
  return one.length > max ? `${one.slice(0, max - 1)}…` : one;
}
