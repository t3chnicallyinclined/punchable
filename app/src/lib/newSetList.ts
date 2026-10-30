// "+" on Sets, and "New set list" in Tonight's ⋯ menu (docs/design/SPEC-2026-10-30-create-flow.md §2, §3): the list
// exists before it is named, the way Reminders' "New List" does. It is titled for today (rules/setListRules
// defaultListTitle: "Friday · 30 Oct", then "(2)"), 7 minutes long, empty; it becomes the current list and opens at
// once (one tap from "+" to an open list; its first bit is one more). The haptic plays once the server has it (§4).
// Offline nothing is written and nothing navigates. The snack offers Rename: the list screen opens its own sheet when
// it is navigated to with a fresh `rename` param (the `/notes?talk=<now>` pattern in lib/ui).
import { router } from 'expo-router';
import { ApiError } from './api';
import { creative } from './creative';
import { haptics } from './haptics';
import { DEFAULT_LIST_TARGET_S, defaultListTitle } from '@/rules/setListRules';
import type { SnackAction } from './ui';

type Snack = (msg: string, undo?: () => Promise<void> | void, action?: SnackAction) => void;
export type NewList = { id: string; title: string };

function failWords(e: any): string {
  if (e instanceof ApiError || !(e instanceof TypeError)) return `Couldn’t start a set list: ${e?.message || 'no answer'}`;
  return 'Can’t start a set list offline. Nothing changed.';
}

/**
 * Creates today's list and opens it. `titles` are the titles already in use (the Sets tab has them; a caller without
 * them lets this fetch the boards once). Resolves to the new list, or null when nothing was written (the snack said why).
 */
export async function newSetList(o: { titles?: readonly string[]; setCurrent: (c: NewList) => void; snack: Snack }): Promise<NewList | null> {
  try {
    const titles = o.titles ?? (await creative.boards().catch(() => ({ boards: [] as { title: string }[] }))).boards.map((b) => b.title);
    const v = await creative.createBoard({ title: defaultListTitle(new Date(), titles), joke_ids: [], target_s: DEFAULT_LIST_TARGET_S });
    const cur: NewList = { id: v.board.id, title: v.board.title };
    o.setCurrent(cur);
    haptics.success();
    router.push(`/list/${encodeURIComponent(cur.id)}` as any);
    o.snack(`Started ${cur.title}`, undefined, {
      label: 'Rename', testID: 'snack-rename',
      run: () => router.navigate({ pathname: '/list/[id]', params: { id: cur.id, rename: String(Date.now()) } } as any),
    });
    return cur;
  } catch (e: any) {
    o.snack(failWords(e));
    return null;
  }
}
