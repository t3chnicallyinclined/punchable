// Shows and sets: one index of every set with its show date, venue short name and laugh stats, plus the per-row
// metrics of a set analysis. Sources, in order of preference:
//   the analysis summary fields (requested by app: docs/API_CONTRACT.md) → GET /api/venues per-set stats (contract)
//   → derived from the laugh events and row windows the API already returns.
import { useEffect, useMemo, useState } from 'react';
import { SetCard, decide, undo, useApi } from './api';
import { VenuesView, creative, useCreative } from './creative';
import {
  audioUnreliable, displayRating, isNonBit, roomComparable, showDate, venueShort,
} from './laughs';

export type SetInfo = {
  id: string; card: SetCard; title: string;
  show: ReturnType<typeof showDate>; venue: { name: string; guessed: boolean; raw: string; yours: boolean }; venueId: string | null;
  lspm: number | null; roomHeat: number | null; pct: number | null; material_s: number | null; roomLaughs: number;
  unreliable: boolean; unreliableYours: boolean; best: { id: string; title: string } | null;
};

// ---- his edits to a set (show date, venue, audio unreliable): one decision each; mirrored locally until the
// API's SetCard carries show_date / audio_unreliable (requested by app).
type SetEdit = { show_date?: string; venue?: string; audio_unreliable?: boolean };
const EKEY = 'pk.setEdits.v1';
let edits: Record<string, SetEdit> = (() => {
  try {
    return JSON.parse((globalThis as any).localStorage?.getItem(EKEY) || '{}');
  } catch {
    return {};
  }
})();
const editListeners = new Set<() => void>();
function saveEdits() {
  try {
    (globalThis as any).localStorage?.setItem(EKEY, JSON.stringify(edits));
  } catch {}
  editListeners.forEach((f) => f());
}
export function onSetEdits(f: () => void) {
  editListeners.add(f);
  return () => {
    editListeners.delete(f);
  };
}
/** Write one decision (node edit on the set) and mirror it. Returns the undo. */
export async function editSet(setId: string, after: SetEdit): Promise<() => Promise<void>> {
  const w = await decide({ target: 'node', action: 'edit', id: setId, after });
  const before = { ...(edits[setId] || {}) };
  edits = { ...edits, [setId]: { ...before, ...after } };
  saveEdits();
  return async () => {
    await undo(w.indexes);
    edits = { ...edits, [setId]: before };
    saveEdits();
  };
}
export const setEditsOf = (id: string) => edits[id] || {};

export function buildSetIndex(cards: SetCard[], venues?: VenuesView | null): Map<string, SetInfo> {
  const stats = new Map<string, { lspm: number; heat: number; pct: number; material_s: number; venue_id: string; name: string }>();
  for (const v of venues?.venues || []) {
    for (const s of v.sets || []) stats.set(`set:${s.set.replace(/^set:/, '')}`, { lspm: s.laugh_s_per_min, heat: s.room_heat, pct: s.pct_laughing, material_s: s.material_s, venue_id: v.venue_id, name: v.name });
  }
  const out = new Map<string, SetInfo>();
  for (const c of cards) {
    const e = edits[c.id] || {};
    const st = stats.get(c.id);
    const lspm = c.laugh_s_per_min ?? st?.lspm ?? null;
    const roomHeat = c.room_heat ?? st?.heat ?? null;
    const markedYours = e.audio_unreliable ?? c.audio_unreliable ?? null;
    const show = e.show_date ? { date: e.show_date, inferred: false, source: 'yours' as const } : showDate(c);
    const vraw = e.venue || c.venue;
    const vs = venueShort(vraw);
    out.set(c.id, {
      id: c.id, card: c, title: c.title, show,
      venue: { name: e.venue ? e.venue : vs.name, guessed: e.venue ? false : vs.guessed, raw: c.venue, yours: !!e.venue },
      venueId: c.venue_id ?? st?.venue_id ?? null,
      lspm, roomHeat, pct: st?.pct ?? null, material_s: st?.material_s ?? null, roomLaughs: c.laughs,
      unreliable: markedYours != null ? !!markedYours : audioUnreliable({ room_heat: roomHeat, room_laughs: c.laughs }) || (roomHeat == null && c.laughs < 10 && c.duration_s >= 180),
      unreliableYours: markedYours != null,
      best: c.analysis?.best ? { id: c.analysis.best.id, title: c.analysis.best.title } : null,
    });
  }
  return out;
}

/** Every set (GET /api/sets) joined with the per-set laugh stats (GET /api/venues), keyed by set id. */
export function useSets() {
  const sets = useApi<{ sets: SetCard[] }>('/api/sets');
  const venues = useCreative<VenuesView>('venues', creative.venues);
  const [gen, setGen] = useState(0);
  useEffect(() => onSetEdits(() => setGen((g) => g + 1)), []);
  const index = useMemo(() => buildSetIndex(sets.data?.sets || [], venues.data), [sets.data, venues.data, gen]); // eslint-disable-line react-hooks/exhaustive-deps
  const list = useMemo(() => [...index.values()].sort((a, b) => (b.show.date || '').localeCompare(a.show.date || '')), [index]);
  return { index, list, loading: sets.loading && !sets.data, error: sets.error, reload: sets.reload, venues, fixture: venues.fixture };
}

// ---- one set's rows with their metrics
export type RowM = {
  row: any; idx: number; nonBit: boolean; lspm: number | null; vs: number | null; comparable: boolean;
  shown: ReturnType<typeof displayRating>; ttfl: number | null;
};
export type SetMetrics = {
  rate: number | null; rateSource: 'summary' | 'venues' | 'derived'; roomLaughs: number; material_s: number;
  pct: number | null; rolling: number | null; longest: { s: number; at_ms: number; title: string | null; idx: number | null } | null;
  quiet: { s: number; at_ms: number } | null; rows: RowM[]; durationMs: number;
};
export function setMetrics(an: any, info?: SetInfo | null): SetMetrics {
  const rows: any[] = an?.rows || [];
  const S = an?.summary || {};
  const laughs: { start_ms: number; end_ms: number; bars: number }[] = [...(an?.laughs || [])].sort((a, b) => a.start_ms - b.start_ms);
  const spanStart = rows.length ? Math.min(...rows.map((r) => r.start_ms)) : 0;
  const spanEnd = rows.length ? Math.max(...rows.map((r) => r.end_ms)) : 0;
  const material_s = S.material_s ?? info?.material_s ?? (spanEnd - spanStart) / 1000;
  const sumL = rows.reduce((a, r) => a + (r.laughs?.laugh_s || 0), 0);
  const sumW = rows.reduce((a, r) => a + (r.laughs?.window_s || 0), 0);
  const rate = S.laugh_s_per_min ?? info?.lspm ?? (sumW ? sumL / (sumW / 60) : null);
  const rateSource: SetMetrics['rateSource'] = S.laugh_s_per_min != null ? 'summary' : info?.lspm != null ? 'venues' : 'derived';
  const roomLaughs = S.laughs ?? info?.roomLaughs ?? laughs.length;
  const pct = S.pct_laughing ?? info?.pct ?? (material_s ? (laughs.reduce((a, l) => a + (l.end_ms - l.start_ms), 0) / 1000 / material_s) * 100 : null);
  const rolling = S.laughs_per_min_merged ?? (material_s ? laughs.length / (material_s / 60) : null);
  let longest: SetMetrics['longest'] = null;
  if (S.longest_laugh_s != null) {
    const lj = S.longest_laugh_joke || {};
    longest = { s: S.longest_laugh_s, at_ms: lj.start_ms ?? 0, title: lj.title ?? null, idx: lj.idx ?? null };
  } else if (laughs.length) {
    const l = laughs.reduce((a, b) => (b.end_ms - b.start_ms > a.end_ms - a.start_ms ? b : a));
    const r = rows.find((x) => x.start_ms <= l.start_ms && l.start_ms <= x.end_ms);
    longest = { s: (l.end_ms - l.start_ms) / 1000, at_ms: l.start_ms, title: r ? (r.joke?.title || r.title) : null, idx: r?.idx ?? null };
  }
  let quiet: SetMetrics['quiet'] = null;
  if (S.dead_air_max_s != null) quiet = { s: S.dead_air_max_s, at_ms: S.dead_air_max_at_ms ?? 0 };
  else if (laughs.length) {
    const gaps: [number, number][] = [[laughs[0].start_ms - spanStart, spanStart], [spanEnd - laughs[laughs.length - 1].end_ms, laughs[laughs.length - 1].end_ms]];
    for (let i = 0; i + 1 < laughs.length; i++) gaps.push([laughs[i + 1].start_ms - laughs[i].end_ms, laughs[i].end_ms]);
    const g = gaps.reduce((a, b) => (b[0] > a[0] ? b : a));
    quiet = { s: g[0] / 1000, at_ms: g[1] };
  }
  const rm: RowM[] = rows.map((r) => {
    const nonBit = isNonBit({ status: r.status, title: r.title });
    const w = r.laughs?.window_s ?? 0;
    const lspm = r.laugh_s_per_min ?? (w > 0 ? (r.laughs.laugh_s || 0) / (w / 60) : null);
    const vs = r.vs_room ?? (lspm != null && rate ? lspm / rate : null);
    const comparable = roomComparable(roomLaughs, w);
    const firstTick = (r.ticks || [])[0];
    const ttfl = r.time_to_first_laugh_s ?? (firstTick ? (firstTick.start_ms - r.start_ms) / 1000 : null);
    return { row: r, idx: r.idx, nonBit, lspm, vs, comparable, shown: displayRating({ status: r.status, title: r.title, rating: r.rating, rating_by: r.rating_by, vs_room: vs, laughs: r.laughs }, roomLaughs), ttfl };
  });
  return { rate, rateSource, roomLaughs, material_s, pct, rolling, longest, quiet, rows: rm, durationMs: Math.max(spanEnd, (an?.summary?.duration_s || 0) * 1000) };
}

/** "New tonight": status new, plus anything he re-filed (crowd work / banter) that still got ≥ 1.5× the room. */
export function newTonight(m: SetMetrics) {
  return m.rows.filter((r) => r.row.status === 'new' || (r.row.ai_status && r.row.ai_status !== r.row.status && r.comparable && (r.vs ?? 0) >= 1.5)
    || (r.row.kind_by === 'you' && r.row.status !== 'known' && r.comparable && (r.vs ?? 0) >= 1.5));
}
