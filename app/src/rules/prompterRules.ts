// The prompter band's pure rules (docs/design/SPEC-2026-11-03-film-it.md §2.3 D3, the spike's §5 and §7.1): his lines
// advance one at a time at his pace, the current line at the top of the band (nearest the front lens), at most four
// lines showing. Each line's dwell is its word count over his words-per-minute (the bit's taped run time, else a
// default), never under 1.2 s; Hold freezes the remaining dwell; Next line advances at once and re-arms; the last line
// holds until he stops; nothing runs while idle (Next line before the disc only moves the start line). The band is
// stepped, never a pixel scroll (SPEC D2: motion under his eyes, and nothing for Reduce Motion to fall back to).
// No React, no React Native: tests/prompterRules.test.mjs runs it under node, with a red arm.
import { lineLabel } from './bitLines.ts';

/** 2.4 words a second (SPEC D3, INFERRED) until the bit has a taped run time. */
export const DEFAULT_WPM = 144;
/** A pace outside this is not a pace (a 2-second run time on a 40-word bit): the default is used instead. */
export const WPM_MIN = 60;
export const WPM_MAX = 300;
/** A line is never shown for less than this (SPEC §2.3). */
export const MIN_DWELL_MS = 1200;
/** At most four lines on the band, the current at the top (SPIKE §5); two at font scale >= 1.3 (SPEC §2.3). */
export const BAND_LINES = 4;
export const BAND_LINES_LARGE = 2;
export const BAND_LARGE_AT = 1.3;
/** The fade on a line change; a cut under Reduce Motion (HIG/accessibility: fades replace motion). */
export const SWAP_MS = 120;

export const wordCount = (line: string): number => line.trim().split(/\s+/).filter(Boolean).length;

/** His pace for this bit: its words over its taped run time (`duration_s`, seconds, from his tapes), else the default.
 *  Clamped: a run time that gives a pace no reader has is not his pace. */
export function paceWpm(lines: string[], durationS?: number | null): number {
  const words = lines.reduce((n, l) => n + wordCount(l), 0);
  if (!durationS || !Number.isFinite(durationS) || durationS <= 0 || words <= 0) return DEFAULT_WPM;
  const wpm = words / (durationS / 60);
  if (wpm < WPM_MIN || wpm > WPM_MAX) return DEFAULT_WPM;
  return Math.round(wpm);
}

/** How long a line stays current: its words at his pace, never under MIN_DWELL_MS. */
export function dwellMs(line: string, wpm: number): number {
  const rate = wpm > 0 ? wpm : DEFAULT_WPM;
  return Math.max(MIN_DWELL_MS, Math.round((wordCount(line) / rate) * 60000));
}

/** `i`: the current line. `running`: the take is on (the timer may arm). `held`: the timer is frozen with `leftMs`
 *  of the current line's dwell left. `dueAt`: when the next advance is due (running, not held, not the last line). */
export type Prompter = { i: number; running: boolean; held: boolean; dueAt: number | null; leftMs: number | null };
export const prompterIdle = (i = 0): Prompter => ({ i: Math.max(0, i), running: false, held: false, dueAt: null, leftMs: null });

const last = (lines: string[]) => Math.max(0, lines.length - 1);
const arm = (p: Prompter, from: number, lines: string[], wpm: number): Prompter =>
  p.i >= last(lines) ? { ...p, dueAt: null, leftMs: null } : { ...p, dueAt: from + dwellMs(lines[p.i] || '', wpm), leftMs: null };

/** The disc was tapped: the band runs from the line he is on. */
export function startPrompter(p: Prompter, now: number, lines: string[], wpm: number): Prompter {
  return arm({ ...p, i: Math.min(p.i, last(lines)), running: true, held: false }, now, lines, wpm);
}
/** The take ended: nothing advances any more (the band keeps the line he stopped on for the review). */
export function stopPrompter(p: Prompter): Prompter {
  return { ...p, running: false, held: false, dueAt: null, leftMs: null };
}
/** Every tick: an advance that is due lands; the next one is armed from when it was due (no drift). Held, idle or on
 *  the last line nothing moves. */
export function tickPrompter(p: Prompter, now: number, lines: string[], wpm: number): Prompter {
  if (!p.running || p.held || p.dueAt == null || now < p.dueAt) return p;
  let q: Prompter = p;
  // several advances can be due after a long stall (the app was in the background); each lands in order
  while (q.running && !q.held && q.dueAt != null && now >= q.dueAt && q.i < last(lines)) {
    q = arm({ ...q, i: q.i + 1 }, q.dueAt, lines, wpm);
  }
  return q;
}
/** Next line: at once, and the timer re-arms for the new line (stays on the last line; while held it stays held with
 *  the new line's whole dwell; idle it only moves the start line). */
export function nextLine(p: Prompter, now: number, lines: string[], wpm: number): Prompter {
  const i = Math.min(p.i + 1, last(lines));
  if (!p.running) return { ...p, i };
  if (p.held) return { ...p, i, dueAt: null, leftMs: i >= last(lines) ? null : dwellMs(lines[i] || '', wpm) };
  return arm({ ...p, i }, now, lines, wpm);
}
/** Hold: the remaining dwell is frozen (the take keeps recording; only the band waits). */
export function holdPrompter(p: Prompter, now: number): Prompter {
  if (!p.running || p.held) return p;
  return { ...p, held: true, dueAt: null, leftMs: p.dueAt == null ? null : Math.max(0, p.dueAt - now) };
}
/** Go: the frozen dwell runs out from now. */
export function goPrompter(p: Prompter, now: number, lines: string[], wpm: number): Prompter {
  if (!p.running || !p.held) return p;
  if (p.i >= last(lines)) return { ...p, held: false, dueAt: null, leftMs: null };
  return { ...p, held: false, dueAt: now + (p.leftMs ?? dwellMs(lines[p.i] || '', wpm)), leftMs: null };
}
export const toggleHold = (p: Prompter, now: number, lines: string[], wpm: number): Prompter => (p.held ? goPrompter(p, now, lines, wpm) : holdPrompter(p, now));

/** The lines on the band: the current one first, then what follows, at most `n`. */
export function bandWindow(lines: string[], i: number, n: number): string[] {
  const from = Math.max(0, Math.min(i, last(lines)));
  return lines.slice(from, from + Math.max(1, n));
}
/** How many lines the band holds at the OS text size (SPEC §2.3: two from 1.3×). */
export const bandLines = (fontScale: number): number => (Number.isFinite(fontScale) && fontScale >= BAND_LARGE_AT ? BAND_LINES_LARGE : BAND_LINES);
/** VoiceOver for the band (one element): "Line 2 of 7, punch: …" (rules/bitLines lineLabel). */
export const bandLabel = (i: number, count: number, punch: number[], text: string): string => lineLabel(i, count, punch, text);
/** The left dock control's word. */
export const holdWord = (held: boolean): string => (held ? 'Go' : 'Hold');
