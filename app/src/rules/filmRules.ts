// Film it (docs/design/SPEC-2026-11-03-film-it.md, built on the spike's v1 path, SPIKE-2026-11-03 §7.1): the camera
// screen's pure rules. The state machine (idle → countdown → recording → review → saving → done; a call or the
// background ends the take into the review, cut short; a second Keep while saving is a no-op), the one-word reasons
// on the disc, the status lines, the take's row on the bit's On tape screen, and the .srt sidecar cut from the take's
// per-word timings (at most four words or 1.6 s a caption, broken where the words reach the end of one of his lines).
// No React, no React Native: tests/filmRules.test.mjs runs it under node, with a red arm.
import { mmss } from './format.ts';
import { monthDay } from './bitLines.ts';
import type { CutShort, LocalRec, Word } from './memoRules.ts';

export type FilmState = 'idle' | 'countdown' | 'recording' | 'review' | 'saving' | 'done';
/** arm: the disc (the countdown starts) · go: the count reached one, or he tapped it to skip · cancel: Close during the
 *  count · stop: the square · ended: the take ended on its own (a call, the background) · redo · keep · kept · failed
 *  (the Photos save did not go through: the take is still here). */
export type FilmAction = 'arm' | 'go' | 'cancel' | 'stop' | 'ended' | 'redo' | 'keep' | 'kept' | 'failed';

/** The only moves the screen makes; anything else is a no-op. */
export function nextFilmState(s: FilmState, a: FilmAction): FilmState {
  switch (a) {
    case 'arm': return s === 'idle' ? 'countdown' : s;
    case 'go': return s === 'countdown' ? 'recording' : s;
    case 'cancel': return s === 'countdown' ? 'idle' : s;
    case 'stop': case 'ended': return s === 'recording' ? 'review' : s;
    case 'redo': return s === 'review' ? 'idle' : s;
    case 'keep': return s === 'review' ? 'saving' : s;
    case 'kept': return s === 'saving' ? 'done' : s;
    case 'failed': return s === 'saving' ? 'review' : s;
  }
}

/** 3 · 2 · 1 over the preview, a selection haptic a tick (SPEC D6); nothing hums while recording. */
export const COUNTDOWN_S = 3;
/** A rant is a minute; the cap is a guard (SPEC §2.9). */
export const MAX_TAKE_S = 600;
/** Under this much free space the disc greys "no room" (SPEC §2.9). */
export const LOW_STORAGE_BYTES = 500 * 1024 * 1024;
/** The take as it is written: 1080p portrait HEVC (the spike's §6 figure: about 60 MB a minute). */
export const VIDEO_CODEC = 'hvc1';
export const VIDEO_QUALITY = '1080p';

export type DiscReason = 'no camera' | 'no mic' | 'no room' | null;
/** Why the disc is greyed, one word, the first that applies (the camera before the mic before the room). */
export function discReason(x: { camera: boolean; mic: boolean; freeBytes: number | null }): DiscReason {
  if (!x.camera) return 'no camera';
  if (!x.mic) return 'no mic';
  if (x.freeBytes != null && Number.isFinite(x.freeBytes) && x.freeBytes < LOW_STORAGE_BYTES) return 'no room';
  return null;
}
/** The status word before the disc, with the one qualifier that has a fix (he frees space). */
export function readyStatus(reason: DiscReason, countingDown = false): string {
  if (countingDown) return 'Starting…';
  return reason === 'no room' ? 'Ready · low storage' : 'Ready';
}

/** How the take ended when the file came back without the square: the cap (`maxDuration`) is not cut short at all;
 *  our stop from the background (or a file that came back while the app was not in front: the lock's own stop can
 *  land before ours) = the app left the foreground; otherwise the session was taken (a call, Siri, an alarm). */
export function cutShortOf(weStopped: boolean, byBackground: boolean, durationMs = 0, maxS = MAX_TAKE_S): CutShort | null {
  if (!weStopped && durationMs >= (maxS - 1) * 1000) return null;
  if (byBackground) return 'background';
  if (!weStopped) return 'call';
  return null;
}
export const cutShortWords = (c: CutShort | null | undefined): string => (c === 'call' ? 'cut short by a call' : c === 'background' ? 'cut short' : '');

/** The one status line under the review player: "0:48", "0:31 · cut short by a call"; with the words once they exist
 *  (after Keep, from the queue): "0:48 · 112 words", "0:48 · no words". */
export function reviewStatus(durationMs: number, cutShort: CutShort | null | undefined, words?: number | null): string {
  const parts = [mmss(Math.max(0, durationMs) / 1000, { floor: true })];
  const c = cutShortWords(cutShort);
  if (c) parts.push(c);
  if (words != null) parts.push(words > 0 ? `${words} ${words === 1 ? 'word' : 'words'}` : 'no words');
  return parts.join(' · ');
}
/** The Leave sheet's message. */
export const leaveMessage = (durationMs: number): string => `${mmss(Math.max(0, durationMs) / 1000, { floor: true })} recorded.`;

/** The number of words in a take's text (the row's "112 words"); null while the words do not exist yet. */
export function wordTotal(r: Pick<LocalRec, 'state' | 'text' | 'words'>): number | null {
  if (r.state === 'no_speech') return 0;
  if (r.words?.length) return r.words.length;
  const t = (r.text || '').trim();
  if (!t) return null;
  return t.split(/\s+/).filter(Boolean).length;
}
/** A kept take on the bit's On tape screen: "Filmed · Nov 3 · 0:48", second line "112 words" (or "cut short by a
 *  call · 112 words"), else the queue's own words while they are on their way. */
export function takeRow(r: LocalRec, queueWords: string): { label: string; sub: string } {
  const label = ['Filmed', monthDay(r.recorded_at), r.duration_s > 0 ? mmss(r.duration_s, { floor: true }) : ''].filter(Boolean).join(' · ');
  const n = wordTotal(r);
  const done = n != null && (r.state === 'filed' || r.state === 'new_idea' || r.state === 'sent' || r.state === 'no_speech');
  const sub = [cutShortWords(r.cut_short), done ? (n > 0 ? `${n} ${n === 1 ? 'word' : 'words'}` : 'no words') : queueWords].filter(Boolean).join(' · ');
  return { label, sub };
}
/** How many kept takes a bit has on this phone ("filmed 2" on the On tape row). */
export const filmedCount = (recs: Pick<LocalRec, 'kind' | 'bit'>[], bitId: string): number => recs.filter((r) => r.kind === 'film' && r.bit?.id === bitId).length;

// ------------------------------------------------------------------ the .srt sidecar
/** A caption: at most CUE_WORDS words or CUE_S seconds (SPEC D9), never across the end of one of his lines. */
export const CUE_WORDS = 4;
export const CUE_S = 1.6;
export type Cue = { text: string; s: number; e: number };

const fold = (s: string) => s.toLowerCase().replace(/[^\p{L}\p{N}']+/gu, ' ').trim();
/** The normalised last two words of each line: where a caption may break because a line of his ended there. */
export function lineEnds(lines: string[]): Set<string> {
  const out = new Set<string>();
  for (const l of lines) {
    const w = fold(l).split(' ').filter(Boolean);
    if (w.length) out.add(w.slice(-2).join(' '));
  }
  return out;
}
/** The take's words cut into captions: first at the ends of his lines (a caption never crosses one), then wherever
 *  the words run past CUE_S, then each run of more than CUE_WORDS words into near-equal parts (3 + 3 + 3, not
 *  4 + 4 + 1). Timings must be monotonic (memoRules addFinal makes them so); a word that is not is pushed after the
 *  one before, never dropped. */
export function cuesOf(words: Word[], lines: string[] = [], maxWords = CUE_WORDS, maxS = CUE_S): Cue[] {
  const ends = lineEnds(lines);
  // 1. the stream, cleaned, cut into phrases at his line ends
  const phrases: Word[][] = [];
  let cur: Word[] = [];
  let last = 0;
  let prev = '';
  for (const raw of words) {
    const w = (raw.w || '').trim();
    if (!w) continue;
    const s = Math.max(last, Number.isFinite(raw.s) ? raw.s : last);
    const e = Math.max(s, Number.isFinite(raw.e) ? raw.e : s);
    last = e;
    cur.push({ w, s, e });
    const tail = fold(`${prev} ${w}`);
    prev = w;
    if (ends.has(tail) || (ends.has(fold(w)) && cur.length === 1)) { phrases.push(cur); cur = []; prev = ''; }
  }
  if (cur.length) phrases.push(cur);
  // 2. each phrase into runs that fit the time cap, 3. each run into near-equal parts under the word cap
  const out: Cue[] = [];
  const cap = Math.max(1, maxWords);
  for (const ph of phrases) {
    const runs: Word[][] = [];
    let run: Word[] = [];
    for (const w of ph) {
      if (run.length && w.e - run[0].s > maxS) { runs.push(run); run = []; }
      run.push(w);
    }
    if (run.length) runs.push(run);
    for (const r of runs) {
      const k = Math.ceil(r.length / cap);
      const base = Math.floor(r.length / k);
      let extra = r.length - base * k;
      let at = 0;
      for (let p = 0; p < k; p++) {
        const n = base + (extra > 0 ? 1 : 0);
        if (extra > 0) extra--;
        const part = r.slice(at, at + n);
        at += n;
        out.push({ text: part.map((x) => x.w).join(' '), s: part[0].s, e: Math.max(part[part.length - 1].e, part[0].s) });
      }
    }
  }
  return out;
}
/** "00:00:01,200" (SubRip: hours, minutes, seconds, a comma, milliseconds). */
export function srtTime(seconds: number): string {
  const ms = Math.max(0, Math.round(seconds * 1000));
  const h = Math.floor(ms / 3600000);
  const m = Math.floor((ms % 3600000) / 60000);
  const s = Math.floor((ms % 60000) / 1000);
  const r = ms % 1000;
  const two = (n: number) => String(n).padStart(2, '0');
  return `${two(h)}:${two(m)}:${two(s)},${String(r).padStart(3, '0')}`;
}
/** The sidecar's text; a cue shorter than 0.3 s is shown for 0.3 s (a flash is not a caption); '' with no words. */
export function srtOf(words: Word[], lines: string[] = []): string {
  const cues = cuesOf(words, lines);
  if (!cues.length) return '';
  return cues.map((c, i) => `${i + 1}\n${srtTime(c.s)} --> ${srtTime(Math.max(c.e, c.s + 0.3))}\n${c.text}\n`).join('\n') + '\n';
}
/** The sidecar's NAME beside the take's file: the same name, .srt. */
export const srtNameOf = (file: string): string => file.replace(/\.[^.]+$/, '') + '.srt';
