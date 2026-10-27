// The single input bar (MixHome / MixAsk): "Jot it or ask it" + the big red hold-to-talk button. One input on
// every screen; the app decides whether it was a bit, a question or a command (lib/say.ts, POST /api/say).
// Hold to talk: web = the browser's speech recognition while the button is held (Chrome, Safari; Firefox focuses
// the field); phone = expo-audio records while held and the voice note goes up to be transcribed (lib/holdToTalk).
import React, { useCallback, useRef, useState } from 'react';
import { Pressable, Text, TextInput, View } from 'react-native';
import Svg, { Path, Rect } from 'react-native-svg';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { C, F } from './theme';
import { haptics } from './haptics';
import { useHoldToTalk } from './holdToTalk';
import { useUI } from './ui';
import type { Worker } from './ask';

export { canListen, useHoldToTalk } from './holdToTalk';

function MicGlyph({ size = 22, color = '#fff' }: { size?: number; color?: string }) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth={2} strokeLinecap="round">
      <Rect x={9} y={3} width={6} height={11} rx={3} />
      <Path d="M5 11a7 7 0 0014 0M12 18v3" />
    </Svg>
  );
}

/**
 * The bar. `onSay(text)` gets the typed or spoken words; the screen decides what to do (usually lib/say.sayOrQueue
 * and a hop to /say). `worker` (when the screen has it) shows the honest line when his PC is off.
 */
export function SayBar({ onSay, placeholder = 'Jot it or ask it', hint, worker, testID = 'say', autoFocus, busy, pad = true }: {
  onSay: (text: string, how: 'typed' | 'spoken') => void; placeholder?: string; hint?: string | null; worker?: Worker | null;
  testID?: string; autoFocus?: boolean; busy?: boolean; pad?: boolean;
}) {
  const [text, setText] = useState('');
  const [interim, setInterim] = useState('');
  const input = useRef<any>(null);
  const ins = useSafeAreaInsets();
  const ui = useUI();
  const heard = useCallback((t: string) => { setInterim(''); setText(''); onSay(t, 'spoken'); }, [onSay]);
  const noted = useCallback((r: { status: string; queued?: boolean; error?: string }) => {
    if (r.status === 'error') return ui.snack(r.error || 'Could not save the voice note');
    if (r.queued) return ui.snack('No connection: the voice note is kept on this phone and goes up when Punchable is back');
    ui.snack('Voice note sent · your PC transcribes it and it lands in the inbox');
  }, [ui]);
  const talk = useHoldToTalk(heard, setInterim, noted);
  const send = () => {
    const t = text.trim();
    if (!t || busy) return;
    setText('');
    onSay(t, 'typed');
  };
  const line = worker && !worker.online ? 'Your PC is off: questions wait and get answered when it’s back. Bits save now.'
    : hint === undefined ? 'Say a bit or ask a question. It knows which.' : hint;
  const listening = talk.on && talk.mode === 'listen';
  const recording = talk.on && talk.mode === 'record';
  const talkLabel = !talk.supported ? 'Type it (this device can’t listen)' : talk.mode === 'record'
    ? (recording ? 'Recording; let go to send the voice note' : 'Hold to record a voice note')
    : (listening ? 'Listening; let go to send' : 'Hold to talk');
  return (
    <View testID={`${testID}-bar`} style={{ paddingHorizontal: pad ? 16 : 0, paddingTop: 8, paddingBottom: pad ? 8 + ins.bottom * 0 : 0, gap: 10, alignItems: 'center' }}>
      {!!line && <Text testID={`${testID}-hint`} maxFontSizeMultiplier={1.4} style={{ fontSize: 13, fontFamily: F.body, color: worker && !worker.online ? C.amber : C.mut2, textAlign: 'center' }}>{line}</Text>}
      <View style={{ width: '100%', maxWidth: 720, flexDirection: 'row', alignItems: 'center', gap: 10 }}>
        <View style={{ flex: 1, minWidth: 0, minHeight: 52, borderRadius: 26, backgroundColor: C.card, paddingHorizontal: 16, flexDirection: 'row', alignItems: 'center' }}>
          <TextInput ref={input} testID={`${testID}-input`} value={listening && interim ? interim : text} onChangeText={setText} onSubmitEditing={send} returnKeyType="send"
            blurOnSubmit={false} autoFocus={autoFocus} editable={!talk.on} placeholder={listening ? 'Listening…' : recording ? 'Recording… let go to send' : placeholder} placeholderTextColor={talk.on ? C.amber : C.dim}
            accessibilityLabel={placeholder} maxLength={4000} maxFontSizeMultiplier={1.4}
            style={{ flex: 1, minWidth: 0, minHeight: 52, color: C.text, fontFamily: F.body, fontSize: 16 }} />
          {!!text.trim() && !talk.on && (
            <Pressable testID={`${testID}-send`} accessibilityRole="button" accessibilityLabel="Send" onPress={send} disabled={busy} hitSlop={6}
              style={{ minHeight: 44, paddingHorizontal: 14, borderRadius: 22, backgroundColor: C.amber, alignItems: 'center', justifyContent: 'center', marginRight: -10 }}>
              <Text maxFontSizeMultiplier={1.3} style={{ fontFamily: F.semi, fontSize: 14, color: C.bg }}>Go</Text>
            </Pressable>
          )}
        </View>
        <Pressable testID={`${testID}-talk`} accessibilityRole="button" accessibilityLabel={talkLabel}
          onPressIn={() => { if (!talk.supported) { input.current?.focus(); return; } haptics.light(); talk.start(); }}
          onPressOut={() => { if (talk.supported) { talk.stop(); haptics.light(); } }}
          style={{ width: 72, height: 72, borderRadius: 36, borderWidth: 4, borderColor: talk.on ? C.amber : C.borderStrong, backgroundColor: C.bg, alignItems: 'center', justifyContent: 'center' }}>
          <View style={{ width: 54, height: 54, borderRadius: 27, backgroundColor: talk.supported ? C.rec : C.card, alignItems: 'center', justifyContent: 'center', transform: [{ scale: talk.on ? 0.86 : 1 }] }}>
            {recording ? <Text accessibilityLabel="Recording" style={{ fontFamily: F.g7, fontSize: 11, color: '#fff', letterSpacing: 0.6 }}>● REC</Text> : <MicGlyph color={talk.supported ? '#fff' : C.mut2} />}
          </View>
        </Pressable>
      </View>
    </View>
  );
}
