// iOS/Android: the recorder over expo-audio (useAudioRecorder). The options, the meter maths, the clock and the mic
// choice are pure, in lib/recorderLogic (tested); this file is the wiring:
//   - SET_RECORDING: 48 kHz AAC in an .m4a (api-dev/server.py AUDIO_EXT takes it; engine/audio.py normalises through
//     ffmpeg), two channels, 160 kbps, metering on, written straight to the document directory.
//   - The audio session records in the background (app.json UIBackgroundModes: audio) and does not mix, so a locked
//     phone at the stage edge keeps taping. expo-audio 58 does not expose the AVAudioSession mode: ios/AudioRecorder.swift
//     sets `.playAndRecord, mode: .default` itself (measurement mode, AGC off, would be a config plugin: a follow-up).
//   - The screen stays awake while a tape runs (expo-keep-awake, one tag, released on stop and cancel).
//   - A phone call: expo-audio pauses the recorder and resumes it when the call ends (ios/AudioModule.swift
//     handleInterruptionBegan / resumeInterruptedPlayers). Here the clock notices the gap (interrupted() is true, the
//     elapsed time holds) and his Resume tap restarts it if the system did not. The file is the same file throughout.
//   - A media-services reset (the audio daemon restarted): expo-audio prepares a NEW file; the partial tape is the
//     one at the uri remembered at start, and stop() hands that one back.
//   - A wired or USB mic is preferred over the built-in one; a Bluetooth headset is never the input while another
//     exists (expo-audio opens the session with allowBluetoothHFP, which would tape the room at telephone quality).
import { useCallback, useMemo, useRef } from 'react';
import { AudioModule, setAudioModeAsync, useAudioRecorder } from 'expo-audio';
import type { RecordingStatus } from 'expo-audio';
import { activateKeepAwakeAsync, deactivateKeepAwake } from 'expo-keep-awake';
import type { Rec, Recording } from './recorderBase';
import { KEEP_AWAKE_TAG, SET_RECORDING, clockElapsed, clockObserve, clockPause, clockResume, clockStart, meterLevel, pickInput } from './recorderLogic';
import type { Clock } from './recorderLogic';

export type { Rec, Recording } from './recorderBase';

let modeSet = false;
async function prepareSession() {
  const p = await AudioModule.requestRecordingPermissionsAsync();
  if (!p.granted) throw new Error('Microphone access is off. Allow it in Settings › Punchable.');
  if (!modeSet) {
    // AudioMode fields: docs.expo.dev/versions/v58.0.0/sdk/audio/#audiomode (no category/mode fields exist there)
    await setAudioModeAsync({ allowsRecording: true, allowsBackgroundRecording: true, shouldPlayInBackground: true, playsInSilentMode: true, interruptionMode: 'doNotMix' });
    modeSet = true;
  }
}

const awake = (on: boolean) => (on ? activateKeepAwakeAsync(KEEP_AWAKE_TAG) : deactivateKeepAwake(KEEP_AWAKE_TAG)).catch(() => {});

export function useRecorder(): Rec {
  // The listener is captured once per recorder (expo-audio subscribes with deps [recorder.id]); it writes to a ref.
  const lastStatus = useRef<RecordingStatus | null>(null);
  const onStatus = useCallback((s: RecordingStatus) => { lastStatus.current = s; }, []);
  const recorder = useAudioRecorder(SET_RECORDING, onStatus);
  const st = useRef<'idle' | 'recording' | 'paused' | 'stopped'>('idle');
  const clock = useRef<Clock>(clockStart(0));
  const firstUri = useRef<string | null>(null);

  return useMemo<Rec>(() => {
    // One native getStatus per call; the clock learns whether the system paused or resumed the recorder.
    const observe = () => {
      const s = recorder.getStatus();
      if (st.current === 'recording') clock.current = clockObserve(clock.current, s.isRecording, Date.now());
      return s;
    };
    return {
      supported: true,
      state: () => st.current,
      async start() {
        await prepareSession();
        lastStatus.current = null;
        await recorder.prepareToRecordAsync();
        try {
          // prepared-only API (AudioModule.types.d.ts getAvailableInputs/setInput); a failure here is not a failure to tape
          const want = pickInput(recorder.getAvailableInputs(), await recorder.getCurrentInput().catch(() => null));
          if (want) recorder.setInput(want.uid);
        } catch {}
        recorder.record();
        firstUri.current = recorder.uri;
        clock.current = clockStart(Date.now());
        st.current = 'recording';
        void awake(true);
      },
      pause() {
        if (st.current !== 'recording') return;
        recorder.pause();
        clock.current = clockPause(clock.current, Date.now(), 'user');
        st.current = 'paused';
      },
      resume() {
        if (st.current === 'paused') {
          clock.current = clockResume(clock.current, Date.now());
          recorder.record();
          st.current = 'recording';
        } else if (st.current === 'recording' && clock.current.by === 'system') {
          // his tap after a call the system did not resume: the one place a resume is driven from here
          recorder.record();
          clock.current = clockResume(clock.current, Date.now());
        }
      },
      async stop(): Promise<Recording> {
        const durationMs = clockElapsed(clock.current, Date.now());
        await recorder.stop();
        st.current = 'stopped';
        void awake(false);
        const reset = !!lastStatus.current?.mediaServicesDidReset || !!lastStatus.current?.hasError;
        const uri = (reset && firstUri.current) || recorder.uri || firstUri.current;
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
        void awake(false);
      },
      elapsedMs() {
        if (st.current === 'idle' || st.current === 'stopped') return 0;
        observe();
        return clockElapsed(clock.current, Date.now());
      },
      level() {
        if (st.current !== 'recording') return 0;
        const s = observe();
        return s.isRecording ? meterLevel(s.metering) : 0; // metering: dBFS, averagePower(forChannel: 0)
      },
      interrupted() {
        if (st.current !== 'recording') return false;
        observe();
        return clock.current.by === 'system';
      },
    };
  }, [recorder]);
}
