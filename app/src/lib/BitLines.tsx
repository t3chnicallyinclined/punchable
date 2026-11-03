// The bit as lines (SPEC 2026-11-03 §3.1, §3.2): each line a 44-pt control at the prose cap (2.0×), the punch in
// Inter 700 with one amber dot in a 14-pt gutter (never colour alone: weight, the dot, and the spoken word), the lines
// after the last punch folded under "+ 3 tags". Tap a line: it is selected (card2 fill, a 3-pt amber inset edge, a
// selection haptic) and its actions appear under it; tap it again: Write opens at that line. VoiceOver gets the same
// actions on the line itself, so nothing depends on the second tap. While an old version is read there are no actions.
import React, { useState } from 'react';
import { PixelRatio, Pressable, Text, View } from 'react-native';
import { PROSE_CAP, ROW_CAP, foldAt, foldLabel, isPunch, lineLabel, tagsFolded } from '@/rules/bitLines';
import { haptics } from './haptics';
import { C, F, linkColor } from './theme';
import { Btn } from './ui';

/** An action under a line; it receives the line it was invoked on (a VoiceOver action never goes through the stored selection). */
export type LineAction = { label: string; onPress: (i: number) => void; testID?: string; disabled?: boolean };

export function BitLines({ lines, punch, selected, onSelect, onOpenAt, actions, testID }: {
  lines: string[]; punch: number[]; selected: number | null; onSelect: (i: number | null) => void; onOpenAt: (i: number) => void;
  /** The actions under the selected line (none while reading an old version). */
  actions: LineAction[] | null; testID?: string;
}) {
  const [tagsOpen, setTagsOpen] = useState(false);
  const fs = PixelRatio.getFontScale();
  const count = lines.length;
  const fold = foldAt(count, punch);
  const folded = tagsOpen ? 0 : tagsFolded(count, punch);
  const shown = folded ? lines.slice(0, fold) : lines;
  const dot = Math.round(6 * Math.min(PROSE_CAP, Math.max(1, fs)));
  const live = (actions || []).filter((a) => !a.disabled);
  return (
    <View testID={testID}>
      {shown.map((text, i) => {
        const on = selected === i;
        const p = isPunch(punch, i);
        return (
          <View key={`${i}:${text}`}>
            <Pressable testID={testID ? `${testID}-${i}` : undefined} accessibilityRole="button" accessibilityLabel={lineLabel(i, count, punch, text)}
              accessibilityState={{ selected: on }} accessibilityHint={on ? 'Opens Write at this line' : undefined}
              accessibilityActions={live.map((a) => ({ name: a.label, label: a.label }))}
              onAccessibilityAction={(e) => { onSelect(i); live.find((a) => a.label === e.nativeEvent.actionName)?.onPress(i); }}
              onPress={() => { if (on) onOpenAt(i); else { haptics.selection(); onSelect(i); } }}
              style={{ flexDirection: 'row', alignItems: 'flex-start', gap: 8, minHeight: 44, paddingVertical: 9, paddingRight: 8, paddingLeft: 5, marginHorizontal: -8, borderRadius: 10, borderLeftWidth: 3, borderLeftColor: on ? C.amber : 'transparent', backgroundColor: on ? C.card2 : 'transparent' }}>
              <View style={{ width: 14, alignItems: 'center', justifyContent: 'center', height: Math.round(26 * Math.min(PROSE_CAP, Math.max(1, fs))) }}>
                {p && <View style={{ width: dot, height: dot, borderRadius: dot / 2, backgroundColor: C.amber }} />}
              </View>
              <Text maxFontSizeMultiplier={PROSE_CAP} style={{ flex: 1, minWidth: 0, fontFamily: p ? F.bold : F.body, fontSize: 17, lineHeight: 26, color: C.text }}>{text}</Text>
            </Pressable>
            {on && !!actions?.length && (
              <View testID={testID ? `${testID}-actions` : undefined} accessibilityLabel="Actions for this line" style={{ flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: 6, paddingLeft: 14, paddingVertical: 4 }}>
                {actions.map((a) => <Btn key={a.label} small testID={a.testID} label={a.label} disabled={a.disabled} onPress={() => a.onPress(i)} />)}
              </View>
            )}
          </View>
        );
      })}
      {folded > 0 && (
        <Pressable testID={testID ? `${testID}-fold` : undefined} accessibilityRole="button" accessibilityState={{ expanded: false }} onPress={() => setTagsOpen(true)}
          style={({ pressed }) => ({ minHeight: 44, justifyContent: 'center', paddingLeft: 14, opacity: pressed ? 0.7 : 1 })}>
          <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 15, lineHeight: 20, color: linkColor() }}>{foldLabel(folded)}</Text>
        </Pressable>
      )}
    </View>
  );
}
