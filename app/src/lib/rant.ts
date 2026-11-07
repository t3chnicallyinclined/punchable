// The Rant tab's calls (docs/design/SPEC-2026-11-06-rant-tab.md §2, §3; pk-api writes/rant.rs, Rust only): his topics,
// the bits with a rant cut, a rant drafted from a topic and his ticked sources (a `rant_draft` job for pk-worker), and
// Save rant (a new bit, its first version a rant cut, linked to the bits it drew on, the topic marked ranted). The
// pure rules are rules/rantRules.ts.
import { useEffect, useState } from 'react';
import { decide, get, patch, post } from './api';
import { addVersion } from './bitWork';
import { simple } from './simple';
import { fromLinks, matchCount, rantTitle, skippedNew, type DraftLine, type Marks, type RantRow, type Topic } from '@/rules/rantRules';

const enc = encodeURIComponent;

export const rant = {
  topics: () => get<{ topics: Topic[] }>('/api/topics'),
  addTopic: (text: string) => post<{ ok: boolean; topic: Topic }>('/api/topics', { text }),
  deleteTopic: (id: string) => patch<{ ok: boolean }>(`/api/topics/${enc(id)}`, { delete: true }),
  ranted: (id: string, joke: string) => patch<{ ok: boolean; topic: Topic }>(`/api/topics/${enc(id)}`, { ranted: joke }),
  rants: () => get<{ rants: RantRow[] }>('/api/rants'),
  /** 202 with the job to poll, or 200 with a cached result (rules/sessionRules jobView reads both). */
  draft: (topic: string, sources: string[], topicId?: string | null, fresh = false) =>
    post<any>('/api/rant/draft', { topic, sources, ...(topicId ? { topic_id: topicId } : {}), ...(fresh ? { fresh: true } : {}) }),
};

/** How many of his bits each topic matches (today's search, close hits only): asked once per topic text per session,
 *  three at a time; a failed search leaves the count out (the row shows nothing rather than a wrong number). */
const counts = new Map<string, number>();
export function useTopicCounts(topics: Topic[]): Record<string, number> {
  const [, bump] = useState(0);
  const key = topics.map((t) => t.text).join('\n');
  useEffect(() => {
    let alive = true;
    const todo = topics.filter((t) => !t.ranted_at && !counts.has(t.text));
    (async () => {
      for (let i = 0; i < todo.length; i += 3) {
        await Promise.all(todo.slice(i, i + 3).map(async (t) => {
          try { counts.set(t.text, matchCount(await get(`/api/search?q=${enc(t.text)}&limit=12`))); } catch {}
        }));
        if (alive) bump((n) => n + 1);
      }
    })();
    return () => { alive = false; };
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps
  return Object.fromEntries(topics.filter((t) => counts.has(t.text)).map((t) => [t.id, counts.get(t.text) as number]));
}

/** Save rant (§3.4): the bit, its first version (a rant cut, `source: rant_draft`, `draft: true` so the profile learns
 *  what he kept), the "from" links to the bits whose lines he kept, the topic marked ranted, and the skipped new lines
 *  sent as lines he passed over. The bit and its version must land; a link or the topic mark that fails is reported,
 *  never undone. Resolves to the bit's id and what did not land. */
export async function saveRant(o: {
  topic: string; topicId?: string | null; lines: DraftLine[]; marks: Marks; text: string;
  /** the bit an earlier attempt made (its version failed): the retry writes onto it, never a second empty bit */
  into?: string | null; onBit?: (id: string) => void;
}): Promise<{ id: string; missed: string[] }> {
  let id: string | null = o.into || null;
  if (!id) {
    const w = await decide({ target: 'node', action: 'add', after: { type: 'joke', title: rantTitle(o.topic), stage: 'idea' }, reason: 'a rant from Write a rant' });
    id = (w.decisions?.[0]?.id as string | undefined) || null;
    if (!id) throw new Error('no id came back');
    o.onBit?.(id);
  }
  await addVersion(id, o.text, 'written with Write a rant', true, 'rant_draft');
  const missed: string[] = [];
  for (const src of fromLinks(o.lines, o.marks)) {
    try {
      await decide({ target: 'edge', action: 'add', after: { src: id, dst: src, kind: 'custom', custom_kind: 'from' }, reason: 'the rant draws on it' });
    } catch { missed.push('a link'); }
  }
  if (o.topicId) {
    try { await rant.ranted(o.topicId, id); } catch { missed.push('the topic mark'); }
  }
  for (const line of skippedNew(o.lines, o.marks)) simple.dismissDraft(line, id);
  return { id, missed };
}
