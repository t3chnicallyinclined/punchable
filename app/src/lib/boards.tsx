// "To a set" from anywhere (UX_SPEC 7.1): tap adds the bit to the current set list (the last one of HIS he opened,
// still live) with a snackbar "Added to … · Change · Undo"; with no such list the picker opens and nothing is written
// (docs/design/SPEC-2026-10-29-set-lists-and-stage-notes.md §1: never a draft, never the newest by default).
// Long-press opens the picker. The picker is lib/SetPicker (SPEC 2026-11-16 §4.5): his live lists, multi-tick, staged
// until Done; `pick` opens it as its own sheet route (/setpick).
import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { router } from 'expo-router';
import { ApiError } from './api';
import { BoardListItem, creative } from './creative';
import { openSetPicker } from './SetPicker';
import { mmss } from '@/rules/format';
import { addTarget } from '@/rules/setListRules';
import { useUI } from './ui';

const KEY = 'pk.board.current';
type Cur = { id: string; title: string } | null;
function readCur(): Cur {
  try {
    const v = (globalThis as any).localStorage?.getItem(KEY);
    return v ? JSON.parse(v) : null;
  } catch {
    return null;
  }
}
function writeCur(c: Cur) {
  try {
    (globalThis as any).localStorage?.setItem(KEY, JSON.stringify(c));
  } catch {}
}

type BoardCtx = {
  current: Cur;
  setCurrent: (c: Cur) => void;
  /** Resolves true when the bit was written to a board; false when the picker opened instead or the write failed. */
  add: (jokeId: string, title?: string) => Promise<boolean>;
  pick: (jokeId: string, title?: string) => void;
};
const Ctx = createContext<BoardCtx | null>(null);
export const useBoards = () => {
  const c = useContext(Ctx);
  if (!c) throw new Error('useBoards outside BoardProvider');
  return c;
};

export function BoardProvider({ children }: { children: React.ReactNode }) {
  const ui = useUI();
  const [current, setCur] = useState<Cur>(() => readCur());
  const curRef = useRef(current);
  curRef.current = current;

  const setCurrent = useCallback((c: Cur) => {
    setCur(c);
    writeCur(c);
  }, []);

  // the Working set pinned in the sidebar
  useEffect(() => {
    let alive = true;
    creative.workingsets().then((r) => {
      const w = r.working_sets[0];
      if (alive && w) ui.setWorkingSet({ id: w.id, title: w.title, sub: `v${w.version} · ${w.n} bits · ${mmss(w.target_s)}`, target_s: w.target_s });
    }).catch(() => {});
    return () => {
      alive = false;
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const addTo = useCallback(async (board: { id: string; title: string }, jokeId: string, title?: string, quiet?: boolean): Promise<boolean> => {
    try {
      await creative.addItem(board.id, jokeId);
    } catch (e: any) {
      if (e instanceof ApiError && e.status === 409) {
        ui.snack(`${title || 'It'} is already on ${board.title}`, undefined, { label: 'Open', run: () => router.push(`/list/${encodeURIComponent(board.id)}` as any) });
        return false;
      }
      ui.snack(`Couldn't add: ${e.message}`);
      return false;
    }
    setCurrent(board);
    if (quiet) return true;
    ui.snack(`Added to ${board.title}`, async () => {
      await creative.patchBoard(board.id, { remove: { joke_id: jokeId } });
    }, { label: 'Change', testID: 'snack-change', run: () => openSetPicker(jokeId, title) });
    return true;
  }, [setCurrent, ui]);

  const add = useCallback(async (jokeId: string, title?: string) => {
    // the current list must still be his and live (he may have opened a draft, or archived it on another device);
    // anything else opens the picker: no list is ever chosen for him
    const bs = curRef.current ? (await creative.boards().catch(() => ({ boards: [] as BoardListItem[] }))).boards : [];
    const target = addTarget(curRef.current, bs);
    if (!target) {
      openSetPicker(jokeId, title);
      return false;
    }
    return addTo({ id: target.id, title: target.title }, jokeId, title);
  }, [addTo]);

  const pick = useCallback((jokeId: string, title?: string) => openSetPicker(jokeId, title), []);

  const value = useMemo(() => ({ current, setCurrent, add, pick }), [current, setCurrent, add, pick]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
