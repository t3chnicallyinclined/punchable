// The record control (Stage and Record share it), built to the 2026-10-29 spec (ios-design-expert, HIG buttons /
// feedback / modality / playing-haptics): nothing records until he taps the red disc; while recording the disc is a
// red square that STOPS AND SAVES (Voice Memos: stopping is not loss, so no sheet); Discard is the hard path, behind
// Close. Pause is a secondary control; a phone call shows as "Paused by a call" with the same Resume. 92 pt hit region
// on the centre control, 60 pt on the sides, all above the home indicator: one-handed in the dark.
// The state machine is pure (lib/recorderLogic nextRecordState) and tested in tests/recorder.test.mjs.
import React from 'react';
import { Platform, PlatformColor, Pressable, Text, View } from 'react-native';
import { C, F, fmtMs } from './theme';
import type { RecordState } from './recorderLogic';

const RED = Platform.OS === 'ios' ? (PlatformColor('systemRed') as any) : C.rec;
const RING = 92;
const DISC = 80;
const SQUARE = 30;

/** `busy`: the microphone is being opened (the tap landed; nothing to do but wait). The disc dims and ignores taps
 *  until the recorder answers, so a second tap cannot start a second take over the first. */
export function RecordButton({ state, interrupted, busy, onStart, onStop, onResume, testID = 'rec' }: {
  state: RecordState; interrupted?: boolean; busy?: boolean; onStart: () => void; onStop: () => void; onResume: () => void; testID?: string;
}) {
  const recording = state === 'recording' && !interrupted;
  const paused = state === 'paused' || (state === 'recording' && !!interrupted);
  const saving = state === 'saving';
  const starting = !!busy && state === 'idle';
  const held = saving || starting;
  const label = saving ? 'Saving' : starting ? 'Starting' : recording ? 'Stop and save the recording' : paused ? 'Resume recording' : 'Start recording';
  const press = () => { if (held) return; if (recording) onStop(); else if (paused) onResume(); else onStart(); };
  return (
    <Pressable testID={`${testID}-main`} accessibilityRole="button" accessibilityLabel={label} accessibilityState={{ disabled: held, busy: held }} disabled={held} onPress={press}
      style={({ pressed }) => ({ width: RING, height: RING, borderRadius: RING / 2, borderWidth: 2, borderColor: C.border, alignItems: 'center', justifyContent: 'center', opacity: pressed ? 0.8 : 1 })}>
      {recording ? (
        <View style={{ width: SQUARE, height: SQUARE, borderRadius: 7, backgroundColor: RED }} />
      ) : (
        <View style={{ width: DISC, height: DISC, borderRadius: DISC / 2, backgroundColor: saving ? C.border : RED, opacity: starting ? 0.6 : 1, alignItems: 'center', justifyContent: 'center' }}>
          {paused && <Text maxFontSizeMultiplier={1.2} style={{ fontFamily: F.semi, fontSize: 15, color: '#fff' }}>Resume</Text>}
        </View>
      )}
    </Pressable>
  );
}

/** A side control: 60 pt, plain; `tone` colours the label. */
export function SideButton({ label, onPress, disabled, tone = 'plain', testID, prominent }: { label: string; onPress: () => void; disabled?: boolean; tone?: 'plain' | 'amber'; testID?: string; prominent?: boolean }) {
  return (
    <Pressable testID={testID} accessibilityRole="button" accessibilityLabel={label} accessibilityState={{ disabled: !!disabled }} disabled={disabled} onPress={onPress}
      style={({ pressed }) => ({ flex: 1, minHeight: 60, borderRadius: 16, alignItems: 'center', justifyContent: 'center', paddingHorizontal: 8,
        backgroundColor: prominent ? C.amber : C.card, opacity: disabled ? 0.4 : pressed ? 0.8 : 1 })}>
      <Text numberOfLines={1} maxFontSizeMultiplier={1.3} style={{ fontFamily: F.semi, fontSize: 17, color: prominent ? C.bg : tone === 'amber' ? C.amber : C.text }}>{label}</Text>
    </Pressable>
  );
}

/** The dock: [left] [record] [right]. The centre never moves, so his thumb learns one place. */
export function RecordDock({ left, right, children }: { left: React.ReactNode; right: React.ReactNode; children: React.ReactNode }) {
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 12 }}>
      <View style={{ flex: 1 }}>{left}</View>
      {children}
      <View style={{ flex: 1 }}>{right}</View>
    </View>
  );
}

/** The one clock: 84 pt tabular, the status word, the target bar with the light, the room's level while taping. */
export function RecordClock({ ms, status, targetS, lightS, level, interrupted, testID = 'rec' }: {
  ms: number; status: string; targetS?: number; lightS?: number; level?: number | null; interrupted?: boolean; testID?: string;
}) {
  const target = targetS || 0;
  const light = lightS ?? Math.max(0, target - 60);
  const lit = target > 0 && ms >= light * 1000;
  const pct = target ? Math.min(100, (ms / (target * 1000)) * 100) : 0;
  return (
    <View style={{ gap: 10 }}>
      <View style={{ alignItems: 'center', gap: 2 }}>
        <Text testID={`${testID}-clock`} maxFontSizeMultiplier={1.2} style={{ fontFamily: F.black, fontSize: 84, lineHeight: 90, letterSpacing: -2, color: lit ? C.gapText : C.text, fontVariant: ['tabular-nums'] }}>{fmtMs(ms)}</Text>
        <Text testID={`${testID}-status`} maxFontSizeMultiplier={1.4} style={{ fontFamily: F.semi, fontSize: 15, color: status === 'Recording' ? RED : C.mut2 }}>
          {status}{target ? ` · of ${fmtMs(target * 1000)} · light at ${fmtMs(light * 1000)}` : ''}
        </Text>
      </View>
      {!!target && (
        <View style={{ height: 8, borderRadius: 4, backgroundColor: C.card, overflow: 'hidden' }}>
          <View style={{ width: `${pct}%`, height: '100%', backgroundColor: lit ? C.rec : C.amber }} />
          <View style={{ position: 'absolute', left: `${(light / target) * 100}%`, top: 0, bottom: 0, width: 2, backgroundColor: C.text }} />
        </View>
      )}
      {level != null && (
        <View testID={`${testID}-level`} accessibilityElementsHidden importantForAccessibility="no-hide-descendants" style={{ height: 6, borderRadius: 3, backgroundColor: C.card, overflow: 'hidden' }}>
          <View style={{ width: `${Math.round(Math.max(0, Math.min(1, level)) * 100)}%`, height: '100%', backgroundColor: interrupted ? C.tabOff : level > 0.55 ? C.amber : C.border }} />
        </View>
      )}
    </View>
  );
}
