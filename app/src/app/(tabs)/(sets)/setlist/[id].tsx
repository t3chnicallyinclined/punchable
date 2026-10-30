// Set list (UX_SPEC 4.5 primary action): huge type, 1–3 words per bit, target times, one try-tonight hint per bit.
// The screen stays awake while it is open.
import React from 'react';
import { Text, View } from 'react-native';
import { router, useLocalSearchParams } from 'expo-router';
import { useKeepAwake } from 'expo-keep-awake';
import { WSView, creative, useCreative } from '@/lib/creative';
import { mmss } from '@/rules/format';
import { C, F } from '@/lib/theme';
import { Btn, Empty, T } from '@/lib/ui';
import { Screen } from '@/lib/Screen';
import { TopBar } from '@/lib/TopBar';

/** "The dog is the size of a couch (how to measure)" -> "Dog couch size". */
export function cue(title: string) {
  const stop = new Set(['is', 'a', 'the', 'my', 'of', 'as', 'to', 'and', 'in', 'on', 'i', 'about']);
  const words = title.split(' (')[0].replace(/[:/]/g, ' ').split(/\s+/).filter((w) => w && !stop.has(w.toLowerCase()));
  return words.slice(0, 3).join(' ');
}

export default function SetList() {
  useKeepAwake();
  const p = useLocalSearchParams<{ id: string }>();
  const id = decodeURIComponent(String(p.id || ''));
  const v = useCreative<WSView>(`ws:${id}`, () => creative.workingset(id));
  const W = v.data;
  return (
    <Screen title="Set list" task deep testID="setlist" header={<TopBar title="Set list" back task onBack={() => (router.canGoBack() ? router.back() : router.navigate('/boards' as any))} />}
      dock={<Btn label="Done" onPress={() => (router.canGoBack() ? router.back() : router.navigate('/boards' as any))} />}>
      {!W ? <Empty text="Loading…" /> : (
        <View style={{ gap: 18 }}>
          <T v="small" testID="awake">Screen stays on · {mmss(W.bits.reduce((a, b) => a + b.target_s, 0))} of {mmss(W.version.target_s)}</T>
          {W.bits.map((b, i) => (
            <View key={b.joke_id} testID={`cue-${i}`} style={{ gap: 2 }}>
              <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 12 }}>
                <Text maxFontSizeMultiplier={1.2} style={{ fontFamily: F.g7, fontSize: 40, lineHeight: 46, color: C.text, flex: 1 }}>{cue(b.title)}</Text>
                <Text style={{ fontFamily: F.g6, fontSize: 20, color: C.mut, fontVariant: ['tabular-nums'] }}>{mmss(b.target_s)}</Text>
              </View>
              {!!b.notes[0] && <Text style={{ fontFamily: F.body, fontSize: 16, color: C.amber }}>try: {b.notes[0].text.replace(/^[^:]+:\s*/, '').slice(0, 70)}</Text>}
            </View>
          ))}
        </View>
      )}
    </Screen>
  );
}
