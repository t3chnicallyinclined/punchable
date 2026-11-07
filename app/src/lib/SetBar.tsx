// Tonight's set list as one bar (a segment per bit, width = its length, red = a gap before it) and the card around it
// that opens the list. Moved off the retired Tonight tab to the top of Sets (SPEC 2026-11-06 R2).
import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { router } from 'expo-router';
import type { TonightScreen } from './say';
import { mmss } from '@/rules/format';
import { C, F } from './theme';

/** The set as one bar: a segment per bit, width = its length, red = a gap before it (no link to the bit before). */
export function SetBar({ segments, height = 10, testID }: { segments: { short_name: string; duration_s: number; gap: boolean }[]; height?: number; testID?: string }) {
  if (!segments.length) return <View testID={testID} style={{ height, borderRadius: height / 2, backgroundColor: C.surface2 }} />;
  return (
    <View testID={testID} style={{ flexDirection: 'row', gap: 3, height, borderRadius: height / 2, overflow: 'hidden' }}>
      {segments.map((sg, i) => (
        <View key={i} testID={`${testID}-seg-${i}`} accessibilityLabel={`${sg.short_name}${sg.gap ? ', gap before it' : ''}`}
          style={{ flex: Math.max(10, sg.duration_s || 30), backgroundColor: sg.gap ? C.gap : C.amber, opacity: sg.gap ? 1 : 0.7 + 0.3 * ((i % 3) / 2) }} />
      ))}
    </View>
  );
}

/** Tonight's list as a card: the title and run time, the bar, the short names; tap opens the list (or starts one). */
export function TonightCard({ board, onNew, testID = 'tonight-set' }: { board: TonightScreen['board'] | undefined; onNew: () => void; testID?: string }) {
  const b = board || null;
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={b ? `${b.title}, ${b.runtime}. Open the set list` : 'No set list yet. Start one for tonight'}
      onPress={() => (b ? router.push(`/list/${encodeURIComponent(b.id)}` as any) : onNew())}
      style={({ pressed }) => ({ padding: 16, borderRadius: 20, backgroundColor: C.card, gap: 10, opacity: pressed ? 0.85 : 1 })}>
      <View style={{ flexDirection: 'row', justifyContent: 'space-between', alignItems: 'baseline' }}>
        <Text numberOfLines={1} style={{ flex: 1, fontFamily: F.bold, fontSize: 15, color: C.text }}>{b ? `Your ${Math.round(b.target_s / 60)} for tonight` : 'No set list yet'}</Text>
        <Text testID={`${testID}-runtime`} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>{b ? mmss(b.runtime_s) : ''}</Text>
      </View>
      <SetBar segments={b?.segments || []} testID={`${testID}-bar`} />
      <Text numberOfLines={2} style={{ fontFamily: F.body, fontSize: 13, color: C.mut2 }}>
        {b ? b.segments.map((sg, i) => (
          <Text key={i}>{i ? ' · ' : ''}{sg.gap ? <Text style={{ color: C.gapText }}>gap · </Text> : null}{sg.short_name}</Text>
        )) : 'Tap here to start one for tonight.'}
      </Text>
    </Pressable>
  );
}
