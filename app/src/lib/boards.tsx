// "To a set" from anywhere (UX_SPEC 7.1): tap adds the bit to the current set list (the last one of HIS he opened,
// still live) with a snackbar "Added to … · Change · Undo"; with no such list the picker opens and nothing is written
// (docs/design/SPEC-2026-10-29-set-lists-and-stage-notes.md §1: never a draft, never the newest by default).
// Long-press opens the picker. The picker lists his live lists only (rules/setListRules pickerLists).
import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { Pressable, TextInput, View } from 'react-native';
import { router } from 'expo-router';
import { ApiError } from './api';
import { BoardListItem, creative } from './creative';
import { C, F } from './theme';
import { mmss } from '@/rules/format';
import { addTarget, pickerLists } from '@/rules/setListRules';
import { Btn, Sheet, T, s, useUI } from './ui';

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
  const [picker, setPicker] = useState<{ jokeId: string; title?: string; moveFrom?: string } | null>(null);
  const [list, setList] = useState<BoardListItem[] | null>(null);
  const [newTitle, setNewTitle] = useState('');
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
    }, { label: 'Change', testID: 'snack-change', run: () => setPicker({ jokeId, title, moveFrom: board.id }) });
    return true;
  }, [setCurrent, ui]);

  const add = useCallback(async (jokeId: string, title?: string) => {
    // the current list must still be his and live (he may have opened a draft, or archived it on another device);
    // anything else opens the picker: no list is ever chosen for him
    const bs = curRef.current ? (await creative.boards().catch(() => ({ boards: [] as BoardListItem[] }))).boards : [];
    const target = addTarget(curRef.current, bs);
    if (!target) {
      setPicker({ jokeId, title });
      return false;
    }
    return addTo({ id: target.id, title: target.title }, jokeId, title);
  }, [addTo]);

  const pick = useCallback((jokeId: string, title?: string) => setPicker({ jokeId, title }), []);

  useEffect(() => {
    if (picker) {
      setList(null);
      setNewTitle('');
      creative.boards().then((r) => setList(pickerLists(r.boards))).catch(() => setList([]));
    }
  }, [picker]);

  const choose = async (b: { id: string; title: string }) => {
    const p = picker!;
    setPicker(null);
    if (p.moveFrom && p.moveFrom !== b.id) await creative.patchBoard(p.moveFrom, { remove: { joke_id: p.jokeId } }).catch(() => {});
    await addTo(b, p.jokeId, p.title);
  };
  const createAndAdd = async () => {
    const p = picker!;
    const t = newTitle.trim() || `${p.title || 'New'} set list`;
    setPicker(null);
    if (p.moveFrom) await creative.patchBoard(p.moveFrom, { remove: { joke_id: p.jokeId } }).catch(() => {});
    try {
      const v = await creative.createBoard({ title: t, joke_ids: [p.jokeId] });
      setCurrent({ id: v.board.id, title: v.board.title });
      ui.snack(`Started ${v.board.title}`, undefined, { label: 'Open', run: () => router.push(`/list/${encodeURIComponent(v.board.id)}` as any) });
    } catch (e: any) {
      ui.snack(`Couldn't create the set list: ${e.message}`);
    }
  };

  const value = useMemo(() => ({ current, setCurrent, add, pick }), [current, setCurrent, add, pick]);
  return (
    <Ctx.Provider value={value}>
      {children}
      <Sheet visible={!!picker} onClose={() => setPicker(null)} label="Add to which set list?" testID="board-picker">
        <T v="h2" style={{ marginBottom: 4 }}>{picker?.moveFrom ? 'Move to which set list?' : 'Add to which set list?'}</T>
        {!!picker?.title && <T v="mut" style={{ marginBottom: 12 }}>{picker.title}</T>}
        <View style={{ gap: 8 }}>
          {list == null && <T v="mut">Loading your set lists…</T>}
          {list?.length === 0 && <T v="mut">No set list of yours yet. Start one below.</T>}
          {list?.slice(0, 8).map((b) => (
            <Pressable key={b.id} testID={`pick-board-${b.id}`} accessibilityRole="button" onPress={() => choose({ id: b.id, title: b.title })}
              style={[s.inner, { minHeight: 52, flexDirection: 'row', alignItems: 'center', gap: 10 }]}>
              <View style={{ flex: 1 }}>
                <T v="semi" numberOfLines={1}>{b.working_set ? '★ ' : ''}{b.title}</T>
                <T v="small">{b.n} bits · {b.runtime}{current?.id === b.id ? ' · current' : ''}</T>
              </View>
            </Pressable>
          ))}
          <View style={[s.inner, { gap: 8 }]}>
            <T v="semi">New set list</T>
            <TextInput testID="new-board-title" value={newTitle} onChangeText={setNewTitle} placeholder="Name it (e.g. Friday · 7)" placeholderTextColor={C.dim}
              style={{ minHeight: 44, borderWidth: 1, borderColor: C.border, borderRadius: 8, paddingHorizontal: 12, color: C.text, fontFamily: F.body, fontSize: 16 }} />
            <Btn testID="new-board-create" label="Create and add" v="add" small onPress={createAndAdd} />
          </View>
        </View>
        <Btn label="Cancel" v="ghost" onPress={() => setPicker(null)} style={{ marginTop: 8 }} />
      </Sheet>
    </Ctx.Provider>
  );
}
