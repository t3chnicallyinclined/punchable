// Voice memos and taped sets on the phone (docs/design/SPEC-2026-10-31-voice-memos.md): the rules, pure and tested
// (tests/memoRules.test.mjs). The audio stays in Documents/recordings; each recording has one row in a small local
// index; it is transcribed ON the phone (lib/transcribe.native) and only the words go to the server: a memo as an
// idea (POST /api/ideas, filed on a bit), a set (and a memo too long for one idea) as POST /api/sets/transcript.
// lib/recordings.native.ts is the wiring (files, the queue, the network); everything that decides is here.
import { backoffMs } from './uploadsCore.ts';

export type RecKind = 'memo' | 'set';
export type RecState =
  | 'queued' | 'transcribing'                                 // waiting for, or in, the on-device transcriber
  | 'words' | 'sending'                                       // words on the phone, not yet on the server
  | 'filed' | 'new_idea' | 'sent'                             // done: on a bit, a new idea, or a set transcript
  | 'no_speech' | 'failed' | 'needs_permission' | 'unavailable';

/** One recording on this phone. `file` is a NAME under Documents/recordings, never an absolute path: the app
 *  container's path changes across updates, so the URI is rebuilt from documentDirectory at use. */
export type LocalRec = {
  id: string; kind: RecKind; file: string; recorded_at: string; duration_s: number;
  /** a memo started on a bit's page: filed on that bit */
  bit?: { id: string; title: string } | null;
  /** a set taped from a set list */
  board?: { id: string; title: string } | null;
  title?: string | null;
  state: RecState;
  /** 0..1 of the current run; `peak` is the furthest an earlier, interrupted run got (the shown % never goes back) */
  progress?: number; peak?: number;
  /** the app left the foreground mid-run: the row says Paused (the run restarts from zero when it is back) */
  paused?: boolean;
  text?: string | null;
  segments?: Seg[] | null;
  engine?: string | null;
  idea_id?: string | null; merge_id?: string | null; filed_on?: { id: string; title: string } | null;
  error?: string | null;
  marks_ms?: number[];
  attempts: number; next_at: number;
};
export type Seg = { text: string; start: number; end: number };

/** The API's caps (creative_api.set_transcript; server.idea_result). */
export const SEGMENTS_MAX = 5000;
export const IDEA_MAX = 4000;
export const WATCHDOG_MS = 120_000;
/** How long a run may say nothing before it is aborted: 2 minutes, or a third of the file's length for a long one
 *  (iOS before 18 sends no final until the end of the file; the file is fed at most ~5.7x real time) [review]. */
export function watchdogMs(durationS: number): number {
  return Math.max(WATCHDOG_MS, durationS > 0 ? Math.round((durationS * 1000) / 3) : 0);
}
export const ENGINE = 'ios-sfspeech-ondevice';
export const KEEP_AWAKE_TRANSCRIBE = 'punchable-transcribe';

// ------------------------------------------------------------------ the transcript, from the recognizer's finals
/** What the recognizer has said so far in one run. */
export type Acc = { segs: Seg[]; prev: string; lastEnd: number };
export const emptyAcc = (): Acc => ({ segs: [], prev: '', lastEnd: 0 });
type FinalSeg = { startTimeMillis: number; endTimeMillis: number };

/** One `result` event. Only finals count (interimResults is off); each final is one utterance of a long file; the
 *  second and later carry a leading space; the last can be empty or repeat the one before (package source,
 *  ExpoSpeechRecognitionModule.swift:459-518). Whether timestamps run across the file or restart per final is not
 *  known until a device build, so a final that starts before the previous one ended is moved to follow it. */
export function addFinal(acc: Acc, ev: { isFinal: boolean; results: { transcript: string; segments?: FinalSeg[] }[] }): Acc {
  const r = ev.results?.[0];
  if (!ev.isFinal || !r) return acc;
  const text = (r.transcript || '').trim();
  if (!text || text === acc.prev) return acc;
  const segs = r.segments || [];
  let start = segs.length ? segs[0].startTimeMillis / 1000 : acc.lastEnd;
  let end = segs.length ? segs[segs.length - 1].endTimeMillis / 1000 : start;
  if (!(start >= 0) || !(end >= 0)) { start = acc.lastEnd; end = acc.lastEnd; }
  if (start < acc.lastEnd - 1) {
    const shift = acc.lastEnd - start;
    start += shift;
    end += shift;
  }
  end = Math.max(end, start);
  const seg = { text, start: round2(start), end: round2(end) };
  return { segs: [...acc.segs, seg], prev: text, lastEnd: seg.end };
}
const round2 = (x: number) => Math.round(x * 100) / 100;

/** At most `cap` segments: adjacent pairs are merged until it fits (a 45-minute set can pass the API's 5000). */
export function capSegments(segs: Seg[], cap = SEGMENTS_MAX): Seg[] {
  let out = segs;
  while (out.length > cap) {
    const next: Seg[] = [];
    for (let i = 0; i < out.length; i += 2) {
      const a = out[i];
      const b = out[i + 1];
      next.push(b ? { text: `${a.text} ${b.text}`, start: a.start, end: b.end } : a);
    }
    out = next;
  }
  return out;
}
export const textOf = (segs: Seg[]) => segs.map((s) => s.text).join(' ').replace(/\s+/g, ' ').trim();

/** Progress of a run: where the last final ended, over the duration. */
export function progressOf(acc: Acc, durationS: number): number {
  if (!(durationS > 0)) return 0;
  return Math.max(0, Math.min(1, acc.lastEnd / durationS));
}

// ------------------------------------------------------------------ how a run ended
export type Outcome = 'done' | 'no_speech' | 'requeue' | 'needs_permission' | 'unavailable' | 'failed';
/** The package's error codes (ExpoSpeechRecognitionModule.swift:531-607) to what the row does next. `ours` = we
 *  aborted it (the app went to the background, the watchdog): it goes back in the queue. A busy recognizer or a lost
 *  speech daemon (`network`) is retried; anything we do not know fails with the audio kept: never a retry loop. */
export function outcomeOf(error: string | null, segCount: number, ours: boolean): Outcome {
  if (ours) return 'requeue';
  if (!error) return segCount ? 'done' : 'no_speech';
  switch (error) {
    case 'no-speech': case 'nomatch': return segCount ? 'done' : 'no_speech';
    case 'not-allowed': return 'needs_permission';
    case 'service-not-allowed': case 'on-device-unavailable': return 'unavailable';
    case 'busy': case 'network': return 'requeue';
    default: return 'failed';    // audio-capture, interrupted, language-not-supported, an aborted we did not cause, …
  }
}
/** A requeued run that keeps coming back stops after this many tries (then it is failed, Try again). */
export const REQUEUE_MAX = 6;
/** After a run ended `requeue`. `byUs` = we stopped it: `bg` (the app left the foreground: the row says Paused) or `rec`
 *  (a recording started: it just waits); no try is counted. Otherwise (busy, the speech daemon lost, the watchdog) it
 *  backs off and counts a try, failing after REQUEUE_MAX. */
export function afterRequeue(r: LocalRec, now: number, byUs: 'bg' | 'rec' | null, error: string | null): LocalRec {
  const peak = Math.max(r.peak || 0, r.progress || 0);
  if (byUs) return { ...r, state: 'queued', paused: byUs === 'bg', peak, progress: 0 };
  if (r.attempts + 1 >= REQUEUE_MAX) return { ...r, state: 'failed', error, attempts: 0, progress: 0, paused: false };
  return { ...r, state: 'queued', attempts: r.attempts + 1, next_at: now + backoffMs(r.attempts), peak, progress: 0, paused: false };
}

// ------------------------------------------------------------------ the queue
/** The next recording to transcribe: memos before sets (seconds, not minutes), oldest first; none while one runs; a
 *  row backing off after a busy recognizer waits for its `next_at` [review]. */
export function nextToTranscribe(recs: LocalRec[], now = Number.MAX_SAFE_INTEGER): LocalRec | null {
  if (recs.some((r) => r.state === 'transcribing')) return null;
  const q = recs.filter((r) => r.state === 'queued' && (r.next_at || 0) <= now);
  q.sort((a, b) => (a.kind === b.kind ? a.recorded_at.localeCompare(b.recorded_at) : a.kind === 'memo' ? -1 : 1));
  return q[0] || null;
}
/** Words waiting for the server whose backoff has passed. */
export function dueToSend(recs: LocalRec[], now: number): LocalRec[] {
  return recs.filter((r) => r.state === 'words' && r.next_at <= now).sort((a, b) => a.next_at - b.next_at);
}
/** After a send that did not reach the server (no signal, a 5xx, a 401 the refresh could not fix): back to words. */
export function afterSendFailure(r: LocalRec, now: number, error: string): LocalRec {
  return { ...r, state: 'words', attempts: r.attempts + 1, next_at: now + backoffMs(r.attempts), error: error.slice(0, 200) };
}
/** The server refused the words for good (a 4xx that will not change): failed, the audio kept. */
export function afterRefusal(r: LocalRec, error: string): LocalRec {
  return { ...r, state: 'failed', error: error.slice(0, 200) };
}
/** On launch: a row left `transcribing` or `sending` by a killed app goes back to where it can resume. */
export function recover(r: LocalRec): LocalRec {
  if (r.state === 'transcribing') return { ...r, state: 'queued', paused: true, peak: Math.max(r.peak || 0, r.progress || 0), progress: 0 };
  if (r.state === 'sending') return { ...r, state: 'words', next_at: 0 };
  return r;
}
/** Try again: back to the transcriber (or, with the words already here, back to sending). */
export function retry(r: LocalRec): LocalRec {
  if (r.text && (r.state === 'failed' || r.state === 'words')) return { ...r, state: 'words', next_at: 0, attempts: 0, error: null };
  return { ...r, state: 'queued', error: null, progress: 0, paused: false, attempts: 0, next_at: 0 };
}

// ------------------------------------------------------------------ where the words go
/** A memo is one idea; one too long for an idea (or a set) is a set transcript, which the server splits into bits. */
export function routeOf(r: Pick<LocalRec, 'kind'>, text: string): 'idea' | 'set' {
  return r.kind === 'memo' && text.length <= IDEA_MAX ? 'idea' : 'set';
}
export function dayOf(iso: string): string {
  return /^\d{4}-\d{2}-\d{2}/.test(iso) ? iso.slice(0, 10) : 'unknown';
}
/** POST /api/ideas: the words, where they came from (server.idea_result keeps `from.note`). */
export function ideaBody(r: LocalRec, text: string) {
  return {
    text,
    from: { note: `Voice memo, ${dayOf(r.recorded_at)}` },
    reason: `[memo] voice memo recorded on the phone ${dayOf(r.recorded_at)}${r.duration_s > 0 ? `, ${Math.round(r.duration_s)} s` : ''}, transcribed on the phone${r.bit ? ` (from ${r.bit.title})` : ''}`,
  };
}
/** POST /api/sets/transcript: seconds, at most 5000 segments, 200-character title. */
export function setBody(r: LocalRec, segs: Seg[], engine: string) {
  const title = (r.title || r.board?.title || (r.kind === 'memo' ? `Voice memo ${dayOf(r.recorded_at)}` : `Set ${dayOf(r.recorded_at)}`)).slice(0, 200);
  return { segments: capSegments(segs), title, recorded_at: r.recorded_at, duration_s: Math.max(0, Math.round(r.duration_s)), engine };
}
/** What filing a memo started on a bit takes, from what POST /api/ideas answered: nothing (it auto-attached there),
 *  an attach, or an undo of the auto-attach to another bit and then the attach. */
export function filingSteps(bitId: string | null | undefined, attach: { decision?: string; into?: string; id?: string } | null | undefined): { undo?: string; attach?: boolean } {
  if (!bitId) return {};
  if (attach?.decision === 'attach' && attach.into === bitId) return {};
  if (attach?.decision === 'attach' && attach.id) return { undo: attach.id, attach: true };
  return { attach: true };
}

// ------------------------------------------------------------------ what the row says (SPEC §6, §7)
export function wordsTitle(text: string | null | undefined, fallback: string): string {
  const w = (text || '').trim().split(/\s+/).filter(Boolean);
  if (!w.length) return fallback;
  return w.slice(0, 8).join(' ') + (w.length > 8 ? '…' : '');
}
export function recTitle(r: LocalRec): string {
  if (r.kind === 'set') return r.title || r.board?.title || 'Set';
  return wordsTitle(r.text, 'Memo');
}
/** The percent to show, or null: never below what an earlier run reached (a restart begins at zero). */
export function shownPercent(r: Pick<LocalRec, 'progress' | 'peak' | 'paused'>): number | null {
  if (r.paused) return null;
  const p = r.progress || 0;
  if (p < (r.peak || 0)) return null;
  return Math.floor(p * 100);
}
export function statusWords(r: LocalRec, online = true): string {
  switch (r.state) {
    case 'queued': return r.paused ? 'Paused · finishes when Punchable is open' : 'Waiting to transcribe';
    case 'transcribing': {
      if (r.kind === 'memo') return 'Transcribing…';
      const pc = shownPercent(r);
      return pc == null ? 'Transcribing…' : `Transcribing · ${pc}%`;
    }
    case 'words': return online && r.attempts === 0 ? 'Sending…' : 'Waiting for a connection';
    case 'sending': return 'Sending…';
    case 'filed': return `On ${r.filed_on?.title || 'a bit'}`;
    case 'new_idea': return 'New idea';
    case 'sent': return 'Sent · finding the bits';
    case 'no_speech': return 'No speech heard';
    case 'failed': return r.error === 'language-not-supported' ? 'Speech in this iPhone’s language isn’t available' : 'Couldn’t transcribe · Try again';
    case 'needs_permission': return 'Needs speech recognition · Open Settings';
    case 'unavailable': return 'Turn on Dictation to transcribe · Try again';
  }
}
/** The Delete sheet's message (SPEC §6): only a filed or sent row keeps its words anywhere. */
export function deleteMessage(r: LocalRec): string {
  if (r.state === 'filed') return `The words stay on ${r.filed_on?.title || 'the bit'}.`;
  if (r.state === 'new_idea' || r.state === 'sent') return 'The words stay in Punchable.';
  if (r.text) return 'The words haven’t reached Punchable yet. This can’t be undone.';
  return 'It hasn’t been transcribed. This can’t be undone.';
}
/** Waiting on this phone: not done, not stopped on something he must fix. */
export const isPending = (r: LocalRec) => r.state === 'queued' || r.state === 'transcribing' || r.state === 'words' || r.state === 'sending';

// ------------------------------------------------------------------ files
/** A generated, URL-safe file name (the native recognizer force-unwraps URL(string:); an imported file's own name may
 *  hold spaces or characters that would crash it). */
export function safeFileName(nowIso: string, rand: string, ext: string): string {
  const e = (ext || 'm4a').toLowerCase().replace(/[^a-z0-9]/g, '').slice(0, 5) || 'm4a';
  return `${nowIso.replace(/[^0-9A-Za-z-]/g, '-')}-${rand.replace(/[^a-z0-9]/gi, '').slice(0, 8)}.${e}`;
}
export const isSafeFileName = (name: string) => /^[0-9A-Za-z._-]+$/.test(name) && !name.startsWith('.');
/** The URI the recognizer gets: a clean file:// path with nothing to escape. */
export const isCleanFileUri = (uri: string) => /^file:\/\/\/[0-9A-Za-z._\-/]+$/.test(uri);

/** The index file's rows, as read back: anything malformed is dropped, a killed run is recovered. */
export function parseIndex(raw: unknown): LocalRec[] {
  if (!Array.isArray(raw)) return [];
  return raw
    .filter((x): x is LocalRec => !!x && typeof x === 'object' && typeof (x as any).id === 'string' && typeof (x as any).file === 'string'
      && isSafeFileName((x as any).file) && ((x as any).kind === 'memo' || (x as any).kind === 'set') && typeof (x as any).state === 'string')
    .map((x) => recover({ ...x, attempts: x.attempts || 0, next_at: x.next_at || 0 }));
}
