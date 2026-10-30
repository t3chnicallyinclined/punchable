// Rows for "In your sets" (Bits search) and "Elsewhere" (a bit's page), fed by api-dev/search_api.py:
// GET /api/search?q= and GET /api/jokes/<id>/similar. A taped moment's where-and-when, its deep link into the set at
// that second, and how close a vector match is in words a comic reads. Pure (no React Native): tests/searchRows.test.mjs.
import { mmss, monthYear } from './format.ts';

export type Moment = {
  id: string; set: string | null; set_title?: string | null; venue?: string | null; date?: string | null;
  start_ms?: number | null; end_ms?: number | null; text: string; joke?: string | null; joke_title?: string | null;
  score?: number | null; status?: 'confirmed' | 'proposed' | 'rejected' | string;
};
export type FoundBit = { id: string; title: string; stage?: string; summary?: string; short_name?: string | null; why?: string | null; distance?: number | null };
export type SearchResult = { query: string; bits: FoundBit[]; moments: Moment[]; vectors: boolean };
export type SimilarResult = { performed: Moment[]; like: { bits: FoundBit[]; moments: Moment[] }; vectors: boolean; text: string };


/** "The Cellar · Mar 2025 · 1:00" — venue (else the set's title), the month, where in the tape. Parts missing are left out. */
export function whenWhere(m: Pick<Moment, 'venue' | 'set_title' | 'date' | 'start_ms'>): string {
  const parts: string[] = [];
  const where = (m.venue || m.set_title || '').trim();
  if (where) parts.push(where);
  const when = monthYear(m.date);
  if (when) parts.push(when);
  if (m.start_ms != null && m.start_ms >= 0) parts.push(mmss(m.start_ms / 1000));
  return parts.join(' · ');
}

/** The set screen, scrolled to this moment and playing it (set/[id].tsx reads `at` and `play`). */
export function momentHref(m: Pick<Moment, 'set' | 'start_ms'>): string | null {
  if (!m.set) return null;
  return `/set/${encodeURIComponent(m.set)}?at=${m.start_ms ?? ''}&play=1`;
}

/** Cosine distance (0 = the same words in the same order) in words. Null when the match was words only. */
export function closeness(d?: number | null): 'Very close' | 'Close' | 'A bit like it' | null {
  if (d == null || Number.isNaN(d)) return null;
  if (d < 0.22) return 'Very close';
  if (d < 0.4) return 'Close';
  return 'A bit like it';
}

/** One line of the moment for a row: whitespace folded, cut at a word, an ellipsis if it goes on. */
export function oneLine(text?: string | null, max = 140): string {
  const t = (text || '').replace(/\s+/g, ' ').trim();
  if (t.length <= max) return t;
  const cut = t.lastIndexOf(' ', max);
  return t.slice(0, cut > max * 0.6 ? cut : max).trimEnd() + '…';
}
