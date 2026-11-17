// After (docs/design/SPEC-2026-11-16-bits-sets-tapes.md Slice 5): the take he just recorded from the walk-through,
// cut up by bit. One row per bit of the set, in the set's order: its time on tape (several visits add up), "Skipped"
// when he never reached it, and, once the phone has transcribed the take, what he actually said there (his words,
// cut by his own Next taps: rules/runRules wordsBySplit). Where what he said is mostly not in the written bit the row
// says "You said it differently" and opens the bit, where he can put the new wording in. Nothing here writes; the
// words go to the server through the recordings queue as they do for every set (with the list and the splits). No
// laughs: a tape made on the phone has none (§5 R4). Reads the local row (lib/recordings) and the set's build (for the
// bits' written words). Phone only: the web keeps no recordings and goes back to the set.
import React, { useMemo } from 'react';
import { Platform, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { useApi } from '@/lib/api';
import { useRecordings } from '@/lib/recordings';
import type { Build } from '@/lib/simple';
import { C } from '@/lib/theme';
import { Empty, List, ListRow, T, TextBtn } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TopBar } from '@/lib/TopBar';
import { mmss } from '@/rules/format';
import { isPending, statusWords } from '@/rules/memoRules';
import { afterRows } from '@/rules/runRules';

const enc = encodeURIComponent;

export default function After() {
  const p = useLocalSearchParams<{ rec: string }>();
  const recId = decodeURIComponent(String(p.rec || ''));
  const recs = useRecordings();
  const r = useMemo(() => recs.find((x) => x.id === recId) || null, [recs, recId]);
  const boardId = r?.board?.id || null;
  const b = useApi<Build>(boardId ? `/api/boards/${enc(boardId)}/build` : null);
  const texts = useMemo(() => Object.fromEntries((b.data?.steps || []).map((s) => [s.joke_id, s.text ? [s.text.setup, s.text.reveal].filter(Boolean).join('\n') : s.idea || ''])), [b.data]);
  const rows = useMemo(() => (r ? afterRows(r.run_bits || [], r.splits || [], r.bit_words, texts) : []), [r, texts]);
  const done = () => (router.canGoBack() ? router.back() : router.navigate((boardId ? `/list/${enc(boardId)}` : '/tapes') as any));
  const openBit = (id: string) => router.navigate(`/joke/${enc(id)}` as any);
  const reached = rows.filter((x) => !x.skipped).length;
  const header = <TopBar task title="After" left={<View style={{ width: 66 }} />} right={<TextBtn testID="after-done" label="Done" onPress={done} />} />;

  if (Platform.OS === 'web' || !r) {
    return (
      <Screen title="After" task deep testID="after" header={header}>
        <Empty text={Platform.OS === 'web' ? 'After is in the iPhone app.' : 'This recording is no longer on this iPhone.'} />
      </Screen>
    );
  }
  const total = mmss(r.duration_s);
  const words = r.bit_words ? null : isPending(r) ? statusWords(r) : r.state === 'sent' ? null : statusWords(r);
  return (
    <Screen title="After" task deep testID="after" header={header}>
      <View style={{ gap: 4 }}>
        <T v="h1" accessibilityRole="header">{r.title || r.board?.title || 'Set'}</T>
        <T v="mut" testID="after-sub">{`${total} · ${reached} of ${rows.length} ${rows.length === 1 ? 'bit' : 'bits'}${r.mode === 'video' ? ' · in Photos' : ''}`}</T>
        {!!words && <T v="small" testID="after-words-state">{`Your words: ${words.toLowerCase()}`}</T>}
      </View>
      {!rows.length ? <Empty text="This take followed no set." /> : (
        <List>
          {rows.map((x, k) => (
            <View key={x.id} testID={`after-row-${k}`} style={{ paddingVertical: 4 }}>
              <ListRow testID={`after-bit-${k}`} title={<T v="semi" numberOfLines={1}>{x.name}</T>}
                sub={x.skipped ? 'Skipped' : `${mmss(x.ms / 1000)}${x.spans > 1 ? ` · ${x.spans} times` : ''}`}
                onPress={() => openBit(x.id)} />
              {x.words != null && !x.skipped && (
                <T v="body" testID={`after-said-${k}`} style={{ color: x.words ? C.text : C.mut, paddingBottom: 6 }}>{x.words || 'No words heard here.'}</T>
              )}
              {x.changed && (
                <TextBtn testID={`after-changed-${k}`} label="You said it differently · Open the bit" accessibilityLabel={`You said ${x.name} differently. Open the bit`} onPress={() => openBit(x.id)} />
              )}
            </View>
          ))}
        </List>
      )}
    </Screen>
  );
}
