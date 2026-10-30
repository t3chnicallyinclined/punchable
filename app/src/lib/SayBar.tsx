// The single input bar (MixHome / MixAsk): "Jot it or ask it" + the big red hold-to-talk button. One input on
// every screen; the app decides whether it was a bit, a question or a command (lib/say.ts, POST /api/say).
// Hold to talk: web = the browser's speech recognition while the button is held (Chrome, Safari; Firefox focuses
// the field); phone = the button focuses the field (jots are typed; voice is for recording sets only). lib/holdToTalk.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useFocusEffect } from 'expo-router';
import { Platform, Pressable, Text, TextInput, View } from 'react-native';
import Svg, { Path, Rect } from 'react-native-svg';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { C, F } from './theme';
import { haptics } from './haptics';
import { holdHaptic } from './feedback';
import { useHoldToTalk } from './holdToTalk';
import { BarConfig, Icon, useUI } from './ui';
import { ACCESSORY } from './accessory';
import { useFlags } from './useFlags';
import type { Worker } from './ask';

export { canListen, useHoldToTalk } from './holdToTalk';

function MicGlyph({ size = 22, color = '#fff' }: { size?: number; color?: string }) {
  if (Platform.OS !== 'web') return <Icon name="mic" size={size} color={color} sw={2} />;
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={2} strokeLinecap="round">
      <Rect x={9} y={3} width={6} height={11} rx={3} />
      <Path d="M5 11a7 7 0 0014 0M12 18v3" />
    </Svg>
  );
}

/**
 * The bar. `onSay(text)` gets the typed or spoken words; the screen decides what to do (usually lib/say.sayOrQueue
 * and a hop to /say). `worker` (when the screen has it) shows the honest line when the writer is offline or paused.
 */
export type SayBarProps = {
  onSay: (text: string, how: 'typed' | 'spoken') => void; placeholder?: string; hint?: string | null; worker?: Worker | null;
  testID?: string; autoFocus?: boolean; busy?: boolean; pad?: boolean;
  /** `dock` (default): the full bar pinned above the tab bar. `accessory`: inside the tab bar's bottom accessory
   *  (iOS 26; the system draws the glass, so nothing here does). `inline`: the accessory beside a minimised tab bar:
   *  the field only, no hint, no record ring. */
  variant?: 'dock' | 'accessory' | 'inline';
};

/** The sizes per variant: the field's height, the ring's outer and inner diameters (0 = no ring), the hint line. */
export const BAR_SIZES = {
  dock: { field: 52, ring: 72, inner: 54, hint: true, padX: 16, padY: 8, gap: 10 },
  accessory: { field: 44, ring: 48, inner: 36, hint: false, padX: 8, padY: 4, gap: 8 },
  inline: { field: 40, ring: 0, inner: 0, hint: false, padX: 6, padY: 2, gap: 6 },
} as const;

/**
 * A screen's input bar. Returns the element for Screen's `bar` prop, or null on iOS 26 where the tab bar's bottom
 * accessory draws it (lib/TabsShell.native): the screen publishes its config while it is in front, and keeps it
 * current when the placeholder, the hint or the writer's status change.
 */
export function useSayBar(cfg: SayBarProps): React.ReactNode {
  const ui = useUI();
  // the accessory needs the system's glass (ACCESSORY) AND the remote switch (rules/flags: /api/health flags.glass_accessory)
  const accessory = ACCESSORY && useFlags().glassAccessory;
  const ref = useRef(cfg);
  ref.current = cfg;
  const focused = useRef(false);
  const publish = useCallback(() => {
    const c = ref.current;
    const b: BarConfig = { onSay: (t, how) => ref.current.onSay(t, how), placeholder: c.placeholder, hint: c.hint, worker: c.worker, testID: c.testID, autoFocus: c.autoFocus, busy: c.busy };
    ui.setBar(b);
  }, [ui.setBar]); // eslint-disable-line react-hooks/exhaustive-deps
  useFocusEffect(useCallback(() => {
    if (!accessory) return;
    focused.current = true;
    publish();
    return () => { focused.current = false; ui.setBar(null); };
  }, [publish, accessory])); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (accessory && focused.current) publish();
  }, [cfg.placeholder, cfg.hint, cfg.worker?.online, cfg.worker?.paused, cfg.busy, publish, accessory]);
  if (accessory) return null;
  return <SayBar {...cfg} />;
}

export function SayBar({ onSay, placeholder = 'Jot it or ask it', hint, worker, testID = 'say', autoFocus, busy, pad = true, variant = 'dock' }: SayBarProps) {
  const Z = BAR_SIZES[variant];
  const [text, setText] = useState('');
  const [interim, setInterim] = useState('');
  const input = useRef<any>(null);
  const ins = useSafeAreaInsets();
  const ui = useUI();
  const heard = useCallback((t: string) => { setInterim(''); setText(''); onSay(t, 'spoken'); }, [onSay]);
  const noted = useCallback((r: { status: string; queued?: boolean; error?: string }) => {
    if (r.status === 'error') return ui.snack(r.error || 'Could not save the voice note');
    if (r.queued) return ui.snack('No connection: the voice note is kept on this phone and goes up when Punchable is back');
    ui.snack('Voice note sent · it waits for a transcript (on your phone for now), then lands in the inbox');
  }, [ui]);
  const talk = useHoldToTalk(heard, setInterim, noted);
  const send = () => {
    const t = text.trim();
    if (!t || busy) return;
    setText('');
    onSay(t, 'typed');
  };
  const line = worker && worker.online && worker.paused ? `AI paused for today: ${worker.paused}. Bits save now.` : worker && !worker.online ? 'The writer is offline: questions wait and get answered when it’s back. Bits save now.'
    : hint === undefined ? 'Say a bit or ask a question. It knows which.' : hint;
  const listening = talk.on && talk.mode === 'listen';
  const recording = talk.on && talk.mode === 'record';
  const talkLabel = !talk.supported ? 'Type it (this device can’t listen)' : talk.mode === 'record'
    ? (recording ? 'Recording; let go to send the voice note' : 'Hold to record a voice note')
    : (listening ? 'Listening; let go to send' : 'Hold to talk');
  return (
    <View testID={`${testID}-bar`} style={{ paddingHorizontal: pad ? Z.padX : 0, paddingTop: Z.padY, paddingBottom: pad ? Z.padY + ins.bottom * 0 : 0, gap: Z.gap, alignItems: 'center' }}>
      {!!line && Z.hint && <Text testID={`${testID}-hint`} maxFontSizeMultiplier={1.4} style={{ fontSize: 13, fontFamily: F.body, color: worker && !worker.online ? C.amber : C.mut2, textAlign: 'center' }}>{line}</Text>}
      <View style={{ width: '100%', maxWidth: 720, flexDirection: 'row', alignItems: 'center', gap: Z.gap }}>
        <View style={{ flex: 1, minWidth: 0, minHeight: Z.field, borderRadius: Z.field / 2, backgroundColor: C.card, paddingHorizontal: 16, flexDirection: 'row', alignItems: 'center' }}>
          <TextInput ref={input} testID={`${testID}-input`} value={listening && interim ? interim : text} onChangeText={setText} onSubmitEditing={send} returnKeyType="send"
            blurOnSubmit={false} autoFocus={autoFocus} editable={!talk.on} placeholder={listening ? 'Listening…' : recording ? 'Recording… let go to send' : placeholder} placeholderTextColor={talk.on ? C.amber : C.dim}
            accessibilityLabel={placeholder} maxLength={4000} maxFontSizeMultiplier={1.4}
            style={{ flex: 1, minWidth: 0, minHeight: Z.field, color: C.text, fontFamily: F.body, fontSize: 16 }} />
          {!!text.trim() && !talk.on && (
            <Pressable testID={`${testID}-send`} accessibilityRole="button" accessibilityLabel="Send" onPress={send} disabled={busy} hitSlop={6}
              style={{ minHeight: 44, paddingHorizontal: 14, borderRadius: 22, backgroundColor: C.amber, alignItems: 'center', justifyContent: 'center', marginRight: -10 }}>
              <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.semi, fontSize: 14, color: C.bg }}>Go</Text>
            </Pressable>
          )}
        </View>
        {Z.ring > 0 && <Pressable testID={`${testID}-talk`} accessibilityRole="button" accessibilityLabel={talkLabel}
          onPressIn={() => { if (!talk.supported) { input.current?.focus(); return; } const h = holdHaptic('in'); if (h) haptics[h](); talk.start(); }}
          onPressOut={() => { if (talk.supported) { talk.stop(); const h = holdHaptic('out'); if (h) haptics[h](); } }}
          style={{ width: Z.ring, height: Z.ring, borderRadius: Z.ring / 2, borderWidth: variant === 'dock' ? 4 : 3, borderColor: talk.on ? C.amber : C.borderStrong, backgroundColor: C.bg, alignItems: 'center', justifyContent: 'center' }}>
          <View style={{ width: Z.inner, height: Z.inner, borderRadius: Z.inner / 2, backgroundColor: talk.supported ? C.rec : C.card, alignItems: 'center', justifyContent: 'center', transform: [{ scale: talk.on ? 0.86 : 1 }] }}>
            {recording ? <Text accessibilityLabel="Recording" style={{ fontFamily: F.g7, fontSize: variant === 'dock' ? 11 : 9, color: '#fff', letterSpacing: 0.6 }}>● REC</Text> : <MicGlyph size={variant === 'dock' ? 22 : 18} color={talk.supported ? '#fff' : C.mut2} />}
          </View>
        </Pressable>}
      </View>
    </View>
  );
}
