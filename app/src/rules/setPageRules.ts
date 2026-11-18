// A set is a page (docs/design/SPEC-2026-11-17-one-notebook.md screen 5): a numbered list of bits with his note under
// each and its time, and a last row he types in: `@` plus letters (or just letters) lists the bits that match, and
// "New bit called …" makes one. These rules are pure: what he is asking for as he types, which bits are offered and in
// what order, the words of the line under the title, and the row a tap adds. No React: tests/setPageRules.test.mjs runs
// them under node, with a --red arm per rule.

import { aboutMin } from './notebookRules.ts';

/** A bit as the list knows it (GET /api/jokes). */
export type BitLike = { id: string; title: string; short_name?: string | null; short_title?: string | null };
/** How many bits the typing row offers at most (a short list under the thumb, not a search screen). */
export const MAX_OFFERS = 5;
/** A new bit's name, at most (a title; the words go on its own page). */
export const MAX_NAME = 80;

const fold = (s: string) => s.toLowerCase().normalize('NFKD').replace(/[̀-ͯ]/g, '').replace(/[^a-z0-9]+/g, ' ').trim();

/** What he typed in the last row, as a search: the `@` is optional and spaces at the ends do not count. Empty = he has
 *  asked for nothing yet (no list is drawn). */
export function asked(text: string): string {
  return (text || '').replace(/^\s*@\s*/, '').replace(/\s+/g, ' ').trim();
}

/** The name a bit goes by on a set (its short name when it has one). */
export const nameOf = (b: BitLike): string => (b.short_name || '').trim() || (b.short_title || '').trim() || (b.title || '').split(' (')[0].trim() || 'Untitled';

/** The bits offered for what he typed: never one already in the set; a name that starts with his letters first, then
 *  one with a word that starts with them, then one that holds them anywhere; ties keep the list's own order (the bit
 *  he touched last first); at most MAX_OFFERS. */
export function offers<T extends BitLike>(bits: readonly T[], inSet: ReadonlySet<string> | readonly string[], text: string, max = MAX_OFFERS): T[] {
  const q = fold(asked(text));
  if (!q) return [];
  const taken = inSet instanceof Set ? inSet : new Set(inSet as readonly string[]);
  const ranked: { b: T; rank: number; i: number }[] = [];
  bits.forEach((b, i) => {
    if (taken.has(b.id)) return;
    const names = [fold(nameOf(b)), fold(b.title || '')];
    let rank = 3;
    for (const n of names) {
      if (!n) continue;
      if (n.startsWith(q)) rank = Math.min(rank, 0);
      else if (n.split(' ').some((w) => w.startsWith(q)) || n.includes(` ${q}`)) rank = Math.min(rank, 1);
      else if (n.includes(q)) rank = Math.min(rank, 2);
    }
    if (rank < 3) ranked.push({ b, rank, i });
  });
  return ranked.sort((x, y) => x.rank - y.rank || x.i - y.i).slice(0, max).map((x) => x.b);
}

/** The name "New bit called …" would give: his letters as he typed them, cut to a title's length; null when he typed
 *  nothing, or a bit already goes by exactly that name (it is offered above instead; a second one would be a twin). */
export function newBitName(bits: readonly BitLike[], text: string): string | null {
  const q = asked(text).slice(0, MAX_NAME).trim();
  if (!q) return null;
  const f = fold(q);
  if (!f || bits.some((b) => fold(nameOf(b)) === f || fold(b.title || '') === f)) return null;
  return q;
}

/** The button's words. */
export const newBitLabel = (name: string): string => `New bit called “${name}”`;

/** The line under the title (the mockup's words): "5 bits · about 7 min · you have 8". The slot is left out when the
 *  set has none; a set that runs over says by how much instead. */
export function summaryLine(n: number, runtimeS: number | null | undefined, targetS: number | null | undefined): string {
  const parts = [`${n} ${n === 1 ? 'bit' : 'bits'}`];
  const about = n ? aboutMin(runtimeS) : null;
  if (about) parts.push(about);
  if (targetS && targetS > 0) {
    const slot = Math.round(targetS / 60);
    const over = runtimeS && runtimeS > targetS ? Math.ceil((runtimeS - targetS) / 60) : 0;
    parts.push(over > 0 ? `${over} min over your ${slot}` : `you have ${slot}`);
  }
  return parts.join(' · ');
}

/** The hint under the list (the mockup's line; the grip is the handle that moves a row). */
export const HINT = 'Type a line to add a bit. Hold the grip to move one.';
