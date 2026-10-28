// What I've learned about you (UX_SPEC 4.12, P2): the AI's read of his taste. Until GET /api/learned exists
// (requested by app), it shows the cold-start line from his own decision count.
import React, { useMemo } from 'react';
import { View } from 'react-native';
import { useApi } from '@/lib/api';
import { Card, List, ListRow, T } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

export default function Learned() {
  const learned = useApi<any>('/api/learned', { cache: false });
  const dec = useApi<{ decisions?: any[]; items?: any[] }>('/api/decisions?limit=1000', { cache: false });
  const mine = useMemo(() => ((dec.data?.decisions || dec.data?.items || (Array.isArray(dec.data) ? dec.data : [])) as any[]).filter((d) => !d.undone && !('undo_of' in d)), [dec.data]);
  const byKind = useMemo(() => {
    const m = new Map<string, number>();
    for (const d of mine) {
      const k = d.target === 'edge' ? 'connections and labels' : String(d.id || '').startsWith('perf:') ? 'ratings' : 'stages and ideas';
      m.set(k, (m.get(k) || 0) + 1);
    }
    return [...m.entries()];
  }, [mine]);
  const statements: any[] = learned.data?.statements || [];
  return (
    <Screen title="What I’ve learned" back testID="learned">
      <T v="h1">What I’ve learned about you</T>
      <Card style={{ gap: 6 }} testID="learned-status">
        <T>{learned.data?.status_line || `You’ve made ${mine.length} decisions. I start describing your taste after 5 of one kind.`}</T>
        <List>{byKind.map(([k, n]) => <ListRow key={k} title={k} sub={`${n} ${n === 1 ? 'decision' : 'decisions'}`} chevron={false} />)}</List>
      </Card>
      {!!statements.length && (
        <Card style={{ gap: 6 }}>
          {statements.map((st, i) => <View key={i}><T>{st.text}</T><T v="small">from {typeof st.support === 'object' && st.support ? st.support.n : (st.support ?? st.n)} decisions</T></View>)}
        </Card>
      )}
      <T v="small">New AI versions only replace the old one if they do better on your checked examples. Last check: not enough examples yet.</T>
    </Screen>
  );
}
