// Web: no file transcription (the iPhone app transcribes on the phone: transcribe.native.ts). This twin keeps
// expo-speech-recognition out of the web bundle; nothing on the web calls it (lib/recordings.ts is a stub).
import type { Outcome, Seg, Word } from '@/rules/memoRules';

export const transcribeSupported = false;
export type TranscribeResult = { outcome: Outcome; segs: Seg[]; words: Word[]; error: string | null; engine: string };
export type TranscribeRun = { done: Promise<TranscribeResult>; abort: () => void };

export async function speechAllowed(): Promise<boolean> {
  return false;
}
export function transcribeFile(_uri: string, _durationS: number, _onProgress?: (p: number) => void): TranscribeRun {
  return { done: Promise.resolve({ outcome: 'unavailable', segs: [], words: [], error: 'iphone-only', engine: 'none' }), abort: () => {} };
}
