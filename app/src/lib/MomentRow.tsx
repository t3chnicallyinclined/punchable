// One taped moment as a row (rules/searchRows): where and when, the line he said, the bit it belongs to if any; tap =
// the set screen at that second, playing. Used under a Bits search ("In your sets") and on a bit's page ("Elsewhere").
import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { router } from 'expo-router';
import { Moment, momentHref, oneLine, whenWhere } from '@/rules/searchRows';
import { C, F } from './theme';

export function MomentRow({ m, testID, showBit = true }: { m: Moment; testID?: string; showBit?: boolean }) {
  const href = momentHref(m);
  const ww = whenWhere(m);
  const line = oneLine(m.text);
  return (
    <Pressable testID={testID} accessibilityRole="button" disabled={!href}
      accessibilityLabel={`${ww}. ${line}${showBit && m.joke_title ? `. In ${m.joke_title}` : ''}. Hear it`}
      onPress={() => href && router.push(href as any)}
      style={(s: any) => ({ minHeight: 56, paddingVertical: 8, paddingHorizontal: 12, borderRadius: 12, gap: 2, backgroundColor: s.pressed || s.hovered ? C.card2 : C.card })}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ flex: 1, fontFamily: F.semi, fontSize: 13, color: C.amber }}>{ww || 'On tape'}</Text>
        {showBit && !!m.joke_title && <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ maxWidth: '45%', fontFamily: F.body, fontSize: 12, color: C.mut2 }}>in {m.joke_title}</Text>}
      </View>
      {!!line && <Text numberOfLines={2} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 15, lineHeight: 21, color: C.text }}>“{line}”</Text>}
    </Pressable>
  );
}
