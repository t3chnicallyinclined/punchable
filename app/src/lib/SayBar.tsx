// The single input bar (MixHome / MixAsk): "Jot it or ask it" + the big red hold-to-talk button. One input on
// every screen; the app decides whether it was a bit or a question (lib/say.ts, POST /api/say).
// Hold to talk: the browser's speech recognition while the button is held (Chrome, Safari); where there is none
// (Firefox, the phone build until expo-speech-recognition is added) the button focuses the text field instead.
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Platform, Pressable, Text, TextInput, View } from 'react-native';
import Svg, { Path, Rect } from 'react-native-svg';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { C, F } from './theme';
import type { Worker } from './ask';

type Rec = { start: () => void; stop: () => void; abort: () => void; onresult: any; onend: any; onerror: any; lang: string; interimResults: boolean; continuous: boolean };
function speechCtor(): (new () => Rec) | null {
  if (Platform.OS !== 'web') return null;
  const g = globalThis as any;
  return g.SpeechRecognition || g.webkitSpeechRecognition || null;
}
export const canListen = () => !!speechCtor();

/** Press and hold: listens while held; on release the words are handed over (interim words show in the field). */
export function useHoldToTalk(onHeard: (text: string) => void, onInterim?: (text: string) => void) {
  const rec = useRef<Rec | null>(null);
  const heard = useRef('');
  const [on, setOn] = useState(false);
  const [since, setSince] = useState(0);
  const start = useCallback(() => {
    const Ctor = speechCtor();
    if (!Ctor || rec.current) return false;
    const r = new Ctor();
    r.lang = 'en-US';
    r.interimResults = true;
    r.continuous = true;
    heard.current = '';
    r.onresult = (e: any) => {
      let final = '';
      let interim = '';
      for (let i = 0; i < (e.results?.length || 0); i++) {
        const res = e.results[i];
        const t = res[0]?.transcript || '';
        if (res.isFinal) final += t;
        else interim += t;
      }
      heard.current = (final || interim).trim();
      onInterim?.((final + ' ' + interim).trim());
    };
    r.onend = () => {
      rec.current = null;
      setOn(false);
      const t = heard.current.trim();
      heard.current = '';
      if (t) onHeard(t);
    };
    r.onerror = () => { rec.current = null; setOn(false); };
    rec.current = r;
    setOn(true);
    setSince(Date.now());
    try { r.start(); } catch { rec.current = null; setOn(false); return false; }
    return true;
  }, [onHeard, onInterim]);
  const stop = useCallback(() => { try { rec.current?.stop(); } catch {} }, []);
  useEffect(() => () => { try { rec.current?.abort(); } catch {} }, []);
  return { supported: canListen(), on, since, start, stop };
}

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
  const heard = useCallback((t: string) => { setInterim(''); setText(''); onSay(t, 'spoken'); }, [onSay]);
  const talk = useHoldToTalk(heard, setInterim);
  const send = () => {
    const t = text.trim();
    if (!t || busy) return;
    setText('');
    onSay(t, 'typed');
  };
  const line = worker && !worker.online ? 'Your PC is off: questions wait and get answered when it’s back. Bits save now.'
    : hint === undefined ? 'Say a bit or ask a question. It knows which.' : hint;
  return (
    <View testID={`${testID}-bar`} style={{ paddingHorizontal: pad ? 16 : 0, paddingTop: 8, paddingBottom: pad ? 8 + ins.bottom * 0 : 0, gap: 10, alignItems: 'center' }}>
      {!!line && <Text testID={`${testID}-hint`} maxFontSizeMultiplier={1.4} style={{ fontSize: 13, fontFamily: F.body, color: worker && !worker.online ? C.amber : C.mut2, textAlign: 'center' }}>{line}</Text>}
      <View style={{ width: '100%', maxWidth: 720, flexDirection: 'row', alignItems: 'center', gap: 10 }}>
        <View style={{ flex: 1, minWidth: 0, height: 52, borderRadius: 26, backgroundColor: C.card, paddingHorizontal: 16, flexDirection: 'row', alignItems: 'center' }}>
          <TextInput ref={input} testID={`${testID}-input`} value={talk.on && interim ? interim : text} onChangeText={setText} onSubmitEditing={send} returnKeyType="send"
            blurOnSubmit={false} autoFocus={autoFocus} editable={!talk.on} placeholder={talk.on ? 'Listening…' : placeholder} placeholderTextColor={talk.on ? C.amber : C.dim}
            accessibilityLabel={placeholder} maxLength={4000}
            style={{ flex: 1, minWidth: 0, height: 52, color: C.text, fontFamily: F.body, fontSize: 16 }} />
          {!!text.trim() && !talk.on && (
            <Pressable testID={`${testID}-send`} accessibilityRole="button" accessibilityLabel="Send" onPress={send} disabled={busy}
              style={{ height: 36, paddingHorizontal: 12, borderRadius: 18, backgroundColor: C.amber, alignItems: 'center', justifyContent: 'center', marginRight: -8 }}>
              <Text style={{ fontFamily: F.semi, fontSize: 14, color: C.bg }}>Go</Text>
            </Pressable>
          )}
        </View>
        <Pressable testID={`${testID}-talk`} accessibilityRole="button" accessibilityLabel={talk.supported ? (talk.on ? 'Listening; let go to send' : 'Hold to talk') : 'Type it (this device can’t listen)'}
          onPressIn={() => { if (!talk.supported) { input.current?.focus(); return; } talk.start(); }}
          onPressOut={() => talk.supported && talk.stop()}
          style={{ width: 72, height: 72, borderRadius: 36, borderWidth: 4, borderColor: talk.on ? C.amber : '#3A3A3F', backgroundColor: C.bg, alignItems: 'center', justifyContent: 'center' }}>
          <View style={{ width: 54, height: 54, borderRadius: 27, backgroundColor: talk.supported ? C.rec : C.card, alignItems: 'center', justifyContent: 'center', transform: [{ scale: talk.on ? 0.86 : 1 }] }}>
            <MicGlyph color={talk.supported ? '#fff' : C.mut2} />
          </View>
        </Pressable>
      </View>
    </View>
  );
}
