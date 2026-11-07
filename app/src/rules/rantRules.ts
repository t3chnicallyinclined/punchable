// The Rant tab's pure rules (docs/design/SPEC-2026-11-06-rant-tab.md §12: the creative space): a topic's row on the
// tab, where he left off, the topic page's material (Kept, Found, Dropped) with his taps applied at once, a card's
// head and its lines, what a draft draws on, the filmed takes, and the rants no topic names. No React, no network:
// tests/rantRules.test.mjs runs it under node, with a --red arm.
import { mmss } from './format.ts';
import { linesOf, monthDay } from './bitLines.ts';
import { DEFAULT_WPM } from './prompterRules.ts';

/** A topic, as GET /api/topics sends a row: the topic and its counts (`found` null until the server has looked). */
export type Topic = {
  id: string; text: string; created?: string | null; touched?: string | null; ranted_at?: string | null; rant?: string | null;
  kept?: number; jots?: number; dropped?: number; found?: number | null; found_more?: boolean;
  rants?: string[]; last_lines?: string[]; v?: number | null; at?: string | null; words?: number | null; rant_id?: string | null;
  /** the newest rant's version unit (a local draft written over another one is stale) */
  unit?: string | null;
};
/** A bit with a rant cut, as GET /api/rants sends it. */
export type RantRow = { id: string; title: string; short_name: string; versions: number; v: number; unit: string; at?: string | null; hook?: boolean; written?: boolean };
/** A filmed take on this phone (rules/memoRules LocalRec, the fields used here). */
export type Take = { id: string; kind: string; recorded_at: string; duration_s: number; bit?: { id: string; title: string } | null };
/** One piece of material (GET /api/topics/<id>, §12.7 `Mat`). */
export type Mat = {
  ref: string; kind: 'bit' | 'moment' | 'jot'; label: string; short_name?: string | null; v?: number | null; at?: string | null;
  words: string; punch?: number[]; venue?: string | null; set?: string | null; start_ms?: number | null; distance?: number | null;
  state?: State | null;
};
export type State = 'kept' | 'dropped';
/** One rant on a topic page (its newest rant cut, each line with where it came from). */
export type RantDoc = { id: string; title: string; short_name: string; v: number; versions: number; at?: string | null; unit: string; words: number; lines: { text: string; by?: 'him' | 'ai' | 'yours' | null; from?: string | null; label?: string | null; edited?: boolean; punch?: boolean }[] };
/** GET /api/topics/<id>. */
export type TopicPage = { topic: Topic; rants: RantDoc[]; kept: Mat[]; found: Mat[]; found_more: boolean; dropped: Mat[] };

/** Rows one page of Found shows; the server says "more" past it ("30+"). */
export const FOUND_CAP = 30;

/** The run time of a number of words at his pace: "0:58". */
export const runOf = (words: number | null | undefined, wpm: number = DEFAULT_WPM): string => mmss(((words || 0) / (wpm > 0 ? wpm : DEFAULT_WPM)) * 60);

/** The filmed takes of a bit, newest first (Film it's takes kept on this phone). */
export function takesOf(takes: Take[], bit: string | null | undefined): Take[] {
  if (!bit) return [];
  return takes.filter((t) => t.kind === 'film' && t.bit?.id === bit).sort((a, b) => b.recorded_at.localeCompare(a.recorded_at));
}

/** A topic row's one sub line (§12.2), the first that applies: "Rant v3 · filmed Nov 6" · "Rant v3" · "3 kept · 9 found
 *  · 1 jot" (zero parts left out, "30+ found" at the cap) · "Nothing yet". Empty while the server has not looked yet
 *  and there is nothing else to say (never a number with nothing behind it). */
export function topicSub(t: Topic, filmedAt?: string | null): string {
  if (t.rant_id && t.v) return `Rant v${t.v}${filmedAt ? ` · filmed ${monthDay(filmedAt)}` : ''}`;
  const kept = t.kept || 0;
  const jots = t.jots || 0;
  const parts: string[] = [];
  if (kept) parts.push(`${kept} kept`);
  if (t.found) parts.push(`${foundWord(t.found, !!t.found_more)} found`);
  if (jots) parts.push(jots === 1 ? '1 jot' : `${jots} jots`);
  if (parts.length) return parts.join(' · ');
  return t.found == null ? '' : 'Nothing yet';
}

/** "9" / "30+". */
export const foundWord = (n: number, more: boolean): string => `${n}${more ? '+' : ''}`;

/** Where he left off (§12.2): the most recently touched topic with a rant (rows come last touched first). */
export const leftOff = (topics: Topic[]): Topic | null => topics.find((t) => !!t.rant_id && !!t.v) || null;

/** The rants no topic names (made from a bit's Make it a rant): rows in Topics that open the bit. */
export function rantsWithoutTopic(rants: RantRow[], topics: Topic[]): RantRow[] {
  const named = new Set(topics.flatMap((t) => [...(t.rants || []), ...(t.rant ? [t.rant] : [])]));
  return rants.filter((r) => !named.has(r.id));
}

/** The topic page's lists with his taps applied at once (`marks`: a ref's new state, `null` = back in Found), and the
 *  rows Find more or the search brought (`extra`, appended to Found). Each ref is in one list only. */
export function materialRows(page: Pick<TopicPage, 'kept' | 'found' | 'dropped'> | null | undefined, marks: Record<string, State | null> = {}, extra: Mat[] = []): { kept: Mat[]; found: Mat[]; dropped: Mat[] } {
  const all = new Map<string, Mat>();
  const base = new Map<string, State | null>();
  for (const [list, st] of [[page?.kept, 'kept'], [page?.found, null], [page?.dropped, 'dropped'], [extra, undefined]] as const) {
    for (const m of list || []) {
      if (all.has(m.ref)) continue;
      all.set(m.ref, m);
      base.set(m.ref, st === undefined ? (m.state ?? null) : st);
    }
  }
  const out = { kept: [] as Mat[], found: [] as Mat[], dropped: [] as Mat[] };
  const order: string[] = [];
  // a tap moves the card to the top of its new list (the newest kept first), never jots above his jots
  const tapped = Object.keys(marks).filter((r) => all.has(r));
  for (const r of [...tapped.reverse(), ...[...all.keys()].filter((r) => !(r in marks))]) order.push(r);
  for (const r of order) {
    const m = all.get(r) as Mat;
    const st = r in marks ? marks[r] : base.get(r) ?? null;
    (st === 'kept' ? out.kept : st === 'dropped' ? out.dropped : out.found).push(m);
  }
  const jotsFirst = (a: Mat, b: Mat) => Number(b.kind === 'jot') - Number(a.kind === 'jot');
  out.kept.sort(jotsFirst);
  // Found stays closest first: a row put back returns to its place
  const rank = new Map<string, number>();
  for (const x of [...(page?.found || []), ...extra]) if (!rank.has(x.ref)) rank.set(x.ref, rank.size);
  out.found.sort((a, b) => (rank.get(a.ref) ?? -1) - (rank.get(b.ref) ?? -1));
  return out;
}

/** What a draft draws on (§12.3 C4: nothing has to be sorted): Kept when anything is kept, else the Found he has not
 *  dropped. `null` when there is nothing. The button's words: "Draft it from 3 kept" / "Draft it from 9 found". */
export function draftFrom(rows: { kept: Mat[]; found: Mat[] }, foundMore = false): { n: number; word: string } | null {
  if (rows.kept.length) return { n: rows.kept.length, word: `${rows.kept.length} kept` };
  if (rows.found.length) return { n: rows.found.length, word: `${foundWord(rows.found.length, foundMore)} found` };
  return null;
}

/** A card's head (§12.3): "Free employee · v3 · Oct 29" · "On tape · Nov 2 · The Back Room · 0:41" · "Jot · Nov 7". */
export function cardHead(m: Mat): string {
  if (m.kind === 'jot') return ['Jot', monthDay(m.at)].filter(Boolean).join(' · ');
  if (m.kind === 'moment') {
    const at = m.start_ms != null && m.start_ms >= 0 ? mmss(m.start_ms / 1000, { floor: true }) : '';
    return ['On tape', monthDay(m.at), m.venue || '', at].filter(Boolean).join(' · ');
  }
  return [m.short_name || m.label, m.v ? `v${m.v}` : '', monthDay(m.at)].filter(Boolean).join(' · ');
}

/** A card's lines: the first three, then how many more ("+ 2 more lines" unfolds in place). */
export const CARD_LINES = 3;
export function cardLines(m: Mat, open = false): { lines: string[]; more: number; punch: number[] } {
  const all = linesOf(m.words);
  const shown = open ? all : all.slice(0, CARD_LINES);
  const punch = (m.punch || []).filter((i) => Number.isInteger(i) && i >= 0 && i < shown.length);
  return { lines: shown, more: all.length - shown.length, punch };
}
export const moreWord = (n: number): string => `+ ${n} more ${n === 1 ? 'line' : 'lines'}`;

/** VoiceOver for a card: one element (§12.3): "Free employee, version 3, kept: I scan, I bag…". */
export function cardA11y(m: Mat, state: State | null): string {
  const who = m.kind === 'moment' ? `On tape${m.venue ? ` at ${m.venue}` : ''}` : m.kind === 'jot' ? 'Your jot' : `${m.short_name || m.label}${m.v ? `, version ${m.v}` : ''}`;
  const st = state === 'kept' ? ', kept' : state === 'dropped' ? ', dropped' : '';
  return `${who}${st}: ${linesOf(m.words).join(' ')}`;
}

/** The label a line pulled from this material carries in the script: "Free employee v3" / "On tape Nov 2" / "Jot". */
export function sourceLabel(m: Mat): string {
  if (m.kind === 'moment') return ['On tape', monthDay(m.at)].filter(Boolean).join(' ');
  if (m.kind === 'jot') return 'your jot';
  return m.label || m.short_name || 'your bit';
}

/** The new bit's title: the topic, first letter up, at most 60 characters, cut at a word. */
export function rantTitle(topic: string): string {
  const t = topic.replace(/\s+/g, ' ').trim();
  let cut = t;
  if (t.length > 60) {
    const sp = t.slice(0, 60).lastIndexOf(' ');
    cut = sp > 30 ? t.slice(0, sp) : t.slice(0, 60);
  }
  return cut ? cut[0].toUpperCase() + cut.slice(1) : 'Rant';
}

/** A filmed take's line: "Nov 6 · 0:58 · in Photos". */
export function filmedSub(t: Take): string {
  return [monthDay(t.recorded_at), mmss(t.duration_s), 'in Photos'].filter(Boolean).join(' · ');
}

/** The filmed takes of rants on this phone, newest first, each with the rant and its topic when one names it. */
export function filmedList(takes: Take[], rants: RantRow[], topics: Topic[]): { take: Take; rant: RantRow; topic: Topic | null }[] {
  const byId = new Map(rants.map((r) => [r.id, r]));
  const topicOf = (id: string) => topics.find((t) => (t.rants || []).includes(id) || t.rant === id) || null;
  return takes
    .filter((t) => t.kind === 'film' && t.bit && byId.has(t.bit.id))
    .sort((a, b) => b.recorded_at.localeCompare(a.recorded_at))
    .map((t) => ({ take: t, rant: byId.get((t.bit as { id: string }).id) as RantRow, topic: topicOf((t.bit as { id: string }).id) }));
}
