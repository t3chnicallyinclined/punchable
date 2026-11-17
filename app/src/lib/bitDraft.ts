// A bit's local draft (SPEC 2026-11-16 Slice 2; owner: "a bit is a document"): the words he typed that no version
// holds yet, kept on the phone (Documents/bit/<bit>.json; localStorage on the web) a moment after every keystroke, so
// nothing typed is lost offline or if the app dies. The page (lib/BitDoc) writes and drops it; `flushBitDrafts` sends
// what is still waiting when the app next has a connection and that bit's page is not open. Rules: rules/lineRules.
import { get } from './api';
import { addVersion } from './bitWork';
import { allDraftsRaw, dropDraftRaw, readDraftRaw, writeDraftRaw } from './draftFiles';
import { homeIndex, newestOf, sourceFor, versionsOf } from '@/rules/bitLines';
import { docText, draftName, flushPlan, parseBitDraft, type BitDraft } from '@/rules/lineRules';

const SPACE = 'bit';
const enc = encodeURIComponent;

export const readBitDraft = async (bit: string, cut: 'stage' | 'rant'): Promise<BitDraft | null> => parseBitDraft(await readDraftRaw(SPACE, draftName(bit, cut)));
/** Resolves false when the phone refused the write (the page then says the words are not kept). */
export const writeBitDraft = (d: BitDraft): Promise<boolean> => writeDraftRaw(SPACE, draftName(d.bit, d.cut), JSON.stringify(d));
export const dropBitDraft = (bit: string, cut: 'stage' | 'rant'): Promise<void> => dropDraftRaw(SPACE, draftName(bit, cut));

/** The bits whose page is open: that page saves its own draft. Counted, not a set: a bit reopened while the page he
 *  just left is still finishing its save must not lose its mark when that older page lets go. */
const open = new Map<string, number>();
export const bitDocOpen = (bit: string, cut: 'stage' | 'rant') => {
  const k = draftName(bit, cut);
  open.set(k, (open.get(k) || 0) + 1);
  let held = true;
  return () => {
    if (!held) return;
    held = false;
    const n = (open.get(k) || 1) - 1;
    if (n > 0) open.set(k, n); else open.delete(k);
  };
};

/** The bits with words on the page that no version holds yet (typed and not saved, or a save on its way): a card's
 *  sheet must not write a version from the older text meanwhile. Kept by the page's document (lib/BitDoc). */
const waiting = new Set<string>();
export const setBitPending = (bit: string, cut: 'stage' | 'rant', on: boolean) => { if (on) waiting.add(draftName(bit, cut)); else waiting.delete(draftName(bit, cut)); };
export const bitPending = (bit: string): boolean => waiting.has(draftName(bit, 'stage'));

let flushing = false;
/** Drafts left on this phone (offline, or the app died mid-sentence): each saved as a new version of its bit, over
 *  whatever the bit holds now, then dropped. Stops at the first failure (still offline: the rest wait). Returns how
 *  many versions it wrote. */
export async function flushBitDrafts(): Promise<number> {
  if (flushing) return 0;
  flushing = true;
  let wrote = 0;
  try {
    for (const raw of await allDraftsRaw(SPACE)) {
      const d = parseBitDraft(raw);
      if (!d || open.has(draftName(d.bit, d.cut))) continue;
      try {
        const page = await get<any>(`/api/jokes/${enc(d.bit)}`);
        const vs = versionsOf(page);
        const i = d.cut === 'rant' ? newestOf(vs, 'rant') : homeIndex(vs);
        // a draft that some version of that cut already says is a save whose answer was lost: dropped, never re-sent
        const plan = flushPlan(d, i >= 0 ? vs[i]?.text : null, vs.filter((x) => x.cut === d.cut).map((x) => x.text));
        if (plan === 'keep') continue;
        if (plan === 'save') {
          await addVersion(d.bit, docText(d.text), 'written on the bit page (sent when the connection came back)', false, sourceFor(d.cut, 'write'));
          wrote++;
        }
        await dropBitDraft(d.bit, d.cut);
      } catch (e: any) {
        // the bit is gone (retired elsewhere and deleted): its draft has nowhere to go and stays until he opens it
        if (e?.status === 404) continue;
        break;
      }
    }
  } finally {
    flushing = false;
  }
  return wrote;
}
