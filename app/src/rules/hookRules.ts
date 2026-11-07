// Hook check on the script (docs/design/SPEC-2026-11-06-rant-tab.md §5): no AI, a reading of the first lines at his
// pace. The seconds to the hook's last word (line 1's words at rules/prompterRules paceWpm), flagged when over 3 s
// ("Hook lands late: 4.2 s"); how many words come before the first line that is his; whether line 1 is a question, a
// claim or a scene (a word, not a score). Pure: tests/hookRules.test.mjs runs it under node, with a --red arm.
import { DEFAULT_WPM, wordCount } from './prompterRules.ts';

/** A hook that lands after this many seconds is late (SPEC §5). */
export const HOOK_LATE_S = 3;

export type HookShape = 'question' | 'claim' | 'scene';
export type HookCheck = { seconds: number; late: boolean; wordsBeforeHis: number | null; shape: HookShape };

// A line that opens on a time, a place or "I was ..." puts him somewhere: a scene. Anything else that is not a
// question states something: a claim.
const SCENE = /^(so,?\s+)?(i was|i'm (at|in|standing|sitting|walking)|i am (at|in)|i went|i walked|i got|picture|imagine|yesterday|last (night|week|month|year)|this (morning|afternoon|week)|the other (day|night)|today,? i|you're (at|in|standing|walking)|you are (at|in)|you walk|you go)\b/i;
const CLOSERS = /["”’')\]]+$/u;
const OPENERS = /^["“‘'(\[]+/u;

/** Question, claim or scene, from the words of line 1. */
export function shapeOf(line: string): HookShape {
  const t = (line || '').trim().replace(CLOSERS, '').trim();
  if (t.endsWith('?')) return 'question';
  if (SCENE.test(t.replace(OPENERS, ''))) return 'scene';
  return 'claim';
}

/** The check over the script's lines (the first is the hook); `his[i]` says line i is his words. */
export function hookCheck(lines: string[], his: boolean[], wpm: number = DEFAULT_WPM): HookCheck | null {
  const ls = lines.map((l) => (l || '').trim()).filter(Boolean);
  if (!ls.length) return null;
  const rate = wpm > 0 ? wpm : DEFAULT_WPM;
  const seconds = Math.round((wordCount(ls[0]) / rate) * 600) / 10;
  const first = his.findIndex(Boolean);
  const wordsBeforeHis = first < 0 ? null : ls.slice(0, first).reduce((n, l) => n + wordCount(l), 0);
  return { seconds, late: seconds > HOOK_LATE_S, wordsBeforeHis, shape: shapeOf(ls[0]) };
}

/** "Hook lands at 2.1 s" / "Hook lands late: 4.2 s". */
export function hookWord(c: HookCheck): string {
  const s = c.seconds.toFixed(1);
  return c.late ? `Hook lands late: ${s} s` : `Hook lands at ${s} s`;
}

/** The second line of the check: "a claim · 6 words before yours" / "a question · opens on your words". */
export function hookDetail(c: HookCheck): string {
  const n = c.wordsBeforeHis;
  const yours = n == null ? 'none of it is yours yet' : n === 0 ? 'opens on your words' : `${n} ${n === 1 ? 'word' : 'words'} before yours`;
  return `${c.shape === 'question' ? 'a question' : c.shape === 'scene' ? 'a scene' : 'a claim'} · ${yours}`;
}
