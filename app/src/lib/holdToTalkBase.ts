// Hold to talk, web: the browser's speech recognition while the button is held (Chrome, Safari). Where there is
// none (Firefox) the button focuses the field instead. The phone build (holdToTalk.native.ts) records the voice
// with expo-audio and sends it up as a voice note that his PC transcribes.
import { useCallback, useEffect, useRef, useState } from 'react';
import { Platform } from 'react-native';

type Rec = { start: () => void; stop: () => void; abort: () => void; onresult: any; onend: any; onerror: any; lang: string; interimResults: boolean; continuous: boolean };
function speechCtor(): (new () => Rec) | null {
  if (Platform.OS !== 'web') return null;
  const g = globalThis as any;
  return g.SpeechRecognition || g.webkitSpeechRecognition || null;
}
export const canListen = () => !!speechCtor();

export type HoldToTalk = {
  /** The button records or listens (else it only focuses the field). */
  supported: boolean;
  /** 'listen' = words come back as text; 'record' = a voice note goes up to be transcribed. */
  mode: 'listen' | 'record' | 'none';
  on: boolean;
  since: number;
  start: () => boolean;
  stop: () => void;
};

/** Press and hold: listens while held; on release the words are handed over (interim words show in the field). */
export function useHoldToTalk(onHeard: (text: string) => void, onInterim?: (text: string) => void, _onNote?: (r: { status: string; queued?: boolean }) => void): HoldToTalk {
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
  const supported = canListen();
  return { supported, mode: supported ? 'listen' : 'none', on, since, start, stop };
}
