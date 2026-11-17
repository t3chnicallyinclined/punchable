// The marks he has made and not sent yet (SPEC 2026-11-16 §3.2 and Slice 3 as built): one tray for his bit and one for
// the draft he is reading (round two), kept on the phone like the bit's own draft (Documents/marks/<bit>.json; the
// web: localStorage), so they survive leaving the page, closing the app and going offline. The page, the tray sheet
// and the draft screen share one copy in memory and hear each other's changes. Rules: rules/markRules.ts.
import { useCallback, useEffect, useState } from 'react';
import { dropDraftRaw, readDraftRaw, writeDraftRaw } from './draftFiles';
import * as M from '@/rules/markRules';

const SPACE = 'marks';
type Key = string;
const mem = new Map<Key, M.MarkSet>();
const loaded = new Map<Key, Promise<M.MarkSet>>();
const listeners = new Map<Key, Set<() => void>>();

const empty = (bit: string, on: string | null): M.MarkSet => ({ v: 1, bit, on, marks: [], at: '' });

/** The tray for a bit (`on` null) or for the draft of round `on`, read from the phone once. A tray kept for an older
 *  draft (another round since) starts empty. */
export function loadMarks(bit: string, on: string | null): Promise<M.MarkSet> {
  const k = M.markSetName(bit, on);
  let p = loaded.get(k);
  if (!p) {
    p = readDraftRaw(SPACE, k).then((raw) => {
      const s = M.parseMarkSet(raw);
      const ok = s && s.bit === bit && (s.on || null) === (on || null) ? s : empty(bit, on);
      if (!mem.has(k)) mem.set(k, ok);
      return mem.get(k) as M.MarkSet;
    });
    loaded.set(k, p);
  }
  return p;
}

function emit(k: Key) {
  listeners.get(k)?.forEach((l) => l());
}

/** One write after another per tray: the last change is the one that stays on the phone (iOS dev review). */
const writes = new Map<Key, Promise<void>>();

/** Replace a tray's marks (and keep them on the phone). */
export async function setMarks(bit: string, on: string | null, marks: M.Mark[]): Promise<void> {
  const k = M.markSetName(bit, on);
  await loadMarks(bit, on);
  const s: M.MarkSet = { v: 1, bit, on, marks, at: new Date().toISOString() };
  mem.set(k, s);
  emit(k);
  const next = (writes.get(k) || Promise.resolve()).then(async () => {
    if (marks.length) await writeDraftRaw(SPACE, k, JSON.stringify(s));
    else await dropDraftRaw(SPACE, k);
  });
  writes.set(k, next.catch(() => {}));
  await next;
}

/** The tray as it is now in memory (a Send right after a note was typed reads this, never a render's older copy). */
export function peekMarks(bit: string, on: string | null): M.Mark[] {
  const s = mem.get(M.markSetName(bit, on));
  return s && s.bit === bit && (s.on || null) === (on || null) ? s.marks : [];
}

let n = 0;
/** A fresh id for a mark made on this phone. */
export const markId = () => `m${Date.now().toString(36)}${(n++).toString(36)}`;

/** One tray, live: its marks and the changes to it. `on` = the round whose draft is marked, null for his bit. */
export function useMarks(bit: string, on: string | null) {
  const k = M.markSetName(bit, on);
  const [set, setSet] = useState<M.MarkSet>(() => mem.get(k) || empty(bit, on));
  useEffect(() => {
    let alive = true;
    const hear = () => { if (alive) setSet(mem.get(k) || empty(bit, on)); };
    const ls = listeners.get(k) || new Set();
    ls.add(hear);
    listeners.set(k, ls);
    void loadMarks(bit, on).then(hear);
    return () => { alive = false; ls.delete(hear); };
  }, [k, bit, on]);
  // a tray kept for another round's draft is not this one's
  const marks = set.bit === bit && (set.on || null) === (on || null) ? set.marks : [];
  const replace = useCallback((next: M.Mark[]) => setMarks(bit, on, next), [bit, on]);
  return { marks, replace };
}

/** The Send asked for from the tray sheet: the page that owns the text sends (it knows the version and can save first). */
type SendAsk = { bit: string; on: string | null };
const sendListeners = new Set<(a: SendAsk) => void>();
export const sendAsked = {
  emit(a: SendAsk) { sendListeners.forEach((l) => l(a)); },
  subscribe(l: (a: SendAsk) => void) { sendListeners.add(l); return () => { sendListeners.delete(l); }; },
};
