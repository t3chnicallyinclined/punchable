// One notebook (docs/design/SPEC-2026-11-17-one-notebook.md screen 1 and §3): the app opens on ONE list of every bit,
// set and tape, newest first, grouped by day, with a filter (All · Bits · Sets · Tapes), a Pinned section and a search
// over all of it. A row is a title, one line under it, and a status line only when there is something to say ("New
// draft ready", "2 marks · 1 tape"). These rules are pure: what a row says, where it sorts, which day it is under,
// what the filter and the search leave, and where each old tab's URL lands. No React: tests/notebookRules.test.mjs
// runs them under node, with a --red arm per rule.

import { isFreshBit } from './bitWorkRules.ts';

export type Filter = 'all' | 'bits' | 'sets' | 'tapes';
export const FILTERS: [Filter, string][] = [['all', 'All'], ['bits', 'Bits'], ['sets', 'Sets'], ['tapes', 'Tapes']];
export type Kind = 'bit' | 'set' | 'tape' | 'topic';

/** One row of the notebook. */
export type Row = {
  /** `bit:<id>` … : what a pin names. */ key: string;
  kind: Kind; id: string; title: string;
  /** One line under the title: a sentence of his from the page, or what the row is. */ line: string;
  /** When it was last touched (an ISO time or a date): what the list is sorted by. */ at: string;
  /** Something to say about it, or nothing. `hot` = it is waiting for him (drawn in the accent, never colour alone). */
  status: string | null; hot: boolean;
  /** Where a tap goes; null when the row draws itself (a recording kept on this phone, with its own controls). */
  href: string | null;
  /** A recording kept on this phone (lib/recordings), by id. */ rec?: string;
  /** The word at the row's end when the list mixes kinds ("Set", "Tape"). */ tag: string | null;
};

const enc = encodeURIComponent;
const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "about 7 min", "under a minute". */
export function aboutMin(seconds: number | null | undefined): string | null {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return null;
  const m = Math.round(seconds / 60);
  return m < 1 ? 'under a minute' : `about ${m} min`;
}

/** A bit's status line: a draft that waits beats everything; a draft being written says so; else his marks and the
 *  tapes of it on this phone. Null when there is nothing to say (most rows). */
export function bitStatus(o: { round?: 'waiting' | 'ready' | 'refused' | 'failed' | null; marks?: number; tapes?: number }): { text: string; hot: boolean } | null {
  if (o.round === 'ready') return { text: 'New draft ready', hot: true };
  if (o.round === 'waiting') return { text: 'Writing a new draft…', hot: false };
  const parts: string[] = [];
  if (o.marks) parts.push(plural(o.marks, 'mark', 'marks'));
  if (o.tapes) parts.push(plural(o.tapes, 'tape', 'tapes'));
  if (o.round === 'refused' || o.round === 'failed') parts.unshift('No draft came');
  return parts.length ? { text: parts.join(' · '), hot: false } : null;
}

/** A bit as the server lists it (GET /api/jokes; pk-wire JokeSummary). `punch` is a sentence of the newest version,
 *  an exact substring: his words. `summary` is shown only on a bit he typed in himself (`yours`): it is then his idea
 *  as he wrote it; on any other bit it may be the app's wording, and the row never passes that off as his. */
export type BitLike = {
  id: string; title: string; short_title?: string | null; short_name?: string | null; summary?: string | null; punch?: string | null; yours?: boolean;
  touched?: string | null; last_seen?: string | null; last_performed?: string | null; first_seen?: string | null;
};
const head = (s: string | null | undefined, n: number) => (s || '').slice(0, n);
/** The server's own "recent" key (pk-api jokes.rs recent_key): the newest of his last touch, the newest version, the
 *  last night on stage and the first day. */
export function bitAt(j: BitLike): string {
  return [head(j.touched, 19), head(j.last_seen, 10), head(j.last_performed, 10), head(j.first_seen, 10)].sort().pop() || '';
}
export function bitRow(j: BitLike, o: { round?: 'waiting' | 'ready' | 'refused' | 'failed' | null; sentAt?: string | null; marks?: number; tapes?: number } = {}): Row {
  const st = bitStatus(o);
  const at = bitAt(j);
  // a page made by the pencil and not named yet carries a marker title (rules/bitWorkRules freshBitTitle)
  const title = isFreshBit(j.title) ? 'New page' : (j.title || '').split(' (')[0] || j.short_title || j.short_name || 'Untitled';
  return {
    key: `bit:${j.id}`, kind: 'bit', id: j.id, title,
    line: (j.punch || (j.yours ? j.summary : '') || '').replace(/\s+/g, ' ').trim(),
    // a draft he asked for lifts its page to when he asked: it is the newest thing about it
    at: o.sentAt && o.sentAt > at ? o.sentAt : at,
    status: st?.text ?? null, hot: !!st?.hot,
    // the page itself, also when a draft waits: its main button then reads "New draft ready" and opens the sheet over it
    href: `/joke/${enc(j.id)}`, tag: null,
  };
}

/** A set as the server lists it (GET /api/boards). */
export type SetLike = { id: string; title: string; n: number; runtime_s?: number | null; updated?: string | null };
export function setRow(b: SetLike, lastTaped?: string | null): Row {
  const parts = [plural(b.n, 'bit', 'bits'), aboutMin(b.runtime_s), lastTaped ? `last taped ${lastTaped}` : null].filter(Boolean);
  return { key: `set:${b.id}`, kind: 'set', id: b.id, title: b.title || 'Untitled set', line: parts.join(' · '), at: b.updated || '', status: null, hot: false, href: `/list/${enc(b.id)}`, tag: 'Set' };
}

/** A topic of the old Rant tab that no set was made from yet (rules/tapesRules topicsNotYetSets). */
export function topicRow(t: { id: string; text: string; touched?: string | null; created?: string | null }): Row {
  return { key: `topic:${t.id}`, kind: 'topic', id: t.id, title: t.text || 'Untitled topic', line: 'A topic · not a set yet', at: t.touched || t.created || '', status: null, hot: false, href: `/topic/${enc(t.id)}`, tag: 'Set' };
}

/** A tape: a recording kept on this phone (it draws itself), or a set the server holds. */
export function tapeRow(t: { from: 'phone'; id: string; title: string; at: string; kind: string } | { from: 'server'; id: string; title: string; at: string; line: string }): Row {
  if (t.from === 'phone') return { key: `rec:${t.id}`, kind: 'tape', id: t.id, title: t.title, line: t.kind, at: t.at, status: null, hot: false, href: null, rec: t.id, tag: 'Tape' };
  return { key: `tape:${t.id}`, kind: 'tape', id: t.id, title: t.title, line: t.line, at: t.at, status: null, hot: false, href: `/set/${enc(t.id)}`, tag: 'Tape' };
}

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
/** The local day a time falls on. A bare date (2026-11-14) is that day here, wherever "here" is. */
function dayOf(at: string): Date | null {
  if (!at) return null;
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(at);
  const d = m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])) : new Date(at);
  if (Number.isNaN(d.getTime())) return null;
  return new Date(d.getFullYear(), d.getMonth(), d.getDate());
}

/** The heading a row sits under: Today, Yesterday, This week (the five days before that), This month, then the
 *  month's name (with the year once it is not this year), Earlier for a row with no date. */
export function dayGroup(at: string, now: Date): string {
  const d = dayOf(at);
  if (!d) return 'Earlier';
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = Math.round((today.getTime() - d.getTime()) / 86400000);
  if (days <= 0) return 'Today';
  if (days === 1) return 'Yesterday';
  if (days < 7) return 'This week';
  if (d.getFullYear() === today.getFullYear() && d.getMonth() === today.getMonth()) return 'This month';
  return d.getFullYear() === today.getFullYear() ? MONTHS[d.getMonth()] : `${MONTHS[d.getMonth()]} ${d.getFullYear()}`;
}

const kindOf: Record<Exclude<Filter, 'all'>, Kind[]> = { bits: ['bit'], sets: ['set', 'topic'], tapes: ['tape'] };
const words = (s: string) => s.toLowerCase().replace(/\s+/g, ' ').trim();

/** The list as drawn: what the filter leaves, a search over each row's title and line (`q`; the server's own search
 *  adds what the words of a page or a tape hold), the pinned rows first in their own section and never again below,
 *  then the rest newest first under their day. */
export function notebook(rows: readonly Row[], o: { filter: Filter; q?: string; pins?: readonly string[]; now: Date }): { pinned: Row[]; groups: { title: string; rows: Row[] }[]; count: number } {
  const q = words(o.q || '');
  const kept = rows
    .filter((r) => o.filter === 'all' || kindOf[o.filter].includes(r.kind))
    .filter((r) => !q || words(`${r.title} ${r.line}`).includes(q));
  const sorted = [...kept].sort((a, b) => (b.at || '').localeCompare(a.at || '') || a.key.localeCompare(b.key));
  const pins = new Set(q ? [] : o.pins || []);
  const pinned = sorted.filter((r) => pins.has(r.key));
  const groups: { title: string; rows: Row[] }[] = [];
  for (const r of sorted) {
    if (pins.has(r.key)) continue;
    const title = dayGroup(r.at, o.now);
    const last = groups[groups.length - 1];
    if (last && last.title === title) last.rows.push(r); else groups.push({ title, rows: [r] });
  }
  return { pinned, groups, count: kept.length };
}

/** A search: the pages the server found, in its order (by meaning and by word), then the other rows whose title or
 *  line holds the words, newest first; each only when the filter shows its kind. */
export function found(bits: readonly Row[], others: readonly Row[], q: string, filter: Filter): Row[] {
  const w = words(q);
  const show = (r: Row) => filter === 'all' || kindOf[filter].includes(r.kind);
  const rest = others.filter((r) => show(r) && !!w && words(`${r.title} ${r.line}`).includes(w))
    .sort((a, b) => (b.at || '').localeCompare(a.at || '') || a.key.localeCompare(b.key));
  return [...bits.filter(show), ...rest];
}

/** "42 pages" under the list (a page is a bit or a set; with a filter on, its own word). */
export function countLabel(n: number, filter: Filter): string {
  if (filter === 'bits') return plural(n, 'bit', 'bits');
  if (filter === 'sets') return plural(n, 'set', 'sets');
  if (filter === 'tapes') return plural(n, 'tape', 'tapes');
  return plural(n, 'page', 'pages');
}

/** Pin or unpin (the newest pin first). */
export function togglePin(pins: readonly string[], key: string): string[] {
  return pins.includes(key) ? pins.filter((k) => k !== key) : [key, ...pins];
}

/** `/?show=bits`: the filter an old tab's URL asks for; anything else is All. */
export function showOf(param: string | string[] | null | undefined): Filter {
  const v = Array.isArray(param) ? param[0] : param;
  return v === 'bits' || v === 'sets' || v === 'tapes' ? v : 'all';
}

/** Where each old tab root lands: the notebook, with that tab's filter on. */
export const OLD_ROOTS: Record<string, string> = { '/bits': '/?show=bits', '/sets': '/?show=sets', '/tapes': '/?show=tapes', '/filmed': '/?show=tapes' };

/** What the pencil makes: a set while the filter is on Sets, else a blank page. */
export const pencilMakes = (filter: Filter): 'set' | 'page' => (filter === 'sets' ? 'set' : 'page');
