// Sets (MixSets, docs/UX_2026-10.md): set lists as Sharpie-on-paper cards, each joke's short name in order (the
// boards list carries `names`, one request), tonight's first; "Performed · N" lists the recorded sets. A set list IS
// a board (/list/<id>); "New set list" is the create sheet.
import React, { useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { router } from 'expo-router';
import { BoardListItem, creative, useCreative } from '@/lib/creative';
import { fmtDur, fmtShortDate } from '@/lib/laughs';
import { useSets } from '@/lib/sets';
import { useBoards } from '@/lib/boards';
import { CreateSheet } from '@/lib/CreateSheet';
import { C, F } from '@/lib/theme';
import { Avatar, Btn, Empty, ErrorCard, FixtureBanner, List, ListRow, Screen, Seg, T, useWide } from '@/lib/ui';

type Seg_ = 'lists' | 'performed';
const PAPER = '#F2EEE4';
const INK = '#1B1B1F';
const MARKER = '#2A2A2E';

/** A set list on paper: the title in marker, the short names down the page. */
export function PaperCard({ b, current, onPress, testID }: { b: BoardListItem & { names?: string[] }; current?: boolean; onPress: () => void; testID?: string }) {
  const names = b.names || [];
  const shown = names.slice(0, 6);
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={`${b.title}, ${b.n} jokes, ${b.runtime}. Open`} onPress={onPress}
      style={({ pressed }) => ({ width: '100%', borderRadius: 6, backgroundColor: PAPER, padding: 16, paddingBottom: 18, gap: 8, transform: [{ rotate: current ? '-0.6deg' : '0.4deg' }], opacity: pressed ? 0.9 : 1, shadowColor: '#000', shadowOpacity: 0.5, shadowRadius: 10, shadowOffset: { width: 0, height: 6 } })}>
      <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 8 }}>
        <Text numberOfLines={1} style={{ flex: 1, fontFamily: F.black, fontSize: 20, letterSpacing: -0.3, color: INK }}>{b.title}</Text>
        <Text style={{ fontFamily: F.bold, fontSize: 14, color: MARKER, fontVariant: ['tabular-nums'] }}>{b.runtime}</Text>
      </View>
      <View style={{ gap: 3, paddingLeft: 6, borderLeftWidth: 2, borderLeftColor: '#D8D2C4' }}>
        {shown.map((n, i) => <Text key={i} numberOfLines={1} style={{ fontFamily: F.bold, fontSize: 17, color: MARKER, letterSpacing: -0.2 }}>{n}</Text>)}
        {names.length > shown.length && <Text style={{ fontFamily: F.semi, fontSize: 14, color: '#6B6660' }}>+ {names.length - shown.length} more</Text>}
        {!names.length && <Text style={{ fontFamily: F.semi, fontSize: 14, color: '#6B6660' }}>(empty)</Text>}
      </View>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <Text style={{ flex: 1, fontFamily: F.medium, fontSize: 12, color: '#6B6660' }}>{current ? 'tonight · ' : ''}{b.target_s ? `${fmtDur(b.target_s)} slot · ` : ''}{fmtShortDate(b.updated)}</Text>
        {b.source === 'inferred' && <Text style={{ fontFamily: F.semi, fontSize: 11, color: '#6B6660', borderWidth: 1, borderColor: '#C9C2B2', borderRadius: 4, paddingHorizontal: 5 }}>draft · found for you</Text>}
      </View>
    </Pressable>
  );
}

export default function Sets() {
  const wide = useWide();
  const cur = useBoards();
  const b = useCreative<{ boards: (BoardListItem & { names?: string[] })[] }>('boards', creative.boards, { poll: 30000 });
  const sets = useSets();
  const [seg, setSeg] = useState<Seg_>('lists');
  const [create, setCreate] = useState(false);
  const all = b.data?.boards || [];
  const his = all.filter((x) => x.source !== 'inferred');
  const found = all.filter((x) => x.source === 'inferred');
  const curId = cur.current?.id || his[0]?.id || null;
  const ordered = [...his.filter((x) => x.id === curId), ...his.filter((x) => x.id !== curId)];
  return (
    <Screen title="Sets" testID="sets" right={<View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}><Btn testID="new-setlist" small v="add" label="＋ New" onPress={() => setCreate(true)} /><Avatar /></View>}>
      <Seg<Seg_> testID="sets-seg" items={[['lists', 'Set lists'], ['performed', `Performed · ${sets.list.length}`]]} value={seg} onChange={setSeg} />
      {seg === 'lists' ? (
        <>
          <FixtureBanner show={b.fixture} what="GET /api/boards" />
          {b.error && !b.data ? <ErrorCard error={b.error} onRetry={b.reload} /> : null}
          {b.data && !his.length && <Empty text="No set list yet. ＋ New, or say “new 7 for Friday”."><Btn small v="add" label="New set list" onPress={() => setCreate(true)} /></Empty>}
          <View style={{ flexDirection: wide ? 'row' : 'column', flexWrap: 'wrap', gap: 18, paddingVertical: 6 }}>
            {ordered.map((x) => (
              <View key={x.id} style={{ width: wide ? 300 : '100%' }}>
                <PaperCard b={x} current={x.id === curId} testID={`setlist-${x.id}`} onPress={() => { cur.setCurrent({ id: x.id, title: x.title }); router.push(`/list/${encodeURIComponent(x.id)}` as any); }} />
              </View>
            ))}
          </View>
          {!!found.length && (
            <View style={{ gap: 6 }}>
              <T v="eb">Found for you</T>
              <T v="small">Bits that share a world. Open one and make it yours.</T>
              <List>
                {found.map((x) => <ListRow key={x.id} testID={`found-${x.id}`} title={x.title} sub={`${x.n} jokes · ~${x.runtime}`} onPress={() => router.push(`/list/${encodeURIComponent(x.id)}` as any)} />)}
              </List>
            </View>
          )}
        </>
      ) : (
        <View>
          {sets.error && !sets.list.length ? <ErrorCard error={sets.error} /> : null}
          {!sets.list.length && !sets.error && <Empty text="No recorded sets yet. Record one from ＋, or import a YouTube link." />}
          <List>
            {sets.list.map((x) => (
              <ListRow key={x.id} testID={`performed-${x.id}`} title={<T v="semi" numberOfLines={1}>{x.venue.name}</T>}
                sub={`${fmtShortDate(x.show.date)} · ${fmtDur(x.card.duration_s)}${x.best ? ` · ${x.best.title.split(' (')[0]}` : ''}`}
                onPress={() => router.push(`/set/${encodeURIComponent(x.id)}` as any)} />
            ))}
          </List>
        </View>
      )}
      <CreateSheet visible={create} onClose={() => setCreate(false)} start="set" />
    </Screen>
  );
}
