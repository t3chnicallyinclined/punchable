// A bit is a page; marking is selecting (docs/design/SPEC-2026-11-17-one-notebook.md screens 2 and 3). One bar sits
// above the keyboard: with a caret it offers Say it · Film it · Punch it up; with words selected it offers Funnier ·
// Shorter · 3 options · Note for exactly those words. Each of the four is a switch on the selected words, the way Bold
// is in a text editor: tap it and they are marked that way, tap it again and the mark is gone, tap another and how he
// asked changes. A mark is still the Redline mark of rules/markRules (a Change on quoted words, found again by the
// words themselves), with one more word: `intent`. These rules are pure: which mark the selection is on, what a tap on
// a button does, what the bar says, what Punch it up means right now. No React: tests/pageRules.test.mjs runs them
// under node, with a --red arm per rule.

import { MAX_NOTE, MAX_QUOTE, addMark, locate, markCount, type Intent, type Mark, type RoundState } from './markRules.ts';

/** The four buttons the selection offers. */
export type Ask = 'funnier' | 'shorter' | 'options' | 'note';
export const ASKS: { k: Ask; label: string; a11y: string; hint: string }[] = [
  { k: 'funnier', label: 'Funnier', a11y: 'Funnier', hint: 'Marks these words to be made funnier in the next draft' },
  { k: 'shorter', label: 'Shorter', a11y: 'Shorter', hint: 'Marks these words to be said in fewer words' },
  { k: 'options', label: '3 options', a11y: 'Three options for these words', hint: 'Asks for three takes on these words in the next draft' },
  { k: 'note', label: 'Note', a11y: 'Add a note to these words', hint: 'Marks these words with a note you type' },
];

/** How a mark is named in the list under the page and to VoiceOver. */
export function askWord(m: { type: string; intent?: Intent | null }): string {
  if (m.type === 'keep') return 'Keep';
  if (m.type === 'ask') return 'Question';
  return m.intent === 'funnier' ? 'Funnier' : m.intent === 'shorter' ? 'Shorter' : m.intent === 'options' ? '3 options' : m.intent === 'again' ? 'Another take' : 'Note';
}

/** The Change that is on exactly these words (the same words at the same place), if there is one. */
export function markOn(text: string, set: readonly Mark[], target: { text: string; at: number } | null): Mark | null {
  if (!target) return null;
  const at = locate(text, target);
  if (at == null) return null;
  return set.find((m) => m.type === 'change' && m.text === target.text && locate(text, m) === at) || null;
}

/** Which of the four buttons are lit for the selected words. */
export function lit(mark: Mark | null): Record<Ask, boolean> {
  return {
    funnier: mark?.intent === 'funnier', shorter: mark?.intent === 'shorter', options: mark?.intent === 'options',
    note: !!mark && !!mark.note.trim(),
  };
}

/** A tap on Funnier, Shorter or 3 options with words selected. Not marked that way yet: they are (a mark is made, or
 *  the one on these words asks this way now). Already marked that way: the ask is taken off, and the mark goes with it
 *  unless it carries a note. `said` is the word for VoiceOver. */
export function toggleAsk(text: string, set: Mark[], target: { text: string; at: number }, ask: Exclude<Ask, 'note'>, id: string): { marks: Mark[]; said: string; error?: string } {
  const on = markOn(text, set, target);
  const word = ASKS.find((a) => a.k === ask)?.label || ask;
  if (on && on.intent === ask) {
    if (on.note.trim()) return { marks: set.map((m) => (m.id === on.id ? { ...m, intent: undefined } : m)), said: `${word} off. The note stays` };
    return { marks: set.filter((m) => m.id !== on.id), said: 'Mark removed' };
  }
  if (on) return { marks: set.map((m) => (m.id === on.id ? { ...m, intent: ask } : m)), said: `Marked: ${word}` };
  const r = addMark(text, set, { id, type: 'change', text: target.text, at: target.at, note: '', intent: ask });
  return r.error ? { marks: set, said: r.error, error: r.error } : { marks: r.marks, said: `Marked: ${word}` };
}

/** A tap on Note with words selected: the mark its note belongs to (made now, with no ask, when the words had none). */
export function markForNote(text: string, set: Mark[], target: { text: string; at: number }, id: string): { marks: Mark[]; mark: Mark | null; error?: string } {
  const on = markOn(text, set, target);
  if (on) return { marks: set, mark: on };
  const r = addMark(text, set, { id, type: 'change', text: target.text, at: target.at, note: '' });
  if (r.error) return { marks: set, mark: null, error: r.error };
  return { marks: r.marks, mark: r.marks.find((m) => m.id === id) || null };
}

/** His note on a mark, saved. A mark with no ask and no note says nothing, so it goes. */
export function saveNote(set: Mark[], id: string, note: string): Mark[] {
  const n = note.trim().slice(0, MAX_NOTE);
  return set.flatMap((m) => {
    if (m.id !== id) return [m];
    if (!n && m.type === 'change' && !m.intent) return [];
    return [{ ...m, note: m.type === 'keep' ? '' : n }];
  });
}

/** The sentence over "Get a new draft" (the mockup's words). */
export const marksLine = (n: number): string => `${markCount(n)} on this page. Anything you didn't mark stays word for word.`;

/** What the bar's main button is right now. `send` = his marks go out as one round; `choose` = nothing is marked yet
 *  (it asks: this paragraph, or the whole page); `open` = the draft (or why there is none). */
export type Punch = { label: string; does: 'send' | 'choose' | 'open' | 'none'; hint: string };
export function punch(o: { marks: number; round: RoundState; online: boolean; sending: boolean; empty: boolean }): Punch {
  if (o.sending) return { label: 'Sending…', does: 'none', hint: 'Your marks are on their way' };
  if (o.round === 'waiting') return { label: 'Writing…', does: 'open', hint: 'A new draft is being written. Open it to wait there' };
  if (o.marks > 0) return o.online ? { label: 'Get a new draft', does: 'send', hint: 'Sends your marks once; a whole new draft comes back' } : { label: 'Get a new draft', does: 'none', hint: 'Offline. Your marks wait on this iPhone' };
  if (o.round === 'ready') return { label: 'New draft ready', does: 'open', hint: 'Opens the new draft' };
  if (o.round === 'refused' || o.round === 'failed') return { label: 'No draft · why', does: 'open', hint: 'Says why no draft came, and puts your marks back' };
  if (o.empty) return { label: 'Punch it up', does: 'none', hint: 'Nothing on the page yet' };
  return o.online ? { label: 'Punch it up', does: 'choose', hint: 'Asks for a funnier draft of this paragraph or the whole page' } : { label: 'Punch it up', does: 'none', hint: 'Offline' };
}

/** What Punch it up can be on with nothing marked: the paragraph the caret is in (the lines around it up to a blank
 *  line), and the whole page; each null when it holds no words or is longer than one mark may quote. */
export function punchTargets(textRaw: string, caret: number | null): { paragraph: { text: string; at: number } | null; whole: { text: string; at: number } | null } {
  const text = (textRaw || '').replace(/\r\n?/g, '\n');
  const span = (a: number, b: number) => {
    while (a < b && /\s/.test(text[a])) a++;
    while (b > a && /\s/.test(text[b - 1])) b--;
    return a < b && [...text.slice(a, b)].length <= MAX_QUOTE ? { text: text.slice(a, b), at: a } : null;
  };
  const whole = span(0, text.length);
  const c = Math.max(0, Math.min(caret ?? text.length, text.length));
  // back to the blank line above, on to the blank line below
  const blank = /\n[ \t]*\n/g;
  let a = 0;
  let b = text.length;
  let m: RegExpExecArray | null;
  while ((m = blank.exec(text))) {
    if (m.index + m[0].length <= c) a = m.index + m[0].length;
    else if (m.index >= c) { b = m.index; break; }
    else { a = m.index + m[0].length; }
  }
  return { paragraph: span(a, b), whole };
}

/** The quiet word in the bar: "Saved" once his words are a version, "Saving…" while one is on its way, nothing while a
 *  reason is shown instead (offline, too long). */
export function savedWord(o: { pending: boolean; note: string | null; typed: boolean; versions: number }): string {
  if (o.note) return '';
  if (o.pending) return o.typed ? '' : 'Saving…';
  return o.versions > 0 ? 'Saved' : '';
}
