// A set made from a Rant topic (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §3.6, Slice 6): the topic's rants and
// kept bits are the set's bits; what he kept from his TAPES for that topic (a moment is not a bit) is listed under the
// bits as "Also kept", read through the topic (`seed.topic`, GET /api/topics/<id>) until he makes one a bit. The
// topic's "Draft a rant from a topic" lives in the set's More menu as "Draft it as one piece…" (§7 Q4), which opens
// the topic's rant script. A set no topic made shows nothing here.
import React from 'react';
import { View } from 'react-native';
import { router } from 'expo-router';
import { useApi } from './api';
import type { BoardListItem } from './creative';
import { List, ListRow, T } from './ui';
import { fmtShortDate } from '@/rules/format';
import { boardOfTopic } from '@/rules/tapesRules';

const enc = encodeURIComponent;
type Moment = { ref: string; kind: string; words?: string; venue?: string | null; at?: string | null; set?: string | null; start_ms?: number | null };

/** The topic a set was made from, or null (the boards list carries each list's seed; live and archived). */
export function useTopicOf(boardId: string): string | null {
  const live = useApi<{ boards: BoardListItem[] }>('/api/boards');
  const arch = useApi<{ boards: BoardListItem[] }>('/api/boards?archived=1');
  const all = [...(live.data?.boards || []), ...(arch.data?.boards || [])];
  const b = all.find((x) => x.id === boardId);
  if (!b?.seed?.topic) return null;
  return boardOfTopic(all, b.seed.topic)?.id === boardId ? b.seed.topic : null;
}

/** "Also kept": the taped moments he kept for the topic this set was made from. */
export function AlsoKept({ topic }: { topic: string | null }) {
  const t = useApi<{ kept: Moment[] }>(topic ? `/api/topics/${enc(topic)}` : null);
  const moments = (t.data?.kept || []).filter((m) => m.kind === 'moment' && m.words);
  if (!topic || !moments.length) return null;
  return (
    <View testID="also-kept" style={{ gap: 2, marginTop: 12 }}>
      <T v="eb" accessibilityRole="header">{`Also kept · ${moments.length}`}</T>
      <T v="small">From your tapes, kept for this topic. Open one to make it a bit.</T>
      <List>
        {moments.map((m) => (
          <ListRow key={m.ref} testID={`also-kept-${m.ref}`} title={<T numberOfLines={3}>{m.words}</T>}
            sub={['On tape', m.venue || null, m.at ? fmtShortDate(m.at) : null].filter(Boolean).join(' · ')}
            onPress={m.set ? () => router.push(`/set/${enc(m.set!)}${m.start_ms != null ? `?at=${Math.round(m.start_ms)}` : ''}` as any) : undefined} />
        ))}
      </List>
    </View>
  );
}
