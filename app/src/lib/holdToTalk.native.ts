// Hold to talk, iOS/Android: no on-device speech-to-text yet, so the red button records the voice (expo-audio,
// lib/recorder.native) while held and, on release, sends it up as a voice note (POST /api/upload, kind note) through
// the retry queue. His PC transcribes it and files it like any memo (it lands in the inbox). Under 0.7 s = a tap:
// the field gets focus instead, nothing is sent.
import { useCallback, useRef, useState } from 'react';
import { haptics } from './haptics';
import { useRecorder } from './recorder';
import { enqueueUpload } from './uploads';
import type { HoldToTalk } from './holdToTalk';

export type { HoldToTalk } from './holdToTalk';
export const canListen = () => false;
const MIN_MS = 700;

export function useHoldToTalk(_onHeard: (text: string) => void, _onInterim?: (text: string) => void, onNote?: (r: { status: string; queued?: boolean; error?: string }) => void): HoldToTalk {
  const rec = useRecorder();
  const [on, setOn] = useState(false);
  const [since, setSince] = useState(0);
  const busy = useRef(false);
  const start = useCallback(() => {
    if (busy.current || on) return false;
    busy.current = true;
    setSince(Date.now());
    rec.start().then(() => {
      setOn(true);
      haptics.light();
    }).catch((e) => {
      onNote?.({ status: 'error', error: e?.message || 'Microphone not available' });
    }).finally(() => {
      busy.current = false;
    });
    return true;
  }, [on, rec, onNote]);
  const stop = useCallback(() => {
    if (rec.state() !== 'recording' && rec.state() !== 'paused') return;
    setOn(false);
    haptics.light();
    const short = rec.elapsedMs() < MIN_MS;
    rec.stop().then(async (r) => {
      if (short) return; // a tap, not a note
      const res = await enqueueUpload(r, { kind: 'note', filename: r.name, title: `Voice note ${new Date().toLocaleString()}`, durationS: r.durationMs / 1000, recordedAt: new Date().toISOString() });
      onNote?.(res);
    }).catch((e) => onNote?.({ status: 'error', error: e?.message || 'Could not save the note' }));
  }, [rec, onNote]);
  return { supported: true, mode: 'record', on, since, start, stop };
}
