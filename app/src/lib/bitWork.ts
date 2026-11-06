// The bit workbench's writes (2026-10-29; owner: "open a bit, edit and speak to my material, brainstorm, then
// organize the joke live"). Every call here is undoable: a version is two decisions (their indexes), a merge or an
// attach has a merge_id for POST /api/merge/undo. The pure rules (pickMain, answerLine) live in rules/bitWorkRules.ts, tested in tests/bitWork.test.mjs.
import { post } from './api';

export type Version = { key: string; label: string; date: string | null; text: string; doc?: string | null };
export type Added = { ok: boolean; unit: string; joke: string; indexes: number[]; learned?: unknown };
export type Merged = { ok: boolean; merge_id: string; indexes?: number[] };

const enc = encodeURIComponent;

/** A new version of the bit (the old ones stay). `draft` = the words came from an AI answer or draft he kept.
 *  `source` = the version's cut and origin (rules/bitLines `sourceFor`: `rant`, `rant_session`, `hook`, `rant_hook`);
 *  it wins over the draft's own `draft` source, so a kept rant is both learned from and a rant cut. */
export const addVersion = (jokeId: string, text: string, reason?: string, draft = false, source?: string) =>
  post<Added>(`/api/jokes/${enc(jokeId)}/versions`, {
    text, ...(reason ? { reason } : {}), ...(draft ? { draft: true, source: 'draft' } : {}), ...(source ? { source } : {}),
  });

/** Film it's two sessions on one version (POST /api/jokes/<id>/rant | /hook): 202 with the job to poll, or 200 with a
 *  cached result. `exclude` = the openers he already saw ("Show others"). */
export const askSession = (jokeId: string, mode: 'rant' | 'hook', unit: string, exclude?: string[]) =>
  post<any>(`/api/jokes/${enc(jokeId)}/${mode}`, { unit, ...(exclude?.length ? { exclude } : {}) });

/** Fold other bits into this one: their versions and tapes move under it; a typed idea's words become a version. */
export const mergeInto = (main: string, others: string[], reason?: string) =>
  post<Merged>('/api/merge', { main, others, ...(reason ? { reason } : {}) });

/** His call that a note belongs under a bit (no judge). */
export const attachNote = (note: string, into: string, reason?: string) =>
  post<Merged>('/api/merge/attach', { note, into, ...(reason ? { reason } : {}) });

export const undoMerge = (merge_id: string) => post<{ ok: boolean }>('/api/merge/undo', { merge_id });

export { answerLine, pickMain } from '@/rules/bitWorkRules';
