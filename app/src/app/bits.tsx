// Bits (MixBits, docs/UX_2026-10.md): every bit as a list, a status dot (kills · working · fresh · resting) and how
// long it runs; a search box that also takes a question (a question goes to the conversation); Bits · Notes · Map;
// "N pairs look like the same idea → Review" when the merge judge found some. One list request, virtualized.
import React, { memo, useCallback, useEffect, useMemo, useState } from 'react';
import { FlatList, Pressable, Text, TextInput, View } from 'react-native';
import { router } from 'expo-router';
import { ApiError, JokeSummary, get, useApi } from '@/lib/api';
import { fmtDur } from '@/lib/laughs';
import { VoiceMemos } from '@/lib/memos';
import { isQuestion, shortNameOf } from '@/lib/say';
import { C, F } from '@/lib/theme';
import { Avatar, Empty, ErrorCard, Icon, Screen, Seg, T, useWide } from '@/lib/ui';

type View_ = 'bits' | 'notes' | 'map';
type Status = 'kills' | 'working' | 'fresh' | 'resting';
const DOT: Record<Status, string> = { kills: '#30D158', working: C.amber, fresh: '#64D2FF', resting: C.tabOff };
const WORD: Record<Status, string> = { kills: 'Kills', working: 'Working', fresh: 'Fresh', resting: 'Resting' };
const ROW_H = 64;

/** One word from his tapes and his stage: kills (big laughs, more than once) · working (on stage) · fresh (never tried) · resting (retired). */
export function statusOf(j: JokeSummary): Status {
  if (j.stage === 'retired') return 'resting';
  if ((j.hit_sets || 0) >= 2 || (j.avg_vs_room != null && j.avg_vs_room >= 1.3 && (j.sets || 0) >= 2)) return 'kills';
  if ((j.sets || 0) > 0 || j.stage === 'working') return 'working';
  return 'fresh';
}
function subOf(j: JokeSummary, st: Status) {
  const bits = [WORD[st]];
  if (j.sets) bits.push(`${j.sets} set${j.sets === 1 ? '' : 's'}`);
  if (j.versions > 1) bits.push(`${j.versions} versions`);
  if (!j.sets) bits.push('never performed');
  return bits.join(' · ');
}

const Row = memo(function Row({ j }: { j: JokeSummary }) {
  const st = statusOf(j);
  return (
    <Pressable testID={`bit-${j.id}`} accessibilityRole="button" accessibilityLabel={`${j.short_title || j.title}. ${subOf(j, st)}. Open`} onPress={() => router.push(`/joke/${encodeURIComponent(j.id)}` as any)}
      style={(s: any) => ({ height: ROW_H, flexDirection: 'row', alignItems: 'center', gap: 12, paddingHorizontal: 4, borderBottomWidth: 1, borderBottomColor: '#1F1F22', backgroundColor: s.hovered ? C.card2 : 'transparent' })}>
      <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: DOT[st] }} accessibilityLabel={WORD[st]} />
      <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
        <Text numberOfLines={1} style={{ fontFamily: F.semi, fontSize: 16, color: C.text }}>{j.short_title || shortNameOf(j.title)}</Text>
        <Text numberOfLines={1} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>{subOf(j, st)}</Text>
      </View>
      <Text style={{ fontFamily: F.medium, fontSize: 14, color: C.mut2, fontVariant: ['tabular-nums'] }}>{j.duration_s != null && j.sets ? fmtDur(j.duration_s) : j.duration_s != null ? `~${fmtDur(j.duration_s)}` : '—'}</Text>
    </Pressable>
  );
});

export default function Bits() {
  const wide = useWide();
  const [view, setView] = useState<View_>('bits');
  const [q, setQ] = useState('');
  const [dq, setDq] = useState('');
  useEffect(() => {
    const t = setTimeout(() => setDq(q.trim()), 300);
    return () => clearTimeout(t);
  }, [q]);
  const question = dq.length >= 3 && isQuestion(dq);
  const list = useApi<{ items: JokeSummary[]; total: number }>(question ? null : `/api/jokes?sort=performed&limit=500${dq ? `&q=${encodeURIComponent(dq)}&mode=hybrid` : ''}`, { poll: 60000 });
  const [pairs, setPairs] = useState<number | null>(null);
  useEffect(() => {
    let alive = true;
    get<{ groups: any[] }>('/api/merge/candidates').then((r) => alive && setPairs((r.groups || []).length)).catch((e) => alive && setPairs(e instanceof ApiError ? 0 : null));
    return () => { alive = false; };
  }, []);
  const items = list.data?.items || [];
  const counts = useMemo(() => {
    const c: Record<Status, number> = { kills: 0, working: 0, fresh: 0, resting: 0 };
    for (const j of items) c[statusOf(j)]++;
    return c;
  }, [items]);
  const askIt = useCallback(() => router.push({ pathname: '/say', params: { text: dq, screen: 'bits' } } as any), [dq]);

  const header = (
    <View style={{ gap: 10, paddingBottom: 6 }}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10, height: 48, borderRadius: 24, backgroundColor: C.card, paddingHorizontal: 14 }}>
        <Icon name="search" size={18} color={C.mut2} />
        <TextInput testID="bits-search" value={q} onChangeText={setQ} placeholder="Search your bits, or ask" placeholderTextColor={C.dim} returnKeyType="search"
          onSubmitEditing={() => question && askIt()} accessibilityLabel="Search your bits, or ask"
          style={{ flex: 1, minWidth: 0, height: 48, color: C.text, fontFamily: F.body, fontSize: 16 }} />
        {!!q && <Pressable accessibilityRole="button" accessibilityLabel="Clear" onPress={() => setQ('')} style={{ width: 32, height: 32, alignItems: 'center', justifyContent: 'center' }}><Text style={{ color: C.mut2, fontSize: 18 }}>×</Text></Pressable>}
      </View>
      {question && (
        <Pressable testID="bits-ask" accessibilityRole="button" onPress={askIt} style={{ padding: 12, borderRadius: 14, backgroundColor: C.card2, flexDirection: 'row', alignItems: 'center', gap: 10 }}>
          <Icon name="chat" size={18} color={C.amber} /><T style={{ flex: 1 }}>Ask your material: “{dq}”</T><T style={{ color: C.amber }}>Ask ›</T>
        </Pressable>
      )}
      <Seg<View_> testID="bits-seg" items={[['bits', 'Bits'], ['notes', 'Notes'], ['map', 'Map']]} value={view} onChange={(v) => (v === 'map' ? router.push('/shows' as any) : setView(v))} />
      {view === 'bits' && !!pairs && (
        <Pressable testID="merge-banner" accessibilityRole="button" onPress={() => router.push('/merge' as any)}
          style={{ padding: 12, borderRadius: 14, backgroundColor: C.card2, flexDirection: 'row', alignItems: 'center', gap: 10 }}>
          <T style={{ flex: 1 }}>{pairs} {pairs === 1 ? 'pair looks' : 'pairs look'} like the same idea. Merge them into one bit each?</T>
          <T style={{ color: C.amber, fontFamily: F.semi }}>Review ›</T>
        </Pressable>
      )}
    </View>
  );

  if (view === 'notes') {
    return (
      <Screen title="Bits" testID="bits" right={<Avatar />}>
        {header}
        <VoiceMemos />
      </Screen>
    );
  }
  return (
    <Screen title="Bits" testID="bits" right={<Avatar />} scroll={false}>
      <FlatList
        data={items} keyExtractor={(j) => j.id} renderItem={({ item }) => <Row j={item} />}
        getItemLayout={(_, i) => ({ length: ROW_H, offset: ROW_H * i, index: i })} initialNumToRender={16} windowSize={9} removeClippedSubviews
        keyboardShouldPersistTaps="handled"
        contentContainerStyle={{ paddingHorizontal: wide ? 32 : 16, paddingTop: 12, paddingBottom: 140, maxWidth: wide ? 1120 : 720, width: '100%', alignSelf: 'center' }}
        ListHeaderComponent={header}
        ListEmptyComponent={list.loading && !list.data ? <Empty text="Loading your bits…" /> : list.rawError && !list.data ? <ErrorCard error={list.rawError} onRetry={list.reload} /> : <Empty text={dq ? 'Nothing matches.' : 'No bits yet. Say one.'} />}
        ListFooterComponent={items.length ? (
          <View testID="bits-legend" style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 14, paddingVertical: 14, justifyContent: 'center' }}>
            {(Object.keys(DOT) as Status[]).map((k) => (
              <View key={k} style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}><View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: DOT[k] }} /><T v="small">{WORD[k]} · {counts[k]}</T></View>
            ))}
          </View>
        ) : undefined}
      />
    </Screen>
  );
}
