// Say it or ask it (docs/UX_2026-10.md): the one input on every screen. POST /api/say decides whether the text is a
// question (-> a conversation with his material, ask_api) or a bit (-> saved as an idea, attached to the bit it
// clearly belongs to). The classification is the app's (by ai:intent on the server); his "Not a question? Save as
// a bit" tap is sent as `kind` and recorded as his own override.
//
// Offline rule (the review): a lost punchline costs more than a misfiled question. When the API can't be reached the
// text is kept in a per-device outbox (localStorage, a convenience, not state) as a bit, shown "saving…", and sent
// as soon as the API answers again.
import { useEffect, useState } from 'react';
import { ApiError, get, post, useOnline } from './api';
import type { Chat, Worker } from './ask';

import type { SayKind } from './sayRules';
export type { SayKind };
export type SayResult = {
  kind: SayKind; intent: string; guess: SayKind; rule: string; overridden: boolean; text: string; worker: Worker;
  chat?: Chat;
  idea?: { ok: boolean; id: string; indexes: number[]; attach?: { decision: 'attach' | 'ambiguous' | 'new'; id?: string; into?: string; title?: string } };
  saved_to?: { joke_id: string; title: string | null } | null;
  undo?: { POST: string; indexes: number[]; detach?: { POST: string; merge_id: string } };
};
export type SayContext = { set_id?: string; joke_id?: string; board_id?: string; screen?: string };

export type TonightScreen = {
  date: string; weekday: string;
  board: null | {
    id: string; title: string; target_s: number; runtime_s: number; runtime: string; status: 'short' | 'on' | 'over' | 'none'; status_words: string;
    weak_n: number; source?: string; segments: { joke_id: string; short_name: string; duration_s: number; gap: boolean }[];
    first_gap: null | { from: string; to: string; from_name: string; to_name: string };
  };
  to_sort: number; latest_set: null | { id: string; title: string | null; date: string | null; venue: string | null; reviewed: boolean | null };
  questions: string[]; worker: Worker; counts: { bits: number; sets: number | null };
};

export const say = {
  send: (text: string, opts?: { kind?: SayKind; context?: SayContext }) =>
    post<SayResult>('/api/say', { text, ...(opts?.kind ? { kind: opts.kind } : {}), ...(opts?.context ? { context: opts.context } : {}) }),
  tonight: () => get<TonightScreen>('/api/tonight/screen'),
};

export { guessKind, isQuestion, positionIn, shortNameOf } from './sayRules';

// ---------------------------------------------------------------- the outbox (per-device; flushed when the API is back)
const OUTBOX = 'pk.say.outbox.v1';
export type Queued = { key: string; text: string; at: number; context?: SayContext };
function readOutbox(): Queued[] {
  try {
    const raw = (globalThis as any).localStorage?.getItem(OUTBOX);
    return raw ? (JSON.parse(raw) as Queued[]) : [];
  } catch {
    return [];
  }
}
function writeOutbox(q: Queued[]) {
  try {
    (globalThis as any).localStorage?.setItem(OUTBOX, JSON.stringify(q));
  } catch {}
}
const outboxListeners = new Set<() => void>();
const notify = () => outboxListeners.forEach((f) => f());
export const outbox = {
  list: readOutbox,
  push(text: string, context?: SayContext): Queued {
    const it = { key: `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`, text, at: Date.now(), context };
    writeOutbox([...readOutbox(), it]);
    notify();
    return it;
  },
  drop(key: string) {
    writeOutbox(readOutbox().filter((x) => x.key !== key));
    notify();
  },
};
let flushing = false;
/** Send everything waiting, oldest first, as bits (kind: 'bit': offline it was never classified; he can re-ask). */
export async function flushOutbox(onSent?: (q: Queued, r: SayResult) => void) {
  if (flushing) return;
  flushing = true;
  try {
    for (const q of readOutbox()) {
      try {
        const r = await say.send(q.text, { kind: 'bit', context: q.context });
        outbox.drop(q.key);
        onSent?.(q, r);
      } catch (e) {
        if (e instanceof ApiError) outbox.drop(q.key); // the API refused it (bad text): it will never send; drop it
        else break; // still offline: keep the rest for the next time
      }
    }
  } finally {
    flushing = false;
  }
}
export function useOutbox() {
  const [items, setItems] = useState<Queued[]>(() => readOutbox());
  const online = useOnline();
  useEffect(() => {
    const f = () => setItems(readOutbox());
    outboxListeners.add(f);
    return () => {
      outboxListeners.delete(f);
    };
  }, []);
  useEffect(() => {
    if (online && readOutbox().length) flushOutbox();
  }, [online]);
  return items;
}

/** Say it: the API's answer, or (no network) a queued bit. An ApiError (4xx/5xx) is thrown: that is a real refusal. */
export async function sayOrQueue(text: string, opts?: { kind?: SayKind; context?: SayContext }): Promise<{ queued: Queued } | { result: SayResult }> {
  try {
    return { result: await say.send(text, opts) };
  } catch (e) {
    if (e instanceof ApiError) throw e;
    return { queued: outbox.push(text, opts?.context) };
  }
}
