// The pure part of taping a set on the phone (no React, no native modules; tests/recorder.test.mjs runs it under node).
// recorder.native.ts wires it to expo-audio. Three things live here:
//   1. SET_RECORDING: the expo-audio RecordingOptions for a comedy room (48 kHz AAC in an .m4a, two channels when
//      the mic has them, 160 kbps: speech plus laughs, not music). The option names are expo-audio 58's
//      (docs.expo.dev/versions/v58.0.0/sdk/audio/#recordingoptions; node_modules/expo-audio/build/Audio.types.d.ts
//      RecordingOptions / RecordingOptionsIos). `outputFormat: 'aac '` is IOSOutputFormat.MPEG4AAC and
//      `audioQuality: 127` is AudioQuality.MAX (build/RecordingConstants.js, the HIGH_QUALITY preset); the literals are
//      here so this file imports nothing native. `directory: 'document'`: the cache may be purged mid-set
//      (Audio.types.d.ts, RecordingOptions.fileName note); the document directory is not.
//   2. The meter: expo-audio's `metering` is AVAudioRecorder.averagePower(forChannel: 0), dBFS, -160..0
//      (node_modules/expo-audio/ios/AudioRecorder.swift getRecordingStatus). A bar meter wants dB scaled over a floor,
//      not raw amplitude (at -20 dBFS, amplitude is 0.1: a dead bar for a normal room).
//   3. The clock: elapsed time net of pauses, his own (Pause) and the system's (a phone call: expo-audio pauses the
//      recorder, AudioModule.swift handleInterruptionBegan → pauseForSystem, and resumes it when the call ends with
//      `.shouldResume`, resumeInterruptedPlayers → resumeAfterSystemPause). The clock only watches `isRecording` and
//      never drives the native recorder itself: AudioRecorder.swift startRecording ignores the result of
//      AVAudioRecorder.record(), so a resume attempted DURING the call would be reported as recording and would also
//      clear the flag the native auto-resume relies on. The owner's own Resume tap is the one exception.
import type { RecordingOptions } from 'expo-audio';

/** The keep-awake tag the recorder holds while a tape runs (expo-keep-awake activateKeepAwakeAsync/deactivateKeepAwake). */
export const KEEP_AWAKE_TAG = 'punchable-recording';

export const SET_RECORDING: RecordingOptions = {
  extension: '.m4a',
  sampleRate: 48000,
  numberOfChannels: 2,
  bitRate: 160000,
  isMeteringEnabled: true,
  directory: 'document',
  android: { outputFormat: 'mpeg4', audioEncoder: 'aac' },
  ios: { outputFormat: 'aac ', audioQuality: 127 },
  web: { mimeType: 'audio/webm', bitsPerSecond: 160000 },
};

// ---------------------------------------------------------------- the meter
/** dBFS → linear amplitude (0..1). -6 dB ≈ 0.5, -20 dB = 0.1. Non-finite or missing → 0. */
export function dbToLinear(db: number | undefined | null): number {
  if (typeof db !== 'number' || !Number.isFinite(db)) return 0;
  return Math.max(0, Math.min(1, Math.pow(10, db / 20)));
}

/** dBFS → a bar height 0..1 over a floor (default -60 dBFS: a quiet room sits near the bottom, a laugh fills it). */
export function meterLevel(db: number | undefined | null, floor = -60): number {
  if (typeof db !== 'number' || !Number.isFinite(db) || floor >= 0) return 0;
  return Math.max(0, Math.min(1, (db - floor) / -floor));
}

// ---------------------------------------------------------------- the clock
export type PausedBy = 'none' | 'user' | 'system';
export type Clock = { startedAt: number; pausedTotal: number; pausedAt: number; by: PausedBy };

export const clockStart = (now: number): Clock => ({ startedAt: now, pausedTotal: 0, pausedAt: 0, by: 'none' });

/** Pause the clock (idempotent: a second pause does not move pausedAt). */
export const clockPause = (c: Clock, now: number, by: Exclude<PausedBy, 'none'>): Clock => (c.pausedAt ? c : { ...c, pausedAt: now, by });

/** Resume: the paused stretch joins pausedTotal. A clock that is not paused is returned as is. */
export const clockResume = (c: Clock, now: number): Clock =>
  c.pausedAt ? { startedAt: c.startedAt, pausedTotal: c.pausedTotal + Math.max(0, now - c.pausedAt), pausedAt: 0, by: 'none' } : c;

/** Time on tape: now (or the moment it paused) minus the start minus every pause. Never negative. */
export const clockElapsed = (c: Clock, now: number): number => Math.max(0, (c.pausedAt || now) - c.startedAt - c.pausedTotal);

/**
 * Each tick, what the native recorder reports. A system pause (the recorder stopped while he did not press Pause)
 * opens a 'system' gap; the recorder coming back closes it. His own pause is never touched: he ends it himself.
 */
export function clockObserve(c: Clock, nativeRecording: boolean, now: number): Clock {
  if (c.by === 'user') return c;
  if (!nativeRecording && !c.pausedAt) return clockPause(c, now, 'system');
  if (nativeRecording && c.by === 'system') return clockResume(c, now);
  return c;
}

// ---------------------------------------------------------------- the input
export type InputLike = { uid: string; name?: string; type?: unknown };

/** The port type as a string: AVAudioSession.Port raw values ('MicrophoneBuiltIn', 'MicrophoneWired', 'BluetoothHFP',
 *  'UsbAudio', 'HeadsetMic', 'LineIn', …). expo-audio hands the struct over in getAvailableInputs and the raw value
 *  in getCurrentInput (ios/AudioUtils.swift:7 vs :18), so both shapes are accepted. */
export function portType(x: InputLike): string {
  const t: any = x.type;
  const s = t && typeof t === 'object' && 'rawValue' in t ? t.rawValue : t;
  return typeof s === 'string' ? s : '';
}

const RANK: Record<string, number> = { UsbAudio: 0, MicrophoneWired: 1, LineIn: 1, HeadsetMic: 2, MicrophoneBuiltIn: 3 };
/**
 * Which mic tapes the room. expo-audio always opens the session with allowBluetoothHFP (ios/AudioModule.swift, the
 * playAndRecord branch), so a connected headset becomes the input at telephone quality. Order: a USB or wired mic,
 * then the phone's own, never Bluetooth HFP while anything else exists. Returns null when the current input already
 * is the pick (nothing to set) or when there is no better choice.
 */
export function pickInput(inputs: InputLike[], current?: InputLike | null): InputLike | null {
  const rank = (x: InputLike) => RANK[portType(x)] ?? (portType(x) === 'BluetoothHFP' ? 9 : 5);
  const best = [...inputs].sort((a, b) => rank(a) - rank(b))[0];
  if (!best) return null;
  if (current && (current.uid === best.uid || rank(current) <= rank(best))) return null;
  return best;
}

// ---- the record control's states (lib/RecordControl; docs/design/HIG.md §3.9, 2026-10-29)
/** idle: nothing taped yet · recording · paused (his pause; a call shows as recording + interrupted) · saving: stopped, uploading. */
export type RecordState = 'idle' | 'recording' | 'paused' | 'saving';
export type RecordAction = 'start' | 'pause' | 'resume' | 'stop';

/** The only moves the control makes. Anything else is a no-op (a second tap during saving, a pause while idle). */
export function nextRecordState(s: RecordState, a: RecordAction): RecordState {
  if (a === 'start') return s === 'idle' ? 'recording' : s;
  if (a === 'pause') return s === 'recording' ? 'paused' : s;
  if (a === 'resume') return s === 'paused' ? 'recording' : s;
  if (a === 'stop') return s === 'recording' || s === 'paused' ? 'saving' : s;
  return s;
}

/** The word beside the clock. */
export function statusWord(s: RecordState, interrupted: boolean, clockOnly: boolean): string {
  if (s === 'saving') return 'Saving…';
  if (s === 'recording') return interrupted ? 'Paused by a call' : 'Recording';
  if (s === 'paused') return 'Paused';
  return clockOnly ? 'Clock only' : 'Ready';
}
