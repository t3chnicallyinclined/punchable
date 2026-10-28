// iOS/Android: the recorder over expo-audio (useAudioRecorder, HIGH_QUALITY = AAC in an .m4a, which the pipeline
// takes: api-dev/server.py AUDIO_EXT and engine/config.toml audio_ext both list .m4a; engine/audio.py normalises
// through ffmpeg). The audio session allows recording in the background (app.json UIBackgroundModes: audio) and
// does not mix with other audio, so a locked phone at the stage edge keeps taping. The file is moved from the
// cache into the document directory (safe from the OS) before it is handed to the upload queue.
import { useMemo, useRef } from 'react';
import { AudioModule, RecordingPresets, setAudioModeAsync, useAudioRecorder } from 'expo-audio';
import type { Rec, Recording } from './recorderBase';

export type { Rec, Recording } from './recorderBase';

let modeSet = false;
async function prepareSession() {
  const p = await AudioModule.requestRecordingPermissionsAsync();
  if (!p.granted) throw new Error('Microphone access is off. Allow it in Settings › Punchable.');
  if (!modeSet) {
    await setAudioModeAsync({ allowsRecording: true, allowsBackgroundRecording: true, shouldPlayInBackground: true, playsInSilentMode: true, interruptionMode: 'doNotMix' });
    modeSet = true;
  }
}

export function useRecorder(): Rec {
  const recorder = useAudioRecorder({ ...RecordingPresets.HIGH_QUALITY, isMeteringEnabled: true });
  const st = useRef<'idle' | 'recording' | 'paused' | 'stopped'>('idle');
  const startedAt = useRef(0);
  const pausedTotal = useRef(0);
  const pausedAt = useRef(0);
  return useMemo<Rec>(() => ({
    supported: true,
    state: () => st.current,
    async start() {
      await prepareSession();
      await recorder.prepareToRecordAsync();
      recorder.record();
      startedAt.current = Date.now();
      pausedTotal.current = 0;
      st.current = 'recording';
    },
    pause() {
      if (st.current !== 'recording') return;
      recorder.pause();
      pausedAt.current = Date.now();
      st.current = 'paused';
    },
    resume() {
      if (st.current !== 'paused') return;
      pausedTotal.current += Date.now() - pausedAt.current;
      recorder.record();
      st.current = 'recording';
    },
    async stop(): Promise<Recording> {
      if (st.current === 'paused') pausedTotal.current += Date.now() - pausedAt.current;
      const durationMs = Math.max(0, Date.now() - startedAt.current - pausedTotal.current);
      await recorder.stop();
      st.current = 'stopped';
      const uri = recorder.uri;
      if (!uri) throw new Error('The recording was not saved');
      const FS: any = await import('expo-file-system/legacy');
      const dir = `${FS.documentDirectory}recordings/`;
      await FS.makeDirectoryAsync(dir, { intermediates: true }).catch(() => {});
      const ext = (uri.split('.').pop() || 'm4a').toLowerCase();
      const name = `${new Date().toISOString().replace(/[:.]/g, '-')}.${ext}`;
      const dest = dir + name;
      await FS.moveAsync({ from: uri, to: dest });
      return { uri: dest, name, type: ext === 'm4a' ? 'audio/mp4' : ext === 'caf' ? 'audio/x-caf' : 'audio/mp4', durationMs };
    },
    cancel() {
      if (st.current === 'recording' || st.current === 'paused') recorder.stop().catch(() => {});
      st.current = 'stopped';
    },
    elapsedMs() {
      if (st.current === 'idle' || st.current === 'stopped') return 0;
      const now = st.current === 'paused' ? pausedAt.current : Date.now();
      return Math.max(0, now - startedAt.current - pausedTotal.current);
    },
    level() {
      if (st.current !== 'recording') return 0;
      const m = recorder.getStatus().metering; // dBFS, about -160..0
      return typeof m === 'number' ? Math.max(0, Math.min(1, (m + 50) / 50)) : 0;
    },
  }), [recorder]);
}
