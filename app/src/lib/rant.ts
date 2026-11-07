// The Rant tab's calls (docs/design/SPEC-2026-11-06-rant-tab.md §12.7; pk-api writes/rant.rs and writes/topic.rs, Rust
// only): his topics with their counts, a topic as a page of his words, its search and Find more, the marks on its
// material (keep, drop, put back, a jot), the script's versions and where each line came from, Draft the rest and
// Claude on a line (jobs the worker runs, polled here). The pure rules are rules/rantRules.ts and rules/scriptRules.ts.
import { decide, get, patch, post } from './api';
import { addVersion } from './bitWork';
import { simple } from './simple';
import { jobView } from '@/rules/sessionRules';
import { rantTitle, type Mat, type State, type Topic, type TopicPage } from '@/rules/rantRules';
import { newAiLines, sourceOf, toSave, type DocLine, type Drafted, type SLine } from '@/rules/scriptRules';

const enc = encodeURIComponent;

export const rant = {
  topics: () => get<{ topics: Topic[] }>('/api/topics'),
  page: (id: string) => get<TopicPage>(`/api/topics/${enc(id)}`),
  /** The page's search (`q`) or Find more (no `q`, the next page then what sounds like what he kept). */
  material: (id: string, q?: string | null, offset = 0) =>
    get<{ rows: Mat[]; more: boolean }>(`/api/topics/${enc(id)}/material?${q ? `q=${enc(q)}&` : ''}offset=${offset}`),
  addTopic: (text: string) => post<{ ok: boolean; topic: Topic }>('/api/topics', { text }),
  rename: (id: string, text: string) => patch<{ ok: boolean; topic: Topic }>(`/api/topics/${enc(id)}`, { rename: text }),
  deleteTopic: (id: string) => patch<{ ok: boolean }>(`/api/topics/${enc(id)}`, { delete: true }),
  undelete: (id: string) => patch<{ ok: boolean; topic: Topic }>(`/api/topics/${enc(id)}`, { undelete: true }),
  /** keep / drop a bit or a moment on this topic; `null` = unmark (un-keep, Put back, every Undo). */
  mark: (id: string, ref: string, state: State | null) =>
    patch<{ ok: boolean; topic: Topic }>(`/api/topics/${enc(id)}`, { [state === 'kept' ? 'keep' : state === 'dropped' ? 'drop' : 'unmark']: ref }),
  jot: (id: string, ref: string) => patch<{ ok: boolean; topic: Topic }>(`/api/topics/${enc(id)}`, { jot: ref }),
  ranted: (id: string, joke: string) => patch<{ ok: boolean; topic: Topic }>(`/api/topics/${enc(id)}`, { ranted: joke }),
  /** Where each line of that version came from (the `script` store). */
  marks: (id: string, unit: string, lines: unknown[]) => patch<{ ok: boolean }>(`/api/topics/${enc(id)}`, { script: { unit, lines } }),
  rants: () => get<{ rants: any[] }>('/api/rants'),
  /** Draft the rest: no sources named = the topic's Kept, else its Found; `keep` = his lines so far, fixed. */
  draft: (topic: string, topicId: string, keep: string[], fresh = true) =>
    post<any>('/api/rant/draft', { topic, topic_id: topicId, keep, ...(fresh ? { fresh: true } : {}) }),
  line: (o: { joke?: string | null; lines: string[]; at: number; ask?: string | null; say?: string | null; fresh?: boolean }) =>
    post<any>('/api/rant/line', {
      lines: o.lines, at: o.at, ...(o.joke ? { joke: o.joke } : {}), ...(o.ask ? { ask: o.ask } : {}), ...(o.say ? { say: o.say } : {}), ...(o.fresh ? { fresh: true } : {}),
    }),
};

// the tab hears a change made elsewhere (an Undo of a delete on the topic page lands after the tab is back in view)
const changes = new Set<() => void>();
export const onTopicsChanged = (f: () => void) => { changes.add(f); return () => { changes.delete(f); }; };
export const topicsChanged = () => changes.forEach((f) => f());

const POLL_MS = 2500;
const POLL_FOR_MS = 4 * 60 * 1000; // Opus with thinking can take a while; past this the worker line says why

/** A queued job, polled to its end: the result, or why not. `onWait` hears what the job waits for. */
export async function pollJob<T>(first: any, alive: () => boolean, onWait?: (waitingFor: string | null) => void): Promise<{ ok: true; result: T } | { ok: false; error: string }> {
  let view = jobView<T>(first);
  const until = Date.now() + POLL_FOR_MS;
  while (view.state === 'waiting') {
    if (!alive()) return { ok: false, error: 'closed' };
    onWait?.(view.waitingFor || null);
    if (!view.job || Date.now() > until) return { ok: false, error: 'still waiting' };
    await new Promise((r) => setTimeout(r, POLL_MS));
    const jid: string = view.job;
    view = jobView<T>(await get(`/api/jobs/${enc(jid)}`).catch(() => ({ status: 'queued', id: jid })));
  }
  if (view.state === 'failed') return { ok: false, error: view.error };
  if (view.state !== 'done') return { ok: false, error: 'still waiting' };
  return { ok: true, result: view.result };
}

export type DraftResult = { status?: string; why?: string | null; lines?: Drafted[]; dropped?: number };
export type LineResult = { status?: string; why?: string | null; options?: { text: string }[] };

/** Save the script (§12.4, Back saves): a new version of the rant (its first version makes the bit, a rant cut), the
 *  topic told when the bit is new, then where each line came from. The version must land; a mark that fails is left
 *  out (the lines then show nothing, never a wrong mark). Claude's lines new in this version teach the profile (kept). */
export async function saveScript(o: {
  topicId: string; topicText: string; rantId: string | null; lines: SLine[]; saved: DocLine[] | null;
  /** the bit an earlier attempt made (its version failed): the retry writes onto it, never a second empty bit */
  onBit?: (id: string) => void;
}): Promise<{ id: string; unit: string; indexes: number[]; created: boolean } | null> {
  const s = toSave(o.lines);
  if (!s.text) return null;
  let id = o.rantId;
  let created = false;
  if (!id) {
    const w = await decide({ target: 'node', action: 'add', after: { type: 'joke', title: rantTitle(o.topicText), stage: 'idea' }, reason: 'a rant on a topic' });
    id = (w.decisions?.[0]?.id as string | undefined) || null;
    if (!id) throw new Error('no id came back');
    created = true;
    o.onBit?.(id);
  }
  const fresh = newAiLines(o.lines, o.saved);
  const r = await addVersion(id, s.text, 'the rant, written on its topic', fresh.length > 0, sourceOf(o.lines));
  if (created || !o.rantId) {
    try { await rant.ranted(o.topicId, id); } catch { /* the bit is saved; the topic finds it on the next try */ }
  }
  try { await rant.marks(o.topicId, r.unit, s.marks); } catch { /* no marks rather than wrong ones */ }
  return { id, unit: r.unit, indexes: r.indexes || [], created };
}

/** The options he passed over teach the profile (the standing learning loop); the one he used is a kept line through
 *  the next save. */
export function passedOver(options: string[], used: string | null, bit?: string | null) {
  for (const t of options) if (t !== used) simple.dismissDraft(t, bit || undefined);
}
