// Tapes (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §2.7, Slice 6): ONE list of everything he recorded, newest
// first: the recordings kept on this phone (sets taped here, filmed sets and takes, voice memos: lib/recordings, with
// their state, play, Delete, Try again) and the sets the server holds (GET /api/sets; a row opens the set's page). It
// replaces Sets › Recorded, Bits' takes list and the Rant tab's Filmed (/filmed redirects here). Rows open today's
// pages (Slice 7 makes a tape its transcript cut by bit). A set taped on this phone whose words reached the server is
// shown once (rules/tapesRules). The toolbar records: a set (the free recorder) or a memo; the avatar opens You and
// Settings, as on every tab.
import React, { useCallback, useMemo } from 'react';
import { Platform, View } from 'react-native';
import { Stack, router, useFocusEffect } from 'expo-router';
import { useRecordings } from '@/lib/recordings';
import { RecRow } from '@/lib/RecRows';
import { useSets } from '@/lib/sets';
import { Screen } from '@/lib/Screen';
import { C } from '@/lib/theme';
import { Avatar, Empty, ErrorCard, Icon, IconBtn, List, ListRow, Skeleton, T, useUI } from '@/lib/ui';
import { fmtShortDate, mmss } from '@/rules/format';
import { tapeKind, tapeRows, type PhoneTape, type ServerTape } from '@/rules/tapesRules';

const native = Platform.OS !== 'web';
const enc = encodeURIComponent;

export default function Tapes() {
  const ui = useUI();
  const recs = useRecordings();
  const sets = useSets();
  useFocusEffect(useCallback(() => { void sets.reload?.(); }, [])); // eslint-disable-line react-hooks/exhaustive-deps
  const server = useMemo<ServerTape[]>(() => sets.list.map((x) => ({ id: x.id, title: x.title || x.venue?.name || 'Set', date: x.show?.date || null })), [sets.list]);
  const rows = useMemo(() => tapeRows(recs as PhoneTape[], server), [recs, server]);
  const byId = useMemo(() => new Map(sets.list.map((x) => [x.id, x])), [sets.list]);
  const recordSet = () => ui.withMic(() => router.push('/record' as any));
  const recordMemo = () => ui.withMic(() => router.push('/record?kind=memo' as any));

  return (
    <Screen title="Tapes" testID="tapes" largeTitle={native} onRefresh={() => sets.reload?.()}
      right={native ? undefined : <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
        <IconBtn testID="tapes-record" label="Record a set" onPress={recordSet}><Icon name="record" size={22} color={C.rec} /></IconBtn>
        <Avatar /></View>}>
      {native && (
        <Stack.Toolbar placement="right">
          {/* HIG/toolbars: the record glyph, never play; the two recorders in one menu (a set, a memo) */}
          <Stack.Toolbar.Menu icon="record.circle" accessibilityLabel="Record">
            <Stack.Toolbar.MenuAction icon="record.circle" onPress={recordSet}>Record a set</Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="mic" onPress={recordMemo}>Record a memo</Stack.Toolbar.MenuAction>
          </Stack.Toolbar.Menu>
          <Stack.Toolbar.View><Avatar ask={false} /></Stack.Toolbar.View>
        </Stack.Toolbar>
      )}
      {sets.error && !sets.list.length ? <ErrorCard error={sets.error} onRetry={sets.reload} /> : null}
      {sets.loading && !recs.length && <Skeleton lines={5} height={22} testID="tapes-skeleton" />}
      {!sets.loading && !rows.length && <Empty text="Nothing recorded yet. Open a set and tap Record or Film, or record a memo from the record button." />}
      {!!rows.length && <T v="small" testID="tapes-count">{rows.length} {rows.length === 1 ? 'tape' : 'tapes'}</T>}
      <List>
        {rows.map((r) => {
          if (r.from === 'phone') {
            const rec = recs.find((x) => x.id === r.rec.id);
            return rec ? <View key={r.key} testID={`tape-${r.rec.id}`}><RecRow r={rec} /></View> : null;
          }
          const x = byId.get(r.set.id);
          const sub = [tapeKind(r), r.set.date ? fmtShortDate(r.set.date) : null, x ? mmss(x.card.duration_s) : null, x?.best ? x.best.title.split(' (')[0] : null].filter(Boolean).join(' · ');
          return (
            <ListRow key={r.key} testID={`tape-${r.set.id}`} title={<T v="semi" numberOfLines={1}>{r.set.title}</T>} sub={sub}
              onPress={() => router.push(`/set/${enc(r.set.id)}` as any)} />
          );
        })}
      </List>
    </Screen>
  );
}
