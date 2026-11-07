// The Rant tab's pure rules (docs/design/SPEC-2026-11-06-rant-tab.md §2, §3; slice 1): which rants are Drafts and which
// are Finished, a topic's row, which search hits Gather offers and ticks, Film a rant's target, and the assembly of a
// drafted rant: every line is in until he takes it out (his: Cut; a new one: Skip). The owner's standing rule "no review
// step is ever required" wins over the spec's "Keeps or Skips each new line" (§3.3): a new line shows as the current
// state with the AI badge and he edits. No React, no network: tests/rantRules.test.mjs runs it under node, --red arm.
import { mmss } from './format.ts';
import { monthDay } from './bitLines.ts';

/** A topic, as GET /api/topics sends it. */
export type Topic = { id: string; text: string; created?: string | null; ranted_at?: string | null; rant?: string | null };
/** A bit with a rant cut, as GET /api/rants sends it. */
export type RantRow = { id: string; title: string; short_name: string; versions: number; v: number; unit: string; at?: string | null; hook?: boolean; written?: boolean };
/** A filmed take on this phone (rules/memoRules LocalRec, the fields used here). */
export type Take = { id: string; kind: string; recorded_at: string; duration_s: number; bit?: { id: string; title: string } | null };
/** One drafted line (pk-worker rant_draft): his = word for word from `source` (checked on the server); else new. */
export type DraftLine = { role: string; text: string; his: boolean; source?: string | null; source_label?: string | null; source_kind?: string | null };
/** Per line: false = taken out (Cut / Skip), anything else = in. */
export type Marks = (boolean | undefined)[];

/** A search hit closer than this counts as "about" the topic (rules/searchRows closeness: "Close" and nearer). */
export const MATCH_CLOSE = 0.4;

/** Drafts are rants not filmed on this phone, newest first as the server sends them; Finished are the filmed takes
 *  of rants, newest first, each with the rant it came from. */
export function splitRants(rants: RantRow[], takes: Take[]): { drafts: RantRow[]; finished: { take: Take; rant: RantRow }[] } {
  const byId = new Map(rants.map((r) => [r.id, r]));
  const filmed = new Set<string>();
  const finished: { take: Take; rant: RantRow }[] = [];
  for (const t of takes) {
    const r = t.kind === 'film' && t.bit ? byId.get(t.bit.id) : undefined;
    if (!r) continue;
    filmed.add(r.id);
    finished.push({ take: t, rant: r });
  }
  finished.sort((a, b) => b.take.recorded_at.localeCompare(a.take.recorded_at));
  return { drafts: rants.filter((r) => !filmed.has(r.id)), finished };
}

/** A draft's second line: "v3 · Nov 6 · hook". */
export function draftSub(r: RantRow): string {
  return [`v${r.v}`, monthDay(r.at), r.hook ? 'hook' : ''].filter(Boolean).join(' · ');
}

/** A finished video's line: "Nov 6 · 0:58 · in Photos". */
export function finishedSub(t: Take): string {
  return [monthDay(t.recorded_at), mmss(t.duration_s), 'in Photos'].filter(Boolean).join(' · ');
}

/** A topic's trailing words: "ranted Nov 6" once it became a rant, else how many of his bits match ("3 bits"). */
export function topicSub(t: Topic, matches: number | null | undefined): string {
  if (t.ranted_at) return `ranted ${monthDay(t.ranted_at)}`;
  if (matches == null) return '';
  return `${matches} ${matches === 1 ? 'bit' : 'bits'}`;
}

type Found = { id: string; title?: string; short_name?: string | null; summary?: string; distance?: number | null };
type FoundMoment = { id: string; text: string; venue?: string | null; set_title?: string | null; date?: string | null };

/** A hit is about the topic when the words matched (no distance) or the meaning is close. */
export const isClose = (d?: number | null): boolean => d == null || (Number.isFinite(d) && d < MATCH_CLOSE);

/** How many of his bits a topic matches (today's search): the close ones only; a vector search always returns its
 *  limit, so counting every hit would say "6 bits" for anything. */
export function matchCount(r: { bits?: Found[] } | null | undefined): number {
  return (r?.bits || []).filter((b) => isClose(b.distance)).length;
}

export type Hit = { id: string; kind: 'bit' | 'moment'; title: string; sub: string; ticked: boolean };

/** Gather's rows from a search: his bits first (ticked when close), then taped moments (never ticked for him), each id
 *  once. Hits from "Find more" (`already` non-empty) are appended unticked after what is listed. */
export function gatherHits(r: { bits?: Found[]; moments?: FoundMoment[] } | null | undefined, already: Hit[] = []): Hit[] {
  const seen = new Set(already.map((h) => h.id));
  const out = [...already];
  const first = !already.length;
  for (const b of r?.bits || []) {
    if (seen.has(b.id)) continue;
    seen.add(b.id);
    out.push({ id: b.id, kind: 'bit', title: b.short_name || b.title || b.id, sub: (b.summary || '').trim(), ticked: first && isClose(b.distance) });
  }
  for (const m of r?.moments || []) {
    if (seen.has(m.id)) continue;
    seen.add(m.id);
    out.push({ id: m.id, kind: 'moment', title: (m.text || '').replace(/\s+/g, ' ').trim().slice(0, 120), sub: ['On tape', m.venue || m.set_title || '', monthDay(m.date)].filter(Boolean).join(' · '), ticked: false });
  }
  return out;
}

/** At most this many sources go to the draft (pk-api MAX_SOURCES). */
export const MAX_SOURCES = 12;
/** The ticked ids, in list order, at most MAX_SOURCES. */
export const tickedIds = (hits: Hit[]): string[] => hits.filter((h) => h.ticked).map((h) => h.id).slice(0, MAX_SOURCES);

/** Film a rant: nothing to film (no draft), the one draft, or a pick among several (newest first, five at most). */
export function filmTarget(drafts: RantRow[]): { k: 'none' } | { k: 'one'; id: string } | { k: 'pick'; rows: RantRow[] } {
  if (!drafts.length) return { k: 'none' };
  if (drafts.length === 1) return { k: 'one', id: drafts[0].id };
  return { k: 'pick', rows: drafts.slice(0, 5) };
}

/** Whether a line is in: every line, his or new, until he takes it out (no review step: the owner's standing rule). */
export const isIn = (_l: DraftLine, mark: boolean | undefined): boolean => mark !== false;

const kept = (lines: DraftLine[], marks: Marks): DraftLine[] => lines.filter((l, i) => isIn(l, marks[i]) && !!l.text.trim());

/** The rant's lines as saved: the kept ones, in order. */
export const assemble = (lines: DraftLine[], marks: Marks): string[] => kept(lines, marks).map((l) => l.text.trim());

/** Save shows once the rant has a line. */
export const canSaveRant = (lines: DraftLine[], marks: Marks): boolean => assemble(lines, marks).length > 0;

/** The new lines he skipped: the learning loop's lines he passed over (never his own, never one he left in). */
export function skippedNew(lines: DraftLine[], marks: Marks): string[] {
  return lines.filter((l, i) => !l.his && marks[i] === false).map((l) => l.text.trim()).filter(Boolean);
}

/** Which kept lines are his, aligned with `assemble` (the hook check's input). */
export const hisOfKept = (lines: DraftLine[], marks: Marks): boolean[] => kept(lines, marks).map((l) => l.his);

/** The ticked bits the saved rant links to ("from" links): the bits whose lines he kept; moments are not bits. */
export const fromLinks = (lines: DraftLine[], marks: Marks): string[] =>
  [...new Set(kept(lines, marks).filter((l) => l.his && (l.source || '').startsWith('joke:')).map((l) => l.source as string))];

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

/** VoiceOver for a drafted line: "Line 3 of 8, the turn. Yours, from Self-checkout v8: …" / "New, by Claude: …, skipped". */
export function lineLabel(l: DraftLine, i: number, n: number, mark: boolean | undefined): string {
  const head = `Line ${i + 1} of ${n}, the ${l.role}.`;
  const who = l.his ? `Yours${l.source_label ? `, from ${l.source_label}` : ''}` : 'New, by Claude';
  const state = mark === false ? (l.his ? ', cut' : ', skipped') : '';
  return `${head} ${who}: ${l.text}${state}`;
}

/** The button under a line: "Cut" / "Skip" while it is in, "Put back" once taken out. */
export const lineAction = (l: DraftLine, mark: boolean | undefined): string => (mark === false ? 'Put back' : l.his ? 'Cut' : 'Skip');
