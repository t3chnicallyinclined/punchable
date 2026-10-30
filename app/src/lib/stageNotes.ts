// What the notes sheet (/bitnotes) shows, handed over in memory: the stage and the set list already hold the build
// (GET /api/boards/<id>/build carries each bit's words), so the sheet never fetches. The stage sets it on load and on
// every current-bit change with `onNext` = its own Next (the sheet's footer advances the stage); the set list sets it
// with the tapped row and no `onNext` (the footer then opens the bit). Plain TS, one module-level value, a subscribe
// for React (useSyncExternalStore). Docs: docs/design/SPEC-2026-10-29-set-lists-and-stage-notes.md §2.1.
import { useSyncExternalStore } from 'react';
import type { Step } from './simple';

export type StageNotes = {
  boardId: string; title: string; steps: Step[]; index: number;
  /** The stage's Next: advances the stage (its haptic, its clock) and, through `set`, this store. Absent off stage. */
  onNext?: (() => void) | null;
};

let value: StageNotes | null = null;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export const stageNotes = {
  get: (): StageNotes | null => value,
  set(v: StageNotes | null) { value = v; emit(); },
  /** A partial change (the current bit moved, the steps re-read) over what is there; nothing when nothing is set. */
  patch(p: Partial<StageNotes>) { if (value) { value = { ...value, ...p }; emit(); } },
  /** Forget it when its owner (the stage, the set list) goes: a later sheet shows the empty state, never a stale set. */
  clear(owner: string) { if (value && value.boardId === owner) { value = null; emit(); } },
  subscribe(l: () => void) { listeners.add(l); return () => { listeners.delete(l); }; },
};

export function useStageNotes(): StageNotes | null {
  return useSyncExternalStore(stageNotes.subscribe, stageNotes.get, stageNotes.get);
}
