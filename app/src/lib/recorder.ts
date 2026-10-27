// Web: the recorder hook over the MediaRecorder recorder in lib/media. The phone build uses recorder.native.ts
// (expo-audio). Both answer the same shape, so the record screen, the stage and hold-to-talk don't know which.
import { useMemo, useRef } from 'react';
import { createRecorder, micSupported } from './media';
import type { Recorder } from './media';

/** What a stopped recording is: a Blob (web) or a file on the phone (native). */
export type Recording = { blob?: Blob; uri?: string; name: string; type: string; durationMs: number };
export type Rec = {
  supported: boolean;
  state: () => 'idle' | 'recording' | 'paused' | 'stopped';
  start: () => Promise<void>;
  pause: () => void;
  resume: () => void;
  stop: () => Promise<Recording>;
  cancel: () => void;
  elapsedMs: () => number;
  level: () => number;
};

export function useRecorder(): Rec {
  const inner = useRef<Recorder | null>(null);
  return useMemo<Rec>(() => ({
    supported: micSupported(),
    state: () => inner.current?.state() || 'idle',
    async start() {
      const r = createRecorder();
      inner.current = r;
      await r.start();
    },
    pause: () => inner.current?.pause(),
    resume: () => inner.current?.resume(),
    async stop() {
      const r = inner.current;
      if (!r) throw new Error('Not recording');
      const durationMs = r.elapsedMs();
      const blob = await r.stop();
      const ext = blob.type.includes('mp4') ? 'm4a' : blob.type.includes('ogg') ? 'ogg' : 'webm';
      return { blob, name: `recording.${ext}`, type: blob.type || 'audio/webm', durationMs };
    },
    cancel: () => inner.current?.cancel(),
    elapsedMs: () => inner.current?.elapsedMs() || 0,
    level: () => inner.current?.level() || 0,
  }), []);
}
