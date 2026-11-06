// The Versions row open (SPEC 2026-11-03 §3.5, D5): a stepper, not a strip. Two 44-pt arrows (older · newer; hold to
// repeat, a selection haptic per step, none under Reduce Motion), the label "v5 of 8 · Oct 20", a second line with
// the version's source doc and run time; `adjustable` for VoiceOver with increment (newer) / decrement (older).
// Under it, while an old version is read: "Restore v5 as a new version". The haptic plays under Reduce Motion too
// (HIG/motion: "supplement visual feedback by also using alternatives like haptics"; haptics have their own switch).
import React, { useEffect, useRef } from 'react';
import { Pressable, Text, View } from 'react-native';
import type { BitVersion } from '@/rules/bitLines';
import { ROW_CAP, restoreLabel, stepperLabel, stepperSub, stepperValue } from '@/rules/bitLines';
import { haptics } from './haptics';
import { C, F, linkColor } from './theme';
import { Icon } from './ui';

const HOLD_MS = 450;
const REPEAT_MS = 170;

function Arrow({ label, name, disabled, onStep, testID }: { label: string; name: 'back' | 'chevron'; disabled: boolean; onStep: () => void; testID?: string }) {
  const timer = useRef<any>(null);
  const ticker = useRef<any>(null);
  // the repeat reads the latest step function: the one captured at press time would compare against a stale `vi`
  const latest = useRef(onStep);
  useEffect(() => { latest.current = onStep; });
  const stop = () => { clearTimeout(timer.current); clearInterval(ticker.current); timer.current = null; ticker.current = null; };
  useEffect(() => stop, []);
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={label} disabled={disabled}
      onPressIn={() => { latest.current(); timer.current = setTimeout(() => { ticker.current = setInterval(() => latest.current(), REPEAT_MS); }, HOLD_MS); }}
      onPressOut={stop}
      style={({ pressed }) => ({ width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center', backgroundColor: pressed ? C.card2 : C.card, opacity: disabled ? 0.35 : 1 })}>
      <Icon name={name} size={18} color={linkColor()} sw={2.4} />
    </Pressable>
  );
}

export function VersionStepper({ versions, vi, reading, onStep, onRestore, durationS, estimated, testID }: {
  versions: BitVersion[]; vi: number;
  /** an older version of its cut, so Restore is offered (Film it §2.6: the newest rant cut is not "old"); default vi > 0 */
  reading?: boolean;
  onStep: (i: number) => void; onRestore: () => void; durationS?: number | null; estimated?: boolean; testID?: string;
}) {
  const n = versions.length;
  const v = versions[vi];
  const step = (to: number) => {
    if (to < 0 || to >= n || to === vi) return;
    haptics.selection();
    onStep(to);
  };
  const older = () => step(vi + 1);
  const newer = () => step(vi - 1);
  const sub = stepperSub(v?.doc, vi === 0 ? durationS : null, estimated);
  return (
    <View testID={testID} style={{ paddingVertical: 4, borderBottomWidth: 1, borderBottomColor: C.hairline }}>
      <View accessible accessibilityRole="adjustable" accessibilityLabel="Version" accessibilityValue={{ text: stepperValue(vi, n, v?.date, v) +(sub ? `, ${sub}` : '') }}
        accessibilityActions={[{ name: 'increment', label: 'Newer' }, { name: 'decrement', label: 'Older' }]}
        onAccessibilityAction={(e) => (e.nativeEvent.actionName === 'increment' ? newer() : older())}
        style={{ flexDirection: 'row', alignItems: 'center', gap: 4, minHeight: 52 }}>
        <Arrow testID={testID ? `${testID}-older` : undefined} label="Older" name="back" disabled={vi >= n - 1} onStep={older} />
        <View style={{ flex: 1, minWidth: 0, alignItems: 'center', gap: 2 }}>
          <Text maxFontSizeMultiplier={ROW_CAP} numberOfLines={2} style={{ fontFamily: F.semi, fontSize: 15, lineHeight: 20, color: C.text, textAlign: 'center' }}>{stepperLabel(vi, n, v?.date, v)}</Text>
          {!!sub && <Text maxFontSizeMultiplier={ROW_CAP} numberOfLines={2} style={{ fontFamily: F.body, fontSize: 13, lineHeight: 18, color: C.mut2, textAlign: 'center' }}>{sub}</Text>}
        </View>
        <Arrow testID={testID ? `${testID}-newer` : undefined} label="Newer" name="chevron" disabled={vi <= 0} onStep={newer} />
      </View>
      {(reading ?? vi > 0) && (
        <Pressable testID={testID ? `${testID}-restore` : undefined} accessibilityRole="button" onPress={onRestore}
          style={({ pressed }) => ({ minHeight: 44, justifyContent: 'center', opacity: pressed ? 0.7 : 1 })}>
          <Text maxFontSizeMultiplier={ROW_CAP} style={{ fontFamily: F.semi, fontSize: 17, lineHeight: 22, color: linkColor() }}>{restoreLabel(vi, n)}</Text>
        </Pressable>
      )}
    </View>
  );
}
