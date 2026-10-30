// Client for the Simple Punchable routes (docs/API_CONTRACT.md "Simple Punchable"): Write, the joke map, the set
// builder. The merge spot is lib/MergeHint.tsx.
import { get, post } from './api';
import { Job, creative } from './creative';
import { retry } from '@/rules/retry';

/** A bit as the app shows it anywhere: short title, his line (verbatim), its date, one stage line in words. */
export type BitCard = {
  joke_id: string; title: string; short_name?: string; stage?: string | null; quote: string | null; quote_node?: string | null; cut_start?: boolean; cut_end?: boolean;
  date: string | null; stage_line: string | null; proven: boolean; nights: number; duration_s?: number; estimated?: boolean;
  why?: string; mine?: boolean; proven_pair?: boolean; unit_id?: string; rank?: number;
};
export type Related = { query: string; items: BitCard[]; total: number };
export type MapGroupKey = 'next' | 'tags' | 'same_world' | 'callbacks' | 'versions';
export type JokeMap = { joke: BitCard & { text?: string }; groups: { key: MapGroupKey; title: string; items: BitCard[] }[] };
export type Step = BitCard & { position: number; transition_in: null | { words: string; weak: boolean; kind: 'proven' | 'link' | 'jump'; from: string; to: string } };
export type Build = {
  board: { id: string; title: string; target_s: number; target_lo_s: number }; runtime_s: number; runtime: string; estimated_n: number;
  status: 'short' | 'on' | 'over' | 'none'; status_words: string; weak_n: number; steps: Step[]; next: BitCard[];
  notes: { rule: string; text: string; suggest?: string | null }[]; working_set: string | null;
};
export type Fact = { text: string; source_title: string; source_url: string };
export type Facts = { idea?: string; status: 'ok' | 'none' | 'provider_error' | 'queued' | 'empty'; facts: Fact[]; dropped?: number; label?: string; job?: Job };
export type RiffIdea = { text: string; built_on: string[]; draft: true; label: string };
export type Riff = { idea?: string; status: 'ok' | 'none' | 'no_grounding' | 'provider_error' | 'queued'; ideas: RiffIdea[]; why?: string; job?: Job };

const enc = encodeURIComponent;
export const simple = {
  related: (q: string, cap = 10) => get<Related>(`/api/notebook/related?q=${enc(q)}&cap=${cap}`),
  jokeMap: (id: string, cap = 8) => get<JokeMap>(`/api/jokes/${enc(id)}/map?cap=${cap}`),
  build: (boardId: string) => get<Build>(`/api/boards/${enc(boardId)}/build`),
  facts: (text: string, fresh = false) => post<any>('/api/facts', fresh ? { text, fresh } : { text }),
  riff: (idea: string, lines: string[], fresh = false) => post<any>('/api/riff', { idea, lines, ...(fresh ? { fresh } : {}) }),
  /** A riff line → an idea in his notebook. `attach` (the merge module): the line is clearly one existing bit and is
   *  filed as a dated note of `into` instead; Undo = POST /api/merge/undo {merge_id: attach.id}. `draft` = one of the
   *  AI's riff drafts taken as his ("Use as mine"), edited or not: the server's learned section counts it as kept. */
  saveIdea: (text: string, idea: string, draft = false) => post<{ ok: boolean; id: string; indexes: number[]; attach?: { decision: 'attach' | 'ambiguous' | 'new'; id?: string; into?: string; title?: string } }>(
    '/api/ideas', { text, reason: `riff on: ${idea.slice(0, 200)}`, ...(draft ? { draft: true } : {}) }),
  /** He dismissed one of the AI's drafts (Write's riff, Brainstorm's punch-up): the voice profile's learned section
   *  (api-dev/profile_api.py, POST /api/profile/voice/learn) remembers it as passed over. Fire and forget: three
   *  tries in the background, never a snack, never a block; a lost dismiss costs nothing but the signal. */
  dismissDraft: (text: string, bit: string | null = null): void => {
    void retry(() => post('/api/profile/voice/learn', { dropped: [{ text, ...(bit ? { bit } : {}) }] }), { tries: 3, delayMs: 500 }).catch(() => {});
  },
};

/**
 * A slow draft (facts, riff, punch-up, ask): 200 = the cached result; 202 = queued for the writer on the server. Polls the
 * job a few times; still queued after that = {status: 'queued'} (the screen says so and keeps working).
 */
export async function draft<T extends { status?: string }>(call: () => Promise<any>, onQueued?: () => void, tries = 8, every = 2500): Promise<T> {
  const r: any = await call();
  if (!(r?.__status === 202 || r?.status === 'queued') || !r?.job?.id) return r as T;
  onQueued?.(); // the screen says "queued" now, and keeps polling quietly
  const jid = r.job.id;
  for (let k = 0; k < tries; k++) {
    await new Promise((res) => setTimeout(res, every));
    const j = await creative.job<T>(jid).catch(() => null);
    if (j?.status === 'done' && j.result) return j.result as T;
    if (j?.status === 'failed') throw new Error(j.error || 'the AI provider didn’t answer');
  }
  return { status: 'queued', job: r.job } as unknown as T;
}

/** The verbatim line with the "…" the server says it cut. */
export const quoteText = (b: Pick<BitCard, 'quote' | 'cut_start' | 'cut_end'>) => (b.quote ? `${b.cut_start ? '…' : ''}${b.quote}${b.cut_end ? '…' : ''}` : '');
