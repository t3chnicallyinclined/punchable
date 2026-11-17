// Around this bit, In these sets and the Add to a set picker (docs/design/SPEC-2026-11-16-bits-sets-tapes.md §3.1,
// §4.5; Slice 1). Pure: what a card says, which buttons its half sheet offers, the one call each button makes and the
// call that undoes it, where a pulled line lands, and the picker's staged ticks turned into calls. The server decides
// what is on the page (GET /api/jokes/<id>/around); nothing here adds or hides a card.
//
// The rule that matters most: "Not connected" must reach the right place. A card with a stored edge rejects the edge
// (POST /api/decision); a note filed under the bit is taken off (POST /api/merge/undo); every other card is a
// computed suggestion with nothing to reject, so the pair is remembered (POST /api/links/dismiss). Each has an Undo.
// tests/aroundRules.test.mjs holds the cases and a --red arm for that rule.
import { inBit, insertAfter } from './bitLines.ts';
import type { DocEdit } from './lineRules.ts';

export type GroupKey = 'same_idea' | 'callbacks' | 'on_stage' | 'loose' | 'same_shape';
export type AroundKind = 'bit' | 'line' | 'moment' | 'wording' | 'note' | 'idea';
export type Does = 'merge' | 'add' | 'use' | 'connect' | 'link' | 'attach' | 'dismiss';
export type EdgeToAdd = { kind: string; src: string; dst: string };
export type AroundItem = {
  id: string; kind: AroundKind; ref: string; title: string; words: string[]; meta: string; why: string;
  source: 'his' | 'ai'; edge: string | null; line: { i: number; text: string } | null; does: Does[];
  /** The edge "Connect" adds when the card has none. */ connect?: EdgeToAdd;
  /** The record that filed a note under this bit (POST /api/merge/undo takes it off). */ attach?: string;
};
export type AroundGroup = { key: GroupKey; title: string; items: AroundItem[] };
export type AroundSet = { board: string; title: string; position: number; count: number; notes: string[] };
export type Around = { joke: string; unit: string | null; groups: AroundGroup[]; sets: AroundSet[] };

export type Call = { method: 'POST' | 'PATCH'; path: string; body: Record<string, unknown> };
/** What a button does: one call, or a new version of the bit (the page's own write), or nothing with its reason. */
export type Plan =
  | { kind: 'call'; call: Call }
  /** `lines` = the bit's lines after it; `edit` = the same change as one edit of the document, which keeps the blank
   *  lines between his ideas where they were (rules/lineRules applyEdit). */
  | { kind: 'version'; lines: string[]; reason: string; edit: DocEdit }
  | { kind: 'none'; why: string };

const enc = encodeURIComponent;
const KNOWN: Does[] = ['merge', 'add', 'use', 'connect', 'link', 'attach', 'dismiss'];

/** The groups that hold something, in the server's order. */
export function shownGroups(a: Around | null | undefined): AroundGroup[] {
  return (a?.groups || []).filter((g) => g.items.length > 0);
}
/** How many cards are around the bit. */
export function aroundCount(a: Around | null | undefined): number {
  return (a?.groups || []).reduce((n, g) => n + g.items.length, 0);
}
/** A card by its id, with the group it is in. */
export function findItem(a: Around | null | undefined, id: string): { group: AroundGroup; item: AroundItem } | null {
  for (const group of a?.groups || []) {
    const item = group.items.find((x) => x.id === id);
    if (item) return { group, item };
  }
  return null;
}
/** The section rows' counts. */
export const countMeta = (n: number): string => String(Math.max(0, n));
/** "bit 2 of 6" on a set row. */
export function setPlace(s: { position: number; count: number }): string {
  return `bit ${s.position} of ${s.count}`;
}
/** A set row's second line: his notes for the bit in that set, or nothing (there are none until sets have notes). */
export function setNotesLine(s: { notes?: string[] | null }): string {
  return (s.notes || []).map((x) => x.trim()).filter(Boolean).join(' · ');
}

/** The button's word. No Yes / No (HIG.md §9): the verb says what happens. */
export function actionLabel(item: Pick<AroundItem, 'kind'>, action: Does): string {
  switch (action) {
    case 'merge': return 'Merge into this bit';
    case 'add': return 'Add this line';
    case 'use': return 'Use this wording';
    case 'connect': return 'Connect';
    case 'link': return 'This tape is this bit';
    case 'attach': return 'File under this bit';
    case 'dismiss': return item.kind === 'note' ? 'Not this bit' : 'Not connected';
  }
}
/** The buttons of a card's half sheet, in the server's order; an action this build does not know is left out. */
export function actionsFor(item: AroundItem): { key: Does; label: string; quiet: boolean }[] {
  return item.does.filter((d) => KNOWN.includes(d)).map((key) => ({ key, label: actionLabel(item, key), quiet: key === 'dismiss' }));
}
/** Whether the action writes a line into the bit, so the sheet shows the line picker and where it lands. */
export const writesLine = (action: Does): boolean => action === 'add' || action === 'use';

/** The line of the bit a card belongs to, found by its WORDS (lines have no ids, B1): the server's index when that
 *  line still says the same, else the line that says it now, else none (the bit changed under the card). */
export function anchorIndex(lines: string[], item: Pick<AroundItem, 'line'>): number | null {
  const a = item.line;
  if (!a) return null;
  if (lines[a.i] === a.text) return a.i;
  const at = lines.indexOf(a.text);
  return at >= 0 ? at : null;
}
/** The line of a card he picked (the first when the pick is past the end). */
const pickedWords = (item: Pick<AroundItem, 'words'>, pick: number): string => (item.words[pick] ?? item.words[0] ?? '').trim();
/** "Add this line": the picked line goes after the line the card belongs to, else at the end. Null when the bit
 *  already says it (nothing to add). */
export function pullLine(lines: string[], item: AroundItem, pick: number): string[] | null {
  const words = (item.words[pick] ?? item.words[0] ?? '').trim();
  if (!words || inBit(lines, words)) return null;
  return insertAfter(lines, anchorIndex(lines, item), words);
}
/** "Use this wording": the picked line takes the place of the line the card belongs to; with no such line it is
 *  added at the end. Null when the bit already says it. */
export function swapWording(lines: string[], item: AroundItem, pick: number): string[] | null {
  const words = (item.words[pick] ?? item.words[0] ?? '').trim();
  if (!words || inBit(lines, words)) return null;
  const at = anchorIndex(lines, item);
  if (at == null) return [...lines, words];
  return lines.map((l, i) => (i === at ? words : l));
}

/** The one thing a button does. `lines` = the bit's newest lines, `pick` = the line of the card he tapped. */
export function planFor(bit: string, group: Pick<AroundGroup, 'key' | 'title'>, item: AroundItem, action: Does, lines: string[], pick = 0): Plan {
  const decision = (body: Record<string, unknown>): Plan => ({ kind: 'call', call: { method: 'POST', path: '/api/decision', body } });
  switch (action) {
    case 'merge':
      return { kind: 'call', call: { method: 'POST', path: '/api/merge', body: { main: bit, others: [item.ref], reason: `merged from Around this bit (${group.title})` } } };
    case 'attach':
      return { kind: 'call', call: { method: 'POST', path: '/api/merge/attach', body: { note: item.ref, into: bit } } };
    case 'connect':
    case 'link':
      if (item.edge) return decision({ target: 'edge', id: item.edge, action: 'confirm' });
      if (!item.connect) return { kind: 'none', why: 'Nothing to connect' };
      // the decision route wants a reason on an added edge: the group's name
      return decision({ target: 'edge', action: 'add', after: item.connect, reason: group.title });
    case 'dismiss':
      if (item.edge) return decision({ target: 'edge', id: item.edge, action: 'reject' });
      if (item.attach) return { kind: 'call', call: { method: 'POST', path: '/api/merge/undo', body: { merge_id: item.attach } } };
      return { kind: 'call', call: { method: 'POST', path: '/api/links/dismiss', body: { a: bit, b: item.ref, group: group.key } } };
    case 'add': {
      const next = pullLine(lines, item, pick);
      return next ? { kind: 'version', lines: next, reason: `pulled in from ${group.title}`, edit: { op: 'insert', at: anchorIndex(lines, item), words: pickedWords(item, pick) } } : { kind: 'none', why: 'The bit already says that' };
    }
    case 'use': {
      const next = swapWording(lines, item, pick);
      const at = anchorIndex(lines, item);
      const words = pickedWords(item, pick);
      return next ? { kind: 'version', lines: next, reason: 'the wording from the stage', edit: at == null ? { op: 'insert', at: null, words } : { op: 'replace', at, words } } : { kind: 'none', why: 'The bit already says that' };
    }
  }
}

/** The call that undoes a call's answer, or null when there is none. `answer` = the JSON the call returned. */
export function undoFor(bit: string, item: Pick<AroundItem, 'ref'>, call: Call, answer: any): Call | null {
  const post = (path: string, body: Record<string, unknown>): Call => ({ method: 'POST', path, body });
  switch (call.path) {
    case '/api/decision':
      return Array.isArray(answer?.indexes) && answer.indexes.length ? post('/api/undo', { indexes: answer.indexes }) : null;
    case '/api/merge':
    case '/api/merge/attach':
      return answer?.merge_id ? post('/api/merge/undo', { merge_id: answer.merge_id }) : null;
    case '/api/links/dismiss':
      return answer?.id ? post('/api/links/undismiss', { id: answer.id }) : null;
    case '/api/merge/undo':
      // a note taken off the bit goes back under it
      return post('/api/merge/attach', { note: item.ref, into: bit });
    default:
      return null;
  }
}

/** The snack after a button did its work. */
export function doneWord(item: Pick<AroundItem, 'kind' | 'title'>, action: Does): string {
  switch (action) {
    case 'merge': return `${item.title} is now part of this bit`;
    case 'add': return 'Line added';
    case 'use': return 'Line changed';
    case 'connect': return 'Connected';
    case 'link': return 'Linked: that tape is this bit';
    case 'attach': return 'Filed under this bit';
    case 'dismiss': return item.kind === 'note' ? 'Taken off this bit' : 'Not connected';
  }
}

// ------------------------------------------------------------------ Around this bit, drawn on the page itself
// (owner on build 28: "when i open a bit i dont automatically see other connections"). Under the document each group
// that holds something shows its header and its first cards; the rest are one tap away ("See all 5"). The server
// decides the order of the groups and of the cards in each; nothing here re-sorts, adds or hides a card.

/** How many cards of a group the page draws before "See all". */
export const INLINE_CAP = 2;
export type InlineGroup = {
  key: GroupKey; title: string;
  /** The cards drawn on the page: the group's first ones, in the server's order. */ cards: AroundItem[];
  /** Every card in the group. */ total: number;
  /** The count "See all" names; 0 when the page already shows them all (no such row). */ more: number;
};
/** The groups as the page draws them: an empty group is not there, a group shows at most `cap` cards. */
export function inlineGroups(a: Around | null | undefined, cap = INLINE_CAP): InlineGroup[] {
  const n = Math.max(1, Math.floor(cap));
  return shownGroups(a).map((g) => ({ key: g.key, title: g.title, cards: g.items.slice(0, n), total: g.items.length, more: g.items.length > n ? g.items.length : 0 }));
}
/** "See all 5". */
export const seeAllLabel = (n: number): string => `See all ${n}`;
/** One group by its key (the sheet behind "See all"). */
export function findGroup(a: Around | null | undefined, key: string): AroundGroup | null {
  return (a?.groups || []).find((g) => g.key === key && g.items.length > 0) || null;
}
/** The one button a card carries on the page: its first action that is not the quiet one. `direct` = one tap does it
 *  from the page; false when he must first pick which of the card's lines he means (the sheet has the picker). A card
 *  with no such action has no button (its tap opens the sheet). */
export function inlineAction(item: AroundItem): { key: Does; label: string; direct: boolean } | null {
  const a = actionsFor(item).find((x) => !x.quiet);
  if (!a) return null;
  return { key: a.key, label: a.label, direct: !(writesLine(a.key) && item.words.length > 1) };
}

// ------------------------------------------------------------------ Add to a set: the multi-tick picker

export type PickBoard = { id: string; title: string; n?: number | null; runtime?: string | null };
export type PickRow = { id: string; title: string; sub: string; on: boolean };

/** The picker's rows: his live lists in the order served, ticked where the bit already is. */
export function pickerRows(boards: readonly PickBoard[], ticked: ReadonlySet<string>): PickRow[] {
  return boards.map((b) => ({
    id: b.id, title: b.title,
    sub: [b.n == null ? '' : `${b.n} ${b.n === 1 ? 'bit' : 'bits'}`, b.runtime || ''].filter(Boolean).join(' · '),
    on: ticked.has(b.id),
  }));
}
/** A tap on a row: the tick flips. */
export function toggled(ticked: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(ticked);
  if (!next.delete(id)) next.add(id);
  return next;
}
/** What Done must write: the sets ticked now that were not, and the ones unticked. */
export function stagedDiff(was: ReadonlySet<string>, now: ReadonlySet<string>): { add: string[]; remove: string[] } {
  return { add: [...now].filter((x) => !was.has(x)), remove: [...was].filter((x) => !now.has(x)) };
}
/** The name of a new set as it is sent: trimmed, at most 200 characters (the route's limit); empty = none. */
export function newSetTitle(typed: string): string {
  return typed.trim().slice(0, 200);
}
/** Whether Done has anything to write. */
export function hasChanges(diff: { add: string[]; remove: string[] }, typed: string): boolean {
  return diff.add.length > 0 || diff.remove.length > 0 || newSetTitle(typed).length > 0;
}
/** Done as calls, in the order they are sent: the new set (made with the bit in it), each add, each remove. */
export function pickerCalls(joke: string, diff: { add: string[]; remove: string[] }, typed = '', note = ''): Call[] {
  const title = newSetTitle(typed);
  // words he took from the bit (Slice 3, Take it elsewhere) go under it as a note in each set it is added to
  const n = note.trim();
  return [
    ...(title ? [{ method: 'POST' as const, path: '/api/boards', body: { title, joke_ids: [joke] } }] : []),
    ...diff.add.map((b) => ({ method: 'POST' as const, path: `/api/boards/${enc(b)}/items`, body: n ? { joke_id: joke, note: n } : { joke_id: joke } })),
    ...diff.remove.map((b) => ({ method: 'PATCH' as const, path: `/api/boards/${enc(b)}`, body: { remove: { joke_id: joke } } })),
  ];
}
/** What Done actually wrote (a call that failed is in none of them). */
export type PickDone = { added: string[]; removed: string[]; created: string[] };
/** Undo as calls: the bit comes off what it was added to, a set made here is deleted, and it goes back where it was
 *  taken from, at the place it sat (`places`: board -> its 1-based position before). */
export function pickerUndo(joke: string, done: PickDone, places: Record<string, number> = {}): Call[] {
  return [
    ...done.added.map((b) => ({ method: 'PATCH' as const, path: `/api/boards/${enc(b)}`, body: { remove: { joke_id: joke } } })),
    ...done.created.map((b) => ({ method: 'PATCH' as const, path: `/api/boards/${enc(b)}`, body: { delete: true } })),
    ...done.removed.map((b) => ({
      method: 'PATCH' as const, path: `/api/boards/${enc(b)}`,
      body: { add: { joke_id: joke, ...(places[b] >= 1 ? { at: places[b] - 1 } : {}) } },
    })),
  ];
}
/** The one snack after Done: "Added to Friday late show", "Added to 2 sets", "Removed from 1 set", or both. */
export function pickerSnack(done: PickDone, titles: Record<string, string> = {}): string {
  const plus = [...done.created, ...done.added];
  const minus = done.removed;
  const sets = (n: number) => `${n} ${n === 1 ? 'set' : 'sets'}`;
  const name = (ids: string[]) => (ids.length === 1 && titles[ids[0]] ? titles[ids[0]] : sets(ids.length));
  if (plus.length && minus.length) return `Added to ${sets(plus.length)} · removed from ${sets(minus.length)}`;
  if (plus.length) return `Added to ${name(plus)}`;
  if (minus.length) return `Removed from ${name(minus)}`;
  return '';
}
