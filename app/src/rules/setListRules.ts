// Set lists: which are his, which the app made, which are put away, the words on the drafts row, the one list "To a
// set" may write to, and what the notes sheet shows for a bit (docs/design/SPEC-2026-10-29-set-lists-and-stage-notes.md
// §1.2, §2.1). Pure: no imports, runs under node (tests/setListRules.test.mjs). The screen never says "inferred",
// "seed" or "AI": a machine-made list is a "draft", named after where it came from ("from your Insights").

export type ListLike = {
  id: string; title?: string; source?: string | null; archived?: boolean | null; by?: string | null;
  seed?: { from?: string | null } | null;
};

/** `archived` first; then the app's drafts (`source` "inferred"); the rest is his. No `source` = his: his wins when unsure. */
export function splitLists<T extends ListLike>(boards: readonly T[]): { his: T[]; drafts: T[]; archived: T[] } {
  const his: T[] = [];
  const drafts: T[] = [];
  const archived: T[] = [];
  for (const b of boards) (b.archived === true ? archived : b.source === 'inferred' ? drafts : his).push(b);
  return { his, drafts, archived };
}

/** "insights.chunks" -> "Insights": the first word of the server's seed name, capitalised. Nothing else leaks. */
export function seedWord(from?: string | null): string | null {
  const w = (from || '').split(/[.\s:/_-]/)[0].trim();
  return w ? w[0].toUpperCase() + w.slice(1).toLowerCase() : null;
}

/** The disclosure row's label: "15 drafts from your Insights" (one seed), "15 drafts the app made" (mixed or none),
 *  "1 draft from your Insights", "A sample list" (the demo list alone). Empty string for no drafts. */
export function draftsLabel(drafts: readonly ListLike[]): string {
  const n = drafts.length;
  if (n === 0) return '';
  if (n === 1 && drafts[0].by === 'ai:demo') return 'A sample list';
  const words = new Set(drafts.map((d) => seedWord(d.seed?.from)));
  const one = words.size === 1 ? [...words][0] : null;
  const noun = n === 1 ? 'draft' : 'drafts';
  return one ? `${n} ${noun} from your ${one}` : `${n} ${noun} the app made`;
}

/** The list a bit's "To a set" writes to: the current one when it is his and live; else null = open the picker.
 *  Never a draft, never an archived list, never the newest one by default (a write he did not choose). */
export function addTarget<T extends ListLike>(current: { id: string } | null | undefined, boards: readonly T[]): T | null {
  if (!current) return null;
  return splitLists(boards).his.find((b) => b.id === current.id) || null;
}

/** The picker's rows: his live lists, in the order served. */
export function pickerLists<T extends ListLike>(boards: readonly T[]): T[] {
  return splitLists(boards).his;
}

export type NoteStep = {
  quote?: string | null; cut_start?: boolean; cut_end?: boolean; idea?: string | null;
  text?: { setup?: string | null; reveal?: string | null; from?: unknown } | null;
};
export type Notes = { kind: 'written' | 'idea' | 'quote' | 'split' | 'empty'; setup: string | null; reveal: string | null; idea: string | null };
const NONE: Notes = { kind: 'empty', setup: null, reveal: null, idea: null };

/** What the notes sheet shows for a bit: the written version (`text` with a `from`: setup, reveal) -> else his typed
 *  idea whole (the server's `text` is then its guessed split, `from` null) -> else his taped line as the body -> else
 *  the unattributed split -> else nothing ("No words saved for this bit yet."). */
export function notesFor(step: NoteStep | null | undefined): Notes {
  if (!step) return NONE;
  const setup = step.text?.setup?.trim() || null;
  const reveal = step.text?.reveal?.trim() || null;
  const written = step.text != null && step.text.from != null;
  if (written && (setup || reveal)) return { kind: 'written', setup, reveal, idea: null };
  const idea = step.idea?.trim() || null;
  if (idea) return { kind: 'idea', setup: null, reveal: null, idea };
  const q = step.quote?.trim();
  if (q) return { kind: 'quote', setup: `${step.cut_start ? '…' : ''}${q}${step.cut_end ? '…' : ''}`, reveal: null, idea: null };
  if (setup || reveal) return { kind: 'split', setup, reveal, idea: null };
  return NONE;
}

/** The sheet footer's word: on stage "Next bit" / "Last bit"; from a set list "Open the bit". */
export function notesFooter(fromStage: boolean, index: number, n: number): { label: string; disabled: boolean } {
  if (!fromStage) return { label: 'Open the bit', disabled: n === 0 };
  const last = n === 0 || index >= n - 1;
  return { label: last ? 'Last bit' : 'Next bit', disabled: last };
}

/** VoiceOver's words for a stage row: "3 of 7, Kale, current. Notes" ("done" for a crossed-out row). */
export function stageRowLabel(i: number, n: number, name: string, cur: number): string {
  const state = i === cur ? ', current' : i < cur ? ', done' : '';
  return `${i + 1} of ${n}, ${name}${state}. Notes`;
}
