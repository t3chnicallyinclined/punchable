// "+" on Tonight and on Bits is this one function (docs/design/SPEC-2026-10-30-create-flow.md §2, §6.1): the bit
// exists at once under a marker title (rules/bitWorkRules freshBitTitle) and its page opens in the editor with the
// keyboard up; the first Save names it from its first line (joke/[id].tsx). The haptic plays once the server has the
// bit, never on the tap (§4). Offline nothing is written and nothing navigates: the snack says what to do instead.
import { useCallback } from 'react';
import { router } from 'expo-router';
import { ApiError, decide } from './api';
import { haptics } from './haptics';
import { freshBitTitle } from '@/rules/bitWorkRules';
import { useUI } from './ui';

type Snack = (msg: string) => void;

/** The words for a failed "+": the server's own when it answered; the offline line when nothing answered. */
function failWords(e: any): string {
  if (e instanceof ApiError || !(e instanceof TypeError)) return `Couldn’t start a bit: ${e?.message || 'no answer'}`;
  return 'Can’t start a bit offline. Ask it in the search field and it saves when you’re back.';
}

/** Creates the bit and opens it in the editor. Resolves to its id, or null when nothing was written (the snack said why). */
export async function newBit(snack: Snack, where: 'Tonight' | 'Bits'): Promise<string | null> {
  try {
    const w = await decide({ target: 'node', action: 'add', after: { type: 'joke', title: freshBitTitle(new Date()), stage: 'idea' }, reason: `new bit (${where} +)` });
    const id = w.decisions?.[0]?.id;
    if (!id) throw new Error('no id came back');
    haptics.success();
    router.push({ pathname: `/joke/${encodeURIComponent(id)}`, params: { edit: '1' } } as any);
    return id;
  } catch (e: any) {
    snack(failWords(e));
    return null;
  }
}

/** The screen's "+" handler: `newBit` bound to this app's snackbar. */
export function useNewBit(where: 'Tonight' | 'Bits'): () => void {
  const ui = useUI();
  return useCallback(() => { void newBit(ui.snack, where); }, [ui, where]);
}
