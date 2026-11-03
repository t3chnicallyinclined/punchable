// The one muted idea line under the bar (SPEC 2026-11-03 §3.1): `joke.summary` as a 44-pt control, one line (two from
// the re-stack point), a tiny AI badge while Claude's words stand and he has not touched them (the standing badge
// rule), the placeholder "Idea" when empty. Tap = edit. VoiceOver: "Idea: … Written by Claude. Edit".
import React from 'react';
import { PixelRatio, Pressable, Text, View } from 'react-native';
import { ROW_CAP, ideaLines } from '@/rules/bitLines';
import { C, F } from './theme';
import { AIBadge } from './ui';

export function IdeaLine({ summary, ai, onEdit, testID }: { summary?: string | null; ai?: boolean; onEdit: () => void; testID?: string }) {
  const fs = PixelRatio.getFontScale();
  const text = (summary || '').trim();
  return (
    <Pressable testID={testID} accessibilityRole="button" onPress={onEdit}
      accessibilityLabel={text ? `Idea: ${text}.${ai ? ' Written by Claude.' : ''} Edit` : 'Idea. Add one'}
      style={({ pressed }) => ({ minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: 6, opacity: pressed ? 0.7 : 1 })}>
      <Text maxFontSizeMultiplier={ROW_CAP} numberOfLines={ideaLines(fs)} style={{ flex: 1, minWidth: 0, fontFamily: F.body, fontSize: 15, lineHeight: 20, color: text ? C.mut2 : C.dim }}>{text || 'Idea'}</Text>
      {!!text && ai && <View style={{ flexShrink: 0 }}><AIBadge testID={testID ? `${testID}-ai` : undefined} /></View>}
    </Pressable>
  );
}
