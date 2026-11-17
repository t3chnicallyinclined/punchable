// A set as a card on the Sets tab (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §2.5, Slice 4): its name, how many
// bits and how long, the bits' names in order on one run (three lines at most), then "Tonight", the slot and when he
// last edited it. The words are rules/setNoteRules (cardMeta, cardNames). The system card colours, so it reads in light and dark.
// "Last recorded" waits for Slice 5: the server is not yet told which set a tape followed.
import React from 'react';
import { PixelRatio, Pressable, Text, View } from 'react-native';
import type { BoardListItem } from './creative';
import type { rowActions } from './RowMenu';
import { fmtShortDate, mmss } from '@/rules/format';
import { stackMeta } from '@/rules/bitLines';
import { cardFoot, cardMeta, cardNames } from '@/rules/setNoteRules';
import { C, F } from './theme';

export function SetCard({ b, current, onPress, onLongPress, a11y, testID }: {
  b: BoardListItem & { names?: string[] }; current?: boolean; onPress: () => void; onLongPress?: () => void; a11y?: ReturnType<typeof rowActions>; testID?: string;
}) {
  const names = cardNames(b.names);
  const meta = cardMeta(b.n, b.runtime);
  const foot = cardFoot(!!current, b.target_s ? mmss(b.target_s) : null, b.updated ? fmtShortDate(b.updated) : null);
  // at the large text sizes the count goes under the name, as the section rows do (HIG.md §6)
  const stacked = stackMeta(PixelRatio.getFontScale());
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={[b.title, meta, names, foot].filter(Boolean).join(', ')} accessibilityHint="Opens the set" onPress={onPress} onLongPress={onLongPress} delayLongPress={350} {...a11y}
      style={(st: any) => ({ width: '100%', minHeight: 64, borderRadius: 12, paddingVertical: 12, paddingHorizontal: 14, gap: 4, backgroundColor: st.pressed || st.hovered ? C.card2 : C.card })}>
      <View style={{ flexDirection: stacked ? 'column' : 'row', alignItems: stacked ? 'flex-start' : 'baseline', gap: stacked ? 0 : 8 }}>
        <Text numberOfLines={stacked ? 2 : 1} maxFontSizeMultiplier={1.6} style={{ flex: stacked ? undefined : 1, minWidth: 0, fontFamily: F.bold, fontSize: 17, lineHeight: 22, color: C.text }}>{b.title}</Text>
        <Text testID={testID ? `${testID}-meta` : undefined} numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2, fontVariant: ['tabular-nums'] }}>{meta}</Text>
      </View>
      <Text testID={testID ? `${testID}-names` : undefined} numberOfLines={3} maxFontSizeMultiplier={1.6} style={{ fontFamily: F.body, fontSize: 15, lineHeight: 21, color: names ? C.text2 : C.mut2 }}>{names || 'Nothing in it yet'}</Text>
      {!!foot && <Text numberOfLines={1} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2 }}>{foot}</Text>}
    </Pressable>
  );
}
