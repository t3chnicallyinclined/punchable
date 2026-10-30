// Client for "Your sets" (docs/API_CONTRACT.md "Your sets"; engine creative/mysets.py): many sets at once, each opened
// and worked on. A set is a working set on the server, so the old working-set screens keep working for the same id.
import { get, patch, post } from './api';
import { BitCard } from './simple';
import { C } from './theme';

export type Label = 'killed' | 'solid' | 'mixed' | 'quiet' | 'untested';
export type Action =
  | { kind: 'swap'; joke_id: string; with: BitCard & { label: Label; label_words: string }; words: string }
  | { kind: 'tag'; joke_id: string; tag: BitCard; words: string }
  | { kind: 'bridge'; from: string; to: string; words: string }
  | { kind: 'move'; joke_id: string; to: number; words: string }
  | { kind: 'cut'; joke_id: string; words: string }
  | { kind: 'add'; joke_id: string; card: BitCard & { label: Label; label_words: string }; words: string };
export type Spot = { kind: 'quiet' | 'dropping' | 'stretch' | 'transition' | 'long' | 'over' | 'short' | 'closer'; joke_id: string | null; words: string; actions: Action[]; from?: string; to?: string };
export type SetBit = BitCard & {
  position: number; label: Label; label_words: string; last_words: string | null; locked: boolean;
  transition_in: null | { words: string; weak: boolean; kind: 'proven' | 'link' | 'jump'; from: string; to: string }; tough: Spot[];
};
export type SetVersion = { v: number; created_at: string; note: string; n: number; target_s: number; by: 'you' | 'Punchable'; titles: string[] };
export type SetView = {
  set: { id: string; title: string; target_s: number; target_lo_s: number; length_words: string; version: number; storyboard_id: string | null; created_at: string; updated_at: string; made_by: 'you' | 'Punchable' };
  runtime_s: number; runtime: string; estimated_n: number; status: 'short' | 'on' | 'over' | 'none'; status_words: string;
  health: string; tough_n: number; bits: SetBit[]; tough: Spot[]; next: (BitCard & { label: Label; label_words: string })[];
  tapes: { n: number; words: string; last: null | { set: string; date: string; venue_label: string; bits_done: number; bits_planned: number; same_order: boolean } };
  versions: SetVersion[];
};
export type SetSummary = {
  id: string; title: string; target_s: number; length_words: string; n: number; runtime_s: number; runtime: string; estimated_n: number;
  status: SetView['status']; status_words: string; health: string; tough_n: number; version: number; storyboard_id: string | null; updated: string; made_by: 'you' | 'Punchable';
};
export type Trim = { from: string; title: string; target_s: number; runtime_s: number; runtime: string; status: SetView['status']; status_words: string; keep: (BitCard & { label: Label; label_words: string })[]; cut: (BitCard & { label: Label; label_words: string })[]; why: string };

const enc = encodeURIComponent;
export const mysets = {
  list: () => get<{ sets: SetSummary[]; lengths: number[] }>('/api/mysets'),
  one: (id: string) => get<SetView>(`/api/mysets/${enc(id)}`),
  create: (b: { title: string; target_s: number; joke_ids?: string[] }) => post<SetView>('/api/mysets', b),
  fromBoard: (boardId: string, target_s?: number, title?: string) => post<SetView>('/api/mysets', { board_id: boardId, ...(target_s ? { target_s } : {}), ...(title ? { title } : {}) }),
  trim: (id: string, target_s: number) => get<Trim>(`/api/mysets/${enc(id)}/trim?target_s=${target_s}`),
  fromSet: (id: string, target_s: number, joke_ids?: string[], title?: string) => post<SetView>('/api/mysets', { from_set: id, target_s, ...(joke_ids ? { joke_ids } : {}), ...(title ? { title } : {}) }),
  edit: (id: string, b: { joke_ids?: string[]; target_s?: number; title?: string; note?: string; delete?: boolean }) => patch<SetView>(`/api/mysets/${enc(id)}`, b),
  lock: (id: string, joke_id: string, unlock = false) => post<SetView>(`/api/mysets/${enc(id)}/lock`, { joke_id, ...(unlock ? { unlock } : {}) }),
  bridge: (id: string, from: string, to: string, fresh = false) => post<any>(`/api/mysets/${enc(id)}/bridge`, { from, to, ...(fresh ? { fresh } : {}) }),
};

/** "10 min", "7.5 min" */
export const lengthWords = (s: number) => (s % 60 ? `${(s / 60).toFixed(1)} min` : `${s / 60} min`);
export const LABEL_WORD: Record<Label, string> = { killed: 'Killed', solid: 'Solid', mixed: 'Mixed', quiet: 'Quiet', untested: 'Untested' };
export const labelColor = (l: Label) => (l === 'killed' ? C.green : l === 'solid' ? C.text2 : l === 'mixed' ? C.amber : l === 'quiet' ? C.redSoft : C.mut);
