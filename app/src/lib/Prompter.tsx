// The prompter band (docs/design/SPEC-2026-11-03-film-it.md §2.3; the spike's §5): his lines, the current one at the
// top of the band (the band is pinned under the Dynamic Island, so the top line is the one nearest the front lens), at
// most four showing (two from font scale 1.3), each at type.prompter (26 / 32 Inter 600, DESIGN.md §4), the punch
// Inter 700 with the amber dot in the 14-pt gutter (weight, the dot and the word "punch" for VoiceOver: never colour
// alone); the lines after the current one a dim step (label.3), never a smaller size. The band is one VoiceOver
// element: "Line 2 of 7, punch: …", a line change announced, the two gestures also as actions. Tap = Next line;
// long-press = Hold / Go (the dock's 60-pt buttons do the same: HIG/accessibility "offer alternatives to gestures").
// A line change fades 120 ms; under Reduce Motion it cuts. The pacing is rules/prompterRules; this draws it.
import React, { useEffect, useRef, useState } from 'react';
import { AccessibilityInfo, Animated, PixelRatio, Pressable, View, useWindowDimensions } from 'react-native';
import { isPunch } from '@/rules/bitLines';
import { SWAP_MS, bandLabel, bandLines, bandWindow } from '@/rules/prompterRules';
import { C, F } from './theme';
import { useReduceMotion } from './ui';

/** type.prompter (DESIGN.md §4, PROPOSED → set here): 26 / 32, Inter 600; the punch 700. Cap 1.6: the band is a
 *  control's height (it must leave the preview a face), and he reads it at arm's length, not at reading distance. */
export const PROMPTER_SIZE = 26;
export const PROMPTER_LEADING = 32;
export const PROMPTER_CAP = 1.6;
/** The band's vertical padding (space.5) and the gutter the dot sits in (the bit page's 14). */
const PAD = 12;
const GUTTER = 14;
/** The band never takes more than this share of the window: the preview must stay a face (SPEC §2.3). */
const MAX_SHARE = 0.4;

export function PromptBand({ lines, punch, i, running, held, onNext, onToggleHold, testID }: {
  lines: string[]; punch: number[]; i: number; running: boolean; held: boolean; onNext: () => void; onToggleHold: () => void; testID?: string;
}) {
  const reduceMotion = useReduceMotion();
  const { height } = useWindowDimensions();
  const fs = PixelRatio.getFontScale();
  const n = bandLines(fs);
  const shown = bandWindow(lines, i, n);
  const cur = Math.max(0, Math.min(i, Math.max(0, lines.length - 1)));
  const [fade] = useState(() => new Animated.Value(1));
  const first = useRef(true);
  useEffect(() => {
    if (first.current) { first.current = false; return; }
    if (!reduceMotion) {
      fade.setValue(0.35);
      Animated.timing(fade, { toValue: 1, duration: SWAP_MS, useNativeDriver: true }).start();
    }
    // iOS has no live region: the new current line is said once (the clock is not announced)
    if (running && lines[cur]) AccessibilityInfo.announceForAccessibility?.(lines[cur]);
  }, [cur]); // eslint-disable-line react-hooks/exhaustive-deps
  const scale = Math.min(PROMPTER_CAP, Math.max(1, fs));
  const lh = Math.round(PROMPTER_LEADING * scale);
  const dot = Math.round(6 * scale);
  const label = lines.length ? bandLabel(cur, lines.length, punch, lines[cur] || '') : 'Nothing to read';
  const actions = running
    ? [{ name: 'next', label: 'Next line' }, { name: 'hold', label: held ? 'Go' : 'Hold' }]
    : [{ name: 'next', label: 'Next line' }];
  return (
    <Pressable testID={testID} accessible accessibilityRole="button" accessibilityLabel={label}
      accessibilityHint={running ? 'Tap for the next line. Hold to pause the lines.' : 'Tap for the next line.'}
      accessibilityActions={actions} onAccessibilityAction={(e) => (e.nativeEvent.actionName === 'hold' ? onToggleHold() : onNext())}
      onPress={onNext} onLongPress={running ? onToggleHold : undefined} delayLongPress={400}
      style={{ paddingVertical: PAD, paddingHorizontal: 16, backgroundColor: C.bgDeep, maxHeight: Math.round(height * MAX_SHARE), overflow: 'hidden', minHeight: lh + PAD * 2 }}>
      <Animated.View style={{ opacity: fade }}>
        {shown.map((text, k) => {
          const li = cur + k;
          const p = isPunch(punch, li);
          const current = k === 0;
          return (
            <View key={`${li}:${text}`} style={{ flexDirection: 'row', alignItems: 'flex-start', gap: 8 }} testID={testID ? `${testID}-line-${k}` : undefined}>
              <View style={{ width: GUTTER, height: lh, alignItems: 'center', justifyContent: 'center' }}>
                {p && <View style={{ width: dot, height: dot, borderRadius: dot / 2, backgroundColor: C.amber, opacity: current ? 1 : 0.6 }} />}
              </View>
              <Animated.Text maxFontSizeMultiplier={PROMPTER_CAP} style={{ flex: 1, minWidth: 0, fontFamily: p ? F.bold : F.semi, fontSize: PROMPTER_SIZE, lineHeight: PROMPTER_LEADING, color: current ? C.text : C.mut2 }}>
                {text}
              </Animated.Text>
            </View>
          );
        })}
      </Animated.View>
    </Pressable>
  );
}
