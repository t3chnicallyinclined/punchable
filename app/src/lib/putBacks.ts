// What he chose on a draft, change by change (docs/design/SPEC-2026-11-17-one-notebook.md screen 4): kept per bit on
// the phone (Documents/putbacks/<bit>.json; the web: localStorage), so the draft opens the way he left it, and a
// change he turned down stays turned down when the next draft of the same bit comes (Try again makes a new round; his
// own words do not move between rounds, so a change is known by where his words were and what they said:
// rules/draftRules hunkKey). `used` (a change outside his marks that he took after all) holds for one draft only.
// Emptied when he uses a draft or keeps his own words.
import { dropDraftRaw, readDraftRaw, writeDraftRaw } from './draftFiles';

const SPACE = 'putbacks';
export type PutBacks = { /** the round these were chosen on */ job: string | null; back: Set<string>; used: Set<string> };
const mem = new Map<string, PutBacks>();
const strings = (x: unknown): string[] => (Array.isArray(x) ? x.filter((k): k is string => typeof k === 'string') : []);

/** His choices on this bit's draft (read from the phone once). */
export async function loadPutBacks(bit: string): Promise<PutBacks> {
  const had = mem.get(bit);
  if (had) return had;
  let out: PutBacks = { job: null, back: new Set(), used: new Set() };
  try {
    const d = JSON.parse((await readDraftRaw(SPACE, bit)) || 'null');
    if (d && d.bit === bit) out = { job: typeof d.job === 'string' ? d.job : null, back: new Set(strings(d.back)), used: new Set(strings(d.used)) };
  } catch { /* nothing kept */ }
  const now = mem.get(bit) || out;
  mem.set(bit, now);
  return now;
}

/** Remember his choices as they are now (the whole lists, replacing the ones before). */
export async function savePutBacks(bit: string, job: string | null, back: readonly string[], used: readonly string[]): Promise<void> {
  mem.set(bit, { job, back: new Set(back), used: new Set(used) });
  if (back.length || used.length) await writeDraftRaw(SPACE, bit, JSON.stringify({ v: 1, bit, job, back, used }));
  else await dropDraftRaw(SPACE, bit);
}

export const clearPutBacks = (bit: string) => savePutBacks(bit, null, [], []);
