// Laugh-metric rules (UX_SPEC section 5). Pure functions, no imports, so `tests/laughs.test.mjs` can run them
// with `node --experimental-strip-types` against every real analysis.json.
//
// The rules the UI enforces:
// - laugh-s/min leads (1 dp). Raw detector laughs/min is shown only inside the Why sheet.
// - Banter / walk-on / sign-off / crowd work get no rating and no metrics.
// - The displayed rating word is derived from vs_room (killed >= 1.5, worked 0.8-1.5, meh 0.2-0.8, died < 0.2 or
//   no laugh in a window >= 20 s), unless the comic set it himself.
// - x room is hidden when the room had < 10 laughs or the window is < 20 s.
// - Trend words (rising / holding / slipping) only with >= 4 non-excluded nights, never a percentage.
// - Fragments of one bit in one set are merged before any per-night stat.

export type Rating = 'killed' | 'worked' | 'meh' | 'died';
export const RATINGS: Rating[] = ['killed', 'worked', 'meh', 'died'];

const isNum = (x: unknown): x is number => typeof x === 'number' && isFinite(x);

/** Round to 1 decimal. */
export const r1 = (x: number | null | undefined): number | null => (isNum(x) ? Math.round(x * 10) / 10 : null);
/** "5.1" — never two decimals. */
export const fmt1 = (x: number | null | undefined, empty = '–') => (isNum(x) ? (Math.round(x * 10) / 10).toFixed(1) : empty);
/** "9%" — integer percent. */
export const fmtPct = (x: number | null | undefined) => (isNum(x) ? `${Math.round(x)}%` : '–');
/** Seconds as m:ss. */
export const fmtDur = (s: number | null | undefined) => {
  if (!isNum(s)) return '–';
  const t = Math.max(0, Math.round(s));
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`;
};
/** Whole seconds: "5 s". */
export const fmtSec = (s: number | null | undefined) => (isNum(s) ? `${Math.round(s)} s` : '–');
/** "1.8× room". */
export const fmtVs = (v: number | null | undefined) => (isNum(v) ? `${fmt1(v)}× room` : '');

// ------------------------------------------------------------------ what counts as a bit
const NON_BIT_STATUS = new Set(['banter', 'crowd_work', 'walk_on', 'sign_off']);
const NON_BIT_TITLE = /^(walk[- ]?on|sign[- ]?off|banter|crowd work)\b/i;
/** Banter, walk-on, sign-off and crowd work: no rating, no metrics (they still count in the room's totals). */
export function isNonBit(row: { status?: string | null; kind?: string | null; title?: string | null }): boolean {
  return NON_BIT_STATUS.has(String(row.status || '')) || NON_BIT_STATUS.has(String(row.kind || '')) || NON_BIT_TITLE.test(String(row.title || ''));
}
export function nonBitWord(row: { status?: string | null; kind?: string | null; title?: string | null }): string {
  const t = String(row.title || '');
  if (/^walk/i.test(t)) return 'walk-on';
  if (/^sign/i.test(t)) return 'sign-off';
  if (row.status === 'crowd_work' || row.kind === 'crowd_work') return 'crowd work';
  return 'banter';
}

// ------------------------------------------------------------------ comparability and ratings
export const MIN_ROOM_LAUGHS = 10;
export const MIN_WINDOW_S = 20;
/** x room is only meaningful when the room laughed at least 10 times and the bit ran at least 20 s. */
export function roomComparable(roomLaughs: number | null | undefined, windowS?: number | null): boolean {
  if (!isNum(roomLaughs) || roomLaughs < MIN_ROOM_LAUGHS) return false;
  if (isNum(windowS) && windowS < MIN_WINDOW_S) return false;
  return true;
}

/** Section 5.5 bands. `laughS` lets a silent long window read "died" even without a ratio. */
export function ratingFromVsRoom(vs: number | null | undefined, laughS?: number | null, windowS?: number | null): Rating | null {
  if (isNum(laughS) && laughS === 0 && isNum(windowS) && windowS >= MIN_WINDOW_S) return 'died';
  if (!isNum(vs)) return null;
  if (vs >= 1.5) return 'killed';
  if (vs >= 0.8) return 'worked';
  if (vs >= 0.2) return 'meh';
  return 'died';
}

export type RowLike = {
  status?: string | null; kind?: string | null; title?: string | null;
  rating?: string | null; rating_by?: string | null; ai_rating?: string | null;
  vs_room?: number | null; laugh_s_per_min?: number | null;
  laughs?: { laugh_s?: number | null; window_s?: number | null; count?: number | null } | null;
};
export type Shown = { word: Rating | null; by: 'you' | 'numbers' | 'none'; measured: boolean };
/**
 * The rating word a row shows. His edit wins; non-bits show nothing; otherwise it is derived from the numbers,
 * so the word and the bar can't disagree. When the room is too quiet to compare, only the measurement shows.
 */
export function displayRating(row: RowLike, roomLaughs: number | null | undefined): Shown {
  if (isNonBit(row)) return { word: null, by: 'none', measured: false };
  if (row.rating_by === 'you' && row.rating && (RATINGS as string[]).includes(row.rating)) return { word: row.rating as Rating, by: 'you', measured: true };
  const w = row.laughs?.window_s;
  if (!roomComparable(roomLaughs, w)) {
    // a silent long window is still "died" (no ratio needed)
    const silent = ratingFromVsRoom(null, row.laughs?.laugh_s ?? null, w ?? null);
    return { word: silent, by: silent ? 'numbers' : 'none', measured: !!silent };
  }
  const word = ratingFromVsRoom(row.vs_room, row.laughs?.laugh_s ?? null, w ?? null);
  return { word, by: word ? 'numbers' : 'none', measured: true };
}

// ------------------------------------------------------------------ room heat, unreliable audio
/** Room heat (set laugh-s/min over his median) as words. A cold room is "a quiet room", never a bad night. */
export function heatWord(roomHeat: number | null | undefined): 'quiet room' | 'about your usual' | 'hot room' | null {
  if (!isNum(roomHeat)) return null;
  if (roomHeat < 0.8) return 'quiet room';
  if (roomHeat < 1.2) return 'about your usual';
  return 'hot room';
}
/** 0 = quiet, 1 = usual, 2 = hot (pins, calendar dots). */
export const heatStep = (roomHeat: number | null | undefined): 0 | 1 | 2 | null => (!isNum(roomHeat) ? null : roomHeat < 0.8 ? 0 : roomHeat < 1.2 ? 1 : 2);

/** "audio unreliable": room_heat < 0.3 with fewer than 10 room laughs, or he marked it. Excluded from averages and trends. */
export function audioUnreliable(s: { room_heat?: number | null; room_laughs?: number | null; marked?: boolean | null }): boolean {
  if (s.marked) return true;
  return isNum(s.room_heat) && s.room_heat < 0.3 && isNum(s.room_laughs) && s.room_laughs < MIN_ROOM_LAUGHS;
}

// ------------------------------------------------------------------ nights, trends
export type Night = { set: string; date?: string | null; label?: string | null; venue?: string | null; laugh_s: number; window_s: number; vs_room?: number | null; excluded?: boolean; [k: string]: any };
/** One entry per set: split performance fragments of one bit in one set are summed first. */
export function mergeNights(perfs: Night[]): (Night & { laugh_s_per_min: number; fragments: number })[] {
  const by = new Map<string, Night & { fragments: number; _vsw: number }>();
  for (const p of perfs) {
    const cur = by.get(p.set);
    if (!cur) {
      by.set(p.set, { ...p, fragments: 1, _vsw: isNum(p.vs_room) ? p.vs_room * p.window_s : 0 });
    } else {
      cur.laugh_s += p.laugh_s;
      cur.window_s += p.window_s;
      cur.fragments += 1;
      cur._vsw += isNum(p.vs_room) ? p.vs_room * p.window_s : 0;
      cur.excluded = cur.excluded || p.excluded;
    }
  }
  return [...by.values()].map(({ _vsw, ...n }) => ({
    ...n,
    vs_room: n.fragments > 1 ? (n.window_s ? _vsw / n.window_s : null) : n.vs_room ?? null,
    laugh_s_per_min: n.window_s > 0 ? n.laugh_s / (n.window_s / 60) : 0,
  }));
}

export const MIN_TREND_NIGHTS = 4;
export type TrendWord = 'rising' | 'holding' | 'slipping' | 'early read';
/** Least-squares slope over equal spacing, relative to the mean; ±15 % over the run decides the word. */
export function trendWord(values: { v: number; excluded?: boolean }[]): TrendWord {
  const ys = values.filter((x) => !x.excluded && isNum(x.v)).map((x) => x.v);
  if (ys.length < MIN_TREND_NIGHTS) return 'early read';
  const n = ys.length;
  const mx = (n - 1) / 2;
  const my = ys.reduce((a, b) => a + b, 0) / n;
  let num = 0;
  let den = 0;
  ys.forEach((y, i) => {
    num += (i - mx) * (y - my);
    den += (i - mx) ** 2;
  });
  if (!my) return 'holding';
  const rel = ((num / den) * (n - 1)) / my;
  return rel > 0.15 ? 'rising' : rel < -0.15 ? 'slipping' : 'holding';
}

// ------------------------------------------------------------------ small visual scales
/** Desk strength dots from a bit's avg vs_room: 0 = never performed (hollow). */
export function strengthDots(avgVsRoom: number | null | undefined, performed = true): 0 | 1 | 2 | 3 | 4 {
  if (!performed || !isNum(avgVsRoom)) return 0;
  return avgVsRoom < 0.8 ? 1 : avgVsRoom < 1.3 ? 2 : avgVsRoom < 2 ? 3 : 4;
}
/** Working-set cells: 0 = < 0.8x, 1 = 0.8-1.3x, 2 = >= 1.3x, null = not performed. */
export const vsBand = (vs: number | null | undefined): 0 | 1 | 2 | null => (!isNum(vs) ? null : vs < 0.8 ? 0 : vs < 1.3 ? 1 : 2);
/** Reception meter (0-4 bars) from vs_room, aligned with the rating bands. */
export const receptionBars = (vs: number | null | undefined): 0 | 1 | 2 | 3 | 4 =>
  !isNum(vs) ? 0 : vs < 0.2 ? 0 : vs < 0.8 ? 1 : vs < 1.5 ? 2 : vs < 2.2 ? 3 : 4;
export const RECEPTION_WORDS = ['no laugh', 'a little', 'with the room', 'above the room', 'way above the room'];
/** Detector bars per laugh (0-4) in words, for punchlines. */
export const BAR_WORDS = ['no laugh', 'chuckle', 'laugh', 'big laugh', 'applause break'];

/** Perfected: the stability word for locked bits' CV across the last nights. */
export function stabilityWord(cv: number | null | undefined, nights: number): 'steady' | 'settling' | 'swinging' | 'too early' {
  if (nights < 3 || !isNum(cv)) return 'too early';
  return cv < 0.25 ? 'steady' : cv < 0.5 ? 'settling' : 'swinging';
}

/** Section 8.3: confidence in words at rest; the % only in the Why sheet. */
export function confWord(c: number | null | undefined): 'sure' | 'likely' | 'maybe' | 'long shot' | null {
  if (!isNum(c)) return null;
  return c >= 0.9 ? 'sure' : c >= 0.75 ? 'likely' : c >= 0.5 ? 'maybe' : 'long shot';
}

// ------------------------------------------------------------------ show dates
const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
/**
 * The date of the show, not the upload: a date written in the title or description ("8-16-2017", "5/8/23",
 * "6/20") wins over the upload date, and is marked as inferred. A year-less date takes the upload's year
 * (or the year before, if that would put the show after the upload).
 */
export function showDate(s: { date?: string | null; title?: string | null; description?: string | null; show_date?: string | null; show_date_source?: string | null }):
  { date: string | null; inferred: boolean; source: 'yours' | 'title' | 'description' | 'upload' | 'api' } {
  if (s.show_date) return { date: s.show_date, inferred: s.show_date_source !== 'yours', source: s.show_date_source === 'yours' ? 'yours' : 'api' };
  const up = s.date || null;
  const upY = up ? Number(up.slice(0, 4)) : null;
  for (const [src, text] of [['title', s.title], ['description', s.description]] as const) {
    const m = String(text || '').match(/(?:^|[^\d])(\d{1,2})[/-](\d{1,2})(?:[/-](\d{2,4}))?(?!\d)/);
    if (!m) continue;
    const mo = Number(m[1]);
    const d = Number(m[2]);
    if (mo < 1 || mo > 12 || d < 1 || d > 31) continue;
    let y = m[3] ? Number(m[3]) : upY;
    if (y == null) continue;
    if (y < 100) y += 2000;
    let iso = `${y}-${String(mo).padStart(2, '0')}-${String(d).padStart(2, '0')}`;
    if (!m[3] && up && iso > up) iso = `${y - 1}${iso.slice(4)}`;
    if (up && iso > up) continue; // a show can't be after its upload
    return { date: iso, inferred: true, source: src };
  }
  return { date: up, inferred: false, source: 'upload' };
}
export const fmtShortDate = (iso?: string | null) => {
  if (!iso) return '';
  const [y, m, d] = iso.slice(0, 10).split('-').map(Number);
  return `${MON[(m || 1) - 1]} ${d} '${String(y).slice(2)}`;
};

// ------------------------------------------------------------------ venue short names
/** "Mabel's Comedy Club, Springfield" -> "Mabel's"; "Unknown (title 'Beach jokes'; maybe Seaside)" -> "Seaside?" */
export function venueShort(raw?: string | null): { name: string; guessed: boolean } {
  const r = String(raw || '').trim();
  if (!r) return { name: 'Unknown venue', guessed: true };
  const maybe = r.match(/maybe ([^)';]+)/i);
  if (/^unknown/i.test(r)) return { name: maybe ? `${maybe[1].trim()}?` : 'Unknown venue', guessed: true };
  const guessed = /\b(likely|maybe|\(show\))/i.test(r);
  let name = r.split(',')[0].replace(/\(show\)/i, '').replace(/\blikely\b.*$/i, '').trim();
  name = name.replace(/^Comix at\s+/i, 'Comix ').replace(/\s+(Comedy Club|Comedy Show)$/i, '').trim();
  return { name: guessed ? `${name}?` : name, guessed };
}
