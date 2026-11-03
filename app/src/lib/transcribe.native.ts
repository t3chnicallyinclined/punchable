// On-device transcription of one recording on this iPhone (SPEC 2026-10-31 §1): expo-speech-recognition 58.0.1 over
// Apple's SFSpeechRecognizer with requiresOnDeviceRecognition. The audio never leaves the phone. The traps in the
// package, each guarded here (package source under node_modules/expo-speech-recognition/ios):
//   -. the package loads its native module when it is imported and throws if the binary lacks it
//      (build/ExpoSpeechRecognitionModule.js:4); this file is reached from the root layout, so the import is LAZY and
//      a binary without the module (an older dev client) sees "unavailable", never a crash at launch [review].
//   0. with the on-device flag, start() skips the speech permission check (ExpoSpeechRecognitionModule.swift:207-214),
//      so the prompt is ours: requestSpeechRecognizerPermissionsAsync, then the microphone (the package wants it even
//      for a file, :217-223). Denied -> needs permission; start() is never called.
//   1. the on-device flag is applied only if the recognizer supports it (ExpoSpeechRecognizer.swift:588-590): otherwise
//      the audio would go to Apple's servers without a word. supportsOnDeviceRecognition() / isRecognitionAvailable()
//      build the recognizer from the DEVICE locale (:352-364), so `lang` is the device's own locale: the recognizer
//      checked is the one used. Either false -> unavailable; start() is never called. (Residual, INFERRED: if the
//      runtime's Intl locale and iOS's Locale.current ever differ, the check can answer for the other recognizer; a
//      3-line Swift patch closes it, docs/design/SPEC-2026-10-31-voice-memos.md §10.)
//   2. no background handling: the caller (lib/recordings.native) aborts on background and runs it again later; the
//      watchdog here aborts a run that has said nothing for a while (longer for long files: iOS before 18 sends no
//      final until the end of the file).
//   3. an unreadable file raises no error and the Swift force-unwraps URL(string:) (:234): the file must exist and
//      the URI must be a clean file:// path (rules/memoRules isCleanFileUri).
//   4. one event stream for every run: a run that ended on our own timer (an abort that never sent `end`) leaves the
//      recognizer running, and its late `end` would land on the NEXT run and read as "no speech" [review]. So after
//      such an end the module is `dirty`, and the next run first aborts and waits for that `end` before it listens.
// The finals become segments in rules/memoRules (addFinal: one per utterance, leading space, repeated last final),
// and the per-segment timings become `words` (a filmed take's captions, SPEC 2026-11-03 Film it; the memo and set
// paths ignore them). The file route never touches AVAudioSession (:337-342): the recorder's audio mode is left alone.
// A filmed take is a QuickTime .mov on the same route (the spike's v1 path): whether AVAudioFile opens it is the
// owner's device check D3. If it does not, the package's catch (ExpoSpeechRecognizer.swift:336-339) ends an EMPTY
// request, which the recognizer reports as no speech: the row reads "no words" on a take with clear speech (not
// "Couldn't transcribe"), the .mov stays in Documents, and a 30-line Swift export to .m4a before this call is the fix.
import type { ExpoSpeechRecognitionErrorEvent, ExpoSpeechRecognitionResultEvent } from 'expo-speech-recognition';
import { ENGINE, addFinal, emptyAcc, isCleanFileUri, outcomeOf, progressOf, watchdogMs } from '@/rules/memoRules';
import type { Acc, Outcome, Seg, Word } from '@/rules/memoRules';

export const transcribeSupported = true;
export type TranscribeResult = { outcome: Outcome; segs: Seg[]; words: Word[]; error: string | null; engine: string };
export type TranscribeRun = { done: Promise<TranscribeResult>; abort: () => void };

type SRModule = typeof import('expo-speech-recognition').ExpoSpeechRecognitionModule;
let srPromise: Promise<SRModule | null> | null = null;
/** The native module, or null when this binary does not have it. */
function speech(): Promise<SRModule | null> {
  if (!srPromise) srPromise = import('expo-speech-recognition').then((m) => m.ExpoSpeechRecognitionModule ?? null).catch(() => null);
  return srPromise;
}

/** The device's own language for the recognizer (see trap 1); en-US when the runtime has no Intl locale. */
function deviceLang(): string {
  try {
    const l = Intl.DateTimeFormat().resolvedOptions().locale;
    return typeof l === 'string' && /^[a-z]{2,3}([-_][A-Za-z0-9]{2,8})*$/.test(l) ? l : 'en-US';
  } catch {
    return 'en-US';
  }
}

/** Speech recognition permission (and the microphone the package also wants). Never at launch: called on the first
 *  transcription, right after a memo or set is saved (HIG/privacy). */
export async function speechAllowed(): Promise<boolean> {
  const SR = await speech();
  if (!SR) return false;
  const s = await SR.requestSpeechRecognizerPermissionsAsync();
  if (!s.granted) return false;
  const m = await SR.requestMicrophonePermissionsAsync();
  return !!m.granted;
}

let dirty = false;   // a run ended on our timer: the recognizer may still be running (trap 4)
/** Stop whatever an earlier run left running and wait for the `end` that OUR abort causes (abort sends `error:
 *  aborted`, then `end`, asynchronously): an earlier `end` may be the stuck run's own late one, and resolving on it
 *  would let our pending abort land on the next run [review]. At most 3 s; never throws. */
async function drain(SR: SRModule) {
  if (!dirty) return;
  await new Promise<void>((resolve) => {
    const subs: { remove: () => void }[] = [];
    let sawAbort = false;
    let t: ReturnType<typeof setTimeout> | null = null;
    const done = () => { if (t) clearTimeout(t); subs.forEach((s) => s.remove()); resolve(); };
    try {
      t = setTimeout(done, 3000);
      subs.push(SR.addListener('error', (e: ExpoSpeechRecognitionErrorEvent) => { if (e.error === 'aborted') sawAbort = true; }));
      subs.push(SR.addListener('end', () => { if (sawAbort) done(); }));
      SR.abort();
    } catch {
      done();
    }
  });
  dirty = false;
}

const end = (outcome: Outcome, error: string | null, segs: Seg[] = [], words: Word[] = []): TranscribeResult => ({ outcome, segs, words, error, engine: ENGINE });

/** Transcribe one file. Never rejects: the result says what happened (rules/memoRules outcomeOf). */
export function transcribeFile(uri: string, durationS: number, onProgress?: (p: number) => void): TranscribeRun {
  let ours = false;            // we aborted it (background, watchdog): back in the queue, never "no speech"
  let abortNow: () => void = () => { ours = true; };
  const done = (async (): Promise<TranscribeResult> => {
    if (!isCleanFileUri(uri)) return end('failed', 'bad-uri');
    try {
      const FS: any = await import('expo-file-system/legacy');
      if (!(await FS.getInfoAsync(uri)).exists) return end('failed', 'missing-file');
    } catch {
      return end('failed', 'missing-file');
    }
    const SR = await speech();
    if (!SR) return end('unavailable', 'on-device-unavailable');
    if (ours) return end('requeue', 'aborted');
    try {
      if (!(await speechAllowed())) return end('needs_permission', 'not-allowed');
    } catch {
      return end('needs_permission', 'not-allowed');
    }
    let onDevice = false;
    try {
      onDevice = SR.supportsOnDeviceRecognition() && SR.isRecognitionAvailable();
    } catch {
      onDevice = false;
    }
    if (!onDevice) return end('unavailable', 'on-device-unavailable');
    await drain(SR);
    if (ours) return end('requeue', 'aborted');

    return new Promise<TranscribeResult>((resolve) => {
      let acc: Acc = emptyAcc();
      let error: string | null = null;
      let finished = false;
      let started = false;
      let dog: ReturnType<typeof setTimeout> | null = null;
      let late: ReturnType<typeof setTimeout> | null = null;
      const subs: { remove: () => void }[] = [];
      const finish = (byTimer = false) => {
        if (finished) return;
        finished = true;
        if (byTimer) dirty = true;
        if (dog) clearTimeout(dog);
        if (late) clearTimeout(late);
        subs.forEach((s) => s.remove());
        resolve(end(outcomeOf(error, acc.segs.length, ours), error, acc.segs, acc.words));
      };
      const stop = () => {
        try { SR.abort(); } catch {}
        late = setTimeout(() => finish(true), 3000);   // abort sends `aborted` then `end`; if neither comes, end it here
      };
      const kick = () => {
        if (dog) clearTimeout(dog);
        dog = setTimeout(() => { ours = true; stop(); }, watchdogMs(durationS));
      };
      abortNow = () => {
        if (finished) return;
        ours = true;
        if (!started) return finish();
        stop();
      };
      subs.push(SR.addListener('result', (e: ExpoSpeechRecognitionResultEvent) => {
        kick();
        const before = acc.segs.length;
        acc = addFinal(acc, e);
        if (acc.segs.length !== before) onProgress?.(progressOf(acc, durationS));
      }));
      subs.push(SR.addListener('error', (e: ExpoSpeechRecognitionErrorEvent) => {
        if (!error && !(e.error === 'no-speech' && acc.segs.length)) error = e.error;
      }));
      subs.push(SR.addListener('nomatch', () => { if (!error) error = 'nomatch'; }));
      subs.push(SR.addListener('end', () => finish()));
      kick();
      try {
        SR.start({ lang: deviceLang(), requiresOnDeviceRecognition: true, addsPunctuation: true, maxAlternatives: 1, continuous: false, interimResults: false, audioSource: { uri } });
        started = true;
      } catch (e: any) {
        error = String(e?.code || e?.message || 'start-failed');
        finish();
      }
    });
  })().catch((e: any) => end('failed', String(e?.message || 'internal').slice(0, 80)));   // never rejects: the queue must move on
  return { done, abort: () => abortNow() };
}
