// A set's bullet notes and seams (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §3.3; Slice 4). Pure: what the
// notes editor under a bit holds, what Return, Backspace and "Add a note" do to it, when a save is due and which call
// makes it, what the snack says, the words of the seam between two bits, and what a Sets card says. Runs under node
// (tests/setNoteRules.test.mjs, with a --red arm).
//
// The rule that matters most: the words the app derives for a bit he has written nothing under (its setup and
// reveal, or his typed idea) are shown, never edited and never saved. His first note replaces them; until then the
// editor starts empty, so nothing the app picked is ever stored as a note of his.

/** The server's caps (`pk_creative::reducers::setnotes`). */
export const MAX_NOTES = 12;
export const MAX_NOTE_CHARS = 200;

export type NotesFrom = 'his' | 'derived';
export type NotedStep = { notes?: string[] | null; notes_from?: NotesFrom | null };
/** One bullet in the editor: a stable key (so a field keeps its focus when another is added or removed) and its text. */
export type Draft = { key: string; text: string };

/** A note as it is stored (the server's `clean_note`, the same class of characters, so a save never comes back
 *  different and asks to be saved again): control characters (a pasted newline, a tab), line and paragraph
 *  separators and direction overrides become spaces, the ends are trimmed, and it is cut at the cap. */
export function cleanNote(text: string): string {
  const flat = (text || '').replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029\u202a-\u202e\u2066-\u2069]/g, ' ').trim();
  return Array.from(flat).slice(0, MAX_NOTE_CHARS).join('').trim();
}

/** The list a save sends: each note cleaned, the blank ones dropped, at most the cap. */
export function cleanNotes(texts: readonly string[]): string[] {
  return texts.map(cleanNote).filter(Boolean).slice(0, MAX_NOTES);
}

export const sameNotes = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((x, i) => x === b[i]);

/** His own notes for a step: the served list when it is his, else none (derived words are not his notes). */
export function hisNotes(step: NotedStep | null | undefined): string[] {
  return step && step.notes_from === 'his' ? cleanNotes(step.notes || []) : [];
}

/** What is shown, read-only, while he has written nothing: the derived words. None once he has notes of his own. */
export function derivedNotes(step: NotedStep | null | undefined): string[] {
  return step && step.notes_from !== 'his' ? (step.notes || []).map((t) => (t || '').trim()).filter(Boolean) : [];
}

let seq = 0;
const draft = (text: string): Draft => ({ key: `n${++seq}`, text });

/** The editor's bullets when it opens: his notes, never the derived ones. */
export function editorNotes(step: NotedStep | null | undefined): Draft[] {
  return hisNotes(step).map(draft);
}

export const canAddNote = (drafts: readonly Draft[]) => drafts.length < MAX_NOTES;

export type Edit = {
  drafts: Draft[];
  /** The bullet that takes the cursor next; null = leave it where it is. */
  focus: string | null;
  /** He is finished with this bit's notes: the keyboard goes (Return on an empty bullet). */
  done?: boolean;
};

/** Return in bullet `key`: a new empty bullet under it, focused. Return on an empty bullet ends the writing (the one
 *  thumb way to put the keyboard away: Return, Return). At the cap nothing is added. */
export function onReturn(drafts: readonly Draft[], key: string): Edit {
  const i = drafts.findIndex((d) => d.key === key);
  if (i >= 0 && !cleanNote(drafts[i].text)) return { drafts: drafts.slice(), focus: null, done: true };
  if (i < 0 || !canAddNote(drafts)) return { drafts: drafts.slice(), focus: null };
  const added = draft('');
  return { drafts: [...drafts.slice(0, i + 1), added, ...drafts.slice(i + 1)], focus: added.key };
}

/** Backspace in an empty bullet: it goes, and the cursor goes to the one above (the one below when it was the first;
 *  nowhere when it was the only one). A bullet with words in it is not touched: Backspace deletes a letter as usual. */
export function onBackspace(drafts: readonly Draft[], key: string): Edit {
  const i = drafts.findIndex((d) => d.key === key);
  if (i < 0 || drafts[i].text !== '') return { drafts: drafts.slice(), focus: null };
  const rest = drafts.filter((d) => d.key !== key);
  return { drafts: rest, focus: rest.length ? rest[Math.max(0, i - 1)].key : null };
}

/** "Add a note" (and the first tap on a bit with no notes of his): an empty bullet at the end, focused; when the last
 *  one is already empty the cursor goes there instead; at the cap the last bullet takes the cursor. */
export function addNote(drafts: readonly Draft[]): Edit {
  const last = drafts[drafts.length - 1];
  if (last && (!cleanNote(last.text) || !canAddNote(drafts))) return { drafts: drafts.slice(), focus: last.key };
  const added = draft('');
  return { drafts: [...drafts, added], focus: added.key };
}

/** Typing in bullet `key`. A pasted block of several lines stays one note (the field is one line). */
export function onType(drafts: readonly Draft[], key: string, text: string): Draft[] {
  return drafts.map((d) => (d.key === key ? { ...d, text } : d));
}

/** When he leaves the notes of a bit: the blank bullets go. */
export function onLeave(drafts: readonly Draft[]): Draft[] {
  return drafts.filter((d) => cleanNote(d.text));
}

export type Call = { method: 'POST'; path: string; body: Record<string, unknown> };
const enc = encodeURIComponent;

/** The save that is due, or null when what he has typed is what is stored. `saved` is his notes as the server has
 *  them (never the derived ones). */
export function planSave(boardId: string, jokeId: string, saved: readonly string[], drafts: readonly Draft[]): Call | null {
  const notes = cleanNotes(drafts.map((d) => d.text));
  if (sameNotes(notes, saved)) return null;
  return { method: 'POST', path: `/api/boards/${enc(boardId)}/notes`, body: { joke_id: jokeId, notes } };
}

/** Undo of a save: the same call with the list it replaced (the server's `was`); null when there is nothing to put back. */
export function undoSave(boardId: string, jokeId: string, res: { notes?: string[]; was?: string[] } | null | undefined): Call | null {
  if (!res || !Array.isArray(res.was) || sameNotes(res.was, res.notes || [])) return null;
  return { method: 'POST', path: `/api/boards/${enc(boardId)}/notes`, body: { joke_id: jokeId, notes: res.was } };
}

/** `small` in order inside `big`. */
function within(small: readonly string[], big: readonly string[]): boolean {
  let i = 0;
  for (const x of big) if (i < small.length && x === small[i]) i++;
  return i === small.length;
}

/** What a save did, in a few words: the menu's untimed Undo is named after it ("Undo: Note added"). */
export function saveWords(was: readonly string[], now: readonly string[]): string {
  if (within(was, now)) {
    const n = now.length - was.length;
    return n === 1 ? 'Note added' : `${n} notes added`;
  }
  if (within(now, was)) {
    const n = was.length - now.length;
    return n === 1 ? 'Note removed' : `${n} notes removed`;
  }
  return 'Notes changed';
}

/** The snack after a save. Adding notes is quiet (he sees them where he typed them; the menu's Undo still takes them
 *  back); a note that went or changed says so, and that snack carries Undo. */
export function saveSnack(was: readonly string[], now: readonly string[]): string {
  return within(was, now) ? '' : saveWords(was, now);
}

/** VoiceOver's words for one bullet's field. */
export function noteLabel(j: number, n: number, name: string): string {
  return `Note ${j + 1} of ${n} for ${name}`;
}

// ------------------------------------------------------------------------------------------------ the seam

export type SameShape = { source: 'his' | 'ai'; shared: string[]; edge: string | null };
export type TransitionIn = {
  words: string; weak: boolean; kind: 'proven' | 'link' | 'jump'; from: string; to: string; same_shape?: SameShape | null;
  /** Whose tie the words name: his (he did it, wrote it, made or confirmed it), the AI's reading, or none to name. */
  source?: 'his' | 'ai' | null;
};
export type Seam = {
  /** good = a tie that is his (he did it, wrote it or confirmed it); warn = worth a look; none = nothing of his ties
   *  them yet (no tie at all, or only the app's reading: that never gets the check). Never colour alone: `mark` too. */
  tone: 'good' | 'warn' | 'none';
  mark: 'check' | 'warn' | 'none';
  label: string;
  detail: string;
  /** "Draft a bridge" is offered (the two have no line between them). */
  bridge: boolean;
  /** The tie is the app's reading, not something he said or did. */
  ai: boolean;
  key: string;
};

const DETAIL: Record<string, string> = {
  'Done back to back on stage': 'You have done these two back to back on stage.',
  'Your segue': 'You wrote a way from one into the other.',
  'Could lead in': 'There is a suggested way from one into the other. It is not yours until you keep it.',
  Callback: 'One of these calls back to the other.',
  'Same premise': 'These two come from the same premise.',
  Linked: 'These two are linked in your notebook.',
  'No link yet': 'Nothing ties these two together yet.',
};

/** The seam between a bit and the one before it, from the build's `transition_in`; null for the first bit. "Same
 *  shape twice" wins the label: it is the one thing here that asks him to move something. A tie that is only the
 *  app's reading keeps its name, carries the AI mark and gets no check: the check is for what he did or said. */
export function seamOf(t: TransitionIn | null | undefined): Seam | null {
  if (!t) return null;
  const key = `${t.from}>${t.to}`;
  const words = (t.words || '').trim();
  if (t.same_shape) {
    const moves = (t.same_shape.shared || []).filter(Boolean);
    const first = t.same_shape.source === 'his' ? 'You said these two have the same shape.'
      : moves.length ? `Both use ${moves.join(', ')}.` : 'These two turn the same way.';
    return { tone: 'warn', mark: 'warn', label: 'Same shape twice', detail: `${first} Back to back, the second one usually gets a smaller laugh.`,
      bridge: !!t.weak, ai: t.same_shape.source !== 'his', key };
  }
  if (t.weak) {
    return { tone: 'none', mark: 'none', label: 'No bridge yet', detail: DETAIL[words] || (words ? `${words}.` : DETAIL['No link yet']), bridge: true, ai: false, key };
  }
  const label = t.kind === 'proven' ? 'Usual order' : words || 'Linked';
  const ai = t.kind !== 'proven' && (t.source === 'ai' || words === 'Could lead in');
  return { tone: ai ? 'none' : 'good', mark: ai ? 'none' : 'check', label, detail: DETAIL[words] || DETAIL[label] || '', bridge: false, ai, key };
}

/** VoiceOver's words for the seam row. */
export function seamLabel(s: Seam, before: string, after: string): string {
  return `Between ${before} and ${after}: ${s.label}${s.ai ? ', suggested by AI' : ''}`;
}

// ------------------------------------------------------------------------------------------------ a Sets card

/** "4 bits · 6:10" on a card; "Empty" for a list with nothing on it. */
export function cardMeta(n: number, runtime: string): string {
  if (!n) return 'Empty';
  return `${n} ${n === 1 ? 'bit' : 'bits'} · ${runtime}`;
}

/** The bits' names on one run, in order: "Roommate · Smart fridge · GPS". Empty for an empty list. */
export function cardNames(names: readonly string[] | null | undefined): string {
  return (names || []).map((n) => (n || '').trim()).filter(Boolean).join(' · ');
}

/** A card's last line: "Tonight · 7:00 slot · Edited Nov 16" (each part only when there is one). `slot` and `edited`
 *  come formatted (rules/format). The date is when he last touched the list, and says so: it is not a show date. */
export function cardFoot(current: boolean, slot: string | null, edited: string | null): string {
  return [current ? 'Tonight' : '', slot ? `${slot} slot` : '', edited ? `Edited ${edited}` : ''].filter(Boolean).join(' · ');
}

// ------------------------------------------------------------------------------------------------ Undo last change

/** The list's menu item that takes back the last change made on this screen, with no clock on it (HIG.md §7: the
 *  snack's Undo is timed; this one is not). `what` is the snack's own words for the change. */
export function undoLabel(what: string | null | undefined): string {
  const w = (what || '').trim();
  return w ? `Undo: ${w}` : 'Undo last change';
}
