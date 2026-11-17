// The walk-through (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §3.4, §4.4, Slice 5): one screen records sound or
// films a set, one bit at a time, and keeps a SPLIT each time he leaves a bit: `{joke_id, start_ms, end_ms}` on the
// recording's own position. For sound that position is the recorder's clock (lib/recorder elapsedMs, which holds
// still while a call has the microphone), so a call's pause is inside no split; for video it is the time since the
// camera began (the spec's known early bias, a device check). Going Back and forward again adds a second span for the
// same bit. After the take, the phone's own transcript is cut by those splits into what he said per bit (the After
// screen). Pure and tested (tests/runRules.test.mjs); app/run/[id].tsx and app/after/[rec].tsx are the wiring.
import { LOW_STORAGE_BYTES } from './filmRules.ts';

/** One span of the recording spent on one bit, in milliseconds from the recording's start. */
export type Split = { joke_id: string; start_ms: number; end_ms: number };
/** A bit of the set as the walk-through shows it (its order is the set's). */
export type RunBit = { id: string; name: string };
/** Where the walk-through is: the bit on screen, where its open span began (null while nothing records), the spans. */
export type Run = { i: number; at: number | null; splits: Split[] };

/** The server's cap (POST /api/sets/transcript): at most this many splits. */
export const SPLITS_MAX = 500;
/** A span shorter than this is a tap passing through a bit, not time spent on it. */
export const MIN_SPAN_MS = 250;

export const runIdle = (i = 0): Run => ({ i, at: null, splits: [] });

/** Recording began at position `pos` (0 for a fresh take): the bit on screen opens its span there. */
export function runStart(r: Run, pos: number): Run {
  return { ...r, at: Math.max(0, pos) };
}

/** Closes the open span at `pos` (nothing when no span is open or it is too short to count). */
function close(r: Run, pos: number, ids: readonly string[]): Split[] {
  if (r.at == null) return r.splits;
  const id = ids[r.i];
  const start = Math.max(0, Math.round(r.at));
  const end = Math.round(pos);
  if (!id || end - start < MIN_SPAN_MS) return r.splits;
  return [...r.splits, { joke_id: id, start_ms: start, end_ms: end }];
}

/** Next or Back: the bit on screen becomes `to` (kept inside the set). While recording, the bit he leaves gets its
 *  span up to `pos` and the new one opens at `pos`; before the disc it only moves the highlight. */
export function runGo(r: Run, to: number, pos: number, ids: readonly string[]): Run {
  const j = Math.max(0, Math.min(ids.length - 1, to));
  if (j === r.i) return r;
  if (r.at == null) return { ...r, i: j };
  return { i: j, at: pos, splits: close(r, pos, ids) };
}

/** The take ended at `pos`: every span, the last one closed. */
export function runEnd(r: Run, pos: number, ids: readonly string[]): Split[] {
  return close(r, pos, ids);
}

/** What goes to the server: whole milliseconds, inside the recording, in order, never overlapping, each on a bit of
 *  the set, at most SPLITS_MAX (the route refuses anything else, so the phone never sends it). */
export function splitsForSend(splits: readonly Split[], durationMs: number, ids: readonly string[]): Split[] {
  const on = new Set(ids);
  const dur = Math.max(0, Math.round(durationMs));
  const sorted = splits
    .filter((s) => on.has(s.joke_id) && Number.isFinite(s.start_ms) && Number.isFinite(s.end_ms))
    .map((s) => ({ joke_id: s.joke_id, start_ms: Math.max(0, Math.round(s.start_ms)), end_ms: Math.min(dur, Math.round(s.end_ms)) }))
    .sort((a, b) => a.start_ms - b.start_ms);
  const out: Split[] = [];
  let last = 0;
  for (const s of sorted) {
    const start = Math.max(s.start_ms, last);
    if (s.end_ms - start < 1) continue;
    out.push({ ...s, start_ms: start });
    last = s.end_ms;
    if (out.length >= SPLITS_MAX) break;
  }
  return out;
}

/** A transcript segment as the phone keeps it (seconds). */
export type Seg = { text: string; start: number; end: number };

/** What he said on each bit: every segment goes to the span its middle falls in; a segment in no span (before the
 *  first tap, in a pause) belongs to no bit. Keyed by joke id, the spans of one bit joined in time order. */
export function wordsBySplit(segs: readonly Seg[], splits: readonly Split[]): Record<string, string> {
  const out: Record<string, string[]> = {};
  for (const g of segs) {
    const mid = ((g.start + g.end) / 2) * 1000;
    const s = splits.find((x) => mid >= x.start_ms && mid < x.end_ms);
    const t = (g.text || '').trim();
    if (s && t) (out[s.joke_id] ||= []).push(t);
  }
  return Object.fromEntries(Object.entries(out).map(([k, v]) => [k, v.join(' ').replace(/\s+/g, ' ').trim()]));
}

const contentWords = (t: string) => (t.toLowerCase().match(/[\p{L}\p{N}']+/gu) || []).map((w) => w.replace(/'/g, '')).filter((w) => w.length >= 4);

/** He said it differently from the written bit: at least 4 of his spoken content words (four letters or more) are in
 *  no line of the bit, and they are at least 30% of what he said. A reading of the words, never a judgement on them. */
export function saidDifferently(said: string, written: string): boolean {
  const spoken = contentWords(said);
  if (!spoken.length) return false;
  const known = new Set(contentWords(written));
  const fresh = spoken.filter((w) => !known.has(w)).length;
  return fresh >= 4 && fresh / spoken.length >= 0.3;
}

/** One row of the After screen. `words` is null until the phone has the words; `skipped` = no time on it at all. */
export type AfterRow = { id: string; name: string; ms: number; spans: number; skipped: boolean; words: string | null; changed: boolean };

/** The After screen, in the set's order: each bit's time on tape, how many times he came to it, what he said there
 *  (once transcribed), and whether he said it differently from what is written (`texts`, when the page has them). */
export function afterRows(bits: readonly RunBit[], splits: readonly Split[], words: Record<string, string> | null | undefined, texts?: Record<string, string>): AfterRow[] {
  return bits.map((b) => {
    const mine = splits.filter((s) => s.joke_id === b.id);
    const ms = mine.reduce((a, s) => a + Math.max(0, s.end_ms - s.start_ms), 0);
    const said = words ? words[b.id] || '' : null;
    return {
      id: b.id, name: b.name, ms, spans: mine.length, skipped: !mine.length, words: said,
      changed: !!said && texts?.[b.id] != null && saidDifferently(said, texts[b.id]),
    };
  });
}

/** The Next control's word and whether it acts: on the last bit it greys as "Last bit" and never turns into Finish
 *  under the same thumb (§4.4: a double tap there would end the take). */
export function nextControl(i: number, n: number): { label: string; disabled: boolean } {
  return n > 0 && i < n - 1 ? { label: 'Next bit', disabled: false } : { label: 'Last bit', disabled: true };
}
export function backControl(i: number): { label: string; disabled: boolean } {
  return { label: 'Back', disabled: i <= 0 };
}

/** A filmed set's longest take (§4.4: video ends at a cap; Film it's one bit keeps ten minutes). Twenty minutes covers
 *  a feature spot; heat and file size at that length are a device check. */
export const SET_TAKE_S = 20 * 60;

/** A filmed set's floor of free space: a 20-minute 1080p HEVC take is about 1.2 to 1.5 GB, and Keep puts a second copy
 *  in Photos until the words exist (iOS dev review). Under it the disc greys "no room". */
export const SET_MIN_FREE_BYTES = 2 * 1024 * 1024 * 1024;
/** The free space as the one-bit take's check reads it (rules/filmRules discReason, floor 500 MB), shifted so that a
 *  set's floor applies; null (unknown) stays null. */
export function setRoom(free: number | null): number | null {
  if (free == null || !Number.isFinite(free)) return free;
  return free - (SET_MIN_FREE_BYTES - LOW_STORAGE_BYTES);
}
/** The take's file-size cap: the free space less a margin for Photos' copy and the phone, or none when unknown. */
export function setFileCap(free: number | null): number | null {
  if (free == null || !Number.isFinite(free)) return null;
  return Math.max(200 * 1024 * 1024, Math.floor((free - 700 * 1024 * 1024) / 2));
}
