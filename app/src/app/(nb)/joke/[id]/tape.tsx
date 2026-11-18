// On tape (SPEC 2026-11-03 §3.7): a bit's tape history, pushed from the On tape row: the performed rows (each a deep
// link into the set at that second) and, from Film it (SPEC 2026-11-03 §2.7), the FILMED rows: the takes he kept on
// this phone, read from the recordings index ("Filmed · Nov 3 · 0:48 · 112 words"; the play glyph opens the take;
// Delete the words is the row's own). One read: /api/jokes/<id>/similar. The taped moments and the bits that sound
// like it were here until the redesign's Slice 1 (SPEC 2026-11-16 §2.2): they are groups of "Around this bit" on the
// bit page now (On stage, Same idea), where "link it" and "merge it in" live.
import React, { useMemo } from 'react';
import { View } from 'react-native';
import { useLocalSearchParams } from 'expo-router';
import { useApi } from '@/lib/api';
import { MomentRow } from '@/lib/MomentRow';
import { RecRows } from '@/lib/RecRows';
import { useRecordings } from '@/lib/recordings';
import type { SimilarResult } from '@/rules/searchRows';
import { countWord, killedCount, killedWord } from '@/rules/bitLines';
import { ErrorCard, Skeleton, T } from '@/lib/ui';
import { Screen } from '@/lib/Screen';

const enc = encodeURIComponent;

export default function TapePage() {
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const sim = useApi<SimilarResult>(`/api/jokes/${enc(id)}/similar?limit=8`);
  const reload = () => { sim.reload(); };
  const recs = useRecordings();
  const takes = useMemo(() => recs.filter((r) => r.kind === 'film' && r.bit?.id === id), [recs, id]);

  if (sim.rawError && !sim.data) return <Screen title="On tape" back><ErrorCard error={String(sim.rawError)} onRetry={reload} /></Screen>;
  if (!sim.data) return <Screen title="On tape" back><Skeleton lines={5} height={18} testID="tape-skeleton" /></Screen>;
  const s = sim.data;
  const nights = s.performed.length;
  const killed = killedCount(s.performed as { rating?: string | null }[]);
  return (
    <Screen title="On tape" back testID="tapepage" maxWidth={760}>
      <View style={{ gap: 8 }} testID="tape-performed">
        <T v="eb" accessibilityRole="header">{nights ? [countWord(nights, 'night', 'nights'), killedWord(killed)].filter(Boolean).join(' · ') : 'Never on tape'}</T>
        {s.performed.map((x, i) => <MomentRow key={x.id} m={x} showBit={false} testID={`tape-performed-${i}`} />)}
      </View>
      {!!takes.length && (
        <View style={{ gap: 8 }} testID="tape-filmed">
          <T v="eb" accessibilityRole="header">{countWord(takes.length, 'take filmed', 'takes filmed')}</T>
          <RecRows recs={takes} testID="tape-take" />
        </View>
      )}
    </Screen>
  );
}
