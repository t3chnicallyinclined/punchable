// The redline round's calls (SPEC 2026-11-16 §3.2 and Slice 3 as built; pk-api writes/redline.rs, Rust only): send his
// marks once, read the round open on a bit (polled while the worker writes the draft), close it (used, discarded, or
// Undo of a use), and the bits whose draft waits for him. Rules: rules/markRules.ts.
import { useEffect } from 'react';
import { get, post, useApi } from './api';
import type { Intent, MarkType, Round } from '@/rules/markRules';

const enc = encodeURIComponent;
/** How often the page asks again while the worker is writing the draft. */
const POLL_MS = 4000;

export const redlinePath = (bit: string) => `/api/jokes/${enc(bit)}/redline`;

export type RoundRead = { joke: string; newest: string | null; round: Round | null; last: { job: string; verdict: string; unit: string | null; at: string } | null };
export type Waiting = { joke: string; title: string; short_name: string; job: string; status: string; state: 'waiting' | 'ready' | 'refused' | 'failed'; stale: boolean; sent_at: string };

export const redline = {
  send: (bit: string, body: { unit?: string; on?: string; marks: { type: MarkType; text: string; at: number; note?: string; intent?: Intent }[] }) =>
    post<{ status: string; job: { id: string }; round: any }>(redlinePath(bit), body),
  read: (bit: string) => get<RoundRead>(redlinePath(bit)),
  resolve: (bit: string, job: string, verdict: 'used' | 'discarded' | 'open', unit?: string) =>
    post<{ ok: boolean; round: Round | null }>(`${redlinePath(bit)}/resolve`, { job, verdict, ...(unit ? { unit } : {}) }),
  waiting: () => get<{ open: Waiting[] }>('/api/redlines'),
};

/** The round on a bit, read again every few seconds while the draft is being written. */
export function useRound(bit: string | null) {
  const r = useApi<RoundRead>(bit ? redlinePath(bit) : null, { cache: false });
  const st = r.data?.round?.status;
  const writing = st === 'queued' || st === 'claimed';
  const reload = r.reload;
  useEffect(() => {
    if (!writing) return;
    const t = setInterval(() => { void reload(); }, POLL_MS);
    return () => clearInterval(t);
  }, [writing, reload]);
  return r;
}
