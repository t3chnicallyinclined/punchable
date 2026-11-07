// Filmed (SPEC 2026-11-06 §12.2): the takes of rants kept on this phone, newest first: the topic (or the rant) and
// "Nov 6 · 0:58 · in Photos"; a tap plays the take. The video is in Photos; only its words ever reached the server.
import React from 'react';
import { router } from 'expo-router';
import { useApi } from '@/lib/api';
import { useRecordings } from '@/lib/recordings';
import { Screen } from '@/lib/Screen';
import { Empty, List, ListRow, T } from '@/lib/ui';
import { filmedList, filmedSub, type RantRow, type Take, type Topic } from '@/rules/rantRules';

const enc = encodeURIComponent;

export default function Filmed() {
  const takes = useRecordings();
  const rs = useApi<{ rants: RantRow[] }>('/api/rants');
  const tp = useApi<{ topics: Topic[] }>('/api/topics');
  const list = filmedList(takes as Take[], rs.data?.rants || [], tp.data?.topics || []);
  return (
    <Screen title="Filmed" testID="filmed" backLabel="Rant">
      {!list.length && <Empty text="Nothing filmed on this phone yet." />}
      <List>
        {list.map(({ take, rant, topic }) => (
          <ListRow key={take.id} testID={`filmed-${take.id}`} title={<T v="semi" numberOfLines={2}>{topic?.text || rant.short_name || rant.title}</T>}
            sub={filmedSub(take)} onPress={() => router.push(`/take/${enc(take.id)}` as any)} />
        ))}
      </List>
    </Screen>
  );
}
